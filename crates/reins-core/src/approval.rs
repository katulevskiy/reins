//! What happens when the user decides: approve or deny a parked request (optionally
//! creating a standing grant), and answer an AI-connection pairing.

use std::collections::BTreeSet;

use reins_policy::{AddrRule, Grant, Pattern, ReadScope, Scope, SendScope};
use reins_proto::PROTOCOL_VERSION;
use reins_proto::gmail::{GrantAction, GrantRequest, MessageSummary, ToolCall};
use reins_proto::ids::{ConnectionId, GrantId};
use reins_proto::pairing::{PairingRequest, PairingResponse, normalize_user_code};
use reins_proto::relay::{RelayOutcome, ToolResult};

use crate::autopilot::Verdict;
use crate::autopilot::context::{Decision, deciding};
use crate::engine::Engine;
use crate::gmail::parse::ParsedMessage;
use crate::handler::{self, send_detail};
use crate::phone_api::ApiFailure;
use crate::phone_api::check_id;
use crate::session::api_call;
use crate::store::{AuditInfo, AuditRecord, PendingRow, unix_now};
use crate::types::{ApprovalChoice, ApprovalKind, GrantScopeChoice, PairingView, PendingKind, StandingGrant};
use crate::views::{self, ParkedRequest};
use crate::{CoreError, text};

fn policy_error(e: impl std::fmt::Display) -> CoreError {
    CoreError::invalid(e.to_string())
}

fn address_rules(addresses: &[String], domains: &[String]) -> Result<Vec<AddrRule>, CoreError> {
    let mut rules = Vec::new();
    for a in addresses {
        rules.push(AddrRule::exact(a).map_err(policy_error)?);
    }
    for d in domains {
        rules.push(AddrRule::domain(d).map_err(policy_error)?);
    }
    Ok(rules)
}

fn subject_pattern(scope: &GrantScopeChoice) -> Result<Option<Pattern>, CoreError> {
    scope
        .subject_pattern
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| Pattern::literal(s).map_err(policy_error))
        .transpose()
}

/// Builds the grant a user asked for alongside an approval. Pure and strict: an
/// unconstrained read scope or a send scope without recipients is refused.
pub fn build_standing_grant(
    standing: &StandingGrant,
    kind: ApprovalKind,
    selected: &[String],
    connection: &ConnectionId,
    now: i64,
) -> Result<Grant, CoreError> {
    let expires_at = match standing.duration_secs {
        None => None,
        Some(0) => return Err(CoreError::invalid("the grant duration must be longer than zero")),
        Some(secs) => Some(now.saturating_add(i64::try_from(secs).unwrap_or(i64::MAX))),
    };
    let scope = match kind {
        ApprovalKind::Grant | ApprovalKind::Accounts | ApprovalKind::Fetch | ApprovalKind::Write => {
            return Err(CoreError::invalid("a permission request is not a standing grant"));
        }
        ApprovalKind::Search | ApprovalKind::Read => {
            let s = &standing.scope;
            if s.all_mail
                && (s.selected_messages_only
                    || !s.sender_addresses.is_empty()
                    || !s.sender_domains.is_empty()
                    || s.subject_pattern.as_deref().is_some_and(|p| !p.trim().is_empty()))
            {
                return Err(CoreError::invalid("allowing all mail cannot be combined with other limits"));
            }
            let scope = ReadScope {
                any: s.all_mail,
                message_ids: s.selected_messages_only.then(|| selected.iter().cloned().collect()),
                from: address_rules(&s.sender_addresses, &s.sender_domains)?,
                subject: subject_pattern(s)?,
                ..ReadScope::default()
            };
            Scope::Read(scope)
        }
        ApprovalKind::Send => {
            let s = &standing.scope;
            if s.all_mail {
                return Err(CoreError::invalid("sending cannot be allowed for all recipients"));
            }
            Scope::Send(SendScope {
                recipients: address_rules(&s.recipient_addresses, &s.recipient_domains)?,
                subject: subject_pattern(s)?,
                body: None,
            })
        }
    };
    Grant::new(GrantId(uuid::Uuid::new_v4().to_string()), connection.clone(), scope, now, expires_at, standing.max_uses)
        .map_err(policy_error)
}

