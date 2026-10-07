//! Review and pull request (ticket 42): everything the Worktree's branch would bring into its PR,
//! read from where it split from its Base to the files on disk (committed or not, untracked files
//! too), and the PR made from it: whatever isn't committed yet is committed, the branch is pushed,
//! and the GitHub CLI opens the PR with the user as its assignee.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::git_status::ChangeKind;

/// What "Review" shows: every file the PR would change, and what the PR step starts from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Review {
    /// None when HEAD is detached (there's no branch to make a PR from).
    pub branch: Option<String>,
    /// The Worktree's Base, and where the branch split from it (each file's diff runs from there).
    pub base: String,
    pub split: String,
    pub files: Vec<ReviewFile>,
    /// The branch's commits since the split, oldest first.
    pub commits: Vec<ReviewCommit>,
    /// How many files have changes that aren't committed yet (committed on "Create PR").
    pub uncommitted: u32,
    /// A merge, rebase or the like is in progress: no PR until it's finished or aborted.
    pub operation_in_progress: bool,
    /// The remote the branch is pushed to, and its branches (without `<remote>/`), to target.
    pub remote: String,
    pub targets: Vec<String>,
    /// The branch the PR goes into unless another is chosen: the Base's, else the default one.
    pub target: String,
}

/// One of the branch's commits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewCommit {
    pub id: String,
    pub short_id: String,
    pub subject: String,
}

/// What one commit changed on its own: its files, and the parent their diffs run from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitChanges {
    pub parent: String,
    pub files: Vec<ReviewFile>,
}

/// A file the PR would change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewFile {
    /// Relative to the Worktree, `/`-separated.
    pub path: String,
    pub change: ChangeKind,
    /// A rename's original path.
    pub renamed_from: Option<String>,
    /// It has changes that aren't committed yet (some of them, at least).
    pub uncommitted: bool,
}

/// "Create PR", as asked.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestRequest {
    /// The branch it goes into (without `<remote>/`).
    pub target: String,
    pub title: String,
    #[serde(default)]
    pub body: String,
    /// What to commit the uncommitted changes with, if there are any.
    #[serde(default)]
    pub commit_message: String,
    #[serde(default)]
    pub draft: bool,
    /// Go ahead while a session in the Worktree is Working.
    #[serde(default)]
    pub even_if_working: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PullRequestOutcome {
    /// Sessions in the Worktree are Working (they may still be changing files): ask first. Nothing
    /// was done.
    SessionsWorking { sessions: Vec<String> },
    /// The PR at `url` was opened; `committed` is the commit made of the uncommitted changes first.
    Created {
        url: String,
        committed: Option<String>,
    },
    /// The branch already had an open PR (at `url`): the push updated it.
    AlreadyOpen {
        url: String,
        committed: Option<String>,
    },
    /// The remote has commits the branch hasn't, so it turned the push down (anything committed
    /// first stays committed).
    PushRejected { committed: Option<String> },
}

/// The PR already open from the Worktree's branch: "Review" then pushes to it instead.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenPullRequest {
    pub number: u64,
    pub url: String,
    /// The branch it goes into.
    pub target: String,
}

/// What "Push" (to a branch whose PR is open) did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PushToPullRequestOutcome {
    /// Sessions in the Worktree are Working: ask first. Nothing was done.
    SessionsWorking { sessions: Vec<String> },
    /// Pushed to `to`; `committed` is the commit made of the uncommitted changes first.
    Pushed {
        to: String,
        committed: Option<String>,
    },
    /// The remote has commits the branch hasn't (anything committed first stays committed).
    Rejected { committed: Option<String> },
}

/// How "Create PR" is getting on, as it goes: a step starting, or output from what it runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PullRequestProgress {
    Step { text: String },
    Output { text: String },
}

/// `<remote>/<branch>` as the branch on the remote, if it's one of `remote`'s.
pub(crate) fn on_remote<'a>(name: &'a str, remote: &str) -> Option<&'a str> {
    name.strip_prefix(remote)?.strip_prefix('/')
}

/// How long `gh` may take to open a PR before it's stopped.
const GH_TIMEOUT: Duration = Duration::from_secs(120);

/// What to tell the user when `gh` couldn't run or isn't logged in.
const GH_HINT: &str =
    "Creating a PR takes the GitHub CLI: install `gh`, run `gh auth login` in a terminal once, then try again.";

/// Signs, in `gh`'s words, that it isn't logged in.
const GH_LOGIN_SIGNS: &[&str] = &["gh auth login", "authentication", "not logged", "401"];

/// What `gh pr create` did.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Opened {
    Created(String),
    AlreadyOpen(String),
}

/// Opens a PR from `branch` into `target` with `gh pr create` (in `worktree`, so for its repo),
/// assigned to whoever `gh` is logged in as. Never prompts. Its output goes to `sink` as it comes.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn create(
    gh: &Path,
    worktree: &Path,
    branch: &str,
    target: &str,
    title: &str,
    body: &str,
    draft: bool,
    sink: crate::process::Sink<'_>,
) -> Result<Opened, String> {
    let mut args = vec!["pr", "create", "--head", branch, "--base", target];
    args.extend(["--title", title, "--body", body, "--assignee", "@me"]);
    args.extend(draft.then_some("--draft"));
    let out = run_gh(gh, worktree, &args, Some(sink)).await?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    opened(out.status.success(), &stdout, &stderr)
}

/// The open PR from `branch`, if it has one (`gh pr list --head`).
pub(crate) async fn open_for(
    gh: &Path,
    worktree: &Path,
    branch: &str,
) -> Result<Option<OpenPullRequest>, String> {
    let args = [
        "pr",
        "list",
        "--head",
        branch,
        "--state",
        "open",
        "--json",
        "number,url,baseRefName",
        "--limit",
        "1",
    ];
    let out = run_gh(gh, worktree, &args, None).await?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    if !out.status.success() {
        return Err(gh_failure(&stdout, &String::from_utf8_lossy(&out.stderr)));
    }
    listed(&stdout)
}

/// Reads `gh pr list --json number,url,baseRefName`: its first PR.
fn listed(stdout: &str) -> Result<Option<OpenPullRequest>, String> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Listed {
        number: u64,
        url: String,
        base_ref_name: String,
    }
    let prs: Vec<Listed> = serde_json::from_str(stdout.trim())
        .map_err(|e| format!("gh said something unexpected: {e}"))?;
    Ok(prs.into_iter().next().map(|pr| OpenPullRequest {
        number: pr.number,
        url: pr.url,
        target: pr.base_ref_name,
    }))
}

