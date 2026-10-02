use diesel::prelude::*;

use crate::{
    api::EmptyResult,
    db::{DbConn, DbConnInner, schema::rewarden_devices},
    error::MapResult,
};

use super::{DeviceId, UserId};

/// The user's approval device: exactly one per user (spec §4.1, contracts A1).
#[derive(Clone, Debug, Identifiable, Queryable, Insertable)]
#[diesel(table_name = rewarden_devices)]
#[diesel(primary_key(user_uuid))]
pub struct RewardenDevice {
    pub user_uuid: UserId,
    pub device_uuid: DeviceId,
    pub fcm_token: Option<String>,
    /// Unix seconds.
    pub updated_at: i64,
    /// SHA-256 (hex) of the device's device key (`rewarden_proto::device::DEVICE_KEY_HEADER`); `None` for a row
    /// from before device keys, or a client that sent none.
    pub key_hash: Option<String>,
}

impl RewardenDevice {
    pub async fn find_by_user(user_uuid: &UserId, conn: &DbConn) -> Option<Self> {
        conn.run(move |c| q_find_by_user(c, user_uuid)).await
    }

    /// Makes `self` the user's approval device and returns the device it replaced.
    pub async fn replace(&self, conn: &DbConn) -> Result<Option<Self>, crate::Error> {
        conn.run(move |c| q_replace(c, self)).await.map_res("Error saving Rewarden device")
    }

    /// Forgets `fcm_token` if it is still the stored token (FCM reported it unregistered).
    pub async fn clear_fcm_token(user_uuid: &UserId, fcm_token: &str, conn: &DbConn) -> EmptyResult {
        conn.run(move |c| q_clear_fcm_token(c, user_uuid, fcm_token)).await.map_res("Error clearing Rewarden FCM token")
    }
}

fn q_find_by_user(c: &mut DbConnInner, user_uuid: &UserId) -> Option<RewardenDevice> {
    rewarden_devices::table.filter(rewarden_devices::user_uuid.eq(user_uuid)).first::<RewardenDevice>(c).ok()
}

fn q_replace(c: &mut DbConnInner, row: &RewardenDevice) -> QueryResult<Option<RewardenDevice>> {
    c.transaction(|c| {
        let previous = rewarden_devices::table
            .filter(rewarden_devices::user_uuid.eq(&row.user_uuid))
            .first::<RewardenDevice>(c)
            .optional()?;
        diesel::delete(rewarden_devices::table.filter(rewarden_devices::user_uuid.eq(&row.user_uuid))).execute(c)?;
        diesel::insert_into(rewarden_devices::table).values(row).execute(c)?;
        Ok(previous)
    })
}

fn q_clear_fcm_token(c: &mut DbConnInner, user_uuid: &UserId, fcm_token: &str) -> QueryResult<()> {
    diesel::update(
        rewarden_devices::table
            .filter(rewarden_devices::user_uuid.eq(user_uuid))
            .filter(rewarden_devices::fcm_token.eq(fcm_token)),
    )
    .set(rewarden_devices::fcm_token.eq(None::<String>))
    .execute(c)
    .map(|_| ())
}

#[cfg(all(test, sqlite))]
pub(super) fn test_db() -> DbConnInner {
    use diesel::Connection;
    use diesel_migrations::MigrationHarness;
    let mut c = SqliteConnection::establish(":memory:").unwrap();
    // The model tests use bare user ids; the users table is not populated here.
    diesel::sql_query("PRAGMA foreign_keys = OFF").execute(&mut c).unwrap();
    c.run_pending_migrations(crate::db::sqlite_migrations::MIGRATIONS).unwrap();
    DbConnInner::Sqlite(c)
}

#[cfg(all(test, sqlite))]
mod tests {
    use super::*;

    fn device(user: &str, dev: &str, token: Option<&str>) -> RewardenDevice {
        RewardenDevice {
            user_uuid: UserId::from(user.to_owned()),
            device_uuid: DeviceId::from(dev.to_owned()),
            fcm_token: token.map(str::to_owned),
            updated_at: 1,
            key_hash: Some(format!("key-of-{dev}")),
        }
    }

    #[test]
    fn replace_returns_previous_device() {
        let mut c = test_db();
        assert!(q_find_by_user(&mut c, &UserId::from("u1".to_owned())).is_none());
        assert!(q_replace(&mut c, &device("u1", "d1", Some("t1"))).unwrap().is_none());
        let prev = q_replace(&mut c, &device("u1", "d2", None)).unwrap().unwrap();
        assert_eq!(prev.device_uuid, DeviceId::from("d1".to_owned()));
        assert_eq!(prev.fcm_token.as_deref(), Some("t1"));
        let now = q_find_by_user(&mut c, &UserId::from("u1".to_owned())).unwrap();
        assert_eq!(now.device_uuid, DeviceId::from("d2".to_owned()));
        assert_eq!(now.fcm_token, None);
        assert_eq!(now.key_hash.as_deref(), Some("key-of-d2"));
        assert_eq!(prev.key_hash.as_deref(), Some("key-of-d1"));
    }

    #[test]
    fn devices_are_per_user_and_token_can_be_cleared() {
        let mut c = test_db();
        q_replace(&mut c, &device("u1", "d1", Some("t1"))).unwrap();
        q_replace(&mut c, &device("u2", "d9", Some("t9"))).unwrap();
        q_clear_fcm_token(&mut c, &UserId::from("u1".to_owned()), "stale").unwrap();
        assert_eq!(q_find_by_user(&mut c, &UserId::from("u1".to_owned())).unwrap().fcm_token.as_deref(), Some("t1"));
        q_clear_fcm_token(&mut c, &UserId::from("u1".to_owned()), "t1").unwrap();
        assert_eq!(q_find_by_user(&mut c, &UserId::from("u1".to_owned())).unwrap().fcm_token, None);
        assert_eq!(q_find_by_user(&mut c, &UserId::from("u2".to_owned())).unwrap().fcm_token.as_deref(), Some("t9"));
    }
}
