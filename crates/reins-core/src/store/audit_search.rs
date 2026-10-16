//! Searching the Activity, on this device only.
//!
//! The index is an FTS5 table in SQLite's `temp` schema of the store's connection. The store's database lives only in
//! memory (`mod.rs`), and `temp` is not part of it: `temp_store=MEMORY` keeps it off disk, and neither the sealed copy
//! on disk (`serialize("main")`) nor a synced snapshot (`snapshot::TABLES`) holds it. So the index is exactly as
//! protected as the opened entries it was built from, and is written nowhere new: no text is ever on disk because of
//! search, sealed or not. It goes with the connection (sign out, lock, another process's newer copy) and is built
//! again, from the sealed entries, by the next search.
//!
//! Each search first brings the index up to date with `audit`: an entry gone, or replaced (its sealed info is sealed
//! anew, with a new nonce, whenever it is written), or whose approval came or went, is indexed again. Writing `temp`
//! changes nothing anyone else sees, so it never seals or writes the store ([`super::protected::StoreLock::temp_only`]).
//!
//! What is indexed is what is shown: each text is cleaned as a line on screen is ([`clean`]: no control or bidi
//! characters, invisible ones as U+FFFD), so a hidden character can neither join two words nor split one unseen, and
//! a snippet's highlights fall on the very text the user reads. The query is cleaned the same way; it is never sent.

use std::collections::{HashMap, HashSet};

use reins_proto::gmail::ToolCall;
use rusqlite::params_from_iter;
use rusqlite::types::Value as Sql;
use serde_json::Value;

use super::Store;
use super::audit::{AuditRecord, sealed_rows};
use crate::CoreError;
use crate::text;
use crate::types::{
    ActivityConnection, ActivityCursor, ActivityDecider, ActivityFacets, ActivityField, ActivityHighlight,
    ActivityOutcome, ActivityQuery, ActivitySnippet, ActivityStamp,
};
use crate::views::ParkedRequest;

/// The most bytes of text indexed for one entry: the fields in their order ([`document`]) until it is full.
pub const MAX_INDEXED: usize = 16 * 1024;
/// The most results on one page.
pub const PAGE_MAX: u32 = 200;
/// The most entries [`Store::activity_stamps`] returns.
pub const STAMPS_MAX: u32 = 50_000;
/// Entries indexed per pass: opened outside the lock, written under it.
const BATCH: usize = 400;
/// The most words of a query, and characters of one word, that count.
const MAX_WORDS: usize = 16;
const MAX_TERM: usize = 64;
/// Characters of a snippet: some before the first match, then up to the whole.
const SNIPPET_BEFORE: usize = 36;
const SNIPPET_CHARS: usize = 140;
/// The most characters of a field a snippet is looked for in.
const SNIPPET_SCAN: usize = 32 * 1024;

const TEMP_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS temp.audit_search_rows (
    seq INTEGER PRIMARY KEY,
    tag BLOB,
    snap INTEGER NOT NULL,
    decided_by TEXT NOT NULL
);
CREATE VIRTUAL TABLE IF NOT EXISTS temp.audit_search USING fts5(
    doc, content='', contentless_delete=1, tokenize='unicode61 remove_diacritics 0'
);
";

/// What tells a stored entry from the one indexed: the start of its sealed info (the nonce, new at each sealing) and
/// whether it has its approval.
const TAG: &str = "substr(a.info, 1, 24)";

/// Text as a line on screen shows it: invisible characters as U+FFFD, no control or bidi characters, one line.
pub(crate) fn clean(s: &str) -> String {
    text::one_line(&text::mark_invisible(s))
}

/// A character of a word, as the index's tokenizer (`unicode61`) reads them.
fn word_char(c: char) -> bool {
    c.is_alphanumeric()
}

/// A query as the index reads it: the FTS5 expression (every word, each a phrase of its parts, the last part a
/// prefix) and the parts, lower case, to set apart in what is shown.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Parsed {
    pub fts: Option<String>,
    pub terms: Vec<String>,
}

