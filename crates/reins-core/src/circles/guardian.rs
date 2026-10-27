//! Guardianship on the phone (Decisions v2 P7, 04 §4, 08 R7).
//!
//! Personal guardianship is **cooperative and tamper-evident**: the ward's phone holds the credentials and keeps the
//! guardian's review in force; the guardian's phone checks everything it can and says plainly what it can't.
//!
//! - **The ward** (the arrangement's owner) chose a notice at setup (0, 3 or 7 days). Ending signs an
//!   `EndRequested` that takes exactly that notice; the obligations stay until it takes effect. A minor's request
//!   waits for the guardian's consent. A guardianship is never paused by the ward. The ward turns the weekly digest
//!   on or off, accepts (or not) a minor mark the guardian proposed, and sees every decision and note.
//! - **The guardian** (the helper) proposes a minor mark, consents to a minor's end, or leaves at once. Its phone runs
//!   [`circle::check_guardian_change`] on every change of the ward against its own copy of the terms and the time the
//!   server logged it: an end before its notice, a pause, a backdated step show as "Ended early" or "Paused by Sam".
//!   What the server reports (a reset, a deletion) can't be signed by keys that are gone: it shows as "Reported by
//!   the server". A roster change is checked against the chain pinned by the safety code.

use crate::session::api_call;
use reins_proto::circle::{
    self, AgePlatform, AgeSource, CircleKind, GuardianChange, GuardianNotice, GuardianNoticeKind, GuardianStatus,
    LinkChange, LinkState, LinkView, MinorMark, Notice, Side, UNTIL_CONSENT,
};
use serde::{Deserialize, Serialize};

use super::{
    AgeSignalInput, AgeSignalPlatform, CircleEvent, GuardianDecisionView, GuardianNoticeView, Held, Me, Standing,
    WardView, not_available, verify,
};
use crate::{engine::Engine, error::CoreError, phone_api::ApiFailure, session::Session, store::unix_now};

/// How many decisions and notices a phone keeps per account.
const KEEP_ROWS: usize = 200;
/// The weekly digest goes out on the first sync after Monday 09:00 (UTC: the core keeps no time zone).
const DIGEST_HOUR_SECS: i64 = 9 * 3600;
/// The most automatic decisions a digest lists.
const DIGEST_LINES: usize = circle::MAX_NOTICE_LINES;

/// A decision a guardian made on one of this account's requests (the ward's side).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct GuardianDecision {
    pub link: String,
    pub at: i64,
    pub title: String,
    pub approved: bool,
    pub note: Option<String>,
}

/// What a guardian's phone learned about a ward.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct GuardianRow {
    pub id: String,
    pub link: String,
    pub ward: String,
    pub line: String,
    pub at: i64,
    pub verified: bool,
    /// The change it is about, when one (a server notice of the same change isn't shown twice).
    #[serde(default)]
    pub seq: Option<u64>,
}

pub(crate) fn trim_decisions(rows: &mut Vec<GuardianDecision>) {
    if rows.len() > KEEP_ROWS {
        let drop = rows.len() - KEEP_ROWS;
        rows.drain(..drop);
    }
}

fn trim_rows(rows: &mut Vec<GuardianRow>) {
    if rows.len() > KEEP_ROWS {
        let drop = rows.len() - KEEP_ROWS;
        rows.drain(..drop);
    }
}

/// "sam" of "sam@example.com": how a line names the ward.
pub(crate) fn short_name(label: &str) -> String {
    let name = label.split('@').next().unwrap_or(label).trim();
    let mut chars = name.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => label.to_owned(),
    }
}

/// The line a guardian's phone shows for a ward's change it refused (the check's verdict, in words).
pub(crate) fn refused_line(c: &LinkChange, ward: &str) -> String {
    match (&c.guardian, c.state) {
        (None, LinkState::Paused) => format!("Paused by {}", short_name(ward)),
        _ => "Ended early".to_owned(),
    }
}

/// The line for a ward's change that keeps the terms.
pub(crate) fn kept_line(c: &LinkChange, status: &GuardianStatus) -> Option<String> {
    match &c.guardian {
        Some(GuardianChange::EndRequested {
            ..
        }) if status.needs_consent() => Some("Asked to leave".to_owned()),
        Some(GuardianChange::EndRequested {
            ..
        }) => Some("Leaving after the notice".to_owned()),
        Some(GuardianChange::MinorAccepted {
            ..
        }) => Some("Accepted: a minor's account".to_owned()),
        Some(GuardianChange::Digest {
            on: true,
        }) => Some("Weekly summary on".to_owned()),
        Some(GuardianChange::Digest {
            on: false,
        }) => Some("Weekly summary off".to_owned()),
        _ => None,
    }
}

