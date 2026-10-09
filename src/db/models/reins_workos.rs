use diesel::prelude::*;

use crate::{
    api::EmptyResult,
    db::{
        DbConn, DbConnInner,
        schema::{
            devices, reins_connections, reins_devices, reins_refresh_tokens, reins_settings, reins_sso_sessions,
            sso_users,
        },
    },
    error::MapResult,
};

use super::{DeviceId, UserId};

/// One named value of server state that outlives a restart (the WorkOS events cursor).
#[derive(Clone, Debug, Identifiable, Queryable, Insertable)]
#[diesel(table_name = reins_settings)]
#[diesel(primary_key(name))]
pub struct ReinsSetting {
    pub name: String,
    pub value: String,
}

impl ReinsSetting {
    pub async fn get(name: &str, conn: &DbConn) -> Result<Option<String>, crate::Error> {
        conn.run(move |c| q_get_setting(c, name)).await.map_res("Error reading WorkOS cursor")
    }

    pub async fn set(name: &str, value: &str, conn: &DbConn) -> EmptyResult {
        conn.run(move |c| q_set_setting(c, name, value)).await.map_res("Error saving Reins setting")
    }

    pub async fn pending_workos_deletions(after: &str, conn: &DbConn) -> Result<Vec<Self>, crate::Error> {
        conn.run(move |c| {
            reins_settings::table
                .filter(reins_settings::name.like("workos.delete.%"))
                .filter(reins_settings::name.gt(after))
                .order(reins_settings::name.asc())
                .limit(10)
                .load::<Self>(c)
                .map_res("Error reading pending WorkOS deletions")
        })
        .await
    }

    pub async fn remove(name: &str, conn: &DbConn) -> EmptyResult {
        conn.run(move |c| {
            diesel::delete(reins_settings::table.filter(reins_settings::name.eq(name)))
                .execute(c)
                .map_res("Error deleting Reins setting")
        })
        .await
    }
}

/// The SSO provider session a device signed in with (WorkOS: the access token's `sid`).
#[derive(Clone, Debug, Identifiable, Queryable, Insertable)]
#[diesel(table_name = reins_sso_sessions)]
#[diesel(primary_key(session_id))]
pub struct ReinsSsoSession {
    pub session_id: String,
    pub user_uuid: UserId,
    pub device_uuid: DeviceId,
    pub created_at: i64,
}

impl ReinsSsoSession {
    /// Only sessions belonging to the authenticated account and this device may be signed out.
    pub async fn latest_for_device(
        user: &UserId,
        device: &DeviceId,
        conn: &DbConn,
    ) -> Result<Option<Self>, crate::Error> {
        conn.run(move |c| {
            reins_sso_sessions::table
                .filter(reins_sso_sessions::user_uuid.eq(user))
                .filter(reins_sso_sessions::device_uuid.eq(device))
                .order((reins_sso_sessions::created_at.desc(), reins_sso_sessions::session_id.desc()))
                .first::<Self>(c)
                .optional()
                .map_res("Error finding this device's SSO session")
        })
        .await
    }

    /// Remembers the session of a sign-in; the same session signing in again (a second device) keeps the first row.
    pub async fn save(&self, conn: &DbConn) -> EmptyResult {
        conn.run(move |c| q_save_session(c, self)).await.map_res("Error saving SSO session")
    }

    /// Revoke the device and forget its session together. A database failure leaves the mapping for a retry.
    pub async fn revoke(session_id: &str, conn: &DbConn) -> EmptyResult {
        conn.run(move |c| q_revoke_session(c, session_id)).await.map_res("Error revoking WorkOS session")
    }

    pub async fn revoke_device(user: &UserId, device: &DeviceId, conn: &DbConn) -> EmptyResult {
        conn.run(move |c| {
            c.transaction(|c| {
                diesel::delete(devices::table.filter(devices::user_uuid.eq(user)).filter(devices::uuid.eq(device)))
                    .execute(c)?;
                diesel::delete(
                    reins_sso_sessions::table
                        .filter(reins_sso_sessions::user_uuid.eq(user))
                        .filter(reins_sso_sessions::device_uuid.eq(device)),
                )
                .execute(c)?;
                Ok::<_, diesel::result::Error>(())
            })
            .map_res("Error signing out this device")
        })
        .await
    }
}

/// Resolve the identity even if Vaultwarden deleted the user before Reins cleanup completed.
pub async fn user_id_by_identifier(identifier: &str, conn: &DbConn) -> Result<Option<UserId>, crate::Error> {
    conn.run(move |c| {
        sso_users::table
            .filter(sso_users::identifier.eq(identifier))
            .select(sso_users::user_uuid)
            .first::<UserId>(c)
            .optional()
            .map_res("Error finding WorkOS identity")
    })
    .await
}

