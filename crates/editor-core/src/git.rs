//! The one place the core runs `git` (ADR 0004). New git operations belong here, so a library can
//! later replace a hot path without touching callers. Output is always machine-readable
//! (`--porcelain` / `-z`), prompts are off (`GIT_TERMINAL_PROMPT=0`), and nothing here takes
//! git's optional locks (`GIT_OPTIONAL_LOCKS=0`), so background refreshes never block an Agent.
//! Optional locks only cover git's opportunistic index refresh, so turning them off is safe for the
//! editor's own commands too.

use std::path::{Path, PathBuf};

use tokio::process::Command;

fn git(cwd: &Path) -> Command {
    let mut cmd = crate::process::command("git");
    cmd.current_dir(cwd)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0");
    cmd
}

/// Runs git and returns stdout, or `None` if it failed.
async fn output(cwd: &Path, args: &[&str]) -> Option<String> {
    run(cwd, args).await.ok()
}

/// Runs git and returns stdout, or git's own error message (stderr) if it failed.
async fn run(cwd: &Path, args: &[&str]) -> Result<String, String> {
    let out = git(cwd)
        .args(args)
        .output()
        .await
        .map_err(|e| format!("could not run git: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_owned())
    }
}

/// Fetches `origin` (if the repository has one) so new Worktrees start from what's upstream now.
pub(crate) async fn fetch_origin(repo: &Path) -> Result<(), String> {
    let remotes = run(repo, &["remote"]).await?;
    if !remotes.lines().any(|r| r.trim() == "origin") {
        return Ok(());
    }
    run(repo, &["fetch", "--quiet", "--prune", "origin"])
        .await
        .map(|_| ())
}

/// `origin/<default branch>` if origin has one, else the main checkout's branch (or HEAD).
pub(crate) async fn default_base(repo: &Path) -> String {
    if let Some(origin_head) = output(
        repo,
        &[
            "symbolic-ref",
            "--quiet",
            "--short",
            "refs/remotes/origin/HEAD",
        ],
    )
    .await
    {
        return origin_head.trim().to_owned();
    }
    output(repo, &["symbolic-ref", "--quiet", "--short", "HEAD"])
        .await
        .map(|b| b.trim().to_owned())
        .unwrap_or_else(|| "HEAD".into())
}

/// A local or remote-tracking branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ListedBranch {
    /// `feat/x` for a local branch, `origin/feat/x` for a remote one.
    pub(crate) name: String,
    pub(crate) remote: bool,
    /// The Worktree a local branch is checked out in, if any.
    pub(crate) checked_out_in: Option<PathBuf>,
}

pub(crate) async fn branches(repo: &Path) -> Result<Vec<ListedBranch>, String> {
    let out = run(
        repo,
        &[
            "for-each-ref",
            "--format=%(refname)%00%(worktreepath)",
            "refs/heads",
            "refs/remotes",
        ],
    )
    .await?;
    Ok(out
        .lines()
        .filter_map(|line| {
            let (refname, worktree) = line.split_once('\0').unwrap_or((line, ""));
            let checked_out_in = (!worktree.is_empty()).then(|| PathBuf::from(worktree));
            if let Some(name) = refname.strip_prefix("refs/heads/") {
                return Some(ListedBranch {
                    name: name.to_owned(),
                    remote: false,
                    checked_out_in,
                });
            }
            let name = refname.strip_prefix("refs/remotes/")?;
            (!name.ends_with("/HEAD")).then(|| ListedBranch {
                name: name.to_owned(),
                remote: true,
                checked_out_in: None,
            })
        })
        .collect())
}

/// Adds a Worktree at `path` on a new branch `name` starting at `base`.
pub(crate) async fn worktree_add_new_branch(
    repo: &Path,
    path: &Path,
    name: &str,
    base: &str,
) -> Result<(), String> {
    let path = path.to_string_lossy();
    run(
        repo,
        &["worktree", "add", "--quiet", "-b", name, &path, base],
    )
    .await
    .map(|_| ())
}

/// Adds a Worktree at `path` on the existing local branch `name`.
pub(crate) async fn worktree_add_existing(
    repo: &Path,
    path: &Path,
    name: &str,
) -> Result<(), String> {
    let path = path.to_string_lossy();
    run(repo, &["worktree", "add", "--quiet", &path, name])
        .await
        .map(|_| ())
}

