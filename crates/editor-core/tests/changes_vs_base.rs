//! Ticket 21: "Changes vs base" lists what a Worktree's branch changed since it split from its
//! Base (a three-dot diff), and shows each file's diff from the split. The Base is the Worktree's:
//! the default, its start point, or one set for it, remembered across restarts. Real `git`.

mod support;

use std::path::{Path, PathBuf};

use editor_core::{
    BaseChange, BaseChanges, ChangeKind, Core, CoreError, DiffLine, DiffLineKind, NewWorktree,
};
use support::*;

fn write(root: &Path, rel: &str, text: &str) {
    std::fs::write(root.join(rel), text).unwrap();
}

/// A repo whose `origin/main` has `a.rs`, `b.rs` and `c.rs`, and an Agent Worktree from it that
/// adds `new.rs`, changes `a.rs`, deletes `b.rs` and renames `c.rs` to `d.rs`. `origin/main` then
/// moves on (`z.rs`, and its own change to `a.rs`), which isn't the branch's change. The
/// Worktree's path.
async fn reviewed_branch(core: &Core, setup: &RepoWithOrigin) -> PathBuf {
    let repo = setup.repo();
    git(&repo, &["config", "user.name", "Test"]);
    git(&repo, &["config", "user.email", "test@example.com"]);
    write(&repo, "a.rs", "fn a() {}\n\n\n\n\nfn a2() {}\n");
    write(&repo, "b.rs", "fn b() {}\n");
    write(
        &repo,
        "c.rs",
        "fn c() {\n    // a file long enough that a rename is found\n}\n",
    );
    git(&repo, &["add", "."]);
    commit(&repo, "files");
    git(&repo, &["push", "--quiet", "origin", "main"]);
    core.open_workspace(&repo).await.unwrap();
    let worktree = core
        .create_worktree(NewWorktree::NewBranch {
            name: "agent/review".into(),
            start_point: None, // origin/main
        })
        .await
        .unwrap()
        .worktree
        .path;
    write(&worktree, "new.rs", "fn new() {}\n");
    write(
        &worktree,
        "a.rs",
        "fn a() { changed }\n\n\n\n\nfn a2() {}\n",
    );
    git(&worktree, &["rm", "--quiet", "b.rs"]);
    git(&worktree, &["mv", "c.rs", "d.rs"]);
    git(&worktree, &["add", "."]);
    commit(&worktree, "the Agent's work");
    // The Base moves on after the split, a.rs included.
    write(&repo, "z.rs", "fn z() {}\n");
    write(&repo, "a.rs", "fn a() {}\n\n\n\n\nfn a2() { theirs }\n");
    git(&repo, &["add", "."]);
    commit(&repo, "main moves on");
    git(&repo, &["push", "--quiet", "origin", "main"]);
    git(&worktree, &["fetch", "--quiet"]);
    // Uncommitted work isn't the branch's change (yet).
    write(&worktree, "new.rs", "fn new() { not committed }\n");
    worktree
}

fn change<'a>(files: &'a [BaseChange], path: &str) -> &'a BaseChange {
    files
        .iter()
        .find(|f| f.path == path)
        .unwrap_or_else(|| panic!("{path} in {files:?}"))
}

fn lines(kind: DiffLineKind, diff: &[DiffLine]) -> Vec<String> {
    diff.iter()
        .filter(|l| l.kind == kind)
        .map(|l| l.text.clone())
        .collect()
}

