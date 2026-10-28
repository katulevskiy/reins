//! An org policy as code (`docs/design/decisions-v2/05-organizations.md` §7.3, `02-policy-and-authority.md` §7): the
//! TOML files of an org's policy repo (`acme/reins-policy`) compile, deterministically, to one [`OrgPolicy`] and the
//! rollout plan it ships with, and an [`OrgPolicy`] renders back to those files (console drafts and templates export to
//! the repo). `compile(render(p)) == p`.
//!
//! ```toml
//! # reins-policy.toml
//! version = 1
//! org = "acme"
//! explain = "Acme policy"
//! cell_review = { reviewers = "requester", class = "instant", window = "2m" }
//! [rollout]
//! pilot_group = "Platform"
//! stages = ["pilot", "25%", "100%"]
//! dwell = "24h"
//! guards = { max_ask_rate_delta_pp = 2.0, max_median_ttd = "30m", min_decisions = 50 }
//!
//! # rules/prod.toml
//! [[rule]]
//! id = "prod-deploys"
//! explain = "Acme requires Security to approve production deploys"
//! when = { resource = "acme/prod-*", effects = ["Infra", "GitPush"], levels = ["L2", "L3"] }
//! review = { reviewers = "any of Security", quorum = 1, class = "deferred", window = "4h" }
//! ```
//!
//! Also `matrix.toml` (cells per zone), `facts.toml` (verified facts) and `posture.toml` (what a deciding device must
//! show). Every table refuses unknown keys; every error names its file and line. Groups and people are named as people
//! write them (`any of Security`, `person: kim@acme.com`): the server maps them to ids ([`map_reviewers`]).
//!
//! Pure: the repo is input, never authority. A compiled policy is a draft; activating it takes two Security admins.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use reins_proto::org::Role;
use reins_proto::rollout::{Guards, RolloutPlan, Stage};
use serde::Deserialize;
use toml::Spanned;

use crate::context::{Facts, Zone};
use crate::effects::{EffectKind, Level};
use crate::matrix::{Cell, Issuer, Matrix, Rule, Then, When};
use crate::org::{Hours, ORG_POLICY_VERSION, OrgPolicy, OrgRule, PostureRule, ReviewClass, ReviewSpec, Reviewers};

/// The files a policy repo is read from; anything else in the repo is ignored.
pub const MAIN_FILE: &str = "reins-policy.toml";
pub const MATRIX_FILE: &str = "matrix.toml";
pub const FACTS_FILE: &str = "facts.toml";
pub const POSTURE_FILE: &str = "posture.toml";
pub const RULES_DIR: &str = "rules/";

/// The most policy files a repo may have, and their largest total size: beyond either it isn't read at all.
pub const MAX_FILES: usize = 200;
pub const MAX_BYTES: usize = 1024 * 1024;

/// Whether `path` (relative, `/`-separated) is one of the files a policy is compiled from.
#[must_use]
pub fn is_policy_file(path: &str) -> bool {
    [MAIN_FILE, MATRIX_FILE, FACTS_FILE, POSTURE_FILE].contains(&path) || is_rules_file(path)
}

/// Refuses a repo over [`MAX_FILES`] policy files or [`MAX_BYTES`] of them.
pub fn check_size(files: &BTreeMap<String, String>) -> Result<(), String> {
    let policy: Vec<&String> = files.iter().filter(|(p, _)| is_policy_file(p)).map(|(_, t)| t).collect();
    if policy.len() > MAX_FILES {
        return Err(format!("more than {MAX_FILES} policy files"));
    }
    if policy.iter().map(|t| t.len()).sum::<usize>() > MAX_BYTES {
        return Err("policy files over 1 MiB".to_owned());
    }
    Ok(())
}

/// One stage as written: `pilot`, `25%`, `100%` (or `everyone`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageSource {
    Pilot,
    Percent(u8),
    All,
}

/// The rollout plan as written. Pilot members are resolved from `pilot_group` when the policy is activated.
#[derive(Clone, Debug, PartialEq, serde::Serialize, Deserialize)]
#[serde(default)]
pub struct RolloutPlanSource {
    pub pilot_group: Option<String>,
    pub stages: Vec<StageSource>,
    pub dwell_secs: u64,
    pub guards: Guards,
}

impl Default for RolloutPlanSource {
    fn default() -> Self {
        Self {
            pilot_group: None,
            stages: Vec::new(),
            dwell_secs: reins_proto::rollout::DEFAULT_DWELL_SECS,
            guards: Guards::default(),
        }
    }
}

impl RolloutPlanSource {
    /// The plan with the pilot group's members (user ids) filled in.
    #[must_use]
    pub fn resolve(&self, mut pilot_members: Vec<String>) -> RolloutPlan {
        pilot_members.sort();
        pilot_members.dedup();
        RolloutPlan {
            stages: self
                .stages
                .iter()
                .map(|s| match s {
                    StageSource::Pilot => Stage::Pilot {
                        members: pilot_members.clone(),
                    },
                    StageSource::Percent(p) => Stage::Percent(*p),
                    StageSource::All => Stage::All,
                })
                .collect(),
            dwell_secs: self.dwell_secs,
            guards: self.guards,
        }
    }
}

/// A compiled policy repo.
#[derive(Clone, Debug, PartialEq)]
pub struct Compiled {
    pub policy: OrgPolicy,
    /// [`OrgPolicy::hash`] of `policy` as compiled (the server's saved draft adds its revision and verified facts).
    pub hash: String,
    pub plan: RolloutPlanSource,
}

/// What's wrong, and where.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceError {
    pub file: String,
    /// 1-based; 0 when the file itself is missing.
    pub line: usize,
    pub message: String,
}

impl std::fmt::Display for SourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}: {}", self.file, self.line, self.message)
    }
}

