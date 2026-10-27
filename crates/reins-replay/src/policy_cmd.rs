//! `reins-replay policy …`: an org's policy repo in its own CI (`docs/self-hosting.md`, "Policy repo").
//!
//! - `policy check <dir>`: compiles the repo; prints `file:line: message` for each error and exits 1, else 0.
//! - `policy render <dir>`: rewrites the files in their normal form (what the console exports), in place.
//! - `policy simulate <dir> --history <file.jsonl> --base <dir> [--out report.json]`: replays the history (the
//!   console's export of retained decisions, [`Retained`] JSON lines) under the base policy and the draft; prints the
//!   summary line and writes the report.
//!
//! The server compiles and simulates every PR itself, with its verified facts and the org's group ids: that report is
//! the activation evidence. This one is for authors, before they push.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use reins_policy::org::OrgPolicy;
use reins_policy::simulate::{Retained, SimulationReport, simulate_org};
use reins_policy::source::{self, Compiled, check_size, compile, is_policy_file};

pub const USAGE: &str = "usage:
  reins-replay policy check DIR
  reins-replay policy render DIR
  reins-replay policy simulate DIR --history FILE.jsonl --base DIR [--out report.json]";

/// The policy files under `dir` (relative paths with `/`), skipping hidden directories such as `.git`.
pub fn read_repo(dir: &Path) -> Result<BTreeMap<String, String>, String> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, String>) -> Result<(), String> {
        let entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            if entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            if path.is_dir() {
                walk(root, &path, out)?;
                continue;
            }
            let rel = path.strip_prefix(root).map_err(|e| e.to_string())?.to_string_lossy().replace('\\', "/");
            if is_policy_file(&rel) {
                out.insert(rel, std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?);
            }
        }
        Ok(())
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out)?;
    check_size(&out)?;
    Ok(out)
}

/// Compiles `dir`; the errors as `file:line: message` lines.
pub fn check(dir: &Path) -> Result<Result<Compiled, Vec<String>>, String> {
    let files = read_repo(dir)?;
    Ok(compile(&files).map_err(|errs| errs.iter().map(ToString::to_string).collect()))
}

/// Rewrites `dir`'s policy files in their rendered form; rules files the render doesn't produce are removed.
pub fn render(dir: &Path) -> Result<Result<(), Vec<String>>, String> {
    let c = match check(dir)? {
        Ok(c) => c,
        Err(e) => return Ok(Err(e)),
    };
    let files = source::render(&c.policy, &c.plan);
    for old in read_repo(dir)?.keys().filter(|p| !files.contains_key(*p)) {
        std::fs::remove_file(dir.join(old)).map_err(|e| format!("{old}: {e}"))?;
    }
    for (path, text) in &files {
        let to = dir.join(path);
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(&to, text).map_err(|e| format!("{path}: {e}"))?;
    }
    Ok(Ok(()))
}

/// Reads [`Retained`] JSON lines (blank lines skipped).
pub fn read_history(path: &Path) -> Result<Vec<Retained>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    text.lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty())
        .map(|(i, l)| serde_json::from_str(l).map_err(|e| format!("{}:{}: {e}", path.display(), i + 1)))
        .collect()
}

fn compiled(dir: &Path) -> Result<OrgPolicy, String> {
    match check(dir)? {
        Ok(c) => Ok(c.policy),
        Err(e) => Err(e.join("\n")),
    }
}

/// The report for draft `dir` against `base` over `history` (members' own policies aren't exported: none apply).
pub fn simulate(dir: &Path, history: &Path, base: &Path) -> Result<SimulationReport, String> {
    let draft = compiled(dir)?;
    let base = compiled(base)?;
    Ok(simulate_org(&read_history(history)?, &BTreeMap::new(), &base, &draft))
}

fn value(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|p| args.get(p + 1)).cloned()
}

/// `reins-replay policy …` (`args` after `policy`).
pub fn main(args: &[String]) -> Result<ExitCode, String> {
    let dir = args.get(1).filter(|a| !a.starts_with("--")).map(PathBuf::from).ok_or(USAGE)?;
    match args.first().map(String::as_str) {
        Some("check") => match check(&dir)? {
            Ok(c) => {
                println!("ok: {} rules, {}", c.policy.rules.len(), c.hash);
                Ok(ExitCode::SUCCESS)
            }
            Err(lines) => {
                for l in lines {
                    println!("{l}");
                }
                Ok(ExitCode::from(1))
            }
        },
        Some("render") => match render(&dir)? {
            Ok(()) => Ok(ExitCode::SUCCESS),
            Err(lines) => {
                for l in lines {
                    println!("{l}");
                }
                Ok(ExitCode::from(1))
            }
        },
        Some("simulate") => {
            let history = value(args, "--history").ok_or(USAGE)?;
            let base = value(args, "--base").ok_or(USAGE)?;
            let out = value(args, "--out").unwrap_or_else(|| "report.json".to_owned());
            let report = simulate(&dir, Path::new(&history), Path::new(&base))?;
            println!("{}", report.line());
            let json = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?;
            std::fs::write(&out, json + "\n").map_err(|e| format!("{out}: {e}"))?;
            Ok(ExitCode::SUCCESS)
        }
        _ => Err(USAGE.to_owned()),
    }
}
