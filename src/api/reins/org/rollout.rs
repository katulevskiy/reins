//! The staged rollout controller (`docs/design/decisions-v2/05-organizations.md` §7.3): "a pilot group, then 25%, then
//! everyone, with automatic rollback if the ask rate or SLA crosses a set threshold."
//!
//! - Every step was signed at activation ([`super::activation`]). The controller only *publishes* them: the next stage
//!   when the dwell passed with no breach and enough decisions in both cohorts, the rollback when a guard trips. A
//!   stage is never published before its signed `not_before`, and executors ignore one that is early anyway.
//! - [`effective_policy`] is the policy a member is decided under, by [`reins_proto::rollout::effective`] over the
//!   published steps: the very function executors run on the same signed steps.
//! - Guards act only with at least `min_decisions` in both cohorts; a member's cohort is stable for the whole rollout;
//!   a rollback is terminal and the candidate goes back to draft.

use reins_policy::org::OrgPolicy;
use reins_proto::{
    managed::Envelope,
    org::Capability,
    rollout::{Rollout, RolloutPlan, Stage, effective, newest},
};
use serde_json::{Value, json};

use super::{OrgError, audit, policy, require};
use crate::db::{
    DbConnInner,
    models::{
        reins_org,
        reins_org_keys::{self, policy_state},
        reins_org_policy_code::{self as code, ReinsOrgRollout, rollout_state},
        reins_org_request::ReinsOrgRequest,
    },
};

/// Finished rollouts executors are still given (their steps decide old decisions in flight).
pub const RECENT_SECS: i64 = 7 * 86_400;

fn internal(e: impl std::fmt::Display) -> OrgError {
    OrgError::Internal(e.to_string())
}

/// Every step of `row`, signed, in seq order.
pub fn envelopes(row: &ReinsOrgRollout) -> Result<Vec<Envelope>, OrgError> {
    serde_json::from_str(&row.envelopes).map_err(internal)
}

pub fn plan(row: &ReinsOrgRollout) -> Result<RolloutPlan, OrgError> {
    serde_json::from_str(&row.plan).map_err(internal)
}

fn stages_count(row: &ReinsOrgRollout) -> Result<i32, OrgError> {
    i32::try_from(plan(row)?.stages.len()).map_err(internal)
}

/// The published steps of `row`: stages `1..=published_seq`, and the rollback once rolled back.
pub fn published_envelopes(row: &ReinsOrgRollout) -> Result<Vec<Envelope>, OrgError> {
    let n = stages_count(row)?;
    let all = envelopes(row)?;
    Ok(all
        .into_iter()
        .enumerate()
        .filter(|(i, _)| {
            let seq = i32::try_from(*i).unwrap_or(i32::MAX) + 1;
            seq <= row.published_seq.min(n) || (seq == n + 1 && row.state == rollout_state::ROLLED_BACK)
        })
        .map(|(_, e)| e)
        .collect())
}

fn bodies(envs: &[Envelope]) -> Result<Vec<Rollout>, OrgError> {
    envs.iter().map(|e| e.parse::<Rollout>().map_err(internal)).collect()
}

/// The published steps of `org`'s open and recently finished rollouts (what executors are given).
pub fn published_for_org(c: &mut DbConnInner, org: &str, now: i64) -> Result<Vec<Envelope>, OrgError> {
    let mut out = Vec::new();
    for row in code::q_recent_rollouts(c, org, now - RECENT_SECS)? {
        out.extend(published_envelopes(&row)?);
    }
    Ok(out)
}

/// The policy `member` is decided under now, its hash, and the rollout step that chose it (pinned in the decision).
pub fn effective_policy(
    c: &mut DbConnInner,
    org: &str,
    member: &str,
    now: i64,
) -> Result<(OrgPolicy, String, Option<(String, u32)>), OrgError> {
    let (active, _) =
        policy::active(c, org)?.ok_or(OrgError::Invalid("Your organization hasn't activated a policy yet."))?;
    let active_hash = active.hash();
    let steps = bodies(&published_for_org(c, org, now)?)?;
    let hash = effective(&steps, member, &active_hash, now);
    let pin = newest(&steps, now).map(|r| (r.rollout_id.clone(), r.seq));
    if hash == active_hash {
        return Ok((active, hash, pin));
    }
    let env =
        policy::by_hash(c, org, &hash)?.ok_or_else(|| internal(format!("rollout names unsigned policy {hash}")))?;
    let p: OrgPolicy = env.parse().map_err(internal)?;
    if p.hash() != hash {
        return Err(internal(format!("policy {hash} doesn't hash as named")));
    }
    Ok((p, hash, pin))
}

