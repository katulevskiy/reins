//! What account sync keeps on this phone besides the data: the server revision it last agreed on (and the highest
//! authentic one it has seen), a hash of that state for a quick "anything new?", and the state itself (the base of a
//! three-way merge), sealed with this installation's device key and bound to its revision and account. A phone with
//! changes the server never got merges them with what another phone uploaded meanwhile instead of being refused; see
//! `merge.rs` for how, and `account_sync.rs` for the network side.
//!
//! Revisions only move forward: a server state older than one this phone agreed on or saw is refused (with a
//! notice), never merged and never uploaded over; the base can only be replaced by a newer revision's.
use super::snapshot::{self, Snapshot, Unreadable, canonical_hash, raw_hash};
use super::{Store, merge};
use crate::CoreError;
use rusqlite::{Connection, OptionalExtension, params};
use zeroize::Zeroizing;

const REVISION: &str = "account-state.revision";
const SEEN: &str = "account-state.seen";
const SYNCED: &str = "account-state.synced";
/// When this phone last agreed with the server (its own clock): a base older than the tombstone horizon is not used.
const AGREED_AT: &str = "account-state.agreed-at";
const NOTICE: &str = "account-state.notice";
/// The side files (see [`Store::write_side`]): the last state both sides agreed on, and this phone's own state as it
/// was before a merge without a base replaced parts of it. The backup is never uploaded; only
/// [`Store::restore_sync_backup`], which the user asks for, brings it back, through the merge rules.
const BASE: &str = "sync-base";
const BACKUP: &str = "sync-backup";

pub(crate) const NOTICE_REPLACED: &str = "Synced with your other phone. A backup of this phone's changes is kept.";
pub(crate) const NOTICE_NEWER: &str = "Update Reins to sync with your other phone.";
pub(crate) const NOTICE_UNREADABLE: &str = "Couldn't read your other phone's changes.";
pub(crate) const NOTICE_OLDER: &str = "The server sent an older copy of this account. It was ignored.";

/// A state to upload: the revision it follows, and the state.
pub(crate) type Upload = (u64, Zeroizing<Vec<u8>>);

/// What [`Store::pull`] did.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Pulled {
    /// The server has nothing newer.
    Current,
    /// The merge is the server's state: nothing to upload (what of this phone's the rules dropped is in the backup).
    Imported,
    /// The merge has something the server does not: it still has to be uploaded.
    Merged,
    /// The server's state cannot be used by this version: a notice says so and nothing changed.
    Unusable,
}

/// Brings a snapshot written by any version to this version's tables, columns and data rules: read into an empty
/// database of this schema (missing columns take their defaults), with every data rewrite of the migrations applied.
/// Whatever it is (the server's, a base, a backup), what it carries that has a tombstone is dropped.
fn normalize(plain: &[u8]) -> Result<Snapshot, Unreadable> {
    let parsed = snapshot::parse(plain)?;
    let mut conn = super::protected::memory()?;
    snapshot::import_snapshot(&mut conn, parsed, true)?;
    // Rewrites, not removals by anyone: no tombstones (nor ones stamped with today's date in every copy).
    super::drop_tombstone_triggers(&conn)?;
    conn.execute_batch(super::DATA_FIXUPS)?;
    let mut normalized = snapshot::parse(&snapshot::export(&conn, true)?)?;
    merge::bury(&mut normalized);
    Ok(normalized)
}

fn empty_like(snapshot: &Snapshot) -> Snapshot {
    let mut empty = snapshot.clone();
    for table in empty.tables.values_mut() {
        table.rows.clear();
    }
    empty
}

