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
//! A turn's `"write": [{ "path": "a.rs", "text": "…", "afterMs": 0 }]` writes files in the session's
//! folder (after any permission is answered, whatever the answer), like an Agent's edit.
//!
//! A top-level `"failResume": true` makes `session/resume` fail (`Session not found`).
//!
//! A top-level `"listed": [{ "sessionId": "…", "cwd": "…", "title": "…", "updatedAt": "…" }]` is
//! what `session/list` answers with (conversations from elsewhere, like a terminal's), after this
//! process's own sessions that have a finished turn. The `cwd` param is ignored: Claude Code lists
//! the repository's other worktrees' conversations too, so the client has to filter anyway.
//!
//! A top-level `"commands": [{ "name": "review", "description": "…", "input": { "hint": "[pr]" } }]`
//! is sent as an `available_commands_update` after each `session/new`, like the real adapter's.
//!
//! Each `session/prompt` consumes the next turn (across all sessions); once the script runs out, the
//! agent echoes the prompt. A `permission` turn first announces the tool call, then asks
//! `session/request_permission` (id `"perm-1"`, `"perm-2"`, …) and waits for the answer, which it
//! echoes as a `[permission <optionId>]` chunk before sending `chunks`.
//!
//! Each finished turn (the prompt and the Agent's reply) is kept in the JSON file named by
//! `FAKE_ACP_HISTORY`, like Claude Code's own transcripts, so `session/load` can replay it as
//! `user_message_chunk` / `agent_message_chunk` updates, even from a later process.
//! A `session/load` of a session with no finished turn fails with `-32002` ("Resource not
//! found"), as Claude Code's does: it has no conversation until the first message.
//!
//! Every received message is appended to the file named by `FAKE_ACP_LOG`, preceded by a
//! `{"started": <pid>}` line, so tests can assert on what the core sent. A `session/close` is
//! followed by `{"closedWhileCwdExists": <bool>}`: whether the session's folder was still there.
//!
//! It also answers `--print <text>` by printing `<text>` and exiting, so tests can stand it in for
//! `git --version` / `node --version`. With `--until <file>` after that, it waits for `<file>` to exist
//! before exiting (a setup command that's still running for as long as a test needs); with
//! `--beat <file>` as well, it rewrites `<file>` with a counter while it waits (so a test can tell
//! it's alive).

use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::{Lines, StdinLock, Write};
use std::path::Path;
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
    /// Answer `session/resume` with an error (a conversation that can't be picked back up).
    #[serde(default)]
    fail_resume: bool,
    /// Sent as `available_commands_update` after each `session/new` (ACP's command objects).
    #[serde(default)]
    commands: Vec<Value>,
    /// Conversations `session/list` answers with, besides this process's own.
    #[serde(default)]
    listed: Vec<Value>,
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
    /// Tool calls announced (pending) and then updated to their final status, before anything else.
    #[serde(default)]
    tool_calls: Vec<ToolCallScript>,
    /// Tool calls announced once any permission is answered (before `write`).
    #[serde(default)]
    tool_calls_after: Vec<ToolCallScript>,
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
    /// Keep the turn going (Working) until the client sends `session/cancel` (or goes away).
    #[serde(default)]
    until_cancelled: bool,
    /// Files to write (paths relative to the session's folder) once any permission is answered,
    /// whatever the answer, before sending `chunks`: the Agent editing a file.
    #[serde(default)]
    write: Vec<WriteScript>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WriteScript {
    path: String,
    text: String,
    /// Wait this long first.
    #[serde(default)]
    after_ms: u64,
}

#[derive(Deserialize)]
struct ToolCallScript {
    title: String,
    kind: String,
    path: Option<String>,
    #[serde(default = "completed")]
    status: String,
}

fn completed() -> String {
    "completed".into()
}

#[derive(Deserialize)]
struct Permission {
    title: String,
    kind: String,
    diff: Option<Value>,
    /// The file it's about, when there's no diff (a read, say).
    path: Option<String>,
}

