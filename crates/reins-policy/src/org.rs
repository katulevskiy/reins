//! Managed mode's evaluation (`docs/design/decisions-v2/05-organizations.md` §6–§7, `02-policy-and-authority.md` §3,
//! `08-review-resolutions.md` R3, R5): the org's decision service and its executors both run [`evaluate_managed`], so
//! an executor re-checks every decision with the same code against the same issuer-signed [`OrgPolicy`].
//!
//! - The org's rules are hard: an org `deny` denies, an org `review` names who must approve ([`ReviewSpec`]).
//! - The member's own policy only tightens (02 §3.4: a Managed profile has no "owner permits wider than org"): their
//!   matrix makes cells stricter, their deny and review rules add up, and their permits are dropped.
//! - The always-ask floor, certainty `None` and L3 stay hard reviews by the requester (08 R4).
//! - A soft Ask cell is reviewed as the org says ([`OrgPolicy::cell_review`], the requester by default).
//!
//! Pure and deterministic: the same request, policy and member policy always give the same [`ManagedVerdict`].

use std::collections::BTreeMap;

use reins_proto::org::Role;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::context::{Base, Facts, GITHUB, Resolved, Zone, parse_repo, repo_zone, request_zone};
use crate::effects::{Analysis, Certainty, Effect, Level, analyze_tool};
use crate::matrix::{self, Cell, Evaluation, Input, Issuer, Matrix, Outcome, Rule, Snapshot, Then, Why};

/// `version` of an [`OrgPolicy`].
pub const ORG_POLICY_VERSION: u32 = 1;
/// Instant reviews: default and longest window (03 §1).
pub const INSTANT_DEFAULT_SECS: u64 = 120;
pub const INSTANT_MAX_SECS: u64 = 600;
/// Deferred reviews: default and longest window (03 §1).
pub const DEFERRED_DEFAULT_SECS: u64 = 4 * 3600;
pub const DEFERRED_MAX_SECS: u64 = 7 * 24 * 3600;
/// The tool a credential lease is evaluated as (`reins_proto::managed::LEASE_TOOL`).
pub const LEASE_TOOL: &str = "git_lease";

/// Who satisfies a review.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reviewers {
    /// The member who asked (their own review, on their enrolled phone).
    Requester,
    /// Any member of the group (its id).
    Group(String),
    /// Any member holding the role.
    Role(Role),
    /// One of these members (user ids).
    Members(Vec<String>),
}

/// Instant or Deferred (03 §1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewClass {
    Instant,
    Deferred,
}

const fn one() -> u8 {
    1
}

const fn yes() -> bool {
    true
}

/// A review obligation (02 §2's `review { reviewers, quorum, distinct_persons, exclude_requester, class, window }`).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ReviewSpec {
    pub reviewers: Reviewers,
    #[serde(default = "one")]
    pub quorum: u8,
    /// One person satisfies at most one obligation.
    #[serde(default = "yes")]
    pub distinct_persons: bool,
    /// No self-approval (05 §6).
    #[serde(default = "yes")]
    pub exclude_requester: bool,
    pub class: ReviewClass,
    /// Seconds; 0 is the class default.
    #[serde(default)]
    pub window_secs: u64,
}

impl ReviewSpec {
    /// The requester's own review: Instant, two minutes (08 R12).
    #[must_use]
    pub fn owner() -> Self {
        Self {
            reviewers: Reviewers::Requester,
            quorum: 1,
            distinct_persons: true,
            exclude_requester: false,
            class: ReviewClass::Instant,
            window_secs: INSTANT_DEFAULT_SECS,
        }
    }

    /// The window in seconds, the class default when unset.
    #[must_use]
    pub fn window(&self) -> u64 {
        match (self.class, self.window_secs) {
            (ReviewClass::Instant, 0) => INSTANT_DEFAULT_SECS,
            (ReviewClass::Deferred, 0) => DEFERRED_DEFAULT_SECS,
            (_, w) => w,
        }
    }

    fn validate(&self) -> Result<(), String> {
        if self.quorum == 0 {
            return Err("a review needs a quorum of at least 1".to_owned());
        }
        let max = match self.class {
            ReviewClass::Instant => INSTANT_MAX_SECS,
            ReviewClass::Deferred => DEFERRED_MAX_SECS,
        };
        if self.window_secs > max {
            return Err(format!("a {:?} review's window is at most {max} seconds", self.class));
        }
        if let Reviewers::Members(m) = &self.reviewers
            && m.is_empty()
        {
            return Err("a review by members names at least one".to_owned());
        }
        Ok(())
    }

