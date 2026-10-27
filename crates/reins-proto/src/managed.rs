//! Managed mode (Decisions v2 · O2): what the org's decision service, its executors, the reviewers' phones and the
//! audit verifier exchange, and every message one of them signs (`docs/design/decisions-v2/06-trust-and-compliance.md`
//! §3, §5; `08-review-resolutions.md` R5).
//!
//! - A request is bound by [`request_digest`]: the org, the member, the agent connection, the tool and its arguments
//!   in [`canonical_json`]. An approval, a decision and an execution all name that digest, so what runs is what was
//!   decided.
//! - A reviewer's phone signs [`review_message`] with its enrolled device key (ECDSA P-256): 06 §3's "(request digest,
//!   decision, policy snapshot hash, time)", bound to the org and the request or admin change.
//! - The decision service and the org issuer sign [`Envelope`]s (Ed25519): a policy snapshot, a roster, a decision, an
//!   audit checkpoint; an executor signs its receipts the same way. The body is canonical JSON text, signed as is.
//! - The audit log exports as [`ExportLine`]s, each event hashed with [`crate::org::event_hash`].
//!
//! Pure data: signatures are made and checked by the crates that hold keys (`reins-audit`, `reins-executor`, the
//! server, the phone core).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::canon::{Canon, contexts, hex32, sha256};
use crate::org::{AdminAction, Role};

/// `v` of a [`DecisionRecord`].
pub const DECISION_VERSION: u32 = 1;

/// The longest tool name, connection id or member id a digest takes.
const MAX_ID: usize = 256;

/// `v` as canonical JSON: object keys sorted (by their UTF-8 bytes) at every depth, no whitespace, strings and numbers
/// written as `serde_json` writes them. Two senders that agree on the value agree on these bytes.
#[must_use]
pub fn canonical_json(v: &Value) -> String {
    let mut out = String::new();
    write_canonical(v, &mut out);
    out
}

fn write_canonical(v: &Value, out: &mut String) {
    match v {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&Value::String((*k).clone()).to_string());
                out.push(':');
                write_canonical(&map[*k], out);
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_canonical(item, out);
            }
            out.push(']');
        }
        other => out.push_str(&other.to_string()),
    }
}

/// The digest of one Managed request: SHA-256 (hex) of the canon encoding of (`reins/org-request/v1`, org, member,
/// connection, tool, canonical JSON of the arguments). The member is the requester's user id, the connection the
/// agent connection's id.
#[must_use]
pub fn request_digest(org: &str, member: &str, connection: &str, tool: &str, args: &Value) -> String {
    let mut c = Canon::new(contexts::ORG_REQUEST);
    c.str(org).str(member).str(connection).str(tool).str(&canonical_json(args));
    hex32(&sha256(&c.finish()))
}

/// Whether the ids a digest takes are plausible (bounded, no control characters).
#[must_use]
pub fn ids_ok(ids: &[&str]) -> bool {
    ids.iter().all(|s| !s.is_empty() && s.len() <= MAX_ID && !s.chars().any(char::is_control))
}

/// The digest of a pending admin change: (`reins/org-change/v1`, org, change id, the stored action JSON text).
#[must_use]
pub fn change_digest(org: &str, change: &str, action_json: &str) -> String {
    let mut c = Canon::new(contexts::ORG_CHANGE);
    c.str(org).str(change).str(action_json);
    hex32(&sha256(&c.finish()))
}

/// What a review is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewSubject {
    /// An agent's request.
    Request,
    /// A pending admin change (05 §3).
    Change,
}

impl ReviewSubject {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Request => "request",
            Self::Change => "change",
        }
    }
}

/// What a reviewer's device signs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewDecision {
    Approve,
    Deny,
    /// The proposer's own signature on an admin change that needs two device signatures (06 §3).
    Propose,
}

impl ReviewDecision {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Approve => "approve",
            Self::Deny => "deny",
            Self::Propose => "propose",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        [Self::Approve, Self::Deny, Self::Propose].into_iter().find(|d| d.as_str() == s)
    }
}

/// What a reviewer's enrolled device key signs (ECDSA P-256 with SHA-256 over these bytes): the canon encoding of
/// (`reins/org-review/v1`, org, subject, id, digest, decision, snapshot, at). `snapshot` is the policy snapshot hash
/// the decision was made against (empty for an admin change); `at` the reviewer's Unix seconds.
#[must_use]
pub fn review_message(
    org: &str,
    subject: ReviewSubject,
    id: &str,
    digest: &str,
    decision: ReviewDecision,
    snapshot: &str,
    at: i64,
) -> Vec<u8> {
    let mut c = Canon::new(contexts::ORG_REVIEW);
    c.str(org).str(subject.as_str()).str(id).str(digest).str(decision.as_str()).str(snapshot).i64(at);
    c.finish()
}

