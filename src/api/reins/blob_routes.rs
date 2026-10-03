//! The blob store over HTTP (files spec, S1).
//!
//! - Phone API under `/reins/api/blobs` (approval device only, owner checked on every call).
//! - Public capability URLs `/reins/blob/<secret>`: `PUT`/`POST` uploads into a slot, `GET` downloads a released
//!   file. The URL is the only credential, so it is never logged (see [`super::loggable_path`]).
//!
//! Bodies stream between the network and disk; a 1 GiB upload never sits in memory. Nothing here logs a header, a
//! body, a file's content or a secret.

use std::io::SeekFrom;

use reins_proto::{
    blob::{
        BlobDecision, BlobDownload, BlobFetch, BlobInfo, BlobPurpose, BlobSend, BlobSendResult, BlobSlot,
        BlobSlotRequest, MAX_SEND_RESPONSE, SendBody,
    },
    device::{ApiError, codes},
    ids::{ConnectionId, RequestId},
    pairing::{PushKind, PushMessage},
};
use rocket::{
    Data, Request, Route, State,
    data::ToByteUnit,
    http::Status,
    request::{FromRequest, Outcome},
    response::{Responder, Response, status::Custom},
    serde::json::Json,
};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncSeekExt};

use super::{
    HUB,
    blob::{BlobError, Owner, Received, WriteTicket},
    blob_io::{BlobWriter, FileBody, WriteError, parse_range, write_reader},
    device_api::{
        DeviceKey, PhoneResult, already_answered, api_err, bad_request, not_found, parse_versioned, rate_limited,
        read_body_limited, require_approval_device, too_many_running, user_key,
    },
    limits::{self, Admitted},
    now_unix,
    outbound::{self, NoBody, OutboundError},
    push,
    sniff::{BINARY_TYPE, content_disposition},
};
use crate::{
    CONFIG,
    auth::Headers,
    db::{
        DbConn, DbPool,
        models::{ReinsConnection, ReinsDevice, UserId},
    },
};

/// Path prefix of the public capability URLs.
pub const PUBLIC_PREFIX: &str = "/reins/blob/";
/// JSON bodies of the phone's blob calls (a `JsonBase64` template is the largest).
const MAX_JSON_BODY: u64 = 2 * 1024 * 1024;
/// Capability secrets are 43 characters; ids 22.
const MAX_TOKEN_LEN: usize = 64;

pub fn routes() -> Vec<Route> {
    routes![
        open_slot,
        get_info,
        post_decision,
        get_content,
        put_output,
        post_send,
        post_fetch,
        delete_blob,
        put_upload,
        post_upload,
        get_download
    ]
}

/// A capability URL for `secret`.
pub fn blob_url(secret: &str) -> String {
    format!("{}{PUBLIC_PREFIX}{secret}", CONFIG.domain().trim_end_matches('/'))
}

pub fn blob_err(e: BlobError) -> Custom<Json<ApiError>> {
    match e {
        BlobError::NotFound => not_found(),
        BlobError::Invalid(m) => bad_request(m),
        BlobError::UserLimit(m) => api_err(Status::TooManyRequests, codes::BAD_REQUEST, m),
        BlobError::ServerFull => api_err(
            Status::InsufficientStorage,
            codes::INTERNAL,
            "The server holds too many files right now; try again later.",
        ),
        BlobError::Conflict(m) => api_err(Status::Conflict, codes::BAD_REQUEST, m),
        BlobError::AlreadyDecided => already_answered(),
        BlobError::Storage(e) => {
            error!("Reins blob store: {e}");
            api_err(Status::InternalServerError, codes::INTERNAL, "Server error, please retry")
        }
    }
}

pub fn outbound_err(e: OutboundError) -> Custom<Json<ApiError>> {
    match e {
        OutboundError::Refused(m) => bad_request(m),
        OutboundError::Failed(m) => api_err(Status::BadGateway, codes::BAD_REQUEST, m),
    }
}

/// Admits one request the server makes for `user`'s phone: a place among their running ones, then their rate.
pub fn admit_outbound(user: &str) -> PhoneResult<Admitted<String>> {
    let key = user.to_owned();
    let running = limits::OUTBOUND_RUNNING
        .try_enter(&key)
        .ok_or_else(|| too_many_running("too many outbound requests of this account are running"))?;
    limits::OUTBOUND_REQUESTS
        .check(&key)
        .map_err(|wait| rate_limited("too many outbound requests for this account", wait))?;
    Ok(running)
}

