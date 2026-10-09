//! How the engine handles a purchase. A request is worked out ([`Payments::plan`]), refused when a budget says so,
//! approved at once when a spend limit covers it (and Lockdown is off), else parked for the user, who sees it like a
//! receipt and may change the address and the payment method. A report of how checkout went is recorded without
//! asking. Standing permissions never cover a purchase and Autopilot never approves one (the hard floor).

use reins_proto::connector::ConnectorCall;
use reins_proto::payments::{PURCHASE_COMPLETE_OP, PURCHASE_REQUEST_OP};
use reins_proto::relay::{RelayOutcome, RelayRequest, ToolResult};

use super::{Approval, BY_USER, Plan, PurchaseChoice, limit_may_use};
use crate::autopilot::context::{Decision, deciding};
use crate::autopilot::{AutopilotMode, Verdict};
use crate::connector::flow::{DESKTOP_ONLY, action_of};
use crate::connector::sealed::{client_key, nonce_arg};
use crate::engine::Engine;
use crate::session::Session;
use crate::store::unix_now;
use crate::views::{ParkedConnector, ParkedRequest};
use crate::{CoreError, text};

impl Engine {
    /// The desktop app's key and nonce when a purchase asks for card details sealed to it (`reins mcp` does). A key
    /// for a connection with no desktop app pinned is ignored: the answer is what any AI gets. A key other than the one
    /// pinned is an error.
    fn seal_target(&self, request: &RelayRequest, call: &ConnectorCall) -> Result<Option<([u8; 32], String)>, ()> {
        if call.str_arg("client_key").is_none() {
            return Ok(None);
        }
        match self.store.desktop_key(&request.connection_id.0) {
            Ok(None) => Ok(None),
            Ok(Some(_)) if self.desktop_key_matches(&request.connection_id, call) => {
                match (client_key(call), nonce_arg(call)) {
                    (Ok(key), Ok(nonce)) => Ok(Some((key, nonce))),
                    _ => Err(()),
                }
            }
            _ => Err(()),
        }
    }

    /// A purchase request or a report from an AI.
    pub(crate) async fn payments_write(
        &self,
        session: &Session,
        request: &RelayRequest,
        call: &ConnectorCall,
    ) -> Result<(), CoreError> {
        let connection = request.connection_id.0.as_str();
        if call.op == PURCHASE_COMPLETE_OP {
            let result = {
                let one = self.payments.purchases.lock().await;
                self.payments.complete(&one, connection, call).await
            };
            return match result {
                Ok((data, detail)) => {
                    let audit = self.audit_connector(request, call, action_of(call), "sent", &detail, None, &[]);
                    self.finish(session, request, audit, connector_result(data)).await
                }
                Err(e) => self.fail_connector(session, request, call, &e).await,
            };
        }
        if call.op != PURCHASE_REQUEST_OP {
            return self.fail_connector_text(session, request, call, "Payments cannot do that.").await;
        }
        let Ok(seal_to) = self.seal_target(request, call) else {
            return self.fail_connector_text(session, request, call, DESKTOP_ONLY).await;
        };
        let now = unix_now();
        let one = self.payments.purchases.lock().await;
        // Lockdown stops spend limits too; the waiting request is then denied with everything else. Before a limit
        // may approve, what this AI's earlier cards were charged with is read again (and shown with the plan).
        let locked = self.ap_mode_for(connection, now)? == AutopilotMode::Lockdown;
        let limits = !locked && self.payments.limits_may_apply(&one, connection, now).await?;
        let plan = match self.payments.plan(call, connection, &request.connection_label).await {
            Ok(plan) => plan,
            Err(e) => return self.fail_connector(session, request, call, &e).await,
        };
        if let Some(why) = self.payments.refusal(&plan.cart, connection, now)? {
            drop(one);
            let detail = format!("refused by your budget: {}", plan.cart.summary());
            let audit = self.audit_connector(request, call, action_of(call), "denied", &detail, None, &[]);
            let reason = format!("Refused by the user's budget in Reins. {why}");
            return self
                .finish(
                    session,
                    request,
                    audit,
                    RelayOutcome::Denied {
                        reason: Some(reason),
                    },
                )
                .await;
        }
        if limits && let Some(limit) = self.payments.covering_limit(&plan, connection, now)? {
            let approval = Approval {
                purchase_id: &request.id.0,
                connection_id: connection,
                connection_label: &request.connection_label,
                approved_by: &limit.id,
                seal_to,
            };
            let done = self.payments.perform(&one, &plan, None, None, &approval).await;
            drop(one);
            return match done {
                Ok(data) => {
                    let detail = format!("approved by your spend limit: {}", plan.cart.summary());
                    let mut audit = self.audit_connector(request, call, action_of(call), "sent", &detail, None, &[]);
                    audit.info.note = Some(format!("Within the spend limit: {}", limit_summary(&limit)));
                    self.finish(session, request, audit, connector_result(data)).await
                }
                Err(e) => self.fail_connector(session, request, call, &e).await,
            };
        }
        drop(one);
        self.park_request(&ParkedRequest {
            request: request.clone(),
            messages: Vec::new(),
            covered: std::collections::BTreeMap::new(),
            account: request.account.clone(),
            accounts: Vec::new(),
            shared_accounts: Vec::new(),
            connector: Some(ParkedConnector {
                preview: Some(plan.preview()),
                purchase: Some(plan),
                ..ParkedConnector::default()
            }),
        })
    }

