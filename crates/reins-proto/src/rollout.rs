//! Staged policy rollouts (Decisions v2 · O4, `docs/design/decisions-v2/05-organizations.md` §7.3): "a pilot group,
//! then 25%, then everyone, with automatic rollback if the ask rate or SLA crosses a set threshold."
//!
//! - At activation the org issuer key pre-signs every stage of a rollout, and its rollback, as [`Rollout`] bodies in
//!   [`crate::managed::EnvelopeKind::Rollout`] envelopes. The decision service publishes them in order: the next stage
//!   when the dwell passes cleanly, the rollback when a guard trips.
//! - [`effective`] names the policy snapshot a member is decided under: the newest rollout's candidate when the member
//!   is in its cohort, its stable snapshot otherwise, and the active snapshot when there is no rollout. The decision
//!   service and every executor compute it with this same function from the same signed bodies, so a decision service
//!   can't pick the looser version for a member on its own.
//! - Cohorts are stable: a member's bucket is a hash of (rollout, member), percentages nest, and the pilot group stays in
//!   the cohort for every later stage. A rollback is terminal and wins over every stage.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::canon::sha256;
use crate::managed::canonical_json;

/// One stage of a rollout.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    /// The pilot group: these members (user ids, sorted), resolved when the rollout is activated.
    Pilot {
        members: Vec<String>,
    },
    /// This share of members (0–100), by a stable hash of (rollout, member).
    Percent(u8),
    /// Everyone.
    All,
}

impl Stage {
    /// Short words for people ("Pilot", "25%", "Everyone").
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::Pilot {
                ..
            } => "Pilot".to_owned(),
            Self::Percent(p) => format!("{p}%"),
            Self::All => "Everyone".to_owned(),
        }
    }
}

/// When a stage rolls back by itself, and how much evidence it takes to act at all.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Guards {
    /// The candidate's ask rate may exceed the stable one's by at most this many percentage points.
    pub max_ask_rate_delta_pp: f64,
    /// The candidate's median time to decision (reviewed requests) may be at most this long.
    pub max_median_ttd_secs: u64,
    /// Neither cohort is judged (no rollback, no advance) with fewer decisions than this.
    pub min_decisions: u32,
}

impl Default for Guards {
    fn default() -> Self {
        Self {
            max_ask_rate_delta_pp: 2.0,
            max_median_ttd_secs: 1800,
            min_decisions: 50,
        }
    }
}

/// The default dwell per stage: a day.
pub const DEFAULT_DWELL_SECS: u64 = 86_400;

const fn default_dwell() -> u64 {
    DEFAULT_DWELL_SECS
}

/// What two Security admins approve with a policy: its stages, how long each must run cleanly, and its guards.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RolloutPlan {
    pub stages: Vec<Stage>,
    #[serde(default = "default_dwell")]
    pub dwell_secs: u64,
    #[serde(default)]
    pub guards: Guards,
}

impl Default for RolloutPlan {
    fn default() -> Self {
        Self {
            stages: Vec::new(),
            dwell_secs: DEFAULT_DWELL_SECS,
            guards: Guards::default(),
        }
    }
}

impl RolloutPlan {
    /// Refuses a plan that can't be followed: percentages out of order or out of range, a pilot anywhere but first,
    /// anything after "everyone", or a plan that never reaches everyone.
    pub fn validate(&self) -> Result<(), String> {
        if self.stages.is_empty() {
            return Ok(());
        }
        let mut last = 0u8;
        for (i, s) in self.stages.iter().enumerate() {
            match s {
                Stage::Pilot {
                    ..
                } if i != 0 => return Err("the pilot stage comes first".to_owned()),
                Stage::Percent(p) => {
                    if *p == 0 || *p >= 100 || *p <= last {
                        return Err("percent stages grow from 1% to 99%".to_owned());
                    }
                    last = *p;
                }
                Stage::All if i + 1 != self.stages.len() => return Err("everyone is the last stage".to_owned()),
                Stage::Pilot {
                    ..
                }
                | Stage::All => {}
            }
        }
        if self.stages.last() != Some(&Stage::All) {
            return Err("a rollout ends with everyone (100%)".to_owned());
        }
        if !(self.guards.max_ask_rate_delta_pp.is_finite() && self.guards.max_ask_rate_delta_pp >= 0.0) {
            return Err("the ask rate guard is a number of percentage points".to_owned());
        }
        Ok(())
    }

    /// The pilot members, when the plan starts with a pilot stage.
    #[must_use]
    pub fn pilot(&self) -> Vec<String> {
        match self.stages.first() {
            Some(Stage::Pilot {
                members,
            }) => members.clone(),
            _ => Vec::new(),
        }
    }
}

/// One signed step of a rollout: a stage published, or the rollback (`stage: None`, `terminal`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rollout {
    pub org: String,
    pub rollout_id: String,
    /// The org's rollouts in the order they were activated: the newest one decides.
    pub epoch: u64,
    /// Stages are `1..=n`; the rollback is `n + 1`.
    pub seq: u32,
    /// The policy hash members outside the cohort stay on.
    pub stable: String,
    /// The policy hash being rolled out.
    pub candidate: String,
    /// The pilot group (sorted user ids): in the cohort at every stage.
    #[serde(default)]
    pub pilot: Vec<String>,
    /// `None`: rolled back.
    pub stage: Option<Stage>,
    /// A rollback: final, and it wins over every stage of the same rollout.
    pub terminal: bool,
}

