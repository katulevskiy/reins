//! Deferred decisions on this computer: tickets (`rt_…`) for what the reviewer will decide later.
//!
//! - A hook's question or a `reins ask` that the reviewer did not answer in time, moved with Decide later, or that was
//!   asked to be deferred, becomes a [`LocalTicket`]. The agent is told it is parked and carries on.
//! - Once approved, the phone's sealed answer is collected ([`refresh`]) into a one-time [`Pass`]: the exact command
//!   (by [`pass_digest`] over the raw call) goes through once, within the hour after approval.
//! - A push the reviewer will decide later is kept whole under `parked/` ([`park_push`]). Once approved, [`refresh`]
//!   collects the credential and pushes it, but only if every ref is still where it was when git pushed
//!   (force-with-lease); else the ticket is stale and nothing is pushed.
//! - Each harness session hears what changed for its tickets once ([`TicketStore::news_for`]).
//!
//! Everything lives in `state_dir/tickets.json` (0600). The hooks, `reins ask`, `reins wait` and the daemon all change
//! it, so every change goes through [`TicketStore::update`], under a file lock. Nothing secret is kept: a pass is a
//! digest and an expiry, a parked push keeps its pack and where it goes, never a token.

use std::collections::HashMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use bytes::Bytes;
use reins_proto::desktop::{AskAnswer, CredentialGrant, SEALED_FIELD};
use reins_proto::relay::{RelayOutcome, ToolResult};
use reins_proto::ticket::{AGENT_RUN_VALIDITY_SECS, DesktopTicket, TicketState};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use zeroize::Zeroizing;

use crate::auth::Credential;
use crate::config::{Config, Paths, write_private};
use crate::git::pktline::{Pkt, Reader};
use crate::identity::Identity;
use crate::server::LinkError;
use crate::server::client::{CallStatus, DesktopClient};

/// The largest push kept for a later review; a larger one is refused as it waits.
pub const MAX_PARKED_BODY: u64 = 256 << 20;
/// How often the daemon looks at its tickets.
pub const REFRESH_EVERY: Duration = Duration::from_secs(30);
/// How long a hook spends looking at tickets when the daemon does not run.
pub const HOOK_REFRESH: Duration = Duration::from_secs(3);
/// Closed tickets are forgotten after a week, and the file never holds more than this many.
const MAX_TICKETS: usize = 500;
const KEEP_CLOSED_SECS: i64 = 7 * 86_400;
const MAX_PASSES: usize = 200;
/// The longest summary kept (and said to the agent).
const MAX_SUMMARY: usize = 200;
/// The largest answer of the git host read back (the ref list before a push, its result).
const MAX_GIT_ANSWER: usize = 32 << 20;
/// The all-zero object id (SHA-1 and SHA-256 alike): a ref that does not exist.
fn is_zero(oid: &str) -> bool {
    !oid.is_empty() && oid.bytes().all(|b| b == b'0')
}

/// What a ticket is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TicketKind {
    /// A harness hook's question: the agent re-runs the command once approved.
    Hook,
    /// `reins ask`: the same question asked again answers yes once approved.
    Ask,
    /// A git push kept here: pushed by this app once approved.
    Push,
}

impl TicketKind {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Hook => "hook",
            Self::Ask => "ask",
            Self::Push => "push",
        }
    }
}

/// Where a parked push goes, and how git asked for its result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParkedPush {
    /// The repository's git URL on the host (`https://github.com/owner/name.git`), redirects already followed.
    pub upstream: String,
    /// `owner/name`: what the phone's credential must be for.
    pub repo: String,
    /// The body is gzip-compressed (git's `Content-Encoding`).
    #[serde(default)]
    pub gzip: bool,
    /// git asked for a side-band, so the host's result comes in one.
    #[serde(default)]
    pub sideband: bool,
    /// git asked for a report of each ref (`report-status`).
    #[serde(default)]
    pub report: bool,
}

/// One ticket this computer waits on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalTicket {
    pub ticket: String,
    pub kind: TicketKind,
    /// The harness session that asked (hooks); `None`: told to the next session that calls a hook.
    #[serde(default)]
    pub session: Option<String>,
    /// What it is about, one line (`git push --force origin fix/ci`).
    pub summary: String,
    /// What an approval must be for: the call's digest ([`pass_digest`]) or the push's
    /// ([`reins_proto::desktop::push_digest`]).
    pub digest: String,
    /// The question's nonce, which the phone's sealed answer echoes.
    pub nonce: String,
    pub created_at: i64,
    /// When an answer is expected (Unix seconds; 0: not said).
    pub expected_by: i64,
    /// Where it stands, as [`TicketState::as_str`]. Hooks and asks are `approved` once their pass is granted; pushes
    /// `done`, `failed` or `stale` once tried.
    pub state: String,
    /// The state its session was last told about.
    #[serde(default)]
    pub told: Option<String>,
    /// The repository of a push (`github.com/owner/name`).
    #[serde(default)]
    pub repo: Option<String>,
    /// A push's ref updates: `(old, new, ref)`.
    #[serde(default)]
    pub refs: Vec<(String, String, String)>,
    /// The reviewer's note with a denial, or why it failed.
    #[serde(default)]
    pub note: Option<String>,
    /// What changed, for a stale ticket.
    #[serde(default)]
    pub stale: Option<String>,
    #[serde(default)]
    pub push: Option<ParkedPush>,
}

impl LocalTicket {
    #[must_use]
    pub fn state(&self) -> Option<TicketState> {
        TicketState::parse(&self.state)
    }

    /// Still waiting for the server: a review, or (a push) the approval to push. A stale push is over here: its body is
    /// gone, git must push again.
    #[must_use]
    pub fn waits(&self) -> bool {
        self.state().is_some_and(|s| s.is_open() && !(self.kind == TicketKind::Push && s == TicketState::Stale))
    }

