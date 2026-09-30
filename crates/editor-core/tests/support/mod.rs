//! Test harness for driving the core through its public API against the fake ACP agent
//! (see the spec's Testing Decisions). Every core test should build on this.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use editor_core::{
    AdapterCommand, Core, CoreConfig, CoreEvent, SessionId, SessionState, TranscriptDelta,
    TranscriptItem, TranscriptStream,
};
use tokio::sync::broadcast;

pub const TIMEOUT: Duration = Duration::from_secs(10);

/// A temporary directory holding a fresh git repository with one commit.
pub fn git_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("temp dir");
    let git = |args: &[&str]| {
        let status = Command::new("git").args(args).current_dir(dir.path()).status().expect("run git");
        assert!(status.success(), "git {args:?} failed");
    };
    git(&["init", "--quiet"]);
    git(&["-c", "user.name=Test", "-c", "user.email=test@example.com", "commit", "--quiet", "--allow-empty", "-m", "init"]);
    dir
}

pub fn fake_agent_path() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_fake-acp-agent"))
}

/// A fake-agent script plus a log file the fake agent appends every received message to.
pub struct FakeAgent {
    dir: tempfile::TempDir,
}

impl FakeAgent {
    pub fn new(script: &str) -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::write(dir.path().join("script.json"), script).expect("write script");
        Self { dir }
    }

    /// Replaces the script; takes effect when the fake agent process starts (on the first session).
    pub fn set_script(&self, script: &str) {
        std::fs::write(self.dir.path().join("script.json"), script).expect("write script");
    }

    pub fn command(&self) -> AdapterCommand {
        AdapterCommand {
            program: fake_agent_path(),
            args: vec![],
            env: vec![
                ("FAKE_ACP_SCRIPT".into(), self.dir.path().join("script.json").display().to_string()),
                ("FAKE_ACP_LOG".into(), self.log_path().display().to_string()),
            ],
        }
    }

    pub fn log_path(&self) -> PathBuf {
        self.dir.path().join("log.jsonl")
    }

    /// Every JSON message the fake agent received (plus its `{"started": pid}` lines).
    pub fn log(&self) -> Vec<serde_json::Value> {
        std::fs::read_to_string(self.log_path())
            .unwrap_or_default()
            .lines()
            .map(|l| serde_json::from_str(l).expect("log line is JSON"))
            .collect()
    }

    pub fn received(&self, method: &str) -> Vec<serde_json::Value> {
        self.log().into_iter().filter(|m| m["method"] == method).collect()
    }
}

pub fn core_with(agent: &FakeAgent) -> Core {
    Core::new(CoreConfig { adapter: agent.command() })
}

/// Polls `check` until it holds (or panics after `TIMEOUT`), for facts no event announces.
pub async fn eventually(what: &str, mut check: impl FnMut() -> bool) {
    tokio::time::timeout(TIMEOUT, async {
        while !check() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {what}"));
}

/// Waits for state changes of `session`, returning them in order, until `last` is seen.
pub async fn states_until(
    events: &mut broadcast::Receiver<CoreEvent>,
    session: SessionId,
    last: SessionState,
) -> Vec<SessionState> {
    let mut seen = vec![];
    tokio::time::timeout(TIMEOUT, async {
        loop {
            if let CoreEvent::SessionStateChanged { session_id, state } = events.recv().await.expect("event") {
                if session_id == session {
                    seen.push(state);
                    if state == last {
                        return;
                    }
                }
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {last:?}; saw {seen:?}"));
    seen
}

/// Applies streamed deltas the way the frontend does, until the rebuilt transcript equals `expected`.
/// Returns how many batches it took.
pub async fn stream_until(stream: &mut TranscriptStream, expected: &[TranscriptItem]) -> usize {
    let mut rebuilt: Vec<TranscriptItem> = vec![];
    let mut batches = 0;
    tokio::time::timeout(TIMEOUT, async {
        while rebuilt != expected {
            let batch = stream.next().await.expect("stream open");
            batches += 1;
            for delta in batch {
                apply(&mut rebuilt, delta);
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out; rebuilt {rebuilt:?}, expected {expected:?}"));
    batches
}

fn apply(items: &mut Vec<TranscriptItem>, delta: TranscriptDelta) {
    match delta {
        TranscriptDelta::Reset { items: all } => *items = all,
        TranscriptDelta::ItemAdded { index, item } => {
            assert_eq!(index, items.len(), "items are appended in order");
            items.push(item);
        }
        TranscriptDelta::ItemUpdated { index, item } => items[index] = item,
        TranscriptDelta::TextAppended { index, text } => match &mut items[index] {
            TranscriptItem::Agent { text: t } => t.push_str(&text),
            other => panic!("text appended to non-agent item {other:?}"),
        },
    }
}

pub fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).expect("canonicalize")
}
