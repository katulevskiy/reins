//! Processing of relayed requests: fetch what the AI asked for, evaluate it against
//! the grants (atomically reserving uses), then either answer at once or park the
//! request for the user.

use std::collections::{BTreeMap, BTreeSet};

use reins_policy::{MessageFacts, SendDecision};
use reins_proto::gmail::{MessageSummary, OutgoingEmail, ToolCall, normalize_account};
use reins_proto::pairing::PairingRequest;
use reins_proto::relay::{AccountInfo, IntegrationInfo, RelayOutcome, RelayRequest, RelayResponse, ToolResult};
use reins_proto::{PROTOCOL_VERSION, check_version};

use crate::connector::flow;
use crate::engine::Engine;
use crate::gmail::parse::ParsedMessage;
use crate::phone_api::ApiFailure;
use crate::session::{Session, api_call};
use crate::store::{AuditRecord, unix_now};
use crate::types::PendingKind;
use crate::views::{self, ParkedRequest};
use crate::{CoreError, text};

/// What the AI is told when the phone cannot reach Gmail (never a URL or token).
pub(crate) fn ai_message(e: &CoreError) -> String {
    match e {
        CoreError::GmailNeedsConsent => {
            "Gmail is not connected on the phone. Ask the user to open the Reins app and connect Gmail.".to_owned()
        }
        CoreError::Network {
            ..
        } => "The phone could not reach Gmail. Try again in a moment.".to_owned(),
        other => format!("Gmail could not complete the request: {other}"),
    }
}

impl Engine {
    /// Processes a request received by long-poll (deduplicates against push).
    pub(crate) async fn process_request(&self, session: &Session, request: RelayRequest) -> Result<(), CoreError> {
        let Some(_guard) = self.begin(&request.id.0)? else {
            return Ok(());
        };
        self.process_request_unguarded(session, request).await
    }

    /// Processes a request the caller already claimed with `begin`.
    pub(crate) async fn process_request_unguarded(
        &self,
        session: &Session,
        mut request: RelayRequest,
    ) -> Result<(), CoreError> {
        if check_version(request.v).is_err() {
            return self.reject(session, &request, "This version of the Reins app is too old for the server.").await;
        }
        // Defense in depth: the server normalizes too, but a call is never trusted as received.
        let Ok(call) = request.call.clone().normalized() else {
            return self.reject(session, &request, "The request was malformed.").await;
        };
        request.call = call.clone();
        if let ToolCall::Mcp(mcp_call) = &call {
            return self.handle_mcp(session, request.clone(), mcp_call).await;
        }
        // A file the AI wants to pass on: an upload link at once, the user decides when the file arrives.
        if matches!(call, ToolCall::RequestUpload { .. }) {
            return self.handle_request_upload(session, &request).await;
        }
        if let ToolCall::Connector(connector_call) = &call {
            let scope = self.blob_scope();
            return crate::blob::within(scope, self.handle_connector(session, request.clone(), connector_call)).await;
        }
        if let ToolCall::ListAccounts {
            service,
            ask_for_more,
        } = &call
        {
            return self.handle_list_accounts(session, &request, service.as_deref(), *ask_for_more).await;
        }
        // Defense in depth again: the account is an address the server already checked.
        let Ok(named) = request.account.as_deref().map(normalize_account).transpose() else {
            return self.reject(session, &request, "The account was malformed.").await;
        };
        request.account = match self.resolve_account(named.as_deref()).await {
            Ok(account) => Some(account),
            Err(message) => {
                request.account = named;
                return self.fail_with(session, &request, call_action(&call), &message).await;
            }
        };
        match call {
            ToolCall::GmailSearch {
                query,
                max_results,
            } => self.handle_search(session, &request, &query, max_results).await,
            ToolCall::GmailRead {
                message_ids,
            } => self.handle_read(session, &request, &message_ids).await,
            ToolCall::GmailSend {
                email,
            } => self.handle_send(session, &request, &email).await,
            // Never auto-answered: a permission is only ever created by the user's decision.
            ToolCall::RequestGrant {
                ..
            } => self.park_request(&ParkedRequest {
                request: request.clone(),
                messages: Vec::new(),
                covered: BTreeMap::default(),
                account: request.account.clone(),
                accounts: Vec::new(),
                shared_accounts: Vec::new(),
                connector: None,
            }),
            ToolCall::ListAccounts {
                ..
            }
            | ToolCall::Connector(_)
            | ToolCall::Mcp(_)
            | ToolCall::RequestUpload {
                ..
            } => Ok(()),
        }
    }

