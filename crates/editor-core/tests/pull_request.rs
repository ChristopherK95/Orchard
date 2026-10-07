//! Ticket 42: Review lists everything the PR would bring (committed, uncommitted and untracked),
//! and "Create PR" commits what's left, pushes, and runs `gh pr create` (a fake one here).
#![cfg(unix)]

mod support;

use std::path::{Path, PathBuf};

use editor_core::{
    ChangeKind, Core, CoreConfig, PullRequestOutcome, PullRequestProgress, PullRequestRequest,
    PushToPullRequestOutcome,
};
use support::*;

async fn open(core: &Core, repo: &Path) -> PathBuf {
    git(repo, &["config", "user.name", "Test"]);
    git(repo, &["config", "user.email", "test@example.com"]);
    core.open_workspace(repo).await.unwrap().root
}

/// A fake `gh` that logs its arguments (one per line) and answers as `gh pr create` would.
struct FakeGh {
    dir: tempfile::TempDir,
}

impl FakeGh {
    fn new(script_tail: &str) -> Self {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("args");
        let script = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\n{script_tail}\n",
            log.display()
        );
        let path = dir.path().join("gh");
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        Self { dir }
    }

    fn path(&self) -> PathBuf {
        self.dir.path().join("gh")
    }

    fn args(&self) -> Vec<String> {
        std::fs::read_to_string(self.dir.path().join("args"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }
}

fn core_with_gh(fake: &FakeAgent, gh: &FakeGh) -> Core {
    Core::new(CoreConfig {
        gh: Some(gh.path()),
        ..CoreConfig::new(fake.command())
    })
}

fn write(dir: &Path, name: &str, text: &str) {
    std::fs::write(dir.join(name), text).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn review_lists_committed_uncommitted_and_untracked_changes() {
    let setup = RepoWithOrigin::new();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let gh = FakeGh::new("echo https://github.com/o/r/pull/1");
    let core = core_with_gh(&fake, &gh);
    let root = open(&core, &setup.repo()).await;
    write(&root, "kept.txt", "one\n");
    git(&root, &["add", "kept.txt"]);
    commit(&root, "kept");
    git(&root, &["push", "--quiet", "origin", "main"]);
    git(
        &root,
        &["checkout", "--quiet", "-b", "agent/work", "origin/main"],
    );
    write(&root, "committed.txt", "a\n");
    git(&root, &["add", "committed.txt"]);
    commit(&root, "Add committed");
    write(&root, "kept.txt", "one\ntwo\n");
    write(&root, "new.txt", "fresh\n");

    let review = core.review(&root).await.unwrap();
    assert_eq!(review.branch.as_deref(), Some("agent/work"));
    assert_eq!(review.target, "main");
    assert!(review.targets.contains(&"main".to_owned()));
    let subjects: Vec<&str> = review.commits.iter().map(|c| c.subject.as_str()).collect();
    assert_eq!(subjects, ["Add committed"]);
    assert_eq!(review.uncommitted, 2);
    let file = |p: &str| review.files.iter().find(|f| f.path == p).unwrap().clone();
    assert_eq!(file("committed.txt").change, ChangeKind::Added);
    assert!(!file("committed.txt").uncommitted);
    assert_eq!(file("kept.txt").change, ChangeKind::Modified);
    assert!(file("kept.txt").uncommitted);
    assert_eq!(file("new.txt").change, ChangeKind::Added);
    assert_eq!(review.files.len(), 3);

    let lines = core
        .review_diff(&root, &review.split, "kept.txt", None, ChangeKind::Modified)
        .await
        .unwrap();
    assert!(lines.iter().any(|l| l.text == "two"));
}

#[tokio::test(flavor = "multi_thread")]
async fn create_pr_commits_what_is_left_pushes_and_runs_gh() {
    let setup = RepoWithOrigin::new();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let gh = FakeGh::new("echo 'Creating pull request'; echo; echo https://github.com/o/r/pull/9");
    let core = core_with_gh(&fake, &gh);
    let root = open(&core, &setup.repo()).await;
    git(&root, &["checkout", "--quiet", "-b", "agent/work"]);
    write(&root, "new.txt", "fresh\n");

    let outcome = core
        .create_pull_request(
            &root,
            PullRequestRequest {
                target: "main".into(),
                title: "Add new".into(),
                body: "Why it's needed".into(),
                commit_message: "Add new.txt".into(),
                ..Default::default()
            },
            |_| {},
        )
        .await
        .unwrap();
    let PullRequestOutcome::Created { url, committed } = outcome else {
        panic!("not created: {outcome:?}");
    };
    assert_eq!(url, "https://github.com/o/r/pull/9");
    assert!(committed.is_some());
    // Committed (untracked file and all) and pushed.
    assert!(core.git_status(&root).await.unwrap().files.is_empty());
    assert_eq!(
        rev_parse(&setup.root().join("origin.git"), "agent/work"),
        rev_parse(&root, "HEAD")
    );
    let args = gh.args();
    let after = |flag: &str| args[args.iter().position(|a| a == flag).unwrap() + 1].clone();
    assert_eq!(args[..2], ["pr", "create"]);
    assert_eq!(after("--base"), "main");
    assert_eq!(after("--head"), "agent/work");
    assert_eq!(after("--title"), "Add new");
    assert_eq!(after("--body"), "Why it's needed");
    assert_eq!(after("--assignee"), "@me");
    assert!(!args.contains(&"--draft".to_owned()));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_open_pr_for_the_branch_is_reported_and_a_bad_request_does_nothing() {
    let setup = RepoWithOrigin::new();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let gh = FakeGh::new(
        "echo 'a pull request for branch \"agent/work\" into branch \"main\" already exists:' >&2; echo https://github.com/o/r/pull/3 >&2; exit 1",
    );
    let core = core_with_gh(&fake, &gh);
    let root = open(&core, &setup.repo()).await;
    git(&root, &["checkout", "--quiet", "-b", "agent/work"]);
    commit(&root, "work");

    // On the target itself, or without a title: nothing is pushed.
    let request = |target: &str, title: &str| PullRequestRequest {
        target: target.into(),
        title: title.into(),
        ..Default::default()
    };
    assert!(core
        .create_pull_request(&root, request("agent/work", "T"), |_| {})
        .await
        .is_err());
    assert!(core
        .create_pull_request(&root, request("main", " "), |_| {})
        .await
        .is_err());
    assert!(gh.args().is_empty());

    let outcome = core
        .create_pull_request(&root, request("main", "Work"), |_| {})
        .await
        .unwrap();
    assert_eq!(
        outcome,
        PullRequestOutcome::AlreadyOpen {
            url: "https://github.com/o/r/pull/3".into(),
            committed: None
        }
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_commit_shows_only_what_it_changed() {
    let setup = RepoWithOrigin::new();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let gh = FakeGh::new("true");
    let core = core_with_gh(&fake, &gh);
    let root = open(&core, &setup.repo()).await;
    git(&root, &["checkout", "--quiet", "-b", "agent/work"]);
    write(&root, "first.txt", "one\n");
    git(&root, &["add", "first.txt"]);
    commit(&root, "First");
    write(&root, "first.txt", "one\ntwo\n");
    write(&root, "second.txt", "b\n");
    git(&root, &["add", "-A"]);
    commit(&root, "Second");
    write(&root, "loose.txt", "not committed\n");

    let review = core.review(&root).await.unwrap();
    assert_eq!(review.files.len(), 3);
    let second = &review.commits[1];
    assert_eq!(second.subject, "Second");
    assert_eq!(second.id, rev_parse(&root, "HEAD"));

    let changes = core.commit_changes(&root, &second.id).await.unwrap();
    assert_eq!(changes.parent, rev_parse(&root, "HEAD~1"));
    let paths: Vec<(&str, ChangeKind)> = changes
        .files
        .iter()
        .map(|f| (f.path.as_str(), f.change))
        .collect();
    assert_eq!(
        paths,
        [
            ("first.txt", ChangeKind::Modified),
            ("second.txt", ChangeKind::Added)
        ]
    );
    let lines = core
        .commit_diff(
            &root,
            &changes.parent,
            &second.id,
            "first.txt",
            None,
            ChangeKind::Modified,
        )
        .await
        .unwrap();
    let added: Vec<&str> = lines
        .iter()
        .filter(|l| l.kind == editor_core::DiffLineKind::Added)
        .map(|l| l.text.as_str())
        .collect();
    assert_eq!(added, ["two"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failing_pre_push_hook_says_what_it_found() {
    use std::os::unix::fs::PermissionsExt;
    let setup = RepoWithOrigin::new();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let gh = FakeGh::new("echo https://github.com/o/r/pull/1");
    let core = core_with_gh(&fake, &gh);
    let root = open(&core, &setup.repo()).await;
    git(&root, &["checkout", "--quiet", "-b", "agent/work"]);
    commit(&root, "work");
    // A linter's report goes to stdout; git adds its own line on stderr.
    let hook = root.join(".git/hooks/pre-push");
    std::fs::write(
        &hook,
        "#!/bin/sh\necho 'src/a.ts 3:1 error no-unused-vars'\nexit 1\n",
    )
    .unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();

    let progress = std::sync::Mutex::new(vec![]);
    let err = core
        .create_pull_request(
            &root,
            PullRequestRequest {
                target: "main".into(),
                title: "Work".into(),
                ..Default::default()
            },
            |p| progress.lock().unwrap().push(p),
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("no-unused-vars"), "{err}");
    // The hook's report came out live, after the push step started.
    let progress = progress.into_inner().unwrap();
    assert!(
        matches!(&progress[0], PullRequestProgress::Step { text } if text.starts_with("Pushing agent/work"))
    );
    let streamed: String = progress
        .iter()
        .filter_map(|p| match p {
            PullRequestProgress::Output { text } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert!(streamed.contains("no-unused-vars"), "{streamed}");
    assert!(err.contains("failed to push"), "{err}");
    assert!(gh.args().is_empty(), "no PR without the push");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_branch_with_an_open_pr_pushes_to_it() {
    let setup = RepoWithOrigin::new();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let gh = FakeGh::new(
        r#"echo '[{"baseRefName":"main","number":12,"url":"https://github.com/o/r/pull/12"}]'"#,
    );
    let core = core_with_gh(&fake, &gh);
    let root = open(&core, &setup.repo()).await;
    git(&root, &["checkout", "--quiet", "-b", "agent/work"]);
    commit(&root, "work");

    let open_pr = core.open_pull_request(&root).await.unwrap().unwrap();
    assert_eq!((open_pr.number, open_pr.target.as_str()), (12, "main"));
    let args = gh.args();
    assert_eq!(args[..2], ["pr", "list"]);
    assert!(args.contains(&"agent/work".to_owned()));

    // Uncommitted changes need a message; nothing is pushed without one.
    write(&root, "more.txt", "more\n");
    assert!(core
        .push_to_pull_request(&root, " ", false, |_| {})
        .await
        .is_err());
    let progress = std::sync::Mutex::new(vec![]);
    let outcome = core
        .push_to_pull_request(&root, "More", false, |p| progress.lock().unwrap().push(p))
        .await
        .unwrap();
    let PushToPullRequestOutcome::Pushed { to, committed } = outcome else {
        panic!("not pushed: {outcome:?}");
    };
    assert_eq!(to, "origin/agent/work");
    assert!(committed.is_some());
    assert!(core.git_status(&root).await.unwrap().files.is_empty());
    assert_eq!(
        rev_parse(&setup.root().join("origin.git"), "agent/work"),
        rev_parse(&root, "HEAD")
    );
    let steps: Vec<String> = progress
        .into_inner()
        .unwrap()
        .into_iter()
        .filter_map(|p| match p {
            PullRequestProgress::Step { text } => Some(text),
            _ => None,
        })
        .collect();
    assert_eq!(steps, ["Committing 1 file", "Pushing agent/work to origin"]);
}
