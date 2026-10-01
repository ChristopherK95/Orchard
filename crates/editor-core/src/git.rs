//! The one place the core runs `git` (ADR 0004). New git operations belong here, so a library can
//! later replace a hot path without touching callers. Output is always machine-readable
//! (`--porcelain` / `-z`), prompts are off (`GIT_TERMINAL_PROMPT=0`), and nothing here takes
//! git's optional locks (`GIT_OPTIONAL_LOCKS=0`), so background refreshes never block an Agent.
//! Optional locks only cover git's opportunistic index refresh, so turning them off is safe for the
//! editor's own commands too.

use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::process::Command;

use crate::create_worktree::BranchInfo;
use crate::remove_worktree::CommitSummary;

fn git(cwd: &Path) -> Command {
    let mut cmd = crate::process::command("git");
    cmd.current_dir(cwd)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        // No terminal to ask on: Git for Windows would otherwise wait on "Deletion of directory
        // failed. Should I try again? (y/n)" when a folder is in use.
        .stdin(std::process::Stdio::null());
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

/// How long a fetch may take before the editor gives up and uses what was fetched last (a login
/// prompt from a credential helper would otherwise hang it).
const FETCH_TIMEOUT: Duration = Duration::from_secs(30);

async fn has_origin(repo: &Path) -> bool {
    run(repo, &["remote"])
        .await
        .is_ok_and(|remotes| remotes.lines().any(|r| r.trim() == "origin"))
}

/// Fetches `origin` (if the repository has one) so new Worktrees start from what's upstream now.
pub(crate) async fn fetch_origin(repo: &Path) -> Result<(), String> {
    if !has_origin(repo).await {
        return Ok(());
    }
    match tokio::time::timeout(FETCH_TIMEOUT, run(repo, &["fetch", "--quiet", "origin"])).await {
        Ok(result) => result.map(|_| ()),
        Err(_) => Err(format!("timed out after {} s", FETCH_TIMEOUT.as_secs())),
    }
}

/// `origin`'s URL, which a repo's settings section can be keyed by.
pub(crate) async fn origin_url(repo: &Path) -> Option<String> {
    let url = output(repo, &["remote", "get-url", "origin"]).await?;
    Some(url.trim().to_owned()).filter(|url| !url.is_empty())
}

/// Where git's helper programs live (on Windows, inside the Git for Windows install).
pub(crate) async fn exec_path() -> Option<PathBuf> {
    let path = output(Path::new("."), &["--exec-path"]).await?;
    Some(PathBuf::from(path.trim())).filter(|p| p.is_absolute())
}

/// Where a new branch starts by default: `origin/HEAD`'s target, else origin's copy of the main
/// checkout's branch (or `origin/main` / `origin/master`), else the main checkout's own branch.
pub(crate) async fn default_start_point(repo: &Path) -> String {
    let origin_head = output(
        repo,
        &[
            "symbolic-ref",
            "--quiet",
            "--short",
            "refs/remotes/origin/HEAD",
        ],
    )
    .await;
    if let Some(origin_head) = origin_head {
        return origin_head.trim().to_owned();
    }
    let local = output(repo, &["symbolic-ref", "--quiet", "--short", "HEAD"])
        .await
        .map(|b| b.trim().to_owned());
    if has_origin(repo).await {
        let candidates = local
            .iter()
            .cloned()
            .chain(["main".into(), "master".into()]);
        for branch in candidates {
            let remote = format!("origin/{branch}");
            if is_commit(repo, &remote).await {
                return remote;
            }
        }
    }
    local.unwrap_or_else(|| "HEAD".into())
}

/// Whether `rev` names a commit (branch, tag, id, …). `--end-of-options` stops git reading a
/// `rev` that starts with `-` as an option.
pub(crate) async fn is_commit(repo: &Path, rev: &str) -> bool {
    let commit = format!("{rev}^{{commit}}");
    run(
        repo,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            "--end-of-options",
            &commit,
        ],
    )
    .await
    .is_ok()
}

/// Local and remote-tracking branches, with where each local one is checked out.
pub(crate) async fn branches(repo: &Path) -> Result<Vec<BranchInfo>, String> {
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
                return Some(BranchInfo {
                    name: name.to_owned(),
                    remote: false,
                    checked_out_in,
                });
            }
            let name = refname.strip_prefix("refs/remotes/")?;
            (!name.ends_with("/HEAD")).then(|| BranchInfo {
                name: name.to_owned(),
                remote: true,
                checked_out_in: None,
            })
        })
        .collect())
}

/// `git worktree add <options…> <path> <rest…>`.
async fn worktree_add(
    repo: &Path,
    options: &[&str],
    path: &Path,
    rest: &[&str],
) -> Result<(), String> {
    let path = path.to_string_lossy();
    let mut args = vec!["worktree", "add", "--quiet"];
    args.extend_from_slice(options);
    args.push(&path);
    args.extend_from_slice(rest);
    run(repo, &args).await.map(|_| ())
}

/// Adds a Worktree at `path` on a new branch `name` starting at `start` (already verified).
pub(crate) async fn worktree_add_new_branch(
    repo: &Path,
    path: &Path,
    name: &str,
    start: &str,
) -> Result<(), String> {
    worktree_add(repo, &["-b", name], path, &[start]).await
}