    /// Whether `member` (with these groups and roles) may satisfy this review for `requester`.
    #[must_use]
    pub fn eligible(&self, member: &str, groups: &[String], roles: &[Role], requester: &str) -> bool {
        if self.exclude_requester && member == requester && self.reviewers != Reviewers::Requester {
            return false;
        }
        match &self.reviewers {
            Reviewers::Requester => member == requester,
            Reviewers::Group(g) => groups.iter().any(|x| x == g),
            Reviewers::Role(r) => roles.contains(r),
            Reviewers::Members(list) => list.iter().any(|x| x == member),
        }
    }
}

/// An org rule: P2's [`Rule`] (issued by the org), with who reviews when it says `review`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrgRule {
    #[serde(flatten)]
    pub rule: Rule,
    #[serde(default)]
    pub review: Option<ReviewSpec>,
}

/// What a device must show to decide (06 §6). Checked by the decision service; executors trust the signed roster's
/// `may_decide`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PostureRule {
    pub screen_lock: bool,
    /// Refuse rooted or jailbroken devices, and devices that couldn't tell.
    pub no_root: bool,
    /// Require a verified platform attestation.
    pub attested: bool,
    /// Minimum OS version per OS name (`iOS`, `Android`).
    pub min_os: BTreeMap<String, String>,
    pub min_app: Option<String>,
    /// A posture older than this no longer counts.
    pub max_age_days: u32,
}

impl Default for PostureRule {
    fn default() -> Self {
        Self {
            screen_lock: true,
            no_root: true,
            attested: false,
            min_os: BTreeMap::new(),
            min_app: None,
            max_age_days: 30,
        }
    }
}

/// An org's policy snapshot: what the org issuer key signs and every Managed decision names by [`OrgPolicy::hash`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrgPolicy {
    pub version: u32,
    /// The org id: every rule is issued by `Issuer::Org(org)`.
    pub org: String,
    #[serde(default)]
    pub revision: u64,
    #[serde(default)]
    pub matrix: Matrix,
    #[serde(default)]
    pub rules: Vec<OrgRule>,
    /// The org's verified facts (05 §7.1: from verified sources, never from the agent).
    #[serde(default)]
    pub facts: Facts,
    /// Who reviews a soft Ask cell.
    #[serde(default = "ReviewSpec::owner")]
    pub cell_review: ReviewSpec,
    #[serde(default)]
    pub posture: PostureRule,
    #[serde(default)]
    pub explain: String,
}

fn id_ok(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'@' | b'-'))
}

impl OrgPolicy {
    /// An empty policy for `org` (the default matrix, no rules).
    #[must_use]
    pub fn empty(org: &str) -> Self {
        Self {
            version: ORG_POLICY_VERSION,
            org: org.to_owned(),
            revision: 0,
            matrix: Matrix::default(),
            rules: Vec::new(),
            facts: Facts::default(),
            cell_review: ReviewSpec::owner(),
            posture: PostureRule::default(),
            explain: String::new(),
        }
    }

    /// Hex SHA-256 of the policy's JSON (field order fixed by the types, maps ordered).
    #[must_use]
    pub fn hash(&self) -> String {
        let json = serde_json::to_vec(self).unwrap_or_default();
        data_encoding::HEXLOWER.encode(&Sha256::digest(&json))
    }

    /// Refuses a policy that can't be what the org meant: another issuer, duplicate or malformed ids, a review rule
    /// without reviewers (or reviewers on a rule that doesn't review), a bad window or quorum.
    pub fn validate(&self) -> Result<(), String> {
        if self.version != ORG_POLICY_VERSION {
            return Err(format!("policy version {} is not supported", self.version));
        }
        let mut ids: Vec<&str> = Vec::new();
        for r in &self.rules {
            let id = r.rule.id.as_str();
            if !id_ok(id) {
                return Err(format!("rule id {id:?} must be 1-64 of a-z 0-9 . _ @ -"));
            }
            if ids.contains(&id) {
                return Err(format!("rule id {id} is used twice"));
            }
            ids.push(id);
            if r.rule.issuer != Issuer::Org(self.org.clone()) {
                return Err(format!("rule {id} must be issued by this organization"));
            }
            match (r.rule.then, &r.review) {
                (Then::Review, Some(spec)) => spec.validate().map_err(|e| format!("rule {id}: {e}"))?,
                (Then::Review, None) => return Err(format!("rule {id} reviews but names no reviewers")),
                (_, Some(_)) => return Err(format!("rule {id} names reviewers but doesn't review")),
                (_, None) => {}
            }
        }
        self.cell_review.validate().map_err(|e| format!("cell review: {e}"))
    }
}

