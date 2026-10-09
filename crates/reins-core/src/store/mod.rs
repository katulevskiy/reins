//! SQLite exists only in memory. Complete snapshots (including metadata) are authenticated
//! ciphertext on disk. Installation keys are wrapped by the platform [`KeyWrapper`];
//! account stores use separate keys derived from the owning account vault key.
//!
//! One connection behind one mutex: every method is a short critical section
//! and the mutex doubles as the single-writer lock that makes grant
//! evaluation and use reservation atomic (see `grants.rs`).

mod accounts;
mod audit;
mod desktop;
mod grants;
mod mcp;
mod pending;
mod protected;
mod snapshot;
pub(crate) use protected::Owner;
mod secrets;

use std::fmt;
use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, params};
use zeroize::Zeroizing;

pub use accounts::StoredAccount;
pub use audit::{
    AUDIT_RETENTION, AuditAutopilot, AuditEmail, AuditInfo, AuditMessage, AuditRecord, DETAIL_UNAVAILABLE,
};
pub use grants::StoredGrant;
pub use mcp::{McpTool, SECRET_SERVICE as MCP_SECRET_SERVICE, SIGN_IN_SERVICE as MCP_SIGN_IN_SERVICE, StoredMcpServer};
pub use pending::{HANDLED_RETENTION_SECS, PENDING_TTL_SECS, PendingRow};

use crate::crypto::Dek;
use crate::{CoreError, ForeignError, KeyWrapper};

const DB_FILE: &str = "reins.db";
const DEK_FILE: &str = "dek.bin";
const DEK_CHECK_AAD: &str = "meta.dek_check";
const DEK_CHECK_PLAINTEXT: &[u8] = b"reins-dek-check";

