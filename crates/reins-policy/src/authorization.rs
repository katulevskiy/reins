//! Deterministic ABAC composition. No IO, clock, authentication, execution, ML, signature checking or reservations.
//!
//! The runtime supplies the complete authenticated policy set and [`AuthorityBindings`] for the SAME immutable
//! action/records being evaluated. Wire claims alone cannot construct that trust. A permit is advisory: the runtime
//! must atomically reserve every budget in `authorization_path`, revalidate policy/membership leases, bind the exact
//! request digest to an authorization receipt, and enforce it at execution time. Review requirements here are not
//! considered satisfied; a separate receipt verifier must collect the exact reviewers and quorum.

use std::collections::{BTreeMap, BTreeSet};

use reins_proto::authorization::{
    ActionContext, AttributeKey, AttributeValue, Condition, Currency, EvidenceSource, Fact, MAX_AUTHORIZATION_POLICIES,
    PolicyEffect, PolicyLifecycle, PolicyRecord, ReviewRequirement,
};

/// Authenticated runtime facts, deliberately NOT serializable. Creating this is an authority boundary, not merely
/// a type conversion. Policy IDs attest the exact supplied records/revisions and issuer authority; verified keys
/// attest the exact supplied values and provenance, not a reusable key or an evidence ID claimed by an agent.
/// The runtime must include ALL applicable mandatory policies; omission cannot be detected by a pure evaluator.
#[derive(Clone, Debug)]
pub struct AuthorityBindings<'a> {
    pub principal_id: &'a str,
    pub tenant_id: Option<&'a str>,
    pub connection_id: &'a str,
    pub device_id: Option<&'a str>,
    pub executor_principal_id: &'a str,
    pub executor_device_id: &'a str,
    pub account_id: &'a str,
    pub resource_id: Option<&'a str>,
    pub request_digest: &'a str,
    /// Account discovery/access is a separate prerequisite; ABAC can only narrow it.
    pub account_access: bool,
    pub verified_attribute_keys: &'a [AttributeKey],
    pub authorized_policy_ids: &'a [String],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BaselineDecision {
    Deny,
    Review,
    Permit,
}

/// Usage is a snapshot for preflight only. Runtime reservations must cover the chosen path atomically, including
/// parent permissions. Reserved usage includes unresolved execution outcomes and cannot be reset to retry them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsageSnapshot {
    pub policy_id: String,
    pub revision: u64,
    pub committed_uses: u64,
    pub reserved_uses: u64,
    pub committed_cost_microunits: u64,
    pub reserved_cost_microunits: u64,
    pub cost_currency: Option<Currency>,
}

