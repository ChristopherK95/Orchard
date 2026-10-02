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

/// A temporary directory holding a fresh git repository (branch `main`) with one commit.
pub fn git_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("temp dir");
    git(dir.path(), &["init", "--quiet", "-b", "main"]);
    commit(dir.path(), "init");
    dir
}

/// A repository cloned from a bare `origin` (both on branch `main`), in a temp folder of its own so
/// Worktrees created next to the repo stay inside it: `<tmp>/origin.git`, `<tmp>/repo`.
pub struct RepoWithOrigin {
    dir: tempfile::TempDir,
}

impl RepoWithOrigin {
    pub fn new() -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        let origin = dir.path().join("origin.git");
        git(
            dir.path(),
            &[
                "init",
                "--quiet",
                "--bare",
                "-b",
                "main",
                origin.to_str().unwrap(),
            ],
        );
        let seed = dir.path().join("seed");
        git(
            dir.path(),
            &["init", "--quiet", "-b", "main", seed.to_str().unwrap()],
        );
        git(
            &seed,
            &["remote", "add", "origin", origin.to_str().unwrap()],
        );
        commit(&seed, "init");
        git(&seed, &["push", "--quiet", "origin", "main"]);
        git(
            dir.path(),
            &["clone", "--quiet", origin.to_str().unwrap(), "repo"],
        );
        Self { dir }
    }

    pub fn repo(&self) -> PathBuf {
        self.dir.path().join("repo")
    }

    pub fn root(&self) -> &Path {
        self.dir.path()
    }

    /// Adds a commit to origin's `branch` (from another clone), returning its id.
    pub fn push_to_origin(&self, branch: &str, message: &str) -> String {
        let seed = self.dir.path().join("seed");
        git(&seed, &["checkout", "--quiet", "-B", branch]);
        commit(&seed, message);
        git(&seed, &["push", "--quiet", "origin", branch]);
        rev_parse(&seed, "HEAD")
    }
}

pub fn rev_parse(dir: &Path, rev: &str) -> String {
    let out = Command::new("git")
        .args(["rev-parse", rev])
        .current_dir(dir)
        .output()
        .expect("run git");
    assert!(out.status.success(), "git rev-parse {rev} failed");
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// Runs `git` in `dir`, panicking on failure.
pub fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(dir)
        .status()
        .expect("run git");
    assert!(status.success(), "git {args:?} failed in {}", dir.display());
}

/// An empty commit in `dir`.
pub fn commit(dir: &Path, message: &str) {
    git(
        dir,
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "--quiet",
            "--allow-empty",
            "-m",
            message,
        ],
    );
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
                (
                    "FAKE_ACP_SCRIPT".into(),
                    self.dir.path().join("script.json").display().to_string(),
                ),
                ("FAKE_ACP_LOG".into(), self.log_path().display().to_string()),
                (
                    "FAKE_ACP_HISTORY".into(),
                    self.dir.path().join("history.json").display().to_string(),
                ),
            ],
        }
    }

    /// Where a core made with `core_with_state` keeps its app state.
    pub fn state_path(&self) -> PathBuf {
        self.dir.path().join("state.json")
    }

    pub fn settings_path(&self) -> PathBuf {
        self.dir.path().join("settings.toml")
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

    /// How many times the fake agent process has started.
    pub fn starts(&self) -> usize {
        self.log()
            .iter()
            .filter(|m| m.get("started").is_some())
            .count()
    }

    /// Kills the running fake agent process (the adapter crashing).
    pub fn kill(&self) {
        let pid = self
            .log()
            .iter()
            .rev()
            .find_map(|m| m["started"].as_u64())
            .expect("the fake agent started");
        let status = if cfg!(windows) {
            Command::new("taskkill")
                .args(["/F", "/PID", &pid.to_string()])
                .output()
        } else {
            Command::new("kill").args(["-9", &pid.to_string()]).output()
        }
        .expect("run kill");
        assert!(
            status.status.success(),
            "couldn't kill the fake agent {pid}"
        );
    }

    pub fn received(&self, method: &str) -> Vec<serde_json::Value> {
        self.log()
            .into_iter()
            .filter(|m| m["method"] == method)
            .collect()
    }
}

pub fn core_with(agent: &FakeAgent) -> Core {
    Core::new(CoreConfig {
        adapter: agent.command(),
        settings_path: None,
        state_path: None,
    })
}

