//! Ticket 28: the Columns view. Each column is a view slot with its own visible Tab, streaming at
//! the same time as the others; pinned Worktrees are remembered across restarts.

mod support;

use editor_core::{NewWorktree, SessionState, TranscriptDelta, TranscriptItem};
use support::*;

const ECHO: &str = r#"{"turns":[]}"#;

fn exchange(text: &str) -> [TranscriptItem; 2] {
    [
        TranscriptItem::User {
            text: text.into(),
            edit_notes: vec![],
        },
        TranscriptItem::Agent {
            text: format!("Echo: {text}"),
        },
    ]
}

#[tokio::test(flavor = "multi_thread")]
async fn tabs_in_different_slots_stream_side_by_side_and_count_nothing_unread() {
    let repo = git_repo();
    let fake = FakeAgent::new(ECHO);
    let core = core_with(&fake);
    core.open_workspace(repo.path()).await.unwrap();
    let mut events = core.subscribe();
    let a = core.new_session().await.unwrap();
    let b = core.new_session().await.unwrap();
    let mut stream_a = core.show_session_in("column:a", a).unwrap();
    let mut stream_b = core.show_session_in("column:b", b).unwrap();

    core.send_prompt(a, "left").await.unwrap();
    core.send_prompt(b, "right").await.unwrap();
    states_until(&mut events, a, SessionState::Idle).await;

    stream_until(&mut stream_a, &exchange("left")).await;
    stream_until(&mut stream_b, &exchange("right")).await;
    assert_eq!(core.session_info(a).unwrap().unread, 0);
    assert_eq!(
        core.session_info(b).unwrap().unread,
        0,
        "both Tabs are visible"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn showing_another_tab_in_a_slot_ends_only_that_slots_stream() {
    let repo = git_repo();
    let fake = FakeAgent::new(ECHO);
    let core = core_with(&fake);
    core.open_workspace(repo.path()).await.unwrap();
    let mut events = core.subscribe();
    let a = core.new_session().await.unwrap();
    let b = core.new_session().await.unwrap();
    let c = core.new_session().await.unwrap();
    let mut stream_a = core.show_session_in("column:a", a).unwrap();
    let mut stream_b = core.show_session_in("column:b", b).unwrap();
    assert!(matches!(
        stream_a.next().await.unwrap()[0],
        TranscriptDelta::Reset { .. }
    ));
    assert!(matches!(
        stream_b.next().await.unwrap()[0],
        TranscriptDelta::Reset { .. }
    ));

    let _stream_c = core.show_session_in("column:b", c).unwrap();

    let ended =
        tokio::time::timeout(TIMEOUT, async { while stream_b.next().await.is_some() {} }).await;
    assert!(ended.is_ok(), "column b now shows c");
    core.send_prompt(a, "still here").await.unwrap();
    states_until(&mut events, a, SessionState::Idle).await;
    stream_until(&mut stream_a, &exchange("still here")).await;

    core.send_prompt(b, "unseen").await.unwrap();
    states_until(&mut events, b, SessionState::Idle).await;
    assert_eq!(
        core.session_info(b).unwrap().unread,
        1,
        "b is no longer visible anywhere"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn hiding_one_slot_keeps_a_tab_another_slot_shows_visible() {
    let repo = git_repo();
    let fake = FakeAgent::new(ECHO);
    let core = core_with(&fake);
    core.open_workspace(repo.path()).await.unwrap();
    let mut events = core.subscribe();
    let a = core.new_session().await.unwrap();
    let _tabs_view = core.show_session(a).unwrap();
    let mut column = core.show_session_in("column:a", a).unwrap();

    core.hide_tab_in(editor_core::TABS_SLOT); // back from the Tabs view to the Columns view

    core.send_prompt(a, "seen in the column").await.unwrap();
    states_until(&mut events, a, SessionState::Idle).await;
    stream_until(&mut column, &exchange("seen in the column")).await;
    assert_eq!(core.session_info(a).unwrap().unread, 0);

    core.hide_tab_in("column:a");
    core.send_prompt(a, "now unseen").await.unwrap();
    states_until(&mut events, a, SessionState::Idle).await;
    assert_eq!(core.session_info(a).unwrap().unread, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn pinned_worktrees_survive_a_restart_and_unknown_ones_are_refused() {
    let setup = RepoWithOrigin::new();
    let fake = FakeAgent::new(ECHO);
    let (root, worktree) = {
        let core = core_with_state(&fake);
        let root = core.open_workspace(&setup.repo()).await.unwrap().root;
        let worktree = core
            .create_worktree(NewWorktree::NewBranch {
                name: "agent/pinned".into(),
                start_point: None,
            })
            .await
            .unwrap()
            .worktree
            .path;
        assert_eq!(
            core.set_pinned(&worktree, true).unwrap(),
            [worktree.clone()]
        );
        assert_eq!(
            core.set_pinned(&root, true).unwrap(),
            [worktree.clone(), root.clone()]
        );
        assert_eq!(core.set_pinned(&root, false).unwrap(), [worktree.clone()]);
        assert!(core.set_pinned(&root.join("nowhere"), true).is_err());
        (root, worktree)
    }; // the editor quits

    let core = core_with_state(&fake);
    assert_eq!(core.open_workspace(&setup.repo()).await.unwrap().root, root);
    assert_eq!(core.pinned_worktrees(), [worktree]);
}
