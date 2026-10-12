//! The composer's context meter: how much of its context window a session fills, and the tokens it
//! has spent.

mod support;

use editor_core::{CoreEvent, SessionUsage};
use support::*;

#[tokio::test(flavor = "multi_thread")]
async fn a_sessions_usage_comes_from_the_agent_and_its_turns_add_up() {
    let repo = git_repo();
    let fake = FakeAgent::new(
        r#"{"turns":[
            {"chunks":["one"],
             "usage":{"used":12000,"size":200000,"cost":{"amount":0.25,"currency":"USD"}},
             "promptUsage":{"inputTokens":10,"outputTokens":200,"cachedReadTokens":3000,"cachedWriteTokens":400,"totalTokens":3610}},
            {"chunks":["two"],
             "usage":{"used":15000,"size":200000},
             "promptUsage":{"inputTokens":5,"outputTokens":100,"cachedReadTokens":12000,"cachedWriteTokens":0,"totalTokens":12105}}
        ]}"#,
    );
    let core = core_with(&fake);
    let mut events = core.subscribe();
    core.open_workspace(repo.path()).await.unwrap();
    let id = core.new_session().await.unwrap();
    assert_eq!(core.session_usage(id).unwrap(), SessionUsage::default());

    core.send_prompt(id, "first").await.unwrap();
    let after_first = SessionUsage {
        context_used: Some(12000),
        context_size: Some(200000),
        input_tokens: 10,
        output_tokens: 200,
        cache_read_tokens: 3000,
        cache_write_tokens: 400,
        cost_usd: Some(0.25),
    };
    eventually("the first turn's usage", || {
        core.session_usage(id).unwrap() == after_first
    })
    .await;

    // (An update without a cost keeps the last one.)
    core.send_prompt(id, "second").await.unwrap();
    let after_second = SessionUsage {
        context_used: Some(15000),
        input_tokens: 15,
        output_tokens: 300,
        cache_read_tokens: 15000,
        ..after_first
    };
    eventually("both turns' usage", || {
        core.session_usage(id).unwrap() == after_second
    })
    .await;

    // Each change was announced, the last with the totals.
    let mut last = None;
    while let Ok(event) = events.try_recv() {
        if let CoreEvent::UsageChanged { session_id, usage } = event {
            assert_eq!(session_id, id);
            last = Some(usage);
        }
    }
    assert_eq!(last, Some(after_second));
}
