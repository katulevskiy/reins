//! Ticket endpoints (Decisions v2, 03):
//!
//! - the approval device: `GET /reins/api/tickets` (its open and approved tickets, after a reinstall) and
//!   `POST /reins/api/tickets/<id>/events` (its signed transitions: Later, "Why?", a denial with a note, an approval
//!   to collect later, stale, expired);
//! - the desktop app: `GET /reins/desktop/tickets` and `POST /reins/desktop/tickets/<ticket>/stale` (a push whose
//!   remote moved);
//! - any connection: `GET`, `PUT`, `DELETE /reins/agent/callback` (its webhook).
//!
//! Every lookup is scoped: the device to its account, a connection to its own tickets.

use data_encoding::BASE64URL_NOPAD;
use reins_proto::{
    device::codes,
    ticket::{
        self, DesktopTicket, DesktopTickets, TicketEvents, TicketItem, TicketState, TicketStatus, request_of, ticket_of,
    },
};
use rocket::{Data, Route, State, http::Status, serde::json::Json};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{
    HUB,
    desktop_routes::DesktopResponse,
    device_api::{
        DeviceKey, PhoneResult, api_err, not_found, parse_versioned, read_body_limited, require_approval_device,
        user_key,
    },
    mcp_routes::{McpRequest, authenticate, check_connection_rate},
    now_unix,
    tickets::{self, ParkError, TicketError},
    webhooks,
};
use crate::{
    auth::Headers,
    db::{
        DbConn, DbPool,
        models::{ReinsCallback, ReinsConnection, ReinsTicket},
    },
};

/// Largest events body (a few signed events with a short note).
const MAX_EVENTS_BODY: u64 = 64 * 1024;
/// Closed tickets the desktop app and `reins_tickets` still list.
pub const RECENT_SECS: i64 = 86_400;

pub fn routes() -> Vec<Route> {
    routes![
        get_tickets,
        post_events,
        get_desktop_tickets,
        post_stale,
        post_pushed,
        get_callback,
        put_callback,
        delete_callback
    ]
}

/// A 32-byte public key as base64url without padding.
pub fn is_key(raw: &str) -> bool {
    BASE64URL_NOPAD.decode(raw.as_bytes()).is_ok_and(|k| k.len() == 32)
}

pub fn ticket_error(e: &TicketError) -> rocket::response::status::Custom<Json<reins_proto::device::ApiError>> {
    match e {
        TicketError::NotFound => not_found(),
        TicketError::BadSignature => {
            api_err(Status::Forbidden, codes::BAD_SIGNATURE, "The ticket event is not signed by this device")
        }
        TicketError::BadTransition => {
            api_err(Status::Conflict, codes::ALREADY_ANSWERED, "The ticket is not where this event can move it from")
        }
        TicketError::Db => api_err(Status::InternalServerError, "internal", "Could not save the ticket"),
    }
}

#[derive(Serialize)]
struct PhoneTickets {
    tickets: Vec<TicketItem>,
}

/// The account's open and approved tickets, sealed as stored: what the device rebuilds its inbox from.
#[get("/reins/api/tickets")]
async fn get_tickets(headers: Headers, key: DeviceKey, conn: DbConn) -> PhoneResult<Json<PhoneTickets>> {
    require_approval_device(&headers, &key, &conn).await?;
    let rows = ReinsTicket::list_for_user(&user_key(&headers), &conn).await;
    Ok(Json(PhoneTickets {
        tickets: rows.iter().filter_map(|r| tickets::item(r, false)).collect(),
    }))
}

