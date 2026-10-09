//! "Add another phone" ([`reins_proto::join`]): a phone of the account that cannot open its keys asks the approval
//! device for the account secret; the server holds the request and relays the sealed answer, which it cannot open.
//!
//! - `POST /reins/api/joins` (any signed-in device of the account but the approval device): parks the request and
//!   wakes the approval device (push `join`). One open request per device (a new one replaces it), a few per account.
//! - `GET /reins/api/joins/<id>`: the asking device gets its [`JoinState`]; the approval device the
//!   [`JoinRequest`].
//! - `POST /reins/api/joins/<id>/response` (the approval device): approves with the sealed secret, or denies.
//!
//! An approval also lets the asking device take the approval role (`PUT /reins/api/device`) without another proof:
//! once, within [`TAKEOVER_TTL`], and only with the device key it asked with ([`JoinHub::take_takeover`]).
//!
//! Everything is in memory for [`ITEM_TTL`] like pairings.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};

use reins_proto::{
    PROTOCOL_VERSION,
    device::codes,
    join::{
        JoinAnswer, JoinCreated, JoinRequest, JoinState, JoinStatus, MAX_DEVICE_NAME_CHARS, MAX_SEALED_CHARS, NewJoin,
    },
    pairing::{PushKind, PushMessage},
};
use rocket::{Data, Route, State, http::Status, serde::json::Json};

use super::{
    HUB, ITEM_TTL,
    device_api::{
        DeviceKey, PhoneResult, api_err, bad_request, is_caller, not_found, parse_versioned, read_body_limited,
        require_approval_device, user_key,
    },
    now_unix,
    pairing::sanitize_display,
    push,
    relay::ItemSignal,
};
use crate::{
    auth::Headers,
    db::{DbConn, DbPool, models::ReinsDevice},
    util::get_uuid,
};

/// Open requests one account may have at once.
pub const MAX_PER_USER: usize = 3;
/// Open requests the server holds in all.
pub const MAX_JOINS: usize = 1_000;
/// How long after the approval the asking device may take the approval role on its strength (seconds).
pub const TAKEOVER_TTL: i64 = 5 * 60;
const MAX_BODY_BYTES: u64 = 8 * 1024;

