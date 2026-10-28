//! The policy the phone signs for a paired computer (`docs/design/decisions-v2/08-review-resolutions.md` R1): the
//! computer's daemon decides hook-level requests (shell commands, file tools) with the owner's own Auto rules, without
//! a round trip, and asks the phone for everything else.
//!
//! - [`DesktopPolicy`] is the snapshot: the matrix, the owner's rules, the facts the computer needs (owners,
//!   repositories, hosts: never people, domains or chats), the mode with its end and the connection's tasks. It carries
//!   a version, an issue time and a short validity, and names the computer's key and the profile it is for.
//! - [`Tighten`] is a "tighten now": Lockdown, Deny and Review rules, ended tasks, or "stop deciding here". It applies on
//!   top of whatever the computer holds, a snapshot or none, until a snapshot of the same or a later version arrives.
//! - Both travel as [`Signed`] (the phone's device key, the key the computer pinned, P4) inside a sealed box to the
//!   computer's key; the server relays opaque blobs.
//! - [`effective`] is what the computer decides with: the snapshot, its tightenings and the local configuration, which
//!   only ever makes it stricter.
//!
//! Pure: signatures are checked by a function the caller passes (the Ed25519 implementation lives in the apps).

use std::collections::{BTreeMap, BTreeSet};

use reins_proto::autopilot::Mode;
use serde::{Deserialize, Serialize};

use crate::context::{FactKind, Facts, Zone};
use crate::desktop_folders;
use crate::effects::normalize;
use crate::matrix::{Matrix, Rule, Snapshot, Task, Then};

/// The format of the messages.
pub const FORMAT: u32 = 1;
/// How long a snapshot is used after it was issued, by the computer's clock whatever the signed `not_after` says. Short,
/// so that freshness is the rollback protection: an older snapshot replayed (by the server, or a file put back) is
/// stale within minutes, and then the phone is asked.
pub const VALIDITY_SECS: i64 = 600;
/// How often the phone signs its snapshot again when nothing changed (it also does at once on any change).
pub const REFRESH_SECS: i64 = 240;
/// How long a tightening holds when no snapshot supersedes it.
pub const TIGHTEN_SECS: i64 = 86_400;
/// The longest validity the computer accepts.
pub const MAX_VALIDITY_SECS: i64 = 86_400;
/// How far ahead of the computer's clock an issue time may be.
pub const MAX_SKEW_SECS: i64 = 300;
/// The profile a computer's hooks run in (08 R6: hook-level commands are the agent's own profile's, and the hooks run
/// in the computer's own state, the Personal profile).
pub const PERSONAL: &str = "personal";
/// The largest signed body the computer reads.
pub const MAX_BODY_BYTES: usize = 192 * 1024;
/// What every signature covers before the body.
const CONTEXT: &[u8] = b"reins/desktop-policy/v1\n";

/// Who a message is for and when it holds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Binding {
    /// Only grows (per signing key): an older message never replaces a newer one.
    pub version: u64,
    /// Unix seconds, by the phone's clock.
    pub issued_at: i64,
    /// Unix seconds: not valid from then.
    pub not_after: i64,
    /// The computer's X25519 key the phone pinned for the connection (base64url).
    pub client_key: String,
    /// The computer's connection: tasks and rules are per connection.
    pub connection_id: String,
    /// [`PERSONAL`], or an org's Managed profile (`org:<id>`).
    pub profile: String,
}

/// The phone's mode for the connection, and when it ends (`None`: until changed).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModeState {
    pub mode: Mode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<i64>,
}

/// The snapshot a computer decides with.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesktopPolicy {
    pub binding: Binding,
    /// The phone's matrix decides (Auto's setup done, the switch on): else the computer keeps today's flow.
    pub decides: bool,
    pub mode: ModeState,
    /// The matrix, the owner's rules, the connection's tasks and the facts the computer needs ([`desktop_facts`]).
    pub snapshot: Snapshot,
    /// GitHub logins the owner's facts are kept by (login → account id), as the phone last knew them.
    #[serde(default)]
    pub github_ids: BTreeMap<String, String>,
    /// Folders where agents' work is expected (`~/Documents/dev`): local roots count only inside these.
    #[serde(default)]
    pub roots: Vec<String>,
    /// Scratch folders the owner confirmed (deleting in them loses nothing): a local scratch folder counts only when it
    /// is one of these, or strictly inside a root.
    #[serde(default)]
    pub scratch: Vec<String>,
}

/// What a tightening does until a snapshot supersedes it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tightening {
    /// Every tool use is refused.
    #[serde(default)]
    pub lockdown: bool,
    /// Deny and Review rules (anything else is dropped).
    #[serde(default)]
    pub rules: Vec<Rule>,
    /// Tasks that cover nothing any more.
    #[serde(default)]
    pub ended_tasks: Vec<String>,
    /// Nothing is decided here: everything goes to the phone as before.
    #[serde(default)]
    pub stop_auto: bool,
}

impl Tightening {
    fn is_empty(&self) -> bool {
        !self.lockdown && self.rules.is_empty() && self.ended_tasks.is_empty() && !self.stop_auto
    }
}

/// A "tighten now".
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tighten {
    pub binding: Binding,
    pub tightening: Tightening,
}

/// What the phone signs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PolicyMessage {
    Snapshot(Box<DesktopPolicy>),
    Tighten(Tighten),
}

/// The body of every message, with its format.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Body {
    v: u32,
    #[serde(flatten)]
    message: PolicyMessage,
}

impl PolicyMessage {
    #[must_use]
    pub fn binding(&self) -> &Binding {
        match self {
            Self::Snapshot(p) => &p.binding,
            Self::Tighten(t) => &t.binding,
        }
    }

    #[must_use]
    pub fn version(&self) -> u64 {
        self.binding().version
    }
}

/// A message and the phone's Ed25519 signature over [`signing_bytes`] of its body.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Signed {
    /// The message's JSON, exactly as signed.
    pub body: String,
    /// The signer's public key, base64url without padding.
    pub key: String,
    /// base64url without padding.
    pub sig: String,
}

