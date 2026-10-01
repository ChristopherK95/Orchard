//! Creating a Worktree from the editor (ticket 07): where it goes, what it's called, and which
//! branch it starts from. The Worktree lives next to the repo, at `<repo>.worktrees/<slug>/`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// What to create.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum NewWorktree {
    /// A new branch `name`, from `base` (default: freshly fetched `origin/<default>`).
    NewBranch { name: String, base: Option<String> },
    /// Check out an existing branch: local (`feat/x`) or remote (`origin/feat/x`, which gets a
    /// local branch tracking it).
    ExistingBranch { name: String },
}

/// A branch, as the "existing branch" picker shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchInfo {
    /// `feat/x` for a local branch, `origin/feat/x` for a remote one.
    pub name: String,
    pub remote: bool,
    /// The Worktree a local branch is checked out in; such a branch can't be checked out again.
    pub checked_out_in: Option<PathBuf>,
}

/// `<parent>/<repo>.worktrees/<branch as a folder name>`.
pub(crate) fn folder_for(root: &Path, branch: &str) -> PathBuf {
    let repo = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "repo".into());
    let parent = root.parent().unwrap_or(root);
    parent.join(format!("{repo}.worktrees")).join(slug(branch))
}

/// A branch name as a single safe folder name: `agent/fix-login` becomes `agent-fix-login`.
fn slug(branch: &str) -> String {
    let slug: String = branch
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let slug = slug.trim_matches(['-', '.']).to_owned();
    if slug.is_empty() {
        "worktree".into()
    } else {
        slug
    }
}

/// The first `agent/task-N` not already taken.
pub(crate) fn suggest_name(taken: &[String]) -> String {
    (1..)
        .map(|n| format!("agent/task-{n}"))
        .find(|name| !taken.contains(name))
        .expect("an unused name")
}

/// The local branch name for an existing branch: `origin/feat/x` becomes `feat/x`.
pub(crate) fn local_name(branch: &str, remote: bool) -> &str {
    if remote {
        branch
            .split_once('/')
            .map(|(_, rest)| rest)
            .unwrap_or(branch)
    } else {
        branch
    }
}
