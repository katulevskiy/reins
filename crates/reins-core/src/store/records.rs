//! The bridge between this store's tables and the op log's records (`crate::live_sync`): what a row of each synced
//! table is as a record, and back.
//!
//! A record's value is the row in plain JSON, one entry per column: integers and text as they are, bytes as
//! `{"$b64": ...}`. Columns this store seals for itself (Activity's detail and info, Autopilot profiles and memory,
//! tokens) are opened on the way out and sealed again on the way in, so a record never depends on this phone's data
//! key. Per table:
//!
//! | Table | Record key | Value |
//! |---|---|---|
//! | `grants` | the id | the columns, `grant_json` parsed as `grant` (the op-log rules join grants) |
//! | `autopilot_settings` | the scope (`*` for the global one) | the columns |
//! | `autopilot_profiles`, `mcp_servers` | the id | the columns |
//! | `autopilot_targets` | the key | the columns |
//! | `accounts`, `secrets` | `["service","account"]` | the columns (a token is sealed to the credential key on top, by the sync) |
//! | `desktop_keys`, `connection_prefs` | the connection id | the columns |
//! | `autopilot_memory` | a hash of the entry (each phone numbers its own) | the columns but the id |
//! | `meta` | the key without `app.` (`~` before a key without it) | JSON settings as they are; other text `{"$text"}`, numbers `{"$int"}`, bytes `{"$b64"}` |
//! | `audit` (Activity) | its op's [`crate::store::oplog::activity_key`] | the columns but the local seq |
//!
//! Never a record: this installation's own keys and metadata (`snapshot.rs`'s device lists), sign-in state, the
//! store's own tombstones (the op log keeps its own), and keys the sync reserves for itself (`~reins.`).

use data_encoding::BASE64URL_NOPAD;
use reins_proto::canon::{hex32, sha256};
use reins_proto::oplog::{MAX_RECORD_KEY_CHARS, Table};
use rusqlite::types::Value;
use rusqlite::{Connection, params_from_iter};
use serde_json::{Map, Value as Json};

use super::snapshot::{DEVICE_META, DEVICE_SECRETS};
use super::{Store, TOMBSTONE_PREFIX};
use crate::CoreError;

/// The global Autopilot setting's scope in the store, and its record key.
const GLOBAL_SCOPE: (&str, &str) = ("", "*");
/// Record keys the sync keeps for itself (view-key grants): never a store row.
pub(crate) const RESERVED_PREFIX: &str = "~reins.";
/// `meta` keys that are this installation's or sign-in's, besides [`DEVICE_META`].
const LOCAL_META_PREFIXES: &[&str] = &["app.session.", "app.active-", "app.account-state."];
/// `meta` rows this store seals, by key: their sealing context.
const SEALED_META: &[(&str, &str)] = &[("autopilot_salt", crate::autopilot::store::SALT_AAD)];

/// The synced tables besides `meta` and `audit`: the op log's table, the store's, and the key columns (none: a log
/// told apart by content).
pub(crate) const TABLES: &[(Table, &str, &[&str])] = &[
    (Table::Grants, "grants", &["id"]),
    (Table::AutopilotSettings, "autopilot_settings", &["scope"]),
    (Table::Profiles, "autopilot_profiles", &["id"]),
    (Table::AutopilotMemory, "autopilot_memory", &[]),
    (Table::Targets, "autopilot_targets", &["key"]),
    (Table::Accounts, "accounts", &["service", "account"]),
    (Table::Secrets, "secrets", &["service", "account"]),
    (Table::DesktopKeys, "desktop_keys", &["connection_id"]),
    (Table::McpServers, "mcp_servers", &["id"]),
    (Table::ConnectionPrefs, "connection_prefs", &["connection_id"]),
];
/// The column a log table numbers its rows by on each phone (never part of a record).
const LOG_ID: &[(&str, &str)] = &[("autopilot_memory", "id"), ("audit", "seq")];

/// One row as a record.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SyncRow {
    pub table: Table,
    pub key: String,
    pub value: Json,
    /// How this store finds the row again (its key columns, or its local id), JSON.
    pub local: String,
}

