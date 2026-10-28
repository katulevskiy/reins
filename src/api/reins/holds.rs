//! The email send hold (Decisions v2, 01 §5, 08 R10): approved emails waiting out their undo window, sealed to the
//! approval device.
//!
//! - the approval device: `POST /reins/api/holds` (hold one), `GET /reins/api/holds` (what waits),
//!   `DELETE /reins/api/holds/<id>` (Undo) and `POST /reins/api/holds/<id>/claim` (the release: one claim, which
//!   deletes the ciphertext);
//! - the connection that asked (the desktop app): `DELETE /reins/desktop/holds/<id>` (Undo), and the agent's
//!   `reins_cancel` ([`cancel_request`]).
//!
//! The server picks the release time on its own clock and wakes the phone then ([`PushKind::Hold`], silent). It can't
//! send anything itself: it has no Gmail token and no key to the ciphertext.

use std::time::Duration;

use reins_proto::{
    hold::{self, HoldClaimed, HoldCreate, HoldCreated, HoldItem, HoldList},
    ids::RequestId,
    pairing::{PushKind, PushMessage},
};
use rocket::{Data, Route, State, http::Status, serde::json::Json};
use serde_json::json;

use super::{
    desktop_routes::DesktopResponse,
    device_api::{
        DeviceKey, PhoneResult, api_err, bad_request, internal, not_found, parse_versioned, read_body_limited,
        require_approval_device, user_key,
    },
    mcp_routes::{McpRequest, authenticate, check_connection_rate},
    now_unix, push,
};
use crate::{
    auth::Headers,
    db::{
        DbConn, DbPool,
        models::{HoldClaim, HoldInserted, ReinsConnection, ReinsDevice, ReinsSendHold, UserId},
    },
};

/// The largest create body: the ciphertext and a few short fields.
const MAX_CREATE_BODY: u64 = hold::MAX_SEALED_CHARS as u64 + 4096;
/// How often the sweeper looks for holds due (a hold also wakes the phone on its own timer).
const SWEEP_EVERY: Duration = Duration::from_secs(5);

pub fn routes() -> Vec<Route> {
    routes![create, list, undo, claim, desktop_undo]
}

fn too_many(message: &str) -> rocket::response::status::Custom<Json<reins_proto::device::ApiError>> {
    api_err(Status::Conflict, "too_many_holds", message)
}

/// Holds a send: the phone's sealed email and how long to hold it. The release time is the server's.
#[post("/reins/api/holds", data = "<data>")]
async fn create(
    data: Data<'_>,
    headers: Headers,
    key: DeviceKey,
    conn: DbConn,
    pool: &State<DbPool>,
) -> PhoneResult<Json<HoldCreated>> {
    require_approval_device(&headers, &key, &conn).await?;
    let user = user_key(&headers);
    let body: HoldCreate = parse_versioned(&read_body_limited(data, MAX_CREATE_BODY).await?)?;
    if body.secs == 0 || body.secs > hold::MAX_SECS {
        return Err(bad_request(format!("A hold is 1 to {} seconds", hold::MAX_SECS)));
    }
    if body.sealed.is_empty() || body.sealed.len() > hold::MAX_SEALED_CHARS {
        return Err(bad_request("The sealed email is empty or too large"));
    }
    if data_encoding::BASE64URL_NOPAD.decode(body.sealed.as_bytes()).is_err() {
        return Err(bad_request("The sealed email is not base64url"));
    }
    if reins_proto::ticket::request_of(&body.request_id.0).is_none_or(|r| r != body.request_id) {
        return Err(bad_request("Not a request id"));
    }
    // Only one of this account's connections can be named: it may cancel the hold.
    let user_id = UserId::from(user.clone());
    if ReinsConnection::find_by_uuid_and_user(&body.connection_id, &user_id, &conn).await.is_none() {
        return Err(not_found());
    }
    let now = now_unix();
    let row = ReinsSendHold {
        id: uuid::Uuid::new_v4().to_string(),
        user_uuid: user,
        device_uuid: headers.device.uuid.to_string(),
        connection_uuid: body.connection_id,
        request_id: body.request_id.0,
        sealed: body.sealed,
        release_at: now.saturating_add(i64::from(body.secs)),
        created_at: now,
        woken: false,
    };
    match row.insert(&conn).await.map_err(|e| internal(&e))? {
        HoldInserted::Created => {}
        HoldInserted::TooMany => return Err(too_many("This account has too many sends waiting")),
        HoldInserted::TooLarge => return Err(too_many("This account's sends waiting are too large")),
        HoldInserted::Exists => {
            return Err(api_err(Status::Conflict, reins_proto::device::codes::ALREADY_ANSWERED, "This send is held"));
        }
    }
    wake_at(pool.inner().clone(), row.id.clone(), body.secs);
    Ok(Json(HoldCreated {
        id: row.id,
        release_at: row.release_at,
        now,
    }))
}

