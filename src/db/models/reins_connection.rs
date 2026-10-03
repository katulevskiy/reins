use diesel::prelude::*;
use rewarden_proto::{device::ConnectionInfo, ids::ConnectionId};

use crate::{
    api::EmptyResult,
    db::{
        DbConn, DbConnInner,
        schema::{rewarden_connections, rewarden_refresh_tokens},
    },
    error::MapResult,
    util::get_uuid,
};

use super::UserId;

/// `last_used_at` is written at most once per this many seconds.
const TOUCH_INTERVAL_SECS: i64 = 60;

/// An AI client authorized by a user (one per completed OAuth pairing).
#[derive(Clone, Debug, Identifiable, Queryable, Insertable)]
#[diesel(table_name = rewarden_connections)]
#[diesel(primary_key(uuid))]
pub struct RewardenConnection {
    pub uuid: String,
    pub user_uuid: UserId,
    pub client_id: String,
    pub client_name: String,
    pub client_host: String,
    pub label: String,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
}

/// An MCP refresh token, stored only as its SHA-256 hex digest.
#[derive(Clone, Debug, Identifiable, Queryable, Insertable)]
#[diesel(table_name = rewarden_refresh_tokens)]
#[diesel(primary_key(token_hash))]
pub struct RewardenRefreshToken {
    pub token_hash: String,
    pub connection_uuid: String,
    /// Unix seconds; the token is dead at `now >= expires_at`.
    pub expires_at: i64,
}

impl RewardenConnection {
    pub fn new(
        user_uuid: UserId,
        client_id: String,
        client_name: String,
        client_host: String,
        label: String,
        now: i64,
    ) -> Self {
        Self {
            uuid: get_uuid(),
            user_uuid,
            client_id,
            client_name,
            client_host,
            label,
            created_at: now,
            last_used_at: None,
        }
    }

    pub fn to_info(&self) -> ConnectionInfo {
        ConnectionInfo {
            id: ConnectionId(self.uuid.clone()),
            label: self.label.clone(),
            client_name: self.client_name.clone(),
            client_host: self.client_host.clone(),
            created_at: self.created_at,
            last_used_at: self.last_used_at,
        }
    }

    pub async fn save(&self, conn: &DbConn) -> EmptyResult {
        conn.run(move |c| q_save(c, self)).await.map_res("Error saving Rewarden connection")
    }

    pub async fn find_by_uuid_and_user(uuid: &str, user_uuid: &UserId, conn: &DbConn) -> Option<Self> {
        conn.run(move |c| q_find_by_uuid_and_user(c, uuid, user_uuid)).await
    }

    /// Lookup by id alone, for tokens that identify the connection but not the user.
    pub async fn find_by_uuid(uuid: &str, conn: &DbConn) -> Option<Self> {
        conn.run(move |c| q_find_by_uuid(c, uuid)).await
    }

    pub async fn find_by_user(user_uuid: &UserId, conn: &DbConn) -> Vec<Self> {
        conn.run(move |c| q_find_by_user(c, user_uuid)).await
    }

    pub async fn touch(uuid: &str, now: i64, conn: &DbConn) -> EmptyResult {
        conn.run(move |c| q_touch(c, uuid, now)).await.map_res("Error updating Rewarden connection")
    }

    /// Deletes the connection and its refresh tokens (SQLite does not enforce the FK cascade).
    pub async fn delete(&self, conn: &DbConn) -> EmptyResult {
        conn.run(move |c| q_delete(c, &self.uuid)).await.map_res("Error deleting Rewarden connection")
    }
}

impl RewardenRefreshToken {
    pub async fn save(&self, conn: &DbConn) -> EmptyResult {
        conn.run(move |c| q_save_token(c, self)).await.map_res("Error saving Rewarden refresh token")
    }

    /// Consumes the token: deletes it and returns it only if it existed and was still valid.
    pub async fn take(token_hash: &str, now: i64, conn: &DbConn) -> Option<Self> {
        conn.run(move |c| q_take_token(c, token_hash, now)).await
    }

    pub async fn delete_expired(now: i64, conn: &DbConn) -> EmptyResult {
        conn.run(move |c| q_delete_expired_tokens(c, now)).await.map_res("Error purging Rewarden refresh tokens")
    }
}

