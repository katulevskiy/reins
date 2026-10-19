//! The common mandatory authorization gate. Every mediated request is checked before provider dispatch; legacy
//! grants, Autopilot and manual approval recheck the same snapshot at their effect/disclosure boundary. This first
//! slice activates restrictions only. Explicit permits/delegations require execution receipts and atomic budget
//! reservations and are visibly rejected until that support is installed.

use reins_policy::authorization::{
    self, AuthorityBindings, AuthorizationDecision, BaselineDecision, DecisionOutcome, EvaluationInput,
};
use reins_proto::authorization::{
    ActionContext, ActorContext, AttributeFact, AttributeKey, AttributeValue, Evidence, EvidenceSource, Fact,
    OperationId, PolicyEffect, PolicyLifecycle, PolicyRecord, ResourceContext,
};
use reins_proto::gmail::ToolCall;
use reins_proto::relay::RelayRequest;

use crate::{
    CoreError, OwnerCheck,
    engine::Engine,
    store::{AuthorizationState, unix_now},
};

/// A local tap on the current deciding phone is the only supported reviewer proof in this first slice. External
/// roles/quorums are stored and held; they are never satisfied by treating an ordinary owner tap as all reviewers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AuthorizationSource {
    Automatic,
    OwnerReview,
}

/// A local normalization result, deliberately not serializable and not an account-access or execution proof.
pub(crate) struct NormalizedAuthorizationAction {
    pub action: ActionContext,
    pub verified_attribute_keys: Vec<AttributeKey>,
}

impl Engine {
    pub(crate) fn policy_owner(&self) -> Result<String, CoreError> {
        if let Some(owner) = self.store.bound_owner() {
            return Ok(owner.user_id);
        }
        let session = self.session()?;
        self.store
            .meta_get(&crate::session::account_id_key(&session.server, &session.email()))?
            .filter(|id| !id.is_empty())
            .ok_or_else(|| CoreError::invalid("the account identity could not be verified"))
    }

    pub fn authorization_policies(&self) -> Result<AuthorizationState, CoreError> {
        let owner = self.policy_owner()?;
        let state = self.store.authorization_state()?.unwrap_or_else(|| AuthorizationState::empty(&owner));
        if state.owner != owner {
            return Err(CoreError::invalid("authorization belongs to another account"));
        }
        Ok(state)
    }

    /// Authenticated owner CAS, not an agent tool. This API cannot grant account visibility or activate authority
    /// the current execution adapters do not enforce. The caller can store external review requirements; requests
    /// matching them remain held until the reviewer proof implementation is available.
    pub async fn save_authorization_policy(
        &self,
        expected_revision: u64,
        policy: PolicyRecord,
        owner: &dyn OwnerCheck,
    ) -> Result<AuthorizationState, CoreError> {
        let session = self.session()?;
        if !owner
            .confirm("Save the account's authorization policy".to_owned())
            .await
            .map_err(|_| CoreError::invalid("the owner check failed"))?
        {
            return Err(CoreError::invalid("the owner did not confirm the authorization policy"));
        }
        self.require_decider_fresh(&session).await?;
        let mut state = self.authorization_policies()?;
        if state.conflicted {
            return Err(CoreError::invalid("resolve the authorization conflict before editing policies"));
        }
        validate_activation(&policy, &state.owner)?;
        if state.revoked_ids.contains(&policy.id) && policy.lifecycle != PolicyLifecycle::Revoked {
            return Err(CoreError::invalid("a revoked policy cannot be resumed; create a new identity"));
        }
        if policy.lifecycle == PolicyLifecycle::Revoked {
            state.revoked_ids.insert(policy.id.clone());
        }
        if let Some(current) = state.policies.iter_mut().find(|p| p.id == policy.id) {
            *current = policy;
        } else {
            if policy.revision != 1 {
                return Err(CoreError::invalid("a new policy starts at revision one"));
            }
            state.policies.push(policy);
        }
        state.revision =
            expected_revision.checked_add(1).ok_or_else(|| CoreError::invalid("policy revision exhausted"))?;
        self.store.authorization_replace(expected_revision, &state)?;
        Ok(state)
    }

