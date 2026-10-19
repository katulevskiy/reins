//! Three-way merge of portable account snapshots: the state both phones last agreed on (the base), this phone's,
//! and the server's. What the snapshots carry and how each table merges:
//!
//! | table | merge |
//! |---|---|
//! | `grants` | security: never more permissive than either side or the base (see [`merge_grant`]); deleted on either side stays deleted |
//! | `autopilot_settings` | changed on one side: that side's (a cleared lockdown or a stopped bypass stays so); on both, security: lockdown wins, else the less permissive mode, the earlier bypass end, one stopped stays stopped; clocks never decide |
//! | `audit` (Activity) | grow-only: the union of both phones' entries, trimmed to the newest [`AUDIT_RETENTION`]; never a conflict |
//! | `autopilot_memory` | entries added on either side are kept; one forgotten on either side goes |
//! | `meta` | per key; tombstones too (kept on either side when there is no base); JSON settings merge entry by entry (always-ask: asking wins) |
//! | `accounts`, `secrets`, `desktop_keys`, `mcp_servers`, `connection_prefs`, `autopilot_profiles`, `autopilot_targets` | per row: a change on one side wins; on both, deleted beats edited, else the server's |
//!
//! Removing a guarded record ([`super::GUARDED`]: grants, accounts, tokens, paired computers' keys, MCP servers,
//! approved targets) leaves a synced tombstone; [`bury`] drops every guarded record that has one, from every merge
//! and every snapshot read (the server's, a base, a backup). Tombstones last [`super::TOMBSTONE_HORIZON_SECS`]; a base
//! older than that is not used (`sync.rs`).
//!
//! Not synced at all (this phone's own, `snapshot.rs`): `session`, `pending`, `handled`, `autopilot_suggestions`,
//! `autopilot_rate`, installation keys and metadata, and `audit_snapshots`.
//!
//! Without a base (an older version kept only a hash of it, it was lost, or it is older than the tombstone horizon)
//! the server's version wins: what only this phone has is dropped (kept in a backup), grants and modes still only
//! tighten, and logs keep both phones' entries. [`Merged::dropped_local`] then says whether anything was lost.
use super::AUDIT_RETENTION;
use super::snapshot::{Cell, Snapshot, Table, identity};
use reins_policy::Grant;
use serde_json::Value as Json;
use std::collections::{BTreeMap, BTreeSet};

pub(super) struct Merged {
    pub snapshot: Snapshot,
    /// Something this phone had is not in the result although nothing says it was deleted or replaced on purpose
    /// (only without a base).
    pub dropped_local: bool,
}

type Row = Vec<Cell>;

/// How a table's rows are told apart.
enum Kind {
    /// By these primary key columns.
    Keyed(&'static [&'static str]),
    /// A log numbered by this column, which each phone counts on its own: rows are told apart by their content.
    /// `grow_only`: nothing is ever removed by a merge (Activity); otherwise a removal on either side carries.
    Log(&'static str, bool),
}

/// Every portable table needs a rule here (a test checks): one without is merged as a whole.
fn kind(table: &str) -> Option<Kind> {
    Some(match table {
        "meta" | "autopilot_targets" => Kind::Keyed(&["key"]),
        "grants" | "mcp_servers" | "autopilot_profiles" => Kind::Keyed(&["id"]),
        "connection_prefs" | "desktop_keys" => Kind::Keyed(&["connection_id"]),
        "accounts" | "secrets" => Kind::Keyed(&["service", "account"]),
        "autopilot_settings" => Kind::Keyed(&["scope"]),
        "audit" => Kind::Log("seq", true),
        "autopilot_memory" => Kind::Log("id", false),
        _ => return None,
    })
}

/// `now` (unix seconds) only decides which tombstones are past [`super::TOMBSTONE_HORIZON_SECS`] and go.
pub(super) fn merge(base: Option<&Snapshot>, local: &Snapshot, server: &Snapshot, now: i64) -> Merged {
    let mut dropped_local = false;
    let mut tables = BTreeMap::new();
    let names: BTreeSet<&String> = local.tables.keys().chain(server.tables.keys()).collect();
    for name in names {
        let empty = |t: Option<&Table>| Table {
            columns: t.map(|t| t.columns.clone()).unwrap_or_default(),
            rows: Vec::new(),
        };
        let s = server.tables.get(name).cloned().unwrap_or_else(|| empty(local.tables.get(name)));
        let l = local.tables.get(name).cloned().unwrap_or_else(|| empty(Some(&s)));
        let b = base.map(|b| b.tables.get(name).cloned().unwrap_or_else(|| empty(Some(&s))));
        let merged = if l.columns != s.columns || b.as_ref().is_some_and(|b| b.columns != s.columns) {
            // Snapshots are brought to this version's columns before merging; anything else cannot be compared.
            dropped_local |= l.rows != s.rows;
            s
        } else {
            match kind(name) {
                Some(Kind::Keyed(key)) => keyed(name, key, b.as_ref(), &l, &s, &mut dropped_local),
                Some(Kind::Log(seq, grow_only)) => log(seq, grow_only, b.as_ref(), &l, &s),
                None => whole(b.as_ref(), l, s, &mut dropped_local),
            }
        };
        tables.insert(name.clone(), merged);
    }
    let mut snapshot = Snapshot {
        version: 1,
        tables,
    };
    prune_tombstones(&mut snapshot, now);
    bury(&mut snapshot);
    Merged {
        snapshot,
        dropped_local,
    }
}

