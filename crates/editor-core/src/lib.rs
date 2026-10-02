//! The Rust core of the agent-first editor. It owns every process and all state; the frontend is a
//! view over its commands and events (ADR 0003). Vocabulary follows `CONTEXT.md`.

mod acp;
mod app_state;
mod auto_suspend;
mod core;
mod create_worktree;
mod diff;
mod documents;
mod edit_notes;
mod files;
mod git;
mod git_status;
mod permissions;
mod prerequisites;
mod process;
mod remove_worktree;
mod session;
mod settings;
mod setup;
mod worktrees;

pub use crate::acp::{AcpError, AdapterCommand, PROTOCOL_VERSION};
pub use crate::auto_suspend::{AutoSuspendReason, Clock, MemoryProbe, MemorySample};
pub use crate::core::{
    AutoSuspension, Core, CoreConfig, CoreError, CoreEvent, RecentSession, TranscriptStream,
    WorkspaceInfo,
};
pub use crate::create_worktree::{BranchInfo, BranchList, CreatedWorktree, NewWorktree};
pub use crate::documents::{FileContent, OpenDocument, OpenedFile, PoppedOutFile, SaveOver};
pub use crate::edit_notes::EditNote;
pub use crate::files::{DirEntry, FileMatch, FileWatchConfig, IndexStats, WatchStatus};
pub use crate::git_status::{CommitOutcome, CommitRequest, GitFile, GitStatus, LastCommit};
pub use crate::prerequisites::{check_prerequisites, MissingPrerequisite, ToolCommand, Tools};
pub use crate::remove_worktree::{CommitSummary, RemovalCheck, RemoveWorktree, RemovedWorktree};
pub use crate::session::{
    DiffLine, DiffLineKind, PermissionMode, PermissionOption, PermissionOptionKind,
    PermissionOutcome, PermissionRequest, SessionId, SessionInfo, SessionState, ToolCallStatus,
    TranscriptDelta, TranscriptItem, TranscriptPage, TRANSCRIPT_PAGE,
};
pub use crate::settings::{
    AgentSettings, EditorSettings, LoadedSettings, Notifications, RepoSettings, Settings,
    WindowsShell,
};
pub use crate::setup::{SetupInfo, SetupStatus};
pub use crate::worktrees::WorktreeInfo;
