//! An org's policy (`docs/design/decisions-v2/05-organizations.md` §7.3, `06` §3): a draft (by a Policy author or a
//! Security admin), activated by a two-person, device-signed admin change ([`AdminAction::ActivatePolicy`]), and then
//! signed by the org issuer key. Executors accept only snapshots the issuer signed (08 R5); every decision names the
//! snapshot it was made against.
//!
//! Facts the org didn't author but verified (its verified domains, its GitHub organization) are added to every draft
//! as Work, so the signed snapshot carries them. Simulation, policy as code and staged rollout are O4
//! ([`super::simulation`], [`super::policy_repo`], [`super::activation`], [`super::rollout`]).

use reins_policy::{
    context::{FactKind, Zone},
    matrix::Issuer,
    org::OrgPolicy,
};
use reins_proto::{
    managed::{Envelope, canonical_json},
    org::Capability,
};
use serde_json::json;

use super::{OrgError, audit, require};
use crate::db::{
    DbConnInner,
    models::{
        reins_org,
        reins_org_keys::{self, ReinsOrgPolicy, policy_state},
    },
};

/// Largest policy accepted, in bytes.
pub const MAX_POLICY_BYTES: usize = 256 * 1024;

/// 05 §7.2's *Developer baseline*: routine work (L0–L2) in the org's own repositories and with its people is Auto, a
/// soft Ask is the member's own review, and L3 and the always-ask floor are always reviewed.
pub const DEVELOPER_BASELINE: &str = include_str!("policy_baseline.json");

/// Parses a draft for `org`: every rule is the org's, and the org's verified domains and GitHub organization are Work.
pub fn parse_draft(c: &mut DbConnInner, org: &str, body: &str) -> Result<OrgPolicy, OrgError> {
    if body.len() > MAX_POLICY_BYTES {
        return Err(OrgError::Invalid("The policy is too large."));
    }
    let mut policy: OrgPolicy = serde_json::from_str(body).map_err(|e| {
        info!("Policy draft of {org} does not parse: {e}");
        OrgError::Invalid("The policy isn't valid JSON for a Reins org policy.")
    })?;
    org.clone_into(&mut policy.org);
    for r in &mut policy.rules {
        r.rule.issuer = Issuer::Org(org.to_owned());
    }
    for d in reins_org::q_domains(c, org)?.into_iter().filter(|d| d.verified_at.is_some()) {
        policy.facts.set(FactKind::Domain, &d.domain, Zone::Work);
    }
    if let Some(gh) = reins_org::q_org(c, org)?.and_then(|t| t.github_org) {
        policy.facts.set(FactKind::Owner, &gh, Zone::Work);
    }
    policy.validate().map_err(|e| {
        info!("Policy draft of {org} refused: {e}");
        OrgError::Invalid("The policy is not valid (rule ids, reviewers, windows or quorum).")
    })?;
    Ok(policy)
}

/// Saves a draft as the next revision; returns (revision, hash). Needs [`Capability::AuthorPolicy`].
pub fn save_draft(
    c: &mut DbConnInner,
    org: &str,
    author: &str,
    body: &str,
    now: i64,
) -> Result<(u64, String), OrgError> {
    save_draft_with(c, org, author, body, None, None, now)
}

