//! Ticket 29: the Recent Workspaces the Workspace picker lists.

mod support;

use editor_core::{Core, SessionId, SessionState};
use support::*;

const ECHO: &str = r#"{"turns":[]}"#;

async fn turn(core: &Core, id: SessionId, text: &str) {
    let mut events = core.subscribe();
    core.send_prompt(id, text).await.unwrap();
    states_until(&mut events, id, SessionState::Idle).await;
}

/// Opens `repo` in a fresh editor (a restart), so it's saved as the most recently opened.
async fn open_in_new_editor(fake: &FakeAgent, repo: &std::path::Path) -> Core {
    // (Opening times are in milliseconds; keep two opens apart.)
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let core = core_with_state(fake);
    core.open_workspace(repo).await.unwrap();
    core
}

#[tokio::test(flavor = "multi_thread")]
async fn opened_workspaces_are_listed_newest_first_with_their_tab_counts() {
    let fake = FakeAgent::new(ECHO);
    let (first, second) = (git_repo(), git_repo());
    let core = open_in_new_editor(&fake, first.path()).await;
    let id = core.new_session().await.unwrap();
    turn(&core, id, "one").await;
    drop(core);
    let core = open_in_new_editor(&fake, second.path()).await;

    let recent = core.recent_workspaces("");
    let roots: Vec<_> = recent.iter().map(|w| w.root.clone()).collect();
    assert_eq!(roots.len(), 2);
    assert!(roots[0].ends_with(second.path().file_name().unwrap()));
    assert!(roots[1].ends_with(first.path().file_name().unwrap()));
    assert_eq!(
        recent[0].sessions, 0,
        "a Workspace with no Tabs is listed too"
    );
    assert_eq!(recent[1].sessions, 1);
    assert!(recent.iter().all(|w| w.exists && w.opened.is_some()));
    assert_eq!(
        recent[1].name,
        first.path().file_name().unwrap().to_string_lossy()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_filter_matches_paths_and_says_which_characters() {
    let fake = FakeAgent::new(ECHO);
    let (first, second) = (git_repo(), git_repo());
    drop(open_in_new_editor(&fake, first.path()).await);
    let core = open_in_new_editor(&fake, second.path()).await;
    let name = first
        .path()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();

    let found = core.recent_workspaces(&name);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].name, name);
    assert_eq!(found[0].indices.len(), name.len());
    assert!(core.recent_workspaces("zzzzzzzzzzzz").is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_removed_workspace_keeps_its_tabs_and_comes_back_when_opened() {
    let fake = FakeAgent::new(ECHO);
    let repo = git_repo();
    let core = open_in_new_editor(&fake, repo.path()).await;
    let root = core.recent_workspaces("")[0].root.clone();
    let id = core.new_session().await.unwrap();
    turn(&core, id, "one").await;
    drop(core);

    let core = core_with_state(&fake);
    core.remove_recent_workspace(&root).unwrap();
    assert!(core.recent_workspaces("").is_empty());

    core.open_workspace(repo.path()).await.unwrap();
    assert_eq!(core.sessions().len(), 1, "its Tab is restored");
    let recent = core.recent_workspaces("");
    assert_eq!(recent.len(), 1);
    assert_eq!(recent[0].sessions, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_workspace_whose_folder_is_gone_stays_listed() {
    let fake = FakeAgent::new(ECHO);
    let repo = git_repo();
    drop(open_in_new_editor(&fake, repo.path()).await);
    drop(repo);

    let core = core_with_state(&fake);
    let recent = core.recent_workspaces("");
    assert_eq!(recent.len(), 1);
    assert!(!recent[0].exists);
}