    /// What its session is told when it changed, if anything.
    #[must_use]
    pub fn news(&self) -> Option<String> {
        let head = format!("Reins: {} ({})", self.ticket, self.summary);
        let note = |n: &Option<String>| n.as_deref().map(str::trim).filter(|n| !n.is_empty()).map(str::to_owned);
        Some(match (self.state()?, self.kind) {
            (TicketState::Approved, TicketKind::Hook | TicketKind::Ask) => {
                format!("{head} was approved — re-run it now.")
            }
            (TicketState::Done, TicketKind::Push) => format!("{head} was approved and pushed."),
            (TicketState::Denied, _) => match note(&self.note) {
                Some(n) => format!("{head} was denied: {n}"),
                None => format!("{head} was denied."),
            },
            (TicketState::Expired, _) => format!("{head} expired, not approved."),
            (TicketState::Stale, _) => {
                format!("{head} is stale: {}.", note(&self.stale).unwrap_or_else(|| "it changed".to_owned()))
            }
            (TicketState::Cancelled, _) => format!("{head} was cancelled."),
            (TicketState::Failed, _) => match note(&self.note) {
                Some(n) => format!("{head} failed: {n}"),
                None => format!("{head} failed."),
            },
            _ => return None,
        })
    }

    fn overlaps(&self, other: &Self) -> bool {
        self.repo == other.repo && self.refs.iter().any(|(_, _, a)| other.refs.iter().any(|(_, _, b)| a == b))
    }
}

/// A one-time pass: the command with this digest goes through once, until `expires_at`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pass {
    pub ticket: String,
    pub digest: String,
    pub expires_at: i64,
    /// The harness session a hook's pass is for: only a hook call of that session uses it. `None`: a `reins ask`
    /// pass, which no hook can use.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
}

/// `state_dir/tickets.json`.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct TicketStore {
    #[serde(skip)]
    file: PathBuf,
    #[serde(default)]
    tickets: Vec<LocalTicket>,
    #[serde(default)]
    passes: Vec<Pass>,
    /// Parked pushes this app pushed whose report to the server (`pushed`) has not gone through yet.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    unreported: Vec<String>,
    /// The approval phone's signing key (base64url), pinned the first time the server named one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    signing_key: Option<String>,
    /// Another key the server named since: nothing is collected or pushed until the user compares it with the phone
    /// and trusts it (`reins tickets --trust-key`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    changed_key: Option<String>,
}

/// What `reins tickets` and `reins wait` say while the phone's signing key changed and is not trusted yet.
#[must_use]
pub fn key_changed_message(was: &str, now: &str) -> String {
    format!(
        "The phone's signing key changed (was {was}, now {now}). Compare with the phone (the connection's More), then \
         run: reins tickets --trust-key"
    )
}

/// The twelve digits the phone shows for a signing key (the key itself, cut, if it is not one).
fn fingerprint(key: &str) -> String {
    reins_proto::ticket::signing_key_fingerprint(key).unwrap_or_else(|| key.chars().take(16).collect())
}

fn store_file(paths: &Paths) -> PathBuf {
    paths.state_dir.join("tickets.json")
}

