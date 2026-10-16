use reins_proto::oplog::Table;
use serde_json::json;

use super::*;
use crate::store::tests::{audit_record, open};

/// A store with a row in every synced table, sealed columns included.
fn filled(dir: &std::path::Path) -> Store {
    let store = open(dir);
    store.insert_grant(&crate::store::grants::tests::read_grant("g1", "c1", None), "Claude").unwrap();
    store.secret_put("github", "octo", b"ghp_token").unwrap();
    store.secret_put("reins.device-key", "", b"this phone's own").unwrap();
    store.meta_set("secrets.always-ask", r#"{"github":true}"#).unwrap();
    store.meta_set("theme", "dark").unwrap();
    store.meta_set("quoted", r#""already JSON""#).unwrap();
    store.meta_set("session.account-id:x", "u1").unwrap();
    store.append_audit(&audit_record()).unwrap();
    let conn = store.lock().unwrap();
    conn.execute_batch(
        "INSERT INTO accounts (service, account, added_at) VALUES ('github', 'octo', 7);
         INSERT INTO autopilot_settings (scope, mode, bypass_until, profile_id, updated_at) VALUES ('', 'auto', 99, NULL, 5);
         INSERT INTO autopilot_settings (scope, mode, bypass_until, profile_id, updated_at) VALUES ('c1', NULL, NULL, 'p1', 6);
         INSERT INTO autopilot_targets (key, at) VALUES ('t1', 3);
         INSERT INTO desktop_keys (connection_id, public_key, pinned_at) VALUES ('c1', 'pk', 4);
         INSERT INTO connection_prefs (connection_id, icon) VALUES ('c1', 'star');
         INSERT INTO mcp_servers (id, name, url, auth, added_at) VALUES ('m1', 'Notes', 'https://notes.example', 'none', 8);",
    )
    .unwrap();
    let profile = store.seal("autopilot.profile:p1", b"{\"name\":\"Work\"}").unwrap();
    conn.execute("INSERT INTO autopilot_profiles (id, created_at, data) VALUES ('p1', 2, ?1)", [profile]).unwrap();
    let memory = store.seal("autopilot.memory:p1:r1", b"{\"verdict\":\"approve\"}").unwrap();
    conn.execute(
        "INSERT INTO autopilot_memory (profile_id, request_id, at, data) VALUES ('p1', 'r1', 9, ?1)",
        [memory],
    )
    .unwrap();
    drop(conn);
    store
}

fn by_key(rows: &[SyncRow]) -> std::collections::BTreeMap<(String, String), Json> {
    rows.iter().map(|r| ((format!("{:?}", r.table), r.key.clone()), r.value.clone())).collect()
}

#[test]
fn every_synced_row_goes_through_the_log_and_back_under_another_phones_key() {
    let (a_dir, b_dir) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let a = filled(a_dir.path());
    let rows = a.sync_rows().unwrap();
    let keys: Vec<(Table, &str)> = rows.iter().map(|r| (r.table, r.key.as_str())).collect();
    for expected in [
        (Table::Grants, "g1"),
        (Table::Secrets, r#"["github","octo"]"#),
        (Table::Accounts, r#"["github","octo"]"#),
        (Table::AutopilotSettings, "*"),
        (Table::AutopilotSettings, "c1"),
        (Table::Profiles, "p1"),
        (Table::Targets, "t1"),
        (Table::DesktopKeys, "c1"),
        (Table::ConnectionPrefs, "c1"),
        (Table::McpServers, "m1"),
        (Table::Meta, "secrets.always-ask"),
        (Table::Meta, "theme"),
    ] {
        assert!(keys.contains(&expected), "{expected:?} in {keys:?}");
    }
    // Plain JSON for the rules: the grant parsed, a JSON setting as it is, a token opened.
    let get = |t: Table, k: &str| rows.iter().find(|r| r.table == t && r.key == k).unwrap().value.clone();
    assert_eq!(get(Table::Grants, "g1")["grant"]["id"], "g1");
    assert_eq!(get(Table::Meta, "secrets.always-ask"), json!({"github": true}));
    assert_eq!(get(Table::Meta, "theme"), json!({"$text": "dark"}));
    assert_eq!(get(Table::Secrets, r#"["github","octo"]"#)["value"], b64(b"ghp_token"));
    // Never: this phone's own key, sign-in state.
    assert!(!rows.iter().any(|r| r.value.to_string().contains("this phone")), "a device key");
    assert!(!keys.iter().any(|(_, k)| k.contains("session")), "sign-in state");

    // Another phone (another data key) writes them: the same records come out of it.
    let b = open(b_dir.path());
    for row in &rows {
        b.write_sync_row(row.table, &row.key, None, Some(&row.value)).unwrap();
    }
    assert_eq!(by_key(&b.sync_rows().unwrap()), by_key(&rows));
    assert_eq!(b.secret_get("github", "octo").unwrap().as_deref(), Some(&b"ghp_token"[..]));
    assert_eq!(b.meta_get("quoted").unwrap().as_deref(), Some(r#""already JSON""#));
}

#[test]
fn a_record_never_writes_this_installations_own_rows() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(dir.path());
    let device = json!({"service": "reins.device-key", "account": "", "value": b64(b"attacker")});
    assert!(store.write_sync_row(Table::Secrets, r#"["reins.device-key",""]"#, None, Some(&device)).is_err());
    assert!(store.write_sync_row(Table::Secrets, "x", Some(r#"["reins.device-key",""]"#), None).is_err());
    for key in ["~device_id", "~dek_check", "session.account-id:x", "tombstone.grants:[\"g1\"]"] {
        assert!(store.write_sync_row(Table::Meta, key, None, Some(&json!({"$text": "x"}))).is_err(), "{key}");
    }
    assert!(store.write_sync_row(Table::Meta, "~reins.view-grant/x/2", None, Some(&json!({}))).is_err());
    assert!(store.write_sync_row(Table::Grants, "g1", None, Some(&json!({"grant": {"not": "a grant"}}))).is_err());
}

#[test]
fn removals_find_the_row_and_logs_are_told_apart_by_content() {
    let dir = tempfile::tempdir().unwrap();
    let store = filled(dir.path());
    let rows = store.sync_rows().unwrap();
    let account = rows.iter().find(|r| r.table == Table::Accounts).unwrap();
    store.write_sync_row(Table::Accounts, &account.key, Some(&account.local), None).unwrap();
    let memory = rows.iter().find(|r| r.table == Table::AutopilotMemory).unwrap();
    assert!(memory.key.starts_with('#') && memory.value.get("id").is_none(), "{memory:?}");
    store.write_sync_row(Table::AutopilotMemory, &memory.key, Some(&memory.local), None).unwrap();
    let after = store.sync_rows().unwrap();
    assert!(!after.iter().any(|r| r.table == Table::Accounts || r.table == Table::AutopilotMemory));
    // A memory entry another phone wrote is added with this phone's own number, the same record.
    let added = store.write_sync_row(Table::AutopilotMemory, &memory.key, None, Some(&memory.value)).unwrap();
    assert!(added.is_some());
    let again = store.sync_rows().unwrap();
    assert_eq!(again.iter().find(|r| r.table == Table::AutopilotMemory).unwrap().key, memory.key);
}

#[test]
fn activity_entries_come_out_after_a_seq_and_go_in_with_a_new_one() {
    let (a_dir, b_dir) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let a = filled(a_dir.path());
    a.append_audit(&audit_record()).unwrap();
    let all = a.audit_rows_after(0, 10).unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(a.audit_rows_after(all[0].0, 10).unwrap().len(), 1);
    assert!(all[0].1.get("seq").is_none());
    assert_eq!(all[0].1["detail"], b64(b"2 messages"));
    let b = open(b_dir.path());
    b.insert_sync_audit(&all[0].1).unwrap();
    assert_eq!(b.activity(5).unwrap()[0].detail, "2 messages");
    assert_eq!(b.audit_head().unwrap(), 1);
}

#[test]
fn a_viewer_forgets_every_token_but_keeps_its_own_keys() {
    let dir = tempfile::tempdir().unwrap();
    let store = filled(dir.path());
    store.secret_put("payments.mandate-key", "", b"mandate").unwrap();
    assert_eq!(store.wipe_tokens().unwrap(), 2);
    assert_eq!(store.secret_get("github", "octo").unwrap(), None);
    assert_eq!(store.secret_get("payments.mandate-key", "").unwrap(), None);
    assert!(store.secret_get("reins.device-key", "").unwrap().is_some());
}

#[test]
fn digests_ignore_entry_order() {
    assert_eq!(digest(&json!({"a": 1, "b": {"x": 1, "y": 2}})), digest(&json!({"b": {"y": 2, "x": 1}, "a": 1})));
    assert_ne!(digest(&json!({"a": 1})), digest(&json!({"a": 2})));
}

#[test]
fn proposal_rows_are_written_all_or_none_and_never_over_a_row_there() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(dir.path());
    let account = json!({"service": "github", "account": "octo", "added_at": 1});
    let bad = json!({"service": "github", "account": "octo2", "added_at": "not a number", "extra": 1});
    // A bad second row: the first is not left behind.
    assert!(store.insert_sync_rows(&[(Table::Accounts, account.clone()), (Table::Accounts, bad)]).is_err());
    assert!(store.sync_rows().unwrap().iter().all(|r| r.table != Table::Accounts));
    store.insert_sync_rows(&[(Table::Accounts, account)]).unwrap();
    // The same key again: refused, nothing replaced.
    let replaced = json!({"service": "github", "account": "octo", "added_at": 99});
    assert!(store.insert_sync_rows(&[(Table::Accounts, replaced)]).is_err());
    let rows = store.sync_rows().unwrap();
    assert_eq!(rows.iter().find(|r| r.table == Table::Accounts).unwrap().value["added_at"], 1);
}

#[test]
fn an_untrusted_start_takes_nothing_that_loosens_anything() {
    let dir = tempfile::tempdir().unwrap();
    let store = filled(dir.path());
    store.meta_set("vault.secret-store.seen", r#"["a/dev"]"#).unwrap();
    let rows = store.sync_rows().unwrap();
    let get = |t: Table, k: &str| rows.iter().find(|r| r.table == t && r.key == k).unwrap().value.clone();
    let now = 1_800_000_000;
    let tight = |t: Table, k: &str, v: Json| store.tighten_record(t, k, &v, now).unwrap();
    let written = |t: Table, k: &str, v: Json| match tight(t, k, v) {
        Tightened::Write(v) => v,
        other => panic!("{other:?}"),
    };
    assert_eq!(written(Table::Grants, "g1", get(Table::Grants, "g1"))["grant"]["revoked"], true);
    let global = written(Table::AutopilotSettings, "*", get(Table::AutopilotSettings, "*"));
    assert_eq!((global["mode"].clone(), global["bypass_until"].clone()), (json!("manual"), Json::Null));
    let lockdown = json!({"scope": "c2", "mode": "lockdown", "bypass_until": 9, "updated_at": 1});
    assert_eq!(written(Table::AutopilotSettings, "c2", lockdown)["mode"], "lockdown", "Lockdown stays");
    assert_eq!(written(Table::AutopilotSettings, "c1", get(Table::AutopilotSettings, "c1"))["mode"], Json::Null);
    let profile = written(Table::Profiles, "p1", get(Table::Profiles, "p1"));
    let data: Json = serde_json::from_slice(&bytes_of(&profile["data"]).unwrap()).unwrap();
    assert_eq!(data["name"], "Work");
    for skipped in [Table::AutopilotMemory, Table::Targets, Table::DesktopKeys] {
        let any = rows.iter().find(|r| r.table == skipped).unwrap();
        assert_eq!(tight(skipped, &any.key, any.value.clone()), Tightened::Skip, "{skipped:?}");
    }
    assert_eq!(tight(Table::Secrets, "x", json!({})), Tightened::KeepLocal);
    let server =
        written(Table::McpServers, "m1", json!({"id": "m1", "tools": "[{\"read_only\":true}]", "status": "ok"}));
    assert_eq!((server["tools"].clone(), server["status"].clone()), (json!("[]"), json!("needs_sign_in")));
    assert_eq!(
        written(Table::Meta, "secrets.always-ask", json!({"a": false, "b": true})),
        json!({"a": true, "b": true})
    );
    assert_eq!(
        written(Table::Meta, "starting_policy", json!({"$text": "headlines_first_hour"})),
        json!({"$text": "ask_every_time"})
    );
    let config = written(
        Table::Meta,
        "payments.config",
        json!({"limits": [{"id": "l"}], "tolerance_pct": 50,
        "default_method": "card:1", "cards": ["1"], "pay_on_phone": true, "merchant_account": true, "budgets": [1]}),
    );
    assert_eq!(
        config,
        json!({"limits": [], "tolerance_pct": 0, "default_method": null, "default_address": null,
        "cards": [], "pay_on_phone": false, "merchant_account": false, "budgets": [1]})
    );
    assert_eq!(tight(Table::Meta, "payments.ledger", json!([])), Tightened::KeepLocal);
    assert_eq!(written(Table::Meta, "~autopilot_created:c1", json!({"$text": "5"})), json!({"$text": now.to_string()}));
    // Never fewer secrets seen than this phone saw.
    assert_eq!(written(Table::Meta, "vault.secret-store.seen", json!(["b/prod"])), json!(["a/dev", "b/prod"]));
}
