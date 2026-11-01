//! The email send hold on the phone (Decisions v2, 01 §5, 08 R10).
//!
//! An approved Gmail send (by the owner, by Auto or by a mode) does not go at once:
//!
//! 1. **Hold.** The phone seals the request to its own data key ([`crate::store::Store::seal`], bound to the request
//!    id) and gives the server only that ciphertext and the hold's length. It keeps a small record (ids and times, no
//!    email) and answers the agent "will send in 30 s unless undone". Activity shows "Sending in 30 s · Undo".
//! 2. **Undo** (here, on the desktop app, or the agent's `reins_cancel`) deletes the ciphertext on the server and the
//!    record here.
//! 3. **Release.** When the hold is due (the server's silent push, the app's own timer, the next sync), the phone moves
//!    its record to Sending (a compare-and-swap in its own store, so one process of this phone claims), claims the
//!    hold (one claim on the server, which deletes the ciphertext), opens it, checks the request again (Lockdown, the
//!    policy as it is now) and sends. Never before the hold ran out on this phone's clock, whatever the server says.
//! 4. **Late.** Claimed more than [`hold::LATE_SECS`] after its release (on either clock), it is not sent: Activity
//!    asks "Still send?" ([`Engine::send_late`]). So does a policy that now wants the owner to review it.
//!
//! The level is unchanged by the hold: P3's rules give a message no lower level for a net (only file deletes, git
//! and file snapshots are lowered), so the send is decided at its own level and the hold only adds the Undo. A hold
//! that fails leaves an automatic decision to the owner (asked at its own level); an owner's own approval was asked
//! at its own level already, so it sends at once, without an Undo.

use reins_proto::{
    gmail::ToolCall,
    hold::{self, HoldCreate},
    relay::{RelayOutcome, RelayRequest, ToolResult},
};
use serde::{Deserialize, Serialize};

use crate::{
    CoreError,
    authorization::AuthorizationSource,
    engine::Engine,
    phone_api::ApiFailure,
    session::{Session, api_call},
    store::{AuditRecord, unix_now},
};

/// The owner's hold, in seconds (`0`: off).
pub(crate) const SECS_KEY: &str = "send_hold.secs";
/// This phone's holds (sealed with the data key).
const RECORDS_KEY: &str = "send_hold.records";
/// A Sending record older than this was left by a process that ended mid-send: its outcome is unknown.
const SENDING_STALE_SECS: i64 = 10 * 60;

/// Where a hold is, as Activity shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum HoldState {
    /// Waiting out its hold: "Sending in 30 s · Undo".
    Waiting,
    /// Released: being sent now.
    Sending,
    /// Not sent, because the phone woke too late or the policy wants the owner now: "Still send?".
    Late,
}

/// A held send, on its Activity entry.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct ActivityHold {
    pub id: String,
    /// When it goes (this phone's clock, Unix seconds).
    pub release_at: i64,
    pub state: HoldState,
}

