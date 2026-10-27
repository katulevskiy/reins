//! Independent verification of every job (08 R5): "Executors re-evaluate every request independently with the same
//! engine library, against the policy snapshot signed by the org issuer key … An executor refuses if the decision
//! service's verdict and its own differ; the snapshot isn't signed by the issuer; a review-required request lacks valid
//! reviewer device signatures."
//!
//! [`verify_job`] trusts nothing the server says that a pinned key didn't sign: the decision (countersigned by the
//! decision service), the policy snapshots and the roster (signed by the org issuer), the member's own policy and the
//! reviews (signed by enrolled devices the roster lists). Then it judges the request itself.

use reins_audit::verify_envelope;
use reins_policy::effects::Level;
use reins_policy::org::{
    Approval, ManagedOutcome, ManagedRequest, ManagedVerdict, MemberPolicy, Met, OrgPolicy, ResolvedSets, RosterFacts,
    evaluate_managed, obligations_met_resolved,
};
use reins_proto::managed::{
    DecisionRecord, Envelope, EnvelopeKind, Job, JobKind, LEASE_TOOL, ReviewDecision, ReviewSignature, ReviewSubject,
    Roster, RosterMember, Verdict, member_policy_message, request_digest, review_message, webauthn_challenge,
    webauthn_signed_bytes,
};

use crate::config::{MAX_ROSTER_AGE_SECS, Pins};

/// Why an executor won't act on a job. Nothing is called upstream after any of these.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Refusal {
    #[error("the decision is not countersigned by the pinned decision-service key")]
    BadCountersig,
    #[error("the decision is for another organization")]
    WrongOrg,
    #[error("the policy snapshot is not signed by the pinned issuer key")]
    UnsignedPolicy,
    #[error("the policy snapshot is not the one the decision names")]
    PolicyMismatch,
    #[error("the roster is not signed by the pinned issuer key")]
    UnsignedRoster,
    #[error("the roster is too old")]
    StaleRoster,
    #[error("the job is not the request that was decided (digest mismatch)")]
    DigestMismatch,
    #[error("the decision service said {theirs}; this executor's evaluation says {ours}")]
    VerdictDiffers {
        ours: String,
        theirs: String,
    },
    #[error("reviews still missing: {}", .0.join(", "))]
    MissingApprovals(Vec<String>),
    #[error("the approval of {member} does not verify")]
    BadApproval {
        member: String,
    },
    #[error("the decision expired")]
    Expired,
    #[error("the member's policy is not signed by one of their devices")]
    UnsignedMemberPolicy,
    #[error("the policy changed since the decision: {0}")]
    Stale(String),
}

impl Refusal {
    /// The words a receipt carries.
    #[must_use]
    pub fn reason(&self) -> String {
        self.to_string()
    }

    /// `stale` (the world changed: ask again) rather than `refused`.
    #[must_use]
    pub fn is_stale(&self) -> bool {
        matches!(self, Self::Stale(_))
    }
}

/// A job that passed every check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verified {
    /// The countersigned decision.
    pub record: DecisionRecord,
    /// The requester's email, from the signed roster (whom a Google Workspace action impersonates).
    pub member_email: String,
}

/// [`verify_job_with`] with the default roster age ([`MAX_ROSTER_AGE_SECS`]).
pub fn verify_job(
    job: &Job,
    pins: &Pins,
    policy: &Envelope,
    current: &Envelope,
    roster: &Envelope,
    now: i64,
) -> Result<Verified, Refusal> {
    verify_job_with(job, pins, policy, current, roster, now, MAX_ROSTER_AGE_SECS)
}

