//! Ticket 17: when the user saves a file by hand, each session that read or edited it gets an Edit
//! note, sent with its next prompt; sessions that never looked at it get nothing, and none is woken.

mod support;

use std::path::PathBuf;

use editor_core::{Core, CoreEvent, SaveOver, SessionId, SessionState, TranscriptItem};
use serde_json::{json, Value};
use support::*;

/// A Workspace with `a.rs` committed; its path.
async fn workspace(fake: &FakeAgent) -> (tempfile::TempDir, Core, PathBuf) {
    let repo = git_repo();
    std::fs::write(repo.path().join("a.rs"), "fn a() {}\n").unwrap();
    let core = core_with(fake);
    core.open_workspace(repo.path()).await.unwrap();
    let path = core.worktrees()[0].path.join("a.rs");
    (repo, core, path)
}

/// A turn in which the Agent reads `path`.
fn read_turn(path: &std::path::Path) -> Value {
    json!({ "toolCalls": [{ "title": "Read a.rs", "kind": "read", "path": path }], "chunks": ["read it"] })
}

async fn prompt(core: &Core, session: SessionId, text: &str) {
    let mut states = core.subscribe();
    core.send_prompt(session, text).await.unwrap();
    states_until(&mut states, session, SessionState::Idle).await;
}

/// The user saves `a.rs` from the Manual editor.
async fn save(core: &Core, path: &std::path::Path, text: &str) {
    let version = core.read_file(path).await.unwrap().version;
    core.save_file(path, text, "\n", SaveOver::Version { version }, "main")
        .await
        .unwrap();
}

