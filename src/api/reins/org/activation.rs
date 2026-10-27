//! Activating an org policy (`docs/design/decisions-v2/05-organizations.md` §7.3, `06` §3, `08` R5):
//!
//! - **Evidence.** Once a policy is active, an activation names a simulation report made for exactly the draft's hash,
//!   at most a day old, that found no drift. A draft changed after it was simulated is another hash: it needs its own
//!   simulation. A draft a newer push to the same PR (or branch) superseded can't be activated.
//! - **Two people, always.** Activation takes two holders of [`Capability::ActivatePolicy`], each signing on their
//!   own enrolled phone: there is no sole-holder path and two-person approval can't be turned off for it. The only
//!   exception is a Family tenant (P7), where every guardian holding the capability (at least one) decides.
//! - **Staged.** With a policy already active, activation starts a rollout ([`super::rollout`]): the issuer signs the
//!   candidate and pre-signs every stage and the rollback; the first stage is published at once.

use reins_policy::source::RolloutPlanSource;
use reins_proto::{
    managed::EnvelopeKind,
    org::Capability,
    rollout::{Rollout, RolloutPlan, Stage},
};
use serde_json::{Value, json};

use super::{
    OrgError, audit,
    kms::{self, KeyPurpose},
    policy,
};
use crate::db::{
    DbConnInner,
    models::{
        reins_org,
        reins_org_keys::{self, ReinsOrgPolicy, policy_state},
        reins_org_policy_code::{self as code, ReinsOrgRollout, rollout_state},
    },
};

/// How old a simulation may be to back an activation.
pub const EVIDENCE_SECS: i64 = 86_400;
/// The tier whose guardians activate alone (P7's `Tier::Family`, by its stored name).
pub const FAMILY_TIER: &str = "family";

/// Whether `org` is a Family tenant.
pub fn is_family(c: &mut DbConnInner, org: &str) -> Result<bool, OrgError> {
    Ok(reins_org::q_org(c, org)?.is_some_and(|t| t.tier == FAMILY_TIER))
}

/// Whether an activation may apply on its proposer's device signature alone: only in a Family with one guardian.
pub fn may_apply_alone(c: &mut DbConnInner, org: &str) -> Result<bool, OrgError> {
    Ok(is_family(c, org)? && reins_org::q_holders(c, org, Capability::ActivatePolicy)?.len() <= 1)
}

/// The source prefix whose newer drafts supersede this one: `pr#12@` for `pr#12@<sha>`, `main@` for `main@<sha>`.
fn source_prefix(source_ref: &str) -> Option<&str> {
    source_ref.find('@').map(|i| &source_ref[..=i])
}

/// Checks an `ActivatePolicy { revision, hash, report }` as of now (at proposal and again at approval).
pub fn check(
    c: &mut DbConnInner,
    org: &str,
    revision: u64,
    hash: &str,
    report: Option<&str>,
    now: i64,
) -> Result<ReinsOrgPolicy, OrgError> {
    let row = policy::check_draft(c, org, revision, hash)?;
    let holders = reins_org::q_holders(c, org, Capability::ActivatePolicy)?;
    if is_family(c, org)? {
        if holders.is_empty() {
            return Err(OrgError::Invalid("Activation needs a guardian."));
        }
    } else if holders.len() < 2 {
        return Err(OrgError::Invalid("Activation needs a second Security admin."));
    }
    if let Some(src) = row.source_ref.as_deref()
        && let Some(prefix) = source_prefix(src)
        && code::q_newest_from_source(c, org, prefix)?.is_some_and(|newest| newest > row.revision)
    {
        return Err(OrgError::Invalid("A newer push replaced this draft."));
    }
    if policy::active(c, org)?.is_none() {
        // The first policy: no decision was ever made, so there is no history to simulate.
        return Ok(row);
    }
    let Some(report) = report else {
        return Err(OrgError::Invalid("Simulate this draft first."));
    };
    let Some((r, stored)) = super::simulation::report(c, org, report)? else {
        return Err(OrgError::Invalid("Simulate this draft first."));
    };
    if !crate::crypto::ct_eq(&stored.draft_hash, hash) || r.draft_hash != hash {
        return Err(OrgError::Invalid("That simulation is of another draft. Simulate this one."));
    }
    if stored.ran_at < now - EVIDENCE_SECS {
        return Err(OrgError::Invalid("That simulation is over a day old. Simulate again."));
    }
    if r.drift > 0 {
        return Err(OrgError::Invalid("The simulation found drift: the active policy doesn't reproduce history."));
    }
    Ok(row)
}

