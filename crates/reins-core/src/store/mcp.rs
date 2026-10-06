//! The MCP servers the user added ("universal MCP"): address, how the phone signs in, the OAuth client it registered,
//! the tools last listed and which of them are heavy. Tokens are not here: they live in the secrets table.

use rusqlite::{OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::Store;
use crate::CoreError;

/// Where the tokens of a server are kept in the secrets table (account = server id).
pub const SECRET_SERVICE: &str = "mcp";
/// Where a sign-in that was started keeps its PKCE verifier and state (account = server id).
pub const SIGN_IN_SERVICE: &str = "mcp_signin";

/// One tool as the server listed it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpTool {
    pub name: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: String,
    pub input_schema: Value,
    #[serde(default)]
    pub read_only: bool,
    #[serde(default)]
    pub destructive: bool,
}

/// One added MCP server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredMcpServer {
    pub id: String,
    pub name: String,
    /// The MCP endpoint, as added.
    pub url: String,
    /// "none" | "token" | "oauth"
    pub auth: String,
    /// The OAuth client registration and endpoints (JSON), for "oauth".
    pub client: Option<String>,
    pub tools: Vec<McpTool>,
    /// Names of the tools whose results go through the server.
    pub heavy: Vec<String>,
    /// "ok" | "needs_sign_in" | "error"
    pub status: String,
    pub error: Option<String>,
    /// When the tools were last listed; `None` until the first successful connection.
    pub listed_at: Option<i64>,
    pub added_at: i64,
}

const COLUMNS: &str = "id, name, url, auth, client, tools, heavy, status, error, listed_at, added_at";

fn from_row(r: &Row<'_>) -> rusqlite::Result<StoredMcpServer> {
    let tools: String = r.get(5)?;
    let heavy: String = r.get(6)?;
    Ok(StoredMcpServer {
        id: r.get(0)?,
        name: r.get(1)?,
        url: r.get(2)?,
        auth: r.get(3)?,
        client: r.get(4)?,
        tools: serde_json::from_str(&tools).unwrap_or_default(),
        heavy: serde_json::from_str(&heavy).unwrap_or_default(),
        status: r.get(7)?,
        error: r.get(8)?,
        listed_at: r.get(9)?,
        added_at: r.get(10)?,
    })
}

fn json<T: Serialize>(value: &T) -> Result<String, CoreError> {
    serde_json::to_string(value).map_err(|e| CoreError::storage(e.to_string()))
}

impl Store {
    /// Every added server, oldest first.
    pub fn mcp_servers(&self) -> Result<Vec<StoredMcpServer>, CoreError> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(&format!("SELECT {COLUMNS} FROM mcp_servers ORDER BY added_at, id"))?;
        let rows = stmt.query_map([], from_row)?.collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn mcp_server(&self, id: &str) -> Result<Option<StoredMcpServer>, CoreError> {
        Ok(self
            .lock()?
            .query_row(&format!("SELECT {COLUMNS} FROM mcp_servers WHERE id = ?1"), params![id], from_row)
            .optional()?)
    }

    /// Adds a server, or replaces the one with the same id.
    pub fn mcp_put(&self, s: &StoredMcpServer) -> Result<(), CoreError> {
        self.lock()?.execute(
            &format!(
                "INSERT INTO mcp_servers ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11) \
                 ON CONFLICT (id) DO UPDATE SET name = ?2, url = ?3, auth = ?4, client = ?5, tools = ?6, heavy = ?7, \
                 status = ?8, error = ?9, listed_at = ?10"
            ),
            params![
                s.id,
                s.name,
                s.url,
                s.auth,
                s.client,
                json(&s.tools)?,
                json(&s.heavy)?,
                s.status,
                s.error,
                s.listed_at,
                s.added_at
            ],
        )?;
        Ok(())
    }

    /// The tools just listed; the heavy marks of tools that are gone are dropped.
    pub fn mcp_set_tools(&self, id: &str, tools: &[McpTool], now: i64) -> Result<(), CoreError> {
        let Some(mut server) = self.mcp_server(id)? else {
            return Err(CoreError::NotFound);
        };
        server.heavy.retain(|h| tools.iter().any(|t| &t.name == h));
        self.lock()?.execute(
            "UPDATE mcp_servers SET tools = ?2, heavy = ?3, listed_at = ?4, status = 'ok', error = NULL WHERE id = ?1",
            params![id, json(&tools)?, json(&server.heavy)?, now],
        )?;
        Ok(())
    }

    pub fn mcp_set_status(&self, id: &str, status: &str, error: Option<&str>) -> Result<(), CoreError> {
        self.lock()?
            .execute("UPDATE mcp_servers SET status = ?2, error = ?3 WHERE id = ?1", params![id, status, error])?;
        Ok(())
    }

    pub fn mcp_set_client(&self, id: &str, client: Option<&str>) -> Result<(), CoreError> {
        self.lock()?.execute("UPDATE mcp_servers SET client = ?2 WHERE id = ?1", params![id, client])?;
        Ok(())
    }

