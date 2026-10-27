//! Circles: shared control between Reins accounts in Personal mode (`docs/design/decisions-v2/04-shared-control.md`,
//! 08 R8).
//!
//! Two accounts make an **arrangement**: the **owner** (whose requests are reviewed) and a **helper** (a delegate, a
//! co-signer, a guardian or a watcher). Each side is known by its roster's **owner key** ([`crate::roster`]), pinned
//! once by a six-digit safety code; everything later is verified against it:
//!
//! 1. **Invitation** (commit-then-reveal, as [`crate::enroll`]):
//!    - the owner posts a commitment ([`invite_commitment`]) to its account, owner key, roster epoch and a fresh
//!      nonce, and gets a one-time link;
//!    - the helper opens it and answers with a challenge ([`InviteChallenge`]): its owner key, epoch and nonce;
//!    - the owner reveals its nonce ([`InviteReveal`]) with the proposed terms sealed to the helper's approval phone;
//!    - both phones show [`InviteTranscript::code`]. A server that swaps a key must fix it before the other side's
//!      nonce is known, so its digits are random.
//! 2. **Signatures:** the helper signs the arrangement with its owner key ([`SignedArrangement::helper_sig`]), then the
//!    owner signs it and the helper's signature ([`SignedArrangement::owner_sig`]). An arrangement counts only with
//!    both, so a server can neither invent one nor widen one.
//! 3. **Review packets** ([`SignedPacket`]) are signed by the owner's approval phone and sealed to the helper's approval
//!    phone; **answers** ([`SignedCircleAnswer`]) are signed by the helper's approval phone and sealed to the owner's.
//!    The receiving phone checks the signer is the other side's *current* primary, through the roster chain from the
//!    anchor it pinned ([`peer_keys`]). A new owner key (a recovery) suspends the arrangement.
//!
//! Signed bodies travel as the exact JSON text that was signed (`body`); the signature covers its SHA-256 under a
//! context of its own ([`crate::canon::contexts`]), so no re-serialization is ever verified. This module does no
//! cryptography beyond SHA-256: signature checks take a `verify` function (strict Ed25519).

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    PROTOCOL_VERSION, ValidationError,
    canon::{
        Canon, contexts, decode_payload, encode_payload, hex32, parse_hex32, parse_sig, sha256, strict_ed25519_key,
    },
    desktop::decode_key,
    roster::{RosterEntry, verify_step},
    validate::invalid,
};

/// How long an invitation link stays open.
pub const INVITE_TTL_SECS: i64 = 7 * 86_400;
/// The largest sealed blob (base64url characters): a packet, an answer, a copy of an arrangement.
pub const MAX_SEALED_CHARS: usize = 96 * 1024;
/// The longest label (a person, a scope line).
pub const MAX_LABEL_CHARS: usize = 120;
/// The longest note a helper writes with an answer.
pub const MAX_NOTE_CHARS: usize = 500;
/// How far an answer's time may sit outside its packet's window.
pub const ANSWER_SKEW_SECS: i64 = 300;
/// The longest peer roster chain a phone reads.
pub const MAX_PEER_CHAIN: usize = 256;
/// The most items any list of a scope holds.
pub const MAX_SCOPE_ITEMS: usize = 32;
/// Instant windows (seconds), as an Instant review's (03 §1).
pub const INSTANT_MIN_SECS: u64 = 30;
pub const INSTANT_MAX_SECS: u64 = 600;
pub const INSTANT_DEFAULT_SECS: u64 = 120;
/// Deferred windows (seconds).
pub const DEFERRED_MIN_SECS: u64 = 60;
pub const DEFERRED_MAX_SECS: u64 = 604_800;
pub const DEFERRED_DEFAULT_SECS: u64 = 14_400;

/// The effect kinds a scope may name: the snake_case names of `reins_policy::effects::EffectKind` (the policy crate
/// asserts they match).
pub const SCOPE_EFFECTS: &[&str] = &[
    "file_read",
    "file_write",
    "file_delete",
    "git_push",
    "git_ref_delete",
    "message",
    "publish",
    "permission",
    "money",
    "secret_out",
    "secret_use",
    "network",
    "infra",
    "exec",
    "data_read",
    "data_write",
    "unknown",
];
/// The zones a scope may name (`reins_policy::context::Zone`).
pub const SCOPE_ZONES: &[&str] = &["mine", "friends", "work", "outside"];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CircleKind {
    /// The delegate **or** the owner decides.
    Delegate,
    /// The owner **and** the co-signer decide.
    Cosign,
    /// The guardian must approve (cooperative in Personal mode, 08 R7).
    Guardian,
    /// Informed only: no buttons.
    Watcher,
}

impl CircleKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Delegate => "delegate",
            Self::Cosign => "cosign",
            Self::Guardian => "guardian",
            Self::Watcher => "watcher",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        [Self::Delegate, Self::Cosign, Self::Guardian, Self::Watcher].into_iter().find(|k| k.as_str() == s)
    }
}

/// What happens when a helper's window runs out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnExpiry {
    /// Not approved (the default).
    #[default]
    Deny,
    /// The owner decides alone (a Delegate only).
    Owner,
}

/// What a packet shows the helper (04 §5).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Redaction {
    /// The card as the owner sees it, with the detail (an email's subject and recipients, never its body).
    Full,
    /// The effects and the sentence, no detail.
    Summary,
}

/// Whether the helper is expected at hand (03 §1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Class {
    Instant,
    Deferred,
}

/// What an arrangement covers, as effect filters (04 §2). An empty list matches anything; all empty is everything.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Scope {
    /// Names from [`SCOPE_EFFECTS`].
    pub effects: Vec<String>,
    /// Names from [`SCOPE_ZONES`].
    pub zones: Vec<String>,
    /// 0..=3: the least level covered.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_level: Option<u8>,
    /// 0..=3: the highest level covered (a delegate for routine requests only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_level: Option<u8>,
    /// Connection ids of the owner (the agent).
    pub connections: Vec<String>,
    /// Accounts the requests act on (e.g. `founder@startup.com`).
    pub accounts: Vec<String>,
}

/// An arrangement's terms (04 §1–2).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Terms {
    pub kind: CircleKind,
    pub scope: Scope,
    pub class: Class,
    pub window_secs: u64,
    pub on_expiry: OnExpiry,
    pub redaction: Redaction,
    /// The scope in words, shown to both sides ("Money", "Everything ChatGPT does").
    pub scope_line: String,
}

/// One side, as pinned by the safety code.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Party {
    /// The server's id of the account.
    pub account: String,
    /// Shown only ("dana@example.com"); the safety code, not the label, is what is trusted.
    pub label: String,
    /// The roster's owner key (Ed25519, base64url) at `epoch`.
    pub owner_pub: String,
    /// The roster epoch the code pinned.
    pub epoch: u64,
    /// That roster entry's hash (hex): the anchor later chains are verified from.
    pub entry_hash: String,
}

/// An arrangement between two accounts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Arrangement {
    pub v: u32,
    /// 32 lowercase hex characters, chosen by the owner's phone.
    pub id: String,
    pub revision: u64,
    pub owner: Party,
    pub helper: Party,
    pub terms: Terms,
    pub created_at: i64,
}

/// An arrangement with both signatures. `body` is the arrangement's JSON exactly as the helper signed it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedArrangement {
    pub body: String,
    /// The helper's owner key over [`helper_message`].
    pub helper_sig: String,
    /// The owner's owner key over [`owner_message`]; empty until the owner confirms.
    #[serde(default)]
    pub owner_sig: String,
}

