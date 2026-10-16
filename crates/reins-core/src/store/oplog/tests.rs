use reins_proto::oplog::{Change, ChangeRefusal, OpBody, OpClass, OpRefusal, Restriction, SyncedOp, Table};
use reins_proto::roster::RosterReason;
use rusqlite::Connection;
use serde_json::json;

use super::checkpoint::{self, CheckpointCheck, HeartbeatCheck};
use super::*;
use crate::roles::chain::tests::{genesis, member, next};
use crate::roles::keys::{OwnerSeed, SigningKey, ViewKeyring};

const NOW: i64 = 1_800_000_000;

/// p primary (key 1) with viewers v (2) and w (3), at epoch 3; `handed`: then a handover to v (epoch 4).
struct World {
    owner: OwnerSeed,
    roster: LocalRoster,
    keyring: ViewKeyring,
    counters: std::collections::HashMap<String, u64>,
}

fn key_of(author: &str) -> SigningKey {
    let n = match author {
        "p" => 1,
        "v" => 2,
        "w" => 3,
        _ => 9,
    };
    SigningKey::from_seed(&[n; 32]).unwrap()
}

impl World {
    fn new(handed: bool) -> Self {
        let owner = OwnerSeed::generate().unwrap();
        let g = genesis(&owner);
        let e2 = next(&g, RosterReason::Enroll, member("p", 1), vec![member("v", 2)], &owner);
        let e3 = next(&e2, RosterReason::Enroll, member("p", 1), vec![member("v", 2), member("w", 3)], &owner);
        let mut chain = vec![g.clone(), e2, e3];
        if handed {
            let e4 =
                next(&chain[2], RosterReason::Handover, member("v", 2), vec![member("w", 3), member("p", 1)], &owner);
            chain.push(e4);
        }
        let roster = LocalRoster::created(g).unwrap().updated(&chain).unwrap();
        let mut keyring = ViewKeyring::default();
        keyring.insert(owner.view_key("acct", 1));
        Self {
            owner,
            roster,
            keyring,
            counters: std::collections::HashMap::new(),
        }
    }

    fn ctx(&self) -> Context<'_> {
        Context {
            account: "acct",
            roster: &self.roster,
            keyring: &self.keyring,
            now: NOW,
        }
    }

    /// The next op of `author` at `epoch`, numbered, signed and sealed properly.
    fn op(&mut self, author: &str, epoch: u64, class: OpClass, change: Change) -> Op {
        let counter = self.counters.entry(author.to_owned()).or_insert(0);
        *counter += 1;
        let body = OpBody {
            v: 1,
            change,
        };
        seal_op("acct", &self.keyring, &key_of(author), author, epoch, *counter, class, false, &body).unwrap()
    }
}

fn store() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(SCHEMA).unwrap();
    conn
}

/// Applies `ops` from seq `first` on.
fn run(conn: &mut Connection, w: &World, first: u64, ops: &[Op]) -> Vec<Outcome> {
    ops.iter()
        .enumerate()
        .map(|(i, op)| {
            let synced = SyncedOp {
                seq: first + u64::try_from(i).unwrap(),
                op: op.clone(),
            };
            apply(conn, &w.ctx(), &synced).unwrap()
        })
        .collect()
}

fn put(table: Table, key: &str, prev: Option<Version>, row: serde_json::Value) -> Change {
    Change::Put {
        table,
        key: key.to_owned(),
        value: serde_json::to_value(RecordPut {
            prev,
            row,
        })
        .unwrap(),
    }
}

fn remove(table: Table, key: &str) -> Change {
    Change::Remove {
        table,
        key: key.to_owned(),
    }
}

fn grant_row(revoked: bool, expires_at: i64) -> serde_json::Value {
    let mut g = crate::store::grants::tests::read_grant("g1", "c1", None);
    g.expires_at = Some(expires_at);
    g.revoked = revoked;
    json!({"grant": g, "connection_label": "Claude"})
}

fn value(conn: &Connection, table: Table, key: &str) -> Option<serde_json::Value> {
    record(conn, table, key).unwrap().and_then(|r| r.value)
}

fn version(conn: &Connection, table: Table, key: &str) -> Version {
    record(conn, table, key).unwrap().unwrap().version
}

