//! What the paired desktop app may change of Autopilot (`desktop_autopilot_mode`, `desktop_autopilot_correct`), and
//! the one rule that governs it: the computer never makes Reins more permissive on its own authority.
//!
//! The phone reads every such call itself and decides which way it goes, whatever the computer says:
//! - **Stricter, applied at once** (like ending a work session): Lockdown; a global mode no looser than the one the
//!   user picked and than the one an Overnight returns to; stopping the bypass or Overnight that runs; teaching
//!   Autopilot that a request should have been denied.
//! - **Looser, asked**: Auto from Manual (or over an Overnight that returns to Manual), leaving Lockdown, teaching
//!   Autopilot that a request should have been approved. These park only when the call carries the app's own
//!   [`DesktopAsk`] (boxed from the key the phone pinned for this connection: the server cannot make one up, change it
//!   or send it twice), worded by the phone from its own state when they are viewed, held to approve, asked every time
//!   (never remembered, never one tap), and are never decided automatically: not by Autopilot, not by Overnight, not
//!   by a bypass and not by Lockdown (which would deny the way out of it). Only the owner's own approval applies them.
//!   A lockdown that starts denies those already waiting.
//!
//! A bypass and Overnight are never started from the computer: they start on the phone, after its owner check.
//! A computer in its own Lockdown changes nothing of Autopilot. A correction only ever concerns a decision Autopilot
//! made for a request of the connection that sends it, and the phone names that request from its own activity.

use reins_proto::autopilot::{ASK_DIR, ASK_TTL_SECS, DesktopAsk};
use reins_proto::connector::{AUTOPILOT_CORRECT_OP, AUTOPILOT_MODE_OP, ConnectorCall, DESKTOP};

use super::modes::{self, ModeRow};
use super::types::{AutopilotMode, Verdict};
use crate::CoreError;
use crate::engine::Engine;
use crate::store::AuditRecord;

/// Where the nonces of the asks the phone accepted are kept, until they would be too old anyway.
const SEEN_ASKS: &str = "autopilot.desktop.seen";
/// How far ahead of the phone's clock an ask may be dated.
const CLOCK_SKEW_SECS: i64 = 5 * 60;

/// What a computer asks of the global mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModeChange {
    /// Manual, Auto or Lockdown.
    Set(AutopilotMode),
    /// End the bypass or Overnight that runs.
    Stop,
}

/// A call only the owner's own approval may apply (never an automatic decision).
#[must_use]
pub fn owner_only(service: &str, op: &str) -> bool {
    service == DESKTOP && (op == AUTOPILOT_MODE_OP || op == AUTOPILOT_CORRECT_OP)
}

/// What no automatic mode decides, Bypass included: a change of Autopilot from a computer ([`owner_only`]), and a task
/// an agent proposes (it loosens policy: 02 §1, an agent proposes and only a principal decides).
#[must_use]
pub fn owners_alone(service: &str, op: &str) -> bool {
    owner_only(service, op) || (service == reins_proto::connector::TASKS && reins_proto::connector::tasks::proposes(op))
}

/// The change a `desktop_autopilot_mode` call asks for, read from the call (which the phone checked against its
/// tool description).
pub fn mode_change(call: &ConnectorCall) -> Result<ModeChange, CoreError> {
    Ok(match call.str_arg("mode").unwrap_or_default() {
        "manual" => ModeChange::Set(AutopilotMode::Manual),
        "auto" => ModeChange::Set(AutopilotMode::Auto),
        "lockdown" => ModeChange::Set(AutopilotMode::Lockdown),
        "stop" => ModeChange::Stop,
        _ => return Err(CoreError::service("Unknown mode (a bypass and Overnight start on the phone).")),
    })
}

