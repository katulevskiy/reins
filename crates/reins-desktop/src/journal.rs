//! This computer's activity log: every request Reins decided here, who decided it and how it ended (approved on the
//! phone, denied, timed out, allowed or blocked by the settings), in `activity.jsonl` in the state directory (0600).
//! The daemon, `reins hook`, `reins ask`, `reins run` and `reins mcp` all append to it; the desktop app shows it.
//!
//! One JSON object per line. A request that waits is written twice under the same id: first `waiting`, then its end;
//! readers keep the last line of each id. Above [`MAX_BYTES`] the file is moved to `activity.jsonl.old` (one
//! generation is kept). Nothing secret is written: the summary and detail are what the phone shows anyway, never a
//! token or a released secret.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::Paths;

/// Above this size the log is moved aside.
pub const MAX_BYTES: u64 = 2 << 20;
const MAX_WHAT: usize = 300;
const MAX_DETAIL: usize = 2_000;

/// What kind of request.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// git through the proxy: clone, fetch, push.
    Git,
    /// A signature or the key list from the SSH agent.
    Ssh,
    /// A key for the API proxy.
    Api,
    /// Secrets for `reins run`.
    Secrets,
    /// A harness hook before a command.
    Command,
    /// A harness hook before reading or changing a file.
    File,
    /// `reins ask`.
    Ask,
    /// A tool call through `reins mcp`.
    Mcp,
}

impl Kind {
    pub const ALL: [Self; 8] =
        [Self::Git, Self::Ssh, Self::Api, Self::Secrets, Self::Command, Self::File, Self::Ask, Self::Mcp];

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Git => "git",
            Self::Ssh => "SSH",
            Self::Api => "API key",
            Self::Secrets => "Secrets",
            Self::Command => "Command",
            Self::File => "File",
            Self::Ask => "Question",
            Self::Mcp => "MCP tool",
        }
    }
}

/// How a request stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// Waiting for an answer.
    Waiting,
    /// Someone said yes (on the phone, or on this computer in local mode), or a standing grant covered it.
    Approved,
    Denied,
    /// Nobody answered in time.
    TimedOut,
    /// Could not be asked (not paired, the server unreachable, a malformed answer).
    Failed,
    /// Let through by this computer's settings without asking.
    Allowed,
    /// Refused by this computer's settings without asking.
    Blocked,
}

impl Outcome {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Waiting => "Waiting",
            Self::Approved => "Approved",
            Self::Denied => "Denied",
            Self::TimedOut => "Timed out",
            Self::Failed => "Failed",
            Self::Allowed => "Allowed",
            Self::Blocked => "Blocked",
        }
    }

    /// Still open.
    #[must_use]
    pub fn open(self) -> bool {
        self == Self::Waiting
    }
}

/// Who answers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decider {
    /// The phone, through the Reins server (Reins 2FA).
    #[default]
    Phone,
    /// The person at this computer (local mode).
    Local,
    /// This computer's settings, without asking anyone.
    Settings,
}

/// One request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    /// Unix seconds when it was asked.
    pub at: i64,
    pub kind: Kind,
    /// Who asked: the harness ("Claude Code"), or the program (`git`, `ssh`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// The service or host it is for (`github.com`, `openai`, `gmail`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service: Option<String>,
    /// One line: what would happen.
    pub what: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default)]
    pub decider: Decider,
    pub outcome: Outcome,
    /// Why it was denied or failed, as it was said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Unix seconds when it ended.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<i64>,
}

fn cut(text: &str, max: usize, one_line: bool) -> String {
    let text = if one_line {
        text.split_whitespace().collect::<Vec<_>>().join(" ")
    } else {
        text.chars().filter(|c| !c.is_control() || matches!(c, '\n' | '\t')).collect()
    };
    if text.chars().count() <= max {
        return text;
    }
    let mut s: String = text.chars().take(max - 1).collect();
    s.push('…');
    s
}

