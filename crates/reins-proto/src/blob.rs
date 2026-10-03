//! Large files, moved through the server for one operation while the phone stays in control.
//!
//! The phone still runs every tool. When a tool needs a file too large for a tool call (an upload) or would return
//! one (a download), the phone asks the server to hold that one file for a short time:
//!
//! - **Upload**: the phone opens a slot ([`BlobSlotRequest`]) and answers the AI with its `upload_url` and what to do
//!   next. The AI sends the bytes there (`PUT`, no other authentication: the URL is the capability). The server keeps
//!   them, works out a [`BlobPreview`] and tells the phone. A slot opened for a tool ([`BlobPurpose::ToolInput`]) is
//!   approved together with the tool call that names it (`blob=<id>`), with the preview shown; a slot opened by
//!   `reins_upload` ([`BlobPurpose::Upload`]) is approved on its own when the upload arrives, and only then can it
//!   be downloaded or used.
//! - **Use**: the phone has the server send the bytes onward ([`BlobSend`]: the server streams them to the URL the
//!   phone names, with the headers the phone gives for this one request), or reads them itself
//!   (`GET /reins/api/blobs/<id>/content`) when it must transform them (vault encryption).
//! - **Download**: the phone has the server fetch a large result ([`BlobFetch`]), or uploads one it made itself
//!   (`PUT /reins/api/blobs/<id>/content`), and answers the AI with a short-lived `download_url`.
//!
//! Every file is deleted when its operation is done, and in any case when it expires ([`MAX_BLOB_TTL_SECS`]).

use serde::{Deserialize, Serialize};

use crate::ids::{ConnectionId, RequestId};

/// Results up to this size are answered inline as before; larger ones become a download link.
pub const INLINE_LIMIT: u64 = 256 * 1024;
/// Largest file the server holds.
pub const MAX_BLOB_BYTES: u64 = 1 << 30;
/// Longest a file stays on the server.
pub const MAX_BLOB_TTL_SECS: u32 = 60 * 60;
/// Default lifetime of a slot or a download link.
pub const DEFAULT_BLOB_TTL_SECS: u32 = 30 * 60;
/// Longest text preview kept.
pub const MAX_PREVIEW_TEXT: usize = 4_000;
/// Images up to this size are shown whole as their preview.
pub const MAX_PREVIEW_IMAGE: usize = 512 * 1024;
/// Largest response body a [`BlobSend`] hands back to the phone.
pub const MAX_SEND_RESPONSE: usize = 256 * 1024;

/// A blob id (random, opaque to the AI).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BlobId(pub String);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BlobPurpose {
    /// The file a tool call needs (`tool` is the MCP tool name); approved with that call.
    ToolInput {
        tool: String,
    },
    /// `reins_upload`: a file the AI wants to pass on as a link; approved when it arrives.
    Upload {
        /// What the AI said it is for, shown to the user.
        reason: String,
    },
    /// A result made available to the AI.
    Output,
}

/// `POST /reins/api/blobs`: the phone opens a slot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlobSlotRequest {
    pub v: u32,
    /// The AI connection the file belongs to: only its calls may name the blob.
    pub connection_id: ConnectionId,
    /// The request that asked for it, when one did.
    #[serde(default)]
    pub request_id: Option<RequestId>,
    /// File name as the AI gave it (shown, never used as a path).
    pub name: String,
    #[serde(default)]
    pub content_type: Option<String>,
    /// Upload limit, at most [`MAX_BLOB_BYTES`].
    pub max_bytes: u64,
    pub purpose: BlobPurpose,
    /// At most [`MAX_BLOB_TTL_SECS`].
    pub ttl_secs: u32,
}

/// The answer to [`BlobSlotRequest`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlobSlot {
    pub id: BlobId,
    /// `PUT` the bytes here (`curl -T file URL`). Single use.
    pub upload_url: String,
    /// For [`BlobPurpose::Upload`]: where the file can be downloaded once the user approved it.
    #[serde(default)]
    pub download_url: Option<String>,
    /// Unix seconds.
    pub expires_at: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlobState {
    /// The slot waits for the upload.
    Waiting,
    /// The bytes arrived; nobody decided yet.
    Uploaded,
    Approved,
    Denied,
}

/// What the user sees of a file before deciding.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BlobPreview {
    /// Valid UTF-8 text: its start.
    Text {
        head: String,
        truncated: bool,
    },
    /// An image (PNG, JPEG, GIF, WebP): base64 of the whole file when at most [`MAX_PREVIEW_IMAGE`], else none.
    Image {
        mime: String,
        #[serde(default)]
        data_base64: Option<String>,
    },
    /// Anything else: the type guessed from its first bytes ("PDF document", "ZIP archive", "data").
    Binary {
        description: String,
    },
    /// Not uploaded yet.
    None,
}