pub(crate) fn parse(query: &str) -> Parsed {
    // Hidden characters typed or pasted into the search are dropped, never matched.
    let cleaned: String = query.chars().filter(|c| !text::is_invisible(*c) && !c.is_control()).collect();
    let mut phrases = Vec::new();
    let mut terms: Vec<String> = Vec::new();
    for word in cleaned.split_whitespace().take(MAX_WORDS) {
        let parts: Vec<String> = word
            .split(|c: char| !word_char(c))
            .filter(|p| !p.is_empty())
            .map(|p| p.chars().take(MAX_TERM).collect::<String>().to_lowercase())
            .collect();
        if parts.is_empty() {
            continue;
        }
        // Only letters and digits reach the expression: nothing in it can be FTS5 syntax.
        phrases.push(format!("\"{}\"*", parts.join(" ")));
        for p in parts {
            if !terms.contains(&p) {
                terms.push(p);
            }
        }
    }
    terms.sort_by(|a, b| b.chars().count().cmp(&a.chars().count()).then_with(|| a.cmp(b)));
    Parsed {
        fts: (!phrases.is_empty()).then(|| phrases.join(" AND ")),
        terms,
    }
}

/// The parts of `text` that match `terms` (lower case), as byte ranges: each term at the start of a word, matching
/// as much as the term holds; matches separated only by punctuation (`@`, `.`, `/`, `-`, `_`) join, so an address or
/// a path found by its parts is set apart whole.
pub(crate) fn hits(text: &str, terms: &[String]) -> Vec<(usize, usize)> {
    if terms.is_empty() {
        return Vec::new();
    }
    let chars: Vec<(usize, char)> =
        text.char_indices().map(|(i, c)| (i, c.to_lowercase().next().unwrap_or(c))).collect();
    let terms: Vec<Vec<char>> = terms.iter().map(|t| t.chars().collect()).collect();
    let mut found: Vec<(usize, usize)> = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let starts_word = word_char(chars[i].1) && (i == 0 || !word_char(chars[i - 1].1));
        if starts_word {
            let longest = terms
                .iter()
                .filter(|t| t.len() <= chars.len() - i && t.iter().zip(&chars[i..]).all(|(a, (_, b))| a == b))
                .map(Vec::len)
                .max();
            if let Some(n) = longest {
                let end = chars.get(i + n).map_or(text.len(), |(b, _)| *b);
                found.push((chars[i].0, end));
                i += n;
                continue;
            }
        }
        i += 1;
    }
    let mut joined: Vec<(usize, usize)> = Vec::new();
    for (start, end) in found {
        if let Some(last) = joined.last_mut() {
            let gap = &text[last.1..start];
            if gap.chars().count() <= 2 && gap.chars().all(|c| !word_char(c) && !c.is_whitespace()) {
                last.1 = end;
                continue;
            }
        }
        joined.push((start, end));
    }
    joined
}

/// `text` (cleaned) in pieces, the matching ones set apart.
#[cfg(test)]
fn highlight(text: &str, terms: &[String]) -> Vec<ActivityHighlight> {
    pieces(text, &hits(text, terms), 0, text.len())
}

/// `text[from..to]` in pieces, the parts within `ranges` set apart.
fn pieces(text: &str, ranges: &[(usize, usize)], from: usize, to: usize) -> Vec<ActivityHighlight> {
    let mut spans = Vec::new();
    let mut push = |s: &str, hit: bool| {
        if !s.is_empty() {
            spans.push(ActivityHighlight {
                text: s.to_owned(),
                hit,
            });
        }
    };
    let mut at = from;
    for &(start, end) in ranges {
        let (start, end) = (start.max(from), end.min(to));
        if start >= end {
            continue;
        }
        push(&text[at..start], false);
        push(&text[start..end], true);
        at = end;
    }
    if at < to {
        push(&text[at..to], false);
    }
    spans
}