/// One of the user's PRs in the repo, as the sidebar and the PR board show it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MyPullRequest {
    pub number: u64,
    pub title: String,
    pub url: String,
    /// The branch it comes from, and the one it goes into.
    pub head: String,
    pub base: String,
    pub state: PullRequestState,
    pub draft: bool,
    /// Where it stands: its column on the PR board.
    pub stage: PullRequestStage,
    /// `APPROVED`, `CHANGES_REQUESTED` or `REVIEW_REQUIRED`; None when no review is needed.
    pub review_decision: Option<String>,
    pub reviewers: Vec<Reviewer>,
    pub checks: Vec<Check>,
    /// It can't be merged as it is: it conflicts with the branch it goes into.
    pub conflicts: bool,
    pub updated_at: String,
    pub merged_at: Option<String>,
    pub closed_at: Option<String>,
}

/// Where one of the user's PRs stands, as the PR board sorts them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PullRequestStage {
    Draft,
    /// Open, and no one has approved it or asked for changes yet.
    WaitingForReview,
    ChangesRequested,
    /// A check failed (and no one asked for changes).
    ChecksFailing,
    /// Approved, with no check failing.
    Approved,
    Merged,
    Closed,
}

/// The `gh pr list` fields `MyPullRequest` is read from.
const MINE_FIELDS: &str = "number,title,url,state,isDraft,headRefName,baseRefName,reviewDecision,latestReviews,reviewRequests,statusCheckRollup,mergeable,updatedAt,mergedAt,closedAt";

/// How many of the user's merged or closed PRs are listed (the latest).
const MINE_DONE: &str = "20";

/// The PRs whoever `gh` is logged in as opened in the repo (`gh pr list --author @me`): every open
/// one, then the latest merged or closed ones; each lot most recently updated first.
pub(crate) async fn mine(gh: &Path, worktree: &Path) -> Result<Vec<MyPullRequest>, String> {
    let list = |state, limit| {
        [
            "pr",
            "list",
            "--author",
            "@me",
            "--state",
            state,
            "--json",
            MINE_FIELDS,
            "--limit",
            limit,
        ]
    };
    let (open, done) = (list("open", "100"), list("closed", MINE_DONE));
    let (open, done) = tokio::join!(gh_text(gh, worktree, &open), gh_text(gh, worktree, &done));
    let mut prs = mine_listed(&open?)?;
    prs.extend(mine_listed(&done?)?);
    Ok(prs)
}

/// Reads `gh pr list --json MINE_FIELDS`, most recently updated first.
fn mine_listed(stdout: &str) -> Result<Vec<MyPullRequest>, String> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Listed {
        number: u64,
        title: String,
        url: String,
        state: String,
        #[serde(default)]
        is_draft: bool,
        head_ref_name: String,
        base_ref_name: String,
        #[serde(default)]
        review_decision: Option<String>,
        #[serde(default)]
        latest_reviews: Vec<GhReview>,
        #[serde(default)]
        review_requests: Vec<Actor>,
        #[serde(default)]
        status_check_rollup: Vec<Status>,
        #[serde(default)]
        mergeable: String,
        updated_at: String,
        merged_at: Option<String>,
        closed_at: Option<String>,
    }
    let listed: Vec<Listed> = serde_json::from_str(stdout.trim())
        .map_err(|e| format!("gh said something unexpected: {e}"))?;
    let mut prs: Vec<MyPullRequest> = listed
        .into_iter()
        .map(|pr| {
            let state = PullRequestState::read(&pr.state);
            let review_decision = pr.review_decision.filter(|d| !d.is_empty());
            let reviewers = reviewers_of(pr.latest_reviews, pr.review_requests);
            let checks = latest_checks(pr.status_check_rollup);
            let stage = stage_of(
                state,
                pr.is_draft,
                review_decision.as_deref(),
                &reviewers,
                &checks,
            );
            MyPullRequest {
                number: pr.number,
                title: pr.title,
                url: pr.url,
                head: pr.head_ref_name,
                base: pr.base_ref_name,
                state,
                draft: pr.is_draft,
                stage,
                review_decision,
                reviewers,
                checks,
                conflicts: pr.mergeable == "CONFLICTING",
                updated_at: pr.updated_at,
                merged_at: pr.merged_at.filter(|at| !at.is_empty()),
                closed_at: pr.closed_at.filter(|at| !at.is_empty()),
            }
        })
        .collect();
    prs.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(prs)
}

/// Where a PR stands. Changes are asked for while a reviewer's latest review asks for them and
/// they haven't been asked to review again (GitHub's decision stays "changes requested" until that
/// reviewer approves, however they've been answered). Asked-for changes come before failing checks
/// (someone's waiting on them). Approved is GitHub's decision; without one (no review required),
/// someone's approval.
fn stage_of(
    state: PullRequestState,
    draft: bool,
    decision: Option<&str>,
    reviewers: &[Reviewer],
    checks: &[Check],
) -> PullRequestStage {
    match state {
        PullRequestState::Merged => return PullRequestStage::Merged,
        PullRequestState::Closed => return PullRequestStage::Closed,
        PullRequestState::Open if draft => return PullRequestStage::Draft,
        PullRequestState::Open => {}
    }
    let changes = reviewers
        .iter()
        .any(|r| r.state == ReviewerState::ChangesRequested && !r.requested);
    let approved = match decision {
        Some(decision) => decision == "APPROVED",
        None => reviewers.iter().any(|r| r.state == ReviewerState::Approved),
    };
    if changes {
        PullRequestStage::ChangesRequested
    } else if checks.iter().any(|c| c.outcome == CheckOutcome::Failure) {
        PullRequestStage::ChecksFailing
    } else if approved {
        PullRequestStage::Approved
    } else {
        PullRequestStage::WaitingForReview
    }
}

