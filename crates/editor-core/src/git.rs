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
        return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
    }
    Err(failure(&out))
}

/// What a git that failed said: its stderr, or its stdout when that's empty (`commit` with nothing
/// staged says so there).
fn failure(out: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&out.stderr).trim().to_owned();
    match stderr.is_empty() {
        true => String::from_utf8_lossy(&out.stdout).trim().to_owned(),
        false => stderr,
    }
}

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
    run_remote(repo, &["fetch", "--quiet", "origin"])
        .await
        .map(drop)
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
    // (Not tracking where it starts: an Agent's branch made from `origin/main` would otherwise
    // compare, pull and push against `main`. It tracks its own name once it's pushed.)
    worktree_add(repo, &["--no-track", "-b", name], path, &[start]).await
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

/// Whether `commit` (an ancestor of `base`) is on `base`'s first-parent line: where a branch that
/// started from it, or was fast-forwarded into it, sits. A branch merged with a merge commit is
/// off that line (a second parent's side).
pub(crate) async fn on_first_parent_line(worktree: &Path, commit: &str, base: &str) -> bool {
    if resolve_commit(worktree, base).await.as_deref() == Some(commit) {
        return true;
    }
    let exclude = format!("^{commit}");
    // Walks `base`'s first parents down to `commit`'s history; on the line, the last one's parent
    // is `commit` itself.
    let Some(out) = output(
        worktree,
        &["rev-list", "--first-parent", "--parents", base, &exclude],
    )
    .await
    else {
        return false;
    };
    out.lines()
        .last()
        .and_then(|line| line.split(' ').nth(1))
        .is_some_and(|parent| parent == commit)
}

/// Whether commits were ever made on `branch` (going by its reflog): a branch fast-forwarded into
/// its Base had some; a new one, or one only pulled or reset since, hadn't.
pub(crate) async fn commits_were_made_on(worktree: &Path, branch: &str) -> bool {
    let reference = format!("refs/heads/{branch}");
    output(worktree, &["log", "-g", "--format=%gs", &reference, "--"])
        .await
        .is_some_and(|log| {
            log.lines()
                .any(|entry| entry.starts_with("commit") || entry.starts_with("cherry-pick"))
        })
}

/// Whether the trees of `a` and `b` differ (the branch changed anything at all since `a`).
pub(crate) async fn trees_differ(worktree: &Path, a: &str, b: &str) -> bool {
    run(worktree, &["diff", "--quiet", a, b, "--"])
        .await
        .is_err()
}

/// Whether merging `commit` into `base` would leave `base` as it is: everything `commit` changed is
/// already there, as after a squash or rebase merge. (`merge-tree` touches no index or folder.)
pub(crate) async fn merge_adds_nothing(worktree: &Path, base: &str, commit: &str) -> bool {
    let base_tree = format!("{base}^{{tree}}");
    let Some(base_tree) = output(worktree, &["rev-parse", "--verify", &base_tree]).await else {
        return false;
    };
    // Exit 1 is a conflict: then something differs.
    output(
        worktree,
        &["merge-tree", "--write-tree", "--no-messages", base, commit],
    )
    .await
    .is_some_and(|out| out.lines().next() == Some(base_tree.trim()))
}

/// Whether every commit `commit` has that `base` hasn't has an equivalent patch in `base` (a
/// rebase merge, even where `base` changed the same lines later).
pub(crate) async fn all_picked_into(worktree: &Path, base: &str, commit: &str) -> bool {
    output(worktree, &["cherry", base, commit])
        .await
        .is_some_and(|out| {
            let mut lines = out.lines().peekable();
            lines.peek().is_some() && lines.all(|line| line.starts_with('-'))
        })
}

/// When `commit` was committed (Unix seconds).
pub(crate) async fn commit_time(worktree: &Path, commit: &str) -> Option<i64> {
    output(worktree, &["log", "-1", "--format=%ct", commit, "--"])
        .await?
        .trim()
        .parse()
        .ok()
}

