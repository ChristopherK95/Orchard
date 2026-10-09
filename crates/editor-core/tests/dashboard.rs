//! Issue 43: what the Dashboard shows that the core works out — the Worktrees that need attention,
//! with real `git` and a bare origin.

mod support;

use std::path::{Path, PathBuf};
use std::process::Command;

use editor_core::{AttentionReason, Core, GitOperation, NewWorktree};
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

/// A commit in `dir` that writes `text` to `file`.
fn commit_file(dir: &Path, file: &str, text: &str) {
    std::fs::write(dir.join(file), text).unwrap();
    git(dir, &["add", "--", file]);
    commit(dir, &format!("write {file}"));
}

#[tokio::test(flavor = "multi_thread")]
async fn worktrees_needing_attention_are_listed_with_every_reason() {
    let setup = RepoWithOrigin::new();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    let root = core.open_workspace(&setup.repo()).await.unwrap().root;

    // Pushed once, then one more commit and two files not committed.
    let a = new_worktree(&core, "a").await;
    git(&a, &["push", "--quiet", "-u", "origin", "a"]);
    commit(&a, "not pushed");
    std::fs::write(a.join("one.txt"), "1").unwrap();
    std::fs::write(a.join("two.txt"), "2").unwrap();

    // Never pushed, one commit since its Base.
    let b = new_worktree(&core, "b").await;
    commit(&b, "only here");

    // Left in the middle of a rebase that conflicts.
    let c = new_worktree(&core, "c").await;
    git(&c, &["checkout", "--quiet", "-b", "c-other"]);
    commit_file(&c, "x.txt", "other\n");
    git(&c, &["checkout", "--quiet", "c"]);
    commit_file(&c, "x.txt", "mine\n");
    let rebase = Command::new("git")
        .args(["rebase", "c-other"])
        .current_dir(&c)
        .output()
        .unwrap();
    assert!(!rebase.status.success(), "the rebase conflicts");

    // Tracking a branch origin has moved on from.
    setup.push_to_origin("d", "first");
    let d = core
        .create_worktree(NewWorktree::ExistingBranch {
            name: "origin/d".into(),
        })
        .await
        .unwrap()
        .worktree
        .path;
    setup.push_to_origin("d", "second");
    git(&d, &["fetch", "--quiet"]);

    let listed = core.worktree_attention().await.unwrap();
    let reasons = |path: &Path| {
        listed
            .iter()
            .find(|w| w.path == path)
            .map(|w| w.reasons.clone())
    };
    assert_eq!(
        reasons(&root),
        None,
        "the main checkout is clean and pushed"
    );
    assert_eq!(
        reasons(&a),
        Some(vec![
            AttentionReason::Uncommitted { files: 2 },
            AttentionReason::Unpushed { commits: 1 },
        ])
    );
    assert_eq!(
        reasons(&b),
        Some(vec![AttentionReason::NoUpstream { commits: 1 }])
    );
    assert!(
        reasons(&c).unwrap().contains(&AttentionReason::Operation {
            operation: GitOperation::Rebase
        }),
        "{listed:?}"
    );
    assert_eq!(
        reasons(&d),
        Some(vec![AttentionReason::Behind { commits: 1 }])
    );
    let a_row = listed.iter().find(|w| w.path == a).unwrap();
    assert_eq!(a_row.branch.as_deref(), Some("a"));
}