/// The PR of the Worktree's branch as "PR" shows it: where it stands, who's on it, and what's been
/// said (the conversation and the comments on its code).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestDetails {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub state: PullRequestState,
    pub draft: bool,
    /// Markdown.
    pub body: String,
    pub author: String,
    pub created_at: String,
    pub updated_at: String,
    pub merged_at: Option<String>,
    pub merged_by: Option<String>,
    pub closed_at: Option<String>,
    /// The branch it comes from, and the one it goes into.
    pub head: String,
    pub base: String,
    pub additions: u64,
    pub deletions: u64,
    pub changed_files: u64,
    /// GitHub's word on its reviews: `APPROVED`, `CHANGES_REQUESTED`, `REVIEW_REQUIRED`, or none.
    pub review_decision: Option<String>,
    /// `MERGEABLE`, `CONFLICTING` or `UNKNOWN`.
    pub mergeable: String,
    /// `CLEAN`, `BLOCKED`, `BEHIND`, `DIRTY`, `UNSTABLE`, `DRAFT`, `HAS_HOOKS` or `UNKNOWN`.
    pub merge_state: String,
    pub labels: Vec<PullRequestLabel>,
    pub milestone: Option<String>,
    pub assignees: Vec<String>,
    /// Everyone asked to review it or who has, with where they stand.
    pub reviewers: Vec<Reviewer>,
    pub checks: Vec<Check>,
    /// The conversation: comments on the PR itself, oldest first.
    pub comments: Vec<PullRequestComment>,
    /// Every review submitted, oldest first.
    pub reviews: Vec<SubmittedReview>,
    /// The comments on its code, by thread, oldest first.
    pub threads: Vec<CodeThread>,
    /// Why the comments on its code couldn't be read (the rest still could).
    pub threads_error: Option<String>,
    /// The repo's organisation's top-level teams, each with everyone in it or its sub-teams, to
    /// filter comments by (none when the repo belongs to a user).
    pub teams: Vec<Team>,
    /// Why the teams couldn't be read (the rest still could).
    pub teams_error: Option<String>,
    pub commits: Vec<PullRequestCommit>,
}

/// A top-level team of the repo's organisation: its members and its sub-teams' members.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Team {
    pub name: String,
    pub slug: String,
    pub members: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PullRequestState {
    Open,
    Closed,
    Merged,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestLabel {
    pub name: String,
    /// Hex, without `#`.
    pub color: String,
}

/// Someone (or a team) on the PR's review.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Reviewer {
    pub name: String,
    pub team: bool,
    /// Their latest review, or `Requested` when they haven't reviewed yet.
    pub state: ReviewerState,
    /// Asked to review (again, when they already have: then their review is waited on anew).
    pub requested: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ReviewerState {
    Requested,
    Approved,
    ChangesRequested,
    Commented,
    Dismissed,
}

/// One CI check or commit status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Check {
    pub name: String,
    pub workflow: Option<String>,
    pub outcome: CheckOutcome,
    pub url: Option<String>,
    /// The GitHub Actions job it is (it can be re-run), if it's one.
    pub job: Option<ActionsJob>,
}

/// A GitHub Actions job: the workflow run it's in, and its id as its link has it (not always the
/// one `gh run rerun --job` takes: see `rerun`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionsJob {
    pub run: u64,
    pub job: u64,
}

impl ActionsJob {
    /// The job a check run's details link (`…/actions/runs/<run>/job/<job>`, or `jobs`) is to.
    fn from_url(url: &str) -> Option<Self> {
        let (_, rest) = url.split_once("/actions/runs/")?;
        let mut parts = rest.split(['/', '?', '#']);
        let run = parts.next()?.parse().ok()?;
        matches!(parts.next()?, "job" | "jobs").then_some(())?;
        let job = parts.next()?.parse().ok()?;
        Some(ActionsJob { run, job })
    }
}

/// Re-runs GitHub Actions jobs (`gh run rerun`): the job `job` names (its id, else its name), or
/// else every failed job of `run`. A job's link doesn't always carry the id `--job` takes, so the
/// run's jobs are asked for first and the job found among them.
pub(crate) async fn rerun(
    gh: &Path,
    worktree: &Path,
    run: u64,
    job: Option<(u64, String)>,
) -> Result<(), String> {
    let run = run.to_string();
    let Some((id, name)) = job else {
        return gh_text(gh, worktree, &["run", "rerun", &run, "--failed"])
            .await
            .map(drop);
    };
    let jq = ".jobs[] | [.databaseId, .name] | @tsv";
    let jobs = gh_text(
        gh,
        worktree,
        &["run", "view", &run, "--json", "jobs", "--jq", jq],
    )
    .await?;
    let id = job_id(&jobs, id, &name)
        .ok_or_else(|| format!("run {run} has no job \"{name}\" to re-run."))?;
    gh_text(gh, worktree, &["run", "rerun", "--job", &id.to_string()])
        .await
        .map(drop)
}

