//! Ticket 08: the hand-edited TOML settings file, loaded at startup and reloaded live on save.

mod support;

use editor_core::CoreEvent;
use support::*;

#[tokio::test(flavor = "multi_thread")]
async fn settings_load_at_startup() {
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with_settings(&fake, "[notifications]\nturn_finished = true\n");

    let loaded = core.settings();
    assert!(loaded.settings.notifications.turn_finished);
    assert_eq!(loaded.error, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn without_a_settings_file_the_defaults_apply() {
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);

    let loaded = core.settings();
    assert!(
        !loaded.settings.notifications.turn_finished,
        "off by default"
    );
    assert_eq!(loaded.error, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn saved_changes_apply_live_and_invalid_toml_keeps_the_last_good_settings() {
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with_settings(&fake, "[notifications]\nturn_finished = false\n");
    let mut events = core.subscribe();

    std::fs::write(
        fake.settings_path(),
        "[notifications]\nturn_finished = true\n",
    )
    .unwrap();
    let changed = next_event(&mut events, "the saved settings", |e| match e {
        CoreEvent::SettingsChanged { settings } => Some(settings),
        _ => None,
    })
    .await;
    assert!(changed.settings.notifications.turn_finished);
    assert_eq!(changed.error, None);

    std::fs::write(fake.settings_path(), "[notifications\nturn_finished = ").unwrap();
    let broken = next_event(&mut events, "the settings error", |e| match e {
        CoreEvent::SettingsChanged { settings } if settings.error.is_some() => Some(settings),
        _ => None,
    })
    .await;
    assert!(
        broken.settings.notifications.turn_finished,
        "the last good settings stay in force"
    );
    assert!(core.settings().settings.notifications.turn_finished);

    std::fs::write(
        fake.settings_path(),
        "[notifications]\nturn_finished = false\n",
    )
    .unwrap();
    let fixed = next_event(&mut events, "the fixed settings", |e| match e {
        CoreEvent::SettingsChanged { settings } if settings.error.is_none() => Some(settings),
        _ => None,
    })
    .await;
    assert!(!fixed.settings.notifications.turn_finished);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_invalid_file_at_startup_reports_the_error_and_uses_the_defaults() {
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with_settings(&fake, "notifications = [");

    let loaded = core.settings();
    assert!(loaded.error.is_some());
    assert!(!loaded.settings.notifications.turn_finished);
}

#[tokio::test(flavor = "multi_thread")]
async fn open_repo_settings_adds_a_section_for_this_repo_once() {
    let setup = RepoWithOrigin::new();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with_settings(&fake, "[notifications]\nturn_finished = true\n");
    core.open_workspace(&setup.repo()).await.unwrap();

    let path = core.open_repo_settings().await.unwrap();
    assert_eq!(path, fake.settings_path());
    let origin = origin_url(&setup.repo());
    assert!(
        core.settings().settings.repos.contains_key(&origin),
        "keyed by origin URL: {:?}",
        core.settings().settings.repos.keys().collect::<Vec<_>>()
    );
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(
        written.starts_with("[notifications]\nturn_finished = true\n"),
        "what was there is kept: {written}"
    );

    core.open_repo_settings().await.unwrap();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        written,
        "not added twice"
    );
}
