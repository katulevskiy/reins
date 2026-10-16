//! Phone roles: which phone of an account decides (the **primary**) and which only watch (**viewers**).
//!
//! The roster is a hash-chained, append-only list of [`RosterEntry`]s, each signed by the account's **owner key**: the
//! first entry (epoch 1) by its own `owner_pub`, every later one by the `owner_pub` of the entry before it. Phones and
//! desktops verify the chain from the owner key they pinned and refuse an epoch lower than one they have seen; the
//! server checks each new entry the same way ([`check_successor`], [`verify_step`]) and appends it with a
//! compare-and-swap on the epoch.
//!
//! What may change from one entry to the next depends on its [`RosterReason`]:
//!
//! | Reason | Primary | Viewers | Owner and credential keys | Recovery wrap |
//! |---|---|---|---|---|
//! | `create` | the approval device (epoch 1 only) | none | new | new |
//! | `enroll` | unchanged | the previous ones and at least one more | unchanged | unchanged |
//! | `remove` | unchanged | some of the previous ones, at least one fewer | unchanged | new |
//! | `handover` | a previous viewer | the previous ones, the new primary out, the old primary may join | unchanged | new |
//! | `recovery` | another device | some of the previous ones; the old primary is gone | a new owner key, any credential key | new |
//! | `rewrap` | unchanged | unchanged | unchanged | new (the master password changed) |
//!
//! Every entry names the SHA-256 of the recovery wrap in force (`recovery_wrap_hash`), so the server can neither
//! swap the wrap nor serve an older one. Every change of who is primary or who was removed brings a new wrap (sealed
//! over its place in the chain), so a wrap from before a removal or a handover never fits the roster after it.
//!
//! Endpoints (server, `/reins/api`): `GET /roster` ([`Roster`]; any signed-in device of the account, an empty list
//! before the account has a roster, 404 on a server without phone roles), `POST /roster` ([`RosterAppend`]),
//! `POST /roster/recovery-wrap` ([`RecoveryWrapRequest`] → [`RecoveryWrap`]: only with the master password hash,
//! counted like takeover proofs), `PUT /member` ([`MemberRegistration`]), the handover requests (`POST /handovers`,
//! `GET /handovers/<id>`, `POST /handovers/<id>/response`) and `GET`/`DELETE /handover-bundle`.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    PROTOCOL_VERSION, ValidationError,
    canon::{Canon, contexts, decode_payload, hex32, parse_hex32, parse_sig, sha256, strict_ed25519_key},
    desktop::{decode_key, encode_key},
    device::valid_device_id,
    validate::invalid,
};

/// Viewers a roster may list.
pub const MAX_VIEWERS: usize = 8;
/// Longest member name (characters).
pub const MAX_MEMBER_NAME_CHARS: usize = 64;
/// Longest account id (the server's user id).
pub const MAX_ACCOUNT_CHARS: usize = 64;
/// Largest recovery wrap (base64url characters).
pub const MAX_RECOVERY_WRAP_CHARS: usize = 4 * 1024;
/// Largest sealed handover bundle (base64url characters).
pub const MAX_BUNDLE_CHARS: usize = 16 * 1024;

/// One phone of the roster.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    /// Its Vaultwarden device id (what the server authenticates, with its device key).
    pub device_id: String,
    /// Shown to the user ("Pixel 8"); at most [`MAX_MEMBER_NAME_CHARS`], no control characters.
    pub name: String,
    /// Its device signing key (Ed25519), base64url: signs its ops, answers and heartbeats.
    pub sign_pub: String,
    /// Its device box key (X25519, `reins.inbox-key`), base64url: what the view key and bundles are sealed to.
    pub box_pub: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RosterReason {
    Create,
    Enroll,
    Remove,
    Handover,
    Recovery,
    Rewrap,
}

impl RosterReason {
    /// Whether an entry of this reason brings a new recovery wrap (and must): `create`, `recovery`, `rewrap`, and every
    /// `remove` and `handover`.
    #[must_use]
    pub const fn brings_wrap(self) -> bool {
        !matches!(self, Self::Enroll)
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Enroll => "enroll",
            Self::Remove => "remove",
            Self::Handover => "handover",
            Self::Recovery => "recovery",
            Self::Rewrap => "rewrap",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Primary,
    Viewer,
}

/// One entry of the roster.
///
/// Signed message ([`crate::canon`], context `reins/roster-entry/v1`): `v` (u32), `account` (string), `epoch`
/// (u64), `prev_hash` (optional 32 bytes), `reason` (string), `primary` (member), `viewers` (list of members),
/// `owner_pub` (32 bytes), `credential_pub` (32 bytes), `recovery_wrap_hash` (32 bytes), `at` (i64). A member is `device_id` (string), `name`
/// (string), `sign_pub` (32 bytes), `box_pub` (32 bytes).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RosterEntry {
    pub v: u32,
    /// The server's id of the account (its user id), so that an entry of one account means nothing in another.
    pub account: String,
    /// 1 for `create`, then +1 per entry.
    pub epoch: u64,
    /// [`RosterEntry::hash`] of the entry before, hex; `None` at epoch 1.
    pub prev_hash: Option<String>,
    pub reason: RosterReason,
    pub primary: Member,
    pub viewers: Vec<Member>,
    /// The owner key (Ed25519) that signs the next entry, base64url.
    pub owner_pub: String,
    /// The credential key (X25519) that tokens and proposals' payloads are sealed to, base64url.
    pub credential_pub: String,
    /// [`recovery_wrap_hash`] of the recovery wrap in force, hex.
    pub recovery_wrap_hash: String,
    /// Unix seconds, as the signing phone saw them (shown, never compared).
    pub at: i64,
    /// Ed25519 signature by the owner key in force ([`RosterEntry::signer_pub`]), base64url.
    pub sig: String,
}

