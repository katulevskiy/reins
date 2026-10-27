//! Deferred decisions on the phone (Decisions v2, 03; 08 R9).
//!
//! A ticket reaches this phone sealed to its box key ([`TicketItem`]). It is opened here and checked against the
//! digest the server stored, then goes through the same path as any request, with its own deadline instead of the
//! relay's ten minutes. Every transition this phone makes is signed with its device key: an approval, a denial with a
//! note, Later, a "Why?", an expiry it confirms. That is what the server records and what agents' webhooks carry.
//!
//! Who runs an approved ticket:
//! - a write runs here when approved, and the server keeps its minimal result;
//! - a read, or a desktop answer, is only approved here. It runs when an agent collects it, and only if this phone's
//!   own record of the approval is still valid and the request's digest is the one it approved. The server saying
//!   "approved" is never enough.

use data_encoding::BASE64URL_NOPAD;
use reins_proto::{
    PROTOCOL_VERSION,
    ids::ConnectionId,
    relay::{RelayOutcome, RelayRequest},
    ticket::{
        self, Execution, SealedTicketBody, SignedTicketEvent, TicketEvent, TicketEvents, TicketItem, TicketState,
        digest_of, event_message, ticket_of,
    },
};

use crate::{
    CoreError,
    engine::Engine,
    phone_api::ApiFailure,
    session::{Session, api_call},
    store::{tickets::LocalTicket, unix_now},
    types::TicketView,
};

/// What a ticket's own record says about a request that is about to run.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Revalidation {
    /// Not a ticket, or still as approved: go ahead.
    Valid,
    /// Its window ran out.
    Expired,
    /// The request is not the one the ticket was made for.
    Stale(String),
}

/// Whether this phone's own record proves ticket `local` expired at `now`: unapproved and past its deadline, or
/// approved and past its approval's validity (or already used). Nothing else is signed as expired.
pub(crate) fn may_sign_expired(local: &LocalTicket, now: i64) -> bool {
    match local.approved_at {
        None => now > local.deadline,
        Some(_) => local.valid_until.is_none_or(|until| until <= now) && local.state != TicketState::Done.as_str(),
    }
}

/// The nonce a desktop answer echoes, when `request` is a desktop call.
fn request_nonce(request: &RelayRequest) -> Option<String> {
    match &request.call {
        reins_proto::gmail::ToolCall::Connector(c) => c.str_arg("nonce").map(str::to_owned),
        _ => None,
    }
}

/// What a collect releases: the ids the owner approved that the read still finds, in the read's order.
pub(crate) fn collect_selection(approved: &[String], found: &[String]) -> Vec<String> {
    found.iter().filter(|id| approved.contains(id)).cloned().collect()
}

/// Whether `parked` (as it is about to run) is still the request `local` was made for, at `now`.
pub(crate) fn revalidate(local: &LocalTicket, parked: &RelayRequest, now: i64) -> Revalidation {
    if now > local.deadline && local.approved_at.is_none() {
        return Revalidation::Expired;
    }
    let Ok(original) = serde_json::from_str::<RelayRequest>(&local.request) else {
        return Revalidation::Stale("the request could not be read back".to_owned());
    };
    if digest_of(&original) != local.digest {
        return Revalidation::Stale("the request is not the one the ticket names".to_owned());
    }
    let normalized = original.call.clone().normalized().ok();
    if original.id != parked.id
        || original.connection_id != parked.connection_id
        || normalized.as_ref() != Some(&parked.call)
    {
        return Revalidation::Stale("the request changed since it was asked".to_owned());
    }
    Revalidation::Valid
}

impl Engine {
    /// Signs `state` for ticket `local` with this device's key.
    pub(crate) fn sign_event(
        &self,
        local: &LocalTicket,
        state: TicketState,
        change: impl FnOnce(&mut TicketEvent),
    ) -> Result<SignedTicketEvent, CoreError> {
        let mut event = TicketEvent {
            v: PROTOCOL_VERSION,
            ticket: ticket_of(&reins_proto::ids::RequestId(local.id.clone())),
            // Where this phone last knew the ticket to be: the server applies the event only from there.
            from: TicketState::parse(&local.state).unwrap_or(TicketState::Reviewing),
            state,
            at: unix_now(),
            digest: local.digest.clone(),
            connection_id: ConnectionId(local.connection_id.clone()),
            deadline: None,
            note: None,
            result: None,
            stale: None,
        };
        change(&mut event);
        let key = self.sign_key()?;
        Ok(SignedTicketEvent {
            sig: key.sign_b64(&event_message(&event)),
            key: key.public_b64(),
            event,
        })
    }

