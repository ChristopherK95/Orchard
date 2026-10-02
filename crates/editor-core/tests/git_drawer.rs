//! Ticket 18: the Git drawer's status, stage/unstage, commit/amend and discard, with real `git`.

mod support;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use editor_core::{
    CommitOutcome, CommitRequest, Core, CoreError, CoreEvent, GitFile, SessionState,
};
use serde_json::json;
use support::*;

/// A Workspace on `repo` (which gets an identity to commit as); its root.
async fn open(core: &Core, repo: &Path) -> PathBuf {
    git(repo, &["config", "user.name", "Test"]);
    git(repo, &["config", "user.email", "test@example.com"]);
    core.open_workspace(repo).await.unwrap().root
}

fn write(root: &Path, rel: &str, text: &str) {
    std::fs::write(root.join(rel), text).unwrap();
}

/// `a.rs` and `b.rs` committed.
fn with_files(repo: &Path) {
    write(repo, "a.rs", "fn a() {}\n");
    write(repo, "b.rs", "fn b() {}\n");
    git(repo, &["add", "."]);
    commit(repo, "files");
}

async fn files(core: &Core, root: &Path) -> Vec<GitFile> {
    core.git_status(root).await.unwrap().files
}

fn file<'a>(files: &'a [GitFile], path: &str) -> &'a GitFile {
    files
        .iter()
        .find(|f| f.path == path)
        .unwrap_or_else(|| panic!("{path} in {files:?}"))
}

/// A file's text, whatever line endings git checked it out with (`core.autocrlf`).
fn read(root: &Path, rel: &str) -> String {
    std::fs::read_to_string(root.join(rel))
        .unwrap()
        .replace("\r\n", "\n")
}