/// How long a one-time pass lasts after the user approved a request the AI had already stopped waiting for.
const RETRY_PASS_SECS: i64 = 15 * 60;

fn rule_from(entry: &str) -> Result<AddrRule, CoreError> {
    match entry.strip_prefix('@') {
        Some(domain) => AddrRule::domain(domain),
        None => AddrRule::exact(entry),
    }
    .map_err(policy_error)
}

/// Builds the permission an AI asked for. `shorten_to` lets the user only ever make it shorter.
pub fn grant_from_request(
    request: &GrantRequest,
    connection: &ConnectionId,
    now: i64,
    shorten_to: Option<u64>,
) -> Result<Grant, CoreError> {
    let secs = shorten_to.map_or(request.duration_secs, |s| s.min(request.duration_secs));
    let subject = request.subject_contains.as_deref().map(Pattern::literal).transpose().map_err(policy_error)?;
    let scope = match request.action {
        GrantAction::Read => Scope::Read(ReadScope {
            any: request.any,
            from: request.from.iter().map(|r| rule_from(r)).collect::<Result<_, _>>()?,
            subject,
            ..ReadScope::default()
        }),
        GrantAction::Send => Scope::Send(SendScope {
            recipients: request.recipients.iter().map(|r| rule_from(r)).collect::<Result<_, _>>()?,
            subject,
            body: None,
        }),
    };
    let expires = now.saturating_add(i64::try_from(secs).unwrap_or(i64::MAX));
    Grant::new(
        GrantId(uuid::Uuid::new_v4().to_string()),
        connection.clone(),
        scope,
        now,
        Some(expires),
        request.max_uses,
    )
    .map_err(policy_error)
}

impl Engine {
    pub(crate) fn parked(&self, request_id: &str) -> Result<(PendingRow, ParkedRequest), CoreError> {
        let row = self.store.pending_item(request_id, unix_now())?.ok_or(CoreError::NotFound)?;
        if row.kind != PendingKind::Request {
            return Err(CoreError::NotFound);
        }
        let parked = serde_json::from_slice(&row.payload).map_err(|_| CoreError::storage("corrupt parked request"))?;
        Ok((row, parked))
    }

    /// A request parked before accounts existed names none: it is about the only one connected.
    async fn bind_account(&self, parked: &mut ParkedRequest) -> Result<(), CoreError> {
        if parked.request.account.is_none() {
            let account = self.resolve_account(None).await.map_err(CoreError::gmail)?;
            parked.request.account = Some(account);
        }
        Ok(())
    }

    /// The user approved. The action runs first; on a Gmail failure the request stays parked. A request Autopilot
    /// evaluated is remembered with the user's answer.
    pub async fn approve(&self, request_id: &str, choice: ApprovalChoice) -> Result<(), CoreError> {
        let Some(_claim) = self.claim(request_id) else {
            return Err(CoreError::NotFound);
        };
        let decision = Decision::new("", self.ap_human_note(request_id));
        let (result, _) = deciding(decision, self.approve_claimed(request_id, choice)).await;
        if result.is_ok() {
            self.ap_learn(request_id, Verdict::Approve);
        }
        result
    }

    /// The user refused.
    pub async fn deny(&self, request_id: &str) -> Result<(), CoreError> {
        let Some(_claim) = self.claim(request_id) else {
            return Err(CoreError::NotFound);
        };
        let decision = Decision::new("", self.ap_human_note(request_id));
        let (result, _) = deciding(decision, self.deny_claimed(request_id)).await;
        if result.is_ok() {
            self.ap_learn(request_id, Verdict::Deny);
        }
        result
    }

