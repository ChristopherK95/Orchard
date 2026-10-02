//! The Git drawer's view of a Worktree (ticket 18): its branch, how it stands against upstream, and
//! every changed file with what's staged and what isn't.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitStatus {
    /// None when HEAD is detached.
    pub branch: Option<String>,
    pub upstream: Option<String>,
    pub ahead: Option<u32>,
    pub behind: Option<u32>,
    pub files: Vec<GitFile>,
    /// None before the first commit.
    pub last_commit: Option<LastCommit>,
    /// A merge, rebase, cherry-pick or revert in progress (whoever started it): its conflicted files
    /// are the `conflicted` ones.
    pub operation: Option<GitOperation>,
    /// The sessions in this Worktree in the middle of a turn, Working or Needs you (a branch switch
    /// waits for them).
    pub mid_turn: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum GitOperation {
    Merge,
    Rebase,
    CherryPick,
    Revert,
    /// `git am`, applying patches.
    Am,
}

impl GitOperation {
    /// The git command that started it (and `--abort`s it).
    pub(crate) fn command(self) -> &'static str {
        match self {
            Self::Merge => "merge",
            Self::Rebase => "rebase",
            Self::CherryPick => "cherry-pick",
            Self::Revert => "revert",
            Self::Am => "am",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LastCommit {
    pub id: String,
    pub subject: String,
}

/// A changed file. `staged` and `unstaged` are git's status letters (`M`, `A`, `D`, `R`, …;
/// `?` for an untracked file, which is unstaged).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitFile {
    /// Relative to the Worktree, `/`-separated.
    pub path: String,
    pub staged: Option<String>,
    pub unstaged: Option<String>,
    /// A rename's (or copy's) original path.
    pub renamed_from: Option<String>,
    /// Unmerged (a conflict to resolve).
    pub conflicted: bool,
}

/// A commit as asked for in the drawer, and how far it's been confirmed.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitRequest {
    pub message: String,
    /// "Amend last commit".
    #[serde(default)]
    pub amend: bool,
    /// "Commit anyway" while a session in the Worktree is Working.
    #[serde(default)]
    pub even_if_working: bool,
    /// Amend even though the last commit is already pushed.
    #[serde(default)]
    pub even_if_pushed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum CommitOutcome {
    Committed {
        id: String,
    },
    /// Sessions in the Worktree are Working (they may be changing what's committed): ask first.
    SessionsWorking {
        sessions: Vec<String>,
    },
    /// Amending would rewrite a commit that's already pushed: ask first.
    AlreadyPushed,
}

/// What a push did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PushOutcome {
    /// To `remote/branch`.
    Pushed { to: String },
    /// The remote has commits this branch hasn't: pull first (or, if it has diverged, merge or
    /// rebase, which the editor leaves to an Agent or a terminal).
    Rejected,
}

/// What a pull did. It only ever fast-forwards.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PullOutcome {
    /// Sessions in the Worktree are mid-turn (a pull rewrites files under them): ask first.
    SessionsWorking {
        sessions: Vec<String>,
    },
    UpToDate,
    FastForwarded {
        commits: u32,
    },
    /// Both sides have commits the other hasn't: that takes a merge or a rebase, which the editor
    /// leaves to an Agent or a terminal.
    Diverged {
        ahead: u32,
        behind: u32,
    },
}

/// Parses `git status --porcelain=v2 --branch -z`.
pub(crate) fn parse(out: &str) -> GitStatus {
    let mut status = GitStatus::default();
    let letter = |c: char| (c != '.').then(|| c.to_string());
    let mut entries = out.split('\0').filter(|e| !e.is_empty());
    while let Some(entry) = entries.next() {
        if let Some(head) = entry.strip_prefix("# branch.head ") {
            status.branch = (head != "(detached)").then(|| head.to_owned());
        } else if let Some(upstream) = entry.strip_prefix("# branch.upstream ") {
            status.upstream = Some(upstream.to_owned());
        } else if let Some(ab) = entry.strip_prefix("# branch.ab ") {
            let mut parts = ab.split(' ');
            status.ahead = parts
                .next()
                .and_then(|a| a.trim_start_matches('+').parse().ok());
            status.behind = parts
                .next()
                .and_then(|b| b.trim_start_matches('-').parse().ok());
        } else if let Some(path) = entry.strip_prefix("? ") {
            status.files.push(GitFile {
                path: path.to_owned(),
                staged: None,
                unstaged: Some("?".into()),
                renamed_from: None,
                conflicted: false,
            });
        } else if let Some(kind) = entry
            .chars()
            .next()
            .filter(|c| matches!(c, '1' | '2' | 'u'))
        {
            // "1 XY sub mH mI mW hH hI path"; "2 … Xscore path" then the original as its own entry;
            // "u XY sub m1 m2 m3 mW h1 h2 h3 path".
            let fields = match kind {
                '1' => 8,
                '2' => 9,
                _ => 10,
            };
            let mut parts = entry.splitn(fields + 1, ' ');
            let xy: Vec<char> = parts.nth(1).unwrap_or("..").chars().collect();
            let Some(path) = parts.nth(fields - 2) else {
                continue;
            };
            // (A copy's source is its own file: only a rename's original goes with it.)
            let original = (kind == '2').then(|| entries.next().unwrap_or_default().to_owned());
            let renamed_from = original.filter(|_| xy.contains(&'R'));
            let conflicted = kind == 'u';
            status.files.push(GitFile {
                path: path.to_owned(),
                staged: (!conflicted).then(|| letter(xy[0])).flatten(),
                unstaged: if conflicted {
                    Some("U".into())
                } else {
                    letter(xy[1])
                },
                renamed_from,
                conflicted,
            });
        }
    }
    status
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn branches_files_and_renames_are_read() {
        let out = [
            "# branch.oid abc",
            "# branch.head main",
            "# branch.upstream origin/main",
            "# branch.ab +2 -1",
            "1 M. N... 100644 100644 100644 aaa bbb src/a b.rs",
            "1 .D N... 100644 100644 000000 aaa aaa gone.rs",
            "2 R. N... 100644 100644 100644 aaa aaa R100 new.rs",
            "old.rs",
            "u UU N... 100644 100644 100644 100644 a b c both.rs",
            "? notes.txt",
            "",
        ]
        .join("\0");
        let status = parse(&out);
        assert_eq!(status.branch.as_deref(), Some("main"));
        assert_eq!((status.ahead, status.behind), (Some(2), Some(1)));
        let f = |p: &str| status.files.iter().find(|f| f.path == p).unwrap().clone();
        assert_eq!(
            (f("src/a b.rs").staged, f("src/a b.rs").unstaged),
            (Some("M".into()), None)
        );
        assert_eq!(
            (f("gone.rs").staged, f("gone.rs").unstaged),
            (None, Some("D".into()))
        );
        assert_eq!(f("new.rs").renamed_from.as_deref(), Some("old.rs"));
        assert!(f("both.rs").conflicted);
        assert_eq!(f("notes.txt").unstaged.as_deref(), Some("?"));
        assert_eq!(status.files.len(), 5);
    }
}