    /// Opens a ticket the server sealed to this device and checks it is what the server's metadata says.
    fn open_ticket(&self, item: &TicketItem) -> Result<RelayRequest, CoreError> {
        let sealed = BASE64URL_NOPAD
            .decode(item.sealed.as_bytes())
            .map_err(|_| CoreError::invalid("a ticket was not sealed as expected"))?;
        let plain = self.box_key()?.open(&sealed)?;
        let body: SealedTicketBody =
            serde_json::from_slice(&plain).map_err(|_| CoreError::invalid("a ticket could not be read"))?;
        let request = body.request;
        if request.id != item.id || request.connection_id != item.connection_id || digest_of(&request) != item.digest {
            return Err(CoreError::invalid("a ticket's body does not match what the server says it is"));
        }
        Ok(request)
    }

    /// The agent's answer to a "Why?", sealed to this device.
    fn open_reply(&self, sealed: &str) -> Option<String> {
        let bytes = BASE64URL_NOPAD.decode(sealed.as_bytes()).ok()?;
        let plain = self.box_key().ok()?.open(&bytes).ok()?;
        // The agent's words: invisible and bidi characters show instead of acting.
        let text = crate::text::mark_invisible(&String::from_utf8_lossy(&plain));
        ticket::clean_text(&text, ticket::MAX_REPLY_CHARS)
    }

    /// Takes the tickets of one poll.
    /// Takes the tickets of one poll. Every one is tried; a ticket that could not be opened or kept is an error the
    /// app shows (and the next sync's reconciliation tries it again), never a silent stall.
    pub(crate) async fn take_tickets(&self, session: &Session, items: Vec<TicketItem>) -> Result<(), CoreError> {
        let mut failed = None;
        for item in items {
            if let Err(e) = self.take_ticket(session, &item).await {
                log::warn!("could not take a ticket: {e}");
                failed = Some(e);
            }
        }
        failed.map_or(Ok(()), Err)
    }

    /// Takes every open ticket the server holds that this phone does not show: one delivered while this phone could
    /// not keep it (killed mid-sync, storage full) comes back instead of being lost.
    pub(crate) async fn reconcile_tickets(&self, session: &Session) -> Result<(), CoreError> {
        let all = match api_call!(session, |api| api.tickets()) {
            Ok(all) => all,
            // A server from before tickets has none.
            Err(ApiFailure::Status {
                status: 404,
                ..
            }) => return Ok(()),
            Err(e) => return Err(e.into_core()),
        };
        let now = unix_now();
        let missing: Vec<TicketItem> = all
            .into_iter()
            .filter(|item| item.state.is_open())
            .filter(|item| !matches!(self.store.pending_item(&item.id.0, now), Ok(Some(_))))
            .filter(|item| !matches!(self.store.ticket(&item.id.0), Ok(Some(l)) if l.state == item.state.as_str()))
            .collect();
        self.take_tickets(session, missing).await
    }

