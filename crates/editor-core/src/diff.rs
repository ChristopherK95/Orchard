//! Unified diffs: for permission cards (the edit an Agent asks to make) and the Manual editor's
//! diff view (its text against the file on disk).

use similar::{ChangeTag, TextDiff};

use crate::session::{DiffLine, DiffLineKind};

/// The unified diff from `old` to `new`, three lines of context, cut short after `max_lines`.
pub(crate) fn unified_diff(old: &str, new: &str, max_lines: usize) -> Vec<DiffLine> {
    let diff = TextDiff::from_lines(old, new);
    let mut lines = vec![];
    for hunk in diff.unified_diff().context_radius(3).iter_hunks() {
        lines.push(DiffLine {
            kind: DiffLineKind::Hunk,
            text: hunk.header().to_string().trim_end().to_owned(),
        });
        for change in hunk.iter_changes() {
            let kind = match change.tag() {
                ChangeTag::Equal => DiffLineKind::Context,
                ChangeTag::Delete => DiffLineKind::Removed,
                ChangeTag::Insert => DiffLineKind::Added,
            };
            lines.push(DiffLine {
                kind,
                text: change.value().trim_end_matches(['\n', '\r']).to_owned(),
            });
        }
    }
    if lines.len() > max_lines {
        let hidden = lines.len() - max_lines;
        lines.truncate(max_lines);
        lines.push(DiffLine {
            kind: DiffLineKind::Hunk,
            text: format!("… {hidden} more lines"),
        });
    }
    lines
}