/// The job among a run's (`<databaseId>\t<name>` lines) whose id is `id`, else whose name is `name`.
fn job_id(jobs: &str, id: u64, name: &str) -> Option<u64> {
    let jobs: Vec<(u64, &str)> = jobs
        .lines()
        .filter_map(|line| {
            let (id, name) = line.split_once('\t')?;
            Some((id.trim().parse().ok()?, name))
        })
        .collect();
    jobs.iter()
        .find(|(j, _)| *j == id)
        .or_else(|| jobs.iter().find(|(_, n)| *n == name))
        .map(|(j, _)| *j)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CheckOutcome {
    Pending,
    Success,
    Failure,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestComment {
    pub author: String,
    pub body: String,
    pub created_at: String,
    pub url: Option<String>,
    pub edited: bool,
    /// Hidden on GitHub (as off-topic, outdated, …).
    pub minimized: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmittedReview {
    pub author: String,
    /// `APPROVED`, `CHANGES_REQUESTED`, `COMMENTED` or `DISMISSED`.
    pub state: String,
    pub body: String,
    pub submitted_at: String,
}

/// A comment on the PR's code and its replies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeThread {
    pub path: String,
    /// The line it's on now; None once the code it was on has changed (outdated).
    pub line: Option<u64>,
    /// The line it was made on.
    pub original_line: Option<u64>,
    pub url: String,
    /// The few lines of diff it's about, ending at its line.
    pub diff_hunk: String,
    pub comments: Vec<PullRequestComment>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestCommit {
    pub short_id: String,
    pub subject: String,
    pub author: String,
    pub date: String,
}

/// What `gh pr view` is asked for.
const DETAIL_FIELDS: &str = "number,title,url,state,isDraft,body,author,createdAt,updatedAt,mergedAt,mergedBy,closedAt,headRefName,baseRefName,additions,deletions,changedFiles,reviewDecision,mergeable,mergeStateStatus,labels,milestone,assignees,reviewRequests,latestReviews,reviews,statusCheckRollup,comments,commits";

/// The comments on the PR's code, one JSON object a line (`gh api --paginate` with this `--jq`).
const THREAD_JQ: &str = ".[] | {id, in_reply_to_id, path, line, original_line, html_url, diff_hunk, body, created_at, user: .user.login}";

/// The PR `which` names (`gh pr view`: a number, or a branch's open PR, else its latest), if there
/// is one, with the comments on its code (`gh api`).
pub(crate) async fn details_for(
    gh: &Path,
    worktree: &Path,
    which: &str,
) -> Result<Option<PullRequestDetails>, String> {
    let out = run_gh(
        gh,
        worktree,
        &["pr", "view", which, "--json", DETAIL_FIELDS],
        None,
    )
    .await?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        if stderr.contains("no pull requests found") {
            return Ok(None);
        }
        return Err(gh_failure(&stdout, &stderr));
    }
    let mut details = viewed(&stdout)?;
    let comments_of = format!("repos/{{owner}}/{{repo}}/pulls/{}/comments", details.number);
    let args = ["api", &comments_of, "--paginate", "--jq", THREAD_JQ];
    let (threads_read, teams_read) =
        tokio::join!(gh_text(gh, worktree, &args), teams_for(gh, worktree));
    match threads_read.and_then(|out| threads(&out)) {
        Ok(threads) => details.threads = threads,
        Err(e) => details.threads_error = Some(e),
    }
    match teams_read {
        Ok(teams) => details.teams = teams,
        Err(e) => details.teams_error = Some(e),
    }
    Ok(Some(details))
}

/// What `gh` printed, or why it failed.
async fn gh_text(gh: &Path, worktree: &Path, args: &[&str]) -> Result<String, String> {
    let out = run_gh(gh, worktree, args, None).await?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    match out.status.success() {
        true => Ok(stdout),
        false => Err(gh_failure(&stdout, &String::from_utf8_lossy(&out.stderr))),
    }
}

/// How long an organisation's teams are kept before they're read again.
const TEAMS_FRESH: Duration = Duration::from_secs(15 * 60);

/// The teams read lately, by organisation.
type TeamCache =
    std::sync::Mutex<std::collections::HashMap<String, (std::time::Instant, Vec<Team>)>>;

/// The top-level teams of the repo's organisation with their members (GitHub counts a sub-team's
/// members as its parent's too), or none when a user owns the repo. Kept for `TEAMS_FRESH`.
async fn teams_for(gh: &Path, worktree: &Path) -> Result<Vec<Team>, String> {
    static CACHE: std::sync::OnceLock<TeamCache> = std::sync::OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    let owner_jq = r#"[.owner.type, .owner.login] | join(" ")"#;
    let owner = gh_text(
        gh,
        worktree,
        &["api", "repos/{owner}/{repo}", "--jq", owner_jq],
    )
    .await?;
    let Some(org) = owner
        .trim()
        .strip_prefix("Organization ")
        .map(str::to_owned)
    else {
        return Ok(Vec::new());
    };
    if let Some((at, teams)) = cache.lock().unwrap().get(&org) {
        if at.elapsed() < TEAMS_FRESH {
            return Ok(teams.clone());
        }
    }
    let unreadable = |e: String| {
        format!("Couldn't read {org}'s teams (`gh auth refresh -s read:org` gives gh access): {e}")
    };
    let listed = gh_text(
        gh,
        worktree,
        &[
            "api",
            &format!("orgs/{org}/teams"),
            "--paginate",
            "--jq",
            TEAM_JQ,
        ],
    )
    .await
    .map_err(unreadable)?;
    // Every team's members at once (each is a request of its own).
    let mut reads = tokio::task::JoinSet::new();
    for (i, (name, slug)) in top_level(&listed)?.into_iter().enumerate() {
        let (gh, worktree) = (gh.to_owned(), worktree.to_owned());
        let members_of = format!("orgs/{org}/teams/{slug}/members");
        reads.spawn(async move {
            let args = ["api", &members_of, "--paginate", "--jq", ".[].login"];
            let members = gh_text(&gh, &worktree, &args).await;
            (i, name, slug, members)
        });
    }
    let mut teams = Vec::new();
    while let Some(read) = reads.join_next().await {
        let (i, name, slug, members) = read.map_err(|e| e.to_string())?;
        let mut members: Vec<String> = members
            .map_err(unreadable)?
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_owned)
            .collect();
        members.sort();
        members.dedup();
        teams.push((
            i,
            Team {
                name,
                slug,
                members,
            },
        ));
    }
    teams.sort_by_key(|(i, _)| *i);
    let teams: Vec<Team> = teams.into_iter().map(|(_, t)| t).collect();
    cache
        .lock()
        .unwrap()
        .insert(org, (std::time::Instant::now(), teams.clone()));
    Ok(teams)
}

/// The organisation's teams, one JSON object a line (`gh api --paginate` with this `--jq`).
const TEAM_JQ: &str = ".[] | {name, slug, parent: .parent.slug}";

/// The top-level teams (name and slug) among `TEAM_JQ`'s lines.
fn top_level(stdout: &str) -> Result<Vec<(String, String)>, String> {
    #[derive(Deserialize)]
    struct Line {
        name: String,
        slug: String,
        parent: Option<String>,
    }
    let mut teams = Vec::new();
    for line in stdout.lines().filter(|l| !l.trim().is_empty()) {
        let t: Line =
            serde_json::from_str(line).map_err(|e| format!("gh said something unexpected: {e}"))?;
        if t.parent.is_none() {
            teams.push((t.name, t.slug));
        }
    }
    teams.sort();
    Ok(teams)
}

/// Someone (or a team, or an app) as `gh --json` gives them.
#[derive(Deserialize, Default)]
#[serde(default)]
struct Actor {
    login: Option<String>,
    name: Option<String>,
    slug: Option<String>,
    #[serde(rename = "__typename")]
    typename: Option<String>,
}
impl Actor {
    fn login(self) -> String {
        self.login
            .or(self.slug)
            .or(self.name)
            .unwrap_or_else(|| "ghost".into())
    }
}

/// A submitted review, as `gh --json latestReviews,reviews` gives it.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhReview {
    #[serde(default)]
    author: Actor,
    state: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    submitted_at: Option<String>,
}

/// A check run or commit status, as `gh --json statusCheckRollup` gives it.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Status {
    #[serde(rename = "__typename", default)]
    typename: String,
    name: Option<String>,
    context: Option<String>,
    workflow_name: Option<String>,
    status: Option<String>,
    conclusion: Option<String>,
    state: Option<String>,
    details_url: Option<String>,
    target_url: Option<String>,
    /// When a check run started; a commit status, when it was set.
    started_at: Option<String>,
    created_at: Option<String>,
}

/// The checks of `statusCheckRollup`, each once: it lists every attempt of a check (a re-run job
/// is listed again, its earlier attempt still there), and only the latest of a check's attempts
/// (by workflow and name) says where it stands. Each is where its first attempt was listed.
fn latest_checks(statuses: Vec<Status>) -> Vec<Check> {
    let mut latest: Vec<Status> = Vec::new();
    for status in statuses {
        let key = |s: &Status| {
            let name = s.name.clone().or(s.context.clone());
            (s.typename.clone(), s.workflow_name.clone(), name)
        };
        match latest.iter_mut().find(|l| key(l) == key(&status)) {
            Some(earlier) if status.since() >= earlier.since() => *earlier = status,
            Some(_) => {}
            None => latest.push(status),
        }
    }
    latest.into_iter().map(Status::check).collect()
}

