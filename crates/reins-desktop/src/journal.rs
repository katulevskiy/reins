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
    /// The phone's id for the request (what "This was wrong" names).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    /// Who decided on the phone when the user did not (Autopilot, a mode). `None`: the user, or not said. Only who
    /// is written to the log: Autopilot's confidence and reasons are kept in memory (see [`Attribution`]).
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "lenient")]
    pub autopilot: Option<Attribution>,
    /// While it waits: what Autopilot would do with it. Never written to the log: an AI of the same user that reads it
    /// would learn how to word its next ask; the app asks the server for it instead.
    #[serde(skip)]
    pub leaning: Option<Lean>,
    /// What the request does, as the consequence engine read it on this computer (Decisions v2): its level, its
    /// certainty and one line per effect. Shown in Activity; it decides nothing yet.
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "lenient")]
    pub effects: Option<reins_policy::effects::Summary>,
    /// The safety net applied to it (Decisions v2 P3): what was kept, and the id `reins net restore` takes.
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "lenient")]
    pub nets: Option<NetRecord>,
    /// What the matrix decided on this computer, or would have decided in shadow (Decisions v2).
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "lenient")]
    pub matrix: Option<crate::consequences::Shadow>,
}

/// A safety net applied to a request, as Activity shows it ("Netted · Restore").
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetRecord {
    /// The plan id (`reins net restore <plan>`).
    pub plan: String,
    pub kinds: Vec<reins_policy::effects::nets::NetKind>,
    /// One line per net ("Keeps 2 deleted paths in the Reins trash for 7 days").
    pub lines: Vec<String>,
    /// Unix seconds until which it can be restored (the trash's keeping time; refs last longer).
    pub until: i64,
    /// The net was applied: it kept something (read from the store when the log is read, never written to it). Until
    /// then Activity says the net was not applied.
    #[serde(skip)]
    pub ran: bool,
    /// Something is still kept for it (read with `ran`): a net that ran with nothing kept any more is gone.
    #[serde(skip)]
    pub kept: bool,
}

/// Who decided a request on the phone without the user, as the phone's answer said (made safe to show). Only `by`
/// and `correctable` are written to the log; the confidence and the reasons stay in memory, like the leaning.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attribution {
    pub by: reins_proto::autopilot::DecidedBy,
    /// Autopilot's confidence, in percent.
    #[serde(skip)]
    pub confidence: Option<u8>,
    #[serde(skip)]
    pub reasons: Vec<reins_proto::autopilot::Reason>,
    /// "This was wrong" may be sent for it.
    #[serde(default)]
    pub correctable: bool,
}

/// What Autopilot would do with a request that waits, and how sure it is (percent).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lean {
    pub leaning: reins_proto::autopilot::Leaning,
    pub confidence: u8,
}

#[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "a fraction from 0 to 1 as a percentage")]
fn percent(fraction: f32) -> u8 {
    (fraction.clamp(0.0, 1.0) * 100.0).round() as u8
}

impl From<reins_proto::autopilot::Decided> for Attribution {
    fn from(d: reins_proto::autopilot::Decided) -> Self {
        let d = d.cleaned();
        Self {
            by: d.by,
            confidence: d.confidence.map(percent),
            reasons: d.reasons,
            correctable: d.correctable,
        }
    }
}

impl From<&reins_proto::autopilot::Suggestion> for Lean {
    fn from(s: &reins_proto::autopilot::Suggestion) -> Self {
        Self {
            leaning: s.leaning,
            confidence: percent(s.confidence),
        }
    }
}

/// A field a newer app may write in a shape this one does not know: read as absent, never losing the entry.
fn lenient<'de, D: serde::Deserializer<'de>, T: serde::de::DeserializeOwned>(d: D) -> Result<Option<T>, D::Error> {
    let value = <serde_json::Value as Deserialize>::deserialize(d)?;
    Ok(serde_json::from_value(value).ok())
}

