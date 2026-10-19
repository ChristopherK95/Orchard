//! Issue 44: a Code review of a PR the user reviews, in a read-only Review Worktree detached at
//! the PR's head, with its comments kept as a Pending review until they're submitted.
#![cfg(unix)]

mod support;

use std::path::{Path, PathBuf};

use editor_core::{
    CodeReview, Core, CoreError, CoreEvent, DiffSide, NewReviewComment, Question, SetupStatus,
    Verdict,
};
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

fn added(lines: &[editor_core::DiffLine]) -> Vec<&str> {
    lines
        .iter()
        .filter(|l| l.kind == editor_core::DiffLineKind::Added)
        .map(|l| l.text.as_str())
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn the_diff_is_the_prs_and_can_narrow_to_since_the_last_review() {
    let setup = RepoWithOrigin::new();
    let a = push_pr(&setup, &[("a", "first\n"), ("b", "one\n")], "A");
    let head = push_pr(&setup, &[("b", "one\ntwo\n")], "B");
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let reviews = format!(
        r#"[{{"author":{{"login":"me"}},"state":"COMMENTED","submittedAt":"2026-01-02T00:00:00Z","commit":{{"oid":"{a}"}}}}]"#
    );
    let gh = review_gh(&pr_view(&head, true, &reviews), "");
    let core = core_with_gh(&fake, &gh);
    open(&core, &setup.repo()).await;

    let review = core.open_code_review(7).await.unwrap();
    let since = review.since_last_review.expect("reviewed before");
    assert_eq!(since.commit, a);
    let since_paths: Vec<&str> = since.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(since_paths, ["b"]);

    let full = core.code_review_diff(7, "b", false).await.unwrap();
    assert_eq!(added(&full), ["one", "two"]);
    let narrowed = core.code_review_diff(7, "b", true).await.unwrap();
    assert_eq!(added(&narrowed), ["two"]);

    // The diff is the PR's, whatever is in the Review Worktree's files.
    std::fs::write(review.worktree.join("a"), "junk\n").unwrap();
    let a_diff = core.code_review_diff(7, "a", false).await.unwrap();
    assert_eq!(added(&a_diff), ["first"]);
    assert!(core
        .code_review_diff(7, "a", true)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_first_review_has_no_since_last_review() {
    let setup = RepoWithOrigin::new();
    let head = push_pr(&setup, &[("a", "first\n")], "A");
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let gh = review_gh(&pr_view(&head, true, "[]"), "");
    let core = core_with_gh(&fake, &gh);
    open(&core, &setup.repo()).await;
    let review = core.open_code_review(7).await.unwrap();
    assert_eq!(review.since_last_review, None);
}

fn comment_on(path: &str, start_line: Option<u32>, line: u32, body: &str) -> NewReviewComment {
    NewReviewComment {
        path: path.into(),
        side: DiffSide::Right,
        start_line,
        line,
        body: body.into(),
        from_question: false,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_pending_review_survives_a_restart() {
    let setup = RepoWithOrigin::new();
    let head = push_pr(&setup, &[("a", "l1\nl2\nl3\nl4\n")], "A");
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let gh = review_gh(&pr_view(&head, true, "[]"), "");
    let state = fake.state_path();
    let core = core_with_gh_and_state(&fake, &gh, &state);
    open(&core, &setup.repo()).await;
    core.open_code_review(7).await.unwrap();
    let mut events = core.subscribe();

    let single = core
        .add_review_comment(7, comment_on("a", None, 2, "Why l2?"))
        .await
        .unwrap();
    assert_eq!(single.line_text, "l2");
    assert!(!single.outdated);
    next_event(&mut events, "the Pending review to change", |e| match e {
        CoreEvent::PendingReviewChanged { number: 7 } => Some(()),
        _ => None,
    })
    .await;
    let range = core
        .add_review_comment(7, comment_on("a", Some(1), 3, "These three"))
        .await
        .unwrap();
    assert_ne!(range.id, single.id);
    core.edit_review_comment(7, range.id, "These three lines")
        .await
        .unwrap();
    core.add_review_question(
        7,
        Question {
            path: "a".into(),
            side: DiffSide::Right,
            start_line: None,
            line: 4,
            attachment: "a:4-4 (new)".into(),
        },
    )
    .await
    .unwrap();
    let off = core
        .add_review_comment(7, comment_on("a", None, 40, "Nowhere"))
        .await
        .unwrap_err();
    assert!(matches!(off, CoreError::NotInDiff), "{off:?}");
    drop(core);

    let core = core_with_gh_and_state(&fake, &gh, &state);
    open(&core, &setup.repo()).await;
    let review = core.open_code_review(7).await.unwrap();
    let comments: Vec<(Option<u32>, u32, &str, &str)> = review
        .pending
        .comments
        .iter()
        .map(|c| (c.start_line, c.line, c.body.as_str(), c.line_text.as_str()))
        .collect();
    assert_eq!(
        comments,
        [
            (None, 2, "Why l2?", "l2"),
            (Some(1), 3, "These three lines", "l3")
        ]
    );
    assert_eq!(review.pending.questions.len(), 1);
    assert_eq!(review.pending.questions[0].attachment, "a:4-4 (new)");

    core.remove_review_comment(7, single.id).await.unwrap();
    assert!(core.remove_review_comment(7, single.id).await.is_err());
    let review = core.open_code_review(7).await.unwrap();
    assert_eq!(review.pending.comments.len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn submit_sends_one_call_and_a_failure_keeps_the_pending_review() {
    let setup = RepoWithOrigin::new();
    std::fs::create_dir_all(setup.root().join("seed/src")).unwrap();
    let head = push_pr(
        &setup,
        &[("src/a.rs", "fn a() {}\n"), ("src/b.rs", "fn b() {\n}\n")],
        "A and B",
    );
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let sent = setup.root().join("stdin");
    let marker = setup.root().join("failed-once");
    let submit = format!(
        "'api -X POST repos/{{owner}}/{{repo}}/pulls/7/reviews --input -') cat > {sent}\n\
         if [ ! -f {marker} ]; then touch {marker}\n\
         echo '{{\"message\":\"Unprocessable Entity\",\"errors\":[\"Line could not be resolved in src/b.rs\"],\"status\":\"422\"}}'\n\
         echo 'gh: Unprocessable Entity (HTTP 422)' >&2; exit 1; fi\n\
         echo '{{\"id\":1}}' ;;",
        sent = sent.display(),
        marker = marker.display()
    );
    let gh = review_gh(&pr_view(&head, true, "[]"), &submit);
    let core = core_with_gh(&fake, &gh);
    open(&core, &setup.repo()).await;
    core.open_code_review(7).await.unwrap();
    core.add_review_comment(7, comment_on("src/a.rs", None, 1, "Name it better"))
        .await
        .unwrap();
    core.add_review_comment(7, comment_on("src/b.rs", Some(1), 2, "Empty?"))
        .await
        .unwrap();

    let err = core
        .submit_review(7, Verdict::RequestChanges, "A few things")
        .await
        .unwrap_err();
    let CoreError::SubmitFailed { message, paths } = err else {
        panic!("not a failed submit: {err:?}");
    };
    assert!(message.contains("could not be resolved"), "{message}");
    assert_eq!(paths, ["src/b.rs"]);
    let review = core.open_code_review(7).await.unwrap();
    assert_eq!(review.pending.comments.len(), 2, "a failure keeps them");

    let mut events = core.subscribe();
    core.submit_review(7, Verdict::RequestChanges, "A few things")
        .await
        .unwrap();
    next_event(&mut events, "the Pending review to empty", |e| match e {
        CoreEvent::PendingReviewChanged { number: 7 } => Some(()),
        _ => None,
    })
    .await;
    let body: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&sent).unwrap()).unwrap();
    assert_eq!(body["commit_id"], head.as_str());
    assert_eq!(body["event"], "REQUEST_CHANGES");
    assert_eq!(body["body"], "A few things");
    let comments = body["comments"].as_array().unwrap();
    assert_eq!(comments.len(), 2);
    assert_eq!(comments[0]["path"], "src/a.rs");
    assert_eq!(comments[0]["side"], "RIGHT");
    assert_eq!(comments[0]["line"], 1);
    assert!(comments[0].get("start_line").is_none());
    assert_eq!(comments[1]["start_line"], 1);
    assert_eq!(comments[1]["start_side"], "RIGHT");
    assert_eq!(comments[1]["line"], 2);
    assert_eq!(comments[1]["body"], "Empty?");
    let review = core.open_code_review(7).await.unwrap();
    assert!(review.pending.comments.is_empty());

    // As GitHub, a comment or change request needs something said.
    let empty = core
        .submit_review(7, Verdict::Comment, " ")
        .await
        .unwrap_err();
    assert!(matches!(empty, CoreError::EmptyReview), "{empty:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reply_is_posted_to_its_thread() {
    let setup = RepoWithOrigin::new();
    let head = push_pr(&setup, &[("a", "first\n")], "A");
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let reply = "'api -X POST repos/{owner}/{repo}/pulls/7/comments/41/replies'*) \
         echo '{\"id\":42,\"in_reply_to_id\":41,\"body\":\"Fair point\",\"created_at\":\"2026-01-03T00:00:00Z\",\"html_url\":\"https://github.com/o/r/pull/7#discussion_r42\",\"user\":{\"login\":\"me\"}}' ;;";
    let gh = review_gh(&pr_view(&head, true, "[]"), reply);
    let core = core_with_gh(&fake, &gh);
    open(&core, &setup.repo()).await;
    core.open_code_review(7).await.unwrap();

    let posted = core.reply_to_thread(7, 41, "Fair point").await.unwrap();
    assert_eq!(
        gh.args(),
        [
            "api",
            "-X",
            "POST",
            "repos/{owner}/{repo}/pulls/7/comments/41/replies",
            "-f",
            "body=Fair point"
        ]
    );
    assert_eq!(posted.id, Some(42));
    assert_eq!(posted.author, "me");
    assert_eq!(posted.body, "Fair point");
    assert_eq!(posted.created_at, "2026-01-03T00:00:00Z");
    assert_eq!(
        posted.url.as_deref(),
        Some("https://github.com/o/r/pull/7#discussion_r42")
    );
}
