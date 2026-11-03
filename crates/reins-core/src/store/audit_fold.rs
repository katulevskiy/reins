//! One row per request in the Activity.
//!
//! Before a policy-checked execution the log gets an execution record (`execute`, outcome `started`), and after it
//! `sent` or `unknown` (`Engine::authorization_attempt_audit`): crash and "do not repeat" evidence. The log keeps them
//! as they are (it is append-only: the org audit, the oplog and its hash chains depend on it). Only what is shown
//! folds them: they are never rows of their own, and what they tell goes on the rows of the same attempt
//! ([`ActivityExecution`]). An attempt with only execution records (its outcome row pruned, or never written) keeps
//! the newest of them as its row, so nothing vanishes.
//!
//! Entries are grouped by what the phone itself bound when the attempt started ([`group_of`]): the trace's attempt id
//! (a UUID made on the phone) and the request digest, with the request id. The request id alone comes from the
//! server or the agent: a reused one never folds one request's records into another's row. An attempt is closed
//! only by a result the phone wrote after the call returned ([`Facts::result`]), never by an approval written
//! before it, and only when that result is newer than the attempt's latest start.
//!
//! The same rules run in SQL for search, its filters and the day counts (`audit_search`): [`shown`] there is
//! `SHOWN`, an [`ExecutionState::Unknown`] is `doubt`.

use std::collections::HashMap;

use super::audit::AuditRecord;
use crate::authorization_trace::AuthorizationPhase;
use crate::types::{ActivityExecution, ExecutionState};

/// A start with no result after this long (seconds) is an unknown outcome.
pub const EXECUTION_UNKNOWN_AFTER: i64 = 5 * 60;

/// The action of an execution record.
pub(crate) const EXECUTE: &str = "execute";

/// The attempt an entry belongs to, from its authorization trace: the request id with the attempt id and request
/// digest the phone bound when the attempt started. `None` for an entry without a trace (nothing the phone bound
/// says which attempt it is: it is never grouped, whatever request id it names).
pub(crate) fn group_of(r: &AuditRecord) -> Option<String> {
    let t = r.info.authorization.as_ref()?;
    (!t.attempt_id.is_empty() && !t.request_id.is_empty())
        .then(|| format!("{}\u{1f}{}\u{1f}{}", t.request_id, t.attempt_id, t.request_digest))
}

/// Whether an entry is an execution record: evidence about its attempt, never a row of its own.
pub(crate) fn is_execution(r: &AuditRecord) -> bool {
    r.action == EXECUTE && r.info.authorization.is_some()
}

/// What the fold reads of an entry of an attempt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Facts {
    pub seq: i64,
    pub at: i64,
    pub execution: bool,
    pub outcome: String,
    /// It is the phone's result of the attempt ([`Facts::is_result`]).
    pub result: bool,
    /// It says the attempt may or may not have happened.
    pub unknown: bool,
}

impl Facts {
    pub(crate) fn of(r: &AuditRecord) -> Self {
        let execution = is_execution(r);
        let phase = r.info.authorization.as_ref().map(|t| t.phase);
        let result = Self::is_result(execution, &r.outcome, phase);
        Self {
            seq: r.seq,
            at: r.at,
            execution,
            result,
            unknown: r.outcome == "unknown" || (result && phase == Some(AuthorizationPhase::Unknown)),
            outcome: r.outcome.clone(),
        }
    }

    /// Whether an entry is the phone's own result of its attempt, written after the call returned: an execution
    /// record's `sent` / `unknown` / `succeeded_withheld`, or the outcome row the execution flow stamps with the
    /// attempt's trace in its `Succeeded` or `Unknown` phase (the connector and permission flows, and the desktop's
    /// push credential and command approval, which the phone hands over and the desktop never reports on). An
    /// approval, a held send or any row without that stamp is not: the call may have stopped part way after it.
    pub(crate) fn is_result(execution: bool, outcome: &str, phase: Option<AuthorizationPhase>) -> bool {
        if execution {
            matches!(outcome, "sent" | "unknown" | "succeeded_withheld")
        } else {
            outcome != "held" && matches!(phase, Some(AuthorizationPhase::Succeeded | AuthorizationPhase::Unknown))
        }
    }
}

