//! WorkOS user lifecycle into Reins (with WorkOS AuthKit as the SSO provider).
//!
//! WorkOS owns the identity: the email, whether the user still exists, which sessions are live. A background task
//! reads the WorkOS Events API (`GET /events?events=user.updated,user.deleted,session.revoked&after=<cursor>`) every
//! `REINS_WORKOS_SYNC_SECS`, and at once when a signed webhook delivery arrives (`POST
//! /reins/workos/webhook`, `REINS_WORKOS_WEBHOOK_SECRET`). The webhook only wakes the task: its body is not
//! trusted, the events read from the API are. The cursor (the id of the last applied event) is kept in
//! `reins_settings`, so a restart continues where it stopped.
//!
//! What each event does to the account the WorkOS user signed in to (found through `sso_users`):
//!
//! - `user.updated`: a new, verified email becomes the account's email (an unverified one waits for the event that
//!   verifies it; an email another account has is left alone and logged). The name follows.
//! - `user.deleted`: the account is deleted with its vault, devices, approval device, AI and desktop connections and
//!   their refresh tokens.
//! - `session.revoked`: the device that signed in with that session is signed out (its Vaultwarden device goes, so
//!   its tokens stop working at once). AI connections stay: they are the phone's to revoke.

use std::{sync::LazyLock, time::Duration};

use rocket::{Route, data::ToByteUnit, http::Status};
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::Notify;

use crate::{
    CONFIG,
    db::{
        DbConn, DbPool,
        models::{ReinsSetting, ReinsSsoSession, SsoUser, User, reins_workos},
    },
    sso_workos,
};

/// The events the sync applies.
pub const EVENTS: &str = "user.updated,user.deleted,session.revoked";
/// Events per page (the Events API's largest page).
const PAGE: usize = 100;
/// `reins_settings` name of the cursor.
const CURSOR: &str = "workos.events.after";
/// A webhook delivery older (or newer) than this is refused.
const WEBHOOK_TOLERANCE_MS: i64 = 5 * 60 * 1000;
/// Largest webhook body read.
const WEBHOOK_MAX_BYTES: u64 = 256 * 1024;

/// Wakes the sync before its next tick (a webhook delivery).
static NUDGE: LazyLock<Notify> = LazyLock::new(Notify::new);

/// Whether the sync runs: SSO through WorkOS and a non-zero interval.
pub fn enabled() -> bool {
    CONFIG.sso_enabled() && sso_workos::enabled() && CONFIG.reins_workos_sync_secs() > 0
}

/// Starts the background task (call once, inside the runtime).
pub fn spawn(pool: DbPool) {
    if !enabled() {
        return;
    }
    let every = Duration::from_secs(CONFIG.reins_workos_sync_secs());
    info!("WorkOS sync: reading events every {}s", every.as_secs());
    tokio::spawn(async move {
        loop {
            match pool.get().await {
                Ok(conn) => match sync_once(&conn).await {
                    Ok(0) => {}
                    Ok(n) => info!("WorkOS sync: applied {n} events"),
                    Err(e) => warn!("WorkOS sync failed: {e}"),
                },
                Err(e) => warn!("WorkOS sync: no database connection: {e:?}"),
            }
            tokio::select! {
                () = tokio::time::sleep(every) => {}
                () = NUDGE.notified() => {}
            }
        }
    });
}

#[derive(Debug, Deserialize)]
pub struct EventsPage {
    pub data: Vec<Event>,
}

#[derive(Debug, Deserialize)]
pub struct Event {
    pub id: String,
    pub event: String,
    #[serde(default)]
    pub data: Value,
}

/// What one event asks of Reins.
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    /// The WorkOS user `user_id` now has this email (verified) and name.
    Update {
        user_id: String,
        email: Option<String>,
        name: Option<String>,
    },
    Delete {
        user_id: String,
    },
    RevokeSession {
        session_id: String,
    },
    Ignore,
}

