//! The core's public API: the same surface the frontend drives over Tauri IPC (ADR 0003).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};
use tokio::sync::{broadcast, mpsc};

use crate::acp::{AcpError, AdapterCommand, Connection, Incoming, Responder, PROTOCOL_VERSION};
use crate::git;
use crate::permissions;
use crate::session::{
    PermissionMode, PermissionOutcome, SessionId, SessionInfo, SessionState, Transcript, TranscriptDelta,
    TranscriptItem,
};

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
    SessionCreated { session: SessionInfo },
    #[serde(rename_all = "camelCase")]
    SessionStateChanged { session_id: SessionId, state: SessionState },
    #[serde(rename_all = "camelCase")]
    PermissionModeChanged { session_id: SessionId, mode: PermissionMode },
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
}

#[derive(Default)]
struct State {
    workspace: Option<WorkspaceInfo>,
    sessions: HashMap<SessionId, Arc<Session>>,
    by_acp_id: HashMap<String, SessionId>,
    next_session: u64,
}

struct Session {
    info: Mutex<SessionInfo>,
    acp_id: String,
    connection: Arc<Connection>,
    transcript: Mutex<Transcript>,
    deltas: broadcast::Sender<TranscriptDelta>,
    /// Every tool call the Agent announced, merged with its updates, keyed by ACP tool call id.
    tool_calls: Mutex<HashMap<String, Value>>,
    /// The permission question the Agent is waiting on, if any.
    pending_permission: Mutex<Option<PendingPermission>>,
}