impl Entry {
    /// A new request, waiting; `what` is cut to one line.
    #[must_use]
    pub fn new(kind: Kind, what: &str) -> Self {
        Self {
            id: crate::server::random_token(9),
            at: crate::now_unix(),
            kind,
            source: None,
            service: None,
            what: cut(what, MAX_WHAT, true),
            detail: None,
            decider: Decider::Phone,
            outcome: Outcome::Waiting,
            reason: None,
            ended_at: None,
        }
    }

    #[must_use]
    pub fn source(mut self, source: Option<&str>) -> Self {
        self.source = source.filter(|s| !s.trim().is_empty()).map(|s| cut(s, 60, true));
        self
    }

    #[must_use]
    pub fn service(mut self, service: Option<&str>) -> Self {
        self.service = service.filter(|s| !s.trim().is_empty()).map(|s| cut(s, 120, true));
        self
    }

    #[must_use]
    pub fn detail(mut self, detail: Option<&str>) -> Self {
        self.detail = detail.filter(|d| !d.trim().is_empty()).map(|d| cut(d.trim_end(), MAX_DETAIL, false));
        self
    }

    #[must_use]
    pub fn decider(mut self, decider: Decider) -> Self {
        self.decider = decider;
        self
    }

    /// Ends it.
    pub fn end(&mut self, outcome: Outcome, reason: Option<&str>) {
        self.outcome = outcome;
        self.reason = reason.filter(|r| !r.trim().is_empty()).map(|r| cut(r, MAX_WHAT, true));
        self.ended_at = Some(crate::now_unix());
    }
}

/// Where the log is, and the notification settings for "check your phone".
#[derive(Clone, Debug)]
pub struct Journal {
    /// `None`: writes nothing (tests, and code that has no state directory).
    paths: Option<Paths>,
}

impl Journal {
    #[must_use]
    pub fn new(paths: &Paths) -> Self {
        Self {
            paths: Some(paths.clone()),
        }
    }

    /// A journal that writes nothing and notifies nobody.
    #[must_use]
    pub fn off() -> Self {
        Self {
            paths: None,
        }
    }

    #[must_use]
    pub fn file(paths: &Paths) -> PathBuf {
        paths.state_dir.join("activity.jsonl")
    }

    /// Appends `entry` (a failure is logged, never returned: the log must not stop a request).
    pub fn record(&self, entry: &Entry) {
        let Some(paths) = &self.paths else {
            return;
        };
        if let Err(e) = append(&Self::file(paths), entry) {
            log::warn!("activity log: {e}");
        }
    }

    /// "Check your phone" on the desktop, as `[notify]` says.
    pub fn notify_waiting(&self, what: &str) {
        let Some(paths) = &self.paths else {
            return;
        };
        let config = crate::config::Config::load(paths).map(|c| c.notify).unwrap_or_default();
        crate::notify::phone_waiting(&config, &paths.state_dir, what);
    }
}

fn append(file: &Path, entry: &Entry) -> std::io::Result<()> {
    if std::fs::metadata(file).is_ok_and(|m| m.len() > MAX_BYTES) {
        let mut old = file.as_os_str().to_owned();
        old.push(".old");
        std::fs::rename(file, old)?;
    }
    let mut line = serde_json::to_vec(entry).map_err(std::io::Error::other)?;
    line.push(b'\n');
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    // One write per line: appends from several processes do not interleave.
    options.open(file)?.write_all(&line)
}

/// The requests in the log, newest first, at most `limit`. A `waiting` entry older than `stale_after` seconds whose
/// end was never written (the process asking was killed) reads as failed.
#[must_use]
pub fn read(paths: &Paths, limit: usize, stale_after: i64) -> Vec<Entry> {
    let file = Journal::file(paths);
    let mut old = file.as_os_str().to_owned();
    old.push(".old");
    let mut text = std::fs::read_to_string(PathBuf::from(old)).unwrap_or_default();
    text.push('\n');
    text.push_str(&std::fs::read_to_string(&file).unwrap_or_default());
    parse(&text, limit, stale_after, crate::now_unix())
}