/// Verifies `job` against the policy snapshot it was decided with (`policy`), the policy active now (`current`) and the
/// roster, in this order:
///
/// 1. the decision is countersigned by the pinned decision-service key, for the pinned org;
/// 2. it hasn't expired;
/// 3. both policies are signed by the pinned issuer key and are the snapshots the decision and the job name;
/// 4. the roster is signed by the issuer, for this org, and at most `max_roster_age` old;
/// 5. what would run is exactly the countersigned digest (and for a lease, exactly the countersigned lease);
/// 6. the member's own policy, when there is one, is signed by one of their devices and is the one decided with;
/// 7. this executor's own evaluation agrees with the decision, and every review it requires is signed by an eligible,
///    posture-passing device;
/// 8. the policy active now still allows it (otherwise [`Refusal::Stale`], 03 §6).
#[allow(clippy::too_many_arguments)]
pub fn verify_job_with(
    job: &Job,
    pins: &Pins,
    policy: &Envelope,
    current: &Envelope,
    roster: &Envelope,
    now: i64,
    max_roster_age: i64,
) -> Result<Verified, Refusal> {
    // 1. The countersignature.
    let decision = &job.decision;
    if decision.kind != EnvelopeKind::Decision || !verify_envelope(decision, std::slice::from_ref(&pins.decision)) {
        return Err(Refusal::BadCountersig);
    }
    let record: DecisionRecord = decision.parse().map_err(|_| Refusal::BadCountersig)?;
    if decision.org != pins.org || record.org != pins.org {
        return Err(Refusal::WrongOrg);
    }
    // 2. Validity.
    if now > record.valid_until {
        return Err(Refusal::Expired);
    }
    // 3. The policies.
    let decided_with = signed_policy(policy, pins, &record.snapshot)?;
    let active = signed_policy(current, pins, &job.current_policy)?;
    // 4. The roster.
    let roster = signed_roster(roster, pins, now, max_roster_age)?;
    // 5. The digest, and a lease's binding.
    let digest = request_digest(&record.org, &record.member, &record.connection, &job.tool, &job.args);
    if digest != record.digest || job.tool != record.tool || job.request != record.request {
        return Err(Refusal::DigestMismatch);
    }
    check_lease(job, &record)?;
    // 6. The member's own policy.
    let requester = roster.member(&record.member).ok_or(Refusal::UnsignedRoster)?;
    let member_policy = member_policy(job, &record, requester)?;
    // 7. Re-evaluation as decided.
    let request = |at: i64| ManagedRequest {
        org: &record.org,
        member: &record.member,
        connection: &record.connection,
        tool: &job.tool,
        args: &job.args,
        now: at,
        reason: record.reason.as_deref(),
    };
    let ours = evaluate_managed(&request(record.evaluated_at), &decided_with, member_policy.as_ref());
    let differs = |ours: &str| Refusal::VerdictDiffers {
        ours: ours.to_owned(),
        theirs: record.verdict.as_str().to_owned(),
    };
    match (ours.outcome, record.verdict) {
        (ManagedOutcome::Auto, Verdict::Auto) => {}
        (ManagedOutcome::Review, Verdict::Approved) => {
            let approvals = verified_approvals(&record, &roster, pins, ours.evaluation.level)?;
            match weigh(&ours, &record, &roster, &approvals) {
                Met::Yes => {}
                Met::Waiting(ids) => return Err(Refusal::MissingApprovals(ids)),
                Met::Denied => return Err(differs("denied")),
            }
        }
        (o, _) => return Err(differs(o.as_str())),
    }
    // 8. The world now.
    let now_verdict = evaluate_managed(&request(now), &active, member_policy.as_ref());
    match now_verdict.outcome {
        ManagedOutcome::Auto => {}
        ManagedOutcome::Deny => {
            return Err(Refusal::Stale(format!("now denied ({})", why(&now_verdict))));
        }
        ManagedOutcome::Review => {
            let approvals = verified_approvals(&record, &roster, pins, now_verdict.evaluation.level)?;
            if weigh(&now_verdict, &record, &roster, &approvals) != Met::Yes {
                return Err(Refusal::Stale(format!("now needs a review ({})", why(&now_verdict))));
            }
        }
    }
    Ok(Verified {
        member_email: requester.email.clone(),
        record,
    })
}

/// What made a verdict: the deciding rule's words and the obligations' reasons.
fn why(v: &ManagedVerdict) -> String {
    let mut parts: Vec<String> = v.obligations.iter().flat_map(|o| o.why.iter().cloned()).collect();
    if !v.explain.is_empty() {
        parts.insert(0, v.explain.clone());
    }
    parts.join("; ")
}

fn signed_policy(env: &Envelope, pins: &Pins, expected: &str) -> Result<OrgPolicy, Refusal> {
    if env.kind != EnvelopeKind::Policy
        || env.org != pins.org
        || !verify_envelope(env, std::slice::from_ref(&pins.issuer))
    {
        return Err(Refusal::UnsignedPolicy);
    }
    let policy: OrgPolicy = env.parse().map_err(|_| Refusal::UnsignedPolicy)?;
    if policy.org != pins.org {
        return Err(Refusal::UnsignedPolicy);
    }
    if policy.hash() != expected {
        return Err(Refusal::PolicyMismatch);
    }
    Ok(policy)
}

