//! Deferred decisions (Decisions v2, 03 §9): tickets, their transitions, the approval device's public keys and the
//! connections' webhooks.
//!
//! What is stored in clear is metadata only: the ticket, its state, deadlines, connection and the request digest. A
//! request's body is sealed to the approval device before it gets here (`sealed_body`); so is an agent's reply to a
//! "Why?". The reviewer's note and question, and a write's minimal result, are in clear: the agent, which has no key,
//! must be able to read them.
//!
//! State changes are compare-and-swap on `state` ([`ReinsTicket::update`]), so the same code is atomic on SQLite,
//! MySQL and PostgreSQL without row locks.

use diesel::prelude::*;

use crate::{
    api::EmptyResult,
    db::{
        DbConn, DbConnInner,
        schema::{reins_callbacks, reins_device_keys, reins_ticket_events, reins_tickets},
    },
    error::MapResult,
};

/// States a ticket waits for a reviewer in (`reins_proto::ticket::TicketState::is_open`).
pub const OPEN_STATES: [&str; 6] = ["received", "reviewing", "deferred", "needs_info", "escalated", "stale"];
/// States after which nothing more happens.
pub const TERMINAL_STATES: [&str; 6] = ["done", "failed", "unknown", "denied", "expired", "cancelled"];

#[derive(Clone, Debug, PartialEq, Eq, Queryable, Insertable, AsChangeset)]
#[diesel(table_name = reins_tickets)]
#[diesel(treat_none_as_null = true)]
pub struct ReinsTicket {
    /// The request id; agents see `rt_<id>`.
    pub id: String,
    pub user_uuid: String,
    pub connection_uuid: String,
    pub connection_label: String,
    pub state: String,
    pub digest: String,
    /// `on_approve` or `on_collect` (`reins_proto::ticket::Execution`).
    pub execution: String,
    /// `deny` or `escalate`.
    pub on_expiry: String,
    /// How an approved ticket ends: `collect` (done when an agent received its result), `report` (a parked push: done
    /// when the desktop app reports it pushed) or `run` (a write the phone runs).
    pub finish: String,
    pub created_at: i64,
    /// When it became a ticket: the Deferred window's start.
    pub deferred_at: i64,
    pub deadline: i64,
    pub window_secs: i64,
    pub escalated: bool,
    /// When the approval device approved it.
    pub approved_at: Option<i64>,
    /// How long an approval stays usable (03 §6).
    pub valid_secs: i64,
    /// `SealedTicketBody`, sealed to the approval device's box key (base64url).
    pub sealed_body: String,
    /// The reviewer's open "Why?".
    pub question: Option<String>,
    /// The agent's answer to it, sealed to the approval device's box key (base64url).
    pub sealed_reply: Option<String>,
    /// The reviewer's note to the agent (a denial).
    pub note: Option<String>,
    /// A write's minimal result (JSON).
    pub result: Option<String>,
    /// What changed since the approval.
    pub stale: Option<String>,
    /// The approval device has seen this version.
    pub delivered: bool,
    pub updated_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Queryable, Insertable)]
#[diesel(table_name = reins_ticket_events)]
pub struct ReinsTicketEvent {
    pub ticket_id: String,
    pub seq: i32,
    pub state: String,
    /// A `SignedTicketEvent` the approval device signed; otherwise the server's (or the desktop app's) record.
    pub signed: bool,
    pub event: String,
    pub created_at: i64,
}

/// The approval device's public keys, as it published them.
#[derive(Clone, Debug, PartialEq, Eq, Queryable, Insertable, AsChangeset)]
#[diesel(table_name = reins_device_keys)]
pub struct ReinsDeviceKeys {
    pub user_uuid: String,
    pub device_uuid: String,
    /// Ed25519, base64url.
    pub sign_pub: String,
    /// X25519, base64url.
    pub box_pub: String,
    pub updated_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Queryable, Insertable, AsChangeset)]
#[diesel(table_name = reins_callbacks)]
pub struct ReinsCallback {
    pub connection_uuid: String,
    pub url: String,
    pub verified_at: i64,
}

impl ReinsTicket {
    #[must_use]
    pub fn is_open(&self) -> bool {
        OPEN_STATES.contains(&self.state.as_str())
    }

