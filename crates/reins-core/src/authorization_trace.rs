//! Captured evidence of the narrowing gate, never a serialized execution permission. The exact policy snapshot is
//! recorded before a provider call, and carried forward; results never reconstruct a decision from newer policy.

use std::sync::atomic::{AtomicBool, Ordering};

use reins_proto::authorization::{AttributeKey, EvidenceSource, Fact, ReviewRequirement};
use reins_proto::relay::RelayOutcome;
use reins_proto::relay::RelayRequest;
use serde::{Deserialize, Serialize};

use crate::CoreError;
use crate::authorization::{AuthorizationSource, verify_decision};
use crate::engine::Engine;
use crate::store::{AuditRecord, unix_now};

/// Explicit local lifetime for safe protocol retries of the same invocation. It cannot be deserialized, cloned or
/// used by another request; a newly resumed approval must pass the durable barrier again and will be refused.
pub(crate) struct AuthorizationInvocation<'a> {
    pub request: &'a RelayRequest,
    pub source: AuthorizationSource,
    started: AtomicBool,
}

impl<'a> AuthorizationInvocation<'a> {
    pub(crate) fn new(request: &'a RelayRequest, source: AuthorizationSource) -> Self {
        Self {
            request,
            source,
            started: AtomicBool::new(false),
        }
    }

    pub(crate) fn begin(&self, engine: &Engine) -> Result<(), CoreError> {
        engine.authorization_execute(self.request, self.source)?;
        if !self.started.load(Ordering::Acquire) {
            if !engine.store.start_invocation(&self.request.id.0, unix_now())? {
                return Err(CoreError::invalid(
                    "This request was already attempted. Check Activity and the provider; do not repeat it.",
                ));
            }
            self.started.store(true, Ordering::Release);
        }
        Ok(())
    }
}

pub(crate) const RESULT_WITHHELD: &str = "The provider reported success, but its result was withheld because the account policy changed. Do not repeat this action. Check Activity and the provider.";
pub(crate) const OUTCOME_UNKNOWN: &str = "The action was attempted, but its outcome is unknown. It may already have happened. Do not repeat this request; check Activity and the provider before creating a new request.";

/// What is independently known before response delivery. This is local bookkeeping, never an authority proof.
#[derive(Clone, Copy)]
pub(crate) enum ResponseEffect {
    Read,
    Succeeded,
    Unknown,
}

