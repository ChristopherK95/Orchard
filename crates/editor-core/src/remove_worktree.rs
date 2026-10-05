//! Removing a Worktree (ticket 09) without losing work: what removal would discard is listed first,
//! and only an explicit "Discard and remove" of exactly that lets it go. The main checkout is never
//! removable.
//!
//! The rule: uncommitted changes are always lost with the folder; commits only when no branch keeps
//! them (HEAD is detached, or the branch is deleted too), unless the Base has their changes anyway
//! (a squash or rebase merge). Commits are listed either way.

use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::git;
use crate::merged::MergeState;
use crate::session::SessionId;

/// How many changed files, ignored entries or commits the check lists (it counts them all).
const LIST_LIMIT: usize = 100;

/// A commit, as the removal check lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitSummary {
    /// The short commit id.
    pub id: String,
    pub subject: String,
}

/// What the user chose in the removal dialog.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveWorktree {
    /// "Discard and remove": the `fingerprint` of the check the user saw. If the work changed
    /// since (say, an Agent kept editing), removal is refused rather than losing unseen work.
    pub discard: Option<String>,
    /// "Delete branch too".
    pub delete_branch: bool,
}

/// A finished removal, and anything the user should know about it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemovedWorktree {
    /// E.g. the Worktree is gone but its branch couldn't be deleted.
    pub warning: Option<String>,
}

/// What removing a Worktree would stop and lose, for the confirmation dialog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemovalCheck {
    pub worktree: PathBuf,
    /// `None` when HEAD is detached.
    pub branch: Option<String>,
    /// The Agent sessions that removal stops first.
    pub sessions: Vec<SessionId>,
    /// Uncommitted changes as `XY path` lines (`??` = untracked), the first `LIST_LIMIT` of them.
    pub changes: Vec<String>,
    pub changed_count: usize,
    /// Ignored files and folders (`node_modules/`, `.env`), deleted with the Worktree: shown so
    /// nothing goes unannounced, though they don't need "Discard and remove".
    pub ignored: Vec<String>,
    pub ignored_count: usize,
    /// Commits only this Worktree has: not pushed, not merged into the Base, and on no other branch
    /// or tag (the first `LIST_LIMIT`, newest first).
    pub unpushed: Vec<CommitSummary>,
    pub unpushed_count: usize,
    /// What `merged` and `unpushed` are measured against: the Worktree's Base.
    pub base: String,
    /// The branch is merged into the Base (its commits, or for a squash or rebase merge its
    /// changes), or has nothing of its own: "Delete branch too" starts ticked, and deleting it
    /// loses nothing.
    pub merged: bool,
    /// Where it was merged: the Base, or another branch its PR went into (`origin/project`).
    /// `None` when it isn't, or has nothing of its own.
    pub merged_into: Option<String>,
    /// Removing the Worktree loses work (uncommitted changes, or commits on a detached HEAD).
    pub discard_to_remove: bool,
    /// Deleting the branch as well loses its unpushed commits.
    pub discard_to_delete_branch: bool,
    /// Identifies exactly the work listed here; "Discard and remove" sends it back.
    pub fingerprint: String,
}

/// The facts a check is made from.
pub(crate) struct Facts<'a> {
    pub(crate) worktree: &'a Path,
    pub(crate) branch: Option<&'a str>,
    pub(crate) changes: Vec<String>,
    pub(crate) ignored: Vec<String>,
    pub(crate) unpushed: (Vec<CommitSummary>, usize),
    pub(crate) base: &'a str,
    pub(crate) merged: bool,
    pub(crate) merged_into: Option<String>,
    pub(crate) fingerprint: String,
}

impl RemovalCheck {
    /// The one place the discard rule lives.
    pub(crate) fn from_facts(facts: Facts) -> Self {
        let changed_count = facts.changes.len();
        let ignored_count = facts.ignored.len();
        let (unpushed, unpushed_count) = facts.unpushed;
        Self {
            worktree: facts.worktree.to_owned(),
            branch: facts.branch.map(str::to_owned),
            sessions: vec![],
            changes: facts.changes.into_iter().take(LIST_LIMIT).collect(),
            changed_count,
            ignored: facts.ignored.into_iter().take(LIST_LIMIT).collect(),
            ignored_count,
            discard_to_remove: changed_count > 0 || (facts.branch.is_none() && unpushed_count > 0),
            discard_to_delete_branch: unpushed_count > 0 && !facts.merged,
            unpushed,
            unpushed_count,
            base: facts.base.to_owned(),
            merged: facts.merged,
            merged_into: facts.merged_into,
            fingerprint: facts.fingerprint,
        }
    }

    /// What `options` would lose that the user didn't agree to discard, if anything.
    pub(crate) fn would_lose(&self, options: &RemoveWorktree) -> Option<String> {
        if !(self.discard_to_remove || options.delete_branch && self.discard_to_delete_branch) {
            return None;
        }
        let describe = || {
            let mut lost = vec![];
            if self.changed_count > 0 {
                lost.push(plural(self.changed_count, "uncommitted change"));
            }
            let no_branch_keeps_them = self.branch.is_none() || options.delete_branch;
            if no_branch_keeps_them && self.unpushed_count > 0 && !self.merged {
                lost.push(format!(
                    "{} that no remote, branch or tag has",
                    plural(self.unpushed_count, "commit")
                ));
            }
            lost.join(" and ")
        };
        match &options.discard {
            Some(seen) if *seen == self.fingerprint => None,
            Some(_) => Some(format!("{}, which changed after you looked", describe())),
            None => Some(describe()),
        }
    }
}

