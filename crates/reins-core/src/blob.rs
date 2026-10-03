//! Large files, moved through the Reins server for one operation while the phone stays in control (see
//! [`reins_proto::blob`]).
//!
//! - **Inputs.** A tool that takes a file (`github_release_asset_upload`, `github_file_put`, `vault_attachment_add`,
//!   `vault_send_create`) called without its content gets [`UploadInstructions`] back: the phone opened a slot for that
//!   tool. Called again with `blob=<id>`, the file is checked (this connection's, uploaded, unexpired, for this tool),
//!   shown with the preview, and used when the write runs: GitHub gets it from the server ([`send`]), the vault
//!   encrypts it on the phone ([`read`]). It is deleted once used.
//! - **Uploads.** `reins_upload` opens a slot of its own and answers at once; when the file arrives the user
//!   decides on it ([`crate::types::PendingKind::Blob`]), and only then can it be downloaded.
//! - **Large results.** Connectors mark an item whose content is too large to hand over inline (a marker in its
//!   `extra`, see [`fetch_marker`] and [`output_marker`]); when the item is released — never before — the server
//!   fetches it in the phone's name, or receives what the phone decrypted, and the AI gets a download link.
//!
//! Connectors reach the server through the request's [`Scope`], set by the engine around everything it asks a
//! connector to do; without one (a connector used on its own) files are handled inline as before.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use data_encoding::BASE64;
use reins_proto::PROTOCOL_VERSION;
use reins_proto::blob::{
    BlobDecision, BlobDownload, BlobFetch, BlobInfo, BlobPreview, BlobPurpose, BlobSend, BlobSendResult, BlobSlot,
    BlobSlotRequest, BlobState, DEFAULT_BLOB_TTL_SECS, INLINE_LIMIT, MAX_BLOB_BYTES, UploadInstructions, is_blob_id,
};
use reins_proto::connector::ConnectorCall;
use reins_proto::ids::ConnectionId;
use reins_proto::relay::{RelayOutcome, RelayRequest, ToolResult};
use reqwest::{Method, StatusCode};
use serde_json::{Map, Value, json};
use zeroize::Zeroizing;

use crate::connector::{Connector, Item, Preview, flow};
use crate::engine::Engine;
use crate::http::error_text;
use crate::phone_api::{ApiFailure, PhoneApi, check_id};
use crate::session::{Session, api_call};
use crate::store::{AuditInfo, AuditRecord, unix_now};
use crate::types::{BlobView, PendingItem, PendingKind};
use crate::views::ParkedRequest;
use crate::{CoreError, text};

/// The activity's service for uploads (`reins_upload`).
pub const SERVICE_FILES: &str = "files";
/// The most a file the phone reads itself may be (to encrypt it for the vault).
pub const MAX_READ_BYTES: u64 = 100 * 1024 * 1024;
/// The largest file GitHub's contents API takes.
const MAX_CONTENTS_BYTES: u64 = 100 * 1024 * 1024;
/// Key of the marker in `Item::extra` that asks for an item's content to be handed over as a link.
pub const DELIVER_KEY: &str = "_reins_deliver";
/// Moving a large file takes longer than an API call.
const TRANSFER_TIMEOUT: Duration = Duration::from_mins(15);
/// How much of a text file the write preview shows.
const PREVIEW_HEAD: usize = 300;

// ---- the server API -------------------------------------------------------------------------------------------------

fn failure(status: StatusCode, body: &str) -> ApiFailure {
    if status == StatusCode::UNAUTHORIZED {
        return ApiFailure::Unauthorized;
    }
    let (code, message) = error_text(status, body);
    ApiFailure::Status {
        status: status.as_u16(),
        code,
        message,
    }
}

fn check_blob(id: &str) -> Result<(), ApiFailure> {
    if is_blob_id(id) {
        Ok(())
    } else {
        Err(CoreError::invalid("malformed file id").into())
    }
}

