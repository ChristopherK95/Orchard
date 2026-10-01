//! A scripted ACP agent that stands in for `claude-agent-acp` in the core's tests.
//!
//! It speaks newline-delimited JSON-RPC over stdio like the real adapter. Behaviour comes from a
//! JSON script named by `FAKE_ACP_SCRIPT`:
//!
//! ```json
//! {
//!   "initialMode": "default",
//!   "turns": [
//!     { "chunks": ["Hel", "lo"], "delayMs": 5 },
//!     { "permission": { "title": "Edit a.rs", "kind": "edit",
//!                       "diff": { "path": "a.rs", "oldText": "x\n", "newText": "y\n" } },
//!       "chunks": [" done"] },
//!     { "exit": true }
//!   ]
//! }
//! ```
//!
//! Each `session/prompt` consumes the next turn (across all sessions); once the script runs out, the
//! agent echoes the prompt. A `permission` turn first announces the tool call, then asks
//! `session/request_permission` (id `"perm-1"`, `"perm-2"`, …) and waits for the answer, which it
//! echoes as a `[permission <optionId>]` chunk before sending `chunks`.
//!
//! Every received message is appended to the file named by `FAKE_ACP_LOG`, preceded by a
//! `{"started": <pid>}` line, so tests can assert on what the core sent.
//!
//! It also answers `--print <text>` by printing `<text>` and exiting, so tests can stand it in for
//! `git --version` / `node --version`.

use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::{Lines, StdinLock, Write};
use std::time::Duration;

use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Script {
    #[serde(default)]
    turns: VecDeque<Turn>,
    #[serde(default = "default_mode")]
    initial_mode: String,
}

fn default_mode() -> String {
    "default".into()
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct Turn {
    #[serde(default)]
    chunks: Vec<String>,
    /// Separate Agent messages (each with its own message id), sent after `chunks`.
    #[serde(default)]
    messages: Vec<String>,
    #[serde(default)]
    delay_ms: u64,
    /// Ask for permission before sending `chunks`.
    permission: Option<Permission>,
    /// A second question asked alongside `permission`, before either is answered (parallel tool calls).
    also_ask: Option<Permission>,
    /// Ask, but end the turn without waiting for the answer.
    #[serde(default)]
    abandon: bool,
    /// Ask, then exit the process before the answer arrives.
    #[serde(default)]
    exit_while_asking: bool,
    /// Exit the process (after sending `chunks`) instead of finishing the turn.
    #[serde(default)]
    exit: bool,
}

#[derive(Deserialize)]
struct Permission {
    title: String,
    kind: String,
    diff: Option<Value>,
}

struct Agent {
    script: Script,
    log: Option<File>,
    stdin: Lines<StdinLock<'static>>,
    sessions: u32,
    requests: u32,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--print") {
        println!("{}", args.get(2).cloned().unwrap_or_default());
        return;
    }

    let script = std::env::var("FAKE_ACP_SCRIPT")
        .ok()
        .map(|path| {
            let text = std::fs::read_to_string(path).expect("read FAKE_ACP_SCRIPT");
            serde_json::from_str(&text).expect("FAKE_ACP_SCRIPT is valid JSON")
        })
        .unwrap_or(Script { turns: VecDeque::new(), initial_mode: default_mode() });
    let log = std::env::var("FAKE_ACP_LOG")
        .ok()
        .map(|path| OpenOptions::new().create(true).append(true).open(path).expect("open FAKE_ACP_LOG"));
    let mut agent = Agent { script, log, stdin: std::io::stdin().lines(), sessions: 0, requests: 0 };
    agent.record(&json!({ "started": std::process::id() }));
    agent.run();
}

