//! `reins-replay run [--engine guard|effects|desktop] [--policy FILE] [--corpus DIR] [--baseline FILE] [--check] [--json]
//! [--verbose] [--write-baseline]`, and `reins-replay import --log FILE --labels FILE --out FILE` (local only: it reads
//! the owner's activity log).

use std::path::PathBuf;
use std::process::ExitCode;

use reins_replay::corpus::Corpus;
use reins_replay::engine::{EngineKind, Policy};
use reins_replay::report::{self, Baseline};

const USAGE: &str = "usage:
  reins-replay run [--engine guard|effects|desktop] [--policy FILE] [--corpus DIR] [--baseline FILE] [--check] [--json] [--verbose] [--write-baseline]
  reins-replay import --log FILE --labels FILE --out FILE
  reins-replay policy check|render DIR
  reins-replay policy simulate DIR --history FILE.jsonl --base DIR [--out report.json]";

fn value(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|p| args.get(p + 1)).cloned()
}

fn read_json<T: serde::de::DeserializeOwned>(path: &PathBuf) -> Result<T, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display()))
}

fn run(args: &[String]) -> Result<ExitCode, String> {
    let engine = match value(args, "--engine").as_deref() {
        None | Some("effects") => EngineKind::Effects,
        Some("guard") => EngineKind::Guard,
        Some("desktop") => EngineKind::Desktop,
        Some(other) => return Err(format!("unknown engine {other}")),
    };
    let corpus_dir = value(args, "--corpus").map_or_else(reins_replay::default_corpus, PathBuf::from);
    let check = args.iter().any(|a| a == "--check");
    let given = value(args, "--policy");
    let policy_path = given.clone().map_or_else(reins_replay::default_policy, PathBuf::from);
    // A policy named (or the gate's own) must be there and valid: a typo must not fall back to another policy.
    let policy: Policy = if policy_path.exists() || given.is_some() || check {
        read_json(&policy_path)?
    } else {
        Policy::default()
    };
    let corpus = Corpus::load(&corpus_dir)?;
    let r = report::run(&corpus, &policy, engine);
    let baseline_path = value(args, "--baseline").map_or_else(reins_replay::default_baseline, PathBuf::from);
    if args.iter().any(|a| a == "--write-baseline") {
        let b = Baseline {
            recall: r.recall,
            known_fp_asked: r.known_fp_asked,
            full_certainty: r.full_certainty.unwrap_or(0.0),
            asks: r.asks,
            ask_rate: r.ask_rate,
            developer_ask_rate: Some(r.developer_ask_rate),
            overnight_decisions_mean: r.overnight.as_ref().map(|o| o.decisions_mean),
        };
        let text = serde_json::to_string_pretty(&b).map_err(|e| e.to_string())?;
        std::fs::write(&baseline_path, format!("{text}\n")).map_err(|e| format!("{}: {e}", baseline_path.display()))?;
    }
    if args.iter().any(|a| a == "--json") {
        println!("{}", serde_json::to_string_pretty(&r).map_err(|e| e.to_string())?);
    } else {
        print!("{}", report::text(&r, args.iter().any(|a| a == "--verbose")));
    }
    if check {
        // The gate compares with its baseline: one that is missing is a failure, not a pass.
        let baseline: Baseline = read_json(&baseline_path)?;
        let fails = report::gate(&r, Some(&baseline));
        if !fails.is_empty() {
            for f in &fails {
                eprintln!("replay gate: {f}");
            }
            return Ok(ExitCode::FAILURE);
        }
        eprintln!("replay gate: passed");
    }
    Ok(ExitCode::SUCCESS)
}

/// The branch checked out in `cwd` at `at`, from its worktree's reflog (`checkout: moving from A to B`): the last move
/// before `at`, else the first move's origin after it, else today's branch when it never moved. `None` for a folder
/// that is gone, a detached head, or anything git does not answer.
fn branch_at(cwd: &str, at: i64) -> Option<String> {
    let git = |args: &[&str]| -> Option<String> {
        let out = std::process::Command::new("git").arg("-C").arg(cwd).args(args).output().ok()?;
        out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    };
    let log = git(&["reflog", "show", "--date=unix", "--format=%gd %gs", "HEAD"])?;
    // Newest first: `HEAD@{1730000000} checkout: moving from a to b`.
    let moves: Vec<(i64, String, String)> = log
        .lines()
        .filter_map(|l| {
            let (stamp, rest) = l.split_once(' ')?;
            let t = stamp.strip_prefix("HEAD@{")?.strip_suffix('}')?.parse().ok()?;
            let (from, to) = rest.strip_prefix("checkout: moving from ")?.split_once(" to ")?;
            Some((t, from.to_owned(), to.to_owned()))
        })
        .collect();
    let branch = if let Some((_, _, to)) = moves.iter().find(|(t, ..)| *t <= at) {
        to.clone()
    } else if let Some((_, from, _)) = moves.last() {
        from.clone()
    } else {
        git(&["branch", "--show-current"])?.trim().to_owned()
    };
    let detached = branch.is_empty() || (branch.len() >= 7 && branch.chars().all(|c| c.is_ascii_hexdigit()));
    (!detached).then_some(branch)
}