/// What a release pass did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, uniffi::Record)]
pub struct HoldsReleased {
    pub sent: u32,
    /// Sends that now ask "Still send?" (the app may say so: it is an ask).
    pub late: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RecordState {
    Held,
    Sending,
    Late,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Source {
    Owner,
    Automatic,
    Ticket,
}

impl Source {
    fn of(source: AuthorizationSource) -> Self {
        match source {
            AuthorizationSource::OwnerReview => Self::Owner,
            AuthorizationSource::Automatic => Self::Automatic,
            AuthorizationSource::TicketCollect => Self::Ticket,
        }
    }

    fn authorization(self) -> AuthorizationSource {
        match self {
            Self::Owner => AuthorizationSource::OwnerReview,
            Self::Automatic => AuthorizationSource::Automatic,
            Self::Ticket => AuthorizationSource::TicketCollect,
        }
    }
}

/// One hold of this phone. Holds only ids and times while held: the email is on the server, sealed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Record {
    id: String,
    request_id: String,
    secs: u32,
    /// This phone's clock: it never sends before this, whatever the server says.
    release_at: i64,
    source: Source,
    state: RecordState,
    #[serde(default)]
    since: i64,
    /// Late only: the request, kept (sealed with the data key) until the owner decides.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    request: Option<RelayRequest>,
}

/// What goes to the server, sealed: the request exactly as approved, and who approved it.
#[derive(Serialize, Deserialize)]
struct Sealed {
    v: u32,
    request: RelayRequest,
    source: Source,
}

/// A hold that was applied.
pub(crate) struct Held {
    pub id: String,
    pub secs: u32,
    /// When it goes, on the server's clock (what the agent is told).
    pub send_at: i64,
}

fn aad(request_id: &str) -> String {
    format!("reins/send-hold/v1/{request_id}")
}

fn email_of(request: &RelayRequest) -> Option<&reins_proto::gmail::OutgoingEmail> {
    match &request.call {
        ToolCall::GmailSend {
            email,
        } => Some(email),
        _ => None,
    }
}

/// A server that has no holds (older than this build) answers the route itself with a 404 that is not ours.
fn unsupported(e: &ApiFailure) -> bool {
    matches!(e, ApiFailure::Status { status: 404 | 405, code, .. } if code != reins_proto::device::codes::NOT_FOUND)
}

impl Engine {
    /// The owner's hold in seconds: [`hold::DEFAULT_SECS`] until changed, `0` when off.
    pub fn send_hold_secs(&self) -> u32 {
        self.store
            .meta_get(SECS_KEY)
            .ok()
            .flatten()
            .and_then(|v| v.parse::<u32>().ok())
            .filter(|s| *s <= hold::MAX_SECS)
            .unwrap_or(hold::DEFAULT_SECS)
    }

    /// Sets the hold, 0 (off) to [`hold::MAX_SECS`] seconds. It changes no decision (the level is the same with or
    /// without it), so it asks no owner check.
    pub fn set_send_hold_secs(&self, secs: u32) -> Result<(), CoreError> {
        if secs > hold::MAX_SECS {
            return Err(CoreError::invalid(format!("The send hold is 0 to {} seconds.", hold::MAX_SECS)));
        }
        self.store.meta_set(SECS_KEY, &secs.to_string())
    }

