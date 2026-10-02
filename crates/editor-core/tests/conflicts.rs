//! Ticket 16: when an Agent changes a file open in a Manual editor, a clean editor is told to
//! reload it and a dirty one is told there's a conflict (and never to reload). Real temp
//! repositories, real file watching, and the fake agent writing the file.

mod support;

use std::path::{Path, PathBuf};
use std::time::Duration;

use editor_core::{Core, CoreEvent, SaveOver, SessionState, TranscriptItem};
use serde_json::{json, Value};
use support::*;
use tokio::sync::broadcast;

/// A Workspace with `a.rs` committed and open in the `main` window; `a.rs`'s path and version.
async fn open_file(fake: &FakeAgent) -> (tempfile::TempDir, Core, PathBuf, String) {
    let repo = git_repo();
    std::fs::write(repo.path().join("a.rs"), "fn a() {}\n").unwrap();
    let core = core_with(fake);
    core.open_workspace(repo.path()).await.unwrap();
    let path = core.worktrees()[0].path.join("a.rs");
    let version = core.read_file(&path).await.unwrap().version;
    // (Opening it is what has its Worktree watched.)
    core.document_opened(&path, "main", &version).await.unwrap();
    (repo, core, path, version)
}

/// Has the Agent run a turn with `writes` (to `a.rs`) and waits for it to end. Its states come on
/// a receiver of their own, so nothing the test waits for is swallowed.
async fn agent_writes(core: &Core, fake: &FakeAgent, writes: Value) {
    let turn = json!({ "write": writes, "chunks": ["done"] });
    fake.set_script(&json!({ "turns": [turn] }).to_string());
    let mut states = core.subscribe();
    let session = core.new_session().await.unwrap();
    core.send_prompt(session, "edit a.rs").await.unwrap();
    states_until(&mut states, session, SessionState::Idle).await;
}

