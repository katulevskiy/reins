//! The subset of the Gmail REST v1 JSON the core reads.

use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
pub struct ListResponse {
    #[serde(default)]
    pub messages: Vec<MessageRef>,
}

#[derive(Debug, Deserialize)]
pub struct MessageRef {
    pub id: String,
}

#[derive(Debug, Deserialize)]
pub struct GmailMessage {
    pub id: String,
    #[serde(rename = "threadId", default)]
    pub thread_id: String,
    #[serde(rename = "labelIds", default)]
    pub label_ids: Vec<String>,
    #[serde(default)]
    pub snippet: String,
    /// Milliseconds since the epoch, as a decimal string.
    #[serde(rename = "internalDate", default)]
    pub internal_date: Option<String>,
    #[serde(default)]
    pub payload: Option<Part>,
}

#[derive(Debug, Default, Deserialize)]
pub struct Part {
    /// "0", "1", "1.2": stable for a message, unlike `attachmentId`, which Gmail makes anew for every fetch.
    #[serde(rename = "partId", default)]
    pub part_id: String,
    #[serde(rename = "mimeType", default)]
    pub mime_type: String,
    #[serde(default)]
    pub filename: String,
    #[serde(default)]
    pub headers: Vec<Header>,
    #[serde(default)]
    pub body: Option<PartBody>,
    #[serde(default)]
    pub parts: Vec<Part>,
}

#[derive(Debug, Deserialize)]
pub struct Header {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Default, Deserialize)]
pub struct PartBody {
    /// base64url, possibly unpadded.
    #[serde(default)]
    pub data: Option<String>,
    #[serde(rename = "attachmentId", default)]
    pub attachment_id: Option<String>,
    /// Bytes, as Gmail counts them.
    #[serde(default)]
    pub size: u64,
}

#[derive(Debug, Deserialize)]
pub struct SendResponse {
    pub id: String,
    #[serde(rename = "threadId", default)]
    pub thread_id: String,
}