/// A member's own stricter policy (their matrix and their deny/review rules). Permits are ignored.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MemberPolicy {
    pub matrix: Option<Matrix>,
    pub rules: Vec<Rule>,
}

impl MemberPolicy {
    #[must_use]
    pub fn hash(&self) -> String {
        let json = serde_json::to_vec(self).unwrap_or_default();
        data_encoding::HEXLOWER.encode(&Sha256::digest(&json))
    }
}

/// One Managed request.
#[derive(Clone, Copy, Debug)]
pub struct ManagedRequest<'a> {
    pub org: &'a str,
    /// The requester's user id.
    pub member: &'a str,
    pub connection: &'a str,
    pub tool: &'a str,
    pub args: &'a Value,
    /// When it is judged (rules' `not_after`).
    pub now: i64,
}

/// What policy says.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedOutcome {
    Auto,
    Review,
    Deny,
}

impl ManagedOutcome {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Review => "review",
            Self::Deny => "deny",
        }
    }
}

/// One review that must be satisfied.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Obligation {
    /// `cell`, `floor`, `level3`, `unreadable`, or a rule id (the first, when several asked the same reviewers).
    pub id: String,
    pub spec: ReviewSpec,
    /// Every reason it is required.
    pub why: Vec<String>,
}

/// The decision and its trace.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagedVerdict {
    pub outcome: ManagedOutcome,
    pub obligations: Vec<Obligation>,
    /// P2's evaluation of the composed snapshot (zone, level, why, the composed snapshot's hash).
    pub evaluation: Evaluation,
    /// The org policy's hash ([`OrgPolicy::hash`]).
    pub snapshot: String,
    /// The member policy's hash, when one applied.
    pub member_policy: Option<String>,
    /// The deciding org rule's words (a deny's reason).
    pub explain: String,
    /// The plain lines of what the request does.
    pub lines: Vec<String>,
}

impl ManagedVerdict {
    /// The obligation ids.
    #[must_use]
    pub fn obligation_ids(&self) -> Vec<String> {
        self.obligations.iter().map(|o| o.id.clone()).collect()
    }
}

/// What a lease of `access` to `repo` amounts to: a read, or push access (which can lose work, so `lost`).
#[must_use]
pub fn lease_analysis(repo: &str, access: &str) -> Analysis {
    let effect = if access == "read" {
        Effect::DataRead {
            service: "git".to_owned(),
            resource: repo.to_owned(),
        }
    } else {
        Effect::DataWrite {
            service: "git".to_owned(),
            what: format!("Push to {repo}"),
            lost: true,
        }
    };
    Analysis::new(vec![effect], Certainty::Full, Vec::new())
}

/// The analysis of a Managed request: a connector tool's effects, or a credential lease.
#[must_use]
pub fn analyze_request(tool: &str, args: &Value) -> Analysis {
    if tool == LEASE_TOOL {
        let repo = args.get("repo").and_then(Value::as_str).unwrap_or_default();
        let access = args.get("access").and_then(Value::as_str).unwrap_or("push");
        return lease_analysis(repo, access);
    }
    analyze_tool(tool, args)
}

/// The stricter of two matrices, cell by cell.
#[must_use]
pub fn stricter(org: &Matrix, member: &Matrix) -> Matrix {
    let mut out = org.clone();
    for zone in Zone::ALL {
        for level in [Level::L0, Level::L1, Level::L2, Level::L3] {
            let theirs = member.cell(zone, level);
            // `org.loosens(…, theirs)` is true when theirs is looser than the org's cell; otherwise theirs is at
            // least as strict.
            if !org.loosens(zone, level, theirs) {
                out.set(zone, level, theirs);
            }
        }
    }
    out
}

