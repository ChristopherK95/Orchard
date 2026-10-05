//! Whether a Worktree's branch has been merged into its Base, so the Worktree can be removed: the
//! Worktrees overview lists every Worktree with this, and the removal check uses it too.
//!
//! A PR can land as a merge commit, a fast-forward, a squash or a rebase. The first two keep the
//! branch's commits (they're in the Base's history); the last two make new ones, so what's checked
//! is whether the Base already has the branch's changes. A remote branch deleted after its PR
//! closed is reported too, though on its own it doesn't say the PR was merged.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::git;

/// How a Worktree's branch stands against its Base (or, failing that, another branch it was merged
/// into: a PR can target a project branch rather than the default one).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum MergeState {
    /// Its commits are in `into`'s history (a merge commit, or a fast-forward into the Base).
    Merged { into: String },
    /// Its commits aren't, but its changes are: a squash or rebase merge into `into`.
    ChangesInBase { into: String },
    /// It has no commits of its own: nothing was done on it yet (or it was reset to the Base).
    NothingNew,
    /// It has `commits` the Base hasn't, and its remote branch was deleted: the PR may have been
    /// merged in a way that doesn't show here, or closed without merging.
    RemoteDeleted { commits: u32 },
    /// It has `commits` the Base hasn't.
    NotMerged { commits: u32 },
}

impl MergeState {
    /// Removing the Worktree (and its branch) loses nothing the Base doesn't have.
    pub(crate) fn in_base(&self) -> bool {
        matches!(
            self,
            Self::Merged { .. } | Self::ChangesInBase { .. } | Self::NothingNew
        )
    }
}

/// One row of the Worktrees overview.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeMerge {
    pub path: PathBuf,
    /// `None` when HEAD is detached.
    pub branch: Option<String>,
    pub is_main: bool,
    /// What it's measured against: the Worktree's Base.
    pub base: String,
    /// `None` for the main checkout, and when it couldn't be told (see `error`).
    pub merge: Option<MergeState>,
    pub error: Option<String>,
    /// Uncommitted changes, which removing the Worktree would lose whatever `merge` says.
    pub changed: u32,
}

/// How many other remote branches are looked at when the branch isn't merged into its Base.
const OTHER_TARGETS: usize = 25;

/// How `worktree`'s HEAD stands against its Base, `base` (shown by name) at `base_id` (a commit id,
/// resolved in the main checkout so the Worktree's own `HEAD` can't stand in for the Base). If it
/// isn't merged there, the remote branches that moved since its last commit are tried: a PR into a
/// project branch counts too.
pub(crate) async fn detect(
    worktree: &Path,
    branch: Option<&str>,
    base: &str,
    base_id: &str,
) -> MergeState {
    let head = git::resolve_commit(worktree, "HEAD")
        .await
        .unwrap_or_default();
    if git::is_merged(worktree, &head, base_id).await {
        // On the Base's own line it either never left it or was fast-forwarded in: only one with
        // commits made on it did any work.
        let fast_forwarded = match branch {
            Some(branch) => git::commits_were_made_on(worktree, branch).await,
            None => false,
        };
        let off_the_line = !git::on_first_parent_line(worktree, &head, base_id).await;
        return match off_the_line || fast_forwarded {
            true => MergeState::Merged { into: base.into() },
            false => MergeState::NothingNew,
        };
    }
    // (Commits that change nothing, such as empty ones, don't count as found anywhere.)
    let split = git::merge_base(worktree, base_id).await.ok();
    let changes_something = match &split {
        Some(split) => git::trees_differ(worktree, split, &head).await,
        None => true,
    };
    if changes_something && has_changes_of(worktree, base_id, &head).await {
        return MergeState::ChangesInBase { into: base.into() };
    }
    if changes_something {
        if let Some(merged) = merged_elsewhere(worktree, branch, base_id, &head).await {
            return merged;
        }
    }
    let commits = git::count_not_in(worktree, base_id, &head).await;
    match branch {
        Some(branch) if git::upstream_gone(worktree, branch).await => {
            MergeState::RemoteDeleted { commits }
        }
        _ => MergeState::NotMerged { commits },
    }
}

