//! The core's public API: the same surface the frontend drives over Tauri IPC (ADR 0003).

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};
use tokio::sync::{broadcast, mpsc, oneshot};

use crate::acp::{AcpError, AdapterCommand, Connection, Incoming, Responder, PROTOCOL_VERSION};
use crate::app_state::{self, AppState, SavedSession, WorkspaceState};
use crate::auto_suspend::{
    self, AutoSuspendReason, Candidate, Clock, Limits, MemoryProbe, SystemClock, SystemProbe,
};
use crate::create_worktree::{self, BranchInfo, BranchList, CreatedWorktree, NewWorktree};
use crate::documents::{
    self, CheckTicket, DocumentTracker, OpenDocument, OpenedFile, PoppedOutFile, SaveOver,
};
use crate::files::{
    ActorNews, DirEntry, FileMatch, FileWatchConfig, IndexStats, WatchStatus, WorktreeActor,
};
use crate::git;
use crate::permissions;
use crate::remove_worktree::{self, RemovalCheck, RemoveWorktree, RemovedWorktree};
use crate::session::{
    PermissionMode, PermissionOutcome, SessionId, SessionInfo, SessionState, Transcript,
    TranscriptDelta, TranscriptItem, TranscriptPage,
};
use crate::settings::{self, LoadedSettings, WindowsShell};
use crate::setup::{self, SetupInfo, SetupStatus};
use crate::worktrees::{self, Discovery, WorktreeInfo};

/// How long streamed transcript changes are gathered before being flushed to the visible Tab.
const FLUSH_INTERVAL: Duration = Duration::from_millis(16);

/// The adapter is restarted after a crash unless it has crashed more than `CRASH_LIMIT` times
/// within `CRASH_WINDOW`.
const CRASH_LIMIT: usize = 3;
const CRASH_WINDOW: Duration = Duration::from_secs(60);

/// How long a stopped session's Agent gets to confirm it closed before removal goes ahead anyway.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(10);

/// `session/close`, given `CLOSE_TIMEOUT` to answer; `None` if the Agent didn't.
async fn close_acp(connection: &Connection, acp_id: &str) -> Option<Result<Value, AcpError>> {
    let close = connection.request("session/close", json!({ "sessionId": acp_id }));
    tokio::time::timeout(CLOSE_TIMEOUT, close).await.ok()
}

/// Puts the Agent's ACP session in `mode`.
async fn apply_mode(
    connection: &Connection,
    acp_id: &str,
    mode: PermissionMode,
) -> Result<(), AcpError> {
    connection
        .request(
            "session/set_mode",
            json!({ "sessionId": acp_id, "modeId": mode.acp_id() }),
        )
        .await
        .map(|_| ())
}

/// A prompt error meaning the Agent no longer has the session (e.g. it was closed under it):
/// nothing more will happen there, so it counts as Exited.
fn session_gone(err: &AcpError) -> bool {
    matches!(err, AcpError::Closed)
        || matches!(err, AcpError::Rpc { message, .. } if message.to_lowercase().contains("not found"))
}

/// Refuses removal options that would lose work the user didn't agree to discard.
fn refuse_loss(check: &RemovalCheck, options: &RemoveWorktree) -> Result<(), CoreError> {
    match check.would_lose(options) {
        Some(lost) => Err(CoreError::WouldDiscard(lost)),
        None => Ok(()),
    }
}

/// Whether a failed delete looks like Windows' "in use by another process" (worth retrying), not a
/// refusal that will stay one.
fn in_use(err: &str) -> bool {
    let err = err.to_lowercase();
    [
        "permission denied",
        "being used by another process",
        "access is denied",
        "invalid argument",
        "directory not empty",
    ]
    .iter()
    .any(|sign| err.contains(sign))
}

/// `git worktree remove` is retried this often (a while apart) while Windows still holds the folder.
const REMOVE_RETRIES: u32 = 30;
const REMOVE_RETRY_DELAY: Duration = Duration::from_millis(300);

pub struct CoreConfig {
    pub adapter: AdapterCommand,
    /// The settings file (ticket 08); `None` runs on the defaults, with nothing read or watched.
    pub settings_path: Option<PathBuf>,
    /// Where open Tabs and Recent sessions are kept between runs (ticket 11); `None` keeps nothing.
    pub state_path: Option<PathBuf>,
    /// How memory is read for auto-suspend (ticket 12); `None` reads the OS's.
    pub memory_probe: Option<Arc<dyn MemoryProbe>>,
    /// The clock Idle time is measured by; `None` is the real one.
    pub clock: Option<Arc<dyn Clock>>,
    /// How Worktrees' files are watched (ticket 13).
    pub file_watch: FileWatchConfig,
}

impl CoreConfig {
    /// A config that runs `adapter`, with no settings file or saved state, on the real OS.
    pub fn new(adapter: AdapterCommand) -> Self {
        Self {
            adapter,
            settings_path: None,
            state_path: None,
            memory_probe: None,
            clock: None,
            file_watch: FileWatchConfig::default(),
        }
    }
}

/// How often the memory monitor checks the Agents' memory and Idle times.
const MONITOR_INTERVAL: Duration = Duration::from_secs(10);

/// After auto-suspending, memory isn't acted on for this long: the reading may still include the
/// processes being closed.
const AUTO_SUSPEND_COOLDOWN: Duration = Duration::from_secs(20);

/// A Worktree's actor, started once (by whoever needs it first).
type ActorSlot = Arc<tokio::sync::OnceCell<Arc<WorktreeActor>>>;

/// How long after a burst of file news a Worktree's branch status is refreshed.
const STATUS_SETTLE: Duration = Duration::from_millis(200);
/// A disk check that raced the editor's own save (or an open) is done again after this.
const RECHECK_AFTER: Duration = Duration::from_millis(150);
/// Diffs longer than this are cut short in the Manual editor's diff view.
const MAX_VIEW_DIFF_LINES: usize = 5_000;

/// A session auto-suspend stopped, and why.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoSuspension {
    pub session: SessionInfo,
    pub reason: AutoSuspendReason,
}

