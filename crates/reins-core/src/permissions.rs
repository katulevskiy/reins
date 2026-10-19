//! Capability-driven native permissions. All choices compile to the shared ABAC policy representation.

use std::collections::{BTreeMap, BTreeSet};

use reins_policy::{AccountsScope, Grant, Scope};
use reins_proto::authorization::{
    Condition, OperationId, PolicyEffect, PolicyIssuer, PolicyKind, PolicyLifecycle, PolicyRecord, PolicyScope,
    ReviewRequirement, UsageLimits,
};
use reins_proto::canon::{hex32, sha256};
use reins_proto::ids::{ConnectionId, GrantId};
use reins_proto::relay::RelayRequest;

use crate::engine::Engine;
use crate::permission_types::{
    AccountPermissionPreset, AccountPermissionsDraft, AccountPermissionsView, CapabilityPermissionInput,
    DelegationChange, DelegationDraft, DelegationState, DelegationView, PermissionAgentView, PermissionCapability,
    PermissionCapabilityView, PermissionController, PermissionMode, PermissionPreview, PermissionRestriction,
    PermissionScopeKind, PermissionScopeOption, PermissionScopeOptions, PermissionScopeSelection,
};
use crate::store::{AuthorizationState, authorization::ManagedOperation, permissions as storage, unix_now};
use crate::{CoreError, OwnerCheck};

pub(crate) use storage::{PermissionReservation, VerifiedPermission};
pub const MAX_PERMISSION_SECS: i64 = 7 * 86_400;
pub const MAX_DELEGATION_SECS: i64 = 24 * 3_600;
const DEFAULT_USES: u64 = 1_000;

fn capability_key(capability: PermissionCapability) -> &'static str {
    match capability {
        PermissionCapability::MailSearch => "mail-search",
        PermissionCapability::MailRead => "mail-read",
        PermissionCapability::MailSend => "mail-send",
        PermissionCapability::IntegrationRead => "integration-read",
        PermissionCapability::IntegrationWrite => "integration-write",
    }
}

fn capabilities(service: &str) -> Vec<PermissionCapability> {
    if service == "gmail" {
        vec![PermissionCapability::MailSearch, PermissionCapability::MailRead, PermissionCapability::MailSend]
    } else {
        vec![PermissionCapability::IntegrationRead, PermissionCapability::IntegrationWrite]
    }
}

