//! Issue 44: a Code review of a PR the user reviews, in a read-only Review Worktree detached at
//! the PR's head, with its comments kept as a Pending review until they're submitted.
#![cfg(unix)]

mod support;

use std::path::{Path, PathBuf};

use editor_core::{
    CodeReview, Core, CoreError, CoreEvent, DiffSide, FileContent, NewReviewComment,
    PermissionMode, Question, SaveOver, SetupStatus, Verdict,
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
         'api graphql'*) echo '{{\"data\":{{\"node\":{{\"files\":{{\"nodes\":[],\"pageInfo\":{{\"hasNextPage\":false}}}}}}}}}}' ;;\n\
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
        .stdout(std::process::Stdio::null())
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
    // Its review session starts once the setup has finished.
    eventually("the review session", || {
        fake.received("session/new").len() == 1
    })
    .await;
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

#[tokio::test(flavor = "multi_thread")]
async fn viewed_ticks_are_githubs() {
    let setup = RepoWithOrigin::new();
    let head = push_pr(&setup, &[("a", "first\n"), ("b", "second\n")], "A and B");
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    // (GitHub pages the files: `a` comes on the first page, `b` on the second.)
    let graphql = "'api graphql'*unmarkFileAsViewed*) echo 'gh: Something went wrong' >&2; exit 1 ;;\n\
         'api graphql'*markFileAsViewed*) echo '{\"data\":{}}' ;;\n\
         'api graphql'*after=page2*) echo '{\"data\":{\"node\":{\"files\":{\"nodes\":[{\"path\":\"b\",\"viewerViewedState\":\"UNVIEWED\"}],\"pageInfo\":{\"hasNextPage\":false,\"endCursor\":null}}}}}' ;;\n\
         'api graphql'*viewerViewedState*) echo '{\"data\":{\"node\":{\"files\":{\"nodes\":[{\"path\":\"a\",\"viewerViewedState\":\"VIEWED\"}],\"pageInfo\":{\"hasNextPage\":true,\"endCursor\":\"page2\"}}}}}' ;;";
    let gh = review_gh(&pr_view(&head, true, "[]"), graphql);
    let core = core_with_gh(&fake, &gh);
    open(&core, &setup.repo()).await;

    let review = core.open_code_review(7).await.unwrap();
    let viewed: Vec<(&str, bool)> = review
        .files
        .iter()
        .map(|f| (f.path.as_str(), f.viewed))
        .collect();
    assert_eq!(viewed, [("a", true), ("b", false)]);

    core.set_review_file_viewed(7, "b", true).await.unwrap();
    let args = gh.args();
    assert_eq!(&args[..2], ["api", "graphql"]);
    assert!(
        args.iter()
            .any(|a| a.starts_with("query=") && a.contains("markFileAsViewed")),
        "{args:?}"
    );
    assert!(args.iter().any(|a| a == "id=PR_kw7"), "{args:?}");
    assert!(args.iter().any(|a| a == "path=b"), "{args:?}");

    let failed = core.set_review_file_viewed(7, "a", false).await;
    assert!(failed.is_err(), "a failure is said");
}

/// Commits `files` on `main` in the seed clone and pushes it, before the PR branches off.
fn push_main(setup: &RepoWithOrigin, files: &[(&str, &str)], message: &str) {
    let seed = setup.root().join("seed");
    git(&seed, &["checkout", "--quiet", "main"]);
    for (path, text) in files {
        std::fs::write(seed.join(path), text).unwrap();
        git(&seed, &["add", path]);
    }
    commit(&seed, message);
    git(&seed, &["push", "--quiet", "origin", "main"]);
}