#[test]
fn ops_apply_in_seq_order_and_a_gap_a_replay_or_a_fork_is_caught() {
    let mut w = World::new(false);
    let mut conn = store();
    let a = w.op("p", 3, OpClass::Primary, put(Table::Accounts, "github/octo", None, json!({"added_at": 1})));
    let b = w.op("p", 3, OpClass::Primary, put(Table::Accounts, "gmail/me", None, json!({"added_at": 2})));
    assert_eq!(run(&mut conn, &w, 1, std::slice::from_ref(&a)), [Outcome::Applied]);
    // The same op again: a duplicate. A seq ahead: withheld ops.
    assert_eq!(run(&mut conn, &w, 1, &[a]), [Outcome::Duplicate]);
    let gap = apply(
        &mut conn,
        &w.ctx(),
        &SyncedOp {
            seq: 3,
            op: b.clone(),
        },
    );
    assert_eq!(
        gap,
        Err(Halt::Gap {
            expected: 2,
            got: 3
        })
    );
    assert_eq!(state(&conn).unwrap().seq, 1, "nothing of it was kept");
    // Another op at a seq this phone applied: two histories. Applying stops for good.
    let forked = apply(
        &mut conn,
        &w.ctx(),
        &SyncedOp {
            seq: 1,
            op: b.clone(),
        },
    );
    assert!(matches!(forked, Err(Halt::Forked(_))), "{forked:?}");
    assert!(matches!(
        apply(
            &mut conn,
            &w.ctx(),
            &SyncedOp {
                seq: 2,
                op: b
            }
        ),
        Err(Halt::Forked(_))
    ));
    assert!(state(&conn).unwrap().forked.is_some());
}

#[test]
fn only_the_roster_decides_who_writes_what() {
    let mut w = World::new(false);
    let mut conn = store();
    let viewer_primary = w.op("w", 3, OpClass::Primary, put(Table::Grants, "g1", None, grant_row(false, 9_000)));
    let stranger = w.op("x", 3, OpClass::Proposal, put(Table::Proposals, "x/1", None, json!({})));
    let mut other_account = w.op("p", 3, OpClass::Primary, put(Table::Meta, "k", None, json!(1)));
    other_account.account = "acct2".to_owned();
    other_account.sig = key_of("p").sign_b64(&other_account.signing_bytes().unwrap());
    assert_eq!(
        run(&mut conn, &w, 1, &[viewer_primary, stranger, other_account]),
        [
            Outcome::Refused(Refusal::NotAuthorized(OpRefusal::NotPrimary)),
            Outcome::Refused(Refusal::NotAuthorized(OpRefusal::NotAMember)),
            Outcome::Refused(Refusal::NotAuthorized(OpRefusal::WrongAccount)),
        ]
    );
    assert_eq!(value(&conn, Table::Grants, "g1"), None);
    // Refused ops still count: the history is the server's sequence, as on every phone.
    assert_eq!(state(&conn).unwrap().seq, 3);
}

