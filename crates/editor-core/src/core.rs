//! The core's public API: the same surface the frontend drives over Tauri IPC (ADR 0003).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};
use tokio::sync::{broadcast, mpsc, oneshot};

use crate::acp::{AcpError, AdapterCommand, Connection, Incoming, Responder, PROTOCOL_VERSION};
use crate::create_worktree::{self, BranchInfo, BranchList, CreatedWorktree, NewWorktree};
use crate::git;
use crate::permissions;
use crate::session::{
    PermissionMode, PermissionOutcome, SessionId, SessionInfo, SessionState, Transcript,
    TranscriptDelta, TranscriptItem, TranscriptPage,
};
use crate::worktrees::{self, Discovery, WorktreeInfo};

/// How long streamed transcript changes are gathered before being flushed to the visible Tab.
const FLUSH_INTERVAL: Duration = Duration::from_millis(16);

pub struct CoreConfig {
    pub adapter: AdapterCommand,
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
    #[error("the Agent session has exited")]
    SessionExited,
    #[error("the Agent session isn't waiting for a permission answer")]
    NoPendingPermission,
    #[error("`{0}` isn't one of the options the Agent offered")]
    UnknownPermissionOption(String),
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
}

#[derive(Default)]
struct State {
    workspace: Option<WorkspaceInfo>,
    worktrees: Vec<WorktreeInfo>,
    sessions: HashMap<SessionId, Arc<Session>>,
    by_acp_id: HashMap<String, SessionId>,
    next_session: u64,
}

struct Session {
    /// What the frontend sees. `state` here is only ever written by `update`, derived from `control`.
    info: Mutex<SessionInfo>,
    acp_id: String,
    connection: Arc<Connection>,
    transcript: Mutex<Transcript>,
    deltas: broadcast::Sender<TranscriptDelta>,
    events: broadcast::Sender<CoreEvent>,
    /// Whether this session's Tab is the visible one (and so isn't collecting unread items).
    visible: AtomicBool,
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
    /// Permission questions the Agent is waiting on, oldest first.
    questions: Vec<OpenQuestion>,
}

