//! Review and pull request (ticket 42): everything the Worktree's branch would bring into its PR,
//! read from where it split from its Base to the files on disk (committed or not, untracked files
//! too), and the PR made from it: whatever isn't committed yet is committed, the branch is pushed,
//! and the GitHub CLI opens the PR with the user as its assignee.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::git_status::ChangeKind;

/// What "Review" shows: every file the PR would change, and what the PR step starts from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Review {
    /// None when HEAD is detached (there's no branch to make a PR from).
    pub branch: Option<String>,
    /// The Worktree's Base, and where the branch split from it (each file's diff runs from there).
    pub base: String,
    pub split: String,
    pub files: Vec<ReviewFile>,
    /// The subjects of the branch's commits since the split, oldest first.
    pub commits: Vec<String>,
    /// How many files have changes that aren't committed yet (committed on "Create PR").
    pub uncommitted: u32,
    /// A merge, rebase or the like is in progress: no PR until it's finished or aborted.
    pub operation_in_progress: bool,
    /// The remote the branch is pushed to, and its branches (without `<remote>/`), to target.
    pub remote: String,
    pub targets: Vec<String>,
    /// The branch the PR goes into unless another is chosen: the Base's, else the default one.
    pub target: String,
}

/// A file the PR would change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewFile {
    /// Relative to the Worktree, `/`-separated.
    pub path: String,
    pub change: ChangeKind,
    /// A rename's original path.
    pub renamed_from: Option<String>,
    /// It has changes that aren't committed yet (some of them, at least).
    pub uncommitted: bool,
}

/// "Create PR", as asked.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestRequest {
    /// The branch it goes into (without `<remote>/`).
    pub target: String,
    pub title: String,
    #[serde(default)]
    pub body: String,
    /// What to commit the uncommitted changes with, if there are any.
    #[serde(default)]
    pub commit_message: String,
    #[serde(default)]
    pub draft: bool,
    /// Go ahead while a session in the Worktree is Working.
    #[serde(default)]
    pub even_if_working: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PullRequestOutcome {
    /// Sessions in the Worktree are Working (they may still be changing files): ask first. Nothing
    /// was done.
    SessionsWorking { sessions: Vec<String> },
    /// The PR at `url` was opened; `committed` is the commit made of the uncommitted changes first.
    Created {
        url: String,
        committed: Option<String>,
    },
    /// The branch already had an open PR (at `url`): the push updated it.
    AlreadyOpen {
        url: String,
        committed: Option<String>,
    },
    /// The remote has commits the branch hasn't, so it turned the push down (anything committed
    /// first stays committed).
    PushRejected { committed: Option<String> },
}

/// `<remote>/<branch>` as the branch on the remote, if it's one of `remote`'s.
pub(crate) fn on_remote<'a>(name: &'a str, remote: &str) -> Option<&'a str> {
    name.strip_prefix(remote)?.strip_prefix('/')
}

/// How long `gh` may take to open a PR before it's stopped.
const GH_TIMEOUT: Duration = Duration::from_secs(120);

/// What to tell the user when `gh` couldn't run or isn't logged in.
const GH_HINT: &str =
    "Creating a PR takes the GitHub CLI: install `gh`, run `gh auth login` in a terminal once, then try again.";

/// Signs, in `gh`'s words, that it isn't logged in.
const GH_LOGIN_SIGNS: &[&str] = &["gh auth login", "authentication", "not logged", "401"];

/// What `gh pr create` did.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Opened {
    Created(String),
    AlreadyOpen(String),
}