const SCHEMA: &str = "
PRAGMA journal_mode = WAL;
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value BLOB NOT NULL);
CREATE TABLE IF NOT EXISTS session (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    server_url TEXT NOT NULL,
    email TEXT NOT NULL,
    refresh_token BLOB NOT NULL
);
CREATE TABLE IF NOT EXISTS grants (
    id TEXT PRIMARY KEY,
    connection_id TEXT NOT NULL,
    connection_label TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    grant_json TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS grants_connection ON grants (connection_id);
CREATE TABLE IF NOT EXISTS audit (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    at INTEGER NOT NULL,
    connection_label TEXT NOT NULL,
    action TEXT NOT NULL,
    outcome TEXT NOT NULL,
    grant_id TEXT,
    detail BLOB
);
CREATE TABLE IF NOT EXISTS pending (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL CHECK (kind IN ('request', 'pairing')),
    created_at INTEGER NOT NULL,
    parked_at INTEGER NOT NULL,
    payload BLOB NOT NULL
);
CREATE TABLE IF NOT EXISTS handled (id TEXT PRIMARY KEY, at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS connection_prefs (connection_id TEXT PRIMARY KEY, icon TEXT);
";

/// Columns added after the first release, applied once each (`PRAGMA user_version`).
const MIGRATIONS: [&str; 8] = [
    "
ALTER TABLE audit ADD COLUMN connection_id TEXT NOT NULL DEFAULT '';
ALTER TABLE audit ADD COLUMN service TEXT NOT NULL DEFAULT 'gmail';
ALTER TABLE audit ADD COLUMN account TEXT;
ALTER TABLE audit ADD COLUMN item_count INTEGER NOT NULL DEFAULT 0;
ALTER TABLE audit ADD COLUMN info BLOB;
ALTER TABLE grants ADD COLUMN last_used_at INTEGER;
ALTER TABLE grants ADD COLUMN origin TEXT NOT NULL DEFAULT 'approval';
",
    // Several accounts per service. The one account of earlier versions (cached as `gmail_account`) becomes the
    // first registered account and its grants are bound to it, so they never cover an account added later.
    "
CREATE TABLE accounts (
    service TEXT NOT NULL,
    account TEXT NOT NULL,
    added_at INTEGER NOT NULL,
    PRIMARY KEY (service, account)
);
INSERT OR IGNORE INTO accounts (service, account, added_at)
    SELECT 'gmail', lower(value), CAST(strftime('%s', 'now') AS INTEGER) FROM meta WHERE key = 'app.gmail_account';
UPDATE grants SET grant_json = json_set(grant_json, '$.account',
    (SELECT lower(value) FROM meta WHERE key = 'app.gmail_account'))
    WHERE json_extract(grant_json, '$.account') IS NULL
    AND EXISTS (SELECT 1 FROM meta WHERE key = 'app.gmail_account');
",
    // Other integrations: what they need to keep secret (a Telegram session, a GitHub token), sealed with the data key,
    // and the operation each audit entry was about.
    "
CREATE TABLE secrets (
    service TEXT NOT NULL,
    account TEXT NOT NULL,
    value BLOB NOT NULL,
    PRIMARY KEY (service, account)
);
ALTER TABLE audit ADD COLUMN op TEXT NOT NULL DEFAULT '';
",
    // The Reins desktop app: its public key, pinned to its connection when the user approved the pairing.
    "
CREATE TABLE desktop_keys (
    connection_id TEXT PRIMARY KEY,
    public_key TEXT NOT NULL,
    pinned_at INTEGER NOT NULL
);
",
    // MCP servers the user added: how the phone signs in, their tools and which are heavy (tokens are secrets).
    "
CREATE TABLE mcp_servers (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    url TEXT NOT NULL,
    auth TEXT NOT NULL,
    client TEXT,
    tools TEXT NOT NULL DEFAULT '[]',
    heavy TEXT NOT NULL DEFAULT '[]',
    status TEXT NOT NULL DEFAULT 'ok',
    error TEXT,
    listed_at INTEGER,
    added_at INTEGER NOT NULL
);
",
    // Uploads waiting for the user's decision are parked too: the kind check gains `blob`.
    "
CREATE TABLE pending_kinds (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL CHECK (kind IN ('request', 'pairing', 'blob')),
    created_at INTEGER NOT NULL,
    parked_at INTEGER NOT NULL,
    payload BLOB NOT NULL
);
INSERT INTO pending_kinds (id, kind, created_at, parked_at, payload)
    SELECT id, kind, created_at, parked_at, payload FROM pending;
DROP TABLE pending;
ALTER TABLE pending_kinds RENAME TO pending;
",
    // Autopilot (spec 2026-10-01): modes (scope '' = global, else a connection id), profiles and decision memory
    // (sealed: everything learned), what was evaluated for each parked request (sealed), auto-approvals for the rate
    // limit, and the targets approved per connection (keyed hashes only).
    "
CREATE TABLE autopilot_settings (
    scope TEXT PRIMARY KEY,
    mode TEXT,
    bypass_until INTEGER,
    profile_id TEXT,
    updated_at INTEGER NOT NULL
);
CREATE TABLE autopilot_profiles (
    id TEXT PRIMARY KEY,
    created_at INTEGER NOT NULL,
    data BLOB NOT NULL
);
CREATE TABLE autopilot_memory (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    profile_id TEXT NOT NULL,
    request_id TEXT NOT NULL,
    at INTEGER NOT NULL,
    data BLOB NOT NULL
);
CREATE INDEX autopilot_memory_profile ON autopilot_memory (profile_id, id);
CREATE TABLE autopilot_suggestions (
    request_id TEXT PRIMARY KEY,
    at INTEGER NOT NULL,
    data BLOB NOT NULL
);
CREATE TABLE autopilot_rate (
    connection_id TEXT NOT NULL,
    at INTEGER NOT NULL
);
CREATE INDEX autopilot_rate_connection ON autopilot_rate (connection_id, at);
CREATE TABLE autopilot_targets (
    key TEXT PRIMARY KEY,
    at INTEGER NOT NULL
);
",
    // Another phone asking for the account's keys is parked too: the kind check gains `join`.
    "
CREATE TABLE pending_kinds (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL CHECK (kind IN ('request', 'pairing', 'blob', 'join')),
    created_at INTEGER NOT NULL,
    parked_at INTEGER NOT NULL,
    payload BLOB NOT NULL
);
INSERT INTO pending_kinds (id, kind, created_at, parked_at, payload)
    SELECT id, kind, created_at, parked_at, payload FROM pending;
DROP TABLE pending;
ALTER TABLE pending_kinds RENAME TO pending;
",
];

fn migrate(conn: &Connection) -> Result<(), CoreError> {
    // A database that was never versioned (0) is the first release's schema (1).
    let mut version = conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))?.max(1);
    for (index, sql) in MIGRATIONS.iter().enumerate() {
        if version == i64::try_from(index + 1).unwrap_or(i64::MAX) {
            version += 1;
            conn.execute_batch(&format!("BEGIN;{sql}PRAGMA user_version = {version};COMMIT;"))?;
        }
    }
    Ok(())
}