/// Evaluates a Managed request (see the module docs).
#[must_use]
pub fn evaluate_managed(req: &ManagedRequest<'_>, org: &OrgPolicy, member: Option<&MemberPolicy>) -> ManagedVerdict {
    let analysis = analyze_request(req.tool, req.args);
    let resolved = resolve(req, &analysis, &org.facts);
    let mut rules: Vec<Rule> = org.rules.iter().map(|r| r.rule.clone()).collect();
    let mut matrix = org.matrix.clone();
    if let Some(m) = member {
        if let Some(mm) = &m.matrix {
            matrix = stricter(&org.matrix, mm);
        }
        for r in m.rules.iter().filter(|r| matches!(r.then, Then::Deny | Then::Review)) {
            let mut r = r.clone();
            // Kept apart from the org's ids.
            r.id = format!("member:{}", r.id);
            r.issuer = Issuer::Owner;
            rules.push(r);
        }
    }
    let snapshot = Snapshot {
        version: 1,
        matrix,
        rules,
        tasks: Vec::new(),
        facts: org.facts.clone(),
    };
    let evaluation = matrix::evaluate(
        &Input {
            analysis: &analysis,
            resolved: &resolved,
            connection: req.connection,
            seen_resource: false,
            now: req.now,
            // Overnight is the owner's own mode (P5); an org's Managed decisions don't park.
            overnight: false,
        },
        &snapshot,
    );
    let (outcome, obligations) = match evaluation.outcome {
        Outcome::Auto => (ManagedOutcome::Auto, Vec::new()),
        Outcome::Deny => (ManagedOutcome::Deny, Vec::new()),
        Outcome::Ask {
            hard,
        }
        | Outcome::Park {
            hard,
        } => {
            let mut why = evaluation.why.clone();
            // Hard obligations stop P2's evaluation before the cell. The org's cell review still applies: a member's
            // rule (or the floor) only adds to what the org requires, never stands in for it.
            if hard && cell_needs_review(req, &analysis, &resolved, &snapshot, org) {
                why.push(Why::Cell);
            }
            (ManagedOutcome::Review, obligations_of(&why, org))
        }
    };
    let explain = match outcome {
        ManagedOutcome::Auto => String::new(),
        _ => evaluation.explain.clone(),
    };
    ManagedVerdict {
        outcome,
        obligations,
        snapshot: org.hash(),
        member_policy: member.map(MemberPolicy::hash),
        explain,
        lines: analysis.lines(),
        evaluation,
    }
}

/// Where a request acts. A lease names its repository directly (it has no effect that names one): the repository's
/// zone, and the repository as the resource, so the org's resource rules apply to leases as to any other request.
fn resolve(req: &ManagedRequest<'_>, analysis: &Analysis, facts: &Facts) -> Resolved {
    if req.tool == LEASE_TOOL {
        let repo = req.args.get("repo").and_then(Value::as_str).and_then(|r| parse_repo(r, Some(GITHUB)));
        return match repo {
            Some(r) => {
                let (zone, unknown) = repo_zone(facts, &r);
                Resolved {
                    zone,
                    unknown: unknown.into_iter().collect(),
                    resource: Some(r.key()),
                    parts: vec![r.key()],
                }
            }
            // Not a repository anyone can name: Outside, and it matches no permit.
            None => Resolved {
                zone: Zone::Outside,
                unknown: Vec::new(),
                resource: None,
                parts: Vec::new(),
            },
        };
    }
    request_zone(
        analysis,
        facts,
        &Base {
            repo: None,
            remotes: None,
            bare_host: Some(GITHUB),
        },
    )
}

/// Whether the composed matrix cell asks, unless one of the org's permits satisfies it: judged as if there were no
/// hard obligation (no floor, full certainty, no deny or review rules).
fn cell_needs_review(
    req: &ManagedRequest<'_>,
    analysis: &Analysis,
    resolved: &Resolved,
    snapshot: &Snapshot,
    org: &OrgPolicy,
) -> bool {
    let mut soft = analysis.clone();
    soft.floor.clear();
    soft.certainty = Certainty::Full;
    let permits = Snapshot {
        rules: org.rules.iter().filter(|r| r.rule.then == Then::Permit).map(|r| r.rule.clone()).collect(),
        ..snapshot.clone()
    };
    let e = matrix::evaluate(
        &Input {
            analysis: &soft,
            resolved,
            connection: req.connection,
            seen_resource: false,
            now: req.now,
            // Overnight is the owner's own mode (P5); an org's Managed decisions don't park.
            overnight: false,
        },
        &permits,
    );
    match e.outcome {
        Outcome::Auto => false,
        Outcome::Ask {
            hard: false,
        }
        | Outcome::Park {
            hard: false,
        } => true,
        // Only L3 is left hard here: ask the cell unless the matrix itself says Auto.
        Outcome::Ask {
            hard: true,
        }
        | Outcome::Park {
            hard: true,
        }
        | Outcome::Deny => snapshot.matrix.cell(resolved.zone, soft.level) != Cell::Auto,
    }
}