/// One reviewer's signed decision.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewSignature {
    /// The reviewer's user id.
    pub member: String,
    /// The device that signed (its id in the roster), or with `webauthn` the passkey's credential id.
    pub device: String,
    pub decision: ReviewDecision,
    pub at: i64,
    /// DER ECDSA P-256, base64: over [`review_message`], or with `webauthn` over [`webauthn_signed_bytes`].
    pub sig: String,
    /// A signed web approval (O3): the passkey's assertion over [`webauthn_challenge`] of the [`review_message`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub webauthn: Option<WebAuthnProof>,
}

/// What a WebAuthn assertion carries besides its signature, base64url without padding as the browser gave them.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WebAuthnProof {
    pub authenticator_data: String,
    pub client_data_json: String,
}

/// The WebAuthn challenge of a web approval: base64url (no padding) of SHA-256 of the [`review_message`] it stands for,
/// so the passkey's signature binds the org, the request, its digest, the decision, the snapshot and the time.
#[must_use]
pub fn webauthn_challenge(review_message: &[u8]) -> String {
    data_encoding::BASE64URL_NOPAD.encode(&sha256(review_message))
}

/// Authenticator data flags (WebAuthn §6.1).
const FLAG_UP: u8 = 0x01;
const FLAG_UV: u8 = 0x04;
const FLAG_BE: u8 = 0x08;

/// Why a WebAuthn assertion doesn't stand for the approval it claims.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum WebAuthnError {
    #[error("the assertion can't be read")]
    Malformed,
    #[error("not an assertion of this challenge")]
    Challenge,
    #[error("asserted for another origin or relying party")]
    Origin,
    #[error("the user wasn't present and verified")]
    NotVerified,
    #[error("a synced passkey (backup eligible) can't approve")]
    Synced,
}

/// Checks a web approval's assertion (everything but its signature) and returns the bytes its signature covers:
/// `authenticator_data ‖ SHA-256(client_data_json)`.
///
/// - the client data is a `webauthn.get` of exactly `challenge`, from `origin`, not cross-origin;
/// - the authenticator data is for `rp_id`, with user presence and user verification, and **not** backup eligible
///   (a device-bound passkey, 05 §6).
pub fn webauthn_signed_bytes(
    proof: &WebAuthnProof,
    challenge: &str,
    origin: &str,
    rp_id: &str,
) -> Result<Vec<u8>, WebAuthnError> {
    let b64 = data_encoding::BASE64URL_NOPAD;
    let auth =
        b64.decode(proof.authenticator_data.trim_end_matches('=').as_bytes()).map_err(|_| WebAuthnError::Malformed)?;
    let client =
        b64.decode(proof.client_data_json.trim_end_matches('=').as_bytes()).map_err(|_| WebAuthnError::Malformed)?;
    let data: Value = serde_json::from_slice(&client).map_err(|_| WebAuthnError::Malformed)?;
    if data["type"] != "webauthn.get" || data["challenge"].as_str() != Some(challenge) {
        return Err(WebAuthnError::Challenge);
    }
    if data["origin"].as_str() != Some(origin) || data["crossOrigin"] == Value::Bool(true) {
        return Err(WebAuthnError::Origin);
    }
    if auth.len() < 37 {
        return Err(WebAuthnError::Malformed);
    }
    if auth[..32] != sha256(rp_id.as_bytes()) {
        return Err(WebAuthnError::Origin);
    }
    let flags = auth[32];
    if flags & FLAG_UP == 0 || flags & FLAG_UV == 0 {
        return Err(WebAuthnError::NotVerified);
    }
    if flags & FLAG_BE != 0 {
        return Err(WebAuthnError::Synced);
    }
    let mut signed = auth;
    signed.extend_from_slice(&sha256(&client));
    Ok(signed)
}

/// What a member's enrolled phone signs to confirm a web passkey as theirs: the canon encoding of
/// (`reins/org-passkey/v1`, org, credential id, SHA-256 hex of the key's SPKI, registered at, at). It binds the exact
/// key and registration, not just an id the browser chose. It is signed only to confirm: a refusal is an unsigned
/// revoke, so a refusal never carries a signature that could be replayed as a confirmation.
#[must_use]
pub fn passkey_confirm_message(
    org: &str,
    credential_id: &str,
    spki_hash: &str,
    registered_at: i64,
    at: i64,
) -> Vec<u8> {
    let mut c = Canon::new(contexts::ORG_PASSKEY);
    c.str(org).str(credential_id).str(spki_hash).i64(registered_at).i64(at);
    c.finish()
}