#[test]
fn signatures_counters_seals_and_changes_are_all_checked() {
    let mut w = World::new(false);
    let mut conn = store();
    // Signed by another key than the author's.
    let mut forged = w.op("p", 3, OpClass::Primary, put(Table::Meta, "a", None, json!(1)));
    forged.sig = key_of("w").sign_b64(&forged.signing_bytes().unwrap());
    // Relabelled after signing (a viewer's op as primary).
    let mut relabelled = w.op("w", 3, OpClass::Restrictive, Change::Restrict(Restriction::Lockdown));
    relabelled.notify = true;
    // A replayed counter (p's op 1 again, re-signed with a new body).
    let first = w.op("p", 3, OpClass::Primary, put(Table::Meta, "b", None, json!(1)));
    let mut replay = w.op("p", 3, OpClass::Primary, put(Table::Meta, "c", None, json!(1)));
    replay.counter = first.counter;
    replay.sig = key_of("p").sign_b64(&replay.signing_bytes().unwrap());
    // A body moved under another envelope does not open.
    let mut moved = w.op("p", 3, OpClass::Primary, put(Table::Meta, "d", None, json!(1)));
    moved.body = first.body.clone();
    moved.sig = key_of("p").sign_b64(&moved.signing_bytes().unwrap());
    // A viewer's put labelled restrictive; a proposal for another phone's key.
    let brake_put =
        w.op("w", 3, OpClass::Restrictive, put(Table::AutopilotSettings, "*", None, json!({"mode": "bypass"})));
    let not_own = w.op("w", 3, OpClass::Proposal, put(Table::Proposals, "v/1", None, json!({})));
    assert_eq!(
        run(&mut conn, &w, 1, &[forged, relabelled, first, replay, moved, brake_put, not_own]),
        [
            Outcome::Refused(Refusal::BadSignature),
            Outcome::Refused(Refusal::BadSignature),
            Outcome::Applied,
            Outcome::Refused(Refusal::CounterReplay),
            Outcome::Refused(Refusal::Unopenable),
            Outcome::Refused(Refusal::Change(ChangeRefusal::NotRestrictive)),
            Outcome::Refused(Refusal::Change(ChangeRefusal::NotOwnProposal)),
        ]
    );
    // Counters that never arrived are flagged (the forged op's counter 1 never applied: a gap from 1); an op past
    // them still applies.
    w.counters.insert("p".to_owned(), 10);
    let skipped = w.op("p", 3, OpClass::Primary, put(Table::Meta, "e", None, json!(1)));
    assert_eq!(run(&mut conn, &w, 8, &[skipped]), [Outcome::Applied]);
    let gaps = counter_gaps(&conn).unwrap();
    assert!(gaps.iter().any(|(author, _, from)| author == "p" && *from == 1), "{gaps:?}");
}

#[test]
fn epochs_and_view_keys_this_phone_lacks_halt_applying() {
    let mut w = World::new(false);
    let mut conn = store();
    let ahead = w.op("p", 9, OpClass::Primary, put(Table::Meta, "a", None, json!(1)));
    assert_eq!(
        apply(
            &mut conn,
            &w.ctx(),
            &SyncedOp {
                seq: 1,
                op: ahead
            }
        ),
        Err(Halt::UnknownEpoch(9))
    );
    // A phone anchored at epoch 2 cannot verify epoch 1.
    let mut anchored = World::new(false);
    anchored.roster.entries.remove(0);
    let before = w.op("p", 1, OpClass::Primary, put(Table::Meta, "a", None, json!(1)));
    assert_eq!(
        apply(
            &mut conn,
            &anchored.ctx(),
            &SyncedOp {
                seq: 1,
                op: before
            }
        ),
        Err(Halt::Unverifiable(1))
    );
    // Sealed with a view key that came in later (kid 3, after a removal).
    let mut newer = ViewKeyring::default();
    newer.insert(w.owner.view_key("acct", 3));
    let body = OpBody {
        v: 1,
        change: put(Table::Meta, "a", None, json!(1)),
    };
    let sealed = seal_op("acct", &newer, &key_of("p"), "p", 3, 50, OpClass::Primary, false, &body).unwrap();
    assert_eq!(
        apply(
            &mut conn,
            &w.ctx(),
            &SyncedOp {
                seq: 1,
                op: sealed.clone()
            }
        ),
        Err(Halt::MissingKey(3))
    );
    w.keyring.insert(w.owner.view_key("acct", 3));
    assert_eq!(run(&mut conn, &w, 1, &[sealed]), [Outcome::Applied]);
}