/// The obligations a trace asks for; obligations with the same reviewers merge (one approval may satisfy both).
fn obligations_of(why: &[Why], org: &OrgPolicy) -> Vec<Obligation> {
    let mut out: Vec<Obligation> = Vec::new();
    let mut add = |id: String, spec: ReviewSpec| {
        if let Some(o) = out.iter_mut().find(|o| o.spec == spec) {
            if !o.why.contains(&id) {
                o.why.push(id);
            }
        } else {
            out.push(Obligation {
                id: id.clone(),
                spec,
                why: vec![id],
            });
        }
    };
    for w in why {
        match w {
            Why::Rule(id) => {
                let spec = org
                    .rules
                    .iter()
                    .find(|r| &r.rule.id == id)
                    .and_then(|r| r.review.clone())
                    .unwrap_or_else(ReviewSpec::owner);
                add(id.clone(), spec);
            }
            Why::Floor(_) => add("floor".to_owned(), ReviewSpec::owner()),
            Why::Level3 => add("level3".to_owned(), ReviewSpec::owner()),
            Why::Unreadable => add("unreadable".to_owned(), ReviewSpec::owner()),
            // Overnight's stricter rule for people asks the cell's reviewers too (Managed never parks).
            Why::Cell | Why::FirstTime | Why::Overnight => add("cell".to_owned(), org.cell_review.clone()),
            // A task's envelope is a permit.
            Why::Permit(_) | Why::Task(_) => {}
        }
    }
    out
}

/// One reviewer's signed answer, as policy weighs it (signatures are checked before this).
#[derive(Clone, Copy, Debug)]
pub struct Approval<'a> {
    pub member: &'a str,
    pub groups: &'a [String],
    pub roles: &'a [Role],
    /// Approve (`true`) or deny.
    pub approve: bool,
}

/// Whether the obligations are met.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Met {
    Yes,
    /// The obligations still waiting.
    Waiting(Vec<String>),
    /// An eligible reviewer denied.
    Denied,
}

