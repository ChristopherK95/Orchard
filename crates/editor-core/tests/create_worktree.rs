//! Ticket 07: creating a Worktree from the editor, with real `git` and a bare origin.

mod support;

use editor_core::{CoreError, NewWorktree};
use support::*;

#[tokio::test(flavor = "multi_thread")]
async fn defaults_create_an_agent_branch_from_freshly_fetched_origin_next_to_the_repo() {
    let setup = RepoWithOrigin::new();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    core.open_workspace(&setup.repo()).await.unwrap();
    // origin moves on after the clone; only a fetch can see this commit
    let newest = setup.push_to_origin("main", "landed upstream");

    let name = core.suggest_branch_name().await.unwrap();
    assert!(name.starts_with("agent/"), "{name}");
    let created = core
        .create_worktree(NewWorktree::NewBranch {
            name: name.clone(),
            start_point: None,
        })
        .await
        .unwrap()
        .worktree;

    let slug = name.replace('/', "-");
    let expected = setup.root().join("repo.worktrees").join(&slug);
    assert_eq!(canonical(&created.path), canonical(&expected));
    assert_eq!(created.branch.as_deref(), Some(name.as_str()));
    assert_eq!(
        rev_parse(&created.path, "HEAD"),
        newest,
        "based on the fetched origin/main"
    );
    assert!(
        core.worktrees().iter().any(|w| w.path == created.path),
        "listed straight away"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_different_base_can_be_chosen() {
    let setup = RepoWithOrigin::new();
    commit(&setup.repo(), "local only");
    let local_head = rev_parse(&setup.repo(), "HEAD");
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    core.open_workspace(&setup.repo()).await.unwrap();

    let created = core
        .create_worktree(NewWorktree::NewBranch {
            name: "agent/from-local".into(),
            start_point: Some("main".into()),
        })
        .await
        .unwrap()
        .worktree;

    assert_eq!(rev_parse(&created.path, "HEAD"), local_head);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_existing_local_or_remote_branch_can_be_checked_out() {
    let setup = RepoWithOrigin::new();
    git(&setup.repo(), &["branch", "feat/local"]);
    let remote_tip = setup.push_to_origin("feat/remote", "a colleague's work");
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    core.open_workspace(&setup.repo()).await.unwrap();

    let local = core
        .create_worktree(NewWorktree::ExistingBranch {
            name: "feat/local".into(),
        })
        .await
        .unwrap()
        .worktree;
    assert_eq!(local.branch.as_deref(), Some("feat/local"));

    let branches = core.branches().await.unwrap().branches;
    assert!(
        branches
            .iter()
            .any(|b| b.name == "origin/feat/remote" && b.remote),
        "{branches:?}"
    );
    let remote = core
        .create_worktree(NewWorktree::ExistingBranch {
            name: "origin/feat/remote".into(),
        })
        .await
        .unwrap()
        .worktree;
    assert_eq!(
        remote.branch.as_deref(),
        Some("feat/remote"),
        "a local branch tracking the remote one"
    );
    assert_eq!(rev_parse(&remote.path, "HEAD"), remote_tip);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_branch_checked_out_elsewhere_is_refused_with_a_clear_message() {
    let setup = RepoWithOrigin::new();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    core.open_workspace(&setup.repo()).await.unwrap();

    let err = core
        .create_worktree(NewWorktree::ExistingBranch {
            name: "main".into(),
        })
        .await
        .unwrap_err();

    assert!(
        matches!(&err, CoreError::BranchCheckedOut { branch, .. } if branch == "main"),
        "{err}"
    );
    assert!(err.to_string().contains("already checked out"), "{err}");
    let branches = core.branches().await.unwrap().branches;
    let main = branches.iter().find(|b| b.name == "main").unwrap();
    assert!(main.checked_out_in.is_some(), "the picker can grey it out");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_start_point_that_looks_like_an_option_is_refused_not_passed_to_git() {
    let setup = RepoWithOrigin::new();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    core.open_workspace(&setup.repo()).await.unwrap();

    let err = core
        .create_worktree(NewWorktree::NewBranch {
            name: "agent/sneaky".into(),
            start_point: Some("--detach".into()),
        })
        .await
        .unwrap_err();

    assert!(
        matches!(&err, CoreError::UnknownStartPoint(s) if s == "--detach"),
        "{err}"
    );
    assert_eq!(core.worktrees().len(), 1, "nothing was created");
}

#[tokio::test(flavor = "multi_thread")]
async fn branch_names_that_would_share_a_folder_get_their_own() {
    let setup = RepoWithOrigin::new();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    core.open_workspace(&setup.repo()).await.unwrap();
    let new = |name: &str| NewWorktree::NewBranch {
        name: name.into(),
        start_point: None,
    };

    let first = core.create_worktree(new("feat/x")).await.unwrap().worktree;
    let second = core.create_worktree(new("feat-x")).await.unwrap().worktree;

    let folder = |p: &std::path::Path| p.file_name().unwrap().to_string_lossy().into_owned();
    assert_eq!(folder(&first.path), "feat-x");
    assert_eq!(folder(&second.path), "feat-x-2");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_default_start_point_is_origins_main_even_without_origin_head() {
    let setup = RepoWithOrigin::new();
    // A repo whose origin was added by hand has no refs/remotes/origin/HEAD.
    git(&setup.repo(), &["remote", "set-head", "origin", "--delete"]);
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    core.open_workspace(&setup.repo()).await.unwrap();

    assert_eq!(core.default_start_point().await.unwrap(), "origin/main");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_fetch_still_creates_the_worktree_and_says_so() {
    let setup = RepoWithOrigin::new();
    let gone = setup.root().join("no-such-origin.git");
    git(
        &setup.repo(),
        &["remote", "set-url", "origin", gone.to_str().unwrap()],
    );
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    core.open_workspace(&setup.repo()).await.unwrap();

    let created = core
        .create_worktree(NewWorktree::NewBranch {
            name: "agent/offline".into(),
            start_point: None,
        })
        .await
        .unwrap();

    assert_eq!(created.worktree.branch.as_deref(), Some("agent/offline"));
    let warning = created.warning.expect("the stale start point is reported");
    assert!(warning.contains("origin/main"), "{warning}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_branch_name_that_already_exists_is_refused() {
    let setup = RepoWithOrigin::new();
    git(&setup.repo(), &["branch", "agent/taken"]);
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    core.open_workspace(&setup.repo()).await.unwrap();

    let err = core
        .create_worktree(NewWorktree::NewBranch {
            name: "agent/taken".into(),
            start_point: None,
        })
        .await
        .unwrap_err();

    assert!(
        matches!(&err, CoreError::BranchExists(name) if name == "agent/taken"),
        "{err}"
    );
    assert_ne!(core.suggest_branch_name().await.unwrap(), "agent/taken");
}
