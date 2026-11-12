//! Exports the decision history (the Activity list) as a CSV or JSON file's contents.
//!
//! Built from [`ActivityEntry`], what the Activity screen shows, so a file matches the screen: the same titles, the
//! same outcome grouping ([`ActivityOutcome`]: approved, denied or failed) and the same deciders. Entries are written in
//! the order given (the engine passes them newest first). Everything is computed on the phone; nothing is sent anywhere.
//!
//! # Privacy
//!
//! This is a decision history, not a content archive. An export never holds message or email bodies, recipients,
//! subjects, search queries, the entry's `detail` line, the complete request (`request.full`), the approval snapshot,
//! a push's files or changes, raw authorization traces, or a secret or credential of any kind. Only who asked, what
//! kind of thing, and how it was decided are exported. Entries are exported through [`Entry`] and [`CSV_COLUMNS`] only,
//! so a field of [`ActivityEntry`] added later stays out of a file until it is added here on purpose.
//!
//! # JSON (version 1)
//!
//! ```json
//! {
//!   "format": "reins.decision-history",
//!   "version": 1,
//!   "exported_at": "2026-11-12T09:30:00Z",
//!   "scope": "this-device",
//!   "entries": [ { ... } ]
//! }
//! ```
//!
//! Times are RFC 3339 in UTC, to the second. Each entry has (absent values are left out, never `null`):
//!
//! - `id`: the entry's stable row number on this device; `at`: when it was logged.
//! - `service`, `account`, `action` (kind: "read", "send", "grant", ...), `title` (the operation in a few words, never
//!   its subject), `label` (`provider` and `action` badge), `count`.
//! - `outcome`: the stored outcome ("released", "sent", "granted", "denied", "error", ...); `decision`: "approved",
//!   "denied" or "failed" (`failed` also covers an execution that may or may not have happened).
//! - `decided_by`: "you" (the user, or a grant the user gave) or the stored actor ("autopilot", "rule", "circle",
//!   "bypass", "lockdown", ...).
//! - `connection_id`, `connection` (its label).
//! - `request`: `agent`, `cwd`, `why`, `purpose`, `asked_at`.
//! - `autopilot`: `mode`, `suggested` ("approve", "deny" or "ask"), `confidence` (0 to 1), `risk` (a word), `reason`.
//! - `note` (the reviewer's note), `grant_summary`, `hold` (`state`, `release_at`), `execution` (`state`, `started_at`).
//!
//! Readers must ignore fields they do not know; `version` changes only when a field changes meaning or goes away.
//!
//! # CSV
//!
//! RFC 4180: CRLF line endings (also after the last row), a header row, fields with `,`, `"`, CR or LF quoted, quotes
//! doubled. Columns: [`CSV_COLUMNS`]. A text cell starting with `=`, `+`, `-`, `@`, tab or CR gets a leading `'` so a
//! spreadsheet does not run it as a formula; the numeric columns (`count`, `autopilot_confidence`) are never changed.

use serde::Serialize;

use crate::autopilot::AutopilotNote;
use crate::autopilot::types::{AutopilotMode, Verdict};
use crate::send_hold::HoldState;
use crate::text::iso_utc as rfc3339;
use crate::types::{ActivityEntry, ActivityExport, ActivityExportFormat, ActivityOutcome, ExecutionState};

/// The JSON document's `format`.
pub const JSON_FORMAT: &str = "reins.decision-history";
/// The JSON document's `version`.
pub const JSON_VERSION: u32 = 1;

/// The CSV header, in order.
pub const CSV_COLUMNS: [&str; 16] = [
    "time_utc",
    "decision",
    "decided_by",
    "service",
    "account",
    "action",
    "summary",
    "connection",
    "agent",
    "purpose",
    "why",
    "note",
    "count",
    "execution",
    "autopilot_suggestion",
    "autopilot_confidence",
];

/// The file for `entries` (kept in their order) as of `now` (Unix seconds).
#[must_use]
pub fn export(format: ActivityExportFormat, entries: &[ActivityEntry], now: i64) -> ActivityExport {
    let (extension, mime_type, contents) = match format {
        ActivityExportFormat::Csv => ("csv", "text/csv", csv(entries)),
        ActivityExportFormat::Json => ("json", "application/json", json(entries, now)),
    };
    ActivityExport {
        file_name: format!("reins-decisions-{}.{extension}", rfc3339(now).get(..10).unwrap_or_default()),
        mime_type: mime_type.to_owned(),
        contents,
        entries: u32::try_from(entries.len()).unwrap_or(u32::MAX),
    }
}