    /// Approves a request the caller already claimed (the user's tap, or Autopilot).
    pub(crate) async fn approve_claimed(&self, request_id: &str, choice: ApprovalChoice) -> Result<(), CoreError> {
        let session = self.session()?;
        let (_, mut parked) = self.parked(request_id)?;
        let now = unix_now();
        if matches!(parked.request.call, ToolCall::Mcp(_)) {
            return self.approve_mcp(&session, request_id, &parked, &choice, now).await;
        }
        if matches!(parked.kind(), ApprovalKind::Fetch | ApprovalKind::Write) {
            let scope = self.blob_scope();
            return crate::blob::within(scope, self.approve_connector(&session, request_id, &parked, &choice, now))
                .await;
        }
        if matches!(parked.kind(), ApprovalKind::Fetch | ApprovalKind::Write) {
            return self.approve_connector(&session, request_id, &parked, &choice, now).await;
        }
        if parked.kind() == ApprovalKind::Accounts {
            // About an integration as a whole: no single account to settle on.
            return self.approve_accounts(&session, request_id, &parked, &choice, now).await;
        }
        self.bind_account(&mut parked).await?;
        let request = &parked.request;
        let call = request.call.clone().normalized().map_err(policy_error)?;
        let kind = parked.kind();
        if kind == ApprovalKind::Grant {
            return self.approve_grant(&session, request_id, &parked, &choice, now).await;
        }

        // Which of the shown messages are released: already covered ones plus the user's picks.
        let shown: BTreeSet<&str> = parked.messages.iter().map(|m| m.id.as_str()).collect();
        if let Some(unknown) = choice.selected_message_ids.iter().find(|id| !shown.contains(id.as_str())) {
            return Err(CoreError::invalid(format!("message {unknown} was not part of this request")));
        }
        let picked: BTreeSet<&str> = choice.selected_message_ids.iter().map(String::as_str).collect();
        let released: Vec<&MessageSummary> = parked
            .messages
            .iter()
            .filter(|m| parked.covered.contains_key(&m.id) || picked.contains(m.id.as_str()))
            .collect();
        let released_ids: Vec<String> = released.iter().map(|m| m.id.clone()).collect();

        // Validate the standing grant before anything irreversible happens.
        let new_grant = choice
            .standing
            .as_ref()
            .map(|s| build_standing_grant(s, kind, &released_ids, &request.connection_id, now))
            .transpose()?
            .map(|g| g.for_account(request.account.clone()));

        let gmail = self.gmail_for(handler::account_of(&parked.request));
        let (result, action, detail) = match &call {
            ToolCall::GmailSearch {
                query,
                ..
            } => (
                ToolResult::Search {
                    messages: released.iter().map(|m| (*m).clone()).collect(),
                },
                "search",
                format!("{} for \"{}\"", count(released.len()), text::one_line(query)),
            ),
            ToolCall::GmailRead {
                ..
            } => {
                let messages = gmail.fetch(&released_ids, true).await?;
                let detail = format!("{} read", count(messages.len()));
                (
                    ToolResult::Read {
                        messages: messages.into_iter().map(ParsedMessage::into_full).collect(),
                    },
                    "read",
                    detail,
                )
            }
            ToolCall::GmailSend {
                email,
            } => {
                let sent = gmail.send(email).await?;
                (ToolResult::Sent(sent), "send", send_detail(email))
            }
            ToolCall::RequestGrant {
                ..
            }
            | ToolCall::ListAccounts {
                ..
            }
            | ToolCall::Connector(_)
            | ToolCall::Mcp(_)
            | ToolCall::RequestUpload {
                ..
            } => return Err(CoreError::invalid("this request is not answered by approving it")),
        };

        // The action happened: settle local state before answering the server.
        let covering: BTreeSet<GrantId> = parked.covered.values().map(|g| GrantId(g.clone())).collect();
        self.store.consume_active(&covering, now)?;
        if let Some(grant) = &new_grant {
            self.store.insert_grant(grant, &parked.label())?;
        }
        let outcome = if matches!(kind, ApprovalKind::Send) {
            "sent"
        } else {
            "released"
        };
        // The AI stopped waiting before the user decided: a read gets a one-time pass so that simply asking
        // again works. (A send has already happened; asking again would only duplicate it.)
        let late = request.wait_until.is_some_and(|t| now > t);
        let needs_pass = late
            && new_grant.is_none()
            && matches!(kind, ApprovalKind::Search | ApprovalKind::Read)
            && !released_ids.is_empty();
        let note = if needs_pass {
            let pass = Grant::new(
                GrantId(uuid::Uuid::new_v4().to_string()),
                request.connection_id.clone(),
                Scope::Read(ReadScope {
                    message_ids: Some(released_ids.iter().cloned().collect()),
                    ..ReadScope::default()
                }),
                now,
                Some(now + RETRY_PASS_SECS),
                Some(1),
            )
            .map_err(policy_error)?
            .for_account(request.account.clone());
            self.store.insert_grant_from(&pass, &parked.label(), "retry")?;
            Some("Approved after the AI stopped waiting. A one-time pass lets it ask again for 15 minutes.".to_owned())
        } else {
            None
        };
        let count_released = if matches!(kind, ApprovalKind::Send) {
            call_recipients(&call)
        } else {
            released.len()
        };
        let mut audit = self.audit(
            request,
            action,
            outcome,
            &format!("approved: {detail}"),
            new_grant.as_ref().map(|g| g.id.0.clone()),
            count_released,
            &released,
        );
        audit.info.note = note;
        self.store.append_audit(&audit)?;
        self.store.remove_pending(request_id)?;
        self.store.mark_handled(request_id, now)?;
        self.notifier.item_resolved(request_id.to_owned());
        self.respond(
            &session,
            request_id,
            RelayOutcome::Result {
                result,
            },
        )
        .await
    }

