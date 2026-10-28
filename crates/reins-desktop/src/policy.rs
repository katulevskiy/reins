//! The phone-signed policy on this computer (Decisions v2, `docs/design/decisions-v2/08-review-resolutions.md` R1;
//! `reins_policy::desktop_policy`). The phone signs the owner's Auto rules for this computer with its device key and
//! seals them to this app's key; the server relays them. The daemon pulls them on start and long-polls for news; a
//! hook decides with what is kept here:
//!
//! - every message is checked against the phone's signing key pinned with the tickets (P4,
//!   [`crate::tickets::TicketStore::signing_key`]), for this app's key and the Personal profile, within its validity;
//! - a snapshot replaces the one kept only when its version is higher (no rollback by a server or an old copy);
//! - a tightening applies on top of whatever is kept, a snapshot or none, until a snapshot as new arrives;
//! - what is kept is re-checked on every read: an edited file decides nothing.
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
const HOOK_LOOK_EVERY: i64 = 300;
/// A snapshot that ends within this is looked at again by a hook.
const ENDS_SOON: i64 = 600;

/// Strict Ed25519 over `msg`.
#[must_use]
pub fn verify(key: &[u8; 32], msg: &[u8], sig: &[u8; 64]) -> bool {
    use ring::signature::{ED25519, UnparsedPublicKey};
    reins_proto::canon::strict_ed25519_key(key) && UnparsedPublicKey::new(&ED25519, key).verify(msg, sig).is_ok()
}

/// `state_dir/policy.json`.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct PolicyStore {
    #[serde(skip)]
    file: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    snapshot: Option<Signed>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tightens: Vec<Signed>,
    /// The key that signed what is kept: another pinned key makes it the old phone's, dropped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    signer: Option<String>,
    /// The highest snapshot version applied for `signer`: one not above it is refused.
    #[serde(default)]
    floor: u64,
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

/// The second copy of the highest snapshot version applied (`policy.floor`, also in the `reins` guard group).
fn floor_file(paths: &Paths) -> PathBuf {
    paths.state_dir.join("policy.floor")
}

/// The highest snapshot version applied, and the phone key that signed it: a floor belongs to one phone, so a new
/// phone (a re-pairing, a new key trusted) starts from nothing, and an old phone's floor never blocks it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Floor {
    pub signer: Option<String>,
    pub version: u64,
}

