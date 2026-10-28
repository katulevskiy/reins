//! The phone-signed policy on this computer (Decisions v2, `docs/design/decisions-v2/08-review-resolutions.md` R1;
//! `reins_policy::desktop_policy`). The phone signs the owner's Auto rules for this computer with its device key and
//! seals them to this app's key; the server relays them. The daemon pulls them on start and long-polls for news; a
//! hook decides with what is kept here:
//!
//! - every message is checked against the phone's signing key pinned with the tickets (P4,
//!   [`crate::tickets::TicketStore::signing_key`]), for this app's key and the Personal profile;
//! - **freshness is the rollback protection:** a snapshot is used only within
//!   [`reins_policy::desktop_policy::VALIDITY_SECS`] of its signed issue time, by this computer's clock. The phone signs
//!   a new one every few minutes while it is online and at once on any change; an older one replayed (by the server,
//!   or a file put back) is stale within minutes, and then the phone is asked;
//! - within that window a newer version wins (the daemon also remembers, in memory, the newest version each phone key
//!   signed, and never takes an older one);
//! - a tightening applies on top of whatever is kept, a snapshot or none, until a snapshot as new arrives;
//! - logged out, nothing kept is used;
//! - what is kept is re-checked on every read: an edited file decides nothing. The signer is only ever the key pinned
//!   in `tickets.json`; nothing in `policy.json` names one.
//!
//! `state_dir/policy.json` is in the `reins` guard group (an agent changing it is asked about), and so is
//! `tickets.json`, which holds the pinned key. For shell commands Reins stays a guardrail, not a boundary (08 R1): an
//! agent running as the same user could skip the hooks altogether.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use reins_policy::desktop_policy::{self as dp, DesktopPolicy, PolicyMessage, Refusal, Signed, Tighten};
use reins_proto::desktop::PolicyMail;
use serde::{Deserialize, Serialize};

use crate::config::{Config, Paths, write_private};
use crate::identity::Identity;
use crate::server::LinkError;
use crate::server::client::DesktopClient;
use crate::tickets::TicketStore;

/// How long one pick-up waits on the server for news.
pub const POLL_WAIT: Duration = Duration::from_secs(reins_proto::desktop::POLICY_WAIT_MAX_SECS);
/// After a failure (offline, the server restarting), before trying again.
const RETRY: Duration = Duration::from_secs(30);
/// A server too old to know the policy is asked again this much later.
const TOO_OLD_RETRY: Duration = Duration::from_mins(15);
/// How long a hook waits for the server when no daemon keeps the policy fresh.
pub const HOOK_LOOK: Duration = Duration::from_millis(1_500);
/// A hook looks itself at most this often.
const HOOK_LOOK_EVERY: i64 = 60;
/// A snapshot that goes stale within this is looked at again by a hook.
const ENDS_SOON: i64 = 120;

/// Strict Ed25519 over `msg`.
#[must_use]
pub fn verify(key: &[u8; 32], msg: &[u8], sig: &[u8; 64]) -> bool {
    use ring::signature::{ED25519, UnparsedPublicKey};
    reins_proto::canon::strict_ed25519_key(key) && UnparsedPublicKey::new(&ED25519, key).verify(msg, sig).is_ok()
}

/// `state_dir/policy.json`: the signed messages as they came (re-checked on every read), and the uses of budgeted
/// tasks spent here. Unknown fields (a `signer` an agent adds) are ignored.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct PolicyStore {
    #[serde(skip)]
    file: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    snapshot: Option<Signed>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tightens: Vec<Signed>,
    /// Uses of budgeted tasks spent on this computer (the phone's `uses_left` doesn't count them).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    spent: BTreeMap<String, u32>,
    /// Unix seconds: when the server was last asked.
    #[serde(default)]
    looked_at: i64,
}

fn store_file(paths: &Paths) -> PathBuf {
    paths.state_dir.join("policy.json")
}

