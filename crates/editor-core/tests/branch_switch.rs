//! Ticket 20: switching a Worktree's branch from the Git drawer (blocked while a session there is
//! mid-turn; never to a branch checked out elsewhere), and a merge or rebase in progress shown with
//! its conflicted files and aborted. Real `git`.

mod support;

use std::path::{Path, PathBuf};

use editor_core::{Core, CoreError, GitOperation, NewWorktree, SessionState};
use serde_json::json;
use support::*;

async fn open(core: &Core, repo: &Path) -> PathBuf {
    git(repo, &["config", "user.name", "Test"]);
    git(repo, &["config", "user.email", "test@example.com"]);
    core.open_workspace(repo).await.unwrap().root
}

fn write(root: &Path, rel: &str, text: &str) {
    std::fs::write(root.join(rel), text).unwrap();
}

fn commit_file(root: &Path, rel: &str, text: &str, message: &str) {
    write(root, rel, text);
    git(root, &["add", rel]);
    commit(root, message);
}

fn branch_of(core: &Core, worktree: &Path) -> Option<String> {
    core.worktrees()
        .into_iter()
        .find(|w| w.path == worktree)
        .unwrap()
        .branch
}

#[tokio::test(flavor = "multi_thread")]
async fn switching_keeps_the_worktrees_sessions_with_it() {
    let repo = git_repo();
    git(repo.path(), &["branch", "feature"]);
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    let root = open(&core, repo.path()).await;
    let session = core.new_session().await.unwrap();

    core.switch_branch(&root, "feature").await.unwrap();
    assert_eq!(branch_of(&core, &root).as_deref(), Some("feature"));
    assert_eq!(
        core.git_status(&root).await.unwrap().branch.as_deref(),
        Some("feature")
    );
    let info = core.session_info(session).unwrap();
    assert_eq!(info.worktree, root, "bound to the Worktree, not the branch");
    assert_eq!(core.sessions().len(), 1);
    // And it carries on there.
    let mut states = core.subscribe();
    core.send_prompt(session, "still here?").await.unwrap();
    states_until(&mut states, session, SessionState::Idle).await;
    assert_eq!(
        fake.received("session/prompt").len(),
        1,
        "the same session took it"
    );
    assert_eq!(core.session_info(session).unwrap().worktree, root);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_remote_branch_gets_a_local_one_tracking_it() {
    let setup = RepoWithOrigin::new();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    let root = open(&core, &setup.repo()).await;
    let theirs = setup.push_to_origin("theirs", "a colleague's branch");
    git(&root, &["fetch", "--quiet"]);

    core.switch_branch(&root, "origin/theirs").await.unwrap();
    let status = core.git_status(&root).await.unwrap();
    assert_eq!(status.branch.as_deref(), Some("theirs"));
    assert_eq!(status.upstream.as_deref(), Some("origin/theirs"));
    assert_eq!(rev_parse(&root, "HEAD"), theirs);
}

#[tokio::test(flavor = "multi_thread")]
async fn switching_waits_while_a_session_is_mid_turn() {
    let repo = git_repo();
    git(repo.path(), &["branch", "feature"]);
    let fake = FakeAgent::new(&json!({ "turns": [{ "untilCancelled": true }] }).to_string());
    let core = core_with(&fake);
    let root = open(&core, repo.path()).await;
    let session = core.new_session().await.unwrap();
    let mut states = core.subscribe();
    core.send_prompt(session, "work").await.unwrap();
    states_until(&mut states, session, SessionState::Working).await;

    assert_eq!(
        core.git_status(&root).await.unwrap().mid_turn,
        ["Session 1"]
    );
    assert!(matches!(
        core.switch_branch(&root, "feature").await,
        Err(CoreError::SessionsWorking(names)) if names == ["Session 1"]
    ));
    assert_eq!(branch_of(&core, &root).as_deref(), Some("main"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_branch_checked_out_in_another_worktree_cant_be_switched_to() {
    let repo = git_repo();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    let root = open(&core, repo.path()).await;
    let other = core
        .create_worktree(NewWorktree::NewBranch {
            name: "agent/busy".into(),
            start_point: Some("main".into()),
        })
        .await
        .unwrap()
        .worktree
        .path;

    assert!(matches!(
        core.switch_branch(&root, "agent/busy").await,
        Err(CoreError::BranchCheckedOut { worktree, .. }) if worktree == other
    ));
    assert!(matches!(
        core.switch_branch(&root, "no/such").await,
        Err(CoreError::UnknownBranch(_))
    ));
}

/// `main` and `other` both change `f.txt`: merging or rebasing one onto the other conflicts.
fn conflicting_branches(root: &Path) {
    commit_file(root, "f.txt", "base\n", "base");
    git(root, &["checkout", "--quiet", "-b", "other"]);
    commit_file(root, "f.txt", "theirs\n", "theirs");
    git(root, &["checkout", "--quiet", "main"]);
    commit_file(root, "f.txt", "ours\n", "ours");
}

/// Runs git, which is expected to fail (a conflict).
fn git_fails(dir: &Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .args(["-c", "user.name=Test", "-c", "user.email=test@example.com"])
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap()
        .status;
    assert!(!status.success(), "git {args:?} was meant to conflict");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_conflicted_merge_is_shown_and_aborts() {
    let repo = git_repo();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    let root = open(&core, repo.path()).await;
    conflicting_branches(&root);
    let before = rev_parse(&root, "HEAD");
    git_fails(&root, &["merge", "--quiet", "other"]); // (an Agent, say)

    let status = core.git_status(&root).await.unwrap();
    assert_eq!(status.operation, Some(GitOperation::Merge));
    let conflicted: Vec<&str> = status
        .files
        .iter()
        .filter(|f| f.conflicted)
        .map(|f| f.path.as_str())
        .collect();
    assert_eq!(conflicted, ["f.txt"]);

    core.abort_operation(&root).await.unwrap();
    let status = core.git_status(&root).await.unwrap();
    assert_eq!(status.operation, None);
    assert!(status.files.is_empty());
    assert_eq!(rev_parse(&root, "HEAD"), before);
    assert!(matches!(
        core.abort_operation(&root).await,
        Err(CoreError::NothingToAbort)
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_conflicted_rebase_is_shown_and_aborts() {
    let repo = git_repo();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    let root = open(&core, repo.path()).await;
    conflicting_branches(&root);
    let before = rev_parse(&root, "HEAD");
    git_fails(&root, &["rebase", "--quiet", "other"]);

    let status = core.git_status(&root).await.unwrap();
    assert_eq!(status.operation, Some(GitOperation::Rebase));
    assert!(status
        .files
        .iter()
        .any(|f| f.path == "f.txt" && f.conflicted));

    core.abort_operation(&root).await.unwrap();
    let status = core.git_status(&root).await.unwrap();
    assert_eq!(
        (status.operation, status.branch.as_deref()),
        (None, Some("main"))
    );
    assert_eq!(rev_parse(&root, "HEAD"), before);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_remote_branch_with_a_local_one_uses_it_or_says_where_it_is() {
    let setup = RepoWithOrigin::new();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    let root = open(&core, &setup.repo()).await;
    setup.push_to_origin("theirs", "a colleague's branch");
    git(&root, &["fetch", "--quiet"]);
    git(&root, &["branch", "--quiet", "theirs", "origin/theirs"]); // a local one, free

    core.switch_branch(&root, "origin/theirs").await.unwrap();
    assert_eq!(branch_of(&core, &root).as_deref(), Some("theirs"));

    // Checked out in another Worktree now: the remote row leads there too.
    core.switch_branch(&root, "main").await.unwrap();
    let other = core
        .create_worktree(NewWorktree::ExistingBranch {
            name: "theirs".into(),
        })
        .await
        .unwrap()
        .worktree
        .path;
    assert!(matches!(
        core.switch_branch(&root, "origin/theirs").await,
        Err(CoreError::BranchCheckedOut { worktree, .. }) if worktree == other
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_waiting_on_a_permission_card_blocks_a_switch_too() {
    let repo = git_repo();
    git(repo.path(), &["branch", "feature"]);
    let turn = json!({ "permission": { "title": "Edit", "kind": "edit",
        "diff": { "path": "a.rs", "oldText": "", "newText": "x" } } });
    let fake = FakeAgent::new(&json!({ "turns": [turn] }).to_string());
    let core = core_with(&fake);
    let root = open(&core, repo.path()).await;
    let session = core.new_session().await.unwrap();
    let mut states = core.subscribe();
    core.send_prompt(session, "edit").await.unwrap();
    states_until(&mut states, session, SessionState::NeedsYou).await;

    assert!(matches!(
        core.switch_branch(&root, "feature").await,
        Err(CoreError::SessionsWorking(_))
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_merge_started_elsewhere_reaches_the_drawer_live() {
    let repo = git_repo();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    let root = open(&core, repo.path()).await;
    conflicting_branches(&root);
    core.show_worktree(&root).await.unwrap();
    let mut events = core.subscribe();

    git_fails(&root, &["merge", "--quiet", "other"]); // (an Agent, or a terminal)
    next_event(&mut events, "the git status to change", |e| match e {
        editor_core::CoreEvent::GitStatusChanged { worktree } if worktree == root => Some(()),
        _ => None,
    })
    .await;
    assert_eq!(
        core.git_status(&root).await.unwrap().operation,
        Some(GitOperation::Merge)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cherry_pick_of_several_commits_between_steps_is_shown_and_aborts() {
    let repo = git_repo();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    let root = open(&core, repo.path()).await;
    commit_file(&root, "f.txt", "base\n", "base");
    git(&root, &["checkout", "--quiet", "-b", "picks"]);
    commit_file(&root, "f.txt", "one\n", "one");
    commit_file(&root, "g.txt", "two\n", "two");
    git(&root, &["checkout", "--quiet", "main"]);
    commit_file(&root, "f.txt", "ours\n", "ours");
    let before = rev_parse(&root, "HEAD");
    git_fails(&root, &["cherry-pick", "main..picks"]); // conflicts on the first
                                                       // Resolved and committed by hand: only the sequencer says it's still going.
    write(&root, "f.txt", "resolved\n");
    git(&root, &["add", "f.txt"]);
    git(
        &root,
        &["-c", "core.editor=true", "commit", "--quiet", "--no-edit"],
    );
    assert!(!root.join(".git").join("CHERRY_PICK_HEAD").exists());

    let status = core.git_status(&root).await.unwrap();
    assert_eq!(status.operation, Some(GitOperation::CherryPick));
    core.abort_operation(&root).await.unwrap();
    assert_eq!(core.git_status(&root).await.unwrap().operation, None);
    // (git keeps the commit made by hand: it doesn't rewind past a HEAD that moved.)
    assert_ne!(rev_parse(&root, "HEAD"), before);
}
