//! Versioned, bounded authorization data. These are claims, never authentication.
//!
//! A runtime must bind identities and evidence to authenticated connector/device/organization state before handing
//! them to the policy evaluator. Deserializing `source: "organization"` does not establish that authority. Exact
//! request digests stay separate from operation families: permission to one payload never authorizes another.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::{ValidationError, validate::invalid};

pub const AUTHORIZATION_VERSION: u32 = 1;
pub const MAX_AUTHORIZATION_BYTES: usize = 64 * 1024;
pub const MAX_AUTHORIZATION_POLICIES: usize = 256;
pub const MAX_AUTHORIZATION_ITEMS: usize = 128;
pub const MAX_AUTHORIZATION_TEXT: usize = 2048;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionContext {
    pub v: u32,
    pub request_id: String,
    /// Lowercase SHA-256 of the original, complete invocation; supplied/checked by the authenticated runtime.
    pub request_digest: String,
    pub actor: ActorContext,
    pub operation: OperationId,
    pub resource: ResourceContext,
    pub facts: Vec<AttributeFact>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActorContext {
    /// Human beneficiary on whose behalf the agent acts; distinct from the credential-owning executor.
    pub principal_id: String,
    /// `None` is a personal account, not a wildcard matching organizations.
    pub tenant_id: Option<String>,
    pub connection_id: String,
    /// Paired requesting device/computer.
    pub device_id: Option<String>,
    pub executor_principal_id: String,
    pub executor_device_id: String,
}

/// An exact operation family. No prefix, regex or wildcard matching, including for newly introduced capabilities.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationId {
    pub service: String,
    pub capability: String,
    pub action: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceContext {
    /// Stable connector account ID, not a display name or agent assertion.
    pub account_id: String,
    pub resource_id: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttributeKey {
    Repository,
    Branch,
    Recipients,
    Environment,
    Session,
    CredentialIds,
    ArtifactDigest,
    Workspace,
    NetworkDestinations,
    NetworkRestricted,
    CostMicrounits,
    CostCurrency,
}

/// Supported monetary units for v1 budgets; unknown units are rejected, and there is no implicit FX conversion.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Currency {
    Usd,
    Eur,
    Gbp,
    Cad,
    Aud,
    Chf,
    Jpy,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case", deny_unknown_fields)]
pub enum AttributeValue {
    String(String),
    StringSet(Vec<String>),
    Bool(bool),
    U64(u64),
    Currency(Currency),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttributeFact {
    pub key: AttributeKey,
    pub fact: Fact,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum Fact {
    Known {
        value: AttributeValue,
        evidence: Evidence,
    },
    Unknown {
        reason: UnknownReason,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnknownReason {
    Missing,
    Unresolved,
    Unavailable,
    Untrusted,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    pub id: String,
    pub source: EvidenceSource,
    pub observed_at: i64,
    /// Evidence is stale at this instant. A runtime must apply an appropriate maximum lease when collecting it.
    pub valid_until: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceSource {
    Connector,
    PairedDevice,
    LocalCore,
    Organization,
    Agent,
}

/// Persisted via authenticated owner/organization updates. `revision` is a CAS version, not proof of authority.
/// Revoked records are tombstones and must not be removed merely because they do not currently grant anything.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyRecord {
    pub v: u32,
    pub id: String,
    pub revision: u64,
    pub issuer: PolicyIssuer,
    pub kind: PolicyKind,
    pub lifecycle: PolicyLifecycle,
    pub valid_from: i64,
    pub expires_at: Option<i64>,
    pub scope: PolicyScope,
    pub conditions: Vec<Condition>,
    pub effect: PolicyEffect,
    /// Every referenced permission remains a conjunctive upper bound on a delegation's effective authority.
    pub parent_policy_ids: Vec<String>,
    pub limits: UsageLimits,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyIssuer {
    pub principal_id: String,
    pub tenant_id: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyKind {
    Mandatory,
    AccountDefault,
    Permission,
    Delegation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyLifecycle {
    Active,
    Paused,
    Revoked,
}

/// Empty lists mean any identity in the policy's authenticated ownership boundary. Authorizing effects MUST
/// enumerate connections, accounts and operations; a newly paired connection/capability never inherits authority.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyScope {
    pub principal_ids: Vec<String>,
    pub connection_ids: Vec<String>,
    pub device_ids: Vec<String>,
    pub account_ids: Vec<String>,
    pub resource_ids: Vec<String>,
    pub operations: Vec<OperationId>,
}

/// A bounded conjunction; no executable expressions, regex, scripts, shell evaluation or implicit coercion.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "operator", rename_all = "snake_case", deny_unknown_fields)]
pub enum Condition {
    StringEquals {
        key: AttributeKey,
        value: String,
    },
    StringIn {
        key: AttributeKey,
        values: Vec<String>,
    },
    SetWithin {
        key: AttributeKey,
        values: Vec<String>,
        require_nonempty: bool,
    },
    BoolEquals {
        key: AttributeKey,
        value: bool,
    },
    U64AtMost {
        key: AttributeKey,
        value: u64,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum PolicyEffect {
    Deny,
    RequireReview {
        requirements: Vec<ReviewRequirement>,
    },
    Permit,
    EligibleForAuto,
    /// An upper bound available for issuing delegations. It grants no execution authority by itself.
    DelegationAuthority,
}

/// Structural review requirements only. This module does not verify signatures, memberships or approval receipts.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewRequirement {
    /// Tenant-scoped role. Personal owner review uses the reserved `owner` role.
    pub role_id: String,
    pub quorum: u32,
    /// Requirements in the same nonempty group must be satisfied by distinct people, even with multiple roles.
    pub distinct_humans_group: Option<String>,
    pub exclude_requester: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsageLimits {
    pub max_uses: Option<u64>,
    pub max_cost_microunits: Option<u64>,
    pub cost_currency: Option<Currency>,
}

impl ActionContext {
    pub fn validate(&self) -> Result<(), ValidationError> {
        version(self.v)?;
        text("request_id", &self.request_id)?;
        if self.request_digest.len() != 64
            || !self.request_digest.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(invalid("request_digest", "expected lowercase SHA-256 hex"));
        }
        for id in [
            &self.actor.principal_id,
            &self.actor.connection_id,
            &self.actor.executor_principal_id,
            &self.actor.executor_device_id,
            &self.resource.account_id,
        ] {
            text("identity", id)?;
        }
        optional_text("tenant_id", self.actor.tenant_id.as_deref())?;
        optional_text("device_id", self.actor.device_id.as_deref())?;
        optional_text("resource_id", self.resource.resource_id.as_deref())?;
        self.operation.validate()?;
        count("facts", self.facts.len())?;
        let mut keys = BTreeSet::new();
        for fact in &self.facts {
            if !keys.insert(fact.key) {
                return Err(invalid("facts", "duplicate attribute"));
            }
            if let Fact::Known {
                value,
                evidence,
            } = &fact.fact
            {
                value.validate(fact.key)?;
                text("evidence.id", &evidence.id)?;
                if evidence.observed_at < 0 || evidence.valid_until <= evidence.observed_at {
                    return Err(invalid("evidence", "invalid observation interval"));
                }
            }
        }
        bounded_encoding(self)
    }

    pub fn from_json(bytes: &[u8]) -> Result<Self, ValidationError> {
        let value: Self = bounded_decode(bytes)?;
        value.validate()?;
        Ok(value)
    }
}

impl OperationId {
    pub fn validate(&self) -> Result<(), ValidationError> {
        for part in [&self.service, &self.capability, &self.action] {
            text("operation", part)?;
            if part.len() > 128 || !part.bytes().all(|b| b.is_ascii_alphanumeric() || b"._:-".contains(&b)) {
                return Err(invalid("operation", "expected exact ASCII identifier"));
            }
        }
        Ok(())
    }
}

impl PolicyRecord {
    pub fn validate(&self) -> Result<(), ValidationError> {
        version(self.v)?;
        text("policy.id", &self.id)?;
        text("policy.issuer", &self.issuer.principal_id)?;
        optional_text("policy.tenant", self.issuer.tenant_id.as_deref())?;
        if self.revision == 0 || self.valid_from < 0 || self.expires_at.is_some_and(|t| t <= self.valid_from) {
            return Err(invalid("policy", "invalid revision or validity interval"));
        }
        for ids in [
            &self.scope.principal_ids,
            &self.scope.connection_ids,
            &self.scope.device_ids,
            &self.scope.account_ids,
            &self.scope.resource_ids,
            &self.parent_policy_ids,
        ] {
            strings("policy.scope", ids)?;
        }
        count("policy.operations", self.scope.operations.len())?;
        unique("policy.operations", &self.scope.operations)?;
        for operation in &self.scope.operations {
            operation.validate()?;
        }
        count("policy.conditions", self.conditions.len())?;
        for condition in &self.conditions {
            condition.validate()?;
        }
        if matches!(
            self.effect,
            PolicyEffect::Permit | PolicyEffect::EligibleForAuto | PolicyEffect::DelegationAuthority
        ) && (self.scope.connection_ids.is_empty()
            || self.scope.account_ids.is_empty()
            || self.scope.operations.is_empty())
        {
            return Err(invalid(
                "policy.scope",
                "authorizing effects require explicit connections, accounts and operations",
            ));
        }
        if self.kind == PolicyKind::Mandatory
            && matches!(
                self.effect,
                PolicyEffect::Permit | PolicyEffect::EligibleForAuto | PolicyEffect::DelegationAuthority
            )
        {
            return Err(invalid("policy.kind", "mandatory rules may only deny or require review"));
        }
        if self.effect == PolicyEffect::DelegationAuthority && self.kind != PolicyKind::Permission {
            return Err(invalid("policy.effect", "delegation authority must be a permission"));
        }
        if self.kind == PolicyKind::Delegation {
            if self.parent_policy_ids.is_empty()
                || self.expires_at.is_none()
                || !matches!(self.effect, PolicyEffect::Permit | PolicyEffect::EligibleForAuto)
            {
                return Err(invalid("policy.delegation", "requires parent authority, expiry and authorizing effect"));
            }
        } else if !self.parent_policy_ids.is_empty() {
            return Err(invalid("policy.parents", "only delegations have parents"));
        }
        if self.parent_policy_ids.contains(&self.id) {
            return Err(invalid("policy.parents", "self-reference"));
        }
        if let PolicyEffect::RequireReview {
            requirements,
        } = &self.effect
        {
            count("policy.review", requirements.len())?;
            if requirements.is_empty() {
                return Err(invalid("policy.review", "at least one reviewer is required"));
            }
            unique("policy.review", requirements)?;
            for requirement in requirements {
                text("policy.review.role", &requirement.role_id)?;
                optional_text("policy.review.group", requirement.distinct_humans_group.as_deref())?;
                if requirement.quorum == 0 || requirement.quorum > 128 {
                    return Err(invalid("policy.review.quorum", "expected 1..=128"));
                }
            }
        }
        if self.limits.max_uses == Some(0) || self.limits.max_cost_microunits == Some(0) {
            return Err(invalid("policy.limits", "limits must be positive"));
        }
        if self.limits.max_cost_microunits.is_some() != self.limits.cost_currency.is_some() {
            return Err(invalid("policy.limits", "monetary limits require an explicit matching currency"));
        }
        if !matches!(
            self.effect,
            PolicyEffect::Permit | PolicyEffect::EligibleForAuto | PolicyEffect::DelegationAuthority
        ) && self.limits != UsageLimits::default()
        {
            return Err(invalid("policy.limits", "only authorizing effects consume budgets"));
        }
        bounded_encoding(self)
    }

    pub fn from_json(bytes: &[u8]) -> Result<Self, ValidationError> {
        let value: Self = bounded_decode(bytes)?;
        value.validate()?;
        Ok(value)
    }
}

impl Condition {
    #[must_use]
    pub const fn key(&self) -> AttributeKey {
        match self {
            Self::StringEquals {
                key,
                ..
            }
            | Self::StringIn {
                key,
                ..
            }
            | Self::SetWithin {
                key,
                ..
            }
            | Self::BoolEquals {
                key,
                ..
            }
            | Self::U64AtMost {
                key,
                ..
            } => *key,
        }
    }

    fn validate(&self) -> Result<(), ValidationError> {
        match self {
            Self::StringEquals {
                key,
                value,
            } => AttributeValue::String(value.clone()).validate(*key),
            Self::StringIn {
                key,
                values,
            } => {
                strings("condition.values", values)?;
                if values.is_empty() || !key.is_string() {
                    return Err(invalid("condition", "expected a nonempty string selection"));
                }
                Ok(())
            }
            Self::SetWithin {
                key,
                values,
                require_nonempty,
            } => {
                if *key == AttributeKey::Recipients && !require_nonempty {
                    return Err(invalid("condition", "recipient authorization requires at least one recipient"));
                }
                AttributeValue::StringSet(values.clone()).validate(*key)
            }
            Self::BoolEquals {
                key,
                value,
            } => AttributeValue::Bool(*value).validate(*key),
            Self::U64AtMost {
                key,
                value,
            } => AttributeValue::U64(*value).validate(*key),
        }
    }
}

impl AttributeKey {
    const fn is_string(self) -> bool {
        matches!(
            self,
            Self::Repository
                | Self::Branch
                | Self::Environment
                | Self::Session
                | Self::ArtifactDigest
                | Self::Workspace
        )
    }

    const fn is_set(self) -> bool {
        matches!(self, Self::Recipients | Self::CredentialIds | Self::NetworkDestinations)
    }
}

impl AttributeValue {
    fn validate(&self, key: AttributeKey) -> Result<(), ValidationError> {
        match self {
            Self::String(value) if key.is_string() => text("attribute.value", value),
            Self::StringSet(values) if key.is_set() => strings("attribute.values", values),
            Self::Bool(_) if key == AttributeKey::NetworkRestricted => Ok(()),
            Self::U64(_) if key == AttributeKey::CostMicrounits => Ok(()),
            Self::Currency(_) if key == AttributeKey::CostCurrency => Ok(()),
            _ => Err(invalid("attribute", "type does not match attribute key")),
        }
    }
}

fn version(v: u32) -> Result<(), ValidationError> {
    if v == AUTHORIZATION_VERSION {
        Ok(())
    } else {
        Err(invalid("v", "unsupported authorization version"))
    }
}

fn text(field: &'static str, value: &str) -> Result<(), ValidationError> {
    if value.is_empty() || value.len() > MAX_AUTHORIZATION_TEXT || value.chars().any(char::is_control) {
        Err(invalid(field, "expected bounded, nonempty text without control characters"))
    } else {
        Ok(())
    }
}

fn optional_text(field: &'static str, value: Option<&str>) -> Result<(), ValidationError> {
    value.map_or(Ok(()), |v| text(field, v))
}

fn count(field: &'static str, value: usize) -> Result<(), ValidationError> {
    if value <= MAX_AUTHORIZATION_ITEMS {
        Ok(())
    } else {
        Err(invalid(field, "too many items"))
    }
}

fn unique<T: Ord>(field: &'static str, values: &[T]) -> Result<(), ValidationError> {
    if values.iter().collect::<BTreeSet<_>>().len() == values.len() {
        Ok(())
    } else {
        Err(invalid(field, "duplicate item"))
    }
}

fn strings(field: &'static str, values: &[String]) -> Result<(), ValidationError> {
    count(field, values.len())?;
    unique(field, values)?;
    for value in values {
        text(field, value)?;
    }
    Ok(())
}

fn bounded_encoding(value: &impl Serialize) -> Result<(), ValidationError> {
    struct BoundedWriter(usize);
    impl std::io::Write for BoundedWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > MAX_AUTHORIZATION_BYTES.saturating_sub(self.0) {
                return Err(std::io::Error::other("authorization size limit"));
            }
            self.0 += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    serde_json::to_writer(BoundedWriter(0), value).map_err(|_| invalid("authorization", "encoded size limit exceeded"))
}

fn bounded_decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, ValidationError> {
    if bytes.len() > MAX_AUTHORIZATION_BYTES {
        return Err(invalid("authorization", "encoded size limit exceeded"));
    }
    serde_json::from_slice(bytes).map_err(|_| invalid("authorization", "invalid authorization JSON"))
}