fn import(args: &[String]) -> Result<ExitCode, String> {
    let log = PathBuf::from(value(args, "--log").ok_or(USAGE)?);
    let labels = PathBuf::from(value(args, "--labels").ok_or(USAGE)?);
    let out = PathBuf::from(value(args, "--out").ok_or(USAGE)?);
    let labels: reins_replay::anonymize::Labels = read_json(&labels)?;
    let text = std::fs::read_to_string(&log).map_err(|e| format!("{}: {e}", log.display()))?;
    let cases = reins_replay::anonymize::import_with(&text, &labels, &branch_at)?;
    let mut lines = String::new();
    for c in &cases {
        lines.push_str(&serde_json::to_string(c).map_err(|e| e.to_string())?);
        lines.push('\n');
    }
    std::fs::write(&out, lines).map_err(|e| format!("{}: {e}", out.display()))?;
    eprintln!("{} cases written to {}", cases.len(), out.display());
    Ok(ExitCode::SUCCESS)
}

/// `reins-replay explain [--policy FILE] ID…`: a case's request, effects and decision, for reviewing labels and the engine.
fn explain(args: &[String]) -> Result<ExitCode, String> {
    let corpus_dir = value(args, "--corpus").map_or_else(reins_replay::default_corpus, PathBuf::from);
    let corpus = Corpus::load(&corpus_dir)?;
    let policy_file = value(args, "--policy");
    let policy: Policy = match &policy_file {
        Some(p) => read_json(&PathBuf::from(p))?,
        None => Policy::default(),
    };
    let flag_values = [value(args, "--corpus"), policy_file];
    for id in args.iter().filter(|a| !a.starts_with("--") && !flag_values.iter().flatten().any(|v| v == *a)) {
        let Some(case) = corpus.cases.iter().find(|c| &c.id == id) else {
            eprintln!("{id}: no such case");
            continue;
        };
        let d = reins_replay::engine::decide(&corpus.context, case, &policy, EngineKind::Effects);
        println!("=== {id} · {:?} · {:?} {:?} · {}", case.kind, case.label, case.tags, d.why);
        if let Some(c) = &case.command {
            println!("{c}");
        }
        if let Some(a) = &d.analysis {
            println!("--- level {:?} · certainty {:?} · floor {:?}", a.level, a.certainty, a.floor);
            for e in &a.effects {
                println!("  {e:?}");
            }
            for u in &a.unresolved {
                println!("  ? {u}");
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// `reins-replay unresolved`: what the engine could not read, by reason, over the corpus.
fn unresolved(args: &[String]) -> Result<ExitCode, String> {
    let corpus_dir = value(args, "--corpus").map_or_else(reins_replay::default_corpus, PathBuf::from);
    let corpus = Corpus::load(&corpus_dir)?;
    let mut by: std::collections::BTreeMap<String, Vec<String>> = std::collections::BTreeMap::new();
    for case in &corpus.cases {
        let a = reins_replay::engine::analyze(&corpus.context, case);
        if a.certainty == reins_policy::effects::Certainty::Full || !case.kind.developer() {
            continue;
        }
        for u in &a.unresolved {
            by.entry(u.clone()).or_default().push(format!("{}:{:?}", case.id, a.certainty));
        }
    }
    let mut v: Vec<_> = by.into_iter().collect();
    v.sort_by_key(|(_, ids)| std::cmp::Reverse(ids.len()));
    for (u, ids) in v {
        println!("{:3} {u} · {}", ids.len(), ids.iter().take(4).cloned().collect::<Vec<_>>().join(" "));
    }
    Ok(ExitCode::SUCCESS)
}

/// `reins-replay list [--auto|--ask] [--level L2] [--certainty Partial]`: one line per matching case.
fn list(args: &[String]) -> Result<ExitCode, String> {
    let corpus_dir = value(args, "--corpus").map_or_else(reins_replay::default_corpus, PathBuf::from);
    let corpus = Corpus::load(&corpus_dir)?;
    let policy: Policy = match value(args, "--policy") {
        Some(p) => read_json(&PathBuf::from(p))?,
        None => Policy::default(),
    };
    let level = value(args, "--level");
    let certainty = value(args, "--certainty");
    for case in &corpus.cases {
        let d = reins_replay::engine::decide(&corpus.context, case, &policy, EngineKind::Effects);
        let Some(a) = &d.analysis else {
            continue;
        };
        if (args.iter().any(|x| x == "--auto") && d.ask) || (args.iter().any(|x| x == "--ask") && !d.ask) {
            continue;
        }
        if level.as_deref().is_some_and(|l| format!("{:?}", a.level) != l)
            || certainty.as_deref().is_some_and(|c| format!("{:?}", a.certainty) != c)
        {
            continue;
        }
        println!(
            "{} · {:?} · {:?} {:?} · {} · {}",
            case.id,
            case.kind,
            a.level,
            a.certainty,
            d.why,
            a.lines().join(" | ")
        );
    }
    Ok(ExitCode::SUCCESS)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("run") => run(&args[1..]),
        Some("import") => import(&args[1..]),
        Some("explain") => explain(&args[1..]),
        Some("unresolved") => unresolved(&args[1..]),
        Some("list") => list(&args[1..]),
        Some("policy") => reins_replay::policy_cmd::main(&args[1..]),
        _ => Err(USAGE.to_owned()),
    };
    match result {
        Ok(code) => code,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::from(2)
        }
    }
}
