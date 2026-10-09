//! The Dashboard (issue 43): what it shows that the core works out — the Worktrees that need
//! attention, and why.

use std::path::PathBuf;

use serde::Serialize;

use crate::git_status::{GitOperation, GitStatus};

/// A Worktree of the Workspace (the main checkout too) that needs attention, with every reason.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeAttention {
    pub path: PathBuf,
    /// None when HEAD is detached.
    pub branch: Option<String>,
    pub reasons: Vec<AttentionReason>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AttentionReason {
    /// Changed files not committed (untracked ones too).
    Uncommitted { files: u32 },
    /// Commits its upstream hasn't.
    Unpushed { commits: u32 },
    /// Commits its upstream has that it hasn't.
    Behind { commits: u32 },
    /// A branch with no upstream, and commits since its Base.
    NoUpstream { commits: u32 },
    /// A merge, rebase, cherry-pick, revert or `git am` in progress.
    Operation { operation: GitOperation },
}

/// Why a Worktree with this status needs attention (none: it doesn't). `since_base` is how many
/// commits its branch has made since its Base, which counts only when it has no upstream.
pub(crate) fn reasons(status: &GitStatus, since_base: u32) -> Vec<AttentionReason> {
    let mut reasons = vec![];
    if !status.files.is_empty() {
        reasons.push(AttentionReason::Uncommitted {
            files: status.files.len() as u32,
        });
    }
    match status.ahead {
        Some(commits) if commits > 0 => reasons.push(AttentionReason::Unpushed { commits }),
        _ => {}
    }
    match status.behind {
        Some(commits) if commits > 0 => reasons.push(AttentionReason::Behind { commits }),
        _ => {}
    }
    if status.branch.is_some() && status.upstream.is_none() && since_base > 0 {
        reasons.push(AttentionReason::NoUpstream {
            commits: since_base,
        });
    }
    if let Some(operation) = status.operation {
        reasons.push(AttentionReason::Operation { operation });
    }
    reasons
}