/// The SSO identity account `uuid` signs in with, if any (an account deleting itself deletes its WorkOS user too).
pub async fn identifier_of(uuid: &UserId, conn: &DbConn) -> Result<Option<String>, crate::Error> {
    conn.run(move |c| {
        sso_users::table
            .filter(sso_users::user_uuid.eq(uuid))
            .select(sso_users::identifier)
            .first::<String>(c)
            .optional()
            .map_res("Error finding the account's SSO identity")
    })
    .await
}

/// A missing user and a database failure must remain distinguishable while applying lifecycle events.
pub async fn user_by_id(uuid: &UserId, conn: &DbConn) -> Result<Option<super::User>, crate::Error> {
    conn.run(move |c| {
        crate::db::schema::users::table
            .filter(crate::db::schema::users::uuid.eq(uuid))
            .first::<super::User>(c)
            .optional()
            .map_res("Error reading WorkOS account")
    })
    .await
}

/// Deletes what Reins keeps for a user beyond Vaultwarden's own tables (which `User::delete` clears): the approval
/// device, the AI and desktop connections with their refresh tokens, SSO sessions, the SSO identity and the vault's
/// passkeys. SQLite does not enforce the foreign keys' cascades, so this does not rely on them.
pub async fn delete_reins_data(user_uuid: &UserId, conn: &DbConn) -> EmptyResult {
    conn.run(move |c| q_delete_reins_data(c, user_uuid)).await.map_res("Error deleting the user's Reins data")
}

/// What a vault reset (`api::reins::account_reset`) clears of the account's sign-ins: the approval device, every other
/// device (its Vaultwarden device, which holds its refresh token, and its SSO session), and the AI and desktop
/// connections with their refresh tokens (they would send their requests to whoever reset the vault). `keep`, the
/// device that asked, stays signed in.
pub async fn reset_devices(user_uuid: &UserId, keep: &DeviceId, conn: &DbConn) -> EmptyResult {
    conn.run(move |c| q_reset_devices(c, user_uuid, keep)).await.map_res("Error signing out the account's devices")
}

fn q_reset_devices(c: &mut DbConnInner, user_uuid: &UserId, keep: &DeviceId) -> QueryResult<()> {
    c.transaction(|c| {
        let connections = reins_connections::table
            .filter(reins_connections::user_uuid.eq(user_uuid))
            .select(reins_connections::uuid)
            .load::<String>(c)?;
        diesel::delete(reins_refresh_tokens::table.filter(reins_refresh_tokens::connection_uuid.eq_any(&connections)))
            .execute(c)?;
        diesel::delete(reins_connections::table.filter(reins_connections::user_uuid.eq(user_uuid))).execute(c)?;
        diesel::delete(reins_devices::table.filter(reins_devices::user_uuid.eq(user_uuid))).execute(c)?;
        diesel::delete(devices::table.filter(devices::user_uuid.eq(user_uuid)).filter(devices::uuid.ne(keep)))
            .execute(c)?;
        diesel::delete(
            reins_sso_sessions::table
                .filter(reins_sso_sessions::user_uuid.eq(user_uuid))
                .filter(reins_sso_sessions::device_uuid.ne(keep)),
        )
        .execute(c)?;
        Ok(())
    })
}

fn q_get_setting(c: &mut DbConnInner, name: &str) -> QueryResult<Option<String>> {
    reins_settings::table
        .filter(reins_settings::name.eq(name))
        .select(reins_settings::value)
        .first::<String>(c)
        .optional()
}

fn q_set_setting(c: &mut DbConnInner, name: &str, value: &str) -> QueryResult<()> {
    c.transaction(|c| {
        diesel::delete(reins_settings::table.filter(reins_settings::name.eq(name))).execute(c)?;
        diesel::insert_into(reins_settings::table)
            .values(ReinsSetting {
                name: name.to_owned(),
                value: value.to_owned(),
            })
            .execute(c)
            .map(|_| ())
    })
}

fn q_save_session(c: &mut DbConnInner, row: &ReinsSsoSession) -> QueryResult<()> {
    c.transaction(|c| {
        let known = reins_sso_sessions::table
            .filter(reins_sso_sessions::session_id.eq(&row.session_id))
            .count()
            .get_result::<i64>(c)?;
        if known == 0 {
            diesel::insert_into(reins_sso_sessions::table).values(row).execute(c)?;
        }
        Ok(())
    })
}