impl Control {
    fn state(&self) -> SessionState {
        if self.exited {
            SessionState::Exited
        } else if !self.questions.is_empty() {
            SessionState::NeedsYou
        } else if self.in_turn {
            SessionState::Working
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
        Self {
            inner: Arc::new(Inner {
                config,
                events,
                state: Mutex::default(),
                adapter: tokio::sync::Mutex::new(None),
                visible_tab: Mutex::new(None),
                discovery: Mutex::new(None),
                refresh_lock: tokio::sync::Mutex::new(()),
            }),
        }
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
        }
        self.inner.watch_worktrees(&workspace.root).await;
        Ok(workspace)
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
        Ok(CreatedWorktree { worktree, warning })
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

    /// Starts a new Agent session in one of the Workspace's Worktrees.
    pub async fn new_session_in(&self, worktree: &Path) -> Result<SessionId, CoreError> {
        self.workspace()?;
        let wanted = worktrees::normalize(worktree.to_owned());
        let root = self
            .worktrees()
            .into_iter()
            .map(|w| w.path)
            .find(|path| *path == wanted)
            .ok_or_else(|| CoreError::UnknownWorktree(worktree.to_owned()))?;
        let connection = self.adapter().await?;
        let created = connection
            .request("session/new", json!({ "cwd": root, "mcpServers": [] }))
            .await?;
        let acp_id = created["sessionId"].as_str().unwrap_or_default().to_owned();
        // Every Tab starts in Ask for edits, whatever the Agent's own settings default to.
        let current_mode = created["modes"]["currentModeId"].as_str();
        if current_mode.is_some_and(|mode| mode != PermissionMode::AskForEdits.acp_id()) {
            connection
                .request(
                    "session/set_mode",
                    json!({ "sessionId": acp_id, "modeId": PermissionMode::AskForEdits.acp_id() }),
                )
                .await?;
        }

        let mut state = self.inner.state.lock().expect("state lock");
        state.next_session += 1;
        let id = SessionId(state.next_session);
        let info = SessionInfo {
            id,
            name: format!("Session {}", id.0),
            worktree: root,
            state: SessionState::Idle,
            permission_mode: PermissionMode::AskForEdits,
            unread: 0,
        };
        let (deltas, _) = broadcast::channel(4096);
        let session = Arc::new(Session {
            info: Mutex::new(info.clone()),
            acp_id: acp_id.clone(),
            connection,
            transcript: Mutex::default(),
            deltas,
            events: self.inner.events.clone(),
            visible: AtomicBool::new(false),
            tool_calls: Mutex::default(),
            control: Mutex::default(),
        });
        state.sessions.insert(id, session);
        state.by_acp_id.insert(acp_id, id);
        drop(state);
        let _ = self
            .inner
            .events
            .send(CoreEvent::SessionCreated { session: info });
        Ok(id)
    }

    /// Sends a prompt; returns once it's on its way. The reply streams to watchers.
    pub async fn send_prompt(&self, id: SessionId, text: &str) -> Result<(), CoreError> {
        let session = self.session(id)?;
        session.update(|control| match control.state() {
            SessionState::Working | SessionState::NeedsYou => Err(CoreError::SessionBusy),
            SessionState::Exited => Err(CoreError::SessionExited),
            SessionState::Idle => {
                session.record(|t| {
                    t.push(TranscriptItem::User {
                        text: text.to_owned(),
                    })
                });
                control.in_turn = true;
                Ok(())
            }
        })?;

        let params =
            json!({ "sessionId": session.acp_id, "prompt": [{ "type": "text", "text": text }] });
        let inner = self.inner.clone();
        tokio::spawn(async move {
            let result = session.connection.request("session/prompt", params).await;
            // The Agent may have committed or changed files: refresh ahead/changed counts.
            let refresh = inner.clone();
            tokio::spawn(async move { refresh.refresh_worktrees().await });
            session.update(|control| {
                // A question still open when the turn ends will never be answered.
                session.cancel_questions(control);
                control.in_turn = false;
                match result {
                    Ok(_) => {}
                    Err(AcpError::Closed) => control.exited = true,
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

    /// Switches how much the session's Agent may do without asking.
    pub async fn set_permission_mode(
        &self,
        id: SessionId,
        mode: PermissionMode,
    ) -> Result<(), CoreError> {
        let session = self.session(id)?;
        session
            .connection
            .request(
                "session/set_mode",
                json!({ "sessionId": session.acp_id, "modeId": mode.acp_id() }),
            )
            .await?;
        session.set_mode(mode);
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

    /// The shared adapter connection, started (and initialised) on first use or after it exited.
    async fn adapter(&self) -> Result<Arc<Connection>, CoreError> {
        let mut slot = self.inner.adapter.lock().await;
        if let Some(connection) = slot.as_ref().filter(|c| !c.is_closed()) {
            return Ok(connection.clone());
        }
        let weak = Arc::downgrade(&self.inner);
        let connection = Arc::new(Connection::spawn(
            &self.inner.config.adapter,
            move |incoming| {
                if let Some(inner) = Weak::upgrade(&weak) {
                    inner.handle(incoming);
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
        *slot = Some(connection.clone());
        Ok(connection)
    }
}

impl Inner {
    /// Re-lists the Worktrees. Refreshes run one at a time (so an older listing can't overwrite a
    /// newer one), and a listing is dropped if another Workspace was opened meanwhile.
    async fn refresh_worktrees(&self) {
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
        let listed = worktrees::list(&root).await;
        let mut state = self.state.lock().expect("state lock");
        if state.workspace.as_ref().map(|w| &w.root) != Some(&root) {
            return;
        }
        if state.worktrees != listed {
            state.worktrees = listed.clone();
            let _ = self
                .events
                .send(CoreEvent::WorktreesChanged { worktrees: listed });
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

    /// Handles what the adapter sends us, in arrival order (see `acp`).
    fn handle(&self, incoming: Incoming) {
        match incoming {
            Incoming::Notification { method, params } if method == "session/update" => {
                let Some(session) = self.session_for(&params) else {
                    return;
                };
                let update = &params["update"];
                match update["sessionUpdate"].as_str() {
                    Some("agent_message_chunk") if update["content"]["type"] == "text" => {
                        let text = update["content"]["text"].as_str().unwrap_or_default();
                        let message_id = update["messageId"].as_str().map(str::to_owned);
                        session.record(|t| t.append_agent_text(text, message_id));
                    }
                    Some("tool_call" | "tool_call_update") => session.note_tool_call(update),
                    // The Agent can change mode itself, e.g. when leaving plan mode.
                    Some("current_mode_update") => {
                        if let Some(mode) = update["currentModeId"]
                            .as_str()
                            .and_then(PermissionMode::from_acp_id)
                        {
                            session.set_mode(mode);
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
            Incoming::Closed => {
                let sessions: Vec<_> = self
                    .state
                    .lock()
                    .expect("state lock")
                    .sessions
                    .values()
                    .cloned()
                    .collect();
                for session in sessions {
                    session.update(|control| {
                        session.cancel_questions(control);
                        control.in_turn = false;
                        control.exited = true;
                    });
                }
            }
        }
    }

    fn session_for(&self, params: &Value) -> Option<Arc<Session>> {
        let state = self.state.lock().expect("state lock");
        let id = state.by_acp_id.get(params["sessionId"].as_str()?)?;
        state.sessions.get(id).cloned()
    }
}

impl Session {
    /// Changes the facts behind the session's state under one lock, then publishes the state they
    /// now imply. Lock order: control, then transcript, then info.
    fn update<R>(&self, change: impl FnOnce(&mut Control) -> R) -> R {
        let mut control = self.control.lock().expect("control lock");
        let result = change(&mut control);
        let state = control.state();
        let mut info = self.info.lock().expect("info lock");
        if info.state != state {
            info.state = state;
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
            && !self.visible.load(Ordering::SeqCst);
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