#[test]
fn a_writer_that_saw_the_record_replaces_it_and_one_that_did_not_is_joined() {
    let mut w = World::new(false);
    let mut conn = store();
    let created = w.op("p", 3, OpClass::Primary, put(Table::Grants, "g1", None, grant_row(false, 5_000)));
    run(&mut conn, &w, 1, &[created]);
    let v1 = version(&conn, Table::Grants, "g1");
    // The primary extends it, having seen it: replaced.
    let extended = w.op("p", 3, OpClass::Primary, put(Table::Grants, "g1", Some(v1.clone()), grant_row(false, 9_000)));
    run(&mut conn, &w, 2, &[extended]);
    let row = value(&conn, Table::Grants, "g1").unwrap();
    assert_eq!(row["grant"]["expires_at"], 9_000);
    // A viewer pulls the brake on it.
    let revoke = w.op(
        "w",
        3,
        OpClass::Restrictive,
        Change::Restrict(Restriction::RevokeGrant {
            grant_id: "g1".to_owned(),
        }),
    );
    assert_eq!(run(&mut conn, &w, 3, &[revoke]), [Outcome::Applied]);
    assert_eq!(value(&conn, Table::Grants, "g1").unwrap()["grant"]["revoked"], true);
    // The primary, which had not seen the revoke, extends it again: joined, still revoked, the earlier expiry.
    let v2 = Version {
        counter: 2,
        ..v1
    };
    let stale = w.op("p", 3, OpClass::Primary, put(Table::Grants, "g1", Some(v2), grant_row(false, 99_000)));
    assert_eq!(run(&mut conn, &w, 4, &[stale]), [Outcome::Applied]);
    let row = value(&conn, Table::Grants, "g1").unwrap();
    assert_eq!((row["grant"]["revoked"].clone(), row["grant"]["expires_at"].clone()), (json!(true), json!(9_000)));
    // Having seen it (prev is the version now), the primary resumes it on purpose.
    let seen = version(&conn, Table::Grants, "g1");
    let resumed = w.op("p", 3, OpClass::Primary, put(Table::Grants, "g1", Some(seen), grant_row(false, 99_000)));
    run(&mut conn, &w, 5, &[resumed]);
    assert_eq!(value(&conn, Table::Grants, "g1").unwrap()["grant"]["revoked"], false);
}

#[test]
fn lockdown_and_a_stopped_bypass_hold_against_stale_writes() {
    let mut w = World::new(false);
    let mut conn = store();
    let bypass = w.op(
        "p",
        3,
        OpClass::Primary,
        put(Table::AutopilotSettings, "*", None, json!({"mode": "auto", "bypass_until": 5_000})),
    );
    let conn_bypass = w.op(
        "p",
        3,
        OpClass::Primary,
        put(Table::AutopilotSettings, "c1", None, json!({"mode": null, "bypass_until": 7_000})),
    );
    run(&mut conn, &w, 1, &[bypass, conn_bypass]);
    let v_global = version(&conn, Table::AutopilotSettings, "*");
    let stop = w.op("v", 3, OpClass::Restrictive, Change::Restrict(Restriction::StopBypass));
    run(&mut conn, &w, 3, &[stop]);
    for scope in ["*", "c1"] {
        assert_eq!(value(&conn, Table::AutopilotSettings, scope).unwrap()["bypass_until"], json!(null), "{scope}");
    }
    let lock = w.op("w", 3, OpClass::Restrictive, Change::Restrict(Restriction::Lockdown));
    run(&mut conn, &w, 4, &[lock]);
    // The primary's stale write (it saw the bypass, not the brake): Lockdown wins, the bypass stays stopped.
    let stale = w.op(
        "p",
        3,
        OpClass::Primary,
        put(Table::AutopilotSettings, "*", Some(v_global), json!({"mode": "overnight", "bypass_until": 9_000})),
    );
    run(&mut conn, &w, 5, &[stale]);
    let row = value(&conn, Table::AutopilotSettings, "*").unwrap();
    assert_eq!((row["mode"].clone(), row["bypass_until"].clone()), (json!("lockdown"), json!(null)));
    // Lifted on purpose, by a primary that saw the Lockdown.
    let seen = version(&conn, Table::AutopilotSettings, "*");
    let lift =
        w.op("p", 3, OpClass::Primary, put(Table::AutopilotSettings, "*", Some(seen), json!({"mode": "manual"})));
    run(&mut conn, &w, 6, &[lift]);
    assert_eq!(value(&conn, Table::AutopilotSettings, "*").unwrap()["mode"], "manual");
}