/// A window of `text` (cleaned, one line) that opens a little before byte `anchor`, with `…` where it was cut. The
/// matches are found in the whole text, then cut to the window: a word cut in two is never taken for one that starts
/// there.
fn window(text: &str, terms: &[String], anchor: usize) -> Vec<ActivityHighlight> {
    let ranges = hits(text, terms);
    let starts: Vec<usize> = text.char_indices().map(|(i, _)| i).collect();
    let n = starts.len();
    let at_char = starts.partition_point(|b| *b < anchor);
    let first = at_char.saturating_sub(SNIPPET_BEFORE);
    let last = (first + SNIPPET_CHARS).min(n);
    let byte = |c: usize| starts.get(c).copied().unwrap_or(text.len());
    let mut from = byte(first);
    let mut to = byte(last);
    // Start and end on whole words where there is one to start or end on.
    if first > 0
        && let Some(space) = text[from..byte(at_char)].find(' ').filter(|i| *i < SNIPPET_BEFORE / 2)
    {
        from += space + 1;
    }
    if last < n
        && let Some(space) =
            text[from..to].rfind(' ').map(|i| from + i).filter(|i| *i > anchor && *i - from > (to - from) * 3 / 4)
    {
        to = space;
    }
    let mut spans = Vec::new();
    if from > 0 {
        spans.push(ActivityHighlight {
            text: "…".to_owned(),
            hit: false,
        });
    }
    spans.extend(pieces(text, &ranges, from, to));
    if to < text.len() {
        spans.push(ActivityHighlight {
            text: "…".to_owned(),
            hit: false,
        });
    }
    spans
}

/// Everything of an entry that search reads, in order: what its row shows first, the big texts last. Each text is
/// cleaned ([`clean`]); empty ones are left out.
pub(crate) fn document(r: &AuditRecord, snapshot: Option<&ParkedRequest>) -> Vec<(ActivityField, String)> {
    let mut doc = Doc::default();
    let label = crate::titles::of_entry(r);
    doc.add(ActivityField::Title, &label.title());
    doc.add(ActivityField::Title, &crate::views::op_title(&r.service, &r.op));
    doc.add(ActivityField::Title, crate::views::service_name(&r.service));
    doc.add(ActivityField::Agent, &r.connection_label);
    if let Some(q) = &r.info.request {
        doc.add(ActivityField::Agent, &q.agent);
    }
    doc.add(ActivityField::Agent, r.account.as_deref().unwrap_or_default());
    doc.add(ActivityField::Title, &r.detail);
    let info = &r.info;
    if let Some(e) = &info.email {
        for to in e.to.iter().chain(&e.cc) {
            doc.add(ActivityField::Address, to);
        }
        doc.add(ActivityField::Subject, &e.subject);
    }
    for m in &info.messages {
        doc.add(ActivityField::Address, &m.from);
        doc.add(ActivityField::Subject, &m.subject);
    }
    for a in &info.accounts {
        doc.add(ActivityField::Address, a);
    }
    doc.add(ActivityField::Body, info.query.as_deref().unwrap_or_default());
    if let Some(q) = &info.request {
        doc.add(ActivityField::Command, &q.full);
        doc.add(ActivityField::File, &q.cwd);
        doc.add(ActivityField::Request, &q.purpose);
        doc.add(ActivityField::Request, &q.why);
    }
    for t in &info.targets {
        doc.add(ActivityField::Request, t);
    }
    if let Some(push) = &info.git {
        push_fields(&mut doc, &push.repo, &push.summary);
    }
    doc.add(ActivityField::Body, info.note.as_deref().unwrap_or_default());
    doc.add(ActivityField::Body, info.grant_summary.as_deref().unwrap_or_default());
    if let Some(e) = &info.email {
        doc.add(ActivityField::Body, &e.body);
    }
    for m in &info.messages {
        doc.add(ActivityField::Body, &m.text);
    }
    if let Some(p) = snapshot {
        snapshot_fields(&mut doc, p);
    }
    doc.fields
}