fn signed_roster(env: &Envelope, pins: &Pins, now: i64, max_age: i64) -> Result<Roster, Refusal> {
    if env.kind != EnvelopeKind::Roster
        || env.org != pins.org
        || !verify_envelope(env, std::slice::from_ref(&pins.issuer))
    {
        return Err(Refusal::UnsignedRoster);
    }
    let roster: Roster = env.parse().map_err(|_| Refusal::UnsignedRoster)?;
    if roster.org != pins.org {
        return Err(Refusal::UnsignedRoster);
    }
    if now.saturating_sub(roster.issued_at) > max_age {
        return Err(Refusal::StaleRoster);
    }
    Ok(roster)
}

/// A lease job mints exactly the countersigned lease; an execution is never a lease.
fn check_lease(job: &Job, record: &DecisionRecord) -> Result<(), Refusal> {
    match job.kind {
        JobKind::Lease => {
            let ok = job.tool == LEASE_TOOL
                && matches!((&record.lease, &job.lease), (Some(bound), Some(lease)) if bound.matches(lease, &job.args));
            if ok {
                Ok(())
            } else {
                Err(Refusal::DigestMismatch)
            }
        }
        JobKind::Execute => {
            if job.tool == LEASE_TOOL || record.lease.is_some() || job.lease.is_some() {
                Err(Refusal::DigestMismatch)
            } else {
                Ok(())
            }
        }
    }
}

/// The member's own policy: present exactly when the decision names one, signed by one of their decision-capable
/// devices, and the one the decision names.
fn member_policy(
    job: &Job,
    record: &DecisionRecord,
    requester: &RosterMember,
) -> Result<Option<MemberPolicy>, Refusal> {
    let (signed, expected) = match (&job.member_policy, &record.member_policy) {
        (None, None) => return Ok(None),
        (Some(s), Some(h)) => (s, h),
        _ => return Err(Refusal::UnsignedMemberPolicy),
    };
    let msg = member_policy_message(&record.org, &record.member, signed.version, signed.at, &signed.body);
    let signed_ok = requester
        .devices
        .iter()
        .any(|d| d.device == signed.device && d.may_decide && p256_verify(&d.key, &signed.sig, &msg));
    if !signed_ok {
        return Err(Refusal::UnsignedMemberPolicy);
    }
    let policy: MemberPolicy = serde_json::from_str(&signed.body).map_err(|_| Refusal::UnsignedMemberPolicy)?;
    if &policy.hash() != expected {
        return Err(Refusal::UnsignedMemberPolicy);
    }
    Ok(Some(policy))
}

/// One verified review: who, and whether they approved.
struct Reviewed<'a> {
    member: &'a RosterMember,
    approve: bool,
}

/// Whether a signed web approval (O3) stands: a passkey the roster lists for this member, asserted at the pinned
/// server origin over the review message's challenge, user-verified and device-bound, its signature good, and the
/// request not L3 by this executor's own evaluation (05 §6: L3 always needs the app).
fn web_approval_ok(a: &ReviewSignature, member: &RosterMember, msg: &[u8], pins: &Pins, level: Level) -> bool {
    let (Some(proof), Some(origin)) = (&a.webauthn, pins.web_origin.as_deref()) else {
        return false;
    };
    if level == Level::L3 {
        return false;
    }
    let Some(rp_id) = reqwest::Url::parse(origin).ok().and_then(|u| u.host_str().map(str::to_owned)) else {
        return false;
    };
    let Some(key) = member.passkeys.iter().find(|p| p.credential_id == a.device) else {
        return false;
    };
    match webauthn_signed_bytes(proof, &webauthn_challenge(msg), origin, &rp_id) {
        Ok(signed) => p256_verify(&key.key, &a.sig, &signed),
        Err(_) => false,
    }
}

/// Every review in the decision, each signed by a decision-capable device of its reviewer over 06 §3's message, or by
/// a passkey of theirs the roster lists (a signed web approval, never for L3).
fn verified_approvals<'a>(
    record: &DecisionRecord,
    roster: &'a Roster,
    pins: &Pins,
    level: Level,
) -> Result<Vec<Reviewed<'a>>, Refusal> {
    record
        .approvals
        .iter()
        .map(|a| {
            let bad = || Refusal::BadApproval {
                member: a.member.clone(),
            };
            let approve = match a.decision {
                ReviewDecision::Approve => true,
                ReviewDecision::Deny => false,
                ReviewDecision::Propose => return Err(bad()),
            };
            let member = roster.member(&a.member).ok_or_else(bad)?;
            let msg = review_message(
                &record.org,
                ReviewSubject::Request,
                &record.request,
                &record.digest,
                a.decision,
                &record.snapshot,
                a.at,
            );
            let ok = if a.webauthn.is_some() {
                web_approval_ok(a, member, &msg, pins, level)
            } else {
                member.devices.iter().any(|d| d.device == a.device && d.may_decide && p256_verify(&d.key, &a.sig, &msg))
            };
            if ok {
                Ok(Reviewed {
                    member,
                    approve,
                })
            } else {
                Err(bad())
            }
        })
        .collect()
}

