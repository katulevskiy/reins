//! What Autopilot's page and a profile's page count: how often Autopilot agreed with the user, and what a profile's
//! last days would have been at other thresholds (the stored evaluations replayed through [`decide::would`]).

use std::collections::BTreeMap;

use super::decide::{self, Probs};
use super::memory::MemoryRow;
use super::store::Suggestion;
use super::types::{ThresholdReplay, Verdict};

/// (share where Autopilot would have done what the user did, how many that is out of), over the decisions it would
/// have made on its own.
#[expect(clippy::cast_precision_loss, reason = "counts of at most a few thousand")]
pub fn agreement<'a>(rows: impl IntoIterator<Item = &'a MemoryRow>) -> (Option<f32>, u32) {
    let (mut compared, mut matched) = (0_u32, 0_u32);
    for row in rows {
        if row.would == Verdict::Ask {
            continue;
        }
        compared += 1;
        if row.would == row.label {
            matched += 1;
        }
    }
    ((compared > 0).then(|| matched as f32 / compared as f32), compared)
}

/// What the user wanted for an evaluated request: what they later said ("This was wrong"), else what they decided,
/// else what Autopilot decided and they left standing. `None` while it waits, or when Bypass or Lockdown decided it.
fn wanted(s: &Suggestion, corrections: &BTreeMap<String, Verdict>) -> Option<Verdict> {
    if let Some(v) = corrections.get(&s.request_id) {
        return Some(*v);
    }
    let d = s.decided.as_ref()?;
    matches!(d.by.as_str(), "" | "autopilot").then_some(d.verdict)
}

/// Replays a profile's evaluations at (θ_a, θ_d): each judged request is decided or asked as Auto would have at these
/// thresholds (class gates aside); requests that always ask count as asked.
pub fn replay(
    suggestions: &[Suggestion],
    corrections: &BTreeMap<String, Verdict>,
    thresholds: (f32, f32),
    days: u32,
) -> ThresholdReplay {
    let mut out = ThresholdReplay {
        days,
        ..ThresholdReplay::default()
    };
    for s in suggestions {
        if s.floor.is_some() {
            out.asked += 1;
            continue;
        }
        let Some(j) = &s.judged else {
            continue;
        };
        let probs = Probs {
            approve: j.probs[0],
            deny: j.probs[1],
            ask: j.probs[2],
        };
        match decide::would(probs, s.novel, j.escalate, thresholds) {
            Verdict::Ask => out.asked += 1,
            v => {
                out.decided += 1;
                if wanted(s, corrections).is_some_and(|w| w != v) {
                    out.undone += 1;
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::autopilot::memory::{Embedding, Source};
    use crate::autopilot::store::{Decided, Judged};

    fn suggestion(id: &str, approve: f32, deny: f32, decided: Option<(&str, Verdict)>) -> Suggestion {
        Suggestion {
            request_id: id.to_owned(),
            at: 0,
            kind: "request".to_owned(),
            connection_id: "c1".to_owned(),
            connection_label: "Claude".to_owned(),
            mode: super::super::types::AutopilotMode::Manual,
            profile_id: "p".to_owned(),
            profile_name: "Personal".to_owned(),
            class_key: "gmail/send".to_owned(),
            label: String::new(),
            target_key: String::new(),
            s_facts: String::new(),
            s_full: String::new(),
            novel: false,
            floor: None,
            judged: Some(Judged {
                model_id: "m".to_owned(),
                probs: [approve, deny, 1.0 - approve - deny],
                confidence: 0.9,
                escalate: 0.0,
                base_facts: [0.0; 3],
                base_full: [0.0; 3],
                e_facts: Embedding::from_vector(&[1.0]),
                e_full: Embedding::from_vector(&[1.0]),
                neighbours: vec![],
            }),
            verdict: Verdict::Ask,
            reason: String::new(),
            decided: decided.map(|(by, verdict)| Decided {
                by: by.to_owned(),
                verdict,
                at: 0,
            }),
        }
    }

    #[test]
    fn replaying_at_other_thresholds_counts_what_would_have_been_decided() {
        let user = |v| Some(("", v));
        let list = vec![
            suggestion("a", 0.97, 0.01, user(Verdict::Approve)),
            suggestion("b", 0.93, 0.02, user(Verdict::Approve)),
            suggestion("c", 0.91, 0.02, user(Verdict::Deny)),
            suggestion("d", 0.02, 0.88, user(Verdict::Deny)),
            suggestion("e", 0.50, 0.30, None),
            Suggestion {
                floor: Some("a grant".to_owned()),
                judged: None,
                ..suggestion("f", 0.0, 0.0, None)
            },
            // Bypass decided it without judging: nothing to replay.
            Suggestion {
                judged: None,
                ..suggestion("g", 0.0, 0.0, Some(("bypass", Verdict::Approve)))
            },
        ];
        let none = BTreeMap::new();
        let balanced = replay(&list, &none, (0.95, 0.90), 30);
        assert_eq!((balanced.decided, balanced.asked, balanced.undone), (1, 5, 0), "{balanced:?}");
        let custom = replay(&list, &none, (0.90, 0.85), 30);
        assert_eq!((custom.decided, custom.asked, custom.undone), (4, 2, 1), "c would have been approved: {custom:?}");
        assert_eq!(custom.days, 30);

        // An automatic approval the user later said was wrong counts as undone.
        let auto = vec![suggestion("h", 0.97, 0.01, Some(("autopilot", Verdict::Approve)))];
        assert_eq!(replay(&auto, &none, (0.95, 0.90), 30).undone, 0);
        let corrected = BTreeMap::from([("h".to_owned(), Verdict::Deny)]);
        assert_eq!(replay(&auto, &corrected, (0.95, 0.90), 30).undone, 1);
    }

    #[test]
    fn a_new_target_is_asked_at_any_threshold() {
        let novel = Suggestion {
            novel: true,
            ..suggestion("a", 0.99, 0.0, None)
        };
        let r = replay(&[novel], &BTreeMap::new(), (0.80, 0.80), 30);
        assert_eq!((r.decided, r.asked), (0, 1));
    }

    #[test]
    fn agreement_counts_only_what_autopilot_would_have_decided() {
        let row = |would, label| MemoryRow {
            request_id: String::new(),
            at: 0,
            connection_id: String::new(),
            class_key: String::new(),
            label,
            weight: 1.0,
            source: Source::User,
            would,
            short_label: String::new(),
            facts: String::new(),
            novel: false,
            target_key: String::new(),
            model_id: String::new(),
            base_facts: [0.0; 3],
            base_full: [0.0; 3],
            e_facts: Embedding::from_vector(&[1.0]),
            e_full: Embedding::from_vector(&[1.0]),
        };
        assert_eq!(agreement(&[]), (None, 0));
        let rows = [
            row(Verdict::Approve, Verdict::Approve),
            row(Verdict::Approve, Verdict::Approve),
            row(Verdict::Deny, Verdict::Deny),
            row(Verdict::Approve, Verdict::Deny),
            row(Verdict::Ask, Verdict::Deny),
        ];
        let (share, compared) = agreement(&rows);
        assert_eq!(compared, 4);
        assert!((share.unwrap() - 0.75).abs() < 1e-6);
    }
}