pub fn plan(event: &Event) -> Action {
    let str_of = |k: &str| event.data.get(k).and_then(Value::as_str).map(str::to_owned);
    match event.event.as_str() {
        "user.updated" => {
            let Some(user_id) = str_of("id") else {
                return Action::Ignore;
            };
            let verified = event.data.get("email_verified").and_then(Value::as_bool) == Some(true);
            let email = str_of("email").filter(|_| verified).map(|e| e.trim().to_lowercase());
            let name = [str_of("first_name"), str_of("last_name")]
                .into_iter()
                .flatten()
                .filter(|s| !s.trim().is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            Action::Update {
                user_id,
                email,
                name: (!name.is_empty()).then_some(name),
            }
        }
        "user.deleted" => str_of("id").map_or(Action::Ignore, |user_id| Action::Delete {
            user_id,
        }),
        "session.revoked" => str_of("id").map_or(Action::Ignore, |session_id| Action::RevokeSession {
            session_id,
        }),
        _ => Action::Ignore,
    }
}

async fn fetch_page(after: Option<&str>) -> Result<EventsPage, String> {
    let base = sso_workos::api_base().map_err(|e| e.to_string())?;
    let client = sso_workos::http_client().map_err(|e| e.to_string())?;
    let mut url = url::Url::parse(&format!("{base}/events")).map_err(|e| format!("invalid WorkOS URL: {e}"))?;
    url.query_pairs_mut().append_pair("events", EVENTS).append_pair("limit", &PAGE.to_string());
    if let Some(after) = after {
        url.query_pairs_mut().append_pair("after", after);
    }
    let response = client
        .get(url)
        .bearer_auth(sso_workos::api_key())
        .send()
        .await
        .map_err(|e| format!("could not reach WorkOS: {e}"))?;
    let status = response.status();
    let body = response.text().await.map_err(|e| format!("could not read the WorkOS events: {e}"))?;
    if !status.is_success() {
        let message = serde_json::from_str::<Value>(&body)
            .ok()
            .and_then(|v| v.get("message").and_then(Value::as_str).map(str::to_owned))
            .unwrap_or_default();
        return Err(format!("WorkOS answered {status} {message}"));
    }
    serde_json::from_str(&body).map_err(|e| format!("unexpected WorkOS events answer: {e}"))
}

/// Reads and applies every event after the cursor; returns how many there were. The cursor moves after each applied
/// event, so a failure retries from the event that failed.
pub async fn sync_once(conn: &DbConn) -> Result<usize, String> {
    let mut cursor = ReinsSetting::get(CURSOR, conn).await;
    let mut applied = 0;
    loop {
        let page = fetch_page(cursor.as_deref()).await?;
        let full = page.data.len() >= PAGE;
        for event in page.data {
            apply(&plan(&event), conn).await.map_err(|e| format!("event {} ({}): {e}", event.id, event.event))?;
            ReinsSetting::set(CURSOR, &event.id, conn).await.map_err(|e| e.to_string())?;
            cursor = Some(event.id);
            applied += 1;
        }
        if !full {
            return Ok(applied);
        }
    }
}

async fn reins_user(workos_user_id: &str, conn: &DbConn) -> Option<User> {
    SsoUser::find_by_identifier(&sso_workos::identifier(workos_user_id), conn).await.map(|(user, _)| user)
}

async fn apply(action: &Action, conn: &DbConn) -> Result<(), String> {
    match action {
        Action::Ignore => Ok(()),
        Action::Update {
            user_id,
            email,
            name,
        } => {
            let Some(mut user) = reins_user(user_id, conn).await else {
                return Ok(());
            };
            let mut changed = false;
            if let Some(email) = email
                && *email != user.email
            {
                if User::find_by_mail(email, conn).await.is_some() {
                    error!(
                        "WorkOS sync: {user_id} changed its email to {email}, which another account has; kept {}",
                        user.email
                    );
                } else {
                    info!("WorkOS sync: account {} changes its email from {} to {email}", user.uuid, user.email);
                    user.email.clone_from(email);
                    changed = true;
                }
            }
            if let Some(name) = name
                && *name != user.name
            {
                user.name.clone_from(name);
                changed = true;
            }
            if changed {
                user.save(conn).await.map_err(|e| e.to_string())?;
            }
            Ok(())
        }
        Action::Delete {
            user_id,
        } => {
            let Some(user) = reins_user(user_id, conn).await else {
                return Ok(());
            };
            info!("WorkOS sync: {user_id} was deleted; deleting account {}", user.uuid);
            let uuid = user.uuid.clone();
            reins_workos::delete_reins_data(&uuid, conn).await.map_err(|e| e.to_string())?;
            if let Err(e) = user.delete(conn).await {
                // The last owner of an organization: Vaultwarden refuses; the admin has to step in.
                error!("WorkOS sync: could not delete account {uuid}: {e}");
            }
            super::HUB.forget_user(uuid.as_ref());
            Ok(())
        }
        Action::RevokeSession {
            session_id,
        } => {
            let Some(session) = ReinsSsoSession::take(session_id, conn).await else {
                return Ok(());
            };
            info!("WorkOS sync: session {session_id} was revoked; signing out device {}", session.device_uuid);
            reins_workos::sign_out_device(&session.user_uuid, &session.device_uuid, conn)
                .await
                .map_err(|e| e.to_string())
        }
    }
}

/// Checks a `WorkOS-Signature` header (`t=<unix ms>, v1=<hex HMAC-SHA256 of "<t>.<body>">`).
pub fn signature_ok(header: &str, body: &[u8], secret: &str, now_ms: i64) -> bool {
    let mut t = None;
    let mut v1 = None;
    for part in header.split(',') {
        match part.trim().split_once('=') {
            Some(("t", value)) => t = Some(value.trim()),
            Some(("v1", value)) => v1 = Some(value.trim()),
            _ => {}
        }
    }
    let (Some(t), Some(v1)) = (t, v1) else {
        return false;
    };
    let Ok(at) = t.parse::<i64>() else {
        return false;
    };
    if (now_ms - at).abs() > WEBHOOK_TOLERANCE_MS {
        return false;
    }
    let Ok(sig) = data_encoding::HEXLOWER_PERMISSIVE.decode(v1.as_bytes()) else {
        return false;
    };
    let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, secret.as_bytes());
    let mut signed = Vec::with_capacity(t.len() + 1 + body.len());
    signed.extend_from_slice(t.as_bytes());
    signed.push(b'.');
    signed.extend_from_slice(body);
    ring::hmac::verify(&key, &signed, &sig).is_ok()
}

