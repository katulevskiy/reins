use diesel::prelude::*;

use crate::{
    api::EmptyResult,
    db::{
        DbConn, DbConnInner,
        schema::{
            devices, rewarden_connections, rewarden_devices, rewarden_refresh_tokens, rewarden_settings,
            rewarden_sso_sessions, sso_users,
        },
    },
    error::MapResult,
};

use super::{DeviceId, UserId};

/// One named value of server state that outlives a restart (the WorkOS events cursor).
#[derive(Clone, Debug, Identifiable, Queryable, Insertable)]
#[diesel(table_name = rewarden_settings)]
#[diesel(primary_key(name))]
pub struct RewardenSetting {
    pub name: String,
    pub value: String,
}

impl RewardenSetting {
    pub async fn get(name: &str, conn: &DbConn) -> Option<String> {
        conn.run(move |c| q_get_setting(c, name)).await
    }

    pub async fn set(name: &str, value: &str, conn: &DbConn) -> EmptyResult {
        conn.run(move |c| q_set_setting(c, name, value)).await.map_res("Error saving Rewarden setting")
    }
}

/// The SSO provider session a device signed in with (WorkOS: the access token's `sid`).
#[derive(Clone, Debug, Identifiable, Queryable, Insertable)]
#[diesel(table_name = rewarden_sso_sessions)]
#[diesel(primary_key(session_id))]
pub struct RewardenSsoSession {
    pub session_id: String,
    pub user_uuid: UserId,
    pub device_uuid: DeviceId,
    pub created_at: i64,
}

impl RewardenSsoSession {
    /// Remembers the session of a sign-in; the same session signing in again (a second device) keeps the first row.
    pub async fn save(&self, conn: &DbConn) -> EmptyResult {
        conn.run(move |c| q_save_session(c, self)).await.map_res("Error saving SSO session")
    }

    /// Forgets `session_id` and returns what it was for.
    pub async fn take(session_id: &str, conn: &DbConn) -> Option<Self> {
        conn.run(move |c| q_take_session(c, session_id)).await
    }
}

/// Signs one device out: its Vaultwarden device row goes, so its refresh token and access tokens stop working at once
/// (the next sign-in makes the row again).
pub async fn sign_out_device(user_uuid: &UserId, device_uuid: &DeviceId, conn: &DbConn) -> EmptyResult {
    conn.run(move |c| q_sign_out_device(c, user_uuid, device_uuid)).await.map_res("Error signing the device out")
}

/// Deletes what Reins keeps for a user beyond Vaultwarden's own tables (which `User::delete` clears): the approval
/// device, the AI and desktop connections with their refresh tokens, SSO sessions and the SSO identity. SQLite does
/// not enforce the foreign keys' cascades, so this does not rely on them.
pub async fn delete_reins_data(user_uuid: &UserId, conn: &DbConn) -> EmptyResult {
    conn.run(move |c| q_delete_reins_data(c, user_uuid)).await.map_res("Error deleting the user's Reins data")
}

fn q_get_setting(c: &mut DbConnInner, name: &str) -> Option<String> {
    rewarden_settings::table
        .filter(rewarden_settings::name.eq(name))
        .select(rewarden_settings::value)
        .first::<String>(c)
        .ok()
}

fn q_set_setting(c: &mut DbConnInner, name: &str, value: &str) -> QueryResult<()> {
    c.transaction(|c| {
        diesel::delete(rewarden_settings::table.filter(rewarden_settings::name.eq(name))).execute(c)?;
        diesel::insert_into(rewarden_settings::table)
            .values(RewardenSetting {
                name: name.to_owned(),
                value: value.to_owned(),
            })
            .execute(c)
            .map(|_| ())
    })
}

fn q_save_session(c: &mut DbConnInner, row: &RewardenSsoSession) -> QueryResult<()> {
    c.transaction(|c| {
        let known = rewarden_sso_sessions::table
            .filter(rewarden_sso_sessions::session_id.eq(&row.session_id))
            .count()
            .get_result::<i64>(c)?;
        if known == 0 {
            diesel::insert_into(rewarden_sso_sessions::table).values(row).execute(c)?;
        }
        Ok(())
    })
}