/// Current unix time in seconds.
pub fn unix_now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

impl From<rusqlite::Error> for CoreError {
    fn from(e: rusqlite::Error) -> Self {
        Self::storage(e.to_string())
    }
}

/// The persisted part of a Vaultwarden session. The access token is never stored.
#[derive(Clone, PartialEq, Eq)]
pub struct StoredSession {
    pub server_url: String,
    pub email: String,
    pub refresh_token: Zeroizing<String>,
}

impl fmt::Debug for StoredSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StoredSession")
            .field("server_url", &self.server_url)
            .field("email", &self.email)
            .field("refresh_token", &"<redacted>")
            .finish()
    }
}

pub struct Store {
    active: std::sync::atomic::AtomicBool,
    conn: Mutex<Option<Connection>>,
    dek: Mutex<Option<Dek>>,
    persistence: Option<protected::Persistence>,
    control: Option<Arc<Store>>,
    control_owner: Mutex<Option<Owner>>,
    pub(crate) owner: Option<Owner>,
    dek_was_reset: bool,
    /// Bumped whenever the data changes (a write, or a newer copy loaded from disk), so account sync can skip
    /// exporting and hashing everything after calls that only read.
    writes: std::sync::atomic::AtomicU64,
}

impl fmt::Debug for Store {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Store").field("dek_was_reset", &self.dek_was_reset).finish_non_exhaustive()
    }
}

impl Store {
    /// Opens (or creates) the store in `data_dir`.
    ///
    /// Missing or invalid existing keys fail closed without rewriting encrypted data.
    /// Legacy SQLite is encrypted durably before its plaintext files are removed.
    pub fn open(data_dir: &Path, keys: &dyn KeyWrapper) -> Result<Self, CoreError> {
        fs::create_dir_all(data_dir).map_err(|e| CoreError::storage(format!("cannot create data directory: {e}")))?;
        let protected_path = data_dir.join("reins.sealed");
        let disk = if !protected_path.exists() && data_dir.join(DB_FILE).exists() {
            Some(Connection::open(data_dir.join(DB_FILE))?)
        } else {
            None
        };
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(SCHEMA)?;
        migrate(&conn)?;
        conn.execute_batch("PRAGMA temp_store=MEMORY; PRAGMA secure_delete=ON;")?;
        let dek_path = data_dir.join(DEK_FILE);
        let (existing, had_key) = match fs::read(&dek_path) {
            Ok(wrapped) => match keys.unwrap(wrapped) {
                Ok(raw) => {
                    let raw = Zeroizing::new(raw);
                    (
                        Dek::from_bytes(&raw).ok().filter(|dek| key_check_passes(disk.as_ref().unwrap_or(&conn), dek)),
                        true,
                    )
                }
                Err(ForeignError::NeedsUserInteraction) => {
                    return Err(CoreError::storage("the device keystore is locked; unlock the phone and try again"));
                }
                Err(ForeignError::Failed {
                    reason: message,
                }) => {
                    drop(message);
                    return Err(CoreError::storage(
                        "the device encryption key is unavailable; encrypted account data was preserved",
                    ));
                }
            },
            Err(e) if e.kind() == ErrorKind::NotFound => (None, false),
            Err(e) => return Err(CoreError::storage(format!("cannot read the data key: {e}"))),
        };
        let (dek, dek_was_reset) = match existing {
            Some(dek) => (dek, false),
            None if protected_path.exists() || had_key => {
                return Err(CoreError::storage(
                    "the device encryption key is unavailable; encrypted account data was preserved",
                ));
            }
            None => (create_dek(&dek_path, keys)?, false),
        };
        write_key_check_if_missing(&conn, &dek)?;
        let mut conn = conn;
        if let Some(disk) = disk {
            disk.execute_batch(SCHEMA)?;
            migrate(&disk)?;
            let legacy = snapshot::export(&disk, false)?;
            snapshot::import(&mut conn, &legacy, false)?;
            drop(disk);
        }
        let store = Self {
            active: std::sync::atomic::AtomicBool::new(true),
            conn: Mutex::new(Some(conn)),
            dek: Mutex::new(Some(dek)),
            dek_was_reset,
            writes: std::sync::atomic::AtomicU64::new(0),
            persistence: Some(protected::Persistence::new(protected_path, "reins.device-store.v1".to_owned())),
            control: None,
            control_owner: Mutex::new(None),
            owner: None,
        };
        protected::protect(&store, data_dir)?;
        Ok(store)
    }

