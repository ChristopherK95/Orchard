//! Ticket 12: Idle sessions are Suspended when the Agents use too much memory, or when they've been
//! Idle too long; Working and Needs you sessions never are. The memory reading and clock are fakes.

mod support;

use std::sync::Arc;
use std::time::Duration;

use editor_core::{AutoSuspendReason, Core, CoreEvent, MemorySample, SessionId, SessionState};
use support::*;

const GB: u64 = 1024 * 1024 * 1024;
const ECHO: &str = r#"{"turns":[]}"#;

fn using(agents_gb: u64) -> MemorySample {
    MemorySample {
        agents_bytes: agents_gb * GB,
        available_bytes: 16 * GB,
        total_bytes: 32 * GB,
    }
}

fn state(core: &Core, id: SessionId) -> SessionState {
    core.session_info(id).unwrap().state
}

/// Runs a check and returns what it announced (in one event), grouped by reason.
async fn check(core: &Core) -> Vec<(Vec<SessionId>, AutoSuspendReason)> {
    let mut events = core.subscribe();
    core.check_auto_suspend().await;
    let mut announcements = 0;
    let mut announced: Vec<(Vec<SessionId>, AutoSuspendReason)> = vec![];
    while let Ok(event) = events.try_recv() {
        if let CoreEvent::AutoSuspended { suspended } = event {
            announcements += 1;
            for s in suspended {
                match announced.iter_mut().find(|(_, reason)| *reason == s.reason) {
                    Some((ids, _)) => ids.push(s.session.id),
                    None => announced.push((vec![s.session.id], s.reason)),
                }
            }
        }
    }
    assert!(announcements <= 1, "one announcement per check");
    announced
}

/// Past auto-suspend's cooldown, so memory counts again.
fn after_cooldown(clock: &FakeClock) {
    clock.advance(Duration::from_secs(30));
}

struct Setup {
    _repo: tempfile::TempDir,
    fake: FakeAgent,
    core: Core,
    memory: Arc<FakeMemory>,
    clock: Arc<FakeClock>,
}

