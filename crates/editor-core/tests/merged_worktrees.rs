//! The Worktrees overview: whether each Worktree's branch is merged into its Base, however its PR
//! landed, with real `git` and a bare origin (PRs are merged from another clone, as a host would).

mod support;

use std::path::{Path, PathBuf};

use editor_core::{Core, MergeState, NewWorktree, RemoveWorktree};
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

/// A commit in `dir` that adds `file`.
fn commit_file(dir: &Path, file: &str) {
    std::fs::write(dir.join(file), format!("{file}\n")).unwrap();
    git(dir, &["add", "--", file]);
    commit(dir, &format!("add {file}"));
}

/// Pushes the Worktree's branch to origin under its own name.
fn push(worktree: &Path, branch: &str) {
    git(
        worktree,
        &[
            "push",
            "--quiet",
            "origin",
            &format!("HEAD:refs/heads/{branch}"),
        ],
    );
}

/// Merges origin's `branch` into origin's `main` the way `how` says (see `merge_pr_into`).
fn merge_pr(setup: &RepoWithOrigin, branch: &str, how: &str) {
    merge_pr_into(setup, branch, "main", how);
}

/// Merges origin's `branch` into origin's `target` the way `how` says, from the seed clone, then
/// deletes the branch on origin (as a host does once a PR is merged).
fn merge_pr_into(setup: &RepoWithOrigin, branch: &str, target: &str, how: &str) {
    let seed = setup.root().join("seed");
    git(&seed, &["fetch", "--quiet", "origin"]);
    let start = format!("origin/{target}");
    git(&seed, &["checkout", "--quiet", "-B", target, &start]);
    let theirs = format!("origin/{branch}");
    let ident = ["-c", "user.name=Host", "-c", "user.email=host@example.com"];
    let run = |args: &[&str]| git(&seed, &[&ident[..], args].concat());
    match how {
        "merge" => run(&["merge", "--quiet", "--no-ff", "-m", "Merge PR", &theirs]),
        "squash" => {
            run(&["merge", "--quiet", "--squash", &theirs]);
            run(&["commit", "--quiet", "-m", "Squashed PR"]);
        }
        "rebase" => run(&["cherry-pick", &format!("{target}..{theirs}")]),
        _ => unreachable!(),
    }
    git(&seed, &["push", "--quiet", "origin", target]);
    git(&seed, &["push", "--quiet", "origin", "--delete", branch]);
}

async fn state_of(core: &Core, worktree: &Path) -> Option<MergeState> {
    let rows = core.merge_overview().await.unwrap();
    rows.into_iter()
        .find(|row| row.path == *worktree)
        .expect("listed")
        .merge
}