/// The blob endpoints of the phone API (`/reins/api/blobs…`).
impl PhoneApi<'_> {
    /// `POST /blobs`: opens a slot.
    pub async fn blob_slot(&self, request: &BlobSlotRequest) -> Result<BlobSlot, ApiFailure> {
        let slot: BlobSlot = Self::json(self.request(Method::POST, "/blobs").json(request)).await?;
        check_blob(&slot.id.0)?;
        Ok(slot)
    }

    /// `GET /blobs/<id>`.
    pub async fn blob_info(&self, id: &str) -> Result<BlobInfo, ApiFailure> {
        check_blob(id)?;
        let info: BlobInfo = Self::json(self.request(Method::GET, &format!("/blobs/{id}"))).await?;
        if info.id.0 != id {
            return Err(CoreError::invalid("the server returned a different file").into());
        }
        Ok(info)
    }

    /// `POST /blobs/<id>/decision`: the user's answer on an upload.
    pub async fn blob_decision(&self, id: &str, decision: &BlobDecision) -> Result<(), ApiFailure> {
        check_blob(id)?;
        Self::send(self.request(Method::POST, &format!("/blobs/{id}/decision")).json(decision)).await.map(drop)
    }

    /// `GET /blobs/<id>/content`: the bytes, at most `max`.
    pub async fn blob_content(&self, id: &str, max: u64) -> Result<Zeroizing<Vec<u8>>, ApiFailure> {
        check_blob(id)?;
        let mut resp =
            self.request(Method::GET, &format!("/blobs/{id}/content")).timeout(TRANSFER_TIMEOUT).send().await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(failure(status, &body));
        }
        let too_large = || ApiFailure::Core(CoreError::service(format!("The file is larger than {}.", size_text(max))));
        if resp.content_length().is_some_and(|n| n > max) {
            return Err(too_large());
        }
        let mut out = Zeroizing::new(Vec::new());
        while let Some(chunk) = resp.chunk().await? {
            if (out.len() + chunk.len()) as u64 > max {
                return Err(too_large());
            }
            out.extend_from_slice(&chunk);
        }
        Ok(out)
    }

    /// `PUT /blobs/output`: a result the phone made, for the AI to download.
    pub async fn blob_output(
        &self,
        connection: &ConnectionId,
        name: &str,
        ttl_secs: u32,
        bytes: Vec<u8>,
    ) -> Result<BlobDownload, ApiFailure> {
        check_id(&connection.0)?;
        let builder = self
            .request(Method::PUT, "/blobs/output")
            .query(&[("connection_id", connection.0.as_str()), ("name", name), ("ttl_secs", &ttl_secs.to_string())])
            .header("Content-Type", "application/octet-stream")
            .timeout(TRANSFER_TIMEOUT)
            .body(bytes);
        let download: BlobDownload = Self::json(builder).await?;
        check_blob(&download.id.0)?;
        Ok(download)
    }

    /// `POST /blobs/<id>/send`: the server streams the file to `send.url`.
    pub async fn blob_send(&self, id: &str, send: &BlobSend) -> Result<BlobSendResult, ApiFailure> {
        check_blob(id)?;
        Self::json(self.request(Method::POST, &format!("/blobs/{id}/send")).json(send).timeout(TRANSFER_TIMEOUT)).await
    }

    /// `POST /blobs/fetch`: the server downloads a large result.
    pub async fn blob_fetch(&self, fetch: &BlobFetch) -> Result<BlobDownload, ApiFailure> {
        let download: BlobDownload =
            Self::json(self.request(Method::POST, "/blobs/fetch").json(fetch).timeout(TRANSFER_TIMEOUT)).await?;
        check_blob(&download.id.0)?;
        Ok(download)
    }

    /// `DELETE /blobs/<id>`.
    pub async fn blob_delete(&self, id: &str) -> Result<(), ApiFailure> {
        check_blob(id)?;
        Self::send(self.request(Method::DELETE, &format!("/blobs/{id}"))).await.map(drop)
    }
}

/// What a connector is told when the server no longer has a file.
fn gone(e: ApiFailure) -> CoreError {
    match e {
        ApiFailure::Status {
            status: 404 | 410,
            ..
        } => CoreError::service(
            "The uploaded file is gone (it expired or was already used). Call the tool again without `blob` for a new \
             upload link.",
        ),
        other => other.into_core(),
    }
}

// ---- the request's scope --------------------------------------------------------------------------------------------

tokio::task_local! {
    static SCOPE: Scope;
}

/// The request a connector is working for: how it reaches the server's file endpoints.
#[derive(Clone)]
pub struct Scope {
    session: Arc<Session>,
}

/// Runs `work` with `scope` as the current request's (or as it is, without one).
pub(crate) async fn within<F: Future>(scope: Option<Scope>, work: F) -> F::Output {
    match scope {
        Some(s) => SCOPE.scope(s, work).await,
        None => work.await,
    }
}

fn current() -> Result<Scope, CoreError> {
    SCOPE.try_with(Clone::clone).map_err(|_| CoreError::service("Files can only be moved for a request from an AI."))
}

/// Whether large results can be handed over as links (the engine runs the connector for a request).
pub fn linking() -> bool {
    SCOPE.try_with(|_| ()).is_ok()
}

