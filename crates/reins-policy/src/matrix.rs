//! The zone × level matrix and policy evaluation (`docs/design/decisions-v2/01-decision-engine.md` §4,
//! `02-policy-and-authority.md` §3 as corrected by `08-review-resolutions.md` R3 and R4).
//!
//! - **Deny** comes only from an explicit rule (a "never" rule of the owner, a repo's or an org's). Auto never denies.
//! - **Hard obligations** (the always-ask floor, certainty `None`, anything L3, `review` rules) can't be satisfied by a
//!   permit.
//! - A matrix cell **Ask** is a soft default (`review_by_default`): any one qualifying permit satisfies it (permits
//!   combine as OR). A task's permit qualifies only inside its envelope and ceiling (an AND bound).
//!
//! Pure and deterministic: the same input and [`Snapshot`] always give the same [`Evaluation`], and the snapshot's
//! hash is recorded with it.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::context::{Facts, Resolved, Zone};
use crate::effects::{Analysis, Certainty, Effect, EffectKind, Level};

/// One cell of the matrix.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Cell {
    /// Permit, subject to the model's sanity check.
    Auto,
    /// Ask the owner (soft: a permit satisfies it).
    Ask,
    /// Ask the first time per resource, then Auto (01 §4, Friends L1).
    AskFirst,
}

impl Cell {
    /// Higher is stricter.
    fn strictness(self) -> u8 {
        match self {
            Self::Auto => 0,
            Self::AskFirst => 1,
            Self::Ask => 2,
        }
    }

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::Ask => "Ask",
            Self::AskFirst => "Ask first",
        }
    }
}

/// Cells per zone, indexed by level (L0..L3).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Matrix {
    pub mine: [Cell; 4],
    pub friends: [Cell; 4],
    pub work: [Cell; 4],
    pub outside: [Cell; 4],
}

impl Default for Matrix {
    /// 01 §4, Personal mode. (Mine L2 is "to Mine/Friends people": a request reaching anyone less trusted isn't Mine.)
    fn default() -> Self {
        use Cell::{Ask, AskFirst, Auto};
        Self {
            mine: [Auto, Auto, Auto, Ask],
            friends: [Auto, AskFirst, Auto, Ask],
            work: [Auto, Ask, Ask, Ask],
            outside: [Ask, Ask, Ask, Ask],
        }
    }
}

impl Matrix {
    fn row(&self, zone: Zone) -> &[Cell; 4] {
        match zone {
            Zone::Mine => &self.mine,
            Zone::Friends => &self.friends,
            Zone::Work => &self.work,
            Zone::Outside => &self.outside,
        }
    }

    #[must_use]
    pub fn cell(&self, zone: Zone, level: Level) -> Cell {
        self.row(zone)[level as usize]
    }

    pub fn set(&mut self, zone: Zone, level: Level, cell: Cell) {
        let row = match zone {
            Zone::Mine => &mut self.mine,
            Zone::Friends => &mut self.friends,
            Zone::Work => &mut self.work,
            Zone::Outside => &mut self.outside,
        };
        row[level as usize] = cell;
    }

    /// Whether setting `cell` at (`zone`, `level`) loosens the matrix (and so needs the owner check).
    #[must_use]
    pub fn loosens(&self, zone: Zone, level: Level, cell: Cell) -> bool {
        cell.strictness() < self.cell(zone, level).strictness()
    }
}

/// Who wrote a rule.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Issuer {
    Owner,
    /// A repository's rules file (`owner/name`).
    Repo(String),
    Org(String),
    /// An approved task (its id).
    Task(String),
}

impl Issuer {
    /// Which explanation shows first: the higher issuer's.
    fn rank(&self) -> u8 {
        match self {
            Self::Org(_) => 0,
            Self::Repo(_) => 1,
            Self::Owner => 2,
            Self::Task(_) => 3,
        }
    }
}

/// What a rule does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Then {
    Deny,
    /// A hard review: no permit satisfies it.
    Review,
    /// Satisfies a soft Ask cell.
    Permit,
}

/// Which requests a rule covers. An empty list matches anything.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct When {
    pub zones: Vec<Zone>,
    pub levels: Vec<Level>,
    /// The rule covers a request when it has an effect of one of these kinds.
    pub effects: Vec<EffectKind>,
    /// Exact (lowercase), or a prefix ending in `*`.
    pub resource: Option<String>,
    pub connection: Option<String>,
}

fn glob(pattern: &str, value: &str) -> bool {
    let (p, v) = (pattern.to_ascii_lowercase(), value.to_ascii_lowercase());
    match p.strip_suffix('*') {
        Some(prefix) => v.starts_with(prefix),
        None => p == v,
    }
}

/// How a rule has to cover a request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cover {
    /// A deny or a review: it applies when any effect kind or any party matches (stricter).
    Any,
    /// A permit: it applies only when it covers every effect kind and every party.
    All,
}

impl When {
    fn matches(&self, input: &Input<'_>, kinds: &[EffectKind], cover: Cover) -> bool {
        let effects = self.effects.is_empty()
            || match cover {
                Cover::Any => kinds.iter().any(|k| self.effects.contains(k)),
                Cover::All => kinds.iter().all(|k| self.effects.contains(k)),
            };
        let resource = self.resource.as_deref().is_none_or(|r| {
            let parts = &input.resolved.parts;
            match cover {
                Cover::Any => parts.iter().any(|v| glob(r, v)),
                Cover::All => !parts.is_empty() && parts.iter().all(|v| glob(r, v)),
            }
        });
        (self.zones.is_empty() || self.zones.contains(&input.resolved.zone))
            && (self.levels.is_empty() || self.levels.contains(&input.analysis.level))
            && effects
            && resource
            && self.connection.as_deref().is_none_or(|c| c == input.connection)
    }
}