/// The minor mark a ward's acceptance records, from the phone's age signal. An adult's signal refuses it.
pub(crate) fn mark_of(signal: AgeSignalInput, now: i64) -> Result<MinorMark, CoreError> {
    let source = match signal {
        AgeSignalInput::Adult => return Err(CoreError::invalid("This account isn't a minor's.")),
        AgeSignalInput::Minor {
            upper,
            platform,
        } => AgeSource::Platform {
            kind: match platform {
                AgeSignalPlatform::Apple => AgePlatform::AppleDeclaredAgeRange,
                AgeSignalPlatform::GooglePlay => AgePlatform::GooglePlayAgeSignals,
            },
            upper: upper.filter(|u| *u < 18),
        },
        AgeSignalInput::Unavailable => AgeSource::GuardianAndWard,
    };
    Ok(MinorMark {
        source,
        at: now,
    })
}

/// The Monday 09:00 (UTC) at or before `now`.
pub(crate) fn digest_boundary(now: i64) -> i64 {
    let day = now.div_euclid(86_400);
    // 1970-01-01 was a Thursday: Monday is day 4 of that week.
    let monday = day - (day - 4).rem_euclid(7);
    let at = monday * 86_400 + DIGEST_HOUR_SECS;
    if at > now {
        at - 7 * 86_400
    } else {
        at
    }
}

fn weekday(at: i64) -> &'static str {
    ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"][usize::try_from(at.div_euclid(86_400).rem_euclid(7)).unwrap_or(0)]
}

/// One verified step applied to `held` (its status, its last change, its standing).
fn step(held: &mut Held, c: &LinkChange, logged_at: Option<i64>) -> Result<(), circle::CircleError> {
    let terms = held.arrangement.terms.guardian_terms().ok_or(circle::CircleError::Mismatch("kind"))?;
    let mut status = held.guardian_status().unwrap_or_else(|| GuardianStatus::of(&terms));
    held.last_seq = held.last_seq.max(c.seq);
    let applied = status.apply(&terms, c, logged_at);
    if applied.is_ok() {
        if status.ended {
            held.standing = Standing::Ended;
        }
        held.guardian = Some(status);
    }
    applied
}

/// One verified change of a guardianship applied to `held` (a refused one changes nothing), and what the guardian's
/// phone shows for it: the ward's steps only, a refused one as "Ended early" or "Paused by Sam".
fn judge(held: &mut Held, c: &LinkChange, logged_at: Option<i64>, link: &str, ward: &str) -> Option<GuardianRow> {
    let applied = step(held, c, logged_at);
    if held.side != Side::Helper || c.by != Side::Owner {
        return None;
    }
    let shown = match applied {
        Ok(()) => kept_line(c, held.guardian.as_ref()?)?,
        Err(e) => {
            log::warn!("a guardianship change breaks its terms: {e}");
            refused_line(c, ward)
        }
    };
    Some(GuardianRow {
        id: format!("{link}:{}", c.seq),
        link: link.to_owned(),
        ward: ward.to_owned(),
        line: shown,
        at: logged_at.unwrap_or(c.at),
        verified: true,
        seq: Some(c.seq),
    })
}

impl Engine {
    // ---- signing ----------------------------------------------------------------------------------------------------

    /// Signs and sends one change of `link` (a guardianship step, or a plain state); returns it once the server took
    /// it.
    async fn guardian_send(
        &self,
        session: &Session,
        me: &Me,
        held: &Held,
        state: LinkState,
        guardian: Option<GuardianChange>,
    ) -> Result<LinkChange, CoreError> {
        let change = LinkChange {
            v: circle::version(),
            link: held.id().to_owned(),
            arrangement: circle::arrangement_hash(&held.signed.body),
            seq: held.last_seq + 1,
            state,
            by: held.side,
            at: unix_now(),
            guardian,
        };
        change.validate_for(CircleKind::Guardian).map_err(|e| CoreError::invalid(e.to_string()))?;
        let body = serde_json::to_string(&change).map_err(|e| CoreError::storage(e.to_string()))?;
        let sig = self.circle_owner_key(&me.slot)?.sign_b64(&circle::change_message(&body));
        let signed = circle::SignedChange {
            body,
            sig,
        };
        api_call!(session, |api| api.circle_change(held.id(), &signed)).map_err(|e| match e {
            ApiFailure::Status {
                status: 409,
                ..
            } => CoreError::invalid("This guardianship changed meanwhile. Try again."),
            other => not_available(other),
        })?;
        Ok(change)
    }

