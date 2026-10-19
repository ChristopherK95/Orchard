//! Code review (issue 44): going through a PR the user reviews in a Review Worktree, a Worktree of
//! its own detached at the PR's fetched head, read-only throughout. The diff is always the PR's
//! (its head against the merge base with its base branch), never the Worktree's files.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::git;
use crate::git_status::ChangeKind;
use crate::pull_request::{DiffSide, PullRequestComment};
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
    /// Ticked Viewed on GitHub by the user.
    pub viewed: bool,
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

/// What a submitted review says of the PR.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Verdict {
    Approve,
    RequestChanges,
    Comment,
}

impl Verdict {
    /// GitHub's review `event`.
    fn event(self) -> &'static str {
        match self {
            Verdict::Approve => "APPROVE",
            Verdict::RequestChanges => "REQUEST_CHANGES",
            Verdict::Comment => "COMMENT",
        }
    }
}

/// The body of the one `POST …/pulls/<N>/reviews` that submits the review on `head`.
pub(crate) fn submission(
    head: &str,
    verdict: Verdict,
    summary: &str,
    comments: &[PendingComment],
) -> serde_json::Value {
    let comments: Vec<serde_json::Value> = comments
        .iter()
        .map(|c| {
            let mut comment = serde_json::json!({
                "path": c.path,
                "side": c.side.github(),
                "line": c.line,
                "body": c.body,
            });
            if let Some(start) = c.start_line.filter(|start| *start != c.line) {
                comment["start_line"] = start.into();
                comment["start_side"] = c.side.github().into();
            }
            comment
        })
        .collect();
    serde_json::json!({
        "commit_id": head,
        "event": verdict.event(),
        "body": summary,
        "comments": comments,
    })
}

/// What GitHub said when it turned a submission down: its message and its errors (a 422's
/// `errors[]`, strings or objects with a `message`), and which of `paths` they name.
pub(crate) fn submit_failure(
    printed: &str,
    failure: &str,
    paths: &[&str],
) -> (String, Vec<String>) {
    let mut said: Vec<String> = vec![];
    if let Ok(body) = serde_json::from_str::<serde_json::Value>(printed.trim()) {
        if let Some(message) = body["message"].as_str() {
            said.push(message.to_owned());
        }
        for error in body["errors"].as_array().into_iter().flatten() {
            match error.as_str().or_else(|| error["message"].as_str()) {
                Some(text) => said.push(text.to_owned()),
                None => said.push(error.to_string()),
            }
        }
    }
    let message = match said.is_empty() {
        true => failure.trim().to_owned(),
        false => said.join("\n"),
    };
    let mut named: Vec<String> = vec![];
    for path in paths {
        if message.contains(path) && !named.iter().any(|p| p == path) {
            named.push((*path).to_owned());
        }
    }
    (message, named)
}

/// The comment GitHub made of a reply (what `POST …/comments/<id>/replies` prints).
pub(crate) fn posted_reply(printed: &str) -> Result<PullRequestComment, String> {
    #[derive(Deserialize)]
    struct User {
        login: String,
    }
    #[derive(Deserialize)]
    struct Posted {
        id: u64,
        #[serde(default)]
        body: String,
        created_at: String,
        html_url: Option<String>,
        user: Option<User>,
    }
    let posted: Posted = serde_json::from_str(printed.trim())
        .map_err(|e| format!("gh said something unexpected: {e}"))?;
    Ok(PullRequestComment {
        id: Some(posted.id),
        author: posted.user.map_or_else(|| "ghost".into(), |u| u.login),
        body: posted.body,
        created_at: posted.created_at,
        url: posted.html_url,
        edited: false,
        minimized: false,
    })
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
                viewed: false,
            }
        })
        .collect())
}

/// The files of a PR (by its GraphQL node id) and whether the user ticked them Viewed, a page of
/// 100 at a time.
const VIEWED_QUERY: &str = "query($id: ID!, $after: String) { node(id: $id) { ... on PullRequest { files(first: 100, after: $after) { nodes { path viewerViewedState } pageInfo { hasNextPage endCursor } } } } }";

