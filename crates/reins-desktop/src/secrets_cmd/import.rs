//! `reins secrets import`: moves what a scan found into the vault on the phone (`secrets_import`), after the person
//! picked what goes. `--undo` brings the files back from the trash; `--finish` moves the SSH keys an import kept until
//! the Reins agent signed with them. A person's command ([`super::person`]): the scan it runs is approved on the
//! phone, the move is the phone's batch approval, and an undo (which puts secrets back into files) is approved on the
//! phone too.

use std::collections::BTreeSet;
use std::path::PathBuf;

use super::person::{self, Needs, Terminal};
use crate::config::{Config, Paths};
use crate::secrets_import::{self, Context, Receipt, real, receipt};
use crate::secrets_scan::{Caller, ScanReport};

#[derive(Clone, Debug, clap::Args)]
pub struct ImportArgs {
    /// A report from `reins secrets scan --json` (default: scan now).
    #[arg(long, value_name = "FILE")]
    pub from_scan: Option<PathBuf>,
    /// What to move, by the ids `reins secrets scan` shows (a file's id moves its pre-ticked secrets),
    /// comma-separated. Without it, you pick on the terminal.
    #[arg(long, value_delimiter = ',', value_name = "IDS")]
    pub select: Vec<String>,
    /// Move everything the scan pre-ticks.
    #[arg(long)]
    pub all_suggested: bool,
    /// Keep the local copies too: the vault gets a backup, nothing on this computer changes.
    #[arg(long)]
    pub keep_local: bool,
    /// Undo an import: put back the files it changed (from the trash). `last` or a receipt file.
    #[arg(long, value_name = "RECEIPT", conflicts_with_all = ["from_scan", "select", "all_suggested", "keep_local", "finish"])]
    pub undo: Option<String>,
    /// Move to the trash the SSH keys an import kept until the Reins agent signed with them (`last` or a receipt).
    #[arg(long, value_name = "RECEIPT", num_args = 0..=1, default_missing_value = "last",
          conflicts_with_all = ["from_scan", "select", "all_suggested", "keep_local"])]
    pub finish: Option<String>,
    /// Delete for good the originals an import sent to the trash (they still hold the secrets). Undo can no longer
    /// bring them back. `last` or a receipt.
    #[arg(long, value_name = "RECEIPT", num_args = 0..=1, default_missing_value = "last",
          conflicts_with_all = ["from_scan", "select", "all_suggested", "keep_local", "undo", "finish"])]
    pub purge_originals: Option<String>,
}

/// What the picker shows: one line per finding, ticked or not, numbered from 1.
#[must_use]
pub fn picker_lines(report: &ScanReport, ticked: &BTreeSet<String>) -> Vec<String> {
    report
        .findings
        .iter()
        .enumerate()
        .map(|(n, f)| {
            let on = f.items.iter().filter(|i| ticked.contains(&i.id)).count();
            let mark = if on > 0 {
                "[x]"
            } else {
                "[ ]"
            };
            let risks: Vec<&str> = f.risks.iter().map(|r| r.label()).collect();
            format!("{mark} {:>2}. {}  {}  {}  {}", n + 1, f.title, f.place, f.summary, risks.join(", "))
        })
        .collect()
}

/// What the person typed at the picker.
#[derive(Debug, PartialEq, Eq)]
pub enum Picked {
    /// Go on with what is ticked.
    Done,
    Quit,
    /// Ticked or unticked these findings (0-based).
    Toggle(Vec<usize>),
    Bad(String),
}

/// Reads a picker line: Enter goes on, `q` stops, numbers (or ranges, `2-4`) tick or untick.
#[must_use]
pub fn parse_pick(line: &str, count: usize) -> Picked {
    let line = line.trim();
    if line.is_empty() {
        return Picked::Done;
    }
    if line.eq_ignore_ascii_case("q") {
        return Picked::Quit;
    }
    let mut out = Vec::new();
    for word in line.split(|c: char| c == ',' || c.is_whitespace()).filter(|w| !w.is_empty()) {
        let (a, b) = word.split_once('-').unwrap_or((word, word));
        let (Ok(a), Ok(b)) = (a.parse::<usize>(), b.parse::<usize>()) else {
            return Picked::Bad(format!("{word} is not a number from the list"));
        };
        if a == 0 || b < a || b > count {
            return Picked::Bad(format!("{word} is not a number from the list (1 to {count})"));
        }
        out.extend((a - 1)..b);
    }
    Picked::Toggle(out)
}

/// Ticks or unticks a finding: on, its pre-ticked items (or all that may be secrets); off, none of it.
pub fn toggle(report: &ScanReport, ticked: &mut BTreeSet<String>, n: usize) {
    let Some(f) = report.findings.get(n) else {
        return;
    };
    if f.items.iter().any(|i| ticked.contains(&i.id)) {
        for i in &f.items {
            ticked.remove(&i.id);
        }
    } else if let Ok(ids) = secrets_import::select(report, std::slice::from_ref(&f.id), false) {
        ticked.extend(ids);
    }
}