/// Why a roster entry was refused.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum RosterError {
    #[error(transparent)]
    Invalid(#[from] ValidationError),
    #[error("the entry is for another account")]
    WrongAccount,
    /// The epoch is not the one after the current one (a replay, a rollback or a concurrent change).
    #[error("expected epoch {expected}, got {got}")]
    WrongEpoch {
        expected: u64,
        got: u64,
    },
    #[error("prev_hash does not match the current entry")]
    BrokenChain,
    #[error("{0}")]
    Reason(&'static str),
    #[error("the owner key's signature does not verify")]
    BadSignature,
}

fn key32(field: &'static str, value: &str) -> Result<[u8; 32], ValidationError> {
    let raw = decode_key(value).ok_or_else(|| invalid(field, "must be 32 bytes, base64url without padding"))?;
    // One spelling per key: what decodes must encode back to the same text.
    if encode_key(&raw) != value {
        return Err(invalid(field, "is not in canonical base64url"));
    }
    Ok(raw)
}

/// Signed text cannot be cleaned up by whoever relays it, so it is refused unless it is already clean: no control
/// characters and nothing that hides or reorders text ([`crate::payments::is_format_char`]).
fn check_text(field: &'static str, value: &str, max_chars: usize) -> Result<(), ValidationError> {
    if value.is_empty() || value.chars().count() > max_chars {
        return Err(invalid(field, format!("must be 1 to {max_chars} characters")));
    }
    if value.chars().any(|c| c.is_control() || crate::payments::is_format_char(c)) {
        return Err(invalid(field, "must not contain control or invisible formatting characters"));
    }
    Ok(())
}

impl Member {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if !valid_device_id(&self.device_id) {
            return Err(invalid("device_id", "must be 1 to 36 characters of [A-Za-z0-9_-]"));
        }
        check_text("name", &self.name, MAX_MEMBER_NAME_CHARS)?;
        if !strict_ed25519_key(&key32("sign_pub", &self.sign_pub)?) {
            return Err(invalid("sign_pub", "is a small-order or non-canonical Ed25519 key"));
        }
        key32("box_pub", &self.box_pub)?;
        Ok(())
    }

    /// The signing key's 32 bytes (after [`Member::validate`]).
    pub fn sign_key(&self) -> Result<[u8; 32], ValidationError> {
        key32("sign_pub", &self.sign_pub)
    }

    /// The same phone with the same keys (the name may differ).
    #[must_use]
    pub fn same_keys(&self, other: &Self) -> bool {
        self.device_id == other.device_id && self.sign_pub == other.sign_pub && self.box_pub == other.box_pub
    }

    fn write(&self, c: &mut Canon) -> Result<(), ValidationError> {
        c.str(&self.device_id)
            .str(&self.name)
            .key(&key32("sign_pub", &self.sign_pub)?)
            .key(&key32("box_pub", &self.box_pub)?);
        Ok(())
    }
}

impl RosterEntry {
    /// Shape checks that need no other entry: versions, keys, sizes, a unique device per member.
    pub fn validate(&self) -> Result<(), ValidationError> {
        crate::check_version(self.v)?;
        check_text("account", &self.account, MAX_ACCOUNT_CHARS)?;
        if self.epoch == 0 {
            return Err(invalid("epoch", "starts at 1"));
        }
        match (&self.prev_hash, self.epoch) {
            (None, 1) => {}
            (Some(_), 1) => return Err(invalid("prev_hash", "must be null at epoch 1")),
            (None, _) => return Err(invalid("prev_hash", "is required after epoch 1")),
            (Some(h), _) => {
                parse_hex32(h).ok_or_else(|| invalid("prev_hash", "must be 64 lowercase hex digits"))?;
            }
        }
        if (self.epoch == 1) != (self.reason == RosterReason::Create) {
            return Err(invalid("reason", "`create` is epoch 1, and epoch 1 is `create`"));
        }
        if self.viewers.len() > MAX_VIEWERS {
            return Err(invalid("viewers", format!("at most {MAX_VIEWERS}")));
        }
        self.primary.validate()?;
        let mut devices = std::collections::BTreeSet::from([self.primary.device_id.as_str()]);
        let mut sign_keys = std::collections::BTreeSet::from([self.primary.sign_pub.as_str()]);
        for viewer in &self.viewers {
            viewer.validate()?;
            if !devices.insert(&viewer.device_id) || !sign_keys.insert(&viewer.sign_pub) {
                return Err(invalid("viewers", "every member is another device with its own signing key"));
            }
        }
        if !strict_ed25519_key(&key32("owner_pub", &self.owner_pub)?) {
            return Err(invalid("owner_pub", "is a small-order or non-canonical Ed25519 key"));
        }
        key32("credential_pub", &self.credential_pub)?;
        parse_hex32(&self.recovery_wrap_hash)
            .ok_or_else(|| invalid("recovery_wrap_hash", "must be 64 lowercase hex digits"))?;
        parse_sig(&self.sig).ok_or_else(|| invalid("sig", "must be 64 bytes, base64url without padding"))?;
        Ok(())
    }