pub struct EvaluationInput<'a> {
    pub action: &'a ActionContext,
    pub policies: &'a [PolicyRecord],
    pub authority: &'a AuthorityBindings<'a>,
    pub now: i64,
    pub usage: &'a [UsageSnapshot],
    /// Existing grant/Auto result, still subordinate to mandatory policy. A matching explicit authorizing scope
    /// supersedes legacy permits even while paused/revoked/expired or outside its conditions and budget.
    pub baseline: BaselineDecision,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecisionOutcome {
    Permit,
    EligibleForAuto,
    Review,
    Deny,
    Hold,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct PolicyVersionRef {
    pub id: String,
    pub revision: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthorizationDecision {
    pub outcome: DecisionOutcome,
    pub request_digest: String,
    pub determining_policies: Vec<String>,
    pub required_reviews: Vec<ReviewRequirement>,
    /// One deterministic matching allow path, including all parent bounds. Empty for baseline permits.
    pub authorization_path: Vec<PolicyVersionRef>,
    pub reasons: Vec<DecisionReason>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct DecisionReason {
    pub code: ReasonCode,
    pub policy_id: Option<String>,
    pub attribute: Option<AttributeKey>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ReasonCode {
    InvalidAction,
    IdentityMismatch,
    AccountAccessRequired,
    InvalidPolicy,
    UnauthorizedPolicy,
    InvalidPolicyGraph,
    InvalidUsage,
    BaselineDenied,
    PolicyDenied,
    ReviewRequired,
    UnknownEvidence,
    UntrustedEvidence,
    StaleEvidence,
    InactivePermission,
    ConditionsNotMet,
    UsageUnavailable,
    LimitReached,
    ExplicitPermission,
    AutoEligible,
    LegacyPermission,
    NoPermission,
}

/// Evaluate a request against a complete authenticated policy snapshot. Deny wins; unresolved mandatory guards
/// hold; all review obligations accumulate; otherwise eligible permission sources are ORed while each delegation
/// and its ancestors are ANDed. Input/policy order never determines the result.
#[must_use]
pub fn evaluate(input: &EvaluationInput<'_>) -> AuthorizationDecision {
    evaluate_inner(input)
}

fn evaluate_inner(input: &EvaluationInput<'_>) -> AuthorizationDecision {
    let mut result = DecisionBuilder::default();
    if input.now < 0 || input.action.validate().is_err() {
        result.reason(ReasonCode::InvalidAction, None, None);
        return result.finish(input, DecisionOutcome::Hold, vec![]);
    }
    if !identity_matches(input) {
        result.reason(ReasonCode::IdentityMismatch, None, None);
        return result.finish(input, DecisionOutcome::Deny, vec![]);
    }
    if !input.authority.account_access {
        result.reason(ReasonCode::AccountAccessRequired, None, None);
        return result.finish(input, DecisionOutcome::Deny, vec![]);
    }
    if let Err(code) = validate_snapshot(input) {
        result.reason(code, None, None);
        return result.finish(input, DecisionOutcome::Hold, vec![]);
    }
    let policies: BTreeMap<_, _> = input.policies.iter().map(|p| (p.id.as_str(), p)).collect();
    let mut denied = input.baseline == BaselineDecision::Deny;
    if denied {
        result.reason(ReasonCode::BaselineDenied, None, None);
    }
    let mut hold = false;
    let mut supersedes_baseline = false;
    let mut permit_paths = vec![];
    let mut auto_paths = vec![];
    let mut memo = BTreeMap::new();

    for policy in policies.values() {
        if !ownership_matches(policy, input.action) {
            continue;
        }
        let scope = scope_matches(policy, input.action);
        if scope == Match::No {
            continue;
        }
        if matches!(policy.effect, PolicyEffect::Permit | PolicyEffect::EligibleForAuto) {
            supersedes_baseline = true;
            let permission = permission_status(policy, input, &policies, &mut memo);
            if permission.usable {
                if permission.auto {
                    auto_paths.push((policy.id.clone(), permission.path));
                } else {
                    permit_paths.push((policy.id.clone(), permission.path));
                }
            } else {
                result.reasons.extend(permission.reasons);
            }
            continue;
        }
        if policy.effect == PolicyEffect::DelegationAuthority || !active(policy, input.now) {
            continue;
        }
        let (conditions, reasons) = conditions_match(policy, input);
        let matched = scope.and(conditions);
        if matched == Match::No {
            continue;
        }
        result.policies.insert(policy.id.clone());
        if matched == Match::Unknown {
            hold = true;
            result.reasons.extend(reasons);
            if scope == Match::Unknown {
                result.reason(ReasonCode::UnknownEvidence, Some(&policy.id), None);
            }
            continue;
        }
        match &policy.effect {
            PolicyEffect::Deny => {
                denied = true;
                result.reason(ReasonCode::PolicyDenied, Some(&policy.id), None);
            }
            PolicyEffect::RequireReview {
                requirements,
            } => {
                result.reviews.extend(requirements.iter().cloned());
                result.reason(ReasonCode::ReviewRequired, Some(&policy.id), None);
            }
            _ => unreachable!("authorizing effects handled above"),
        }
    }

    if denied {
        return result.finish(input, DecisionOutcome::Deny, vec![]);
    }
    if hold {
        return result.finish(input, DecisionOutcome::Hold, vec![]);
    }
    if !result.reviews.is_empty() {
        return result.finish(input, DecisionOutcome::Review, vec![]);
    }
    // BTreeMap iteration makes both chosen authorization path and diagnostics independent of policy order.
    if let Some((id, path)) = permit_paths.into_iter().next() {
        result.reason(ReasonCode::ExplicitPermission, Some(&id), None);
        return result.finish(input, DecisionOutcome::Permit, path);
    }
    if input.baseline == BaselineDecision::Permit && !supersedes_baseline {
        result.reason(ReasonCode::LegacyPermission, None, None);
        return result.finish(input, DecisionOutcome::Permit, vec![]);
    }
    if let Some((id, path)) = auto_paths.into_iter().next() {
        result.reason(ReasonCode::AutoEligible, Some(&id), None);
        return result.finish(input, DecisionOutcome::EligibleForAuto, path);
    }
    result.reason(ReasonCode::NoPermission, None, None);
    result.finish(input, DecisionOutcome::Review, vec![])
}

#[derive(Default)]
struct DecisionBuilder {
    policies: BTreeSet<String>,
    reviews: BTreeSet<ReviewRequirement>,
    reasons: BTreeSet<DecisionReason>,
}

impl DecisionBuilder {
    fn reason(&mut self, code: ReasonCode, policy_id: Option<&str>, attribute: Option<AttributeKey>) {
        self.reasons.insert(reason(code, policy_id, attribute));
    }

    fn finish(
        mut self,
        input: &EvaluationInput<'_>,
        outcome: DecisionOutcome,
        path: Vec<PolicyVersionRef>,
    ) -> AuthorizationDecision {
        self.policies.extend(path.iter().map(|p| p.id.clone()));
        self.policies.extend(self.reasons.iter().filter_map(|r| r.policy_id.clone()));
        AuthorizationDecision {
            outcome,
            request_digest: if input.action.request_digest.len() == 64 {
                input.action.request_digest.clone()
            } else {
                String::new()
            },
            determining_policies: self.policies.into_iter().collect(),
            required_reviews: self.reviews.into_iter().collect(),
            authorization_path: path,
            reasons: self.reasons.into_iter().collect(),
        }
    }
}

fn reason(code: ReasonCode, policy_id: Option<&str>, attribute: Option<AttributeKey>) -> DecisionReason {
    DecisionReason {
        code,
        policy_id: policy_id.map(str::to_owned),
        attribute,
    }
}

fn identity_matches(input: &EvaluationInput<'_>) -> bool {
    let a = &input.action.actor;
    let b = input.authority;
    a.principal_id == b.principal_id
        && a.tenant_id.as_deref() == b.tenant_id
        && a.connection_id == b.connection_id
        && a.device_id.as_deref() == b.device_id
        && a.executor_principal_id == b.executor_principal_id
        && a.executor_device_id == b.executor_device_id
        && input.action.resource.account_id == b.account_id
        && input.action.resource.resource_id.as_deref() == b.resource_id
        && input.action.request_digest == b.request_digest
}

fn validate_snapshot(input: &EvaluationInput<'_>) -> Result<(), ReasonCode> {
    if input.policies.len() > MAX_AUTHORIZATION_POLICIES {
        return Err(ReasonCode::InvalidPolicy);
    }
    let mut policies = BTreeMap::new();
    for policy in input.policies {
        if policy.validate().is_err() || policies.insert(policy.id.as_str(), policy).is_some() {
            return Err(ReasonCode::InvalidPolicy);
        }
        if !input.authority.authorized_policy_ids.contains(&policy.id) {
            return Err(ReasonCode::UnauthorizedPolicy);
        }
    }
    let mut visited = BTreeSet::new();
    let mut visiting = BTreeSet::new();
    for policy in input.policies {
        validate_ancestors(policy, &policies, &mut visited, &mut visiting)?;
    }
    if input.usage.len() > MAX_AUTHORIZATION_POLICIES {
        return Err(ReasonCode::InvalidUsage);
    }
    let mut usages = BTreeSet::new();
    for usage in input.usage {
        if !usages.insert(&usage.policy_id) || usage.revision == 0 {
            return Err(ReasonCode::InvalidUsage);
        }
    }
    Ok(())
}

fn validate_ancestors<'a>(
    policy: &'a PolicyRecord,
    policies: &BTreeMap<&str, &'a PolicyRecord>,
    visited: &mut BTreeSet<&'a str>,
    visiting: &mut BTreeSet<&'a str>,
) -> Result<(), ReasonCode> {
    if visited.contains(policy.id.as_str()) {
        return Ok(());
    }
    if !visiting.insert(&policy.id) {
        return Err(ReasonCode::InvalidPolicyGraph);
    }
    for id in &policy.parent_policy_ids {
        let parent = policies.get(id.as_str()).ok_or(ReasonCode::InvalidPolicyGraph)?;
        if parent.issuer.tenant_id != policy.issuer.tenant_id
            || (parent.issuer.tenant_id.is_none() && parent.issuer.principal_id != policy.issuer.principal_id)
            || !matches!(
                parent.effect,
                PolicyEffect::Permit | PolicyEffect::EligibleForAuto | PolicyEffect::DelegationAuthority
            )
        {
            return Err(ReasonCode::InvalidPolicyGraph);
        }
        validate_ancestors(parent, policies, visited, visiting)?;
    }
    visiting.remove(policy.id.as_str());
    visited.insert(&policy.id);
    Ok(())
}

fn ownership_matches(policy: &PolicyRecord, action: &ActionContext) -> bool {
    policy.issuer.tenant_id == action.actor.tenant_id
        && (policy.issuer.tenant_id.is_some() || policy.issuer.principal_id == action.actor.executor_principal_id)
}

fn active(policy: &PolicyRecord, now: i64) -> bool {
    policy.lifecycle == PolicyLifecycle::Active && policy.valid_from <= now && policy.expires_at.is_none_or(|t| now < t)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Match {
    Yes,
    No,
    Unknown,
}

impl Match {
    const fn and(self, other: Self) -> Self {
        match (self, other) {
            (Self::No, _) | (_, Self::No) => Self::No,
            (Self::Unknown, _) | (_, Self::Unknown) => Self::Unknown,
            _ => Self::Yes,
        }
    }
}

fn scope_matches(policy: &PolicyRecord, action: &ActionContext) -> Match {
    let scope = &policy.scope;
    let mut matched = Match::Yes;
    for (ids, actual) in [
        (&scope.principal_ids, Some(action.actor.principal_id.as_str())),
        (&scope.connection_ids, Some(action.actor.connection_id.as_str())),
        (&scope.device_ids, action.actor.device_id.as_deref()),
        (&scope.account_ids, Some(action.resource.account_id.as_str())),
        (&scope.resource_ids, action.resource.resource_id.as_deref()),
    ] {
        if ids.is_empty() {
            continue;
        }
        matched = matched.and(match actual {
            None => Match::Unknown,
            Some(id) if ids.iter().any(|i| i == id) => Match::Yes,
            Some(_) => Match::No,
        });
    }
    if !scope.operations.is_empty() && !scope.operations.contains(&action.operation) {
        return Match::No;
    }
    matched
}

fn trusted_value<'a>(key: AttributeKey, input: &'a EvaluationInput<'_>) -> Result<&'a AttributeValue, ReasonCode> {
    let fact = input.action.facts.iter().find(|f| f.key == key).ok_or(ReasonCode::UnknownEvidence)?;
    let Fact::Known {
        value,
        evidence,
    } = &fact.fact
    else {
        return Err(ReasonCode::UnknownEvidence);
    };
    if evidence.source == EvidenceSource::Agent || !input.authority.verified_attribute_keys.contains(&key) {
        return Err(ReasonCode::UntrustedEvidence);
    }
    if evidence.observed_at > input.now || input.now >= evidence.valid_until {
        return Err(ReasonCode::StaleEvidence);
    }
    Ok(value)
}

fn conditions_match(policy: &PolicyRecord, input: &EvaluationInput<'_>) -> (Match, BTreeSet<DecisionReason>) {
    let mut matched = Match::Yes;
    let mut reasons = BTreeSet::new();
    for condition in &policy.conditions {
        let key = condition.key();
        let value = match trusted_value(key, input) {
            Ok(value) => value,
            Err(code) => {
                matched = matched.and(Match::Unknown);
                reasons.insert(reason(code, Some(&policy.id), Some(key)));
                continue;
            }
        };
        let applies = match (condition, value) {
            (
                Condition::StringEquals {
                    value: expected,
                    ..
                },
                AttributeValue::String(actual),
            ) => actual == expected,
            (
                Condition::StringIn {
                    values,
                    ..
                },
                AttributeValue::String(actual),
            ) => values.contains(actual),
            (
                Condition::SetWithin {
                    values,
                    require_nonempty,
                    ..
                },
                AttributeValue::StringSet(actual),
            ) => (!require_nonempty || !actual.is_empty()) && actual.iter().all(|v| values.contains(v)),
            (
                Condition::BoolEquals {
                    value: expected,
                    ..
                },
                AttributeValue::Bool(actual),
            ) => actual == expected,
            (
                Condition::U64AtMost {
                    value: maximum,
                    ..
                },
                AttributeValue::U64(actual),
            ) => actual <= maximum,
            _ => false, // Input validation rejects type mismatches; never coerce if that boundary changes.
        };
        if !applies {
            matched = Match::No;
        }
    }
    (matched, reasons)
}

#[derive(Clone, Default)]
struct PermissionStatus {
    usable: bool,
    auto: bool,
    path: Vec<PolicyVersionRef>,
    reasons: BTreeSet<DecisionReason>,
}

fn permission_status(
    policy: &PolicyRecord,
    input: &EvaluationInput<'_>,
    policies: &BTreeMap<&str, &PolicyRecord>,
    memo: &mut BTreeMap<String, PermissionStatus>,
) -> PermissionStatus {
    if let Some(status) = memo.get(&policy.id) {
        return status.clone();
    }
    let mut status = PermissionStatus {
        usable: true,
        auto: policy.effect == PolicyEffect::EligibleForAuto,
        ..PermissionStatus::default()
    };
    status.path.push(PolicyVersionRef {
        id: policy.id.clone(),
        revision: policy.revision,
    });
    let scope = scope_matches(policy, input.action);
    if !ownership_matches(policy, input.action) || scope != Match::Yes {
        status.usable = false;
        status.reasons.insert(reason(
            if scope == Match::Unknown {
                ReasonCode::UnknownEvidence
            } else {
                ReasonCode::ConditionsNotMet
            },
            Some(&policy.id),
            None,
        ));
    }
    if !active(policy, input.now) {
        status.usable = false;
        status.reasons.insert(reason(ReasonCode::InactivePermission, Some(&policy.id), None));
    }
    let (conditions, reasons) = conditions_match(policy, input);
    if conditions != Match::Yes {
        status.usable = false;
        status.reasons.extend(reasons);
        if conditions == Match::No {
            status.reasons.insert(reason(ReasonCode::ConditionsNotMet, Some(&policy.id), None));
        }
    }
    if let Err(code) = limits_allow(policy, input) {
        status.usable = false;
        status.reasons.insert(reason(code, Some(&policy.id), None));
    }
    for parent_id in &policy.parent_policy_ids {
        // Graph was validated as a bounded DAG before any permission evaluation.
        let parent = permission_status(policies[parent_id.as_str()], input, policies, memo);
        status.usable &= parent.usable;
        status.auto |= parent.auto;
        status.path.extend(parent.path);
        status.reasons.extend(parent.reasons);
    }
    status.path.sort();
    status.path.dedup();
    memo.insert(policy.id.clone(), status.clone());
    status
}

fn limits_allow(policy: &PolicyRecord, input: &EvaluationInput<'_>) -> Result<(), ReasonCode> {
    if policy.limits.max_uses.is_none() && policy.limits.max_cost_microunits.is_none() {
        return Ok(());
    }
    let usage = input
        .usage
        .iter()
        .find(|u| u.policy_id == policy.id && u.revision == policy.revision)
        .ok_or(ReasonCode::UsageUnavailable)?;
    if let Some(limit) = policy.limits.max_uses {
        let uses = usage.committed_uses.checked_add(usage.reserved_uses).ok_or(ReasonCode::LimitReached)?;
        if uses >= limit {
            return Err(ReasonCode::LimitReached);
        }
    }
    if let Some(limit) = policy.limits.max_cost_microunits {
        let AttributeValue::U64(cost) = trusted_value(AttributeKey::CostMicrounits, input)? else {
            return Err(ReasonCode::UnknownEvidence);
        };
        let AttributeValue::Currency(currency) = trusted_value(AttributeKey::CostCurrency, input)? else {
            return Err(ReasonCode::UnknownEvidence);
        };
        if Some(*currency) != policy.limits.cost_currency || usage.cost_currency != policy.limits.cost_currency {
            return Err(ReasonCode::UsageUnavailable);
        }
        let total = usage
            .committed_cost_microunits
            .checked_add(usage.reserved_cost_microunits)
            .and_then(|spent| spent.checked_add(*cost))
            .ok_or(ReasonCode::LimitReached)?;
        if total > limit {
            return Err(ReasonCode::LimitReached);
        }
    }
    Ok(())
}
