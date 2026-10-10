//! Account stores live only in memory while unlocked. Disk contains authenticated ciphertext, including metadata.
use super::{SCHEMA, Store, migrate, snapshot, write_atomically};
use crate::{
    CoreError,
    crypto::{Dek, VaultKey},
};
use ring::{digest, hmac};
use rusqlite::Connection;
use std::{
    fs,
    ops::{Deref, DerefMut},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicU64, Ordering},
    },
    time::SystemTime,
};
use zeroize::Zeroizing;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Owner {
    pub server: String,
    pub user_id: String,
}
impl Owner {
    pub fn aad(&self) -> String {
        format!("reins.account-state.v1:{}", serde_json::json!([self.server, self.user_id]))
    }
    pub fn id(&self) -> String {
        data_encoding::HEXLOWER.encode(digest::digest(&digest::SHA256, self.aad().as_bytes()).as_ref())
    }
}
pub(super) struct Persistence {
    pub path: PathBuf,
    pub aad: String,
    pub stamp: Mutex<Option<(SystemTime, u64)>>,
    pub failure: Mutex<Option<String>>,
    /// The lock other processes (the iOS notification extension) respect, shared by everything this store does at
    /// once: taken by the first user, given back by the last. While this process holds it nobody else writes the
    /// file, so the users after the first skip opening, locking and checking it.
    held: Mutex<Held>,
    /// The newest snapshot a write took and nobody has written yet, with its sequence number.
    pending: Mutex<Option<(u64, Zeroizing<Vec<u8>>)>>,
    /// The sequence number on disk. Held while sealing and writing, so writes reach the disk in order.
    written: Mutex<u64>,
    sequence: AtomicU64,
}
#[derive(Default)]
struct Held {
    file: Option<fs::File>,
    users: usize,
}
impl Persistence {
    pub fn new(path: PathBuf, aad: String) -> Self {
        Self {
            path,
            aad,
            stamp: Mutex::new(None),
            failure: Mutex::new(None),
            held: Mutex::new(Held::default()),
            pending: Mutex::new(None),
            written: Mutex::new(0),
            sequence: AtomicU64::new(0),
        }
    }
    /// Joins the users of the cross-process lock, taking it when nobody here holds it. True for the first user,
    /// who must check whether another process changed the file meanwhile.
    fn acquire(&self) -> Result<bool, CoreError> {
        let mut held = self.held.lock().unwrap_or_else(PoisonError::into_inner);
        if held.users > 0 {
            held.users += 1;
            return Ok(false);
        }
        if held.file.is_none() {
            held.file = Some(
                fs::OpenOptions::new()
                    .create(true)
                    .truncate(false)
                    .read(true)
                    .write(true)
                    .open(self.path.with_extension("lock"))
                    .map_err(|_| CoreError::storage("cannot lock account data"))?,
            );
        }
        held.file.as_ref().expect("lock file").lock().map_err(|_| CoreError::storage("cannot lock account data"))?;
        held.users = 1;
        Ok(true)
    }
    fn release(&self) {
        let mut held = self.held.lock().unwrap_or_else(PoisonError::into_inner);
        held.users -= 1;
        if held.users == 0
            && let Some(f) = &held.file
        {
            drop(f.unlock());
        }
    }
    fn fail(&self, e: &CoreError) {
        *self.failure.lock().unwrap_or_else(PoisonError::into_inner) = Some(e.to_string());
        log::error!("could not persist encrypted account data");
    }
}
pub(crate) struct StoreLock<'a> {
    store: &'a Store,
    /// Taken back on drop before a write is sealed, so reads wait for the snapshot only, not for the disk.
    conn: Option<MutexGuard<'a, Option<Connection>>>,
    changes: u64,
}
impl Deref for StoreLock<'_> {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        self.conn.as_ref().and_then(|c| c.as_ref()).expect("open store lock")
    }
}
impl DerefMut for StoreLock<'_> {
    fn deref_mut(&mut self) -> &mut Connection {
        self.conn.as_mut().and_then(|c| c.as_mut()).expect("open store lock")
    }
}
impl Drop for StoreLock<'_> {
    fn drop(&mut self) {
        let changed = self.total_changes() != self.changes;
        if changed {
            self.store.writes.fetch_add(1, Ordering::AcqRel);
        }
        let Some(p) = &self.store.persistence else {
            return;
        };
        // The copy is taken under the connection; sealing and the two fsyncs happen after letting it go. The call
        // still returns only once its write is on disk (or a later write that includes it is).
        let taken = if changed {
            snapshot(self, p).map(Some)
        } else {
            Ok(None)
        };
        drop(self.conn.take());
        match taken.and_then(|seq| seq.map_or(Ok(()), |seq| write_through(self.store, p, seq))) {
            Ok(()) => {}
            Err(e) => p.fail(&e),
        }
        p.release();
    }
}
fn stamp(path: &Path) -> Option<(SystemTime, u64)> {
    fs::metadata(path).ok().and_then(|m| m.modified().ok().map(|t| (t, m.len())))
}
/// Queues the data as it is now to be written; returns its sequence number. Call with the connection held.
fn snapshot(conn: &Connection, p: &Persistence) -> Result<u64, CoreError> {
    let seq = p.sequence.fetch_add(1, Ordering::AcqRel) + 1;
    let plain = Zeroizing::new(conn.serialize("main")?.to_vec());
    let mut pending = p.pending.lock().unwrap_or_else(PoisonError::into_inner);
    if pending.as_ref().is_none_or(|(queued, _)| *queued < seq) {
        *pending = Some((seq, plain));
    }
    Ok(seq)
}
/// Makes sure snapshot `seq` (or a newer one) is on disk. Writes that queue while another is being written share
/// the next write: the newest snapshot is the only one that still needs to reach the disk. Call holding the
/// cross-process lock.
fn write_through(store: &Store, p: &Persistence, seq: u64) -> Result<(), CoreError> {
    let mut written = p.written.lock().unwrap_or_else(PoisonError::into_inner);
    if *written >= seq {
        return Ok(());
    }
    let Some((newest, plain)) = p.pending.lock().unwrap_or_else(PoisonError::into_inner).take() else {
        return Ok(());
    };
    let sealed = store.seal(&p.aad, &plain)?;
    write_atomically(&p.path, &sealed)?;
    *p.stamp.lock().unwrap_or_else(PoisonError::into_inner) = stamp(&p.path);
    *written = newest;
    Ok(())
}
fn memory() -> Result<Connection, CoreError> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch(SCHEMA)?;
    migrate(&conn)?;
    conn.execute_batch("PRAGMA temp_store=MEMORY; PRAGMA secure_delete=ON;")?;
    Ok(conn)
}
impl Store {
    pub(crate) fn lock(&self) -> Result<StoreLock<'_>, CoreError> {
        self.check_owner()?;
        let mut conn = self.conn.lock().unwrap_or_else(PoisonError::into_inner);
        self.check_owner()?;
        if conn.is_none() {
            return Err(CoreError::NotLoggedIn);
        }
        if let Some(p) = &self.persistence {
            if p.failure.lock().unwrap_or_else(PoisonError::into_inner).is_some() {
                return Err(CoreError::storage("encrypted account data could not be saved"));
            }
            if p.acquire()?
                && let Err(e) = self.reload_if_changed(&mut conn, p)
            {
                p.release();
                return Err(e);
            }
        }
        let changes = conn.as_ref().expect("open").total_changes();
        Ok(StoreLock {
            store: self,
            conn: Some(conn),
            changes,
        })
    }
    /// Another process wrote the file since this one last read or wrote it: its copy replaces the one in memory.
    fn reload_if_changed(&self, conn: &mut Option<Connection>, p: &Persistence) -> Result<(), CoreError> {
        let current = stamp(&p.path);
        if current.is_some() && current != *p.stamp.lock().unwrap_or_else(PoisonError::into_inner) {
            let bytes = fs::read(&p.path).map_err(|_| CoreError::storage("cannot read account data"))?;
            let plain = Zeroizing::new(self.unseal(&p.aad, &bytes)?);
            let mut loaded = memory()?;
            loaded.deserialize_read_exact("main", std::io::Cursor::new(plain.as_slice()), plain.len(), false)?;
            loaded.execute_batch("PRAGMA temp_store=MEMORY; PRAGMA secure_delete=ON;")?;
            migrate(&loaded)?;
            *conn = Some(loaded);
            *p.stamp.lock().unwrap_or_else(PoisonError::into_inner) = current;
            self.writes.fetch_add(1, Ordering::AcqRel);
        }
        Ok(())
    }
    /// How many times the data has changed so far; equal counts mean equal data.
    pub(crate) fn write_count(&self) -> u64 {
        self.writes.load(Ordering::Acquire)
    }
    pub(crate) fn flush(&self) -> Result<(), CoreError> {
        if let Some(p) = &self.persistence
            && p.failure.lock().unwrap_or_else(PoisonError::into_inner).is_some()
        {
            return Err(CoreError::storage("encrypted account data could not be saved"));
        }
        Ok(())
    }
    pub(crate) fn close(&self) -> Result<(), CoreError> {
        self.active.store(false, Ordering::Release);
        // Waits for whoever holds the connection; a write among them has queued its snapshot by then.
        drop(self.conn.lock().unwrap_or_else(PoisonError::into_inner).take());
        if let Some(p) = &self.persistence {
            // A write that let go of the connection may still be sealing: it (or this) finishes before the key goes.
            let latest = p.sequence.load(Ordering::Acquire);
            let written = p.acquire().and_then(|_| {
                let result = write_through(self, p, latest);
                p.release();
                result
            });
            if let Err(e) = written {
                p.fail(&e);
            }
        }
        let persisted = self.flush();
        *self.dek.lock().unwrap_or_else(PoisonError::into_inner) = None;
        persisted
    }
    pub(crate) fn account(
        control: Arc<Self>,
        directory: &Path,
        owner: Owner,
        key: &VaultKey,
    ) -> Result<Self, CoreError> {
        fs::create_dir_all(directory).map_err(|_| CoreError::storage("cannot create account directory"))?;
        let raw = key.to_bytes();
        let derived = Zeroizing::new(
            hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, &raw), owner.aad().as_bytes()).as_ref().to_vec(),
        );
        let store = Self {
            active: std::sync::atomic::AtomicBool::new(true),
            conn: Mutex::new(Some(memory()?)),
            dek: Mutex::new(Some(Dek::from_bytes(&derived)?)),
            dek_was_reset: false,
            writes: AtomicU64::new(0),
            persistence: Some(Persistence::new(directory.join(format!("{}.sealed", owner.id())), owner.aad())),
            control: Some(control),
            control_owner: Mutex::new(None),
            owner: Some(owner),
        };
        // Acquiring the first lock loads and authenticates the file if it exists. Never fall back to another account.
        drop(store.lock()?);
        Ok(store)
    }
    pub(crate) fn ephemeral(control: Option<Arc<Self>>) -> Result<Self, CoreError> {
        let (dek, _) = Dek::generate()?;
        Ok(Self {
            active: std::sync::atomic::AtomicBool::new(true),
            conn: Mutex::new(Some(memory()?)),
            dek: Mutex::new(Some(dek)),
            dek_was_reset: false,
            writes: AtomicU64::new(0),
            persistence: None,
            control,
            control_owner: Mutex::new(None),
            owner: None,
        })
    }
    pub(crate) fn export_account(&self) -> Result<Zeroizing<Vec<u8>>, CoreError> {
        snapshot::export(&*self.lock()?, true)
    }
    pub(crate) fn has_account_rows(plain: &[u8]) -> Result<bool, CoreError> {
        snapshot::has_rows(plain)
    }
    pub(crate) fn import_account(&self, plain: &[u8]) -> Result<(), CoreError> {
        snapshot::import(&mut *self.lock()?, plain, true)
    }
    pub(crate) fn seal_account(&self, revision: u64, plain: &[u8]) -> Result<Vec<u8>, CoreError> {
        self.check_owner()?;
        let owner = self.owner.as_ref().ok_or(CoreError::NotLoggedIn)?;
        self.seal(&format!("{}:revision:{revision}", owner.aad()), plain)
    }
    pub(crate) fn unseal_account(&self, revision: u64, blob: &[u8]) -> Result<Zeroizing<Vec<u8>>, CoreError> {
        self.check_owner()?;
        let owner = self.owner.as_ref().ok_or(CoreError::NotLoggedIn)?;
        self.unseal(&format!("{}:revision:{revision}", owner.aad()), blob).map(Zeroizing::new)
    }
}