/// The fields of a document, without repeating a text already in it.
#[derive(Default)]
struct Doc {
    fields: Vec<(ActivityField, String)>,
    seen: HashSet<String>,
}

impl Doc {
    fn add(&mut self, field: ActivityField, raw: &str) {
        let t = clean(raw);
        if !t.is_empty() && self.seen.insert(t.clone()) {
            self.fields.push((field, t));
        }
    }
}

fn push_fields(doc: &mut Doc, repo: &str, summary: &reins_proto::desktop::PushSummary) {
    doc.add(ActivityField::Repo, repo);
    for u in &summary.updates {
        let short = u.name.strip_prefix("refs/heads/").or_else(|| u.name.strip_prefix("refs/tags/")).unwrap_or(&u.name);
        doc.add(ActivityField::Branch, short);
    }
    for c in summary.updates.iter().flat_map(|u| &u.commits) {
        doc.add(ActivityField::Commit, &format!("{} {}", c.sha, c.subject));
    }
    let files =
        || summary.updates.iter().flat_map(|u| u.files.iter().chain(u.commits.iter().flat_map(|c| c.files.iter())));
    for f in files() {
        doc.add(ActivityField::File, &f.path);
    }
    for f in files() {
        if let Some(patch) = &f.patch {
            doc.add(ActivityField::Diff, patch);
        }
    }
}

/// What this device's copy of the approval adds: the call's arguments (an MCP tool and what it was given, a command,
/// a URL, a path), what was found or would be written, and a push's changes as they were shown.
fn snapshot_fields(doc: &mut Doc, p: &ParkedRequest) {
    if let ToolCall::Connector(call) = &p.request.call {
        if crate::connector::git::is_push(call)
            && let Ok(summary) = crate::connector::git::summary_arg(call)
        {
            push_fields(doc, call.str_arg("repo").unwrap_or_default(), &summary);
        }
        doc.add(ActivityField::Request, &call.op);
        for (name, value) in &call.args {
            if name == "summary" {
                continue;
            }
            walk(doc, name, value);
        }
    } else if let Ok(value) = serde_json::to_value(&p.request.call) {
        walk(doc, "", &value);
    }
    for m in &p.messages {
        if let Ok(value) = serde_json::to_value(m) {
            walk(doc, "", &value);
        }
    }
    if let Some(held) = &p.connector {
        if let Some(mcp) = &held.mcp
            && let Ok(value) = serde_json::to_value(mcp)
        {
            walk(doc, "", &value);
        }
        for item in &held.items {
            doc.add(ActivityField::Subject, &item.title);
            doc.add(ActivityField::Body, &item.snippet);
            doc.add(ActivityField::Body, item.body.as_deref().unwrap_or_default());
        }
        if let Some(preview) = &held.preview
            && let Ok(value) = serde_json::to_value(preview)
        {
            walk(doc, "", &value);
        }
    }
}

/// Keys whose values are ids, times or flags: not worth finding.
const SKIPPED_KEYS: &[&str] =
    &["id", "ids", "request_id", "connection_id", "v", "covered", "thread_id", "message_id", "status", "kind"];

/// Every text in `value`, filed by the key it is under.
fn walk(doc: &mut Doc, key: &str, value: &Value) {
    match value {
        Value::String(s) => doc.add(field_of(key, s), s),
        Value::Array(items) => {
            for item in items {
                walk(doc, key, item);
            }
        }
        Value::Object(map) => {
            for (k, v) in map {
                if !SKIPPED_KEYS.contains(&k.as_str()) {
                    walk(doc, k, v);
                }
            }
        }
        _ => {}
    }
}

