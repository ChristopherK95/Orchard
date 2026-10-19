//! Code review (issue 44): going through a PR the user reviews in a Review Worktree, a Worktree of
//! its own detached at the PR's fetched head, read-only throughout. The diff is always the PR's
//! (its head against the merge base with its base branch), never the Worktree's files.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::attachments::Attachment;
use crate::git;
use crate::git_status::ChangeKind;
use crate::pull_request::{DiffSide, PullRequestComment, PullRequestDetails};
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

/// How far from where it was a pending comment's line is looked for after new commits.
const REANCHOR_REACH: u32 = 20;

/// Where a pending comment's line is in `new_diff` (after new commits): the line on its side with
/// the same text within `REANCHOR_REACH` lines of where it was, the nearest if there are several;
/// None if there's none (the comment is outdated).
pub(crate) fn reanchor(old: &PendingComment, new_diff: &[DiffLine]) -> Option<u32> {
    numbered(new_diff)
        .filter_map(|(on_old, on_new, text)| {
            let at = match old.side {
                DiffSide::Left => on_old,
                DiffSide::Right => on_new,
            }?;
            (text == old.line_text).then_some(at)
        })
        .filter(|at| at.abs_diff(old.line) <= REANCHOR_REACH)
        .min_by_key(|at| at.abs_diff(old.line))
}

