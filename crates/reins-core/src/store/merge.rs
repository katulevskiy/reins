//! Three-way merge of portable account snapshots: the state both phones last agreed on (the base), this phone's,
//! and the server's. Each record is merged on its own:
//!
//! - changed on one side only: that side's version;
//! - changed on both: deleted wins over edited; a grant becomes the more restrictive of the two (revoked over
//!   active, the earlier expiry, the fewer uses left, never a changed scope); a row with `updated_at` the newer one;
//!   anything else the server's (what the other phone already uploaded);
//! - logs (activity, Autopilot memory): entries added on either side are all kept, numbered after the server's.
//!
//! Without a base (an older version kept only a hash of it, or it was lost) nothing can tell an addition here from a
//! deletion there: the server's version wins, except that a grant is never left less restrictive than this phone's
//! and log entries are still all kept. [`Merged::dropped_local`] then says whether anything of this phone's was lost.
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
    /// An append-only log numbered by this column, which each phone counts on its own.
    Log(&'static str),
}

/// Every portable table needs a rule here (a test checks): one without is merged as a whole.
fn kind(table: &str) -> Option<Kind> {
    Some(match table {
        "meta" | "autopilot_targets" => Kind::Keyed(&["key"]),
        "grants" | "mcp_servers" | "autopilot_profiles" => Kind::Keyed(&["id"]),
        "connection_prefs" | "desktop_keys" => Kind::Keyed(&["connection_id"]),
        "accounts" | "secrets" => Kind::Keyed(&["service", "account"]),
        "autopilot_settings" => Kind::Keyed(&["scope"]),
        "audit" => Kind::Log("seq"),
        "autopilot_memory" => Kind::Log("id"),
        _ => return None,
    })
}

pub(super) fn merge(base: Option<&Snapshot>, local: &Snapshot, server: &Snapshot) -> Merged {
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
                Some(Kind::Log(seq)) => log(seq, b.as_ref(), &l, &s),
                None => whole(b.as_ref(), l, s, &mut dropped_local),
            }
        };
        tables.insert(name.clone(), merged);
    }
    Merged {
        snapshot: Snapshot {
            version: 1,
            tables,
        },
        dropped_local,
    }
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
        let (lr, sr) = (l.get(k), s.get(k));
        let br = b.as_ref().and_then(|b| b.get(k));
        let picked = if lr == sr {
            lr.cloned()
        } else if b.is_some() && br == lr {
            sr.cloned()
        } else if b.is_some() && br == sr {
            lr.cloned()
        } else {
            let resolved = conflict(name, &server.columns, br, lr, sr);
            *dropped_local |=
                b.is_none() && lr.is_some() && resolved.as_ref() != lr && !restricts(name, lr, resolved.as_ref());
            resolved
        };
        rows.extend(picked);
    }
    Table {
        columns: server.columns.clone(),
        rows,
    }
}

/// Without a base, a grant made more restrictive by the merge is not a loss: that is the merge doing its job.
fn restricts(table: &str, local: Option<&Row>, resolved: Option<&Row>) -> bool {
    table == "grants" && local.is_some() && resolved.is_some()
}