fn field_of(key: &str, value: &str) -> ActivityField {
    let key = key.to_ascii_lowercase();
    let has = |names: &[&str]| names.iter().any(|n| key == *n || key.ends_with(&format!("_{n}")));
    if has(&["url", "uri", "href", "link", "endpoint"]) || value.starts_with("https://") || value.starts_with("http://")
    {
        ActivityField::Url
    } else if has(&["command", "cmd", "script", "shell", "full", "argv"]) {
        ActivityField::Command
    } else if has(&["path", "paths", "file", "files", "filename", "cwd", "dir", "directory", "folder"]) {
        ActivityField::File
    } else if has(&["patch", "diff"]) {
        ActivityField::Diff
    } else if has(&["sha", "oid", "commit"]) {
        ActivityField::Commit
    } else if has(&["repo", "repository"]) {
        ActivityField::Repo
    } else if has(&["branch", "ref", "base", "head"]) {
        ActivityField::Branch
    } else if has(&["subject", "title"]) {
        ActivityField::Subject
    } else if has(&["from", "to", "cc", "bcc", "email", "address", "recipient", "recipients", "sender"]) {
        ActivityField::Address
    } else if has(&["body", "text", "snippet", "message", "content", "query", "q", "description"]) {
        ActivityField::Body
    } else {
        ActivityField::Request
    }
}

/// The text indexed for a document: its fields, one per line, up to [`MAX_INDEXED`] bytes.
fn indexed(fields: &[(ActivityField, String)]) -> String {
    let mut out = String::new();
    for (_, t) in fields {
        if out.len() + t.len() + 1 > MAX_INDEXED {
            out.push('\n');
            out.push_str(text::truncate_bytes(t, MAX_INDEXED.saturating_sub(out.len())));
            break;
        }
        out.push('\n');
        out.push_str(t);
    }
    out
}

/// Where an entry matched outside what its row shows, set apart; `None` when its row shows every word.
pub(crate) fn snippet(fields: &[(ActivityField, String)], terms: &[String]) -> Option<ActivitySnippet> {
    if terms.is_empty() {
        return None;
    }
    let row = |f: &ActivityField| matches!(f, ActivityField::Title | ActivityField::Agent);
    let shown: String = fields.iter().filter(|(f, _)| row(f)).map(|(_, t)| t.as_str()).collect::<Vec<_>>().join(" \n ");
    let missing: Vec<String> =
        terms.iter().filter(|t| hits(&shown, std::slice::from_ref(t)).is_empty()).cloned().collect();
    if missing.is_empty() {
        return None;
    }
    let scan = |t: &str| text::truncate_bytes(t, SNIPPET_SCAN).to_owned();
    let (field, t) = fields
        .iter()
        .filter(|(f, _)| !row(f))
        .find(|(_, t)| !hits(&scan(t), &missing).is_empty())
        .or_else(|| fields.iter().filter(|(f, _)| !row(f)).find(|(_, t)| !hits(&scan(t), terms).is_empty()))?;
    // The window opens on a word the row lacks; every word searched is set apart in it.
    let t = scan(t);
    let anchor = hits(&t, &missing).first().map_or(0, |(s, _)| *s);
    Some(ActivitySnippet {
        field: *field,
        spans: window(&t, terms, anchor),
    })
}

fn outcome_of(outcome: &str) -> ActivityOutcome {
    match outcome {
        "denied" => ActivityOutcome::Denied,
        "error" => ActivityOutcome::Failed,
        _ => ActivityOutcome::Approved,
    }
}

fn decider_of(decided_by: &str) -> ActivityDecider {
    match decided_by {
        "bypass" => ActivityDecider::Bypass,
        "lockdown" => ActivityDecider::Lockdown,
        "" => ActivityDecider::You,
        // "autopilot", "overnight", and whatever mode a newer version adds.
        _ => ActivityDecider::Autopilot,
    }
}