pub struct Signature(String);

#[rocket::async_trait]
impl<'r> rocket::request::FromRequest<'r> for Signature {
    type Error = ();

    async fn from_request(req: &'r rocket::Request<'_>) -> rocket::request::Outcome<Self, ()> {
        match req.headers().get_one("WorkOS-Signature") {
            Some(h) => rocket::request::Outcome::Success(Self(h.to_owned())),
            None => rocket::request::Outcome::Error((Status::Unauthorized, ())),
        }
    }
}

/// A WorkOS webhook delivery: when its signature holds, the sync runs now.
#[post("/reins/workos/webhook", data = "<body>")]
async fn webhook(signature: Signature, body: rocket::Data<'_>) -> Status {
    let Some(secret) = CONFIG.reins_workos_webhook_secret().filter(|s| !s.is_empty()) else {
        return Status::NotFound;
    };
    let Ok(bytes) = body.open(WEBHOOK_MAX_BYTES.bytes()).into_bytes().await else {
        return Status::BadRequest;
    };
    if !bytes.is_complete() {
        return Status::PayloadTooLarge;
    }
    if !signature_ok(&signature.0, &bytes, &secret, chrono::Utc::now().timestamp_millis()) {
        return Status::Unauthorized;
    }
    NUDGE.notify_one();
    Status::Ok
}

pub fn routes() -> Vec<Route> {
    if enabled() {
        routes![webhook]
    } else {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn event(kind: &str, data: Value) -> Event {
        Event {
            id: "event_1".to_owned(),
            event: kind.to_owned(),
            data,
        }
    }

    #[test]
    fn email_changes_apply_once_verified() {
        let data = json!({"id": "user_1", "email": "New@Example.com", "email_verified": false, "first_name": "Ada"});
        assert_eq!(
            plan(&event("user.updated", data)),
            Action::Update {
                user_id: "user_1".to_owned(),
                email: None,
                name: Some("Ada".to_owned()),
            }
        );
        let data = json!({"id": "user_1", "email": "New@Example.com", "email_verified": true,
                          "first_name": "Ada", "last_name": "Lovelace"});
        assert_eq!(
            plan(&event("user.updated", data)),
            Action::Update {
                user_id: "user_1".to_owned(),
                email: Some("new@example.com".to_owned()),
                name: Some("Ada Lovelace".to_owned()),
            }
        );
    }

    #[test]
    fn deletions_and_revocations_name_their_object() {
        assert_eq!(
            plan(&event("user.deleted", json!({"id": "user_1", "email": "a@b.c"}))),
            Action::Delete {
                user_id: "user_1".to_owned()
            }
        );
        assert_eq!(
            plan(&event("session.revoked", json!({"id": "session_1", "user_id": "user_1"}))),
            Action::RevokeSession {
                session_id: "session_1".to_owned()
            }
        );
        assert_eq!(plan(&event("user.deleted", json!({}))), Action::Ignore);
        assert_eq!(plan(&event("session.created", json!({"id": "session_1"}))), Action::Ignore);
    }

    #[test]
    fn events_pages_parse() {
        let page: EventsPage = serde_json::from_value(json!({
            "object": "list",
            "data": [{"object": "event", "id": "event_01", "event": "user.deleted",
                      "data": {"id": "user_1"}, "created_at": "2026-10-02T10:26:09.096Z"}],
            "list_metadata": {"after": null}
        }))
        .unwrap();
        assert_eq!(page.data[0].id, "event_01");
        assert_eq!(
            plan(&page.data[0]),
            Action::Delete {
                user_id: "user_1".to_owned()
            }
        );
    }

    fn sign(secret: &str, t: i64, body: &str) -> String {
        let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, secret.as_bytes());
        let tag = ring::hmac::sign(&key, format!("{t}.{body}").as_bytes());
        format!("t={t}, v1={}", data_encoding::HEXLOWER.encode(tag.as_ref()))
    }

    #[test]
    fn webhook_signatures_are_checked() {
        let now = 1_790_000_000_000;
        let body = r#"{"id":"event_1","event":"user.deleted"}"#;
        let header = sign("whsec", now - 1000, body);
        assert!(signature_ok(&header, body.as_bytes(), "whsec", now));
        assert!(!signature_ok(&header, body.as_bytes(), "other", now));
        assert!(!signature_ok(&header, b"{}", "whsec", now));
        // Too old, or garbled.
        assert!(!signature_ok(&sign("whsec", now - 10 * 60 * 1000, body), body.as_bytes(), "whsec", now));
        assert!(!signature_ok("t=abc, v1=00", body.as_bytes(), "whsec", now));
        assert!(!signature_ok("v1=00", body.as_bytes(), "whsec", now));
    }
}
