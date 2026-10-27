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

use std::collections::{BTreeMap, BTreeSet};

use reins_proto::org::Role;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::context::{Base, FactKind, Facts, GITHUB, Resolved, Zone, parse_repo, repo_zone, request_zone};
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

/// The longest escalation chain an obligation may name (05 §6: backup group, manager, then Security admin).
pub const MAX_ESCALATION: usize = 3;
/// The most reviewer sets one [`Reviewers::AnyOf`] may join.
pub const MAX_ANY_OF: usize = 8;
/// The longest reason a `require_reason` permit accepts.
pub const MAX_REASON: usize = 500;

/// Who satisfies a review.
///
/// `Requester`, `Group`, `Role` and `Members` are decided from the member's standing alone (the issuer-signed roster).
/// The others are **relationships** (05 §6) the decision service resolves at submit from verified sources only (the
/// directory, the org's read-only GitHub App, the on-call provider, the org's verified facts) and countersigns in the
/// decision ([`reins_proto::managed::DecisionRecord::resolved`]).
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
    /// The requester's manager, from the directory (or a Security admin on the Team tier).
    Manager,
    /// The repository's code owners, from its default branch's `CODEOWNERS`.
    CodeOwners,
    /// The owner named by the resource's verified `tag` fact.
    ResourceOwner {
        tag: String,
    },
    /// Whoever is on call now for the org's schedule `alias` (PagerDuty, Opsgenie, Incident.io).
    OnCall {
        alias: String,
    },
    /// The backup group of a group (the org's routing settings).
    BackupOf(String),
    /// Anyone any of these admits (flat: no `AnyOf` inside).
    AnyOf(Vec<Reviewers>),
}

impl Reviewers {
    /// Whether the member's own standing admits them (no resolution needed).
    fn admits_static(&self, member: &str, groups: &[String], roles: &[Role], requester: &str) -> bool {
        match self {
            Self::Requester => member == requester,
            Self::Group(g) => groups.iter().any(|x| x == g),
            Self::Role(r) => roles.contains(r),
            Self::Members(list) => list.iter().any(|x| x == member),
            Self::AnyOf(list) => list.iter().any(|r| r.admits_static(member, groups, roles, requester)),
            Self::Manager
            | Self::CodeOwners
            | Self::ResourceOwner {
                ..
            }
            | Self::OnCall {
                ..
            }
            | Self::BackupOf(_) => false,
        }
    }

    /// Whether it names the requester (directly or in an `AnyOf`).
    fn names_requester(&self) -> bool {
        match self {
            Self::Requester => true,
            Self::AnyOf(list) => list.iter().any(Self::names_requester),
            _ => false,
        }
    }

    /// Whether resolving it takes the decision service (a relationship, on-call, a backup).
    #[must_use]
    pub fn is_dynamic(&self) -> bool {
        match self {
            Self::Requester | Self::Group(_) | Self::Role(_) | Self::Members(_) => false,
            Self::AnyOf(list) => list.iter().any(Self::is_dynamic),
            _ => true,
        }
    }

    /// The first group it names (the default chain's backup is that group's).
    #[must_use]
    pub fn first_group(&self) -> Option<&str> {
        match self {
            Self::Group(g) => Some(g),
            Self::AnyOf(list) => list.iter().find_map(Self::first_group),
            _ => None,
        }
    }

    /// The reviewer sets it joins (itself, or an `AnyOf`'s members).
    #[must_use]
    pub fn parts(&self) -> Vec<&Self> {
        match self {
            Self::AnyOf(list) => list.iter().collect(),
            other => vec![other],
        }
    }

