//! The composer's `/` menu: the slash commands (and skills) the Agent offers, per session.

mod support;

use editor_core::SlashCommand;
use support::*;

#[tokio::test(flavor = "multi_thread")]
async fn a_sessions_slash_commands_come_from_the_agent_and_are_announced() {
    let repo = git_repo();
    let fake = FakeAgent::new(
        r#"{"turns":[],"commands":[
            {"name":"review","description":"Review a pull request","input":{"hint":"[pr number]"}},
            {"name":"init","description":"Write a CLAUDE.md","input":null},
            {"description":"no name: skipped"}
        ]}"#,
    );
    let core = core_with(&fake);
    core.open_workspace(repo.path()).await.unwrap();
    let id = core.new_session().await.unwrap();

    let expected = vec![
        SlashCommand {
            name: "review".into(),
            description: "Review a pull request".into(),
            hint: Some("[pr number]".into()),
        },
        SlashCommand {
            name: "init".into(),
            description: "Write a CLAUDE.md".into(),
            hint: None,
        },
    ];
    // (The list can come before the session is registered: it's kept for it then.)
    eventually("the slash commands", || {
        core.available_commands(id).unwrap() == expected
    })
    .await;
}