/// The tombstone key of a guarded record (as the store's triggers write it), if `table` is guarded.
fn tombstone_of(table: &str, columns: &[String], row: &Row) -> Option<String> {
    let (_, key) = super::GUARDED.iter().find(|(t, _)| *t == table)?;
    let parts: Vec<String> = key
        .iter()
        .map(|k| match cell(columns, row, k) {
            Some(Cell::Text(v)) => v.clone(),
            Some(Cell::Integer(v)) => v.to_string(),
            _ => String::new(),
        })
        .collect();
    Some(format!("{}{table}:{}", super::TOMBSTONE_PREFIX, serde_json::to_string(&parts).ok()?))
}

fn tombstones(snapshot: &Snapshot) -> BTreeSet<String> {
    let Some(meta) = snapshot.tables.get("meta") else {
        return BTreeSet::new();
    };
    let Some(key) = meta.column("key") else {
        return BTreeSet::new();
    };
    meta.rows
        .iter()
        .filter_map(|r| match r.get(key) {
            Some(Cell::Text(k)) if k.starts_with(super::TOMBSTONE_PREFIX) => Some(k.clone()),
            _ => None,
        })
        .collect()
}

/// Drops every guarded record that has a tombstone: removed on some phone, it stays removed, whatever any snapshot
/// (a merge's side, a base, a backup, the server's) still holds.
pub(super) fn bury(snapshot: &mut Snapshot) {
    let buried = tombstones(snapshot);
    if buried.is_empty() {
        return;
    }
    for (name, table) in &mut snapshot.tables {
        let columns = table.columns.clone();
        table.rows.retain(|r| tombstone_of(name, &columns, r).is_none_or(|t| !buried.contains(&t)));
    }
}

/// Tombstones older than the horizon go: a phone whose base is older than that keeps only what the server has
/// (`sync.rs`), so nothing they guarded can come back through it.
fn prune_tombstones(snapshot: &mut Snapshot, now: i64) {
    let Some(meta) = snapshot.tables.get_mut("meta") else {
        return;
    };
    let (Some(key), Some(value)) = (meta.column("key"), meta.column("value")) else {
        return;
    };
    let cutoff = now.saturating_sub(super::TOMBSTONE_HORIZON_SECS);
    meta.rows.retain(|r| {
        let stone = matches!(r.get(key), Some(Cell::Text(k)) if k.starts_with(super::TOMBSTONE_PREFIX));
        !stone || !matches!(r.get(value), Some(Cell::Integer(at)) if *at < cutoff)
    });
}

fn whole(base: Option<&Table>, local: Table, server: Table, dropped_local: &mut bool) -> Table {
    match base {
        Some(b) if *b == server => local,
        Some(b) if *b == local => server,
        _ => {
            *dropped_local |= local.rows != server.rows;
            server
        }
    }
}

fn rows_by_key(table: &Table, key: &[usize]) -> BTreeMap<String, Row> {
    table
        .rows
        .iter()
        .map(|row| {
            let k: Vec<Option<&Cell>> = key.iter().map(|i| row.get(*i)).collect();
            (serde_json::to_string(&k).unwrap_or_default(), row.clone())
        })
        .collect()
}

/// The three versions of one record: the base's (`has_base`: there is a base snapshot at all), this phone's and the
/// server's.
struct Versions<'a> {
    has_base: bool,
    base: Option<&'a Row>,
    local: Option<&'a Row>,
    server: Option<&'a Row>,
}