/// The commits on `target`'s first-parent line that `since` doesn't have, oldest first, with their
/// commit times (Unix seconds).
pub(crate) async fn first_parent_line(
    worktree: &Path,
    target: &str,
    since: &str,
) -> Vec<(String, i64)> {
    let exclude = format!("^{since}");
    let out = output(
        worktree,
        &[
            "log",
            "--first-parent",
            "--reverse",
            "--format=%H %ct",
            target,
            &exclude,
            "--",
        ],
    )
    .await
    .unwrap_or_default();
    out.lines()
        .filter_map(|line| {
            let (id, time) = line.split_once(' ')?;
            Some((id.to_owned(), time.parse().ok()?))
        })
        .collect()
}

/// A remote-tracking branch: its short name (`origin/project`) and the commit it's at.
pub(crate) struct RemoteBranch {
    pub(crate) short: String,
    pub(crate) id: String,
}

/// Up to `limit` remote-tracking branches last committed to at or after `since` (Unix seconds),
/// newest first; symbolic ones (`origin/HEAD`) left out.
pub(crate) async fn remote_branches_since(
    worktree: &Path,
    since: i64,
    limit: usize,
) -> Vec<RemoteBranch> {
    let out = output(
        worktree,
        &[
            "for-each-ref",
            "--sort=-committerdate",
            "--format=%(refname:short)%00%(objectname)%00%(committerdate:unix)%00%(symref)",
            "refs/remotes",
        ],
    )
    .await
    .unwrap_or_default();
    out.lines()
        .filter_map(|line| {
            let mut fields = line.split('\0');
            let short = fields.next()?;
            let id = fields.next()?;
            let time: i64 = fields.next()?.parse().ok()?;
            let symref = fields.next().unwrap_or_default();
            Some((short, id, time, symref))
        })
        .filter(|(_, _, _, symref)| symref.is_empty())
        .take_while(|(_, _, time, _)| *time >= since)
        .take(limit)
        .map(|(short, id, _, _)| RemoteBranch {
            short: short.to_owned(),
            id: id.to_owned(),
        })
        .collect()
}

/// How many commits `commit` has that `base` hasn't.
pub(crate) async fn count_not_in(worktree: &Path, base: &str, commit: &str) -> u32 {
    let range = format!("{base}..{commit}");
    output(worktree, &["rev-list", "--count", &range])
        .await
        .and_then(|n| n.trim().parse().ok())
        .unwrap_or(0)
}

/// The branch `branch` was made from, going by its reflog: the last time it was reset onto another
/// branch ("reset: moving to origin/project"), else where it was created ("branch: Created from
/// origin/project"); only a branch other than itself that still exists counts (not `HEAD~1` or
/// a commit id). `None` once the reflog has expired.
pub(crate) async fn created_from(worktree: &Path, branch: &str) -> Option<String> {
    let reference = format!("refs/heads/{branch}");
    let log = output(worktree, &["log", "-g", "--format=%gs", &reference, "--"]).await?;
    // Newest first.
    for entry in log.lines() {
        let Some(from) = entry
            .strip_prefix("reset: moving to ")
            .or_else(|| entry.strip_prefix("branch: Created from "))
        else {
            continue;
        };
        let from = from.trim();
        let name = from
            .trim_start_matches("refs/remotes/")
            .trim_start_matches("refs/heads/");
        // (Nor its own remote branch, `origin/<branch>`.)
        if name == branch || name.split_once('/').is_some_and(|(_, rest)| rest == branch) {
            continue;
        }
        for full in [format!("refs/remotes/{name}"), format!("refs/heads/{name}")] {
            if run(worktree, &["show-ref", "--verify", "--quiet", &full])
                .await
                .is_ok()
            {
                return Some(from.to_owned());
            }
        }
    }
    None
}

/// How many merge commits are on `target`'s first-parent line since `commit`: how many PRs went
/// into it since then.
pub(crate) async fn merges_since(worktree: &Path, commit: &str, target: &str) -> u32 {
    let range = format!("{commit}..{target}");
    output(
        worktree,
        &["rev-list", "--count", "--first-parent", "--merges", &range],
    )
    .await
    .and_then(|n| n.trim().parse().ok())
    .unwrap_or(0)
}

