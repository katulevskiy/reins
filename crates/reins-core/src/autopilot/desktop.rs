//! What the paired desktop app may change of Autopilot (`desktop_autopilot_mode`, `desktop_autopilot_correct`), and
//! the one rule that governs it: the computer never makes Reins more permissive on its own authority.
//!
//! The phone reads every such call itself and decides which way it goes, whatever the computer says:
//! - **Stricter, applied at once** (like ending a work session): Lockdown; a global mode no looser than the one the
//!   user picked; stopping the bypass or Overnight that runs; teaching Autopilot that a request should have been
//!   denied.
//! - **Looser, asked**: Auto from Manual, any bypass, leaving Lockdown, teaching Autopilot that a request should have
//!   been approved. These park like any request, worded by the phone, held to approve, asked every time (never
//!   remembered, never one tap), and are never decided automatically: not by Autopilot, not by Overnight, not by a
//!   bypass (which would approve its own extension) and not by Lockdown (which would deny the way out of it). Only the
//!   owner's own approval applies them, and the change is read again from the call when it is applied.
//!
//! Overnight is never started from the computer: its start asks the owner's check on the phone itself.
//! A correction only ever concerns a request of the connection that sends it.

use reins_proto::connector::{AUTOPILOT_CORRECT_OP, AUTOPILOT_MODE_OP, ConnectorCall, DESKTOP};

use super::modes::{self, ModeRow};
use super::types::{AutopilotMode, Verdict};
use crate::CoreError;
use crate::engine::Engine;

/// What a computer asks of the global mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModeChange {
    /// Manual, Auto or Lockdown.
    Set(AutopilotMode),
    /// A bypass of every AI for this many minutes.
    Bypass(u16),
    /// End the bypass or Overnight that runs.
    Stop,
}

/// A call only the owner's own approval may apply (never an automatic decision).
#[must_use]
pub fn owner_only(service: &str, op: &str) -> bool {
    service == DESKTOP && (op == AUTOPILOT_MODE_OP || op == AUTOPILOT_CORRECT_OP)
}

