use diesel::prelude::*;

use crate::{
    api::EmptyResult,
    db::{DbConn, DbConnInner, schema::rewarden_clients},
    error::MapResult,
    util::get_uuid,
};

/// An MCP client registered through Dynamic Client Registration (public client).
#[derive(Clone, Debug, Identifiable, Queryable, Insertable)]
#[diesel(table_name = rewarden_clients)]
#[diesel(primary_key(client_id))]
pub struct RewardenClient {
    pub client_id: String,
    pub client_name: String,
    /// JSON array of exact redirect URIs.
    pub redirect_uris: String,
    /// Unix seconds.
    pub created_at: i64,
}

impl RewardenClient {
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
        conn.run(move |c| q_save(c, self)).await.map_res("Error saving Rewarden client")
    }

    pub async fn find(client_id: &str, conn: &DbConn) -> Option<Self> {
        conn.run(move |c| q_find(c, client_id)).await
    }
}

fn q_save(c: &mut DbConnInner, row: &RewardenClient) -> QueryResult<()> {
    diesel::insert_into(rewarden_clients::table).values(row).execute(c).map(|_| ())
}

fn q_find(c: &mut DbConnInner, client_id: &str) -> Option<RewardenClient> {
    rewarden_clients::table.filter(rewarden_clients::client_id.eq(client_id)).first::<RewardenClient>(c).ok()
}

#[cfg(all(test, sqlite))]
mod tests {
    use super::super::rewarden_device::test_db;
    use super::*;

    #[test]
    fn client_round_trips_redirect_uris() {
        let mut c = test_db();
        let uris = vec!["https://claude.ai/api/mcp/auth_callback".to_owned(), "http://localhost/callback".to_owned()];
        let client = RewardenClient::new("Claude".to_owned(), &uris, 7);
        assert_eq!(client.client_id.len(), 36);
        q_save(&mut c, &client).unwrap();
        let found = q_find(&mut c, &client.client_id).unwrap();
        assert_eq!(found.client_name, "Claude");
        assert_eq!(found.redirect_uri_list(), uris);
        assert!(q_find(&mut c, "nope").is_none());
    }

    #[test]
    fn corrupt_redirect_uris_yield_empty_list() {
        let client = RewardenClient {
            client_id: "x".to_owned(),
            client_name: "x".to_owned(),
            redirect_uris: "not json".to_owned(),
            created_at: 0,
        };
        assert!(client.redirect_uri_list().is_empty());
    }
}