/// One cohort's numbers in a stage.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Cohort {
    pub decisions: u32,
    pub asked: u32,
    /// Percent of decisions asked.
    pub ask_rate: f64,
    /// Median seconds from request to decision over reviewed requests (0 with none).
    pub median_ttd: u64,
}

fn measure<'a>(rows: impl Iterator<Item = &'a ReinsOrgRequest>) -> Cohort {
    let mut n = 0u32;
    let mut asked = 0u32;
    let mut ttd: Vec<i64> = Vec::new();
    for r in rows {
        n += 1;
        if r.outcome == "review" {
            asked += 1;
            if let Some(d) = r.decided_at {
                ttd.push((d - r.created_at).max(0));
            }
        }
    }
    ttd.sort_unstable();
    Cohort {
        decisions: n,
        asked,
        ask_rate: if n == 0 {
            0.0
        } else {
            f64::from(asked) * 100.0 / f64::from(n)
        },
        median_ttd: ttd.get(ttd.len() / 2).map_or(0, |t| u64::try_from(*t).unwrap_or(0)),
    }
}

/// What the guards see now: (candidate, stable). In the last stage nobody is on stable, so the stable cohort is the
/// whole rollout's.
pub fn numbers(c: &mut DbConnInner, row: &ReinsOrgRollout) -> Result<(Cohort, Cohort), OrgError> {
    let stage_rows = code::q_rollout_requests(c, &row.org_uuid, &row.rollout_id, row.stage_started_at)?;
    let candidate = measure(stage_rows.iter().filter(|r| r.snapshot == row.candidate));
    let mut stable = measure(stage_rows.iter().filter(|r| r.snapshot == row.stable));
    if stable.decisions == 0 {
        let all = code::q_rollout_requests(c, &row.org_uuid, &row.rollout_id, row.created_at)?;
        stable = measure(all.iter().filter(|r| r.snapshot == row.stable));
    }
    Ok((candidate, stable))
}

/// What a tick decided.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// Waiting: for the dwell, or for this many more decisions.
    Wait {
        more_decisions: u32,
    },
    Advanced {
        seq: i32,
    },
    Completed,
    /// Rolled back: who to tell (Security admins).
    RolledBack {
        notify: Vec<String>,
    },
}

/// One rollout's tick (see the module docs).
pub fn tick(c: &mut DbConnInner, row: &ReinsOrgRollout, now: i64) -> Result<Step, OrgError> {
    if row.state != rollout_state::RUNNING {
        return Ok(Step::Wait {
            more_decisions: 0,
        });
    }
    let p = plan(row)?;
    let g = p.guards;
    let (cand, stable) = numbers(c, row)?;
    let enough = cand.decisions >= g.min_decisions && stable.decisions >= g.min_decisions;
    if enough {
        let ask = cand.ask_rate - stable.ask_rate > g.max_ask_rate_delta_pp;
        let slow = cand.median_ttd > g.max_median_ttd_secs;
        if ask || slow {
            let why = if ask {
                "ask rate"
            } else {
                "time to decision"
            };
            let detail = json!({"why": why, "candidate": cohort_json(&cand), "stable": cohort_json(&stable),
                                "guards": {"max_ask_rate_delta_pp": g.max_ask_rate_delta_pp,
                                           "max_median_ttd_secs": g.max_median_ttd_secs,
                                           "min_decisions": g.min_decisions}});
            rollback(c, row, None, &detail, now)?;
            return Ok(Step::RolledBack {
                notify: reins_org::q_holders(c, &row.org_uuid, Capability::ActivatePolicy)?,
            });
        }
    }
    let dwell = i64::try_from(p.dwell_secs).unwrap_or(i64::MAX);
    if now.saturating_sub(row.stage_started_at) < dwell {
        return Ok(Step::Wait {
            more_decisions: 0,
        });
    }
    if !enough {
        return Ok(Step::Wait {
            more_decisions: g.min_decisions.saturating_sub(cand.decisions.min(stable.decisions)),
        });
    }
    advance(c, row, None, now)
}

