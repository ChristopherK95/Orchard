//! An Action's shell is started with no Terminal panel showing it, so the core answers the
//! terminal queries a shell asks as it starts. fish asks for the Primary Device Attributes and
//! waits 10 seconds for the answer. (A test binary of its own: it sets `$SHELL` for the process.)

#![cfg(unix)]

mod support;

use std::time::{Duration, Instant};

use editor_core::{TerminalOutput, TABS_SLOT};
use support::*;

#[tokio::test(flavor = "multi_thread")]
async fn an_actions_fish_shell_runs_its_command_without_waiting_on_an_unanswered_query() {
    let Ok(fish) = which("fish") else {
        eprintln!("skipped: fish isn't installed");
        return;
    };
    // SAFETY: the only test in this binary, set before any shell starts.
    unsafe { std::env::set_var("SHELL", fish) };
    let setup = RepoWithOrigin::new();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let settings = repo_settings(
        &origin_url(&setup.repo()),
        r#"actions = [{ name = "Hi", run = ["echo 'sa''ys-hi'"] }]"#,
    );
    let core = core_with_settings(&fake, &settings);
    let root = core.open_workspace(&setup.repo()).await.unwrap().root;

    let began = Instant::now();
    let started = core.run_action(&root, "Hi").await.unwrap();
    // (Out of sight a while, as when the panel opens on the Action's other shell.)
    tokio::time::sleep(Duration::from_millis(500)).await;
    let (_, mut stream) = core
        .open_terminal(TABS_SLOT, &root, Some(started[0].id), 80, 24)
        .await
        .unwrap();
    let mut seen = String::new();
    tokio::time::timeout(Duration::from_secs(30), async {
        while !seen.contains("says-hi") {
            match stream.next().await.expect("the terminal stream ended") {
                TerminalOutput::Output { text } | TerminalOutput::Replay { text } => {
                    seen.push_str(&text)
                }
                TerminalOutput::Exited { code } => panic!("fish exited ({code:?}): {seen}"),
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out: {seen}"));
    assert!(
        began.elapsed() < Duration::from_secs(5),
        "took {:?}: {seen}",
        began.elapsed()
    );
    assert!(!seen.contains("could not read response"), "{seen}");
}

fn which(program: &str) -> Result<std::path::PathBuf, ()> {
    std::env::var_os("PATH")
        .and_then(|path| {
            std::env::split_paths(&path)
                .map(|dir| dir.join(program))
                .find(|p| p.is_file())
        })
        .ok_or(())
}