/// Weighs `approvals` against `obligations` (02 §3 rule 2): every obligation needs `quorum` distinct eligible people;
/// with `distinct_persons`, one person counts toward one such obligation only. Any eligible deny denies.
#[must_use]
pub fn obligations_met(obligations: &[Obligation], requester: &str, approvals: &[Approval<'_>]) -> Met {
    let eligible = |o: &Obligation, a: &Approval<'_>| o.spec.eligible(a.member, a.groups, a.roles, requester);
    if approvals.iter().any(|a| !a.approve && obligations.iter().any(|o| eligible(o, a))) {
        return Met::Denied;
    }
    let mut people: Vec<&str> = approvals.iter().filter(|a| a.approve).map(|a| a.member).collect();
    people.sort_unstable();
    people.dedup();
    let approval_of = |p: &str| approvals.iter().find(|a| a.approve && a.member == p);
    // How many obligations each person could serve: the least flexible are used first.
    let reach = |p: &str| approval_of(p).map_or(0, |a| obligations.iter().filter(|o| eligible(o, a)).count());
    let mut order: Vec<usize> = (0..obligations.len()).collect();
    order.sort_by_key(|&i| {
        people.iter().filter(|p| approval_of(p).is_some_and(|a| eligible(&obligations[i], a))).count()
    });
    let mut used: Vec<&str> = Vec::new();
    let mut waiting: Vec<String> = Vec::new();
    for i in order {
        let o = &obligations[i];
        let mut candidates: Vec<&str> = people
            .iter()
            .copied()
            .filter(|p| approval_of(p).is_some_and(|a| eligible(o, a)))
            .filter(|p| !(o.spec.distinct_persons && used.contains(p)))
            .collect();
        candidates.sort_by_key(|p| reach(p));
        let take: Vec<&str> = candidates.into_iter().take(usize::from(o.spec.quorum)).collect();
        if take.len() < usize::from(o.spec.quorum) {
            waiting.push(o.id.clone());
            continue;
        }
        if o.spec.distinct_persons {
            used.extend(take);
        }
    }
    if waiting.is_empty() {
        Met::Yes
    } else {
        waiting.sort();
        Met::Waiting(waiting)
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use serde_json::json;

    use super::*;
    use crate::context::FactKind;
    use crate::matrix::When;

    fn acme() -> OrgPolicy {
        let mut p = OrgPolicy::empty("o1");
        p.facts.set(FactKind::Owner, "acme", Zone::Work);
        p.facts.set(FactKind::Domain, "acme.com", Zone::Work);
        p.matrix.work = [Cell::Auto, Cell::Auto, Cell::Auto, Cell::Ask];
        p
    }

    fn org_rule(id: &str, when: When, then: Then, review: Option<ReviewSpec>) -> OrgRule {
        OrgRule {
            rule: Rule {
                id: id.to_owned(),
                issuer: Issuer::Org("o1".to_owned()),
                when,
                then,
                not_after: None,
                accepted: false,
                explain: format!("{id} says so"),
            },
            review,
        }
    }

    fn sec(quorum: u8) -> ReviewSpec {
        ReviewSpec {
            reviewers: Reviewers::Group("sec".to_owned()),
            quorum,
            distinct_persons: true,
            exclude_requester: true,
            class: ReviewClass::Deferred,
            window_secs: 0,
        }
    }

    fn eval(tool: &str, args: &Value, p: &OrgPolicy, m: Option<&MemberPolicy>) -> ManagedVerdict {
        evaluate_managed(
            &ManagedRequest {
                org: "o1",
                member: "lee",
                connection: "c1",
                tool,
                args,
                now: 1_000,
            },
            p,
            m,
        )
    }

    fn issue(repo: &str) -> Value {
        json!({"repo": repo, "title": "Flaky test", "body": "x"})
    }

    #[test]
    fn routine_work_in_org_repos_is_auto() {
        let v = eval("github_create_issue", &issue("acme/api"), &acme(), None);
        assert_eq!(v.outcome, ManagedOutcome::Auto, "{v:?}");
        assert!(v.obligations.is_empty());
        assert_eq!(v.snapshot, acme().hash());
        // Outside the org it is the Outside row: asked.
        let v = eval("github_create_issue", &issue("someone/x"), &acme(), None);
        assert_eq!(v.outcome, ManagedOutcome::Review);
        assert_eq!(v.obligation_ids(), ["cell"]);
    }

    #[test]
    fn an_org_deny_rule_denies_and_explains() {
        let mut p = acme();
        p.rules.push(org_rule(
            "prod-deny",
            When {
                resource: Some("acme/prod-*".to_owned()),
                ..When::default()
            },
            Then::Deny,
            None,
        ));
        let v = eval("github_create_issue", &issue("acme/prod-db"), &p, None);
        assert_eq!(v.outcome, ManagedOutcome::Deny);
        assert_eq!(v.explain, "prod-deny says so");
        assert_eq!(eval("github_create_issue", &issue("acme/api"), &p, None).outcome, ManagedOutcome::Auto);
    }

    #[test]
    fn an_org_review_rule_names_its_reviewers() {
        let mut p = acme();
        p.rules.push(org_rule(
            "prod-review",
            When {
                resource: Some("acme/prod-*".to_owned()),
                ..When::default()
            },
            Then::Review,
            Some(sec(2)),
        ));
        let v = eval("github_create_issue", &issue("acme/prod-db"), &p, None);
        assert_eq!(v.outcome, ManagedOutcome::Review);
        assert_eq!(v.obligations.len(), 1);
        assert_eq!(v.obligations[0].spec, sec(2));
        assert_eq!(v.obligations[0].spec.window(), DEFERRED_DEFAULT_SECS);
    }

    #[test]
    fn member_permits_never_widen_the_org() {
        let member = MemberPolicy {
            matrix: None,
            rules: vec![Rule {
                id: "mine".to_owned(),
                issuer: Issuer::Owner,
                when: When::default(),
                then: Then::Permit,
                not_after: None,
                accepted: true,
                explain: String::new(),
            }],
        };
        let v = eval("github_create_issue", &issue("someone/x"), &acme(), Some(&member));
        assert_eq!(v.outcome, ManagedOutcome::Review);
        assert_eq!(v.member_policy, Some(member.hash()));
    }

    #[test]
    fn member_matrix_only_tightens() {
        let mut tight = acme().matrix;
        tight.work = [Cell::Auto, Cell::Auto, Cell::Ask, Cell::Ask];
        let member = MemberPolicy {
            matrix: Some(tight),
            rules: Vec::new(),
        };
        let v = eval("github_create_issue", &issue("acme/api"), &acme(), Some(&member));
        assert_eq!(v.outcome, ManagedOutcome::Review);
        assert_eq!(v.obligations[0].spec, ReviewSpec::owner());
        // A looser member matrix changes nothing.
        let mut loose = acme().matrix;
        loose.outside = [Cell::Auto; 4];
        let member = MemberPolicy {
            matrix: Some(loose),
            rules: Vec::new(),
        };
        assert_eq!(
            eval("github_create_issue", &issue("someone/x"), &acme(), Some(&member)).outcome,
            ManagedOutcome::Review
        );
    }

    #[test]
    fn member_deny_rules_add_up() {
        let member = MemberPolicy {
            matrix: None,
            rules: vec![Rule {
                id: "no-api".to_owned(),
                issuer: Issuer::Owner,
                when: When {
                    resource: Some("acme/api".to_owned()),
                    ..When::default()
                },
                then: Then::Deny,
                not_after: None,
                accepted: false,
                explain: "Not from this profile".to_owned(),
            }],
        };
        assert_eq!(
            eval("github_create_issue", &issue("acme/api"), &acme(), Some(&member)).outcome,
            ManagedOutcome::Deny
        );
    }

    #[test]
    fn the_floor_is_a_hard_owner_review() {
        let mut p = acme();
        p.matrix = Matrix {
            mine: [Cell::Auto; 4],
            friends: [Cell::Auto; 4],
            work: [Cell::Auto; 4],
            outside: [Cell::Auto; 4],
        };
        let v = eval("reins_list_accounts", &json!({}), &p, None);
        assert_eq!(v.outcome, ManagedOutcome::Review, "{v:?}");
        assert_eq!(v.obligation_ids(), ["floor"]);
        assert_eq!(v.obligations[0].spec, ReviewSpec::owner());
    }

    proptest! {
        #[test]
        fn same_inputs_same_verdict(title in ".{0,40}", repo in "(acme|someone)/[a-z]{1,8}") {
            let args = json!({"repo": repo, "title": title});
            let a = eval("github_create_issue", &args, &acme(), None);
            let b = eval("github_create_issue", &args, &acme(), None);
            prop_assert_eq!(a, b);
        }
    }

    fn ap<'a>(member: &'a str, groups: &'a [String], approve: bool) -> Approval<'a> {
        Approval {
            member,
            groups,
            roles: &[],
            approve,
        }
    }

    #[test]
    fn quorum_and_distinct_persons() {
        let g = vec!["sec".to_owned()];
        let none: Vec<String> = Vec::new();
        let o = vec![Obligation {
            id: "r".into(),
            spec: sec(2),
            why: vec!["r".into()],
        }];
        assert_eq!(
            obligations_met(&o, "lee", &[ap("kim", &g, true), ap("kim", &g, true)]),
            Met::Waiting(vec!["r".into()])
        );
        assert_eq!(obligations_met(&o, "lee", &[ap("kim", &g, true), ap("dana", &g, true)]), Met::Yes);
        // The requester is excluded even in the group; a non-member doesn't count.
        assert_eq!(
            obligations_met(&o, "lee", &[ap("kim", &g, true), ap("lee", &g, true), ap("bob", &none, true)]),
            Met::Waiting(vec!["r".into()])
        );
        assert_eq!(obligations_met(&o, "lee", &[ap("kim", &g, true), ap("dana", &g, false)]), Met::Denied);
        // A deny by someone who isn't eligible weighs nothing.
        assert_eq!(
            obligations_met(&o, "lee", &[ap("kim", &g, true), ap("dana", &g, true), ap("bob", &none, false)]),
            Met::Yes
        );
    }

    #[test]
    fn one_person_cannot_fill_two_obligations_with_distinct_persons() {
        let both = vec!["sec".to_owned(), "fin".to_owned()];
        let fin = vec!["fin".to_owned()];
        let mut fin_spec = sec(1);
        fin_spec.reviewers = Reviewers::Group("fin".into());
        let o = vec![
            Obligation {
                id: "a".into(),
                spec: sec(1),
                why: vec![],
            },
            Obligation {
                id: "b".into(),
                spec: fin_spec,
                why: vec![],
            },
        ];
        assert_eq!(obligations_met(&o, "lee", &[ap("kim", &both, true)]), Met::Waiting(vec!["b".into()]));
        // With a second person who can only do Finance, the first goes to Security.
        assert_eq!(obligations_met(&o, "lee", &[ap("kim", &both, true), ap("ann", &fin, true)]), Met::Yes);
        // The requester's own review is theirs.
        let own = vec![Obligation {
            id: "cell".into(),
            spec: ReviewSpec::owner(),
            why: vec![],
        }];
        assert_eq!(obligations_met(&own, "lee", &[ap("lee", &[], true)]), Met::Yes);
        assert_eq!(obligations_met(&own, "lee", &[ap("kim", &both, true)]), Met::Waiting(vec!["cell".into()]));
    }

    #[test]
    fn git_leases_are_judged_by_repo_zone() {
        let v = eval(LEASE_TOOL, &json!({"repo": "acme/api", "access": "push"}), &acme(), None);
        assert_eq!(v.outcome, ManagedOutcome::Auto, "{v:?}");
        assert_eq!(v.evaluation.zone, Zone::Work);
        let v = eval(LEASE_TOOL, &json!({"repo": "someone/x", "access": "push"}), &acme(), None);
        assert_eq!(v.outcome, ManagedOutcome::Review);
        assert_eq!(v.evaluation.zone, Zone::Outside);
    }

    #[test]
    fn an_org_resource_rule_applies_to_a_lease() {
        let mut p = acme();
        p.rules.push(org_rule(
            "prod-deny",
            When {
                resource: Some("acme/prod".to_owned()),
                ..When::default()
            },
            Then::Deny,
            None,
        ));
        p.rules.push(org_rule(
            "infra-review",
            When {
                resource: Some("acme/infra-*".to_owned()),
                ..When::default()
            },
            Then::Review,
            Some(sec(1)),
        ));
        let lease = |repo: &str, access: &str| eval(LEASE_TOOL, &json!({"repo": repo, "access": access}), &p, None);
        assert_eq!(lease("acme/prod", "push").outcome, ManagedOutcome::Deny);
        assert_eq!(lease("Acme/Prod", "read").outcome, ManagedOutcome::Deny);
        let v = lease("acme/infra-live", "push");
        assert_eq!(v.outcome, ManagedOutcome::Review);
        assert_eq!(v.obligations[0].spec, sec(1));
        assert_eq!(lease("acme/api", "push").outcome, ManagedOutcome::Auto);
        // A repository that can't be named is Outside.
        assert_eq!(lease("not a repo", "push").evaluation.zone, Zone::Outside);
    }

    #[test]
    fn a_member_review_rule_adds_to_the_orgs_cell_review() {
        let mut p = acme();
        p.cell_review = sec(1);
        let member = MemberPolicy {
            matrix: None,
            rules: vec![Rule {
                id: "mine".to_owned(),
                issuer: Issuer::Owner,
                when: When::default(),
                then: Then::Review,
                not_after: None,
                accepted: false,
                explain: String::new(),
            }],
        };
        // Outside: the org's cell asks Security; the member's rule asks the member. Both are required.
        let v = eval("github_create_issue", &issue("someone/x"), &p, Some(&member));
        assert_eq!(v.outcome, ManagedOutcome::Review);
        let specs: Vec<&ReviewSpec> = v.obligations.iter().map(|o| &o.spec).collect();
        assert!(specs.contains(&&sec(1)) && specs.contains(&&ReviewSpec::owner()), "{v:?}");
        // In an Auto cell, the member's rule adds only their own review.
        let v = eval("github_create_issue", &issue("acme/api"), &p, Some(&member));
        assert_eq!(v.obligations.iter().map(|o| &o.spec).collect::<Vec<_>>(), [&ReviewSpec::owner()]);
        // The floor doesn't hide the cell either.
        let v = eval("reins_list_accounts", &json!({}), &p, None);
        assert!(v.obligations.iter().any(|o| o.spec == ReviewSpec::owner()), "{v:?}");
    }

    #[test]
    fn policies_with_foreign_issuers_or_bad_windows_are_invalid() {
        assert!(acme().validate().is_ok());
        let mut p = acme();
        let mut r = org_rule("x", When::default(), Then::Deny, None);
        r.rule.issuer = Issuer::Org("other".into());
        p.rules.push(r);
        assert!(p.validate().unwrap_err().contains("issued by this organization"));
        let mut p = acme();
        let mut spec = sec(1);
        spec.window_secs = DEFERRED_MAX_SECS + 1;
        p.rules.push(org_rule("x", When::default(), Then::Review, Some(spec)));
        assert!(p.validate().is_err());
        let mut p = acme();
        p.rules.push(org_rule("x", When::default(), Then::Review, None));
        assert!(p.validate().is_err());
        let mut p = acme();
        p.rules.push(org_rule("x", When::default(), Then::Deny, None));
        p.rules.push(org_rule("x", When::default(), Then::Deny, None));
        assert!(p.validate().unwrap_err().contains("twice"));
        let mut p = acme();
        p.rules.push(org_rule("Bad Id", When::default(), Then::Deny, None));
        assert!(p.validate().is_err());
        let mut p = acme();
        p.cell_review.quorum = 0;
        assert!(p.validate().is_err());
    }

    #[test]
    fn a_policy_round_trips_and_hashes_stably() {
        let mut p = acme();
        p.rules.push(org_rule("prod-review", When::default(), Then::Review, Some(sec(1))));
        let text = serde_json::to_string(&p).unwrap();
        let back: OrgPolicy = serde_json::from_str(&text).unwrap();
        assert_eq!(back, p);
        assert_eq!(back.hash(), p.hash());
        // Defaults fill in.
        let minimal: OrgPolicy = serde_json::from_value(json!({"version": 1, "org": "o1"})).unwrap();
        assert_eq!(minimal.cell_review, ReviewSpec::owner());
        assert!(minimal.posture.screen_lock && minimal.posture.no_root);
    }
}