fn message(text: &str) -> CommitRequest {
    CommitRequest {
        message: text.into(),
        ..CommitRequest::default()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn status_shows_the_branch_and_what_is_staged_and_not() {
    let repo = git_repo();
    with_files(repo.path());
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    let root = open(&core, repo.path()).await;
    write(&root, "a.rs", "fn a() { 1 }\n");
    std::fs::remove_file(root.join("b.rs")).unwrap();
    write(&root, "new.rs", "fn new() {}\n");

    let status = core.git_status(&root).await.unwrap();
    assert_eq!(status.branch.as_deref(), Some("main"));
    assert_eq!(status.last_commit.unwrap().subject, "files");
    let files = status.files;
    assert_eq!(file(&files, "a.rs").unstaged.as_deref(), Some("M"));
    assert_eq!(file(&files, "b.rs").unstaged.as_deref(), Some("D"));
    assert_eq!(file(&files, "new.rs").unstaged.as_deref(), Some("?"));
    assert!(files.iter().all(|f| f.staged.is_none()));
}

#[tokio::test(flavor = "multi_thread")]
async fn whole_files_stage_and_unstage_one_at_a_time_or_all_at_once() {
    let repo = git_repo();
    with_files(repo.path());
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    let root = open(&core, repo.path()).await;
    write(&root, "a.rs", "fn a() { 1 }\n");
    std::fs::remove_file(root.join("b.rs")).unwrap();
    write(&root, "new file.rs", "fn new() {}\n");

    core.stage(&root, &["a.rs".into(), "b.rs".into()])
        .await
        .unwrap();
    let now = files(&core, &root).await;
    assert_eq!(
        (
            file(&now, "a.rs").staged.as_deref(),
            file(&now, "a.rs").unstaged.as_deref()
        ),
        (Some("M"), None)
    );
    assert_eq!(
        file(&now, "b.rs").staged.as_deref(),
        Some("D"),
        "a deletion stages too"
    );
    assert_eq!(file(&now, "new file.rs").staged, None);

    core.unstage(&root, &["a.rs".into()]).await.unwrap();
    let now = files(&core, &root).await;
    assert_eq!(
        (
            file(&now, "a.rs").staged.as_deref(),
            file(&now, "a.rs").unstaged.as_deref()
        ),
        (None, Some("M"))
    );

    core.stage_all(&root).await.unwrap();
    assert!(files(&core, &root)
        .await
        .iter()
        .all(|f| f.staged.is_some() && f.unstaged.is_none()));
    core.unstage_all(&root).await.unwrap();
    assert!(files(&core, &root).await.iter().all(|f| f.staged.is_none()));
}

#[tokio::test(flavor = "multi_thread")]
async fn commit_and_amend() {
    let repo = git_repo();
    with_files(repo.path());
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    let root = open(&core, repo.path()).await;
    write(&root, "a.rs", "fn a() { 1 }\n");
    core.stage_all(&root).await.unwrap();

    let outcome = core.commit(&root, message("Change a")).await.unwrap();
    let CommitOutcome::Committed { id } = outcome else {
        panic!("{outcome:?}")
    };
    let status = core.git_status(&root).await.unwrap();
    assert!(status.files.is_empty());
    let last = status.last_commit.unwrap();
    assert_eq!((last.id, last.subject.as_str()), (id, "Change a"));

    // Amend: the change goes into the last commit, with a new message.
    write(&root, "b.rs", "fn b() { 2 }\n");
    core.stage_all(&root).await.unwrap();
    let amend = CommitRequest {
        amend: true,
        ..message("Change a and b")
    };
    assert!(matches!(
        core.commit(&root, amend).await.unwrap(),
        CommitOutcome::Committed { .. }
    ));
    let status = core.git_status(&root).await.unwrap();
    assert_eq!(status.last_commit.unwrap().subject, "Change a and b");
    assert!(status.files.is_empty());
    let count = std::process::Command::new("git")
        .args(["rev-list", "--count", "HEAD"])
        .current_dir(&root)
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&count.stdout).trim(),
        "3",
        "init, files, the amended one"
    );

    // Nothing to commit, or no message: git's (or the core's) own error.
    assert!(core.commit(&root, message("Nothing")).await.is_err());
    write(&root, "a.rs", "fn a() { 3 }\n");
    core.stage_all(&root).await.unwrap();
    assert!(core.commit(&root, message("  ")).await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn amending_a_pushed_commit_asks_first() {
    let setup = RepoWithOrigin::new();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    let root = open(&core, &setup.repo()).await;
    write(&root, "a.rs", "fn a() {}\n");
    core.stage_all(&root).await.unwrap();
    let amend = CommitRequest {
        amend: true,
        ..message("Amended")
    };

    assert_eq!(
        core.commit(&root, amend.clone()).await.unwrap(),
        CommitOutcome::AlreadyPushed
    );
    assert_eq!(
        core.git_status(&root).await.unwrap().files.len(),
        1,
        "nothing committed"
    );
    let anyway = CommitRequest {
        even_if_pushed: true,
        ..amend
    };
    assert!(matches!(
        core.commit(&root, anyway).await.unwrap(),
        CommitOutcome::Committed { .. }
    ));

    // Not pushed (a new local commit): no question.
    write(&root, "b.rs", "fn b() {}\n");
    core.stage_all(&root).await.unwrap();
    core.commit(&root, message("Local")).await.unwrap();
    write(&root, "c.rs", "fn c() {}\n");
    core.stage_all(&root).await.unwrap();
    let amend = CommitRequest {
        amend: true,
        ..message("Local, amended")
    };
    assert!(matches!(
        core.commit(&root, amend).await.unwrap(),
        CommitOutcome::Committed { .. }
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn discard_restores_changed_files_and_removes_new_ones() {
    let repo = git_repo();
    with_files(repo.path());
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    let root = open(&core, repo.path()).await;
    write(&root, "a.rs", "fn a() { mine }\n");
    core.stage(&root, &["a.rs".into()]).await.unwrap();
    write(&root, "a.rs", "fn a() { more }\n"); // staged and unstaged changes both
    std::fs::remove_file(root.join("b.rs")).unwrap();
    write(&root, "untracked.rs", "x\n");
    write(&root, "added.rs", "y\n");
    core.stage(&root, &["added.rs".into()]).await.unwrap();

    for path in ["a.rs", "b.rs", "untracked.rs", "added.rs"] {
        core.discard(&root, path).await.unwrap();
    }
    assert_eq!(read(&root, "a.rs"), "fn a() {}\n");
    assert_eq!(read(&root, "b.rs"), "fn b() {}\n");
    assert!(!root.join("untracked.rs").exists());
    assert!(!root.join("added.rs").exists());
    assert!(files(&core, &root).await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn discarding_a_rename_brings_the_original_back() {
    let repo = git_repo();
    with_files(repo.path());
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    let root = open(&core, repo.path()).await;
    git(&root, &["mv", "a.rs", "renamed.rs"]);
    assert_eq!(
        file(&files(&core, &root).await, "renamed.rs")
            .renamed_from
            .as_deref(),
        Some("a.rs")
    );

    core.discard(&root, "renamed.rs").await.unwrap();
    assert!(root.join("a.rs").exists() && !root.join("renamed.rs").exists());
    assert!(files(&core, &root).await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn committing_while_a_session_is_working_asks_first() {
    let repo = git_repo();
    with_files(repo.path());
    let fake = FakeAgent::new(&json!({ "turns": [{ "untilCancelled": true }] }).to_string());
    let core = core_with(&fake);
    let root = open(&core, repo.path()).await;
    let session = core.new_session().await.unwrap();
    let mut states = core.subscribe();
    core.send_prompt(session, "work").await.unwrap();
    states_until(&mut states, session, SessionState::Working).await;
    write(&root, "a.rs", "fn a() { 1 }\n");
    core.stage_all(&root).await.unwrap();

    let outcome = core.commit(&root, message("Mid-turn")).await.unwrap();
    assert_eq!(
        outcome,
        CommitOutcome::SessionsWorking {
            sessions: vec!["Session 1".into()]
        }
    );
    assert_eq!(files(&core, &root).await.len(), 1, "nothing committed");
    let anyway = CommitRequest {
        even_if_working: true,
        ..message("Mid-turn")
    };
    assert!(matches!(
        core.commit(&root, anyway).await.unwrap(),
        CommitOutcome::Committed { .. }
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_status_is_live_soon_after_the_last_change_of_a_burst() {
    let repo = git_repo();
    with_files(repo.path());
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    let root = open(&core, repo.path()).await;
    core.show_worktree(&root).await.unwrap();
    let mut events = core.subscribe();

    // An Agent writing a new file every 50 ms for half a second.
    for i in 0..10 {
        write(&root, &format!("new{i}.rs"), "fn x() {}\n");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let last = Instant::now();
    while events.try_recv().is_ok() {} // (what came during the burst)
    next_event(&mut events, "the git status to change", |e| match e {
        CoreEvent::GitStatusChanged { worktree } if worktree == root => Some(()),
        _ => None,
    })
    .await;
    // The watcher waits 200 ms for quiet, then reads what changed: a little more on a slow machine.
    assert!(
        last.elapsed() < Duration::from_millis(700),
        "{:?}",
        last.elapsed()
    );
    assert_eq!(files(&core, &root).await.len(), 10);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_file_edited_after_staging_unstages_and_discards() {
    let repo = git_repo();
    with_files(repo.path());
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    let root = open(&core, repo.path()).await;
    write(&root, "n.rs", "one\n");
    core.stage(&root, &["n.rs".into()]).await.unwrap();
    write(&root, "n.rs", "two\n"); // AM

    core.discard(&root, "n.rs").await.unwrap();
    assert!(!root.join("n.rs").exists());
    assert!(files(&core, &root).await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn before_the_first_commit_files_still_unstage() {
    let repo = tempfile::tempdir().unwrap();
    git(repo.path(), &["init", "--quiet", "-b", "main"]);
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    let root = open(&core, repo.path()).await;
    write(&root, "a.rs", "one\n");
    write(&root, "b.rs", "one\n");
    core.stage_all(&root).await.unwrap();
    write(&root, "a.rs", "two\n"); // AM

    core.unstage(&root, &["a.rs".into()]).await.unwrap();
    let now = files(&core, &root).await;
    assert_eq!(file(&now, "a.rs").staged, None);
    assert_eq!(file(&now, "b.rs").staged.as_deref(), Some("A"));
    core.unstage_all(&root).await.unwrap();
    assert!(files(&core, &root).await.iter().all(|f| f.staged.is_none()));
    assert_eq!(read(&root, "a.rs"), "two\n", "the files themselves stay");

    core.stage(&root, &["b.rs".into()]).await.unwrap();
    core.discard(&root, "b.rs").await.unwrap();
    assert!(!root.join("b.rs").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rename_unstages_as_one() {
    let repo = git_repo();
    with_files(repo.path());
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    let root = open(&core, repo.path()).await;
    git(&root, &["mv", "a.rs", "renamed.rs"]);

    core.unstage(&root, &["renamed.rs".into()]).await.unwrap();
    let now = files(&core, &root).await;
    assert!(now.iter().all(|f| f.staged.is_none()), "{now:?}");
    assert_eq!(file(&now, "a.rs").unstaged.as_deref(), Some("D"));
    assert_eq!(file(&now, "renamed.rs").unstaged.as_deref(), Some("?"));
}

#[tokio::test(flavor = "multi_thread")]
async fn bad_requests_are_refused_with_a_reason() {
    let repo = git_repo();
    with_files(repo.path());
    let elsewhere = tempfile::tempdir().unwrap();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    let root = open(&core, repo.path()).await;
    write(&root, "a.rs", "fn a() { 1 }\n");

    // No files means none, never all.
    assert!(core.stage(&root, &[]).await.is_err());
    assert!(files(&core, &root).await.iter().all(|f| f.staged.is_none()));
    assert!(matches!(
        core.git_status(elsewhere.path()).await,
        Err(CoreError::UnknownWorktree(_))
    ));
    // Nothing staged: git's own words (from stdout), not an empty error.
    let Err(err) = core.commit(&root, message("Nothing")).await else {
        panic!("committed nothing")
    };
    assert!(!err.to_string().trim().is_empty(), "{err:?}");
}