    /// A guardianship this account holds on `side`, refreshed from the server first (its latest steps).
    async fn guardian_held(&self, session: &Session, me: &Me, link: &str, side: Side) -> Result<Held, CoreError> {
        self.circle_links_sync(session, me).await?;
        let held = self.circle_state(&me.slot)?.held(link).cloned().ok_or(CoreError::NotFound)?;
        if held.arrangement.terms.kind != CircleKind::Guardian {
            return Err(CoreError::invalid("This isn't a guardianship."));
        }
        if held.side != side {
            return Err(CoreError::invalid(match side {
                Side::Owner => "Only the person this guardianship is for can do that.",
                Side::Helper => "Only the guardian can do that.",
            }));
        }
        if !matches!(held.standing, Standing::Active) {
            return Err(CoreError::invalid("This guardianship isn't active."));
        }
        Ok(held)
    }

    /// Signs `guardian` as this side and applies it here as the server will log it.
    async fn guardian_do(
        &self,
        link: &str,
        side: Side,
        state: LinkState,
        guardian: impl FnOnce(&Held, i64) -> Result<GuardianChange, CoreError>,
    ) -> Result<(), CoreError> {
        let session = self.session()?;
        let me = self.circle_me(&session).await?;
        let held = self.guardian_held(&session, &me, link, side).await?;
        let change = guardian(&held, unix_now())?;
        let sent = self.guardian_send(&session, &me, &held, state, Some(change)).await?;
        let ended = self.with_circles(&me.slot, |s| {
            let h = s.held_mut(link).ok_or(CoreError::NotFound)?;
            // Our own change: it arrived when we made it.
            step(h, &sent, Some(sent.at)).map_err(|e| CoreError::invalid(e.to_string()))?;
            Ok(!h.in_force(unix_now()) || h.standing == Standing::Ended)
        })?;
        if ended {
            Box::pin(self.circle_release(&session, &me, link)).await;
        }
        self.notifier.circle_changed(CircleEvent::Changed);
        Ok(())
    }

    // ---- the ward ---------------------------------------------------------------------------------------------------

    /// The ward ends it: after the notice it chose, or (a minor) once the guardian consents. The guardian's review
    /// stays until then.
    pub(crate) async fn guardian_request_end(&self, link: &str) -> Result<(), CoreError> {
        self.guardian_do(link, Side::Owner, LinkState::Active, |held, now| {
            let status = held.guardian_status().ok_or(CoreError::NotFound)?;
            if status.ending.is_some() {
                return Err(CoreError::invalid("You asked to leave already."));
            }
            let terms = held.arrangement.terms.guardian_terms().ok_or(CoreError::NotFound)?;
            Ok(GuardianChange::EndRequested {
                effective_at: circle::guardian_effective(&terms, status.minor.is_some(), now).unwrap_or(UNTIL_CONSENT),
            })
        })
        .await
    }

    /// The ward accepts the minor mark the guardian proposed, with the phone's age signal. An adult's signal refuses.
    pub(crate) async fn guardian_accept_minor(&self, link: &str, signal: AgeSignalInput) -> Result<(), CoreError> {
        let mark = mark_of(signal, unix_now())?;
        self.guardian_do(link, Side::Owner, LinkState::Active, move |held, _| {
            let status = held.guardian_status().ok_or(CoreError::NotFound)?;
            if !status.minor_proposed {
                return Err(CoreError::invalid("Your guardian hasn't asked for this."));
            }
            Ok(GuardianChange::MinorAccepted {
                mark,
            })
        })
        .await
    }

    /// The ward turns the weekly summary of automatic decisions in scope on or off (04 §4: opt-in by the ward).
    pub(crate) async fn guardian_set_digest(&self, link: &str, on: bool) -> Result<(), CoreError> {
        self.guardian_do(link, Side::Owner, LinkState::Active, move |_, _| {
            Ok(GuardianChange::Digest {
                on,
            })
        })
        .await?;
        // The first summary covers the week after opting in, never what came before.
        if on && let Some(slot) = self.circle_slot_now() {
            let id = link.to_owned();
            self.with_circles(&slot, |s| {
                s.digests.retain(|(l, _)| *l != id);
                s.digests.push((id.clone(), unix_now()));
                Ok(())
            })?;
        }
        Ok(())
    }

