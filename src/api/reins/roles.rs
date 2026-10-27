//! Phone roles (`reins_proto::roster`): one primary decides, viewers watch. The server stores the owner-signed roster
//! chain and enforces it: it appends an entry only when the owner key's signature verifies, the epoch is the next one
//! (compare-and-swap) and the caller may make that change; it binds each member to the device key it enrolled with;
//! and it moves the approval role (`reins_devices`) with the roster's primary.
//!
//! An account without a roster is a legacy account: nothing here changes how it works. `GET /reins/api/roster`
//! answers an empty list then (404 means a server without phone roles).
//!
//! - `GET /reins/api/roster` (any signed-in device of the account): the chain.
//! - `POST /reins/api/roster`: the next entry. `create` from the approval device, `enroll`/`remove`/`handover`/
//!   `rewrap` from the primary, `recovery` from the device it makes primary, with the master password hash.
//! - `POST /reins/api/roster/recovery-wrap` (any device of the account, with the master password hash): the wrap.
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
    oplog::{Checkpoint, Heartbeat, MAX_RECOVERY_LOG_BYTES, MAX_SYNC_OPS, RecoveryLog, RecoveryLogRequest, SyncedOp},
    pairing::{PushKind, PushMessage},
    roster::{
        HANDOVER_REQUEST_SKEW_SECS, HandoverAnswer, HandoverBundle, HandoverCreated, HandoverRequest,
        HandoverSignature, HandoverState, HandoverStatus, MAX_BUNDLE_CHARS, MAX_RECOVERY_WRAP_CHARS, Member,
        MemberRegistration, NewHandover, RecoveryWrap, RecoveryWrapRequest, Roster, RosterAppend, RosterAppended,
        RosterEntry, RosterError, RosterReason, handover_request_message, recovery_wrap_hash, verify_step,
    },
};
use rocket::{Data, Route, State, http::Status, serde::json::Json};

use super::{
    HUB, ITEM_TTL,
    device_api::{
        DeviceKey, PhoneResult, api_err, bad_request, clear_proven_device, internal, normalize_fcm_token, not_found,
        parse_versioned, rate_limited, read_body_limited, require_approval_device, user_key,
    },
    now_unix, push,
    relay::ItemSignal,
};
use crate::{
    auth::Headers,
    db::{
        DbConn, DbPool,
        models::{
            Listed, ROLE_PRIMARY, ROLE_VIEWER, ReinsDevice, ReinsMember, ReinsOps, ReinsRoster, ReinsRosterRow,
            WriteError,
        },
    },
    util::get_uuid,
};