fn too_large(limit: u64) -> Custom<Json<ApiError>> {
    api_err(Status::PayloadTooLarge, codes::BAD_REQUEST, format!("The file is larger than {limit} bytes."))
}

/// The blob's new owner: the caller, through one of their own AI connections.
pub async fn owner_of(
    headers: &Headers,
    connection_id: &ConnectionId,
    request_id: Option<RequestId>,
    conn: &DbConn,
) -> PhoneResult<Owner> {
    let unknown = || api_err(Status::NotFound, codes::NOT_FOUND, "Unknown connection");
    if connection_id.0.is_empty() || connection_id.0.len() > MAX_TOKEN_LEN {
        return Err(unknown());
    }
    if request_id.as_ref().is_some_and(|r| r.0.len() > MAX_TOKEN_LEN) {
        return Err(bad_request("`request_id` is not valid"));
    }
    let connection =
        ReinsConnection::find_by_uuid_and_user(&connection_id.0, &headers.user.uuid, conn).await.ok_or_else(unknown)?;
    Ok(Owner {
        user: user_key(headers),
        connection_id: connection_id.clone(),
        connection_label: connection.label,
        request_id,
    })
}

pub fn download_of(info: &BlobInfo, secret: &str) -> BlobDownload {
    BlobDownload {
        id: info.id.clone(),
        download_url: blob_url(secret),
        name: info.name.clone(),
        size: info.size,
        sha256: info.sha256.clone(),
        content_type: info.content_type.clone(),
        expires_at: info.expires_at,
    }
}

/// Writes bytes the server already holds as a new output blob of `owner` (large proxied MCP content).
pub async fn store_output(owner: &Owner, name: &str, bytes: &[u8], ttl_secs: u32) -> PhoneResult<BlobDownload> {
    let size = bytes.len().max(1) as u64;
    let ticket = HUB.blobs.open_output(owner, name, Some(size), ttl_secs, now_unix()).map_err(blob_err)?;
    if ticket.limit < size {
        // The quotas left less room than this exact result needs.
        HUB.blobs.abort_write(&ticket.id);
        return Err(api_err(
            Status::TooManyRequests,
            codes::BAD_REQUEST,
            format!("The account's file quotas leave no room for this {size}-byte result right now."),
        ));
    }
    let written = super::blob_io::write_bytes(bytes, &ticket.file).await;
    finish_output(ticket, written)
}

/// Records a finished output write (or undoes a failed one) and builds its download.
fn finish_output(ticket: WriteTicket, written: Result<Received, WriteError>) -> PhoneResult<BlobDownload> {
    let received = match written {
        Ok(r) => r,
        Err(e) => {
            HUB.blobs.abort_write(&ticket.id);
            return Err(match e {
                WriteError::TooLarge => too_large(ticket.limit),
                WriteError::Io(m) => {
                    warn!("Reins output blob not stored: {m}");
                    api_err(Status::BadGateway, codes::BAD_REQUEST, "The file could not be stored.")
                }
            });
        }
    };
    let (_, info) = HUB.blobs.finish_write(&ticket.id, received, now_unix()).map_err(blob_err)?;
    let secret = ticket.download_secret.unwrap_or_default();
    Ok(download_of(&info, &secret))
}

// ---------------------------------------------------------------------------------------
// File responses
// ---------------------------------------------------------------------------------------

/// The `Range` request header, if any.
pub struct RangeHeader(Option<String>);

#[rocket::async_trait]
impl<'r> FromRequest<'r> for RangeHeader {
    type Error = std::convert::Infallible;

    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        Outcome::Success(Self(request.headers().get_one("Range").map(str::to_owned)))
    }
}

/// A blob's bytes, streamed from disk.
pub struct FileResponse {
    file: tokio::fs::File,
    size: u64,
    /// `(start, length)` of a partial answer.
    range: Option<(u64, u64)>,
    content_type: String,
    disposition: Option<String>,
}