fn lines_of(lines: &[&str]) -> String {
    lines.iter().map(|l| format!("{l}\n")).collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn only_lines_githubs_diff_shows_take_comments() {
    // Two changes whose hunks git (and so GitHub) keeps apart, while another diff algorithm
    // would join them into one.
    let before = lines_of(&[
        "}", "{", "a", "", "a", "a", "{", "a", "{", "b", "", "a", "a", "{", "b", "}", "a", "{",
        "}", "{", "", "", "", "b", "b",
    ]);
    let after = lines_of(&[
        "}", "z", "a", "", "a", "a", "{", "a", "{", "b", "", "a", "{", "b", "}", "a", "{", "}", "",
        "", "", "b", "b",
    ]);
    let setup = RepoWithOrigin::new();
    push_main(&setup, &[("f", &before)], "Base");
    let head = push_pr(&setup, &[("f", &after)], "Change f");
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let gh = review_gh(&pr_view(&head, true, "[]"), "");
    let core = core_with_gh(&fake, &gh);
    open(&core, &setup.repo()).await;
    core.open_code_review(7).await.unwrap();

    // git's hunks: new lines 1-5, 9-14 and 16-21.
    core.add_review_comment(7, comment_on("f", None, 9, "In the second hunk"))
        .await
        .unwrap();
    let between = core
        .add_review_comment(7, comment_on("f", None, 15, "Between hunks"))
        .await
        .unwrap_err();
    assert!(matches!(between, CoreError::NotInDiff), "{between:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_removed_comments_id_isnt_used_again() {
    let setup = RepoWithOrigin::new();
    let head = push_pr(&setup, &[("a", "l1\nl2\nl3\n")], "A");
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let gh = review_gh(&pr_view(&head, true, "[]"), "");
    let state = fake.state_path();
    let core = core_with_gh_and_state(&fake, &gh, &state);
    open(&core, &setup.repo()).await;
    core.open_code_review(7).await.unwrap();

    let first = core
        .add_review_comment(7, comment_on("a", None, 1, "One"))
        .await
        .unwrap();
    let newest = core
        .add_review_comment(7, comment_on("a", None, 2, "Two"))
        .await
        .unwrap();
    core.remove_review_comment(7, newest.id).await.unwrap();
    let again = core
        .add_review_comment(7, comment_on("a", None, 2, "Two again"))
        .await
        .unwrap();
    assert!(![first.id, newest.id].contains(&again.id), "{}", again.id);

    // Nor after a restart, with the Pending review emptied.
    core.remove_review_comment(7, first.id).await.unwrap();
    core.remove_review_comment(7, again.id).await.unwrap();
    drop(core);
    let core = core_with_gh_and_state(&fake, &gh, &state);
    open(&core, &setup.repo()).await;
    core.open_code_review(7).await.unwrap();
    let after_restart = core
        .add_review_comment(7, comment_on("a", None, 3, "Three"))
        .await
        .unwrap();
    assert!(
        ![first.id, newest.id, again.id].contains(&after_restart.id),
        "{}",
        after_restart.id
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn refresh_keeps_comments_and_clears_viewed_on_changed_files() {
    let setup = RepoWithOrigin::new();
    let numbered: Vec<String> = (1..=30).map(|n| format!("l{n}")).collect();
    let numbered: Vec<&str> = numbered.iter().map(String::as_str).collect();
    let head = push_pr(
        &setup,
        &[("a", &lines_of(&numbered)), ("b", "unchanged\n")],
        "A and B",
    );
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let graphql = "'api graphql'*unmarkFileAsViewed*) echo '{\"data\":{}}' ;;\n\
         'api graphql'*viewerViewedState*) echo '{\"data\":{\"node\":{\"files\":{\"nodes\":[{\"path\":\"a\",\"viewerViewedState\":\"VIEWED\"},{\"path\":\"b\",\"viewerViewedState\":\"VIEWED\"}],\"pageInfo\":{\"hasNextPage\":false,\"endCursor\":null}}}}}' ;;";
    let gh = review_gh(&pr_view(&head, true, "[]"), graphql);
    let core = core_with_gh(&fake, &gh);
    open(&core, &setup.repo()).await;
    let review = core.open_code_review(7).await.unwrap();
    let range = core
        .add_review_comment(7, comment_on("a", Some(9), 10, "These two"))
        .await
        .unwrap();
    let gone = core
        .add_review_comment(7, comment_on("a", None, 20, "This one"))
        .await
        .unwrap();
    assert_eq!(core.code_review_new_commits(7).await.unwrap(), 0);

    // Three lines in above the first comment, the second's line deleted.
    let mut changed: Vec<&str> = numbered.clone();
    changed.retain(|l| *l != "l20");
    for (at, line) in [(5, "new1"), (6, "new2"), (7, "new3")] {
        changed.insert(at, line);
    }
    let new_head = push_pr(&setup, &[("a", &lines_of(&changed))], "Move a's lines");
    assert_eq!(core.code_review_new_commits(7).await.unwrap(), 1);

    let refreshed = core.refresh_code_review(7).await.unwrap();
    assert_eq!(refreshed.head, new_head);
    assert_eq!(rev_parse(&review.worktree, "HEAD"), new_head);
    assert_eq!(core.code_review_new_commits(7).await.unwrap(), 0);
    let comments = &refreshed.pending.comments;
    let moved = comments.iter().find(|c| c.id == range.id).unwrap();
    assert_eq!((moved.start_line, moved.line), (Some(12), 13));
    assert!(!moved.outdated);
    let outdated = comments.iter().find(|c| c.id == gone.id).unwrap();
    assert!(outdated.outdated);
    let viewed: Vec<(&str, bool)> = refreshed
        .files
        .iter()
        .map(|f| (f.path.as_str(), f.viewed))
        .collect();
    assert_eq!(viewed, [("a", false), ("b", true)]);
    let unmarked: Vec<Vec<String>> = gh
        .calls()
        .into_iter()
        .filter(|call| call.iter().any(|a| a.contains("unmarkFileAsViewed")))
        .collect();
    assert_eq!(unmarked.len(), 1, "{unmarked:?}");
    assert!(unmarked[0].iter().any(|a| a == "path=a"), "{unmarked:?}");

    // An outdated comment holds the submit back.
    let err = core
        .submit_review(7, Verdict::Comment, "Looks fine")
        .await
        .unwrap_err();
    assert!(
        matches!(err, CoreError::OutdatedComments { count: 1 }),
        "{err:?}"
    );
}

/// Waits until the Review Worktree's setup isn't running (it has none here, so it ends at once).
async fn setup_over(core: &Core, worktree: &Path) {
    eventually("setup to be over", || {
        !matches!(
            core.setup(worktree).map(|s| s.status),
            Some(SetupStatus::Running { .. } | SetupStatus::StartingSession)
        )
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_review_worktree_is_read_only() {
    let setup = RepoWithOrigin::new();
    let head = push_pr(&setup, &[("a", "first\n")], "A");
    let fake = FakeAgent::new(r#"{"turns":[{"chunks":["Done planning"],"setMode":"default"}]}"#);
    let gh = review_gh(&pr_view(&head, true, "[]"), "");
    let core = core_with_gh(&fake, &gh);
    open(&core, &setup.repo()).await;
    let review = core.open_code_review(7).await.unwrap();
    let file = review.worktree.join("a");

    let opened = core.read_file(&file).await.unwrap();
    assert!(
        matches!(
            opened.content,
            FileContent::Text {
                read_only: true,
                ..
            }
        ),
        "{:?}",
        opened.content
    );
    let write = core
        .save_file(&file, "changed\n", "\n", SaveOver::Anything, "main")
        .await
        .unwrap_err();
    assert!(matches!(write, CoreError::ReadOnlyWorktree), "{write:?}");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "first\n");
    let discard = core.discard(&review.worktree, "a").await.unwrap_err();
    assert!(
        matches!(discard, CoreError::ReadOnlyWorktree),
        "{discard:?}"
    );

    let session = review.session.expect("the review session");
    let info = core.session_info(session).unwrap();
    assert_eq!(info.permission_mode, PermissionMode::Plan);
    assert!(info.mode_locked);
    let set_mode = fake.received("session/set_mode");
    assert_eq!(set_mode.len(), 1);
    assert_eq!(set_mode[0]["params"]["modeId"], "plan");
    let refused = core
        .set_permission_mode(session, PermissionMode::AcceptEdits)
        .await
        .unwrap_err();
    assert!(matches!(refused, CoreError::ModeLocked), "{refused:?}");

    // The Agent leaving Plan mode by itself is put back in it.
    let mut events = core.subscribe();
    core.send_prompt(session, "Plan it").await.unwrap();
    next_event(&mut events, "Plan mode again", |e| match e {
        CoreEvent::PermissionModeChanged {
            session_id,
            mode: PermissionMode::Plan,
        } if session_id == session => Some(()),
        CoreEvent::PermissionModeChanged { mode, .. } => panic!("left Plan mode for {mode:?}"),
        _ => None,
    })
    .await;
    eventually("plan to be set again", || {
        fake.received("session/set_mode").len() == 2
    })
    .await;
    assert_eq!(
        fake.received("session/set_mode")[1]["params"]["modeId"],
        "plan"
    );
    assert_eq!(
        core.session_info(session).unwrap().permission_mode,
        PermissionMode::Plan
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_review_session_is_still_locked_in_plan_mode_after_a_restart() {
    let setup = RepoWithOrigin::new();
    let head = push_pr(&setup, &[("a", "first\n")], "A");
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let gh = review_gh(&pr_view(&head, true, "[]"), "");
    let state = fake.state_path();
    let core = core_with_gh_and_state(&fake, &gh, &state);
    open(&core, &setup.repo()).await;
    let review = core.open_code_review(7).await.unwrap();
    let session = review.session.expect("the review session");
    // (Only a session that has had a turn is kept as a Tab.)
    core.send_prompt(session, "What does it do?").await.unwrap();
    eventually("the Agent to answer", || {
        core.transcript(session)
            .unwrap()
            .iter()
            .any(|item| matches!(item, editor_core::TranscriptItem::Agent { .. }))
    })
    .await;
    drop(core);

    let core = core_with_gh_and_state(&fake, &gh, &state);
    open(&core, &setup.repo()).await;
    let restored = core
        .sessions()
        .into_iter()
        .find(|s| s.worktree == review.worktree)
        .expect("restored");
    assert!(restored.mode_locked);
    assert_eq!(restored.permission_mode, PermissionMode::Plan);
}

#[tokio::test(flavor = "multi_thread")]
async fn one_review_session_per_pr_gets_the_review_prompt() {
    let setup = RepoWithOrigin::new();
    let head = push_pr(&setup, &[("a", "first\n"), ("b", "second\n")], "A and B");
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let gh = review_gh(&pr_view(&head, true, "[]"), "");
    let core = core_with_gh(&fake, &gh);
    open(&core, &setup.repo()).await;
    let review = core.open_code_review(7).await.unwrap();
    setup_over(&core, &review.worktree).await;

    let session = core.claude_review(7).await.unwrap();
    let info = core.session_info(session).unwrap();
    assert_eq!(info.worktree, review.worktree);
    assert_eq!(info.permission_mode, PermissionMode::Plan);
    eventually("the review prompt", || {
        fake.received("session/prompt").len() == 1
    })
    .await;
    let prompts = fake.received("session/prompt");
    let prompt = prompts[0]["params"]["prompt"].to_string();
    for needed in [
        head.as_str(),
        review.merge_base.as_str(),
        "#7",
        "Make it better",
        "Why it's better",
        "`a`",
        "`b`",
    ] {
        assert!(prompt.contains(needed), "{needed} not in {prompt}");
    }
    eventually("the Agent to answer", || {
        core.transcript(session)
            .unwrap()
            .iter()
            .any(|item| matches!(item, editor_core::TranscriptItem::Agent { .. }))
    })
    .await;

    assert_eq!(
        core.claude_review(7).await.unwrap(),
        session,
        "the same one"
    );
    assert_eq!(core.code_review_session(7).await.unwrap(), session);
    eventually("the second answer", || {
        core.session_info(session).unwrap().state == editor_core::SessionState::Idle
            && fake.received("session/prompt").len() == 2
    })
    .await;

    // Closed, it's reopened (its conversation with it), not started afresh.
    core.close_tab(session).await.unwrap();
    let reopened = core.code_review_session(7).await.unwrap();
    assert_eq!(fake.received("session/new").len(), 1);
    assert!(core.recent_sessions(&review.worktree).is_empty());
    assert_eq!(
        core.session_info(reopened).unwrap().worktree,
        review.worktree
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_question_attaches_the_lines_asked_about() {
    let setup = RepoWithOrigin::new();
    let numbered: String = (1..=20).map(|n| format!("l{n}\n")).collect();
    push_main(&setup, &[("a", &numbered)], "Base");
    let head = push_pr(&setup, &[("a", &numbered.replace("l10\n", "ten\n"))], "Ten");
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let gh = review_gh(&pr_view(&head, true, "[]"), "");
    let core = core_with_gh(&fake, &gh);
    open(&core, &setup.repo()).await;
    core.open_code_review(7).await.unwrap();

    let new = core
        .ask_attachment(7, "a", DiffSide::Right, Some(9), 10)
        .await
        .unwrap();
    let editor_core::Attachment::Text { name, text, .. } = new else {
        panic!("not text: {new:?}");
    };
    assert_eq!(name, "a:9-10 (new)");
    assert!(
        text.contains("ten") && text.contains("l4") && text.contains("l15"),
        "{text}"
    );
    assert!(!text.contains("l3\n") && !text.contains("l16"), "{text}");
    let old = core
        .ask_attachment(7, "a", DiffSide::Left, None, 10)
        .await
        .unwrap();
    let editor_core::Attachment::Text { name, text, .. } = old else {
        panic!("not text: {old:?}");
    };
    assert_eq!(name, "a:10-10 (old)");
    assert!(text.contains("l10") && !text.contains("ten"), "{text}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_review_worktree_is_removable_once_its_review_is_done() {
    let setup = RepoWithOrigin::new();
    let head = push_pr(&setup, &[("a", "first\n")], "A");
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let submit =
        "'api -X POST repos/{owner}/{repo}/pulls/7/reviews --input -') echo '{\"id\":1}' ;;";
    let gh = review_gh(&pr_view(&head, true, "[]"), submit);
    let state = fake.state_path();
    let core = core_with_gh_and_state(&fake, &gh, &state);
    let root = open(&core, &setup.repo()).await;
    let review = core.open_code_review(7).await.unwrap();
    setup_over(&core, &review.worktree).await;
    let review_row = |rows: Vec<editor_core::WorktreeMerge>| {
        rows.into_iter()
            .find(|row| row.path == review.worktree)
            .expect("listed")
            .review
    };

    let before = review_row(core.merge_overview().await.unwrap()).expect("a review row");
    assert_eq!((before.number, before.done), (7, false));

    let session = core.code_review_session(7).await.unwrap();
    core.submit_review(7, Verdict::Approve, "").await.unwrap();
    let after = review_row(core.merge_overview().await.unwrap()).unwrap();
    assert!(after.done, "submitted");
    let check = core.removal_check(&review.worktree).await.unwrap();
    assert!(
        !check.discard_to_remove,
        "the PR's commits aren't lost with it: {check:?}"
    );
    assert_eq!(check.sessions, [session]);
    assert_eq!(check.review_session, Some(session));

    git(&root, &["update-ref", "refs/orchard/review/7-last", &head]);
    core.remove_worktree(&review.worktree, Default::default())
        .await
        .unwrap();
    for gone in ["refs/orchard/review/7", "refs/orchard/review/7-last"] {
        assert_eq!(
            git_out(&root, &["rev-parse", "--verify", "-q", gone]),
            None,
            "{gone}"
        );
    }
    drop(core);

    let core = core_with_gh_and_state(&fake, &gh, &state);
    open(&core, &setup.repo()).await;
    let forgotten = core.code_review_diff(7, "a", false).await.unwrap_err();
    assert!(
        matches!(forgotten, CoreError::NoCodeReview(7)),
        "{forgotten:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_merged_prs_review_is_done() {
    let setup = RepoWithOrigin::new();
    let head = push_pr(&setup, &[("a", "first\n")], "A");
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let marker = setup.root().join("merged");
    // (`pr view 7 --json state` says MERGED once the marker is there.)
    let state_arm = format!(
        "'pr view 7 --json state') if [ -f {m} ]; then echo '{{\"state\":\"MERGED\"}}'; else echo '{{\"state\":\"OPEN\"}}'; fi ;;",
        m = marker.display()
    );
    let gh = review_gh(&pr_view(&head, true, "[]"), &state_arm);
    let core = core_with_gh(&fake, &gh);
    open(&core, &setup.repo()).await;
    let review = core.open_code_review(7).await.unwrap();
    let done = |rows: Vec<editor_core::WorktreeMerge>| {
        rows.into_iter()
            .find(|row| row.path == review.worktree)
            .and_then(|row| row.review)
            .map(|r| r.done)
    };
    assert_eq!(done(core.merge_overview().await.unwrap()), Some(false));
    std::fs::write(&marker, "").unwrap();
    assert_eq!(done(core.merge_overview().await.unwrap()), Some(true));
}

/// The ACP id of the conversation `session` last sent a prompt in (the Agent's own id for it).
fn prompted_acp_id(fake: &FakeAgent) -> String {
    let prompts = fake.received("session/prompt");
    let last = prompts.last().expect("a prompt was sent");
    last["params"]["sessionId"].as_str().unwrap().to_owned()
}

/// Waits for `session` to have the Agent's answer in its transcript.
async fn answered(core: &Core, session: editor_core::SessionId) {
    eventually("the Agent to answer", || {
        core.transcript(session)
            .unwrap()
            .iter()
            .any(|item| matches!(item, editor_core::TranscriptItem::Agent { .. }))
    })
    .await;
}

/// Takes `acp_id` out of every Recent sessions list in the state file, as if it had been closed so
/// long ago it dropped out of them.
fn forget_recent(state: &Path, acp_id: &str) {
    fn walk(value: &mut serde_json::Value, acp_id: &str) {
        match value {
            serde_json::Value::Object(map) => {
                if let Some(serde_json::Value::Array(recent)) = map.get_mut("recent") {
                    recent.retain(|s| s["acpId"] != acp_id);
                }
                map.values_mut().for_each(|v| walk(v, acp_id));
            }
            serde_json::Value::Array(items) => items.iter_mut().for_each(|v| walk(v, acp_id)),
            _ => {}
        }
    }
    let text = std::fs::read_to_string(state).unwrap();
    assert!(text.contains(acp_id), "{acp_id} isn't in the state file");
    let mut saved: serde_json::Value = serde_json::from_str(&text).unwrap();
    walk(&mut saved, acp_id);
    std::fs::write(state, serde_json::to_string(&saved).unwrap()).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_review_conversation_survives_closing_restarting_and_leaving_recent() {
    let setup = RepoWithOrigin::new();
    let head = push_pr(&setup, &[("a", "first\n")], "A");
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let gh = review_gh(&pr_view(&head, true, "[]"), "");
    let state = fake.state_path();
    let core = core_with_gh_and_state(&fake, &gh, &state);
    open(&core, &setup.repo()).await;

    let review = core.open_code_review(7).await.unwrap();
    let session = review.session.expect("the review session starts with it");
    let info = core.session_info(session).unwrap();
    assert_eq!(info.worktree, review.worktree);
    assert_eq!(info.name, "Review #7 Make it better");
    core.send_prompt(session, "What does it do?").await.unwrap();
    answered(&core, session).await;
    let acp_id = prompted_acp_id(&fake);

    // Its Tab closed, it's in the Recent sessions and comes back from there.
    core.close_tab(session).await.unwrap();
    let recent = core.recent_sessions(&review.worktree);
    assert_eq!(recent.len(), 1);
    assert_eq!(recent[0].acp_id, acp_id);
    let reopened = core.code_review_session(7).await.unwrap();
    assert_eq!(core.recent_sessions(&review.worktree).len(), 0);
    core.send_prompt(reopened, "And then?").await.unwrap();
    eventually("the second prompt", || {
        fake.received("session/prompt").len() == 2
    })
    .await;
    assert_eq!(prompted_acp_id(&fake), acp_id);
    drop(core);

    // After a restart its Tab is back, the same conversation.
    let core = core_with_gh_and_state(&fake, &gh, &state);
    open(&core, &setup.repo()).await;
    let review = core.open_code_review(7).await.unwrap();
    let restored = review.session.expect("its Tab is back");
    let tabs: Vec<_> = core
        .sessions()
        .into_iter()
        .filter(|s| s.worktree == review.worktree)
        .collect();
    assert_eq!(tabs.len(), 1, "{tabs:?}");
    assert_eq!(tabs[0].id, restored);
    assert_eq!(tabs[0].name, "Review #7 Make it better");
    core.close_tab(restored).await.unwrap();
    drop(core);

    // Closed so long ago it left the Recent sessions: still that conversation, by its id.
    forget_recent(&state, &acp_id);
    let core = core_with_gh_and_state(&fake, &gh, &state);
    open(&core, &setup.repo()).await;
    assert!(core.recent_sessions(&review.worktree).is_empty());
    let loads_before = fake.received("session/load").len();
    let review = core.open_code_review(7).await.unwrap();
    let again = review.session.expect("the review session");
    assert_eq!(core.code_review_session(7).await.unwrap(), again);
    let loads = fake.received("session/load");
    assert!(loads.len() > loads_before, "the conversation was loaded");
    assert_eq!(
        loads.last().unwrap()["params"]["sessionId"],
        acp_id.as_str()
    );
    assert_eq!(
        core.session_info(again).unwrap().name,
        "Review #7 Make it better"
    );
    assert_eq!(
        fake.received("session/new").len(),
        1,
        "never started afresh"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn no_other_session_in_a_review_worktree() {
    let setup = RepoWithOrigin::new();
    let head = push_pr(&setup, &[("a", "first\n")], "A");
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let gh = review_gh(&pr_view(&head, true, "[]"), "");
    let core = core_with_gh(&fake, &gh);
    open(&core, &setup.repo()).await;
    let review = core.open_code_review(7).await.unwrap();
    setup_over(&core, &review.worktree).await;

    let refused = core.new_session_in(&review.worktree).await.unwrap_err();
    assert!(
        matches!(refused, CoreError::ReviewWorktreeSession),
        "{refused:?}"
    );
    let in_it = core
        .sessions()
        .into_iter()
        .filter(|s| s.worktree == review.worktree)
        .count();
    assert_eq!(in_it, 1, "only the review session");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_review_session_starts_when_setup_finishes() {
    let setup = RepoWithOrigin::new();
    let head = push_pr(&setup, &[("a", "first\n")], "A");
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let long = "A title much too long to fit on a Tab in full";
    let view = pr_view(&head, true, "[]").replace("Make it better", long);
    let gh = review_gh(&view, "");
    let release = setup.root().join("release-setup");
    with_setup(
        &fake,
        &setup.repo(),
        &print_then_wait("setting-up", &release),
    );
    let core = core_with_gh_and_state(&fake, &gh, &fake.state_path());
    open(&core, &setup.repo()).await;
    let mut events = core.subscribe();

    let review = core.open_code_review(7).await.unwrap();
    assert!(review.setup.is_some());
    assert_eq!(review.session, None, "not while its setup runs");
    assert!(fake.received("session/new").is_empty());

    std::fs::write(&release, "").unwrap();
    let created = next_event(&mut events, "the review session", |e| match e {
        CoreEvent::SessionCreated { session } if session.worktree == review.worktree => {
            Some(session)
        }
        _ => None,
    })
    .await;
    assert!(created.name.chars().count() <= 40, "{}", created.name);
    assert!(
        created.name.starts_with("Review #7 A title"),
        "{}",
        created.name
    );
    assert!(created.name.ends_with('…'), "{}", created.name);
    assert_eq!(created.permission_mode, PermissionMode::Plan);
    let again = core.open_code_review(7).await.unwrap();
    assert_eq!(again.session, Some(created.id));
    assert_eq!(fake.received("session/new").len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_conversation_the_agent_cant_resume_gets_a_fresh_session() {
    let setup = RepoWithOrigin::new();
    let head = push_pr(&setup, &[("a", "first\n")], "A");
    let fake = FakeAgent::new(r#"{"turns":[],"failResume":true}"#);
    let gh = review_gh(&pr_view(&head, true, "[]"), "");
    let state = fake.state_path();
    let core = core_with_gh_and_state(&fake, &gh, &state);
    open(&core, &setup.repo()).await;
    let review = core.open_code_review(7).await.unwrap();
    let first = review.session.expect("the review session");
    // (Never prompted, it has no conversation the Agent could load back.)
    core.close_tab(first).await.unwrap();
    assert!(core.recent_sessions(&review.worktree).is_empty());

    let fresh = core.code_review_session(7).await.unwrap();
    assert_eq!(fake.received("session/new").len(), 2, "a fresh one");
    let in_it: Vec<_> = core
        .sessions()
        .into_iter()
        .filter(|s| s.worktree == review.worktree)
        .collect();
    assert_eq!(in_it.len(), 1, "no dead Tab is left: {in_it:?}");
    assert_eq!(in_it[0].id, fresh);
    assert_eq!(in_it[0].state, editor_core::SessionState::Idle);
    assert_eq!(in_it[0].name, "Review #7 Make it better");
    assert!(core.recent_sessions(&review.worktree).is_empty());

    // The fresh one is the review's from now on.
    core.send_prompt(fresh, "Hello").await.unwrap();
    answered(&core, fresh).await;
    let fresh_acp = prompted_acp_id(&fake);
    drop(core);
    let core = core_with_gh_and_state(&fake, &gh, &state);
    open(&core, &setup.repo()).await;
    core.open_code_review(7).await.unwrap();
    let back = core.code_review_session(7).await.unwrap();
    core.send_prompt(back, "Still you?").await.unwrap();
    eventually("the prompt", || fake.received("session/prompt").len() == 2).await;
    assert_eq!(prompted_acp_id(&fake), fresh_acp);
}