struct Entry {
    user: String,
    /// The asking device (Vaultwarden device id).
    device: String,
    /// Hash of the asking device's device key; an approval lets only this key take the approval role.
    key_hash: Option<String>,
    request: JoinRequest,
    delivered: bool,
    status: JoinStatus,
    sealed: Option<String>,
    expires_at: i64,
    /// When it was approved; the takeover it allows runs out [`TAKEOVER_TTL`] later.
    approved_at: Option<i64>,
    /// The approval was spent on a takeover.
    takeover_used: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum JoinError {
    /// The account has [`MAX_PER_USER`] open requests, or the server [`MAX_JOINS`].
    Full,
    Invalid(String),
}

#[derive(Debug, PartialEq, Eq)]
pub enum AnswerError {
    NotFound,
    AlreadyAnswered,
    Invalid(String),
}

pub struct JoinHub {
    entries: Mutex<HashMap<String, Entry>>,
    signal: Arc<ItemSignal>,
}

/// What `GET /reins/api/joins/<id>` shows the caller.
#[derive(Debug, PartialEq, Eq)]
pub enum View {
    Requester(JoinState),
    Approver(JoinRequest),
}

fn ttl_secs() -> i64 {
    i64::try_from(ITEM_TTL.as_secs()).unwrap_or(600)
}

impl JoinHub {
    pub fn new(signal: Arc<ItemSignal>) -> Self {
        Self {
            entries: Mutex::default(),
            signal,
        }
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<String, Entry>> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn create(
        &self,
        user: &str,
        device: &str,
        key_hash: Option<&str>,
        new: &NewJoin,
        now: i64,
    ) -> Result<JoinCreated, JoinError> {
        if reins_proto::desktop::decode_key(&new.public_key).is_none() {
            return Err(JoinError::Invalid("public_key must be 32 bytes, base64url".to_owned()));
        }
        let mut name = sanitize_display(&new.device_name, MAX_DEVICE_NAME_CHARS);
        if name.is_empty() {
            "A phone".clone_into(&mut name);
        }
        let mut entries = self.lock();
        entries.retain(|_, e| e.expires_at > now && !(e.user == user && e.device == device));
        if entries.len() >= MAX_JOINS || entries.values().filter(|e| e.user == user).count() >= MAX_PER_USER {
            return Err(JoinError::Full);
        }
        let id = get_uuid();
        let expires_at = now + ttl_secs();
        entries.insert(
            id.clone(),
            Entry {
                user: user.to_owned(),
                device: device.to_owned(),
                key_hash: key_hash.map(str::to_owned),
                request: JoinRequest {
                    v: PROTOCOL_VERSION,
                    id: id.clone(),
                    device_name: name,
                    public_key: new.public_key.clone(),
                    created_at: now,
                },
                delivered: false,
                status: JoinStatus::Waiting,
                sealed: None,
                expires_at,
                approved_at: None,
                takeover_used: false,
            },
        );
        drop(entries);
        self.signal.notify();
        Ok(JoinCreated {
            id,
            expires_at,
        })
    }

    /// Waiting requests of `user` the approval device has not seen yet; they are marked seen.
    pub fn take_undelivered(&self, user: &str, now: i64) -> Vec<JoinRequest> {
        let mut entries = self.lock();
        let mut out: Vec<JoinRequest> = entries
            .values_mut()
            .filter(|e| e.user == user && !e.delivered && e.status == JoinStatus::Waiting && e.expires_at > now)
            .map(|e| {
                e.delivered = true;
                e.request.clone()
            })
            .collect();
        out.sort_by_key(|r| r.created_at);
        out
    }

    pub fn view(&self, user: &str, device: &str, id: &str, now: i64) -> Option<View> {
        let mut entries = self.lock();
        let entry = entries.get_mut(id).filter(|e| e.user == user)?;
        if entry.expires_at <= now {
            return (entry.device == device).then_some(View::Requester(JoinState {
                status: JoinStatus::Expired,
                sealed: None,
            }));
        }
        if entry.device == device {
            Some(View::Requester(JoinState {
                status: entry.status,
                sealed: entry.sealed.clone(),
            }))
        } else {
            entry.delivered = true;
            (entry.status == JoinStatus::Waiting).then(|| View::Approver(entry.request.clone()))
        }
    }

    pub fn answer(&self, user: &str, id: &str, answer: &JoinAnswer, now: i64) -> Result<(), AnswerError> {
        let mut entries = self.lock();
        let entry =
            entries.get_mut(id).filter(|e| e.user == user && e.expires_at > now).ok_or(AnswerError::NotFound)?;
        if entry.status != JoinStatus::Waiting {
            return Err(AnswerError::AlreadyAnswered);
        }
        if answer.approve {
            let sealed = answer
                .sealed
                .as_deref()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| AnswerError::Invalid("an approval carries the sealed secret".to_owned()))?;
            if sealed.len() > MAX_SEALED_CHARS {
                return Err(AnswerError::Invalid("sealed is too long".to_owned()));
            }
            entry.status = JoinStatus::Approved;
            entry.sealed = Some(sealed.to_owned());
            entry.approved_at = Some(now);
        } else {
            entry.status = JoinStatus::Denied;
        }
        Ok(())
    }

    /// Spends an approval of `device`'s request, asked with the device key `key_hash`, on taking the approval role:
    /// `true` once per approval, within [`TAKEOVER_TTL`].
    pub fn take_takeover(&self, user: &str, device: &str, key_hash: Option<&str>, now: i64) -> bool {
        let Some(key_hash) = key_hash else {
            return false;
        };
        let mut entries = self.lock();
        let grant = entries.values_mut().find(|e| {
            e.user == user
                && e.device == device
                && e.status == JoinStatus::Approved
                && !e.takeover_used
                && e.approved_at.is_some_and(|at| now < at + TAKEOVER_TTL)
                && e.key_hash.as_deref().is_some_and(|k| crate::crypto::ct_eq(k, key_hash))
        });
        grant.map(|e| e.takeover_used = true).is_some()
    }

    pub fn purge(&self, now: i64) {
        // Kept a little past the deadline, so the asking phone still learns that it expired.
        self.lock().retain(|_, e| e.expires_at + ttl_secs() > now);
    }

    /// Drops the requests of one device of `user` with any sealed secret or takeover grant they carry (a device signed
    /// out).
    pub fn forget_device(&self, user: &str, device: &str) {
        self.lock().retain(|_, e| !(e.user == user && e.device.eq_ignore_ascii_case(device)));
    }