/// What the editors are told about `path`: everything until things have been quiet for a moment.
async fn told_about(events: &mut broadcast::Receiver<CoreEvent>, path: &Path) -> Vec<CoreEvent> {
    let mut told = vec![];
    // Up to 5 s for the first, then 500 ms of quiet.
    let mut wait = Duration::from_secs(5);
    while let Ok(Ok(event)) = tokio::time::timeout(wait, events.recv()).await {
        if let CoreEvent::DocumentChangedOnDisk { path: p, .. }
        | CoreEvent::DocumentConflicted { path: p, .. }
        | CoreEvent::DocumentBackOnDisk { path: p, .. } = &event
        {
            if p == path {
                told.push(event);
                wait = Duration::from_millis(500);
            }
        }
    }
    told
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_editing_a_clean_open_file_reloads_it() {
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let (_repo, core, path, _) = open_file(&fake).await;
    let mut events = core.subscribe();
    agent_writes(
        &core,
        &fake,
        json!([{ "path": "a.rs", "text": "fn a() { agent }\n" }]),
    )
    .await;

    let told = told_about(&mut events, &path).await;
    assert!(
        matches!(&told[..], [CoreEvent::DocumentChangedOnDisk { window, .. }] if window == "main"),
        "{told:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_editing_a_dirty_open_file_is_a_conflict_never_a_reload() {
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let (_repo, core, path, _) = open_file(&fake).await;
    core.document_changed(&path, "main", true);
    let mut events = core.subscribe();
    agent_writes(
        &core,
        &fake,
        json!([{ "path": "a.rs", "text": "fn a() { agent }\n" }]),
    )
    .await;

    let told = told_about(&mut events, &path).await;
    assert!(
        matches!(&told[..], [CoreEvent::DocumentConflicted { window, deleted: false, .. }] if window == "main"),
        "{told:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn quick_edits_one_after_another_leave_the_editor_on_the_last() {
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let (_repo, core, path, _) = open_file(&fake).await;
    // The editor's part: reload whenever told to (as the frontend does).
    let reloader = {
        let core = core.clone();
        let mut events = core.subscribe();
        tokio::spawn(async move {
            while let Ok(event) = events.recv().await {
                if let CoreEvent::DocumentChangedOnDisk { path, window } = event {
                    let version = core.read_file(&path).await.unwrap().version;
                    core.document_opened(&path, &window, &version)
                        .await
                        .unwrap();
                }
            }
        })
    };
    agent_writes(
        &core,
        &fake,
        json!([
            { "path": "a.rs", "text": "fn a() { one }\n" },
            { "path": "a.rs", "text": "fn a() { two }\n", "afterMs": 250 },
        ]),
    )
    .await;

    let last = core.read_file(&path).await.unwrap().version;
    tokio::time::timeout(Duration::from_secs(5), async {
        while core.open_documents()[0].version != last {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("the editor reloaded the last edit");
    reloader.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_change_undone_takes_the_conflict_back() {
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let (_repo, core, path, _) = open_file(&fake).await;
    core.document_changed(&path, "main", true);
    let mut events = core.subscribe();
    agent_writes(
        &core,
        &fake,
        json!([
            { "path": "a.rs", "text": "fn a() { agent }\n" },
            { "path": "a.rs", "text": "fn a() {}\n", "afterMs": 600 },
        ]),
    )
    .await;

    let told = told_about(&mut events, &path).await;
    assert!(
        matches!(
            &told[..],
            [
                CoreEvent::DocumentConflicted { .. },
                CoreEvent::DocumentBackOnDisk { .. }
            ]
        ),
        "{told:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_deleted_open_file_is_a_conflict_even_when_clean() {
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let (_repo, core, path, _) = open_file(&fake).await;
    let mut events = core.subscribe();
    std::fs::remove_file(&path).unwrap();

    let told = told_about(&mut events, &path).await;
    assert!(
        matches!(
            &told[..],
            [CoreEvent::DocumentConflicted { deleted: true, .. }]
        ),
        "{told:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_editors_own_save_tells_only_the_other_windows_with_the_file() {
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let (_repo, core, path, version) = open_file(&fake).await;
    core.document_opened(&path, "editor-9", &version)
        .await
        .unwrap();
    let mut events = core.subscribe();
    core.save_file(
        &path,
        "fn a() { mine }\n",
        "\n",
        SaveOver::Version { version },
        "main",
    )
    .await
    .unwrap();

    let told = told_about(&mut events, &path).await;
    assert!(
        matches!(&told[..], [CoreEvent::DocumentChangedOnDisk { window, .. }] if window == "editor-9"),
        "{told:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_file_changed_before_it_was_reported_open_is_caught_at_once() {
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let (_repo, core, path, version) = open_file(&fake).await;
    std::fs::write(&path, "fn a() { meanwhile }\n").unwrap();
    let mut events = core.subscribe();
    // Another window read it before the change, and says so only now.
    core.document_opened(&path, "editor-3", &version)
        .await
        .unwrap();

    let told = told_about(&mut events, &path).await;
    assert!(
        told.iter().any(
            |e| matches!(e, CoreEvent::DocumentChangedOnDisk { window, .. } if window == "editor-3")
        ),
        "{told:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_edit_card_names_its_file_and_the_open_files_say_which_are_unsaved() {
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let (_repo, core, path, _) = open_file(&fake).await;
    let mut events = core.subscribe();
    core.document_changed(&path, "main", true);
    let documents = next_event(&mut events, "the open files", |e| match e {
        CoreEvent::DocumentsChanged { documents } => Some(documents),
        _ => None,
    })
    .await;
    assert!(documents.iter().any(|d| d.path == path && d.dirty));

    let edit = json!({ "permission": { "title": "Edit a.rs", "kind": "edit",
        "diff": { "path": path, "oldText": "fn a() {}\n", "newText": "fn b() {}\n" } } });
    let read = json!({ "permission": { "title": "Read a.rs", "kind": "read", "path": path } });
    // The latest card.
    let card = |session| {
        core.transcript(session)
            .unwrap()
            .into_iter()
            .rev()
            .find_map(|item| match item {
                TranscriptItem::Permission { request, .. } => Some(request),
                _ => None,
            })
            .expect("a card")
    };
    // (The fake agent reads its script once, as it starts.)
    fake.set_script(&json!({ "turns": [edit, read] }).to_string());
    let session = core.new_session().await.unwrap();
    core.send_prompt(session, "edit").await.unwrap();
    states_until(&mut events, session, SessionState::NeedsYou).await;
    let asked = card(session);
    assert_eq!(asked.file, Some(path.clone()));
    core.answer_permission(session, &asked.tool_call_id, &asked.options[0].id)
        .await
        .unwrap();
    states_until(&mut events, session, SessionState::Idle).await;

    // Reading a file with unsaved changes is nothing to warn about.
    core.send_prompt(session, "read").await.unwrap();
    states_until(&mut events, session, SessionState::NeedsYou).await;
    assert_eq!(card(session).title, "Read a.rs");
    assert_eq!(card(session).file, None);
}
