//! What the sidebar's session cards show besides the state: the session's character name, the model
//! it runs on, and when it was last active.

mod support;

use editor_core::{CoreEvent, SessionState};
use support::*;

const MODELS: &str = r#""models":[
    {"value":"default","name":"Default (recommended)","description":"Opus 4.8"},
    {"value":"sonnet","name":"Sonnet 5"}
]"#;

#[tokio::test(flavor = "multi_thread")]
async fn new_sessions_get_character_names_in_turn() {
    let repo = git_repo();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    core.open_workspace(repo.path()).await.unwrap();
    let first = core.new_session().await.unwrap();
    let second = core.new_session().await.unwrap();
    assert_eq!(core.session_info(first).unwrap().name, "Bramble");
    assert_eq!(core.session_info(second).unwrap().name, "Clementine");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_model_is_what_the_agent_says_and_follows_its_changes() {
    let repo = git_repo();
    let fake = FakeAgent::new(&format!(
        r#"{{{MODELS},"turns":[{{"chunks":["ok"],"setModel":"sonnet"}}]}}"#
    ));
    let core = core_with(&fake);
    let mut events = core.subscribe();
    core.open_workspace(repo.path()).await.unwrap();
    let id = core.new_session().await.unwrap();
    assert_eq!(
        core.session_info(id).unwrap().model.as_deref(),
        Some("Opus 4.8"),
        "the default option names the model it stands for"
    );

    core.send_prompt(id, "switch").await.unwrap();
    eventually("the new model", || {
        core.session_info(id).unwrap().model.as_deref() == Some("Sonnet 5")
    })
    .await;
    let mut announced = None;
    while let Ok(event) = events.try_recv() {
        if let CoreEvent::SessionModelChanged { session_id, model } = event {
            assert_eq!(session_id, id);
            announced = Some(model);
        }
    }
    assert_eq!(announced.as_deref(), Some("Sonnet 5"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_turn_marks_the_session_active_and_a_restart_remembers_it_and_the_model() {
    let repo = git_repo();
    let fake = FakeAgent::new(&format!(r#"{{{MODELS},"turns":[]}}"#));
    let active = {
        let core = core_with_state(&fake);
        let mut events = core.subscribe();
        core.open_workspace(repo.path()).await.unwrap();
        let id = core.new_session().await.unwrap();
        assert_eq!(
            core.session_info(id).unwrap().last_active,
            None,
            "not active yet"
        );
        core.send_prompt(id, "hello").await.unwrap();
        states_until(&mut events, id, SessionState::Idle).await;
        let active = core.session_info(id).unwrap().last_active.expect("active");
        eventually("saved", || {
            std::fs::read_to_string(fake.state_path())
                .is_ok_and(|s| s.contains(&active.to_string()))
        })
        .await;
        active
    }; // the editor quits

    let core = core_with_state(&fake);
    core.open_workspace(repo.path()).await.unwrap();
    let restored = &core.sessions()[0];
    assert_eq!(restored.name, "Bramble");
    assert_eq!(restored.last_active, Some(active));
    assert_eq!(
        restored.model.as_deref(),
        Some("Opus 4.8"),
        "shown before it's resumed"
    );
    let next = core.new_session().await.unwrap();
    assert_eq!(
        core.session_info(next).unwrap().name,
        "Clementine",
        "names carry on"
    );
}