fn cohort_json(c: &Cohort) -> Value {
    json!({"decisions": c.decisions, "asked": c.asked, "ask_rate": (c.ask_rate * 10.0).round() / 10.0,
           "median_ttd_secs": c.median_ttd})
}

/// Publishes the next stage, or completes after the last; never a stage before its signed `not_before`.
pub fn advance(c: &mut DbConnInner, row: &ReinsOrgRollout, actor: Option<&str>, now: i64) -> Result<Step, OrgError> {
    let n = stages_count(row)?;
    if row.published_seq >= n {
        complete(c, row, actor, now)?;
        return Ok(Step::Completed);
    }
    let next = row.published_seq + 1;
    let steps = bodies(&envelopes(row)?)?;
    let step = steps.get(usize::try_from(next - 1).unwrap_or(usize::MAX)).ok_or_else(|| internal("missing step"))?;
    if step.not_before > now {
        return Err(OrgError::Invalid("The next stage isn't due yet."));
    }
    let mut moved = row.clone();
    moved.published_seq = next;
    moved.stage_started_at = now;
    moved.updated_at = now;
    if !code::q_save_rollout(c, &moved, &row.state)? {
        return Err(OrgError::Invalid("This rollout changed meanwhile."));
    }
    audit::record(
        c,
        &row.org_uuid,
        audit::kinds::POLICY_STAGE_ADVANCED,
        actor,
        Some(&row.rollout_id),
        &json!({"rollout_id": row.rollout_id, "seq": next, "stage": step.stage.as_ref().map(Stage::label),
                "candidate": row.candidate}),
        now,
    )?;
    Ok(Step::Advanced {
        seq: next,
    })
}

/// The last stage ran cleanly: the candidate becomes the active policy and the previous one retires.
pub fn complete(c: &mut DbConnInner, row: &ReinsOrgRollout, actor: Option<&str>, now: i64) -> Result<(), OrgError> {
    let mut done = row.clone();
    done.state = rollout_state::COMPLETE.to_owned();
    done.updated_at = now;
    if !code::q_save_rollout(c, &done, &row.state)? {
        return Err(OrgError::Invalid("This rollout changed meanwhile."));
    }
    code::q_retire_active(c, &row.org_uuid)?;
    if !code::q_set_policy_state(
        c,
        &row.org_uuid,
        row.revision,
        &[policy_state::CANDIDATE],
        policy_state::ACTIVE,
        None,
        None,
    )? {
        return Err(internal(format!("rollout {} has no candidate revision {}", row.rollout_id, row.revision)));
    }
    audit::record(
        c,
        &row.org_uuid,
        audit::kinds::POLICY_ROLLOUT_COMPLETED,
        actor,
        Some(&row.rollout_id),
        &json!({"rollout_id": row.rollout_id, "revision": row.revision, "active": row.candidate, "retired": row.stable}),
        now,
    )?;
    Ok(())
}

/// Publishes the rollback (terminal): everyone is back on the stable policy, and the candidate is a draft again.
pub fn rollback(
    c: &mut DbConnInner,
    row: &ReinsOrgRollout,
    actor: Option<&str>,
    why: &Value,
    now: i64,
) -> Result<(), OrgError> {
    if ![rollout_state::RUNNING, rollout_state::PAUSED].contains(&row.state.as_str()) {
        return Err(OrgError::Invalid("This rollout is over."));
    }
    let mut back = row.clone();
    back.state = rollout_state::ROLLED_BACK.to_owned();
    back.reason = Some(why.to_string());
    back.updated_at = now;
    if !code::q_save_rollout(c, &back, &row.state)? {
        return Err(OrgError::Invalid("This rollout changed meanwhile."));
    }
    code::q_set_policy_state(
        c,
        &row.org_uuid,
        row.revision,
        &[policy_state::CANDIDATE],
        policy_state::DRAFT,
        None,
        None,
    )?;
    audit::record(
        c,
        &row.org_uuid,
        audit::kinds::POLICY_ROLLED_BACK,
        actor,
        Some(&row.rollout_id),
        &json!({"rollout_id": row.rollout_id, "revision": row.revision, "candidate": row.candidate,
                "stable": row.stable, "seq": stages_count(row)? + 1, "why": why}),
        now,
    )?;
    Ok(())
}