    /// The bytes the owner key signs (see the type's docs).
    pub fn signing_bytes(&self) -> Result<Vec<u8>, ValidationError> {
        let mut c = Canon::new(contexts::ROSTER_ENTRY);
        c.u32(self.v).str(&self.account).u64(self.epoch);
        let prev = self.prev_hash.as_deref().map(|h| parse_hex32(h).ok_or_else(|| invalid("prev_hash", "not a hash")));
        c.opt_key(prev.transpose()?.as_ref());
        c.str(self.reason.as_str());
        self.primary.write(&mut c)?;
        c.count(self.viewers.len());
        for viewer in &self.viewers {
            viewer.write(&mut c)?;
        }
        let wrap = parse_hex32(&self.recovery_wrap_hash).ok_or_else(|| invalid("recovery_wrap_hash", "not a hash"))?;
        c.key(&key32("owner_pub", &self.owner_pub)?)
            .key(&key32("credential_pub", &self.credential_pub)?)
            .key(&wrap)
            .i64(self.at);
        Ok(c.finish())
    }

    /// What the next entry's `prev_hash` names: SHA-256 over context `reins/roster-hash/v1` and
    /// [`RosterEntry::signing_bytes`].
    pub fn hash(&self) -> Result<[u8; 32], ValidationError> {
        let mut c = Canon::new(contexts::ROSTER_HASH);
        c.bytes(&self.signing_bytes()?);
        Ok(sha256(&c.finish()))
    }

    /// [`RosterEntry::hash`] as it travels.
    pub fn hash_hex(&self) -> Result<String, ValidationError> {
        self.hash().map(|h| hex32(&h))
    }

    /// The owner key that must have signed this entry: its own at epoch 1, else the previous entry's.
    #[must_use]
    pub fn signer_pub<'a>(&'a self, prev: Option<&'a Self>) -> &'a str {
        prev.map_or(&self.owner_pub, |p| &p.owner_pub)
    }

    /// The primary, then the viewers.
    pub fn members(&self) -> impl Iterator<Item = &Member> {
        std::iter::once(&self.primary).chain(&self.viewers)
    }

    /// The member with `device_id`, and its role.
    #[must_use]
    pub fn role_of(&self, device_id: &str) -> Option<(Role, &Member)> {
        if self.primary.device_id == device_id {
            return Some((Role::Primary, &self.primary));
        }
        self.viewers.iter().find(|m| m.device_id == device_id).map(|m| (Role::Viewer, m))
    }

    #[must_use]
    pub fn member(&self, device_id: &str) -> Option<&Member> {
        self.role_of(device_id).map(|(_, m)| m)
    }
}

/// Whether `next` may follow `prev` (`None`: `next` is the first entry): the chain, the epoch and the rules of its
/// reason (module docs). Signatures are [`verify_step`]'s.
pub fn check_successor(prev: Option<&RosterEntry>, next: &RosterEntry) -> Result<(), RosterError> {
    next.validate()?;
    let Some(prev) = prev else {
        if next.epoch != 1 {
            return Err(RosterError::WrongEpoch {
                expected: 1,
                got: next.epoch,
            });
        }
        if !next.viewers.is_empty() {
            return Err(RosterError::Reason("a new roster has no viewers yet"));
        }
        return Ok(());
    };
    if next.account != prev.account {
        return Err(RosterError::WrongAccount);
    }
    let expected = prev.epoch.saturating_add(1);
    if next.epoch != expected {
        return Err(RosterError::WrongEpoch {
            expected,
            got: next.epoch,
        });
    }
    if next.prev_hash.as_deref() != Some(prev.hash_hex()?.as_str()) {
        return Err(RosterError::BrokenChain);
    }
    let recovery = next.reason == RosterReason::Recovery;
    if !recovery && (next.owner_pub != prev.owner_pub || next.credential_pub != prev.credential_pub) {
        return Err(RosterError::Reason("only a recovery changes the owner or credential key"));
    }
    if next.reason.brings_wrap() == (next.recovery_wrap_hash == prev.recovery_wrap_hash) {
        return Err(RosterError::Reason(
            "a recovery, rewrap, removal or handover brings a new recovery wrap, nothing else does",
        ));
    }
    let had = |m: &Member| prev.viewers.iter().any(|p| p.same_keys(m));
    match next.reason {
        RosterReason::Create => return Err(RosterError::Reason("`create` is only the first entry")),
        RosterReason::Enroll => {
            if !next.primary.same_keys(&prev.primary) {
                return Err(RosterError::Reason("an enrollment keeps the primary"));
            }
            let kept = prev.viewers.iter().all(|p| next.viewers.iter().any(|n| n.same_keys(p)));
            if !kept || next.viewers.len() <= prev.viewers.len() {
                return Err(RosterError::Reason("an enrollment keeps every viewer and adds at least one"));
            }
        }
        RosterReason::Remove => {
            if !next.primary.same_keys(&prev.primary) {
                return Err(RosterError::Reason("a removal keeps the primary"));
            }
            if !next.viewers.iter().all(had) || next.viewers.len() >= prev.viewers.len() {
                return Err(RosterError::Reason("a removal keeps only some of the viewers"));
            }
        }
        RosterReason::Handover => {
            if next.primary.device_id == prev.primary.device_id || !had(&next.primary) {
                return Err(RosterError::Reason("a handover makes one of the viewers primary"));
            }
            let others_kept = next.viewers.iter().all(|n| had(n) || n.same_keys(&prev.primary))
                && prev
                    .viewers
                    .iter()
                    .filter(|p| p.device_id != next.primary.device_id)
                    .all(|p| next.viewers.iter().any(|n| n.same_keys(p)));
            if !others_kept {
                return Err(RosterError::Reason("a handover only swaps the two phones' roles"));
            }
        }
        RosterReason::Recovery => {
            if next.primary.device_id == prev.primary.device_id {
                return Err(RosterError::Reason("a recovery makes another device primary"));
            }
            if next.owner_pub == prev.owner_pub {
                return Err(RosterError::Reason("a recovery introduces a new owner key"));
            }
            if next.member(&prev.primary.device_id).is_some() || !next.viewers.iter().all(had) {
                return Err(RosterError::Reason("a recovery removes the old primary and adds no viewer"));
            }
        }
        RosterReason::Rewrap => {
            let same_viewers = next.viewers.len() == prev.viewers.len() && next.viewers.iter().all(had);
            if !next.primary.same_keys(&prev.primary) || !same_viewers {
                return Err(RosterError::Reason("a rewrap changes only the recovery wrap"));
            }
        }
    }
    Ok(())
}