/// The SQL that keeps an entry for `q`'s filters (on `audit a` joined with the indexed rows `r`), with its values.
fn filters(q: &ActivityQuery, fts: Option<&str>) -> (String, Vec<Sql>) {
    let mut clauses: Vec<String> = Vec::new();
    let mut values: Vec<Sql> = Vec::new();
    let mut one_of = |column: &str, items: Vec<String>, clauses: &mut Vec<String>| {
        if !items.is_empty() {
            clauses.push(format!("{column} IN ({})", vec!["?"; items.len()].join(", ")));
            values.extend(items.into_iter().map(Sql::Text));
        }
    };
    let outcomes: Vec<String> = q
        .outcomes
        .iter()
        .flat_map(|o| match o {
            ActivityOutcome::Approved => &["released", "sent", "granted"][..],
            ActivityOutcome::Denied => &["denied"][..],
            ActivityOutcome::Failed => &["error"][..],
        })
        .map(|s| (*s).to_owned())
        .collect();
    one_of("a.outcome", outcomes, &mut clauses);
    one_of("a.service", q.services.clone(), &mut clauses);
    one_of("a.action", q.actions.clone(), &mut clauses);
    one_of("a.connection_id", q.connections.clone(), &mut clauses);
    if !q.deciders.is_empty() {
        let mut any = Vec::new();
        for d in &q.deciders {
            any.push(match d {
                ActivityDecider::You => "r.decided_by = ''",
                ActivityDecider::Bypass => "r.decided_by = 'bypass'",
                ActivityDecider::Lockdown => "r.decided_by = 'lockdown'",
                ActivityDecider::Autopilot => "r.decided_by NOT IN ('', 'bypass', 'lockdown')",
            });
        }
        clauses.push(format!("({})", any.join(" OR ")));
    }
    if let Some(since) = q.since {
        clauses.push("a.at >= ?".to_owned());
        values.push(Sql::Integer(since));
    }
    if let Some(until) = q.until {
        clauses.push("a.at < ?".to_owned());
        values.push(Sql::Integer(until));
    }
    if let Some(fts) = fts {
        clauses.push("a.seq IN (SELECT rowid FROM temp.audit_search WHERE audit_search MATCH ?)".to_owned());
        values.push(Sql::Text(fts.to_owned()));
    }
    let sql = if clauses.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", clauses.join(" AND "))
    };
    (sql, values)
}

/// An entry and where it matched, before it is shown.
pub(crate) struct Found {
    pub record: AuditRecord,
    pub snippet: Option<ActivitySnippet>,
}

pub(crate) struct FoundPage {
    pub found: Vec<Found>,
    pub next: Option<ActivityCursor>,
    pub total: u32,
    pub terms: Vec<String>,
}

/// An entry as it goes into the index.
struct Indexed {
    seq: i64,
    tag: Option<Vec<u8>>,
    snap: bool,
    decided_by: String,
    doc: String,
}

const JOINED: &str = "FROM audit a JOIN temp.audit_search_rows r ON r.seq = a.seq";

impl Store {
    /// Brings the index up to date with the entries: drops what went or changed, indexes what is new.
    fn refresh_search(&self) -> Result<(), CoreError> {
        let todo: Vec<i64> = {
            let mut conn = self.read()?;
            conn.execute_batch(TEMP_SCHEMA)?;
            let stale: Vec<i64> = conn
                .prepare(&format!(
                    "SELECT r.seq FROM temp.audit_search_rows r LEFT JOIN audit a ON a.seq = r.seq \
                     LEFT JOIN audit_snapshots s ON s.seq = r.seq \
                     WHERE a.seq IS NULL OR r.tag IS NOT {TAG} OR r.snap <> (s.seq IS NOT NULL)"
                ))?
                .query_map([], |row| row.get(0))?
                .collect::<Result<_, _>>()?;
            if !stale.is_empty() {
                let tx = conn.transaction()?;
                for seq in &stale {
                    tx.execute("DELETE FROM temp.audit_search WHERE rowid = ?1", [seq])?;
                    tx.execute("DELETE FROM temp.audit_search_rows WHERE seq = ?1", [seq])?;
                }
                tx.commit()?;
            }
            let todo = conn
                .prepare(
                    "SELECT a.seq FROM audit a LEFT JOIN temp.audit_search_rows r ON r.seq = a.seq \
                     WHERE r.seq IS NULL ORDER BY a.seq DESC",
                )?
                .query_map([], |row| row.get(0))?
                .collect::<Result<_, _>>()?;
            conn.temp_only();
            todo
        };
        for batch in todo.chunks(BATCH) {
            self.index_batch(batch)?;
        }
        Ok(())
    }

