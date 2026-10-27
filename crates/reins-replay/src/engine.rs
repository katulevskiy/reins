//! Engines a corpus is replayed through: `guard` (how Reins decides today) and `effects` (the consequence engine plus
//! a policy file: the 01 §4 matrix, the 08 R4 floor and certainty `None`).
//!
//! Zones come from the corpus context only (the owner's GitHub owners, the workspace, the owner's devices), the way
//! the P2 setup screens will confirm them. Nothing here reads a case's label: decisions never see the answer.

use reins_policy::effects::{
    self, Analysis, Area, Certainty, Channel, CodeSource, Context as EngineContext, Effect, Level, NetMethod, Workspace,
};
use serde::{Deserialize, Serialize};

use crate::corpus::{Case, Context, Kind};

/// Whose a thing is (01 §3), least trusted last.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Zone {
    Mine,
    Friends,
    Work,
    Outside,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Cell {
    Auto,
    Ask,
}

/// The zone × level matrix and how partial certainty is treated.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Policy {
    pub name: String,
    pub mine: [Cell; 4],
    pub friends: [Cell; 4],
    pub work: [Cell; 4],
    pub outside: [Cell; 4],
    /// Ask whenever certainty is `Partial`. Off, as 01 §2.4 has it: a Partial request's level is already its worst
    /// plausible effect in the workspace (at least L1) and the zone's cell decides; the model's risk rating (P2) is what
    /// may force an ask. On, the recorded corpus asks 11.2% and 11 known false positives return.
    #[serde(default)]
    pub ask_on_partial: bool,
    /// Safety nets (P3): a command from a harness whose hook can rewrite it is decided on its netted levels.
    #[serde(default)]
    pub nets: bool,
}

impl Default for Policy {
    /// 01 §4, Personal mode.
    fn default() -> Self {
        use Cell::{Ask, Auto};
        Self {
            name: "Personal default (01 §4)".to_owned(),
            mine: [Auto, Auto, Auto, Ask],
            friends: [Auto, Ask, Auto, Ask],
            work: [Auto, Ask, Ask, Ask],
            outside: [Ask, Ask, Ask, Ask],
            ask_on_partial: false,
            nets: false,
        }
    }
}

impl Policy {
    fn cell(&self, zone: Zone, level: Level) -> Cell {
        let row = match zone {
            Zone::Mine => &self.mine,
            Zone::Friends => &self.friends,
            Zone::Work => &self.work,
            Zone::Outside => &self.outside,
        };
        row[level as usize]
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

/// The owner of `github.com/<owner>/<name>` or `<owner>/<name>`.
fn owner(repo: &str) -> Option<&str> {
    // `https://host/owner/name.git`, `git@host:owner/name`, `github.com/owner/name`, `owner/name`.
    let r = match repo.split_once("://") {
        Some((_, rest)) => rest.split_once('/').map_or("", |(_, path)| path),
        None => match repo.split_once(':') {
            Some((host, path)) if host.contains('@') => path,
            _ => repo.strip_prefix("github.com/").unwrap_or(repo),
        },
    };
    let (o, n) = r.split_once('/')?;
    (!o.is_empty() && !n.is_empty()).then_some(o)
}

fn repo_zone(ctx: &Context, repo: &str, fallback: Zone) -> Zone {
    match owner(repo) {
        Some(o) if ctx.mine_owners.iter().any(|m| m.eq_ignore_ascii_case(o)) => Zone::Mine,
        Some(_) => Zone::Outside,
        // `origin`, "this repository": the repository the agent works in.
        None => fallback,
    }
}

fn host_zone(ctx: &Context, host: &str) -> Zone {
    if effects::local_host(host) || ctx.mine_hosts.iter().any(|h| h.eq_ignore_ascii_case(host)) {
        Zone::Mine
    } else {
        Zone::Outside
    }
}

fn area_zone(area: &Area, ctx: &Context) -> Zone {
    match area {
        Area::Generated | Area::Scratch | Area::Source | Area::Unknown => Zone::Mine,
        Area::Home | Area::System => Zone::Outside,
        Area::Remote(h) => host_zone(ctx, h),
    }
}

/// The request's zone: the least trusted of what its effects change or reach (01 §3). What only reads (a file that
/// is not a secret, public data from the network) adds no zone: 01 §3 would make every read outside the workspace
/// Outside, so `grep Host ~/.ssh/config` would ask at L0; the replay reads the matrix for what a request changes.
#[must_use]
pub fn zone(ctx: &Context, case: &Case, a: &Analysis) -> Zone {
    let base = match case.kind {
        Kind::GitPush | Kind::GitRead => repo_zone(ctx, case.repo.as_deref().unwrap_or_default(), Zone::Outside),
        // The repository the call will act on, exactly as the connector reads it; one it doesn't name exactly is
        // Outside.
        Kind::Mcp => {
            case.args.as_ref().filter(|a| a.get("repo").is_some() || a.get("owner").is_some()).map_or(Zone::Mine, |a| {
                match (a.get("repo"), a.get("owner").and_then(|o| o.as_str())) {
                    // `owner` alone (listing a user's repositories): that user.
                    (None, Some(o)) => repo_zone(ctx, &format!("{o}/*"), Zone::Outside),
                    _ => effects::tool_repo(a).map_or(Zone::Outside, |r| repo_zone(ctx, &r, Zone::Outside)),
                }
            })
        }
        _ => Zone::Mine,
    };
    let mut z = base;
    for e in &a.effects {
        let ez = match e {
            Effect::FileWrite {
                path,
            } => area_zone(&path.area, ctx),
            Effect::FileDelete {
                paths,
                ..
            } => paths.iter().map(|p| area_zone(&p.area, ctx)).max().unwrap_or(Zone::Mine),
            Effect::GitPush {
                repo,
                ..
            }
            | Effect::GitRefDelete {
                repo,
                ..
            } => repo_zone(ctx, repo, base),
            Effect::Message {
                channel: Channel::GitHub,
                to,
                ..
            } => to.iter().map(|r| repo_zone(ctx, r, base)).max().unwrap_or(base),
            Effect::Message {
                ..
            }
            | Effect::Infra {
                local: false,
                ..
            }
            | Effect::Exec {
                source: CodeSource::Downloaded,
                ..
            } => Zone::Outside,
            Effect::Network {
                host,
                method: NetMethod::Write | NetMethod::Connect | NetMethod::UploadExecutable,
            } => host_zone(ctx, host),
            _ => continue,
        };
        z = z.max(ez);
    }
    z
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
            }
        }
        EngineKind::Effects => {
            let a = if policy.nets {
                netted(ctx, case)
            } else {
                None
            }
            .unwrap_or_else(|| analyze(ctx, case));
            let z = zone(ctx, case, &a);
            let (ask, why) = if !a.floor.is_empty() {
                (true, format!("always asked: {}", a.floor.iter().map(|f| f.label()).collect::<Vec<_>>().join(", ")))
            } else if a.certainty == Certainty::None {
                (true, "couldn't be read".to_owned())
            } else if policy.ask_on_partial && a.certainty == Certainty::Partial {
                (true, "partly unread".to_owned())
            } else if policy.cell(z, a.level) == Cell::Ask {
                (true, format!("{z:?} · {}", a.level.label()))
            } else {
                (false, format!("Auto · {z:?} · {}", a.level.label()))
            };
            Decision {
                ask,
                why,
                analysis: Some(a),
                zone: Some(z),
            }
        }
    }
}
