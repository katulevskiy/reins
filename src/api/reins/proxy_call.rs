//! `POST /reins/api/mcp/call` (files spec, S4): one MCP request made by the server for the phone, for a tool whose
//! results are too large to pass through the phone. The server sends the JSON-RPC request with the phone's headers
//! (same SSRF rules as every outbound request), reads a JSON or SSE answer, and keeps every large content item as an
//! output blob, replaced by a `resource_link` to its download, so the phone gets back a small result.

use data_encoding::BASE64;
use reins_proto::{
    blob::{BlobDownload, INLINE_LIMIT},
    remote_mcp::{ProxyCall, ProxyCallResult},
};
use rocket::{Data, Route, http::Status, serde::json::Json};
use serde_json::{Value, json};

use super::{
    blob_routes::{admit_outbound, outbound_err, owner_of, store_output},
    device_api::{
        DeviceKey, PhoneResult, api_err, bad_request, parse_versioned, read_body_limited, require_approval_device,
    },
    outbound::{self, BytesBody},
    sniff::{self, valid_file_name},
};
use crate::{auth::Headers, db::DbConn};

/// The request: MCP arguments are bounded at 512 KiB, plus headers.
const MAX_BODY: u64 = 2 * 1024 * 1024;
/// Largest answer read from the MCP server (it is parsed as JSON, so it is held in memory).
pub const MAX_RESPONSE_BYTES: usize = 64 * 1024 * 1024;
const MAX_SESSION_ID: usize = 256;

pub fn routes() -> Vec<Route> {
    routes![post_call]
}

/// Server-sent events, fed in arbitrary chunks; yields the `data` of each complete event.
#[derive(Debug, Default)]
pub struct SseParser {
    line: Vec<u8>,
    data: String,
    has_data: bool,
}

impl SseParser {
    pub fn push(&mut self, chunk: &[u8]) -> Vec<String> {
        let mut events = Vec::new();
        for &byte in chunk {
            if byte == b'\n' {
                let mut line = std::mem::take(&mut self.line);
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                self.line_done(&line, &mut events);
            } else {
                self.line.push(byte);
            }
        }
        events
    }

    /// The last event when the stream ends without a blank line.
    pub fn finish(&mut self) -> Option<String> {
        let mut events = Vec::new();
        let line = std::mem::take(&mut self.line);
        if !line.is_empty() {
            self.line_done(&line, &mut events);
        }
        self.line_done(&[], &mut events);
        events.pop()
    }

    fn line_done(&mut self, line: &[u8], events: &mut Vec<String>) {
        if line.is_empty() {
            if self.has_data {
                events.push(std::mem::take(&mut self.data));
                self.has_data = false;
            }
            return;
        }
        if line.starts_with(b":") {
            return;
        }
        let (field, value) = match line.iter().position(|b| *b == b':') {
            Some(at) => (&line[..at], line[at + 1..].strip_prefix(b" ").unwrap_or(&line[at + 1..])),
            None => (line, &[][..]),
        };
        if field == b"data" {
            if self.has_data {
                self.data.push('\n');
            }
            self.data.push_str(&String::from_utf8_lossy(value));
            self.has_data = true;
        }
    }
}

/// The JSON-RPC response for `id` among SSE event data.
fn matching_response(event: &str, id: &Value) -> Option<Value> {
    let value: Value = serde_json::from_str(event).ok()?;
    let obj = value.as_object()?;
    (obj.get("id") == Some(id) && (obj.contains_key("result") || obj.contains_key("error"))).then_some(value)
}

/// A content item too large to hand back inline, as bytes to store.
#[derive(Debug, PartialEq, Eq)]
pub struct Offload {
    pub index: usize,
    pub bytes: Vec<u8>,
    pub name: String,
    pub mime: String,
}