/// Why something of a Circle was refused.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum CircleError {
    #[error(transparent)]
    Invalid(#[from] ValidationError),
    #[error("a signature does not verify")]
    BadSignature,
    #[error("the signer is not the other side's current approval phone")]
    NotCurrentSigner,
    #[error("{0} does not match")]
    Mismatch(&'static str),
    #[error("the answer's time is outside its window")]
    Stale,
    #[error("the roster does not chain from the pinned entry")]
    BrokenChain,
    #[error("this is not newer than what was accepted before")]
    Replayed,
}

// ---- checks -------------------------------------------------------------------------------------------------------

/// Text a person reads and someone signs: 1..=`max` characters, no control or invisible formatting characters.
fn clean(field: &'static str, value: &str, max: usize) -> Result<(), ValidationError> {
    if value.is_empty() || value.chars().count() > max {
        return Err(invalid(field, format!("must be 1 to {max} characters")));
    }
    if value.chars().any(|c| c.is_control() || crate::payments::is_format_char(c)) {
        return Err(invalid(field, "must not contain control or invisible formatting characters"));
    }
    Ok(())
}

fn ed_key(field: &'static str, value: &str) -> Result<[u8; 32], ValidationError> {
    let raw = decode_key(value).ok_or_else(|| invalid(field, "must be 32 bytes, base64url without padding"))?;
    if crate::desktop::encode_key(&raw) != value || !strict_ed25519_key(&raw) {
        return Err(invalid(field, "is not a canonical Ed25519 key"));
    }
    Ok(raw)
}

fn x_key(field: &'static str, value: &str) -> Result<[u8; 32], ValidationError> {
    let raw = decode_key(value).ok_or_else(|| invalid(field, "must be 32 bytes, base64url without padding"))?;
    if crate::desktop::encode_key(&raw) != value {
        return Err(invalid(field, "is not canonical base64url"));
    }
    Ok(raw)
}

/// An id a phone draws: 32 lowercase hex characters.
#[must_use]
pub fn valid_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// A 16-byte nonce as it travels (base64url).
#[must_use]
pub fn valid_nonce16(nonce: &str) -> bool {
    decode_payload(nonce).is_some_and(|n| n.len() == 16)
}

fn digest(field: &'static str, value: &str) -> Result<[u8; 32], ValidationError> {
    parse_hex32(value).ok_or_else(|| invalid(field, "must be 64 lowercase hex digits"))
}

fn list(field: &'static str, items: &[String], allowed: Option<&[&str]>) -> Result<(), ValidationError> {
    if items.len() > MAX_SCOPE_ITEMS {
        return Err(invalid(field, format!("at most {MAX_SCOPE_ITEMS}")));
    }
    for item in items {
        clean(field, item, MAX_LABEL_CHARS)?;
        if let Some(allowed) = allowed
            && !allowed.contains(&item.as_str())
        {
            return Err(invalid(field, format!("unknown name {item:?}")));
        }
    }
    Ok(())
}

impl Scope {
    pub fn validate(&self) -> Result<(), ValidationError> {
        list("scope.effects", &self.effects, Some(SCOPE_EFFECTS))?;
        list("scope.zones", &self.zones, Some(SCOPE_ZONES))?;
        list("scope.connections", &self.connections, None)?;
        list("scope.accounts", &self.accounts, None)?;
        if self.min_level.is_some_and(|l| l > 3) || self.max_level.is_some_and(|l| l > 3) {
            return Err(invalid("scope.min_level", "levels are 0 to 3"));
        }
        if let (Some(lo), Some(hi)) = (self.min_level, self.max_level)
            && lo > hi
        {
            return Err(invalid("scope.max_level", "must not be below min_level"));
        }
        Ok(())
    }

    /// Everything the owner's agents do.
    #[must_use]
    pub fn is_everything(&self) -> bool {
        self.effects.is_empty()
            && self.zones.is_empty()
            && self.min_level.is_none()
            && self.max_level.is_none()
            && self.connections.is_empty()
            && self.accounts.is_empty()
    }
}

/// The window bounds of a class.
#[must_use]
pub const fn window_bounds(class: Class) -> (u64, u64) {
    match class {
        Class::Instant => (INSTANT_MIN_SECS, INSTANT_MAX_SECS),
        Class::Deferred => (DEFERRED_MIN_SECS, DEFERRED_MAX_SECS),
    }
}

/// A window within its class's bounds (`None`: the class's default).
#[must_use]
pub fn clamp_window(class: Class, secs: Option<u64>) -> u64 {
    let (lo, hi) = window_bounds(class);
    let default = match class {
        Class::Instant => INSTANT_DEFAULT_SECS,
        Class::Deferred => DEFERRED_DEFAULT_SECS,
    };
    secs.unwrap_or(default).clamp(lo, hi)
}

impl Terms {
    /// 04's defaults for `kind`: Deferred, its default window, deny on expiry; summary for a guardian or watcher.
    #[must_use]
    pub fn default_for(kind: CircleKind) -> Self {
        Self {
            kind,
            scope: Scope::default(),
            class: Class::Deferred,
            window_secs: DEFERRED_DEFAULT_SECS,
            on_expiry: OnExpiry::Deny,
            redaction: match kind {
                CircleKind::Delegate | CircleKind::Cosign => Redaction::Full,
                CircleKind::Guardian | CircleKind::Watcher => Redaction::Summary,
            },
            scope_line: "Everything".to_owned(),
        }
    }

    pub fn validate(&self) -> Result<(), ValidationError> {
        self.scope.validate()?;
        clean("terms.scope_line", &self.scope_line, MAX_LABEL_CHARS)?;
        let (lo, hi) = window_bounds(self.class);
        if !(lo..=hi).contains(&self.window_secs) {
            return Err(invalid("terms.window_secs", format!("must be {lo} to {hi} for this class")));
        }
        if self.on_expiry == OnExpiry::Owner && self.kind != CircleKind::Delegate {
            return Err(invalid("terms.on_expiry", "only a delegate's window may fall back to the owner"));
        }
        if self.kind == CircleKind::Watcher && self.redaction != Redaction::Summary {
            return Err(invalid("terms.redaction", "a watcher sees summaries only"));
        }
        Ok(())
    }
}

impl Party {
    pub fn validate(&self) -> Result<(), ValidationError> {
        clean("party.account", &self.account, 64)?;
        clean("party.label", &self.label, MAX_LABEL_CHARS)?;
        ed_key("party.owner_pub", &self.owner_pub)?;
        if self.epoch == 0 {
            return Err(invalid("party.epoch", "starts at 1"));
        }
        digest("party.entry_hash", &self.entry_hash)?;
        Ok(())
    }
}

impl Arrangement {
    pub fn validate(&self) -> Result<(), ValidationError> {
        crate::check_version(self.v)?;
        if !valid_id(&self.id) {
            return Err(invalid("id", "must be 32 lowercase hex characters"));
        }
        if self.revision == 0 {
            return Err(invalid("revision", "starts at 1"));
        }
        self.owner.validate()?;
        self.helper.validate()?;
        // Nobody shares control with themself: not by account, not by key (02 §3, `exclude_requester`).
        if self.owner.account == self.helper.account || self.owner.owner_pub == self.helper.owner_pub {
            return Err(invalid("helper", "must be another account with another owner key"));
        }
        self.terms.validate()
    }
}

/// The JSON text of an arrangement, as it is signed.
pub fn arrangement_body(a: &Arrangement) -> Result<String, ValidationError> {
    a.validate()?;
    serde_json::to_string(a).map_err(|e| invalid("arrangement", e.to_string()))
}

/// The SHA-256 (hex) of a signed arrangement's body: what a [`LinkChange`] names.
#[must_use]
pub fn arrangement_hash(body: &str) -> String {
    hex32(&sha256(body.as_bytes()))
}

/// What the helper's owner key signs: context `reins/circle-arrangement/v1` and the body's SHA-256.
#[must_use]
pub fn helper_message(body: &str) -> Vec<u8> {
    let mut c = Canon::new(contexts::CIRCLE_ARRANGEMENT);
    c.key(&sha256(body.as_bytes()));
    c.finish()
}

/// What the owner's owner key signs when it confirms: context `reins/circle-activate/v1`, the body's SHA-256 and the
/// helper's signature (64 bytes).
pub fn owner_message(body: &str, helper_sig: &str) -> Result<Vec<u8>, ValidationError> {
    let sig = parse_sig(helper_sig).ok_or_else(|| invalid("helper_sig", "must be 64 bytes, base64url"))?;
    let mut c = Canon::new(contexts::CIRCLE_ACTIVATE);
    c.key(&sha256(body.as_bytes())).bytes(&sig);
    Ok(c.finish())
}

fn verify_sig(
    key: &str,
    msg: &[u8],
    sig: &str,
    verify: &impl Fn(&[u8; 32], &[u8], &[u8; 64]) -> bool,
) -> Result<(), CircleError> {
    let key = ed_key("key", key)?;
    let sig = parse_sig(sig).ok_or(CircleError::BadSignature)?;
    if verify(&key, msg, &sig) {
        Ok(())
    } else {
        Err(CircleError::BadSignature)
    }
}

impl SignedArrangement {
    /// The arrangement, parsed and validated (signatures unchecked: [`verify_arrangement`]).
    pub fn arrangement(&self) -> Result<Arrangement, CircleError> {
        let a: Arrangement =
            serde_json::from_str(&self.body).map_err(|e| CircleError::Invalid(invalid("body", e.to_string())))?;
        a.validate()?;
        Ok(a)
    }
}

/// The arrangement, if the helper's signature verifies (and the owner's, when `need_owner`), each under the owner key
/// the arrangement names for that side.
pub fn verify_arrangement(
    s: &SignedArrangement,
    need_owner: bool,
    verify: impl Fn(&[u8; 32], &[u8], &[u8; 64]) -> bool,
) -> Result<Arrangement, CircleError> {
    let a = s.arrangement()?;
    verify_sig(&a.helper.owner_pub, &helper_message(&s.body), &s.helper_sig, &verify)?;
    if need_owner {
        verify_sig(&a.owner.owner_pub, &owner_message(&s.body, &s.helper_sig)?, &s.owner_sig, &verify)?;
    }
    Ok(a)
}

// ---- invitations --------------------------------------------------------------------------------------------------

/// The owner's commitment, hex: SHA-256 over context `reins/circle-commit/v1`, `owner_account` (string), `owner_pub`
/// (32 bytes), `owner_epoch` (u64) and the owner's nonce (32 bytes). `None` unless the key is 32 bytes.
#[must_use]
pub fn invite_commitment(owner_account: &str, owner_pub: &str, owner_epoch: u64, nonce: &[u8; 32]) -> Option<String> {
    let mut c = Canon::new(contexts::CIRCLE_COMMIT);
    c.str(owner_account).key(&decode_key(owner_pub)?).u64(owner_epoch).key(nonce);
    Some(hex32(&sha256(&c.finish())))
}

/// Whether the revealed nonce opens `committed` (constant time in the hash).
#[must_use]
pub fn check_invite_commitment(
    committed: &str,
    owner_account: &str,
    owner_pub: &str,
    owner_epoch: u64,
    nonce: &[u8; 32],
) -> bool {
    let (Some(want), Some(got)) = (
        parse_hex32(committed),
        invite_commitment(owner_account, owner_pub, owner_epoch, nonce).and_then(|h| parse_hex32(&h)),
    ) else {
        return false;
    };
    crate::enroll::ct_eq(&want, &got)
}

/// Everything both phones agree on when their codes match.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InviteTranscript<'a> {
    /// The server's id of the invitation.
    pub invite: &'a str,
    pub owner_account: &'a str,
    pub owner_pub: &'a str,
    pub owner_epoch: u64,
    pub helper_account: &'a str,
    pub helper_pub: &'a str,
    pub helper_epoch: u64,
    pub owner_nonce: [u8; 32],
    pub helper_nonce: [u8; 32],
}