    /// Without a service: which integrations the user has (no accounts, no approval). With one: its accounts, but only
    /// once the user allowed it (a grant, or a decision on the spot).
    async fn handle_list_accounts(
        &self,
        session: &Session,
        request: &RelayRequest,
        service: Option<&str>,
        ask_for_more: bool,
    ) -> Result<(), CoreError> {
        if self.gmail_accounts().is_empty() {
            // Registers the phone's default Google account when it was connected before accounts were tracked.
            self.resolve_account(None).await.ok();
        }
        let Some(service) = service else {
            let integrations: Vec<IntegrationInfo> = self
                .integrations()
                .into_iter()
                .map(|service| IntegrationInfo {
                    name: views::service_name(&service).to_owned(),
                    service,
                })
                .collect();
            let detail = format!("listed {}", plural(integrations.len(), "integration", "integrations"));
            let audit = self.audit(request, "accounts", "released", &detail, None, integrations.len(), &[]);
            return self
                .finish(
                    session,
                    request,
                    audit,
                    RelayOutcome::Result {
                        result: ToolResult::Integrations {
                            integrations,
                        },
                    },
                )
                .await;
        };
        let accounts = self.accounts_of(service);
        if accounts.is_empty() {
            let message = "That integration is not connected on the user's phone.";
            return self.fail_with(session, request, "accounts", message).await;
        }
        // What this AI may already see; the rest is withheld until the user says otherwise.
        let coverage = self.store.account_coverage(&request.connection_id, service, unix_now())?;
        let shared: Vec<String> = accounts.iter().filter(|a| coverage.covers(a)).cloned().collect();
        let withheld = accounts.len() - shared.len();
        if !shared.is_empty() && (withheld == 0 || !ask_for_more) {
            let used = coverage
                .grants
                .iter()
                .filter(|(_, named)| named.is_empty() || named.iter().any(|a| shared.contains(a)))
                .map(|(id, _)| id.clone())
                .collect();
            self.store.consume_active(&used, unix_now())?;
            let grant = used_first(&coverage, &shared);
            return self.share_accounts(session, request, service, shared, withheld, grant).await;
        }
        self.park_request(&ParkedRequest {
            request: request.clone(),
            messages: Vec::new(),
            covered: BTreeMap::default(),
            account: None,
            accounts,
            shared_accounts: shared,
            connector: None,
        })
    }

    /// Answers an accounts request and logs exactly which accounts were shown.
    pub(crate) async fn share_accounts(
        &self,
        session: &Session,
        request: &RelayRequest,
        service: &str,
        accounts: Vec<String>,
        withheld: usize,
        grant_id: Option<String>,
    ) -> Result<(), CoreError> {
        let mut detail =
            format!("shared {} of {}", plural(accounts.len(), "account", "accounts"), views::service_name(service));
        if withheld > 0 {
            detail = format!("{detail}, {withheld} kept private");
        }
        let mut audit = self.audit(request, "accounts", "released", &detail, grant_id, accounts.len(), &[]);
        audit.service = service.to_owned();
        audit.info.accounts.clone_from(&accounts);
        let infos = accounts
            .into_iter()
            .map(|account| AccountInfo {
                service: service.to_owned(),
                account,
            })
            .collect();
        self.finish(
            session,
            request,
            audit,
            RelayOutcome::Result {
                result: ToolResult::Accounts {
                    accounts: infos,
                    withheld: u32::try_from(withheld).unwrap_or(u32::MAX),
                },
            },
        )
        .await
    }

    async fn handle_search(
        &self,
        session: &Session,
        request: &RelayRequest,
        query: &str,
        max_results: u32,
    ) -> Result<(), CoreError> {
        let now = unix_now();
        let gmail = self.gmail_for(account_of(request));
        let full = self.store.needs_body(&request.connection_id, request.account.as_deref(), now)?;
        let parsed = match gmail.list(query, max_results).await {
            Ok(ids) => gmail.fetch(&ids, full).await,
            Err(e) => Err(e),
        };
        let parsed = match parsed {
            Ok(p) => p,
            Err(e) => return self.fail(session, request, "search", &e).await,
        };
        let detail = format!("{} for \"{}\"", plural_messages(parsed.len()), text::one_line(query));
        self.decide_read(session, request, "search", &detail, parsed, |summaries, _| ToolResult::Search {
            messages: summaries,
        })
        .await
    }

    async fn handle_read(&self, session: &Session, request: &RelayRequest, ids: &[String]) -> Result<(), CoreError> {
        let gmail = self.gmail_for(account_of(request));
        let parsed = match gmail.fetch(ids, true).await {
            Ok(p) => p,
            Err(e) => return self.fail(session, request, "read", &e).await,
        };
        if parsed.is_empty() {
            // Nothing to evaluate: an empty set is trivially "allowed", which would answer with an empty read.
            let message = if ids.len() == 1 {
                "That message was not found in Gmail."
            } else {
                "None of those messages were found in Gmail."
            };
            return self.fail_with(session, request, "read", message).await;
        }
        let detail = format!("{} read", plural_messages(parsed.len()));
        self.decide_read(session, request, "read", &detail, parsed, |_, full| ToolResult::Read {
            messages: full,
        })
        .await
    }