#[test]
fn removals_beat_concurrent_edits_and_activity_only_grows() {
    let mut w = World::new(false);
    let mut conn = store();
    let add = w.op("p", 3, OpClass::Primary, put(Table::Accounts, "github/octo", None, json!({"added_at": 1})));
    run(&mut conn, &w, 1, &[add]);
    let v1 = version(&conn, Table::Accounts, "github/octo");
    let gone = w.op("p", 3, OpClass::Primary, remove(Table::Accounts, "github/octo"));
    let stale = w.op("p", 3, OpClass::Primary, put(Table::Accounts, "github/octo", Some(v1), json!({"added_at": 2})));
    assert_eq!(run(&mut conn, &w, 2, &[gone, stale]), [Outcome::Applied, Outcome::Superseded]);
    assert_eq!(value(&conn, Table::Accounts, "github/octo"), None);
    // Added again by a writer that saw the removal.
    let tomb = version(&conn, Table::Accounts, "github/octo");
    let again = w.op("p", 3, OpClass::Primary, put(Table::Accounts, "github/octo", Some(tomb), json!({"added_at": 3})));
    assert_eq!(run(&mut conn, &w, 4, &[again]), [Outcome::Applied]);
    // A removed grant never comes back, whoever saw what.
    let g = w.op("p", 3, OpClass::Primary, put(Table::Grants, "g1", None, grant_row(false, 1)));
    let g_gone = w.op("p", 3, OpClass::Primary, remove(Table::Grants, "g1"));
    run(&mut conn, &w, 5, &[g, g_gone]);
    let tomb = version(&conn, Table::Grants, "g1");
    let back = w.op("p", 3, OpClass::Primary, put(Table::Grants, "g1", Some(tomb), grant_row(false, 1)));
    assert_eq!(run(&mut conn, &w, 7, &[back]), [Outcome::Superseded]);
    // Activity: an entry never changes and is never removed by an op.
    let entry = w.op("p", 3, OpClass::Primary, put(Table::Audit, "op-1", None, json!({"detail": "first"})));
    let changed = w.op("p", 3, OpClass::Primary, put(Table::Audit, "op-1", None, json!({"detail": "rewritten"})));
    let dropped = w.op("p", 3, OpClass::Primary, remove(Table::Audit, "op-1"));
    assert_eq!(
        run(&mut conn, &w, 8, &[entry, changed, dropped]),
        [Outcome::Applied, Outcome::Superseded, Outcome::Superseded]
    );
    assert_eq!(value(&conn, Table::Audit, "op-1").unwrap()["detail"], "first");
}

#[test]
fn json_settings_merge_by_entry_and_asking_every_time_wins() {
    let mut w = World::new(false);
    let mut conn = store();
    let first = w.op("p", 3, OpClass::Primary, put(Table::Meta, "secrets.always-ask", None, json!({"a": true})));
    let other =
        w.op("p", 3, OpClass::Primary, put(Table::Meta, "secrets.always-ask", None, json!({"a": false, "b": true})));
    run(&mut conn, &w, 1, &[first, other]);
    assert_eq!(value(&conn, Table::Meta, "secrets.always-ask").unwrap(), json!({"a": true, "b": true}));
}

#[test]
fn a_replaced_primary_keeps_only_the_brake() {
    let mut w = World::new(true);
    let mut conn = store();
    // History before the handover (signed at epoch 3, applied before the log reached epoch 4): p's ops are good.
    let before = w.op("p", 3, OpClass::Primary, put(Table::Meta, "a", None, json!(1)));
    // The new primary writes at epoch 4.
    let after = w.op("v", 4, OpClass::Primary, put(Table::Meta, "b", None, json!(2)));
    // p's late ops, signed at epoch 3 (a viewer now): its permissive one is refused, its brake applies.
    let late = w.op("p", 3, OpClass::Primary, put(Table::Meta, "c", None, json!(3)));
    let late_brake = w.op("p", 3, OpClass::Restrictive, Change::Restrict(Restriction::Lockdown));
    // p's ops signed at epoch 4 (as a viewer now): no primary op, and the brake as any viewer.
    let as_viewer = w.op("p", 4, OpClass::Primary, put(Table::Meta, "d", None, json!(4)));
    assert_eq!(
        run(&mut conn, &w, 1, &[before, after, late, late_brake, as_viewer]),
        [
            Outcome::Applied,
            Outcome::Applied,
            Outcome::Refused(Refusal::NotAuthorized(OpRefusal::NotPrimary)),
            Outcome::Applied,
            Outcome::Refused(Refusal::NotAuthorized(OpRefusal::NotPrimary)),
        ]
    );
    assert_eq!(value(&conn, Table::AutopilotSettings, "*").unwrap()["mode"], "lockdown");
    assert_eq!(state(&conn).unwrap().log_epoch, 4);
}

