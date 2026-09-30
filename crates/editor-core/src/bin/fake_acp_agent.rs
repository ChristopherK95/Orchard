//! A scripted ACP agent that stands in for `claude-agent-acp` in the core's tests.
//!
//! It speaks newline-delimited JSON-RPC over stdio like the real adapter. Behaviour comes from a
//! JSON script named by `FAKE_ACP_SCRIPT`:
//!
//! ```json
//! { "turns": [ { "chunks": ["Hel", "lo"], "delayMs": 5 }, { "exit": true } ] }
//! ```
//!
//! Each `session/prompt` consumes the next turn (across all sessions); once the script runs out, the
//! agent echoes the prompt. Every received message is appended to the file named by `FAKE_ACP_LOG`,
//! preceded by a `{"started": <pid>}` line, so tests can assert on what the core sent.
//!
//! It also answers `--print <text>` by printing `<text>` and exiting, so tests can stand it in for
//! `git --version` / `node --version`.

use std::collections::VecDeque;
use std::fs::OpenOptions;
use std::io::{BufRead, Write};
use std::time::Duration;

use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Deserialize, Default)]
struct Script {
    #[serde(default)]
    turns: VecDeque<Turn>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct Turn {
    #[serde(default)]
    chunks: Vec<String>,
    #[serde(default)]
    delay_ms: u64,
    /// Exit the process (after sending `chunks`) instead of finishing the turn.
    #[serde(default)]
    exit: bool,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--print") {
        println!("{}", args.get(2).cloned().unwrap_or_default());
        return;
    }

    let mut script: Script = std::env::var("FAKE_ACP_SCRIPT")
        .ok()
        .map(|path| {
            let text = std::fs::read_to_string(path).expect("read FAKE_ACP_SCRIPT");
            serde_json::from_str(&text).expect("FAKE_ACP_SCRIPT is valid JSON")
        })
        .unwrap_or_default();
    let mut log = std::env::var("FAKE_ACP_LOG")
        .ok()
        .map(|path| OpenOptions::new().create(true).append(true).open(path).expect("open FAKE_ACP_LOG"));
    let mut record = |value: &Value| {
        if let Some(file) = log.as_mut() {
            writeln!(file, "{value}").expect("write log");
        }
    };
    record(&json!({ "started": std::process::id() }));

    let stdout = std::io::stdout();
    let send = |value: Value| {
        let mut out = stdout.lock();
        writeln!(out, "{value}").expect("write stdout");
        out.flush().expect("flush stdout");
    };

    let mut sessions = 0;
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let msg: Value = serde_json::from_str(&line).expect("client sent JSON");
        record(&msg);
        let id = msg.get("id").cloned();
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        let result = match msg["method"].as_str() {
            Some("initialize") => json!({ "protocolVersion": 1, "agentCapabilities": {}, "authMethods": [] }),
            Some("session/new") => {
                sessions += 1;
                json!({ "sessionId": format!("fake-{sessions}") })
            }
            Some("session/prompt") => {
                let session_id = params["sessionId"].clone();
                let turn = script.turns.pop_front().unwrap_or_else(|| Turn {
                    chunks: vec!["Echo: ".into(), params["prompt"][0]["text"].as_str().unwrap_or_default().into()],
                    ..Turn::default()
                });
                for chunk in &turn.chunks {
                    std::thread::sleep(Duration::from_millis(turn.delay_ms));
                    send(json!({
                        "jsonrpc": "2.0",
                        "method": "session/update",
                        "params": {
                            "sessionId": session_id,
                            "update": { "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": chunk } }
                        }
                    }));
                }
                if turn.exit {
                    std::process::exit(1);
                }
                json!({ "stopReason": "end_turn" })
            }
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