impl<'r> Responder<'r, 'static> for FileResponse {
    fn respond_to(self, _: &'r Request<'_>) -> Result<Response<'static>, Status> {
        let mut response = Response::build();
        let content_type = if self.content_type.is_empty() {
            BINARY_TYPE.to_owned()
        } else {
            self.content_type
        };
        response
            .raw_header("Content-Type", content_type)
            .raw_header("X-Content-Type-Options", "nosniff")
            .raw_header("Cache-Control", "no-store")
            .raw_header("Content-Security-Policy", "sandbox; default-src 'none'")
            .raw_header("Accept-Ranges", "bytes");
        if let Some(disposition) = self.disposition {
            response.raw_header("Content-Disposition", disposition);
        }
        match self.range {
            None => {
                response.sized_body(usize::try_from(self.size).ok(), self.file);
            }
            Some((start, length)) => {
                response
                    .status(Status::PartialContent)
                    .raw_header("Content-Range", format!("bytes {start}-{}/{}", start + length - 1, self.size))
                    .streamed_body(self.file.take(length));
            }
        }
        response.ok()
    }
}

async fn open_file(path: &std::path::Path) -> Option<tokio::fs::File> {
    tokio::fs::File::open(path).await.ok()
}

// ---------------------------------------------------------------------------------------
// Phone API
// ---------------------------------------------------------------------------------------

/// Opens a slot the AI uploads into.
#[post("/reins/api/blobs", data = "<data>")]
async fn open_slot(data: Data<'_>, headers: Headers, key: DeviceKey, conn: DbConn) -> PhoneResult<Json<BlobSlot>> {
    require_approval_device(&headers, &key, &conn).await?;
    let request: BlobSlotRequest = parse_versioned(&read_body_limited(data, MAX_JSON_BODY).await?)?;
    let owner = owner_of(&headers, &request.connection_id, request.request_id.clone(), &conn).await?;
    let slot = HUB.blobs.open_slot(&owner, &request, now_unix()).map_err(blob_err)?;
    Ok(Json(BlobSlot {
        id: slot.id,
        upload_url: blob_url(&slot.upload_secret),
        download_url: slot.download_secret.as_deref().map(blob_url),
        expires_at: slot.expires_at,
    }))
}

#[get("/reins/api/blobs/<id>")]
async fn get_info(id: &str, headers: Headers, key: DeviceKey, conn: DbConn) -> PhoneResult<Json<BlobInfo>> {
    require_approval_device(&headers, &key, &conn).await?;
    HUB.blobs.info(&user_key(&headers), id, now_unix()).map(Json).ok_or_else(not_found)
}

/// The user's answer for an upload made with `reins_upload`.
#[post("/reins/api/blobs/<id>/decision", data = "<data>")]
async fn post_decision(
    id: &str,
    data: Data<'_>,
    headers: Headers,
    key: DeviceKey,
    conn: DbConn,
) -> PhoneResult<Json<BlobInfo>> {
    require_approval_device(&headers, &key, &conn).await?;
    let decision: BlobDecision = parse_versioned(&read_body_limited(data, MAX_JSON_BODY).await?)?;
    HUB.blobs.decide(&user_key(&headers), id, decision.approved, now_unix()).map(Json).map_err(blob_err)
}

/// The bytes, for the phone to transform them itself (vault encryption); `Range` optional.
#[get("/reins/api/blobs/<id>/content")]
async fn get_content(
    id: &str,
    range: RangeHeader,
    headers: Headers,
    key: DeviceKey,
    conn: DbConn,
) -> PhoneResult<FileResponse> {
    require_approval_device(&headers, &key, &conn).await?;
    drop(conn);
    let user = user_key(&headers);
    let readable = HUB.blobs.readable(&user, id, now_unix()).map_err(blob_err)?;
    let range = parse_range(range.0.as_deref(), readable.size).map_err(|()| {
        api_err(Status::RangeNotSatisfiable, codes::BAD_REQUEST, format!("The file has {} bytes.", readable.size))
    })?;
    let length = range.map_or(readable.size, |(_, length)| length);
    HUB.blobs.charge_transfer(&user, length, now_unix()).map_err(blob_err)?;
    let mut file = open_file(&readable.file).await.ok_or_else(not_found)?;
    if let Some((start, _)) = range {
        file.seek(SeekFrom::Start(start)).await.map_err(|_| not_found())?;
    }
    Ok(FileResponse {
        file,
        size: readable.size,
        range,
        content_type: readable.content_type,
        disposition: None,
    })
}

