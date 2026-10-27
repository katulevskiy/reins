//! The credentials this executor has leased out, kept on disk (`state_dir/leases.json`, mode 0600) until they are
//! revoked upstream. A token is saved before it is sealed and handed over, so an executor that crashes right after
//! minting still knows to revoke it (Review Focus 3: a lease never outlives deprovisioning).

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

/// How long a revoked lease is remembered (so a late revocation notice is still acknowledged).
const TOMBSTONE_SECS: i64 = 7 * 24 * 3600;

/// One minted token.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Held {
    pub token: String,
    /// The lease's expiry (countersigned): it is revoked then at the latest.
    pub expires_at: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Data {
    /// Lease id → the tokens minted for it (more than one if a mint was retried after a crash).
    #[serde(default)]
    leases: BTreeMap<String, Vec<Held>>,
    /// Lease id → when it was revoked upstream.
    #[serde(default)]
    revoked: BTreeMap<String, i64>,
}

pub struct LeaseStore {
    path: PathBuf,
    data: Mutex<Data>,
}

/// Writes `bytes` to `path` with owner-only permissions, atomically (a temporary file, synced, then renamed).
pub(crate) fn write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension("tmp");
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let err = |e: std::io::Error| format!("writing {}: {e}", path.display());
    let mut f = opts.open(&tmp).map_err(err)?;
    f.write_all(bytes).map_err(err)?;
    f.sync_all().map_err(err)?;
    std::fs::rename(&tmp, path).map_err(err)
}

impl LeaseStore {
    /// Opens (or starts) the store in `state_dir`.
    pub fn open(state_dir: &Path) -> Result<Self, String> {
        let path = state_dir.join("leases.json");
        let data = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| format!("{} is damaged: {e}", path.display()))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Data::default(),
            Err(e) => return Err(format!("reading {}: {e}", path.display())),
        };
        Ok(Self {
            path,
            data: Mutex::new(data),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Data> {
        self.data.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn save(&self, data: &Data) -> Result<(), String> {
        let bytes = serde_json::to_vec(data).map_err(|e| e.to_string())?;
        write_private(&self.path, &bytes)
    }

    /// Records a minted token, on disk, before it is handed over.
    pub fn hold(&self, lease: &str, token: &str, expires_at: i64) -> Result<(), String> {
        let mut data = self.lock();
        data.leases.entry(lease.to_owned()).or_default().push(Held {
            token: token.to_owned(),
            expires_at,
        });
        self.save(&data)
    }

    /// The tokens held for `lease`.
    #[must_use]
    pub fn tokens(&self, lease: &str) -> Vec<Held> {
        self.lock().leases.get(lease).cloned().unwrap_or_default()
    }

    /// Whether this executor issued `lease` (held now, or revoked recently).
    #[must_use]
    pub fn knows(&self, lease: &str) -> bool {
        let data = self.lock();
        data.leases.contains_key(lease) || data.revoked.contains_key(lease)
    }

    /// The leases past their expiry that still hold tokens.
    #[must_use]
    pub fn expired(&self, now: i64) -> Vec<String> {
        self.lock()
            .leases
            .iter()
            .filter(|(_, held)| held.iter().any(|h| h.expires_at <= now))
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Forgets `lease`'s tokens once they are revoked upstream, and remembers that it was.
    pub fn revoked(&self, lease: &str, now: i64) -> Result<(), String> {
        let mut data = self.lock();
        data.leases.remove(lease);
        data.revoked.insert(lease.to_owned(), now);
        data.revoked.retain(|_, at| now - *at < TOMBSTONE_SECS);
        self.save(&data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_kept_on_disk_privately_until_revoked() {
        let dir = tempfile::tempdir().unwrap();
        let store = LeaseStore::open(dir.path()).unwrap();
        assert!(!store.knows("ls_1"));
        store.hold("ls_1", "ghs_a", 100).unwrap();
        store.hold("ls_1", "ghs_b", 100).unwrap();
        store.hold("ls_2", "ghs_c", 500).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join("leases.json")).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        // A restarted executor still knows what to revoke.
        let again = LeaseStore::open(dir.path()).unwrap();
        assert_eq!(again.tokens("ls_1").iter().map(|h| h.token.as_str()).collect::<Vec<_>>(), ["ghs_a", "ghs_b"]);
        assert_eq!(again.expired(200), ["ls_1"]);
        again.revoked("ls_1", 200).unwrap();
        let after = LeaseStore::open(dir.path()).unwrap();
        assert!(after.tokens("ls_1").is_empty());
        assert!(after.knows("ls_1"), "a revoked lease is remembered");
        assert_eq!(after.expired(200), Vec::<String>::new());
        assert_eq!(after.tokens("ls_2").len(), 1);
    }
}
