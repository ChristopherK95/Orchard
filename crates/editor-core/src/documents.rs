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