/// SHA-256 (hex) of a passkey's SPKI as the server stores it (base64 text, trimmed).
#[must_use]
pub fn passkey_spki_hash(spki_b64: &str) -> String {
    hex32(&sha256(spki_b64.trim().as_bytes()))
}

/// The short code both the console and the phone show for a passkey, so the member confirms the key they just added:
/// the first 8 hex digits of [`passkey_spki_hash`], as `abcd-ef01`.
#[must_use]
pub fn passkey_fingerprint(spki_b64: &str) -> String {
    let h = passkey_spki_hash(spki_b64);
    format!("{}-{}", &h[..4], &h[4..8])
}

/// Whether a review at `level` (`L0`…`L3`, as a trace records it) may be offered in Slack or Teams, as a link to the
/// signed web approval (05 §6: L0–L2 only; L3 always needs the app). Anything else, unknown included, may not.
#[must_use]
pub fn channel_allowed(level: &str) -> bool {
    matches!(level, "L0" | "L1" | "L2")
}

/// What an [`Envelope`] carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvelopeKind {
    /// An org policy snapshot, signed by the org issuer key.
    Policy,
    /// The org's roster of members and enrolled devices, signed by the org issuer key.
    Roster,
    /// A [`DecisionRecord`], countersigned by the decision service key.
    Decision,
    /// An audit [`Checkpoint`], signed by the decision service key.
    Checkpoint,
    /// The head of an audit export (a [`Checkpoint`] of its last event), signed by the decision service key.
    Head,
    /// An executor's [`Receipt`], signed by the executor's key.
    Receipt,
    /// One step of a staged policy rollout ([`crate::rollout::Rollout`]), pre-signed by the org issuer key (O4).
    Rollout,
}

impl EnvelopeKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Policy => "policy",
            Self::Roster => "roster",
            Self::Decision => "decision",
            Self::Checkpoint => "checkpoint",
            Self::Head => "head",
            Self::Receipt => "receipt",
            Self::Rollout => "rollout",
        }
    }
}

/// A body signed with an Ed25519 key. `body` is canonical JSON text and is signed byte for byte.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Envelope {
    pub kind: EnvelopeKind,
    pub org: String,
    pub body: String,
    /// Which key signed (the signer's key id).
    pub key_id: String,
    /// Ed25519 over [`envelope_message`], base64url without padding.
    pub sig: String,
}

impl Envelope {
    /// The body parsed as `T`.
    pub fn parse<T: serde::de::DeserializeOwned>(&self) -> Result<T, serde_json::Error> {
        serde_json::from_str(&self.body)
    }

    /// SHA-256 (hex) of the body text: a policy snapshot's hash is its envelope body's.
    #[must_use]
    pub fn body_hash(&self) -> String {
        hex32(&sha256(self.body.as_bytes()))
    }
}

/// What an [`Envelope`]'s key signs: (`reins/org-envelope/v1`, kind, org, key id, body).
#[must_use]
pub fn envelope_message(kind: EnvelopeKind, org: &str, key_id: &str, body: &str) -> Vec<u8> {
    let mut c = Canon::new(contexts::ORG_ENVELOPE);
    c.str(kind.as_str()).str(org).str(key_id).str(body);
    c.finish()
}

/// The org's members and their enrolled devices, as of `issued_at` (signed by the issuer key, [`EnvelopeKind::Roster`]).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Roster {
    pub org: String,
    pub version: u64,
    pub issued_at: i64,
    pub members: Vec<RosterMember>,
    /// Group id → its backup group id (O3's escalation, checked by executors).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub backups: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RosterMember {
    /// The user id.
    pub user: String,
    pub email: String,
    /// Group ids.
    pub groups: Vec<String>,
    pub roles: Vec<Role>,
    pub devices: Vec<RosterDevice>,
    /// Their manager's user id, from the directory (or a Security admin on the Team tier).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manager: Option<String>,
    /// Web passkeys they registered and confirmed on their enrolled phone (signed web approvals).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub passkeys: Vec<RosterPasskey>,
}

/// A device-bound passkey a member may approve with on the web (L0–L2).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RosterPasskey {
    /// The WebAuthn credential id, base64url.
    pub credential_id: String,
    /// P-256 SubjectPublicKeyInfo DER, base64.
    pub key: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RosterDevice {
    pub device: String,
    /// P-256 SubjectPublicKeyInfo DER, base64.
    pub key: String,
    /// It passes the org's posture rule now (06 §6): only such a device's signature counts.
    pub may_decide: bool,
}

