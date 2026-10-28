//! P2 replaces P1's stand-in zones (Mine for the workspace, the owner's GitHub owners and devices, Outside otherwise)
//! with the context graph. On the recorded history the two agree, except where the graph knows more (a fact the
//! stand-in didn't have) or less (it never guesses that a repository named only `origin` is the owner's outside a
//! known worktree). This test keeps P1's `zone()` as the oracle and lists every difference.

use reins_policy::context::Zone;
use reins_policy::effects::{self, Analysis, Area, Channel, CodeSource, Effect, NetMethod};
use reins_replay::corpus::{Case, Context, Corpus, Kind};
use reins_replay::engine::{self, EngineKind, Policy};

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

#[test]
fn matrix_zones_are_never_looser_than_the_stand_in() {
    let corpus = Corpus::load(&reins_replay::default_corpus()).expect("the corpus loads");
    let policy = Policy::default();
    let mut stricter = Vec::new();
    let mut looser = Vec::new();
    for case in corpus.cases.iter().filter(|c| !c.tagged("adversarial")) {
        let d = engine::decide(&corpus.context, case, &policy, EngineKind::Effects);
        let a = d.analysis.as_ref().expect("an analysis");
        let old = zone(&corpus.context, case, a);
        let new = d.zone.expect("a zone");
        let line = format!("{} {:?}: {old:?} → {new:?} · {}", case.id, case.kind, a.lines().join(" | "));
        match new.cmp(&old) {
            std::cmp::Ordering::Greater => stricter.push(case.id.clone()),
            std::cmp::Ordering::Less => looser.push(line),
            std::cmp::Ordering::Equal => {}
        }
    }
    assert!(looser.is_empty(), "{}", looser.join("\n"));
    // Where the graph is stricter: a remote on a host that isn't github.com (`gh-direct`), and a command that works in
    // two worktrees, whose `origin` is not one repository's (the review's rule: Outside). A `cd` into a worktree the
    // command just added (`git worktree add P && cd P`) stays in the same repository.
    stricter.sort();
    let known = ["9JZnBzfbt8GP", "XPaCSf89MVZd", "squIutYcLgRP"];
    assert_eq!(stricter, known, "a new difference: look at it, then list it here");
}