async fn diff(core: &Core, worktree: &Path, changes: &BaseChanges, path: &str) -> Vec<DiffLine> {
    let file = change(&changes.files, path);
    core.diff_vs_base(
        worktree,
        &changes.split,
        &file.path,
        file.renamed_from.as_deref(),
        file.change,
    )
    .await
    .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn the_list_is_what_the_branch_changed_since_it_split_from_its_base() {
    let setup = RepoWithOrigin::new();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    let worktree = reviewed_branch(&core, &setup).await;

    let changes = core.changes_vs_base(&worktree).await.unwrap();
    assert_eq!(
        (changes.base.as_str(), changes.is_default),
        ("origin/main", true)
    );
    let files = &changes.files;
    assert_eq!(change(files, "new.rs").change, ChangeKind::Added);
    assert_eq!(change(files, "a.rs").change, ChangeKind::Modified);
    assert_eq!(change(files, "b.rs").change, ChangeKind::Deleted);
    let renamed = change(files, "d.rs");
    assert_eq!(
        (renamed.change, renamed.renamed_from.as_deref()),
        (ChangeKind::Renamed, Some("c.rs"))
    );
    assert_eq!(
        files.len(),
        4,
        "not z.rs (the Base's), nor uncommitted work: {files:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn each_files_diff_runs_from_the_split_to_the_branch() {
    let setup = RepoWithOrigin::new();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    let worktree = reviewed_branch(&core, &setup).await;
    let changes = core.changes_vs_base(&worktree).await.unwrap();

    // From the split, not the Base's tip (which changed a2 since): only the branch's change.
    let modified = diff(&core, &worktree, &changes, "a.rs").await;
    assert_eq!(lines(DiffLineKind::Removed, &modified), ["fn a() {}"]);
    assert_eq!(
        lines(DiffLineKind::Added, &modified),
        ["fn a() { changed }"]
    );

    // As committed (not the uncommitted edit).
    let added = diff(&core, &worktree, &changes, "new.rs").await;
    assert_eq!(lines(DiffLineKind::Added, &added), ["fn new() {}"]);
    assert!(lines(DiffLineKind::Removed, &added).is_empty());

    let deleted = diff(&core, &worktree, &changes, "b.rs").await;
    assert_eq!(lines(DiffLineKind::Removed, &deleted), ["fn b() {}"]);

    let renamed = diff(&core, &worktree, &changes, "d.rs").await;
    assert!(renamed.is_empty(), "renamed, not changed: {renamed:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_base_can_be_changed_checked_in_the_worktree_and_set_back() {
    let setup = RepoWithOrigin::new();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    let worktree = reviewed_branch(&core, &setup).await;
    write(&worktree, "later.rs", "fn later() {}\n");
    git(&worktree, &["add", "later.rs"]);
    commit(&worktree, "later");

    // HEAD~1 is this branch's (the main checkout's HEAD has no parent at all).
    core.set_base(&worktree, Some("HEAD~1")).await.unwrap();
    let changes = core.changes_vs_base(&worktree).await.unwrap();
    assert_eq!(
        (changes.base.as_str(), changes.is_default),
        ("HEAD~1", false)
    );
    let paths: Vec<&str> = changes.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, ["later.rs"]);

    assert!(matches!(
        core.set_base(&worktree, Some("no/such/base")).await,
        Err(CoreError::UnknownBase(_))
    ));
    assert_eq!(
        core.changes_vs_base(&worktree).await.unwrap().base,
        "HEAD~1",
        "kept"
    );

    core.set_base(&worktree, None).await.unwrap();
    assert_eq!(
        core.changes_vs_base(&worktree).await.unwrap().files.len(),
        5
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_worktree_made_from_a_start_point_is_compared_against_it_across_restarts() {
    let setup = RepoWithOrigin::new();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let repo = setup.repo();
    git(&repo, &["config", "user.name", "Test"]);
    git(&repo, &["config", "user.email", "test@example.com"]);
    let child = {
        let core = core_with_state(&fake);
        core.open_workspace(&repo).await.unwrap();
        // Work in progress on a branch of its own, then a Worktree branched off it.
        git(&repo, &["checkout", "--quiet", "-b", "wip"]);
        write(&repo, "wip.rs", "fn wip() {}\n");
        git(&repo, &["add", "wip.rs"]);
        commit(&repo, "wip");
        git(&repo, &["checkout", "--quiet", "main"]);
        let child = core
            .create_worktree(NewWorktree::NewBranch {
                name: "agent/child".into(),
                start_point: Some("wip".into()),
            })
            .await
            .unwrap()
            .worktree
            .path;
        write(&child, "child.rs", "fn child() {}\n");
        git(&child, &["add", "child.rs"]);
        commit(&child, "child");
        let changes = core.changes_vs_base(&child).await.unwrap();
        assert_eq!((changes.base.as_str(), changes.is_default), ("wip", false));
        child
    }; // the editor quits

    let core = core_with_state(&fake);
    core.open_workspace(&repo).await.unwrap();
    let changes = core.changes_vs_base(&child).await.unwrap();
    assert_eq!(changes.base, "wip");
    let paths: Vec<&str> = changes.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, ["child.rs"], "not wip's own work");
}