impl Roster {
    #[must_use]
    pub fn member(&self, user: &str) -> Option<&RosterMember> {
        self.members.iter().find(|m| m.user == user)
    }
}

/// How a decision ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// Policy permitted it without a review.
    Auto,
    /// Every review obligation was met.
    Approved,
    Denied,
}

impl Verdict {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Approved => "approved",
            Self::Denied => "denied",
        }
    }
}

/// A decision as the decision service countersigns it ([`EnvelopeKind::Decision`]); what an executor acts on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionRecord {
    pub v: u32,
    pub org: String,
    /// The request id (`or_…`).
    pub request: String,
    /// [`request_digest`].
    pub digest: String,
    /// The requester's user id.
    pub member: String,
    pub connection: String,
    pub tool: String,
    pub verdict: Verdict,
    /// The org policy snapshot hash the decision was made against.
    pub snapshot: String,
    /// The member's own policy's hash, when they set one.
    #[serde(default)]
    pub member_policy: Option<String>,
    /// The obligation ids that applied.
    #[serde(default)]
    pub obligations: Vec<String>,
    #[serde(default)]
    pub approvals: Vec<ReviewSignature>,
    /// For a lease, exactly what may be minted and for whom: an executor mints nothing that differs.
    #[serde(default)]
    pub lease: Option<LeaseBinding>,
    /// Obligation id → the members its relationships, on-call schedules and escalations resolved to (O3), from
    /// verified sources only. Executors check what they can against the roster and take the rest on this
    /// countersignature.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub resolved: BTreeMap<String, Vec<String>>,
    /// The agent's reason, when it gave one (a `require_reason` permit is judged with it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// When the request was evaluated (the time rules were judged at).
    pub evaluated_at: i64,
    /// The rollout step (rollout id, seq) whose cohorts chose `snapshot`, when a staged rollout was under way (O4).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rollout: Option<(String, u32)>,
    pub decided_at: i64,
    /// Not executed after this, it expires (03 §6: 24 h for executor-run actions).
    pub valid_until: i64,
}

/// How an execution ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReceiptStatus {
    Succeeded,
    Failed,
    /// The executor refused to act on it (08 R5): nothing was called.
    Refused,
    /// The world changed since the decision (03 §6): nothing was called; ask again.
    Stale,
}

impl ReceiptStatus {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Refused => "refused",
            Self::Stale => "stale",
        }
    }
}

/// An executor's account of one job ([`EnvelopeKind::Receipt`], signed by the executor).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Receipt {
    pub org: String,
    pub request: String,
    pub digest: String,
    pub executor: String,
    pub status: ReceiptStatus,
    /// The minimal result (an id, a status), or the read's result for the agent.
    #[serde(default)]
    pub result: Value,
    #[serde(default)]
    pub reason: String,
    pub at: i64,
}

/// What an executor is asked to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    /// Run the tool call.
    Execute,
    /// Mint a short-lived credential and seal it to the requester.
    Lease,
}

/// Read or push access to a repository.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LeaseAccess {
    Read,
    Push,
}

impl LeaseAccess {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Push => "push",
        }
    }
}

/// The tool name a lease request is evaluated as.
pub const LEASE_TOOL: &str = "git_lease";

/// What a decision allows a lease to be: inside the countersigned [`DecisionRecord`], so a server can't change the
/// repository, the access, the recipient or the lifetime of a lease while reusing a valid decision.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeaseBinding {
    pub lease: String,
    pub repo: String,
    pub access: LeaseAccess,
    /// SHA-256 (hex) of the box key's text as the requester sent it.
    pub box_key_hash: String,
    pub expires_at: i64,
}

impl LeaseBinding {
    /// The binding of `job`.
    #[must_use]
    pub fn of(job: &LeaseJob) -> Self {
        Self {
            lease: job.lease.clone(),
            repo: job.repo.clone(),
            access: job.access,
            box_key_hash: hex32(&sha256(job.box_key.as_bytes())),
            expires_at: job.expires_at,
        }
    }

    /// Whether `job` is exactly this lease, and the request's arguments (what the digest covers) name the same
    /// repository, access and box key.
    #[must_use]
    pub fn matches(&self, job: &LeaseJob, args: &Value) -> bool {
        let arg = |k: &str| args.get(k).and_then(Value::as_str);
        Self::of(job) == *self
            && arg("repo") == Some(job.repo.as_str())
            && arg("access") == Some(job.access.as_str())
            && arg("box_key") == Some(job.box_key.as_str())
    }
}

