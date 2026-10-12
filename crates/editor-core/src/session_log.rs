//! The session log (issue 47, for 46): each Agent session's start, state changes and close, with
//! times, appended one JSON line each to a file of its own, kept a year. Shown nowhere yet.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::Serialize;

use crate::session::SessionState;

/// One line of the log. `at` is in milliseconds since the Unix epoch; `session` is the ACP id,
/// which stays the same across restarts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub(crate) enum Entry {
    /// The editor holds the session from now: `new` when the Agent just created it, else it was
    /// restored or reopened.
    Opened {
        at: u64,
        session: String,
        workspace: PathBuf,
        worktree: PathBuf,
        new: bool,
        state: SessionState,
    },
    /// Its state changed.
    State {
        at: u64,
        session: String,
        state: SessionState,
    },
    /// The editor no longer holds it: its Tab was closed, or its Worktree removed, or the
    /// Workspace closed.
    Closed { at: u64, session: String },
}

/// How long entries are kept: a year, in milliseconds.
const KEEP_FOR: u64 = 365 * 24 * 60 * 60 * 1000;

/// When a line was written, if it can be read.
fn written_at(line: &str) -> Option<u64> {
    serde_json::from_str::<serde_json::Value>(line).ok()?["at"].as_u64()
}

/// Drops the lines of the log at `path` written before `since` (and unreadable ones), if its first
/// line is one of them; through a `.partial` file and a rename, so it's never left half-written.
/// No file is fine.
fn prune(path: &Path, since: u64) -> std::io::Result<()> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err),
    };
    let mut first = String::new();
    std::io::BufRead::read_line(&mut std::io::BufReader::new(file), &mut first)?;
    if first.is_empty() || written_at(&first).is_some_and(|at| at >= since) {
        return Ok(());
    }
    let kept: String = std::fs::read_to_string(path)?
        .lines()
        .filter(|line| written_at(line).is_some_and(|at| at >= since))
        .flat_map(|line| [line, "\n"])
        .collect();
    // Per process, as the app state's, so two editors starting at once don't write the same file.
    let partial = path.with_extension(format!("jsonl.{}.partial", std::process::id()));
    std::fs::write(&partial, kept)?;
    std::fs::rename(&partial, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&partial);
    })
}

/// The log file, appended to one line at a time.
pub(crate) struct SessionLog {
    path: PathBuf,
    /// Held for each append, so lines never interleave. The innermost lock: nothing is locked, or
    /// awaited, while it's held.
    writing: Mutex<()>,
}

impl SessionLog {
    /// The log at `path`, its entries older than `KEEP_FOR` dropped first. Only the first line is
    /// read to tell: the file is rewritten only when that one is old (or unreadable).
    pub(crate) fn open(path: PathBuf, now_ms: u64) -> SessionLog {
        let since = now_ms.saturating_sub(KEEP_FOR);
        if let Err(err) = prune(&path, since) {
            eprintln!("couldn't drop old entries from {}: {err}", path.display());
        }
        SessionLog {
            path,
            writing: Mutex::new(()),
        }
    }

    /// Appends an entry with one write (the file and its folder made if missing). A failure is
    /// only reported: the log must never break a session.
    pub(crate) fn append(&self, entry: Entry) {
        let mut line = match serde_json::to_string(&entry) {
            Ok(line) => line,
            Err(err) => return eprintln!("couldn't record a session's change: {err}"),
        };
        line.push('\n');
        let _writing = self.writing.lock().expect("session log lock");
        let written = (|| {
            if let Some(folder) = self.path.parent() {
                std::fs::create_dir_all(folder)?;
            }
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.path)?
                .write_all(line.as_bytes())
        })();
        if let Err(err) = written {
            eprintln!(
                "couldn't record a session's change in {}: {err}",
                self.path.display()
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(path: &Path) -> Vec<serde_json::Value> {
        std::fs::read_to_string(path)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    #[test]
    fn entries_are_appended_as_json_lines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("not yet").join("sessions.jsonl");
        let log = SessionLog::open(path.clone(), 1_000);
        log.append(Entry::Opened {
            at: 1_000,
            session: "acp-1".into(),
            workspace: PathBuf::from("/repo"),
            worktree: PathBuf::from("/repo.worktrees/x"),
            new: true,
            state: SessionState::Idle,
        });
        log.append(Entry::State {
            at: 2_000,
            session: "acp-1".into(),
            state: SessionState::Working,
        });
        let lines = lines(&path);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["kind"], "opened");
        assert_eq!(lines[0]["session"], "acp-1");
        assert_eq!(lines[0]["new"], true);
        assert_eq!(lines[0]["worktree"], "/repo.worktrees/x");
        assert_eq!(lines[1]["kind"], "state");
        assert_eq!(lines[1]["session"], "acp-1");
        assert_eq!(lines[1]["state"], "working");
        assert_eq!(lines[1]["at"], 2_000);
    }

    #[test]
    fn entries_older_than_a_year_are_dropped_on_open() {
        const DAY_MS: u64 = 86_400_000;
        let now = 1_000 * DAY_MS;
        let line = |at: u64| format!(r#"{{"kind":"closed","at":{at},"session":"acp-1"}}"#);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sessions.jsonl");
        let old = [
            line(now - 400 * DAY_MS),
            "not json".to_owned(),
            line(now - 10 * DAY_MS),
            line(now - DAY_MS),
        ];
        std::fs::write(&path, old.join("\n") + "\n").unwrap();
        SessionLog::open(path.clone(), now);
        let left: Vec<u64> = lines(&path)
            .iter()
            .map(|l| l["at"].as_u64().unwrap())
            .collect();
        assert_eq!(left, [now - 10 * DAY_MS, now - DAY_MS]);

        // A recent first line: the file isn't rewritten (its bad line stays).
        let recent = [line(now - 10 * DAY_MS), "not json".to_owned()].join("\n") + "\n";
        std::fs::write(&path, &recent).unwrap();
        SessionLog::open(path.clone(), now);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), recent);

        // No file yet is fine.
        SessionLog::open(dir.path().join("none.jsonl"), now);
        assert!(!dir.path().join("none.jsonl").exists());
    }
}
