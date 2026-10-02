//! Ticket 09: removing a Worktree without losing work, with real `git` and a bare origin.

mod support;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use editor_core::{
    Core, CoreError, CoreEvent, NewWorktree, RemovalCheck, RemoveWorktree, SessionState,
};
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

fn branch_exists(repo: &Path, branch: &str) -> bool {
    let out = Command::new("git")
        .args(["branch", "--list", branch])
        .current_dir(repo)
        .output()
        .expect("run git");
    !String::from_utf8_lossy(&out.stdout).trim().is_empty()
}

fn keep_branch() -> RemoveWorktree {
    RemoveWorktree {
        discard: None,
        delete_branch: false,
    }
}

fn delete_branch() -> RemoveWorktree {
    RemoveWorktree {
        discard: None,
        delete_branch: true,
    }
}

/// "Discard and remove", as the dialog sends it after showing `check`.
fn discarding(check: &RemovalCheck, delete_branch: bool) -> RemoveWorktree {
    RemoveWorktree {
        discard: Some(check.fingerprint.clone()),
        delete_branch,
    }
}

fn refused(result: Result<editor_core::RemovedWorktree, CoreError>) -> bool {
    matches!(result, Err(CoreError::WouldDiscard(_)))
}

#[tokio::test(flavor = "multi_thread")]
async fn a_clean_pushed_worktree_needs_only_a_single_confirmation() {
    let setup = RepoWithOrigin::new();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    core.open_workspace(&setup.repo()).await.unwrap();
    let path = new_worktree(&core, "agent/clean").await;

    let check = core.removal_check(&path).await.unwrap();
    assert!(check.changes.is_empty() && check.unpushed.is_empty());
    assert!(check.merged, "nothing on it the Base doesn't have");
    assert!(!check.discard_to_remove && !check.discard_to_delete_branch);

    let removed = core.remove_worktree(&path, delete_branch()).await.unwrap();
    assert_eq!(removed.warning, None);
    assert!(!path.exists());
    assert!(core.worktrees().iter().all(|w| w.path != path));
    assert!(!branch_exists(&setup.repo(), "agent/clean"));
}