/// What waits for this account: ids and release times, never a ciphertext.
#[get("/reins/api/holds")]
async fn list(headers: Headers, key: DeviceKey, conn: DbConn) -> PhoneResult<Json<HoldList>> {
    require_approval_device(&headers, &key, &conn).await?;
    let rows = ReinsSendHold::list_for_user(&user_key(&headers), &conn).await;
    Ok(Json(HoldList {
        holds: rows
            .into_iter()
            .map(|r| HoldItem {
                id: r.id,
                request_id: RequestId(r.request_id),
                release_at: r.release_at,
            })
            .collect(),
        now: now_unix(),
    }))
}

/// Undo on the phone: the ciphertext is deleted.
#[delete("/reins/api/holds/<id>")]
async fn undo(id: &str, headers: Headers, key: DeviceKey, conn: DbConn) -> PhoneResult<Status> {
    require_approval_device(&headers, &key, &conn).await?;
    if !hold::valid_id(id) {
        return Err(not_found());
    }
    if ReinsSendHold::cancel_for_user(id, &user_key(&headers), &conn).await.map_err(|e| internal(&e))? {
        Ok(Status::NoContent)
    } else {
        Err(not_found())
    }
}

/// The release: the device that sealed the hold claims it once it is due. The claim deletes the ciphertext, so a second
/// claim (a replay, a race) gets nothing.
#[post("/reins/api/holds/<id>/claim")]
async fn claim(id: &str, headers: Headers, key: DeviceKey, conn: DbConn) -> PhoneResult<Json<HoldClaimed>> {
    require_approval_device(&headers, &key, &conn).await?;
    if !hold::valid_id(id) {
        return Err(not_found());
    }
    let now = now_unix();
    let device = headers.device.uuid.to_string();
    match ReinsSendHold::claim(id, &user_key(&headers), &device, now, &conn).await.map_err(|e| internal(&e))? {
        HoldClaim::Claimed(row) => Ok(Json(HoldClaimed {
            id: row.id,
            request_id: RequestId(row.request_id),
            sealed: row.sealed,
            release_at: row.release_at,
            now,
            late: hold::is_late(row.release_at, now),
        })),
        HoldClaim::NotYet(release_at) => {
            Err(api_err(Status::Conflict, "not_yet", format!("Not released before {release_at}")))
        }
        HoldClaim::NotFound => Err(not_found()),
    }
}