    /// One guardianship as the ward sees it: the scope, every decision with its note, how it ends.
    pub(crate) fn guardian_ward_view(&self, link: &str) -> Result<WardView, CoreError> {
        let slot = self.circle_slot_now().ok_or(CoreError::NotFound)?;
        let state = self.circle_state(&slot)?;
        let held = state.held(link).ok_or(CoreError::NotFound)?;
        let terms = held.arrangement.terms.guardian_terms().ok_or_else(|| CoreError::invalid("not a guardianship"))?;
        if held.side != Side::Owner {
            return Err(CoreError::invalid("not your guardianship"));
        }
        let status = held.guardian_status().unwrap_or_else(|| GuardianStatus::of(&terms));
        let mut decisions: Vec<GuardianDecisionView> = state
            .guardian_decisions
            .iter()
            .filter(|d| d.link == link)
            .map(|d| GuardianDecisionView {
                at: d.at,
                title: d.title.clone(),
                approved: d.approved,
                note: d.note.clone(),
            })
            .collect();
        decisions.sort_by_key(|d| std::cmp::Reverse(d.at));
        Ok(WardView {
            link_id: link.to_owned(),
            guardian: held.other().label.clone(),
            scope_line: held.arrangement.terms.scope_line.clone(),
            notice_days: terms.notice_days,
            minor: status.minor.is_some(),
            minor_proposed: status.minor_proposed,
            decisions,
            ending: status.ending.map(|e| e.effective_at).filter(|at| *at != UNTIL_CONSENT),
            needs_consent: status.needs_consent(),
            digest: status.digest,
        })
    }

    // ---- the guardian -----------------------------------------------------------------------------------------------

    /// The guardian asks to mark the ward's account a minor's; nothing changes until the ward accepts.
    pub(crate) async fn guardian_propose_minor(&self, link: &str) -> Result<(), CoreError> {
        self.guardian_do(link, Side::Helper, LinkState::Active, |held, _| {
            if held.guardian_status().is_some_and(|g| g.minor.is_some()) {
                return Err(CoreError::invalid("This account is marked a minor's already."));
            }
            Ok(GuardianChange::MinorProposed)
        })
        .await
    }

    /// The guardian consents to a minor's end request: it ends now.
    pub(crate) async fn guardian_consent_end(&self, link: &str) -> Result<(), CoreError> {
        self.guardian_do(link, Side::Helper, LinkState::Ended, |held, _| {
            let status = held.guardian_status().ok_or(CoreError::NotFound)?;
            let ending = status
                .ending
                .filter(|_| status.needs_consent())
                .ok_or_else(|| CoreError::invalid("Nothing waits for your consent."))?;
            Ok(GuardianChange::EndConsented {
                request_at: ending.requested_at,
            })
        })
        .await
    }

    /// What this account was told as a guardian, newest first.
    pub(crate) fn guardian_notices(&self) -> Result<Vec<GuardianNoticeView>, CoreError> {
        let Some(slot) = self.circle_slot_now() else {
            return Ok(Vec::new());
        };
        let state = self.circle_state(&slot)?;
        let mut out: Vec<GuardianNoticeView> = state.guardian_notices.iter().map(|r| row_view(&state, r)).collect();
        out.sort_by_key(|n| std::cmp::Reverse(n.at));
        Ok(out)
    }

    fn guardian_tell(&self, slot: &str, row: &GuardianRow) -> Result<(), CoreError> {
        let view = self.with_circles(slot, |s| {
            if s.guardian_notices.iter().any(|r| r.id == row.id) {
                return Ok(None);
            }
            s.guardian_notices.push(row.clone());
            trim_rows(&mut s.guardian_notices);
            Ok(Some(row_view(s, row)))
        })?;
        if let Some(notice) = view {
            self.notifier.circle_changed(CircleEvent::GuardianNotice {
                notice,
            });
        }
        Ok(())
    }

    // ---- sync -------------------------------------------------------------------------------------------------------