/// Whether `branch` tracks a remote branch that's gone (deleted on the remote, seen at a fetch with
/// `--prune`): what a host does to a PR's branch once it's merged, if set to.
pub(crate) async fn upstream_gone(worktree: &Path, branch: &str) -> bool {
    let reference = format!("refs/heads/{branch}");
    output(
        worktree,
        &["for-each-ref", "--format=%(upstream:track)", &reference],
    )
    .await
    .is_some_and(|track| track.trim() == "[gone]")
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

/// Every file in the Worktree git doesn't ignore and that's there (tracked, plus untracked but not
/// ignored; not tracked files deleted from disk), as `/`-separated paths relative to it, optionally
/// only under `under` (a relative folder).
pub(crate) async fn files(worktree: &Path, under: Option<&str>) -> Result<Vec<String>, String> {
    let list = |extra: &'static [&'static str]| {
        let mut args = vec!["ls-files", "-z"];
        args.extend_from_slice(extra);
        if let Some(under) = under {
            args.extend(["--", under]);
        }
        args
    };
    let all = run(
        worktree,
        &list(&[
            "--cached",
            "--others",
            "--exclude-standard",
            "--deduplicate",
        ]),
    )
    .await?;
    let deleted = run(worktree, &list(&["--deleted"])).await?;
    let deleted: std::collections::HashSet<&str> =
        deleted.split('\0').filter(|p| !p.is_empty()).collect();
    Ok(all
        .split('\0')
        .filter(|p| !p.is_empty() && !deleted.contains(p))
        .map(str::to_owned)
        .collect())
}

/// Which of `paths` (relative to the Worktree, none empty) git ignores.
pub(crate) async fn check_ignored(
    worktree: &Path,
    paths: &[String],
) -> Result<Vec<String>, String> {
    if paths.is_empty() {
        return Ok(vec![]);
    }
    let mut cmd = git(worktree);
    cmd.args(["check-ignore", "--stdin", "-z"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let mut child = cmd.spawn().map_err(|e| format!("could not run git: {e}"))?;
    let mut input = paths.join("\0");
    input.push('\0');
    // Written while the output is read, so a big list can't fill both pipes and stall.
    let writer = child.stdin.take().map(|mut stdin| {
        tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            let _ = stdin.write_all(input.as_bytes()).await;
        }) // (dropping stdin ends the input)
    });
    let out = child.wait_with_output().await.map_err(|e| e.to_string())?;
    if let Some(writer) = writer {
        let _ = writer.await;
    }
    // Exit 1 just means none of them is ignored.
    if !out.status.success() && out.status.code() != Some(1) {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_owned());
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .split('\0')
        .filter(|p| !p.is_empty())
        .map(str::to_owned)
        .collect())
}
/// The Worktree's own git folder (`.git` for the main checkout, `.git/worktrees/<name>` else).
pub(crate) async fn git_dir(worktree: &Path) -> Option<PathBuf> {
    let out = output(worktree, &["rev-parse", "--absolute-git-dir"]).await?;
    Some(PathBuf::from(out.trim()))
}

/// The Git drawer's status: branch, upstream standing, every changed (and untracked) file, and the
/// last commit.
pub(crate) async fn status(worktree: &Path) -> Result<crate::git_status::GitStatus, String> {
    let out = run(
        worktree,
        &[
            "status",
            "--porcelain=v2",
            "--branch",
            "-z",
            "--untracked-files=all",
        ],
    )
    .await?;
    let mut status = crate::git_status::parse(&out);
    status.last_commit = last_commit(worktree)
        .await
        .map(|(id, subject)| crate::git_status::LastCommit { id, subject });
    status.operation = operation(worktree).await;
    Ok(status)
}