    /// Denies a request the caller already claimed (the user's tap, or Autopilot).
    pub(crate) async fn deny_claimed(&self, request_id: &str) -> Result<(), CoreError> {
        let session = self.session()?;
        let (_, parked) = self.parked(request_id)?;
        let now = unix_now();
        let shown: Vec<&MessageSummary> = parked.messages.iter().collect();
        let audit = self.denied_connector(&parked).or_else(|| self.denied_mcp(&parked)).unwrap_or_else(|| {
            self.audit(
                &parked.request,
                parked.action(),
                "denied",
                &crate::autopilot::context::denied_detail(),
                None,
                parked.count() as usize,
                &shown,
            )
        });
        self.store.append_audit(&audit)?;
        self.store.remove_pending(request_id)?;
        self.store.mark_handled(request_id, now)?;
        self.notifier.item_resolved(request_id.to_owned());
        self.discard_input(&session, &parked).await;
        self.respond(
            &session,
            request_id,
            RelayOutcome::Denied {
                reason: None,
            },
        )
        .await
    }

    /// The user agreed to show an AI the accounts of one integration, once or for a while.
    async fn approve_accounts(
        &self,
        session: &crate::session::Session,
        request_id: &str,
        parked: &ParkedRequest,
        choice: &ApprovalChoice,
        now: i64,
    ) -> Result<(), CoreError> {
        let ToolCall::ListAccounts {
            service: Some(service),
            ..
        } = &parked.request.call
        else {
            return Err(CoreError::invalid("not a request to see accounts"));
        };
        // The picks (in `selected_message_ids`, as addresses) must be among what was offered and not shared before.
        let connected = self.accounts_of(service);
        let offered: BTreeSet<&str> =
            parked.accounts.iter().filter(|a| !parked.shared_accounts.contains(a)).map(String::as_str).collect();
        if let Some(unknown) = choice.selected_message_ids.iter().find(|a| !offered.contains(a.as_str())) {
            return Err(CoreError::invalid(format!("account {unknown} was not part of this request")));
        }
        let picked: Vec<String> = parked
            .accounts
            .iter()
            .filter(|a| choice.selected_message_ids.contains(a) && connected.contains(a))
            .cloned()
            .collect();
        if picked.is_empty() {
            return Err(CoreError::invalid("select at least one account, or deny the request"));
        }
        // The AI is shown what it already could see plus the picks; anything else stays private.
        let coverage = self.store.account_coverage(&parked.request.connection_id, service, now)?;
        let shown: Vec<String> =
            connected.iter().filter(|a| picked.contains(a) || coverage.covers(a)).cloned().collect();
        let withheld = connected.len() - shown.len();
        let grant = choice
            .standing
            .as_ref()
            .map(|s| {
                let secs =
                    s.duration_secs.filter(|&d| d > 0).ok_or_else(|| CoreError::invalid("choose for how long"))?;
                let expires = now.saturating_add(i64::try_from(secs).unwrap_or(i64::MAX));
                Grant::new(
                    GrantId(uuid::Uuid::new_v4().to_string()),
                    parked.request.connection_id.clone(),
                    Scope::Accounts(reins_policy::AccountsScope {
                        service: service.clone(),
                        accounts: picked.clone(),
                    }),
                    now,
                    Some(expires),
                    s.max_uses,
                )
                .map_err(policy_error)
            })
            .transpose()?;
        if let Some(grant) = &grant {
            self.store.insert_grant_from(grant, &parked.label(), "approval")?;
        }
        self.store.remove_pending(request_id)?;
        self.notifier.item_resolved(request_id.to_owned());
        self.share_accounts(session, &parked.request, service, shown, withheld, grant.map(|g| g.id.0)).await
    }

