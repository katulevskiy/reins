//! Portable database snapshots. The serialized bytes are always sealed before touching disk or the server.
use crate::CoreError;
use ring::digest;
use rusqlite::{Connection, OptionalExtension, types::Value};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use zeroize::Zeroizing;

/// Not `audit_snapshots`: an entry's approval (up to 256 KiB each) stays on the phone that showed it. A sealed copy on
/// disk is the whole database instead, so it keeps it.
pub(super) const TABLES: &[&str] = &[
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
/// Tables that belong to this installation only: never in a portable snapshot.
const DEVICE_TABLES: &[&str] = &["session", "pending", "handled", "autopilot_suggestions", "autopilot_rate"];
/// Installation metadata (`meta.key`), besides every `app.session.*` key: never in a portable snapshot.
pub(super) const DEVICE_META: &[&str] = &[
    "device_id",
    "dek_check",
    "app.account-state.revision",
    "app.account-state.synced",
    "app.account-state.notice",
    "app.account-state.seen",
    "app.account-state.agreed-at",
];
/// Installation keys (`secrets.service`): never in a portable snapshot, and refused in one.
pub(super) const DEVICE_SECRETS: &[&str] = &[
    "reins.device-key",
    "reins.inbox-key",
    "reins.account-secret",
    "vault",
    "reins.join",
    "mcp_signin",
    "payments.provider",
];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct Snapshot {
    pub version: u32,
    pub tables: BTreeMap<String, Table>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct Table {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Cell>>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) enum Cell {
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
impl Table {
    pub fn column(&self, name: &str) -> Option<usize> {
        self.columns.iter().position(|c| c == name)
    }
}
fn quote(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}
fn list(items: &[&str]) -> String {
    items.iter().map(|s| format!("'{s}'")).collect::<Vec<_>>().join(",")
}
/// The rows of `table` a portable snapshot carries (and an import replaces): all but this installation's own.
fn portable_rows(table: &str) -> String {
    match table {
        "meta" => format!(" WHERE key NOT IN ({}) AND key NOT LIKE 'app.session.%'", list(DEVICE_META)),
        "secrets" => format!(" WHERE service NOT IN ({})", list(DEVICE_SECRETS)),
        _ => String::new(),
    }
}
fn is_device_row(table: &str, row: &[Cell]) -> bool {
    match (table, row.first()) {
        ("secrets", Some(Cell::Text(s))) => DEVICE_SECRETS.contains(&s.as_str()),
        ("meta", Some(Cell::Text(s))) => DEVICE_META.contains(&s.as_str()) || s.starts_with("app.session."),
        _ => false,
    }
}

pub(super) fn parse(bytes: &[u8]) -> Result<Snapshot, CoreError> {
    serde_json::from_slice(bytes).map_err(|_| CoreError::storage("invalid account data"))
}
pub(super) fn encode(snapshot: &Snapshot) -> Result<Zeroizing<Vec<u8>>, CoreError> {
    serde_json::to_vec(snapshot).map(Zeroizing::new).map_err(|_| CoreError::storage("cannot encode account data"))
}
pub(super) fn has_rows(snapshot: &Snapshot) -> bool {
    snapshot.tables.values().any(|table| !table.rows.is_empty())
}

/// The hash of exactly these bytes: what versions before canonical hashes kept as the last synced state.
pub(super) fn raw_hash(bytes: &[u8]) -> String {
    data_encoding::HEXLOWER.encode(digest::digest(&digest::SHA256, bytes).as_ref())
}
/// A hash of the data, not of how it was written down: tables by name, each row's cells by column name, rows in
/// sorted order. Two snapshots of the same rows hash alike whatever order SQLite returned them in.
pub(super) fn canonical_hash(snapshot: &Snapshot) -> String {
    let mut context = digest::Context::new(&digest::SHA256);
    for (name, table) in &snapshot.tables {
        let mut order: Vec<usize> = (0..table.columns.len()).collect();
        order.sort_by(|a, b| table.columns[*a].cmp(&table.columns[*b]));
        let mut rows: Vec<Vec<u8>> = table
            .rows
            .iter()
            .map(|row| {
                let named: Vec<(&str, Option<&Cell>)> =
                    order.iter().map(|i| (table.columns[*i].as_str(), row.get(*i))).collect();
                serde_json::to_vec(&named).unwrap_or_default()
            })
            .collect();
        rows.sort();
        context.update(&serde_json::to_vec(&name).unwrap_or_default());
        context.update(&u64::try_from(rows.len()).unwrap_or(u64::MAX).to_be_bytes());
        for row in rows {
            context.update(&u64::try_from(row.len()).unwrap_or(u64::MAX).to_be_bytes());
            context.update(&row);
        }
    }
    format!("c1:{}", data_encoding::HEXLOWER.encode(context.finish().as_ref()))
}

fn table_exists(conn: &Connection, name: &str) -> Result<bool, CoreError> {
    Ok(conn
        .query_row("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1", [name], |_| Ok(()))
        .optional()?
        .is_some())
}

pub(super) fn export(conn: &Connection, portable: bool) -> Result<Zeroizing<Vec<u8>>, CoreError> {
    export_tables(conn, portable, false)
}
/// [`export`] of a database written by an older version, before it is brought up to date: the tables it does not
/// have yet are left out, as that version's own export left them out.
pub(super) fn export_present(conn: &Connection) -> Result<Zeroizing<Vec<u8>>, CoreError> {
    export_tables(conn, true, true)
}
fn export_tables(conn: &Connection, portable: bool, present_only: bool) -> Result<Zeroizing<Vec<u8>>, CoreError> {
    let mut tables = BTreeMap::new();
    for name in TABLES {
        if (portable && DEVICE_TABLES.contains(name)) || (present_only && !table_exists(conn, name)?) {
            continue;
        }
        let predicate = if portable {
            portable_rows(name)
        } else {
            String::new()
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
    encode(&Snapshot {
        version: 1,
        tables,
    })
}

/// Why a snapshot cannot be read into this version's tables.
#[derive(Debug)]
pub(super) enum Unreadable {
    /// It has tables or columns this version does not know: a newer version wrote it.
    Newer,
    Invalid(CoreError),
}
impl From<CoreError> for Unreadable {
    fn from(e: CoreError) -> Self {
        Self::Invalid(e)
    }
}
impl From<rusqlite::Error> for Unreadable {
    fn from(e: rusqlite::Error) -> Self {
        Self::Invalid(e.into())
    }
}
impl From<Unreadable> for CoreError {
    fn from(e: Unreadable) -> Self {
        match e {
            Unreadable::Newer => Self::storage("account data from a newer version of Reins"),
            Unreadable::Invalid(e) => e,
        }
    }
}

pub(super) fn import(conn: &mut Connection, bytes: &[u8], portable: bool) -> Result<(), CoreError> {
    import_snapshot(conn, parse(bytes)?, portable).map_err(CoreError::from)
}

/// Replaces the tables `snapshot` has with its rows (a portable one keeps this installation's own rows). A table or
/// column the snapshot lacks (an older version wrote it) takes the schema's defaults; one this version does not know
/// is [`Unreadable::Newer`] and changes nothing.
pub(super) fn import_snapshot(conn: &mut Connection, snapshot: Snapshot, portable: bool) -> Result<(), Unreadable> {
    if snapshot.version != 1 {
        return Err(CoreError::storage("unsupported account data version").into());
    }
    let tx = conn.transaction()?;
    // Rows replaced here were not deleted by anyone: no tombstones for them.
    super::drop_tombstone_triggers(&tx)?;
    for (name, table) in snapshot.tables {
        if !TABLES.contains(&name.as_str()) {
            return Err(Unreadable::Newer);
        }
        if portable && DEVICE_TABLES.contains(&name.as_str()) {
            return Err(CoreError::storage("invalid account data table").into());
        }
        let actual = tx
            .prepare(&format!("SELECT * FROM {} LIMIT 0", quote(&name)))?
            .column_names()
            .iter()
            .map(|s| (*s).to_owned())
            .collect::<Vec<_>>();
        if table.columns.iter().any(|c| !actual.contains(c)) {
            return Err(Unreadable::Newer);
        }
        // An entry's approval is kept by its sequence number: find each entry again among the new rows.
        let kept_snapshots = if portable && name == "audit" {
            audit_snapshot_owners(&tx)?
        } else {
            Vec::new()
        };
        // Portable snapshots omit installation metadata and keys; retain these local rows.
        let predicate = if portable {
            portable_rows(&name)
        } else {
            String::new()
        };
        tx.execute(&format!("DELETE FROM {}{predicate}", quote(&name)), [])?;
        let columns = table.columns.iter().map(|s| quote(s)).collect::<Vec<_>>().join(",");
        let placeholders = (0..table.columns.len()).map(|_| "?").collect::<Vec<_>>().join(",");
        let mut insert = tx.prepare(&format!("INSERT INTO {} ({columns}) VALUES ({placeholders})", quote(&name)))?;
        let seq = table.column("seq");
        let mut new_seqs: HashMap<String, Vec<i64>> = HashMap::new();
        for row in table.rows {
            if row.len() != table.columns.len() {
                return Err(CoreError::storage("invalid account data row").into());
            }
            // Never accept secret or session-key rows from a portable snapshot, even if its author supplies them.
            if portable && is_device_row(&name, &row) {
                return Err(CoreError::storage(if name == "secrets" {
                    "account data contains an installation key"
                } else {
                    "account data contains installation metadata"
                })
                .into());
            }
            if !kept_snapshots.is_empty()
                && let Some(i) = seq
                && let Some(Cell::Integer(s)) = row.get(i)
            {
                new_seqs.entry(identity(&table.columns, &row, "seq")).or_default().push(*s);
            }
            insert.execute(rusqlite::params_from_iter(row.into_iter().map(Value::from)))?;
        }
        drop(insert);
        if !kept_snapshots.is_empty() {
            move_audit_snapshots(&tx, kept_snapshots, &mut new_seqs)?;
        }
    }
    tx.execute_batch(&super::tombstone_triggers())?;
    tx.commit()?;
    Ok(())
}

/// A row without its sequence number (`skip`): what identifies a log entry across phones, which number theirs
/// independently.
pub(super) fn identity(columns: &[String], row: &[Cell], skip: &str) -> String {
    let named: BTreeMap<&str, &Cell> =
        columns.iter().zip(row).filter(|(c, _)| c.as_str() != skip).map(|(c, v)| (c.as_str(), v)).collect();
    serde_json::to_string(&named).unwrap_or_default()
}
/// The entries that have a kept approval: their sequence number and identity.
fn audit_snapshot_owners(conn: &Connection) -> Result<Vec<(i64, String)>, CoreError> {
    if !table_exists(conn, "audit_snapshots")? {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare("SELECT * FROM audit WHERE seq IN (SELECT seq FROM audit_snapshots)")?;
    let columns = stmt.column_names().iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
    let seq = columns.iter().position(|c| c == "seq").ok_or_else(|| CoreError::storage("audit has no seq"))?;
    let count = columns.len();
    let rows = stmt
        .query_map([], |r| (0..count).map(|i| r.get::<_, Value>(i).map(Cell::from)).collect::<Result<Vec<_>, _>>())?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows
        .into_iter()
        .filter_map(|row| match row.get(seq) {
            Some(Cell::Integer(s)) => Some((*s, identity(&columns, &row, "seq"))),
            _ => None,
        })
        .collect())
}
/// Moves each kept approval to its entry's new sequence number; an approval whose entry is gone goes too.
fn move_audit_snapshots(
    conn: &Connection,
    owners: Vec<(i64, String)>,
    new_seqs: &mut HashMap<String, Vec<i64>>,
) -> Result<(), CoreError> {
    conn.execute("UPDATE audit_snapshots SET seq = -1 - seq", [])?;
    for (old, id) in owners {
        if let Some(seqs) = new_seqs.get_mut(&id)
            && !seqs.is_empty()
        {
            let new = seqs.remove(0);
            conn.execute("UPDATE audit_snapshots SET seq = ?1 WHERE seq = ?2", [new, -1 - old])?;
        }
    }
    conn.execute("DELETE FROM audit_snapshots WHERE seq < 0", [])?;
    Ok(())
}