fn q_take_session(c: &mut DbConnInner, session_id: &str) -> Option<RewardenSsoSession> {
    let row = rewarden_sso_sessions::table
        .filter(rewarden_sso_sessions::session_id.eq(session_id))
        .first::<RewardenSsoSession>(c)
        .ok()?;
    diesel::delete(rewarden_sso_sessions::table.filter(rewarden_sso_sessions::session_id.eq(session_id)))
        .execute(c)
        .ok()?;
    Some(row)
}

fn q_sign_out_device(c: &mut DbConnInner, user_uuid: &UserId, device_uuid: &DeviceId) -> QueryResult<()> {
    diesel::delete(devices::table.filter(devices::uuid.eq(device_uuid)).filter(devices::user_uuid.eq(user_uuid)))
        .execute(c)
        .map(|_| ())
}

fn q_delete_reins_data(c: &mut DbConnInner, user_uuid: &UserId) -> QueryResult<()> {
    c.transaction(|c| {
        let connections = rewarden_connections::table
            .filter(rewarden_connections::user_uuid.eq(user_uuid))
            .select(rewarden_connections::uuid)
            .load::<String>(c)?;
        diesel::delete(
            rewarden_refresh_tokens::table.filter(rewarden_refresh_tokens::connection_uuid.eq_any(&connections)),
        )
        .execute(c)?;
        diesel::delete(rewarden_connections::table.filter(rewarden_connections::user_uuid.eq(user_uuid))).execute(c)?;
        diesel::delete(rewarden_devices::table.filter(rewarden_devices::user_uuid.eq(user_uuid))).execute(c)?;
        diesel::delete(rewarden_sso_sessions::table.filter(rewarden_sso_sessions::user_uuid.eq(user_uuid)))
            .execute(c)?;
        diesel::delete(sso_users::table.filter(sso_users::user_uuid.eq(user_uuid))).execute(c)?;
        Ok(())
    })
}

#[cfg(all(test, sqlite))]
mod tests {
    use super::super::rewarden_device::test_db;
    use super::*;

    fn uid(s: &str) -> UserId {
        UserId::from(s.to_owned())
    }

    #[test]
    fn settings_are_replaced() {
        let mut c = test_db();
        assert_eq!(q_get_setting(&mut c, "workos.cursor"), None);
        q_set_setting(&mut c, "workos.cursor", "event_1").unwrap();
        q_set_setting(&mut c, "workos.cursor", "event_2").unwrap();
        assert_eq!(q_get_setting(&mut c, "workos.cursor").as_deref(), Some("event_2"));
    }

    #[test]
    fn a_session_is_taken_once_and_its_first_device_kept() {
        let mut c = test_db();
        let row = |dev: &str| RewardenSsoSession {
            session_id: "session_1".to_owned(),
            user_uuid: uid("u1"),
            device_uuid: DeviceId::from(dev.to_owned()),
            created_at: 1,
        };
        q_save_session(&mut c, &row("d1")).unwrap();
        q_save_session(&mut c, &row("d2")).unwrap();
        let taken = q_take_session(&mut c, "session_1").unwrap();
        assert_eq!(taken.device_uuid, DeviceId::from("d1".to_owned()));
        assert!(q_take_session(&mut c, "session_1").is_none());
    }

    #[test]
    fn deleting_reins_data_keeps_other_users() {
        use crate::db::models::{RewardenConnection, RewardenRefreshToken};
        let mut c = test_db();
        for user in ["u1", "u2"] {
            let conn = RewardenConnection::new(
                uid(user),
                "client".to_owned(),
                "Claude".to_owned(),
                "claude.ai".to_owned(),
                "Claude".to_owned(),
                1,
            );
            diesel::insert_into(rewarden_connections::table).values(&conn).execute(&mut c).unwrap();
            let token = RewardenRefreshToken {
                token_hash: format!("hash-{user}"),
                connection_uuid: conn.uuid.clone(),
                expires_at: 99,
            };
            diesel::insert_into(rewarden_refresh_tokens::table).values(&token).execute(&mut c).unwrap();
            q_save_session(
                &mut c,
                &RewardenSsoSession {
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
            rewarden_refresh_tokens::table.select(rewarden_refresh_tokens::token_hash).load(&mut c).unwrap();
        assert_eq!(tokens, ["hash-u2"]);
        let users: Vec<String> =
            rewarden_connections::table.select(rewarden_connections::user_uuid).load(&mut c).unwrap();
        assert_eq!(users, ["u2"]);
        assert!(q_take_session(&mut c, "session-u1").is_none());
        assert!(q_take_session(&mut c, "session-u2").is_some());
    }
}
