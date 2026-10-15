//! Phone roles (`reins_proto::roster`): one primary decides, viewers watch. The server stores the owner-signed roster
//! chain and enforces it: it appends an entry only when the owner key's signature verifies, the epoch is the next one
//! (compare-and-swap) and the caller may make that change; it binds each member to the device key it enrolled with;
//! and it moves the approval role (`reins_devices`) with the roster's primary.
//!
//! An account without a roster is a legacy account: nothing here changes how it works. `GET /reins/api/roster`
//! answers an empty list then (404 means a server without phone roles).
//!
//! - `GET /reins/api/roster` (any signed-in device of the account): the chain and the recovery wrap.
//! - `POST /reins/api/roster`: the next entry. `create` from the approval device, `enroll`/`remove`/`handover` from
//!   the primary, `recovery` from the device it makes primary, with the master password hash.
//! - `PUT /reins/api/roster/recovery-wrap` (the primary).
//! - `PUT /reins/api/member` (a member): its push token.
//! - `POST /reins/api/handovers` (a viewer), `GET /reins/api/handovers/<id>`, `POST /reins/api/handovers/<id>/response`
//!   (the primary, to deny), `GET`/`DELETE /reins/api/handover-bundle` (the primary a handover named).

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};

use reins_proto::{
    PROTOCOL_VERSION,
    answer::{AnswerKind, SignedAnswer, answer_message},
    device::codes,
    pairing::{PushKind, PushMessage},
    roster::{
        HandoverAnswer, HandoverBundle, HandoverCreated, HandoverRequest, HandoverState, HandoverStatus,
        MAX_BUNDLE_CHARS, MAX_RECOVERY_WRAP_CHARS, Member, MemberRegistration, NewHandover, RecoveryWrapUpdate, Roster,
        RosterAppend, RosterAppended, RosterEntry, RosterError, RosterReason, verify_step,
    },
};
use rocket::{Data, Route, State, http::Status, serde::json::Json};

use super::{
    HUB, ITEM_TTL,
    device_api::{
        DeviceKey, PhoneResult, api_err, bad_request, internal, normalize_fcm_token, not_found, parse_versioned,
        rate_limited, read_body_limited, require_approval_device, user_key,
    },
    now_unix, push,
    relay::ItemSignal,
};
use crate::{
    auth::Headers,
    db::{
        DbConn, DbPool,
        models::{
            Listed, ROLE_PRIMARY, ROLE_VIEWER, ReinsDevice, ReinsMember, ReinsRoster, ReinsRosterRow, WriteError,
        },
    },
    util::get_uuid,
};

pub fn routes() -> Vec<Route> {
    routes![
        get_roster,
        post_roster,
        put_recovery_wrap,
        put_member,
        post_handover,
        get_handover,
        post_handover_response,
        get_handover_bundle,
        delete_handover_bundle
    ]
}

/// Largest `POST /reins/api/roster` body.
const MAX_ROSTER_BODY: u64 = 64 * 1024;

/// Ed25519 (strict, ring) as `reins_proto::roster::verify_step` and the others take it.
pub fn ed25519_verify(public_key: &[u8; 32], message: &[u8], signature: &[u8; 64]) -> bool {
    ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, public_key).verify(message, signature).is_ok()
}

/// The account's roster chain, oldest first; empty for a legacy account.
pub async fn chain(user: &str, conn: &DbConn) -> PhoneResult<Vec<RosterEntry>> {
    let rows = ReinsRoster::entries(user, conn).await.map_err(|e| internal(&e))?;
    rows.iter()
        .map(|row| serde_json::from_str::<RosterEntry>(&row.entry))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| internal(&crate::Error::new("A stored Reins roster entry does not parse", e.to_string())))
}

/// Whether the account has a roster (phone roles are on).
pub async fn has_roster(user: &str, conn: &DbConn) -> PhoneResult<bool> {
    Ok(ReinsRoster::latest(user, conn).await.map_err(|e| internal(&e))?.is_some())
}

fn no_roster() -> rocket::response::status::Custom<Json<reins_proto::device::ApiError>> {
    api_err(Status::Conflict, codes::NO_ROSTER, "This account has no roster; its approval device creates one")
}

fn not_a_member() -> rocket::response::status::Custom<Json<reins_proto::device::ApiError>> {
    api_err(Status::Forbidden, codes::NOT_A_MEMBER, "This device is not a member of the account's roster")
}

