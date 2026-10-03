//! Ticket 30: switching the window to another Workspace closes the open one as quitting would.

mod support;

use editor_core::{Core, SessionId, SessionState};
use support::*;

const ECHO: &str = r#"{"turns":[]}"#;

async fn turn(core: &Core, id: SessionId, text: &str) {
    let mut events = core.subscribe();
    core.send_prompt(id, text).await.unwrap();
    states_until(&mut events, id, SessionState::Idle).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn switching_stops_the_old_workspace_and_switching_back_restores_its_tabs() {
    let fake = FakeAgent::new(ECHO);
    let (first, second) = (git_repo(), git_repo());
    let core = core_with_state(&fake);
    let first_root = core.open_workspace(first.path()).await.unwrap().root;
    let id = core.new_session().await.unwrap();
    turn(&core, id, "one").await;
    let acp_id = fake.received("session/prompt")[0]["params"]["sessionId"].clone();

    let second_root = core.open_workspace(second.path()).await.unwrap().root;
    assert_ne!(first_root, second_root);
    assert!(core.sessions().is_empty(), "no Tab comes across");
    assert!(core.worktrees().iter().all(|w| w.path == second_root));
    assert!(
        fake.received("session/close")
            .iter()
            .any(|c| c["params"]["sessionId"] == acp_id),
        "the old Workspace's Agent was stopped"
    );
    assert!(core.session_info(id).is_err());

    core.open_workspace(first.path()).await.unwrap();
    let sessions = core.sessions();
    assert_eq!(sessions.len(), 1, "its Tab is restored");
    assert_eq!(sessions[0].state, SessionState::Suspended);
    assert_ne!(sessions[0].id, id, "session ids aren't reused");
    assert_eq!(sessions[0].worktree, first_root);
}

#[tokio::test(flavor = "multi_thread")]
async fn opening_the_open_workspace_again_changes_nothing() {
    let fake = FakeAgent::new(ECHO);
    let repo = git_repo();
    let core = core_with_state(&fake);
    core.open_workspace(repo.path()).await.unwrap();
    let id = core.new_session().await.unwrap();
    turn(&core, id, "one").await;

    core.open_workspace(repo.path()).await.unwrap();
    assert_eq!(core.session_info(id).unwrap().state, SessionState::Idle);
    assert!(fake.received("session/close").is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_folder_that_isnt_a_repository_leaves_the_open_workspace_alone() {
    let fake = FakeAgent::new(ECHO);
    let repo = git_repo();
    let core = core_with_state(&fake);
    let root = core.open_workspace(repo.path()).await.unwrap().root;
    let id = core.new_session().await.unwrap();
    let elsewhere = tempfile::tempdir().unwrap();

    assert!(core.open_workspace(elsewhere.path()).await.is_err());
    assert!(core.session_info(id).is_ok());
    assert_eq!(core.workspace_for(repo.path()).await.unwrap().root, root);
}