/// Has the server send an uploaded file onward (GitHub), then deletes it when the destination accepted it.
pub(crate) async fn send(id: &str, send: &BlobSend) -> Result<BlobSendResult, CoreError> {
    let scope = current()?;
    let result = api_call!(&scope.session, |api| api.blob_send(id, send)).map_err(gone)?;
    if (200..300).contains(&result.status) {
        discard(&scope.session, id).await;
    }
    Ok(result)
}

/// The bytes of an uploaded file, at most `max` (to transform them on the phone).
pub(crate) async fn read(id: &str, max: u64) -> Result<Zeroizing<Vec<u8>>, CoreError> {
    let scope = current()?;
    api_call!(&scope.session, |api| api.blob_content(id, max)).map_err(gone)
}

/// The uploaded file was used: the server deletes it.
pub(crate) async fn used(id: &str) {
    if let Ok(scope) = current() {
        discard(&scope.session, id).await;
    }
}

async fn discard(session: &Session, id: &str) {
    match api_call!(session, |api| api.blob_delete(id)) {
        Ok(())
        | Err(ApiFailure::Status {
            status: 404,
            ..
        }) => {}
        Err(e) => log::warn!("could not delete a file on the server: {}", e.into_core()),
    }
}

/// Marks an item's content to be fetched by the server when the item is released: `url` with the integration's
/// headers ([`Connector::fetch_headers`]) and this `Accept`.
pub fn fetch_marker(url: &str, accept: &str, name: &str, max: u64) -> Value {
    json!({"kind": "fetch", "url": url, "accept": accept, "name": name, "max": max.min(MAX_BLOB_BYTES)})
}

/// Marks an item whose `body` is the base64 of content made on the phone, to be uploaded when the item is released.
pub fn output_marker(name: &str) -> Value {
    json!({"kind": "output", "name": name})
}

// ---- file inputs of tools -------------------------------------------------------------------------------------------

/// A tool that takes a file.
struct FileTool {
    /// The arguments that carry the content inline.
    content: &'static [&'static str],
    /// The largest file it takes through the server.
    max: u64,
}

fn file_tool(call: &ConnectorCall) -> Option<(&'static str, FileTool)> {
    let spec = call.spec()?;
    let tool = match spec.tool {
        "github_release_asset_upload" => FileTool {
            content: &["content_base64"],
            max: MAX_BLOB_BYTES,
        },
        "github_file_put" => FileTool {
            content: &["content", "content_base64"],
            max: MAX_CONTENTS_BYTES,
        },
        "vault_attachment_add" => FileTool {
            content: &["content_base64"],
            max: MAX_READ_BYTES,
        },
        // A text Send takes no file (and text given for a file Send is a mistake the vault explains).
        "vault_send_create"
            if call.str_arg("type") == Some("file")
                && !call.args.contains_key("text")
                && !call.args.contains_key("hidden") =>
        {
            FileTool {
                content: &["content_base64"],
                max: MAX_READ_BYTES,
            }
        }
        _ => return None,
    };
    Some((spec.tool, tool))
}

/// The file name a slot for `call` gets (shown to the user, never a path).
fn input_name(call: &ConnectorCall) -> String {
    let given = match call.op.as_str() {
        "release_asset_upload" => call.str_arg("name"),
        "file_put" => call.str_arg("path").and_then(|p| p.rsplit('/').next()),
        _ => call.str_arg("file_name"),
    };
    let name = clean_name(given.unwrap_or_default());
    if name.is_empty() {
        "file".to_owned()
    } else {
        name
    }
}

fn clean_name(name: &str) -> String {
    text::truncate_chars(&text::one_line(name).replace(['/', '\\'], "_"), 200)
}

/// The file a write uses, checked; shown with its preview.
pub(crate) struct FileInput(Option<BlobInfo>);

impl FileInput {
    /// The preview with the file added: what it is, and the file itself for `ApprovalView.blob`.
    pub(crate) fn show(self, mut preview: Preview) -> Preview {
        let Some(info) = self.0 else {
            return preview;
        };
        preview.lines.push(format!(
            "Uploaded file: {} · {} · {}",
            clean_name(&info.name),
            size_text(info.size),
            text::one_line(&info.content_type)
        ));
        preview.lines.push(format!("SHA-256: {}", text::one_line(&info.sha256)));
        match &info.preview {
            BlobPreview::Text {
                head,
                truncated,
            } => {
                let shown = text::truncate_chars(head, PREVIEW_HEAD);
                let more = if *truncated || shown.len() < head.len() {
                    "\n…"
                } else {
                    ""
                };
                preview.lines.push(format!("Starts with:\n{shown}{more}"));
            }
            BlobPreview::Binary {
                description,
            } => preview.lines.push(format!("Looks like: {}", text::one_line(description))),
            BlobPreview::Image {
                ..
            } => preview.lines.push("An image (shown below)".to_owned()),
            BlobPreview::None => {}
        }
        preview.blob = Some(info);
        preview
    }
}