fn q_save(c: &mut DbConnInner, row: &RewardenConnection) -> QueryResult<()> {
    diesel::insert_into(rewarden_connections::table).values(row).execute(c).map(|_| ())
}

fn q_find_by_uuid_and_user(c: &mut DbConnInner, uuid: &str, user_uuid: &UserId) -> Option<RewardenConnection> {
    rewarden_connections::table
        .filter(rewarden_connections::uuid.eq(uuid))
        .filter(rewarden_connections::user_uuid.eq(user_uuid))
        .first::<RewardenConnection>(c)
        .ok()
}

fn q_find_by_uuid(c: &mut DbConnInner, uuid: &str) -> Option<RewardenConnection> {
    rewarden_connections::table.filter(rewarden_connections::uuid.eq(uuid)).first::<RewardenConnection>(c).ok()
}

fn q_find_by_user(c: &mut DbConnInner, user_uuid: &UserId) -> Vec<RewardenConnection> {
    rewarden_connections::table
        .filter(rewarden_connections::user_uuid.eq(user_uuid))
        .order((rewarden_connections::created_at.asc(), rewarden_connections::uuid.asc()))
        .load::<RewardenConnection>(c)
        .unwrap_or_default()
}

fn q_touch(c: &mut DbConnInner, uuid: &str, now: i64) -> QueryResult<()> {
    diesel::update(
        rewarden_connections::table.filter(rewarden_connections::uuid.eq(uuid)).filter(
            rewarden_connections::last_used_at
                .is_null()
                .or(rewarden_connections::last_used_at.lt(now - TOUCH_INTERVAL_SECS)),
        ),
    )
    .set(rewarden_connections::last_used_at.eq(Some(now)))
    .execute(c)
    .map(|_| ())
}

fn q_delete(c: &mut DbConnInner, uuid: &str) -> QueryResult<()> {
    c.transaction(|c| {
        diesel::delete(rewarden_refresh_tokens::table.filter(rewarden_refresh_tokens::connection_uuid.eq(uuid)))
            .execute(c)?;
        diesel::delete(rewarden_connections::table.filter(rewarden_connections::uuid.eq(uuid))).execute(c)?;
        Ok(())
    })
}

fn q_save_token(c: &mut DbConnInner, row: &RewardenRefreshToken) -> QueryResult<()> {
    diesel::insert_into(rewarden_refresh_tokens::table).values(row).execute(c).map(|_| ())
}

fn q_take_token(c: &mut DbConnInner, token_hash: &str, now: i64) -> Option<RewardenRefreshToken> {
    let row = rewarden_refresh_tokens::table
        .filter(rewarden_refresh_tokens::token_hash.eq(token_hash))
        .first::<RewardenRefreshToken>(c)
        .ok()?;
    // Only the caller whose DELETE removed the row may use it (concurrent refreshes race here).
    let deleted =
        diesel::delete(rewarden_refresh_tokens::table.filter(rewarden_refresh_tokens::token_hash.eq(token_hash)))
            .execute(c)
            .ok()?;
    (deleted == 1 && now < row.expires_at).then_some(row)
}

fn q_delete_expired_tokens(c: &mut DbConnInner, now: i64) -> QueryResult<()> {
    diesel::delete(rewarden_refresh_tokens::table.filter(rewarden_refresh_tokens::expires_at.le(now)))
        .execute(c)
        .map(|_| ())
}

#[cfg(all(test, sqlite))]
mod tests {
    use super::super::rewarden_device::test_db;
    use super::*;

    fn conn_for(user: &str, now: i64) -> RewardenConnection {
        RewardenConnection::new(
            UserId::from(user.to_owned()),
            "https://chatgpt.com/oauth/client.json".to_owned(),
            "ChatGPT".to_owned(),
            "chatgpt.com".to_owned(),
            "My ChatGPT".to_owned(),
            now,
        )
    }

    fn token(hash: &str, conn: &RewardenConnection, expires_at: i64) -> RewardenRefreshToken {
        RewardenRefreshToken {
            token_hash: hash.to_owned(),
            connection_uuid: conn.uuid.clone(),
            expires_at,
        }
    }