/// A result the phone made itself (a decrypted vault attachment) becomes a download for the AI.
#[put("/reins/api/blobs/output?<connection_id>&<name>&<ttl_secs>&<request_id>", data = "<data>")]
#[expect(clippy::too_many_arguments, reason = "Rocket hands the query fields and guards over as arguments")]
async fn put_output(
    connection_id: Option<&str>,
    name: Option<&str>,
    ttl_secs: Option<u32>,
    request_id: Option<&str>,
    data: Data<'_>,
    headers: Headers,
    key: DeviceKey,
    conn: DbConn,
) -> PhoneResult<Json<BlobDownload>> {
    require_approval_device(&headers, &key, &conn).await?;
    let (Some(connection_id), Some(name)) = (connection_id, name) else {
        return Err(bad_request("`connection_id` and `name` are required"));
    };
    let owner = owner_of(&headers, &ConnectionId::from(connection_id), request_id.map(RequestId::from), &conn).await?;
    drop(conn);
    let ticket = HUB.blobs.open_output(&owner, name, None, ttl_secs.unwrap_or(0), now_unix()).map_err(blob_err)?;
    let written = write_reader(data.open((ticket.limit + 1).bytes()), &ticket.file, ticket.limit).await;
    finish_output(ticket, written).map(Json)
}

/// Streams a blob to the URL the phone names, with the phone's headers for this one request.
#[post("/reins/api/blobs/<id>/send", data = "<data>")]
async fn post_send(
    id: &str,
    data: Data<'_>,
    headers: Headers,
    key: DeviceKey,
    conn: DbConn,
) -> PhoneResult<Json<BlobSendResult>> {
    require_approval_device(&headers, &key, &conn).await?;
    drop(conn);
    let send: BlobSend = parse_versioned(&read_body_limited(data, MAX_JSON_BODY).await?)?;
    let method = match send.method.to_ascii_uppercase().as_str() {
        "PUT" => reqwest::Method::PUT,
        "POST" => reqwest::Method::POST,
        _ => return Err(bad_request("`method` must be PUT or POST")),
    };
    let user = user_key(&headers);
    let readable = HUB.blobs.readable(&user, id, now_unix()).map_err(blob_err)?;
    let phone_headers = outbound::header_map(&send.headers, &["content-type"]).map_err(outbound_err)?;
    let content_type = match &send.body {
        SendBody::JsonBase64 {
            ..
        } => "application/json".to_owned(),
        SendBody::Raw => send
            .headers
            .iter()
            .find(|(k, _)| k.trim().eq_ignore_ascii_case("content-type"))
            .map_or_else(|| BINARY_TYPE.to_owned(), |(_, v)| v.clone()),
    };
    let mut fixed = reqwest::header::HeaderMap::new();
    let content_type = reqwest::header::HeaderValue::from_str(&content_type)
        .map_err(|_| bad_request("The content type is not valid"))?;
    fixed.insert(reqwest::header::CONTENT_TYPE, content_type);
    if let SendBody::JsonBase64 {
        json,
        field,
    } = &send.body
    {
        super::blob_io::json_frame(json, field).map_err(bad_request)?;
    }
    let _running = admit_outbound(&user)?;
    HUB.blobs.charge_transfer(&user, readable.size, now_unix()).map_err(blob_err)?;
    let body = FileBody {
        file: readable.file,
        size: readable.size,
        mode: send.body,
    };
    let response =
        outbound::execute(method, &send.url, &phone_headers, &fixed, &body, true).await.map_err(outbound_err)?;
    let status = response.status().as_u16();
    let kept: Vec<(String, String)> = ["content-type", "location"]
        .iter()
        .filter_map(|name| {
            response.headers().get(*name).and_then(|v| v.to_str().ok()).map(|v| ((*name).to_owned(), v.to_owned()))
        })
        .collect();
    let (bytes, truncated) = outbound::read_limited(response, MAX_SEND_RESPONSE).await.map_err(outbound_err)?;
    Ok(Json(BlobSendResult {
        status,
        headers: kept,
        body: String::from_utf8_lossy(&bytes).into_owned(),
        truncated,
    }))
}