/// The arguments a lease request is evaluated (and digested) with.
#[must_use]
pub fn lease_args(repo: &str, access: LeaseAccess, box_key: &str) -> Value {
    serde_json::json!({"repo": repo, "access": access.as_str(), "box_key": box_key})
}

/// A lease to mint.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeaseJob {
    pub lease: String,
    /// `owner/name` on GitHub.
    pub repo: String,
    pub access: LeaseAccess,
    /// The requester's X25519 public key, base64url: the credential is sealed to it.
    pub box_key: String,
    pub expires_at: i64,
}

/// A member's own stricter policy, signed by their enrolled device.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedMemberPolicy {
    /// Canonical JSON of the member policy.
    pub body: String,
    /// Strictly greater than the version it replaces, so an old signed policy can't be replayed over a newer one.
    pub version: u64,
    /// When the phone signed it (Unix seconds).
    pub at: i64,
    pub device: String,
    /// DER ECDSA P-256 over [`member_policy_message`], base64.
    pub sig: String,
}

/// What a member's device signs to set their own policy: (`reins/org-member-policy/v1`, org, member, version, at,
/// body).
#[must_use]
pub fn member_policy_message(org: &str, member: &str, version: u64, at: i64, body: &str) -> Vec<u8> {
    let mut c = Canon::new(contexts::ORG_MEMBER_POLICY);
    c.str(org).str(member).u64(version).i64(at).str(body);
    c.finish()
}

/// One job an executor claimed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Job {
    pub request: String,
    pub kind: JobKind,
    pub tool: String,
    pub args: Value,
    /// The countersigned [`DecisionRecord`].
    pub decision: Envelope,
    /// The hash of the policy now active (revalidation, 03 §6).
    pub current_policy: String,
    #[serde(default)]
    pub member_policy: Option<SignedMemberPolicy>,
    #[serde(default)]
    pub lease: Option<LeaseJob>,
}

/// `POST /reins/api/org/leases`: a Managed desktop daemon asks for a repository credential.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeaseRequest {
    pub repo: String,
    pub access: LeaseAccess,
    /// X25519 public key (base64url) the credential is sealed to.
    pub box_key: String,
}

/// A lease as its requester sees it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeaseView {
    pub id: String,
    /// `pending`, `issued`, `denied`, `revoked` or `expired`.
    pub state: String,
    /// The credential sealed to the requester's box key, once issued.
    #[serde(default)]
    pub sealed: Option<String>,
    pub expires_at: i64,
    /// Why it was denied, in the issuer's words.
    #[serde(default)]
    pub explain: String,
}

/// What an executor signs on each call to the server (Ed25519): (`reins/org-exec/v1`, org, executor, method, path,
/// at, SHA-256 of the body).
#[must_use]
pub fn exec_request_message(org: &str, executor: &str, method: &str, path: &str, at: i64, body: &[u8]) -> Vec<u8> {
    let mut c = Canon::new(contexts::ORG_EXEC);
    c.str(org).str(executor).str(method).str(path).i64(at).key(&sha256(body));
    c.finish()
}

/// The chain head at one event: what a checkpoint and an export head sign.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checkpoint {
    pub org: String,
    pub seq: i64,
    pub hash: String,
    pub at: i64,
}

/// One audit event as exported (the stored row, field for field).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditEvent {
    pub org: String,
    pub seq: i64,
    pub at: i64,
    pub kind: String,
    #[serde(default)]
    pub actor: Option<String>,
    #[serde(default)]
    pub subject: Option<String>,
    /// The stored detail text (JSON), byte for byte.
    pub detail: String,
    pub prev_hash: String,
    pub hash: String,
}

impl AuditEvent {
    /// The hash this event should carry ([`crate::org::event_hash`]).
    #[must_use]
    pub fn expected_hash(&self) -> String {
        crate::org::event_hash(
            &crate::org::EventFields {
                org: &self.org,
                seq: self.seq,
                at: self.at,
                kind: &self.kind,
                actor: self.actor.as_deref(),
                subject: self.subject.as_deref(),
                detail: &self.detail,
            },
            &self.prev_hash,
        )
    }
}

/// One line of an audit export (JSON Lines).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum ExportLine {
    Event(AuditEvent),
    /// A signed [`Checkpoint`] ([`EnvelopeKind::Checkpoint`]).
    Checkpoint(Envelope),
    /// The export's signed head ([`EnvelopeKind::Head`]): its last line.
    Head(Envelope),
}

/// A request under review, as a reviewer's phone receives it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestBody {
    pub org: String,
    pub member: String,
    pub connection: String,
    pub tool: String,
    pub args: Value,
}