    /// Shared tail of search and read: evaluate, then release or park.
    async fn decide_read(
        &self,
        session: &Session,
        request: &RelayRequest,
        action: &str,
        detail: &str,
        parsed: Vec<ParsedMessage>,
        build: impl FnOnce(Vec<MessageSummary>, Vec<reins_proto::gmail::MessageFull>) -> ToolResult,
    ) -> Result<(), CoreError> {
        let facts: Vec<MessageFacts> = parsed.iter().map(|p| p.facts.clone()).collect();
        // A login code or a password is never released by a grant: it always waits for the user's tick.
        let held: Vec<bool> = parsed.iter().map(|p| views::looks_sensitive(&p.summary)).collect();
        let decision = self.store.evaluate_read_holding(
            &request.connection_id,
            request.account.as_deref(),
            &facts,
            &held,
            unix_now(),
        )?;
        let summaries: Vec<MessageSummary> = parsed.iter().map(|p| p.summary.clone()).collect();
        if decision.fully_allowed() {
            let grant_id = decision.grants_used().into_iter().next().map(|g| g.0);
            let full = parsed.into_iter().map(ParsedMessage::into_full).collect();
            let refs: Vec<&MessageSummary> = summaries.iter().collect();
            let audit = self.audit(request, action, "released", detail, grant_id, summaries.len(), &refs);
            return self
                .finish(
                    session,
                    request,
                    audit,
                    RelayOutcome::Result {
                        result: build(summaries, full),
                    },
                )
                .await;
        }
        let covered = decision.allowed.iter().map(|(i, g)| (summaries[*i].id.clone(), g.0.clone())).collect();
        self.park_request(&ParkedRequest {
            request: request.clone(),
            messages: summaries,
            covered,
            account: request.account.clone(),
            accounts: Vec::new(),
            shared_accounts: Vec::new(),
            connector: None,
        })
    }

    async fn handle_send(
        &self,
        session: &Session,
        request: &RelayRequest,
        email: &OutgoingEmail,
    ) -> Result<(), CoreError> {
        match self.store.evaluate_send_and_reserve(
            &request.connection_id,
            request.account.as_deref(),
            email,
            unix_now(),
        )? {
            SendDecision::Allowed(grant) => match self.gmail_for(account_of(request)).send(email).await {
                Ok(sent) => {
                    let audit = self.audit(
                        request,
                        "send",
                        "sent",
                        &send_detail(email),
                        Some(grant.0),
                        email.recipients().count(),
                        &[],
                    );
                    self.finish(
                        session,
                        request,
                        audit,
                        RelayOutcome::Result {
                            result: ToolResult::Sent(sent),
                        },
                    )
                    .await
                }
                Err(e) => {
                    // Nothing was sent: give the reserved use back.
                    self.store.refund(&BTreeSet::from([grant]))?;
                    self.fail(session, request, "send", &e).await
                }
            },
            SendDecision::NeedsApproval => self.park_request(&ParkedRequest {
                request: request.clone(),
                messages: Vec::new(),
                covered: BTreeMap::default(),
                account: request.account.clone(),
                accounts: Vec::new(),
                shared_accounts: Vec::new(),
                connector: None,
            }),
        }
    }

    /// An audit entry for `request`, tagged with the connector/account and carrying what the details screen shows.
    #[allow(clippy::too_many_arguments, reason = "an audit entry is described by this many independent facts")]
    #[allow(clippy::unused_self, reason = "kept a method beside finish and fail, which it is always used with")]
    pub(crate) fn audit(
        &self,
        request: &RelayRequest,
        action: &str,
        outcome: &str,
        detail: &str,
        grant_id: Option<String>,
        count: usize,
        released: &[&MessageSummary],
    ) -> AuditRecord {
        let (service, account) = (views::SERVICE_GMAIL.to_owned(), request.account.clone());
        AuditRecord {
            seq: 0,
            at: unix_now(),
            connection_id: request.connection_id.0.clone(),
            connection_label: text::one_line(&request.connection_label),
            action: action.to_owned(),
            outcome: outcome.to_owned(),
            detail: detail.to_owned(),
            grant_id,
            service,
            account,
            count: u32::try_from(count).unwrap_or(u32::MAX),
            info: views::audit_info(&request.call, released),
            op: String::new(),
        }
    }