#[derive(Serialize)]
struct Document<'a> {
    format: &'static str,
    version: u32,
    exported_at: String,
    scope: &'static str,
    entries: Vec<Entry<'a>>,
}

#[derive(Serialize)]
struct Entry<'a> {
    id: i64,
    at: String,
    service: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    account: Option<&'a str>,
    action: &'a str,
    #[serde(skip_serializing_if = "str::is_empty")]
    title: &'a str,
    outcome: &'a str,
    decision: &'static str,
    decided_by: &'a str,
    connection_id: &'a str,
    #[serde(skip_serializing_if = "str::is_empty")]
    connection: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<Label<'a>>,
    count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    request: Option<Request<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    autopilot: Option<Autopilot<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    grant_summary: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    hold: Option<Hold>,
    #[serde(skip_serializing_if = "Option::is_none")]
    execution: Option<Execution>,
}

#[derive(Serialize)]
struct Label<'a> {
    provider: &'a str,
    action: &'a str,
}

#[derive(Serialize)]
struct Request<'a> {
    #[serde(skip_serializing_if = "str::is_empty")]
    agent: &'a str,
    #[serde(skip_serializing_if = "str::is_empty")]
    cwd: &'a str,
    #[serde(skip_serializing_if = "str::is_empty")]
    why: &'a str,
    #[serde(skip_serializing_if = "str::is_empty")]
    purpose: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    asked_at: Option<String>,
}

#[derive(Serialize)]
struct Autopilot<'a> {
    mode: &'static str,
    suggested: &'static str,
    confidence: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    risk: Option<&'static str>,
    #[serde(skip_serializing_if = "str::is_empty")]
    reason: &'a str,
}

#[derive(Serialize)]
struct Hold {
    state: &'static str,
    release_at: String,
}

#[derive(Serialize)]
struct Execution {
    state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    started_at: Option<String>,
}

fn entry(e: &ActivityEntry) -> Entry<'_> {
    let request = e.request.as_ref().filter(|r| {
        !(r.agent.is_empty() && r.cwd.is_empty() && r.why.is_empty() && r.purpose.is_empty() && r.asked_at <= 0)
    });
    Entry {
        id: e.id,
        at: rfc3339(e.at),
        service: &e.service,
        account: e.account.as_deref().filter(|a| !a.is_empty()),
        action: &e.action,
        title: title(e),
        outcome: &e.outcome,
        decision: decision(e),
        decided_by: decided_by(e),
        connection_id: &e.connection_id,
        connection: &e.connection_label,
        label: e.label.as_ref().map(|l| Label {
            provider: &l.provider,
            action: &l.action,
        }),
        count: e.count,
        request: request.map(|r| Request {
            agent: &r.agent,
            cwd: &r.cwd,
            why: &r.why,
            purpose: &r.purpose,
            asked_at: (r.asked_at > 0).then(|| rfc3339(r.asked_at)),
        }),
        autopilot: e.autopilot.as_ref().map(autopilot),
        note: e.info.note.as_deref().filter(|n| !n.is_empty()),
        grant_summary: e.info.grant_summary.as_deref().filter(|n| !n.is_empty()),
        hold: e.hold.as_ref().map(|h| Hold {
            state: match h.state {
                HoldState::Waiting => "waiting",
                HoldState::Sending => "sending",
                HoldState::Late => "late",
            },
            release_at: rfc3339(h.release_at),
        }),
        execution: e.execution.as_ref().map(|x| Execution {
            state: execution_state(x.state),
            started_at: (x.started_at > 0).then(|| rfc3339(x.started_at)),
        }),
    }
}

fn autopilot(a: &AutopilotNote) -> Autopilot<'_> {
    Autopilot {
        mode: match a.mode {
            AutopilotMode::Manual => "manual",
            AutopilotMode::Auto => "auto",
            AutopilotMode::Overnight => "overnight",
            AutopilotMode::Bypass => "bypass",
            AutopilotMode::Lockdown => "lockdown",
        },
        suggested: verdict(a.suggested),
        confidence: confidence(a.confidence),
        risk: a.risk.as_ref().map(|r| r.level.word()),
        reason: &a.reason,
    }
}

