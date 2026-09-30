//! Agent session vocabulary shared with the frontend (see `CONTEXT.md`).

use std::path::PathBuf;

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct SessionId(pub u64);

/// The Agent session states this slice supports. Needs you and Suspended arrive in later tickets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SessionState {
    /// The Agent is mid-turn.
    Working,
    /// The turn has finished and the Agent is waiting for the next prompt.
    Idle,
    /// The Agent's process ended unexpectedly.
    Exited,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub id: SessionId,
    pub name: String,
    pub worktree: PathBuf,
    pub state: SessionState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TranscriptItem {
    User { text: String },
    Agent { text: String },
    /// Something the editor tells the user about the session (errors, declined requests).
    Notice { text: String },
}

/// A change to a session's transcript, as streamed to the visible Tab.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TranscriptDelta {
    /// Replace everything: sent first on every stream, and again if the stream fell behind.
    Reset { items: Vec<TranscriptItem> },
    ItemAdded { index: usize, item: TranscriptItem },
    /// Streamed text appended to the Agent message at `index`.
    TextAppended { index: usize, text: String },
}

/// A session's transcript, producing the delta for every change it applies.
#[derive(Default)]
pub(crate) struct Transcript {
    items: Vec<TranscriptItem>,
    /// The ACP message id of the Agent message still receiving chunks, if one is open.
    open_agent_message: Option<Option<String>>,
}

impl Transcript {
    pub(crate) fn items(&self) -> &[TranscriptItem] {
        &self.items
    }

    pub(crate) fn push(&mut self, item: TranscriptItem) -> TranscriptDelta {
        self.open_agent_message = None;
        self.items.push(item.clone());
        TranscriptDelta::ItemAdded { index: self.items.len() - 1, item }
    }

    pub(crate) fn append_agent_text(&mut self, text: &str, message_id: Option<String>) -> TranscriptDelta {
        let continues = match (&self.open_agent_message, &message_id) {
            (Some(Some(open)), Some(id)) => open == id,
            (Some(_), _) => true,
            (None, _) => false,
        };
        if continues {
            let index = self.items.len() - 1;
            if let TranscriptItem::Agent { text: t } = &mut self.items[index] {
                t.push_str(text);
            }
            return TranscriptDelta::TextAppended { index, text: text.to_owned() };
        }
        let delta = self.push(TranscriptItem::Agent { text: text.to_owned() });
        self.open_agent_message = Some(message_id);
        delta
    }
}