    fn validate(&self, exclude_requester: bool, in_chain: bool) -> Result<(), String> {
        match self {
            Self::Members(m) if m.is_empty() => Err("a review by members names at least one".to_owned()),
            Self::Requester if in_chain => Err("an escalation never goes to the requester".to_owned()),
            Self::ResourceOwner {
                tag,
            } if !id_ok(tag) => Err(format!("resource owner tag {tag:?} must be 1-64 of a-z 0-9 . _ @ -")),
            Self::OnCall {
                alias,
            } if !id_ok(alias) => Err(format!("on-call alias {alias:?} must be 1-64 of a-z 0-9 . _ @ -")),
            Self::AnyOf(list) => {
                if list.is_empty() || list.len() > MAX_ANY_OF {
                    return Err(format!("any of joins 1 to {MAX_ANY_OF} reviewer sets"));
                }
                for r in list {
                    if matches!(r, Self::AnyOf(_)) {
                        return Err("any of can't hold another any of".to_owned());
                    }
                    if matches!(r, Self::Requester) && (exclude_requester || in_chain) {
                        return Err("any of names the requester only when exclude_requester is off".to_owned());
                    }
                    r.validate(exclude_requester, in_chain)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Whether the decision service may have resolved `member` for this reviewer set, judged from the issuer-signed
    /// roster: groups, roles, members, the manager and group backups are checked; code owners, on-call and resource
    /// owners are dynamic and trusted from the countersigned resolution (`docs/security-model.md`).
    fn may_resolve(&self, member: &str, groups: &[String], roles: &[Role], facts: &RosterFacts<'_>) -> bool {
        match self {
            // The requester is never someone a resolution adds.
            Self::Requester => false,
            Self::Group(g) => groups.iter().any(|x| x == g),
            Self::Role(r) => roles.contains(r),
            Self::Members(list) => list.iter().any(|x| x == member),
            Self::Manager => facts.requester_manager == Some(member),
            Self::BackupOf(g) => facts.backups.get(g).is_some_and(|b| groups.iter().any(|x| x == b)),
            Self::CodeOwners
            | Self::ResourceOwner {
                ..
            }
            | Self::OnCall {
                ..
            } => true,
            Self::AnyOf(list) => list.iter().any(|r| r.may_resolve(member, groups, roles, facts)),
        }
    }
}

/// What an executor reads from the issuer-signed roster to check a countersigned resolution.
#[derive(Clone, Copy, Debug)]
pub struct RosterFacts<'a> {
    /// The requester's manager (user id).
    pub requester_manager: Option<&'a str>,
    /// Group id → its backup group id.
    pub backups: &'a BTreeMap<String, String>,
}

/// Obligation id → the members its relationships and escalations resolved to.
pub type ResolvedSets = BTreeMap<String, BTreeSet<String>>;

/// A weekly window in a time zone (a change window, business hours).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Hours {
    /// The days it opens on.
    pub days: Vec<chrono::Weekday>,
    /// `HH:MM`, inclusive.
    pub start: String,
    /// `HH:MM`, exclusive; at or before `start` it closes the next day.
    pub end: String,
    /// IANA time zone (`Europe/Berlin`).
    pub tz: String,
}

fn hhmm(s: &str) -> Option<u32> {
    let (h, m) = s.split_once(':')?;
    if h.len() != 2 || m.len() != 2 {
        return None;
    }
    let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
    (h < 24 && m < 60).then_some(h * 60 + m)
}

impl Hours {
    fn validate(&self) -> Result<(), String> {
        if self.days.is_empty() {
            return Err("a window names at least one day".to_owned());
        }
        if hhmm(&self.start).is_none() || hhmm(&self.end).is_none() {
            return Err("window times are HH:MM".to_owned());
        }
        if self.tz.parse::<chrono_tz::Tz>().is_err() {
            return Err(format!("{:?} isn't a time zone", self.tz));
        }
        Ok(())
    }

    /// Whether `at` (Unix seconds) is inside the window. A window that can't be read is never open.
    #[must_use]
    pub fn contains(&self, at: i64) -> bool {
        use chrono::{Datelike, TimeZone, Timelike};
        let (Some(start), Some(end), Ok(tz)) = (hhmm(&self.start), hhmm(&self.end), self.tz.parse::<chrono_tz::Tz>())
        else {
            return false;
        };
        let Some(local) = tz.timestamp_opt(at, 0).single() else {
            return false;
        };
        let minute = local.hour() * 60 + local.minute();
        let today = self.days.contains(&local.weekday());
        if start < end {
            today && (start..end).contains(&minute)
        } else {
            // Overnight: from `start` on an opening day to `end` the day after.
            (today && minute >= start) || (self.days.contains(&local.weekday().pred()) && minute < end)
        }
    }
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

#[allow(clippy::trivially_copy_pass_by_ref, reason = "serde's skip_serializing_if takes a reference")]
const fn is_false(b: &bool) -> bool {
    !*b
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
    /// Who it escalates to, step by step, when a window passes unanswered (at most [`MAX_ESCALATION`]). Empty is the
    /// org default ([`ReviewSpec::chain`]). Escalation adds reviewers; it never removes any.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub escalate: Vec<Reviewers>,
    /// Breaks quiet hours (03 §5.2).
    #[serde(default, skip_serializing_if = "is_false")]
    pub urgent: bool,
    /// A time-boxed approval: valid this long after the decision (at most the executor's 24 h).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_valid_secs: Option<u64>,
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
            escalate: Vec::new(),
            urgent: false,
            approval_valid_secs: None,
        }
    }

    /// A review by `reviewers`: one person, not the requester, Deferred with the class window.
    #[must_use]
    pub fn by(reviewers: Reviewers) -> Self {
        Self {
            reviewers,
            quorum: 1,
            distinct_persons: true,
            exclude_requester: true,
            class: ReviewClass::Deferred,
            window_secs: 0,
            escalate: Vec::new(),
            urgent: false,
            approval_valid_secs: None,
        }
    }

    /// The escalation steps after the first: `escalate`, or the org default (05 §6: the first group's backup, the
    /// requester's manager, then Security admins). The requester's own review doesn't escalate.
    #[must_use]
    pub fn chain(&self) -> Vec<Reviewers> {
        if !self.escalate.is_empty() {
            return self.escalate.clone();
        }
        if self.reviewers == Reviewers::Requester {
            return Vec::new();
        }
        let mut out = Vec::new();
        if let Some(g) = self.reviewers.first_group() {
            out.push(Reviewers::BackupOf(g.to_owned()));
        }
        out.push(Reviewers::Manager);
        out.push(Reviewers::Role(Role::SecurityAdmin));
        out
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
        self.reviewers.validate(self.exclude_requester, false)?;
        if self.escalate.len() > MAX_ESCALATION {
            return Err(format!("an escalation has at most {MAX_ESCALATION} steps"));
        }
        for step in &self.escalate {
            step.validate(true, true)?;
        }
        if self.approval_valid_secs.is_some_and(|s| s == 0 || s > 24 * 3600) {
            return Err("a time-boxed approval lasts 1 second to 24 hours".to_owned());
        }
        Ok(())
    }

    /// Whether the requester may answer it themselves: only a review that names them, and only their own review when
    /// `exclude_requester` is on (05 §6: no self-approval).
    fn requester_may(&self) -> bool {
        self.reviewers.names_requester() && (!self.exclude_requester || self.reviewers == Reviewers::Requester)
    }

    /// Whether `member` (with these groups and roles) may satisfy this review for `requester`, from their standing
    /// alone (relationships need [`ReviewSpec::eligible_resolved`]).
    #[must_use]
    pub fn eligible(&self, member: &str, groups: &[String], roles: &[Role], requester: &str) -> bool {
        self.eligible_resolved(member, groups, roles, requester, None)
    }

    /// Whether `member` may satisfy this review: their standing admits them, or the obligation's resolution
    /// (`resolved`: its relationships and escalations) names them. The requester counts only where
    /// [`ReviewSpec::requester_may`], never through a resolution.
    #[must_use]
    pub fn eligible_resolved(
        &self,
        member: &str,
        groups: &[String],
        roles: &[Role],
        requester: &str,
        resolved: Option<&BTreeSet<String>>,
    ) -> bool {
        if member == requester {
            return self.requester_may();
        }
        self.reviewers.admits_static(member, groups, roles, requester) || resolved.is_some_and(|s| s.contains(member))
    }

    /// Whether a countersigned resolution may name `member` (not the requester) for this obligation: its reviewers or
    /// a step of its escalation chain admit them, judged from the issuer-signed roster ([`RosterFacts`]).
    #[must_use]
    pub fn may_resolve(
        &self,
        member: &str,
        groups: &[String],
        roles: &[Role],
        requester: &str,
        facts: &RosterFacts<'_>,
    ) -> bool {
        member != requester
            && (self.reviewers.may_resolve(member, groups, roles, facts)
                || self.chain().iter().any(|r| r.may_resolve(member, groups, roles, facts)))
    }
}

/// An org rule: P2's [`Rule`] (issued by the org), with who reviews when it says `review`, and filters on facts the
/// matrix doesn't know (who asks, a reason, the time, an amount, a new party). A rule whose filter doesn't hold is
/// left out before the composed snapshot is built, so [`matrix::evaluate`] is unchanged.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrgRule {
    #[serde(flatten)]
    pub rule: Rule,
    #[serde(default)]
    pub review: Option<ReviewSpec>,
    /// Only requests by these members (user ids).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub members: Vec<String>,
    /// Permits only: it holds when the agent gave a reason (1–[`MAX_REASON`] characters).
    #[serde(default, skip_serializing_if = "is_false")]
    pub require_reason: bool,
    /// It applies only outside every one of these windows.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outside_hours: Vec<Hours>,
    /// Deny and review only: a Money effect at or above this many minor units (an unknown amount counts as above).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_amount_minor: Option<u64>,
    /// The currency `min_amount_minor` is in (ISO 4217); another currency counts as above.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
    /// Deny and review only: a party the org's facts don't know (a new payee, a new recipient).
    #[serde(default, skip_serializing_if = "is_false")]
    pub new_party: bool,
}

