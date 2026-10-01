//! Startup checks for the tools the editor shells out to: git ≥ 2.55 (ADR 0004) and Node ≥ 22.12 for
//! the ACP adapter (ADR 0003).

use std::path::PathBuf;

use serde::Serialize;

#[derive(Debug, Clone)]
pub struct ToolCommand {
    pub program: PathBuf,
    pub args: Vec<String>,
}

impl ToolCommand {
    fn system(program: &str) -> Self {
        Self {
            program: program.into(),
            args: vec!["--version".into()],
        }
    }
}

/// How to ask each tool for its version.
#[derive(Debug, Clone)]
pub struct Tools {
    pub git: ToolCommand,
    pub node: ToolCommand,
}

impl Default for Tools {
    fn default() -> Self {
        Self {
            git: ToolCommand::system("git"),
            node: ToolCommand::system("node"),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MissingPrerequisite {
    pub tool: String,
    /// The installed version, if the tool ran at all.
    pub found: Option<String>,
    /// What to tell the user, naming what to install.
    pub message: String,
}

struct Requirement {
    tool: &'static str,
    display: &'static str,
    minimum: (u64, u64),
    install: &'static str,
}

const GIT: Requirement = Requirement {
    tool: "git",
    display: "Git",
    minimum: (2, 55),
    install: "Install a current Git from https://git-scm.com/downloads (on Windows: `winget upgrade Git.Git`).",
};
/// 22.12: the pinned `claude-agent-acp` needs Node ≥ 22, and the Vite 8 dev toolchain ≥ 22.12.
const NODE: Requirement = Requirement {
    tool: "node",
    display: "Node.js",
    minimum: (22, 12),
    install: "Install a current Node.js LTS from https://nodejs.org.",
};

/// Every prerequisite that's missing or too old; empty when the editor can start.
pub async fn check_prerequisites(tools: &Tools) -> Vec<MissingPrerequisite> {
    let (git, node) = tokio::join!(check(&tools.git, &GIT), check(&tools.node, &NODE));
    git.into_iter().chain(node).collect()
}

async fn check(command: &ToolCommand, requirement: &Requirement) -> Option<MissingPrerequisite> {
    let (major, minor) = requirement.minimum;
    let needed = format!(
        "{} {major}.{minor} or newer is required",
        requirement.display
    );
    let output = crate::process::command(&command.program)
        .args(&command.args)
        .output()
        .await;
    let found = output
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| parse_version(&String::from_utf8_lossy(&o.stdout)));
    let Some((version, parsed)) = found else {
        return Some(MissingPrerequisite {
            tool: requirement.tool.into(),
            found: None,
            message: format!(
                "{needed}, but `{}` was not found. {}",
                requirement.tool, requirement.install
            ),
        });
    };
    if parsed >= requirement.minimum {
        return None;
    }
    Some(MissingPrerequisite {
        tool: requirement.tool.into(),
        message: format!("{needed} (found {version}). {}", requirement.install),
        found: Some(version),
    })
}

/// Finds the first `major.minor[.patch]` in output like `git version 2.45.2.windows.1` or `v22.4.0`.
fn parse_version(output: &str) -> Option<(String, (u64, u64))> {
    output.split_whitespace().find_map(|word| {
        let word = word.trim_start_matches('v');
        let parts: Vec<&str> = word
            .split('.')
            .take_while(|p| p.parse::<u64>().is_ok())
            .take(3)
            .collect();
        if parts.len() < 2 {
            return None;
        }
        Some((
            parts.join("."),
            (parts[0].parse().ok()?, parts[1].parse().ok()?),
        ))
    })
}