/// What a record from an untrusted start becomes ([`Store::tighten_record`]).
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Tightened {
    /// Written as this.
    Write(Json),
    /// Not taken, and gone from the account (the primary publishes the removal).
    Skip,
    /// Not taken; this phone's own (if any) stays, and nothing is published of it.
    KeepLocal,
}

/// Everything in `mine` and in `theirs` (lists: each item once; maps: every key, this phone's value first).
fn union(mine: Option<Json>, theirs: Json) -> Json {
    match (mine, theirs) {
        (Some(Json::Array(mut a)), Json::Array(b)) => {
            for x in b {
                if !a.contains(&x) {
                    a.push(x);
                }
            }
            Json::Array(a)
        }
        (Some(Json::Object(mut a)), Json::Object(b)) => {
            for (k, x) in b {
                a.entry(k).or_insert(x);
            }
            Json::Object(a)
        }
        (Some(mine), _) if !mine.is_null() => mine,
        (_, theirs) => theirs,
    }
}

impl SyncRow {
    /// A digest of the value, whatever order its entries come in.
    pub fn digest(&self) -> String {
        digest(&self.value)
    }
}

/// A digest of a record value (entries in sorted order at every level).
pub(crate) fn digest(value: &Json) -> String {
    hex32(&sha256(canonical(value).as_bytes()))
}

fn canonical(value: &Json) -> String {
    match value {
        Json::Object(map) => {
            let mut entries: Vec<(&String, &Json)> = map.iter().collect();
            entries.sort_by(|a, b| a.0.cmp(b.0));
            let inner: Vec<String> =
                entries.into_iter().map(|(k, v)| format!("{}:{}", Json::String(k.clone()), canonical(v))).collect();
            format!("{{{}}}", inner.join(","))
        }
        Json::Array(items) => format!("[{}]", items.iter().map(canonical).collect::<Vec<_>>().join(",")),
        other => other.to_string(),
    }
}

/// The store table of an op-log table this bridge carries.
pub(crate) fn sql_table(table: Table) -> Option<&'static str> {
    match table {
        Table::Meta => Some("meta"),
        Table::Audit => Some("audit"),
        _ => TABLES.iter().find(|(t, _, _)| *t == table).map(|(_, name, _)| *name),
    }
}

fn key_columns(name: &str) -> &'static [&'static str] {
    TABLES.iter().find(|(_, n, _)| *n == name).map_or(&[], |(_, _, k)| k)
}

fn log_id(name: &str) -> Option<&'static str> {
    LOG_ID.iter().find(|(t, _)| *t == name).map(|(_, c)| *c)
}

fn bad(what: &str) -> CoreError {
    CoreError::invalid(format!("a synced record is malformed: {what}"))
}

fn b64(bytes: &[u8]) -> Json {
    let mut m = Map::new();
    m.insert("$b64".to_owned(), Json::String(BASE64URL_NOPAD.encode(bytes)));
    Json::Object(m)
}

fn bytes_of(value: &Json) -> Option<Vec<u8>> {
    let text = value.as_object().filter(|m| m.len() == 1)?.get("$b64")?.as_str()?;
    BASE64URL_NOPAD.decode(text.as_bytes()).ok()
}

/// What the store seals `column` of `table` with, for this row (its other columns).
fn column_aad(table: &str, column: &str, row: &Map<String, Json>) -> Option<String> {
    let text = |c: &str| row.get(c).and_then(Json::as_str).unwrap_or_default().to_owned();
    match (table, column) {
        ("audit", "detail") => Some(super::audit::DETAIL_AAD.to_owned()),
        ("audit", "info") => Some(super::audit::INFO_AAD.to_owned()),
        ("secrets", "value") => Some(super::secrets::aad(&text("service"), &text("account"))),
        ("autopilot_profiles", "data") => Some(crate::autopilot::store::profile_aad(&text("id"))),
        ("autopilot_memory", "data") => {
            Some(crate::autopilot::store::memory_aad(&text("profile_id"), &text("request_id")))
        }
        _ => None,
    }
}

