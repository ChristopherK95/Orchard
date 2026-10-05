//! The Terminal panel: one shell per Worktree, in the Worktree's folder, kept running out of sight.

mod support;

use std::path::Path;
use std::time::Duration;

use editor_core::{Core, NewWorktree, RemoveWorktree, TerminalOutput, TerminalStream};
use support::*;

/// A shell can take a while to start (PowerShell loads the user's profile).
const SHELL_TIMEOUT: Duration = Duration::from_secs(30);

/// Prints the terminal's marker variable (set for every shell it starts), then the folder it's in.
fn print_marker_and_folder() -> &'static str {
    if cfg!(windows) {
        "Write-Output \"[$env:TERM_PROGRAM]\" (Get-Location).Path\r"
    } else {
        "echo \"[$TERM_PROGRAM]\" \"$PWD\"\r"
    }
}

/// Reads until the output so far contains `needle`, returning all of it. Answers the cursor
/// position queries a terminal (xterm.js, in the app) would: ConPTY waits for one as it starts.
async fn output_until(
    core: &Core,
    worktree: &Path,
    stream: &mut TerminalStream,
    needle: &str,
) -> String {
    let mut seen = String::new();
    tokio::time::timeout(SHELL_TIMEOUT, async {
        while !seen.contains(needle) {
            match stream.next().await.expect("the terminal stream ended") {
                TerminalOutput::Output { text } => {
                    for _ in text.matches("[6n") {
                        let _ = core.terminal_input(worktree, "[1;1R");
                    }
                    seen.push_str(&text);
                }
                TerminalOutput::Replay { text } => seen.push_str(&text),
                TerminalOutput::Exited { code } => panic!("the shell exited ({code:?}): {seen}"),
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {needle:?} in: {seen}"));
    seen
}

async fn exited(stream: &mut TerminalStream) {
    tokio::time::timeout(SHELL_TIMEOUT, async {
        loop {
            if let TerminalOutput::Exited { .. } = stream.next().await.expect("stream ended") {
                return;
            }
        }
    })
    .await
    .expect("timed out waiting for the shell to exit");
}

/// A core (its fake Agent is never started: terminals don't need one).
fn core() -> (FakeAgent, Core) {
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    (fake, core)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_worktrees_shell_runs_in_its_folder_and_keeps_its_output_while_out_of_sight() {
    let setup = RepoWithOrigin::new();
    let (_fake, core) = core();
    let root = core.open_workspace(&setup.repo()).await.unwrap().root;

    let mut stream = core.open_terminal(&root, 120, 30).await.unwrap();
    core.terminal_input(&root, print_marker_and_folder())
        .unwrap();
    let seen = output_until(&core, &root, &mut stream, "[Orchard]").await;
    let folder = root.file_name().unwrap().to_string_lossy().into_owned();
    let seen = seen + &output_until(&core, &root, &mut stream, &folder).await;
    assert!(seen.contains(&folder), "{seen}");

    // Hidden, then shown again: the same shell, with what it printed.
    core.hide_terminal();
    let mut again = core.open_terminal(&root, 100, 30).await.unwrap();
    match again.next().await {
        Some(TerminalOutput::Replay { text }) => assert!(text.contains("[Orchard]"), "{text}"),
        other => panic!("expected the kept output first, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_exited_shell_is_replaced_by_a_new_one_when_asked_again() {
    let setup = RepoWithOrigin::new();
    let (_fake, core) = core();
    let root = core.open_workspace(&setup.repo()).await.unwrap().root;

    let mut stream = core.open_terminal(&root, 80, 24).await.unwrap();
    core.terminal_input(&root, print_marker_and_folder())
        .unwrap();
    output_until(&core, &root, &mut stream, "[Orchard]").await;
    core.terminal_input(&root, "exit\r").unwrap();
    exited(&mut stream).await;
    eventually("the exited shell to go", || {
        core.terminal_input(&root, "x").is_err()
    })
    .await;

    let mut fresh = core.open_terminal(&root, 80, 24).await.unwrap();
    core.terminal_input(&root, print_marker_and_folder())
        .unwrap();
    output_until(&core, &root, &mut fresh, "[Orchard]").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn removing_a_worktree_stops_the_shell_standing_in_it() {
    let setup = RepoWithOrigin::new();
    let (_fake, core) = core();
    core.open_workspace(&setup.repo()).await.unwrap();
    let path = core
        .create_worktree(NewWorktree::NewBranch {
            name: "agent/shell".into(),
            start_point: Some("main".into()),
        })
        .await
        .unwrap()
        .worktree
        .path;

    let mut stream = core.open_terminal(&path, 80, 24).await.unwrap();
    core.terminal_input(&path, print_marker_and_folder())
        .unwrap();
    output_until(&core, &path, &mut stream, "[Orchard]").await;

    let removed = core
        .remove_worktree(
            &path,
            RemoveWorktree {
                discard: None,
                delete_branch: true,
            },
        )
        .await
        .unwrap();
    assert_eq!(removed.warning, None);
    assert!(!path.exists());
    assert!(
        core.terminal_input(&path, "x").is_err(),
        "its shell is gone"
    );
}
