//! Ticket 02: the Agent's tool calls reach the transcript as compact rows.

mod support;

use editor_core::{SessionState, ToolCallStatus, TranscriptItem};
use serde_json::json;
use support::*;

#[tokio::test(flavor = "multi_thread")]
async fn tool_calls_appear_in_the_transcript_and_follow_their_updates() {
    let repo = git_repo();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    let root = core.open_workspace(repo.path()).await.unwrap().root;
    let turn = json!({
        "toolCalls": [
            { "title": "Read src/lib.rs", "kind": "read", "path": root.join("src").join("lib.rs") },
            { "title": "cargo test", "kind": "execute", "status": "failed" }
        ],
        "chunks": ["Tests fail."]
    });
    fake.set_script(&json!({ "turns": [turn] }).to_string());
    let mut events = core.subscribe();
    let session = core.new_session().await.unwrap();
    let mut stream = core.show_session(session).unwrap();

    core.send_prompt(session, "run the tests").await.unwrap();

    let expected = vec![
        TranscriptItem::User {
            text: "run the tests".into(),
            edit_notes: vec![],
        },
        TranscriptItem::ToolCall {
            tool_call_id: "call-1".into(),
            title: "Read src/lib.rs".into(),
            tool_kind: Some("read".into()),
            target: Some(
                std::path::Path::new("src")
                    .join("lib.rs")
                    .display()
                    .to_string(),
            ),
            status: ToolCallStatus::Completed,
        },
        TranscriptItem::ToolCall {
            tool_call_id: "call-2".into(),
            title: "cargo test".into(),
            tool_kind: Some("execute".into()),
            target: None,
            status: ToolCallStatus::Failed,
        },
        TranscriptItem::Agent {
            text: "Tests fail.".into(),
        },
    ];
    stream_until(&mut stream, &expected).await;
    states_until(&mut events, session, SessionState::Idle).await;
    assert_eq!(core.transcript(session).unwrap(), expected);
}

#[tokio::test(flavor = "multi_thread")]
async fn tool_calls_in_a_background_tab_dont_count_as_unread() {
    let repo = git_repo();
    let fake = FakeAgent::new(
        r#"{"turns":[{"toolCalls":[{"title":"Read a.rs","kind":"read"}],"chunks":["Read it."]}]}"#,
    );
    let core = core_with(&fake);
    core.open_workspace(repo.path()).await.unwrap();
    let mut events = core.subscribe();
    let a = core.new_session().await.unwrap();
    let b = core.new_session().await.unwrap();
    let _visible = core.show_session(a).unwrap();

    core.send_prompt(b, "read").await.unwrap();
    states_until(&mut events, b, SessionState::Idle).await;

    assert_eq!(
        core.session_info(b).unwrap().unread,
        1,
        "only the Agent's message is unread news"
    );
}
