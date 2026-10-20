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

/// Force an authenticated operation to suspend in its 401 token refresh. Its first invocation was authorized;
/// any retry must independently recheck the policy installed while this token request is delayed.
pub async fn suspend_refresh(
    server: &wiremock::MockServer,
    method: &str,
    endpoint: &str,
) -> std::sync::Arc<tokio::sync::Notify> {
    use std::sync::Arc;
    use std::time::Duration;
    use wiremock::matchers::{method as verb, path};
    use wiremock::{Mock, ResponseTemplate};
    Mock::given(verb(method))
        .and(path(endpoint))
        .respond_with(ResponseTemplate::new(401))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(server)
        .await;
    let entered = Arc::new(tokio::sync::Notify::new());
    let signal = Arc::clone(&entered);
    Mock::given(verb("POST"))
        .and(path("/identity/connect/token"))
        .respond_with(move |_: &wiremock::Request| {
            signal.notify_one();
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({
                    "access_token": crate::common::account_token(), "refresh_token": "REFRESH", "expires_in": 7200
                }))
                .set_delay(Duration::from_millis(600))
        })
        .with_priority(1)
        .mount(server)
        .await;
    entered
}