    /// Resolves all concurrent versions after the owner inspects them. The caller supplies the complete chosen
    /// policy set with explicit revision increments for changed records; immutable IDs/revocations still apply.
    pub async fn resolve_authorization_conflict(
        &self,
        expected_revision: u64,
        policies: Vec<PolicyRecord>,
        owner: &dyn OwnerCheck,
    ) -> Result<AuthorizationState, CoreError> {
        let session = self.session()?;
        self.require_decider()?;
        let mut state = self.authorization_policies()?;
        if !state.conflicted {
            return Err(CoreError::invalid("there is no authorization conflict to resolve"));
        }
        for policy in &policies {
            validate_activation(policy, &state.owner)?;
        }
        if !owner
            .confirm("Resolve the account's conflicting authorization policies".to_owned())
            .await
            .map_err(|_| CoreError::invalid("the owner check failed"))?
        {
            return Err(CoreError::invalid("the owner did not confirm the policy resolution"));
        }
        self.require_decider_fresh(&session).await?;
        state.resolved_from = Some(crate::store::authorization::conflict_digest(&state));
        state.policies = policies;
        state.conflicted = false;
        state.conflicting_policies.clear();
        state.revision =
            expected_revision.checked_add(1).ok_or_else(|| CoreError::invalid("policy revision exhausted"))?;
        for policy in &state.policies {
            if policy.lifecycle == PolicyLifecycle::Revoked {
                state.revoked_ids.insert(policy.id.clone());
            }
        }
        self.store.authorization_replace(expected_revision, &state)?;
        Ok(state)
    }

    /// Complete authenticated state. Positive adapters must also refuse `overflow_digest`, and must independently
    /// prove account admission; owner-resolved restrictive overflow is not recovered positive authority.
    pub(crate) fn authenticated_authorization_state(&self) -> Result<Option<AuthorizationState>, CoreError> {
        let Some(state) = self.store.authorization_state()? else {
            return Ok(None);
        };
        if state.conflicted {
            return Err(CoreError::invalid(
                "authorization policy is held for owner review after a conflict or unverified restore",
            ));
        }
        let owner = self.policy_owner()?;
        if state.owner != owner || state.policies.iter().any(|p| p.issuer.tenant_id.is_some()) {
            return Err(CoreError::invalid("authorization policy authority could not be verified"));
        }
        Ok(Some(state))
    }

    /// Normalize the actual request using the account the connector resolved. The returned facts describe this
    /// action; the caller still authenticates account admission, complete policies and execution authority.
    pub(crate) fn normalized_authorization_action(
        &self,
        request: &RelayRequest,
        resolved_account: &str,
        now: i64,
    ) -> Result<NormalizedAuthorizationAction, CoreError> {
        let owner = self.policy_owner()?;
        let (service, capability, action) = operation(&request.call);
        let account_id = account_resource_id(&service, resolved_account);
        let resource_id = if let ToolCall::Mcp(call) = &request.call {
            Some(call.tool.clone())
        } else {
            None
        };
        let bytes =
            serde_json::to_vec(&(request, &owner, &account_id)).map_err(|e| CoreError::storage(e.to_string()))?;
        let digest = reins_proto::canon::hex32(&reins_proto::canon::sha256(&bytes));
        let device = self.store.device_id()?;
        let mut facts = Vec::new();
        if let ToolCall::GmailSend {
            email,
        } = &request.call
        {
            facts.push(AttributeFact {
                key: AttributeKey::Recipients,
                fact: Fact::Known {
                    value: AttributeValue::StringSet(
                        email
                            .recipients()
                            .map(str::to_owned)
                            .collect::<std::collections::BTreeSet<_>>()
                            .into_iter()
                            .collect(),
                    ),
                    evidence: Evidence {
                        id: digest.clone(),
                        source: EvidenceSource::LocalCore,
                        observed_at: now,
                        valid_until: now.saturating_add(60),
                    },
                },
            });
        }
        if let ToolCall::GmailRead {
            message_ids,
        } = &request.call
        {
            facts.push(AttributeFact {
                key: AttributeKey::ResourceIds,
                fact: Fact::Known {
                    value: AttributeValue::StringSet(
                        message_ids.iter().cloned().collect::<std::collections::BTreeSet<_>>().into_iter().collect(),
                    ),
                    evidence: Evidence {
                        id: digest.clone(),
                        source: EvidenceSource::LocalCore,
                        observed_at: now,
                        valid_until: now.saturating_add(60),
                    },
                },
            });
        }
        let verified_attribute_keys = facts.iter().map(|fact| fact.key).collect();
        Ok(NormalizedAuthorizationAction {
            action: ActionContext {
                v: 1,
                request_id: request.id.0.clone(),
                request_digest: digest,
                actor: ActorContext {
                    principal_id: owner.clone(),
                    tenant_id: None,
                    connection_id: request.connection_id.0.clone(),
                    device_id: None,
                    executor_principal_id: owner,
                    executor_device_id: device,
                },
                operation: OperationId {
                    service,
                    capability,
                    action,
                },
                resource: ResourceContext {
                    account_id,
                    resource_id,
                },
                facts,
            },
            verified_attribute_keys,
        })
    }

