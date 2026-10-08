//! A commit's hooks run with the login shell's environment: started from the desktop, Orchard's
//! own `PATH` may lead to another `node` than the user's version manager's, and a hook running the
//! project's linter then fails. (A test binary of its own: it sets `$SHELL` for the process.)

#![cfg(unix)]

mod support;

use std::os::unix::fs::PermissionsExt;

use editor_core::{CommitOutcome, CommitRequest};
use support::*;

#[tokio::test(flavor = "multi_thread")]
async fn a_commits_hooks_see_what_the_shells_profile_sets() {
    let dir = tempfile::tempdir().unwrap();
    // Stands in for a login shell whose profile exports a variable (nvm putting its `node` first).
    let shell = dir.path().join("shell");
    std::fs::write(
        &shell,
        "#!/bin/bash\nexport ORCHARD_PROFILE_VAR=from-profile\neval \"$4\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o755)).unwrap();
    // SAFETY: the only test in this binary, set before any shell starts.
    unsafe { std::env::set_var("SHELL", &shell) };

    let repo = git_repo();
    git(repo.path(), &["config", "user.name", "Test"]);
    git(repo.path(), &["config", "user.email", "test@example.com"]);
    let hook = repo.path().join(".git/hooks/pre-commit");
    std::fs::write(
        &hook,
        "#!/bin/sh\n[ \"$ORCHARD_PROFILE_VAR\" = from-profile ] || { echo 'no profile env'; exit 1; }\n",
    )
    .unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();

    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    let root = core.open_workspace(repo.path()).await.unwrap().root;
    std::fs::write(root.join("a.rs"), "fn a() {}\n").unwrap();
    core.stage_all(&root).await.unwrap();

    let outcome = core
        .commit(
            &root,
            CommitRequest {
                message: "Change a".into(),
                ..CommitRequest::default()
            },
        )
        .await
        .unwrap();
    assert!(
        matches!(outcome, CommitOutcome::Committed { .. }),
        "{outcome:?}"
    );
}