/// The bytes a signature covers: a domain-separation line, then the body.
#[must_use]
pub fn signing_bytes(body: &str) -> Vec<u8> {
    let mut out = CONTEXT.to_vec();
    out.extend_from_slice(body.as_bytes());
    out
}

/// Signs `message` with `key` (the signer's public key, base64url) through `sign` (the bytes → a base64url
/// signature).
#[must_use]
pub fn sign(message: &PolicyMessage, key: &str, sign: &dyn Fn(&[u8]) -> String) -> Signed {
    let body = serde_json::to_string(&Body {
        v: FORMAT,
        message: message.clone(),
    })
    .unwrap_or_default();
    Signed {
        sig: sign(&signing_bytes(&body)),
        key: key.to_owned(),
        body,
    }
}

/// Why a message is not used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// Not signed by the pinned key (or no key is pinned).
    OtherKey,
    BadSignature,
    Unreadable,
    UnknownFormat,
    /// For another computer's key.
    OtherComputer,
    /// For another profile (08 R6).
    OtherProfile,
    Expired,
    /// Issued ahead of this computer's clock.
    FromTheFuture,
    /// Valid for longer than [`MAX_VALIDITY_SECS`].
    TooLong,
    /// Not newer than what was applied.
    Older,
}

impl Refusal {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::OtherKey => "not signed by the phone's pinned key",
            Self::BadSignature => "the signature doesn't verify",
            Self::Unreadable => "unreadable",
            Self::UnknownFormat => "a format this version doesn't read",
            Self::OtherComputer => "for another computer",
            Self::OtherProfile => "for another profile",
            Self::Expired => "expired",
            Self::FromTheFuture => "issued ahead of this computer's clock",
            Self::TooLong => "valid for too long",
            Self::Older => "older than the one applied",
        }
    }
}

fn b64(s: &str) -> Option<Vec<u8>> {
    data_encoding::BASE64URL_NOPAD.decode(s.as_bytes()).ok()
}

/// A strict Ed25519 check over key, message, signature.
pub type Verify = dyn Fn(&[u8; 32], &[u8], &[u8; 64]) -> bool;

/// The message, when `signed` is signed by `pinned` (base64url) and its signature verifies (`verify`).
pub fn open(signed: &Signed, pinned: Option<&str>, verify: &Verify) -> Result<PolicyMessage, Refusal> {
    if pinned.is_none_or(|p| p != signed.key) {
        return Err(Refusal::OtherKey);
    }
    let key: [u8; 32] = b64(&signed.key).and_then(|k| k.try_into().ok()).ok_or(Refusal::BadSignature)?;
    let sig: [u8; 64] = b64(&signed.sig).and_then(|s| s.try_into().ok()).ok_or(Refusal::BadSignature)?;
    if signed.body.len() > MAX_BODY_BYTES {
        return Err(Refusal::Unreadable);
    }
    if !verify(&key, &signing_bytes(&signed.body), &sig) {
        return Err(Refusal::BadSignature);
    }
    let v: serde_json::Value = serde_json::from_str(&signed.body).map_err(|_| Refusal::Unreadable)?;
    if v.get("v").and_then(serde_json::Value::as_u64) != Some(u64::from(FORMAT)) {
        return Err(Refusal::UnknownFormat);
    }
    let body: Body = serde_json::from_value(v).map_err(|_| Refusal::Unreadable)?;
    Ok(body.message)
}

/// Whether `b` is for this computer (`client_key`), its Personal profile, and holds at `now`.
pub fn check(b: &Binding, client_key: &str, now: i64) -> Result<(), Refusal> {
    if b.client_key != client_key {
        return Err(Refusal::OtherComputer);
    }
    if b.profile != PERSONAL {
        return Err(Refusal::OtherProfile);
    }
    if b.issued_at > now + MAX_SKEW_SECS {
        return Err(Refusal::FromTheFuture);
    }
    if b.not_after - b.issued_at > MAX_VALIDITY_SECS {
        return Err(Refusal::TooLong);
    }
    if now >= b.not_after {
        return Err(Refusal::Expired);
    }
    Ok(())
}

/// [`check`], and for a snapshot also within [`VALIDITY_SECS`] of its issue (the computer's clock bounds it, not only
/// the phone's `not_after`). A tightening holds until its own `not_after`: it only ever tightens.
pub fn check_message(m: &PolicyMessage, client_key: &str, now: i64) -> Result<(), Refusal> {
    let b = m.binding();
    check(b, client_key, now)?;
    match m {
        PolicyMessage::Snapshot(_) if now >= b.issued_at.saturating_add(VALIDITY_SECS) => Err(Refusal::Expired),
        _ => Ok(()),
    }
}

/// What of the phone's facts a computer needs: owners, repositories and hosts, with their labels; and the GitHub
/// logins the owner's id-kept owner facts are shown as (login → id), so the computer reads them as the phone does.
/// People, domains, chats and the owner's addresses stay on the phone.
#[must_use]
pub fn desktop_facts(phone: &Facts) -> (Facts, BTreeMap<String, String>) {
    let facts = Facts {
        owners: phone.owners.clone(),
        repos: phone.repos.clone(),
        hosts: phone.hosts.clone(),
        labels: phone
            .labels
            .iter()
            .filter(|(k, _)| k.starts_with("owner:") || k.starts_with("repo:") || k.starts_with("host:"))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        ..Facts::default()
    };
    let mut ids: BTreeMap<String, String> = BTreeMap::new();
    let mut twice: BTreeSet<String> = BTreeSet::new();
    for (k, login) in &phone.labels {
        let Some(id) = k.strip_prefix("owner:#") else {
            continue;
        };
        if !phone.owners.contains_key(&format!("#{id}")) {
            continue;
        }
        let login = login.trim().to_ascii_lowercase();
        let plain = !login.is_empty()
            && login.len() <= 100
            && login.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
        if !plain {
            continue;
        }
        // Two accounts labelled alike: neither is read by its login.
        if ids.insert(login.clone(), id.to_owned()).is_some_and(|other| other != id) {
            twice.insert(login);
        }
    }
    for login in twice {
        ids.remove(&login);
    }
    (facts, ids)
}

