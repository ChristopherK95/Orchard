//! The Workspace picker's "New repository…": a new folder made a git repository, then opened.

mod support;

use std::process::Command;

use editor_core::{Core, NewWorktree};
use support::*;

const ECHO: &str = r#"{"turns":[]}"#;

/// The first commit needs an identity, and the test machine's git may have none. (Every test here
/// sets the same values, so setting them from parallel tests is harmless.)
fn with_git_identity() {
    for (key, value) in [
        ("GIT_AUTHOR_NAME", "Test"),
        ("GIT_AUTHOR_EMAIL", "test@example.com"),
        ("GIT_COMMITTER_NAME", "Test"),
        ("GIT_COMMITTER_EMAIL", "test@example.com"),
    ] {
        unsafe { std::env::set_var(key, value) };
    }
}

fn core(fake: &FakeAgent) -> Core {
    with_git_identity();
    core_with_state(fake)
}

fn commits(repo: &std::path::Path) -> usize {
    let out = Command::new("git")
        .args(["rev-list", "--count", "HEAD"])
        .current_dir(repo)
        .output()
        .expect("run git");
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .unwrap_or(0)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_repository_has_a_first_commit_and_opens_as_a_workspace() {
    let fake = FakeAgent::new(ECHO);
    let core = core(&fake);
    let location = tempfile::tempdir().unwrap();

    let dir = core
        .create_repository(location.path(), " fresh ")
        .await
        .unwrap();
    assert_eq!(dir.file_name().unwrap(), "fresh");
    assert!(dir.join(".git").is_dir());
    assert_eq!(commits(&dir), 1);

    let workspace = core.open_workspace(&dir).await.unwrap();
    assert_eq!(workspace.name, "fresh");
    // (The first commit is what a new Worktree's branch starts from.)
    core.create_worktree(NewWorktree::NewBranch {
        name: "feature".into(),
        start_point: None,
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_empty_folder_already_there_is_used() {
    let fake = FakeAgent::new(ECHO);
    let core = core(&fake);
    let location = tempfile::tempdir().unwrap();
    std::fs::create_dir(location.path().join("empty")).unwrap();

    let dir = core
        .create_repository(location.path(), "empty")
        .await
        .unwrap();
    assert_eq!(commits(&dir), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_folder_with_files_or_a_bad_name_is_refused_and_left_alone() {
    let fake = FakeAgent::new(ECHO);
    let core = core(&fake);
    let location = tempfile::tempdir().unwrap();
    let full = location.path().join("full");
    std::fs::create_dir(&full).unwrap();
    std::fs::write(full.join("notes.txt"), "keep me").unwrap();

    assert!(core
        .create_repository(location.path(), "full")
        .await
        .is_err());
    assert!(!full.join(".git").exists());
    assert_eq!(
        std::fs::read_to_string(full.join("notes.txt")).unwrap(),
        "keep me"
    );

    for name in ["", "  ", "a/b", r"a\b", "..", "what?"] {
        assert!(
            core.create_repository(location.path(), name).await.is_err(),
            "{name:?} is refused"
        );
    }
    let missing = location.path().join("missing");
    assert!(core.create_repository(&missing, "x").await.is_err());
    assert_eq!(std::fs::read_dir(location.path()).unwrap().count(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_repository_has_no_pull_requests_rather_than_github_errors() {
    let fake = FakeAgent::new(ECHO);
    let core = core(&fake);
    let location = tempfile::tempdir().unwrap();
    let dir = core
        .create_repository(location.path(), "fresh")
        .await
        .unwrap();
    core.open_workspace(&dir).await.unwrap();

    // (With no remote, the GitHub CLI isn't asked: it would only say "no git remotes found".)
    assert!(core.my_pull_requests(&dir).await.unwrap().is_empty());
    assert!(core.review_requests(&dir).await.unwrap().is_empty());
    assert!(core.open_pull_request(&dir).await.unwrap().is_none());
    assert!(core
        .pull_request_details(&dir, None)
        .await
        .unwrap()
        .is_none());
    let statistics = core.statistics(&dir, 30).await.unwrap();
    assert_eq!(statistics.days, 30);
    assert_eq!(statistics.still_waiting, 0);
}
