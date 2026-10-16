//! Account stores live only in memory while unlocked. Disk contains authenticated ciphertext, including metadata.
use super::{Store, prepare, replace_file, snapshot, sync_parent, write_atomically};
use crate::{
    CoreError, StoreLockGuard,
    crypto::{Dek, VaultKey},
};
use ring::{digest, hmac};
use rusqlite::Connection;
use std::{
    fs,
    ops::{Deref, DerefMut},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, MutexGuard, PoisonError, RwLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime},
};
use zeroize::Zeroizing;

/// How long taking the cross-process lock waits for another process before the call fails as busy, unless the
/// platform chose its own wait (`install_store_lock_guard`). Another process holds it for milliseconds at a time; one
/// that holds it longer is stuck (suspended, say), and waiting on it forever would stall this process as well.
const DEFAULT_LOCK_WAIT: Duration = Duration::from_secs(10);
/// What a call fails with when the lock stays taken for the whole wait. Transient: the same call succeeds later.
pub(crate) const STORE_BUSY: &str = "account data is busy in another part of the app; try again";
/// What a call fails with when the platform's guard refuses the lock (the process is being suspended).
pub(crate) const STORE_SUSPENDING: &str = "the app is being suspended; try again";

/// The platform's guard over the cross-process lock and the wait, for the stores this process opens from now on.
static LOCK_GUARD: RwLock<Option<(Arc<dyn StoreLockGuard>, Duration)>> = RwLock::new(None);

/// Installs `guard` for every store opened after this (call it before the first `ReinsCore`).
pub(crate) fn install_lock_guard(guard: Arc<dyn StoreLockGuard>, wait: Duration) {
    *LOCK_GUARD.write().unwrap_or_else(PoisonError::into_inner) = Some((guard, wait));
}