/// The request a `desktop_autopilot_correct` call is about, and what it should have been.
pub fn correction(call: &ConnectorCall) -> Result<(String, Verdict), CoreError> {
    let id = call.str_arg("request_id").unwrap_or_default().trim().to_owned();
    if id.is_empty() || id.len() > 100 || !id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_')) {
        return Err(CoreError::service("`request_id` is not a request id."));
    }
    let verdict = match call.str_arg("should_have").unwrap_or_default() {
        "approve" => Verdict::Approve,
        "deny" => Verdict::Deny,
        _ => return Err(CoreError::service("`should_have` is approve or deny.")),
    };
    Ok((id, verdict))
}

fn openness(mode: AutopilotMode) -> u8 {
    mode.wire().openness()
}

/// The stricter of two modes.
fn stricter_of(a: AutopilotMode, b: AutopilotMode) -> AutopilotMode {
    if openness(a) <= openness(b) {
        a
    } else {
        b
    }
}

/// What the global mode comes to without the computer: the mode the user picked, and, while an Overnight runs, the
/// mode it returns to (Manual unless the user said otherwise), whichever is stricter. A bypass is not counted: it
/// ends by itself.
fn settles_to(global: &ModeRow, now: i64) -> AutopilotMode {
    let base = modes::global_base(global, now);
    if global.overnighting(now) {
        let after = global.overnight_return.filter(|m| *m != AutopilotMode::Overnight).unwrap_or(AutopilotMode::Manual);
        stricter_of(base, after)
    } else {
        base
    }
}

/// Whether `change` never lets more through than `global` does now or will by itself: then the phone applies it at
/// once.
#[must_use]
pub fn mode_is_stricter(change: ModeChange, global: &ModeRow, now: i64) -> bool {
    match change {
        ModeChange::Set(AutopilotMode::Lockdown) | ModeChange::Stop => true,
        // Setting a mode ends any bypass or Overnight: what remains is the mode set, which must be no looser than
        // where the phone would settle by itself (a global Lockdown is the strictest: leaving it is asked).
        ModeChange::Set(mode) => openness(mode) <= openness(settles_to(global, now)),
    }
}

/// The mode a stop returns to: the one the bypass or Overnight interrupted.
fn stopped_to(global: &ModeRow, now: i64) -> Option<AutopilotMode> {
    if global.bypassing(now) {
        global.base(now)
    } else if global.overnighting(now) {
        global.overnight_return.filter(|m| *m != AutopilotMode::Overnight)
    } else {
        global.mode
    }
}

/// The phone's wording of a change, from the phone's state when it is viewed (never the computer's words).
#[must_use]
pub fn mode_question(change: ModeChange, global: &ModeRow, now: i64, computer: &str) -> (String, String) {
    let detail = format!(
        "Asked from {computer}. It changes the mode of every AI on this phone. The computer cannot make Reins more \
         permissive on its own: only your approval here can."
    );
    let locked = global.mode == Some(AutopilotMode::Lockdown);
    let night = global.overnighting(now);
    let question = match change {
        ModeChange::Set(AutopilotMode::Auto) if locked => "Turn on Auto and leave Lockdown?",
        ModeChange::Set(AutopilotMode::Auto) if night => "End Overnight and keep every AI on Auto?",
        ModeChange::Set(AutopilotMode::Auto) => "Turn on Auto for every AI?",
        ModeChange::Set(AutopilotMode::Manual) if locked => "Set every AI to Manual and leave Lockdown?",
        ModeChange::Set(AutopilotMode::Manual) => "Set every AI to Manual?",
        ModeChange::Set(_) => "Lock down every AI?",
        ModeChange::Stop => "Stop the bypass or Overnight that runs?",
    };
    (question.to_owned(), detail)
}

/// The phone's wording of a correction, naming the request from the phone's own activity entry (never from anything
/// the computer or the server sent).
#[must_use]
pub fn correction_question(verdict: Verdict, entry: &AuditRecord, computer: &str) -> (String, String) {
    let question = match verdict {
        Verdict::Approve => "Teach Autopilot to approve requests like this one?",
        _ => "Teach Autopilot to deny requests like this one?",
    }
    .to_owned();
    let label = crate::titles::of_entry(entry);
    let what = crate::text::truncate_chars(
        &crate::text::one_line(&format!("{} {}{}", label.lead, label.subject, label.tail)),
        200,
    );
    let decided = if entry.outcome == "denied" {
        "denied"
    } else {
        "allowed"
    };
    let detail =
        format!("Request: {what}\nAutopilot {decided} it for {computer}.\nAsked from {computer} (\"This was wrong\").");
    (question, detail)
}