/// [`check_successor`], then the owner key's signature: `verify(public_key, message, signature)` is Ed25519
/// verification (strict, as `ring::signature::ED25519` or `ed25519_dalek::VerifyingKey::verify_strict`).
pub fn verify_step(
    prev: Option<&RosterEntry>,
    next: &RosterEntry,
    verify: impl FnOnce(&[u8; 32], &[u8], &[u8; 64]) -> bool,
) -> Result<(), RosterError> {
    check_successor(prev, next)?;
    let signer = key32("owner_pub", next.signer_pub(prev))?;
    let sig = parse_sig(&next.sig).ok_or_else(|| invalid("sig", "not a signature"))?;
    if verify(&signer, &next.signing_bytes()?, &sig) {
        Ok(())
    } else {
        Err(RosterError::BadSignature)
    }
}

/// Verifies a whole chain from its first entry (epoch 1). `pinned_owner`: the owner key of epoch 1 this device pinned,
/// if it has; then the chain must start with it.
pub fn verify_chain(
    entries: &[RosterEntry],
    pinned_owner: Option<&str>,
    verify: impl Fn(&[u8; 32], &[u8], &[u8; 64]) -> bool,
) -> Result<(), RosterError> {
    if let (Some(first), Some(pinned)) = (entries.first(), pinned_owner)
        && first.owner_pub != pinned
    {
        return Err(RosterError::BadSignature);
    }
    let mut prev = None;
    for entry in entries {
        verify_step(prev, entry, &verify)?;
        prev = Some(entry);
    }
    Ok(())
}

/// The phone that was primary just before the current primary took the role (whose late restrictive ops still
/// count, [`crate::oplog::authorize_op`]): the primary of the latest entry whose primary is another device or key.
/// `entries` ascend by epoch and end with the current entry.
#[must_use]
pub fn previous_primary(entries: &[RosterEntry]) -> Option<&Member> {
    let current = &entries.last()?.primary;
    entries.iter().rev().map(|e| &e.primary).find(|p| !p.same_keys(current))
}

/// The SHA-256 (hex) of a recovery wrap's decoded bytes, as entries name it; `None` unless it is base64url.
#[must_use]
pub fn recovery_wrap_hash(wrap: &str) -> Option<String> {
    decode_payload(wrap).map(|raw| hex32(&sha256(&raw)))
}

/// `GET /reins/api/roster`: the whole chain, oldest first (empty while the account has none). The recovery wrap is
/// not in it: it is fetched with a proof ([`RecoveryWrapRequest`]).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Roster {
    pub entries: Vec<RosterEntry>,
}

/// `POST /reins/api/roster`: the next entry, with what its reason needs.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RosterAppend {
    pub entry: RosterEntry,
    /// The owner seed and view key, sealed with authenticated encryption under a key derived from the master key
    /// (opaque to the server). Required with `create`, `recovery` and `rewrap`, and refused with the others; its
    /// [`recovery_wrap_hash`] must be the entry's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery_wrap: Option<String>,
    /// `handover` only, required: the owner seed and the account keys sealed to the new primary's box key, kept for it
    /// until it fetches and drops them (`GET`/`DELETE /reins/api/handover-bundle`). A sealed box does not say who
    /// sealed it, so the old primary signs the plaintext first with its signing key, over this entry's hash
    /// (sign-then-seal); the new primary refuses a bundle whose signature does not verify against the previous
    /// entry's primary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bundle: Option<String>,
    /// `create` and `recovery`, required: the master password hash of the recovery code or master password, checked
    /// and counted per device like the takeover proofs (a `create` so proves that its wrap is sealed under the
    /// account's master key).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub master_password_hash: Option<String>,
}

impl std::fmt::Debug for RosterAppend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RosterAppend")
            .field("entry", &self.entry)
            .field("recovery_wrap", &self.recovery_wrap.as_ref().map(String::len))
            .field("bundle", &self.bundle.as_ref().map(String::len))
            .field("master_password_hash", &self.master_password_hash.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

/// What `POST /reins/api/roster` answers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RosterAppended {
    pub epoch: u64,
    /// [`RosterEntry::hash`] of the new entry, hex.
    pub hash: String,
}

/// `POST /reins/api/roster/recovery-wrap`: the proof that the caller knows the recovery code or master password (its
/// master password hash), checked and counted like the takeover proofs.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryWrapRequest {
    pub master_password_hash: String,
}

impl std::fmt::Debug for RecoveryWrapRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecoveryWrapRequest").finish_non_exhaustive()
    }
}

