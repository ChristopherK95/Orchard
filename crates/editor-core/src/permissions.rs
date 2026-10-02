//! Turns an ACP permission request (plus what the tool call announced earlier) into the card the
//! user answers.

use std::path::Path;

use serde_json::{json, Value};

use crate::diff::unified_diff;
use crate::session::{
    PermissionOption, PermissionOptionKind, PermissionOutcome, PermissionRequest, ToolCallStatus,
    TranscriptItem,
};

/// Diffs longer than this are cut short on the card; the full change is still what gets applied.
const MAX_DIFF_LINES: usize = 400;

/// `tool_call` is the tool call as announced so far, merged with the request's `toolCall`. Paths
/// inside `worktree` are shown relative to it.
pub(crate) fn request_from(
    tool_call: &Value,
    options: &Value,
    worktree: &Path,
) -> PermissionRequest {
    let diffs: Vec<&Value> = tool_call["content"]
        .as_array()
        .map(|content| content.iter().filter(|c| c["type"] == "diff").collect())
        .unwrap_or_default();
    let target = target_of(tool_call, worktree);
    let diff = (!diffs.is_empty()).then(|| {
        diffs
            .iter()
            .flat_map(|d| {
                unified_diff(
                    d["oldText"].as_str().unwrap_or_default(),
                    d["newText"].as_str().unwrap_or_default(),
                    MAX_DIFF_LINES,
                )
            })
            .collect()
    });
    PermissionRequest {
        tool_call_id: tool_call["toolCallId"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        title: tool_call["title"]
            .as_str()
            .unwrap_or("A tool call")
            .to_owned(),
        kind: tool_call["kind"].as_str().map(str::to_owned),
        target,
        file: file_of(tool_call, worktree),
        diff,
        options: options
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|o| {
                Some(PermissionOption {
                    id: o["optionId"].as_str()?.to_owned(),
                    name: o["name"].as_str().unwrap_or_default().to_owned(),
                    kind: PermissionOptionKind::from_acp(o["kind"].as_str()?)?,
                })
            })
            .collect(),
    }
}

/// The transcript row for a tool call as known so far.
pub(crate) fn tool_call_row(tool_call: &Value, worktree: &Path) -> TranscriptItem {
    TranscriptItem::ToolCall {
        tool_call_id: tool_call["toolCallId"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        title: tool_call["title"]
            .as_str()
            .unwrap_or("A tool call")
            .to_owned(),
        tool_kind: tool_call["kind"].as_str().map(str::to_owned),
        target: target_of(tool_call, worktree),
        status: ToolCallStatus::from_acp(tool_call["status"].as_str().unwrap_or("pending")),
    }
}

/// The file or command a tool call acts on; paths inside `worktree` are shown relative to it.
fn target_of(tool_call: &Value, worktree: &Path) -> Option<String> {
    tool_call["content"]
        .as_array()
        .and_then(|content| content.iter().find(|c| c["type"] == "diff"))
        .and_then(|d| d["path"].as_str())
        .or_else(|| tool_call["locations"][0]["path"].as_str())
        .or_else(|| tool_call["rawInput"]["command"].as_str())
        .or_else(|| tool_call["rawInput"]["file_path"].as_str())
        .map(|target| match Path::new(target).strip_prefix(worktree) {
            Ok(relative) => relative.display().to_string(),
            Err(_) => target.to_owned(),
        })
}

/// The file an edit (or a delete or move) changes, canonical when it exists: its diff's, or its
/// first location's (relative ones taken as inside `worktree`). None for other tools: reading a
/// file with unsaved changes is nothing to warn about.
fn file_of(tool_call: &Value, worktree: &Path) -> Option<std::path::PathBuf> {
    let has_diff = tool_call["content"]
        .as_array()
        .is_some_and(|content| content.iter().any(|c| c["type"] == "diff"));
    let changes_files = matches!(tool_call["kind"].as_str(), Some("edit" | "delete" | "move"));
    if !has_diff && !changes_files {
        return None;
    }
    let path = tool_call["content"]
        .as_array()
        .and_then(|content| content.iter().find(|c| c["type"] == "diff"))
        .and_then(|d| d["path"].as_str())
        .or_else(|| tool_call["locations"][0]["path"].as_str())
        .or_else(|| tool_call["rawInput"]["file_path"].as_str())?;
    Some(crate::worktrees::normalize(worktree.join(path)))
}

/// The files a tool call reads or edits (canonical when they exist): its diffs', its locations',
/// and the `file_path`/`path` it was given. Only for tools that read or change files; a search's
/// locations are hits, not files the Agent has seen whole.
pub(crate) fn files_of(tool_call: &Value, worktree: &Path) -> Vec<std::path::PathBuf> {
    let content = tool_call["content"].as_array();
    let has_diff = content.is_some_and(|content| content.iter().any(|c| c["type"] == "diff"));
    let reads_or_changes = matches!(
        tool_call["kind"].as_str(),
        Some("read" | "edit" | "delete" | "move")
    );
    if !has_diff && !reads_or_changes {
        return vec![];
    }
    let diffs = content
        .into_iter()
        .flatten()
        .filter(|c| c["type"] == "diff")
        .filter_map(|d| d["path"].as_str());
    let locations = tool_call["locations"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|l| l["path"].as_str());
    let given = [
        &tool_call["rawInput"]["file_path"],
        &tool_call["rawInput"]["path"],
    ]
    .into_iter()
    .filter_map(|p| p.as_str());
    diffs
        .chain(locations)
        .chain(given)
        .map(|p| crate::worktrees::normalize(worktree.join(p)))
        .collect()
}

/// The `RequestPermissionResponse` for an answer.
pub(crate) fn acp_outcome(outcome: &PermissionOutcome) -> Value {
    match outcome {
        PermissionOutcome::Selected { option_id } => {
            json!({ "outcome": { "outcome": "selected", "optionId": option_id } })
        }
        PermissionOutcome::Cancelled => json!({ "outcome": { "outcome": "cancelled" } }),
    }
}

/// Shallow-merges an ACP tool call update into what's known about the tool call so far.
pub(crate) fn merge_tool_call(known: &mut Value, update: &Value) {
    let (Some(known), Some(update)) = (known.as_object_mut(), update.as_object()) else {
        return;
    };
    for (key, value) in update {
        if key != "sessionUpdate" && !value.is_null() {
            known.insert(key.clone(), value.clone());
        }
    }
}