/// The version a signed message says it is (unchecked: for ordering what is kept only).
fn version_of(signed: &Signed) -> u64 {
    serde_json::from_str::<serde_json::Value>(&signed.body)
        .ok()
        .and_then(|v| v.pointer("/binding/version").and_then(serde_json::Value::as_u64))
        .unwrap_or(0)
}

impl PolicyStore {
    /// What is kept (nothing when the file is missing or unreadable: then the phone is asked, as before).
    #[must_use]
    pub fn load(paths: &Paths) -> Self {
        let file = store_file(paths);
        let mut store: Self =
            std::fs::read(&file).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        store.file = file;
        store
    }

    fn save(&self) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        write_private(&self.file, &bytes).map_err(|e| format!("{}: {e}", self.file.display()))
    }

    /// Loads, changes with `f` and saves, under a lock.
    pub fn update<R>(paths: &Paths, f: impl FnOnce(&mut Self) -> R) -> Result<R, String> {
        paths.ensure().map_err(|e| e.to_string())?;
        let lock_file = paths.state_dir.join("policy.lock");
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

    /// The highest version kept (what the server is asked for news after).
    #[must_use]
    pub fn newest(&self) -> u64 {
        self.snapshot.iter().chain(&self.tightens).map(version_of).max().unwrap_or(0)
    }
}

/// The pinned signing key of the phone (P4), and whether it is in dispute (the server named another one since).
fn pinned(paths: &Paths) -> (Option<String>, bool) {
    let t = TicketStore::load(paths);
    (t.signing_key().map(str::to_owned), t.key_change().is_some())
}

/// The message in `signed`, checked: signed by `key`, for `client_key` and the Personal profile, fresh at `now`.
fn open(signed: &Signed, key: Option<&str>, client_key: &str, now: i64) -> Result<PolicyMessage, Refusal> {
    let m = dp::open(signed, key, &verify)?;
    dp::check_message(&m, client_key, now)?;
    Ok(m)
}

/// What the computer holds now, checked.
#[derive(Clone, Debug, Default)]
pub struct Held {
    pub policy: Option<DesktopPolicy>,
    pub tightens: Vec<Tighten>,
    pub spent: BTreeMap<String, u32>,
    /// Why a kept snapshot isn't used.
    pub refused: Option<Refusal>,
    /// The phone's key changed and isn't trusted yet: no snapshot is used.
    pub key_changed: bool,
    pub looked_at: i64,
}

/// What is kept for this app (`client_key`), checked at `now`. A snapshot is used only while the pinned key is not in
/// dispute; tightenings signed by the pinned key apply either way (they only ever tighten).
#[must_use]
pub fn held(paths: &Paths, client_key: &str, now: i64) -> Held {
    let store = PolicyStore::load(paths);
    let (key, disputed) = pinned(paths);
    let mut refused = None;
    let policy = match store.snapshot.as_ref().map(|s| open(s, key.as_deref(), client_key, now)) {
        _ if disputed => None,
        Some(Ok(PolicyMessage::Snapshot(p))) => Some(*p),
        Some(Ok(_)) => {
            refused = Some(Refusal::Unreadable);
            None
        }
        Some(Err(r)) => {
            refused = Some(r);
            None
        }
        None => None,
    };
    let tightens = store
        .tightens
        .iter()
        .filter_map(|s| match open(s, key.as_deref(), client_key, now) {
            Ok(PolicyMessage::Tighten(t)) => Some(t),
            _ => None,
        })
        .collect();
    Held {
        policy,
        tightens,
        spent: store.spent,
        refused,
        key_changed: disputed,
        looked_at: store.looked_at,
    }
}

/// What a pick-up changed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Applied {
    pub snapshots: usize,
    pub tightens: usize,
    pub refused: Vec<Refusal>,
    /// The highest version the server sent (refused ones too), to ask after next time.
    pub seen: u64,
    /// The newest snapshot version applied or kept, signed by the pinned key (for the daemon's memory).
    pub newest: u64,
}

