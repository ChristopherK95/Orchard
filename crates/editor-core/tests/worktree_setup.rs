//! Ticket 08: Worktree setup, the repo's commands run in each new Worktree before its first Agent
//! session. The commands are written to mean the same in `bash` and PowerShell.

mod support;

use std::path::Path;

use editor_core::{Core, CoreError, CoreEvent, NewWorktree, SessionId, SetupStatus};
use support::*;
use tokio::sync::broadcast;

fn repo_settings(key: &str, body: &str) -> String {
    format!("[repos.{}]\n{body}\n", toml_literal(key))
}

async fn new_worktree(core: &Core, name: &str) -> editor_core::CreatedWorktree {
    core.create_worktree(NewWorktree::NewBranch {
        name: name.into(),
        start_point: Some("main".into()),
    })
    .await
    .unwrap()
}

/// Waits for the setup in `worktree` to finish, returning the session it started.
async fn setup_done(events: &mut broadcast::Receiver<CoreEvent>, worktree: &Path) -> SessionId {
    next_event(events, "setup to finish", |e| match e {
        CoreEvent::SetupChanged {
            worktree: w,
            status,
            ..
        } if w == worktree => match status {
            SetupStatus::Done { session_id } => Some(session_id),
            SetupStatus::Failed { message, .. } => panic!("setup failed: {message}"),
            _ => None,
        },
        _ => None,
    })
    .await
}