fn operations(service: &str, capability: PermissionCapability) -> Vec<OperationId> {
    let gmail = match capability {
        PermissionCapability::MailSearch => Some(("read", "search")),
        PermissionCapability::MailRead => Some(("read", "read")),
        PermissionCapability::MailSend => Some(("write", "send")),
        _ => None,
    };
    if let Some((capability, action)) = gmail {
        return if service == "gmail" {
            vec![OperationId {
                service: service.into(),
                capability: capability.into(),
                action: action.into(),
            }]
        } else {
            vec![]
        };
    }
    reins_proto::connector::specs()
        .iter()
        .filter(|s| {
            s.service == service
                && (s.effect == reins_proto::connector::Effect::Write)
                    == (capability == PermissionCapability::IntegrationWrite)
        })
        .map(|s| OperationId {
            service: service.into(),
            capability: s.effect.access().into(),
            action: s.op.into(),
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn title(capability: PermissionCapability) -> &'static str {
    match capability {
        PermissionCapability::MailSearch => "Search mail",
        PermissionCapability::MailRead => "Read message bodies",
        PermissionCapability::MailSend => "Send mail",
        PermissionCapability::IntegrationRead => "Read and search",
        PermissionCapability::IntegrationWrite => "Make changes",
    }
}

fn description(capability: PermissionCapability) -> &'static str {
    match capability {
        PermissionCapability::MailSearch => {
            "Search results include subjects and snippets. Suspected codes and passwords still require review."
        }
        PermissionCapability::MailRead => {
            "Read messages in this account. Suspected codes and passwords still require review."
        }
        PermissionCapability::MailSend => {
            "Send only to the recipients you explicitly select, including every To and Cc recipient."
        }
        _ => "This integration supports blocking now. Additional automatic permissions need an enforcing adapter.",
    }
}

fn default_scope(capability: PermissionCapability) -> PermissionScopeSelection {
    PermissionScopeSelection {
        kind: if capability == PermissionCapability::MailSend {
            PermissionScopeKind::SelectedRecipients
        } else {
            PermissionScopeKind::AllInAccount
        },
        ids: vec![],
    }
}

fn supported_modes(service: &str) -> Vec<PermissionMode> {
    if service == "gmail" {
        vec![PermissionMode::Ask, PermissionMode::Auto, PermissionMode::Block]
    } else {
        vec![PermissionMode::Block]
    }
}

fn scopes(capability: PermissionCapability) -> Vec<PermissionScopeKind> {
    match capability {
        PermissionCapability::MailSend => vec![PermissionScopeKind::SelectedRecipients],
        PermissionCapability::MailRead => {
            vec![PermissionScopeKind::AllInAccount, PermissionScopeKind::SelectedResources]
        }
        _ => vec![PermissionScopeKind::AllInAccount],
    }
}

fn account_prefix(account_id: &str) -> String {
    format!("account:{}:", hex32(&sha256(account_id.as_bytes())))
}

fn mode(effect: &PolicyEffect) -> PermissionMode {
    match effect {
        PolicyEffect::Permit | PolicyEffect::EligibleForAuto | PolicyEffect::DelegationAuthority => {
            PermissionMode::Auto
        }
        PolicyEffect::Deny => PermissionMode::Block,
        PolicyEffect::RequireReview {
            ..
        } => PermissionMode::Ask,
    }
}

fn scope_of(policy: &PolicyRecord, capability: PermissionCapability) -> PermissionScopeSelection {
    policy
        .conditions
        .iter()
        .find_map(|condition| match condition {
            Condition::SetWithin {
                key: reins_proto::authorization::AttributeKey::Recipients,
                values,
                ..
            } => Some(PermissionScopeSelection {
                kind: PermissionScopeKind::SelectedRecipients,
                ids: values.clone(),
            }),
            Condition::SetWithin {
                key: reins_proto::authorization::AttributeKey::ResourceIds,
                values,
                ..
            } => Some(PermissionScopeSelection {
                kind: PermissionScopeKind::SelectedResources,
                ids: values.clone(),
            }),
            _ => None,
        })
        .unwrap_or_else(|| default_scope(capability))
}

fn scope_summary(scope: &PermissionScopeSelection) -> String {
    match scope.kind {
        PermissionScopeKind::AllInAccount => "Within this account".into(),
        PermissionScopeKind::SelectedRecipients => {
            if scope.ids.is_empty() {
                "Choose recipients".into()
            } else {
                format!("Only {}", scope.ids.join(", "))
            }
        }
        PermissionScopeKind::SelectedResources => format!("{} selected messages", scope.ids.len()),
    }
}

fn owner_review() -> PolicyEffect {
    PolicyEffect::RequireReview {
        requirements: vec![ReviewRequirement {
            role_id: "owner".into(),
            quorum: 1,
            distinct_humans_group: None,
            exclude_requester: false,
        }],
    }
}

fn validate_ids(ids: &[String], require: bool) -> Result<(), CoreError> {
    if ids.len() > 32
        || (require && ids.is_empty())
        || ids.iter().any(|id| id.is_empty() || id.len() > 256 || id.chars().any(char::is_control))
        || ids.iter().collect::<BTreeSet<_>>().len() != ids.len()
    {
        return Err(CoreError::invalid("choose a bounded set of distinct identities"));
    }
    Ok(())
}

fn conditions(rule: &CapabilityPermissionInput) -> Result<Vec<Condition>, CoreError> {
    validate_ids(
        &rule.scope.ids,
        rule.mode == PermissionMode::Auto && rule.scope.kind != PermissionScopeKind::AllInAccount,
    )?;
    if rule.mode != PermissionMode::Auto {
        return Ok(vec![]);
    }
    if !scopes(rule.capability).contains(&rule.scope.kind) {
        return Err(CoreError::invalid("this scope is not supported by the capability"));
    }
    match rule.scope.kind {
        PermissionScopeKind::AllInAccount => {
            if !rule.scope.ids.is_empty() {
                return Err(CoreError::invalid("all-account scope cannot contain selected IDs"));
            }
            Ok(vec![])
        }
        PermissionScopeKind::SelectedRecipients => {
            let canonical: Vec<_> = rule
                .scope
                .ids
                .iter()
                .map(|id| reins_proto::normalize_address(id).map_err(|e| CoreError::invalid(e.to_string())))
                .collect::<Result<_, _>>()?;
            if canonical != rule.scope.ids {
                return Err(CoreError::invalid("choose the normalized recipient options returned by Reins"));
            }
            Ok(vec![Condition::SetWithin {
                key: reins_proto::authorization::AttributeKey::Recipients,
                values: canonical,
                require_nonempty: true,
            }])
        }
        PermissionScopeKind::SelectedResources => Ok(vec![Condition::SetWithin {
            key: reins_proto::authorization::AttributeKey::ResourceIds,
            values: rule.scope.ids.clone(),
            require_nonempty: true,
        }]),
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "a policy record binds independent identity, scope, lifetime and budget fields"
)]
fn rule_record(
    owner: &str,
    id: String,
    service: &str,
    account: &str,
    agents: &[String],
    rule: &CapabilityPermissionInput,
    now: i64,
    expires: Option<i64>,
    limit: Option<u64>,
) -> Result<PolicyRecord, CoreError> {
    let ops = operations(service, rule.capability);
    if ops.is_empty() || !supported_modes(service).contains(&rule.mode) {
        return Err(CoreError::invalid("this permission choice has no execution adapter"));
    }
    let automatic = rule.mode == PermissionMode::Auto;
    validate_ids(agents, automatic)?;
    let conditions = conditions(rule)?;
    let effect = match rule.mode {
        PermissionMode::Ask => owner_review(),
        PermissionMode::Auto => PolicyEffect::Permit,
        PermissionMode::Block => PolicyEffect::Deny,
    };
    let record = PolicyRecord {
        v: 1,
        id,
        revision: 1,
        issuer: PolicyIssuer {
            principal_id: owner.into(),
            tenant_id: None,
        },
        kind: PolicyKind::AccountDefault,
        lifecycle: PolicyLifecycle::Active,
        valid_from: now,
        expires_at: if automatic {
            expires
        } else {
            None
        },
        scope: PolicyScope {
            connection_ids: agents.to_vec(),
            account_ids: vec![crate::authorization::account_resource_id(service, account)],
            operations: ops,
            ..PolicyScope::default()
        },
        conditions,
        effect,
        parent_policy_ids: vec![],
        limits: if automatic {
            UsageLimits {
                max_uses: limit,
                max_cost_microunits: None,
                cost_currency: None,
            }
        } else {
            UsageLimits::default()
        },
    };
    record.validate().map_err(|e| CoreError::invalid(e.to_string()))?;
    Ok(record)
}

impl Engine {
    fn permission_account(&self, service: &str, account: &str) -> Result<String, CoreError> {
        self.store
            .accounts()?
            .into_iter()
            .find(|a| a.service == service && a.account == account)
            .map(|a| a.account)
            .ok_or_else(|| CoreError::invalid("choose an account verified and connected on this phone"))
    }

    pub async fn account_permissions(&self, service: &str, account: &str) -> Result<AccountPermissionsView, CoreError> {
        let account = self.permission_account(service, account)?;
        let state = self.authorization_policies()?;
        let account_id = crate::authorization::account_resource_id(service, &account);
        let prefix = account_prefix(&account_id);
        let managed = state.managed_scopes.iter().any(|s| s.account_id == account_id);
        let mut restrictions = vec![];
        if state.conflicted || state.overflow_digest.is_some() {
            restrictions.push(PermissionRestriction {
                controller: PermissionController::Conflict,
                policy_id: None,
                reason: "Automatic permissions require a complete reconciled policy history".into(),
            });
        }
        let usage_verified = self.store.permission_history_ready().is_ok();
        let history_ready = self.store.permission_history_ready().unwrap_or(false);
        let positive_available = self.holds_roster_now()? && self.decides() && history_ready;
        if !positive_available {
            restrictions.push(PermissionRestriction { controller: PermissionController::UnsupportedAdapter, policy_id: None, reason: "Automatic permissions require a verified primary phone and complete execution history. Reconcile pending or conflicting attempts before enabling Auto.".into() });
        }
        let connections = self.connections().await?;
        let mut agents = vec![];
        for connection in connections {
            if self.store.permission_connection_revoked(&connection.id)? {
                continue;
            }
            let access = self
                .store
                .account_coverage(&ConnectionId(connection.id.clone()), service, unix_now())?
                .covers(&account);
            agents.push(PermissionAgentView {
                selected: state.policies.iter().any(|p| {
                    p.id.starts_with(&prefix)
                        && p.lifecycle == PolicyLifecycle::Active
                        && p.scope.connection_ids.contains(&connection.id)
                }),
                connection_id: connection.id,
                label: connection.label,
                has_account_access: access,
            });
        }
        let now = unix_now();
        let mut views = vec![];
        for capability in capabilities(service) {
            let suffix = format!(":{}", capability_key(capability));
            let own = state.policies.iter().find(|p| {
                p.id.starts_with(&prefix) && p.id.ends_with(&suffix) && p.lifecycle != PolicyLifecycle::Revoked
            });
            let scope = own.map_or_else(|| default_scope(capability), |p| scope_of(p, capability));
            let actual_scope_summary =
                if let Some(policy) = own.filter(|policy| mode(&policy.effect) != PermissionMode::Auto) {
                    if policy.scope.connection_ids.is_empty() {
                        "Every request of this capability from all agents".into()
                    } else {
                        format!(
                            "Every request of this capability from {} selected agents",
                            policy.scope.connection_ids.len()
                        )
                    }
                } else if own.is_none() && managed {
                    "Review by default for every agent in this account".into()
                } else {
                    scope_summary(&scope)
                };
            let mut floors = restrictions.clone();
            for policy in &state.policies {
                if policy.id.starts_with(&prefix)
                    || policy.lifecycle != PolicyLifecycle::Active
                    || policy.valid_from > now
                    || policy.expires_at.is_some_and(|e| now >= e)
                {
                    continue;
                }
                if !policy.scope.account_ids.is_empty() && !policy.scope.account_ids.contains(&account_id) {
                    continue;
                }
                let ops = operations(service, capability);
                if !policy.scope.operations.is_empty() && !ops.iter().any(|op| policy.scope.operations.contains(op)) {
                    continue;
                }
                if matches!(policy.effect, PolicyEffect::Deny | PolicyEffect::RequireReview { .. }) {
                    floors.push(PermissionRestriction {
                        controller: if policy.issuer.tenant_id.is_some() {
                            PermissionController::Organization
                        } else {
                            PermissionController::Owner
                        },
                        policy_id: Some(policy.id.clone()),
                        reason: "Additional policy requirements remain in force for matching actions".into(),
                    });
                }
            }
            if service != "gmail" {
                floors.push(PermissionRestriction {
                    controller: PermissionController::UnsupportedAdapter,
                    policy_id: None,
                    reason:
                        "This integration currently supports blocking; automatic execution needs an enforcing adapter"
                            .into(),
                });
            }
            let effective_reason = if state.conflicted {
                "Policy conflict holds execution".into()
            } else if own.is_some_and(|p| p.expires_at.is_some_and(|e| now >= e)) {
                "This permission expired; requests require review".into()
            } else if own.is_some() {
                "Applies within the selected scope and all additional requirements".into()
            } else if managed {
                "Review by default; active scoped delegations can authorize within all mandatory requirements".into()
            } else {
                "Existing grants and Auto settings apply; no account defaults have been saved".into()
            };
            views.push(PermissionCapabilityView {
                capability,
                title: title(capability).into(),
                description: description(capability).into(),
                supported_modes: if state.conflicted || state.overflow_digest.is_some() {
                    vec![]
                } else {
                    supported_modes(service)
                        .into_iter()
                        .filter(|mode| *mode != PermissionMode::Auto || positive_available)
                        .collect()
                },
                supported_scopes: scopes(capability),
                configured_mode: own.map(|p| mode(&p.effect)).or(managed.then_some(PermissionMode::Ask)),
                scope_summary: actual_scope_summary,
                scope,
                effective_reason,
                restrictions: floors,
            });
        }
        let automatic: Vec<_> = state
            .policies
            .iter()
            .filter(|p| {
                p.id.starts_with(&prefix) && p.effect == PolicyEffect::Permit && p.lifecycle == PolicyLifecycle::Active
            })
            .collect();
        let usage = automatic.iter().map(|p| self.store.permission_budget_uses(&p.id)).collect::<Result<Vec<_>, _>>();
        let usage_verified = usage_verified && usage.is_ok();
        let used = usage.as_ref().map_or(0, |uses| uses.iter().copied().max().unwrap_or(0));
        let remaining_uses = if usage_verified {
            automatic
                .iter()
                .zip(usage.unwrap_or_default())
                .map(|(policy, uses)| policy.limits.max_uses.unwrap_or(0).saturating_sub(uses))
                .min()
        } else {
            None
        };
        Ok(AccountPermissionsView {
            used,
            usage_verified,
            remaining_uses,
            service: service.into(),
            account,
            account_id,
            revision: state.revision,
            managed,
            conflicted: state.conflicted,
            capabilities: views,
            agents,
            available_presets: if service == "gmail" {
                if positive_available {
                    vec![
                        AccountPermissionPreset::ReviewEveryRequest,
                        AccountPermissionPreset::ReadAssist,
                        AccountPermissionPreset::Custom,
                    ]
                } else {
                    vec![AccountPermissionPreset::ReviewEveryRequest, AccountPermissionPreset::Custom]
                }
            } else {
                vec![AccountPermissionPreset::Custom]
            },
            expires_at: state.policies.iter().filter(|p| p.id.starts_with(&prefix)).filter_map(|p| p.expires_at).min(),
            maximum_duration_secs: u64::try_from(MAX_PERMISSION_SECS).expect("positive"),
            delegation_maximum_duration_secs: u64::try_from(MAX_DELEGATION_SECS).expect("positive"),
            scope_summary: if managed {
                "Account-specific policies; new agents receive no access automatically".into()
            } else {
                "No account defaults configured".into()
            },
            restrictions,
        })
    }

    pub async fn account_permission_draft(
        &self,
        service: &str,
        account: &str,
        preset: AccountPermissionPreset,
    ) -> Result<AccountPermissionsDraft, CoreError> {
        let view = self.account_permissions(service, account).await?;
        if !view.available_presets.contains(&preset) {
            return Err(CoreError::invalid("this preset is unavailable for the integration"));
        }
        let rules = view
            .capabilities
            .iter()
            .map(|capability| CapabilityPermissionInput {
                capability: capability.capability,
                mode: match preset {
                    AccountPermissionPreset::ReviewEveryRequest => PermissionMode::Ask,
                    AccountPermissionPreset::ReadAssist => {
                        if capability.capability == PermissionCapability::MailSend {
                            PermissionMode::Ask
                        } else {
                            PermissionMode::Auto
                        }
                    }
                    AccountPermissionPreset::Custom => capability
                        .configured_mode
                        .unwrap_or_else(|| capability.supported_modes.first().copied().unwrap_or(PermissionMode::Ask)),
                },
                scope: if preset == AccountPermissionPreset::Custom {
                    capability.scope.clone()
                } else {
                    default_scope(capability.capability)
                },
            })
            .collect();
        Ok(AccountPermissionsDraft {
            service: service.into(),
            account: account.into(),
            expected_revision: view.revision,
            preset,
            agent_ids: view.agents.into_iter().filter(|a| a.selected).map(|a| a.connection_id).collect(),
            grant_account_access: false,
            expires_at: if preset == AccountPermissionPreset::Custom && view.managed {
                view.expires_at
            } else {
                Some(unix_now().saturating_add(MAX_PERMISSION_SECS))
            },
            max_uses: if preset == AccountPermissionPreset::Custom && view.managed {
                view.remaining_uses.or(Some(DEFAULT_USES))
            } else {
                Some(DEFAULT_USES)
            },
            rules,
        })
    }

    fn candidate_permissions(
        &self,
        draft: &AccountPermissionsDraft,
    ) -> Result<(AuthorizationState, Vec<Grant>), CoreError> {
        self.permission_account(&draft.service, &draft.account)?;
        let mut state = self.authorization_policies()?;
        if state.revision != draft.expected_revision || state.conflicted || state.overflow_digest.is_some() {
            return Err(CoreError::invalid("reload and reconcile authorization before saving"));
        }
        validate_ids(&draft.agent_ids, draft.rules.iter().any(|r| r.mode == PermissionMode::Auto))?;
        if draft.rules.is_empty()
            || draft.rules.len() > 8
            || draft.rules.iter().map(|r| capability_key(r.capability)).collect::<BTreeSet<_>>().len()
                != draft.rules.len()
        {
            return Err(CoreError::invalid("choose distinct supported capabilities"));
        }
        if draft.rules.len() != capabilities(&draft.service).len() {
            return Err(CoreError::invalid(
                "save the complete capability choices so an omitted rule cannot retain an earlier permission",
            ));
        }
        if draft.rules.iter().any(|r| r.mode == PermissionMode::Auto)
            && !(self.holds_roster_now()? && self.decides() && self.store.permission_history_ready().unwrap_or(false))
        {
            return Err(CoreError::invalid("automatic permissions require the verified primary phone"));
        }
        let now = unix_now();
        if draft.rules.iter().any(|r| r.mode == PermissionMode::Auto)
            && !draft.expires_at.is_some_and(|e| now < e && e <= now.saturating_add(MAX_PERMISSION_SECS))
        {
            return Err(CoreError::invalid("automatic account permissions must expire within seven days"));
        }
        let limit = draft.max_uses.unwrap_or(DEFAULT_USES);
        if draft.rules.iter().any(|rule| rule.mode == PermissionMode::Auto) && (limit == 0 || limit > 100_000) {
            return Err(CoreError::invalid("choose a use limit between one and 100000"));
        }
        let account_id = crate::authorization::account_resource_id(&draft.service, &draft.account);
        let prefix = account_prefix(&account_id);
        let mut budgets = vec![];
        for rule in &draft.rules {
            let suffix = format!(":{}", capability_key(rule.capability));
            let id = state
                .policies
                .iter()
                .find(|policy| {
                    policy.id.starts_with(&prefix)
                        && policy.id.ends_with(&suffix)
                        && policy.lifecycle != PolicyLifecycle::Revoked
                })
                .map_or_else(
                    || format!("{prefix}{}:{key}", uuid::Uuid::new_v4(), key = capability_key(rule.capability)),
                    |policy| policy.id.clone(),
                );
            let maximum = if rule.mode == PermissionMode::Auto {
                self.store
                    .permission_budget_uses(&id)?
                    .checked_add(limit)
                    .ok_or_else(|| CoreError::invalid("permission usage is exhausted"))?
            } else {
                0
            };
            u32::try_from(maximum).map_err(|_| CoreError::invalid("permission usage is exhausted"))?;
            let mut record = rule_record(
                &state.owner,
                id,
                &draft.service,
                &draft.account,
                &draft.agent_ids,
                rule,
                now,
                draft.expires_at,
                Some(maximum),
            )?;
            if let Some(current) = state.policies.iter_mut().find(|p| p.id == record.id) {
                record.revision =
                    current.revision.checked_add(1).ok_or_else(|| CoreError::invalid("policy revision exhausted"))?;
                *current = record.clone();
            } else {
                state.policies.push(record.clone());
            }
            for operation in &record.scope.operations {
                state.managed_scopes.insert(ManagedOperation {
                    account_id: account_id.clone(),
                    operation: operation.clone(),
                });
            }
            if rule.mode == PermissionMode::Auto {
                budgets.push(storage::budget_grant(&record, now)?);
            }
        }
        state.revision =
            state.revision.checked_add(1).ok_or_else(|| CoreError::invalid("policy revision exhausted"))?;
        state.validate()?;
        Ok((state, budgets))
    }

    async fn admissions(
        &self,
        service: &str,
        account: &str,
        ids: &[String],
        grant_access: bool,
        expires_at: Option<i64>,
        origin: &str,
    ) -> Result<Vec<(Grant, String, String)>, CoreError> {
        let connections: BTreeMap<_, _> = self.connections().await?.into_iter().map(|c| (c.id.clone(), c)).collect();
        let mut admissions = vec![];
        for id in ids {
            let connection =
                connections.get(id).ok_or_else(|| CoreError::invalid("a selected agent is no longer connected"))?;
            if self.store.permission_connection_revoked(id)? {
                return Err(CoreError::invalid("a selected agent was revoked; reconnect it with a new identity"));
            }
            if self.store.account_coverage(&ConnectionId(id.clone()), service, unix_now())?.covers(account) {
                continue;
            }
            if !grant_access {
                return Err(CoreError::invalid("explicitly grant account access to the selected agents first"));
            }
            let grant = Grant::new(
                GrantId(uuid::Uuid::new_v4().to_string()),
                ConnectionId(id.clone()),
                Scope::Accounts(AccountsScope {
                    service: service.into(),
                    accounts: vec![account.into()],
                }),
                unix_now(),
                expires_at,
                None,
            )
            .map_err(|e| CoreError::invalid(e.to_string()))?;
            admissions.push((grant, connection.label.clone(), origin.into()));
        }
        Ok(admissions)
    }

    pub fn preview_account_permissions(&self, draft: &AccountPermissionsDraft) -> Result<PermissionPreview, CoreError> {
        let (candidate, _) = self.candidate_permissions(draft)?;
        self.permission_preview(
            &candidate,
            &draft.service,
            &draft.account,
            draft.expected_revision,
            format!("Save {} account rules for {} selected agents", draft.rules.len(), draft.agent_ids.len()),
        )
    }

    pub async fn save_account_permissions(
        &self,
        draft: &AccountPermissionsDraft,
        owner: &dyn OwnerCheck,
    ) -> Result<AccountPermissionsView, CoreError> {
        self.candidate_permissions(draft)?;
        self.admissions(
            &draft.service,
            &draft.account,
            &draft.agent_ids,
            draft.grant_account_access,
            draft.expires_at,
            "policy_access",
        )
        .await?;
        let summary = format!(
            "Save {} permissions for {}. {} selected agents{}. Ask/Block covers all requests of that capability from those agents. Automatic permissions expire at {:?}, with at most {:?} further uses per capability. {} Existing mandatory requirements remain in force.",
            draft.rules.len(),
            draft.account,
            draft.agent_ids.len(),
            if draft.grant_account_access {
                " may discover this account"
            } else {
                " retain their existing account access"
            },
            draft.expires_at,
            draft.max_uses,
            draft
                .rules
                .iter()
                .map(|rule| format!(
                    "{}: {:?} ({})",
                    title(rule.capability),
                    rule.mode,
                    if rule.mode == PermissionMode::Auto {
                        scope_summary(&rule.scope)
                    } else {
                        "every request of this capability".into()
                    }
                ))
                .collect::<Vec<_>>()
                .join("; ")
        );
        if !owner.confirm(summary).await.map_err(|_| CoreError::invalid("the owner check failed"))? {
            return Err(CoreError::invalid("the owner did not confirm these account permissions"));
        }
        let fence = self.policy_mutation_fence().await?;
        self.require_decider_fresh(self.session()?.as_ref()).await?;
        let (candidate, budgets) = self.candidate_permissions(draft)?;
        let admissions = self
            .admissions(
                &draft.service,
                &draft.account,
                &draft.agent_ids,
                draft.grant_account_access,
                draft.expires_at,
                "policy_access",
            )
            .await?;
        self.require_decider_fresh(self.session()?.as_ref()).await?;
        self.store.permissions_apply(draft.expected_revision, &candidate, &admissions, &budgets)?;
        drop(fence);
        self.account_permissions(&draft.service, &draft.account).await
    }

    pub fn permission_scope_options(
        &self,
        service: &str,
        account: &str,
        capability: PermissionCapability,
        query: &str,
        limit: u32,
    ) -> Result<PermissionScopeOptions, CoreError> {
        self.permission_account(service, account)?;
        if !capabilities(service).contains(&capability) {
            return Err(CoreError::invalid("this capability does not belong to the selected integration"));
        }
        let kinds = scopes(capability);
        let mut options = BTreeMap::new();
        let needle = query.to_lowercase();
        for entry in self.store.activity(500)? {
            if entry.service != service || entry.account.as_deref() != Some(account) {
                continue;
            }
            if capability == PermissionCapability::MailSend {
                if let Some(email) = entry.info.email {
                    for address in email.to.into_iter().chain(email.cc) {
                        if let Ok(id) = reins_proto::normalize_address(&address) {
                            options.insert(
                                id.clone(),
                                (id, "Recipient seen in this account".into(), PermissionScopeKind::SelectedRecipients),
                            );
                        }
                    }
                }
            } else if capability == PermissionCapability::MailRead {
                for message in entry.info.messages {
                    if !message.id.is_empty() {
                        options.insert(
                            message.id.clone(),
                            (message.subject, message.from, PermissionScopeKind::SelectedResources),
                        );
                    }
                }
            }
        }
        if capability == PermissionCapability::MailSend
            && let Ok(address) = reins_proto::normalize_address(query)
        {
            options.insert(
                address.clone(),
                (
                    address,
                    "Explicitly selected address; every outgoing recipient must match".into(),
                    PermissionScopeKind::SelectedRecipients,
                ),
            );
        }
        let options = options
            .into_iter()
            .filter(|(id, (label, _, _))| {
                needle.is_empty() || id.to_lowercase().contains(&needle) || label.to_lowercase().contains(&needle)
            })
            .take(usize::try_from(limit.clamp(1, 100)).expect("bounded"))
            .map(|(id, (label, detail, kind))| PermissionScopeOption {
                id,
                label,
                detail,
                service: service.into(),
                account: account.into(),
                kind,
            })
            .collect();
        Ok(PermissionScopeOptions {
            kinds,
            options,
            allows_address_entry: service == "gmail" && capability == PermissionCapability::MailSend,
            unsupported_reason: (service != "gmail")
                .then(|| "Scoped automatic execution is not implemented for this integration".into()),
        })
    }

    fn permission_preview(
        &self,
        candidate: &AuthorizationState,
        service: &str,
        account: &str,
        revision: u64,
        summary: String,
    ) -> Result<PermissionPreview, CoreError> {
        use reins_policy::authorization::{
            AuthorityBindings, BaselineDecision, DecisionOutcome, EvaluationInput, UsageSnapshot,
        };
        let mut eligible = 0u32;
        let mut review = 0u32;
        let mut blocked = 0u32;
        let mut incomplete = false;
        let now = unix_now();
        let ids: Vec<_> = candidate.policies.iter().map(|policy| policy.id.clone()).collect();
        let usage = candidate
            .policies
            .iter()
            .filter(|p| p.limits.max_uses.is_some())
            .map(|p| {
                Ok(UsageSnapshot {
                    policy_id: p.id.clone(),
                    revision: p.revision,
                    committed_uses: self.store.permission_budget_uses(&p.id)?,
                    reserved_uses: 0,
                    committed_cost_microunits: 0,
                    reserved_cost_microunits: 0,
                    cost_currency: None,
                })
            })
            .collect::<Result<Vec<_>, CoreError>>();
        incomplete |= usage.is_err();
        let usage = usage.unwrap_or_default();
        let history = self.store.activity(500)?;
        if history.len() == 500 {
            incomplete = true;
        }
        for entry in history {
            if entry.service != service || entry.account.as_deref() != Some(account) {
                continue;
            }
            let Some(snapshot) = self.store.audit_snapshot(entry.seq)? else {
                incomplete = true;
                continue;
            };
            let normalized = self.normalized_authorization_action(&snapshot.request, account, now)?;
            let action = &normalized.action;
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
            // Simulation only: assumes separately admitted account access. This result is never executable proof.
            let decision = reins_policy::authorization::evaluate(&EvaluationInput {
                action,
                policies: &candidate.policies,
                authority: &authority,
                now,
                usage: &usage,
                baseline: BaselineDecision::Review,
            });
            match decision.outcome {
                DecisionOutcome::Deny => blocked += 1,
                DecisionOutcome::Permit if !snapshot.messages.iter().any(crate::views::looks_sensitive) => {
                    eligible += 1;
                }
                _ => review += 1,
            }
        }
        Ok(PermissionPreview { revision, summary, warnings: vec!["Counts simulate retained requests against this draft and current usage; separate agent account admission is still required".into(), "Mandatory restrictions, unknown facts and suspected codes/passwords still require review".into()], eligible_history_count: Some(eligible), review_history_count: Some(review), blocked_history_count: Some(blocked), history_complete: !incomplete })
    }

    fn permission_epoch(&self) -> Result<Option<u64>, CoreError> {
        if self.role_now()? != crate::roles::PhoneRole::Primary {
            return Ok(None);
        }
        let slot = self.account_slot_now()?.ok_or_else(|| CoreError::invalid("the account roster is unavailable"))?;
        Ok(self.local_roster(&slot)?.map(|roster| roster.epoch()))
    }

    pub(crate) fn permission_managed(&self, request: &RelayRequest) -> Result<bool, CoreError> {
        let Some(account) = request.account.as_deref() else {
            return Ok(false);
        };
        let normalized = self.normalized_authorization_action(request, account, unix_now())?;
        Ok(self
            .authenticated_authorization_state()?
            .is_some_and(|state| storage::is_managed(&state, &normalized.action)))
    }

    pub(crate) fn permission_route(&self, request: &RelayRequest) -> Result<PermissionReservation, CoreError> {
        let Some(account) = request.account.as_deref() else {
            return Ok(PermissionReservation::Review);
        };
        let normalized = self.normalized_authorization_action(request, account, unix_now())?;
        let Some(epoch) = self.permission_epoch()? else {
            return if self.permission_managed(request)? {
                Ok(PermissionReservation::Review)
            } else {
                Ok(PermissionReservation::Legacy)
            };
        };
        self.store.permission_reserve_at_epoch(
            &normalized.action,
            &normalized.verified_attribute_keys,
            unix_now(),
            epoch,
        )
    }

    pub(crate) fn revalidate_permission(
        &self,
        request: &RelayRequest,
        proof: &VerifiedPermission,
    ) -> Result<(), CoreError> {
        let account =
            request.account.as_deref().ok_or_else(|| CoreError::invalid("the request account is unresolved"))?;
        let normalized = self.normalized_authorization_action(request, account, unix_now())?;
        self.store.permission_revalidate_at_epoch(
            proof,
            &normalized.action,
            &normalized.verified_attribute_keys,
            unix_now(),
            self.permission_epoch()?.ok_or_else(|| CoreError::invalid("the verified primary changed"))?,
        )
    }

    pub(crate) async fn permission_before_send(
        &self,
        session: &crate::session::Session,
        request: &RelayRequest,
        proof: &VerifiedPermission,
    ) -> Result<(), CoreError> {
        self.require_decider_fresh(session).await?;
        self.revalidate_permission(request, proof)
    }

    pub(crate) async fn legacy_before_send(
        &self,
        session: &crate::session::Session,
        request: &RelayRequest,
        source: crate::authorization::AuthorizationSource,
    ) -> Result<(), CoreError> {
        self.require_decider_fresh(session).await?;
        self.authorization_execute(request, source)
    }

    pub(crate) fn permission_finished(
        &self,
        request: &RelayRequest,
        proof: VerifiedPermission,
        succeeded: bool,
    ) -> Result<(), CoreError> {
        let trace = proof.trace().clone();
        self.store.permission_outcome(proof, succeeded)?;
        self.authorization_attempt_event(
            request,
            &trace,
            if succeeded {
                crate::authorization_trace::AuthorizationPhase::Succeeded
            } else {
                crate::authorization_trace::AuthorizationPhase::Unknown
            },
        )
    }

    pub(crate) fn legacy_send_attempt(&self, request: &RelayRequest) -> Result<storage::SendAttempt, CoreError> {
        let account =
            request.account.as_deref().ok_or_else(|| CoreError::invalid("the sending account is unresolved"))?;
        let normalized = self.normalized_authorization_action(request, account, unix_now())?;
        self.store.legacy_send_attempt(&normalized.action, unix_now())
    }

    pub(crate) fn legacy_send_finished(&self, attempt: storage::SendAttempt, succeeded: bool) -> Result<(), CoreError> {
        self.store.legacy_send_outcome(attempt, succeeded)
    }

    /// Existing grants keep their legacy availability. Persist their local replay marker before invoking Gmail;
    /// this is not the cross-device durable authority required for new positive permissions.
    pub(crate) async fn legacy_send_durable(&self) -> Result<(), CoreError> {
        self.require_decider_fresh(self.session()?.as_ref()).await?;
        self.store.flush()
    }

    /// A local permit does not survive a crash/handover unless its receipt and every consumption reached the
    /// account's authenticated history. An offline or incomplete sync therefore stops positive execution.
    pub(crate) async fn permission_durable(
        &self,
        request: &RelayRequest,
        proof: &VerifiedPermission,
    ) -> Result<(), CoreError> {
        if !(self.holds_roster_now()? && self.decides() && self.store.permission_history_ready().unwrap_or(false)) {
            return Err(CoreError::invalid("automatic execution requires a verified single-primary roster"));
        }
        self.authorization_attempt_event(
            request,
            proof.trace(),
            crate::authorization_trace::AuthorizationPhase::Started,
        )?;
        self.execution_attempt_durable().await?;
        self.revalidate_permission(request, proof)
    }

    pub(crate) async fn execution_attempt_durable(&self) -> Result<(), CoreError> {
        self.require_decider_fresh(self.session()?.as_ref()).await?;
        self.store.flush()?;
        if self.store.owner.is_some() {
            self.sync_account_state().await?;
            if self.holds_roster_now()? {
                let state = self.live_sync_state()?;
                if self.has_unsent_ops()?
                    || !self.unpublished_rows()?.is_empty()
                    || state.notice.is_some()
                    || state.forked.is_some()
                    || state.waiting_for_key
                    || state.disagrees
                    || state.applied_seq != state.head_seq
                {
                    return Err(CoreError::invalid(
                        "execution usage has not reached a complete authenticated account history",
                    ));
                }
            } else if self.store.pending_upload()?.is_some() {
                return Err(CoreError::invalid("execution usage has not reached the account's durable state"));
            }
        } else if !cfg!(test) {
            return Err(CoreError::invalid("unlock the account before enabling automatic permissions"));
        }
        self.require_decider_fresh(self.session()?.as_ref()).await?;
        Ok(())
    }

    fn candidate_delegation(
        &self,
        draft: &DelegationDraft,
        identity: &str,
    ) -> Result<(AuthorizationState, Vec<Grant>), CoreError> {
        if !(self.holds_roster_now()? && self.decides() && self.store.permission_history_ready().unwrap_or(false)) {
            return Err(CoreError::invalid("delegations require this account’s verified primary phone"));
        }
        self.permission_account(&draft.service, &draft.account)?;
        let mut state = self.authorization_policies()?;
        if state.revision != draft.expected_revision || state.conflicted || state.overflow_digest.is_some() {
            return Err(CoreError::invalid("reload and reconcile authorization before issuing a delegation"));
        }
        validate_ids(&draft.agent_ids, true)?;
        let now = unix_now();
        if draft.expires_at <= now
            || draft.expires_at > now.saturating_add(MAX_DELEGATION_SECS)
            || draft.max_uses == 0
            || draft.max_uses > 100_000
        {
            return Err(CoreError::invalid(
                "delegations need a one-day-or-shorter timebox and a use limit between one and 100000",
            ));
        }
        if draft.service != "gmail"
            || draft.rules.is_empty()
            || draft.rules.len() > 3
            || draft.rules.iter().any(|r| r.mode != PermissionMode::Auto)
            || draft.rules.iter().map(|r| capability_key(r.capability)).collect::<BTreeSet<_>>().len()
                != draft.rules.len()
        {
            return Err(CoreError::invalid("choose distinct supported automatic delegation capabilities"));
        }
        let account_id = crate::authorization::account_resource_id(&draft.service, &draft.account);
        let mut budgets = vec![];
        for rule in &draft.rules {
            let mut child = rule_record(
                &state.owner,
                format!("delegation:{identity}:{}", capability_key(rule.capability)),
                &draft.service,
                &draft.account,
                &draft.agent_ids,
                rule,
                now,
                Some(draft.expires_at),
                Some(draft.max_uses),
            )?;
            let mut authority = child.clone();
            authority.id = format!("delegation-authority:{identity}:{}", capability_key(rule.capability));
            authority.kind = PolicyKind::Permission;
            authority.effect = PolicyEffect::DelegationAuthority;
            child.kind = PolicyKind::Delegation;
            child.parent_policy_ids = vec![authority.id.clone()];
            child.validate().map_err(|e| CoreError::invalid(e.to_string()))?;
            authority.validate().map_err(|e| CoreError::invalid(e.to_string()))?;
            for operation in &child.scope.operations {
                state.managed_scopes.insert(ManagedOperation {
                    account_id: account_id.clone(),
                    operation: operation.clone(),
                });
            }
            budgets.push(storage::budget_grant(&child, now)?);
            state.policies.push(authority);
            state.policies.push(child);
        }
        state.revision =
            state.revision.checked_add(1).ok_or_else(|| CoreError::invalid("policy revision exhausted"))?;
        state.validate()?;
        Ok((state, budgets))
    }

    pub async fn delegation_draft(&self, service: &str, account: &str) -> Result<DelegationDraft, CoreError> {
        if !(self.holds_roster_now()? && self.decides() && self.store.permission_history_ready().unwrap_or(false)) {
            return Err(CoreError::invalid("delegations require this account’s verified primary phone"));
        }
        let defaults = self.account_permission_draft(service, account, AccountPermissionPreset::ReadAssist).await?;
        if service != "gmail" {
            return Err(CoreError::invalid("automatic delegations are unavailable for this integration"));
        }
        Ok(DelegationDraft {
            service: service.into(),
            account: account.into(),
            expected_revision: defaults.expected_revision,
            agent_ids: defaults.agent_ids,
            grant_account_access: false,
            rules: defaults.rules.into_iter().filter(|rule| rule.mode == PermissionMode::Auto).collect(),
            expires_at: unix_now().saturating_add(3_600),
            max_uses: 20,
        })
    }

    pub fn preview_delegation(&self, draft: &DelegationDraft) -> Result<PermissionPreview, CoreError> {
        let (candidate, _) = self.candidate_delegation(draft, &uuid::Uuid::new_v4().to_string())?;
        self.permission_preview(
            &candidate,
            &draft.service,
            &draft.account,
            draft.expected_revision,
            format!(
                "Delegate {} capabilities to {} agents, at most {} total requests, until {}",
                draft.rules.len(),
                draft.agent_ids.len(),
                draft.max_uses,
                draft.expires_at
            ),
        )
    }

    pub async fn issue_delegation(
        &self,
        draft: &DelegationDraft,
        owner: &dyn OwnerCheck,
    ) -> Result<DelegationView, CoreError> {
        let identity = uuid::Uuid::new_v4().to_string();
        self.candidate_delegation(draft, &identity)?;
        self.admissions(
            &draft.service,
            &draft.account,
            &draft.agent_ids,
            draft.grant_account_access,
            Some(draft.expires_at),
            &format!("policy_access:{identity}"),
        )
        .await?;
        let summary = format!(
            "Delegate {} on {} to {} selected agents, at most {} total requests, until {}. {}",
            draft.rules.iter().map(|r| title(r.capability)).collect::<Vec<_>>().join(", "),
            draft.account,
            draft.agent_ids.len(),
            draft.max_uses,
            draft.expires_at,
            draft.rules.iter().map(|r| scope_summary(&r.scope)).collect::<Vec<_>>().join("; ")
        );
        if !owner.confirm(summary).await.map_err(|_| CoreError::invalid("the owner check failed"))? {
            return Err(CoreError::invalid("the owner did not confirm this delegation"));
        }
        let fence = self.policy_mutation_fence().await?;
        self.require_decider_fresh(self.session()?.as_ref()).await?;
        let (candidate, budgets) = self.candidate_delegation(draft, &identity)?;
        let admissions = self
            .admissions(
                &draft.service,
                &draft.account,
                &draft.agent_ids,
                draft.grant_account_access,
                Some(draft.expires_at),
                &format!("policy_access:{identity}"),
            )
            .await?;
        self.require_decider_fresh(self.session()?.as_ref()).await?;
        self.store.permissions_apply(draft.expected_revision, &candidate, &admissions, &budgets)?;
        drop(fence);
        self.delegations()?
            .into_iter()
            .find(|d| d.id == identity)
            .ok_or_else(|| CoreError::storage("the saved delegation is unavailable"))
    }

    pub fn delegations(&self) -> Result<Vec<DelegationView>, CoreError> {
        let state = self.authorization_policies()?;
        let mut groups: BTreeMap<String, Vec<&PolicyRecord>> = BTreeMap::new();
        for policy in &state.policies {
            let parts: Vec<_> = policy.id.split(':').collect();
            if policy.kind == PolicyKind::Delegation && parts.len() == 3 && parts[0] == "delegation" {
                groups.entry(parts[1].into()).or_default().push(policy);
            }
        }
        let mut views = vec![];
        for (id, policies) in groups {
            let first = policies[0];
            let Some(raw_account) = first.scope.account_ids.first() else {
                return Err(CoreError::storage("delegation account is missing"));
            };
            let account: Vec<String> =
                serde_json::from_str(raw_account).map_err(|_| CoreError::storage("delegation account is invalid"))?;
            if account.len() != 2 {
                return Err(CoreError::storage("delegation account is invalid"));
            }
            let expires_at = policies
                .iter()
                .filter_map(|p| p.expires_at)
                .min()
                .ok_or_else(|| CoreError::storage("delegation expiry is missing"))?;
            let parent_ids: BTreeSet<_> = policies.iter().flat_map(|p| &p.parent_policy_ids).collect();
            let all: Vec<_> =
                policies.iter().copied().chain(state.policies.iter().filter(|p| parent_ids.contains(&p.id))).collect();
            let status = if all.iter().any(|p| p.lifecycle == PolicyLifecycle::Revoked) {
                DelegationState::Revoked
            } else if unix_now() >= expires_at {
                DelegationState::Expired
            } else if all.iter().any(|p| p.lifecycle == PolicyLifecycle::Paused) {
                DelegationState::Paused
            } else {
                DelegationState::Active
            };
            let rules: Vec<_> = policies
                .iter()
                .map(|p| {
                    let capability = capabilities(&account[0])
                        .into_iter()
                        .find(|c| p.id.ends_with(capability_key(*c)))
                        .ok_or_else(|| CoreError::storage("delegation capability is unknown"))?;
                    Ok(CapabilityPermissionInput {
                        capability,
                        mode: PermissionMode::Auto,
                        scope: scope_of(p, capability),
                    })
                })
                .collect::<Result<_, CoreError>>()?;
            let scope_summary = rules
                .iter()
                .map(|r| format!("{}: {}", title(r.capability), scope_summary(&r.scope)))
                .collect::<Vec<_>>()
                .join("; ");
            let usage = self.store.permission_budget_uses(&first.id);
            let usage_verified = usage.is_ok();
            let used = usage.unwrap_or(0);
            let ids: Vec<_> = all.iter().map(|p| p.id.clone()).collect();
            let unresolved = self.store.permission_unresolved(&ids).unwrap_or(0);
            let max_uses = policies
                .iter()
                .filter_map(|p| p.limits.max_uses)
                .min()
                .ok_or_else(|| CoreError::storage("delegation usage limit is missing"))?;
            let restrictions =
                if state.conflicted || state.overflow_digest.is_some() || !(self.holds_roster_now()? && self.decides())
                {
                    vec![PermissionRestriction {
                        controller: PermissionController::Conflict,
                        policy_id: None,
                        reason: "Automatic authority is held until its complete policy history is reconciled".into(),
                    }]
                } else {
                    vec![]
                };
            views.push(DelegationView {
                id,
                revision: state.revision,
                title: format!(
                    "{} in {}",
                    rules.iter().map(|r| title(r.capability)).collect::<Vec<_>>().join(" and "),
                    account[1]
                ),
                service: account[0].clone(),
                account: account[1].clone(),
                state: status,
                expires_at,
                agent_ids: first.scope.connection_ids.clone(),
                rules,
                used,
                usage_verified,
                unresolved,
                max_uses,
                scope_summary,
                effective_reason: if !usage_verified {
                    "Execution usage could not be verified; automatic execution is held".into()
                } else if used >= max_uses {
                    "Usage limit reached; further requests require review".into()
                } else if status == DelegationState::Active {
                    "Within the delegation and every mandatory account requirement".into()
                } else {
                    "This delegation cannot authorize automatic execution".into()
                },
                restrictions,
            });
        }
        Ok(views)
    }

    pub async fn change_delegation(
        &self,
        identity: &str,
        expected_revision: u64,
        change: DelegationChange,
        owner: &dyn OwnerCheck,
    ) -> Result<DelegationView, CoreError> {
        if change == DelegationChange::Resume
            && !(self.holds_roster_now()? && self.decides() && self.store.permission_history_ready().unwrap_or(false))
        {
            return Err(CoreError::invalid(
                "resuming delegation requires verified primary authority and complete execution history",
            ));
        }
        let mut state = self.authorization_policies()?;
        if state.revision != expected_revision || state.conflicted || state.overflow_digest.is_some() {
            return Err(CoreError::invalid("reload and reconcile authorization before changing a delegation"));
        }
        let child_prefix = format!("delegation:{identity}:");
        let parent_prefix = format!("delegation-authority:{identity}:");
        let mut changed = false;
        for policy in &mut state.policies {
            if !policy.id.starts_with(&child_prefix) && !policy.id.starts_with(&parent_prefix) {
                continue;
            }
            if change == DelegationChange::Resume
                && (policy.lifecycle == PolicyLifecycle::Revoked || policy.expires_at.is_none_or(|e| unix_now() >= e))
            {
                return Err(CoreError::invalid(
                    "an ended or revoked delegation cannot be resumed; issue a new bounded delegation",
                ));
            }
            policy.lifecycle = match change {
                DelegationChange::Resume => PolicyLifecycle::Active,
                DelegationChange::Pause => PolicyLifecycle::Paused,
                DelegationChange::Revoke => PolicyLifecycle::Revoked,
            };
            policy.revision =
                policy.revision.checked_add(1).ok_or_else(|| CoreError::invalid("policy revision exhausted"))?;
            if change == DelegationChange::Revoke {
                state.revoked_ids.insert(policy.id.clone());
            }
            changed = true;
        }
        if !changed {
            return Err(CoreError::NotFound);
        }
        state.revision =
            state.revision.checked_add(1).ok_or_else(|| CoreError::invalid("policy revision exhausted"))?;
        if !owner
            .confirm(format!("{change:?} the selected delegation"))
            .await
            .map_err(|_| CoreError::invalid("the owner check failed"))?
        {
            return Err(CoreError::invalid("the owner did not confirm this delegation change"));
        }
        let fence = self.policy_mutation_fence().await?;
        self.require_decider_fresh(self.session()?.as_ref()).await?;
        if change == DelegationChange::Resume
            && state.policies.iter().any(|policy| {
                (policy.id.starts_with(&child_prefix) || policy.id.starts_with(&parent_prefix))
                    && policy.expires_at.is_none_or(|expiry| unix_now() >= expiry)
            })
        {
            return Err(CoreError::invalid("the delegation expired while awaiting confirmation"));
        }
        self.store.permissions_apply(expected_revision, &state, &[], &[])?;
        drop(fence);
        self.delegations()?.into_iter().find(|d| d.id == identity).ok_or(CoreError::NotFound)
    }
}