/// "812 bytes", "1.4 MB".
pub fn size_text(bytes: u64) -> String {
    #[allow(clippy::cast_precision_loss, reason = "a size shown with one decimal")]
    let f = bytes as f64;
    match bytes {
        b if b < 1024 => format!("{b} bytes"),
        b if b < 1024 * 1024 => format!("{:.1} KB", f / 1024.0),
        b if b < 1024 * 1024 * 1024 => format!("{:.1} MB", f / (1024.0 * 1024.0)),
        _ => format!("{:.1} GB", f / (1024.0 * 1024.0 * 1024.0)),
    }
}

/// What the user sees of a file the server holds.
pub fn blob_view(info: &BlobInfo) -> BlobView {
    let (preview_text, preview_image) = match &info.preview {
        BlobPreview::Text {
            head,
            ..
        } => (Some(text::neutralize(head)), None),
        BlobPreview::Binary {
            description,
        } => (Some(text::one_line(description)), None),
        BlobPreview::Image {
            data_base64,
            ..
        } => (None, data_base64.as_deref().and_then(|d| BASE64.decode(d.as_bytes()).ok())),
        BlobPreview::None => (None, None),
    };
    BlobView {
        id: info.id.0.clone(),
        connection_label: text::truncate_chars(&text::one_line(&info.connection_label), 64),
        name: clean_name(&info.name),
        size: info.size,
        content_type: text::one_line(&info.content_type),
        sha256: text::one_line(&info.sha256),
        purpose: match &info.purpose {
            BlobPurpose::Upload {
                reason,
            } => text::truncate_chars(&text::one_line(reason), 300),
            BlobPurpose::ToolInput {
                tool,
            } => format!("For {}", text::one_line(tool)),
            BlobPurpose::Output => "A result for the AI".to_owned(),
        },
        preview_text,
        preview_image,
        created_at: info.created_at,
        expires_at: info.expires_at,
    }
}

/// An upload waiting for the user, as the pending list shows it.
pub(crate) fn blob_item(info: &BlobInfo) -> PendingItem {
    let view = blob_view(info);
    PendingItem {
        kind: PendingKind::Blob,
        id: view.id.clone(),
        title: format!("{} wants to share a file: {}", view.connection_label, view.name),
        subtitle: format!("{} · {}", size_text(view.size), view.purpose),
        created_at: info.created_at,
        connection_id: info.connection_id.0.clone(),
        connection_label: view.connection_label,
        action: "upload".to_owned(),
        count: 1,
        service: SERVICE_FILES.to_owned(),
        account: None,
        wait_until: Some(info.expires_at),
        op: String::new(),
        op_title: String::new(),
        suggestion: None,
    }
}

/// An upload the user may decide on: an uploaded `reins_upload` file that has not expired.
fn awaits_decision(info: &BlobInfo, now: i64) -> bool {
    matches!(info.purpose, BlobPurpose::Upload { .. })
        && info.state == BlobState::Uploaded
        && info.expires_at > now
        && is_blob_id(&info.id.0)
        && check_id(&info.connection_id.0).is_ok()
}

fn upload_audit(info: &BlobInfo, outcome: &str, detail: String) -> AuditRecord {
    AuditRecord {
        seq: 0,
        at: unix_now(),
        connection_id: info.connection_id.0.clone(),
        connection_label: text::one_line(&info.connection_label),
        action: "upload".to_owned(),
        outcome: outcome.to_owned(),
        detail,
        grant_id: None,
        service: SERVICE_FILES.to_owned(),
        account: None,
        count: 1,
        info: AuditInfo {
            note: match &info.purpose {
                BlobPurpose::Upload {
                    reason,
                } => Some(text::one_line(reason)),
                _ => None,
            },
            ..AuditInfo::default()
        },
        op: String::new(),
    }
}

impl Engine {
    /// The scope connectors work in for a request.
    pub(crate) fn blob_scope(&self) -> Option<Scope> {
        self.session().ok().map(|session| Scope {
            session,
        })
    }

    /// An audit entry for a file step of a call to another integration.
    fn file_audit(&self, request: &RelayRequest, call: &ConnectorCall, outcome: &str, detail: &str) -> AuditRecord {
        let mut audit = self.audit(request, flow::action_of(call), outcome, detail, None, 1, &[]);
        audit.service.clone_from(&call.service);
        audit.op.clone_from(&call.op);
        audit
    }