#[test]
fn proposals_wait_for_the_primary_and_never_come_back() {
    let mut w = World::new(false);
    let mut conn = store();
    let propose = w.op("w", 3, OpClass::Proposal, put(Table::Proposals, "w/github", None, json!({"sealed": "x"})));
    let other = w.op("v", 3, OpClass::Proposal, put(Table::Proposals, "v/mcp", None, json!({"sealed": "y"})));
    run(&mut conn, &w, 1, &[propose, other]);
    let waiting = proposals(&conn, NOW).unwrap();
    assert_eq!(
        waiting.iter().map(|p| (p.key.as_str(), p.author.as_str())).collect::<Vec<_>>(),
        [("w/github", "w"), ("v/mcp", "v")]
    );
    // The primary resolves one; the viewer withdraws the other. A copy of the first put does not bring it back.
    let resolve = w.op("p", 3, OpClass::Primary, remove(Table::Proposals, "w/github"));
    let withdraw = w.op("v", 3, OpClass::Proposal, remove(Table::Proposals, "v/mcp"));
    let again = w.op("w", 3, OpClass::Proposal, put(Table::Proposals, "w/github", None, json!({"sealed": "x"})));
    assert_eq!(
        run(&mut conn, &w, 3, &[resolve, withdraw, again]),
        [Outcome::Applied, Outcome::Applied, Outcome::Superseded]
    );
    assert!(proposals(&conn, NOW).unwrap().is_empty());
    // The primary never writes a proposal; and an unresolved one expires after a week.
    let primary_put = w.op("p", 3, OpClass::Primary, put(Table::Proposals, "p/x", None, json!({})));
    let fresh = w.op("w", 3, OpClass::Proposal, put(Table::Proposals, "w/vault", None, json!({})));
    assert_eq!(
        run(&mut conn, &w, 6, &[primary_put, fresh]),
        [Outcome::Refused(Refusal::Change(ChangeRefusal::PrimaryProposal)), Outcome::Applied]
    );
    assert_eq!(proposals(&conn, NOW).unwrap().len(), 1);
    assert!(proposals(&conn, NOW + PROPOSAL_TTL_SECS).unwrap().is_empty());
}

#[test]
fn tombstones_go_once_every_member_passed_them_or_after_the_horizon() {
    let mut w = World::new(false);
    let mut conn = store();
    let add = w.op("p", 3, OpClass::Primary, put(Table::Accounts, "a", None, json!({})));
    let gone = w.op("p", 3, OpClass::Primary, remove(Table::Accounts, "a"));
    let add_b = w.op("p", 3, OpClass::Primary, put(Table::Accounts, "b", None, json!({})));
    let gone_b = w.op("p", 3, OpClass::Primary, remove(Table::Accounts, "b"));
    run(&mut conn, &w, 1, &[add, gone, add_b, gone_b]);
    // Members acknowledged up to seq 2: the first tombstone goes, the second stays.
    assert_eq!(gc(&conn, 2, NOW).unwrap(), 1);
    assert!(record(&conn, Table::Accounts, "a").unwrap().is_none());
    assert!(record(&conn, Table::Accounts, "b").unwrap().is_some());
    assert_eq!(gc(&conn, 2, NOW).unwrap(), 0);
    // Past the horizon, whoever lags.
    assert_eq!(gc(&conn, 0, NOW + TOMBSTONE_HORIZON_SECS + 1).unwrap(), 1);
}