/// The text blocks of the last prompt the Agent got.
fn last_prompt(fake: &FakeAgent) -> Vec<String> {
    let prompts = fake.received("session/prompt");
    prompts.last().expect("a prompt")["params"]["prompt"]
        .as_array()
        .unwrap()
        .iter()
        .map(|block| block["text"].as_str().unwrap_or_default().to_owned())
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_that_read_the_file_gets_the_diff_with_its_next_prompt_and_others_nothing() {
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let (_repo, core, path) = workspace(&fake).await;
    fake.set_script(&json!({ "turns": [read_turn(&path)] }).to_string());
    let reader = core.new_session().await.unwrap();
    prompt(&core, reader, "look at a.rs").await;
    let other = core.new_session().await.unwrap();

    let mut events = core.subscribe();
    let prompts = fake.received("session/prompt").len();
    save(&core, &path, "fn a() { mine }\n").await;
    assert_eq!(
        fake.received("session/prompt").len(),
        prompts,
        "nobody woken"
    );
    let (to, notes) = next_event(&mut events, "an Edit note", |e| match e {
        CoreEvent::EditNotesChanged { session_id, notes } => Some((session_id, notes)),
        _ => None,
    })
    .await;
    assert_eq!(to, reader);
    assert_eq!(
        (notes[0].name.as_str(), notes[0].added, notes[0].removed),
        ("a.rs", 1, 1)
    );
    assert!(core.edit_notes(other).unwrap().is_empty(), "never read it");

    prompt(&core, other, "hello").await;
    assert_eq!(last_prompt(&fake), ["hello"]);

    prompt(&core, reader, "carry on").await;
    let sent = last_prompt(&fake);
    assert_eq!(sent.len(), 2, "{sent:?}");
    assert!(
        sent[0].contains("a.rs (+1 -1)") && sent[0].contains("+fn a() { mine }"),
        "{}",
        sent[0]
    );
    assert_eq!(sent[1], "carry on");
    assert!(core.edit_notes(reader).unwrap().is_empty(), "delivered");
    // Shown under the message it went with.
    let shown = core
        .transcript(reader)
        .unwrap()
        .into_iter()
        .rev()
        .find_map(|item| match item {
            TranscriptItem::User {
                text, edit_notes, ..
            } => Some((text, edit_notes)),
            _ => None,
        });
    assert_eq!(
        shown,
        Some(("carry on".to_owned(), vec!["a.rs (+1 −1)".to_owned()]))
    );

    // Delivered: the next prompt carries nothing more.
    prompt(&core, reader, "again").await;
    assert_eq!(last_prompt(&fake), ["again"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_suspended_session_keeps_its_notes_without_being_woken_until_it_resumes() {
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let (_repo, core, path) = workspace(&fake).await;
    fake.set_script(&json!({ "turns": [read_turn(&path)] }).to_string());
    let session = core.new_session().await.unwrap();
    prompt(&core, session, "look at a.rs").await;
    core.suspend_session(session).await.unwrap();
    let resumes = fake.received("session/resume").len();

    save(&core, &path, "fn a() { one }\n").await;
    save(&core, &path, "fn a() { two }\n").await;
    assert_eq!(
        core.session_info(session).unwrap().state,
        SessionState::Suspended
    );
    assert_eq!(fake.received("session/resume").len(), resumes, "not woken");
    let notes = core.edit_notes(session).unwrap();
    assert_eq!(notes.len(), 1, "one note for both saves");

    prompt(&core, session, "carry on").await;
    let sent = last_prompt(&fake);
    assert!(
        sent[0].contains("-fn a() {}") && sent[0].contains("+fn a() { two }"),
        "{}",
        sent[0]
    );
    assert!(!sent[0].contains("one"), "the saves combine: {}", sent[0]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_removed_note_isnt_sent() {
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let (_repo, core, path) = workspace(&fake).await;
    fake.set_script(&json!({ "turns": [read_turn(&path)] }).to_string());
    let session = core.new_session().await.unwrap();
    prompt(&core, session, "look at a.rs").await;
    save(&core, &path, "fn a() { mine }\n").await;
    assert_eq!(core.edit_notes(session).unwrap().len(), 1);

    core.remove_edit_note(session, &path).unwrap();
    assert!(core.edit_notes(session).unwrap().is_empty());
    prompt(&core, session, "carry on").await;
    assert_eq!(last_prompt(&fake), ["carry on"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn after_a_restart_a_restored_tab_still_gets_notes_for_what_it_read() {
    let repo = git_repo();
    std::fs::write(repo.path().join("a.rs"), "fn a() {}\n").unwrap();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let path = {
        let core = core_with_state(&fake);
        core.open_workspace(repo.path()).await.unwrap();
        let path = core.worktrees()[0].path.join("a.rs");
        fake.set_script(&json!({ "turns": [read_turn(&path)] }).to_string());
        let session = core.new_session().await.unwrap();
        prompt(&core, session, "look at a.rs").await;
        path
    }; // the editor quits

    let core = core_with_state(&fake);
    core.open_workspace(repo.path()).await.unwrap();
    let restored = core.sessions()[0].id;
    save(&core, &path, "fn a() { mine }\n").await;
    assert_eq!(core.edit_notes(restored).unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_agent_reading_the_file_itself_drops_its_note() {
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let (_repo, core, path) = workspace(&fake).await;
    // Turn 2 asks first, then reads a.rs once answered.
    let ask_then_read = json!({
        "permission": { "title": "Run tests", "kind": "execute" },
        "toolCallsAfter": [{ "title": "Read a.rs", "kind": "read", "path": path }],
        "chunks": ["done"]
    });
    fake.set_script(&json!({ "turns": [read_turn(&path), ask_then_read] }).to_string());
    let session = core.new_session().await.unwrap();
    prompt(&core, session, "look at a.rs").await;
    let mut states = core.subscribe();
    core.send_prompt(session, "run the tests").await.unwrap();
    states_until(&mut states, session, SessionState::NeedsYou).await;

    // Saved while the Agent waits; then it reads the file itself.
    save(&core, &path, "fn a() { mine }\n").await;
    assert_eq!(core.edit_notes(session).unwrap().len(), 1);
    let card = core
        .transcript(session)
        .unwrap()
        .into_iter()
        .find_map(|item| match item {
            TranscriptItem::Permission { request, .. } => Some(request),
            _ => None,
        })
        .unwrap();
    core.answer_permission(session, &card.tool_call_id, &card.options[0].id)
        .await
        .unwrap();
    states_until(&mut states, session, SessionState::Idle).await;
    assert!(
        core.edit_notes(session).unwrap().is_empty(),
        "it has seen it"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn notes_a_turn_never_delivered_wait_for_the_next() {
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let (_repo, core, path) = workspace(&fake).await;
    // The Agent reads the file, then dies on the next prompt.
    fake.set_script(&json!({ "turns": [read_turn(&path), { "exit": true }] }).to_string());
    let session = core.new_session().await.unwrap();
    prompt(&core, session, "look at a.rs").await;
    save(&core, &path, "fn a() { mine }\n").await;

    let mut events = core.subscribe();
    core.send_prompt(session, "carry on").await.unwrap();
    // Taken for the prompt, then put back once it fails (the session has Exited by then or soon).
    let back = next_event(&mut events, "the notes put back", |e| match e {
        CoreEvent::EditNotesChanged { notes, .. } if !notes.is_empty() => Some(notes),
        _ => None,
    })
    .await;
    assert_eq!(back.len(), 1);
    assert_eq!(core.edit_notes(session).unwrap().len(), 1, "put back");
}
