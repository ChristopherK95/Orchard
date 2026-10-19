//! Issue 44: a Code review of a PR the user reviews, in a read-only Review Worktree detached at
//! the PR's head, with its comments kept as a Pending review until they're submitted.
#![cfg(unix)]

mod support;

use std::path::{Path, PathBuf};

use editor_core::{CodeReview, Core, CoreError, CoreEvent, SetupStatus};
use support::*;
use tokio::sync::broadcast;

/// `gh pr view 7` as GitHub would answer it for a PR from `feat` into `main`, the user `me` asked
/// to review it (`requested`) or not.
fn pr_view(head: &str, requested: bool, reviews: &str) -> String {
    let requests = match requested {
        true => r#"[{"login":"me"}]"#,
        false => "[]",
    };
    format!(
        r#"{{"id":"PR_kw7","number":7,"title":"Make it better","body":"Why it's better","url":"https://github.com/o/r/pull/7","state":"OPEN","createdAt":"2026-01-01T00:00:00Z","updatedAt":"2026-01-01T00:00:00Z","mergedAt":null,"mergedBy":null,"closedAt":null,"headRefName":"feat","baseRefName":"main","headRefOid":"{head}","baseRefOid":"","milestone":null,"reviewRequests":{requests},"reviews":{reviews}}}"#
    )
}

/// A fake `gh` answering for PR 7 with `view` and the user as `me`; `extra` is more `case` arms,
/// tried first.
fn review_gh(view: &str, extra: &str) -> FakeGh {
    FakeGh::new(&format!(
        "case \"$*\" in\n\
         {extra}\n\
         'pr view'*) cat <<'JSON'\n{view}\nJSON\n;;\n\
         'api user'*) echo me ;;\n\
         'api repos/{{owner}}/{{repo}} '*) echo 'User o' ;;\n\
         'api repos/{{owner}}/{{repo}}/pulls/7/comments'*) ;;\n\
         *) echo \"unexpected gh $*\" >&2; exit 1 ;;\n\
         esac"
    ))
}

/// Commits `files` (path, contents) on the seed clone's `feat` branch and pushes it as PR 7's head
/// (`refs/pull/7/head`, as GitHub keeps it), returning the commit.
fn push_pr(setup: &RepoWithOrigin, files: &[(&str, &str)], message: &str) -> String {
    let seed = setup.root().join("seed");
    if Command::new("git")
        .args(["rev-parse", "--verify", "--quiet", "feat"])
        .current_dir(&seed)
        .status()
        .is_ok_and(|s| s.success())
    {
        git(&seed, &["checkout", "--quiet", "feat"]);
    } else {
        git(&seed, &["checkout", "--quiet", "-b", "feat", "main"]);
    }
    for (path, text) in files {
        std::fs::write(seed.join(path), text).unwrap();
        git(&seed, &["add", path]);
    }
    commit(&seed, message);
    git(
        &seed,
        &[
            "push",
            "--quiet",
            "--force",
            "origin",
            "HEAD:refs/pull/7/head",
        ],
    );
    rev_parse(&seed, "HEAD")
}

use std::process::Command;

