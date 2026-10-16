use diesel::prelude::*;

use crate::{
    api::EmptyResult,
    auth::reins::hash_token,
    db::{
        DbConn, DbConnInner,
        schema::{reins_clients, reins_connections, reins_jwt_clients, reins_refresh_tokens},
    },
    error::MapResult,
    util::get_uuid,
};

/// Clients (metadata-document URLs) ever seen authenticating with `private_key_jwt`: in their metadata document
/// (recorded here, by SHA-256) or in a stored grant. Reins never accepts such a client without a valid assertion
/// again, whatever an older grant or a later document says.
pub struct ReinsJwtClient;

impl ReinsJwtClient {
    /// Whether `client_id` has been seen with `private_key_jwt`.
    pub async fn seen(client_id: &str, conn: &DbConn) -> Result<bool, crate::Error> {
        conn.run(move |c| q_seen(c, client_id)).await.map_res("Error reading Reins client authentication")
    }

    /// Records that `client_id` has been seen with `private_key_jwt` (idempotent).
    pub async fn mark(client_id: &str, now: i64, conn: &DbConn) -> EmptyResult {
        conn.run(move |c| q_mark(c, client_id, now)).await.map_res("Error saving Reins client authentication")
    }
}

fn q_seen(c: &mut DbConnInner, client_id: &str) -> QueryResult<bool> {
    let marked = reins_jwt_clients::table
        .filter(reins_jwt_clients::client_hash.eq(hash_token(client_id)))
        .count()
        .get_result::<i64>(c)?;
    if marked > 0 {
        return Ok(true);
    }
    let connections =
        reins_connections::table.filter(reins_connections::client_id.eq(client_id)).select(reins_connections::uuid);
    let keyed_grants = reins_refresh_tokens::table
        .filter(reins_refresh_tokens::client_auth.ne("none"))
        .filter(reins_refresh_tokens::connection_uuid.eq_any(connections))
        .count()
        .get_result::<i64>(c)?;
    Ok(keyed_grants > 0)
}

fn q_mark(c: &mut DbConnInner, client_id: &str, now: i64) -> QueryResult<()> {
    let hash = hash_token(client_id);
    let known = |c: &mut DbConnInner| {
        reins_jwt_clients::table.filter(reins_jwt_clients::client_hash.eq(&hash)).count().get_result::<i64>(c)
    };
    if known(c)? > 0 {
        return Ok(());
    }
    let inserted = diesel::insert_into(reins_jwt_clients::table)
        .values((reins_jwt_clients::client_hash.eq(&hash), reins_jwt_clients::first_seen_at.eq(now)))
        .execute(c);
    match inserted {
        Ok(_) => Ok(()),
        // Two requests raced to mark the same client: the other one's row is just as good.
        Err(e) => match known(c) {
            Ok(n) if n > 0 => Ok(()),
            _ => Err(e),
        },
    }
}

/// An MCP client registered through Dynamic Client Registration (public client).
#[derive(Clone, Debug, Identifiable, Queryable, Insertable)]
#[diesel(table_name = reins_clients)]
#[diesel(primary_key(client_id))]
pub struct ReinsClient {
    pub client_id: String,
    pub client_name: String,
    /// JSON array of exact redirect URIs.
    pub redirect_uris: String,
    /// Unix seconds.
    pub created_at: i64,
}

impl ReinsClient {
    pub fn new(client_name: String, redirect_uris: &[String], now: i64) -> Self {
        Self {
            client_id: get_uuid(),
            client_name,
            redirect_uris: serde_json::to_string(redirect_uris).expect("a list of strings always serializes"),
            created_at: now,
        }
    }

    /// Registered redirect URIs; a corrupt column yields an empty list (matches nothing).
    pub fn redirect_uri_list(&self) -> Vec<String> {
        serde_json::from_str(&self.redirect_uris).unwrap_or_default()
    }

    pub async fn save(&self, conn: &DbConn) -> EmptyResult {
        conn.run(move |c| q_save(c, self)).await.map_res("Error saving Reins client")
    }

    pub async fn find(client_id: &str, conn: &DbConn) -> Option<Self> {
        conn.run(move |c| q_find(c, client_id)).await
    }
}

fn q_save(c: &mut DbConnInner, row: &ReinsClient) -> QueryResult<()> {
    diesel::insert_into(reins_clients::table).values(row).execute(c).map(|_| ())
}

fn q_find(c: &mut DbConnInner, client_id: &str) -> Option<ReinsClient> {
    reins_clients::table.filter(reins_clients::client_id.eq(client_id)).first::<ReinsClient>(c).ok()
}

#[cfg(all(test, sqlite))]
mod tests {
    use super::super::reins_device::test_db;
    use super::*;

    #[test]
    fn client_round_trips_redirect_uris() {
        let mut c = test_db();
        let uris = vec!["https://claude.ai/api/mcp/auth_callback".to_owned(), "http://localhost/callback".to_owned()];
        let client = ReinsClient::new("Claude".to_owned(), &uris, 7);
        assert_eq!(client.client_id.len(), 36);
        q_save(&mut c, &client).unwrap();
        let found = q_find(&mut c, &client.client_id).unwrap();
        assert_eq!(found.client_name, "Claude");
        assert_eq!(found.redirect_uri_list(), uris);
        assert!(q_find(&mut c, "nope").is_none());
    }

    #[test]
    fn a_client_is_seen_with_keys_once_marked_or_once_a_keyed_grant_exists() {
        use super::super::{ReinsConnection, ReinsRefreshToken, UserId};
        let mut c = test_db();
        let chatgpt = "https://chatgpt.com/oauth/client.json";
        assert!(!q_seen(&mut c, chatgpt).unwrap());
        q_mark(&mut c, chatgpt, 1).unwrap();
        q_mark(&mut c, chatgpt, 2).unwrap();
        assert!(q_seen(&mut c, chatgpt).unwrap(), "marked (twice is fine)");
        assert!(!q_seen(&mut c, "https://other.example/client.json").unwrap());
        // A stored grant that says private_key_jwt marks its client too; one that says none does not.
        let other = "https://other.example/client.json";
        let connection = ReinsConnection::new(
            UserId::from("u1".to_owned()),
            other.to_owned(),
            "Other".to_owned(),
            "other.example".to_owned(),
            "Other".to_owned(),
            1,
        );
        diesel::insert_into(reins_connections::table).values(&connection).execute(&mut c).unwrap();
        let grant = |hash: &str, auth: &str| ReinsRefreshToken {
            token_hash: hash.to_owned(),
            connection_uuid: connection.uuid.clone(),
            expires_at: 99,
            client_auth: auth.to_owned(),
        };
        diesel::insert_into(reins_refresh_tokens::table).values(&grant("h1", "none")).execute(&mut c).unwrap();
        assert!(!q_seen(&mut c, other).unwrap());
        diesel::insert_into(reins_refresh_tokens::table)
            .values(&grant("h2", "private_key_jwt"))
            .execute(&mut c)
            .unwrap();
        assert!(q_seen(&mut c, other).unwrap());
    }

    #[test]
    fn corrupt_redirect_uris_yield_empty_list() {
        let client = ReinsClient {
            client_id: "x".to_owned(),
            client_name: "x".to_owned(),
            redirect_uris: "not json".to_owned(),
            created_at: 0,
        };
        assert!(client.redirect_uri_list().is_empty());
    }
}