    /// The user agreed to a permission the AI asked for.
    async fn approve_grant(
        &self,
        session: &crate::session::Session,
        request_id: &str,
        parked: &ParkedRequest,
        choice: &ApprovalChoice,
        now: i64,
    ) -> Result<(), CoreError> {
        let ToolCall::RequestGrant {
            grant: asked,
        } = &parked.request.call
        else {
            return Err(CoreError::invalid("not a permission request"));
        };
        let shorten_to = choice.standing.as_ref().and_then(|s| s.duration_secs);
        let grant = grant_from_request(asked, &parked.request.connection_id, now, shorten_to)?
            .for_account(parked.request.account.clone());
        self.store.insert_grant_from(&grant, &parked.label(), "ai_request")?;
        let summary = views::grant_summary(&grant.scope);
        let expires_at = grant.expires_at.unwrap_or(now);
        let mut audit = self.audit(
            &parked.request,
            "grant",
            "granted",
            &format!("allowed: {summary}"),
            Some(grant.id.0.clone()),
            1,
            &[],
        );
        audit.info.grant_summary = Some(summary.clone());
        self.store.append_audit(&audit)?;
        self.store.remove_pending(request_id)?;
        self.store.mark_handled(request_id, now)?;
        self.notifier.item_resolved(request_id.to_owned());
        self.respond(
            session,
            request_id,
            RelayOutcome::Result {
                result: ToolResult::Granted {
                    summary,
                    expires_at,
                    max_uses: grant.max_uses,
                },
            },
        )
        .await
    }

    /// Creates a permission ahead of time, without any request (the user's own "new grant").
    pub async fn create_grant(
        &self,
        connection_id: &str,
        account: &str,
        kind: ApprovalKind,
        standing: StandingGrant,
    ) -> Result<(), CoreError> {
        check_id(connection_id)?;
        let account = self
            .gmail_accounts()
            .into_iter()
            .find(|a| a.eq_ignore_ascii_case(account.trim()))
            .ok_or_else(|| CoreError::invalid("choose one of the connected accounts"))?;
        if !matches!(kind, ApprovalKind::Read | ApprovalKind::Send) {
            return Err(CoreError::invalid("choose whether the permission is for reading or for sending"));
        }
        let connection =
            self.connections().await?.into_iter().find(|c| c.id == connection_id).ok_or(CoreError::NotFound)?;
        let now = unix_now();
        let conn_id = ConnectionId(connection_id.to_owned());
        let grant = build_standing_grant(&standing, kind, &[], &conn_id, now)?.for_account(Some(account.clone()));
        self.store.insert_grant_from(&grant, &connection.label, "user")?;
        let (service, account) = (views::SERVICE_GMAIL.to_owned(), Some(account));
        self.store.append_audit(&AuditRecord {
            seq: 0,
            at: now,
            connection_id: connection_id.to_owned(),
            connection_label: connection.label,
            action: "grant".to_owned(),
            outcome: "granted".to_owned(),
            detail: format!("created: {}", views::grant_summary(&grant.scope)),
            grant_id: Some(grant.id.0.clone()),
            service,
            account,
            count: 1,
            op: String::new(),
            info: AuditInfo {
                grant_summary: Some(views::grant_summary(&grant.scope)),
                note: Some("Created by you in advance.".to_owned()),
                ..AuditInfo::default()
            },
        })?;
        Ok(())
    }

