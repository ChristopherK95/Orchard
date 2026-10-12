//! Issue 47 (for 46): each Agent session's start, state changes and close are recorded, with times,
//! in a log kept across restarts.

mod support;

use editor_core::{Core, SessionId, SessionState};
use serde_json::Value;
use support::*;

const ECHO: &str = r#"{"turns":[]}"#;

async fn turn(core: &Core, id: SessionId, text: &str) {
    let mut events = core.subscribe();
    core.send_prompt(id, text).await.unwrap();
    states_until(&mut events, id, SessionState::Idle).await;
}

/// The ACP id of the session that was sent `text`.
fn acp_id_sent(fake: &FakeAgent, text: &str) -> String {
    fake.received("session/prompt")
        .into_iter()
        .find(|m| m["params"]["prompt"][0]["text"] == text)
        .expect("the prompt was sent")["params"]["sessionId"]
        .as_str()
        .unwrap()
        .to_owned()
}

/// The log's lines about one session, in order.
fn lines_of(lines: &[Value], acp_id: &str) -> Vec<Value> {
    lines
        .iter()
        .filter(|l| l["session"] == acp_id)
        .cloned()
        .collect()
}

/// Each line's kind, with its state for `opened`/`state` ones.
fn kinds(lines: &[Value]) -> Vec<String> {
    lines
        .iter()
        .map(|l| match l["state"].as_str() {
            Some(state) => format!("{} {state}", l["kind"].as_str().unwrap()),
            None => l["kind"].as_str().unwrap().to_owned(),
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_sessions_start_and_state_changes_are_recorded_across_restarts() {
    let repo = git_repo();
    let fake = FakeAgent::new(ECHO);
    let core = core_with_session_log(&fake);
    let root = core.open_workspace(repo.path()).await.unwrap().root;
    let first = core.new_session().await.unwrap();
    turn(&core, first, "one").await;
    // (Only a Tab that was sent a prompt is saved, and so restored.)
    let second = core.new_session().await.unwrap();
    turn(&core, second, "two").await;
    core.close_tab(first).await.unwrap();
    let (first_id, second_id) = (acp_id_sent(&fake, "one"), acp_id_sent(&fake, "two"));
    eventually("the first session's close to be recorded", || {
        kinds(&lines_of(&fake.session_log(), &first_id)).last() == Some(&"closed".to_owned())
    })
    .await;

    let log = fake.session_log();
    let first_lines = lines_of(&log, &first_id);
    assert_eq!(
        kinds(&first_lines),
        // (Closing the Tab stops its Agent, so it's Exited before it's closed.)
        [
            "opened idle",
            "state working",
            "state idle",
            "state exited",
            "closed"
        ]
    );
    assert_eq!(first_lines[0]["new"], true);
    assert_eq!(first_lines[0]["workspace"], root.display().to_string());
    assert_eq!(first_lines[0]["worktree"], root.display().to_string());
    let second_lines = lines_of(&log, &second_id);
    assert_eq!(
        kinds(&second_lines),
        ["opened idle", "state working", "state idle"]
    );
    assert_eq!(second_lines[0]["new"], true);
    for lines in [&first_lines, &second_lines] {
        let times: Vec<u64> = lines.iter().map(|l| l["at"].as_u64().unwrap()).collect();
        assert!(times.windows(2).all(|t| t[0] <= t[1]), "{times:?}");
    }
    drop(core);

    // The editor after a restart: the old lines stay, and the restored Tab is opened again.
    let before = fake.session_log();
    let core = core_with_session_log(&fake);
    core.open_workspace(repo.path()).await.unwrap();
    let after = fake.session_log();
    assert_eq!(after[..before.len()], before[..]);
    let restored = lines_of(&after[before.len()..], &second_id);
    assert_eq!(kinds(&restored), ["opened suspended"]);
    assert_eq!(restored[0]["new"], false);
    assert!(lines_of(&after[before.len()..], &first_id).is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn year_old_entries_are_dropped_at_startup() {
    let fake = FakeAgent::new(ECHO);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let line = |at: u64| format!(r#"{{"kind":"closed","at":{at},"session":"acp-1"}}"#);
    std::fs::write(
        fake.session_log_path(),
        format!("{}\n{}\n", line(1), line(now)),
    )
    .unwrap();
    let _core = core_with_session_log(&fake);
    let left = fake.session_log();
    assert_eq!(left.len(), 1, "{left:?}");
    assert_eq!(left[0]["at"], now);
}
