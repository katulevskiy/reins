//! Atomic positive authorization. Internal grant rows are durable consumption floors, not executable grants.
//! Exact-request receipts survive ambiguous outcomes and are never an independent source of authority.

use std::collections::BTreeSet;

use reins_policy::authorization::{
    AuthorityBindings, BaselineDecision, DecisionOutcome, EvaluationInput, UsageSnapshot,
};
use reins_policy::{Grant, Scope, ServiceScope};
use reins_proto::authorization::{ActionContext, AttributeKey, PolicyKind, PolicyRecord};
use reins_proto::canon::{hex32, sha256};
use reins_proto::ids::{ConnectionId, GrantId};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{AuthorizationState, Store, authorization};
use crate::CoreError;
use crate::authorization_trace::{
    AuthorizationFactSource, AuthorizationPhase, AuthorizationPolicyVersion, AuthorizationReviewSource,
    AuthorizationTrace,
};

pub(crate) const BUDGET_ORIGIN: &str = "policy_budget";
pub(crate) const BUDGET_SERVICE: &str = "authorization_budget";
pub(crate) const BUDGET_PREFIX: &str = "reins-policy-budget-";
pub(crate) const RECEIPT_PREFIX: &str = "authorization.receipt.";
pub(crate) const RECEIPT_STORE_PREFIX: &str = "app.authorization.receipt.";
const REVOKED_CONNECTION_PREFIX: &str = "authorization.revoked_connection.";
const REVOKED_CONNECTION_STORE_PREFIX: &str = "app.authorization.revoked_connection.";

pub(crate) fn retained_metadata_key(key: &str) -> bool {
    key.starts_with(RECEIPT_PREFIX) || key.starts_with(REVOKED_CONNECTION_PREFIX)
}
pub(crate) fn retained_store_key(key: &str) -> bool {
    key.starts_with(RECEIPT_STORE_PREFIX) || key.starts_with(REVOKED_CONNECTION_STORE_PREFIX)
}
fn revoked_connection_key(id: &str) -> String {
    format!("{REVOKED_CONNECTION_STORE_PREFIX}{}", hex32(&sha256(id.as_bytes())))
}
fn connection_revoked(conn: &Connection, id: &str) -> Result<bool, CoreError> {
    conn.query_row("SELECT EXISTS(SELECT 1 FROM meta WHERE key=?1)", [revoked_connection_key(id)], |r| r.get(0))
        .map_err(CoreError::from)
}
pub(super) fn revoke_connection_in(conn: &Connection, id: &str) -> Result<(), CoreError> {
    conn.execute("INSERT OR IGNORE INTO meta(key,value) VALUES (?1,'true')", [revoked_connection_key(id)])?;
    Ok(())
}