/// A closed Tab in a Worktree's Recent sessions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentSession {
    /// The ACP session id, which `reopen_session` takes.
    pub acp_id: String,
    pub name: String,
    pub worktree: PathBuf,
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum CoreError {
    #[error("no Workspace is open")]
    NoWorkspace,
    #[error("`{0}` is not inside a git repository")]
    NotARepository(PathBuf),
    #[error("`{0}` is not a Worktree of this Workspace")]
    UnknownWorktree(PathBuf),
    #[error("the branch `{0}` already exists; check it out as an existing branch instead")]
    BranchExists(String),
    #[error("`{branch}` is already checked out in {}; open that Worktree instead", worktree.display())]
    BranchCheckedOut { branch: String, worktree: PathBuf },
    #[error("there is no branch `{0}`")]
    UnknownBranch(String),
    #[error("`{0}` isn't a branch, tag or commit in this repository")]
    UnknownStartPoint(String),
    #[error("git created {} but doesn't list it as a Worktree", .0.display())]
    WorktreeNotListed(PathBuf),
    #[error("git: {0}")]
    Git(String),
    #[error("no such Agent session")]
    UnknownSession,
    #[error("the Agent session is still working on the previous prompt")]
    SessionBusy,
    #[error("the Agent session has exited; resume it first")]
    SessionExited,
    #[error("the Agent session is running; only a Suspended or Exited one can be resumed")]
    SessionRunning,
    #[error("the Agent session is stopping or starting; try again in a moment")]
    SessionInTransition,
    #[error("the Agent doesn't support {0}")]
    Unsupported(&'static str),
    #[error("no closed session to reopen")]
    NothingToReopen,
    #[error("`{0}` isn't shown or in use, so its files aren't indexed")]
    NotWatched(PathBuf),
    #[error("`{}` isn't in this Workspace, so it can't be edited here", .0.display())]
    NotEditable(PathBuf),
    #[error("`{}` changed on disk since it was opened", .0.display())]
    FileChangedOnDisk(PathBuf),
    #[error("{0}")]
    File(String),
    #[error("this window has no popped-out file to show")]
    NoPopOut,
    #[error("that session isn't in this Worktree's Recent sessions")]
    UnknownRecentSession,
    #[error("the Agent session isn't waiting for a permission answer")]
    NoPendingPermission,
    #[error("`{0}` isn't one of the options the Agent offered")]
    UnknownPermissionOption(String),
    #[error("the editor has no settings file")]
    NoSettingsFile,
    #[error("couldn't write the settings file: {0}")]
    SettingsWrite(String),
    #[error("this Worktree's setup is still running")]
    SetupRunning,
    #[error("this Worktree's setup failed; Retry it or Start anyway")]
    SetupFailed,
    #[error("this Worktree has no failed setup to retry or skip")]
    NoFailedSetup,
    #[error("the main checkout can't be removed")]
    MainCheckout,
    #[error("this Worktree is being removed")]
    BeingRemoved,
    #[error("an Agent session in this Worktree didn't stop in time; try again")]
    SessionWontStop,
    #[error("removing it would lose {0}; choose Discard and remove to go ahead")]
    WouldDiscard(String),
    #[error(transparent)]
    Acp(#[from] AcpError),
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceInfo {
    /// The main checkout's root.
    pub root: PathBuf,
    pub name: String,
}

/// Small broadcast events. Transcript content is never broadcast; it streams only to watchers.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum CoreEvent {
    SessionCreated {
        session: SessionInfo,
    },
    #[serde(rename_all = "camelCase")]
    SessionStateChanged {
        session_id: SessionId,
        state: SessionState,
    },
    #[serde(rename_all = "camelCase")]
    PermissionModeChanged {
        session_id: SessionId,
        mode: PermissionMode,
    },
    #[serde(rename_all = "camelCase")]
    SessionUnreadChanged {
        session_id: SessionId,
        unread: usize,
    },
    /// The Worktree list or a Worktree's branch/status changed.
    WorktreesChanged {
        worktrees: Vec<WorktreeInfo>,
    },
    /// The settings file was saved: the settings now in force, or why the file was rejected.
    SettingsChanged {
        settings: LoadedSettings,
    },
    /// The session was stopped and is gone (e.g. its Worktree was removed).
    #[serde(rename_all = "camelCase")]
    SessionClosed {
        session_id: SessionId,
    },
    /// Idle sessions were Suspended to free memory or because they'd been Idle a long time (all
    /// of one check's, together).
    AutoSuspended {
        suspended: Vec<AutoSuspension>,
    },
    /// A Worktree's files changed (created, deleted, renamed, or their git status), for the Files
    /// drawer and Ctrl+P.
    FilesChanged {
        worktree: PathBuf,
    },
    /// Watching a Worktree's files failed (the OS's watch limit); it's polled instead.
    FileWatchFallback {
        worktree: PathBuf,
        message: String,
    },
    /// A popped-out window closed before it showed its file: `window` (where it came from) should
    /// reopen it, unsaved changes and all.
    PopOutReturned {
        window: String,
        file: PoppedOutFile,
    },
    /// A file tab in `window`'s Manual editor, with no unsaved changes, has a file that changed on
    /// disk (an Agent edited it, say): the editor reloads it.
    DocumentChangedOnDisk {
        window: String,
        path: PathBuf,
    },
    /// A file tab in `window`'s Manual editor, with unsaved changes, has a file that changed on disk
    /// (or was deleted): the editor asks what to do, and never reloads it by itself.
    DocumentConflicted {
        window: String,
        path: PathBuf,
        deleted: bool,
    },
    /// A file tab told its file changed on disk has it back as the editor has it (the Agent undid
    /// its change, say): nothing to ask any more.
    DocumentBackOnDisk {
        window: String,
        path: PathBuf,
    },
    /// The files open in Manual editors changed: one opened or closed, or got (or lost) unsaved
    /// changes. For permission cards warning about an edit to a file with unsaved changes.
    DocumentsChanged {
        documents: Vec<OpenDocument>,
    },
    /// A Worktree's Recent sessions changed (a Tab was closed or reopened).
    RecentSessionsChanged {
        worktree: PathBuf,
        sessions: Vec<RecentSession>,
    },
    /// A Worktree's setup moved on (to a command, a failure, or its first session).
    SetupChanged {
        worktree: PathBuf,
        /// As the settings file had them when the setup started (or was last retried).
        commands: Vec<String>,
        status: SetupStatus,
    },
    /// Output from the running setup command.
    SetupOutput {
        worktree: PathBuf,
        text: String,
    },
}

/// Batches of transcript changes for one session, flushed about once per frame.
pub struct TranscriptStream {
    rx: mpsc::Receiver<Vec<TranscriptDelta>>,
}

impl TranscriptStream {
    pub async fn next(&mut self) -> Option<Vec<TranscriptDelta>> {
        self.rx.recv().await
    }
}

#[derive(Clone)]
pub struct Core {
    inner: Arc<Inner>,
}

struct Inner {
    config: CoreConfig,
    events: broadcast::Sender<CoreEvent>,
    state: Mutex<State>,
    /// The single shared ACP adapter; serialised so concurrent callers never start two.
    adapter: tokio::sync::Mutex<Option<Arc<Connection>>>,
    /// The session whose Tab is visible, and the switch that ends its stream when another is shown.
    visible_tab: Mutex<Option<(Arc<Session>, oneshot::Sender<()>)>>,
    /// Watches for Worktrees added or removed outside the editor.
    discovery: Mutex<Option<Arc<Discovery>>>,
    /// Serialises Worktree refreshes.
    refresh_lock: tokio::sync::Mutex<()>,
    settings: Mutex<LoadedSettings>,
    /// Reloads the settings file when it's saved; dropping it stops the watch.
    settings_watch: Mutex<Option<notify::RecommendedWatcher>>,
    /// Serialises the editor's own writes to the settings file.
    settings_write: tokio::sync::Mutex<()>,
    /// When the adapter last crashed with sessions to restart (within `CRASH_WINDOW`).
    crashes: Mutex<Vec<std::time::Instant>>,
    /// Numbers each adapter process (`Connection::generation`).
    generations: AtomicU64,
    /// The state saved by the last run, to restore Workspaces from.
    app_state: Mutex<AppState>,
    /// Whether this run may save state (not over a file it couldn't read).
    state_writable: bool,
    /// Serialises saves, so an older snapshot can't be written over a newer one.
    persist_lock: Mutex<()>,
    memory_probe: Arc<dyn MemoryProbe>,
    clock: Arc<dyn Clock>,
    /// The memory monitor runs once a Workspace is open.
    monitoring: AtomicBool,
    /// When auto-suspend last stopped something (by `clock`), for its cooldown.
    last_auto_suspend: Mutex<Option<std::time::Instant>>,
    /// Worktree actors (file index and watching), for the Worktree being looked at and those with
    /// sessions. Each starts (indexes) outside the lock; whoever needs one meanwhile waits for it.
    actors: Mutex<HashMap<PathBuf, ActorSlot>>,
    /// Worktrees whose branch status is due a refresh (coalesced: one per Worktree at a time).
    status_due: Mutex<HashSet<PathBuf>>,
    /// Worktrees whose watch-limit toast has been shown this run (it's shown once).
    fallback_told: Mutex<HashSet<PathBuf>>,
    /// The Worktree being looked at.
    shown_worktree: Mutex<Option<PathBuf>>,
    /// Files open in Manual editors, and popped-out files held for their windows.
    documents: Mutex<DocumentTracker>,
}

#[derive(Default)]
struct State {
    workspace: Option<WorkspaceInfo>,
    worktrees: Vec<WorktreeInfo>,
    /// Worktree setups started by this editor, by Worktree path.
    setups: HashMap<PathBuf, SetupRun>,
    /// Worktrees being removed: no session or setup may start in them.
    removing: std::collections::HashSet<PathBuf>,
    /// Closed Tabs, most recently closed first (Recent sessions, all Worktrees).
    recent: Vec<SavedSession>,
    /// The highest `Session N` name given so far.
    last_name: u64,
    /// The ACP id of the Tab last shown.
    active: Option<String>,
    sessions: HashMap<SessionId, Arc<Session>>,
    by_acp_id: HashMap<String, SessionId>,
    next_session: u64,
}

impl State {
    /// The next `Session N` name.
    fn new_name(&mut self) -> String {
        self.last_name += 1;
        format!("Session {}", self.last_name)
    }

    /// Keeps `new_name` clear of a remembered name.
    fn note_name(&mut self, name: &str) {
        if let Some(n) = name.strip_prefix("Session ").and_then(|n| n.parse().ok()) {
            self.last_name = self.last_name.max(n);
        }
    }
}

struct SetupRun {
    info: SetupInfo,
    shell: WindowsShell,
    /// The task running it, aborted (killing its command) if the Worktree goes away.
    task: Option<tokio::task::JoinHandle<()>>,
}

impl SetupRun {
    fn changed(&self) -> CoreEvent {
        CoreEvent::SetupChanged {
            worktree: self.info.worktree.clone(),
            commands: self.info.commands.clone(),
            status: self.info.status.clone(),
        }
    }

    fn stop(&self) {
        if let Some(task) = &self.task {
            task.abort(); // its command is killed when the task drops it
        }
    }
}

/// Where a claimed setup carries on from.
enum Resume {
    Command(usize),
    Session,
}

struct Session {
    /// What the frontend sees. `state` here is only ever written by `update`, derived from `control`.
    info: Mutex<SessionInfo>,
    acp_id: String,
    /// The adapter connection it lives on (a new one after it's resumed on a restarted adapter).
    connection: Mutex<Option<Arc<Connection>>>,
    transcript: Mutex<Transcript>,
    deltas: broadcast::Sender<TranscriptDelta>,
    events: broadcast::Sender<CoreEvent>,
    /// Whether this session's Tab is the visible one (and so isn't collecting unread items).
    visible: AtomicBool,
    /// `session/load` is replaying its conversation (old news, so not unread).
    replaying: AtomicBool,
    /// Woken whenever a suspend, resume or load settles, for whoever waits on that.
    settled: tokio::sync::Notify,
    /// Since when it's been Idle (by `clock`), for auto-suspend's longest-Idle-first.
    idle_since: Mutex<Option<std::time::Instant>>,
    clock: Arc<dyn Clock>,
    /// Every tool call the Agent announced, merged with its updates, keyed by ACP tool call id.
    tool_calls: Mutex<HashMap<String, KnownToolCall>>,
    control: Mutex<Control>,
}

/// A tool call as merged from the Agent's announcements, and its transcript row once added.
struct KnownToolCall {
    call: Value,
    row: Option<usize>,
}

/// The facts a session's state is derived from, changed only under one lock (see `Session::update`),
/// so turn ends, answers and crashes can't interleave into a wrong state.
#[derive(Default)]
struct Control {
    in_turn: bool,
    exited: bool,
    /// Its Agent process was stopped on purpose; resuming brings the conversation back.
    suspended: bool,
    /// Being brought back (`session/resume`), on request or after the adapter restarted. Whoever
    /// sets it runs `Inner::resume_claimed`, which clears it.
    resuming: bool,
    /// Being Suspended: `session/close` sent, not yet confirmed.
    suspending: bool,
    /// Closed for good (its Worktree is being removed); a resume in flight must not revive it.
    closed: bool,
    /// Its conversation is in the transcript. A Tab restored after a restart starts without it,
    /// and `session/load` (which replays it) brings it in.
    loaded: bool,
    /// A restored Tab's conversation is being loaded to show it (it stays Suspended).
    loading: bool,
    /// Auto-suspend couldn't stop it; it isn't tried again until its next turn or resume.
    auto_suspend_failed: bool,
    /// Loading it to show it failed and the Tab says so (once).
    load_failed: bool,
    /// Permission questions the Agent is waiting on, oldest first.
    questions: Vec<OpenQuestion>,
}

impl Control {
    /// Mid-suspend, mid-resume or loading: nothing else may start until that settles (see
    /// `Session::settled`).
    fn in_transition(&self) -> bool {
        self.resuming || self.suspending || self.loading
    }

    fn state(&self) -> SessionState {
        // (Loading leaves it Suspended: no Agent is put to work by just looking at a Tab.)
        if self.resuming || self.suspending {
            SessionState::Working
        } else if self.exited {
            SessionState::Exited
        } else if !self.questions.is_empty() {
            SessionState::NeedsYou
        } else if self.in_turn {
            SessionState::Working
        } else if self.suspended {
            SessionState::Suspended
        } else {
            SessionState::Idle
        }
    }
}

struct OpenQuestion {
    tool_call_id: String,
    /// The permission card's index in the transcript.
    index: usize,
    option_ids: Vec<String>,
    responder: Responder,
}

impl Core {
    pub fn new(config: CoreConfig) -> Self {
        let (events, _) = broadcast::channel(1024);
        let memory_probe: Arc<dyn MemoryProbe> = config
            .memory_probe
            .clone()
            .unwrap_or_else(|| Arc::new(SystemProbe::default()));
        let clock: Arc<dyn Clock> = config
            .clock
            .clone()
            .unwrap_or_else(|| Arc::new(SystemClock));
        // A state file that's there but can't be read is never saved over (it would lose it).
        let (saved_state, state_writable) = match config.state_path.as_deref().map(app_state::load)
        {
            Some(Ok(saved)) => (saved, true),
            Some(Err(err)) => {
                eprintln!("{err}; open Tabs won't be saved this run");
                (AppState::default(), false)
            }
            None => (AppState::default(), false),
        };
        let loaded = match &config.settings_path {
            Some(path) => settings::apply(&LoadedSettings::default(), settings::read(path)),
            None => LoadedSettings::default(),
        };
        let inner = Arc::new(Inner {
            config,
            events,
            state: Mutex::default(),
            adapter: tokio::sync::Mutex::new(None),
            visible_tab: Mutex::new(None),
            discovery: Mutex::new(None),
            refresh_lock: tokio::sync::Mutex::new(()),
            settings: Mutex::new(loaded),
            settings_watch: Mutex::new(None),
            settings_write: tokio::sync::Mutex::new(()),
            crashes: Mutex::default(),
            generations: AtomicU64::new(1),
            app_state: Mutex::new(saved_state),
            state_writable,
            persist_lock: Mutex::new(()),
            memory_probe,
            clock,
            monitoring: AtomicBool::new(false),
            last_auto_suspend: Mutex::new(None),
            actors: Mutex::default(),
            status_due: Mutex::default(),
            fallback_told: Mutex::default(),
            shown_worktree: Mutex::new(None),
            documents: Mutex::default(),
        });
        if let Some(path) = &inner.config.settings_path {
            let weak = Arc::downgrade(&inner);
            let watch = settings::watch(path, move || match weak.upgrade() {
                Some(inner) => {
                    inner.reload_settings();
                    true
                }
                None => false,
            });
            if watch.is_none() {
                eprintln!(
                    "can't watch {}: settings changes apply after a restart",
                    path.display()
                );
            }
            *inner.settings_watch.lock().expect("settings watch lock") = watch;
            // A save between the first read and the watch starting would otherwise be missed.
            inner.reload_settings();
        }
        Self { inner }
    }

    /// The settings in force, and the settings file's error if it doesn't parse.
    pub fn settings(&self) -> LoadedSettings {
        self.inner.settings.lock().expect("settings lock").clone()
    }

    /// Makes sure the settings file has a section for this repo (adding a template keyed by its
    /// `origin` URL, else its path) and returns the file's path, for the user to edit.
    pub async fn open_repo_settings(&self) -> Result<PathBuf, CoreError> {
        let path = self
            .inner
            .config
            .settings_path
            .clone()
            .ok_or(CoreError::NoSettingsFile)?;
        let workspace = self.workspace()?;
        let origin = git::origin_url(&workspace.root).await;
        // One at a time, or two quick clicks could both add the section (a duplicate table).
        let _one_at_a_time = self.inner.settings_write.lock().await;
        // Read it now: a section saved a moment ago may not have been reloaded yet.
        self.inner.reload_settings();
        let needs_section = {
            let loaded = self.inner.settings.lock().expect("settings lock");
            // A file that doesn't parse is left for the user to fix (it may have the section already).
            loaded.error.is_none()
                && loaded
                    .settings
                    .repo(origin.as_deref(), &workspace.root)
                    .is_none()
        };
        if needs_section {
            let key = origin.unwrap_or_else(|| workspace.root.display().to_string());
            settings::add_repo_section(&path, &key, &workspace.name)
                .map_err(|e| CoreError::SettingsWrite(e.to_string()))?;
            self.inner.reload_settings();
        }
        Ok(path)
    }

    /// The setup this editor ran (or is running) in `worktree`, if any.
    pub fn setup(&self, worktree: &Path) -> Option<SetupInfo> {
        let worktree = worktrees::normalize(worktree.to_owned());
        self.inner
            .state
            .lock()
            .expect("state lock")
            .setups
            .get(&worktree)
            .map(|run| run.info.clone())
    }

    /// Every setup this editor ran (or is running) in this Workspace, for a view opening late.
    pub fn setups(&self) -> Vec<SetupInfo> {
        self.inner
            .state
            .lock()
            .expect("state lock")
            .setups
            .values()
            .map(|run| run.info.clone())
            .collect()
    }

    /// Reruns a failed setup from the command that failed, with the repo's setup commands as the
    /// settings file has them now (so a command fixed there is the one that reruns).
    pub async fn retry_setup(&self, worktree: &Path) -> Result<(), CoreError> {
        let worktree = worktrees::normalize(worktree.to_owned());
        let root = self.workspace()?.root;
        let (commands, shell) = self.repo_setup(&root).await.unwrap_or_default();
        let resume = self.inner.claim_failed_setup(&worktree, |run| {
            let from = match run.info.status {
                SetupStatus::Failed { step, .. } => step,
                _ => commands.len(), // only the session failed
            };
            run.info.commands = commands;
            run.shell = shell;
            if from < run.info.commands.len() {
                Resume::Command(from)
            } else {
                Resume::Session
            }
        })?;
        self.resume_setup(worktree, resume);
        Ok(())
    }

    /// Skips the rest of a failed setup and starts the Worktree's first session.
    pub fn start_anyway(&self, worktree: &Path) -> Result<(), CoreError> {
        let worktree = worktrees::normalize(worktree.to_owned());
        let resume = self
            .inner
            .claim_failed_setup(&worktree, |_| Resume::Session)?;
        self.resume_setup(worktree, resume);
        Ok(())
    }

    /// The repo's setup commands for this OS (none if empty) and shell, from freshly read settings.
    async fn repo_setup(&self, root: &Path) -> Option<(Vec<String>, WindowsShell)> {
        let origin = git::origin_url(root).await;
        // A save a moment ago may not have been reloaded yet.
        self.inner.reload_settings();
        let loaded = self.inner.settings.lock().expect("settings lock");
        let repo = loaded.settings.repo(origin.as_deref(), root)?;
        let commands = repo.setup_commands().to_vec();
        (!commands.is_empty()).then_some((commands, repo.windows_shell))
    }

    /// Runs a claimed setup on from `resume` in the background, keeping a handle to stop it. The
    /// handle is stored under the same lock the task is started under, so a removal can't miss it.
    fn resume_setup(&self, worktree: PathBuf, resume: Resume) {
        let mut state = self.inner.state.lock().expect("state lock");
        let Some(run) = state.setups.get_mut(&worktree) else {
            return;
        };
        let core = self.clone();
        let path = worktree.clone();
        run.task = Some(tokio::spawn(async move {
            match resume {
                Resume::Command(from) => core.run_setup(path, from).await,
                Resume::Session => core.start_first_session(path).await,
            }
        }));
    }

    /// Runs setup commands from `from` on, stopping at the first failure; then starts the session.
    async fn run_setup(&self, worktree: PathBuf, from: usize) {
        let Some((commands, shell)) = self
            .inner
            .state
            .lock()
            .expect("state lock")
            .setups
            .get(&worktree)
            .map(|run| (run.info.commands.clone(), run.shell))
        else {
            return;
        };
        for (step, command) in commands.iter().enumerate().skip(from) {
            self.inner
                .set_setup_status(&worktree, SetupStatus::Running { step });
            self.inner.setup_output(&worktree, format!("> {command}\n"));
            let result = setup::run(command, &worktree, shell, |text| {
                self.inner.setup_output(&worktree, text)
            })
            .await;
            if let Err(message) = result {
                self.inner.setup_output(&worktree, format!("{message}\n"));
                self.inner
                    .set_setup_status(&worktree, SetupStatus::Failed { step, message });
                return;
            }
        }
        self.inner
            .set_setup_status(&worktree, SetupStatus::StartingSession);
        self.start_first_session(worktree).await;
    }

    /// Starts the Worktree's first session on a task of its own: stopping the setup mustn't drop a
    /// `session/new` half way (the Agent would start one nobody closes).
    async fn start_first_session(&self, worktree: PathBuf) {
        let core = self.clone();
        let _ = tokio::spawn(async move { core.open_first_session(worktree).await }).await;
    }

    async fn open_first_session(&self, worktree: PathBuf) {
        let status = match self.start_session_in(&worktree).await {
            Ok(session_id) => SetupStatus::Done { session_id },
            Err(err) => {
                let message = format!("the Agent session didn't start: {err}");
                self.inner.setup_output(&worktree, format!("{message}\n"));
                SetupStatus::SessionFailed { message }
            }
        };
        self.inner.set_setup_status(&worktree, status);
    }
    pub fn subscribe(&self) -> broadcast::Receiver<CoreEvent> {
        self.inner.events.subscribe()
    }

    /// Opens the git repository containing `path` as the Workspace, lists its Worktrees and starts
    /// watching for Worktrees added or removed elsewhere.
    pub async fn open_workspace(&self, path: &Path) -> Result<WorkspaceInfo, CoreError> {
        let toplevel = git::toplevel(path)
            .await
            .ok_or_else(|| CoreError::NotARepository(path.to_owned()))?;
        // The main checkout, even if `path` is inside another Worktree.
        let root = match git::worktree_list(&toplevel)
            .await
            .and_then(|w| w.into_iter().next())
        {
            Some(main) => worktrees::normalize(main.path),
            None => worktrees::normalize(toplevel),
        };
        let name = root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let workspace = WorkspaceInfo { root, name };
        let listed = worktrees::list(&workspace.root).await;
        {
            let mut state = self.inner.state.lock().expect("state lock");
            state.workspace = Some(workspace.clone());
            state.worktrees = listed;
            // Setups belong to the previous Workspace's Worktrees.
            for (_, run) in state.setups.drain() {
                run.stop();
            }
        }
        self.inner.restore(&workspace.root);
        self.inner.watch_worktrees(&workspace.root).await;
        self.inner.update_actors().await;
        self.start_monitor();
        Ok(workspace)
    }

    /// Checks the Agents' memory and Idle times every `MONITOR_INTERVAL` from now on.
    fn start_monitor(&self) {
        if self.inner.monitoring.swap(true, Ordering::SeqCst) {
            return;
        }
        let weak = Arc::downgrade(&self.inner);
        tokio::spawn(async move {
            let mut ticks = tokio::time::interval(MONITOR_INTERVAL);
            ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            ticks.tick().await; // (the first tick is immediate)
            loop {
                ticks.tick().await;
                let Some(inner) = weak.upgrade() else {
                    return; // the core is gone
                };
                Core { inner }.check_auto_suspend().await;
            }
        });
    }

    /// One tick of the memory monitor (run every 10 s; public so a check can be run on demand):
    /// suspends Idle sessions as the settings say, longest-Idle first while the `claude` processes
    /// are over the memory limit or the OS is low on memory, and any Idle past the idle
    /// auto-suspend time. Working and Needs you sessions are never touched. Announces what it did
    /// with one `AutoSuspended`.
    pub async fn check_auto_suspend(&self) {
        let (candidates, running) = {
            let state = self.inner.state.lock().expect("state lock");
            let mut candidates = vec![];
            let mut running = 0;
            for session in state.sessions.values() {
                let info = session.info.lock().expect("info lock");
                if matches!(
                    info.state,
                    SessionState::Idle | SessionState::Working | SessionState::NeedsYou
                ) {
                    running += 1;
                }
                let blocked = session
                    .control
                    .lock()
                    .expect("control lock")
                    .auto_suspend_failed;
                let idle_since = *session.idle_since.lock().expect("idle lock");
                if let (SessionState::Idle, Some(idle_since), false) =
                    (info.state, idle_since, blocked)
                {
                    candidates.push(Candidate {
                        id: info.id,
                        idle_since,
                    });
                }
            }
            (candidates, running)
        };
        if candidates.is_empty() {
            return;
        }
        // Not while the adapter is starting (that can take a while): next time.
        let Ok(adapter) = self.inner.adapter.try_lock() else {
            return;
        };
        let adapter_pid = adapter
            .as_ref()
            .filter(|c| !c.is_closed())
            .and_then(|c| c.pid());
        drop(adapter);
        let probe = self.inner.memory_probe.clone();
        let Ok(sample) = tokio::task::spawn_blocking(move || probe.sample(adapter_pid)).await
        else {
            return;
        };
        let now = self.inner.clock.now();
        // Just after suspending, the reading may still include the processes being closed.
        let cooling_down = self
            .inner
            .last_auto_suspend
            .lock()
            .expect("auto-suspend lock")
            .is_some_and(|at| now.saturating_duration_since(at) < AUTO_SUSPEND_COOLDOWN);
        let agents = self.settings().settings.agents;
        let limits = Limits {
            // 0 means no limit, as does an idle time of 0 (rather than suspending everything).
            memory_limit_bytes: (agents.memory_limit_mb > 0 && !cooling_down)
                .then(|| agents.memory_limit_mb.saturating_mul(1024 * 1024)),
            low_memory: !cooling_down,
            idle_after: (agents.idle_suspend && agents.idle_suspend_minutes > 0)
                .then(|| Duration::from_secs(agents.idle_suspend_minutes.saturating_mul(60))),
        };
        let chosen = auto_suspend::choose(&candidates, running, sample, limits, now);
        let mut suspended = vec![];
        for (id, reason) in chosen {
            let idle_since = candidates.iter().find(|c| c.id == id).map(|c| c.idle_since);
            if let Some(session) = self.auto_suspend(id, idle_since).await {
                suspended.push(AutoSuspension { session, reason });
            }
        }
        if !suspended.is_empty() {
            *self
                .inner
                .last_auto_suspend
                .lock()
                .expect("auto-suspend lock") = Some(now);
            let _ = self
                .inner
                .events
                .send(CoreEvent::AutoSuspended { suspended });
        }
    }

    /// Suspends a session chosen while Idle since `idle_since`, if it still is (not used since);
    /// its info if it did. A session whose Agent wouldn't stop isn't tried again until its next
    /// turn, so a failing suspend can't repeat every tick.
    async fn auto_suspend(
        &self,
        id: SessionId,
        idle_since: Option<std::time::Instant>,
    ) -> Option<SessionInfo> {
        let session = self.session(id).ok()?;
        if *session.idle_since.lock().expect("idle lock") != idle_since {
            return None;
        }
        let stopped = self.suspend_session(id).await;
        let info = session.info.lock().expect("info lock").clone();
        if info.state == SessionState::Suspended {
            return Some(info);
        }
        if stopped.is_err() {
            session.update(|control| control.auto_suspend_failed = true);
        }
        None
    }
    /// The Workspace's Worktrees, main checkout first.
    pub fn worktrees(&self) -> Vec<WorktreeInfo> {
        self.inner
            .state
            .lock()
            .expect("state lock")
            .worktrees
            .clone()
    }

    /// Re-lists the Worktrees and their status (e.g. when the window regains focus), emitting
    /// `WorktreesChanged` if anything differs.
    pub async fn refresh_worktrees(&self) {
        self.inner.refresh_worktrees().await;
    }

    /// Creates a Worktree next to the repo (`<repo>.worktrees/<folder>/`) and returns it, listed.
    /// A new branch starts from `start_point`, or by default from `origin/<default>` after a fetch.
    pub async fn create_worktree(&self, spec: NewWorktree) -> Result<CreatedWorktree, CoreError> {
        let root = self.workspace()?.root;
        let (path, warning) = create_worktree::create(&root, &spec).await?;
        self.inner.refresh_worktrees().await;
        let path = worktrees::normalize(path);
        let worktree = self
            .worktrees()
            .into_iter()
            .find(|w| w.path == path)
            .ok_or(CoreError::WorktreeNotListed(path))?;
        let setup = self.start_setup(&root, &worktree.path).await;
        Ok(CreatedWorktree {
            worktree,
            warning,
            setup,
        })
    }

    /// Starts the repo's Worktree setup in a new Worktree, if it has one.
    async fn start_setup(&self, root: &Path, worktree: &Path) -> Option<SetupInfo> {
        let (commands, shell) = self.repo_setup(root).await?;
        let info = SetupInfo {
            worktree: worktree.to_owned(),
            commands,
            status: SetupStatus::Running { step: 0 },
            output: String::new(),
        };
        self.inner.state.lock().expect("state lock").setups.insert(
            worktree.to_owned(),
            SetupRun {
                info: info.clone(),
                shell,
                task: None,
            },
        );
        self.resume_setup(worktree.to_owned(), Resume::Command(0));
        Some(info)
    }

    /// What removing `worktree` would stop and lose, for the confirmation dialog.
    pub async fn removal_check(&self, worktree: &Path) -> Result<RemovalCheck, CoreError> {
        let (root, info) = self.removable(worktree)?;
        self.inspect_for_removal(&root, &info).await
    }

    /// Removes a Worktree: stops its setup and Agent sessions first, then `git worktree remove`.
    /// Refuses (before stopping anything) if that would lose work the user didn't choose to
    /// discard; "Delete branch too" deletes the local branch afterwards. While it runs, no session
    /// or setup starts in the Worktree.
    pub async fn remove_worktree(
        &self,
        worktree: &Path,
        options: RemoveWorktree,
    ) -> Result<RemovedWorktree, CoreError> {
        let (root, info) = self.removable(worktree)?;
        refuse_loss(&self.inspect_for_removal(&root, &info).await?, &options)?;
        if !self
            .inner
            .state
            .lock()
            .expect("state lock")
            .removing
            .insert(info.path.clone())
        {
            return Err(CoreError::BeingRemoved);
        }
        let removed = self.stop_and_remove(&root, &info, &options).await;
        {
            let mut state = self.inner.state.lock().expect("state lock");
            state.removing.remove(&info.path);
            if removed.is_ok() {
                // Nothing to reopen them in now.
                state.recent.retain(|s| s.worktree != info.path);
            }
        }
        self.inner.refresh_worktrees().await;
        if removed.is_ok() {
            self.inner.recent_changed(&info.path);
        }
        self.inner.persist();
        // Removed: its actor stays stopped; refused or failed: it's watched again.
        self.inner.update_actors().await;
        removed
    }

    async fn stop_and_remove(
        &self,
        root: &Path,
        info: &WorktreeInfo,
        options: &RemoveWorktree,
    ) -> Result<RemovedWorktree, CoreError> {
        self.stop_setup(&info.path).await;
        for session in self.sessions_in(&info.path) {
            self.close_session(session).await?;
        }
        // Its files' watch holds the folder open (on Windows, enough to stop it being deleted).
        self.inner
            .actors
            .lock()
            .expect("actors lock")
            .remove(&info.path);
        // An Agent may have changed something before it stopped.
        let check = self.inspect_for_removal(root, info).await?;
        refuse_loss(&check, options)?;
        // git needs --force only to delete uncommitted changes the user chose to discard.
        let force = options.discard.is_some() && check.changed_count > 0;
        let mut warnings: Vec<String> = self
            .remove_folder(root, &info.path, force)
            .await?
            .into_iter()
            .collect();
        if let (true, Some(branch)) = (options.delete_branch, &info.branch) {
            if let Err(err) = git::delete_branch(root, branch).await {
                warnings.push(format!("Its branch `{branch}` couldn't be deleted: {err}"));
            }
        }
        Ok(RemovedWorktree {
            warning: (!warnings.is_empty()).then(|| warnings.join(" ")),
        })
    }

    /// `git worktree remove`, retried while Windows still holds the folder for a moment after the
    /// processes in it exited. git may unregister the Worktree and delete its files before failing
    /// on the folder itself; then only that emptied folder is left, and failing to delete it is a
    /// warning (the Worktree is gone) rather than an error.
    async fn remove_folder(
        &self,
        root: &Path,
        path: &Path,
        force: bool,
    ) -> Result<Option<String>, CoreError> {
        let mut attempt = 0;
        let mut git_ran = false;
        loop {
            let Some(listed) = git::worktree_list(root).await else {
                return Err(CoreError::Git("couldn't list the Worktrees".into()));
            };
            let listed: Vec<PathBuf> = listed
                .into_iter()
                .map(|w| worktrees::normalize(w.path))
                .collect();
            if listed.iter().any(|w| w != path && w.starts_with(path)) {
                return Err(CoreError::Git(format!(
                    "another Worktree is inside {}; remove that one first",
                    path.display()
                )));
            }
            let registered = listed.iter().any(|w| w == path);
            let removed = if registered {
                git_ran = true;
                git::worktree_remove(root, path, force).await
            } else if !git_ran || path.join(".git").exists() {
                // Not git's half-finished removal (that deletes `.git` first): leave it alone.
                return Err(CoreError::Git(format!(
                    "{} is no longer a Worktree of this repository",
                    path.display()
                )));
            } else {
                match std::fs::remove_dir_all(path) {
                    Err(err) if err.kind() != std::io::ErrorKind::NotFound => {
                        Err(format!("couldn't delete {}: {err}", path.display()))
                    }
                    _ => Ok(()),
                }
            };
            match removed {
                Ok(()) => return Ok(None),
                Err(err) if attempt < REMOVE_RETRIES && path.exists() && in_use(&err) => {
                    attempt += 1;
                    tokio::time::sleep(REMOVE_RETRY_DELAY).await;
                }
                Err(err) if !registered => {
                    return Ok(Some(format!(
                        "git no longer has the Worktree, but its emptied folder is still in use ({err}); delete {} when nothing is using it.",
                        path.display()
                    )))
                }
                Err(err) => return Err(CoreError::Git(err)),
            }
        }
    }

    /// The Workspace's root and the linked (not main) Worktree at `worktree`.
    fn removable(&self, worktree: &Path) -> Result<(PathBuf, WorktreeInfo), CoreError> {
        let root = self.workspace()?.root;
        let wanted = worktrees::normalize(worktree.to_owned());
        let info = self
            .worktrees()
            .into_iter()
            .find(|w| w.path == wanted)
            .ok_or_else(|| CoreError::UnknownWorktree(worktree.to_owned()))?;
        if info.is_main {
            return Err(CoreError::MainCheckout);
        }
        Ok((root, info))
    }

    async fn inspect_for_removal(
        &self,
        root: &Path,
        info: &WorktreeInfo,
    ) -> Result<RemovalCheck, CoreError> {
        let base = git::default_start_point(root).await;
        // By commit id, resolved in the main checkout: in the Worktree, a Base of `HEAD` (or a name
        // its own branch shadows) would mean its own HEAD, and everything would look merged.
        let base_id = git::resolve_commit(root, &base)
            .await
            .ok_or_else(|| CoreError::Git(format!("couldn't find the Base `{base}`")))?;
        let mut check =
            remove_worktree::inspect(&info.path, info.branch.as_deref(), &base, &base_id)
                .await
                .map_err(CoreError::Git)?;
        check.sessions = self
            .sessions_in(&info.path)
            .iter()
            .map(|s| s.info.lock().expect("info lock").id)
            .collect();
        check.sessions.sort();
        Ok(check)
    }

    fn sessions_in(&self, worktree: &Path) -> Vec<Arc<Session>> {
        self.inner
            .state
            .lock()
            .expect("state lock")
            .sessions
            .values()
            .filter(|s| s.info.lock().expect("info lock").worktree == worktree)
            .cloned()
            .collect()
    }

    /// Stops a Worktree's setup (with everything its command started) and forgets it. A first
    /// session it's starting finishes on its own task, and is refused or closed (see `removing`).
    async fn stop_setup(&self, worktree: &Path) {
        let run = self
            .inner
            .state
            .lock()
            .expect("state lock")
            .setups
            .remove(worktree);
        if let Some(task) = run.and_then(|run| run.task) {
            task.abort();
            let _ = task.await; // returns once the command has been dropped, and so killed
        }
    }

    /// Ends an Agent session for good: open questions are cancelled, the Agent closes it (its Claude
    /// process exits), and it's gone from the editor. One mid-resume is closed by the resume when it
    /// lands (see `resume_claimed`). If the Agent doesn't confirm in time, the session stays (Exited)
    /// and removal stops, rather than deleting a folder it's still in.
    async fn close_session(&self, session: Arc<Session>) -> Result<(), CoreError> {
        if !self.stop_agent(&session).await {
            session.update(|control| control.closed = false);
            return Err(CoreError::SessionWontStop);
        }
        self.forget_session(&session);
        Ok(())
    }

    /// Stops a session's Agent for good (cancelling a turn or questions first) and marks it closed,
    /// so nothing revives it. Whether the Agent confirmed (or had nothing running): one mid-resume
    /// is closed by the resume when it lands (see `resume_claimed`).
    async fn stop_agent(&self, session: &Session) -> bool {
        let running = session.update(|control| {
            session.cancel_questions(control);
            control.in_turn = false;
            let running = !control.suspended && !control.exited && !control.resuming;
            control.exited = true;
            control.closed = true;
            running
        });
        let Some(connection) = session.connection().filter(|_| running) else {
            return true;
        };
        // Stop a turn in progress first (closing does too, but this works on any ACP agent).
        let _ = connection.notify("session/cancel", json!({ "sessionId": session.acp_id }));
        // An error reply (e.g. the adapter already dropped it, or exited) means it's gone too.
        connection.supports_session("close")
            && close_acp(&connection, &session.acp_id).await.is_some()
    }

    /// Drops a closed session from the editor and ends its Tab's stream.
    fn forget_session(&self, session: &Arc<Session>) {
        let id = session.info.lock().expect("info lock").id;
        {
            let mut state = self.inner.state.lock().expect("state lock");
            state.sessions.remove(&id);
            state.by_acp_id.remove(&session.acp_id);
        }
        let mut visible = self.inner.visible_tab.lock().expect("visible tab lock");
        if visible
            .as_ref()
            .is_some_and(|(shown, _)| Arc::ptr_eq(shown, session))
        {
            visible.take(); // ends its stream
        }
        drop(visible);
        let _ = self
            .inner
            .events
            .send(CoreEvent::SessionClosed { session_id: id });
        self.inner.actors_may_change();
    }
    /// Local and remote branches, for the "existing branch" picker. Fetches first, so a colleague's
    /// branch pushed a minute ago is there; if that fails, it says so and lists what was last fetched.
    pub async fn branches(&self) -> Result<BranchList, CoreError> {
        let root = self.workspace()?.root;
        let warning = git::fetch_origin(&root)
            .await
            .err()
            .map(|err| format!("Couldn't fetch origin ({err}); showing branches as last fetched."));
        let branches = git::branches(&root)
            .await
            .map_err(CoreError::Git)?
            .into_iter()
            .map(|b| BranchInfo {
                checked_out_in: b.checked_out_in.map(worktrees::normalize),
                ..b
            })
            .collect();
        Ok(BranchList { branches, warning })
    }

    /// A free `agent/task-N` name (not taken locally or on a remote) for a new Worktree's branch.
    pub async fn suggest_branch_name(&self) -> Result<String, CoreError> {
        let root = self.workspace()?.root;
        let branches = git::branches(&root).await.map_err(CoreError::Git)?;
        Ok(create_worktree::suggest_name(&branches))
    }

    /// Where a new branch starts by default: `origin/<default>`, else the main checkout's branch.
    pub async fn default_start_point(&self) -> Result<String, CoreError> {
        Ok(git::default_start_point(&self.workspace()?.root).await)
    }
    /// Starts a new Agent session in the Workspace's main checkout.
    pub async fn new_session(&self) -> Result<SessionId, CoreError> {
        let root = self.workspace()?.root;
        self.new_session_in(&root).await
    }

    /// Starts a new Agent session in one of the Workspace's Worktrees. A Worktree whose setup hasn't
    /// finished gets its first session from the setup (or Start anyway), not from here.
    pub async fn new_session_in(&self, worktree: &Path) -> Result<SessionId, CoreError> {
        let wanted = worktrees::normalize(worktree.to_owned());
        let status = self
            .inner
            .state
            .lock()
            .expect("state lock")
            .setups
            .get(&wanted)
            .map(|run| run.info.status.clone());
        match status {
            Some(SetupStatus::Running { .. } | SetupStatus::StartingSession) => {
                Err(CoreError::SetupRunning)
            }
            Some(SetupStatus::Failed { .. } | SetupStatus::SessionFailed { .. }) => {
                Err(CoreError::SetupFailed)
            }
            Some(SetupStatus::Done { .. }) | None => self.start_session_in(worktree).await,
        }
    }

    async fn start_session_in(&self, worktree: &Path) -> Result<SessionId, CoreError> {
        self.workspace()?;
        let wanted = worktrees::normalize(worktree.to_owned());
        let root = self
            .worktrees()
            .into_iter()
            .map(|w| w.path)
            .find(|path| *path == wanted)
            .ok_or_else(|| CoreError::UnknownWorktree(worktree.to_owned()))?;
        let being_removed = || {
            self.inner
                .state
                .lock()
                .expect("state lock")
                .removing
                .contains(&root)
        };
        if being_removed() {
            return Err(CoreError::BeingRemoved);
        }
        let connection = self.adapter().await?;
        let created = connection
            .request("session/new", json!({ "cwd": root, "mcpServers": [] }))
            .await?;
        let acp_id = created["sessionId"].as_str().unwrap_or_default().to_owned();
        // Every Tab starts in Ask for edits, whatever the Agent's own settings default to.
        let current_mode = created["modes"]["currentModeId"].as_str();
        if current_mode.is_some_and(|mode| mode != PermissionMode::AskForEdits.acp_id()) {
            apply_mode(&connection, &acp_id, PermissionMode::AskForEdits).await?;
        }

        // Registered under the same lock `remove_worktree` marks the Worktree under, so a session
        // either is in `sessions_in` for removal to close, or is refused here.
        let added = {
            let mut state = self.inner.state.lock().expect("state lock");
            if state.removing.contains(&root) {
                None
            } else {
                let saved = SavedSession {
                    acp_id: acp_id.clone(),
                    name: state.new_name(),
                    worktree: root,
                    permission_mode: PermissionMode::AskForEdits,
                };
                Some(
                    self.inner
                        .register_session(&mut state, saved, Some(connection.clone())),
                )
            }
        };
        let Some(info) = added else {
            // Removal began while the Agent was starting it: it mustn't outlive the folder.
            let _ = close_acp(&connection, &acp_id).await;
            return Err(CoreError::BeingRemoved);
        };
        let id = info.id;
        let _ = self
            .inner
            .events
            .send(CoreEvent::SessionCreated { session: info });
        self.inner.persist();
        self.inner.actors_may_change();
        Ok(id)
    }

    /// The open Tabs' sessions, in Tab order (e.g. as restored after a restart).
    pub fn sessions(&self) -> Vec<SessionInfo> {
        let state = self.inner.state.lock().expect("state lock");
        let mut sessions: Vec<_> = state
            .sessions
            .values()
            .map(|s| s.info.lock().expect("info lock").clone())
            .collect();
        sessions.sort_by_key(|s| s.id);
        sessions
    }

    /// The Worktree being looked at: its files are indexed and watched (as are those of
    /// Worktrees with sessions); other Worktrees' actors stop.
    pub async fn show_worktree(&self, worktree: &Path) -> Result<(), CoreError> {
        let wanted = worktrees::normalize(worktree.to_owned());
        if !self.worktrees().iter().any(|w| w.path == wanted) {
            return Err(CoreError::UnknownWorktree(worktree.to_owned()));
        }
        *self
            .inner
            .shown_worktree
            .lock()
            .expect("shown worktree lock") = Some(wanted);
        self.inner.update_actors().await;
        Ok(())
    }

    /// Ctrl+P: the Worktree's files best matching `query` (typos allowed, ranked below exact
    /// matches), at most `limit`. Empty until the Worktree has been shown.
    pub async fn find_files(
        &self,
        worktree: &Path,
        query: &str,
        limit: usize,
    ) -> Result<Vec<FileMatch>, CoreError> {
        Ok(self.actor(worktree).await?.find(query, limit))
    }

    /// The Files drawer: a folder of the Worktree (relative, `""` for its root), folders first,
    /// with changed files marked.
    pub async fn list_dir(&self, worktree: &Path, dir: &str) -> Result<Vec<DirEntry>, CoreError> {
        Ok(self.actor(worktree).await?.list(dir))
    }

    /// How the Worktree's files are being followed, if they are.
    pub async fn file_watch(&self, worktree: &Path) -> Option<WatchStatus> {
        self.actor(worktree).await.ok().map(|actor| actor.status())
    }

    /// Opens a file for the Manual editor: one of the Workspace's Worktrees', or the settings file.
    pub async fn read_file(&self, path: &Path) -> Result<OpenedFile, CoreError> {
        let path = self.editable(path)?;
        tokio::task::spawn_blocking(move || documents::read(&path))
            .await
            .map_err(|e| CoreError::File(e.to_string()))?
            .map_err(CoreError::File)
    }

    /// Saves a Manual editor's text (`\n` line endings, written as `line_ending`) if `over`
    /// allows: only over the version it read, or over anything once the user said so. Refused
    /// with `FileChangedOnDisk` if the file changed since. The file's new version.
    pub async fn save_file(
        &self,
        path: &Path,
        text: &str,
        line_ending: &str,
        over: SaveOver,
        window: &str,
    ) -> Result<String, CoreError> {
        let path = self.editable(path)?;
        let is_settings = self
            .inner
            .config
            .settings_path
            .as_ref()
            .is_some_and(|s| worktrees::normalize(s.clone()) == path);
        // The editor's own writes to the settings file (a new repo section) wait for this one.
        let _settings_write = match is_settings {
            true => Some(self.inner.settings_write.lock().await),
            false => None,
        };
        let target = path.clone();
        let text = text.to_owned();
        let line_ending = line_ending.to_owned();
        // (While it writes, the file watcher's news of it isn't an Agent's change.)
        self.inner.documents().save_started(&path);
        let saved = tokio::task::spawn_blocking(move || {
            documents::save(&target, &text, &line_ending, &over).map_err(|err| match err {
                documents::SaveError::Changed => CoreError::FileChangedOnDisk(target.clone()),
                documents::SaveError::Io(message) => CoreError::File(message),
            })
        })
        .await
        .map_err(|e| CoreError::File(e.to_string()))
        .and_then(|saved| saved);
        {
            let mut docs = self.inner.documents();
            if let Ok(version) = &saved {
                // The tracker knows the version the editor now has (whatever the frontend says later).
                docs.saved(&path, window, version);
            }
            docs.save_finished(&path);
        }
        self.inner.documents_changed();
        self.inner.check_document(&path);
        // The settings apply as soon as they're saved (the file watch would catch it a moment later).
        if saved.is_ok() && is_settings {
            self.inner.reload_settings();
        }
        saved
    }

    /// The files open in Manual editors, wherever they are.
    pub fn open_documents(&self) -> Vec<OpenDocument> {
        self.inner.documents().all()
    }

    /// A Manual editor in `window` opened `path` at `version`.
    /// Its Worktree's files are watched from now on (for changes on disk), and the file is checked at
    /// once: it may have changed since it was read.
    pub async fn document_opened(
        &self,
        path: &Path,
        window: &str,
        version: &str,
    ) -> Result<(), CoreError> {
        let path = self.editable(path)?;
        self.inner.documents().opened(path.clone(), window, version);
        self.inner.documents_changed();
        self.inner.update_actors().await;
        self.inner.check_document(&path);
        Ok(())
    }

    /// The diff from `text` (a Manual editor's) to the file on disk now.
    pub async fn diff_with_disk(
        &self,
        path: &Path,
        text: &str,
    ) -> Result<Vec<crate::session::DiffLine>, CoreError> {
        let path = self.editable(path)?;
        let opened = tokio::task::spawn_blocking(move || documents::read(&path))
            .await
            .map_err(|e| CoreError::File(e.to_string()))?
            .map_err(CoreError::File)?;
        match opened.content {
            documents::FileContent::Text { text: on_disk, .. } => Ok(crate::diff::unified_diff(
                text,
                &on_disk,
                MAX_VIEW_DIFF_LINES,
            )),
            _ => Err(CoreError::File(format!(
                "{} isn't text on disk, so there's no diff to show",
                opened.path.display()
            ))),
        }
    }

    /// The file in `window` now has (or no longer has) unsaved changes.
    pub fn document_changed(&self, path: &Path, window: &str, dirty: bool) {
        let path = self.tracked_path(path);
        self.inner.documents().changed(&path, window, dirty);
        self.inner.documents_changed();
    }

    /// A Manual editor in `window` closed `path`.
    pub fn document_closed(&self, path: &Path, window: &str) {
        let path = self.tracked_path(path);
        self.inner.documents().closed(&path, window);
        self.inner.documents_changed();
    }

    /// The windows `path` is open in.
    pub fn windows_with(&self, path: &Path) -> Vec<String> {
        let path = self.tracked_path(path);
        self.inner.documents().windows_with(&path)
    }

    /// Moves a file from `from`'s Manual editor into a new window (whose label this returns); the
    /// core holds it, unsaved changes and all, until that window is gone.
    pub fn pop_out(&self, from: &str, file: PoppedOutFile) -> Result<String, CoreError> {
        let path = self.editable(&file.path)?;
        let window = self
            .inner
            .documents()
            .pop_out(from, PoppedOutFile { path, ..file });
        self.inner.documents_changed();
        Ok(window)
    }

    /// The popped-out `window` collects its file (again after a reload: the latest copy), and hears
    /// afresh about changes to it on disk.
    pub async fn collect_pop_out(&self, window: &str) -> Result<PoppedOutFile, CoreError> {
        let file = self.inner.documents().collect(window);
        self.inner.documents_changed();
        let file = file.ok_or(CoreError::NoPopOut)?;
        self.inner.update_actors().await;
        self.inner.check_document(&file.path);
        Ok(file)
    }

    /// The popped-out `window`'s file as it is now (so reloading the window loses nothing).
    pub fn update_pop_out(&self, window: &str, text: String, saved_text: String, version: String) {
        self.inner
            .documents()
            .update_pop_out(window, text, saved_text, version);
    }

    /// The popped-out window couldn't be opened: the file stays where it was.
    pub fn cancel_pop_out(&self, window: &str) {
        self.inner.documents().cancel_pop_out(window);
        self.inner.documents_changed();
    }

    /// `window`'s page is (re)loading: nothing is open in it until its editors say so again.
    pub fn page_loading(&self, window: &str) {
        self.inner.documents().page_loading(window);
        self.inner.documents_changed();
    }

    /// `window` closed: its files aren't open any more. A popped-out file it never collected comes
    /// back (`PopOutReturned`) rather than being lost.
    pub fn window_closed(&self, window: &str) {
        let returned = self.inner.documents().window_closed(window);
        self.inner.documents_changed();
        if let Some((window, file)) = returned {
            let _ = self
                .inner
                .events
                .send(CoreEvent::PopOutReturned { window, file });
        }
    }

    /// The tracked form of `path` (canonical when it can be; else as given, so a file whose
    /// Worktree has gone can still be closed).
    fn tracked_path(&self, path: &Path) -> PathBuf {
        self.editable(path)
            .unwrap_or_else(|_| worktrees::normalize(path.to_owned()))
    }
    /// `path`, canonical, if it's a file the Manual editor may open: inside one of the Workspace's
    /// Worktrees (not their `.git`; no `..` escapes, no symlinks out, none dangling), or the
    /// settings file. Anything that can't be resolved is refused.
    fn editable(&self, path: &Path) -> Result<PathBuf, CoreError> {
        let refused = || CoreError::NotEditable(path.to_owned());
        let canonical = match path.canonicalize() {
            Ok(canonical) => worktrees::normalize(canonical),
            // Not there (deleted meanwhile, say), and not a dangling link: its folder, resolved.
            Err(_) if std::fs::symlink_metadata(path).is_err() => {
                let parent = path
                    .parent()
                    .ok_or_else(refused)?
                    .canonicalize()
                    .map_err(|_| refused())?;
                worktrees::normalize(parent).join(path.file_name().ok_or_else(refused)?)
            }
            Err(_) => return Err(refused()),
        };
        let settings = self
            .inner
            .config
            .settings_path
            .as_ref()
            .map(|s| worktrees::normalize(s.clone()));
        if settings.as_ref() == Some(&canonical) {
            return Ok(canonical);
        }
        let in_worktree = self.worktrees().into_iter().find_map(|w| {
            canonical
                .strip_prefix(&w.path)
                .ok()
                .map(|rel| rel.components().all(|c| c.as_os_str() != ".git"))
        });
        match in_worktree {
            Some(true) => Ok(canonical),
            _ => Err(refused()),
        }
    }
    /// The Worktree's index: how many files, and how often it's been re-read in full.
    pub async fn file_index_stats(&self, worktree: &Path) -> Option<IndexStats> {
        self.actor(worktree).await.ok().map(|actor| actor.stats())
    }

    /// The Worktree's actor, waiting for its first index if it's still starting.
    async fn actor(&self, worktree: &Path) -> Result<Arc<WorktreeActor>, CoreError> {
        let wanted = worktrees::normalize(worktree.to_owned());
        let slot = self
            .inner
            .actors
            .lock()
            .expect("actors lock")
            .get(&wanted)
            .cloned()
            .ok_or_else(|| CoreError::NotWatched(worktree.to_owned()))?;
        Ok(self.inner.start_actor(&wanted, &slot).await)
    }

    /// The Tab shown last (before the editor last closed, if it's been restored), to show again.
    pub fn last_active_session(&self) -> Option<SessionId> {
        let state = self.inner.state.lock().expect("state lock");
        state
            .active
            .as_ref()
            .and_then(|acp_id| state.by_acp_id.get(acp_id))
            .copied()
    }

    /// A Worktree's Recent sessions (closed Tabs), most recently closed first.
    pub fn recent_sessions(&self, worktree: &Path) -> Vec<RecentSession> {
        let worktree = worktrees::normalize(worktree.to_owned());
        self.inner.recent_in(&worktree)
    }

    /// Closes a Tab without losing its conversation: the Agent process stops, and the session
    /// goes to its Worktree's Recent sessions (to reopen later, even after a restart).
    pub async fn close_tab(&self, id: SessionId) -> Result<(), CoreError> {
        let session = self.session(id)?;
        session.wait_settled().await;
        if session
            .control
            .lock()
            .expect("control lock")
            .in_transition()
        {
            return Err(CoreError::SessionInTransition);
        }
        // A slow close only means the process lingers a while; the Tab goes either way.
        let _ = self.stop_agent(&session).await;
        let saved = session.saved();
        self.forget_session(&session);
        {
            let mut state = self.inner.state.lock().expect("state lock");
            app_state::remember_closed(&mut state.recent, saved.clone());
        }
        self.inner.recent_changed(&saved.worktree);
        self.inner.persist();
        Ok(())
    }

    /// Reopens one of the Recent sessions in a new Tab, with its conversation.
    pub async fn reopen_session(&self, acp_id: &str) -> Result<SessionId, CoreError> {
        let saved = {
            let mut state = self.inner.state.lock().expect("state lock");
            let at = state
                .recent
                .iter()
                .position(|s| s.acp_id == acp_id)
                .ok_or(CoreError::UnknownRecentSession)?;
            let worktree = state.recent[at].worktree.clone();
            if !state.worktrees.iter().any(|w| w.path == worktree) {
                return Err(CoreError::UnknownWorktree(worktree));
            }
            if state.removing.contains(&worktree) {
                return Err(CoreError::BeingRemoved);
            }
            state.recent.remove(at)
        };
        self.reopen(saved).await
    }

    /// Reopens the most recently closed session whose Worktree is still there (`Ctrl+Shift+T`);
    /// `None` if there's nothing to reopen.
    pub async fn reopen_last_closed(&self) -> Result<Option<SessionId>, CoreError> {
        let saved = {
            let mut state = self.inner.state.lock().expect("state lock");
            let at = state.recent.iter().position(|s| {
                state.worktrees.iter().any(|w| w.path == s.worktree)
                    && !state.removing.contains(&s.worktree)
            });
            match at {
                Some(at) => state.recent.remove(at),
                None => return Ok(None),
            }
        };
        self.reopen(saved).await.map(Some)
    }

    /// A closed session back in a Tab: registered already resuming (so nothing else can claim it
    /// first), then its conversation is loaded. If that fails it stays, Suspended, saying why.
    async fn reopen(&self, saved: SavedSession) -> Result<SessionId, CoreError> {
        let worktree = saved.worktree.clone();
        let (info, session) = {
            let mut state = self.inner.state.lock().expect("state lock");
            let info = self.inner.register_session(&mut state, saved, None);
            let session = state.sessions[&info.id].clone();
            session.update(|control| control.resuming = true);
            let info = session.info.lock().expect("info lock").clone(); // now showing it resuming
            (info, session)
        };
        let id = info.id;
        let _ = self
            .inner
            .events
            .send(CoreEvent::SessionCreated { session: info });
        self.inner.recent_changed(&worktree);
        self.inner.persist();
        self.inner.actors_may_change();
        if let Err(err) = self.inner.resume_claimed(&session, None).await {
            session.record(|t| {
                t.push(TranscriptItem::Notice {
                    text: format!("Couldn't reopen the conversation: {err}"),
                })
            });
        }
        Ok(id)
    }

    /// Sends a prompt; returns once it's on its way. The reply streams to watchers. A Suspended
    /// session is resumed first (the same conversation); if that fails, nothing was sent.
    pub async fn send_prompt(&self, id: SessionId, text: &str) -> Result<(), CoreError> {
        let session = self.session(id)?;
        session.wait_settled().await;
        let resume_first = session.update(|control| {
            if control.in_transition() {
                return Err(CoreError::SessionInTransition);
            }
            match control.state() {
                SessionState::Suspended => {
                    control.resuming = true;
                    Ok(true)
                }
                SessionState::Idle => Ok(false),
                SessionState::Exited => Err(CoreError::SessionExited),
                SessionState::Working | SessionState::NeedsYou => Err(CoreError::SessionBusy),
            }
        })?;
        let start_turn = |control: &mut Control| {
            control.auto_suspend_failed = false;
            session.record(|t| {
                t.push(TranscriptItem::User {
                    text: text.to_owned(),
                })
            });
            control.in_turn = true;
        };
        if resume_first {
            self.inner
                .resume_claimed_then(&session, None, start_turn)
                .await?;
        } else {
            session.update(|control| match control.state() {
                SessionState::Idle => {
                    start_turn(control);
                    Ok(())
                }
                SessionState::Exited => Err(CoreError::SessionExited),
                _ => Err(CoreError::SessionBusy),
            })?;
        }

        let Some(connection) = session.connection() else {
            // Only a restored Tab that was never resumed has none, and it was resumed above.
            session.update(|control| control.in_turn = false);
            return Err(CoreError::Acp(AcpError::Closed));
        };
        let params =
            json!({ "sessionId": session.acp_id, "prompt": [{ "type": "text", "text": text }] });
        let inner = self.inner.clone();
        tokio::spawn(async move {
            let result = connection.request("session/prompt", params).await;
            // The Agent may have committed or changed files: refresh ahead/changed counts.
            let refresh = inner.clone();
            tokio::spawn(async move { refresh.refresh_worktrees().await });
            session.update(|control| {
                // The adapter crashed and the session has been resumed elsewhere since (or closed):
                // this turn's ending is old news.
                let current = session.connection();
                if control.closed || !current.is_some_and(|c| Arc::ptr_eq(&c, &connection)) {
                    return;
                }
                // A question still open when the turn ends will never be answered.
                session.cancel_questions(control);
                control.in_turn = false;
                match result {
                    Ok(_) => {}
                    Err(err) if session_gone(&err) => control.exited = true,
                    Err(err) => session.record(|t| {
                        t.push(TranscriptItem::Notice {
                            text: format!("The turn failed: {err}"),
                        })
                    }),
                }
            });
        });
        Ok(())
    }
    /// Answers the permission card for `tool_call_id` with one of the options the Agent offered.
    pub async fn answer_permission(
        &self,
        id: SessionId,
        tool_call_id: &str,
        option_id: &str,
    ) -> Result<(), CoreError> {
        let session = self.session(id)?;
        session.update(|control| {
            let at = control
                .questions
                .iter()
                .position(|q| q.tool_call_id == tool_call_id)
                .ok_or(CoreError::NoPendingPermission)?;
            if !control.questions[at]
                .option_ids
                .iter()
                .any(|o| o == option_id)
            {
                return Err(CoreError::UnknownPermissionOption(option_id.to_owned()));
            }
            let question = control.questions.remove(at);
            let outcome = PermissionOutcome::Selected {
                option_id: option_id.to_owned(),
            };
            question
                .responder
                .result(permissions::acp_outcome(&outcome));
            session.record(|t| t.resolve_permission(question.index, outcome));
            Ok(())
        })
    }

    /// Stops an Idle session's Agent process to free its memory; the conversation stays in the
    /// Tab, and the next prompt brings it back. It shows as Working until the Agent confirms.
    pub async fn suspend_session(&self, id: SessionId) -> Result<(), CoreError> {
        let session = self.session(id)?;
        session.wait_settled().await;
        let stop = session.update(|control| {
            if control.in_transition() {
                return Err(CoreError::SessionInTransition);
            }
            match control.state() {
                SessionState::Idle => {
                    control.suspending = true;
                    Ok(true)
                }
                // Nothing running to stop.
                SessionState::Suspended | SessionState::Exited => Ok(false),
                SessionState::Working | SessionState::NeedsYou => Err(CoreError::SessionBusy),
            }
        })?;
        if !stop {
            return Ok(());
        }
        let Some(connection) = session.connection() else {
            // Nothing running to stop.
            session.update(|control| {
                control.suspending = false;
                control.suspended = true;
            });
            return Ok(());
        };
        let stopped = if !connection.supports_session("close") {
            Err(CoreError::Unsupported("suspending sessions"))
        } else {
            match close_acp(&connection, &session.acp_id).await {
                // A dead adapter has stopped it as surely as a close.
                Some(Ok(_)) | Some(Err(AcpError::Closed)) => Ok(()),
                Some(Err(err)) => Err(err.into()),
                None => Err(CoreError::SessionWontStop),
            }
        };
        session.update(|control| {
            control.suspending = false;
            control.suspended = stopped.is_ok();
        });
        stopped
    }

    /// Brings an Exited (or Suspended) session's conversation back, Idle and ready for a prompt.
    pub async fn resume_session(&self, id: SessionId) -> Result<(), CoreError> {
        let session = self.session(id)?;
        session.wait_settled().await;
        session.update(|control| {
            if control.in_transition() {
                return Err(CoreError::SessionInTransition);
            }
            match control.state() {
                SessionState::Suspended | SessionState::Exited => {
                    control.resuming = true;
                    Ok(())
                }
                _ => Err(CoreError::SessionRunning),
            }
        })?;
        self.inner.resume_claimed(&session, None).await
    }

    /// Switches how much the session's Agent may do without asking. A session that isn't running
    /// (Suspended, Exited, or mid-suspend/resume) takes the mode when it's resumed.
    pub async fn set_permission_mode(
        &self,
        id: SessionId,
        mode: PermissionMode,
    ) -> Result<(), CoreError> {
        let session = self.session(id)?;
        let stopped = session.update(|control| {
            let stopped = control.in_transition() || control.suspended || control.exited;
            if stopped {
                session.set_mode(mode); // under the control lock, so a resume can't miss it
            }
            stopped
        });
        if !stopped {
            apply_mode(&*session.live()?, &session.acp_id, mode).await?;
            session.set_mode(mode);
        }
        self.inner.persist();
        Ok(())
    }
    pub fn session_info(&self, id: SessionId) -> Result<SessionInfo, CoreError> {
        Ok(self.session(id)?.info.lock().expect("info lock").clone())
    }

    /// The whole transcript (mostly for tests; the frontend pages through `show_session` and
    /// `transcript_page`).
    pub fn transcript(&self, id: SessionId) -> Result<Vec<TranscriptItem>, CoreError> {
        Ok(self
            .session(id)?
            .transcript
            .lock()
            .expect("transcript lock")
            .items()
            .to_vec())
    }

    /// Up to a page of transcript items just before index `before`, for scrolling back past the
    /// page `show_session` sent.
    pub fn transcript_page_before(
        &self,
        id: SessionId,
        before: usize,
    ) -> Result<TranscriptPage, CoreError> {
        Ok(self
            .session(id)?
            .transcript
            .lock()
            .expect("transcript lock")
            .page_before(before))
    }

    /// Makes no Tab visible (e.g. a Worktree with no sessions is selected): ends the visible Tab's
    /// stream, and its new output counts as unread again.
    pub fn hide_tabs(&self) {
        let previous = self
            .inner
            .visible_tab
            .lock()
            .expect("visible tab lock")
            .take();
        if let Some((previous, _stop_previous)) = previous {
            previous.set_visible(false);
        } // dropping `_stop_previous` ends that stream
    }

    /// Makes `id` the visible Tab: ends the previous visible Tab's stream, marks this session read,
    /// and streams its transcript: a `Reset` with the latest page, then batched changes.
    pub fn show_session(&self, id: SessionId) -> Result<TranscriptStream, CoreError> {
        let session = self.session(id)?;
        // A restored Tab's conversation comes in when it's first looked at.
        self.inner.load_for_view(session.clone());
        let newly_active = {
            let mut state = self.inner.state.lock().expect("state lock");
            let active = Some(session.acp_id.clone());
            let changed = state.active != active;
            state.active = active;
            changed
        };
        if newly_active {
            self.inner.persist(); // to show it again after a restart
        }
        let (stop, mut stopped) = oneshot::channel::<()>();
        let previous = self
            .inner
            .visible_tab
            .lock()
            .expect("visible tab lock")
            .replace((session.clone(), stop));
        if let Some((previous, _stop_previous)) = previous {
            previous.set_visible(false);
        } // dropping `_stop_previous` ends that stream

        let (tx, rx) = mpsc::channel(64);
        let (initial, mut deltas) = {
            // Under the transcript lock, so no item can be counted unread after we mark it read.
            let transcript = session.transcript.lock().expect("transcript lock");
            session.visible.store(true, Ordering::SeqCst);
            session.mark_read();
            (transcript.latest_page(), session.deltas.subscribe())
        };
        tokio::spawn(async move {
            let mut batch = vec![initial];
            loop {
                if tx.send(std::mem::take(&mut batch)).await.is_err() {
                    return; // the watcher went away
                }
                // Wait for the next change, then gather whatever else arrives within the flush window.
                let first = tokio::select! {
                    _ = &mut stopped => return, // another Tab is visible now
                    delta = session.next_delta(&mut deltas) => delta,
                };
                let Some(first) = first else { return };
                batch.push(first);
                let deadline = tokio::time::Instant::now() + FLUSH_INTERVAL;
                while let Ok(Some(delta)) =
                    tokio::time::timeout_at(deadline, session.next_delta(&mut deltas)).await
                {
                    batch.push(delta);
                }
            }
        });
        Ok(TranscriptStream { rx })
    }

    fn workspace(&self) -> Result<WorkspaceInfo, CoreError> {
        self.inner
            .state
            .lock()
            .expect("state lock")
            .workspace
            .clone()
            .ok_or(CoreError::NoWorkspace)
    }

    fn session(&self, id: SessionId) -> Result<Arc<Session>, CoreError> {
        self.inner
            .state
            .lock()
            .expect("state lock")
            .sessions
            .get(&id)
            .cloned()
            .ok_or(CoreError::UnknownSession)
    }

    async fn adapter(&self) -> Result<Arc<Connection>, CoreError> {
        self.inner.connect().await
    }
}

impl Inner {
    fn documents(&self) -> std::sync::MutexGuard<'_, DocumentTracker> {
        self.documents.lock().expect("documents lock")
    }

    /// Starts actors for the Worktree being looked at and those with sessions, and stops the rest
    /// (dimmed and not looked at). Never for a Worktree being removed.
    async fn update_actors(self: &Arc<Self>) {
        let open: Vec<PathBuf> = self.documents().all().into_iter().map(|d| d.path).collect();
        let wanted: BTreeSet<PathBuf> = {
            let state = self.state.lock().expect("state lock");
            let usable = |path: &PathBuf| {
                state.worktrees.iter().any(|w| &w.path == path) && !state.removing.contains(path)
            };
            let mut wanted: BTreeSet<PathBuf> = state
                .sessions
                .values()
                .map(|s| s.info.lock().expect("info lock").worktree.clone())
                .filter(usable)
                .collect();
            // (A file open in a Manual editor is watched for changes on disk.)
            wanted.extend(
                state
                    .worktrees
                    .iter()
                    .map(|w| &w.path)
                    .filter(|w| open.iter().any(|p| p.starts_with(w)))
                    .filter(|w| usable(w))
                    .cloned(),
            );
            if let Some(shown) = self
                .shown_worktree
                .lock()
                .expect("shown worktree lock")
                .clone()
                .filter(usable)
            {
                wanted.insert(shown);
            }
            wanted
        };
        let starting: Vec<(PathBuf, ActorSlot)> = {
            let mut actors = self.actors.lock().expect("actors lock");
            actors.retain(|path, _| wanted.contains(path)); // dropping one stops it
            let mut starting = vec![];
            for path in wanted {
                if !actors.contains_key(&path) {
                    let slot = ActorSlot::default();
                    actors.insert(path.clone(), slot.clone());
                    starting.push((path, slot));
                }
            }
            starting
        };
        for (path, slot) in starting {
            self.start_actor(&path, &slot).await;
        }
    }

    /// The actor in `slot`, started (and indexed) by whichever caller gets there first.
    async fn start_actor(
        self: &Arc<Self>,
        worktree: &Path,
        slot: &ActorSlot,
    ) -> Arc<WorktreeActor> {
        slot.get_or_init(|| {
            WorktreeActor::start(
                worktree.to_owned(),
                self.config.file_watch,
                self.news_sink(),
            )
        })
        .await
        .clone()
    }

    /// Where actors report: file changes become events, status changes a (coalesced) refresh of
    /// that Worktree's branch status, and a watch-limit fallback a toast (once per Worktree).
    fn news_sink(self: &Arc<Self>) -> crate::files::NewsSink {
        let weak = Arc::downgrade(self);
        Arc::new(move |worktree: &Path, news| {
            let Some(inner) = weak.upgrade() else {
                return;
            };
            let worktree = worktree.to_owned();
            match news {
                ActorNews::Files => {
                    let _ = inner.events.send(CoreEvent::FilesChanged { worktree });
                }
                ActorNews::Status => inner.status_due(worktree),
                ActorNews::Touched(paths) => inner.check_documents(&worktree, paths),
                ActorNews::Fallback(message) => {
                    let first = inner
                        .fallback_told
                        .lock()
                        .expect("fallback lock")
                        .insert(worktree.clone());
                    if first {
                        let _ = inner
                            .events
                            .send(CoreEvent::FileWatchFallback { worktree, message });
                    }
                }
            }
        })
    }

    /// Compares the open files under `worktree` among `touched` (`None`: all) with what's on disk,
    /// and tells each editor whose file moved on: reload it if it's clean, ask if it isn't.
    fn check_documents(self: &Arc<Self>, worktree: &Path, touched: Option<Vec<PathBuf>>) {
        let to_check = self.documents().checks_for(worktree, touched.as_deref());
        self.check_disk(to_check);
    }

    /// Checks one open file against the disk (after it's opened or saved, say).
    fn check_document(self: &Arc<Self>, path: &Path) {
        let ticket = self.documents().ticket(path);
        self.check_disk(vec![(path.to_owned(), ticket)]);
    }

    /// Reads each file's disk version (since its epoch) and tells the editors whose file moved on.
    /// A read that raced a save or an open is done again a moment later, so no change goes unseen.
    fn check_disk(self: &Arc<Self>, to_check: Vec<(PathBuf, CheckTicket)>) {
        if to_check.is_empty() {
            return;
        }
        let inner = self.clone();
        tokio::spawn(async move {
            let Ok(on_disk) = tokio::task::spawn_blocking(move || {
                to_check
                    .into_iter()
                    .map(|(path, ticket)| (documents::disk_version(&path), path, ticket))
                    .collect::<Vec<_>>()
            })
            .await
            else {
                return;
            };
            for (disk, path, ticket) in on_disk {
                // (Unreadable just now: the next change, or poll, looks again.)
                let Some(disk) = disk else { continue };
                let checked = inner.documents().changed_on_disk(&path, &disk, ticket);
                let Some(notices) = checked else {
                    let inner = inner.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(RECHECK_AFTER).await;
                        inner.check_document(&path);
                    });
                    continue;
                };
                let deleted = disk == documents::version_of(None);
                for notice in notices {
                    let path = path.clone();
                    let _ = inner.events.send(match notice {
                        documents::Notice::Back { window } => {
                            CoreEvent::DocumentBackOnDisk { window, path }
                        }
                        documents::Notice::Changed { window, dirty } if dirty || deleted => {
                            CoreEvent::DocumentConflicted {
                                window,
                                path,
                                deleted,
                            }
                        }
                        documents::Notice::Changed { window, .. } => {
                            CoreEvent::DocumentChangedOnDisk { window, path }
                        }
                    });
                }
            }
        });
    }

    /// Tells the frontend the open files after a change to them.
    fn documents_changed(&self) {
        let documents = self.documents().all();
        let _ = self.events.send(CoreEvent::DocumentsChanged { documents });
    }

    /// Refreshes a Worktree's branch status a moment from now, once however often it's asked.
    fn status_due(self: &Arc<Self>, worktree: PathBuf) {
        if !self
            .status_due
            .lock()
            .expect("status due lock")
            .insert(worktree.clone())
        {
            return; // already due
        }
        let inner = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(STATUS_SETTLE).await;
            inner
                .status_due
                .lock()
                .expect("status due lock")
                .remove(&worktree);
            inner.refresh_status_of(&worktree).await;
        });
    }

    /// `update_actors` in the background (from places that can't wait for it).
    fn actors_may_change(self: &Arc<Self>) {
        let inner = self.clone();
        tokio::spawn(async move { inner.update_actors().await });
    }

    /// Adds a Tab's session: running on `connection`, or (with none) Suspended until resumed, its
    /// conversation to be loaded then. Call with the state lock held.
    fn register_session(
        &self,
        state: &mut State,
        saved: SavedSession,
        connection: Option<Arc<Connection>>,
    ) -> SessionInfo {
        state.next_session += 1;
        let id = SessionId(state.next_session);
        let stopped = connection.is_none();
        let info = SessionInfo {
            id,
            name: saved.name,
            worktree: saved.worktree,
            state: if stopped {
                SessionState::Suspended
            } else {
                SessionState::Idle
            },
            permission_mode: saved.permission_mode,
            unread: 0,
        };
        let (deltas, _) = broadcast::channel(4096);
        let session = Arc::new(Session {
            info: Mutex::new(info.clone()),
            acp_id: saved.acp_id.clone(),
            connection: Mutex::new(connection),
            transcript: Mutex::default(),
            deltas,
            events: self.events.clone(),
            visible: AtomicBool::new(false),
            replaying: AtomicBool::new(false),
            settled: tokio::sync::Notify::new(),
            idle_since: Mutex::new((!stopped).then(|| self.clock.now())),
            clock: self.clock.clone(),
            tool_calls: Mutex::default(),
            control: Mutex::new(Control {
                suspended: stopped,
                loaded: !stopped,
                ..Control::default()
            }),
        });
        state.sessions.insert(id, session);
        state.by_acp_id.insert(saved.acp_id, id);
        info
    }

    fn recent_in(&self, worktree: &Path) -> Vec<RecentSession> {
        self.state
            .lock()
            .expect("state lock")
            .recent
            .iter()
            .filter(|s| s.worktree == worktree)
            .map(|s| RecentSession {
                acp_id: s.acp_id.clone(),
                name: s.name.clone(),
                worktree: s.worktree.clone(),
            })
            .collect()
    }

    fn recent_changed(&self, worktree: &Path) {
        let _ = self.events.send(CoreEvent::RecentSessionsChanged {
            worktree: worktree.to_owned(),
            sessions: self.recent_in(worktree),
        });
    }

    /// Saves this Workspace's open Tabs (in order), Recent sessions and last shown Tab.
    fn persist(&self) {
        let Some(path) = self
            .config
            .state_path
            .as_ref()
            .filter(|_| self.state_writable)
        else {
            return;
        };
        // Held across the snapshot and the write, so saves land in the order they were taken.
        let _in_order = self.persist_lock.lock().expect("persist lock");
        let (root, saved) = {
            let state = self.state.lock().expect("state lock");
            let Some(workspace) = &state.workspace else {
                return;
            };
            let mut open: Vec<_> = state.sessions.values().collect();
            open.sort_by_key(|s| s.info.lock().expect("info lock").id);
            let saved = WorkspaceState {
                tabs: open.into_iter().map(|s| s.saved()).collect(),
                recent: state.recent.clone(),
                active: state.active.clone(),
            };
            (workspace.root.clone(), saved)
        };
        if let Err(err) = app_state::save_workspace(path, &root, &saved) {
            eprintln!("couldn't save the open Tabs: {err}");
        }
    }

    /// Brings back the Tabs open when the editor last closed this Workspace, Suspended (no Agent
    /// starts until one is used), in their Worktrees and order, and its Recent sessions. Ones whose
    /// Worktree is gone are dropped (but not because listing the Worktrees failed).
    fn restore(&self, root: &Path) {
        let saved = self
            .app_state
            .lock()
            .expect("app state lock")
            .workspaces
            .get(root)
            .cloned()
            .unwrap_or_default();
        let mut created = vec![];
        {
            let mut state = self.state.lock().expect("state lock");
            let listing_failed = state.worktrees.is_empty();
            let live = |state: &State, s: &SavedSession| {
                state.worktrees.iter().any(|w| w.path == s.worktree)
                    || (listing_failed && s.worktree.exists())
            };
            for s in saved.tabs.iter().chain(&saved.recent) {
                state.note_name(&s.name);
            }
            for tab in saved.tabs {
                if live(&state, &tab) && !state.by_acp_id.contains_key(&tab.acp_id) {
                    created.push(self.register_session(&mut state, tab, None));
                }
            }
            state.recent = saved
                .recent
                .into_iter()
                .filter(|s| live(&state, s))
                .collect();
            state.active = saved.active;
        }
        for info in created {
            let _ = self
                .events
                .send(CoreEvent::SessionCreated { session: info });
        }
    }
    /// The shared adapter connection, started (and initialised) on first use or after it exited.
    async fn connect(self: &Arc<Self>) -> Result<Arc<Connection>, CoreError> {
        let mut slot = self.adapter.lock().await;
        if let Some(connection) = slot.as_ref().filter(|c| !c.is_closed()) {
            return Ok(connection.clone());
        }
        let weak = Arc::downgrade(self);
        let generation = self.generations.fetch_add(1, Ordering::Relaxed);
        let connection = Arc::new(Connection::spawn(
            &self.config.adapter,
            generation,
            move |incoming| {
                if let Some(inner) = Weak::upgrade(&weak) {
                    inner.handle(incoming, generation);
                }
            },
        )?);
        let init = connection
            .request(
                "initialize",
                json!({
                    "protocolVersion": PROTOCOL_VERSION,
                    "clientCapabilities": { "fs": { "readTextFile": false, "writeTextFile": false }, "terminal": false },
                    "clientInfo": { "name": "agent-editor", "version": env!("CARGO_PKG_VERSION") },
                }),
            )
            .await?;
        if init["protocolVersion"] != json!(PROTOCOL_VERSION) {
            return Err(AcpError::UnsupportedProtocol(init["protocolVersion"].clone()).into());
        }
        connection.set_capabilities(init["agentCapabilities"].clone());
        *slot = Some(connection.clone());
        Ok(connection)
    }

    /// Brings a session whose `resuming` the caller set back on `connection` (default: the current
    /// adapter), and clears `resuming`. On success it's Idle on the new connection, in its current
    /// permission mode; on failure it's left as it was (Suspended stays Suspended). A session closed
    /// meanwhile (its Worktree is being removed) isn't revived: it's closed on the new connection.
    async fn resume_claimed(
        self: &Arc<Self>,
        session: &Arc<Session>,
        connection: Option<Arc<Connection>>,
    ) -> Result<(), CoreError> {
        self.resume_claimed_then(session, connection, |_| {}).await
    }

    /// `resume_claimed`, running `then` in the same state change as a successful resume (so a
    /// prompt waiting on it starts without the session passing through Idle). Everything that can
    /// fail happens before that change.
    async fn resume_claimed_then(
        self: &Arc<Self>,
        session: &Arc<Session>,
        connection: Option<Arc<Connection>>,
        then: impl FnOnce(&mut Control),
    ) -> Result<(), CoreError> {
        let reopened = self.reopen(session, connection).await;
        let outcome = session.update(|control| {
            control.resuming = false;
            match reopened {
                Ok(connection) if control.closed => {
                    Err((CoreError::BeingRemoved, Some(connection)))
                }
                // It died again while resuming; `adapter_closed` didn't see it, as it wasn't on it yet.
                Ok(connection) if connection.is_closed() => {
                    Err((CoreError::Acp(AcpError::Closed), None))
                }
                Ok(connection) => {
                    *session.connection.lock().expect("connection lock") = Some(connection);
                    control.suspended = false;
                    control.exited = false;
                    control.auto_suspend_failed = false;
                    then(control);
                    Ok(())
                }
                Err(err) => Err((err, None)),
            }
        });
        match outcome {
            Ok(()) => Ok(()),
            Err((err, revived)) => {
                if let Some(connection) = revived {
                    let _ = close_acp(&connection, &session.acp_id).await;
                }
                Err(err)
            }
        }
    }

    /// Picks the session's conversation up on `connection` (default: the current adapter):
    /// `session/resume`, which doesn't replay it (the transcript is already here), or for a
    /// restored Tab whose transcript isn't, `session/load`, which replays it into an emptied
    /// transcript. Then puts it in the session's permission mode, as that is by the end.
    async fn reopen(
        self: &Arc<Self>,
        session: &Session,
        connection: Option<Arc<Connection>>,
    ) -> Result<Arc<Connection>, CoreError> {
        let cwd = session.info.lock().expect("info lock").worktree.clone();
        if self
            .state
            .lock()
            .expect("state lock")
            .removing
            .contains(&cwd)
        {
            return Err(CoreError::BeingRemoved);
        }
        let connection = match connection {
            Some(connection) => connection,
            None => self.connect().await?,
        };
        let loaded = session.control.lock().expect("control lock").loaded;
        let params = json!({ "sessionId": session.acp_id, "cwd": cwd, "mcpServers": [] });
        let resumed = if loaded || !connection.supports_load() {
            if !connection.supports_session("resume") {
                return Err(CoreError::Unsupported("resuming sessions"));
            }
            let resumed = connection.request("session/resume", params).await?;
            if !loaded {
                // It carries on where it left off; only the earlier messages can't be shown.
                session.update(|control| control.loaded = true);
                session.record(|t| {
                    t.push(TranscriptItem::Notice {
                        text: "This Agent can't show the conversation from before the restart."
                            .into(),
                    })
                });
            }
            resumed
        } else {
            // Start from empty, so a load that failed half way (or ran before) can't double it.
            // The replay comes as updates before the reply, and isn't news to the user.
            session.record(|t| t.clear());
            session.tool_calls.lock().expect("tool calls lock").clear();
            session.replaying.store(true, Ordering::SeqCst);
            let replayed = connection.request("session/load", params).await;
            session.replaying.store(false, Ordering::SeqCst);
            let replayed = replayed?;
            session.update(|control| control.loaded = true);
            replayed
        };
        let mut current = resumed["modes"]["currentModeId"]
            .as_str()
            .and_then(PermissionMode::from_acp_id);
        // The mode can change while this runs (it's set locally on a stopped session).
        for _ in 0..3 {
            let mode = session.info.lock().expect("info lock").permission_mode;
            if current == Some(mode) {
                break;
            }
            apply_mode(&connection, &session.acp_id, mode).await?;
            current = Some(mode);
        }
        Ok(connection)
    }
    /// Adapter process `generation` ended. Its Idle sessions are resumed on one restarted adapter;
    /// ones that were mid-turn (or waiting on you) become Exited, since that turn is lost; Suspended
    /// ones, and ones already mid-suspend or mid-resume, are left to that. Only a crash that had
    /// sessions to restart counts towards the restart limit.
    fn adapter_closed(self: &Arc<Self>, generation: u64) {
        let on_it: Vec<_> = self
            .state
            .lock()
            .expect("state lock")
            .sessions
            .values()
            .filter(|s| s.connection().is_some_and(|c| c.generation == generation))
            .cloned()
            .collect();
        let mut idle = vec![];
        for session in on_it {
            session.update(|control| {
                let mid_turn = control.in_turn || !control.questions.is_empty();
                session.cancel_questions(control);
                control.in_turn = false;
                let settled = control.in_transition()
                    || control.suspended
                    || control.exited
                    || control.closed;
                if mid_turn && !control.in_transition() && !control.suspended {
                    control.exited = true;
                } else if !settled {
                    idle.push(session.clone());
                }
            });
        }
        if idle.is_empty() {
            return;
        }
        if !self.may_restart() {
            for session in idle {
                session.exit_with_notice(
                    "The Agent adapter keeps crashing, so it wasn't restarted. Resume to try again.",
                );
            }
            return;
        }
        for session in &idle {
            session.update(|control| control.resuming = true);
        }
        let inner = self.clone();
        tokio::spawn(async move {
            // One restart for all of them (a failing restart mustn't spawn one adapter per session).
            let connection = match inner.connect().await {
                Ok(connection) => Some(connection),
                Err(err) => {
                    for session in &idle {
                        session.update(|control| control.resuming = false);
                        session.exit_with_notice(&format!(
                            "The Agent adapter couldn't be restarted: {err}"
                        ));
                    }
                    None
                }
            };
            let Some(connection) = connection else { return };
            for session in idle {
                if let Err(err) = inner
                    .resume_claimed(&session, Some(connection.clone()))
                    .await
                {
                    session.exit_with_notice(&format!(
                        "The Agent stopped and couldn't be resumed: {err}"
                    ));
                }
            }
        });
    }

    /// Loads a restored Tab's conversation (`session/load` replays it) and closes the Agent again,
    /// so looking at a Tab never leaves an Agent running; it stays Suspended throughout. Does
    /// nothing if it's loaded or busy, or if loading it already failed (the Tab says why).
    fn load_for_view(self: &Arc<Self>, session: Arc<Session>) {
        let claimed = session.update(|control| {
            let wanted = !control.loaded
                && !control.load_failed
                && control.suspended
                && !control.in_transition()
                && !control.closed;
            if wanted {
                control.loading = true;
            }
            wanted
        });
        if !claimed {
            return;
        }
        let inner = self.clone();
        tokio::spawn(async move {
            let reopened = inner.reopen(&session, None).await;
            let stopped = match &reopened {
                Ok(connection) => matches!(
                    close_acp(connection, &session.acp_id).await,
                    Some(Ok(_)) | Some(Err(AcpError::Closed))
                ),
                Err(_) => true,
            };
            session.update(|control| {
                control.loading = false;
                match reopened {
                    Ok(connection) => {
                        *session.connection.lock().expect("connection lock") = Some(connection);
                        // If the Agent didn't confirm the close, it's running: say so (Idle).
                        control.suspended = stopped;
                    }
                    Err(err) => {
                        control.load_failed = true;
                        session.record(|t| {
                            t.push(TranscriptItem::Notice {
                                text: format!(
                                    "Couldn't load the conversation: {err}. Sending a message tries again."
                                ),
                            })
                        });
                    }
                }
            });
        });
    }
    /// Whether to restart the adapter after a crash: not after `CRASH_LIMIT` within `CRASH_WINDOW`
    /// (something is wrong, and restarting would only loop).
    fn may_restart(&self) -> bool {
        let mut crashes = self.crashes.lock().expect("crashes lock");
        let now = std::time::Instant::now();
        crashes.retain(|at| now.duration_since(*at) < CRASH_WINDOW);
        crashes.push(now);
        crashes.len() <= CRASH_LIMIT
    }
    /// Re-reads the settings file, publishing the result if it changed anything.
    fn reload_settings(&self) {
        let Some(path) = &self.config.settings_path else {
            return;
        };
        // Read under the lock, so an older read can't be applied over a newer one.
        let mut loaded = self.settings.lock().expect("settings lock");
        let next = settings::apply(&loaded, settings::read(path));
        if *loaded != next {
            *loaded = next.clone();
            let _ = self
                .events
                .send(CoreEvent::SettingsChanged { settings: next });
        }
    }

    /// Lets `decide` say where a failed setup carries on from (it may update the run first), and
    /// marks it so under the lock, so only one Retry or Start anyway takes effect.
    fn claim_failed_setup(
        &self,
        worktree: &Path,
        decide: impl FnOnce(&mut SetupRun) -> Resume,
    ) -> Result<Resume, CoreError> {
        let mut state = self.state.lock().expect("state lock");
        if state.removing.contains(worktree) {
            return Err(CoreError::BeingRemoved);
        }
        let run = state
            .setups
            .get_mut(worktree)
            .ok_or(CoreError::NoFailedSetup)?;
        if !matches!(
            run.info.status,
            SetupStatus::Failed { .. } | SetupStatus::SessionFailed { .. }
        ) {
            return Err(CoreError::NoFailedSetup);
        }
        let resume = decide(run);
        run.info.status = match resume {
            Resume::Command(step) => SetupStatus::Running { step },
            Resume::Session => SetupStatus::StartingSession,
        };
        let _ = self.events.send(run.changed());
        Ok(resume)
    }

    fn set_setup_status(&self, worktree: &Path, status: SetupStatus) {
        let mut state = self.state.lock().expect("state lock");
        let Some(run) = state.setups.get_mut(worktree) else {
            return;
        };
        if run.info.status != status {
            run.info.status = status;
            let _ = self.events.send(run.changed());
        }
    }

    /// Keeps setup output for late viewers and streams it to current ones.
    fn setup_output(&self, worktree: &Path, text: String) {
        let mut state = self.state.lock().expect("state lock");
        let Some(run) = state.setups.get_mut(worktree) else {
            return;
        };
        setup::keep_output(&mut run.info.output, &text);
        let _ = self.events.send(CoreEvent::SetupOutput {
            worktree: worktree.to_owned(),
            text,
        });
    }

    /// Re-lists the Worktrees with each one's branch status.
    async fn refresh_worktrees(self: &Arc<Self>) {
        self.relist(None).await;
    }

    /// Re-lists the Worktrees, refreshing only `worktree`'s branch status (and any new one's): what
    /// a Worktree actor asks for when its files or git folder changed.
    async fn refresh_status_of(self: &Arc<Self>, worktree: &Path) {
        self.relist(Some(worktree)).await;
    }

    /// Refreshes run one at a time (so an older listing can't overwrite a newer one), and a
    /// listing is dropped if another Workspace was opened meanwhile.
    async fn relist(self: &Arc<Self>, only: Option<&Path>) {
        let _one_at_a_time = self.refresh_lock.lock().await;
        let root_now = || {
            self.state
                .lock()
                .expect("state lock")
                .workspace
                .as_ref()
                .map(|w| w.root.clone())
        };
        let Some(root) = root_now() else {
            return;
        };
        let listed = match only {
            Some(worktree) => {
                let previous = self.state.lock().expect("state lock").worktrees.clone();
                worktrees::list_refreshing(&root, &previous, worktree).await
            }
            None => worktrees::list(&root).await,
        };
        let mut state = self.state.lock().expect("state lock");
        if state.workspace.as_ref().map(|w| &w.root) != Some(&root) {
            return;
        }
        if state.worktrees != listed {
            let paths =
                |list: &[WorktreeInfo]| list.iter().map(|w| w.path.clone()).collect::<Vec<_>>();
            let list_changed = paths(&state.worktrees) != paths(&listed);
            // A removed Worktree's setup is over (and its command can't keep running there).
            state.setups.retain(|path, run| {
                let listed = listed.iter().any(|w| &w.path == path);
                if !listed {
                    run.stop();
                }
                listed
            });
            state.worktrees = listed.clone();
            // A Worktree that's gone takes its Recent sessions with it (a failed listing doesn't).
            let mut gone: Vec<PathBuf> = vec![];
            if !listed.is_empty() {
                state.recent.retain(|s| {
                    let live = listed.iter().any(|w| w.path == s.worktree);
                    if !live && !gone.contains(&s.worktree) {
                        gone.push(s.worktree.clone());
                    }
                    live
                });
            }
            drop(state);
            let _ = self
                .events
                .send(CoreEvent::WorktreesChanged { worktrees: listed });
            if list_changed {
                self.actors_may_change(); // a Worktree that's gone stops being watched
            }
            if !gone.is_empty() {
                for worktree in &gone {
                    self.recent_changed(worktree);
                }
                self.persist();
            }
        }
    }

    /// Re-lists the Worktrees whenever the repository's `worktrees/` folder changes (debounced, so
    /// one `git worktree add` triggers one refresh).
    async fn watch_worktrees(self: &Arc<Self>, root: &Path) {
        // Stop watching the previous Workspace first, even if watching this one fails.
        self.discovery.lock().expect("discovery lock").take();
        let Some(common_dir) = git::common_dir(root).await else {
            return;
        };
        let Some((discovery, mut changes)) = Discovery::start(&common_dir) else {
            return;
        };
        let weak = Arc::downgrade(self);
        let discovery = Arc::new(discovery);
        *self.discovery.lock().expect("discovery lock") = Some(discovery.clone());
        let watching = Arc::downgrade(&discovery);
        tokio::spawn(async move {
            while changes.recv().await.is_some() {
                tokio::time::sleep(Duration::from_millis(200)).await;
                while changes.try_recv().is_ok() {}
                let (Some(inner), Some(discovery)) = (weak.upgrade(), watching.upgrade()) else {
                    return; // the core, or this Workspace's watch, is gone
                };
                discovery.watch_worktrees_dir();
                inner.refresh_worktrees().await;
            }
        });
    }

    /// Handles what adapter process `generation` sends us, in arrival order (see `acp`).
    fn handle(self: &Arc<Self>, incoming: Incoming, generation: u64) {
        match incoming {
            Incoming::Notification { method, params } if method == "session/update" => {
                let Some(session) = self.session_for(&params) else {
                    return;
                };
                let update = &params["update"];
                match update["sessionUpdate"].as_str() {
                    // Only meaningful while `session/load` replays a conversation: live, the
                    // user's message is already in the transcript.
                    Some("user_message_chunk")
                        if update["content"]["type"] == "text"
                            && session.replaying.load(Ordering::SeqCst) =>
                    {
                        let text = update["content"]["text"].as_str().unwrap_or_default();
                        let message_id = update["messageId"].as_str().map(str::to_owned);
                        session.record(|t| t.append_user_text(text, message_id));
                    }
                    Some("agent_message_chunk") if update["content"]["type"] == "text" => {
                        let text = update["content"]["text"].as_str().unwrap_or_default();
                        let message_id = update["messageId"].as_str().map(str::to_owned);
                        session.record(|t| t.append_agent_text(text, message_id));
                    }
                    Some("tool_call" | "tool_call_update") => session.note_tool_call(update),
                    // The Agent can change mode itself, e.g. when leaving plan mode.
                    // (A replayed mode change is history: the Tab's own mode is applied after.)
                    Some("current_mode_update") if !session.replaying.load(Ordering::SeqCst) => {
                        if let Some(mode) = update["currentModeId"]
                            .as_str()
                            .and_then(PermissionMode::from_acp_id)
                        {
                            session.set_mode(mode);
                            self.persist();
                        }
                    }
                    _ => {}
                }
            }
            Incoming::Notification { .. } => {}
            Incoming::Request {
                method,
                params,
                responder,
            } if method == "session/request_permission" => match self.session_for(&params) {
                Some(session) => session.ask_permission(&params, responder),
                None => responder.result(permissions::acp_outcome(&PermissionOutcome::Cancelled)),
            },
            Incoming::Request { responder, .. } => {
                responder.error(-32601, "method not supported by this client")
            }
            Incoming::Closed => self.adapter_closed(generation),
        }
    }

    fn session_for(&self, params: &Value) -> Option<Arc<Session>> {
        let state = self.state.lock().expect("state lock");
        let id = state.by_acp_id.get(params["sessionId"].as_str()?)?;
        state.sessions.get(id).cloned()
    }
}