    /// Records the audit entry and the handled id, then answers the server.
    pub(crate) async fn finish(
        &self,
        session: &Session,
        request: &RelayRequest,
        audit: AuditRecord,
        outcome: RelayOutcome,
    ) -> Result<(), CoreError> {
        self.store.append_audit(&audit)?;
        self.store.mark_handled(&request.id.0, unix_now())?;
        self.respond(session, &request.id.0, outcome).await
    }

    pub(crate) async fn respond(&self, session: &Session, id: &str, outcome: RelayOutcome) -> Result<(), CoreError> {
        let response = RelayResponse {
            v: PROTOCOL_VERSION,
            outcome,
        };
        match api_call!(session, |api| api.respond(id, &response)) {
            // Ok, or expired on the server / answered already: nothing more to do.
            Ok(())
            | Err(ApiFailure::Status {
                status: 404 | 409,
                ..
            }) => Ok(()),
            Err(e) => Err(e.into_core()),
        }
    }

    async fn fail_with(
        &self,
        session: &Session,
        request: &RelayRequest,
        action: &str,
        message: &str,
    ) -> Result<(), CoreError> {
        let audit = self.audit(request, action, "error", message, None, views::requested_count(&request.call), &[]);
        self.finish(
            session,
            request,
            audit,
            RelayOutcome::Error {
                message: message.to_owned(),
            },
        )
        .await
    }

    async fn fail(
        &self,
        session: &Session,
        request: &RelayRequest,
        action: &str,
        e: &CoreError,
    ) -> Result<(), CoreError> {
        // The entry still says how many emails were asked for, never "0".
        let audit =
            self.audit(request, action, "error", &e.to_string(), None, views::requested_count(&request.call), &[]);
        self.finish(
            session,
            request,
            audit,
            RelayOutcome::Error {
                message: ai_message(e),
            },
        )
        .await
    }

    pub(crate) async fn reject(
        &self,
        session: &Session,
        request: &RelayRequest,
        message: &str,
    ) -> Result<(), CoreError> {
        let audit = self.audit(request, "request", "error", message, None, views::requested_count(&request.call), &[]);
        self.finish(
            session,
            request,
            audit,
            RelayOutcome::Error {
                message: message.to_owned(),
            },
        )
        .await
    }

    /// Parks a request for the user. The app is told by Autopilot's pass, which runs after every push and sync and
    /// may decide it instead.
    pub(crate) fn park_request(&self, parked: &ParkedRequest) -> Result<(), CoreError> {
        let payload = serde_json::to_vec(parked).map_err(|e| CoreError::storage(e.to_string()))?;
        let request = &parked.request;
        self.store.park(&request.id.0, PendingKind::Request, request.created_at, unix_now(), &payload)?;
        Ok(())
    }

    /// Validates, cleans and parks a pairing request (always left to the user; the pass tells the app).
    pub(crate) fn park_pairing(&self, pairing: PairingRequest) -> Result<(), CoreError> {
        check_version(pairing.v).map_err(|e| CoreError::invalid(e.to_string()))?;
        let pairing = views::clean_pairing(pairing);
        let payload = serde_json::to_vec(&pairing).map_err(|e| CoreError::storage(e.to_string()))?;
        self.store.park(&pairing.id.0, PendingKind::Pairing, pairing.created_at, unix_now(), &payload)?;
        Ok(())
    }
}

/// The account a resolved request acts as (empty only for requests that never got that far).
pub(crate) fn account_of(request: &RelayRequest) -> &str {
    request.account.as_deref().unwrap_or_default()
}

/// "search" | "read" | "send" | "grant" | "accounts": what a call is, for the activity log.
fn call_action(call: &ToolCall) -> &'static str {
    match call {
        ToolCall::GmailSearch {
            ..
        } => "search",
        ToolCall::GmailRead {
            ..
        } => "read",
        ToolCall::GmailSend {
            ..
        } => "send",
        ToolCall::RequestGrant {
            ..
        } => "grant",
        ToolCall::ListAccounts {
            ..
        } => "accounts",
        ToolCall::Connector(call) => flow::action_of(call),
        ToolCall::Mcp(_) => "write",
        ToolCall::RequestUpload {
            ..
        } => "upload",
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

/// The grant to credit an answer to: the first one that named a shared account (or all of them).
fn used_first(coverage: &reins_policy::AccountCoverage, shared: &[String]) -> Option<String> {
    coverage
        .grants
        .iter()
        .find(|(_, named)| named.is_empty() || named.iter().any(|a| shared.contains(a)))
        .map(|(id, _)| id.0.clone())
}

fn plural_messages(n: usize) -> String {
    plural(n, "message", "messages")
}

pub(crate) fn send_detail(email: &OutgoingEmail) -> String {
    format!("to {}: {}", email.to.join(", "), text::one_line(&email.subject))
}
