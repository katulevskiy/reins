//! Phone-facing API (contracts §A) under `{domain_path}/rewarden/api`.
//!
//! Auth is the normal Vaultwarden login (`Headers`); every endpoint but A1 also requires the
//! caller to be the user's registered approval device.

use std::time::Duration;

use rewarden_proto::{
    check_version,
    device::{
        ApiError, Connections, DeviceRegistered, DeviceRegistration, MAX_PENDING_WAIT_SECS, PairingResult, Pending,
        codes,
    },
    ids::{ConnectionId, PairingId, RequestId},
    pairing::{PairingRequest, PairingResponse, PushKind, PushMessage},
    relay::{RelayRequest, RelayResponse},
};
use rocket::{
    Catcher, Data, Route, State, data::ToByteUnit, http::Status, response::status::Custom, serde::json::Json,
};
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::{
    HUB, fcm, now_unix,
    pairing::{PairingAnswer, PairingAnswerError},
    relay::AnswerError,
};
use crate::{
    auth::Headers,
    db::{
        DbConn, DbPool,
        models::{RewardenConnection, RewardenDevice},
    },
};

pub type PhoneResult<T> = Result<T, Custom<Json<ApiError>>>;

pub const MAX_FCM_TOKEN_BYTES: usize = 4096;
const MAX_BODY_MIB: u64 = 20;

pub fn routes() -> Vec<Route> {
    routes![
        put_device,
        put_services,
        get_pending,
        get_request,
        post_request_response,
        get_pairing,
        post_pairing_response,
        get_connections,
        delete_connection
    ]
}

pub fn catchers() -> Vec<Catcher> {
    catchers![unauthorized, not_found_catcher]
}

pub fn api_err(status: Status, code: &str, message: impl Into<String>) -> Custom<Json<ApiError>> {
    Custom(status, Json(ApiError::new(code, message)))
}

pub fn bad_request(message: impl Into<String>) -> Custom<Json<ApiError>> {
    api_err(Status::BadRequest, codes::BAD_REQUEST, message)
}

pub fn not_found() -> Custom<Json<ApiError>> {
    api_err(Status::NotFound, codes::NOT_FOUND, "Unknown or expired id")
}

/// 429 for a rate limit: what was limited, and in how many seconds to try again.
pub fn rate_limited(what: &str, wait: Duration) -> Custom<Json<ApiError>> {
    api_err(Status::TooManyRequests, codes::RATE_LIMITED, super::limits::rate_limited_text(what, wait))
}

/// 429 for a concurrency limit: `what` are still running.
pub fn too_many_running(what: &str) -> Custom<Json<ApiError>> {
    api_err(Status::TooManyRequests, codes::RATE_LIMITED, format!("Rate limited: {what}; try again when one ends."))
}

pub fn already_answered() -> Custom<Json<ApiError>> {
    api_err(Status::Conflict, codes::ALREADY_ANSWERED, "This item was already answered")
}

pub fn internal(e: &crate::Error) -> Custom<Json<ApiError>> {
    error!("Rewarden phone API: {e:?}");
    api_err(Status::InternalServerError, codes::INTERNAL, "Server error, please retry")
}

/// `wait` query value: invalid → 0, larger than 25 → 25 (contracts A2).
pub fn clamp_wait(raw: Option<&str>) -> u32 {
    raw.and_then(|w| w.trim().parse::<u32>().ok()).unwrap_or(0).min(MAX_PENDING_WAIT_SECS)
}

/// Decision 32: trimmed, empty → `None`, bounded, no control characters.
pub fn normalize_fcm_token(raw: Option<String>) -> Result<Option<String>, String> {
    let Some(token) = raw.map(|t| t.trim().to_owned()).filter(|t| !t.is_empty()) else {
        return Ok(None);
    };
    if token.len() > MAX_FCM_TOKEN_BYTES || token.chars().any(char::is_control) {
        return Err(format!("fcm_token must be at most {MAX_FCM_TOKEN_BYTES} bytes without control characters"));
    }
    Ok(Some(token))
}

