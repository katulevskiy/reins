//! What the phone says beside its answers, heard while a question is asked: the request's id, where Autopilot leans
//! while it waits ([`reins_proto::autopilot::Suggestion`]), and who decided once it answered
//! ([`reins_proto::autopilot::Decided`]). The server's desktop client tells it to the question being asked on this
//! task ([`hear`]); the activity log keeps it with the question's entry ([`crate::journal::Entry::hear`]).
//!
//! All of it is shown, none of it trusted: it decides nothing on this computer.

use std::future::Future;
use std::sync::Arc;

use tokio::sync::watch;

use crate::journal::{Attribution, Lean};
use crate::server::client::{CallAnswer, CallStatus};

/// What was heard so far about the question being asked.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Heard {
    pub request_id: Option<String>,
    pub leaning: Option<Lean>,
    pub decided: Option<Attribution>,
}

tokio::task_local! {
    static HEARING: Arc<watch::Sender<Heard>>;
}

/// Runs `question` hearing what the phone says beside its answers; the receiver sees each change while it runs.
pub fn listen<F: Future>(question: F) -> (impl Future<Output = F::Output>, watch::Receiver<Heard>) {
    let (tx, rx) = watch::channel(Heard::default());
    (HEARING.scope(Arc::new(tx), question), rx)
}

/// What `answer` says beside the answer itself.
#[must_use]
pub fn heard_of(answer: &CallAnswer) -> Heard {
    let mut heard = Heard {
        request_id: Some(answer.request_id.clone()),
        ..Heard::default()
    };
    match &answer.status {
        CallStatus::Answered(_) => heard.decided = answer.decided.clone().map(Attribution::from),
        CallStatus::Pending | CallStatus::Offline => heard.leaning = answer.suggestion.as_ref().map(Lean::from),
        CallStatus::Deferred {
            ..
        }
        | CallStatus::Closed {
            ..
        } => {}
    }
    heard
}

/// Tells the question being asked on this task (if one listens) what the server answered about it.
pub fn hear(answer: &CallAnswer) {
    let _outside = HEARING.try_with(|tx| {
        tx.send_if_modified(|heard| {
            let next = heard_of(answer);
            let changed = next != *heard;
            *heard = next;
            changed
        })
    });
}

#[cfg(test)]
mod tests {
    use reins_proto::autopilot::{Decided, DecidedBy, Leaning, Reason, Suggestion};
    use reins_proto::relay::RelayOutcome;

    use super::*;

    fn answer(status: CallStatus, decided: Option<Decided>, suggestion: Option<Suggestion>) -> CallAnswer {
        CallAnswer {
            request_id: "r1".to_owned(),
            status,
            decided,
            suggestion,
        }
    }

    #[tokio::test]
    async fn a_question_hears_the_leaning_then_who_decided() {
        let (question, mut rx) = listen(async {
            hear(&answer(
                CallStatus::Pending,
                None,
                Some(Suggestion {
                    v: 1,
                    leaning: Leaning::Approve,
                    confidence: 0.914,
                }),
            ));
            hear(&answer(
                CallStatus::Answered(RelayOutcome::Denied {
                    reason: None,
                }),
                Some(Decided {
                    by: DecidedBy::Autopilot,
                    confidence: Some(0.97),
                    reasons: vec![Reason::Routine {
                        approve: 97,
                    }],
                    correctable: true,
                }),
                None,
            ));
        });
        question.await;
        let heard = rx.borrow_and_update().clone();
        assert_eq!(heard.request_id.as_deref(), Some("r1"));
        assert_eq!(heard.leaning, None, "answered: it leans no more");
        let decided = heard.decided.unwrap();
        assert_eq!((decided.by, decided.confidence, decided.correctable), (DecidedBy::Autopilot, Some(97), true));
        assert_eq!(
            decided.reasons,
            [Reason::Routine {
                approve: 97
            }]
        );
        // Nobody listening: nothing happens.
        hear(&answer(CallStatus::Pending, None, None));
    }
}
