//! The accounts the user has connected (several per service), and the grants tied to them.

use rusqlite::params;

use super::Store;
use crate::CoreError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredAccount {
    /// "gmail"
    pub service: String,
    /// Lower-case address.
    pub account: String,
    pub added_at: i64,
}

impl Store {
    /// Every connected account, oldest first.
    pub fn accounts(&self) -> Result<Vec<StoredAccount>, CoreError> {
        let conn = self.lock();
        let mut stmt = conn.prepare("SELECT service, account, added_at FROM accounts ORDER BY added_at, account")?;
        let rows = stmt
            .query_map([], |r| {
                Ok(StoredAccount {
                    service: r.get(0)?,
                    account: r.get(1)?,
                    added_at: r.get(2)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Registers an account; false when it was already there. The first account also takes over the grants
    /// made before accounts existed.
    pub fn add_account(&self, service: &str, account: &str, now: i64) -> Result<bool, CoreError> {
        let account = account.trim().to_lowercase();
        let conn = self.lock();
        let first: bool = conn.query_row("SELECT COUNT(*) = 0 FROM accounts", [], |r| r.get(0))?;
        let added = conn.execute(
            "INSERT OR IGNORE INTO accounts (service, account, added_at) VALUES (?1, ?2, ?3)",
            params![service, account, now],
        )? > 0;
        if added && first {
            conn.execute(
                "UPDATE grants SET grant_json = json_set(grant_json, '$.account', ?1) \
                 WHERE json_extract(grant_json, '$.account') IS NULL AND json_extract(grant_json, '$.scope.action') != 'accounts'",
                params![account],
            )?;
        }
        Ok(added)
    }

    /// Forgets an account and deletes its grants. Returns false when it was not registered.
    pub fn remove_account(&self, service: &str, account: &str) -> Result<bool, CoreError> {
        let account = account.trim().to_lowercase();
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        let removed =
            tx.execute("DELETE FROM accounts WHERE service = ?1 AND account = ?2", params![service, account])? > 0;
        if removed {
            // Gmail's grants carry no service of their own.
            tx.execute(
                "DELETE FROM grants WHERE json_extract(grant_json, '$.account') = ?1 \
                 AND COALESCE(json_extract(grant_json, '$.scope.service'), 'gmail') = ?2",
                params![account, service],
            )?;
        }
        tx.commit()?;
        Ok(removed)
    }
}

#[cfg(test)]
mod tests {
    use crate::store::grants::tests::read_grant;
    use crate::store::tests::open;

    #[test]
    fn accounts_are_kept_in_order_and_unique() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());
        assert!(store.add_account("gmail", "Work@Gmail.com", 10).unwrap());
        assert!(store.add_account("gmail", "me@gmail.com", 20).unwrap());
        assert!(!store.add_account("gmail", "work@gmail.com", 30).unwrap(), "same address, other case");
        let names: Vec<_> = store.accounts().unwrap().into_iter().map(|a| a.account).collect();
        assert_eq!(names, ["work@gmail.com", "me@gmail.com"]);
        assert!(store.remove_account("gmail", "WORK@gmail.com").unwrap());
        assert!(!store.remove_account("gmail", "work@gmail.com").unwrap());
        assert_eq!(store.accounts().unwrap().len(), 1);
    }

    #[test]
    fn the_first_account_adopts_unbound_grants_and_removing_it_deletes_them() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());
        store.insert_grant(&read_grant("g1", "c1", None), "Claude").unwrap();
        store.add_account("gmail", "me@gmail.com", 1).unwrap();
        store
            .insert_grant(&read_grant("g2", "c1", None).for_account(Some("other@gmail.com".into())), "Claude")
            .unwrap();
        store.add_account("gmail", "other@gmail.com", 2).unwrap();
        let accounts: Vec<_> = store.grants().unwrap().into_iter().map(|g| (g.grant.id.0, g.grant.account)).collect();
        assert!(accounts.contains(&("g1".to_owned(), Some("me@gmail.com".to_owned()))), "{accounts:?}");
        assert!(accounts.contains(&("g2".to_owned(), Some("other@gmail.com".to_owned()))), "{accounts:?}");
        store.remove_account("gmail", "me@gmail.com").unwrap();
        let left: Vec<_> = store.grants().unwrap().into_iter().map(|g| g.grant.id.0).collect();
        assert_eq!(left, ["g2"]);
    }
}