/// Opens a PR from `branch` into `target` with `gh pr create` (in `worktree`, so for its repo),
/// assigned to whoever `gh` is logged in as. Never prompts.
pub(crate) async fn create(
    gh: &Path,
    worktree: &Path,
    branch: &str,
    target: &str,
    title: &str,
    body: &str,
    draft: bool,
) -> Result<Opened, String> {
    let mut cmd = crate::process::command(gh);
    for (key, value) in gh_env().await {
        cmd.env(key, value);
    }
    cmd.current_dir(worktree)
        .args(["pr", "create", "--head", branch, "--base", target])
        .args(["--title", title, "--body", body, "--assignee", "@me"])
        .args(draft.then_some("--draft"))
        .env("GH_PROMPT_DISABLED", "1")
        .env("GH_NO_UPDATE_NOTIFIER", "1")
        .env("NO_COLOR", "1")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    crate::process::ProcessTree::own_group(&mut cmd);
    let child = cmd.spawn().map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => GH_HINT.to_owned(),
        _ => format!("could not run gh: {e}"),
    })?;
    let _tree = crate::process::ProcessTree::attach(&child);
    let out = tokio::time::timeout(GH_TIMEOUT, child.wait_with_output())
        .await
        .map_err(|_| format!("gh took over {} s and was stopped.", GH_TIMEOUT.as_secs()))?
        .map_err(|e| format!("could not run gh: {e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    opened(out.status.success(), &stdout, &stderr)
}

/// Reads what `gh pr create` said: the new PR's URL (its last line), or the open one's when there
/// already is one.
fn opened(success: bool, stdout: &str, stderr: &str) -> Result<Opened, String> {
    let url = |text: &str| {
        text.split_whitespace()
            .rev()
            .find(|word| word.starts_with("https://") || word.starts_with("http://"))
            .map(str::to_owned)
    };
    if success {
        return url(stdout)
            .map(Opened::Created)
            .ok_or_else(|| format!("gh opened the PR but didn't say where: {}", stdout.trim()));
    }
    if stderr.contains("already exists") {
        if let Some(url) = url(stderr) {
            return Ok(Opened::AlreadyOpen(url));
        }
    }
    let message = match stderr.trim() {
        "" => stdout.trim(),
        stderr => stderr,
    };
    let lower = message.to_lowercase();
    Err(
        match GH_LOGIN_SIGNS.iter().any(|sign| lower.contains(sign)) {
            true => format!("{message}\n{GH_HINT}"),
            false => message.to_owned(),
        },
    )
}

/// The environment `gh` runs with: on Unix the login shell's (started from the desktop, Orchard's
/// own `PATH` may not have `gh`, nor a `GH_TOKEN` the profile sets).
async fn gh_env() -> Vec<(OsString, OsString)> {
    #[cfg(unix)]
    return crate::setup::shell_env().await.to_vec();
    #[cfg(not(unix))]
    return vec![];
}

/// The GitHub CLI to run: the configured one, else `gh` (found on `PATH`).
pub(crate) fn gh_program(configured: Option<&PathBuf>) -> PathBuf {
    configured.cloned().unwrap_or_else(|| PathBuf::from("gh"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_new_prs_url_is_its_last_line() {
        let out =
            "Creating pull request for agent/x into main in o/r\n\nhttps://github.com/o/r/pull/7\n";
        assert_eq!(
            opened(true, out, ""),
            Ok(Opened::Created("https://github.com/o/r/pull/7".into()))
        );
    }

    #[test]
    fn an_open_pr_for_the_branch_is_found() {
        let err = "a pull request for branch \"agent/x\" into branch \"main\" already exists:\nhttps://github.com/o/r/pull/3\n";
        assert_eq!(
            opened(false, "", err),
            Ok(Opened::AlreadyOpen("https://github.com/o/r/pull/3".into()))
        );
    }

    #[test]
    fn a_missing_login_says_how_to_log_in() {
        let err = "To get started with GitHub CLI, please run:  gh auth login";
        assert!(opened(false, "", err).unwrap_err().contains(GH_HINT));
    }

    #[test]
    fn a_remote_branch_loses_its_remote() {
        assert_eq!(on_remote("origin/main", "origin"), Some("main"));
        assert_eq!(on_remote("origin/feat/x", "origin"), Some("feat/x"));
        assert_eq!(on_remote("upstream/main", "origin"), None);
        assert_eq!(on_remote("originals/main", "origin"), None);
    }
}