    /// A guardianship's logged changes, each checked in order against the terms this phone holds and the time the
    /// server logged it. A change of the other side that breaks them changes nothing here; the guardian's phone shows
    /// it ("Ended early"). Returns whether anything changed.
    pub(crate) fn circle_guardian_sync(&self, me: &Me, link: &LinkView) -> Result<bool, CoreError> {
        let Some(held) = self.circle_state(&me.slot)?.held(&link.id).cloned() else {
            return Ok(false);
        };
        if held.standing == Standing::Pending || held.arrangement.terms.kind != CircleKind::Guardian {
            return Ok(false);
        }
        let hash = circle::arrangement_hash(&held.signed.body);
        // The history, or (an older server) the last change, without its arrival time: then nothing is accepted.
        let log: Vec<(circle::SignedChange, Option<i64>)> = if link.changes.is_empty() {
            link.change.iter().map(|c| (c.clone(), None)).collect()
        } else {
            link.changes.iter().map(|l| (l.change.clone(), Some(l.at))).collect()
        };
        let mut changed = false;
        let mut rows = Vec::new();
        let ward = held.arrangement.owner.label.clone();
        let mut current = held.clone();
        for (signed, logged_at) in log {
            let Ok(c) = signed.change() else {
                continue;
            };
            if c.seq <= current.last_seq || current.standing == Standing::Ended {
                continue;
            }
            let key = match c.by {
                Side::Owner => &held.arrangement.owner.owner_pub,
                Side::Helper => &held.arrangement.helper.owner_pub,
            };
            let c = match signed.verify(key, &hash, CircleKind::Guardian, current.last_seq, verify) {
                Ok(c) => c,
                Err(e) => {
                    log::warn!("a guardianship change was refused: {e}");
                    continue;
                }
            };
            changed = true;
            if let Some(row) = judge(&mut current, &c, logged_at, &link.id, &ward) {
                rows.push(row);
            }
        }
        // A notice that ran out ends it here too.
        if current.standing == Standing::Active && !current.in_force(unix_now()) {
            current.standing = Standing::Ended;
            changed = true;
        }
        if changed {
            let ended = current.standing == Standing::Ended && held.standing != Standing::Ended;
            self.with_circles(&me.slot, |s| {
                if let Some(h) = s.held_mut(&link.id) {
                    h.last_seq = current.last_seq;
                    h.guardian.clone_from(&current.guardian);
                    h.standing = current.standing;
                }
                Ok(())
            })?;
            for row in rows {
                self.guardian_tell(&me.slot, &row)?;
            }
            if ended && held.side == Side::Owner {
                log::info!("a guardianship ended after its notice");
            }
        }
        Ok(changed)
    }

    /// A guardianship that vanished from the server without an end (only the server could drop it): the guardian is
    /// told, as reported by the server. Returns whether anything changed.
    pub(crate) fn circle_guardian_vanished(&self, me: &Me, links: &[LinkView]) -> Result<bool, CoreError> {
        let state = self.circle_state(&me.slot)?;
        let gone: Vec<Held> = state
            .held
            .iter()
            .filter(|h| {
                h.arrangement.terms.kind == CircleKind::Guardian
                    && h.side == Side::Helper
                    && matches!(h.standing, Standing::Active | Standing::Paused)
                    && !links.iter().any(|l| l.id == h.id())
            })
            .cloned()
            .collect();
        for h in &gone {
            self.with_circles(&me.slot, |s| {
                if let Some(h) = s.held_mut(h.id()) {
                    h.standing = Standing::Suspended;
                }
                Ok(())
            })?;
            self.guardian_tell(
                &me.slot,
                &GuardianRow {
                    id: format!("{}:gone", h.id()),
                    link: h.id().to_owned(),
                    ward: h.arrangement.owner.label.clone(),
                    line: "Guardianship gone".to_owned(),
                    at: unix_now(),
                    verified: false,
                    seq: None,
                },
            )?;
        }
        Ok(!gone.is_empty())
    }

    /// The server's notices for this account as a guardian, each checked where a phone can.
    pub(crate) async fn guardian_take_notices(&self, session: &Session, me: &Me) -> Result<(), CoreError> {
        let notices: Vec<GuardianNotice> =
            api_call!(session, |api| api.guardian_notices()).map_err(ApiFailure::into_core)?;
        for notice in notices {
            let Some(held) = self.circle_state(&me.slot)?.held(&notice.link).cloned() else {
                continue;
            };
            if held.side != Side::Helper || held.arrangement.terms.kind != CircleKind::Guardian {
                continue;
            }
            let ward = held.arrangement.owner.label.clone();
            let row = |line: &str, verified: bool| GuardianRow {
                id: notice.id.clone(),
                link: notice.link.clone(),
                ward: ward.clone(),
                line: line.to_owned(),
                at: notice.at,
                verified,
                seq: None,
            };
            match &notice.kind {
                GuardianNoticeKind::LinkChanged {
                    change,
                } => {
                    let hash = circle::arrangement_hash(&held.signed.body);
                    match change.verify(&held.arrangement.owner.owner_pub, &hash, CircleKind::Guardian, 0, verify) {
                        // The links' history carries it: its check made the row already.
                        Ok(c) if c.seq <= held.last_seq => {}
                        Ok(c) => {
                            let terms =
                                held.arrangement.terms.guardian_terms().unwrap_or(circle::GuardianTerms::LEGACY);
                            let minor = held.guardian_status().is_some_and(|g| g.minor.is_some());
                            let line = match circle::check_guardian_change(&terms, minor, &c, Some(notice.at)) {
                                Ok(()) => kept_line(&c, &held.guardian_status().unwrap_or_default())
                                    .unwrap_or_else(|| "Changed".to_owned()),
                                Err(_) => refused_line(&c, &ward),
                            };
                            self.guardian_tell(&me.slot, &row(&line, true))?;
                        }
                        Err(_) => self.guardian_tell(&me.slot, &row("Changed", false))?,
                    }
                }
                GuardianNoticeKind::RosterChanged {
                    reason,
                    ..
                } => {
                    let line = match self.circle_peer(session, &me.slot, &notice.link).await {
                        Ok(_) => Some(match reason {
                            reins_proto::roster::RosterReason::Remove => "Removed a phone",
                            _ => "New phone",
                        }),
                        // Its owner key changed: P6 suspended the guardianship; the chain proved it.
                        Err(_)
                            if self
                                .circle_state(&me.slot)?
                                .held(&notice.link)
                                .is_some_and(|h| h.standing == Standing::Suspended) =>
                        {
                            Some("Account recovered")
                        }
                        Err(_) => None,
                    };
                    match line {
                        Some(line) => self.guardian_tell(&me.slot, &row(line, true))?,
                        None => self.guardian_tell(&me.slot, &row("Phones changed", false))?,
                    }
                }
                GuardianNoticeKind::AccountReset => self.guardian_tell(&me.slot, &row("Reset the account", false))?,
                GuardianNoticeKind::AccountDeleted => {
                    self.guardian_tell(&me.slot, &row("Deleted the account", false))?;
                }
            }
        }
        Ok(())
    }