/// The calling device's member row, when it is bound to this device key. `current`: only the roster's current
/// primary or viewers (else a removed member too, whose late restrictive ops may still count).
async fn caller_member(headers: &Headers, key: &DeviceKey, conn: &DbConn, current: bool) -> PhoneResult<ReinsMember> {
    let user = user_key(headers);
    if !has_roster(&user, conn).await? {
        return Err(no_roster());
    }
    let member = ReinsMember::find(&user, &headers.device.uuid.to_string(), conn).await.map_err(|e| internal(&e))?;
    match member {
        Some(m)
            if (!current || m.is_current())
                && matches!((&m.key_hash, key.hash()), (Some(kept), Some(given)) if crate::crypto::ct_eq(kept, given)) =>
        {
            Ok(m)
        }
        _ => Err(not_a_member()),
    }
}

/// A current member of the account's roster (primary or viewer), with its device key.
pub async fn require_member(headers: &Headers, key: &DeviceKey, conn: &DbConn) -> PhoneResult<ReinsMember> {
    caller_member(headers, key, conn, true).await
}

/// A member the roster lists or listed (with its device key): who may post ops; `authorize_op` decides which.
pub async fn require_any_member(headers: &Headers, key: &DeviceKey, conn: &DbConn) -> PhoneResult<ReinsMember> {
    caller_member(headers, key, conn, false).await
}

/// The approval device, or (with a roster) any current member: who may read what the members share. Without a
/// roster the answer is the approval device's, unchanged.
pub async fn require_reader(headers: &Headers, key: &DeviceKey, conn: &DbConn) -> PhoneResult<()> {
    let approval = require_approval_device(headers, key, conn).await;
    if approval.is_ok() || !has_roster(&user_key(headers), conn).await? {
        return approval;
    }
    require_member(headers, key, conn).await.map(drop)
}

/// The member that is primary at `epoch` when it is still the current primary with the same keys: whose signature a
/// primary-only message (an answer, a checkpoint, a heartbeat) signed at `epoch` must carry.
pub fn primary_at(entries: &[RosterEntry], epoch: u64) -> Option<&Member> {
    let current = entries.last()?;
    let at = entries.iter().find(|e| e.epoch == epoch)?;
    at.primary.same_keys(&current.primary).then_some(&at.primary)
}

/// For an account with a roster, the A4 body is a [`SignedAnswer`]: the primary's signature over the exact answer
/// bytes, which are returned. Legacy accounts keep sending the answer itself.
pub fn open_signed_answer(
    user: &str,
    kind: AnswerKind,
    id: &str,
    body: &[u8],
    entries: &[RosterEntry],
) -> PhoneResult<Vec<u8>> {
    let signature_required = || {
        api_err(
            Status::Forbidden,
            codes::SIGNATURE_REQUIRED,
            "This account has phone roles: answers carry the primary's signature",
        )
    };
    let value: serde_json::Value =
        serde_json::from_slice(body).map_err(|e| bad_request(format!("invalid JSON: {e}")))?;
    if value.get("answer").is_none() || value.get("sig").is_none() {
        return Err(signature_required());
    }
    let envelope: SignedAnswer = parse_versioned(body)?;
    let bytes = envelope.answer_bytes().map_err(|e| bad_request(e.to_string()))?;
    let bad = || api_err(Status::Forbidden, codes::BAD_SIGNATURE, "The answer is not signed by the primary");
    let signer = primary_at(entries, envelope.epoch).ok_or_else(bad)?;
    let key = signer.sign_key().map_err(|_| bad())?;
    let sig = reins_proto::canon::parse_sig(&envelope.sig).ok_or_else(bad)?;
    if !ed25519_verify(&key, &answer_message(user, kind, id, envelope.epoch, &bytes), &sig) {
        return Err(bad());
    }
    Ok(bytes)
}

#[get("/reins/api/roster")]
async fn get_roster(headers: Headers, conn: DbConn) -> PhoneResult<Json<Roster>> {
    let user = user_key(&headers);
    let rows = ReinsRoster::entries(&user, &conn).await.map_err(|e| internal(&e))?;
    let recovery_wrap = rows.last().and_then(|r| r.recovery_wrap.clone());
    Ok(Json(Roster {
        entries: chain(&user, &conn).await?,
        recovery_wrap,
    }))
}

fn roster_error(e: &RosterError) -> rocket::response::status::Custom<Json<reins_proto::device::ApiError>> {
    match e {
        RosterError::WrongEpoch {
            ..
        } => api_err(Status::Conflict, codes::EPOCH_CONFLICT, e.to_string()),
        RosterError::BadSignature => api_err(Status::Forbidden, codes::BAD_SIGNATURE, e.to_string()),
        _ => api_err(Status::BadRequest, codes::BAD_ROSTER, e.to_string()),
    }
}