    fn parked_pairing(&self, id: &str) -> Result<PairingRequest, CoreError> {
        let row = self.store.pending_item(id, unix_now())?.ok_or(CoreError::NotFound)?;
        if row.kind != PendingKind::Pairing {
            return Err(CoreError::NotFound);
        }
        serde_json::from_slice(&row.payload).map_err(|_| CoreError::storage("corrupt parked pairing"))
    }

    pub fn pairing_view(&self, id: &str) -> Result<PairingView, CoreError> {
        Ok(views::pairing_view(&self.parked_pairing(id)?))
    }

    /// The pairing a computer's code stands for (scanned from its QR code, or from a link): the server hands it to this
    /// phone, which parks it like a pushed one. It is answered with [`Engine::answer_pairing`]: the user still taps the
    /// number the computer shows and compares its key.
    pub async fn pairing_by_code(&self, user_code: &str) -> Result<PairingView, CoreError> {
        let code = normalize_user_code(user_code).ok_or_else(|| {
            CoreError::invalid("That is not a Reins pairing code. Scan the QR code your computer shows.")
        })?;
        let session = self.session()?;
        let pairing = match api_call!(&session, |api| api.claim_pairing(&code)) {
            Ok(p) => p,
            Err(ApiFailure::Status {
                status: 409,
                ..
            }) => {
                return Err(CoreError::invalid(
                    "This code was already used. Show a new one on your computer and scan it again.",
                ));
            }
            Err(e) => return Err(e.into_core()),
        };
        let id = pairing.id.0.clone();
        // Scanned twice: the pairing is parked already.
        if let Ok(view) = self.pairing_view(&id) {
            return Ok(view);
        }
        let Some(_guard) = self.begin(&id)? else {
            return self.pairing_view(&id);
        };
        self.park_pairing(pairing)?;
        self.pairing_view(&id)
    }

    /// Sends the user's answer for an AI-connection request.
    pub async fn answer_pairing(
        &self,
        id: &str,
        approve: bool,
        chosen_code: Option<u8>,
        label: Option<String>,
    ) -> Result<(), CoreError> {
        let session = self.session()?;
        let Some(_claim) = self.claim(id) else {
            return Err(CoreError::NotFound);
        };
        let pairing = self.parked_pairing(id)?;
        if approve {
            let Some(code) = chosen_code else {
                return Err(CoreError::invalid("choose the number shown on your computer or in the browser"));
            };
            if !pairing.choices.contains(&code) {
                return Err(CoreError::invalid("that number was not offered"));
            }
        }
        let response = PairingResponse {
            v: PROTOCOL_VERSION,
            approved: approve,
            chosen_code: if approve {
                chosen_code
            } else {
                None
            },
            label: label.map(|l| text::truncate_chars(&text::one_line(&l), 64)).filter(|l| !l.is_empty()),
        };
        let now = unix_now();
        let result = api_call!(&session, |api| api.answer_pairing(id, &response));
        let (outcome, detail) = match &result {
            Ok(r) if approve && r.connection_id.is_some() => ("released", format!("connected {}", pairing.client_name)),
            Ok(_) => ("denied", format!("refused {}", pairing.client_name)),
            Err(ApiFailure::Status {
                status: 409,
                code,
                ..
            }) if code == "wrong_code" => ("error", "wrong number chosen; the connection was cancelled".to_owned()),
            Err(_) => return result.map(drop).map_err(ApiFailure::into_core),
        };
        // The desktop app's key, compared by the user on both screens, now belongs to this connection.
        let new_connection = result.as_ref().ok().filter(|_| approve).and_then(|r| r.connection_id.as_ref());
        let key = pairing.client_key.as_deref().and_then(reins_proto::desktop::decode_key);
        let mut note = format!("From {}", pairing.client_host);
        if let (Some(connection), Some(key)) = (new_connection, key) {
            let key = reins_proto::desktop::encode_key(&key);
            self.store.pin_desktop_key(&connection.0, &key, now)?;
            if let Some(fingerprint) = reins_proto::desktop::key_fingerprint(&key) {
                note = format!("{note}. Desktop app key {fingerprint}");
            }
        }
        self.store.append_audit(&AuditRecord {
            seq: 0,
            at: now,
            connection_id: result.as_ref().ok().and_then(|r| r.connection_id.clone()).map(|c| c.0).unwrap_or_default(),
            connection_label: pairing.client_name.clone(),
            action: "pair".to_owned(),
            outcome: outcome.to_owned(),
            detail,
            grant_id: None,
            service: String::new(),
            account: None,
            count: 1,
            op: String::new(),
            info: AuditInfo {
                note: Some(note),
                ..AuditInfo::default()
            },
        })?;
        self.store.remove_pending(id)?;
        self.store.mark_handled(id, now)?;
        self.notifier.item_resolved(id.to_owned());
        result.map(drop).map_err(ApiFailure::into_core)
    }
}