pub(crate) struct AuthorizedResponse {
    pub source: AuthorizationSource,
    pub effect: ResponseEffect,
    pub settle_pending: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorizationReviewSource {
    Automatic,
    Owner,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorizationPhase {
    /// The gate passed and an external call is being attempted; no outcome is known yet.
    Started,
    /// The provider returned success. This is not proof of independent effects beyond its response.
    Succeeded,
    /// The call failed after the final boundary. It may already have acted; consumption must not be refunded.
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorizationPolicyVersion {
    pub id: String,
    pub revision: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorizationFactSource {
    pub key: AttributeKey,
    pub source: EvidenceSource,
    pub observed_at: i64,
    pub valid_until: i64,
}

/// Encrypted audit-only evidence, with no attribute values or provider content. Deserializing this record confers
/// no authority; all execution paths must freshly check actual policy and account admission.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorizationTrace {
    pub v: u32,
    pub attempt_id: String,
    pub request_id: String,
    pub request_digest: String,
    pub policy_state_revision: u64,
    pub policy_state_digest: String,
    pub policies: Vec<AuthorizationPolicyVersion>,
    pub determining_policy_ids: Vec<String>,
    pub required_reviews: Vec<ReviewRequirement>,
    pub source: AuthorizationReviewSource,
    pub verified_facts: Vec<AuthorizationFactSource>,
    pub checked_at: i64,
    pub phase: AuthorizationPhase,
}

impl AuthorizationTrace {
    pub(crate) fn stamp(&self, audit: &mut AuditRecord, phase: AuthorizationPhase) {
        let mut trace = self.clone();
        trace.phase = phase;
        audit.info.authorization = Some(trace);
    }
}

impl Engine {
    /// Recheck after each session-token await, before sending any result. A policy hold gets only a fixed safe
    /// response, with the provider's actual outcome retained separately from whether its result was disclosed.
    pub(crate) async fn finish_authorized(
        &self,
        session: &crate::session::Session,
        request: &RelayRequest,
        mut audit: AuditRecord,
        outcome: RelayOutcome,
        context: AuthorizedResponse,
    ) -> Result<(), CoreError> {
        let mut blocked = false;
        let mut recorded = false;
        let result = self
            .respond_checked(session, &request.id.0, outcome, || {
                if let Err(error) = self.authorization_execute(request, context.source) {
                    blocked = true;
                    return Err(error);
                }
                if !recorded {
                    self.store.append_audit(&audit)?;
                    self.store.mark_handled(&request.id.0, unix_now())?;
                    recorded = true;
                }
                Ok(())
            })
            .await;
        let result = if blocked {
            let (status, message, phase) = match context.effect {
                ResponseEffect::Succeeded => ("succeeded_withheld", RESULT_WITHHELD, AuthorizationPhase::Succeeded),
                ResponseEffect::Read | ResponseEffect::Unknown => {
                    ("unknown", OUTCOME_UNKNOWN, AuthorizationPhase::Unknown)
                }
            };
            status.clone_into(&mut audit.outcome);
            message.clone_into(&mut audit.detail);
            audit.count = 0;
            audit.info.messages.clear();
            audit.info.note = Some(message.to_owned());
            if let Some(trace) = &mut audit.info.authorization {
                trace.phase = phase;
            }
            self.finish(
                session,
                request,
                audit,
                RelayOutcome::Error {
                    message: message.to_owned(),
                },
            )
            .await
        } else {
            // Token acquisition itself can fail after an effect. Keep the known outcome even when no response
            // attempt got as far as its guard; never make the original invocation executable again.
            if !recorded {
                self.store.append_audit(&audit)?;
                self.store.mark_handled(&request.id.0, unix_now())?;
            }
            result
        };
        if context.settle_pending {
            self.store.remove_pending(&request.id.0)?;
            self.notifier.item_resolved(request.id.0.clone());
        }
        result
    }

    /// Local audit evidence for an activity row. This is deliberately not an execution permission or agent tool.
    pub fn activity_authorization(&self, entry_id: i64) -> Result<Option<AuthorizationTrace>, CoreError> {
        Ok(self.store.audit_entry(entry_id)?.and_then(|record| record.info.authorization))
    }

    /// The final gate and its evidence come from the same immutable snapshot. The started event is persisted before
    /// invocation. A crash leaves an honest started/unknown attempt rather than falsely reporting success.
    pub(crate) fn authorization_attempt_started(
        &self,
        request: &RelayRequest,
        source: AuthorizationSource,
    ) -> Result<Option<AuthorizationTrace>, CoreError> {
        let Some(snapshot) = self.authorization_snapshot(request)? else {
            return Ok(None);
        };
        let crate::authorization::NarrowingAuthorizationSnapshot {
            decision,
            state,
            normalized,
        } = snapshot;
        verify_decision(&request.call, source, &decision)?;
        let checked_at = unix_now();
        let mut policies: Vec<_> = state
            .policies
            .iter()
            .map(|p| AuthorizationPolicyVersion {
                id: p.id.clone(),
                revision: p.revision,
            })
            .collect();
        policies.sort_by(|a, b| a.id.cmp(&b.id));
        let state_bytes = serde_json::to_vec(&state).map_err(|e| CoreError::storage(e.to_string()))?;
        let trace = AuthorizationTrace {
            v: 1,
            attempt_id: uuid::Uuid::new_v4().to_string(),
            request_id: request.id.0.clone(),
            request_digest: decision.request_digest,
            policy_state_revision: state.revision,
            policy_state_digest: reins_proto::canon::hex32(&reins_proto::canon::sha256(&state_bytes)),
            policies,
            determining_policy_ids: decision.determining_policies,
            required_reviews: decision.required_reviews,
            source: match source {
                AuthorizationSource::Automatic => AuthorizationReviewSource::Automatic,
                AuthorizationSource::OwnerReview => AuthorizationReviewSource::Owner,
            },
            verified_facts: normalized
                .action
                .facts
                .iter()
                .filter_map(|fact| match &fact.fact {
                    Fact::Known {
                        evidence,
                        ..
                    } if normalized.verified_attribute_keys.contains(&fact.key) => Some(AuthorizationFactSource {
                        key: fact.key,
                        source: evidence.source,
                        observed_at: evidence.observed_at,
                        valid_until: evidence.valid_until,
                    }),
                    _ => None,
                })
                .collect(),
            checked_at,
            phase: AuthorizationPhase::Started,
        };
        self.authorization_attempt_event(request, &trace, AuthorizationPhase::Started)?;
        Ok(Some(trace))
    }

    pub(crate) fn authorization_disclosure_check(
        &self,
        request: &RelayRequest,
        source: AuthorizationSource,
        trace: Option<&AuthorizationTrace>,
    ) -> Result<(), CoreError> {
        if let Err(error) = self.authorization_execute(request, source) {
            if let Some(trace) = trace {
                self.authorization_attempt_event(request, trace, AuthorizationPhase::Unknown)?;
            }
            return Err(error);
        }
        Ok(())
    }

    pub(crate) fn authorization_attempt_event(
        &self,
        request: &RelayRequest,
        trace: &AuthorizationTrace,
        phase: AuthorizationPhase,
    ) -> Result<(), CoreError> {
        let (outcome, detail) = match phase {
            AuthorizationPhase::Started => (
                "started",
                "Policy checked; external action or disclosure is being attempted. Outcome is not yet known.",
            ),
            AuthorizationPhase::Succeeded => {
                ("sent", "The provider returned success for this policy-authorized attempt.")
            }
            AuthorizationPhase::Unknown => {
                ("unknown", "The attempt did not return success. It may already have acted; check before trying again.")
            }
        };
        let mut audit = self.audit(request, "execute", outcome, detail, None, 1, &[]);
        match &request.call {
            reins_proto::gmail::ToolCall::Connector(call) => {
                audit.service.clone_from(&call.service);
                audit.op.clone_from(&call.op);
            }
            reins_proto::gmail::ToolCall::Mcp(call) => {
                audit.service = crate::mcp::service_of(&call.server);
                audit.op.clone_from(&call.tool);
            }
            _ => {}
        }
        trace.stamp(&mut audit, phase);
        self.store.append_audit(&audit)?;
        Ok(())
    }
}