#[tokio::test(flavor = "multi_thread")]
async fn each_way_a_pr_lands_shows_as_merged() {
    let setup = RepoWithOrigin::new();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    core.open_workspace(&setup.repo()).await.unwrap();
    let mut worktrees = vec![];
    for (branch, how) in [
        ("pr/merge", "merge"),
        ("pr/squash", "squash"),
        ("pr/rebase", "rebase"),
    ] {
        let path = new_worktree(&core, branch).await;
        commit_file(&path, &format!("{how}-1.txt"));
        commit_file(&path, &format!("{how}-2.txt"));
        push(&path, branch);
        worktrees.push((path, branch, how));
    }
    for (_, branch, how) in &worktrees {
        merge_pr(&setup, branch, how);
    }
    core.fetch(&setup.repo()).await.unwrap();

    for (path, _, how) in &worktrees {
        let expected = match *how {
            "merge" => MergeState::Merged {
                into: "origin/main".into(),
            },
            _ => MergeState::ChangesInBase {
                into: "origin/main".into(),
            },
        };
        assert_eq!(state_of(&core, path).await, Some(expected), "{how}");
    }

    // A squashed branch's commits are on nothing else, but its changes are in the Base: removing
    // it with its branch needs no "Discard and remove".
    let (squashed, _, _) = &worktrees[1];
    let check = core.removal_check(squashed).await.unwrap();
    assert!(check.merged);
    assert_eq!(check.unpushed_count, 2, "still listed");
    assert!(!check.discard_to_remove && !check.discard_to_delete_branch);
    core.remove_worktree(
        squashed,
        RemoveWorktree {
            discard: None,
            delete_branch: true,
        },
    )
    .await
    .unwrap();
    assert!(!squashed.exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_fast_forward_counts_as_merged_but_a_fresh_branch_does_not() {
    let setup = RepoWithOrigin::new();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    core.open_workspace(&setup.repo()).await.unwrap();
    let fresh = new_worktree(&core, "agent/fresh").await;
    let landed = new_worktree(&core, "agent/landed").await;
    commit_file(&landed, "landed.txt");
    git(&landed, &["push", "--quiet", "origin", "HEAD:main"]);
    // The Base moves on past both.
    let seed = setup.root().join("seed");
    git(&seed, &["fetch", "--quiet", "origin"]);
    git(&seed, &["checkout", "--quiet", "-B", "main", "origin/main"]);
    commit(&seed, "later");
    git(&seed, &["push", "--quiet", "origin", "main"]);
    core.fetch(&setup.repo()).await.unwrap();

    assert_eq!(state_of(&core, &fresh).await, Some(MergeState::NothingNew));
    assert_eq!(
        state_of(&core, &landed).await,
        Some(MergeState::Merged {
            into: "origin/main".into()
        })
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn unmerged_work_is_counted_and_a_deleted_remote_branch_is_told_apart() {
    let setup = RepoWithOrigin::new();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    core.open_workspace(&setup.repo()).await.unwrap();
    let wip = new_worktree(&core, "agent/wip").await;
    commit_file(&wip, "wip.txt");
    let closed = new_worktree(&core, "agent/closed").await;
    commit_file(&closed, "closed.txt");
    git(
        &closed,
        &[
            "push",
            "--quiet",
            "-u",
            "origin",
            "HEAD:refs/heads/agent/closed",
        ],
    );
    // Closed without merging: the host deletes the branch all the same.
    git(
        &closed,
        &["push", "--quiet", "origin", "--delete", "agent/closed"],
    );
    // Commits that change nothing aren't "found in the Base".
    let empty = new_worktree(&core, "agent/empty").await;
    commit(&empty, "nothing in it");
    core.fetch(&setup.repo()).await.unwrap();

    assert_eq!(
        state_of(&core, &wip).await,
        Some(MergeState::NotMerged { commits: 1 })
    );
    assert_eq!(
        state_of(&core, &closed).await,
        Some(MergeState::RemoteDeleted { commits: 1 })
    );
    assert_eq!(
        state_of(&core, &empty).await,
        Some(MergeState::NotMerged { commits: 1 })
    );
    let main = core.merge_overview().await.unwrap().remove(0);
    assert!(main.is_main && main.merge.is_none());
}

/// A project branch on origin, made from `main` with a commit of its own.
fn project_branch(setup: &RepoWithOrigin, name: &str) {
    let seed = setup.root().join("seed");
    git(&seed, &["fetch", "--quiet", "origin"]);
    git(&seed, &["checkout", "--quiet", "-B", name, "origin/main"]);
    commit(&seed, "project starts");
    git(&seed, &["push", "--quiet", "origin", name]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pr_merged_into_a_project_branch_counts_as_merged_there() {
    let setup = RepoWithOrigin::new();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    core.open_workspace(&setup.repo()).await.unwrap();
    project_branch(&setup, "project/big");
    // Both start from main (their Base): their PRs went to the project branch instead.
    let mut worktrees = vec![];
    for (branch, how) in [("pr/merged", "merge"), ("pr/squashed", "squash")] {
        let path = new_worktree(&core, branch).await;
        commit_file(&path, &format!("{how}.txt"));
        push(&path, branch);
        merge_pr_into(&setup, branch, "project/big", how);
        worktrees.push(path);
    }
    core.fetch(&setup.repo()).await.unwrap();

    assert_eq!(
        state_of(&core, &worktrees[0]).await,
        Some(MergeState::Merged {
            into: "origin/project/big".into()
        })
    );
    assert_eq!(
        state_of(&core, &worktrees[1]).await,
        Some(MergeState::ChangesInBase {
            into: "origin/project/big".into()
        })
    );
    let check = core.removal_check(&worktrees[1]).await.unwrap();
    assert!(check.merged && !check.discard_to_delete_branch);
    assert_eq!(check.merged_into.as_deref(), Some("origin/project/big"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_branch_stacked_on_this_one_is_not_where_it_was_merged() {
    let setup = RepoWithOrigin::new();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    core.open_workspace(&setup.repo()).await.unwrap();
    let base_pr = new_worktree(&core, "pr/first").await;
    commit_file(&base_pr, "first.txt");
    push(&base_pr, "pr/first");
    // A second PR built on top of the first: it has the first one's commit, on its own line.
    commit_file(&base_pr, "second.txt");
    git(
        &base_pr,
        &["push", "--quiet", "origin", "HEAD:refs/heads/pr/second"],
    );
    git(&base_pr, &["reset", "--quiet", "--hard", "HEAD~1"]);
    core.fetch(&setup.repo()).await.unwrap();

    assert_eq!(
        state_of(&core, &base_pr).await,
        Some(MergeState::NotMerged { commits: 1 })
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_branch_that_merged_the_project_branch_in_is_not_where_it_was_merged() {
    // As in a real repo: the PR's head on the host had the project branch merged in ("Update
    // branch"), which the local branch never fetched; it was merged into the project branch and
    // deleted. Then another PR's branch merged the project branch in, so it has the commits too.
    let setup = RepoWithOrigin::new();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    core.open_workspace(&setup.repo()).await.unwrap();
    project_branch(&setup, "project/big");
    let path = new_worktree(&core, "pr/done").await;
    commit_file(&path, "done.txt");
    push(&path, "pr/done");
    let seed = setup.root().join("seed");
    let ident = ["-c", "user.name=Host", "-c", "user.email=host@example.com"];
    let run = |args: &[&str]| git(&seed, &[&ident[..], args].concat());
    // The project moves on, and the host updates the PR's branch with it.
    git(&seed, &["fetch", "--quiet", "origin"]);
    git(
        &seed,
        &[
            "checkout",
            "--quiet",
            "-B",
            "project/big",
            "origin/project/big",
        ],
    );
    commit(&seed, "project moves on");
    git(&seed, &["push", "--quiet", "origin", "project/big"]);
    git(
        &seed,
        &["checkout", "--quiet", "-B", "pr/done", "origin/pr/done"],
    );
    run(&[
        "merge",
        "--quiet",
        "--no-ff",
        "-m",
        "Update branch",
        "project/big",
    ]);
    git(&seed, &["push", "--quiet", "origin", "pr/done"]);
    merge_pr_into(&setup, "pr/done", "project/big", "merge");
    // Another PR's branch, newer, merges the project branch in.
    git(
        &seed,
        &["checkout", "--quiet", "-B", "pr/other", "origin/main"],
    );
    commit(&seed, "other work");
    run(&[
        "merge",
        "--quiet",
        "--no-ff",
        "-m",
        "Merge project in",
        "origin/project/big",
    ]);
    git(&seed, &["push", "--quiet", "origin", "pr/other"]);
    core.fetch(&setup.repo()).await.unwrap();

    assert_eq!(
        state_of(&core, &path).await,
        Some(MergeState::Merged {
            into: "origin/project/big".into()
        })
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_feature_branch_made_from_the_project_branch_after_the_merge_is_not_the_target() {
    // A feature branch made from the project branch right after the PR landed shares the very
    // commit it arrived with; the project branch, which PRs keep going into, is the target.
    let setup = RepoWithOrigin::new();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    core.open_workspace(&setup.repo()).await.unwrap();
    project_branch(&setup, "project/big");
    let done = new_worktree(&core, "pr/done").await;
    commit_file(&done, "done.txt");
    push(&done, "pr/done");
    merge_pr_into(&setup, "pr/done", "project/big", "merge");
    let seed = setup.root().join("seed");
    git(
        &seed,
        &["checkout", "--quiet", "-B", "feature", "project/big"],
    );
    commit(&seed, "feature work");
    git(&seed, &["push", "--quiet", "origin", "feature"]);
    let later = new_worktree(&core, "pr/later").await;
    commit_file(&later, "later.txt");
    push(&later, "pr/later");
    merge_pr_into(&setup, "pr/later", "project/big", "merge");
    core.fetch(&setup.repo()).await.unwrap();

    assert_eq!(
        state_of(&core, &done).await,
        Some(MergeState::Merged {
            into: "origin/project/big".into()
        })
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_worktree_made_outside_the_editor_is_measured_against_the_branch_it_was_made_from() {
    let setup = RepoWithOrigin::new();
    project_branch(&setup, "project/big");
    git(&setup.repo(), &["fetch", "--quiet", "origin"]);
    let path = setup.root().join("outside");
    git(
        &setup.repo(),
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "pr/outside",
            path.to_str().unwrap(),
            "origin/project/big",
        ],
    );
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    core.open_workspace(&setup.repo()).await.unwrap();

    let row = core
        .merge_overview()
        .await
        .unwrap()
        .into_iter()
        .find(|r| r.branch.as_deref() == Some("pr/outside"))
        .expect("listed");
    assert_eq!(row.base, "origin/project/big");
    assert_eq!(row.merge, Some(MergeState::NothingNew));
}