    /// The ward's weekly digest (opt-in): one notice per guardianship, on the first sync after Monday 09:00, listing
    /// the automatic decisions in its scope. No bodies, recipients or titles: the day and the matrix line.
    pub(crate) async fn guardian_digest(&self, session: &Session, me: &Me) {
        let now = unix_now();
        let boundary = digest_boundary(now);
        let Ok(state) = self.circle_state(&me.slot) else {
            return;
        };
        for held in state
            .held
            .iter()
            .filter(|h| h.side == Side::Owner && h.in_force(now) && h.guardian_status().is_some_and(|g| g.digest))
        {
            let last = state.digests.iter().find(|(l, _)| l == held.id()).map_or(0, |(_, at)| *at);
            if last >= boundary {
                continue;
            }
            let since = boundary - 7 * 86_400;
            let lines = match self.guardian_digest_lines(held, since, boundary) {
                Ok(lines) => lines,
                Err(e) => {
                    log::info!("the weekly summary could not be read: {e}");
                    continue;
                }
            };
            let count = lines.len();
            let notice = Notice {
                text: format!(
                    "This week: {count} automatic decision{}",
                    if count == 1 {
                        ""
                    } else {
                        "s"
                    }
                ),
                outcome: None,
                at: now,
                lines: lines.into_iter().take(DIGEST_LINES).collect(),
            };
            match self.circle_notice(session, me, held.id(), notice).await {
                Ok(()) => {
                    let id = held.id().to_owned();
                    if let Err(e) = self.with_circles(&me.slot, |s| {
                        s.digests.retain(|(l, _)| *l != id);
                        s.digests.push((id.clone(), now));
                        Ok(())
                    }) {
                        log::warn!("Circle state: {e}");
                    }
                }
                Err(e) => log::info!("the weekly summary could not be sent: {e}"),
            }
        }
    }

    /// The automatic decisions in `held`'s scope logged in `[since, until)`, oldest first.
    fn guardian_digest_lines(&self, held: &Held, since: i64, until: i64) -> Result<Vec<String>, CoreError> {
        let scope = &held.arrangement.terms.scope;
        let mut lines = Vec::new();
        for record in self.store.activity(2_000)?.into_iter().rev() {
            if record.at < since || record.at >= until {
                continue;
            }
            let Some(matrix) = record.info.autopilot.as_ref().and_then(|a| a.matrix.as_ref()) else {
                continue;
            };
            if matrix.shadow || matrix.outcome != reins_policy::matrix::Outcome::Auto {
                continue;
            }
            let input = reins_policy::circles::Input {
                kinds: &matrix.kinds,
                zone: matrix.zone,
                level: matrix.level,
                certainty: reins_policy::effects::Certainty::Full,
                connection: &record.connection_id,
                account: record.account.as_deref(),
            };
            if !reins_policy::circles::scope_matches(CircleKind::Guardian, scope, &input) {
                continue;
            }
            lines.push(circle::shown_text(
                &format!("{} · {}", weekday(record.at), matrix.line),
                circle::MAX_CARD_LINE,
                false,
            ));
        }
        Ok(lines)
    }
}