impl Status {
    /// When this attempt started (None, for one queued and not started yet, sorts first, so the
    /// listing's order decides between such).
    fn since(&self) -> Option<&str> {
        self.started_at
            .as_deref()
            .or(self.created_at.as_deref())
            .filter(|at| !at.is_empty() && !at.starts_with("0001-"))
    }

    fn check(self) -> Check {
        let outcome = if self.typename == "StatusContext" {
            match self.state.as_deref() {
                Some("SUCCESS") => CheckOutcome::Success,
                Some("FAILURE" | "ERROR") => CheckOutcome::Failure,
                _ => CheckOutcome::Pending,
            }
        } else if self.status.as_deref() != Some("COMPLETED") {
            CheckOutcome::Pending
        } else {
            match self.conclusion.as_deref() {
                Some("SUCCESS") => CheckOutcome::Success,
                Some("SKIPPED" | "NEUTRAL" | "STALE") => CheckOutcome::Skipped,
                _ => CheckOutcome::Failure,
            }
        };
        let job = match self.typename.as_str() {
            "CheckRun" => self.details_url.as_deref().and_then(ActionsJob::from_url),
            _ => None,
        };
        Check {
            name: self.name.or(self.context).unwrap_or_default(),
            workflow: self.workflow_name.filter(|w| !w.is_empty()),
            outcome,
            url: self
                .details_url
                .or(self.target_url)
                .filter(|u| !u.is_empty()),
            job,
        }
    }
}

impl PullRequestState {
    fn read(state: &str) -> Self {
        match state {
            "MERGED" => PullRequestState::Merged,
            "CLOSED" => PullRequestState::Closed,
            _ => PullRequestState::Open,
        }
    }
}

/// Who reviewed (their latest review), then who's asked to and hasn't, or is asked again.
fn reviewers_of(latest_reviews: Vec<GhReview>, review_requests: Vec<Actor>) -> Vec<Reviewer> {
    let mut reviewers: Vec<Reviewer> = Vec::new();
    for review in latest_reviews {
        let state = match review.state.as_str() {
            "APPROVED" => ReviewerState::Approved,
            "CHANGES_REQUESTED" => ReviewerState::ChangesRequested,
            "COMMENTED" => ReviewerState::Commented,
            "DISMISSED" => ReviewerState::Dismissed,
            _ => continue, // (PENDING: a review still being written)
        };
        let name = review.author.login();
        if !reviewers.iter().any(|r| r.name == name) {
            reviewers.push(Reviewer {
                name,
                team: false,
                state,
                requested: false,
            });
        }
    }
    for request in review_requests {
        let team = request.typename.as_deref() == Some("Team");
        let name = request.login();
        match reviewers.iter_mut().find(|r| r.name == name) {
            Some(r) => r.requested = true,
            None => reviewers.push(Reviewer {
                name,
                team,
                state: ReviewerState::Requested,
                requested: true,
            }),
        }
    }
    reviewers
}