impl TicketStore {
    /// What is kept (empty when nothing is, or the file cannot be read: tickets are a convenience, never a grant).
    #[must_use]
    pub fn load(paths: &Paths) -> Self {
        let file = store_file(paths);
        let mut store = match std::fs::read(&file) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
                log::warn!("{}: {e}; starting over", file.display());
                Self::default()
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(e) => {
                log::warn!("{}: {e}", file.display());
                Self::default()
            }
        };
        store.file = file;
        store
    }

    pub fn save(&mut self) -> Result<(), String> {
        let now = crate::now_unix();
        self.passes.retain(|p| p.expires_at > now);
        self.tickets.retain(|t| t.waits() || t.created_at + KEEP_CLOSED_SECS > now);
        if self.tickets.len() > MAX_TICKETS {
            self.tickets.sort_by_key(|t| (t.waits(), t.created_at));
            let extra = self.tickets.len() - MAX_TICKETS;
            self.tickets.drain(..extra);
            self.tickets.sort_by_key(|t| t.created_at);
        }
        let known: Vec<String> = self.tickets.iter().map(|t| t.ticket.clone()).collect();
        self.unreported.retain(|u| known.contains(u));
        if self.passes.len() > MAX_PASSES {
            let extra = self.passes.len() - MAX_PASSES;
            self.passes.drain(..extra);
        }
        let bytes = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        write_private(&self.file, &bytes).map_err(|e| format!("{}: {e}", self.file.display()))
    }

    /// Loads, changes with `f` and saves, holding the lock so no other process changes it meanwhile (a pass is used
    /// once even by two hooks at the same instant).
    pub fn update<R>(paths: &Paths, f: impl FnOnce(&mut Self) -> R) -> Result<R, String> {
        paths.ensure().map_err(|e| e.to_string())?;
        let lock_file = paths.state_dir.join("tickets.lock");
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock_file)
            .map_err(|e| format!("{}: {e}", lock_file.display()))?;
        lock.lock().map_err(|e| format!("{}: {e}", lock_file.display()))?;
        let mut store = Self::load(paths);
        let r = f(&mut store);
        let saved = store.save();
        drop(lock);
        saved.map(|()| r)
    }

    #[must_use]
    pub fn tickets(&self) -> &[LocalTicket] {
        &self.tickets
    }

    #[must_use]
    pub fn get(&self, ticket: &str) -> Option<&LocalTicket> {
        self.tickets.iter().find(|t| t.ticket == ticket)
    }

    #[must_use]
    pub fn passes(&self) -> &[Pass] {
        &self.passes
    }

    /// Adds `t`, or (the same ticket again: the server shares a ticket between identical requests) keeps the one there,
    /// whose nonce the phone's answer echoes.
    pub fn add(&mut self, t: LocalTicket) {
        match self.tickets.iter_mut().find(|k| k.ticket == t.ticket) {
            Some(known) if known.waits() => {
                if known.session.is_none() {
                    known.session = t.session;
                }
            }
            Some(known) => *known = t,
            None => self.tickets.push(t),
        }
    }

    /// Whether any ticket waits for the server (so a hook has a reason to look).
    #[must_use]
    pub fn any_waiting(&self) -> bool {
        self.tickets.iter().any(LocalTicket::waits)
    }

    /// The pinned signing key of the approval phone.
    #[must_use]
    pub fn signing_key(&self) -> Option<&str> {
        self.signing_key.as_deref()
    }

    /// The pinned key's fingerprint (`4821 9930 1274`).
    #[must_use]
    pub fn signing_key_fingerprint(&self) -> Option<String> {
        self.signing_key.as_deref().map(fingerprint)
    }

    /// The fingerprints of the pinned key and of the different one the server named since, while it is not trusted.
    #[must_use]
    pub fn key_change(&self) -> Option<(String, String)> {
        Some((fingerprint(self.signing_key.as_deref()?), fingerprint(self.changed_key.as_deref()?)))
    }

    /// The server named `key` as the phone's: pinned when none is yet; a different one is held back until trusted
    /// (the pinned one again clears it). Whether answers may be collected.
    pub fn see_signing_key(&mut self, key: &str) -> bool {
        if reins_proto::ticket::signing_key_fingerprint(key).is_none() {
            log::warn!("tickets: the server named a signing key that is not one; ignored");
            return self.changed_key.is_none();
        }
        match self.signing_key.as_deref() {
            None => self.signing_key = Some(key.to_owned()),
            Some(pinned) if pinned == key => self.changed_key = None,
            Some(_) => self.changed_key = Some(key.to_owned()),
        }
        self.changed_key.is_none()
    }

    /// Pins the changed key the user compared with the phone; the new pinned key, or `None` when nothing changed.
    pub fn trust_key(&mut self) -> Option<String> {
        let key = self.changed_key.take()?;
        self.signing_key = Some(key.clone());
        Some(key)
    }

    /// Whether a look at the server is due: a ticket waits, or a parked push's report is owed.
    #[must_use]
    pub fn needs_look(&self) -> bool {
        self.any_waiting() || !self.unreported.is_empty()
    }

    /// What changed since `session` was last told: its own tickets, and those of no session. Each change is told once.
    pub fn news_for(&mut self, session: &str) -> Vec<String> {
        let mut out = Vec::new();
        for t in &mut self.tickets {
            if t.session.as_deref().is_some_and(|s| s != session) || t.told.as_deref() == Some(t.state.as_str()) {
                continue;
            }
            if let Some(line) = t.news() {
                t.told = Some(t.state.clone());
                out.push(line);
            }
        }
        out
    }

    /// Uses the pass for `digest` in `session` (a hook's; `None`: `reins ask`), if there is one that has not expired:
    /// once. A pass from another session, or a hook's for `reins ask` and the other way round, does nothing.
    pub fn take_pass(&mut self, digest: &str, session: Option<&str>, now: i64) -> Option<Pass> {
        self.passes.retain(|p| p.expires_at > now);
        let at = self.passes.iter().position(|p| p.digest == digest && p.session.as_deref() == session)?;
        Some(self.passes.remove(at))
    }

    /// Turns the phone's approval of `ticket` into a pass, when it is for exactly that ticket: approved, the ticket's
    /// nonce, its digest, not expired. Valid until the answer's expiry and at most an hour from now (when it is
    /// granted). A ticket gets one pass: an answer collected again is refused. A hook's pass is bound to the hook's
    /// session (a hook ticket without one gets none); an ask's to none.
    pub fn grant_pass(&mut self, ticket: &str, answer: &AskAnswer, now: i64) -> bool {
        let Some(t) = self.tickets.iter_mut().find(|t| t.ticket == ticket && t.kind != TicketKind::Push) else {
            return false;
        };
        let Some(expires_at) = answer.expires_at.filter(|e| *e > now) else {
            return false;
        };
        let session = match t.kind {
            TicketKind::Hook => match &t.session {
                Some(s) => Some(s.clone()),
                None => return false,
            },
            _ => None,
        };
        if !answer.approved
            || answer.nonce != t.nonce
            || answer.digest.as_deref() != Some(t.digest.as_str())
            || t.state() == Some(TicketState::Approved)
        {
            return false;
        }
        TicketState::Approved.as_str().clone_into(&mut t.state);
        let pass = Pass {
            ticket: t.ticket.clone(),
            digest: t.digest.clone(),
            expires_at: expires_at.min(now + AGENT_RUN_VALIDITY_SECS),
            session,
        };
        self.passes.retain(|p| p.ticket != pass.ticket);
        self.passes.push(pass);
        true
    }

    fn apply(&mut self, c: &Change, now: i64) {
        if let Some(answer) = &c.answer {
            if !self.grant_pass(&c.ticket, answer, now) {
                log::warn!("ticket {}: the phone's answer does not match it; refused", c.ticket);
                if let Some(t) = self.tickets.iter_mut().find(|t| t.ticket == c.ticket)
                    && t.state() != Some(TicketState::Approved)
                {
                    TicketState::Failed.as_str().clone_into(&mut t.state);
                    t.note = Some("The phone's answer does not match this ticket; refused.".to_owned());
                }
            }
            return;
        }
        if c.unreported && !self.unreported.contains(&c.ticket) {
            self.unreported.push(c.ticket.clone());
        }
        let Some(t) = self.tickets.iter_mut().find(|t| t.ticket == c.ticket) else {
            return;
        };
        c.state.as_str().clone_into(&mut t.state);
        if c.note.is_some() {
            t.note.clone_from(&c.note);
        }
        if c.stale.is_some() {
            t.stale.clone_from(&c.stale);
        }
        if let Some(by) = c.expected_by.filter(|b| *b > 0) {
            t.expected_by = by;
        }
    }
}

/// The domain of a pass's digest: what a later approval lets through once.
pub const PASS_DOMAIN: &str = "reins-hook-pass/1";

/// A pass's digest over `parts`, each as given (never cut or cleaned for display), length-prefixed after
/// [`PASS_DOMAIN`] and `kind` (`hook`, `ask`), so no two different calls share one.
#[must_use]
pub fn pass_digest(kind: &str, parts: &[&[u8]]) -> String {
    let mut h = Sha256::new();
    for part in [PASS_DOMAIN.as_bytes(), kind.as_bytes()].iter().chain(parts) {
        h.update((part.len() as u64).to_be_bytes());
        h.update(part);
    }
    data_encoding::HEXLOWER.encode(&h.finalize())
}

