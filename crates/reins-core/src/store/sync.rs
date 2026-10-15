//! What account sync keeps on this phone besides the data: the server revision it last agreed on, a hash of that
//! state for a quick "anything new?", and the state itself (the base of a three-way merge), sealed in a file of its
//! own next to the account's. A phone with changes the server never got merges them with what another phone uploaded
//! meanwhile instead of being refused; see `merge.rs` for how, and `account_sync.rs` for the network side.
use super::snapshot::{self, Snapshot, Unreadable, canonical_hash, raw_hash};
use super::{Store, merge};
use crate::CoreError;
use rusqlite::{Connection, OptionalExtension, params};
use zeroize::Zeroizing;

const REVISION: &str = "account-state.revision";
const SYNCED: &str = "account-state.synced";
const NOTICE: &str = "account-state.notice";
/// The side files (see [`Store::write_side`]): the last state both sides agreed on, and this phone's own state as it
/// was before a merge that had no base replaced parts of it.
const BASE: &str = "sync-base";
const BACKUP: &str = "sync-backup";

pub(crate) const NOTICE_REPLACED: &str = "Synced with your other phone. A backup of this phone's changes is kept.";
pub(crate) const NOTICE_NEWER: &str = "Update Reins to sync with your other phone.";
pub(crate) const NOTICE_UNREADABLE: &str = "Couldn't read your other phone's changes.";

/// A state to upload: the revision it follows, and the state.
pub(crate) type Upload = (u64, Zeroizing<Vec<u8>>);

/// What [`Store::pull`] did.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Pulled {
    /// The server has nothing newer.
    Current,
    /// This phone had nothing new: it now has the server's state.
    Imported,
    /// Both had changes: this phone has the merge, which still has to be uploaded.
    Merged,
    /// The server is behind this phone (it lost its copy): this phone's is to be uploaded over it.
    Behind,
    /// The server's state cannot be used by this version: a notice says so and nothing changed.
    Unusable,
}

/// Brings a snapshot written by any version to this version's tables, columns and data rules: read into an empty
/// database of this schema (missing columns take their defaults), with every data rewrite of the migrations applied.
fn normalize(plain: &[u8]) -> Result<Snapshot, Unreadable> {
    let parsed = snapshot::parse(plain)?;
    let mut conn = super::protected::memory()?;
    snapshot::import_snapshot(&mut conn, parsed, true)?;
    conn.execute_batch(super::DATA_FIXUPS)?;
    Ok(snapshot::parse(&snapshot::export(&conn, true)?)?)
}

fn empty_like(snapshot: &Snapshot) -> Snapshot {
    let mut empty = snapshot.clone();
    for table in empty.tables.values_mut() {
        table.rows.clear();
    }
    empty
}

/// Called with a database sealed by an older version (`pre`: its portable export then) after it was migrated: when
/// it was in sync before the migration, it still is. A migration alone is never a change of this phone's.
pub(super) fn rebaseline(conn: &Connection, pre: &[u8]) -> Result<(), CoreError> {
    let key = format!("app.{SYNCED}");
    let synced: Option<String> =
        conn.query_row("SELECT value FROM meta WHERE key = ?1", params![key], |r| r.get(0)).optional()?;
    let Some(synced) = synced else {
        return Ok(());
    };
    let matched = synced == raw_hash(pre) || snapshot::parse(pre).is_ok_and(|s| canonical_hash(&s) == synced);
    if matched {
        let post = snapshot::parse(&snapshot::export(conn, true)?)?;
        conn.execute("UPDATE meta SET value = ?2 WHERE key = ?1", params![key, canonical_hash(&post)])?;
    }
    Ok(())
}

