//! The accounts the user has connected (several per service), and the grants tied to them.

use rusqlite::{OptionalExtension, params};

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
    /// Moves the vault alias, resealed key, grants, session and cached account id in one durable transaction.
    /// Secrets bind their account name as AAD, so a SQL-only rename would make them undecryptable.
    pub(crate) fn rename_session_account(
        &self,
        service: &str,
        from: &str,
        session: &super::StoredSession,
        account_id_key: &str,
        account_id: &str,
    ) -> Result<(), CoreError> {
        let to = &session.email;
        let token = self.seal("session.refresh_token", session.refresh_token.as_bytes())?;
        let mut conn = self.lock()?;
        let tx = conn.transaction()?;
        if from != to {
            let raw: Option<Vec<u8>> = tx
                .query_row(
                    "SELECT value FROM secrets WHERE service = ?1 AND account = ?2",
                    params![service, from],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(raw) = raw {
                let secret = zeroize::Zeroizing::new(self.unseal(&format!("secret.{service}.{from}"), &raw)?);
                let sealed = self.seal(&format!("secret.{service}.{to}"), &secret)?;
                tx.execute("INSERT INTO secrets (service, account, value) VALUES (?1, ?2, ?3) ON CONFLICT (service, account) DO UPDATE SET value = excluded.value", params![service, to, sealed])?;
                tx.execute("DELETE FROM secrets WHERE service = ?1 AND account = ?2", params![service, from])?;
            }
            tx.execute("INSERT OR IGNORE INTO accounts (service, account, added_at) SELECT service, ?3, added_at FROM accounts WHERE service = ?1 AND account = ?2", params![service, from, to])?;
            tx.execute("DELETE FROM accounts WHERE service = ?1 AND account = ?2", params![service, from])?;
            tx.execute("UPDATE grants SET grant_json = json_set(grant_json, '$.account', ?3) WHERE json_extract(grant_json, '$.account') = ?2 AND COALESCE(json_extract(grant_json, '$.scope.service'), 'gmail') = ?1", params![service, from, to])?;
        }
        tx.execute(
            "INSERT INTO meta (key, value) VALUES (?1, ?2) ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            params![format!("app.{account_id_key}"), account_id],
        )?;
        tx.execute("INSERT INTO session (id, server_url, email, refresh_token) VALUES (1, ?1, ?2, ?3) ON CONFLICT (id) DO UPDATE SET server_url = excluded.server_url, email = excluded.email, refresh_token = excluded.refresh_token", params![session.server_url, session.email, token])?;
        tx.commit()?;
        drop(conn);
        if let Some(control) = &self.control {
            control.meta_set(account_id_key, account_id)?;
            self.save_session(session)?;
        }
        Ok(())
    }

    /// Every connected account, oldest first.
    pub fn accounts(&self) -> Result<Vec<StoredAccount>, CoreError> {
        let conn = self.lock()?;
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
        let conn = self.lock()?;
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
        let mut conn = self.lock()?;
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
    fn session_rename_keeps_other_services_and_rolls_back_everything_on_write_failure() {
        use super::super::StoredSession;
        use reins_policy::{Scope, ServiceScope};
        use zeroize::Zeroizing;
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());
        let (old, new) = ("old@example.com", "new@example.com");
        let initial = StoredSession {
            server_url: "https://app.example.com".into(),
            email: old.into(),
            refresh_token: Zeroizing::new("refresh-old".into()),
        };
        store.save_session(&initial).unwrap();
        store.secret_put("vault", old, b"key").unwrap();
        store.add_account("vault", old, 10).unwrap();
        store.add_account("gmail", old, 11).unwrap();
        let mut vault = read_grant("vault-grant", "c1", None).for_account(Some(old.into()));
        vault.scope = Scope::Service(ServiceScope {
            service: "vault".into(),
            access: "read".into(),
            resources: vec!["item-id".into()],
            labels: vec![],
            any: false,
            classes: vec![],
        });
        store.insert_grant(&vault, "AI").unwrap();
        store.insert_grant(&read_grant("mail-grant", "c1", None).for_account(Some(old.into())), "AI").unwrap();
        let renamed = StoredSession {
            email: new.into(),
            refresh_token: Zeroizing::new("refresh-new".into()),
            ..initial.clone()
        };
        store.lock().unwrap().execute_batch("CREATE TEMP TRIGGER refuse_rename BEFORE INSERT ON session BEGIN SELECT RAISE(ABORT, 'test write failure'); END;").unwrap();
        assert!(store.rename_session_account("vault", old, &renamed, "subject-new", "account-id").is_err());
        assert_eq!(store.load_session().unwrap().unwrap(), initial);
        assert_eq!(store.secret_get("vault", old).unwrap().unwrap(), b"key");
        assert!(store.secret_get("vault", new).unwrap().is_none());
        assert!(store.meta_get("subject-new").unwrap().is_none());
        assert!(store.grants().unwrap().iter().all(|g| g.grant.account.as_deref() == Some(old)));
        store.lock().unwrap().execute_batch("DROP TRIGGER refuse_rename").unwrap();
        store.rename_session_account("vault", old, &renamed, "subject-new", "account-id").unwrap();
        assert_eq!(store.load_session().unwrap().unwrap(), renamed);
        assert_eq!(store.secret_get("vault", new).unwrap().unwrap(), b"key");
        assert_eq!(store.meta_get("subject-new").unwrap().as_deref(), Some("account-id"));
        let grants = store.grants().unwrap();
        assert_eq!(grants.iter().find(|g| g.grant.id.0 == "vault-grant").unwrap().grant.account.as_deref(), Some(new));
        assert_eq!(grants.iter().find(|g| g.grant.id.0 == "mail-grant").unwrap().grant.account.as_deref(), Some(old));
        assert!(store.accounts().unwrap().iter().any(|a| a.service == "gmail" && a.account == old));
        // Two concurrent metadata sources can discover the same rename; repeating it must never delete the key.
        store.rename_session_account("vault", new, &renamed, "subject-new", "account-id").unwrap();
        assert_eq!(store.secret_get("vault", new).unwrap().unwrap(), b"key");
        assert!(store.accounts().unwrap().iter().any(|a| a.service == "vault" && a.account == new));
    }

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
