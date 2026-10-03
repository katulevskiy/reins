//! `laya-try <package-dir> [situation-file ...]`: what a Laya package says about situations in the Autopilot format
//! (spec §4). Without files, one situation is read from standard input. For each one: the probabilities from the facts
//! alone and with the AI-written part, after the prompt-injection rule, and the verdict with no memory.

use std::io::Read;
use std::path::Path;
use std::process::ExitCode;

use reins_laya::Judge;

fn pct(p: f32) -> String {
    format!("{:5.1}%", p * 100.0)
}

fn row(name: &str, p: [f32; 3]) -> String {
    format!("{name:<9} approve {}  deny {}  ask {}", pct(p[0]), pct(p[1]), pct(p[2]))
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(dir) = args.first() else {
        eprintln!("usage: laya-try <package-dir> [situation-file ...]   (stdin when no file is given)");
        return ExitCode::from(2);
    };
    let judge = match Judge::open(Path::new(dir)) {
        Ok(j) => j,
        Err(e) => {
            eprintln!("cannot open the package {dir}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let mut situations: Vec<(String, String)> = Vec::new();
    if args.len() == 1 {
        let mut text = String::new();
        if let Err(e) = std::io::stdin().read_to_string(&mut text) {
            eprintln!("cannot read standard input: {e}");
            return ExitCode::FAILURE;
        }
        situations.push(("stdin".to_owned(), text));
    }
    for file in &args[1..] {
        match std::fs::read_to_string(file) {
            Ok(text) => situations.push((file.clone(), text)),
            Err(e) => {
                eprintln!("cannot read {file}: {e}");
                return ExitCode::FAILURE;
            }
        }
    }
    println!(
        "package {} (max_len {}, temperature {:.3})",
        judge.laya.id,
        judge.laya.config.max_len,
        judge.laya.config.temperature()
    );
    for (name, text) in situations {
        match judge.judge(&text) {
            Ok(j) => {
                println!("\n== {name}");
                println!("{}", row("facts", j.facts.probs));
                if j.full != j.facts {
                    println!("{}", row("full", j.full.probs));
                }
                let c = j.combined;
                println!("{}", row("combined", [c.approve, c.deny, c.ask]));
                println!(
                    "escalate {}  confidence {}  verdict {:?}",
                    pct(j.facts.escalate.max(j.full.escalate)),
                    pct(c.confidence()),
                    j.verdict
                );
            }
            Err(e) => {
                eprintln!("{name}: {e}");
                return ExitCode::FAILURE;
            }
        }
    }
    ExitCode::SUCCESS
}
