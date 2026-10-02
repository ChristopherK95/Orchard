//! Ticket 22: the Board's cards show each session's pending permission, which can be answered from
//! there (the frontend draws the Board; the core says what's waiting).

mod support;

use editor_core::SessionState;
use serde_json::json;
use support::*;

#[tokio::test(flavor = "multi_thread")]
async fn a_session_that_needs_you_has_its_oldest_question_pending_until_answered() {
    let repo = git_repo();
    let ask = |title: &str| {
        json!({ "permission": { "title": title, "kind": "edit",
            "diff": { "path": "a.rs", "oldText": "", "newText": "x" } } })
    };
    let fake = FakeAgent::new(&json!({ "turns": [ask("Edit a.rs")] }).to_string());
    let core = core_with(&fake);
    core.open_workspace(repo.path()).await.unwrap();
    let asking = core.new_session().await.unwrap();
    let idle = core.new_session().await.unwrap();
    assert_eq!(core.pending_permission(idle).unwrap(), None);

    let mut states = core.subscribe();
    core.send_prompt(asking, "edit").await.unwrap();
    states_until(&mut states, asking, SessionState::NeedsYou).await;
    let pending = core
        .pending_permission(asking)
        .unwrap()
        .expect("a question");
    assert_eq!(pending.title, "Edit a.rs");
    assert_eq!(core.pending_permission(idle).unwrap(), None);

    // Answered from the Board's card: nothing pending, and the turn carries on.
    core.answer_permission(asking, &pending.tool_call_id, &pending.options[0].id)
        .await
        .unwrap();
    states_until(&mut states, asking, SessionState::Idle).await;
    assert_eq!(core.pending_permission(asking).unwrap(), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn with_two_questions_the_oldest_is_pending_then_the_next() {
    let repo = git_repo();
    let question = |title: &str| {
        json!({ "title": title, "kind": "edit",
            "diff": { "path": "a.rs", "oldText": "", "newText": "x" } })
    };
    let turn = json!({ "permission": question("First"), "alsoAsk": question("Second") });
    let fake = FakeAgent::new(&json!({ "turns": [turn] }).to_string());
    let core = core_with(&fake);
    core.open_workspace(repo.path()).await.unwrap();
    let session = core.new_session().await.unwrap();
    let mut states = core.subscribe();
    core.send_prompt(session, "edit").await.unwrap();
    states_until(&mut states, session, SessionState::NeedsYou).await;
    // (Needs you comes with the first; wait for the second to be asked too.)
    tokio::time::timeout(TIMEOUT, async {
        loop {
            let cards = core
                .transcript(session)
                .unwrap()
                .iter()
                .filter(|i| matches!(i, editor_core::TranscriptItem::Permission { .. }))
                .count();
            if cards == 2 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("both questions asked");

    let first = core.pending_permission(session).unwrap().unwrap();
    assert_eq!(first.title, "First");
    core.answer_permission(session, &first.tool_call_id, &first.options[0].id)
        .await
        .unwrap();
    let second = core.pending_permission(session).unwrap().unwrap();
    assert_eq!(second.title, "Second");
    assert_eq!(
        core.session_info(session).unwrap().state,
        SessionState::NeedsYou
    );
}