/// Every running rollout's tick (the sweeper); the Security admins to tell about rollbacks.
pub fn tick_all(c: &mut DbConnInner, now: i64) -> Result<Vec<String>, OrgError> {
    let mut notify = Vec::new();
    for row in code::q_running_rollouts(c)? {
        match tick(c, &row, now) {
            Ok(Step::RolledBack {
                notify: users,
            }) => notify.extend(users),
            Ok(_) => {}
            Err(e) => warn!("Rollout {} of {}: {e:?}", row.rollout_id, row.org_uuid),
        }
    }
    notify.sort();
    notify.dedup();
    Ok(notify)
}

/// A console control on a rollout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    Pause,
    Resume,
    Advance,
    Rollback,
}

impl Control {
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "pause" => Some(Self::Pause),
            "resume" => Some(Self::Resume),
            "advance" => Some(Self::Advance),
            "rollback" => Some(Self::Rollback),
            _ => None,
        }
    }
}

/// Pause, resume, advance or roll back a rollout, by one holder of [`Capability::ActivatePolicy`]: its stages were
/// approved at activation, and a rollback is always allowed. Each is audited.
pub fn control(c: &mut DbConnInner, org: &str, actor: &str, id: &str, what: Control, now: i64) -> Result<(), OrgError> {
    require(c, org, actor, Capability::ActivatePolicy)?;
    let row = code::q_rollout(c, org, id)?.ok_or(OrgError::NotFound)?;
    match what {
        Control::Rollback => rollback(c, &row, Some(actor), &json!({"why": "rolled back by hand"}), now),
        Control::Advance => {
            if row.state != rollout_state::RUNNING {
                return Err(OrgError::Invalid("Resume the rollout first."));
            }
            advance(c, &row, Some(actor), now).map(drop)
        }
        Control::Pause | Control::Resume => {
            let (from, to, kind) = if what == Control::Pause {
                (rollout_state::RUNNING, rollout_state::PAUSED, audit::kinds::POLICY_ROLLOUT_PAUSED)
            } else {
                (rollout_state::PAUSED, rollout_state::RUNNING, audit::kinds::POLICY_ROLLOUT_RESUMED)
            };
            if row.state != from {
                return Err(OrgError::Invalid("The rollout isn't in that state."));
            }
            let mut moved = row.clone();
            to.clone_into(&mut moved.state);
            moved.updated_at = now;
            if !code::q_save_rollout(c, &moved, from)? {
                return Err(OrgError::Invalid("This rollout changed meanwhile."));
            }
            audit::record(c, org, kind, Some(actor), Some(id), &json!({"rollout_id": id}), now).map(drop)
        }
    }
}

/// The revision's policy hash and state, for display.
pub fn revision_state(c: &mut DbConnInner, org: &str, revision: i64) -> Result<Option<(String, String)>, OrgError> {
    Ok(reins_org_keys::q_policy(c, org, revision)?.map(|r| (r.hash, r.state)))
}

#[cfg(all(test, sqlite))]
mod tests {
    use super::*;

    #[test]
    fn measure_counts_asks_and_median() {
        let mk = |outcome: &str, created: i64, decided: Option<i64>| {
            let mut r = crate::db::models::reins_org_request::tests::request("o1", "or_x", "approved");
            r.outcome = outcome.to_owned();
            r.created_at = created;
            r.decided_at = decided;
            r
        };
        let rows =
            [mk("auto", 0, Some(0)), mk("review", 0, Some(10)), mk("review", 0, Some(30)), mk("review", 0, None)];
        let m = measure(rows.iter());
        assert_eq!((m.decisions, m.asked, m.median_ttd), (4, 3, 30));
        assert!((m.ask_rate - 75.0).abs() < 1e-9);
        assert_eq!(measure(std::iter::empty()).ask_rate, 0.0);
    }
}