impl Store {
    /// The server revision this phone last agreed with (0: never synced).
    pub(crate) fn sync_revision(&self) -> Result<u64, CoreError> {
        Ok(self.meta_get(REVISION)?.and_then(|s| s.parse::<u64>().ok()).unwrap_or(0))
    }
    /// A sync problem to show in a banner, until dismissed.
    pub(crate) fn sync_notice(&self) -> Result<Option<String>, CoreError> {
        self.meta_get(NOTICE)
    }
    /// The server's state does not open with this account's key.
    pub(crate) fn note_sync_unreadable(&self) -> Result<(), CoreError> {
        self.meta_set(NOTICE, NOTICE_UNREADABLE)
    }
    pub(crate) fn clear_sync_notice(&self) -> Result<(), CoreError> {
        self.lock()?.execute("DELETE FROM meta WHERE key = ?1", params![format!("app.{NOTICE}")])?;
        Ok(())
    }
    /// The state to upload and the revision it follows, or `None` when the server already has it.
    pub(crate) fn pending_upload(&self) -> Result<Option<Upload>, CoreError> {
        let plain = self.export_account()?;
        if self.synced_matches(&plain, &snapshot::parse(&plain)?)? {
            return Ok(None);
        }
        Ok(Some((self.sync_revision()?, plain)))
    }
    /// The server took `plain` as `revision`.
    pub(crate) fn mark_uploaded(&self, revision: u64, plain: &[u8]) -> Result<(), CoreError> {
        self.agree(revision, plain, &snapshot::parse(plain)?)
    }
    /// The last synced state (current versions keep a canonical hash, older ones a hash of the bytes they wrote) is
    /// what this phone has now.
    fn synced_matches(&self, plain: &[u8], parsed: &Snapshot) -> Result<bool, CoreError> {
        Ok(self.meta_get(SYNCED)?.is_some_and(|h| h == canonical_hash(parsed) || h == raw_hash(plain)))
    }
    /// This phone and the server agree on `state` as `revision`: remembered as the base of the next merge.
    fn agree(&self, revision: u64, plain: &[u8], state: &Snapshot) -> Result<(), CoreError> {
        let mut base = Zeroizing::new(revision.to_be_bytes().to_vec());
        base.extend_from_slice(plain);
        // Without its base the next merge falls back to the server's version: worse, never wrong.
        if let Err(e) = self.write_side(BASE, &base) {
            log::warn!("could not keep the synced account state as a merge base: {e}");
        }
        self.meta_set(SYNCED, &canonical_hash(state))?;
        self.meta_set(REVISION, &revision.to_string())
    }
    /// The base this phone kept for `revision`, if it has one that opens.
    fn sync_base(&self, revision: u64) -> Option<Snapshot> {
        let plain = match self.read_side(BASE) {
            Ok(Some(plain)) => plain,
            Ok(None) => return None,
            Err(e) => {
                log::warn!("the account sync base cannot be read ({e}); merging without it");
                return None;
            }
        };
        let (kept, state) = plain.split_at_checked(8)?;
        if u64::from_be_bytes(kept.try_into().ok()?) != revision {
            return None;
        }
        normalize(state).ok()
    }

