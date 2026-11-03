//! One row per request in the Activity.
//!
//! Before a policy-checked execution the log gets an execution record (`execute`, outcome `started`), and after it
//! `sent` or `unknown` (`Engine::authorization_attempt_audit`): crash and "do not repeat" evidence. The log keeps them
//! as they are (it is append-only: the org audit, the oplog and its hash chains depend on it). Only what is shown
//! folds them: they are never rows of their own, and what they tell goes on the rows of the same request
//! ([`ActivityExecution`]). A request with only execution records (its outcome row pruned, or never written) keeps
//! the newest of them as its row, so nothing vanishes.
//!
//! The same rules run in SQL for search, its filters and the day counts (`audit_search`): [`shown`] there is
//! `SHOWN`, an [`ExecutionState::Unknown`] is `doubt`.

use std::collections::HashMap;

use super::audit::AuditRecord;
use crate::types::{ActivityExecution, ExecutionState};

/// A start with no result after this long (seconds) is an unknown outcome.
pub const EXECUTION_UNKNOWN_AFTER: i64 = 5 * 60;

/// The action of an execution record.
pub(crate) const EXECUTE: &str = "execute";

/// The request an entry is about: its authorization trace's, else the one Autopilot noted; `None` when neither says.
pub(crate) fn request_of(r: &AuditRecord) -> Option<&str> {
    r.info
        .authorization
        .as_ref()
        .map(|t| t.request_id.as_str())
        .or_else(|| r.info.autopilot.as_ref().map(|a| a.request_id.as_str()))
        .filter(|id| !id.is_empty())
}

/// Whether an entry is an execution record: evidence about its request, never a row of its own.
pub(crate) fn is_execution(r: &AuditRecord) -> bool {
    r.action == EXECUTE && r.info.authorization.is_some()
}

/// What the fold reads of an entry of a request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Facts {
    pub seq: i64,
    pub at: i64,
    pub execution: bool,
    pub outcome: String,
}

impl Facts {
    pub(crate) fn of(r: &AuditRecord) -> Self {
        Self {
            seq: r.seq,
            at: r.at,
            execution: is_execution(r),
            outcome: r.outcome.clone(),
        }
    }

    /// Whether it tells how the attempt ended: an execution record's result, or any outcome row of the request (a
    /// desktop's push or command answered, an email sent) but a held send, which has not gone yet.
    fn closes(&self) -> bool {
        if self.execution {
            matches!(self.outcome.as_str(), "sent" | "unknown")
        } else {
            self.outcome != "held"
        }
    }
}

/// The execution of a request, from all its entries; `None` when it has no execution record.
pub(crate) fn execution(group: &[Facts], now: i64) -> Option<ActivityExecution> {
    let mut records: Vec<&Facts> = group.iter().filter(|f| f.execution).collect();
    if records.is_empty() {
        return None;
    }
    records.sort_by_key(|f| f.seq);
    let started_at = records.iter().filter(|f| f.outcome == "started").map(|f| f.at).min();
    let state = if group.iter().any(|f| f.outcome == "unknown") {
        ExecutionState::Unknown
    } else if group.iter().any(|f| f.outcome == "succeeded_withheld") {
        ExecutionState::Withheld
    } else if group.iter().any(Facts::closes) {
        ExecutionState::Done
    } else if started_at.is_some_and(|at| now.saturating_sub(at) < EXECUTION_UNKNOWN_AFTER) {
        ExecutionState::Pending
    } else {
        ExecutionState::Unknown
    };
    Some(ActivityExecution {
        state,
        records: records.iter().map(|f| f.seq).collect(),
        started_at: started_at.unwrap_or(0),
    })
}

/// Whether entry `seq` of a request is a row: any but an execution record, and that only when the request has nothing
/// else, the newest of them.
pub(crate) fn shown(group: &[Facts], seq: i64) -> bool {
    match group.iter().find(|f| f.seq == seq) {
        Some(me) if me.execution => !group.iter().any(|f| !f.execution || f.seq > seq),
        _ => true,
    }
}

