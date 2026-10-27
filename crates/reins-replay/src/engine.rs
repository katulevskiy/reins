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
    /// Ask whenever certainty is `Partial` (P2 gives these to the model's risk head instead).
    #[serde(default)]
    pub ask_on_partial: bool,
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
        Kind::Mcp => case
            .args
            .as_ref()
            .and_then(|a| {
                a.get("repo").and_then(|r| r.as_str()).map(|r| {
                    let r = if r.contains('/') {
                        r.to_owned()
                    } else {
                        format!("{}/{r}", a.get("owner").and_then(|o| o.as_str()).unwrap_or("?"))
                    };
                    repo_zone(ctx, &r, Zone::Mine)
                })
            })
            .unwrap_or(Zone::Mine),
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
            } => Zone::Outside,
            Effect::Network {
                host,
                method: NetMethod::Write | NetMethod::Connect | NetMethod::UploadExecutable,
            } => host_zone(ctx, host),
            Effect::Infra {
                local: false,
                ..
            }
            | Effect::Exec {
                source: CodeSource::Downloaded,
                ..
            } => Zone::Outside,
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
            let a = analyze(ctx, case);
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
