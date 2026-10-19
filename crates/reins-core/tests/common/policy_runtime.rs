//! Helpers for real mandatory runtime integration regressions.
#![allow(dead_code)]
use reins_core::{Engine, ForeignError, OwnerCheck};
use reins_proto::authorization::{
    OperationId, PolicyEffect, PolicyIssuer, PolicyKind, PolicyLifecycle, PolicyRecord, PolicyScope, ReviewRequirement,
    UsageLimits,
};

pub struct Owner;
#[async_trait::async_trait]
impl OwnerCheck for Owner {
    async fn confirm(&self, _: String) -> Result<bool, ForeignError> {
        Ok(true)
    }
}

pub fn review(role: &str) -> PolicyEffect {
    PolicyEffect::RequireReview {
        requirements: vec![ReviewRequirement {
            role_id: role.to_owned(),
            quorum: 1,
            distinct_humans_group: None,
            exclude_requester: false,
        }],
    }
}

pub fn policy(engine: &Engine, service: &str, capability: &str, action: &str, effect: PolicyEffect) -> PolicyRecord {
    PolicyRecord {
        v: 1,
        id: "required-review".to_owned(),
        revision: 1,
        issuer: PolicyIssuer {
            principal_id: engine.authorization_policies().unwrap().owner,
            tenant_id: None,
        },
        kind: PolicyKind::Mandatory,
        lifecycle: PolicyLifecycle::Active,
        valid_from: 0,
        expires_at: None,
        scope: PolicyScope {
            operations: vec![OperationId {
                service: service.to_owned(),
                capability: capability.to_owned(),
                action: action.to_owned(),
            }],
            ..PolicyScope::default()
        },
        conditions: vec![],
        effect,
        parent_policy_ids: vec![],
        limits: UsageLimits::default(),
    }
}