fn check_payload(field: &str, value: &str, max: usize) -> PhoneResult<()> {
    if value.is_empty() || value.len() > max || reins_proto::canon::decode_payload(value).is_none() {
        return Err(api_err(
            Status::BadRequest,
            codes::BAD_ROSTER,
            format!("{field} must be 1 to {max} characters of base64url"),
        ));
    }
    Ok(())
}

/// The next roster entry. Who may append what:
///
/// - `create` (no roster yet): the approval device, naming itself primary, with a recovery wrap;
/// - `enroll`, `remove`, `handover`: the current primary; a new viewer must have enrolled through a join from its
///   device ([`super::join::JoinHub::enrollment_binding`]), and a handover carries the sealed bundle;
/// - `recovery`: the device the entry makes primary, with the master password hash (counted like takeover proofs).
///
/// The entry must verify against the owner key (`verify_step`) and name the next epoch; a concurrent append makes the
/// later one fail with 409 `epoch_conflict`. When the primary changes, the approval role moves with it, unanswered
/// requests, pairings and joins go to the new primary, the old one is told by push (`replaced`) and the new one too
/// (`handover`).
#[post("/reins/api/roster", data = "<data>")]
async fn post_roster(
    data: Data<'_>,
    headers: Headers,
    key: DeviceKey,
    conn: DbConn,
    pool: &State<DbPool>,
) -> PhoneResult<Json<RosterAppended>> {
    let user = user_key(&headers);
    let caller = headers.device.uuid.to_string();
    let append: RosterAppend = serde_json::from_slice(&read_body_limited(data, MAX_ROSTER_BODY).await?)
        .map_err(|e| bad_request(format!("invalid body: {e}")))?;
    let entry = append.entry;
    if entry.account != user {
        return Err(api_err(Status::BadRequest, codes::BAD_ROSTER, "The entry names another account"));
    }
    let entries = chain(&user, &conn).await?;
    let prev = entries.last();
    if let Some(prev) = prev
        && entry.epoch <= prev.epoch
    {
        return Err(api_err(
            Status::Conflict,
            codes::EPOCH_CONFLICT,
            format!(
                "The roster is at epoch {}; an entry for epoch {} is a rollback or a replay",
                prev.epoch, entry.epoch
            ),
        ));
    }

    // Who may append this entry.
    let caller_key = key.hash().map(str::to_owned);
    match (entry.reason, prev) {
        (RosterReason::Create, None) => {
            require_approval_device(&headers, &key, &conn).await?;
            if entry.primary.device_id != caller {
                return Err(api_err(
                    Status::BadRequest,
                    codes::BAD_ROSTER,
                    "The approval device creates the roster as its primary",
                ));
            }
            if caller_key.is_none() {
                return Err(bad_request("Creating a roster needs the device key header"));
            }
            if append.recovery_wrap.is_none() {
                return Err(api_err(Status::BadRequest, codes::BAD_ROSTER, "A new roster carries the recovery wrap"));
            }
        }
        (_, None) => return Err(no_roster()),
        (RosterReason::Recovery, Some(_)) => {
            if entry.primary.device_id != caller {
                return Err(api_err(
                    Status::BadRequest,
                    codes::BAD_ROSTER,
                    "A recovery makes the device asking primary",
                ));
            }
            if caller_key.is_none() {
                return Err(bad_request("A recovery needs the device key header"));
            }
            check_recovery_proof(&headers, append.master_password_hash.as_deref())?;
        }
        (_, Some(prev)) => {
            require_approval_device(&headers, &key, &conn).await?;
            if prev.primary.device_id != caller {
                return Err(api_err(
                    Status::Forbidden,
                    codes::NOT_APPROVAL_DEVICE,
                    "Only the primary changes the roster",
                ));
            }
        }
    }
    // Counted once the caller may make this change, so another device of the account cannot spend the budget.
    if let Err(wait) = super::limits::ROSTER_CHANGES.check(&user) {
        return Err(rate_limited("too many roster changes for this account", wait));
    }
    verify_step(prev, &entry, ed25519_verify).map_err(|e| roster_error(&e))?;
    match (entry.reason, &append.bundle) {
        (RosterReason::Handover, Some(bundle)) => check_payload("bundle", bundle, MAX_BUNDLE_CHARS)?,
        (RosterReason::Handover, None) => {
            return Err(api_err(Status::BadRequest, codes::BAD_ROSTER, "A handover carries the sealed bundle"));
        }
        (_, Some(_)) => return Err(api_err(Status::BadRequest, codes::BAD_ROSTER, "Only a handover carries a bundle")),
        (_, None) => {}
    }
    if let Some(wrap) = &append.recovery_wrap {
        check_payload("recovery_wrap", wrap, MAX_RECOVERY_WRAP_CHARS)?;
    }

    // Bind each member to a device key.
    let members = ReinsMember::all(&user, &conn).await.map_err(|e| internal(&e))?;
    let now = now_unix();
    let mut listed = Vec::new();
    for (role, member) in
        std::iter::once((ROLE_PRIMARY, &entry.primary)).chain(entry.viewers.iter().map(|v| (ROLE_VIEWER, v)))
    {
        let kept = members
            .iter()
            .find(|m| m.device_uuid == member.device_id && m.sign_pub == member.sign_pub && m.is_current());
        let key_hash = if member.device_id == caller
            && matches!(entry.reason, RosterReason::Create | RosterReason::Recovery)
        {
            caller_key.clone()
        } else if kept.is_some() {
            None
        } else {
            let bound = HUB.joins.enrollment_binding(&user, &member.device_id, &member.box_pub, &member.sign_pub, now);
            if bound.is_none() {
                return Err(api_err(
                    Status::BadRequest,
                    codes::BAD_ROSTER,
                    format!(
                        "{} has not enrolled from its device: a new member needs an enrollment join with these keys that went through the challenge and the reveal",
                        member.device_id
                    ),
                ));
            }
            bound
        };
        listed.push(Listed {
            device_uuid: member.device_id.clone(),
            role,
            sign_pub: member.sign_pub.clone(),
            key_hash,
        });
    }

    let hash = entry.hash_hex().map_err(|e| bad_request(e.to_string()))?;
    let epoch = i64::try_from(entry.epoch).map_err(|_| bad_request("epoch out of range"))?;
    let row = ReinsRosterRow {
        user_uuid: user.clone(),
        epoch,
        entry: serde_json::to_string(&entry).map_err(|e| bad_request(e.to_string()))?,
        entry_hash: hash.clone(),
        recovery_wrap: append.recovery_wrap,
        bundle: append.bundle,
        created_at: now,
    };
    match ReinsRoster::append(row, listed, now, &conn).await {
        Ok(()) => {}
        Err(WriteError::Conflict | WriteError::Retry) => {
            return Err(api_err(
                Status::Conflict,
                codes::EPOCH_CONFLICT,
                "The roster changed meanwhile; fetch it again",
            ));
        }
        Err(WriteError::Full) => return Err(bad_request("roster full")),
        Err(WriteError::Db(e)) => return Err(internal(&e)),
    }
    info!("Reins: roster of user {user} is at epoch {} ({})", entry.epoch, entry.reason.as_str());

    let primary_moved = prev.is_some_and(|p| p.primary.device_id != entry.primary.device_id);
    if primary_moved {
        move_approval_role(&headers, &entry, now, &conn, pool.inner()).await?;
    }
    HUB.live.bump(&user);
    // Every other member learns of the new epoch on its next poll; wake the viewers too.
    let members = ReinsMember::all(&user, &conn).await.map_err(|e| internal(&e))?;
    let head = super::live::log_head(&user, &conn).await.unwrap_or(0);
    for m in members.iter().filter(|m| m.role == ROLE_VIEWER && m.device_uuid != caller) {
        push::spawn_member_push(
            pool.inner().clone(),
            headers.user.uuid.clone(),
            m.fcm_token.clone(),
            PushMessage::sync(head, false),
        );
    }
    Ok(Json(RosterAppended {
        epoch: entry.epoch,
        hash,
    }))
}