/// The owner's rules a computer gets: those for any connection or for this one, and permits only where they name no
/// person (a Deny or Review rule always goes: leaving one out would loosen).
#[must_use]
pub fn desktop_rules(rules: &[Rule], connection_id: &str) -> Vec<Rule> {
    rules
        .iter()
        .filter(|r| r.when.connection.as_deref().is_none_or(|c| c == connection_id))
        .filter(|r| r.then != Then::Permit || !r.when.resource.as_deref().is_some_and(|x| x.contains('@')))
        .cloned()
        .collect()
}

/// The connection's own tasks.
#[must_use]
pub fn desktop_tasks(tasks: &[Task], connection_id: &str) -> Vec<Task> {
    tasks.iter().filter(|t| t.connection == connection_id).cloned().collect()
}

/// What the phone sends first when `new` tightens anything `old` allowed: Lockdown, new Deny or Review rules, ended
/// tasks; anything else stricter (a cell, a fact, a mode, the matrix no longer deciding, a narrowed task, a permit or
/// root gone) stops the computer deciding until `new` arrives. `None`: `new` only loosens or keeps.
#[must_use]
pub fn tighten_of(old: &DesktopPolicy, new: &DesktopPolicy) -> Option<Tightening> {
    let mut tight = Tightening {
        lockdown: new.mode.mode == Mode::Lockdown && old.mode.mode != Mode::Lockdown,
        ..Tightening::default()
    };
    let (was, now) = (&old.snapshot, &new.snapshot);
    tight.rules = now.rules.iter().filter(|r| r.then != Then::Permit && !was.rules.contains(r)).cloned().collect();
    for task in &was.tasks {
        match now.tasks.iter().find(|x| x.id == task.id) {
            None => tight.ended_tasks.push(task.id.clone()),
            Some(x) if x != task => tight.ended_tasks.push(task.id.clone()),
            Some(_) => {}
        }
    }
    let stricter_facts = [FactKind::Owner, FactKind::Repo, FactKind::Host].iter().any(|kind| {
        let (before, after) = (fact_map(&was.facts, *kind), fact_map(&now.facts, *kind));
        before.iter().any(|(k, z)| after.get(k).is_none_or(|z2| z2 > z))
    });
    let lost_ids = old.github_ids.iter().any(|(login, id)| new.github_ids.get(login) != Some(id));
    let permit_gone = was.rules.iter().any(|r| r.then == Then::Permit && !now.rules.contains(r));
    let mode_stricter = new.mode.mode != Mode::Lockdown && new.mode.mode.openness() < old.mode.mode.openness();
    let ends_sooner = new.mode.mode == old.mode.mode
        && match (old.mode.until, new.mode.until) {
            (None, Some(_)) => true,
            (Some(then), Some(sooner)) => sooner < then,
            _ => false,
        };
    let roots_gone =
        old.roots.iter().any(|r| !new.roots.contains(r)) || old.scratch.iter().any(|s| !new.scratch.contains(s));
    tight.stop_auto = (old.decides && !new.decides)
        || mode_stricter
        || (ends_sooner && new.mode.mode != Mode::Lockdown)
        || now.matrix.any_stricter_than(&was.matrix)
        || stricter_facts
        || lost_ids
        || permit_gone
        || roots_gone;
    // Coalesced: a tightening sent while Lockdown is on carries it, so the newest tightening covers the older ones'.
    if !tight.is_empty() && new.mode.mode == Mode::Lockdown {
        tight.lockdown = true;
    }
    (!tight.is_empty()).then_some(tight)
}

fn fact_map(f: &Facts, kind: FactKind) -> &BTreeMap<String, Zone> {
    match kind {
        FactKind::Owner => &f.owners,
        FactKind::Repo => &f.repos,
        FactKind::Domain => &f.domains,
        FactKind::Person => &f.people,
        FactKind::Host => &f.hosts,
        FactKind::Chat => &f.chats,
    }
}

fn fact_map_mut(f: &mut Facts, kind: FactKind) -> &mut BTreeMap<String, Zone> {
    match kind {
        FactKind::Owner => &mut f.owners,
        FactKind::Repo => &mut f.repos,
        FactKind::Domain => &mut f.domains,
        FactKind::Person => &mut f.people,
        FactKind::Host => &mut f.hosts,
        FactKind::Chat => &mut f.chats,
    }
}

/// This computer's own configuration, which can only make the signed policy stricter.
#[derive(Clone, Copy, Debug)]
pub struct Local<'a> {
    /// `[auto] facts`.
    pub facts: &'a Facts,
    /// `[auto] matrix`: tightens cell by cell when it isn't the default.
    pub matrix: &'a Matrix,
    /// `[workspace] roots`.
    pub roots: &'a [String],
    /// `[workspace] scratch`.
    pub scratch: &'a [String],
    /// The home folder, for `~`.
    pub home: &'a str,
    /// What the folders named here (local and signed) resolve to on this computer, symlinks followed (an entry missing:
    /// it doesn't resolve, or isn't known).
    pub canonical: &'a BTreeMap<String, String>,
}

/// How a request is decided here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gate {
    /// Every tool use is refused.
    Lockdown,
    /// Everything is allowed, except what a Deny rule names.
    Bypass,
    /// The matrix decides.
    Matrix {
        overnight: bool,
    },
    /// Today's flow: the guard asks the phone.
    Fallback(Fallback),
}

/// Why the computer doesn't decide.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fallback {
    NoSnapshot,
    /// The phone's matrix doesn't decide (Auto's setup isn't done, or it's switched off).
    NotDeciding,
    /// Auto is off on the phone.
    Manual,
    /// The mode's end passed: what follows is the phone's to say.
    ModeEnded,
    /// The phone tightened something the computer can't apply on its own.
    Tightened,
}

impl Fallback {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::NoSnapshot => "no snapshot from the phone",
            Self::NotDeciding => "the phone's matrix doesn't decide",
            Self::Manual => "Auto is off",
            Self::ModeEnded => "the phone's mode ended",
            Self::Tightened => "the phone tightened its rules",
        }
    }
}