/// The execution of an attempt, from all its entries; `None` when it has no execution record.
pub(crate) fn execution(group: &[Facts], now: i64) -> Option<ActivityExecution> {
    let mut records: Vec<&Facts> = group.iter().filter(|f| f.execution).collect();
    if records.is_empty() {
        return None;
    }
    records.sort_by_key(|f| f.seq);
    let started = || records.iter().filter(|f| f.outcome == "started");
    let started_at = started().map(|f| f.at).min();
    let latest_start = started().map(|f| f.seq).max().unwrap_or(0);
    // Only a result written after the latest start says how it ended.
    let results: Vec<&Facts> = group.iter().filter(|f| f.result && f.seq > latest_start).collect();
    let state = if group.iter().any(|f| f.unknown) {
        ExecutionState::Unknown
    } else if results.iter().any(|f| f.outcome == "succeeded_withheld") {
        ExecutionState::Withheld
    } else if !results.is_empty() {
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

/// Whether entry `seq` of an attempt is a row: any but an execution record, and that only when the attempt has
/// nothing else, the newest of them.
pub(crate) fn shown(group: &[Facts], seq: i64) -> bool {
    match group.iter().find(|f| f.seq == seq) {
        Some(me) if me.execution => !group.iter().any(|f| !f.execution || f.seq > seq),
        _ => true,
    }
}

/// `records` folded, in their order: each attempt's execution records leave the list, and what they tell goes on the
/// attempt's rows. Only the entries given are read: an attempt's entries outside them are not known here.
pub(crate) fn fold(records: Vec<AuditRecord>, now: i64) -> Vec<(AuditRecord, Option<ActivityExecution>)> {
    let mut groups: HashMap<String, Vec<Facts>> = HashMap::new();
    for r in &records {
        if let Some(key) = group_of(r) {
            groups.entry(key).or_default().push(Facts::of(r));
        }
    }
    records
        .into_iter()
        .filter_map(|r| {
            let Some(group) = group_of(&r).and_then(|key| groups.get(&key)) else {
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
    use crate::authorization_trace::{AuthorizationReviewSource, AuthorizationTrace};
    use crate::store::tests::{audit_record, open};

    const NOW: i64 = 1_000_000;

    /// The trace of `request`'s attempt (its attempt id and digest made from `attempt`), in `phase`.
    fn trace(request: &str, attempt: &str, phase: AuthorizationPhase) -> AuthorizationTrace {
        AuthorizationTrace {
            v: 1,
            attempt_id: format!("attempt-{attempt}"),
            request_id: request.to_owned(),
            request_digest: format!("digest-{attempt}"),
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

    /// The group key of `request`'s attempt as these helpers write it.
    pub(crate) fn key(request: &str) -> String {
        let mut r = audit_record();
        r.info.authorization = Some(trace(request, request, AuthorizationPhase::Started));
        group_of(&r).unwrap()
    }

    /// An execution record of `request`, as `Engine::authorization_attempt_audit` writes it.
    pub(crate) fn attempt(request: &str, outcome: &str, at: i64) -> AuditRecord {
        attempt_of(request, request, outcome, at)
    }

    pub(crate) fn attempt_of(request: &str, attempt: &str, outcome: &str, at: i64) -> AuditRecord {
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
        r.info.authorization = Some(trace(request, attempt, phase));
        r
    }

    /// The row that decided `request`, with its trace stamped as the execution flow does once the call returned.
    pub(crate) fn decision(request: &str, outcome: &str, at: i64) -> AuditRecord {
        decision_of(request, request, outcome, at, AuthorizationPhase::Succeeded)
    }

    pub(crate) fn decision_of(
        request: &str,
        attempt: &str,
        outcome: &str,
        at: i64,
        phase: AuthorizationPhase,
    ) -> AuditRecord {
        let mut r = audit_record();
        r.at = at;
        r.service = "desktop".to_owned();
        r.op = "ssh".to_owned();
        "write".clone_into(&mut r.action);
        outcome.clone_into(&mut r.outcome);
        r.info.decided_by = "bypass".to_owned();
        r.info.authorization = Some(trace(request, attempt, phase));
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
        assert_eq!(rows.len(), 2, "an attempt with nothing else keeps its record as its row");
        assert_eq!((rows[0].1.as_str(), state(rows[0].2.as_ref())), ("started", Some(ExecutionState::Pending)));
        let rows = folded(&records, NOW + 1);
        assert_eq!(state(rows[0].2.as_ref()), Some(ExecutionState::Unknown), "five minutes on, no result: unknown");
        // A held send has not gone yet: it closes nothing.
        let held = [attempt("r1", "started", NOW - 600), decision("r1", "held", NOW - 599)];
        let rows = folded(&held, NOW);
        assert_eq!((rows.len(), rows[0].1.as_str()), (1, "held"));
        assert_eq!(state(rows[0].2.as_ref()), Some(ExecutionState::Unknown));
    }

    /// An action the phone carries out itself: an approval row of the request is no result. The call may have
    /// stopped part way after it, so with no result five minutes on, it must not be repeated.
    #[test]
    fn an_approval_of_a_phone_run_action_never_closes_its_attempt() {
        let mut approval = decision_of("r1", "r1", "released", NOW - 600, AuthorizationPhase::Started);
        approval.service = "telegram".to_owned();
        approval.op = "send".to_owned();
        let mut started = attempt("r1", "started", NOW - 599);
        started.service = "telegram".to_owned();
        started.op = "send".to_owned();
        let rows = folded(&[approval.clone(), started.clone()], NOW);
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!((rows[0].1.as_str(), state(rows[0].2.as_ref())), ("released", Some(ExecutionState::Unknown)));
        let rows = folded(&[approval, started], NOW - 400);
        assert_eq!(state(rows[0].2.as_ref()), Some(ExecutionState::Pending), "within five minutes it is running");
        // A result stamped before the latest start (written for an earlier start) does not close the later one.
        let early = [
            attempt("r2", "started", NOW - 900),
            decision("r2", "sent", NOW - 899),
            attempt("r2", "started", NOW - 898),
        ];
        assert_eq!(state(folded(&early, NOW)[0].2.as_ref()), Some(ExecutionState::Unknown));
        // Nor does an outcome row that carries no trace: nothing binds it to the attempt.
        let mut untraced = decision("r3", "sent", NOW - 1);
        untraced.info.authorization = None;
        let rows = folded(&[attempt("r3", "started", NOW - 900), untraced], NOW);
        assert_eq!(rows.len(), 2, "never grouped with it: {rows:?}");
        assert_eq!(state(rows[0].2.as_ref()), None);
        assert_eq!((rows[1].1.as_str(), state(rows[1].2.as_ref())), ("started", Some(ExecutionState::Unknown)));
    }

    /// A push through the desktop's git proxy, or a command it runs: the phone answers it (the credential, the yes)
    /// and logs that, stamped with the attempt's trace, as the request's outcome row; the desktop never reports
    /// more. That row closes the attempt.
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
    }

    /// The request id comes from the server or the agent. Two requests that share one, attempted apart (each with
    /// its own attempt id and digest), never fold together: neither hides the other's start.
    #[test]
    fn requests_sharing_an_id_with_different_digests_stay_apart() {
        let rows = folded(
            &[
                attempt_of("same", "first", "started", NOW - 900),
                decision_of("same", "first", "sent", NOW - 899, AuthorizationPhase::Succeeded),
                attempt_of("same", "second", "started", NOW - 800),
            ],
            NOW,
        );
        assert_eq!(rows.len(), 2, "{rows:?}");
        assert_eq!((rows[0].1.as_str(), state(rows[0].2.as_ref())), ("started", Some(ExecutionState::Unknown)));
        assert_eq!((rows[1].1.as_str(), state(rows[1].2.as_ref())), ("sent", Some(ExecutionState::Done)));
        assert_eq!(
            (rows[0].2.as_ref().unwrap().records.as_slice(), rows[1].2.as_ref().unwrap().records.as_slice()),
            (&[3][..], &[1][..])
        );
        // The same attempt id with another digest is another attempt too.
        let mut forged = decision_of("same", "x", "sent", NOW - 1, AuthorizationPhase::Succeeded);
        forged.info.authorization.as_mut().unwrap().attempt_id = "attempt-second".to_owned();
        let rows = folded(&[attempt_of("same", "second", "started", NOW - 800), forged], NOW);
        assert_eq!(rows.len(), 2, "{rows:?}");
        assert_eq!(state(rows[1].2.as_ref()), Some(ExecutionState::Unknown));
    }

    #[test]
    fn a_request_without_its_decision_row_keeps_one_row() {
        let rows = folded(&[attempt("r1", "started", NOW - 900), attempt("r1", "unknown", NOW - 899)], NOW);
        assert_eq!(rows.len(), 1, "nothing vanishes, nothing doubles");
        assert_eq!((rows[0].0, rows[0].1.as_str()), (2, "unknown"));
        assert_eq!(rows[0].2.as_ref().unwrap().records, [1, 2]);
        assert_eq!(state(rows[0].2.as_ref()), Some(ExecutionState::Unknown));
        // Rows of no attempt, or of attempts without execution records, are as they were.
        let mut plain = audit_record();
        plain.at = NOW;
        let rows = folded(&[plain, decision("r2", "sent", NOW)], NOW);
        assert_eq!(rows.iter().map(|r| r.2.clone()).collect::<Vec<_>>(), [None, None]);
    }
}