impl Session {
    /// Waits (a while, at most) for a suspend, resume or load in progress to settle, so a send or
    /// close right after one carries on instead of failing.
    async fn wait_settled(&self) {
        let deadline = tokio::time::Instant::now() + CLOSE_TIMEOUT * 2;
        loop {
            let notified = self.settled.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if !self.control.lock().expect("control lock").in_transition() {
                return;
            }
            if tokio::time::timeout_at(deadline, notified).await.is_err() {
                return; // the caller then reports it's still in transition
            }
        }
    }

    /// How it's remembered between runs.
    fn saved(&self) -> SavedSession {
        let info = self.info.lock().expect("info lock");
        SavedSession {
            acp_id: self.acp_id.clone(),
            name: info.name.clone(),
            worktree: info.worktree.clone(),
            permission_mode: info.permission_mode,
        }
    }

    /// The adapter connection it's on: `None` for a restored Tab that hasn't been resumed yet.
    fn connection(&self) -> Option<Arc<Connection>> {
        self.connection.lock().expect("connection lock").clone()
    }

    fn live(&self) -> Result<Arc<Connection>, CoreError> {
        self.connection().ok_or(CoreError::Acp(AcpError::Closed))
    }

    /// Marks it Exited, saying why in its transcript.
    fn exit_with_notice(&self, why: &str) {
        self.update(|control| {
            control.exited = true;
            self.record(|t| {
                t.push(TranscriptItem::Notice {
                    text: why.to_owned(),
                })
            });
        });
    }

