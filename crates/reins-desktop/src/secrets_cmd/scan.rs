//! `reins secrets scan [PATH...] [--json]`: what the scanner found, as a table of names, places and risks (never a
//! value), or as the report's JSON for `reins secrets import --from-scan`. A person's command only
//! ([`super::person`]).

use std::fmt::Write as _;
use std::io::IsTerminal as _;
use std::path::PathBuf;

use crate::config::{Config, Paths};
use crate::secrets_scan::{self, Caller, Group, Risk, ScanOptions, ScanReport, SkipReason, Stage, StageState};

use super::person::{self, Needs, Terminal};

#[derive(Clone, Debug, clap::Args)]
pub struct ScanArgs {
    /// Only these files and folders (default: the project folders, git repositories and the usual credential files).
    pub paths: Vec<PathBuf>,
    /// The report as JSON (for `reins secrets import --from-scan`): names, places and masked previews only.
    #[arg(long)]
    pub json: bool,
}

/// The question asked on the terminal before a scan.
pub const QUESTION: &str =
    "Scan this computer for secrets? Only names, places and masked previews are shown, and nothing leaves it.";

/// Runs the scan after the person confirmed it.
pub fn run(args: &ScanArgs, paths: &Paths, config: &Config) -> Result<Vec<String>, String> {
    let home = crate::config::home_dir()?;
    person::check(
        "reins secrets scan",
        QUESTION,
        &Caller::from_env(),
        Terminal::now(),
        Needs {
            stdout: !args.json,
        },
        &mut person::ask_tty,
    )?;
    let mut opts = ScanOptions::new(home.clone());
    opts.caller = Caller::Person;
    opts.exposure = secrets_scan::Exposure::detect(paths, config, &home);
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    opts.paths = args
        .paths
        .iter()
        .map(|p| {
            if p.is_absolute() {
                p.clone()
            } else {
                cwd.join(p)
            }
        })
        .collect();
    let shown = std::io::stderr().is_terminal();
    let mut last: Option<Stage> = None;
    let report = secrets_scan::scan(&opts, &mut |p| {
        let running = p.stages.iter().find(|(_, s)| **s == StageState::Running).map(|(st, _)| *st);
        if shown && running.is_some() && running != last {
            last = running;
            if let Some(stage) = running {
                eprintln!("Looking at {} ({})…", stage.label(), stage.places());
            }
        }
    })?;
    if args.json {
        return serde_json::to_string_pretty(&report).map(|j| vec![j]).map_err(|e| e.to_string());
    }
    Ok(table(&report))
}

fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

/// The report as lines to read: a headline, each group's findings with their place, size and risks, what was
/// skipped, and what to do next.
#[must_use]
pub fn table(report: &ScanReport) -> Vec<String> {
    let mut lines = Vec::new();
    if report.findings.is_empty() {
        lines.push("No secrets found.".to_owned());
    } else {
        let places = report.findings.len();
        let mut head =
            format!("{} in {}", plural(report.secrets(), "secret", "secrets"), plural(places, "place", "places"));
        for (risk, words) in [
            (Risk::ReadableByAgents, "readable by AI agents"),
            (Risk::InGitHistory, "in git history"),
            (Risk::NoPassphrase, "without a passphrase"),
        ] {
            let n = report.with_risk(risk);
            if n > 0 {
                let _ = write!(head, " · {n} {words}");
            }
        }
        lines.push(head);
        let title_w = report.findings.iter().map(|f| f.title.chars().count()).max().unwrap_or(0).min(36);
        let place_w = report.findings.iter().map(|f| f.place.chars().count()).max().unwrap_or(0).min(40);
        for group in Group::ALL {
            let in_group: Vec<_> = report.findings.iter().filter(|f| f.group == group).collect();
            if in_group.is_empty() {
                continue;
            }
            lines.push(String::new());
            lines.push(group.label().to_uppercase());
            for f in in_group {
                let risks: Vec<&str> = f.risks.iter().map(|r| r.label()).collect();
                let tick = if f.suggested {
                    "•"
                } else {
                    " "
                };
                lines.push(format!(
                    "{tick} {:title_w$}  {:place_w$}  {:16}  {}  [{}]",
                    f.title,
                    f.place,
                    f.summary,
                    risks.join(", "),
                    f.id
                ));
            }
        }
        if report.with_risk(Risk::InGitHistory) > 0 {
            lines.push(String::new());
            lines.push(
                "Files in git history stay published after they move: rotate the keys in them (new keys at each \
                 service)."
                    .to_owned(),
            );
        }
    }
    let mut notes = Vec::new();
    if report.skipped_total > 0 {
        let count = |r: SkipReason| report.skipped.iter().filter(|s| s.reason == r).count();
        let parts: Vec<String> = [
            (SkipReason::TooBig, "over 1 MB"),
            (SkipReason::Binary, "binary"),
            (SkipReason::OtherUser, "another user's"),
            (SkipReason::SymlinkOutside, "links out of the home folder"),
            (SkipReason::Unreadable, "unreadable"),
            (SkipReason::Helper, "kept by a credential helper"),
        ]
        .iter()
        .filter_map(|(r, words)| {
            let n = count(*r);
            (n > 0).then(|| format!("{n} {words}"))
        })
        .collect();
        notes.push(format!("Skipped {}: {}.", report.skipped_total, parts.join(", ")));
    }
    if let Some(b) = &report.budget {
        notes.push(format!(
            "Stopped early at its {} limit, at {}: name the folders to scan (reins secrets scan ~/dev).",
            b.limit, b.at
        ));
    }
    if !notes.is_empty() {
        lines.push(String::new());
        lines.extend(notes);
    }
    if !report.findings.is_empty() {
        lines.push(String::new());
        lines.push(
            "• pre-ticked. Move them into your vault: reins secrets import (or the Reins app: Keys & secrets)."
                .to_owned(),
        );
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets_scan::tests::{fake, fixture, leaks};

    #[test]
    fn the_table_names_places_and_risks_never_values() {
        let fx = fixture();
        let mut opts = fx.options();
        opts.exposure.agents = vec!["Claude Code".to_owned()];
        let report = secrets_scan::scan(&opts, &mut |_| {}).unwrap();
        let lines = table(&report);
        let text = lines.join("\n");
        assert!(lines[0].contains("secrets in") && lines[0].contains("readable by AI agents"), "{}", lines[0]);
        for want in
            ["PROJECT .ENV FILES", "KEYS AND TOKENS", "CLOUD AND SHELL", "acme-web/.env", "~/dev/acme-web", "6 keys"]
        {
            assert!(text.contains(want), "{want} missing:\n{text}");
        }
        assert!(text.contains("Skipped") && text.contains("over 1 MB"), "{text}");
        for secret in &fx.secrets {
            assert!(!leaks(&text, secret), "a value in the table");
        }
        assert!(!leaks(&text, &fake("sk-proj-", 48)));
    }

    #[test]
    fn an_empty_report_says_so() {
        assert_eq!(table(&ScanReport::default()), ["No secrets found."]);
    }
}