/// What `POST /reins/api/roster/recovery-wrap` answers: the wrap the latest entry names.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryWrap {
    pub epoch: u64,
    pub recovery_wrap: String,
}

/// `PUT /reins/api/member`: a member's push token (viewers get `sync` pushes; the primary's token stays with
/// `PUT /reins/api/device`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberRegistration {
    #[serde(default)]
    pub fcm_token: Option<String>,
}

/// `POST /reins/api/handovers` (a viewer: "Decide on this phone").
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewHandover {
    pub v: u32,
    /// The viewer's signature over its request; the server keeps it with the request and the primary verifies it
    /// against the roster, so a server can neither make a request up nor point one at another phone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signed: Option<HandoverSignature>,
}

/// Most a handover request's `at` may differ from the primary's clock (seconds): a request lives 10 minutes.
pub const HANDOVER_REQUEST_SKEW_SECS: i64 = 15 * 60;

/// A viewer's signature over its handover request. The server names a request only once it exists, so the viewer
/// signs a fresh nonce of its own in the id's place.
///
/// Signed message (context `reins/handover-request/v1`): `v` (u32), `account` (string), `epoch` (u64), `device_id`
/// (string), `nonce` (32 bytes), `at` (i64).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandoverSignature {
    /// The roster epoch the viewer signed at: the primary refuses a request from another epoch.
    pub epoch: u64,
    /// 32 random bytes, base64url.
    pub nonce: String,
    /// Unix seconds, the viewer's clock.
    pub at: i64,
    /// Ed25519 by the viewer's `sign_pub` at `epoch`, base64url.
    pub sig: String,
}

impl HandoverSignature {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.epoch == 0 {
            return Err(invalid("epoch", "starts at 1"));
        }
        crate::enroll::parse_nonce(&self.nonce).ok_or_else(|| invalid("nonce", "must be 32 bytes, base64url"))?;
        parse_sig(&self.sig).ok_or_else(|| invalid("sig", "must be 64 bytes, base64url without padding"))?;
        Ok(())
    }
}

/// The bytes a viewer signs to ask for the primary role (see [`HandoverSignature`]); `None` for a nonce that is not
/// 32 bytes.
#[must_use]
pub fn handover_request_message(account: &str, epoch: u64, device_id: &str, nonce: &str, at: i64) -> Option<Vec<u8>> {
    let nonce = crate::enroll::parse_nonce(nonce)?;
    let mut c = Canon::new(contexts::HANDOVER_REQUEST);
    c.u32(PROTOCOL_VERSION).str(account).u64(epoch).str(device_id).key(&nonce).i64(at);
    Some(c.finish())
}

/// What `POST /reins/api/handovers` answers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandoverCreated {
    pub id: String,
    pub expires_at: i64,
}

/// A viewer's handover request, as the primary sees it (in `GET /pending` and `GET /handovers/<id>`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandoverRequest {
    pub v: u32,
    pub id: String,
    /// The viewer asking.
    pub device_id: String,
    /// Its name in the roster (as the server read it; the primary shows the name its verified roster gives).
    pub device_name: String,
    pub created_at: i64,
    /// The viewer's signature ([`NewHandover::signed`]), as the server kept it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signed: Option<HandoverSignature>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HandoverStatus {
    Waiting,
    /// A `handover` entry made the asking viewer primary.
    Done,
    Denied,
    /// Unknown or past its time.
    Expired,
}

/// What the asking viewer polls (`GET /reins/api/handovers/<id>`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandoverState {
    pub status: HandoverStatus,
    /// The epoch of the `handover` entry, with `Done`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub epoch: Option<u64>,
}

/// `POST /reins/api/handovers/<id>/response` (the primary): only a denial is answered this way; a yes is the
/// `handover` roster entry itself.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandoverAnswer {
    pub v: u32,
    pub approve: bool,
}

/// `GET /reins/api/handover-bundle` (the primary a handover named, until it deletes it).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandoverBundle {
    pub epoch: u64,
    pub sealed: String,
}

impl RosterEntry {
    /// The first entry of an account's roster, unsigned (`sig` empty): the caller signs
    /// [`RosterEntry::signing_bytes`] with the owner key of `owner_pub`.
    #[must_use]
    pub fn genesis(
        account: &str,
        primary: Member,
        owner_pub: String,
        credential_pub: String,
        recovery_wrap_hash: String,
        at: i64,
    ) -> Self {
        Self {
            v: PROTOCOL_VERSION,
            account: account.to_owned(),
            epoch: 1,
            prev_hash: None,
            reason: RosterReason::Create,
            primary,
            viewers: Vec::new(),
            owner_pub,
            credential_pub,
            recovery_wrap_hash,
            at,
            sig: String::new(),
        }
    }