fn extension(mime: &str) -> &'static str {
    match mime.split(';').next().unwrap_or_default().trim() {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "audio/mpeg" => "mp3",
        "audio/wav" | "audio/x-wav" => "wav",
        "application/pdf" => "pdf",
        "application/json" => "json",
        m if m.starts_with("text/") => "txt",
        _ => "bin",
    }
}

fn str_of<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

/// The file name a resource's URI suggests, if it is a usable one.
fn name_from_uri(uri: &str) -> Option<String> {
    let path = uri.split(['?', '#']).next()?;
    let last = path.rsplit('/').next()?;
    valid_file_name(last).ok().filter(|n| n.contains('.'))
}

/// What to store for one content item, if it is larger than [`INLINE_LIMIT`].
fn offload_item(index: usize, item: &Value) -> Option<Offload> {
    if serde_json::to_vec(item).map_or(0, |v| v.len()) as u64 <= INLINE_LIMIT {
        return None;
    }
    let kind = str_of(item, "type")?;
    let n = index + 1;
    let (bytes, mime, name) = match kind {
        "image" | "audio" => {
            let bytes = BASE64.decode(str_of(item, "data")?.as_bytes()).ok()?;
            let mime = str_of(item, "mimeType").map_or_else(|| sniff::sniff(&bytes).content_type, str::to_owned);
            let name = format!("{kind}-{n}.{}", extension(&mime));
            (bytes, mime, name)
        }
        "text" => (str_of(item, "text")?.as_bytes().to_vec(), sniff::TEXT_TYPE.to_owned(), format!("text-{n}.txt")),
        "resource" => {
            let resource = item.get("resource")?;
            let (bytes, default_mime) = if let Some(text) = str_of(resource, "text") {
                (text.as_bytes().to_vec(), sniff::TEXT_TYPE)
            } else {
                (BASE64.decode(str_of(resource, "blob")?.as_bytes()).ok()?, sniff::BINARY_TYPE)
            };
            let mime = str_of(resource, "mimeType").unwrap_or(default_mime).to_owned();
            let name = str_of(resource, "uri")
                .and_then(name_from_uri)
                .unwrap_or_else(|| format!("resource-{n}.{}", extension(&mime)));
            (bytes, mime, name)
        }
        _ => return None,
    };
    Some(Offload {
        index,
        bytes,
        name,
        mime,
    })
}

/// The large items of a JSON-RPC response's `result.content`.
pub fn plan_offloads(response: &Value) -> Vec<Offload> {
    let Some(items) = response.pointer("/result/content").and_then(Value::as_array) else {
        return Vec::new();
    };
    items.iter().enumerate().filter_map(|(i, item)| offload_item(i, item)).collect()
}

/// Puts the link to a stored item where the item was.
pub fn replace_with_link(response: &mut Value, offload: &Offload, download: &BlobDownload) {
    if let Some(slot) = response.pointer_mut(&format!("/result/content/{}", offload.index)) {
        *slot = json!({
            "type": "resource_link",
            "uri": download.download_url,
            "name": offload.name,
            "mimeType": offload.mime,
            "size": download.size
        });
    }
}

/// The request must be one JSON-RPC request with an id the answer can be matched to.
fn request_id(request: &Value) -> Result<Value, String> {
    let ok = request.get("jsonrpc").and_then(Value::as_str) == Some("2.0")
        && request.get("method").is_some_and(Value::is_string);
    match request.get("id") {
        Some(id) if ok && (id.is_string() || id.is_number()) => Ok(id.clone()),
        _ => Err("`request` must be one JSON-RPC 2.0 request with an id".to_owned()),
    }
}