impl OrgRule {
    /// A rule with no filters.
    #[must_use]
    pub fn new(rule: Rule, review: Option<ReviewSpec>) -> Self {
        Self {
            rule,
            review,
            members: Vec::new(),
            require_reason: false,
            outside_hours: Vec::new(),
            min_amount_minor: None,
            currency: None,
            new_party: false,
        }
    }

    fn validate(&self) -> Result<(), String> {
        let id = &self.rule.id;
        let permit = self.rule.then == Then::Permit;
        if self.require_reason && !permit {
            return Err(format!("rule {id}: only a permit requires a reason"));
        }
        if permit && (self.min_amount_minor.is_some() || self.new_party) {
            // An unknown amount or party counts as matching: on a permit that would widen.
            return Err(format!("rule {id}: amount and new-party filters are for deny and review rules"));
        }
        if self.currency.is_some() && self.min_amount_minor.is_none() {
            return Err(format!("rule {id}: a currency goes with an amount"));
        }
        if let Some(c) = &self.currency
            && reins_proto::payments::normalize_currency(c).is_err()
        {
            return Err(format!("rule {id}: {c:?} isn't a currency"));
        }
        for h in &self.outside_hours {
            h.validate().map_err(|e| format!("rule {id}: {e}"))?;
        }
        Ok(())
    }

