//! Edit notes (ticket 17): when the user saves a file by hand, each Agent session that read or
//! edited it is told what changed, with its next prompt. One diff per file, against the version the
//! Agent last saw (so several saves combine), cut down to a line when it's too big to be useful.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::diff::unified_diff;
use crate::session::{DiffLine, DiffLineKind};

/// A note's diff longer than this just says the file changed substantially.
pub(crate) const NOTE_MAX_LINES: usize = 200;

/// How the notes sent with a prompt begin and end, so a replayed conversation can find them.
pub(crate) const NOTES_START: &str = "[Edit notes: ";
pub(crate) const NOTES_END: &str = "[End of edit notes]";

/// An Edit note as the chip above the composer shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditNote {
    /// Canonical.
    pub path: PathBuf,
    /// Relative to the session's Worktree, `/`-separated.
    pub name: String,
    pub added: usize,
    pub removed: usize,
    /// None when it's over `NOTE_MAX_LINES`, or the file was also changed by someone else meanwhile:
    /// the note only says it changed substantially (read it again).
    pub diff: Option<Vec<DiffLine>>,
}

impl EditNote {
    /// "a.rs (+1 −1)": how a sent note is shown under the message it went with.
    fn tag(&self) -> String {
        format!("{} (+{} −{})", self.name, self.added, self.removed)
    }
}

#[derive(Clone)]
struct Pending {
    name: String,
    /// What the Agent last saw (as of the last note delivered, or before the first save since).
    base: String,
    current: String,
    /// The file also changed between the user's saves (another Agent, say): not all of the diff is
    /// the user's, so the note just says to read it again.
    mixed: bool,
}

/// Notes taken for a prompt: what to send, how to show them, and what to put back if the prompt
/// never reaches the Agent.
pub(crate) struct Delivery {
    pub(crate) text: String,
    pub(crate) tags: Vec<String>,
    taken: BTreeMap<PathBuf, Pending>,
}

/// One session's notes waiting for its next prompt.
#[derive(Default)]
pub(crate) struct EditNotes {
    pending: BTreeMap<PathBuf, Pending>,
}

impl EditNotes {
    /// The user saved `path`, which was `before` and is now `after` (both with `\n` line endings).
    /// Whether the notes changed.
    pub(crate) fn saved(&mut self, path: PathBuf, name: String, before: &str, after: &str) -> bool {
        let note = self.pending.entry(path.clone()).or_insert_with(|| Pending {
            name,
            base: before.to_owned(),
            current: before.to_owned(),
            mixed: false,
        });
        if note.current != before {
            note.mixed = true;
        }
        if note.current == after {
            return false;
        }
        note.current = after.to_owned();
        if note.base == note.current {
            self.pending.remove(&path); // saved back as the Agent saw it: nothing to tell
        }
        true
    }

    /// The note on `path` is dropped: the user removed it (the Agent isn't told about those
    /// changes; a later save is noted against the file as it is then), or the session read or edited
    /// the file itself, so it has seen it as it is. Whether there was one.
    pub(crate) fn remove(&mut self, path: &Path) -> bool {
        self.pending.remove(path).is_some()
    }

    pub(crate) fn notes(&self) -> Vec<EditNote> {
        self.pending
            .iter()
            .map(|(path, note)| {
                let lines = unified_diff(&note.base, &note.current, usize::MAX);
                let count = |kind: DiffLineKind| lines.iter().filter(|l| l.kind == kind).count();
                EditNote {
                    path: path.clone(),
                    name: note.name.clone(),
                    added: count(DiffLineKind::Added),
                    removed: count(DiffLineKind::Removed),
                    diff: (!note.mixed && lines.len() <= NOTE_MAX_LINES).then_some(lines),
                }
            })
            .collect()
    }

    /// The notes to send ahead of a prompt; they're delivered (the next saves are noted against
    /// what was sent) unless `put_back`. None when there are none.
    pub(crate) fn take(&mut self) -> Option<Delivery> {
        let notes = self.notes();
        if notes.is_empty() {
            return None;
        }
        let tags: Vec<String> = notes.iter().map(EditNote::tag).collect();
        let mut text = format!(
            "{NOTES_START}{}]\nSince you last saw them, the user edited these files by hand in the editor (paths relative to your working directory):\n",
            tags.join(", ")
        );
        for note in notes {
            match note.diff {
                Some(lines) => {
                    let body = to_text(&lines);
                    // A fence longer than any run of backticks in the diff (a Markdown file's own).
                    let longest = body
                        .split(|c| c != '`')
                        .map(str::len)
                        .max()
                        .unwrap_or(0);
                    let fence = "`".repeat(longest.max(2) + 1);
                    text.push_str(&format!(
                        "\n{} (+{} -{}):\n{fence}diff\n{body}{fence}\n",
                        note.name, note.added, note.removed
                    ));
                }
                None => text.push_str(&format!(
                    "\n{} changed substantially (+{} -{}); read it again before relying on what you saw.\n",
                    note.name, note.added, note.removed
                )),
            }
        }
        text.push_str(NOTES_END);
        Some(Delivery {
            text,
            tags,
            taken: std::mem::take(&mut self.pending),
        })
    }