    /// Marks a tool heavy (its results go through the server) or not. False when the server or tool is unknown.
    pub fn mcp_set_heavy(&self, id: &str, tool: &str, heavy: bool) -> Result<bool, CoreError> {
        let mut conn = self.lock()?;
        let tx = conn.transaction()?;
        let row: Option<(String, String)> = tx
            .query_row("SELECT tools, heavy FROM mcp_servers WHERE id = ?1", params![id], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()?;
        let Some((tools, marks)) = row else {
            return Ok(false);
        };
        let tools: Vec<McpTool> = serde_json::from_str(&tools).unwrap_or_default();
        if !tools.iter().any(|t| t.name == tool) {
            return Ok(false);
        }
        let mut marks: Vec<String> = serde_json::from_str(&marks).unwrap_or_default();
        marks.retain(|m| m != tool);
        if heavy {
            marks.push(tool.to_owned());
        }
        tx.execute("UPDATE mcp_servers SET heavy = ?2 WHERE id = ?1", params![id, json(&marks)?])?;
        tx.commit()?;
        Ok(true)
    }

    /// Forgets a server with its tokens, its unfinished sign-in and the permissions given for it (`grant_service` is
    /// the service its grants name). False when it was not there.
    pub fn mcp_remove(&self, id: &str, grant_service: &str) -> Result<bool, CoreError> {
        let mut conn = self.lock()?;
        let tx = conn.transaction()?;
        let removed = tx.execute("DELETE FROM mcp_servers WHERE id = ?1", params![id])? > 0;
        tx.execute(
            "DELETE FROM secrets WHERE account = ?1 AND service IN (?2, ?3)",
            params![id, SECRET_SERVICE, SIGN_IN_SERVICE],
        )?;
        tx.execute(
            "DELETE FROM grants WHERE json_extract(grant_json, '$.scope.service') = ?1",
            params![grant_service],
        )?;
        tx.commit()?;
        Ok(removed)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::store::tests::open;

    fn server(id: &str, added_at: i64) -> StoredMcpServer {
        StoredMcpServer {
            id: id.to_owned(),
            name: "Linear".to_owned(),
            url: "https://mcp.linear.app/mcp".to_owned(),
            auth: "oauth".to_owned(),
            client: Some("{\"client_id\":\"c\"}".to_owned()),
            tools: Vec::new(),
            heavy: Vec::new(),
            status: "needs_sign_in".to_owned(),
            error: None,
            listed_at: None,
            added_at,
        }
    }

    fn tool(name: &str) -> McpTool {
        McpTool {
            name: name.to_owned(),
            title: None,
            description: "d".to_owned(),
            input_schema: json!({"type": "object"}),
            read_only: true,
            destructive: false,
        }
    }

    #[test]
    fn servers_keep_their_tools_heavy_marks_and_secrets_until_removed() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());
        store.mcp_put(&server("b", 2)).unwrap();
        store.mcp_put(&server("a", 1)).unwrap();
        assert_eq!(store.mcp_servers().unwrap().iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["a", "b"]);

        store.mcp_set_tools("a", &[tool("search"), tool("export")], 10).unwrap();
        assert!(store.mcp_set_heavy("a", "export", true).unwrap());
        assert!(!store.mcp_set_heavy("a", "nope", true).unwrap(), "an unknown tool cannot be marked");
        assert!(!store.mcp_set_heavy("zz", "export", true).unwrap());
        let a = store.mcp_server("a").unwrap().unwrap();
        assert_eq!(
            (a.status.as_str(), a.listed_at, a.heavy.as_slice()),
            ("ok", Some(10), ["export".to_owned()].as_slice())
        );

        // A tool that disappears loses its mark.
        store.mcp_set_tools("a", &[tool("search")], 11).unwrap();
        assert_eq!(store.mcp_server("a").unwrap().unwrap().heavy.len(), 0);

        store.mcp_set_status("a", "error", Some("could not connect")).unwrap();
        assert_eq!(store.mcp_server("a").unwrap().unwrap().error.as_deref(), Some("could not connect"));

        store.secret_put(SECRET_SERVICE, "a", b"tokens").unwrap();
        store.secret_put(SIGN_IN_SERVICE, "a", b"verifier").unwrap();
        store.secret_put(SECRET_SERVICE, "b", b"other").unwrap();
        assert!(store.mcp_remove("a", "mcp_a").unwrap());
        assert!(!store.mcp_remove("a", "mcp_a").unwrap());
        assert_eq!(store.mcp_server("a").unwrap(), None);
        assert_eq!(store.secret_get(SECRET_SERVICE, "a").unwrap(), None);
        assert_eq!(store.secret_get(SIGN_IN_SERVICE, "a").unwrap(), None);
        assert!(store.secret_get(SECRET_SERVICE, "b").unwrap().is_some(), "other servers keep theirs");
    }
}
