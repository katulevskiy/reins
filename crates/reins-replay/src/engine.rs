//! Engines a corpus is replayed through: `guard` (how Reins decides today) and `effects` (the consequence engine, the
//! context graph and the matrix of `reins_policy`: the 01 §4 matrix, the 08 R3 rules and the 08 R4 floor).
//!
//! Zones come from the corpus context only (the owner's GitHub owners, the workspace, the owner's devices) as
//! `reins_policy::context::Facts`, the way the setup screens confirm them. Nothing here reads a case's label: decisions never see the answer.

use reins_policy::context::{self as graph, Facts, Zone};
use reins_policy::effects::{self, Analysis, Certainty, Context as EngineContext, Workspace};
use reins_policy::matrix::{self, Evaluation, Input, Outcome, Snapshot};
use serde::{Deserialize, Serialize};

use crate::corpus::{Case, Context, Kind};

/// The policy a corpus is replayed with: a matrix snapshot (its facts come from the corpus context) and a knob.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Policy {
    pub name: String,
    /// The matrix, rules and tasks (`reins_policy::matrix::Snapshot`); its facts are merged with the corpus's.
    #[serde(flatten)]
    pub snapshot: Snapshot,
    /// Ask whenever certainty is `Partial`. Off, as 01 §2.4 has it: a Partial request's level is already its worst
    /// plausible effect in the workspace (at least L1) and the zone's cell decides; the model's risk rating is what may
    /// force an ask. On, the recorded corpus asks 11.2% and 11 known false positives return.
    #[serde(default)]
    pub ask_on_partial: bool,
}

