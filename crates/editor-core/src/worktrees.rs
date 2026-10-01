//! The Workspace's Worktrees (ticket 06): listed with `git worktree list` and kept live by watching
//! the repository's `worktrees/` folder, so Worktrees made in a terminal (or by an Agent) show up.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use notify::{RecursiveMode, Watcher};
use serde::Serialize;
use tokio::sync::mpsc;

use crate::git;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeInfo {
    pub path: PathBuf,
    /// The checked-out branch, or `None` when HEAD is detached.
    pub branch: Option<String>,
    /// HEAD's short commit id.
    pub head: String,
    /// The main checkout (always listed first).
    pub is_main: bool,
    /// Commits ahead of / behind the branch's upstream; `None` without an upstream.
    pub ahead: Option<u32>,
    pub behind: Option<u32>,
    /// Changed, staged, unmerged and untracked paths.
    pub changed: u32,
}

/// Every Worktree of the repository whose main checkout is `root`, main checkout first.
pub(crate) async fn list(root: &Path) -> Vec<WorktreeInfo> {
    let mut worktrees = vec![];
    for (i, listed) in git::worktree_list(root)
        .await
        .unwrap_or_default()
        .into_iter()
        .enumerate()
    {
        let status = git::status_summary(&listed.path).await.unwrap_or_default();
        worktrees.push(WorktreeInfo {
            path: normalize(listed.path),
            branch: listed.branch,
            head: listed.head.chars().take(7).collect(),
            is_main: i == 0,
            ahead: status.ahead,
            behind: status.behind,
            changed: status.changed,
        });
    }
    worktrees
}

/// Canonical form without Windows' `\\?\` prefix, so paths from git, the OS and the user compare
/// equal. Falls back to the path as given if it doesn't exist (any more).
pub(crate) fn normalize(path: PathBuf) -> PathBuf {
    match std::fs::canonicalize(&path) {
        Ok(canonical) => match canonical.to_str().and_then(|s| s.strip_prefix(r"\\?\")) {
            Some(rest) if rest.as_bytes().get(1) == Some(&b':') => PathBuf::from(rest),
            _ => canonical,
        },
        Err(_) => path,
    }
}

/// Watches `<git common dir>` and its `worktrees/` folder (non-recursively, so ordinary git activity
/// inside each Worktree's admin folder isn't noise) and sends a signal when a Worktree may have
/// been added or removed. Dropping it stops the watch.
pub(crate) struct Discovery {
    watcher: Arc<Mutex<notify::RecommendedWatcher>>,
    worktrees_dir: PathBuf,
}

impl Discovery {
    pub(crate) fn start(common_dir: &Path) -> Option<(Self, mpsc::UnboundedReceiver<()>)> {
        let (tx, rx) = mpsc::unbounded_channel();
        let watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            let touches_worktrees = event.is_ok_and(|e| {
                e.paths
                    .iter()
                    .any(|p| p.components().any(|c| c.as_os_str() == "worktrees"))
            });
            if touches_worktrees {
                let _ = tx.send(());
            }
        })
        .ok()?;
        let discovery = Self {
            watcher: Arc::new(Mutex::new(watcher)),
            worktrees_dir: common_dir.join("worktrees"),
        };
        discovery
            .watcher
            .lock()
            .expect("watcher lock")
            .watch(common_dir, RecursiveMode::NonRecursive)
            .ok()?;
        discovery.watch_worktrees_dir();
        Some((discovery, rx))
    }

    /// `worktrees/` only exists once a second Worktree does; (re)watch it whenever it might.
    pub(crate) fn watch_worktrees_dir(&self) {
        if self.worktrees_dir.is_dir() {
            let _ = self
                .watcher
                .lock()
                .expect("watcher lock")
                .watch(&self.worktrees_dir, RecursiveMode::NonRecursive);
        }
    }
}