/// The roster's new primary takes the approval role: its member row's device key and push token become the
/// approval device's, waiting items are handed to it, the old approval device is told (`replaced`) and the new one
/// too (`handover`, with the id of the request it made, if any).
async fn move_approval_role(
    headers: &Headers,
    entry: &RosterEntry,
    now: i64,
    conn: &DbConn,
    pool: &DbPool,
) -> PhoneResult<()> {
    let user = user_key(headers);
    let member = ReinsMember::find(&user, &entry.primary.device_id, conn)
        .await
        .map_err(|e| internal(&e))?
        .ok_or_else(|| internal(&crate::Error::new("The new primary has no member row", "")))?;
    let row = ReinsDevice {
        user_uuid: headers.user.uuid.clone(),
        device_uuid: entry.primary.device_id.clone().into(),
        fcm_token: member.fcm_token.clone(),
        updated_at: now,
        key_hash: member.key_hash.clone(),
    };
    let previous = row.replace(conn).await.map_err(|e| internal(&e))?;
    HUB.relay.requeue(&user);
    HUB.pairings.requeue(&user);
    HUB.joins.requeue(&user);
    HUB.handovers.requeue(&user);
    let handover_id = HUB.handovers.complete(&user, &entry.primary.device_id, entry.epoch, now);
    if let Some(old) = previous.filter(|old| old.device_uuid.to_string() != entry.primary.device_id) {
        push::spawn_push(
            pool.clone(),
            headers.user.uuid.clone(),
            old.fcm_token,
            PushMessage {
                t: PushKind::Replaced,
                id: String::new(),
            },
        );
    }
    push::spawn_push(
        pool.clone(),
        headers.user.uuid.clone(),
        member.fcm_token,
        PushMessage {
            t: PushKind::Handover,
            id: handover_id.unwrap_or_else(|| entry.epoch.to_string()),
        },
    );
    if entry.reason == RosterReason::Recovery {
        warn!("Reins: user {user} recovered the primary role on device {}", entry.primary.device_id);
    }
    Ok(())
}