/// Waits for the setup in `worktree` to fail, returning the failed step and why.
async fn setup_failed(
    events: &mut broadcast::Receiver<CoreEvent>,
    worktree: &Path,
) -> (usize, String) {
    next_event(events, "setup to fail", |e| match e {
        CoreEvent::SetupChanged {
            worktree: w,
            status,
            ..
        } if w == worktree => match status {
            SetupStatus::Failed { step, message } => Some((step, message)),
            SetupStatus::Done { .. } => panic!("setup succeeded"),
            _ => None,
        },
        _ => None,
    })
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn setup_runs_in_order_in_the_new_worktree_then_starts_its_first_session() {
    let setup = RepoWithOrigin::new();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let settings = repo_settings(
        &origin_url(&setup.repo()),
        r#"setup = ["mkdir first", "cd first; mkdir second", "echo setup-says-hi"]"#,
    );
    let core = core_with_settings(&fake, &settings);
    core.open_workspace(&setup.repo()).await.unwrap();
    let mut events = core.subscribe();

    let created = new_worktree(&core, "agent/with-setup").await;
    let path = created.worktree.path.clone();
    let started = created.setup.expect("setup runs");
    assert_eq!(started.commands.len(), 3);
    assert!(
        fake.received("session/new").is_empty(),
        "the first session waits for setup"
    );

    let session = setup_done(&mut events, &path).await;
    assert!(
        path.join("first").join("second").is_dir(),
        "ran in order, in the Worktree"
    );
    assert_eq!(core.session_info(session).unwrap().worktree, path);
    let output = core.setup(&path).expect("setup is kept").output;
    assert!(output.contains("mkdir first"), "commands echoed: {output}");
    assert!(
        output.contains("setup-says-hi"),
        "output captured: {output}"
    );
}

/// A setup command that prints `text`, then runs until `until` exists. Forward slashes, so the
/// same text works in bash and PowerShell.
fn print_then_wait(text: &str, until: &Path) -> String {
    let slashes = |p: &Path| p.display().to_string().replace('\\', "/");
    format!(
        "{} --print {text} --until {}",
        slashes(&fake_agent_path()),
        slashes(until)
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn output_streams_while_the_command_runs_and_the_first_session_waits() {
    let setup = RepoWithOrigin::new();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let go = setup.root().join("go");
    let settings = repo_settings(
        &origin_url(&setup.repo()),
        &format!(
            "setup = [{}]",
            toml_literal(&print_then_wait("streamed-line", &go))
        ),
    );
    let core = core_with_settings(&fake, &settings);
    core.open_workspace(&setup.repo()).await.unwrap();
    let mut events = core.subscribe();

    let path = new_worktree(&core, "agent/streams").await.worktree.path;
    let mut streamed = String::new();
    next_event(&mut events, "the command's output", |e| match e {
        CoreEvent::SetupOutput { worktree, text } if worktree == path => {
            streamed.push_str(&text);
            streamed.contains("streamed-line").then_some(())
        }
        _ => None,
    })
    .await;
    // The command is still running (it waits for `go`), so this is streaming, not a replay.
    assert!(matches!(
        core.setup(&path).unwrap().status,
        SetupStatus::Running { step: 0 }
    ));
    assert!(matches!(
        core.new_session_in(&path).await,
        Err(CoreError::SetupRunning)
    ));
    assert!(fake.received("session/new").is_empty());

    std::fs::write(&go, "").unwrap();
    let session = setup_done(&mut events, &path).await;
    assert_eq!(core.session_info(session).unwrap().worktree, path);
}

#[tokio::test(flavor = "multi_thread")]
async fn retry_runs_the_command_as_fixed_in_the_settings_file() {
    let setup = RepoWithOrigin::new();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let key = origin_url(&setup.repo());
    let core = core_with_settings(
        &fake,
        &repo_settings(&key, r#"setup = ["mkdir one", "exit 3"]"#),
    );
    core.open_workspace(&setup.repo()).await.unwrap();
    let mut events = core.subscribe();

    let path = new_worktree(&core, "agent/fixed").await.worktree.path;
    assert_eq!(setup_failed(&mut events, &path).await.0, 1);
    assert!(matches!(
        core.new_session_in(&path).await,
        Err(CoreError::SetupFailed)
    ));

    std::fs::write(
        fake.settings_path(),
        repo_settings(&key, r#"setup = ["mkdir one", "mkdir two"]"#),
    )
    .unwrap();
    core.retry_setup(&path).await.unwrap();
    setup_done(&mut events, &path).await;
    assert!(path.join("two").is_dir(), "the fixed command ran");
    assert_eq!(core.setup(&path).unwrap().commands[1], "mkdir two");
}

#[cfg(windows)]
#[tokio::test(flavor = "multi_thread")]
async fn setup_can_run_in_git_bash_on_windows() {
    let setup = RepoWithOrigin::new();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let settings = repo_settings(
        &origin_url(&setup.repo()),
        "setup = [\"[[ -d . ]] && mkdir bashed\"]\nwindows_shell = \"git-bash\"",
    );
    let core = core_with_settings(&fake, &settings);
    core.open_workspace(&setup.repo()).await.unwrap();
    let mut events = core.subscribe();

    let path = new_worktree(&core, "agent/bash").await.worktree.path;
    setup_done(&mut events, &path).await;
    assert!(path.join("bashed").is_dir());
}
#[tokio::test(flavor = "multi_thread")]
async fn a_failing_command_stops_the_list_and_retry_reruns_from_it() {
    let setup = RepoWithOrigin::new();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    // Step 1 fails until the branch `fixed` exists; rerunning step 0 would fail too (it exists).
    let settings = repo_settings(
        &origin_url(&setup.repo()),
        r#"setup = ["mkdir once", "git rev-parse --verify --quiet fixed", "mkdir after"]"#,
    );
    let core = core_with_settings(&fake, &settings);
    core.open_workspace(&setup.repo()).await.unwrap();
    let mut events = core.subscribe();

    let path = new_worktree(&core, "agent/fails").await.worktree.path;
    let (step, message) = setup_failed(&mut events, &path).await;
    assert_eq!(step, 1, "{message}");
    assert!(path.join("once").is_dir());
    assert!(!path.join("after").exists(), "stopped at the failure");
    assert!(
        fake.received("session/new").is_empty(),
        "no session after a failure"
    );

    git(&setup.repo(), &["branch", "fixed"]);
    core.retry_setup(&path).await.unwrap();
    let session = setup_done(&mut events, &path).await;
    assert!(path.join("after").is_dir());
    assert_eq!(core.session_info(session).unwrap().worktree, path);
}

#[tokio::test(flavor = "multi_thread")]
async fn start_anyway_starts_the_session_without_the_rest_of_the_list() {
    let setup = RepoWithOrigin::new();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let settings = repo_settings(
        &origin_url(&setup.repo()),
        r#"setup = ["exit 3", "mkdir never"]"#,
    );
    let core = core_with_settings(&fake, &settings);
    core.open_workspace(&setup.repo()).await.unwrap();
    let mut events = core.subscribe();

    let path = new_worktree(&core, "agent/anyway").await.worktree.path;
    let (step, message) = setup_failed(&mut events, &path).await;
    assert_eq!(step, 0);
    assert!(message.contains('3'), "says how it failed: {message}");

    core.start_anyway(&path).unwrap();
    let session = setup_done(&mut events, &path).await;
    assert_eq!(core.session_info(session).unwrap().worktree, path);
    assert!(!path.join("never").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_per_os_override_replaces_the_shared_list() {
    let setup = RepoWithOrigin::new();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let settings = repo_settings(
        &origin_url(&setup.repo()),
        "setup = [\"mkdir shared\"]\nsetup_windows = [\"mkdir windows\"]\nsetup_linux = [\"mkdir linux\"]",
    );
    let core = core_with_settings(&fake, &settings);
    core.open_workspace(&setup.repo()).await.unwrap();
    let mut events = core.subscribe();

    let path = new_worktree(&core, "agent/per-os").await.worktree.path;
    setup_done(&mut events, &path).await;
    let this_os = if cfg!(windows) { "windows" } else { "linux" };
    assert!(path.join(this_os).is_dir());
    assert!(!path.join("shared").exists(), "replaced, not added to");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_repo_section_falls_back_to_the_main_checkout_path_and_origin_wins() {
    let setup = RepoWithOrigin::new();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let by_path = repo_settings(
        &setup.repo().display().to_string(),
        r#"setup = ["mkdir by-path"]"#,
    );
    let elsewhere = repo_settings(
        "https://example.com/another/repo.git",
        r#"setup = ["mkdir wrong-repo"]"#,
    );
    let core = core_with_settings(&fake, &format!("{by_path}\n{elsewhere}"));
    core.open_workspace(&setup.repo()).await.unwrap();
    let mut events = core.subscribe();

    let path = new_worktree(&core, "agent/by-path").await.worktree.path;
    setup_done(&mut events, &path).await;
    assert!(path.join("by-path").is_dir());

    let by_origin = repo_settings(&origin_url(&setup.repo()), r#"setup = ["mkdir by-origin"]"#);
    std::fs::write(fake.settings_path(), format!("{by_path}\n{by_origin}")).unwrap();
    eventually("the new settings", || {
        core.settings().settings.repos.len() == 2
            && core
                .settings()
                .settings
                .repos
                .contains_key(&origin_url(&setup.repo()))
    })
    .await;
    let path = new_worktree(&core, "agent/by-origin").await.worktree.path;
    setup_done(&mut events, &path).await;
    assert!(path.join("by-origin").is_dir());
    assert!(!path.join("by-path").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn without_setup_nothing_runs_and_no_session_starts_on_its_own() {
    let setup = RepoWithOrigin::new();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    core.open_workspace(&setup.repo()).await.unwrap();

    let created = new_worktree(&core, "agent/plain").await;
    assert!(created.setup.is_none());
    assert!(core.setup(&created.worktree.path).is_none());
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert!(fake.received("session/new").is_empty());
}