impl Engine {
    /// Reads a desktop call that changes Autopilot as it arrives: whether it only makes Reins stricter (applied at
    /// once). Refused: from a computer in its own Lockdown; a correction of a decision Autopilot did not make for this
    /// computer; anything looser without the app's own [`DesktopAsk`] for exactly this call.
    pub(crate) fn desktop_autopilot_intake(
        &self,
        connection_id: &str,
        call: &ConnectorCall,
    ) -> Result<bool, CoreError> {
        if self.store.ap_mode_row(connection_id)?.mode == Some(AutopilotMode::Lockdown) {
            return Err(CoreError::service(
                "This computer is in Lockdown: it changes nothing of Autopilot. Change it on the phone.",
            ));
        }
        let now = crate::store::unix_now();
        let stricter = match call.op.as_str() {
            AUTOPILOT_MODE_OP => mode_is_stricter(mode_change(call)?, &self.store.ap_mode_row("")?, now),
            AUTOPILOT_CORRECT_OP => {
                let (request_id, verdict) = correction(call)?;
                self.desktop_decision_entry(connection_id, &request_id)?;
                verdict == Verdict::Deny
            }
            _ => return Err(CoreError::service("not an Autopilot change")),
        };
        if !stricter {
            self.check_desktop_ask(call, now)?;
        }
        Ok(stricter)
    }

    /// Checks the call's `proof`: a [`DesktopAsk`] the paired app boxed from its key (the call's `client_key`, which
    /// the flow already matched to the key pinned for this connection) to this phone's key, for exactly this call,
    /// recent, and never seen before.
    fn check_desktop_ask(&self, call: &ConnectorCall, now: i64) -> Result<(), CoreError> {
        let refused = |why: &str| {
            CoreError::service(format!("This change could not be verified as the computer's ({why}); refused."))
        };
        let proof = call.str_arg("proof").ok_or_else(|| refused("no proof"))?;
        let app = crate::connector::sealed::client_key(call)?;
        let nonce = crate::connector::sealed::nonce_arg(call)?;
        let plain = crate::connector::sealed::open_from(app, &*self.store.inbox_key()?, proof)
            .ok_or_else(|| refused("not boxed from its key to this phone's"))?;
        let ask: DesktopAsk = serde_json::from_slice(&plain).map_err(|_| refused("unreadable"))?;
        if ask.dir != ASK_DIR || ask.op != call.op || ask.nonce != nonce || ask.args != DesktopAsk::covered(&call.args)
        {
            return Err(refused("made for another call"));
        }
        if ask.created_at < now - ASK_TTL_SECS || ask.created_at > now + CLOCK_SKEW_SECS {
            return Err(refused("too old, or the computer's clock is wrong"));
        }
        let mut seen: std::collections::BTreeMap<String, i64> =
            self.store.meta_get(SEEN_ASKS)?.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        seen.retain(|_, until| *until >= now);
        if seen.contains_key(&nonce) {
            return Err(refused("sent before"));
        }
        seen.insert(nonce, ask.created_at + ASK_TTL_SECS);
        self.store.meta_set(
            SEEN_ASKS,
            &serde_json::to_string(&seen).map_err(|_| CoreError::storage("cannot record the ask"))?,
        )
    }

