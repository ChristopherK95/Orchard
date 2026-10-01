//! The Rust core of the agent-first editor. It owns every process and all state; the frontend is a
//! view over its commands and events (ADR 0003). Vocabulary follows `CONTEXT.md`.

mod acp;
mod core;
mod git;
mod permissions;
mod prerequisites;
mod process;
mod session;
mod worktrees;

pub use crate::acp::{AcpError, AdapterCommand, PROTOCOL_VERSION};
pub use crate::core::{Core, CoreConfig, CoreError, CoreEvent, TranscriptStream, WorkspaceInfo};
pub use crate::prerequisites::{check_prerequisites, MissingPrerequisite, ToolCommand, Tools};
pub use crate::session::{
    DiffLine, DiffLineKind, PermissionMode, PermissionOption, PermissionOptionKind,
    PermissionOutcome, PermissionRequest, SessionId, SessionInfo, SessionState, ToolCallStatus,
    TranscriptDelta, TranscriptItem, TranscriptPage, TRANSCRIPT_PAGE,
};
pub use crate::worktrees::WorktreeInfo;
