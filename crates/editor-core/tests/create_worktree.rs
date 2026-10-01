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

    let name = core.suggest_branch_name().await;
    assert!(name.starts_with("agent/"), "{name}");
    let created = core
        .create_worktree(NewWorktree::NewBranch {
            name: name.clone(),
            base: None,
        })
        .await
        .unwrap();

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
            base: Some("main".into()),
        })
        .await
        .unwrap();

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
        .unwrap();
    assert_eq!(local.branch.as_deref(), Some("feat/local"));

    let branches = core.branches().await.unwrap();
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
        .unwrap();
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
    let branches = core.branches().await.unwrap();
    let main = branches.iter().find(|b| b.name == "main").unwrap();
    assert!(main.checked_out_in.is_some(), "the picker can grey it out");
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
            base: None,
        })
        .await
        .unwrap_err();

    assert!(
        matches!(&err, CoreError::BranchExists(name) if name == "agent/taken"),
        "{err}"
    );
    assert_ne!(core.suggest_branch_name().await, "agent/taken");
}
