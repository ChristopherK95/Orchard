//! Creating a Worktree from the editor (ticket 07): where it goes, what it's called, and which
//! branch it starts from. The Worktree lives next to the repo, at `<repo>.worktrees/<folder>/`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::core::CoreError;
use crate::git;
use crate::setup::SetupInfo;
use crate::worktrees::{self, WorktreeInfo};

/// What to create.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum NewWorktree {
    /// A new branch `name`, starting at `start_point` (default: freshly fetched
    /// `origin/<default>`). Not the glossary's "Base", which is what a branch is compared against.
    #[serde(rename_all = "camelCase")]
    NewBranch {
        name: String,
        start_point: Option<String>,
    },
    /// Check out an existing branch: local (`feat/x`) or remote (`origin/feat/x`, which gets a
    /// local branch tracking it).
    ExistingBranch { name: String },
}

/// The created Worktree, and anything the user should know about how it was made.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatedWorktree {
    pub worktree: WorktreeInfo,
    /// E.g. the fetch failed, so the Worktree started from what was fetched last.
    pub warning: Option<String>,
    /// The repo's Worktree setup, now running; its first session starts when it's done. `None`
    /// when the repo has no setup, so the caller starts the session.
    pub setup: Option<SetupInfo>,
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

/// The branch list, and a warning if it couldn't be refreshed from origin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchList {
    pub branches: Vec<BranchInfo>,
    pub warning: Option<String>,
}

/// Creates the Worktree described by `spec` for the repository whose main checkout is `root`, and
/// returns its folder plus any warning. The caller refreshes the Worktree list.
pub(crate) async fn create(
    root: &Path,
    spec: &NewWorktree,
) -> Result<(PathBuf, Option<String>), CoreError> {
    match spec {
        NewWorktree::NewBranch { name, start_point } => {
            let branches = git::branches(root).await.map_err(CoreError::Git)?;
            if branches.iter().any(|b| !b.remote && b.name == *name) {
                return Err(CoreError::BranchExists(name.clone()));
            }
            let fetched = git::fetch_origin(root).await;
            let start = match start_point {
                Some(start) => start.clone(),
                None => git::default_start_point(root).await,
            };
            // Checked by git itself, and never passed on if it looks like an option.
            if start.starts_with('-') || !git::is_commit(root, &start).await {
                return Err(CoreError::UnknownStartPoint(start));
            }
            let path = free_folder(root, name);
            git::worktree_add_new_branch(root, &path, name, &start)
                .await
                .map_err(CoreError::Git)?;
            let warning = fetched.err().map(|err| {
                format!(
                    "Couldn't fetch origin ({err}), so this started from the last fetched {start}."
                )
            });
            Ok((path, warning))
        }
        NewWorktree::ExistingBranch { name } => {
            let fetched = git::fetch_origin(root).await;
            let branches = git::branches(root).await.map_err(CoreError::Git)?;
            let branch = branches
                .iter()
                .find(|b| b.name == *name)
                .ok_or_else(|| CoreError::UnknownBranch(name.clone()))?;
            let local = local_name(&branch.name, branch.remote);
            // A remote branch may already have a local branch of the same name: use that.
            let local_branch = branches.iter().find(|b| !b.remote && b.name == local);
            if let Some(worktree) = local_branch.and_then(|b| b.checked_out_in.clone()) {
                return Err(CoreError::BranchCheckedOut {
                    branch: local.to_owned(),
                    worktree: worktrees::normalize(worktree),
                });
            }
            let path = free_folder(root, local);
            if local_branch.is_some() {
                git::worktree_add_existing(root, &path, local).await
            } else {
                git::worktree_add_tracking(root, &path, local, &branch.name).await
            }
            .map_err(CoreError::Git)?;
            let warning = fetched.err().map(|err| {
                format!("Couldn't fetch origin ({err}); checked out {name} as last fetched.")
            });
            Ok((path, warning))
        }
    }
}

/// `<parent>/<repo>.worktrees/<branch as a folder name>`, with `-2`, `-3`, … if that's taken.
fn free_folder(root: &Path, branch: &str) -> PathBuf {
    let repo = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "repo".into());
    let parent = root
        .parent()
        .unwrap_or(root)
        .join(format!("{repo}.worktrees"));
    let base = folder_name(branch);
    (1..)
        .map(|n| {
            parent.join(if n == 1 {
                base.clone()
            } else {
                format!("{base}-{n}")
            })
        })
        .find(|path| !path.exists())
        .expect("a free folder")
}

const MAX_FOLDER_NAME: usize = 60;

/// A branch name as one safe, reasonably short folder name: `agent/fix-login` → `agent-fix-login`.
fn folder_name(branch: &str) -> String {
    let name: String = branch
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '-'
            }
        })
        .collect();
    let name: String = name
        .trim_matches(['-', '.'])
        .chars()
        .take(MAX_FOLDER_NAME)
        .collect();
    let name = name.trim_end_matches(['-', '.']).to_owned();
    if name.is_empty() {
        "worktree".into()
    } else {
        name
    }
}

/// The first `agent/task-N` not already taken locally or on a remote.
pub(crate) fn suggest_name(branches: &[BranchInfo]) -> String {
    let taken: Vec<&str> = branches
        .iter()
        .map(|b| local_name(&b.name, b.remote))
        .collect();
    (1..)
        .map(|n| format!("agent/task-{n}"))
        .find(|name| !taken.contains(&name.as_str()))
        .expect("an unused name")
}

/// The local branch name for an existing branch: `origin/feat/x` becomes `feat/x`.
fn local_name(branch: &str, remote: bool) -> &str {
    if remote {
        branch
            .split_once('/')
            .map(|(_, rest)| rest)
            .unwrap_or(branch)
    } else {
        branch
    }
}
