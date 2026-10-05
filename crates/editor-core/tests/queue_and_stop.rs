//! Ticket 39: queuing a message while the Agent works, stopping a turn, and attaching files and
//! images to a prompt, driven through the core's public API.

mod support;

use editor_core::{
    Attachment, CoreError, PermissionOutcome, QueuedPrompt, SessionState, TranscriptItem,
};
use serde_json::json;
use support::*;

/// A turn that asks to edit `src/lib.rs`, then reports the answer it got.
fn edit_turn() -> serde_json::Value {
    json!({
        "permission": {
            "title": "Edit src/lib.rs",
            "kind": "edit",
            "diff": { "path": "src/lib.rs", "oldText": "a\n", "newText": "b\n" }
        },
        "chunks": [" done"]
    })
}

async fn started(
    script: serde_json::Value,
) -> (
    tempfile::TempDir,
    FakeAgent,
    editor_core::Core,
    editor_core::SessionId,
    tokio::sync::broadcast::Receiver<editor_core::CoreEvent>,
) {
    let repo = git_repo();
    let fake = FakeAgent::new(&script.to_string());
    let core = core_with(&fake);
    core.open_workspace(repo.path()).await.unwrap();
    let events = core.subscribe();
    let session = core.new_session().await.unwrap();
    (repo, fake, core, session, events)
}

fn queued(text: &str) -> Option<QueuedPrompt> {
    Some(QueuedPrompt {
        text: text.into(),
        attachments: vec![],
    })
}

