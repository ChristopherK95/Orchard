//! Ticket 11: closing Tabs without losing them, reopening them, and restoring after a restart.

mod support;

use editor_core::{
    Core, CoreError, CoreEvent, NewWorktree, PermissionMode, SessionId, SessionState,
    TranscriptItem,
};
use support::*;

const ECHO: &str = r#"{"turns":[]}"#;

async fn turn(core: &Core, id: SessionId, text: &str) {
    let mut events = core.subscribe();
    core.send_prompt(id, text).await.unwrap();
    states_until(&mut events, id, SessionState::Idle).await;
}

fn conversation(texts: &[(&str, &str)]) -> Vec<TranscriptItem> {
    texts
        .iter()
        .flat_map(|(user, agent)| {
            [
                TranscriptItem::User {
                    text: (*user).into(),
                    edit_notes: vec![],
                    attachments: vec![],
                },
                TranscriptItem::Agent {
                    text: (*agent).into(),
                },
            ]
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn closing_a_tab_stops_it_and_lists_it_under_recent_sessions() {
    let repo = git_repo();
    let fake = FakeAgent::new(ECHO);
    let core = core_with_state(&fake);
    let root = core.open_workspace(repo.path()).await.unwrap().root;
    let id = core.new_session().await.unwrap();
    turn(&core, id, "one").await;
    let acp_id = fake.received("session/prompt")[0]["params"]["sessionId"].clone();
    let mut events = core.subscribe();

    core.close_tab(id).await.unwrap();
    next_event(&mut events, "the Tab to close", |e| match e {
        CoreEvent::SessionClosed { session_id } if session_id == id => Some(()),
        _ => None,
    })
    .await;
    assert!(matches!(
        core.session_info(id),
        Err(CoreError::UnknownSession)
    ));
    assert_eq!(
        fake.received("session/close")[0]["params"]["sessionId"],
        acp_id
    );
    let recent = core.recent_sessions(&root);
    assert_eq!(recent.len(), 1);
    assert_eq!(recent[0].name, "Session 1");
    assert_eq!(recent[0].acp_id, acp_id.as_str().unwrap());
    assert!(
        core.worktrees().iter().any(|w| w.path == root),
        "closing a Worktree's last Tab leaves the Worktree"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn reopening_from_recent_sessions_resumes_the_conversation() {
    let repo = git_repo();
    let fake = FakeAgent::new(ECHO);
    let core = core_with_state(&fake);
    let root = core.open_workspace(repo.path()).await.unwrap().root;
    let id = core.new_session().await.unwrap();
    turn(&core, id, "one").await;
    core.close_tab(id).await.unwrap();
    let recent = core.recent_sessions(&root);

    let reopened = core.reopen_session(&recent[0].acp_id).await.unwrap();
    let info = core.session_info(reopened).unwrap();
    assert_eq!(info.name, "Session 1");
    assert_eq!(info.state, SessionState::Idle);
    assert_eq!(
        core.transcript(reopened).unwrap(),
        conversation(&[("one", "Echo: one")]),
        "the conversation is back"
    );
    assert!(core.recent_sessions(&root).is_empty(), "no longer recent");

    turn(&core, reopened, "two").await;
    assert_eq!(
        core.transcript(reopened).unwrap(),
        conversation(&[("one", "Echo: one"), ("two", "Echo: two")])
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn reopen_last_closed_brings_back_the_most_recent_close() {
    let repo = git_repo();
    let fake = FakeAgent::new(ECHO);
    let core = core_with_state(&fake);
    core.open_workspace(repo.path()).await.unwrap();
    let first = core.new_session().await.unwrap();
    turn(&core, first, "one").await;
    let second = core.new_session().await.unwrap();
    turn(&core, second, "two").await; // (only a used Tab has a conversation to reopen)
    core.close_tab(second).await.unwrap();
    core.close_tab(first).await.unwrap();

    let reopened = core.reopen_last_closed().await.unwrap().unwrap();
    assert_eq!(core.session_info(reopened).unwrap().name, "Session 1");
    let again = core.reopen_last_closed().await.unwrap().unwrap();
    assert_eq!(core.session_info(again).unwrap().name, "Session 2");
    assert_eq!(
        core.reopen_last_closed().await.unwrap(),
        None,
        "nothing left: not an error"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_restart_restores_every_open_tab_as_suspended() {
    let setup = RepoWithOrigin::new();
    let fake = FakeAgent::new(ECHO);
    let (root, worktree) = {
        let core = core_with_state(&fake);
        let root = core.open_workspace(&setup.repo()).await.unwrap().root;
        let worktree = core
            .create_worktree(NewWorktree::NewBranch {
                name: "agent/restored".into(),
                start_point: None,
            })
            .await
            .unwrap()
            .worktree
            .path;
        let main_tab = core.new_session().await.unwrap();
        turn(&core, main_tab, "remember me").await;
        let worktree_tab = core.new_session_in(&worktree).await.unwrap();
        turn(&core, worktree_tab, "in the worktree").await;
        core.set_permission_mode(worktree_tab, PermissionMode::Plan)
            .await
            .unwrap();
        core.show_session(worktree_tab).unwrap(); // the Tab last looked at
        let closed = core.new_session().await.unwrap();
        turn(&core, closed, "closed later").await; // (a used Tab: Recent sessions keeps it)
        core.close_tab(closed).await.unwrap();
        (root, worktree)
    }; // the editor quits

    let started_before = fake.starts();
    let core = core_with_state(&fake);
    core.open_workspace(&setup.repo()).await.unwrap();
    let restored = core.sessions();
    assert_eq!(
        core.last_active_session(),
        Some(restored[1].id),
        "shown again"
    );
    let summary: Vec<_> = restored
        .iter()
        .map(|s| {
            (
                s.name.as_str(),
                s.worktree.clone(),
                s.state,
                s.permission_mode,
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            (
                "Session 1",
                root.clone(),
                SessionState::Suspended,
                PermissionMode::AskForEdits
            ),
            (
                "Session 2",
                worktree.clone(),
                SessionState::Suspended,
                PermissionMode::Plan
            ),
        ],
        "same names, order, Worktrees and modes"
    );
    assert_eq!(core.recent_sessions(&root)[0].name, "Session 3");
    assert_eq!(fake.starts(), started_before, "restoring starts no Agent");

    // Opening a restored Tab shows its conversation, and it stays Suspended.
    let main_tab = restored[0].id;
    let mut stream = core.show_session(main_tab).unwrap();
    stream_until(
        &mut stream,
        &conversation(&[("remember me", "Echo: remember me")]),
    )
    .await;
    eventually("the Agent to be closed again", || {
        fake.received("session/close").len() == 2
    })
    .await;
    // Loading never showed it as anything but Suspended, and sending right away waits for it.
    assert_eq!(
        core.session_info(main_tab).unwrap().state,
        SessionState::Suspended
    );
    assert_eq!(fake.received("session/load").len(), 1);

    // Sending resumes it; the conversation isn't loaded twice.
    turn(&core, main_tab, "and now?").await;
    assert_eq!(
        core.transcript(main_tab).unwrap(),
        conversation(&[
            ("remember me", "Echo: remember me"),
            ("and now?", "Echo: and now?")
        ])
    );
    assert_eq!(fake.received("session/load").len(), 1);

    // A restored Tab sent to before it was ever shown loads its conversation on the way.
    let worktree_tab = restored[1].id;
    turn(&core, worktree_tab, "hello").await;
    assert_eq!(
        core.transcript(worktree_tab).unwrap(),
        conversation(&[
            ("in the worktree", "Echo: in the worktree"),
            ("hello", "Echo: hello")
        ])
    );
    assert_eq!(fake.received("session/load").len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn sending_while_a_restored_tab_loads_waits_for_it() {
    let repo = git_repo();
    let fake = FakeAgent::new(ECHO);
    {
        let core = core_with_state(&fake);
        core.open_workspace(repo.path()).await.unwrap();
        let id = core.new_session().await.unwrap();
        turn(&core, id, "before").await;
    }
    let core = core_with_state(&fake);
    core.open_workspace(repo.path()).await.unwrap();
    let id = core.sessions()[0].id;

    let _stream = core.show_session(id).unwrap(); // starts loading it
    turn(&core, id, "straight away").await; // doesn't fail: waits, then resumes
    assert_eq!(
        core.transcript(id).unwrap(),
        conversation(&[
            ("before", "Echo: before"),
            ("straight away", "Echo: straight away")
        ]),
        "loaded once, then the new turn"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn closing_and_reopening_survive_a_restart() {
    let repo = git_repo();
    let fake = FakeAgent::new(ECHO);
    {
        let core = core_with_state(&fake);
        core.open_workspace(repo.path()).await.unwrap();
        let id = core.new_session().await.unwrap();
        turn(&core, id, "kept").await;
        core.close_tab(id).await.unwrap();
    }
    let core = core_with_state(&fake);
    core.open_workspace(repo.path()).await.unwrap();
    assert!(core.sessions().is_empty(), "it was closed, not open");

    let reopened = core.reopen_last_closed().await.unwrap().unwrap();
    assert_eq!(
        core.transcript(reopened).unwrap(),
        conversation(&[("kept", "Echo: kept")])
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_tab_never_sent_a_prompt_is_neither_restored_nor_kept_in_recent_sessions() {
    let setup = RepoWithOrigin::new();
    let fake = FakeAgent::new(ECHO);
    let root = {
        let core = core_with_state(&fake);
        let root = core.open_workspace(&setup.repo()).await.unwrap().root;
        let used = core.new_session().await.unwrap();
        turn(&core, used, "keep me").await;
        let _unused = core.new_session().await.unwrap();
        let closed_unused = core.new_session().await.unwrap();
        core.close_tab(closed_unused).await.unwrap();
        assert!(core.recent_sessions(&root).is_empty(), "nothing to reopen");
        root
    }; // the editor quits

    let core = core_with_state(&fake);
    core.open_workspace(&setup.repo()).await.unwrap();
    let names: Vec<_> = core.sessions().into_iter().map(|s| s.name).collect();
    assert_eq!(
        names,
        ["Session 1"],
        "Claude Code has no conversation for the others"
    );
    assert!(core.recent_sessions(&root).is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_restored_tab_whose_conversation_claude_code_lacks_is_closed_when_shown() {
    let setup = RepoWithOrigin::new();
    let fake = FakeAgent::new(ECHO);
    {
        let core = core_with_state(&fake);
        core.open_workspace(&setup.repo()).await.unwrap();
        let id = core.new_session().await.unwrap();
        turn(&core, id, "soon forgotten").await;
    } // the editor quits
    std::fs::remove_file(fake.history_path()).unwrap(); // Claude Code's transcript is gone

    let core = core_with_state(&fake);
    core.open_workspace(&setup.repo()).await.unwrap();
    let mut events = core.subscribe();
    let id = core.sessions()[0].id;
    let _stream = core.show_session(id).unwrap();

    next_event(&mut events, "the Tab to close", |event| match event {
        CoreEvent::SessionClosed { session_id } if session_id == id => Some(()),
        _ => None,
    })
    .await;
    assert!(core.sessions().is_empty());
    drop(core);
    let core = core_with_state(&fake);
    core.open_workspace(&setup.repo()).await.unwrap();
    assert!(core.sessions().is_empty(), "and it isn't restored again");
}
