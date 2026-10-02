//! Ticket 10: Suspended sessions, resuming them, and recovering from an adapter crash.

mod support;

use editor_core::{CoreError, SessionId, SessionState, TranscriptItem};
use support::*;

const ECHO: &str = r#"{"turns":[]}"#;
const ASKS: &str = r#"{"turns":[{"permission":{"title":"Edit a.rs","kind":"edit"}}]}"#;

fn agent_texts(core: &editor_core::Core, id: SessionId) -> Vec<String> {
    core.transcript(id)
        .unwrap()
        .into_iter()
        .filter_map(|item| match item {
            TranscriptItem::Agent { text } => Some(text),
            _ => None,
        })
        .collect()
}

/// The ACP session id the core used for the `n`th prompt the fake agent received.
fn prompted_session(fake: &FakeAgent, n: usize) -> serde_json::Value {
    fake.received("session/prompt")[n]["params"]["sessionId"].clone()
}

#[tokio::test(flavor = "multi_thread")]
async fn suspend_stops_the_agent_and_sending_resumes_the_same_conversation() {
    let repo = git_repo();
    let fake = FakeAgent::new(ECHO);
    let core = core_with(&fake);
    core.open_workspace(repo.path()).await.unwrap();
    let id = core.new_session().await.unwrap();
    let mut events = core.subscribe();
    core.send_prompt(id, "one").await.unwrap();
    states_until(&mut events, id, SessionState::Idle).await;
    let acp_id = prompted_session(&fake, 0);

    core.suspend_session(id).await.unwrap();
    assert_eq!(
        core.session_info(id).unwrap().state,
        SessionState::Suspended
    );
    let closed = fake.received("session/close");
    assert_eq!(closed.len(), 1);
    assert_eq!(closed[0]["params"]["sessionId"], acp_id);
    assert_eq!(
        agent_texts(&core, id),
        ["Echo: one"],
        "the conversation stays"
    );

    core.send_prompt(id, "two").await.unwrap();
    let seen = states_until(&mut events, id, SessionState::Idle).await;
    assert!(seen.contains(&SessionState::Working), "{seen:?}");
    let resumed = fake.received("session/resume");
    assert_eq!(resumed.len(), 1);
    assert_eq!(
        resumed[0]["params"]["sessionId"], acp_id,
        "the same conversation"
    );
    assert_eq!(
        canonical(std::path::Path::new(
            resumed[0]["params"]["cwd"].as_str().unwrap()
        )),
        canonical(repo.path())
    );
    assert_eq!(prompted_session(&fake, 1), acp_id);
    assert_eq!(agent_texts(&core, id), ["Echo: one", "Echo: two"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_mid_turn_or_waiting_on_you_is_not_suspended() {
    let repo = git_repo();
    let fake = FakeAgent::new(ASKS);
    let core = core_with(&fake);
    core.open_workspace(repo.path()).await.unwrap();
    let id = core.new_session().await.unwrap();
    let mut events = core.subscribe();
    core.send_prompt(id, "edit").await.unwrap();
    states_until(&mut events, id, SessionState::NeedsYou).await;

    assert!(matches!(
        core.suspend_session(id).await,
        Err(CoreError::SessionBusy)
    ));
    assert!(fake.received("session/close").is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_permission_mode_survives_a_resume() {
    let repo = git_repo();
    let fake = FakeAgent::new(ECHO);
    let core = core_with(&fake);
    core.open_workspace(repo.path()).await.unwrap();
    let id = core.new_session().await.unwrap();
    core.suspend_session(id).await.unwrap();
    core.set_permission_mode(id, editor_core::PermissionMode::Plan)
        .await
        .unwrap();

    let mut events = core.subscribe();
    core.send_prompt(id, "plan it").await.unwrap();
    states_until(&mut events, id, SessionState::Idle).await;
    // Set while it was Suspended (nothing sent then), applied after the resume, before the prompt.
    let order: Vec<String> = fake
        .log()
        .into_iter()
        .filter_map(|m| {
            let method = m["method"].as_str()?;
            match method {
                "session/set_mode" => Some(format!("set_mode {}", m["params"]["modeId"].as_str()?)),
                "session/resume" | "session/prompt" | "session/close" => Some(method.to_owned()),
                _ => None,
            }
        })
        .collect();
    assert_eq!(
        order,
        [
            "session/close",
            "session/resume",
            "set_mode plan",
            "session/prompt"
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_running_session_is_not_resumed() {
    let repo = git_repo();
    let fake = FakeAgent::new(ECHO);
    let core = core_with(&fake);
    core.open_workspace(repo.path()).await.unwrap();
    let id = core.new_session().await.unwrap();

    assert!(matches!(
        core.resume_session(id).await,
        Err(CoreError::SessionRunning)
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn if_resuming_fails_the_message_is_not_sent_and_it_stays_suspended() {
    let repo = git_repo();
    let fake = FakeAgent::new(ECHO);
    let core = core_with(&fake);
    core.open_workspace(repo.path()).await.unwrap();
    let id = core.new_session().await.unwrap();
    core.suspend_session(id).await.unwrap();
    // The next adapter can't pick the conversation back up.
    fake.set_script(r#"{"turns":[],"failResume":true}"#);
    fake.kill();

    assert!(core.send_prompt(id, "lost?").await.is_err());
    assert_eq!(
        core.session_info(id).unwrap().state,
        SessionState::Suspended
    );
    assert!(
        !core
            .transcript(id)
            .unwrap()
            .iter()
            .any(|item| matches!(item, TranscriptItem::User { .. })),
        "not shown as sent (the composer keeps the text)"
    );
    assert!(fake.received("session/prompt").is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_suspended_session_stays_suspended_through_a_crash() {
    let repo = git_repo();
    let fake = FakeAgent::new(ECHO);
    let core = core_with(&fake);
    core.open_workspace(repo.path()).await.unwrap();
    let id = core.new_session().await.unwrap();
    core.suspend_session(id).await.unwrap();

    fake.kill();
    // Nothing announces a crash that needs no restart; give the editor a moment to notice it.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert_eq!(
        core.session_info(id).unwrap().state,
        SessionState::Suspended
    );
    assert_eq!(fake.starts(), 1, "no restart just for a Suspended session");

    let mut events = core.subscribe();
    core.send_prompt(id, "back").await.unwrap();
    states_until(&mut events, id, SessionState::Idle).await;
    assert_eq!(fake.starts(), 2, "started when it was needed");
    assert_eq!(agent_texts(&core, id), ["Echo: back"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_adapter_crash_resumes_idle_sessions_and_mid_turn_ones_become_exited() {
    let repo = git_repo();
    let fake = FakeAgent::new(ASKS);
    let core = core_with(&fake);
    core.open_workspace(repo.path()).await.unwrap();
    let idle = core.new_session().await.unwrap();
    let busy = core.new_session().await.unwrap();
    let mut events = core.subscribe();
    core.send_prompt(busy, "edit").await.unwrap();
    states_until(&mut events, busy, SessionState::NeedsYou).await;
    let busy_acp = prompted_session(&fake, 0);

    fake.set_script(ECHO); // what the restarted agent will do
    let mut busy_events = core.subscribe();
    fake.kill();
    states_until(&mut busy_events, busy, SessionState::Exited).await;
    eventually("the restart", || fake.starts() == 2).await;
    eventually("the idle session to come back", || {
        core.session_info(idle).unwrap().state == SessionState::Idle
            && !fake.received("session/resume").is_empty()
    })
    .await;
    let resumed = fake.received("session/resume");
    assert_eq!(resumed.len(), 1, "only the idle one: {resumed:?}");
    assert_ne!(resumed[0]["params"]["sessionId"], busy_acp);

    let mut events = core.subscribe();
    core.send_prompt(idle, "still here?").await.unwrap();
    states_until(&mut events, idle, SessionState::Idle).await;
    assert_eq!(agent_texts(&core, idle), ["Echo: still here?"]);
    assert_eq!(
        prompted_session(&fake, 1),
        resumed[0]["params"]["sessionId"],
        "the resumed conversation is the idle session's"
    );
    assert!(matches!(
        core.send_prompt(busy, "and you?").await,
        Err(CoreError::SessionExited)
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_exited_session_can_be_resumed() {
    let repo = git_repo();
    let fake = FakeAgent::new(ASKS);
    let core = core_with(&fake);
    core.open_workspace(repo.path()).await.unwrap();
    let id = core.new_session().await.unwrap();
    let mut events = core.subscribe();
    core.send_prompt(id, "edit").await.unwrap();
    states_until(&mut events, id, SessionState::NeedsYou).await;
    let acp_id = prompted_session(&fake, 0);
    fake.set_script(ECHO);
    fake.kill();
    states_until(&mut events, id, SessionState::Exited).await;

    core.resume_session(id).await.unwrap();
    assert_eq!(core.session_info(id).unwrap().state, SessionState::Idle);
    let resumed = fake.received("session/resume");
    assert_eq!(resumed.last().unwrap()["params"]["sessionId"], acp_id);

    let mut events = core.subscribe();
    core.send_prompt(id, "again").await.unwrap();
    states_until(&mut events, id, SessionState::Idle).await;
    assert_eq!(agent_texts(&core, id).last().unwrap(), "Echo: again");
}

#[tokio::test(flavor = "multi_thread")]
async fn sessions_made_after_a_restart_keep_their_own_ids() {
    let repo = git_repo();
    let fake = FakeAgent::new(ECHO);
    let core = core_with(&fake);
    core.open_workspace(repo.path()).await.unwrap();
    let first = core.new_session().await.unwrap();
    fake.kill();
    eventually("the restart", || fake.starts() == 2).await;
    eventually("the session to come back", || {
        core.session_info(first).unwrap().state == SessionState::Idle
            && !fake.received("session/resume").is_empty()
    })
    .await;

    let second = core.new_session().await.unwrap();
    let mut events = core.subscribe();
    core.send_prompt(second, "new").await.unwrap();
    states_until(&mut events, second, SessionState::Idle).await;
    core.send_prompt(first, "old").await.unwrap();
    states_until(&mut events, first, SessionState::Idle).await;
    assert_eq!(agent_texts(&core, second), ["Echo: new"]);
    assert_eq!(agent_texts(&core, first), ["Echo: old"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_adapter_that_keeps_crashing_is_not_restarted_forever() {
    let repo = git_repo();
    let fake = FakeAgent::new(ECHO);
    let core = core_with(&fake);
    core.open_workspace(repo.path()).await.unwrap();
    let id = core.new_session().await.unwrap();
    for crash in 1..=3 {
        fake.kill();
        eventually("the restart", || fake.starts() == crash + 1).await;
        eventually("the session to come back", || {
            core.session_info(id).unwrap().state == SessionState::Idle
                && fake.received("session/resume").len() == crash
        })
        .await;
    }

    let mut events = core.subscribe();
    fake.kill();
    states_until(&mut events, id, SessionState::Exited).await;
    assert_eq!(fake.starts(), 4, "no fifth start");
    assert_eq!(fake.received("session/resume").len(), 3);
    assert!(
        core.transcript(id).unwrap().iter().any(
            |item| matches!(item, TranscriptItem::Notice { text } if text.contains("keeps crashing"))
        ),
        "says why it stopped"
    );
    // Resuming by hand still works: the user decides to try again.
    core.resume_session(id).await.unwrap();
    assert_eq!(fake.starts(), 5);
}