/// A core persisting its app state in the fake agent's temp folder; a second one made the same way
/// is the editor after a restart.
pub fn core_with_state(agent: &FakeAgent) -> Core {
    Core::new(CoreConfig {
        adapter: agent.command(),
        settings_path: None,
        state_path: Some(agent.state_path()),
    })
}

/// A core reading its settings from `settings.toml` in the fake agent's temp folder, written first.
pub fn core_with_settings(agent: &FakeAgent, settings: &str) -> Core {
    std::fs::write(agent.settings_path(), settings).expect("write settings");
    Core::new(CoreConfig {
        adapter: agent.command(),
        settings_path: Some(agent.settings_path()),
        state_path: None,
    })
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

/// Waits for the first event `pick` accepts, returning what it made of it.
pub async fn next_event<T>(
    events: &mut broadcast::Receiver<CoreEvent>,
    what: &str,
    mut pick: impl FnMut(CoreEvent) -> Option<T>,
) -> T {
    tokio::time::timeout(TIMEOUT, async {
        loop {
            if let Some(found) = pick(events.recv().await.expect("event")) {
                return found;
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {what}"))
}

/// A TOML literal string (no escapes, so Windows paths can go in as they are).
pub fn toml_literal(text: &str) -> String {
    assert!(
        !text.contains('\''),
        "{text} can't be a TOML literal string"
    );
    format!("'{text}'")
}

/// A settings-file section for the repo keyed `key` (an origin URL or a path).
pub fn repo_settings(key: &str, body: &str) -> String {
    format!("[repos.{}]\n{body}\n", toml_literal(key))
}

/// A setup command that prints `text`, then runs until `until` exists. Forward slashes, so the
/// same text works in bash and PowerShell.
pub fn print_then_wait(text: &str, until: &Path) -> String {
    let slashes = |p: &Path| p.display().to_string().replace('\\', "/");
    format!(
        "{} --print {text} --until {}",
        slashes(&fake_agent_path()),
        slashes(until)
    )
}

/// `origin`'s URL as git reports it, which is how a repo's settings section is keyed.
pub fn origin_url(repo: &Path) -> String {
    let out = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .current_dir(repo)
        .output()
        .expect("run git");
    assert!(out.status.success(), "git remote get-url failed");
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
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
            if let CoreEvent::SessionStateChanged { session_id, state } =
                events.recv().await.expect("event")
            {
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

/// Applies streamed deltas the way the frontend does, until the rebuilt transcript (from the start
/// of the first page sent) equals `expected`. Returns how many batches it took.
pub async fn stream_until(stream: &mut TranscriptStream, expected: &[TranscriptItem]) -> usize {
    let mut rebuilt = Rebuilt::default();
    let mut batches = 0;
    tokio::time::timeout(TIMEOUT, async {
        while rebuilt.items != expected {
            let batch = stream.next().await.expect("stream open");
            batches += 1;
            for delta in batch {
                rebuilt.apply(delta);
            }
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!(
            "timed out; rebuilt {:?}, expected {expected:?}",
            rebuilt.items
        )
    });
    batches
}

/// The frontend's view of a transcript: the items from `start` on (indexes in deltas are absolute).
#[derive(Default)]
struct Rebuilt {
    start: usize,
    items: Vec<TranscriptItem>,
}

impl Rebuilt {
    fn apply(&mut self, delta: TranscriptDelta) {
        match delta {
            TranscriptDelta::Reset { start, items } => (self.start, self.items) = (start, items),
            TranscriptDelta::ItemAdded { index, item } => {
                assert_eq!(
                    index,
                    self.start + self.items.len(),
                    "items are appended in order"
                );
                self.items.push(item);
            }
            // Changes to items before the loaded page don't concern the view.
            TranscriptDelta::ItemUpdated { index, .. }
            | TranscriptDelta::TextAppended { index, .. }
                if index < self.start => {}
            TranscriptDelta::ItemUpdated { index, item } => self.items[index - self.start] = item,
            TranscriptDelta::TextAppended { index, text } => {
                match &mut self.items[index - self.start] {
                    TranscriptItem::Agent { text: t } => t.push_str(&text),
                    other => panic!("text appended to non-Agent item {other:?}"),
                }
            }
        }
    }
}

pub fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).expect("canonicalize")
}
