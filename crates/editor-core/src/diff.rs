//! Unified diffs: for permission cards (the edit an Agent asks to make) and the Manual editor's
//! diff view (its text against the file on disk), and git's own for a Code review.

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
    cut_short(lines, max_lines)
}

/// The lines of `git diff`'s output for one file (its hunks; the file header and "\ No newline at
/// end of file" left out), cut short after `max_lines`.
pub(crate) fn parse_git_diff(out: &str, max_lines: usize) -> Vec<DiffLine> {
    let mut lines = vec![];
    let mut in_hunks = false;
    for line in out.lines() {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.starts_with("@@ ") {
            in_hunks = true;
            lines.push(DiffLine {
                kind: DiffLineKind::Hunk,
                text: line.to_owned(),
            });
            continue;
        }
        if !in_hunks {
            continue;
        }
        let (kind, text) = match line.split_at_checked(1) {
            Some((" ", text)) => (DiffLineKind::Context, text),
            Some(("+", text)) => (DiffLineKind::Added, text),
            Some(("-", text)) => (DiffLineKind::Removed, text),
            // (An empty context line can lose its space on the way.)
            None => (DiffLineKind::Context, ""),
            _ => continue,
        };
        lines.push(DiffLine {
            kind,
            text: text.to_owned(),
        });
    }
    cut_short(lines, max_lines)
}

/// `lines` with all past `max_lines` replaced by a line saying how many there were.
fn cut_short(mut lines: Vec<DiffLine>, max_lines: usize) -> Vec<DiffLine> {
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