/// The pilot group's members (user ids): the group named `name` in `org`.
fn pilot_members(c: &mut DbConnInner, org: &str, name: &str) -> Result<Vec<String>, OrgError> {
    let Some((group, _, _)) = reins_org::q_groups(c, org)?.into_iter().find(|(_, n, _)| n.eq_ignore_ascii_case(name))
    else {
        return Err(OrgError::Invalid("The rollout's pilot group isn't in this organization."));
    };
    let memberships = reins_org::q_group_members(c, &group)?;
    let mut users = Vec::new();
    for m in reins_org::q_members(c, org)? {
        let (membership, user) = (m.uuid.to_string(), m.user_uuid.to_string());
        if memberships.contains(&membership) && super::member_of(c, org, &user)?.is_some() {
            users.push(user);
        }
    }
    users.sort();
    Ok(users)
}

/// The draft's rollout plan, pilot members resolved now; a draft without one rolls out to everyone at once.
fn plan_of(c: &mut DbConnInner, org: &str, row: &ReinsOrgPolicy) -> Result<RolloutPlan, OrgError> {
    let source: RolloutPlanSource = match row.rollout_plan.as_deref() {
        Some(text) => serde_json::from_str(text).map_err(|e| OrgError::Internal(format!("stored plan: {e}")))?,
        None => RolloutPlanSource::default(),
    };
    let pilot = match &source.pilot_group {
        Some(g) => pilot_members(c, org, g)?,
        None => Vec::new(),
    };
    let mut plan = source.resolve(pilot);
    if plan.stages.is_empty() {
        plan.stages = vec![Stage::All];
    }
    plan.validate().map_err(|e| {
        info!("Org {org}: rollout plan of revision {} refused: {e}", row.revision);
        OrgError::Invalid("The rollout plan isn't valid.")
    })?;
    Ok(plan)
}

/// Activates draft `revision` (`hash`), from an approved admin change (`actor` approved it). With no active policy it
/// becomes active at once (O2); otherwise it becomes the candidate of a rollout whose every step is signed now.
pub fn activate(
    c: &mut DbConnInner,
    org: &str,
    revision: u64,
    hash: &str,
    report: Option<&str>,
    actor: &str,
    now: i64,
) -> Result<(), OrgError> {
    let row = policy::check_draft(c, org, revision, hash)?;
    let body: Value = serde_json::from_str(&row.body).map_err(|e| OrgError::Internal(e.to_string()))?;
    let env = kms::envelope(c, org, KeyPurpose::Issuer, EnvelopeKind::Policy, &body, now)?;
    let env_text = serde_json::to_string(&env).map_err(|e| OrgError::Internal(e.to_string()))?;
    let Some((active, _)) = policy::active(c, org)? else {
        if !reins_org_keys::q_activate(c, org, row.revision, &env_text, now)? {
            return Err(OrgError::Invalid("That policy revision isn't a draft."));
        }
        audit::record(
            c,
            org,
            audit::kinds::POLICY_ACTIVATED,
            Some(actor),
            None,
            &json!({"revision": revision, "hash": hash, "issuer_key": env.key_id, "report": report}),
            now,
        )?;
        return Ok(());
    };
    if code::q_open_rollout(c, org)?.is_some() {
        return Err(OrgError::Invalid("A rollout is in progress. Let it finish or roll it back first."));
    }
    let stable = active.hash();
    let plan = plan_of(c, org, &row)?;
    let rollout_id = format!("ro_{}", crate::util::get_uuid());
    let epoch = code::q_last_epoch(c, org)? + 1;
    let steps = Rollout::steps(org, &rollout_id, u64::try_from(epoch).unwrap_or(0), &stable, hash, &plan, now);
    let mut envelopes = Vec::with_capacity(steps.len());
    for step in &steps {
        let value: Value = serde_json::from_str(&step.body()).map_err(|e| OrgError::Internal(e.to_string()))?;
        envelopes.push(kms::envelope(c, org, KeyPurpose::Issuer, EnvelopeKind::Rollout, &value, now)?);
    }
    if !code::q_set_policy_state(
        c,
        org,
        row.revision,
        &[policy_state::DRAFT],
        policy_state::CANDIDATE,
        Some(&env_text),
        Some(now),
    )? {
        return Err(OrgError::Invalid("That policy revision isn't a draft."));
    }
    let rollout = ReinsOrgRollout {
        rollout_id: rollout_id.clone(),
        org_uuid: org.to_owned(),
        revision: row.revision,
        epoch,
        state: rollout_state::RUNNING.to_owned(),
        stable,
        candidate: hash.to_owned(),
        plan: serde_json::to_string(&plan).map_err(|e| OrgError::Internal(e.to_string()))?,
        envelopes: serde_json::to_string(&envelopes).map_err(|e| OrgError::Internal(e.to_string()))?,
        published_seq: 1,
        stage_started_at: now,
        reason: None,
        created_at: now,
        updated_at: now,
    };
    code::q_insert_rollout(c, &rollout)?;
    let approved_by = approvers(c, org, revision, hash)?;
    audit::record(
        c,
        org,
        audit::kinds::POLICY_ACTIVATED,
        Some(actor),
        Some(&rollout_id),
        &json!({"revision": revision, "hash": hash, "issuer_key": env.key_id, "report": report,
                "rollout_id": rollout_id, "stable": rollout.stable,
                "stages": plan.stages.iter().map(Stage::label).collect::<Vec<_>>(), "dwell_secs": plan.dwell_secs,
                "approvers": approved_by}),
        now,
    )?;
    // A single "everyone" stage has nothing to wait for: it completes now.
    if plan.stages.len() == 1 {
        super::rollout::complete(c, &rollout, None, now)?;
    }
    Ok(())
}

