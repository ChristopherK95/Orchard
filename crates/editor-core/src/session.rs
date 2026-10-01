//! Agent session vocabulary shared with the frontend (see `CONTEXT.md`).

use std::path::PathBuf;

use serde::Serialize;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, serde::Deserialize,
)]
#[serde(transparent)]
pub struct SessionId(pub u64);

/// The Agent session states this slice supports. Suspended arrives in a later ticket.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SessionState {
    /// The Agent is mid-turn.
    Working,
    /// The Agent is blocked on the user: a permission prompt is waiting.
    NeedsYou,
    /// The turn has finished and the Agent is waiting for the next prompt.
    Idle,
    /// The Agent's process ended without being Suspended (crash, or it quit).
    Exited,
}

/// How much the Agent may do without asking, chosen per Agent session (in its Tab).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PermissionMode {
    AskForEdits,
    AcceptEdits,
    Plan,
}

impl PermissionMode {
    /// The mode id `claude-agent-acp` uses.
    pub(crate) fn acp_id(self) -> &'static str {
        match self {
            Self::AskForEdits => "default",
            Self::AcceptEdits => "acceptEdits",
            Self::Plan => "plan",
        }
    }

    pub(crate) fn from_acp_id(id: &str) -> Option<Self> {
        [Self::AskForEdits, Self::AcceptEdits, Self::Plan]
            .into_iter()
            .find(|m| m.acp_id() == id)
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub id: SessionId,
    pub name: String,
    pub worktree: PathBuf,
    pub state: SessionState,
    pub permission_mode: PermissionMode,
    /// New transcript items (Agent messages, cards, notices) since the Tab was last shown.
    pub unread: usize,
}

/// How many transcript items a Tab gets when it's shown, and per `transcript_page_before`.
pub const TRANSCRIPT_PAGE: usize = 200;

/// A run of transcript items starting at absolute index `start`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptPage {
    pub start: usize,
    pub items: Vec<TranscriptItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TranscriptItem {
    User {
        text: String,
    },
    Agent {
        text: String,
    },
    /// Something the editor tells the user about the session (errors, declined requests).
    Notice {
        text: String,
    },
    /// A permission card: what the Agent wants to do, and the answer once given.
    Permission {
        request: PermissionRequest,
        outcome: Option<PermissionOutcome>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionRequest {
    pub tool_call_id: String,
    pub title: String,
    /// ACP's tool kind: `edit`, `execute`, `read`, …
    pub kind: Option<String>,
    /// The file or command the tool acts on.
    pub target: Option<String>,
    /// For edits, the change the Agent wants to make, shown before approval.
    pub diff: Option<Vec<DiffLine>>,
    /// The choices the Agent offers, in its order and with its names.
    pub options: Vec<PermissionOption>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionOption {
    pub id: String,
    pub name: String,
    pub kind: PermissionOptionKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PermissionOptionKind {
    AllowOnce,
    AllowAlways,
    RejectOnce,
    RejectAlways,
}

impl PermissionOptionKind {
    pub(crate) fn from_acp(kind: &str) -> Option<Self> {
        Some(match kind {
            "allow_once" => Self::AllowOnce,
            "allow_always" => Self::AllowAlways,
            "reject_once" => Self::RejectOnce,
            "reject_always" => Self::RejectAlways,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PermissionOutcome {
    #[serde(rename_all = "camelCase")]
    Selected { option_id: String },
    /// The turn ended (or the Agent went away) before an answer was given.
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DiffLineKind {
    /// An `@@ -a,b +c,d @@` hunk header.
    Hunk,
    Context,
    Added,
    Removed,
}

/// A change to a session's transcript, as streamed to the visible Tab.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TranscriptDelta {
    /// Replace everything with the latest page: items `start..` of the transcript. Sent first on
    /// every stream, and again if the stream fell behind. Indexes in other deltas are absolute.
    Reset {
        start: usize,
        items: Vec<TranscriptItem>,
    },
    ItemAdded {
        index: usize,
        item: TranscriptItem,
    },
    /// Streamed text appended to the Agent message at `index`.
    TextAppended {
        index: usize,
        text: String,
    },
    /// The item at `index` changed (e.g. a permission card got its answer).
    ItemUpdated {
        index: usize,
        item: TranscriptItem,
    },
}

/// A session's transcript, producing the delta for every change it applies.
#[derive(Default)]
pub(crate) struct Transcript {
    items: Vec<TranscriptItem>,
    /// The Agent message (the last item) still receiving chunks, if there is one.
    open_agent_message: Option<OpenAgentMessage>,
}

struct OpenAgentMessage {
    /// ACP's message id, when the adapter sends one; chunks without an id continue the open message.
    message_id: Option<String>,
}

impl OpenAgentMessage {
    fn continued_by(&self, message_id: &Option<String>) -> bool {
        match (&self.message_id, message_id) {
            (Some(open), Some(id)) => open == id,
            _ => true,
        }
    }
}

impl Transcript {
    pub(crate) fn items(&self) -> &[TranscriptItem] {
        &self.items
    }

    /// A `Reset` to the latest page.
    pub(crate) fn latest_page(&self) -> TranscriptDelta {
        let start = self.items.len().saturating_sub(TRANSCRIPT_PAGE);
        TranscriptDelta::Reset {
            start,
            items: self.items[start..].to_vec(),
        }
    }

    /// Up to a page of items just before index `before` (for scrolling back).
    pub(crate) fn page_before(&self, before: usize) -> TranscriptPage {
        let end = before.min(self.items.len());
        let start = end.saturating_sub(TRANSCRIPT_PAGE);
        TranscriptPage {
            start,
            items: self.items[start..end].to_vec(),
        }
    }

    pub(crate) fn push(&mut self, item: TranscriptItem) -> TranscriptDelta {
        self.open_agent_message = None;
        self.items.push(item.clone());
        TranscriptDelta::ItemAdded {
            index: self.items.len() - 1,
            item,
        }
    }

    pub(crate) fn append_agent_text(
        &mut self,
        text: &str,
        message_id: Option<String>,
    ) -> TranscriptDelta {
        if self
            .open_agent_message
            .as_ref()
            .is_some_and(|open| open.continued_by(&message_id))
        {
            let index = self.items.len() - 1;
            if let TranscriptItem::Agent { text: t } = &mut self.items[index] {
                t.push_str(text);
            }
            return TranscriptDelta::TextAppended {
                index,
                text: text.to_owned(),
            };
        }
        let delta = self.push(TranscriptItem::Agent {
            text: text.to_owned(),
        });
        self.open_agent_message = Some(OpenAgentMessage { message_id });
        delta
    }

    /// Records the answer on the permission card at `index`.
    pub(crate) fn resolve_permission(
        &mut self,
        index: usize,
        answer: PermissionOutcome,
    ) -> TranscriptDelta {
        if let TranscriptItem::Permission { outcome, .. } = &mut self.items[index] {
            *outcome = Some(answer);
        }
        TranscriptDelta::ItemUpdated {
            index,
            item: self.items[index].clone(),
        }
    }
}
