//! Issue 43: what the Dashboard shows that the core works out — the Worktrees that need attention,
//! with real `git` and a bare origin, and what's asked of GitHub (a fake `gh` here).

mod support;

use std::path::{Path, PathBuf};
use std::process::Command;

use editor_core::{AttentionReason, Core, GitOperation, NewWorktree};
use support::*;

async fn new_worktree(core: &Core, name: &str) -> PathBuf {
    core.create_worktree(NewWorktree::NewBranch {
        name: name.into(),
        start_point: None,
    })
    .await
    .unwrap()
    .worktree
    .path
}

/// A commit in `dir` that writes `text` to `file`.
fn commit_file(dir: &Path, file: &str, text: &str) {
    std::fs::write(dir.join(file), text).unwrap();
    git(dir, &["add", "--", file]);
    commit(dir, &format!("write {file}"));
}

#[tokio::test(flavor = "multi_thread")]
async fn worktrees_needing_attention_are_listed_with_every_reason() {
    let setup = RepoWithOrigin::new();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    let root = core.open_workspace(&setup.repo()).await.unwrap().root;

    // Pushed once, then one more commit and two files not committed.
    let a = new_worktree(&core, "a").await;
    git(&a, &["push", "--quiet", "-u", "origin", "a"]);
    commit(&a, "not pushed");
    std::fs::write(a.join("one.txt"), "1").unwrap();
    std::fs::write(a.join("two.txt"), "2").unwrap();

    // Never pushed, one commit since its Base.
    let b = new_worktree(&core, "b").await;
    commit(&b, "only here");

    // Left in the middle of a rebase that conflicts.
    let c = new_worktree(&core, "c").await;
    git(&c, &["checkout", "--quiet", "-b", "c-other"]);
    commit_file(&c, "x.txt", "other\n");
    git(&c, &["checkout", "--quiet", "c"]);
    commit_file(&c, "x.txt", "mine\n");
    let rebase = Command::new("git")
        .args(["rebase", "c-other"])
        .current_dir(&c)
        .output()
        .unwrap();
    assert!(!rebase.status.success(), "the rebase conflicts");

    // Tracking a branch origin has moved on from.
    setup.push_to_origin("d", "first");
    let d = core
        .create_worktree(NewWorktree::ExistingBranch {
            name: "origin/d".into(),
        })
        .await
        .unwrap()
        .worktree
        .path;
    setup.push_to_origin("d", "second");
    git(&d, &["fetch", "--quiet"]);

    let listed = core.worktree_attention().await.unwrap();
    let reasons = |path: &Path| {
        listed
            .iter()
            .find(|w| w.path == path)
            .map(|w| w.reasons.clone())
    };
    assert_eq!(
        reasons(&root),
        None,
        "the main checkout is clean and pushed"
    );
    assert_eq!(
        reasons(&a),
        Some(vec![
            AttentionReason::Uncommitted { files: 2 },
            AttentionReason::Unpushed { commits: 1 },
        ])
    );
    assert_eq!(
        reasons(&b),
        Some(vec![AttentionReason::NoUpstream { commits: 1 }])
    );
    assert!(
        reasons(&c).unwrap().contains(&AttentionReason::Operation {
            operation: GitOperation::Rebase
        }),
        "{listed:?}"
    );
    assert_eq!(
        reasons(&d),
        Some(vec![AttentionReason::Behind { commits: 1 }])
    );
    let a_row = listed.iter().find(|w| w.path == a).unwrap();
    assert_eq!(a_row.branch.as_deref(), Some("a"));
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn review_requests_are_asked_of_gh_for_the_user_directly() {
    let setup = RepoWithOrigin::new();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let gh = FakeGh::new(
        r#"cat <<'JSON'
{"data":{"viewer":{"login":"me"},"search":{"nodes":[
  {"number":3,"title":"Fix it","url":"https://github.com/o/r/pull/3","author":{"login":"ann"},
   "headRefName":"fix","latestReviews":{"nodes":[]},
   "timelineItems":{"nodes":[{"createdAt":"2026-10-08T12:00:00Z","requestedReviewer":{"login":"me"}}]},
   "commits":{"nodes":[]}}]}}}
JSON"#,
    );
    let core = core_with_gh(&fake, &gh);
    let root = core.open_workspace(&setup.repo()).await.unwrap().root;

    let requests = core.review_requests(&root).await.unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].number, 3);
    assert_eq!(requests[0].author, "ann");
    assert_eq!(
        requests[0].requested_at.as_deref(),
        Some("2026-10-08T12:00:00Z")
    );
    assert!(!requests[0].re_requested);

    let args = gh.args();
    assert_eq!(&args[..2], ["api", "graphql"], "{args:?}");
    assert!(
        args.iter().any(|a| a.contains("user-review-requested:@me")
            && a.contains("draft:false")
            && a.contains("is:open")),
        "{args:?}"
    );
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn failing_check_logs_are_asked_of_gh_per_failed_job() {
    let setup = RepoWithOrigin::new();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    // The failed job's link says job 1; the run's jobs say it's 777.
    let gh = FakeGh::new(
        r#"case "$1 $2" in
"pr view") cat <<'JSON'
{"title":"Fix the parser","statusCheckRollup":[
  {"__typename":"CheckRun","name":"test","workflowName":"CI","status":"COMPLETED","conclusion":"FAILURE",
   "detailsUrl":"https://github.com/o/r/actions/runs/55/job/1","startedAt":"2026-10-08T12:00:00Z"},
  {"__typename":"CheckRun","name":"lint","workflowName":"CI","status":"COMPLETED","conclusion":"SUCCESS",
   "detailsUrl":"https://github.com/o/r/actions/runs/55/job/2","startedAt":"2026-10-08T12:00:00Z"}]}
JSON
;;
"run view")
  case "$*" in
  *--log-failed*) printf 'test\tRun tests\tassertion failed: parse("1+")\n' ;;
  *) printf '777\ttest\n778\tlint\n' ;;
  esac