/// The device's signed transitions for request or ticket `id`. Later on a request still Instant parks it first.
#[post("/reins/api/tickets/<id>/events", data = "<data>")]
async fn post_events(
    id: &str,
    data: Data<'_>,
    headers: Headers,
    key: DeviceKey,
    conn: DbConn,
    pool: &State<DbPool>,
) -> PhoneResult<Json<TicketStatus>> {
    require_approval_device(&headers, &key, &conn).await?;
    let user = user_key(&headers);
    let body: TicketEvents = parse_versioned(&read_body_limited(data, MAX_EVENTS_BODY).await?)?;
    let request_id = request_of(id).ok_or_else(not_found)?;
    // Later, or a "Why?", on a request still Instant: it becomes a ticket first.
    let later =
        body.events.first().is_some_and(|e| matches!(e.event.state, TicketState::Deferred | TicketState::NeedsInfo));
    if later && let Some(request) = HUB.relay.unanswered(&user, &request_id) {
        let parked = tickets::park(&user, &request, TicketState::Reviewing, body.window_secs, &conn, pool.inner())
            .await
            .map_err(|e| match e {
                ParkError::NoBoxKey => api_err(Status::Conflict, "cannot_defer", "This device published no box key"),
                ParkError::TooMany => {
                    api_err(Status::Conflict, "too_many", "This connection has too many open tickets")
                }
                ParkError::PhoneHasIt => api_err(Status::Conflict, codes::ALREADY_ANSWERED, "It was answered already"),
                ParkError::Db => api_err(Status::InternalServerError, "internal", "Could not save the ticket"),
            })?;
        if parked.id != request_id.0 {
            // The same request was already waiting as a ticket: this one joined it (the phone drops its card).
            return Ok(Json(TicketStatus {
                ticket: ticket_of(&reins_proto::ids::RequestId(parked.id.clone())),
                state: TicketState::parse(&parked.state).unwrap_or(TicketState::Deferred),
                deadline: parked.deadline,
            }));
        }
    }
    let mut row = ReinsTicket::find_for_user(&request_id.0, &user, &conn).await.ok_or_else(not_found)?;
    tickets::apply_signed(&mut row, &body.events, body.window_secs, body.on_expiry, &conn)
        .await
        .map_err(|e| ticket_error(&e))?;
    let state = TicketState::parse(&row.state).unwrap_or(TicketState::Unknown);
    // The device knows what it just did: this version needs no delivery.
    if !row.delivered
        && let Err(e) = ReinsTicket::mark_delivered(vec![row.id.clone()], &conn).await
    {
        warn!("Reins tickets: {e:?}");
    }
    Ok(Json(TicketStatus {
        ticket: ticket_of(&request_id),
        state,
        deadline: row.deadline,
    }))
}

/// A ticket as a connection sees it: never its body.
pub fn desktop_view(row: &ReinsTicket) -> Option<DesktopTicket> {
    Some(DesktopTicket {
        ticket: ticket_of(&reins_proto::ids::RequestId(row.id.clone())),
        state: TicketState::parse(&row.state)?,
        deadline: row.deadline,
        note: row.note.clone(),
        question: row.question.clone(),
        stale: row.stale.clone(),
    })
}

async fn connection_of(request: &McpRequest, conn: &DbConn) -> Result<ReinsConnection, DesktopResponse> {
    if request.origin.is_some() {
        return Err(DesktopResponse::error(Status::Forbidden, "forbidden_origin", "Browsers may not call this API."));
    }
    let connection =
        authenticate(request.authorization.as_deref(), conn).await.map_err(DesktopResponse::unauthorized)?;
    check_connection_rate(&connection)
        .map_err(|wait| DesktopResponse::rate_limited(super::mcp_routes::CONNECTION_REQUESTS_LIMITED, wait))?;
    Ok(connection)
}

/// This connection's open tickets and those closed in the last day.
#[get("/reins/desktop/tickets")]
async fn get_desktop_tickets(request: McpRequest, conn: DbConn) -> DesktopResponse {
    let connection = match connection_of(&request, &conn).await {
        Ok(c) => c,
        Err(r) => return r,
    };
    let rows = ReinsTicket::list_for_connection(&connection.uuid, now_unix() - RECENT_SECS, &conn).await;
    let signing_key = tickets::device_keys(&connection.user_uuid.to_string(), &conn).await.map(|k| k.sign_pub);
    let body = DesktopTickets {
        tickets: rows.iter().filter_map(desktop_view).collect(),
        signing_key,
    };
    DesktopResponse::json(Status::Ok, serde_json::to_value(body).unwrap_or(Value::Null))
}

#[derive(Deserialize)]
struct StaleBody {
    diff: String,
}