    async fn take_ticket(&self, session: &Session, item: &TicketItem) -> Result<(), CoreError> {
        let id = item.id.0.clone();
        let now = unix_now();
        let known = self.store.ticket(&id)?;
        if item.state == TicketState::Approved {
            if item.collect {
                return self.collect(session, item, known).await;
            }
            return Ok(());
        }
        if item.state == TicketState::Expired {
            // Signed only when this phone's own record says so; the server's word drops the card, nothing more.
            if let Some(mut local) = known.filter(|l| !l.expiry_signed && may_sign_expired(l, now)) {
                let event = self.sign_event(&local, TicketState::Expired, |_| {})?;
                // Confirms the expiry the server recorded, so webhooks carry a signed "not approved".
                let posted = self.post_events(session, &id, vec![event], None).await;
                if let Err(e) = posted {
                    log::info!("the expiry could not be confirmed: {e}");
                }
                local.expiry_signed = true;
                TicketState::Expired.as_str().clone_into(&mut local.state);
                self.store.save_ticket(&local, now)?;
            }
            self.drop_ticket_card(&id)?;
            return Ok(());
        }
        if item.state.is_terminal() {
            self.store.remove_ticket(&id)?;
            return self.drop_ticket_card(&id);
        }
        let request = self.open_ticket(item)?;
        let mut local = known.unwrap_or_default();
        local.id.clone_from(&id);
        local.connection_id.clone_from(&item.connection_id.0);
        local.request = serde_json::to_string(&request).map_err(|e| CoreError::storage(e.to_string()))?;
        local.digest.clone_from(&item.digest);
        ticket::execution(&request.call).as_str().clone_into(&mut local.execution);
        local.valid_secs = ticket::approval_validity(&request.call);
        item.state.as_str().clone_into(&mut local.state);
        local.deferred_at = item.deferred_at;
        local.deadline = item.deadline;
        local.question.clone_from(&item.question);
        local.reply = item.sealed_reply.as_deref().and_then(|r| self.open_reply(r));
        local.stale.clone_from(&item.stale);
        // Open again (stale, re-asked): any earlier approval is spent. It takes a new review, signed anew; a replayed
        // "approved" from the server finds nothing here to collect.
        let reopened = local.approved_at.is_some() || item.state == TicketState::Stale;
        local.approved_at = None;
        local.valid_until = None;
        local.approved_ids = None;
        self.store.save_ticket(&local, now)?;
        if reopened {
            // Answered once: asked again, with "Since you approved: …".
            self.store.forget_handled(&id)?;
        }
        // New to this phone (a deferral it did not see as Instant, a reinstall): the ordinary path parks it.
        let mut request = request;
        request.wait_until = Some(item.deadline);
        self.process_request(session, request).await?;
        self.notifier_refresh(&id);
        Ok(())
    }

    /// A push about a ticket (not a relay request): its news comes with the poll.
    pub(crate) async fn take_ticket_news(&self, _session: &Session) -> Result<(), CoreError> {
        Box::pin(self.sync_inner(0)).await.map(drop)
    }

    /// Tells the app an item changed (a ticket's deadline, a question, an answer).
    fn notifier_refresh(&self, id: &str) {
        if let Ok(Some(row)) = self.store.pending_item(id, unix_now())
            && let Ok(parked) = serde_json::from_slice::<crate::views::ParkedRequest>(&row.payload)
        {
            self.notifier.item_pending(self.pending_request_item(&parked));
        }
    }

    fn drop_ticket_card(&self, id: &str) -> Result<(), CoreError> {
        if self.store.remove_pending(id)? {
            self.notifier.item_resolved(id.to_owned());
        }
        Ok(())
    }

