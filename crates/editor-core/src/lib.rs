//! The Rust core of the agent-first editor. It owns every process and all state; the frontend is a
//! view over its commands and events (ADR 0003). Vocabulary follows `CONTEXT.md`.

mod acp;
mod core;
mod prerequisites;
mod session;

pub use crate::acp::{AcpError, AdapterCommand, PROTOCOL_VERSION};
pub use crate::core::{Core, CoreConfig, CoreError, CoreEvent, TranscriptStream, WorkspaceInfo};
pub use crate::prerequisites::{check_prerequisites, MissingPrerequisite, ToolCommand, Tools};
pub use crate::session::{SessionId, SessionInfo, SessionState, TranscriptDelta, TranscriptItem};