#[test]
fn checkpoints_load_on_a_new_phone_and_refuse_forgery_rollback_and_forks() {
    let mut w = World::new(false);
    let mut primary = store();
    let ops = [
        w.op("p", 3, OpClass::Primary, put(Table::Grants, "g1", None, grant_row(false, 9_000))),
        w.op("p", 3, OpClass::Primary, put(Table::Meta, "k", None, json!("v"))),
        w.op("w", 3, OpClass::Proposal, put(Table::Proposals, "w/1", None, json!({}))),
        w.op("p", 3, OpClass::Primary, remove(Table::Meta, "k")),
    ];
    run(&mut primary, &w, 1, &ops);
    let cp = checkpoint::produce(&primary, &w.ctx(), &key_of("p"), "p", 3).unwrap();
    assert_eq!(cp.seq, 4);
    // A new phone loads it: the same records, proposals, counters and history.
    let mut fresh = store();
    assert_eq!(checkpoint::load(&mut fresh, &w.ctx(), &cp).unwrap(), CheckpointCheck::Loaded);
    assert_eq!(state(&fresh).unwrap(), state(&primary).unwrap());
    assert_eq!(record(&fresh, Table::Grants, "g1").unwrap(), record(&primary, Table::Grants, "g1").unwrap());
    assert_eq!(proposals(&fresh, NOW).unwrap(), proposals(&primary, NOW).unwrap());
    // It applies the next op as the primary does, counters included.
    let more = w.op("p", 3, OpClass::Primary, put(Table::Meta, "x", None, json!(1)));
    assert_eq!(run(&mut fresh, &w, 5, std::slice::from_ref(&more)), run(&mut primary, &w, 5, &[more]));
    let mut replayed = w.op("p", 3, OpClass::Primary, put(Table::Meta, "y", None, json!(1)));
    replayed.counter = 1;
    replayed.sig = key_of("p").sign_b64(&replayed.signing_bytes().unwrap());
    assert_eq!(run(&mut fresh, &w, 6, &[replayed]), [Outcome::Refused(Refusal::CounterReplay)]);
    // Loading it again where the store is past it: compared, not loaded.
    assert_eq!(checkpoint::load(&mut primary, &w.ctx(), &cp).unwrap(), CheckpointCheck::Matches);

    // Forged: signed by a viewer, for another account, a tampered body, or another checkpoint's body.
    let mut by_viewer = cp.clone();
    by_viewer.author = "w".to_owned();
    by_viewer.sig = key_of("w").sign_b64(&by_viewer.signing_bytes().unwrap());
    let mut other_account = cp.clone();
    other_account.account = "acct2".to_owned();
    let mut tampered = cp.clone();
    let mut raw = decode_payload(&tampered.body).unwrap();
    *raw.last_mut().unwrap() ^= 1;
    tampered.body = encode_payload(&raw);
    let mut moved = checkpoint::produce(&primary, &w.ctx(), &key_of("p"), "p", 3).unwrap();
    moved.body = cp.body.clone();
    moved.sig = key_of("p").sign_b64(&moved.signing_bytes().unwrap());
    for bad in [by_viewer, other_account, tampered, moved] {
        assert!(checkpoint::load(&mut store(), &w.ctx(), &bad).is_err(), "{bad:?}");
    }
    // Ahead in seq but signed at an epoch older than the log showed: a rollback.
    let mut lagging = store();
    let late = w.op("p", 3, OpClass::Primary, put(Table::Meta, "z", None, json!(1)));
    run(&mut lagging, &w, 1, &[late]);
    lagging.execute("UPDATE oplog_state SET log_epoch = 9", []).unwrap();
    assert_eq!(checkpoint::load(&mut lagging, &w.ctx(), &cp), Err(Halt::Rollback));
    // A phone that applied other ops up to the checkpoint's seq saw another history: a fork.
    let mut other = store();
    let mut x = World::new(false);
    x.owner = OwnerSeed::from_bytes(w.owner.as_bytes()).unwrap();
    x.keyring = w.keyring.clone();
    x.roster = w.roster.clone();
    let others: Vec<Op> =
        (0..4).map(|i| x.op("p", 3, OpClass::Primary, put(Table::Meta, &format!("o{i}"), None, json!(i)))).collect();
    run(&mut other, &x, 1, &others);
    assert!(matches!(checkpoint::load(&mut other, &w.ctx(), &cp), Err(Halt::Forked(_))));
}

