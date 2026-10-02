//! Files for the Manual editor (ticket 14): read with a version, saved only over that version.
//! Big files open read-only, binaries as a placeholder, minified ones soft-wrapped. The editor
//! always works in `\n` text: the file's own line ending is noted on read and put back on save.

use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Bigger than this opens read-only, without highlighting.
const READ_ONLY_OVER: u64 = 5 * 1024 * 1024;
/// Bigger than this isn't opened at all (it would only freeze the window).
const TOO_BIG_OVER: u64 = 64 * 1024 * 1024;
/// A line longer than this in a file of few lines is minified code: soft-wrap it.
const MINIFIED_LINE: usize = 2000;

/// A file as the Manual editor opens it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenedFile {
    /// Canonical: the same file always has the same path.
    pub path: PathBuf,
    pub content: FileContent,
    /// The version on disk that was read; saving sends it back.
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum FileContent {
    #[serde(rename_all = "camelCase")]
    Text {
        /// With `\n` line endings, whatever the file has.
        text: String,
        /// The file's line ending, put back on save: `"\r\n"` or `"\n"` (for a file with both,
        /// the more common one).
        line_ending: String,
        /// The file has both kinds of line ending; saving makes them all `line_ending`.
        mixed_line_endings: bool,
        /// Too big to edit (and to highlight).
        read_only: bool,
        /// Minified: one huge line, to soft-wrap.
        minified: bool,
    },
    /// Not text: shown as a placeholder.
    Binary { bytes: u64 },
    /// Text, but not in UTF-8 (say, Windows-1252): shown as a placeholder rather than garbled.
    NotUtf8 { bytes: u64 },
    /// Too big to open.
    TooBig { bytes: u64 },
}

/// What a save may write over.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SaveOver {
    /// Only the version the editor read (or last saved); anything else is refused.
    Version { version: String },
    /// Whatever is on disk now: the user chose to overwrite a newer file.
    Anything,
}

/// The version of a file's bytes (`"missing"` when there's no file).
pub(crate) fn version_of(bytes: Option<&[u8]>) -> String {
    match bytes {
        None => "missing".into(),
        Some(bytes) => {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            bytes.hash(&mut hasher);
            format!("{:x}-{:016x}", bytes.len(), hasher.finish())
        }
    }
}

pub(crate) fn read(path: &Path) -> Result<OpenedFile, String> {
    let size = std::fs::metadata(path)
        .map_err(|e| format!("couldn't open {}: {e}", path.display()))?
        .len();
    if size > TOO_BIG_OVER {
        return Ok(OpenedFile {
            path: path.to_owned(),
            content: FileContent::TooBig { bytes: size },
            version: format!("size-{size}"),
        });
    }
    let bytes =
        std::fs::read(path).map_err(|e| format!("couldn't read {}: {e}", path.display()))?;
    let version = version_of(Some(&bytes));
    let content = if bytes.iter().take(8192).any(|b| *b == 0) {
        FileContent::Binary { bytes: size }
    } else {
        match String::from_utf8(bytes) {
            Err(_) => FileContent::NotUtf8 { bytes: size },
            Ok(text) => {
                let crlf = text.matches("\r\n").count();
                let lf = text.matches('\n').count() - crlf;
                let line_ending = if crlf > lf { "\r\n" } else { "\n" };
                let text = if crlf > 0 {
                    text.replace("\r\n", "\n")
                } else {
                    text
                };
                let minified =
                    text.lines().count() <= 3 && text.lines().any(|l| l.len() > MINIFIED_LINE);
                FileContent::Text {
                    read_only: size > READ_ONLY_OVER,
                    minified,
                    line_ending: line_ending.into(),
                    mixed_line_endings: crlf > 0 && lf > 0,
                    text,
                }
            }
        }
    };
    Ok(OpenedFile {
        path: path.to_owned(),
        content,
        version,
    })
}

/// Why a save didn't happen.
pub(crate) enum SaveError {
    /// The file on disk isn't the version the editor read (an Agent changed it, say).
    Changed,
    Io(String),
}

/// Writes `text` (with `\n` line endings, written as `line_ending`) if `over` allows; the new
/// version. The write replaces the file in one step (a temporary file renamed over it), so an Agent
/// or the file watcher never sees it half-written, and a failed save leaves it as it was. (A write
/// by someone else between the check and the rename can still be lost: the window is tiny.)
pub(crate) fn save(
    path: &Path,
    text: &str,
    line_ending: &str,
    over: &SaveOver,
) -> Result<String, SaveError> {
    if let SaveOver::Version { version } = over {
        let on_disk = std::fs::read(path).ok();
        if version_of(on_disk.as_deref()) != *version {
            return Err(SaveError::Changed);
        }
    }
    let bytes = if line_ending == "\r\n" {
        text.replace("\r\n", "\n").replace('\n', "\r\n")
    } else {
        text.to_owned()
    };
    let io = |e: std::io::Error| SaveError::Io(format!("couldn't save {}: {e}", path.display()));
    let name = path
        .file_name()
        .ok_or_else(|| io(std::io::ErrorKind::InvalidInput.into()))?;
    let partial = path.with_file_name(format!(
        ".{}.{}.saving",
        name.to_string_lossy(),
        std::process::id()
    ));
    std::fs::write(&partial, &bytes).map_err(io)?;
    if let Ok(meta) = std::fs::metadata(path) {
        let _ = std::fs::set_permissions(&partial, meta.permissions());
    }
    if let Err(err) = std::fs::rename(&partial, path) {
        let _ = std::fs::remove_file(&partial);
        // (Replacing can fail while another program holds the file open: write it in place.)
        std::fs::write(path, &bytes).map_err(|_| io(err))?;
    }
    Ok(version_of(Some(bytes.as_bytes())))
}

