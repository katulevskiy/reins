//! How one opened, authorized change lands in its record (the design's per-table rules, `merge.rs`'s lattice):
//!
//! | Table | A put that saw the record (`prev` is its version) | A put that did not (concurrent, late) | A removal |
//! |---|---|---|---|
//! | `grants` | replaces it | [`join_grants`]: revoked wins, the earlier expiry and smaller limit, a scope change revokes, the most uses | a tombstone a put never lifts (ids are not reused) |
//! | `autopilot_settings` | replaces it | Lockdown wins, else the less permissive mode; the earlier bypass end, a stopped one stays stopped | a tombstone |
//! | `meta` | replaces it | merged entry by entry, "ask every time" wins; a value of another kind keeps the stored one | a tombstone |
//! | `audit` | keyed by the op that writes it ([`activity_key`]), so it is written once: never changed, never back once trimmed | kept | ignored (grow-only; the newest entries kept as the log goes) |
//! | the others | replaces it | the later op's (last writer in `seq` order) | a tombstone |
//!
//! A put on a tombstone lifts it only when it names the tombstone's version (its writer saw the removal and adds the
//! record again on purpose); otherwise the removal beats the concurrent edit. A put on a record this phone does not
//! have is a new record: never seen, or a tombstone dropped long after the removal, which happens only to records whose
//! removal is no security decision (grants, accounts, secrets, MCP servers, desktop keys and targets keep theirs).
//!
//! Restrictive ops ([`Restriction`]) only tighten, whoever sends them, and take the record's version, so the
//! primary's next put of it is a concurrent one (joined) unless it saw the restriction: revoke one grant, Lockdown
//! (the global setting), stop every running bypass. Proposals live in their own table.

use reins_policy::Grant;
use reins_proto::oplog::{Change, Op, Restriction, Table};
use rusqlite::{OptionalExtension, Transaction, params};
use serde_json::Value as Json;

use super::{Outcome, Record, RecordPut, Refusal, Version, as_i64, record, records, table_name};
use crate::CoreError;
use crate::store::lattice::{ALWAYS_ASK_KEY, join_bypass_until, join_grants, permissiveness};

/// The global Autopilot setting's key (the store's scope `""`; record keys are never empty).
pub const GLOBAL_SCOPE: &str = "*";

fn write(
    tx: &Transaction<'_>,
    table: Table,
    key: &str,
    value: Option<&Json>,
    version: &Version,
    seq: u64,
    now: i64,
) -> Result<(), CoreError> {
    let value = value.map(serde_json::to_string).transpose().map_err(|e| CoreError::storage(e.to_string()))?;
    tx.execute(
        "INSERT INTO oplog_records (tbl, key, value, epoch, author, counter, seq, at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT (tbl, key) DO UPDATE SET value = ?3, epoch = ?4, author = ?5, counter = ?6, seq = ?7, at = ?8",
        params![
            table_name(table),
            key,
            value,
            as_i64(version.epoch),
            version.author,
            as_i64(version.counter),
            as_i64(seq),
            now
        ],
    )?;
    Ok(())
}

pub(super) fn apply_change(
    tx: &Transaction<'_>,
    op: &Op,
    change: &Change,
    seq: u64,
    now: i64,
) -> Result<Outcome, CoreError> {
    let version = Version::of(op);
    match change {
        Change::Put {
            table: Table::Proposals,
            key,
            value,
        } => propose(tx, op, key, value, seq, now),
        Change::Remove {
            table: Table::Proposals,
            key,
        } => resolve(tx, key, seq, now),
        Change::Put {
            table,
            key,
            value,
        } => {
            let Ok(put) = serde_json::from_value::<RecordPut>(value.clone()) else {
                return Ok(Outcome::Refused(Refusal::Invalid("a put carries {prev, row}".to_owned())));
            };
            put_record(tx, *table, key, put, &version, seq, now)
        }
        Change::Remove {
            table: Table::Audit,
            ..
        } => Ok(Outcome::Superseded),
        Change::Remove {
            table: Table::Meta,
            key,
        } if key == crate::store::authorization::RECORD_KEY
            || key.starts_with(crate::store::permissions::RECEIPT_PREFIX) =>
        {
            let current = record(tx, Table::Meta, key)?.and_then(|r| r.value);
            let held = if key.starts_with(crate::store::permissions::RECEIPT_PREFIX) {
                crate::store::permissions::receipt_incoming(current.as_ref(), None)
            } else {
                crate::store::authorization::incoming(current.as_ref(), None)
            };
            write(tx, Table::Meta, key, Some(&held), &version, seq, now)?;
            Ok(Outcome::Applied)
        }
        Change::Remove {
            table,
            key,
        } => {
            write(tx, *table, key, None, &version, seq, now)?;
            Ok(Outcome::Applied)
        }
        Change::Restrict(r) => restrict(tx, r, &version, seq, now),
    }
}

