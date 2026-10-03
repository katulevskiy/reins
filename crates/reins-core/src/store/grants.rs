//! Grant rows and the atomic evaluate-and-reserve operations.
//!
//! Every method that evaluates grants holds the store mutex and runs inside
//! one `BEGIN IMMEDIATE` transaction, so two concurrent requests can never both
//! spend the last use of a grant.

use std::collections::BTreeSet;

use rewarden_policy::{
    Grant, MessageFacts, ReadDecision, SendDecision, evaluate_read, evaluate_send, needs_body, record_uses,
};
use rewarden_proto::gmail::OutgoingEmail;
use rewarden_proto::ids::{ConnectionId, GrantId};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use super::Store;
use crate::CoreError;

/// A grant with the label of its connection at creation time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredGrant {
    pub grant: Grant,
    pub connection_label: String,
    /// Unix seconds of the last request this grant covered.
    pub last_used_at: Option<i64>,
    /// "approval" (chosen while approving), "ai_request" (the AI asked and the user agreed), "user" (created
    /// ahead of time) or "retry" (a one-time pass after a late approval).
    pub origin: String,
}

fn parse_row(id: &str, connection_id: &str, json: &str) -> Option<Grant> {
    match serde_json::from_str::<Grant>(json) {
        Ok(g) if g.id.0 == id && g.connection_id.0 == connection_id => Some(g),
        Ok(_) => {
            log::warn!("grant row {id} does not match its content; ignored");
            None
        }
        Err(e) => {
            log::warn!("grant row {id} is corrupt ({e}); ignored");
            None
        }
    }
}

fn all_connection_grants(conn: &Connection, connection: &ConnectionId) -> Result<Vec<Grant>, CoreError> {
    let mut stmt = conn.prepare("SELECT id, connection_id, grant_json FROM grants WHERE connection_id = ?1")?;
    let rows = stmt
        .query_map(params![connection.0], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows.iter().filter_map(|(id, c, json)| parse_row(id, c, json)).collect())
}

/// The grants of `connection` that can cover a request about `account`.
fn connection_grants(
    conn: &Connection,
    connection: &ConnectionId,
    account: Option<&str>,
) -> Result<Vec<Grant>, CoreError> {
    let mut grants = all_connection_grants(conn, connection)?;
    grants.retain(|g| g.covers_account(account));
    Ok(grants)
}