struct PendingPermission {
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
            }),
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<CoreEvent> {
        self.inner.events.subscribe()
    }

    /// Opens the git repository containing `path` as the Workspace.
    pub async fn open_workspace(&self, path: &Path) -> Result<WorkspaceInfo, CoreError> {
        let root = git::toplevel(path).await.ok_or_else(|| CoreError::NotARepository(path.to_owned()))?;
        let root = std::fs::canonicalize(&root).map(strip_verbatim).unwrap_or(root);
        let name = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let workspace = WorkspaceInfo { root, name };
        self.inner.state.lock().expect("state lock").workspace = Some(workspace.clone());
        Ok(workspace)
    }

    /// Starts a new Agent session in the Workspace's main checkout.
    pub async fn new_session(&self) -> Result<SessionId, CoreError> {
        let root = self.workspace()?.root;
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
        };
        let (deltas, _) = broadcast::channel(4096);
        let session = Arc::new(Session {
            info: Mutex::new(info.clone()),
            acp_id: acp_id.clone(),
            connection,
            transcript: Mutex::default(),
            deltas,
            tool_calls: Mutex::default(),
            pending_permission: Mutex::default(),
        });
        state.sessions.insert(id, session);
        state.by_acp_id.insert(acp_id, id);
        drop(state);
        let _ = self.inner.events.send(CoreEvent::SessionCreated { session: info });
        Ok(id)
    }

    /// Sends a prompt; returns once it's on its way. The reply streams to watchers.
    pub async fn send_prompt(&self, id: SessionId, text: &str) -> Result<(), CoreError> {
        let session = self.session(id)?;
        match session.state() {
            SessionState::Working | SessionState::NeedsYou => return Err(CoreError::SessionBusy),
            SessionState::Exited => return Err(CoreError::SessionExited),
            SessionState::Idle => {}
        }
        session.record(|t| t.push(TranscriptItem::User { text: text.to_owned() }));
        self.inner.set_state(&session, SessionState::Working);

        let inner = self.inner.clone();
        let params = json!({ "sessionId": session.acp_id, "prompt": [{ "type": "text", "text": text }] });
        tokio::spawn(async move {
            let result = session.connection.request("session/prompt", params).await;
            // A question still open when the turn ends will never be answered.
            session.cancel_pending_permission();
            match result {
                Ok(_) => inner.set_state(&session, SessionState::Idle),
                Err(AcpError::Closed) => inner.set_state(&session, SessionState::Exited),
                Err(err) => {
                    session.record(|t| t.push(TranscriptItem::Notice { text: format!("The turn failed: {err}") }));
                    inner.set_state(&session, SessionState::Idle);
                }
            }
        });
        Ok(())
    }

    /// Answers the permission card the session is waiting on with one of the Agent's options.
    pub async fn answer_permission(&self, id: SessionId, option_id: &str) -> Result<(), CoreError> {
        let session = self.session(id)?;
        let pending = {
            let mut slot = session.pending_permission.lock().expect("permission lock");
            let pending = slot.as_ref().ok_or(CoreError::NoPendingPermission)?;
            if !pending.option_ids.iter().any(|o| o == option_id) {
                return Err(CoreError::UnknownPermissionOption(option_id.to_owned()));
            }
            slot.take().expect("checked above")
        };
        // Working before the answer goes out: the Agent may finish the turn straight away.
        self.inner.set_state(&session, SessionState::Working);
        session.record(|t| {
            t.resolve_permission(pending.index, PermissionOutcome::Selected { option_id: option_id.to_owned() })
        });
        pending.responder.result(json!({ "outcome": { "outcome": "selected", "optionId": option_id } }));
        Ok(())
    }

    /// Switches how much the session's Agent may do without asking.
    pub async fn set_permission_mode(&self, id: SessionId, mode: PermissionMode) -> Result<(), CoreError> {
        let session = self.session(id)?;
        session
            .connection
            .request("session/set_mode", json!({ "sessionId": session.acp_id, "modeId": mode.acp_id() }))
            .await?;
        self.inner.set_mode(&session, mode);
        Ok(())
    }

    pub fn session_info(&self, id: SessionId) -> Result<SessionInfo, CoreError> {
        Ok(self.session(id)?.info.lock().expect("info lock").clone())
    }

    pub fn transcript(&self, id: SessionId) -> Result<Vec<TranscriptItem>, CoreError> {
        Ok(self.session(id)?.transcript.lock().expect("transcript lock").items().to_vec())
    }

    /// Streams a session's transcript: a `Reset` with everything so far, then batched changes.
    pub fn watch_session(&self, id: SessionId) -> Result<TranscriptStream, CoreError> {
        let session = self.session(id)?;
        let (tx, rx) = mpsc::channel(64);
        let (initial, mut deltas) = {
            let transcript = session.transcript.lock().expect("transcript lock");
            (transcript.items().to_vec(), session.deltas.subscribe())
        };
        tokio::spawn(async move {
            let mut batch = vec![TranscriptDelta::Reset { items: initial }];
            loop {
                if tx.send(std::mem::take(&mut batch)).await.is_err() {
                    return; // the watcher went away
                }
                // Wait for the next change, then gather whatever else arrives within the flush window.
                let Some(first) = session.next_delta(&mut deltas).await else { return };
                batch.push(first);
                let deadline = tokio::time::Instant::now() + FLUSH_INTERVAL;
                while let Ok(Some(delta)) = tokio::time::timeout_at(deadline, session.next_delta(&mut deltas)).await {
                    batch.push(delta);
                }
            }
        });
        Ok(TranscriptStream { rx })
    }

    fn workspace(&self) -> Result<WorkspaceInfo, CoreError> {
        self.inner.state.lock().expect("state lock").workspace.clone().ok_or(CoreError::NoWorkspace)
    }

    fn session(&self, id: SessionId) -> Result<Arc<Session>, CoreError> {
        self.inner.state.lock().expect("state lock").sessions.get(&id).cloned().ok_or(CoreError::UnknownSession)
    }

    /// The shared adapter connection, started (and initialised) on first use or after it exited.
    async fn adapter(&self) -> Result<Arc<Connection>, CoreError> {
        let mut slot = self.inner.adapter.lock().await;
        if let Some(connection) = slot.as_ref().filter(|c| !c.is_closed()) {
            return Ok(connection.clone());
        }
        let weak = Arc::downgrade(&self.inner);
        let connection = Arc::new(Connection::spawn(&self.inner.config.adapter, move |incoming| {
            if let Some(inner) = Weak::upgrade(&weak) {
                inner.handle(incoming);
            }
        })?);
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
    fn set_state(&self, session: &Session, state: SessionState) {
        let mut info = session.info.lock().expect("info lock");
        if info.state == state {
            return;
        }
        info.state = state;
        let _ = self.events.send(CoreEvent::SessionStateChanged { session_id: info.id, state });
    }

    fn set_mode(&self, session: &Session, mode: PermissionMode) {
        let mut info = session.info.lock().expect("info lock");
        if info.permission_mode == mode {
            return;
        }
        info.permission_mode = mode;
        let _ = self.events.send(CoreEvent::PermissionModeChanged { session_id: info.id, mode });
    }

    /// Handles what the adapter sends us, in arrival order (see `acp`).
    fn handle(&self, incoming: Incoming) {
        match incoming {
            Incoming::Notification { method, params } if method == "session/update" => {
                let Some(session) = self.session_for(&params) else { return };
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
                        if let Some(mode) = update["currentModeId"].as_str().and_then(PermissionMode::from_acp_id) {
                            self.set_mode(&session, mode);
                        }
                    }
                    _ => {}
                }
            }
            Incoming::Notification { .. } => {}
            Incoming::Request { method, params, responder } if method == "session/request_permission" => {
                let Some(session) = self.session_for(&params) else {
                    return responder.result(json!({ "outcome": { "outcome": "cancelled" } }));
                };
                session.ask_permission(&params, responder);
                self.set_state(&session, SessionState::NeedsYou);
            }
            Incoming::Request { responder, .. } => responder.error(-32601, "method not supported by this client"),
            Incoming::Closed => {
                let sessions: Vec<_> = self.state.lock().expect("state lock").sessions.values().cloned().collect();
                for session in sessions {
                    session.cancel_pending_permission();
                    self.set_state(&session, SessionState::Exited);
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
    fn state(&self) -> SessionState {
        self.info.lock().expect("info lock").state
    }

    /// Applies a transcript change and streams its delta, atomically with respect to `watch_session`.
    fn record(&self, change: impl FnOnce(&mut Transcript) -> TranscriptDelta) -> TranscriptDelta {
        let mut transcript = self.transcript.lock().expect("transcript lock");
        let delta = change(&mut transcript);
        let _ = self.deltas.send(delta.clone());
        delta
    }

    fn note_tool_call(&self, update: &Value) {
        let Some(id) = update["toolCallId"].as_str() else { return };
        let mut tool_calls = self.tool_calls.lock().expect("tool calls lock");
        permissions::merge_tool_call(tool_calls.entry(id.to_owned()).or_insert_with(|| json!({})), update);
    }

    /// Puts a permission card in the transcript and holds the question until it's answered.
    fn ask_permission(&self, params: &Value, responder: Responder) {
        let mut tool_call = params["toolCall"]["toolCallId"]
            .as_str()
            .and_then(|id| self.tool_calls.lock().expect("tool calls lock").get(id).cloned())
            .unwrap_or_else(|| json!({}));
        permissions::merge_tool_call(&mut tool_call, &params["toolCall"]);
        let request = permissions::request_from(&tool_call, &params["options"]);
        let option_ids = request.options.iter().map(|o| o.id.clone()).collect();
        // Only one question at a time: a newer one supersedes an unanswered older one.
        self.cancel_pending_permission();
        let TranscriptDelta::ItemAdded { index, .. } =
            self.record(|t| t.push(TranscriptItem::Permission { request, outcome: None }))
        else {
            unreachable!("push always adds an item")
        };
        *self.pending_permission.lock().expect("permission lock") = Some(PendingPermission { index, option_ids, responder });
    }

    fn cancel_pending_permission(&self) {
        let pending = self.pending_permission.lock().expect("permission lock").take();
        if let Some(pending) = pending {
            pending.responder.result(json!({ "outcome": { "outcome": "cancelled" } }));
            self.record(|t| t.resolve_permission(pending.index, PermissionOutcome::Cancelled));
        }
    }

    /// The next change for a watcher; if the watcher fell behind, a `Reset` to the current transcript.
    /// `None` once the session is gone.
    async fn next_delta(&self, deltas: &mut broadcast::Receiver<TranscriptDelta>) -> Option<TranscriptDelta> {
        match deltas.recv().await {
            Ok(delta) => Some(delta),
            Err(broadcast::error::RecvError::Lagged(_)) => Some(TranscriptDelta::Reset {
                items: self.transcript.lock().expect("transcript lock").items().to_vec(),
            }),
            Err(broadcast::error::RecvError::Closed) => None,
        }
    }
}

/// `canonicalize` on Windows yields `\\?\C:\...`; child processes and users expect `C:\...`.
fn strip_verbatim(path: PathBuf) -> PathBuf {
    match path.to_str().and_then(|s| s.strip_prefix(r"\\?\")) {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => PathBuf::from(rest),
        _ => path,
    }
}