    /// Whether the rule's filters hold for this request.
    fn applies(&self, req: &ManagedRequest<'_>, analysis: &Analysis, resolved: &Resolved, facts: &Facts) -> bool {
        if !self.members.is_empty() && !self.members.iter().any(|m| m == req.member) {
            return false;
        }
        if self.require_reason && !req.reason.is_some_and(|r| !r.trim().is_empty() && r.chars().count() <= MAX_REASON) {
            return false;
        }
        if self.outside_hours.iter().any(|h| h.contains(req.now)) {
            return false;
        }
        if let Some(min) = self.min_amount_minor
            && !money_at_least(analysis, req.args, min, self.currency.as_deref())
        {
            return false;
        }
        if self.new_party && resolved.unknown.is_empty() && !unknown_payee(analysis, req.args, facts) {
            return false;
        }
        true
    }
}

/// Whether a Money effect pays someone the org's facts don't know: the merchant's site (`merchant_url`) has no domain
/// or host fact. A payee that can't be named is unknown.
fn unknown_payee(analysis: &Analysis, args: &Value, facts: &Facts) -> bool {
    if !analysis.effects.iter().any(|e| matches!(e, Effect::Money { .. })) {
        return false;
    }
    let host = args.get("merchant_url").and_then(Value::as_str).and_then(|u| reins_proto::payments::https_host(u).ok());
    let Some(host) = host else {
        return true;
    };
    if facts.get(FactKind::Host, &host).is_some() {
        return false;
    }
    // The host or a parent domain of it (at least two labels) the org knows.
    let labels: Vec<&str> = host.split('.').collect();
    !(0..labels.len().saturating_sub(1)).any(|i| facts.get(FactKind::Domain, &labels[i..].join(".")).is_some())
}