    fn hold_records(&self) -> Result<Vec<Record>, CoreError> {
        Ok(match self.store.meta_get_sealed(RECORDS_KEY)? {
            Some(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
            None => Vec::new(),
        })
    }

    /// Changes the records as one transaction (another process of this phone can't write in between).
    fn update_holds<T>(&self, change: impl FnOnce(&mut Vec<Record>) -> T) -> Result<T, CoreError> {
        self.store.meta_update_sealed(RECORDS_KEY, |current| {
            let mut records: Vec<Record> =
                current.and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default();
            let out = change(&mut records);
            let bytes = serde_json::to_vec(&records).map_err(|e| CoreError::storage(e.to_string()))?;
            Ok((Some(bytes), out))
        })
    }

    /// Moves record `id` from `from` to `to` (setting `since`); `true` for the one caller that did.
    fn move_hold(&self, id: &str, from: RecordState, to: RecordState) -> Result<bool, CoreError> {
        let now = unix_now();
        self.update_holds(|records| match records.iter_mut().find(|r| r.id == id && r.state == from) {
            Some(r) => {
                r.state = to;
                r.since = now;
                true
            }
            None => false,
        })
    }

    fn drop_hold(&self, id: &str) -> Result<Option<Record>, CoreError> {
        self.update_holds(|records| records.iter().position(|r| r.id == id).map(|at| records.remove(at)))
    }

    /// The hold on an Activity entry: a held send while it waits or goes, a "Still send?" while it asks.
    pub(crate) fn hold_of(&self, entry: &crate::types::ActivityEntry, hold_id: Option<&str>) -> Option<ActivityHold> {
        let id = hold_id?;
        let record = self.hold_records().ok()?.into_iter().find(|r| r.id == id)?;
        let state = match (entry.outcome.as_str(), record.state) {
            ("held", RecordState::Held) => HoldState::Waiting,
            ("held", RecordState::Sending) => HoldState::Sending,
            ("not_sent", RecordState::Late) => HoldState::Late,
            _ => return None,
        };
        Some(ActivityHold {
            id: record.id,
            release_at: record.release_at,
            state,
        })
    }

    /// Holds the Gmail send `request` (approved by `source`) for the owner's hold. `Ok(None)`: no hold (off, or a server
    /// without holds); the caller sends now. An error: the hold could not be applied.
    pub(crate) async fn hold_send(
        &self,
        session: &Session,
        request: &RelayRequest,
        source: AuthorizationSource,
    ) -> Result<Option<Held>, CoreError> {
        let secs = self.send_hold_secs();
        if secs == 0 || email_of(request).is_none() {
            return Ok(None);
        }
        let source = Source::of(source);
        let sealed = Sealed {
            v: reins_proto::PROTOCOL_VERSION,
            request: request.clone(),
            source,
        };
        let plain =
            zeroize::Zeroizing::new(serde_json::to_vec(&sealed).map_err(|e| CoreError::storage(e.to_string()))?);
        let ciphertext = self.store.seal(&aad(&request.id.0), &plain)?;
        let body = HoldCreate {
            v: reins_proto::PROTOCOL_VERSION,
            request_id: request.id.clone(),
            connection_id: request.connection_id.0.clone(),
            sealed: data_encoding::BASE64URL_NOPAD.encode(&ciphertext),
            secs,
        };
        if body.sealed.len() > hold::MAX_SEALED_CHARS {
            return Err(CoreError::invalid("This email is too large to hold."));
        }
        let created = match api_call!(session, |api| api.create_hold(&body)) {
            Ok(created) => created,
            Err(e) if unsupported(&e) => {
                log::info!("this server has no send hold; the email is sent at once");
                return Ok(None);
            }
            Err(e) => return Err(e.into_core()),
        };
        let record = Record {
            id: created.id.clone(),
            request_id: request.id.0.clone(),
            secs,
            release_at: unix_now().saturating_add(i64::from(secs)),
            source,
            state: RecordState::Held,
            since: unix_now(),
            request: None,
        };
        if let Err(e) = self.update_holds(|records| records.push(record)) {
            // Not recorded here, so never released: take it back on the server too.
            if let Err(undo) = api_call!(session, |api| api.delete_hold(&created.id)) {
                log::warn!("a hold not recorded here stays on the server, never claimed: {}", undo.into_core());
            }
            return Err(e);
        }
        Ok(Some(Held {
            id: created.id,
            secs,
            send_at: created.release_at,
        }))
    }

    /// Answers the agent that request `request_id` is held, and logs it ("Sending in 30 s").
    pub(crate) async fn answer_held(
        &self,
        session: &Session,
        request_id: &str,
        request: &RelayRequest,
        held: &Held,
    ) -> Result<(), CoreError> {
        let email = email_of(request).ok_or_else(|| CoreError::invalid("not an email"))?;
        let detail = crate::handler::send_detail(email);
        let mut audit =
            self.audit(request, "send", "held", &format!("approved: {detail}"), None, email.recipients().count(), &[]);
        audit.info.hold = Some(held.id.clone());
        audit.info.note = Some(format!("Sending in {} s", held.secs));
        self.store.remove_pending(request_id)?;
        self.notifier.item_resolved(request_id.to_owned());
        let data = hold::held_result(
            &held.id,
            &reins_proto::ticket::ticket_of(&request.id),
            &crate::text::iso_utc(held.send_at),
            held.secs,
        );
        let answered = self
            .finish(
                session,
                request,
                audit.clone(),
                RelayOutcome::Result {
                    result: ToolResult::Connector {
                        data,
                    },
                },
            )
            .await;
        if let Err(e) = answered {
            // The hold stands (Undo works, the release sends); only the agent missed hearing of it.
            log::warn!("the agent could not be told its email is held: {e}");
            if !self.store.invocation_started(&request.id.0)? {
                self.store.append_audit(&audit)?;
                self.store.mark_handled(&request.id.0, unix_now())?;
            }
        }
        Ok(())
    }

    fn hold_audit(&self, request: &RelayRequest, hold_id: &str, outcome: &str, detail: &str) -> AuditRecord {
        let count = email_of(request).map_or(0, |e| e.recipients().count());
        let mut audit = self.audit(request, "send", outcome, detail, None, count, &[]);
        audit.info.hold = Some(hold_id.to_owned());
        audit
    }

    /// A hold that ended without a request to name (undone elsewhere, its record lost its email): logged by its ids.
    fn hold_note(&self, record: &Record, outcome: &str, detail: &str) {
        let request = RelayRequest {
            v: reins_proto::PROTOCOL_VERSION,
            id: reins_proto::ids::RequestId(record.request_id.clone()),
            connection_id: reins_proto::ids::ConnectionId(String::new()),
            connection_label: String::new(),
            created_at: record.since,
            wait_until: None,
            account: None,
            call: ToolCall::ListAccounts {
                service: Some("gmail".to_owned()),
                ask_for_more: false,
            },
            batch: None,
        };
        let request = record.request.clone().unwrap_or(request);
        if let Err(e) = self.store.append_audit(&self.hold_audit(&request, &record.id, outcome, detail)) {
            log::warn!("could not log a held send: {e}");
        }
    }

    /// Undo: a send waiting out its hold is not sent (the server deletes its ciphertext); a "Still send?" is dropped.
    pub async fn undo_send(&self, hold_id: &str) -> Result<(), CoreError> {
        self.require_decider()?;
        // Out of the records first: from now on nothing on this phone releases it, whatever the network does.
        let taken = self.update_holds(|records| {
            match records.iter().position(|r| r.id == hold_id && r.state != RecordState::Sending) {
                Some(at) => Ok(records.remove(at)),
                None if records.iter().any(|r| r.id == hold_id) => Err(CoreError::invalid("It is being sent.")),
                None => Err(CoreError::NotFound),
            }
        })??;
        if taken.state == RecordState::Held {
            let session = self.session()?;
            match api_call!(&session, |api| api.delete_hold(hold_id)) {
                Ok(())
                | Err(ApiFailure::Status {
                    status: 404,
                    ..
                }) => {}
                // It stays on the server until it is purged: never claimed, never sent.
                Err(e) => log::warn!(
                    "the held email is kept from sending, but the server could not delete it yet: {}",
                    e.into_core()
                ),
            }
        }
        let detail = if taken.state == RecordState::Late {
            "Not sent"
        } else {
            "Undone"
        };
        self.hold_note(&taken, "undone", detail);
        Ok(())
    }

    /// Releases the holds that are due; drops those undone elsewhere. Run by the hold push, the app's timer and every
    /// sync.
    pub async fn release_holds(&self) -> Result<HoldsReleased, CoreError> {
        let mut done = HoldsReleased::default();
        let records = self.hold_records()?;
        if records.is_empty() || !self.decides() {
            return Ok(done);
        }
        let session = self.session()?;
        let now = unix_now();
        // A process that ended mid-send: the email may or may not have gone. Never sent again.
        for r in records.iter().filter(|r| r.state == RecordState::Sending && now - r.since > SENDING_STALE_SECS) {
            if self.drop_hold(&r.id)?.is_some() {
                self.hold_note(r, "unknown", "The send outcome is unknown. Gmail may have sent it; check Sent.");
            }
        }
        let held: Vec<&Record> = records.iter().filter(|r| r.state == RecordState::Held).collect();
        if held.is_empty() {
            return Ok(done);
        }
        let waiting = api_call!(&session, |api| api.holds()).map_err(ApiFailure::into_core)?;
        for record in held {
            if !waiting.holds.iter().any(|h| h.id == record.id) {
                // Undone on the desktop app or by the agent (or purged): it is never sent.
                if self.drop_hold(&record.id)?.is_some() {
                    self.hold_note(record, "undone", "Undone");
                }
                continue;
            }
            if unix_now() < record.release_at {
                continue;
            }
            // One process of this phone claims: the one that moved it to Sending.
            if !self.move_hold(&record.id, RecordState::Held, RecordState::Sending)? {
                continue;
            }
            match Box::pin(self.release_one(&session, record)).await {
                Ok(Released::Sent) => done.sent += 1,
                Ok(Released::Late) => done.late += 1,
                Ok(Released::Not) => {}
                Err(e) => log::warn!("a held email could not be released yet: {e}"),
            }
        }
        Ok(done)
    }

    async fn release_one(&self, session: &Session, record: &Record) -> Result<Released, CoreError> {
        let claimed = match api_call!(session, |api| api.claim_hold(&record.id)) {
            Ok(claimed) => claimed,
            Err(ApiFailure::Status {
                status: 404,
                ..
            }) => {
                self.drop_hold(&record.id)?;
                self.hold_note(record, "undone", "Undone");
                return Ok(Released::Not);
            }
            Err(e) => {
                // Not claimed (not yet due on the server, offline): it waits as it was.
                self.move_hold(&record.id, RecordState::Sending, RecordState::Held)?;
                return Err(e.into_core());
            }
        };
        let opened = data_encoding::BASE64URL_NOPAD
            .decode(claimed.sealed.as_bytes())
            .ok()
            .and_then(|blob| self.store.unseal(&aad(&record.request_id), &blob).ok())
            .and_then(|plain| serde_json::from_slice::<Sealed>(&zeroize::Zeroizing::new(plain)).ok())
            // The ciphertext is bound to this request: another hold's (a server replaying an old one) never opens.
            .filter(|s| s.request.id.0 == record.request_id && claimed.request_id.0 == record.request_id);
        let Some(sealed) = opened else {
            self.drop_hold(&record.id)?;
            self.hold_note(record, "not_sent", "Not sent: the held email could not be opened on this phone.");
            return Ok(Released::Not);
        };
        let request = sealed.request;
        let late = claimed.late || hold::is_late(record.release_at, unix_now());
        if late {
            return self
                .ask_again(record, request, "Not sent: the phone was offline. Still send?")
                .map(|()| Released::Late);
        }
        if self.ap_mode_for(&request.connection_id.0, unix_now())? == crate::autopilot::AutopilotMode::Lockdown {
            self.drop_hold(&record.id)?;
            self.store.append_audit(&self.hold_audit(&request, &record.id, "not_sent", "Not sent: Lockdown is on."))?;
            return Ok(Released::Not);
        }
        // The policy as it is now: what needs the owner now is asked again rather than sent.
        if let Err(e) = self.authorization_execute(&request, sealed.source.authorization()) {
            log::info!("a held email needs the owner now: {e}");
            return self.ask_again(record, request, "Not sent: it needs you now. Still send?").map(|()| Released::Late);
        }
        self.send_held(session, &record.id, &request, sealed.source.authorization()).await.map(|()| Released::Sent)
    }

    /// Keeps the email here and asks the owner "Still send?" on its Activity entry.
    fn ask_again(&self, record: &Record, request: RelayRequest, detail: &str) -> Result<(), CoreError> {
        let audit = self.hold_audit(&request, &record.id, "not_sent", detail);
        let now = unix_now();
        let id = record.id.clone();
        self.update_holds(move |records| {
            if let Some(r) = records.iter_mut().find(|r| r.id == id) {
                r.state = RecordState::Late;
                r.since = now;
                r.request = Some(request);
            }
        })?;
        self.store.append_audit(&audit)?;
        Ok(())
    }

    /// The owner's "Still send?": sends a held email that was not sent in time, as the owner's own approval now.
    pub async fn send_late(&self, hold_id: &str) -> Result<(), CoreError> {
        let session = self.session()?;
        self.require_decider_fresh(&session).await?;
        if !self.move_hold(hold_id, RecordState::Late, RecordState::Sending)? {
            return Err(CoreError::NotFound);
        }
        let request = self.hold_records()?.into_iter().find(|r| r.id == hold_id).and_then(|r| r.request);
        let Some(request) = request else {
            self.drop_hold(hold_id)?;
            return Err(CoreError::NotFound);
        };
        let lockdown =
            self.ap_mode_for(&request.connection_id.0, unix_now())? == crate::autopilot::AutopilotMode::Lockdown;
        if lockdown {
            self.move_hold(hold_id, RecordState::Sending, RecordState::Late)?;
            return Err(CoreError::invalid("Lockdown is on."));
        }
        if let Err(e) = self.authorization_execute(&request, AuthorizationSource::OwnerReview) {
            self.move_hold(hold_id, RecordState::Sending, RecordState::Late)?;
            return Err(e);
        }
        self.send_held(&session, hold_id, &request, AuthorizationSource::OwnerReview).await
    }

    /// Sends a released hold, with the same checks an approval runs right before Gmail, and logs how it went. Its record
    /// goes whatever happens: never sent twice.
    async fn send_held(
        &self,
        session: &Session,
        hold_id: &str,
        request: &RelayRequest,
        source: AuthorizationSource,
    ) -> Result<(), CoreError> {
        let email = email_of(request).ok_or_else(|| CoreError::invalid("not an email"))?;
        let _fence = self.permission_execution_fence().await?;
        let attempt = match self.legacy_send_attempt(request) {
            Ok(attempt) => attempt,
            Err(e) => {
                self.drop_hold(hold_id)?;
                self.store.append_audit(&self.hold_audit(request, hold_id, "not_sent", &format!("Not sent: {e}")))?;
                return Err(e);
            }
        };
        if let Err(e) = self.legacy_send_durable().await {
            self.legacy_send_finished(attempt, false)?;
            self.move_hold(hold_id, RecordState::Sending, RecordState::Held)?;
            return Err(e);
        }
        let sent = self
            .gmail_for(crate::handler::account_of(request))
            .send_checked(email, || self.legacy_before_send(session, request, source))
            .await;
        self.legacy_send_finished(attempt, sent.is_ok())?;
        self.drop_hold(hold_id)?;
        let audit = match &sent {
            Ok(_) => {
                self.hold_audit(request, hold_id, "sent", &format!("Sent: {}", crate::handler::send_detail(email)))
            }
            Err(_) => self.hold_audit(
                request,
                hold_id,
                "unknown",
                "The send outcome is unknown. Gmail may have sent it; check Sent before sending it again.",
            ),
        };
        self.store.append_audit(&audit)?;
        sent.map(drop)
    }
}

enum Released {
    Sent,
    Late,
    Not,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_carry_no_email_while_held() {
        let r = Record {
            id: "h1".to_owned(),
            request_id: "r1".to_owned(),
            secs: 30,
            release_at: 130,
            source: Source::Automatic,
            state: RecordState::Held,
            since: 100,
            request: None,
        };
        let text = serde_json::to_string(&r).unwrap();
        assert!(!text.contains("request\""), "{text}");
        assert_eq!(serde_json::from_str::<Record>(&text).unwrap(), r);
    }

    #[test]
    fn sources_round_trip() {
        for s in [AuthorizationSource::OwnerReview, AuthorizationSource::Automatic, AuthorizationSource::TicketCollect]
        {
            assert_eq!(Source::of(s).authorization(), s);
        }
    }

    #[test]
    fn only_a_foreign_404_means_no_holds_here() {
        let status = |status: u16, code: &str| ApiFailure::Status {
            status,
            code: code.to_owned(),
            message: String::new(),
        };
        assert!(unsupported(&status(404, "")));
        assert!(!unsupported(&status(404, reins_proto::device::codes::NOT_FOUND)), "our own not-found is a failure");
        assert!(!unsupported(&status(409, "too_many_holds")));
    }
}