    /// Drops `user`'s requests with any sealed secret they carry (a deleted account).
    pub fn forget_user(&self, user: &str) {
        self.lock().retain(|_, e| e.user != user);
    }
}

/// Step 1: a phone of the account asks for the secret.
#[post("/reins/api/joins", data = "<data>")]
async fn post_join(
    data: Data<'_>,
    headers: Headers,
    key: DeviceKey,
    conn: DbConn,
    pool: &State<DbPool>,
) -> PhoneResult<Json<JoinCreated>> {
    let approval = ReinsDevice::find_by_user(&headers.user.uuid, &conn).await;
    let Some(approval) = approval else {
        return Err(api_err(
            Status::Conflict,
            codes::NOT_APPROVAL_DEVICE,
            "This account has no approval device to ask; use the recovery code",
        ));
    };
    if is_caller(&approval, &headers, &key) {
        return Err(bad_request("This is the approval device"));
    }
    let new: NewJoin = parse_versioned(&read_body_limited(data, MAX_BODY_BYTES).await?)?;
    let created = HUB
        .joins
        .create(&user_key(&headers), &headers.device.uuid.to_string(), key.hash(), &new, now_unix())
        .map_err(|e| match e {
            JoinError::Full => api_err(Status::TooManyRequests, codes::RATE_LIMITED, "Too many open requests"),
            JoinError::Invalid(m) => bad_request(m),
        })?;
    push::spawn_push(
        pool.inner().clone(),
        headers.user.uuid.clone(),
        approval.fcm_token,
        PushMessage {
            t: PushKind::Join,
            id: created.id.clone(),
        },
    );
    Ok(Json(created))
}

#[get("/reins/api/joins/<id>")]
async fn get_join(id: &str, headers: Headers, key: DeviceKey, conn: DbConn) -> PhoneResult<Json<serde_json::Value>> {
    let view =
        HUB.joins.view(&user_key(&headers), &headers.device.uuid.to_string(), id, now_unix()).ok_or_else(not_found)?;
    match view {
        View::Requester(state) => Ok(Json(serde_json::to_value(state).unwrap_or_default())),
        View::Approver(request) => {
            require_approval_device(&headers, &key, &conn).await?;
            Ok(Json(serde_json::to_value(request).unwrap_or_default()))
        }
    }
}

#[post("/reins/api/joins/<id>/response", data = "<data>")]
async fn post_join_response(
    id: &str,
    data: Data<'_>,
    headers: Headers,
    key: DeviceKey,
    conn: DbConn,
) -> PhoneResult<Status> {
    require_approval_device(&headers, &key, &conn).await?;
    let answer: JoinAnswer = parse_versioned(&read_body_limited(data, MAX_BODY_BYTES).await?)?;
    HUB.joins.answer(&user_key(&headers), id, &answer, now_unix()).map_err(|e| match e {
        AnswerError::NotFound => not_found(),
        AnswerError::AlreadyAnswered => super::device_api::already_answered(),
        AnswerError::Invalid(m) => bad_request(m),
    })?;
    Ok(Status::NoContent)
}

pub fn routes() -> Vec<Route> {
    routes![post_join, get_join, post_join_response]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hub() -> JoinHub {
        JoinHub::new(Arc::new(ItemSignal::new()))
    }

    fn new_join(name: &str) -> NewJoin {
        NewJoin {
            v: 1,
            device_name: name.to_owned(),
            public_key: reins_proto::desktop::encode_key(&[9u8; 32]),
        }
    }

    #[test]
    fn a_join_goes_from_the_new_phone_to_the_approval_device_and_back() {
        let hub = hub();
        let created = hub.create("u1", "new-phone", Some("k-new"), &new_join("Pixel\u{202E} 9"), 100).unwrap();
        // The approval device sees it once (cleaned name), the other account never.
        let seen = hub.take_undelivered("u1", 101);
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].device_name, "Pixel 9");
        assert!(hub.take_undelivered("u1", 101).is_empty());
        assert!(hub.take_undelivered("u2", 101).is_empty());
        assert!(hub.view("u2", "x", &created.id, 101).is_none());
        assert!(matches!(hub.view("u1", "approver", &created.id, 101), Some(View::Approver(_))));
        assert_eq!(
            hub.view("u1", "new-phone", &created.id, 101),
            Some(View::Requester(JoinState {
                status: JoinStatus::Waiting,
                sealed: None
            }))
        );
        let approve = JoinAnswer {
            v: 1,
            approve: true,
            sealed: Some("SEALED".to_owned()),
        };
        assert_eq!(hub.answer("u2", &created.id, &approve, 102), Err(AnswerError::NotFound));
        hub.answer("u1", &created.id, &approve, 102).unwrap();
        assert_eq!(hub.answer("u1", &created.id, &approve, 103), Err(AnswerError::AlreadyAnswered));
        assert_eq!(
            hub.view("u1", "new-phone", &created.id, 103),
            Some(View::Requester(JoinState {
                status: JoinStatus::Approved,
                sealed: Some("SEALED".to_owned())
            }))
        );
        // Answered: the approval device no longer gets it.
        assert!(hub.view("u1", "approver", &created.id, 103).is_none());
    }

    #[test]
    fn an_approval_lets_the_asking_key_take_over_once_for_a_while() {
        let hub = hub();
        let approve = JoinAnswer {
            v: 1,
            approve: true,
            sealed: Some("SEALED".to_owned()),
        };
        let created = hub.create("u1", "new", Some("k-new"), &new_join("Pixel"), 100).unwrap();
        assert!(!hub.take_takeover("u1", "new", Some("k-new"), 101), "not before the approval");
        hub.answer("u1", &created.id, &approve, 110).unwrap();
        assert!(!hub.take_takeover("u2", "new", Some("k-new"), 111), "another account");
        assert!(!hub.take_takeover("u1", "other", Some("k-new"), 111), "another device");
        assert!(!hub.take_takeover("u1", "new", Some("k-other"), 111), "the same device id with another key");
        assert!(!hub.take_takeover("u1", "new", None, 111), "no key");
        assert!(hub.take_takeover("u1", "new", Some("k-new"), 111));
        assert!(!hub.take_takeover("u1", "new", Some("k-new"), 112), "once");

        // Too late, denied, or asked without a key: no takeover.
        let late = hub.create("u1", "new", Some("k-new"), &new_join("Pixel"), 200).unwrap();
        hub.answer("u1", &late.id, &approve, 200).unwrap();
        assert!(!hub.take_takeover("u1", "new", Some("k-new"), 200 + TAKEOVER_TTL));
        let denied = hub.create("u1", "d2", Some("k2"), &new_join("b"), 300).unwrap();
        let deny = JoinAnswer {
            v: 1,
            approve: false,
            sealed: None,
        };
        hub.answer("u1", &denied.id, &deny, 301).unwrap();
        assert!(!hub.take_takeover("u1", "d2", Some("k2"), 302));
        let keyless = hub.create("u1", "d3", None, &new_join("c"), 300).unwrap();
        hub.answer("u1", &keyless.id, &approve, 301).unwrap();
        assert!(!hub.take_takeover("u1", "d3", None, 302));
    }

    #[test]
    fn limits_replacement_and_expiry() {
        let hub = hub();
        let first = hub.create("u1", "d1", None, &new_join("a"), 100).unwrap();
        let again = hub.create("u1", "d1", None, &new_join("a"), 101).unwrap();
        assert!(hub.view("u1", "d1", &first.id, 101).is_none(), "a device's new request replaces its old one");
        hub.create("u1", "d2", None, &new_join("b"), 101).unwrap();
        hub.create("u1", "d3", None, &new_join("c"), 101).unwrap();
        assert_eq!(hub.create("u1", "d4", None, &new_join("d"), 101), Err(JoinError::Full));
        let bad = NewJoin {
            public_key: "short".to_owned(),
            ..new_join("x")
        };
        assert!(matches!(hub.create("u2", "d1", None, &bad, 101), Err(JoinError::Invalid(_))));
        let late = again.expires_at;
        assert_eq!(
            hub.view("u1", "d1", &again.id, late),
            Some(View::Requester(JoinState {
                status: JoinStatus::Expired,
                sealed: None
            }))
        );
        let deny = JoinAnswer {
            v: 1,
            approve: false,
            sealed: None,
        };
        assert_eq!(hub.answer("u1", &again.id, &deny, late), Err(AnswerError::NotFound));
        let no_secret = JoinAnswer {
            v: 1,
            approve: true,
            sealed: None,
        };
        assert!(matches!(hub.answer("u1", &again.id, &no_secret, 105), Err(AnswerError::Invalid(_))));
        hub.purge(late + 10_000);
        assert!(hub.view("u1", "d1", &again.id, late).is_none());
    }
}
