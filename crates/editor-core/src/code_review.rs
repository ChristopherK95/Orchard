//! Code review (issue 44): going through a PR the user reviews in a Review Worktree, a Worktree of
//! its own detached at the PR's fetched head, read-only throughout. The diff is always the PR's
//! (its head against the merge base with its base branch), never the Worktree's files.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::git;
use crate::git_status::ChangeKind;
use crate::setup::SetupInfo;

/// A Code review, as its overlay shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeReview {
    pub number: u64,
    pub title: String,
    /// The Review Worktree.
    pub worktree: PathBuf,
    /// The PR's head commit, as fetched (what the Review Worktree is at).
    pub head: String,
    /// The branch the PR goes into, and where the PR's head split from it.
    pub base: String,
    pub merge_base: String,
    pub files: Vec<CodeReviewFile>,
    /// What changed since the user's last review, when they've reviewed it before.
    pub since_last_review: Option<SinceLastReview>,
    /// The Review Worktree's setup, while it runs (or after it failed) in this run of the editor.
    pub setup: Option<SetupInfo>,
}

/// A file the PR changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeReviewFile {
    /// `/`-separated, relative to the repository.
    pub path: String,
    pub change: ChangeKind,
    /// A rename's original path.
    pub renamed_from: Option<String>,
    pub additions: u32,
    pub deletions: u32,
}

/// The files changed since the user last reviewed the PR (a re-requested review).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SinceLastReview {
    /// The commit their last review was on.
    pub commit: String,
    pub files: Vec<CodeReviewFile>,
}

/// Where GitHub keeps PR `number`'s head, which a fork's PR has too.
pub(crate) fn pull_head_ref(number: u64) -> String {
    format!("refs/pull/{number}/head")
}

/// Where the fetched head is kept, so it outlives `git gc` and a force-push of the PR.
pub(crate) fn review_ref(number: u64) -> String {
    format!("refs/orchard/review/{number}")
}

/// Where the commit of the user's last review is kept, if it had to be fetched.
pub(crate) fn last_review_ref(number: u64) -> String {
    format!("refs/orchard/review/{number}-last")
}

/// The Review Worktree's folder name (made free with `-2`, … if taken).
pub(crate) fn folder_name(number: u64) -> String {
    format!("review-pr-{number}")
}

/// The files the PR changes from `merge_base` to `head`, with their added and deleted lines.
pub(crate) async fn files(
    repo: &Path,
    merge_base: &str,
    head: &str,
) -> Result<Vec<CodeReviewFile>, String> {
    let changes = git::changes_between(repo, merge_base, head).await?;
    let counts = git::numstat(repo, merge_base, head).await?;
    Ok(changes
        .into_iter()
        .map(|(letter, path, renamed_from)| {
            let (additions, deletions) = counts.get(&path).copied().unwrap_or_default();
            CodeReviewFile {
                change: ChangeKind::from_letter(letter),
                path,
                renamed_from,
                additions,
                deletions,
            }
        })
        .collect())
}
