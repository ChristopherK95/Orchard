//! Ticket 48: the Worktree switcher's "Close all and forget" closes sessions without keeping them.

mod support;

use editor_core::{Core, CoreError, CoreEvent, SessionId, SessionState};
use support::*;

const ECHO: &str = r#"{"turns":[]}"#;

async fn turn(core: &Core, id: SessionId, text: &str) {
    let mut events = core.subscribe();
    core.send_prompt(id, text).await.unwrap();
    states_until(&mut events, id, SessionState::Idle).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn closing_and_forgetting_a_tab_stops_it_without_keeping_it_in_recent_sessions() {
    let repo = git_repo();
    let fake = FakeAgent::new(ECHO);
    let root = {
        let core = core_with_state(&fake);
        let root = core.open_workspace(repo.path()).await.unwrap().root;
        let kept = core.new_session().await.unwrap();
        turn(&core, kept, "kept").await;
        core.close_tab(kept).await.unwrap();
        let forgotten = core.new_session().await.unwrap();
        turn(&core, forgotten, "forgotten").await;
        let mut events = core.subscribe();

        core.close_and_forget(forgotten).await.unwrap();
        next_event(&mut events, "the Tab to close", |e| match e {
            CoreEvent::SessionClosed { session_id } if session_id == forgotten => Some(()),
            _ => None,
        })
        .await;
        assert!(matches!(
            core.session_info(forgotten),
            Err(CoreError::UnknownSession)
        ));
        assert_eq!(fake.received("session/close").len(), 2, "its Agent stopped");
        let names: Vec<_> = core
            .recent_sessions(&root)
            .into_iter()
            .map(|s| s.name)
            .collect();
        assert_eq!(names, ["Bramble"], "only the Tab closed the usual way");
        root
    }; // the editor quits

    let core = core_with_state(&fake);
    core.open_workspace(repo.path()).await.unwrap();
    assert!(core.sessions().is_empty());
    let names: Vec<_> = core
        .recent_sessions(&root)
        .into_iter()
        .map(|s| s.name)
        .collect();
    assert_eq!(names, ["Bramble"], "nor is it back after a restart");
}