    /// The question the owner sees for a parked change of Autopilot, worded now from the phone's own state.
    pub(crate) fn desktop_autopilot_view(
        &self,
        connection_id: &str,
        call: &ConnectorCall,
        computer: &str,
    ) -> Option<crate::types::AskView> {
        let (question, detail) = if call.op == AUTOPILOT_MODE_OP {
            let global = self.store.ap_mode_row("").ok()?;
            mode_question(mode_change(call).ok()?, &global, crate::store::unix_now(), computer)
        } else {
            let (request_id, verdict) = correction(call).ok()?;
            let entry = self.desktop_decision_record(connection_id, &request_id).ok()?;
            correction_question(verdict, &entry, computer)
        };
        Some(crate::types::AskView {
            question,
            detail: Some(detail),
            topic: None,
            rule: None,
            uncertain: false,
            destructive: true,
            kind: None,
            branch: None,
            default_branch: None,
        })
    }

    /// Applies the desktop's `call` for its connection: it was found stricter on arrival, or the owner approved it.
    pub(crate) async fn apply_desktop_autopilot(
        &self,
        connection_id: &str,
        call: &ConnectorCall,
    ) -> Result<serde_json::Value, CoreError> {
        match call.op.as_str() {
            AUTOPILOT_MODE_OP => {
                match mode_change(call)? {
                    ModeChange::Set(mode) => self.set_autopilot_mode(None, Some(mode), None).await?,
                    ModeChange::Stop => {
                        let now = crate::store::unix_now();
                        let global = self.store.ap_mode_row("")?;
                        if global.bypassing(now) || global.overnighting(now) {
                            self.set_autopilot_mode(None, stopped_to(&global, now), None).await?;
                        }
                    }
                }
                Ok(serde_json::json!({"applied": true}))
            }
            AUTOPILOT_CORRECT_OP => {
                let (request_id, verdict) = correction(call)?;
                let seq = self.desktop_decision_entry(connection_id, &request_id)?;
                self.correct_decision(seq, verdict).await?;
                Ok(serde_json::json!({"applied": true}))
            }
            _ => Err(CoreError::service("not an Autopilot change")),
        }
    }

    /// The activity entry Autopilot wrote for `request_id`, a request of `connection_id` (another connection's
    /// decisions are never found from a computer).
    fn desktop_decision_record(&self, connection_id: &str, request_id: &str) -> Result<AuditRecord, CoreError> {
        let since = crate::store::unix_now() - 30 * 86_400;
        self.store
            .audit_since(connection_id, since)?
            .into_iter()
            .find(|e| {
                e.info.autopilot.as_ref().is_some_and(|n| n.request_id == request_id) && !e.info.decided_by.is_empty()
            })
            .ok_or_else(|| CoreError::service("Autopilot did not decide that request of this computer."))
    }