/// Adds a Worktree at `path` on a new local branch `name` tracking the remote branch `remote`.
pub(crate) async fn worktree_add_tracking(
    repo: &Path,
    path: &Path,
    name: &str,
    remote: &str,
) -> Result<(), String> {
    let path = path.to_string_lossy();
    run(
        repo,
        &[
            "worktree", "add", "--quiet", "--track", "-b", name, &path, remote,
        ],
    )
    .await
    .map(|_| ())
}

/// The root of the checkout containing `path`, or `None` if it isn't inside a git repository.
pub(crate) async fn toplevel(path: &Path) -> Option<PathBuf> {
    let out = output(path, &["rev-parse", "--show-toplevel"]).await?;
    Some(PathBuf::from(out.trim()))
}

/// The repository's shared git directory (where `worktrees/` lives).
pub(crate) async fn common_dir(path: &Path) -> Option<PathBuf> {
    let out = output(
        path,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )
    .await?;
    Some(PathBuf::from(out.trim()))
}

/// One entry of `git worktree list`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ListedWorktree {
    pub(crate) path: PathBuf,
    pub(crate) head: String,
    /// The checked-out branch (`main`, `feat/x`), or `None` when detached.
    pub(crate) branch: Option<String>,
    /// git lists the main checkout first; this is false for the linked Worktrees after it, and for
    /// all of them in a repository whose main entry is bare (and so isn't listed).
    pub(crate) is_main: bool,
}

/// Every live worktree of the repository containing `path`, main checkout first. Worktrees whose
/// folder is gone (git calls them "prunable") are left out.
pub(crate) async fn worktree_list(path: &Path) -> Option<Vec<ListedWorktree>> {
    let out = output(path, &["worktree", "list", "--porcelain", "-z"]).await?;
    Some(parse_worktree_list(&out))
}

fn parse_worktree_list(out: &str) -> Vec<ListedWorktree> {
    let mut worktrees = vec![];
    // With -z, each attribute ends in NUL and an empty attribute ends a record.
    let records = out
        .split("\0\0")
        .filter(|r| !r.trim_matches('\0').is_empty());
    for (position, record) in records.enumerate() {
        let (mut path, mut head, mut branch, mut skip) = (None, String::new(), None, false);
        for attribute in record.split('\0') {
            let (key, value) = attribute.split_once(' ').unwrap_or((attribute, ""));
            match key {
                "worktree" => path = Some(PathBuf::from(value)),
                "HEAD" => head = value.to_owned(),
                "branch" => branch = Some(value.trim_start_matches("refs/heads/").to_owned()),
                "bare" | "prunable" => skip = true,
                _ => {}
            }
        }
        if let (Some(path), false) = (path, skip) {
            worktrees.push(ListedWorktree {
                path,
                head,
                branch,
                is_main: position == 0,
            });
        }
    }
    worktrees
}

/// A worktree's branch tracking and how many files differ from HEAD.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct StatusSummary {
    /// Commits ahead of / behind the upstream; `None` without an upstream.
    pub(crate) ahead: Option<u32>,
    pub(crate) behind: Option<u32>,
    /// Changed, staged, unmerged and untracked paths.
    pub(crate) changed: u32,
}

pub(crate) async fn status_summary(worktree: &Path) -> Option<StatusSummary> {
    let out = output(worktree, &["status", "--porcelain=v2", "--branch", "-z"]).await?;
    Some(parse_status(&out))
}

fn parse_status(out: &str) -> StatusSummary {
    let mut summary = StatusSummary::default();
    let mut entries = out.split('\0').filter(|e| !e.is_empty());
    while let Some(entry) = entries.next() {
        if let Some(ab) = entry.strip_prefix("# branch.ab ") {
            let mut parts = ab.split(' ');
            summary.ahead = parts
                .next()
                .and_then(|a| a.trim_start_matches('+').parse().ok());
            summary.behind = parts
                .next()
                .and_then(|b| b.trim_start_matches('-').parse().ok());
        } else if entry.starts_with("2 ") {
            summary.changed += 1;
            entries.next(); // a rename's original path is its own entry
        } else if entry.starts_with("1 ") || entry.starts_with("u ") || entry.starts_with("? ") {
            summary.changed += 1;
        }
    }
    summary
}