/// Who proposed and approved the activation of (`revision`, `hash`), from its change.
fn approvers(c: &mut DbConnInner, org: &str, revision: u64, hash: &str) -> Result<Vec<String>, OrgError> {
    let mut out = Vec::new();
    for ch in reins_org::q_changes(c, org, 200)? {
        let Ok(reins_proto::org::AdminAction::ActivatePolicy {
            revision: r,
            hash: h,
            ..
        }) = serde_json::from_str(&ch.action)
        else {
            continue;
        };
        if r == revision && h == hash {
            out.push(ch.proposed_by.clone());
            if let Some(by) = ch.decided_by {
                out.push(by);
            }
            break;
        }
    }
    Ok(out)
}

#[cfg(all(test, sqlite))]
pub(crate) mod tests {
    use diesel::prelude::*;
    use reins_audit::verify_envelope;
    use reins_policy::{
        org::OrgPolicy,
        source::{RolloutPlanSource, StageSource},
    };
    use reins_proto::{
        managed::Envelope,
        org::{AdminAction, Role},
        rollout::Guards,
    };

    use super::*;
    use crate::api::reins::org::{
        admin::{self, Proposed},
        decisions::tests::{ask, issue, pilot},
        kms::tests::trusted,
        simulation,
        testing::{db, member},
    };
    use crate::db::models::reins_org_request;

    /// The active pilot policy with one more review rule (`api-review` on `acme/api`, by group `sec`), saved as a draft
    /// with `plan`; returns (revision, hash).
    pub fn stricter_draft(
        c: &mut DbConnInner,
        author: &str,
        plan: Option<&RolloutPlanSource>,
        now: i64,
    ) -> (u64, String) {
        let (mut p, _) = policy::active(c, "o1").unwrap().unwrap();
        p.rules.push(
            serde_json::from_value(json!({"id": "api-review", "issuer": "owner", "when": {"resource": "acme/api"},
                "then": "review", "explain": "API needs Security",
                "review": {"reviewers": {"group": "sec"}, "class": "deferred"}}))
            .unwrap(),
        );
        policy::save_draft_with(c, "o1", author, &serde_json::to_string(&p).unwrap(), Some("console"), plan, now)
            .unwrap()
    }

    /// The standard plan: pilot (group `sec`), 25%, everyone; a minute's dwell, guards at `min` decisions.
    pub fn staged(min: u32) -> RolloutPlanSource {
        RolloutPlanSource {
            pilot_group: Some("Group sec".into()),
            stages: vec![StageSource::Pilot, StageSource::Percent(25), StageSource::All],
            dwell_secs: 60,
            guards: Guards {
                min_decisions: min,
                ..Guards::default()
            },
        }
    }

    fn activation(revision: u64, hash: &str, report: Option<&str>) -> AdminAction {
        AdminAction::ActivatePolicy {
            revision,
            hash: hash.to_owned(),
            report: report.map(str::to_owned),
        }
    }

