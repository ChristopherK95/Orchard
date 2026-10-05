//! Ticket 04: several Agent sessions per Worktree; only the visible Tab streams its transcript.

mod support;

use editor_core::{CoreEvent, SessionState, TranscriptDelta, TranscriptItem, TRANSCRIPT_PAGE};
use serde_json::json;
use support::*;

#[tokio::test(flavor = "multi_thread")]
async fn sessions_run_side_by_side_in_one_worktree_and_each_reports_its_state() {
    let repo = git_repo();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    core.open_workspace(repo.path()).await.unwrap();
    let mut events_a = core.subscribe();
    let mut events_b = core.subscribe();
    let a = core.new_session().await.unwrap();
    let b = core.new_session().await.unwrap();
    let mut stream_a = core.show_session(a).unwrap();

    core.send_prompt(a, "one").await.unwrap();
    core.send_prompt(b, "two").await.unwrap();

    assert_eq!(
        states_until(&mut events_a, a, SessionState::Idle).await,
        [SessionState::Working, SessionState::Idle]
    );
    assert_eq!(
        states_until(&mut events_b, b, SessionState::Idle).await,
        [SessionState::Working, SessionState::Idle]
    );
    let worktree = core.session_info(a).unwrap().worktree;
    assert_eq!(
        core.session_info(b).unwrap().worktree,
        worktree,
        "same Worktree"
    );
    stream_until(
        &mut stream_a,
        &[
            TranscriptItem::User {
                text: "one".into(),
                edit_notes: vec![],
                attachments: vec![],
            },
            TranscriptItem::Agent {
                text: "Echo: one".into(),
            },
        ],
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn showing_another_session_ends_the_previous_tabs_stream() {
    let repo = git_repo();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    core.open_workspace(repo.path()).await.unwrap();
    let a = core.new_session().await.unwrap();
    let b = core.new_session().await.unwrap();
    let mut stream_a = core.show_session(a).unwrap();
    assert!(matches!(
        stream_a.next().await.unwrap()[0],
        TranscriptDelta::Reset { .. }
    ));

    let _stream_b = core.show_session(b).unwrap();

    let ended =
        tokio::time::timeout(TIMEOUT, async { while stream_a.next().await.is_some() {} }).await;
    assert!(ended.is_ok(), "a background Tab gets no transcript stream");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_background_sessions_output_never_reaches_the_visible_tabs_stream() {
    let repo = git_repo();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    core.open_workspace(repo.path()).await.unwrap();
    let mut events = core.subscribe();
    let a = core.new_session().await.unwrap();
    let b = core.new_session().await.unwrap();
    let mut stream_a = core.show_session(a).unwrap();
    assert!(matches!(
        stream_a.next().await.unwrap()[..],
        [TranscriptDelta::Reset { .. }]
    ));

    core.send_prompt(b, "background work").await.unwrap();
    states_until(&mut events, b, SessionState::Idle).await;

    let nothing =
        tokio::time::timeout(std::time::Duration::from_millis(300), stream_a.next()).await;
    assert!(
        nothing.is_err(),
        "A's stream carried something: {nothing:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_background_session_counts_unread_output_until_it_is_shown() {
    let repo = git_repo();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    core.open_workspace(repo.path()).await.unwrap();
    let mut events = core.subscribe();
    let a = core.new_session().await.unwrap();
    let b = core.new_session().await.unwrap();
    let _visible = core.show_session(a).unwrap();

    core.send_prompt(b, "in the background").await.unwrap();
    states_until(&mut events, b, SessionState::Idle).await;

    assert_eq!(
        core.session_info(b).unwrap().unread,
        1,
        "one Agent message arrived unseen"
    );
    assert_eq!(core.session_info(a).unwrap().unread, 0);
    let mut events = core.subscribe();
    let mut stream_b = core.show_session(b).unwrap();
    assert_eq!(
        core.session_info(b).unwrap().unread,
        0,
        "showing the Tab marks it read"
    );
    assert!(matches!(
        events.try_recv(),
        Ok(CoreEvent::SessionUnreadChanged { session_id, unread: 0 }) if session_id == b
    ));
    stream_until(
        &mut stream_b,
        &[
            TranscriptItem::User {
                text: "in the background".into(),
                edit_notes: vec![],
                attachments: vec![],
            },
            TranscriptItem::Agent {
                text: "Echo: in the background".into(),
            },
        ],
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn showing_a_long_transcript_sends_the_latest_page_and_older_pages_on_request() {
    let repo = git_repo();
    let messages: Vec<String> = (0..TRANSCRIPT_PAGE + 50)
        .map(|i| format!("message {i}"))
        .collect();
    let fake = FakeAgent::new(&json!({ "turns": [{ "messages": messages }] }).to_string());
    let core = core_with(&fake);
    core.open_workspace(repo.path()).await.unwrap();
    let mut events = core.subscribe();
    let session = core.new_session().await.unwrap();
    core.send_prompt(session, "talk a lot").await.unwrap();
    states_until(&mut events, session, SessionState::Idle).await;
    let everything = core.transcript(session).unwrap();
    assert_eq!(
        everything.len(),
        1 + messages.len(),
        "one prompt, then one item per Agent message"
    );

    let mut stream = core.show_session(session).unwrap();

    let first = stream.next().await.unwrap();
    let TranscriptDelta::Reset { start, items } = &first[0] else {
        panic!("a stream starts with a Reset")
    };
    assert_eq!(items.len(), TRANSCRIPT_PAGE);
    assert_eq!(*start, everything.len() - TRANSCRIPT_PAGE);
    assert_eq!(items[..], everything[*start..]);
    let older = core.transcript_page_before(session, *start).unwrap();
    assert_eq!(
        (older.start, &older.items[..]),
        (0, &everything[..*start]),
        "the page before the first one sent"
    );
    assert!(
        core.transcript_page_before(session, 0)
            .unwrap()
            .items
            .is_empty(),
        "nothing before the beginning"
    );
}