    fn index_batch(&self, seqs: &[i64]) -> Result<(), CoreError> {
        let list = seqs.iter().map(i64::to_string).collect::<Vec<_>>().join(",");
        // Read sealed under the lock, with what tells them apart; opened after letting it go.
        let (rows, tags, snaps) = {
            let conn = self.read()?;
            let rows = sealed_rows(&conn, &format!("WHERE seq IN ({list})"), [])?;
            let tags: HashMap<i64, Option<Vec<u8>>> = conn
                .prepare(&format!("SELECT a.seq, {TAG} FROM audit a WHERE a.seq IN ({list})"))?
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<Result<_, _>>()?;
            let snaps: HashMap<i64, Vec<u8>> = conn
                .prepare(&format!("SELECT seq, data FROM audit_snapshots WHERE seq IN ({list})"))?
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<Result<_, _>>()?;
            (rows, tags, snaps)
        };
        let docs: Vec<Indexed> = rows
            .into_iter()
            .map(|row| {
                let seq = row.seq;
                let record = self.open_audit(row);
                let snapshot = snaps.get(&seq).and_then(|b| self.open_snapshot(b));
                let doc = indexed(&document(&record, snapshot.as_ref()));
                Indexed {
                    seq,
                    tag: tags.get(&seq).cloned().flatten(),
                    snap: snaps.contains_key(&seq),
                    decided_by: record.info.decided_by,
                    doc,
                }
            })
            .collect();
        let mut conn = self.read()?;
        // The connection may be a newer copy since (another process wrote the file): its index starts over.
        conn.execute_batch(TEMP_SCHEMA)?;
        let tx = conn.transaction()?;
        for Indexed {
            seq,
            tag,
            snap,
            decided_by,
            doc,
        } in docs
        {
            let added = tx.execute(
                "INSERT OR IGNORE INTO temp.audit_search_rows (seq, tag, snap, decided_by) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![seq, tag, snap, decided_by],
            )?;
            if added == 1 {
                tx.execute("INSERT INTO temp.audit_search (rowid, doc) VALUES (?1, ?2)", rusqlite::params![seq, doc])?;
            }
        }
        tx.commit()?;
        conn.temp_only();
        Ok(())
    }