    /// An agent collects an approved read or desktop answer: it runs now, only on this phone's own valid approval of
    /// exactly this request.
    async fn collect(&self, session: &Session, item: &TicketItem, local: Option<LocalTicket>) -> Result<(), CoreError> {
        let id = item.id.0.clone();
        let now = unix_now();
        let Some(mut local) = local.filter(|l| {
            l.approved_at.is_some()
                && l.execution == Execution::OnCollect.as_str()
                && l.state == TicketState::Approved.as_str()
        }) else {
            // Never approved here (another phone, a reinstall, a server that says so): nothing runs.
            log::warn!("a collect for a ticket this phone has no approval of was refused");
            return Ok(());
        };
        if may_sign_expired(&local, now) {
            let event = self.sign_event(&local, TicketState::Expired, |_| {})?;
            return self.post_events(session, &id, vec![event], None).await.map(drop);
        }
        let request = match self.open_ticket(item) {
            Ok(request) if digest_of(&request) == local.digest => request,
            _ => {
                let event = self.sign_event(&local, TicketState::Stale, |e| {
                    e.stale = Some("the request is not the one that was approved".to_owned());
                })?;
                return self.post_events(session, &id, vec![event], None).await.map(drop);
            }
        };
        // Runs again: the earlier answer was the decision, this one the result. A desktop answer it mints expires an
        // hour after the approval, not after this collect.
        if let Some(nonce) = request_nonce(&request) {
            crate::connector::desktop::limit_ask(&nonce, local.valid_until.unwrap_or(now));
        }
        self.store.forget_handled(&id)?;
        self.process_request(session, request).await?;
        if self.store.pending_item(&id, now)?.is_some() {
            let Some(_claim) = self.claim(&id) else {
                return Ok(());
            };
            let view = self.approval_view(&id)?;
            let approved: Vec<String> =
                local.approved_ids.as_deref().and_then(|j| serde_json::from_str(j).ok()).unwrap_or_default();
            let found: Vec<String> = view.messages.iter().map(|m| m.id.clone()).collect();
            let choice = crate::types::ApprovalChoice {
                // Only what the owner released, of what the read finds now: anything new needs a new review.
                selected_message_ids: collect_selection(&approved, &found),
                standing: None,
            };
            self.approve_claimed(&id, choice, crate::authorization::AuthorizationSource::TicketCollect).await?;
        }
        // One use here: the approval is spent. The server closes the ticket once an agent received the result (a
        // parked push: once the desktop app reports it pushed), so a result nobody picked up is not lost with it.
        TicketState::Done.as_str().clone_into(&mut local.state);
        local.valid_until = None;
        self.store.save_ticket(&local, now)
    }

    /// Keeps what the owner picked when approving ticket `id` (a read): what a collect may release.
    pub(crate) fn record_ticket_selection(&self, id: &str, picked: &[String]) -> Result<(), CoreError> {
        let Some(mut local) = self.store.ticket(id)? else {
            return Ok(());
        };
        if local.approved_at.is_some() {
            return Ok(());
        }
        local.approved_ids = Some(serde_json::to_string(picked).map_err(|e| CoreError::storage(e.to_string()))?);
        self.store.save_ticket(&local, unix_now())
    }

    /// Posts this phone's signed transitions of ticket `id`.
    pub(crate) async fn post_events(
        &self,
        session: &Session,
        id: &str,
        events: Vec<SignedTicketEvent>,
        window_secs: Option<u64>,
    ) -> Result<ticket::TicketStatus, CoreError> {
        let body = TicketEvents {
            v: PROTOCOL_VERSION,
            events,
            window_secs,
            on_expiry: None,
        };
        api_call!(session, |api| api.post_ticket_events(id, &body)).map_err(ApiFailure::into_core)
    }

    /// The signed transitions that go with answering ticket `id` with `outcome` (none for a request that is not a
    /// ticket, or for a collected result).
    pub(crate) fn answer_events(&self, id: &str, outcome: &RelayOutcome) -> Result<Vec<SignedTicketEvent>, CoreError> {
        let Some(local) = self.store.ticket(id)? else {
            return Ok(Vec::new());
        };
        let on_approve = local.execution == Execution::OnApprove.as_str();
        let collected = local.approved_at.is_some();
        if collected {
            // Approved first (signed, accepted by the server), then run: what is left to say is how it ended.
            return Ok(match outcome {
                RelayOutcome::Result {
                    result,
                } if on_approve => {
                    let minimal = ticket::minimal_result(result);
                    vec![self.sign_event(&local, TicketState::Done, |e| e.result = Some(minimal))?]
                }
                RelayOutcome::Error {
                    ..
                } if on_approve => vec![self.sign_event(&local, TicketState::Failed, |_| {})?],
                _ => Vec::new(),
            });
        }
        Ok(match outcome {
            RelayOutcome::Result {
                result,
            } if !collected => {
                let approved = self.sign_event(&local, TicketState::Approved, |_| {})?;
                if on_approve {
                    let minimal = ticket::minimal_result(result);
                    let done = self.sign_event(&local, TicketState::Done, |e| {
                        e.from = TicketState::Approved;
                        e.result = Some(minimal);
                    })?;
                    vec![approved, done]
                } else {
                    vec![approved]
                }
            }
            RelayOutcome::Denied {
                reason,
            } if !collected => {
                let note = reason.as_deref().and_then(|r| ticket::clean_text(r, ticket::MAX_NOTE_CHARS));
                vec![self.sign_event(&local, TicketState::Denied, |e| e.note = note)?]
            }
            RelayOutcome::Error {
                ..
            } if on_approve && !collected => vec![
                self.sign_event(&local, TicketState::Approved, |_| {})?,
                self.sign_event(&local, TicketState::Failed, |e| e.from = TicketState::Approved)?,
            ],
            _ => Vec::new(),
        })
    }