impl InviteTranscript<'_> {
    /// The six digits ("482 193"): SHA-256 over context `reins/circle-code/v1`, `invite`, `owner_account` (strings),
    /// `owner_pub` (32 bytes), `owner_epoch` (u64), `helper_account` (string), `helper_pub` (32 bytes), `helper_epoch`
    /// (u64), the owner's then the helper's nonce (32 bytes each).
    pub fn code(&self) -> Result<String, ValidationError> {
        let mut c = Canon::new(contexts::CIRCLE_CODE);
        c.str(self.invite)
            .str(self.owner_account)
            .key(&ed_key("owner_pub", self.owner_pub)?)
            .u64(self.owner_epoch)
            .str(self.helper_account)
            .key(&ed_key("helper_pub", self.helper_pub)?)
            .u64(self.helper_epoch)
            .key(&self.owner_nonce)
            .key(&self.helper_nonce);
        let d = sha256(&c.finish());
        let n = u64::from_be_bytes([d[0], d[1], d[2], d[3], d[4], d[5], d[6], d[7]]) % 1_000_000;
        Ok(format!("{:03} {:03}", n / 1000, n % 1000))
    }
}

/// `POST /reins/api/circles/invites`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewInvite {
    pub v: u32,
    pub kind: CircleKind,
    pub commitment: String,
    pub owner_pub: String,
    pub owner_epoch: u64,
}

impl NewInvite {
    pub fn validate(&self) -> Result<(), ValidationError> {
        crate::check_version(self.v)?;
        digest("commitment", &self.commitment)?;
        ed_key("owner_pub", &self.owner_pub)?;
        if self.owner_epoch == 0 {
            return Err(invalid("owner_epoch", "starts at 1"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InviteCreated {
    pub id: String,
    /// The one-time secret of the link (base64url, 32 bytes); the server keeps only its SHA-256.
    pub token: String,
    pub expires_at: i64,
}

/// `POST /reins/api/circles/join {token}`: what a helper sees before answering.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InviteOpened {
    pub v: u32,
    pub id: String,
    pub kind: CircleKind,
    /// The inviting account's email, as the server knows it (a label, not a proof).
    pub inviter: String,
    pub inviter_account: String,
    pub commitment: String,
    pub owner_pub: String,
    pub owner_epoch: u64,
    pub expires_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JoinBody {
    pub token: String,
}

/// The helper's contribution.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InviteChallenge {
    pub v: u32,
    /// The link's secret (blanked when the server shows the challenge to the owner).
    #[serde(default)]
    pub token: String,
    pub helper_pub: String,
    pub helper_epoch: u64,
    /// 32 random bytes, base64url.
    pub nonce: String,
}

impl InviteChallenge {
    pub fn validate(&self) -> Result<(), ValidationError> {
        crate::check_version(self.v)?;
        ed_key("helper_pub", &self.helper_pub)?;
        if self.helper_epoch == 0 {
            return Err(invalid("helper_epoch", "starts at 1"));
        }
        crate::enroll::parse_nonce(&self.nonce).ok_or_else(|| invalid("nonce", "must be 32 bytes, base64url"))?;
        Ok(())
    }
}

/// The owner's nonce and the proposed terms ([`SealedTerms`], sealed to the helper's approval phone).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InviteReveal {
    pub v: u32,
    pub nonce: String,
    pub sealed_terms: String,
}

impl InviteReveal {
    pub fn validate(&self) -> Result<(), ValidationError> {
        crate::check_version(self.v)?;
        crate::enroll::parse_nonce(&self.nonce).ok_or_else(|| invalid("nonce", "must be 32 bytes, base64url"))?;
        sealed("sealed_terms", &self.sealed_terms)
    }
}

/// Inside [`InviteReveal::sealed_terms`]: the arrangement's body as the owner proposes it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SealedTerms {
    pub v: u32,
    pub body: String,
}

/// The helper's signed arrangement ([`SignedArrangement`] without `owner_sig`), sealed to the owner's approval phone.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InviteAccept {
    pub v: u32,
    pub sealed: String,
}

/// The owner's confirmation: the link's id (the arrangement id) and each side's copy of the [`SignedArrangement`],
/// sealed to that side's roster credential key.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InviteActivate {
    pub v: u32,
    pub link: String,
    pub owner_copy: String,
    pub helper_copy: String,
}

fn sealed(field: &'static str, value: &str) -> Result<(), ValidationError> {
    if value.is_empty() || value.len() > MAX_SEALED_CHARS || decode_payload(value).is_none() {
        return Err(invalid(field, format!("must be base64url, at most {MAX_SEALED_CHARS} characters")));
    }
    Ok(())
}

impl InviteAccept {
    pub fn validate(&self) -> Result<(), ValidationError> {
        crate::check_version(self.v)?;
        sealed("sealed", &self.sealed)
    }
}

impl InviteActivate {
    pub fn validate(&self) -> Result<(), ValidationError> {
        crate::check_version(self.v)?;
        if !valid_id(&self.link) {
            return Err(invalid("link", "must be 32 lowercase hex characters"));
        }
        sealed("owner_copy", &self.owner_copy)?;
        sealed("helper_copy", &self.helper_copy)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InviteState {
    Open,
    Challenged,
    Revealed,
    Accepted,
    Active,
    Declined,
    Cancelled,
    Expired,
}

impl InviteState {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Challenged => "challenged",
            Self::Revealed => "revealed",
            Self::Accepted => "accepted",
            Self::Active => "active",
            Self::Declined => "declined",
            Self::Cancelled => "cancelled",
            Self::Expired => "expired",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        [
            Self::Open,
            Self::Challenged,
            Self::Revealed,
            Self::Accepted,
            Self::Active,
            Self::Declined,
            Self::Cancelled,
            Self::Expired,
        ]
        .into_iter()
        .find(|k| k.as_str() == s)
    }

    /// The protocol's order: each step once, forward only. Declining, cancelling and expiring end any open invitation.
    #[must_use]
    pub fn can_go(self, to: Self) -> bool {
        use InviteState::{Accepted, Active, Cancelled, Challenged, Declined, Expired, Open, Revealed};
        matches!(
            (self, to),
            (Open, Challenged)
                | (Challenged, Revealed)
                | (Revealed, Accepted)
                | (Accepted, Active)
                | (Open | Challenged | Revealed | Accepted, Declined | Cancelled | Expired)
        )
    }

    #[must_use]
    pub const fn is_open(self) -> bool {
        matches!(self, Self::Open | Self::Challenged | Self::Revealed | Self::Accepted)
    }
}

/// An invitation as either side reads it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InviteStatus {
    pub v: u32,
    pub id: String,
    pub state: InviteState,
    pub kind: CircleKind,
    pub owner_account: String,
    /// The inviting account's email (a label).
    pub owner: String,
    pub owner_pub: String,
    pub owner_epoch: u64,
    pub commitment: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub helper_account: Option<String>,
    /// The helper's email (a label).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub helper: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub challenge: Option<InviteChallenge>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reveal: Option<InviteReveal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accept: Option<InviteAccept>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    pub expires_at: i64,
}