/// The paths of the PR's files the user has ticked Viewed on GitHub.
pub(crate) async fn viewed_files(
    gh: &Path,
    repo: &Path,
    pull_request_id: &str,
) -> Result<HashSet<String>, String> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct File {
        path: String,
        viewer_viewed_state: String,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct PageInfo {
        has_next_page: bool,
        end_cursor: Option<String>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Files {
        nodes: Vec<File>,
        page_info: PageInfo,
    }
    let mut viewed = HashSet::new();
    let mut after: Option<String> = None;
    loop {
        let query = format!("query={VIEWED_QUERY}");
        let id = format!("id={pull_request_id}");
        let cursor = after.as_ref().map(|after| format!("after={after}"));
        let mut args = vec!["api", "graphql", "-f", &query, "-f", &id];
        if let Some(cursor) = &cursor {
            args.extend(["-f", cursor]);
        }
        let printed = crate::pull_request::gh_text(gh, repo, &args).await?;
        let answer: serde_json::Value = serde_json::from_str(printed.trim())
            .map_err(|e| format!("gh said something unexpected: {e}"))?;
        let files: Files = serde_json::from_value(answer["data"]["node"]["files"].clone())
            .map_err(|e| format!("gh said something unexpected: {e}"))?;
        viewed.extend(
            files
                .nodes
                .into_iter()
                .filter(|f| f.viewer_viewed_state == "VIEWED")
                .map(|f| f.path),
        );
        match files.page_info {
            PageInfo {
                has_next_page: true,
                end_cursor: Some(cursor),
            } => after = Some(cursor),
            _ => return Ok(viewed),
        }
    }
}

/// Ticks (or unticks) the PR's file `path` Viewed on GitHub.
pub(crate) async fn set_viewed(
    gh: &Path,
    repo: &Path,
    pull_request_id: &str,
    path: &str,
    viewed: bool,
) -> Result<(), String> {
    let mutation = match viewed {
        true => "markFileAsViewed",
        false => "unmarkFileAsViewed",
    };
    let query = format!(
        "query=mutation($id: ID!, $path: String!) {{ {mutation}(input: {{pullRequestId: $id, path: $path}}) {{ clientMutationId }} }}"
    );
    let id = format!("id={pull_request_id}");
    let path = format!("path={path}");
    let args = ["api", "graphql", "-f", &query, "-f", &id, "-f", &path];
    crate::pull_request::gh_text(gh, repo, &args)
        .await
        .map(drop)
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
    fn a_failed_submit_names_the_files_githubs_errors_name() {
        let printed = r#"{"message":"Validation Failed","errors":[{"message":"pull_request_review_thread.line must be part of the diff","path":"x"},"src/b.rs is outdated"]}"#;
        let (message, paths) = submit_failure(
            printed,
            "gh: HTTP 422",
            &["src/a.rs", "src/b.rs", "src/b.rs"],
        );
        assert!(message.starts_with("Validation Failed\n"), "{message}");
        assert!(message.contains("must be part of the diff"), "{message}");
        assert_eq!(paths, ["src/b.rs"]);
        // A network failure prints no body: gh's own words then.
        let (message, paths) =
            submit_failure("", "error connecting to api.github.com", &["src/a.rs"]);
        assert_eq!(message, "error connecting to api.github.com");
        assert!(paths.is_empty());
    }

    #[test]
    fn a_new_files_lines_count_from_one() {
        let diff = diff("", "a\nb\n");
        assert_eq!(line_text(&diff, DiffSide::Right, 1).as_deref(), Some("a"));
        assert_eq!(line_text(&diff, DiffSide::Right, 2).as_deref(), Some("b"));
        assert_eq!(line_text(&diff, DiffSide::Left, 1), None);
    }
}