/// An optional part of a digest: absent and empty differ.
#[must_use]
pub fn optional(part: Option<&str>) -> Vec<u8> {
    part.map_or_else(|| vec![0], |p| [&[1], p.as_bytes()].concat())
}

/// The digest a `reins ask` pass is bound to: the folder it runs in and the question, detail and topic as typed.
#[must_use]
pub fn ask_digest(cwd: Option<&str>, question: &str, detail: Option<&str>, topic: Option<&str>) -> String {
    pass_digest("ask", &[&optional(cwd), question.as_bytes(), &optional(detail), &optional(topic)])
}

/// A time ahead, short and rounded up: `1 min`, `25 min`, `4 h`, `3 d`.
#[must_use]
pub fn within(secs: i64) -> String {
    let up = |unit: i64| (secs.max(1) + unit - 1) / unit;
    match secs {
        ..3_600 => format!("{} min", up(60)),
        3_600..172_800 => format!("{} h", up(3_600)),
        _ => format!("{} d", up(86_400)),
    }
}

/// What a hook tells the agent about a question parked for review.
#[must_use]
pub fn parked_hook_message(ticket: &str, expected_by: i64, now: i64) -> String {
    let when = if expected_by > now {
        format!(", expected within {}", within(expected_by - now))
    } else {
        String::new()
    };
    format!(
        "Parked for review (ticket {ticket}{when}). Continue with other work; when it's approved, re-run this exact \
         command."
    )
}

/// One line, at most [`MAX_SUMMARY`] characters.
#[must_use]
pub fn summary_of(text: &str) -> String {
    let line = reins_proto::desktop::printable(text);
    if line.chars().count() <= MAX_SUMMARY {
        line
    } else {
        format!("{}…", line.chars().take(MAX_SUMMARY - 1).collect::<String>())
    }
}

fn parked_dir(paths: &Paths) -> PathBuf {
    paths.state_dir.join("parked")
}

/// Where the body of the parked push `ticket` is kept.
#[must_use]
pub fn parked_file(paths: &Paths, ticket: &str) -> PathBuf {
    parked_dir(paths).join(format!("{ticket}.body"))
}

fn ticket_ok(ticket: &str) -> bool {
    ticket.starts_with(reins_proto::ticket::TICKET_PREFIX) && reins_proto::ticket::request_of(ticket).is_some()
}

/// Keeps a push for a later review: its body (as git sent it) under `parked/`, and `ticket`. The same ticket again
/// (git run again before an answer) keeps what is there. Blocking.
pub fn park_push(paths: &Paths, ticket: LocalTicket, body: &crate::proxy::body::Spool) -> Result<(), String> {
    if !ticket_ok(&ticket.ticket) {
        return Err("the server named no usable ticket".to_owned());
    }
    if body.len() > MAX_PARKED_BODY {
        return Err(format!("it is larger than {} MiB", MAX_PARKED_BODY >> 20));
    }
    TicketStore::update(paths, |store| {
        let file = parked_file(paths, &ticket.ticket);
        if store.get(&ticket.ticket).is_some_and(LocalTicket::waits) && file.is_file() {
            return Ok(());
        }
        let dir = parked_dir(paths);
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
                .map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let shown = |e: std::io::Error| format!("{}: {e}", file.display());
        // NamedTempFile is 0600 on Unix.
        let mut tmp = tempfile::NamedTempFile::new_in(&dir).map_err(shown)?;
        let mut reader = body.reader().map_err(shown)?;
        std::io::copy(&mut reader, &mut tmp).map_err(shown)?;
        tmp.flush().map_err(shown)?;
        tmp.persist(&file).map_err(|e| shown(e.error))?;
        store.add(ticket);
        Ok(())
    })?
}

/// Whether the daemon answers on its control API (asked briefly).
pub async fn daemon_running(paths: &Paths, config: &Config) -> bool {
    let Ok(client) = crate::control::Client::new(paths, config.listen) else {
        return false;
    };
    matches!(tokio::time::timeout(Duration::from_millis(700), client.status()).await, Ok(Ok(_)))
}

/// What one look at a ticket changed.
#[derive(Clone, Debug)]
struct Change {
    ticket: String,
    state: TicketState,
    note: Option<String>,
    stale: Option<String>,
    expected_by: Option<i64>,
    /// The phone's approval of a hook's or an ask's question, for [`TicketStore::grant_pass`].
    answer: Option<AskAnswer>,
    /// A parked push went, but telling the server so did not: it is told on a later look.
    unreported: bool,
}

impl Change {
    fn to(ticket: &str, state: TicketState) -> Self {
        Self {
            ticket: ticket.to_owned(),
            state,
            note: None,
            stale: None,
            expected_by: None,
            answer: None,
            unreported: false,
        }
    }

    fn note(mut self, note: Option<String>) -> Self {
        self.note = note.map(|n| reins_proto::desktop::printable(&n)).filter(|n| !n.is_empty());
        self
    }

    fn stale(mut self, stale: Option<String>) -> Self {
        self.stale = stale.map(|n| reins_proto::desktop::printable(&n)).filter(|n| !n.is_empty());
        self
    }

    /// It ends the wait here: answered, closed, or (a push) stale, which git pushes again.
    fn settles(&self) -> bool {
        self.answer.is_some() || !self.state.is_open() || self.state == TicketState::Stale
    }
}

/// Looks at the tickets that wait (only `only`, when named): their state on the server, and collects approved ones.
/// Approved hooks and asks become passes; approved pushes are pushed when `pushes` (the daemon), in ticket order per
/// repository and ref. Returns how many tickets changed.
pub async fn refresh(paths: &Paths, client: &DesktopClient, pushes: bool) -> Result<usize, String> {
    refresh_only(paths, client, None, pushes).await
}