const MAX_UNRESOLVED: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ReceiptOutcome {
    Started,
    Succeeded,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReceiptPolicy {
    pub id: String,
    pub revision: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExecutionReceipt {
    pub v: u32,
    pub owner: String,
    pub attempt_id: String,
    pub executor_device_id: String,
    pub roster_epoch: u64,
    pub connection_id: String,
    pub request_id: String,
    pub request_digest: String,
    pub at: i64,
    pub expires_at: i64,
    pub policies: Vec<ReceiptPolicy>,
    pub outcome: ReceiptOutcome,
}

/// Only `permission_reserve` can construct this. It cannot be deserialized, cloned or minted by a native view.
pub(crate) struct VerifiedPermission {
    key: String,
    receipt: ExecutionReceipt,
    trace: AuthorizationTrace,
    temporary: bool,
}

/// Audit-only evidence captured from a locally verified positive path. This is not execution authority.
#[derive(Clone)]
pub(crate) struct PermissionAuditStamp {
    trace: AuthorizationTrace,
    temporary: bool,
}

impl PermissionAuditStamp {
    pub(crate) fn trace(&self) -> &AuthorizationTrace {
        &self.trace
    }

    pub(crate) fn stamp(&self, audit: &mut super::AuditRecord, phase: AuthorizationPhase) {
        audit.info.decided_by = if self.temporary {
            "temporary_permission"
        } else {
            "account_permission"
        }
        .into();
        self.trace.stamp(audit, phase);
    }
}

impl VerifiedPermission {
    pub(crate) fn audit_stamp(&self) -> PermissionAuditStamp {
        PermissionAuditStamp {
            trace: self.trace.clone(),
            temporary: self.temporary,
        }
    }
}

/// Replay protection for an independently authorized legacy send; this grants no account or action authority.
pub(crate) struct SendAttempt {
    key: String,
    receipt: ExecutionReceipt,
}

#[allow(clippy::large_enum_variant, reason = "the short-lived execution proof remains owned by its one invocation")]
pub(crate) enum PermissionReservation {
    Legacy,
    Review,
    Authorized(VerifiedPermission),
}

pub(crate) fn budget_id(policy_id: &str) -> String {
    let parts: Vec<_> = policy_id.split(':').collect();
    let identity = if parts.len() == 3 && matches!(parts[0], "delegation" | "delegation-authority") {
        format!("delegation:{}", parts[1])
    } else {
        policy_id.to_owned()
    };
    format!("{BUDGET_PREFIX}{}", hex32(&sha256(identity.as_bytes())))
}

pub(crate) fn budget_grant(policy: &PolicyRecord, now: i64) -> Result<Grant, CoreError> {
    let maximum =
        policy.limits.max_uses.ok_or_else(|| CoreError::invalid("automatic permissions require a use limit"))?;
    u32::try_from(maximum).map_err(|_| CoreError::invalid("the use limit is too large"))?;
    Grant::new(
        GrantId(budget_id(&policy.id)),
        ConnectionId("__reins_policy_budget__".into()),
        Scope::Service(ServiceScope {
            service: BUDGET_SERVICE.into(),
            access: "read".into(),
            resources: vec!["counter".into()],
            labels: vec![],
            any: false,
            classes: vec![],
            ops: vec![],
        }),
        now,
        None,
        Some(u32::MAX),
    )
    .map_err(|e| CoreError::invalid(e.to_string()))
}

pub(crate) fn is_budget(grant: &Grant) -> bool {
    grant.id.0.starts_with(BUDGET_PREFIX) || matches!(&grant.scope, Scope::Service(s) if s.service == BUDGET_SERVICE)
}

pub(crate) fn reject_budget_id(id: &GrantId) -> Result<(), CoreError> {
    if id.0.starts_with(BUDGET_PREFIX) {
        Err(CoreError::invalid("authorization counters are controlled by their policy"))
    } else {
        Ok(())
    }
}

fn load_budget(conn: &Connection, id: &str) -> Result<Option<Grant>, CoreError> {
    let raw: Option<String> = conn
        .query_row("SELECT grant_json FROM grants WHERE id=?1 AND origin=?2", params![id, BUDGET_ORIGIN], |r| r.get(0))
        .optional()?;
    raw.map(|raw| {
        let grant: Grant =
            serde_json::from_str(&raw).map_err(|_| CoreError::storage("invalid authorization counter"))?;
        if grant.id.0 != id || !is_budget(&grant) {
            return Err(CoreError::storage("authorization counter identity mismatch"));
        }
        Ok(grant)
    })
    .transpose()
}

fn insert_grant(conn: &Connection, grant: &Grant, label: &str, origin: &str) -> Result<(), CoreError> {
    conn.execute(
        "INSERT INTO grants(id,connection_id,connection_label,created_at,grant_json,origin) VALUES (?1,?2,?3,?4,?5,?6)",
        params![
            grant.id.0,
            grant.connection_id.0,
            label,
            grant.created_at,
            serde_json::to_string(grant).map_err(|e| CoreError::storage(e.to_string()))?,
            origin
        ],
    )?;
    Ok(())
}

fn account_admitted(conn: &Connection, action: &ActionContext, now: i64) -> Result<bool, CoreError> {
    if connection_revoked(conn, &action.actor.connection_id)? {
        return Ok(false);
    }
    let tuple: Vec<String> = serde_json::from_str(&action.resource.account_id)
        .map_err(|_| CoreError::invalid("invalid account identity"))?;
    if tuple.len() != 2 || tuple[0] != action.operation.service {
        return Err(CoreError::invalid("invalid account identity"));
    }
    let connected: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM accounts WHERE service=?1 AND account=?2)",
        params![tuple[0], tuple[1]],
        |row| row.get(0),
    )?;
    if !connected {
        return Ok(false);
    }
    let mut stmt = conn.prepare("SELECT grant_json,origin FROM grants WHERE connection_id=?1")?;
    let rows = stmt
        .query_map([&action.actor.connection_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
        .collect::<Result<Vec<_>, _>>()?;
    for (raw, origin) in rows {
        if origin == BUDGET_ORIGIN || !crate::autopilot::store::origin_in_force(conn, &origin, now)? {
            continue;
        }
        let grant: Grant =
            serde_json::from_str(&raw).map_err(|_| CoreError::storage("an account admission could not be verified"))?;
        if grant.connection_id.0 == action.actor.connection_id
            && grant.is_active(now)
            && matches!(&grant.scope, Scope::Accounts(scope) if scope.service == tuple[0] && (scope.accounts.is_empty() || scope.accounts.contains(&tuple[1])))
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn usage(conn: &Connection, policies: &[PolicyRecord]) -> Result<Vec<UsageSnapshot>, CoreError> {
    let mut snapshots = vec![];
    let floors = receipt_floors(conn)?;
    for policy in policies {
        if policy.limits.max_cost_microunits.is_some() {
            return Err(CoreError::invalid("monetary execution evidence is not supported"));
        }
        if policy.limits.max_uses.is_none() {
            continue;
        }
        if let Some(grant) = load_budget(conn, &budget_id(&policy.id))? {
            if grant.revoked {
                continue;
            }
            snapshots.push(UsageSnapshot {
                policy_id: policy.id.clone(),
                revision: policy.revision,
                committed_uses: u64::from(grant.uses).max(floors.get(&grant.id.0).copied().unwrap_or(0)),
                reserved_uses: 0,
                committed_cost_microunits: 0,
                reserved_cost_microunits: 0,
                cost_currency: None,
            });
        }
    }
    Ok(snapshots)
}

fn evaluate(
    conn: &Connection,
    state: &AuthorizationState,
    action: &ActionContext,
    verified: &[AttributeKey],
    now: i64,
) -> Result<reins_policy::authorization::AuthorizationDecision, CoreError> {
    if state.conflicted
        || state.overflow_digest.is_some()
        || state.owner != action.actor.executor_principal_id
        || state.policies.iter().any(|p| p.issuer.tenant_id.is_some())
    {
        return Err(CoreError::invalid("automatic authority requires a complete authenticated policy history"));
    }
    let locked: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM autopilot_settings WHERE scope IN ('',?1) AND mode='lockdown')",
        [&action.actor.connection_id],
        |r| r.get(0),
    )?;
    if locked {
        return Err(CoreError::invalid("Lockdown prevents automatic execution"));
    }
    let ids: Vec<_> = state.policies.iter().map(|p| p.id.clone()).collect();
    let authority = AuthorityBindings {
        principal_id: &action.actor.principal_id,
        tenant_id: action.actor.tenant_id.as_deref(),
        connection_id: &action.actor.connection_id,
        device_id: action.actor.device_id.as_deref(),
        executor_principal_id: &action.actor.executor_principal_id,
        executor_device_id: &action.actor.executor_device_id,
        account_id: &action.resource.account_id,
        resource_id: action.resource.resource_id.as_deref(),
        request_digest: &action.request_digest,
        account_access: account_admitted(conn, action, now)?,
        verified_attribute_keys: verified,
        authorized_policy_ids: &ids,
    };
    Ok(reins_policy::authorization::evaluate(&EvaluationInput {
        action,
        policies: &state.policies,
        authority: &authority,
        now,
        usage: &usage(conn, &state.policies)?,
        baseline: BaselineDecision::Review,
    }))
}

pub(crate) fn is_managed(state: &AuthorizationState, action: &ActionContext) -> bool {
    // Account discovery/admission stays separate. Once managed, newly introduced data/effect capabilities ask too.
    !matches!(action.operation.capability.as_str(), "accounts" | "permission")
        && state.managed_scopes.iter().any(|s| s.account_id == action.resource.account_id)
}

fn receipt_key(action: &ActionContext) -> String {
    let bytes =
        serde_json::to_vec(&(&action.actor.executor_principal_id, &action.actor.connection_id, &action.request_id))
            .expect("identity serializes");
    format!("{RECEIPT_PREFIX}{}", hex32(&sha256(&bytes)))
}

impl Store {
    /// Handover must preserve brakes and consumption history even when no policy document was installed.
    pub(crate) fn has_authorization_protection(&self) -> Result<bool, CoreError> {
        let conn = self.read()?;
        // Unknown/corrupt policy state fails closed instead of being treated as an unmanaged account.
        if authorization::read(&conn)?.is_some() {
            return Ok(true);
        }
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM meta WHERE key GLOB 'app.authorization.*')
                 OR EXISTS(SELECT 1 FROM grants WHERE origin=?1)",
            [BUDGET_ORIGIN],
            |row| row.get(0),
        )
        .map_err(CoreError::from)
    }

    #[cfg(test)]
    fn permission_reserve(
        &self,
        action: &ActionContext,
        verified: &[AttributeKey],
        now: i64,
    ) -> Result<PermissionReservation, CoreError> {
        self.permission_reserve_at_epoch(action, verified, now, 1)
    }

    #[cfg(test)]
    fn permission_revalidate(
        &self,
        proof: &VerifiedPermission,
        action: &ActionContext,
        verified: &[AttributeKey],
        now: i64,
    ) -> Result<(), CoreError> {
        self.permission_revalidate_at_epoch(proof, action, verified, now, 1)
    }

    pub(crate) fn legacy_send_attempt(&self, action: &ActionContext, now: i64) -> Result<SendAttempt, CoreError> {
        action.validate().map_err(|e| CoreError::invalid(e.to_string()))?;
        if action.operation.service != "gmail"
            || action.operation.capability != "write"
            || action.operation.action != "send"
        {
            return Err(CoreError::invalid("this receipt adapter only handles Gmail sending"));
        }
        let conn = self.lock()?;
        let key = receipt_key(action);
        let previous: Option<String> =
            conn.query_row("SELECT value FROM meta WHERE key=?1", [format!("app.{key}")], |r| r.get(0)).optional()?;
        if previous.is_some() {
            return Err(CoreError::invalid(
                "this send already has an execution attempt; check its outcome before issuing a new request",
            ));
        }
        let receipt = ExecutionReceipt {
            v: 1,
            attempt_id: uuid::Uuid::new_v4().to_string(),
            executor_device_id: action.actor.executor_device_id.clone(),
            roster_epoch: 0,
            owner: action.actor.executor_principal_id.clone(),
            connection_id: action.actor.connection_id.clone(),
            request_id: action.request_id.clone(),
            request_digest: action.request_digest.clone(),
            at: now,
            expires_at: now.saturating_add(60),
            policies: vec![],
            outcome: ReceiptOutcome::Started,
        };
        conn.execute(
            "INSERT INTO meta(key,value) VALUES (?1,?2)",
            params![
                format!("app.{key}"),
                serde_json::to_string(&receipt).map_err(|e| CoreError::storage(e.to_string()))?
            ],
        )?;
        Ok(SendAttempt {
            key,
            receipt,
        })
    }

    pub(crate) fn legacy_send_outcome(&self, attempt: SendAttempt, succeeded: bool) -> Result<(), CoreError> {
        let conn = self.lock()?;
        let stored: Option<String> = conn
            .query_row("SELECT value FROM meta WHERE key=?1", [format!("app.{}", attempt.key)], |r| r.get(0))
            .optional()?;
        if stored.as_deref().and_then(|raw| serde_json::from_str::<ExecutionReceipt>(raw).ok()).as_ref()
            != Some(&attempt.receipt)
        {
            return Err(CoreError::invalid("execution attempt changed; its unknown outcome remains held"));
        }
        let mut receipt = attempt.receipt;
        receipt.outcome = if succeeded {
            ReceiptOutcome::Succeeded
        } else {
            ReceiptOutcome::Unknown
        };
        conn.execute(
            "UPDATE meta SET value=?2 WHERE key=?1",
            params![
                format!("app.{}", attempt.key),
                serde_json::to_string(&receipt).map_err(|e| CoreError::storage(e.to_string()))?
            ],
        )?;
        Ok(())
    }
    /// Policy, explicit account admission and fresh budget counters are committed together or not at all.
    pub(crate) fn permissions_apply(
        &self,
        expected: u64,
        next: &AuthorizationState,
        admissions: &[(Grant, String, String)],
        budgets: &[Grant],
    ) -> Result<(), CoreError> {
        let mut conn = self.lock()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        authorization::replace_in(&tx, expected, next)?;
        for (grant, label, origin) in admissions {
            if connection_revoked(&tx, &grant.connection_id.0)? {
                return Err(CoreError::invalid("a selected agent was revoked; reconnect it with a new identity"));
            }
            insert_grant(&tx, grant, label, origin)?;
        }
        let mut seen = BTreeSet::new();
        for grant in budgets {
            if seen.insert(&grant.id) && load_budget(&tx, &grant.id.0)?.is_none() {
                insert_grant(&tx, grant, "Authorization usage", BUDGET_ORIGIN)?;
            }
        }
        tx.commit().map_err(CoreError::from)
    }

    pub(crate) fn permission_reserve_at_epoch(
        &self,
        action: &ActionContext,
        verified: &[AttributeKey],
        now: i64,
        epoch: u64,
    ) -> Result<PermissionReservation, CoreError> {
        if epoch == 0 {
            return Err(CoreError::invalid("a verified roster epoch is required"));
        }
        let mut conn = self.lock()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let Some(state) = authorization::read(&tx)? else {
            return Ok(PermissionReservation::Legacy);
        };
        if !is_managed(&state, action) {
            return Ok(PermissionReservation::Legacy);
        }
        let key = receipt_key(action);
        let stored: Option<String> =
            tx.query_row("SELECT value FROM meta WHERE key=?1", [format!("app.{key}")], |r| r.get(0)).optional()?;
        if stored.is_some() {
            return Err(CoreError::invalid(
                "this request already has an execution attempt; its result must be reconciled before retrying",
            ));
        }
        let decision = evaluate(&tx, &state, action, verified, now)?;
        if matches!(decision.outcome, DecisionOutcome::Deny | DecisionOutcome::Hold) {
            return Err(CoreError::invalid("the action is blocked or its authority cannot be verified"));
        }
        if decision.outcome != DecisionOutcome::Permit || decision.authorization_path.is_empty() {
            return Ok(PermissionReservation::Review);
        }
        let receipts = receipt_rows(&tx)?;
        if receipts.iter().filter(|(_, r)| r.outcome != ReceiptOutcome::Succeeded).count() >= MAX_UNRESOLVED {
            return Err(CoreError::invalid(
                "too many execution outcomes need reconciliation; automatic execution is paused",
            ));
        }
        let mut expires_at = now.saturating_add(60);
        let floors = receipt_floors(&tx)?;
        let mut policies = vec![];
        let mut budgets = BTreeSet::new();
        let mut temporary = false;
        for bound in &decision.authorization_path {
            let policy = state
                .policies
                .iter()
                .find(|p| p.id == bound.id && p.revision == bound.revision)
                .ok_or_else(|| CoreError::invalid("authorization changed before execution"))?;
            temporary |= policy.kind == PolicyKind::Delegation;
            if let Some(expiry) = policy.expires_at {
                expires_at = expires_at.min(expiry);
            }
            if policy.limits.max_uses.is_some() {
                budgets.insert(budget_id(&policy.id));
            }
            policies.push(ReceiptPolicy {
                id: bound.id.clone(),
                revision: bound.revision,
            });
        }
        for id in budgets {
            let mut grant =
                load_budget(&tx, &id)?.ok_or_else(|| CoreError::invalid("authorization usage is unavailable"))?;
            if grant.revoked || grant.max_uses.is_none_or(|limit| grant.uses >= limit) {
                return Err(CoreError::invalid("authorization usage limit reached"));
            }
            grant.uses = u32::try_from(u64::from(grant.uses).max(floors.get(&id).copied().unwrap_or(0)))
                .map_err(|_| CoreError::invalid("authorization usage exhausted"))?
                .checked_add(1)
                .ok_or_else(|| CoreError::invalid("authorization usage exhausted"))?;
            tx.execute(
                "UPDATE grants SET grant_json=?2,last_used_at=?3 WHERE id=?1",
                params![id, serde_json::to_string(&grant).map_err(|e| CoreError::storage(e.to_string()))?, now],
            )?;
        }
        let receipt = ExecutionReceipt {
            v: 1,
            attempt_id: uuid::Uuid::new_v4().to_string(),
            executor_device_id: action.actor.executor_device_id.clone(),
            roster_epoch: epoch,
            owner: state.owner.clone(),
            connection_id: action.actor.connection_id.clone(),
            request_id: action.request_id.clone(),
            request_digest: action.request_digest.clone(),
            at: now,
            expires_at,
            policies,
            outcome: ReceiptOutcome::Started,
        };
        tx.execute(
            "INSERT INTO meta(key,value) VALUES (?1,?2)",
            params![
                format!("app.{key}"),
                serde_json::to_string(&receipt).map_err(|e| CoreError::storage(e.to_string()))?
            ],
        )?;
        let trace = AuthorizationTrace {
            v: 1,
            attempt_id: receipt.attempt_id.clone(),
            request_id: action.request_id.clone(),
            request_digest: action.request_digest.clone(),
            policy_state_revision: state.revision,
            policy_state_digest: hex32(&sha256(
                &serde_json::to_vec(&state).map_err(|e| CoreError::storage(e.to_string()))?,
            )),
            policies: state
                .policies
                .iter()
                .map(|policy| AuthorizationPolicyVersion {
                    id: policy.id.clone(),
                    revision: policy.revision,
                })
                .collect(),
            determining_policy_ids: decision.determining_policies,
            required_reviews: decision.required_reviews,
            source: AuthorizationReviewSource::Automatic,
            verified_facts: action
                .facts
                .iter()
                .filter_map(|fact| match &fact.fact {
                    reins_proto::authorization::Fact::Known {
                        evidence,
                        ..
                    } if verified.contains(&fact.key) => Some(AuthorizationFactSource {
                        key: fact.key,
                        source: evidence.source,
                        observed_at: evidence.observed_at,
                        valid_until: evidence.valid_until,
                    }),
                    _ => None,
                })
                .collect(),
            checked_at: now,
            phase: AuthorizationPhase::Started,
        };
        tx.commit()?;
        Ok(PermissionReservation::Authorized(VerifiedPermission {
            key,
            receipt,
            trace,
            temporary,
        }))
    }

    /// Recheck exact identity, membership/admission, every policy revision and mandatory requirement immediately
    /// before the adapter acts. This spends nothing further; the preceding atomic transaction already did so.
    pub(crate) fn permission_revalidate_at_epoch(
        &self,
        proof: &VerifiedPermission,
        action: &ActionContext,
        verified: &[AttributeKey],
        now: i64,
        epoch: u64,
    ) -> Result<(), CoreError> {
        if epoch == 0
            || proof.receipt.roster_epoch != epoch
            || proof.receipt.executor_device_id != action.actor.executor_device_id
            || proof.receipt.request_digest != action.request_digest
            || proof.key != receipt_key(action)
            || now >= proof.receipt.expires_at
        {
            return Err(CoreError::invalid("execution authorization changed or expired"));
        }
        let conn = self.read()?;
        let stored: Option<String> = conn
            .query_row("SELECT value FROM meta WHERE key=?1", [format!("app.{}", proof.key)], |r| r.get(0))
            .optional()?;
        let current: ExecutionReceipt = stored
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .ok_or_else(|| CoreError::invalid("execution attempt could not be verified"))?;
        if current != proof.receipt || current.outcome != ReceiptOutcome::Started {
            return Err(CoreError::invalid("this execution attempt was already resolved or changed"));
        }
        let state =
            authorization::read(&conn)?.ok_or_else(|| CoreError::invalid("execution authorization is unavailable"))?;
        let ids: BTreeSet<_> = proof.receipt.policies.iter().map(|p| (&p.id, p.revision)).collect();
        if !ids.iter().all(|(id, revision)| {
            state.policies.iter().any(|p| {
                &p.id == *id
                    && p.revision == *revision
                    && p.lifecycle == reins_proto::authorization::PolicyLifecycle::Active
                    && p.valid_from <= now
                    && p.expires_at.is_none_or(|t| now < t)
            })
        }) {
            return Err(CoreError::invalid("execution permission was paused, revoked or changed"));
        }
        let floors = receipt_floors(&conn)?;
        for policy in &state.policies {
            if !ids.contains(&(&policy.id, policy.revision)) {
                continue;
            }
            if let Some(maximum) = policy.limits.max_uses {
                let id = budget_id(&policy.id);
                let counter =
                    load_budget(&conn, &id)?.ok_or_else(|| CoreError::invalid("authorization usage is unavailable"))?;
                let consumed = u64::from(counter.uses).max(floors.get(&id).copied().unwrap_or(0));
                if counter.revoked || consumed > maximum {
                    return Err(CoreError::invalid(
                        "concurrent execution exhausted the delegation before this action started",
                    ));
                }
            }
        }
        // The already-reserved use may be the final one. Re-evaluate ceilings/evidence/admission without spending a
        // second use; only this exact proof's path has its use budgets lifted for this one check.
        let mut check = state;
        for policy in &mut check.policies {
            if ids.contains(&(&policy.id, policy.revision)) {
                policy.limits.max_uses = None;
            }
        }
        let decision = evaluate(&conn, &check, action, verified, now)?;
        if decision.outcome != DecisionOutcome::Permit
            || decision.authorization_path.iter().map(|p| (&p.id, p.revision)).collect::<BTreeSet<_>>() != ids
        {
            return Err(CoreError::invalid("execution permission no longer satisfies the reserved account policy"));
        }
        Ok(())
    }

    pub(crate) fn permission_outcome(&self, proof: VerifiedPermission, succeeded: bool) -> Result<(), CoreError> {
        let conn = self.lock()?;
        let stored: Option<String> = conn
            .query_row("SELECT value FROM meta WHERE key=?1", [format!("app.{}", proof.key)], |r| r.get(0))
            .optional()?;
        if stored.as_deref().and_then(|raw| serde_json::from_str::<ExecutionReceipt>(raw).ok()).as_ref()
            != Some(&proof.receipt)
        {
            return Err(CoreError::invalid("execution attempt changed; its unknown outcome remains held"));
        }
        let mut receipt = proof.receipt;
        receipt.outcome = if succeeded {
            ReceiptOutcome::Succeeded
        } else {
            ReceiptOutcome::Unknown
        };
        conn.execute(
            "UPDATE meta SET value=?2 WHERE key=?1",
            params![
                format!("app.{}", proof.key),
                serde_json::to_string(&receipt).map_err(|e| CoreError::storage(e.to_string()))?
            ],
        )?;
        Ok(())
    }

    pub(crate) fn permission_history_ready(&self) -> Result<bool, CoreError> {
        let conn = self.read()?;
        Ok(receipt_rows(&conn)?.iter().filter(|(_, receipt)| receipt.outcome != ReceiptOutcome::Succeeded).count()
            < MAX_UNRESOLVED)
    }

    pub(crate) fn permission_connection_revoked(&self, id: &str) -> Result<bool, CoreError> {
        connection_revoked(&*self.read()?, id)
    }

    pub(crate) fn permission_budget_uses(&self, policy_id: &str) -> Result<u64, CoreError> {
        let conn = self.read()?;
        let id = budget_id(policy_id);
        Ok(load_budget(&conn, &id)?
            .map_or(0, |g| u64::from(g.uses))
            .max(receipt_floors(&conn)?.get(&id).copied().unwrap_or(0)))
    }

    pub(crate) fn permission_unresolved(&self, policy_ids: &[String]) -> Result<u64, CoreError> {
        let conn = self.read()?;
        Ok(u64::try_from(
            receipt_rows(&conn)?
                .iter()
                .filter(|(_, r)| {
                    r.outcome != ReceiptOutcome::Succeeded && r.policies.iter().any(|p| policy_ids.contains(&p.id))
                })
                .count(),
        )
        .unwrap_or(u64::MAX))
    }
}