/// [`save_draft`], with where the draft came from (`pr#12@<sha>`, `main@<sha>`, `console`, `template:<name>`) and the
/// rollout plan it ships with. The policy repo saves as [`super::simulation::SYSTEM_ACTOR`]: what it saves is a draft
/// only, which two Security admins must still activate.
#[allow(clippy::too_many_arguments, reason = "one draft's parts")]
pub fn save_draft_with(
    c: &mut DbConnInner,
    org: &str,
    author: &str,
    body: &str,
    source_ref: Option<&str>,
    plan: Option<&reins_policy::source::RolloutPlanSource>,
    now: i64,
) -> Result<(u64, String), OrgError> {
    if author != super::simulation::SYSTEM_ACTOR {
        require(c, org, author, Capability::AuthorPolicy)?;
    }
    let mut policy = parse_draft(c, org, body)?;
    let revision = reins_org_keys::q_last_revision(c, org)? + 1;
    policy.revision = u64::try_from(revision).unwrap_or(0);
    let hash = policy.hash();
    let text = canonical_json(&serde_json::to_value(&policy).map_err(|e| OrgError::Internal(e.to_string()))?);
    reins_org_keys::q_insert_policy(
        c,
        &ReinsOrgPolicy {
            org_uuid: org.to_owned(),
            revision,
            hash: hash.clone(),
            body: text,
            envelope: None,
            state: policy_state::DRAFT.to_owned(),
            created_by: author.to_owned(),
            created_at: now,
            activated_at: None,
            source_ref: source_ref.map(str::to_owned),
            rollout_plan: match plan {
                Some(p) => Some(serde_json::to_string(p).map_err(|e| OrgError::Internal(e.to_string()))?),
                None => None,
            },
        },
    )?;
    audit::record(
        c,
        org,
        audit::kinds::POLICY_DRAFTED,
        Some(author),
        None,
        &json!({"revision": revision, "hash": hash, "source": source_ref}),
        now,
    )?;
    Ok((policy.revision, hash))
}

/// The draft `revision` with `hash`, ready to activate.
pub fn check_draft(c: &mut DbConnInner, org: &str, revision: u64, hash: &str) -> Result<ReinsOrgPolicy, OrgError> {
    let rev = i64::try_from(revision).map_err(|_| OrgError::NotFound)?;
    let row = reins_org_keys::q_policy(c, org, rev)?.ok_or(OrgError::NotFound)?;
    if row.state != policy_state::DRAFT {
        return Err(OrgError::Invalid("That policy revision isn't a draft."));
    }
    if !crate::crypto::ct_eq(&row.hash, hash) {
        return Err(OrgError::Invalid("That policy changed since it was proposed."));
    }
    Ok(row)
}

/// Activates the draft, from an approved admin change only: the issuer key signs it, and it becomes active at once
/// (the first policy) or the candidate of a staged rollout (O4, [`super::activation::activate`]).
pub fn apply_activation(
    c: &mut DbConnInner,
    org: &str,
    revision: u64,
    hash: &str,
    report: Option<&str>,
    actor: &str,
    now: i64,
) -> Result<(), OrgError> {
    super::activation::activate(c, org, revision, hash, report, actor, now)
}

fn parse_row(row: &ReinsOrgPolicy) -> Result<(OrgPolicy, Envelope), OrgError> {
    let env: Envelope = serde_json::from_str(row.envelope.as_deref().unwrap_or_default())
        .map_err(|e| OrgError::Internal(format!("stored policy envelope: {e}")))?;
    let policy: OrgPolicy = env.parse().map_err(|e| OrgError::Internal(format!("stored policy: {e}")))?;
    Ok((policy, env))
}

/// The active policy and its signed envelope.
pub fn active(c: &mut DbConnInner, org: &str) -> Result<Option<(OrgPolicy, Envelope)>, OrgError> {
    match reins_org_keys::q_active_policy(c, org)? {
        Some(row) => parse_row(&row).map(Some),
        None => Ok(None),
    }
}

/// A signed policy of `org` by its snapshot hash (active or retired).
pub fn by_hash(c: &mut DbConnInner, org: &str, hash: &str) -> Result<Option<Envelope>, OrgError> {
    match reins_org_keys::q_policy_by_hash(c, org, hash)? {
        Some(row) => parse_row(&row).map(|(_, env)| Some(env)),
        None => Ok(None),
    }
}

#[cfg(all(test, sqlite))]
mod tests {
    use reins_audit::verify_envelope;
    use reins_proto::org::Role;

    use super::*;
    use crate::api::reins::org::{
        kms::tests::{trusted, use_test_kms},
        kms::{self, KeyPurpose},
        testing::{add_org, db, member},
    };