/// Whether `target` has everything `head` changed: a squash or rebase merge.
async fn has_changes_of(worktree: &Path, target: &str, head: &str) -> bool {
    git::merge_adds_nothing(worktree, target, head).await
        || git::all_picked_into(worktree, target, head).await
}

/// A remote branch other than the Base that `head` was merged into, if any. Only a merge commit
/// or the changes count there, not a fast-forward: a branch made on top of this one (a stacked
/// PR) has its commits on its own line too, and isn't where it was merged.
///
/// Several branches can have them: once a PR is merged into a project branch, every branch that
/// merges the project branch in has them too. Those got them through it, so the commit they
/// arrived with there has the project branch's arrival in its history: the branch where they
/// arrived first is where it was merged (by history, or failing that by time).
async fn merged_elsewhere(
    worktree: &Path,
    branch: Option<&str>,
    base_id: &str,
    head: &str,
) -> Option<MergeState> {
    let since = git::commit_time(worktree, head).await?;
    // Where it arrived first so far (with that branch's commit id), and the branch's name.
    let mut found: Option<(Arrived, String)> = None;
    let mut into = String::new();
    for target in git::remote_branches_since(worktree, since, OTHER_TARGETS).await {
        // Its own remote branch, and the Base under another name, aren't somewhere else.
        let own =
            branch.is_some_and(|b| target.short.split_once('/').map(|(_, rest)| rest) == Some(b));
        if own || target.id == base_id {
            continue;
        }
        let arrived = if git::is_merged(worktree, head, &target.id).await {
            if git::on_first_parent_line(worktree, head, &target.id).await {
                continue;
            }
            arrival(worktree, &target.id, head, Arrival::Commits).await
        } else if has_changes_of(worktree, &target.id, head).await {
            arrival(worktree, &target.id, head, Arrival::Changes).await
        } else {
            continue;
        };
        let Some(arrived) = arrived else {
            continue;
        };
        let earlier = match &found {
            None => true,
            // The same commit on both lines: one branch was made from the other after it arrived
            // (a feature branch off the project branch). The one PRs kept going into is where it
            // was merged.
            Some((first, first_id)) if arrived.commit == first.commit => {
                git::merges_since(worktree, &arrived.commit, &target.id).await
                    > git::merges_since(worktree, &first.commit, first_id).await
            }
            Some((first, _)) => {
                if git::is_merged(worktree, &arrived.commit, &first.commit).await {
                    true
                } else if git::is_merged(worktree, &first.commit, &arrived.commit).await {
                    false
                } else {
                    arrived.time < first.time
                }
            }
        };
        if earlier {
            found = Some((arrived, target.id.clone()));
            into = target.short;
        }
    }
    found.map(|(arrived, _)| match arrived.what {
        Arrival::Commits => MergeState::Merged { into },
        Arrival::Changes => MergeState::ChangesInBase { into },
    })
}

/// The commit on a branch's first-parent line that `head`'s work arrived with.
struct Arrived {
    commit: String,
    /// Its commit time (Unix seconds).
    time: i64,
    what: Arrival,
}

/// What arrived on a branch: `head`'s commits, or (squashed or rebased) its changes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Arrival {
    Commits,
    Changes,
}

/// Where `head`'s commits (or changes) arrived on `target`: the first commit on its first-parent
/// line that has them (found by halving: once there, they stay). `target` has them at its tip.
async fn arrival(worktree: &Path, target: &str, head: &str, what: Arrival) -> Option<Arrived> {
    let since = match what {
        Arrival::Commits => head.to_owned(),
        Arrival::Changes => git::merge_base(worktree, target).await.ok()?,
    };
    let mut line = git::first_parent_line(worktree, target, &since).await;
    let has = |commit: &str| {
        let commit = commit.to_owned();
        async move {
            match what {
                Arrival::Commits => git::is_merged(worktree, head, &commit).await,
                Arrival::Changes => has_changes_of(worktree, &commit, head).await,
            }
        }
    };
    // `line` is oldest first; the tip (last) has them.
    let (mut lo, mut hi) = (0, line.len().checked_sub(1)?);
    while lo < hi {
        let mid = (lo + hi) / 2;
        match has(&line[mid].0).await {
            true => hi = mid,
            false => lo = mid + 1,
        }
    }
    let (commit, time) = line.swap_remove(lo);
    Some(Arrived { commit, time, what })
}