/// The change a `desktop_autopilot_mode` call asks for, read from the call (which the phone checked against its
/// tool description).
pub fn mode_change(call: &ConnectorCall) -> Result<ModeChange, CoreError> {
    let minutes = call.args.get("minutes").and_then(serde_json::Value::as_i64);
    let change = match call.str_arg("mode").unwrap_or_default() {
        "manual" => ModeChange::Set(AutopilotMode::Manual),
        "auto" => ModeChange::Set(AutopilotMode::Auto),
        "lockdown" => ModeChange::Set(AutopilotMode::Lockdown),
        "stop" => ModeChange::Stop,
        "bypass" => {
            let m = minutes.ok_or_else(|| CoreError::service("A bypass needs `minutes`."))?;
            let m = u16::try_from(m)
                .ok()
                .filter(|m| *m >= 1 && i64::from(*m) <= reins_proto::connector::MAX_DESKTOP_BYPASS_MINUTES);
            ModeChange::Bypass(
                m.ok_or_else(|| CoreError::service("A bypass from the computer lasts 1 minute to a day."))?,
            )
        }
        _ => return Err(CoreError::service("Unknown mode.")),
    };
    if minutes.is_some() && !matches!(change, ModeChange::Bypass(_)) {
        return Err(CoreError::service("`minutes` is only for a bypass."));
    }
    Ok(change)
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

/// Whether `change` never lets more through than `global` does now or later: then the phone applies it at once.
#[must_use]
pub fn mode_is_stricter(change: ModeChange, global: &ModeRow, now: i64) -> bool {
    match change {
        ModeChange::Set(AutopilotMode::Lockdown) | ModeChange::Stop => true,
        // Setting a mode ends any bypass or Overnight: what remains is the mode set, which must be no looser than the
        // one the user picked (a global Lockdown is the strictest: leaving it is asked).
        ModeChange::Set(mode) => openness(mode) <= openness(modes::global_base(global, now)),
        ModeChange::Bypass(_) => false,
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

/// The phone's wording of a change, for its approval and its activity entry (never the computer's words).
#[must_use]
pub fn mode_question(change: ModeChange, computer: &str) -> (String, String) {
    let detail = format!(
        "Asked from {computer}. It changes the mode of every AI on this phone. The computer cannot make Reins more \
         permissive on its own: only your approval here can."
    );
    let question = match change {
        ModeChange::Set(AutopilotMode::Auto) => "Turn on Auto for every AI?".to_owned(),
        ModeChange::Set(AutopilotMode::Manual) => "Set every AI to Manual (leaves Lockdown)?".to_owned(),
        ModeChange::Set(other) => format!("Set every AI to {}?", other.as_str()),
        ModeChange::Bypass(m) if m % 60 == 0 => format!("Start Bypass for every AI for {} h?", m / 60),
        ModeChange::Bypass(m) => format!("Start Bypass for every AI for {m} min?"),
        ModeChange::Stop => "Stop the bypass or Overnight that runs?".to_owned(),
    };
    (question, detail)
}

/// The phone's wording of a correction.
#[must_use]
pub fn correction_question(verdict: Verdict, computer: &str) -> (String, String) {
    let question = match verdict {
        Verdict::Approve => "Teach Autopilot to approve requests like this one?",
        _ => "Teach Autopilot to deny requests like this one?",
    }
    .to_owned();
    let detail = format!(
        "Asked from {computer} (\"This was wrong\" on a request Autopilot denied). Autopilot will lean towards \
         approving similar requests."
    );
    (question, detail)
}

impl Engine {
    /// Whether the desktop's `call` is stricter now, read by the phone (see the module docs).
    pub(crate) fn desktop_autopilot_stricter(&self, call: &ConnectorCall) -> Result<bool, CoreError> {
        match call.op.as_str() {
            AUTOPILOT_MODE_OP => {
                let global = self.store.ap_mode_row("")?;
                Ok(mode_is_stricter(mode_change(call)?, &global, crate::store::unix_now()))
            }
            AUTOPILOT_CORRECT_OP => Ok(correction(call)?.1 == Verdict::Deny),
            _ => Err(CoreError::service("not an Autopilot change")),
        }
    }

    /// Applies the desktop's `call` for its connection. `stricter`: the phone found it stricter now (applied without
    /// asking); otherwise the owner approved it.
    pub(crate) async fn apply_desktop_autopilot(
        &self,
        connection_id: &str,
        call: &ConnectorCall,
    ) -> Result<serde_json::Value, CoreError> {
        match call.op.as_str() {
            AUTOPILOT_MODE_OP => {
                let change = mode_change(call)?;
                match change {
                    ModeChange::Set(mode) => self.set_autopilot_mode(None, Some(mode), None).await?,
                    ModeChange::Bypass(minutes) => self.start_bypass(None, Some(u32::from(minutes))).await?,
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
    fn desktop_decision_entry(&self, connection_id: &str, request_id: &str) -> Result<i64, CoreError> {
        let since = crate::store::unix_now() - 30 * 86_400;
        self.store
            .audit_since(connection_id, since)?
            .into_iter()
            .find(|e| e.info.autopilot.as_ref().is_some_and(|n| n.request_id == request_id))
            .map(|e| e.seq)
            .ok_or_else(|| CoreError::service("Autopilot did not decide that request of this computer."))
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

    #[test]
    fn only_what_never_loosens_applies_at_once() {
        let now = 1_000;
        let manual = row(None);
        let auto = row(Some(AutopilotMode::Auto));
        let locked = row(Some(AutopilotMode::Lockdown));
        let mut bypassing = row(Some(AutopilotMode::Manual));
        bypassing.bypass_until = Some(now + 600);
        let set = |m| ModeChange::Set(m);
        // Stricter: Lockdown always, stopping, a mode no looser than the user's.
        assert!(mode_is_stricter(set(AutopilotMode::Lockdown), &auto, now));
        assert!(mode_is_stricter(ModeChange::Stop, &bypassing, now));
        assert!(mode_is_stricter(set(AutopilotMode::Manual), &auto, now));
        assert!(mode_is_stricter(set(AutopilotMode::Manual), &bypassing, now), "ends the bypass, keeps Manual");
        assert!(mode_is_stricter(set(AutopilotMode::Auto), &auto, now));
        // Looser: Auto from Manual, any bypass, leaving Lockdown, Auto that would outlast a bypass over Manual.
        assert!(!mode_is_stricter(set(AutopilotMode::Auto), &manual, now));
        assert!(!mode_is_stricter(set(AutopilotMode::Auto), &bypassing, now));
        assert!(!mode_is_stricter(ModeChange::Bypass(15), &manual, now));
        assert!(!mode_is_stricter(ModeChange::Bypass(15), &bypassing, now));
        assert!(!mode_is_stricter(set(AutopilotMode::Manual), &locked, now));
        assert!(!mode_is_stricter(set(AutopilotMode::Auto), &locked, now));
    }

    #[test]
    fn the_phone_reads_the_call_itself() {
        let c = |args: serde_json::Value| mode_change(&call(AUTOPILOT_MODE_OP, &args));
        assert_eq!(c(json!({"mode": "auto"})).unwrap(), ModeChange::Set(AutopilotMode::Auto));
        assert_eq!(c(json!({"mode": "bypass", "minutes": 60})).unwrap(), ModeChange::Bypass(60));
        assert!(c(json!({"mode": "bypass"})).is_err(), "a bypass from the computer always ends");
        assert!(c(json!({"mode": "bypass", "minutes": 100_000})).is_err());
        assert!(c(json!({"mode": "auto", "minutes": 5})).is_err());
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
    fn the_phone_words_the_question() {
        let (q, d) = mode_question(ModeChange::Bypass(120), "Dana's MacBook");
        assert_eq!(q, "Start Bypass for every AI for 2 h?");
        assert!(d.contains("Dana's MacBook") && d.contains("only your approval"));
        assert!(correction_question(Verdict::Approve, "x").0.contains("approve"));
    }
}