/// Applies what the server sent: pins the phone's signing key the first time one is named (P4), opens each message
/// with this app's key, and keeps what is signed by the pinned key, for this app and profile, fresh and newer than
/// both what is kept and `memory` (the newest the daemon saw from that key; 0 for a hook).
pub fn apply(paths: &Paths, identity: &Identity, mail: &PolicyMail, now: i64, memory: u64) -> Result<Applied, String> {
    if let Some(k) = &mail.signing_key {
        TicketStore::update(paths, |s| s.see_signing_key(k))?;
    }
    let (key, disputed) = pinned(paths);
    let client_key = identity.public_key();
    PolicyStore::update(paths, |store| {
        let mut out = Applied {
            seen: mail.items.iter().map(|i| i.version).max().unwrap_or(0),
            ..Applied::default()
        };
        store.looked_at = now;
        let open = |s: &Signed| open(s, key.as_deref(), &client_key, now);
        // What is kept counts only while it opens with the pinned key (another phone's, or an edited one, doesn't).
        let mut kept = store.snapshot.as_ref().map(open).and_then(Result::ok).map_or(0, |m| m.version());
        if kept == 0 {
            store.snapshot = None;
        }
        for item in &mail.items {
            let signed: Option<Signed> =
                identity.unseal(&item.sealed).ok().and_then(|plain| serde_json::from_slice(&plain).ok());
            let Some(signed) = signed else {
                out.refused.push(Refusal::Unreadable);
                continue;
            };
            match open(&signed) {
                Ok(PolicyMessage::Snapshot(p)) if !disputed && p.binding.version > kept.max(memory) => {
                    kept = p.binding.version;
                    store.snapshot = Some(signed);
                    out.snapshots += 1;
                }
                // A tightening only tightens: taken as it comes, until a snapshot as new supersedes it.
                Ok(PolicyMessage::Tighten(_)) => {
                    if !store.tightens.iter().any(|s| s.body == signed.body) {
                        store.tightens.push(signed);
                        out.tightens += 1;
                    }
                }
                Ok(_) => out.refused.push(Refusal::Older),
                Err(r) => out.refused.push(r),
            }
        }
        // Tightenings the kept snapshot superseded, or that no longer open, go; the newest are kept.
        store.tightens.retain(|s| matches!(open(s), Ok(PolicyMessage::Tighten(t)) if t.binding.version > kept));
        store.tightens.sort_by_key(|s| std::cmp::Reverse(version_of(s)));
        store.tightens.truncate(reins_proto::desktop::MAX_TIGHTENS);
        out.newest = kept;
        out
    })
}

/// Counts one use of budgeted task `id` spent here.
pub fn spend(paths: &Paths, id: &str) {
    if let Err(e) = PolicyStore::update(paths, |s| *s.spent.entry(id.to_owned()).or_default() += 1) {
        log::warn!("policy: {e}");
    }
}

/// Forgets everything kept (logging out).
pub fn forget(paths: &Paths) {
    let file = store_file(paths);
    if let Err(e) = std::fs::remove_file(&file)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        log::warn!("{}: {e}", file.display());
    }
}

/// Asks the server for what is newer than `after`, waiting up to `wait`, and applies it (`memory`: see [`apply`]).
pub async fn pull(
    paths: &Paths,
    identity: &Identity,
    client: &DesktopClient,
    after: u64,
    wait: Duration,
    memory: u64,
) -> Result<Applied, LinkError> {
    let mail = client.policy(after, wait).await?;
    let applied = apply(paths, identity, &mail, crate::now_unix(), memory).map_err(LinkError::Failed)?;
    for r in &applied.refused {
        log::info!("policy: a message from the server was not used: {}", r.label());
    }
    if applied.snapshots + applied.tightens > 0 {
        log::info!("policy: {} snapshot(s), {} tightening(s) from the phone", applied.snapshots, applied.tightens);
    }
    Ok(applied)
}