/// An explicit rule.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rule {
    pub id: String,
    pub issuer: Issuer,
    pub when: When,
    pub then: Then,
    /// Unix seconds; the rule no longer applies from then.
    #[serde(default)]
    pub not_after: Option<i64>,
    /// A repo's looser rule applies only once the owner accepted it (02 §6).
    #[serde(default)]
    pub accepted: bool,
    #[serde(default)]
    pub explain: String,
}

impl Rule {
    fn live(&self, now: i64) -> bool {
        self.not_after.is_none_or(|t| now < t)
    }
}

/// The highest level a task may make Auto (01 §12: L1 by default, L2 for Mine repositories; never L3).
pub const MAX_CEILING: Level = Level::L2;

/// An approved task's envelope: its permits hold only inside it (01 §12, 08 R3). Every list that isn't empty bounds
/// the request: all of the request's parts must be inside it (an AND bound, never an OR).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub connection: String,
    /// Clamped to [`MAX_CEILING`].
    pub ceiling: Level,
    #[serde(default)]
    pub zones: Vec<Zone>,
    #[serde(default)]
    pub effects: Vec<EffectKind>,
    /// Repositories (fact keys, `*` suffix globs); with `people`, every resource part must match one of the two.
    #[serde(default)]
    pub resources: Vec<String>,
    /// Branch globs (`fix/*`, case-sensitive) every pushed or deleted branch must match, when not empty.
    #[serde(default)]
    pub branches: Vec<String>,
    /// Absolute roots every written or deleted path must be under, when not empty.
    #[serde(default)]
    pub paths: Vec<String>,
    /// People the task may reach: exact addresses or ids, or `@domain`. Empty: none (no message is covered).
    #[serde(default)]
    pub people: Vec<String>,
    /// Uses left of the task's budget; `Some(0)`: spent.
    #[serde(default)]
    pub uses_left: Option<u32>,
    pub not_after: i64,
}

impl Default for Task {
    fn default() -> Self {
        Self {
            id: String::new(),
            connection: String::new(),
            ceiling: Level::L1,
            zones: Vec::new(),
            effects: Vec::new(),
            resources: Vec::new(),
            branches: Vec::new(),
            paths: Vec::new(),
            people: Vec::new(),
            uses_left: None,
            not_after: 0,
        }
    }
}

/// `value` matches `pattern` exactly (case-sensitive), or `pattern` ends in `*` and `value` starts with the rest.
fn glob_exact(pattern: &str, value: &str) -> bool {
    match pattern.strip_suffix('*') {
        Some(prefix) => value.starts_with(prefix),
        None => pattern == value,
    }
}

/// `path` is `root` or below it, component by component; a `..` or a relative path is never inside anything.
fn under(root: &str, path: &str) -> bool {
    let root = root.trim_end_matches('/');
    let clean = |p: &str| p.starts_with('/') && !p.split('/').any(|c| c == ".." || c == ".");
    clean(path) && clean(root) && !root.is_empty() && (path == root || path.starts_with(&format!("{root}/")))
}

/// A person the task lists: the exact address or id, or a `@domain` entry for an address at that domain.
fn person_in(people: &[String], who: &str) -> bool {
    let who = who.trim().to_ascii_lowercase();
    people.iter().any(|p| {
        let p = p.to_ascii_lowercase();
        if p.starts_with('@') {
            who.contains('@') && who.ends_with(&p)
        } else {
            p == who
        }
    })
}

impl Task {
    /// Whether this task's permit covers `input` (whose effect kinds are `kinds`).
    #[must_use]
    pub fn holds(&self, input: &Input<'_>, kinds: &[EffectKind]) -> bool {
        let a = input.analysis;
        let parts = &input.resolved.parts;
        let in_envelope = |v: &String| self.resources.iter().any(|r| glob(r, v)) || person_in(&self.people, v);
        let resources_ok = (self.resources.is_empty() && self.people.is_empty())
            || (!parts.is_empty() && parts.iter().all(in_envelope));
        // Unknown never widens: an Outside request is covered only by what the envelope names.
        let outside_ok = input.resolved.zone != Zone::Outside || !(self.resources.is_empty() && self.people.is_empty());
        // Branches bound git, folders bound files, people bound messages: once branches or folders are named, a kind
        // of effect no list bounds is outside the task (reads aside).
        let bounded = !self.branches.is_empty() || !self.paths.is_empty();
        let effects_ok = a.effects.iter().all(|e| match e {
            Effect::GitPush {
                refname,
                ..
            }
            | Effect::GitRefDelete {
                refname,
                ..
            } => {
                let branch = refname.strip_prefix("refs/heads/").unwrap_or(refname);
                if self.branches.is_empty() {
                    !bounded
                } else {
                    self.branches.iter().any(|b| glob_exact(b, branch))
                }
            }
            Effect::FileWrite {
                path,
            } => {
                if self.paths.is_empty() {
                    !bounded
                } else {
                    self.paths.iter().any(|r| under(r, &path.path))
                }
            }
            Effect::FileDelete {
                paths,
                ..
            } => {
                if self.paths.is_empty() {
                    !bounded
                } else {
                    paths.iter().all(|p| self.paths.iter().any(|r| under(r, &p.path)))
                }
            }
            Effect::Message {
                to,
                ..
            } => !to.is_empty() && to.iter().all(|t| person_in(&self.people, t)),
            Effect::FileRead {
                ..
            }
            | Effect::DataRead {
                ..
            } => true,
            _ => !bounded,
        });
        input.now < self.not_after
            && input.connection == self.connection
            && self.uses_left != Some(0)
            && a.level <= self.ceiling.min(MAX_CEILING)
            && (self.zones.is_empty() || self.zones.contains(&input.resolved.zone))
            && (self.effects.is_empty() || kinds.iter().all(|k| self.effects.contains(k)))
            && resources_ok
            && outside_ok
            && effects_ok
    }
}