    async fn refuse_file(
        &self,
        session: &Session,
        request: &RelayRequest,
        call: &ConnectorCall,
        message: &str,
    ) -> Result<(), CoreError> {
        let audit = self.file_audit(request, call, "error", message);
        self.finish(
            session,
            request,
            audit,
            RelayOutcome::Error {
                message: message.to_owned(),
            },
        )
        .await
    }

    /// Before a write is previewed: the file it needs. `None` when the request was answered here (with an upload link
    /// because no content was given, or with why the named upload cannot be used).
    pub(crate) async fn file_input(
        &self,
        session: &Session,
        request: &RelayRequest,
        call: &ConnectorCall,
    ) -> Result<Option<FileInput>, CoreError> {
        let Some((tool, file)) = file_tool(call) else {
            return Ok(Some(FileInput(None)));
        };
        // Content given, even empty (an empty file is a file), is the tool's to check.
        let inline = file.content.iter().any(|k| call.args.get(*k).is_some_and(|v| !v.is_null()));
        match (inline, call.str_arg("blob").filter(|b| !b.is_empty())) {
            (true, Some(_)) => {
                let message = "Give the file either inline or as `blob`, not both.";
                self.refuse_file(session, request, call, message).await.map(|()| None)
            }
            (true, None) => Ok(Some(FileInput(None))),
            (false, None) => self.upload_instructions(session, request, call, tool, &file).await.map(|()| None),
            (false, Some(id)) => match self.checked_input(session, request, tool, &file, id).await? {
                Ok(info) => Ok(Some(FileInput(Some(info)))),
                Err(message) => self.refuse_file(session, request, call, &message).await.map(|()| None),
            },
        }
    }

    /// Opens a slot for the file `call` needs and tells the AI where to upload it.
    async fn upload_instructions(
        &self,
        session: &Session,
        request: &RelayRequest,
        call: &ConnectorCall,
        tool: &str,
        file: &FileTool,
    ) -> Result<(), CoreError> {
        let name = input_name(call);
        let slot_request = BlobSlotRequest {
            v: PROTOCOL_VERSION,
            connection_id: request.connection_id.clone(),
            request_id: Some(request.id.clone()),
            name: name.clone(),
            content_type: call.str_arg("content_type").map(str::to_owned),
            max_bytes: file.max,
            purpose: BlobPurpose::ToolInput {
                tool: tool.to_owned(),
            },
            ttl_secs: DEFAULT_BLOB_TTL_SECS,
        };
        let slot = match api_call!(session, |api| api.blob_slot(&slot_request)) {
            Ok(slot) => slot,
            Err(e) => {
                log::warn!("could not open an upload slot: {}", e.into_core());
                let message = "The phone could not prepare an upload link on the Reins server. Try again later.";
                return self.refuse_file(session, request, call, message).await;
            }
        };
        let content = if call.op == "file_put" {
            "`content` or `content_base64`"
        } else {
            "`content_base64`"
        };
        let instructions = UploadInstructions {
            status: "upload_required".to_owned(),
            blob: slot.id.clone(),
            upload_url: slot.upload_url.clone(),
            max_bytes: file.max,
            expires_at: slot.expires_at,
            next: format!(
                "No file was given. Upload it first: curl -T <file> '{}' (at most {}, before {}). Then call {tool} \
                 again with the same arguments plus \"blob\": \"{}\", without {content}. The user sees the file \
                 and approves on their phone.",
                slot.upload_url,
                size_text(file.max),
                text::iso_utc(slot.expires_at),
                slot.id.0
            ),
        };
        let data = serde_json::to_value(&instructions).map_err(|e| CoreError::storage(e.to_string()))?;
        let audit = self.file_audit(request, call, "released", &format!("gave an upload link for {name}"));
        self.finish(
            session,
            request,
            audit,
            RelayOutcome::Result {
                result: ToolResult::Connector {
                    data,
                },
            },
        )
        .await
    }