/// The daemon's pick-up: once on start, then a long poll, so a Lockdown or another tightening arrives within seconds.
/// It remembers, per phone key, the newest snapshot version it applied (memory only: never read back from a file), and
/// takes nothing older from that key.
pub async fn puller(paths: Paths, identity: std::sync::Arc<Identity>) {
    let mut newest: BTreeMap<String, u64> = BTreeMap::new();
    let mut seen = 0u64;
    let mut signer: Option<String> = None;
    let mut wait = Duration::ZERO;
    loop {
        if crate::server::oauth::logged_in_server(&paths).is_none() {
            tokio::time::sleep(RETRY).await;
            continue;
        }
        let client = match DesktopClient::new(&paths) {
            Ok(c) => c,
            Err(e) => {
                log::warn!("policy: {e}");
                tokio::time::sleep(RETRY).await;
                continue;
            }
        };
        // Another phone: the old one's versions say nothing about the new one's.
        let now_signer = pinned(&paths).0;
        if now_signer != signer {
            seen = 0;
            signer = now_signer;
        }
        let memory = signer.as_ref().and_then(|k| newest.get(k)).copied().unwrap_or(0);
        let after = PolicyStore::load(&paths).newest().max(seen);
        let started = std::time::Instant::now();
        match pull(&paths, &identity, &client, after, wait, memory).await {
            Ok(a) => {
                seen = seen.max(a.seen);
                if let Some(k) = &signer {
                    let n = newest.entry(k.clone()).or_default();
                    *n = (*n).max(a.newest);
                }
                wait = POLL_WAIT;
                // A server that answers at once again and again (nothing new, or only what is refused): not a hot loop.
                if started.elapsed() < Duration::from_secs(1) && a.snapshots + a.tightens == 0 {
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
            }
            Err(LinkError::NotFound) => tokio::time::sleep(TOO_OLD_RETRY).await,
            Err(e) => {
                log::info!("policy: {e}");
                tokio::time::sleep(RETRY).await;
            }
        }
    }
}

/// When a fresh snapshot stops being used (the earlier of its signed end and [`dp::VALIDITY_SECS`] after its issue).
fn fresh_until(p: &DesktopPolicy) -> i64 {
    p.binding.not_after.min(p.binding.issued_at.saturating_add(dp::VALIDITY_SECS))
}

/// Before a hook decides, when no daemon keeps the policy fresh: one quick look ([`HOOK_LOOK`]) if nothing usable is
/// kept (or it goes stale soon) and no look was made lately.
pub async fn look_if_stale(paths: &Paths, config: &Config) {
    let Ok(identity) = Identity::load_or_create(&paths.identity_file()) else {
        return;
    };
    let now = crate::now_unix();
    let h = held(paths, &identity.public_key(), now);
    let fresh = h.policy.as_ref().is_some_and(|p| fresh_until(p) - now > ENDS_SOON);
    if fresh || now - h.looked_at < HOOK_LOOK_EVERY || crate::tickets::daemon_running(paths, config).await {
        return;
    }
    let Ok(client) = DesktopClient::new(paths) else {
        return;
    };
    let after = PolicyStore::load(paths).newest();
    match tokio::time::timeout(HOOK_LOOK, pull(paths, &identity, &client, after, Duration::ZERO, 0)).await {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => log::info!("policy: {e}"),
        Err(_) => {
            log::info!("policy: no answer within {} ms", HOOK_LOOK.as_millis());
            if let Err(e) = PolicyStore::update(paths, |s| s.looked_at = now) {
                log::warn!("policy: {e}");
            }
        }
    }
}

/// What a hook decides with now: the kept snapshot and tightenings, with this computer's configuration (stricter only).
/// Logged out, nothing kept is used.
#[must_use]
pub fn effective(paths: &Paths, config: &Config, now: i64) -> (dp::Effective, Held) {
    let home = crate::config::home_dir().map(|h| h.to_string_lossy().into_owned()).unwrap_or_default();
    let h = if crate::server::oauth::logged_in_server(paths).is_none() {
        Held::default()
    } else {
        match Identity::load_or_create(&paths.identity_file()) {
            Ok(identity) => held(paths, &identity.public_key(), now),
            Err(e) => {
                log::warn!("policy: {e}");
                Held::default()
            }
        }
    };
    let local = dp::Local {
        facts: &config.auto.facts,
        matrix: &config.auto.matrix,
        roots: &config.workspace.roots,
        scratch: &config.workspace.scratch,
        home: &home,
    };
    (dp::effective(h.policy.as_ref(), &h.tightens, &local, &h.spent, now), h)
}

/// For `reins status`: where the computer's Auto rules come from.
#[must_use]
pub fn status_line(paths: &Paths, config: &Config, now: i64) -> String {
    let (e, h) = effective(paths, config, now);
    let until = |p: &DesktopPolicy| format!("{} min", (fresh_until(p) - now).max(0).div_euclid(60));
    match (e.gate, &h.policy) {
        (dp::Gate::Lockdown, _) => "Lockdown, from the phone".to_owned(),
        (dp::Gate::Bypass, Some(p)) => format!("Bypass, from the phone (fresh for {})", until(p)),
        (
            dp::Gate::Matrix {
                overnight,
            },
            Some(p),
        ) => format!(
            "the phone's{} (version {}, fresh for {})",
            if overnight {
                ", Overnight"
            } else {
                ""
            },
            p.binding.version,
            until(p)
        ),
        (dp::Gate::Fallback(why), _) => {
            let why = match (h.key_changed, h.refused) {
                (true, _) => "the phone's key changed: run `reins tickets`",
                (false, Some(r)) => r.label(),
                (false, None) => why.label(),
            };
            format!("asks the phone ({why})")
        }
        _ => "asks the phone".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reins_policy::context::{FactKind, Zone};
    use reins_policy::desktop_policy::{Binding, ModeState, Tightening};
    use reins_proto::autopilot::Mode;
    use reins_proto::desktop::{MailedPolicy, PolicyKind};
    use ring::signature::{Ed25519KeyPair, KeyPair as _};

    pub(crate) struct Phone {
        pair: Ed25519KeyPair,
    }

    impl Phone {
        pub(crate) fn new() -> Self {
            let rng = ring::rand::SystemRandom::new();
            let doc = Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
            Self {
                pair: Ed25519KeyPair::from_pkcs8(doc.as_ref()).unwrap(),
            }
        }

        pub(crate) fn key(&self) -> String {
            data_encoding::BASE64URL_NOPAD.encode(self.pair.public_key().as_ref())
        }

        pub(crate) fn signed(&self, m: &PolicyMessage) -> Signed {
            dp::sign(m, &self.key(), &|b| data_encoding::BASE64URL_NOPAD.encode(self.pair.sign(b).as_ref()))
        }

        pub(crate) fn sealed(&self, to: &str, m: &PolicyMessage) -> MailedPolicy {
            MailedPolicy {
                version: m.version(),
                kind: match m {
                    PolicyMessage::Snapshot(_) => PolicyKind::Snapshot,
                    PolicyMessage::Tighten(_) => PolicyKind::Tighten,
                },
                sealed: crate::identity::seal_to(to, &serde_json::to_vec(&self.signed(m)).unwrap()).unwrap(),
            }
        }
    }

    pub(crate) fn snapshot(to: &str, version: u64, now: i64) -> PolicyMessage {
        let mut snapshot = reins_policy::matrix::Snapshot::default();
        snapshot.facts.set(FactKind::Owner, "dkat", Zone::Mine);
        PolicyMessage::Snapshot(Box::new(DesktopPolicy {
            binding: Binding {
                version,
                issued_at: now,
                not_after: now + dp::VALIDITY_SECS,
                client_key: to.to_owned(),
                connection_id: "conn-1".to_owned(),
                profile: dp::PERSONAL.to_owned(),
            },
            decides: true,
            mode: ModeState {
                mode: Mode::Auto,
                until: None,
            },
            snapshot,
            github_ids: BTreeMap::new(),
            roots: vec!["~/Documents/dev".to_owned()],
            scratch: Vec::new(),
        }))
    }

    fn lockdown(to: &str, version: u64, now: i64) -> PolicyMessage {
        PolicyMessage::Tighten(Tighten {
            binding: Binding {
                version,
                issued_at: now,
                not_after: now + dp::TIGHTEN_SECS,
                client_key: to.to_owned(),
                connection_id: "conn-1".to_owned(),
                profile: dp::PERSONAL.to_owned(),
            },
            tightening: Tightening {
                lockdown: true,
                ..Tightening::default()
            },
        })
    }

    fn setup() -> (tempfile::TempDir, Paths, Identity, Phone) {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        paths.ensure().unwrap();
        let identity = Identity::load_or_create(&paths.identity_file()).unwrap();
        (dir, paths, identity, Phone::new())
    }

    /// A session, as `reins login` leaves it.
    fn log_in(paths: &Paths) {
        let session = serde_json::json!({"server": "https://reins.example", "client_id": "c",
            "token_endpoint": "https://reins.example/reins/oauth/token", "access_token": "a",
            "access_expires_at": NOW + 3600, "refresh_token": "r"});
        write_private(&paths.session_file(), session.to_string().as_bytes()).unwrap();
        assert!(crate::server::oauth::logged_in_server(paths).is_some());
    }

    fn mail(phone: &Phone, items: Vec<MailedPolicy>) -> PolicyMail {
        PolicyMail {
            items,
            signing_key: Some(phone.key()),
        }
    }

    fn give(paths: &Paths, id: &Identity, phone: &Phone, m: &PolicyMessage, now: i64) -> Applied {
        apply(paths, id, &mail(phone, vec![phone.sealed(&id.public_key(), m)]), now, 0).unwrap()
    }

    const NOW: i64 = 1_800_000_000;

    #[test]
    fn a_signed_snapshot_is_kept_and_used() {
        let (_d, paths, id, phone) = setup();
        let me = id.public_key();
        assert_eq!(give(&paths, &id, &phone, &snapshot(&me, 5, NOW), NOW).snapshots, 1);
        let h = held(&paths, &me, NOW + 60);
        assert_eq!(h.policy.map(|p| p.binding.version), Some(5));
        assert_eq!(TicketStore::load(&paths).signing_key(), Some(phone.key().as_str()), "pinned on first sight");
    }

    #[test]
    fn older_versions_are_refused() {
        let (_d, paths, id, phone) = setup();
        let me = id.public_key();
        let newer = phone.sealed(&me, &snapshot(&me, 9, NOW));
        let older = phone.sealed(&me, &snapshot(&me, 8, NOW));
        apply(&paths, &id, &mail(&phone, vec![newer.clone()]), NOW, 0).unwrap();
        let a = apply(&paths, &id, &mail(&phone, vec![older.clone(), newer]), NOW, 0).unwrap();
        assert_eq!((a.snapshots, a.refused), (0, vec![Refusal::Older, Refusal::Older]));
        assert_eq!(held(&paths, &me, NOW).policy.map(|p| p.binding.version), Some(9));
        // The daemon remembers the newest it applied: with the file emptied, an older one is still refused.
        forget(&paths);
        let a = apply(&paths, &id, &mail(&phone, vec![older]), NOW, 9).unwrap();
        assert_eq!(a.refused, [Refusal::Older]);
    }

    /// Freshness is the rollback protection: a snapshot put back (or replayed by the server) after its few minutes is
    /// refused, whatever its signed `not_after` says.
    #[test]
    fn a_replayed_old_snapshot_is_stale() {
        let (_d, paths, id, phone) = setup();
        let me = id.public_key();
        let mut old = snapshot(&me, 3, NOW);
        if let PolicyMessage::Snapshot(p) = &mut old {
            p.mode.mode = Mode::Bypass;
            p.binding.not_after = NOW + dp::MAX_VALIDITY_SECS;
        }
        give(&paths, &id, &phone, &old, NOW);
        let kept = std::fs::read(store_file(&paths)).unwrap();
        give(&paths, &id, &phone, &snapshot(&me, 4, NOW + 300), NOW + 300);
        // Later: the old file put back.
        let later = NOW + dp::VALIDITY_SECS + 1;
        std::fs::write(store_file(&paths), &kept).unwrap();
        let h = held(&paths, &me, later);
        assert_eq!((h.policy.is_none(), h.refused), (true, Some(Refusal::Expired)));
        // The server replaying it: refused as well.
        let a = give(&paths, &id, &phone, &old, later);
        assert_eq!(a.refused, [Refusal::Expired]);
    }

    /// Nothing in `policy.json` names the signer: a `signer` field an agent adds, with a snapshot it signed, changes
    /// nothing.
    #[test]
    fn a_tampered_signer_is_ignored() {
        let (_d, paths, id, phone) = setup();
        let me = id.public_key();
        give(&paths, &id, &phone, &snapshot(&me, 1, NOW), NOW);
        let agent = Phone::new();
        let mut forged = snapshot(&me, 2, NOW);
        if let PolicyMessage::Snapshot(p) = &mut forged {
            p.mode.mode = Mode::Bypass;
        }
        let store = serde_json::json!({"snapshot": agent.signed(&forged), "signer": agent.key(), "floor": 0});
        std::fs::write(store_file(&paths), store.to_string()).unwrap();
        let h = held(&paths, &me, NOW);
        assert_eq!((h.policy.is_none(), h.refused), (true, Some(Refusal::OtherKey)));
    }

    #[test]
    fn logged_out_nothing_kept_is_used() {
        let (_d, paths, id, phone) = setup();
        let me = id.public_key();
        let now = crate::now_unix();
        give(&paths, &id, &phone, &snapshot(&me, 1, now), now);
        let config = Config::default();
        assert_eq!(effective(&paths, &config, now).0.gate, dp::Gate::Fallback(dp::Fallback::NoSnapshot));
        log_in(&paths);
        assert_eq!(
            effective(&paths, &config, now).0.gate,
            dp::Gate::Matrix {
                overnight: false
            }
        );
    }

    #[test]
    fn another_key_is_refused() {
        let (_d, paths, id, phone) = setup();
        let me = id.public_key();
        give(&paths, &id, &phone, &snapshot(&me, 1, NOW), NOW);
        // The server swaps in its own key: named as the phone's, and signing a looser snapshot.
        let server = Phone::new();
        assert_eq!(give(&paths, &id, &server, &snapshot(&me, 2, NOW), NOW).snapshots, 0);
        // The changed key is held back (P4): no snapshot is used until the user compares and trusts it.
        let h = held(&paths, &me, NOW);
        assert!(h.key_changed && h.policy.is_none());
        // Sealed by anyone to this app's key, signed by a key not pinned: refused.
        let a = apply(
            &paths,
            &id,
            &PolicyMail {
                items: vec![server.sealed(&me, &snapshot(&me, 3, NOW))],
                signing_key: None,
            },
            NOW,
            0,
        )
        .unwrap();
        assert_eq!(a.refused, [Refusal::OtherKey]);
    }

    /// A new phone (re-paired, a new key trusted) starts from its own versions: the old phone's never block it.
    #[test]
    fn a_new_phone_starts_afresh() {
        let (_d, paths, id, old_phone) = setup();
        let me = id.public_key();
        give(&paths, &id, &old_phone, &snapshot(&me, 900, NOW), NOW);
        let new_phone = Phone::new();
        TicketStore::update(&paths, |s| {
            s.see_signing_key(&new_phone.key());
            s.trust_key()
        })
        .unwrap();
        assert_eq!(give(&paths, &id, &new_phone, &snapshot(&me, 1, NOW), NOW).snapshots, 1);
        assert_eq!(held(&paths, &me, NOW).policy.map(|p| p.binding.version), Some(1));
    }

    #[test]
    fn other_profile_or_computer_is_refused() {
        let (_d, paths, id, phone) = setup();
        let me = id.public_key();
        let mut work = snapshot(&me, 4, NOW);
        if let PolicyMessage::Snapshot(p) = &mut work {
            p.binding.profile = "org:acme".to_owned();
        }
        let other = Identity::generate().public_key();
        let a = apply(
            &paths,
            &id,
            &mail(
                &phone,
                vec![
                    phone.sealed(&me, &work),
                    // Sealed to this app, but signed for another computer's key.
                    phone.sealed(&me, &snapshot(&other, 5, NOW)),
                    // Sealed to another computer: this app can't open it.
                    phone.sealed(&other, &snapshot(&other, 6, NOW)),
                ],
            ),
            NOW,
            0,
        )
        .unwrap();
        assert_eq!(a.refused, [Refusal::OtherProfile, Refusal::OtherComputer, Refusal::Unreadable]);
        assert!(held(&paths, &me, NOW).policy.is_none());
    }

    #[test]
    fn stale_or_edited_falls_back() {
        let (_d, paths, id, phone) = setup();
        let me = id.public_key();
        give(&paths, &id, &phone, &snapshot(&me, 1, NOW), NOW);
        assert!(held(&paths, &me, NOW + dp::VALIDITY_SECS - 1).policy.is_some());
        let h = held(&paths, &me, NOW + dp::VALIDITY_SECS);
        assert_eq!((h.policy.is_none(), h.refused), (true, Some(Refusal::Expired)));
        // An agent edits the kept snapshot (Bypass, say): it no longer verifies.
        let file = store_file(&paths);
        let text = std::fs::read_to_string(&file).unwrap();
        std::fs::write(&file, text.replace("auto", "bypass")).unwrap();
        let h = held(&paths, &me, NOW);
        assert_eq!((h.policy.is_none(), h.refused), (true, Some(Refusal::BadSignature)));
    }

    #[test]
    fn a_lockdown_applies_at_once_with_or_without_a_snapshot() {
        let (_d, paths, id, phone) = setup();
        log_in(&paths);
        let me = id.public_key();
        let now = crate::now_unix();
        let config = Config::default();
        // Without a snapshot.
        give(&paths, &id, &phone, &lockdown(&me, 7, now), now);
        assert_eq!(effective(&paths, &config, now).0.gate, dp::Gate::Lockdown);
        // Superseded by a snapshot as new.
        give(&paths, &id, &phone, &snapshot(&me, 8, now), now);
        assert_ne!(effective(&paths, &config, now).0.gate, dp::Gate::Lockdown);
        assert!(PolicyStore::load(&paths).tightens.is_empty(), "superseded tightenings go");
        // On top of a fresh snapshot, even from a version below what the daemon remembers.
        let a = apply(&paths, &id, &mail(&phone, vec![phone.sealed(&me, &lockdown(&me, 9, now))]), now, 1_000).unwrap();
        assert_eq!(a.tightens, 1);
        assert_eq!(effective(&paths, &config, now).0.gate, dp::Gate::Lockdown);
    }

    #[test]
    fn spent_uses_are_counted_here() {
        let (_d, paths, _id, _phone) = setup();
        spend(&paths, "t1");
        spend(&paths, "t1");
        assert_eq!(PolicyStore::load(&paths).spent.get("t1"), Some(&2));
    }
}
