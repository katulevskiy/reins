//! Who decides the request being answered, so that its activity entry says so (spec §8).
//!
//! An approval or a denial goes through the same code whoever decides it; around that code the engine sets the
//! decider and Autopilot's note, and [`crate::store::Store::append_audit`] stamps them on the entry it writes. The
//! entry's row number is kept so the notification can point at it.

use std::future::Future;
use std::sync::Mutex;

use crate::store::{AuditAutopilot, AuditInfo};

tokio::task_local! {
    static DECISION: Decision;
}

/// The decider of the request being answered.
#[derive(Debug, Default)]
pub struct Decision {
    /// "" (the user) | "autopilot" | "bypass" | "lockdown"
    pub decided_by: String,
    pub note: Option<AuditAutopilot>,
    /// The activity entry written while deciding.
    pub seq: Mutex<Option<i64>>,
    /// The origin a permission granted while deciding is stored with (a bypass marks its own, to take them back when
    /// it ends); `None`: the usual one.
    pub grant_origin: Option<String>,
}

impl Decision {
    pub fn new(decided_by: &str, note: Option<AuditAutopilot>) -> Self {
        Self {
            decided_by: decided_by.to_owned(),
            note,
            seq: Mutex::new(None),
            grant_origin: None,
        }
    }

    /// Permissions granted while deciding are stored with `origin`.
    #[must_use]
    pub fn granting_as(mut self, origin: Option<String>) -> Self {
        self.grant_origin = origin;
        self
    }
}

/// The origin to store a permission granted now with: the decision's own, else `usual`.
pub fn grant_origin(usual: &str) -> String {
    DECISION.try_with(|d| d.grant_origin.clone()).ok().flatten().unwrap_or_else(|| usual.to_owned())
}

/// Runs `work` with `decision` as the decider; returns its output and the activity entry it wrote.
pub async fn deciding<F: Future>(decision: Decision, work: F) -> (F::Output, Option<i64>) {
    DECISION
        .scope(decision, async move {
            let out = work.await;
            let seq = DECISION.with(|d| *d.seq.lock().unwrap_or_else(std::sync::PoisonError::into_inner));
            (out, seq)
        })
        .await
}

/// Adds the current decider to an activity entry about to be written (entries written outside a decision are left
/// as they are).
pub fn stamp(info: &mut AuditInfo) {
    DECISION
        .try_with(|d| {
            if info.decided_by.is_empty() {
                info.decided_by.clone_from(&d.decided_by);
            }
            if info.autopilot.is_none() {
                info.autopilot.clone_from(&d.note);
            }
        })
        .ok();
}

/// The detail of a denial's activity entry: who denied it.
pub fn denied_detail() -> String {
    let by = DECISION.try_with(|d| d.decided_by.clone()).unwrap_or_default();
    match by.as_str() {
        "autopilot" => "denied by Autopilot",
        "lockdown" => "denied by Lockdown",
        "rule" => "denied by your rule",
        _ => "denied by the user",
    }
    .to_owned()
}

/// Remembers the row number of the entry written while deciding.
pub fn written(seq: i64) {
    DECISION
        .try_with(|d| {
            *d.seq.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(seq);
        })
        .ok();
}

/// Who decided the request being answered, as its answer tells the desktop app (`None`: the user, or nothing decided
/// around this answer). The answer passes the server unsealed, so it carries Autopilot's own reading and counts of
/// what the user did with similar requests, never their labels (they can name other AIs' requests).
pub fn told() -> Option<reins_proto::autopilot::Decided> {
    use reins_proto::autopilot::{Decided, DecidedBy};
    DECISION
        .try_with(|d| {
            let by = match d.decided_by.as_str() {
                // An explicit rule decided it on its own: the desktop app (which has no word for a rule) shows it as
                // an automatic decision.
                "autopilot" | "rule" => DecidedBy::Autopilot,
                "overnight" => DecidedBy::Overnight,
                "bypass" => DecidedBy::Bypass,
                "lockdown" => DecidedBy::Lockdown,
                _ => return None,
            };
            let note = d.note.as_ref();
            let reasons = note.map(|n| reasons_of(by, n)).unwrap_or_default();
            Some(
                Decided {
                    by,
                    confidence: note.map(|n| n.confidence).filter(|c| *c > 0.0),
                    reasons,
                    correctable: note.is_some() && by != DecidedBy::Lockdown,
                }
                .cleaned(),
            )
        })
        .ok()
        .flatten()
}

/// Autopilot's reasons for the desktop app, as codes and numbers only (the desktop words them): the note's own `reason`
/// can quote another request's label or what the risk model read in this one, so it never leaves the phone this way.
fn reasons_of(by: reins_proto::autopilot::DecidedBy, n: &AuditAutopilot) -> Vec<reins_proto::autopilot::Reason> {
    use crate::autopilot::{RiskLevel, Verdict};
    use reins_proto::autopilot::{DecidedBy, Reason, RiskWord};
    #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "a percentage of a probability")]
    let pct = |p: f32| (p.clamp(0.0, 1.0) * 100.0).round() as u8;
    let mut reasons = Vec::new();
    match (by, n.suggested) {
        (DecidedBy::Bypass, _) => reasons.push(Reason::Bypass),
        (DecidedBy::Lockdown, _) => reasons.push(Reason::Lockdown),
        (_, Verdict::Approve) => reasons.push(Reason::Routine {
            approve: pct(n.p_approve),
        }),
        (_, Verdict::Deny) => reasons.push(Reason::Harmful {
            deny: pct(n.p_deny),
        }),
        (_, Verdict::Ask) => {}
    }
    let said = |verb: &str| {
        let count = n.neighbours.iter().filter(|l| l.starts_with(&format!("{verb}:"))).count();
        u8::try_from(count).unwrap_or(u8::MAX)
    };
    let (approved, denied) = (said("approved"), said("denied"));
    if approved > 0 || denied > 0 {
        reasons.push(Reason::LikeYours {
            approved,
            denied,
        });
    }
    if let Some(risk) = &n.risk {
        reasons.push(Reason::Risk {
            level: match risk.level {
                RiskLevel::Benign => RiskWord::Routine,
                RiskLevel::Risky => RiskWord::Undoable,
                RiskLevel::Severe => RiskWord::HardToUndo,
                RiskLevel::Catastrophic => RiskWord::Irreversible,
            },
        });
    }
    reasons
}