    pub async fn insert(&self, conn: &DbConn) -> EmptyResult {
        conn.run(move |c| q_insert(c, self)).await.map_res("Error saving a Reins ticket")
    }

    pub async fn find(id: &str, conn: &DbConn) -> Option<Self> {
        conn.run(move |c| q_find(c, id)).await
    }

    /// `id`, only when `connection` made it: an agent never learns another connection's tickets exist.
    pub async fn find_for_connection(id: &str, connection: &str, conn: &DbConn) -> Option<Self> {
        conn.run(move |c| q_find(c, id).filter(|t| t.connection_uuid == connection)).await
    }

    /// `id`, only when it is `user`'s.
    pub async fn find_for_user(id: &str, user: &str, conn: &DbConn) -> Option<Self> {
        conn.run(move |c| q_find(c, id).filter(|t| t.user_uuid == user)).await
    }

    /// The open ticket of `connection` for the same request, if any (duplicates share a ticket).
    pub async fn open_by_digest(connection: &str, digest: &str, conn: &DbConn) -> Option<Self> {
        conn.run(move |c| q_open_by_digest(c, connection, digest)).await
    }

    pub async fn count_open(connection: &str, conn: &DbConn) -> i64 {
        conn.run(move |c| q_count_open(c, connection)).await
    }

    /// `connection`'s open and approved tickets, and those that closed at `since` or later, newest first.
    pub async fn list_for_connection(connection: &str, since: i64, conn: &DbConn) -> Vec<Self> {
        conn.run(move |c| q_list_for_connection(c, connection, since)).await
    }

    /// `user`'s tickets the approval device must know about: open ones and approved ones not yet run.
    pub async fn list_for_user(user: &str, conn: &DbConn) -> Vec<Self> {
        conn.run(move |c| q_list_for_user(c, user)).await
    }

    /// `user`'s tickets the approval device has not seen in their current version.
    pub async fn undelivered(user: &str, conn: &DbConn) -> Vec<Self> {
        conn.run(move |c| q_undelivered(c, user)).await
    }

    /// Open tickets whose window ended at `now` or before, and approved ones whose approval is no longer usable.
    pub async fn due(now: i64, conn: &DbConn) -> Vec<Self> {
        conn.run(move |c| q_due(c, now)).await
    }

    /// Writes `self` if the stored state is still `from`; `false` when someone moved it first.
    pub async fn update(&self, from: &str, conn: &DbConn) -> Result<bool, crate::Error> {
        conn.run(move |c| q_update(c, self, from)).await.map_res("Error updating a Reins ticket")
    }

    /// Removes a ticket and its events (one that never took: the phone answered first).
    pub async fn delete(id: &str, conn: &DbConn) -> EmptyResult {
        conn.run(move |c| {
            diesel::delete(reins_ticket_events::table.filter(reins_ticket_events::ticket_id.eq(id))).execute(c)?;
            diesel::delete(reins_tickets::table.find(id)).execute(c).map(|_| ())
        })
        .await
        .map_res("Error deleting a Reins ticket")
    }

    pub async fn mark_delivered(ids: Vec<String>, conn: &DbConn) -> EmptyResult {
        conn.run(move |c| q_mark_delivered(c, &ids)).await.map_res("Error updating Reins tickets")
    }

    /// Drops closed tickets last changed before `before`.
    pub async fn purge(before: i64, conn: &DbConn) -> EmptyResult {
        conn.run(move |c| q_purge(c, before)).await.map_res("Error purging Reins tickets")
    }
}

impl ReinsTicketEvent {
    pub async fn append(ticket: &str, state: &str, signed: bool, event: &str, now: i64, conn: &DbConn) -> EmptyResult {
        conn.run(move |c| q_append_event(c, ticket, state, signed, event, now))
            .await
            .map_res("Error saving a Reins ticket event")
    }

    pub async fn list(ticket: &str, conn: &DbConn) -> Vec<Self> {
        conn.run(move |c| q_events(c, ticket)).await
    }
}

impl ReinsDeviceKeys {
    /// `user`'s keys as last published (check `device_uuid` against the current approval device).
    pub async fn find(user: &str, conn: &DbConn) -> Option<Self> {
        conn.run(move |c| reins_device_keys::table.find(user).first::<Self>(c).ok()).await
    }