/// Downloads a large result into a new blob the AI may fetch.
#[post("/reins/api/blobs/fetch", data = "<data>")]
async fn post_fetch(data: Data<'_>, headers: Headers, key: DeviceKey, conn: DbConn) -> PhoneResult<Json<BlobDownload>> {
    require_approval_device(&headers, &key, &conn).await?;
    let fetch: BlobFetch = parse_versioned(&read_body_limited(data, MAX_JSON_BODY).await?)?;
    let owner = owner_of(&headers, &fetch.connection_id, fetch.request_id.clone(), &conn).await?;
    drop(conn);
    let phone_headers = outbound::header_map(&fetch.headers, &[]).map_err(outbound_err)?;
    let _running = admit_outbound(&owner.user)?;
    let ticket = HUB
        .blobs
        .open_output(&owner, &fetch.name, Some(fetch.max_bytes), fetch.ttl_secs, now_unix())
        .map_err(blob_err)?;
    let fixed = reqwest::header::HeaderMap::new();
    let response =
        match outbound::execute(reqwest::Method::GET, &fetch.url, &phone_headers, &fixed, &NoBody, true).await {
            Ok(r) => r,
            Err(e) => {
                HUB.blobs.abort_write(&ticket.id);
                return Err(outbound_err(e));
            }
        };
    if !response.status().is_success() {
        HUB.blobs.abort_write(&ticket.id);
        let host = response.url().host_str().unwrap_or_default().to_owned();
        return Err(api_err(Status::BadGateway, codes::BAD_REQUEST, format!("{host} answered {}.", response.status())));
    }
    if response.content_length().is_some_and(|n| n > ticket.limit) {
        HUB.blobs.abort_write(&ticket.id);
        return Err(too_large(ticket.limit));
    }
    let written = write_response(response, &ticket).await;
    finish_output(ticket, written).map(Json)
}

async fn write_response(mut response: reqwest::Response, ticket: &WriteTicket) -> Result<Received, WriteError> {
    let mut writer = BlobWriter::create(&ticket.file, ticket.limit).await?;
    while let Some(chunk) = response.chunk().await.map_err(|_| WriteError::Io("the download broke off".to_owned()))? {
        writer.write(&chunk).await?;
    }
    writer.finish().await
}

#[delete("/reins/api/blobs/<id>")]
async fn delete_blob(id: &str, headers: Headers, key: DeviceKey, conn: DbConn) -> PhoneResult<Status> {
    require_approval_device(&headers, &key, &conn).await?;
    HUB.blobs.remove(&user_key(&headers), id).map_err(blob_err)?;
    Ok(Status::NoContent)
}

// ---------------------------------------------------------------------------------------
// Public capability URLs
// ---------------------------------------------------------------------------------------

type Public = Custom<Json<Value>>;

fn public_error(status: Status, error: &str, message: &str) -> Public {
    Custom(status, Json(json!({"error": error, "message": message})))
}

fn unknown_link() -> Public {
    public_error(Status::NotFound, "not_found", "This link is unknown, used up or expired.")
}

/// What the uploader should do next, in plain words.
fn next_step(info: &BlobInfo) -> String {
    match &info.purpose {
        BlobPurpose::ToolInput {
            tool,
        } => format!(
            "Received. Now call {tool} again with blob=\"{}\"; the user approves that call on their phone and sees this \
             file.",
            info.id.0
        ),
        BlobPurpose::Upload {
            ..
        } => "Received. The user is asked on their phone whether to share this file; once they approve, the \
              download link you were given works until it expires."
            .to_owned(),
        BlobPurpose::Output => "Received.".to_owned(),
    }
}

async fn upload(secret: &str, data: Data<'_>, pool: &DbPool) -> Public {
    if secret.len() > MAX_TOKEN_LEN {
        return unknown_link();
    }
    let ticket = match HUB.blobs.begin_upload(secret, now_unix()) {
        Ok(t) => t,
        Err(BlobError::Conflict(m)) => return public_error(Status::Conflict, "conflict", &m),
        Err(BlobError::UserLimit(m)) => return public_error(Status::TooManyRequests, "rate_limited", &m),
        Err(_) => return unknown_link(),
    };
    let written = write_reader(data.open((ticket.limit + 1).bytes()), &ticket.file, ticket.limit).await;
    let received = match written {
        Ok(r) => r,
        Err(e) => {
            HUB.blobs.abort_write(&ticket.id);
            return match e {
                WriteError::TooLarge => public_error(
                    Status::PayloadTooLarge,
                    "too_large",
                    &format!("The file is larger than the {} bytes this link accepts.", ticket.limit),
                ),
                WriteError::Io(m) => {
                    warn!("Reins upload not stored: {m}");
                    public_error(Status::BadRequest, "upload_failed", "The upload broke off; send the file again.")
                }
            };
        }
    };
    let Ok((user, info)) = HUB.blobs.finish_write(&ticket.id, received, now_unix()) else {
        return unknown_link();
    };
    if matches!(info.purpose, BlobPurpose::Upload { .. }) {
        wake_phone(pool, user, &info.id.0).await;
    }
    Custom(
        Status::Created,
        Json(json!({"blob": info.id, "size": info.size, "sha256": info.sha256, "next": next_step(&info)})),
    )
}