    /// Dana proposes, Kim approves.
    pub fn activate_by_two(c: &mut DbConnInner, revision: u64, hash: &str, report: &str, now: i64) {
        let (Proposed::Pending(ch), _) =
            admin::propose(c, "o1", "dana", &activation(revision, hash, Some(report)), None, now).unwrap()
        else {
            panic!("an activation always waits for a second person");
        };
        admin::approve(c, "o1", &ch, "kim", None, now + 1).unwrap();
    }

    #[test]
    fn submit_retains_a_body_free_analysis() {
        let mut c = db();
        pilot(&mut c);
        let args = json!({"repo": "acme/api", "title": "Rotate the payroll key", "body": "the key is hunter2"});
        let s = ask(&mut c, &args, 100);
        let row = reins_org_request::q_request(&mut c, "o1", &s.id).unwrap().unwrap();
        let kept = row.analysis.unwrap();
        assert!(!kept.contains("payroll") && !kept.contains("hunter2"), "{kept}");
        let input: reins_policy::org::SimInput = serde_json::from_str(&kept).unwrap();
        assert_eq!((input.member.as_str(), input.connection.as_str(), input.at), ("lee", "conn-lee", 100));
        // With no rollout under way, no rollout is pinned.
        assert_eq!((row.rollout_id, row.rollout_seq), (None, None));
    }

    #[test]
    fn activation_without_fresh_simulation_is_refused() {
        let mut c = db();
        pilot(&mut c);
        let (rev, hash) = stricter_draft(&mut c, "dana", None, 50);
        let refused = |c: &mut DbConnInner, report: Option<&str>, now: i64| {
            admin::propose(c, "o1", "dana", &activation(rev, &hash, report), None, now).unwrap_err()
        };
        assert_eq!(refused(&mut c, None, 60), OrgError::Invalid("Simulate this draft first."));
        assert_eq!(refused(&mut c, Some("nope"), 60), OrgError::Invalid("Simulate this draft first."));
        let r = simulation::run(&mut c, "o1", "dana", rev, None, None, None, 100).unwrap().hash();
        assert!(
            admin::propose(&mut c, "o1", "dana", &activation(rev, &hash, Some(&r)), None, 100 + EVIDENCE_SECS).is_ok()
        );
        assert_eq!(
            refused(&mut c, Some(&r), 101 + EVIDENCE_SECS),
            OrgError::Invalid("That simulation is over a day old. Simulate again.")
        );
    }

    #[test]
    fn a_changed_draft_needs_a_new_simulation() {
        let mut c = db();
        pilot(&mut c);
        let (rev, hash) = stricter_draft(&mut c, "dana", None, 50);
        let report = simulation::run(&mut c, "o1", "dana", rev, None, None, None, 60).unwrap().hash();
        // A new push or console edit is another revision and another hash: the old report doesn't cover it.
        let (rev2, hash2) = stricter_draft(&mut c, "dana", Some(&staged(1)), 70);
        assert_ne!(hash, hash2);
        assert_eq!(
            admin::propose(&mut c, "o1", "dana", &activation(rev2, &hash2, Some(&report)), None, 80).unwrap_err(),
            OrgError::Invalid("That simulation is of another draft. Simulate this one.")
        );
        // Nor can the report be paired with the old hash under the new revision.
        assert!(admin::propose(&mut c, "o1", "dana", &activation(rev2, &hash, Some(&report)), None, 80).is_err());
        // A newer push to the same PR supersedes the older draft.
        let mut p: OrgPolicy = policy::active(&mut c, "o1").unwrap().unwrap().0;
        p.explain = "push 1".into();
        let (a, ha) = policy::save_draft_with(
            &mut c,
            "o1",
            "dana",
            &serde_json::to_string(&p).unwrap(),
            Some("pr#7@aaa"),
            None,
            90,
        )
        .unwrap();
        p.explain = "push 2".into();
        policy::save_draft_with(&mut c, "o1", "dana", &serde_json::to_string(&p).unwrap(), Some("pr#7@bbb"), None, 91)
            .unwrap();
        let ra = simulation::run(&mut c, "o1", "dana", a, None, None, None, 92).unwrap().hash();
        assert_eq!(
            admin::propose(&mut c, "o1", "dana", &activation(a, &ha, Some(&ra)), None, 93).unwrap_err(),
            OrgError::Invalid("A newer push replaced this draft.")
        );
    }