impl Rollout {
    /// The canonical JSON an issuer signs as the envelope body.
    #[must_use]
    pub fn body(&self) -> String {
        canonical_json(&serde_json::to_value(self).unwrap_or_default())
    }

    /// Whether `member` is in this step's cohort (never after a rollback).
    #[must_use]
    pub fn includes(&self, member: &str) -> bool {
        match &self.stage {
            None => false,
            Some(stage) => {
                self.pilot.binary_search_by(|m| m.as_str().cmp(member)).is_ok()
                    || in_cohort(&self.rollout_id, member, stage)
            }
        }
    }

    /// The policy hash `member` is decided under at this step.
    #[must_use]
    pub fn hash_for(&self, member: &str) -> &str {
        if self.includes(member) {
            &self.candidate
        } else {
            &self.stable
        }
    }

    /// Every step of a rollout for `plan`, pre-signed at activation: the stages as `1..=n` and the rollback as `n + 1`.
    #[must_use]
    pub fn steps(
        org: &str,
        rollout_id: &str,
        epoch: u64,
        stable: &str,
        candidate: &str,
        plan: &RolloutPlan,
    ) -> Vec<Self> {
        let mut pilot = plan.pilot();
        pilot.sort();
        pilot.dedup();
        let step = |seq: usize, stage: Option<Stage>| Self {
            org: org.to_owned(),
            rollout_id: rollout_id.to_owned(),
            epoch,
            seq: u32::try_from(seq).unwrap_or(u32::MAX),
            stable: stable.to_owned(),
            candidate: candidate.to_owned(),
            pilot: pilot.clone(),
            terminal: stage.is_none(),
            stage,
        };
        let mut out: Vec<Self> = plan.stages.iter().enumerate().map(|(i, s)| step(i + 1, Some(s.clone()))).collect();
        out.push(step(plan.stages.len() + 1, None));
        out
    }
}