/// Takes the exclusive lock on `file`, waiting at most `wait` for another process to give it back. Never blocks
/// without a bound, so that a process suspended with the lock held cannot stall this one.
fn lock_within(file: &fs::File, wait: Duration) -> Result<(), CoreError> {
    let started = Instant::now();
    let mut pause = Duration::from_millis(1);
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(()),
            Err(fs::TryLockError::WouldBlock) => {}
            Err(fs::TryLockError::Error(_)) => return Err(CoreError::storage("cannot lock account data")),
        }
        let waited = started.elapsed();
        if waited >= wait {
            return Err(CoreError::storage(STORE_BUSY));
        }
        std::thread::sleep(pause.min(wait.saturating_sub(waited)));
        pause = (pause * 2).min(Duration::from_millis(25));
    }
}

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
    /// The platform's guard, told when the lock is taken and given back (iOS keeps the process from being suspended
    /// in between), and how long taking it may wait.
    guard: Option<Arc<dyn StoreLockGuard>>,
    wait: Duration,
}
#[derive(Default)]
struct Held {
    file: Option<fs::File>,
    users: usize,
}
impl Persistence {
    pub fn new(path: PathBuf, aad: String) -> Self {
        let (guard, wait) = LOCK_GUARD
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map_or((None, DEFAULT_LOCK_WAIT), |(g, w)| (Some(Arc::clone(g)), *w));
        Self {
            path,
            aad,
            stamp: Mutex::new(None),
            failure: Mutex::new(None),
            held: Mutex::new(Held::default()),
            pending: Mutex::new(None),
            written: Mutex::new(0),
            sequence: AtomicU64::new(0),
            guard,
            wait,
        }
    }
    /// Joins the users of the cross-process lock, taking it when nobody here holds it. True for the first user,
    /// who must check whether another process changed the file meanwhile.
    ///
    /// Taking it may wait [`Self::wait`] for another process, then fails as busy ([`STORE_BUSY`]); the guard may
    /// refuse it ([`STORE_SUSPENDING`]). Either way nothing is held and the caller fails without changing anything.
    fn acquire(&self) -> Result<bool, CoreError> {
        let mut held = self.held.lock().unwrap_or_else(PoisonError::into_inner);
        if held.users > 0 {
            held.users += 1;
            return Ok(false);
        }
        let file = match held.file.take() {
            Some(file) => file,
            None => fs::OpenOptions::new()
                .create(true)
                .truncate(false)
                .read(true)
                .write(true)
                .open(self.path.with_extension("lock"))
                .map_err(|_| CoreError::storage("cannot lock account data"))?,
        };
        let locked = self.lock_guarded(&file);
        held.file = Some(file);
        locked?;
        held.users = 1;
        Ok(true)
    }
    #[cfg(test)]
    fn guarded(&mut self, guard: Arc<dyn StoreLockGuard>, wait: Duration) {
        self.guard = Some(guard);
        self.wait = wait;
    }
    /// Takes the file lock with the guard's promise around it.
    fn lock_guarded(&self, file: &fs::File) -> Result<(), CoreError> {
        if let Some(guard) = &self.guard
            && !guard.locking()
        {
            return Err(CoreError::storage(STORE_SUSPENDING));
        }
        let locked = lock_within(file, self.wait);
        if locked.is_err()
            && let Some(guard) = &self.guard
        {
            guard.unlocked();
        }
        locked
    }
    fn release(&self) {
        let mut held = self.held.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(users) = held.users.checked_sub(1) else {
            return;
        };
        held.users = users;
        if users == 0
            && let Some(f) = &held.file
        {
            drop(f.unlock());
            if let Some(guard) = &self.guard {
                guard.unlocked();
            }
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
    /// Whether this holds a share of the cross-process lock (`Store::lock`); a read (`Store::read`) gave it back once
    /// it had checked for another process's copy.
    shared: bool,
}
// The `expect`s below cannot fail: `Store::lock` hands out a `StoreLock` only for an open connection (and a reload
// only ever replaces it with another), and the guard is taken out of `conn` only in `drop`, after the last use.
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
impl StoreLock<'_> {
    /// Everything written under this lock so far went to the `temp` schema (this process's memory, never serialized):
    /// there is nothing to seal and write, and the data others see has not changed. Only for a lock that has written
    /// nothing else.
    pub(super) fn temp_only(&mut self) {
        self.changes = self.total_changes();
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
        if changed && !self.shared {
            // A read (`Store::read`) that wrote: a mistake in the caller. What it wrote is kept, under the lock taken
            // now; another process may have written in between.
            debug_assert!(false, "a write through Store::read");
            log::error!("a write through a store read; writing it under the lock now");
            if let Err(e) = p.acquire() {
                drop(self.conn.take());
                p.fail(&e);
                return;
            }
            self.shared = true;
        }
        // The copy is taken under the connection; sealing and the fsyncs happen after letting it go, and the
        // directory's fsync after giving back the cross-process lock too, which other processes wait on (iOS: a
        // process must not be suspended holding it). The call still returns only once its write is on disk (or a
        // later write that includes it is).
        let taken = if changed {
            snapshot(self, p).map(Some)
        } else {
            Ok(None)
        };
        drop(self.conn.take());
        let written = taken.and_then(|seq| seq.map_or(Ok(None), |seq| write_through(self.store, p, seq)));
        if self.shared {
            p.release();
        }
        if let Err(e) = written.and_then(|unsynced| unsynced.map_or(Ok(()), Unsynced::sync)) {
            p.fail(&e);
        }
    }
}
/// Another process's sealed file, read under the cross-process lock.
struct SealedCopy {
    sealed: Vec<u8>,
    stamp: Option<(SystemTime, u64)>,
}
/// The platform guard held for a window that holds other locks in the shared container, without the store's own: the
/// legacy SQLite file, whose connection keeps SQLite's file locks while it is open. Given back on drop.
pub(super) struct Awake(Option<Arc<dyn StoreLockGuard>>);
pub(super) fn stay_awake() -> Result<Awake, CoreError> {
    let guard = LOCK_GUARD.read().unwrap_or_else(PoisonError::into_inner).as_ref().map(|(g, _)| Arc::clone(g));
    if let Some(g) = &guard
        && !g.locking()
    {
        return Err(CoreError::storage(STORE_SUSPENDING));
    }
    Ok(Awake(guard))
}
impl Drop for Awake {
    fn drop(&mut self) {
        if let Some(g) = &self.0 {
            g.unlocked();
        }
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
/// Makes sure snapshot `seq` (or a newer one) is in place. Writes that queue while another is being written share
/// the next write: the newest snapshot is the only one that still needs to reach the disk. Call holding the
/// cross-process lock.
///
/// When this call wrote the file it returns the write's turn (`Some`): give back the cross-process lock, then
/// [`Unsynced::sync`]. Other processes read the new file as soon as it is renamed into place; only the rename's
/// durability (the directory's fsync) is left, and it no longer keeps them waiting. Writes in this process wait for
/// it, so none returns before what it wrote is on disk.
fn write_through<'p>(store: &Store, p: &'p Persistence, seq: u64) -> Result<Option<Unsynced<'p>>, CoreError> {
    let mut written = p.written.lock().unwrap_or_else(PoisonError::into_inner);
    if *written >= seq {
        return Ok(None);
    }
    let Some((newest, plain)) = p.pending.lock().unwrap_or_else(PoisonError::into_inner).take() else {
        return Ok(None);
    };
    let sealed = store.seal(&p.aad, &plain)?;
    replace_file(&p.path, &sealed)?;
    *p.stamp.lock().unwrap_or_else(PoisonError::into_inner) = stamp(&p.path);
    *written = newest;
    Ok(Some(Unsynced {
        path: &p.path,
        _turn: written,
    }))
}
/// A write renamed into place whose directory is not synced yet; holds the write turn until it is.
#[must_use]
struct Unsynced<'p> {
    path: &'p Path,
    _turn: MutexGuard<'p, u64>,
}
impl Unsynced<'_> {
    fn sync(self) -> Result<(), CoreError> {
        sync_parent(self.path)
    }
}
pub(super) fn memory() -> Result<Connection, CoreError> {
    let conn = Connection::open_in_memory()?;
    prepare(&conn)?;
    Ok(conn)
}
impl Store {
    /// The store's connection, holding the cross-process lock until the guard drops (and what it wrote is on disk): for
    /// anything that may write. Another process's newer copy, if any, is loaded first.
    pub(crate) fn lock(&self) -> Result<StoreLock<'_>, CoreError> {
        self.open_lock(true)
    }
    /// The store's connection for reading only (or writing the `temp` schema, which is never saved). The
    /// cross-process lock is held just long enough to check for another process's newer copy and read its bytes;
    /// opening and loading that copy, and the reads, need only this process's lock. A shorter hold: iOS ends a process
    /// suspended while it holds the lock.
    pub(crate) fn read(&self) -> Result<StoreLock<'_>, CoreError> {
        self.open_lock(false)
    }
    fn open_lock(&self, write: bool) -> Result<StoreLock<'_>, CoreError> {
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
            // Only the first user here checks: while this process holds the lock, nobody else writes the file.
            let changed = if p.acquire()? {
                Self::read_if_changed(p)
            } else {
                Ok(None)
            };
            if !write {
                p.release();
            }
            let loaded = changed.and_then(|copy| copy.map_or(Ok(()), |copy| self.load_copy(&mut conn, p, &copy)));
            if let Err(e) = loaded {
                if write {
                    p.release();
                }
                return Err(e);
            }
        }
        // Open: checked above, and a reload only ever puts a connection in its place.
        let changes = conn.as_ref().map_or(0, Connection::total_changes);
        Ok(StoreLock {
            store: self,
            conn: Some(conn),
            changes,
            shared: write && self.persistence.is_some(),
        })
    }
    /// The file's bytes and stamp, when another process wrote it since this one last read or wrote it. Call holding
    /// the cross-process lock: only the `stat` and the read happen under it.
    fn read_if_changed(p: &Persistence) -> Result<Option<SealedCopy>, CoreError> {
        let current = stamp(&p.path);
        if current.is_none() || current == *p.stamp.lock().unwrap_or_else(PoisonError::into_inner) {
            return Ok(None);
        }
        let sealed = fs::read(&p.path).map_err(|_| CoreError::storage("cannot read account data"))?;
        Ok(Some(SealedCopy {
            sealed,
            stamp: current,
        }))
    }
    /// Another process's copy replaces the one in memory. Needs only this process's lock (`conn`): the bytes were
    /// read under the cross-process one, and a newer file since has another stamp, which the next lock loads.
    fn load_copy(&self, conn: &mut Option<Connection>, p: &Persistence, copy: &SealedCopy) -> Result<(), CoreError> {
        let plain = Zeroizing::new(self.unseal(&p.aad, &copy.sealed)?);
        let mut loaded = Connection::open_in_memory()?;
        // The copy replaces the whole database, schema included: it has the tables of whoever sealed it.
        loaded.deserialize_read_exact("main", std::io::Cursor::new(plain.as_slice()), plain.len(), false)?;
        // Sealed by an older version: what it exported then, to tell whether it was in sync before migrating.
        let before_migrating = (self.owner.is_some() && super::schema_version(&loaded)? < super::SCHEMA_VERSION)
            .then(|| snapshot::export_present(&loaded).ok())
            .flatten();
        prepare(&loaded)?;
        if let Some(pre) = before_migrating {
            super::sync::rebaseline(&loaded, &pre)?;
        }
        *conn = Some(loaded);
        *p.stamp.lock().unwrap_or_else(PoisonError::into_inner) = copy.stamp;
        self.writes.fetch_add(1, Ordering::AcqRel);
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
                result.and_then(|unsynced| unsynced.map_or(Ok(()), Unsynced::sync))
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
    #[cfg(test)]
    pub(crate) fn import_account(&self, plain: &[u8]) -> Result<(), CoreError> {
        snapshot::import(&mut *self.lock()?, plain, true)
    }
    /// A file of this account's besides its data (`<owner>.<name>.sealed`), sealed with the account's key: the
    /// account sync's merge base and backup. `None` for a store that is not an account's.
    fn side_file(&self, name: &str) -> Option<(PathBuf, &Store, String)> {
        let p = self.persistence.as_ref()?;
        let owner = self.owner.as_ref()?;
        let control = self.control.as_deref()?;
        Some((p.path.with_extension(format!("{name}.sealed")), control, owner.id()))
    }
    /// Reads side file `name`, sealed with this installation's device key (never the account's, which other phones
    /// have) and authenticated with `context` and the account's id: a file of another account, another revision or
    /// another purpose does not open.
    pub(super) fn read_side(&self, name: &str, context: &str) -> Result<Option<Zeroizing<Vec<u8>>>, CoreError> {
        self.check_owner()?;
        let Some((path, device, account)) = self.side_file(name) else {
            return Ok(None);
        };
        match fs::read(&path) {
            Ok(sealed) => device
                .unseal(&format!("reins.{name}.v1:{account}:{context}"), &sealed)
                .map(|plain| Some(Zeroizing::new(plain))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(_) => Err(CoreError::storage("cannot read account data")),
        }
    }
    pub(super) fn write_side(&self, name: &str, context: &str, plain: &[u8]) -> Result<(), CoreError> {
        self.check_owner()?;
        let Some((path, device, account)) = self.side_file(name) else {
            return Ok(());
        };
        write_atomically(&path, &device.seal(&format!("reins.{name}.v1:{account}:{context}"), plain)?)
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
    let p = store.persistence.as_ref().ok_or_else(|| CoreError::storage("cannot protect an unsaved store"))?;
    let guard = store.lock()?;
    let written = snapshot(&guard, p).and_then(|seq| write_through(store, p, seq));
    drop(guard);
    if let Some(unsynced) = written? {
        unsynced.sync()?;
    }
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
    /// Makes `store`'s sealed file what a version from before entry snapshots (#71) wrote: schema version 9 and no
    /// `audit_snapshots` table.
    fn seal_as_before_snapshots(store: &Store) {
        store.lock().unwrap().execute_batch("DROP TABLE audit_snapshots; PRAGMA user_version = 9;").unwrap();
        // Schema changes are not row changes: a write makes the store seal the database as it now is.
        store.meta_set("sealed-by", "an older version").unwrap();
    }

    /// What approving a request writes: an entry with its approval's snapshot.
    fn approved_entry() -> crate::store::AuditRecord {
        let parked: crate::views::ParkedRequest = serde_json::from_value(serde_json::json!({
            "request": {"v": 1, "id": "r1", "connection_id": "c1", "connection_label": "Claude", "created_at": 9,
                        "call": {"tool": "gmail_search", "query": "from:bank", "max_results": 10}},
            "messages": [], "covered": {}, "account": null
        }))
        .unwrap();
        crate::store::AuditRecord {
            seq: 0,
            at: 10,
            connection_id: "c1".to_owned(),
            connection_label: "Claude".to_owned(),
            action: "search".to_owned(),
            outcome: "released".to_owned(),
            detail: "approved: 1 message".to_owned(),
            grant_id: None,
            service: "gmail".to_owned(),
            account: None,
            count: 1,
            info: crate::store::AuditInfo {
                snapshot: Some(Box::new(parked)),
                ..crate::store::AuditInfo::default()
            },
            op: String::new(),
        }
    }

    fn approves_and_keeps_the_snapshot(store: &Store) {
        let seq = store.append_audit(&approved_entry()).expect("the approval is logged");
        assert!(store.audit_snapshot(seq).unwrap().is_some(), "and its approval is kept");
    }

    #[test]
    fn a_device_store_sealed_before_snapshots_existed_gets_the_table() {
        let dir = tempfile::tempdir().unwrap();
        seal_as_before_snapshots(&Store::open(dir.path(), &FakeKeys::default()).unwrap());
        let store = Store::open(dir.path(), &FakeKeys::default()).unwrap();
        approves_and_keeps_the_snapshot(&store);
        let version: i64 = store.lock().unwrap().query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(version, i64::try_from(super::super::MIGRATIONS.len()).unwrap() + 1);
        // And the table is sealed with the rest: the next open has it without upgrading again.
        drop(store);
        approves_and_keeps_the_snapshot(&Store::open(dir.path(), &FakeKeys::default()).unwrap());
    }

    #[test]
    fn an_account_store_sealed_before_snapshots_existed_gets_the_table() {
        let dir = tempfile::tempdir().unwrap();
        let root = Arc::new(Store::open(dir.path(), &FakeKeys::default()).unwrap());
        let key = VaultKey::from_bytes(&[7; 64]).unwrap();
        let old = open_account(&root, dir.path(), owner("a"), &key);
        fill(&old);
        seal_as_before_snapshots(&old);
        old.close().unwrap();
        let store = open_account(&root, dir.path(), owner("a"), &key);
        assert_eq!(store.activity(50).unwrap().len(), 1, "what the older version kept is still there");
        approves_and_keeps_the_snapshot(&store);
    }

    #[test]
    fn a_copy_written_by_another_process_before_snapshots_existed_gets_the_table() {
        // The app is up to date; the other process (the iOS notification extension) still runs the older version.
        let dir = tempfile::tempdir().unwrap();
        let root = Arc::new(Store::open(dir.path(), &FakeKeys::default()).unwrap());
        let key = VaultKey::from_bytes(&[7; 64]).unwrap();
        let app = open_account(&root, dir.path(), owner("a"), &key);
        let extension = Store::account(Arc::clone(&root), dir.path(), owner("a"), &key).unwrap();
        seal_as_before_snapshots(&extension);
        approves_and_keeps_the_snapshot(&app);
    }

    #[test]
    fn an_approval_is_logged_even_when_its_snapshot_cannot_be_kept() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path(), &FakeKeys::default()).unwrap();
        store.lock().unwrap().execute_batch("DROP TABLE audit_snapshots").unwrap();
        let seq = store.append_audit(&approved_entry()).expect("a lost snapshot never fails the approval");
        assert_eq!(store.audit_entry(seq).unwrap().unwrap().detail, "approved: 1 message");
        assert!(store.audit_snapshot(seq).unwrap().is_none(), "the entry opens without its approval");
    }

    /// A platform guard that records what it is told, and whether the file was free at that moment (tried from
    /// another open file, as another process would).
    #[derive(Default)]
    struct Recorder {
        refuse: std::sync::atomic::AtomicBool,
        lock_file: Mutex<Option<PathBuf>>,
        calls: Mutex<Vec<(&'static str, bool)>>,
    }
    impl Recorder {
        fn free(&self) -> bool {
            let path = self.lock_file.lock().unwrap().clone().unwrap();
            let other = fs::OpenOptions::new().read(true).write(true).open(path).unwrap();
            match other.try_lock() {
                Ok(()) => {
                    other.unlock().unwrap();
                    true
                }
                Err(fs::TryLockError::WouldBlock) => false,
                Err(fs::TryLockError::Error(e)) => panic!("{e}"),
            }
        }
        fn calls(&self) -> Vec<(&'static str, bool)> {
            std::mem::take(&mut *self.calls.lock().unwrap())
        }
    }
    impl StoreLockGuard for Recorder {
        fn locking(&self) -> bool {
            let free = self.free();
            self.calls.lock().unwrap().push(("locking", free));
            !self.refuse.load(Ordering::SeqCst)
        }
        fn unlocked(&self) {
            let free = self.free();
            self.calls.lock().unwrap().push(("unlocked", free));
        }
    }
    /// An account store whose lock is watched by a [`Recorder`], with `wait` to take it.
    fn guarded_account(dir: &Path, wait: Duration) -> (Arc<Store>, Store, Arc<Recorder>, PathBuf) {
        let root = Arc::new(Store::open(dir, &FakeKeys::default()).unwrap());
        let key = VaultKey::from_bytes(&[7; 64]).unwrap();
        let mut store = open_account(&root, dir, owner("a"), &key);
        let recorder = Arc::new(Recorder::default());
        let p = store.persistence.as_mut().unwrap();
        let lock_file = p.path.with_extension("lock");
        *recorder.lock_file.lock().unwrap() = Some(lock_file.clone());
        p.guarded(Arc::<Recorder>::clone(&recorder), wait);
        (root, store, recorder, lock_file)
    }

    #[test]
    fn the_file_lock_is_held_only_between_the_guards_locking_and_unlocked() {
        let dir = tempfile::tempdir().unwrap();
        let (_root, store, recorder, _) = guarded_account(dir.path(), DEFAULT_LOCK_WAIT);
        store.meta_set("k", "v").unwrap();
        assert_eq!(store.meta_get("k").unwrap().as_deref(), Some("v"));
        // Free when told it is about to be taken, and when told it was given back (a write, then a read).
        assert_eq!(recorder.calls(), [("locking", true), ("unlocked", true), ("locking", true), ("unlocked", true)]);
        let held = store.lock().unwrap();
        assert!(!recorder.free(), "held in between");
        drop(held);
        assert!(recorder.free());
        assert_eq!(recorder.calls(), [("locking", true), ("unlocked", true)]);
    }

    #[test]
    fn a_read_gives_the_file_lock_back_once_it_has_checked_for_another_copy() {
        let dir = tempfile::tempdir().unwrap();
        let (root, store, recorder, _) = guarded_account(dir.path(), DEFAULT_LOCK_WAIT);
        store.meta_set("k", "v").unwrap();
        // Another process (the extension) writes the file: the read loads its copy, the file lock already given back.
        let key = VaultKey::from_bytes(&[7; 64]).unwrap();
        let other = Store::account(Arc::clone(&root), dir.path(), owner("a"), &key).unwrap();
        other.meta_set("k", "from the extension").unwrap();
        drop(recorder.calls());
        let reading = store.read().unwrap();
        assert!(recorder.free(), "a read holds only this process's lock");
        assert_eq!(recorder.calls(), [("locking", true), ("unlocked", true)]);
        drop(reading);
        assert_eq!(store.meta_get("k").unwrap().as_deref(), Some("from the extension"));
        // A write keeps it until what it wrote is in place.
        let writing = store.lock().unwrap();
        assert!(!recorder.free());
        drop(writing);
        assert!(recorder.free());
    }

    #[test]
    fn a_refused_lock_fails_the_call_and_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let (_root, store, recorder, _) = guarded_account(dir.path(), DEFAULT_LOCK_WAIT);
        recorder.refuse.store(true, Ordering::SeqCst);
        assert_eq!(store.meta_set("k", "v").unwrap_err(), CoreError::storage(STORE_SUSPENDING));
        assert_eq!(recorder.calls(), [("locking", true)], "never taken, so never given back");
        recorder.refuse.store(false, Ordering::SeqCst);
        assert_eq!(store.meta_get("k").unwrap(), None);
        store.meta_set("k", "v").unwrap();
        assert_eq!(store.meta_get("k").unwrap().as_deref(), Some("v"), "and the store works on");
    }

    #[test]
    fn a_lock_another_process_keeps_fails_as_busy_after_the_wait() {
        let dir = tempfile::tempdir().unwrap();
        let wait = Duration::from_millis(150);
        let (_root, store, recorder, lock_file) = guarded_account(dir.path(), wait);
        store.meta_set("k", "v").unwrap();
        drop(recorder.calls());
        // Another process (a notification extension suspended mid-write, say) holds the lock and does not let go.
        let other = fs::OpenOptions::new().read(true).write(true).open(&lock_file).unwrap();
        other.lock().unwrap();
        let started = Instant::now();
        assert_eq!(store.meta_get("k").unwrap_err(), CoreError::storage(STORE_BUSY));
        let waited = started.elapsed();
        assert!(waited >= wait && waited < wait * 10, "{waited:?}");
        assert_eq!(recorder.calls(), [("locking", false), ("unlocked", false)], "the guard is let go too");
        other.unlock().unwrap();
        assert_eq!(store.meta_get("k").unwrap().as_deref(), Some("v"), "the same call works once it is free");
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