    /// The upload `id` named by a call of `tool`, when it may be used: this connection's, uploaded, unexpired, made
    /// for this tool. The error is what the AI is told.
    async fn checked_input(
        &self,
        session: &Session,
        request: &RelayRequest,
        tool: &str,
        file: &FileTool,
        id: &str,
    ) -> Result<Result<BlobInfo, String>, CoreError> {
        let again = format!("Call {tool} again without `blob` for a new upload link.");
        if !is_blob_id(id) {
            return Ok(Err(format!("`blob` is not the id of an upload. {again}")));
        }
        let info = match api_call!(session, |api| api.blob_info(id)) {
            Ok(info) => info,
            Err(ApiFailure::Status {
                status: 404 | 410,
                ..
            }) => return Ok(Err(format!("There is no such upload (it may have expired). {again}"))),
            Err(e) => return Err(e.into_core()),
        };
        if info.connection_id != request.connection_id {
            return Ok(Err(format!("That upload belongs to another connection. {again}")));
        }
        if info.purpose
            != (BlobPurpose::ToolInput {
                tool: tool.to_owned(),
            })
        {
            return Ok(Err(format!("That upload was made for something else. {again}")));
        }
        if info.expires_at <= unix_now() {
            return Ok(Err(format!("That upload has expired. {again}")));
        }
        match info.state {
            BlobState::Uploaded => {}
            BlobState::Waiting => {
                return Ok(Err(
                    "The file has not arrived yet: upload it to the link you were given (curl -T), then call again."
                        .to_owned(),
                ));
            }
            BlobState::Approved | BlobState::Denied => {
                return Ok(Err(format!("That upload cannot be used for this. {again}")));
            }
        }
        if info.size == 0 {
            return Ok(Err(format!("The uploaded file is empty. {again}")));
        }
        if info.size > file.max {
            return Ok(Err(format!("The uploaded file is larger than {} allows ({}).", tool, size_text(file.max))));
        }
        Ok(Ok(info))
    }

    /// The user refused a write that named an upload: the file is not needed any more.
    pub(crate) async fn discard_input(&self, session: &Session, parked: &ParkedRequest) {
        let blob = parked.connector.as_ref().and_then(|c| c.preview.as_ref()).and_then(|p| p.blob.as_ref());
        if let Some(info) = blob {
            discard(session, &info.id.0).await;
        }
    }

    /// Hands over released items whose content is too large to go inline: each marked one gets a download link (the
    /// server fetches it in the phone's name, or receives what the phone made) and loses the marker.
    pub(crate) async fn deliver_files(
        &self,
        session: &Session,
        connector: &dyn Connector,
        account: &str,
        request: &RelayRequest,
        mut items: Vec<Item>,
    ) -> Result<Vec<Item>, CoreError> {
        for item in &mut items {
            let Some(marker) = item.extra.remove(DELIVER_KEY) else {
                continue;
            };
            let name = clean_name(marker["name"].as_str().unwrap_or("file"));
            let download = match marker["kind"].as_str() {
                Some("fetch") => {
                    let url = marker["url"].as_str().unwrap_or_default().to_owned();
                    let mut headers = connector.fetch_headers(account).await?;
                    if let Some(accept) = marker["accept"].as_str().filter(|a| !a.is_empty()) {
                        headers.push(("Accept".to_owned(), accept.to_owned()));
                    }
                    let fetch = BlobFetch {
                        v: PROTOCOL_VERSION,
                        connection_id: request.connection_id.clone(),
                        request_id: Some(request.id.clone()),
                        url,
                        headers,
                        name: name.clone(),
                        max_bytes: marker["max"].as_u64().unwrap_or(MAX_BLOB_BYTES).min(MAX_BLOB_BYTES),
                        ttl_secs: DEFAULT_BLOB_TTL_SECS,
                    };
                    api_call!(session, |api| api.blob_fetch(&fetch)).map_err(ApiFailure::into_core)?
                }
                Some("output") => {
                    let encoded = item.body.take().unwrap_or_default();
                    let bytes = Zeroizing::new(
                        BASE64.decode(encoded.as_bytes()).map_err(|_| CoreError::storage("corrupt parked file"))?,
                    );
                    let conn = &request.connection_id;
                    api_call!(session, |api| api.blob_output(conn, &name, DEFAULT_BLOB_TTL_SECS, bytes.to_vec()))
                        .map_err(ApiFailure::into_core)?
                }
                _ => return Err(CoreError::storage("corrupt parked item")),
            };
            link_into(&mut item.extra, &download);
            item.snippet = format!("{} · {} · as a download link", item.snippet, size_text(download.size));
        }
        Ok(items)
    }

    // ---- reins_upload --------------------------------------------------------------------------------------------

