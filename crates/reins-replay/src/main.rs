//! `reins-replay run [--engine guard|effects] [--policy FILE] [--corpus DIR] [--baseline FILE] [--check] [--json]
//! [--verbose] [--write-baseline]`, and `reins-replay import --log FILE --labels FILE --out FILE` (local only: it reads
//! the owner's activity log).

use std::path::PathBuf;
use std::process::ExitCode;

use reins_replay::corpus::Corpus;
use reins_replay::engine::{EngineKind, Policy};
use reins_replay::report::{self, Baseline};

const USAGE: &str = "usage:
  reins-replay run [--engine guard|effects] [--policy FILE] [--corpus DIR] [--baseline FILE] [--check] [--json] [--verbose] [--write-baseline]
  reins-replay import --log FILE --labels FILE --out FILE";

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
        Some(other) => return Err(format!("unknown engine {other}")),
    };
    let corpus_dir = value(args, "--corpus").map_or_else(reins_replay::default_corpus, PathBuf::from);
    let policy_path = value(args, "--policy").map_or_else(reins_replay::default_policy, PathBuf::from);
    let policy: Policy = if policy_path.exists() {
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
        };
        let text = serde_json::to_string_pretty(&b).map_err(|e| e.to_string())?;
        std::fs::write(&baseline_path, format!("{text}\n")).map_err(|e| format!("{}: {e}", baseline_path.display()))?;
    }
    if args.iter().any(|a| a == "--json") {
        println!("{}", serde_json::to_string_pretty(&r).map_err(|e| e.to_string())?);
    } else {
        print!("{}", report::text(&r, args.iter().any(|a| a == "--verbose")));
    }
    if args.iter().any(|a| a == "--check") {
        let baseline: Option<Baseline> = if baseline_path.exists() {
            Some(read_json(&baseline_path)?)
        } else {
            None
        };
        let fails = report::gate(&r, baseline.as_ref());
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

fn import(args: &[String]) -> Result<ExitCode, String> {
    let log = PathBuf::from(value(args, "--log").ok_or(USAGE)?);
    let labels = PathBuf::from(value(args, "--labels").ok_or(USAGE)?);
    let out = PathBuf::from(value(args, "--out").ok_or(USAGE)?);
    let labels: reins_replay::anonymize::Labels = read_json(&labels)?;
    let text = std::fs::read_to_string(&log).map_err(|e| format!("{}: {e}", log.display()))?;
    let cases = reins_replay::anonymize::import(&text, &labels)?;
    let mut lines = String::new();
    for c in &cases {
        lines.push_str(&serde_json::to_string(c).map_err(|e| e.to_string())?);
        lines.push('\n');
    }
    std::fs::write(&out, lines).map_err(|e| format!("{}: {e}", out.display()))?;
    eprintln!("{} cases written to {}", cases.len(), out.display());
    Ok(ExitCode::SUCCESS)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("run") => run(&args[1..]),
        Some("import") => import(&args[1..]),
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