fn cell_json(cell: Value) -> Json {
    match cell {
        Value::Null => Json::Null,
        Value::Integer(i) => Json::from(i),
        Value::Real(f) => serde_json::Number::from_f64(f).map_or(Json::Null, Json::Number),
        Value::Text(s) => Json::String(s),
        Value::Blob(b) => b64(&b),
    }
}

fn json_cell(value: &Json) -> Result<Value, CoreError> {
    Ok(match value {
        Json::Null => Value::Null,
        Json::Bool(b) => Value::Integer(i64::from(*b)),
        Json::Number(n) => match n.as_i64() {
            Some(i) => Value::Integer(i),
            None => Value::Real(n.as_f64().ok_or_else(|| bad("a number"))?),
        },
        Json::String(s) => Value::Text(s.clone()),
        other => Value::Blob(bytes_of(other).ok_or_else(|| bad("a column value"))?),
    })
}

/// A record key for `local` (the key columns' values): their text, a JSON list of several, a hash when too long.
fn record_key(name: &str, local: &[Json]) -> String {
    let key = match local {
        [Json::String(s)] if name == "autopilot_settings" && s == GLOBAL_SCOPE.0 => GLOBAL_SCOPE.1.to_owned(),
        [Json::String(s)] => s.clone(),
        [other] => other.to_string(),
        many => Json::Array(many.to_vec()).to_string(),
    };
    let fits = !key.is_empty()
        && key.chars().count() <= MAX_RECORD_KEY_CHARS
        && !key.chars().any(char::is_control)
        && !key.starts_with('#')
        && !key.starts_with(RESERVED_PREFIX);
    if fits {
        key
    } else {
        format!("#{}", hex32(&sha256(key.as_bytes())))
    }
}

/// How the store finds the row a record names, from the record key alone (a row this phone had before it took the
/// log); `None` for a key that is a hash.
pub(crate) fn local_of(table: Table, key: &str) -> Option<String> {
    if key.starts_with('#') || key.starts_with(RESERVED_PREFIX) {
        return None;
    }
    if table == Table::Meta {
        return Some(Json::Array(vec![Json::String(meta_store_key(key))]).to_string());
    }
    let name = sql_table(table)?;
    match key_columns(name) {
        [_] => {
            let value = if name == "autopilot_settings" && key == GLOBAL_SCOPE.1 {
                GLOBAL_SCOPE.0
            } else {
                key
            };
            Some(Json::Array(vec![Json::String(value.to_owned())]).to_string())
        }
        [_, _] => {
            let parts: Vec<Json> = serde_json::from_str(key).ok()?;
            (parts.len() == 2 && parts.iter().all(Json::is_string)).then(|| Json::Array(parts).to_string())
        }
        _ => None,
    }
}

/// The record key a proposed or written row files under (its key columns).
pub(crate) fn key_of_row(table: Table, row: &Json) -> Option<String> {
    let name = sql_table(table)?;
    let keys = key_columns(name);
    if keys.is_empty() {
        return None;
    }
    let local: Vec<Json> = keys.iter().map(|k| row.get(*k).cloned().unwrap_or(Json::Null)).collect();
    local.iter().all(|v| !v.is_null()).then(|| record_key(name, &local))
}

fn is_local_meta(key: &str) -> bool {
    DEVICE_META.contains(&key)
        || key.starts_with(TOMBSTONE_PREFIX)
        || LOCAL_META_PREFIXES.iter().any(|p| key.starts_with(p))
}

/// A `meta` key's record key, and back.
fn meta_record_key(key: &str) -> String {
    key.strip_prefix("app.").map_or_else(|| format!("~{key}"), str::to_owned)
}

fn meta_store_key(record: &str) -> String {
    record.strip_prefix('~').map_or_else(|| format!("app.{record}"), str::to_owned)
}

