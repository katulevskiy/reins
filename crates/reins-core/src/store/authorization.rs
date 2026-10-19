//! Versioned account authorization state. It uses an existing encrypted, signed-primary Meta record so snapshots
//! and the op log carry exactly the same policy set. Concurrent changes are held for explicit owner resolution;
//! neither clocks nor JSON's ordinary entry-by-entry merge may manufacture authority.

use std::collections::BTreeSet;

use reins_proto::authorization::{PolicyLifecycle, PolicyRecord};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::Store;
use crate::CoreError;

pub(crate) const RECORD_KEY: &str = "authorization.state";
pub(crate) const STORE_KEY: &str = "app.authorization.state";
// Leave room for the signed op envelope and {prev,row}; the op body limit is 64 KiB.
pub const MAX_STATE_BYTES: usize = 48 * 1024;
pub const MAX_POLICIES: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorizationState {
    pub v: u32,
    pub owner: String,
    pub revision: u64,
    /// Never automatically resolved. The authenticated owner must inspect and replace the complete policy set.
    pub conflicted: bool,
    pub policies: Vec<PolicyRecord>,
    /// Concurrent versions are kept for inspection; resolving a conflict is an explicit owner operation.
    #[serde(default)]
    pub conflicting_policies: Vec<PolicyRecord>,
    /// Policy identities are never reused, even after a restore or a later deliberate widening.
    pub revoked_ids: BTreeSet<String>,
}

impl AuthorizationState {
    pub fn empty(owner: &str) -> Self {
        Self {
            v: 1,
            owner: owner.to_owned(),
            revision: 0,
            conflicted: false,
            policies: Vec::new(),
            conflicting_policies: Vec::new(),
            revoked_ids: BTreeSet::new(),
        }
    }

    pub fn validate(&self) -> Result<(), CoreError> {
        if self.v != 1
            || self.owner.is_empty()
            || self.owner.len() > 256
            || self.policies.len() > MAX_POLICIES
            || self.conflicting_policies.len() > MAX_POLICIES
            || self.revoked_ids.len() > MAX_POLICIES
            || self.revoked_ids.iter().any(|id| id.is_empty() || id.len() > 256)
        {
            return Err(CoreError::invalid("unsupported or oversized authorization state"));
        }
        let mut ids = BTreeSet::new();
        for policy in &self.policies {
            policy.validate().map_err(|e| CoreError::invalid(e.to_string()))?;
            if policy.issuer.principal_id != self.owner || !ids.insert(&policy.id) {
                return Err(CoreError::invalid("authorization policy has another owner or a duplicate identity"));
            }
            if self.revoked_ids.contains(&policy.id) && policy.lifecycle != PolicyLifecycle::Revoked {
                return Err(CoreError::invalid("a revoked policy identity cannot be reused"));
            }
            if policy.lifecycle == PolicyLifecycle::Revoked && !self.revoked_ids.contains(&policy.id) {
                return Err(CoreError::invalid("a revoked policy requires a permanent tombstone"));
            }
        }
        for policy in &self.conflicting_policies {
            policy.validate().map_err(|e| CoreError::invalid(e.to_string()))?;
            if policy.issuer.principal_id != self.owner {
                return Err(CoreError::invalid("a conflicting policy belongs to another owner"));
            }
        }
        if !self.conflicted && !self.conflicting_policies.is_empty() {
            return Err(CoreError::invalid("unresolved policy versions remain"));
        }
        if serde_json::to_vec(self).map_err(|e| CoreError::storage(e.to_string()))?.len() > MAX_STATE_BYTES {
            return Err(CoreError::invalid("authorization state is too large"));
        }
        Ok(())
    }
}

pub(super) fn read(conn: &Connection) -> Result<Option<AuthorizationState>, CoreError> {
    let text: Option<String> =
        conn.query_row("SELECT value FROM meta WHERE key = ?1", [STORE_KEY], |r| r.get(0)).optional()?;
    text.map(|text| {
        if text.len() > MAX_STATE_BYTES {
            return Err(CoreError::storage("authorization state is too large"));
        }
        let state: AuthorizationState =
            serde_json::from_str(&text).map_err(|_| CoreError::storage("authorization state cannot be verified"))?;
        state.validate()?;
        Ok(state)
    })
    .transpose()
}