fn row_view(state: &super::CircleState, r: &GuardianRow) -> GuardianNoticeView {
    let needs_consent = state.held(&r.link).is_some_and(|h| {
        h.side == Side::Helper
            && h.standing == Standing::Active
            && h.guardian_status().is_some_and(|g| g.needs_consent())
    }) && r.line == "Asked to leave";
    GuardianNoticeView {
        id: r.id.clone(),
        link_id: r.link.clone(),
        ward: r.ward.clone(),
        line: r.line.clone(),
        at: r.at,
        verified: r.verified,
        needs_consent,
    }
}

#[cfg(test)]
mod tests {
    use reins_proto::circle::{Arrangement, Party, SignedArrangement, Terms};

    use super::*;

    const DAY: i64 = 86_400;

    fn party(account: &str) -> Party {
        Party {
            account: account.to_owned(),
            label: format!("{account}@example.com"),
            owner_pub: String::new(),
            epoch: 1,
            entry_hash: String::new(),
        }
    }

    fn held(side: Side, days: u8) -> Held {
        let mut terms = Terms::default_for(CircleKind::Guardian);
        terms.guardian = Some(circle::GuardianTerms {
            notice_days: days,
            minor: None,
            digest: false,
        });
        Held {
            signed: SignedArrangement {
                body: String::new(),
                helper_sig: String::new(),
                owner_sig: String::new(),
            },
            arrangement: Arrangement {
                v: 1,
                id: "0123456789abcdef0123456789abcdef".to_owned(),
                revision: 1,
                owner: party("sam"),
                helper: party("mom"),
                terms,
                created_at: 1,
            },
            side,
            standing: Standing::Active,
            anchor_epoch: 1,
            anchor_hash: String::new(),
            seen_epoch: 1,
            peer: None,
            last_seq: 0,
            since: 1,
            guardian: None,
        }
    }

    fn at(by: Side, state: LinkState, seq: u64, t: i64, guardian: Option<GuardianChange>) -> LinkChange {
        LinkChange {
            seq,
            at: t,
            ..change(by, state, guardian)
        }
    }

    fn end(seq: u64, t: i64, effective_at: i64) -> LinkChange {
        at(
            Side::Owner,
            LinkState::Active,
            seq,
            t,
            Some(GuardianChange::EndRequested {
                effective_at,
            }),
        )
    }

    fn line(h: &mut Held, c: &LinkChange, logged_at: i64) -> Option<String> {
        judge(h, c, Some(logged_at), "l", "sam@example.com").map(|r| r.line)
    }

    #[test]
    fn obligations_stay_during_the_notice() {
        let t = 1_000_000;
        let mut ward = held(Side::Owner, 3);
        assert_eq!(judge(&mut ward, &end(1, t, t + 3 * DAY), Some(t), "l", "sam"), None, "the ward is told nothing");
        assert!(ward.in_force(t + 3 * DAY - 1), "the guardian's review stays until the notice ends");
        assert!(!ward.in_force(t + 3 * DAY));
        assert_eq!(ward.standing, Standing::Active, "nothing is ended early");
    }

    #[test]
    fn a_minors_end_changes_nothing_until_consent() {
        let t = 1_000_000;
        let mut ward = held(Side::Owner, 7);
        ward.guardian = Some(GuardianStatus {
            minor: Some(mark_of(AgeSignalInput::Unavailable, 1).unwrap()),
            ..GuardianStatus::default()
        });
        judge(&mut ward, &end(1, t, UNTIL_CONSENT), Some(t), "l", "sam");
        assert!(ward.in_force(t + 365 * DAY));
        assert!(ward.guardian_status().unwrap().needs_consent());
        let consent = at(
            Side::Helper,
            LinkState::Ended,
            2,
            t + 10,
            Some(GuardianChange::EndConsented {
                request_at: t,
            }),
        );
        judge(&mut ward, &consent, Some(t + 10), "l", "sam");
        assert_eq!(ward.standing, Standing::Ended);
        assert!(!ward.in_force(t + 11));
    }