fn receipt_rows(conn: &Connection) -> Result<Vec<(String, ExecutionReceipt)>, CoreError> {
    let mut stmt = conn.prepare("SELECT key,value FROM meta WHERE key LIKE ?1")?;
    let rows = stmt
        .query_map([format!("{RECEIPT_STORE_PREFIX}%")], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
        .collect::<Result<Vec<_>, _>>()?;
    rows.into_iter()
        .map(|(key, raw)| {
            let receipt: ExecutionReceipt =
                serde_json::from_str(&raw).map_err(|_| CoreError::storage("an execution receipt is invalid"))?;
            if receipt.v != 1 {
                return Err(CoreError::storage("this execution receipt version needs an app update"));
            }
            let identity = serde_json::to_vec(&(&receipt.owner, &receipt.connection_id, &receipt.request_id))
                .expect("identity serializes");
            let expected = format!("{RECEIPT_STORE_PREFIX}{}", hex32(&sha256(&identity)));
            if key != expected
                || uuid::Uuid::parse_str(&receipt.attempt_id).is_err()
                || receipt.policies.len() > reins_proto::authorization::MAX_AUTHORIZATION_ITEMS
                || receipt
                    .policies
                    .iter()
                    .any(|policy| policy.id.is_empty() || policy.id.len() > 2048 || policy.revision == 0)
            {
                return Err(CoreError::storage("an execution receipt identity is invalid"));
            }
            Ok((key, receipt))
        })
        .collect()
}

fn receipt_floors(conn: &Connection) -> Result<std::collections::BTreeMap<String, u64>, CoreError> {
    let mut floors = std::collections::BTreeMap::new();
    for (_, receipt) in receipt_rows(conn)? {
        // A child and all its ancestors can share one bounded delegation counter. Spend it once per attempt.
        let budgets: BTreeSet<_> = receipt.policies.iter().map(|p| budget_id(&p.id)).collect();
        for budget in budgets {
            *floors.entry(budget).or_insert(0u64) += 1;
        }
    }
    Ok(floors)
}

/// Presence is the replay barrier. Missing, malformed or conflicting replicas can never erase it.
pub(crate) fn receipt_incoming(previous: Option<&Value>, next: Option<&Value>) -> Value {
    match (previous, next) {
        (Some(a), Some(b)) if a == b => a.clone(),
        (Some(a), Some(b)) => {
            let parsed = (
                serde_json::from_value::<ExecutionReceipt>(a.clone()),
                serde_json::from_value::<ExecutionReceipt>(b.clone()),
            );
            if let (Ok(mut old), Ok(new)) = parsed {
                if old.v != 1 || new.v != 1 {
                    return serde_json::json!({"execution_attempt":"unsupported_version"});
                }
                let mut comparison = new.clone();
                comparison.outcome = old.outcome;
                if old == comparison {
                    old.outcome = match (old.outcome, new.outcome) {
                        (ReceiptOutcome::Succeeded, _) | (_, ReceiptOutcome::Succeeded) => ReceiptOutcome::Succeeded,
                        (ReceiptOutcome::Unknown, _) | (_, ReceiptOutcome::Unknown) => ReceiptOutcome::Unknown,
                        _ => ReceiptOutcome::Started,
                    };
                    return serde_json::to_value(old).expect("receipt serializes");
                }
            }
            // Different attempts for one request are ambiguous. Both replicas converge to a hold marker;
            // neither retains a local Started receipt that could validate its own execution proof.
            serde_json::json!({"execution_attempt":"conflict"})
        }
        (Some(a), None) => a.clone(),
        (None, Some(b)) => b.clone(),
        (None, None) => serde_json::json!({"execution_attempt":"unverified"}),
    }
}

/// Disconnect is a narrowing authority change, performed in the account-removal transaction.
/// Retain counter/receipt history and mandatory restrictions; reconnect can only create fresh authority IDs.
pub(super) fn account_removed(conn: &Connection, service: &str, account: &str) -> Result<(), CoreError> {
    use reins_proto::authorization::{PolicyEffect, PolicyLifecycle};
    let account_id = crate::authorization::account_resource_id(service, account);
    if let Some(mut state) = authorization::read(conn)? {
        let expected = state.revision;
        let mut changed = false;
        for policy in &mut state.policies {
            if policy.scope.account_ids.contains(&account_id)
                && policy.lifecycle != PolicyLifecycle::Revoked
                && matches!(
                    policy.effect,
                    PolicyEffect::Permit | PolicyEffect::EligibleForAuto | PolicyEffect::DelegationAuthority
                )
            {
                policy.lifecycle = PolicyLifecycle::Revoked;
                policy.revision =
                    policy.revision.checked_add(1).ok_or_else(|| CoreError::invalid("policy revision exhausted"))?;
                state.revoked_ids.insert(policy.id.clone());
                changed = true;
            }
        }
        if changed {
            state.revision = expected.checked_add(1).ok_or_else(|| CoreError::invalid("policy revision exhausted"))?;
            authorization::replace_in(conn, expected, &state)?;
        }
    }
    let mut statement = conn.prepare("SELECT grant_json FROM grants WHERE origin != ?1")?;
    let grants = statement.query_map([BUDGET_ORIGIN], |row| row.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
    let mut statement = conn.prepare("SELECT account FROM accounts WHERE service=?1 ORDER BY account")?;
    let remaining = statement.query_map([service], |row| row.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
    for raw in grants {
        let mut grant: Grant = serde_json::from_str(&raw)
            .map_err(|_| CoreError::storage("an account permission could not be verified"))?;
        if let Scope::Accounts(scope) = &mut grant.scope {
            if scope.service != service
                || (!scope.accounts.is_empty() && !scope.accounts.iter().any(|id| id == account))
            {
                continue;
            }
            if scope.accounts.is_empty() {
                scope.accounts.clone_from(&remaining);
            } else {
                scope.accounts.retain(|id| id != account);
            }
            if scope.accounts.is_empty() {
                grant.revoked = true;
            }
            conn.execute(
                "UPDATE grants SET grant_json=?2 WHERE id=?1",
                params![grant.id.0, serde_json::to_string(&grant).map_err(|e| CoreError::storage(e.to_string()))?],
            )?;
        }
    }
    Ok(())
}

pub(super) struct PreservedPermissions {
    receipts: Vec<(String, String)>,
    budgets: Vec<Grant>,
}

pub(super) fn before_import(conn: &Connection) -> Result<PreservedPermissions, CoreError> {
    let mut statement = conn.prepare("SELECT key,value FROM meta WHERE key LIKE ?1 OR key LIKE ?2")?;
    let receipts = statement
        .query_map([format!("{RECEIPT_STORE_PREFIX}%"), format!("{REVOKED_CONNECTION_STORE_PREFIX}%")], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut statement = conn.prepare("SELECT grant_json FROM grants WHERE origin=?1")?;
    let rows = statement.query_map([BUDGET_ORIGIN], |r| r.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
    let budgets = rows
        .into_iter()
        .map(|raw| {
            serde_json::from_str(&raw).map_err(|_| CoreError::storage("an authorization counter cannot be verified"))
        })
        .collect::<Result<_, _>>()?;
    Ok(PreservedPermissions {
        receipts,
        budgets,
    })
}

pub(super) fn after_import(conn: &Connection, previous: PreservedPermissions) -> Result<(), CoreError> {
    for (key, raw) in previous.receipts {
        let old = serde_json::from_str(&raw).unwrap_or_else(|_| serde_json::json!({"execution_attempt":"unverified"}));
        let new: Option<String> =
            conn.query_row("SELECT value FROM meta WHERE key=?1", [&key], |r| r.get(0)).optional()?;
        let new = new.and_then(|text| serde_json::from_str(&text).ok());
        let kept = receipt_incoming(Some(&old), new.as_ref());
        conn.execute(
            "INSERT INTO meta(key,value) VALUES (?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![key, kept.to_string()],
        )?;
    }
    for old in previous.budgets {
        if let Some(new) = load_budget(conn, &old.id.0)? {
            let joined = super::lattice::join_grants(&new, &old, None);
            conn.execute(
                "UPDATE grants SET grant_json=?2 WHERE id=?1",
                params![old.id.0, serde_json::to_string(&joined).map_err(|e| CoreError::storage(e.to_string()))?],
            )?;
        } else {
            insert_grant(conn, &old, "Authorization usage", BUDGET_ORIGIN)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::authorization::ManagedOperation;
    use reins_proto::authorization::*;
    use std::sync::Arc;

    fn action(id: &str) -> ActionContext {
        ActionContext {
            v: 1,
            request_id: id.into(),
            request_digest: hex32(&sha256(id.as_bytes())),
            actor: ActorContext {
                principal_id: "owner".into(),
                tenant_id: None,
                connection_id: "agent".into(),
                device_id: None,
                executor_principal_id: "owner".into(),
                executor_device_id: "phone".into(),
            },
            operation: OperationId {
                service: "gmail".into(),
                capability: "write".into(),
                action: "send".into(),
            },
            resource: ResourceContext {
                account_id: crate::authorization::account_resource_id("gmail", "me@example.com"),
                resource_id: None,
            },
            facts: vec![AttributeFact {
                key: AttributeKey::Recipients,
                fact: Fact::Known {
                    value: AttributeValue::StringSet(vec!["friend@example.com".into()]),
                    evidence: Evidence {
                        id: "recipients".into(),
                        source: EvidenceSource::LocalCore,
                        observed_at: 10,
                        valid_until: 100,
                    },
                },
            }],
        }
    }

    fn policy(id: &str, limit: u64) -> PolicyRecord {
        PolicyRecord {
            v: 1,
            id: id.into(),
            revision: 1,
            issuer: PolicyIssuer {
                principal_id: "owner".into(),
                tenant_id: None,
            },
            kind: PolicyKind::Permission,
            lifecycle: PolicyLifecycle::Active,
            valid_from: 1,
            expires_at: Some(100),
            scope: PolicyScope {
                connection_ids: vec!["agent".into()],
                account_ids: vec![action("a").resource.account_id],
                operations: vec![action("a").operation],
                ..PolicyScope::default()
            },
            conditions: vec![Condition::SetWithin {
                key: AttributeKey::Recipients,
                values: vec!["friend@example.com".into()],
                require_nonempty: true,
            }],
            effect: PolicyEffect::Permit,
            parent_policy_ids: vec![],
            limits: UsageLimits {
                max_uses: Some(limit),
                ..UsageLimits::default()
            },
        }
    }

    fn setup(policies: Vec<PolicyRecord>) -> (tempfile::TempDir, Arc<Store>) {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(crate::store::tests::open(dir.path()));
        store.add_account("gmail", "me@example.com", 1).unwrap();
        let mut state = AuthorizationState::empty("owner");
        state.revision = 1;
        state.managed_scopes.insert(ManagedOperation {
            account_id: action("a").resource.account_id,
            operation: action("a").operation,
        });
        let budgets = policies
            .iter()
            .filter(|p| p.limits.max_uses.is_some())
            .map(|p| budget_grant(p, 10).unwrap())
            .collect::<Vec<_>>();
        state.policies = policies;
        let admission = Grant::new(
            GrantId("admission".into()),
            ConnectionId("agent".into()),
            Scope::Accounts(reins_policy::AccountsScope {
                service: "gmail".into(),
                accounts: vec!["me@example.com".into()],
            }),
            1,
            None,
            None,
        )
        .unwrap();
        store.permissions_apply(0, &state, &[(admission, "Agent".into(), String::new())], &budgets).unwrap();
        (dir, store)
    }

    fn reserve(store: &Store, id: &str) -> VerifiedPermission {
        match store.permission_reserve(&action(id), &[AttributeKey::Recipients], 10).unwrap() {
            PermissionReservation::Authorized(proof) => proof,
            _ => panic!("expected permit"),
        }
    }

    #[test]
    fn handover_protection_includes_retained_brakes_receipts_and_budget_floors() {
        for key in [
            "app.authorization.revoked_connection.agent",
            "app.authorization.receipt.attempt",
            "app.authorization.future_brake",
        ] {
            let dir = tempfile::tempdir().unwrap();
            let store = crate::store::tests::open(dir.path());
            assert!(!store.has_authorization_protection().unwrap());
            store.lock().unwrap().execute("INSERT INTO meta(key,value) VALUES (?1,'true')", [key]).unwrap();
            assert!(store.authorization_state().unwrap().is_none());
            assert!(store.has_authorization_protection().unwrap(), "{key}");
        }
        let (_dir, store) = setup(vec![policy("permission", 3)]);
        assert!(store.has_authorization_protection().unwrap());
        store.lock().unwrap().execute("DELETE FROM meta WHERE key=?1", [authorization::STORE_KEY]).unwrap();
        assert!(store.authorization_state().unwrap().is_none());
        assert!(store.has_authorization_protection().unwrap(), "hidden budget rows still carry consumption");
        store.lock().unwrap().execute("DELETE FROM grants WHERE origin=?1", [BUDGET_ORIGIN]).unwrap();
        assert!(!store.has_authorization_protection().unwrap(), "ordinary account grants are not policy state");
        store
            .lock()
            .unwrap()
            .execute("INSERT INTO meta(key,value) VALUES (?1,'invalid')", [authorization::STORE_KEY])
            .unwrap();
        assert!(store.has_authorization_protection().is_err());
    }

    #[test]
    fn atomic_concurrent_reservations_never_overspend_and_retries_never_mint_another_attempt() {
        let (_dir, store) = setup(vec![policy("permission", 3)]);
        #[allow(
            clippy::needless_collect,
            reason = "start all worker threads before joining to exercise concurrent reservations"
        )]
        let workers: Vec<_> = (0..12)
            .map(|i| {
                let store = Arc::clone(&store);
                std::thread::spawn(move || {
                    matches!(
                        store.permission_reserve(&action(&format!("r{i}")), &[AttributeKey::Recipients], 10).unwrap(),
                        PermissionReservation::Authorized(_)
                    )
                })
            })
            .collect();
        assert_eq!(workers.into_iter().map(|w| w.join().unwrap()).filter(|allowed| *allowed).count(), 3);
        assert_eq!(store.permission_budget_uses("permission").unwrap(), 3);
        let (_dir2, second) = setup(vec![policy("permission", 3)]);
        let proof = reserve(&second, "same");
        second.permission_revalidate(&proof, &action("same"), &[AttributeKey::Recipients], 10).unwrap();
        assert!(second.permission_reserve(&action("same"), &[AttributeKey::Recipients], 10).is_err());
        let mut changed = action("same");
        changed.request_digest = hex32(&sha256(b"changed"));
        assert!(second.permission_reserve(&changed, &[AttributeKey::Recipients], 10).is_err());
        second.permission_outcome(proof, false).unwrap();
        assert_eq!(second.permission_budget_uses("permission").unwrap(), 1);
        assert!(second.permission_reserve(&action("same"), &[AttributeKey::Recipients], 11).is_err());
    }

    #[test]
    fn exact_account_agent_recipient_admission_and_expiry_are_enforced() {
        let (_dir, store) = setup(vec![policy("permission", 3)]);
        let mut wrong = action("wrong");
        wrong.actor.connection_id = "other".into();
        assert!(store.permission_reserve(&wrong, &[AttributeKey::Recipients], 10).is_err());
        let mut wrong = action("wrong");
        wrong.resource.account_id = crate::authorization::account_resource_id("gmail", "other@example.com");
        assert!(matches!(
            store.permission_reserve(&wrong, &[AttributeKey::Recipients], 10).unwrap(),
            PermissionReservation::Legacy
        ));
        let mut cc = action("cc");
        cc.facts[0].fact = Fact::Known {
            value: AttributeValue::StringSet(vec!["friend@example.com".into(), "corporate@example.com".into()]),
            evidence: Evidence {
                id: "cc".into(),
                source: EvidenceSource::LocalCore,
                observed_at: 10,
                valid_until: 100,
            },
        };
        assert!(matches!(
            store.permission_reserve(&cc, &[AttributeKey::Recipients], 10).unwrap(),
            PermissionReservation::Review
        ));
        assert!(!matches!(
            store.permission_reserve(&action("untrusted"), &[], 10).unwrap(),
            PermissionReservation::Authorized(_)
        ));
        assert!(matches!(
            store.permission_reserve(&action("expired"), &[AttributeKey::Recipients], 100).unwrap(),
            PermissionReservation::Review
        ));
        let proof = reserve(&store, "bound");
        let mut device = action("bound");
        device.actor.executor_device_id = "other-phone".into();
        assert!(store.permission_revalidate(&proof, &device, &[AttributeKey::Recipients], 10).is_err());
        assert!(store.permission_revalidate(&proof, &action("bound"), &[AttributeKey::Recipients], 70).is_err());
    }

    #[test]
    fn every_ancestor_shares_one_budget_and_pause_or_new_mandatory_review_invalidates_receipt() {
        let mut authority = policy("delegation-authority:d:mail-send", 2);
        authority.effect = PolicyEffect::DelegationAuthority;
        let mut child = policy("delegation:d:mail-send", 2);
        child.kind = PolicyKind::Delegation;
        child.parent_policy_ids = vec![authority.id.clone()];
        let (_dir, store) = setup(vec![authority, child]);
        let proof = reserve(&store, "first");
        assert_eq!(store.permission_budget_uses("delegation:d:mail-send").unwrap(), 1);
        let mut state = store.authorization_state().unwrap().unwrap();
        state.revision += 1;
        state.policies[0].lifecycle = PolicyLifecycle::Paused;
        state.policies[0].revision += 1;
        store.permissions_apply(1, &state, &[], &[]).unwrap();
        assert!(store.permission_revalidate(&proof, &action("first"), &[AttributeKey::Recipients], 10).is_err());
        assert!(matches!(
            store.permission_reserve(&action("paused"), &[AttributeKey::Recipients], 10).unwrap(),
            PermissionReservation::Review
        ));
        state.policies[0].lifecycle = PolicyLifecycle::Active;
        state.policies[0].revision += 1;
        state.revision += 1;
        store.permissions_apply(2, &state, &[], &[]).unwrap();
        assert_eq!(store.permission_budget_uses("delegation:d:mail-send").unwrap(), 1);
        let proof = reserve(&store, "second");
        let mut review = policy("mandatory-review", 2);
        review.kind = PolicyKind::Mandatory;
        review.limits = UsageLimits::default();
        review.effect = PolicyEffect::RequireReview {
            requirements: vec![ReviewRequirement {
                role_id: "legal".into(),
                quorum: 2,
                distinct_humans_group: None,
                exclude_requester: true,
            }],
        };
        state.policies.push(review);
        state.revision += 1;
        store.permissions_apply(3, &state, &[], &[]).unwrap();
        assert!(store.permission_revalidate(&proof, &action("second"), &[AttributeKey::Recipients], 10).is_err());
        assert_eq!(store.permission_budget_uses("delegation-authority:d:mail-send").unwrap(), 2);
    }

    #[test]
    fn receipts_count_concurrent_branches_without_common_base_and_survive_missing_or_older_counters() {
        let (_da, a) = setup(vec![policy("permission", 2)]);
        let (_db, b) = setup(vec![policy("permission", 2)]);
        let pa = reserve(&a, "a");
        let pb = reserve(&b, "b");
        let branch = {
            let conn = b.read().unwrap();
            before_import(&conn).unwrap()
        };
        {
            let conn = a.lock().unwrap();
            after_import(&conn, branch).unwrap();
        }
        assert_eq!(a.permission_budget_uses("permission").unwrap(), 2, "max(1,1) counters alone would lose a branch");
        a.permission_revalidate(&pa, &action("a"), &[AttributeKey::Recipients], 10).unwrap();
        let conn = a.lock().unwrap();
        conn.execute("DELETE FROM grants WHERE origin=?1", [BUDGET_ORIGIN]).unwrap();
        drop(conn);
        assert_eq!(a.permission_budget_uses("permission").unwrap(), 2);
        assert!(!matches!(
            a.permission_reserve(&action("new"), &[AttributeKey::Recipients], 10).unwrap(),
            PermissionReservation::Authorized(_)
        ));
        assert!(a.permission_revalidate(&pa, &action("a"), &[AttributeKey::Recipients], 10).is_err());
        b.permission_outcome(pb, false).unwrap();
        let saved = {
            let conn = b.read().unwrap();
            before_import(&conn).unwrap()
        };
        {
            let conn = b.lock().unwrap();
            conn.execute("DELETE FROM meta WHERE key LIKE ?1", [format!("{RECEIPT_STORE_PREFIX}%")]).unwrap();
            conn.execute("DELETE FROM grants WHERE origin=?1", [BUDGET_ORIGIN]).unwrap();
            after_import(&conn, saved).unwrap();
        }
        assert_eq!(b.permission_budget_uses("permission").unwrap(), 1);
        assert!(b.permission_reserve(&action("b"), &[AttributeKey::Recipients], 10).is_err());
    }

    #[test]
    fn simultaneous_same_request_attempts_and_changed_devices_converge_to_hold() {
        let (_da, a) = setup(vec![policy("permission", 4)]);
        let (_db, b) = setup(vec![policy("permission", 4)]);
        let pa = reserve(&a, "same");
        let pb = reserve(&b, "same");
        assert_ne!(pa.receipt.attempt_id, pb.receipt.attempt_id);
        let va = serde_json::to_value(&pa.receipt).unwrap();
        let vb = serde_json::to_value(&pb.receipt).unwrap();
        assert_eq!(receipt_incoming(Some(&va), Some(&vb)), receipt_incoming(Some(&vb), Some(&va)));
        let mut device = pb.receipt.clone();
        device.executor_device_id = "another-primary".into();
        assert_eq!(
            receipt_incoming(Some(&va), Some(&serde_json::to_value(device).unwrap())),
            serde_json::json!({"execution_attempt":"conflict"})
        );
        for (store, proof) in [(&a, pa), (&b, pb)] {
            let merged = receipt_incoming(Some(&va), Some(&vb));
            store
                .lock()
                .unwrap()
                .execute(
                    "UPDATE meta SET value=?2 WHERE key=?1",
                    params![format!("app.{}", proof.key), merged.to_string()],
                )
                .unwrap();
            assert!(store.permission_revalidate(&proof, &action("same"), &[AttributeKey::Recipients], 10).is_err());
            assert!(store.permission_outcome(proof, true).is_err(), "a local outcome cannot erase conflict");
        }
    }

    #[test]
    fn internal_counters_are_not_grants_and_managed_accounts_never_return_to_legacy() {
        let (_dir, store) = setup(vec![policy("permission", 3)]);
        assert!(store.grants().unwrap().iter().all(|g| !is_budget(&g.grant)));
        let id = GrantId(budget_id("permission"));
        assert!(store.revoke_grant(&id).is_err());
        assert!(store.extend_grant(&id, 10, 10).is_err());
        assert!(store.resume_grant(&id, 10, 10).is_err());
        assert!(store.refund(&BTreeSet::from([id])).is_err());
        let mut state = store.authorization_state().unwrap().unwrap();
        state.managed_scopes.clear();
        state.revision += 1;
        assert!(store.permissions_apply(1, &state, &[], &[]).is_err());
        store
            .lock()
            .unwrap()
            .execute("INSERT INTO autopilot_settings(scope,mode,updated_at) VALUES ('','lockdown',10)", [])
            .unwrap();
        assert!(store.permission_reserve(&action("locked"), &[AttributeKey::Recipients], 10).is_err());
    }

    #[test]
    fn new_verified_account_baseline_is_atomic_and_reconnect_does_not_change_existing_accounts() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::store::tests::open(dir.path());
        store.add_account("gmail", "old@example.com", 1).unwrap();
        assert!(!store.add_verified_account("owner", "gmail", "old@example.com", 10).unwrap());
        assert!(store.authorization_state().unwrap().is_none());
        assert!(store.add_verified_account("owner", "gmail", "me@example.com", 10).unwrap());
        assert!(
            store.permission_reserve(&action("new"), &[AttributeKey::Recipients], 10).is_err(),
            "account defaults grant no independent admission"
        );
        assert!(
            store
                .authorization_state()
                .unwrap()
                .unwrap()
                .managed_scopes
                .iter()
                .any(|s| s.account_id == action("x").resource.account_id)
        );
        let conn = store.lock().unwrap();
        conn.execute_batch(
            "CREATE TEMP TRIGGER refuse_baseline BEFORE UPDATE ON meta BEGIN SELECT RAISE(ABORT,'test failure'); END;",
        )
        .unwrap();
        drop(conn);
        assert!(store.add_verified_account("owner", "gmail", "failed@example.com", 11).is_err());
        assert!(!store.accounts().unwrap().iter().any(|a| a.account == "failed@example.com"));
    }
    #[test]
    fn unknown_settlement_is_monotone_commutative_and_survives_sync_and_restore() {
        let (_dir, store) = setup(vec![policy("permission", 3)]);
        let proof = reserve(&store, "settlement");
        let started = serde_json::to_value(&proof.receipt).unwrap();
        let mut unknown = proof.receipt.clone();
        unknown.outcome = ReceiptOutcome::Unknown;
        let unknown = serde_json::to_value(unknown).unwrap();
        assert_eq!(receipt_incoming(Some(&started), Some(&unknown)), unknown);
        assert_eq!(receipt_incoming(Some(&unknown), Some(&started)), unknown);
        let previous = PreservedPermissions {
            receipts: vec![(format!("app.{}", proof.key), unknown.to_string())],
            budgets: vec![],
        };
        {
            let conn = store.lock().unwrap();
            after_import(&conn, previous).unwrap();
        }
        assert!(store.permission_revalidate(&proof, &action("settlement"), &[AttributeKey::Recipients], 10).is_err());
        store.write_sync_row(reins_proto::oplog::Table::Meta, &proof.key, None, Some(&started)).unwrap();
        let kept: String = store
            .read()
            .unwrap()
            .query_row("SELECT value FROM meta WHERE key=?1", [format!("app.{}", proof.key)], |row| row.get(0))
            .unwrap();
        assert_eq!(serde_json::from_str::<Value>(&kept).unwrap(), unknown);
        assert!(store.permission_outcome(proof, true).is_err(), "local completion cannot erase imported settlement");
    }
    #[test]
    fn imported_positive_policies_without_managed_markers_never_reenable_legacy() {
        for lifecycle in [PolicyLifecycle::Active, PolicyLifecycle::Paused, PolicyLifecycle::Revoked] {
            for service in ["gmail", "telegram"] {
                let (_dir, store) = setup(vec![policy("permission", 3)]);
                let mut state = store.authorization_state().unwrap().unwrap();
                state.managed_scopes.clear();
                state.policies[0].lifecycle = lifecycle;
                state.policies[0].scope.operations[0].service = service.into();
                state.policies[0].scope.account_ids[0] =
                    crate::authorization::account_resource_id(service, "me@example.com");
                if lifecycle == PolicyLifecycle::Revoked {
                    state.revoked_ids.insert("permission".into());
                }
                assert!(state.validate().is_err());
                store.meta_set(authorization::RECORD_KEY, &serde_json::to_string(&state).unwrap()).unwrap();
                assert!(
                    store.permission_reserve(&action("imported"), &[AttributeKey::Recipients], 10).is_err(),
                    "missing marker is invalid authority, never legacy absence"
                );
            }
        }
    }
    #[test]
    fn revoked_connections_dominate_admissions_and_survive_sync_and_restore() {
        let (_dir, store) = setup(vec![policy("permission", 3)]);
        let proof = reserve(&store, "before-revoke");
        let prior = store.export_account().unwrap();
        store.remove_connection_grants(&"agent".into()).unwrap();
        assert!(store.permission_connection_revoked("agent").unwrap());
        assert!(store.permission_reserve(&action("after-revoke"), &[AttributeKey::Recipients], 10).is_err());
        assert!(
            store.permission_revalidate(&proof, &action("before-revoke"), &[AttributeKey::Recipients], 10).is_err()
        );
        let key = revoked_connection_key("agent");
        let logical = key.strip_prefix("app.").unwrap();
        store.write_sync_row(reins_proto::oplog::Table::Meta, logical, None, None).unwrap();
        assert!(store.permission_connection_revoked("agent").unwrap(), "signed remove cannot erase revocation");
        store.import_account(&prior).unwrap();
        assert!(store.permission_connection_revoked("agent").unwrap(), "old restore cannot erase revocation");
        assert!(store.permission_reserve(&action("restored"), &[AttributeKey::Recipients], 10).is_err());
    }

    #[test]
    fn unsupported_receipt_versions_hold_and_cannot_promote_settlement() {
        let (_dir, store) = setup(vec![policy("permission", 3)]);
        let proof = reserve(&store, "versioned");
        let valid = serde_json::to_value(&proof.receipt).unwrap();
        let mut unsupported = valid.clone();
        unsupported["v"] = serde_json::json!(2);
        unsupported["outcome"] = serde_json::json!("succeeded");
        let held = receipt_incoming(Some(&valid), Some(&unsupported));
        assert_eq!(held, receipt_incoming(Some(&unsupported), Some(&valid)));
        assert!(serde_json::from_value::<ExecutionReceipt>(held).is_err());
        store
            .lock()
            .unwrap()
            .execute(
                "UPDATE meta SET value=?2 WHERE key=?1",
                params![format!("app.{}", proof.key), unsupported.to_string()],
            )
            .unwrap();
        assert!(store.permission_history_ready().is_err());
        assert!(store.permission_reserve(&action("other"), &[AttributeKey::Recipients], 10).is_err());
    }
}
