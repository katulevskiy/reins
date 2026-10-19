//! Grant rows and the atomic evaluate-and-reserve operations.
//!
//! Every method that evaluates grants holds the store mutex and runs inside
//! one `BEGIN IMMEDIATE` transaction, so two concurrent requests can never both
//! spend the last use of a grant.

use std::collections::BTreeSet;

use reins_policy::{
    Grant, MessageFacts, ReadDecision, SendDecision, evaluate_read, evaluate_send, needs_body, record_uses,
};
use reins_proto::gmail::OutgoingEmail;
use reins_proto::ids::{ConnectionId, GrantId};
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

/// The grants of `connection` that may be used now. Every evaluation reads them here, so this is where a grant a
/// bypass gave stops counting the moment its bypass is no longer in force (read in the caller's transaction), whether
/// or not taking it back has happened yet: it fails closed.
fn all_connection_grants(conn: &Connection, connection: &ConnectionId, now: i64) -> Result<Vec<Grant>, CoreError> {
    let mut stmt = conn.prepare("SELECT id, connection_id, grant_json, origin FROM grants WHERE connection_id = ?1")?;
    let rows = stmt
        .query_map(params![connection.0], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut grants = Vec::with_capacity(rows.len());
    for (id, c, json, origin) in &rows {
        if origin == super::permissions::BUDGET_ORIGIN {
            continue;
        }
        if !crate::autopilot::store::origin_in_force(conn, origin, now)? {
            continue;
        }
        grants.extend(parse_row(id, c, json));
    }
    Ok(grants)
}

/// The grants of `connection` that can cover a request about `account`.
fn connection_grants(
    conn: &Connection,
    connection: &ConnectionId,
    account: Option<&str>,
    now: i64,
) -> Result<Vec<Grant>, CoreError> {
    let mut grants = all_connection_grants(conn, connection, now)?;
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

    /// Stores a grant. One a bypass gives is stored only while that bypass is still in force, checked in the same
    /// transaction: a request decided during a bypass that finishes after it was stopped gives nothing standing.
    pub fn insert_grant_from(&self, grant: &Grant, connection_label: &str, origin: &str) -> Result<(), CoreError> {
        if super::permissions::is_budget(grant) || origin == super::permissions::BUDGET_ORIGIN {
            return Err(CoreError::invalid("authorization counters cannot be created through grant controls"));
        }
        let json = serde_json::to_string(grant).map_err(|e| CoreError::storage(e.to_string()))?;
        let mut conn = self.lock()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if !crate::autopilot::store::origin_in_force(&tx, origin, crate::store::unix_now())? {
            log::warn!("a permission from a bypass that has ended was not kept");
            return Err(CoreError::invalid("the bypass ended before this permission was given; it waits for you"));
        }
        tx.execute(
            "INSERT INTO grants (id, connection_id, connection_label, created_at, grant_json, origin) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![grant.id.0, grant.connection_id.0, connection_label, grant.created_at, json, origin],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Every grant (active, expired, used up or revoked), newest first. Corrupt rows are skipped.
    pub fn grants(&self) -> Result<Vec<StoredGrant>, CoreError> {
        let conn = self.read()?;
        let mut stmt = conn.prepare(
            "SELECT id, connection_id, connection_label, grant_json, last_used_at, origin FROM grants \
             WHERE origin != 'policy_budget' ORDER BY created_at DESC, id",
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
        super::permissions::reject_budget_id(id)?;
        let mut conn = self.lock()?;
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
        super::permissions::reject_budget_id(id)?;
        let mut conn = self.lock()?;
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
        if connection.0 == "__reins_policy_budget__" {
            return Err(CoreError::invalid("authorization counters cannot be removed through grant controls"));
        }
        self.lock()?.execute("DELETE FROM grants WHERE connection_id = ?1", params![connection.0])?;
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
        super::permissions::reject_budget_id(id)?;
        let mut conn = self.lock()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let Some(old) = load_grant(&tx, id)? else {
            return Ok(false);
        };
        if old.is_active(now) {
            return Err(CoreError::invalid("that grant is still active"));
        }
        let mut fresh = make(&old)?;
        if super::permissions::is_budget(&fresh) {
            return Err(CoreError::invalid("authorization counters cannot be created through grant controls"));
        }
        fresh.id = old.id.clone();
        fresh.connection_id = old.connection_id.clone();
        fresh.created_at = now;
        fresh.account = old.account;
        save_grant(&tx, &fresh)?;
        tx.execute("UPDATE grants SET created_at = ?2 WHERE id = ?1", params![id.0, now])?;
        tx.commit()?;
        Ok(true)
    }

    /// Lets an active grant run for `duration_secs` from `now` instead of until its end; its uses so far stay. The
    /// same limits as a new grant apply (a grant for everything ends within its maximum). `Ok(false)` when there is no
    /// such grant.
    pub fn extend_grant(&self, id: &GrantId, duration_secs: i64, now: i64) -> Result<bool, CoreError> {
        super::permissions::reject_budget_id(id)?;
        let mut conn = self.lock()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let Some(old) = load_grant(&tx, id)? else {
            return Ok(false);
        };
        if !old.is_active(now) {
            return Err(CoreError::invalid("that grant has ended; resume it instead"));
        }
        let mut fresh = Grant::new(
            old.id.clone(),
            old.connection_id.clone(),
            old.scope.clone(),
            now,
            Some(now.saturating_add(duration_secs)),
            old.max_uses,
        )
        .map_err(|e| CoreError::invalid(e.to_string()))?;
        fresh.uses = old.uses;
        fresh.account = old.account;
        save_grant(&tx, &fresh)?;
        tx.commit()?;
        Ok(true)
    }

    /// Whether an active grant of `connection` for `account` needs message bodies to decide.
    pub fn needs_body(&self, connection: &ConnectionId, account: Option<&str>, now: i64) -> Result<bool, CoreError> {
        let conn = self.read()?;
        Ok(needs_body(&connection_grants(&conn, connection, account, now)?, connection, now))
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
        self.evaluate_read_holding(connection, account, facts, &[], now)
    }

    /// Like [`Store::evaluate_read_and_reserve`], with the messages `held[i]` marks (a login code, a password) never
    /// covered by any grant: they always wait for the user.
    pub fn evaluate_read_holding(
        &self,
        connection: &ConnectionId,
        account: Option<&str>,
        facts: &[MessageFacts],
        held: &[bool],
        now: i64,
    ) -> Result<ReadDecision, CoreError> {
        self.evaluate_holding(connection, account, facts, held, false, now).map(|(decision, _)| decision)
    }

    /// Evaluates what a Gmail search found: like [`Store::evaluate_read_holding`], and a message no read grant covers
    /// may still be covered by a grant for Gmail's headlines. Returns the decision and the messages released as
    /// headlines only, which the caller must strip to their metadata. Never use this for reading messages.
    pub fn evaluate_search_holding(
        &self,
        connection: &ConnectionId,
        account: Option<&str>,
        facts: &[MessageFacts],
        held: &[bool],
        now: i64,
    ) -> Result<(ReadDecision, BTreeSet<usize>), CoreError> {
        self.evaluate_holding(connection, account, facts, held, true, now)
    }

    fn evaluate_holding(
        &self,
        connection: &ConnectionId,
        account: Option<&str>,
        facts: &[MessageFacts],
        held: &[bool],
        headlines: bool,
        now: i64,
    ) -> Result<(ReadDecision, BTreeSet<usize>), CoreError> {
        let mut conn = self.lock()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut grants = connection_grants(&tx, connection, account, now)?;
        let mut decision = evaluate_read(&grants, connection, facts, now);
        let is_held = |i: usize| held.get(i).copied().unwrap_or(false);
        let (kept, withheld): (Vec<_>, Vec<_>) = decision.allowed.into_iter().partition(|(i, _)| !is_held(*i));
        decision.allowed = kept;
        decision.needs_approval.extend(withheld.into_iter().map(|(i, _)| i));
        decision.needs_approval.sort_unstable();
        // What no read grant covers may be seen as a headline, never what is held.
        let mut headlines_only = BTreeSet::new();
        if headlines
            && let Some(grant) =
                reins_policy::headlines_allows(&grants, connection, account, crate::views::SERVICE_GMAIL, now)
        {
            let (seen, waiting): (Vec<usize>, Vec<usize>) = decision.needs_approval.iter().partition(|i| !is_held(**i));
            decision.needs_approval = waiting;
            for i in seen {
                decision.allowed.push((i, grant.clone()));
                headlines_only.insert(i);
            }
            decision.allowed.sort_unstable();
            if !decision.fully_allowed() {
                // A search that waits for the user anyway: a headline never counts as covered there, since what is
                // covered is released in full once the user approves. Each of them waits for the user's tick.
                decision.allowed.retain(|(i, _)| !headlines_only.contains(i));
                decision.needs_approval.extend(headlines_only.iter().copied());
                decision.needs_approval.sort_unstable();
                headlines_only.clear();
            }
        }
        if decision.fully_allowed() {
            let used = decision.grants_used();
            record_uses(&mut grants, &used);
            for grant in grants.iter().filter(|g| used.contains(&g.id)) {
                save_grant(&tx, grant)?;
                touch_used(&tx, &grant.id, now)?;
            }
        }
        tx.commit()?;
        Ok((decision, headlines_only))
    }

    /// The grant that lets `connection`, acting as `account`, see the headlines of what a list of `service` finds; one
    /// use of it is spent in the same transaction. `None` when there is none.
    pub fn headlines_reserve(
        &self,
        connection: &ConnectionId,
        account: Option<&str>,
        service: &str,
        now: i64,
    ) -> Result<Option<GrantId>, CoreError> {
        let mut conn = self.lock()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut grants = all_connection_grants(&tx, connection, now)?;
        let found = reins_policy::headlines_allows(&grants, connection, account, service, now);
        if let Some(id) = &found {
            record_uses(&mut grants, &BTreeSet::from([id.clone()]));
            if let Some(grant) = grants.iter().find(|g| &g.id == id) {
                save_grant(&tx, grant)?;
                touch_used(&tx, &grant.id, now)?;
            }
        }
        tx.commit()?;
        Ok(found)
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
        op: &str,
        resources: &[String],
        reserve: bool,
        now: i64,
    ) -> Result<std::collections::BTreeMap<String, GrantId>, CoreError> {
        let mut conn = self.lock()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut grants = all_connection_grants(&tx, connection, now)?;
        let mut covered = std::collections::BTreeMap::new();
        for resource in resources {
            if let Some(id) =
                reins_policy::service_allows(&grants, connection, account, service, access, class, op, resource, now)
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

    /// The grant that lets `connection`, acting as `account` (the vault), have the secrets `facts` describes released
    /// without asking, and the command scope it names; one use of it is spent in the same transaction.
    pub fn secrets_reserve(
        &self,
        connection: &ConnectionId,
        account: Option<&str>,
        facts: &reins_policy::SecretsFacts<'_>,
        now: i64,
    ) -> Result<Option<(GrantId, reins_proto::desktop::CommandScope, Option<i64>)>, CoreError> {
        let mut conn = self.lock()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut grants = all_connection_grants(&tx, connection, now)?;
        let found = reins_policy::secrets_allows(&grants, connection, account, facts, now);
        let mut out = None;
        if let Some(id) = &found {
            record_uses(&mut grants, &BTreeSet::from([id.clone()]));
            if let Some(grant) = grants.iter().find(|g| &g.id == id) {
                save_grant(&tx, grant)?;
                touch_used(&tx, &grant.id, now)?;
                if let reins_policy::Scope::Secrets(s) = &grant.scope {
                    out = Some((id.clone(), s.command.clone(), grant.expires_at));
                }
            }
        }
        tx.commit()?;
        Ok(out)
    }

    /// Which accounts of `service` `connection` may be shown without asking.
    pub fn account_coverage(
        &self,
        connection: &ConnectionId,
        service: &str,
        now: i64,
    ) -> Result<reins_policy::AccountCoverage, CoreError> {
        let conn = self.read()?;
        let grants = all_connection_grants(&conn, connection, now)?;
        Ok(reins_policy::account_coverage(&grants, connection, service, now))
    }

    /// Evaluates a send; when allowed, one use of the grant is reserved.
    pub fn evaluate_send_and_reserve(
        &self,
        connection: &ConnectionId,
        account: Option<&str>,
        email: &OutgoingEmail,
        now: i64,
    ) -> Result<SendDecision, CoreError> {
        let mut conn = self.lock()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut grants = connection_grants(&tx, connection, account, now)?;
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
        for id in ids {
            super::permissions::reject_budget_id(id)?;
        }
        let mut conn = self.lock()?;
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
        for id in ids {
            super::permissions::reject_budget_id(id)?;
        }
        let mut conn = self.lock()?;
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

    use reins_policy::{AddrRule, ReadScope, Scope, SendScope};

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

    /// Every evaluation reads grants through one place: a grant a bypass gave counts there only while that bypass
    /// runs (the global one, or the connection's own), however it ended, even if it was never taken back.
    #[test]
    fn a_bypass_grant_counts_only_while_its_bypass_runs() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());
        let set = |scope: &str, mode: Option<&str>, until: Option<i64>| {
            store
                .lock()
                .unwrap()
                .execute(
                    "INSERT INTO autopilot_settings (scope, mode, bypass_until, profile_id, updated_at) \
                     VALUES (?1, ?2, ?3, NULL, 0) ON CONFLICT (scope) DO UPDATE SET mode = ?2, bypass_until = ?3",
                    params![scope, mode, until],
                )
                .unwrap();
        };
        let read = |c: &str| store.evaluate_read_and_reserve(&c.into(), None, &[fact("m", "a@bank.com")], 100).unwrap();
        // Refused while no bypass of its scope runs.
        assert!(store.insert_grant_from(&read_grant("g1", "c1", None), "Claude", "bypass").is_err());
        set("", None, Some(i64::MAX));
        set("c2", None, Some(i64::MAX));
        store.insert_grant_from(&read_grant("g1", "c1", None), "Claude", "bypass").unwrap();
        store.insert_grant_from(&read_grant("g2", "c2", None), "Cursor", "bypass:c2").unwrap();
        store.insert_grant_from(&read_grant("g3", "c3", None), "Mine", "user").unwrap();
        assert!(read("c1").fully_allowed() && read("c2").fully_allowed() && read("c3").fully_allowed());
        // c2's own bypass ends: only its grant stops counting.
        set("c2", None, Some(50));
        assert!(read("c1").fully_allowed() && !read("c2").fully_allowed());
        assert!(store.account_coverage(&"c2".into(), "gmail", 100).is_ok());
        // A global lockdown stops every bypass's grant; the user's own still counts. None was revoked.
        set("", Some("lockdown"), None);
        assert!(!read("c1").fully_allowed() && read("c3").fully_allowed());
        assert!(store.grants().unwrap().iter().all(|g| !g.grant.revoked));
        assert_eq!(store.headlines_reserve(&"c1".into(), None, "gmail", 100).unwrap(), None);
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
        assert_eq!(store.grants().unwrap().len(), 0);
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
        let conn = store.lock().unwrap();
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
        assert_eq!(store.grants().unwrap().len(), 0);
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

    #[test]
    fn an_active_grant_is_extended_with_its_uses_and_an_ended_one_is_not() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());
        let mut capped = read_grant("g1", "c1", Some(3));
        capped.expires_at = Some(100);
        store.insert_grant(&capped, "ChatGPT").unwrap();
        assert!(
            store
                .evaluate_read_and_reserve(&"c1".into(), None, &[fact("m1", "a@bank.com")], 50)
                .unwrap()
                .fully_allowed()
        );
        assert!(store.extend_grant(&"g1".into(), 3_600, 90).unwrap());
        let g = store.grants().unwrap().into_iter().find(|g| g.grant.id.0 == "g1").unwrap().grant;
        assert_eq!((g.expires_at, g.uses, g.max_uses), (Some(3_690), 1, Some(3)), "a new end, the same uses");
        assert!(store.extend_grant(&"g1".into(), 3_600, 5_000).is_err(), "ended: resumed, not extended");
        assert!(!store.extend_grant(&"nope".into(), 3_600, 90).unwrap());
    }
}
