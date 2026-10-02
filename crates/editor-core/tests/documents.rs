//! Ticket 15: the core tracks which files are open in which window, and whether they have unsaved
//! changes, including across a pop-out (the file's text and cursor moved to a new window).

mod support;

use editor_core::{CoreError, CoreEvent, PoppedOutFile};
use support::*;

fn popped(path: std::path::PathBuf, text: &str, saved: &str, version: &str) -> PoppedOutFile {
    PoppedOutFile {
        path,
        text: text.into(),
        saved_text: saved.into(),
        cursor: 9,
        anchor: 9,
        version: version.into(),
        line_ending: "\n".into(),
        wrap: false,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn opening_editing_saving_and_closing_are_tracked() {
    let repo = git_repo();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    core.open_workspace(repo.path()).await.unwrap();
    let path = repo.path().join("a.rs");
    std::fs::write(&path, "fn a() {}\n").unwrap();
    let opened = core.read_file(&path).await.unwrap();

    core.document_opened(&path, "main", &opened.version)
        .unwrap();
    let docs = core.open_documents();
    assert_eq!(docs.len(), 1);
    assert_eq!(
        docs[0].path,
        core.worktrees()[0].path.join("a.rs"),
        "canonical"
    );
    assert_eq!((docs[0].window.as_str(), docs[0].dirty), ("main", false));

    core.document_changed(&path, "main", true);
    assert!(core.open_documents()[0].dirty);
    // A save records the new version itself.
    let version = core
        .save_file(
            &path,
            "fn a() { 1 }\n",
            "\n",
            editor_core::SaveOver::Version {
                version: opened.version.clone(),
            },
            "main",
        )
        .await
        .unwrap();
    core.document_changed(&path, "main", false);
    assert_eq!(core.open_documents()[0].version, version);
    assert!(!core.open_documents()[0].dirty);

    core.document_closed(&path, "main");
    assert!(core.open_documents().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_popped_out_file_keeps_its_text_cursor_and_unsaved_state() {
    let repo = git_repo();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    core.open_workspace(repo.path()).await.unwrap();
    let path = repo.path().join("b.rs");
    std::fs::write(&path, "fn b() {}\n").unwrap();
    let opened = core.read_file(&path).await.unwrap();
    core.document_opened(&path, "main", &opened.version)
        .unwrap();
    core.document_changed(&path, "main", true);

    let window = core
        .pop_out(
            "main",
            popped(
                path.clone(),
                "fn b() { edited }\n",
                "fn b() {}\n",
                &opened.version,
            ),
        )
        .unwrap();
    // Tracked in the new window straight away (before it has even loaded), still unsaved; the
    // pane closing its tab afterwards doesn't change that.
    core.document_closed(&path, "main");
    let docs = core.open_documents();
    assert_eq!(docs.len(), 1);
    assert_eq!(
        (docs[0].window.as_str(), docs[0].dirty),
        (window.as_str(), true)
    );

    let collected = core.collect_pop_out(&window).unwrap();
    assert_eq!(collected.text, "fn b() { edited }\n");
    assert_eq!(collected.cursor, 9);
    assert_eq!(core.windows_with(&path), std::slice::from_ref(&window));

    // The window closes (after asking): its files are no longer open.
    core.window_closed(&window);
    assert!(core.open_documents().is_empty());
    assert!(matches!(
        core.collect_pop_out(&window),
        Err(CoreError::NoPopOut)
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pop_out_closed_before_it_showed_its_file_gives_the_text_back() {
    let repo = git_repo();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    core.open_workspace(repo.path()).await.unwrap();
    let path = repo.path().join("c.rs");
    std::fs::write(&path, "fn c() {}\n").unwrap();
    let mut events = core.subscribe();
    let window = core
        .pop_out("main", popped(path, "unsaved work\n", "fn c() {}\n", "v"))
        .unwrap();
    core.window_closed(&window);
    let (back_to, file) = next_event(&mut events, "the file given back", |e| match e {
        CoreEvent::PopOutReturned { window, file } => Some((window, file)),
        _ => None,
    })
    .await;
    assert_eq!(
        (back_to.as_str(), file.text.as_str()),
        ("main", "unsaved work\n")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn only_editable_files_are_tracked_or_popped_out() {
    let repo = git_repo();
    let elsewhere = tempfile::tempdir().unwrap();
    let outside = elsewhere.path().join("x.txt");
    std::fs::write(&outside, "").unwrap();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    core.open_workspace(repo.path()).await.unwrap();
    assert!(matches!(
        core.document_opened(&outside, "main", "v"),
        Err(CoreError::NotEditable(_))
    ));
    assert!(matches!(
        core.pop_out("main", popped(outside, "", "", "v")),
        Err(CoreError::NotEditable(_))
    ));
}