/// The desktop app found the world moved since the approval (a push whose remote ref moved): the ticket is Stale, and
/// the phone asks again with the difference.
#[post("/reins/desktop/tickets/<id>/stale", data = "<data>")]
async fn post_stale(
    id: &str,
    data: Data<'_>,
    request: McpRequest,
    conn: DbConn,
    pool: &State<DbPool>,
) -> DesktopResponse {
    let connection = match connection_of(&request, &conn).await {
        Ok(c) => c,
        Err(r) => return r,
    };
    let Some(request_id) = request_of(id) else {
        return DesktopResponse::not_found();
    };
    let Ok(body) = data.open(rocket::data::ToByteUnit::kibibytes(8)).into_bytes().await else {
        return DesktopResponse::error(Status::BadRequest, "invalid_request", "Unreadable body.");
    };
    let Ok(stale) = serde_json::from_slice::<StaleBody>(&body) else {
        return DesktopResponse::error(Status::BadRequest, "invalid_request", "Expected {\"diff\": \"...\"}.");
    };
    let Some(diff) = ticket::clean_text(&stale.diff, ticket::MAX_NOTE_CHARS) else {
        return DesktopResponse::error(Status::BadRequest, "invalid_request", "The diff is empty.");
    };
    if !HUB.is_desktop(&connection.user_uuid.to_string(), &connection.uuid) {
        return DesktopResponse::error(
            Status::Forbidden,
            "not_desktop",
            "Only the desktop app reports a moved remote.",
        );
    }
    let Some(mut row) = ReinsTicket::find_for_connection(&request_id.0, &connection.uuid, &conn).await else {
        return DesktopResponse::not_found();
    };
    let text = diff.clone();
    let now = now_unix();
    // Asked again with the difference: a new window, and the approval it had is spent.
    let restart = |t: &mut ReinsTicket| {
        t.stale = Some(text);
        t.approved_at = None;
        t.deferred_at = now;
        t.deadline = now.saturating_add(t.window_secs);
    };
    if !tickets::transition(&mut row, TicketState::Stale, "desktop", &json!({"stale": diff}), restart, &conn).await {
        return DesktopResponse::error(Status::Conflict, "conflict", "The ticket is not approved.");
    }
    tickets::wake_phone(&row.user_uuid, &row.id, &conn, pool.inner()).await;
    DesktopResponse::json(Status::Ok, json!({"ticket": ticket_of(&request_id), "state": "stale"}))
}

/// The desktop app pushed a parked push it was approved for: the ticket is done (its approval spent).
#[post("/reins/desktop/tickets/<id>/pushed")]
async fn post_pushed(id: &str, request: McpRequest, conn: DbConn) -> DesktopResponse {
    let connection = match connection_of(&request, &conn).await {
        Ok(c) => c,
        Err(r) => return r,
    };
    if !HUB.is_desktop(&connection.user_uuid.to_string(), &connection.uuid) {
        return DesktopResponse::error(Status::Forbidden, "not_desktop", "Only the desktop app reports a push.");
    }
    let Some(request_id) = request_of(id) else {
        return DesktopResponse::not_found();
    };
    let Some(mut row) = ReinsTicket::find_for_connection(&request_id.0, &connection.uuid, &conn).await else {
        return DesktopResponse::not_found();
    };
    if row.finish != "report"
        || !tickets::transition(&mut row, TicketState::Done, "desktop", &json!({"reason": "pushed"}), |_| {}, &conn)
            .await
    {
        return DesktopResponse::error(Status::Conflict, "conflict", "The ticket is not an approved push.");
    }
    DesktopResponse::json(Status::Ok, json!({"ticket": ticket_of(&request_id), "state": "done"}))
}

async fn bearer(request: &McpRequest, conn: &DbConn) -> Result<ReinsConnection, DesktopResponse> {
    connection_of(request, conn).await
}

/// This connection's webhook and the key its events are signed with (the approval device's).
#[get("/reins/agent/callback")]
async fn get_callback(request: McpRequest, conn: DbConn) -> DesktopResponse {
    let connection = match bearer(&request, &conn).await {
        Ok(c) => c,
        Err(r) => return r,
    };
    let url = ReinsCallback::find(&connection.uuid, &conn).await.map(|c| c.url);
    let key = tickets::device_keys(&connection.user_uuid.to_string(), &conn).await.map(|k| k.sign_pub);
    DesktopResponse::json(Status::Ok, key_answer(url.as_deref(), key.as_deref()))
}

#[derive(Deserialize)]
struct CallbackBody {
    url: String,
}