/// Reads `gh pr view --json DETAIL_FIELDS` (without the comments on its code).
fn viewed(stdout: &str) -> Result<PullRequestDetails, String> {
    #[derive(Deserialize)]
    struct Label {
        name: String,
        #[serde(default)]
        color: String,
    }
    #[derive(Deserialize)]
    struct Milestone {
        title: String,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Comment {
        #[serde(default)]
        author: Actor,
        #[serde(default)]
        body: String,
        created_at: String,
        #[serde(default)]
        url: Option<String>,
        #[serde(default)]
        includes_created_edit: bool,
        #[serde(default)]
        is_minimized: bool,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct CommitAuthor {
        login: Option<String>,
        name: Option<String>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Commit {
        oid: String,
        message_headline: String,
        #[serde(default)]
        authors: Vec<CommitAuthor>,
        #[serde(default)]
        committed_date: String,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Viewed {
        number: u64,
        title: String,
        url: String,
        state: String,
        #[serde(default)]
        is_draft: bool,
        #[serde(default)]
        body: String,
        #[serde(default)]
        author: Actor,
        created_at: String,
        updated_at: String,
        merged_at: Option<String>,
        merged_by: Option<Actor>,
        closed_at: Option<String>,
        head_ref_name: String,
        base_ref_name: String,
        #[serde(default)]
        additions: u64,
        #[serde(default)]
        deletions: u64,
        #[serde(default)]
        changed_files: u64,
        #[serde(default)]
        review_decision: Option<String>,
        #[serde(default)]
        mergeable: String,
        #[serde(default)]
        merge_state_status: String,
        #[serde(default)]
        labels: Vec<Label>,
        milestone: Option<Milestone>,
        #[serde(default)]
        assignees: Vec<Actor>,
        #[serde(default)]
        review_requests: Vec<Actor>,
        #[serde(default)]
        latest_reviews: Vec<GhReview>,
        #[serde(default)]
        reviews: Vec<GhReview>,
        #[serde(default)]
        status_check_rollup: Vec<Status>,
        #[serde(default)]
        comments: Vec<Comment>,
        #[serde(default)]
        commits: Vec<Commit>,
    }

    let pr: Viewed = serde_json::from_str(stdout.trim())
        .map_err(|e| format!("gh said something unexpected: {e}"))?;
    let state = PullRequestState::read(&pr.state);
    let reviewers = reviewers_of(pr.latest_reviews, pr.review_requests);
    let checks = latest_checks(pr.status_check_rollup);

    Ok(PullRequestDetails {
        number: pr.number,
        title: pr.title,
        url: pr.url,
        state,
        draft: pr.is_draft,
        body: pr.body,
        author: pr.author.login(),
        created_at: pr.created_at,
        updated_at: pr.updated_at,
        merged_at: pr.merged_at.filter(|t| !t.is_empty()),
        merged_by: pr.merged_by.map(Actor::login),
        closed_at: pr.closed_at.filter(|t| !t.is_empty()),
        head: pr.head_ref_name,
        base: pr.base_ref_name,
        additions: pr.additions,
        deletions: pr.deletions,
        changed_files: pr.changed_files,
        review_decision: pr.review_decision.filter(|d| !d.is_empty()),
        mergeable: pr.mergeable,
        merge_state: pr.merge_state_status,
        labels: pr
            .labels
            .into_iter()
            .map(|l| PullRequestLabel {
                name: l.name,
                color: l.color,
            })
            .collect(),
        milestone: pr.milestone.map(|m| m.title),
        assignees: pr.assignees.into_iter().map(Actor::login).collect(),
        reviewers,
        checks,
        comments: pr
            .comments
            .into_iter()
            .map(|c| PullRequestComment {
                author: c.author.login(),
                body: c.body,
                created_at: c.created_at,
                url: c.url,
                edited: c.includes_created_edit,
                minimized: c.is_minimized,
            })
            .collect(),
        reviews: pr
            .reviews
            .into_iter()
            .filter(|r| r.state != "PENDING")
            .map(|r| SubmittedReview {
                author: r.author.login(),
                state: r.state,
                body: r.body,
                submitted_at: r.submitted_at.unwrap_or_default(),
            })
            .collect(),
        threads: Vec::new(),
        threads_error: None,
        teams: Vec::new(),
        teams_error: None,
        commits: pr
            .commits
            .into_iter()
            .map(|c| PullRequestCommit {
                short_id: c.oid.chars().take(7).collect(),
                subject: c.message_headline,
                author: c
                    .authors
                    .into_iter()
                    .next()
                    .and_then(|a| a.login.filter(|l| !l.is_empty()).or(a.name))
                    .unwrap_or_default(),
                date: c.committed_date,
            })
            .collect(),
    })
}

/// Reads the comments on the PR's code (`THREAD_JQ`'s lines) into threads: each first comment
/// with its replies, oldest first.
fn threads(stdout: &str) -> Result<Vec<CodeThread>, String> {
    #[derive(Deserialize)]
    struct Line {
        id: u64,
        in_reply_to_id: Option<u64>,
        path: String,
        line: Option<u64>,
        original_line: Option<u64>,
        html_url: String,
        #[serde(default)]
        diff_hunk: String,
        #[serde(default)]
        body: String,
        created_at: String,
        user: Option<String>,
    }
    let mut threads: Vec<(u64, CodeThread)> = Vec::new();
    let mut replies: Vec<Line> = Vec::new();
    for line in stdout.lines().filter(|l| !l.trim().is_empty()) {
        let c: Line =
            serde_json::from_str(line).map_err(|e| format!("gh said something unexpected: {e}"))?;
        if c.in_reply_to_id.is_some() {
            replies.push(c);
            continue;
        }
        let comment = PullRequestComment {
            author: c.user.unwrap_or_else(|| "ghost".into()),
            body: c.body,
            created_at: c.created_at,
            url: Some(c.html_url.clone()),
            edited: false,
            minimized: false,
        };
        threads.push((
            c.id,
            CodeThread {
                path: c.path,
                line: c.line,
                original_line: c.original_line,
                url: c.html_url,
                diff_hunk: c.diff_hunk,
                comments: vec![comment],
            },
        ));
    }
    for c in replies {
        // (Replies all answer a thread's first comment.)
        if let Some((_, thread)) = threads
            .iter_mut()
            .find(|(id, _)| Some(*id) == c.in_reply_to_id)
        {
            thread.comments.push(PullRequestComment {
                author: c.user.unwrap_or_else(|| "ghost".into()),
                body: c.body,
                created_at: c.created_at,
                url: Some(c.html_url),
                edited: false,
                minimized: false,
            });
        }
    }
    let mut threads: Vec<CodeThread> = threads.into_iter().map(|(_, t)| t).collect();
    for t in &mut threads {
        t.comments.sort_by(|a, b| a.created_at.cmp(&b.created_at));
    }
    threads.sort_by(|a, b| a.comments[0].created_at.cmp(&b.comments[0].created_at));
    Ok(threads)
}

/// Runs `gh` with `args` in `worktree`, never prompting, its output going to `sink` as it comes.
async fn run_gh(
    gh: &Path,
    worktree: &Path,
    args: &[&str],
    sink: Option<crate::process::Sink<'_>>,
) -> Result<std::process::Output, String> {
    let mut cmd = crate::process::command(gh);
    for (key, value) in gh_env().await {
        cmd.env(key, value);
    }
    cmd.current_dir(worktree)
        .args(args)
        .env("GH_PROMPT_DISABLED", "1")
        .env("GH_NO_UPDATE_NOTIFIER", "1")
        .env("NO_COLOR", "1")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    crate::process::ProcessTree::own_group(&mut cmd);
    let child = cmd.spawn().map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => GH_HINT.to_owned(),
        _ => format!("could not run gh: {e}"),
    })?;
    let _tree = crate::process::ProcessTree::attach(&child);
    tokio::time::timeout(GH_TIMEOUT, crate::process::collect(child, sink))
        .await
        .map_err(|_| format!("gh took over {} s and was stopped.", GH_TIMEOUT.as_secs()))?
        .map_err(|e| format!("could not run gh: {e}"))
}

/// Reads what `gh pr create` said: the new PR's URL (its last line), or the open one's when there
/// already is one.
fn opened(success: bool, stdout: &str, stderr: &str) -> Result<Opened, String> {
    let url = |text: &str| {
        text.split_whitespace()
            .rev()
            .find(|word| word.starts_with("https://") || word.starts_with("http://"))
            .map(str::to_owned)
    };
    if success {
        return url(stdout)
            .map(Opened::Created)
            .ok_or_else(|| format!("gh opened the PR but didn't say where: {}", stdout.trim()));
    }
    if stderr.contains("already exists") {
        if let Some(url) = url(stderr) {
            return Ok(Opened::AlreadyOpen(url));
        }
    }
    Err(gh_failure(stdout, stderr))
}

/// What a `gh` that failed said, with how to log in if that's what it needs.
fn gh_failure(stdout: &str, stderr: &str) -> String {
    let message = match stderr.trim() {
        "" => stdout.trim(),
        stderr => stderr,
    };
    let lower = message.to_lowercase();
    match GH_LOGIN_SIGNS.iter().any(|sign| lower.contains(sign)) {
        true => format!("{message}\n{GH_HINT}"),
        false => message.to_owned(),
    }
}

/// The environment `gh` runs with: on Unix the login shell's (started from the desktop, Orchard's
/// own `PATH` may not have `gh`, nor a `GH_TOKEN` the profile sets).
async fn gh_env() -> Vec<(OsString, OsString)> {
    #[cfg(unix)]
    return crate::setup::shell_env().await.to_vec();
    #[cfg(not(unix))]
    return vec![];
}

/// The GitHub CLI to run: the configured one, else `gh` (found on `PATH`).
pub(crate) fn gh_program(configured: Option<&PathBuf>) -> PathBuf {
    configured.cloned().unwrap_or_else(|| PathBuf::from("gh"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_actions_checks_link_names_its_run_and_job() {
        let job = |url| ActionsJob::from_url(url);
        assert_eq!(
            job("https://github.com/o/r/actions/runs/123/job/456"),
            Some(ActionsJob { run: 123, job: 456 })
        );
        assert_eq!(
            job("https://github.com/o/r/actions/runs/123/job/456?pr=7"),
            Some(ActionsJob { run: 123, job: 456 })
        );
        assert_eq!(
            job("https://github.com/o/r/actions/runs/123/jobs/9"),
            Some(ActionsJob { run: 123, job: 9 })
        );
        assert_eq!(job("https://github.com/o/r/actions/runs/123"), None);
        assert_eq!(job("https://ci.example.com/build/9"), None);
    }

    #[test]
    fn only_a_checks_latest_attempt_counts() {
        let statuses: Vec<Status> = serde_json::from_str(
            r#"[
            {"__typename":"CheckRun","name":"ci_tests","workflowName":"CI","status":"COMPLETED","conclusion":"FAILURE","startedAt":"2026-10-07T10:00:00Z"},
            {"__typename":"CheckRun","name":"deploy","workflowName":"Deploy","status":"COMPLETED","conclusion":"SUCCESS","startedAt":"2026-10-07T10:00:00Z"},
            {"__typename":"CheckRun","name":"ci_tests","workflowName":"CI","status":"IN_PROGRESS","conclusion":"","startedAt":"2026-10-07T11:00:00Z"},
            {"__typename":"CheckRun","name":"deploy","workflowName":"Other","status":"COMPLETED","conclusion":"SUCCESS","startedAt":"2026-10-07T10:00:00Z"},
            {"__typename":"CheckRun","name":"deploy","workflowName":"Deploy","status":"COMPLETED","conclusion":"FAILURE","startedAt":"2026-10-07T09:00:00Z"},
            {"__typename":"StatusContext","context":"ext","state":"SUCCESS","createdAt":"2026-10-07T10:00:00Z"}
        ]"#,
        )
        .unwrap();
        let checks = latest_checks(statuses);
        let seen: Vec<_> = checks
            .iter()
            .map(|c| (c.workflow.as_deref(), c.name.as_str(), c.outcome))
            .collect();
        assert_eq!(
            seen,
            [
                (Some("CI"), "ci_tests", CheckOutcome::Pending),
                (Some("Deploy"), "deploy", CheckOutcome::Success),
                (Some("Other"), "deploy", CheckOutcome::Success),
                (None, "ext", CheckOutcome::Success),
            ]
        );
    }

    #[test]
    fn a_job_to_rerun_is_found_by_its_id_else_its_name() {
        let jobs = "11\tbuild\n22\tlint\n";
        assert_eq!(job_id(jobs, 22, "whatever"), Some(22));
        assert_eq!(job_id(jobs, 9, "build"), Some(11));
        assert_eq!(job_id(jobs, 9, "deploy"), None);
    }

    #[test]
    fn my_prs_are_listed_most_recently_updated_first() {
        let out = r#"[
            {"number":3,"title":"Old","url":"u3","state":"OPEN","isDraft":false,"headRefName":"a","baseRefName":"main","reviewDecision":"","latestReviews":[],"reviewRequests":[{"__typename":"User","login":"bo"}],"statusCheckRollup":[],"mergeable":"CONFLICTING","updatedAt":"2026-01-01T00:00:00Z","mergedAt":null,"closedAt":null},
            {"number":9,"title":"New","url":"u9","state":"MERGED","isDraft":false,"headRefName":"b","baseRefName":"dev","reviewDecision":"APPROVED","latestReviews":[],"reviewRequests":[],"statusCheckRollup":[],"mergeable":"UNKNOWN","updatedAt":"2026-02-01T00:00:00Z","mergedAt":"2026-02-01T00:00:00Z","closedAt":"2026-02-01T00:00:00Z"}
        ]"#;
        let prs = mine_listed(out).unwrap();
        assert_eq!(prs.iter().map(|p| p.number).collect::<Vec<_>>(), [9, 3]);
        assert_eq!(prs[0].stage, PullRequestStage::Merged);
        assert_eq!(prs[1].stage, PullRequestStage::WaitingForReview);
        assert_eq!(prs[1].review_decision, None);
        assert!(prs[1].conflicts && !prs[0].conflicts);
        assert_eq!(prs[1].reviewers[0].name, "bo");
        assert_eq!((prs[1].head.as_str(), prs[1].base.as_str()), ("a", "main"));
    }

    #[test]
    fn a_prs_stage_goes_by_its_reviewers_and_asked_for_changes_come_first() {
        use PullRequestStage::*;
        let open = PullRequestState::Open;
        let check = |outcome| Check {
            name: "ci".into(),
            workflow: None,
            outcome,
            url: None,
            job: None,
        };
        let failing = [check(CheckOutcome::Failure)];
        let passing = [check(CheckOutcome::Success), check(CheckOutcome::Pending)];
        let by = |state| Reviewer {
            name: "bo".into(),
            team: false,
            state,
            requested: false,
        };
        let again = |state| Reviewer {
            requested: true,
            ..by(state)
        };
        let stage = |decision, reviewers: &[Reviewer], checks: &[Check]| {
            stage_of(open, false, decision, reviewers, checks)
        };
        let changes = [by(ReviewerState::ChangesRequested)];
        assert_eq!(
            stage(Some("CHANGES_REQUESTED"), &changes, &failing),
            ChangesRequested
        );
        assert_eq!(stage(None, &changes, &[]), ChangesRequested);
        // Asked to review again: the changes are answered, whatever GitHub's decision still says.
        let answered = [again(ReviewerState::ChangesRequested)];
        assert_eq!(
            stage(Some("CHANGES_REQUESTED"), &answered, &passing),
            WaitingForReview
        );
        assert_eq!(
            stage(Some("CHANGES_REQUESTED"), &answered, &failing),
            ChecksFailing
        );
        assert_eq!(stage(Some("APPROVED"), &[], &failing), ChecksFailing);
        assert_eq!(stage(Some("APPROVED"), &[], &passing), Approved);
        assert_eq!(
            stage(
                Some("REVIEW_REQUIRED"),
                &[by(ReviewerState::Approved)],
                &passing
            ),
            WaitingForReview
        );
        // No review required: someone's approval is enough.
        assert_eq!(
            stage(None, &[by(ReviewerState::Approved)], &passing),
            Approved
        );
        assert_eq!(
            stage(
                None,
                &[
                    by(ReviewerState::Approved),
                    again(ReviewerState::ChangesRequested)
                ],
                &passing
            ),
            Approved
        );
        assert_eq!(
            stage(None, &[by(ReviewerState::Commented)], &[]),
            WaitingForReview
        );
        assert_eq!(stage_of(open, true, Some("APPROVED"), &[], &failing), Draft);
        assert_eq!(
            stage_of(PullRequestState::Closed, false, None, &[], &[]),
            Closed
        );
    }

    #[test]
    fn the_new_prs_url_is_its_last_line() {
        let out =
            "Creating pull request for agent/x into main in o/r\n\nhttps://github.com/o/r/pull/7\n";
        assert_eq!(
            opened(true, out, ""),
            Ok(Opened::Created("https://github.com/o/r/pull/7".into()))
        );
    }

    #[test]
    fn an_open_pr_for_the_branch_is_found() {
        let err = "a pull request for branch \"agent/x\" into branch \"main\" already exists:\nhttps://github.com/o/r/pull/3\n";
        assert_eq!(
            opened(false, "", err),
            Ok(Opened::AlreadyOpen("https://github.com/o/r/pull/3".into()))
        );
    }

    #[test]
    fn a_missing_login_says_how_to_log_in() {
        let err = "To get started with GitHub CLI, please run:  gh auth login";
        assert!(opened(false, "", err).unwrap_err().contains(GH_HINT));
    }

    #[test]
    fn the_listed_pr_is_read() {
        let out = r#"[{"baseRefName":"main","number":12,"url":"https://github.com/o/r/pull/12"}]"#;
        assert_eq!(
            listed(out),
            Ok(Some(OpenPullRequest {
                number: 12,
                url: "https://github.com/o/r/pull/12".into(),
                target: "main".into()
            }))
        );
        assert_eq!(listed("[]\n"), Ok(None));
    }

    #[test]
    fn a_remote_branch_loses_its_remote() {
        assert_eq!(on_remote("origin/main", "origin"), Some("main"));
        assert_eq!(on_remote("origin/feat/x", "origin"), Some("feat/x"));
        assert_eq!(on_remote("upstream/main", "origin"), None);
        assert_eq!(on_remote("originals/main", "origin"), None);
    }
    #[test]
    fn reviewers_stand_by_their_latest_review_and_requests_wait() {
        let out = r#"{"number":5,"title":"T","url":"u","state":"OPEN","isDraft":true,"body":"b",
            "author":{"login":"me"},"createdAt":"c","updatedAt":"u","mergedAt":null,"mergedBy":null,
            "closedAt":null,"headRefName":"feat","baseRefName":"main","additions":3,"deletions":1,
            "changedFiles":2,"reviewDecision":"","mergeable":"MERGEABLE","mergeStateStatus":"CLEAN",
            "labels":[{"name":"bug","color":"d73a4a"}],"milestone":{"title":"v1"},
            "assignees":[{"login":"me","name":"Me"}],
            "reviewRequests":[{"__typename":"User","login":"ann"},{"__typename":"Team","name":"Core","slug":"o/core"}],
            "latestReviews":[{"author":{"login":"ann"},"state":"CHANGES_REQUESTED","body":"","submittedAt":"t1"},
                             {"author":{"login":"bob"},"state":"APPROVED","body":"","submittedAt":"t2"},
                             {"author":{"login":"me"},"state":"PENDING","body":"","submittedAt":null}],
            "reviews":[],"comments":[],"commits":[],
            "statusCheckRollup":[{"__typename":"CheckRun","name":"build","workflowName":"CI","status":"COMPLETED","conclusion":"SUCCESS","detailsUrl":"d"},
                                 {"__typename":"CheckRun","name":"lint","workflowName":"","status":"IN_PROGRESS","conclusion":"","detailsUrl":""},
                                 {"__typename":"StatusContext","context":"deploy","state":"ERROR","targetUrl":"t"}]}"#;
        let pr = viewed(out).unwrap();
        assert_eq!(pr.state, PullRequestState::Open);
        assert!(pr.draft);
        assert_eq!(pr.review_decision, None);
        assert_eq!(pr.milestone.as_deref(), Some("v1"));
        assert_eq!(
            pr.reviewers,
            vec![
                Reviewer {
                    name: "ann".into(),
                    team: false,
                    state: ReviewerState::ChangesRequested,
                    requested: true
                },
                Reviewer {
                    name: "bob".into(),
                    team: false,
                    state: ReviewerState::Approved,
                    requested: false
                },
                Reviewer {
                    name: "o/core".into(),
                    team: true,
                    state: ReviewerState::Requested,
                    requested: true
                },
            ]
        );
        let outcomes: Vec<_> = pr
            .checks
            .iter()
            .map(|c| (c.name.as_str(), c.outcome))
            .collect();
        assert_eq!(
            outcomes,
            vec![
                ("build", CheckOutcome::Success),
                ("lint", CheckOutcome::Pending),
                ("deploy", CheckOutcome::Failure)
            ]
        );
        assert_eq!(pr.checks[1].workflow, None);
    }

    #[test]
    fn only_top_level_teams_are_groups() {
        let out = concat!(
            r#"{"name":"QA","slug":"qa","parent":null}"#,
            "\n",
            r#"{"name":"Manager","slug":"manager","parent":"developers"}"#,
            "\n",
            r#"{"name":"Developers","slug":"developers","parent":null}"#,
            "\n",
        );
        assert_eq!(
            top_level(out),
            Ok(vec![
                ("Developers".into(), "developers".into()),
                ("QA".into(), "qa".into())
            ])
        );
    }

    #[test]
    fn code_comments_gather_into_threads() {
        let out = concat!(
            r#"{"id":1,"in_reply_to_id":null,"path":"a.rs","line":4,"original_line":4,"html_url":"h1","diff_hunk":"@@","body":"why?","created_at":"2026-01-01T00:00:00Z","user":"ann"}"#,
            "\n",
            r#"{"id":3,"in_reply_to_id":null,"path":"b.rs","line":null,"original_line":9,"html_url":"h3","diff_hunk":"@@","body":"typo","created_at":"2026-01-03T00:00:00Z","user":"bob"}"#,
            "\n",
            r#"{"id":2,"in_reply_to_id":1,"path":"a.rs","line":4,"original_line":4,"html_url":"h2","diff_hunk":"@@","body":"because","created_at":"2026-01-02T00:00:00Z","user":"me"}"#,
            "\n",
        );
        let threads = threads(out).unwrap();
        assert_eq!(threads.len(), 2);
        assert_eq!(threads[0].path, "a.rs");
        let said: Vec<_> = threads[0]
            .comments
            .iter()
            .map(|c| c.body.as_str())
            .collect();
        assert_eq!(said, ["why?", "because"]);
        assert_eq!(threads[1].line, None);
        assert_eq!(threads[1].original_line, Some(9));
    }
}