async fn setup(script: &str, settings: &str) -> Setup {
    let repo = git_repo();
    let fake = FakeAgent::new(script);
    let memory = Arc::new(FakeMemory::default());
    let clock = Arc::new(FakeClock::default());
    let core = core_with_memory(&fake, settings, &memory, &clock);
    core.open_workspace(repo.path()).await.unwrap();
    Setup {
        _repo: repo,
        fake,
        core,
        memory,
        clock,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn over_the_memory_limit_the_longest_idle_sessions_go_first_until_under_it() {
    let s = setup(ECHO, "[agents]\nmemory_limit_mb = 4096\n").await;
    let oldest = s.core.new_session().await.unwrap();
    s.clock.advance(Duration::from_secs(60));
    let middle = s.core.new_session().await.unwrap();
    s.clock.advance(Duration::from_secs(60));
    let newest = s.core.new_session().await.unwrap();

    s.memory.set(using(3));
    assert!(check(&s.core).await.is_empty(), "under the limit");

    // 5 GB across 3 sessions: about 1.7 GB each, 1 GB over: one of them.
    s.memory.set(using(5));
    let announced = check(&s.core).await;
    assert_eq!(announced.len(), 1);
    assert_eq!(announced[0].0, [oldest]);
    assert!(matches!(
        announced[0].1,
        AutoSuspendReason::MemoryLimit { used_bytes, limit_bytes }
            if used_bytes == 5 * GB && limit_bytes == 4 * GB
    ));
    assert_eq!(state(&s.core, oldest), SessionState::Suspended);
    assert_eq!(state(&s.core, middle), SessionState::Idle);
    assert_eq!(s.fake.received("session/close").len(), 1);

    // Still over on the next reading: right after a suspend nothing more (the closed process may
    // not have left the reading yet), then the next-longest Idle ones.
    s.memory.set(using(9));
    assert!(check(&s.core).await.is_empty(), "cooling down");
    after_cooldown(&s.clock);
    let announced = check(&s.core).await;
    assert_eq!(announced[0].0, [middle, newest]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_limit_of_zero_means_no_limit() {
    let s = setup(
        ECHO,
        "[agents]\nmemory_limit_mb = 0\nidle_suspend = true\nidle_suspend_minutes = 0\n",
    )
    .await;
    let id = s.core.new_session().await.unwrap();
    s.memory.set(using(50));
    s.clock.advance(Duration::from_secs(3600));
    assert!(check(&s.core).await.is_empty());
    assert_eq!(state(&s.core, id), SessionState::Idle);
}

#[tokio::test(flavor = "multi_thread")]
async fn low_os_memory_suspends_the_longest_idle_session() {
    let s = setup(ECHO, "[agents]\nmemory_limit_mb = 16384\n").await;
    let oldest = s.core.new_session().await.unwrap();
    s.clock.advance(Duration::from_secs(60));
    let newer = s.core.new_session().await.unwrap();
    s.memory.set(MemorySample {
        agents_bytes: 6 * GB,
        available_bytes: GB / 4,
        total_bytes: 16 * GB,
    });
    let announced = check(&s.core).await;
    assert_eq!(announced.len(), 1);
    assert_eq!(announced[0].0, [oldest]);
    assert!(matches!(
        announced[0].1,
        AutoSuspendReason::LowMemory { .. }
    ));
    assert_eq!(state(&s.core, newer), SessionState::Idle);
}

#[tokio::test(flavor = "multi_thread")]
async fn suspended_sessions_are_neither_chosen_nor_counted() {
    let s = setup(ECHO, "[agents]\nmemory_limit_mb = 4096\n").await;
    let already = s.core.new_session().await.unwrap();
    s.clock.advance(Duration::from_secs(60));
    let a = s.core.new_session().await.unwrap();
    let b = s.core.new_session().await.unwrap();
    s.core.suspend_session(already).await.unwrap();
    // 5 GB over the 2 running sessions (not 3): 2.5 GB each, 1 GB over: one.
    s.memory.set(using(5));
    let announced = check(&s.core).await;
    assert_eq!(announced[0].0.len(), 1);
    assert!(announced[0].0[0] == a || announced[0].0[0] == b);
}

#[tokio::test(flavor = "multi_thread")]
async fn both_reasons_in_one_check_are_one_announcement() {
    let s = setup(
        ECHO,
        "[agents]\nmemory_limit_mb = 4096\nidle_suspend = true\nidle_suspend_minutes = 30\n",
    )
    .await;
    let oldest = s.core.new_session().await.unwrap();
    s.clock.advance(Duration::from_secs(10 * 60));
    let older = s.core.new_session().await.unwrap();
    s.clock.advance(Duration::from_secs(40 * 60));
    let fresh = s.core.new_session().await.unwrap();
    // 5 GB over 3: one goes for memory (the longest Idle); the next is past 30 minutes Idle.
    s.memory.set(using(5));
    let announced = check(&s.core).await; // (asserts it's a single event)
    assert_eq!(announced.len(), 2);
    assert_eq!(announced[0].0, [oldest]);
    assert!(matches!(
        announced[0].1,
        AutoSuspendReason::MemoryLimit { .. }
    ));
    assert_eq!(
        announced[1],
        (vec![older], AutoSuspendReason::Idle { minutes: 30 })
    );
    assert_eq!(state(&s.core, fresh), SessionState::Idle);
}
#[tokio::test(flavor = "multi_thread")]
async fn a_session_that_was_just_used_counts_as_idle_from_then() {
    let s = setup(ECHO, "[agents]\nmemory_limit_mb = 4096\n").await;
    let first = s.core.new_session().await.unwrap();
    s.clock.advance(Duration::from_secs(60));
    let second = s.core.new_session().await.unwrap();
    s.clock.advance(Duration::from_secs(60));
    let mut events = s.core.subscribe();
    s.core.send_prompt(first, "still here").await.unwrap();
    states_until(&mut events, first, SessionState::Idle).await;

    s.memory.set(using(5));
    assert_eq!(
        check(&s.core).await[0].0,
        [second],
        "first is the fresher one now"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn needs_you_and_working_sessions_are_never_auto_suspended() {
    let asks = r#"{"turns":[{"permission":{"title":"Edit a.rs","kind":"edit"}}]}"#;
    let s = setup(
        asks,
        "[agents]\nmemory_limit_mb = 1024\nidle_suspend = true\nidle_suspend_minutes = 1\n",
    )
    .await;
    let waiting = s.core.new_session().await.unwrap();
    let idle = s.core.new_session().await.unwrap();
    let mut events = s.core.subscribe();
    s.core.send_prompt(waiting, "edit").await.unwrap();
    states_until(&mut events, waiting, SessionState::NeedsYou).await;

    s.clock.advance(Duration::from_secs(3600));
    s.memory.set(using(50));
    let announced = check(&s.core).await;
    let suspended: Vec<SessionId> = announced.into_iter().flat_map(|(ids, _)| ids).collect();
    assert_eq!(suspended, [idle]);
    assert_eq!(state(&s.core, waiting), SessionState::NeedsYou);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_working_session_is_never_auto_suspended() {
    // A turn that runs until it's cancelled, so it's Working for as long as the test needs.
    let busy = r#"{"turns":[{"untilCancelled":true}]}"#;
    let s = setup(
        busy,
        "[agents]\nmemory_limit_mb = 1024\nidle_suspend = true\nidle_suspend_minutes = 1\n",
    )
    .await;
    let working = s.core.new_session().await.unwrap();
    let idle = s.core.new_session().await.unwrap();
    let mut events = s.core.subscribe();
    s.core.send_prompt(working, "go").await.unwrap();
    states_until(&mut events, working, SessionState::Working).await;

    s.clock.advance(Duration::from_secs(3600));
    s.memory.set(using(50));
    let suspended: Vec<SessionId> = check(&s.core)
        .await
        .into_iter()
        .flat_map(|(ids, _)| ids)
        .collect();
    assert_eq!(suspended, [idle], "only the Idle one");
    assert_eq!(state(&s.core, working), SessionState::Working);
}

#[tokio::test(flavor = "multi_thread")]
async fn idle_auto_suspend_takes_sessions_idle_longer_than_the_setting() {
    let s = setup(
        ECHO,
        "[agents]\nidle_suspend = true\nidle_suspend_minutes = 30\n",
    )
    .await;
    s.memory.set(using(1));
    let long_idle = s.core.new_session().await.unwrap();
    s.clock.advance(Duration::from_secs(20 * 60));
    let short_idle = s.core.new_session().await.unwrap();
    s.clock.advance(Duration::from_secs(11 * 60));

    let announced = check(&s.core).await;
    assert_eq!(announced.len(), 1);
    assert_eq!(announced[0].0, [long_idle]);
    assert_eq!(announced[0].1, AutoSuspendReason::Idle { minutes: 30 });
    assert_eq!(state(&s.core, short_idle), SessionState::Idle);
}

#[tokio::test(flavor = "multi_thread")]
async fn idle_auto_suspend_is_off_by_default() {
    let s = setup(ECHO, "").await;
    s.memory.set(using(1));
    let id = s.core.new_session().await.unwrap();
    s.clock.advance(Duration::from_secs(24 * 3600));
    assert!(check(&s.core).await.is_empty());
    assert_eq!(state(&s.core, id), SessionState::Idle);
}

#[tokio::test(flavor = "multi_thread")]
async fn both_settings_take_effect_live() {
    let s = setup(ECHO, "[agents]\nmemory_limit_mb = 16384\n").await;
    let id = s.core.new_session().await.unwrap();
    s.memory.set(using(5));
    s.clock.advance(Duration::from_secs(40 * 60));
    assert!(
        check(&s.core).await.is_empty(),
        "under 16 GB, idle suspend off"
    );

    std::fs::write(
        s.fake.settings_path(),
        "[agents]\nmemory_limit_mb = 16384\nidle_suspend = true\nidle_suspend_minutes = 30\n",
    )
    .unwrap();
    eventually("the new settings", || {
        s.core.settings().settings.agents.idle_suspend
    })
    .await;
    assert_eq!(
        check(&s.core).await[0].1,
        AutoSuspendReason::Idle { minutes: 30 }
    );

    // And the memory limit: a fresh session, under 16 GB but over a new 4 GB.
    let fresh = s.core.new_session().await.unwrap();
    after_cooldown(&s.clock);
    std::fs::write(s.fake.settings_path(), "[agents]\nmemory_limit_mb = 4096\n").unwrap();
    eventually("the new limit", || {
        s.core.settings().settings.agents.memory_limit_mb == 4096
    })
    .await;
    let announced = check(&s.core).await;
    assert_eq!(announced[0].0, [fresh]);
    assert!(matches!(
        announced[0].1,
        AutoSuspendReason::MemoryLimit { .. }
    ));
    assert_eq!(state(&s.core, id), SessionState::Suspended);
}