    /// Changes the facts behind the session's state under one lock, then publishes the state they
    /// now imply. Lock order: control, then transcript, then info.
    fn update<R>(&self, change: impl FnOnce(&mut Control) -> R) -> R {
        let mut control = self.control.lock().expect("control lock");
        let suspending_before = control.suspending;
        let result = change(&mut control);
        if !control.in_transition() {
            self.settled.notify_waiters();
        }
        let state = control.state();
        let mut info = self.info.lock().expect("info lock");
        if info.state != state {
            info.state = state;
            let mut idle_since = self.idle_since.lock().expect("idle lock");
            if !(control.suspending || suspending_before) {
                *idle_since = (state == SessionState::Idle).then(|| self.clock.now());
            } else if state == SessionState::Suspended {
                *idle_since = None;
            } // (a suspend in progress, or one that failed, leaves its Idle time as it was)
            let _ = self.events.send(CoreEvent::SessionStateChanged {
                session_id: info.id,
                state,
            });
        }
        result
    }

    fn set_mode(&self, mode: PermissionMode) {
        let mut info = self.info.lock().expect("info lock");
        if info.permission_mode != mode {
            info.permission_mode = mode;
            let _ = self.events.send(CoreEvent::PermissionModeChanged {
                session_id: info.id,
                mode,
            });
        }
    }

