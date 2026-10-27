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
use crate::effects::{Analysis, Certainty, EffectKind, Level};

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

impl When {
    fn matches(&self, input: &Input<'_>, kinds: &[EffectKind]) -> bool {
        (self.zones.is_empty() || self.zones.contains(&input.resolved.zone))
            && (self.levels.is_empty() || self.levels.contains(&input.analysis.level))
            && (self.effects.is_empty() || kinds.iter().any(|k| self.effects.contains(k)))
            && self.resource.as_deref().is_none_or(|r| input.resolved.resource.as_deref().is_some_and(|v| glob(r, v)))
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

/// An approved task's envelope: its permits hold only inside it (01 §12, 08 R3).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub connection: String,
    pub ceiling: Level,
    #[serde(default)]
    pub zones: Vec<Zone>,
    #[serde(default)]
    pub effects: Vec<EffectKind>,
    /// Prefix globs; the request's resource must match one when not empty.
    #[serde(default)]
    pub resources: Vec<String>,
    pub not_after: i64,
}

impl Task {
    fn holds(&self, input: &Input<'_>, kinds: &[EffectKind]) -> bool {
        input.now < self.not_after
            && input.connection == self.connection
            && input.analysis.level <= self.ceiling
            && (self.zones.is_empty() || self.zones.contains(&input.resolved.zone))
            && (self.effects.is_empty() || kinds.iter().all(|k| self.effects.contains(k)))
            && (self.resources.is_empty()
                || input.resolved.resource.as_deref().is_some_and(|v| self.resources.iter().any(|r| glob(r, v))))
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
        }
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
                };
                format!("{word} · {} · {}", self.zone.label(), self.level.label())
            }
        }
    }
}

/// Evaluates `input` against `snap` (R3): deny rules, then hard obligations, then the cell and the permits.
#[must_use]
pub fn evaluate(input: &Input<'_>, snap: &Snapshot) -> Evaluation {
    let a = input.analysis;
    let mut kinds: Vec<EffectKind> = a.effects.iter().map(crate::effects::Effect::kind).collect();
    kinds.sort();
    kinds.dedup();
    let mut live: Vec<&Rule> =
        snap.rules.iter().filter(|r| r.live(input.now) && r.when.matches(input, &kinds)).collect();
    live.sort_by(|x, y| x.issuer.rank().cmp(&y.issuer.rank()).then_with(|| x.id.cmp(&y.id)));
    let done = |outcome: Outcome, why: Vec<Why>, explain: String| Evaluation {
        outcome,
        zone: input.resolved.zone,
        level: a.level,
        certainty: a.certainty,
        why,
        explain,
        snapshot: snap.hash(),
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
    let reviews: Vec<&&Rule> = live.iter().filter(|r| r.then == Then::Review).collect();
    hard.extend(reviews.iter().map(|r| Why::Rule(r.id.clone())));
    if !hard.is_empty() {
        let explain = reviews.first().map(|r| r.explain.clone()).unwrap_or_default();
        return done(
            Outcome::Ask {
                hard: true,
            },
            hard,
            explain,
        );
    }

    // 3. The cell, which any one qualifying permit satisfies.
    match snap.matrix.cell(input.resolved.zone, a.level) {
        Cell::Auto => return done(Outcome::Auto, vec![Why::Cell], String::new()),
        Cell::AskFirst if input.seen_resource => return done(Outcome::Auto, vec![Why::FirstTime], String::new()),
        Cell::Ask | Cell::AskFirst => {}
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
    let why = if snap.matrix.cell(input.resolved.zone, a.level) == Cell::AskFirst {
        Why::FirstTime
    } else {
        Why::Cell
    };
    done(
        Outcome::Ask {
            hard: false,
        },
        vec![why],
        String::new(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::{FactKind, request_zone};
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
        let r = request_zone(a, &s.facts, None);
        evaluate(
            &Input {
                analysis: a,
                resolved: &r,
                connection: "c1",
                seen_resource: seen,
                now: 1_000,
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
        fn nothing_denies_without_a_rule(level in 0usize..4, zone in 0usize..4, cert in 0usize..3, seen: bool) {
            let levels = [Level::L0, Level::L1, Level::L2, Level::L3];
            let certainty = [Certainty::Full, Certainty::Partial, Certainty::None][cert];
            let mut a = analysis(Effect::Unknown { reason: "x".to_owned() }, certainty);
            a.level = levels[level];
            let resolved = Resolved { zone: Zone::ALL[zone], unknown: Vec::new(), resource: None };
            let s = snap();
            let e = evaluate(&Input { analysis: &a, resolved: &resolved, connection: "c", seen_resource: seen, now: 0 }, &s);
            prop_assert_ne!(e.outcome, Outcome::Deny);
            if a.level == Level::L3 || certainty == Certainty::None {
                prop_assert_eq!(e.outcome, Outcome::Ask { hard: true });
            }
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
            resources: Vec::new(),
            not_after: i64::MAX,
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
}
