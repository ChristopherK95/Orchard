//! Ticket 19: push, fetch and fast-forward-only pull against a temp bare origin; a fetch when the
//! window gets focus at most every 5 minutes; and a remote that wants a login fails fast.

mod support;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use editor_core::{Core, CoreConfig, NewWorktree, PullOutcome, PushOutcome, SessionState};
use serde_json::json;
use support::*;

async fn open(core: &Core, repo: &Path) -> PathBuf {
    git(repo, &["config", "user.name", "Test"]);
    git(repo, &["config", "user.email", "test@example.com"]);
    core.open_workspace(repo).await.unwrap().root
}

/// A commit in the bare origin's `main`.
fn origin_main(setup: &RepoWithOrigin) -> String {
    rev_parse(&setup.root().join("origin.git"), "main")
}

#[tokio::test(flavor = "multi_thread")]
async fn push_sends_local_commits_and_gives_a_new_branch_its_upstream() {
    let setup = RepoWithOrigin::new();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    let root = open(&core, &setup.repo()).await;
    commit(&root, "local");

    assert!(matches!(
        core.push(&root).await.unwrap(),
        PushOutcome::Pushed { .. }
    ));
    assert_eq!(origin_main(&setup), rev_parse(&root, "HEAD"));

    // A branch that's never been pushed goes to origin under its own name.
    git(&root, &["checkout", "--quiet", "-b", "agent/new"]);
    commit(&root, "on the branch");
    core.push(&root).await.unwrap();
    assert_eq!(
        rev_parse(&setup.root().join("origin.git"), "agent/new"),
        rev_parse(&root, "HEAD")
    );
    let status = core.git_status(&root).await.unwrap();
    assert_eq!(status.upstream.as_deref(), Some("origin/agent/new"));
    assert_eq!((status.ahead, status.behind), (Some(0), Some(0)));
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_then_a_fast_forward_pull() {
    let setup = RepoWithOrigin::new();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    let root = open(&core, &setup.repo()).await;
    let theirs = setup.push_to_origin("main", "theirs");

    core.fetch(&root).await.unwrap();
    let status = core.git_status(&root).await.unwrap();
    assert_eq!((status.ahead, status.behind), (Some(0), Some(1)));

    assert_eq!(
        core.pull(&root, false).await.unwrap(),
        PullOutcome::FastForwarded { commits: 1 }
    );
    assert_eq!(rev_parse(&root, "HEAD"), theirs);
    assert_eq!(
        core.pull(&root, false).await.unwrap(),
        PullOutcome::UpToDate
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_diverged_pull_changes_nothing_and_says_why() {
    let setup = RepoWithOrigin::new();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    let root = open(&core, &setup.repo()).await;
    setup.push_to_origin("main", "theirs");
    commit(&root, "mine");
    let mine = rev_parse(&root, "HEAD");

    assert_eq!(
        core.pull(&root, false).await.unwrap(),
        PullOutcome::Diverged {
            ahead: 1,
            behind: 1
        }
    );
    assert_eq!(rev_parse(&root, "HEAD"), mine, "no merge, no rebase");
    // Even when git is set to rebase on pull.
    git(&root, &["config", "pull.rebase", "true"]);
    assert!(matches!(
        core.pull(&root, false).await.unwrap(),
        PullOutcome::Diverged { .. }
    ));
    assert_eq!(rev_parse(&root, "HEAD"), mine);
}

#[tokio::test(flavor = "multi_thread")]
async fn focus_fetches_the_shown_worktree_at_most_every_five_minutes() {
    let setup = RepoWithOrigin::new();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let clock = Arc::new(FakeClock::default());
    let core = Core::new(CoreConfig {
        clock: Some(clock.clone()),
        ..CoreConfig::new(fake.command())
    });
    let root = open(&core, &setup.repo()).await;
    assert!(core.window_focused().await.is_empty(), "nothing shown yet");
    core.show_worktree(&root).await.unwrap();

    assert_eq!(core.window_focused().await, std::slice::from_ref(&root));
    setup.push_to_origin("main", "theirs");
    clock.advance(Duration::from_secs(4 * 60));
    assert!(core.window_focused().await.is_empty(), "too soon");
    assert_eq!(core.git_status(&root).await.unwrap().behind, Some(0));

    clock.advance(Duration::from_secs(60));
    assert_eq!(core.window_focused().await, std::slice::from_ref(&root));
    assert_eq!(core.git_status(&root).await.unwrap().behind, Some(1));
}

/// An HTTP "remote" that wants a login for everything.
fn remote_wanting_a_login() -> String {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/repo.git", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for mut socket in listener.incoming().flatten() {
            let mut buf = [0u8; 4096];
            let _ = socket.read(&mut buf);
            let _ = socket.write_all(
                b"HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic realm=\"x\"\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            );
        }
    });
    url
}

#[tokio::test(flavor = "multi_thread")]
async fn a_remote_that_wants_a_login_fails_fast_with_the_terminal_hint() {
    let repo = git_repo();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    let root = open(&core, repo.path()).await;
    let url = remote_wanting_a_login();
    git(&root, &["remote", "add", "origin", &url]);
    // (No credential helper here: the machine's own would otherwise answer, or open a window.)
    git(&root, &["config", "credential.helper", ""]);

    let started = Instant::now();
    let err = core.push(&root).await.unwrap_err().to_string();
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "{:?}",
        started.elapsed()
    );
    assert!(err.contains("run `git push` in a terminal once"), "{err}");
    let err = core.fetch(&root).await.unwrap_err().to_string();
    assert!(err.contains("run `git push` in a terminal once"), "{err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_worktree_made_from_origin_main_pushes_to_its_own_branch() {
    let setup = RepoWithOrigin::new();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    open(&core, &setup.repo()).await;
    let worktree = core
        .create_worktree(NewWorktree::NewBranch {
            name: "agent/fix".into(),
            start_point: None, // origin/main
        })
        .await
        .unwrap()
        .worktree
        .path;
    let before = origin_main(&setup);
    commit(&worktree, "the Agent's work");

    let outcome = core.push(&worktree).await.unwrap();
    assert_eq!(
        outcome,
        PushOutcome::Pushed {
            to: "origin/agent/fix".into()
        }
    );
    assert_eq!(origin_main(&setup), before, "main untouched");
    assert_eq!(
        rev_parse(&setup.root().join("origin.git"), "agent/fix"),
        rev_parse(&worktree, "HEAD")
    );
    let status = core.git_status(&worktree).await.unwrap();
    assert_eq!(status.upstream.as_deref(), Some("origin/agent/fix"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_push_the_remote_turns_down_says_so() {
    let setup = RepoWithOrigin::new();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    let root = open(&core, &setup.repo()).await;
    setup.push_to_origin("main", "theirs");
    commit(&root, "mine");

    assert_eq!(core.push(&root).await.unwrap(), PushOutcome::Rejected);
}

#[tokio::test(flavor = "multi_thread")]
async fn pulling_while_a_session_is_mid_turn_asks_first() {
    let setup = RepoWithOrigin::new();
    let fake = FakeAgent::new(&json!({ "turns": [{ "untilCancelled": true }] }).to_string());
    let core = core_with(&fake);
    let root = open(&core, &setup.repo()).await;
    let theirs = setup.push_to_origin("main", "theirs");
    let session = core.new_session().await.unwrap();
    let mut states = core.subscribe();
    core.send_prompt(session, "work").await.unwrap();
    states_until(&mut states, session, SessionState::Working).await;

    assert_eq!(
        core.pull(&root, false).await.unwrap(),
        PullOutcome::SessionsWorking {
            sessions: vec!["Session 1".into()]
        }
    );
    assert_ne!(rev_parse(&root, "HEAD"), theirs, "nothing pulled");
    assert_eq!(
        core.pull(&root, true).await.unwrap(),
        PullOutcome::FastForwarded { commits: 1 }
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn one_focus_fetch_brings_every_followed_worktree_up_to_date() {
    let setup = RepoWithOrigin::new();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let clock = Arc::new(FakeClock::default());
    let core = Core::new(CoreConfig {
        clock: Some(clock.clone()),
        ..CoreConfig::new(fake.command())
    });
    let root = open(&core, &setup.repo()).await;
    // A second Worktree on its own branch tracking origin/main, with a session (so it's followed).
    let other = core
        .create_worktree(NewWorktree::NewBranch {
            name: "agent/other".into(),
            start_point: None,
        })
        .await
        .unwrap()
        .worktree
        .path;
    git(
        &other,
        &["branch", "--quiet", "--set-upstream-to=origin/main"],
    );
    core.new_session_in(&other).await.unwrap();
    core.show_worktree(&root).await.unwrap();
    // A manual fetch counts: focus waits five minutes after it.
    core.fetch(&root).await.unwrap();
    setup.push_to_origin("main", "theirs");
    assert!(core.window_focused().await.is_empty(), "just fetched");

    clock.advance(Duration::from_secs(5 * 60));
    let fetched = core.window_focused().await;
    assert!(
        fetched.contains(&root) && fetched.contains(&other),
        "{fetched:?}"
    );
    assert_eq!(core.git_status(&root).await.unwrap().behind, Some(1));
    assert_eq!(core.git_status(&other).await.unwrap().behind, Some(1));
}
