//! Ticket 06: the Workspace's Worktrees, listed with real `git` and kept live.

mod support;

use std::path::Path;

use editor_core::{CoreEvent, WorktreeInfo};
use support::*;

fn same_path(a: &Path, b: &Path) -> bool {
    canonical(a) == canonical(b)
}

/// Waits for a `WorktreesChanged` event whose list satisfies `check`.
async fn worktrees_until(
    events: &mut tokio::sync::broadcast::Receiver<CoreEvent>,
    what: &str,
    check: impl Fn(&[WorktreeInfo]) -> bool,
) -> Vec<WorktreeInfo> {
    tokio::time::timeout(TIMEOUT, async {
        loop {
            if let CoreEvent::WorktreesChanged { worktrees } = events.recv().await.expect("event") {
                if check(&worktrees) {
                    return worktrees;
                }
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for worktrees: {what}"))
}

#[tokio::test(flavor = "multi_thread")]
async fn the_main_checkout_comes_first_followed_by_the_other_worktrees() {
    let repo = git_repo();
    let elsewhere = tempfile::tempdir().unwrap();
    let feature = elsewhere.path().join("feature");
    git(
        repo.path(),
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "feat/x",
            feature.to_str().unwrap(),
        ],
    );
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);

    core.open_workspace(repo.path()).await.unwrap();
    let worktrees = core.worktrees();

    assert_eq!(worktrees.len(), 2);
    assert!(worktrees[0].is_main);
    assert!(same_path(&worktrees[0].path, repo.path()));
    assert_eq!(worktrees[0].branch.as_deref(), Some("main"));
    assert!(!worktrees[1].is_main);
    assert!(same_path(&worktrees[1].path, &feature));
    assert_eq!(worktrees[1].branch.as_deref(), Some("feat/x"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_worktree_reports_ahead_behind_and_changed_files() {
    let repo = git_repo();
    let elsewhere = tempfile::tempdir().unwrap();
    let feature = elsewhere.path().join("feature");
    git(
        repo.path(),
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "feat/y",
            feature.to_str().unwrap(),
        ],
    );
    git(&feature, &["branch", "--quiet", "--set-upstream-to=main"]);
    commit(&feature, "one");
    commit(&feature, "two");
    std::fs::write(feature.join("new.txt"), "hello").unwrap();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);

    core.open_workspace(repo.path()).await.unwrap();
    let worktrees = core.worktrees();

    let feat = worktrees
        .iter()
        .find(|w| w.branch.as_deref() == Some("feat/y"))
        .unwrap();
    assert_eq!((feat.ahead, feat.behind), (Some(2), Some(0)));
    assert_eq!(feat.changed, 1);
    let main = &worktrees[0];
    assert_eq!(
        (main.ahead, main.behind),
        (None, None),
        "main has no upstream"
    );
    assert_eq!(main.changed, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_can_start_in_any_worktree_and_runs_there() {
    let repo = git_repo();
    let elsewhere = tempfile::tempdir().unwrap();
    let feature = elsewhere.path().join("feature");
    git(
        repo.path(),
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "feat/z",
            feature.to_str().unwrap(),
        ],
    );
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    core.open_workspace(repo.path()).await.unwrap();
    let worktree = core.worktrees()[1].path.clone();

    let session = core.new_session_in(&worktree).await.unwrap();

    assert!(same_path(
        &core.session_info(session).unwrap().worktree,
        &feature
    ));
    let cwd = fake.received("session/new")[0]["params"]["cwd"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(same_path(cwd.as_ref(), &feature));
    assert!(
        core.new_session_in(elsewhere.path()).await.is_err(),
        "not a Worktree of this Workspace"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn worktrees_added_or_removed_in_a_terminal_appear_and_disappear_live() {
    let repo = git_repo();
    let elsewhere = tempfile::tempdir().unwrap();
    let feature = elsewhere.path().join("feature");
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    core.open_workspace(repo.path()).await.unwrap();
    let mut events = core.subscribe();

    git(
        repo.path(),
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "feat/live",
            feature.to_str().unwrap(),
        ],
    );
    worktrees_until(&mut events, "the new worktree", |w| {
        w.iter().any(|w| same_path(&w.path, &feature))
    })
    .await;

    git(
        repo.path(),
        &["worktree", "remove", feature.to_str().unwrap()],
    );
    let after = worktrees_until(&mut events, "the worktree gone", |w| w.len() == 1).await;
    assert!(after[0].is_main);
}

#[tokio::test(flavor = "multi_thread")]
async fn worktree_paths_with_spaces_and_renamed_files_are_handled() {
    let repo = git_repo();
    let elsewhere = tempfile::tempdir().unwrap();
    let spaced = elsewhere.path().join("my feature");
    git(
        repo.path(),
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "feat/spaced",
            spaced.to_str().unwrap(),
        ],
    );
    std::fs::write(spaced.join("old name.txt"), "x").unwrap();
    git(&spaced, &["add", "old name.txt"]);
    commit(&spaced, "add a file");
    git(&spaced, &["mv", "old name.txt", "new name.txt"]);
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);

    core.open_workspace(repo.path()).await.unwrap();

    let feat = core
        .worktrees()
        .into_iter()
        .find(|w| w.branch.as_deref() == Some("feat/spaced"))
        .unwrap();
    assert!(same_path(&feat.path, &spaced));
    assert_eq!(feat.changed, 1, "a rename is one change");
}

#[tokio::test(flavor = "multi_thread")]
async fn hiding_all_tabs_ends_the_stream_and_output_counts_as_unread_again() {
    let repo = git_repo();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    core.open_workspace(repo.path()).await.unwrap();
    let mut events = core.subscribe();
    let session = core.new_session().await.unwrap();
    let mut stream = core.show_session(session).unwrap();
    stream.next().await.unwrap();

    // e.g. the user selected a Worktree that has no sessions
    core.hide_tabs();
    core.send_prompt(session, "while hidden").await.unwrap();
    states_until(&mut events, session, editor_core::SessionState::Idle).await;

    let ended =
        tokio::time::timeout(TIMEOUT, async { while stream.next().await.is_some() {} }).await;
    assert!(ended.is_ok(), "the hidden Tab's stream ended");
    assert_eq!(core.session_info(session).unwrap().unread, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn refreshing_picks_up_changes_without_the_watcher() {
    let repo = git_repo();
    let elsewhere = tempfile::tempdir().unwrap();
    let feature = elsewhere.path().join("feature");
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    core.open_workspace(repo.path()).await.unwrap();
    git(
        repo.path(),
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "feat/focus",
            feature.to_str().unwrap(),
        ],
    );

    // What the frontend calls when the window regains focus.
    core.refresh_worktrees().await;

    assert!(core
        .worktrees()
        .iter()
        .any(|w| same_path(&w.path, &feature)));
}