/// Everything a decision is made against: the matrix, the rules, the tasks and the facts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Snapshot {
    pub version: u32,
    pub matrix: Matrix,
    pub rules: Vec<Rule>,
    pub tasks: Vec<Task>,
    pub facts: Facts,
}

impl Default for Snapshot {
    fn default() -> Self {
        Self {
            version: 1,
            matrix: Matrix::default(),
            rules: Vec::new(),
            tasks: Vec::new(),
            facts: Facts::default(),
        }
    }
}

impl Snapshot {
    /// The hex SHA-256 of the snapshot's JSON (maps are ordered, so equal snapshots hash equal).
    #[must_use]
    pub fn hash(&self) -> String {
        let json = serde_json::to_vec(self).unwrap_or_default();
        data_encoding::HEXLOWER.encode(&Sha256::digest(&json))
    }
}

/// One request to evaluate.
#[derive(Clone, Copy, Debug)]
pub struct Input<'a> {
    pub analysis: &'a Analysis,
    pub resolved: &'a Resolved,
    /// The agent connection asking.
    pub connection: &'a str,
    /// The owner approved this resource before (for [`Cell::AskFirst`]).
    pub seen_resource: bool,
    pub now: i64,
    /// Overnight (01 §11): what would ask is parked, people are stricter, the owner's own L1 work is looser.
    pub overnight: bool,
    /// `analysis` is a planned safety net's `after` (`effects::nets::NetPlan`), and the net runs before the effect.
    /// Overnight's looser L1 needs it (01 §11: "L1 in Mine and Friends with nets").
    pub netted: bool,
}

/// What the evaluation says.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// Permitted, subject to the model's sanity check.
    Auto,
    /// The owner reviews; `hard`: no permit could satisfy it.
    Ask {
        hard: bool,
    },
    /// An explicit rule denies it.
    Deny,
    /// Overnight: the owner reviews it in the morning (a Deferred ticket); `hard` as for [`Outcome::Ask`].
    Park {
        hard: bool,
    },
}

impl Outcome {
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Ask {
                ..
            } => "ask",
            Self::Deny => "deny",
            Self::Park {
                ..
            } => "park",
        }
    }

    /// Whether the owner reviews it (now or in the morning).
    #[must_use]
    pub fn reviews(self) -> bool {
        matches!(self, Self::Ask { .. } | Self::Park { .. })
    }

    /// Whether no permit could satisfy it.
    #[must_use]
    pub fn hard(self) -> bool {
        matches!(
            self,
            Self::Ask {
                hard: true
            } | Self::Park {
                hard: true
            }
        )
    }
}

/// Why, one part at a time (02 §4's trace).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Why {
    /// A rule that denies or requires review (its id).
    Rule(String),
    /// The always-ask floor (its words).
    Floor(String),
    /// Anything L3 is never Auto.
    Level3,
    /// Certainty `None`.
    Unreadable,
    /// The matrix cell.
    Cell,
    /// Friends L1: the first time for this resource.
    FirstTime,
    /// A permit that satisfied the cell (its id).
    Permit(String),
    /// Overnight: its looser L1 for the owner's own work, or (hard) its stricter rule for Work and Outside people.
    Overnight,
    /// An approved task's envelope covered it (its id).
    Task(String),
}

/// A decision and its trace.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Evaluation {
    pub outcome: Outcome,
    pub zone: Zone,
    pub level: Level,
    pub certainty: Certainty,
    pub why: Vec<Why>,
    /// The deciding rule's own words, if a rule decided.
    pub explain: String,
    /// [`Snapshot::hash`].
    pub snapshot: String,
}

impl Evaluation {
    /// One line for Activity: "Auto · Mine · Undoable".
    #[must_use]
    pub fn line(&self) -> String {
        match self.outcome {
            Outcome::Deny if !self.explain.is_empty() => format!("Deny · {}", self.explain),
            o => {
                let word = match o {
                    Outcome::Auto => "Auto",
                    Outcome::Ask {
                        ..
                    } => "Ask",
                    Outcome::Deny => "Deny",
                    Outcome::Park {
                        ..
                    } => "Park",
                };
                format!("{word} · {} · {}", self.zone.label(), self.level.label())
            }
        }
    }
}