fn verdict(v: Verdict) -> &'static str {
    match v {
        Verdict::Approve => "approve",
        Verdict::Deny => "deny",
        Verdict::Ask => "ask",
    }
}

fn execution_state(s: ExecutionState) -> &'static str {
    match s {
        ExecutionState::Pending => "pending",
        ExecutionState::Done => "done",
        ExecutionState::Unknown => "unknown",
        ExecutionState::Withheld => "withheld",
    }
}

/// Confidence rounded to two places (an `f32` widened would print as 0.8999999761581421).
fn confidence(c: f32) -> f64 {
    (f64::from(c.clamp(0.0, 1.0)) * 100.0).round() / 100.0
}

/// The operation in a few words; for an entry without one, the label's words before its subject (never the subject).
fn title(e: &ActivityEntry) -> &str {
    if e.op_title.is_empty() {
        e.label.as_ref().map_or("", |l| l.lead.as_str())
    } else {
        &e.op_title
    }
}

/// "approved", "denied" or "failed": the grouping of [`ActivityOutcome`] (an unknown execution counts as failed).
fn decision(e: &ActivityEntry) -> &'static str {
    let doubt = e.execution.as_ref().is_some_and(|x| x.state == ExecutionState::Unknown);
    match crate::store::activity_outcome_of(&e.outcome, doubt) {
        ActivityOutcome::Approved => "approved",
        ActivityOutcome::Denied => "denied",
        ActivityOutcome::Failed => "failed",
    }
}

fn decided_by(e: &ActivityEntry) -> &str {
    if e.decided_by.is_empty() {
        "you"
    } else {
        &e.decided_by
    }
}

fn json(entries: &[ActivityEntry], now: i64) -> String {
    let document = Document {
        format: JSON_FORMAT,
        version: JSON_VERSION,
        exported_at: rfc3339(now),
        scope: "this-device",
        entries: entries.iter().map(entry).collect(),
    };
    // Plain data of strings and numbers: serialising cannot fail.
    let mut out = serde_json::to_string_pretty(&document).unwrap_or_default();
    out.push('\n');
    out
}

fn csv(entries: &[ActivityEntry]) -> String {
    let mut out = String::new();
    push_row(&mut out, CSV_COLUMNS.iter().map(|c| (*c).to_owned()));
    for e in entries {
        let x = entry(e);
        let request = x.request.as_ref();
        let (suggested, conf) = x
            .autopilot
            .as_ref()
            .map_or((String::new(), String::new()), |a| (guard(a.suggested), format!("{:.2}", a.confidence)));
        push_row(
            &mut out,
            [
                x.at.clone(),
                guard(x.decision),
                guard(x.decided_by),
                guard(x.service),
                guard(x.account.unwrap_or_default()),
                guard(x.action),
                guard(x.title),
                guard(x.connection),
                guard(request.map_or("", |r| r.agent)),
                guard(request.map_or("", |r| r.purpose)),
                guard(request.map_or("", |r| r.why)),
                guard(x.note.unwrap_or_default()),
                x.count.to_string(),
                guard(x.execution.as_ref().map_or("", |s| s.state)),
                suggested,
                conf,
            ]
            .into_iter(),
        );
    }
    out
}

/// Defuses a spreadsheet formula: a text cell that starts with `=`, `+`, `-`, `@`, tab or CR gets a leading `'`.
fn guard(cell: &str) -> String {
    if cell.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        format!("'{cell}")
    } else {
        cell.to_owned()
    }
}

/// RFC 4180: quote a field holding a comma, quote, CR or LF; double its quotes.
fn quote(field: &str) -> String {
    if field.contains([',', '"', '\r', '\n']) {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_owned()
    }
}

fn push_row(out: &mut String, fields: impl Iterator<Item = String>) {
    let mut first = true;
    for field in fields {
        if !first {
            out.push(',');
        }
        first = false;
        out.push_str(&quote(&field));
    }
    out.push_str("\r\n");
}

#[cfg(test)]
mod tests;