    /// Applies a transcript change and streams its delta, atomically with respect to `show_session`.
    /// New items count as unread while the Tab isn't visible.
    fn record(&self, change: impl FnOnce(&mut Transcript) -> TranscriptDelta) {
        let mut transcript = self.transcript.lock().expect("transcript lock");
        let delta = change(&mut transcript);
        let unseen = matches!(&delta, TranscriptDelta::ItemAdded { item, .. } if !matches!(item, TranscriptItem::User { .. } | TranscriptItem::ToolCall { .. }))
            && !self.visible.load(Ordering::SeqCst)
            && !self.replaying.load(Ordering::SeqCst);
        let _ = self.deltas.send(delta);
        if unseen {
            let mut info = self.info.lock().expect("info lock");
            info.unread += 1;
            let _ = self.events.send(CoreEvent::SessionUnreadChanged {
                session_id: info.id,
                unread: info.unread,
            });
        }
    }

    fn set_visible(&self, visible: bool) {
        let _transcript = self.transcript.lock().expect("transcript lock"); // same ordering as `record`
        self.visible.store(visible, Ordering::SeqCst);
    }

    /// Clears the unread count. Call with the transcript lock held (lock order: transcript, then info).
    fn mark_read(&self) {
        let mut info = self.info.lock().expect("info lock");
        if info.unread != 0 {
            info.unread = 0;
            let _ = self.events.send(CoreEvent::SessionUnreadChanged {
                session_id: info.id,
                unread: 0,
            });
        }
    }