impl Default for Policy {
    /// 01 §4, Personal mode.
    fn default() -> Self {
        Self {
            name: "Personal default (01 §4)".to_owned(),
            snapshot: Snapshot::default(),
            ask_on_partial: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineKind {
    /// Today: the guard's rules decide what reaches the phone, and everything that does is asked.
    Guard,
    /// The consequence engine and a policy.
    Effects,
}

/// One case's decision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decision {
    pub ask: bool,
    /// Why, in words.
    pub why: String,
    pub analysis: Option<Analysis>,
    pub zone: Option<Zone>,
    /// The matrix's evaluation (the effects engine only).
    pub evaluation: Option<Evaluation>,
}

/// The engine's context for a case.
#[must_use]
pub fn engine_context(ctx: &Context, case: &Case) -> EngineContext {
    let workspace = Workspace {
        home: ctx.home.clone(),
        roots: ctx.roots.clone(),
        scratch: ctx.scratch.clone(),
        worktrees: ctx.worktrees.clone(),
        tracked: Vec::new(),
    };
    let mut c = EngineContext::new(case.cwd.as_deref().unwrap_or_default(), workspace);
    c.default_branch.clone_from(&ctx.default_branch);
    c.branch.clone_from(&case.branch);
    for (k, v) in &ctx.env {
        c.env.entry(k.clone()).or_insert_with(|| v.clone());
    }
    c
}

/// What the consequence engine makes of a case.
#[must_use]
pub fn analyze(ctx: &Context, case: &Case) -> Analysis {
    let ec = engine_context(ctx, case);
    match case.kind {
        Kind::Command => effects::analyze_command(case.command.as_deref().unwrap_or_default(), &ec),
        Kind::File => {
            let files: Vec<&str> = case.files.iter().flatten().map(String::as_str).collect();
            effects::analyze_files(&files, case.write.unwrap_or(false), &ec)
        }
        Kind::GitPush => match &case.push {
            Some(p) => effects::analyze_push(case.repo.as_deref().unwrap_or_default(), p),
            None => effects::analyze_question(),
        },
        Kind::GitRead => effects::analyze_git_read(case.repo.as_deref().unwrap_or_default()),
        Kind::Mcp => effects::analyze_tool(
            case.tool.as_deref().unwrap_or_default(),
            case.args.as_ref().unwrap_or(&serde_json::Value::Null),
        ),
        Kind::Ask => effects::analyze_question(),
    }
}

/// The repository a case acts on, as the request names it exactly: the git remote, or the connector's `owner`/`repo`
/// arguments. An `owner` alone (listing a user's repositories) stands for that owner (`owner/.owner`, a name no fact
/// is ever written for). Arguments that name a repository inexactly give an unparsable base: Outside.
fn base_repo(ctx: &Context, case: &Case) -> Option<String> {
    match case.kind {
        // A command or a file hook in a known worktree: `origin` is that worktree's repository.
        Kind::Command | Kind::File => {
            let cwd = case.cwd.as_deref()?;
            let ws = engine_context(ctx, case).workspace;
            let in_worktree = ws.recoverable(&effects::normalize(cwd, cwd, &ws.home));
            in_worktree.then(|| ctx.origin.clone()).flatten()
        }
        Kind::GitPush | Kind::GitRead => Some(case.repo.clone().unwrap_or_default()),
        Kind::Mcp => {
            let a = case.args.as_ref().filter(|a| a.get("repo").is_some() || a.get("owner").is_some())?;
            match (a.get("repo"), a.get("owner").and_then(|o| o.as_str())) {
                (None, Some(o)) => Some(format!("{o}/.owner")),
                _ => Some(effects::tool_repo(a).unwrap_or_default()),
            }
        }
        Kind::Ask => None,
    }
}

/// The facts a corpus context verifies.
#[must_use]
pub fn facts(ctx: &Context, policy: &Policy) -> Facts {
    let mut f = ctx.facts();
    for (k, v) in &policy.snapshot.facts.owners {
        f.owners.insert(k.clone(), *v);
    }
    f
}

/// What the matrix says of a case.
#[must_use]
pub fn evaluate(ctx: &Context, case: &Case, policy: &Policy, a: &Analysis) -> Evaluation {
    let mut snap = policy.snapshot.clone();
    snap.facts = facts(ctx, policy);
    let base = base_repo(ctx, case);
    let resolved = graph::request_zone(a, &snap.facts, base.as_deref());
    matrix::evaluate(
        &Input {
            analysis: a,
            resolved: &resolved,
            connection: case.agent.as_deref().unwrap_or_default(),
            seen_resource: false,
            now: case.at,
        },
        &snap,
    )
}

/// Decides one case.
#[must_use]
pub fn decide(ctx: &Context, case: &Case, policy: &Policy, engine: EngineKind) -> Decision {
    match engine {
        EngineKind::Guard => {
            // Everything recorded reached the phone and was asked (Autopilot decided none of the 781). A synthetic
            // command is asked when the guard's rules match it or can't read it, as the hook asks today.
            let ask = match (case.kind, case.original.outcome.is_empty()) {
                (Kind::Command, true) => {
                    let text = case.command.as_deref().unwrap_or_default();
                    let r = reins_policy::guard::read(&[text], &[]);
                    r.found.is_some() || !r.certain
                }
                (Kind::File, true) => {
                    case.files.iter().flatten().any(|f| reins_policy::guard::classify_file(f).is_some())
                }
                _ => true,
            };
            Decision {
                ask,
                why: if ask {
                    "asked today".to_owned()
                } else {
                    "no guard rule matches".to_owned()
                },
                analysis: None,
                zone: None,
                evaluation: None,
            }
        }
        EngineKind::Effects => {
            let a = analyze(ctx, case);
            let e = evaluate(ctx, case, policy, &a);
            let (ask, why) = if !a.floor.is_empty() {
                (true, format!("always asked: {}", a.floor.iter().map(|f| f.label()).collect::<Vec<_>>().join(", ")))
            } else if policy.ask_on_partial && a.certainty == Certainty::Partial && e.outcome == Outcome::Auto {
                (true, "partly unread".to_owned())
            } else {
                (e.outcome != Outcome::Auto, e.line())
            };
            Decision {
                ask,
                why,
                zone: Some(e.zone),
                analysis: Some(a),
                evaluation: Some(e),
            }
        }
    }
}