struct Agent {
    script: Script,
    log: Option<File>,
    stdin: Lines<StdinLock<'static>>,
    sessions: u32,
    requests: u32,
    /// Each session's working directory, by session id.
    cwds: std::collections::HashMap<String, String>,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--print") {
        println!("{}", args.get(2).cloned().unwrap_or_default());
        if let (Some("--until"), Some(file)) = (args.get(3).map(String::as_str), args.get(4)) {
            let beat = match (args.get(5).map(String::as_str), args.get(6)) {
                (Some("--beat"), Some(beat)) => Some(beat),
                _ => None,
            };
            let mut count = 0u64;
            while !std::path::Path::new(file).exists() {
                if let Some(beat) = beat {
                    count += 1;
                    let _ = std::fs::write(beat, count.to_string());
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        return;
    }

    let script = std::env::var("FAKE_ACP_SCRIPT")
        .ok()
        .map(|path| {
            let text = std::fs::read_to_string(path).expect("read FAKE_ACP_SCRIPT");
            serde_json::from_str(&text).expect("FAKE_ACP_SCRIPT is valid JSON")
        })
        .unwrap_or(Script {
            turns: VecDeque::new(),
            initial_mode: default_mode(),
            fail_resume: false,
            commands: vec![],
            listed: vec![],
        });
    let log = std::env::var("FAKE_ACP_LOG").ok().map(|path| {
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .expect("open FAKE_ACP_LOG")
    });
    let mut agent = Agent {
        script,
        log,
        stdin: std::io::stdin().lines(),
        sessions: 0,
        requests: 0,
        cwds: Default::default(),
    };
    agent.record(&json!({ "started": std::process::id() }));
    agent.run();
}

impl Agent {
    fn run(&mut self) {
        while let Some(msg) = self.read() {
            let Some(method) = msg["method"].as_str().map(str::to_owned) else {
                continue;
            };
            let id = msg.get("id").cloned();
            let params = msg.get("params").cloned().unwrap_or(Value::Null);
            let result = match method.as_str() {
                "initialize" => {
                    json!({
                        "protocolVersion": 1,
                        "agentCapabilities": {
                            "loadSession": true,
                            "sessionCapabilities": { "close": {}, "resume": {}, "list": {} }
                        },
                        "authMethods": []
                    })
                }
                "session/new" => {
                    self.sessions += 1;
                    // Unique across restarts, like the real adapter's ids.
                    let session_id = format!("fake-{}-{}", std::process::id(), self.sessions);
                    let cwd = params["cwd"].as_str().unwrap_or_default().to_owned();
                    self.cwds.insert(session_id.clone(), cwd);
                    let mut created = self.modes();
                    created["sessionId"] = json!(session_id);
                    created
                }
                // Picks a (closed or earlier process's) session back up, without replaying it.
                "session/resume" if self.script.fail_resume => {
                    if let Some(id) = id {
                        send(
                            json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32002, "message": "Session not found" } }),
                        );
                    }
                    continue;
                }
                "session/resume" => {
                    let session_id = params["sessionId"].as_str().unwrap_or_default().to_owned();
                    let cwd = params["cwd"].as_str().unwrap_or_default().to_owned();
                    self.cwds.insert(session_id, cwd);
                    self.modes()
                }
                // Picks a session back up and replays its conversation first.
                "session/load" => {
                    let session_id = params["sessionId"].as_str().unwrap_or_default().to_owned();
                    let history = read_history();
                    // Like Claude Code, which writes a conversation down with its first message.
                    if history.get(&session_id).is_none() {
                        if let Some(id) = id {
                            send(
                                json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32002, "message": format!("Resource not found: {session_id}") } }),
                            );
                        }
                        continue;
                    }
                    let cwd = params["cwd"].as_str().unwrap_or_default().to_owned();
                    self.cwds.insert(session_id.clone(), cwd);
                    for entry in history[&session_id].as_array().into_iter().flatten() {
                        let kind = if entry["role"] == "user" {
                            "user_message_chunk"
                        } else {
                            "agent_message_chunk"
                        };
                        notify_update(
                            &json!(session_id),
                            json!({ "sessionUpdate": kind, "content": { "type": "text", "text": entry["text"] } }),
                        );
                    }
                    self.modes()
                }
                "session/list" => self.list(),
                "session/set_mode" => json!({}),
                "session/close" => self.close(&params),
                "session/prompt" => self.prompt(&params),
                _ => {
                    if let Some(id) = id {
                        send(
                            json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": "method not found" } }),
                        );
                    }
                    continue;
                }
            };
            if let Some(id) = id {
                send(json!({ "jsonrpc": "2.0", "id": id, "result": result }));
            }
            // Like the real adapter: a new session's slash commands follow its creation.
            if method == "session/new" && !self.script.commands.is_empty() {
                let update = json!({ "sessionUpdate": "available_commands_update", "availableCommands": self.script.commands });
                notify_update(&result["sessionId"], update);
            }
        }
    }

    fn list(&self) -> Value {
        let history = read_history();
        let own = self
            .cwds
            .iter()
            .filter(|(id, _)| history.get(id.as_str()).is_some())
            .map(|(id, cwd)| json!({ "sessionId": id, "cwd": cwd }));
        let sessions: Vec<Value> = own.chain(self.script.listed.iter().cloned()).collect();
        json!({ "sessions": sessions })
    }

    fn close(&mut self, params: &Value) -> Value {
        let cwd = params["sessionId"]
            .as_str()
            .and_then(|id| self.cwds.get(id));
        let exists = cwd.is_some_and(|cwd| std::path::Path::new(cwd).exists());
        self.record(&json!({ "closedWhileCwdExists": exists }));
        json!({})
    }

    /// The modes part of a `session/new` or `session/resume` result.
    fn modes(&self) -> Value {
        json!({
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

    fn prompt(&mut self, params: &Value) -> Value {
        let session_id = params["sessionId"].clone();
        let turn = self.script.turns.pop_front().unwrap_or_else(|| Turn {
            chunks: vec![
                "Echo: ".into(),
                params["prompt"][0]["text"]
                    .as_str()
                    .unwrap_or_default()
                    .into(),
            ],
            ..Turn::default()
        });
        self.announce(&session_id, turn.tool_calls);
        let mut chunks = turn.chunks;
        let questions: Vec<Permission> = turn.permission.into_iter().chain(turn.also_ask).collect();
        if !questions.is_empty() {
            let request_ids: Vec<String> = questions
                .into_iter()
                .map(|q| self.ask_permission(&session_id, q))
                .collect();
            if turn.exit_while_asking {
                std::process::exit(1);
            }
            if !turn.abandon {
                let answers = self.await_answers(&request_ids);
                chunks.insert(
                    0,
                    answers
                        .iter()
                        .map(|a| format!("[permission {a}]"))
                        .collect(),
                );
            }
        }
        self.announce(&session_id, turn.tool_calls_after);
        let cwd = session_id
            .as_str()
            .and_then(|id| self.cwds.get(id))
            .cloned()
            .unwrap_or_default();
        for write in &turn.write {
            std::thread::sleep(Duration::from_millis(write.after_ms));
            std::fs::write(Path::new(&cwd).join(&write.path), &write.text).expect("write a file");
        }
        for chunk in &chunks {
            std::thread::sleep(Duration::from_millis(turn.delay_ms));
            notify_update(
                &session_id,
                json!({ "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": chunk } }),
            );
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
        if turn.until_cancelled {
            self.wait_for_cancel();
            return json!({ "stopReason": "cancelled" });
        }
        let reply: String = chunks.iter().chain(&turn.messages).cloned().collect();
        let prompt = params["prompt"][0]["text"].as_str().unwrap_or_default();
        let mut history = read_history();
        let entries = &mut history[session_id.as_str().unwrap_or_default()];
        if !entries.is_array() {
            *entries = json!([]);
        }
        let entries = entries.as_array_mut().expect("array");
        entries.push(json!({ "role": "user", "text": prompt }));
        entries.push(json!({ "role": "agent", "text": reply }));
        if let Ok(path) = std::env::var("FAKE_ACP_HISTORY") {
            std::fs::write(path, history.to_string()).expect("write FAKE_ACP_HISTORY");
        }
        json!({ "stopReason": "end_turn" })
    }

    /// Announces each tool call (pending), then updates it to its final status.
    fn announce(&mut self, session_id: &Value, calls: Vec<ToolCallScript>) {
        for call in calls {
            self.requests += 1;
            let tool_call_id = format!("call-{}", self.requests);
            let locations: Vec<Value> = call.path.iter().map(|p| json!({ "path": p })).collect();
            notify_update(
                session_id,
                json!({ "sessionUpdate": "tool_call", "toolCallId": tool_call_id, "title": call.title,
                        "kind": call.kind, "status": "pending", "locations": locations }),
            );
            notify_update(
                session_id,
                json!({ "sessionUpdate": "tool_call_update", "toolCallId": tool_call_id, "status": call.status }),
            );
        }
    }

    /// Announces the tool call and asks permission for it; returns the request id to await.
    fn ask_permission(&mut self, session_id: &Value, permission: Permission) -> String {
        self.requests += 1;
        let tool_call_id = format!("call-{}", self.requests);
        let content: Vec<Value> = permission.diff.iter().map(|d| json!({ "type": "diff", "path": d["path"], "oldText": d["oldText"], "newText": d["newText"] })).collect();
        let locations: Vec<Value> = permission
            .diff
            .iter()
            .map(|d| json!({ "path": d["path"] }))
            .chain(permission.path.iter().map(|p| json!({ "path": p })))
            .collect();
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

    /// Waits for `session/cancel`, answering other sessions' quick requests meanwhile.
    fn wait_for_cancel(&mut self) {
        loop {
            let Some(msg) = self.read() else {
                std::process::exit(0) // the client went away
            };
            if msg["method"] == "session/cancel" {
                return;
            }
            self.answer_quick(&msg);
        }
    }

    /// While a turn blocks (on a permission answer, or until cancelled): answers another session's
    /// quick request, and refuses anything else rather than leave it hanging.
    fn answer_quick(&mut self, msg: &Value) {
        let (Some(method), Some(id)) = (msg["method"].as_str(), msg.get("id")) else {
            return;
        };
        match method {
            "session/close" => {
                let reply = self.close(&msg["params"]);
                send(json!({ "jsonrpc": "2.0", "id": id, "result": reply }));
            }
            "session/set_mode" => send(json!({ "jsonrpc": "2.0", "id": id, "result": {} })),
            _ => send(json!({ "jsonrpc": "2.0", "id": id, "error": {
                "code": -32000, "message": format!("the fake agent is mid-turn and can't take {method}")
            } })),
        }
    }

    /// Waits for every request's answer (in any order); returns the chosen option ids (or
    /// `"cancelled"`) in request order.
    fn await_answers(&mut self, request_ids: &[String]) -> Vec<String> {
        let mut answers: Vec<Option<String>> = vec![None; request_ids.len()];
        while answers.iter().any(Option::is_none) {
            let Some(msg) = self.read() else {
                std::process::exit(0)
            }; // the client went away
            if msg.get("method").is_some() {
                self.answer_quick(&msg);
                continue;
            }
            if let Some(i) = request_ids.iter().position(|id| msg["id"] == json!(id)) {
                let outcome = &msg["result"]["outcome"];
                answers[i] = Some(
                    outcome["optionId"]
                        .as_str()
                        .unwrap_or("cancelled")
                        .to_owned(),
                );
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

/// The conversations so far, by session id (`{}` without a history file).
fn read_history() -> Value {
    std::env::var("FAKE_ACP_HISTORY")
        .ok()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_else(|| json!({}))
}

fn notify_update(session_id: &Value, update: Value) {
    send(
        json!({ "jsonrpc": "2.0", "method": "session/update", "params": { "sessionId": session_id, "update": update } }),
    );
}

fn send(value: Value) {
    let mut out = std::io::stdout().lock();
    writeln!(out, "{value}").expect("write stdout");
    out.flush().expect("flush stdout");
}