/// A recovery's proof: the master password hash of the recovery code or master password, counted against the
/// account like takeover proofs.
fn check_recovery_proof(headers: &Headers, hash: Option<&str>) -> PhoneResult<()> {
    let user = user_key(headers);
    let Some(hash) = hash.filter(|h| !h.is_empty()) else {
        return Err(api_err(
            Status::Forbidden,
            codes::PROOF_REQUIRED,
            "A recovery needs the recovery code or master password",
        ));
    };
    if let Err(wait) = super::limits::DEVICE_PROOFS.check(&user) {
        return Err(rate_limited("too many wrong recovery codes or passwords for this account", wait));
    }
    if headers.user.check_valid_password(hash) {
        return Ok(());
    }
    super::limits::DEVICE_PROOFS.fail(&user);
    warn!("Reins: a wrong proof to recover the primary role of user {user}");
    Err(api_err(Status::Forbidden, codes::WRONG_PROOF, "That is neither the recovery code nor the master password"))
}

/// The primary re-wraps the owner seed and view key (the master password changed).
#[put("/reins/api/roster/recovery-wrap", data = "<data>")]
async fn put_recovery_wrap(data: Data<'_>, headers: Headers, key: DeviceKey, conn: DbConn) -> PhoneResult<Status> {
    require_approval_device(&headers, &key, &conn).await?;
    let user = user_key(&headers);
    let latest = ReinsRoster::latest(&user, &conn).await.map_err(|e| internal(&e))?.ok_or_else(no_roster)?;
    let update: RecoveryWrapUpdate = serde_json::from_slice(&read_body_limited(data, 16 * 1024).await?)
        .map_err(|e| bad_request(format!("invalid body: {e}")))?;
    check_payload("recovery_wrap", &update.recovery_wrap, MAX_RECOVERY_WRAP_CHARS)?;
    ReinsRoster::set_recovery_wrap(&user, latest.epoch, &update.recovery_wrap, &conn)
        .await
        .map_err(|e| internal(&e))?;
    Ok(Status::NoContent)
}

/// A member's push token (viewers get `sync` pushes with it).
#[put("/reins/api/member", data = "<data>")]
async fn put_member(data: Data<'_>, headers: Headers, key: DeviceKey, conn: DbConn) -> PhoneResult<Status> {
    let member = require_member(&headers, &key, &conn).await?;
    let registration: MemberRegistration = serde_json::from_slice(&read_body_limited(data, 8 * 1024).await?)
        .map_err(|e| bad_request(format!("invalid body: {e}")))?;
    let token = normalize_fcm_token(registration.fcm_token).map_err(bad_request)?;
    ReinsMember::set_token(&member.user_uuid, &member.device_uuid, token, now_unix(), &conn)
        .await
        .map_err(|e| internal(&e))?;
    Ok(Status::NoContent)
}

/// Open handover requests one account may have.
const MAX_HANDOVERS_PER_USER: usize = 3;

struct HandoverEntry {
    user: String,
    device: String,
    request: HandoverRequest,
    delivered: bool,
    status: HandoverStatus,
    epoch: Option<u64>,
    expires_at: i64,
}

/// Viewers asking to become primary ("Decide on this phone"), in memory for [`ITEM_TTL`]. Limited per account only.
pub struct HandoverHub {
    entries: Mutex<HashMap<String, HandoverEntry>>,
    signal: Arc<ItemSignal>,
}

