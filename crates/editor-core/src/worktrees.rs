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
    /// The PR number of a Review Worktree (set by the core, which keeps the Code reviews).
    pub review: Option<u64>,
}

/// How many Worktrees' `git status` run at once when all are listed: each scans its whole folder,
/// so one after another is slow with many Worktrees, and all at once swamps the disk.
const STATUS_AT_ONCE: usize = 6;

/// Every Worktree of the repository whose main checkout is `root`, main checkout first.
pub(crate) async fn list(root: &Path) -> Vec<WorktreeInfo> {
    let listed = git::worktree_list(root).await.unwrap_or_default();
    let limit = Arc::new(tokio::sync::Semaphore::new(STATUS_AT_ONCE));
    let mut reading = tokio::task::JoinSet::new();
    for (i, worktree) in listed.iter().enumerate() {
        let (limit, path) = (limit.clone(), worktree.path.clone());
        reading.spawn(async move {
            let _turn = limit.acquire().await;
            (i, git::status_summary(&path).await.unwrap_or_default())
        });
    }
    let mut statuses = vec![git::StatusSummary::default(); listed.len()];
    while let Some(Ok((i, status))) = reading.join_next().await {
        statuses[i] = status;
    }
    listed
        .into_iter()
        .zip(statuses)
        .map(|(listed, status)| WorktreeInfo {
            path: normalize(listed.path),
            branch: listed.branch,
            head: listed.head.chars().take(7).collect(),
            is_main: listed.is_main,
            ahead: status.ahead,
            behind: status.behind,
            changed: status.changed,
            review: None,
        })
        .collect()
}

/// Like `list`, but only `refresh`'s branch status (and that of any Worktree not in `previous`)
/// is read again; the others keep theirs from `previous`.
pub(crate) async fn list_refreshing(
    root: &Path,
    previous: &[WorktreeInfo],
    refresh: &Path,
) -> Vec<WorktreeInfo> {
    let mut worktrees = vec![];
    for listed in git::worktree_list(root).await.unwrap_or_default() {
        let path = normalize(listed.path.clone());
        let known = previous.iter().find(|w| w.path == path);
        let (ahead, behind, changed) = match known {
            Some(w) if path != refresh => (w.ahead, w.behind, w.changed),
            _ => {
                let status = git::status_summary(&listed.path).await.unwrap_or_default();
                (status.ahead, status.behind, status.changed)
            }
        };
        worktrees.push(WorktreeInfo {
            path,
            branch: listed.branch,
            head: listed.head.chars().take(7).collect(),
            is_main: listed.is_main,
            ahead,
            behind,
            changed,
            review: None,
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

/// What changed in the repository's shared git folder, as `Discovery` tells it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum RepoChange {
    /// A Worktree may have been added or removed.
    Worktrees,
    /// A local branch moved (it may be checked out in a Worktree).
    Branch(String),
    /// Remote branches moved (a fetch, a push), so any branch's ahead/behind may have.
    Remotes,
}

/// Watches `<git common dir>` and its `worktrees/` folder (non-recursively, so ordinary git activity
/// inside each Worktree's admin folder isn't noise), and the shared `refs/`, and sends what may have
/// changed: a Worktree added or removed, a branch moved, remote branches moved. One watch for the
/// whole repository, so a ref change wakes only the Worktrees it concerns (each Worktree's own git
/// folder is its actor's). Dropping it stops the watch.
pub(crate) struct Discovery {
    watcher: Arc<Mutex<notify::RecommendedWatcher>>,
    worktrees_dir: PathBuf,
}

impl Discovery {
    pub(crate) fn start(common_dir: &Path) -> Option<(Self, mpsc::UnboundedReceiver<RepoChange>)> {
        let (tx, rx) = mpsc::unbounded_channel();
        let worktrees_dir = common_dir.join("worktrees");
        let common = common_dir.to_owned();
        let watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            let Ok(event) = event else { return };
            if !crate::files::is_change(&event) {
                return; // (listing `worktrees/`, as `git worktree list` does, is no change)
            }
            for path in &event.paths {
                if let Some(change) = repo_change(&common, path) {
                    let _ = tx.send(change);
                }
            }
        })
        .ok()?;
        let discovery = Self {
            watcher: Arc::new(Mutex::new(watcher)),
            worktrees_dir,
        };
        {
            let mut watcher = discovery.watcher.lock().expect("watcher lock");
            watcher
                .watch(common_dir, RecursiveMode::NonRecursive)
                .ok()?;
            // (Without it, branches are still seen by the Worktrees' own watches of their HEAD.)
            let _ = watcher.watch(&common_dir.join("refs"), RecursiveMode::Recursive);
        }
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

/// What a change to `path` in the shared git folder `common` means for the Worktrees, if anything:
/// `worktrees/` and its entries; a local branch's ref; remote refs, or `packed-refs` (which holds
/// both). Index and HEAD writes in the folder itself (the main checkout's) are its actor's, and
/// tags, the stash and lock files are noise.
fn repo_change(common: &Path, path: &Path) -> Option<RepoChange> {
    let rel = path.strip_prefix(common).ok()?;
    let rel: Vec<String> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    let rel = rel.join("/");
    if rel.ends_with(".lock") {
        return None;
    }
    if rel == "worktrees" || rel.starts_with("worktrees/") {
        return Some(RepoChange::Worktrees);
    }
    if rel == "packed-refs" || rel.starts_with("refs/remotes/") {
        return Some(RepoChange::Remotes);
    }
    let branch = rel.strip_prefix("refs/heads/")?;
    Some(RepoChange::Branch(branch.to_owned()))
}
