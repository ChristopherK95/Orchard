//! The Dashboard (issue 43): what it shows that the core works out — the Worktrees that need
//! attention, and why, and the reviews requested from the user — and the prompts that start an
//! Agent session on a PR with failing checks or review feedback.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::git_status::{GitOperation, GitStatus};
use crate::pull_request::{latest_checks, Check, Status, SubmittedReview};

/// A Worktree of the Workspace (the main checkout too) that needs attention, with every reason.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeAttention {
    pub path: PathBuf,
    /// None when HEAD is detached.
    pub branch: Option<String>,
    pub reasons: Vec<AttentionReason>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AttentionReason {
    /// Changed files not committed (untracked ones too).
    Uncommitted { files: u32 },
    /// Commits its upstream hasn't.
    Unpushed { commits: u32 },
    /// Commits its upstream has that it hasn't.
    Behind { commits: u32 },
    /// A branch with no upstream, and commits since its Base.
    NoUpstream { commits: u32 },
    /// A merge, rebase, cherry-pick, revert or `git am` in progress.
    Operation { operation: GitOperation },
}

/// Why a Worktree with this status needs attention (none: it doesn't). `since_base` is how many
/// commits its branch has made since its Base, which counts only when it has no upstream.
pub(crate) fn reasons(status: &GitStatus, since_base: u32) -> Vec<AttentionReason> {
    let mut reasons = vec![];
    if !status.files.is_empty() {
        reasons.push(AttentionReason::Uncommitted {
            files: status.files.len() as u32,
        });
    }
    match status.ahead {
        Some(commits) if commits > 0 => reasons.push(AttentionReason::Unpushed { commits }),
        _ => {}
    }
    match status.behind {
        Some(commits) if commits > 0 => reasons.push(AttentionReason::Behind { commits }),
        _ => {}
    }
    if status.branch.is_some() && status.upstream.is_none() && since_base > 0 {
        reasons.push(AttentionReason::NoUpstream {
            commits: since_base,
        });
    }
    if let Some(operation) = status.operation {
        reasons.push(AttentionReason::Operation { operation });
    }
    reasons
}

/// An open, non-draft PR asking the user (not a team of theirs) for a review.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewRequest {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub author: String,
    /// The branch it comes from.
    pub head: String,
    /// Its latest commit's checks.
    pub checks: Vec<Check>,
    /// When the user was last asked (None if GitHub doesn't say).
    pub requested_at: Option<String>,
    /// They've reviewed it before and are asked again.
    pub re_requested: bool,
}

/// What `gh api graphql` is asked for the review requests: `$q` is the search, `REVIEW_SEARCH`.
pub(crate) const REVIEW_QUERY: &str = "query($q: String!) {
  viewer { login }
  search(query: $q, type: ISSUE, first: 50) {
    nodes {
      ... on PullRequest {
        number title url author { login } headRefName
        latestReviews(first: 50) { nodes { author { login } } }
        timelineItems(itemTypes: REVIEW_REQUESTED_EVENT, last: 20) {
          nodes { ... on ReviewRequestedEvent { createdAt requestedReviewer { ... on User { login } } } }
        }
        commits(last: 1) { nodes { commit { statusCheckRollup { contexts(first: 100) { nodes {
          __typename
          ... on CheckRun { name conclusion status detailsUrl startedAt checkSuite { workflowRun { workflow { name } } } }
          ... on StatusContext { context state targetUrl createdAt }
        } } } } } }
      }
    }
  }
}";

/// The search for them (`gh` fills in the Worktree's repo): asked of the user directly, so not
/// through a team.
pub(crate) const REVIEW_SEARCH: &str =
    "repo:{owner}/{repo} is:pr is:open draft:false user-review-requested:@me";

/// A GraphQL connection's `nodes`.
#[derive(Deserialize)]
pub(crate) struct Nodes<T> {
    #[serde(default = "Vec::new")]
    pub(crate) nodes: Vec<T>,
}