    /// Narrowing preflight is private so its legacy baseline cannot be mistaken for new access authority.
    fn authorization_decision(&self, request: &RelayRequest) -> Result<Option<AuthorizationDecision>, CoreError> {
        let Some(state) = self.authenticated_authorization_state()? else {
            return Ok(None);
        };
        let (service, capability, action) = operation(&request.call);
        // Account IDs are connector-scoped canonical tuples. A display name or an agent-provided repository name
        // is never promoted to an authenticated account, environment, device, or repository classification.
        let named = if matches!(request.call, ToolCall::ListAccounts { .. } | ToolCall::Mcp(_)) {
            None
        } else {
            request.account.clone()
        };
        let mut accounts = named.map_or_else(
            || {
                if let ToolCall::Mcp(call) = &request.call {
                    vec![call.server.clone()]
                } else if service == reins_proto::connector::DESKTOP {
                    vec![crate::connector::desktop::DESKTOP_ACCOUNT.to_owned()]
                } else {
                    self.accounts_of(&service)
                }
            },
            |a| vec![a],
        );
        if accounts.is_empty() {
            if state.policies.iter().any(|policy| {
                !policy.scope.account_ids.is_empty()
                    && (policy.scope.operations.is_empty()
                        || policy
                            .scope
                            .operations
                            .iter()
                            .any(|op| op.service == service && op.capability == capability && op.action == action))
            }) {
                return Err(CoreError::invalid(
                    "the account required to evaluate this policy could not be established",
                ));
            }
            accounts.push(String::new());
        }
        // The common gate only narrows an independently established path. Positive records are evaluated
        // separately with actual admission and transactional usage; a missing budget here cannot mint authority.
        let restrictions: Vec<_> = state
            .policies
            .iter()
            .filter(|policy| matches!(policy.effect, PolicyEffect::Deny | PolicyEffect::RequireReview { .. }))
            .cloned()
            .collect();
        let ids: Vec<String> = restrictions.iter().map(|p| p.id.clone()).collect();
        let mut result = None;
        for account in accounts {
            let now = unix_now();
            let normalized = self.normalized_authorization_action(request, &account, now)?;
            let action = normalized.action;
            // This gate evaluates mandatory restrictions on an already existing legacy authorization path, and
            // never supplies new auto-authority. Account-discovery still goes through its separate grant/owner
            // approval path. Future Permit evaluation must use actual account-access proof, never this preflight.
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
                account_access: true,
                verified_attribute_keys: &normalized.verified_attribute_keys,
                authorized_policy_ids: &ids,
            };
            let decision = authorization::evaluate(&EvaluationInput {
                action: &action,
                policies: &restrictions,
                authority: &authority,
                now,
                usage: &[],
                baseline: if crate::store::permissions::is_managed(&state, &action) {
                    BaselineDecision::Review
                } else {
                    BaselineDecision::Permit
                },
            });
            if result.as_ref().is_none_or(|old: &AuthorizationDecision| rank(decision.outcome) > rank(old.outcome)) {
                result = Some(decision);
            }
        }
        Ok(result)
    }

    pub(crate) fn authorization_automatic(&self, request: &RelayRequest) -> Result<bool, CoreError> {
        Ok(self
            .authorization_decision(request)?
            .is_none_or(|d| d.outcome == DecisionOutcome::Permit && d.authorization_path.is_empty()))
    }

    /// Preflight happens before account resolution can contact a provider, preview collection or model prompts.
    /// Review is implemented for Gmail sends in this slice; matching unsupported requirements are visibly held.
    pub(crate) fn authorization_preflight(&self, request: &RelayRequest) -> Result<(), CoreError> {
        let Some(decision) = self.authorization_decision(request)? else {
            return Ok(());
        };
        if decision.outcome == DecisionOutcome::Review && is_gmail_action(request) {
            return Ok(());
        }
        self.authorization_execute(request, AuthorizationSource::Automatic)
    }

    pub(crate) fn authorization_execute(
        &self,
        request: &RelayRequest,
        source: AuthorizationSource,
    ) -> Result<(), CoreError> {
        let Some(decision) = self.authorization_decision(request)? else {
            return Ok(());
        };
        match decision.outcome {
            DecisionOutcome::Permit if decision.authorization_path.is_empty() => Ok(()),
            DecisionOutcome::Review
                if is_gmail_action(request)
                    && source == AuthorizationSource::OwnerReview
                    && decision.required_reviews.iter().all(|r| {
                        r.role_id == "owner"
                            && r.quorum == 1
                            && !r.exclude_requester
                            && r.distinct_humans_group.is_none()
                    }) =>
            {
                Ok(())
            }
            DecisionOutcome::Deny => Err(CoreError::invalid("this action is blocked by an account policy")),
            DecisionOutcome::Review => {
                Err(CoreError::invalid("this action requires the reviewers specified by its account policy"))
            }
            _ => Err(CoreError::invalid("this action is held because its authorization cannot be verified")),
        }
    }
}