    /// Merges a tool call (or its update) into what's known, and adds or updates its transcript row.
    fn note_tool_call(&self, update: &Value) {
        let Some(id) = update["toolCallId"].as_str() else {
            return;
        };
        let worktree = self.info.lock().expect("info lock").worktree.clone();
        let mut tool_calls = self.tool_calls.lock().expect("tool calls lock");
        let known = tool_calls
            .entry(id.to_owned())
            .or_insert_with(|| KnownToolCall {
                call: json!({}),
                row: None,
            });
        permissions::merge_tool_call(&mut known.call, update);
        let row = permissions::tool_call_row(&known.call, &worktree);
        match known.row {
            Some(index) => self.record(|t| t.replace(index, row)),
            None => self.record(|t| {
                known.row = Some(t.items().len());
                t.push(row)
            }),
        }
    }

    /// Puts a permission card in the transcript and holds the question until it's answered. The
    /// card is published under the control lock, so it can't be answered before it's registered.
    fn ask_permission(&self, params: &Value, responder: Responder) {
        let mut tool_call = params["toolCall"]["toolCallId"]
            .as_str()
            .and_then(|id| {
                self.tool_calls
                    .lock()
                    .expect("tool calls lock")
                    .get(id)
                    .map(|known| known.call.clone())
            })
            .unwrap_or_else(|| json!({}));
        permissions::merge_tool_call(&mut tool_call, &params["toolCall"]);
        let worktree = self.info.lock().expect("info lock").worktree.clone();
        let request = permissions::request_from(&tool_call, &params["options"], &worktree);
        self.update(|control| {
            if control.exited {
                return responder.result(permissions::acp_outcome(&PermissionOutcome::Cancelled));
            }
            let tool_call_id = request.tool_call_id.clone();
            let option_ids = request.options.iter().map(|o| o.id.clone()).collect();
            let mut index = 0;
            self.record(|t| {
                index = t.items().len();
                t.push(TranscriptItem::Permission {
                    request,
                    outcome: None,
                })
            });
            control.questions.push(OpenQuestion {
                tool_call_id,
                index,
                option_ids,
                responder,
            });
        });
    }

    /// Tells the Agent every open question is cancelled and marks their cards so.
    fn cancel_questions(&self, control: &mut Control) {
        for question in control.questions.drain(..) {
            question
                .responder
                .result(permissions::acp_outcome(&PermissionOutcome::Cancelled));
            self.record(|t| t.resolve_permission(question.index, PermissionOutcome::Cancelled));
        }
    }

    /// The next change for a watcher; if the watcher fell behind, a `Reset` to the current transcript.
    /// `None` once the session is gone.
    async fn next_delta(
        &self,
        deltas: &mut broadcast::Receiver<TranscriptDelta>,
    ) -> Option<TranscriptDelta> {
        match deltas.recv().await {
            Ok(delta) => Some(delta),
            Err(broadcast::error::RecvError::Lagged(_)) => Some(
                self.transcript
                    .lock()
                    .expect("transcript lock")
                    .latest_page(),
            ),
            Err(broadcast::error::RecvError::Closed) => None,
        }
    }
}