impl Agent {
    fn run(&mut self) {
        while let Some(msg) = self.read() {
            let Some(method) = msg["method"].as_str().map(str::to_owned) else { continue };
            let id = msg.get("id").cloned();
            let params = msg.get("params").cloned().unwrap_or(Value::Null);
            let result = match method.as_str() {
                "initialize" => json!({ "protocolVersion": 1, "agentCapabilities": {}, "authMethods": [] }),
                "session/new" => {
                    self.sessions += 1;
                    json!({
                        "sessionId": format!("fake-{}", self.sessions),
                        "modes": {
                            "currentModeId": self.script.initial_mode,
                            "availableModes": [
                                { "id": "default", "name": "Manual" },
                                { "id": "acceptEdits", "name": "Accept edits" },
                                { "id": "plan", "name": "Plan" },
                                { "id": "auto", "name": "Auto" }
                            ]
                        }
                    })
                }
                "session/set_mode" => json!({}),
                "session/prompt" => self.prompt(&params),
                _ => {
                    if let Some(id) = id {
                        send(json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": "method not found" } }));
                    }
                    continue;
                }
            };
            if let Some(id) = id {
                send(json!({ "jsonrpc": "2.0", "id": id, "result": result }));
            }
        }
    }

    fn prompt(&mut self, params: &Value) -> Value {
        let session_id = params["sessionId"].clone();
        let turn = self.script.turns.pop_front().unwrap_or_else(|| Turn {
            chunks: vec!["Echo: ".into(), params["prompt"][0]["text"].as_str().unwrap_or_default().into()],
            ..Turn::default()
        });
        let mut chunks = turn.chunks;
        let questions: Vec<Permission> = turn.permission.into_iter().chain(turn.also_ask).collect();
        if !questions.is_empty() {
            let request_ids: Vec<String> = questions.into_iter().map(|q| self.ask_permission(&session_id, q)).collect();
            if turn.exit_while_asking {
                std::process::exit(1);
            }
            if !turn.abandon {
                let answers = self.await_answers(&request_ids);
                chunks.insert(0, answers.iter().map(|a| format!("[permission {a}]")).collect());
            }
        }
        for chunk in &chunks {
            std::thread::sleep(Duration::from_millis(turn.delay_ms));
            notify_update(&session_id, json!({ "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": chunk } }));
        }
        for (i, message) in turn.messages.iter().enumerate() {
            notify_update(
                &session_id,
                json!({ "sessionUpdate": "agent_message_chunk", "messageId": format!("msg-{i}"),
                        "content": { "type": "text", "text": message } }),
            );
        }
        if turn.exit {
            std::process::exit(1);
        }
        json!({ "stopReason": "end_turn" })
    }

    /// Announces the tool call and asks permission for it; returns the request id to await.
    fn ask_permission(&mut self, session_id: &Value, permission: Permission) -> String {
        self.requests += 1;
        let tool_call_id = format!("call-{}", self.requests);
        let content: Vec<Value> = permission.diff.iter().map(|d| json!({ "type": "diff", "path": d["path"], "oldText": d["oldText"], "newText": d["newText"] })).collect();
        let locations: Vec<Value> = permission.diff.iter().map(|d| json!({ "path": d["path"] })).collect();
        notify_update(
            session_id,
            json!({
                "sessionUpdate": "tool_call", "toolCallId": tool_call_id, "title": permission.title,
                "kind": permission.kind, "status": "pending", "content": content, "locations": locations
            }),
        );
        let request_id = format!("perm-{}", self.requests);
        send(json!({
            "jsonrpc": "2.0", "id": request_id, "method": "session/request_permission",
            "params": {
                "sessionId": session_id,
                // Like the real adapter, only the id: the details came with the tool call above.
                "toolCall": { "toolCallId": tool_call_id },
                "options": [
                    { "optionId": "allow", "name": "Allow", "kind": "allow_once" },
                    { "optionId": "allow-always", "name": "Always allow", "kind": "allow_always" },
                    { "optionId": "reject", "name": "Reject", "kind": "reject_once" }
                ]
            }
        }));
        request_id
    }

    /// Waits for every request's answer (in any order); returns the chosen option ids (or
    /// `"cancelled"`) in request order.
    fn await_answers(&mut self, request_ids: &[String]) -> Vec<String> {
        let mut answers: Vec<Option<String>> = vec![None; request_ids.len()];
        while answers.iter().any(Option::is_none) {
            let Some(msg) = self.read() else { std::process::exit(0) }; // the client went away
            if msg.get("method").is_some() {
                continue;
            }
            if let Some(i) = request_ids.iter().position(|id| msg["id"] == json!(id)) {
                let outcome = &msg["result"]["outcome"];
                answers[i] = Some(outcome["optionId"].as_str().unwrap_or("cancelled").to_owned());
            }
        }
        answers.into_iter().flatten().collect()
    }

    fn read(&mut self) -> Option<Value> {
        loop {
            let line = self.stdin.next()?.ok()?;
            if line.trim().is_empty() {
                continue;
            }
            let msg: Value = serde_json::from_str(&line).expect("client sent JSON");
            self.record(&msg);
            return Some(msg);
        }
    }

    fn record(&mut self, value: &Value) {
        if let Some(file) = self.log.as_mut() {
            writeln!(file, "{value}").expect("write log");
        }
    }
}

fn notify_update(session_id: &Value, update: Value) {
    send(json!({ "jsonrpc": "2.0", "method": "session/update", "params": { "sessionId": session_id, "update": update } }));
}

fn send(value: Value) {
    let mut out = std::io::stdout().lock();
    writeln!(out, "{value}").expect("write stdout");
    out.flush().expect("flush stdout");
}