    /// True when `open` found a data key it could not use and discarded the
    /// encrypted data (false on first creation).
    pub fn dek_was_reset(&self) -> bool {
        self.dek_was_reset
    }

    pub(crate) fn seal(&self, aad: &str, plaintext: &[u8]) -> Result<Vec<u8>, CoreError> {
        self.dek
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .ok_or(CoreError::NotLoggedIn)?
            .seal(aad, plaintext)
    }

    pub(crate) fn unseal(&self, aad: &str, blob: &[u8]) -> Result<Vec<u8>, CoreError> {
        self.dek.lock().unwrap_or_else(PoisonError::into_inner).as_ref().ok_or(CoreError::NotLoggedIn)?.open(aad, blob)
    }

    /// This installation's stable device identifier (UUID v4), created on first use.
    pub fn device_id(&self) -> Result<String, CoreError> {
        if let Some(control) = &self.control {
            return control.device_id();
        }
        let conn = self.lock()?;
        let existing: Option<String> =
            conn.query_row("SELECT value FROM meta WHERE key = 'device_id'", [], |r| r.get(0)).optional()?;
        if let Some(id) = existing {
            return Ok(id);
        }
        let id = uuid::Uuid::new_v4().to_string();
        conn.execute("INSERT INTO meta (key, value) VALUES ('device_id', ?1)", params![id])?;
        Ok(id)
    }

    pub fn save_session(&self, session: &StoredSession) -> Result<(), CoreError> {
        if let Some(control) = &self.control
            && let Some(owner) = self.bound_owner()
        {
            self.check_owner()?;
            return control.save_session_for(session, &owner);
        }
        let token = self.seal("session.refresh_token", session.refresh_token.as_bytes())?;
        self.lock()?.execute(
            "INSERT INTO session (id, server_url, email, refresh_token) VALUES (1, ?1, ?2, ?3)
                 ON CONFLICT (id) DO UPDATE SET server_url = ?1, email = ?2, refresh_token = ?3",
            params![session.server_url, session.email, token],
        )?;
        Ok(())
    }

