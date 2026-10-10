//! Phone-facing API (contracts §A) under `{domain_path}/reins/api`.
//!
//! Auth is the normal Vaultwarden login (`Headers`); every endpoint but A1 also requires the
//! caller to be the user's registered approval device ([`is_caller`]: its Vaultwarden device and its device key). A1
//! itself lets another device take that role only with a proof ([`check_takeover`]).

use std::time::Duration;

use reins_proto::{
    check_version,
    device::{
        AccountSecretRotation, ApiError, Connections, DEVICE_KEY_HEADER, DeviceInfo, DeviceRegistered,
        DeviceRegistration, DeviceSignOut, Devices, MAX_PENDING_WAIT_SECS, PairingResult, Pending, TAKEOVER_REFUSED,
        codes, device_key_hash,
    },
    ids::{ConnectionId, PairingId, RequestId},
    pairing::{PairingClaim, PairingRequest, PairingResponse, PushKind, PushMessage},
    relay::{RelayRequest, RelayResponse},
};
use rocket::{
    Catcher, Data, Request, Route, State,
    data::ToByteUnit,
    http::Status,
    request::{FromRequest, Outcome},
    response::status::Custom,
    serde::json::Json,
};
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::{
    HUB, apns,
    device_flow::{ClaimError, DEVICE_GRANTS},
    now_unix,
    pairing::{PairingAnswer, PairingAnswerError},
    push,
    relay::AnswerError,
};
use crate::{
    auth::{ClientIp, Headers},
    db::{
        DbConn, DbPool,
        models::{Device, DeviceId, ReinsConnection, ReinsDevice, ReinsDeviceSignout, ReinsSsoSession},
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
        post_pairing_claim,
        get_connections,
        delete_connection,
        get_devices,
        delete_device,
        post_account_secret,
        post_logout
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
    error!("Reins phone API: {e:?}");
    api_err(Status::InternalServerError, codes::INTERNAL, "Server error, please retry")
}

/// `wait` query value: invalid → 0, larger than 25 → 25 (contracts A2).
pub fn clamp_wait(raw: Option<&str>) -> u32 {
    raw.and_then(|w| w.trim().parse::<u32>().ok()).unwrap_or(0).min(MAX_PENDING_WAIT_SECS)
}

/// Decision 32: trimmed, empty → `None`, bounded, no control characters. The iOS app's tokens
/// (`apns:` or `apns-sandbox:` and 64 to 200 hex digits) must be well formed and are stored with lowercase hex.
pub fn normalize_fcm_token(raw: Option<String>) -> Result<Option<String>, String> {
    let Some(token) = raw.map(|t| t.trim().to_owned()).filter(|t| !t.is_empty()) else {
        return Ok(None);
    };
    if token.len() > MAX_FCM_TOKEN_BYTES || token.chars().any(char::is_control) {
        return Err(format!("fcm_token must be at most {MAX_FCM_TOKEN_BYTES} bytes without control characters"));
    }
    match apns::normalize_token(&token) {
        Some(apns) => apns.map(Some).map_err(|e| format!("fcm_token: {e}")),
        None => Ok(Some(token)),
    }
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

/// Return the provider's logout URL to the native app. Visiting it in the same browser as sign-in clears
/// AuthKit's cookie; a server-side HTTP request cannot do that. The caller cannot supply a session id or redirect.
#[post("/reins/api/logout")]
async fn post_logout(headers: Headers, conn: DbConn) -> PhoneResult<Json<Value>> {
    let browser_url = if crate::sso_workos::enabled() {
        ReinsSsoSession::latest_for_device(&headers.user.uuid, &headers.device.uuid, &conn)
            .await
            .map_err(|e| internal(&e))?
            .map(|session| crate::sso_workos::logout_url(&session.session_id))
            .transpose()
            .map_err(|e| internal(&e))?
    } else {
        None
    };
    // Invalidate this device's access and refresh tokens immediately, even if the browser is later closed.
    ReinsSsoSession::revoke_device(&headers.user.uuid, &headers.device.uuid, &conn).await.map_err(|e| internal(&e))?;
    Ok(Json(serde_json::json!({"browser_url": browser_url})))
}

/// The caller's device key ([`DEVICE_KEY_HEADER`]) as the server keeps it ([`device_key_hash`]); `None` when the
/// header is missing or malformed.
pub struct DeviceKey(Option<String>);

impl DeviceKey {
    pub fn hash(&self) -> Option<&str> {
        self.0.as_deref()
    }
}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for DeviceKey {
    type Error = ();

    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, ()> {
        Outcome::Success(Self(request.headers().get_one(DEVICE_KEY_HEADER).and_then(device_key_hash)))
    }
}

/// Whether the approval device `row` is the caller: the same Vaultwarden device, with the same device key. The device
/// id alone proves nothing (the account's device list shows it, and a sign-in may claim any id). A row kept before
/// device keys existed matches on the device alone, until that device registers again with its key.
pub fn is_caller(row: &ReinsDevice, headers: &Headers, key: &DeviceKey) -> bool {
    row.device_uuid == headers.device.uuid
        && match (&row.key_hash, key.hash()) {
            (None, _) => true,
            (Some(kept), Some(given)) => crate::crypto::ct_eq(kept, given),
            (Some(_), None) => false,
        }
}

pub async fn require_approval_device(headers: &Headers, key: &DeviceKey, conn: &DbConn) -> PhoneResult<()> {
    match ReinsDevice::find_by_user(&headers.user.uuid, conn).await {
        Some(device) if is_caller(&device, headers, key) => {
            HUB.presence.seen(&user_key(headers), now_unix());
            Ok(())
        }
        _ => Err(api_err(
            Status::Forbidden,
            codes::NOT_APPROVAL_DEVICE,
            "This device is not the Reins approval device; register it with PUT /reins/api/device",
        )),
    }
}

fn answer_error(e: &AnswerError) -> Custom<Json<ApiError>> {
    match e {
        AnswerError::NotFound => not_found(),
        AnswerError::AlreadyAnswered => already_answered(),
    }
}

/// Whether a device that is not the account's approval device may take the role: an approval of its "add another
/// phone" request (spent here), or the master password hash of the account secret or master password. Wrong hashes
/// count against the account ([`super::limits::DEVICE_PROOFS`]).
fn check_takeover(headers: &Headers, key: &DeviceKey, master_password_hash: Option<&str>) -> PhoneResult<()> {
    let user = user_key(headers);
    if HUB.joins.take_takeover(&user, &headers.device.uuid.to_string(), key.hash(), now_unix()) {
        return Ok(());
    }
    let Some(hash) = master_password_hash.filter(|h| !h.is_empty()) else {
        return Err(api_err(Status::Forbidden, codes::PROOF_REQUIRED, TAKEOVER_REFUSED));
    };
    if let Err(wait) = super::limits::DEVICE_PROOFS.check(&user) {
        return Err(rate_limited("too many wrong recovery codes or passwords for this account", wait));
    }
    if headers.user.check_valid_password(hash) {
        return Ok(());
    }
    super::limits::DEVICE_PROOFS.fail(&user);
    warn!("Reins: a device of user {user} sent a wrong proof to become the approval device");
    Err(api_err(Status::Forbidden, codes::WRONG_PROOF, TAKEOVER_REFUSED))
}

/// A1: makes the calling device the approval device; tells the replaced one via push. The account's first approval
/// device, and the approval device registering again, need no proof; any other device needs one ([`check_takeover`]).
#[put("/reins/api/device", data = "<data>")]
async fn put_device(
    data: Data<'_>,
    headers: Headers,
    key: DeviceKey,
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
    let current = ReinsDevice::find_by_user(&headers.user.uuid, &conn).await;
    if let Some(current) = &current
        && !is_caller(current, &headers, &key)
    {
        check_takeover(&headers, &key, registration.master_password_hash.as_deref())?;
    }
    let row = ReinsDevice {
        user_uuid: headers.user.uuid.clone(),
        device_uuid: headers.device.uuid.clone(),
        fcm_token,
        updated_at: now_unix(),
        key_hash: key.hash().map(str::to_owned),
    };
    let previous = row.replace(&conn).await.map_err(|e| internal(&e))?;
    let replaced = previous.filter(|p| !is_caller(p, &headers, &key));
    // Not to this phone itself (the same push token: it lost its device key and proved itself again).
    if let Some(old) = replaced.as_ref().filter(|old| old.fcm_token != row.fcm_token) {
        push::spawn_push(
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
#[put("/reins/api/services", data = "<data>")]
async fn put_services(data: Data<'_>, headers: Headers, key: DeviceKey, conn: DbConn) -> PhoneResult<Status> {
    require_approval_device(&headers, &key, &conn).await?;
    drop(conn);
    let report: reins_proto::device::ServicesReport =
        serde_json::from_slice(&read_body(data).await?).map_err(|e| bad_request(format!("invalid body: {e}")))?;
    let known: std::collections::BTreeSet<&str> =
        reins_proto::connector::specs().iter().map(|s| s.service).chain(std::iter::once("gmail")).collect();
    let mut services: Vec<String> = report.services.into_iter().filter(|s| known.contains(s.as_str())).collect();
    services.sort();
    services.dedup();
    let user = user_key(&headers);
    HUB.set_services(&user, services);
    HUB.set_mcp_servers(&user, report.mcp);
    Ok(Status::NoContent)
}

/// A2: undelivered requests and pairings; long-polls up to `wait` seconds when empty.
#[get("/reins/api/pending?<wait>")]
async fn get_pending(
    wait: Option<String>,
    headers: Headers,
    key: DeviceKey,
    conn: DbConn,
    pool: &State<DbPool>,
) -> PhoneResult<Json<Pending>> {
    require_approval_device(&headers, &key, &conn).await?;
    // Never hold a pooled DB connection during a long-poll.
    drop(conn);
    let _poll = super::limits::DEVICE_POLLS
        .try_enter(&headers.device.uuid.to_string())
        .ok_or_else(|| too_many_running("too many long-polls of this device are open"))?;
    let wait = Duration::from_secs(u64::from(clamp_wait(wait.as_deref())));
    let mut pending = HUB.pending(&user_key(&headers), wait).await;
    if crate::sso_workos::enabled() {
        // Carry the authoritative email with the existing poll; no extra mobile profile requests are needed.
        // Reacquire only after the wait so a change during a long-poll appears in that response.
        let conn = pool.get().await.map_err(|e| internal(&e))?;
        let user = crate::db::models::reins_workos::user_by_id(&headers.user.uuid, &conn)
            .await
            .map_err(|e| internal(&e))?
            .filter(|user| user.enabled)
            .ok_or_else(|| api_err(Status::Unauthorized, codes::UNAUTHORIZED, "The account is no longer active"))?;
        pending.account_email = Some(user.email);
    }
    Ok(Json(pending))
}

/// A3
#[get("/reins/api/requests/<id>")]
async fn get_request(id: &str, headers: Headers, key: DeviceKey, conn: DbConn) -> PhoneResult<Json<RelayRequest>> {
    require_approval_device(&headers, &key, &conn).await?;
    HUB.relay.fetch(&user_key(&headers), &RequestId::from(id)).map(Json).ok_or_else(not_found)
}

/// A4
#[post("/reins/api/requests/<id>/response", data = "<data>")]
async fn post_request_response(
    id: &str,
    data: Data<'_>,
    headers: Headers,
    key: DeviceKey,
    conn: DbConn,
) -> PhoneResult<Status> {
    require_approval_device(&headers, &key, &conn).await?;
    let response: RelayResponse = parse_versioned(&read_body(data).await?)?;
    HUB.relay.answer(&user_key(&headers), &RequestId::from(id), response.outcome).map_err(|e| answer_error(&e))?;
    Ok(Status::NoContent)
}

/// A5
#[get("/reins/api/pairings/<id>")]
async fn get_pairing(id: &str, headers: Headers, key: DeviceKey, conn: DbConn) -> PhoneResult<Json<PairingRequest>> {
    require_approval_device(&headers, &key, &conn).await?;
    HUB.pairings.fetch(&user_key(&headers), &PairingId::from(id)).map(Json).ok_or_else(not_found)
}

/// A6: creates the connection when the right code was chosen.
#[post("/reins/api/pairings/<id>/response", data = "<data>")]
async fn post_pairing_response(
    id: &str,
    data: Data<'_>,
    headers: Headers,
    key: DeviceKey,
    conn: DbConn,
) -> PhoneResult<Json<PairingResult>> {
    require_approval_device(&headers, &key, &conn).await?;
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
            let connection = ReinsConnection::new(
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

/// A6b: the phone scanned a computer's QR code (or opened its link): the pairing its code stands for, started now for
/// this account and answered with A6 like any other.
#[post("/reins/api/pairings/claim", data = "<data>")]
async fn post_pairing_claim(
    data: Data<'_>,
    headers: Headers,
    key: DeviceKey,
    conn: DbConn,
) -> PhoneResult<Json<PairingRequest>> {
    require_approval_device(&headers, &key, &conn).await?;
    drop(conn);
    let claim: PairingClaim = parse_versioned(&read_body_limited(data, 4096).await?)?;
    // Counted with the pairings started for this account's email: every claim may start one.
    if let Err(wait) = super::limits::PAIRINGS.check(&headers.user.email.to_lowercase()) {
        return Err(rate_limited("too many connection requests for this account", wait));
    }
    match DEVICE_GRANTS.claim(&user_key(&headers), &claim.user_code, &HUB.pairings, now_unix()) {
        Ok(pairing) => Ok(Json(pairing)),
        Err(ClaimError::NotFound) => Err(api_err(
            Status::NotFound,
            codes::NOT_FOUND,
            "Unknown or expired pairing code; show a new one on the computer",
        )),
        Err(ClaimError::Taken) => {
            Err(api_err(Status::Conflict, codes::ALREADY_ANSWERED, "Another account's phone already scanned this code"))
        }
        Err(ClaimError::Busy) => {
            Err(api_err(Status::ServiceUnavailable, codes::INTERNAL, "The server is busy; try again in a minute"))
        }
    }
}

/// A7
#[get("/reins/api/connections")]
async fn get_connections(headers: Headers, key: DeviceKey, conn: DbConn) -> PhoneResult<Json<Connections>> {
    require_approval_device(&headers, &key, &conn).await?;
    let connections = ReinsConnection::find_by_user(&headers.user.uuid, &conn).await;
    Ok(Json(Connections {
        connections: connections.iter().map(ReinsConnection::to_info).collect(),
    }))
}

/// A8: access tokens die at once because every MCP call re-checks the connection.
#[delete("/reins/api/connections/<id>")]
async fn delete_connection(id: &str, headers: Headers, key: DeviceKey, conn: DbConn) -> PhoneResult<Status> {
    require_approval_device(&headers, &key, &conn).await?;
    let Some(connection) = ReinsConnection::find_by_uuid_and_user(id, &headers.user.uuid, &conn).await else {
        return Err(not_found());
    };
    connection.delete(&conn).await.map_err(|e| internal(&e))?;
    Ok(Status::NoContent)
}

/// A9: the devices signed in to the account (phones, Bitwarden apps, the web vault), the approval device marked.
#[get("/reins/api/devices")]
async fn get_devices(headers: Headers, key: DeviceKey, conn: DbConn) -> PhoneResult<Json<Devices>> {
    require_approval_device(&headers, &key, &conn).await?;
    let approval = ReinsDevice::find_by_user(&headers.user.uuid, &conn).await;
    let mut devices: Vec<DeviceInfo> = Device::find_by_user(&headers.user.uuid, &conn)
        .await
        .into_iter()
        .map(|d| DeviceInfo {
            approval: approval.as_ref().is_some_and(|a| a.device_uuid == d.uuid),
            this_device: d.uuid == headers.device.uuid,
            id: d.uuid.to_string(),
            name: d.name.chars().filter(|c| !c.is_control()).take(100).collect(),
            kind: d.atype,
            created_at: d.created_at.and_utc().timestamp(),
            last_seen_at: d.updated_at.and_utc().timestamp(),
        })
        .collect();
    devices.sort_by_key(|d| (!d.this_device, !d.approval, std::cmp::Reverse(d.last_seen_at)));
    Ok(Json(Devices {
        devices,
    }))
}

/// A10: signs another device of the account out (a lost phone), from the approval device and with proof typed now (the
/// recovery code's or the master password's hash, counted like a takeover proof). In one transaction its Vaultwarden
/// device goes (its access token is refused at once, its refresh token is gone), with its SSO session mappings and any
/// approval role still naming it, and the sign-out is recorded: a sign-in with that device id is refused from then on
/// ([`crate::api::identity`]). Its WorkOS sessions are ended at WorkOS too (so the sign-in cookie it kept no longer
/// signs anyone in), its push registration is dropped, and its join requests are forgotten. The device asking signs
/// itself out with A-logout instead.
#[delete("/reins/api/devices/<id>", data = "<data>")]
async fn delete_device(
    id: &str,
    data: Data<'_>,
    headers: Headers,
    key: DeviceKey,
    ip: ClientIp,
    conn: DbConn,
) -> PhoneResult<Status> {
    require_approval_device(&headers, &key, &conn).await?;
    let body: DeviceSignOut = serde_json::from_slice(&read_body_limited(data, 4096).await?)
        .map_err(|e| bad_request(format!("invalid body: {e}")))?;
    let id = DeviceId::from(id.to_owned());
    let Some(target) = Device::find_by_uuid_and_user(&id, &headers.user.uuid, &conn).await else {
        return Err(api_err(Status::NotFound, codes::UNKNOWN_DEVICE, "The account has no device with that id"));
    };
    // The row found, not the id given: under MySQL's case-insensitive lookup another spelling finds this phone's row.
    if target.uuid == headers.device.uuid {
        return Err(bad_request("This is the device asking; sign it out with Sign out instead"));
    }
    check_proof(&headers, &body.master_password_hash)?;
    let user = user_key(&headers);
    info!("Reins: device {} of user {user} signed out from device {} ({})", target.uuid, headers.device.uuid, ip.ip);
    let signout = ReinsDeviceSignout {
        user_uuid: headers.user.uuid.clone(),
        device_uuid: target.uuid.clone(),
        signed_out_at: now_unix(),
        by_device_name: super::pairing::sanitize_display(&headers.device.name, 100),
    };
    // First, so that no sign-in with this id starts while the rest happens.
    signout.record(&conn).await.map_err(|e| internal(&e))?;
    if crate::sso_workos::enabled() {
        let sessions =
            ReinsDeviceSignout::sessions_of(&headers.user.uuid, &target.uuid, &conn).await.map_err(|e| internal(&e))?;
        for session in sessions {
            // Not ended at WorkOS: nothing more is done, and the user is told to try again (the id stays refused).
            if let Err(e) = crate::sso_workos::revoke_session(&session).await {
                warn!("Reins: ending a signed-out device's WorkOS session failed: {e:?}");
                return Err(api_err(
                    Status::BadGateway,
                    codes::INTERNAL,
                    "WorkOS did not end that phone's sign-in, so it is not signed out yet. Try again in a minute.",
                ));
            }
        }
    }
    if crate::CONFIG.push_enabled()
        && let Err(e) = crate::api::unregister_push_device(target.push_uuid.as_ref()).await
    {
        warn!("Reins: unregistering a signed-out device from push failed: {e:?}");
    }
    signout.sign_out(&conn).await.map_err(|e| internal(&e))?;
    HUB.joins.forget_device(&user, &target.uuid.to_string());
    Ok(Status::NoContent)
}

/// A new recovery code, from the approval device: the account secret's password hash and the vault key's wrapping are
/// replaced, so the old code no longer proves anything (taking the approval role, signing a device out, unlocking a new
/// phone). The vault passkeys' copies, sealed to the old secret, are dropped, and pending "add another phone" requests
/// with them. The vault key itself, and so the vault and the encrypted account state, stay as they are.
#[post("/reins/api/account/secret", data = "<data>")]
async fn post_account_secret(
    data: Data<'_>,
    mut headers: Headers,
    key: DeviceKey,
    ip: ClientIp,
    conn: DbConn,
) -> PhoneResult<Status> {
    require_approval_device(&headers, &key, &conn).await?;
    let body: AccountSecretRotation = serde_json::from_slice(&read_body_limited(data, 8192).await?)
        .map_err(|e| bad_request(format!("invalid body: {e}")))?;
    let well_formed = |s: &str| !s.is_empty() && s.len() <= 1024 && !s.chars().any(char::is_control);
    if !well_formed(&body.new_master_password_hash) || !well_formed(&body.key) || !body.key.starts_with("2.") {
        return Err(bad_request("the new hash and key are required"));
    }
    check_proof(&headers, &body.master_password_hash)?;
    let user = user_key(&headers);
    headers
        .user
        .set_password(&body.new_master_password_hash, Some(body.key), false, None, &conn)
        .await
        .map_err(|e| internal(&e))?;
    headers.user.save(&conn).await.map_err(|e| internal(&e))?;
    crate::db::models::ReinsVaultPasskey::delete_all(&headers.user.uuid, &conn).await.map_err(|e| internal(&e))?;
    HUB.joins.forget_user(&user);
    info!("Reins: the account secret of user {user} was replaced from device {} ({})", headers.device.uuid, ip.ip);
    Ok(Status::NoContent)
}

/// Proof typed now that the caller may sign a device out: the master password hash of the account secret or of the
/// master password. Wrong hashes count against the account, with the takeover proofs.
fn check_proof(headers: &Headers, hash: &str) -> PhoneResult<()> {
    let user = user_key(headers);
    if let Err(wait) = super::limits::DEVICE_PROOFS.check(&user) {
        return Err(rate_limited("too many wrong recovery codes or passwords for this account", wait));
    }
    if !hash.is_empty() && headers.user.check_valid_password(hash) {
        return Ok(());
    }
    super::limits::DEVICE_PROOFS.fail(&user);
    warn!("Reins: a wrong proof to sign a device out of user {user}");
    Err(api_err(Status::Forbidden, codes::WRONG_PROOF, "That is neither the recovery code nor the master password"))
}

#[catch(401)]
fn unauthorized() -> Json<ApiError> {
    Json(ApiError::new(codes::UNAUTHORIZED, "Missing or invalid Vaultwarden access token"))
}

#[catch(404)]
fn not_found_catcher() -> Json<ApiError> {
    Json(ApiError::new(codes::NOT_FOUND, "No such Reins API endpoint"))
}

#[cfg(test)]
mod tests {
    use reins_proto::relay::{RelayOutcome, RelayResponse};

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
    fn apns_tokens_must_be_64_to_200_hex_digits_and_are_lowercased() {
        let hex = "0123456789abcdef".repeat(4);
        for prefix in ["apns:", "apns-sandbox:"] {
            let upper = format!(" {prefix}{} \n", hex.to_ascii_uppercase());
            assert_eq!(normalize_fcm_token(Some(upper)), Ok(Some(format!("{prefix}{hex}"))));
            assert!(normalize_fcm_token(Some(format!("{prefix}{}", &hex[2..]))).unwrap_err().contains("64 to 200 hex"));
            // A simulator's token is 80 bytes; Apple allows for up to 100.
            let simulator = "ab".repeat(80);
            assert_eq!(
                normalize_fcm_token(Some(format!("{prefix}{simulator}"))),
                Ok(Some(format!("{prefix}{simulator}")))
            );
            assert!(normalize_fcm_token(Some(format!("{prefix}{}", "ab".repeat(101)))).is_err());
            assert!(normalize_fcm_token(Some(format!("{prefix}{hex}0"))).is_err());
            assert!(normalize_fcm_token(Some(format!("{prefix}{}zz", &hex[2..]))).is_err());
            assert!(normalize_fcm_token(Some(format!("{prefix}{}:{}", &hex[..32], &hex[33..]))).is_err());
            assert!(normalize_fcm_token(Some(prefix.to_owned())).is_err());
        }
        // Not an APNs prefix: an FCM token, kept as is.
        let fcm = format!("APNS:{hex}");
        assert_eq!(normalize_fcm_token(Some(fcm.clone())), Ok(Some(fcm)));
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