async fn refresh_only(
    paths: &Paths,
    client: &DesktopClient,
    only: Option<&str>,
    pushes: bool,
) -> Result<usize, String> {
    let snapshot = TicketStore::load(paths);
    let mut waiting: Vec<&LocalTicket> =
        snapshot.tickets.iter().filter(|t| t.waits() && only.is_none_or(|o| o == t.ticket)).collect();
    let owed: Vec<&String> = snapshot.unreported.iter().filter(|u| only.is_none_or(|o| o == u.as_str())).collect();
    if waiting.is_empty() && owed.is_empty() {
        return Ok(0);
    }
    // Ticket order: when it was made, then the order it was kept in (the same second).
    let order = |t: &LocalTicket| (t.created_at, snapshot.tickets.iter().position(|k| k.ticket == t.ticket));
    waiting.sort_by_key(|t| order(t));
    let (listed, key) = match client.tickets().await {
        Ok(l) => (Some(l.tickets), l.signing_key),
        Err(LinkError::NotFound) => (None, None),
        Err(e) => return Err(e.to_string()),
    };
    // Answers are collected (passes granted, pushes pushed) only while the phone's key is the pinned one.
    let trusted = match key {
        Some(k) => TicketStore::update(paths, |s| s.see_signing_key(&k))?,
        None => snapshot.changed_key.is_none(),
    };
    if !trusted {
        log::warn!("tickets: the phone's signing key changed; nothing is collected until it is trusted");
    }
    // Pushes made but not yet reported: told now, until the server takes it or the ticket is no longer approved there.
    let mut reported = Vec::new();
    for ticket in owed {
        let there = listed.as_ref().and_then(|l| l.iter().find(|d| &d.ticket == ticket)).map(|d| d.state);
        let done = match there {
            Some(s) if s != TicketState::Approved => true,
            _ => match client.report_pushed(ticket).await {
                Ok(()) | Err(LinkError::NotFound) => true,
                Err(e) => {
                    log::info!("ticket {ticket}: reporting it pushed: {e}");
                    false
                }
            },
        };
        if done {
            reported.push(ticket.clone());
        }
    }
    if !reported.is_empty() {
        TicketStore::update(paths, |s| s.unreported.retain(|u| !reported.contains(u)))?;
    }
    let identity = Identity::load_or_create(&paths.identity_file()).map_err(|e| e.to_string())?;
    // Tickets still open after what this look changed: a push waits while an earlier one to the same ref does.
    let mut open: Vec<&LocalTicket> = snapshot.tickets.iter().filter(|t| t.waits()).collect();
    let mut changes = Vec::new();
    for t in waiting {
        let seen: Option<&DesktopTicket> = listed.as_ref().and_then(|l| l.iter().find(|d| d.ticket == t.ticket));
        let change = match seen {
            Some(d) if server_may_set(d.state) => listed_change(t, d),
            // Approved (or said to be done): collect, and check. Not listed (an older server, or closed long ago):
            // ask about it alone.
            _ => {
                // A push is collected by the daemon only, and waits while an earlier one to the same ref is open.
                let collects = trusted
                    && (t.kind != TicketKind::Push
                        || (pushes
                            && !open.iter().any(|e| {
                                e.ticket != t.ticket
                                    && e.kind == TicketKind::Push
                                    && order(e) < order(t)
                                    && e.overlaps(t)
                            })));
                if seen.is_some() && !collects {
                    None
                } else {
                    look(paths, client, &identity, t, collects).await
                }
            }
        };
        if let Some(c) = change {
            if c.settles() {
                open.retain(|o| o.ticket != c.ticket);
            }
            changes.push(c);
        }
    }
    let n = changes.len();
    if n > 0 {
        let now = crate::now_unix();
        TicketStore::update(paths, |s| {
            for c in &changes {
                s.apply(c, now);
            }
        })?;
    }
    // A push that will not be pushed from here (denied, expired, cancelled, stale) keeps nothing.
    for c in changes.iter().filter(|c| c.settles()) {
        let body = parked_file(paths, &c.ticket);
        if snapshot.get(&c.ticket).is_some_and(|t| t.kind == TicketKind::Push)
            && let Err(e) = std::fs::remove_file(&body)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            log::warn!("{}: {e}", body.display());
        }
    }
    Ok(n)
}

/// Whether the server's word alone may put a ticket in `state`. It may close or reopen one (denied, expired, stale…),
/// never say it went well: approved is only ever this app's own check of the phone's sealed answer
/// ([`TicketStore::grant_pass`], [`check_grant`]), and done only a push this app made itself.
fn server_may_set(state: TicketState) -> bool {
    !matches!(state, TicketState::Approved | TicketState::Revalidating | TicketState::Executing | TicketState::Done)
}

/// The change the ticket list says, if any (an approved ticket is collected instead).
fn listed_change(t: &LocalTicket, d: &DesktopTicket) -> Option<Change> {
    if !server_may_set(d.state) {
        return None;
    }
    let same = t.state() == Some(d.state)
        && (d.stale.is_none() || d.stale == t.stale)
        && (d.note.is_none() || d.note == t.note)
        && d.deadline == t.expected_by;
    if same {
        return None;
    }
    let mut c = Change::to(&t.ticket, d.state).note(d.note.clone()).stale(d.stale.clone());
    c.expected_by = Some(d.deadline);
    Some(c)
}

/// Polls one ticket: its state, or (approved, `collect`) the phone's answer, then for a push the push itself.
async fn look(
    paths: &Paths,
    client: &DesktopClient,
    identity: &Identity,
    t: &LocalTicket,
    collect: bool,
) -> Option<Change> {
    if !collect {
        return None;
    }
    let answer = match client.poll(&t.ticket).await {
        Ok(a) => a,
        Err(LinkError::NotFound) => {
            return Some(
                Change::to(&t.ticket, TicketState::Expired)
                    .note(Some("The Reins server no longer has this ticket.".to_owned())),
            );
        }
        Err(e) => {
            log::info!("ticket {}: {e}", t.ticket);
            return None;
        }
    };
    match answer.status {
        CallStatus::Answered(outcome) => Some(collected(paths, client, identity, t, outcome).await),
        // Approved, but the phone did not produce the answer in time: next time.
        CallStatus::Deferred {
            state: Some(TicketState::Approved),
            ..
        }
        | CallStatus::Pending
        | CallStatus::Offline => None,
        CallStatus::Deferred {
            state,
            expected_by,
            ..
        } => {
            let state = state.unwrap_or(TicketState::Deferred);
            if !server_may_set(state) {
                return None;
            }
            let mut c = Change::to(&t.ticket, state);
            c.expected_by = Some(expected_by);
            (t.state() != Some(state) || (expected_by > 0 && expected_by != t.expected_by)).then_some(c)
        }
        CallStatus::Closed {
            state,
            stale,
        } if server_may_set(state) => Some(Change::to(&t.ticket, state).stale(stale)),
        // Closed as if it went well, with nothing this app can check: over, not approved.
        CallStatus::Closed {
            state,
            ..
        } => Some(Change::to(&t.ticket, TicketState::Unknown).note(Some(format!(
            "The Reins server closed it as {}, with no answer this app can verify.",
            state.as_str()
        )))),
    }
}

