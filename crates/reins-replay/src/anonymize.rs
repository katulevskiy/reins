//! Turns the desktop app's activity log (`~/.local/state/reins/activity.jsonl`) into corpus cases, anonymized.
//!
//! Run on the owner's computer only (`reins-replay import`); the log never leaves it. What is kept is what the engine
//! needs to read the request faithfully: the command's structure, its programs, flags and paths inside
//! `~/Documents/dev`. What is removed: email addresses, IP and MAC addresses, UUIDs, long hexadecimal ids, tokens and
//! key-like strings, the owner's GitHub name, and the private terms the labels file lists (host names, project ids).

use std::collections::BTreeMap;

use regex::Regex;
use reins_proto::desktop::{PushSummary, RefChange, RefUpdate, ZERO_OID};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::corpus::{Case, Kind, Label, Original};

/// Hand labels: overrides by case id, and the terms to remove.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Labels {
    /// The owner's GitHub name, replaced by `owner_alias`.
    #[serde(default)]
    pub owner: String,
    #[serde(default)]
    pub owner_alias: String,
    /// Private words (host names, project ids, names): each replaced by `term-N` (or its given alias).
    #[serde(default)]
    pub private_terms: BTreeMap<String, String>,
    /// Ignore records at or after this time (Unix seconds): the log keeps growing.
    #[serde(default)]
    pub until: Option<i64>,
    #[serde(default)]
    pub overrides: BTreeMap<String, Override>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Override {
    pub label: Label,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub note: Option<String>,
    /// Leave the case out of the corpus (a duplicate, a test of Reins itself).
    #[serde(default)]
    pub skip: bool,
}

/// Removes private data from `text`.
pub struct Scrubber {
    rules: Vec<(Regex, Replace)>,
}

enum Replace {
    With(String),
    By(fn(&str) -> String),
}

/// An IP address, unless it is this computer's own.
fn ip(m: &str) -> String {
    if m.starts_with("127.") || m == "0.0.0.0" {
        m.to_owned()
    } else {
        "192.0.2.1".to_owned()
    }
}

/// Long strings of mixed letters and digits with no separators a path or a word has: keys and ids.
fn key_like(m: &str) -> String {
    let digits = m.chars().filter(char::is_ascii_digit).count();
    let upper = m.chars().filter(char::is_ascii_uppercase).count();
    let lower = m.chars().filter(char::is_ascii_lowercase).count();
    if digits >= 3 && upper >= 2 && lower >= 2 {
        "REDACTED".to_owned()
    } else {
        m.to_owned()
    }
}

impl Scrubber {
    /// # Panics
    /// Never: the patterns are fixed, and the private terms are escaped.
    #[must_use]
    pub fn new(labels: &Labels) -> Self {
        let mut rules: Vec<(Regex, Replace)> = Vec::new();
        let mut add = |p: &str, to: Replace| rules.push((Regex::new(p).expect("a fixed pattern"), to));
        for (term, alias) in &labels.private_terms {
            add(&format!(r"(?i)\b{}\b", regex::escape(term)), Replace::With(alias.clone()));
        }
        if !labels.owner.is_empty() {
            add(&format!(r"(?i){}", regex::escape(&labels.owner)), Replace::With(labels.owner_alias.clone()));
        }
        add(
            r"\b(gh[pousr]_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,}|sk-[A-Za-z0-9_-]{16,}|xox[abprs]-[A-Za-z0-9-]{10,}|AKIA[0-9A-Z]{16}|AIza[0-9A-Za-z_-]{30,})\b",
            Replace::With("REDACTED".to_owned()),
        );
        add(
            r"\b[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}\b",
            Replace::With("00000000-0000-4000-8000-000000000000".to_owned()),
        );
        add(r"\b([0-9a-fA-F]{2}:){5}[0-9a-fA-F]{2}\b", Replace::With("02:00:00:00:00:01".to_owned()));
        add(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}", Replace::With("person@example.com".to_owned()));
        add(r"\b(?:(?:25[0-5]|2[0-4]\d|1?\d?\d)\.){3}(?:25[0-5]|2[0-4]\d|1?\d?\d)\b", Replace::By(ip));
        add(r"\b[0-9a-f]{16,}\b", Replace::With("abcdef0123456789".to_owned()));
        add(r"\b[A-Za-z0-9]{20,}\b", Replace::By(key_like));
        Self {
            rules,
        }
    }