impl Store {
    fn unseal_column(&self, table: &str, column: &str, row: &Map<String, Json>, sealed: &Json) -> Option<Json> {
        match column_aad(table, column, row) {
            Some(aad) => {
                let raw = bytes_of(sealed)?;
                Some(b64(&self.unseal(&aad, &raw).ok()?))
            }
            None => Some(sealed.clone()),
        }
    }

    /// The rows of `name` as plain JSON objects (sealed columns opened). A row whose sealed column does not open (not
    /// this account's key) is left out.
    fn plain_rows(&self, conn: &Connection, name: &str, filter: &str) -> Result<Vec<Map<String, Json>>, CoreError> {
        let mut stmt = conn.prepare(&format!("SELECT * FROM {name}{filter}"))?;
        let columns: Vec<String> = stmt.column_names().iter().map(|c| (*c).to_owned()).collect();
        let raw = stmt
            .query_map([], |r| (0..columns.len()).map(|i| r.get::<_, Value>(i)).collect::<Result<Vec<_>, _>>())?
            .collect::<Result<Vec<_>, _>>()?;
        let mut out = Vec::with_capacity(raw.len());
        'rows: for cells in raw {
            let mut row: Map<String, Json> = columns.iter().cloned().zip(cells.into_iter().map(cell_json)).collect();
            let sealed: Vec<String> = columns.iter().filter(|c| column_aad(name, c, &row).is_some()).cloned().collect();
            for column in sealed {
                let value = row.get(&column).cloned().unwrap_or(Json::Null);
                if value.is_null() {
                    continue;
                }
                let Some(plain) = self.unseal_column(name, &column, &row, &value) else {
                    log::warn!("a {name} row does not open with this account's key; it is not synced");
                    continue 'rows;
                };
                row.insert(column, plain);
            }
            out.push(row);
        }
        Ok(out)
    }

    /// Every synced row but Activity's, as records.
    pub(crate) fn sync_rows(&self) -> Result<Vec<SyncRow>, CoreError> {
        let conn = self.lock()?;
        let mut rows = Vec::new();
        for (table, name, keys) in TABLES {
            let filter = if *name == "secrets" {
                format!(" WHERE service NOT IN ({})", quoted(DEVICE_SECRETS))
            } else {
                String::new()
            };
            for mut row in self.plain_rows(&conn, name, &filter)? {
                let (key, local) = if let Some(id) = log_id(name) {
                    let local = Json::Array(vec![row.remove(id).unwrap_or(Json::Null)]);
                    (format!("#{}", digest(&Json::Object(row.clone()))), local)
                } else {
                    let local: Vec<Json> = keys.iter().map(|k| row.get(*k).cloned().unwrap_or(Json::Null)).collect();
                    (record_key(name, &local), Json::Array(local))
                };
                if *name == "grants" {
                    let parsed = row
                        .remove("grant_json")
                        .and_then(|g| g.as_str().and_then(|s| serde_json::from_str::<Json>(s).ok()))
                        .ok_or_else(|| bad("a grant"))?;
                    row.insert("grant".to_owned(), parsed);
                }
                rows.push(SyncRow {
                    table: *table,
                    key,
                    value: Json::Object(row),
                    local: local.to_string(),
                });
            }
        }
        let mut stmt = conn.prepare("SELECT key, value FROM meta")?;
        let metas = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Value>(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        for (key, value) in metas {
            if is_local_meta(&key) {
                continue;
            }
            let Some(value) = self.meta_value(&key, value) else {
                log::warn!("a setting does not open with this account's key; it is not synced");
                continue;
            };
            rows.push(SyncRow {
                table: Table::Meta,
                key: meta_record_key(&key),
                value,
                local: Json::Array(vec![Json::String(key)]).to_string(),
            });
        }
        Ok(rows)
    }

    fn meta_value(&self, key: &str, value: Value) -> Option<Json> {
        let tagged = |tag: &str, v: Json| {
            let mut m = Map::new();
            m.insert(tag.to_owned(), v);
            Json::Object(m)
        };
        Some(match value {
            Value::Text(s) => match serde_json::from_str::<Json>(&s) {
                Ok(json @ (Json::Object(_) | Json::Array(_))) if !is_tagged(&json) => json,
                _ => tagged("$text", Json::String(s)),
            },
            Value::Integer(i) => tagged("$int", Json::from(i)),
            Value::Real(f) => tagged("$text", Json::String(f.to_string())),
            Value::Null => tagged("$text", Json::String(String::new())),
            Value::Blob(b) => match SEALED_META.iter().find(|(k, _)| *k == key) {
                Some((_, aad)) => b64(&self.unseal(aad, &b).ok()?),
                None => b64(&b),
            },
        })
    }

    /// The Activity entries after the local `seq`, oldest first, at most `limit`: their seq and record value.
    pub(crate) fn audit_rows_after(&self, seq: i64, limit: usize) -> Result<Vec<(i64, Json)>, CoreError> {
        let conn = self.lock()?;
        let filter = format!(" WHERE seq > {seq} ORDER BY seq LIMIT {}", i64::try_from(limit).unwrap_or(i64::MAX));
        Ok(self
            .plain_rows(&conn, "audit", &filter)?
            .into_iter()
            .filter_map(|mut row| {
                let seq = row.remove("seq")?.as_i64()?;
                Some((seq, Json::Object(row)))
            })
            .collect())
    }

    /// The highest local Activity seq (0: none).
    pub(crate) fn audit_head(&self) -> Result<i64, CoreError> {
        Ok(self.lock()?.query_row("SELECT coalesce(max(seq), 0) FROM audit", [], |r| r.get(0))?)
    }

    /// Writes a record into its table (`None`: removes the row `local` names). Sealed columns are sealed again with this
    /// account's key. Refuses this installation's own rows (a device key, sign-in state), whatever the record says.
    /// Returns how the store finds the row now (for a log table, its new local id).
    pub(crate) fn write_sync_row(
        &self,
        table: Table,
        key: &str,
        local: Option<&str>,
        value: Option<&Json>,
    ) -> Result<Option<String>, CoreError> {
        let conn = self.lock()?;
        self.write_row_in(&conn, table, key, local, value, false)
    }

    /// Adds `rows` (new rows only: one whose key exists refuses all), all of them or none.
    pub(crate) fn insert_sync_rows(&self, rows: &[(Table, Json)]) -> Result<(), CoreError> {
        let conn = self.lock()?;
        conn.execute_batch("SAVEPOINT sync_rows")?;
        let written = rows.iter().try_for_each(|(table, value)| {
            let key = key_of_row(*table, value).ok_or_else(|| bad("a row without its key"))?;
            self.write_row_in(&conn, *table, &key, None, Some(value), true).map(drop)
        });
        match written {
            Ok(()) => conn.execute_batch("RELEASE sync_rows")?,
            Err(e) => {
                conn.execute_batch("ROLLBACK TO sync_rows; RELEASE sync_rows")?;
                return Err(e);
            }
        }
        Ok(())
    }

    /// How a record from a start this phone could not verify (a checkpoint, or the history before a recovery, which a
    /// thief holding the old primary could have written) is taken: nothing of it loosens anything until the user sets
    /// it again.
    /// - a grant revoked; a mode Manual (Lockdown stays; one that follows the global stays so), no bypass;
    /// - an Autopilot profile back to its defaults (its name kept); its memory and approved targets not taken;
    /// - a paired computer's key not taken (it pairs again); an MCP server's tools to be listed again, after the user
    ///   checks its address (it waits for a sign-in);
    /// - "always ask" asking everywhere; the starting rule asking every time; every connection new to Autopilot;
    /// - the payment settings back to asking for everything (no spend limit, no tolerance, no default method or
    ///   address, no card switched on); the payment ledger and tokens not taken (this phone's own stay);
    /// - the desktop secrets seen never fewer than this phone already saw.
    pub(crate) fn tighten_record(
        &self,
        table: Table,
        key: &str,
        value: &Json,
        now: i64,
    ) -> Result<Tightened, CoreError> {
        let mut v = value.clone();
        let set = |v: &mut Json, k: &str, x: Json| {
            if let Some(obj) = v.as_object_mut() {
                obj.insert(k.to_owned(), x);
            }
        };
        Ok(match table {
            Table::Grants => {
                if let Some(grant) = v.get_mut("grant") {
                    set(grant, "revoked", Json::Bool(true));
                }
                Tightened::Write(v)
            }
            Table::AutopilotSettings => {
                let loosens = v.get("mode").and_then(Json::as_str).is_some_and(|m| m != "lockdown");
                if loosens {
                    set(&mut v, "mode", Json::from("manual"));
                }
                set(&mut v, "bypass_until", Json::Null);
                Tightened::Write(v)
            }
            Table::Profiles => {
                let profile = v
                    .get("data")
                    .and_then(bytes_of)
                    .and_then(|raw| serde_json::from_slice::<crate::autopilot::store::Profile>(&raw).ok());
                let Some(profile) = profile else {
                    return Ok(Tightened::Skip);
                };
                let reset = crate::autopilot::store::Profile {
                    name: profile.name,
                    icon: profile.icon,
                    ..crate::autopilot::store::Profile::default()
                };
                set(&mut v, "data", b64(&serde_json::to_vec(&reset).map_err(|e| CoreError::storage(e.to_string()))?));
                Tightened::Write(v)
            }
            Table::AutopilotMemory | Table::Targets | Table::DesktopKeys => Tightened::Skip,
            Table::Secrets => Tightened::KeepLocal,
            Table::McpServers => {
                for (column, x) in [("tools", Json::from("[]")), ("heavy", Json::from("[]")), ("listed_at", Json::Null)]
                {
                    set(&mut v, column, x);
                }
                set(&mut v, "status", Json::from("needs_sign_in"));
                set(&mut v, "error", Json::from("Check this server's address, then refresh it."));
                Tightened::Write(v)
            }
            Table::Meta => match key {
                "secrets.always-ask" => {
                    if let Some(obj) = v.as_object_mut() {
                        for x in obj.values_mut() {
                            *x = Json::Bool(true);
                        }
                    }
                    Tightened::Write(v)
                }
                "starting_policy" => Tightened::Write(serde_json::json!({"$text": "ask_every_time"})),
                "payments.config" => {
                    for (k, x) in [
                        ("limits", serde_json::json!([])),
                        ("tolerance_pct", Json::from(0)),
                        ("default_method", Json::Null),
                        ("default_address", Json::Null),
                        ("cards", serde_json::json!([])),
                        ("pay_on_phone", Json::Bool(false)),
                        ("merchant_account", Json::Bool(false)),
                    ] {
                        set(&mut v, k, x);
                    }
                    Tightened::Write(v)
                }
                "payments.ledger" => Tightened::KeepLocal,
                "vault.secret-store.seen" => {
                    let mine =
                        self.meta_get("vault.secret-store.seen")?.and_then(|s| serde_json::from_str::<Json>(&s).ok());
                    Tightened::Write(union(mine, v))
                }
                k if k.starts_with("~autopilot_created:") => {
                    Tightened::Write(serde_json::json!({"$text": now.to_string()}))
                }
                _ => Tightened::Write(v),
            },
            _ => Tightened::Write(v),
        })
    }

    fn write_row_in(
        &self,
        conn: &Connection,
        table: Table,
        key: &str,
        local: Option<&str>,
        value: Option<&Json>,
        insert_only: bool,
    ) -> Result<Option<String>, CoreError> {
        let name = sql_table(table).ok_or_else(|| bad("a table the sync does not carry"))?;
        if key.starts_with(RESERVED_PREFIX) {
            return Err(bad("a reserved key"));
        }
        if table == Table::Meta {
            return self.write_meta(conn, key, value);
        }
        let Some(value) = value else {
            let Some(local) = local else {
                return Ok(None);
            };
            let parts: Vec<Json> = serde_json::from_str(local).map_err(|_| bad("a row's key"))?;
            let columns = match log_id(name) {
                Some(id) => vec![id],
                None => key_columns(name).to_vec(),
            };
            if columns.len() != parts.len() || columns.is_empty() {
                return Err(bad("a row's key"));
            }
            if name == "secrets" && parts.first().and_then(Json::as_str).is_some_and(|s| DEVICE_SECRETS.contains(&s)) {
                return Err(bad("an installation key"));
            }
            let clause = columns.iter().map(|c| format!("{c} = ?")).collect::<Vec<_>>().join(" AND ");
            let cells = parts.iter().map(json_cell).collect::<Result<Vec<_>, _>>()?;
            conn.execute(&format!("DELETE FROM {name} WHERE {clause}"), params_from_iter(cells))?;
            return Ok(None);
        };
        let mut row = value.as_object().cloned().ok_or_else(|| bad("a row"))?;
        // A record that does not carry its key column (built by a rule from nothing) takes it from its key.
        if let [column] = key_columns(name)
            && !row.contains_key(*column)
            && let Some(parts) = local_of(table, key).and_then(|l| serde_json::from_str::<Vec<Json>>(&l).ok())
            && let Some(first) = parts.into_iter().next()
        {
            row.insert((*column).to_owned(), first);
        }
        if name == "autopilot_settings" && !row.contains_key("updated_at") {
            row.insert("updated_at".to_owned(), Json::from(crate::store::unix_now()));
        }
        if name == "grants" {
            let grant = row.remove("grant").ok_or_else(|| bad("a grant"))?;
            serde_json::from_value::<reins_policy::Grant>(grant.clone()).map_err(|_| bad("a grant"))?;
            row.insert("grant_json".to_owned(), Json::String(grant.to_string()));
        }
        if name == "secrets" && row.get("service").and_then(Json::as_str).is_some_and(|s| DEVICE_SECRETS.contains(&s)) {
            return Err(bad("an installation key"));
        }
        let actual: Vec<String> = conn
            .prepare(&format!("SELECT * FROM {name} LIMIT 0"))?
            .column_names()
            .iter()
            .map(|c| (*c).to_owned())
            .collect();
        let id = log_id(name);
        let mut columns = Vec::new();
        let mut cells = Vec::new();
        for (column, v) in &row {
            if Some(column.as_str()) == id {
                continue;
            }
            if !actual.contains(column) && v.is_null() {
                // Something the op-log rules note (a stopped bypass's start), not a column here.
                continue;
            }
            if !actual.contains(column) {
                return Err(CoreError::storage(format!(
                    "a synced {name} row has a column this version lacks: {column}"
                )));
            }
            let cell = match column_aad(name, column, &row) {
                Some(aad) if !v.is_null() => {
                    Value::Blob(self.seal(&aad, &bytes_of(v).ok_or_else(|| bad("a sealed column"))?)?)
                }
                _ => json_cell(v)?,
            };
            columns.push(column.clone());
            cells.push(cell);
        }
        if columns.is_empty() {
            return Err(bad("a row"));
        }
        let placeholders = vec!["?"; columns.len()].join(",");
        if let Some(id) = id {
            // A log entry: added once (with this phone's own number), never changed.
            conn.execute(
                &format!("INSERT INTO {name} ({}) VALUES ({placeholders})", columns.join(",")),
                params_from_iter(cells),
            )?;
            let new: i64 = conn.query_row("SELECT last_insert_rowid()", [], |r| r.get(0))?;
            let _ = id;
            return Ok(Some(Json::Array(vec![Json::from(new)]).to_string()));
        }
        let keys = key_columns(name);
        let updates = columns
            .iter()
            .filter(|c| !keys.contains(&c.as_str()))
            .map(|c| format!("{c} = excluded.{c}"))
            .collect::<Vec<_>>();
        let conflict = if insert_only {
            // A row that exists already refuses the write (the key's uniqueness fails it).
            String::new()
        } else if updates.is_empty() {
            format!("ON CONFLICT ({}) DO NOTHING", keys.join(","))
        } else {
            format!("ON CONFLICT ({}) DO UPDATE SET {}", keys.join(","), updates.join(","))
        };
        conn.execute(
            &format!("INSERT INTO {name} ({}) VALUES ({placeholders}) {conflict}", columns.join(",")),
            params_from_iter(cells),
        )?;
        let local: Vec<Json> = keys.iter().map(|k| row.get(*k).cloned().unwrap_or(Json::Null)).collect();
        if record_key(name, &local) != key {
            log::warn!("a synced {name} row is filed under another key than its columns give");
        }
        Ok(Some(Json::Array(local).to_string()))
    }

    fn write_meta(&self, conn: &Connection, key: &str, value: Option<&Json>) -> Result<Option<String>, CoreError> {
        let store_key = meta_store_key(key);
        if is_local_meta(&store_key) {
            return Err(bad("this installation's own setting"));
        }
        let local = Json::Array(vec![Json::String(store_key.clone())]).to_string();
        let Some(value) = value else {
            conn.execute("DELETE FROM meta WHERE key = ?1", [&store_key])?;
            return Ok(Some(local));
        };
        let cell = match value.as_object().filter(|m| m.len() == 1) {
            Some(m) if m.contains_key("$text") => {
                Value::Text(m["$text"].as_str().ok_or_else(|| bad("a setting"))?.to_owned())
            }
            Some(m) if m.contains_key("$int") => Value::Integer(m["$int"].as_i64().ok_or_else(|| bad("a setting"))?),
            Some(m) if m.contains_key("$b64") => {
                let raw = bytes_of(value).ok_or_else(|| bad("a setting"))?;
                match SEALED_META.iter().find(|(k, _)| *k == store_key) {
                    Some((_, aad)) => Value::Blob(self.seal(aad, &raw)?),
                    None => Value::Blob(raw),
                }
            }
            _ => Value::Text(value.to_string()),
        };
        conn.execute(
            "INSERT INTO meta (key, value) VALUES (?1, ?2) ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            rusqlite::params![store_key, cell],
        )?;
        Ok(Some(local))
    }

    /// `value` as this store keeps it (what [`Self::write_sync_row`] leaves out, dropped): what the mirror digests, so
    /// that a record the op-log rules annotated is not taken for a change of this phone's.
    pub(crate) fn normalized(&self, table: Table, value: &Json) -> Result<Json, CoreError> {
        let (Some(name), Some(map)) = (sql_table(table), value.as_object()) else {
            return Ok(value.clone());
        };
        if table == Table::Meta {
            return Ok(value.clone());
        }
        let actual: Vec<String> = self
            .lock()?
            .prepare(&format!("SELECT * FROM {name} LIMIT 0"))?
            .column_names()
            .iter()
            .map(|c| (*c).to_owned())
            .collect();
        Ok(Json::Object(
            map.iter()
                .filter(|(k, v)| actual.contains(k) || (name == "grants" && k.as_str() == "grant") || !v.is_null())
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        ))
    }

    /// Adds an Activity entry the log brought (with this phone's own seq).
    pub(crate) fn insert_sync_audit(&self, value: &Json) -> Result<i64, CoreError> {
        let local = self.write_sync_row(Table::Audit, "activity", None, Some(value))?;
        local
            .and_then(|l| serde_json::from_str::<Vec<i64>>(&l).ok())
            .and_then(|v| v.first().copied())
            .ok_or_else(|| CoreError::storage("no seq for the new Activity entry"))
    }

    /// Forgets every integration token and the payment mandate key this phone holds for the account (a viewer
    /// holds none: the primary opens them). This installation's own keys stay.
    pub(crate) fn wipe_tokens(&self) -> Result<usize, CoreError> {
        let conn = self.lock()?;
        Ok(conn.execute(&format!("DELETE FROM secrets WHERE service NOT IN ({})", quoted(DEVICE_SECRETS)), [])?)
    }
}

fn is_tagged(json: &Json) -> bool {
    json.as_object().is_some_and(|m| m.len() == 1 && m.keys().all(|k| k.starts_with('$')))
}

fn quoted(items: &[&str]) -> String {
    items.iter().map(|s| format!("'{s}'")).collect::<Vec<_>>().join(",")
}

#[cfg(test)]
mod tests;