/// The merge, rebase, cherry-pick or revert in progress in the Worktree, if any: what its own git
/// folder holds while one waits to be finished.
pub(crate) async fn operation(worktree: &Path) -> Option<crate::git_status::GitOperation> {
    use crate::git_status::GitOperation;
    let dir = git_dir(worktree).await?;
    let has = |name: &str| dir.join(name).exists();
    if has("rebase-apply/applying") {
        Some(GitOperation::Am) // (`git am` keeps its state where a rebase would)
    } else if has("rebase-merge") || has("rebase-apply") {
        Some(GitOperation::Rebase)
    } else if has("MERGE_HEAD") {
        Some(GitOperation::Merge)
    } else if has("CHERRY_PICK_HEAD") {
        Some(GitOperation::CherryPick)
    } else if has("REVERT_HEAD") {
        Some(GitOperation::Revert)
    } else if has("sequencer/todo") {
        // A cherry-pick or revert of several commits, between them: its next step says which.
        let todo = std::fs::read_to_string(dir.join("sequencer/todo")).unwrap_or_default();
        match todo.trim_start().starts_with("revert") {
            true => Some(GitOperation::Revert),
            false => Some(GitOperation::CherryPick),
        }
    } else {
        None
    }
}

/// Aborts the operation in progress: the Worktree goes back to how it was before it started.
pub(crate) async fn abort(
    worktree: &Path,
    operation: crate::git_status::GitOperation,
) -> Result<(), String> {
    run(worktree, &[operation.command(), "--abort"])
        .await
        .map(drop)
}

/// Switches the Worktree to the local branch `local`, made first from `track` (a remote branch,
/// which it then tracks) if given. Uncommitted changes come along if git can carry them, else it
/// refuses.
pub(crate) async fn switch(
    worktree: &Path,
    local: &str,
    track: Option<&str>,
) -> Result<(), String> {
    match track {
        Some(remote) => {
            run(
                worktree,
                &["switch", "--quiet", "-c", local, "--track", remote],
            )
            .await
        }
        None => run(worktree, &["switch", "--quiet", local]).await,
    }
    .map(drop)
}

/// Runs git on `paths` (relative, `/`-separated), as pathspecs that match only themselves. Never
/// on none: an empty pathspec would mean everything.
async fn run_on(worktree: &Path, args: &[&str], paths: &[String]) -> Result<String, String> {
    if paths.is_empty() {
        return Err("No files given.".into());
    }
    let paths: Vec<String> = paths.iter().map(|p| format!(":(literal){p}")).collect();
    let mut all: Vec<&str> = args.to_vec();
    all.push("--");
    all.extend(paths.iter().map(String::as_str));
    run(worktree, &all).await
}

async fn has_head(worktree: &Path) -> bool {
    run(worktree, &["rev-parse", "--verify", "-q", "HEAD"])
        .await
        .is_ok()
}

/// `paths` with the originals of any staged renames among them: a rename is staged and unstaged
/// as one.
async fn with_rename_originals(worktree: &Path, paths: &[String]) -> Result<Vec<String>, String> {
    let status = status(worktree).await?;
    let mut all = paths.to_vec();
    for file in status.files {
        if let (true, Some(from)) = (paths.contains(&file.path), file.renamed_from) {
            all.push(from);
        }
    }
    Ok(all)
}

/// Out of the index only (`-f`: even a new file edited since it was staged; the file itself stays).
const UNSTAGE_NEW: &[&str] = &["rm", "-q", "-r", "-f", "--cached", "--ignore-unmatch"];

/// Stages whole files (changes, new files and deletions alike).
pub(crate) async fn stage(worktree: &Path, paths: &[String]) -> Result<(), String> {
    let paths = with_rename_originals(worktree, paths).await?;
    run_on(worktree, &["add", "-A"], &paths).await.map(drop)
}

pub(crate) async fn stage_all(worktree: &Path) -> Result<(), String> {
    run(worktree, &["add", "-A"]).await.map(drop)
}

/// Unstages whole files (before the first commit too, when there's no HEAD to reset to).
pub(crate) async fn unstage(worktree: &Path, paths: &[String]) -> Result<(), String> {
    let paths = with_rename_originals(worktree, paths).await?;
    if has_head(worktree).await {
        run_on(worktree, &["reset", "-q", "HEAD"], &paths)
            .await
            .map(drop)
    } else {
        run_on(worktree, UNSTAGE_NEW, &paths).await.map(drop)
    }
}

pub(crate) async fn unstage_all(worktree: &Path) -> Result<(), String> {
    if has_head(worktree).await {
        run(worktree, &["reset", "-q"]).await.map(drop)
    } else {
        run_on(worktree, UNSTAGE_NEW, &[".".to_owned()])
            .await
            .map(drop)
    }
}