/// What a request is decided with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Effective {
    pub gate: Gate,
    /// The matrix, rules, tasks and facts (with the GitHub ids set).
    pub snapshot: Snapshot,
    /// The connection the rules and tasks are for (the computer's).
    pub connection: String,
    /// Workspace roots and scratch folders that count.
    pub roots: Vec<String>,
    pub scratch: Vec<String>,
    /// The snapshot's version (0 without one).
    pub version: u64,
}

/// The forms a folder is read in on this computer: as written (`~` read as `home`) and, when it resolves, the folder
/// it resolves to (`canonical`, symlinks followed, from the computer).
fn forms(entry: &str, home: &str, canonical: &BTreeMap<String, String>) -> Vec<String> {
    let mut out = vec![normalize(entry, "/", home)];
    if let Some(c) = canonical.get(entry)
        && !out.contains(c)
    {
        out.push(c.clone());
    }
    out
}

/// A folder in any of its forms is one [`desktop_folders::refusal`] refuses (an agent pointing a root's symlink at the
/// home folder makes it as broad as the home).
fn refused(entry: &str, home: &str, canonical: &BTreeMap<String, String>) -> bool {
    forms(entry, home, canonical).iter().any(|f| desktop_folders::refusal(f, home).is_some())
}

/// The signed folders that count here, each in its forms (`~/Documents/dev` and what it resolves to), refused ones
/// left out.
fn signed_forms(signed: &[String], home: &str, canonical: &BTreeMap<String, String>) -> Vec<String> {
    signed.iter().filter(|s| !refused(s, home, canonical)).flat_map(|s| forms(s, home, canonical)).collect()
}

/// The local roots inside a signed one, in any of their forms (and none without one); refused ones never.
#[must_use]
pub fn clamp_roots(
    signed: &[String],
    local: &[String],
    home: &str,
    canonical: &BTreeMap<String, String>,
) -> Vec<String> {
    let signed = signed_forms(signed, home, canonical);
    let mut out: Vec<String> = Vec::new();
    let counts = |f: &str| signed.iter().any(|s| desktop_folders::inside(s, f, home));
    for l in local.iter().filter(|l| !refused(l, home, canonical)) {
        keep(&mut out, &forms(l, home, canonical), &counts);
    }
    out
}

/// Of a folder's forms (as written first, then what it resolves to): the written one when either counts (it is the same
/// folder), the resolved one only when it counts itself.
fn keep(out: &mut Vec<String>, forms: &[String], counts: &dyn Fn(&str) -> bool) {
    let any = forms.iter().any(|f| counts(f));
    for (i, f) in forms.iter().enumerate() {
        if ((i == 0 && any) || counts(f)) && !out.contains(f) {
            out.push(f.clone());
        }
    }
}

/// The local scratch folders that count: one the phone signed as scratch (the same folder), or strictly inside a signed
/// root (a scratch folder's deletes lose nothing, so a root itself, or anything above one, never is); refused ones
/// never.
#[must_use]
pub fn clamp_scratch(
    roots: &[String],
    signed_scratch: &[String],
    local: &[String],
    home: &str,
    canonical: &BTreeMap<String, String>,
) -> Vec<String> {
    let (roots, signed_scratch) = (signed_forms(roots, home, canonical), signed_forms(signed_scratch, home, canonical));
    let same = |a: &str, b: &str| desktop_folders::same(a, b, home);
    let mut out: Vec<String> = Vec::new();
    let counts = |f: &str| {
        signed_scratch.iter().any(|s| same(s, f))
            || roots.iter().any(|r| desktop_folders::inside(r, f, home) && !same(r, f))
    };
    for l in local.iter().filter(|l| !refused(l, home, canonical)) {
        keep(&mut out, &forms(l, home, canonical), &counts);
    }
    out
}