    fn desktop_decision_entry(&self, connection_id: &str, request_id: &str) -> Result<i64, CoreError> {
        self.desktop_decision_record(connection_id, request_id).map(|e| e.seq)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn call(op: &str, args: &serde_json::Value) -> ConnectorCall {
        ConnectorCall {
            service: DESKTOP.to_owned(),
            op: op.to_owned(),
            args: args.as_object().unwrap().clone(),
        }
    }

    fn row(mode: Option<AutopilotMode>) -> ModeRow {
        ModeRow {
            mode,
            ..ModeRow::default()
        }
    }

    /// An Overnight running until `now + 600`, returning to `back` (`None`: the default, Manual).
    fn overnight(now: i64, back: Option<AutopilotMode>) -> ModeRow {
        ModeRow {
            mode: Some(AutopilotMode::Overnight),
            overnight_since: Some(now - 60),
            overnight_until: Some(now + 600),
            overnight_return: back,
            ..ModeRow::default()
        }
    }

    #[test]
    fn only_what_never_loosens_applies_at_once() {
        let now = 1_000;
        let manual = row(None);
        let auto = row(Some(AutopilotMode::Auto));
        let locked = row(Some(AutopilotMode::Lockdown));
        let mut bypassing = row(Some(AutopilotMode::Manual));
        bypassing.bypass_until = Some(now + 600);
        let set = ModeChange::Set;
        // Stricter: Lockdown always, stopping, a mode no looser than the user's.
        assert!(mode_is_stricter(set(AutopilotMode::Lockdown), &auto, now));
        assert!(mode_is_stricter(ModeChange::Stop, &bypassing, now));
        assert!(mode_is_stricter(set(AutopilotMode::Manual), &auto, now));
        assert!(mode_is_stricter(set(AutopilotMode::Manual), &bypassing, now), "ends the bypass, keeps Manual");
        assert!(mode_is_stricter(set(AutopilotMode::Auto), &auto, now));
        // Looser: Auto from Manual, Auto that would outlast a bypass over Manual, leaving Lockdown.
        assert!(!mode_is_stricter(set(AutopilotMode::Auto), &manual, now));
        assert!(!mode_is_stricter(set(AutopilotMode::Auto), &bypassing, now));
        assert!(!mode_is_stricter(set(AutopilotMode::Manual), &locked, now));
        assert!(!mode_is_stricter(set(AutopilotMode::Auto), &locked, now));
    }

    #[test]
    fn auto_over_an_overnight_is_judged_by_where_the_overnight_returns() {
        let now = 1_000;
        // Back to Manual at its end: Auto now would keep Auto after it, so it is asked.
        let to_manual = overnight(now, None);
        assert!(!mode_is_stricter(ModeChange::Set(AutopilotMode::Auto), &to_manual, now));
        assert!(!mode_is_stricter(
            ModeChange::Set(AutopilotMode::Auto),
            &overnight(now, Some(AutopilotMode::Manual)),
            now
        ));
        assert!(mode_is_stricter(ModeChange::Set(AutopilotMode::Manual), &to_manual, now));
        // Back to Auto at its end: Auto now ends it early and leaves the phone where it would have been.
        let to_auto = overnight(now, Some(AutopilotMode::Auto));
        assert!(mode_is_stricter(ModeChange::Set(AutopilotMode::Auto), &to_auto, now));
        // Once it has ended, only the mode the user picked counts.
        assert!(!mode_is_stricter(ModeChange::Set(AutopilotMode::Auto), &to_manual, now + 601));
    }

    #[test]
    fn the_phone_reads_the_call_itself() {
        let c = |args: serde_json::Value| mode_change(&call(AUTOPILOT_MODE_OP, &args));
        assert_eq!(c(json!({"mode": "auto"})).unwrap(), ModeChange::Set(AutopilotMode::Auto));
        assert!(c(json!({"mode": "bypass", "minutes": 60})).is_err(), "a bypass starts on the phone only");
        assert!(c(json!({"mode": "overnight"})).is_err(), "Overnight starts on the phone only");
        let fix = |args: serde_json::Value| correction(&call(AUTOPILOT_CORRECT_OP, &args));
        assert_eq!(
            fix(json!({"request_id": "r-1", "should_have": "deny"})).unwrap(),
            ("r-1".to_owned(), Verdict::Deny)
        );
        assert!(fix(json!({"request_id": "../x", "should_have": "deny"})).is_err());
        assert!(fix(json!({"request_id": "r1", "should_have": "ask"})).is_err());
        assert!(owner_only(DESKTOP, AUTOPILOT_MODE_OP) && owner_only(DESKTOP, AUTOPILOT_CORRECT_OP));
        assert!(!owner_only(DESKTOP, "ask"));
    }

    #[test]
    fn the_phone_words_the_question_from_its_own_state() {
        let now = 1_000;
        let auto = ModeChange::Set(AutopilotMode::Auto);
        let (q, d) = mode_question(auto, &row(None), now, "Dana's MacBook");
        assert_eq!(q, "Turn on Auto for every AI?");
        assert!(d.contains("Dana's MacBook") && d.contains("only your approval"));
        assert_eq!(
            mode_question(auto, &row(Some(AutopilotMode::Lockdown)), now, "x").0,
            "Turn on Auto and leave Lockdown?"
        );
        assert_eq!(
            mode_question(ModeChange::Set(AutopilotMode::Manual), &row(Some(AutopilotMode::Lockdown)), now, "x").0,
            "Set every AI to Manual and leave Lockdown?"
        );
        assert_eq!(mode_question(auto, &overnight(now, None), now, "x").0, "End Overnight and keep every AI on Auto?");
    }
}