    #[must_use]
    pub fn scrub(&self, text: &str) -> String {
        let mut s = text.to_owned();
        for (re, to) in &self.rules {
            s = match to {
                Replace::With(t) => re.replace_all(&s, t.as_str()).into_owned(),
                Replace::By(f) => re.replace_all(&s, |c: &regex::Captures<'_>| f(&c[0])).into_owned(),
            };
        }
        s
    }
}

/// One record of the activity log (lines of one id merged, the last value winning).
#[derive(Clone, Debug, Default, Deserialize)]
struct Record {
    id: String,
    #[serde(default)]
    at: i64,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    service: Option<String>,
    #[serde(default)]
    what: String,
    #[serde(default)]
    detail: Option<String>,
    #[serde(default)]
    outcome: String,
}

/// The lines a hook writes before the command (`Rule:`, `Matched:`, `Group:`, `Branch:`, `Default branch:`).
fn strip_header(detail: &str) -> &str {
    let mut rest = detail;
    for prefix in ["Rule: ", "Matched: ", "Group: ", "Branch: ", "Default branch: "] {
        if let Some(r) = rest.strip_prefix(prefix) {
            rest = r.split_once('\n').map_or("", |(_, r)| r);
        }
    }
    rest.strip_prefix('\n').unwrap_or(rest)
}

/// The command (or files) and the folder of a hook's detail.
fn body_and_cwd(detail: &str) -> (String, Option<String>) {
    let rest = strip_header(detail);
    // Older details put the rule after the folder.
    let rest = rest.split("\nRule: ").next().unwrap_or(rest);
    match rest.rsplit_once("\n\nIn: ") {
        Some((body, cwd)) => (body.to_owned(), Some(cwd.lines().next().unwrap_or_default().trim().to_owned())),
        None => (rest.to_owned(), None),
    }
}

/// A command cut short by the log (2,000 characters, then `…`): an open heredoc is closed with its delimiter and an
/// open quote's last line dropped, so the engine reads what was there. `None` when it was not cut.
#[must_use]
pub fn repair(command: &str) -> Option<String> {
    let cut = command.strip_suffix('…')?;
    let mut text = cut.to_owned();
    for _ in 0..8 {
        let parsed = reins_policy::shell::ast::parse(&text);
        if parsed.complete {
            return Some(text);
        }
        if let Some(p) = parsed.problems.iter().find(|p| p.starts_with("the heredoc `")) {
            let delim = p.trim_start_matches("the heredoc `").split('`').next().unwrap_or("EOF").to_owned();
            if !text.ends_with('\n') {
                // The last line is cut: drop it.
                text = text.rsplit_once('\n').map_or_else(String::new, |(a, _)| a.to_owned());
                text.push('\n');
            }
            text.push_str(&delim);
            text.push('\n');
            continue;
        }
        // An unclosed quote or substitution: drop the last command (up to a newline, `;`, `&&`, `||` or `|`).
        let trimmed = text.trim_end();
        let cut_at = ["\n", "; ", " && ", " || ", " | "].iter().filter_map(|sep| trimmed.rfind(sep)).max();
        match cut_at {
            Some(at) if at > 0 => text = trimmed[..at].to_owned(),
            _ => return Some(String::new()),
        }
    }
    Some(text)
}

/// Arguments the log does not keep (it keeps only their names): plausible values for the owner's own repositories.
fn synthetic_args(names: &[&str], owner: &str) -> Value {
    let mut m = serde_json::Map::new();
    for n in names {
        let v = match *n {
            "repo" => json!(format!("{owner}/reins")),
            "owner" => json!(owner),
            "head" | "branch" => json!("feat/change"),
            "base" => json!("main"),
            "number" | "pull_number" | "issue_number" | "run_id" | "job_id" => json!(1),
            "title" => json!("A change"),
            "body" => json!("What it does."),
            "workflow" | "workflow_id" => json!("ci.yml"),
            "ref" => json!("main"),
            "request_id" => json!("r1"),
            "merge_method" => json!("squash"),
            "state" => json!("open"),
            "draft" => json!(false),
            _ => json!("x"),
        };
        m.insert((*n).to_owned(), v);
    }
    Value::Object(m)
}

/// `branch (N commits)` items of a push's line.
fn push_summary(what: &str, detail: &str) -> PushSummary {
    let refs = what.split_once(": ").map_or("", |(_, r)| r);
    let rewritten = detail
        .lines()
        .find_map(|l| l.split("rewritten ").nth(1))
        .and_then(|n| n.trim().parse::<usize>().ok())
        .unwrap_or(0);
    let deleted = detail
        .lines()
        .find_map(|l| l.split("deleted ").nth(1))
        .and_then(|n| n.split_whitespace().next()?.parse::<usize>().ok())
        .unwrap_or(0);
    let mut updates = Vec::new();
    for item in refs.split(", ") {
        let (name, commits) = match item.split_once(" (") {
            Some((n, c)) => (n.trim(), c.split_whitespace().next().and_then(|x| x.parse::<u32>().ok()).unwrap_or(1)),
            None => (item.trim(), 0),
        };
        if name.is_empty() {
            continue;
        }
        let change = if commits == 0 {
            RefChange::Create
        } else {
            RefChange::Update
        };
        updates.push(RefUpdate {
            name: format!("refs/heads/{name}"),
            change,
            old: if change == RefChange::Create {
                ZERO_OID.to_owned()
            } else {
                "a".repeat(40)
            },
            new: "b".repeat(40),
            fast_forward: if change == RefChange::Create {
                None
            } else {
                Some(true)
            },
            commit_count: commits,
            commits: Vec::new(),
            files_changed: 0,
            files: Vec::new(),
            additions: None,
            deletions: None,
        });
    }
    // The log keeps counts only: the first `rewritten` updates are taken as the force-pushed ones.
    for u in updates.iter_mut().filter(|u| u.change == RefChange::Update).take(rewritten) {
        u.fast_forward = Some(false);
    }
    for u in updates.iter_mut().rev().take(deleted) {
        u.change = RefChange::Delete;
        u.new = ZERO_OID.to_owned();
    }
    PushSummary {
        updates,
        default_branch: Some("main".to_owned()),
        ..PushSummary::default()
    }
}

/// The corpus cases of an activity log.
///
/// # Errors
/// A line that is not JSON.
pub fn import(log: &str, labels: &Labels) -> Result<Vec<Case>, String> {
    let scrub = Scrubber::new(labels);
    let mut merged: BTreeMap<String, (usize, Value)> = BTreeMap::new();
    for (n, line) in log.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let v: Value = serde_json::from_str(line).map_err(|e| format!("line {}: {e}", n + 1))?;
        let id = v.get("id").and_then(Value::as_str).unwrap_or_default().to_owned();
        let entry = merged.entry(id).or_insert((n, json!({})));
        if let (Some(dst), Some(src)) = (entry.1.as_object_mut(), v.as_object()) {
            for (k, val) in src {
                dst.insert(k.clone(), val.clone());
            }
        }
    }
    let mut records: Vec<(usize, Record)> =
        merged.into_values().filter_map(|(n, v)| serde_json::from_value::<Record>(v).ok().map(|r| (n, r))).collect();
    records.sort_by_key(|(n, _)| *n);
    let owner = if labels.owner_alias.is_empty() {
        "owner".to_owned()
    } else {
        labels.owner_alias.clone()
    };
    let mut cases = Vec::new();
    for (_, r) in records {
        if labels.until.is_some_and(|u| r.at >= u) {
            continue;
        }
        let ov = labels.overrides.get(&r.id);
        if ov.is_some_and(|o| o.skip) {
            continue;
        }
        let detail = r.detail.clone().unwrap_or_default();
        let mut tags = Vec::new();
        let mut case = Case {
            id: r.id.clone(),
            at: r.at,
            kind: Kind::Ask,
            agent: r.source.as_deref().map(|s| scrub.scrub(s)),
            command: None,
            cwd: None,
            files: None,
            write: None,
            repo: None,
            push: None,
            tool: None,
            args: None,
            original: Original {
                outcome: r.outcome.clone(),
                guard_rule: r
                    .service
                    .as_deref()
                    .filter(|_| matches!(r.kind.as_str(), "command" | "file"))
                    .map(|s| scrub.scrub(s)),
            },
            label: Label::Routine,
            tags: Vec::new(),
            note: None,
        };
        match r.kind.as_str() {
            "command" => {
                case.kind = Kind::Command;
                let (body, cwd) = body_and_cwd(&detail);
                let mut command = body.strip_prefix("Command:\n").map_or_else(
                    || r.what.split_once(" wants to run: ").map_or(body.clone(), |(_, c)| c.to_owned()),
                    str::to_owned,
                );
                if let Some(fixed) = repair(&command) {
                    command = fixed;
                    tags.push("repaired".to_owned());
                }
                case.command = Some(scrub.scrub(&command));
                case.cwd = cwd.map(|c| scrub.scrub(&c));
            }
            "file" => {
                case.kind = Kind::File;
                let (body, cwd) = body_and_cwd(&detail);
                let files: Vec<String> = body
                    .strip_prefix("Files:\n")
                    .unwrap_or(&body)
                    .lines()
                    .filter(|l| !l.trim().is_empty())
                    .map(|l| scrub.scrub(l.trim()))
                    .collect();
                case.files = Some(files);
                case.write = Some(r.what.contains(" wants to change "));
                case.cwd = cwd.map(|c| scrub.scrub(&c));
            }
            "git" => {
                case.repo = r.service.as_deref().map(|s| scrub.scrub(s));
                if r.what.starts_with("push") {
                    case.kind = Kind::GitPush;
                    case.push = Some(push_summary(&scrub.scrub(&r.what), &detail));
                } else {
                    case.kind = Kind::GitRead;
                }
            }
            "mcp" => {
                case.kind = Kind::Mcp;
                let tool = r.what.rsplit(": ").next().unwrap_or_default().to_owned();
                let names: Vec<&str> =
                    detail.strip_prefix("Arguments: ").map(|a| a.split(", ").collect()).unwrap_or_default();
                case.args = Some(synthetic_args(&names, &owner));
                case.tool = Some(tool);
                tags.push("synthetic_args".to_owned());
            }
            _ => {
                case.kind = Kind::Ask;
                case.label = Label::NeedsHuman;
                tags.push("explicit_question".to_owned());
                case.note = Some(scrub.scrub(&r.what));
            }
        }
        match r.outcome.as_str() {
            "timed_out" => tags.push("timed_out".to_owned()),
            "denied" => tags.push("owner_denied".to_owned()),
            _ => {}
        }
        if let Some(o) = ov {
            case.label = o.label;
            tags.extend(o.tags.iter().cloned());
            case.note.clone_from(&o.note);
        }
        // The guard's false alarms: what it flagged as keys, `.env`, dropped data or wiped disks, labelled routine.
        let flagged = case.original.guard_rule.as_deref().is_some_and(|g| {
            g.starts_with("file:")
                || g.contains("drop table")
                || g.contains("mkfs")
                || g.contains("drop database")
                || g.contains("truncate")
        });
        if flagged && case.label == Label::Routine && !tags.iter().any(|t| t == "known_fp") {
            tags.push("known_fp".to_owned());
        }
        tags.sort();
        tags.dedup();
        case.tags = tags;
        cases.push(case);
    }
    Ok(cases)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anonymizer_strips_private_tokens() {
        let labels = Labels {
            owner: "katname".to_owned(),
            owner_alias: "dkat".to_owned(),
            private_terms: [("secrethost".to_owned(), "host-1".to_owned())].into_iter().collect(),
            ..Labels::default()
        };
        let s = Scrubber::new(&labels);
        let out = s.scrub(
            "ssh debian@203.0.113.7 && git push github.com/katname/reins && grep secrethost ~/.ssh/config; infisical run --projectId=e21d41de-d3f4-41bf-accb-b4c2df9f164d; echo ghp_abcdefghijklmnopqrstuvwxyz0123; tts cgSgspJ2msm6clMCkdW9 36:d8:a1:d4:f8:c9",
        );
        for bad in ["203.0.113.7", "katname", "secrethost", "e21d41de", "ghp_", "cgSgspJ2msm6clMCkdW9", "36:d8"] {
            assert!(!out.contains(bad), "{bad} in {out}");
        }
        assert!(out.contains("dkat/reins") && out.contains("host-1") && out.contains("~/.ssh/config"), "{out}");
        assert_eq!(
            s.scrub("cargo build --target-dir target-198 assembleFullReleaseAndroidTest"),
            "cargo build --target-dir target-198 assembleFullReleaseAndroidTest"
        );
        assert_eq!(s.scrub("curl http://127.0.0.1:8080/alive"), "curl http://127.0.0.1:8080/alive");
    }