/// What `GET /reins/api/handovers/<id>` shows the caller.
#[derive(Debug, PartialEq, Eq)]
pub enum HandoverView {
    Requester(HandoverState),
    Approver(HandoverRequest),
}

fn ttl_secs() -> i64 {
    i64::try_from(ITEM_TTL.as_secs()).unwrap_or(600)
}

impl HandoverHub {
    pub fn new(signal: Arc<ItemSignal>) -> Self {
        Self {
            entries: Mutex::default(),
            signal,
        }
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<String, HandoverEntry>> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A new request of `device` (replacing its earlier one); `None` when the account has
    /// [`MAX_HANDOVERS_PER_USER`] open.
    pub fn create(&self, user: &str, device: &str, name: &str, now: i64) -> Option<HandoverCreated> {
        let mut entries = self.lock();
        entries.retain(|_, e| e.expires_at > now && !(e.user == user && e.device == device));
        if entries.values().filter(|e| e.user == user).count() >= MAX_HANDOVERS_PER_USER {
            return None;
        }
        let id = get_uuid();
        let expires_at = now + ttl_secs();
        entries.insert(
            id.clone(),
            HandoverEntry {
                user: user.to_owned(),
                device: device.to_owned(),
                request: HandoverRequest {
                    v: PROTOCOL_VERSION,
                    id: id.clone(),
                    device_id: device.to_owned(),
                    device_name: name.to_owned(),
                    created_at: now,
                },
                delivered: false,
                status: HandoverStatus::Waiting,
                epoch: None,
                expires_at,
            },
        );
        drop(entries);
        self.signal.notify();
        Some(HandoverCreated {
            id,
            expires_at,
        })
    }

    fn waiting<'a>(
        entries: &'a HashMap<String, HandoverEntry>,
        user: &'a str,
        now: i64,
    ) -> impl Iterator<Item = &'a HandoverEntry> {
        entries.values().filter(move |e| e.user == user && e.status == HandoverStatus::Waiting && e.expires_at > now)
    }

    pub fn take_undelivered(&self, user: &str, now: i64) -> Vec<HandoverRequest> {
        let mut entries = self.lock();
        let mut out: Vec<HandoverRequest> = entries
            .values_mut()
            .filter(|e| e.user == user && !e.delivered && e.status == HandoverStatus::Waiting && e.expires_at > now)
            .map(|e| {
                e.delivered = true;
                e.request.clone()
            })
            .collect();
        out.sort_by_key(|r| r.created_at);
        out
    }

    pub fn peek_waiting(&self, user: &str, now: i64) -> Vec<HandoverRequest> {
        let entries = self.lock();
        let mut out: Vec<HandoverRequest> = Self::waiting(&entries, user, now).map(|e| e.request.clone()).collect();
        out.sort_by_key(|r| r.created_at);
        out
    }

    pub fn view(&self, user: &str, device: &str, id: &str, now: i64) -> Option<HandoverView> {
        let mut entries = self.lock();
        let entry = entries.get_mut(id).filter(|e| e.user == user)?;
        if entry.device == device {
            let status = if entry.status == HandoverStatus::Waiting && entry.expires_at <= now {
                HandoverStatus::Expired
            } else {
                entry.status
            };
            return Some(HandoverView::Requester(HandoverState {
                status,
                epoch: entry.epoch,
            }));
        }
        entry.delivered = true;
        (entry.status == HandoverStatus::Waiting && entry.expires_at > now)
            .then(|| HandoverView::Approver(entry.request.clone()))
    }

    /// The primary said no.
    pub fn deny(&self, user: &str, id: &str, now: i64) -> Result<(), bool> {
        let mut entries = self.lock();
        let entry = entries.get_mut(id).filter(|e| e.user == user && e.expires_at > now).ok_or(false)?;
        if entry.status != HandoverStatus::Waiting {
            return Err(true);
        }
        entry.status = HandoverStatus::Denied;
        Ok(())
    }

    /// A roster entry made `device` primary: its waiting request (if any) is done; returns its id.
    pub fn complete(&self, user: &str, device: &str, epoch: u64, now: i64) -> Option<String> {
        let mut entries = self.lock();
        let mut done = None;
        for (id, e) in entries.iter_mut() {
            if e.user == user && e.device == device && e.status == HandoverStatus::Waiting && e.expires_at > now {
                e.status = HandoverStatus::Done;
                e.epoch = Some(epoch);
                done = Some(id.clone());
            }
        }
        done
    }

    pub fn requeue(&self, user: &str) {
        for e in self.lock().values_mut().filter(|e| e.user == user && e.status == HandoverStatus::Waiting) {
            e.delivered = false;
        }
    }

    pub fn answered_ids(&self, user: &str) -> Vec<String> {
        let mut ids: Vec<String> = self
            .lock()
            .iter()
            .filter(|(_, e)| e.user == user && e.status != HandoverStatus::Waiting)
            .map(|(id, _)| id.clone())
            .collect();
        ids.sort();
        ids
    }

    pub fn purge(&self, now: i64) {
        self.lock().retain(|_, e| e.expires_at + ttl_secs() > now);
    }

    pub fn forget_user(&self, user: &str) {
        self.lock().retain(|_, e| e.user != user);
    }
}