/// Tells the approval device that an upload waits for the user.
async fn wake_phone(pool: &DbPool, user: String, blob_id: &str) {
    let Ok(conn) = pool.get().await else {
        warn!("No DB connection to wake the Reins device for an upload; it must poll");
        return;
    };
    let user_uuid = UserId::from(user);
    if let Some(device) = ReinsDevice::find_by_user(&user_uuid, &conn).await {
        push::spawn_push(
            pool.clone(),
            user_uuid,
            device.fcm_token,
            PushMessage {
                t: PushKind::Blob,
                id: blob_id.to_owned(),
            },
        );
    }
}

/// `curl -T file URL`: the AI uploads into a slot. The URL is the credential; no other authentication.
#[put("/reins/blob/<secret>", data = "<data>")]
async fn put_upload(secret: &str, data: Data<'_>, pool: &State<DbPool>) -> Public {
    upload(secret, data, pool.inner()).await
}

/// The same upload as a raw `POST` body, for clients that cannot `PUT`.
#[post("/reins/blob/<secret>", data = "<data>")]
async fn post_upload(secret: &str, data: Data<'_>, pool: &State<DbPool>) -> Public {
    upload(secret, data, pool.inner()).await
}

/// Downloads an output or an approved upload, always as an attachment.
#[get("/reins/blob/<secret>")]
async fn get_download(secret: &str) -> Result<FileResponse, Public> {
    if secret.len() > MAX_TOKEN_LEN {
        return Err(unknown_link());
    }
    let readable = HUB.blobs.begin_download(secret, now_unix()).map_err(|e| match e {
        BlobError::UserLimit(m) => public_error(Status::TooManyRequests, "rate_limited", &m),
        _ => unknown_link(),
    })?;
    let file = open_file(&readable.file).await.ok_or_else(unknown_link)?;
    Ok(FileResponse {
        file,
        size: readable.size,
        range: None,
        content_type: readable.content_type,
        disposition: Some(content_disposition(&readable.name)),
    })
}

#[cfg(test)]
mod tests {
    use reins_proto::blob::{BlobId, BlobPreview, BlobState};

    use super::*;

    fn info(purpose: BlobPurpose) -> BlobInfo {
        BlobInfo {
            v: 1,
            id: BlobId("blob-id-0123456789".into()),
            connection_id: "c1".into(),
            connection_label: "Claude".into(),
            request_id: None,
            name: "a.bin".into(),
            purpose,
            state: BlobState::Uploaded,
            size: 1,
            sha256: String::new(),
            content_type: String::new(),
            preview: BlobPreview::None,
            created_at: 0,
            expires_at: 1,
        }
    }

    #[test]
    fn the_uploader_is_told_what_comes_next() {
        let tool = next_step(&info(BlobPurpose::ToolInput {
            tool: "github_file_put".into(),
        }));
        assert!(tool.contains("github_file_put") && tool.contains("blob=\"blob-id-0123456789\""), "{tool}");
        let upload = next_step(&info(BlobPurpose::Upload {
            reason: "r".into(),
        }));
        assert!(upload.contains("approve"), "{upload}");
    }

    #[test]
    fn errors_map_to_statuses() {
        assert_eq!(blob_err(BlobError::NotFound).0, Status::NotFound);
        assert_eq!(blob_err(BlobError::AlreadyDecided).1.0.error, codes::ALREADY_ANSWERED);
        assert_eq!(blob_err(BlobError::UserLimit("x".into())).0, Status::TooManyRequests);
        assert_eq!(blob_err(BlobError::ServerFull).0, Status::InsufficientStorage);
        assert_eq!(outbound_err(OutboundError::Refused("x".into())).0, Status::BadRequest);
        assert_eq!(outbound_err(OutboundError::Failed("x".into())).0, Status::BadGateway);
    }
}