// ---- links --------------------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkState {
    Active,
    Paused,
    Ended,
}

impl LinkState {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Paused => "paused",
            Self::Ended => "ended",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        [Self::Active, Self::Paused, Self::Ended].into_iter().find(|k| k.as_str() == s)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Owner,
    Helper,
}

/// A change of a link's state: pause or resume (the owner), end (either side; leaving is the helper ending it).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkChange {
    pub v: u32,
    pub link: String,
    /// [`arrangement_hash`] of the signed arrangement it changes: a change of one arrangement means nothing for
    /// another, or for a later revision.
    pub arrangement: String,
    /// 1, 2, …: one more than the last change of this link either side accepted. A change that isn't strictly newer is
    /// a replay ([`SignedChange::verify`]).
    pub seq: u64,
    pub state: LinkState,
    pub by: Side,
    pub at: i64,
}

impl LinkChange {
    pub fn validate(&self) -> Result<(), ValidationError> {
        crate::check_version(self.v)?;
        if !valid_id(&self.link) {
            return Err(invalid("link", "must be 32 lowercase hex characters"));
        }
        digest("arrangement", &self.arrangement)?;
        if self.seq == 0 {
            return Err(invalid("seq", "starts at 1"));
        }
        if self.by == Side::Helper && self.state != LinkState::Ended {
            return Err(invalid("state", "a helper can only end an arrangement"));
        }
        Ok(())
    }
}

/// A [`LinkChange`]'s JSON and the changer's owner-key signature over [`change_message`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedChange {
    pub body: String,
    pub sig: String,
}

/// Context `reins/circle-change/v1` and the body's SHA-256.
#[must_use]
pub fn change_message(body: &str) -> Vec<u8> {
    let mut c = Canon::new(contexts::CIRCLE_CHANGE);
    c.key(&sha256(body.as_bytes()));
    c.finish()
}

impl SignedChange {
    pub fn change(&self) -> Result<LinkChange, CircleError> {
        let c: LinkChange =
            serde_json::from_str(&self.body).map_err(|e| CircleError::Invalid(invalid("body", e.to_string())))?;
        c.validate()?;
        Ok(c)
    }

    /// The change, if `key` (the changer's pinned owner key) signed it, for this arrangement (`arrangement`: its
    /// [`arrangement_hash`]), and it is strictly newer than `last_seq` (the last change this phone accepted for the
    /// link; 0 for none). An older or repeated change is [`CircleError::Replayed`].
    pub fn verify(
        &self,
        key: &str,
        arrangement: &str,
        last_seq: u64,
        verify: impl Fn(&[u8; 32], &[u8], &[u8; 64]) -> bool,
    ) -> Result<LinkChange, CircleError> {
        let c = self.change()?;
        verify_sig(key, &change_message(&self.body), &self.sig, &verify)?;
        if c.arrangement != arrangement {
            return Err(CircleError::Mismatch("arrangement"));
        }
        if c.seq <= last_seq {
            return Err(CircleError::Replayed);
        }
        Ok(c)
    }
}

/// `GET /reins/api/circles/links`: one arrangement as the caller's side sees its routing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkView {
    pub id: String,
    pub side: Side,
    pub kind: CircleKind,
    pub state: LinkState,
    /// The other side's email (a label).
    pub peer: String,
    pub peer_account: String,
    /// The caller's copy of the [`SignedArrangement`], sealed to its credential key.
    pub copy: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change: Option<SignedChange>,
    pub updated_at: i64,
}

// ---- packets and answers ------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PacketKind {
    /// A request waiting for the helper's answer.
    Review,
    /// Something a watcher (or a helper) is told; no answer.
    Notice,
}

impl PacketKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Review => "review",
            Self::Notice => "notice",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        [Self::Review, Self::Notice].into_iter().find(|k| k.as_str() == s)
    }
}

/// The card a helper decides from (04 §6.2): as the owner sees it, within the scope's redaction. Never the raw request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewCard {
    pub title: String,
    pub subtitle: String,
    pub headline: String,
    pub service: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    /// The owner's agent ("ChatGPT").
    pub agent: String,
    /// What it does ("Ask · Outside · Reaches people").
    pub line: String,
    /// Only with [`Redaction::Full`]: e.g. an email's subject and recipients.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub created_at: i64,
    pub deadline: i64,
}

/// Longest card fields (characters).
pub const MAX_CARD_TITLE: usize = 200;
pub const MAX_CARD_LINE: usize = 300;
pub const MAX_CARD_DETAIL: usize = 2_000;
pub const MAX_CARD_SHORT: usize = 120;
pub const MAX_ACCOUNT_CHARS: usize = 254;

/// Whether `c` may stand in signed text a person reads: no control character (a newline only where `newlines`), and
/// nothing that hides or reorders text (bidi controls, zero-width characters: [`crate::payments::is_format_char`]).
fn shown_char(c: char, newlines: bool) -> bool {
    (!c.is_control() || (newlines && c == '\n')) && !crate::payments::is_format_char(c)
}

/// Text that may be empty: at most `max` characters, every one [`shown_char`].
fn shown(field: &'static str, value: &str, max: usize, newlines: bool) -> Result<(), ValidationError> {
    if value.chars().count() > max {
        return Err(invalid(field, format!("at most {max} characters")));
    }
    if !value.chars().all(|c| shown_char(c, newlines)) {
        return Err(invalid(field, "must not contain control or invisible formatting characters"));
    }
    Ok(())
}

/// `value` with every character [`shown_char`] refuses removed, cut to `max` characters: what a signer writes, so
/// that the receiver's check ([`shown`]) passes.
#[must_use]
pub fn shown_text(value: &str, max: usize, newlines: bool) -> String {
    value.chars().filter(|c| shown_char(*c, newlines)).take(max).collect()
}

impl ReviewCard {
    /// Every text field within its length, with no control, bidi or invisible character (empty is allowed, except
    /// the title).
    pub fn validate(&self) -> Result<(), ValidationError> {
        clean("card.title", &self.title, MAX_CARD_TITLE)?;
        shown("card.subtitle", &self.subtitle, MAX_CARD_LINE, false)?;
        shown("card.headline", &self.headline, MAX_CARD_LINE, false)?;
        shown("card.service", &self.service, MAX_CARD_SHORT, false)?;
        if let Some(account) = &self.account {
            shown("card.account", account, MAX_ACCOUNT_CHARS, false)?;
        }
        shown("card.agent", &self.agent, MAX_CARD_SHORT, false)?;
        shown("card.line", &self.line, MAX_CARD_LINE, false)?;
        if let Some(detail) = &self.detail {
            shown("card.detail", detail, MAX_CARD_DETAIL, true)?;
        }
        Ok(())
    }

    /// The same card with every text field made valid ([`shown_text`]): what the owner's phone signs.
    #[must_use]
    pub fn cleaned(self) -> Self {
        let title = shown_text(&self.title, MAX_CARD_TITLE, false);
        Self {
            title: if title.is_empty() {
                "Request".to_owned()
            } else {
                title
            },
            subtitle: shown_text(&self.subtitle, MAX_CARD_LINE, false),
            headline: shown_text(&self.headline, MAX_CARD_LINE, false),
            service: shown_text(&self.service, MAX_CARD_SHORT, false),
            account: self.account.map(|a| shown_text(&a, MAX_ACCOUNT_CHARS, false)),
            agent: shown_text(&self.agent, MAX_CARD_SHORT, false),
            line: shown_text(&self.line, MAX_CARD_LINE, false),
            detail: self.detail.map(|d| shown_text(&d, MAX_CARD_DETAIL, true)),
            ..self
        }
    }

    /// The card a helper with `redaction` may see.
    #[must_use]
    pub fn redacted(mut self, redaction: Redaction) -> Self {
        if redaction == Redaction::Summary {
            self.detail = None;
        }
        self
    }
}

/// What a watcher is told ("Approved: Send email to ann@acme.com", "Bypass on for 1 h").
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notice {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    pub at: i64,
}