    /// The entry after this one, unsigned (`sig` empty), with the same owner and credential keys and recovery wrap (a
    /// recovery or rewrap sets new ones before signing). The caller signs it with this entry's owner key.
    pub fn successor(
        &self,
        reason: RosterReason,
        primary: Member,
        viewers: Vec<Member>,
        at: i64,
    ) -> Result<Self, ValidationError> {
        Ok(Self {
            v: PROTOCOL_VERSION,
            account: self.account.clone(),
            epoch: self.epoch.saturating_add(1),
            prev_hash: Some(self.hash_hex()?),
            reason,
            primary,
            viewers,
            owner_pub: self.owner_pub.clone(),
            credential_pub: self.credential_pub.clone(),
            recovery_wrap_hash: self.recovery_wrap_hash.clone(),
            at,
            sig: String::new(),
        })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A fake signature scheme for the tests: the "signature" is SHA-256(key || message) twice. Proto has no Ed25519;
    /// what is tested here is which key must sign which bytes.
    pub(crate) fn fake_sign(key: &[u8; 32], msg: &[u8]) -> [u8; 64] {
        let mut input = key.to_vec();
        input.extend_from_slice(msg);
        let h = sha256(&input);
        let mut sig = [0; 64];
        sig[..32].copy_from_slice(&h);
        sig[32..].copy_from_slice(&h);
        sig
    }

    pub(crate) fn fake_verify(key: &[u8; 32], msg: &[u8], sig: &[u8; 64]) -> bool {
        fake_sign(key, msg) == *sig
    }

    pub(crate) fn key(n: u8) -> String {
        encode_key(&[n; 32])
    }

    pub(crate) fn member(id: &str, n: u8) -> Member {
        Member {
            device_id: id.to_owned(),
            name: format!("Phone {id}"),
            sign_pub: key(n),
            box_pub: key(n.wrapping_add(100)),
        }
    }

    /// Owner keys are `key(owner)`; "signing" with owner seed `owner` means `fake_sign(&[owner; 32], ..)`.
    pub(crate) fn signed(mut entry: RosterEntry, signer: u8) -> RosterEntry {
        let sig = fake_sign(&[signer; 32], &entry.signing_bytes().unwrap());
        entry.sig = crate::canon::encode_sig(&sig);
        entry
    }

    pub(crate) fn genesis() -> RosterEntry {
        signed(RosterEntry::genesis("acct", member("p", 1), key(50), key(60), hex32(&[70; 32]), 10), 50)
    }

    pub(crate) fn next(
        prev: &RosterEntry,
        reason: RosterReason,
        primary: Member,
        viewers: Vec<Member>,
        owner: u8,
    ) -> RosterEntry {
        let mut entry = prev.successor(reason, primary, viewers, 20).unwrap();
        entry.owner_pub = key(owner);
        if reason == RosterReason::Recovery {
            entry.credential_pub = key(61);
        }
        if reason.brings_wrap() {
            entry.recovery_wrap_hash = hex32(&[71 + u8::try_from(prev.epoch).unwrap(); 32]);
        }
        let signer = decode_key(&prev.owner_pub).unwrap()[0];
        signed(entry, signer)
    }

    #[test]
    fn a_chain_of_every_reason_verifies() {
        let e1 = genesis();
        let e2 = next(&e1, RosterReason::Enroll, member("p", 1), vec![member("v", 2)], 50);
        let e3 = next(&e2, RosterReason::Enroll, member("p", 1), vec![member("v", 2), member("w", 3)], 50);
        let e4 = next(&e3, RosterReason::Remove, member("p", 1), vec![member("v", 2)], 50);
        let e5 = next(&e4, RosterReason::Handover, member("v", 2), vec![member("p", 1)], 50);
        let e6 = next(&e5, RosterReason::Recovery, member("x", 4), vec![], 51);
        let e7 = next(&e6, RosterReason::Enroll, member("x", 4), vec![member("y", 5)], 51);
        let e8 = next(&e7, RosterReason::Rewrap, member("x", 4), vec![member("y", 5)], 51);
        let chain = [e1, e2, e3, e4, e5, e6, e7, e8];
        verify_chain(&chain, Some(&key(50)), fake_verify).unwrap();
        assert_eq!(verify_chain(&chain, Some(&key(9)), fake_verify), Err(RosterError::BadSignature));
        // After the recovery, the old owner key signs nothing.
        let mut forged = chain[7].successor(RosterReason::Remove, member("x", 4), vec![], 30).unwrap();
        forged.recovery_wrap_hash = hex32(&[99; 32]);
        let forged = signed(forged, 50);
        assert_eq!(verify_step(Some(&chain[7]), &forged, fake_verify), Err(RosterError::BadSignature));
        assert_eq!(previous_primary(&chain).map(|m| m.device_id.as_str()), Some("v"));
        assert_eq!(previous_primary(&chain[..4]), None);
        assert_eq!(previous_primary(&chain[..5]).map(|m| m.device_id.as_str()), Some("p"));
    }

    #[test]
    fn a_rollback_a_replay_or_a_fork_is_refused() {
        let e1 = genesis();
        let e2 = next(&e1, RosterReason::Enroll, member("p", 1), vec![member("v", 2)], 50);
        assert_eq!(
            check_successor(Some(&e2), &e2),
            Err(RosterError::WrongEpoch {
                expected: 3,
                got: 2
            })
        );
        assert!(matches!(
            check_successor(Some(&e2), &e1),
            Err(RosterError::Invalid(_) | RosterError::WrongEpoch { .. })
        ));
        let mut fork = e2.clone();
        fork.prev_hash = Some(hex32(&[0; 32]));
        assert_eq!(check_successor(Some(&e1), &fork), Err(RosterError::BrokenChain));
        let mut other = next(&e1, RosterReason::Enroll, member("p", 1), vec![member("v", 2)], 50);
        other.account = "another".to_owned();
        assert_eq!(check_successor(Some(&e1), &other), Err(RosterError::WrongAccount));
        assert_eq!(
            check_successor(None, &e2),
            Err(RosterError::WrongEpoch {
                expected: 1,
                got: 2
            })
        );
    }

    #[test]
    fn each_reason_changes_only_what_it_may() {
        let e1 = genesis();
        let e2 = next(&e1, RosterReason::Enroll, member("p", 1), vec![member("v", 2)], 50);
        let refused = |entry: RosterEntry, prev: &RosterEntry| {
            assert!(matches!(check_successor(Some(prev), &entry), Err(RosterError::Reason(_))), "{entry:?}");
        };
        refused(next(&e1, RosterReason::Enroll, member("q", 9), vec![member("v", 2)], 50), &e1);
        refused(next(&e1, RosterReason::Enroll, member("p", 1), vec![], 50), &e1);
        refused(next(&e1, RosterReason::Enroll, member("p", 1), vec![member("v", 2)], 52), &e1);
        refused(next(&e2, RosterReason::Remove, member("p", 1), vec![member("v", 2)], 50), &e2);
        refused(next(&e2, RosterReason::Remove, member("p", 1), vec![member("z", 7)], 50), &e2);
        refused(next(&e2, RosterReason::Handover, member("z", 7), vec![member("p", 1)], 50), &e2);
        refused(next(&e2, RosterReason::Handover, member("v", 2), vec![member("p", 1), member("z", 7)], 50), &e2);
        // The viewer's keys must be the enrolled ones.
        refused(next(&e2, RosterReason::Handover, member("v", 3), vec![member("p", 1)], 50), &e2);
        refused(next(&e2, RosterReason::Recovery, member("x", 4), vec![member("v", 2)], 50), &e2);
        refused(next(&e2, RosterReason::Recovery, member("x", 4), vec![member("p", 1)], 51), &e2);
        refused(next(&e2, RosterReason::Recovery, member("x", 4), vec![member("z", 7)], 51), &e2);
        refused(next(&e2, RosterReason::Recovery, member("p", 1), vec![], 51), &e2);
        // The recovery wrap: a recovery or rewrap brings a new one, nothing else changes it.
        let mut keeps_wrap = next(&e2, RosterReason::Recovery, member("x", 4), vec![member("v", 2)], 51);
        keeps_wrap.recovery_wrap_hash.clone_from(&e2.recovery_wrap_hash);
        refused(signed(keeps_wrap, 50), &e2);
        let mut sneaks_wrap = next(&e2, RosterReason::Enroll, member("p", 1), vec![member("v", 2), member("w", 3)], 50);
        sneaks_wrap.recovery_wrap_hash = hex32(&[9; 32]);
        refused(signed(sneaks_wrap, 50), &e2);
        refused(next(&e2, RosterReason::Rewrap, member("p", 1), vec![], 50), &e2);
        // A removal or a handover that keeps the old wrap: refused (the old wrap would still fit after it).
        for (reason, primary, viewers) in [
            (RosterReason::Remove, member("p", 1), vec![]),
            (RosterReason::Handover, member("v", 2), vec![member("p", 1)]),
        ] {
            let mut keeps = next(&e2, reason, primary, viewers, 50);
            keeps.recovery_wrap_hash.clone_from(&e2.recovery_wrap_hash);
            refused(signed(keeps, 50), &e2);
        }
        check_successor(Some(&e2), &next(&e2, RosterReason::Rewrap, member("p", 1), vec![member("v", 2)], 50)).unwrap();
        assert_eq!(recovery_wrap_hash("d3JhcA"), Some(hex32(&sha256(b"wrap"))));
        assert!(
            check_successor(Some(&e2), &next(&e2, RosterReason::Create, member("p", 1), vec![member("v", 2)], 50))
                .is_err()
        );
        // A recovery from a viewer that keeps the other viewers.
        let e3 = next(&e2, RosterReason::Enroll, member("p", 1), vec![member("v", 2), member("w", 3)], 50);
        check_successor(Some(&e3), &next(&e3, RosterReason::Recovery, member("v", 2), vec![member("w", 3)], 51))
            .unwrap();
    }

    #[test]
    fn shapes_are_checked() {
        let mut e = genesis();
        e.viewers.push(member("p", 9));
        assert!(e.validate().is_err(), "a device twice");
        let mut e = genesis();
        e.primary.name = "bad\nname".to_owned();
        assert!(e.validate().is_err());
        e.primary.name = "Pixel\u{202E}9".to_owned();
        assert!(e.validate().is_err(), "a bidi override in a signed name");
        let mut e = genesis();
        e.primary.sign_pub = "short".to_owned();
        assert!(e.validate().is_err());
        e.primary.sign_pub = encode_key(&[0; 32]);
        assert!(e.validate().is_err(), "a small-order signing key");
        let mut e = genesis();
        let mut identity = [0u8; 32];
        identity[0] = 1;
        e.owner_pub = encode_key(&identity);
        assert!(e.validate().is_err(), "a small-order owner key");
        let mut e = genesis();
        e.prev_hash = Some(hex32(&[1; 32]));
        assert!(e.validate().is_err());
        let mut e = genesis();
        e.viewers = (0..=MAX_VIEWERS).map(|i| member(&format!("v{i}"), u8::try_from(i).unwrap() + 2)).collect();
        assert!(e.validate().is_err());
        let mut e = genesis();
        e.v = 2;
        assert!(e.validate().is_err());
        assert!(check_successor(None, &genesis()).is_ok());
        let mut with_viewer = genesis();
        with_viewer.viewers.push(member("v", 2));
        assert!(check_successor(None, &with_viewer).is_err());
    }

    #[test]
    fn signing_bytes_cover_every_field_and_ignore_the_signature() {
        let e = genesis();
        let base = e.signing_bytes().unwrap();
        let mut changed = e.clone();
        changed.sig = crate::canon::encode_sig(&[9; 64]);
        assert_eq!(changed.signing_bytes().unwrap(), base);
        let edits: [fn(&mut RosterEntry); 11] = [
            |e| e.account.push('x'),
            |e| e.epoch += 1,
            |e| e.reason = RosterReason::Enroll,
            |e| e.primary.name.push('x'),
            |e| e.primary.box_pub = key(77),
            |e| e.viewers.push(member("v", 2)),
            |e| e.owner_pub = key(78),
            |e| e.credential_pub = key(79),
            |e| e.recovery_wrap_hash = hex32(&[2; 32]),
            |e| e.at += 1,
            |e| e.prev_hash = Some(hex32(&[1; 32])),
        ];
        for edit in edits {
            let mut e2 = e.clone();
            edit(&mut e2);
            assert_ne!(e2.signing_bytes().unwrap(), base);
        }
        // Golden: the encoding is stable across releases (a change here breaks every pinned roster).
        assert_eq!(hex32(&e.hash().unwrap()), GENESIS_HASH);
    }

    const GENESIS_HASH: &str = "b6cb47ee595ad1ee32338e33f46053dce249e14f42697e9fa7c1688bc497a198";

    #[test]
    fn real_ed25519_signatures_verify_through_the_closure() {
        use ring::signature::{ED25519, Ed25519KeyPair, KeyPair, UnparsedPublicKey};
        let owner = Ed25519KeyPair::from_seed_unchecked(&[42; 32]).unwrap();
        let owner_pub = encode_key(&owner.public_key().as_ref().try_into().unwrap());
        let mut entry = RosterEntry::genesis("acct", member("p", 1), owner_pub, key(60), hex32(&[70; 32]), 10);
        entry.sig = crate::canon::encode_sig(&owner.sign(&entry.signing_bytes().unwrap()).as_ref().try_into().unwrap());
        let ring_verify =
            |k: &[u8; 32], m: &[u8], s: &[u8; 64]| UnparsedPublicKey::new(&ED25519, k).verify(m, s).is_ok();
        verify_step(None, &entry, ring_verify).unwrap();
        let mut tampered = entry.clone();
        tampered.primary.name = "Evil".to_owned();
        assert_eq!(verify_step(None, &tampered, ring_verify), Err(RosterError::BadSignature));
        // The enrollment after it, signed by the same owner key.
        let mut e2 = entry.successor(RosterReason::Enroll, member("p", 1), vec![member("v", 2)], 11).unwrap();
        e2.sig = crate::canon::encode_sig(&owner.sign(&e2.signing_bytes().unwrap()).as_ref().try_into().unwrap());
        verify_chain(&[entry, e2], None, ring_verify).unwrap();
    }

    #[test]
    fn a_handover_request_signs_account_epoch_device_nonce_and_time() {
        let nonce = crate::canon::encode_payload(&[7; 32]);
        let base = handover_request_message("acct", 3, "v", &nonce, 100).unwrap();
        let other_nonce = crate::canon::encode_payload(&[8; 32]);
        for other in [
            handover_request_message("acct2", 3, "v", &nonce, 100),
            handover_request_message("acct", 4, "v", &nonce, 100),
            handover_request_message("acct", 3, "w", &nonce, 100),
            handover_request_message("acct", 3, "v", &other_nonce, 100),
            handover_request_message("acct", 3, "v", &nonce, 101),
        ] {
            assert_ne!(other.unwrap(), base);
        }
        assert_eq!(handover_request_message("acct", 3, "v", "short", 100), None);
        let signed = HandoverSignature {
            epoch: 3,
            nonce,
            at: 100,
            sig: crate::canon::encode_sig(&[1; 64]),
        };
        signed.validate().unwrap();
        let no_epoch = HandoverSignature {
            epoch: 0,
            ..signed.clone()
        };
        assert!(no_epoch.validate().is_err());
        let short = HandoverSignature {
            nonce: "x".to_owned(),
            ..signed
        };
        assert!(short.validate().is_err());
        // Requests without a signature still parse: the field is optional on the wire.
        let old: NewHandover = serde_json::from_value(serde_json::json!({"v": 1})).unwrap();
        assert_eq!(old.signed, None);
        let unsigned = NewHandover {
            v: 1,
            signed: None,
        };
        assert_eq!(serde_json::to_value(unsigned).unwrap(), serde_json::json!({"v": 1}));
    }

    #[test]
    fn wire_format() {
        let e = genesis();
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["reason"], "create");
        assert_eq!(v["prev_hash"], serde_json::Value::Null);
        assert_eq!(v["primary"]["device_id"], "p");
        assert_eq!(serde_json::from_value::<RosterEntry>(v).unwrap(), e);
        let append = RosterAppend {
            entry: e,
            recovery_wrap: Some("wrap".to_owned()),
            bundle: None,
            master_password_hash: Some("SECRET".to_owned()),
        };
        assert!(!format!("{append:?}").contains("SECRET"));
        let v = serde_json::to_value(&append).unwrap();
        assert!(v.get("bundle").is_none());
        assert_eq!(
            serde_json::to_value(HandoverState {
                status: HandoverStatus::Waiting,
                epoch: None
            })
            .unwrap(),
            serde_json::json!({"status": "waiting"})
        );
        assert_eq!(serde_json::to_value(Roster::default()).unwrap(), serde_json::json!({"entries": []}));
    }
}