    #[test]
    fn connections_are_scoped_to_their_user() {
        let mut c = test_db();
        let a = conn_for("u1", 10);
        let b = conn_for("u1", 20);
        q_save(&mut c, &a).unwrap();
        q_save(&mut c, &b).unwrap();
        q_save(&mut c, &conn_for("u2", 5)).unwrap();
        let mine: Vec<String> =
            q_find_by_user(&mut c, &UserId::from("u1".to_owned())).into_iter().map(|x| x.uuid).collect();
        assert_eq!(mine, vec![a.uuid.clone(), b.uuid.clone()]);
        assert!(q_find_by_uuid_and_user(&mut c, &a.uuid, &UserId::from("u1".to_owned())).is_some());
        assert!(q_find_by_uuid_and_user(&mut c, &a.uuid, &UserId::from("u2".to_owned())).is_none());
    }

    #[test]
    fn find_by_uuid_ignores_the_user() {
        let mut c = test_db();
        let a = conn_for("u1", 10);
        q_save(&mut c, &a).unwrap();
        assert_eq!(q_find_by_uuid(&mut c, &a.uuid).unwrap().user_uuid, UserId::from("u1".to_owned()));
        assert!(q_find_by_uuid(&mut c, "nope").is_none());
    }

    #[test]
    fn to_info_maps_fields() {
        let a = conn_for("u1", 10);
        let info = a.to_info();
        assert_eq!(info.id.0, a.uuid);
        assert_eq!(info.label, "My ChatGPT");
        assert_eq!(info.client_host, "chatgpt.com");
        assert_eq!((info.created_at, info.last_used_at), (10, None));
    }

    #[test]
    fn touch_is_throttled_to_once_a_minute() {
        let mut c = test_db();
        let a = conn_for("u1", 10);
        q_save(&mut c, &a).unwrap();
        let user = UserId::from("u1".to_owned());
        q_touch(&mut c, &a.uuid, 100).unwrap();
        assert_eq!(q_find_by_uuid_and_user(&mut c, &a.uuid, &user).unwrap().last_used_at, Some(100));
        q_touch(&mut c, &a.uuid, 159).unwrap();
        assert_eq!(q_find_by_uuid_and_user(&mut c, &a.uuid, &user).unwrap().last_used_at, Some(100));
        q_touch(&mut c, &a.uuid, 161).unwrap();
        assert_eq!(q_find_by_uuid_and_user(&mut c, &a.uuid, &user).unwrap().last_used_at, Some(161));
    }

    #[test]
    fn refresh_tokens_are_single_use_and_expire() {
        let mut c = test_db();
        let a = conn_for("u1", 10);
        q_save(&mut c, &a).unwrap();
        q_save_token(&mut c, &token("h1", &a, 1000)).unwrap();
        q_save_token(&mut c, &token("h2", &a, 50)).unwrap();
        assert_eq!(q_take_token(&mut c, "h1", 999).unwrap().connection_uuid, a.uuid);
        assert!(q_take_token(&mut c, "h1", 999).is_none(), "second use must fail");
        assert!(q_take_token(&mut c, "h2", 50).is_none(), "expired at exactly expires_at");
        assert!(q_take_token(&mut c, "h2", 10).is_none(), "an expired take still consumed the row");
        assert!(q_take_token(&mut c, "unknown", 0).is_none());
    }

    #[test]
    fn deleting_a_connection_deletes_its_refresh_tokens() {
        let mut c = test_db();
        let a = conn_for("u1", 10);
        let b = conn_for("u1", 11);
        q_save(&mut c, &a).unwrap();
        q_save(&mut c, &b).unwrap();
        q_save_token(&mut c, &token("ha", &a, 1000)).unwrap();
        q_save_token(&mut c, &token("hb", &b, 1000)).unwrap();
        q_delete(&mut c, &a.uuid).unwrap();
        assert!(q_find_by_uuid_and_user(&mut c, &a.uuid, &UserId::from("u1".to_owned())).is_none());
        assert!(q_take_token(&mut c, "ha", 0).is_none());
        assert!(q_take_token(&mut c, "hb", 0).is_some());
    }

    #[test]
    fn delete_expired_keeps_live_tokens() {
        let mut c = test_db();
        let a = conn_for("u1", 10);
        q_save(&mut c, &a).unwrap();
        q_save_token(&mut c, &token("old", &a, 100)).unwrap();
        q_save_token(&mut c, &token("new", &a, 300)).unwrap();
        q_delete_expired_tokens(&mut c, 200).unwrap();
        assert!(q_take_token(&mut c, "old", 0).is_none());
        assert!(q_take_token(&mut c, "new", 0).is_some());
    }
}