/// Reads an SSE answer until the response to `id` arrives.
async fn read_sse(mut response: reqwest::Response, id: &Value) -> Result<Option<Value>, String> {
    let mut parser = SseParser::default();
    let mut read = 0usize;
    while let Some(chunk) = response.chunk().await.map_err(|_| "The MCP server's answer broke off.".to_owned())? {
        read += chunk.len();
        if read > MAX_RESPONSE_BYTES {
            return Err("The MCP server's answer is too large.".to_owned());
        }
        if let Some(found) = parser.push(&chunk).iter().find_map(|event| matching_response(event, id)) {
            return Ok(Some(found));
        }
    }
    Ok(parser.finish().and_then(|event| matching_response(&event, id)))
}

#[post("/reins/api/mcp/call", data = "<data>")]
async fn post_call(
    data: Data<'_>,
    headers: Headers,
    key: DeviceKey,
    conn: DbConn,
) -> PhoneResult<Json<ProxyCallResult>> {
    require_approval_device(&headers, &key, &conn).await?;
    let call: ProxyCall = parse_versioned(&read_body_limited(data, MAX_BODY).await?)?;
    let owner = owner_of(&headers, &call.connection_id, None, &conn).await?;
    drop(conn);
    let id = request_id(&call.request).map_err(bad_request)?;
    let phone_headers = outbound::header_map(&call.headers, &["content-type", "accept"]).map_err(outbound_err)?;
    // Each call may hold up to `MAX_RESPONSE_BYTES` in memory: the account's concurrent calls are bounded.
    let _running = admit_outbound(&owner.user)?;
    let mut fixed = reqwest::header::HeaderMap::new();
    fixed.insert(reqwest::header::CONTENT_TYPE, reqwest::header::HeaderValue::from_static("application/json"));
    fixed.insert(
        reqwest::header::ACCEPT,
        reqwest::header::HeaderValue::from_static("application/json, text/event-stream"),
    );
    let body = BytesBody(serde_json::to_vec(&call.request).unwrap_or_default().into());
    let response = outbound::execute(reqwest::Method::POST, &call.endpoint, &phone_headers, &fixed, &body, false)
        .await
        .map_err(outbound_err)?;
    let status = response.status().as_u16();
    let session_id = response
        .headers()
        .get("mcp-session-id")
        .and_then(|v| v.to_str().ok())
        .filter(|v| !v.is_empty() && v.len() <= MAX_SESSION_ID && v.bytes().all(|b| b.is_ascii_graphic()))
        .map(str::to_owned);
    let sse = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.trim_start().to_ascii_lowercase().starts_with("text/event-stream"));
    let gateway = |m: &str| api_err(Status::BadGateway, reins_proto::device::codes::BAD_REQUEST, m);
    let mut answer = if sse {
        read_sse(response, &id).await.map_err(|m| gateway(&m))?
    } else {
        let (bytes, truncated) = outbound::read_limited(response, MAX_RESPONSE_BYTES).await.map_err(outbound_err)?;
        if truncated {
            return Err(gateway("The MCP server's answer is too large."));
        }
        serde_json::from_slice(&bytes).ok()
    };
    let mut downloads = Vec::new();
    if let Some(answer) = answer.as_mut() {
        for offload in plan_offloads(answer) {
            let download = store_output(&owner, &offload.name, &offload.bytes, 0).await?;
            replace_with_link(answer, &offload, &download);
            downloads.push(download);
        }
    }
    Ok(Json(ProxyCallResult {
        status,
        response: answer,
        session_id,
        downloads,
    }))
}

#[cfg(test)]
mod tests {
    use reins_proto::blob::BlobId;

    use super::*;

    #[test]
    fn sse_events_are_assembled_across_chunks() {
        let stream = b": comment\r\nevent: message\r\ndata: {\"a\":\r\ndata: 1}\r\n\r\nid: 7\ndata: second\n\n";
        for cut in 0..stream.len() {
            let mut parser = SseParser::default();
            let mut events = parser.push(&stream[..cut]);
            events.extend(parser.push(&stream[cut..]));
            assert_eq!(events, ["{\"a\":\n1}", "second"], "cut at {cut}");
        }
        let mut unterminated = SseParser::default();
        assert!(unterminated.push(b"data: last").is_empty());
        assert_eq!(unterminated.finish().as_deref(), Some("last"));
        assert_eq!(SseParser::default().finish(), None);
    }