/// The picker on the terminal; `None` when the person stopped.
fn pick(report: &ScanReport) -> Result<Option<BTreeSet<String>>, String> {
    let mut ticked = secrets_import::select(report, &[], true)?;
    loop {
        eprintln!();
        for line in picker_lines(report, &ticked) {
            eprintln!("{line}");
        }
        let line = person::ask_tty("\nNumbers to tick or untick (2 5, 3-6), Enter to go on, q to stop: ")
            .ok_or("no terminal to pick on")?;
        match parse_pick(&line, report.findings.len()) {
            Picked::Done => return Ok(Some(ticked)),
            Picked::Quit => return Ok(None),
            Picked::Toggle(ns) => {
                for n in ns {
                    toggle(report, &mut ticked, n);
                }
            }
            Picked::Bad(why) => eprintln!("{why}"),
        }
    }
}

fn load_report(file: &PathBuf) -> Result<ScanReport, String> {
    let text = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let report: ScanReport = serde_json::from_str(&text)
        .map_err(|e| format!("{}: not a report of reins secrets scan ({e})", file.display()))?;
    if report.version != crate::secrets_scan::SCHEMA_VERSION {
        return Err(format!("{}: made by another version of reins; scan again", file.display()));
    }
    Ok(report)
}

/// The lines after an import: what changed and how to use it, what is left, how to undo.
#[must_use]
pub fn summary(receipt: &Receipt, path: Option<&std::path::Path>) -> Vec<String> {
    let mut lines = Vec::new();
    let moved = receipt.moved.len();
    lines.push(match moved {
        0 => "Nothing reached your vault.".to_owned(),
        1 => "1 secret is in your vault.".to_owned(),
        n => format!("{n} secrets are in your vault."),
    });
    let on_disk = moved - receipt.off_disk();
    if moved > 0 && on_disk > 0 {
        lines.push(format!("{on_disk} of them are still on disk too (see below)."));
    }
    for d in &receipt.done {
        lines.push(format!(
            "{} {}",
            if d.ok {
                "✓"
            } else {
                "!"
            },
            d.what
        ));
        lines.push(format!("    {}", d.how));
    }
    for n in &receipt.notes {
        lines.push(format!("! {n}"));
    }
    if let Some(p) = path {
        if !receipt.trashed().is_empty() {
            lines.push(format!(
                "The originals in the trash still hold the secrets. Once all works: reins secrets import --purge-originals {}",
                p.display()
            ));
        }
        lines.push(format!("Undo: reins secrets import --undo {}", p.display()));
    }
    lines
}