/// `member`'s bucket in `rollout_id`, 0–99: the first two bytes of SHA-256(rollout id ‖ 0x00 ‖ member), mod 100.
#[must_use]
pub fn bucket(rollout_id: &str, member: &str) -> u8 {
    let mut bytes = Vec::with_capacity(rollout_id.len() + 1 + member.len());
    bytes.extend_from_slice(rollout_id.as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(member.as_bytes());
    let h = sha256(&bytes);
    u8::try_from(u16::from_be_bytes([h[0], h[1]]) % 100).unwrap_or(99)
}

/// Whether `member` is in `stage` of `rollout_id`: pilot membership, a bucket under the percentage, or everyone.
#[must_use]
pub fn in_cohort(rollout_id: &str, member: &str, stage: &Stage) -> bool {
    match stage {
        Stage::Pilot {
            members,
        } => members.iter().any(|m| m == member),
        Stage::Percent(p) => bucket(rollout_id, member) < *p,
        Stage::All => true,
    }
}

/// Each rollout's current step: its rollback when it has one (whatever the seqs), else its highest seq.
#[must_use]
pub fn current_steps(rollouts: &[Rollout]) -> Vec<&Rollout> {
    let mut by_id: BTreeMap<&str, &Rollout> = BTreeMap::new();
    for r in rollouts {
        let keep = match by_id.get(r.rollout_id.as_str()) {
            None => true,
            Some(cur) => match (cur.terminal, r.terminal) {
                (true, _) => false,
                (false, true) => true,
                (false, false) => r.seq > cur.seq,
            },
        };
        if keep {
            by_id.insert(&r.rollout_id, r);
        }
    }
    by_id.into_values().collect()
}

/// The newest rollout's current step (highest epoch, then rollout id for a deterministic tie).
#[must_use]
pub fn newest(rollouts: &[Rollout]) -> Option<&Rollout> {
    current_steps(rollouts)
        .into_iter()
        .max_by(|a, b| a.epoch.cmp(&b.epoch).then_with(|| a.rollout_id.cmp(&b.rollout_id)))
}

/// The policy hash `member` is decided under: the newest rollout's candidate when the member is in its cohort, its
/// stable hash otherwise (and after a rollback), and `active` when there is no rollout.
#[must_use]
pub fn effective(rollouts: &[Rollout], member: &str, active: &str) -> String {
    newest(rollouts).map_or_else(|| active.to_owned(), |r| r.hash_for(member).to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(stages: Vec<Stage>) -> RolloutPlan {
        RolloutPlan {
            stages,
            ..RolloutPlan::default()
        }
    }

    fn standard() -> Vec<Rollout> {
        let p = plan(vec![
            Stage::Pilot {
                members: vec!["u-pilot".into()],
            },
            Stage::Percent(25),
            Stage::All,
        ]);
        Rollout::steps("o1", "ro_1", 1, "S", "C", &p)
    }

    fn members(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("user-{i}")).collect()
    }

    #[test]
    fn cohorts_are_stable() {
        for m in members(200) {
            assert_eq!(bucket("ro_1", &m), bucket("ro_1", &m));
            assert_eq!(in_cohort("ro_1", &m, &Stage::Percent(25)), in_cohort("ro_1", &m, &Stage::Percent(25)));
        }
        // Another rollout draws other cohorts.
        let differ = members(200).iter().filter(|m| bucket("ro_1", m) != bucket("ro_2", m)).count();
        assert!(differ > 150, "{differ}");
        // About a quarter at 25%.
        let in25 = members(2000).iter().filter(|m| in_cohort("ro_1", m, &Stage::Percent(25))).count();
        assert!((400..600).contains(&in25), "{in25}");
    }

    #[test]
    fn percent_cohorts_nest() {
        for m in members(500) {
            for (lo, hi) in [(1, 25), (25, 50), (50, 99)] {
                if in_cohort("ro_1", &m, &Stage::Percent(lo)) {
                    assert!(in_cohort("ro_1", &m, &Stage::Percent(hi)), "{m} {lo} {hi}");
                }
            }
            assert!(in_cohort("ro_1", &m, &Stage::All));
            assert!(!in_cohort("ro_1", &m, &Stage::Percent(0)));
        }
        // The pilot stays in at every later stage.
        let steps = standard();
        assert!(steps[..3].iter().all(|s| s.includes("u-pilot")));
    }

    #[test]
    fn pilot_is_exact_membership() {
        let pilot = Stage::Pilot {
            members: vec!["u-a".into(), "u-b".into()],
        };
        assert!(in_cohort("ro_1", "u-a", &pilot));
        assert!(!in_cohort("ro_1", "u-ab", &pilot));
        assert!(!in_cohort("ro_1", "U-A", &pilot));
        let steps = standard();
        assert_eq!(effective(&steps[..1], "u-pilot", "S"), "C");
        assert!(members(300).iter().all(|m| effective(&steps[..1], m, "S") == "S"));
    }

    #[test]
    fn rollback_supersedes_every_stage() {
        let steps = standard();
        assert_eq!(steps.len(), 4);
        assert_eq!((steps[3].seq, steps[3].terminal, steps[3].stage.as_ref()), (4, true, None));
        // Everyone at 100%, until the rollback is published: then everyone is back on stable.
        assert!(members(50).iter().all(|m| effective(&steps[..3], m, "S") == "C"));
        assert!(members(50).iter().all(|m| effective(&steps, m, "S") == "S"));
        assert_eq!(effective(&steps, "u-pilot", "S"), "S");
        // Whatever order they arrive in.
        let mut shuffled = steps;
        shuffled.reverse();
        assert_eq!(effective(&shuffled, "u-pilot", "S"), "S");
        // No rollout: the active policy.
        assert_eq!(effective(&[], "u-pilot", "A"), "A");
    }

    #[test]
    fn a_stage_after_rollback_is_ignored() {
        let steps = standard();
        let mut later = steps[2].clone();
        later.seq = 9;
        let set = vec![steps[0].clone(), steps[3].clone(), later];
        assert!(members(50).iter().all(|m| effective(&set, m, "S") == "S"));
        // A newer rollout (higher epoch) decides; an older one is ignored.
        let p = plan(vec![Stage::All]);
        let next = Rollout::steps("o1", "ro_2", 2, "S", "D", &p);
        let mut all = steps;
        all.push(next[0].clone());
        assert_eq!(effective(&all, "u-x", "S"), "D");
    }

    #[test]
    fn bodies_are_canonical() {
        let r = &standard()[1];
        let body = r.body();
        assert!(
            body.starts_with(
                r#"{"candidate":"C","epoch":1,"org":"o1","pilot":["u-pilot"],"rollout_id":"ro_1","seq":2,"#
            )
        );
        assert_eq!(serde_json::from_str::<Rollout>(&body).unwrap(), *r);
        assert_eq!(body, canonical_json(&serde_json::from_str(&body).unwrap()));
        assert!(body.contains(r#""stage":{"percent":25}"#), "{body}");
    }

    #[test]
    fn plans_must_be_followable() {
        assert!(plan(vec![]).validate().is_ok());
        assert!(standard().is_empty() || plan(vec![Stage::Percent(25), Stage::All]).validate().is_ok());
        assert!(plan(vec![Stage::Percent(25)]).validate().is_err());
        assert!(plan(vec![Stage::Percent(50), Stage::Percent(25), Stage::All]).validate().is_err());
        assert!(plan(vec![Stage::All, Stage::Percent(25)]).validate().is_err());
        assert!(
            plan(vec![
                Stage::Percent(25),
                Stage::Pilot {
                    members: vec![]
                },
                Stage::All
            ])
            .validate()
            .is_err()
        );
        assert!(plan(vec![Stage::Percent(100), Stage::All]).validate().is_err());
        let defaults: RolloutPlan = serde_json::from_str(r#"{"stages":["all"]}"#).unwrap();
        assert_eq!((defaults.dwell_secs, defaults.guards), (DEFAULT_DWELL_SECS, Guards::default()));
    }
}