/// Both sides changed the record (or, without a base, they differ).
fn conflict(
    name: &str,
    columns: &[String],
    base: Option<&Row>,
    local: Option<&Row>,
    server: Option<&Row>,
) -> Option<Row> {
    let (Some(l), Some(s)) = (local, server) else {
        // Deleted on one side, changed on the other: deleted. Without a base, the server's version.
        return if base.is_some() {
            None
        } else {
            server.cloned()
        };
    };
    match name {
        "grants" => merge_grant(columns, base, l, s),
        "meta" => Some(merge_meta(columns, base, l, s)),
        _ => {
            if let Some(i) = columns.iter().position(|c| c == "updated_at")
                && let (Some(Cell::Integer(lt)), Some(Cell::Integer(st))) = (l.get(i), s.get(i))
                && lt > st
            {
                return Some(l.clone());
            }
            Some(s.clone())
        }
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
/// The earlier of two limits, where `None` is no limit.
fn tighter<T: Ord + Copy>(a: Option<T>, b: Option<T>) -> Option<T> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

/// Two versions of one grant, both changed: the merge is never less restrictive than either. Revoked wins; a grant
/// whose scope, account or connection differ between the two is dropped (neither can be shown to be the narrower);
/// otherwise the earlier expiry, the smaller use limit, and every use either phone counted.
fn merge_grant(columns: &[String], base: Option<&Row>, local: &Row, server: &Row) -> Option<Row> {
    let (Some(l), Some(s)) = (grant_of(columns, local), grant_of(columns, server)) else {
        return None;
    };
    if s.revoked {
        return Some(server.clone());
    }
    if l.revoked {
        return Some(local.clone());
    }
    if l.id != s.id || l.connection_id != s.connection_id || l.scope != s.scope || l.account != s.account {
        return None;
    }
    let mut merged = s.clone();
    merged.expires_at = tighter(l.expires_at, s.expires_at);
    merged.max_uses = tighter(l.max_uses, s.max_uses);
    // Uses counted on each phone since the base add up; never fewer than either phone counted.
    let base_uses = base.and_then(|b| grant_of(columns, b)).map_or(0, |b| b.uses);
    let added = l.uses.saturating_add(s.uses).saturating_sub(base_uses);
    merged.uses = added.max(l.uses).max(s.uses);
    let mut row = server.clone();
    set_cell(columns, &mut row, "grant_json", Cell::Text(serde_json::to_string(&merged).ok()?));
    if let (Some(Cell::Integer(a)), Some(Cell::Integer(b))) =
        (cell(columns, local, "last_used_at"), cell(columns, server, "last_used_at"))
    {
        set_cell(columns, &mut row, "last_used_at", Cell::Integer(*a.max(b)));
    } else if let Some(Cell::Integer(a)) = cell(columns, local, "last_used_at") {
        set_cell(columns, &mut row, "last_used_at", Cell::Integer(*a));
    }
    // A grant a bypass gave lasts only while that bypass does: the more restrictive origin.
    if let Some(Cell::Text(origin)) = cell(columns, local, "origin")
        && crate::autopilot::modes::temporary_scope(origin).is_some()
    {
        set_cell(columns, &mut row, "origin", Cell::Text(origin.clone()));
    }
    Some(row)
}

/// A setting kept as JSON (purchases, choices per environment, revocations to send) merges by its entries: an object
/// key by key, a list by its items' `id` (or the whole item). Any other value: the server's.
fn merge_meta(columns: &[String], base: Option<&Row>, local: &Row, server: &Row) -> Row {
    let json = |row: Option<&Row>| match row.and_then(|r| cell(columns, r, "value")) {
        Some(Cell::Text(t)) => serde_json::from_str::<Json>(t).ok(),
        _ => None,
    };
    let (b, l, s) = (json(base), json(Some(local)), json(Some(server)));
    let merged = match (l, s) {
        (Some(Json::Object(l)), Some(Json::Object(s))) => {
            let b = match b {
                Some(Json::Object(b)) => b,
                _ => serde_json::Map::new(),
            };
            let mut out = serde_json::Map::new();
            let keys: BTreeSet<&String> = l.keys().chain(s.keys()).collect();
            for k in keys {
                let (bv, lv, sv) = (b.get(k), l.get(k), s.get(k));
                // Changed on one side: that side's; on both (or deleted on one): the server's.
                let v = if lv == sv || bv == sv {
                    lv
                } else {
                    sv
                };
                if let Some(v) = v {
                    out.insert(k.clone(), v.clone());
                }
            }
            Some(Json::Object(out))
        }
        (Some(Json::Array(l)), Some(Json::Array(s))) => {
            let item_id = |v: &Json| v.get("id").and_then(Json::as_str).map_or_else(|| v.to_string(), str::to_owned);
            let ids = |items: &[Json]| items.iter().map(item_id).collect::<BTreeSet<_>>();
            let b = match b {
                Some(Json::Array(b)) => ids(&b),
                _ => BTreeSet::new(),
            };
            let (here, there) = (ids(&l), ids(&s));
            // The server's items but those this phone deleted, then the ones this phone added.
            let mut out: Vec<Json> =
                s.into_iter().filter(|v| !b.contains(&item_id(v)) || here.contains(&item_id(v))).collect();
            out.extend(l.into_iter().filter(|v| !there.contains(&item_id(v)) && !b.contains(&item_id(v))));
            Some(Json::Array(out))
        }
        _ => None,
    };
    let mut row = server.clone();
    if let Some(text) = merged.and_then(|m| serde_json::to_string(&m).ok()) {
        set_cell(columns, &mut row, "value", Cell::Text(text));
    }
    row
}

/// Entries added on either side are kept; one deleted on a side (pruned, or forgotten on purpose) goes. The server's
/// keep their numbers; this phone's new ones follow them.
fn log(seq: &str, base: Option<&Table>, local: &Table, server: &Table) -> Table {
    let Some(at) = server.column(seq) else {
        return server.clone();
    };
    let ids = |t: &Table| t.rows.iter().map(|r| identity(&t.columns, r, seq)).collect::<BTreeSet<_>>();
    let (b, l, s) = (base.map(ids).unwrap_or_default(), ids(local), ids(server));
    let mut rows: Vec<Row> = server
        .rows
        .iter()
        .filter(|r| {
            let id = identity(&server.columns, r, seq);
            !b.contains(&id) || l.contains(&id)
        })
        .cloned()
        .collect();
    let mut next = rows
        .iter()
        .filter_map(|r| {
            if let Some(Cell::Integer(n)) = r.get(at) {
                Some(*n)
            } else {
                None
            }
        })
        .max()
        .unwrap_or(0);
    let mut added: Vec<&Row> = local
        .rows
        .iter()
        .filter(|r| {
            let id = identity(&local.columns, r, seq);
            !s.contains(&id) && !b.contains(&id)
        })
        .collect();
    added.sort_by_key(|r| {
        if let Some(Cell::Integer(n)) = r.get(at) {
            *n
        } else {
            0
        }
    });
    for row in added {
        next += 1;
        let mut row = row.clone();
        row[at] = Cell::Integer(next);
        rows.push(row);
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
    fn grants(rows: Vec<Row>) -> Snapshot {
        let columns = ["id", "connection_id", "connection_label", "created_at", "grant_json", "last_used_at", "origin"]
            .map(str::to_owned)
            .to_vec();
        Snapshot {
            version: 1,
            tables: BTreeMap::from([(
                "grants".to_owned(),
                Table {
                    columns,
                    rows,
                },
            )]),
        }
    }
    fn grant(id: &str) -> Grant {
        crate::store::grants::tests::read_grant(id, "c1", Some(1_000))
    }
    fn merged_grants(base: Option<&Snapshot>, local: &Snapshot, server: &Snapshot) -> (Vec<Grant>, bool) {
        let m = merge(base, local, server);
        let t = &m.snapshot.tables["grants"];
        (t.rows.iter().map(|r| grant_of(&t.columns, r).unwrap()).collect(), m.dropped_local)
    }

    #[test]
    fn a_change_on_one_side_wins() {
        let base = grants(vec![grant_row(&grant("g1")), grant_row(&grant("g2"))]);
        let mut revoked = grant("g1");
        revoked.revoked = true;
        // Revoked here, untouched there; and a new grant on the server.
        let local = grants(vec![grant_row(&revoked), grant_row(&grant("g2"))]);
        let server = grants(vec![grant_row(&grant("g1")), grant_row(&grant("g2")), grant_row(&grant("g3"))]);
        let (out, dropped) = merged_grants(Some(&base), &local, &server);
        assert_eq!(out, [revoked, grant("g2"), grant("g3")]);
        assert!(!dropped);
    }

    #[test]
    fn revoked_beats_extended() {
        let base = grants(vec![grant_row(&grant("g1"))]);
        let mut revoked = grant("g1");
        revoked.revoked = true;
        let mut extended = grant("g1");
        extended.expires_at = Some(9_000_000);
        extended.max_uses = None;
        for (local, server) in [(&revoked, &extended), (&extended, &revoked)] {
            let (out, _) =
                merged_grants(Some(&base), &grants(vec![grant_row(local)]), &grants(vec![grant_row(server)]));
            assert_eq!(out.len(), 1);
            assert!(out[0].revoked, "a revoked grant is never resurrected");
        }
    }

    #[test]
    fn deleted_beats_edited() {
        let base = grants(vec![grant_row(&grant("g1"))]);
        let mut used = grant("g1");
        used.uses = 1;
        for (local, server) in
            [(grants(vec![]), grants(vec![grant_row(&used)])), (grants(vec![grant_row(&used)]), grants(vec![]))]
        {
            let (out, _) = merged_grants(Some(&base), &local, &server);
            assert!(out.is_empty(), "a grant deleted on either phone stays deleted");
        }
    }

    #[test]
    fn grants_changed_on_both_sides_merge_to_the_tighter_limits() {
        let mut b = grant("g1");
        b.uses = 2;
        b.max_uses = Some(10);
        b.expires_at = Some(5_000);
        let mut l = b.clone();
        l.uses = 4;
        l.expires_at = Some(9_000); // resumed for longer here
        let mut s = b.clone();
        s.uses = 3;
        s.max_uses = Some(6);
        let (out, _) = merged_grants(
            Some(&grants(vec![grant_row(&b)])),
            &grants(vec![grant_row(&l)]),
            &grants(vec![grant_row(&s)]),
        );
        assert_eq!(out.len(), 1);
        assert_eq!((out[0].uses, out[0].max_uses, out[0].expires_at), (5, Some(6), Some(5_000)));
        assert!(!out[0].revoked);
    }

    #[test]
    fn a_grant_whose_scope_differs_on_the_two_phones_is_dropped() {
        let b = grant("g1");
        let mut l = b.clone();
        l.uses = 1;
        let mut s = b.clone();
        s.scope = Scope::Read(reins_policy::ReadScope {
            from: vec![reins_policy::AddrRule::domain("example.com").unwrap()],
            ..reins_policy::ReadScope::default()
        });
        let (out, _) = merged_grants(
            Some(&grants(vec![grant_row(&b)])),
            &grants(vec![grant_row(&l)]),
            &grants(vec![grant_row(&s)]),
        );
        assert!(out.is_empty(), "neither scope can be shown to be the narrower one");
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
    fn log_entries_from_both_phones_are_all_kept_and_renumbered() {
        let cols = ["seq", "detail"];
        let base = one("audit", table(&cols, vec![entry(1, "old"), entry(2, "pruned here")]));
        let local = one("audit", table(&cols, vec![entry(1, "old"), entry(3, "here a"), entry(4, "here b")]));
        let server = one("audit", table(&cols, vec![entry(1, "old"), entry(2, "pruned here"), entry(3, "there")]));
        let m = merge(Some(&base), &local, &server);
        let rows = &m.snapshot.tables["audit"].rows;
        assert_eq!(rows, &[entry(1, "old"), entry(3, "there"), entry(4, "here a"), entry(5, "here b")]);
        assert!(!m.dropped_local);
        // Without a base nothing is lost either: entries are only ever added.
        let m = merge(None, &local, &server);
        assert_eq!(m.snapshot.tables["audit"].rows.len(), 5);
    }

    #[test]
    fn a_setting_changed_on_both_phones_keeps_the_newest() {
        let cols = ["scope", "mode", "bypass_until", "profile_id", "updated_at"];
        let row = |mode: &str, at: i64| {
            vec![Cell::Text(String::new()), Cell::Text(mode.to_owned()), Cell::Null, Cell::Null, Cell::Integer(at)]
        };
        let base = one("autopilot_settings", table(&cols, vec![row("ask", 1)]));
        let newer_here = one("autopilot_settings", table(&cols, vec![row("lockdown", 30)]));
        let older_there = one("autopilot_settings", table(&cols, vec![row("assist", 20)]));
        let m = merge(Some(&base), &newer_here, &older_there);
        assert_eq!(m.snapshot.tables["autopilot_settings"].rows, [row("lockdown", 30)]);
        let m = merge(Some(&base), &older_there, &newer_here);
        assert_eq!(m.snapshot.tables["autopilot_settings"].rows, [row("lockdown", 30)]);
    }

    #[test]
    fn json_settings_merge_by_entry() {
        let cols = ["key", "value"];
        let kv = |v: &str| vec![Cell::Text("app.payments.ledger".to_owned()), Cell::Text(v.to_owned())];
        let base = one("meta", table(&cols, vec![kv(r#"[{"id":"p1"}]"#)]));
        let local = one("meta", table(&cols, vec![kv(r#"[{"id":"p1"},{"id":"here"}]"#)]));
        let server = one("meta", table(&cols, vec![kv(r#"[{"id":"there"},{"id":"p1"}]"#)]));
        let m = merge(Some(&base), &local, &server);
        assert_eq!(m.snapshot.tables["meta"].rows, [kv(r#"[{"id":"there"},{"id":"p1"},{"id":"here"}]"#)]);
        let kv = |v: &str| vec![Cell::Text("app.secrets.always-ask".to_owned()), Cell::Text(v.to_owned())];
        let base = one("meta", table(&cols, vec![kv(r#"{"a/prod":true}"#)]));
        let local = one("meta", table(&cols, vec![kv(r#"{"a/prod":true,"b/prod":true}"#)]));
        let server = one("meta", table(&cols, vec![kv(r#"{"c/dev":false}"#)]));
        let m = merge(Some(&base), &local, &server);
        assert_eq!(m.snapshot.tables["meta"].rows, [kv(r#"{"b/prod":true,"c/dev":false}"#)]);
    }

    #[test]
    fn every_portable_table_has_a_merge_rule() {
        for table in super::super::snapshot::TABLES {
            if !matches!(*table, "session" | "pending" | "handled" | "autopilot_suggestions" | "autopilot_rate") {
                assert!(kind(table).is_some(), "{table} needs a merge rule");
            }
        }
    }
}