    #[test]
    fn an_early_end_is_flagged_to_the_guardian() {
        let t = 1_000_000;
        let mut mom = held(Side::Helper, 7);
        // A plain end, a short notice, a backdated request, a pause: each is "Ended early" or "Paused by Sam" and
        // changes nothing on the guardian's phone.
        assert_eq!(line(&mut mom, &at(Side::Owner, LinkState::Ended, 1, t, None), t).as_deref(), Some("Ended early"));
        assert_eq!(line(&mut mom, &end(2, t, t + DAY), t).as_deref(), Some("Ended early"));
        assert_eq!(line(&mut mom, &end(3, t - 7 * DAY, t), t).as_deref(), Some("Ended early"));
        assert_eq!(
            line(&mut mom, &at(Side::Owner, LinkState::Paused, 4, t, None), t).as_deref(),
            Some("Paused by Sam")
        );
        assert_eq!(mom.standing, Standing::Active);
        assert!(mom.in_force(t + 30 * DAY));
        // The honest request: told, and kept until the notice ends.
        assert_eq!(line(&mut mom, &end(5, t, t + 7 * DAY), t).as_deref(), Some("Leaving after the notice"));
        assert!(mom.in_force(t + 7 * DAY - 1));
        assert!(!mom.in_force(t + 7 * DAY));
        // The guardian's own steps make no row for itself.
        let mut mom = held(Side::Helper, 7);
        assert_eq!(
            line(&mut mom, &at(Side::Helper, LinkState::Active, 1, t, Some(GuardianChange::MinorProposed)), t),
            None
        );
    }

    #[test]
    fn digest_is_off_until_the_ward_opts_in_and_stays_in_scope() {
        let t = 1_000_000;
        let mut ward = held(Side::Owner, 3);
        assert!(!ward.guardian_status().unwrap().digest, "off at setup");
        // The guardian can't turn it on.
        let by_guardian = at(
            Side::Helper,
            LinkState::Active,
            1,
            t,
            Some(GuardianChange::Digest {
                on: true,
            }),
        );
        assert!(by_guardian.validate_for(CircleKind::Guardian).is_err());
        judge(
            &mut ward,
            &at(
                Side::Owner,
                LinkState::Active,
                1,
                t,
                Some(GuardianChange::Digest {
                    on: true,
                }),
            ),
            Some(t),
            "l",
            "sam",
        );
        assert!(ward.guardian_status().unwrap().digest);
        // Only decisions in scope are listed: a Money guardianship hears nothing of a message.
        let money = circle::Scope {
            effects: vec!["money".to_owned()],
            ..circle::Scope::default()
        };
        let kinds = [reins_policy::effects::EffectKind::Message];
        let input = reins_policy::circles::Input {
            kinds: &kinds,
            zone: reins_policy::context::Zone::Outside,
            level: reins_policy::effects::Level::L1,
            certainty: reins_policy::effects::Certainty::Full,
            connection: "c",
            account: None,
        };
        assert!(!reins_policy::circles::scope_matches(CircleKind::Guardian, &money, &input));
    }

    fn change(by: Side, state: LinkState, guardian: Option<GuardianChange>) -> LinkChange {
        LinkChange {
            v: 1,
            link: "0123456789abcdef0123456789abcdef".to_owned(),
            arrangement: "0".repeat(64),
            seq: 1,
            state,
            by,
            at: 1,
            guardian,
        }
    }

    #[test]
    fn a_guardian_cannot_mark_an_adult_minor_alone() {
        // The ward's phone signs the acceptance, and an adult's age signal refuses it.
        assert!(mark_of(AgeSignalInput::Adult, 1).is_err());
        let m = mark_of(
            AgeSignalInput::Minor {
                upper: Some(15),
                platform: AgeSignalPlatform::Apple,
            },
            1,
        )
        .unwrap();
        assert_eq!(
            m.source,
            AgeSource::Platform {
                kind: AgePlatform::AppleDeclaredAgeRange,
                upper: Some(15)
            }
        );
        assert_eq!(mark_of(AgeSignalInput::Unavailable, 1).unwrap().source, AgeSource::GuardianAndWard);
        // A guardian's proposal alone marks nothing.
        let t = circle::GuardianTerms::default();
        let mut s = GuardianStatus::of(&t);
        s.apply(&t, &change(Side::Helper, LinkState::Active, Some(GuardianChange::MinorProposed)), Some(1)).unwrap();
        assert!(s.minor.is_none());
        assert!(s.minor_proposed);
    }

    #[test]
    fn lines_name_what_happened() {
        let paused = change(Side::Owner, LinkState::Paused, None);
        assert_eq!(refused_line(&paused, "sam@example.com"), "Paused by Sam");
        assert_eq!(refused_line(&change(Side::Owner, LinkState::Ended, None), "sam@example.com"), "Ended early");
        assert_eq!(short_name("sam@example.com"), "Sam");
    }

    #[test]
    fn the_digest_goes_after_monday_nine() {
        // Monday 2026-10-26 09:00 UTC = 1_793_005_200.
        let monday_nine = 1_793_005_200;
        assert_eq!(weekday(monday_nine), "Mon");
        assert_eq!(digest_boundary(monday_nine), monday_nine);
        assert_eq!(digest_boundary(monday_nine + 3 * 86_400), monday_nine);
        assert_eq!(digest_boundary(monday_nine - 1), monday_nine - 7 * 86_400);
    }
}