/// `GET /reins/api/org/reviews`: one review waiting for this member.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewItem {
    pub id: String,
    pub subject: ReviewSubject,
    pub org_name: String,
    /// The requester's or proposer's email.
    pub requester: String,
    /// The agent connection's label (requests).
    #[serde(default)]
    pub connection: String,
    pub title: String,
    /// The request itself (requests): the phone recomputes the digest from it.
    #[serde(default)]
    pub request: Option<RequestBody>,
    /// The change (admin changes), with its stored text for [`change_digest`].
    #[serde(default)]
    pub action: Option<AdminAction>,
    #[serde(default)]
    pub action_json: Option<String>,
    pub digest: String,
    /// The policy snapshot hash (requests); empty for changes.
    #[serde(default)]
    pub snapshot: String,
    /// Who must approve, in short words ("Security", "You").
    pub requires: String,
    #[serde(default)]
    pub level: String,
    #[serde(default)]
    pub zone: String,
    pub expires_at: i64,
    /// The viewer's own: their request, or their change to confirm with [`ReviewDecision::Propose`].
    #[serde(default)]
    pub own: bool,
    /// One line per obligation still waiting ("On-call: payments", "2 of Finance", "Code owners").
    #[serde(default)]
    pub needs: Vec<String>,
    /// Approvals so far, as "1 of 2" (empty when one approval decides).
    #[serde(default)]
    pub progress: String,
    /// Where an escalation took it ("Escalated to Security").
    #[serde(default)]
    pub step: Option<String>,
    /// Breaks quiet hours.
    #[serde(default)]
    pub urgent: bool,
    /// The agent's reason, when it gave one.
    #[serde(default)]
    pub reason: Option<String>,
}