impl Store {
    /// Absence is the legacy personal-account behavior. Corrupt/unknown state is an error, never absence.
    pub fn authorization_state(&self) -> Result<Option<AuthorizationState>, CoreError> {
        let conn = self.read()?;
        read(&conn)
    }

    /// An explicit owner's compare-and-swap. The Engine authenticates the owner and verifies the primary role first.
    pub(crate) fn authorization_replace(&self, expected: u64, next: &AuthorizationState) -> Result<(), CoreError> {
        next.validate()?;
        let conn = self.lock()?;
        let previous = read(&conn)?;
        let revision = previous.as_ref().map_or(0, |s| s.revision);
        if revision != expected
            || next.revision
                != expected.checked_add(1).ok_or_else(|| CoreError::invalid("policy revision exhausted"))?
        {
            return Err(CoreError::invalid("authorization changed; reload before saving"));
        }
        if let Some(previous) = &previous {
            if previous.owner != next.owner || !previous.revoked_ids.is_subset(&next.revoked_ids) {
                return Err(CoreError::invalid("authorization owner and revocations are immutable"));
            }
            for old in &previous.policies {
                let new = next
                    .policies
                    .iter()
                    .find(|p| p.id == old.id)
                    .ok_or_else(|| CoreError::invalid("revoke a policy instead of deleting it"))?;
                if old != new
                    && (new.revision
                        != old
                            .revision
                            .checked_add(1)
                            .ok_or_else(|| CoreError::invalid("policy revision exhausted"))?
                        || new.issuer != old.issuer
                        || new.kind != old.kind)
                {
                    return Err(CoreError::invalid("invalid authorization revision or changed policy identity"));
                }
            }
        }
        for policy in &next.policies {
            if previous.as_ref().is_none_or(|old| !old.policies.iter().any(|p| p.id == policy.id))
                && policy.revision != 1
            {
                return Err(CoreError::invalid("a new policy starts at revision one"));
            }
        }
        let text = serde_json::to_string(next).map_err(|e| CoreError::storage(e.to_string()))?;
        conn.execute(
            "INSERT INTO meta (key,value) VALUES (?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![STORE_KEY, text],
        )?;
        // Cached model suggestions do not carry authority across a policy revision. Human learning stays intact.
        conn.execute("DELETE FROM autopilot_suggestions", [])?;
        Ok(())
    }
}

/// Merge two authenticated but concurrent state revisions. Retain all restrictions for owner inspection and hold
/// the whole account. Invalid input also becomes an explicit hold marker, never a missing policy record.
pub(crate) fn conflict(a: &Value, b: &Value) -> Value {
    let (mut a, b) = match (
        serde_json::from_value::<AuthorizationState>(a.clone()),
        serde_json::from_value::<AuthorizationState>(b.clone()),
    ) {
        (Ok(a), Ok(b)) => (a, b),
        (Ok(_), Err(_)) => return unverified(a),
        (Err(_), Ok(_)) => return unverified(b),
        (Err(_), Err(_)) => return serde_json::json!({"v":1,"conflicted":true}),
    };
    a.conflicted = true;
    a.revision = a.revision.max(b.revision);
    a.revoked_ids.extend(b.revoked_ids);
    a.conflicting_policies.extend(b.conflicting_policies);
    for policy in b.policies {
        if !a.policies.iter().any(|p| p.id == policy.id) {
            a.policies.push(policy);
        } else if let Some(existing) = a.policies.iter_mut().find(|p| p.id == policy.id) {
            if policy.lifecycle == PolicyLifecycle::Revoked {
                a.revoked_ids.insert(policy.id.clone());
            }
            if *existing != policy && !a.conflicting_policies.contains(&policy) {
                if policy.revision > existing.revision {
                    a.conflicting_policies.push(existing.clone());
                    *existing = policy;
                } else {
                    a.conflicting_policies.push(policy);
                }
            }
        }
    }
    for policy in &mut a.policies {
        if a.revoked_ids.contains(&policy.id) {
            policy.lifecycle = PolicyLifecycle::Revoked;
        }
    }
    serde_json::to_value(a).unwrap_or_else(|_| serde_json::json!({"v":1,"conflicted":true}))
}