/// Where a pending comment goes in `new_diff`, its range (`start_line`, `line`) moved as its line
/// did; None if its line, or its range's first line, isn't in the diff any more.
pub(crate) fn moved(old: &PendingComment, new_diff: &[DiffLine]) -> Option<(Option<u32>, u32)> {
    let line = reanchor(old, new_diff)?;
    let start = match old.start_line {
        Some(start) => {
            Some(u32::try_from(i64::from(start) + i64::from(line) - i64::from(old.line)).ok()?)
        }
        None => None,
    };
    anchor(new_diff, old.side, start, line)?;
    Some((start, line))
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

/// What "Claude review" asks of the review Agent session: the PR, its commits and files, and to
/// read the whole diff itself and answer in prose.
pub(crate) fn review_prompt(
    details: &PullRequestDetails,
    merge_base: &str,
    head: &str,
    files: &[CodeReviewFile],
) -> String {
    let description = match details.body.trim() {
        "" => "(none)",
        body => body,
    };
    let files: String = files
        .iter()
        .map(|f| {
            let change = match f.change {
                ChangeKind::Added => "added",
                ChangeKind::Modified => "modified",
                ChangeKind::Deleted => "deleted",
                ChangeKind::Renamed => "renamed",
            };
            let from = match &f.renamed_from {
                Some(from) => format!(" from `{from}`"),
                None => String::new(),
            };
            format!(
                "- `{}` ({change}{from}, +{} -{})\n",
                f.path, f.additions, f.deletions
            )
        })
        .collect();
    format!(
        "Review pull request #{number}, \"{title}\", which goes into `{base}`.\n\n\
         Its description:\n{description}\n\n\
         This checkout is the PR's head, `{head}`. It split from `{base}` at `{merge_base}`. \
         Read the whole change with `git diff {merge_base} {head}` (and the files here for \
         context), and review all of it.\n\n\
         The files it changes:\n{files}\n\
         Answer in prose: what the PR does, then anything wrong, risky or unclear, each with its \
         file and line. Don't change any files: this checkout is read-only.",
        number = details.number,
        title = details.title,
        base = details.base,
    )
}

/// How many lines around the lines asked about go with them.
const ASK_CONTEXT: u32 = 5;

/// What "Ask Claude" attaches: the lines `start_line` to `line` of `text` (the file on `side`),
/// with `ASK_CONTEXT` lines around them, numbered and the asked-about ones marked.
pub(crate) fn ask_attachment(
    path: &str,
    side: DiffSide,
    start_line: Option<u32>,
    line: u32,
    text: &str,
    file: Option<PathBuf>,
) -> Attachment {
    let start = start_line.unwrap_or(line).min(line);
    let from = start.saturating_sub(ASK_CONTEXT).max(1);
    let to = line.saturating_add(ASK_CONTEXT);
    let width = to.to_string().len();
    let lines: String = (1..)
        .zip(text.lines())
        .filter(|(n, _)| (from..=to).contains(n))
        .map(|(n, text)| {
            let mark = if (start..=line).contains(&n) {
                '>'
            } else {
                ' '
            };
            format!("{mark} {n:>width$} {text}\n")
        })
        .collect();
    let which = match side {
        DiffSide::Left => "old",
        DiffSide::Right => "new",
    };
    Attachment::Text {
        name: format!("{path}:{start}-{line} ({which})"),
        path: file,
        text: lines,
    }
}

/// Whether PR `number` was merged or closed (`gh pr view --json state`); false if that can't be
/// told.
pub(crate) async fn pull_request_over(gh: &Path, repo: &Path, number: u64) -> bool {
    let number = number.to_string();
    let args = ["pr", "view", &number, "--json", "state"];
    let Ok(printed) = crate::pull_request::gh_text(gh, repo, &args).await else {
        return false;
    };
    serde_json::from_str::<serde_json::Value>(printed.trim())
        .is_ok_and(|view| matches!(view["state"].as_str(), Some("MERGED" | "CLOSED")))
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

    fn pending_on(line: u32, line_text: &str) -> PendingComment {
        PendingComment {
            id: 1,
            path: "f".into(),
            side: DiffSide::Right,
            start_line: None,
            line,
            body: "?".into(),
            line_text: line_text.into(),
            outdated: false,
            from_question: false,
        }
    }

    #[test]
    fn a_comment_follows_its_line() {
        let old: String = (1..=30).map(|n| format!("line {n}\n")).collect();
        let new = old.replace("line 4\n", "line 4\nnew a\nnew b\n");
        let diff = diff("", &new);
        assert_eq!(reanchor(&pending_on(10, "line 10"), &diff), Some(12));
        let range = PendingComment {
            start_line: Some(9),
            ..pending_on(10, "line 10")
        };
        assert_eq!(moved(&range, &diff), Some((Some(11), 12)));
    }

    #[test]
    fn a_comment_whose_line_went_is_outdated() {
        let old: String = (1..=30).map(|n| format!("line {n}\n")).collect();
        let new = old.replace("line 10\n", "");
        assert_eq!(reanchor(&pending_on(10, "line 10"), &diff("", &new)), None);
        // Nor is a line of the same text far away taken for it.
        let far: String = (1..=40).map(|n| format!("line {n}\n")).collect();
        let far = format!("{}line 10\n", far.replace("line 10\n", ""));
        assert_eq!(reanchor(&pending_on(10, "line 10"), &diff("", &far)), None);
    }

    #[test]
    fn of_lines_alike_the_nearest_is_taken() {
        let lines = ["}", "a", "}", "b", "c", "}", "d"];
        let new: String = lines.iter().map(|l| format!("{l}\n")).collect();
        let diff = diff("", &new);
        assert_eq!(reanchor(&pending_on(5, "}"), &diff), Some(6));
        assert_eq!(reanchor(&pending_on(2, "}"), &diff), Some(1));
    }

    #[test]
    fn gits_diff_is_read_into_lines() {
        let out = "diff --git a/f b/f\nindex 1..2 100644\n--- a/f\n+++ b/f\n@@ -1,2 +1,2 @@ fn f() {\n a\n-b\n+c\n\\ No newline at end of file\n";
        let lines = crate::diff::parse_git_diff(out, 100);
        let kinds: Vec<(DiffLineKind, &str)> =
            lines.iter().map(|l| (l.kind, l.text.as_str())).collect();
        assert_eq!(
            kinds,
            [
                (DiffLineKind::Hunk, "@@ -1,2 +1,2 @@ fn f() {"),
                (DiffLineKind::Context, "a"),
                (DiffLineKind::Removed, "b"),
                (DiffLineKind::Added, "c"),
            ]
        );
        assert_eq!(line_text(&lines, DiffSide::Right, 2).as_deref(), Some("c"));
    }

    #[test]
    fn review_prompt_names_the_commits_and_files() {
        let view = r#"{"id":"PR_kw7","number":7,"title":"Make it better","body":"Why","url":"https://github.com/o/r/pull/7","state":"OPEN","createdAt":"2026-01-01T00:00:00Z","updatedAt":"2026-01-01T00:00:00Z","headRefName":"feat","baseRefName":"main","headRefOid":"head456","baseRefOid":""}"#;
        let (details, _) = crate::pull_request::viewed(view).unwrap();
        let files = [CodeReviewFile {
            path: "src/a.rs".into(),
            change: ChangeKind::Renamed,
            renamed_from: Some("src/old.rs".into()),
            additions: 2,
            deletions: 1,
            viewed: false,
        }];
        let prompt = review_prompt(&details, "base123", "head456", &files);
        for needed in [
            "#7",
            "\"Make it better\"",
            "Why",
            "`main`",
            "`git diff base123 head456`",
            "- `src/a.rs` (renamed from `src/old.rs`, +2 -1)",
            "read-only",
        ] {
            assert!(prompt.contains(needed), "{needed} not in {prompt}");
        }
    }

    #[test]
    fn ask_attachment_has_the_lines_and_their_context() {
        let text: String = (1..=30).map(|n| format!("line {n}\n")).collect();
        let Attachment::Text { name, text, path } =
            ask_attachment("f.rs", DiffSide::Right, Some(10), 11, &text, None)
        else {
            panic!("not text");
        };
        assert_eq!(name, "f.rs:10-11 (new)");
        assert_eq!(path, None);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.first(), Some(&"   5 line 5"));
        assert_eq!(lines[5], "> 10 line 10");
        assert_eq!(lines[6], "> 11 line 11");
        assert_eq!(lines.last(), Some(&"  16 line 16"));
        // Near the top, and on the old side.
        let Attachment::Text { name, text, .. } =
            ask_attachment("f.rs", DiffSide::Left, None, 2, "a\nb\nc\n", None)
        else {
            panic!("not text");
        };
        assert_eq!(name, "f.rs:2-2 (old)");
        assert_eq!(text, "  1 a\n> 2 b\n  3 c\n");
    }

    #[test]
    fn a_new_files_lines_count_from_one() {
        let diff = diff("", "a\nb\n");
        assert_eq!(line_text(&diff, DiffSide::Right, 1).as_deref(), Some("a"));
        assert_eq!(line_text(&diff, DiffSide::Right, 2).as_deref(), Some("b"));
        assert_eq!(line_text(&diff, DiffSide::Left, 1), None);
    }
}