fn q_revoke_session(c: &mut DbConnInner, session_id: &str) -> QueryResult<()> {
    c.transaction(|c| {
        let row = reins_sso_sessions::table
            .filter(reins_sso_sessions::session_id.eq(session_id))
            .first::<ReinsSsoSession>(c)
            .optional()?;
        if let Some(row) = row {
            diesel::delete(
                devices::table.filter(devices::uuid.eq(row.device_uuid)).filter(devices::user_uuid.eq(row.user_uuid)),
            )
            .execute(c)?;
            diesel::delete(reins_sso_sessions::table.filter(reins_sso_sessions::session_id.eq(session_id)))
                .execute(c)?;
        }
        Ok(())
    })
}

fn q_delete_reins_data(c: &mut DbConnInner, user_uuid: &UserId) -> QueryResult<()> {
    c.transaction(|c| {
        let connections = reins_connections::table
            .filter(reins_connections::user_uuid.eq(user_uuid))
            .select(reins_connections::uuid)
            .load::<String>(c)?;
        diesel::delete(reins_refresh_tokens::table.filter(reins_refresh_tokens::connection_uuid.eq_any(&connections)))
            .execute(c)?;
        diesel::delete(reins_connections::table.filter(reins_connections::user_uuid.eq(user_uuid))).execute(c)?;
        diesel::delete(reins_devices::table.filter(reins_devices::user_uuid.eq(user_uuid))).execute(c)?;
        diesel::delete(reins_sso_sessions::table.filter(reins_sso_sessions::user_uuid.eq(user_uuid))).execute(c)?;
        diesel::delete(sso_users::table.filter(sso_users::user_uuid.eq(user_uuid))).execute(c)?;
        super::reins_vault_passkey::q_delete_all(c, user_uuid)?;
        Ok(())
    })
}

#[cfg(all(test, sqlite))]
mod tests {
    use super::super::reins_device::test_db;
    use super::*;

    fn uid(s: &str) -> UserId {
        UserId::from(s.to_owned())
    }

    #[test]
    fn settings_are_replaced() {
        let mut c = test_db();
        assert_eq!(q_get_setting(&mut c, "workos.cursor").unwrap(), None);
        q_set_setting(&mut c, "workos.cursor", "event_1").unwrap();
        q_set_setting(&mut c, "workos.cursor", "event_2").unwrap();
        assert_eq!(q_get_setting(&mut c, "workos.cursor").unwrap().as_deref(), Some("event_2"));
    }

    #[test]
    fn a_session_is_revoked_idempotently_and_its_first_device_kept() {
        let mut c = test_db();
        let row = |dev: &str| ReinsSsoSession {
            session_id: "session_1".to_owned(),
            user_uuid: uid("u1"),
            device_uuid: DeviceId::from(dev.to_owned()),
            created_at: 1,
        };
        q_save_session(&mut c, &row("d1")).unwrap();
        q_save_session(&mut c, &row("d2")).unwrap();
        let saved = reins_sso_sessions::table.first::<ReinsSsoSession>(&mut c).unwrap();
        assert_eq!(saved.device_uuid, DeviceId::from("d1".to_owned()));
        q_revoke_session(&mut c, "session_1").unwrap();
        q_revoke_session(&mut c, "session_1").unwrap();
        assert_eq!(reins_sso_sessions::table.count().get_result::<i64>(&mut c).unwrap(), 0);
    }

    #[test]
    fn revocation_failure_keeps_both_device_and_session_for_retry() {
        use super::super::Device;
        let mut c = test_db();
        let device = Device::new(DeviceId::from("d1".to_owned()), uid("u1"), "Phone".to_owned(), 0);
        diesel::insert_into(devices::table).values(&device).execute(&mut c).unwrap();
        q_save_session(
            &mut c,
            &ReinsSsoSession {
                session_id: "session_1".to_owned(),
                user_uuid: uid("u1"),
                device_uuid: device.uuid.clone(),
                created_at: 1,
            },
        )
        .unwrap();
        diesel::sql_query("CREATE TRIGGER fail_session_delete BEFORE DELETE ON reins_sso_sessions BEGIN SELECT RAISE(ABORT, 'database failure'); END")
            .execute(&mut c).unwrap();
        assert!(q_revoke_session(&mut c, "session_1").is_err());
        assert_eq!(devices::table.count().get_result::<i64>(&mut c).unwrap(), 1);
        assert_eq!(reins_sso_sessions::table.count().get_result::<i64>(&mut c).unwrap(), 1);
        diesel::sql_query("DROP TRIGGER fail_session_delete").execute(&mut c).unwrap();
        q_revoke_session(&mut c, "session_1").unwrap();
        assert_eq!(devices::table.count().get_result::<i64>(&mut c).unwrap(), 0);
        assert_eq!(reins_sso_sessions::table.count().get_result::<i64>(&mut c).unwrap(), 0);
    }