/// The phone's answer to an approved ticket.
async fn collected(
    paths: &Paths,
    client: &DesktopClient,
    identity: &Identity,
    t: &LocalTicket,
    outcome: RelayOutcome,
) -> Change {
    let failed = |why: &str| Change::to(&t.ticket, TicketState::Failed).note(Some(why.to_owned()));
    let data = match outcome {
        RelayOutcome::Result {
            result: ToolResult::Connector {
                data,
            },
        } => data,
        RelayOutcome::Result {
            ..
        } => return failed("The phone's answer is not one this app reads."),
        RelayOutcome::Denied {
            reason,
        } => return Change::to(&t.ticket, TicketState::Denied).note(reason),
        RelayOutcome::Error {
            message,
        } => return failed(&reins_proto::desktop::printable(&message)),
    };
    let Some(sealed) =
        data.get(SEALED_FIELD).or_else(|| data.pointer(&format!("/items/0/{SEALED_FIELD}"))).and_then(Value::as_str)
    else {
        return failed("The phone's answer has nothing sealed for this app.");
    };
    if t.kind == TicketKind::Push {
        let Ok(grant) = identity.open_grant(sealed) else {
            return failed("The phone's answer is not a credential sealed to this app; refused.");
        };
        if let Err(why) = check_grant(t, &grant, crate::now_unix()) {
            return failed(&why);
        }
        return push_parked(paths, client, t, grant).await;
    }
    let answer = identity.unseal(sealed).ok().and_then(|plain| serde_json::from_slice::<AskAnswer>(&plain).ok());
    match answer {
        None => failed("The phone's answer is malformed; refused."),
        Some(a) if !a.approved => Change::to(&t.ticket, TicketState::Denied),
        Some(a) => {
            let mut c = Change::to(&t.ticket, TicketState::Approved);
            c.answer = Some(a);
            c
        }
    }
}

/// The phone's credential for a parked push answers exactly it: its nonce, its repository, write access, its digest,
/// not expired.
fn check_grant(t: &LocalTicket, g: &CredentialGrant, now: i64) -> Result<(), String> {
    let repo = t.push.as_ref().map(|p| p.repo.as_str()).unwrap_or_default();
    if g.nonce != t.nonce {
        return Err("The phone's answer is for another request (the nonce differs); refused.".to_owned());
    }
    if g.repo != repo {
        return Err(format!("The phone's answer is for {}, not {repo}; refused.", g.repo));
    }
    if g.access != "write" || g.digest.as_deref() != Some(t.digest.as_str()) {
        return Err("The phone approved a different push (the digest differs); refused.".to_owned());
    }
    if g.expires_at <= now || g.token.is_empty() {
        return Err("The phone's credential has expired; refused.".to_owned());
    }
    Ok(())
}

/// How pushing a parked push ended.
#[derive(Debug, PartialEq, Eq)]
enum PushEnd {
    Done,
    Failed(String),
    /// What moved.
    Stale(String),
}

/// Pushes the parked push of `t` with the phone's credential, if its refs are still where they were; tells the server
/// when they are not. The kept body goes either way.
async fn push_parked(paths: &Paths, client: &DesktopClient, t: &LocalTicket, grant: CredentialGrant) -> Change {
    let body = parked_file(paths, &t.ticket);
    let credential = Credential {
        username: grant.username.clone(),
        token: Zeroizing::new(grant.token.clone()),
        expires_at: grant.expires_at,
    };
    let end = match (&t.push, body.is_file()) {
        (Some(p), true) => push_with(p, &t.refs, &body, &credential).await,
        _ => PushEnd::Failed("The parked push is no longer on this computer.".to_owned()),
    };
    if let Err(e) = std::fs::remove_file(&body)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        log::warn!("{}: {e}", body.display());
    }
    log::info!("ticket {}: parked push {end:?}", t.ticket);
    match end {
        // Done here once the host took it; the server's ticket is done once told (else told on a later look).
        PushEnd::Done => {
            let mut c = Change::to(&t.ticket, TicketState::Done);
            if let Err(e) = client.report_pushed(&t.ticket).await {
                log::info!("ticket {}: reporting it pushed: {e}", t.ticket);
                c.unreported = true;
            }
            c
        }
        PushEnd::Failed(why) => Change::to(&t.ticket, TicketState::Failed).note(Some(why)),
        PushEnd::Stale(diff) => {
            if let Err(e) = client.report_stale(&t.ticket, &diff).await {
                log::warn!("ticket {}: telling the server it is stale: {e}", t.ticket);
            }
            Change::to(&t.ticket, TicketState::Stale).stale(Some(diff))
        }
    }
}

async fn read_answer(resp: reqwest::Response) -> Result<Vec<u8>, String> {
    crate::server::read_limited(resp, MAX_GIT_ANSWER).await
}