/// What this computer decides with at `now`: the snapshot (already checked), the tightenings newer than it (already
/// checked), the local configuration (stricter only) and the uses of budgeted tasks spent here.
#[must_use]
pub fn effective(
    policy: Option<&DesktopPolicy>,
    tightens: &[Tighten],
    local: &Local<'_>,
    spent: &BTreeMap<String, u32>,
    now: i64,
) -> Effective {
    let version = policy.map_or(0, |p| p.binding.version);
    let live: Vec<&Tightening> = tightens
        .iter()
        .filter(|t| t.binding.version > version && now < t.binding.not_after)
        .map(|t| &t.tightening)
        .collect();
    let lockdown = live.iter().any(|t| t.lockdown)
        || policy.is_some_and(|p| p.mode.mode == Mode::Lockdown && p.mode.until.is_none_or(|u| now < u));
    let mut snapshot = match policy {
        Some(p) => p.snapshot.clone(),
        None => Snapshot {
            matrix: local.matrix.clone(),
            facts: local.facts.clone(),
            ..Snapshot::default()
        },
    };
    if let Some(p) = policy {
        // Local facts: only where the signed fact exists and the local zone is stricter.
        for kind in [FactKind::Owner, FactKind::Repo, FactKind::Host] {
            let mine = fact_map(local.facts, kind).clone();
            let signed = fact_map_mut(&mut snapshot.facts, kind);
            for (k, z) in mine {
                if let Some(s) = signed.get_mut(&k)
                    && z > *s
                {
                    *s = z;
                }
            }
        }
        if *local.matrix != Matrix::default() {
            snapshot.matrix = snapshot.matrix.stricter(local.matrix);
        }
        snapshot.facts.github_ids.clone_from(&p.github_ids);
    }
    for t in &live {
        snapshot.rules.extend(t.rules.iter().filter(|r| r.then != Then::Permit).cloned());
        snapshot.tasks.retain(|task| !t.ended_tasks.contains(&task.id));
    }
    for task in &mut snapshot.tasks {
        if let (Some(left), Some(used)) = (task.uses_left, spent.get(&task.id)) {
            task.uses_left = Some(left.saturating_sub(*used));
        }
    }
    let stop = live.iter().any(|t| t.stop_auto);
    let gate = match policy {
        _ if lockdown => Gate::Lockdown,
        None => Gate::Fallback(Fallback::NoSnapshot),
        Some(_) if stop => Gate::Fallback(Fallback::Tightened),
        Some(p) if p.mode.until.is_some_and(|u| now >= u) => Gate::Fallback(Fallback::ModeEnded),
        Some(p) => match p.mode.mode {
            Mode::Lockdown => Gate::Lockdown,
            Mode::Bypass => Gate::Bypass,
            Mode::Manual => Gate::Fallback(Fallback::Manual),
            _ if !p.decides => Gate::Fallback(Fallback::NotDeciding),
            Mode::Auto => Gate::Matrix {
                overnight: false,
            },
            Mode::Overnight => Gate::Matrix {
                overnight: true,
            },
        },
    };
    let (roots, scratch) = match policy {
        Some(p) => (
            clamp_roots(&p.roots, local.roots, local.home, local.canonical),
            clamp_scratch(&p.roots, &p.scratch, local.scratch, local.home, local.canonical),
        ),
        None => (local.roots.to_vec(), local.scratch.to_vec()),
    };
    Effective {
        gate,
        snapshot,
        connection: policy.map(|p| p.binding.connection_id.clone()).unwrap_or_default(),
        roots,
        scratch,
        version,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::{Base, request_zone};
    use crate::effects::{Context, Level, Workspace, analyze_command};
    use crate::matrix::{Cell, Input, Issuer, Outcome, When, evaluate};

    const KEY: &str = "computer-key";

    fn binding(version: u64, now: i64) -> Binding {
        Binding {
            version,
            issued_at: now,
            not_after: now + VALIDITY_SECS,
            client_key: KEY.to_owned(),
            connection_id: "conn-1".to_owned(),
            profile: PERSONAL.to_owned(),
        }
    }

    fn policy(version: u64, now: i64) -> DesktopPolicy {
        let mut snapshot = Snapshot::default();
        snapshot.facts.set(FactKind::Owner, "dkat", Zone::Mine);
        snapshot.facts.set(FactKind::Owner, "acme", Zone::Work);
        DesktopPolicy {
            binding: binding(version, now),
            decides: true,
            mode: ModeState {
                mode: Mode::Auto,
                until: None,
            },
            snapshot,
            github_ids: BTreeMap::new(),
            roots: vec!["~/Documents/dev".to_owned()],
            scratch: vec!["~/.claude/projects/*/memory".to_owned()],
        }
    }

    /// A stand-in signature: the key's first byte, then a hash of the message (tests only).
    fn fake_sign(bytes: &[u8]) -> String {
        use sha2::Digest as _;
        let mut sig = vec![7u8];
        sig.extend_from_slice(&sha2::Sha256::digest(bytes));
        sig.resize(64, 0);
        data_encoding::BASE64URL_NOPAD.encode(&sig)
    }

    fn fake_verify(key: &[u8; 32], msg: &[u8], sig: &[u8; 64]) -> bool {
        use sha2::Digest as _;
        key[0] == 1 && sig[0] == 7 && sig[1..33] == sha2::Sha256::digest(msg)[..]
    }

    fn phone_key() -> String {
        let mut k = [0u8; 32];
        k[0] = 1;
        data_encoding::BASE64URL_NOPAD.encode(&k)
    }

    fn local<'a>(facts: &'a Facts, matrix: &'a Matrix, roots: &'a [String], scratch: &'a [String]) -> Local<'a> {
        Local {
            facts,
            matrix,
            roots,
            scratch,
            home: "/home/me",
            canonical: &NONE,
        }
    }

    static NONE: BTreeMap<String, String> = BTreeMap::new();

    #[test]
    fn signed_bytes_are_domain_separated() {
        assert!(signing_bytes("{}").starts_with(b"reins/desktop-policy/v1\n"));
        assert_ne!(signing_bytes("{}"), b"{}".to_vec());
    }

    #[test]
    fn open_checks_key_and_signature() {
        let m = PolicyMessage::Snapshot(Box::new(policy(3, 1_000)));
        let s = sign(&m, &phone_key(), &fake_sign);
        assert_eq!(open(&s, Some(&phone_key()), &fake_verify).as_ref(), Ok(&m));
        // No key pinned, another pinned key, a key that signed but isn't the pinned one.
        assert_eq!(open(&s, None, &fake_verify), Err(Refusal::OtherKey));
        let mut other = [0u8; 32];
        other[0] = 2;
        let other = data_encoding::BASE64URL_NOPAD.encode(&other);
        assert_eq!(open(&s, Some(&other), &fake_verify), Err(Refusal::OtherKey));
        // An edited body, an edited signature.
        let mut edited = s.clone();
        edited.body = edited.body.replace("\"decides\":true", "\"decides\":false");
        assert_ne!(edited.body, s.body);
        assert_eq!(open(&edited, Some(&phone_key()), &fake_verify), Err(Refusal::BadSignature));
        let mut bad = s.clone();
        bad.sig = "AA".to_owned();
        assert_eq!(open(&bad, Some(&phone_key()), &fake_verify), Err(Refusal::BadSignature));
        // Another format.
        let future = Signed {
            body: s.body.replacen("\"v\":1", "\"v\":2", 1),
            ..s.clone()
        };
        let future = Signed {
            sig: fake_sign(&signing_bytes(&future.body)),
            ..future
        };
        assert_eq!(open(&future, Some(&phone_key()), &fake_verify), Err(Refusal::UnknownFormat));
    }

    #[test]
    fn check_binds_key_profile_and_time() {
        let now = 10_000;
        let b = binding(1, now);
        assert_eq!(check(&b, KEY, now + 10), Ok(()));
        assert_eq!(check(&b, "another-computer", now), Err(Refusal::OtherComputer));
        let org = Binding {
            profile: "org:acme".to_owned(),
            ..b.clone()
        };
        assert_eq!(check(&org, KEY, now), Err(Refusal::OtherProfile));
        assert_eq!(check(&b, KEY, b.not_after), Err(Refusal::Expired));
        let ahead = Binding {
            issued_at: now + 600,
            not_after: now + 600 + VALIDITY_SECS,
            ..b.clone()
        };
        assert_eq!(check(&ahead, KEY, now), Err(Refusal::FromTheFuture));
        let long = Binding {
            not_after: now + 2 * MAX_VALIDITY_SECS,
            ..b
        };
        assert_eq!(check(&long, KEY, now), Err(Refusal::TooLong));
    }

    #[test]
    fn a_snapshot_is_stale_minutes_after_its_issue() {
        let now = 10_000;
        let mut p = policy(1, now);
        // Signed as valid for a day: the computer still uses it only for VALIDITY_SECS.
        p.binding.not_after = now + MAX_VALIDITY_SECS;
        let snap = PolicyMessage::Snapshot(Box::new(p.clone()));
        assert_eq!(check_message(&snap, KEY, now + VALIDITY_SECS - 1), Ok(()));
        assert_eq!(check_message(&snap, KEY, now + VALIDITY_SECS), Err(Refusal::Expired));
        let t = PolicyMessage::Tighten(Tighten {
            binding: p.binding,
            tightening: Tightening {
                lockdown: true,
                ..Tightening::default()
            },
        });
        assert_eq!(check_message(&t, KEY, now + VALIDITY_SECS), Ok(()), "a tightening holds to its own end");
    }

    #[test]
    fn desktop_facts_keep_owners_repos_hosts_only() {
        let mut f = Facts::default();
        f.set(FactKind::Owner, "acme", Zone::Work);
        f.set(FactKind::Repo, "pal/app", Zone::Friends);
        f.set(FactKind::Host, "build.local", Zone::Mine);
        f.set(FactKind::Person, "bob@example.com", Zone::Friends);
        f.set(FactKind::Domain, "acme.io", Zone::Work);
        f.set(FactKind::Chat, "123", Zone::Friends);
        f.own_addresses.insert("me@example.com".to_owned());
        f.set_label(FactKind::Person, "bob@example.com", "Bob");
        f.set_github_owner("583231", "DKat", Zone::Mine);
        f.set_github_owner("9", "twin", Zone::Mine);
        f.set_github_owner("10", "twin", Zone::Work);
        let (d, ids) = desktop_facts(&f);
        assert_eq!(d.owners.get("acme"), Some(&Zone::Work));
        assert_eq!(d.repos.get("pal/app"), Some(&Zone::Friends));
        assert_eq!(d.hosts.get("build.local"), Some(&Zone::Mine));
        assert!(d.people.is_empty() && d.domains.is_empty() && d.chats.is_empty() && d.own_addresses.is_empty());
        assert!(d.labels.keys().all(|k| !k.starts_with("person:")));
        assert_eq!(ids.get("dkat").map(String::as_str), Some("583231"));
        assert!(!ids.contains_key("twin"), "two accounts labelled alike are read by neither");
        let json = serde_json::to_string(&d).unwrap();
        assert!(!json.contains("bob@") && !json.contains("me@example.com"));
    }

    #[test]
    fn desktop_rules_keep_deny_and_review_and_impersonal_permits() {
        let rule = |id: &str, then: Then, resource: Option<&str>, connection: Option<&str>| Rule {
            id: id.to_owned(),
            issuer: Issuer::Owner,
            when: When {
                resource: resource.map(str::to_owned),
                connection: connection.map(str::to_owned),
                ..When::default()
            },
            then,
            not_after: None,
            accepted: false,
            explain: String::new(),
        };
        let rules = vec![
            rule("deny-bob", Then::Deny, Some("bob@x.io"), None),
            rule("permit-bob", Then::Permit, Some("bob@x.io"), None),
            rule("permit-repo", Then::Permit, Some("dkat/*"), None),
            rule("other-conn", Then::Review, None, Some("conn-2")),
            rule("this-conn", Then::Review, None, Some("conn-1")),
        ];
        let ids: Vec<String> = desktop_rules(&rules, "conn-1").into_iter().map(|r| r.id).collect();
        assert_eq!(ids, ["deny-bob", "permit-repo", "this-conn"]);
    }

    fn zone_and_outcome(e: &Effective, command: &str) -> (Zone, Outcome) {
        let ctx = Context::new("/home/me/Documents/dev/app", Workspace::standard("/home/me"));
        let a = analyze_command(command, &ctx);
        let r = request_zone(
            &a,
            &e.snapshot.facts,
            &Base {
                repo: Some("git@github.com:acme/app.git"),
                ..Base::default()
            },
        );
        let ev = evaluate(
            &Input {
                analysis: &a,
                resolved: &r,
                connection: &e.connection,
                seen_resource: false,
                now: 1_000,
                overnight: false,
                netted: false,
            },
            &e.snapshot,
        );
        (r.zone, ev.outcome)
    }

    #[test]
    fn local_config_only_tightens() {
        let p = policy(5, 1_000);
        // An agent's edits: Auto everywhere, acme as Mine, an unknown owner as Mine, `/` and `~` as roots and scratch.
        let mut loose = Matrix::default();
        for z in Zone::ALL {
            for l in [Level::L0, Level::L1, Level::L2, Level::L3] {
                loose.set(z, l, Cell::Auto);
            }
        }
        let mut facts = Facts::default();
        facts.set(FactKind::Owner, "acme", Zone::Mine);
        facts.set(FactKind::Owner, "evil", Zone::Mine);
        let roots = vec!["/".to_owned(), "~".to_owned(), "~/Documents/dev/app".to_owned()];
        let scratch = vec![
            "~".to_owned(),
            "~/Documents/dev".to_owned(),
            "~/Documents/dev/tmp".to_owned(),
            "~/.claude/projects/*/memory".to_owned(),
            "~/Pictures".to_owned(),
        ];
        let e = effective(Some(&p), &[], &local(&facts, &loose, &roots, &scratch), &BTreeMap::new(), 1_000);
        assert_eq!(e.snapshot.matrix, Matrix::default(), "an Auto-everywhere matrix loosens nothing");
        assert_eq!(e.snapshot.facts.get(FactKind::Owner, "acme"), Some(Zone::Work));
        assert_eq!(e.snapshot.facts.get(FactKind::Owner, "evil"), None);
        assert_eq!(e.roots, ["/home/me/Documents/dev/app"]);
        assert_eq!(
            e.scratch,
            ["/home/me/Documents/dev/tmp", "/home/me/.claude/projects/*/memory"],
            "inside a root, or signed"
        );
        assert_eq!(zone_and_outcome(&e, "git push origin HEAD:refs/heads/feat").0, Zone::Work);
        // Stricter local settings do apply: acme as Outside, a stricter cell.
        let mut strict = Matrix::default();
        strict.set(Zone::Mine, Level::L1, Cell::Ask);
        facts.set(FactKind::Owner, "acme", Zone::Outside);
        let e = effective(Some(&p), &[], &local(&facts, &strict, &roots, &scratch), &BTreeMap::new(), 1_000);
        assert_eq!(e.snapshot.facts.get(FactKind::Owner, "acme"), Some(Zone::Outside));
        assert_eq!(e.snapshot.matrix.cell(Zone::Mine, Level::L1), Cell::Ask);
        assert_eq!(e.snapshot.matrix.cell(Zone::Work, Level::L0), Cell::Auto);
    }

    /// A folder counts in the forms it has on this computer: as written, and what it resolves to. One that resolves to
    /// something too broad (an agent pointed its symlink at the home folder) counts in neither.
    #[test]
    fn folders_count_as_they_resolve() {
        let mut p = policy(5, 1_000);
        p.roots = vec!["~/Documents/dev".to_owned(), "/mnt/data/dev".to_owned(), "/".to_owned()];
        p.scratch = vec!["/home/me/.claude/projects/*/memory".to_owned(), "~".to_owned()];
        let (none, m) = (Facts::default(), Matrix::default());
        let roots = vec!["~/Documents/dev".to_owned(), "~/work".to_owned(), "~/src".to_owned()];
        let scratch = vec!["~/.claude/projects/*/memory".to_owned(), "~/Downloads".to_owned()];
        let canonical = BTreeMap::from([
            // `~/work` is a link to the signed `/mnt/data/dev`; `~/src` one an agent pointed at the home folder.
            ("~/work".to_owned(), "/mnt/data/dev".to_owned()),
            ("~/src".to_owned(), "/home/me".to_owned()),
        ]);
        let l = Local {
            canonical: &canonical,
            ..local(&none, &m, &roots, &scratch)
        };
        let e = effective(Some(&p), &[], &l, &BTreeMap::new(), 1_000);
        assert_eq!(e.roots, ["/home/me/Documents/dev", "/home/me/work", "/mnt/data/dev"]);
        assert_eq!(e.scratch, ["/home/me/.claude/projects/*/memory"], "a signed `~` is never scratch");
        // The phone signs nothing: nothing local counts.
        p.roots.clear();
        p.scratch.clear();
        let e = effective(Some(&p), &[], &l, &BTreeMap::new(), 1_000);
        assert!(e.roots.is_empty() && e.scratch.is_empty(), "{e:?}");
    }

    #[test]
    fn a_folder_removed_tightens_at_once() {
        let mut old = policy(1, 1_000);
        old.scratch.push("/home/me/.zeron/uploads".to_owned());
        assert!(tighten_of(&old, &policy(2, 1_000)).is_some_and(|t| t.stop_auto), "a scratch folder removed");
        let mut more = old.clone();
        more.scratch.push("/home/me/Downloads/tmp".to_owned());
        assert_eq!(tighten_of(&old, &more), None, "one more is looser");
    }

    #[test]
    fn the_gate_follows_the_mode() {
        let none = Facts::default();
        let m = Matrix::default();
        let l = local(&none, &m, &[], &[]);
        let gate = |p: Option<&DesktopPolicy>, now: i64| effective(p, &[], &l, &BTreeMap::new(), now).gate;
        assert_eq!(gate(None, 1_000), Gate::Fallback(Fallback::NoSnapshot));
        let mut p = policy(1, 1_000);
        assert_eq!(
            gate(Some(&p), 1_000),
            Gate::Matrix {
                overnight: false
            }
        );
        p.decides = false;
        assert_eq!(gate(Some(&p), 1_000), Gate::Fallback(Fallback::NotDeciding));
        p.mode = ModeState {
            mode: Mode::Bypass,
            until: Some(2_000),
        };
        assert_eq!(gate(Some(&p), 1_000), Gate::Bypass);
        assert_eq!(gate(Some(&p), 2_000), Gate::Fallback(Fallback::ModeEnded));
        p.decides = true;
        p.mode = ModeState {
            mode: Mode::Overnight,
            until: Some(5_000),
        };
        assert_eq!(
            gate(Some(&p), 1_000),
            Gate::Matrix {
                overnight: true
            }
        );
        p.mode.mode = Mode::Manual;
        assert_eq!(gate(Some(&p), 1_000), Gate::Fallback(Fallback::Manual));
        p.mode = ModeState {
            mode: Mode::Lockdown,
            until: None,
        };
        assert_eq!(gate(Some(&p), 1_000), Gate::Lockdown);
    }

    #[test]
    fn tightenings_apply_until_superseded() {
        let none = Facts::default();
        let m = Matrix::default();
        let l = local(&none, &m, &[], &[]);
        let p = policy(10, 1_000);
        let lockdown = Tighten {
            binding: binding(11, 1_000),
            tightening: Tightening {
                lockdown: true,
                ..Tightening::default()
            },
        };
        // Even without a snapshot.
        assert_eq!(effective(None, std::slice::from_ref(&lockdown), &l, &BTreeMap::new(), 1_000).gate, Gate::Lockdown);
        assert_eq!(
            effective(Some(&p), std::slice::from_ref(&lockdown), &l, &BTreeMap::new(), 1_000).gate,
            Gate::Lockdown
        );
        // A snapshot of the same or a later version supersedes it; an expired one is gone.
        let later = policy(11, 1_000);
        assert_ne!(
            effective(Some(&later), std::slice::from_ref(&lockdown), &l, &BTreeMap::new(), 1_000).gate,
            Gate::Lockdown
        );
        assert_ne!(
            effective(Some(&p), std::slice::from_ref(&lockdown), &l, &BTreeMap::new(), lockdown.binding.not_after).gate,
            Gate::Lockdown
        );
        // Rules add, permits never; ended tasks go; stop_auto falls back.
        let deny = Rule {
            id: "never-acme".to_owned(),
            issuer: Issuer::Owner,
            when: When {
                resource: Some("acme/*".to_owned()),
                ..When::default()
            },
            then: Then::Deny,
            not_after: None,
            accepted: false,
            explain: "Not acme".to_owned(),
        };
        let permit = Rule {
            id: "sneaky".to_owned(),
            then: Then::Permit,
            ..deny.clone()
        };
        let mut with_task = p.clone();
        with_task.snapshot.tasks.push(Task {
            id: "t1".to_owned(),
            connection: "conn-1".to_owned(),
            not_after: 9_999,
            ..Task::default()
        });
        let rules = Tighten {
            binding: binding(12, 1_000),
            tightening: Tightening {
                rules: vec![deny, permit],
                ended_tasks: vec!["t1".to_owned()],
                ..Tightening::default()
            },
        };
        let e = effective(Some(&with_task), std::slice::from_ref(&rules), &l, &BTreeMap::new(), 1_000);
        assert_eq!(e.snapshot.rules.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["never-acme"]);
        assert!(e.snapshot.tasks.is_empty());
        assert_eq!(zone_and_outcome(&e, "git push origin HEAD:refs/heads/feat").1, Outcome::Deny);
        let stop = Tighten {
            binding: binding(13, 1_000),
            tightening: Tightening {
                stop_auto: true,
                ..Tightening::default()
            },
        };
        assert_eq!(
            effective(Some(&p), std::slice::from_ref(&stop), &l, &BTreeMap::new(), 1_000).gate,
            Gate::Fallback(Fallback::Tightened)
        );
    }

    #[test]
    fn tighten_of_finds_lockdown_rules_ended_tasks_and_stricter_cells() {
        let old = policy(1, 1_000);
        assert_eq!(tighten_of(&old, &policy(2, 1_000)), None, "nothing changed");
        // Loosening: a cell to Auto, a new fact, a new task, the mode to Bypass: nothing to tighten.
        let mut looser = policy(2, 1_000);
        looser.snapshot.matrix.set(Zone::Work, Level::L1, Cell::Auto);
        looser.snapshot.facts.set(FactKind::Owner, "pal", Zone::Friends);
        looser.mode.mode = Mode::Bypass;
        assert_eq!(tighten_of(&old, &looser), None);
        let mut locked = policy(2, 1_000);
        locked.mode.mode = Mode::Lockdown;
        assert!(tighten_of(&old, &locked).is_some_and(|t| t.lockdown && !t.stop_auto));
        let mut cell = policy(2, 1_000);
        cell.snapshot.matrix.set(Zone::Mine, Level::L0, Cell::Ask);
        assert!(tighten_of(&old, &cell).is_some_and(|t| t.stop_auto));
        let mut fact = policy(2, 1_000);
        fact.snapshot.facts.set(FactKind::Owner, "dkat", Zone::Friends);
        assert!(tighten_of(&old, &fact).is_some_and(|t| t.stop_auto));
        let mut off = policy(2, 1_000);
        off.decides = false;
        assert!(tighten_of(&old, &off).is_some_and(|t| t.stop_auto));
        let mut manual = policy(2, 1_000);
        manual.mode.mode = Mode::Manual;
        assert!(tighten_of(&old, &manual).is_some_and(|t| t.stop_auto));
        let mut rule = policy(2, 1_000);
        rule.snapshot.rules.push(Rule {
            id: "review-me".to_owned(),
            issuer: Issuer::Owner,
            when: When::default(),
            then: Then::Review,
            not_after: None,
            accepted: false,
            explain: String::new(),
        });
        assert!(tighten_of(&old, &rule).is_some_and(|t| t.rules.len() == 1 && !t.stop_auto));
        // Under Lockdown, any later tightening carries it too.
        let (mut was_locked, mut still_locked) = (locked, rule.clone());
        was_locked.mode.mode = Mode::Lockdown;
        still_locked.mode.mode = Mode::Lockdown;
        assert!(tighten_of(&was_locked, &still_locked).is_some_and(|t| t.lockdown && t.rules.len() == 1));
        let mut tasked = old.clone();
        tasked.snapshot.tasks.push(Task {
            id: "t1".to_owned(),
            connection: "conn-1".to_owned(),
            not_after: 9_999,
            ..Task::default()
        });
        assert_eq!(tighten_of(&old, &tasked), None, "a new task loosens");
        assert!(tighten_of(&tasked, &policy(2, 1_000)).is_some_and(|t| t.ended_tasks == ["t1"]));
    }

    #[test]
    fn budgeted_tasks_spend_locally() {
        let none = Facts::default();
        let m = Matrix::default();
        let mut p = policy(1, 1_000);
        p.snapshot.tasks.push(Task {
            id: "t1".to_owned(),
            connection: "conn-1".to_owned(),
            uses_left: Some(3),
            not_after: 9_999,
            ..Task::default()
        });
        let spent = BTreeMap::from([("t1".to_owned(), 5)]);
        let e = effective(Some(&p), &[], &local(&none, &m, &[], &[]), &spent, 1_000);
        assert_eq!(e.snapshot.tasks[0].uses_left, Some(0));
        assert_eq!(e.connection, "conn-1");
    }

    #[test]
    fn always_asked_groups() {
        assert!(crate::guard::always_asked(&["cat ~/.local/state/reins/policy.json"]).iter().any(|g| g.id == "reins"));
        assert!(crate::guard::always_asked(&["vim ~/.local/state/reins/tickets.json"]).iter().any(|g| g.id == "reins"));
        assert!(crate::guard::always_asked(&["rm -rf target"]).is_empty());
        assert!(crate::guard::always_asked(&["git push --force origin feat"]).is_empty());
    }

    #[test]
    fn messages_round_trip_through_json() {
        let m = PolicyMessage::Tighten(Tighten {
            binding: binding(4, 1_000),
            tightening: Tightening {
                lockdown: true,
                ..Tightening::default()
            },
        });
        let s = sign(&m, &phone_key(), &fake_sign);
        assert!(s.body.contains("\"kind\":\"tighten\""), "{}", s.body);
        assert_eq!(open(&s, Some(&phone_key()), &fake_verify), Ok(m));
    }
}