fn is_gmail_action(request: &RelayRequest) -> bool {
    matches!(request.call, ToolCall::GmailSend { .. } | ToolCall::GmailSearch { .. } | ToolCall::GmailRead { .. })
}

pub fn account_resource_id(service: &str, account: &str) -> String {
    serde_json::json!([service, account]).to_string()
}

fn rank(outcome: DecisionOutcome) -> u8 {
    match outcome {
        DecisionOutcome::Permit => 0,
        DecisionOutcome::EligibleForAuto => 1,
        DecisionOutcome::Review => 2,
        DecisionOutcome::Hold => 3,
        DecisionOutcome::Deny => 4,
    }
}

fn operation(call: &ToolCall) -> (String, String, String) {
    let (service, capability, action) = match call {
        ToolCall::GmailSearch {
            ..
        } => ("gmail".to_owned(), "read".to_owned(), "search".to_owned()),
        ToolCall::GmailRead {
            ..
        } => ("gmail".to_owned(), "read".to_owned(), "read".to_owned()),
        ToolCall::GmailSend {
            ..
        } => ("gmail".to_owned(), "write".to_owned(), "send".to_owned()),
        ToolCall::RequestGrant {
            ..
        } => ("gmail".to_owned(), "permission".to_owned(), "create_grant".to_owned()),
        ToolCall::ListAccounts {
            service,
            ..
        } => (service.clone().unwrap_or_else(|| "reins".to_owned()), "accounts".to_owned(), "list".to_owned()),
        ToolCall::Connector(call) => {
            (call.service.clone(), call.effect().map_or("unknown", |e| e.access()).to_owned(), call.op.clone())
        }
        ToolCall::Mcp(call) => (crate::mcp::grant_service(&call.server), "tool".to_owned(), "call".to_owned()),
        ToolCall::RequestUpload {
            ..
        } => ("reins".to_owned(), "files".to_owned(), "upload".to_owned()),
    };
    (service, capability, action)
}

fn validate_activation(policy: &PolicyRecord, owner: &str) -> Result<(), CoreError> {
    policy.validate().map_err(|e| CoreError::invalid(e.to_string()))?;
    if policy.issuer.principal_id != owner || policy.issuer.tenant_id.is_some() {
        return Err(CoreError::invalid("this policy issuer has not been authenticated"));
    }
    if !matches!(policy.effect, PolicyEffect::Deny | PolicyEffect::RequireReview { .. }) {
        return Err(CoreError::invalid(
            "automatic permissions and delegations require execution enforcement that is not available yet",
        ));
    }
    if matches!(policy.effect, PolicyEffect::RequireReview { .. })
        && (policy.scope.operations.is_empty()
            || policy
                .scope
                .operations
                .iter()
                .any(|op| op.service != "gmail" || op.capability != "write" || op.action != "send"))
    {
        return Err(CoreError::invalid(
            "review policies currently support Gmail sending only; this operation needs its review enforcement adapter",
        ));
    }
    Ok(())
}