fn git_out(dir: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// Waits for the setup in `worktree` to finish or fail, saying which.
async fn setup_outcome(events: &mut broadcast::Receiver<CoreEvent>, worktree: &Path) -> bool {
    next_event(events, "setup to end", |e| match e {
        CoreEvent::SetupChanged {
            worktree: w,
            status,
            ..
        } if w == worktree => match status {
            SetupStatus::Finished => Some(true),
            SetupStatus::Failed { .. } => Some(false),
            SetupStatus::Done { .. } => panic!("a Review Worktree's setup started a session"),
            _ => None,
        },
        _ => None,
    })
    .await
}

/// The repo's settings with `setup` as its setup commands.
fn with_setup(fake: &FakeAgent, repo: &Path, setup: &str) {
    let settings = repo_settings(
        &origin_url(repo),
        &format!("setup = [{}]", toml_literal(setup)),
    );
    std::fs::write(fake.settings_path(), settings).unwrap();
}

async fn open(core: &Core, repo: &Path) -> PathBuf {
    core.open_workspace(repo).await.unwrap().root
}

fn runs(counter: &Path) -> usize {
    std::fs::read_to_string(counter)
        .unwrap_or_default()
        .lines()
        .count()
}

#[tokio::test(flavor = "multi_thread")]
async fn opening_a_code_review_makes_its_review_worktree_once() {
    let setup = RepoWithOrigin::new();
    let head = push_pr(&setup, &[("a.txt", "one\n")], "Add a");
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let gh = review_gh(&pr_view(&head, true, "[]"), "");
    let counter = setup.root().join("setup-runs");
    with_setup(
        &fake,
        &setup.repo(),
        &format!("echo ran >> {}", counter.display()),
    );
    let state = fake.state_path();
    let core = core_with_gh_and_state(&fake, &gh, &state);
    open(&core, &setup.repo()).await;
    let mut events = core.subscribe();

    let review: CodeReview = core.open_code_review(7).await.unwrap();
    assert_eq!(review.number, 7);
    assert_eq!(review.title, "Make it better");
    assert!(
        review.worktree.ends_with("review-pr-7"),
        "{:?}",
        review.worktree
    );
    assert_eq!(review.head, head);
    assert_eq!(review.base, "main");
    assert_eq!(review.merge_base, rev_parse(&setup.repo(), "origin/main"));
    let files: Vec<(&str, u32, u32)> = review
        .files
        .iter()
        .map(|f| (f.path.as_str(), f.additions, f.deletions))
        .collect();
    assert_eq!(files, [("a.txt", 1, 0)]);
    assert_eq!(rev_parse(&review.worktree, "HEAD"), head);
    assert_eq!(
        git_out(&review.worktree, &["symbolic-ref", "-q", "HEAD"]),
        None,
        "HEAD is detached"
    );
    assert!(review.setup.is_some(), "its setup runs");
    assert!(setup_outcome(&mut events, &review.worktree).await);
    assert_eq!(runs(&counter), 1);
    assert!(
        fake.received("session/new").is_empty(),
        "no Agent session starts by itself"
    );
    let listed = core
        .worktrees()
        .into_iter()
        .find(|w| w.path == review.worktree)
        .expect("listed");
    assert_eq!(listed.review, Some(7));
    assert_eq!(core.is_review_worktree(&review.worktree), Some(7));

    let again = core.open_code_review(7).await.unwrap();
    assert_eq!(again.worktree, review.worktree);
    drop(core);

    let core = core_with_gh_and_state(&fake, &gh, &state);
    open(&core, &setup.repo()).await;
    let restarted = core.open_code_review(7).await.unwrap();
    assert_eq!(restarted.worktree, review.worktree);
    assert!(restarted.setup.is_none(), "a finished setup isn't rerun");
    let review_worktrees = core.worktrees().into_iter().filter(|w| w.review == Some(7));
    assert_eq!(review_worktrees.count(), 1);
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert_eq!(runs(&counter), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pr_the_user_doesnt_review_wont_open() {
    let setup = RepoWithOrigin::new();
    let head = push_pr(&setup, &[("a.txt", "one\n")], "Add a");
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let gh = review_gh(&pr_view(&head, false, "[]"), "");
    let core = core_with_gh(&fake, &gh);
    let root = open(&core, &setup.repo()).await;

    let err = core.open_code_review(7).await.unwrap_err();
    assert!(matches!(err, CoreError::NotAReviewer), "{err:?}");
    assert_eq!(core.worktrees().len(), 1, "no Review Worktree");
    assert_eq!(
        git_out(
            &root,
            &["rev-parse", "--verify", "-q", "refs/orchard/review/7"]
        ),
        None
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_setup_runs_again_on_the_next_open() {
    let setup = RepoWithOrigin::new();
    let head = push_pr(&setup, &[("a.txt", "one\n")], "Add a");
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let gh = review_gh(&pr_view(&head, true, "[]"), "");
    let marker = setup.root().join("failed-once");
    with_setup(
        &fake,
        &setup.repo(),
        &format!(
            "test -f {m} || {{ touch {m}; exit 1; }}",
            m = marker.display()
        ),
    );
    let state = fake.state_path();
    let core = core_with_gh_and_state(&fake, &gh, &state);
    open(&core, &setup.repo()).await;
    let mut events = core.subscribe();

    let review = core.open_code_review(7).await.unwrap();
    assert!(!setup_outcome(&mut events, &review.worktree).await, "fails");
    drop(core);

    let core = core_with_gh_and_state(&fake, &gh, &state);
    open(&core, &setup.repo()).await;
    let mut events = core.subscribe();
    let review = core.open_code_review(7).await.unwrap();
    assert!(review.setup.is_some(), "a failed setup runs again");
    assert!(setup_outcome(&mut events, &review.worktree).await);
}