    pub async fn save(&self, conn: &DbConn) -> EmptyResult {
        conn.run(move |c| q_save_keys(c, self)).await.map_res("Error saving Reins device keys")
    }
}

impl ReinsCallback {
    pub async fn find(connection: &str, conn: &DbConn) -> Option<Self> {
        conn.run(move |c| reins_callbacks::table.find(connection).first::<Self>(c).ok()).await
    }

    pub async fn save(&self, conn: &DbConn) -> EmptyResult {
        conn.run(move |c| q_save_callback(c, self)).await.map_res("Error saving a Reins webhook")
    }

    pub async fn delete(connection: &str, conn: &DbConn) -> EmptyResult {
        conn.run(move |c| diesel::delete(reins_callbacks::table.find(connection)).execute(c).map(|_| ()))
            .await
            .map_res("Error deleting a Reins webhook")
    }
}

fn q_insert(c: &mut DbConnInner, row: &ReinsTicket) -> QueryResult<()> {
    diesel::insert_into(reins_tickets::table).values(row).execute(c).map(|_| ())
}

fn q_find(c: &mut DbConnInner, id: &str) -> Option<ReinsTicket> {
    reins_tickets::table.find(id).first::<ReinsTicket>(c).ok()
}

fn q_open_by_digest(c: &mut DbConnInner, connection: &str, digest: &str) -> Option<ReinsTicket> {
    reins_tickets::table
        .filter(reins_tickets::connection_uuid.eq(connection))
        .filter(reins_tickets::digest.eq(digest))
        .filter(reins_tickets::state.eq_any(OPEN_STATES))
        .first::<ReinsTicket>(c)
        .ok()
}

fn q_count_open(c: &mut DbConnInner, connection: &str) -> i64 {
    reins_tickets::table
        .filter(reins_tickets::connection_uuid.eq(connection))
        .filter(reins_tickets::state.eq_any(OPEN_STATES))
        .count()
        .get_result(c)
        .unwrap_or(0)
}

fn q_list_for_connection(c: &mut DbConnInner, connection: &str, since: i64) -> Vec<ReinsTicket> {
    reins_tickets::table
        .filter(reins_tickets::connection_uuid.eq(connection))
        .filter(reins_tickets::state.ne_all(TERMINAL_STATES).or(reins_tickets::updated_at.ge(since)))
        .order(reins_tickets::created_at.desc())
        .limit(200)
        .load::<ReinsTicket>(c)
        .unwrap_or_default()
}

fn q_list_for_user(c: &mut DbConnInner, user: &str) -> Vec<ReinsTicket> {
    reins_tickets::table
        .filter(reins_tickets::user_uuid.eq(user))
        .filter(reins_tickets::state.ne_all(TERMINAL_STATES))
        .order(reins_tickets::deadline.asc())
        .limit(500)
        .load::<ReinsTicket>(c)
        .unwrap_or_default()
}

fn q_undelivered(c: &mut DbConnInner, user: &str) -> Vec<ReinsTicket> {
    reins_tickets::table
        .filter(reins_tickets::user_uuid.eq(user))
        .filter(reins_tickets::delivered.eq(false))
        .order(reins_tickets::created_at.asc())
        .limit(100)
        .load::<ReinsTicket>(c)
        .unwrap_or_default()
}

fn q_due(c: &mut DbConnInner, now: i64) -> Vec<ReinsTicket> {
    let mut due: Vec<ReinsTicket> = reins_tickets::table
        .filter(reins_tickets::state.eq_any(["received", "reviewing", "deferred", "needs_info", "escalated", "stale"]))
        .filter(reins_tickets::deadline.le(now))
        .limit(500)
        .load::<ReinsTicket>(c)
        .unwrap_or_default();
    let approved: Vec<ReinsTicket> = reins_tickets::table
        .filter(reins_tickets::state.eq("approved"))
        .order(reins_tickets::approved_at.asc())
        .limit(500)
        .load::<ReinsTicket>(c)
        .unwrap_or_default();
    due.extend(approved.into_iter().filter(|t| t.approved_at.is_some_and(|at| at.saturating_add(t.valid_secs) <= now)));
    due
}

fn q_update(c: &mut DbConnInner, row: &ReinsTicket, from: &str) -> QueryResult<bool> {
    let n = diesel::update(
        reins_tickets::table.filter(reins_tickets::id.eq(&row.id)).filter(reins_tickets::state.eq(from)),
    )
    .set(row)
    .execute(c)?;
    Ok(n == 1)
}