/// The countersigned resolution, checked against the roster: a member stays only where the obligation's reviewers or
/// its escalation chain admit them ([`reins_policy::org::ReviewSpec::may_resolve`]): groups, roles and named members
/// from the roster, the manager from the roster's `manager`, backups from its `backups`. Code owners, on-call and
/// resource owners are taken on the countersignature (`docs/security-model.md`).
fn checked_resolution(verdict: &ManagedVerdict, record: &DecisionRecord, roster: &Roster) -> ResolvedSets {
    let facts = RosterFacts {
        requester_manager: roster.member(&record.member).and_then(|m| m.manager.as_deref()),
        backups: &roster.backups,
    };
    let mut out = ResolvedSets::new();
    for o in &verdict.obligations {
        let Some(named) = record.resolved.get(&o.id) else {
            continue;
        };
        let kept = named
            .iter()
            .filter(|u| {
                roster
                    .member(u)
                    .is_some_and(|m| o.spec.may_resolve(&m.user, &m.groups, &m.roles, &record.member, &facts))
            })
            .cloned()
            .collect();
        out.insert(o.id.clone(), kept);
    }
    out
}

fn weigh(verdict: &ManagedVerdict, record: &DecisionRecord, roster: &Roster, reviewed: &[Reviewed<'_>]) -> Met {
    let approvals: Vec<Approval<'_>> = reviewed
        .iter()
        .map(|r| Approval {
            member: &r.member.user,
            groups: &r.member.groups,
            roles: &r.member.roles,
            approve: r.approve,
        })
        .collect();
    obligations_met_resolved(
        &verdict.obligations,
        &record.member,
        &approvals,
        &checked_resolution(verdict, record, roster),
    )
}

/// The DER prefix of a P-256 SubjectPublicKeyInfo, up to the uncompressed point.
const P256_SPKI_PREFIX: [u8; 26] = [
    0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08, 0x2a, 0x86, 0x48, 0xce,
    0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00,
];

/// Whether `sig_b64` (DER ECDSA, base64) by the device key `spki_b64` (P-256 SPKI DER, base64) signs `msg`.
#[must_use]
pub fn p256_verify(spki_b64: &str, sig_b64: &str, msg: &[u8]) -> bool {
    let Ok(der) = data_encoding::BASE64.decode(spki_b64.trim().as_bytes()) else {
        return false;
    };
    if der.len() != 91 || der[..26] != P256_SPKI_PREFIX || der[26] != 0x04 {
        return false;
    }
    let Ok(sig) = data_encoding::BASE64.decode(sig_b64.trim().as_bytes()) else {
        return false;
    };
    ring::signature::UnparsedPublicKey::new(&ring::signature::ECDSA_P256_SHA256_ASN1, &der[26..])
        .verify(msg, &sig)
        .is_ok()
}

#[cfg(test)]
mod tests {
    use reins_policy::matrix::{Issuer, Rule, Then, When};
    use reins_policy::org::MemberPolicy;
    use reins_proto::managed::{EnvelopeKind, LeaseAccess, Verdict};
    use serde_json::json;

    use super::*;
    use crate::testkit::*;

    fn check(f: &Fixture, job: &Job) -> Result<Verified, Refusal> {
        let p = f.policy_env(&f.policy);
        verify_job(job, &f.pins(), &p, &p, &f.roster_env(), now())
    }

    fn redecide(f: &Fixture, job: &mut Job, edit: impl FnOnce(&mut DecisionRecord)) {
        let mut record: DecisionRecord = job.decision.parse().unwrap();
        edit(&mut record);
        job.decision = f.countersign(&record);
    }

    #[test]
    fn an_auto_job_verifies() {
        let f = Fixture::new();
        let v = check(&f, &f.auto_job()).unwrap();
        assert_eq!(v.member_email, "lee@acme.com");
        assert_eq!(v.record.verdict, Verdict::Auto);
    }

    #[test]
    fn a_reviewed_job_verifies() {
        let f = Fixture::new();
        let (job, _) = f.reviewed_job();
        assert_eq!(check(&f, &job).unwrap().record.verdict, Verdict::Approved);
    }

    #[test]
    fn a_body_that_is_not_the_approved_digest_is_refused() {
        let f = Fixture::new();
        let mut job = f.auto_job();
        job.args["title"] = json!("Delete everything");
        assert_eq!(check(&f, &job), Err(Refusal::DigestMismatch));
        // Another tool with the same arguments.
        let mut job = f.auto_job();
        job.tool = "github_comment".into();
        assert_eq!(check(&f, &job), Err(Refusal::DigestMismatch));
        // Another request's decision.
        let mut job = f.auto_job();
        job.request = "or_2".into();
        assert_eq!(check(&f, &job), Err(Refusal::DigestMismatch));
    }

    #[test]
    fn a_policy_not_signed_by_the_issuer_is_refused() {
        let f = Fixture::new();
        let job = f.auto_job();
        let roster = f.roster_env();
        let forged =
            sign_env(EnvelopeKind::Policy, ORG, "iss-1", &ed25519(9), &serde_json::to_value(&f.policy).unwrap());
        let good = f.policy_env(&f.policy);
        assert_eq!(verify_job(&job, &f.pins(), &forged, &good, &roster, now()), Err(Refusal::UnsignedPolicy));
        assert_eq!(verify_job(&job, &f.pins(), &good, &forged, &roster, now()), Err(Refusal::UnsignedPolicy));
        // Signed, but not the snapshot the decision names.
        let mut other = f.policy.clone();
        other.revision = 7;
        let other = f.policy_env(&other);
        assert_eq!(verify_job(&job, &f.pins(), &other, &good, &roster, now()), Err(Refusal::PolicyMismatch));
        assert_eq!(verify_job(&job, &f.pins(), &good, &other, &roster, now()), Err(Refusal::PolicyMismatch));
        // A decision-service key can't sign a policy.
        let by_decision_key =
            sign_env(EnvelopeKind::Policy, ORG, "dec-1", &f.decision, &serde_json::to_value(&f.policy).unwrap());
        assert_eq!(verify_job(&job, &f.pins(), &by_decision_key, &good, &roster, now()), Err(Refusal::UnsignedPolicy));
    }

    #[test]
    fn a_decision_not_countersigned_is_refused() {
        let f = Fixture::new();
        let mut job = f.auto_job();
        let record: DecisionRecord = job.decision.parse().unwrap();
        job.decision =
            sign_env(EnvelopeKind::Decision, ORG, "dec-1", &ed25519(9), &serde_json::to_value(&record).unwrap());
        assert_eq!(check(&f, &job), Err(Refusal::BadCountersig));
        // The right key, but the body edited after signing.
        let mut job = f.auto_job();
        job.decision.body = job.decision.body.replace("\"auto\"", "\"approved\"");
        assert_eq!(check(&f, &job), Err(Refusal::BadCountersig));
    }

    #[test]
    fn a_server_claiming_auto_for_a_review_request_is_refused() {
        let f = Fixture::new();
        let (mut job, _) = f.reviewed_job();
        redecide(&f, &mut job, |r| {
            r.verdict = Verdict::Auto;
            r.approvals.clear();
        });
        assert_eq!(
            check(&f, &job),
            Err(Refusal::VerdictDiffers {
                ours: "review".into(),
                theirs: "auto".into()
            })
        );
        // Nor approved without the obligation met.
        let (mut job, _) = f.reviewed_job();
        redecide(&f, &mut job, |r| r.approvals.clear());
        assert_eq!(check(&f, &job), Err(Refusal::MissingApprovals(vec!["prod-review".into()])));
        // A denied decision is never run.
        let mut job = f.auto_job();
        redecide(&f, &mut job, |r| r.verdict = Verdict::Denied);
        assert!(matches!(check(&f, &job), Err(Refusal::VerdictDiffers { .. })));
    }

    #[test]
    fn an_org_deny_is_refused_whatever_the_server_says() {
        let mut f = Fixture::new();
        let job = f.auto_job();
        f.policy.rules.push(org_rule("api-deny", "acme/api", Then::Deny, None));
        let mut job2 = job.clone();
        let mut record: DecisionRecord = job.decision.parse().unwrap();
        record.snapshot = f.policy.hash();
        job2.decision = f.countersign(&record);
        job2.current_policy = f.policy.hash();
        assert_eq!(
            check(&f, &job2),
            Err(Refusal::VerdictDiffers {
                ours: "deny".into(),
                theirs: "auto".into()
            })
        );
    }

    #[test]
    fn missing_or_forged_device_signatures_are_refused() {
        let f = Fixture::new();
        // Kim's approval, signed by another key.
        let (mut job, _) = f.reviewed_job();
        redecide(&f, &mut job, |r| r.approvals[0].sig = f.lee.sign(b"anything"));
        assert_eq!(
            check(&f, &job),
            Err(Refusal::BadApproval {
                member: KIM.into()
            })
        );
        // Lee's device claimed as Kim's.
        let (mut job, _) = f.reviewed_job();
        redecide(&f, &mut job, |r| {
            let mut a = f.approve(r, KIM, &f.lee);
            a.device = f.lee.id.clone();
            r.approvals = vec![a];
        });
        assert_eq!(
            check(&f, &job),
            Err(Refusal::BadApproval {
                member: KIM.into()
            })
        );
        // An approval of another digest.
        let (mut job, _) = f.reviewed_job();
        redecide(&f, &mut job, |r| {
            let mut other = r.clone();
            other.digest = "0".repeat(64);
            r.approvals = vec![f.approve(&other, KIM, &f.kim)];
        });
        assert!(matches!(check(&f, &job), Err(Refusal::BadApproval { .. })));
        // The requester's own approval doesn't satisfy a review that excludes them.
        let (mut job, _) = f.reviewed_job();
        redecide(&f, &mut job, |r| r.approvals = vec![f.approve(r, LEE, &f.lee)]);
        assert_eq!(check(&f, &job), Err(Refusal::MissingApprovals(vec!["prod-review".into()])));
    }

    #[test]
    fn an_approval_from_a_device_that_may_not_decide_is_refused() {
        let f = Fixture::new();
        let (mut job, _) = f.reviewed_job();
        redecide(&f, &mut job, |r| r.approvals = vec![f.approve(r, SAM, &f.sam)]);
        assert_eq!(
            check(&f, &job),
            Err(Refusal::BadApproval {
                member: SAM.into()
            })
        );
    }

    #[test]
    fn an_old_roster_is_refused() {
        let mut f = Fixture::new();
        f.roster_at = now() - 901;
        assert_eq!(check(&f, &f.auto_job()), Err(Refusal::StaleRoster));
        f.roster_at = now() - 899;
        assert!(check(&f, &f.auto_job()).is_ok());
        // Not signed by the issuer.
        let job = f.auto_job();
        let p = f.policy_env(&f.policy);
        let forged =
            sign_env(EnvelopeKind::Roster, ORG, "iss-1", &ed25519(9), &serde_json::to_value(f.roster()).unwrap());
        assert_eq!(verify_job(&job, &f.pins(), &p, &p, &forged, now()), Err(Refusal::UnsignedRoster));
    }

    #[test]
    fn a_newer_stricter_policy_makes_it_stale() {
        let f = Fixture::new();
        let mut job2 = f.auto_job();
        let mut stricter = f.policy.clone();
        stricter.revision = 2;
        stricter.rules.push(org_rule("api-review", "acme/api", Then::Review, Some(sec_review())));
        job2.current_policy = stricter.hash();
        let r =
            verify_job(&job2, &f.pins(), &f.policy_env(&f.policy), &f.policy_env(&stricter), &f.roster_env(), now());
        assert!(matches!(&r, Err(Refusal::Stale(why)) if why.contains("api-review")), "{r:?}");
        let mut denying = f.policy.clone();
        denying.rules.push(org_rule("api-deny", "acme/api", Then::Deny, None));
        job2.current_policy = denying.hash();
        let r = verify_job(&job2, &f.pins(), &f.policy_env(&f.policy), &f.policy_env(&denying), &f.roster_env(), now());
        assert!(matches!(&r, Err(Refusal::Stale(why)) if why.contains("api-deny")), "{r:?}");
        // A newer policy that still allows it runs.
        let mut looser = f.policy.clone();
        looser.revision = 3;
        job2.current_policy = looser.hash();
        assert!(
            verify_job(&job2, &f.pins(), &f.policy_env(&f.policy), &f.policy_env(&looser), &f.roster_env(), now())
                .is_ok()
        );
    }

    #[test]
    fn another_orgs_decision_is_refused() {
        let f = Fixture::new();
        let mut job = f.auto_job();
        redecide(&f, &mut job, |r| r.org = "o2".into());
        assert_eq!(check(&f, &job), Err(Refusal::WrongOrg));
        // Another org's policy, even signed with this issuer key.
        let job = f.auto_job();
        let env = sign_env(EnvelopeKind::Policy, "o2", "iss-1", &f.issuer, &serde_json::to_value(&f.policy).unwrap());
        assert_eq!(verify_job(&job, &f.pins(), &env, &env, &f.roster_env(), now()), Err(Refusal::UnsignedPolicy));
    }

    #[test]
    fn an_expired_decision_is_refused() {
        let f = Fixture::new();
        let mut job = f.auto_job();
        redecide(&f, &mut job, |r| r.valid_until = now() - 1);
        assert_eq!(check(&f, &job), Err(Refusal::Expired));
    }

    #[test]
    fn a_tampered_lease_is_refused() {
        let f = Fixture::new();
        let job = f.lease_job("BOX");
        assert!(check(&f, &job).is_ok(), "{:?}", check(&f, &job));
        let tamper = |edit: &dyn Fn(&mut Job)| {
            let mut j = job.clone();
            edit(&mut j);
            check(&f, &j)
        };
        assert_eq!(tamper(&|j| j.lease.as_mut().unwrap().repo = "acme/prod-db".into()), Err(Refusal::DigestMismatch));
        assert_eq!(tamper(&|j| j.lease.as_mut().unwrap().access = LeaseAccess::Push), Err(Refusal::DigestMismatch));
        assert_eq!(tamper(&|j| j.lease.as_mut().unwrap().box_key = "ATTACKER".into()), Err(Refusal::DigestMismatch));
        assert_eq!(tamper(&|j| j.lease.as_mut().unwrap().expires_at += 86_400), Err(Refusal::DigestMismatch));
        assert_eq!(tamper(&|j| j.lease = None), Err(Refusal::DigestMismatch));
        // A lease decision without its binding.
        let mut j = job.clone();
        redecide(&f, &mut j, |r| r.lease = None);
        assert_eq!(check(&f, &j), Err(Refusal::DigestMismatch));
        // Run as a plain execution.
        assert_eq!(tamper(&|j| j.kind = JobKind::Execute), Err(Refusal::DigestMismatch));
    }

    #[test]
    fn a_member_policy_must_be_signed_by_their_device_and_named_by_the_decision() {
        let f = Fixture::new();
        let mine = MemberPolicy {
            matrix: None,
            rules: vec![Rule {
                id: "careful".into(),
                issuer: Issuer::Owner,
                when: When {
                    resource: Some("acme/web".into()),
                    ..When::default()
                },
                then: Then::Review,
                not_after: None,
                accepted: true,
                explain: String::new(),
            }],
        };
        let args = issue("acme/api");
        let record = f.record("github_create_issue", &args, Some(&mine));
        let mut job = f.job("github_create_issue", args, &record);
        job.member_policy = Some(f.member_policy(&mine, &f.lee));
        assert!(check(&f, &job).is_ok(), "{:?}", check(&f, &job));
        // Signed by someone else's device.
        let mut j = job.clone();
        j.member_policy = Some(f.member_policy(&mine, &f.kim));
        assert_eq!(check(&f, &j), Err(Refusal::UnsignedMemberPolicy));
        // Its version or time changed after signing.
        let mut j = job.clone();
        j.member_policy.as_mut().unwrap().version += 1;
        assert_eq!(check(&f, &j), Err(Refusal::UnsignedMemberPolicy));
        // Dropped by the server.
        let mut j = job.clone();
        j.member_policy = None;
        assert_eq!(check(&f, &j), Err(Refusal::UnsignedMemberPolicy));
        // Another (looser) policy than the one decided with.
        let mut j = job;
        j.member_policy = Some(f.member_policy(&MemberPolicy::default(), &f.lee));
        assert_eq!(check(&f, &j), Err(Refusal::UnsignedMemberPolicy));
        // The member's review rule is re-evaluated: their rule on acme/web makes an Auto there differ.
        let args = issue("acme/web");
        let mut record = f.record("github_create_issue", &args, Some(&mine));
        record.verdict = Verdict::Auto;
        let mut j = f.job("github_create_issue", args, &record);
        j.member_policy = Some(f.member_policy(&mine, &f.lee));
        assert!(matches!(check(&f, &j), Err(Refusal::VerdictDiffers { .. })));
    }

    // ---- O3: signed web approvals and countersigned resolutions.

    #[test]
    fn passkey_approvals_verify_against_the_roster() {
        let f = Fixture::new();
        let (mut job, _) = f.reviewed_job();
        redecide(&f, &mut job, |r| r.approvals = vec![f.web_approve(r, KIM, &f.kim_passkey, 0x05)]);
        assert!(check(&f, &job).is_ok(), "{:?}", check(&f, &job));
        // A passkey the roster doesn't list for Kim (Lee's), a synced one, one without user verification.
        for (key, flags) in [(&f.lee_passkey, 0x05), (&f.kim_passkey, 0x0d), (&f.kim_passkey, 0x01)] {
            let (mut job, _) = f.reviewed_job();
            redecide(&f, &mut job, |r| r.approvals = vec![f.web_approve(r, KIM, key, flags)]);
            assert_eq!(
                check(&f, &job),
                Err(Refusal::BadApproval {
                    member: KIM.into()
                }),
                "{flags:#x}"
            );
        }
        // Asserted at another origin.
        let (mut job, _) = f.reviewed_job();
        redecide(&f, &mut job, |r| {
            let mut a = f.web_approve(r, KIM, &f.kim_passkey, 0x05);
            let msg = review_message(
                ORG,
                ReviewSubject::Request,
                &r.request,
                &r.digest,
                ReviewDecision::Approve,
                &r.snapshot,
                a.at,
            );
            let (sig, proof) = f.kim_passkey.assert(&webauthn_challenge(&msg), "https://evil.example", 0x05);
            (a.sig, a.webauthn) = (sig, Some(proof));
            r.approvals = vec![a];
        });
        assert!(matches!(check(&f, &job), Err(Refusal::BadApproval { .. })));
        // An executor with no web origin pinned takes no web approval.
        let (mut job, _) = f.reviewed_job();
        redecide(&f, &mut job, |r| r.approvals = vec![f.web_approve(r, KIM, &f.kim_passkey, 0x05)]);
        let mut pins = f.pins();
        pins.web_origin = None;
        let p = f.policy_env(&f.policy);
        assert!(matches!(verify_job(&job, &pins, &p, &p, &f.roster_env(), now()), Err(Refusal::BadApproval { .. })));
    }

    #[test]
    fn passkey_approvals_are_refused_for_l3() {
        let f = Fixture::new();
        // Deleting a repository is L3: Lee's own review. On the phone it verifies; on the web it never does.
        let args = json!({"method": "DELETE", "path": "/repos/acme/api"});
        let mut record = f.record("github_api_write", &args, None);
        assert_eq!(record.verdict, Verdict::Approved, "{record:?}");
        record.approvals = vec![f.approve(&record, LEE, &f.lee)];
        let job = f.job("github_api_write", args.clone(), &record);
        assert!(check(&f, &job).is_ok(), "{:?}", check(&f, &job));
        record.approvals = vec![f.web_approve(&record, LEE, &f.lee_passkey, 0x05)];
        let job = f.job("github_api_write", args, &record);
        assert_eq!(
            check(&f, &job),
            Err(Refusal::BadApproval {
                member: LEE.into()
            })
        );
    }

    #[test]
    fn a_manager_not_in_the_roster_is_refused() {
        let mut f = Fixture::new();
        f.policy.rules.push(org_rule(
            "mgr",
            "acme/mgr",
            Then::Review,
            // Escalation only to the manager again, so Security admins (Kim) can't be named through the chain.
            Some(reins_policy::org::ReviewSpec {
                escalate: vec![reins_policy::org::Reviewers::Manager],
                ..reins_policy::org::ReviewSpec::by(reins_policy::org::Reviewers::Manager)
            }),
        ));
        let args = issue("acme/mgr");
        let mut record = f.record("github_create_issue", &args, None);
        // The decision service says Sam is Lee's manager; the roster says Kim.
        record.resolved.insert("mgr".into(), vec![SAM.into()]);
        record.approvals = vec![f.approve(&record, SAM, &f.sam)];
        let job = f.job("github_create_issue", args.clone(), &record);
        assert!(matches!(check(&f, &job), Err(Refusal::BadApproval { .. } | Refusal::MissingApprovals(_))));
        // Sam's device may not decide anyway; with Kim named and approving it runs, and only so.
        record.resolved.insert("mgr".into(), vec![KIM.into()]);
        record.approvals = vec![f.approve(&record, KIM, &f.kim)];
        let job = f.job("github_create_issue", args.clone(), &record);
        assert!(check(&f, &job).is_ok(), "{:?}", check(&f, &job));
        // Kim, but the roster names nobody as Lee's manager now.
        f.manager = None;
        assert_eq!(check(&f, &job), Err(Refusal::MissingApprovals(vec!["mgr".into()])));
        // The requester named as their own manager never counts.
        f.manager = Some(LEE.into());
        record.resolved.insert("mgr".into(), vec![LEE.into()]);
        record.approvals = vec![f.approve(&record, LEE, &f.lee)];
        let job = f.job("github_create_issue", args, &record);
        assert_eq!(check(&f, &job), Err(Refusal::MissingApprovals(vec!["mgr".into()])));
    }
}