    /// `reins_upload`: opens a slot and answers at once; the user decides when the file arrives.
    pub(crate) async fn handle_request_upload(
        &self,
        session: &Session,
        request: &RelayRequest,
    ) -> Result<(), CoreError> {
        let reins_proto::gmail::ToolCall::RequestUpload {
            name,
            size,
            content_type,
            reason,
        } = &request.call
        else {
            return Err(CoreError::invalid("not an upload request"));
        };
        let max_bytes = size.saturating_add(size.div_ceil(10)).min(MAX_BLOB_BYTES);
        let slot_request = BlobSlotRequest {
            v: PROTOCOL_VERSION,
            connection_id: request.connection_id.clone(),
            request_id: Some(request.id.clone()),
            name: clean_name(name),
            content_type: content_type.clone(),
            max_bytes,
            purpose: BlobPurpose::Upload {
                reason: reason.clone(),
            },
            ttl_secs: DEFAULT_BLOB_TTL_SECS,
        };
        let mut audit = self.audit(request, "upload", "released", "", None, 1, &[]);
        SERVICE_FILES.clone_into(&mut audit.service);
        audit.info.note = Some(text::one_line(reason));
        let outcome = match api_call!(session, |api| api.blob_slot(&slot_request)) {
            Ok(slot) => {
                audit.detail = format!("gave an upload link for {} ({})", clean_name(name), size_text(*size));
                let download = slot.download_url.clone().unwrap_or_default();
                RelayOutcome::Result {
                    result: ToolResult::Connector {
                        data: json!({
                            "status": "upload_ready",
                            "blob": slot.id,
                            "upload_url": slot.upload_url,
                            "download_url": download,
                            "expires_at": slot.expires_at,
                            "next": format!(
                                "Upload the file with: curl -T <file> '{}' (at most {}, before {}). The user is asked \
                                 on their phone when it arrives; once they approve, it can be downloaded from {}.",
                                slot.upload_url,
                                size_text(max_bytes),
                                text::iso_utc(slot.expires_at),
                                download
                            ),
                        }),
                    },
                }
            }
            Err(e) => {
                log::warn!("could not open an upload slot: {}", e.into_core());
                let message = "The phone could not prepare an upload link on the Reins server. Try again later.";
                "error".clone_into(&mut audit.outcome);
                message.clone_into(&mut audit.detail);
                RelayOutcome::Error {
                    message: message.to_owned(),
                }
            }
        };
        self.finish(session, request, audit, outcome).await
    }

    /// Parks an upload that arrived, for the user (Autopilot's pass then tells the app, or decides it).
    fn park_blob(&self, info: &BlobInfo) -> Result<(), CoreError> {
        let payload = serde_json::to_vec(info).map_err(|e| CoreError::storage(e.to_string()))?;
        self.store.park(&info.id.0, PendingKind::Blob, info.created_at, unix_now(), &payload)?;
        Ok(())
    }

    /// Uploads listed by `GET /pending`.
    pub(crate) fn park_blobs(&self, blobs: Vec<BlobInfo>) -> Result<(), CoreError> {
        let now = unix_now();
        for info in blobs {
            if !awaits_decision(&info, now) {
                continue;
            }
            let Some(_guard) = self.begin(&info.id.0)? else {
                continue;
            };
            if let Err(e) = self.park_blob(&info) {
                log::warn!("could not park an upload: {e}");
            }
        }
        Ok(())
    }

    /// A push said an upload arrived.
    pub(crate) async fn handle_blob_push(&self, id: &str) -> Result<(), CoreError> {
        if !is_blob_id(id) {
            return Err(CoreError::invalid("malformed id"));
        }
        let session = self.session()?;
        let Some(_guard) = self.begin(id)? else {
            return Ok(());
        };
        match api_call!(&session, |api| api.blob_info(id)) {
            Ok(info) if awaits_decision(&info, unix_now()) => self.park_blob(&info),
            Ok(_)
            | Err(ApiFailure::Status {
                status: 404,
                ..
            }) => Ok(()),
            Err(e) => Err(e.into_core()),
        }
    }

    fn parked_blob(&self, id: &str) -> Result<BlobInfo, CoreError> {
        let row = self.store.pending_item(id, unix_now())?.ok_or(CoreError::NotFound)?;
        if row.kind != PendingKind::Blob {
            return Err(CoreError::NotFound);
        }
        serde_json::from_slice(&row.payload).map_err(|_| CoreError::storage("corrupt parked upload"))
    }

    /// An upload waiting for the user.
    pub fn blob_view(&self, id: &str) -> Result<BlobView, CoreError> {
        Ok(blob_view(&self.parked_blob(id)?))
    }

    /// The user decided on an upload: approved, it can be downloaded with its link (and used); refused, the server
    /// deletes it.
    pub async fn answer_blob(&self, id: &str, approve: bool) -> Result<(), CoreError> {
        let Some(_claim) = self.claim(id) else {
            return Err(CoreError::NotFound);
        };
        let note = self.ap_human_note(id);
        let decision = crate::autopilot::context::Decision::new("", note);
        let (result, _) = crate::autopilot::context::deciding(decision, self.answer_blob_claimed(id, approve)).await;
        if result.is_ok() {
            let verdict = if approve {
                crate::autopilot::Verdict::Approve
            } else {
                crate::autopilot::Verdict::Deny
            };
            self.ap_learn(id, verdict);
        }
        result
    }