/// A file open in a Manual editor (the pane, or a popped-out window), as the core tracks it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenDocument {
    /// Canonical.
    pub path: PathBuf,
    /// The label of the window it's open in (`main`, or a popped-out editor's).
    pub window: String,
    /// It has changes not saved yet.
    pub dirty: bool,
    /// The version on disk it was read at (or last saved as).
    pub version: String,
}

/// A file popped out of the pane into a window of its own (ticket 15): what it needs to carry on
/// there. Undo history doesn't come along.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PoppedOutFile {
    pub path: PathBuf,
    /// The text as edited, and as last saved (so the new window knows what's unsaved).
    pub text: String,
    pub saved_text: String,
    /// The selection's cursor (head) and anchor, as CodeMirror offsets into `text` (UTF-16 code
    /// units, not bytes).
    pub cursor: usize,
    pub anchor: usize,
    pub version: String,
    pub line_ending: String,
    /// Soft-wrapped (say, a minified file).
    pub wrap: bool,
}

impl PoppedOutFile {
    fn dirty(&self) -> bool {
        self.text != self.saved_text
    }
}

/// A popped-out window's file, held until that window is gone: so a reload of the window collects
/// it again, and a window closed before it ever collected it gives it back.
struct PopOut {
    file: PoppedOutFile,
    /// The window it came from (where it goes back to).
    from: String,
    collected: bool,
}

/// Which files are open in which Manual editor, and with unsaved changes: what ticket 16 decides
/// reload-or-banner by, and ticket 17 finds the sessions to tell about a save by.
#[derive(Default)]
pub(crate) struct DocumentTracker {
    docs: Vec<OpenDocument>,
    /// By the popped-out window's label.
    pop_outs: std::collections::HashMap<String, PopOut>,
    next_window: u64,
}

impl DocumentTracker {
    pub(crate) fn all(&self) -> Vec<OpenDocument> {
        self.docs.clone()
    }

    pub(crate) fn opened(&mut self, path: PathBuf, window: &str, version: &str) {
        self.docs
            .retain(|d| !(d.path == path && d.window == window));
        self.docs.push(OpenDocument {
            path,
            window: window.to_owned(),
            dirty: false,
            version: version.to_owned(),
        });
    }

    fn entry(&mut self, path: &Path, window: &str) -> Option<&mut OpenDocument> {
        self.docs
            .iter_mut()
            .find(|d| d.path == path && d.window == window)
    }

    pub(crate) fn changed(&mut self, path: &Path, window: &str, dirty: bool) {
        if let Some(doc) = self.entry(path, window) {
            doc.dirty = dirty;
        }
    }

    /// Saved from `window` as `version` (still dirty if typed into meanwhile: `changed` says so).
    pub(crate) fn saved(&mut self, path: &Path, window: &str, version: &str) {
        if let Some(doc) = self.entry(path, window) {
            doc.version = version.to_owned();
        }
    }

    pub(crate) fn closed(&mut self, path: &Path, window: &str) {
        self.docs
            .retain(|d| !(d.path == path && d.window == window));
    }

    /// The windows `path` is open in.
    pub(crate) fn windows_with(&self, path: &Path) -> Vec<String> {
        self.docs
            .iter()
            .filter(|d| d.path == path)
            .map(|d| d.window.clone())
            .collect()
    }

    /// Moves `file` from `from` to a new window; that window's label. The file is tracked there at
    /// once (with its unsaved changes), even before the window has loaded.
    pub(crate) fn pop_out(&mut self, from: &str, file: PoppedOutFile) -> String {
        self.next_window += 1;
        let window = format!("editor-{}", self.next_window);
        self.closed(&file.path, from);
        self.docs.push(OpenDocument {
            path: file.path.clone(),
            window: window.clone(),
            dirty: file.dirty(),
            version: file.version.clone(),
        });
        self.pop_outs.insert(
            window.clone(),
            PopOut {
                file,
                from: from.to_owned(),
                collected: false,
            },
        );
        window
    }