    /// The user approved a parked purchase (from the purchase screen with their picks, or a plain approval).
    pub(crate) async fn approve_parked_purchase(
        &self,
        session: &Session,
        request_id: &str,
        parked: &ParkedRequest,
        choice: Option<&PurchaseChoice>,
    ) -> Result<(), CoreError> {
        let reins_proto::gmail::ToolCall::Connector(call) = &parked.request.call else {
            return Err(CoreError::invalid("not a purchase"));
        };
        let plan: &Plan = parked
            .connector
            .as_ref()
            .and_then(|h| h.purchase.as_ref())
            .ok_or_else(|| CoreError::invalid("not a purchase"))?;
        let request = &parked.request;
        let Ok(seal_to) = self.seal_target(request, call) else {
            return Err(CoreError::invalid(
                "The desktop app that asked is no longer paired with this key. Deny this purchase.",
            ));
        };
        let now = unix_now();
        let connection = request.connection_id.0.as_str();
        // A limit to create alongside: checked before anything is paid.
        if let Some(limit) = choice.and_then(|c| c.limit.as_ref())
            && (!limit_may_use(&limit.method) || !plan.methods.iter().any(|m| m.id == limit.method))
        {
            return Err(CoreError::invalid(
                "A spend limit can pay with a virtual card only: its cap is enforced by the card.",
            ));
        }
        let approval = Approval {
            purchase_id: request_id,
            connection_id: connection,
            connection_label: &request.connection_label,
            approved_by: BY_USER,
            seal_to,
        };
        let data = {
            let one = self.payments.purchases.lock().await;
            // The budgets may have changed while it waited.
            if let Some(why) = self.payments.refusal(&plan.cart, connection, now)? {
                return Err(CoreError::invalid(format!("{why} Change the budget in Payments, or deny this.")));
            }
            self.payments
                .perform(
                    &one,
                    plan,
                    choice.and_then(|c| c.method_id.as_deref()),
                    choice.and_then(|c| c.address_id.as_deref()),
                    &approval,
                )
                .await?
        };
        let mut note = None;
        if let Some(limit) = choice.and_then(|c| c.limit.as_ref()) {
            match self.payments.add_limit(limit, connection, &request.connection_label, now) {
                Ok(made) => note = Some(format!("Spend limit added: {}", made.summary)),
                Err(e) => note = Some(format!("The spend limit was not added: {e}")),
            }
        }
        let detail = format!("approved: {}", plan.cart.summary());
        let mut audit = self.audit_connector(request, call, action_of(call), "sent", &detail, None, &[]);
        audit.info.note = note;
        self.settle(session, request_id, request, audit, connector_result(data)).await
    }

    /// The user approved a purchase on the purchase screen. Like [`Engine::approve`], with their picks.
    pub async fn approve_purchase(&self, request_id: &str, choice: PurchaseChoice) -> Result<(), CoreError> {
        let Some(_claim) = self.claim(request_id) else {
            return Err(CoreError::NotFound);
        };
        let decision = Decision::new("", self.ap_human_note(request_id));
        let work = async {
            let session = self.session()?;
            let (_, parked) = self.parked(request_id)?;
            self.approve_parked_purchase(&session, request_id, &parked, Some(&choice)).await
        };
        let (result, _) = deciding(decision, work).await;
        if result.is_ok() {
            self.ap_learn(request_id, Verdict::Approve);
        }
        result
    }
}

fn connector_result(data: serde_json::Value) -> RelayOutcome {
    RelayOutcome::Result {
        result: ToolResult::Connector {
            data,
        },
    }
}

fn limit_summary(limit: &super::settings::SpendLimit) -> String {
    let amount = |m: i64| reins_proto::payments::display_amount(m, &limit.currency);
    let at = if limit.merchants.is_empty() {
        "any store".to_owned()
    } else {
        limit.merchants.join(", ")
    };
    text::one_line(&format!(
        "up to {} a purchase and {} a {} at {at}",
        amount(limit.per_purchase),
        amount(limit.per_period),
        limit.period.word()
    ))
}