    /// What answering ticket `id` with `outcome` leaves of it here: an approval to collect later, or nothing.
    pub(crate) fn after_answer(&self, id: &str, outcome: &RelayOutcome) -> Result<(), CoreError> {
        let Some(mut local) = self.store.ticket(id)? else {
            return Ok(());
        };
        let now = unix_now();
        let keep = matches!(outcome, RelayOutcome::Result { .. }) && local.execution == Execution::OnCollect.as_str();
        if keep && local.approved_at.is_none() {
            TicketState::Approved.as_str().clone_into(&mut local.state);
            local.approved_at = Some(now);
            local.valid_until = Some(now.saturating_add(local.valid_secs));
            self.store.save_ticket(&local, now)
        } else if keep {
            Ok(())
        } else {
            self.store.remove_ticket(id)
        }
    }

    /// Before an owner's approval of ticket `id` runs anything: the signed Approved goes first, and only the server's
    /// yes (its compare-and-swap: not cancelled, expired or stale meanwhile) lets it run.
    pub(crate) async fn approve_ticket_first(&self, session: &Session, id: &str) -> Result<(), CoreError> {
        let Some(mut local) = self.store.ticket(id)? else {
            return Ok(());
        };
        if local.approved_at.is_some() {
            return Ok(());
        }
        let approved = self.sign_event(&local, TicketState::Approved, |_| {})?;
        let status = self.post_events(session, id, vec![approved], None).await?;
        if status.state != TicketState::Approved {
            return Err(CoreError::invalid("This request is no longer waiting for approval."));
        }
        let now = unix_now();
        TicketState::Approved.as_str().clone_into(&mut local.state);
        local.approved_at = Some(now);
        local.valid_until = Some(now.saturating_add(local.valid_secs));
        self.store.save_ticket(&local, now)
    }

    /// Checks a ticket before its approval runs (03 §6): within its window, and exactly the request it was made for.
    /// A stale one is reported to the server (it asks again) and refused here.
    pub(crate) fn revalidate_ticket(&self, id: &str, parked: &RelayRequest) -> Result<(), CoreError> {
        let Some(local) = self.store.ticket(id)? else {
            return Ok(());
        };
        match revalidate(&local, parked, unix_now()) {
            Revalidation::Valid => Ok(()),
            Revalidation::Expired => Err(CoreError::invalid("This request expired before it was approved.")),
            // Refused here, before anything runs; the ticket stays as it is for the reviewer to deny.
            Revalidation::Stale(why) => Err(CoreError::invalid(format!("This request is stale: {why}."))),
        }
    }

    /// Instant requests this phone holds whose window ran out unanswered become tickets by this phone's own signed word
    /// (the server never parks what the phone has: its answer could be on the way).
    pub(crate) async fn defer_overdue(&self) {
        if !self.decides() {
            return;
        }
        let now = unix_now();
        let window = self.instant_window.load(std::sync::atomic::Ordering::Acquire);
        let Ok(rows) = self.store.pending_rows(now) else {
            return;
        };
        for row in rows.into_iter().filter(|r| r.kind == crate::types::PendingKind::Request) {
            let overdue = row.created_at.saturating_add(window) <= now;
            if overdue
                && matches!(self.store.ticket(&row.id), Ok(None))
                && let Err(e) = self.decide_later(&row.id).await
            {
                log::info!("an overdue request could not become a ticket: {e}");
            }
        }
    }

    /// Later: an Instant request becomes a ticket; the agent is told at once and carries on.
    pub(crate) async fn decide_later(&self, id: &str) -> Result<(), CoreError> {
        self.defer_for(id, None).await
    }

