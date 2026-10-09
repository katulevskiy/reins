use diesel::prelude::*;

use crate::{
    api::EmptyResult,
    crypto,
    db::{DbConn, DbConnInner, schema::reins_vault_passkeys},
    error::MapResult,
};

use super::UserId;

/// A passkey that opens a user's vault: the account secret sealed with a key only the passkey's PRF output derives
/// (`reins_proto::vault_passkey`). The server cannot open it.
#[derive(Clone, Debug, Identifiable, Queryable, Insertable)]
#[diesel(table_name = reins_vault_passkeys)]
#[diesel(primary_key(user_uuid, credential_hash))]
pub struct ReinsVaultPasskey {
    pub user_uuid: UserId,
    /// SHA-256 (hex) of `credential_id`, the key (credential ids may be long).
    pub credential_hash: String,
    /// Base64url, as the phone sent it.
    pub credential_id: String,
    pub wrapped: String,
    pub name: String,
    /// Unix seconds.
    pub created_at: i64,
}

impl ReinsVaultPasskey {
    pub fn new(user_uuid: UserId, credential_id: String, wrapped: String, name: String, created_at: i64) -> Self {
        Self {
            user_uuid,
            credential_hash: credential_hash(&credential_id),
            credential_id,
            wrapped,
            name,
            created_at,
        }
    }

    /// The user's passkeys, oldest first.
    pub async fn find_by_user(user_uuid: &UserId, conn: &DbConn) -> Vec<Self> {
        conn.run(move |c| q_find_by_user(c, user_uuid)).await.unwrap_or_default()
    }

    /// Keeps `self`, replacing the copy for the same passkey; refused (`Ok(false)`) when the user keeps `max` others.
    pub async fn save(&self, max: usize, conn: &DbConn) -> Result<bool, crate::Error> {
        conn.run(move |c| q_save(c, self, max)).await.map_res("Error saving the vault passkey")
    }

    pub async fn delete(user_uuid: &UserId, credential_id: &str, conn: &DbConn) -> EmptyResult {
        conn.run(move |c| q_delete(c, user_uuid, credential_id)).await.map_res("Error removing the vault passkey")
    }

    /// Every passkey of the user (a vault reset: they sealed the old account secret).
    pub async fn delete_all(user_uuid: &UserId, conn: &DbConn) -> EmptyResult {
        conn.run(move |c| q_delete_all(c, user_uuid)).await.map_res("Error removing the vault passkeys")
    }
}

fn credential_hash(credential_id: &str) -> String {
    crypto::sha256_hex(credential_id.as_bytes())
}

fn q_find_by_user(c: &mut DbConnInner, user_uuid: &UserId) -> QueryResult<Vec<ReinsVaultPasskey>> {
    reins_vault_passkeys::table
        .filter(reins_vault_passkeys::user_uuid.eq(user_uuid))
        .order(reins_vault_passkeys::created_at.asc())
        .load::<ReinsVaultPasskey>(c)
}

fn q_save(c: &mut DbConnInner, row: &ReinsVaultPasskey, max: usize) -> QueryResult<bool> {
    c.transaction(|c| {
        let others = reins_vault_passkeys::table
            .filter(reins_vault_passkeys::user_uuid.eq(&row.user_uuid))
            .filter(reins_vault_passkeys::credential_hash.ne(&row.credential_hash))
            .count()
            .get_result::<i64>(c)?;
        if usize::try_from(others).unwrap_or(usize::MAX) >= max {
            return Ok(false);
        }
        diesel::delete(
            reins_vault_passkeys::table
                .filter(reins_vault_passkeys::user_uuid.eq(&row.user_uuid))
                .filter(reins_vault_passkeys::credential_hash.eq(&row.credential_hash)),
        )
        .execute(c)?;
        diesel::insert_into(reins_vault_passkeys::table).values(row).execute(c)?;
        Ok(true)
    })
}

fn q_delete(c: &mut DbConnInner, user_uuid: &UserId, credential_id: &str) -> QueryResult<()> {
    diesel::delete(
        reins_vault_passkeys::table
            .filter(reins_vault_passkeys::user_uuid.eq(user_uuid))
            .filter(reins_vault_passkeys::credential_hash.eq(credential_hash(credential_id))),
    )
    .execute(c)
    .map(drop)
}

pub(super) fn q_delete_all(c: &mut DbConnInner, user_uuid: &UserId) -> QueryResult<()> {
    diesel::delete(reins_vault_passkeys::table.filter(reins_vault_passkeys::user_uuid.eq(user_uuid)))
        .execute(c)
        .map(drop)
}

#[cfg(all(test, sqlite))]
mod tests {
    use super::super::reins_device::test_db;
    use super::*;

    fn uid(s: &str) -> UserId {
        UserId::from(s.to_owned())
    }

    fn passkey(user: &str, id: &str, at: i64) -> ReinsVaultPasskey {
        ReinsVaultPasskey::new(uid(user), id.to_owned(), format!("wrapped-{id}"), "Pixel".to_owned(), at)
    }

    #[test]
    fn a_passkey_is_replaced_not_duplicated_and_the_limit_counts_the_others() {
        let mut c = test_db();
        assert!(q_save(&mut c, &passkey("a", "one", 1), 2).unwrap());
        assert!(q_save(&mut c, &passkey("a", "two", 2), 2).unwrap());
        assert!(!q_save(&mut c, &passkey("a", "three", 3), 2).unwrap(), "over the limit");
        let mut again = passkey("a", "one", 4);
        again.wrapped = "newer".to_owned();
        assert!(q_save(&mut c, &again, 2).unwrap(), "replacing one is not a new one");
        let list = q_find_by_user(&mut c, &uid("a")).unwrap();
        assert_eq!(list.iter().map(|p| p.credential_id.as_str()).collect::<Vec<_>>(), ["two", "one"]);
        assert_eq!(list[1].wrapped, "newer");
        assert!(q_save(&mut c, &passkey("b", "one", 5), 2).unwrap(), "another user's count is separate");
    }

    #[test]
    fn removing_one_or_all_touches_only_that_user() {
        let mut c = test_db();
        for (user, id) in [("a", "one"), ("a", "two"), ("b", "one")] {
            q_save(&mut c, &passkey(user, id, 1), 10).unwrap();
        }
        q_delete(&mut c, &uid("a"), "one").unwrap();
        assert_eq!(q_find_by_user(&mut c, &uid("a")).unwrap().len(), 1);
        q_delete_all(&mut c, &uid("a")).unwrap();
        assert!(q_find_by_user(&mut c, &uid("a")).unwrap().is_empty());
        assert_eq!(q_find_by_user(&mut c, &uid("b")).unwrap().len(), 1);
    }
}