    #[test]
    fn deleting_reins_data_keeps_other_users() {
        use crate::db::models::{ReinsConnection, ReinsRefreshToken};
        let mut c = test_db();
        for user in ["u1", "u2"] {
            let conn = ReinsConnection::new(
                uid(user),
                "client".to_owned(),
                "Claude".to_owned(),
                "claude.ai".to_owned(),
                "Claude".to_owned(),
                1,
            );
            diesel::insert_into(reins_connections::table).values(&conn).execute(&mut c).unwrap();
            let token = ReinsRefreshToken {
                token_hash: format!("hash-{user}"),
                connection_uuid: conn.uuid.clone(),
                expires_at: 99,
            };
            diesel::insert_into(reins_refresh_tokens::table).values(&token).execute(&mut c).unwrap();
            q_save_session(
                &mut c,
                &ReinsSsoSession {
                    session_id: format!("session-{user}"),
                    user_uuid: uid(user),
                    device_uuid: DeviceId::from("d".to_owned()),
                    created_at: 1,
                },
            )
            .unwrap();
        }
        q_delete_reins_data(&mut c, &uid("u1")).unwrap();
        let tokens: Vec<String> =
            reins_refresh_tokens::table.select(reins_refresh_tokens::token_hash).load(&mut c).unwrap();
        assert_eq!(tokens, ["hash-u2"]);
        let users: Vec<String> = reins_connections::table.select(reins_connections::user_uuid).load(&mut c).unwrap();
        assert_eq!(users, ["u2"]);
        let sessions: Vec<String> =
            reins_sso_sessions::table.select(reins_sso_sessions::session_id).load(&mut c).unwrap();
        assert_eq!(sessions, ["session-u2"]);
    }

    #[test]
    fn a_reset_signs_out_every_other_device_and_frees_the_approval_role() {
        use super::super::{Device, ReinsConnection, ReinsDevice, ReinsRefreshToken};
        let mut c = test_db();
        for user in ["u1", "u2"] {
            let conn = ReinsConnection::new(
                uid(user),
                "client".to_owned(),
                "Claude".to_owned(),
                "claude.ai".to_owned(),
                "Claude".to_owned(),
                1,
            );
            diesel::insert_into(reins_connections::table).values(&conn).execute(&mut c).unwrap();
            let token = ReinsRefreshToken {
                token_hash: format!("hash-{user}"),
                connection_uuid: conn.uuid.clone(),
                expires_at: 99,
            };
            diesel::insert_into(reins_refresh_tokens::table).values(&token).execute(&mut c).unwrap();
        }
        for (user, dev) in [("u1", "keep"), ("u1", "old"), ("u2", "other")] {
            let device = Device::new(DeviceId::from(dev.to_owned()), uid(user), "Phone".to_owned(), 0);
            diesel::insert_into(devices::table).values(&device).execute(&mut c).unwrap();
            q_save_session(
                &mut c,
                &ReinsSsoSession {
                    session_id: format!("session-{dev}"),
                    user_uuid: uid(user),
                    device_uuid: device.uuid.clone(),
                    created_at: 1,
                },
            )
            .unwrap();
            let approval = ReinsDevice {
                user_uuid: uid(user),
                device_uuid: device.uuid,
                fcm_token: None,
                updated_at: 1,
                key_hash: None,
            };
            diesel::delete(reins_devices::table.filter(reins_devices::user_uuid.eq(user))).execute(&mut c).unwrap();
            diesel::insert_into(reins_devices::table).values(&approval).execute(&mut c).unwrap();
        }
        q_reset_devices(&mut c, &uid("u1"), &DeviceId::from("keep".to_owned())).unwrap();
        let left: Vec<String> = devices::table.select(devices::uuid).order(devices::uuid).load(&mut c).unwrap();
        assert_eq!(left, ["keep", "other"]);
        let sessions: Vec<String> = reins_sso_sessions::table
            .select(reins_sso_sessions::session_id)
            .order(reins_sso_sessions::session_id)
            .load(&mut c)
            .unwrap();
        assert_eq!(sessions, ["session-keep", "session-other"]);
        let approvals: Vec<String> = reins_devices::table.select(reins_devices::user_uuid).load(&mut c).unwrap();
        assert_eq!(approvals, ["u2"]);
        let connections: Vec<String> =
            reins_connections::table.select(reins_connections::user_uuid).load(&mut c).unwrap();
        assert_eq!(connections, ["u2"]);
        let tokens: Vec<String> =
            reins_refresh_tokens::table.select(reins_refresh_tokens::token_hash).load(&mut c).unwrap();
        assert_eq!(tokens, ["hash-u2"]);
    }
}