    /// [`Engine::decide_later`] with the window a policy states (a Deferred Circle arrangement's), `None` for the
    /// server's default.
    pub(crate) async fn defer_for(&self, id: &str, window_secs: Option<u64>) -> Result<(), CoreError> {
        self.require_decider()?;
        if self.store.ticket(id)?.is_some() {
            return Ok(());
        }
        // Nothing else answers it meanwhile (Autopilot, a notification action).
        let Some(_claim) = self.claim(id) else {
            return Err(CoreError::NotFound);
        };
        let (_, parked) = self.parked(id)?;
        let session = self.session()?;
        let mut local = Self::local_for(&self.as_relayed(&parked.request)?);
        let event = self.sign_event(&local, TicketState::Deferred, |_| {})?;
        let status = self.post_events(&session, id, vec![event], window_secs).await?;
        if status.ticket != ticket_of(&reins_proto::ids::RequestId(id.to_owned())) {
            // The same request already waits as a ticket: this card joined it.
            self.store.mark_handled(id, unix_now())?;
            return self.drop_ticket_card(id);
        }
        status.state.as_str().clone_into(&mut local.state);
        local.deferred_at = unix_now();
        local.deadline = status.deadline;
        // The user's own tap: the app updates from the list, without a notification.
        self.store.save_ticket(&local, unix_now())
    }

    /// "Why?": the question goes to the agent; the request waits (as a ticket) for its answer.
    pub(crate) async fn ask_why(&self, id: &str, question: &str) -> Result<(), CoreError> {
        self.require_decider()?;
        let question = ticket::clean_text(question, ticket::MAX_NOTE_CHARS)
            .ok_or_else(|| CoreError::invalid("The question is empty."))?;
        let (_, parked) = self.parked(id)?;
        let session = self.session()?;
        let mut local = match self.store.ticket(id)? {
            Some(local) => local,
            None => Self::local_for(&self.as_relayed(&parked.request)?),
        };
        let event = self.sign_event(&local, TicketState::NeedsInfo, |e| e.note = Some(question.clone()))?;
        let status = self.post_events(&session, id, vec![event], None).await?;
        status.state.as_str().clone_into(&mut local.state);
        if local.deferred_at == 0 {
            local.deferred_at = unix_now();
        }
        local.deadline = status.deadline;
        local.question = Some(question);
        local.reply = None;
        // The user's own tap: the app updates from the list, without a notification.
        self.store.save_ticket(&local, unix_now())
    }

    /// `parked`'s request as the server relayed it (what the server's digest is over).
    fn as_relayed(&self, parked: &RelayRequest) -> Result<RelayRequest, CoreError> {
        let raw = self
            .store
            .received(&parked.id.0)?
            .ok_or_else(|| CoreError::invalid("This request can no longer be moved to later."))?;
        serde_json::from_str(&raw).map_err(|_| CoreError::storage("corrupt received request"))
    }

    /// A ticket record for a request this phone holds as Instant.
    fn local_for(request: &RelayRequest) -> LocalTicket {
        LocalTicket {
            id: request.id.0.clone(),
            connection_id: request.connection_id.0.clone(),
            request: serde_json::to_string(request).unwrap_or_default(),
            digest: digest_of(request),
            execution: ticket::execution(&request.call).as_str().to_owned(),
            valid_secs: ticket::approval_validity(&request.call),
            state: TicketState::Reviewing.as_str().to_owned(),
            ..LocalTicket::default()
        }
    }

    /// The twelve digits naming this phone's signing key, as agents and the desktop app pin it.
    pub(crate) fn signing_key_fingerprint(&self) -> Result<String, CoreError> {
        ticket::signing_key_fingerprint(&self.sign_key()?.public_b64())
            .ok_or_else(|| CoreError::storage("the signing key is invalid"))
    }