impl Notice {
    pub fn validate(&self) -> Result<(), ValidationError> {
        clean("notice.text", &self.text, MAX_CARD_LINE)?;
        if let Some(outcome) = &self.outcome {
            shown("notice.outcome", outcome, MAX_CARD_SHORT, false)?;
        }
        Ok(())
    }
}

/// The other side's current approval phone, as [`peer_keys`] verified it: what a packet or an answer must name and be
/// signed by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Signer<'a> {
    pub device_id: &'a str,
    pub sign_pub: &'a str,
    pub epoch: u64,
    pub entry_hash: &'a str,
}

impl Signer<'_> {
    /// A message names the entry it was signed under. An older entry (a stale chain, a phone since replaced) or one
    /// the verified chain never reached are both refused.
    fn check(&self, device_id: &str, epoch: u64, entry_hash: &str) -> Result<(), CircleError> {
        if epoch < self.epoch {
            return Err(CircleError::NotCurrentSigner);
        }
        if epoch > self.epoch || entry_hash != self.entry_hash {
            // The signer says its roster is newer than the chain this phone was shown: the chain is stale.
            return Err(CircleError::BrokenChain);
        }
        if device_id != self.device_id {
            return Err(CircleError::NotCurrentSigner);
        }
        Ok(())
    }
}

/// What the owner's approval phone signs into a packet.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PacketBody {
    pub v: u32,
    /// 32 hex, chosen by the owner's phone (the server's packet id).
    pub id: String,
    pub arrangement: String,
    pub kind: PacketKind,
    /// The owner's request digest (hex): what an answer binds; it reveals nothing of the request.
    pub digest: String,
    /// 16 random bytes (base64url) the answer must repeat.
    pub nonce: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub card: Option<ReviewCard>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notice: Option<Notice>,
    /// The owner's approval phone (its Vaultwarden device id), its roster epoch and that entry's hash: the receiver
    /// refuses a roster that doesn't reach this entry, so a stale chain can't vouch for a signer.
    pub device_id: String,
    pub epoch: u64,
    pub entry_hash: String,
    pub at: i64,
}

impl PacketBody {
    pub fn validate(&self) -> Result<(), ValidationError> {
        crate::check_version(self.v)?;
        if !valid_id(&self.id) || !valid_id(&self.arrangement) {
            return Err(invalid("id", "must be 32 lowercase hex characters"));
        }
        digest("digest", &self.digest)?;
        if !valid_nonce16(&self.nonce) {
            return Err(invalid("nonce", "must be 16 bytes, base64url"));
        }
        match (self.kind, &self.card, &self.notice) {
            (PacketKind::Review, Some(card), None) => card.validate()?,
            (PacketKind::Notice, None, Some(notice)) => notice.validate()?,
            (PacketKind::Review, ..) => return Err(invalid("card", "a review carries a card and no notice")),
            (PacketKind::Notice, ..) => return Err(invalid("notice", "a notice carries a notice and no card")),
        }
        clean("device_id", &self.device_id, 64)?;
        if self.epoch == 0 {
            return Err(invalid("epoch", "starts at 1"));
        }
        digest("entry_hash", &self.entry_hash)?;
        Ok(())
    }
}

/// A packet's JSON body and the owner's approval-phone signature over [`packet_message`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedPacket {
    pub body: String,
    pub sig: String,
}

/// Context `reins/circle-packet/v1` and the body's SHA-256.
#[must_use]
pub fn packet_message(body: &str) -> Vec<u8> {
    let mut c = Canon::new(contexts::CIRCLE_PACKET);
    c.key(&sha256(body.as_bytes()));
    c.finish()
}

impl SignedPacket {
    pub fn body(&self) -> Result<PacketBody, CircleError> {
        let b: PacketBody =
            serde_json::from_str(&self.body).map_err(|e| CircleError::Invalid(invalid("body", e.to_string())))?;
        b.validate()?;
        Ok(b)
    }

    /// The body, if the owner's current approval phone signed it, at the roster entry the receiver verified as
    /// current ([`peer_keys`]).
    pub fn verify(
        &self,
        signer: &Signer<'_>,
        verify: impl Fn(&[u8; 32], &[u8], &[u8; 64]) -> bool,
    ) -> Result<PacketBody, CircleError> {
        let b = self.body()?;
        signer.check(&b.device_id, b.epoch, &b.entry_hash)?;
        verify_sig(signer.sign_pub, &packet_message(&self.body), &self.sig, &verify)?;
        Ok(b)
    }
}

/// `POST /reins/api/circles/packets`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewPacket {
    pub v: u32,
    pub id: String,
    pub link: String,
    pub kind: PacketKind,
    pub deadline: i64,
    /// A [`SignedPacket`] sealed to the helper's approval phone.
    pub sealed: String,
}

impl NewPacket {
    pub fn validate(&self, now: i64) -> Result<(), ValidationError> {
        crate::check_version(self.v)?;
        if !valid_id(&self.id) || !valid_id(&self.link) {
            return Err(invalid("id", "must be 32 lowercase hex characters"));
        }
        if self.deadline <= now || self.deadline > now + i64::try_from(DEFERRED_MAX_SECS).unwrap_or(i64::MAX) {
            return Err(invalid("deadline", "must be within 7 days from now"));
        }
        sealed("sealed", &self.sealed)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PacketState {
    Waiting,
    Answered,
    Withdrawn,
    Expired,
}

impl PacketState {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Waiting => "waiting",
            Self::Answered => "answered",
            Self::Withdrawn => "withdrawn",
            Self::Expired => "expired",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        [Self::Waiting, Self::Answered, Self::Withdrawn, Self::Expired].into_iter().find(|k| k.as_str() == s)
    }
}

/// `GET /reins/api/circles/inbox`: a packet addressed to the caller.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InboxItem {
    pub id: String,
    pub link: String,
    pub kind: PacketKind,
    pub state: PacketState,
    pub deadline: i64,
    pub created_at: i64,
    pub sealed: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Vote {
    Approve,
    Deny,
}

/// A helper's answer, signed by its approval phone.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CircleAnswer {
    pub v: u32,
    pub arrangement: String,
    pub packet: String,
    pub digest: String,
    /// The packet's nonce, repeated.
    pub packet_nonce: String,
    pub vote: Vote,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub at: i64,
    /// 16 random bytes of the answer's own.
    pub nonce: String,
    /// The helper's approval phone, and its roster epoch and entry hash when it signed (see [`PacketBody::entry_hash`]).
    pub epoch: u64,
    pub entry_hash: String,
    pub device_id: String,
}

impl CircleAnswer {
    pub fn validate(&self) -> Result<(), ValidationError> {
        crate::check_version(self.v)?;
        if !valid_id(&self.arrangement) || !valid_id(&self.packet) {
            return Err(invalid("id", "must be 32 lowercase hex characters"));
        }
        digest("digest", &self.digest)?;
        if !valid_nonce16(&self.packet_nonce) || !valid_nonce16(&self.nonce) {
            return Err(invalid("nonce", "must be 16 bytes, base64url"));
        }
        if let Some(note) = &self.note {
            clean("note", note, MAX_NOTE_CHARS)?;
        }
        if self.epoch == 0 {
            return Err(invalid("epoch", "starts at 1"));
        }
        digest("entry_hash", &self.entry_hash)?;
        clean("device_id", &self.device_id, 64)?;
        Ok(())
    }
}

/// An answer's JSON body and the helper's approval-phone signature over [`answer_message`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedCircleAnswer {
    pub body: String,
    pub sig: String,
}

/// Context `reins/circle-answer/v1` and the body's SHA-256.
#[must_use]
pub fn answer_message(body: &str) -> Vec<u8> {
    let mut c = Canon::new(contexts::CIRCLE_ANSWER);
    c.key(&sha256(body.as_bytes()));
    c.finish()
}

/// `POST /reins/api/circles/packets/<id>/answer`: a [`SignedCircleAnswer`] sealed to the owner's approval phone.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewAnswer {
    pub v: u32,
    pub sealed: String,
}

impl NewAnswer {
    pub fn validate(&self) -> Result<(), ValidationError> {
        crate::check_version(self.v)?;
        sealed("sealed", &self.sealed)
    }
}

/// `GET /reins/api/circles/answers`: an answer waiting for the owner's phone.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnswerItem {
    pub packet: String,
    pub link: String,
    pub sealed: String,
    pub answered_at: i64,
}

/// `POST /reins/api/circles/answers/seen`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Seen {
    pub ids: Vec<String>,
}

/// What the owner's phone expects of an answer to one of its packets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Expect<'a> {
    pub arrangement: &'a str,
    pub packet: &'a str,
    pub digest: &'a str,
    pub packet_nonce: &'a str,
    pub created_at: i64,
    pub deadline: i64,
    /// The helper's current approval phone ([`peer_keys`]).
    pub signer: Signer<'a>,
}