fn keyed(
    name: &str,
    key: &[&str],
    base: Option<&Table>,
    local: &Table,
    server: &Table,
    dropped_local: &mut bool,
) -> Table {
    let Some(key) = key.iter().map(|k| server.column(k)).collect::<Option<Vec<_>>>() else {
        return whole(base, local.clone(), server.clone(), dropped_local);
    };
    let (l, s) = (rows_by_key(local, &key), rows_by_key(server, &key));
    let b = base.map(|b| rows_by_key(b, &key));
    let keys: BTreeSet<&String> = l.keys().chain(s.keys()).collect();
    let mut rows = Vec::with_capacity(keys.len());
    for k in keys {
        let v = Versions {
            has_base: b.is_some(),
            base: b.as_ref().and_then(|b| b.get(k)),
            local: l.get(k),
            server: s.get(k),
        };
        // Identical on both sides is that, except a grant: held against its base too (both may have resumed it).
        let authorization = name == "meta"
            && matches!(v.local.or(v.server).and_then(|r| cell(&server.columns, r, "key")), Some(Cell::Text(key_name)) if key_name == super::authorization::STORE_KEY);
        let receipt = name == "meta"
            && matches!(v.local.or(v.server).and_then(|r| cell(&server.columns, r, "key")), Some(Cell::Text(key_name)) if key_name.starts_with(super::permissions::RECEIPT_STORE_PREFIX));
        let picked = if receipt {
            receipt_versions(&server.columns, &v)
        } else if authorization {
            authorization_versions(&server.columns, &v)
        } else if v.local == v.server && (name != "grants" || v.base.is_none() || v.local.is_none()) {
            v.local.cloned()
        } else {
            match name {
                "grants" => grant_versions(&server.columns, &v),
                "autopilot_settings" => setting_versions(&server.columns, &v),
                // Without a base nothing tells a tombstone added on one side from one cleared on the other (by adding
                // the record again): kept. With one, the usual rules, so that adding a record again carries.
                // A grant's id is never given out again: its tombstone is never cleared, by anyone.
                "meta"
                    if (!v.has_base || is_grant_tombstone(&server.columns, v.local.or(v.server)))
                        && is_tombstone(&server.columns, v.local.or(v.server)) =>
                {
                    v.local.or(v.server).cloned()
                }
                _ => plain_versions(name, &server.columns, &v),
            }
        };
        *dropped_local |= !v.has_base && v.local.is_some() && picked.as_ref() != v.local && name != "grants";
        *dropped_local |= !v.has_base && name == "grants" && v.local.is_some() && picked.is_none();
        rows.extend(picked);
    }
    Table {
        columns: server.columns.clone(),
        rows,
    }
}

fn authorization_versions(columns: &[String], v: &Versions<'_>) -> Option<Row> {
    let json = |r: Option<&Row>| match r.and_then(|r| cell(columns, r, "value")) {
        Some(Cell::Text(text)) => serde_json::from_str::<Json>(text).ok(),
        _ => None,
    };
    let (base, local, server) = (json(v.base), json(v.local), json(v.server));
    let next = if v.local == v.server || (v.has_base && v.base == v.local) {
        super::authorization::incoming(base.as_ref(), server.as_ref())
    } else if v.has_base && v.base == v.server {
        super::authorization::incoming(base.as_ref(), local.as_ref())
    } else if v.base.is_none() && v.has_base && (v.local.is_none() || v.server.is_none()) {
        super::authorization::incoming(None, local.as_ref().or(server.as_ref()))
    } else {
        super::authorization::conflict(local.as_ref().unwrap_or(&Json::Null), server.as_ref().unwrap_or(&Json::Null))
    };
    let mut row = v.server.or(v.local).or(v.base)?.clone();
    set_cell(columns, &mut row, "value", Cell::Text(next.to_string()));
    Some(row)
}

fn receipt_versions(columns: &[String], v: &Versions<'_>) -> Option<Row> {
    let json = |row: Option<&Row>| match row.and_then(|r| cell(columns, r, "value")) {
        Some(Cell::Text(text)) => Some(serde_json::from_str::<Json>(text).unwrap_or(Json::Null)),
        _ => None,
    };
    let (local, server) = (json(v.local), json(v.server));
    let kept = super::permissions::receipt_incoming(local.as_ref(), server.as_ref());
    let mut row = v.local.or(v.server).or(v.base)?.clone();
    set_cell(columns, &mut row, "value", Cell::Text(kept.to_string()));
    Some(row)
}

fn is_grant_tombstone(columns: &[String], row: Option<&Row>) -> bool {
    matches!(row.and_then(|r| cell(columns, r, "key")), Some(Cell::Text(k)) if k.starts_with(&format!("{}grants:", super::TOMBSTONE_PREFIX)))
}

fn is_tombstone(columns: &[String], row: Option<&Row>) -> bool {
    matches!(row.and_then(|r| cell(columns, r, "key")), Some(Cell::Text(k)) if k.starts_with(super::TOMBSTONE_PREFIX))
}

/// An ordinary record: changed on one side, that side's; changed on both, deleted beats edited, else the server's.
/// Without a base, the server's.
fn plain_versions(name: &str, columns: &[String], v: &Versions<'_>) -> Option<Row> {
    if v.has_base && v.base == v.local {
        return v.server.cloned();
    }
    if v.has_base && v.base == v.server {
        return v.local.cloned();
    }
    let (Some(l), Some(s)) = (v.local, v.server) else {
        return if v.has_base && v.base.is_some() {
            None
        } else {
            v.server.cloned()
        };
    };
    if name == "meta" {
        return Some(merge_meta(columns, v.base, l, s));
    }
    Some(s.clone())
}

/// Grants are merged for security, whichever side changed: never more permissive than either side or the base.
fn grant_versions(columns: &[String], v: &Versions<'_>) -> Option<Row> {
    match (v.local, v.server) {
        (Some(l), Some(s)) => merge_grant(columns, v.base, l, s),
        // Deleted on one side: deleted. New on one side: kept, unless there is no base to tell new from deleted.
        (Some(l), None) => (v.has_base && v.base.is_none()).then(|| l.clone()),
        (None, Some(s)) => v.base.is_none().then(|| s.clone()),
        (None, None) => None,
    }
}