#[test]
fn heartbeats_tell_live_from_behind_and_catch_a_fork() {
    let mut w = World::new(false);
    let mut primary = store();
    let mut viewer = store();
    let ops = [
        w.op("p", 3, OpClass::Primary, put(Table::Meta, "a", None, json!(1))),
        w.op("p", 3, OpClass::Primary, put(Table::Meta, "b", None, json!(2))),
    ];
    run(&mut primary, &w, 1, &ops);
    run(&mut viewer, &w, 1, &ops[..1]);
    let hb = checkpoint::heartbeat(&primary, &w.ctx(), &key_of("p"), "p", 3).unwrap();
    assert_eq!(check_heartbeat(&viewer, &w.ctx(), &hb).unwrap(), HeartbeatCheck::Behind);
    run(&mut viewer, &w, 2, &ops[1..]);
    assert_eq!(check_heartbeat(&viewer, &w.ctx(), &hb).unwrap(), HeartbeatCheck::Matches);
    // Signed by a viewer, or tampered: refused.
    let mut forged = hb.clone();
    forged.author = "v".to_owned();
    forged.sig = key_of("v").sign_b64(&forged.signing_bytes().unwrap());
    assert!(check_heartbeat(&viewer, &w.ctx(), &forged).is_err());
    let mut tampered = hb.clone();
    tampered.head_seq = 1;
    assert!(check_heartbeat(&viewer, &w.ctx(), &tampered).is_err());
    // From an epoch older than the log showed: stale, ignored.
    viewer.execute("UPDATE oplog_state SET log_epoch = 9", []).unwrap();
    assert_eq!(check_heartbeat(&viewer, &w.ctx(), &hb).unwrap(), HeartbeatCheck::Stale);
    viewer.execute("UPDATE oplog_state SET log_epoch = 3", []).unwrap();
    // The primary signs another hash for seq 2 than this phone has: a fork, and applying stops.
    let mut other = hb;
    other.applied_hash = hex32(&[7; 32]);
    other.sig = key_of("p").sign_b64(&other.signing_bytes().unwrap());
    assert!(matches!(check_heartbeat(&viewer, &w.ctx(), &other), Err(Halt::Forked(_))));
    let next = w.op("p", 3, OpClass::Primary, put(Table::Meta, "c", None, json!(3)));
    assert!(matches!(
        apply(
            &mut viewer,
            &w.ctx(),
            &SyncedOp {
                seq: 3,
                op: next
            }
        ),
        Err(Halt::Forked(_))
    ));
}

#[test]
fn two_phones_that_apply_the_same_log_agree_whatever_it_holds() {
    let mut w = World::new(true);
    let (mut a, mut b) = (store(), store());
    let mut bad_sig = w.op("p", 3, OpClass::Primary, put(Table::Meta, "x", None, json!(1)));
    bad_sig.sig = key_of("w").sign_b64(&bad_sig.signing_bytes().unwrap());
    let ops = [
        w.op("p", 3, OpClass::Primary, put(Table::Grants, "g1", None, grant_row(false, 5))),
        bad_sig,
        w.op("w", 4, OpClass::Restrictive, Change::Restrict(Restriction::StopBypass)),
        w.op("v", 4, OpClass::Primary, put(Table::Meta, "y", None, json!(2))),
        w.op("p", 3, OpClass::Primary, put(Table::Meta, "late", None, json!(3))),
    ];
    assert_eq!(run(&mut a, &w, 1, &ops), run(&mut b, &w, 1, &ops));
    assert_eq!(state(&a).unwrap(), state(&b).unwrap());
    assert_eq!(records(&a, Table::Meta).unwrap(), records(&b, Table::Meta).unwrap());
}

#[test]
fn the_store_keeps_the_log_in_its_sealed_database() {
    let mut w = World::new(false);
    let dir = tempfile::tempdir().unwrap();
    let op = w.op("p", 3, OpClass::Primary, put(Table::Meta, "k", None, json!("v")));
    {
        let store = crate::store::tests::open(dir.path());
        let applied = store
            .oplog(|c| {
                apply(
                    c,
                    &w.ctx(),
                    &SyncedOp {
                        seq: 1,
                        op,
                    },
                )
            })
            .unwrap();
        assert_eq!(applied, Ok(Outcome::Applied));
    }
    let store = crate::store::tests::open(dir.path());
    assert_eq!(store.oplog(|c| state(c).unwrap().seq).unwrap(), 1);
    assert_eq!(store.oplog(|c| value(c, Table::Meta, "k")).unwrap(), Some(json!("v")));
    // The op log is this phone's own bookkeeping: never in a portable snapshot.
    let exported = String::from_utf8(store.export_account().unwrap().to_vec()).unwrap();
    assert!(!exported.contains("oplog"));
}
