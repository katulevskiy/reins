//! Engines a corpus is replayed through: `guard` (how Reins decides today), `effects` (the consequence engine, the
//! context graph and the matrix of `reins_policy`: the 01 §4 matrix, the 08 R3 rules and the 08 R4 floor) and
//! `desktop` (08 R1: hook requests decided on the computer with the phone-signed policy, as the computer applies it;
//! everything else as `effects`).
//!
//! Zones come from the corpus context only (the owner's GitHub owners, the workspace, the owner's devices) as
//! `reins_policy::context::Facts`, the way the setup screens confirm them. Nothing here reads a case's label: decisions never see the answer.

use reins_policy::context::{self as graph, Facts, Zone};
use reins_policy::desktop_policy;
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
    /// Safety nets (P3): a command from a harness whose hook can rewrite it is decided on its netted levels.
    #[serde(default)]
    pub nets: bool,
    /// The desktop path (08 R1) before the owner confirmed any of the computer's folders: the phone signs the corpus's
    /// roots and no scratch folder, so only scratch folders inside a root count there. Off (the default): the computer
    /// proposed its roots and scratch folders ([`proposed_folders`]) and the owner confirmed each on the phone.
    #[serde(default)]
    pub unconfirmed_folders: bool,
}

impl Default for Policy {
    /// 01 §4, Personal mode.
    fn default() -> Self {
        Self {
            name: "Personal default (01 §4)".to_owned(),
            snapshot: Snapshot::default(),
            ask_on_partial: false,
            nets: false,
            unconfirmed_folders: false,
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
    /// Hook requests (commands, files) decided on the computer with the policy the phone signs for it
    /// (`reins_policy::desktop_policy`): the facts it gets, its workspace, the nets, and the groups about Reins itself
    /// always asked. The rest as [`EngineKind::Effects`].
    Desktop,
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

/// Harnesses whose hook can hand back a rewritten command (`reins_desktop::nets::hook::rewrites`).
/// The adversarial set ("synthetic") is replayed without nets, as from a harness that can't rewrite (Cursor):
/// `corpus/nets.jsonl` holds the cases that try the nets themselves.
const REWRITING: &[&str] = &["Claude Code", "Codex", "Gemini CLI"];

/// The plan id and program a replayed rewrite uses.
const REPLAY_PLAN: &str = "replay01";
const REPLAY_REINS: &str = "/usr/bin/reins";

/// A command's analysis with its safety nets (P3), when one applies: one disk, the trash in the state folder.
#[must_use]
pub fn netted(ctx: &Context, case: &Case) -> Option<Analysis> {
    if case.kind != Kind::Command || !case.agent.as_deref().is_some_and(|a| REWRITING.contains(&a)) {
        return None;
    }
    let ec = engine_context(ctx, case);
    // The recording computer: home is its own mount (btrfs subvolume) with the trash in it; `/`, `/tmp` and the rest
    // are other disks. The known worktrees are linked worktrees.
    let home = ctx.home.trim_end_matches('/');
    let facts = effects::nets::SameDisk {
        trash: format!("{home}/.local/state/reins/nets"),
        disk: home.to_owned(),
        worktrees: ctx.worktrees.iter().map(|w| effects::normalize(w, "/", home)).collect(),
    };
    let runner = effects::nets::Runner {
        reins: REPLAY_REINS,
        plan: REPLAY_PLAN,
        store: &facts.trash,
    };
    effects::nets::plan_command(case.command.as_deref()?, &ec, &facts, runner).map(|p| p.after)
}

/// The repository a case acts on, as the request names it exactly: the git remote, or the connector's `owner`/`repo`
/// arguments. An `owner` alone (listing a user's repositories) stands for that owner (`owner/.owner`, a name no fact
/// is ever written for). Arguments that name a repository inexactly give an unparsable base: Outside.
fn base_repo(ctx: &Context, case: &Case) -> Option<String> {
    match case.kind {
        // A command or a file hook in a known worktree: `origin` is that worktree's repository, unless the command
        // may act on another one (`git -C`, a `cd` out of it, `GIT_DIR`…).
        Kind::Command | Kind::File => {
            let cwd = case.cwd.as_deref()?;
            let ws = engine_context(ctx, case).workspace;
            let top = worktree_top(&ws, &effects::normalize(cwd, cwd, &ws.home));
            let at = case
                .command
                .as_deref()
                .map_or(graph::RepoAt::Cwd, |c| graph::repo_at(c, cwd, top.as_deref(), &ws.home));
            // The corpus records one `origin` for the owner's known worktrees (the desktop reads each one's own).
            let known = match at {
                graph::RepoAt::Cwd => top.is_some(),
                graph::RepoAt::Dir(d) => worktree_top(&ws, &d).is_some(),
                graph::RepoAt::Unknown => false,
            };
            known.then(|| ctx.origin.clone()).flatten()
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

/// The known worktree `abs` is in (`dir/*` patterns stand for each folder in `dir`), the deepest one.
fn worktree_top(ws: &Workspace, abs: &str) -> Option<String> {
    ws.worktrees
        .iter()
        .filter_map(|w| {
            let w = effects::normalize(w, "/", &ws.home);
            match w.strip_suffix("/*") {
                Some(dir) => {
                    let rest = abs.strip_prefix(&format!("{dir}/"))?;
                    let first = rest.split('/').next().filter(|f| !f.is_empty())?;
                    Some(format!("{dir}/{first}"))
                }
                None => (abs == w || abs.starts_with(&format!("{w}/"))).then_some(w),
            }
        })
        .max_by_key(String::len)
}

/// The host a case's bare `owner/name` means: GitHub's for the phone's GitHub tools, none for a command (where a
/// bare name is a remote's).
fn bare_host(case: &Case) -> Option<&'static str> {
    match case.kind {
        Kind::Mcp if case.tool.as_deref().is_some_and(|t| t.starts_with("github_")) => Some(graph::GITHUB),
        _ => None,
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
    evaluate_with(ctx, case, policy, a, false)
}

/// What the matrix says of a case, in Overnight (`overnight`) or not.
#[must_use]
pub fn evaluate_with(ctx: &Context, case: &Case, policy: &Policy, a: &Analysis, overnight: bool) -> Evaluation {
    let mut snap = policy.snapshot.clone();
    snap.facts = facts(ctx, policy);
    evaluate_snapshot(ctx, case, &snap, case.agent.as_deref().unwrap_or_default(), a, overnight)
}

/// What the matrix says of a case against `snap` (its facts included), for `connection`.
#[must_use]
pub fn evaluate_snapshot(
    ctx: &Context,
    case: &Case,
    snap: &Snapshot,
    connection: &str,
    a: &Analysis,
    overnight: bool,
) -> Evaluation {
    let base = base_repo(ctx, case);
    let resolved = graph::request_zone(
        a,
        &snap.facts,
        &graph::Base {
            repo: base.as_deref(),
            remotes: None,
            bare_host: bare_host(case),
        },
    );
    matrix::evaluate(
        &Input {
            analysis: a,
            resolved: &resolved,
            connection,
            seen_resource: false,
            now: case.at,
            overnight,
            netted: false,
        },
        snap,
    )
}

/// The policy the phone signs for the recording computer (08 R1), as that computer applies it at `case.at`: the facts
/// it gets (owners, repositories, hosts), the owner's rules and the connection's tasks, the corpus's workspace as both
/// the signed and the local one.
#[must_use]
pub fn desktop_effective(ctx: &Context, case: &Case, policy: &Policy) -> desktop_policy::Effective {
    let connection = case.agent.clone().unwrap_or_default();
    let (facts, github_ids) = desktop_policy::desktop_facts(&facts(ctx, policy));
    let snapshot = Snapshot {
        facts,
        rules: desktop_policy::desktop_rules(&policy.snapshot.rules, &connection),
        tasks: desktop_policy::desktop_tasks(&policy.snapshot.tasks, &connection),
        ..policy.snapshot.clone()
    };
    let (roots, scratch) = if policy.unconfirmed_folders {
        (ctx.roots.clone(), Vec::new())
    } else {
        signed_folders(ctx)
    };
    let signed = desktop_policy::DesktopPolicy {
        binding: desktop_policy::Binding {
            version: 1,
            issued_at: case.at,
            not_after: case.at + desktop_policy::VALIDITY_SECS,
            client_key: String::new(),
            connection_id: connection,
            profile: desktop_policy::PERSONAL.to_owned(),
        },
        decides: true,
        mode: desktop_policy::ModeState {
            mode: reins_proto::autopilot::Mode::Auto,
            until: None,
        },
        snapshot,
        github_ids,
        roots,
        scratch,
    };
    let (none, matrix) = (Facts::default(), matrix::Matrix::default());
    let canonical = std::collections::BTreeMap::new();
    let local = desktop_policy::Local {
        facts: &none,
        matrix: &matrix,
        roots: &ctx.roots,
        scratch: &ctx.scratch,
        home: &ctx.home,
        canonical: &canonical,
    };
    desktop_policy::effective(Some(&signed), &[], &local, &std::collections::BTreeMap::new(), case.at)
}

/// What the recording computer proposes to the phone (`reins_desktop::folders`): its roots, then its scratch folders,
/// as absolute paths (the corpus can't resolve links, so `~` is only expanded), refused ones left out.
#[must_use]
pub fn proposed_folders(ctx: &Context) -> Vec<reins_proto::desktop::Folder> {
    use reins_proto::desktop::{Folder, FolderKind};
    let all =
        ctx.roots.iter().map(|r| (r, FolderKind::Root)).chain(ctx.scratch.iter().map(|s| (s, FolderKind::Scratch)));
    all.map(|(entry, kind)| Folder {
        path: effects::normalize(entry, "/", &ctx.home),
        kind,
    })
    .filter(|f| reins_policy::desktop_folders::refusal(&f.path, &ctx.home).is_none())
    .collect()
}

/// The roots and scratch folders the phone signs for the recording computer once the owner confirmed every folder it
/// proposed and the phone showed (`reins_core::autopilot::desktop_folders`): the standard roots and the confirmed ones.
#[must_use]
pub fn signed_folders(ctx: &Context) -> (Vec<String>, Vec<String>) {
    use reins_proto::desktop::FolderKind;
    let mut roots = Workspace::standard("").roots;
    let cards = reins_policy::desktop_folders::open(&proposed_folders(ctx), &roots, &[], &ctx.home);
    let mut scratch = Vec::new();
    for f in cards {
        match f.kind {
            FolderKind::Root => roots.push(f.path),
            FolderKind::Scratch => scratch.push(f.path),
        }
    }
    (roots, scratch)
}

/// A hook case decided on the computer ([`EngineKind::Desktop`]).
fn decide_desktop(ctx: &Context, case: &Case, policy: &Policy) -> Decision {
    let e = desktop_effective(ctx, case, policy);
    let mut dctx = ctx.clone();
    dctx.roots.clone_from(&e.roots);
    dctx.scratch.clone_from(&e.scratch);
    let a = if policy.nets {
        netted(&dctx, case)
    } else {
        None
    }
    .unwrap_or_else(|| analyze(&dctx, case));
    let ev = evaluate_snapshot(&dctx, case, &e.snapshot, &e.connection, &a, false);
    let texts: Vec<&str> = match case.kind {
        Kind::Command => case.command.as_deref().into_iter().collect(),
        _ => case.files.iter().flatten().map(String::as_str).collect(),
    };
    let reins = reins_policy::guard::always_asked(&texts);
    let (ask, why) = if let Some(g) = reins.first() {
        (true, format!("always asked here: {}", g.label))
    } else if policy.ask_on_partial && a.certainty == Certainty::Partial && ev.outcome == Outcome::Auto {
        (true, "partly unread".to_owned())
    } else {
        (ev.outcome != Outcome::Auto, ev.line())
    };
    Decision {
        ask,
        why,
        zone: Some(ev.zone),
        analysis: Some(a),
        evaluation: Some(ev),
    }
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
        EngineKind::Desktop if matches!(case.kind, Kind::Command | Kind::File) => decide_desktop(ctx, case, policy),
        EngineKind::Effects | EngineKind::Desktop => {
            let a = if policy.nets {
                netted(ctx, case)
            } else {
                None
            }
            .unwrap_or_else(|| analyze(ctx, case));
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