/// Evaluates `input` against `snap` (R3): deny rules, then hard obligations, then the cell and the permits. In
/// Overnight (01 §11) every review becomes a park.
#[must_use]
pub fn evaluate(input: &Input<'_>, snap: &Snapshot) -> Evaluation {
    let a = input.analysis;
    let zone = input.resolved.zone;
    let mut kinds: Vec<EffectKind> = a.effects.iter().map(Effect::kind).collect();
    kinds.sort();
    kinds.dedup();
    let cover = |r: &Rule| match r.then {
        Then::Permit => Cover::All,
        Then::Deny | Then::Review => Cover::Any,
    };
    let mut live: Vec<&Rule> =
        snap.rules.iter().filter(|r| r.live(input.now) && r.when.matches(input, &kinds, cover(r))).collect();
    live.sort_by(|x, y| x.issuer.rank().cmp(&y.issuer.rank()).then_with(|| x.id.cmp(&y.id)));
    let done = |outcome: Outcome, why: Vec<Why>, explain: String| Evaluation {
        outcome,
        zone,
        level: a.level,
        certainty: a.certainty,
        why,
        explain,
        snapshot: snap.hash(),
    };
    // What the owner reviews: now (Ask), or in the morning in Overnight (Park).
    let review = |hard: bool| {
        if input.overnight {
            Outcome::Park {
                hard,
            }
        } else {
            Outcome::Ask {
                hard,
            }
        }
    };

    // 1. An explicit deny wins.
    let denies: Vec<&&Rule> = live.iter().filter(|r| r.then == Then::Deny).collect();
    if let Some(first) = denies.first() {
        return done(Outcome::Deny, denies.iter().map(|r| Why::Rule(r.id.clone())).collect(), first.explain.clone());
    }

    // 2. Hard obligations add up.
    let mut hard: Vec<Why> = a.floor.iter().map(|f| Why::Floor(f.label().to_owned())).collect();
    if a.certainty == Certainty::None && !hard.contains(&Why::Unreadable) {
        hard.push(Why::Unreadable);
    }
    if a.level == Level::L3 {
        hard.push(Why::Level3);
    }
    // Overnight: what reaches Work or Outside people always waits for the morning.
    if input.overnight && a.level == Level::L2 && matches!(zone, Zone::Work | Zone::Outside) {
        hard.push(Why::Overnight);
    }
    let reviews: Vec<&&Rule> = live.iter().filter(|r| r.then == Then::Review).collect();
    hard.extend(reviews.iter().map(|r| Why::Rule(r.id.clone())));
    if !hard.is_empty() {
        let explain = reviews.first().map(|r| r.explain.clone()).unwrap_or_default();
        return done(review(true), hard, explain);
    }

    // 3. The cell, which any one qualifying permit satisfies.
    let cell = snap.matrix.cell(zone, a.level);
    match cell {
        Cell::Auto => return done(Outcome::Auto, vec![Why::Cell], String::new()),
        Cell::AskFirst if input.seen_resource => return done(Outcome::Auto, vec![Why::FirstTime], String::new()),
        Cell::Ask | Cell::AskFirst => {}
    }
    // Overnight's own permit: the owner's own L1 work, made recoverable by a net Reins runs first (a backup ref, the
    // trash). Without a planned net it follows the cell (a Friends L1 asks the first time, so it parks).
    if input.overnight && input.netted && a.level == Level::L1 && matches!(zone, Zone::Mine | Zone::Friends) {
        return done(Outcome::Auto, vec![Why::Overnight], String::new());
    }
    let permit = live.iter().find(|r| {
        r.then == Then::Permit
            && match &r.issuer {
                Issuer::Owner | Issuer::Org(_) => true,
                Issuer::Repo(_) => r.accepted,
                Issuer::Task(id) => snap.tasks.iter().any(|t| &t.id == id && t.holds(input, &kinds)),
            }
    });
    if let Some(p) = permit {
        return done(Outcome::Auto, vec![Why::Permit(p.id.clone())], String::new());
    }
    // A task is a permit by itself, inside its envelope only.
    if let Some(t) = snap.tasks.iter().find(|t| t.holds(input, &kinds)) {
        return done(Outcome::Auto, vec![Why::Task(t.id.clone())], String::new());
    }
    let why = if cell == Cell::AskFirst {
        Why::FirstTime
    } else {
        Why::Cell
    };
    done(review(false), vec![why], String::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::{Base, FactKind, GITHUB, request_zone};
    use crate::effects::{Context, Effect, Workspace, analyze_command, analyze_tool};
    use proptest::prelude::*;
    use serde_json::json;

    fn snap() -> Snapshot {
        let mut s = Snapshot::default();
        s.facts.set(FactKind::Owner, "dkat", Zone::Mine);
        s.facts.set(FactKind::Owner, "acme", Zone::Work);
        s.facts.set(FactKind::Owner, "pal", Zone::Friends);
        s
    }

    fn ctx() -> Context {
        Context::new("/home/me/Documents/dev/app", Workspace::standard("/home/me"))
    }

    fn eval(a: &Analysis, s: &Snapshot, seen: bool) -> Evaluation {
        eval_with(a, s, seen, false, "c1", 1_000)
    }

    fn night(a: &Analysis, s: &Snapshot) -> Evaluation {
        eval_with(a, s, false, true, "c1", 1_000)
    }

    fn eval_with(a: &Analysis, s: &Snapshot, seen: bool, overnight: bool, connection: &str, now: i64) -> Evaluation {
        let r = request_zone(
            a,
            &s.facts,
            &Base {
                bare_host: Some(GITHUB),
                ..Base::default()
            },
        );
        evaluate(
            &Input {
                analysis: a,
                resolved: &r,
                connection,
                seen_resource: seen,
                now,
                overnight,
                netted: overnight,
            },
            s,
        )
    }

    fn analysis(e: Effect, certainty: Certainty) -> Analysis {
        Analysis::new(vec![e], certainty, Vec::new())
    }

    fn branch_delete(repo: &str) -> Analysis {
        analysis(
            Effect::GitRefDelete {
                repo: repo.to_owned(),
                refname: "refs/heads/old".to_owned(),
                default_branch: Some(false),
            },
            Certainty::Full,
        )
    }

    fn rule(id: &str, issuer: Issuer, when: When, then: Then) -> Rule {
        Rule {
            id: id.to_owned(),
            issuer,
            when,
            then,
            not_after: None,
            accepted: false,
            explain: format!("{id} says so"),
        }
    }

    #[test]
    fn default_matrix_is_01_4() {
        use Cell::{Ask, AskFirst, Auto};
        let m = Matrix::default();
        assert_eq!(m.mine, [Auto, Auto, Auto, Ask]);
        assert_eq!(m.friends, [Auto, AskFirst, Auto, Ask]);
        assert_eq!(m.work, [Auto, Ask, Ask, Ask]);
        assert_eq!(m.outside, [Ask, Ask, Ask, Ask]);
        assert!(m.loosens(Zone::Work, Level::L1, Auto));
        assert!(!m.loosens(Zone::Mine, Level::L1, Ask));
    }

    #[test]
    fn l3_and_floor_are_hard() {
        let s = snap();
        let rewrite = analysis(
            Effect::GitPush {
                repo: "dkat/app".to_owned(),
                refname: "refs/heads/main".to_owned(),
                default_branch: Some(true),
                rewrites: Some(true),
                commits: 1,
            },
            Certainty::Full,
        );
        let e = eval(&rewrite, &s, false);
        assert_eq!(
            e.outcome,
            Outcome::Ask {
                hard: true
            }
        );
        assert!(e.why.contains(&Why::Level3));
        let key = eval(&analyze_command("cat ~/.ssh/id_ed25519", &ctx()), &s, false);
        assert_eq!(
            key.outcome,
            Outcome::Ask {
                hard: true
            },
            "{key:?}"
        );
        let unread = eval(
            &analysis(
                Effect::Unknown {
                    reason: "x".to_owned(),
                },
                Certainty::None,
            ),
            &s,
            false,
        );
        assert_eq!(
            unread.outcome,
            Outcome::Ask {
                hard: true
            }
        );
        assert_eq!(eval(&analyze_command("rm -rf target", &ctx()), &s, false).outcome, Outcome::Auto);
    }

    #[test]
    fn only_rules_deny() {
        let mut s = snap();
        s.rules.push(rule(
            "never-infra",
            Issuer::Owner,
            When {
                effects: vec![EffectKind::Infra],
                ..When::default()
            },
            Then::Deny,
        ));
        let infra = analysis(
            Effect::Infra {
                target: "terraform:prod".to_owned(),
                change: crate::effects::InfraChange::Apply,
                local: false,
            },
            Certainty::Full,
        );
        let e = eval(&infra, &s, false);
        assert_eq!(e.outcome, Outcome::Deny);
        assert_eq!(e.explain, "never-infra says so");
        assert!(e.line().starts_with("Deny · "));
    }

    proptest! {
        #[test]
        fn nothing_denies_without_a_rule(level in 0usize..4, zone in 0usize..4, cert in 0usize..3, seen: bool, overnight: bool) {
            let levels = [Level::L0, Level::L1, Level::L2, Level::L3];
            let certainty = [Certainty::Full, Certainty::Partial, Certainty::None][cert];
            let mut a = analysis(Effect::Unknown { reason: "x".to_owned() }, certainty);
            a.level = levels[level];
            let resolved = Resolved { zone: Zone::ALL[zone], unknown: Vec::new(), resource: None, parts: Vec::new() };
            let s = snap();
            let e = evaluate(&Input { analysis: &a, resolved: &resolved, connection: "c", seen_resource: seen, now: 0, overnight, netted: overnight }, &s);
            prop_assert_ne!(e.outcome, Outcome::Deny);
            if a.level == Level::L3 || certainty == Certainty::None {
                prop_assert!(e.outcome.hard() && e.outcome.reviews());
            }
            // Overnight never asks now: it parks; outside it, nothing parks.
            prop_assert_eq!(overnight, matches!(e.outcome, Outcome::Park { .. }) || (overnight && e.outcome == Outcome::Auto));
        }
    }

    #[test]
    fn permits_never_satisfy_obligations() {
        let mut s = snap();
        let any = When::default();
        let mut repo = rule("repo-permit", Issuer::Repo("dkat/app".to_owned()), any.clone(), Then::Permit);
        repo.accepted = true;
        s.rules.push(rule("owner-permit", Issuer::Owner, any.clone(), Then::Permit));
        s.rules.push(repo);
        s.rules.push(rule("task-permit", Issuer::Task("t".to_owned()), any, Then::Permit));
        s.tasks.push(Task {
            id: "t".to_owned(),
            connection: "c1".to_owned(),
            ceiling: Level::L3,
            zones: Vec::new(),
            effects: Vec::new(),
            not_after: i64::MAX,
            ..Task::default()
        });
        let delete_home = analyze_command("rm -rf ~/Music", &ctx());
        assert_eq!(
            eval(&delete_home, &s, true).outcome,
            Outcome::Ask {
                hard: true
            }
        );
        let key = analyze_command("cat ~/.ssh/id_rsa", &ctx());
        assert_eq!(
            eval(&key, &s, true).outcome,
            Outcome::Ask {
                hard: true
            }
        );
        // An "Ask me next time" exception on Mine branch deletes.
        s.rules.push(rule(
            "exception",
            Issuer::Owner,
            When {
                zones: vec![Zone::Mine],
                effects: vec![EffectKind::GitRefDelete],
                ..When::default()
            },
            Then::Review,
        ));
        let e = eval(&branch_delete("dkat/app"), &s, true);
        assert_eq!(
            e.outcome,
            Outcome::Ask {
                hard: true
            }
        );
        assert!(e.why.contains(&Why::Rule("exception".to_owned())));
    }

    #[test]
    fn permits_satisfy_soft_cells() {
        let mut s = snap();
        let a = branch_delete("acme/api");
        assert_eq!(
            eval(&a, &s, false).outcome,
            Outcome::Ask {
                hard: false
            },
            "Work L1 asks"
        );
        let mut repo = rule(
            "repo",
            Issuer::Repo("acme/api".to_owned()),
            When {
                resource: Some("acme/*".to_owned()),
                ..When::default()
            },
            Then::Permit,
        );
        s.rules.push(repo.clone());
        assert_eq!(
            eval(&a, &s, false).outcome,
            Outcome::Ask {
                hard: false
            },
            "not accepted"
        );
        repo.accepted = true;
        s.rules = vec![repo];
        let e = eval(&a, &s, false);
        assert_eq!(e.outcome, Outcome::Auto);
        assert_eq!(e.why, vec![Why::Permit("repo".to_owned())]);
        // An expired permit does nothing.
        s.rules[0].not_after = Some(10);
        assert_eq!(
            eval(&a, &s, false).outcome,
            Outcome::Ask {
                hard: false
            }
        );
    }

    #[test]
    fn a_permit_covers_every_kind_and_every_party() {
        let mut s = snap();
        s.facts.set(FactKind::Person, "ann@acme.io", Zone::Work);
        s.facts.set(FactKind::Person, "bob@acme.io", Zone::Work);
        s.rules.push(rule(
            "mail-ann",
            Issuer::Owner,
            When {
                effects: vec![EffectKind::Message],
                resource: Some("ann@acme.io".to_owned()),
                ..When::default()
            },
            Then::Permit,
        ));
        let to = |list: &[&str]| analyze_tool("gmail_send", &json!({ "to": list }));
        assert_eq!(eval(&to(&["ann@acme.io"]), &s, false).outcome, Outcome::Auto);
        assert_eq!(
            eval(&to(&["ann@acme.io", "bob@acme.io"]), &s, false).outcome,
            Outcome::Ask {
                hard: false
            },
            "bob isn't covered"
        );
        // A permit for branch deletes doesn't cover a request that also pushes.
        s.rules.push(rule(
            "deletes",
            Issuer::Owner,
            When {
                effects: vec![EffectKind::GitRefDelete],
                ..When::default()
            },
            Then::Permit,
        ));
        let push = analysis(
            Effect::GitPush {
                repo: "acme/api".to_owned(),
                refname: "refs/heads/x".to_owned(),
                default_branch: Some(false),
                rewrites: Some(true),
                commits: 1,
            },
            Certainty::Full,
        );
        let both =
            Analysis::new([branch_delete("acme/api").effects, push.effects].concat(), Certainty::Full, Vec::new());
        assert_eq!(
            eval(&both, &s, false).outcome,
            Outcome::Ask {
                hard: false
            }
        );
        assert_eq!(eval(&branch_delete("acme/api"), &s, false).outcome, Outcome::Auto);
        // A review on one of the people still asks (any party).
        s.rules.push(rule(
            "ask-bob",
            Issuer::Owner,
            When {
                resource: Some("bob@acme.io".to_owned()),
                ..When::default()
            },
            Then::Review,
        ));
        assert_eq!(
            eval(&to(&["ann@acme.io", "bob@acme.io"]), &s, false).outcome,
            Outcome::Ask {
                hard: true
            }
        );
    }

    #[test]
    fn tasks_bound_their_permits() {
        let base = snap();
        let a = branch_delete("acme/api");
        let task = Task {
            id: "t".to_owned(),
            connection: "c1".to_owned(),
            ceiling: Level::L1,
            zones: vec![Zone::Work],
            effects: vec![EffectKind::GitRefDelete],
            resources: vec!["acme/*".to_owned()],
            not_after: 2_000,
            ..Task::default()
        };
        let with = |t: Task| {
            let mut s = base.clone();
            s.tasks.push(t);
            s.rules.push(rule("tp", Issuer::Task("t".to_owned()), When::default(), Then::Permit));
            s
        };
        assert_eq!(eval(&a, &with(task.clone()), false).outcome, Outcome::Auto);
        for t in [
            Task {
                connection: "c2".to_owned(),
                ..task.clone()
            },
            Task {
                ceiling: Level::L0,
                ..task.clone()
            },
            Task {
                not_after: 500,
                ..task.clone()
            },
            Task {
                zones: vec![Zone::Mine],
                ..task.clone()
            },
            Task {
                resources: vec!["other/*".to_owned()],
                ..task
            },
        ] {
            assert_eq!(
                eval(&a, &with(t.clone()), false).outcome,
                Outcome::Ask {
                    hard: false
                },
                "{t:?}"
            );
        }
        // A permit naming a task that isn't there does nothing.
        let mut s = base.clone();
        s.rules.push(rule("tp", Issuer::Task("gone".to_owned()), When::default(), Then::Permit));
        assert_eq!(
            eval(&a, &s, false).outcome,
            Outcome::Ask {
                hard: false
            }
        );
    }

    #[test]
    fn friends_ask_first() {
        let s = snap();
        let a = branch_delete("pal/lib");
        assert_eq!(
            eval(&a, &s, false).outcome,
            Outcome::Ask {
                hard: false
            }
        );
        assert_eq!(eval(&a, &s, false).why, vec![Why::FirstTime]);
        assert_eq!(eval(&a, &s, true).outcome, Outcome::Auto);
    }

    #[test]
    fn outside_and_people() {
        let s = snap();
        let mail = analyze_tool("gmail_send", &json!({"to": ["x@stranger.io"]}));
        let e = eval(&mail, &s, false);
        assert_eq!(
            e.outcome,
            Outcome::Ask {
                hard: false
            }
        );
        assert_eq!(e.line(), "Ask · Outside · Reaches people");
        assert_eq!(eval(&analyze_command("ls", &ctx()), &s, false).line(), "Auto · Mine · Undoable");
    }

    #[test]
    fn snapshot_hash_is_stable_and_changes() {
        let a = snap();
        assert_eq!(a.hash(), snap().hash());
        assert_eq!(a.hash().len(), 64);
        let mut b = snap();
        b.matrix.set(Zone::Work, Level::L1, Cell::Auto);
        assert_ne!(a.hash(), b.hash());
        let json = serde_json::to_string(&a).unwrap();
        assert_eq!(serde_json::from_str::<Snapshot>(&json).unwrap(), a);
    }

    fn push(repo: &str, branch: &str) -> Analysis {
        analysis(
            Effect::GitPush {
                repo: repo.to_owned(),
                refname: format!("refs/heads/{branch}"),
                default_branch: Some(false),
                rewrites: Some(true),
                commits: 1,
            },
            Certainty::Full,
        )
    }

    fn park(hard: bool) -> Outcome {
        Outcome::Park {
            hard,
        }
    }

    #[test]
    fn overnight_parks_instead_of_asking() {
        let s = snap();
        let outside = branch_delete("octo/x");
        assert_eq!(
            eval(&outside, &s, false).outcome,
            Outcome::Ask {
                hard: false
            }
        );
        let e = night(&outside, &s);
        assert_eq!(e.outcome, park(false));
        assert_eq!(e.line(), "Park · Outside · Recoverable");
        assert_eq!(e.outcome.word(), "park");
        let delete_home = analyze_command("rm -rf ~/Music", &ctx());
        assert_eq!(night(&delete_home, &s).outcome, park(true));
        let key = analyze_command("cat ~/.ssh/id_rsa", &ctx());
        assert_eq!(night(&key, &s).outcome, park(true), "the floor parks, still hard");
        assert_eq!(night(&analyze_command("ls", &ctx()), &s).outcome, Outcome::Auto);
    }

    #[test]
    fn overnight_people_are_stricter() {
        let mut s = snap();
        s.facts.set(FactKind::Person, "ann@acme.io", Zone::Work);
        s.facts.set(FactKind::Person, "pal@pal.dev", Zone::Friends);
        s.matrix.set(Zone::Work, Level::L2, Cell::Auto);
        s.rules.push(rule("mail", Issuer::Owner, When::default(), Then::Permit));
        let to = |who: &str| analyze_tool("gmail_send", &json!({ "to": [who] }));
        assert_eq!(eval(&to("ann@acme.io"), &s, false).outcome, Outcome::Auto, "by day the owner's cell holds");
        let e = night(&to("ann@acme.io"), &s);
        assert_eq!(e.outcome, park(true));
        assert!(e.why.contains(&Why::Overnight));
        assert_eq!(night(&to("x@stranger.io"), &s).outcome, park(true));
        assert_eq!(night(&to("pal@pal.dev"), &s).outcome, Outcome::Auto, "Friends L2 follows the cell");
    }

    #[test]
    fn overnight_own_l1_is_auto() {
        let s = snap();
        let e = night(&branch_delete("pal/lib"), &s);
        assert_eq!(e.outcome, Outcome::Auto);
        assert_eq!(e.why, vec![Why::Overnight]);
        assert_eq!(night(&branch_delete("acme/api"), &s).outcome, park(false), "Work L1 still waits");
        assert_eq!(
            eval(&branch_delete("pal/lib"), &s, false).outcome,
            Outcome::Ask {
                hard: false
            }
        );
        // Without a planned net (the phone's own requests): it follows the cell, so a Friends L1 parks.
        let a = branch_delete("pal/lib");
        let r = request_zone(
            &a,
            &s.facts,
            &Base {
                bare_host: Some(GITHUB),
                ..Base::default()
            },
        );
        let bare = evaluate(
            &Input {
                analysis: &a,
                resolved: &r,
                connection: "c1",
                seen_resource: false,
                now: 1_000,
                overnight: true,
                netted: false,
            },
            &s,
        );
        assert_eq!(bare.outcome, park(false), "the looser L1 needs a net");
    }

    #[test]
    fn overnight_and_tasks_never_satisfy_hard() {
        let mut s = snap();
        s.facts.set(FactKind::Person, "ann@acme.io", Zone::Work);
        s.rules.push(rule("owner-permit", Issuer::Owner, When::default(), Then::Permit));
        s.tasks.push(Task {
            id: "t".to_owned(),
            connection: "c1".to_owned(),
            ceiling: Level::L3,
            people: vec!["@acme.io".to_owned()],
            not_after: i64::MAX,
            ..Task::default()
        });
        for (what, a) in [
            ("secret", analyze_command("cat ~/.ssh/id_rsa", &ctx())),
            ("L3", analyze_command("rm -rf ~/Music", &ctx())),
            ("work person", analyze_tool("gmail_send", &json!({ "to": ["ann@acme.io"] }))),
        ] {
            assert!(night(&a, &s).outcome.hard(), "{what}");
        }
        s.rules.push(rule(
            "exception",
            Issuer::Owner,
            When {
                effects: vec![EffectKind::GitRefDelete],
                ..When::default()
            },
            Then::Review,
        ));
        assert_eq!(night(&branch_delete("pal/lib"), &s).outcome, park(true), "an owner review beats Overnight's L1");
    }

    #[test]
    fn task_envelope_is_an_and_bound() {
        let base = snap();
        let task = Task {
            id: "fix-ci".to_owned(),
            connection: "c1".to_owned(),
            ceiling: Level::L1,
            resources: vec!["acme/*".to_owned()],
            branches: vec!["fix/*".to_owned()],
            not_after: 2_000,
            ..Task::default()
        };
        let with = |t: &Task| {
            let mut s = base.clone();
            s.tasks.push(t.clone());
            s
        };
        let fix = push("acme/api", "fix/ci");
        assert_eq!(fix.level, Level::L1);
        let e = eval(&fix, &with(&task), false);
        assert_eq!(e.outcome, Outcome::Auto);
        assert_eq!(e.why, vec![Why::Task("fix-ci".to_owned())]);
        let ask = Outcome::Ask {
            hard: false,
        };
        assert_eq!(eval_with(&fix, &with(&task), false, false, "c2", 1_000).outcome, ask, "another connection");
        assert_eq!(eval_with(&fix, &with(&task), false, false, "c1", 2_000).outcome, ask, "after its end");
        assert_eq!(eval(&push("acme/api", "main"), &with(&task), false).outcome, ask, "a branch not listed");
        assert_eq!(eval(&push("acme/api", "Fix/ci"), &with(&task), false).outcome, ask, "branches are exact");
        assert_eq!(eval(&push("octo/x", "fix/ci"), &with(&task), false).outcome, ask, "a repo not listed");
        let spent = Task {
            uses_left: Some(0),
            ..task.clone()
        };
        assert_eq!(eval(&fix, &with(&spent), false).outcome, ask, "no uses left");
        let one = Task {
            uses_left: Some(1),
            ..task.clone()
        };
        assert_eq!(eval(&fix, &with(&one), false).outcome, Outcome::Auto);
        // People: none by default; a listed person (or domain) is covered, an unlisted one isn't.
        let mut s = base.clone();
        s.facts.set(FactKind::Person, "ann@acme.io", Zone::Work);
        let wide = Task {
            ceiling: Level::L2,
            resources: Vec::new(),
            branches: Vec::new(),
            ..task.clone()
        };
        let to = |list: &[&str]| analyze_tool("gmail_send", &json!({ "to": list }));
        s.tasks = vec![wide.clone()];
        assert_eq!(eval(&to(&["ann@acme.io"]), &s, false).outcome, ask, "no people listed");
        s.tasks = vec![Task {
            people: vec!["ann@acme.io".to_owned()],
            ..wide
        }];
        assert_eq!(eval(&to(&["ann@acme.io"]), &s, false).outcome, Outcome::Auto);
        assert_eq!(eval(&to(&["ann@acme.io", "x@stranger.io"]), &s, false).outcome, ask, "one unlisted person");
        // The ceiling: an L3 ceiling is read as L2, so nothing L3 is ever covered (it's hard anyway).
        assert_eq!(MAX_CEILING, Level::L2);
        // An Outside request is covered only by what the envelope names.
        let anything = Task {
            resources: Vec::new(),
            branches: Vec::new(),
            ..task
        };
        assert_eq!(eval(&branch_delete("octo/x"), &with(&anything), false).outcome, ask, "unknown never widens");
        assert_eq!(eval(&branch_delete("acme/x"), &with(&anything), false).outcome, Outcome::Auto);
    }

    #[test]
    fn task_paths_are_component_wise() {
        assert!(under("/home/u/src/a", "/home/u/src/a"));
        assert!(under("/home/u/src/a/", "/home/u/src/a/b/c"));
        assert!(!under("/home/u/src/a", "/home/u/src/ab"));
        assert!(!under("/home/u/src/a", "/home/u/src/a/../b"));
        assert!(!under("/home/u/src/a", "src/a/b"));
        assert!(!under("", "/x"));
        assert!(!under("/", "/etc/passwd"), "the root is not a root a task may name");
        let delete = |p: &str| {
            let mut a = analysis(
                Effect::FileDelete {
                    paths: vec![crate::effects::PathRef {
                        path: p.to_owned(),
                        area: crate::effects::Area::Source,
                        recoverable: true,
                    }],
                    recursive: true,
                },
                Certainty::Full,
            );
            a.level = Level::L1;
            a
        };
        let task = Task {
            id: "t".to_owned(),
            connection: "c1".to_owned(),
            paths: vec!["/home/u/src/a".to_owned()],
            not_after: i64::MAX,
            ..Task::default()
        };
        let resolved = Resolved {
            zone: Zone::Work,
            unknown: Vec::new(),
            resource: None,
            parts: Vec::new(),
        };
        let holds = |a: &Analysis| {
            task.holds(
                &Input {
                    analysis: a,
                    resolved: &resolved,
                    connection: "c1",
                    seen_resource: false,
                    now: 0,
                    overnight: false,
                    netted: false,
                },
                &[EffectKind::FileDelete],
            )
        };
        assert!(holds(&delete("/home/u/src/a/build")));
        assert!(!holds(&delete("/home/u/src/ab")));
        assert!(!holds(&delete("/home/u/src/a/../../x")));
        // Folders named: a push or a network write isn't a folder's, so it's outside the task.
        let push = Analysis::new(
            vec![Effect::GitPush {
                repo: "acme/app".to_owned(),
                refname: "refs/heads/x".to_owned(),
                default_branch: Some(false),
                rewrites: Some(false),
                commits: 1,
            }],
            Certainty::Full,
            Vec::new(),
        );
        assert!(!holds(&push));
    }

    #[test]
    fn branches_bound_the_kinds_of_effect() {
        let mut s = snap();
        s.tasks.push(Task {
            id: "t".to_owned(),
            connection: "c1".to_owned(),
            resources: vec!["acme/*".to_owned()],
            branches: vec!["fix/*".to_owned()],
            not_after: i64::MAX,
            ..Task::default()
        });
        assert_eq!(eval(&push("acme/api", "fix/a"), &s, false).outcome, Outcome::Auto);
        // A change in the same repository that isn't a branch's (an issue closed): outside a task that names branches.
        let task = &s.tasks[0];
        let mut close = analysis(
            Effect::DataWrite {
                service: "github".to_owned(),
                what: "issue".to_owned(),
                lost: false,
            },
            Certainty::Full,
        );
        close.level = Level::L1;
        let resolved = Resolved {
            zone: Zone::Work,
            unknown: Vec::new(),
            resource: Some("acme/api".to_owned()),
            parts: vec!["acme/api".to_owned()],
        };
        let input = Input {
            analysis: &close,
            resolved: &resolved,
            connection: "c1",
            seen_resource: false,
            now: 0,
            overnight: false,
            netted: false,
        };
        assert!(!task.holds(&input, &[EffectKind::DataWrite]));
        let wide = Task {
            branches: Vec::new(),
            ..task.clone()
        };
        assert!(wide.holds(&input, &[EffectKind::DataWrite]), "without branches, the repository bounds it");
    }
}