    /// Takes the server's `revision` (`plain`, already opened) into this phone's data. With nothing changed here
    /// since the last sync it replaces the data; with changes, the two are merged (see `merge.rs`) and the caller
    /// uploads the result. Never refuses: a state this version cannot read leaves a notice and changes nothing.
    pub(crate) fn pull(&self, revision: u64, plain: Option<&[u8]>) -> Result<Pulled, CoreError> {
        let local_revision = self.sync_revision()?;
        if revision < local_revision {
            self.meta_set(REVISION, &revision.to_string())?;
            self.lock()?.execute("DELETE FROM meta WHERE key = ?1", params![format!("app.{SYNCED}")])?;
            return Ok(Pulled::Behind);
        }
        let Some(plain) = plain.filter(|_| revision > local_revision) else {
            return Ok(Pulled::Current);
        };
        let server = match normalize(plain) {
            Ok(server) => server,
            Err(Unreadable::Newer) => {
                self.meta_set(NOTICE, NOTICE_NEWER)?;
                return Ok(Pulled::Unusable);
            }
            Err(Unreadable::Invalid(e)) => {
                log::warn!("the server's account state cannot be read: {e}");
                self.meta_set(NOTICE, NOTICE_UNREADABLE)?;
                return Ok(Pulled::Unusable);
            }
        };
        let server_plain = snapshot::encode(&server)?;
        // Never synced: everything here is this phone's own, to add to the server's.
        let base = if local_revision == 0 {
            Some(empty_like(&server))
        } else {
            self.sync_base(local_revision)
        };
        let synced = self.meta_get(SYNCED)?;
        // Read, merged and replaced under one lock: a write meanwhile (an approval) is neither lost nor half merged.
        let (pulled, replaced) = {
            let mut conn = self.lock()?;
            let local_plain = snapshot::export(&conn, true)?;
            let local = snapshot::parse(&local_plain)?;
            let changed = match &base {
                _ if local_revision == 0 => snapshot::has_rows(&local),
                Some(base) => canonical_hash(&local) != canonical_hash(base),
                None => !synced.is_some_and(|h| h == canonical_hash(&local) || h == raw_hash(&local_plain)),
            };
            if changed {
                let merged = merge::merge(base.as_ref(), &local, &server);
                if base.is_none() {
                    // Nothing tells what the server's version replaced on purpose: keep this phone's as it was.
                    self.write_side(BACKUP, &local_plain)?;
                }
                snapshot::import_snapshot(&mut conn, merged.snapshot, true)?;
                (Pulled::Merged, base.is_none() && merged.dropped_local)
            } else {
                snapshot::import_snapshot(&mut conn, server.clone(), true)?;
                (Pulled::Imported, false)
            }
        };
        if replaced {
            self.meta_set(NOTICE, NOTICE_REPLACED)?;
        }
        self.agree(revision, &server_plain, &server)?;
        Ok(pulled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::VaultKey;
    use crate::store::Owner;
    use crate::store::tests::FakeKeys;
    use std::path::Path;
    use std::sync::Arc;

    fn owner() -> Owner {
        Owner {
            server: "https://one.example".to_owned(),
            user_id: "alice".to_owned(),
        }
    }
    /// One phone: its device store and its account store, in `dir`.
    struct Phone {
        root: Arc<Store>,
        dir: std::path::PathBuf,
    }
    impl Phone {
        fn new(dir: &Path) -> Self {
            let root = Arc::new(Store::open(dir, &FakeKeys::default()).unwrap());
            root.meta_set("active-account", &owner().id()).unwrap();
            Self {
                root,
                dir: dir.join("accounts"),
            }
        }
        /// The account store as the app opens it (a restart opens it again).
        fn open(&self) -> Store {
            Store::account(Arc::clone(&self.root), &self.dir, owner(), &VaultKey::from_bytes(&[7; 64]).unwrap())
                .unwrap()
        }
    }
    /// What the server holds: a revision and the opened state.
    #[derive(Default)]
    struct Server {
        revision: u64,
        state: Option<Zeroizing<Vec<u8>>>,
    }
    impl Server {
        /// `store` uploads what it has; false when the server is ahead of it (a 409).
        fn upload(&mut self, store: &Store) -> bool {
            let Some((revision, plain)) = store.pending_upload().unwrap() else {
                return true;
            };
            if revision != self.revision {
                return false;
            }
            self.revision += 1;
            store.mark_uploaded(self.revision, &plain).unwrap();
            self.state = Some(plain);
            true
        }
        fn pull(&self, store: &Store) -> Pulled {
            store.pull(self.revision, self.state.as_deref().map(|s| &s[..])).unwrap()
        }
        /// Pull, merge and upload until the server takes it, as `account_sync.rs` does on a conflict.
        fn sync(&mut self, store: &Store) {
            for _ in 0..3 {
                if self.upload(store) {
                    return;
                }
                self.pull(store);
            }
            panic!("never synced");
        }
    }
    fn names(store: &Store) -> Vec<String> {
        let mut names: Vec<String> = store.accounts().unwrap().into_iter().map(|a| a.account).collect();
        names.sort();
        names
    }
    fn desktop_topic_grant(store: &Store) {
        let json = serde_json::json!({"id": "d1", "connection_id": "c1", "created_at": 10, "expires_at": null,
            "max_uses": null, "uses": 0, "revoked": false, "account": "desktop app",
            "scope": {"action": "service", "service": "desktop", "access": "write", "resources": ["file:.env"]}})
        .to_string();
        store
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO grants (id, connection_id, connection_label, created_at, grant_json) VALUES ('d1', 'c1', 'Laptop', 10, ?1)",
                params![json],
            )
            .unwrap();
    }
    /// Makes the sealed file what a version from before migration 11 wrote, synced at `revision` with the hash that
    /// version kept (of the export's bytes; no base file), with a topic-keyed desktop grant migration 11 drops.
    fn synced_by_an_older_version(store: &Store, revision: u64) {
        desktop_topic_grant(store);
        store.lock().unwrap().execute_batch("PRAGMA user_version = 10;").unwrap();
        let plain = store.export_account().unwrap();
        // Each write seals the database as it now is, schema version included.
        store.meta_set(REVISION, &revision.to_string()).unwrap();
        store.meta_set(SYNCED, &raw_hash(&plain)).unwrap();
    }

    /// The user's state: synced at N by an older version, the app updated (migration 11 drops a grant as it opens),
    /// and the other phone pushed N+1. Before the fix the pull refused with "This account has unsynced changes".
    #[test]
    fn an_update_alone_never_counts_as_unsynced_changes() {
        let dir = tempfile::tempdir().unwrap();
        let phone = Phone::new(dir.path());
        let store = phone.open();
        store.add_account("github", "kept", 1).unwrap();
        // The other phone's state at N+1: what this one had, plus its own change.
        let other = tempfile::tempdir().unwrap();
        let other = Phone::new(other.path()).open();
        other.add_account("github", "kept", 1).unwrap();
        other.add_account("github", "from-the-other-phone", 2).unwrap();
        synced_by_an_older_version(&store, 4);
        drop(store);
        let updated = phone.open();
        assert!(updated.grants().unwrap().is_empty(), "migration 11 ran");
        assert_eq!(updated.pending_upload().unwrap(), None, "the migration is no change of this phone's");
        let pulled = updated.pull(5, Some(&other.export_account().unwrap())).unwrap();
        assert_eq!(pulled, Pulled::Imported);
        assert_eq!(names(&updated), ["from-the-other-phone", "kept"]);
        assert_eq!(updated.sync_notice().unwrap(), None);
        assert_eq!(updated.read_side(BACKUP).unwrap(), None, "nothing of this phone's was set aside");
    }

    /// The same phone with a change of its own the server never got, synced by the older version (no base): the
    /// server's state wins, a grant only tightens, this phone's state is kept in a sealed backup and a notice says so.
    #[test]
    fn without_a_base_the_server_wins_and_this_phones_state_is_backed_up() {
        let dir = tempfile::tempdir().unwrap();
        let phone = Phone::new(dir.path());
        let store = phone.open();
        synced_by_an_older_version(&store, 4);
        store.add_account("github", "only-here", 3).unwrap();
        let other = tempfile::tempdir().unwrap();
        let other = Phone::new(other.path()).open();
        other.add_account("github", "from-the-other-phone", 2).unwrap();
        let pulled = store.pull(5, Some(&other.export_account().unwrap())).unwrap();
        assert_eq!(pulled, Pulled::Merged);
        assert_eq!(names(&store), ["from-the-other-phone"]);
        assert_eq!(store.sync_notice().unwrap().as_deref(), Some(NOTICE_REPLACED));
        let backup = store.read_side(BACKUP).unwrap().expect("a sealed backup");
        assert!(String::from_utf8_lossy(&backup).contains("only-here"));
        let sealed = std::fs::read(phone.dir.join(format!("{}.sync-backup.sealed", owner().id()))).unwrap();
        assert!(!sealed.windows(9).any(|w| w == b"only-here"), "the backup is sealed");
        assert_eq!(store.pending_upload().unwrap(), None, "it has the server's state now: nothing to upload");
    }

    /// A corrupt base is as good as none: never a refusal.
    #[test]
    fn a_corrupt_base_falls_back_to_the_servers_state_with_a_backup() {
        let dir = tempfile::tempdir().unwrap();
        let phone = Phone::new(dir.path());
        let store = phone.open();
        let mut server = Server::default();
        store.add_account("github", "first", 1).unwrap();
        server.sync(&store);
        std::fs::write(phone.dir.join(format!("{}.sync-base.sealed", owner().id())), b"not sealed").unwrap();
        store.add_account("github", "only-here", 2).unwrap();
        let other_dir = tempfile::tempdir().unwrap();
        let other = Phone::new(other_dir.path()).open();
        assert_eq!(server.pull(&other), Pulled::Imported);
        other.add_account("github", "there", 3).unwrap();
        server.sync(&other);
        assert_eq!(server.pull(&store), Pulled::Merged);
        assert_eq!(names(&store), ["first", "there"]);
        assert!(store.read_side(BACKUP).unwrap().is_some());
        assert_eq!(store.sync_notice().unwrap().as_deref(), Some(NOTICE_REPLACED));
    }

    /// Two phones change different things after the same revision: each keeps both, and so does the server.
    #[test]
    fn concurrent_changes_on_two_phones_merge() {
        let (a_dir, b_dir) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let (a, b) = (Phone::new(a_dir.path()).open(), Phone::new(b_dir.path()).open());
        let mut server = Server::default();
        a.add_account("github", "first", 1).unwrap();
        server.sync(&a);
        assert_eq!(server.pull(&b), Pulled::Imported);
        a.add_account("github", "from-a", 2).unwrap();
        b.add_account("github", "from-b", 3).unwrap();
        server.sync(&a);
        server.sync(&b); // its upload is refused once (409): it pulls, merges and uploads again
        assert_eq!(names(&b), ["first", "from-a", "from-b"]);
        assert_eq!(server.pull(&a), Pulled::Imported);
        assert_eq!(names(&a), ["first", "from-a", "from-b"]);
        assert_eq!(b.sync_notice().unwrap(), None, "a merge with a base needs no notice");
        assert_eq!(b.read_side(BACKUP).unwrap(), None);
    }

    /// A grant revoked on one phone while the other extended it ends revoked on both; one deleted on one phone while
    /// the other used it ends deleted.
    #[test]
    fn a_merge_never_resurrects_or_widens_a_grant() {
        let (a_dir, b_dir) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let (a, b) = (Phone::new(a_dir.path()).open(), Phone::new(b_dir.path()).open());
        let mut server = Server::default();
        let g = |id: &str| crate::store::grants::tests::read_grant(id, "c1", Some(5));
        a.insert_grant(&g("revoked-vs-extended"), "Claude").unwrap();
        a.insert_grant(&g("deleted-vs-used"), "Claude").unwrap();
        server.sync(&a);
        server.pull(&b);
        let id = |s: &str| reins_proto::ids::GrantId(s.to_owned());
        // A revokes one and deletes the other (after revoking it: only an ended grant can be deleted).
        assert!(a.revoke_grant(&id("revoked-vs-extended")).unwrap());
        assert!(a.revoke_grant(&id("deleted-vs-used")).unwrap());
        assert!(a.delete_ended_grant(&id("deleted-vs-used"), 100).unwrap());
        // B, meanwhile, extends the first for a year, and uses the second.
        assert!(b.extend_grant(&id("revoked-vs-extended"), 365 * 86_400, 100).unwrap());
        b.lock()
            .unwrap()
            .execute(
                "UPDATE grants SET grant_json = json_set(grant_json, '$.uses', 2) WHERE id = 'deleted-vs-used'",
                [],
            )
            .unwrap();
        server.sync(&a);
        server.sync(&b);
        server.pull(&a);
        for store in [&a, &b] {
            let grants = store.grants().unwrap();
            assert_eq!(grants.len(), 1, "the deleted grant stays deleted");
            assert!(grants[0].grant.revoked, "the revoked grant is not resurrected by the other phone's extension");
        }
    }

    /// The server lost its copy (or answers with an older one): this phone's is uploaded over it, never a refusal.
    #[test]
    fn a_server_behind_this_phone_gets_this_phones_state() {
        let dir = tempfile::tempdir().unwrap();
        let store = Phone::new(dir.path()).open();
        let mut server = Server::default();
        store.add_account("github", "kept", 1).unwrap();
        server.sync(&store);
        server.sync(&store);
        let mut wiped = Server::default();
        assert_eq!(wiped.pull(&store), Pulled::Behind);
        wiped.sync(&store);
        assert_eq!(wiped.revision, 1);
        assert_eq!(names(&store), ["kept"]);
    }

    /// A state from a newer version (a table this one does not know) is left alone, with a notice; nothing changes.
    #[test]
    fn a_state_from_a_newer_version_leaves_a_notice_and_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let store = Phone::new(dir.path()).open();
        store.add_account("github", "kept", 1).unwrap();
        let mut newer: serde_json::Value = serde_json::from_slice(&store.export_account().unwrap()).unwrap();
        newer["tables"]["from_the_future"] = serde_json::json!({"columns": ["x"], "rows": []});
        let pulled = store.pull(3, Some(&serde_json::to_vec(&newer).unwrap())).unwrap();
        assert_eq!(pulled, Pulled::Unusable);
        assert_eq!(store.sync_notice().unwrap().as_deref(), Some(NOTICE_NEWER));
        assert_eq!(names(&store), ["kept"]);
        assert_eq!(store.sync_revision().unwrap(), 0);
    }

