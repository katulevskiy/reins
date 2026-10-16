//! "Add another phone" ([`reins_proto::join`]): a phone of the account that cannot open its keys asks the approval
//! device for the account secret; the server holds the request and relays the sealed answer, which it cannot open.
//!
//! - `POST /reins/api/joins` (any signed-in device of the account but the approval device): parks the request and
//!   wakes the approval device (push `join`). One open request per device (a new one replaces it), at most
//!   [`MAX_PER_USER`] per account, and a few per IP address over time ([`super::limits::JOIN_CREATIONS`]). Expired
//!   requests are dropped on every new request and by the scheduled purge.
//! - `GET /reins/api/joins/<id>`: the asking device gets its [`JoinState`]; the approval device the
//!   [`JoinRequest`].
//! - `POST /reins/api/joins/<id>/response` (the approval device): approves with the sealed secret, or denies.
//!
//! An approval also lets the asking device take the approval role (`PUT /reins/api/device`) without another proof:
//! once, within [`TAKEOVER_TTL`], and only with the device key it asked with ([`JoinHub::take_takeover`]).
//!
//! **Enrolling as a viewer** (phone roles, [`reins_proto::enroll`]): the request also carries the phone's signing key
//! and a commitment; then `POST /reins/api/joins/<id>/challenge` (the approval device, once) and
//! `POST /reins/api/joins/<id>/reveal` (the asking device, once, after the challenge; the server checks the nonce
//! opens the commitment). A roster entry may list a new viewer only when such a join of that device, with those keys,
//! went through both steps ([`JoinHub::enrollment_binding`]): the member is bound to the device key the join was asked
//! with.
//!
//! Everything is in memory for [`ITEM_TTL`] like pairings.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};

