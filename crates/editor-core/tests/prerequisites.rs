//! Ticket 01: startup names exactly what to install when git ≥ 2.55 or Node ≥ 22.12 is missing.
//! (Node 22.12: the pinned `claude-agent-acp` needs Node ≥ 22, and the Vite 8 toolchain ≥ 22.12.)

mod support;

use editor_core::{check_prerequisites, ToolCommand, Tools};
use support::fake_agent_path;

/// A stand-in tool that prints `output` for its version.
fn prints(output: &str) -> ToolCommand {
    ToolCommand { program: fake_agent_path(), args: vec!["--print".into(), output.into()] }
}

fn missing() -> ToolCommand {
    ToolCommand { program: "definitely-not-an-installed-tool-8f3a".into(), args: vec![] }
}

#[tokio::test]
async fn current_git_and_node_pass() {
    let tools = Tools { git: prints("git version 2.56.0.windows.1"), node: prints("v22.12.0") };

    assert!(check_prerequisites(&tools).await.is_empty());
}

#[tokio::test]
async fn an_old_git_is_reported_with_the_version_found_and_the_version_needed() {
    let tools = Tools { git: prints("git version 2.45.2.windows.1"), node: prints("v24.1.0") };

    let problems = check_prerequisites(&tools).await;

    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0].tool, "git");
    assert_eq!(problems[0].found.as_deref(), Some("2.45.2"));
    assert!(problems[0].message.contains("2.55"), "{}", problems[0].message);
    assert!(problems[0].message.contains("2.45.2"), "{}", problems[0].message);
}

#[tokio::test]
async fn an_old_node_is_reported() {
    let tools = Tools { git: prints("git version 2.55.0"), node: prints("v18.19.1") };

    let problems = check_prerequisites(&tools).await;

    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0].tool, "node");
    assert_eq!(problems[0].found.as_deref(), Some("18.19.1"));
    assert!(problems[0].message.contains("22.12"), "{}", problems[0].message);
}

#[tokio::test]
async fn node_22_below_22_12_is_too_old() {
    let tools = Tools { git: prints("git version 2.55.0"), node: prints("v22.4.0") };

    let problems = check_prerequisites(&tools).await;

    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0].found.as_deref(), Some("22.4.0"));
}

#[tokio::test]
async fn missing_tools_are_reported_as_not_found() {
    let tools = Tools { git: missing(), node: missing() };

    let problems = check_prerequisites(&tools).await;

    assert_eq!(problems.iter().map(|p| p.tool.as_str()).collect::<Vec<_>>(), ["git", "node"]);
    assert!(problems.iter().all(|p| p.found.is_none()));
    assert!(problems.iter().all(|p| p.message.contains("not found")), "{problems:?}");
}