/// `records` folded, in their order: each request's execution records leave the list, and what they tell goes on the
/// request's rows. Only the entries given are read: a request's entries outside them are not known here.
pub(crate) fn fold(records: Vec<AuditRecord>, now: i64) -> Vec<(AuditRecord, Option<ActivityExecution>)> {
    let mut groups: HashMap<String, Vec<Facts>> = HashMap::new();
    for r in &records {
        if let Some(id) = request_of(r) {
            groups.entry(id.to_owned()).or_default().push(Facts::of(r));
        }
    }
    records
        .into_iter()
        .filter_map(|r| {
            let Some(group) = request_of(&r).and_then(|id| groups.get(id)) else {
                return Some((r, None));
            };
            shown(group, r.seq).then(|| {
                let execution = execution(group, now);
                (r, execution)
            })
        })
        .collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::authorization_trace::{AuthorizationPhase, AuthorizationReviewSource, AuthorizationTrace};
    use crate::store::tests::{audit_record, open};

    const NOW: i64 = 1_000_000;

    fn trace(request: &str, phase: AuthorizationPhase) -> AuthorizationTrace {
        AuthorizationTrace {
            v: 1,
            attempt_id: format!("attempt-{request}"),
            request_id: request.to_owned(),
            request_digest: String::new(),
            policy_state_revision: 1,
            policy_state_digest: String::new(),
            policies: Vec::new(),
            determining_policy_ids: Vec::new(),
            required_reviews: Vec::new(),
            source: AuthorizationReviewSource::Automatic,
            verified_facts: Vec::new(),
            checked_at: 0,
            phase,
        }
    }

    /// An execution record of `request`, as `Engine::authorization_attempt_audit` writes it.
    pub(crate) fn attempt(request: &str, outcome: &str, at: i64) -> AuditRecord {
        let phase = match outcome {
            "started" => AuthorizationPhase::Started,
            "sent" => AuthorizationPhase::Succeeded,
            _ => AuthorizationPhase::Unknown,
        };
        let mut r = audit_record();
        r.at = at;
        r.service = "desktop".to_owned();
        r.op = "ssh".to_owned();
        EXECUTE.clone_into(&mut r.action);
        outcome.clone_into(&mut r.outcome);
        r.info.authorization = Some(trace(request, phase));
        r
    }

    /// The row that decided `request`, with its trace stamped as the connector flow does.
    pub(crate) fn decision(request: &str, outcome: &str, at: i64) -> AuditRecord {
        let mut r = audit_record();
        r.at = at;
        r.service = "desktop".to_owned();
        r.op = "ssh".to_owned();
        "write".clone_into(&mut r.action);
        outcome.clone_into(&mut r.outcome);
        r.info.decided_by = "bypass".to_owned();
        r.info.authorization = Some(trace(request, AuthorizationPhase::Succeeded));
        r
    }

    /// `records` appended (oldest first), then read back newest first, folded.
    fn folded(records: &[AuditRecord], now: i64) -> Vec<(i64, String, Option<ActivityExecution>)> {
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());
        for r in records {
            store.append_audit(r).unwrap();
        }
        fold(store.activity(100).unwrap(), now).into_iter().map(|(r, e)| (r.seq, r.outcome, e)).collect()
    }

    fn state(e: Option<&ActivityExecution>) -> Option<ExecutionState> {
        e.map(|e| e.state)
    }

    #[test]
    fn a_start_folds_into_the_decision_of_its_request() {
        let rows = folded(&[attempt("r1", "started", NOW - 10), decision("r1", "sent", NOW - 9)], NOW);
        assert_eq!(rows.len(), 1, "one row per request: {rows:?}");
        let (seq, outcome, execution) = &rows[0];
        assert_eq!((*seq, outcome.as_str()), (2, "sent"));
        let execution = execution.as_ref().unwrap();
        assert_eq!(execution.state, ExecutionState::Done, "fine: nothing to show");
        assert_eq!((execution.records.as_slice(), execution.started_at), (&[1][..], NOW - 10));
    }

    #[test]
    fn every_kind_of_execution_record_folds_and_unknown_wins() {
        // The permission path: started, its decision, then the result after it.
        let rows = folded(
            &[
                attempt("r1", "started", NOW - 30),
                decision("r1", "released", NOW - 29),
                attempt("r1", "unknown", NOW - 28),
            ],
            NOW,
        );
        assert_eq!(rows.len(), 1);
        assert_eq!((rows[0].0, state(rows[0].2.as_ref())), (2, Some(ExecutionState::Unknown)));
        assert_eq!(rows[0].2.as_ref().unwrap().records, [1, 3]);
        let rows = folded(
            &[
                attempt("r1", "started", NOW - 30),
                decision("r1", "released", NOW - 29),
                attempt("r1", "sent", NOW - 28),
            ],
            NOW,
        );
        assert_eq!((rows.len(), state(rows[0].2.as_ref())), (1, Some(ExecutionState::Done)));
        let rows = folded(&[attempt("r1", "started", NOW - 30), decision("r1", "succeeded_withheld", NOW - 29)], NOW);
        assert_eq!((rows.len(), state(rows[0].2.as_ref())), (1, Some(ExecutionState::Withheld)));
    }

    #[test]
    fn a_start_with_no_result_is_pending_for_five_minutes_then_unknown() {
        let records = [decision("other", "sent", NOW - 1), attempt("r1", "started", NOW - EXECUTION_UNKNOWN_AFTER + 1)];
        let rows = folded(&records, NOW);
        assert_eq!(rows.len(), 2, "a request with nothing else keeps its record as its row");
        assert_eq!((rows[0].1.as_str(), state(rows[0].2.as_ref())), ("started", Some(ExecutionState::Pending)));
        let rows = folded(&records, NOW + 1);
        assert_eq!(state(rows[0].2.as_ref()), Some(ExecutionState::Unknown), "five minutes on, no result: unknown");
        // A held send has not gone yet: it closes nothing.
        let held = [attempt("r1", "started", NOW - 600), decision("r1", "held", NOW - 599)];
        let rows = folded(&held, NOW);
        assert_eq!((rows.len(), rows[0].1.as_str()), (1, "held"));
        assert_eq!(state(rows[0].2.as_ref()), Some(ExecutionState::Unknown));
    }

    /// A push through the desktop's git proxy, or a command it runs: the phone answers it (the credential, the yes)
    /// and logs that as the request's outcome row; the desktop never reports more. That row closes the request.
    #[test]
    fn a_request_carried_out_off_phone_is_closed_by_its_own_outcome_row() {
        let mut push = decision("push-1", "sent", NOW - 3_000);
        push.service = "github".to_owned();
        push.op = "git_push".to_owned();
        push.detail = "approved: Push 1 commit to katulevskiy/exnomic-server".to_owned();
        let mut started = attempt("push-1", "started", NOW - 3_001);
        started.service = "github".to_owned();
        started.op = "git_push".to_owned();
        let rows =
            folded(&[started, push, attempt("ssh-1", "started", NOW - 2_000), decision("ssh-1", "sent", NOW)], NOW);
        assert_eq!(rows.len(), 2, "{rows:?}");
        assert!(
            rows.iter().all(|(_, outcome, e)| outcome == "sent" && state(e.as_ref()) == Some(ExecutionState::Done))
        );
        // Matched by the request Autopilot noted when the outcome row carries no trace.
        let mut untraced = decision("push-2", "sent", NOW - 1);
        untraced.info.authorization = None;
        untraced.info.autopilot = Some(crate::store::AuditAutopilot {
            request_id: "push-2".to_owned(),
            mode: crate::autopilot::AutopilotMode::Bypass,
            suggested: crate::autopilot::Verdict::Approve,
            p_approve: 1.0,
            p_deny: 0.0,
            confidence: 1.0,
            profile_id: String::new(),
            profile_name: String::new(),
            neighbours: Vec::new(),
            reason: String::new(),
            risk: None,
            matrix: None,
        });
        let rows = folded(&[attempt("push-2", "started", NOW - 900), untraced], NOW);
        assert_eq!((rows.len(), state(rows[0].2.as_ref())), (1, Some(ExecutionState::Done)));
    }

    #[test]
    fn a_request_without_its_decision_row_keeps_one_row() {
        let rows = folded(&[attempt("r1", "started", NOW - 900), attempt("r1", "unknown", NOW - 899)], NOW);
        assert_eq!(rows.len(), 1, "nothing vanishes, nothing doubles");
        assert_eq!((rows[0].0, rows[0].1.as_str()), (2, "unknown"));
        assert_eq!(rows[0].2.as_ref().unwrap().records, [1, 2]);
        assert_eq!(state(rows[0].2.as_ref()), Some(ExecutionState::Unknown));
        // Rows of no request, or of requests without execution records, are as they were.
        let mut plain = audit_record();
        plain.at = NOW;
        let rows = folded(&[plain, decision("r2", "sent", NOW)], NOW);
        assert_eq!(rows.iter().map(|r| r.2.clone()).collect::<Vec<_>>(), [None, None]);
    }
}