use reins_proto::{
    PROTOCOL_VERSION,
    answer::AnswerKind,
    device::codes,
    enroll::{EnrollChallenge, EnrollReveal, check_commitment, parse_nonce},
    join::{
        JoinAnswer, JoinCreated, JoinRequest, JoinState, JoinStatus, MAX_DEVICE_NAME_CHARS, MAX_ENROLL_SEALED_CHARS,
        MAX_SEALED_CHARS, NewJoin,
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
    auth::{ClientIp, Headers},
    db::{DbConn, DbPool, models::ReinsDevice},
    util::get_uuid,
};

/// Open requests one account may have at once.
pub const MAX_PER_USER: usize = 3;
/// How long after the approval the asking device may take the approval role on its strength (seconds).
pub const TAKEOVER_TTL: i64 = 5 * 60;
const MAX_BODY_BYTES: u64 = 48 * 1024;

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
    /// Enrolling: a roster entry listed the phone on the strength of this join (one join binds one entry).
    bound: bool,
    /// Enrolling: the approval device's challenge, once posted.
    challenge: Option<EnrollChallenge>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum JoinError {
    /// The account has [`MAX_PER_USER`] open requests. There is no limit for the whole server: one would let a few
    /// accounts keep every other account's phones from joining or enrolling.
    Full,
    Invalid(String),
}

#[derive(Debug, PartialEq, Eq)]
pub enum AnswerError {
    NotFound,
    AlreadyAnswered,
    Invalid(String),
}

/// Why an enrollment step was refused.
#[derive(Debug, PartialEq, Eq)]
pub enum StepError {
    NotFound,
    /// Not an enrollment, already past this step, or not yet at it.
    OutOfOrder(&'static str),
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
        match (&new.sign_pub, &new.commitment) {
            (None, None) => {}
            (Some(sign_pub), Some(commitment)) => {
                if reins_proto::desktop::decode_key(sign_pub).is_none() {
                    return Err(JoinError::Invalid("sign_pub must be 32 bytes, base64url".to_owned()));
                }
                if reins_proto::canon::parse_hex32(commitment).is_none() {
                    return Err(JoinError::Invalid("commitment must be 64 lowercase hex digits".to_owned()));
                }
            }
            _ => return Err(JoinError::Invalid("an enrollment carries both sign_pub and commitment".to_owned())),
        }
        let mut name = sanitize_display(&new.device_name, MAX_DEVICE_NAME_CHARS);
        if name.is_empty() {
            "A phone".clone_into(&mut name);
        }
        let mut entries = self.lock();
        entries.retain(|_, e| e.expires_at > now && !(e.user == user && e.device == device));
        if entries.values().filter(|e| e.user == user).count() >= MAX_PER_USER {
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
                    device_id: Some(device.to_owned()),
                    sign_pub: new.sign_pub.clone(),
                    commitment: new.commitment.clone(),
                    reveal: None,
                },
                delivered: false,
                status: JoinStatus::Waiting,
                sealed: None,
                expires_at,
                approved_at: None,
                takeover_used: false,
                challenge: None,
                bound: false,
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

    /// The request as `device` may see it: its own state, or (only when `approver`, the caller already checked to be
    /// the approval device) the request, which is then marked seen. Nobody else learns of it or changes it.
    pub fn view(&self, user: &str, device: &str, id: &str, now: i64, approver: bool) -> Option<View> {
        let mut entries = self.lock();
        let entry = entries.get_mut(id).filter(|e| e.user == user)?;
        if entry.expires_at <= now {
            return (entry.device == device).then_some(View::Requester(JoinState {
                status: JoinStatus::Expired,
                sealed: None,
                challenge: None,
            }));
        }
        if entry.device == device {
            Some(View::Requester(JoinState {
                status: entry.status,
                sealed: entry.sealed.clone(),
                challenge: entry.challenge.clone(),
            }))
        } else if approver {
            entry.delivered = true;
            (entry.status == JoinStatus::Waiting).then(|| View::Approver(entry.request.clone()))
        } else {
            None
        }
    }

    pub fn answer(&self, user: &str, id: &str, answer: &JoinAnswer, now: i64) -> Result<(), AnswerError> {
        let mut entries = self.lock();
        let entry =
            entries.get_mut(id).filter(|e| e.user == user && e.expires_at > now).ok_or(AnswerError::NotFound)?;
        if entry.status != JoinStatus::Waiting {
            return Err(AnswerError::AlreadyAnswered);
        }
        if answer.approve && entry.request.commitment.is_some() && entry.request.reveal.is_none() {
            return Err(AnswerError::Invalid(
                "an enrollment is approved only after the challenge and the reveal".to_owned(),
            ));
        }
        if answer.approve {
            let sealed = answer
                .sealed
                .as_deref()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| AnswerError::Invalid("an approval carries the sealed secret".to_owned()))?;
            let max = if entry.request.commitment.is_some() {
                MAX_ENROLL_SEALED_CHARS
            } else {
                MAX_SEALED_CHARS
            };
            if sealed.len() > max {
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

    /// Enrollment step 2: the approval device's challenge, once, to a waiting enrollment that has none.
    pub fn challenge(&self, user: &str, id: &str, challenge: EnrollChallenge, now: i64) -> Result<(), StepError> {
        challenge.validate(now).map_err(|e| StepError::Invalid(e.to_string()))?;
        let mut entries = self.lock();
        let entry = entries.get_mut(id).filter(|e| e.user == user && e.expires_at > now).ok_or(StepError::NotFound)?;
        if entry.request.commitment.is_none() || entry.status != JoinStatus::Waiting || entry.challenge.is_some() {
            return Err(StepError::OutOfOrder("only a waiting enrollment without a challenge takes one"));
        }
        entry.challenge = Some(challenge);
        Ok(())
    }

    /// Enrollment step 3: the asking device's nonce, once, after the challenge; it must open the commitment.
    pub fn reveal(&self, user: &str, device: &str, id: &str, reveal: EnrollReveal, now: i64) -> Result<(), StepError> {
        reveal.validate().map_err(|e| StepError::Invalid(e.to_string()))?;
        let mut entries = self.lock();
        let entry = entries
            .get_mut(id)
            .filter(|e| e.user == user && e.device == device && e.expires_at > now)
            .ok_or(StepError::NotFound)?;
        if entry.challenge.is_none() || entry.request.reveal.is_some() || entry.status != JoinStatus::Waiting {
            return Err(StepError::OutOfOrder("the nonce is revealed once, after the challenge"));
        }
        if entry.challenge.as_ref().is_some_and(|c| now >= c.expires_at) {
            return Err(StepError::OutOfOrder("the challenge expired; ask again"));
        }
        let (Some(commitment), Some(sign_pub), Some(nonce)) =
            (&entry.request.commitment, &entry.request.sign_pub, parse_nonce(&reveal.nonce))
        else {
            return Err(StepError::Invalid("not an enrollment".to_owned()));
        };
        if !check_commitment(commitment, user, device, &entry.request.public_key, sign_pub, &nonce) {
            return Err(StepError::Invalid("the nonce does not open the commitment".to_owned()));
        }
        entry.request.reveal = Some(reveal);
        // The approval device sees the request again, now with the nonce.
        entry.delivered = false;
        drop(entries);
        self.signal.notify();
        Ok(())
    }

    /// Spends the enrollment of `device` with these keys on one roster entry: a join of `device` that went through the
    /// challenge and the reveal, was not denied and bound no entry yet. Returns the join's id (to release it if the
    /// entry is not stored after all) and the device key it was asked with.
    pub fn take_enrollment_binding(
        &self,
        user: &str,
        device: &str,
        box_pub: &str,
        sign_pub: &str,
        now: i64,
    ) -> Option<(String, String)> {
        let mut entries = self.lock();
        let (id, entry) = entries.iter_mut().find(|(_, e)| {
            e.user == user
                && e.device == device
                && e.expires_at > now
                && !e.bound
                && matches!(e.status, JoinStatus::Waiting | JoinStatus::Approved)
                && e.request.public_key == box_pub
                && e.request.sign_pub.as_deref() == Some(sign_pub)
                && e.challenge.is_some()
                && e.request.reveal.is_some()
        })?;
        let key_hash = entry.key_hash.clone()?;
        entry.bound = true;
        Some((id.clone(), key_hash))
    }

    /// The roster entry that took this binding was not stored: the join may bind another.
    pub fn release_binding(&self, id: &str) {
        if let Some(e) = self.lock().get_mut(id) {
            e.bound = false;
        }
    }

    /// Waiting requests of `user`, without marking them seen (what a viewer shows).
    pub fn peek_waiting(&self, user: &str, now: i64) -> Vec<JoinRequest> {
        let mut out: Vec<JoinRequest> = self
            .lock()
            .values()
            .filter(|e| e.user == user && e.status == JoinStatus::Waiting && e.expires_at > now)
            .map(|e| e.request.clone())
            .collect();
        out.sort_by(|a, b| (a.created_at, &a.id).cmp(&(b.created_at, &b.id)));
        out
    }

    /// Ids of `user`'s answered requests still held.
    pub fn answered_ids(&self, user: &str) -> Vec<String> {
        let mut ids: Vec<String> = self
            .lock()
            .iter()
            .filter(|(_, e)| e.user == user && e.status != JoinStatus::Waiting)
            .map(|(id, _)| id.clone())
            .collect();
        ids.sort();
        ids
    }

    /// Hands `user`'s waiting requests to the next `GET /pending` again (a new primary).
    pub fn requeue(&self, user: &str) {
        for e in self.lock().values_mut().filter(|e| e.user == user && e.status == JoinStatus::Waiting) {
            e.delivered = false;
        }
        self.signal.notify();
    }
}

/// Step 1: a phone of the account asks for the secret.
#[post("/reins/api/joins", data = "<data>")]
async fn post_join(
    data: Data<'_>,
    headers: Headers,
    key: DeviceKey,
    ip: ClientIp,
    conn: DbConn,
    pool: &State<DbPool>,
) -> PhoneResult<Json<JoinCreated>> {
    if let Err(wait) = super::limits::JOIN_CREATIONS.check(&ip.ip) {
        return Err(super::device_api::rate_limited("too many requests to add a phone from this address", wait));
    }
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
    // Authorized before anything is marked seen: another device must not take a request from the approval device.
    let approval = require_approval_device(&headers, &key, &conn).await;
    let view = HUB.joins.view(&user_key(&headers), &headers.device.uuid.to_string(), id, now_unix(), approval.is_ok());
    match view {
        Some(View::Requester(state)) => Ok(Json(serde_json::to_value(state).unwrap_or_default())),
        Some(View::Approver(request)) => Ok(Json(serde_json::to_value(request).unwrap_or_default())),
        None => Err(approval.err().unwrap_or_else(not_found)),
    }
}

/// Enrollment step 2 (the approval device).
#[post("/reins/api/joins/<id>/challenge", data = "<data>")]
async fn post_join_challenge(
    id: &str,
    data: Data<'_>,
    headers: Headers,
    key: DeviceKey,
    conn: DbConn,
) -> PhoneResult<Status> {
    require_approval_device(&headers, &key, &conn).await?;
    let challenge: EnrollChallenge = parse_versioned(&read_body_limited(data, MAX_BODY_BYTES).await?)?;
    HUB.joins.challenge(&user_key(&headers), id, challenge, now_unix()).map_err(step_error)?;
    HUB.live.bump(&user_key(&headers));
    Ok(Status::NoContent)
}

/// Enrollment step 3 (the asking device).
#[post("/reins/api/joins/<id>/reveal", data = "<data>")]
async fn post_join_reveal(id: &str, data: Data<'_>, headers: Headers) -> PhoneResult<Status> {
    let reveal: EnrollReveal = parse_versioned(&read_body_limited(data, MAX_BODY_BYTES).await?)?;
    HUB.joins
        .reveal(&user_key(&headers), &headers.device.uuid.to_string(), id, reveal, now_unix())
        .map_err(step_error)?;
    HUB.live.bump(&user_key(&headers));
    Ok(Status::NoContent)
}

fn step_error(e: StepError) -> rocket::response::status::Custom<Json<reins_proto::device::ApiError>> {
    match e {
        StepError::NotFound => not_found(),
        StepError::OutOfOrder(m) => api_err(Status::Conflict, codes::ALREADY_ANSWERED, m),
        StepError::Invalid(m) => bad_request(m),
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
    let user = user_key(&headers);
    let entries = super::roles::chain(&user, &conn).await?;
    let body = read_body_limited(data, MAX_BODY_BYTES).await?;
    // With a roster the answer carries the primary's signature, like A4.
    let answer: JoinAnswer = if entries.is_empty() {
        parse_versioned(&body)?
    } else {
        parse_versioned(&super::roles::open_signed_answer(&user, AnswerKind::Join, id, &body, &entries)?)?
    };
    HUB.joins.answer(&user, id, &answer, now_unix()).map_err(|e| match e {
        AnswerError::NotFound => not_found(),
        AnswerError::AlreadyAnswered => super::device_api::already_answered(),
        AnswerError::Invalid(m) => bad_request(m),
    })?;
    HUB.live.bump(&user_key(&headers));
    Ok(Status::NoContent)
}

pub fn routes() -> Vec<Route> {
    routes![post_join, get_join, post_join_challenge, post_join_reveal, post_join_response]
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
            sign_pub: None,
            commitment: None,
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
        assert!(hub.view("u2", "x", &created.id, 101, true).is_none());
        assert!(matches!(hub.view("u1", "approver", &created.id, 101, true), Some(View::Approver(_))));
        assert_eq!(
            hub.view("u1", "new-phone", &created.id, 101, true),
            Some(View::Requester(JoinState {
                status: JoinStatus::Waiting,
                sealed: None,
                challenge: None,
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
            hub.view("u1", "new-phone", &created.id, 103, true),
            Some(View::Requester(JoinState {
                status: JoinStatus::Approved,
                sealed: Some("SEALED".to_owned()),
                challenge: None,
            }))
        );
        // Answered: the approval device no longer gets it.
        assert!(hub.view("u1", "approver", &created.id, 103, true).is_none());
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
        assert!(hub.view("u1", "d1", &first.id, 101, true).is_none(), "a device's new request replaces its old one");
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
            hub.view("u1", "d1", &again.id, late, true),
            Some(View::Requester(JoinState {
                status: JoinStatus::Expired,
                sealed: None,
                challenge: None,
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
        assert!(hub.view("u1", "d1", &again.id, late, true).is_none());
    }

    #[test]
    fn an_enrollment_goes_commitment_challenge_reveal_and_only_then_binds_its_key() {
        use reins_proto::{
            desktop::encode_key,
            enroll::{commitment, encode_nonce},
        };
        let hub = hub();
        let (box_pub, sign_pub) = (encode_key(&[9u8; 32]), encode_key(&[8u8; 32]));
        let nonce = [5u8; 32];
        let new = NewJoin {
            sign_pub: Some(sign_pub.clone()),
            commitment: commitment("u1", "viewer", &box_pub, &sign_pub, &nonce),
            ..new_join("Pixel")
        };
        let half = NewJoin {
            commitment: None,
            ..new.clone()
        };
        assert!(matches!(hub.create("u1", "viewer", Some("kv"), &half, 100), Err(JoinError::Invalid(_))));
        let created = hub.create("u1", "viewer", Some("kv"), &new, 100).unwrap();
        let request = hub.take_undelivered("u1", 100).pop().unwrap();
        assert_eq!(
            (request.device_id.as_deref(), request.sign_pub.as_deref()),
            (Some("viewer"), Some(sign_pub.as_str()))
        );
        let reveal = |n: [u8; 32]| EnrollReveal {
            v: 1,
            nonce: encode_nonce(&n),
            proof: None,
        };
        let challenge = EnrollChallenge {
            v: 1,
            nonce: encode_nonce(&[1; 32]),
            primary_sign_pub: encode_key(&[2; 32]),
            owner_pub: encode_key(&[3; 32]),
            epoch: 1,
            expires_at: 300,
        };
        assert!(matches!(hub.reveal("u1", "viewer", &created.id, reveal(nonce), 101), Err(StepError::OutOfOrder(_))));
        assert_eq!(hub.take_enrollment_binding("u1", "viewer", &box_pub, &sign_pub, 101), None);
        hub.challenge("u1", &created.id, challenge.clone(), 101).unwrap();
        assert!(matches!(hub.challenge("u1", &created.id, challenge, 101), Err(StepError::OutOfOrder(_))));
        assert!(matches!(hub.reveal("u1", "other", &created.id, reveal(nonce), 102), Err(StepError::NotFound)));
        assert!(matches!(hub.reveal("u1", "viewer", &created.id, reveal([6; 32]), 102), Err(StepError::Invalid(_))));
        let early = JoinAnswer {
            v: 1,
            approve: true,
            sealed: Some("SEALED".to_owned()),
        };
        assert!(
            matches!(hub.answer("u1", &created.id, &early, 102), Err(AnswerError::Invalid(_))),
            "no approval before the reveal"
        );
        assert!(
            matches!(hub.reveal("u1", "viewer", &created.id, reveal(nonce), 300), Err(StepError::OutOfOrder(_))),
            "not after the challenge expired"
        );
        hub.reveal("u1", "viewer", &created.id, reveal(nonce), 102).unwrap();
        assert!(matches!(hub.reveal("u1", "viewer", &created.id, reveal(nonce), 102), Err(StepError::OutOfOrder(_))));
        // Bound to the key that asked, for exactly these keys and this device, once.
        assert_eq!(hub.take_enrollment_binding("u1", "viewer", &box_pub, &encode_key(&[7; 32]), 103), None);
        assert_eq!(hub.take_enrollment_binding("u1", "other", &box_pub, &sign_pub, 103), None);
        assert_eq!(hub.take_enrollment_binding("u2", "viewer", &box_pub, &sign_pub, 103), None);
        let (join, key) = hub.take_enrollment_binding("u1", "viewer", &box_pub, &sign_pub, 103).unwrap();
        assert_eq!((join.as_str(), key.as_str()), (created.id.as_str(), "kv"));
        assert_eq!(hub.take_enrollment_binding("u1", "viewer", &box_pub, &sign_pub, 103), None, "spent");
        hub.release_binding(&join);
        assert!(hub.take_enrollment_binding("u1", "viewer", &box_pub, &sign_pub, 103).is_some(), "released");
        hub.release_binding(&join);
        // Another device of the account does not see it, nor mark it seen.
        assert_eq!(hub.view("u1", "someone", &created.id, 103, false), None);
        // The approval device sees it again, with the nonce; viewers see it without taking it.
        assert!(hub.take_undelivered("u1", 103)[0].reveal.is_some());
        assert_eq!(hub.peek_waiting("u1", 103).len(), 1);
        assert!(hub.take_undelivered("u1", 103).is_empty());
        hub.requeue("u1");
        assert_eq!(hub.take_undelivered("u1", 103).len(), 1, "a new primary gets it again");
        let deny = JoinAnswer {
            v: 1,
            approve: false,
            sealed: None,
        };
        hub.answer("u1", &created.id, &deny, 104).unwrap();
        assert_eq!(hub.take_enrollment_binding("u1", "viewer", &box_pub, &sign_pub, 104), None, "not once denied");
        assert_eq!(hub.answered_ids("u1"), [created.id]);
    }
}