    /// What the app shows of ticket `id`, when the request is one.
    pub(crate) fn ticket_view(&self, id: &str) -> Option<TicketView> {
        let local = self.store.ticket(id).ok().flatten()?;
        Some(TicketView {
            ticket: ticket_of(&reins_proto::ids::RequestId(local.id)),
            state: local.state,
            deferred_at: local.deferred_at,
            deadline: local.deadline,
            question: local.question.map(|q| crate::text::mark_invisible(&q)),
            agent_reply: local.reply.map(|r| crate::text::mark_invisible(&r)),
            // Written by the desktop app or the server: shown, never trusted to read right.
            stale: local.stale.map(|s| crate::text::one_line(&crate::text::mark_invisible(&s))),
        })
    }
}

#[cfg(test)]
mod tests {
    use reins_proto::gmail::{OutgoingEmail, ToolCall};

    use super::*;

    fn request() -> RelayRequest {
        RelayRequest {
            v: 1,
            id: "r1".into(),
            connection_id: "c1".into(),
            connection_label: "Claude".to_owned(),
            created_at: 1,
            wait_until: None,
            account: None,
            call: ToolCall::GmailSend {
                email: Box::new(OutgoingEmail {
                    to: vec!["ann@acme.com".to_owned()],
                    cc: vec![],
                    subject: "Hi".to_owned(),
                    body: "Hello".to_owned(),
                    reply_to_message_id: None,
                }),
            },
        }
    }

    fn local(r: &RelayRequest) -> LocalTicket {
        LocalTicket {
            id: r.id.0.clone(),
            connection_id: r.connection_id.0.clone(),
            request: serde_json::to_string(r).unwrap(),
            digest: digest_of(r),
            deadline: 100,
            ..LocalTicket::default()
        }
    }

    #[test]
    fn expiry_is_signed_only_when_this_phones_record_proves_it() {
        let r = request();
        let open = local(&r);
        assert!(!may_sign_expired(&open, 100), "the server says expired, the deadline says not yet");
        assert!(may_sign_expired(&open, 101));
        let mut approved = local(&r);
        approved.approved_at = Some(50);
        approved.valid_until = Some(3_650);
        approved.state = "approved".to_owned();
        assert!(!may_sign_expired(&approved, 200), "a live approval is never signed away");
        assert!(may_sign_expired(&approved, 3_650));
        approved.state = "done".to_owned();
        approved.valid_until = None;
        assert!(!may_sign_expired(&approved, 9_999), "used, not expired");
    }

    #[test]
    fn a_collect_releases_only_what_the_owner_approved() {
        let approved = vec!["m1".to_owned(), "m3".to_owned()];
        let found = vec!["m0".to_owned(), "m1".to_owned(), "m2".to_owned(), "m3".to_owned()];
        assert_eq!(collect_selection(&approved, &found), ["m1", "m3"], "mail that arrived since is not released");
        assert!(collect_selection(&approved, &["m9".to_owned()]).is_empty());
        assert!(collect_selection(&[], &found).is_empty(), "nothing approved, nothing released");
    }

    #[test]
    fn an_agents_reply_shows_its_invisible_characters() {
        let shown = crate::text::mark_invisible("approve \u{202E}evil\u{200B}");
        assert!(!crate::text::has_invisible(&shown), "{shown:?}");
    }

    #[test]
    fn an_approval_runs_only_on_the_request_it_was_made_for() {
        let r = request();
        let mut parked = r.clone();
        parked.call = parked.call.normalized().unwrap();
        assert_eq!(revalidate(&local(&r), &parked, 50), Revalidation::Valid);
        assert_eq!(revalidate(&local(&r), &parked, 101), Revalidation::Expired, "past its window");
        let mut approved = local(&r);
        approved.approved_at = Some(90);
        assert_eq!(revalidate(&approved, &parked, 101), Revalidation::Valid, "approved in time: validity rules");
        let mut changed = parked.clone();
        if let ToolCall::GmailSend {
            email,
        } = &mut changed.call
        {
            email.to = vec!["eve@evil.com".to_owned()];
        }
        assert!(matches!(revalidate(&local(&r), &changed, 50), Revalidation::Stale(_)), "another recipient");
        let mut forged = local(&r);
        forged.digest = "0".repeat(64);
        assert!(matches!(revalidate(&forged, &parked, 50), Revalidation::Stale(_)), "a digest that is not the body's");
    }
}