fn cell<'a>(columns: &[String], row: &'a Row, name: &str) -> Option<&'a Cell> {
    columns.iter().position(|c| c == name).and_then(|i| row.get(i))
}
fn set_cell(columns: &[String], row: &mut Row, name: &str, value: Cell) {
    if let Some(slot) = columns.iter().position(|c| c == name).and_then(|i| row.get_mut(i)) {
        *slot = value;
    }
}
fn grant_of(columns: &[String], row: &Row) -> Option<Grant> {
    match cell(columns, row, "grant_json") {
        Some(Cell::Text(json)) => serde_json::from_str(json).ok(),
        _ => None,
    }
}
/// Every version of one grant (the base's too): the merge is never more permissive than any of them, whatever the
/// clocks say. Revoked in any version: revoked. A scope, account or connection that differs between versions:
/// revoked (neither can be shown to be the narrower). Otherwise the earliest expiry, the smallest use limit, and every
/// use either phone counted (so the fewest uses left). A grant either version gave only while a bypass lasts keeps that
/// origin. A version that does not parse is no grant at all.
fn merge_grant(columns: &[String], base: Option<&Row>, local: &Row, server: &Row) -> Option<Row> {
    let (Some(l), Some(s)) = (grant_of(columns, local), grant_of(columns, server)) else {
        return None;
    };
    let b = match base {
        Some(row) => Some(grant_of(columns, row)?),
        None => None,
    };
    let merged = super::lattice::join_grants(&s, &l, b.as_ref());
    let mut row = server.clone();
    set_cell(columns, &mut row, "grant_json", Cell::Text(serde_json::to_string(&merged).ok()?));
    let used = [local, server]
        .iter()
        .filter_map(|r| match cell(columns, r, "last_used_at") {
            Some(Cell::Integer(at)) => Some(*at),
            _ => None,
        })
        .max();
    if let Some(at) = used {
        set_cell(columns, &mut row, "last_used_at", Cell::Integer(at));
    }
    for version in [local, server] {
        if let Some(Cell::Text(origin)) = cell(columns, version, "origin")
            && crate::autopilot::modes::temporary_scope(origin).is_some()
        {
            set_cell(columns, &mut row, "origin", Cell::Text(origin.clone()));
        }
    }
    Some(row)
}

/// How much a mode lets through without asking ([`super::lattice::permissiveness`]).
fn permissiveness(mode: Option<&Cell>) -> u8 {
    super::lattice::permissiveness(match mode {
        Some(Cell::Text(m)) => Some(m.as_str()),
        _ => None,
    })
}

/// Autopilot's mode for one scope. Changed on one side only: that side's (a lockdown cleared or a bypass stopped on one
/// phone stays so). Changed on both, merged for security: lockdown on either side wins; else the less permissive mode;
/// a bypass ends at the earlier time, and one stopped on either side stays stopped. Clocks (`updated_at`) decide
/// nothing. Deleted on one side (back to the default): deleted.
fn setting_versions(columns: &[String], v: &Versions<'_>) -> Option<Row> {
    // Changed on one side only: that side's. So a lockdown cleared, or a bypass stopped, on one phone is not brought
    // back by the other phone's unchanged copy.
    if v.has_base && v.base == v.local {
        return v.server.cloned();
    }
    if v.has_base && v.base == v.server {
        return v.local.cloned();
    }
    let (Some(l), Some(s)) = (v.local, v.server) else {
        // Deleted (back to the default) on one side: deleted. New on one side: kept, unless there is no base.
        return if v.base.is_some() || (!v.has_base && v.server.is_none()) {
            None
        } else {
            v.local.or(v.server).cloned()
        };
    };
    let mode = |r: &Row| permissiveness(cell(columns, r, "mode"));
    let mut row = if mode(l) < mode(s) {
        l.clone()
    } else {
        s.clone()
    };
    let until = |r: &Row| match cell(columns, r, "bypass_until") {
        Some(Cell::Integer(t)) => Some(*t),
        _ => None,
    };
    let ends = super::lattice::join_bypass_until(until(l), until(s)).map_or(Cell::Null, Cell::Integer);
    set_cell(columns, &mut row, "bypass_until", ends);
    let at = [l, s]
        .iter()
        .filter_map(|r| {
            if let Some(Cell::Integer(t)) = cell(columns, r, "updated_at") {
                Some(*t)
            } else {
                None
            }
        })
        .max();
    if let Some(at) = at {
        set_cell(columns, &mut row, "updated_at", Cell::Integer(at));
    }
    Some(row)
}

/// A setting kept as JSON (purchases, choices per environment, revocations to send) merges by its entries: an object
/// key by key, a list by its items' `id` (or the whole item). Asking every time wins over not asking. Any other value:
/// the server's.
fn merge_meta(columns: &[String], base: Option<&Row>, local: &Row, server: &Row) -> Row {
    let json = |row: Option<&Row>| match row.and_then(|r| cell(columns, r, "value")) {
        Some(Cell::Text(t)) => serde_json::from_str::<Json>(t).ok(),
        _ => None,
    };
    let asking_wins =
        matches!(cell(columns, server, "key"), Some(Cell::Text(k)) if k == super::lattice::ALWAYS_ASK_KEY);
    let (b, l, s) = (json(base), json(Some(local)), json(Some(server)));
    let merged = match (l, s) {
        (Some(l), Some(s)) => super::lattice::merge_json(b.as_ref(), &l, &s, asking_wins),
        _ => None,
    };
    let mut row = server.clone();
    if let Some(text) = merged.and_then(|m| serde_json::to_string(&m).ok()) {
        set_cell(columns, &mut row, "value", Cell::Text(text));
    }
    row
}