    #[test]
    fn drift_blocks_activation() {
        use crate::db::schema::reins_org_requests;
        let mut c = db();
        pilot(&mut c);
        let s = ask(&mut c, &issue("acme/web"), 40);
        // The record says deny, but the active policy (Auto there) doesn't reproduce it: something is off.
        diesel::update(reins_org_requests::table.filter(reins_org_requests::uuid.eq(&s.id)))
            .set(reins_org_requests::outcome.eq("deny"))
            .execute(&mut c)
            .unwrap();
        let (rev, hash) = stricter_draft(&mut c, "dana", None, 50);
        let r = simulation::run(&mut c, "o1", "dana", rev, None, None, None, 60).unwrap();
        assert_eq!(r.drift, 1);
        assert_eq!(
            admin::propose(&mut c, "o1", "dana", &activation(rev, &hash, Some(&r.hash())), None, 61).unwrap_err(),
            OrgError::Invalid("The simulation found drift: the active policy doesn't reproduce history.")
        );
    }

    #[test]
    fn a_sole_security_admin_cannot_activate() {
        use crate::db::models::reins_org::{SUBJECT_MEMBER, q_revoke};
        let mut c = db();
        pilot(&mut c);
        let (rev, hash) = stricter_draft(&mut c, "dana", None, 50);
        let report = simulation::run(&mut c, "o1", "dana", rev, None, None, None, 60).unwrap().hash();
        // With two-person approval off, still two people: the proposer's phone alone never applies it.
        let mut t = reins_org::q_org(&mut c, "o1").unwrap().unwrap();
        t.two_person = false;
        reins_org::q_save_org(&mut c, &t).unwrap();
        let (Proposed::Pending(ch), _) =
            admin::propose(&mut c, "o1", "dana", &activation(rev, &hash, Some(&report)), None, 61).unwrap()
        else {
            panic!("pending");
        };
        let row = reins_org::q_change(&mut c, "o1", &ch).unwrap().unwrap();
        assert!(!admin::can_apply_alone(&mut c, "o1", &row, &activation(rev, &hash, Some(&report))).unwrap());
        assert!(admin::apply_alone(&mut c, "o1", &ch, "dana", 62).is_err());
        // The only Security admin left can't even propose it.
        q_revoke(&mut c, "o1", SUBJECT_MEMBER, "m-kim", Role::SecurityAdmin.as_str()).unwrap();
        assert_eq!(
            admin::propose(&mut c, "o1", "dana", &activation(rev, &hash, Some(&report)), None, 63).unwrap_err(),
            OrgError::Invalid("Activation needs a second Security admin.")
        );
    }

    #[test]
    fn the_proposer_cannot_approve() {
        let mut c = db();
        pilot(&mut c);
        let (rev, hash) = stricter_draft(&mut c, "dana", None, 50);
        let report = simulation::run(&mut c, "o1", "dana", rev, None, None, None, 60).unwrap().hash();
        let (Proposed::Pending(ch), _) =
            admin::propose(&mut c, "o1", "dana", &activation(rev, &hash, Some(&report)), None, 61).unwrap()
        else {
            panic!("pending");
        };
        assert_eq!(
            admin::approve(&mut c, "o1", &ch, "dana", None, 62).unwrap_err(),
            OrgError::Invalid("You can't approve your own change.")
        );
        // A plain member can't either.
        assert_eq!(admin::approve(&mut c, "o1", &ch, "lee", None, 62).unwrap_err(), OrgError::Forbidden);
        admin::approve(&mut c, "o1", &ch, "kim", None, 63).unwrap();
    }

    #[test]
    fn a_family_with_one_guardian_activates() {
        use crate::api::reins::org::kms::tests::use_test_kms;
        let mut c = db();
        use_test_kms();
        crate::api::reins::org::testing::add_org(&mut c, "o1");
        let mut t = reins_org::q_org(&mut c, "o1").unwrap().unwrap();
        FAMILY_TIER.clone_into(&mut t.tier);
        reins_org::q_save_org(&mut c, &t).unwrap();
        member(&mut c, "o1", "mom", &[Role::SecurityAdmin]);
        let (rev, hash) = policy::save_draft(&mut c, "o1", "mom", policy::DEVELOPER_BASELINE, 5).unwrap();
        let (Proposed::Pending(ch), _) =
            admin::propose(&mut c, "o1", "mom", &activation(rev, &hash, None), None, 6).unwrap()
        else {
            panic!("it waits for the guardian's device signature");
        };
        let row = reins_org::q_change(&mut c, "o1", &ch).unwrap().unwrap();
        assert!(admin::can_apply_alone(&mut c, "o1", &row, &activation(rev, &hash, None)).unwrap());
        admin::apply_alone(&mut c, "o1", &ch, "mom", 7).unwrap();
        assert_eq!(policy::active(&mut c, "o1").unwrap().unwrap().0.revision, rev);
        // With a second guardian, it takes both.
        member(&mut c, "o1", "dad", &[Role::SecurityAdmin]);
        assert!(!may_apply_alone(&mut c, "o1").unwrap());
    }

