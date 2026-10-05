//! Files and images attached to a prompt (ticket 39): checked against what the Agent says it takes
//! in a prompt (its `promptCapabilities`), and sent as ACP content blocks with the message.

use std::path::{Path, PathBuf};

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// The largest image the Agent is sent (Claude's own limit is 5 MB an image).
const MAX_IMAGE_BYTES: usize = 5 * 1024 * 1024;
/// The largest text file: more than that is better read by the Agent itself.
const MAX_TEXT_BYTES: usize = 1024 * 1024;
/// Larger files aren't read at all.
pub(crate) const MAX_BYTES: usize = MAX_IMAGE_BYTES;

/// Why a file over `MAX_BYTES` can't be attached.
pub(crate) fn too_big(name: &str) -> String {
    format!("{name}: files can be up to 5 MB (mention its path instead, and the Agent reads it)")
}

/// A file or image that goes with the next prompt, as the composer holds it until it's sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Attachment {
    /// Sent as an image block.
    #[serde(rename_all = "camelCase")]
    Image {
        name: String,
        mime_type: String,
        /// The image, base64-encoded.
        data: String,
    },
    /// Sent as an embedded resource (the file's text).
    #[serde(rename_all = "camelCase")]
    Text {
        name: String,
        /// Where it came from, if it's a file (a pasted one has no path).
        path: Option<PathBuf>,
        text: String,
    },
}

impl Attachment {
    /// Makes an attachment of a file's contents, or says why it can't be one. `capabilities` is
    /// the Agent's `promptCapabilities`.
    pub(crate) fn from_bytes(
        name: &str,
        path: Option<&Path>,
        mime_type: Option<&str>,
        bytes: Vec<u8>,
        capabilities: &Value,
    ) -> Result<Self, String> {
        let mime_type = mime_type
            .filter(|m| !m.is_empty())
            .map(str::to_owned)
            .or_else(|| image_type(name).map(str::to_owned));
        if let Some(mime_type) = mime_type.filter(|m| m.starts_with("image/")) {
            if !image_type_supported(&mime_type) {
                return Err(format!(
                    "{name}: the Agent takes PNG, JPEG, GIF and WebP images, not {mime_type}"
                ));
            }
            if capabilities["image"] != true {
                return Err(format!("{name}: the Agent doesn't take images in a prompt"));
            }
            if bytes.len() > MAX_IMAGE_BYTES {
                return Err(format!("{name}: images can be up to 5 MB"));
            }
            return Ok(Self::Image {
                name: name.to_owned(),
                mime_type,
                data: base64::engine::general_purpose::STANDARD.encode(bytes),
            });
        }
        let text = match String::from_utf8(bytes) {
            Ok(text) if !text.contains('\0') => text,
            _ => {
                return Err(format!(
                    "{name}: only images and text files can be attached"
                ))
            }
        };
        if capabilities["embeddedContext"] != true {
            return Err(format!("{name}: the Agent doesn't take files in a prompt"));
        }
        if text.len() > MAX_TEXT_BYTES {
            return Err(format!(
                "{name}: text files can be up to 1 MB (mention its path instead, and the Agent reads it)"
            ));
        }
        Ok(Self::Text {
            name: name.to_owned(),
            path: path.map(Path::to_path_buf),
            text,
        })
    }

    pub fn name(&self) -> &str {
        match self {
            Self::Image { name, .. } | Self::Text { name, .. } => name,
        }
    }

    /// The ACP content block it's sent as.
    pub(crate) fn content_block(&self) -> Value {
        match self {
            Self::Image {
                mime_type, data, ..
            } => json!({ "type": "image", "mimeType": mime_type, "data": data }),
            Self::Text { name, path, text } => {
                let uri = match path {
                    Some(path) => file_uri(path),
                    None => name.clone(),
                };
                json!({
                    "type": "resource",
                    "resource": { "uri": uri, "text": text }
                })
            }
        }
    }
}

/// The image type a file name's extension says it is.
fn image_type(name: &str) -> Option<&'static str> {
    let ext = Path::new(name).extension()?.to_str()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "tif" | "tiff" => "image/tiff",
        "ico" => "image/x-icon",
        _ => return None,
    })
}

/// The image types Claude takes.
fn image_type_supported(mime_type: &str) -> bool {
    matches!(
        mime_type,
        "image/png" | "image/jpeg" | "image/gif" | "image/webp"
    )
}

/// A `file://` URI for an absolute path (forward slashes, so Windows paths read as URIs do).
fn file_uri(path: &Path) -> String {
    let path = path.to_string_lossy().replace('\\', "/");
    if path.starts_with('/') {
        format!("file://{path}")
    } else {
        format!("file:///{path}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all() -> Value {
        json!({ "image": true, "embeddedContext": true })
    }

    #[test]
    fn images_are_told_by_their_extension_or_type() {
        let png = Attachment::from_bytes("shot.PNG", None, None, vec![1, 2, 3], &all()).unwrap();
        assert!(matches!(png, Attachment::Image { ref mime_type, .. } if mime_type == "image/png"));
        let pasted =
            Attachment::from_bytes("image", None, Some("image/jpeg"), vec![1], &all()).unwrap();
        assert!(matches!(pasted, Attachment::Image { .. }));
        assert_eq!(
            pasted.content_block(),
            json!({ "type": "image", "mimeType": "image/jpeg", "data": "AQ==" })
        );
    }

    #[test]
    fn text_files_go_as_resources() {
        let path = Path::new("C:\\repo\\notes.md");
        let text =
            Attachment::from_bytes("notes.md", Some(path), None, b"hi".to_vec(), &all()).unwrap();
        assert_eq!(
            text.content_block(),
            json!({ "type": "resource", "resource": { "uri": "file:///C:/repo/notes.md", "text": "hi" } })
        );
    }

    #[test]
    fn what_the_agent_doesnt_take_is_refused_with_a_reason() {
        let none = json!({});
        let err = Attachment::from_bytes("a.png", None, None, vec![1], &none).unwrap_err();
        assert!(err.contains("doesn't take images"), "{err}");
        let err = Attachment::from_bytes("a.txt", None, None, b"x".to_vec(), &none).unwrap_err();
        assert!(err.contains("doesn't take files"), "{err}");
        let err = Attachment::from_bytes("a.bin", None, None, vec![0, 159], &all()).unwrap_err();
        assert!(err.contains("only images and text"), "{err}");
        let err =
            Attachment::from_bytes("a.svg", None, None, b"<svg/>".to_vec(), &all()).unwrap_err();
        assert!(err.contains("not image/svg+xml"), "{err}");
    }
}