/// The ref list before a push, then the push: only when every ref is where git found it (force-with-lease).
async fn push_with(p: &ParkedPush, refs: &[(String, String, String)], body: &Path, c: &Credential) -> PushEnd {
    let http = match crate::http::client(None) {
        Ok(h) => h,
        Err(e) => return PushEnd::Failed(e),
    };
    let Ok(mut auth) = reqwest::header::HeaderValue::from_str(&c.authorization()) else {
        return PushEnd::Failed("The credential is malformed.".to_owned());
    };
    auth.set_sensitive(true);
    let listed = http
        .get(format!("{}/info/refs?service=git-receive-pack", p.upstream))
        .header(reqwest::header::AUTHORIZATION, auth.clone())
        .timeout(Duration::from_secs(60))
        .send()
        .await;
    let listed = match listed {
        Ok(r) if r.status().is_success() => r,
        Ok(r) => return PushEnd::Failed(format!("The git host answered {} to the ref list.", r.status().as_u16())),
        Err(e) => return PushEnd::Failed(format!("Cannot reach the git host: {}", e.without_url())),
    };
    let tips = match read_answer(listed).await.and_then(|b| advertised(&b)) {
        Ok(t) => t,
        Err(e) => return PushEnd::Failed(format!("The git host's ref list: {e}")),
    };
    let moved = moved_refs(refs, &tips);
    if !moved.is_empty() {
        return PushEnd::Stale(moved.join("; "));
    }
    let file = match tokio::fs::File::open(body).await {
        Ok(f) => f,
        Err(e) => return PushEnd::Failed(format!("{}: {e}", body.display())),
    };
    let len = file.metadata().await.map(|m| m.len()).unwrap_or_default();
    let stream = futures_util::stream::try_unfold(file, async |mut f| {
        let mut buf = vec![0u8; 64 << 10];
        let n = tokio::io::AsyncReadExt::read(&mut f, &mut buf).await?;
        if n == 0 {
            return Ok::<_, std::io::Error>(None);
        }
        buf.truncate(n);
        Ok(Some((Bytes::from(buf), f)))
    });
    let mut req = http
        .post(format!("{}/git-receive-pack", p.upstream))
        .header(reqwest::header::AUTHORIZATION, auth)
        .header(reqwest::header::CONTENT_TYPE, "application/x-git-receive-pack-request")
        .header(reqwest::header::ACCEPT, "application/x-git-receive-pack-result")
        .header(reqwest::header::CONTENT_LENGTH, len);
    if p.gzip {
        req = req.header(reqwest::header::CONTENT_ENCODING, "gzip");
    }
    let resp = match req.body(reqwest::Body::wrap_stream(stream)).send().await {
        Ok(r) => r,
        Err(e) => return PushEnd::Failed(format!("Cannot reach the git host: {}", e.without_url())),
    };
    if !resp.status().is_success() {
        return PushEnd::Failed(format!("The git host answered {} to the push.", resp.status().as_u16()));
    }
    match read_answer(resp).await.and_then(|b| push_report(&b, p.sideband, p.report)) {
        Ok(()) => PushEnd::Done,
        Err(e) => PushEnd::Failed(e),
    }
}

/// `<ref> moved from <old7> to <now7>` for each ref that is not where the push expects it (absent: zeros).
fn moved_refs(refs: &[(String, String, String)], tips: &HashMap<String, String>) -> Vec<String> {
    let short = |oid: &str| oid.chars().take(7).collect::<String>();
    refs.iter()
        .filter_map(|(old, _, name)| {
            let now = tips.get(name).map_or("0000000", String::as_str);
            let same = if is_zero(old) {
                !tips.contains_key(name)
            } else {
                now == old
            };
            (!same).then(|| format!("{name} moved from {} to {}", short(old), short(now)))
        })
        .collect()
}

/// The refs a `git-receive-pack` advertisement lists, by name.
fn advertised(body: &[u8]) -> Result<HashMap<String, String>, String> {
    let mut tips = HashMap::new();
    let mut r = Reader::new(body);
    while let Some(pkt) = r.next_pkt().map_err(|e| e.to_string())? {
        let Pkt::Data(data) = pkt else {
            continue;
        };
        let line = String::from_utf8_lossy(data);
        let line = line.trim_end_matches('\n');
        if line.starts_with('#') {
            continue;
        }
        let line = line.split('\0').next().unwrap_or_default();
        let Some((oid, name)) = line.split_once(' ') else {
            return Err("a malformed line".to_owned());
        };
        if name != "capabilities^{}" {
            tips.insert(name.to_owned(), oid.to_owned());
        }
    }
    Ok(tips)
}

/// Whether the host took every ref: `unpack ok` and no `ng` in its report (out of side-band 1 when git asked for one).
/// Without a report asked for, the answer's status is all there is.
fn push_report(body: &[u8], sideband: bool, report: bool) -> Result<(), String> {
    if !report {
        return Ok(());
    }
    let data = if sideband {
        let mut data = Vec::new();
        let mut r = Reader::new(body);
        while let Some(pkt) = r.next_pkt().map_err(|e| e.to_string())? {
            match pkt {
                Pkt::Data([1, rest @ ..]) => data.extend_from_slice(rest),
                Pkt::Data([3, rest @ ..]) => {
                    return Err(format!(
                        "the git host: {}",
                        reins_proto::desktop::printable(&String::from_utf8_lossy(rest))
                    ));
                }
                _ => {}
            }
        }
        data
    } else {
        body.to_vec()
    };
    let mut r = Reader::new(&data);
    let mut unpacked = false;
    while let Some(pkt) = r.next_pkt().map_err(|e| e.to_string())? {
        let Pkt::Data(line) = pkt else {
            continue;
        };
        let line = String::from_utf8_lossy(line);
        let line = line.trim_end_matches('\n');
        if line == "unpack ok" {
            unpacked = true;
        } else if let Some(why) = line.strip_prefix("unpack ") {
            return Err(format!("the git host could not unpack it: {}", reins_proto::desktop::printable(why)));
        } else if let Some(ng) = line.strip_prefix("ng ") {
            return Err(format!("the git host refused {}", reins_proto::desktop::printable(ng)));
        }
    }
    if unpacked {
        Ok(())
    } else {
        Err("the git host sent no report".to_owned())
    }
}

/// How `reins wait` ends for a ticket of `kind`: 0 approved (a question: its pass granted from the phone's verified
/// answer) or pushed (a push: by this app), 1 denied, 2 expired, stale, cancelled, failed or out of time.
#[must_use]
pub fn exit_code(kind: TicketKind, state: TicketState) -> Option<u8> {
    match state {
        TicketState::Approved if kind != TicketKind::Push => Some(0),
        TicketState::Done if kind == TicketKind::Push => Some(0),
        TicketState::Denied => Some(1),
        // A question is never done, a push never merely approved: not something this app made, so not a success.
        TicketState::Approved
        | TicketState::Done
        | TicketState::Expired
        | TicketState::Stale
        | TicketState::Cancelled
        | TicketState::Failed
        | TicketState::Unknown => Some(2),
        _ => None,
    }
}