/// Registers this connection's webhook after the URL echoes a challenge.
#[put("/reins/agent/callback", data = "<data>")]
async fn put_callback(data: Data<'_>, request: McpRequest, conn: DbConn, pool: &State<DbPool>) -> DesktopResponse {
    let connection = match bearer(&request, &conn).await {
        Ok(c) => c,
        Err(r) => return r,
    };
    let Ok(body) = data.open(rocket::data::ToByteUnit::kibibytes(4)).into_bytes().await else {
        return DesktopResponse::error(Status::BadRequest, "invalid_request", "Unreadable body.");
    };
    let Ok(body) = serde_json::from_slice::<CallbackBody>(&body) else {
        return DesktopResponse::error(Status::BadRequest, "invalid_request", "Expected {\"url\": \"https://...\"}.");
    };
    if body.url.len() > 2048 {
        return DesktopResponse::error(Status::BadRequest, "invalid_request", "The URL is too long.");
    }
    // Never hold a pooled connection while the URL answers.
    drop(conn);
    if let Err(e) = webhooks::verify_url(&body.url).await {
        return DesktopResponse::error(Status::BadRequest, "callback_refused", &e);
    }
    let Ok(conn) = pool.get().await else {
        return DesktopResponse::error(Status::ServiceUnavailable, "busy", "Try again in a minute.");
    };
    let row = ReinsCallback {
        connection_uuid: connection.uuid.clone(),
        url: body.url,
        verified_at: now_unix(),
    };
    match row.save(&conn).await {
        Ok(()) => {
            let key = tickets::device_keys(&connection.user_uuid.to_string(), &conn).await.map(|k| k.sign_pub);
            DesktopResponse::json(Status::Ok, key_answer(Some(&row.url), key.as_deref()))
        }
        Err(_) => DesktopResponse::error(Status::InternalServerError, "internal", "Could not save the webhook."),
    }
}

/// A webhook and the key its events are signed with, with the twelve digits the phone shows for it (pin the key; a
/// changed one means compare the digits on the phone again).
fn key_answer(url: Option<&str>, key: Option<&str>) -> Value {
    let fingerprint = key.and_then(ticket::signing_key_fingerprint);
    json!({"url": url, "signing_key": key, "signing_key_fingerprint": fingerprint})
}

#[delete("/reins/agent/callback")]
async fn delete_callback(request: McpRequest, conn: DbConn) -> DesktopResponse {
    let connection = match bearer(&request, &conn).await {
        Ok(c) => c,
        Err(r) => return r,
    };
    match ReinsCallback::delete(&connection.uuid, &conn).await {
        Ok(()) => DesktopResponse::json(Status::Ok, json!({"url": null})),
        Err(_) => DesktopResponse::error(Status::InternalServerError, "internal", "Could not delete the webhook."),
    }
}

/// Keys a device publishes on registration, when both are well-formed.
pub fn published_keys(sign_pub: Option<&str>, box_pub: Option<&str>) -> Option<(String, String)> {
    match (sign_pub, box_pub) {
        (Some(s), Some(b)) if is_key(s) && is_key(b) => Some((s.to_owned(), b.to_owned())),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_must_be_32_byte_base64url() {
        let key = BASE64URL_NOPAD.encode(&[7u8; 32]);
        assert!(is_key(&key));
        assert!(!is_key(&BASE64URL_NOPAD.encode(&[7u8; 31])));
        assert!(!is_key("not base64!"));
        assert!(!is_key(&data_encoding::BASE64.encode(&[7u8; 32])), "padding or + / are not base64url");
        assert_eq!(published_keys(Some(&key), Some(&key)), Some((key.clone(), key.clone())));
        assert_eq!(published_keys(Some(&key), None), None);
        assert_eq!(published_keys(Some("x"), Some(&key)), None);
    }

    #[test]
    fn a_connection_sees_its_ticket_without_the_body() {
        let row = ReinsTicket {
            id: "r1".to_owned(),
            user_uuid: "u".to_owned(),
            connection_uuid: "c".to_owned(),
            connection_label: "Claude".to_owned(),
            state: "denied".to_owned(),
            digest: "d".to_owned(),
            execution: "on_approve".to_owned(),
            on_expiry: "deny".to_owned(),
            finish: "run".to_owned(),
            created_at: 1,
            deferred_at: 1,
            deadline: 9,
            window_secs: 8,
            escalated: false,
            approved_at: None,
            valid_secs: 1,
            sealed_body: "SEALED-BODY".to_owned(),
            question: None,
            sealed_reply: Some("SEALED-REPLY".to_owned()),
            note: Some("use staging".to_owned()),
            result: None,
            stale: None,
            delivered: true,
            updated_at: 1,
        };
        let view = serde_json::to_string(&desktop_view(&row).unwrap()).unwrap();
        assert!(view.contains("\"ticket\":\"rt_r1\"") && view.contains("use staging"));
        assert!(!view.contains("SEALED"), "{view}");
    }
}