/// The answer, if it is the helper's current approval phone's answer to exactly this packet, in its window.
pub fn check_answer(
    s: &SignedCircleAnswer,
    expect: &Expect<'_>,
    verify: impl Fn(&[u8; 32], &[u8], &[u8; 64]) -> bool,
) -> Result<CircleAnswer, CircleError> {
    let a: CircleAnswer =
        serde_json::from_str(&s.body).map_err(|e| CircleError::Invalid(invalid("body", e.to_string())))?;
    a.validate()?;
    if a.arrangement != expect.arrangement {
        return Err(CircleError::Mismatch("arrangement"));
    }
    if a.packet != expect.packet {
        return Err(CircleError::Mismatch("packet"));
    }
    if a.digest != expect.digest {
        return Err(CircleError::Mismatch("digest"));
    }
    if a.packet_nonce != expect.packet_nonce {
        return Err(CircleError::Mismatch("nonce"));
    }
    expect.signer.check(&a.device_id, a.epoch, &a.entry_hash)?;
    if a.at < expect.created_at - ANSWER_SKEW_SECS || a.at > expect.deadline + ANSWER_SKEW_SECS {
        return Err(CircleError::Stale);
    }
    verify_sig(expect.signer.sign_pub, &answer_message(&s.body), &s.sig, &verify)?;
    Ok(a)
}

// ---- the other side's roster ---------------------------------------------------------------------------------------

/// The other side's keys as its roster says now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PeerKeys {
    Current {
        device_id: String,
        sign_pub: String,
        box_pub: String,
        credential_pub: String,
        epoch: u64,
        hash: String,
    },
    /// An entry after the anchor changed the owner key (a recovery): the arrangement is suspended until a new code.
    OwnerChanged {
        epoch: u64,
    },
}

impl PeerKeys {
    /// The approval phone to verify a message against; `None` while suspended.
    #[must_use]
    pub fn signer(&self) -> Option<Signer<'_>> {
        match self {
            Self::Current {
                device_id,
                sign_pub,
                epoch,
                hash,
                ..
            } => Some(Signer {
                device_id,
                sign_pub,
                epoch: *epoch,
                entry_hash: hash,
            }),
            Self::OwnerChanged {
                ..
            } => None,
        }
    }
}

/// Verifies `chain` (the peer's roster, oldest first) from the anchor this phone pinned or last verified (`known`:
/// epoch and entry hash) and returns its current approval phone. Every entry after the anchor must verify against the
/// one before it ([`verify_step`]) and keep `pinned_owner`. The head must be at least `seen_epoch` (the newest epoch
/// this phone ever verified for the peer): a server showing an older head (a rollback) is refused.
pub fn peer_keys(
    pinned_owner: &str,
    known: (u64, &str),
    seen_epoch: u64,
    chain: &[RosterEntry],
    verify: impl Fn(&[u8; 32], &[u8], &[u8; 64]) -> bool,
) -> Result<PeerKeys, CircleError> {
    if chain.len() > MAX_PEER_CHAIN {
        return Err(CircleError::BrokenChain);
    }
    if chain.last().is_none_or(|head| head.epoch < seen_epoch.max(known.0)) {
        return Err(CircleError::Replayed);
    }
    let start = chain.iter().position(|e| e.epoch == known.0).ok_or(CircleError::BrokenChain)?;
    let anchor = &chain[start];
    if anchor.hash_hex().map_err(CircleError::Invalid)? != known.1 {
        return Err(CircleError::BrokenChain);
    }
    if anchor.owner_pub != pinned_owner {
        return Err(CircleError::Mismatch("owner key"));
    }
    let mut prev = anchor;
    for entry in &chain[start + 1..] {
        verify_step(Some(prev), entry, &verify).map_err(|_| CircleError::BrokenChain)?;
        if entry.owner_pub != pinned_owner {
            return Ok(PeerKeys::OwnerChanged {
                epoch: entry.epoch,
            });
        }
        prev = entry;
    }
    Ok(PeerKeys::Current {
        device_id: prev.primary.device_id.clone(),
        sign_pub: prev.primary.sign_pub.clone(),
        box_pub: prev.primary.box_pub.clone(),
        credential_pub: prev.credential_pub.clone(),
        epoch: prev.epoch,
        hash: prev.hash_hex().map_err(CircleError::Invalid)?,
    })
}

/// A fresh protocol-version value, for callers building these types.
#[must_use]
pub const fn version() -> u32 {
    PROTOCOL_VERSION
}

/// 16 bytes as a nonce travels.
#[must_use]
pub fn encode_nonce16(raw: &[u8; 16]) -> String {
    encode_payload(raw)
}

