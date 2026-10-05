//! Ticket 34: opening the Agent's conversations that were started outside the editor (in a
//! terminal, say) in a Tab.

mod support;

use std::path::Path;

use editor_core::{Core, SessionId, SessionState, TranscriptItem};
use serde_json::json;
use support::*;

async fn turn(core: &Core, id: SessionId, text: &str) {
    let mut events = core.subscribe();
    core.send_prompt(id, text).await.unwrap();
    states_until(&mut events, id, SessionState::Idle).await;
}

/// A script whose `session/list` also has `listed`, and a transcript for each of `conversations`.
fn agent_with(listed: serde_json::Value, conversations: serde_json::Value) -> FakeAgent {
    let fake = FakeAgent::new(&json!({ "turns": [], "listed": listed }).to_string());
    std::fs::write(fake.history_path(), conversations.to_string()).unwrap();
    fake
}

fn listed(id: &str, cwd: &Path, title: &str) -> serde_json::Value {
    json!({ "sessionId": id, "cwd": cwd, "title": title, "updatedAt": "2026-10-05T09:30:00.000Z" })
}

#[tokio::test(flavor = "multi_thread")]
async fn lists_the_worktrees_conversations_the_editor_doesnt_have() {
    let repo = git_repo();
    let elsewhere = tempfile::tempdir().unwrap();
    let root = canonical(repo.path());
    let fake = agent_with(
        json!([
            listed("terminal-1", &root, "Fix the login form"),
            listed("other-worktree", elsewhere.path(), "Not this Worktree's"),
        ]),
        json!({}),
    );
    let core = core_with_state(&fake);
    let root = core.open_workspace(repo.path()).await.unwrap().root;
    let open = core.new_session().await.unwrap();
    turn(&core, open, "one").await; // the Agent has a conversation for it now
    let closed = core.new_session().await.unwrap();
    turn(&core, closed, "two").await;
    core.close_tab(closed).await.unwrap();

    let others = core.other_conversations(&root).await.unwrap();
    assert_eq!(
        others.len(),
        1,
        "not a Tab, a Recent session or another folder's: {others:?}"
    );
    assert_eq!(others[0].acp_id, "terminal-1");
    assert_eq!(others[0].title.as_deref(), Some("Fix the login form"));
    assert_eq!(others[0].worktree, root);
    assert_eq!(
        others[0].updated_at.as_deref(),
        Some("2026-10-05T09:30:00.000Z")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn opening_one_loads_its_conversation_in_a_tab_named_after_it() {
    let repo = git_repo();
    let root = canonical(repo.path());
    let fake = agent_with(
        json!([listed("terminal-1", &root, "Fix the login form")]),
        json!({ "terminal-1": [
            { "role": "user", "text": "fix it" },
            { "role": "agent", "text": "Fixed." },
        ] }),
    );
    let core = core_with_state(&fake);
    let root = core.open_workspace(repo.path()).await.unwrap().root;

    let id = core
        .open_conversation(&root, "terminal-1", Some("Fix the login form"))
        .await
        .unwrap();
    let info = core.session_info(id).unwrap();
    assert_eq!(info.name, "Fix the login form");
    assert_eq!(info.worktree, root);
    assert_eq!(info.state, SessionState::Idle);
    assert_eq!(
        core.transcript(id).unwrap(),
        vec![
            TranscriptItem::User {
                text: "fix it".into(),
                edit_notes: vec![]
            },
            TranscriptItem::Agent {
                text: "Fixed.".into()
            },
        ],
        "the conversation is replayed"
    );
    assert_eq!(
        fake.received("session/load")[0]["params"]["sessionId"],
        "terminal-1"
    );
    assert!(
        core.other_conversations(&root).await.unwrap().is_empty(),
        "it's a Tab now"
    );

    turn(&core, id, "and the signup form").await;
    assert_eq!(
        core.transcript(id).unwrap().last(),
        Some(&TranscriptItem::Agent {
            text: "Echo: and the signup form".into()
        })
    );

    let again = core
        .open_conversation(&root, "terminal-1", Some("Fix the login form"))
        .await
        .unwrap();
    assert_eq!(again, id, "opening it again shows its Tab");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_long_or_missing_title_still_makes_a_tab_name() {
    let repo = git_repo();
    let fake = agent_with(
        json!([]),
        json!({
            "long": [{ "role": "user", "text": "a" }, { "role": "agent", "text": "b" }],
            "untitled": [{ "role": "user", "text": "c" }, { "role": "agent", "text": "d" }],
        }),
    );
    let core = core_with_state(&fake);
    let root = core.open_workspace(repo.path()).await.unwrap().root;

    let title = "\n  Refactor the session store so that it keeps every conversation\nsecond line";
    let long = core
        .open_conversation(&root, "long", Some(title))
        .await
        .unwrap();
    assert_eq!(
        core.session_info(long).unwrap().name,
        "Refactor the session store so that it k…"
    );
    let untitled = core
        .open_conversation(&root, "untitled", None)
        .await
        .unwrap();
    assert_eq!(core.session_info(untitled).unwrap().name, "Session 1");
}

#[tokio::test(flavor = "multi_thread")]
async fn one_without_a_transcript_opens_suspended_saying_why() {
    let repo = git_repo();
    let fake = agent_with(json!([]), json!({}));
    let core = core_with_state(&fake);
    let root = core.open_workspace(repo.path()).await.unwrap().root;

    let id = core
        .open_conversation(&root, "gone", Some("Gone"))
        .await
        .unwrap();
    assert_eq!(
        core.session_info(id).unwrap().state,
        SessionState::Suspended
    );
    assert!(matches!(
        core.transcript(id).unwrap().last(),
        Some(TranscriptItem::Notice { text }) if text.starts_with("Couldn't reopen the conversation")
    ));
}
