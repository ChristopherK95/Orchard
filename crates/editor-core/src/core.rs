//! The core's public API: the same surface the frontend drives over Tauri IPC (ADR 0003).

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use base64::Engine;
use serde::Serialize;
use serde_json::{json, Value};
use tokio::sync::{broadcast, mpsc, oneshot};

use crate::acp::{AcpError, AdapterCommand, Connection, Incoming, Responder, PROTOCOL_VERSION};
use crate::app_state::{self, AppState, SavedSession, WorkspaceState};
use crate::attachments::{self, Attachment};
use crate::auto_suspend::{
    self, AutoSuspendReason, Candidate, Clock, Limits, MemoryProbe, SystemClock, SystemProbe,
};
use crate::create_worktree::{self, BranchInfo, BranchList, CreatedWorktree, NewWorktree};
use crate::documents::{
    self, CheckTicket, DocumentTracker, OpenDocument, OpenedFile, PoppedOutFile, SaveOver,
};
use crate::edit_notes::{EditNote, EditNotes};
use crate::files::{
    ActorNews, DirEntry, FileMatch, FileWatchConfig, IndexStats, WatchStatus, WorktreeActor,
};
use crate::git;
use crate::git_status::{
    BaseChange, BaseChanges, ChangeKind, CommitOutcome, CommitRequest, GitStatus, PullOutcome,
    PushOutcome,
};
use crate::merged::{self, WorktreeMerge};
use crate::permissions;
use crate::pull_request::{
    CommitChanges, OpenPullRequest, PullRequestDetails, PullRequestOutcome, PullRequestProgress,
    PullRequestRequest, PushToPullRequestOutcome, Review, ReviewCommit, ReviewFile,
};
use crate::remove_worktree::{self, RemovalCheck, RemoveWorktree, RemovedWorktree};
use crate::session::{
    PermissionMode, PermissionOutcome, SessionId, SessionInfo, SessionState, Transcript,
    TranscriptDelta, TranscriptItem, TranscriptPage,
};
use crate::settings::{self, LoadedSettings, RepoSettings, SettingChange, WindowsShell};
use crate::setup::{self, SetupInfo, SetupStatus};
use crate::terminal::{self, ActionShell, Terminal, TerminalId, TerminalInfo, TerminalStream};
use crate::worktrees::{self, Discovery, WorktreeInfo};

/// A slash command the Agent offers (one of Claude Code's, a custom command or a skill), from ACP's
/// `available_commands_update`. Typing `/name args` as a prompt runs it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SlashCommand {
    pub name: String,
    pub description: String,
    /// What to type after it, if it takes input (e.g. "[file]").
    pub hint: Option<String>,
}

/// A message written while the Agent was working, sent as the next prompt when the turn ends Idle
/// (ticket 39). A session holds at most one; queuing more adds to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueuedPrompt {
    pub text: String,
    pub attachments: Vec<Attachment>,
}

impl SlashCommand {
    /// The commands in an `available_commands_update` (ones without a name are skipped).
    fn list_from(update: &Value) -> Vec<SlashCommand> {
        update["availableCommands"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|c| {
                Some(SlashCommand {
                    name: c["name"].as_str().filter(|n| !n.is_empty())?.to_owned(),
                    description: c["description"].as_str().unwrap_or_default().to_owned(),
                    hint: c["input"]["hint"].as_str().map(str::to_owned),
                })
            })
            .collect()
    }
}

/// ACP's "Resource not found" error: e.g. `session/load` of a conversation Claude Code doesn't have.
const RESOURCE_NOT_FOUND: i64 = -32002;

/// The Tabs view's view slot: its one visible Tab (the Columns view has a slot per column).
pub const TABS_SLOT: &str = "tabs";

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

/// A Tab name from a conversation's title: its first line, cut to `TITLE_NAME_LEN` characters.
fn name_from_title(title: &str) -> Option<String> {
    let line = title.lines().map(str::trim).find(|l| !l.is_empty())?;
    if line.chars().count() <= TITLE_NAME_LEN {
        return Some(line.to_owned());
    }
    let cut: String = line.chars().take(TITLE_NAME_LEN - 1).collect();
    Some(format!("{}…", cut.trim_end()))
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

/// The diff of one file between two of its versions (git's bytes, None where it isn't there).
/// `change` says which side has no file, so a missing side there is empty rather than an error.
fn text_diff(
    path: &str,
    before: Option<Vec<u8>>,
    after: Option<Vec<u8>>,
    change: ChangeKind,
) -> Result<Vec<crate::session::DiffLine>, CoreError> {
    let side = |bytes: Option<Vec<u8>>, absent: bool| match (bytes, absent) {
        (None, true) => Ok(String::new()),
        (None, false) => Err(format!("{path} has no text to compare here (a submodule?)")),
        (Some(bytes), _) => documents::decode_text(bytes)
            .ok_or_else(|| format!("{path} isn't text, so there's no diff to show")),
    };
    let before = side(before, change == ChangeKind::Added).map_err(CoreError::File)?;
    let after = side(after, change == ChangeKind::Deleted).map_err(CoreError::File)?;
    Ok(crate::diff::unified_diff(
        &before,
        &after,
        MAX_VIEW_DIFF_LINES,
    ))
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
    /// The GitHub CLI that opens pull requests; `None` runs `gh` from `PATH`.
    pub gh: Option<PathBuf>,
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
            gh: None,
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
/// A fetch on window focus waits at least this long since the Worktree was last fetched.
const FOCUS_FETCH_EVERY: Duration = Duration::from_secs(5 * 60);
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

/// A conversation the Agent has for a Worktree that isn't one of the editor's Tabs or Recent
/// sessions (one started in a terminal, say), which `open_conversation` opens in a Tab.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OtherConversation {
    /// The ACP session id (Claude Code's own), which `open_conversation` takes.
    pub acp_id: String,
    /// What the Agent calls it (its summary or first prompt), if anything.
    pub title: Option<String>,
    pub worktree: PathBuf,
    /// When it last changed, as the Agent says (ISO 8601).
    pub updated_at: Option<String>,
}

/// How long a Tab named after a conversation's title may be (in characters).
const TITLE_NAME_LEN: usize = 40;

/// How many pages of `session/list` are read at most.
const LIST_PAGES: usize = 10;

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
    #[error("a session here is in the middle of a turn ({}); switch branches once it's done", .0.join(", "))]
    SessionsWorking(Vec<String>),
    #[error("nothing is in progress to abort")]
    NothingToAbort,
    #[error("`{0}` isn't a branch, tag or commit here, so it can't be the Base")]
    UnknownBase(String),
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
    /// A file or image that can't go with a prompt, and why.
    #[error("{0}")]
    Attachment(String),
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
    #[error("couldn't save the Recent Workspaces: {0}")]
    StateNotSaved(String),
    #[error("the settings file has an error; fix it in the file first")]
    SettingsInvalid,
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
    #[error("that terminal isn't running")]
    NoTerminal,
    #[error("{0}")]
    Terminal(String),
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

/// One of the Recent Workspaces, as the Workspace picker lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentWorkspace {
    /// The main checkout's root.
    pub root: PathBuf,
    pub name: String,
    /// When it was last opened (ms since the Unix epoch); none if before this was recorded.
    pub opened: Option<u64>,
    /// How many Tabs opening it restores.
    pub sessions: usize,
    /// Its folder is there (one that isn't can't be opened, but stays listed).
    pub exists: bool,
    /// Byte offsets into `root` of the characters the filter matched.
    pub indices: Vec<u32>,
}

