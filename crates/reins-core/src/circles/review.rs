//! Circle reviews: the owner's holds (packets out, answers in, settled before anything runs) and the helper's reviews
//! (packets in, signed answers out).

use crate::session::api_call;
use reins_policy::{
    circles::{self as policy, Ballot, Obligations, Person, Settle, Votes},
    effects::{Certainty, EffectKind, Level},
};
use reins_proto::{
    canon::decode_payload,
    circle::{
        self, CircleAnswer, CircleKind, Class, Expect, NewAnswer, NewPacket, Notice, OnExpiry, PacketBody, PacketKind,
        PacketState, Redaction, ReviewCard, Side, SignedCircleAnswer, SignedPacket, Vote,
    },
};
use serde::{Deserialize, Serialize};

use reins_proto::relay::RelayRequest;

use super::{
    CircleEvent, CircleNoticeView, CircleProgress, CircleReviewState, CircleReviewView, CircleState, Me, Standing,
    new_id, sealed_to, verify,
};
use crate::{
    autopilot::context::{Decision, deciding},
    engine::Engine,
    error::CoreError,
    phone_api::ApiFailure,
    session::Session,
    store::unix_now,
    types::{ApprovalChoice, PendingItem},
};

/// One helper's part in a hold: its packet, and its answer once verified.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Part {
    pub arrangement: String,
    pub account: String,
    pub label: String,
    pub kind: CircleKind,
    pub class: Class,
    pub window_secs: u64,
    pub on_expiry: OnExpiry,
    pub redaction: Redaction,
    pub packet: String,
    pub nonce: String,
    pub sent: bool,
    pub vote: Option<Vote>,
    pub note: Option<String>,
    pub lapsed: bool,
    pub created_at: i64,
    pub deadline: i64,
}

/// A parked request an arrangement covers (the owner's side).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Hold {
    pub id: String,
    pub digest: String,
    pub owner_account: String,
    pub parts: Vec<Part>,
    /// Watchers told once it is settled (arrangement ids).
    pub watchers: Vec<String>,
    pub owner_vote: Option<bool>,
    /// The owner's picks, as tapped (what runs once the others approved).
    pub selected: Vec<String>,
    pub card: ReviewCard,
    pub deferred: bool,
    pub deadline: i64,
    /// The decision it ended with ("Approved", "Denied"), once settled here.
    pub outcome: Option<String>,
    /// How it runs once settled.
    #[serde(default)]
    pub finish: Finish,
}

/// The owner's tap, with what it picked.
#[derive(Clone, Copy)]
pub(crate) enum Tap<'a> {
    Request(&'a ApprovalChoice),
    Purchase(&'a crate::connector::payments::PurchaseChoice),
    Blob,
}

/// How a settled hold runs: through the owner's approval of a request, a purchase (with the owner's picks), or an
/// upload.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) enum Finish {
    #[default]
    Request,
    Purchase(Option<crate::connector::payments::PurchaseChoice>),
    Blob,
}

impl Hold {
    /// Somebody besides the owner decides.
    pub fn reviews(&self) -> bool {
        !self.parts.is_empty()
    }

    fn obligations(&self) -> Obligations {
        let mut ob = Obligations::default();
        for p in &self.parts {
            let person = Person {
                arrangement: p.arrangement.clone(),
                account: p.account.clone(),
                kind: p.kind,
                class: p.class,
                window_secs: p.window_secs,
                on_expiry: p.on_expiry,
            };
            match p.kind {
                CircleKind::Delegate => ob.delegates.push(person),
                CircleKind::Cosign | CircleKind::Guardian => ob.required.push(person),
                CircleKind::Watcher => ob.watchers.push(person),
            }
        }
        ob
    }

    fn votes(&self) -> Votes {
        Votes {
            owner: self.owner_vote,
            people: self
                .parts
                .iter()
                .filter_map(|p| {
                    p.vote.map(|vote| Ballot {
                        arrangement: p.arrangement.clone(),
                        account: p.account.clone(),
                        vote,
                    })
                })
                .collect(),
            lapsed: self.parts.iter().filter(|p| p.lapsed).map(|p| p.arrangement.clone()).collect(),
        }
    }

    pub fn settle(&self) -> Settle {
        policy::settle(&self.owner_account, &self.obligations(), &self.votes())
    }

    fn label_of(&self, account: &str) -> String {
        self.parts.iter().find(|p| p.account == account).map_or_else(|| account.to_owned(), |p| p.label.clone())
    }

    pub fn progress(&self) -> CircleProgress {
        let (owner, people) = match self.settle() {
            Settle::Waiting {
                owner,
                people,
            } => (owner, people),
            _ => (false, Vec::new()),
        };
        let mut needs: Vec<String> = people.iter().map(|a| self.label_of(a)).collect();
        needs.dedup();
        let approved: Vec<String> =
            self.parts.iter().filter(|p| p.vote == Some(Vote::Approve)).map(|p| p.label.clone()).collect();
        // A delegate's slot: the owner may still decide, so it isn't "waiting for you" when only delegates are asked.
        let delegates_only = self.parts.iter().all(|p| p.kind == CircleKind::Delegate);
        let line = match (needs.first(), owner, approved.first()) {
            (Some(who), true, _) if !delegates_only => format!("Needs {who} too"),
            (Some(who), false, _) => format!("Waiting for {who}"),
            (None, true, Some(who)) => format!("{who} approved, waiting for you"),
            _ if delegates_only && owner => {
                format!("{} can decide too", self.parts.first().map_or("Your delegate", |p| p.label.as_str()))
            }
            _ => String::new(),
        };
        CircleProgress {
            needs,
            approved,
            waiting_for_you: owner && !delegates_only,
            you_approved: self.owner_vote == Some(true),
            deferred: self.deferred,
            deadline: self.deadline,
            line,
        }
    }
}

/// A request of another account waiting for this one (the helper's side).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Review {
    pub id: String,
    pub link: String,
    pub person: String,
    pub kind: CircleKind,
    pub scope_line: String,
    pub card: ReviewCard,
    pub digest: String,
    pub nonce: String,
    pub deadline: i64,
    pub created_at: i64,
    pub state: CircleReviewState,
}