/// `GET /reins/api/blobs/<id>` and what the pending list carries for uploads awaiting a decision.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlobInfo {
    pub v: u32,
    pub id: BlobId,
    pub connection_id: ConnectionId,
    /// Label of the AI connection.
    pub connection_label: String,
    #[serde(default)]
    pub request_id: Option<RequestId>,
    pub name: String,
    pub purpose: BlobPurpose,
    pub state: BlobState,
    /// Bytes received (0 while waiting).
    pub size: u64,
    /// Lowercase hex, empty while waiting.
    pub sha256: String,
    /// Sniffed from the bytes, not taken from the uploader.
    pub content_type: String,
    pub preview: BlobPreview,
    /// Unix seconds.
    pub created_at: i64,
    pub expires_at: i64,
}

/// `POST /reins/api/blobs/<id>/decision`: the user's answer for a [`BlobPurpose::Upload`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlobDecision {
    pub v: u32,
    pub approved: bool,
}

/// `POST /reins/api/blobs/<id>/send`: the server streams the blob to `url` (https only) with these headers, for
/// this one request, and forgets them. The phone deletes the blob afterwards (or it expires).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlobSend {
    pub v: u32,
    /// `PUT` or `POST`.
    pub method: String,
    pub url: String,
    /// Sent as given (`Authorization` included), never stored or logged.
    pub headers: Vec<(String, String)>,
    /// How the bytes travel: as the body, or base64 inside a JSON body (GitHub's contents API).
    #[serde(default)]
    pub body: SendBody,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SendBody {
    /// The blob is the body.
    #[default]
    Raw,
    /// `json` with the blob, base64-encoded, put at top-level key `field`; sent as `application/json`.
    JsonBase64 {
        json: serde_json::Map<String, serde_json::Value>,
        field: String,
    },
}

/// The answer to [`BlobSend`]: what the destination replied.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlobSendResult {
    pub status: u16,
    /// `content-type` and `location` only.
    pub headers: Vec<(String, String)>,
    /// The response body as text, at most [`MAX_SEND_RESPONSE`] bytes.
    pub body: String,
    pub truncated: bool,
}

/// `POST /reins/api/blobs/fetch`: the server downloads `url` (https only, redirects to other hosts followed
/// without the headers) into a new blob the AI may download.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlobFetch {
    pub v: u32,
    pub connection_id: ConnectionId,
    #[serde(default)]
    pub request_id: Option<RequestId>,
    pub url: String,
    pub headers: Vec<(String, String)>,
    /// The name the download gets.
    pub name: String,
    pub max_bytes: u64,
    pub ttl_secs: u32,
}

/// `PUT /reins/api/blobs/output`'s query and `POST /reins/api/blobs/fetch`'s answer: a blob ready to download.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlobDownload {
    pub id: BlobId,
    /// Single-purpose link for the AI: `GET` the file. Works until `expires_at`.
    pub download_url: String,
    pub name: String,
    pub size: u64,
    pub sha256: String,
    pub content_type: String,
    pub expires_at: i64,
}

/// What a tool answers the AI when the file must be uploaded first (a [`BlobSlot`] for a tool input).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UploadInstructions {
    /// Always "upload_required".
    pub status: String,
    pub blob: BlobId,
    pub upload_url: String,
    pub max_bytes: u64,
    pub expires_at: i64,
    /// Plain words for the AI: how to upload (`curl -T <file> '<upload_url>'`) and what to call next.
    pub next: String,
}

/// Lowercase-hex check for ids and digests received from the network.
#[must_use]
pub fn is_blob_id(s: &str) -> bool {
    (16..=64).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn purposes_and_previews_have_stable_wire_forms() {
        assert_eq!(
            serde_json::to_value(BlobPurpose::ToolInput {
                tool: "github_release_asset_upload".into()
            })
            .unwrap(),
            json!({"kind": "tool_input", "tool": "github_release_asset_upload"})
        );
        assert_eq!(serde_json::to_value(BlobPurpose::Output).unwrap(), json!({"kind": "output"}));
        assert_eq!(
            serde_json::to_value(BlobPreview::Text {
                head: "hi".into(),
                truncated: false
            })
            .unwrap(),
            json!({"kind": "text", "head": "hi", "truncated": false})
        );
        assert_eq!(serde_json::to_value(BlobState::Uploaded).unwrap(), json!("uploaded"));
    }

    #[test]
    fn blob_ids_are_plain_tokens() {
        assert!(is_blob_id("aB3_xY9-0123456789"));
        assert!(!is_blob_id("short"));
        assert!(!is_blob_id("../../../etc/passwd-and-more"));
    }
}