#[tokio::test(flavor = "multi_thread")]
async fn uncommitted_changes_are_listed_and_need_discard_and_remove() {
    let setup = RepoWithOrigin::new();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    core.open_workspace(&setup.repo()).await.unwrap();
    let path = new_worktree(&core, "agent/dirty").await;
    std::fs::write(path.join("notes.txt"), "work in progress").unwrap();

    let check = core.removal_check(&path).await.unwrap();
    assert!(
        check.changes.iter().any(|c| c.contains("notes.txt")),
        "{:?}",
        check.changes
    );
    assert!(check.discard_to_remove);

    assert!(refused(core.remove_worktree(&path, keep_branch()).await));
    assert!(path.join("notes.txt").exists(), "nothing was touched");

    core.remove_worktree(&path, discarding(&check, false))
        .await
        .unwrap();
    assert!(!path.exists());
    assert!(
        branch_exists(&setup.repo(), "agent/dirty"),
        "the branch stays unless asked"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn discard_covers_only_the_work_that_was_shown() {
    let setup = RepoWithOrigin::new();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    core.open_workspace(&setup.repo()).await.unwrap();
    let path = new_worktree(&core, "agent/still-editing").await;
    std::fs::write(path.join("notes.txt"), "first draft").unwrap();
    let shown = core.removal_check(&path).await.unwrap();

    // The same file edited again after the dialog opened: same count, different work.
    std::fs::write(path.join("notes.txt"), "a much longer second draft").unwrap();
    assert!(refused(
        core.remove_worktree(&path, discarding(&shown, false)).await
    ));
    assert!(path.join("notes.txt").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn ignored_files_are_listed() {
    let setup = RepoWithOrigin::new();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    core.open_workspace(&setup.repo()).await.unwrap();
    let path = new_worktree(&core, "agent/ignored").await;
    std::fs::write(path.join(".gitignore"), ".env\n").unwrap();
    git(&path, &["add", ".gitignore"]);
    commit(&path, "ignore .env");
    std::fs::write(path.join(".env"), "SECRET=1").unwrap();

    let check = core.removal_check(&path).await.unwrap();
    assert_eq!(check.ignored, vec![".env".to_owned()]);
    assert!(check.changes.is_empty(), "ignored files aren't changes");
}

#[tokio::test(flavor = "multi_thread")]
async fn unpushed_commits_are_listed_and_only_deleting_the_branch_would_lose_them() {
    let setup = RepoWithOrigin::new();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    core.open_workspace(&setup.repo()).await.unwrap();
    let path = new_worktree(&core, "agent/unpushed").await;
    commit(&path, "the Agent's work");

    let check = core.removal_check(&path).await.unwrap();
    assert_eq!(check.unpushed.len(), 1);
    assert_eq!(check.unpushed[0].subject, "the Agent's work");
    assert!(!check.merged);
    assert!(!check.discard_to_remove, "the branch keeps the commit");
    assert!(check.discard_to_delete_branch);

    assert!(refused(core.remove_worktree(&path, delete_branch()).await));
    assert!(path.exists());

    core.remove_worktree(&path, keep_branch()).await.unwrap();
    assert!(!path.exists());
    assert!(branch_exists(&setup.repo(), "agent/unpushed"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pushed_but_unmerged_branch_can_be_deleted_without_discarding() {
    let setup = RepoWithOrigin::new();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    core.open_workspace(&setup.repo()).await.unwrap();
    let path = new_worktree(&core, "agent/pushed").await;
    commit(&path, "in review");
    git(&path, &["push", "--quiet", "origin", "HEAD:agent/pushed"]);

    let check = core.removal_check(&path).await.unwrap();
    assert!(!check.merged, "not starting ticked");
    assert!(check.unpushed.is_empty(), "origin has it");
    assert!(!check.discard_to_delete_branch);

    core.remove_worktree(&path, delete_branch()).await.unwrap();
    assert!(!branch_exists(&setup.repo(), "agent/pushed"));
}

#[tokio::test(flavor = "multi_thread")]
async fn commits_on_a_detached_head_need_discard_and_remove() {
    let setup = RepoWithOrigin::new();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    core.open_workspace(&setup.repo()).await.unwrap();
    let path = new_worktree(&core, "agent/detached").await;
    git(&path, &["checkout", "--quiet", "--detach"]);
    commit(&path, "nobody keeps this");
    core.refresh_worktrees().await;

    let check = core.removal_check(&path).await.unwrap();
    assert_eq!(check.branch, None);
    assert_eq!(check.unpushed[0].subject, "nobody keeps this");
    assert!(check.discard_to_remove);
    assert!(refused(core.remove_worktree(&path, keep_branch()).await));

    core.remove_worktree(&path, discarding(&check, false))
        .await
        .unwrap();
    assert!(!path.exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_branch_merged_into_its_base_is_offered_for_deletion() {
    let setup = RepoWithOrigin::new();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    core.open_workspace(&setup.repo()).await.unwrap();
    let path = new_worktree(&core, "agent/merged").await;
    commit(&path, "landed");
    git(&path, &["push", "--quiet", "origin", "HEAD:main"]);

    let check = core.removal_check(&path).await.unwrap();
    assert!(check.merged);
    assert!(check.unpushed.is_empty());

    core.remove_worktree(&path, delete_branch()).await.unwrap();
    assert!(!branch_exists(&setup.repo(), "agent/merged"));
}

#[tokio::test(flavor = "multi_thread")]
async fn without_origin_a_detached_main_checkout_is_not_mistaken_for_the_base() {
    // No origin, main checkout detached: the Base falls back to `HEAD`, which must mean the main
    // checkout's HEAD, not the Worktree's (or the Worktree's own commits would look merged).
    let repo = git_repo();
    git(repo.path(), &["checkout", "--quiet", "--detach"]);
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    core.open_workspace(repo.path()).await.unwrap();
    let created = core
        .create_worktree(NewWorktree::NewBranch {
            name: "agent/own-work".into(),
            start_point: Some("HEAD".into()),
        })
        .await
        .unwrap()
        .worktree
        .path;
    commit(&created, "only here");

    let check = core.removal_check(&created).await.unwrap();
    assert!(!check.merged);
    assert_eq!(check.unpushed_count, 1);
    core.remove_worktree(&created, keep_branch()).await.unwrap();
    // `git_repo` lives in the system temp folder, so its Worktrees go next to it: tidy up.
    let _ = std::fs::remove_dir_all(created.parent().unwrap());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_main_checkout_cannot_be_removed() {
    let setup = RepoWithOrigin::new();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    let root = core.open_workspace(&setup.repo()).await.unwrap().root;

    assert!(matches!(
        core.removal_check(&root).await,
        Err(CoreError::MainCheckout)
    ));
    let everything = RemoveWorktree {
        discard: Some("anything".into()),
        delete_branch: true,
    };
    assert!(matches!(
        core.remove_worktree(&root, everything).await,
        Err(CoreError::MainCheckout)
    ));
    assert!(root.join(".git").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn running_sessions_are_stopped_before_removal() {
    let setup = RepoWithOrigin::new();
    let fake = FakeAgent::new(r#"{"turns":[{"permission":{"title":"Edit a.rs","kind":"edit"}}]}"#);
    let core = core_with(&fake);
    core.open_workspace(&setup.repo()).await.unwrap();
    let path = new_worktree(&core, "agent/busy").await;
    let session = core.new_session_in(&path).await.unwrap();
    let elsewhere = core.new_session().await.unwrap();
    let mut events = core.subscribe();
    core.send_prompt(session, "edit it").await.unwrap();
    states_until(&mut events, session, SessionState::NeedsYou).await;

    let check = core.removal_check(&path).await.unwrap();
    assert_eq!(check.sessions, vec![session]);

    core.remove_worktree(&path, keep_branch()).await.unwrap();
    next_event(&mut events, "the session to close", |e| match e {
        CoreEvent::SessionClosed { session_id } if session_id == session => Some(()),
        _ => None,
    })
    .await;
    assert!(matches!(
        core.session_info(session),
        Err(CoreError::UnknownSession)
    ));
    let closed = fake.received("session/close");
    assert_eq!(closed.len(), 1);
    assert_eq!(
        closed[0]["params"]["sessionId"],
        fake.received("session/prompt")[0]["params"]["sessionId"]
    );
    let order: Vec<_> = fake
        .log()
        .into_iter()
        .filter_map(|m| m["closedWhileCwdExists"].as_bool())
        .collect();
    assert_eq!(order, vec![true], "closed while the folder still existed");
    assert!(!path.exists());
    assert!(
        core.session_info(elsewhere).is_ok(),
        "other Worktrees' sessions are untouched"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_folder_still_in_use_for_a_moment_is_removed_once_released() {
    let setup = RepoWithOrigin::new();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    core.open_workspace(&setup.repo()).await.unwrap();
    let path = new_worktree(&core, "agent/in-use").await;
    // Something outside the editor (say, a terminal) sits in the folder for a moment. On Windows
    // git unregisters the Worktree and empties it, but can't delete the folder until it's gone.
    let release = setup.root().join("release");
    let mut holder = Command::new(fake_agent_path())
        .args(["--print", "holding", "--until"])
        .arg(&release)
        .current_dir(&path)
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let releasing = std::thread::spawn({
        let release = release.clone();
        move || {
            std::thread::sleep(Duration::from_millis(800));
            std::fs::write(&release, "").unwrap();
        }
    });

    core.remove_worktree(&path, keep_branch()).await.unwrap();
    assert!(!path.exists());
    assert!(core.worktrees().iter().all(|w| w.path != path));
    releasing.join().unwrap();
    holder.wait().unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_running_setup_is_stopped_with_everything_it_started() {
    let setup = RepoWithOrigin::new();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    // The shell runs the fake agent, which waits for a file that never comes and keeps a heartbeat
    // file ticking meanwhile. (If it survived, cargo would wait on it after the tests: a regression
    // here can show up as a hang on Windows.)
    let never = setup.root().join("never");
    let beat = setup.root().join("beat");
    let command = format!(
        "{} --beat {}",
        print_then_wait("waiting", &never),
        beat.display().to_string().replace('\\', "/")
    );
    let settings = repo_settings(
        &origin_url(&setup.repo()),
        &format!("setup = [{}]", toml_literal(&command)),
    );
    let core = core_with_settings(&fake, &settings);
    core.open_workspace(&setup.repo()).await.unwrap();
    let mut events = core.subscribe();
    let path = new_worktree(&core, "agent/setting-up").await;
    next_event(&mut events, "setup output", |e| match e {
        CoreEvent::SetupOutput { text, .. } if text.contains("waiting") => Some(()),
        _ => None,
    })
    .await;
    eventually("the heartbeat", || beat.exists()).await;

    core.remove_worktree(&path, keep_branch()).await.unwrap();
    assert!(!path.exists());
    assert!(core.setup(&path).is_none());
    let last = std::fs::read_to_string(&beat).unwrap_or_default();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        std::fs::read_to_string(&beat).unwrap_or_default(),
        last,
        "the setup command's child is gone"
    );
    assert!(fake.received("session/new").is_empty());
}