/// Runs `reins secrets import`.
pub async fn run(args: &ImportArgs, paths: &Paths, config: &Config) -> Result<Vec<String>, String> {
    let home = crate::config::home_dir()?;
    let system = real::System {
        paths: paths.clone(),
        config: config.clone(),
        home: home.clone(),
    };
    let caller = Caller::from_env();
    if let Some(which) = &args.finish {
        person::check(
            "reins secrets import --finish",
            "Move the kept SSH keys to the trash?",
            &caller,
            Terminal::now(),
            Needs {
                stdout: true,
            },
            &mut person::ask_tty,
        )?;
        let (path, mut receipt) = receipt::load(&paths.state_dir, which)?;
        return receipt::finish(&home, &paths.state_dir, &path, &mut receipt, &system).await;
    }
    if let Some(which) = &args.purge_originals {
        let (path, mut receipt) = receipt::load(&paths.state_dir, which)?;
        let n = receipt.trashed().len();
        person::check(
            "reins secrets import --purge-originals",
            &format!(
                "Delete the {n} original file(s) of this import from the trash for good? Undo cannot bring them back."
            ),
            &caller,
            Terminal::now(),
            Needs {
                stdout: true,
            },
            &mut person::ask_tty,
        )?;
        return receipt::purge(&paths.state_dir, &path, &mut receipt, &system);
    }
    if let Some(which) = &args.undo {
        let (path, mut receipt) = receipt::load(&paths.state_dir, which)?;
        let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
        person::authorize(
            "reins secrets import --undo",
            &format!(
                "Put back the {} file change(s) of this import? Secrets come back into files.",
                receipt.changes.len()
            ),
            &caller,
            Terminal::now(),
            Needs {
                stdout: true,
            },
            &mut person::ask_tty,
            &person::Phone {
                paths,
                config,
            },
            &person::undo_question(&cwd, &caller),
        )
        .await?;
        return receipt::undo(&home, &paths.state_dir, &path, &mut receipt, &system);
    }
    let report = match &args.from_scan {
        Some(file) => {
            person::check(
                "reins secrets import",
                "Move secrets into your vault?",
                &caller,
                Terminal::now(),
                Needs {
                    stdout: true,
                },
                &mut person::ask_tty,
            )?;
            load_report(file)?
        }
        None => super::scan::authorized_scan(&[], true, paths, config, &home).await?,
    };
    if report.findings.is_empty() {
        return Ok(vec!["No secrets found.".to_owned()]);
    }
    // Picking on the terminal (else what --select or --all-suggested says, and new names for every clash).
    let interactive = !args.all_suggested && args.select.is_empty();
    let selected = if interactive {
        match pick(&report)? {
            Some(s) => s,
            None => return Ok(vec!["Nothing done.".to_owned()]),
        }
    } else {
        secrets_import::select(&report, &args.select, args.all_suggested)?
    };
    let computer = crate::secrets_scan::computer_name();
    let mut ctx = Context {
        home,
        state_dir: paths.state_dir.clone(),
        machine: secrets_import::this_machine(paths, &computer)?,
        computer,
        caller: Caller::Person,
        merge: BTreeSet::new(),
    };
    let writer = real::PhoneWriter {
        paths: paths.clone(),
        config: config.clone(),
    };
    // A project the vault already has from another folder: a new name, unless the person merges on purpose (asked
    // here only; without a terminal choice, always the new name).
    if interactive {
        for clash in ctx.plan(&report, &selected, &writer).await.clashes.into_iter().filter(|c| c.known) {
            let from = clash.existing_origin.as_deref().unwrap_or("a folder without a git origin");
            let q = format!(
                "{} already exists in your vault, from {from}. Merge this folder into it instead of naming it {}? [y/N] ",
                clash.name, clash.chosen
            );
            if matches!(person::ask_tty(&q).as_deref().map(str::trim), Some("y" | "Y" | "yes")) {
                ctx.merge.insert(clash.name);
            }
        }
    }
    let keep_local = args.keep_local;
    let (receipt, path) =
        secrets_import::import(&ctx, &report, &selected, keep_local, &writer, &system, &mut |plan, check| {
            eprintln!("\nMove {} secrets from {} into your vault:", plan.count(), plan.computer);
            for line in plan.lines() {
                eprintln!("  {line}");
            }
            for line in plan
                .clashes
                .iter()
                .map(secrets_import::Clash::line)
                .chain(plan.conflicts.iter().map(secrets_import::Conflict::line))
            {
                eprintln!("  ! {line}");
            }
            eprintln!(
                "Then: {}",
                if keep_local {
                    "keep the local copies too"
                } else {
                    "use them through Reins (local copies go to the trash)"
                }
            );
            eprintln!("Check: {check}. Your phone shows the same four digits; approve only if it does.");
            match person::ask_tty("Move them? [y/N] ").as_deref().map(str::trim) {
                Some("y" | "Y" | "yes") => Ok(()),
                _ => Err("Nothing done.".to_owned()),
            }
        })
        .await?;
    Ok(summary(&receipt, path.as_deref()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets_scan::tests::fixture;

    #[test]
    fn the_picker_ticks_and_unticks_findings() {
        let fx = fixture();
        let report = crate::secrets_scan::scan(&fx.options(), &mut |_| {}).unwrap();
        let mut ticked = secrets_import::select(&report, &[], true).unwrap();
        let lines = picker_lines(&report, &ticked);
        assert_eq!(lines.len(), report.findings.len());
        assert!(lines[0].starts_with("[x]  1. acme-web/.env"), "{}", lines[0]);
        toggle(&report, &mut ticked, 0);
        assert!(picker_lines(&report, &ticked)[0].starts_with("[ ]"));
        toggle(&report, &mut ticked, 0);
        assert!(picker_lines(&report, &ticked)[0].starts_with("[x]"));
        assert_eq!(parse_pick("", 5), Picked::Done);
        assert_eq!(parse_pick(" q ", 5), Picked::Quit);
        assert_eq!(parse_pick("2 4-5", 5), Picked::Toggle(vec![1, 3, 4]));
        assert!(matches!(parse_pick("0", 5), Picked::Bad(_)));
        assert!(matches!(parse_pick("6", 5), Picked::Bad(_)));
        assert!(matches!(parse_pick("x", 5), Picked::Bad(_)));
    }

    #[test]
    fn the_summary_says_what_changed_and_how_to_undo() {
        let receipt = Receipt {
            moved: vec![secrets_import::Moved {
                item: "a".to_owned(),
                finding: "f".to_owned(),
                to: "vault:x/y/A".to_owned(),
                on_disk: false,
            }],
            done: vec![secrets_import::Done {
                source: crate::secrets_scan::Source::Dotenv,
                what: "1 .env file → .env.reins".to_owned(),
                how: "reins run -- <your command>".to_owned(),
                ok: true,
            }],
            notes: vec!["x is in git history".to_owned()],
            ..Receipt::default()
        };
        let lines = summary(&receipt, Some(std::path::Path::new("/s/imports/1.json")));
        assert_eq!(lines[0], "1 secret is in your vault.");
        assert_eq!(lines[1], "✓ 1 .env file → .env.reins");
        assert!(lines.last().unwrap().contains("--undo /s/imports/1.json"));
    }

    #[test]
    fn a_report_from_another_version_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("scan.json");
        std::fs::write(&file, r#"{"version": 99, "started_at": 0, "took": 0, "home": "/", "findings": [], "skipped": [], "skipped_total": 0}"#).unwrap();
        assert!(load_report(&file).unwrap_err().contains("another version"));
        std::fs::write(&file, "{").unwrap();
        assert!(load_report(&file).unwrap_err().contains("not a report"));
    }
}
