//! The one place the core runs `git` (ADR 0004). New git operations belong here, so a library can
//! later replace a hot path without touching callers.

use std::path::{Path, PathBuf};

use tokio::process::Command;

fn git(cwd: &Path) -> Command {
    let mut cmd = crate::process::command("git");
    cmd.current_dir(cwd).env("GIT_TERMINAL_PROMPT", "0");
    cmd
}

/// The root of the checkout containing `path`, or `None` if it isn't inside a git repository.
pub(crate) async fn toplevel(path: &Path) -> Option<PathBuf> {
    let out = git(path)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(PathBuf::from(String::from_utf8_lossy(&out.stdout).trim()))
}