// --- The files ---------------------------------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MainFile {
    version: Spanned<u32>,
    org: Spanned<String>,
    #[serde(default)]
    explain: String,
    #[serde(default)]
    cell_review: Option<ReviewSrc>,
    #[serde(default)]
    rollout: Option<RolloutSrc>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReviewSrc {
    reviewers: Spanned<String>,
    #[serde(default)]
    quorum: Option<Spanned<u8>>,
    #[serde(default)]
    distinct_persons: Option<bool>,
    #[serde(default)]
    exclude_requester: Option<bool>,
    class: Spanned<String>,
    #[serde(default)]
    window: Option<Spanned<String>>,
    /// Who it escalates to, step by step, when a window passes unanswered (reviewer expressions).
    #[serde(default)]
    escalate: Vec<Spanned<String>>,
    /// Breaks quiet hours.
    #[serde(default)]
    urgent: bool,
    /// A time-boxed approval: valid this long after the decision.
    #[serde(default)]
    approval_valid: Option<Spanned<String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RolloutSrc {
    #[serde(default)]
    pilot_group: Option<Spanned<String>>,
    #[serde(default)]
    stages: Vec<Spanned<String>>,
    #[serde(default)]
    dwell: Option<Spanned<String>>,
    #[serde(default)]
    guards: Option<GuardsSrc>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GuardsSrc {
    #[serde(default)]
    max_ask_rate_delta_pp: Option<Spanned<toml::Value>>,
    #[serde(default)]
    max_median_ttd: Option<Spanned<String>>,
    #[serde(default)]
    min_decisions: Option<u32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MatrixFile {
    #[serde(default)]
    mine: Option<[Cell; 4]>,
    #[serde(default)]
    friends: Option<[Cell; 4]>,
    #[serde(default)]
    work: Option<[Cell; 4]>,
    #[serde(default)]
    outside: Option<[Cell; 4]>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields, default)]
struct FactsFile {
    owners: BTreeMap<String, Zone>,
    repos: BTreeMap<String, Zone>,
    own_addresses: BTreeSet<String>,
    domains: BTreeMap<String, Zone>,
    people: BTreeMap<String, Zone>,
    hosts: BTreeMap<String, Zone>,
    chats: BTreeMap<String, Zone>,
    /// What an id-keyed fact is shown as (`owner:#583231` → `dkat`): decides nothing.
    labels: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PostureFile {
    #[serde(default)]
    screen_lock: Option<bool>,
    #[serde(default)]
    no_root: Option<bool>,
    #[serde(default)]
    attested: Option<bool>,
    #[serde(default)]
    min_os: BTreeMap<String, String>,
    #[serde(default)]
    min_app: Option<String>,
    #[serde(default)]
    max_age_days: Option<u32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RulesFile {
    #[serde(default)]
    rule: Vec<RuleSrc>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RuleSrc {
    id: Spanned<String>,
    #[serde(default)]
    explain: String,
    #[serde(default)]
    when: Option<WhenSrc>,
    #[serde(default)]
    deny: Option<Spanned<bool>>,
    #[serde(default)]
    permit: Option<Spanned<bool>>,
    #[serde(default)]
    review: Option<ReviewSrc>,
    /// Unix seconds: the rule stops applying then.
    #[serde(default)]
    not_after: Option<i64>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct WhenSrc {
    #[serde(default)]
    zones: Vec<Spanned<String>>,
    #[serde(default)]
    levels: Vec<Spanned<String>>,
    #[serde(default)]
    effects: Vec<Spanned<String>>,
    #[serde(default)]
    resource: Option<String>,
    #[serde(default)]
    connection: Option<String>,
    /// Only requests by these members (emails).
    #[serde(default)]
    members: Vec<Spanned<String>>,
    /// Permits only: the agent gave a reason.
    #[serde(default)]
    require_reason: bool,
    /// It applies only outside every one of these windows.
    #[serde(default)]
    outside_hours: Vec<HoursSrc>,
    /// Deny and review only: a payment of at least this many minor units (in `currency`, when named).
    #[serde(default)]
    min_amount: Option<u64>,
    #[serde(default)]
    currency: Option<String>,
    /// Deny and review only: a party the org's facts don't know.
    #[serde(default)]
    new_party: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HoursSrc {
    days: Vec<Spanned<String>>,
    start: String,
    end: String,
    tz: String,
}

// --- Compiling ---------------------------------------------------------------------------------------------------

/// Errors for one file, with the byte offsets of its text turned into lines.
struct Errs<'a> {
    file: &'a str,
    text: &'a str,
    out: &'a mut Vec<SourceError>,
}

impl Errs<'_> {
    fn at(&mut self, offset: usize, message: impl Into<String>) {
        let line = self.text.get(..offset.min(self.text.len())).map_or(1, |t| t.matches('\n').count() + 1);
        self.out.push(SourceError {
            file: self.file.to_owned(),
            line,
            message: message.into(),
        });
    }

    fn span<T>(&mut self, v: &Spanned<T>, message: impl Into<String>) {
        self.at(v.span().start, message);
    }
}

fn parse_file<T: for<'de> Deserialize<'de>>(file: &str, text: &str, out: &mut Vec<SourceError>) -> Option<T> {
    match toml::from_str::<T>(text) {
        Ok(v) => Some(v),
        Err(e) => {
            let mut errs = Errs {
                file,
                text,
                out,
            };
            errs.at(e.span().map_or(0, |s| s.start), e.message().trim().to_owned());
            None
        }
    }
}

/// `90s`, `2m`, `4h`, `7d` (or a bare number of seconds) as seconds.
pub fn parse_duration(text: &str) -> Result<u64, String> {
    let t = text.trim();
    let (num, unit) = t.find(|c: char| !c.is_ascii_digit()).map_or((t, ""), |i| t.split_at(i));
    let n: u64 = num.parse().map_err(|_| format!("{text:?} isn't a duration (like 90s, 2m, 4h or 7d)"))?;
    let mul = match unit.trim() {
        "" | "s" => 1,
        "m" => 60,
        "h" => 3600,
        "d" => 86_400,
        _ => return Err(format!("{text:?} isn't a duration (like 90s, 2m, 4h or 7d)")),
    };
    n.checked_mul(mul).ok_or_else(|| format!("{text:?} is too long"))
}

/// Seconds in the largest unit that writes them exactly.
#[must_use]
pub fn format_duration(secs: u64) -> String {
    match secs {
        0 => "0s".to_owned(),
        s if s % 86_400 == 0 => format!("{}d", s / 86_400),
        s if s % 3600 == 0 => format!("{}h", s / 3600),
        s if s % 60 == 0 => format!("{}m", s / 60),
        s => format!("{s}s"),
    }
}

/// Who reviews, as written:
/// - `requester`;
/// - `any of <Group>` (the group's name);
/// - `role: <role>` (`security_admin`, or its label, `Security admin`);
/// - `person: <email>[, <email>…]`;
/// - `manager`, `code owners`, `on-call: <alias>`, `resource owner: <tag>`, `backup of <Group>`;
/// - `A OR B` (anyone either admits; flat).
///
/// Two obligations on one request are two rules: there is no `AND`.
pub fn parse_reviewers(expr: &str) -> Result<Reviewers, String> {
    let e = expr.trim();
    let lower = e.to_ascii_lowercase();
    if lower.contains(" and ") {
        return Err("two reviews on one request are two rules (there is no AND)".to_owned());
    }
    if lower.contains(" or ") {
        let mut parts = Vec::new();
        let mut rest = e;
        while let Some(at) = rest.to_ascii_lowercase().find(" or ") {
            parts.push(parse_one(&rest[..at])?);
            rest = &rest[at + 4..];
        }
        parts.push(parse_one(rest)?);
        return Ok(Reviewers::AnyOf(parts));
    }
    parse_one(e)
}

fn parse_one(expr: &str) -> Result<Reviewers, String> {
    let e = expr.trim();
    let lower = e.to_ascii_lowercase();
    match lower.as_str() {
        "requester" => return Ok(Reviewers::Requester),
        "manager" => return Ok(Reviewers::Manager),
        "code owners" => return Ok(Reviewers::CodeOwners),
        _ => {}
    }
    let named = |rest: &str, what: &str| {
        let v = rest.trim();
        if v.is_empty() {
            Err(format!("name the {what}"))
        } else {
            Ok(v.to_owned())
        }
    };
    if let Some(rest) = strip_prefix_ci(e, "any of ") {
        return named(rest, "group: any of <Group>").map(Reviewers::Group);
    }
    if let Some(rest) = strip_prefix_ci(e, "backup of ") {
        return named(rest, "group: backup of <Group>").map(Reviewers::BackupOf);
    }
    if let Some(rest) = strip_prefix_ci(e, "on-call:") {
        return named(rest, "schedule: on-call: <alias>").map(|alias| Reviewers::OnCall {
            alias,
        });
    }
    if let Some(rest) = strip_prefix_ci(e, "resource owner:") {
        return named(rest, "tag: resource owner: <tag>").map(|tag| Reviewers::ResourceOwner {
            tag,
        });
    }
    if let Some(rest) = strip_prefix_ci(e, "role:") {
        let r = rest.trim();
        return Role::parse(r)
            .or_else(|| Role::ALL.into_iter().find(|x| x.label().eq_ignore_ascii_case(r)))
            .map(Reviewers::Role)
            .ok_or_else(|| format!("{r:?} isn't a role"));
    }
    if let Some(rest) = strip_prefix_ci(e, "person:") {
        let people: Vec<String> =
            rest.split(',').map(|p| p.trim().to_ascii_lowercase()).filter(|p| !p.is_empty()).collect();
        if people.is_empty() || people.iter().any(|p| !p.contains('@')) {
            return Err("name people by email: person: kim@acme.com, lee@acme.com".to_owned());
        }
        return Ok(Reviewers::Members(people));
    }
    Err(format!(
        "{e:?} isn't a reviewer (requester, any of <Group>, role: <role>, person: <email>, manager, code owners, \
on-call: <alias>, resource owner: <tag>, backup of <Group>)"
    ))
}

fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    (s.len() >= prefix.len() && s.is_char_boundary(prefix.len()) && s[..prefix.len()].eq_ignore_ascii_case(prefix))
        .then(|| &s[prefix.len()..])
}

/// [`Reviewers`] written back ([`parse_reviewers`]' forms).
#[must_use]
pub fn reviewers_text(r: &Reviewers) -> String {
    match r {
        Reviewers::Requester => "requester".to_owned(),
        Reviewers::Group(g) => format!("any of {g}"),
        Reviewers::Role(role) => format!("role: {}", role.as_str()),
        Reviewers::Members(m) => format!("person: {}", m.join(", ")),
        Reviewers::Manager => "manager".to_owned(),
        Reviewers::CodeOwners => "code owners".to_owned(),
        Reviewers::OnCall {
            alias,
        } => format!("on-call: {alias}"),
        Reviewers::ResourceOwner {
            tag,
        } => format!("resource owner: {tag}"),
        Reviewers::BackupOf(g) => format!("backup of {g}"),
        Reviewers::AnyOf(list) => list.iter().map(reviewers_text).collect::<Vec<_>>().join(" OR "),
    }
}

fn parse_effect(name: &str) -> Option<EffectKind> {
    let want: String = name.chars().filter(|c| *c != '_').collect::<String>().to_ascii_lowercase();
    EFFECTS.iter().copied().find(|k| {
        serde_json::to_value(k).ok().and_then(|v| v.as_str().map(|s| s.replace('_', ""))).as_deref() == Some(&want)
    })
}

/// `git_push` → `GitPush`.
fn effect_name(k: EffectKind) -> String {
    let snake = serde_json::to_value(k).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default();
    snake
        .split('_')
        .map(|w| {
            let mut c = w.chars();
            c.next().map(|f| f.to_ascii_uppercase().to_string() + c.as_str()).unwrap_or_default()
        })
        .collect()
}

const EFFECTS: [EffectKind; 17] = [
    EffectKind::FileRead,
    EffectKind::FileWrite,
    EffectKind::FileDelete,
    EffectKind::GitPush,
    EffectKind::GitRefDelete,
    EffectKind::Message,
    EffectKind::Publish,
    EffectKind::Permission,
    EffectKind::Money,
    EffectKind::SecretOut,
    EffectKind::SecretUse,
    EffectKind::Network,
    EffectKind::Infra,
    EffectKind::Exec,
    EffectKind::DataRead,
    EffectKind::DataWrite,
    EffectKind::Unknown,
];

fn parse_level(s: &str) -> Option<Level> {
    match s.trim().to_ascii_uppercase().as_str() {
        "L0" => Some(Level::L0),
        "L1" => Some(Level::L1),
        "L2" => Some(Level::L2),
        "L3" => Some(Level::L3),
        _ => None,
    }
}

fn parse_zone(s: &str) -> Option<Zone> {
    Zone::ALL.into_iter().find(|z| zone_name(*z).eq_ignore_ascii_case(s.trim()))
}

fn zone_name(z: Zone) -> &'static str {
    match z {
        Zone::Mine => "mine",
        Zone::Friends => "friends",
        Zone::Work => "work",
        Zone::Outside => "outside",
    }
}

fn review_spec(src: &ReviewSrc, e: &mut Errs<'_>) -> Option<ReviewSpec> {
    let mut escalate = Vec::new();
    for step in &src.escalate {
        match parse_reviewers(step.get_ref()) {
            Ok(r) => escalate.push(r),
            Err(m) => {
                e.span(step, m);
                return None;
            }
        }
    }
    let approval_valid_secs = match &src.approval_valid {
        None => None,
        Some(v) => match parse_duration(v.get_ref()) {
            Ok(s) => Some(s),
            Err(m) => {
                e.span(v, m);
                return None;
            }
        },
    };
    let reviewers = match parse_reviewers(src.reviewers.get_ref()) {
        Ok(r) => r,
        Err(m) => {
            e.span(&src.reviewers, m);
            return None;
        }
    };
    let class = match src.class.get_ref().trim().to_ascii_lowercase().as_str() {
        "instant" => ReviewClass::Instant,
        "deferred" => ReviewClass::Deferred,
        _ => {
            e.span(&src.class, "class is instant or deferred");
            return None;
        }
    };
    let window_secs = match &src.window {
        None => 0,
        Some(w) => match parse_duration(w.get_ref()) {
            Ok(s) => s,
            Err(m) => {
                e.span(w, m);
                return None;
            }
        },
    };
    let quorum = src.quorum.as_ref().map_or(1, |q| *q.get_ref());
    let spec = ReviewSpec {
        exclude_requester: src.exclude_requester.unwrap_or(reviewers != Reviewers::Requester),
        reviewers,
        quorum,
        distinct_persons: src.distinct_persons.unwrap_or(true),
        class,
        window_secs,
        escalate,
        urgent: src.urgent,
        approval_valid_secs,
    };
    // The same checks the policy itself makes, at this table's line.
    let mut probe = OrgPolicy::empty("x");
    probe.cell_review = spec.clone();
    if let Err(m) = probe.validate() {
        e.span(&src.reviewers, m.trim_start_matches("cell review: ").to_owned());
        return None;
    }
    Some(spec)
}

fn rollout_plan(src: Option<&RolloutSrc>, e: &mut Errs<'_>) -> RolloutPlanSource {
    let mut plan = RolloutPlanSource::default();
    let Some(src) = src else {
        return plan;
    };
    plan.pilot_group = src.pilot_group.as_ref().map(|g| g.get_ref().trim().to_owned());
    for s in &src.stages {
        let t = s.get_ref().trim().to_ascii_lowercase();
        let stage = match t.as_str() {
            "pilot" => Some(StageSource::Pilot),
            "100%" | "everyone" | "all" => Some(StageSource::All),
            p => p.strip_suffix('%').and_then(|n| n.trim().parse::<u8>().ok()).map(StageSource::Percent),
        };
        match stage {
            Some(st) => plan.stages.push(st),
            None => e.span(s, format!("{:?} isn't a stage (pilot, 25%, 100%)", s.get_ref())),
        }
    }
    if let Some(d) = &src.dwell {
        match parse_duration(d.get_ref()) {
            Ok(s) => plan.dwell_secs = s,
            Err(m) => e.span(d, m),
        }
    }
    if let Some(g) = &src.guards {
        if let Some(v) = &g.max_ask_rate_delta_pp {
            match v
                .get_ref()
                .as_float()
                .or_else(|| v.get_ref().as_integer().and_then(|i| i32::try_from(i).ok()).map(f64::from))
            {
                Some(f) if f.is_finite() && f >= 0.0 => plan.guards.max_ask_rate_delta_pp = f,
                _ => e.span(v, "max_ask_rate_delta_pp is a number of percentage points"),
            }
        }
        if let Some(t) = &g.max_median_ttd {
            match parse_duration(t.get_ref()) {
                Ok(s) => plan.guards.max_median_ttd_secs = s,
                Err(m) => e.span(t, m),
            }
        }
        if let Some(n) = g.min_decisions {
            plan.guards.min_decisions = n;
        }
    }
    let first = src.stages.first().map_or(0, |s| s.span().start);
    let has_pilot = plan.stages.contains(&StageSource::Pilot);
    match (&src.pilot_group, has_pilot) {
        (None, true) => e.at(first, "a pilot stage needs pilot_group"),
        (Some(g), false) => e.span(g, "pilot_group is set but no stage is \"pilot\""),
        (Some(g), true) if g.get_ref().trim().is_empty() => e.span(g, "name the pilot group"),
        _ => {}
    }
    if let Err(m) = plan.resolve(Vec::new()).validate() {
        e.at(first, m);
    }
    plan
}

fn rule_of(org: &str, src: &RuleSrc, e: &mut Errs<'_>) -> Option<OrgRule> {
    let when_src = src.when.as_ref();
    let mut outside_hours = Vec::new();
    if let Some(w) = when_src {
        for h in &w.outside_hours {
            let mut days = Vec::new();
            for d in &h.days {
                let Ok(day) = d.get_ref().trim().parse::<chrono::Weekday>() else {
                    e.span(d, format!("{:?} isn't a day (Mon to Sun)", d.get_ref()));
                    return None;
                };
                days.push(day);
            }
            outside_hours.push(Hours {
                days,
                start: h.start.trim().to_owned(),
                end: h.end.trim().to_owned(),
                tz: h.tz.trim().to_owned(),
            });
        }
    }
    let mut when = When::default();
    let mut ok = true;
    if let Some(w) = when_src {
        for z in &w.zones {
            if let Some(zone) = parse_zone(z.get_ref()) {
                when.zones.push(zone);
            } else {
                e.span(z, format!("{:?} isn't a zone (mine, friends, work, outside)", z.get_ref()));
                ok = false;
            }
        }
        for l in &w.levels {
            if let Some(level) = parse_level(l.get_ref()) {
                when.levels.push(level);
            } else {
                e.span(l, format!("{:?} isn't a level (L0 to L3)", l.get_ref()));
                ok = false;
            }
        }
        for k in &w.effects {
            if let Some(kind) = parse_effect(k.get_ref()) {
                when.effects.push(kind);
            } else {
                e.span(k, format!("{:?} isn't an effect kind", k.get_ref()));
                ok = false;
            }
        }
        when.resource = w.resource.as_deref().map(|r| r.trim().to_ascii_lowercase());
        when.connection.clone_from(&w.connection);
    }
    let set = |b: &Option<Spanned<bool>>| b.as_ref().is_some_and(|v| *v.get_ref());
    let chosen = usize::from(set(&src.deny)) + usize::from(set(&src.permit)) + usize::from(src.review.is_some());
    if chosen != 1 {
        e.span(&src.id, "a rule has exactly one of deny = true, permit = true or review = {…}");
        return None;
    }
    let (then, review) = if set(&src.deny) {
        (Then::Deny, None)
    } else if set(&src.permit) {
        (Then::Permit, None)
    } else {
        let spec = review_spec(src.review.as_ref()?, e)?;
        (Then::Review, Some(spec))
    };
    if !ok {
        return None;
    }
    let mut out = OrgRule::new(
        Rule {
            id: src.id.get_ref().clone(),
            issuer: Issuer::Org(org.to_owned()),
            when,
            then,
            not_after: src.not_after,
            accepted: false,
            explain: src.explain.clone(),
        },
        review,
    );
    if let Some(w) = when_src {
        out.members = w.members.iter().map(|m| m.get_ref().trim().to_ascii_lowercase()).collect();
        out.require_reason = w.require_reason;
        out.min_amount_minor = w.min_amount;
        out.currency = w.currency.as_ref().map(|c| c.trim().to_ascii_uppercase());
        out.new_party = w.new_party;
    }
    out.outside_hours = outside_hours;
    Some(out)
}

fn is_rules_file(path: &str) -> bool {
    path.starts_with(RULES_DIR)
        && std::path::Path::new(path).extension().is_some_and(|x| x.eq_ignore_ascii_case("toml"))
        && !path.contains("/.")
        && !path.contains("..")
}

/// Compiles a policy repo's files (path → text; other paths are ignored). Rules are sorted by id; the whole policy
/// passes [`OrgPolicy::validate`]. Every error names its file and line.
pub fn compile(files: &BTreeMap<String, String>) -> Result<Compiled, Vec<SourceError>> {
    let mut errors: Vec<SourceError> = Vec::new();
    let Some(main_text) = files.get(MAIN_FILE) else {
        return Err(vec![SourceError {
            file: MAIN_FILE.to_owned(),
            line: 0,
            message: "missing: a policy repo starts with reins-policy.toml".to_owned(),
        }]);
    };
    let Some(main) = parse_file::<MainFile>(MAIN_FILE, main_text, &mut errors) else {
        return Err(errors);
    };
    let mut policy = OrgPolicy::empty(main.org.get_ref().trim());
    let plan;
    {
        let mut e = Errs {
            file: MAIN_FILE,
            text: main_text,
            out: &mut errors,
        };
        if *main.version.get_ref() != ORG_POLICY_VERSION {
            e.span(
                &main.version,
                format!("version {} isn't supported (use {ORG_POLICY_VERSION})", main.version.get_ref()),
            );
        }
        if policy.org.is_empty() {
            e.span(&main.org, "name the organization");
        }
        policy.explain.clone_from(&main.explain);
        if let Some(cr) = &main.cell_review
            && let Some(spec) = review_spec(cr, &mut e)
        {
            policy.cell_review = spec;
        }
        plan = rollout_plan(main.rollout.as_ref(), &mut e);
    }
    if let Some(text) = files.get(MATRIX_FILE)
        && let Some(m) = parse_file::<MatrixFile>(MATRIX_FILE, text, &mut errors)
    {
        let d = Matrix::default();
        policy.matrix = Matrix {
            mine: m.mine.unwrap_or(d.mine),
            friends: m.friends.unwrap_or(d.friends),
            work: m.work.unwrap_or(d.work),
            outside: m.outside.unwrap_or(d.outside),
        };
    }
    if let Some(text) = files.get(FACTS_FILE)
        && let Some(f) = parse_file::<FactsFile>(FACTS_FILE, text, &mut errors)
    {
        policy.facts = Facts {
            owners: lower_keys(f.owners),
            repos: lower_keys(f.repos),
            own_addresses: f.own_addresses.into_iter().map(|a| a.trim().to_ascii_lowercase()).collect(),
            domains: lower_keys(f.domains),
            people: lower_keys(f.people),
            hosts: lower_keys(f.hosts),
            chats: lower_keys(f.chats),
            labels: f.labels,
            github_ids: BTreeMap::new(),
        };
    }
    if let Some(text) = files.get(POSTURE_FILE)
        && let Some(p) = parse_file::<PostureFile>(POSTURE_FILE, text, &mut errors)
    {
        let d = PostureRule::default();
        policy.posture = PostureRule {
            screen_lock: p.screen_lock.unwrap_or(d.screen_lock),
            no_root: p.no_root.unwrap_or(d.no_root),
            attested: p.attested.unwrap_or(d.attested),
            min_os: p.min_os,
            min_app: p.min_app,
            max_age_days: p.max_age_days.unwrap_or(d.max_age_days),
        };
    }
    // Where each rule id was written, for errors the whole policy finds.
    let mut lines: BTreeMap<String, (String, usize)> = BTreeMap::new();
    for (path, text) in files.iter().filter(|(p, _)| is_rules_file(p)) {
        let Some(rf) = parse_file::<RulesFile>(path, text, &mut errors) else {
            continue;
        };
        let mut e = Errs {
            file: path,
            text,
            out: &mut errors,
        };
        for src in &rf.rule {
            let id = src.id.get_ref().clone();
            let line = text.get(..src.id.span().start).map_or(1, |t| t.matches('\n').count() + 1);
            if let Some((other, other_line)) = lines.get(&id) {
                e.span(&src.id, format!("rule id {id} is already used at {other}:{other_line}"));
                continue;
            }
            lines.insert(id, (path.clone(), line));
            if let Some(rule) = rule_of(&policy.org, src, &mut e) {
                policy.rules.push(rule);
            }
        }
    }
    policy.rules.sort_by(|a, b| a.rule.id.cmp(&b.rule.id));
    if errors.is_empty()
        && let Err(message) = policy.validate()
    {
        let at = lines
            .iter()
            .find(|(id, _)| message.contains(&format!("rule {id}")) || message.contains(&format!("{id:?}")));
        let (file, line) = at.map_or((MAIN_FILE.to_owned(), 1), |(_, (f, l))| (f.clone(), *l));
        errors.push(SourceError {
            file,
            line,
            message,
        });
    }
    if !errors.is_empty() {
        errors.sort_by(|a, b| (&a.file, a.line).cmp(&(&b.file, b.line)));
        return Err(errors);
    }
    Ok(Compiled {
        hash: policy.hash(),
        policy,
        plan,
    })
}

fn lower_keys(m: BTreeMap<String, Zone>) -> BTreeMap<String, Zone> {
    m.into_iter().map(|(k, v)| (k.trim().to_ascii_lowercase(), v)).collect()
}

/// Maps the reviewers' names the files use to the ids the server keeps, or back: `group` maps a group (`any of`,
/// `backup of`), `person` a member (`person:`, a rule's `members`). Stops at the first name either refuses.
pub fn map_reviewers(
    policy: &mut OrgPolicy,
    mut group: impl FnMut(&str) -> Result<String, String>,
    mut person: impl FnMut(&str) -> Result<String, String>,
) -> Result<(), String> {
    fn one(
        r: &mut Reviewers,
        group: &mut impl FnMut(&str) -> Result<String, String>,
        person: &mut impl FnMut(&str) -> Result<String, String>,
    ) -> Result<(), String> {
        match r {
            Reviewers::Group(g) | Reviewers::BackupOf(g) => *g = group(g)?,
            Reviewers::Members(list) => {
                for m in list.iter_mut() {
                    *m = person(m)?;
                }
            }
            Reviewers::AnyOf(list) => {
                for x in list.iter_mut() {
                    one(x, group, person)?;
                }
            }
            Reviewers::Requester
            | Reviewers::Role(_)
            | Reviewers::Manager
            | Reviewers::CodeOwners
            | Reviewers::OnCall {
                ..
            }
            | Reviewers::ResourceOwner {
                ..
            } => {}
        }
        Ok(())
    }
    for rule in &mut policy.rules {
        for m in &mut rule.members {
            *m = person(m)?;
        }
    }
    let specs =
        std::iter::once(&mut policy.cell_review).chain(policy.rules.iter_mut().filter_map(|r| r.review.as_mut()));
    for spec in specs {
        one(&mut spec.reviewers, &mut group, &mut person)?;
        for step in &mut spec.escalate {
            one(step, &mut group, &mut person)?;
        }
    }
    Ok(())
}

// --- Rendering ---------------------------------------------------------------------------------------------------

fn q(s: &str) -> String {
    toml::Value::String(s.to_owned()).to_string()
}

fn list(items: impl IntoIterator<Item = String>) -> String {
    format!("[{}]", items.into_iter().map(|s| q(&s)).collect::<Vec<_>>().join(", "))
}

fn review_inline(spec: &ReviewSpec) -> String {
    let mut parts = vec![format!("reviewers = {}", q(&reviewers_text(&spec.reviewers)))];
    if spec.quorum != 1 {
        parts.push(format!("quorum = {}", spec.quorum));
    }
    if !spec.distinct_persons {
        parts.push("distinct_persons = false".to_owned());
    }
    if spec.exclude_requester != (spec.reviewers != Reviewers::Requester) {
        parts.push(format!("exclude_requester = {}", spec.exclude_requester));
    }
    let class = match spec.class {
        ReviewClass::Instant => "instant",
        ReviewClass::Deferred => "deferred",
    };
    parts.push(format!("class = {}", q(class)));
    if spec.window_secs != 0 {
        parts.push(format!("window = {}", q(&format_duration(spec.window_secs))));
    }
    if !spec.escalate.is_empty() {
        parts.push(format!("escalate = {}", list(spec.escalate.iter().map(reviewers_text))));
    }
    if spec.urgent {
        parts.push("urgent = true".to_owned());
    }
    if let Some(v) = spec.approval_valid_secs {
        parts.push(format!("approval_valid = {}", q(&format_duration(v))));
    }
    format!("{{ {} }}", parts.join(", "))
}

fn cells(row: [Cell; 4]) -> String {
    list(
        row.iter()
            .map(|c| serde_json::to_value(c).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default()),
    )
}

fn zone_map(out: &mut String, table: &str, m: &BTreeMap<String, Zone>) {
    if m.is_empty() {
        return;
    }
    let _ = writeln!(out, "\n[{table}]");
    for (k, z) in m {
        let _ = writeln!(out, "{} = {}", q(k), q(zone_name(*z)));
    }
}

/// The id prefix a rule's file is named by: `prod-deploys` → `rules/prod.toml`.
fn rules_file_of(id: &str) -> String {
    let prefix = id.split(['-', '.', '_', '@']).next().filter(|p| !p.is_empty()).unwrap_or("rules");
    format!("{RULES_DIR}{prefix}.toml")
}

/// Renders `policy` and `plan` as a policy repo's files (one rules file per id prefix). The policy's `revision` is
/// the server's and isn't written.
#[must_use]
pub fn render(policy: &OrgPolicy, plan: &RolloutPlanSource) -> BTreeMap<String, String> {
    let mut files = BTreeMap::new();
    let mut main = String::new();
    let _ = writeln!(main, "version = {}", policy.version);
    let _ = writeln!(main, "org = {}", q(&policy.org));
    if !policy.explain.is_empty() {
        let _ = writeln!(main, "explain = {}", q(&policy.explain));
    }
    if policy.cell_review != ReviewSpec::owner() {
        let _ = writeln!(main, "cell_review = {}", review_inline(&policy.cell_review));
    }
    if *plan != RolloutPlanSource::default() {
        let _ = writeln!(main, "\n[rollout]");
        if let Some(g) = &plan.pilot_group {
            let _ = writeln!(main, "pilot_group = {}", q(g));
        }
        let stages = plan.stages.iter().map(|s| match s {
            StageSource::Pilot => "pilot".to_owned(),
            StageSource::Percent(p) => format!("{p}%"),
            StageSource::All => "100%".to_owned(),
        });
        let _ = writeln!(main, "stages = {}", list(stages));
        let _ = writeln!(main, "dwell = {}", q(&format_duration(plan.dwell_secs)));
        let g = &plan.guards;
        let _ = writeln!(
            main,
            "guards = {{ max_ask_rate_delta_pp = {:?}, max_median_ttd = {}, min_decisions = {} }}",
            g.max_ask_rate_delta_pp,
            q(&format_duration(g.max_median_ttd_secs)),
            g.min_decisions
        );
    }
    files.insert(MAIN_FILE.to_owned(), main);
    let m = &policy.matrix;
    files.insert(
        MATRIX_FILE.to_owned(),
        format!(
            "# Cells per zone, by level L0 to L3: auto, ask_first or ask.\nmine = {}\nfriends = {}\nwork = {}\noutside = {}\n",
            cells(m.mine),
            cells(m.friends),
            cells(m.work),
            cells(m.outside)
        ),
    );
    if policy.facts != Facts::default() {
        let f = &policy.facts;
        let mut out = String::new();
        if !f.own_addresses.is_empty() {
            let _ = writeln!(out, "own_addresses = {}", list(f.own_addresses.iter().cloned()));
        }
        for (table, m) in [
            ("owners", &f.owners),
            ("repos", &f.repos),
            ("domains", &f.domains),
            ("people", &f.people),
            ("hosts", &f.hosts),
            ("chats", &f.chats),
        ] {
            zone_map(&mut out, table, m);
        }
        if !f.labels.is_empty() {
            let _ = writeln!(out, "\n[labels]");
            for (k, v) in &f.labels {
                let _ = writeln!(out, "{} = {}", q(k), q(v));
            }
        }
        files.insert(FACTS_FILE.to_owned(), out.trim_start().to_owned());
    }
    if policy.posture != PostureRule::default() {
        let p = &policy.posture;
        let mut out = format!(
            "screen_lock = {}\nno_root = {}\nattested = {}\nmax_age_days = {}\n",
            p.screen_lock, p.no_root, p.attested, p.max_age_days
        );
        if let Some(a) = &p.min_app {
            let _ = writeln!(out, "min_app = {}", q(a));
        }
        if !p.min_os.is_empty() {
            let _ = writeln!(out, "\n[min_os]");
            for (os, v) in &p.min_os {
                let _ = writeln!(out, "{} = {}", q(os), q(v));
            }
        }
        files.insert(POSTURE_FILE.to_owned(), out);
    }
    let mut rule_files: BTreeMap<String, String> = BTreeMap::new();
    for r in &policy.rules {
        let out = rule_files.entry(rules_file_of(&r.rule.id)).or_default();
        if !out.is_empty() {
            out.push('\n');
        }
        let _ = writeln!(out, "[[rule]]\nid = {}", q(&r.rule.id));
        if !r.rule.explain.is_empty() {
            let _ = writeln!(out, "explain = {}", q(&r.rule.explain));
        }
        let w = &r.rule.when;
        let mut when = Vec::new();
        if !w.zones.is_empty() {
            when.push(format!("zones = {}", list(w.zones.iter().map(|z| zone_name(*z).to_owned()))));
        }
        if !w.levels.is_empty() {
            when.push(format!("levels = {}", list(w.levels.iter().map(|l| format!("{l:?}")))));
        }
        if !w.effects.is_empty() {
            when.push(format!("effects = {}", list(w.effects.iter().map(|k| effect_name(*k)))));
        }
        if let Some(res) = &w.resource {
            when.push(format!("resource = {}", q(res)));
        }
        if let Some(c) = &w.connection {
            when.push(format!("connection = {}", q(c)));
        }
        if !r.members.is_empty() {
            when.push(format!("members = {}", list(r.members.iter().cloned())));
        }
        if r.require_reason {
            when.push("require_reason = true".to_owned());
        }
        if !r.outside_hours.is_empty() {
            let hours: Vec<String> = r
                .outside_hours
                .iter()
                .map(|h| {
                    format!(
                        "{{ days = {}, start = {}, end = {}, tz = {} }}",
                        list(h.days.iter().map(|d| format!("{d:?}"))),
                        q(&h.start),
                        q(&h.end),
                        q(&h.tz)
                    )
                })
                .collect();
            when.push(format!("outside_hours = [{}]", hours.join(", ")));
        }
        if let Some(m) = r.min_amount_minor {
            when.push(format!("min_amount = {m}"));
        }
        if let Some(c) = &r.currency {
            when.push(format!("currency = {}", q(c)));
        }
        if r.new_party {
            when.push("new_party = true".to_owned());
        }
        if !when.is_empty() {
            let _ = writeln!(out, "when = {{ {} }}", when.join(", "));
        }
        if let Some(t) = r.rule.not_after {
            let _ = writeln!(out, "not_after = {t}");
        }
        match (r.rule.then, &r.review) {
            (Then::Deny, _) => out.push_str("deny = true\n"),
            (Then::Permit, _) => out.push_str("permit = true\n"),
            (Then::Review, Some(spec)) => {
                let _ = writeln!(out, "review = {}", review_inline(spec));
            }
            (Then::Review, None) => out.push_str("review = { reviewers = \"requester\", class = \"instant\" }\n"),
        }
    }
    files.extend(rule_files);
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_dir(root: &std::path::Path, dir: &std::path::Path, out: &mut BTreeMap<String, String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                read_dir(root, &path, out);
            } else {
                let rel = path.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
                out.insert(rel, std::fs::read_to_string(&path).unwrap());
            }
        }
    }

    pub(crate) fn sample() -> BTreeMap<String, String> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/policy_repo");
        let mut out = BTreeMap::new();
        read_dir(&root, &root, &mut out);
        out
    }

    fn with(files: &[(&str, &str)]) -> BTreeMap<String, String> {
        let mut m = sample();
        for (p, t) in files {
            m.insert((*p).to_owned(), (*t).to_owned());
        }
        m
    }

    fn errors(files: &BTreeMap<String, String>) -> Vec<SourceError> {
        compile(files).expect_err("it must not compile")
    }

    #[test]
    fn the_sample_repo_compiles() {
        let c = compile(&sample()).unwrap();
        let p = &c.policy;
        assert_eq!((p.org.as_str(), p.version, p.revision), ("acme", 1, 0));
        let ids: Vec<&str> = p.rules.iter().map(|r| r.rule.id.as_str()).collect();
        assert_eq!(ids, ["payments-review", "prod-deny-db-drop", "prod-deploys"], "sorted by id");
        let deploys = &p.rules[2];
        assert_eq!(deploys.rule.then, Then::Review);
        assert_eq!(deploys.rule.issuer, Issuer::Org("acme".into()));
        assert_eq!(deploys.rule.when.effects, [EffectKind::Infra, EffectKind::GitPush]);
        assert_eq!(deploys.rule.when.levels, [Level::L2, Level::L3]);
        assert_eq!(deploys.rule.when.resource.as_deref(), Some("acme/prod-*"));
        let spec = deploys.review.as_ref().unwrap();
        assert_eq!(spec.reviewers, Reviewers::Group("Security".into()));
        assert_eq!(
            (spec.class, spec.window_secs, spec.quorum, spec.exclude_requester),
            (ReviewClass::Deferred, 14_400, 1, true)
        );
        assert_eq!(p.cell_review, ReviewSpec::owner());
        assert_eq!(p.facts.owners.get("acme"), Some(&Zone::Work));
        assert_eq!(c.plan.pilot_group.as_deref(), Some("Platform"));
        assert_eq!(c.plan.stages, [StageSource::Pilot, StageSource::Percent(25), StageSource::All]);
        assert_eq!(
            (c.plan.dwell_secs, c.plan.guards.max_median_ttd_secs, c.plan.guards.min_decisions),
            (86_400, 1800, 50)
        );
        assert_eq!(c.hash, p.hash());
        // A README and CI files are ignored.
        assert!(sample().contains_key("README.md"));
    }

    fn identity(policy: &OrgPolicy, plan: &RolloutPlanSource) {
        let files = render(policy, plan);
        let back = compile(&files).unwrap_or_else(|e| panic!("{e:?}\n{files:#?}"));
        let mut want = policy.clone();
        want.revision = 0;
        assert_eq!(back.policy, want, "{files:#?}");
        assert_eq!(&back.plan, plan);
        // Rendering what was compiled gives the same files.
        assert_eq!(render(&back.policy, &back.plan), files);
    }

    #[test]
    fn render_then_compile_is_identity() {
        let c = compile(&sample()).unwrap();
        identity(&c.policy, &c.plan);
        let mut baseline: OrgPolicy =
            serde_json::from_str(include_str!("../../../src/api/reins/org/policy_baseline.json")).unwrap();
        baseline.org = "acme".into();
        identity(&baseline, &RolloutPlanSource::default());
        // Every kind of reviewer and rule, and a non-default posture.
        let mut p = c.policy.clone();
        p.posture.attested = true;
        p.posture.min_os.insert("iOS".into(), "18.0".into());
        p.posture.min_app = Some("1.2".into());
        p.cell_review = ReviewSpec {
            reviewers: Reviewers::Role(Role::Approver),
            quorum: 2,
            distinct_persons: false,
            exclude_requester: false,
            class: ReviewClass::Instant,
            window_secs: 90,
            escalate: Vec::new(),
            urgent: false,
            approval_valid_secs: None,
        };
        // O3's escalation, filters and windows render and read back too.
        {
            let r = &mut p.rules[1];
            r.members = vec!["kim@acme.com".into()];
            r.outside_hours = vec![Hours {
                days: vec![chrono::Weekday::Mon, chrono::Weekday::Fri],
                start: "09:00".into(),
                end: "17:30".into(),
                tz: "Europe/Berlin".into(),
            }];
            r.min_amount_minor = Some(50_000);
            r.currency = Some("EUR".into());
            r.new_party = true;
        }
        if let Some(spec) = p.rules[2].review.as_mut() {
            spec.escalate = vec![
                Reviewers::Manager,
                Reviewers::AnyOf(vec![Reviewers::CodeOwners, Reviewers::BackupOf("Security".into())]),
            ];
            spec.urgent = true;
            spec.approval_valid_secs = Some(3600);
        }
        p.rules[0].review.as_mut().unwrap().reviewers =
            Reviewers::Members(vec!["kim@acme.com".into(), "lee@acme.com".into()]);
        p.rules[0].rule.not_after = Some(2_000_000_000);
        p.rules[0].rule.when.zones = vec![Zone::Outside, Zone::Work];
        p.rules[0].rule.when.connection = Some("conn-1".into());
        p.facts.own_addresses.insert("ops@acme.com".into());
        p.facts.hosts.insert("api.acme.com".into(), Zone::Work);
        p.facts.chats.insert("-100123".into(), Zone::Friends);
        p.facts.labels.insert("chat:-100123".into(), "Team chat".into());
        p.revision = 9;
        identity(&p, &c.plan);
    }

    #[test]
    fn errors_name_file_and_line() {
        let bad = "[[rule]]\nid = \"a-1\"\ndeny = true\n\n[[rule]]\nid = \"a-2\"\nwhen = { levels = [\"L9\"] }\ndeny = true\n";
        let e = errors(&with(&[("rules/a.toml", bad)]));
        assert_eq!(e.len(), 1, "{e:?}");
        assert_eq!((e[0].file.as_str(), e[0].line), ("rules/a.toml", 7));
        assert!(e[0].message.contains("L9"), "{e:?}");
        assert_eq!(e[0].to_string(), format!("rules/a.toml:7: {}", e[0].message));
        // A TOML syntax error.
        let e = errors(&with(&[("matrix.toml", "mine = [\"auto\",\n")]));
        assert_eq!(e[0].file, "matrix.toml");
        assert!(e[0].line >= 1);
        // A bad reviewer expression, on its own line.
        let review = "[[rule]]\nid = \"x\"\n\nreview = { reviewers = \"everyone please\", class = \"deferred\" }\n";
        let e = errors(&with(&[("rules/x.toml", review)]));
        assert_eq!((e[0].file.as_str(), e[0].line), ("rules/x.toml", 4), "{e:?}");
        // The policy's own checks, at the rule's line: a duplicate id across files names both places.
        let dup = "[[rule]]\nid = \"prod-deploys\"\ndeny = true\n";
        let e = errors(&with(&[("rules/zzz.toml", dup)]));
        assert_eq!(e[0].file, "rules/zzz.toml");
        assert!(e[0].message.contains("rules/prod.toml"), "{e:?}");
        let e = errors(&with(&[("rules/zzz.toml", "[[rule]]\nid = \"Bad Id\"\ndeny = true\n")]));
        assert_eq!((e[0].file.as_str(), e[0].line), ("rules/zzz.toml", 2), "{e:?}");
        // No main file.
        let mut files = sample();
        files.remove(MAIN_FILE);
        assert_eq!(errors(&files)[0].line, 0);
    }

    #[test]
    fn unknown_keys_are_refused() {
        for (file, text) in [
            ("rules/a.toml", "[[rule]]\nid = \"a\"\ndeny = true\nseverity = \"high\"\n"),
            ("rules/a.toml", "[[rule]]\nid = \"a\"\nwhen = { repo = \"x\" }\ndeny = true\n"),
            ("matrix.toml", "mine = [\"auto\",\"auto\",\"auto\",\"ask\"]\nprod = [\"ask\",\"ask\",\"ask\",\"ask\"]\n"),
            ("facts.toml", "[teams]\nx = \"work\"\n"),
            ("posture.toml", "jailbreak_ok = true\n"),
            (MAIN_FILE, "version = 1\norg = \"acme\"\nmode = \"loose\"\n"),
        ] {
            let e = errors(&with(&[(file, text)]));
            assert_eq!(e[0].file, file, "{text}: {e:?}");
            assert!(e[0].message.contains("unknown"), "{text}: {e:?}");
        }
        // O3's filters and escalation are read, and checked as the policy checks them.
        let ok =
            "[[rule]]\nid = \"a\"\nwhen = { min_amount = 100, currency = \"usd\", new_party = true }\ndeny = true\n";
        let c = compile(&with(&[("rules/a.toml", ok)])).unwrap();
        let a = c.policy.rules.iter().find(|r| r.rule.id == "a").unwrap();
        assert_eq!((a.min_amount_minor, a.currency.as_deref(), a.new_party), (Some(100), Some("USD"), true));
        for text in [
            // require_reason is for permits only.
            "[[rule]]\nid = \"a\"\nwhen = { require_reason = true }\ndeny = true\n",
            "[[rule]]\nid = \"a\"\nwhen = { outside_hours = [{ days = [\"Funday\"], start = \"09:00\", end = \"17:00\", tz = \"UTC\" }] }\ndeny = true\n",
            "[[rule]]\nid = \"a\"\nreview = { reviewers = \"requester\", class = \"instant\", escalate = [\"nobody\"] }\n",
        ] {
            assert!(!errors(&with(&[("rules/a.toml", text)])).is_empty(), "{text}");
        }
        // Exactly one outcome.
        for text in ["[[rule]]\nid = \"a\"\n", "[[rule]]\nid = \"a\"\ndeny = true\npermit = true\n"] {
            assert!(errors(&with(&[("rules/a.toml", text)]))[0].message.contains("exactly one"));
        }
    }

    #[test]
    fn reviewer_expressions_parse() {
        assert_eq!(parse_reviewers("requester"), Ok(Reviewers::Requester));
        assert_eq!(parse_reviewers(" any of Platform Security "), Ok(Reviewers::Group("Platform Security".into())));
        assert_eq!(parse_reviewers("role: security_admin"), Ok(Reviewers::Role(Role::SecurityAdmin)));
        assert_eq!(parse_reviewers("Role: Security admin"), Ok(Reviewers::Role(Role::SecurityAdmin)));
        assert_eq!(
            parse_reviewers("person: Kim@acme.com, lee@acme.com"),
            Ok(Reviewers::Members(vec!["kim@acme.com".into(), "lee@acme.com".into()]))
        );
        for bad in ["", "any of ", "role: wizard", "person: kim", "everyone", "any of A and any of B", "on-call: "] {
            assert!(parse_reviewers(bad).is_err(), "{bad}");
        }
        assert_eq!(
            parse_reviewers("manager OR any of Security or on-call: payments"),
            Ok(Reviewers::AnyOf(vec![
                Reviewers::Manager,
                Reviewers::Group("Security".into()),
                Reviewers::OnCall {
                    alias: "payments".into()
                }
            ]))
        );
        for r in [
            Reviewers::Requester,
            Reviewers::Group("Security".into()),
            Reviewers::Role(Role::Auditor),
            Reviewers::Members(vec!["a@x.com".into()]),
            Reviewers::Manager,
            Reviewers::CodeOwners,
            Reviewers::OnCall {
                alias: "payments".into(),
            },
            Reviewers::ResourceOwner {
                tag: "db".into(),
            },
            Reviewers::BackupOf("Security".into()),
            Reviewers::AnyOf(vec![Reviewers::Manager, Reviewers::BackupOf("Security".into())]),
        ] {
            assert_eq!(parse_reviewers(&reviewers_text(&r)), Ok(r));
        }
        assert_eq!(parse_duration("4h"), Ok(14_400));
        assert_eq!(parse_duration("90"), Ok(90));
        assert!(parse_duration("4 hours").is_err());
        assert_eq!(format_duration(14_400), "4h");
        assert_eq!(format_duration(90), "90s");
    }

    #[test]
    fn compile_is_deterministic() {
        let files = sample();
        let a = compile(&files).unwrap();
        // The same rules in other files and another order: the same policy, the same hash.
        let mut moved = files;
        let prod = moved.remove("rules/prod.toml").unwrap();
        let blocks: Vec<&str> = prod.split("[[rule]]").filter(|b| !b.trim().is_empty()).collect();
        for (i, b) in blocks.iter().rev().enumerate() {
            moved.insert(format!("rules/team/{i}.toml"), format!("[[rule]]{b}"));
        }
        let b = compile(&moved).unwrap();
        assert_eq!(a.hash, b.hash);
        assert_eq!(a.policy, b.policy);
        // And map_reviewers maps names to ids, and back.
        let mut p = a.policy;
        map_reviewers(&mut p, |g| Ok(format!("gid:{g}")), |m| Ok(format!("uid:{m}"))).unwrap();
        assert_eq!(p.rules[2].review.as_ref().unwrap().reviewers, Reviewers::Group("gid:Security".into()));
        assert!(map_reviewers(&mut p, |_| Err("no such group".into()), |m| Ok(m.to_owned())).is_err());
    }
}