/// How long a commit may take (its hooks included) before the editor gives up on it.
const COMMIT_TIMEOUT: Duration = Duration::from_secs(300);

/// Commits what's staged with `message` (amending the last commit if `amend`; an empty message
/// then keeps its own). The new commit's short id.
pub(crate) async fn commit(worktree: &Path, message: &str, amend: bool) -> Result<String, String> {
    let mut args = vec!["commit", "-q"];
    if amend {
        args.push("--amend");
    }
    if message.trim().is_empty() && amend {
        args.push("--no-edit");
    } else {
        args.extend(["-m", message]);
    }
    // (A hook or a signing prompt that never finishes: given up on, and the drawer is usable again.)
    let mut cmd = git(worktree);
    cmd.args(&args).kill_on_drop(true);
    let out = tokio::time::timeout(COMMIT_TIMEOUT, cmd.output())
        .await
        .map_err(|_| {
            "The commit took too long (a hook or signing prompt?) and was stopped.".to_owned()
        })?
        .map_err(|e| format!("could not run git: {e}"))?;
    if !out.status.success() {
        return Err(failure(&out));
    }
    Ok(run(worktree, &["rev-parse", "--short", "HEAD"])
        .await?
        .trim()
        .to_owned())
}

/// The last commit's short id and subject, if there is one.
pub(crate) async fn last_commit(worktree: &Path) -> Option<(String, String)> {
    let out = output(worktree, &["log", "-1", "--format=%h%x00%s"]).await?;
    let (id, subject) = out.trim_end().split_once('\0')?;
    Some((id.to_owned(), subject.to_owned()))
}

/// Whether HEAD is on a remote branch already (so amending it rewrites pushed history). If git
/// can't say, it's taken as not pushed.
pub(crate) async fn head_pushed(worktree: &Path) -> bool {
    output(
        worktree,
        &["branch", "-r", "--contains", "HEAD", "--format=%(refname)"],
    )
    .await
    .is_some_and(|out| !out.trim().is_empty())
}

/// Throws away every change to `path` (staged or not): back to HEAD's version, or gone if HEAD
/// hasn't got it (a new file). A staged rename's original comes back too.
pub(crate) async fn discard(worktree: &Path, path: &str) -> Result<(), String> {
    let has_head = has_head(worktree).await;
    for path in with_rename_originals(worktree, &[path.to_owned()]).await? {
        let in_head = has_head
            && run(worktree, &["cat-file", "-e", &format!("HEAD:{path}")])
                .await
                .is_ok();
        let paths = [path.clone()];
        if in_head {
            run_on(
                worktree,
                &["restore", "--source=HEAD", "--staged", "--worktree"],
                &paths,
            )
            .await?;
        } else {
            run_on(worktree, UNSTAGE_NEW, &paths).await?;
            let on_disk = worktree.join(&path);
            if on_disk.is_dir() {
                return Err(format!(
                    "{path} is a folder (another repository?): delete it yourself if you mean to"
                ));
            }
            match std::fs::remove_file(&on_disk) {
                Err(err) if err.kind() != std::io::ErrorKind::NotFound => {
                    return Err(format!("couldn't delete {path}: {err}"))
                }
                _ => {}
            }
        }
    }
    Ok(())
}

/// How long a push, fetch or pull may take before it's stopped (a dead network; logins fail at once,
/// prompts being off). Generous: a first push of a big branch takes a while.
const NETWORK_TIMEOUT: Duration = Duration::from_secs(300);

/// What to tell the user when git couldn't log in by itself.
pub(crate) const LOGIN_HINT: &str =
    "If git needs you to log in, run `git push` in a terminal once, then try again here.";

/// Signs, in git's (or ssh's, or a credential helper's) words, that it needed a login.
const LOGIN_SIGNS: &[&str] = &[
    "terminal prompts disabled",
    "could not read username",
    "could not read password",
    "authentication failed",
    "permission denied (publickey",
    "host key verification failed",
    "returned error: 401",
    "returned error: 403",
    "user interactivity has been disabled",
    "passphrase",
];