    #[test]
    fn only_the_response_to_our_request_counts() {
        let id = json!(7);
        assert!(matching_response(r#"{"jsonrpc":"2.0","method":"notifications/progress"}"#, &id).is_none());
        assert!(matching_response(r#"{"jsonrpc":"2.0","id":8,"result":{}}"#, &id).is_none());
        assert!(matching_response(r#"{"jsonrpc":"2.0","id":7,"method":"sampling/createMessage"}"#, &id).is_none());
        assert!(matching_response(r#"{"jsonrpc":"2.0","id":7,"result":{}}"#, &id).is_some());
        assert!(matching_response(r#"{"jsonrpc":"2.0","id":7,"error":{"code":1}}"#, &id).is_some());
        assert!(matching_response("not json", &id).is_none());
    }

    #[test]
    fn requests_need_an_id() {
        assert_eq!(request_id(&json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call"})), Ok(json!(3)));
        assert_eq!(request_id(&json!({"jsonrpc": "2.0", "id": "x", "method": "tools/call"})), Ok(json!("x")));
        assert!(request_id(&json!({"jsonrpc": "2.0", "method": "tools/call"})).is_err());
        assert!(request_id(&json!({"jsonrpc": "2.0", "id": null, "method": "tools/call"})).is_err());
        assert!(request_id(&json!({"id": 1, "method": "tools/call"})).is_err());
        assert!(request_id(&json!([1])).is_err());
    }

    #[test]
    fn large_items_become_links_and_small_ones_stay() {
        let image = vec![0x89u8; usize::try_from(INLINE_LIMIT).unwrap()];
        let long_text = "t".repeat(usize::try_from(INLINE_LIMIT).unwrap() + 1);
        let mut response = json!({"jsonrpc": "2.0", "id": 1, "result": {"content": [
            {"type": "text", "text": "small"},
            {"type": "image", "data": BASE64.encode(&image), "mimeType": "image/png"},
            {"type": "text", "text": long_text},
            {"type": "resource", "resource": {"uri": "file:///tmp/report.csv?x=1", "mimeType": "text/csv",
                "text": long_text}},
            {"type": "resource", "resource": {"uri": "mem://x", "blob": BASE64.encode(&image)}},
            {"type": "image", "data": "not base64!!", "mimeType": "image/png", "pad": long_text}
        ]}});
        let plan = plan_offloads(&response);
        let summary: Vec<(usize, &str, &str, usize)> =
            plan.iter().map(|o| (o.index, o.name.as_str(), o.mime.as_str(), o.bytes.len())).collect();
        let n = usize::try_from(INLINE_LIMIT).unwrap();
        assert_eq!(
            summary,
            [
                (1, "image-2.png", "image/png", n),
                (2, "text-3.txt", sniff::TEXT_TYPE, n + 1),
                (3, "report.csv", "text/csv", n + 1),
                (4, "resource-5.bin", sniff::BINARY_TYPE, n),
            ]
        );
        let download = BlobDownload {
            id: BlobId("b".repeat(22)),
            download_url: "https://rw.example/reins/blob/s".into(),
            name: "image-2.png".into(),
            size: 10,
            sha256: String::new(),
            content_type: "image/png".into(),
            expires_at: 0,
        };
        replace_with_link(&mut response, &plan[0], &download);
        assert_eq!(
            response["result"]["content"][1],
            json!({"type": "resource_link", "uri": "https://rw.example/reins/blob/s", "name": "image-2.png",
                "mimeType": "image/png", "size": 10})
        );
        assert_eq!(response["result"]["content"][0]["text"], "small");
        assert!(plan_offloads(&json!({"error": {"code": 1}})).is_empty());
    }
}