/// Parses a relay/pairing message, checking `v` first so a newer phone gets `bad_version`
/// rather than a confusing shape error (proto invariant: receivers call `check_version`).
pub fn parse_versioned<T: DeserializeOwned>(body: &[u8]) -> PhoneResult<T> {
    let value: Value = serde_json::from_slice(body).map_err(|e| bad_request(format!("invalid JSON: {e}")))?;
    let v = value.get("v").and_then(Value::as_u64).and_then(|v| u32::try_from(v).ok()).unwrap_or(0);
    check_version(v).map_err(|e| api_err(Status::BadRequest, codes::BAD_VERSION, e.to_string()))?;
    serde_json::from_value(value).map_err(|e| bad_request(format!("invalid body: {e}")))
}

async fn read_body(data: Data<'_>) -> PhoneResult<Vec<u8>> {
    read_body_limited(data, MAX_BODY_MIB * 1024 * 1024).await
}

/// A JSON body of at most `limit` bytes.
pub async fn read_body_limited(data: Data<'_>, limit: u64) -> PhoneResult<Vec<u8>> {
    let bytes =
        data.open(limit.bytes()).into_bytes().await.map_err(|e| bad_request(format!("unreadable body: {e}")))?;
    if !bytes.is_complete() {
        return Err(api_err(Status::PayloadTooLarge, codes::BAD_REQUEST, "body too large"));
    }
    Ok(bytes.into_inner())
}

pub fn user_key(headers: &Headers) -> String {
    headers.user.uuid.to_string()
}

pub async fn require_approval_device(headers: &Headers, conn: &DbConn) -> PhoneResult<()> {
    match RewardenDevice::find_by_user(&headers.user.uuid, conn).await {
        Some(device) if device.device_uuid == headers.device.uuid => Ok(()),
        _ => Err(api_err(
            Status::Forbidden,
            codes::NOT_APPROVAL_DEVICE,
            "This device is not the Rewarden approval device; register it with PUT /rewarden/api/device",
        )),
    }
}

fn answer_error(e: &AnswerError) -> Custom<Json<ApiError>> {
    match e {
        AnswerError::NotFound => not_found(),
        AnswerError::AlreadyAnswered => already_answered(),
    }
}

/// A1: makes the calling device the approval device; tells the replaced one via push.
#[put("/rewarden/api/device", data = "<data>")]
async fn put_device(
    data: Data<'_>,
    headers: Headers,
    conn: DbConn,
    pool: &State<DbPool>,
) -> PhoneResult<Json<DeviceRegistered>> {
    let body = read_body(data).await?;
    let registration: DeviceRegistration = if body.iter().all(u8::is_ascii_whitespace) {
        DeviceRegistration::default()
    } else {
        serde_json::from_slice(&body).map_err(|e| bad_request(format!("invalid body: {e}")))?
    };
    let fcm_token = normalize_fcm_token(registration.fcm_token).map_err(bad_request)?;
    let row = RewardenDevice {
        user_uuid: headers.user.uuid.clone(),
        device_uuid: headers.device.uuid.clone(),
        fcm_token,
        updated_at: now_unix(),
    };
    let previous = row.replace(&conn).await.map_err(|e| internal(&e))?;
    let replaced = previous.filter(|p| p.device_uuid != row.device_uuid);
    if let Some(old) = &replaced {
        fcm::spawn_push(
            pool.inner().clone(),
            headers.user.uuid.clone(),
            old.fcm_token.clone(),
            PushMessage {
                t: PushKind::Replaced,
                id: String::new(),
            },
        );
    }
    Ok(Json(DeviceRegistered {
        replaced_previous: replaced.is_some(),
    }))
}

/// The integrations that have an account on the phone (ids only) and the MCP servers added there (tools, never
/// tokens); the server lists just their tools to the AI.
#[put("/rewarden/api/services", data = "<data>")]
async fn put_services(data: Data<'_>, headers: Headers, conn: DbConn) -> PhoneResult<Status> {
    require_approval_device(&headers, &conn).await?;
    drop(conn);
    let report: rewarden_proto::device::ServicesReport =
        serde_json::from_slice(&read_body(data).await?).map_err(|e| bad_request(format!("invalid body: {e}")))?;
    let known: std::collections::BTreeSet<&str> =
        rewarden_proto::connector::specs().iter().map(|s| s.service).chain(std::iter::once("gmail")).collect();
    let mut services: Vec<String> = report.services.into_iter().filter(|s| known.contains(s.as_str())).collect();
    services.sort();
    services.dedup();
    let user = user_key(&headers);
    HUB.set_services(&user, services);
    HUB.set_mcp_servers(&user, report.mcp);
    Ok(Status::NoContent)
}