/// A viewer asks to become primary; the primary sees it in `GET /pending` (and is pushed `handover`).
#[post("/reins/api/handovers", data = "<data>")]
async fn post_handover(
    data: Data<'_>,
    headers: Headers,
    key: DeviceKey,
    conn: DbConn,
    pool: &State<DbPool>,
) -> PhoneResult<Json<HandoverCreated>> {
    let member = require_member(&headers, &key, &conn).await?;
    if member.role != ROLE_VIEWER {
        return Err(bad_request("This is the primary"));
    }
    let _: NewHandover = parse_versioned(&read_body_limited(data, 4096).await?)?;
    let user = user_key(&headers);
    let entries = chain(&user, &conn).await?;
    let name = entries
        .last()
        .and_then(|e| e.member(&member.device_uuid))
        .map_or_else(|| "A phone".to_owned(), |m| m.name.clone());
    let created = HUB
        .handovers
        .create(&user, &member.device_uuid, &name, now_unix())
        .ok_or_else(|| api_err(Status::TooManyRequests, codes::RATE_LIMITED, "Too many open handover requests"))?;
    if let Some(approval) = ReinsDevice::find_by_user(&headers.user.uuid, &conn).await {
        push::spawn_push(
            pool.inner().clone(),
            headers.user.uuid.clone(),
            approval.fcm_token,
            PushMessage {
                t: PushKind::Handover,
                id: created.id.clone(),
            },
        );
    }
    HUB.live.bump(&user);
    Ok(Json(created))
}

#[get("/reins/api/handovers/<id>")]
async fn get_handover(
    id: &str,
    headers: Headers,
    key: DeviceKey,
    conn: DbConn,
) -> PhoneResult<Json<serde_json::Value>> {
    let view = HUB
        .handovers
        .view(&user_key(&headers), &headers.device.uuid.to_string(), id, now_unix())
        .ok_or_else(not_found)?;
    match view {
        HandoverView::Requester(state) => Ok(Json(serde_json::to_value(state).unwrap_or_default())),
        HandoverView::Approver(request) => {
            require_approval_device(&headers, &key, &conn).await?;
            Ok(Json(serde_json::to_value(request).unwrap_or_default()))
        }
    }
}

/// The primary denies a handover request (a yes is the `handover` roster entry).
#[post("/reins/api/handovers/<id>/response", data = "<data>")]
async fn post_handover_response(
    id: &str,
    data: Data<'_>,
    headers: Headers,
    key: DeviceKey,
    conn: DbConn,
) -> PhoneResult<Status> {
    require_approval_device(&headers, &key, &conn).await?;
    let answer: HandoverAnswer = parse_versioned(&read_body_limited(data, 4096).await?)?;
    if answer.approve {
        return Err(bad_request("Approve a handover by appending the `handover` roster entry"));
    }
    let user = user_key(&headers);
    HUB.handovers.deny(&user, id, now_unix()).map_err(|answered| {
        if answered {
            super::device_api::already_answered()
        } else {
            not_found()
        }
    })?;
    HUB.live.bump(&user);
    Ok(Status::NoContent)
}

/// The sealed bundle of the latest `handover`, for the primary it named.
#[get("/reins/api/handover-bundle")]
async fn get_handover_bundle(headers: Headers, key: DeviceKey, conn: DbConn) -> PhoneResult<Json<HandoverBundle>> {
    require_approval_device(&headers, &key, &conn).await?;
    let latest =
        ReinsRoster::latest(&user_key(&headers), &conn).await.map_err(|e| internal(&e))?.ok_or_else(no_roster)?;
    let sealed = latest.bundle.ok_or_else(not_found)?;
    Ok(Json(HandoverBundle {
        epoch: u64::try_from(latest.epoch).unwrap_or(0),
        sealed,
    }))
}