fn floor_copy(paths: &Paths) -> Floor {
    std::fs::read(floor_file(paths)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn write_floor(paths: &Paths, floor: &Floor) {
    let file = floor_file(paths);
    let written = serde_json::to_vec(floor)
        .map_err(|e| e.to_string())
        .and_then(|b| write_private(&file, &b).map_err(|e| format!("{}: {e}", file.display())));
    if let Err(e) = written {
        log::warn!("policy: {e}");
    }
}

/// The floor for `signer`: the larger of the copies that belong to it (an agent lowering one copy rolls nothing back).
fn floor_for(store: &PolicyStore, copy: &Floor, signer: Option<&str>) -> u64 {
    let kept = if store.signer.as_deref() == signer {
        store.floor
    } else {
        0
    };
    let copied = if copy.signer.as_deref() == signer {
        copy.version
    } else {
        0
    };
    kept.max(copied)
}

/// The floor of the phone pinned now.
#[must_use]
pub fn floor(paths: &Paths) -> Floor {
    let signer = pinned(paths).0;
    let version = floor_for(&PolicyStore::load(paths), &floor_copy(paths), signer.as_deref());
    Floor {
        signer,
        version,
    }
}

fn snapshot_version(signed: &Signed) -> Option<u64> {
    serde_json::from_str::<serde_json::Value>(&signed.body)
        .ok()
        .and_then(|v| v.pointer("/binding/version").and_then(serde_json::Value::as_u64))
}

/// The daemon's copy of the floor (in memory, where an agent can't lower it): files found below it are raised back
/// and an older snapshot is dropped. Nothing while logged out; for another phone than `known`'s, that phone's floor
/// on disk is taken as it is. Returns the floor known now.
#[must_use]
pub fn hold_floor(paths: &Paths, known: &Floor) -> Floor {
    if crate::server::oauth::logged_in_server(paths).is_none() {
        return known.clone();
    }
    let now = floor(paths);
    if now.signer != known.signer || now.version >= known.version {
        return now;
    }
    // Under the store's lock: everything read again, and only ever raised.
    let restored = PolicyStore::update(paths, |s| {
        if s.signer != known.signer {
            return None;
        }
        let version = floor_for(s, &floor_copy(paths), known.signer.as_deref()).max(known.version);
        log::warn!("policy: the kept version went down; restored to {version}");
        s.floor = version;
        if s.snapshot.as_ref().is_some_and(|signed| snapshot_version(signed).is_none_or(|v| v < version)) {
            s.snapshot = None;
        }
        let f = Floor {
            signer: s.signer.clone(),
            version,
        };
        write_floor(paths, &f);
        Some(f)
    });
    match restored {
        Ok(Some(f)) => f,
        Ok(None) => floor(paths),
        Err(e) => {
            log::warn!("policy: {e}");
            known.clone()
        }
    }
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
        let version = |s: &Signed| {
            serde_json::from_str::<serde_json::Value>(&s.body)
                .ok()
                .and_then(|v| v.pointer("/binding/version").and_then(serde_json::Value::as_u64))
                .unwrap_or(0)
        };
        self.tightens.iter().map(version).chain(std::iter::once(self.floor)).max().unwrap_or(0)
    }
}

/// The pinned signing key of the phone, and whether it is in dispute (the server named another one since).
fn pinned(paths: &Paths) -> (Option<String>, bool) {
    let t = TicketStore::load(paths);
    (t.signing_key().map(str::to_owned), t.key_change().is_some())
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
    let floor = floor_for(&store, &floor_copy(paths), key.as_deref());
    let open = |s: &Signed| {
        let m = dp::open(s, key.as_deref(), &verify)?;
        dp::check(m.binding(), client_key, now)?;
        Ok::<_, Refusal>(m)
    };
    let mut refused = None;
    let policy = match store.snapshot.as_ref().map(open) {
        _ if disputed => None,
        // The snapshot kept is the one that set the floor: anything below it is an older one put back.
        Some(Ok(PolicyMessage::Snapshot(p))) if p.binding.version >= floor => Some(*p),
        Some(Ok(PolicyMessage::Snapshot(_))) => {
            refused = Some(Refusal::Older);
            None
        }
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
        .filter_map(|s| match open(s) {
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
}

/// Applies what the server sent: pins the phone's signing key the first time one is named (P4), opens each message
/// with this app's key, and keeps what is signed by the pinned key, for this app and profile, valid and newer.
pub fn apply(paths: &Paths, identity: &Identity, mail: &PolicyMail, now: i64) -> Result<Applied, String> {
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
        // Another phone (a new key trusted): what the old one signed goes, and its floor with it.
        if store.signer != key {
            store.snapshot = None;
            store.tightens.clear();
            store.spent.clear();
            store.floor = 0;
            store.signer.clone_from(&key);
        }
        store.floor = floor_for(store, &floor_copy(paths), key.as_deref());
        let open = |s: &Signed| {
            dp::open(s, key.as_deref(), &verify).and_then(|m| dp::check(m.binding(), &client_key, now).map(|()| m))
        };
        for item in &mail.items {
            let signed: Option<Signed> =
                identity.unseal(&item.sealed).ok().and_then(|plain| serde_json::from_slice(&plain).ok());
            let Some(signed) = signed else {
                out.refused.push(Refusal::Unreadable);
                continue;
            };
            match open(&signed) {
                Ok(PolicyMessage::Snapshot(p)) if !disputed && p.binding.version > store.floor => {
                    store.floor = p.binding.version;
                    store.snapshot = Some(signed);
                    out.snapshots += 1;
                }
                // A tightening only tightens: taken whatever the floor says (a floor raised by anyone can't hold back a
                // Lockdown), until a snapshot as new supersedes it.
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
        let held = store.snapshot.as_ref().and_then(|s| match open(s) {
            Ok(PolicyMessage::Snapshot(p)) => Some(p.binding.version),
            _ => None,
        });
        store
            .tightens
            .retain(|s| matches!(open(s), Ok(PolicyMessage::Tighten(t)) if held.is_none_or(|v| t.binding.version > v)));
        store.tightens.sort_by_key(|s| std::cmp::Reverse(snapshot_version(s).unwrap_or(0)));
        store.tightens.truncate(reins_proto::desktop::MAX_TIGHTENS);
        write_floor(
            paths,
            &Floor {
                signer: store.signer.clone(),
                version: store.floor,
            },
        );
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
    for file in [store_file(paths), floor_file(paths)] {
        if let Err(e) = std::fs::remove_file(&file)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            log::warn!("{}: {e}", file.display());
        }
    }
}

/// Asks the server for what is newer than what is kept (`after`), waiting up to `wait`, and applies it.
pub async fn pull(
    paths: &Paths,
    identity: &Identity,
    client: &DesktopClient,
    after: u64,
    wait: Duration,
) -> Result<Applied, LinkError> {
    let mail = client.policy(after, wait).await?;
    let applied = apply(paths, identity, &mail, crate::now_unix()).map_err(LinkError::Failed)?;
    for r in &applied.refused {
        log::info!("policy: a message from the server was not used: {}", r.label());
    }
    if applied.snapshots + applied.tightens > 0 {
        log::info!("policy: {} snapshot(s), {} tightening(s) from the phone", applied.snapshots, applied.tightens);
    }
    Ok(applied)
}

/// The daemon's pick-up: once on start, then a long poll, so a Lockdown or another tightening arrives within seconds.
pub async fn puller(paths: Paths, identity: std::sync::Arc<Identity>) {
    let known = std::sync::Arc::new(std::sync::Mutex::new(floor(&paths)));
    tokio::join!(pull_loop(paths.clone(), identity, std::sync::Arc::clone(&known)), keep_floor(paths, known));
}

/// The floor this daemon knows, raised (or moved to a new phone's) by what is on disk now ([`hold_floor`]).
fn keep(paths: &Paths, known: &std::sync::Mutex<Floor>) {
    let mut k = known.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    *k = hold_floor(paths, &k);
}

/// Every second: the files' floor never goes below the one this daemon knows.
async fn keep_floor(paths: Paths, known: std::sync::Arc<std::sync::Mutex<Floor>>) {
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        keep(&paths, &known);
    }
}

async fn pull_loop(paths: Paths, identity: std::sync::Arc<Identity>, known: std::sync::Arc<std::sync::Mutex<Floor>>) {
    let mut seen = 0u64;
    let mut signer = pinned(&paths).0;
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
        // Another phone: what the old one's versions were says nothing about the new one's.
        let now_signer = pinned(&paths).0;
        if now_signer != signer {
            seen = 0;
            signer = now_signer;
        }
        let after = PolicyStore::load(&paths).newest().max(seen);
        let started = std::time::Instant::now();
        match pull(&paths, &identity, &client, after, wait).await {
            Ok(a) => {
                seen = seen.max(a.seen);
                keep(&paths, &known);
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

/// Before a hook decides, when no daemon keeps the policy fresh: one quick look ([`HOOK_LOOK`]) if nothing usable is
/// kept (or it ends soon) and no look was made lately.
pub async fn look_if_stale(paths: &Paths, config: &Config) {
    let Ok(identity) = Identity::load_or_create(&paths.identity_file()) else {
        return;
    };
    let now = crate::now_unix();
    let h = held(paths, &identity.public_key(), now);
    let fresh = h.policy.as_ref().is_some_and(|p| p.binding.not_after - now > ENDS_SOON);
    if fresh || now - h.looked_at < HOOK_LOOK_EVERY || crate::tickets::daemon_running(paths, config).await {
        return;
    }
    let Ok(client) = DesktopClient::new(paths) else {
        return;
    };
    let after = PolicyStore::load(paths).newest();
    match tokio::time::timeout(HOOK_LOOK, pull(paths, &identity, &client, after, Duration::ZERO)).await {
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
#[must_use]
pub fn effective(paths: &Paths, config: &Config, now: i64) -> (dp::Effective, Held) {
    let home = crate::config::home_dir().map(|h| h.to_string_lossy().into_owned()).unwrap_or_default();
    let h = match Identity::load_or_create(&paths.identity_file()) {
        Ok(identity) => held(paths, &identity.public_key(), now),
        Err(e) => {
            log::warn!("policy: {e}");
            Held::default()
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
    let until = |p: &DesktopPolicy| {
        let left = (p.binding.not_after - now).max(0);
        format!("{} h {:02} min", left / 3600, left % 3600 / 60)
    };
    match (e.gate, &h.policy) {
        (dp::Gate::Lockdown, _) => "Lockdown, from the phone".to_owned(),
        (dp::Gate::Bypass, Some(p)) => format!("Bypass, from the phone (valid {})", until(p)),
        (
            dp::Gate::Matrix {
                overnight,
            },
            Some(p),
        ) => format!(
            "the phone's{} (version {}, valid {})",
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

        pub(crate) fn sealed(&self, to: &str, m: &PolicyMessage) -> MailedPolicy {
            let signed =
                dp::sign(m, &self.key(), &|b| data_encoding::BASE64URL_NOPAD.encode(self.pair.sign(b).as_ref()));
            MailedPolicy {
                version: m.version(),
                kind: match m {
                    PolicyMessage::Snapshot(_) => PolicyKind::Snapshot,
                    PolicyMessage::Tighten(_) => PolicyKind::Tighten,
                },
                sealed: crate::identity::seal_to(to, &serde_json::to_vec(&signed).unwrap()).unwrap(),
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

    fn setup() -> (tempfile::TempDir, Paths, Identity, Phone) {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        paths.ensure().unwrap();
        let identity = Identity::load_or_create(&paths.identity_file()).unwrap();
        (dir, paths, identity, Phone::new())
    }

    fn mail(phone: &Phone, items: Vec<MailedPolicy>) -> PolicyMail {
        PolicyMail {
            items,
            signing_key: Some(phone.key()),
        }
    }

    const NOW: i64 = 1_800_000_000;

    #[test]
    fn a_signed_snapshot_is_kept_and_used() {
        let (_d, paths, id, phone) = setup();
        let me = id.public_key();
        let a = apply(&paths, &id, &mail(&phone, vec![phone.sealed(&me, &snapshot(&me, 5, NOW))]), NOW).unwrap();
        assert_eq!(a.snapshots, 1, "{a:?}");
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
        apply(&paths, &id, &mail(&phone, vec![newer.clone()]), NOW).unwrap();
        let a = apply(&paths, &id, &mail(&phone, vec![older, newer]), NOW).unwrap();
        assert_eq!(a.snapshots, 0);
        assert_eq!(a.refused, [Refusal::Older, Refusal::Older]);
        assert_eq!(held(&paths, &me, NOW).policy.map(|p| p.binding.version), Some(9));
    }

    /// A session, as `reins login` leaves it (the daemon holds the floor only while logged in).
    fn log_in(paths: &Paths) {
        let session = serde_json::json!({"server": "https://reins.example", "client_id": "c",
            "token_endpoint": "https://reins.example/reins/oauth/token", "access_token": "a",
            "access_expires_at": NOW + 3600, "refresh_token": "r"});
        write_private(&paths.session_file(), session.to_string().as_bytes()).unwrap();
        assert!(crate::server::oauth::logged_in_server(paths).is_some());
    }

    fn lowered(paths: &Paths, signer: &str, version: u64) {
        write_floor(
            paths,
            &Floor {
                signer: Some(signer.to_owned()),
                version,
            },
        );
    }

    /// An agent puts an older, looser snapshot back and lowers the floor in `policy.json`: the second copy refuses it;
    /// with both copies lowered, the daemon's own (in memory) restores them.
    #[test]
    fn a_lowered_floor_does_not_load_an_older_snapshot() {
        let (_d, paths, id, phone) = setup();
        log_in(&paths);
        let me = id.public_key();
        let mut older = snapshot(&me, 8, NOW);
        if let PolicyMessage::Snapshot(p) = &mut older {
            p.mode.mode = Mode::Bypass;
        }
        apply(&paths, &id, &mail(&phone, vec![phone.sealed(&me, &older)]), NOW).unwrap();
        let old_store = std::fs::read(store_file(&paths)).unwrap();
        apply(&paths, &id, &mail(&phone, vec![phone.sealed(&me, &snapshot(&me, 9, NOW))]), NOW).unwrap();
        let known = floor(&paths);
        assert_eq!(known.version, 9);
        // policy.json put back as it was at version 8 (its floor 8 with it).
        std::fs::write(store_file(&paths), &old_store).unwrap();
        let h = held(&paths, &me, NOW);
        assert_eq!((h.policy.is_none(), h.refused), (true, Some(Refusal::Older)));
        // Both copies lowered: loaded, until the daemon (which kept 9 in memory) restores the floor.
        lowered(&paths, &phone.key(), 8);
        assert_eq!(held(&paths, &me, NOW).policy.map(|p| p.mode.mode), Some(Mode::Bypass));
        assert_eq!(hold_floor(&paths, &known).version, 9);
        assert!(held(&paths, &me, NOW).policy.is_none(), "the older snapshot is gone");
        assert_eq!(floor(&paths).version, 9);
        // A daemon that knew less never lowers what is on disk.
        assert_eq!(
            hold_floor(
                &paths,
                &Floor {
                    signer: Some(phone.key()),
                    version: 3,
                }
            )
            .version,
            9
        );
        assert_eq!(floor(&paths).version, 9);
        // The phone's next snapshot is taken as usual.
        apply(&paths, &id, &mail(&phone, vec![phone.sealed(&me, &snapshot(&me, 10, NOW))]), NOW).unwrap();
        assert_eq!(held(&paths, &me, NOW).policy.map(|p| p.binding.version), Some(10));
    }

    /// Logged out, the daemon holds nothing (`forget` removed the floor with the session).
    #[test]
    fn logged_out_the_floor_is_left_alone() {
        let (_d, paths, id, phone) = setup();
        let me = id.public_key();
        apply(&paths, &id, &mail(&phone, vec![phone.sealed(&me, &snapshot(&me, 9, NOW))]), NOW).unwrap();
        let known = floor(&paths);
        forget(&paths);
        assert_eq!(hold_floor(&paths, &known), known, "nothing written while logged out");
        assert!(!floor_file(&paths).exists());
    }

    /// A new phone (re-paired, a new key trusted) starts from its own versions: the old phone's floor, on disk or in the
    /// daemon's memory, never blocks its snapshots.
    #[test]
    fn a_new_phone_starts_its_own_floor() {
        let (_d, paths, id, old_phone) = setup();
        log_in(&paths);
        let me = id.public_key();
        apply(&paths, &id, &mail(&old_phone, vec![old_phone.sealed(&me, &snapshot(&me, 900, NOW))]), NOW).unwrap();
        let known = floor(&paths);
        let new_phone = Phone::new();
        TicketStore::update(&paths, |s| {
            s.see_signing_key(&new_phone.key());
            s.trust_key()
        })
        .unwrap();
        let a =
            apply(&paths, &id, &mail(&new_phone, vec![new_phone.sealed(&me, &snapshot(&me, 1, NOW))]), NOW).unwrap();
        assert_eq!(a.snapshots, 1, "{a:?}");
        let now = hold_floor(&paths, &known);
        assert_eq!((now.signer.as_deref(), now.version), (Some(new_phone.key().as_str()), 1));
        assert_eq!(held(&paths, &me, NOW).policy.map(|p| p.binding.version), Some(1));
    }

    /// A floor raised by anyone (an agent, to block the phone) can't hold back a tightening: Lockdown still applies.
    #[test]
    fn tightenings_pass_a_raised_floor() {
        let (_d, paths, id, phone) = setup();
        let me = id.public_key();
        apply(&paths, &id, &mail(&phone, vec![phone.sealed(&me, &snapshot(&me, 5, NOW))]), NOW).unwrap();
        lowered(&paths, &phone.key(), u64::MAX);
        let lock = PolicyMessage::Tighten(Tighten {
            binding: Binding {
                version: 6,
                issued_at: NOW,
                not_after: NOW + dp::TIGHTEN_SECS,
                client_key: me.clone(),
                connection_id: "conn-1".to_owned(),
                profile: dp::PERSONAL.to_owned(),
            },
            tightening: Tightening {
                lockdown: true,
                ..Tightening::default()
            },
        });
        let a = apply(&paths, &id, &mail(&phone, vec![phone.sealed(&me, &lock)]), NOW).unwrap();
        assert_eq!(a.tightens, 1, "{a:?}");
        let h = held(&paths, &me, NOW);
        assert!(h.policy.is_none(), "the raised floor refuses the snapshot");
        let config = Config::default();
        let local = dp::Local {
            facts: &config.auto.facts,
            matrix: &config.auto.matrix,
            roots: &[],
            scratch: &[],
            home: "/home/me",
        };
        assert_eq!(dp::effective(None, &h.tightens, &local, &h.spent, NOW).gate, dp::Gate::Lockdown);
    }

    #[test]
    fn another_key_is_refused() {
        let (_d, paths, id, phone) = setup();
        let me = id.public_key();
        apply(&paths, &id, &mail(&phone, vec![phone.sealed(&me, &snapshot(&me, 1, NOW))]), NOW).unwrap();
        // The server swaps in its own key: named as the phone's, and signing a looser snapshot.
        let server = Phone::new();
        let a = apply(&paths, &id, &mail(&server, vec![server.sealed(&me, &snapshot(&me, 2, NOW))]), NOW).unwrap();
        assert_eq!(a.snapshots, 0);
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
        )
        .unwrap();
        assert_eq!(a.refused, [Refusal::OtherKey]);
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
        )
        .unwrap();
        assert_eq!(a.refused, [Refusal::OtherProfile, Refusal::OtherComputer, Refusal::Unreadable]);
        assert!(held(&paths, &me, NOW).policy.is_none());
    }

    #[test]
    fn stale_or_edited_falls_back() {
        let (_d, paths, id, phone) = setup();
        let me = id.public_key();
        apply(&paths, &id, &mail(&phone, vec![phone.sealed(&me, &snapshot(&me, 1, NOW))]), NOW).unwrap();
        assert!(held(&paths, &me, NOW + dp::VALIDITY_SECS - 1).policy.is_some());
        let h = held(&paths, &me, NOW + dp::VALIDITY_SECS);
        assert_eq!((h.policy.is_none(), h.refused), (true, Some(Refusal::Expired)));
        // An agent edits the kept snapshot (Bypass, say): it no longer verifies.
        let file = paths.state_dir.join("policy.json");
        let text = std::fs::read_to_string(&file).unwrap();
        std::fs::write(&file, text.replace("auto", "bypass")).unwrap();
        let h = held(&paths, &me, NOW);
        assert_eq!((h.policy.is_none(), h.refused), (true, Some(Refusal::BadSignature)));
    }

    #[test]
    fn a_lockdown_applies_without_a_snapshot_until_superseded() {
        let (_d, paths, id, phone) = setup();
        let me = id.public_key();
        let lock = PolicyMessage::Tighten(Tighten {
            binding: match snapshot(&me, 7, NOW) {
                PolicyMessage::Snapshot(p) => Binding {
                    not_after: NOW + dp::TIGHTEN_SECS,
                    ..p.binding
                },
                PolicyMessage::Tighten(t) => t.binding,
            },
            tightening: Tightening {
                lockdown: true,
                ..Tightening::default()
            },
        });
        apply(&paths, &id, &mail(&phone, vec![phone.sealed(&me, &lock)]), NOW).unwrap();
        let config = Config::default();
        assert_eq!(effective(&paths, &config, NOW).0.gate, dp::Gate::Lockdown);
        apply(&paths, &id, &mail(&phone, vec![phone.sealed(&me, &snapshot(&me, 8, NOW))]), NOW).unwrap();
        assert_ne!(effective(&paths, &config, NOW).0.gate, dp::Gate::Lockdown);
        assert!(PolicyStore::load(&paths).tightens.is_empty(), "superseded tightenings go");
    }

    #[test]
    fn spent_uses_are_counted_here() {
        let (_d, paths, _id, _phone) = setup();
        spend(&paths, "t1");
        spend(&paths, "t1");
        assert_eq!(PolicyStore::load(&paths).spent.get("t1"), Some(&2));
    }
}