    /// The prompt never reached the Agent: its notes are pending again (with any saves since).
    pub(crate) fn put_back(&mut self, delivery: Delivery) {
        for (path, taken) in delivery.taken {
            match self.pending.get_mut(&path) {
                // Saved again since: diffed from what the Agent really last saw.
                Some(newer) => {
                    newer.mixed |= taken.mixed || newer.base != taken.current;
                    newer.base = taken.base;
                }
                None => {
                    self.pending.insert(path, taken);
                }
            }
        }
        self.pending.retain(|_, note| note.base != note.current);
    }
}

/// The diff as plain unified text (ASCII `-`/`+`), for the Agent.
fn to_text(lines: &[DiffLine]) -> String {
    let mut text = String::new();
    for line in lines {
        text.push_str(match line.kind {
            DiffLineKind::Hunk => "",
            DiffLineKind::Context => " ",
            DiffLineKind::Added => "+",
            DiffLineKind::Removed => "-",
        });
        text.push_str(&line.text);
        text.push('\n');
    }
    text
}

/// A user message as replayed (`session/load`), split into the notes that went with it (their
/// tags) and what the user wrote. None when it carried no notes (or they haven't all arrived yet).
pub(crate) fn split_sent(text: &str) -> Option<(Vec<String>, String)> {
    let rest = text.strip_prefix(NOTES_START)?;
    let (summary, _) = rest.split_once("]\n")?;
    let (_, after) = text.split_once(NOTES_END)?;
    let tags = summary.split(", ").map(str::to_owned).collect();
    Some((tags, after.trim_start_matches('\n').to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn several_saves_make_one_diff_against_what_the_agent_saw() {
        let mut notes = EditNotes::default();
        let a = PathBuf::from("a.rs");
        notes.saved(a.clone(), "a.rs".into(), "one\ntwo\n", "one\n2\n");
        notes.saved(a.clone(), "a.rs".into(), "one\n2\n", "1\n2\n");
        let [note] = &notes.notes()[..] else {
            panic!("one note")
        };
        assert_eq!((note.added, note.removed), (2, 2));

        let sent = notes.take().unwrap();
        assert!(
            sent.text.contains("-two\n") && sent.text.contains("+1\n"),
            "{}",
            sent.text
        );
        assert_eq!(sent.tags, ["a.rs (+2 −2)"]);
        assert!(notes.notes().is_empty());
        // Next time, against what was sent.
        notes.saved(a.clone(), "a.rs".into(), "1\n2\n", "1\n2\n3\n");
        assert_eq!(notes.notes()[0].added, 1);
        assert_eq!(notes.notes()[0].removed, 0);
    }

    #[test]
    fn saved_back_as_it_was_there_is_nothing_to_tell() {
        let mut notes = EditNotes::default();
        notes.saved("a.rs".into(), "a.rs".into(), "x\n", "y\n");
        notes.saved("a.rs".into(), "a.rs".into(), "y\n", "x\n");
        assert!(notes.take().is_none());
    }

    #[test]
    fn a_big_change_only_says_so() {
        let mut notes = EditNotes::default();
        let after: String = (0..300).map(|i| format!("line {i}\n")).collect();
        notes.saved("a.rs".into(), "a.rs".into(), "", &after);
        assert_eq!(notes.notes()[0].diff, None);
        assert!(notes
            .take()
            .unwrap()
            .text
            .contains("changed substantially (+300 -0)"));
    }

    #[test]
    fn a_change_by_someone_else_between_saves_isnt_put_on_the_user() {
        let mut notes = EditNotes::default();
        notes.saved("a.rs".into(), "a.rs".into(), "v0\n", "v1\n");
        // Someone else wrote v2; the user saves v3 over it.
        notes.saved("a.rs".into(), "a.rs".into(), "v2\n", "v3\n");
        assert_eq!(notes.notes()[0].diff, None);
        assert!(notes.take().unwrap().text.contains("read it again"));
    }

    #[test]
    fn notes_put_back_combine_with_saves_since() {
        let mut notes = EditNotes::default();
        notes.saved("a.rs".into(), "a.rs".into(), "v0\n", "v1\n");
        let sent = notes.take().unwrap();
        notes.saved("a.rs".into(), "a.rs".into(), "v1\n", "v2\n");
        notes.put_back(sent);
        let note = &notes.notes()[0];
        let lines = note.diff.as_ref().unwrap();
        assert!(lines.iter().any(|l| l.text == "v0") && lines.iter().any(|l| l.text == "v2"));
    }

    #[test]
    fn a_fence_outlasts_the_backticks_in_the_diff() {
        let mut notes = EditNotes::default();
        notes.saved("a.md".into(), "a.md".into(), "", "```rust\nx\n```\n");
        let text = notes.take().unwrap().text;
        assert!(text.contains("````diff\n"), "{text}");
    }

    #[test]
    fn sent_notes_are_found_again_in_a_replayed_message() {
        let mut notes = EditNotes::default();
        notes.saved("a.rs".into(), "a.rs".into(), "x\n", "y\n");
        let sent = notes.take().unwrap();
        let replayed = format!("{}carry on", sent.text);
        let (tags, text) = split_sent(&replayed).unwrap();
        assert_eq!((tags, text.as_str()), (sent.tags, "carry on"));
        assert_eq!(split_sent("just a message"), None);
    }
}
