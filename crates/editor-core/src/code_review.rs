//! Code review (issue 44): going through a PR the user reviews in a Review Worktree, a Worktree of
//! its own detached at the PR's fetched head, read-only throughout. The diff is always the PR's
//! (its head against the merge base with its base branch), never the Worktree's files.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::git;
use crate::git_status::ChangeKind;
use crate::pull_request::DiffSide;
use crate::session::{DiffLine, DiffLineKind};
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
    /// The comments not submitted yet, and the lines Claude was asked about.
    pub pending: PendingReview,
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

/// The user's comments on the PR's lines, kept by the core until the review is submitted, and the
/// lines they asked Claude about.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingReview {
    pub comments: Vec<PendingComment>,
    pub questions: Vec<Question>,
}

impl PendingReview {
    pub(crate) fn is_empty(&self) -> bool {
        self.comments.is_empty() && self.questions.is_empty()
    }
}

/// A comment on a line (`line`) or a range (`start_line` to `line`) of one side of a file's diff.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingComment {
    pub id: u64,
    pub path: String,
    pub side: DiffSide,
    pub start_line: Option<u32>,
    pub line: u32,
    /// Markdown.
    pub body: String,
    /// The text of `line` when the comment was made, to find the line again after new commits.
    pub line_text: String,
    /// Its line is gone from the diff since (new commits): it must be moved or removed to submit.
    #[serde(default)]
    pub outdated: bool,
    /// It started as an answer of Claude's to a question about these lines.
    #[serde(default)]
    pub from_question: bool,
}

/// A comment to add to the Pending review.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewReviewComment {
    pub path: String,
    pub side: DiffSide,
    #[serde(default)]
    pub start_line: Option<u32>,
    pub line: u32,
    pub body: String,
    #[serde(default)]
    pub from_question: bool,
}

/// Lines Claude was asked about ("Ask Claude"): their gutter marker, which leads to the question
/// in the review session by its attachment's name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Question {
    pub path: String,
    pub side: DiffSide,
    #[serde(default)]
    pub start_line: Option<u32>,
    pub line: u32,
    pub attachment: String,
}

/// The text of line `line` on `side` of a diff, if the diff shows it (as a change or as context).
pub(crate) fn line_text(diff: &[DiffLine], side: DiffSide, line: u32) -> Option<String> {
    numbered(diff)
        .find(|(old, new, _)| match side {
            DiffSide::Left => *old == Some(line),
            DiffSide::Right => *new == Some(line),
        })
        .map(|(_, _, text)| text.to_owned())
}

/// The text of the line a comment on `side` ends on, if the diff shows it and the range's first
/// line too; None if the comment can't be anchored there.
pub(crate) fn anchor(
    diff: &[DiffLine],
    side: DiffSide,
    start_line: Option<u32>,
    line: u32,
) -> Option<String> {
    if let Some(start) = start_line {
        if start > line || line_text(diff, side, start).is_none() {
            return None;
        }
    }
    line_text(diff, side, line)
}

/// Each line a diff shows with its line number on the old side and on the new side (a removed line
/// has no new one, an added line no old one), read from its hunk headers.
fn numbered(diff: &[DiffLine]) -> impl Iterator<Item = (Option<u32>, Option<u32>, &str)> {
    let (mut old, mut new) = (0u32, 0u32);
    diff.iter().filter_map(move |l| match l.kind {
        DiffLineKind::Hunk => {
            if let Some((o, n)) = hunk_starts(&l.text) {
                (old, new) = (o, n);
            }
            None
        }
        DiffLineKind::Context => {
            let at = (Some(old), Some(new), l.text.as_str());
            (old, new) = (old + 1, new + 1);
            Some(at)
        }
        DiffLineKind::Removed => {
            old += 1;
            Some((Some(old - 1), None, l.text.as_str()))
        }
        DiffLineKind::Added => {
            new += 1;
            Some((None, Some(new - 1), l.text.as_str()))
        }
    })
}

/// The first old and new line numbers of a `@@ -a,b +c,d @@` hunk header.
fn hunk_starts(header: &str) -> Option<(u32, u32)> {
    let mut parts = header.strip_prefix("@@ -")?.split(' ');
    let old: u32 = parts.next()?.split(',').next()?.parse().ok()?;
    let new: u32 = parts
        .next()?
        .strip_prefix('+')?
        .split(',')
        .next()?
        .parse()
        .ok()?;
    // (An empty side, `-0,0`, starts at 0: its first line would be 1.)
    Some((old.max(1), new.max(1)))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn diff(old: &str, new: &str) -> Vec<DiffLine> {
        crate::diff::unified_diff(old, new, 1000)
    }

    #[test]
    fn a_comment_off_the_diff_is_refused() {
        let old: String = (1..=20).map(|n| format!("line {n}\n")).collect();
        let new = old.replace("line 10\n", "line ten\n");
        let diff = diff(&old, &new);
        assert_eq!(
            anchor(&diff, DiffSide::Right, None, 10).as_deref(),
            Some("line ten")
        );
        assert_eq!(
            anchor(&diff, DiffSide::Left, None, 10).as_deref(),
            Some("line 10")
        );
        // Context around the change is in the diff too.
        assert_eq!(
            anchor(&diff, DiffSide::Right, Some(8), 12).as_deref(),
            Some("line 12")
        );
        assert_eq!(
            anchor(&diff, DiffSide::Right, None, 2),
            None,
            "outside the hunk"
        );
        assert_eq!(
            anchor(&diff, DiffSide::Right, Some(2), 10),
            None,
            "range starts outside"
        );
        assert_eq!(
            anchor(&diff, DiffSide::Right, Some(11), 10),
            None,
            "backwards"
        );
    }

    #[test]
    fn a_new_files_lines_count_from_one() {
        let diff = diff("", "a\nb\n");
        assert_eq!(line_text(&diff, DiffSide::Right, 1).as_deref(), Some("a"));
        assert_eq!(line_text(&diff, DiffSide::Right, 2).as_deref(), Some("b"));
        assert_eq!(line_text(&diff, DiffSide::Left, 1), None);
    }
}