/// `POST /reins/api/org/reviews/<id>`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewAnswer {
    pub decision: ReviewDecision,
    pub at: i64,
    /// DER ECDSA P-256 over [`review_message`], base64.
    pub signature: String,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn canonical_json_sorts_keys_at_every_depth() {
        let v = json!({"b": 1, "a": {"d": [{"z": 0, "y": 1}], "c": "é\n"}});
        assert_eq!(canonical_json(&v), r#"{"a":{"c":"é\n","d":[{"y":1,"z":0}]},"b":1}"#);
        assert_eq!(canonical_json(&json!(null)), "null");
        assert_eq!(canonical_json(&json!([])), "[]");
    }

    #[test]
    fn request_digests_bind_every_field() {
        let args = json!({"repo": "acme/api", "title": "x"});
        let base = request_digest("o", "m", "c", "t", &args);
        assert_eq!(base.len(), 64);
        assert_ne!(base, request_digest("o2", "m", "c", "t", &args));
        assert_ne!(base, request_digest("o", "m2", "c", "t", &args));
        assert_ne!(base, request_digest("o", "m", "c2", "t", &args));
        assert_ne!(base, request_digest("o", "m", "c", "t2", &args));
        assert_ne!(base, request_digest("o", "m", "c", "t", &json!({"repo": "acme/api", "title": "y"})));
        let reordered: Value = serde_json::from_str(r#"{"title":"x","repo":"acme/api"}"#).unwrap();
        assert_eq!(base, request_digest("o", "m", "c", "t", &reordered));
        // Field boundaries are length-prefixed: shifting a character between fields changes the digest.
        assert_ne!(request_digest("ab", "c", "d", "t", &args), request_digest("a", "bc", "d", "t", &args));
    }

    #[test]
    fn review_messages_differ_by_decision_subject_and_snapshot() {
        let m = |s, d, snap: &str| review_message("o", s, "id", "dg", d, snap, 5);
        let base = m(ReviewSubject::Request, ReviewDecision::Approve, "s1");
        assert_ne!(base, m(ReviewSubject::Change, ReviewDecision::Approve, "s1"));
        assert_ne!(base, m(ReviewSubject::Request, ReviewDecision::Deny, "s1"));
        assert_ne!(base, m(ReviewSubject::Request, ReviewDecision::Approve, "s2"));
        assert_ne!(base, review_message("o", ReviewSubject::Request, "id", "dg", ReviewDecision::Approve, "s1", 6));
        assert_eq!(ReviewDecision::parse("propose"), Some(ReviewDecision::Propose));
        assert_eq!(ReviewDecision::parse("maybe"), None);
    }

    #[test]
    fn envelopes_bind_kind_org_and_key() {
        let base = envelope_message(EnvelopeKind::Policy, "o", "k", "{}");
        assert_ne!(base, envelope_message(EnvelopeKind::Roster, "o", "k", "{}"));
        assert_ne!(base, envelope_message(EnvelopeKind::Policy, "o2", "k", "{}"));
        assert_ne!(base, envelope_message(EnvelopeKind::Policy, "o", "k2", "{}"));
        assert_ne!(base, envelope_message(EnvelopeKind::Policy, "o", "k", "{ }"));
        let env = Envelope {
            kind: EnvelopeKind::Policy,
            org: "o".into(),
            body: "{}".into(),
            key_id: "k".into(),
            sig: String::new(),
        };
        assert_eq!(env.body_hash(), hex32(&sha256(b"{}")));
    }

    #[test]
    fn export_lines_round_trip() {
        let line = ExportLine::Event(AuditEvent {
            org: "o".into(),
            seq: 1,
            at: 2,
            kind: "org.created".into(),
            actor: None,
            subject: Some("s".into()),
            detail: "{}".into(),
            prev_hash: crate::org::GENESIS_HASH.into(),
            hash: "h".into(),
        });
        let text = serde_json::to_string(&line).unwrap();
        assert!(text.starts_with(r#"{"t":"event""#), "{text}");
        assert_eq!(serde_json::from_str::<ExportLine>(&text).unwrap(), line);
    }

    #[test]
    fn statuses_and_kinds_travel_in_snake_case() {
        assert_eq!(serde_json::to_value(ReceiptStatus::Succeeded).unwrap(), json!("succeeded"));
        assert_eq!(serde_json::to_value(JobKind::Lease).unwrap(), json!("lease"));
        assert_eq!(serde_json::to_value(EnvelopeKind::Head).unwrap(), json!("head"));
        assert_eq!(serde_json::to_value(LeaseAccess::Push).unwrap(), json!("push"));
    }

    #[test]
    fn a_tampered_lease_does_not_match_its_decision() {
        let job = LeaseJob {
            lease: "l1".into(),
            repo: "acme/api".into(),
            access: LeaseAccess::Read,
            box_key: "KEY".into(),
            expires_at: 100,
        };
        let args = lease_args("acme/api", LeaseAccess::Read, "KEY");
        let bound = LeaseBinding::of(&job);
        assert!(bound.matches(&job, &args));
        let tamper = |f: &dyn Fn(&mut LeaseJob)| {
            let mut j = job.clone();
            f(&mut j);
            j
        };
        for j in [
            tamper(&|j| j.repo = "acme/prod".into()),
            tamper(&|j| j.access = LeaseAccess::Push),
            tamper(&|j| j.box_key = "ATTACKER".into()),
            tamper(&|j| j.expires_at = 10_000),
            tamper(&|j| j.lease = "l2".into()),
        ] {
            assert!(!bound.matches(&j, &args), "{j:?}");
        }
        // The digested arguments must agree with the job too.
        assert!(!bound.matches(&job, &lease_args("acme/prod", LeaseAccess::Read, "KEY")));
        assert!(!bound.matches(&job, &lease_args("acme/api", LeaseAccess::Push, "KEY")));
        assert!(!bound.matches(&job, &lease_args("acme/api", LeaseAccess::Read, "OTHER")));
    }

    #[test]
    fn change_and_exec_messages_are_distinct_contexts() {
        assert_ne!(change_digest("o", "c", "{}"), change_digest("o", "c2", "{}"));
        let a = exec_request_message("o", "e", "POST", "/p", 1, b"x");
        assert_ne!(a, exec_request_message("o", "e", "POST", "/p", 1, b"y"));
        assert_ne!(a, exec_request_message("o", "e", "GET", "/p", 1, b"x"));
        assert_ne!(member_policy_message("o", "m", 1, 5, "{}"), member_policy_message("o", "m", 2, 5, "{}"));
        assert_ne!(member_policy_message("o", "m", 1, 5, "{}"), member_policy_message("o", "m", 1, 6, "{}"));
        assert!(ids_ok(&["a", "b"]));
        assert!(!ids_ok(&["", "b"]));
        assert!(!ids_ok(&["a\n"]));
    }

    #[test]
    fn old_records_and_rosters_still_parse() {
        // O2's shapes, without O3's fields.
        let record: DecisionRecord = serde_json::from_value(json!({
            "v": 1, "org": "o", "request": "or_1", "digest": "d", "member": "m", "connection": "c", "tool": "t",
            "verdict": "approved", "snapshot": "s", "obligations": ["r"],
            "approvals": [{"member": "k", "device": "p", "decision": "approve", "at": 5, "sig": "x"}],
            "evaluated_at": 1, "decided_at": 2, "valid_until": 3
        }))
        .unwrap();
        assert!(record.resolved.is_empty() && record.reason.is_none() && record.approvals[0].webauthn.is_none());
        // Re-serialized, nothing new appears (a countersigned body stays what it was).
        let text = serde_json::to_string(&record).unwrap();
        assert!(!text.contains("resolved") && !text.contains("webauthn") && !text.contains("reason"), "{text}");
        let roster: Roster = serde_json::from_value(json!({
            "org": "o", "version": 1, "issued_at": 1,
            "members": [{"user": "u", "email": "u@x", "groups": [], "roles": [], "devices": []}]
        }))
        .unwrap();
        assert!(roster.members[0].manager.is_none() && roster.members[0].passkeys.is_empty());
        let text = serde_json::to_string(&roster).unwrap();
        assert!(!text.contains("manager") && !text.contains("passkeys") && !text.contains("backups"), "{text}");
        let item: ReviewItem = serde_json::from_value(json!({
            "id": "or_1", "subject": "request", "org_name": "Acme", "requester": "lee@acme.com", "title": "x",
            "digest": "d", "requires": "Security", "expires_at": 9
        }))
        .unwrap();
        assert!(item.needs.is_empty() && item.step.is_none() && !item.urgent);
    }

    #[test]
    fn webauthn_challenge_is_the_review_message_hash() {
        let msg = review_message("o", ReviewSubject::Request, "or_1", "dg", ReviewDecision::Approve, "s", 5);
        let c = webauthn_challenge(&msg);
        assert_eq!(data_encoding::BASE64URL_NOPAD.decode(c.as_bytes()).unwrap(), sha256(&msg));
        let deny = review_message("o", ReviewSubject::Request, "or_1", "dg", ReviewDecision::Deny, "s", 5);
        assert_ne!(webauthn_challenge(&deny), c);
        assert_ne!(passkey_confirm_message("o", "cred", "h", 1, 2), passkey_confirm_message("o", "cred2", "h", 1, 2));
        assert_ne!(passkey_confirm_message("o", "cred", "h", 1, 2), passkey_confirm_message("o", "cred", "h2", 1, 2));
        assert_ne!(passkey_confirm_message("o", "cred", "h", 1, 2), passkey_confirm_message("o", "cred", "h", 3, 2));
        assert_eq!(passkey_fingerprint("AAAA").len(), 9);
        assert_ne!(passkey_fingerprint("AAAA"), passkey_fingerprint("AAAB"));
        assert!(channel_allowed("L0") && channel_allowed("L2"));
        assert!(!channel_allowed("L3") && !channel_allowed("") && !channel_allowed("l2"));
    }

    fn proof(flags: u8, rp: &str, ty: &str, challenge: &str, origin: &str) -> WebAuthnProof {
        let b64 = data_encoding::BASE64URL_NOPAD;
        let mut auth = sha256(rp.as_bytes()).to_vec();
        auth.push(flags);
        auth.extend_from_slice(&[0, 0, 0, 7]);
        let client = json!({"type": ty, "challenge": challenge, "origin": origin, "crossOrigin": false});
        WebAuthnProof {
            authenticator_data: b64.encode(&auth),
            client_data_json: b64.encode(client.to_string().as_bytes()),
        }
    }

    #[test]
    fn webauthn_assertions_bind_challenge_origin_and_device() {
        let (o, rp) = ("https://reins.example", "reins.example");
        let ok = proof(0x05, rp, "webauthn.get", "ch", o);
        let signed = webauthn_signed_bytes(&ok, "ch", o, rp).unwrap();
        assert_eq!(signed.len(), 37 + 32);
        assert_eq!(webauthn_signed_bytes(&ok, "other", o, rp), Err(WebAuthnError::Challenge));
        assert_eq!(webauthn_signed_bytes(&ok, "ch", "https://evil.example", rp), Err(WebAuthnError::Origin));
        assert_eq!(webauthn_signed_bytes(&ok, "ch", o, "evil.example"), Err(WebAuthnError::Origin));
        let create = proof(0x05, rp, "webauthn.create", "ch", o);
        assert_eq!(webauthn_signed_bytes(&create, "ch", o, rp), Err(WebAuthnError::Challenge));
        // Presence without verification, and a synced (backup eligible) passkey.
        assert_eq!(
            webauthn_signed_bytes(&proof(0x01, rp, "webauthn.get", "ch", o), "ch", o, rp),
            Err(WebAuthnError::NotVerified)
        );
        assert_eq!(
            webauthn_signed_bytes(&proof(0x0d, rp, "webauthn.get", "ch", o), "ch", o, rp),
            Err(WebAuthnError::Synced)
        );
        assert_eq!(
            webauthn_signed_bytes(&proof(0x1d, rp, "webauthn.get", "ch", o), "ch", o, rp),
            Err(WebAuthnError::Synced)
        );
    }
}