    /// [`Engine::answer_blob`] for a caller that already claimed the upload (the user, or Autopilot).
    pub(crate) async fn answer_blob_claimed(&self, id: &str, approve: bool) -> Result<(), CoreError> {
        let session = self.session()?;
        let info = self.parked_blob(id)?;
        let decision = BlobDecision {
            v: PROTOCOL_VERSION,
            approved: approve,
        };
        match api_call!(&session, |api| api.blob_decision(id, &decision)) {
            Ok(())
            | Err(ApiFailure::Status {
                status: 404 | 409,
                ..
            }) => {}
            Err(e) => return Err(e.into_core()),
        }
        let what = format!("{} ({})", clean_name(&info.name), size_text(info.size));
        let audit = if approve {
            upload_audit(&info, "released", format!("approved the upload {what}"))
        } else {
            upload_audit(&info, "denied", format!("refused the upload {what}"))
        };
        self.store.append_audit(&audit)?;
        self.store.remove_pending(id)?;
        self.store.mark_handled(id, unix_now())?;
        self.notifier.item_resolved(id.to_owned());
        Ok(())
    }

    /// The pending-list entry of a parked upload.
    pub(crate) fn blob_pending_item(payload: &[u8]) -> Option<PendingItem> {
        serde_json::from_slice::<BlobInfo>(payload).ok().map(|info| blob_item(&info))
    }
}

/// What the AI gets for a file handed over as a link.
fn link_into(extra: &mut Map<String, Value>, d: &BlobDownload) {
    for (key, value) in [
        ("download_url", json!(d.download_url)),
        ("name", json!(d.name)),
        ("size", json!(d.size)),
        ("sha256", json!(d.sha256)),
        ("content_type", json!(d.content_type)),
        ("expires_at", json!(text::iso_utc(d.expires_at))),
        ("encoding", json!("link")),
    ] {
        extra.insert(key.to_owned(), value);
    }
}

/// Whether content of `size` bytes goes as a link rather than inline (only when links can be made).
pub fn as_link(size: u64) -> bool {
    size > INLINE_LIMIT && linking()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_read_naturally() {
        assert_eq!(size_text(812), "812 bytes");
        assert_eq!(size_text(1536), "1.5 KB");
        assert_eq!(size_text(5 * 1024 * 1024), "5.0 MB");
        assert_eq!(size_text(MAX_BLOB_BYTES), "1.0 GB");
    }

    #[test]
    fn slot_names_are_file_names_only() {
        let call = |op: &str, args: Value| ConnectorCall {
            service: "github".into(),
            op: op.into(),
            args: args.as_object().unwrap().clone(),
        };
        assert_eq!(input_name(&call("file_put", json!({"path": "docs/img/logo.png"}))), "logo.png");
        assert_eq!(input_name(&call("release_asset_upload", json!({"name": "app.apk"}))), "app.apk");
        assert_eq!(input_name(&call("attachment_add", json!({"file_name": "a\\b\u{202e}.txt"}))), "a_b.txt");
        assert_eq!(input_name(&call("attachment_add", json!({}))), "file");
    }

    #[test]
    fn only_uploaded_unexpired_uploads_await_a_decision() {
        let info = BlobInfo {
            v: 1,
            id: reins_proto::blob::BlobId("blob-0123456789abcdef".into()),
            connection_id: "c1".into(),
            connection_label: "Claude".into(),
            request_id: None,
            name: "a.txt".into(),
            purpose: BlobPurpose::Upload {
                reason: "r".into(),
            },
            state: BlobState::Uploaded,
            size: 3,
            sha256: "ab".into(),
            content_type: "text/plain".into(),
            preview: BlobPreview::None,
            created_at: 1,
            expires_at: 100,
        };
        assert!(awaits_decision(&info, 50));
        assert!(!awaits_decision(&info, 100));
        assert!(!awaits_decision(
            &BlobInfo {
                state: BlobState::Waiting,
                ..info.clone()
            },
            50
        ));
        assert!(!awaits_decision(
            &BlobInfo {
                purpose: BlobPurpose::Output,
                ..info.clone()
            },
            50
        ));
        assert!(!awaits_decision(
            &BlobInfo {
                id: reins_proto::blob::BlobId("../x".into()),
                ..info
            },
            50
        ));
    }
}
