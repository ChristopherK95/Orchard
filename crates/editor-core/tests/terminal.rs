//! The Terminal panel: shells in a Worktree's folder (one or several), kept running out of sight.

mod support;

use std::time::Duration;

use editor_core::{
    Core, CoreEvent, NewWorktree, RemoveWorktree, TerminalId, TerminalOutput, TerminalStream,
    TABS_SLOT,
};
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

/// Prints `word` (typed as two halves, so the echo of the command line doesn't match).
fn print(word: &str) -> String {
    let (a, b) = word.split_at(word.len() / 2);
    if cfg!(windows) {
        format!("Write-Output ('{a}' + '{b}')\r")
    } else {
        format!("echo '{a}''{b}'\r")
    }
}

/// Reads until the output so far contains `needle`, returning all of it. Answers the cursor
/// position queries a terminal (xterm.js, in the app) would: ConPTY waits for one as it starts.
async fn output_until(
    core: &Core,
    id: TerminalId,
    stream: &mut TerminalStream,
    needle: &str,
) -> String {
    let mut seen = String::new();
    tokio::time::timeout(SHELL_TIMEOUT, async {
        while !seen.contains(needle) {
            match stream.next().await.expect("the terminal stream ended") {
                TerminalOutput::Output { text } => {
                    for _ in text.matches("\u{1b}[6n") {
                        let _ = core.terminal_input(id, "\u{1b}[1;1R");
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

    let (shell, mut stream) = core
        .open_terminal(TABS_SLOT, &root, None, 120, 30)
        .await
        .unwrap();
    assert_eq!(shell.worktree, root);
    assert!(!shell.name.is_empty());
    core.terminal_input(shell.id, print_marker_and_folder())
        .unwrap();
    let seen = output_until(&core, shell.id, &mut stream, "[Orchard]").await;
    let folder = root.file_name().unwrap().to_string_lossy().into_owned();
    let seen = seen + &output_until(&core, shell.id, &mut stream, &folder).await;
    assert!(seen.contains(&folder), "{seen}");

    // Hidden, then shown again: the same shell, with what it printed.
    core.hide_terminal(TABS_SLOT);
    let (again, mut stream) = core
        .open_terminal(TABS_SLOT, &root, None, 100, 30)
        .await
        .unwrap();
    assert_eq!(again.id, shell.id);
    match stream.next().await {
        Some(TerminalOutput::Replay { text }) => assert!(text.contains("[Orchard]"), "{text}"),
        other => panic!("expected the kept output first, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_exited_shell_is_replaced_by_a_new_one_when_asked_again() {
    let setup = RepoWithOrigin::new();
    let (_fake, core) = core();
    let root = core.open_workspace(&setup.repo()).await.unwrap().root;

    let (shell, mut stream) = core
        .open_terminal(TABS_SLOT, &root, None, 80, 24)
        .await
        .unwrap();
    core.terminal_input(shell.id, print_marker_and_folder())
        .unwrap();
    output_until(&core, shell.id, &mut stream, "[Orchard]").await;
    core.terminal_input(shell.id, "exit\r").unwrap();
    exited(&mut stream).await;
    eventually("the exited shell to go", || {
        core.terminal_input(shell.id, "x").is_err()
    })
    .await;
    assert!(core.terminals().is_empty());

    // Asking for it by id gets a new one.
    let (fresh, mut stream) = core
        .open_terminal(TABS_SLOT, &root, Some(shell.id), 80, 24)
        .await
        .unwrap();
    assert_ne!(fresh.id, shell.id);
    core.terminal_input(fresh.id, print_marker_and_folder())
        .unwrap();
    output_until(&core, fresh.id, &mut stream, "[Orchard]").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_worktree_can_run_several_shells_and_the_panel_switches_between_them() {
    let setup = RepoWithOrigin::new();
    let (_fake, core) = core();
    let root = core.open_workspace(&setup.repo()).await.unwrap().root;
    let mut events = core.subscribe();

    let (first, mut first_stream) = core
        .open_terminal(TABS_SLOT, &root, None, 80, 24)
        .await
        .unwrap();
    let (second, mut second_stream) = core.new_terminal(TABS_SLOT, &root, 80, 24).await.unwrap();
    assert_ne!(first.id, second.id);
    let listed: Vec<TerminalId> = core.terminals().iter().map(|t| t.id).collect();
    assert_eq!(listed, vec![first.id, second.id], "oldest first");
    // Each start was told.
    let mut told = 0;
    while let Ok(event) = events.try_recv() {
        if let CoreEvent::TerminalsChanged { .. } = event {
            told += 1;
        }
    }
    assert_eq!(told, 2);

    // The panel shows the new one: the first stopped sending to it, and keeps running.
    core.terminal_input(second.id, &print("second-shell"))
        .unwrap();
    output_until(&core, second.id, &mut second_stream, "second-shell").await;
    core.terminal_input(first.id, &print("first-shell"))
        .unwrap();
    let drained = tokio::time::timeout(SHELL_TIMEOUT, async {
        while first_stream.next().await.is_some() {}
    })
    .await;
    assert!(
        drained.is_ok(),
        "the first shell's stream ended when the panel moved on"
    );

    // Back to the first by id: what it printed out of sight is there.
    let (back, mut stream) = core
        .open_terminal(TABS_SLOT, &root, Some(first.id), 80, 24)
        .await
        .unwrap();
    assert_eq!(back.id, first.id);
    output_until(&core, first.id, &mut stream, "first-shell").await;

    // Stopping one leaves the other.
    core.close_terminal(second.id).await;
    let listed: Vec<TerminalId> = core.terminals().iter().map(|t| t.id).collect();
    assert_eq!(listed, vec![first.id]);
    assert!(core.terminal_input(second.id, "x").is_err());
    core.terminal_input(first.id, &print("still-here")).unwrap();
    output_until(&core, first.id, &mut stream, "still-here").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn removing_a_worktree_stops_every_shell_standing_in_it() {
    let setup = RepoWithOrigin::new();
    let (_fake, core) = core();
    let root = core.open_workspace(&setup.repo()).await.unwrap().root;
    let path = core
        .create_worktree(NewWorktree::NewBranch {
            name: "agent/shell".into(),
            start_point: Some("main".into()),
        })
        .await
        .unwrap()
        .worktree
        .path;

    let (main_shell, _main_stream) = core
        .open_terminal("column:main", &root, None, 80, 24)
        .await
        .unwrap();
    let (one, mut stream) = core
        .open_terminal(TABS_SLOT, &path, None, 80, 24)
        .await
        .unwrap();
    core.terminal_input(one.id, print_marker_and_folder())
        .unwrap();
    output_until(&core, one.id, &mut stream, "[Orchard]").await;
    let (two, mut stream) = core.new_terminal(TABS_SLOT, &path, 80, 24).await.unwrap();
    core.terminal_input(two.id, print_marker_and_folder())
        .unwrap();
    output_until(&core, two.id, &mut stream, "[Orchard]").await;

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
        core.terminal_input(one.id, "x").is_err(),
        "its shells are gone"
    );
    assert!(
        core.terminal_input(two.id, "x").is_err(),
        "its shells are gone"
    );
    let listed: Vec<TerminalId> = core.terminals().iter().map(|t| t.id).collect();
    assert_eq!(
        listed,
        vec![main_shell.id],
        "another Worktree's shell stays"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn each_column_shows_its_own_worktrees_shell_at_once() {
    let setup = RepoWithOrigin::new();
    let (_fake, core) = core();
    let root = core.open_workspace(&setup.repo()).await.unwrap().root;
    let other = core
        .create_worktree(NewWorktree::NewBranch {
            name: "agent/other".into(),
            start_point: Some("main".into()),
        })
        .await
        .unwrap()
        .worktree
        .path;

    let (a, mut left) = core
        .open_terminal("column:a", &root, None, 80, 24)
        .await
        .unwrap();
    let (b, mut right) = core
        .open_terminal("column:b", &other, None, 80, 24)
        .await
        .unwrap();
    core.terminal_input(a.id, print_marker_and_folder())
        .unwrap();
    core.terminal_input(b.id, print_marker_and_folder())
        .unwrap();
    // Both stream: the one shown second didn't take the first one's panel away.
    output_until(&core, a.id, &mut left, "[Orchard]").await;
    output_until(&core, b.id, &mut right, "[Orchard]").await;
}