/// Whether the request moves at least `min` minor units (of `currency`, when named). An amount or currency that can't
/// be read counts as at least; a request with no Money effect doesn't.
fn money_at_least(analysis: &Analysis, args: &Value, min: u64, currency: Option<&str>) -> bool {
    if !analysis.effects.iter().any(|e| matches!(e, Effect::Money { .. })) {
        return false;
    }
    let given = args.get("currency").and_then(Value::as_str).map(reins_proto::payments::normalize_currency);
    let code = match (given, currency) {
        (Some(Ok(c)), Some(want)) => {
            if reins_proto::payments::normalize_currency(want).ok().as_deref() != Some(c.as_str()) {
                return true;
            }
            c
        }
        (Some(Ok(c)), None) => c,
        // No currency, or one that can't be read: unknown.
        _ => return true,
    };
    let Some(amount) = args.get("amount").or_else(|| args.get("total")) else {
        return true;
    };
    match reins_proto::payments::parse_amount(amount, &code, "amount") {
        Ok(minor) => u64::try_from(minor).map_or(true, |m| m >= min),
        Err(_) => true,
    }
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
    /// Verified resource tags (05 §7.1: from cloud tags or the CMDB, signed into the policy): tag → resource glob (a
    /// prefix ending in `*`, or exact) → owner (a member's email or a group id). [`Reviewers::ResourceOwner`] reads
    /// them.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub resource_tags: BTreeMap<String, BTreeMap<String, String>>,
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
            resource_tags: BTreeMap::new(),
        }
    }

    /// The owner the resource tag `tag` names for any of `resources` (the first glob that matches, in order).
    #[must_use]
    pub fn tag_owner(&self, tag: &str, resources: &[String]) -> Option<&str> {
        let globs = self.resource_tags.get(tag)?;
        resources
            .iter()
            .find_map(|r| {
                let r = r.to_ascii_lowercase();
                globs.iter().find(|(g, _)| match g.to_ascii_lowercase().strip_suffix('*') {
                    Some(prefix) => r.starts_with(prefix),
                    None => g.eq_ignore_ascii_case(&r),
                })
            })
            .map(|(_, owner)| owner.as_str())
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
            r.validate()?;
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
    /// When it is judged (rules' `not_after`, `outside_hours`).
    pub now: i64,
    /// Why the agent says it needs this (MCP `params._meta["dev.reins/reason"]`): shown to reviewers, and what a
    /// `require_reason` permit asks for. Never evidence of anything else.
    pub reason: Option<&'a str>,
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
    // Rules whose filters don't hold for this request are left out: the matrix never sees them.
    let live: Vec<&OrgRule> = org.rules.iter().filter(|r| r.applies(req, &analysis, &resolved, &org.facts)).collect();
    let mut rules: Vec<Rule> = live.iter().map(|r| r.rule.clone()).collect();
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
            if hard && cell_needs_review(req, &analysis, &resolved, &snapshot, &live) {
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
pub fn resolve(req: &ManagedRequest<'_>, analysis: &Analysis, facts: &Facts) -> Resolved {
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
    live: &[&OrgRule],
) -> bool {
    let mut soft = analysis.clone();
    soft.floor.clear();
    soft.certainty = Certainty::Full;
    let permits = Snapshot {
        rules: live.iter().filter(|r| r.rule.then == Then::Permit).map(|r| r.rule.clone()).collect(),
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

/// [`obligations_met_resolved`] without resolutions: standing alone decides.
#[must_use]
pub fn obligations_met(obligations: &[Obligation], requester: &str, approvals: &[Approval<'_>]) -> Met {
    obligations_met_resolved(obligations, requester, approvals, &ResolvedSets::new())
}

/// Weighs `approvals` against `obligations` (02 §3 rule 2): every obligation needs `quorum` distinct eligible people;
/// with `distinct_persons`, one person counts toward one such obligation only. Any eligible deny denies.
///
/// Eligibility ([`ReviewSpec::eligible_resolved`]): groups, roles and members from the reviewer's standing; every
/// relationship, on-call schedule and escalation from `resolved[obligation id]`. The requester never counts through a
/// resolution.
#[must_use]
pub fn obligations_met_resolved(
    obligations: &[Obligation],
    requester: &str,
    approvals: &[Approval<'_>],
    resolved: &ResolvedSets,
) -> Met {
    let eligible = |o: &Obligation, a: &Approval<'_>| {
        o.spec.eligible_resolved(a.member, a.groups, a.roles, requester, resolved.get(&o.id))
    };
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
        OrgRule::new(
            Rule {
                id: id.to_owned(),
                issuer: Issuer::Org("o1".to_owned()),
                when,
                then,
                not_after: None,
                accepted: false,
                explain: format!("{id} says so"),
            },
            review,
        )
    }

    fn sec(quorum: u8) -> ReviewSpec {
        ReviewSpec {
            quorum,
            ..ReviewSpec::by(Reviewers::Group("sec".to_owned()))
        }
    }

    fn eval(tool: &str, args: &Value, p: &OrgPolicy, m: Option<&MemberPolicy>) -> ManagedVerdict {
        eval_as("lee", tool, args, p, m, 1_000, None)
    }

    fn eval_as(
        member: &str,
        tool: &str,
        args: &Value,
        p: &OrgPolicy,
        m: Option<&MemberPolicy>,
        now: i64,
        reason: Option<&str>,
    ) -> ManagedVerdict {
        evaluate_managed(
            &ManagedRequest {
                org: "o1",
                member,
                connection: "c1",
                tool,
                args,
                now,
                reason,
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

    // ---- O3: relationship reviewers, escalation, rule filters, resolved sets.

    fn rule_ids(v: &ManagedVerdict) -> Vec<String> {
        v.obligations.iter().flat_map(|o| o.why.clone()).collect()
    }

    #[test]
    fn new_reviewers_round_trip_and_validate() {
        let all = Reviewers::AnyOf(vec![
            Reviewers::Manager,
            Reviewers::CodeOwners,
            Reviewers::ResourceOwner {
                tag: "owner".into(),
            },
            Reviewers::OnCall {
                alias: "payments".into(),
            },
            Reviewers::BackupOf("sec".into()),
            Reviewers::Group("fin".into()),
        ]);
        let text = serde_json::to_value(&all).unwrap();
        assert_eq!(
            text,
            json!({"any_of": ["manager", "code_owners", {"resource_owner": {"tag": "owner"}},
                              {"on_call": {"alias": "payments"}}, {"backup_of": "sec"}, {"group": "fin"}]})
        );
        assert_eq!(serde_json::from_value::<Reviewers>(text).unwrap(), all);
        let mut p = acme();
        p.rules.push(org_rule("r", When::default(), Then::Review, Some(ReviewSpec::by(all))));
        assert!(p.validate().is_ok(), "{:?}", p.validate());
        let invalid = |r: Reviewers, exclude: bool| {
            let mut p = acme();
            let mut spec = ReviewSpec::by(r);
            spec.exclude_requester = exclude;
            p.rules.push(org_rule("r", When::default(), Then::Review, Some(spec)));
            p.validate().is_err()
        };
        assert!(invalid(Reviewers::AnyOf(vec![Reviewers::AnyOf(vec![Reviewers::Manager])]), true), "nested");
        assert!(invalid(Reviewers::AnyOf(vec![Reviewers::Manager; 9]), true), "more than 8");
        assert!(invalid(Reviewers::AnyOf(vec![]), true), "empty");
        assert!(invalid(Reviewers::AnyOf(vec![Reviewers::Requester, Reviewers::Manager]), true), "self-approval");
        assert!(!invalid(Reviewers::AnyOf(vec![Reviewers::Requester, Reviewers::Group("a".into())]), false));
        assert!(invalid(
            Reviewers::OnCall {
                alias: "Bad Alias".into()
            },
            true
        ));
        // Older policies (no O3 fields) keep their hash: nothing new is written for them.
        let old = acme();
        let text = serde_json::to_string(&old).unwrap();
        assert!(!text.contains("escalate") && !text.contains("resource_tags"), "{text}");
        let mut with_review = acme();
        with_review.rules.push(org_rule("prod", When::default(), Then::Review, Some(sec(1))));
        let text = serde_json::to_string(&with_review).unwrap();
        for absent in ["escalate", "urgent", "approval_valid_secs", "members", "outside_hours", "new_party"] {
            assert!(!text.contains(absent), "{absent} in {text}");
        }
    }

    #[test]
    fn escalation_is_capped_at_three() {
        let mut spec = sec(1);
        assert_eq!(
            spec.chain(),
            [Reviewers::BackupOf("sec".into()), Reviewers::Manager, Reviewers::Role(Role::SecurityAdmin)]
        );
        assert!(ReviewSpec::owner().chain().is_empty(), "the requester's own review doesn't escalate");
        spec.escalate = vec![Reviewers::Manager; 3];
        assert_eq!(spec.chain(), [Reviewers::Manager, Reviewers::Manager, Reviewers::Manager]);
        let mut p = acme();
        p.rules.push(org_rule("r", When::default(), Then::Review, Some(spec.clone())));
        assert!(p.validate().is_ok());
        spec.escalate.push(Reviewers::Role(Role::SecurityAdmin));
        let mut p = acme();
        p.rules.push(org_rule("r", When::default(), Then::Review, Some(spec.clone())));
        assert!(p.validate().unwrap_err().contains("at most 3"));
        // Never to the requester.
        spec.escalate = vec![Reviewers::Requester];
        let mut p = acme();
        p.rules.push(org_rule("r", When::default(), Then::Review, Some(spec)));
        assert!(p.validate().is_err());
    }

    #[test]
    fn a_reason_is_needed_for_reason_permits() {
        let mut p = acme();
        let mut permit = org_rule(
            "why",
            When {
                resource: Some("someone/*".into()),
                ..When::default()
            },
            Then::Permit,
            None,
        );
        permit.require_reason = true;
        p.rules.push(permit);
        assert!(p.validate().is_ok(), "{:?}", p.validate());
        let tool = "github_create_issue";
        let outcome = |reason: Option<&str>| eval_as("lee", tool, &issue("someone/x"), &p, None, 1_000, reason).outcome;
        assert_eq!(outcome(None), ManagedOutcome::Review);
        assert_eq!(outcome(Some("   ")), ManagedOutcome::Review);
        assert_eq!(outcome(Some(&"x".repeat(MAX_REASON + 1))), ManagedOutcome::Review);
        assert_eq!(outcome(Some("Debugging ticket 42")), ManagedOutcome::Auto);
        // Only permits ask for a reason.
        let mut p = acme();
        let mut r = org_rule("why", When::default(), Then::Deny, None);
        r.require_reason = true;
        p.rules.push(r);
        assert!(p.validate().is_err());
    }

    #[test]
    fn outside_hours_follow_the_time_zone() {
        use chrono::Weekday;
        let window = Hours {
            days: vec![Weekday::Mon, Weekday::Tue, Weekday::Wed, Weekday::Thu, Weekday::Fri],
            start: "09:00".into(),
            end: "17:00".into(),
            tz: "Europe/Berlin".into(),
        };
        // Monday 2026-03-23 (CET, UTC+1) and Monday 2026-03-30 (CEST, UTC+2), both 09:30 local.
        let winter = 1_774_254_600; // 2026-03-23T08:30:00Z
        let summer = 1_774_855_800; // 2026-03-30T07:30:00Z
        assert!(window.contains(winter) && window.contains(summer));
        // 08:30 local the same days: closed, though the UTC hour of the winter one is 08:30.
        assert!(!window.contains(winter - 3600) && !window.contains(summer - 3600));
        // Saturday 2026-03-28 12:00 CET.
        assert!(!window.contains(1_774_695_600));
        let overnight = Hours {
            days: vec![Weekday::Fri],
            start: "22:00".into(),
            end: "02:00".into(),
            tz: "UTC".into(),
        };
        assert!(overnight.contains(1_774_648_860), "Friday 22:01"); // 2026-03-27T22:01Z
        assert!(overnight.contains(1_774_663_140), "Saturday 01:59"); // 2026-03-28T01:59Z
        assert!(!overnight.contains(1_774_663_260), "Saturday 02:01");
        // As a filter: Security reviews outside the window only.
        let mut p = acme();
        let mut r = org_rule(
            "after-hours",
            When {
                resource: Some("acme/*".into()),
                ..When::default()
            },
            Then::Review,
            Some(sec(1)),
        );
        r.outside_hours = vec![window];
        p.rules.push(r);
        assert!(p.validate().is_ok());
        let at = |t| eval_as("lee", "github_create_issue", &issue("acme/api"), &p, None, t, None).outcome;
        assert_eq!(at(winter), ManagedOutcome::Auto);
        assert_eq!(at(winter - 3600), ManagedOutcome::Review);
        let mut bad = acme();
        let mut r = org_rule("x", When::default(), Then::Deny, None);
        r.outside_hours = vec![Hours {
            days: vec![Weekday::Mon],
            start: "9:00".into(),
            end: "17:00".into(),
            tz: "Mars/Olympus".into(),
        }];
        bad.rules.push(r);
        assert!(bad.validate().is_err());
    }

    fn purchase(total: &str, currency: Option<&str>, merchant_url: Option<&str>) -> Value {
        let mut v = json!({"merchant": "Acme Supplies", "total": total});
        if let Some(c) = currency {
            v["currency"] = json!(c);
        }
        if let Some(u) = merchant_url {
            v["merchant_url"] = json!(u);
        }
        v
    }

    #[test]
    fn amount_thresholds_and_unknown_amounts() {
        let mut p = acme();
        let mut r = org_rule("big", When::default(), Then::Review, Some(sec(2)));
        r.min_amount_minor = Some(100_000);
        r.currency = Some("usd".into());
        p.rules.push(r);
        assert!(p.validate().is_ok(), "{:?}", p.validate());
        let big = |args: &Value| rule_ids(&eval("payments_purchase_request", args, &p, None)).contains(&"big".into());
        assert!(!big(&purchase("999.99", Some("USD"), None)));
        assert!(big(&purchase("1000.00", Some("USD"), None)));
        assert!(big(&purchase("5000", Some("usd"), None)));
        assert!(big(&purchase("12", Some("EUR"), None)), "another currency is unknown");
        assert!(big(&purchase("12", None, None)), "no currency is unknown");
        assert!(big(&purchase("lots", Some("USD"), None)), "an amount that can't be read is unknown");
        // Not a payment: the rule doesn't apply.
        assert!(!rule_ids(&eval("github_create_issue", &issue("acme/api"), &p, None)).contains(&"big".into()));
        // Never on a permit (unknown would widen).
        let mut p = acme();
        let mut r = org_rule("x", When::default(), Then::Permit, None);
        r.min_amount_minor = Some(1);
        p.rules.push(r);
        assert!(p.validate().is_err());
    }

    #[test]
    fn new_party_rules_apply_to_unknown_payees_only() {
        let mut p = acme();
        p.facts.set(FactKind::Domain, "supplies.example", Zone::Work);
        let mut r = org_rule("new-payee", When::default(), Then::Review, Some(sec(1)));
        r.new_party = true;
        p.rules.push(r);
        let applies =
            |args: &Value| rule_ids(&eval("payments_purchase_request", args, &p, None)).contains(&"new-payee".into());
        assert!(!applies(&purchase("5", Some("USD"), Some("https://shop.supplies.example/cart"))));
        assert!(applies(&purchase("5", Some("USD"), Some("https://unknown-vendor.example/"))));
        assert!(applies(&purchase("5", Some("USD"), None)), "a payee that can't be named is new");
        // An email to a stranger is a new party; one to a colleague isn't.
        let send = |to: &str| json!({"to": to, "subject": "Hi", "body": "x"});
        assert!(rule_ids(&eval("gmail_send", &send("x@stranger.example"), &p, None)).contains(&"new-payee".into()));
        assert!(!rule_ids(&eval("gmail_send", &send("kim@acme.com"), &p, None)).contains(&"new-payee".into()));
    }

    #[test]
    fn member_scoped_rules() {
        let mut p = acme();
        let mut r = org_rule(
            "exec",
            When {
                resource: Some("acme/*".into()),
                ..When::default()
            },
            Then::Review,
            Some(ReviewSpec {
                exclude_requester: false,
                ..ReviewSpec::by(Reviewers::AnyOf(vec![Reviewers::Requester, Reviewers::Group("assistants".into())]))
            }),
        );
        r.members = vec!["ceo".into()];
        p.rules.push(r);
        assert!(p.validate().is_ok(), "{:?}", p.validate());
        let v = eval_as("ceo", "github_create_issue", &issue("acme/api"), &p, None, 1_000, None);
        assert_eq!(v.outcome, ManagedOutcome::Review);
        assert_eq!(eval("github_create_issue", &issue("acme/api"), &p, None).outcome, ManagedOutcome::Auto);
        // The exec or an assistant decides; nobody else.
        let assistants = vec!["assistants".to_owned()];
        let none: Vec<String> = Vec::new();
        let met = |member: &str, groups: &[String]| {
            obligations_met(&v.obligations, "ceo", &[ap(member, groups, true)]) == Met::Yes
        };
        assert!(met("ceo", &none));
        assert!(met("amy", &assistants));
        assert!(!met("bob", &none));
    }

    #[test]
    fn resolved_sets_decide_eligibility() {
        let mut spec = ReviewSpec::by(Reviewers::Manager);
        spec.quorum = 1;
        let o = vec![Obligation {
            id: "mgr".into(),
            spec,
            why: vec![],
        }];
        let none: Vec<String> = Vec::new();
        let mut resolved = ResolvedSets::new();
        resolved.insert("mgr".into(), ["kim".to_owned()].into());
        assert_eq!(obligations_met_resolved(&o, "lee", &[ap("kim", &none, true)], &resolved), Met::Yes);
        assert_eq!(
            obligations_met_resolved(&o, "lee", &[ap("sam", &none, true)], &resolved),
            Met::Waiting(vec!["mgr".into()])
        );
        // Without the resolution nobody can (never the requester, never widened).
        assert_eq!(obligations_met(&o, "lee", &[ap("kim", &none, true)]), Met::Waiting(vec!["mgr".into()]));
        // A resolution naming the requester doesn't make them eligible.
        resolved.insert("mgr".into(), ["lee".to_owned()].into());
        assert_eq!(
            obligations_met_resolved(&o, "lee", &[ap("lee", &none, true)], &resolved),
            Met::Waiting(vec!["mgr".into()])
        );
        // Escalation adds: a Security group obligation also counts its escalated members.
        let o = vec![Obligation {
            id: "prod".into(),
            spec: sec(1),
            why: vec![],
        }];
        let mut resolved = ResolvedSets::new();
        resolved.insert("prod".into(), ["mia".to_owned()].into());
        assert_eq!(obligations_met_resolved(&o, "lee", &[ap("mia", &none, true)], &resolved), Met::Yes);
        assert_eq!(obligations_met_resolved(&o, "lee", &[ap("kim", &["sec".to_owned()], true)], &resolved), Met::Yes);
    }

    #[test]
    fn a_resolution_is_checked_against_the_roster() {
        let backups: BTreeMap<String, String> = [("sec".to_owned(), "sec-backup".to_owned())].into();
        let facts = RosterFacts {
            requester_manager: Some("kim"),
            backups: &backups,
        };
        let none: Vec<String> = Vec::new();
        let mgr = ReviewSpec::by(Reviewers::Manager);
        assert!(mgr.may_resolve("kim", &none, &[], "lee", &facts));
        assert!(!mgr.may_resolve("sam", &none, &[], "lee", &facts), "not the roster's manager");
        assert!(!mgr.may_resolve("lee", &none, &[], "lee", &facts), "never the requester");
        // The default chain: the backup group, the manager, Security admins.
        let s = sec(1);
        assert!(s.may_resolve("bo", &["sec-backup".to_owned()], &[], "lee", &facts));
        assert!(s.may_resolve("kim", &none, &[], "lee", &facts));
        assert!(s.may_resolve("dana", &none, &[Role::SecurityAdmin], "lee", &facts));
        assert!(!s.may_resolve("eve", &["other".to_owned()], &[], "lee", &facts));
        // Dynamic sources are taken on the countersignature.
        assert!(ReviewSpec::by(Reviewers::CodeOwners).may_resolve("eve", &none, &[], "lee", &facts));
    }

    #[test]
    fn resource_tags_name_owners() {
        let mut p = acme();
        p.resource_tags.insert("owner".into(), [("acme/payments-*".to_owned(), "fin".to_owned())].into());
        assert_eq!(p.tag_owner("owner", &["acme/Payments-API".to_owned()]), Some("fin"));
        assert_eq!(p.tag_owner("owner", &["acme/web".to_owned()]), None);
        assert_eq!(p.tag_owner("team", &["acme/payments-api".to_owned()]), None);
    }
}
