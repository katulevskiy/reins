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
            "Gmail is not connected in Reins, or needs the user's permission again. Ask the user to connect Gmail in \
             the Reins app on their phone (Connections), then call the tool again. No new chat is needed."
                .to_owned()
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
        // A phone that only views the account (or was removed from its roster) decides nothing, whatever its grants
        // or Autopilot modes say: the request waits for the primary. The server would refuse its answer too.
        if !self.decides() {
            log::info!("a request reached a phone that does not decide for the account; it waits for the primary");
            return Ok(());
        }
        if check_version(request.v).is_err() {
            return self.reject(session, &request, "This version of the Reins app is too old for the server.").await;
        }
        // Defense in depth: the server normalizes too, but a call is never trusted as received.
        let Ok(call) = request.call.clone().normalized() else {
            return self.reject(session, &request, "The request was malformed.").await;
        };
        request.call = call.clone();
        // One mandatory preflight before EVERY request dispatcher, including accounts, uploads, desktop secrets,
        // payment-specialized calls and MCP. Invalid/unavailable policy is never treated as legacy absence.
        if let Err(error) = self.authorization_preflight(&request) {
            return self.reject(session, &request, &error.to_string()).await;
        }
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
        // Resolve/canonicalize the account before the provider is contacted too: a differently cased address or an
        // omitted account must not evade a restriction on the actual account used.
        if let Err(error) = self.authorization_preflight(&request) {
            return self.reject(session, &request, &error.to_string()).await;
        }
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
            self.authorization_preflight(request)?;
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
        self.authorization_preflight(request)?;
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
        self.decide_read(session, request, "search", &detail, parsed, true, |summaries, _| ToolResult::Search {
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
        self.decide_read(session, request, "read", &detail, parsed, false, |_, full| ToolResult::Read {
            messages: full,
        })
        .await
    }

    /// Shared tail of search and read: evaluate, then release or park. A search (`search`) may also be covered by a
    /// grant for Gmail's headlines (see `crate::starter`): what only that covers is released as its headline (no
    /// snippet). A read never is: opening a message always needs a read grant or the user.
    #[allow(clippy::too_many_arguments, reason = "the tail of two handlers, described by what differs between them")]
    async fn decide_read(
        &self,
        session: &Session,
        request: &RelayRequest,
        action: &str,
        detail: &str,
        parsed: Vec<ParsedMessage>,
        search: bool,
        build: impl FnOnce(Vec<MessageSummary>, Vec<reins_proto::gmail::MessageFull>) -> ToolResult,
    ) -> Result<(), CoreError> {
        let facts: Vec<MessageFacts> = parsed.iter().map(|p| p.facts.clone()).collect();
        // A login code or a password is never released by a grant: it always waits for the user's tick.
        let held: Vec<bool> = parsed.iter().map(|p| views::looks_sensitive(&p.summary)).collect();
        let fence = self.permission_execution_fence().await?;
        let reservation = if held.iter().any(|held| *held) && self.permission_managed(request)? {
            crate::permissions::PermissionReservation::Review
        } else {
            match self.permission_route(request) {
                Ok(reservation) => reservation,
                Err(error) => return self.reject(session, request, &error.to_string()).await,
            }
        };
        match reservation {
            crate::permissions::PermissionReservation::Authorized(proof) => {
                if let Err(error) = self.permission_durable(request, &proof).await {
                    self.permission_finished(request, proof, false)?;
                    return self.fail(session, request, action, &error).await;
                }
                let summaries: Vec<_> = parsed.iter().map(|message| message.summary.clone()).collect();
                let refs: Vec<_> = summaries.iter().collect();
                let audit = self.audit(request, action, "released", detail, None, summaries.len(), &refs);
                let full = parsed.into_iter().map(ParsedMessage::into_full).collect();
                let result = self
                    .finish_checked(
                        session,
                        request,
                        audit,
                        RelayOutcome::Result {
                            result: build(summaries, full),
                        },
                        || self.revalidate_permission(request, &proof),
                    )
                    .await;
                self.permission_finished(request, proof, result.is_ok())?;
                return result;
            }
            crate::permissions::PermissionReservation::Review => {
                return self.park_request(&ParkedRequest {
                    request: request.clone(),
                    messages: parsed.into_iter().map(|message| message.summary).collect(),
                    covered: BTreeMap::new(),
                    account: request.account.clone(),
                    accounts: vec![],
                    shared_accounts: vec![],
                    connector: None,
                });
            }
            crate::permissions::PermissionReservation::Legacy => {}
        }
        drop(fence);
        let (connection, account, now) = (&request.connection_id, request.account.as_deref(), unix_now());
        let (decision, headlines_only) = if search {
            self.store.evaluate_search_holding(connection, account, &facts, &held, now)?
        } else {
            (self.store.evaluate_read_holding(connection, account, &facts, &held, now)?, BTreeSet::new())
        };
        let mut summaries: Vec<MessageSummary> = parsed.iter().map(|p| p.summary.clone()).collect();
        if decision.fully_allowed() {
            self.authorization_preflight(request)?;
            for i in &headlines_only {
                if let Some(m) = summaries.get_mut(*i) {
                    crate::starter::strip_to_headline(m);
                }
            }
            let grant_id = decision.grants_used().into_iter().next().map(|g| g.0);
            let full = parsed.into_iter().map(ParsedMessage::into_full).collect();
            let refs: Vec<&MessageSummary> = summaries.iter().collect();
            let detail = if headlines_only.is_empty() {
                detail.to_owned()
            } else {
                format!("{detail} (headlines only)")
            };
            let mut audit = self.audit(request, action, "released", &detail, grant_id, summaries.len(), &refs);
            if !headlines_only.is_empty() {
                audit.info.note = Some(crate::starter::HEADLINES_NOTE.to_owned());
            }
            return self
                .finish_checked(
                    session,
                    request,
                    audit,
                    RelayOutcome::Result {
                        result: build(summaries, full),
                    },
                    || self.authorization_execute(request, crate::authorization::AuthorizationSource::Automatic),
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
        let fence = self.permission_execution_fence().await?;
        let reservation = match self.permission_route(request) {
            Ok(reservation) => reservation,
            Err(error) => return self.reject(session, request, &error.to_string()).await,
        };
        match reservation {
            crate::permissions::PermissionReservation::Authorized(proof) => {
                if let Err(error) = self.permission_durable(request, &proof).await {
                    self.permission_finished(request, proof, false)?;
                    return self.fail(session, request, "send", &error).await;
                }
                let sent = self
                    .gmail_for(account_of(request))
                    .send_checked(email, || self.permission_before_send(session, request, &proof))
                    .await;
                return if let Ok(sent) = sent {
                    if self.revalidate_permission(request, &proof).is_err() {
                        self.permission_finished(request, proof, true)?;
                        return self.result_withheld(session, request).await;
                    }
                    let audit =
                        self.audit(request, "send", "sent", &send_detail(email), None, email.recipients().count(), &[]);
                    let mut held = false;
                    let result = self
                        .finish_checked(
                            session,
                            request,
                            audit,
                            RelayOutcome::Result {
                                result: ToolResult::Sent(sent),
                            },
                            || {
                                let checked = self.revalidate_permission(request, &proof);
                                held |= checked.is_err();
                                checked
                            },
                        )
                        .await;
                    self.permission_finished(request, proof, true)?;
                    if held {
                        self.result_withheld(session, request).await
                    } else {
                        result
                    }
                } else {
                    self.permission_finished(request, proof, false)?;
                    self.send_outcome_unknown(session, request).await
                };
            }
            crate::permissions::PermissionReservation::Review => {
                return self.park_request(&ParkedRequest {
                    request: request.clone(),
                    messages: vec![],
                    covered: BTreeMap::new(),
                    account: request.account.clone(),
                    accounts: vec![],
                    shared_accounts: vec![],
                    connector: None,
                });
            }
            crate::permissions::PermissionReservation::Legacy => {}
        }
        drop(fence);
        // A mandatory review bypasses standing grants, retry passes and grants made during Bypass/Overnight.
        if !self.authorization_automatic(request)? {
            return self.park_request(&ParkedRequest {
                request: request.clone(),
                messages: Vec::new(),
                covered: BTreeMap::new(),
                account: request.account.clone(),
                accounts: Vec::new(),
                shared_accounts: Vec::new(),
                connector: None,
            });
        }
        match self.store.evaluate_send_and_reserve(
            &request.connection_id,
            request.account.as_deref(),
            email,
            unix_now(),
        )? {
            SendDecision::Allowed(grant) => {
                let _fence = self.permission_execution_fence().await?;
                let prepared = self
                    .authorization_execute(request, crate::authorization::AuthorizationSource::Automatic)
                    .and_then(|()| self.legacy_send_attempt(request));
                let attempt = match prepared {
                    Ok(attempt) => attempt,
                    Err(error) => {
                        self.store.refund(&BTreeSet::from([grant]))?;
                        return self.fail(session, request, "send", &error).await;
                    }
                };
                if let Err(error) = self.legacy_send_durable().await {
                    self.legacy_send_finished(attempt, false)?;
                    // Provider invocation has not begun; this invocation's legacy grant use can be returned.
                    self.store.refund(&BTreeSet::from([grant]))?;
                    return self.fail(session, request, "send", &error).await;
                }
                let sent = self
                    .gmail_for(account_of(request))
                    .send_checked(email, || {
                        self.legacy_before_send(session, request, crate::authorization::AuthorizationSource::Automatic)
                    })
                    .await;
                self.legacy_send_finished(attempt, sent.is_ok())?;
                match sent {
                    Ok(sent) => {
                        if self
                            .authorization_execute(request, crate::authorization::AuthorizationSource::Automatic)
                            .is_err()
                        {
                            return self.result_withheld(session, request).await;
                        }

                        let audit = self.audit(
                            request,
                            "send",
                            "sent",
                            &send_detail(email),
                            Some(grant.0),
                            email.recipients().count(),
                            &[],
                        );
                        let mut held = false;
                        let result = self
                            .finish_checked(
                                session,
                                request,
                                audit,
                                RelayOutcome::Result {
                                    result: ToolResult::Sent(sent),
                                },
                                || {
                                    let checked = self.authorization_execute(
                                        request,
                                        crate::authorization::AuthorizationSource::Automatic,
                                    );
                                    held |= checked.is_err();
                                    checked
                                },
                            )
                            .await;
                        if held {
                            self.result_withheld(session, request).await
                        } else {
                            result
                        }
                    }
                    Err(_) => self.send_outcome_unknown(session, request).await,
                }
            }
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

    pub(crate) async fn result_withheld(&self, session: &Session, request: &RelayRequest) -> Result<(), CoreError> {
        let message = crate::authorization_trace::RESULT_WITHHELD;
        let audit = self.audit(request, call_action(&request.call), "succeeded_withheld", message, None, 0, &[]);
        self.finish(
            session,
            request,
            audit,
            RelayOutcome::Error {
                message: message.into(),
            },
        )
        .await
    }

    pub(crate) async fn send_outcome_unknown(
        &self,
        session: &Session,
        request: &RelayRequest,
    ) -> Result<(), CoreError> {
        let message = "The send outcome is unknown. Gmail may have sent it. Its use was retained; check Sent before issuing a new request.";
        let audit = self.audit(request, "send", "unknown", message, None, 0, &[]);
        self.finish(
            session,
            request,
            audit,
            RelayOutcome::Error {
                message: message.into(),
            },
        )
        .await
    }

    /// An audit entry for `request`, tagged with the connector/account and carrying what the details screen shows.
    #[allow(clippy::too_many_arguments, reason = "an audit entry is described by this many independent facts")]
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
        // The request as its approval showed it: the parked one when it waited, else what it was decided on.
        let parked = self.parked(&request.id.0).map_or_else(
            |_| ParkedRequest {
                request: request.clone(),
                messages: released.iter().map(|m| (*m).clone()).collect(),
                covered: BTreeMap::default(),
                account: request.account.clone(),
                accounts: Vec::new(),
                shared_accounts: Vec::new(),
                connector: None,
            },
            |(_, p)| p,
        );
        let mut info = views::audit_info(&request.call, released);
        info.label = Some(Box::new(crate::titles::of_parked(&parked)));
        info.request = Some(Box::new(crate::titles::request(&request.call, request.created_at)));
        // A push's refs, commits and files travel with the entry to every device; the snapshot stays on this one.
        info.git = crate::store::AuditPush::of_call(&parked.request.call).map(Box::new);
        info.snapshot = Some(Box::new(parked));
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
            info,
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
        self.finish_checked(session, request, audit, outcome, || Ok(())).await
    }

    pub(crate) async fn finish_checked<Check>(
        &self,
        session: &Session,
        request: &RelayRequest,
        audit: AuditRecord,
        outcome: RelayOutcome,
        mut check: Check,
    ) -> Result<(), CoreError>
    where
        Check: FnMut() -> Result<(), CoreError> + Send,
    {
        let mut recorded = false;
        self.respond_checked(session, &request.id.0, outcome, || {
            check()?;
            if !recorded {
                self.store.append_audit(&audit)?;
                self.store.mark_handled(&request.id.0, unix_now())?;
                recorded = true;
            }
            Ok(())
        })
        .await
    }

    pub(crate) async fn respond(&self, session: &Session, id: &str, outcome: RelayOutcome) -> Result<(), CoreError> {
        self.respond_checked(session, id, outcome, || Ok(())).await
    }

    /// Runs the original authorization check after every token acquisition/refresh and immediately before each
    /// response HTTP attempt. Guard errors retain their original CoreError so callers can settle withheld results.
    pub(crate) async fn respond_checked<Check>(
        &self,
        session: &Session,
        id: &str,
        outcome: RelayOutcome,
        mut check: Check,
    ) -> Result<(), CoreError>
    where
        Check: FnMut() -> Result<(), CoreError> + Send,
    {
        // Every answer to a request goes through here: a phone that does not decide sends none.
        self.require_decider()?;
        let response = RelayResponse {
            v: PROTOCOL_VERSION,
            outcome,
        };
        // With a roster, the primary signs the answer's exact bytes (the server and desktops check it).
        let bytes = serde_json::to_vec(&response).map_err(|e| CoreError::storage(e.to_string()))?;
        let signed = self.signed_answer(session, reins_proto::answer::AnswerKind::Request, id, &bytes).await?;
        let sent = if let Some(signed) = &signed {
            api_call!(session, |api| async {
                self.require_decider().map_err(ApiFailure::Core)?;
                check().map_err(ApiFailure::Core)?;
                api.respond_body(id, signed).await
            })
        } else {
            api_call!(session, |api| async {
                self.require_decider().map_err(ApiFailure::Core)?;
                check().map_err(ApiFailure::Core)?;
                api.respond(id, &response).await
            })
        };
        match sent {
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