/// The key of the Activity entry the op of `version` writes. An entry is its op's own: no other op (whoever writes it)
/// can write that key, and its op is never applied twice (its counter), so an entry is never changed, nor written again
/// once trimmed.
pub fn activity_key(version: &Version) -> String {
    format!("{}/{}/{}", version.epoch, version.author, version.counter)
}

fn put_record(
    tx: &Transaction<'_>,
    table: Table,
    key: &str,
    put: RecordPut,
    version: &Version,
    seq: u64,
    now: i64,
) -> Result<Outcome, CoreError> {
    if table == Table::Audit && key != activity_key(version) {
        return Ok(Outcome::Refused(Refusal::Invalid(
            "an Activity entry is keyed by the op that writes it".to_owned(),
        )));
    }
    let current = record(tx, table, key)?;
    // An Activity entry never changes, whoever writes it and whatever it names as replaced.
    if table == Table::Audit && current.is_some() {
        return Ok(Outcome::Superseded);
    }
    let saw = |r: &Record| put.prev.as_ref() == Some(&r.version);
    let row = match current {
        None => put.row,
        Some(r) if r.value.is_none() => {
            // A tombstone: lifted only by a writer that saw it (and never for a grant).
            if table == Table::Grants || !saw(&r) {
                return Ok(Outcome::Superseded);
            }
            put.row
        }
        Some(r) if saw(&r) => {
            if table == Table::Meta && key.starts_with(crate::store::permissions::RECEIPT_PREFIX) {
                crate::store::permissions::receipt_incoming(r.value.as_ref(), Some(&put.row))
            } else if table == Table::Meta && key == crate::store::authorization::RECORD_KEY {
                crate::store::authorization::incoming(r.value.as_ref(), Some(&put.row))
            } else {
                put.row
            }
        }
        Some(r) => {
            let live = r.value.unwrap_or(Json::Null);
            match join(table, key, &live, put.row) {
                Some(joined) => joined,
                None => return Ok(Outcome::Superseded),
            }
        }
    };
    write(tx, table, key, Some(&row), version, seq, now)?;
    Ok(Outcome::Applied)
}

/// A concurrent put met `live`: the table's join (`None`: `live` stays, as it is).
fn join(table: Table, key: &str, live: &Json, newer: Json) -> Option<Json> {
    match table {
        Table::Grants => {
            let (Some(a), Some(b)) = (grant_of(&newer), grant_of(live)) else {
                return None;
            };
            with_grant(newer, &join_grants(&a, &b, None))
        }
        Table::AutopilotSettings => Some(join_settings(live, newer)),
        // Meta records are keyed as the app reads them (without the store's `app.` prefix).
        // Two objects or two lists merge entry by entry; two values of one other kind, the later op's; values of
        // different kinds (a stale `false` against `{"a": true}`) keep the stored one.
        Table::Meta if key.starts_with(crate::store::permissions::RECEIPT_PREFIX) => {
            Some(crate::store::permissions::receipt_incoming(Some(live), Some(&newer)))
        }
        Table::Meta if key == crate::store::authorization::RECORD_KEY => {
            Some(crate::store::authorization::conflict(live, &newer))
        }
        Table::Meta => {
            crate::store::lattice::merge_json(None, live, &newer, Some(key) == ALWAYS_ASK_KEY.strip_prefix("app."))
                .or_else(|| same_kind(live, &newer).then_some(newer))
        }
        Table::Audit => None,
        _ => Some(newer),
    }
}

fn same_kind(a: &Json, b: &Json) -> bool {
    std::mem::discriminant(a) == std::mem::discriminant(b)
}

/// A grant row: an object with the grant under `"grant"`.
fn grant_of(row: &Json) -> Option<Grant> {
    serde_json::from_value(row.get("grant")?.clone()).ok()
}

fn with_grant(mut row: Json, grant: &Grant) -> Option<Json> {
    row.as_object_mut()?.insert("grant".to_owned(), serde_json::to_value(grant).ok()?);
    Some(row)
}

fn mode_of(row: &Json) -> Option<&str> {
    row.get("mode").and_then(Json::as_str)
}