impl<T> Default for Nodes<T> {
    fn default() -> Self {
        Self { nodes: Vec::new() }
    }
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub(crate) struct Login {
    pub(crate) login: Option<String>,
}

/// Reads `REVIEW_QUERY`'s answer: the review requests, the most recently asked first.
pub(crate) fn review_requests_listed(stdout: &str) -> Result<Vec<ReviewRequest>, String> {
    #[derive(Deserialize)]
    struct Answer {
        data: Data,
    }
    #[derive(Deserialize)]
    struct Data {
        viewer: Login,
        search: Nodes<Pr>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Pr {
        number: u64,
        title: String,
        url: String,
        #[serde(default)]
        author: Option<Login>,
        head_ref_name: String,
        #[serde(default)]
        latest_reviews: Nodes<Reviewed>,
        #[serde(default)]
        timeline_items: Nodes<Requested>,
        #[serde(default)]
        commits: Nodes<CommitNode>,
    }
    #[derive(Deserialize)]
    struct Reviewed {
        author: Option<Login>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Requested {
        created_at: Option<String>,
        requested_reviewer: Option<Login>,
    }
    #[derive(Deserialize)]
    struct CommitNode {
        commit: Commit,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Commit {
        status_check_rollup: Option<Rollup>,
    }
    #[derive(Deserialize)]
    struct Rollup {
        contexts: Nodes<Context>,
    }
    let answer: Answer = serde_json::from_str(stdout.trim())
        .map_err(|e| format!("gh said something unexpected: {e}"))?;
    let viewer = answer.data.viewer.login.unwrap_or_default();
    let is_viewer = |who: Option<Login>| who.and_then(|w| w.login).as_deref() == Some(&viewer);
    let mut requests: Vec<ReviewRequest> = answer
        .data
        .search
        .nodes
        .into_iter()
        .map(|pr| {
            let requested_at = pr
                .timeline_items
                .nodes
                .into_iter()
                .filter_map(|event| {
                    let at = event.created_at?;
                    is_viewer(event.requested_reviewer).then_some(at)
                })
                .max();
            let statuses = pr
                .commits
                .nodes
                .into_iter()
                .filter_map(|c| c.commit.status_check_rollup)
                .flat_map(|rollup| rollup.contexts.nodes)
                .map(Context::status)
                .collect();
            ReviewRequest {
                number: pr.number,
                title: pr.title,
                url: pr.url,
                author: pr
                    .author
                    .and_then(|a| a.login)
                    .unwrap_or_else(|| "ghost".into()),
                head: pr.head_ref_name,
                checks: latest_checks(statuses),
                requested_at,
                re_requested: pr
                    .latest_reviews
                    .nodes
                    .into_iter()
                    .any(|r| is_viewer(r.author)),
            }
        })
        .collect();
    requests.sort_by(|a, b| b.requested_at.cmp(&a.requested_at));
    Ok(requests)
}

/// Reads `gh pr view --json title,statusCheckRollup`: the PR's title and its latest checks.
pub(crate) fn titled_checks(stdout: &str) -> Result<(String, Vec<Check>), String> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Viewed {
        #[serde(default)]
        title: String,
        #[serde(default)]
        status_check_rollup: Vec<Status>,
    }
    let pr: Viewed = serde_json::from_str(stdout.trim())
        .map_err(|e| format!("gh said something unexpected: {e}"))?;
    Ok((pr.title, latest_checks(pr.status_check_rollup)))
}

/// How many of a failed job's last log lines go into the prompt.
const LOG_TAIL: usize = 200;

/// The prompt that starts an Agent session on a PR's failing checks: each failed check by name,
/// with the last `LOG_TAIL` lines of its failed job's log (`None`: it has none, as for a check
/// that isn't a GitHub Actions job, which is named with its link instead; `Err`: it couldn't be
/// read, as while its run is still going or once it's expired, named with its link and why).
pub(crate) fn failing_check_text(
    number: u64,
    title: &str,
    failed: &[(Check, Result<Option<String>, String>)],
) -> String {
    let mut text = format!(
        "The checks of PR #{number} ({title}) are failing. Find the cause of each failure below \
         and fix it.\n"
    );
    for (check, log) in failed {
        let name = match &check.workflow {
            Some(workflow) => format!("{workflow} / {}", check.name),
            None => check.name.clone(),
        };
        text.push_str(&format!("\n## {name}\n"));
        match log {
            Ok(Some(log)) => {
                let lines: Vec<&str> = log.lines().collect();
                let tail = &lines[lines.len().saturating_sub(LOG_TAIL)..];
                text.push_str(&format!(
                    "\nThe last {} lines of its failed job's log:\n\n",
                    tail.len()
                ));
                fenced(&mut text, &tail.join("\n"));
            }
            Ok(None) | Err(_) => {
                if let Some(url) = &check.url {
                    text.push_str(&format!("\n{url}\n"));
                }
                if let Err(why) = log {
                    let why = why.lines().next().unwrap_or_default();
                    text.push_str(&format!("\nIts log couldn't be read: {why}\n"));
                }
            }
        }
    }
    text
}

/// A review thread on a PR's code that isn't resolved: where it is, and what's been said, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReviewThread {
    pub(crate) path: String,
    /// The line it's on now, else (outdated) the one it was made on.
    pub(crate) line: Option<u64>,
    /// Who said what.
    pub(crate) comments: Vec<(String, String)>,
}

/// What `gh api graphql` is asked for a PR's review threads (`$owner`, `$name`, `$number`).
pub(crate) const THREAD_QUERY: &str = "query($owner: String!, $name: String!, $number: Int!) {
  repository(owner: $owner, name: $name) {
    pullRequest(number: $number) {
      reviewThreads(first: 100) {
        nodes { isResolved path line originalLine comments(first: 50) { nodes { author { login } body } } }
      }
    }
  }
}";

/// Reads `THREAD_QUERY`'s answer: the threads not resolved.
pub(crate) fn unresolved_threads(stdout: &str) -> Result<Vec<ReviewThread>, String> {
    #[derive(Deserialize)]
    struct Answer {
        data: Data,
    }
    #[derive(Deserialize)]
    struct Data {
        repository: Repository,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Repository {
        pull_request: PullRequest,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct PullRequest {
        review_threads: Nodes<Thread>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Thread {
        is_resolved: bool,
        path: String,
        line: Option<u64>,
        original_line: Option<u64>,
        #[serde(default)]
        comments: Nodes<Comment>,
    }
    #[derive(Deserialize)]
    struct Comment {
        author: Option<Login>,
        #[serde(default)]
        body: String,
    }
    let answer: Answer = serde_json::from_str(stdout.trim())
        .map_err(|e| format!("gh said something unexpected: {e}"))?;
    Ok(answer
        .data
        .repository
        .pull_request
        .review_threads
        .nodes
        .into_iter()
        .filter(|t| !t.is_resolved)
        .map(|t| ReviewThread {
            path: t.path,
            line: t.line.or(t.original_line),
            comments: t
                .comments
                .nodes
                .into_iter()
                .map(|c| {
                    let author = c.author.and_then(|a| a.login);
                    (author.unwrap_or_else(|| "ghost".into()), c.body)
                })
                .collect(),
        })
        .collect())
}

/// Reads `gh pr view --json title,reviews`: the PR's title and every review submitted.
pub(crate) fn titled_reviews(stdout: &str) -> Result<(String, Vec<SubmittedReview>), String> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Viewed {
        #[serde(default)]
        title: String,
        #[serde(default)]
        reviews: Vec<Reviewed>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Reviewed {
        author: Option<Login>,
        state: String,
        #[serde(default)]
        body: String,
        #[serde(default)]
        submitted_at: Option<String>,
    }
    let pr: Viewed = serde_json::from_str(stdout.trim())
        .map_err(|e| format!("gh said something unexpected: {e}"))?;
    let reviews = pr
        .reviews
        .into_iter()
        .map(|r| SubmittedReview {
            author: r
                .author
                .and_then(|a| a.login)
                .unwrap_or_else(|| "ghost".into()),
            state: r.state,
            body: r.body,
            submitted_at: r.submitted_at.unwrap_or_default(),
        })
        .collect();
    Ok((pr.title, reviews))
}

/// The prompt that starts an Agent session on a PR's review feedback: the body of each reviewer's
/// latest review that requests changes (a later approval or dismissal answers it; a comment
/// doesn't), then each unresolved thread on its code with its file and line.
pub(crate) fn review_feedback_text(
    number: u64,
    title: &str,
    reviews: &[SubmittedReview],
    threads: &[ReviewThread],
) -> String {
    let mut latest: Vec<&SubmittedReview> = vec![];
    let mut in_order: Vec<&SubmittedReview> = reviews
        .iter()
        .filter(|r| {
            matches!(
                r.state.as_str(),
                "APPROVED" | "CHANGES_REQUESTED" | "DISMISSED"
            )
        })
        .collect();
    in_order.sort_by(|a, b| a.submitted_at.cmp(&b.submitted_at));
    for review in in_order {
        match latest.iter_mut().find(|r| r.author == review.author) {
            Some(earlier) => *earlier = review,
            None => latest.push(review),
        }
    }
    let mut text = format!(
        "PR #{number} ({title}) has review feedback. Address each point below: make the change \
         asked for, or say why not.\n"
    );
    for review in latest {
        if review.state != "CHANGES_REQUESTED" || review.body.trim().is_empty() {
            continue;
        }
        text.push_str(&format!(
            "\n## {} requested changes\n\n{}\n",
            review.author,
            review.body.trim()
        ));
    }
    for thread in threads {
        let place = match thread.line {
            Some(line) => format!("{}:{line}", thread.path),
            None => thread.path.clone(),
        };
        text.push_str(&format!("\n## {place}\n"));
        for (author, body) in &thread.comments {
            text.push_str(&format!("\n{author}:\n{}\n", body.trim()));
        }
    }
    text
}

/// `body` in a fenced block, its fence longer than any run of backticks in it.
fn fenced(text: &mut String, body: &str) {
    let longest = body.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    let fence = "`".repeat(longest.max(2) + 1);
    text.push_str(&format!("{fence}\n{body}\n{fence}\n"));
}

/// A check run or commit status as GraphQL gives it (`statusCheckRollup.contexts`).
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Context {
    #[serde(rename = "__typename", default)]
    typename: String,
    name: Option<String>,
    context: Option<String>,
    status: Option<String>,
    conclusion: Option<String>,
    state: Option<String>,
    details_url: Option<String>,
    target_url: Option<String>,
    started_at: Option<String>,
    created_at: Option<String>,
    check_suite: Option<CheckSuite>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CheckSuite {
    /// None when it isn't GitHub Actions.
    workflow_run: Option<WorkflowRun>,
}

#[derive(Deserialize)]
struct WorkflowRun {
    workflow: Workflow,
}

#[derive(Deserialize)]
struct Workflow {
    name: String,
}

impl Context {
    /// As `gh --json statusCheckRollup` would have it, so it's read the same way.
    fn status(self) -> Status {
        Status {
            typename: self.typename,
            name: self.name,
            context: self.context,
            workflow_name: self
                .check_suite
                .and_then(|s| s.workflow_run)
                .map(|r| r.workflow.name),
            status: self.status,
            conclusion: self.conclusion,
            state: self.state,
            details_url: self.details_url,
            target_url: self.target_url,
            started_at: self.started_at,
            created_at: self.created_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pull_request::CheckOutcome;

    #[test]
    fn review_requests_say_when_the_user_was_asked_and_if_again() {
        let out = r#"{"data":{"viewer":{"login":"me"},"search":{"nodes":[
            {"number":4,"title":"Older ask","url":"u4","author":{"login":"ann"},"headRefName":"feat/a",
             "latestReviews":{"nodes":[{"author":{"login":"bob"}}]},
             "timelineItems":{"nodes":[
                {"createdAt":"2026-10-01T09:00:00Z","requestedReviewer":{}},
                {"createdAt":"2026-10-02T09:00:00Z","requestedReviewer":{"login":"me"}}]},
             "commits":{"nodes":[{"commit":{"statusCheckRollup":null}}]}},
            {"number":7,"title":"Asked again","url":"u7","author":{"login":"bob"},"headRefName":"feat/b",
             "latestReviews":{"nodes":[{"author":{"login":"me"}}]},
             "timelineItems":{"nodes":[
                {"createdAt":"2026-10-03T09:00:00Z","requestedReviewer":{"login":"me"}},
                {"createdAt":"2026-10-05T09:00:00Z","requestedReviewer":{"login":"me"}},
                {"createdAt":"2026-10-06T09:00:00Z","requestedReviewer":{"login":"cy"}}]},
             "commits":{"nodes":[{"commit":{"statusCheckRollup":{"contexts":{"nodes":[
                {"__typename":"CheckRun","name":"test","conclusion":"FAILURE","status":"COMPLETED",
                 "detailsUrl":"https://github.com/o/r/actions/runs/1/job/2","startedAt":"2026-10-05T10:00:00Z",
                 "checkSuite":{"workflowRun":{"workflow":{"name":"CI"}}}},
                {"__typename":"CheckRun","name":"external","conclusion":null,"status":"IN_PROGRESS",
                 "detailsUrl":"https://ci.example.com/9","startedAt":"2026-10-05T10:00:00Z",
                 "checkSuite":{"workflowRun":null}},
                {"__typename":"StatusContext","context":"deploy","state":"PENDING",
                 "targetUrl":"https://deploy.example.com","createdAt":"2026-10-05T10:00:00Z"}]}}}}]}}
        ]}}}"#;
        let requests = review_requests_listed(out).unwrap();
        let seen: Vec<_> = requests
            .iter()
            .map(|r| {
                (
                    r.number,
                    r.author.as_str(),
                    r.requested_at.as_deref(),
                    r.re_requested,
                )
            })
            .collect();
        assert_eq!(
            seen,
            [
                (7, "bob", Some("2026-10-05T09:00:00Z"), true),
                (4, "ann", Some("2026-10-02T09:00:00Z"), false),
            ]
        );
        assert_eq!(requests[0].head, "feat/b");
        let checks: Vec<_> = requests[0]
            .checks
            .iter()
            .map(|c| (c.workflow.as_deref(), c.name.as_str(), c.outcome))
            .collect();
        assert_eq!(
            checks,
            [
                (Some("CI"), "test", CheckOutcome::Failure),
                (None, "external", CheckOutcome::Pending),
                (None, "deploy", CheckOutcome::Pending),
            ]
        );
        assert!(requests[0].checks[0].job.is_some());
        assert!(requests[1].checks.is_empty());
    }

    #[test]
    fn a_failing_checks_prompt_has_its_name_and_the_logs_last_lines() {
        let check = |name: &str, job| Check {
            name: name.into(),
            workflow: Some("CI".into()),
            outcome: CheckOutcome::Failure,
            url: Some(format!("https://example.com/{name}")),
            job,
        };
        let log: String = (1..=250).map(|n| format!("line {n}\n")).collect();
        let job = Some(crate::pull_request::ActionsJob { run: 1, job: 2 });
        let prompt = failing_check_text(
            12,
            "Fix the parser",
            &[
                (check("test", job), Ok(Some(log))),
                (check("external", None), Ok(None)),
            ],
        );
        assert!(prompt.contains("#12"), "{prompt}");
        assert!(prompt.contains("Fix the parser"), "{prompt}");
        assert!(prompt.contains("CI / test"), "{prompt}");
        assert!(prompt.contains("line 51\n"), "{prompt}");
        assert!(prompt.contains("line 250\n"), "{prompt}");
        assert!(!prompt.contains("line 50\n"), "{prompt}");
        assert!(prompt.contains("CI / external"), "{prompt}");
        assert!(prompt.contains("https://example.com/external"), "{prompt}");
    }

    #[test]
    fn feedback_prompt_has_change_requests_and_unresolved_threads_only() {
        let review = |author: &str, state: &str, body: &str, at: &str| SubmittedReview {
            author: author.into(),
            state: state.into(),
            body: body.into(),
            submitted_at: at.into(),
        };
        let reviews = [
            review(
                "ann",
                "CHANGES_REQUESTED",
                "Split the parser up.",
                "2026-10-01T00:00:00Z",
            ),
            review(
                "bob",
                "APPROVED",
                "Looks fine to me.",
                "2026-10-02T00:00:00Z",
            ),
            review(
                "cy",
                "CHANGES_REQUESTED",
                "Rename everything.",
                "2026-10-03T00:00:00Z",
            ),
            review(
                "ann",
                "COMMENTED",
                "One more thing.",
                "2026-10-04T00:00:00Z",
            ),
            review("cy", "APPROVED", "Fine now.", "2026-10-05T00:00:00Z"),
        ];
        let threads = unresolved_threads(
            r#"{"data":{"repository":{"pullRequest":{"reviewThreads":{"nodes":[
                {"isResolved":true,"path":"src/old.rs","line":3,"originalLine":3,
                 "comments":{"nodes":[{"author":{"login":"bob"},"body":"Done with this."}]}},
                {"isResolved":false,"path":"src/parse.rs","line":null,"originalLine":42,
                 "comments":{"nodes":[{"author":{"login":"ann"},"body":"Why a loop here?"},
                                      {"author":{"login":"me"},"body":"It was quicker."}]}}
            ]}}}}}"#,
        )
        .unwrap();
        let prompt = review_feedback_text(12, "Fix the parser", &reviews, &threads);
        assert!(prompt.contains("#12"), "{prompt}");
        assert!(
            prompt.contains("ann") && prompt.contains("Split the parser up."),
            "{prompt}"
        );
        assert!(!prompt.contains("Looks fine"), "{prompt}");
        assert!(!prompt.contains("Rename everything"), "{prompt}");
        assert!(!prompt.contains("Done with this"), "{prompt}");
        assert!(!prompt.contains("src/old.rs"), "{prompt}");
        assert!(prompt.contains("src/parse.rs:42"), "{prompt}");
        let asked = prompt
            .find("Why a loop here?")
            .expect("the thread's first comment");
        let answered = prompt.find("It was quicker.").expect("its reply");
        assert!(asked < answered, "{prompt}");
    }
}
