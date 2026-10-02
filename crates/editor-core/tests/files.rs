//! Ticket 13: Worktree actors keep a live index of each shown Worktree's files (for Ctrl+P and the
//! Files drawer), watch only what git doesn't ignore, fall back to polling at the watch limit, and
//! keep the branch status live. Real temp repositories.

mod support;

use std::future::Future;
use std::path::Path;
use std::time::Duration;

use editor_core::{Core, CoreConfig, CoreEvent, FileWatchConfig, NewWorktree, WatchStatus};
use support::*;

/// Polls `check` until it holds (or panics after `TIMEOUT`).
async fn until<F: Future<Output = bool>>(what: &str, mut check: impl FnMut() -> F) {
    tokio::time::timeout(TIMEOUT, async {
        while !check().await {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {what}"));
}

async fn has(core: &Core, worktree: &Path, path: &str) -> bool {
    core.find_files(worktree, path, 50)
        .await
        .unwrap()
        .iter()
        .any(|m| m.path == path)
}

fn write(root: &Path, rel: &str, text: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// A repo with tracked, untracked and ignored files.
fn project() -> tempfile::TempDir {
    let repo = git_repo();
    write(repo.path(), ".gitignore", "target/\n*.log\n");
    write(repo.path(), "README.md", "hi");
    write(repo.path(), "src/parser.rs", "fn parse() {}");
    write(repo.path(), "src/lib.rs", "mod parser;");
    git(repo.path(), &["add", "."]);
    commit_all(repo.path(), "files");
    write(repo.path(), "notes.txt", "untracked but not ignored");
    write(repo.path(), "target/debug/app.exe", "ignored");
    write(repo.path(), "debug.log", "ignored too");
    repo
}

fn commit_all(dir: &Path, message: &str) {
    git(
        dir,
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "--quiet",
            "-m",
            message,
        ],
    );
}

async fn shown(repo: &Path, config: FileWatchConfig) -> (FakeAgent, Core, std::path::PathBuf) {
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = Core::new(CoreConfig {
        file_watch: config,
        ..CoreConfig::new(fake.command())
    });
    let root = core.open_workspace(repo).await.unwrap().root;
    core.show_worktree(&root).await.unwrap();
    (fake, core, root)
}

#[tokio::test(flavor = "multi_thread")]
async fn the_index_holds_what_git_doesnt_ignore_and_follows_changes() {
    let repo = project();
    let (_fake, core, root) = shown(repo.path(), FileWatchConfig::default()).await;
    for path in ["README.md", "src/parser.rs", "notes.txt", ".gitignore"] {
        assert!(has(&core, &root, path).await, "{path} indexed");
    }
    assert!(!has(&core, &root, "target/debug/app.exe").await);
    assert!(!has(&core, &root, "debug.log").await);

    write(&root, "src/new_module.rs", "");
    until("a new file", || has(&core, &root, "src/new_module.rs")).await;
    std::fs::remove_file(root.join("README.md")).unwrap();
    until("a deleted file", || async {
        !has(&core, &root, "README.md").await
    })
    .await;
    std::fs::rename(root.join("src/lib.rs"), root.join("src/main.rs")).unwrap();
    until("a renamed file", || async {
        has(&core, &root, "src/main.rs").await && !has(&core, &root, "src/lib.rs").await
    })
    .await;
    write(&root, "docs/guide/intro.md", "a new folder");
    until("a file in a new folder", || {
        has(&core, &root, "docs/guide/intro.md")
    })
    .await;

    // Ignored paths stay out, even ones that appear later (a sentinel shows they've been seen).
    write(&root, "target/release/app.exe", "");
    write(&root, "build.log", "");
    write(&root, "sentinel-1.txt", "");
    until("the sentinel", || has(&core, &root, "sentinel-1.txt")).await;
    assert!(!has(&core, &root, "target/release/app.exe").await);
    assert!(!has(&core, &root, "build.log").await);

    // A deleted tracked file stays gone through a full re-read (here, an edited .gitignore).
    write(&root, ".gitignore", "target/\n*.log\n*.tmp\n");
    write(&root, "sentinel-2.txt", "");
    until("the re-read", || has(&core, &root, "sentinel-2.txt")).await;
    assert!(!has(&core, &root, "README.md").await);
}

#[tokio::test(flavor = "multi_thread")]
async fn ignored_folders_are_never_watched() {
    let repo = project();
    let (_fake, core, root) = shown(repo.path(), FileWatchConfig::default()).await;
    let Some(WatchStatus::Watching { watched }) = core.file_watch(&root).await else {
        panic!("watching");
    };
    assert!(
        watched.iter().all(|w| !w.starts_with(root.join("target"))),
        "{watched:?}"
    );
    if cfg!(windows) {
        assert!(
            watched.contains(&root),
            "one recursive watch of the Worktree"
        );
    } else {
        assert!(watched.contains(&root.join("src")), "{watched:?}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn ctrl_p_ranks_matches_and_tolerates_typos() {
    let repo = project();
    write(repo.path(), "src/praser_old.rs", "");
    let (_fake, core, root) = shown(repo.path(), FileWatchConfig::default()).await;
    until("the new file", || has(&core, &root, "src/praser_old.rs")).await;

    let found = core.find_files(&root, "parser", 10).await.unwrap();
    assert_eq!(found[0].path, "src/parser.rs", "{found:?}");
    let typo_hit = found.iter().position(|m| m.path == "src/praser_old.rs");
    assert!(
        typo_hit.is_some_and(|at| at > 0),
        "a typo match, below: {found:?}"
    );
    let typo = core.find_files(&root, "parsr", 10).await.unwrap();
    assert!(typo.iter().any(|m| m.path == "src/parser.rs"), "{typo:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_files_drawer_lists_a_folder_and_marks_changes() {
    let repo = project();
    let (_fake, core, root) = shown(repo.path(), FileWatchConfig::default()).await;
    let mut events = core.subscribe();
    write(&root, "src/parser.rs", "fn parse() { changed }");
    next_event(&mut events, "the change", |e| match e {
        CoreEvent::FilesChanged { .. } => Some(()),
        _ => None,
    })
    .await;
    until("the change marked", || async {
        let top = core.list_dir(&root, "").await.unwrap();
        top.iter()
            .any(|e| e.name == "src" && e.is_dir && e.has_changes)
    })
    .await;
    let src = core.list_dir(&root, "src").await.unwrap();
    let parser = src.iter().find(|e| e.name == "parser.rs").unwrap();
    assert_eq!(parser.change.as_deref(), Some("M"));
    let top = core.list_dir(&root, "").await.unwrap();
    let names: Vec<&str> = top.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names[0], "src", "folders first: {names:?}");
    let notes = top.iter().find(|e| e.name == "notes.txt").unwrap();
    assert_eq!(notes.change.as_deref(), Some("??"));
    assert!(!names.contains(&"target"), "ignored folders aren't listed");
}

#[tokio::test(flavor = "multi_thread")]
async fn at_the_watch_limit_the_worktree_is_polled_with_a_toast() {
    let repo = project();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = Core::new(CoreConfig {
        file_watch: FileWatchConfig {
            watch_limit: Some(0),
            poll_interval: Duration::from_millis(100),
        },
        ..CoreConfig::new(fake.command())
    });
    let root = core.open_workspace(repo.path()).await.unwrap().root;
    let mut events = core.subscribe();
    core.show_worktree(&root).await.unwrap();
    let message = next_event(&mut events, "the fallback toast", |e| match e {
        CoreEvent::FileWatchFallback { worktree, message } if worktree == root => Some(message),
        _ => None,
    })
    .await;
    if cfg!(target_os = "linux") {
        assert!(message.contains("sysctl"), "{message}");
    }
    assert_eq!(core.file_watch(&root).await, Some(WatchStatus::Polling));

    write(&root, "polled.txt", "");
    until("the polled file", || has(&core, &root, "polled.txt")).await;
    std::fs::remove_file(root.join("README.md")).unwrap();
    until("the deleted tracked file gone", || async {
        !has(&core, &root, "README.md").await
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn actors_stop_for_worktrees_that_are_dimmed_and_not_shown() {
    let setup = RepoWithOrigin::new();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    let root = core.open_workspace(&setup.repo()).await.unwrap().root;
    let other = core
        .create_worktree(NewWorktree::NewBranch {
            name: "agent/files".into(),
            start_point: None,
        })
        .await
        .unwrap()
        .worktree
        .path;

    core.show_worktree(&other).await.unwrap();
    assert!(core.file_watch(&other).await.is_some());
    core.show_worktree(&root).await.unwrap();
    assert!(
        core.file_watch(&other).await.is_none(),
        "no sessions, not shown"
    );

    // A session keeps its Worktree's actor running while another is shown.
    core.new_session_in(&other).await.unwrap();
    until("the actor for the session's Worktree", || async {
        core.file_watch(&other).await.is_some()
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_commit_in_a_terminal_updates_the_branch_status_live() {
    let setup = RepoWithOrigin::new();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    let root = core.open_workspace(&setup.repo()).await.unwrap().root;
    core.show_worktree(&root).await.unwrap();
    let main = || core.worktrees().into_iter().find(|w| w.is_main).unwrap();
    assert_eq!(main().ahead, Some(0));

    write(&root, "work.txt", "in progress");
    until("the change counted", || async { main().changed == 1 }).await;
    git(&root, &["add", "work.txt"]);
    commit_all(&root, "from a terminal");
    until("ahead by one", || async {
        main().ahead == Some(1) && main().changed == 0
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_build_in_an_ignored_folder_costs_no_re_read() {
    let repo = project();
    let (_fake, core, root) = shown(repo.path(), FileWatchConfig::default()).await;
    let before = core.file_index_stats(&root).await.unwrap();
    // More than a burst's worth of writes, all ignored.
    for i in 0..700 {
        write(&root, &format!("target/debug/deps/lib{i}.rlib"), "");
    }
    write(&root, "after-the-build.txt", "");
    until("the sentinel", || has(&core, &root, "after-the-build.txt")).await;
    let after = core.file_index_stats(&root).await.unwrap();
    assert_eq!(
        after.full_reindexes, before.full_reindexes,
        "no full re-read"
    );
    assert_eq!(after.files, before.files + 1, "only the sentinel was added");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_empty_new_folder_is_followed_once_files_arrive() {
    let repo = project();
    let (_fake, core, root) = shown(repo.path(), FileWatchConfig::default()).await;
    std::fs::create_dir(root.join("later")).unwrap();
    write(&root, "sentinel.txt", "");
    until("the sentinel", || has(&core, &root, "sentinel.txt")).await;
    write(&root, "later/file.txt", "");
    until("the file in the once-empty folder", || {
        has(&core, &root, "later/file.txt")
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_large_repo_is_searched_quickly() {
    let repo = git_repo();
    for dir in 0..50 {
        for file in 0..100 {
            write(repo.path(), &format!("pkg_{dir}/src/module_{file}.rs"), "");
        }
    }
    let (_fake, core, root) = shown(repo.path(), FileWatchConfig::default()).await;
    assert_eq!(core.file_index_stats(&root).await.unwrap().files, 5000);
    let started = std::time::Instant::now();
    let found = core.find_files(&root, "pkg_42 module_7", 50).await.unwrap();
    assert!(
        found.iter().any(|m| m.path == "pkg_42/src/module_7.rs"),
        "{found:?}"
    );
    assert!(
        started.elapsed() < Duration::from_millis(500),
        "{:?}",
        started.elapsed()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_linked_worktree_follows_commits_and_fetches_live() {
    let setup = RepoWithOrigin::new();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    core.open_workspace(&setup.repo()).await.unwrap();
    let path = core
        .create_worktree(NewWorktree::NewBranch {
            name: "agent/live".into(),
            start_point: None, // origin/main
        })
        .await
        .unwrap()
        .worktree
        .path;
    // (A new branch tracks nothing until it's pushed; here it follows origin/main.)
    git(&path, &["branch", "--quiet", "--set-upstream-to=origin/main"]);
    core.refresh_worktrees().await;
    core.show_worktree(&path).await.unwrap();
    let info = || {
        core.worktrees()
            .into_iter()
            .find(|w| w.path == path)
            .unwrap()
    };
    assert_eq!((info().ahead, info().behind), (Some(0), Some(0)));

    write(&path, "agent.txt", "");
    git(&path, &["add", "agent.txt"]);
    commit_all(&path, "the Agent's commit");
    until("ahead by one", || async { info().ahead == Some(1) }).await;

    setup.push_to_origin("main", "a colleague's");
    git(&path, &["fetch", "--quiet"]);
    until("behind by one", || async { info().behind == Some(1) }).await;
}