fn until_of(row: &Json) -> Option<i64> {
    row.get("bypass_until").and_then(Json::as_i64)
}

/// Two versions of a setting joined: the less permissive one's fields, and the earlier bypass end (stopped on either
/// side: stopped).
fn join_settings(live: &Json, newer: Json) -> Json {
    let ends = join_bypass_until(until_of(live), until_of(&newer));
    let mut row = if permissiveness(mode_of(live)) < permissiveness(mode_of(&newer)) {
        live.clone()
    } else {
        newer
    };
    if let Some(obj) = row.as_object_mut() {
        obj.insert("bypass_until".to_owned(), ends.map_or(Json::Null, Json::from));
        if ends.is_none() {
            obj.insert("bypass_since".to_owned(), Json::Null);
        }
    }
    row
}

fn restrict(
    tx: &Transaction<'_>,
    restriction: &Restriction,
    version: &Version,
    seq: u64,
    now: i64,
) -> Result<Outcome, CoreError> {
    match restriction {
        Restriction::RevokeGrant {
            grant_id,
        } => {
            let Some(Record {
                value: Some(row),
                ..
            }) = record(tx, Table::Grants, grant_id)?
            else {
                return Ok(Outcome::Superseded);
            };
            let Some(mut grant) = grant_of(&row) else {
                return Ok(Outcome::Superseded);
            };
            grant.revoked = true;
            let row = with_grant(row, &grant).ok_or_else(|| CoreError::storage("invalid grant row"))?;
            write(tx, Table::Grants, grant_id, Some(&row), version, seq, now)?;
        }
        Restriction::Lockdown => {
            let live = record(tx, Table::AutopilotSettings, GLOBAL_SCOPE)?.and_then(|r| r.value);
            // No global setting yet (an account left on Manual), or one removed: a whole row, as the store keeps it.
            let mut row = live.filter(Json::is_object).unwrap_or_else(|| serde_json::json!({"scope": ""}));
            if let Some(obj) = row.as_object_mut() {
                obj.insert("mode".to_owned(), Json::from("lockdown"));
                obj.insert("bypass_until".to_owned(), Json::Null);
                obj.insert("bypass_since".to_owned(), Json::Null);
            }
            write(tx, Table::AutopilotSettings, GLOBAL_SCOPE, Some(&row), version, seq, now)?;
        }
        Restriction::StopBypass => {
            for r in records(tx, Table::AutopilotSettings)? {
                let Some(mut row) = r.value else {
                    continue;
                };
                if until_of(&row).is_none() {
                    continue;
                }
                if let Some(obj) = row.as_object_mut() {
                    obj.insert("bypass_until".to_owned(), Json::Null);
                    obj.insert("bypass_since".to_owned(), Json::Null);
                }
                write(tx, Table::AutopilotSettings, &r.key, Some(&row), version, seq, now)?;
            }
        }
    }
    Ok(Outcome::Applied)
}

/// A viewer's proposal (its own key, `check_change` saw to that): added, or edited while it waits; never brought back
/// once resolved or withdrawn.
fn propose(tx: &Transaction<'_>, op: &Op, key: &str, value: &Json, seq: u64, now: i64) -> Result<Outcome, CoreError> {
    let resolved: Option<bool> = tx
        .query_row("SELECT value IS NULL FROM oplog_proposals WHERE key = ?1", params![key], |r| r.get(0))
        .optional()?;
    if resolved == Some(true) {
        return Ok(Outcome::Superseded);
    }
    let value = serde_json::to_string(value).map_err(|e| CoreError::storage(e.to_string()))?;
    tx.execute(
        "INSERT INTO oplog_proposals (key, author, value, seq, received_at) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (key) DO UPDATE SET value = ?3, seq = ?4",
        params![key, op.author, value, as_i64(seq), now],
    )?;
    Ok(Outcome::Applied)
}

/// The primary resolved a proposal (kept or discarded), or its author withdrew it.
fn resolve(tx: &Transaction<'_>, key: &str, seq: u64, now: i64) -> Result<Outcome, CoreError> {
    let author = key.split('/').next().unwrap_or_default();
    tx.execute(
        "INSERT INTO oplog_proposals (key, author, value, seq, received_at) VALUES (?1, ?2, NULL, ?3, ?4)
             ON CONFLICT (key) DO UPDATE SET value = NULL, seq = ?3",
        params![key, author, as_i64(seq), now],
    )?;
    Ok(Outcome::Applied)
}