    #[test]
    fn activation_pre_signs_every_stage_and_the_rollback() {
        let mut c = db();
        let stable = pilot(&mut c);
        let (rev, hash) = stricter_draft(&mut c, "dana", Some(&staged(5)), 50);
        let report = simulation::run(&mut c, "o1", "dana", rev, None, None, None, 60).unwrap().hash();
        activate_by_two(&mut c, rev, &hash, &report, 100);
        // The active policy is still the stable one; the draft is the candidate, signed.
        assert_eq!(policy::active(&mut c, "o1").unwrap().unwrap().0.hash(), stable);
        let cand = reins_org_keys::q_policy(&mut c, "o1", i64::try_from(rev).unwrap()).unwrap().unwrap();
        assert_eq!(cand.state, policy_state::CANDIDATE);
        assert!(policy::by_hash(&mut c, "o1", &hash).unwrap().is_some());
        let row = code::q_open_rollout(&mut c, "o1").unwrap().unwrap();
        assert_eq!((row.state.as_str(), row.published_seq, row.stable.as_str()), ("running", 1, stable.as_str()));
        let issuer = kms::signing_key(&mut c, "o1", KeyPurpose::Issuer, 1).unwrap();
        let envs: Vec<Envelope> = serde_json::from_str(&row.envelopes).unwrap();
        assert_eq!(envs.len(), 4);
        let steps: Vec<Rollout> = envs.iter().map(|e| e.parse().unwrap()).collect();
        for (i, (e, s)) in envs.iter().zip(&steps).enumerate() {
            assert!(verify_envelope(e, &[trusted(&issuer)]), "step {i}");
            assert_eq!(e.kind, EnvelopeKind::Rollout);
            assert_eq!(
                (s.seq, s.candidate.as_str(), s.stable.as_str()),
                (u32::try_from(i).unwrap() + 1, hash.as_str(), stable.as_str())
            );
        }
        assert_eq!(steps[0].pilot, ["kim"]);
        assert_eq!(steps.iter().map(|s| s.not_before).collect::<Vec<_>>(), [101, 161, 221, 101]);
        assert!(steps[3].terminal && steps[3].stage.is_none());
        // Only the first stage is published.
        assert_eq!(super::super::rollout::published_envelopes(&row).unwrap(), envs[..1]);
        // A second activation waits for this rollout.
        let (rev3, hash3) = stricter_draft(&mut c, "dana", None, 110);
        let r3 = simulation::run(&mut c, "o1", "dana", rev3, None, None, None, 111).unwrap().hash();
        let (Proposed::Pending(ch), _) =
            admin::propose(&mut c, "o1", "dana", &activation(rev3, &hash3, Some(&r3)), None, 112).unwrap()
        else {
            panic!("pending");
        };
        assert_eq!(
            admin::approve(&mut c, "o1", &ch, "kim", None, 113).unwrap_err(),
            OrgError::Invalid("A rollout is in progress. Let it finish or roll it back first.")
        );
    }

    #[test]
    fn policy_authors_simulate_but_cannot_activate() {
        let mut c = db();
        pilot(&mut c);
        member(&mut c, "o1", "pat", &[Role::PolicyAuthor]);
        let (rev, hash) = stricter_draft(&mut c, "pat", None, 50);
        let report = simulation::run(&mut c, "o1", "pat", rev, None, None, None, 60).unwrap();
        assert_eq!(report.draft_hash, hash);
        assert_eq!(
            admin::propose(&mut c, "o1", "pat", &activation(rev, &hash, Some(&report.hash())), None, 61).unwrap_err(),
            OrgError::Forbidden
        );
        // Nobody else simulates.
        assert_eq!(simulation::run(&mut c, "o1", "lee", rev, None, None, None, 62).unwrap_err(), OrgError::Forbidden);
    }
}