/// How often `reins wait` looks.
const WAIT_SPACING: Duration = Duration::from_secs(1);

/// Waits up to `timeout` for `ticket` to be decided; the exit code of [`exit_code`] (2 when the time runs out, or
/// at once while the phone's signing key changed and is not trusted: [`TicketStore::key_change`]). Only a
/// ticket of this computer can end approved: its answer is collected and checked as [`refresh`] does (pushes only
/// when `pushes`). Any other ticket ends at once with 2: nothing the server says about it can be checked here.
pub async fn wait(paths: &Paths, client: &DesktopClient, ticket: &str, timeout: Duration, pushes: bool) -> u8 {
    let deadline = Instant::now() + timeout;
    let settled = |t: &LocalTicket| t.state().filter(|_| !t.waits()).and_then(|s| exit_code(t.kind, s));
    loop {
        let store = TicketStore::load(paths);
        if store.key_change().is_some() {
            return 2;
        }
        let Some(t) = store.get(ticket).cloned() else {
            log::info!("ticket {ticket}: not a ticket of this computer");
            return 2;
        };
        if let Some(code) = settled(&t) {
            return code;
        }
        if let Err(e) = refresh_only(paths, client, Some(ticket), pushes).await {
            log::info!("ticket {ticket}: {e}");
        }
        // Refused while the phone's key changed: the caller says why.
        if TicketStore::load(paths).key_change().is_some() {
            return 2;
        }
        if let Some(code) = TicketStore::load(paths).get(ticket).and_then(settled) {
            return code;
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return 2;
        }
        tokio::time::sleep(WAIT_SPACING.min(remaining)).await;
    }
}

/// Asks the server which key the phone signs with, and pins it when none is ([`TicketStore::see_signing_key`]).
pub async fn see_signing_key(paths: &Paths, client: &DesktopClient) -> Result<(), String> {
    match client.tickets().await {
        Ok(l) => {
            if let Some(k) = l.signing_key {
                TicketStore::update(paths, |s| s.see_signing_key(&k))?;
            }
            Ok(())
        }
        Err(LinkError::NotFound) => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

/// The daemon's look at the tickets every [`REFRESH_EVERY`], when logged in and something waits.
pub async fn refresher(paths: Paths) {
    loop {
        tokio::time::sleep(REFRESH_EVERY).await;
        if crate::server::oauth::logged_in_server(&paths).is_none() || !TicketStore::load(&paths).needs_look() {
            continue;
        }
        match DesktopClient::new(&paths) {
            Ok(client) => match refresh(&paths, &client, true).await {
                Ok(0) => {}
                Ok(n) => log::info!("tickets: {n} changed"),
                Err(e) => log::info!("tickets: {e}"),
            },
            Err(e) => log::warn!("tickets: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_ahead_round_up() {
        assert_eq!(within(14_400), "4 h");
        assert_eq!(within(14_399), "4 h");
        assert_eq!(within(120), "2 min");
        assert_eq!(within(0), "1 min");
        assert_eq!(within(7 * 86_400), "7 d");
        assert_eq!(
            parked_hook_message("rt_a", 1_000 + 14_400, 1_000),
            "Parked for review (ticket rt_a, expected within 4 h). Continue with other work; when it's approved, \
             re-run this exact command."
        );
    }

    #[test]
    fn a_push_waits_for_refs_where_git_found_them() {
        let a = "1".repeat(40);
        let b = "2".repeat(40);
        let zero = "0".repeat(40);
        let tips: HashMap<String, String> = [("refs/heads/main".to_owned(), a.clone())].into();
        let update = |old: &str, name: &str| vec![(old.to_owned(), b.clone(), name.to_owned())];
        assert!(moved_refs(&update(&a, "refs/heads/main"), &tips).is_empty());
        assert!(moved_refs(&update(&zero, "refs/heads/new"), &tips).is_empty());
        assert_eq!(
            moved_refs(&update(&b, "refs/heads/main"), &tips),
            ["refs/heads/main moved from 2222222 to 1111111"]
        );
        assert_eq!(
            moved_refs(&update(&zero, "refs/heads/main"), &tips),
            ["refs/heads/main moved from 0000000 to 1111111"]
        );
        assert_eq!(
            moved_refs(&update(&a, "refs/heads/gone"), &tips),
            ["refs/heads/gone moved from 1111111 to 0000000"]
        );
    }

    #[test]
    fn advertisements_and_reports_are_read() {
        use crate::git::pktline::{FLUSH, encode, sideband};
        let mut adv = encode(b"# service=git-receive-pack\n");
        adv.extend_from_slice(FLUSH);
        adv.extend(encode(format!("{} refs/heads/main\0report-status side-band-64k\n", "a".repeat(40)).as_bytes()));
        adv.extend(encode(format!("{} refs/tags/v1\n", "b".repeat(40)).as_bytes()));
        adv.extend_from_slice(FLUSH);
        let tips = advertised(&adv).unwrap();
        assert_eq!(tips.len(), 2);
        assert_eq!(tips["refs/heads/main"], "a".repeat(40));
        let mut empty = encode(format!("{} capabilities^{{}}\0report-status\n", "0".repeat(40)).as_bytes());
        empty.extend_from_slice(FLUSH);
        assert!(advertised(&empty).unwrap().is_empty());

        let mut ok = encode(b"unpack ok\n");
        ok.extend(encode(b"ok refs/heads/main\n"));
        ok.extend_from_slice(FLUSH);
        assert_eq!(push_report(&ok, false, true), Ok(()));
        let mut banded = sideband(2, b"progress\n");
        banded.extend(sideband(1, &ok));
        banded.extend_from_slice(FLUSH);
        assert_eq!(push_report(&banded, true, true), Ok(()));
        let mut ng = encode(b"unpack ok\n");
        ng.extend(encode(b"ng refs/heads/main non-fast-forward\n"));
        ng.extend_from_slice(FLUSH);
        assert!(push_report(&ng, false, true).unwrap_err().contains("non-fast-forward"));
        assert!(push_report(FLUSH, false, true).is_err());
        assert_eq!(push_report(b"", false, false), Ok(()));
    }
}