fn parse(text: &str, limit: usize, stale_after: i64, now: i64) -> Vec<Entry> {
    let mut order: Vec<String> = Vec::new();
    let mut by_id: std::collections::HashMap<String, Entry> = std::collections::HashMap::new();
    for line in text.lines() {
        let Ok(e) = serde_json::from_str::<Entry>(line) else {
            continue;
        };
        if !by_id.contains_key(&e.id) {
            order.push(e.id.clone());
        }
        by_id.insert(e.id.clone(), e);
    }
    let mut entries: Vec<Entry> = order.into_iter().filter_map(|id| by_id.remove(&id)).collect();
    for e in &mut entries {
        if e.outcome == Outcome::Waiting && now - e.at > stale_after {
            e.outcome = Outcome::Failed;
            e.reason = Some("The request ended without an answer being recorded.".to_owned());
        }
    }
    entries.sort_by_key(|e| std::cmp::Reverse(e.at));
    entries.truncate(limit);
    entries
}

/// Counts per outcome, for the overview.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tally {
    pub waiting: usize,
    pub approved: usize,
    pub denied: usize,
    pub timed_out: usize,
    pub failed: usize,
    pub allowed: usize,
    pub blocked: usize,
}

impl Tally {
    #[must_use]
    pub fn of<'a>(entries: impl IntoIterator<Item = &'a Entry>) -> Self {
        let mut t = Self::default();
        for e in entries {
            *match e.outcome {
                Outcome::Waiting => &mut t.waiting,
                Outcome::Approved => &mut t.approved,
                Outcome::Denied => &mut t.denied,
                Outcome::TimedOut => &mut t.timed_out,
                Outcome::Failed => &mut t.failed,
                Outcome::Allowed => &mut t.allowed,
                Outcome::Blocked => &mut t.blocked,
            } += 1;
        }
        t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waiting_then_ended_reads_as_one_entry_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        paths.ensure().unwrap();
        let j = Journal::new(&paths);
        let mut a = Entry::new(Kind::Command, "Claude Code wants to run:\n rm -rf build")
            .source(Some("Claude Code"))
            .detail(Some("Command:\nrm -rf build\n"));
        a.at -= 10;
        j.record(&a);
        let b = Entry::new(Kind::Git, "push to me/app main").service(Some("github.com"));
        j.record(&b);
        a.end(Outcome::Denied, Some("Denied on your phone."));
        j.record(&a);
        let read = read(&paths, 10, 3_600);
        assert_eq!(read.len(), 2);
        assert_eq!(read[0].id, b.id);
        assert_eq!(read[1].what, "Claude Code wants to run: rm -rf build");
        assert_eq!(read[1].outcome, Outcome::Denied);
        assert_eq!(read[1].detail.as_deref(), Some("Command:\nrm -rf build"));
        assert_eq!(Tally::of(&read).denied, 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(Journal::file(&paths)).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn stale_waits_read_as_failed_and_damage_is_skipped() {
        let mut e = Entry::new(Kind::Ask, "Deploy?");
        e.at = 100;
        let text = format!("{}\nnot json\n", serde_json::to_string(&e).unwrap());
        let read = parse(&text, 10, 60, 1_000);
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].outcome, Outcome::Failed);
        assert_eq!(parse(&text, 10, 60, 120)[0].outcome, Outcome::Waiting);
    }

    #[test]
    fn a_full_log_moves_aside_and_is_still_read() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        paths.ensure().unwrap();
        let first = Entry::new(Kind::Ask, "first");
        Journal::new(&paths).record(&first);
        let file = Journal::file(&paths);
        // Write access (Windows refuses `set_len` on an append-only handle), closed before the log moves aside.
        let big = std::fs::OpenOptions::new().write(true).open(&file).unwrap();
        big.set_len(MAX_BYTES + 1).unwrap();
        drop(big);
        // Padding of NULs: one unreadable line, skipped.
        let second = Entry::new(Kind::Ask, "second");
        Journal::new(&paths).record(&second);
        assert!(std::fs::metadata(&file).unwrap().len() < 1_000);
        let read = read(&paths, 10, 3_600);
        assert!(read.iter().any(|e| e.id == first.id) && read.iter().any(|e| e.id == second.id));
    }

    #[test]
    fn the_off_journal_writes_nothing() {
        Journal::off().record(&Entry::new(Kind::Ask, "x"));
        Journal::off().notify_waiting("x");
    }
}