pub fn routes() -> Vec<Route> {
    routes![
        get_roster,
        post_roster,
        post_recovery_wrap,
        post_recovery_log,
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
    Ok(Json(Roster {
        entries: chain(&user_key(&headers), &conn).await?,
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

fn bad_roster(message: impl Into<String>) -> rocket::response::status::Custom<Json<reins_proto::device::ApiError>> {
    api_err(Status::BadRequest, codes::BAD_ROSTER, message)
}

fn check_payload(field: &str, value: &str, max: usize) -> PhoneResult<()> {
    if value.is_empty() || value.len() > max || reins_proto::canon::decode_payload(value).is_none() {
        return Err(bad_roster(format!("{field} must be 1 to {max} characters of base64url")));
    }
    Ok(())
}

/// The next roster entry. Who may append what:
///
/// - `create` (no roster yet): the approval device, naming itself primary;
/// - `enroll`, `remove`, `handover`, `rewrap`: the current primary; a new viewer must have enrolled through a join
///   from its device ([`super::join::JoinHub::take_enrollment_binding`], spent by this entry), and a handover carries
///   the sealed bundle;
/// - `recovery`: the device the entry makes primary, with the master password hash (counted like takeover proofs).
///
/// `create`, `recovery` and `rewrap` carry the recovery wrap the entry names by hash; the others none. The entry
/// must verify against the owner key (`verify_step`) and name the next epoch; a concurrent append makes the later one
/// fail with 409 `epoch_conflict`. When the primary changes, the approval role moves in the same transaction as the
/// append; then unanswered requests, pairings and joins go to the new primary, and the old one is told by push
/// (`replaced`) and the new one too (`handover`).
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
        return Err(bad_roster("The entry names another account"));
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
            // The wrap is sealed under the master key: the creating phone proves it holds that key, so a mistyped
            // master password never seals a wrap that opens for nobody.
            check_recovery_proof(&headers, &key, append.master_password_hash.as_deref())?;
            if entry.primary.device_id != caller {
                return Err(bad_roster("The approval device creates the roster as its primary"));
            }
            if caller_key.is_none() {
                return Err(bad_request("Creating a roster needs the device key header"));
            }
        }
        (_, None) => return Err(no_roster()),
        (RosterReason::Recovery, Some(_)) => {
            if entry.primary.device_id != caller {
                return Err(bad_roster("A recovery makes the device asking primary"));
            }
            if caller_key.is_none() {
                return Err(bad_request("A recovery needs the device key header"));
            }
            check_recovery_proof(&headers, &key, append.master_password_hash.as_deref())?;
        }
        (_, Some(_)) => {
            // The roster's primary, with its enrolled device key (`require_approval_device` checks the roster).
            require_approval_device(&headers, &key, &conn).await?;
        }
    }
    // Counted once the caller may make this change, so another device of the account cannot spend the budget.
    if let Err(wait) = super::limits::ROSTER_CHANGES.check(&user) {
        return Err(rate_limited("too many roster changes for this account", wait));
    }
    verify_step(prev, &entry, ed25519_verify).map_err(|e| roster_error(&e))?;
    match (entry.reason, &append.bundle) {
        (RosterReason::Handover, Some(bundle)) => check_payload("bundle", bundle, MAX_BUNDLE_CHARS)?,
        (RosterReason::Handover, None) => return Err(bad_roster("A handover carries the sealed bundle")),
        (_, Some(_)) => return Err(bad_roster("Only a handover carries a bundle")),
        (_, None) => {}
    }
    let new_wrap = entry.reason.brings_wrap();
    match (&append.recovery_wrap, new_wrap) {
        (Some(wrap), true) => {
            check_payload("recovery_wrap", wrap, MAX_RECOVERY_WRAP_CHARS)?;
            if recovery_wrap_hash(wrap).as_deref() != Some(entry.recovery_wrap_hash.as_str()) {
                return Err(bad_roster("The recovery wrap is not the one the entry names"));
            }
        }
        (None, true) => {
            return Err(bad_roster(
                "A create, recovery, rewrap, removal or handover entry carries its new recovery wrap",
            ));
        }
        (Some(_), false) => return Err(bad_roster("An enrollment carries no recovery wrap")),
        (None, false) => {}
    }

    // Bind each member to a device key. A new viewer spends the enrollment it went through.
    let members = ReinsMember::all(&user, &conn).await.map_err(|e| internal(&e))?;
    // A handover or recovery names the outgoing primary's last op; the phones take its ops up to there as its history,
    // so it may name none past what this server stored of it.
    if let (Some(named), Some(prev)) = (entry.primary_counter, prev) {
        let stored = members
            .iter()
            .find(|m| m.device_uuid == prev.primary.device_id && m.sign_pub == prev.primary.sign_pub)
            .map_or(0, |m| m.last_counter);
        if i64::try_from(named).map_or(true, |n| n > stored) {
            return Err(bad_roster(format!(
                "The entry names op {named} of the outgoing primary; the server stored up to {stored}"
            )));
        }
    }
    let now = now_unix();
    let mut listed = Vec::new();
    let mut spent: Vec<String> = Vec::new();
    for (role, member) in
        std::iter::once((ROLE_PRIMARY, &entry.primary)).chain(entry.viewers.iter().map(|v| (ROLE_VIEWER, v)))
    {
        let kept = members
            .iter()
            .any(|m| m.device_uuid == member.device_id && m.sign_pub == member.sign_pub && m.is_current());
        let key_hash =
            if member.device_id == caller && matches!(entry.reason, RosterReason::Create | RosterReason::Recovery) {
                caller_key.clone()
            } else if kept {
                None
            } else {
                let Some((join, bound)) =
                    HUB.joins.take_enrollment_binding(&user, &member.device_id, &member.box_pub, &member.sign_pub, now)
                else {
                    for join in &spent {
                        HUB.joins.release_binding(join);
                    }
                    return Err(bad_roster(format!(
                        "{} has not enrolled from its device: a new member needs an enrollment join with these keys \
                     that went through the challenge and the reveal",
                        member.device_id
                    )));
                };
                spent.push(join);
                Some(bound)
            };
        listed.push(Listed {
            device_uuid: member.device_id.clone(),
            role,
            sign_pub: member.sign_pub.clone(),
            key_hash,
        });
    }
    // Current members this entry drops. Captured before `listed` moves into the append.
    let leaving: Vec<(String, Option<String>)> = members
        .iter()
        .filter(|m| m.is_current() && !listed.iter().any(|l| l.device_uuid == m.device_uuid))
        .map(|m| (m.device_uuid.clone(), m.key_hash.clone()))
        .collect();

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
    let primary_moved = prev.is_some_and(|p| p.primary.device_id != entry.primary.device_id);
    let move_role = primary_moved.then(|| entry.primary.device_id.clone());
    let replaced = match ReinsRoster::append(row, listed, move_role, now, &conn).await {
        Ok(replaced) => replaced,
        Err(e) => {
            for join in &spent {
                HUB.joins.release_binding(join);
            }
            return Err(match e {
                WriteError::Conflict | WriteError::Retry => {
                    api_err(Status::Conflict, codes::EPOCH_CONFLICT, "The roster changed meanwhile; fetch it again")
                }
                WriteError::Full => bad_request("roster full"),
                WriteError::Db(e) => internal(&e),
            });
        }
    };
    info!("Reins: roster of user {user} is at epoch {} ({})", entry.epoch, entry.reason.as_str());
    for (device, key) in &leaving {
        clear_proven_device(&user, device, key.as_deref(), &conn).await;
    }

    if primary_moved {
        announce_new_primary(&headers, &entry, replaced, now, &conn, pool.inner()).await?;
    }
    // A ward's guardians hear of a new approval phone, a phone removed or a recovery (08 R7); their phones check it
    // against the chain they pinned.
    if matches!(entry.reason, RosterReason::Handover | RosterReason::Recovery | RosterReason::Remove) {
        let notice = reins_proto::circle::GuardianNoticeKind::RosterChanged {
            reason: entry.reason,
            epoch: entry.epoch,
        };
        if super::guardian_notices::notify_guardians(&user, &notice, &conn).await > 0 {
            super::guardian_notices::push_pending(&conn, pool.inner()).await;
        }
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

/// After the role moved (with the append): waiting items go to the new primary, the old approval device is told
/// (`replaced`) and the new one too (`handover`, with the id of the request it made, if any).
async fn announce_new_primary(
    headers: &Headers,
    entry: &RosterEntry,
    replaced: Option<ReinsDevice>,
    now: i64,
    conn: &DbConn,
    pool: &DbPool,
) -> PhoneResult<()> {
    let user = user_key(headers);
    let member = ReinsMember::find(&user, &entry.primary.device_id, conn).await.map_err(|e| internal(&e))?;
    HUB.relay.requeue(&user);
    HUB.pairings.requeue(&user);
    HUB.joins.requeue(&user);
    HUB.handovers.requeue(&user);
    let handover_id = HUB.handovers.complete(&user, &entry.primary.device_id, entry.epoch, now);
    if let Some(old) = replaced.filter(|old| old.device_uuid.to_string() != entry.primary.device_id) {
        push::spawn_push(
            pool.clone(),
            headers.user.uuid.clone(),
            old.fcm_token,
            PushMessage {
                t: PushKind::Replaced,
                id: String::new(),
                p: None,
            },
        );
    }
    push::spawn_push(
        pool.clone(),
        headers.user.uuid.clone(),
        member.and_then(|m| m.fcm_token),
        PushMessage {
            t: PushKind::Handover,
            id: handover_id.unwrap_or_else(|| entry.epoch.to_string()),
            p: None,
        },
    );
    if entry.reason == RosterReason::Recovery {
        warn!("Reins: user {user} recovered the primary role on device {}", entry.primary.device_id);
    }
    Ok(())
}

/// A proof that the caller knows the recovery code or master password: its master password hash, counted against the
/// account like takeover proofs.
fn check_recovery_proof(headers: &Headers, device_key: &DeviceKey, hash: Option<&str>) -> PhoneResult<()> {
    let user = user_key(headers);
    // Counted twice: per device id (one stolen token cannot use up everyone's tries at once) and against the account's
    // limit shared with every other proof (new device ids never get more guesses). A device key that proved itself
    // before keeps its own tries when others used up the account's. No device key, no exemption.
    let device = format!("{user}/{}", headers.device.uuid);
    let Some(hash) = hash.filter(|h| !h.is_empty()) else {
        return Err(api_err(
            Status::Forbidden,
            codes::PROOF_REQUIRED,
            "This needs the recovery code or master password",
        ));
    };
    if let Err(wait) = super::limits::DEVICE_PROOFS_PER_DEVICE.check(&device) {
        return Err(rate_limited("too many wrong recovery codes or passwords from this device", wait));
    }
    if let Err(wait) = super::limits::DEVICE_PROOFS.check(&user)
        && !device_key.hash().is_some_and(|key| super::limits::PROVEN_DEVICES.is_proven(&user, key))
    {
        return Err(rate_limited("too many wrong recovery codes or passwords for this account", wait));
    }
    if headers.user.check_valid_password(hash) {
        if let Some(key) = device_key.hash() {
            super::limits::PROVEN_DEVICES.remember(&user, key, &headers.device.uuid.to_string());
        }
        return Ok(());
    }
    super::limits::DEVICE_PROOFS_PER_DEVICE.fail(&device);
    super::limits::DEVICE_PROOFS.fail(&user);
    warn!("Reins: a wrong recovery proof for user {user} from device {}", headers.device.uuid);
    Err(api_err(Status::Forbidden, codes::WRONG_PROOF, "That is neither the recovery code nor the master password"))
}

/// The recovery wrap the latest entry names, for a phone about to recover: only with the master password hash
/// (wrong ones count with the takeover proofs). The wrap opens only with the master key, but it is not handed to any
/// device of the account that merely asks.
#[post("/reins/api/roster/recovery-wrap", data = "<data>")]
async fn post_recovery_wrap(
    data: Data<'_>,
    headers: Headers,
    key: DeviceKey,
    conn: DbConn,
) -> PhoneResult<Json<RecoveryWrap>> {
    let request: RecoveryWrapRequest = serde_json::from_slice(&read_body_limited(data, 4096).await?)
        .map_err(|e| bad_request(format!("invalid body: {e}")))?;
    let latest =
        ReinsRoster::latest(&user_key(&headers), &conn).await.map_err(|e| internal(&e))?.ok_or_else(no_roster)?;
    check_recovery_proof(&headers, &key, Some(&request.master_password_hash))?;
    let recovery_wrap = latest.recovery_wrap.clone().ok_or_else(not_found)?;
    let entry: RosterEntry = serde_json::from_str(&latest.entry)
        .map_err(|e| internal(&crate::Error::new("A stored roster entry does not parse", e.to_string())))?;
    let members = ReinsMember::all(&user_key(&headers), &conn).await.map_err(|e| internal(&e))?;
    let primary_counter = members
        .iter()
        .find(|m| m.device_uuid == entry.primary.device_id && m.sign_pub == entry.primary.sign_pub)
        .and_then(|m| u64::try_from(m.last_counter).ok());
    Ok(Json(RecoveryWrap {
        epoch: u64::try_from(latest.epoch).unwrap_or(0),
        recovery_wrap,
        primary_counter,
    }))
}

/// The log, for a phone about to recover (no member yet): it applies it under the roster before its recovery, so that
/// the entry it signs names only the outgoing primary's ops it verified. Only with the recovery proof, like the wrap.
#[post("/reins/api/roster/recovery-log", data = "<data>")]
async fn post_recovery_log(
    data: Data<'_>,
    headers: Headers,
    key: DeviceKey,
    conn: DbConn,
) -> PhoneResult<Json<RecoveryLog>> {
    let request: RecoveryLogRequest = serde_json::from_slice(&read_body_limited(data, 4096).await?)
        .map_err(|e| bad_request(format!("invalid body: {e}")))?;
    let user = user_key(&headers);
    ReinsRoster::latest(&user, &conn).await.map_err(|e| internal(&e))?.ok_or_else(no_roster)?;
    check_recovery_proof(&headers, &key, Some(&request.master_password_hash))?;
    let state = ReinsOps::state(&user, &conn).await.map_err(|e| internal(&e))?;
    let from = i64::try_from(request.after).unwrap_or(i64::MAX).max(state.floor);
    let limit = i64::try_from(MAX_SYNC_OPS).unwrap_or(500);
    let rows = ReinsOps::after(&user, from, limit + 1, &conn).await.map_err(|e| internal(&e))?;
    let mut more = rows.len() > MAX_SYNC_OPS;
    let mut bytes = 0;
    let mut ops = Vec::new();
    for row in rows.iter().take(MAX_SYNC_OPS) {
        bytes += row.op.len();
        if bytes > MAX_RECOVERY_LOG_BYTES && !ops.is_empty() {
            more = true;
            break;
        }
        if let (Ok(seq), Ok(op)) = (u64::try_from(row.seq), serde_json::from_str(&row.op)) {
            ops.push(SyncedOp {
                seq,
                op,
            });
        }
    }
    let checkpoint = if request.with_checkpoint {
        ReinsOps::latest_checkpoint(&user, &conn)
            .await
            .map_err(|e| internal(&e))?
            .and_then(|row| serde_json::from_str::<Checkpoint>(&row.checkpoint).ok())
    } else {
        None
    };
    // The primary's latest signed heartbeat: the head the phone must reach before it vouches for anything.
    let members = ReinsMember::all(&user, &conn).await.map_err(|e| internal(&e))?;
    let heartbeat = members
        .iter()
        .find(|m| m.role == ROLE_PRIMARY)
        .and_then(|m| m.heartbeat.as_deref())
        .and_then(|h| serde_json::from_str::<Heartbeat>(h).ok());
    Ok(Json(RecoveryLog {
        checkpoint,
        heartbeat,
        floor_seq: u64::try_from(state.floor).unwrap_or(0),
        head_seq: u64::try_from(state.head).unwrap_or(0),
        ops,
        more,
    }))
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
    /// Nonces of signed requests seen per (user, device), until their signed time is out of the skew window: a
    /// replayed request is refused.
    nonces: Mutex<HashMap<(String, String, String), i64>>,
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
            nonces: Mutex::default(),
            signal,
        }
    }

    /// Records a signed request's nonce; false when this device sent it already (a replay).
    pub fn fresh_nonce(&self, user: &str, device: &str, signed: &HandoverSignature, now: i64) -> bool {
        let mut nonces = self.nonces.lock().unwrap_or_else(PoisonError::into_inner);
        nonces.retain(|_, at| now.abs_diff(*at) <= HANDOVER_REQUEST_SKEW_SECS.unsigned_abs());
        nonces.insert((user.to_owned(), device.to_owned(), signed.nonce.clone()), signed.at).is_none()
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<String, HandoverEntry>> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A new request of `device` (replacing its earlier one), and whether it replaced a waiting one (the primary was
    /// told of that already); `None` when the account has [`MAX_HANDOVERS_PER_USER`] open.
    pub fn create(
        &self,
        user: &str,
        device: &str,
        name: &str,
        signed: Option<HandoverSignature>,
        now: i64,
    ) -> Option<(HandoverCreated, bool)> {
        let mut entries = self.lock();
        let replaced = entries
            .values()
            .any(|e| e.user == user && e.device == device && e.status == HandoverStatus::Waiting && e.expires_at > now);
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
                    signed,
                },
                delivered: false,
                status: HandoverStatus::Waiting,
                epoch: None,
                expires_at,
            },
        );
        drop(entries);
        self.signal.notify();
        Some((
            HandoverCreated {
                id,
                expires_at,
            },
            replaced,
        ))
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

    /// The request as `device` may see it: its own state, or (only when `approver`, the caller already checked to be
    /// the primary) the request, which is then marked seen. Nobody else learns of it or changes it.
    pub fn view(&self, user: &str, device: &str, id: &str, now: i64, approver: bool) -> Option<HandoverView> {
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
        if !approver {
            return None;
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
    let new: NewHandover = parse_versioned(&read_body_limited(data, 4096).await?)?;
    let user = user_key(&headers);
    if let Err(wait) = super::limits::HANDOVER_REQUESTS.check(&(user.clone(), member.device_uuid.clone())) {
        return Err(rate_limited("too many handover requests from this phone", wait));
    }
    let entries = chain(&user, &conn).await?;
    // The viewer's signature is kept for the primary, which verifies it against its own roster; a bad one is refused
    // here already. (Unsigned requests are still taken from apps that predate it; the primary refuses them.)
    if let Some(signed) = &new.signed {
        signed.validate().map_err(|e| bad_request(e.to_string()))?;
        let current = entries.last().ok_or_else(no_roster)?;
        let signer = current.member(&member.device_uuid).and_then(|m| m.sign_key().ok());
        let message = handover_request_message(&user, signed.epoch, &member.device_uuid, &signed.nonce, signed.at);
        let verified = matches!((signer, message, reins_proto::canon::parse_sig(&signed.sig)),
            (Some(key), Some(message), Some(sig)) if ed25519_verify(&key, &message, &sig));
        let now = now_unix();
        if signed.epoch != current.epoch
            || !verified
            || now.abs_diff(signed.at) > HANDOVER_REQUEST_SKEW_SECS.unsigned_abs()
        {
            return Err(api_err(Status::Forbidden, codes::BAD_SIGNATURE, "The request is not signed by this phone"));
        }
        if signed.policy_enforcement.is_some()
            && !signer
                .is_some_and(|key| signed.policy_support_verifies(&user, &member.device_uuid, &key, ed25519_verify))
        {
            return Err(api_err(
                Status::Forbidden,
                codes::BAD_SIGNATURE,
                "The policy support is not signed by this phone",
            ));
        }
        if !HUB.handovers.fresh_nonce(&user, &member.device_uuid, signed, now) {
            return Err(super::device_api::already_answered());
        }
    }
    let name = entries
        .last()
        .and_then(|e| e.member(&member.device_uuid))
        .map_or_else(|| "A phone".to_owned(), |m| m.name.clone());
    let (created, replaced) = HUB
        .handovers
        .create(&user, &member.device_uuid, &name, new.signed, now_unix())
        .ok_or_else(|| api_err(Status::TooManyRequests, codes::RATE_LIMITED, "Too many open handover requests"))?;
    // A request that replaced this phone's waiting one is not pushed again.
    if !replaced && let Some(approval) = ReinsDevice::find_by_user(&headers.user.uuid, &conn).await {
        push::spawn_push(
            pool.inner().clone(),
            headers.user.uuid.clone(),
            approval.fcm_token,
            PushMessage {
                t: PushKind::Handover,
                id: created.id.clone(),
                p: None,
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
    // Authorized before anything is marked seen: a viewer must not take a request from the primary's list.
    let approval = require_approval_device(&headers, &key, &conn).await;
    let view =
        HUB.handovers.view(&user_key(&headers), &headers.device.uuid.to_string(), id, now_unix(), approval.is_ok());
    match view {
        Some(HandoverView::Requester(state)) => Ok(Json(serde_json::to_value(state).unwrap_or_default())),
        Some(HandoverView::Approver(request)) => Ok(Json(serde_json::to_value(request).unwrap_or_default())),
        None => Err(approval.err().unwrap_or_else(not_found)),
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
        let (created, replaced) = hub.create("u1", "viewer", "Pixel", None, 100).unwrap();
        assert!(!replaced);
        let (again, replaced) = hub.create("u1", "viewer", "Pixel", None, 101).unwrap();
        assert!(replaced, "the primary was told of the first one; the replacement is not pushed again");
        assert!(
            hub.view("u1", "viewer", &created.id, 101, false).is_none(),
            "a device's new request replaces its old one"
        );
        // Only an approver marks it seen.
        assert!(hub.view("u1", "someone", &again.id, 101, false).is_none());
        assert_eq!(hub.take_undelivered("u1", 101).len(), 1, "a viewer's look did not take it");
        hub.requeue("u1");
        assert!(hub.create("u1", "v2", "b", None, 101).is_some() && hub.create("u1", "v3", "c", None, 101).is_some());
        assert!(hub.create("u1", "v4", "d", None, 101).is_none(), "a few per account");
        assert!(hub.create("u2", "v4", "d", None, 101).is_some(), "per account, not for the server");
        assert_eq!(hub.peek_waiting("u1", 102).len(), 3);
        assert_eq!(hub.take_undelivered("u1", 102).len(), 3);
        assert!(hub.take_undelivered("u1", 102).is_empty());
        assert_eq!(hub.complete("u1", "viewer", 4, 103).as_deref(), Some(again.id.as_str()));
        assert_eq!(
            hub.view("u1", "viewer", &again.id, 103, false),
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
        let e1 =
            RosterEntry::genesis("acct", m("p", 1), encode_key(&[9; 32]), encode_key(&[8; 32]), "00".repeat(32), 1);
        let e2 = e1.successor(RosterReason::Enroll, m("p", 1), vec![m("v", 2)], 2).unwrap();
        let e3 = e2.successor(RosterReason::Handover, m("v", 2), vec![m("p", 1)], 3).unwrap();
        let chain = [e1, e2, e3];
        assert_eq!(primary_at(&chain, 3).map(|m| m.device_id.as_str()), Some("v"));
        assert_eq!(primary_at(&chain, 2), None, "p is no longer primary");
        assert_eq!(primary_at(&chain, 4), None);
        assert_eq!(primary_at(&chain[..2], 1).map(|m| m.device_id.as_str()), Some("p"), "p over its whole tenure");
    }
}
