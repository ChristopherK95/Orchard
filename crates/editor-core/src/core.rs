//! The core's public API: the same surface the frontend drives over Tauri IPC (ADR 0003).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};
use tokio::sync::{broadcast, mpsc};

use crate::acp::{AcpError, AdapterCommand, Connection, Incoming, PROTOCOL_VERSION};
use crate::session::{SessionId, SessionInfo, SessionState, Transcript, TranscriptDelta, TranscriptItem};

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
        let out = tokio::process::Command::new("git")
            .args(["rev-parse", "--show-toplevel"])
            .current_dir(path)
            .env("GIT_TERMINAL_PROMPT", "0")
            .output()
            .await
            .map_err(|_| CoreError::NotARepository(path.to_owned()))?;
        if !out.status.success() {
            return Err(CoreError::NotARepository(path.to_owned()));
        }
        let root = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
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

        let mut state = self.inner.state.lock().expect("state lock");
        state.next_session += 1;
        let id = SessionId(state.next_session);
        let info = SessionInfo { id, name: format!("Session {}", id.0), worktree: root, state: SessionState::Idle };
        let (deltas, _) = broadcast::channel(4096);
        let session = Arc::new(Session {
            info: Mutex::new(info.clone()),
            acp_id: acp_id.clone(),
            connection,
            transcript: Mutex::default(),
            deltas,
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
            SessionState::Working => return Err(CoreError::SessionBusy),
            SessionState::Exited => return Err(CoreError::SessionExited),
            SessionState::Idle => {}
        }
        session.record(|t| t.push(TranscriptItem::User { text: text.to_owned() }));
        self.inner.set_state(&session, SessionState::Working);

        let inner = self.inner.clone();
        let params = json!({ "sessionId": session.acp_id, "prompt": [{ "type": "text", "text": text }] });
        tokio::spawn(async move {
            match session.connection.request("session/prompt", params).await {
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
                if !batch.is_empty() && tx.send(std::mem::take(&mut batch)).await.is_err() {
                    return; // the watcher went away
                }
                match deltas.recv().await {
                    Ok(delta) => batch.push(delta),
                    Err(broadcast::error::RecvError::Lagged(_)) => batch.push(session.reset()),
                    Err(broadcast::error::RecvError::Closed) => return,
                }
                let deadline = tokio::time::Instant::now() + FLUSH_INTERVAL;
                while let Ok(received) = tokio::time::timeout_at(deadline, deltas.recv()).await {
                    match received {
                        Ok(delta) => batch.push(delta),
                        Err(broadcast::error::RecvError::Lagged(_)) => batch = vec![session.reset()],
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
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

    /// Handles what the adapter sends us, in arrival order (see `acp`).
    fn handle(&self, incoming: Incoming) {
        match incoming {
            Incoming::Notification { method, params } if method == "session/update" => {
                let Some(session) = self.session_for(&params) else { return };
                let update = &params["update"];
                if update["sessionUpdate"] == "agent_message_chunk" && update["content"]["type"] == "text" {
                    let text = update["content"]["text"].as_str().unwrap_or_default();
                    let message_id = update["messageId"].as_str().map(str::to_owned);
                    session.record(|t| t.append_agent_text(text, message_id));
                }
            }
            Incoming::Notification { .. } => {}
            Incoming::Request { method, params, responder } if method == "session/request_permission" => {
                // Permission cards arrive in ticket 03; until then, decline so the turn can't hang.
                if let Some(session) = self.session_for(&params) {
                    let title = params["toolCall"]["title"].as_str().unwrap_or("a tool call");
                    session.record(|t| {
                        t.push(TranscriptItem::Notice {
                            text: format!("The Agent asked permission for Ã¢â‚¬Å“{title}Ã¢â‚¬Â. Declined: permission cards aren't built yet."),
                        })
                    });
                }
                responder.result(json!({ "outcome": { "outcome": "cancelled" } }));
            }
            Incoming::Request { responder, .. } => responder.error(-32601, "method not supported by this client"),
            Incoming::Closed => {
                let sessions: Vec<_> = self.state.lock().expect("state lock").sessions.values().cloned().collect();
                for session in sessions {
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
    fn record(&self, change: impl FnOnce(&mut Transcript) -> TranscriptDelta) {
        let mut transcript = self.transcript.lock().expect("transcript lock");
        let delta = change(&mut transcript);
        let _ = self.deltas.send(delta);
    }

    fn reset(&self) -> TranscriptDelta {
        TranscriptDelta::Reset { items: self.transcript.lock().expect("transcript lock").items().to_vec() }
    }
}

/// `canonicalize` on Windows yields `\\?\C:\...`; child processes and users expect `C:\...`.
fn strip_verbatim(path: PathBuf) -> PathBuf {
    match path.to_str().and_then(|s| s.strip_prefix(r"\\?\")) {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => PathBuf::from(rest),
        _ => path,
    }
}