/// A2: undelivered requests and pairings; long-polls up to `wait` seconds when empty.
#[get("/rewarden/api/pending?<wait>")]
async fn get_pending(wait: Option<String>, headers: Headers, conn: DbConn) -> PhoneResult<Json<Pending>> {
    require_approval_device(&headers, &conn).await?;
    // Never hold a pooled DB connection during a long-poll.
    drop(conn);
    let _poll = super::limits::DEVICE_POLLS
        .try_enter(&headers.device.uuid.to_string())
        .ok_or_else(|| too_many_running("too many long-polls of this device are open"))?;
    let wait = Duration::from_secs(u64::from(clamp_wait(wait.as_deref())));
    Ok(Json(HUB.pending(&user_key(&headers), wait).await))
}

/// A3
#[get("/rewarden/api/requests/<id>")]
async fn get_request(id: &str, headers: Headers, conn: DbConn) -> PhoneResult<Json<RelayRequest>> {
    require_approval_device(&headers, &conn).await?;
    HUB.relay.fetch(&user_key(&headers), &RequestId::from(id)).map(Json).ok_or_else(not_found)
}

/// A4
#[post("/rewarden/api/requests/<id>/response", data = "<data>")]
async fn post_request_response(id: &str, data: Data<'_>, headers: Headers, conn: DbConn) -> PhoneResult<Status> {
    require_approval_device(&headers, &conn).await?;
    let response: RelayResponse = parse_versioned(&read_body(data).await?)?;
    HUB.relay.answer(&user_key(&headers), &RequestId::from(id), response.outcome).map_err(|e| answer_error(&e))?;
    Ok(Status::NoContent)
}

/// A5
#[get("/rewarden/api/pairings/<id>")]
async fn get_pairing(id: &str, headers: Headers, conn: DbConn) -> PhoneResult<Json<PairingRequest>> {
    require_approval_device(&headers, &conn).await?;
    HUB.pairings.fetch(&user_key(&headers), &PairingId::from(id)).map(Json).ok_or_else(not_found)
}

/// A6: creates the connection when the right code was chosen.
#[post("/rewarden/api/pairings/<id>/response", data = "<data>")]
async fn post_pairing_response(
    id: &str,
    data: Data<'_>,
    headers: Headers,
    conn: DbConn,
) -> PhoneResult<Json<PairingResult>> {
    require_approval_device(&headers, &conn).await?;
    let response: PairingResponse = parse_versioned(&read_body(data).await?)?;
    let pairing_id = PairingId::from(id);
    match HUB.pairings.answer(&user_key(&headers), &pairing_id, &response) {
        Ok(PairingAnswer::Denied) => Ok(Json(PairingResult {
            connection_id: None,
        })),
        Ok(PairingAnswer::WrongCode) => Err(api_err(
            Status::Conflict,
            codes::WRONG_CODE,
            "The chosen code does not match the browser; the connection request was cancelled",
        )),
        Ok(PairingAnswer::Approved {
            client,
            label,
        }) => {
            let connection = RewardenConnection::new(
                headers.user.uuid.clone(),
                client.client_id,
                client.client_name,
                client.client_host,
                label,
                now_unix(),
            );
            if let Err(e) = connection.save(&conn).await {
                HUB.pairings.fail(&pairing_id);
                return Err(internal(&e));
            }
            let connection_id = ConnectionId(connection.uuid);
            HUB.pairings.complete(&pairing_id, connection_id.clone());
            Ok(Json(PairingResult {
                connection_id: Some(connection_id),
            }))
        }
        Err(PairingAnswerError::NotFound) => Err(not_found()),
        Err(PairingAnswerError::AlreadyAnswered) => Err(already_answered()),
        Err(PairingAnswerError::Invalid(message)) => Err(bad_request(message)),
    }
}