/// Undo from the desktop app: only holds of requests its own connection made.
#[delete("/reins/desktop/holds/<id>")]
async fn desktop_undo(id: &str, request: McpRequest, conn: DbConn, pool: &State<DbPool>) -> DesktopResponse {
    if request.origin.is_some() {
        return DesktopResponse::error(Status::Forbidden, "forbidden_origin", "Browsers may not call this API.");
    }
    let connection = match authenticate(request.authorization.as_deref(), &conn).await {
        Ok(c) => c,
        Err(refused) => return DesktopResponse::unauthorized(refused),
    };
    if let Err(wait) = check_connection_rate(&connection) {
        return DesktopResponse::rate_limited(super::mcp_routes::CONNECTION_REQUESTS_LIMITED, wait);
    }
    if !hold::valid_id(id) {
        return DesktopResponse::not_found();
    }
    match ReinsSendHold::cancel_for_connection(Some(id), None, &connection.uuid, &conn).await {
        Ok(Some((user, _))) => {
            tell_phone(&user, id, &conn, pool.inner()).await;
            DesktopResponse::json(Status::Ok, json!({"hold": id, "status": "undone"}))
        }
        Ok(None) => DesktopResponse::not_found(),
        Err(e) => {
            error!("Reins send hold: {e:?}");
            DesktopResponse::error(Status::InternalServerError, "internal", "Could not undo the send.")
        }
    }
}

/// The agent's `reins_cancel` of request `request_id` while its send is held: `true` when the hold was waiting and is
/// gone now (nothing will be sent).
pub async fn cancel_request(request_id: &str, connection: &ReinsConnection, conn: &DbConn, pool: &DbPool) -> bool {
    match ReinsSendHold::cancel_for_connection(None, Some(request_id), &connection.uuid, conn).await {
        Ok(Some((user, hold))) => {
            tell_phone(&user, &hold, conn, pool).await;
            true
        }
        Ok(None) => false,
        Err(e) => {
            error!("Reins send hold: {e:?}");
            false
        }
    }
}

/// A silent push to `user`'s approval device about hold `id` (due, or undone elsewhere).
async fn tell_phone(user: &str, id: &str, conn: &DbConn, pool: &DbPool) {
    let user_uuid = UserId::from(user.to_owned());
    if let Some(device) = ReinsDevice::find_by_user(&user_uuid, conn).await {
        push::spawn_push(
            pool.clone(),
            user_uuid,
            device.fcm_token,
            PushMessage {
                t: PushKind::Hold,
                id: id.to_owned(),
                p: None,
            },
        );
    }
}

/// Wakes the phone for hold `id` once it is due (`secs` from now); the sweeper covers a restart in between.
fn wake_at(pool: DbPool, id: String, secs: u32) {
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(u64::from(secs))).await;
        let Ok(conn) = pool.get().await else {
            return;
        };
        wake(&id, &conn, &pool).await;
    });
}

/// The release push of one due hold, once.
async fn wake(id: &str, conn: &DbConn, pool: &DbPool) {
    let Some(row) = ReinsSendHold::find(id, conn).await.filter(|r| r.release_at <= now_unix()) else {
        return;
    };
    if ReinsSendHold::mark_woken(&row.id, conn).await {
        tell_phone(&row.user_uuid, &row.id, conn, pool).await;
    }
}

/// Wakes the phones of holds due and not woken (after a restart), and drops holds nobody claimed for a week.
pub async fn sweep(pool: &DbPool, now: i64, purge: bool) {
    let Ok(conn) = pool.get().await else {
        return;
    };
    for row in ReinsSendHold::due(now, &conn).await {
        if ReinsSendHold::mark_woken(&row.id, &conn).await {
            tell_phone(&row.user_uuid, &row.id, &conn, pool).await;
        }
    }
    if purge && let Err(e) = ReinsSendHold::purge(now - hold::KEEP_SECS, &conn).await {
        warn!("Reins send holds: {e:?}");
    }
}

/// Starts the sweeper.
pub fn spawn_sweeper(pool: DbPool) {
    tokio::spawn(async move {
        let mut n: u32 = 0;
        loop {
            tokio::time::sleep(SWEEP_EVERY).await;
            n = n.wrapping_add(1);
            sweep(&pool, now_unix(), n.is_multiple_of(720)).await;
        }
    });
}
