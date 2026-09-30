//! Ticket 03: permission cards and permission modes, driven through the core's public API.

mod support;

use editor_core::{
    DiffLine, DiffLineKind, PermissionMode, PermissionOptionKind, PermissionOutcome, SessionState, TranscriptItem,
};
use serde_json::json;
use support::*;

/// A turn that asks to edit `src/lib.rs` (changing `b` to `c`), then reports the answer it got.
fn edit_turn() -> serde_json::Value {
    json!({
        "permission": {
            "title": "Edit src/lib.rs",
            "kind": "edit",
            "diff": { "path": "src/lib.rs", "oldText": "a\nb\n", "newText": "a\nc\n" }
        },
        "chunks": [" done"]
    })
}

async fn session_waiting_for_permission(
    fake: &FakeAgent,
) -> (tempfile::TempDir, editor_core::Core, editor_core::SessionId, tokio::sync::broadcast::Receiver<editor_core::CoreEvent>) {
    let repo = git_repo();
    let core = core_with(fake);
    core.open_workspace(repo.path()).await.unwrap();
    let mut events = core.subscribe();
    let session = core.new_session().await.unwrap();
    core.send_prompt(session, "fix it").await.unwrap();
    assert_eq!(
        states_until(&mut events, session, SessionState::NeedsYou).await,
        vec![SessionState::Working, SessionState::NeedsYou]
    );
    (repo, core, session, events)
}

fn permission_item(items: &[TranscriptItem]) -> &editor_core::PermissionRequest {
    items
        .iter()
        .find_map(|item| match item {
            TranscriptItem::Permission { request, .. } => Some(request),
            _ => None,
        })
        .expect("a permission card in the transcript")
}

fn outcome(items: &[TranscriptItem]) -> Option<PermissionOutcome> {
    items.iter().find_map(|item| match item {
        TranscriptItem::Permission { outcome, .. } => outcome.clone(),
        _ => None,
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn a_permission_request_shows_a_card_with_the_tool_target_diff_and_options() {
    let fake = FakeAgent::new(&json!({ "turns": [edit_turn()] }).to_string());
    let (_repo, core, session, _events) = session_waiting_for_permission(&fake).await;

    let items = core.transcript(session).unwrap();
    let request = permission_item(&items);

    assert_eq!(request.title, "Edit src/lib.rs");
    assert_eq!(request.kind.as_deref(), Some("edit"));
    assert_eq!(request.target.as_deref(), Some("src/lib.rs"));
    assert_eq!(
        request.diff.as_deref(),
        Some(
            &[
                DiffLine { kind: DiffLineKind::Hunk, text: "@@ -1,2 +1,2 @@".into() },
                DiffLine { kind: DiffLineKind::Context, text: "a".into() },
                DiffLine { kind: DiffLineKind::Removed, text: "b".into() },
                DiffLine { kind: DiffLineKind::Added, text: "c".into() },
            ][..]
        )
    );
    let kinds: Vec<_> = request.options.iter().map(|o| o.kind).collect();
    assert_eq!(
        kinds,
        [PermissionOptionKind::AllowOnce, PermissionOptionKind::AllowAlways, PermissionOptionKind::RejectOnce]
    );
    assert_eq!(outcome(&items), None, "not answered yet");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_target_inside_the_worktree_is_shown_relative_to_it() {
    let repo = git_repo();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    // The adapter reports absolute paths under the cwd the core gave it: the Workspace root.
    let root = core.open_workspace(repo.path()).await.unwrap().root;
    let turn = json!({ "permission": { "title": "Edit", "kind": "edit",
        "diff": { "path": root.join("src").join("lib.rs"), "oldText": "a\n", "newText": "b\n" } } });
    fake.set_script(&json!({ "turns": [turn] }).to_string());
    let mut events = core.subscribe();
    let session = core.new_session().await.unwrap();

    core.send_prompt(session, "edit").await.unwrap();
    states_until(&mut events, session, SessionState::NeedsYou).await;

    let items = core.transcript(session).unwrap();
    let expected = std::path::Path::new("src").join("lib.rs").display().to_string();
    assert_eq!(permission_item(&items).target.as_deref(), Some(expected.as_str()));
}

#[tokio::test(flavor = "multi_thread")]
async fn allowing_sends_the_chosen_option_and_the_turn_carries_on() {
    let fake = FakeAgent::new(&json!({ "turns": [edit_turn()] }).to_string());
    let (_repo, core, session, mut events) = session_waiting_for_permission(&fake).await;
    let allow = permission_item(&core.transcript(session).unwrap()).options[0].id.clone();

    core.answer_permission(session, &allow).await.unwrap();

    assert_eq!(
        states_until(&mut events, session, SessionState::Idle).await,
        vec![SessionState::Working, SessionState::Idle]
    );
    let items = core.transcript(session).unwrap();
    assert_eq!(outcome(&items), Some(PermissionOutcome::Selected { option_id: allow.clone() }));
    // The fake agent echoes the option it received back into the chat.
    assert_eq!(items.last(), Some(&TranscriptItem::Agent { text: format!("[permission {allow}] done") }));
}

#[tokio::test(flavor = "multi_thread")]
async fn always_allow_and_deny_send_their_own_options() {
    for pick in [PermissionOptionKind::AllowAlways, PermissionOptionKind::RejectOnce] {
        let fake = FakeAgent::new(&json!({ "turns": [edit_turn()] }).to_string());
        let (_repo, core, session, mut events) = session_waiting_for_permission(&fake).await;
        let request = permission_item(&core.transcript(session).unwrap()).clone();
        let option = request.options.iter().find(|o| o.kind == pick).unwrap();

        core.answer_permission(session, &option.id).await.unwrap();
        states_until(&mut events, session, SessionState::Idle).await;

        let responses: Vec<_> = fake.log().into_iter().filter(|m| m.get("result").is_some() && m["id"] == "perm-1").collect();
        assert_eq!(responses.len(), 1);
        assert_eq!(responses[0]["result"]["outcome"], json!({ "outcome": "selected", "optionId": option.id }));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn answering_when_nothing_is_pending_fails() {
    let repo = git_repo();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    core.open_workspace(repo.path()).await.unwrap();
    let session = core.new_session().await.unwrap();

    assert!(core.answer_permission(session, "allow").await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_session_starts_in_ask_for_edits_even_if_the_agent_defaults_elsewhere() {
    let repo = git_repo();
    let fake = FakeAgent::new(r#"{"initialMode":"acceptEdits","turns":[]}"#);
    let core = core_with(&fake);
    core.open_workspace(repo.path()).await.unwrap();

    let session = core.new_session().await.unwrap();

    assert_eq!(core.session_info(session).unwrap().permission_mode, PermissionMode::AskForEdits);
    let set_mode = fake.received("session/set_mode");
    assert_eq!(set_mode.len(), 1);
    assert_eq!(set_mode[0]["params"]["modeId"], "default");
}

#[tokio::test(flavor = "multi_thread")]
async fn changing_the_permission_mode_is_applied_to_the_session() {
    let repo = git_repo();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    core.open_workspace(repo.path()).await.unwrap();
    let session = core.new_session().await.unwrap();
    assert!(fake.received("session/set_mode").is_empty(), "already in Ask for edits");

    core.set_permission_mode(session, PermissionMode::Plan).await.unwrap();

    let set_mode = fake.received("session/set_mode");
    assert_eq!(set_mode.len(), 1);
    assert_eq!(set_mode[0]["params"]["modeId"], "plan");
    assert_eq!(core.session_info(session).unwrap().permission_mode, PermissionMode::Plan);
}