    /// An entry's kept approval follows it when a merge renumbers this phone's activity.
    #[test]
    fn an_entrys_approval_follows_it_through_a_merge() {
        let (a_dir, b_dir) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let (a, b) = (Phone::new(a_dir.path()).open(), Phone::new(b_dir.path()).open());
        let mut server = Server::default();
        server.sync(&a);
        server.pull(&b);
        let entry = |detail: &str| crate::store::AuditRecord {
            seq: 0,
            at: 10,
            connection_id: "c1".to_owned(),
            connection_label: "Claude".to_owned(),
            action: "search".to_owned(),
            outcome: "released".to_owned(),
            detail: detail.to_owned(),
            grant_id: None,
            service: "gmail".to_owned(),
            account: None,
            count: 1,
            info: crate::store::AuditInfo::default(),
            op: String::new(),
        };
        a.append_audit(&entry("on a")).unwrap();
        server.sync(&a);
        let seq = b.append_audit(&entry("on b")).unwrap();
        b.lock().unwrap().execute("INSERT INTO audit_snapshots (seq, data) VALUES (?1, x'01')", [seq]).unwrap();
        server.sync(&b);
        let activity = b.activity(10).unwrap();
        assert_eq!(activity.len(), 2);
        let mine = activity.iter().find(|e| e.detail == "on b").unwrap();
        assert_ne!(mine.seq, seq, "renumbered after the other phone's entry");
        let kept: Vec<i64> = {
            let conn = b.lock().unwrap();
            let mut stmt = conn.prepare("SELECT seq FROM audit_snapshots").unwrap();
            stmt.query_map([], |r| r.get(0)).unwrap().collect::<Result<_, _>>().unwrap()
        };
        assert_eq!(kept, [mine.seq]);
    }
}