fn number(row: &Row, at: usize) -> i64 {
    if let Some(Cell::Integer(n)) = row.get(at) {
        *n
    } else {
        0
    }
}

/// Entries added on either side are all kept, the server's with their numbers and this phone's new ones after them.
/// A grow-only log (Activity) keeps everything, trimmed to its retention; another log drops what either side removed.
fn log(seq: &str, grow_only: bool, base: Option<&Table>, local: &Table, server: &Table) -> Table {
    let Some(at) = server.column(seq) else {
        return server.clone();
    };
    let ids = |t: &Table| t.rows.iter().map(|r| identity(&t.columns, r, seq)).collect::<BTreeSet<_>>();
    let (b, l, s) = (base.map(ids).unwrap_or_default(), ids(local), ids(server));
    let mut rows: Vec<Row> = server
        .rows
        .iter()
        .filter(|r| {
            grow_only || {
                let id = identity(&server.columns, r, seq);
                !b.contains(&id) || l.contains(&id)
            }
        })
        .cloned()
        .collect();
    let mut next = rows.iter().map(|r| number(r, at)).max().unwrap_or(0);
    let mut added: Vec<&Row> = local
        .rows
        .iter()
        .filter(|r| {
            let id = identity(&local.columns, r, seq);
            !s.contains(&id) && (grow_only || !b.contains(&id))
        })
        .collect();
    added.sort_by_key(|r| number(r, at));
    for row in added {
        next += 1;
        let mut row = row.clone();
        row[at] = Cell::Integer(next);
        rows.push(row);
    }
    if grow_only {
        rows.sort_by_key(|r| number(r, at));
        let excess = rows.len().saturating_sub(usize::try_from(AUDIT_RETENTION).unwrap_or(usize::MAX));
        rows.drain(..excess);
    }
    Table {
        columns: server.columns.clone(),
        rows,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reins_policy::Scope;

    const NOW: i64 = 1_800_000_000;

    fn grant_row(g: &Grant) -> Row {
        vec![
            Cell::Text(g.id.0.clone()),
            Cell::Text(g.connection_id.0.clone()),
            Cell::Text("Claude".to_owned()),
            Cell::Integer(g.created_at),
            Cell::Text(serde_json::to_string(g).unwrap()),
            Cell::Null,
            Cell::Text("approval".to_owned()),
        ]
    }
    fn grants_and_meta(rows: Vec<Row>, tombstones: &[&str]) -> Snapshot {
        let columns = ["id", "connection_id", "connection_label", "created_at", "grant_json", "last_used_at", "origin"]
            .map(str::to_owned)
            .to_vec();
        let meta = tombstones
            .iter()
            .map(|id| {
                vec![
                    Cell::Text(format!("{}grants:[\"{id}\"]", super::super::TOMBSTONE_PREFIX)),
                    Cell::Integer(NOW - 10),
                ]
            })
            .collect();
        Snapshot {
            version: 1,
            tables: BTreeMap::from([
                (
                    "grants".to_owned(),
                    Table {
                        columns,
                        rows,
                    },
                ),
                ("meta".to_owned(), table(&["key", "value"], meta)),
            ]),
        }
    }
    fn grants(rows: Vec<Row>) -> Snapshot {
        grants_and_meta(rows, &[])
    }
    fn grant(id: &str) -> Grant {
        crate::store::grants::tests::read_grant(id, "c1", Some(1_000))
    }
    fn merged_grants(base: Option<&Snapshot>, local: &Snapshot, server: &Snapshot) -> (Vec<Grant>, bool) {
        let m = merge(base, local, server, NOW);
        let t = &m.snapshot.tables["grants"];
        (t.rows.iter().map(|r| grant_of(&t.columns, r).unwrap()).collect(), m.dropped_local)
    }
    fn other_scope() -> Scope {
        Scope::Read(reins_policy::ReadScope {
            from: vec![reins_policy::AddrRule::domain("example.com").unwrap()],
            ..reins_policy::ReadScope::default()
        })
    }

    #[test]
    fn a_change_on_one_side_wins_when_it_tightens_and_new_grants_are_kept() {
        let base = grants(vec![grant_row(&grant("g1")), grant_row(&grant("g2"))]);
        let mut revoked = grant("g1");
        revoked.revoked = true;
        let local = grants(vec![grant_row(&revoked), grant_row(&grant("g2"))]);
        let server = grants(vec![grant_row(&grant("g1")), grant_row(&grant("g2")), grant_row(&grant("g3"))]);
        let (out, dropped) = merged_grants(Some(&base), &local, &server);
        assert_eq!(out, [revoked, grant("g2"), grant("g3")]);
        assert!(!dropped);
    }

    #[test]
    fn revoked_beats_extended_and_a_revoked_base_is_never_resumed_by_a_merge() {
        let base = grants(vec![grant_row(&grant("g1"))]);
        let mut revoked = grant("g1");
        revoked.revoked = true;
        let mut extended = grant("g1");
        extended.expires_at = Some(9_000_000);
        extended.max_uses = None;
        for (local, server) in [(&revoked, &extended), (&extended, &revoked)] {
            let (out, _) =
                merged_grants(Some(&base), &grants(vec![grant_row(local)]), &grants(vec![grant_row(server)]));
            assert!(out[0].revoked, "a revoked grant is never resurrected");
        }
        // Revoked at the base, resumed on the server, used here: still revoked, whatever the clocks.
        let base = grants(vec![grant_row(&revoked)]);
        let mut resumed = grant("g1");
        resumed.created_at = 99_999;
        resumed.expires_at = Some(9_000_000);
        let mut used = revoked;
        used.uses = 1;
        let (out, _) = merged_grants(Some(&base), &grants(vec![grant_row(&used)]), &grants(vec![grant_row(&resumed)]));
        assert!(out[0].revoked);
    }

    #[test]
    fn deleted_or_tombstoned_beats_edited() {
        let base = grants(vec![grant_row(&grant("g1"))]);
        let mut used = grant("g1");
        used.uses = 1;
        for (local, server) in
            [(grants(vec![]), grants(vec![grant_row(&used)])), (grants(vec![grant_row(&used)]), grants(vec![]))]
        {
            let (out, _) = merged_grants(Some(&base), &local, &server);
            assert!(out.is_empty(), "a grant deleted on either phone stays deleted");
        }
        // A tombstone on one side buries the grant even where nothing else tells it was deleted (no base).
        let local = grants_and_meta(vec![], &["g1"]);
        let (out, _) = merged_grants(None, &local, &grants(vec![grant_row(&used)]));
        assert!(out.is_empty());
        let m = merge(None, &local, &grants(vec![grant_row(&used)]), NOW);
        assert_eq!(m.snapshot.tables["meta"].rows.len(), 1, "the tombstone is kept");
    }

    #[test]
    fn grants_changed_on_both_sides_merge_to_the_tighter_limits() {
        let mut b = grant("g1");
        b.uses = 2;
        b.max_uses = Some(10);
        b.expires_at = Some(5_000);
        let mut l = b.clone();
        l.uses = 4;
        l.expires_at = Some(9_000); // extended here
        let mut s = b.clone();
        s.uses = 3;
        s.max_uses = Some(6);
        let (out, _) = merged_grants(
            Some(&grants(vec![grant_row(&b)])),
            &grants(vec![grant_row(&l)]),
            &grants(vec![grant_row(&s)]),
        );
        assert_eq!((out[0].uses, out[0].max_uses, out[0].expires_at), (5, Some(6), Some(5_000)));
        assert!(!out[0].revoked);
    }

    #[test]
    fn a_changed_scope_is_revoked() {
        let b = grant("g1");
        let mut s = b.clone();
        s.scope = other_scope();
        // Changed on one side only, or on both: no merge widens a scope.
        for local in [b.clone(), {
            let mut l = b.clone();
            l.scope = Scope::Read(reins_policy::ReadScope::default());
            l
        }] {
            let (out, _) = merged_grants(
                Some(&grants(vec![grant_row(&b)])),
                &grants(vec![grant_row(&local)]),
                &grants(vec![grant_row(&s)]),
            );
            assert!(out[0].revoked);
        }
    }

    #[test]
    fn without_a_base_the_server_wins_but_grants_only_tighten() {
        let mut revoked_here = grant("g1");
        revoked_here.revoked = true;
        let local = grants(vec![grant_row(&revoked_here), grant_row(&grant("only-here"))]);
        let server = grants(vec![grant_row(&grant("g1")), grant_row(&grant("only-there"))]);
        let (out, dropped) = merged_grants(None, &local, &server);
        let ids: Vec<&str> = out.iter().map(|g| g.id.0.as_str()).collect();
        assert_eq!(ids, ["g1", "only-there"], "what only this phone has may be what the other deleted");
        assert!(out[0].revoked, "revoked here stays revoked");
        assert!(dropped, "this phone's own grant is not in the result");
    }

    fn table(columns: &[&str], rows: Vec<Row>) -> Table {
        Table {
            columns: columns.iter().map(|c| (*c).to_owned()).collect(),
            rows,
        }
    }
    fn one(name: &str, t: Table) -> Snapshot {
        Snapshot {
            version: 1,
            tables: BTreeMap::from([(name.to_owned(), t)]),
        }
    }
    fn entry(seq: i64, what: &str) -> Row {
        vec![Cell::Integer(seq), Cell::Text(what.to_owned())]
    }

    #[test]
    fn activity_is_a_grow_only_union_trimmed_to_its_retention() {
        let cols = ["seq", "detail"];
        let base = one("audit", table(&cols, vec![entry(1, "old"), entry(2, "gone here")]));
        let local = one("audit", table(&cols, vec![entry(1, "old"), entry(3, "here a"), entry(4, "here b")]));
        let server = one("audit", table(&cols, vec![entry(1, "old"), entry(2, "gone here"), entry(3, "there")]));
        for base in [Some(&base), None] {
            let m = merge(base, &local, &server, NOW);
            assert_eq!(
                m.snapshot.tables["audit"].rows,
                [entry(1, "old"), entry(2, "gone here"), entry(3, "there"), entry(4, "here a"), entry(5, "here b")]
            );
            assert!(!m.dropped_local);
        }
        let many = |from: i64, what: &str| {
            one(
                "audit",
                table(&cols, (from..from + AUDIT_RETENTION).map(|i| entry(i, &format!("{what}{i}"))).collect()),
            )
        };
        let m = merge(None, &many(1, "here"), &many(1, "there"), NOW);
        let rows = &m.snapshot.tables["audit"].rows;
        assert_eq!(rows.len(), usize::try_from(AUDIT_RETENTION).unwrap());
        assert_eq!(rows.last().unwrap(), &entry(2 * AUDIT_RETENTION, &format!("here{AUDIT_RETENTION}")));
    }

    #[test]
    fn memory_forgotten_on_either_side_stays_forgotten() {
        let cols = ["id", "data"];
        let base = one("autopilot_memory", table(&cols, vec![entry(1, "kept"), entry(2, "forgotten here")]));
        let local = one("autopilot_memory", table(&cols, vec![entry(1, "kept")]));
        let server =
            one("autopilot_memory", table(&cols, vec![entry(1, "kept"), entry(2, "forgotten here"), entry(3, "new")]));
        let m = merge(Some(&base), &local, &server, NOW);
        assert_eq!(m.snapshot.tables["autopilot_memory"].rows, [entry(1, "kept"), entry(3, "new")]);
    }

    fn setting(mode: Option<&str>, bypass_until: Option<i64>, at: i64) -> Row {
        vec![
            Cell::Text(String::new()),
            mode.map_or(Cell::Null, |m| Cell::Text(m.to_owned())),
            bypass_until.map_or(Cell::Null, Cell::Integer),
            Cell::Null,
            Cell::Integer(at),
        ]
    }
    fn modes(row: Row) -> Snapshot {
        one("autopilot_settings", table(&["scope", "mode", "bypass_until", "profile_id", "updated_at"], vec![row]))
    }
    fn merged_mode(base: Row, local: Row, server: Row) -> Row {
        merge(Some(&modes(base)), &modes(local), &modes(server), NOW).snapshot.tables["autopilot_settings"].rows[0]
            .clone()
    }

    #[test]
    fn lockdown_wins_and_a_stopped_bypass_never_restarts_whatever_the_clocks() {
        // Lockdown set on one side while the other changed too (with a newer clock) wins.
        let m = merged_mode(
            setting(Some("manual"), None, 1),
            setting(Some("lockdown"), None, 2),
            setting(Some("bypass"), Some(9_999), 50),
        );
        assert_eq!(m[1], Cell::Text("lockdown".to_owned()));
        assert_eq!(m[2], Cell::Null);
        // A lockdown cleared on one phone is not brought back by the other phone's unchanged copy; nor is a bypass
        // that ended there brought back by a stale copy here.
        let m = merged_mode(
            setting(Some("lockdown"), None, 1),
            setting(Some("lockdown"), None, 1),
            setting(Some("manual"), None, 50),
        );
        assert_eq!(m[1], Cell::Text("manual".to_owned()));
        let m = merged_mode(
            setting(Some("bypass"), Some(9_999), 1),
            setting(Some("bypass"), Some(9_999), 1),
            setting(Some("manual"), None, 2),
        );
        assert_eq!((m[1].clone(), m[2].clone()), (Cell::Text("manual".to_owned()), Cell::Null));
        // Bypass stopped here (older clock), extended there (newer): stopped.
        let m = merged_mode(
            setting(Some("bypass"), Some(100), 1),
            setting(Some("manual"), None, 2),
            setting(Some("bypass"), Some(9_999), 90),
        );
        assert_eq!((m[1].clone(), m[2].clone()), (Cell::Text("manual".to_owned()), Cell::Null));
        // Overnight against auto, the newer clock on overnight: auto.
        let m = merged_mode(
            setting(Some("manual"), None, 1),
            setting(Some("auto"), None, 2),
            setting(Some("overnight"), None, 90),
        );
        assert_eq!(m[1], Cell::Text("auto".to_owned()));
        // Two bypasses: the earlier end.
        let m = merged_mode(
            setting(None, None, 1),
            setting(Some("bypass"), Some(500), 2),
            setting(Some("bypass"), Some(300), 3),
        );
        assert_eq!(m[2], Cell::Integer(300));
    }

    #[test]
    fn json_settings_merge_by_entry_and_asking_wins() {
        let cols = ["key", "value"];
        let kv = |v: &str| vec![Cell::Text("app.payments.ledger".to_owned()), Cell::Text(v.to_owned())];
        let base = one("meta", table(&cols, vec![kv(r#"[{"id":"p1"}]"#)]));
        let local = one("meta", table(&cols, vec![kv(r#"[{"id":"p1"},{"id":"here"}]"#)]));
        let server = one("meta", table(&cols, vec![kv(r#"[{"id":"there"},{"id":"p1"}]"#)]));
        let m = merge(Some(&base), &local, &server, NOW);
        assert_eq!(m.snapshot.tables["meta"].rows, [kv(r#"[{"id":"there"},{"id":"p1"},{"id":"here"}]"#)]);
        let kv = |v: &str| vec![Cell::Text("app.secrets.always-ask".to_owned()), Cell::Text(v.to_owned())];
        let base = one("meta", table(&cols, vec![kv(r#"{"a/prod":false}"#)]));
        let local = one("meta", table(&cols, vec![kv(r#"{"a/prod":true,"b/prod":true}"#)]));
        let server = one("meta", table(&cols, vec![kv(r#"{"a/prod":false,"c/dev":false}"#)]));
        let m = merge(Some(&base), &local, &server, NOW);
        assert_eq!(m.snapshot.tables["meta"].rows, [kv(r#"{"a/prod":true,"b/prod":true,"c/dev":false}"#)]);
    }

    #[test]
    fn every_portable_table_has_a_merge_rule() {
        for table in super::super::snapshot::TABLES {
            if !matches!(*table, "session" | "pending" | "handled" | "autopilot_suggestions" | "autopilot_rate") {
                assert!(kind(table).is_some(), "{table} needs a merge rule");
            }
        }
    }

    /// A small deterministic generator (xorshift), so that a failure can be replayed from its seed.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    /// One phone's edit of a grant, as the app can make it (and some it cannot: a merge must hold whatever it gets).
    fn edit(rng: &mut Rng, g: &Grant) -> Option<Grant> {
        let mut g = g.clone();
        match rng.below(9) {
            0 => return None, // deleted
            1 => g.revoked = true,
            2 => {
                // resumed or extended
                g.revoked = false;
                g.expires_at = Some(1_000_000 + i64::try_from(rng.below(1_000_000)).unwrap());
                g.uses = 0;
            }
            3 => g.expires_at = Some(i64::try_from(rng.below(10_000)).unwrap()),
            4 => g.expires_at = None,
            5 => g.uses += u32::try_from(rng.below(5)).unwrap() + 1,
            6 => {
                g.max_uses = if rng.below(2) == 0 {
                    None
                } else {
                    Some(u32::try_from(rng.below(20)).unwrap())
                }
            }
            7 => g.scope = other_scope(),
            _ => {}
        }
        Some(g)
    }

    /// Whether `merged` lets through nothing that `side` (a phone's version, `None`: deleted there) does not.
    fn no_more_permissive(merged: &Grant, side: Option<&Grant>) -> bool {
        let Some(side) = side else {
            return false;
        };
        let expiry_ok = match (merged.expires_at, side.expires_at) {
            (_, None) => true,
            (None, Some(_)) => false,
            (Some(m), Some(s)) => m <= s,
        };
        let limit_ok = match (merged.max_uses, side.max_uses) {
            (_, None) => true,
            (None, Some(_)) => false,
            (Some(m), Some(s)) => m <= s,
        };
        (merged.revoked || (!side.revoked && merged.scope == side.scope && merged.account == side.account))
            && expiry_ok
            && limit_ok
            && merged.uses >= side.uses
    }

    #[test]
    fn property_a_merge_is_never_more_permissive_than_either_side() {
        let mut rng = Rng(0x5eed_cafe_f00d_d00d);
        for case in 0..5_000 {
            let mut b = grant("g");
            b.revoked = rng.below(4) == 0;
            b.expires_at = if rng.below(2) == 0 {
                None
            } else {
                Some(i64::try_from(rng.below(10_000)).unwrap())
            };
            b.max_uses = if rng.below(2) == 0 {
                None
            } else {
                Some(u32::try_from(rng.below(20)).unwrap())
            };
            b.uses = u32::try_from(rng.below(5)).unwrap();
            let (l, s) = (edit(&mut rng, &b), edit(&mut rng, &b));
            // A phone that deleted the grant keeps its tombstone, as the store's trigger leaves one.
            let snap = |g: &Option<Grant>| match g {
                Some(g) => grants(vec![grant_row(g)]),
                None => grants_and_meta(vec![], &["g"]),
            };
            for base in [Some(grants(vec![grant_row(&b)])), None] {
                let (out, _) = merged_grants(base.as_ref(), &snap(&l), &snap(&s));
                if let Some(m) = out.first() {
                    assert!(
                        no_more_permissive(m, l.as_ref()) && no_more_permissive(m, s.as_ref()),
                        "case {case}: base {b:?}\nlocal {l:?}\nserver {s:?}\nmerged {m:?}"
                    );
                    if base.is_some() {
                        assert!(
                            no_more_permissive(m, Some(&b)) || !b.revoked,
                            "case {case}: revived the base's revoked grant"
                        );
                        assert!(!b.revoked || m.revoked, "case {case}: a revoked grant came back");
                    }
                }
            }
        }
    }
}