/// Runs a git command that talks to a remote: the user's own credential helpers and SSH setup, but
/// never a prompt or a login window (SSH in batch mode unless the user has an SSH command of their
/// own; Git Credential Manager non-interactive), and never longer than `NETWORK_TIMEOUT`, after
/// which git and everything it started are stopped. A failure that looks like a login problem
/// gets `LOGIN_HINT`.
async fn run_remote(worktree: &Path, args: &[&str]) -> Result<String, String> {
    let mut cmd = git(worktree);
    // (Nobody's askpass either: a terminal's, VS Code's, inherited from wherever the editor started.)
    cmd.args(["-c", "core.askPass="])
        .args(args)
        .env_remove("GIT_ASKPASS")
        .env_remove("SSH_ASKPASS")
        .env("GCM_INTERACTIVE", "never")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let own_ssh = std::env::var_os("GIT_SSH_COMMAND").is_some()
        || std::env::var_os("GIT_SSH").is_some()
        || output(worktree, &["config", "core.sshCommand"])
            .await
            .is_some_and(|c| !c.trim().is_empty());
    // (Someone's own SSH command is theirs to keep: a passphrase it asks for then fails only at
    // the timeout.)
    if !own_ssh {
        cmd.env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes");
    }
    crate::process::ProcessTree::own_group(&mut cmd);
    let child = cmd.spawn().map_err(|e| format!("could not run git: {e}"))?;
    // (Dropped on the way out, it stops whatever git left running: ssh, a credential helper.)
    let _tree = crate::process::ProcessTree::attach(&child);
    let out = match tokio::time::timeout(NETWORK_TIMEOUT, child.wait_with_output()).await {
        Err(_) => {
            return Err(format!(
                "git {} took over {} s and was stopped. {LOGIN_HINT}",
                args[0],
                NETWORK_TIMEOUT.as_secs()
            ))
        }
        Ok(out) => out.map_err(|e| format!("could not run git: {e}"))?,
    };
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
    }
    let message = failure(&out);
    let lower = message.to_lowercase();
    Err(match LOGIN_SIGNS.iter().any(|sign| lower.contains(sign)) {
        true => format!("{message}\n{LOGIN_HINT}"),
        false => message,
    })
}

/// Fetches the branch's remote (or `origin`), pruning branches gone from it.
pub(crate) async fn fetch(worktree: &Path) -> Result<(), String> {
    run_remote(worktree, &["fetch", "--quiet", "--prune"])
        .await
        .map(drop)
}

/// The checked-out branch, if HEAD isn't detached.
async fn current_branch(worktree: &Path) -> Option<String> {
    let out = output(worktree, &["symbolic-ref", "--quiet", "--short", "HEAD"]).await?;
    Some(out.trim().to_owned()).filter(|b| !b.is_empty())
}

async fn config(worktree: &Path, key: &str) -> Option<String> {
    let value = output(worktree, &["config", "--get", key]).await?;
    Some(value.trim().to_owned()).filter(|v| !v.is_empty())
}

/// The remote a branch pushes to: its `pushRemote`, the repo's `pushDefault`, the remote it tracks,
/// the only remote there is, or `origin`.
async fn push_remote(worktree: &Path, branch: &str) -> String {
    for key in [
        format!("branch.{branch}.pushRemote"),
        "remote.pushDefault".to_owned(),
        format!("branch.{branch}.remote"),
    ] {
        if let Some(remote) = config(worktree, &key).await.filter(|r| r != ".") {
            return remote;
        }
    }
    let remotes = output(worktree, &["remote"]).await.unwrap_or_default();
    let remotes: Vec<&str> = remotes
        .lines()
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .collect();
    match remotes[..] {
        [only] => only.to_owned(),
        _ => "origin".to_owned(),
    }
}