/// How many Recent Workspaces the picker lists.
pub const RECENT_WORKSPACES: usize = 20;

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
    /// The slash commands (and skills) the Agent offers in this session changed (it sends the
    /// whole list each time).
    #[serde(rename_all = "camelCase")]
    AvailableCommandsChanged {
        session_id: SessionId,
        commands: Vec<SlashCommand>,
    },
    /// A shell started or went (exited, was stopped, or its Worktree or Workspace went): every
    /// Worktree's running shells, oldest first.
    TerminalsChanged {
        terminals: Vec<TerminalInfo>,
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
    /// A Worktree's git status may have changed (files changed, staged, committed by an Agent or
    /// anyone, once a burst of changes has settled; or its remote refs, by a fetch, push or pull):
    /// for the Git drawer to look again.
    GitStatusChanged {
        worktree: PathBuf,
    },
    /// A session's Edit notes changed: a file it read or edited was saved by hand, a note was
    /// removed, or the notes went with a prompt (none left).
    #[serde(rename_all = "camelCase")]
    EditNotesChanged {
        session_id: SessionId,
        notes: Vec<EditNote>,
    },
    /// A session's queued message changed: queued, added to, taken back to be edited, removed, or
    /// sent (`None` then).
    #[serde(rename_all = "camelCase")]
    QueuedPromptChanged {
        session_id: SessionId,
        queued: Option<QueuedPrompt>,
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
    /// Slash-command lists that arrived for a session before it was registered (the adapter sends
    /// a new session's list right after creating it), by ACP id; `register_session` takes them.
    early_commands: Mutex<HashMap<String, Vec<SlashCommand>>>,
    /// The visible Tabs by view slot (the Tabs view's, or a column's), each with the switch that
    /// ends its stream when another is shown there.
    visible_tabs: Mutex<HashMap<String, (Arc<Session>, oneshot::Sender<()>)>>,
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
    /// When the Workspace's repo was last fetched (by `clock`), so a fetch on focus waits its turn.
    last_fetch: Mutex<Option<std::time::Instant>>,
    /// One push, fetch or pull at a time: every Worktree shares the repo's remote refs, and two
    /// fetches would fight over them.
    remote_op: tokio::sync::Mutex<()>,
    /// Files open in Manual editors, and popped-out files held for their windows.
    documents: Mutex<DocumentTracker>,
    /// The installed fonts, once they've been looked for.
    fonts: tokio::sync::OnceCell<Vec<crate::fonts::FontFamily>>,
    /// The running shells, by id (a Worktree can have several).
    terminals: Mutex<HashMap<TerminalId, Arc<Terminal>>>,
    /// Numbers each terminal (`Terminal::id`).
    next_terminal: AtomicU64,
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
    /// Each Worktree's Base, where it isn't the default (its start point, or one set for it).
    bases: BTreeMap<PathBuf, String>,
    /// The Worktrees pinned as columns of the Columns view (shown in Worktree row order).
    pinned: Vec<PathBuf>,
    /// The pinned Worktrees' column widths, as shares; none while they're even.
    column_shares: BTreeMap<PathBuf, u32>,
    /// When the Workspace was opened (ms since the Unix epoch), for the Recent Workspaces.
    opened: Option<u64>,
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
    /// The files (canonical) its tool calls read or edited: whose hand edits it's told about.
    files: Mutex<HashSet<PathBuf>>,
    /// Hand edits to those files waiting for its next prompt (kept while it's Suspended).
    edit_notes: Mutex<EditNotes>,
    /// The slash commands the Agent last said it offers (none until its process has started).
    commands: Mutex<Vec<SlashCommand>>,
    /// It has been sent a prompt, so Claude Code has a conversation for it (see `SavedSession`).
    started: AtomicBool,
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
    /// The user stopped the turn in progress (`session/cancel` sent): when it ends, the transcript
    /// says so and the queued message stays.
    stopping: bool,
    /// The message to send when the turn ends Idle.
    queued: Option<QueuedPrompt>,
    /// This turn's `session/prompt` has gone to the Agent: a Stop before then is sent once it has
    /// (or the Agent would get the cancel first and the prompt after).
    prompt_sent: bool,
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
            visible_tabs: Mutex::new(HashMap::new()),
            early_commands: Mutex::default(),
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
            last_fetch: Mutex::default(),
            remote_op: tokio::sync::Mutex::new(()),
            documents: Mutex::default(),
            fonts: tokio::sync::OnceCell::new(),
            terminals: Mutex::default(),
            next_terminal: AtomicU64::new(0),
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
    /// The fonts installed on this machine, for the settings page (found once per run, off the
    /// async threads).
    pub async fn installed_fonts(&self) -> Vec<crate::fonts::FontFamily> {
        let found = self
            .inner
            .fonts
            .get_or_init(|| async {
                tokio::task::spawn_blocking(crate::fonts::installed)
                    .await
                    .unwrap_or_default()
            })
            .await;
        found.clone()
    }

    /// The open repo's settings (its defaults if the file has no section for it).
    pub async fn repo_settings(&self) -> Result<RepoSettings, CoreError> {
        let workspace = self.workspace()?;
        let origin = git::origin_url(&workspace.root).await;
        let loaded = self.inner.settings.lock().expect("settings lock");
        Ok(loaded
            .settings
            .repo(origin.as_deref(), &workspace.root)
            .cloned()
            .unwrap_or_default())
    }

    /// Makes one change from the settings page in the settings file, keeping its comments and
    /// layout, and returns the settings now in force. A repo change goes in the open Workspace's
    /// section (added if there's none, as "Repo settings" adds it). Refused while the file doesn't
    /// parse, so nothing is written over it.
    pub async fn change_setting(&self, change: SettingChange) -> Result<LoadedSettings, CoreError> {
        let path = self
            .inner
            .config
            .settings_path
            .clone()
            .ok_or(CoreError::NoSettingsFile)?;
        let repo = match change.is_repo() {
            true => {
                let workspace = self.workspace()?;
                Some((git::origin_url(&workspace.root).await, workspace))
            }
            false => None,
        };
        let _one_at_a_time = self.inner.settings_write.lock().await;
        // Read it now: a change saved a moment ago may not have been reloaded yet.
        self.inner.reload_settings();
        let key = {
            let loaded = self.inner.settings.lock().expect("settings lock");
            if loaded.error.is_some() {
                return Err(CoreError::SettingsInvalid);
            }
            repo.as_ref().map(|(origin, workspace)| {
                match loaded
                    .settings
                    .repo_entry(origin.as_deref(), &workspace.root)
                {
                    Some((key, _)) => Ok(key.clone()),
                    None => Err(origin
                        .clone()
                        .unwrap_or_else(|| workspace.root.display().to_string())),
                }
            })
        };
        let key = match (key, &repo) {
            (Some(Ok(key)), _) => Some(key),
            // No section yet: added the way "Repo settings" adds one, comments and all.
            (Some(Err(key)), Some((_, workspace))) => {
                settings::add_repo_section(&path, &key, &workspace.name)
                    .map_err(|e| CoreError::SettingsWrite(e.to_string()))?;
                Some(key)
            }
            _ => None,
        };
        settings::change(&path, key.as_deref(), &change).map_err(CoreError::SettingsWrite)?;
        self.inner.reload_settings();
        Ok(self.inner.settings.lock().expect("settings lock").clone())
    }

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
    /// watching for Worktrees added or removed elsewhere. Another Workspace already open is closed
    /// first, as quitting would (its Tabs come back when it's opened again); the one already open
    /// is left as it is.
    pub async fn open_workspace(&self, path: &Path) -> Result<WorkspaceInfo, CoreError> {
        let workspace = self.workspace_at(path).await?;
        let open_now = self
            .inner
            .state
            .lock()
            .expect("state lock")
            .workspace
            .clone();
        match open_now {
            Some(open) if open.root == workspace.root => return Ok(open),
            Some(_) => self.close_workspace().await,
            None => {}
        }
        let listed = worktrees::list(&workspace.root).await;
        {
            let mut state = self.inner.state.lock().expect("state lock");
            state.workspace = Some(workspace.clone());
            state.worktrees = listed;
            state.opened = Some(unix_millis());
        }
        self.inner.restore(&workspace.root);
        // (So it heads the Recent Workspaces even if no Tab is ever opened in it.)
        self.inner.persist();
        self.inner.watch_worktrees(&workspace.root).await;
        self.inner.update_actors().await;
        self.start_monitor();
        Ok(workspace)
    }

    /// Closes the open Workspace (if any) the way quitting would: its Tabs are saved, to come back
    /// when it's opened again; its Agents stop; its setups, watchers and Worktree actors go.
    async fn close_workspace(&self) {
        self.inner.persist();
        let sessions: Vec<Arc<Session>> = {
            let mut state = self.inner.state.lock().expect("state lock");
            // (With no Workspace open, nothing below saves over the Tabs just saved.)
            if state.workspace.take().is_none() {
                return;
            }
            state.sessions.values().cloned().collect()
        };
        let mut stopping = tokio::task::JoinSet::new();
        for session in sessions {
            let core = self.clone();
            stopping.spawn(async move {
                session.wait_settled().await;
                // A slow close only means the process lingers a while.
                let _ = core.stop_agent(&session).await;
                core.forget_session(&session);
            });
        }
        while stopping.join_next().await.is_some() {}
        {
            let mut state = self.inner.state.lock().expect("state lock");
            for (_, run) in state.setups.drain() {
                run.stop();
            }
            // Session ids stay unique across Workspaces (a late event can't hit a new Tab).
            *state = State {
                next_session: state.next_session,
                ..State::default()
            };
        }
        self.inner.discovery.lock().expect("discovery lock").take();
        self.inner
            .visible_tabs
            .lock()
            .expect("visible tab lock")
            .clear();
        self.inner.actors.lock().expect("actors lock").clear(); // dropping one stops it
        self.inner.terminals.lock().expect("terminals lock").clear(); // (as does dropping a terminal)
        self.inner.terminals_changed();
        self.inner.status_due.lock().expect("status lock").clear();
        *self
            .inner
            .shown_worktree
            .lock()
            .expect("shown worktree lock") = None;
        *self.inner.last_fetch.lock().expect("last fetch lock") = None;
        // (Its popped-out windows are closed by now; the main window's files go with the view.)
        self.inner.documents().close_all();
        self.inner.documents_changed();
    }

    /// The Workspace that opening `path` opens: the main checkout of the repository it's in.
    async fn workspace_at(&self, path: &Path) -> Result<WorkspaceInfo, CoreError> {
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
        Ok(WorkspaceInfo { root, name })
    }

    /// The Workspace that opening `path` would open (without opening it), so the view can tell a
    /// switch from opening the one already open.
    pub async fn workspace_for(&self, path: &Path) -> Result<WorkspaceInfo, CoreError> {
        self.workspace_at(path).await
    }

    /// The Recent Workspaces matching `query` (best first; all of them, newest first, when it's
    /// empty), at most `RECENT_WORKSPACES`.
    pub fn recent_workspaces(&self, query: &str) -> Vec<RecentWorkspace> {
        let saved = self.inner.saved_state();
        let listed = app_state::recent_workspaces(&saved);
        let found: Vec<(usize, Vec<u32>)> = if query.trim().is_empty() {
            (0..listed.len()).map(|i| (i, vec![])).collect()
        } else {
            let roots: Vec<String> = listed
                .iter()
                .map(|(root, _)| root.to_string_lossy().into_owned())
                .collect();
            crate::files::find(&roots, query, RECENT_WORKSPACES)
                .into_iter()
                .filter_map(|m| Some((roots.iter().position(|r| *r == m.path)?, m.indices)))
                .collect()
        };
        found
            .into_iter()
            .take(RECENT_WORKSPACES)
            .map(|(i, indices)| {
                let (root, saved) = listed[i];
                RecentWorkspace {
                    root: root.clone(),
                    name: root
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    opened: saved.opened,
                    sessions: saved.tabs.len(),
                    exists: root.is_dir(),
                    indices,
                }
            })
            .collect()
    }

    /// Takes a Workspace off the Recent Workspaces; its saved Tabs and Recent sessions stay, and
    /// opening it again lists it again.
    pub fn remove_recent_workspace(&self, root: &Path) -> Result<(), CoreError> {
        let Some(path) = self
            .inner
            .config
            .state_path
            .as_ref()
            .filter(|_| self.inner.state_writable)
        else {
            return Ok(());
        };
        let _in_order = self.inner.persist_lock.lock().expect("persist lock");
        app_state::hide_workspace(path, root).map_err(CoreError::StateNotSaved)
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

    /// Every running shell, oldest first.
    pub fn terminals(&self) -> Vec<TerminalInfo> {
        self.inner.terminal_list()
    }

    /// A shell of the Worktree for the Terminal panel in view slot `slot` (the Tabs view's, or a
    /// column's): `id` if it's still running there, else the Worktree's oldest, else a new one
    /// started at `cols` x `rows` in the repo's shell. Its kept output comes first, then what it
    /// prints from now on. Whatever the slot showed before stops sending there (it keeps running).
    pub async fn open_terminal(
        &self,
        slot: &str,
        worktree: &Path,
        id: Option<TerminalId>,
        cols: u16,
        rows: u16,
    ) -> Result<(TerminalInfo, TerminalStream), CoreError> {
        let worktree = worktrees::normalize(worktree.to_owned());
        let existing = {
            let terminals = self.inner.terminals.lock().expect("terminals lock");
            id.and_then(|id| {
                terminals
                    .get(&id)
                    .filter(|t| t.worktree == worktree)
                    .cloned()
            })
            .or_else(|| {
                terminals
                    .values()
                    .filter(|t| t.worktree == worktree)
                    .min_by_key(|t| t.id)
                    .cloned()
            })
        };
        let (terminal, returning) = match existing {
            Some(terminal) => {
                terminal.resize(cols, rows).map_err(CoreError::Terminal)?;
                (terminal, true)
            }
            None => (
                self.start_terminal(&worktree, cols, rows, None).await?,
                false,
            ),
        };
        Ok(self.show_terminal(slot, &terminal, returning))
    }

    /// Starts another shell in the Worktree and shows it in view slot `slot`'s Terminal panel.
    pub async fn new_terminal(
        &self,
        slot: &str,
        worktree: &Path,
        cols: u16,
        rows: u16,
    ) -> Result<(TerminalInfo, TerminalStream), CoreError> {
        let worktree = worktrees::normalize(worktree.to_owned());
        let terminal = self.start_terminal(&worktree, cols, rows, None).await?;
        Ok(self.show_terminal(slot, &terminal, false))
    }

    fn show_terminal(
        &self,
        slot: &str,
        terminal: &Arc<Terminal>,
        returning: bool,
    ) -> (TerminalInfo, TerminalStream) {
        for other in self
            .inner
            .terminals
            .lock()
            .expect("terminals lock")
            .values()
        {
            if other.id != terminal.id {
                other.detach(slot);
            }
        }
        (terminal.info(), terminal.attach(slot, returning))
    }

    async fn start_terminal(
        &self,
        worktree: &Path,
        cols: u16,
        rows: u16,
        action: Option<ActionShell>,
    ) -> Result<Arc<Terminal>, CoreError> {
        let root = self.workspace()?.root;
        if !self.worktrees().iter().any(|w| w.path == worktree) {
            return Err(CoreError::UnknownWorktree(worktree.to_owned()));
        }
        if self
            .inner
            .state
            .lock()
            .expect("state lock")
            .removing
            .contains(worktree)
        {
            return Err(CoreError::BeingRemoved);
        }
        let shell = terminal::shell(self.repo_shell(&root).await)
            .await
            .map_err(CoreError::Terminal)?;
        let id = self.inner.next_terminal.fetch_add(1, Ordering::Relaxed);
        let weak = Arc::downgrade(&self.inner);
        let terminal = Terminal::start(id, shell, worktree, cols, rows, action, move || {
            // An exited shell goes; the panel asking again starts another.
            if let Some(inner) = weak.upgrade() {
                let gone = inner
                    .terminals
                    .lock()
                    .expect("terminals lock")
                    .remove(&id)
                    .is_some();
                if gone {
                    inner.terminals_changed();
                }
            }
        })
        .map_err(CoreError::Terminal)?;
        self.inner
            .terminals
            .lock()
            .expect("terminals lock")
            .insert(id, terminal.clone());
        self.inner.terminals_changed();
        Ok(terminal)
    }

    /// Runs the open repo's Action `name` in the Worktree: a new shell per command, named after it.
    /// Shells it started there before are stopped first, so running it again restarts it. The
    /// shells aren't shown anywhere yet (a Terminal panel opens them by id).
    pub async fn run_action(
        &self,
        worktree: &Path,
        name: &str,
    ) -> Result<Vec<TerminalInfo>, CoreError> {
        let worktree = worktrees::normalize(worktree.to_owned());
        self.inner.reload_settings(); // (a save a moment ago may not have been reloaded yet)
        let action = self
            .repo_settings()
            .await?
            .actions
            .into_iter()
            .find(|a| a.name == name)
            .ok_or_else(|| CoreError::Terminal(format!("there's no Action called {name:?}")))?;
        let stopping: Vec<TerminalId> = {
            let terminals = self.inner.terminals.lock().expect("terminals lock");
            terminals
                .values()
                .filter(|t| t.worktree == worktree)
                .filter(|t| t.action.as_ref().is_some_and(|a| a.action == action.name))
                .map(|t| t.id)
                .collect()
        };
        for id in stopping {
            self.close_terminal(id).await;
        }
        let several = action.run.len() > 1;
        let mut started = vec![];
        for command in &action.run {
            let shell = ActionShell {
                action: action.name.clone(),
                label: match several {
                    true => command.clone(),
                    false => action.name.clone(),
                },
                command: command.clone(),
            };
            // (The panel that shows it sets its size.)
            let terminal = self.start_terminal(&worktree, 80, 24, Some(shell)).await?;
            started.push(terminal.info());
        }
        Ok(started)
    }

    /// Stops an Action's shell and runs its command again in a new one (in its place in the
    /// Worktree's list of shells only by being the newest).
    pub async fn restart_terminal(&self, id: TerminalId) -> Result<TerminalInfo, CoreError> {
        let old = self.terminal(id).ok_or(CoreError::NoTerminal)?;
        let action = old
            .action
            .clone()
            .ok_or_else(|| CoreError::Terminal("only an Action's shell can be restarted".into()))?;
        let worktree = old.worktree.clone();
        drop(old);
        self.close_terminal(id).await;
        let terminal = self.start_terminal(&worktree, 80, 24, Some(action)).await?;
        Ok(terminal.info())
    }

    /// Types `data` (keystrokes or a paste) into a shell.
    pub fn terminal_input(&self, id: TerminalId, data: &str) -> Result<(), CoreError> {
        self.terminal(id).ok_or(CoreError::NoTerminal)?.write(data);
        Ok(())
    }

    /// The Terminal panel's new size, in characters.
    pub fn resize_terminal(&self, id: TerminalId, cols: u16, rows: u16) -> Result<(), CoreError> {
        self.terminal(id)
            .ok_or(CoreError::NoTerminal)?
            .resize(cols, rows)
            .map_err(CoreError::Terminal)
    }

    /// View slot `slot`'s Terminal panel is hidden: no terminal sends it anything (they keep
    /// running).
    pub fn hide_terminal(&self, slot: &str) {
        for terminal in self
            .inner
            .terminals
            .lock()
            .expect("terminals lock")
            .values()
        {
            terminal.detach(slot);
        }
    }

    /// Stops a shell and everything it started.
    pub async fn close_terminal(&self, id: TerminalId) {
        let terminal = self
            .inner
            .terminals
            .lock()
            .expect("terminals lock")
            .remove(&id);
        if let Some(terminal) = terminal {
            self.inner.terminals_changed();
            terminal.stop().await;
        }
    }

    /// Stops every shell of the Worktree.
    async fn close_terminals_in(&self, worktree: &Path) {
        let worktree = worktrees::normalize(worktree.to_owned());
        let stopping: Vec<Arc<Terminal>> = {
            let mut terminals = self.inner.terminals.lock().expect("terminals lock");
            let ids: Vec<TerminalId> = terminals
                .values()
                .filter(|t| t.worktree == worktree)
                .map(|t| t.id)
                .collect();
            ids.iter().filter_map(|id| terminals.remove(id)).collect()
        };
        if stopping.is_empty() {
            return;
        }
        self.inner.terminals_changed();
        let mut stops = tokio::task::JoinSet::new();
        for terminal in stopping {
            stops.spawn(async move { terminal.stop().await });
        }
        while stops.join_next().await.is_some() {}
    }

    fn terminal(&self, id: TerminalId) -> Option<Arc<Terminal>> {
        self.inner
            .terminals
            .lock()
            .expect("terminals lock")
            .get(&id)
            .cloned()
    }

    /// The shell the repo's settings choose for Windows (its setup commands' and terminals').
    async fn repo_shell(&self, root: &Path) -> WindowsShell {
        let origin = git::origin_url(root).await;
        // A save a moment ago may not have been reloaded yet.
        self.inner.reload_settings();
        let loaded = self.inner.settings.lock().expect("settings lock");
        loaded
            .settings
            .repo(origin.as_deref(), root)
            .map(|repo| repo.windows_shell)
            .unwrap_or_default()
    }

    /// Creates a Worktree next to the repo (`<repo>.worktrees/<folder>/`) and returns it, listed.
    /// A new branch starts from `start_point`, or by default from `origin/<default>` after a fetch.
    pub async fn create_worktree(&self, spec: NewWorktree) -> Result<CreatedWorktree, CoreError> {
        let root = self.workspace()?.root;
        let (path, warning) = create_worktree::create(&root, &spec).await?;
        self.inner.refresh_worktrees().await;
        let path = worktrees::normalize(path);
        // (Branched off a branch other than the default: that's what it's compared against.)
        if let NewWorktree::NewBranch {
            start_point: Some(start),
            ..
        } = &spec
        {
            self.inner
                .state
                .lock()
                .expect("state lock")
                .bases
                .insert(path.clone(), start.clone());
            self.inner.persist();
        }
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
        // Its shells stand in the folder (on Windows, enough to stop it being deleted).
        self.close_terminals_in(&info.path).await;
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
        let (base, base_id) = self.resolved_base(root, &info.path).await?;
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

    /// The Worktree's Base, and the commit it names. By commit id, resolved in the main checkout:
    /// in the Worktree, a Base of `HEAD` (or a name its own branch shadows) would mean its own
    /// HEAD, and everything would look merged.
    async fn resolved_base(
        &self,
        root: &Path,
        worktree: &Path,
    ) -> Result<(String, String), CoreError> {
        let (base, _) = self.base_of(worktree).await;
        let base_id = git::resolve_commit(root, &base)
            .await
            .ok_or_else(|| CoreError::Git(format!("couldn't find the Base `{base}`")))?;
        Ok((base, base_id))
    }

    /// Every Worktree, main checkout first, with whether its branch is merged into its Base: for
    /// the Worktrees overview, where merged ones can be removed. Uses the remote branches as they
    /// are (fetch first for news of merged PRs).
    pub async fn merge_overview(&self) -> Result<Vec<WorktreeMerge>, CoreError> {
        let root = self.workspace()?.root;
        let mut rows = vec![];
        for info in self.worktrees() {
            let (base, _) = self.base_of(&info.path).await;
            let mut row = WorktreeMerge {
                path: info.path.clone(),
                branch: info.branch.clone(),
                is_main: info.is_main,
                base,
                merge: None,
                error: None,
                changed: info.changed,
            };
            if !info.is_main {
                match self.resolved_base(&root, &info.path).await {
                    Ok((base, base_id)) => {
                        let branch = info.branch.as_deref();
                        row.merge = Some(merged::detect(&info.path, branch, &base, &base_id).await)
                    }
                    Err(err) => row.error = Some(err.to_string()),
                }
            }
            rows.push(row);
        }
        Ok(rows)
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
        let _ = session.send_cancel(&connection);
        // An error reply (e.g. the adapter already dropped it, or exited) means it's gone too.
        connection.supports_session("close")
            && close_acp(&connection, &session.acp_id).await.is_some()
    }

    /// Drops a closed session from the editor and ends its Tab's stream.
    fn forget_session(&self, session: &Arc<Session>) {
        self.inner.forget_session(session);
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
                    files: vec![],
                    started: false,
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

    /// The Worktrees pinned as columns of the Columns view (ones that are gone aren't listed).
    pub fn pinned_worktrees(&self) -> Vec<PathBuf> {
        let state = self.inner.state.lock().expect("state lock");
        state
            .pinned
            .iter()
            .filter(|p| state.worktrees.iter().any(|w| &&w.path == p))
            .cloned()
            .collect()
    }

    /// Pins or unpins a Worktree as a column of the Columns view; remembered across restarts. A
    /// column added or closed evens the columns' widths out again.
    pub fn set_pinned(&self, worktree: &Path, pinned: bool) -> Result<Vec<PathBuf>, CoreError> {
        let worktree = self.known_worktree(worktree)?;
        {
            let mut state = self.inner.state.lock().expect("state lock");
            let was = state.pinned.contains(&worktree);
            state.pinned.retain(|p| *p != worktree);
            if pinned {
                state.pinned.push(worktree);
            }
            if was != pinned {
                state.column_shares.clear();
            }
        }
        self.inner.persist();
        Ok(self.pinned_worktrees())
    }

    /// The pinned Worktrees' column widths, as shares of the row: a column is as wide as its share
    /// of their sum (1000 each when they're even, and for one that has none).
    pub fn column_shares(&self) -> BTreeMap<PathBuf, u32> {
        self.inner
            .state
            .lock()
            .expect("state lock")
            .column_shares
            .clone()
    }

    /// Sets the columns' widths (shares, as `column_shares` has them); remembered across
    /// restarts. Only pinned Worktrees can have one; a share of 0 counts as 1.
    pub fn set_column_shares(&self, shares: BTreeMap<PathBuf, u32>) -> Result<(), CoreError> {
        let mut known = BTreeMap::new();
        for (worktree, share) in shares {
            known.insert(self.known_worktree(&worktree)?, share.max(1));
        }
        {
            let mut state = self.inner.state.lock().expect("state lock");
            known.retain(|worktree, _| state.pinned.contains(worktree));
            state.column_shares = known;
        }
        self.inner.persist();
        Ok(())
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
        let after = text.to_owned();
        let text = text.to_owned();
        let line_ending = line_ending.to_owned();
        // (While it writes, the file watcher's news of it isn't an Agent's change.)
        self.inner.documents().save_started(&path);
        let saved = tokio::task::spawn_blocking(move || {
            // What it was, for the Edit notes (`\n` line endings, like the editor's text).
            let before = std::fs::read(&target)
                .ok()
                .and_then(|bytes| String::from_utf8(bytes).ok())
                .map(|text| text.replace("\r\n", "\n"));
            documents::save(&target, &text, &line_ending, &over)
                .map(|version| (version, before))
                .map_err(|err| match err {
                    documents::SaveError::Changed => CoreError::FileChangedOnDisk(target.clone()),
                    documents::SaveError::Io(message) => CoreError::File(message),
                })
        })
        .await
        .map_err(|e| CoreError::File(e.to_string()))
        .and_then(|saved| saved)
        .map(|(version, before)| {
            self.inner
                .note_edit(&path, before.as_deref().unwrap_or_default(), &after);
            version
        });
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

    /// The Git drawer: the Worktree's branch, upstream standing and changed files.
    pub async fn git_status(&self, worktree: &Path) -> Result<GitStatus, CoreError> {
        let worktree = self.known_worktree(worktree)?;
        let mut status = git::status(&worktree).await.map_err(CoreError::Git)?;
        status.mid_turn = self.mid_turn(&worktree);
        Ok(status)
    }

    /// Switches the Worktree to another branch (a local one, or a remote one, which gets a local
    /// branch tracking it). Refused while a session there is mid-turn, and for a branch checked
    /// out in another Worktree. Its sessions stay with it: they're bound to the Worktree, not the
    /// branch.
    pub async fn switch_branch(&self, worktree: &Path, branch: &str) -> Result<(), CoreError> {
        let worktree = self.known_worktree(worktree)?;
        let working = self.mid_turn(&worktree);
        if !working.is_empty() {
            return Err(CoreError::SessionsWorking(working));
        }
        let root = self.workspace()?.root;
        let branches = git::branches(&root).await.map_err(CoreError::Git)?;
        let checkout = crate::create_worktree::resolve_checkout(&branches, branch)?;
        match checkout.checked_out_in {
            Some(here) if here == worktree => return Ok(()), // (already on it)
            Some(elsewhere) => {
                return Err(CoreError::BranchCheckedOut {
                    branch: checkout.local,
                    worktree: elsewhere,
                })
            }
            None => {}
        }
        let switched = git::switch(&worktree, &checkout.local, checkout.track.as_deref())
            .await
            .map_err(CoreError::Git);
        self.inner.git_changed(worktree.clone());
        self.inner.refresh_status_of(&worktree).await; // (its branch, in the Worktree list)
        switched
    }

    /// "Changes vs base": the files the Worktree's branch changed since it split from its Base,
    /// with what kind of change each is, and the split (the merge-base) the diffs run from.
    pub async fn changes_vs_base(&self, worktree: &Path) -> Result<BaseChanges, CoreError> {
        let worktree = self.known_worktree(worktree)?;
        let (base, is_default) = self.base_of(&worktree).await;
        if !git::is_commit(&worktree, &base).await {
            return Err(CoreError::UnknownBase(base));
        }
        let split = git::merge_base(&worktree, &base)
            .await
            .map_err(CoreError::Git)?;
        let files = git::changes_since(&worktree, &split)
            .await
            .map_err(CoreError::Git)?
            .into_iter()
            .map(|(letter, path, renamed_from)| BaseChange {
                path,
                change: ChangeKind::from_letter(letter),
                renamed_from,
            })
            .collect();
        Ok(BaseChanges {
            base,
            is_default,
            split,
            files,
        })
    }

    /// Sets the Worktree's Base (None: back to the default, `origin/<default>`). It's checked in
    /// the Worktree, where `HEAD~2` means this branch's, and remembered across restarts.
    pub async fn set_base(&self, worktree: &Path, base: Option<&str>) -> Result<(), CoreError> {
        let worktree = self.known_worktree(worktree)?;
        let base = base.map(str::trim).filter(|b| !b.is_empty());
        if let Some(base) = base {
            if !git::is_commit(&worktree, base).await {
                return Err(CoreError::UnknownBase(base.to_owned()));
            }
        }
        {
            let mut state = self.inner.state.lock().expect("state lock");
            match base {
                Some(base) => state.bases.insert(worktree.clone(), base.to_owned()),
                None => state.bases.remove(&worktree),
            };
        }
        self.inner.persist();
        self.inner.git_changed(worktree);
        Ok(())
    }

    /// One file's change since the branch split from its Base: from its text at `split` (under
    /// `renamed_from`, for a rename) to its text at HEAD. `change` says which side has no file.
    pub async fn diff_vs_base(
        &self,
        worktree: &Path,
        split: &str,
        path: &str,
        renamed_from: Option<&str>,
        change: ChangeKind,
    ) -> Result<Vec<crate::session::DiffLine>, CoreError> {
        let worktree = self.known_worktree(worktree)?;
        let before = git::file_at(&worktree, split, renamed_from.unwrap_or(path)).await;
        let after = git::file_at(&worktree, "HEAD", path).await;
        text_diff(path, before, after, change)
    }

    /// One file's working change, as the Git drawer lists it: a staged one from HEAD (under
    /// `renamed_from`, for a rename) to the index; an unstaged one from the index to the file on
    /// disk. `change` says which side has no file.
    pub async fn diff_working(
        &self,
        worktree: &Path,
        path: &str,
        renamed_from: Option<&str>,
        change: ChangeKind,
        staged: bool,
    ) -> Result<Vec<crate::session::DiffLine>, CoreError> {
        let worktree = self.known_worktree(worktree)?;
        let relative = Path::new(path);
        if relative.is_absolute()
            || relative
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Err(CoreError::File(format!("{path} isn't in the Worktree")));
        }
        let (before, after) = if staged {
            (
                git::file_at(&worktree, "HEAD", renamed_from.unwrap_or(path)).await,
                git::file_at(&worktree, "", path).await,
            )
        } else {
            let on_disk = worktree.join(relative);
            (
                git::file_at(&worktree, "", path).await,
                tokio::task::spawn_blocking(move || std::fs::read(on_disk))
                    .await
                    .ok()
                    .and_then(Result::ok),
            )
        };
        text_diff(path, before, after, change)
    }

    /// The Worktree's Base: the one set for it (or that it was created from in the editor), else
    /// the branch its branch was made from (as its reflog says), else the default. Whether none was
    /// set for it.
    async fn base_of(&self, worktree: &Path) -> (String, bool) {
        let set = self
            .inner
            .state
            .lock()
            .expect("state lock")
            .bases
            .get(worktree)
            .cloned();
        if let Some(base) = set {
            return (base, false);
        }
        // Not set here: the branch it was made from (outside the editor too, as git recorded it),
        // which is where its PR most likely goes, else the default.
        let branch = self
            .worktrees()
            .into_iter()
            .find(|w| w.path == worktree)
            .and_then(|w| w.branch);
        if let Some(branch) = branch {
            if let Some(from) = git::created_from(worktree, &branch).await {
                return (from, true);
            }
        }
        match self.workspace() {
            Ok(workspace) => (git::default_start_point(&workspace.root).await, true),
            Err(_) => ("HEAD".into(), true),
        }
    }

    /// "Review" (ticket 42): every file the Worktree's PR would change, from where its branch split
    /// from its Base to the files on disk (uncommitted and untracked ones too), with its commits
    /// and the branches the PR could go into.
    pub async fn review(&self, worktree: &Path) -> Result<Review, CoreError> {
        let worktree = self.known_worktree(worktree)?;
        let (base, _) = self.base_of(&worktree).await;
        if !git::is_commit(&worktree, &base).await {
            return Err(CoreError::UnknownBase(base));
        }
        let split = git::merge_base(&worktree, &base)
            .await
            .map_err(CoreError::Git)?;
        let status = git::status(&worktree).await.map_err(CoreError::Git)?;
        let files = git::changes_to_disk(&worktree, &split)
            .await
            .map_err(CoreError::Git)?
            .into_iter()
            .map(|(letter, path, renamed_from)| ReviewFile {
                uncommitted: status
                    .files
                    .iter()
                    .any(|f| f.path == path || renamed_from.as_deref() == Some(f.path.as_str())),
                change: ChangeKind::from_letter(letter),
                path,
                renamed_from,
            })
            .collect();
        let remote = match &status.branch {
            Some(branch) => git::push_remote(&worktree, branch).await,
            None => "origin".to_owned(),
        };
        let branches = git::branches(&worktree).await.map_err(CoreError::Git)?;
        let targets: Vec<String> = branches
            .iter()
            .filter(|b| b.remote)
            .filter_map(|b| crate::pull_request::on_remote(&b.name, &remote))
            .filter(|b| status.branch.as_deref() != Some(*b))
            .map(str::to_owned)
            .collect();
        // The Base's branch on the remote, else the remote's default branch.
        let default = match self.workspace() {
            Ok(workspace) => git::default_start_point(&workspace.root).await,
            Err(_) => String::new(),
        };
        let target = [base.as_str(), default.as_str()]
            .into_iter()
            .find_map(|b| {
                let name = crate::pull_request::on_remote(b, &remote).unwrap_or(b);
                targets.iter().find(|t| *t == name).cloned()
            })
            .or_else(|| {
                targets
                    .iter()
                    .find(|t| *t == "main" || *t == "master")
                    .cloned()
            })
            .unwrap_or_default();
        Ok(Review {
            commits: git::commits_since(&worktree, &split)
                .await
                .into_iter()
                .map(|(id, short_id, subject)| ReviewCommit {
                    id,
                    short_id,
                    subject,
                })
                .collect(),
            uncommitted: status.files.len() as u32,
            operation_in_progress: status.operation.is_some(),
            branch: status.branch,
            base,
            split,
            files,
            remote,
            targets,
            target,
        })
    }

    /// One file's change in the review: from its text at `split` (under `renamed_from`, for a
    /// rename) to the file on disk. `change` says which side has no file.
    pub async fn review_diff(
        &self,
        worktree: &Path,
        split: &str,
        path: &str,
        renamed_from: Option<&str>,
        change: ChangeKind,
    ) -> Result<Vec<crate::session::DiffLine>, CoreError> {
        let worktree = self.known_worktree(worktree)?;
        let relative = Path::new(path);
        if relative.is_absolute()
            || relative
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Err(CoreError::File(format!("{path} isn't in the Worktree")));
        }
        let before = git::file_at(&worktree, split, renamed_from.unwrap_or(path)).await;
        let on_disk = worktree.join(relative);
        let after = tokio::task::spawn_blocking(move || std::fs::read(on_disk))
            .await
            .ok()
            .and_then(Result::ok);
        text_diff(path, before, after, change)
    }

    /// What one of the review's commits changed on its own (against its first parent), and the
    /// parent each file's diff runs from (`commit_diff`).
    pub async fn commit_changes(
        &self,
        worktree: &Path,
        commit: &str,
    ) -> Result<CommitChanges, CoreError> {
        let worktree = self.known_worktree(worktree)?;
        if !git::is_commit(&worktree, commit).await {
            return Err(CoreError::Git(format!("`{commit}` isn't a commit here.")));
        }
        let (parent, changes) = git::commit_changes(&worktree, commit)
            .await
            .map_err(CoreError::Git)?;
        let files = changes
            .into_iter()
            .map(|(letter, path, renamed_from)| ReviewFile {
                change: ChangeKind::from_letter(letter),
                path,
                renamed_from,
                uncommitted: false,
            })
            .collect();
        Ok(CommitChanges { parent, files })
    }

    /// One file's change in one commit: from its text at `parent` (under `renamed_from`, for a
    /// rename) to its text at `commit`. `change` says which side has no file.
    pub async fn commit_diff(
        &self,
        worktree: &Path,
        parent: &str,
        commit: &str,
        path: &str,
        renamed_from: Option<&str>,
        change: ChangeKind,
    ) -> Result<Vec<crate::session::DiffLine>, CoreError> {
        let worktree = self.known_worktree(worktree)?;
        let before = git::file_at(&worktree, parent, renamed_from.unwrap_or(path)).await;
        let after = git::file_at(&worktree, commit, path).await;
        text_diff(path, before, after, change)
    }

    /// "Create PR": commits whatever isn't committed yet (all of it, untracked files too) with the
    /// request's commit message, pushes the branch, and opens a PR from it into `target` with the
    /// GitHub CLI, assigned to whoever it's logged in as. Asks first (doing nothing) while a session
    /// in the Worktree is Working, unless told to go ahead. Each step, and the output of what it
    /// runs (hooks, git's progress, `gh`), goes to `progress` as it happens.
    pub async fn create_pull_request(
        &self,
        worktree: &Path,
        request: PullRequestRequest,
        progress: impl Fn(PullRequestProgress) + Send + Sync,
    ) -> Result<PullRequestOutcome, CoreError> {
        let step = |text: String| progress(PullRequestProgress::Step { text });
        let output = |text: &str| {
            progress(PullRequestProgress::Output {
                text: text.to_owned(),
            })
        };
        let worktree = self.known_worktree(worktree)?;
        let target = request.target.trim();
        let title = request.title.trim();
        if target.is_empty() {
            return Err(CoreError::Git("Choose the branch the PR goes into.".into()));
        }
        if title.is_empty() {
            return Err(CoreError::Git("A PR needs a title.".into()));
        }
        let Some(branch) = git::current_branch(&worktree).await else {
            return Err(CoreError::Git(
                "HEAD is detached: check out a branch to make a PR from.".into(),
            ));
        };
        if branch == target {
            return Err(CoreError::Git(format!(
                "This Worktree is on {target} itself: a PR needs a branch of its own."
            )));
        }
        if !request.even_if_working {
            let working = self.mid_turn(&worktree);
            if !working.is_empty() {
                return Ok(PullRequestOutcome::SessionsWorking { sessions: working });
            }
        }
        let message = match request.commit_message.trim() {
            "" => title,
            message => message,
        };
        let _one_at_a_time = self.inner.remote_op.lock().await;
        let (committed, pushed) = self
            .commit_and_push(&worktree, &branch, message, &progress)
            .await?;
        if pushed == PushOutcome::Rejected {
            return Ok(PullRequestOutcome::PushRejected { committed });
        }
        let gh = crate::pull_request::gh_program(self.inner.config.gh.as_ref());
        step(format!("Opening the PR into {target}"));
        let opened = crate::pull_request::create(
            &gh,
            &worktree,
            &branch,
            target,
            title,
            &request.body,
            request.draft,
            &output,
        )
        .await
        .map_err(CoreError::Git)?;
        Ok(match opened {
            crate::pull_request::Opened::Created(url) => {
                PullRequestOutcome::Created { url, committed }
            }
            crate::pull_request::Opened::AlreadyOpen(url) => {
                PullRequestOutcome::AlreadyOpen { url, committed }
            }
        })
    }

    /// The PR open from the Worktree's branch, if there is one (asked of the GitHub CLI).
    pub async fn open_pull_request(
        &self,
        worktree: &Path,
    ) -> Result<Option<OpenPullRequest>, CoreError> {
        let worktree = self.known_worktree(worktree)?;
        let Some(branch) = git::current_branch(&worktree).await else {
            return Ok(None);
        };
        let gh = crate::pull_request::gh_program(self.inner.config.gh.as_ref());
        crate::pull_request::open_for(&gh, &worktree, &branch)
            .await
            .map_err(CoreError::Git)
    }

    /// The PR of the Worktree's branch (its open one, else its latest), with its reviews and
    /// comments, if it has one (asked of the GitHub CLI).
    pub async fn pull_request_details(
        &self,
        worktree: &Path,
    ) -> Result<Option<PullRequestDetails>, CoreError> {
        let worktree = self.known_worktree(worktree)?;
        let Some(branch) = git::current_branch(&worktree).await else {
            return Ok(None);
        };
        let gh = crate::pull_request::gh_program(self.inner.config.gh.as_ref());
        crate::pull_request::details_for(&gh, &worktree, &branch)
            .await
            .map_err(CoreError::Git)
    }

    /// "Push", for a branch whose PR is already open: commits whatever isn't committed yet with
    /// `commit_message` and pushes, telling `progress` as it goes. Asks first (doing nothing) while
    /// a session in the Worktree is Working, unless `even_if_working`.
    pub async fn push_to_pull_request(
        &self,
        worktree: &Path,
        commit_message: &str,
        even_if_working: bool,
        progress: impl Fn(PullRequestProgress) + Send + Sync,
    ) -> Result<PushToPullRequestOutcome, CoreError> {
        let worktree = self.known_worktree(worktree)?;
        let Some(branch) = git::current_branch(&worktree).await else {
            return Err(CoreError::Git(
                "HEAD is detached: check out a branch to push.".into(),
            ));
        };
        if !even_if_working {
            let working = self.mid_turn(&worktree);
            if !working.is_empty() {
                return Ok(PushToPullRequestOutcome::SessionsWorking { sessions: working });
            }
        }
        let _one_at_a_time = self.inner.remote_op.lock().await;
        let (committed, pushed) = self
            .commit_and_push(&worktree, &branch, commit_message.trim(), &progress)
            .await?;
        Ok(match pushed {
            PushOutcome::Pushed { to } => PushToPullRequestOutcome::Pushed { to, committed },
            PushOutcome::Rejected => PushToPullRequestOutcome::Rejected { committed },
        })
    }

    /// Commits everything that isn't committed yet (untracked files too) with `message`, then
    /// pushes `branch`, telling `progress` each step and its output. The caller holds `remote_op`.
    async fn commit_and_push(
        &self,
        worktree: &Path,
        branch: &str,
        message: &str,
        progress: &(dyn Fn(PullRequestProgress) + Send + Sync),
    ) -> Result<(Option<String>, PushOutcome), CoreError> {
        let step = |text: String| progress(PullRequestProgress::Step { text });
        let output = |text: &str| {
            progress(PullRequestProgress::Output {
                text: text.to_owned(),
            })
        };
        let status = git::status(worktree).await.map_err(CoreError::Git)?;
        if status.operation.is_some() || status.files.iter().any(|f| f.conflicted) {
            return Err(CoreError::Git(
                "A merge or rebase is in progress here: finish or abort it first.".into(),
            ));
        }
        let committed = if status.files.is_empty() {
            None
        } else {
            if message.is_empty() {
                return Err(CoreError::Git(
                    "There are uncommitted changes: give them a commit message.".into(),
                ));
            }
            let files = status.files.len();
            step(format!(
                "Committing {files} file{}",
                if files == 1 { "" } else { "s" }
            ));
            let made = async {
                git::stage_all(worktree).await?;
                git::commit(worktree, message, false, Some(&output)).await
            }
            .await;
            self.inner.status_due(worktree.to_owned());
            Some(made.map_err(CoreError::Git)?)
        };
        step(format!(
            "Pushing {branch} to {}",
            git::push_remote(worktree, branch).await
        ));
        let pushed = git::push(worktree, Some(&output)).await;
        self.inner.git_changed(worktree.to_owned());
        Ok((committed, pushed.map_err(CoreError::Git)?))
    }

    /// Aborts the merge, rebase, cherry-pick or revert in progress in the Worktree.
    pub async fn abort_operation(&self, worktree: &Path) -> Result<(), CoreError> {
        let worktree = self.known_worktree(worktree)?;
        let operation = git::operation(&worktree)
            .await
            .ok_or(CoreError::NothingToAbort)?;
        let aborted = git::abort(&worktree, operation)
            .await
            .map_err(CoreError::Git);
        self.inner.git_changed(worktree.clone());
        self.inner.refresh_status_of(&worktree).await;
        aborted
    }

    /// Stages whole files (paths relative to the Worktree, `/`-separated).
    pub async fn stage(&self, worktree: &Path, paths: &[String]) -> Result<(), CoreError> {
        let worktree = self.known_worktree(worktree)?;
        let done = git::stage(&worktree, paths).await.map_err(CoreError::Git);
        self.inner.status_due(worktree);
        done
    }

    pub async fn unstage(&self, worktree: &Path, paths: &[String]) -> Result<(), CoreError> {
        let worktree = self.known_worktree(worktree)?;
        let done = git::unstage(&worktree, paths).await.map_err(CoreError::Git);
        self.inner.status_due(worktree);
        done
    }

    pub async fn stage_all(&self, worktree: &Path) -> Result<(), CoreError> {
        let worktree = self.known_worktree(worktree)?;
        let done = git::stage_all(&worktree).await.map_err(CoreError::Git);
        self.inner.status_due(worktree);
        done
    }

    pub async fn unstage_all(&self, worktree: &Path) -> Result<(), CoreError> {
        let worktree = self.known_worktree(worktree)?;
        let done = git::unstage_all(&worktree).await.map_err(CoreError::Git);
        self.inner.status_due(worktree);
        done
    }

    /// Commits what's staged. Asks first (without committing) while a session in the Worktree is
    /// Working, and before amending a commit that's already pushed, unless told to go ahead.
    pub async fn commit(
        &self,
        worktree: &Path,
        request: CommitRequest,
    ) -> Result<CommitOutcome, CoreError> {
        let worktree = self.known_worktree(worktree)?;
        if !request.amend && request.message.trim().is_empty() {
            return Err(CoreError::Git("A commit needs a message.".into()));
        }
        if !request.even_if_working {
            let working = self.mid_turn(&worktree);
            if !working.is_empty() {
                return Ok(CommitOutcome::SessionsWorking { sessions: working });
            }
        }
        if request.amend && !request.even_if_pushed && git::head_pushed(&worktree).await {
            return Ok(CommitOutcome::AlreadyPushed);
        }
        let done = git::commit(&worktree, &request.message, request.amend, None)
            .await
            .map_err(CoreError::Git);
        self.inner.status_due(worktree);
        Ok(CommitOutcome::Committed { id: done? })
    }

    /// Fetches the Worktree's remote (which every Worktree of the repo shares). Fails fast (with a
    /// hint) if git would need a login prompt.
    pub async fn fetch(&self, worktree: &Path) -> Result<(), CoreError> {
        let worktree = self.known_worktree(worktree)?;
        let _one_at_a_time = self.inner.remote_op.lock().await;
        self.inner.fetch(&worktree).await.map_err(CoreError::Git)
    }

    /// Pushes the Worktree's branch to the branch of its name on its remote, which it then tracks.
    pub async fn push(&self, worktree: &Path) -> Result<PushOutcome, CoreError> {
        let worktree = self.known_worktree(worktree)?;
        let _one_at_a_time = self.inner.remote_op.lock().await;
        let pushed = git::push(&worktree, None).await.map_err(CoreError::Git);
        self.inner.git_changed(worktree);
        pushed
    }

    /// Fetches, then fast-forwards the branch to its upstream if it can. Never merges or rebases:
    /// a branch that has diverged is left as it is (`Diverged`). Asks first while a session there is
    /// mid-turn (the pull rewrites files under it), unless `even_if_working`.
    pub async fn pull(
        &self,
        worktree: &Path,
        even_if_working: bool,
    ) -> Result<PullOutcome, CoreError> {
        let worktree = self.known_worktree(worktree)?;
        if !even_if_working {
            let working = self.mid_turn(&worktree);
            if !working.is_empty() {
                return Ok(PullOutcome::SessionsWorking { sessions: working });
            }
        }
        let _one_at_a_time = self.inner.remote_op.lock().await;
        self.inner.fetch(&worktree).await.map_err(CoreError::Git)?;
        let Some((ahead, behind)) = git::ahead_behind(&worktree).await else {
            return Err(CoreError::Git(match git::has_upstream_set(&worktree).await {
                true => "The branch this one tracks is gone from the remote (deleted after a merge?), so there's nothing to pull.".into(),
                false => "This branch has no upstream to pull from: push it first.".into(),
            }));
        };
        let outcome = match (ahead, behind) {
            (_, 0) => PullOutcome::UpToDate,
            (0, commits) => {
                let pulled = git::fast_forward(&worktree).await.map_err(CoreError::Git);
                self.inner.git_changed(worktree);
                pulled?;
                PullOutcome::FastForwarded { commits }
            }
            (ahead, behind) => PullOutcome::Diverged { ahead, behind },
        };
        Ok(outcome)
    }

    /// The window got focus: the Worktree list may have moved on while it was in the background,
    /// and the shown Worktrees (the one looked at, and those with sessions) are fetched if the repo
    /// hasn't been for `FOCUS_FETCH_EVERY`. The Worktrees fetched for (failures are quiet: a manual
    /// fetch says what went wrong).
    pub async fn window_focused(&self) -> Vec<PathBuf> {
        self.refresh_worktrees().await;
        let live = self.inner.live_worktrees();
        let shown = self
            .inner
            .shown_worktree
            .lock()
            .expect("shown worktree lock")
            .clone();
        let Some(from) = shown
            .filter(|s| live.contains(s))
            .or_else(|| live.first().cloned())
        else {
            return vec![];
        };
        // Due, and claimed, under one lock (two focus events can't both go).
        let now = self.inner.clock.now();
        {
            let mut last_fetch = self.inner.last_fetch.lock().expect("last fetch lock");
            if last_fetch.is_some_and(|last| now.duration_since(last) < FOCUS_FETCH_EVERY) {
                return vec![];
            }
            *last_fetch = Some(now);
        }
        // (A push, fetch or pull already talking to the remote: no need for another.)
        let Ok(_one_at_a_time) = self.inner.remote_op.try_lock() else {
            return vec![];
        };
        let _ = self.inner.fetch(&from).await;
        live
    }

    /// Throws away every change to the file (staged or not): back to the last commit's version,
    /// or deleted if it's new. The drawer asks first.
    pub async fn discard(&self, worktree: &Path, path: &str) -> Result<(), CoreError> {
        let worktree = self.known_worktree(worktree)?;
        let done = git::discard(&worktree, path).await.map_err(CoreError::Git);
        self.inner.status_due(worktree);
        done
    }

    /// The names of the sessions in `worktree` in the middle of a turn (Needs you too: it edits once
    /// it's answered).
    fn mid_turn(&self, worktree: &Path) -> Vec<String> {
        self.sessions_in(worktree)
            .iter()
            .map(|s| s.info.lock().expect("info lock").clone())
            .filter(|info| matches!(info.state, SessionState::Working | SessionState::NeedsYou))
            .map(|info| info.name)
            .collect()
    }

    /// `worktree`, as the core lists it, if it's one of the Workspace's and isn't being removed.
    fn known_worktree(&self, worktree: &Path) -> Result<PathBuf, CoreError> {
        let wanted = worktrees::normalize(worktree.to_owned());
        let state = self.inner.state.lock().expect("state lock");
        if state.removing.contains(&wanted) {
            return Err(CoreError::BeingRemoved);
        }
        state
            .worktrees
            .iter()
            .map(|w| w.path.clone())
            .find(|w| *w == wanted)
            .ok_or_else(|| CoreError::UnknownWorktree(worktree.to_owned()))
    }

    /// The session's Edit notes waiting for its next prompt.
    pub fn edit_notes(&self, id: SessionId) -> Result<Vec<EditNote>, CoreError> {
        Ok(self
            .session(id)?
            .edit_notes
            .lock()
            .expect("edit notes lock")
            .notes())
    }

    /// The user removed an Edit note: the session isn't told about those changes.
    pub fn remove_edit_note(&self, id: SessionId, path: &Path) -> Result<(), CoreError> {
        let session = self.session(id)?;
        let mut notes = session.edit_notes.lock().expect("edit notes lock");
        if notes.remove(path) {
            let notes = notes.notes();
            let _ = self.inner.events.send(CoreEvent::EditNotesChanged {
                session_id: id,
                notes,
            });
        }
        Ok(())
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
        if saved.started {
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

    /// The Agent's conversations in `worktree` that the editor doesn't have (as a Tab or a Recent
    /// session): ones started in a terminal, or closed so long ago they left the Recent sessions.
    /// Newest first. None when the Agent can't list or load conversations.
    pub async fn other_conversations(
        &self,
        worktree: &Path,
    ) -> Result<Vec<OtherConversation>, CoreError> {
        let wanted = worktrees::normalize(worktree.to_owned());
        if !self.worktrees().iter().any(|w| w.path == wanted) {
            return Err(CoreError::UnknownWorktree(worktree.to_owned()));
        }
        let connection = self.adapter().await?;
        if !connection.supports_session("list") || !connection.supports_load() {
            return Ok(vec![]);
        }
        let mut listed = vec![];
        let mut cursor: Option<String> = None;
        for _ in 0..LIST_PAGES {
            let mut params = json!({ "cwd": wanted });
            if let Some(cursor) = &cursor {
                params["cursor"] = json!(cursor);
            }
            let page = connection.request("session/list", params).await?;
            listed.extend(page["sessions"].as_array().into_iter().flatten().cloned());
            cursor = page["nextCursor"].as_str().map(str::to_owned);
            if cursor.is_none() {
                break;
            }
        }
        let state = self.inner.state.lock().expect("state lock");
        let known = |acp_id: &str| {
            state.by_acp_id.contains_key(acp_id) || state.recent.iter().any(|s| s.acp_id == acp_id)
        };
        Ok(listed
            .iter()
            .filter_map(|s| {
                let acp_id = s["sessionId"].as_str().filter(|id| !id.is_empty())?;
                // Claude Code lists the repository's other Worktrees' conversations too.
                let cwd = worktrees::normalize(PathBuf::from(s["cwd"].as_str()?));
                (cwd == wanted && !known(acp_id)).then(|| OtherConversation {
                    acp_id: acp_id.to_owned(),
                    title: s["title"]
                        .as_str()
                        .map(str::trim)
                        .filter(|t| !t.is_empty())
                        .map(str::to_owned),
                    worktree: wanted.clone(),
                    updated_at: s["updatedAt"].as_str().map(str::to_owned),
                })
            })
            .collect())
    }

    /// Opens one of `other_conversations` in a new Tab, with its conversation, named after its
    /// title. One that's a Tab already is just that Tab; a Recent session is reopened as such.
    pub async fn open_conversation(
        &self,
        worktree: &Path,
        acp_id: &str,
        title: Option<&str>,
    ) -> Result<SessionId, CoreError> {
        let wanted = worktrees::normalize(worktree.to_owned());
        let saved = {
            let mut state = self.inner.state.lock().expect("state lock");
            if let Some(&id) = state.by_acp_id.get(acp_id) {
                return Ok(id);
            }
            if !state.worktrees.iter().any(|w| w.path == wanted) {
                return Err(CoreError::UnknownWorktree(worktree.to_owned()));
            }
            if state.removing.contains(&wanted) {
                return Err(CoreError::BeingRemoved);
            }
            match state.recent.iter().position(|s| s.acp_id == acp_id) {
                Some(at) => state.recent.remove(at),
                None => SavedSession {
                    acp_id: acp_id.to_owned(),
                    name: title
                        .and_then(name_from_title)
                        .unwrap_or_else(|| state.new_name()),
                    worktree: wanted,
                    permission_mode: PermissionMode::AskForEdits,
                    files: vec![],
                    started: true,
                },
            }
        };
        self.reopen(saved).await
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
        self.send_prompt_with(id, text, vec![]).await
    }

    /// `send_prompt` with files or images attached (made by `attach_file` / `attach_data`).
    pub async fn send_prompt_with(
        &self,
        id: SessionId,
        text: &str,
        attachments: Vec<Attachment>,
    ) -> Result<(), CoreError> {
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
        let start_turn = |control: &mut Control| session.start_turn(control, text, &attachments);
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
        self.inner.dispatch(session, connection, text, &attachments);
        Ok(())
    }

    /// Queues a message written while the session is Working (or Needs you): it's sent as the next
    /// prompt when the turn ends Idle. A message already queued gets this one added to it. If the
    /// turn has just ended, it's sent now instead.
    pub async fn queue_prompt(
        &self,
        id: SessionId,
        text: &str,
        attachments: Vec<Attachment>,
    ) -> Result<(), CoreError> {
        let session = self.session(id)?;
        let queued = session.update(|control| {
            if !control.in_turn {
                return false;
            }
            let queued = control.queued.get_or_insert_with(|| QueuedPrompt {
                text: String::new(),
                attachments: vec![],
            });
            if !queued.text.is_empty() && !text.is_empty() {
                queued.text.push_str("\n\n");
            }
            queued.text.push_str(text);
            queued.attachments.extend(attachments.iter().cloned());
            session.queued_changed(Some(queued.clone()));
            true
        });
        if queued {
            Ok(())
        } else {
            self.send_prompt_with(id, text, attachments).await
        }
    }

    /// The session's queued message, if it has one.
    pub fn queued_prompt(&self, id: SessionId) -> Result<Option<QueuedPrompt>, CoreError> {
        Ok(self
            .session(id)?
            .control
            .lock()
            .expect("control lock")
            .queued
            .clone())
    }

    /// Takes the session's queued message back (to edit it in the composer, or to drop it): it
    /// won't be sent.
    pub fn take_queued_prompt(&self, id: SessionId) -> Result<Option<QueuedPrompt>, CoreError> {
        let session = self.session(id)?;
        let taken = session.update(|control| control.queued.take());
        if taken.is_some() {
            session.queued_changed(None);
        }
        Ok(taken)
    }

    /// Stops the turn in progress (`session/cancel`): open permission cards are cancelled, the
    /// session goes Idle once the Agent has stopped, and a queued message isn't sent. Nothing to
    /// do if no turn is running.
    pub fn cancel_turn(&self, id: SessionId) -> Result<(), CoreError> {
        let session = self.session(id)?;
        let stop = session.update(|control| {
            if !control.in_turn || control.exited {
                return false;
            }
            control.stopping = true;
            session.cancel_questions(control);
            control.prompt_sent // (else `dispatch` sends it)
        });
        if let Some(connection) = session.connection().filter(|_| stop) {
            session.send_cancel(&connection)?;
        }
        Ok(())
    }

    /// A file to attach to the session's next prompt, or why it can't be (too big, a kind the
    /// Agent doesn't take in a prompt, not an image or text).
    pub async fn attach_file(&self, id: SessionId, path: &Path) -> Result<Attachment, CoreError> {
        self.session(id)?;
        let name = path
            .file_name()
            .map_or_else(|| path.to_string_lossy(), |n| n.to_string_lossy())
            .into_owned();
        let file = path.to_path_buf();
        let read = tokio::task::spawn_blocking(move || {
            // (Not a huge file read whole only to be refused.)
            if std::fs::metadata(&file)?.len() > attachments::MAX_BYTES as u64 {
                return Ok(None);
            }
            std::fs::read(&file).map(Some)
        });
        let bytes = read
            .await
            .expect("read task")
            .map_err(|err| CoreError::Attachment(format!("{name}: couldn't read it ({err})")))?
            .ok_or_else(|| CoreError::Attachment(attachments::too_big(&name)))?;
        self.attachment(id, &name, Some(path), None, bytes).await
    }

    /// `attach_file` for something pasted (an image from the clipboard, say): its name, its type
    /// if known, and its contents base64-encoded.
    pub async fn attach_data(
        &self,
        id: SessionId,
        name: &str,
        mime_type: Option<&str>,
        data: &str,
    ) -> Result<Attachment, CoreError> {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(|_| CoreError::Attachment(format!("{name}: couldn't read it")))?;
        self.attachment(id, name, None, mime_type, bytes).await
    }

    /// Makes an attachment, checked against what the session's Agent takes in a prompt (asking
    /// the adapter, started if need be: attaching means a prompt is about to go).
    async fn attachment(
        &self,
        id: SessionId,
        name: &str,
        path: Option<&Path>,
        mime_type: Option<&str>,
        bytes: Vec<u8>,
    ) -> Result<Attachment, CoreError> {
        let connection = match self.session(id)?.connection() {
            Some(connection) => connection,
            None => self.adapter().await?,
        };
        Attachment::from_bytes(
            name,
            path,
            mime_type,
            bytes,
            &connection.prompt_capabilities(),
        )
        .map_err(CoreError::Attachment)
    }

    /// The oldest permission card waiting for an answer in the session, if it's Needs you (for the
    /// Board's cards, which answer it from there).
    pub fn pending_permission(
        &self,
        id: SessionId,
    ) -> Result<Option<crate::session::PermissionRequest>, CoreError> {
        let session = self.session(id)?;
        // (Lock order: control, then transcript.)
        let control = session.control.lock().expect("control lock");
        let Some(question) = control.questions.first() else {
            return Ok(None);
        };
        let transcript = session.transcript.lock().expect("transcript lock");
        Ok(match transcript.items().get(question.index) {
            Some(TranscriptItem::Permission { request, .. }) => Some(request.clone()),
            _ => None,
        })
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

    /// The slash commands (and skills) the session's Agent last said it offers: empty until its
    /// process has started (a restored, Suspended session has none yet).
    pub fn available_commands(&self, id: SessionId) -> Result<Vec<SlashCommand>, CoreError> {
        Ok(self
            .session(id)?
            .commands
            .lock()
            .expect("commands lock")
            .clone())
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

    /// Makes no Tab visible in any view slot: ends every stream, and new output counts as unread.
    pub fn hide_tabs(&self) {
        let previous: Vec<_> = self
            .inner
            .visible_tabs
            .lock()
            .expect("visible tab lock")
            .drain()
            .collect();
        for (_, (previous, _stop_previous)) in previous {
            previous.set_visible(false);
        } // dropping `_stop_previous` ends that stream
    }

    /// Makes no Tab visible in `slot` (e.g. a Worktree with no sessions is selected there): ends
    /// that stream, and the session's new output counts as unread again unless another slot shows it.
    pub fn hide_tab_in(&self, slot: &str) {
        let mut visible = self.inner.visible_tabs.lock().expect("visible tab lock");
        if let Some((previous, _stop_previous)) = visible.remove(slot) {
            if !visible.values().any(|(s, _)| Arc::ptr_eq(s, &previous)) {
                previous.set_visible(false);
            }
        } // dropping `_stop_previous` ends that stream
    }

    /// Makes `id` the visible Tab of the Tabs view (`TABS_SLOT`). See `show_session_in`.
    pub fn show_session(&self, id: SessionId) -> Result<TranscriptStream, CoreError> {
        self.show_session_in(TABS_SLOT, id)
    }

    /// Makes `id` the visible Tab in view `slot` (the Tabs view, or one column of the Columns view):
    /// ends the stream that slot had, marks this session read, and streams its transcript: a
    /// `Reset` with the latest page, then batched changes. Slots stream independently, so several
    /// Tabs can be visible at once; a session is visible while any slot shows it.
    pub fn show_session_in(
        &self,
        slot: &str,
        id: SessionId,
    ) -> Result<TranscriptStream, CoreError> {
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
        {
            let mut visible = self.inner.visible_tabs.lock().expect("visible tab lock");
            let previous = visible.insert(slot.to_owned(), (session.clone(), stop));
            if let Some((previous, _stop_previous)) = previous {
                if !visible.values().any(|(s, _)| Arc::ptr_eq(s, &previous)) {
                    previous.set_visible(false);
                }
            } // dropping `_stop_previous` ends that stream
        }

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
    /// Sends the prompt of a turn just started (`Session::start_turn`), with the Edit notes waiting
    /// for it, and finishes the turn when the Agent replies: then a queued message, if any, starts
    /// the next turn.
    fn dispatch(
        self: &Arc<Self>,
        session: Arc<Session>,
        connection: Arc<Connection>,
        text: &str,
        attachments: &[Attachment],
    ) {
        let id = session.id();
        // Hand edits since the Agent last looked go first (and are delivered, shown under the
        // message they went with).
        let delivery = session.edit_notes.lock().expect("edit notes lock").take();
        let mut prompt = vec![];
        if let Some(delivery) = &delivery {
            prompt.push(json!({ "type": "text", "text": delivery.text }));
            session.tag_last_message(delivery.tags.clone());
            let _ = self.events.send(CoreEvent::EditNotesChanged {
                session_id: id,
                notes: vec![],
            });
        }
        prompt.push(json!({ "type": "text", "text": text }));
        prompt.extend(attachments.iter().map(Attachment::content_block));
        let params = json!({ "sessionId": session.acp_id, "prompt": prompt });
        if !session.started.swap(true, Ordering::SeqCst) {
            self.persist(); // (it has a conversation to bring back now)
        }
        let reply = connection.request("session/prompt", params);
        let stopped = session.update(|control| {
            control.prompt_sent = true;
            control.stopping
        });
        if stopped {
            let _ = session.send_cancel(&connection);
        }
        let inner = self.clone();
        tokio::spawn(async move {
            let result = reply.await;
            if let (Err(_), Some(delivery)) = (&result, delivery) {
                // It never got them (the adapter crashed, say): they wait for the next prompt.
                let mut notes = session.edit_notes.lock().expect("edit notes lock");
                notes.put_back(delivery);
                let _ = inner.events.send(CoreEvent::EditNotesChanged {
                    session_id: id,
                    notes: notes.notes(),
                });
            }
            // The Agent may have committed or changed files: refresh ahead/changed counts.
            let refresh = inner.clone();
            tokio::spawn(async move { refresh.refresh_worktrees().await });
            let next = session.update(|control| {
                // The adapter crashed and the session has been resumed elsewhere since (or closed):
                // this turn's ending is old news.
                let current = session.connection();
                if control.closed || !current.is_some_and(|c| Arc::ptr_eq(&c, &connection)) {
                    return None;
                }
                // A question still open when the turn ends will never be answered.
                session.cancel_questions(control);
                control.in_turn = false;
                let stopped = std::mem::take(&mut control.stopping);
                let ok = match result {
                    Ok(_) => true,
                    Err(err) if session_gone(&err) => {
                        control.exited = true;
                        false
                    }
                    Err(err) => {
                        session.record(|t| {
                            t.push(TranscriptItem::Notice {
                                text: format!("The turn failed: {err}"),
                            })
                        });
                        false
                    }
                };
                if stopped {
                    session.record(|t| {
                        t.push(TranscriptItem::Notice {
                            text: "You stopped the turn.".to_owned(),
                        })
                    });
                }
                // Ended Idle: the queued message goes now, one turn straight after the other (it
                // never shows Idle in between, so nothing else can claim the session first).
                if !ok || stopped || control.state() != SessionState::Idle {
                    return None;
                }
                let next = control.queued.take()?;
                session.start_turn(control, &next.text, &next.attachments);
                session.queued_changed(None);
                Some(next)
            });
            if let Some(next) = next {
                inner.dispatch(session, connection, &next.text, &next.attachments);
            }
        });
    }

    /// Every running shell, oldest first.
    fn terminal_list(&self) -> Vec<TerminalInfo> {
        let mut list: Vec<TerminalInfo> = self
            .terminals
            .lock()
            .expect("terminals lock")
            .values()
            .map(|t| t.info())
            .collect();
        list.sort_by_key(|t| t.id);
        list
    }

    fn terminals_changed(&self) {
        let _ = self.events.send(CoreEvent::TerminalsChanged {
            terminals: self.terminal_list(),
        });
    }

    /// Drops a closed session from the editor and ends its Tab's stream. The saved Tabs no longer
    /// list it by the time `SessionClosed` goes out.
    fn forget_session(self: &Arc<Self>, session: &Arc<Session>) {
        let id = session.info.lock().expect("info lock").id;
        {
            let mut state = self.state.lock().expect("state lock");
            state.sessions.remove(&id);
            state.by_acp_id.remove(&session.acp_id);
        }
        self.persist();
        self.visible_tabs
            .lock()
            .expect("visible tab lock")
            .retain(|_, (shown, _)| !Arc::ptr_eq(shown, session)); // ends its streams
        let _ = self
            .events
            .send(CoreEvent::SessionClosed { session_id: id });
        self.actors_may_change();
    }

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
                ActorNews::Status => {
                    // (The watcher has let the burst settle: the Git drawer looks again now.)
                    let _ = inner.events.send(CoreEvent::GitStatusChanged {
                        worktree: worktree.clone(),
                    });
                    inner.status_due(worktree);
                }
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

    /// The user saved `path` by hand (it was `before`, it's now `after`): an Edit note for each
    /// session on its Worktree that read or edited it. None is woken; each gets it with its next
    /// prompt (a Suspended one after it resumes).
    fn note_edit(&self, path: &Path, before: &str, after: &str) {
        let (sessions, owner): (Vec<Arc<Session>>, Option<PathBuf>) = {
            let state = self.state.lock().expect("state lock");
            // (A linked Worktree may sit inside the main one: the deepest that holds it.)
            let owner = state
                .worktrees
                .iter()
                .map(|w| &w.path)
                .filter(|w| path.starts_with(w))
                .max_by_key(|w| w.components().count())
                .cloned();
            (state.sessions.values().cloned().collect(), owner)
        };
        let Some(owner) = owner else {
            return; // (the settings file, say)
        };
        for session in sessions {
            let worktree = session.info.lock().expect("info lock").worktree.clone();
            if worktree != owner {
                continue;
            }
            let Ok(relative) = path.strip_prefix(&worktree) else {
                continue;
            };
            if session.control.lock().expect("control lock").closed
                || !session.files.lock().expect("files lock").contains(path)
            {
                continue;
            }
            let name = relative.to_string_lossy().replace('\\', "/");
            let mut notes = session.edit_notes.lock().expect("edit notes lock");
            if notes.saved(path.to_owned(), name, before, after) {
                let _ = self.events.send(CoreEvent::EditNotesChanged {
                    session_id: session.id(),
                    notes: notes.notes(),
                });
            }
        }
    }

    /// The Worktrees being followed: the one looked at, and those with sessions (their actors run).
    fn live_worktrees(&self) -> Vec<PathBuf> {
        let mut live: Vec<PathBuf> = self
            .actors
            .lock()
            .expect("actors lock")
            .keys()
            .cloned()
            .collect();
        live.sort();
        live
    }

    /// Fetches the repo's remote from `worktree` (under `remote_op`) and notes when (a failed fetch
    /// counts too: focus doesn't retry it at once). Every followed Worktree shares the remote refs,
    /// so they all look again.
    async fn fetch(self: &Arc<Self>, worktree: &Path) -> Result<(), String> {
        *self.last_fetch.lock().expect("last fetch lock") = Some(self.clock.now());
        let fetched = git::fetch(worktree).await;
        let mut changed = self.live_worktrees();
        if !changed.iter().any(|w| w == worktree) {
            changed.push(worktree.to_owned());
        }
        for worktree in changed {
            self.git_changed(worktree);
        }
        fetched
    }

    /// Something the editor did changed `worktree`'s refs (a fetch, push or pull): the drawer and
    /// the Worktree list look again.
    fn git_changed(self: &Arc<Self>, worktree: PathBuf) {
        let _ = self.events.send(CoreEvent::GitStatusChanged {
            worktree: worktree.clone(),
        });
        self.status_due(worktree);
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
            files: Mutex::new(saved.files.iter().cloned().collect()),
            edit_notes: Mutex::default(),
            commands: Mutex::new(
                self.early_commands
                    .lock()
                    .expect("early commands lock")
                    .remove(&saved.acp_id)
                    .unwrap_or_default(),
            ),
            started: AtomicBool::new(saved.started),
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
            // (A Tab never sent a prompt has no conversation to bring back.)
            let mut open: Vec<_> = state
                .sessions
                .values()
                .filter(|s| s.started.load(Ordering::SeqCst))
                .collect();
            open.sort_by_key(|s| s.info.lock().expect("info lock").id);
            let saved = WorkspaceState {
                tabs: open.into_iter().map(|s| s.saved()).collect(),
                recent: state.recent.clone(),
                active: state.active.clone(),
                bases: state.bases.clone(),
                pinned: state.pinned.clone(),
                column_shares: state.column_shares.clone(),
                opened: state.opened,
                // (Opening a Workspace lists it again.)
                hidden: false,
            };
            (workspace.root.clone(), saved)
        };
        if let Err(err) = app_state::save_workspace(path, &root, &saved) {
            eprintln!("couldn't save the open Tabs: {err}");
        }
    }

    /// The app state as saved now (the file can change after startup: this editor saves into it,
    /// and so may another one); what was loaded at startup if it can't be read.
    fn saved_state(&self) -> AppState {
        match self
            .config
            .state_path
            .as_ref()
            .filter(|_| self.state_writable)
        {
            Some(path) => {
                let _in_order = self.persist_lock.lock().expect("persist lock");
                app_state::load(path)
                    .unwrap_or_else(|_| self.app_state.lock().expect("app state lock").clone())
            }
            None => self.app_state.lock().expect("app state lock").clone(),
        }
    }

    /// Brings back the Tabs open when the editor last closed this Workspace, Suspended (no Agent
    /// starts until one is used), in their Worktrees and order, and its Recent sessions. Ones whose
    /// Worktree is gone are dropped (but not because listing the Worktrees failed).
    fn restore(&self, root: &Path) {
        let saved = self
            .saved_state()
            .workspaces
            .remove(root)
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
            state.pinned = saved
                .pinned
                .into_iter()
                .filter(|worktree| {
                    listing_failed || state.worktrees.iter().any(|w| &w.path == worktree)
                })
                .collect();
            state.column_shares = saved
                .column_shares
                .into_iter()
                .filter(|(worktree, _)| state.pinned.contains(worktree))
                .collect();
            state.bases = saved
                .bases
                .into_iter()
                .filter(|(worktree, _)| {
                    listing_failed || state.worktrees.iter().any(|w| &w.path == worktree)
                })
                .collect();
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
                    "clientInfo": { "name": "orchard", "version": env!("CARGO_PKG_VERSION") },
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
            if let Err(CoreError::Acp(AcpError::Rpc {
                code: RESOURCE_NOT_FOUND,
                ..
            })) = &reopened
            {
                // Claude Code has no such conversation (it was never sent a message, or its
                // transcript is gone): retrying can't help, and there's nothing to show. The Tab
                // goes; if its Worktree has no other, the view offers a new session there.
                session.update(|control| {
                    control.loading = false;
                    control.closed = true;
                });
                inner.forget_session(&session);
                return;
            }
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
            // A removed Worktree's shells have nowhere to be (a failed listing keeps them).
            if !listed.is_empty() {
                let mut terminals = self.terminals.lock().expect("terminals lock");
                let before = terminals.len();
                terminals.retain(|_, t| listed.iter().any(|w| w.path == t.worktree));
                let changed = terminals.len() != before;
                drop(terminals);
                if changed {
                    self.terminals_changed();
                }
            }
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
                let update = &params["update"];
                let Some(session) = self.session_for(&params) else {
                    if update["sessionUpdate"] == "available_commands_update" {
                        if let Some(acp_id) = params["sessionId"].as_str() {
                            self.early_commands
                                .lock()
                                .expect("early commands lock")
                                .insert(acp_id.to_owned(), SlashCommand::list_from(update));
                        }
                    }
                    return;
                };
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
                    Some("tool_call" | "tool_call_update") => {
                        // (Its files, for Edit notes after a restart; a replay's go with the next save.)
                        if session.note_tool_call(update)
                            && !session.replaying.load(Ordering::SeqCst)
                        {
                            self.persist();
                        }
                    }
                    Some("available_commands_update") => {
                        let commands = SlashCommand::list_from(update);
                        *session.commands.lock().expect("commands lock") = commands.clone();
                        let session_id = session.info.lock().expect("info lock").id;
                        let _ = self.events.send(CoreEvent::AvailableCommandsChanged {
                            session_id,
                            commands,
                        });
                    }
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
            started: self.started.load(Ordering::SeqCst),
            files: {
                let mut files: Vec<PathBuf> = self
                    .files
                    .lock()
                    .expect("files lock")
                    .iter()
                    .cloned()
                    .collect();
                files.sort();
                files
            },
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
            let mid_turn = |s| matches!(s, SessionState::Working | SessionState::NeedsYou);
            if mid_turn(info.state) != mid_turn(state) {
                // (The Git drawer's "a session is mid-turn here", for switching branches.)
                let _ = self.events.send(CoreEvent::GitStatusChanged {
                    worktree: info.worktree.clone(),
                });
            }
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
    /// Whether it named a file the session hadn't read or edited before.
    fn note_tool_call(&self, update: &Value) -> bool {
        let Some(id) = update["toolCallId"].as_str() else {
            return false;
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
        let new_files = self.remember_files(&known.call, &worktree);
        let row = permissions::tool_call_row(&known.call, &worktree);
        match known.row {
            Some(index) => self.record(|t| t.replace(index, row)),
            None => self.record(|t| {
                known.row = Some(t.items().len());
                t.push(row)
            }),
        }
        new_files
    }

    /// Remembers the files a tool call reads or edits (for Edit notes); whether any was new. The
    /// session sees such a file as it is, so a note on it waiting for the next prompt is dropped
    /// (not for a replayed tool call: that's old news).
    fn remember_files(&self, call: &Value, worktree: &Path) -> bool {
        let files = permissions::files_of(call, worktree);
        if files.is_empty() {
            return false;
        }
        if !self.replaying.load(Ordering::SeqCst) {
            let mut notes = self.edit_notes.lock().expect("edit notes lock");
            let mut dropped = false;
            for file in &files {
                dropped |= notes.remove(file);
            }
            if dropped {
                let _ = self.events.send(CoreEvent::EditNotesChanged {
                    session_id: self.id(),
                    notes: notes.notes(),
                });
            }
        }
        let mut known = self.files.lock().expect("files lock");
        let mut new = false;
        for file in files {
            new |= known.insert(file);
        }
        new
    }

    /// Shows `tags` (the Edit notes sent with it) under the user's latest message.
    fn tag_last_message(&self, tags: Vec<String>) {
        let transcript = self.transcript.lock().expect("transcript lock");
        let Some(index) = transcript
            .items()
            .iter()
            .rposition(|item| matches!(item, TranscriptItem::User { .. }))
        else {
            return;
        };
        let TranscriptItem::User {
            text, attachments, ..
        } = transcript.items()[index].clone()
        else {
            return;
        };
        drop(transcript);
        self.record(|t| {
            t.replace(
                index,
                TranscriptItem::User {
                    text,
                    edit_notes: tags,
                    attachments,
                },
            )
        });
    }

    fn id(&self) -> SessionId {
        self.info.lock().expect("info lock").id
    }

    /// Tells the Agent to stop the turn in progress.
    fn send_cancel(&self, connection: &Connection) -> Result<(), AcpError> {
        connection.notify("session/cancel", json!({ "sessionId": self.acp_id }))
    }

    /// Tells the composer its queued message changed.
    fn queued_changed(&self, queued: Option<QueuedPrompt>) {
        let _ = self.events.send(CoreEvent::QueuedPromptChanged {
            session_id: self.id(),
            queued,
        });
    }

    /// Starts a turn: the user's message goes in the transcript and the session is Working.
    fn start_turn(&self, control: &mut Control, text: &str, attachments: &[Attachment]) {
        control.auto_suspend_failed = false;
        control.stopping = false;
        control.prompt_sent = false;
        self.record(|t| {
            t.push(TranscriptItem::User {
                text: text.to_owned(),
                edit_notes: vec![],
                attachments: attachments.iter().map(|a| a.name().to_owned()).collect(),
            })
        });
        control.in_turn = true;
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

/// Now, in milliseconds since the Unix epoch.
fn unix_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}