/// Adds a Worktree at `path` on the existing local branch `name`.
pub(crate) async fn worktree_add_existing(
    repo: &Path,
    path: &Path,
    name: &str,
) -> Result<(), String> {
    worktree_add(repo, &[], path, &[name]).await
}

/// Adds a Worktree at `path` on a new local branch `name` tracking the remote branch `remote`.
pub(crate) async fn worktree_add_tracking(
    repo: &Path,
    path: &Path,
    name: &str,
    remote: &str,
) -> Result<(), String> {
    worktree_add(repo, &["--track", "-b", name], path, &[remote]).await
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

/// Uncommitted changes in a Worktree, one `XY path` line each (`??` for untracked files), as
/// `git status --short` shows them. Ignored files aren't listed.
pub(crate) async fn changes(worktree: &Path) -> Result<Vec<String>, String> {
    let out = run(
        worktree,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )
    .await?;
    Ok(parse_changes(&out))
}

fn parse_changes(out: &str) -> Vec<String> {
    let mut changes = vec![];
    let mut entries = out.split('\0').filter(|e| !e.is_empty());
    while let Some(entry) = entries.next() {
        // A rename's or copy's original path is its own entry (either column can say so).
        if entry
            .as_bytes()
            .iter()
            .take(2)
            .any(|c| matches!(c, b'R' | b'C'))
        {
            entries.next();
        }
        changes.push(entry.to_owned());
    }
    changes
}

/// Ignored files and folders in a Worktree (folders collapsed, like `node_modules/`): removal
/// deletes them too, though they're not "changes".
pub(crate) async fn ignored(worktree: &Path) -> Result<Vec<String>, String> {
    let out = run(
        worktree,
        &[
            "ls-files",
            "--others",
            "--ignored",
            "--exclude-standard",
            "--directory",
            "-z",
        ],
    )
    .await?;
    Ok(out
        .split('\0')
        .filter(|e| !e.is_empty())
        .map(str::to_owned)
        .collect())
}

/// Uncommitted changes to tracked files, as a binary diff against HEAD (for telling whether they
/// changed, not for showing).
pub(crate) async fn diff_vs_head(worktree: &Path) -> Result<String, String> {
    run(worktree, &["diff", "HEAD", "--binary", "--no-ext-diff"]).await
}

/// The full id of the commit `rev` names in `repo`, if it names one.
pub(crate) async fn resolve_commit(repo: &Path, rev: &str) -> Option<String> {
    let commit = format!("{rev}^{{commit}}");
    let id = output(
        repo,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            "--end-of-options",
            &commit,
        ],
    )
    .await?;
    Some(id.trim().to_owned()).filter(|id| !id.is_empty())
}

/// The arguments selecting commits only `HEAD` has: not on a remote, a tag, `base`, or a local
/// branch other than `branch` (with `branch` `None`, HEAD is detached and no branch holds it).
fn only_here<'a>(exclude: &'a str, base: &'a str) -> Vec<&'a str> {
    let mut args = vec!["HEAD", "--not", "--remotes", "--tags"];
    if !exclude.is_empty() {
        args.push(exclude);
    }
    args.extend(["--branches", "--end-of-options", base]);
    args
}

/// Up to `limit` of the commits only this Worktree's `HEAD` has (newest first), and how many there
/// are in all. These are what removing the Worktree (when detached) or deleting its branch loses.
pub(crate) async fn commits_only_here(
    worktree: &Path,
    branch: Option<&str>,
    base: &str,
    limit: usize,
) -> Result<(Vec<CommitSummary>, usize), String> {
    let exclude = branch.map(|b| format!("--exclude={b}")).unwrap_or_default();
    let selection = only_here(&exclude, base);
    let mut count_args = vec!["rev-list", "--count"];
    count_args.extend(&selection);
    let count = run(worktree, &count_args)
        .await?
        .trim()
        .parse()
        .map_err(|e| format!("unexpected rev-list output: {e}"))?;
    let max = format!("--max-count={limit}");
    let mut log_args = vec!["log", "--format=%h%x00%s", &max];
    log_args.extend(&selection);
    let commits = run(worktree, &log_args)
        .await?
        .lines()
        .filter_map(|line| line.split_once('\0'))
        .map(|(id, subject)| CommitSummary {
            id: id.to_owned(),
            subject: subject.to_owned(),
        })
        .collect();
    Ok((commits, count))
}

/// Whether `commit` is already contained in `base`.
pub(crate) async fn is_merged(worktree: &Path, commit: &str, base: &str) -> bool {
    run(worktree, &["merge-base", "--is-ancestor", commit, base])
        .await
        .is_ok()
}

/// Removes the Worktree at `path`. `force` (only when the user chose "Discard and remove") lets git
/// delete uncommitted changes with it.
pub(crate) async fn worktree_remove(repo: &Path, path: &Path, force: bool) -> Result<(), String> {
    let path = path.display().to_string();
    let mut args = vec!["worktree", "remove"];
    if force {
        args.push("--force");
    }
    args.extend(["--", &path]);
    run(repo, &args).await.map(|_| ())
}

/// Deletes the local branch `name` (whatever it holds: the caller checked nothing is lost).
pub(crate) async fn delete_branch(repo: &Path, name: &str) -> Result<(), String> {
    run(repo, &["branch", "-D", "--", name]).await.map(|_| ())
}