fn base_context(revision: u64) -> String {
    format!("revision:{revision}")
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
    /// The lowest revision this phone still accepts: none below the one it agreed on, or below an authentic one it
    /// saw (and could not use).
    pub(crate) fn sync_floor(&self) -> Result<u64, CoreError> {
        let seen = self.meta_get(SEEN)?.and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
        Ok(self.sync_revision()?.max(seen))
    }
    fn saw(&self, revision: u64) -> Result<(), CoreError> {
        if revision > self.sync_floor()? {
            self.meta_set(SEEN, &revision.to_string())?;
        }
        Ok(())
    }
    /// A sync problem to show in a banner, until dismissed.
    pub(crate) fn sync_notice(&self) -> Result<Option<String>, CoreError> {
        self.meta_get(NOTICE)
    }
    /// The server's state does not open with this account's key (or not for the revision it claims).
    pub(crate) fn note_sync_unreadable(&self) -> Result<(), CoreError> {
        self.meta_set(NOTICE, NOTICE_UNREADABLE)
    }
    pub(crate) fn clear_sync_notice(&self) -> Result<(), CoreError> {
        self.lock()?.execute("DELETE FROM meta WHERE key = ?1", params![format!("app.{NOTICE}")])?;
        Ok(())
    }
    /// The state to upload and the revision it follows, or `None` when the server already has it (or this phone saw a
    /// newer revision it could not take in, which it never uploads over).
    pub(crate) fn pending_upload(&self) -> Result<Option<Upload>, CoreError> {
        let plain = self.export_account()?;
        let revision = self.sync_revision()?;
        if self.synced_matches(&plain, &snapshot::parse(&plain)?)? || self.sync_floor()? > revision {
            return Ok(None);
        }
        Ok(Some((revision, plain)))
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
    /// This phone and the server agree on `state` as `revision`, which must be newer than any it agreed on: the base
    /// of the next merge, sealed for that revision only.
    fn agree(&self, revision: u64, plain: &[u8], state: &Snapshot) -> Result<(), CoreError> {
        if revision <= self.sync_revision()? {
            return Err(CoreError::storage("refusing an older account state"));
        }
        // Without its base the next merge falls back to the server's version: worse, never wrong.
        if let Err(e) = self.write_side(BASE, &base_context(revision), plain) {
            log::warn!("could not keep the synced account state as a merge base: {e}");
        }
        self.meta_set(SYNCED, &canonical_hash(state))?;
        self.meta_set(AGREED_AT, &super::unix_now().to_string())?;
        self.meta_set(REVISION, &revision.to_string())?;
        self.saw(revision)
    }
    /// The base this phone kept for `revision`, if it has one that opens for exactly that revision and is younger than
    /// the tombstone horizon. An older one may predate removals whose tombstones are gone: without it, what only this
    /// phone has is dropped (the server's state wins), so nothing removed elsewhere comes back.
    fn sync_base(&self, revision: u64) -> Option<Snapshot> {
        let agreed = self.meta_get(AGREED_AT).ok().flatten().and_then(|s| s.parse::<i64>().ok())?;
        if agreed < super::unix_now().saturating_sub(super::TOMBSTONE_HORIZON_SECS) {
            log::warn!("the account sync base is older than the tombstone horizon; merging without it");
            return None;
        }
        match self.read_side(BASE, &base_context(revision)) {
            Ok(Some(plain)) => normalize(&plain).ok(),
            Ok(None) => None,
            Err(e) => {
                log::warn!("the account sync base does not open for revision {revision} ({e}); merging without it");
                None
            }
        }
    }

    /// Takes the server's `revision` (`plain`, already opened with the account key for that revision) into this
    /// phone's data. With nothing changed here since the last sync it replaces the data; with changes, the two are
    /// merged (see `merge.rs`) and the caller uploads the result. A revision below [`Self::sync_floor`] is refused with
    /// a notice; a state this version cannot read leaves a notice and changes nothing.
    pub(crate) fn pull(&self, revision: u64, plain: Option<&[u8]>) -> Result<Pulled, CoreError> {
        let local_revision = self.sync_revision()?;
        if revision < self.sync_floor()? {
            self.meta_set(NOTICE, NOTICE_OLDER)?;
            return Err(CoreError::storage("the server returned an older account state"));
        }
        let Some(plain) = plain.filter(|_| revision > local_revision) else {
            return Ok(Pulled::Current);
        };
        self.saw(revision)?;
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
            // Always the merge, never the server's state as it is: even with nothing changed here, a grant revoked here
            // stays revoked, and the tombstones and every other rule apply (the merge of an unchanged side is the
            // other side, but for those).
            let merged = merge::merge(base.as_ref(), &local, &server, super::unix_now());
            if changed && base.is_none() {
                // Nothing tells what the server's version replaced on purpose: keep this phone's as it was, inert.
                self.write_side(BACKUP, BACKUP, &local_plain)?;
            }
            let pulled = if canonical_hash(&merged.snapshot) == canonical_hash(&server) {
                Pulled::Imported
            } else {
                Pulled::Merged
            };
            snapshot::import_snapshot(&mut conn, merged.snapshot, true)?;
            (pulled, changed && base.is_none() && merged.dropped_local)
        };
        if replaced {
            self.meta_set(NOTICE, NOTICE_REPLACED)?;
        }
        self.agree(revision, &server_plain, &server)?;
        Ok(pulled)
    }

    /// The user asked for this phone's backup (kept by a merge without a base) back: it is merged into the data as it
    /// is now, as another phone's would be. Records only the backup has come back, except security records (grants,
    /// accounts and tokens, paired computers' keys, MCP servers, approved targets, Autopilot modes): one of those only
    /// the backup has may be one removed since. The rest merges by the usual rules and tombstones apply, so nothing
    /// becomes more permissive. False when there is no backup. What comes back is uploaded like any change.
    pub(crate) fn restore_sync_backup(&self) -> Result<bool, CoreError> {
        let Some(plain) = self.read_side(BACKUP, BACKUP)? else {
            return Ok(false);
        };
        let backup = normalize(&plain).map_err(CoreError::from)?;
        let mut conn = self.lock()?;
        let current = snapshot::parse(&snapshot::export(&conn, true)?)?;
        let mut merged = merge::merge(Some(&empty_like(&current)), &backup, &current, super::unix_now()).snapshot;
        let guarded = super::GUARDED.iter().map(|(t, k)| (*t, *k)).chain([("autopilot_settings", &["scope"][..])]);
        for (table, key) in guarded {
            let (Some(now), Some(out)) = (current.tables.get(table), merged.tables.get_mut(table)) else {
                continue;
            };
            let Some(at) = key.iter().map(|k| now.column(k)).collect::<Option<Vec<_>>>() else {
                continue;
            };
            let key_of = |r: &Vec<snapshot::Cell>| at.iter().map(|i| format!("{:?}", r.get(*i))).collect::<Vec<_>>();
            let present: std::collections::BTreeSet<_> = now.rows.iter().map(key_of).collect();
            out.rows.retain(|r| present.contains(&key_of(r)));
        }
        snapshot::import_snapshot(&mut conn, merged, true)?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::VaultKey;
    use crate::store::Owner;
    use crate::store::tests::FakeKeys;
    use std::collections::BTreeMap;
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
        assert_eq!(updated.read_side(BACKUP, BACKUP).unwrap(), None, "nothing of this phone's was set aside");
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
        assert_eq!(pulled, Pulled::Imported, "the server's state, nothing of this phone's left to upload");
        assert_eq!(names(&store), ["from-the-other-phone"]);
        assert_eq!(store.sync_notice().unwrap().as_deref(), Some(NOTICE_REPLACED));
        let backup = store.read_side(BACKUP, BACKUP).unwrap().expect("a sealed backup");
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
        assert_eq!(server.pull(&store), Pulled::Imported);
        assert_eq!(names(&store), ["first", "there"]);
        assert!(store.read_side(BACKUP, BACKUP).unwrap().is_some());
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
        assert_eq!(b.read_side(BACKUP, BACKUP).unwrap(), None);
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

    /// The server answers with an older revision (it lost its copy, or replays an old one): refused with a notice,
    /// never merged and never uploaded over. Revisions this phone saw count too.
    #[test]
    fn an_older_or_replayed_state_is_refused_and_never_uploaded_over() {
        let dir = tempfile::tempdir().unwrap();
        let store = Phone::new(dir.path()).open();
        let mut server = Server::default();
        store.add_account("github", "kept", 1).unwrap();
        server.sync(&store);
        let old = server.state.clone();
        store.add_account("github", "second", 2).unwrap();
        server.sync(&store);
        assert_eq!(server.revision, 2);
        // Revision 1 replayed, and a wiped server (revision 0).
        for (revision, state) in [(1, old.clone()), (0, None)] {
            let err = store.pull(revision, state.as_deref().map(Vec::as_slice)).unwrap_err();
            assert!(err.to_string().contains("older account state"), "{err}");
            assert_eq!(store.sync_notice().unwrap().as_deref(), Some(NOTICE_OLDER));
            assert_eq!(store.sync_revision().unwrap(), 2);
            assert_eq!(names(&store), ["kept", "second"]);
        }
        // The same revision again changes nothing.
        assert_eq!(store.pull(2, old.as_deref().map(Vec::as_slice)).unwrap(), Pulled::Current);
        assert_eq!(names(&store), ["kept", "second"]);
        // An authentic newer revision this phone could not use (a newer version's) raises the floor: what follows
        // below it is refused, and nothing is uploaded over it.
        let mut newer: serde_json::Value = serde_json::from_slice(&store.export_account().unwrap()).unwrap();
        newer["tables"]["from_the_future"] = serde_json::json!({"columns": ["x"], "rows": []});
        assert_eq!(store.pull(9, Some(&serde_json::to_vec(&newer).unwrap())).unwrap(), Pulled::Unusable);
        assert_eq!(store.sync_floor().unwrap(), 9);
        assert!(store.pull(5, server.state.as_deref().map(Vec::as_slice)).is_err());
        store.add_account("github", "third", 3).unwrap();
        assert_eq!(store.pending_upload().unwrap(), None, "never uploaded over a newer revision");
    }

    /// The base opens only for the revision it was sealed for, only for this account, and only with this
    /// installation's device key: an older base put back, or one copied to another phone, is no base.
    #[test]
    fn the_base_is_bound_to_its_revision_account_and_device() {
        let (dir, other_dir) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let phone = Phone::new(dir.path());
        let store = phone.open();
        let mut server = Server::default();
        store.add_account("github", "first", 1).unwrap();
        server.sync(&store);
        let path = phone.dir.join(format!("{}.sync-base.sealed", owner().id()));
        let old_base = std::fs::read(&path).unwrap();
        assert!(store.sync_base(1).is_some());
        assert!(store.sync_base(2).is_none(), "not for another revision");
        store.add_account("github", "second", 2).unwrap();
        server.sync(&store);
        assert!(store.sync_base(2).is_some());
        std::fs::write(&path, &old_base).unwrap();
        assert!(store.sync_base(2).is_none(), "an older base put back does not open");
        assert!(!old_base.windows(5).any(|w| w == b"first"), "sealed");
        // The same account on another phone (same account key, another device key) cannot open it.
        let other = Phone::new(other_dir.path());
        let other_store = other.open();
        std::fs::write(other.dir.join(format!("{}.sync-base.sealed", owner().id())), &old_base).unwrap();
        assert!(other_store.sync_base(1).is_none());
        // And the base is never replaced by an older revision's.
        let plain = store.export_account().unwrap();
        assert!(store.mark_uploaded(1, &plain).is_err());
        assert!(store.sync_base(2).is_none(), "still the one put back, not one for revision 1");
    }

    /// The backup a merge without a base kept is never uploaded; restoring it (the user's choice) merges it in by
    /// the usual rules: what only it has comes back, but never a security record (a grant, an account or its token, a
    /// paired computer's key), which may be one removed since.
    #[test]
    fn the_backup_is_inert_until_restored_and_restoring_never_revives_a_security_record() {
        let dir = tempfile::tempdir().unwrap();
        let phone = Phone::new(dir.path());
        let store = phone.open();
        let g = crate::store::grants::tests::read_grant("revived", "c1", Some(5));
        store.insert_grant(&g, "Claude").unwrap();
        synced_by_an_older_version(&store, 4);
        store.add_account("github", "only-here", 3).unwrap();
        store.secret_put("github", "only-here", b"token-only-here").unwrap();
        store.pin_desktop_key("c9", "key-only-here", "Laptop", 1).unwrap();
        store.set_connection_icon("c1", Some("icon-only-here")).unwrap();
        let other = tempfile::tempdir().unwrap();
        let other = Phone::new(other.path()).open();
        other.add_account("github", "from-the-other-phone", 2).unwrap();
        assert_eq!(store.pull(5, Some(&other.export_account().unwrap())).unwrap(), Pulled::Imported);
        assert!(store.grants().unwrap().is_empty(), "the server's state won");
        let upload = store.pending_upload().unwrap();
        assert!(
            upload.is_none_or(|(_, plain)| !String::from_utf8_lossy(&plain).contains("icon-only-here")),
            "the backup is never uploaded on its own"
        );
        assert!(store.restore_sync_backup().unwrap());
        assert_eq!(store.connection_icon("c1").unwrap().as_deref(), Some("icon-only-here"), "restored");
        assert_eq!(names(&store), ["from-the-other-phone"], "an account only the backup has is not brought back");
        assert_eq!(store.secret_get("github", "only-here").unwrap(), None, "nor its token");
        assert_eq!(store.desktop_key("c9").unwrap(), None, "nor a computer's key");
        assert!(store.grants().unwrap().is_empty(), "nor a grant");
        let (_, plain) = store.pending_upload().unwrap().expect("what came back is uploaded like a change");
        assert!(String::from_utf8_lossy(&plain).contains("icon-only-here"));
    }

    /// One phone removes things (a grant, an account and its token, a paired computer, an MCP server) while the other
    /// is offline for 30 days with its old copy of them, which it even uses. Back online, nothing removed comes back:
    /// not through the merge, nor the upload that follows, nor the other phone's next pull.
    #[test]
    fn a_phone_offline_for_30_days_comes_back_with_an_old_grant_and_it_stays_removed() {
        let (a_dir, b_dir) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let (a, b) = (Phone::new(a_dir.path()).open(), Phone::new(b_dir.path()).open());
        let mut server = Server::default();
        let id = |s: &str| reins_proto::ids::GrantId(s.to_owned());
        a.insert_grant(&crate::store::grants::tests::read_grant("old", "c1", Some(50)), "Claude").unwrap();
        a.add_account("github", "work", 1).unwrap();
        a.secret_put("github", "work", b"work-token").unwrap();
        a.pin_desktop_key("c1", "laptop-key", "Laptop", 1).unwrap();
        a.lock()
            .unwrap()
            .execute("INSERT INTO mcp_servers (id, name, url, auth, added_at) VALUES ('m1', 'Old', 'https://m.example', 'none', 1)", [])
            .unwrap();
        server.sync(&a);
        server.pull(&b);
        // B goes offline (its base: this revision). A removes everything, and keeps syncing for a month.
        assert!(a.revoke_grant(&id("old")).unwrap());
        assert!(a.delete_ended_grant(&id("old"), 100).unwrap());
        a.remove_account("github", "work").unwrap();
        a.secret_delete("github", "work").unwrap();
        a.lock().unwrap().execute_batch("DELETE FROM desktop_keys; DELETE FROM mcp_servers;").unwrap();
        server.sync(&a);
        for day in 0..30 {
            a.add_account("github", &format!("day-{day}"), day).unwrap();
            server.sync(&a);
        }
        // B, offline, keeps using its copy of the grant and the token.
        b.lock()
            .unwrap()
            .execute("UPDATE grants SET grant_json = json_set(grant_json, '$.uses', 7), last_used_at = 99", [])
            .unwrap();
        b.secret_put("github", "work", b"refreshed-work-token").unwrap();
        server.sync(&b);
        server.pull(&a);
        for store in [&a, &b] {
            assert!(store.grants().unwrap().is_empty(), "the grant stays removed");
            assert!(!names(store).contains(&"work".to_owned()), "the account stays removed");
            assert_eq!(store.secret_get("github", "work").unwrap(), None, "and its token");
            assert_eq!(store.desktop_key("c1").unwrap(), None, "and the computer's key");
            assert!(store.mcp_servers().unwrap().is_empty(), "and the MCP server");
            assert_eq!(names(store).len(), 30);
        }
    }

    /// Past the tombstone horizon a phone cannot know what was removed while it was away: what only it has is dropped,
    /// unless the server has it too.
    #[test]
    fn a_base_older_than_the_tombstone_horizon_keeps_only_what_the_server_has() {
        let (a_dir, b_dir) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let (a, b) = (Phone::new(a_dir.path()).open(), Phone::new(b_dir.path()).open());
        let mut server = Server::default();
        let id = |s: &str| reins_proto::ids::GrantId(s.to_owned());
        a.insert_grant(&crate::store::grants::tests::read_grant("ancient", "c1", None), "Claude").unwrap();
        a.insert_grant(&crate::store::grants::tests::read_grant("kept", "c1", None), "Claude").unwrap();
        server.sync(&a);
        server.pull(&b);
        assert!(a.revoke_grant(&id("ancient")).unwrap());
        assert!(a.delete_ended_grant(&id("ancient"), 100).unwrap());
        // The removal is older than the horizon by now: its tombstone is gone, and so is B's base's usefulness.
        let long_ago = crate::store::unix_now() - crate::store::TOMBSTONE_HORIZON_SECS - 86_400;
        a.lock().unwrap().execute("UPDATE meta SET value = ?1 WHERE key LIKE 'app.tombstone.%'", [long_ago]).unwrap();
        a.add_account("github", "touch", 1).unwrap();
        server.sync(&a);
        server.pull(&a); // nothing new; the tombstone is still there until a merge prunes it
        let mut state: serde_json::Value = serde_json::from_slice(server.state.as_ref().unwrap()).unwrap();
        state["tables"]["meta"]["rows"]
            .as_array_mut()
            .unwrap()
            .retain(|r| !r[0]["Text"].as_str().unwrap().starts_with("app.tombstone."));
        server.state = Some(Zeroizing::new(serde_json::to_vec(&state).unwrap()));
        b.meta_set(AGREED_AT, &long_ago.to_string()).unwrap();
        b.add_account("github", "b-offline", 2).unwrap();
        server.sync(&b);
        let left: Vec<String> = b.grants().unwrap().into_iter().map(|g| g.grant.id.0).collect();
        assert_eq!(left, ["kept"], "the old grant only B had is dropped; the one the server has stays");
        assert!(!names(&b).contains(&"b-offline".to_owned()), "B's own extras are suspect too (kept in the backup)");
        assert!(b.read_side(BACKUP, BACKUP).unwrap().is_some());
    }

    /// The security-relevant tables and this phone's own state, as rows, for comparing before and after.
    fn security_state(store: &Store) -> BTreeMap<String, Vec<String>> {
        let conn = store.lock().unwrap();
        let mut out = BTreeMap::new();
        for (table, order) in [
            ("grants", "id"),
            ("accounts", "service, account"),
            ("secrets", "service, account"),
            ("desktop_keys", "connection_id"),
            ("mcp_servers", "id"),
            ("autopilot_targets", "key"),
            ("autopilot_settings", "scope"),
            ("pending", "id"),
            ("handled", "id"),
        ] {
            let mut stmt = conn.prepare(&format!("SELECT * FROM {table} ORDER BY {order}")).unwrap();
            let n = stmt.column_count();
            let rows: Vec<String> = stmt
                .query_map([], |r| {
                    Ok((0..n)
                        .map(|i| format!("{:?}", r.get::<_, rusqlite::types::Value>(i).unwrap()))
                        .collect::<Vec<_>>()
                        .join("|"))
                })
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            out.insert(table.to_owned(), rows);
        }
        let mut stmt = conn.prepare("SELECT key FROM meta WHERE key LIKE 'app.tombstone.%' ORDER BY key").unwrap();
        out.insert(
            "tombstones".to_owned(),
            stmt.query_map([], |r| r.get(0)).unwrap().collect::<Result<_, _>>().unwrap(),
        );
        out
    }

    /// What protects the account comes through an app update (the migrations) and a pull unchanged, even when the
    /// server's state is stale or was uploaded by an older version that brought back what was removed: revocations
    /// stay, tombstones stay, removed records stay removed, lockdown stays, and this phone's own pending requests and
    /// handled ids are untouched. Only what migration 11 drops (a topic-keyed desktop answer) goes.
    #[test]
    fn security_state_survives_migrations_and_imports() {
        let dir = tempfile::tempdir().unwrap();
        let phone = Phone::new(dir.path());
        let store = phone.open();
        let id = |s: &str| reins_proto::ids::GrantId(s.to_owned());
        let g = |s: &str| crate::store::grants::tests::read_grant(s, "c1", Some(5));
        store.insert_grant(&g("active"), "Claude").unwrap();
        store.insert_grant(&g("revoked"), "Claude").unwrap();
        assert!(store.revoke_grant(&id("revoked")).unwrap());
        store.insert_grant(&g("gone"), "Claude").unwrap();
        assert!(store.revoke_grant(&id("gone")).unwrap());
        assert!(store.delete_ended_grant(&id("gone"), 100).unwrap());
        store.add_account("github", "work", 1).unwrap();
        store.secret_put("github", "work", b"token").unwrap();
        store.pin_desktop_key("c1", "laptop-key", "Laptop", 1).unwrap();
        store
            .lock()
            .unwrap()
            .execute_batch(
                "INSERT INTO mcp_servers (id, name, url, auth, added_at) VALUES ('m1', 'M', 'https://m.example', 'none', 1);
                 INSERT INTO autopilot_targets (key, at) VALUES ('t1', 1);
                 INSERT INTO autopilot_settings (scope, mode, updated_at) VALUES ('', 'lockdown', 1);
                 INSERT INTO handled (id, at) VALUES ('h1', 1);",
            )
            .unwrap();
        store.park("p1", crate::PendingKind::Request, 1, crate::store::unix_now(), b"{}").unwrap();
        let mut server = Server::default();
        server.sync(&store);
        let before = security_state(&store);
        assert_eq!(before["tombstones"], ["app.tombstone.grants:[\"gone\"]"]);

        // An app update: sealed by version 10, with a topic-keyed desktop answer migration 11 drops.
        desktop_topic_grant(&store);
        store.lock().unwrap().execute_batch("PRAGMA user_version = 10;").unwrap();
        store.meta_set("sealed-by", "an older version").unwrap();
        drop(store);
        let store = phone.open();
        assert_eq!(security_state(&store), before, "the migrations change nothing that protects the account");

        // A pull of a state that brings back what was removed or revoked (as an older version could upload it): the
        // revoked grant active again, the deleted one back without its tombstone, the token and computer key of a
        // later removal still there, lockdown cleared at the same time on the other phone... merged, not imported.
        let mut stale: serde_json::Value = serde_json::from_slice(&store.export_account().unwrap()).unwrap();
        let grants = stale["tables"]["grants"]["rows"].as_array_mut().unwrap();
        for row in grants.iter_mut() {
            if row[0]["Text"] == "revoked" {
                let mut resumed: serde_json::Value = serde_json::from_str(row[4]["Text"].as_str().unwrap()).unwrap();
                resumed["revoked"] = serde_json::json!(false);
                row[4] = serde_json::json!({"Text": resumed.to_string()});
            }
        }
        let mut back = grants[0].clone();
        back[0] = serde_json::json!({"Text": "gone"});
        back[4] = serde_json::json!({"Text": serde_json::to_string(&g("gone")).unwrap()});
        grants.push(back);
        stale["tables"]["meta"]["rows"]
            .as_array_mut()
            .unwrap()
            .retain(|r| !r[0]["Text"].as_str().unwrap().starts_with("app.tombstone."));
        let pulled = store.pull(2, Some(&serde_json::to_vec(&stale).unwrap())).unwrap();
        assert_eq!(pulled, Pulled::Merged, "this phone's tombstone and revocation are to be uploaded back");
        assert_eq!(security_state(&store), before, "nothing revoked, removed or locked down comes back undone");
        let (_, plain) = store.pending_upload().unwrap().unwrap();
        assert!(String::from_utf8_lossy(&plain).contains("app.tombstone.grants:[\\\"gone\\\"]"));
    }

    /// Adding a record again on purpose clears its tombstone, and that reaches the other phone.
    #[test]
    fn adding_a_removed_account_again_syncs() {
        let (a_dir, b_dir) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let (a, b) = (Phone::new(a_dir.path()).open(), Phone::new(b_dir.path()).open());
        let mut server = Server::default();
        a.add_account("github", "work", 1).unwrap();
        server.sync(&a);
        a.remove_account("github", "work").unwrap();
        a.secret_delete("github", "work").unwrap();
        server.sync(&a);
        server.pull(&b);
        assert!(names(&b).is_empty());
        b.add_account("github", "work", 5).unwrap();
        b.add_account("github", "other", 6).unwrap();
        a.add_account("github", "concurrent", 7).unwrap();
        server.sync(&a);
        server.sync(&b);
        server.pull(&a);
        assert_eq!(names(&a), ["concurrent", "other", "work"]);
        assert_eq!(names(&b), ["concurrent", "other", "work"]);
    }

    /// A state that still carries something removed elsewhere (an older version uploaded it) brings it in nowhere:
    /// whatever has a tombstone is dropped from every snapshot read.
    #[test]
    fn an_import_never_brings_back_a_tombstoned_record() {
        let dir = tempfile::tempdir().unwrap();
        let store = Phone::new(dir.path()).open();
        store.insert_grant(&crate::store::grants::tests::read_grant("g", "c1", None), "Claude").unwrap();
        let mut state: serde_json::Value = serde_json::from_slice(&store.export_account().unwrap()).unwrap();
        state["tables"]["meta"]["rows"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!([{"Text": "app.tombstone.grants:[\"g\"]"}, {"Integer": crate::store::unix_now()}]));
        let fresh_dir = tempfile::tempdir().unwrap();
        let fresh = Phone::new(fresh_dir.path()).open();
        assert_eq!(fresh.pull(1, Some(&serde_json::to_vec(&state).unwrap())).unwrap(), Pulled::Imported);
        assert!(fresh.grants().unwrap().is_empty());
    }

    /// The tables that only mean something on this phone are never in what is synced; Activity is.
    #[test]
    fn device_tables_are_never_synced() {
        let dir = tempfile::tempdir().unwrap();
        let store = Phone::new(dir.path()).open();
        crate::store::tests::fill(&store);
        store
            .lock()
            .unwrap()
            .execute_batch(
                "INSERT INTO handled (id, at) VALUES ('h1', 1);
                 INSERT INTO autopilot_rate (connection_id, at) VALUES ('c1', 1);
                 INSERT INTO autopilot_suggestions (request_id, at, data) VALUES ('r1', 1, x'01');",
            )
            .unwrap();
        let state: serde_json::Value = serde_json::from_slice(&store.export_account().unwrap()).unwrap();
        for table in ["session", "pending", "handled", "autopilot_rate", "autopilot_suggestions", "audit_snapshots"] {
            assert!(state["tables"].get(table).is_none(), "{table} is this phone's own");
        }
        assert_eq!(state["tables"]["audit"]["rows"].as_array().unwrap().len(), 1, "Activity is synced");
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

    /// What approving writes on both phones (an Activity entry each, a use of the same grant each) is what made a
    /// phone "unsynced" all the time: the state the old restore refused. It merges without a conflict, a notice or a
    /// backup: both phones' entries, and every use counted.
    #[test]
    fn approving_on_both_phones_never_conflicts() {
        let (a_dir, b_dir) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let (a, b) = (Phone::new(a_dir.path()).open(), Phone::new(b_dir.path()).open());
        let mut server = Server::default();
        a.insert_grant(&crate::store::grants::tests::read_grant("g", "c1", Some(10)), "Claude").unwrap();
        server.sync(&a);
        server.pull(&b);
        let approve = |store: &Store, n: i64| {
            let mut record = crate::store::tests::audit_record();
            record.at = n;
            store.append_audit(&record).unwrap();
            store
                .lock()
                .unwrap()
                .execute("UPDATE grants SET grant_json = json_set(grant_json, '$.uses', json_extract(grant_json, '$.uses') + 1)", [])
                .unwrap();
        };
        for n in 0..3 {
            approve(&a, 100 + n);
            approve(&b, 200 + n);
        }
        server.sync(&a);
        server.sync(&b);
        server.pull(&a);
        for store in [&a, &b] {
            assert_eq!(store.activity(50).unwrap().len(), 6, "both phones' entries");
            assert_eq!(store.grants().unwrap()[0].grant.uses, 6, "every use counted");
            assert_eq!(store.sync_notice().unwrap(), None);
            assert_eq!(store.read_side(BACKUP, BACKUP).unwrap(), None);
        }
    }

    /// A push's Activity entry as the engine logs it: the call's push kept with it, and this phone's snapshot.
    fn push_entry(
        summary: &reins_proto::desktop::PushSummary,
        detail: &str,
        decided_by: &str,
    ) -> crate::store::AuditRecord {
        let call = crate::store::audit_push::tests::push_call(summary);
        let parked = crate::views::ParkedRequest {
            request: reins_proto::relay::RelayRequest {
                v: 1,
                id: "r1".into(),
                connection_id: "desk".into(),
                connection_label: "Laptop".to_owned(),
                created_at: 9,
                wait_until: None,
                account: None,
                call: call.clone(),
            },
            messages: Vec::new(),
            covered: BTreeMap::new(),
            account: None,
            accounts: Vec::new(),
            shared_accounts: Vec::new(),
            connector: None,
        };
        crate::store::AuditRecord {
            seq: 0,
            at: 10,
            connection_id: "desk".to_owned(),
            connection_label: "Laptop".to_owned(),
            action: "write".to_owned(),
            outcome: "sent".to_owned(),
            detail: detail.to_owned(),
            grant_id: None,
            service: "github".to_owned(),
            account: Some("me".to_owned()),
            count: 1,
            op: "git_push".to_owned(),
            info: crate::store::AuditInfo {
                request: Some(Box::new(crate::titles::request(&call, 9))),
                git: crate::store::AuditPush::of_call(&call).map(Box::new),
                snapshot: Some(Box::new(parked)),
                decided_by: decided_by.to_owned(),
                ..crate::store::AuditInfo::default()
            },
        }
    }

    /// A push decided on one phone (by its user, or by Bypass there) shows its commits and files, and their changes
    /// while small, on the other phone, which never saw its approval: from the copy synced with the entry. A copy from
    /// another device that is too large or malformed is dropped there; the entry itself, its outcome and who decided
    /// it stay as they are, and nothing is duplicated or rewritten by later syncs.
    #[test]
    fn a_push_decided_on_one_phone_shows_its_commits_and_files_on_the_other() {
        use crate::store::audit_push::tests::summary;
        let (a_dir, b_dir) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let (a, b) = (Phone::new(a_dir.path()).open(), Phone::new(b_dir.path()).open());
        let mut server = Server::default();
        server.sync(&a);
        server.pull(&b);
        a.append_audit(&push_entry(&summary(2, 2, 5), "small", "bypass")).unwrap();
        a.append_audit(&push_entry(&summary(3, 4, 300), "large", "")).unwrap();
        // Another device's copies that no device of this version writes: one too large, one not a push at all.
        let mut oversize = push_entry(&summary(1, 1, 3), "oversize", "");
        let mut big = oversize.info.git.as_deref().unwrap().clone();
        big.summary.updates[0].commits[0].files[0].patch =
            Some(format!("@@ -1 +1 @@\n{}", vec!["+x".repeat(100); 390].join("\n")));
        oversize.info.git = Some(Box::new(big));
        oversize.info.snapshot = None;
        a.append_audit(&oversize).unwrap();
        let malformed = serde_json::json!({"decided_by": "autopilot", "git": {"repo": 5, "summary": "no"},
            "request": {"full": "git push me/app"}});
        let sealed = a.seal(super::super::audit::INFO_AAD, malformed.to_string().as_bytes()).unwrap();
        a.lock()
            .unwrap()
            .execute(
                "INSERT INTO audit (at, connection_id, connection_label, action, outcome, detail, service, item_count, \
                 info, op) VALUES (11, 'desk', 'Laptop', 'write', 'sent', x'00', 'github', 1, ?1, 'git_push')",
                params![sealed],
            )
            .unwrap();
        server.sync(&a);
        assert_eq!(server.pull(&b), Pulled::Imported);

        let entries = b.activity(10).unwrap();
        assert_eq!(entries.len(), 4);
        let find = |detail: &str| entries.iter().find(|e| e.detail == detail).unwrap().clone();
        let (small, large, oversize) = (find("small"), find("large"), find("oversize"));
        assert!(
            b.audit_snapshot(small.seq).unwrap().is_none(),
            "the approval itself stays on the phone that showed it"
        );

        // The small push, with its diffs; who decided it unchanged.
        let push = b.activity_push(small.seq).unwrap().expect("the push");
        assert!(!push.this_device && push.diffs && !push.diffs_not_kept);
        let r = &push.git.refs[0];
        assert_eq!((r.commit_count, r.commits.len(), r.commits[0].files.len()), (2, 2, 2));
        assert_eq!((r.commits[0].files[0].additions, r.commits[0].files[0].deletions), (Some(3), Some(1)));
        assert_eq!(r.commits[0].files[0].diff.len(), 6);
        let shown = crate::views::activity_entry(&small);
        assert_eq!((shown.decided_by.as_str(), shown.outcome.as_str()), ("bypass", "sent"));
        let listed = shown.request.unwrap().git.expect("the list has the push");
        assert!(!crate::store::has_diffs(&listed), "without diff lines in the list");
        assert_eq!(listed.refs[0].commits.len(), 2);
        let approval = b.activity_approval(small.seq).unwrap().expect("opens on the synced push");
        assert_eq!((approval.kind, approval.op.as_str()), (crate::types::ApprovalKind::Write, "git_push"));
        assert!(approval.quick.is_none() && approval.wait_until.is_none());
        assert!(crate::store::has_diffs(approval.git.as_ref().unwrap()));

        // The large push: commits and files with their counts, the diffs not kept, and said so.
        let push = b.activity_push(large.seq).unwrap().expect("the push");
        assert!(!push.diffs && push.diffs_not_kept);
        let r = &push.git.refs[0];
        assert_eq!((r.commits.len(), r.commits[0].files.len()), (3, 4));
        assert_eq!(r.commits[0].files[0].additions, Some(3));

        // On the phone that showed them: its own copy, every diff.
        let here = a.activity(10).unwrap().into_iter().find(|e| e.detail == "large").unwrap();
        let push = a.activity_push(here.seq).unwrap().unwrap();
        assert!(push.this_device && push.diffs && !push.diffs_not_kept);

        // Too large, or malformed: dropped; the rest of the entry stays.
        assert!(oversize.info.git.is_none() && oversize.info.request.is_some());
        assert!(b.activity_push(oversize.seq).unwrap().is_none());
        assert!(b.activity_approval(oversize.seq).unwrap().is_none(), "the full request is what it shows");
        let bad = entries.iter().find(|e| e.at == 11).unwrap();
        assert!(bad.info.git.is_none());
        assert_eq!(bad.info.decided_by, "autopilot");
        assert_eq!(bad.info.request.as_ref().unwrap().full, "git push me/app");

        // Syncing again rewrites and duplicates nothing.
        let before: Vec<_> = b.activity(10).unwrap();
        b.append_audit(&push_entry(&summary(1, 1, 1), "from b", "")).unwrap();
        server.sync(&b);
        server.pull(&a);
        let after = b.activity(10).unwrap();
        assert_eq!(after.len(), 5);
        assert!(before.iter().all(|e| after.contains(e)));
        assert_eq!(a.activity(10).unwrap().len(), 5);
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