fn user_messages(items: &[TranscriptItem]) -> Vec<String> {
    items
        .iter()
        .filter_map(|item| match item {
            TranscriptItem::User { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

fn prompts_sent(fake: &FakeAgent) -> Vec<serde_json::Value> {
    fake.received("session/prompt")
        .into_iter()
        .map(|m| m["params"]["prompt"].clone())
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_message_queued_while_asking_for_permission_is_sent_once_the_turn_ends() {
    let (_repo, fake, core, session, mut events) = started(json!({ "turns": [edit_turn()] })).await;
    core.send_prompt(session, "fix it").await.unwrap();
    states_until(&mut events, session, SessionState::NeedsYou).await;

    core.queue_prompt(session, "and then the tests", vec![])
        .await
        .unwrap();
    assert_eq!(
        core.queued_prompt(session).unwrap(),
        queued("and then the tests")
    );
    assert_eq!(prompts_sent(&fake).len(), 1, "not sent while the turn runs");

    let request = core
        .pending_permission(session)
        .unwrap()
        .expect("a permission card");
    let (tool_call, allow) = (request.tool_call_id, request.options[0].id.clone());
    core.answer_permission(session, &tool_call, &allow)
        .await
        .unwrap();
    // One turn straight after the other: it isn't Idle in between.
    let states = states_until(&mut events, session, SessionState::Idle).await;
    assert_eq!(states, [SessionState::Working, SessionState::Idle]);

    assert_eq!(core.queued_prompt(session).unwrap(), None);
    assert_eq!(
        user_messages(&core.transcript(session).unwrap()),
        ["fix it", "and then the tests"]
    );
    let prompts = prompts_sent(&fake);
    assert_eq!(prompts.len(), 2);
    assert_eq!(prompts[1][0]["text"], "and then the tests");
}

#[tokio::test(flavor = "multi_thread")]
async fn queuing_again_adds_to_the_queued_message_and_it_can_be_taken_back() {
    let (_repo, _fake, core, session, mut events) =
        started(json!({ "turns": [{ "untilCancelled": true }] })).await;
    core.send_prompt(session, "go").await.unwrap();
    states_until(&mut events, session, SessionState::Working).await;

    core.queue_prompt(session, "one", vec![]).await.unwrap();
    core.queue_prompt(session, "two", vec![]).await.unwrap();
    assert_eq!(core.queued_prompt(session).unwrap(), queued("one\n\ntwo"));

    assert_eq!(
        core.take_queued_prompt(session).unwrap(),
        queued("one\n\ntwo")
    );
    assert_eq!(core.queued_prompt(session).unwrap(), None, "won't be sent");
}

#[tokio::test(flavor = "multi_thread")]
async fn queuing_once_the_turn_has_ended_sends_it_straight_away() {
    let (_repo, fake, core, session, mut events) = started(json!({ "turns": [] })).await;
    core.queue_prompt(session, "hello", vec![]).await.unwrap();
    states_until(&mut events, session, SessionState::Idle).await;
    assert_eq!(core.queued_prompt(session).unwrap(), None);
    assert_eq!(prompts_sent(&fake).len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn stop_cancels_the_turn_and_keeps_the_queued_message() {
    let (_repo, fake, core, session, mut events) =
        started(json!({ "turns": [{ "untilCancelled": true }] })).await;
    core.send_prompt(session, "go").await.unwrap();
    states_until(&mut events, session, SessionState::Working).await;
    core.queue_prompt(session, "later", vec![]).await.unwrap();

    core.cancel_turn(session).unwrap();
    states_until(&mut events, session, SessionState::Idle).await;

    assert_eq!(fake.received("session/cancel").len(), 1);
    assert_eq!(
        prompts_sent(&fake).len(),
        1,
        "the queued message isn't sent"
    );
    assert_eq!(core.queued_prompt(session).unwrap(), queued("later"));
    let items = core.transcript(session).unwrap();
    assert_eq!(
        items.last(),
        Some(&TranscriptItem::Notice {
            text: "You stopped the turn.".into()
        })
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn stop_cancels_an_open_permission_card() {
    let (_repo, _fake, core, session, mut events) =
        started(json!({ "turns": [edit_turn()] })).await;
    core.send_prompt(session, "fix it").await.unwrap();
    states_until(&mut events, session, SessionState::NeedsYou).await;

    core.cancel_turn(session).unwrap();
    states_until(&mut events, session, SessionState::Idle).await;

    let outcome = core
        .transcript(session)
        .unwrap()
        .into_iter()
        .find_map(|item| match item {
            TranscriptItem::Permission { outcome, .. } => outcome,
            _ => None,
        });
    assert_eq!(outcome, Some(PermissionOutcome::Cancelled));
}

#[tokio::test(flavor = "multi_thread")]
async fn stop_with_no_turn_running_does_nothing() {
    let (_repo, fake, core, session, _events) = started(json!({ "turns": [] })).await;
    core.cancel_turn(session).unwrap();
    assert!(fake.received("session/cancel").is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_queued_message_stays_unsent_when_the_agent_exits_mid_turn() {
    let (_repo, fake, core, session, mut events) =
        started(json!({ "turns": [{ "untilCancelled": true }] })).await;
    core.send_prompt(session, "go").await.unwrap();
    states_until(&mut events, session, SessionState::Working).await;
    core.queue_prompt(session, "later", vec![]).await.unwrap();

    fake.kill();
    states_until(&mut events, session, SessionState::Exited).await;
    assert_eq!(core.queued_prompt(session).unwrap(), queued("later"));
}

#[tokio::test(flavor = "multi_thread")]
async fn attachments_go_with_the_prompt_as_content_blocks() {
    let (repo, fake, core, session, mut events) = started(json!({ "turns": [] })).await;
    let image = core
        .attach_data(session, "shot.png", Some("image/png"), "AQID")
        .await
        .unwrap();
    let notes = repo.path().join("notes.txt");
    std::fs::write(&notes, "remember this").unwrap();
    let text = core.attach_file(session, &notes).await.unwrap();
    assert!(matches!(text, Attachment::Text { .. }));

    core.send_prompt_with(session, "look", vec![image, text])
        .await
        .unwrap();
    states_until(&mut events, session, SessionState::Idle).await;

    let prompt = &prompts_sent(&fake)[0];
    assert_eq!(prompt[0], json!({ "type": "text", "text": "look" }));
    assert_eq!(
        prompt[1],
        json!({ "type": "image", "mimeType": "image/png", "data": "AQID" })
    );
    assert_eq!(prompt[2]["type"], "resource");
    assert_eq!(prompt[2]["resource"]["text"], "remember this");
    let attached = core
        .transcript(session)
        .unwrap()
        .into_iter()
        .find_map(|item| match item {
            TranscriptItem::User { attachments, .. } => Some(attachments),
            _ => None,
        });
    assert_eq!(attached, Some(vec!["shot.png".into(), "notes.txt".into()]));
}

#[tokio::test(flavor = "multi_thread")]
async fn kinds_the_agent_doesnt_take_are_refused_with_a_reason() {
    let (repo, _fake, core, session, _events) =
        started(json!({ "turns": [], "promptCapabilities": {} })).await;
    let err = core
        .attach_data(session, "shot.png", Some("image/png"), "AQID")
        .await
        .unwrap_err();
    assert!(matches!(&err, CoreError::Attachment(why) if why.contains("doesn't take images")));
    let notes = repo.path().join("notes.txt");
    std::fs::write(&notes, "x").unwrap();
    let err = core.attach_file(session, &notes).await.unwrap_err();
    assert!(matches!(&err, CoreError::Attachment(why) if why.contains("doesn't take files")));
}