fn q_mark_delivered(c: &mut DbConnInner, ids: &[String]) -> QueryResult<()> {
    diesel::update(reins_tickets::table.filter(reins_tickets::id.eq_any(ids)))
        .set(reins_tickets::delivered.eq(true))
        .execute(c)
        .map(|_| ())
}

fn q_purge(c: &mut DbConnInner, before: i64) -> QueryResult<()> {
    let old = reins_tickets::table
        .filter(reins_tickets::state.eq_any(TERMINAL_STATES))
        .filter(reins_tickets::updated_at.lt(before))
        .select(reins_tickets::id);
    let ids: Vec<String> = old.load(c)?;
    diesel::delete(reins_ticket_events::table.filter(reins_ticket_events::ticket_id.eq_any(&ids))).execute(c)?;
    diesel::delete(reins_tickets::table.filter(reins_tickets::id.eq_any(&ids))).execute(c).map(|_| ())
}

fn q_append_event(
    c: &mut DbConnInner,
    ticket: &str,
    state: &str,
    signed: bool,
    event: &str,
    now: i64,
) -> QueryResult<()> {
    // Two appends racing for one `seq` collide on the primary key; the loser takes the next.
    for _ in 0..3 {
        let last: Option<i32> = reins_ticket_events::table
            .filter(reins_ticket_events::ticket_id.eq(ticket))
            .select(diesel::dsl::max(reins_ticket_events::seq))
            .first(c)?;
        let row = ReinsTicketEvent {
            ticket_id: ticket.to_owned(),
            seq: last.unwrap_or(0).saturating_add(1),
            state: state.to_owned(),
            signed,
            event: event.to_owned(),
            created_at: now,
        };
        match diesel::insert_into(reins_ticket_events::table).values(&row).execute(c) {
            Ok(_) => return Ok(()),
            Err(diesel::result::Error::DatabaseError(diesel::result::DatabaseErrorKind::UniqueViolation, _)) => {}
            Err(e) => return Err(e),
        }
    }
    Err(diesel::result::Error::RollbackTransaction)
}

fn q_events(c: &mut DbConnInner, ticket: &str) -> Vec<ReinsTicketEvent> {
    reins_ticket_events::table
        .filter(reins_ticket_events::ticket_id.eq(ticket))
        .order(reins_ticket_events::seq.asc())
        .load::<ReinsTicketEvent>(c)
        .unwrap_or_default()
}

fn q_save_keys(c: &mut DbConnInner, row: &ReinsDeviceKeys) -> QueryResult<()> {
    c.transaction(|c| {
        diesel::delete(reins_device_keys::table.find(&row.user_uuid)).execute(c)?;
        diesel::insert_into(reins_device_keys::table).values(row).execute(c).map(|_| ())
    })
}

fn q_save_callback(c: &mut DbConnInner, row: &ReinsCallback) -> QueryResult<()> {
    c.transaction(|c| {
        diesel::delete(reins_callbacks::table.find(&row.connection_uuid)).execute(c)?;
        diesel::insert_into(reins_callbacks::table).values(row).execute(c).map(|_| ())
    })
}

#[cfg(all(test, sqlite))]
mod tests {
    use super::*;
    use crate::db::models::reins_device::test_db;

    pub fn ticket(id: &str, connection: &str, state: &str, deadline: i64) -> ReinsTicket {
        ReinsTicket {
            id: id.to_owned(),
            user_uuid: "u1".to_owned(),
            connection_uuid: connection.to_owned(),
            connection_label: "Claude".to_owned(),
            state: state.to_owned(),
            digest: format!("{id:0>64}"),
            execution: "on_approve".to_owned(),
            on_expiry: "deny".to_owned(),
            finish: "run".to_owned(),
            created_at: 1,
            deferred_at: 1,
            deadline,
            window_secs: 100,
            escalated: false,
            approved_at: None,
            valid_secs: 86_400,
            sealed_body: "sealed".to_owned(),
            question: None,
            sealed_reply: None,
            note: None,
            result: None,
            stale: None,
            delivered: false,
            updated_at: 1,
        }
    }