fn call_recipients(call: &ToolCall) -> usize {
    match call {
        ToolCall::GmailSend {
            email,
        } => email.recipients().count(),
        _ => 0,
    }
}

fn count(n: usize) -> String {
    if n == 1 {
        "1 message".to_owned()
    } else {
        format!("{n} messages")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn() -> ConnectionId {
        "c1".into()
    }

    fn scope() -> GrantScopeChoice {
        GrantScopeChoice {
            all_mail: false,
            selected_messages_only: false,
            sender_addresses: vec![],
            sender_domains: vec![],
            subject_pattern: None,
            recipient_addresses: vec![],
            recipient_domains: vec![],
            resources: vec![],
            classes: vec![],
        }
    }

    fn standing(scope: GrantScopeChoice) -> StandingGrant {
        StandingGrant {
            duration_secs: Some(3600),
            max_uses: Some(5),
            scope,
        }
    }

    #[test]
    fn read_grants_from_choices() {
        let mut s = scope();
        s.sender_domains = vec!["Bank.com".to_owned()];
        s.sender_addresses = vec!["a@x.com".to_owned()];
        s.subject_pattern = Some(" statement ".to_owned());
        let g = build_standing_grant(&standing(s), ApprovalKind::Search, &[], &conn(), 1000).unwrap();
        assert_eq!((g.expires_at, g.max_uses, g.uses, g.revoked), (Some(4600), Some(5), 0, false));
        assert_eq!(g.connection_id, conn());
        assert_eq!(uuid::Uuid::parse_str(&g.id.0).unwrap().get_version_num(), 4);
        let Scope::Read(r) = g.scope else {
            panic!("read scope expected")
        };
        assert_eq!(r.from.len(), 2);
        assert_eq!(r.subject.unwrap().source(), "statement");
        assert!(r.message_ids.is_none());
    }

    #[test]
    fn selected_messages_only_pins_the_ids() {
        let mut s = scope();
        s.selected_messages_only = true;
        let g = build_standing_grant(&standing(s), ApprovalKind::Read, &["m1".to_owned(), "m2".to_owned()], &conn(), 0)
            .unwrap();
        let Scope::Read(r) = g.scope else {
            panic!("read scope expected")
        };
        assert_eq!(r.message_ids.unwrap().len(), 2);
        let mut none_selected = scope();
        none_selected.selected_messages_only = true;
        assert!(build_standing_grant(&standing(none_selected), ApprovalKind::Read, &[], &conn(), 0).is_err());
    }

    #[test]
    fn all_mail_grants_are_explicit_time_boxed_and_exclusive() {
        let mut all = scope();
        all.all_mail = true;
        let g = build_standing_grant(&standing(all.clone()), ApprovalKind::Search, &[], &conn(), 1000).unwrap();
        let Scope::Read(r) = &g.scope else {
            panic!("read scope expected")
        };
        assert!(r.any && r.message_ids.is_none() && r.from.is_empty());
        assert_eq!(g.expires_at, Some(4600));

        // Never open-ended, never longer than the policy allows.
        let mut forever = standing(all.clone());
        forever.duration_secs = None;
        assert!(build_standing_grant(&forever, ApprovalKind::Search, &[], &conn(), 0).is_err());
        let mut uses_only = standing(all.clone());
        uses_only.duration_secs = None;
        uses_only.max_uses = Some(3);
        assert!(build_standing_grant(&uses_only, ApprovalKind::Search, &[], &conn(), 0).is_err());
        let mut too_long = standing(all.clone());
        too_long.duration_secs = Some(31 * 86_400);
        assert!(build_standing_grant(&too_long, ApprovalKind::Search, &[], &conn(), 0).is_err());

        // Not combinable with any narrowing field, and never for sending.
        let mut mixed = all.clone();
        mixed.sender_domains = vec!["bank.com".to_owned()];
        assert!(build_standing_grant(&standing(mixed), ApprovalKind::Search, &[], &conn(), 0).is_err());
        let mut pinned = all.clone();
        pinned.selected_messages_only = true;
        assert!(build_standing_grant(&standing(pinned), ApprovalKind::Search, &["m1".to_owned()], &conn(), 0).is_err());
        assert!(build_standing_grant(&standing(all), ApprovalKind::Send, &[], &conn(), 0).is_err());
    }

    #[test]
    fn subject_text_is_literal_not_a_regex() {
        let mut s = scope();
        s.sender_domains = vec!["bank.com".to_owned()];
        s.subject_pattern = Some("a.b(c".to_owned());
        let g = build_standing_grant(&standing(s), ApprovalKind::Read, &[], &conn(), 0).unwrap();
        let Scope::Read(r) = g.scope else {
            panic!("read scope expected")
        };
        let p = r.subject.unwrap();
        assert!(p.is_match("xx a.b(c yy") && !p.is_match("aXb(c"));
    }

    #[test]
    fn send_grants_need_recipients() {
        let mut s = scope();
        s.recipient_domains = vec!["work.com".to_owned()];
        s.recipient_addresses = vec!["boss@x.com".to_owned()];
        let g = build_standing_grant(&standing(s), ApprovalKind::Send, &[], &conn(), 0).unwrap();
        let Scope::Send(send) = g.scope else {
            panic!("send scope expected")
        };
        assert_eq!(send.recipients.len(), 2);
        assert!(build_standing_grant(&standing(scope()), ApprovalKind::Send, &[], &conn(), 0).is_err());
    }

    #[test]
    fn unconstrained_or_malformed_choices_are_refused() {
        assert!(
            build_standing_grant(&standing(scope()), ApprovalKind::Read, &[], &conn(), 0).is_err(),
            "no restriction at all"
        );
        let mut bad = scope();
        bad.sender_addresses = vec!["Bob <bob@x.com>".to_owned()];
        assert!(build_standing_grant(&standing(bad), ApprovalKind::Read, &[], &conn(), 0).is_err());
        let mut ok = scope();
        ok.sender_domains = vec!["bank.com".to_owned()];
        let mut zero_duration = standing(ok.clone());
        zero_duration.duration_secs = Some(0);
        assert!(build_standing_grant(&zero_duration, ApprovalKind::Read, &[], &conn(), 0).is_err());
        let mut zero_uses = standing(ok.clone());
        zero_uses.max_uses = Some(0);
        assert!(build_standing_grant(&zero_uses, ApprovalKind::Read, &[], &conn(), 0).is_err());
        let mut forever = standing(ok);
        forever.duration_secs = None;
        forever.max_uses = None;
        let g = build_standing_grant(&forever, ApprovalKind::Read, &[], &conn(), 0).unwrap();
        assert_eq!((g.expires_at, g.max_uses), (None, None), "until revoked");
    }
}