/// A7
#[get("/rewarden/api/connections")]
async fn get_connections(headers: Headers, conn: DbConn) -> PhoneResult<Json<Connections>> {
    require_approval_device(&headers, &conn).await?;
    let connections = RewardenConnection::find_by_user(&headers.user.uuid, &conn).await;
    Ok(Json(Connections {
        connections: connections.iter().map(RewardenConnection::to_info).collect(),
    }))
}

/// A8: access tokens die at once because every MCP call re-checks the connection.
#[delete("/rewarden/api/connections/<id>")]
async fn delete_connection(id: &str, headers: Headers, conn: DbConn) -> PhoneResult<Status> {
    require_approval_device(&headers, &conn).await?;
    let Some(connection) = RewardenConnection::find_by_uuid_and_user(id, &headers.user.uuid, &conn).await else {
        return Err(not_found());
    };
    connection.delete(&conn).await.map_err(|e| internal(&e))?;
    Ok(Status::NoContent)
}

#[catch(401)]
fn unauthorized() -> Json<ApiError> {
    Json(ApiError::new(codes::UNAUTHORIZED, "Missing or invalid Vaultwarden access token"))
}

#[catch(404)]
fn not_found_catcher() -> Json<ApiError> {
    Json(ApiError::new(codes::NOT_FOUND, "No such Rewarden API endpoint"))
}

#[cfg(test)]
mod tests {
    use rewarden_proto::relay::{RelayOutcome, RelayResponse};

    use super::*;

    #[test]
    fn wait_is_parsed_leniently_and_clamped() {
        assert_eq!(clamp_wait(None), 0);
        assert_eq!(clamp_wait(Some("10")), 10);
        assert_eq!(clamp_wait(Some("25")), 25);
        assert_eq!(clamp_wait(Some("999")), 25);
        assert_eq!(clamp_wait(Some("-1")), 0);
        assert_eq!(clamp_wait(Some("abc")), 0);
    }

    #[test]
    fn fcm_tokens_are_trimmed_and_bounded() {
        assert_eq!(normalize_fcm_token(None), Ok(None));
        assert_eq!(normalize_fcm_token(Some("   ".to_owned())), Ok(None));
        assert_eq!(normalize_fcm_token(Some(" abc:DEF_1 ".to_owned())), Ok(Some("abc:DEF_1".to_owned())));
        assert!(normalize_fcm_token(Some("x".repeat(MAX_FCM_TOKEN_BYTES + 1))).is_err());
        assert!(normalize_fcm_token(Some("a\nb".to_owned())).is_err());
    }

    #[test]
    fn rate_limits_are_429_with_a_retry_time() {
        let e = rate_limited("too many outbound requests for this account", Duration::from_millis(4200));
        assert_eq!((e.0, e.1.0.error.as_str()), (Status::TooManyRequests, codes::RATE_LIMITED));
        assert_eq!(e.1.0.message, "Rate limited: too many outbound requests for this account; try again in 5 s.");
        let e = too_many_running("too many long-polls of this device are open");
        assert_eq!((e.0, e.1.0.error.as_str()), (Status::TooManyRequests, codes::RATE_LIMITED));
    }

    #[test]
    fn versioned_bodies_check_v_before_shape() {
        let ok: RelayResponse = parse_versioned(br#"{"v":1,"outcome":"denied","reason":null}"#).unwrap();
        assert_eq!(
            ok.outcome,
            RelayOutcome::Denied {
                reason: None
            }
        );
        let e = parse_versioned::<RelayResponse>(br#"{"v":2,"outcome":"something-new"}"#).unwrap_err();
        assert_eq!((e.0, e.1.0.error.as_str()), (Status::BadRequest, codes::BAD_VERSION));
        let e = parse_versioned::<RelayResponse>(br#"{"outcome":"denied"}"#).unwrap_err();
        assert_eq!(e.1.0.error, codes::BAD_VERSION);
        let e = parse_versioned::<RelayResponse>(br#"{"v":1,"outcome":"bogus"}"#).unwrap_err();
        assert_eq!((e.0, e.1.0.error.as_str()), (Status::BadRequest, codes::BAD_REQUEST));
        let e = parse_versioned::<RelayResponse>(b"not json").unwrap_err();
        assert_eq!(e.1.0.error, codes::BAD_REQUEST);
    }
}