    /// The popped-out `window` collects its file (again, after a reload: the latest copy).
    pub(crate) fn collect(&mut self, window: &str) -> Option<PoppedOutFile> {
        let pop_out = self.pop_outs.get_mut(window)?;
        pop_out.collected = true;
        let file = pop_out.file.clone();
        if self.entry(&file.path, window).is_none() {
            self.docs.push(OpenDocument {
                path: file.path.clone(),
                window: window.to_owned(),
                dirty: file.dirty(),
                version: file.version.clone(),
            });
        }
        Some(file)
    }

    /// The popped-out `window`'s file as it is now, so a reload of the window loses nothing.
    pub(crate) fn update_pop_out(
        &mut self,
        window: &str,
        text: String,
        saved_text: String,
        version: String,
    ) {
        if let Some(pop_out) = self.pop_outs.get_mut(window) {
            pop_out.file.text = text;
            pop_out.file.saved_text = saved_text;
            pop_out.file.version = version;
        }
    }

    /// The window couldn't be opened: the file is back where it came from.
    pub(crate) fn cancel_pop_out(&mut self, window: &str) {
        if let Some(pop_out) = self.pop_outs.remove(window) {
            self.window_gone(window);
            let file = pop_out.file;
            self.docs.push(OpenDocument {
                dirty: file.dirty(),
                version: file.version,
                path: file.path,
                window: pop_out.from,
            });
        }
    }

    /// `window`'s page is (re)loading: what it had open is gone from it (a popped-out window
    /// collects its file again).
    pub(crate) fn page_loading(&mut self, window: &str) {
        self.window_gone(window);
    }

    /// `window` closed. A popped-out file it never collected, with unsaved changes, comes back
    /// (for the window it came from to reopen) rather than being lost.
    pub(crate) fn window_closed(&mut self, window: &str) -> Option<(String, PoppedOutFile)> {
        self.window_gone(window);
        let pop_out = self.pop_outs.remove(window)?;
        (!pop_out.collected && pop_out.file.dirty()).then_some((pop_out.from, pop_out.file))
    }

    fn window_gone(&mut self, window: &str) {
        self.docs.retain(|d| d.window != window);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, text: &str, saved: &str) -> PoppedOutFile {
        PoppedOutFile {
            path: path.into(),
            text: text.into(),
            saved_text: saved.into(),
            cursor: 0,
            anchor: 0,
            version: "v1".into(),
            line_ending: "\n".into(),
            wrap: false,
        }
    }

    #[test]
    fn the_same_file_popped_out_twice_is_tracked_in_both_windows() {
        let mut docs = DocumentTracker::default();
        docs.opened("a.rs".into(), "main", "v1");
        let first = docs.pop_out("main", file("a.rs", "edited", "orig"));
        docs.closed(Path::new("a.rs"), "main"); // (the pane closes its tab: a no-op by now)
        docs.opened("a.rs".into(), "main", "v1");
        let second = docs.pop_out("main", file("a.rs", "other", "orig"));
        assert!(docs.collect(&first).is_some());
        assert!(docs.collect(&second).is_some());
        let mut windows = docs.windows_with(Path::new("a.rs"));
        windows.sort();
        assert_eq!(windows, [first, second]);
        assert!(docs.all().iter().all(|d| d.dirty));
    }

    #[test]
    fn a_reloaded_pop_out_collects_its_latest_text_again() {
        let mut docs = DocumentTracker::default();
        let window = docs.pop_out("main", file("a.rs", "one", "orig"));
        docs.collect(&window).unwrap();
        docs.update_pop_out(&window, "two".into(), "orig".into(), "v1".into());
        docs.page_loading(&window);
        assert!(docs.all().is_empty(), "nothing open while it reloads");
        assert_eq!(docs.collect(&window).unwrap().text, "two");
        assert_eq!(docs.windows_with(Path::new("a.rs")), [window]);
    }

    #[test]
    fn a_window_closed_before_it_collected_its_file_gives_it_back() {
        let mut docs = DocumentTracker::default();
        let window = docs.pop_out("main", file("a.rs", "unsaved", "orig"));
        let (back_to, returned) = docs.window_closed(&window).expect("given back");
        assert_eq!(
            (back_to.as_str(), returned.text.as_str()),
            ("main", "unsaved")
        );

        // Collected (and then closed, which asked first): nothing comes back.
        let window = docs.pop_out("main", file("a.rs", "unsaved", "orig"));
        docs.collect(&window);
        assert!(docs.window_closed(&window).is_none());
        assert!(docs.all().is_empty());
    }

    #[test]
    fn a_window_that_couldnt_open_leaves_the_file_where_it_was() {
        let mut docs = DocumentTracker::default();
        docs.opened("a.rs".into(), "main", "v1");
        let window = docs.pop_out("main", file("a.rs", "unsaved", "orig"));
        docs.cancel_pop_out(&window);
        assert_eq!(docs.windows_with(Path::new("a.rs")), ["main"]);
        assert!(docs.collect(&window).is_none());
    }
}