fn load_grant(conn: &Connection, id: &GrantId) -> Result<Option<Grant>, CoreError> {
    let row: Option<(String, String, String)> = conn
        .query_row("SELECT id, connection_id, grant_json FROM grants WHERE id = ?1", params![id.0], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .optional()?;
    Ok(row.and_then(|(id, c, json)| parse_row(&id, &c, &json)))
}

fn touch_used(conn: &Connection, id: &GrantId, now: i64) -> Result<(), CoreError> {
    conn.execute("UPDATE grants SET last_used_at = ?2 WHERE id = ?1", params![id.0, now])?;
    Ok(())
}

fn save_grant(conn: &Connection, grant: &Grant) -> Result<(), CoreError> {
    let json = serde_json::to_string(grant).map_err(|e| CoreError::storage(e.to_string()))?;
    conn.execute("UPDATE grants SET grant_json = ?2 WHERE id = ?1", params![grant.id.0, json])?;
    Ok(())
}

impl Store {
    pub fn insert_grant(&self, grant: &Grant, connection_label: &str) -> Result<(), CoreError> {
        self.insert_grant_from(grant, connection_label, "approval")
    }

    pub fn insert_grant_from(&self, grant: &Grant, connection_label: &str, origin: &str) -> Result<(), CoreError> {
        let json = serde_json::to_string(grant).map_err(|e| CoreError::storage(e.to_string()))?;
        self.lock().execute(
            "INSERT INTO grants (id, connection_id, connection_label, created_at, grant_json, origin) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![grant.id.0, grant.connection_id.0, connection_label, grant.created_at, json, origin],
        )?;
        Ok(())
    }

    /// Every grant (active, expired, used up or revoked), newest first. Corrupt rows are skipped.
    pub fn grants(&self) -> Result<Vec<StoredGrant>, CoreError> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT id, connection_id, connection_label, grant_json, last_used_at, origin FROM grants \
             ORDER BY created_at DESC, id",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, Option<i64>>(4)?,
                    r.get::<_, String>(5)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows
            .into_iter()
            .filter_map(|(id, c, label, json, last_used_at, origin)| {
                parse_row(&id, &c, &json).map(|grant| StoredGrant {
                    grant,
                    connection_label: label,
                    last_used_at,
                    origin,
                })
            })
            .collect())
    }

    /// Marks a grant revoked. Returns false when no such grant exists.
    pub fn revoke_grant(&self, id: &GrantId) -> Result<bool, CoreError> {
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let Some(mut grant) = load_grant(&tx, id)? else {
            return Ok(false);
        };
        grant.revoked = true;
        save_grant(&tx, &grant)?;
        tx.commit()?;
        Ok(true)
    }

    /// Removes a grant that is no longer active for good (it cannot be resumed afterwards). Returns `Ok(false)` when
    /// there is no such grant; a grant that still works has to be revoked first.
    pub fn delete_ended_grant(&self, id: &GrantId, now: i64) -> Result<bool, CoreError> {
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let Some(grant) = load_grant(&tx, id)? else {
            return Ok(false);
        };
        if grant.is_active(now) {
            return Err(CoreError::invalid("that grant is still active; revoke it first"));
        }
        tx.execute("DELETE FROM grants WHERE id = ?1", params![id.0])?;
        tx.commit()?;
        Ok(true)
    }

    /// Deletes every grant of a connection (after the connection itself was revoked): with nothing left to
    /// give them to, there is nothing to resume either.
    pub fn remove_connection_grants(&self, connection: &ConnectionId) -> Result<(), CoreError> {
        self.lock().execute("DELETE FROM grants WHERE connection_id = ?1", params![connection.0])?;
        Ok(())
    }

    /// Starts a grant that is no longer active over: it lasts `duration_secs` from `now` with all its uses back.
    /// `Ok(false)` when there is no such grant.
    pub fn resume_grant(&self, id: &GrantId, duration_secs: i64, now: i64) -> Result<bool, CoreError> {
        self.resume_grant_with(id, now, |old| {
            Grant::new(
                old.id.clone(),
                old.connection_id.clone(),
                old.scope.clone(),
                now,
                Some(now.saturating_add(duration_secs)),
                old.max_uses,
            )
            .map_err(|e| CoreError::invalid(e.to_string()))
        })
    }

    /// Starts an ended grant over as whatever `make` builds from it (same id, connection and account; `make` decides
    /// the scope, the end and the number of uses). `Ok(false)` when there is no such grant.
    pub fn resume_grant_with(
        &self,
        id: &GrantId,
        now: i64,
        make: impl FnOnce(&Grant) -> Result<Grant, CoreError>,
    ) -> Result<bool, CoreError> {
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let Some(old) = load_grant(&tx, id)? else {
            return Ok(false);
        };
        if old.is_active(now) {
            return Err(CoreError::invalid("that grant is still active"));
        }
        let mut fresh = make(&old)?;
        fresh.id = old.id.clone();
        fresh.connection_id = old.connection_id.clone();
        fresh.created_at = now;
        fresh.account = old.account;
        save_grant(&tx, &fresh)?;
        tx.execute("UPDATE grants SET created_at = ?2 WHERE id = ?1", params![id.0, now])?;
        tx.commit()?;
        Ok(true)
    }

    /// Whether an active grant of `connection` for `account` needs message bodies to decide.
    pub fn needs_body(&self, connection: &ConnectionId, account: Option<&str>, now: i64) -> Result<bool, CoreError> {
        let conn = self.lock();
        Ok(needs_body(&connection_grants(&conn, connection, account)?, connection, now))
    }

    /// Evaluates a search/read. When every message is covered, one use of each
    /// grant relied on is reserved in the same transaction.
    pub fn evaluate_read_and_reserve(
        &self,
        connection: &ConnectionId,
        account: Option<&str>,
        facts: &[MessageFacts],
        now: i64,
    ) -> Result<ReadDecision, CoreError> {
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut grants = connection_grants(&tx, connection, account)?;
        let decision = evaluate_read(&grants, connection, facts, now);
        if decision.fully_allowed() {
            let used = decision.grants_used();
            record_uses(&mut grants, &used);
            for grant in grants.iter().filter(|g| used.contains(&g.id)) {
                save_grant(&tx, grant)?;
                touch_used(&tx, &grant.id, now)?;
            }
        }
        tx.commit()?;
        Ok(decision)
    }

    /// Which grants let `connection`, acting as `account`, do `access` to each of `resources` of `service`. When
    /// `reserve` is set and every resource is covered, one use of each grant relied on is spent in the same
    /// transaction (nothing is spent for a partial cover, which the user has to decide on anyway).
    #[allow(clippy::too_many_arguments, reason = "a lookup is described by this many independent facts")]
    pub fn service_reserve(
        &self,
        connection: &ConnectionId,
        account: Option<&str>,
        service: &str,
        access: &str,
        class: &str,
        resources: &[String],
        reserve: bool,
        now: i64,
    ) -> Result<std::collections::BTreeMap<String, GrantId>, CoreError> {
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut grants = all_connection_grants(&tx, connection)?;
        let mut covered = std::collections::BTreeMap::new();
        for resource in resources {
            if let Some(id) =
                rewarden_policy::service_allows(&grants, connection, account, service, access, class, resource, now)
            {
                covered.insert(resource.clone(), id);
            }
        }
        if reserve && !resources.is_empty() && covered.len() == resources.iter().collect::<BTreeSet<_>>().len() {
            let used: BTreeSet<GrantId> = covered.values().cloned().collect();
            record_uses(&mut grants, &used);
            for grant in grants.iter().filter(|g| used.contains(&g.id)) {
                save_grant(&tx, grant)?;
                touch_used(&tx, &grant.id, now)?;
            }
        }
        tx.commit()?;
        Ok(covered)
    }

    /// Which accounts of `service` `connection` may be shown without asking.
    pub fn account_coverage(
        &self,
        connection: &ConnectionId,
        service: &str,
        now: i64,
    ) -> Result<rewarden_policy::AccountCoverage, CoreError> {
        let conn = self.lock();
        let grants = all_connection_grants(&conn, connection)?;
        Ok(rewarden_policy::account_coverage(&grants, connection, service, now))
    }

    /// Evaluates a send; when allowed, one use of the grant is reserved.
    pub fn evaluate_send_and_reserve(
        &self,
        connection: &ConnectionId,
        account: Option<&str>,
        email: &OutgoingEmail,
        now: i64,
    ) -> Result<SendDecision, CoreError> {
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut grants = connection_grants(&tx, connection, account)?;
        let decision = evaluate_send(&grants, connection, email, now);
        if let SendDecision::Allowed(id) = &decision {
            record_uses(&mut grants, &BTreeSet::from([id.clone()]));
            if let Some(grant) = grants.iter().find(|g| &g.id == id) {
                save_grant(&tx, grant)?;
                touch_used(&tx, &grant.id, now)?;
            }
        }
        tx.commit()?;
        Ok(decision)
    }

    /// Gives back uses reserved for a call that then failed to execute.
    pub fn refund(&self, ids: &BTreeSet<GrantId>) -> Result<(), CoreError> {
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        for id in ids {
            if let Some(mut grant) = load_grant(&tx, id)? {
                grant.uses = grant.uses.saturating_sub(1);
                save_grant(&tx, &grant)?;
            }
        }
        tx.commit().map_err(CoreError::from)
    }

    /// Consumes one use of each listed grant that is still active (used when
    /// the user approves a request that some grants partially covered).
    pub fn consume_active(&self, ids: &BTreeSet<GrantId>, now: i64) -> Result<(), CoreError> {
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        for id in ids {
            if let Some(mut grant) = load_grant(&tx, id)?
                && grant.is_active(now)
            {
                grant.record_use();
                save_grant(&tx, &grant)?;
                touch_used(&tx, &grant.id, now)?;
            }
        }
        tx.commit().map_err(CoreError::from)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use rewarden_policy::{AddrRule, ReadScope, Scope, SendScope};

    use super::*;
    use crate::store::tests::open;

    pub(crate) fn read_grant(id: &str, conn: &str, max_uses: Option<u32>) -> Grant {
        let scope = ReadScope {
            from: vec![AddrRule::domain("bank.com").unwrap()],
            ..ReadScope::default()
        };
        Grant::new(id.into(), conn.into(), Scope::Read(scope), 10, None, max_uses).unwrap()
    }

    fn send_grant(id: &str, conn: &str, max_uses: Option<u32>) -> Grant {
        let scope = SendScope {
            recipients: vec![AddrRule::domain("work.com").unwrap()],
            subject: None,
            body: None,
        };
        Grant::new(id.into(), conn.into(), Scope::Send(scope), 10, None, max_uses).unwrap()
    }

    fn fact(id: &str, from: &str) -> MessageFacts {
        MessageFacts {
            id: id.to_owned(),
            from: from.to_owned(),
            to: vec![],
            subject: "s".to_owned(),
            body: None,
            labels: vec![],
            date: 20,
        }
    }

    fn email(to: &str) -> OutgoingEmail {
        OutgoingEmail {
            to: vec![to.to_owned()],
            cc: vec![],
            subject: "s".to_owned(),
            body: "b".to_owned(),
            reply_to_message_id: None,
        }
    }

    fn uses(store: &Store, id: &str) -> u32 {
        store.grants().unwrap().into_iter().find(|g| g.grant.id.0 == id).unwrap().grant.uses
    }

    #[test]
    fn insert_list_revoke() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());
        store.insert_grant(&read_grant("g1", "c1", None), "ChatGPT").unwrap();
        store.insert_grant(&send_grant("g2", "c1", Some(2)), "ChatGPT").unwrap();
        assert_eq!(store.grants().unwrap().len(), 2);
        assert!(store.revoke_grant(&"g1".into()).unwrap());
        assert!(!store.revoke_grant(&"nope".into()).unwrap());
        let all = store.grants().unwrap();
        assert_eq!(all.len(), 2, "a revoked grant stays listed so it can be resumed");
        let revoked: Vec<_> = all.iter().filter(|g| g.grant.revoked).map(|g| g.grant.id.0.as_str()).collect();
        assert_eq!(revoked, ["g1"]);
        assert_eq!(all.iter().find(|g| g.grant.id.0 == "g2").unwrap().connection_label, "ChatGPT");
        let d = store.evaluate_read_and_reserve(&"c1".into(), None, &[fact("m1", "a@bank.com")], 100).unwrap();
        assert!(!d.fully_allowed(), "revoked grant no longer applies");
    }

    #[test]
    fn grants_bound_to_an_account_only_cover_that_account() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());
        store.insert_grant(&read_grant("g1", "c1", None).for_account(Some("a@gmail.com".into())), "X").unwrap();
        let msg = [fact("m1", "x@bank.com")];
        let covers = |account: Option<&str>| {
            store.evaluate_read_and_reserve(&"c1".into(), account, &msg, 100).unwrap().fully_allowed()
        };
        assert!(covers(Some("a@gmail.com")));
        assert!(!covers(Some("b@gmail.com")));
        assert!(!covers(None));
        assert!(store.needs_body(&"c1".into(), Some("a@gmail.com"), 100).is_ok());
    }

    #[test]
    fn a_finished_grant_can_be_resumed_for_a_new_period() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());
        store.insert_grant(&read_grant("g1", "c1", Some(1)), "X").unwrap();
        let msg = [fact("m1", "x@bank.com")];
        assert!(store.evaluate_read_and_reserve(&"c1".into(), None, &msg, 100).unwrap().fully_allowed());
        assert!(!store.evaluate_read_and_reserve(&"c1".into(), None, &msg, 101).unwrap().fully_allowed(), "used up");
        assert!(store.resume_grant(&"g1".into(), 3600, 200).unwrap());
        let g = store.grants().unwrap().remove(0);
        assert_eq!((g.grant.uses, g.grant.created_at, g.grant.expires_at), (0, 200, Some(3800)));
        assert!(store.evaluate_read_and_reserve(&"c1".into(), None, &msg, 201).unwrap().fully_allowed());
        assert!(store.resume_grant(&"g1".into(), 3600, 300).unwrap(), "used up again, so it can be resumed again");
        assert!(store.resume_grant(&"g1".into(), 3600, 301).is_err(), "but not while it is running");
    }

    #[test]
    fn only_ended_grants_can_be_deleted_for_good() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());
        store.insert_grant(&read_grant("g1", "c1", None), "X").unwrap();
        assert!(store.delete_ended_grant(&"g1".into(), 100).is_err(), "still active");
        store.revoke_grant(&"g1".into()).unwrap();
        assert!(store.delete_ended_grant(&"g1".into(), 100).unwrap());
        assert!(store.grants().unwrap().is_empty());
        assert!(!store.delete_ended_grant(&"g1".into(), 100).unwrap());
    }

    #[test]
    fn a_revoked_grant_can_be_resumed_but_an_active_one_cannot() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());
        store.insert_grant(&read_grant("g1", "c1", None), "X").unwrap();
        assert!(store.resume_grant(&"g1".into(), 60, 100).is_err(), "still active");
        store.revoke_grant(&"g1".into()).unwrap();
        assert!(store.resume_grant(&"g1".into(), 60, 100).unwrap());
        assert!(!store.grants().unwrap()[0].grant.revoked);
        assert!(!store.resume_grant(&"nope".into(), 60, 100).unwrap());
    }

    #[test]
    fn corrupt_or_mismatched_rows_never_match() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());
        let good = serde_json::to_string(&read_grant("g1", "c1", None)).unwrap();
        let conn = store.lock();
        conn.execute(
            "INSERT INTO grants (id, connection_id, connection_label, created_at, grant_json) VALUES ('bad', 'c1', 'X', 1, '{\"id\":\"bad\",\"scope\":{\"action\":\"read\"}}')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO grants (id, connection_id, connection_label, created_at, grant_json) VALUES ('g1', 'c9', 'X', 1, ?1)",
            params![good],
        )
        .unwrap();
        drop(conn);
        assert!(store.grants().unwrap().is_empty());
        for c in ["c1", "c9"] {
            let d = store.evaluate_read_and_reserve(&c.into(), None, &[fact("m1", "a@bank.com")], 100).unwrap();
            assert_eq!(d.needs_approval, vec![0]);
        }
    }

    #[test]
    fn reserve_only_when_fully_allowed() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());
        store.insert_grant(&read_grant("g1", "c1", Some(5)), "X").unwrap();
        let partial = [fact("m1", "a@bank.com"), fact("m2", "eve@evil.com")];
        assert!(!store.evaluate_read_and_reserve(&"c1".into(), None, &partial, 100).unwrap().fully_allowed());
        assert_eq!(uses(&store, "g1"), 0);
        assert!(store.evaluate_read_and_reserve(&"c1".into(), None, &partial[..1], 100).unwrap().fully_allowed());
        assert_eq!(uses(&store, "g1"), 1);
        store.refund(&BTreeSet::from(["g1".into()])).unwrap();
        assert_eq!(uses(&store, "g1"), 0);
        store.refund(&BTreeSet::from(["g1".into()])).unwrap();
        assert_eq!(uses(&store, "g1"), 0, "refund never goes below zero");
    }

    #[test]
    fn send_reserve_and_consume_active() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());
        store.insert_grant(&send_grant("g1", "c1", Some(1)), "X").unwrap();
        assert_eq!(
            store.evaluate_send_and_reserve(&"c1".into(), None, &email("a@work.com"), 100).unwrap(),
            SendDecision::Allowed("g1".into())
        );
        assert_eq!(
            store.evaluate_send_and_reserve(&"c1".into(), None, &email("a@work.com"), 100).unwrap(),
            SendDecision::NeedsApproval,
            "the only use is spent"
        );
        store.consume_active(&BTreeSet::from(["g1".into()]), 100).unwrap();
        assert_eq!(uses(&store, "g1"), 1, "inactive grants are not consumed further");
    }

    #[test]
    fn concurrent_requests_spend_a_single_use_once() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(open(dir.path()));
        store.insert_grant(&read_grant("g1", "c1", Some(1)), "X").unwrap();
        let allowed = Arc::new(AtomicUsize::new(0));
        let mut threads = Vec::new();
        for _ in 0..8 {
            let (store, allowed) = (Arc::clone(&store), Arc::clone(&allowed));
            threads.push(std::thread::spawn(move || {
                let d = store.evaluate_read_and_reserve(&"c1".into(), None, &[fact("m1", "a@bank.com")], 100).unwrap();
                if d.fully_allowed() {
                    allowed.fetch_add(1, Ordering::SeqCst);
                }
            }));
        }
        for t in threads {
            t.join().unwrap();
        }
        let allowed = allowed.load(Ordering::SeqCst);
        assert_eq!(allowed, 1);
        assert_eq!(uses(&store, "g1"), 1);
    }

    #[test]
    fn remove_connection_grants_only_touches_that_connection() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());
        store.insert_grant(&read_grant("g1", "c1", None), "A").unwrap();
        store.insert_grant(&read_grant("g2", "c2", None), "B").unwrap();
        store.remove_connection_grants(&"c1".into()).unwrap();
        let left: Vec<_> = store.grants().unwrap().into_iter().map(|g| g.grant.id.0).collect();
        assert_eq!(left, vec!["g2"]);
        assert!(!store.needs_body(&"c1".into(), None, 100).unwrap());
    }
}
