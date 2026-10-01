//! Ticket 01: one Agent session end to end, driven through the core's public API.

mod support;

use editor_core::{SessionState, TranscriptItem};
use support::*;

fn user(text: &str) -> TranscriptItem {
    TranscriptItem::User { text: text.into() }
}
fn agent(text: &str) -> TranscriptItem {
    TranscriptItem::Agent { text: text.into() }
}

#[tokio::test(flavor = "multi_thread")]
async fn prompt_streams_the_reply_into_the_tab_and_the_session_returns_to_idle() {
    let repo = git_repo();
    let fake = FakeAgent::new(r#"{"turns":[{"chunks":["Hel","lo ","world"]}]}"#);
    let core = core_with(&fake);

    core.open_workspace(repo.path()).await.unwrap();
    let mut events = core.subscribe();
    let session = core.new_session().await.unwrap();
    let mut stream = core.show_session(session).unwrap();

    core.send_prompt(session, "hi").await.unwrap();

    let expected = vec![user("hi"), agent("Hello world")];
    stream_until(&mut stream, &expected).await;
    assert_eq!(
        states_until(&mut events, session, SessionState::Idle).await,
        vec![SessionState::Working, SessionState::Idle]
    );
    assert_eq!(core.transcript(session).unwrap(), expected);
}

#[tokio::test(flavor = "multi_thread")]
async fn sessions_run_in_the_main_checkout_and_share_one_adapter_process() {
    let repo = git_repo();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    core.open_workspace(repo.path()).await.unwrap();

    core.new_session().await.unwrap();
    core.new_session().await.unwrap();

    let starts = fake.log().into_iter().filter(|m| m.get("started").is_some()).count();
    assert_eq!(starts, 1, "one shared adapter process");
    assert_eq!(fake.received("initialize").len(), 1);
    let new_sessions = fake.received("session/new");
    assert_eq!(new_sessions.len(), 2);
    for request in new_sessions {
        let cwd = request["params"]["cwd"].as_str().unwrap();
        assert_eq!(canonical(cwd.as_ref()), canonical(repo.path()));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn streamed_chunks_are_batched_rather_than_sent_one_by_one() {
    let repo = git_repo();
    // Chunks trickle in ~3 ms apart, so only the ~16 ms flush window can group them.
    let chunks: Vec<String> = (0..100).map(|i| format!("{i} ")).collect();
    let fake = FakeAgent::new(&serde_json::json!({ "turns": [{ "chunks": chunks, "delayMs": 3 }] }).to_string());
    let core = core_with(&fake);
    core.open_workspace(repo.path()).await.unwrap();
    let session = core.new_session().await.unwrap();
    let mut stream = core.show_session(session).unwrap();

    core.send_prompt(session, "count").await.unwrap();

    let expected = vec![user("count"), agent(&chunks.concat())];
    let batches = stream_until(&mut stream, &expected).await;
    assert!(batches < 40, "100 chunks arrived in {batches} batches");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_shows_exited_when_the_adapter_process_dies() {
    let repo = git_repo();
    let fake = FakeAgent::new(r#"{"turns":[{"chunks":["partial"],"exit":true}]}"#);
    let core = core_with(&fake);
    core.open_workspace(repo.path()).await.unwrap();
    let mut events = core.subscribe();
    let session = core.new_session().await.unwrap();

    core.send_prompt(session, "crash please").await.unwrap();

    assert_eq!(
        states_until(&mut events, session, SessionState::Exited).await,
        vec![SessionState::Working, SessionState::Exited]
    );
    assert!(core.send_prompt(session, "again").await.is_err(), "an Exited session takes no prompts");
}

#[tokio::test(flavor = "multi_thread")]
async fn opening_a_folder_that_is_not_a_git_repository_fails() {
    let not_a_repo = tempfile::tempdir().unwrap();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);

    let err = core.open_workspace(not_a_repo.path()).await.unwrap_err();

    assert!(matches!(err, editor_core::CoreError::NotARepository(_)), "{err}");
}
