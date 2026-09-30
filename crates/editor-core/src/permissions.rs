//! Turns an ACP permission request (plus what the tool call announced earlier) into the card the
//! user answers.

use std::path::Path;

use serde_json::Value;
use similar::{ChangeTag, TextDiff};

use crate::session::{DiffLine, DiffLineKind, PermissionOption, PermissionOptionKind, PermissionRequest};

/// Diffs longer than this are cut short on the card; the full change is still what gets applied.
const MAX_DIFF_LINES: usize = 400;

/// `tool_call` is the tool call as announced so far, merged with the request's `toolCall`. Paths
/// inside `worktree` are shown relative to it.
pub(crate) fn request_from(tool_call: &Value, options: &Value, worktree: &Path) -> PermissionRequest {
    let diffs: Vec<&Value> = tool_call["content"]
        .as_array()
        .map(|content| content.iter().filter(|c| c["type"] == "diff").collect())
        .unwrap_or_default();
    let target = diffs
        .first()
        .and_then(|d| d["path"].as_str())
        .or_else(|| tool_call["locations"][0]["path"].as_str())
        .or_else(|| tool_call["rawInput"]["command"].as_str())
        .or_else(|| tool_call["rawInput"]["file_path"].as_str())
        .map(|target| match Path::new(target).strip_prefix(worktree) {
            Ok(relative) => relative.display().to_string(),
            Err(_) => target.to_owned(),
        });
    let diff = (!diffs.is_empty()).then(|| {
        diffs
            .iter()
            .flat_map(|d| unified_diff(d["oldText"].as_str().unwrap_or_default(), d["newText"].as_str().unwrap_or_default()))
            .collect()
    });
    PermissionRequest {
        tool_call_id: tool_call["toolCallId"].as_str().unwrap_or_default().to_owned(),
        title: tool_call["title"].as_str().unwrap_or("A tool call").to_owned(),
        kind: tool_call["kind"].as_str().map(str::to_owned),
        target,
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

/// Shallow-merges an ACP tool call update into what's known about the tool call so far.
pub(crate) fn merge_tool_call(known: &mut Value, update: &Value) {
    let (Some(known), Some(update)) = (known.as_object_mut(), update.as_object()) else { return };
    for (key, value) in update {
        if key != "sessionUpdate" && !value.is_null() {
            known.insert(key.clone(), value.clone());
        }
    }
}

fn unified_diff(old: &str, new: &str) -> Vec<DiffLine> {
    let diff = TextDiff::from_lines(old, new);
    let mut lines = vec![];
    for hunk in diff.unified_diff().context_radius(3).iter_hunks() {
        lines.push(DiffLine { kind: DiffLineKind::Hunk, text: hunk.header().to_string().trim_end().to_owned() });
        for change in hunk.iter_changes() {
            let kind = match change.tag() {
                ChangeTag::Equal => DiffLineKind::Context,
                ChangeTag::Delete => DiffLineKind::Removed,
                ChangeTag::Insert => DiffLineKind::Added,
            };
            lines.push(DiffLine { kind, text: change.value().trim_end_matches(['\n', '\r']).to_owned() });
        }
    }
    if lines.len() > MAX_DIFF_LINES {
        let hidden = lines.len() - MAX_DIFF_LINES;
        lines.truncate(MAX_DIFF_LINES);
        lines.push(DiffLine { kind: DiffLineKind::Hunk, text: format!("… {hidden} more lines") });
    }
    lines
}