fn plural(n: usize, thing: &str) -> String {
    if n == 1 {
        format!("1 {thing}")
    } else {
        format!("{n} {thing}s")
    }
}

/// Checks a (linked) Worktree's state against `base` (shown by name, measured by its commit id);
/// `sessions` is filled in by the caller.
pub(crate) async fn inspect(
    worktree: &Path,
    branch: Option<&str>,
    base: &str,
    base_id: &str,
) -> Result<RemovalCheck, String> {
    let changes = git::changes(worktree).await?;
    let ignored = git::ignored(worktree).await?;
    let diff = git::diff_vs_head(worktree).await?;
    let unpushed = git::commits_only_here(worktree, branch, base_id, LIST_LIMIT).await?;
    let state = match branch {
        Some(_) => Some(crate::merged::detect(worktree, branch, base, base_id).await),
        None => None,
    };
    let merged = state.as_ref().is_some_and(MergeState::in_base);
    let merged_into = match state {
        Some(MergeState::Merged { into } | MergeState::ChangesInBase { into }) => Some(into),
        _ => None,
    };
    let fingerprint = fingerprint(worktree, &changes, &diff, &unpushed);
    Ok(RemovalCheck::from_facts(Facts {
        worktree,
        branch,
        changes,
        ignored,
        unpushed,
        base,
        merged,
        merged_into,
        fingerprint,
    }))
}

/// Changes when any listed work does: tracked edits (the diff), untracked files (size and time),
/// and the commits only this Worktree has. Compared within one run of the editor only.
fn fingerprint(
    worktree: &Path,
    changes: &[String],
    diff: &str,
    (unpushed, unpushed_count): &(Vec<CommitSummary>, usize),
) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    diff.hash(&mut hasher);
    for change in changes {
        change.hash(&mut hasher);
        if let Some(untracked) = change.strip_prefix("?? ") {
            if let Ok(meta) = std::fs::metadata(worktree.join(untracked)) {
                meta.len().hash(&mut hasher);
                meta.modified().ok().hash(&mut hasher);
            }
        }
    }
    unpushed_count.hash(&mut hasher);
    for commit in unpushed {
        commit.id.hash(&mut hasher);
    }
    format!("{:016x}", hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(changed: usize, unpushed: usize, branch: Option<&str>) -> Facts<'_> {
        Facts {
            worktree: Path::new("w"),
            branch,
            changes: vec![" M a".into(); changed],
            ignored: vec![],
            unpushed: (vec![], unpushed),
            base: "origin/main",
            merged: false,
            merged_into: None,
            fingerprint: "seen".into(),
        }
    }

    fn check(changed: usize, unpushed: usize, branch: Option<&str>) -> RemovalCheck {
        RemovalCheck::from_facts(facts(changed, unpushed, branch))
    }

    fn options(discard: Option<&str>, delete_branch: bool) -> RemoveWorktree {
        RemoveWorktree {
            discard: discard.map(str::to_owned),
            delete_branch,
        }
    }

    #[test]
    fn commits_are_only_lost_with_the_branch_or_a_detached_head() {
        assert_eq!(
            check(0, 2, Some("b")).would_lose(&options(None, false)),
            None
        );
        assert_eq!(
            check(0, 2, Some("b"))
                .would_lose(&options(None, true))
                .as_deref(),
            Some("2 commits that no remote, branch or tag has")
        );
        assert_eq!(
            check(0, 1, None)
                .would_lose(&options(None, false))
                .as_deref(),
            Some("1 commit that no remote, branch or tag has")
        );
    }

    #[test]
    fn changes_are_always_lost() {
        assert_eq!(
            check(1, 0, Some("b"))
                .would_lose(&options(None, false))
                .as_deref(),
            Some("1 uncommitted change")
        );
        assert_eq!(
            check(2, 3, None)
                .would_lose(&options(None, false))
                .as_deref(),
            Some("2 uncommitted changes and 3 commits that no remote, branch or tag has")
        );
    }

    #[test]
    fn commits_whose_changes_the_base_has_are_not_lost_with_the_branch() {
        let squashed = RemovalCheck::from_facts(Facts {
            merged: true,
            ..facts(1, 2, Some("b"))
        });
        assert!(!squashed.discard_to_delete_branch);
        assert_eq!(
            squashed.would_lose(&options(None, true)).as_deref(),
            Some("1 uncommitted change"),
            "only the changes"
        );
    }

    #[test]
    fn discard_covers_only_the_work_that_was_shown() {
        assert_eq!(
            check(3, 3, None).would_lose(&options(Some("seen"), true)),
            None
        );
        assert!(check(3, 3, None)
            .would_lose(&options(Some("older"), true))
            .is_some_and(|lost| lost.contains("changed after you looked")));
    }
}