    #[test]
    fn ticket_rows_round_trip_on_sqlite() {
        let mut c = test_db();
        q_insert(&mut c, &ticket("t1", "c1", "deferred", 100)).unwrap();
        q_insert(&mut c, &ticket("t2", "c1", "done", 100)).unwrap();
        q_insert(&mut c, &ticket("t3", "c2", "deferred", 50)).unwrap();
        assert_eq!(q_find(&mut c, "t1").unwrap().connection_uuid, "c1");
        assert_eq!(q_count_open(&mut c, "c1"), 1, "terminal rows do not count");
        assert!(q_open_by_digest(&mut c, "c1", &format!("{:0>64}", "t1")).is_some());
        assert!(q_open_by_digest(&mut c, "c2", &format!("{:0>64}", "t1")).is_none(), "another connection's");
        assert!(q_open_by_digest(&mut c, "c1", &format!("{:0>64}", "t2")).is_none(), "a closed ticket is not shared");

        let mut moved = q_find(&mut c, "t1").unwrap();
        moved.state = "approved".to_owned();
        moved.approved_at = Some(10);
        assert!(q_update(&mut c, &moved, "deferred").unwrap());
        assert!(!q_update(&mut c, &moved, "deferred").unwrap(), "the compare-and-swap lost");

        let due: Vec<String> = q_due(&mut c, 60).into_iter().map(|t| t.id).collect();
        assert_eq!(due, ["t3"], "only past deadlines; approved t1 is valid until 86 410");
        q_insert(&mut c, &ticket("t4", "c3", "reviewing", 30)).unwrap();
        q_insert(&mut c, &ticket("t5", "c3", "received", 30)).unwrap();
        let mut due: Vec<String> = q_due(&mut c, 60).into_iter().map(|t| t.id).collect();
        due.sort();
        assert_eq!(due, ["t3", "t4", "t5"], "every state that waits for a reviewer runs out");
        for id in ["t4", "t5"] {
            diesel::delete(reins_tickets::table.find(id)).execute(&mut c).unwrap();
        }
        let due: Vec<String> = q_due(&mut c, 86_410).into_iter().map(|t| t.id).collect();
        assert_eq!(due, ["t3", "t1"]);

        let listed: Vec<String> = q_list_for_connection(&mut c, "c1", 5).into_iter().map(|t| t.id).collect();
        assert_eq!(listed, ["t1"], "t2 closed before `since`");
        assert_eq!(q_list_for_connection(&mut c, "c1", 0).len(), 2);
        assert_eq!(q_undelivered(&mut c, "u1").len(), 3);
        q_mark_delivered(&mut c, &["t1".to_owned()]).unwrap();
        assert_eq!(q_undelivered(&mut c, "u1").len(), 2);

        q_append_event(&mut c, "t1", "deferred", false, "{}", 1).unwrap();
        q_append_event(&mut c, "t1", "approved", true, "{}", 2).unwrap();
        let seqs: Vec<i32> = q_events(&mut c, "t1").into_iter().map(|e| e.seq).collect();
        assert_eq!(seqs, [1, 2]);
        q_purge(&mut c, 2).unwrap();
        assert!(q_find(&mut c, "t2").is_none(), "closed and old");
        assert!(q_find(&mut c, "t1").is_some());
    }

    #[test]
    fn keys_and_callbacks_are_replaced_not_added() {
        let mut c = test_db();
        let keys = |dev: &str| ReinsDeviceKeys {
            user_uuid: "u1".to_owned(),
            device_uuid: dev.to_owned(),
            sign_pub: "s".to_owned(),
            box_pub: "b".to_owned(),
            updated_at: 1,
        };
        q_save_keys(&mut c, &keys("d1")).unwrap();
        q_save_keys(&mut c, &keys("d2")).unwrap();
        let rows: Vec<ReinsDeviceKeys> = reins_device_keys::table.load(&mut c).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].device_uuid, "d2");
        let cb = |url: &str| ReinsCallback {
            connection_uuid: "c1".to_owned(),
            url: url.to_owned(),
            verified_at: 1,
        };
        q_save_callback(&mut c, &cb("https://a")).unwrap();
        q_save_callback(&mut c, &cb("https://b")).unwrap();
        let rows: Vec<ReinsCallback> = reins_callbacks::table.load(&mut c).unwrap();
        assert_eq!(rows, [cb("https://b")]);
    }
}
