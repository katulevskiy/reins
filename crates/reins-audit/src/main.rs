//! `reins-audit verify <export.jsonl> --key <key_id>=<base64> [--key …] [--anchor <checkpoint.json>]`
//!
//! Verifies an organization's audit export offline (see the library docs). Prints what it verified, or why it
//! failed and exits 1.

use std::io::BufReader;
use std::process::ExitCode;

use reins_audit::{TrustedKey, verify};
use reins_proto::managed::{Checkpoint, Envelope};

const USAGE: &str = "usage: reins-audit verify <export.jsonl> --key <key_id>=<base64> [--anchor <checkpoint.json>]";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(text) => {
            println!("{text}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

/// An anchor file: a bare checkpoint, or a signed checkpoint envelope (whose body is read).
fn read_anchor(path: &str) -> Result<Checkpoint, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    if let Ok(cp) = serde_json::from_str::<Checkpoint>(&text) {
        return Ok(cp);
    }
    let env: Envelope = serde_json::from_str(&text).map_err(|e| format!("{path}: not a checkpoint: {e}"))?;
    env.parse().map_err(|e| format!("{path}: not a checkpoint: {e}"))
}

fn run(args: &[String]) -> Result<String, String> {
    let mut it = args.iter();
    if it.next().map(String::as_str) != Some("verify") {
        return Err(USAGE.to_owned());
    }
    let mut file: Option<&str> = None;
    let mut keys: Vec<TrustedKey> = Vec::new();
    let mut anchor: Option<Checkpoint> = None;
    while let Some(a) = it.next() {
        match a.as_str() {
            "--key" => {
                let spec = it.next().ok_or(USAGE)?;
                keys.push(TrustedKey::parse(spec).ok_or_else(|| format!("not a key: {spec}"))?);
            }
            "--anchor" => anchor = Some(read_anchor(it.next().ok_or(USAGE)?)?),
            f if file.is_none() && !f.starts_with("--") => file = Some(f),
            _ => return Err(USAGE.to_owned()),
        }
    }
    let file = file.ok_or(USAGE)?;
    if keys.is_empty() {
        return Err(format!("name the decision service key with --key\n{USAGE}"));
    }
    let input = std::fs::File::open(file).map_err(|e| format!("{file}: {e}"))?;
    let report = verify(BufReader::new(input), &keys, anchor.as_ref()).map_err(|e| format!("FAILED: {e}"))?;
    Ok(format!(
        "OK: {} events ({}..{}) of {}, {} checkpoints, head {}",
        report.events, report.first_seq, report.last_seq, report.org, report.checkpoints, report.head_hash
    ))
}
