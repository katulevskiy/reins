//! Portable database snapshots. The serialized bytes are always sealed before touching disk or the server.
use crate::CoreError;
use rusqlite::{Connection, types::Value};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use zeroize::Zeroizing;

const TABLES: &[&str] = &[
    "meta",
    "session",
    "grants",
    "audit",
    "pending",
    "handled",
    "connection_prefs",
    "accounts",
    "secrets",
    "desktop_keys",
    "mcp_servers",
    "autopilot_settings",
    "autopilot_profiles",
    "autopilot_memory",
    "autopilot_suggestions",
    "autopilot_rate",
    "autopilot_targets",
];
#[derive(Serialize, Deserialize)]
struct Snapshot {
    version: u32,
    tables: BTreeMap<String, Table>,
}
#[derive(Serialize, Deserialize)]
struct Table {
    columns: Vec<String>,
    rows: Vec<Vec<Cell>>,
}
#[derive(Serialize, Deserialize)]
enum Cell {
    Null,
    Integer(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}
impl From<Value> for Cell {
    fn from(v: Value) -> Self {
        match v {
            Value::Null => Self::Null,
            Value::Integer(v) => Self::Integer(v),
            Value::Real(v) => Self::Real(v),
            Value::Text(v) => Self::Text(v),
            Value::Blob(v) => Self::Blob(v),
        }
    }
}
impl From<Cell> for Value {
    fn from(v: Cell) -> Self {
        match v {
            Cell::Null => Self::Null,
            Cell::Integer(v) => Self::Integer(v),
            Cell::Real(v) => Self::Real(v),
            Cell::Text(v) => Self::Text(v),
            Cell::Blob(v) => Self::Blob(v),
        }
    }
}
fn quote(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

pub(super) fn has_rows(bytes: &[u8]) -> Result<bool, CoreError> {
    let snapshot: Snapshot = serde_json::from_slice(bytes).map_err(|_| CoreError::storage("invalid account data"))?;
    Ok(snapshot.tables.values().any(|table| !table.rows.is_empty()))
}

pub(super) fn export(conn: &Connection, portable: bool) -> Result<Zeroizing<Vec<u8>>, CoreError> {
    let mut tables = BTreeMap::new();
    for name in TABLES {
        if portable && matches!(*name, "session" | "pending" | "handled" | "autopilot_suggestions" | "autopilot_rate") {
            continue;
        }
        let predicate = if portable && *name == "meta" {
            " WHERE key NOT IN ('device_id','dek_check','app.account-state.revision','app.account-state.synced') AND key NOT LIKE 'app.session.%'"
        } else if portable && *name == "secrets" {
            " WHERE service NOT IN ('reins.device-key','reins.inbox-key','reins.account-secret','vault','reins.join','mcp_signin','payments.provider')"
        } else {
            ""
        };
        let mut stmt = conn.prepare(&format!("SELECT * FROM {}{predicate}", quote(name)))?;
        let columns = stmt.column_names().iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        let count = columns.len();
        let rows = stmt
            .query_map([], |r| (0..count).map(|i| r.get::<_, Value>(i).map(Cell::from)).collect::<Result<Vec<_>, _>>())?
            .collect::<Result<Vec<_>, _>>()?;
        tables.insert(
            (*name).to_owned(),
            Table {
                columns,
                rows,
            },
        );
    }
    serde_json::to_vec(&Snapshot {
        version: 1,
        tables,
    })
    .map(Zeroizing::new)
    .map_err(|_| CoreError::storage("cannot encode account data"))
}

pub(super) fn import(conn: &mut Connection, bytes: &[u8], portable: bool) -> Result<(), CoreError> {
    let snapshot: Snapshot = serde_json::from_slice(bytes).map_err(|_| CoreError::storage("invalid account data"))?;
    if snapshot.version != 1 {
        return Err(CoreError::storage("unsupported account data version"));
    }
    let tx = conn.transaction()?;
    for (name, table) in snapshot.tables {
        if !TABLES.contains(&name.as_str())
            || (portable
                && matches!(
                    name.as_str(),
                    "session" | "pending" | "handled" | "autopilot_suggestions" | "autopilot_rate"
                ))
        {
            return Err(CoreError::storage("invalid account data table"));
        }
        let actual = tx
            .prepare(&format!("SELECT * FROM {} LIMIT 0", quote(&name)))?
            .column_names()
            .iter()
            .map(|s| (*s).to_owned())
            .collect::<Vec<_>>();
        if table.columns != actual {
            return Err(CoreError::storage("unsupported account data schema"));
        }
        // Portable snapshots omit installation metadata and keys; retain these local rows.
        let predicate = if portable && name == "meta" {
            " WHERE key NOT IN ('device_id','dek_check','app.account-state.revision','app.account-state.synced') AND key NOT LIKE 'app.session.%'"
        } else if portable && name == "secrets" {
            " WHERE service NOT IN ('reins.device-key','reins.inbox-key','reins.account-secret','vault','reins.join','mcp_signin','payments.provider')"
        } else {
            ""
        };
        tx.execute(&format!("DELETE FROM {}{predicate}", quote(&name)), [])?;
        let columns = table.columns.iter().map(|s| quote(s)).collect::<Vec<_>>().join(",");
        let placeholders = (0..actual.len()).map(|_| "?").collect::<Vec<_>>().join(",");
        let mut insert = tx.prepare(&format!("INSERT INTO {} ({columns}) VALUES ({placeholders})", quote(&name)))?;
        for row in table.rows {
            if row.len() != actual.len() {
                return Err(CoreError::storage("invalid account data row"));
            }
            // Never accept secret or session-key rows from a portable snapshot, even if its author supplies them.
            if portable
                && name == "secrets"
                && matches!(row.first(),Some(Cell::Text(s)) if matches!(s.as_str(),"reins.device-key"|"reins.inbox-key"|"reins.account-secret"|"vault"|"reins.join"|"mcp_signin"))
            {
                return Err(CoreError::storage("account data contains an installation key"));
            }
            if portable
                && name == "meta"
                && matches!(row.first(),Some(Cell::Text(s)) if matches!(s.as_str(),"device_id"|"dek_check"|"app.account-state.revision"|"app.account-state.synced")||s.starts_with("app.session."))
            {
                return Err(CoreError::storage("account data contains installation metadata"));
            }
            insert.execute(rusqlite::params_from_iter(row.into_iter().map(Value::from)))?;
        }
    }
    tx.commit()?;
    Ok(())
}