/// Pushes the branch to the branch of the same name on its remote, which becomes its upstream.
/// Always by name: a branch tracking another (an Agent's branch made from `origin/main`) never
/// pushes onto that one, whatever `push.default` says. A push the remote turns down because it has
/// commits this branch hasn't is `Rejected`, not an error.
pub(crate) async fn push(worktree: &Path) -> Result<crate::git_status::PushOutcome, String> {
    use crate::git_status::PushOutcome;
    let branch = current_branch(worktree)
        .await
        .ok_or("HEAD is detached: check out a branch to push.")?;
    let remote = push_remote(worktree, &branch).await;
    let target = format!("HEAD:refs/heads/{branch}");
    match run_remote(
        worktree,
        &["push", "--quiet", "--set-upstream", &remote, &target],
    )
    .await
    {
        Ok(_) => Ok(PushOutcome::Pushed {
            to: format!("{remote}/{branch}"),
        }),
        Err(message)
            if ["[rejected]", "non-fast-forward", "fetch first"]
                .iter()
                .any(|sign| message.contains(sign)) =>
        {
            Ok(PushOutcome::Rejected)
        }
        Err(message) => Err(message),
    }
}

/// Commits ahead of and behind the upstream (`None` without one, or when it's gone).
pub(crate) async fn ahead_behind(worktree: &Path) -> Option<(u32, u32)> {
    let out = output(
        worktree,
        &["rev-list", "--left-right", "--count", "HEAD...@{upstream}"],
    )
    .await?;
    let mut counts = out.split_whitespace().map(|n| n.parse().ok());
    Some((counts.next()??, counts.next()??))
}

/// Whether the branch is set to track something (even if it's gone from the remote).
pub(crate) async fn has_upstream_set(worktree: &Path) -> bool {
    match current_branch(worktree).await {
        Some(branch) => config(worktree, &format!("branch.{branch}.merge"))
            .await
            .is_some(),
        None => false,
    }
}

/// Fast-forwards the branch to its upstream (already fetched). Never merges or rebases, and never
/// stashes local changes to do it.
pub(crate) async fn fast_forward(worktree: &Path) -> Result<(), String> {
    run(
        worktree,
        &[
            "merge",
            "--ff-only",
            "--no-autostash",
            "--quiet",
            "@{upstream}",
        ],
    )
    .await
    .map(drop)
}

/// Where the Worktree's branch split from `base`: their merge-base.
pub(crate) async fn merge_base(worktree: &Path, base: &str) -> Result<String, String> {
    match run(worktree, &["merge-base", "HEAD", base]).await {
        Ok(split) => Ok(split.trim().to_owned()),
        Err(err) if !err.is_empty() => Err(err),
        Err(_) => {
            let shallow = common_dir(worktree)
                .await
                .is_some_and(|dir| dir.join("shallow").exists());
            Err(match shallow {
                true => format!(
                    "`{base}` and this branch have no commit in common here: the history is shallow, so fetch more of it (`git fetch --unshallow`)."
                ),
                false => format!("`{base}` and this branch have no commit in common."),
            })
        }
    }
}

/// What the branch changed since `split` (where it split from its Base: so a three-dot diff),
/// renames found: `(status letter, path, original path of a rename)`.
pub(crate) async fn changes_since(
    worktree: &Path,
    split: &str,
) -> Result<Vec<(char, String, Option<String>)>, String> {
    let out = run(
        worktree,
        &[
            "diff",
            "--name-status",
            "-z",
            "--find-renames",
            "--no-ext-diff",
            split,
            "HEAD",
        ],
    )
    .await?;
    let mut entries = out.split('\0').filter(|e| !e.is_empty());
    let mut changes = vec![];
    while let Some(status) = entries.next() {
        let letter = status.chars().next().unwrap_or('M');
        // A rename names its original, then where it is now.
        let (path, from) = if letter == 'R' {
            let from = entries.next().unwrap_or_default().to_owned();
            (entries.next().unwrap_or_default().to_owned(), Some(from))
        } else {
            (entries.next().unwrap_or_default().to_owned(), None)
        };
        changes.push((letter, path, from));
    }
    Ok(changes)
}

/// A file's bytes at `rev` (None if it isn't there).
pub(crate) async fn file_at(worktree: &Path, rev: &str, path: &str) -> Option<Vec<u8>> {
    let out = git(worktree)
        .args(["cat-file", "blob", &format!("{rev}:{path}")])
        .output()
        .await
        .ok()?;
    out.status.success().then_some(out.stdout)
}
