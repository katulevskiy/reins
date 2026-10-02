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
}

impl Decision {
    pub fn new(decided_by: &str, note: Option<AuditAutopilot>) -> Self {
        Self {
            decided_by: decided_by.to_owned(),
            note,
            seq: Mutex::new(None),
        }
    }
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
