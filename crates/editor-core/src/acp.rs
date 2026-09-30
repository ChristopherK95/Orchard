//! The connection to the shared ACP adapter process: newline-delimited JSON-RPC 2.0 over stdio.
//!
//! Incoming notifications and requests are handed to a callback *on the reader task, in arrival
//! order*, and responses to our own requests are resolved on that same task. So everything the
//! adapter sent before a response has been handled by the time the awaiting caller sees it.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Child;
use tokio::sync::{mpsc, oneshot};

/// ACP protocol version this client speaks.
pub const PROTOCOL_VERSION: u64 = 1;

/// How to start the ACP adapter. Configurable so tests can substitute the fake agent.
#[derive(Debug, Clone)]
pub struct AdapterCommand {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum AcpError {
    #[error("could not start the ACP adapter `{program}`: {message}")]
    Spawn { program: String, message: String },
    #[error("the ACP adapter exited")]
    Closed,
    #[error("the ACP adapter returned error {code}: {message}")]
    Rpc { code: i64, message: String },
    #[error("the ACP adapter speaks protocol version {0}, but this editor needs version {PROTOCOL_VERSION}")]
    UnsupportedProtocol(Value),
}

pub(crate) enum Incoming {
    Notification { method: String, params: Value },
    Request { method: String, params: Value, responder: Responder },
    /// The adapter process closed its stdout (exited or crashed).
    Closed,
}

/// Replies to one request the adapter sent us.
pub(crate) struct Responder {
    id: Value,
    out: mpsc::UnboundedSender<String>,
}

impl Responder {
    pub(crate) fn result(self, result: Value) {
        let _ = self.out.send(json!({ "jsonrpc": "2.0", "id": self.id, "result": result }).to_string());
    }

    pub(crate) fn error(self, code: i64, message: &str) {
        let _ = self
            .out
            .send(json!({ "jsonrpc": "2.0", "id": self.id, "error": { "code": code, "message": message } }).to_string());
    }
}

type Pending = Arc<Mutex<Option<HashMap<u64, oneshot::Sender<Result<Value, AcpError>>>>>>;

pub(crate) struct Connection {
    out: mpsc::UnboundedSender<String>,
    /// `None` once the adapter has closed; no new requests are accepted then.
    pending: Pending,
    next_id: AtomicU64,
    /// Held so the process is killed when the connection is dropped.
    _child: Child,
}

impl Connection {
    pub(crate) fn spawn(
        command: &AdapterCommand,
        on_incoming: impl Fn(Incoming) + Send + 'static,
    ) -> Result<Self, AcpError> {
        let mut cmd = crate::process::command(&command.program);
        cmd.args(&command.args)
            .envs(command.env.iter().map(|(k, v)| (k, v)))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = cmd.spawn().map_err(|e| AcpError::Spawn {
            program: command.program.display().to_string(),
            message: e.to_string(),
        })?;

        let mut stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");

        let (out, mut out_rx) = mpsc::unbounded_channel::<String>();
        tokio::spawn(async move {
            while let Some(line) = out_rx.recv().await {
                let written = async {
                    stdin.write_all(line.as_bytes()).await?;
                    stdin.write_all(b"\n").await?;
                    stdin.flush().await
                };
                if written.await.is_err() {
                    break;
                }
            }
        });

        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                eprintln!("[acp adapter] {line}");
            }
        });

        let pending: Pending = Arc::new(Mutex::new(Some(HashMap::new())));
        let reader_pending = pending.clone();
        let reader_out = out.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let Ok(msg) = serde_json::from_str::<Value>(&line) else {
                    eprintln!("[acp adapter] ignoring non-JSON line: {line}");
                    continue;
                };
                dispatch(msg, &reader_pending, &reader_out, &on_incoming);
            }
            if let Some(waiting) = reader_pending.lock().expect("pending lock").take() {
                for (_, tx) in waiting {
                    let _ = tx.send(Err(AcpError::Closed));
                }
            }
            on_incoming(Incoming::Closed);
        });

        Ok(Self { out, pending, next_id: AtomicU64::new(1), _child: child })
    }

    pub(crate) async fn request(&self, method: &str, params: Value) -> Result<Value, AcpError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        match self.pending.lock().expect("pending lock").as_mut() {
            Some(waiting) => waiting.insert(id, tx),
            None => return Err(AcpError::Closed),
        };
        let msg = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        if self.out.send(msg.to_string()).is_err() {
            return Err(AcpError::Closed);
        }
        rx.await.unwrap_or(Err(AcpError::Closed))
    }

    pub(crate) fn is_closed(&self) -> bool {
        self.pending.lock().expect("pending lock").is_none()
    }
}

fn dispatch(
    msg: Value,
    pending: &Pending,
    out: &mpsc::UnboundedSender<String>,
    on_incoming: &impl Fn(Incoming),
) {
    let id = msg.get("id").filter(|id| !id.is_null()).cloned();
    if let Some(method) = msg.get("method").and_then(Value::as_str) {
        let method = method.to_owned();
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        on_incoming(match id {
            Some(id) => Incoming::Request { method, params, responder: Responder { id, out: out.clone() } },
            None => Incoming::Notification { method, params },
        });
        return;
    }
    let Some(id) = id.as_ref().and_then(Value::as_u64) else { return };
    let Some(tx) = pending.lock().expect("pending lock").as_mut().and_then(|w| w.remove(&id)) else { return };
    let outcome = match msg.get("error") {
        Some(err) => Err(AcpError::Rpc {
            code: err["code"].as_i64().unwrap_or(0),
            message: err["message"].as_str().unwrap_or_default().to_owned(),
        }),
        None => Ok(msg.get("result").cloned().unwrap_or(Value::Null)),
    };
    let _ = tx.send(outcome);
}