pub(super) fn protect(store: &Store, directory: &Path) -> Result<(), CoreError> {
    let p = store.persistence.as_ref().expect("protected store");
    let guard = store.lock()?;
    let written = snapshot(&guard, p).and_then(|seq| write_through(store, p, seq));
    drop(guard);
    written?;
    // Legacy SQLite and its WAL are removed only after the complete encrypted snapshot is durable.
    for name in ["reins.db", "reins.db-wal", "reins.db-shm"] {
        let path = directory.join(name);
        if path.exists() {
            fs::remove_file(path).map_err(|_| CoreError::storage("cannot remove legacy plaintext store"))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::tests::{FakeKeys, fill};

    fn owner(id: &str) -> Owner {
        Owner {
            server: "https://one.example".to_owned(),
            user_id: id.to_owned(),
        }
    }
    fn open_account(root: &Arc<Store>, dir: &Path, owner: Owner, key: &VaultKey) -> Store {
        root.meta_set("active-account", &owner.id()).unwrap();
        Store::account(Arc::clone(root), dir, owner, key).unwrap()
    }
    #[test]
    fn all_metadata_is_ciphertext_and_only_its_owner_can_reopen_it() {
        let dir = tempfile::tempdir().unwrap();
        let root = Arc::new(Store::open(dir.path(), &FakeKeys::default()).unwrap());
        let account_dir = dir.path().join("accounts");
        let key = VaultKey::from_bytes(&[7; 64]).unwrap();
        let a = open_account(&root, &account_dir, owner("a"), &key);
        fill(&a);
        a.insert_grant(
            &crate::store::grants::tests::read_grant("sensitive-grant", "private-computer", None),
            "Private Laptop",
        )
        .unwrap();
        a.add_account("github", "private-person", 1).unwrap();
        a.secret_put("github", "private-person", b"integration-secret").unwrap();
        a.pin_desktop_key("private-computer", "desktop-public-key", "Laptop", 1).unwrap();
        a.flush().unwrap();
        let sealed = fs::read(account_dir.join(format!("{}.sealed", owner("a").id()))).unwrap();
        for plaintext in [
            b"ChatGPT".as_slice(),
            b"Private Laptop",
            b"sensitive-grant",
            b"private-person",
            b"integration-secret",
            b"desktop-public-key",
        ] {
            assert!(!sealed.windows(plaintext.len()).any(|w| w == plaintext));
        }
        assert!(!dir.path().join("reins.db").exists());
        let b = open_account(&root, &account_dir, owner("b"), &key);
        assert_eq!(b.activity(50).unwrap().len(), 0);
        assert_eq!(b.grants().unwrap().len(), 0);
        assert_eq!(b.accounts().unwrap().len(), 0);
        assert_eq!(b.secret_get("github", "private-person").unwrap(), None);
        assert!(a.activity(50).is_err(), "old store cannot be read after the active owner changes");
        a.close().unwrap();
        assert!(a.unseal_account(1, &sealed).is_err());
        let a = open_account(&root, &account_dir, owner("a"), &key);
        assert_eq!(a.activity(50).unwrap().len(), 1);
        assert_eq!(a.grants().unwrap().len(), 1);
        assert_eq!(a.secret_get("github", "private-person").unwrap().unwrap(), b"integration-secret");
        assert_eq!(a.desktop_key("private-computer").unwrap().as_deref(), Some("desktop-public-key"));
    }
    #[test]
    fn copied_ciphertext_wrong_key_and_wrong_server_fail_closed() {
        let dir = tempfile::tempdir().unwrap();
        let root = Arc::new(Store::open(dir.path(), &FakeKeys::default()).unwrap());
        let key = VaultKey::from_bytes(&[7; 64]).unwrap();
        let a = open_account(&root, dir.path(), owner("a"), &key);
        a.meta_set("private", "secret metadata").unwrap();
        let sealed = fs::read(dir.path().join(format!("{}.sealed", owner("a").id()))).unwrap();
        let other_key = VaultKey::from_bytes(&[8; 64]).unwrap();
        assert!(Store::account(Arc::clone(&root), dir.path(), owner("a"), &other_key).is_err());
        for other in [
            owner("b"),
            Owner {
                server: "https://two.example".to_owned(),
                user_id: "a".to_owned(),
            },
        ] {
            fs::write(dir.path().join(format!("{}.sealed", other.id())), &sealed).unwrap();
            root.meta_set("active-account", &other.id()).unwrap();
            assert!(Store::account(Arc::clone(&root), dir.path(), other, &key).is_err());
        }
    }
    #[test]
    fn encrypted_portable_state_restores_on_another_device_without_installation_keys() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let root_a = Arc::new(Store::open(first.path(), &FakeKeys::default()).unwrap());
        let root_b = Arc::new(Store::open(second.path(), &FakeKeys::default()).unwrap());
        let key = VaultKey::from_bytes(&[7; 64]).unwrap();
        let a = open_account(&root_a, first.path(), owner("a"), &key);
        fill(&a);
        a.secret_put("github", "person", b"token").unwrap();
        a.secret_put("reins.account-secret", "a", b"account secret").unwrap();
        a.secret_put("payments.provider", "privacy", b"virtual card key").unwrap();
        a.secret_put("payments.mandate-key", "this", b"mandate seed").unwrap();
        let sealed = a.seal_account(1, &a.export_account().unwrap()).unwrap();
        let b = open_account(&root_b, second.path(), owner("a"), &key);
        assert!(b.unseal_account(2, &sealed).is_err(), "the server cannot relabel an old revision");
        b.import_account(&b.unseal_account(1, &sealed).unwrap()).unwrap();
        assert_eq!(b.activity(50).unwrap().len(), 1);
        assert_eq!(b.secret_get("github", "person").unwrap().unwrap(), b"token");
        assert_eq!(b.secret_get("reins.account-secret", "a").unwrap(), None);
        assert_eq!(
            b.secret_get("payments.provider", "privacy").unwrap(),
            None,
            "a card provider key stays on its phone"
        );
        assert!(
            b.secret_get("payments.mandate-key", "this").unwrap().is_some(),
            "the account signs mandates with one key"
        );
        assert_eq!(b.pending_rows(super::super::unix_now()).unwrap().len(), 0);
        assert_ne!(a.device_id().unwrap(), b.device_id().unwrap());
    }

    #[test]
    fn imported_state_may_not_carry_a_card_provider_key() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let root_a = Arc::new(Store::open(first.path(), &FakeKeys::default()).unwrap());
        let root_b = Arc::new(Store::open(second.path(), &FakeKeys::default()).unwrap());
        let key = VaultKey::from_bytes(&[7; 64]).unwrap();
        let a = open_account(&root_a, first.path(), owner("a"), &key);
        a.secret_put("github", "person", b"token").unwrap();
        let mut state: serde_json::Value = serde_json::from_slice(&a.export_account().unwrap()).unwrap();
        // Whoever wrote the state (another phone, or anyone with the account key) adds a provider key of their own.
        let rows = state["tables"]["secrets"]["rows"].as_array_mut().unwrap();
        let mut planted = rows[0].clone();
        planted[0] = serde_json::json!({"Text": "payments.provider"});
        rows.push(planted);
        let b = open_account(&root_b, second.path(), owner("a"), &key);
        let err = b.import_account(&serde_json::to_vec(&state).unwrap()).unwrap_err();
        assert!(err.to_string().contains("installation key"), "{err}");
        assert_eq!(b.secret_get("payments.provider", "person").unwrap(), None);
    }

    #[test]
    fn late_token_refresh_and_logout_cannot_replace_the_next_accounts_session() {
        let dir = tempfile::tempdir().unwrap();
        let root = Store::open(dir.path(), &FakeKeys::default()).unwrap();
        let session = |id: &str| super::super::StoredSession {
            server_url: owner(id).server,
            email: format!("{id}@example.com"),
            refresh_token: format!("{id}-private-refresh").into(),
        };
        root.bind_account_session(&session("a"), &owner("a")).unwrap();
        root.bind_account_session(&session("b"), &owner("b")).unwrap();
        assert_eq!(root.save_session_for(&session("a"), &owner("a")).unwrap_err(), CoreError::NotLoggedIn);
        assert_eq!(root.clear_session_for(Some(&owner("a"))).unwrap_err(), CoreError::NotLoggedIn);
        assert_eq!(root.load_session().unwrap().unwrap().email, "b@example.com");
    }
    #[test]
    fn concurrent_writes_all_reach_the_disk_and_a_newer_copy_is_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let root = Arc::new(Store::open(dir.path(), &FakeKeys::default()).unwrap());
        let key = VaultKey::from_bytes(&[7; 64]).unwrap();
        let a = Arc::new(open_account(&root, dir.path(), owner("a"), &key));
        let writers: Vec<_> = (0..8)
            .map(|t| {
                let a = Arc::clone(&a);
                std::thread::spawn(move || {
                    for i in 0..20 {
                        a.meta_set(&format!("w{t}-{i}"), "v").unwrap();
                        // Reads go on while others seal and write.
                        a.meta_get("w0-0").unwrap();
                    }
                })
            })
            .collect();
        for w in writers {
            w.join().unwrap();
        }
        a.close().unwrap();
        let reopened = open_account(&root, dir.path(), owner("a"), &key);
        for t in 0..8 {
            for i in 0..20 {
                assert_eq!(reopened.meta_get(&format!("w{t}-{i}")).unwrap().as_deref(), Some("v"), "w{t}-{i}");
            }
        }
    }
    #[test]
    fn another_process_sees_each_write_and_its_own_are_kept() {
        // Two stores on one file stand for the app and the iOS notification extension.
        let dir = tempfile::tempdir().unwrap();
        let root = Arc::new(Store::open(dir.path(), &FakeKeys::default()).unwrap());
        let key = VaultKey::from_bytes(&[7; 64]).unwrap();
        let app = open_account(&root, dir.path(), owner("a"), &key);
        let extension = Store::account(Arc::clone(&root), dir.path(), owner("a"), &key).unwrap();
        app.meta_set("from-app", "1").unwrap();
        assert_eq!(extension.meta_get("from-app").unwrap().as_deref(), Some("1"));
        extension.meta_set("from-extension", "2").unwrap();
        assert_eq!(app.meta_get("from-extension").unwrap().as_deref(), Some("2"));
        app.meta_set("again", "3").unwrap();
        assert_eq!(extension.meta_get("again").unwrap().as_deref(), Some("3"));
        assert_eq!(extension.meta_get("from-app").unwrap().as_deref(), Some("1"));
    }
}