    /// The saved session; `None` when signed out or when it cannot be decrypted.
    pub fn load_session(&self) -> Result<Option<StoredSession>, CoreError> {
        if let Some(control) = &self.control {
            return control.load_session();
        }
        let row: Option<(String, String, Vec<u8>)> = self
            .lock()?
            .query_row("SELECT server_url, email, refresh_token FROM session WHERE id = 1", [], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .optional()?;
        let Some((server_url, email, sealed)) = row else {
            return Ok(None);
        };
        let Ok(token) = self.unseal("session.refresh_token", &sealed) else {
            log::warn!("saved session cannot be decrypted; signing out");
            self.clear_session()?;
            return Ok(None);
        };
        let refresh_token =
            Zeroizing::new(String::from_utf8(token).map_err(|_| CoreError::storage("corrupt session"))?);
        Ok(Some(StoredSession {
            server_url,
            email,
            refresh_token,
        }))
    }

    pub fn meta_get(&self, key: &str) -> Result<Option<String>, CoreError> {
        Ok(self
            .lock()?
            .query_row("SELECT value FROM meta WHERE key = ?1", params![format!("app.{key}")], |r| {
                r.get::<_, String>(0)
            })
            .optional()?)
    }

    pub fn meta_set(&self, key: &str, value: &str) -> Result<(), CoreError> {
        self.lock()?.execute(
            "INSERT INTO meta (key, value) VALUES (?1, ?2) ON CONFLICT (key) DO UPDATE SET value = ?2",
            params![format!("app.{key}"), value],
        )?;
        Ok(())
    }

    pub(crate) fn bound_owner(&self) -> Option<Owner> {
        self.owner.clone().or_else(|| self.control_owner.lock().unwrap_or_else(PoisonError::into_inner).clone())
    }
    pub(crate) fn bind_control_owner(&self, owner: Owner) {
        *self.control_owner.lock().unwrap_or_else(PoisonError::into_inner) = Some(owner);
    }
    fn active_owner_matches(conn: &Connection, owner: Option<&Owner>) -> Result<bool, CoreError> {
        let active: Option<String> =
            conn.query_row("SELECT value FROM meta WHERE key = 'app.active-account'", [], |r| r.get(0)).optional()?;
        Ok(active.unwrap_or_default() == owner.map(Owner::id).unwrap_or_default())
    }
    fn save_session_for(&self, session: &StoredSession, owner: &Owner) -> Result<(), CoreError> {
        if session.server_url != owner.server {
            return Err(CoreError::NotLoggedIn);
        }
        let sealed = self.seal("session.refresh_token", session.refresh_token.as_bytes())?;
        let conn = self.lock()?;
        if !Self::active_owner_matches(&conn, Some(owner))? {
            return Err(CoreError::NotLoggedIn);
        }
        conn.execute("INSERT INTO session (id, server_url, email, refresh_token) VALUES (1, ?1, ?2, ?3) ON CONFLICT (id) DO UPDATE SET server_url=?1, email=?2, refresh_token=?3", params![session.server_url,session.email,sealed])?;
        Ok(())
    }
    pub(crate) fn bind_account_session(&self, session: &StoredSession, owner: &Owner) -> Result<(), CoreError> {
        let sealed = self.seal("session.refresh_token", session.refresh_token.as_bytes())?;
        let encoded = serde_json::to_string(owner).map_err(|_| CoreError::storage("cannot remember account"))?;
        let server = crate::http::ServerUrl::parse(&session.server_url)?;
        if server.as_str() != owner.server {
            return Err(CoreError::NotLoggedIn);
        }
        let mut conn = self.lock()?;
        let tx = conn.transaction()?;
        for (name, value) in [
            ("active-account".to_owned(), owner.id()),
            ("active-owner".to_owned(), encoded),
            (crate::session::account_id_key(&server, &session.email), owner.user_id.clone()),
        ] {
            tx.execute(
                "INSERT INTO meta (key,value) VALUES (?1,?2) ON CONFLICT(key) DO UPDATE SET value=?2",
                params![format!("app.{name}"), value],
            )?;
        }
        tx.execute("INSERT INTO session (id,server_url,email,refresh_token) VALUES (1,?1,?2,?3) ON CONFLICT(id) DO UPDATE SET server_url=?1,email=?2,refresh_token=?3",params![session.server_url,session.email,sealed])?;
        tx.commit()?;
        drop(conn);
        self.flush()
    }
    pub(crate) fn clear_session_for(&self, owner: Option<&Owner>) -> Result<(), CoreError> {
        let mut conn = self.lock()?;
        if !Self::active_owner_matches(&conn, owner)? {
            return Err(CoreError::NotLoggedIn);
        }
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM session", [])?;
        tx.execute("DELETE FROM meta WHERE key IN ('app.active-account','app.active-owner')", [])?;
        tx.commit()?;
        drop(conn);
        self.flush()
    }

    pub(crate) fn check_owner(&self) -> Result<(), CoreError> {
        if !self.active.load(std::sync::atomic::Ordering::Acquire) {
            return Err(CoreError::NotLoggedIn);
        }
        if let Some(control) = &self.control
            && let Some(owner) = self.bound_owner()
            && control.meta_get("active-account")?.as_deref() != Some(&owner.id())
        {
            return Err(CoreError::NotLoggedIn);
        }
        Ok(())
    }

    /// The icon the user picked for a connection (`None` = automatic).
    pub fn connection_icon(&self, connection_id: &str) -> Result<Option<String>, CoreError> {
        Ok(self
            .lock()?
            .query_row("SELECT icon FROM connection_prefs WHERE connection_id = ?1", params![connection_id], |r| {
                r.get::<_, Option<String>>(0)
            })
            .optional()?
            .flatten())
    }

    pub fn set_connection_icon(&self, connection_id: &str, icon: Option<&str>) -> Result<(), CoreError> {
        self.lock()?.execute(
            "INSERT INTO connection_prefs (connection_id, icon) VALUES (?1, ?2)
                 ON CONFLICT (connection_id) DO UPDATE SET icon = ?2",
            params![connection_id, icon],
        )?;
        Ok(())
    }

    pub fn clear_session(&self) -> Result<(), CoreError> {
        if let Some(control) = &self.control
            && let Some(owner) = self.bound_owner()
        {
            return control.clear_session_for(Some(&owner));
        }
        self.lock()?.execute("DELETE FROM session", [])?;
        Ok(())
    }
}

fn key_check_passes(conn: &Connection, dek: &Dek) -> bool {
    let stored: Option<Vec<u8>> =
        conn.query_row("SELECT value FROM meta WHERE key = 'dek_check'", [], |r| r.get(0)).optional().ok().flatten();
    // A database without a check value was created fresh next to an existing key.
    stored.is_none_or(|blob| dek.open(DEK_CHECK_AAD, &blob).is_ok_and(|p| p == DEK_CHECK_PLAINTEXT))
}

fn write_key_check_if_missing(conn: &Connection, dek: &Dek) -> Result<(), CoreError> {
    let check = dek.seal(DEK_CHECK_AAD, DEK_CHECK_PLAINTEXT)?;
    conn.execute("INSERT OR IGNORE INTO meta (key, value) VALUES ('dek_check', ?1)", params![check])?;
    Ok(())
}

fn create_dek(dek_path: &Path, keys: &dyn KeyWrapper) -> Result<Dek, CoreError> {
    let (dek, raw) = Dek::generate()?;
    let wrapped = keys.wrap(raw.to_vec()).map_err(|e| match e {
        ForeignError::NeedsUserInteraction => {
            CoreError::storage("the device keystore is locked; unlock the phone and try again")
        }
        ForeignError::Failed {
            reason: message,
        } => CoreError::storage(format!("cannot protect the data key: {message}")),
    })?;
    write_atomically(dek_path, &wrapped)?;
    Ok(dek)
}

fn write_atomically(path: &Path, bytes: &[u8]) -> Result<(), CoreError> {
    let tmp: PathBuf = path.with_extension("tmp");
    let fail = |e: std::io::Error| CoreError::storage(format!("cannot write the data key: {e}"));
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&tmp).map_err(fail)?;
    file.write_all(bytes).map_err(fail)?;
    file.sync_all().map_err(fail)?;
    fs::rename(&tmp, path).map_err(fail)?;
    if let Some(parent) = path.parent() {
        fs::File::open(parent).and_then(|f| f.sync_all()).map_err(fail)?;
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::atomic::{AtomicU8, Ordering};

    use super::*;

    pub(crate) const LOCKED: u8 = 1;
    pub(crate) const BROKEN: u8 = 2;

    /// Stand-in for the Keystore: "wraps" by prefixing a tag and xoring.
    #[derive(Default)]
    pub(crate) struct FakeKeys(pub(crate) AtomicU8);

    impl KeyWrapper for FakeKeys {
        fn wrap(&self, plaintext: Vec<u8>) -> Result<Vec<u8>, ForeignError> {
            let mut out = b"KW1".to_vec();
            out.extend(plaintext.iter().map(|b| b ^ 0x5a));
            Ok(out)
        }

        fn unwrap(&self, wrapped: Vec<u8>) -> Result<Vec<u8>, ForeignError> {
            match self.0.load(Ordering::SeqCst) {
                LOCKED => Err(ForeignError::NeedsUserInteraction),
                BROKEN => Err(ForeignError::Failed {
                    reason: "key permanently invalidated".to_owned(),
                }),
                _ => match wrapped.strip_prefix(b"KW1") {
                    Some(rest) => Ok(rest.iter().map(|b| b ^ 0x5a).collect()),
                    None => Err(ForeignError::Failed {
                        reason: "bad blob".to_owned(),
                    }),
                },
            }
        }
    }

    pub(crate) fn open(dir: &Path) -> Store {
        Store::open(dir, &FakeKeys::default()).unwrap()
    }

    fn session() -> StoredSession {
        StoredSession {
            server_url: "https://rw.example.com".to_owned(),
            email: "me@example.com".to_owned(),
            refresh_token: Zeroizing::new("REFRESH-TOKEN-SECRET".to_owned()),
        }
    }

    pub(crate) fn fill(store: &Store) {
        let mut saved = session();
        if let Some(owner) = store.bound_owner() {
            saved.server_url = owner.server;
        }
        store.save_session(&saved).unwrap();
        store.park("r1", crate::PendingKind::Request, 100, unix_now(), b"{\"x\":1}").unwrap();
        store
            .append_audit(&AuditRecord {
                seq: 0,
                at: 5,
                connection_id: "c1".to_owned(),
                connection_label: "ChatGPT".to_owned(),
                action: "search".to_owned(),
                outcome: "released".to_owned(),
                detail: "2 messages".to_owned(),
                grant_id: None,
                service: "gmail".to_owned(),
                account: None,
                count: 2,
                info: AuditInfo::default(),
                op: String::new(),
            })
            .unwrap();
    }

    #[test]
    fn a_first_release_database_is_migrated_in_place() {
        let dir = tempfile::tempdir().unwrap();
        {
            let conn = Connection::open(dir.path().join(DB_FILE)).unwrap();
            conn.execute_batch(
                "PRAGMA user_version = 1;
                 CREATE TABLE meta (key TEXT PRIMARY KEY, value BLOB NOT NULL);
                 CREATE TABLE session (id INTEGER PRIMARY KEY CHECK (id = 1), server_url TEXT NOT NULL, email TEXT NOT NULL, refresh_token BLOB NOT NULL);
                 CREATE TABLE grants (id TEXT PRIMARY KEY, connection_id TEXT NOT NULL, connection_label TEXT NOT NULL, created_at INTEGER NOT NULL, grant_json TEXT NOT NULL);
                 CREATE TABLE audit (seq INTEGER PRIMARY KEY AUTOINCREMENT, at INTEGER NOT NULL, connection_label TEXT NOT NULL, action TEXT NOT NULL, outcome TEXT NOT NULL, grant_id TEXT, detail BLOB);
                 CREATE TABLE pending (id TEXT PRIMARY KEY, kind TEXT NOT NULL, created_at INTEGER NOT NULL, parked_at INTEGER NOT NULL, payload BLOB NOT NULL);
                 CREATE TABLE handled (id TEXT PRIMARY KEY, at INTEGER NOT NULL);
                 INSERT INTO audit (at, connection_label, action, outcome) VALUES (7, 'Old', 'search', 'released');
                 INSERT INTO meta (key, value) VALUES ('app.gmail_account', 'Me@Gmail.com');",
            )
            .unwrap();
            let grant = serde_json::to_string(&grants::tests::read_grant("g1", "c1", None)).unwrap();
            conn.execute(
                "INSERT INTO grants (id, connection_id, connection_label, created_at, grant_json) VALUES ('g1', 'c1', 'Claude', 10, ?1)",
                params![grant],
            )
            .unwrap();
        }
        let store = open(dir.path());
        let old = store.activity(5).unwrap();
        assert_eq!((old[0].at, old[0].connection_label.as_str(), old[0].service.as_str()), (7, "Old", "gmail"));
        assert_eq!(old[0].detail, DETAIL_UNAVAILABLE, "rows from before the details existed have none");
        let version: i64 = store.lock().unwrap().query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(version, 9);
        assert_eq!(store.desktop_key("c1").unwrap(), None, "the desktop keys table exists");
        assert!(store.ap_profiles().unwrap().is_empty(), "the Autopilot tables exist");
        assert_eq!(store.ap_mode_row("").unwrap(), crate::autopilot::modes::ModeRow::default());
        assert_eq!(store.ap_memory("p").unwrap().len(), 0);
        assert!(store.mcp_servers().unwrap().is_empty(), "the MCP servers table exists");
        assert!(store.park("b1", crate::PendingKind::Blob, 1, 2, b"x").unwrap(), "uploads can be parked");
        assert!(store.park("j1", crate::PendingKind::Join, 1, 2, b"x").unwrap(), "other phones can be parked");
        let accounts: Vec<_> = store.accounts().unwrap().into_iter().map(|a| a.account).collect();
        assert_eq!(accounts, ["me@gmail.com"], "the one account of earlier versions is registered");
        assert_eq!(
            store.grants().unwrap()[0].grant.account.as_deref(),
            Some("me@gmail.com"),
            "and its grants are bound to it"
        );
        // Opening again must not try to migrate twice.
        drop(store);
        let again = open(dir.path());
        assert_eq!(again.activity(5).unwrap().len(), 1);
        again.meta_set("k", "v").unwrap();
        assert_eq!(again.meta_get("k").unwrap().as_deref(), Some("v"));
    }

    #[test]
    fn session_round_trips_encrypted() {
        let dir = tempfile::tempdir().unwrap();
        {
            let store = open(dir.path());
            assert!(!store.dek_was_reset(), "first creation is not a reset");
            store.save_session(&session()).unwrap();
        }
        let store = open(dir.path());
        assert!(!store.dek_was_reset());
        assert_eq!(store.load_session().unwrap(), Some(session()));
        drop(store);
        let raw = fs::read(dir.path().join("reins.sealed")).unwrap();
        assert!(!dir.path().join(DB_FILE).exists());
        let wal = fs::read(dir.path().join("reins.db-wal")).unwrap_or_default();
        for bytes in [raw, wal] {
            assert!(!bytes.windows(20).any(|w| w == b"REFRESH-TOKEN-SECRET"), "token stored in clear");
        }
        assert!(!format!("{:?}", session()).contains("SECRET"));
    }

    #[test]
    fn device_id_is_stable_uuid_v4() {
        let dir = tempfile::tempdir().unwrap();
        let first = open(dir.path()).device_id().unwrap();
        assert_eq!(uuid::Uuid::parse_str(&first).unwrap().get_version_num(), 4);
        assert_eq!(open(dir.path()).device_id().unwrap(), first);
    }

    #[test]
    fn lost_key_fails_closed_and_preserves_ciphertext() {
        let dir = tempfile::tempdir().unwrap();
        fill(&open(dir.path()));
        let before = fs::read(dir.path().join("reins.sealed")).unwrap();
        let wrapped = fs::read(dir.path().join(DEK_FILE)).unwrap();
        let broken = FakeKeys(AtomicU8::new(BROKEN));
        assert!(Store::open(dir.path(), &broken).is_err());
        assert_eq!(fs::read(dir.path().join("reins.sealed")).unwrap(), before);
        assert_eq!(fs::read(dir.path().join(DEK_FILE)).unwrap(), wrapped);
        fs::remove_file(dir.path().join(DEK_FILE)).unwrap();
        assert!(Store::open(dir.path(), &FakeKeys::default()).is_err());
        assert_eq!(fs::read(dir.path().join("reins.sealed")).unwrap(), before);
        fs::write(dir.path().join(DEK_FILE), wrapped).unwrap();
        assert_eq!(open(dir.path()).load_session().unwrap(), Some(session()));
    }

    #[test]
    fn replaced_key_file_cannot_open_or_overwrite_data() {
        let dir = tempfile::tempdir().unwrap();
        fill(&open(dir.path()));
        let before = fs::read(dir.path().join("reins.sealed")).unwrap();
        let (_, other) = Dek::generate().unwrap();
        fs::write(dir.path().join(DEK_FILE), FakeKeys::default().wrap(other.to_vec()).unwrap()).unwrap();
        assert!(Store::open(dir.path(), &FakeKeys::default()).is_err());
        assert_eq!(fs::read(dir.path().join("reins.sealed")).unwrap(), before);
    }

    #[test]
    fn locked_keystore_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        fill(&open(dir.path()));
        let locked = FakeKeys(AtomicU8::new(LOCKED));
        assert!(matches!(Store::open(dir.path(), &locked), Err(CoreError::Storage { .. })));
        let store = open(dir.path());
        assert!(!store.dek_was_reset());
        assert_eq!(store.load_session().unwrap(), Some(session()));
        assert_eq!(store.pending_rows(unix_now()).unwrap().len(), 1);
    }

    #[test]
    fn clear_session() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());
        store.save_session(&session()).unwrap();
        store.clear_session().unwrap();
        assert_eq!(store.load_session().unwrap(), None);
    }
}