    /// The entries matching `q`, newest first, `limit` (at most [`PAGE_MAX`]) after `after`, with where each matched.
    pub(crate) fn search_activity(
        &self,
        q: &ActivityQuery,
        after: Option<ActivityCursor>,
        limit: u32,
    ) -> Result<FoundPage, CoreError> {
        self.refresh_search()?;
        let parsed = parse(&q.text);
        let (filter, mut values) = filters(q, parsed.fts.as_deref());
        let limit = limit.clamp(1, PAGE_MAX);
        let (total, page): (u32, Vec<(i64, i64)>) = {
            let conn = self.read()?;
            let total: i64 =
                conn.query_row(&format!("SELECT COUNT(*) {JOINED} {filter}"), params_from_iter(values.iter()), |r| {
                    r.get(0)
                })?;
            let mut sql = format!("SELECT a.seq, a.at {JOINED} {filter}");
            if let Some(c) = after {
                sql.push_str(if filter.is_empty() {
                    " WHERE "
                } else {
                    " AND "
                });
                sql.push_str("(a.at < ? OR (a.at = ? AND a.seq < ?))");
                values.extend([Sql::Integer(c.at), Sql::Integer(c.at), Sql::Integer(c.id)]);
            }
            sql.push_str(" ORDER BY a.at DESC, a.seq DESC LIMIT ?");
            values.push(Sql::Integer(i64::from(limit) + 1));
            let page = conn
                .prepare(&sql)?
                .query_map(params_from_iter(values.iter()), |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<Result<_, _>>()?;
            (u32::try_from(total).unwrap_or(u32::MAX), page)
        };
        let more = page.len() > limit as usize;
        let page = &page[..page.len().min(limit as usize)];
        let next = more
            .then(|| {
                page.last().map(|(id, at)| ActivityCursor {
                    at: *at,
                    id: *id,
                })
            })
            .flatten();
        let list = page.iter().map(|(seq, _)| seq.to_string()).collect::<Vec<_>>().join(",");
        let mut records: HashMap<i64, AuditRecord> = if page.is_empty() {
            HashMap::new()
        } else {
            self.audit_rows(&format!("WHERE seq IN ({list})"), [])?.into_iter().map(|r| (r.seq, r)).collect()
        };
        let mut found = Vec::with_capacity(page.len());
        for (seq, _) in page {
            let Some(record) = records.remove(seq) else {
                continue;
            };
            let snippet = if parsed.terms.is_empty() {
                None
            } else {
                let snapshot = self.audit_snapshot(*seq)?;
                snippet(&document(&record, snapshot.as_ref()), &parsed.terms)
            };
            found.push(Found {
                record,
                snippet,
            });
        }
        Ok(FoundPage {
            found,
            next,
            total,
            terms: parsed.terms,
        })
    }

    /// When each entry matching `q` was logged and how it ended, newest first (at most [`STAMPS_MAX`]): enough to
    /// group them by day and count each day.
    pub(crate) fn activity_stamps(&self, q: &ActivityQuery) -> Result<Vec<ActivityStamp>, CoreError> {
        self.refresh_search()?;
        let parsed = parse(&q.text);
        let (filter, mut values) = filters(q, parsed.fts.as_deref());
        values.push(Sql::Integer(i64::from(STAMPS_MAX)));
        let conn = self.read()?;
        let stamps = conn
            .prepare(&format!(
                "SELECT a.seq, a.at, a.outcome, r.decided_by {JOINED} {filter} ORDER BY a.at DESC, a.seq DESC LIMIT ?"
            ))?
            .query_map(params_from_iter(values.iter()), |r| {
                Ok(ActivityStamp {
                    id: r.get(0)?,
                    at: r.get(1)?,
                    outcome: outcome_of(&r.get::<_, String>(2)?),
                    decider: decider_of(&r.get::<_, String>(3)?),
                })
            })?
            .collect::<Result<_, _>>()?;
        Ok(stamps)
    }

    /// The integrations, kinds and connections the Activity holds.
    pub(crate) fn activity_facets(&self) -> Result<ActivityFacets, CoreError> {
        let conn = self.read()?;
        let column = |sql: &str| -> Result<Vec<String>, CoreError> {
            Ok(conn.prepare(sql)?.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?)
        };
        let services =
            column("SELECT service FROM audit WHERE service <> '' GROUP BY service ORDER BY COUNT(*) DESC, service")?;
        let actions =
            column("SELECT action FROM audit WHERE action <> '' GROUP BY action ORDER BY COUNT(*) DESC, action")?;
        let connections = conn
            .prepare(
                "SELECT connection_id, connection_label FROM audit a WHERE connection_id <> '' AND seq = \
                 (SELECT MAX(seq) FROM audit b WHERE b.connection_id = a.connection_id) ORDER BY seq DESC",
            )?
            .query_map([], |r| {
                Ok(ActivityConnection {
                    id: r.get(0)?,
                    label: text::truncate_chars(&clean(&r.get::<_, String>(1)?), crate::views::MAX_LABEL_CHARS),
                })
            })?
            .collect::<Result<_, _>>()?;
        Ok(ActivityFacets {
            services,
            actions,
            connections,
        })
    }
}

#[cfg(test)]
mod tests;