/// `text` on one line, as `what`, `source` and `service` are kept: each run of whitespace one space.
#[must_use]
pub fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn cut(text: &str, max: usize, single: bool) -> String {
    let text = if single {
        one_line(text)
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
            request_id: None,
            autopilot: None,
            leaning: None,
            effects: None,
            nets: None,
            matrix: None,
        }
    }

    /// What the phone said about it so far ([`crate::told::Heard`]): its request id, where Autopilot leans while it
    /// waits, and who decided once it is answered. `true` when that changed what the log keeps of it.
    pub fn hear(&mut self, heard: &crate::told::Heard) -> bool {
        // What the log keeps: the request id, and who decided.
        let kept = |e: &Self| (e.request_id.clone(), e.autopilot.as_ref().map(|a| (a.by, a.correctable)));
        let before = kept(self);
        if heard.request_id.is_some() {
            self.request_id.clone_from(&heard.request_id);
        }
        self.leaning = heard.leaning;
        if heard.decided.is_some() {
            self.autopilot.clone_from(&heard.decided);
            self.leaning = None;
        }
        before != kept(self)
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

    /// What the request does (see [`Entry::effects`]).
    #[must_use]
    pub fn effects(mut self, effects: Option<reins_policy::effects::Summary>) -> Self {
        self.effects = effects;
        self
    }

    /// The safety net applied to it.
    #[must_use]
    pub fn nets(mut self, nets: Option<NetRecord>) -> Self {
        self.nets = nets;
        self
    }

    /// What the matrix said (see [`Entry::matrix`]).
    #[must_use]
    pub fn matrix(mut self, matrix: Option<crate::consequences::Shadow>) -> Self {
        self.matrix = matrix;
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
    /// Where "Check your phone" goes.
    desk: Desk,
}

/// Where "Check your phone" goes: this computer's notifications, or (tests) a list of what would show.
#[derive(Clone, Debug, Default)]
enum Desk {
    #[default]
    System,
    #[cfg(test)]
    Record {
        delay: std::time::Duration,
        events: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    },
}

/// A "Check your phone" shown for a request; [`Notice::withdraw`] takes it down once the request is answered, where
/// the platform allows.
#[derive(Debug)]
pub struct Notice(NoticeKind);

#[derive(Debug)]
enum NoticeKind {
    System(crate::notify::Shown),
    #[cfg(test)]
    Record(std::sync::Arc<std::sync::Mutex<Vec<String>>>, String),
}

impl Notice {
    /// Takes it down (the request was answered); returns at once.
    pub fn withdraw(self) {
        match self.0 {
            NoticeKind::System(shown) => shown.withdraw(),
            #[cfg(test)]
            NoticeKind::Record(events, what) => events.lock().unwrap().push(format!("withdrawn: {what}")),
        }
    }
}

impl Journal {
    #[must_use]
    pub fn new(paths: &Paths) -> Self {
        Self {
            paths: Some(paths.clone()),
            desk: Desk::System,
        }
    }

    /// A journal that writes nothing and notifies nobody.
    #[must_use]
    pub fn off() -> Self {
        Self {
            paths: None,
            desk: Desk::System,
        }
    }

    /// A journal that writes nothing and lists the notifications it would show (`shown: …`, `withdrawn: …`), after
    /// `delay`.
    #[cfg(test)]
    pub(crate) fn recording(delay: std::time::Duration) -> (Self, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        let events = std::sync::Arc::default();
        let journal = Self {
            paths: None,
            desk: Desk::Record {
                delay,
                events: std::sync::Arc::clone(&events),
            },
        };
        (journal, events)
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

    /// Keeps a push's commits and diff text under its entry's `id` ([`crate::pushes`]) when it is asked; a failure is
    /// logged.
    pub fn keep_push(&self, id: &str, push: &crate::pushes::Push) {
        let Some(paths) = &self.paths else {
            return;
        };
        if let Err(e) = crate::pushes::keep(&paths.state_dir, id, push) {
            log::warn!("kept pushes: {e}");
        }
    }

    /// Drops the push kept under `id`: it was not approved.
    pub fn forget_push(&self, id: &str) {
        if let Some(paths) = &self.paths {
            crate::pushes::forget(&paths.state_dir, id);
        }
    }

    /// "Check your phone" on the desktop, as `[notify]` says, now. `None` when nothing showed.
    #[must_use]
    pub fn notify_waiting(&self, what: &str) -> Option<Notice> {
        match &self.desk {
            Desk::System => {
                let paths = self.paths.as_ref()?;
                let config = crate::config::Config::load(paths).map(|c| c.notify).unwrap_or_default();
                crate::notify::phone_waiting(&config, &paths.state_dir, what).map(|s| Notice(NoticeKind::System(s)))
            }
            #[cfg(test)]
            Desk::Record {
                events,
                ..
            } => {
                events.lock().unwrap().push(format!("shown: {what}"));
                Some(Notice(NoticeKind::Record(std::sync::Arc::clone(events), what.to_owned())))
            }
        }
    }

    /// How long a request waits for the phone before [`Journal::notify_waiting`] (`[notify] delay_ms`): one the
    /// phone's mode or a standing permission decides meanwhile needs nobody.
    #[must_use]
    pub fn notify_delay(&self) -> std::time::Duration {
        match &self.desk {
            Desk::System => self
                .paths
                .as_ref()
                .and_then(|paths| crate::config::Config::load(paths).ok())
                .map(|c| c.notify)
                .unwrap_or_default()
                .delay(),
            #[cfg(test)]
            Desk::Record {
                delay,
                ..
            } => *delay,
        }
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
    let mut entries = parse(&text, limit, stale_after, crate::now_unix());
    // Whether each net was applied: the netted command really ran through the net (or the copies were taken).
    let store = crate::nets::Store::open(paths);
    for n in entries.iter_mut().filter_map(|e| e.nets.as_mut()) {
        n.ran = store.ran(&n.plan);
        n.kept = !store.items(&n.plan).is_empty() || !store.refs(&n.plan).is_empty();
    }
    entries
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
    /// Of those, the ones the phone decided without the user (Autopilot, a mode), and of these the ones it denied.
    pub auto: usize,
    pub auto_denied: usize,
}

impl Tally {
    #[must_use]
    pub fn of<'a>(entries: impl IntoIterator<Item = &'a Entry>) -> Self {
        let mut t = Self::default();
        for e in entries {
            if e.autopilot.is_some() && !e.outcome.open() {
                t.auto += 1;
                if !matches!(e.outcome, Outcome::Approved | Outcome::Allowed) {
                    t.auto_denied += 1;
                }
            }
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
    fn effects_are_kept_and_older_lines_still_read() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        paths.ensure().unwrap();
        let j = Journal::new(&paths);
        let ctx = reins_policy::effects::Context::new(
            "/home/me/Documents/dev/app",
            reins_policy::effects::Workspace::standard("/home/me"),
        );
        let summary = reins_policy::effects::analyze_command("rm -rf target", &ctx).summary();
        let shadow = crate::consequences::Shadow {
            outcome: reins_policy::matrix::Outcome::Auto,
            zone: reins_policy::context::Zone::Mine,
            level: reins_policy::effects::Level::L0,
            line: "Auto · Mine · Undoable".to_owned(),
            snapshot: "ab".repeat(32),
            decided: false,
        };
        let e = Entry::new(Kind::Command, "rm").effects(Some(summary.clone())).matrix(Some(shadow.clone()));
        j.record(&e);
        let read = read(&paths, 10, i64::MAX);
        assert_eq!(read[0].effects.as_ref(), Some(&summary));
        assert_eq!(read[0].matrix.as_ref(), Some(&shadow));
        // A line from before effects, and one whose effects a newer app wrote in another shape.
        let old: Entry =
            serde_json::from_str(r#"{"id":"a","at":1,"kind":"ask","what":"x","outcome":"approved"}"#).unwrap();
        assert!(old.effects.is_none());
        let newer: Entry = serde_json::from_str(
            r#"{"id":"a","at":1,"kind":"ask","what":"x","outcome":"approved","effects":{"level":"L9"}}"#,
        )
        .unwrap();
        assert!(newer.effects.is_none());
        assert!(old.matrix.is_none());
        let odd: Entry = serde_json::from_str(
            r#"{"id":"a","at":1,"kind":"ask","what":"x","outcome":"approved","matrix":{"outcome":"maybe"}}"#,
        )
        .unwrap();
        assert!(odd.matrix.is_none(), "a newer shape is dropped, not an error");
    }

    #[test]
    fn the_off_journal_writes_nothing() {
        Journal::off().record(&Entry::new(Kind::Ask, "x"));
        assert!(Journal::off().notify_waiting("x").is_none());
    }
}
