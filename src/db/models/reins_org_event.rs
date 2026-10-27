//! An organization's admin events (O1): append-only and hash-linked per org, so O2's audit log can take them over and
//! check that none was edited, removed or reordered (`reins_proto::org::event_hash`). Nothing here updates or deletes
//! an event; only deleting the organization removes them.

use diesel::prelude::*;
use reins_proto::org::{EventFields, GENESIS_HASH, event_hash};

use crate::db::{DbConnInner, schema::reins_org_events};

#[derive(Clone, Debug, PartialEq, Eq, Identifiable, Queryable, Insertable)]
#[diesel(table_name = reins_org_events)]
#[diesel(primary_key(org_uuid, seq))]
pub struct ReinsOrgEvent {
    pub org_uuid: String,
    pub seq: i64,
    pub at: i64,
    pub kind: String,
    pub actor: Option<String>,
    pub subject: Option<String>,
    pub detail: String,
    pub prev_hash: String,
    pub hash: String,
}

impl ReinsOrgEvent {
    fn fields(&self) -> EventFields<'_> {
        EventFields {
            org: &self.org_uuid,
            seq: self.seq,
            at: self.at,
            kind: &self.kind,
            actor: self.actor.as_deref(),
            subject: self.subject.as_deref(),
            detail: &self.detail,
        }
    }
}

/// Appends one event to `org`'s log and returns its sequence number. Runs in its own (nested) transaction, so the
/// head it reads is the head it extends; a concurrent append on the same org fails on the primary key and is retried
/// by the caller's transaction, never interleaved.
pub fn append(
    c: &mut DbConnInner,
    org: &str,
    kind: &str,
    actor: Option<&str>,
    subject: Option<&str>,
    detail: &serde_json::Value,
    at: i64,
) -> QueryResult<i64> {
    let mut tries = 0;
    loop {
        match append_once(c, org, kind, actor, subject, detail, at) {
            // Another append took this sequence number meanwhile: read the new head and chain after it.
            Err(diesel::result::Error::DatabaseError(diesel::result::DatabaseErrorKind::UniqueViolation, _))
                if tries < 3 =>
            {
                tries += 1;
            }
            other => return other,
        }
    }
}

fn append_once(
    c: &mut DbConnInner,
    org: &str,
    kind: &str,
    actor: Option<&str>,
    subject: Option<&str>,
    detail: &serde_json::Value,
    at: i64,
) -> QueryResult<i64> {
    c.transaction(|c| {
        let head: Option<(i64, String)> = reins_org_events::table
            .filter(reins_org_events::org_uuid.eq(org))
            .order(reins_org_events::seq.desc())
            .select((reins_org_events::seq, reins_org_events::hash))
            .first(c)
            .optional()?;
        let (seq, prev_hash) = head.map_or((1, GENESIS_HASH.to_owned()), |(s, h)| (s + 1, h));
        let mut row = ReinsOrgEvent {
            org_uuid: org.to_owned(),
            seq,
            at,
            kind: kind.to_owned(),
            actor: actor.map(str::to_owned),
            subject: subject.map(str::to_owned),
            detail: detail.to_string(),
            prev_hash,
            hash: String::new(),
        };
        row.hash = event_hash(&row.fields(), &row.prev_hash);
        diesel::insert_into(reins_org_events::table).values(&row).execute(c)?;
        Ok(seq)
    })
}

/// Newest first: up to `limit` events of `org` before `before` (a sequence number), or the newest.
pub fn q_events(c: &mut DbConnInner, org: &str, before: Option<i64>, limit: i64) -> QueryResult<Vec<ReinsOrgEvent>> {
    let mut q = reins_org_events::table.filter(reins_org_events::org_uuid.eq(org)).into_boxed();
    if let Some(before) = before {
        q = q.filter(reins_org_events::seq.lt(before));
    }
    q.order(reins_org_events::seq.desc()).limit(limit).load(c)
}

/// Oldest first: up to `limit` events of `org` after `after` (a sequence number; 0 for the first).
pub fn q_events_from(c: &mut DbConnInner, org: &str, after: i64, limit: i64) -> QueryResult<Vec<ReinsOrgEvent>> {
    reins_org_events::table
        .filter(reins_org_events::org_uuid.eq(org))
        .filter(reins_org_events::seq.gt(after))
        .order(reins_org_events::seq.asc())
        .limit(limit)
        .load(c)
}

/// The chain head of `org`: the last event's sequence number and hash.
pub fn q_head(c: &mut DbConnInner, org: &str) -> QueryResult<Option<(i64, String)>> {
    reins_org_events::table
        .filter(reins_org_events::org_uuid.eq(org))
        .order(reins_org_events::seq.desc())
        .select((reins_org_events::seq, reins_org_events::hash))
        .first(c)
        .optional()
}

impl ReinsOrgEvent {
    /// The event as an audit export carries it.
    #[must_use]
    pub fn to_audit(&self) -> reins_proto::managed::AuditEvent {
        reins_proto::managed::AuditEvent {
            org: self.org_uuid.clone(),
            seq: self.seq,
            at: self.at,
            kind: self.kind.clone(),
            actor: self.actor.clone(),
            subject: self.subject.clone(),
            detail: self.detail.clone(),
            prev_hash: self.prev_hash.clone(),
            hash: self.hash.clone(),
        }
    }
}

/// Whether `events` (one org's, oldest first, from its first event) form an unbroken chain.
#[must_use]
pub fn verify_chain(events: &[ReinsOrgEvent]) -> bool {
    let mut prev = GENESIS_HASH.to_owned();
    for (expected, e) in (1_i64..).zip(events) {
        if e.seq != expected || e.prev_hash != prev || event_hash(&e.fields(), &e.prev_hash) != e.hash {
            return false;
        }
        prev.clone_from(&e.hash);
    }
    true
}

#[cfg(all(test, sqlite))]
mod tests {
    use serde_json::json;

    use super::super::reins_device::test_db;
    use super::*;

    fn oldest_first(c: &mut DbConnInner, org: &str) -> Vec<ReinsOrgEvent> {
        let mut e = q_events(c, org, None, 100).unwrap();
        e.reverse();
        e
    }

    #[test]
    fn events_chain_and_tampering_shows() {
        let mut c = test_db();
        for (i, kind) in (0_i64..).zip(["org.created", "domain.added", "domain.verified"]) {
            let seq = append(&mut c, "o1", kind, Some("u1"), None, &json!({"n": i}), 10 + i).unwrap();
            assert_eq!(seq, i + 1);
        }
        let events = oldest_first(&mut c, "o1");
        assert!(verify_chain(&events));
        assert_eq!(events[1].prev_hash, events[0].hash);
        let mut edited = events.clone();
        edited[1].detail = json!({"n": 9}).to_string();
        assert!(!verify_chain(&edited));
        let mut removed = events.clone();
        removed.remove(1);
        assert!(!verify_chain(&removed));
        let mut swapped = events;
        swapped.swap(1, 2);
        assert!(!verify_chain(&swapped));
    }

    #[test]
    fn sequences_are_per_org_and_pages_go_back() {
        let mut c = test_db();
        for i in 0..5 {
            append(&mut c, "o1", "x", None, None, &json!({}), i).unwrap();
        }
        assert_eq!(append(&mut c, "o2", "x", None, Some("m1"), &json!({}), 1).unwrap(), 1);
        let page: Vec<i64> = q_events(&mut c, "o1", Some(4), 2).unwrap().iter().map(|e| e.seq).collect();
        assert_eq!(page, [3, 2]);
        assert!(verify_chain(&oldest_first(&mut c, "o2")));
    }
}