/// The new primary has the keys: the server drops the bundle.
#[delete("/reins/api/handover-bundle")]
async fn delete_handover_bundle(headers: Headers, key: DeviceKey, conn: DbConn) -> PhoneResult<Status> {
    require_approval_device(&headers, &key, &conn).await?;
    ReinsRoster::clear_bundle(&user_key(&headers), &conn).await.map_err(|e| internal(&e))?;
    Ok(Status::NoContent)
}

/// For `PUT /reins/api/device` on an account with a roster: only the roster's primary, with its enrolled device key,
/// may (re)register as the approval device. `Ok(None)`: the account has no roster.
pub async fn roster_primary_registration(headers: &Headers, key: &DeviceKey, conn: &DbConn) -> PhoneResult<Option<()>> {
    let user = user_key(headers);
    let Some(latest) = ReinsRoster::latest(&user, conn).await.map_err(|e| internal(&e))? else {
        return Ok(None);
    };
    let entry: RosterEntry = serde_json::from_str(&latest.entry)
        .map_err(|e| internal(&crate::Error::new("A stored Reins roster entry does not parse", e.to_string())))?;
    let member = ReinsMember::find(&user, &headers.device.uuid.to_string(), conn).await.map_err(|e| internal(&e))?;
    let is_primary = entry.primary.device_id == headers.device.uuid.to_string()
        && member.is_some_and(|m| {
            m.role == ROLE_PRIMARY
                && matches!((&m.key_hash, key.hash()), (Some(kept), Some(given)) if crate::crypto::ct_eq(kept, given))
        });
    if is_primary {
        Ok(Some(()))
    } else {
        Err(api_err(
            Status::Conflict,
            codes::USE_HANDOVER,
            "This account has phone roles: the primary hands over, or another phone recovers with the recovery code",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handover_requests_wait_for_the_primary_and_end_with_the_roster() {
        let hub = HandoverHub::new(Arc::new(ItemSignal::new()));
        let created = hub.create("u1", "viewer", "Pixel", 100).unwrap();
        let again = hub.create("u1", "viewer", "Pixel", 101).unwrap();
        assert!(hub.view("u1", "viewer", &created.id, 101).is_none(), "a device's new request replaces its old one");
        assert!(hub.create("u1", "v2", "b", 101).is_some() && hub.create("u1", "v3", "c", 101).is_some());
        assert!(hub.create("u1", "v4", "d", 101).is_none(), "a few per account");
        assert!(hub.create("u2", "v4", "d", 101).is_some(), "per account, not for the server");
        assert_eq!(hub.peek_waiting("u1", 102).len(), 3);
        assert_eq!(hub.take_undelivered("u1", 102).len(), 3);
        assert!(hub.take_undelivered("u1", 102).is_empty());
        assert_eq!(hub.complete("u1", "viewer", 4, 103).as_deref(), Some(again.id.as_str()));
        assert_eq!(
            hub.view("u1", "viewer", &again.id, 103),
            Some(HandoverView::Requester(HandoverState {
                status: HandoverStatus::Done,
                epoch: Some(4)
            }))
        );
        let other = hub.peek_waiting("u1", 103)[0].id.clone();
        assert_eq!(hub.deny("u1", &other, 104), Ok(()));
        assert_eq!(hub.deny("u1", &other, 104), Err(true));
        assert_eq!(hub.deny("u2", &other, 104), Err(false));
        assert_eq!(hub.answered_ids("u1").len(), 2);
        hub.forget_user("u1");
        assert!(hub.peek_waiting("u1", 104).is_empty());
    }

    #[test]
    fn a_primary_signs_only_within_its_tenure() {
        use reins_proto::{desktop::encode_key, roster::Member};
        let m = |id: &str, n: u8| Member {
            device_id: id.to_owned(),
            name: id.to_owned(),
            sign_pub: encode_key(&[n; 32]),
            box_pub: encode_key(&[n + 50; 32]),
        };
        let e1 = RosterEntry::genesis("acct", m("p", 1), encode_key(&[9; 32]), encode_key(&[8; 32]), 1);
        let e2 = e1.successor(RosterReason::Enroll, m("p", 1), vec![m("v", 2)], 2).unwrap();
        let e3 = e2.successor(RosterReason::Handover, m("v", 2), vec![m("p", 1)], 3).unwrap();
        let chain = [e1, e2, e3];
        assert_eq!(primary_at(&chain, 3).map(|m| m.device_id.as_str()), Some("v"));
        assert_eq!(primary_at(&chain, 2), None, "p is no longer primary");
        assert_eq!(primary_at(&chain, 4), None);
        assert_eq!(primary_at(&chain[..2], 1).map(|m| m.device_id.as_str()), Some("p"), "p over its whole tenure");
    }
}