    #[test]
    fn a_draft_must_validate_and_belongs_to_this_org() {
        use_test_kms();
        let mut c = db();
        add_org(&mut c, "o1");
        member(&mut c, "o1", "u1", &[Role::PolicyAuthor]);
        let (rev, hash) = save_draft(&mut c, "o1", "u1", DEVELOPER_BASELINE, 5).unwrap();
        assert_eq!(rev, 1);
        let row = reins_org_keys::q_policy(&mut c, "o1", 1).unwrap().unwrap();
        let parsed: OrgPolicy = serde_json::from_str(&row.body).unwrap();
        assert_eq!((parsed.org.as_str(), parsed.hash()), ("o1", hash));
        assert!(save_draft(&mut c, "o1", "u1", "{not json", 5).is_err());
        let bad = DEVELOPER_BASELINE
            .replace("\"rules\": []", r#""rules": [{"id": "x", "issuer": "owner", "when": {}, "then": "review"}]"#);
        assert!(save_draft(&mut c, "o1", "u1", &bad, 5).is_err(), "a review rule without reviewers");
        // Nobody else drafts.
        member(&mut c, "o1", "u2", &[]);
        assert_eq!(save_draft(&mut c, "o1", "u2", DEVELOPER_BASELINE, 5), Err(OrgError::Forbidden));
    }

    #[test]
    fn activation_waits_for_a_second_security_admin() {
        use crate::api::reins::org::admin::{Proposed, approve, propose};
        use reins_proto::org::AdminAction;
        use_test_kms();
        let mut c = db();
        add_org(&mut c, "o1");
        member(&mut c, "o1", "u1", &[Role::SecurityAdmin]);
        member(&mut c, "o1", "u2", &[Role::SecurityAdmin]);
        let (revision, hash) = save_draft(&mut c, "o1", "u1", DEVELOPER_BASELINE, 5).unwrap();
        let action = AdminAction::ActivatePolicy {
            revision,
            hash,
            report: None,
        };
        let (Proposed::Pending(change), _) = propose(&mut c, "o1", "u1", &action, None, 6).unwrap() else {
            panic!("a second Security admin exists: it must wait");
        };
        assert!(active(&mut c, "o1").unwrap().is_none());
        assert!(approve(&mut c, "o1", &change, "u1", None, 7).is_err(), "not by the proposer");
        approve(&mut c, "o1", &change, "u2", None, 7).unwrap();
        assert_eq!(active(&mut c, "o1").unwrap().unwrap().0.revision, revision);
    }

    #[test]
    fn the_active_envelope_verifies_with_the_issuer_key() {
        use_test_kms();
        let mut c = db();
        add_org(&mut c, "o1");
        member(&mut c, "o1", "u1", &[Role::SecurityAdmin]);
        let (rev, hash) = save_draft(&mut c, "o1", "u1", DEVELOPER_BASELINE, 5).unwrap();
        assert!(apply_activation(&mut c, "o1", rev, "wrong", None, "u1", 6).is_err());
        apply_activation(&mut c, "o1", rev, &hash, None, "u1", 6).unwrap();
        let (policy, env) = active(&mut c, "o1").unwrap().unwrap();
        assert_eq!(policy.hash(), hash);
        let issuer = kms::signing_key(&mut c, "o1", KeyPurpose::Issuer, 7).unwrap();
        let decision = kms::signing_key(&mut c, "o1", KeyPurpose::Decision, 7).unwrap();
        assert!(verify_envelope(&env, &[trusted(&issuer)]));
        assert!(!verify_envelope(&env, &[trusted(&decision)]));
        assert_eq!(by_hash(&mut c, "o1", &hash).unwrap(), Some(env));
        assert!(by_hash(&mut c, "o2", &hash).unwrap().is_none());
        // Activated once.
        assert!(apply_activation(&mut c, "o1", rev, &hash, None, "u1", 8).is_err());
    }
}