;;
esac"#,
    );
    let core = core_with_gh(&fake, &gh);
    let root = core.open_workspace(&setup.repo()).await.unwrap().root;

    let prompt = core.failing_check_prompt(&root, 12).await.unwrap();
    assert!(prompt.contains("CI / test"), "{prompt}");
    assert!(
        prompt.contains(r#"assertion failed: parse("1+")"#),
        "{prompt}"
    );
    assert!(!prompt.contains("lint"), "{prompt}");
    // (The fake keeps the last call's arguments: the log's.)
    assert_eq!(
        gh.args(),
        ["run", "view", "55", "--job", "777", "--log-failed"]
    );
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn review_threads_are_asked_of_gh_with_their_resolved_state() {
    let setup = RepoWithOrigin::new();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let gh = FakeGh::new(
        r#"case "$1 $2" in
"pr view") cat <<'JSON'
{"title":"Fix the parser","reviews":[
  {"author":{"login":"ann"},"state":"CHANGES_REQUESTED","body":"Split it up.","submittedAt":"2026-10-08T12:00:00Z"}]}
JSON
;;
"api graphql") cat <<'JSON'
{"data":{"repository":{"pullRequest":{"reviewThreads":{"nodes":[
  {"isResolved":false,"path":"src/parse.rs","line":7,"originalLine":7,
   "comments":{"nodes":[{"author":{"login":"ann"},"body":"Why a loop here?"}]}}]}}}}}
JSON
;;
esac"#,
    );
    let core = core_with_gh(&fake, &gh);
    let root = core.open_workspace(&setup.repo()).await.unwrap().root;

    let prompt = core.review_feedback_prompt(&root, 12).await.unwrap();
    assert!(prompt.contains("Split it up."), "{prompt}");
    assert!(prompt.contains("src/parse.rs:7"), "{prompt}");
    assert!(prompt.contains("Why a loop here?"), "{prompt}");
    // (The fake keeps the last call's arguments: the threads'.)
    let args = gh.args();
    assert_eq!(&args[..2], ["api", "graphql"], "{args:?}");
    assert!(args.iter().any(|a| a == "number=12"), "{args:?}");
    assert!(args.iter().any(|a| a.contains("isResolved")), "{args:?}");
}