impl Review {
    pub fn view(&self) -> CircleReviewView {
        CircleReviewView {
            id: self.id.clone(),
            link_id: self.link.clone(),
            person: self.person.clone(),
            role: self.kind.into(),
            title: self.card.title.clone(),
            subtitle: self.card.subtitle.clone(),
            headline: self.card.headline.clone(),
            service: self.card.service.clone(),
            agent: self.card.agent.clone(),
            line: self.card.line.clone(),
            detail: self.card.detail.clone(),
            scope_line: self.scope_line.clone(),
            created_at: self.created_at,
            deadline: self.deadline,
            state: self.state,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct NoticeRow {
    pub id: String,
    pub link: String,
    pub person: String,
    pub notice: Notice,
}

/// What a summary shows (04 §5): the effects, the service and the agent; never the title, headline or subtitle, which
/// name subjects and recipients.
fn summary_card(full: &ReviewCard) -> ReviewCard {
    let service = if full.service.is_empty() {
        "a service"
    } else {
        full.service.as_str()
    };
    let agent = if full.agent.is_empty() {
        "An AI"
    } else {
        full.agent.as_str()
    };
    ReviewCard {
        title: format!("{agent} wants to act on {service}"),
        subtitle: String::new(),
        headline: String::new(),
        service: full.service.clone(),
        account: None,
        agent: full.agent.clone(),
        line: full.line.clone(),
        detail: None,
        created_at: full.created_at,
        deadline: full.deadline,
    }
    .cleaned()
}

/// Packets per page of the inbox (the server's), and how many pages a sync reads at most.
const PAGE: usize = 500;
const PAGES: usize = 10;

/// How long settled reviews and notices stay listed.
const KEEP_SECS: i64 = 7 * 86_400;

/// The deadline of `window` from `now`, kept within the Instant window the server holds requests for when Instant.
fn deadline_of(class: Class, window: u64, now: i64) -> i64 {
    now.saturating_add(i64::try_from(circle::clamp_window(class, Some(window))).unwrap_or(i64::MAX))
}

impl Engine {
    // ---- the owner --------------------------------------------------------------------------------------------------

    /// The obligations the owner's arrangements put on `request`, its effect line and the owner's account; `None`
    /// when no arrangement covers it.
    fn circle_obligations(&self, state: &CircleState, request: &RelayRequest) -> Option<(Obligations, String, String)> {
        // Arrangements in force bind (a guardianship until its end takes effect: its notice running, or a minor's
        // request waiting for consent, 04 §4); a suspended one (the helper's keys changed) binds too, and its requests
        // wait for the owner to end it (never fewer reviews because a key went missing). A paused one adds nothing.
        let now = unix_now();
        let all: Vec<&circle::Arrangement> = state
            .held
            .iter()
            .filter(|h| h.side == Side::Owner && (h.in_force(now) || h.standing == Standing::Suspended))
            .map(|h| &h.arrangement)
            .collect();
        if all.is_empty() {
            return None;
        }
        let note = self.ap_evaluate(request, false, unix_now(), false).ok().flatten().map(|(n, _)| n);
        let unknown = [EffectKind::Unknown];
        let (kinds, zone, level, certainty, line) = match &note {
            Some(n) if !n.kinds.is_empty() => (n.kinds.as_slice(), n.zone, n.level, Certainty::Full, n.line.clone()),
            // Couldn't be read: inside every co-signer's, guardian's and watcher's scope, no delegate's.
            _ => (&unknown[..], reins_policy::context::Zone::Outside, Level::L3, Certainty::None, String::new()),
        };
        let input = policy::Input {
            kinds,
            zone,
            level,
            certainty,
            connection: &request.connection_id.0,
            account: request.account.as_deref(),
        };
        let owner_account = all[0].owner.account.clone();
        let ob = policy::obligations(&owner_account, &all, &input);
        (!ob.is_empty() || !ob.watchers.is_empty()).then_some((ob, line, owner_account))
    }

    /// Whether a co-signer or a guardian must approve `request`. Then nothing runs without parking: no standing grant,
    /// managed permit, read grant or Bypass-minted grant, and no automatic mode (`authorization_automatic` and
    /// `permission_route` say review). Fails closed: Circle state that can't be read counts as yes.
    pub(crate) fn circle_requires_review(&self, request: &RelayRequest) -> bool {
        let slot = match self.circle_slot_now() {
            Ok(Some(slot)) => slot,
            Ok(None) => return false,
            Err(e) => {
                log::warn!("the Circle state could not be read; the request waits for review: {e}");
                return true;
            }
        };
        match self.circle_state(&slot) {
            Ok(state) => self.circle_obligations(&state, request).is_some_and(|(ob, ..)| !ob.required.is_empty()),
            Err(e) => {
                log::warn!("the Circle state could not be read; the request waits for review: {e}");
                true
            }
        }
    }

    /// A request was parked: the arrangements that cover it, as a hold. Local only (no network): a hold exists before
    /// Autopilot looks, so nothing a Circle must approve runs without it. Packets go out after ([`Engine::circle_send`]).
    pub(crate) fn circle_route(&self, id: &str) -> Result<bool, CoreError> {
        if self.circle_slot_now()?.is_none() {
            return Ok(false);
        }
        let (_, parked) = self.parked(id)?;
        let finish = if parked.connector.as_ref().is_some_and(|c| c.purchase.is_some()) {
            Finish::Purchase(None)
        } else {
            Finish::Request
        };
        let mut item = self.pending_request_item(&parked);
        self.names().item(&mut item);
        self.circle_route_with(id, &parked.request, &item, finish)
    }

    /// An upload arrived and waits for the owner: the arrangements that cover it, as a hold.
    pub(crate) fn circle_route_blob(
        &self,
        id: &str,
        request: &RelayRequest,
        item: &PendingItem,
    ) -> Result<bool, CoreError> {
        if self.circle_slot_now()?.is_none() {
            return Ok(false);
        }
        self.circle_route_with(id, request, item, Finish::Blob)
    }

    fn circle_route_with(
        &self,
        id: &str,
        request: &RelayRequest,
        item: &PendingItem,
        finish: Finish,
    ) -> Result<bool, CoreError> {
        let Some(slot) = self.circle_slot_now()? else {
            return Ok(false);
        };
        let state = self.circle_state(&slot)?;
        if state.holds.iter().any(|h| h.id == id) {
            return Ok(state.holds.iter().any(|h| h.id == id && h.reviews()));
        }
        let Some((ob, line, owner_account)) = self.circle_obligations(&state, request) else {
            return Ok(false);
        };
        let now = unix_now();
        let mut parts = Vec::new();
        for p in ob.reviewers() {
            let held = state.held(&p.arrangement).ok_or(CoreError::NotFound)?;
            parts.push(Part {
                arrangement: p.arrangement.clone(),
                account: p.account.clone(),
                label: held.other().label.clone(),
                kind: p.kind,
                class: p.class,
                window_secs: p.window_secs,
                on_expiry: p.on_expiry,
                redaction: held.arrangement.terms.redaction,
                packet: new_id()?,
                nonce: circle::encode_nonce16(&crate::crypto::random_bytes::<16>()?),
                sent: false,
                vote: None,
                note: None,
                // A suspended arrangement can't be answered: its window is over at once.
                lapsed: held.standing == Standing::Suspended,
                created_at: now,
                // Each helper's own window (its arrangement's class and length).
                deadline: deadline_of(p.class, p.window_secs, now),
            });
        }
        let deadline = parts.iter().map(|p| p.deadline).min().unwrap_or_else(|| deadline_of(Class::Deferred, 0, now));
        let card = ReviewCard {
            title: item.title.clone(),
            subtitle: item.subtitle.clone(),
            headline: item.headline.clone(),
            service: item.service.clone(),
            account: item.account.clone(),
            agent: item.connection_label.clone(),
            line,
            detail: (!item.subtitle.is_empty()).then(|| item.subtitle.clone()),
            created_at: now,
            deadline,
        }
        .cleaned();
        let hold = Hold {
            id: id.to_owned(),
            digest: reins_proto::ticket::digest_of(request),
            owner_account,
            parts,
            watchers: ob.watchers.iter().map(|w| w.arrangement.clone()).collect(),
            owner_vote: None,
            selected: Vec::new(),
            card,
            deferred: ob.class() == Class::Deferred,
            deadline,
            outcome: None,
            finish,
        };
        let reviews = hold.reviews();
        self.with_circles(&slot, |s| {
            if !s.holds.iter().any(|h| h.id == hold.id) {
                s.holds.push(hold);
            }
            Ok(())
        })?;
        Ok(reviews)
    }

    /// The hold of request `id`, when somebody besides the owner decides it. An unreadable state is an error (the
    /// callers fail closed).
    pub(crate) fn circle_hold(&self, id: &str) -> Result<Option<Hold>, CoreError> {
        let Some(slot) = self.circle_slot_now()? else {
            return Ok(None);
        };
        Ok(self.circle_state(&slot)?.holds.into_iter().find(|h| h.id == id && h.reviews()))
    }

    /// The hold waits for others (the owner's tap is in, or isn't needed): an approval didn't run.
    pub(crate) fn circle_waiting(&self, id: &str) -> bool {
        match self.circle_hold(id) {
            Ok(Some(h)) => h.outcome.is_none(),
            Ok(None) => false,
            Err(_) => true,
        }
    }

    /// The owner's card line ("Needs Alex too").
    pub(crate) fn circle_progress(&self, id: &str) -> Option<CircleProgress> {
        self.circle_hold(id).ok().flatten().map(|h| h.progress())
    }

    /// Sends what a hold still owes: its packets; a Deferred hold becomes a ticket first.
    pub(crate) async fn circle_send(&self, session: &Session, id: &str) -> Result<(), CoreError> {
        let Some(hold) = self.circle_hold(id)? else {
            return Ok(());
        };
        let me = self.circle_me(session).await?;
        if hold.deferred && self.store.ticket(id)?.is_none() {
            let window = hold.parts.iter().map(|p| p.window_secs).min();
            if let Err(e) = self.defer_for(id, window).await {
                log::warn!("a Circle request could not become a ticket: {e}");
            }
        }
        for part in hold.parts.iter().filter(|p| !p.sent && !p.lapsed) {
            match self.circle_send_part(session, &me, &hold, part).await {
                Ok(()) => {
                    self.with_circles(&me.slot, |s| {
                        if let Some(p) = s
                            .holds
                            .iter_mut()
                            .find(|h| h.id == hold.id)
                            .and_then(|h| h.parts.iter_mut().find(|p| p.packet == part.packet))
                        {
                            p.sent = true;
                        }
                        Ok(())
                    })?;
                }
                Err(e) => log::warn!("a Circle review could not be sent: {e}"),
            }
        }
        Ok(())
    }

    async fn circle_send_part(&self, session: &Session, me: &Me, hold: &Hold, part: &Part) -> Result<(), CoreError> {
        let peer = self.circle_peer(session, &me.slot, &part.arrangement).await?;
        let mut card = match part.redaction {
            Redaction::Full => hold.card.clone(),
            Redaction::Summary => summary_card(&hold.card),
        };
        card.deadline = part.deadline;
        let body = PacketBody {
            v: circle::version(),
            id: part.packet.clone(),
            arrangement: part.arrangement.clone(),
            kind: PacketKind::Review,
            digest: hold.digest.clone(),
            nonce: part.nonce.clone(),
            card: Some(card),
            notice: None,
            device_id: me.device_id.clone(),
            epoch: me.epoch,
            entry_hash: me.entry_hash.clone(),
            at: unix_now(),
        };
        self.circle_post(session, &peer.box_pub, &body, part.deadline).await
    }

    async fn circle_post(
        &self,
        session: &Session,
        to: &str,
        body: &PacketBody,
        deadline: i64,
    ) -> Result<(), CoreError> {
        body.validate().map_err(|e| CoreError::invalid(e.to_string()))?;
        let json = serde_json::to_string(body).map_err(|e| CoreError::storage(e.to_string()))?;
        let signed = SignedPacket {
            sig: self.sign_key()?.sign_b64(&circle::packet_message(&json)),
            body: json,
        };
        let packet = NewPacket {
            v: circle::version(),
            id: body.id.clone(),
            link: body.arrangement.clone(),
            kind: body.kind,
            deadline,
            sealed: sealed_to(to, &serde_json::to_vec(&signed).map_err(|e| CoreError::storage(e.to_string()))?)?,
        };
        api_call!(session, |api| api.circle_packet(&packet)).map_err(ApiFailure::into_core)
    }

    /// At the top of an approval: the owner's tap fills the owner's slot. `true`: settle approves, go on; `false`: the
    /// request waits for the others (nothing runs). Auto, Bypass and Overnight never fill a Circle request's slots.
    pub(crate) fn circle_gate(
        &self,
        id: &str,
        tap: Tap<'_>,
        source: crate::authorization::AuthorizationSource,
    ) -> Result<bool, CoreError> {
        use crate::authorization::AuthorizationSource;
        let Some(slot) = self.circle_slot_now()? else {
            return Ok(true);
        };
        let Some(hold) = self.circle_hold(id)? else {
            return Ok(true);
        };
        if hold.outcome.as_deref() == Some("Approved") {
            return Ok(true);
        }
        match source {
            AuthorizationSource::OwnerReview => {}
            AuthorizationSource::Automatic | AuthorizationSource::TicketCollect => {
                return Err(CoreError::invalid("This request needs your Circle first."));
            }
        }
        let settled = self.with_circles(&slot, |s| {
            let Some(h) = s.holds.iter_mut().find(|h| h.id == id) else {
                return Ok(Settle::Approved);
            };
            h.owner_vote = Some(true);
            match tap {
                Tap::Request(choice) => choice.selected_message_ids.clone_into(&mut h.selected),
                // What pays and where it goes, as the owner picked it: what runs once the others approved.
                Tap::Purchase(choice) => h.finish = Finish::Purchase(Some(choice.clone())),
                Tap::Blob => {}
            }
            let settled = h.settle();
            if settled == Settle::Approved {
                h.outcome = Some("Approved".to_owned());
            }
            Ok(settled)
        })?;
        match settled {
            Settle::Approved => Ok(true),
            Settle::Denied {
                ..
            } => Err(CoreError::invalid("Someone in your Circle denied this.")),
            Settle::Waiting {
                ..
            } => {
                self.circle_refresh_card(id);
                Ok(false)
            }
        }
    }

    /// The owner's card changed (progress): the app refreshes it without a new notification.
    fn circle_refresh_card(&self, _id: &str) {
        self.notifier.circle_changed(CircleEvent::Changed);
    }

    /// The owner denied (or Lockdown did): the hold is settled; its packets are withdrawn on the next sync.
    pub(crate) fn circle_denied(&self, id: &str) {
        let Ok(Some(slot)) = self.circle_slot_now() else {
            return;
        };
        if let Err(e) = self.with_circles(&slot, |s| {
            if let Some(h) = s.holds.iter_mut().find(|h| h.id == id) {
                h.owner_vote = Some(false);
                h.outcome.get_or_insert_with(|| "Denied".to_owned());
            }
            Ok(())
        }) {
            log::warn!("Circle state: {e}");
        }
    }

    /// Takes the helpers' answers: each verified against its arrangement, its packet, the request's digest and the
    /// helper's current approval phone, once; then settles what they complete.
    pub(crate) async fn circle_take_answers(&self, session: &Session, me: &Me) -> Result<(), CoreError> {
        // A batch at a time: each one taken is marked seen, so the next batch is the rest.
        for _ in 0..PAGES {
            let more = Box::pin(self.circle_take_answer_batch(session, me)).await?;
            if !more {
                break;
            }
        }
        Ok(())
    }

    /// One batch of answers; `true` when the server may have more.
    async fn circle_take_answer_batch(&self, session: &Session, me: &Me) -> Result<bool, CoreError> {
        let items = api_call!(session, |api| api.circle_answers()).map_err(ApiFailure::into_core)?;
        if items.is_empty() {
            return Ok(false);
        }
        let more = items.len() >= PAGE;
        let mut seen = Vec::new();
        let mut touched = Vec::new();
        for item in items {
            let state = self.circle_state(&me.slot)?;
            let found = state.holds.iter().find_map(|h| {
                h.parts
                    .iter()
                    .find(|p| p.packet == item.packet && p.arrangement == item.link)
                    .map(|p| (h.clone(), p.clone()))
            });
            let Some((hold, part)) = found else {
                seen.push(item.packet);
                continue;
            };
            if part.vote.is_some() || part.lapsed {
                seen.push(item.packet);
                continue;
            }
            match item.state {
                PacketState::Answered => match self.circle_check_answer(session, me, &hold, &part, &item).await {
                    Ok(answer) => {
                        self.with_circles(&me.slot, |s| {
                            if let Some(p) = s
                                .holds
                                .iter_mut()
                                .find(|h| h.id == hold.id)
                                .and_then(|h| h.parts.iter_mut().find(|p| p.packet == part.packet))
                                && p.vote.is_none()
                            {
                                p.vote = Some(answer.vote);
                                p.note.clone_from(&answer.note);
                                // The ward sees every decision its guardian made, and the note (04 §4).
                                if p.kind == CircleKind::Guardian {
                                    s.guardian_decisions.push(super::guardian::GuardianDecision {
                                        link: p.arrangement.clone(),
                                        at: unix_now(),
                                        title: hold.card.title.clone(),
                                        approved: answer.vote == Vote::Approve,
                                        note: answer.note.clone(),
                                    });
                                    super::guardian::trim_decisions(&mut s.guardian_decisions);
                                }
                            }
                            Ok(())
                        })?;
                        touched.push(hold.id.clone());
                        seen.push(item.packet);
                    }
                    // A network failure is tried again; a refused answer is seen (and never counts).
                    Err(CoreError::Network {
                        ..
                    }) => {}
                    Err(e) => {
                        log::warn!("a Circle answer was refused: {e}");
                        seen.push(item.packet);
                    }
                },
                // The server says its window ran out: believed only once this phone's clock agrees (a server can't
                // cut a helper's window short); until then the answer may still come.
                PacketState::Expired if unix_now() < part.deadline => seen.push(item.packet),
                PacketState::Expired => {
                    self.with_circles(&me.slot, |s| {
                        if let Some(p) = s
                            .holds
                            .iter_mut()
                            .find(|h| h.id == hold.id)
                            .and_then(|h| h.parts.iter_mut().find(|p| p.packet == part.packet))
                        {
                            p.lapsed = true;
                        }
                        Ok(())
                    })?;
                    touched.push(hold.id.clone());
                    seen.push(item.packet);
                }
                PacketState::Withdrawn | PacketState::Waiting => seen.push(item.packet),
            }
        }
        if !seen.is_empty()
            && let Err(e) = api_call!(session, |api| api.circle_seen(seen.clone()))
        {
            log::warn!("Circle answers could not be marked seen: {e:?}");
        }
        touched.dedup();
        for id in touched {
            Box::pin(self.circle_settle(session, me, &id)).await;
        }
        Ok(more)
    }

    async fn circle_check_answer(
        &self,
        session: &Session,
        me: &Me,
        hold: &Hold,
        part: &Part,
        item: &circle::AnswerItem,
    ) -> Result<CircleAnswer, CoreError> {
        // A paused or ended arrangement takes no answer, whenever it was signed.
        let row = self.circle_state(&me.slot)?.held(&part.arrangement).cloned().ok_or(CoreError::NotFound)?;
        if row.standing != Standing::Active || row.side != Side::Owner {
            return Err(CoreError::invalid("the arrangement is not active"));
        }
        let sealed = item.sealed.as_deref().ok_or_else(|| CoreError::invalid("no answer"))?;
        let raw = decode_payload(sealed).ok_or_else(|| CoreError::invalid("bad answer"))?;
        let plain = self.box_key()?.open(&raw).map_err(|_| CoreError::invalid("the answer doesn't open"))?;
        let signed: SignedCircleAnswer =
            serde_json::from_slice(&plain).map_err(|_| CoreError::invalid("bad answer"))?;
        let peer = self.circle_peer(session, &me.slot, &part.arrangement).await?;
        let expect = Expect {
            arrangement: &part.arrangement,
            packet: &part.packet,
            digest: &hold.digest,
            packet_nonce: &part.nonce,
            created_at: part.created_at,
            deadline: part.deadline,
            signer: circle::Signer {
                device_id: &peer.device_id,
                sign_pub: &peer.sign_pub,
                epoch: peer.epoch,
                entry_hash: &peer.hash,
            },
        };
        circle::check_answer(&signed, &expect, verify).map_err(|e| CoreError::invalid(e.to_string()))
    }

    /// Settles a hold: denies, runs (through the owner's own approval path), or refreshes the card.
    pub(crate) async fn circle_settle(&self, session: &Session, me: &Me, id: &str) {
        // Any hold, also one whose parts were all dropped (the owner ended the arrangement after tapping Approve).
        let Some(hold) = self.circle_state(&me.slot).ok().and_then(|s| s.holds.into_iter().find(|h| h.id == id)) else {
            return;
        };
        if hold.outcome.is_some() && !hold.reviews() {
            return;
        }
        if self.store.pending_item(id, unix_now()).ok().flatten().is_none() {
            return;
        }
        match hold.settle() {
            Settle::Denied {
                by,
                lapsed,
            } => {
                let note = match (&by, lapsed) {
                    (_, true) => "Not approved in time".to_owned(),
                    (Some(account), _) => {
                        let who = hold.label_of(account);
                        let said = hold.parts.iter().find(|p| &p.account == account).and_then(|p| p.note.clone());
                        match said {
                            Some(n) => format!("Denied by {who}: {n}"),
                            None => format!("Denied by {who}"),
                        }
                    }
                    (None, false) => return,
                };
                self.circle_denied(id);
                let Some(_claim) = self.claim(id) else {
                    return;
                };
                let decision = Decision::new("circle", None).by_circle(note.clone());
                let result = match hold.finish {
                    Finish::Blob => deciding(decision, Box::pin(self.answer_blob_claimed(id, false))).await.0,
                    Finish::Request | Finish::Purchase(_) => {
                        deciding(decision, Box::pin(self.deny_claimed_with(id, Some(note)))).await.0
                    }
                };
                if let Err(e) = result {
                    log::warn!("a Circle denial could not be answered: {e}");
                }
            }
            Settle::Approved => {
                let Some(_claim) = self.claim(id) else {
                    return;
                };
                let who: Vec<String> =
                    hold.parts.iter().filter(|p| p.vote == Some(Vote::Approve)).map(|p| p.label.clone()).collect();
                let line = format!("Approved by {}", who.join(", "));
                let choice = ApprovalChoice {
                    selected_message_ids: hold.selected.clone(),
                    standing: None,
                };
                // The delegate (or everyone) approved: the owner's slot is filled; this runs as the owner's review.
                if let Err(e) = self.with_circles(&me.slot, |s| {
                    if let Some(h) = s.holds.iter_mut().find(|h| h.id == id) {
                        h.outcome = Some("Approved".to_owned());
                    }
                    Ok(())
                }) {
                    log::warn!("Circle state: {e}");
                    return;
                }
                let decision = if who.is_empty() {
                    Decision::new("", None)
                } else {
                    Decision::new("circle", None).by_circle(line)
                };
                let result = match &hold.finish {
                    Finish::Request => {
                        deciding(
                            decision,
                            Box::pin(self.approve_claimed(
                                id,
                                choice,
                                crate::authorization::AuthorizationSource::OwnerReview,
                            )),
                        )
                        .await
                        .0
                    }
                    // A purchase pays only with the owner's own picks, from the purchase screen.
                    Finish::Purchase(Some(picked)) => {
                        let work = async {
                            let (_, parked) = self.parked(id)?;
                            self.approve_parked_purchase(session, id, &parked, Some(picked)).await
                        };
                        deciding(decision, Box::pin(work)).await.0
                    }
                    Finish::Purchase(None) => Err(CoreError::invalid("waits for the owner's picks")),
                    Finish::Blob => deciding(decision, Box::pin(self.answer_blob_claimed(id, true))).await.0,
                };
                if let Err(e) = result {
                    log::warn!("a request the Circle approved could not run: {e}");
                }
            }
            Settle::Waiting {
                ..
            } => self.circle_refresh_card(id),
        }
    }

    /// Windows that ran out; holds whose request is gone (withdraw their packets, tell watchers); packets not sent yet.
    pub(crate) async fn circle_expire(&self, session: &Session, me: &Me) {
        let now = unix_now();
        let Ok(state) = self.circle_state(&me.slot) else {
            return;
        };
        for hold in &state.holds {
            let pending = self.store.pending_item(&hold.id, now).ok().flatten().is_some();
            if !pending {
                for part in hold.parts.iter().filter(|p| p.sent && p.vote.is_none() && !p.lapsed) {
                    match api_call!(session, |api| api.circle_withdraw(&part.packet)) {
                        Ok(())
                        | Err(ApiFailure::Status {
                            status: 404 | 409,
                            ..
                        }) => {}
                        Err(e) => log::info!("a Circle packet could not be withdrawn: {e:?}"),
                    }
                }
                self.circle_tell_watchers(session, me, hold).await;
                if let Err(e) = self.with_circles(&me.slot, |s| {
                    s.holds.retain(|h| h.id != hold.id);
                    Ok(())
                }) {
                    log::warn!("Circle state: {e}");
                }
                continue;
            }
            let lapsing = hold.parts.iter().any(|p| p.vote.is_none() && !p.lapsed && p.deadline <= now);
            if lapsing {
                if let Err(e) = self.with_circles(&me.slot, |s| {
                    if let Some(h) = s.holds.iter_mut().find(|h| h.id == hold.id) {
                        for p in h.parts.iter_mut().filter(|p| p.vote.is_none() && p.deadline <= now) {
                            p.lapsed = true;
                        }
                    }
                    Ok(())
                }) {
                    log::warn!("Circle state: {e}");
                }
                Box::pin(self.circle_settle(session, me, &hold.id)).await;
            } else if hold.parts.iter().any(|p| !p.sent && !p.lapsed)
                && let Err(e) = self.circle_send(session, &hold.id).await
            {
                log::info!("Circle packets: {e}");
            }
        }
    }

    /// An arrangement was paused or ended: a delegate's or a co-signer's part of what it held is dropped (the owner
    /// decides alone; it is the owner's account), a guardian's part lapses (denied: a ward ending a guardianship never
    /// approves what waits, 08 R7). Then each settles.
    pub(crate) async fn circle_release(&self, session: &Session, me: &Me, link: &str) {
        let Ok(state) = self.circle_state(&me.slot) else {
            return;
        };
        let ids: Vec<String> = state
            .holds
            .iter()
            .filter(|h| h.parts.iter().any(|p| p.arrangement == link))
            .map(|h| h.id.clone())
            .collect();
        if let Err(e) = self.with_circles(&me.slot, |s| {
            for h in &mut s.holds {
                for p in h.parts.iter_mut().filter(|p| p.arrangement == link && p.kind == CircleKind::Guardian) {
                    if p.vote != Some(Vote::Deny) {
                        p.vote = None;
                        p.lapsed = true;
                    }
                }
                h.parts.retain(|p| p.arrangement != link || p.kind == CircleKind::Guardian);
            }
            for r in s.reviews.iter_mut().filter(|r| r.link == link && r.state == CircleReviewState::Waiting) {
                r.state = CircleReviewState::Withdrawn;
            }
            Ok(())
        }) {
            log::warn!("Circle state: {e}");
        }
        for id in ids {
            Box::pin(self.circle_settle(session, me, &id)).await;
        }
    }

    /// Watchers hear what was decided (no buttons, no notification).
    async fn circle_tell_watchers(&self, session: &Session, me: &Me, hold: &Hold) {
        let outcome = hold.outcome.clone().unwrap_or_else(|| "Decided".to_owned());
        for link in &hold.watchers {
            let notice = Notice {
                text: circle::shown_text(&format!("{outcome}: {}", hold.card.title), circle::MAX_CARD_LINE, false),
                outcome: Some(outcome.clone()),
                at: unix_now(),
                lines: Vec::new(),
            };
            if let Err(e) = self.circle_notice(session, me, link, notice).await {
                log::info!("a Circle notice could not be sent: {e}");
            }
        }
    }

    /// One notice to the watcher (or helper) of `link`.
    pub(crate) async fn circle_notice(
        &self,
        session: &Session,
        me: &Me,
        link: &str,
        notice: Notice,
    ) -> Result<(), CoreError> {
        let peer = self.circle_peer(session, &me.slot, link).await?;
        let now = unix_now();
        let body = PacketBody {
            v: circle::version(),
            id: new_id()?,
            arrangement: link.to_owned(),
            kind: PacketKind::Notice,
            digest: reins_proto::canon::hex32(&[0; 32]),
            nonce: circle::encode_nonce16(&crate::crypto::random_bytes::<16>()?),
            card: None,
            notice: Some(notice),
            device_id: me.device_id.clone(),
            epoch: me.epoch,
            entry_hash: me.entry_hash.clone(),
            at: now,
        };
        self.circle_post(session, &peer.box_pub, &body, now + KEEP_SECS).await
    }

    /// Bypass turned on: every watcher hears it (04 §6.3: "Sam turned on Bypass for 1 h").
    pub(crate) async fn circle_bypass_on(&self, text: &str) {
        let Ok(session) = self.session() else {
            return;
        };
        let Ok(Some(slot)) = self.circle_slot_now() else {
            return;
        };
        let watchers: Vec<String> = match self.circle_state(&slot) {
            Ok(s) => s
                .held
                .iter()
                .filter(|h| {
                    h.side == Side::Owner
                        && h.standing == Standing::Active
                        && h.arrangement.terms.kind == CircleKind::Watcher
                })
                .map(|h| h.arrangement.id.clone())
                .collect(),
            Err(_) => return,
        };
        if watchers.is_empty() {
            return;
        }
        let Ok(me) = self.circle_me(&session).await else {
            return;
        };
        for link in watchers {
            let notice = Notice {
                text: circle::shown_text(text, circle::MAX_CARD_LINE, false),
                outcome: None,
                at: unix_now(),
                lines: Vec::new(),
            };
            if let Err(e) = self.circle_notice(&session, &me, &link, notice).await {
                log::info!("a Circle notice could not be sent: {e}");
            }
        }
    }

    // ---- the helper -------------------------------------------------------------------------------------------------

    /// Takes the packets addressed to this account: each opened, checked against the owner's current approval phone and
    /// its arrangement, and kept. A new review is the one Circle notification.
    pub(crate) async fn circle_take_inbox(&self, session: &Session, me: &Me) -> Result<(), CoreError> {
        // Every page (waiting packets come first), up to a bound.
        let mut items = Vec::new();
        for page in 0..PAGES {
            let got = api_call!(session, |api| api.circle_inbox(page * PAGE)).map_err(ApiFailure::into_core)?;
            let full = got.len() >= PAGE;
            items.extend(got);
            if !full {
                break;
            }
        }
        let mut changed = false;
        for item in items {
            let state = self.circle_state(&me.slot)?;
            let known_review = state.reviews.iter().find(|r| r.id == item.id).cloned();
            if let Some(review) = known_review {
                let to = match item.state {
                    PacketState::Withdrawn => Some(CircleReviewState::Withdrawn),
                    PacketState::Expired => Some(CircleReviewState::Expired),
                    _ => None,
                };
                if let Some(to) = to
                    && review.state == CircleReviewState::Waiting
                {
                    self.with_circles(&me.slot, |s| {
                        if let Some(r) = s.reviews.iter_mut().find(|r| r.id == item.id) {
                            r.state = to;
                        }
                        Ok(())
                    })?;
                    changed = true;
                }
                continue;
            }
            if state.notices.iter().any(|n| n.id == item.id) || item.state != PacketState::Waiting {
                continue;
            }
            let Some(held) = state.held(&item.link).cloned() else {
                continue;
            };
            if held.side != Side::Helper || held.standing != Standing::Active {
                continue;
            }
            match self.circle_open_packet(session, me, &item).await {
                Ok(body) => {
                    let person = held.arrangement.owner.label.clone();
                    match (body.kind, body.card, body.notice) {
                        (PacketKind::Review, Some(card), None) => {
                            if held.arrangement.terms.kind == CircleKind::Watcher {
                                continue;
                            }
                            let review = Review {
                                id: body.id,
                                link: item.link.clone(),
                                person,
                                kind: held.arrangement.terms.kind,
                                scope_line: held.arrangement.terms.scope_line.clone(),
                                card,
                                digest: body.digest,
                                nonce: body.nonce,
                                deadline: item.deadline,
                                created_at: item.created_at,
                                state: CircleReviewState::Waiting,
                            };
                            let view = review.view();
                            self.with_circles(&me.slot, |s| {
                                if !s.reviews.iter().any(|r| r.id == review.id) {
                                    s.reviews.push(review);
                                }
                                Ok(())
                            })?;
                            self.notifier.circle_changed(CircleEvent::ReviewWaiting {
                                review: view,
                            });
                        }
                        (PacketKind::Notice, None, Some(notice)) => {
                            self.with_circles(&me.slot, |s| {
                                if !s.notices.iter().any(|n| n.id == body.id) {
                                    s.notices.push(NoticeRow {
                                        id: body.id,
                                        link: item.link.clone(),
                                        person,
                                        notice,
                                    });
                                }
                                Ok(())
                            })?;
                            changed = true;
                        }
                        _ => {}
                    }
                }
                Err(e) => log::warn!("a Circle packet was refused: {e}"),
            }
        }
        let now = unix_now();
        self.with_circles(&me.slot, |s| {
            s.reviews.retain(|r| r.state == CircleReviewState::Waiting || r.deadline > now - KEEP_SECS);
            s.notices.retain(|n| n.notice.at > now - KEEP_SECS);
            Ok(())
        })?;
        if changed {
            self.notifier.circle_changed(CircleEvent::Changed);
        }
        Ok(())
    }

    async fn circle_open_packet(
        &self,
        session: &Session,
        me: &Me,
        item: &circle::InboxItem,
    ) -> Result<PacketBody, CoreError> {
        let raw = decode_payload(&item.sealed).ok_or_else(|| CoreError::invalid("bad packet"))?;
        let plain = self.box_key()?.open(&raw).map_err(|_| CoreError::invalid("the packet doesn't open"))?;
        let signed: SignedPacket = serde_json::from_slice(&plain).map_err(|_| CoreError::invalid("bad packet"))?;
        let peer = self.circle_peer(session, &me.slot, &item.link).await?;
        let body = signed
            .verify(
                &circle::Signer {
                    device_id: &peer.device_id,
                    sign_pub: &peer.sign_pub,
                    epoch: peer.epoch,
                    entry_hash: &peer.hash,
                },
                verify,
            )
            .map_err(|e| CoreError::invalid(e.to_string()))?;
        if body.id != item.id || body.arrangement != item.link || body.kind != item.kind {
            return Err(CoreError::invalid("the packet is not the one the server named"));
        }
        Ok(body)
    }

    pub(crate) fn circle_reviews(&self) -> Result<Vec<CircleReviewView>, CoreError> {
        let Some(slot) = self.circle_slot_now()? else {
            return Ok(Vec::new());
        };
        let mut out: Vec<CircleReviewView> = self.circle_state(&slot)?.reviews.iter().map(Review::view).collect();
        out.sort_by_key(|r| (r.state != CircleReviewState::Waiting, r.deadline));
        Ok(out)
    }

    pub(crate) fn circle_notices(&self) -> Result<Vec<CircleNoticeView>, CoreError> {
        let Some(slot) = self.circle_slot_now()? else {
            return Ok(Vec::new());
        };
        let mut out: Vec<CircleNoticeView> = self
            .circle_state(&slot)?
            .notices
            .iter()
            .map(|n| CircleNoticeView {
                id: n.id.clone(),
                person: n.person.clone(),
                text: n.notice.text.clone(),
                outcome: n.notice.outcome.clone(),
                at: n.notice.at,
                lines: n.notice.lines.clone(),
            })
            .collect();
        out.sort_by_key(|n| std::cmp::Reverse(n.at));
        Ok(out)
    }

    /// The helper decides (on its approval phone only): a signed answer, sealed to the owner's approval phone.
    pub(crate) async fn circle_answer(&self, id: &str, approve: bool, note: Option<String>) -> Result<(), CoreError> {
        let session = self.session()?;
        let me = self.circle_me(&session).await?;
        let state = self.circle_state(&me.slot)?;
        let review = state.reviews.iter().find(|r| r.id == id).cloned().ok_or(CoreError::NotFound)?;
        if review.state != CircleReviewState::Waiting {
            return Err(CoreError::invalid("This was already settled."));
        }
        if review.deadline <= unix_now() {
            return Err(CoreError::invalid("This waited too long; it can't be answered now."));
        }
        let held = state.held(&review.link).cloned().ok_or(CoreError::NotFound)?;
        if held.standing != Standing::Active {
            return Err(CoreError::invalid("This arrangement isn't active."));
        }
        let peer = self.circle_peer(&session, &me.slot, &review.link).await?;
        let note = note.map(|n| circle::shown_text(n.trim(), circle::MAX_NOTE_CHARS, false)).filter(|n| !n.is_empty());
        let answer = CircleAnswer {
            v: circle::version(),
            arrangement: review.link.clone(),
            packet: review.id.clone(),
            digest: review.digest.clone(),
            packet_nonce: review.nonce.clone(),
            vote: if approve {
                Vote::Approve
            } else {
                Vote::Deny
            },
            note,
            at: unix_now(),
            nonce: circle::encode_nonce16(&crate::crypto::random_bytes::<16>()?),
            epoch: me.epoch,
            entry_hash: me.entry_hash.clone(),
            device_id: me.device_id.clone(),
        };
        answer.validate().map_err(|e| CoreError::invalid(e.to_string()))?;
        let body = serde_json::to_string(&answer).map_err(|e| CoreError::storage(e.to_string()))?;
        let signed = SignedCircleAnswer {
            sig: self.sign_key()?.sign_b64(&circle::answer_message(&body)),
            body,
        };
        let new = NewAnswer {
            v: circle::version(),
            sealed: sealed_to(
                &peer.box_pub,
                &serde_json::to_vec(&signed).map_err(|e| CoreError::storage(e.to_string()))?,
            )?,
        };
        api_call!(&session, |api| api.circle_answer(id, &new)).map_err(|e| match e {
            ApiFailure::Status {
                status: 409,
                ..
            } => CoreError::invalid("This was already settled."),
            other => other.into_core(),
        })?;
        self.with_circles(&me.slot, |s| {
            if let Some(r) = s.reviews.iter_mut().find(|r| r.id == id) {
                r.state = if approve {
                    CircleReviewState::Approved
                } else {
                    CircleReviewState::Denied
                };
            }
            Ok(())
        })?;
        self.circle_receipt(&review, approve);
        self.notifier.circle_changed(CircleEvent::Changed);
        Ok(())
    }

    /// The helper's Activity keeps "Approved for Dana: <title>" (04 §5 receipts).
    fn circle_receipt(&self, review: &Review, approve: bool) {
        let verb = if approve {
            "Approved"
        } else {
            "Denied"
        };
        let mut info = crate::store::AuditInfo {
            note: Some(format!("{verb} for {}", review.person)),
            decided_by: "circle".to_owned(),
            ..Default::default()
        };
        info.targets = vec![review.person.clone()];
        let record = crate::store::AuditRecord {
            seq: 0,
            at: unix_now(),
            connection_id: String::new(),
            connection_label: review.person.clone(),
            action: "circle".to_owned(),
            outcome: if approve {
                "approved".to_owned()
            } else {
                "denied".to_owned()
            },
            detail: circle::shown_text(&format!("{verb} for {}: {}", review.person, review.card.title), 300, false),
            grant_id: None,
            service: review.card.service.clone(),
            account: None,
            count: 1,
            info,
            op: String::new(),
        };
        if let Err(e) = self.store.append_audit(&record) {
            log::warn!("the Circle receipt could not be kept: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(arrangement: &str, account: &str, kind: CircleKind) -> Part {
        Part {
            arrangement: arrangement.to_owned(),
            account: account.to_owned(),
            label: format!("{account}@x"),
            kind,
            class: Class::Deferred,
            window_secs: 3600,
            on_expiry: OnExpiry::Deny,
            redaction: Redaction::Full,
            packet: "p".repeat(32),
            nonce: String::new(),
            sent: true,
            vote: None,
            note: None,
            lapsed: false,
            created_at: 1,
            deadline: 100,
        }
    }

    fn hold(parts: Vec<Part>) -> Hold {
        Hold {
            id: "r1".to_owned(),
            digest: String::new(),
            owner_account: "dana".to_owned(),
            parts,
            watchers: vec![],
            owner_vote: None,
            selected: vec![],
            card: ReviewCard {
                title: "Pay".to_owned(),
                subtitle: String::new(),
                headline: String::new(),
                service: String::new(),
                account: None,
                agent: String::new(),
                line: String::new(),
                detail: None,
                created_at: 1,
                deadline: 100,
            },
            deferred: true,
            deadline: 100,
            outcome: None,
            finish: Finish::Request,
        }
    }

    #[test]
    fn a_summary_names_no_subject_or_recipient() {
        let full = ReviewCard {
            title: "Claude wants to send \"Quarterly numbers\" to ann@acme.com".to_owned(),
            subtitle: "ann@acme.com".to_owned(),
            headline: "Sends Quarterly numbers to ann@acme.com".to_owned(),
            service: "gmail".to_owned(),
            account: Some("dana@gmail.com".to_owned()),
            agent: "Claude".to_owned(),
            line: "Ask · Outside · Reaches people".to_owned(),
            detail: Some("To: ann@acme.com".to_owned()),
            created_at: 1,
            deadline: 2,
        };
        let s = summary_card(&full);
        let shown = format!("{} {} {} {:?} {} {:?}", s.title, s.subtitle, s.headline, s.detail, s.line, s.account);
        assert!(!shown.contains("ann@acme.com") && !shown.contains("Quarterly"), "{shown}");
        assert_eq!(s.title, "Claude wants to act on gmail");
        assert!(s.validate().is_ok());
    }

    #[test]
    fn the_card_line_says_who_else_decides() {
        let mut h = hold(vec![part("a1", "alex", CircleKind::Cosign)]);
        assert_eq!(h.progress().line, "Needs alex@x too");
        assert!(h.progress().waiting_for_you);
        h.owner_vote = Some(true);
        assert_eq!(h.progress().line, "Waiting for alex@x");
        h.owner_vote = None;
        h.parts[0].vote = Some(Vote::Approve);
        assert_eq!(h.progress().line, "alex@x approved, waiting for you");
        let d = hold(vec![part("a2", "maria", CircleKind::Delegate)]);
        assert_eq!(d.progress().line, "maria@x can decide too");
        assert!(!d.progress().waiting_for_you);
    }
}