pub(crate) fn unverified(value: &Value) -> Value {
    let mut value = value.clone();
    if let Some(map) = value.as_object_mut() {
        map.insert("conflicted".to_owned(), Value::Bool(true));
    } else {
        value = serde_json::json!({"v":1,"conflicted":true});
    }
    value
}

/// Missing or rolled-back policy state is retained as a hold, never interpreted as legacy absence.
pub(crate) fn incoming(previous: Option<&Value>, next: Option<&Value>) -> Value {
    let Some(next) = next else {
        return unverified(previous.unwrap_or(&Value::Null));
    };
    let Ok(next_state) = serde_json::from_value::<AuthorizationState>(next.clone()) else {
        return unverified(previous.unwrap_or(next));
    };
    if next_state.validate().is_err() {
        return unverified(previous.unwrap_or(next));
    }
    let Some(previous) = previous else {
        return next.clone();
    };
    let Ok(previous_state) = serde_json::from_value::<AuthorizationState>(previous.clone()) else {
        return unverified(previous);
    };
    if previous_state.validate().is_err() {
        return unverified(previous);
    }
    if previous_state.owner != next_state.owner
        || previous_state.revision > next_state.revision
        || !previous_state.revoked_ids.is_subset(&next_state.revoked_ids)
        || previous_state.policies.iter().any(|p| !next_state.policies.iter().any(|n| n.id == p.id))
    {
        return conflict(previous, next);
    }
    next.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{snapshot, tests::open};
    use reins_proto::authorization::{OperationId, PolicyEffect, PolicyIssuer, PolicyKind, PolicyScope, UsageLimits};
    use reins_proto::oplog::Table;

    fn policy() -> PolicyRecord {
        PolicyRecord {
            v: 1,
            id: "corporate-send".to_owned(),
            revision: 1,
            issuer: PolicyIssuer {
                principal_id: "owner".to_owned(),
                tenant_id: None,
            },
            kind: PolicyKind::Mandatory,
            lifecycle: PolicyLifecycle::Active,
            valid_from: 0,
            expires_at: None,
            scope: PolicyScope {
                operations: vec![OperationId {
                    service: "gmail".to_owned(),
                    capability: "write".to_owned(),
                    action: "send".to_owned(),
                }],
                ..PolicyScope::default()
            },
            conditions: vec![],
            effect: PolicyEffect::Deny,
            parent_policy_ids: vec![],
            limits: UsageLimits::default(),
        }
    }

    fn state() -> AuthorizationState {
        AuthorizationState {
            revision: 1,
            policies: vec![policy()],
            ..AuthorizationState::empty("owner")
        }
    }

    #[test]
    fn absent_policy_is_legacy_but_invalid_and_unknown_policy_are_errors() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());
        assert_eq!(store.authorization_state().unwrap(), None);
        for raw in
            ["garbage", r#"{"v":2,"owner":"owner","revision":1,"conflicted":false,"policies":[],"revoked_ids":[]}"#]
        {
            store.meta_set(RECORD_KEY, raw).unwrap();
            assert!(store.authorization_state().is_err());
        }
    }

    #[test]
    fn owner_cas_never_loses_a_simultaneous_edit() {
        let dir = tempfile::tempdir().unwrap();
        let store = std::sync::Arc::new(open(dir.path()));
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let tasks = [0, 1].map(|_| {
            let store = std::sync::Arc::clone(&store);
            let barrier = std::sync::Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                store.authorization_replace(0, &state()).is_ok()
            })
        });
        assert_eq!(tasks.into_iter().map(|t| t.join().unwrap()).filter(|success| *success).count(), 1);
        assert_eq!(store.authorization_state().unwrap().unwrap(), state());
    }

    #[test]
    fn revoked_id_owner_and_kind_are_immutable() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());
        store.authorization_replace(0, &state()).unwrap();
        let mut revoked = state();
        revoked.revision = 2;
        revoked.policies[0].revision = 2;
        revoked.policies[0].lifecycle = PolicyLifecycle::Revoked;
        revoked.revoked_ids.insert("corporate-send".to_owned());
        store.authorization_replace(1, &revoked).unwrap();
        let mut next = revoked.clone();
        next.revision = 3;
        next.policies[0].revision = 3;
        next.policies[0].lifecycle = PolicyLifecycle::Active;
        assert!(store.authorization_replace(2, &next).is_err());
        next = revoked.clone();
        next.revision = 3;
        next.revoked_ids.clear();
        assert!(store.authorization_replace(2, &next).is_err());
        next = revoked.clone();
        next.revision = 3;
        next.policies[0].revision = 3;
        next.policies[0].kind = PolicyKind::AccountDefault;
        assert!(store.authorization_replace(2, &next).is_err());
        assert_eq!(store.authorization_state().unwrap().unwrap(), revoked);
    }

    #[test]
    fn concurrent_state_changes_hold_and_revocation_wins() {
        let a = state();
        let mut b = state();
        b.revision = 2;
        b.policies[0].revision = 2;
        b.policies[0].lifecycle = PolicyLifecycle::Revoked;
        b.revoked_ids.insert("corporate-send".to_owned());
        let merged: AuthorizationState =
            serde_json::from_value(conflict(&serde_json::to_value(&a).unwrap(), &serde_json::to_value(&b).unwrap()))
                .unwrap();
        assert!(merged.conflicted);
        assert!(merged.revoked_ids.contains("corporate-send"));
        assert_eq!(merged.policies[0].lifecycle, PolicyLifecycle::Revoked);
        assert!(
            incoming(Some(&serde_json::to_value(b).unwrap()), Some(&serde_json::to_value(a).unwrap()))["conflicted"]
                .as_bool()
                .unwrap()
        );
    }

    #[test]
    fn restoring_an_older_snapshot_cannot_remove_an_installed_policy() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());
        let before = snapshot::export(&store.read().unwrap(), true).unwrap();
        store.authorization_replace(0, &state()).unwrap();
        snapshot::import(&mut store.lock().unwrap(), &before, true).unwrap();
        let restored = store.authorization_state().unwrap().unwrap();
        assert!(restored.conflicted);
        assert_eq!(restored.policies[0].effect, PolicyEffect::Deny);
    }

    #[test]
    fn signed_meta_removal_and_unverified_start_hold_instead_of_legacy_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());
        store.authorization_replace(0, &state()).unwrap();
        store.write_sync_row(Table::Meta, RECORD_KEY, None, None).unwrap();
        assert!(store.authorization_state().unwrap().unwrap().conflicted);
        let value = serde_json::to_value(state()).unwrap();
        let crate::store::records::Tightened::Write(value) =
            store.tighten_record(Table::Meta, RECORD_KEY, &value, 100).unwrap()
        else {
            panic!("must retain a hold")
        };
        assert_eq!(value["conflicted"], true);
    }

    #[test]
    fn state_size_and_record_count_are_bounded_before_persistence() {
        let mut oversized = state();
        oversized.revoked_ids = (0..=MAX_POLICIES).map(|i| format!("revoked-{i}")).collect();
        assert!(oversized.validate().is_err());
        oversized = state();
        oversized.owner = "x".repeat(MAX_STATE_BYTES);
        assert!(oversized.validate().is_err());
    }

    #[test]
    fn malformed_updates_cannot_destroy_revocation_floors_before_a_restore() {
        let mut revoked = state();
        revoked.revoked_ids.insert("corporate-send".to_owned());
        revoked.policies[0].lifecycle = PolicyLifecycle::Revoked;
        let revoked = serde_json::to_value(revoked).unwrap();
        for bad in [serde_json::json!(false), serde_json::json!({"v":99}), serde_json::json!({"policies":[]})] {
            let held = incoming(Some(&revoked), Some(&bad));
            assert_eq!(held["revoked_ids"], revoked["revoked_ids"]);
            assert_eq!(held["conflicted"], true);
            let stale = serde_json::to_value(state()).unwrap();
            let restored = incoming(Some(&held), Some(&stale));
            assert_eq!(restored["revoked_ids"], revoked["revoked_ids"]);
            assert_eq!(restored["conflicted"], true);
        }
    }
}