/// Whether `key` is an X25519 key as it travels (32 bytes, canonical base64url).
#[must_use]
pub fn valid_box_key(key: &str) -> bool {
    x_key("key", key).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        desktop::encode_key,
        roster::{
            RosterReason,
            tests::{fake_sign, fake_verify, genesis, key, member, next},
        },
    };

    fn sign(seed: u8, msg: &[u8]) -> String {
        crate::canon::encode_sig(&fake_sign(&[seed; 32], msg))
    }

    fn party(account: &str, owner: u8) -> Party {
        Party {
            account: account.to_owned(),
            label: format!("{account}@example.com"),
            owner_pub: key(owner),
            epoch: 1,
            entry_hash: hex32(&[owner; 32]),
        }
    }

    fn arrangement(kind: CircleKind) -> Arrangement {
        Arrangement {
            v: 1,
            id: "0123456789abcdef0123456789abcdef".to_owned(),
            revision: 1,
            owner: party("dana", 50),
            helper: party("alex", 80),
            terms: Terms::default_for(kind),
            created_at: 100,
        }
    }

    fn signed(a: &Arrangement) -> SignedArrangement {
        let body = arrangement_body(a).unwrap();
        let helper_sig = sign(80, &helper_message(&body));
        let owner_sig = sign(50, &owner_message(&body, &helper_sig).unwrap());
        SignedArrangement {
            body,
            helper_sig,
            owner_sig,
        }
    }

    #[test]
    fn arrangement_round_trips_and_signatures_verify() {
        let a = arrangement(CircleKind::Cosign);
        let s = signed(&a);
        let json = serde_json::to_string(&s).unwrap();
        let back: SignedArrangement = serde_json::from_str(&json).unwrap();
        assert_eq!(verify_arrangement(&back, true, fake_verify).unwrap(), a);
        // Before the owner confirms, only the helper's signature exists.
        let mut half = s;
        half.owner_sig = String::new();
        assert!(verify_arrangement(&half, false, fake_verify).is_ok());
        assert!(verify_arrangement(&half, true, fake_verify).is_err());
    }

    #[test]
    fn an_arrangement_with_oneself_is_invalid() {
        let mut a = arrangement(CircleKind::Cosign);
        a.helper.account = "dana".to_owned();
        assert!(a.validate().is_err());
        let mut b = arrangement(CircleKind::Cosign);
        b.helper.owner_pub = key(50);
        assert!(b.validate().is_err());
    }

    #[test]
    fn a_forged_arrangement_is_refused() {
        let a = arrangement(CircleKind::Cosign);
        // Signed by a key the helper does not hold.
        let body = arrangement_body(&a).unwrap();
        let bad_helper = SignedArrangement {
            helper_sig: sign(81, &helper_message(&body)),
            owner_sig: String::new(),
            body: body.clone(),
        };
        assert_eq!(verify_arrangement(&bad_helper, false, fake_verify), Err(CircleError::BadSignature));
        // A wider scope under the old signatures.
        let mut s = signed(&a);
        let mut wider = a.clone();
        wider.terms.scope.effects = vec!["money".to_owned()];
        wider.terms.kind = CircleKind::Delegate;
        s.body = arrangement_body(&wider).unwrap();
        assert_eq!(verify_arrangement(&s, true, fake_verify), Err(CircleError::BadSignature));
        // Another helper, signing for itself, under the owner's old signature.
        let mut swapped = a.clone();
        swapped.helper = party("eve", 90);
        let body = arrangement_body(&swapped).unwrap();
        let helper_sig = sign(90, &helper_message(&body));
        let forged = SignedArrangement {
            owner_sig: signed(&a).owner_sig,
            helper_sig,
            body,
        };
        assert!(verify_arrangement(&forged, false, fake_verify).is_ok());
        assert_eq!(verify_arrangement(&forged, true, fake_verify), Err(CircleError::BadSignature));
    }

    #[test]
    fn terms_bounds() {
        let mut t = Terms::default_for(CircleKind::Cosign);
        t.class = Class::Instant;
        t.window_secs = 601;
        assert!(t.validate().is_err());
        t.window_secs = 120;
        assert!(t.validate().is_ok());
        let mut t = Terms::default_for(CircleKind::Cosign);
        t.on_expiry = OnExpiry::Owner;
        assert!(t.validate().is_err());
        let mut t = Terms::default_for(CircleKind::Delegate);
        t.on_expiry = OnExpiry::Owner;
        assert!(t.validate().is_ok());
        let mut t = Terms::default_for(CircleKind::Watcher);
        t.redaction = Redaction::Full;
        assert!(t.validate().is_err());
        let mut t = Terms::default_for(CircleKind::Cosign);
        t.scope.effects = vec!["teleport".to_owned()];
        assert!(t.validate().is_err());
        t.scope.effects = vec!["money".to_owned()];
        t.scope.zones = vec!["outside".to_owned()];
        t.scope.min_level = Some(4);
        assert!(t.validate().is_err());
        t.scope.min_level = Some(3);
        assert!(t.validate().is_ok());
        assert_eq!(clamp_window(Class::Instant, None), 120);
        assert_eq!(clamp_window(Class::Deferred, Some(1)), 60);
    }

    fn transcript(keys: &[String; 2], on: u8, hn: u8) -> InviteTranscript<'_> {
        InviteTranscript {
            invite: "inv",
            owner_account: "dana",
            owner_pub: &keys[0],
            owner_epoch: 3,
            helper_account: "alex",
            helper_pub: &keys[1],
            helper_epoch: 1,
            owner_nonce: [on; 32],
            helper_nonce: [hn; 32],
        }
    }

    #[test]
    fn invite_code_needs_both_nonces_and_keys() {
        let keys = [key(50), key(80)];
        let base = transcript(&keys, 1, 2).code().unwrap();
        assert_eq!(base.len(), 7);
        assert_eq!(base, transcript(&keys, 1, 2).code().unwrap());
        assert_ne!(base, transcript(&keys, 9, 2).code().unwrap());
        assert_ne!(base, transcript(&keys, 1, 9).code().unwrap());
        let swapped = [key(50), key(81)];
        assert_ne!(base, transcript(&swapped, 1, 2).code().unwrap());
        let swapped = [key(51), key(80)];
        assert_ne!(base, transcript(&swapped, 1, 2).code().unwrap());
        let mut other = transcript(&keys, 1, 2);
        other.helper_account = "eve";
        assert_ne!(base, other.code().unwrap());
        let mut other = transcript(&keys, 1, 2);
        other.invite = "inv2";
        assert_ne!(base, other.code().unwrap());
        // The commitment opens only with its nonce, account, key and epoch.
        let c = invite_commitment("dana", &key(50), 3, &[1; 32]).unwrap();
        assert!(check_invite_commitment(&c, "dana", &key(50), 3, &[1; 32]));
        assert!(!check_invite_commitment(&c, "dana", &key(50), 3, &[2; 32]));
        assert!(!check_invite_commitment(&c, "dana", &key(51), 3, &[1; 32]));
        assert!(!check_invite_commitment(&c, "dana", &key(50), 4, &[1; 32]));
        assert!(!check_invite_commitment(&c, "eve", &key(50), 3, &[1; 32]));
    }

    type Change = Box<dyn Fn(&mut CircleAnswer)>;

    fn signer<'a>(sign_pub: &'a str, epoch: u64, entry_hash: &'a str) -> Signer<'a> {
        Signer {
            device_id: "dana-phone",
            sign_pub,
            epoch,
            entry_hash,
        }
    }

    fn answer() -> CircleAnswer {
        CircleAnswer {
            v: 1,
            arrangement: "0123456789abcdef0123456789abcdef".to_owned(),
            packet: "fedcba9876543210fedcba9876543210".to_owned(),
            digest: hex32(&[7; 32]),
            packet_nonce: encode_nonce16(&[1; 16]),
            vote: Vote::Approve,
            note: None,
            at: 150,
            nonce: encode_nonce16(&[2; 16]),
            epoch: 4,
            entry_hash: hex32(&[4; 32]),
            device_id: "alex-phone".to_owned(),
        }
    }

    fn signed_answer(a: &CircleAnswer, seed: u8) -> SignedCircleAnswer {
        let body = serde_json::to_string(a).unwrap();
        SignedCircleAnswer {
            sig: sign(seed, &answer_message(&body)),
            body,
        }
    }

    #[test]
    fn answers_bind_their_packet_and_digest() {
        let sign_pub = key(33);
        let digest = hex32(&[7; 32]);
        let nonce = encode_nonce16(&[1; 16]);
        let head = hex32(&[4; 32]);
        let expect = Expect {
            arrangement: "0123456789abcdef0123456789abcdef",
            packet: "fedcba9876543210fedcba9876543210",
            digest: &digest,
            packet_nonce: &nonce,
            created_at: 100,
            deadline: 400,
            signer: Signer {
                device_id: "alex-phone",
                sign_pub: &sign_pub,
                epoch: 4,
                entry_hash: &head,
            },
        };
        let good = answer();
        assert_eq!(check_answer(&signed_answer(&good, 33), &expect, fake_verify).unwrap(), good);
        // Signed by another key (a viewer, an old primary).
        assert_eq!(check_answer(&signed_answer(&good, 34), &expect, fake_verify), Err(CircleError::BadSignature));
        let cases: Vec<(Change, CircleError)> = vec![
            (
                Box::new(|a| a.arrangement = "1123456789abcdef0123456789abcdef".to_owned()),
                CircleError::Mismatch("arrangement"),
            ),
            (Box::new(|a| a.packet = "0edcba9876543210fedcba9876543210".to_owned()), CircleError::Mismatch("packet")),
            (Box::new(|a| a.digest = hex32(&[8; 32])), CircleError::Mismatch("digest")),
            (Box::new(|a| a.packet_nonce = encode_nonce16(&[9; 16])), CircleError::Mismatch("nonce")),
            (Box::new(|a| a.device_id = "alex-viewer".to_owned()), CircleError::NotCurrentSigner),
            (Box::new(|a| a.epoch = 3), CircleError::NotCurrentSigner),
            // The answer names a newer roster entry than the chain this phone was shown: the chain is stale.
            (Box::new(|a| a.epoch = 5), CircleError::BrokenChain),
            (Box::new(|a| a.entry_hash = hex32(&[5; 32])), CircleError::BrokenChain),
            (Box::new(|a| a.at = 400 + ANSWER_SKEW_SECS + 1), CircleError::Stale),
            (Box::new(|a| a.at = 100 - ANSWER_SKEW_SECS - 1), CircleError::Stale),
        ];
        for (change, err) in cases {
            let mut a = good.clone();
            change(&mut a);
            assert_eq!(check_answer(&signed_answer(&a, 33), &expect, fake_verify), Err(err));
        }
        // A note with invisible characters is refused.
        let mut a = good;
        a.note = Some("ok\u{202e}".to_owned());
        assert!(matches!(check_answer(&signed_answer(&a, 33), &expect, fake_verify), Err(CircleError::Invalid(_))));
    }

    #[test]
    fn packets_verify_against_the_current_primary_only() {
        let body = PacketBody {
            v: 1,
            id: "fedcba9876543210fedcba9876543210".to_owned(),
            arrangement: "0123456789abcdef0123456789abcdef".to_owned(),
            kind: PacketKind::Review,
            digest: hex32(&[7; 32]),
            nonce: encode_nonce16(&[1; 16]),
            card: Some(ReviewCard {
                title: "Send email".to_owned(),
                subtitle: "to ann@acme.com".to_owned(),
                headline: String::new(),
                service: "gmail".to_owned(),
                account: None,
                agent: "ChatGPT".to_owned(),
                line: "Ask · Outside".to_owned(),
                detail: Some("Subject: hi".to_owned()),
                created_at: 100,
                deadline: 400,
            }),
            notice: None,
            device_id: "dana-phone".to_owned(),
            epoch: 2,
            entry_hash: hex32(&[2; 32]),
            at: 100,
        };
        let json = serde_json::to_string(&body).unwrap();
        let p = SignedPacket {
            sig: sign(21, &packet_message(&json)),
            body: json,
        };
        let (k, other) = (key(21), key(22));
        let (h2, h3) = (hex32(&[2; 32]), hex32(&[3; 32]));
        let (k, other, h2, h3) = (k.as_str(), other.as_str(), h2.as_str(), h3.as_str());
        assert_eq!(p.verify(&signer(k, 2, h2), fake_verify).unwrap(), body);
        // The receiver knows a newer entry: this phone was replaced.
        assert_eq!(p.verify(&signer(k, 3, h3), fake_verify), Err(CircleError::NotCurrentSigner));
        // The receiver was shown an older head than the signer's: refused as stale.
        assert_eq!(p.verify(&signer(k, 1, h2), fake_verify), Err(CircleError::BrokenChain));
        assert_eq!(p.verify(&signer(k, 2, h3), fake_verify), Err(CircleError::BrokenChain));
        assert_eq!(p.verify(&signer(other, 2, h2), fake_verify), Err(CircleError::BadSignature));
        // Card text with a bidi override or a control character is refused; the signer's cleaned card passes.
        let mut sly = body;
        let card = sly.card.as_mut().unwrap();
        card.subtitle = "to ann@acme.com\u{202e}moc.live".to_owned();
        assert!(sly.validate().is_err());
        let card = sly.card.take().unwrap().cleaned();
        assert_eq!(card.subtitle, "to ann@acme.commoc.live");
        sly.card = Some(card);
        assert!(sly.validate().is_ok());
        let mut sly2 = sly.clone();
        sly2.card.as_mut().unwrap().title = "Send\u{0007}".to_owned();
        assert!(sly2.validate().is_err());
        let mut long = sly.clone();
        long.card.as_mut().unwrap().line = "x".repeat(MAX_CARD_LINE + 1);
        assert!(long.validate().is_err());
        // Newlines are allowed in the detail only.
        let mut multi = sly.clone();
        multi.card.as_mut().unwrap().detail = Some("To: a\nSubject: b".to_owned());
        assert!(multi.validate().is_ok());
        multi.card.as_mut().unwrap().headline = "a\nb".to_owned();
        assert!(multi.validate().is_err());
        // Empty optional fields are fine.
        let mut bare = sly;
        let c = bare.card.as_mut().unwrap();
        c.subtitle.clear();
        c.headline.clear();
        c.detail = None;
        assert!(bare.validate().is_ok());
        // A notice is checked the same way.
        let mut notice = bare.clone();
        notice.kind = PacketKind::Notice;
        notice.card = None;
        notice.notice = Some(Notice {
            text: "Bypass on\u{200b}".to_owned(),
            outcome: None,
            at: 1,
        });
        assert!(notice.validate().is_err());
    }

    #[test]
    fn summary_cards_carry_no_detail() {
        let card = ReviewCard {
            title: "Pay".to_owned(),
            subtitle: "$42".to_owned(),
            headline: String::new(),
            service: "payments".to_owned(),
            account: None,
            agent: "ChatGPT".to_owned(),
            line: "Ask".to_owned(),
            detail: Some("card ending 4242".to_owned()),
            created_at: 1,
            deadline: 2,
        };
        assert!(card.clone().redacted(Redaction::Summary).detail.is_none());
        assert!(card.redacted(Redaction::Full).detail.is_some());
    }

    #[test]
    fn peer_roster_must_chain_from_the_anchor() {
        let e1 = genesis();
        let e2 = next(&e1, RosterReason::Enroll, member("p", 1), vec![member("v", 2)], 50);
        let e3 = next(&e2, RosterReason::Handover, member("v", 2), vec![member("p", 1)], 50);
        let pinned = e1.owner_pub.clone();
        let h1 = e1.hash_hex().unwrap();
        let chain = vec![e1.clone(), e2.clone(), e3.clone()];
        // A handover verifies through the chain and names the new primary.
        match peer_keys(&pinned, (1, &h1), 1, &chain, fake_verify).unwrap() {
            PeerKeys::Current {
                device_id,
                epoch,
                ..
            } => {
                assert_eq!(device_id, "v");
                assert_eq!(epoch, 3);
            }
            other @ PeerKeys::OwnerChanged {
                ..
            } => panic!("{other:?}"),
        }
        // From a later anchor too.
        let h2 = e2.hash_hex().unwrap();
        assert!(matches!(peer_keys(&pinned, (2, &h2), 2, &chain, fake_verify), Ok(PeerKeys::Current { .. })));
        // A wrong anchor hash, a missing anchor, another pinned key.
        assert_eq!(peer_keys(&pinned, (1, &h2), 1, &chain, fake_verify), Err(CircleError::BrokenChain));
        assert_eq!(peer_keys(&pinned, (2, &h1), 1, &chain, fake_verify), Err(CircleError::BrokenChain));
        assert_eq!(peer_keys(&key(99), (1, &h1), 1, &chain, fake_verify), Err(CircleError::Mismatch("owner key")));
        // A tampered later entry.
        let mut bad = e3.clone();
        bad.primary = member("evil", 9);
        assert_eq!(
            peer_keys(&pinned, (1, &h1), 1, &[e1.clone(), e2.clone(), bad], fake_verify),
            Err(CircleError::BrokenChain)
        );
        // A recovery (a new owner key) suspends.
        let e4 = next(&e3, RosterReason::Recovery, member("n", 7), vec![], 77);
        assert_eq!(
            peer_keys(&pinned, (1, &h1), 1, &[e1.clone(), e2.clone(), e3.clone(), e4], fake_verify),
            Ok(PeerKeys::OwnerChanged {
                epoch: 4
            })
        );
        // A rollback: this phone saw epoch 3, the server now shows a chain ending at 2 (before the handover).
        assert_eq!(peer_keys(&pinned, (1, &h1), 3, &[e1.clone(), e2.clone()], fake_verify), Err(CircleError::Replayed));
        // A chain ending before the anchor itself.
        let h3 = e3.hash_hex().unwrap();
        assert_eq!(peer_keys(&pinned, (3, &h3), 3, &[e1, e2], fake_verify), Err(CircleError::Replayed));
    }

    #[test]
    fn invite_states_move_forward_only() {
        use InviteState::*;
        assert!(Open.can_go(Challenged));
        assert!(Challenged.can_go(Revealed));
        assert!(Revealed.can_go(Accepted));
        assert!(Accepted.can_go(Active));
        assert!(Revealed.can_go(Declined));
        assert!(!Open.can_go(Revealed));
        assert!(!Challenged.can_go(Challenged));
        assert!(!Active.can_go(Cancelled));
        assert!(!Declined.can_go(Challenged));
    }

    #[test]
    fn a_helper_can_only_end() {
        let hash = arrangement_hash("{}");
        let mut c = LinkChange {
            v: 1,
            link: "0123456789abcdef0123456789abcdef".to_owned(),
            arrangement: hash.clone(),
            seq: 3,
            state: LinkState::Paused,
            by: Side::Helper,
            at: 1,
        };
        assert!(c.validate().is_err());
        c.state = LinkState::Ended;
        assert!(c.validate().is_ok());
        let body = serde_json::to_string(&c).unwrap();
        let s = SignedChange {
            sig: sign(80, &change_message(&body)),
            body,
        };
        assert_eq!(s.verify(&key(80), &hash, 2, fake_verify).unwrap(), c);
        assert_eq!(s.verify(&key(50), &hash, 2, fake_verify), Err(CircleError::BadSignature));
    }

    #[test]
    fn link_changes_are_not_replayed() {
        let hash = arrangement_hash("{\"a\":1}");
        let change = |seq: u64, state: LinkState| {
            let c = LinkChange {
                v: 1,
                link: "0123456789abcdef0123456789abcdef".to_owned(),
                arrangement: hash.clone(),
                seq,
                state,
                by: Side::Owner,
                at: 1,
            };
            let body = serde_json::to_string(&c).unwrap();
            SignedChange {
                sig: sign(50, &change_message(&body)),
                body,
            }
        };
        let pause = change(1, LinkState::Paused);
        let resume = change(2, LinkState::Active);
        assert!(pause.verify(&key(50), &hash, 0, fake_verify).is_ok());
        assert!(resume.verify(&key(50), &hash, 1, fake_verify).is_ok());
        // An old pause, played again after the resume, is refused; so is the same change twice.
        assert_eq!(pause.verify(&key(50), &hash, 2, fake_verify), Err(CircleError::Replayed));
        assert_eq!(resume.verify(&key(50), &hash, 2, fake_verify), Err(CircleError::Replayed));
        // A change of another arrangement (or another revision of it) means nothing here.
        let other = arrangement_hash("{\"a\":2}");
        assert_eq!(pause.verify(&key(50), &other, 0, fake_verify), Err(CircleError::Mismatch("arrangement")));
        // seq 0 is never valid.
        assert!(change(0, LinkState::Paused).change().is_err());
    }

    #[test]
    fn old_pending_parses() {
        let p: crate::device::Pending = serde_json::from_str("{}").unwrap();
        assert!(!p.circles);
        assert_eq!(encode_key(&[0; 32]).len(), 43);
    }
}