    #[test]
    fn repair_closes_cut_heredocs() {
        assert_eq!(repair("echo a"), None);
        let fixed = repair("cd /a && python3 - <<'EOF'\np='x'\ns=open(p).re…").unwrap();
        assert!(reins_policy::shell::ast::parse(&fixed).complete, "{fixed}");
        assert!(fixed.ends_with("EOF\n"));
        let fixed = repair("ls && echo 'half…").unwrap();
        assert!(reins_policy::shell::ast::parse(&fixed).complete, "{fixed}");
    }

    #[test]
    fn records_become_cases() {
        let log = [
            r#"{"id":"a","at":1,"kind":"command","source":"Claude Code","service":"command:rm -r","what":"x","detail":"Rule: Recursive deletes · rm -r\nMatched: rm -rf target\nGroup: deletes (destructive)\n\nCommand:\nrm -rf target\n\nIn: /home/user/Documents/dev/app","decider":"phone","outcome":"waiting"}"#,
            r#"{"id":"a","at":1,"kind":"command","outcome":"approved"}"#,
            r#"{"id":"b","at":2,"kind":"git","source":"git","service":"github.com/katname/reins","what":"push to github.com/katname/reins: feat/x (2 commits)","outcome":"approved"}"#,
            r#"{"id":"c","at":3,"kind":"mcp","source":"Claude Code","what":"Claude Code: github_pr_create","detail":"Arguments: repo, head, base","outcome":"denied"}"#,
            r#"{"id":"d","at":4,"kind":"command","service":"file:.env","what":"x","detail":"Command:\ncat .env\n\nIn: /home/user/Documents/dev/app","outcome":"approved"}"#,
        ]
        .join("\n");
        let labels = Labels {
            owner: "katname".to_owned(),
            owner_alias: "dkat".to_owned(),
            ..Labels::default()
        };
        let cases = import(&log, &labels).unwrap();
        assert_eq!(cases.len(), 4);
        assert_eq!(
            (cases[0].kind, cases[0].command.as_deref(), cases[0].cwd.as_deref()),
            (Kind::Command, Some("rm -rf target"), Some("/home/user/Documents/dev/app"))
        );
        assert_eq!(cases[0].original.outcome, "approved");
        assert_eq!(cases[1].repo.as_deref(), Some("github.com/dkat/reins"));
        assert_eq!(cases[1].push.as_ref().unwrap().updates[0].commit_count, 2);
        assert_eq!(cases[2].args.as_ref().unwrap()["repo"], "dkat/reins");
        assert!(cases[2].tagged("owner_denied"));
        assert!(cases[3].tagged("known_fp"));
    }
}
