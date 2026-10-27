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
    /// The device that signed (its id in the roster).
    pub device: String,
    pub decision: ReviewDecision,
    pub at: i64,
    /// DER ECDSA P-256, base64.
    pub sig: String,
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
}
