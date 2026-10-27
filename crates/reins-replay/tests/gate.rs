//! The replay gate (Decisions v2 P1 acceptance, `docs/design/decisions-v2/07-roadmap.md` §1): on the committed corpus,
//! every labelled real risk and every case for a person is asked, no known false positive is, and Full certainty and
//! the ask rate stay within their ratchets of `baseline.json`.

use reins_replay::corpus::Corpus;
use reins_replay::engine::{EngineKind, Policy};
use reins_replay::report::{self, Baseline};

#[test]
fn replay_gate() {
    let corpus = Corpus::load(&reins_replay::default_corpus()).expect("the corpus loads");
    let policy: Policy =
        serde_json::from_slice(&std::fs::read(reins_replay::default_policy()).expect("the policy file"))
            .expect("a policy");
    let r = report::run(&corpus, &policy, EngineKind::Effects);
    let baseline: Baseline =
        serde_json::from_slice(&std::fs::read(reins_replay::default_baseline()).expect("baseline.json"))
            .expect("a baseline");
    let fails = report::gate(&r, Some(&baseline));
    assert!(fails.is_empty(), "{fails:?}\n{}", report::text(&r, false));
    assert_eq!(r.real_risk_auto, 0, "no labelled risk Auto (P2)");
    // P2's line (07 §1) is a developer ask rate ≤ 5%. With the hardened engine and the review's stricter zones the
    // matrix asks 48 of 738 (6.5%): 44 hard obligations (L3, unreadable, the floor; 18 are L3 deletes in Mine that
    // P3's nets make recoverable) and 4 pushes after more than one `cd`, whose `origin` can't be read. The baseline's
    // ratchet holds it from rising meanwhile.
    let rate = baseline.developer_ask_rate.expect("the baseline records the developer ask rate");
    assert!(r.developer_ask_rate <= rate + 1e-9, "developer ask rate {:.4} > {rate:.4}", r.developer_ask_rate);
}

#[test]
fn the_gate_fails_a_shrunk_corpus_and_a_person_case_decided_automatically() {
    let corpus = Corpus::load(&reins_replay::default_corpus()).expect("the corpus loads");
    let mut r = report::run(&corpus, &Policy::default(), EngineKind::Effects);
    r.real_risk = 1;
    r.real_risk_asked = 1;
    assert!(report::gate(&r, None).iter().any(|f| f.contains("shrank")));
    let mut r = report::run(&corpus, &Policy::default(), EngineKind::Effects);
    r.needs_human_asked = r.needs_human.saturating_sub(1);
    assert!(report::gate(&r, None).iter().any(|f| f.contains("for a person")));
}

#[test]
fn full_certainty_and_the_ask_rate_are_ratchets() {
    let corpus = Corpus::load(&reins_replay::default_corpus()).expect("the corpus loads");
    let r = report::run(&corpus, &Policy::default(), EngineKind::Effects);
    let full = r.full_certainty.expect("the effects engine reads certainty");
    let base = |full_certainty: f64, ask_rate: f64| Baseline {
        recall: 1.0,
        known_fp_asked: 0,
        full_certainty,
        asks: r.asks,
        ask_rate,
        developer_ask_rate: None,
        overnight_decisions_mean: None,
    };
    assert!(report::gate(&r, Some(&base(full + 0.004, r.ask_rate))).is_empty(), "within half a point");
    assert!(report::gate(&r, Some(&base(full + 0.006, r.ask_rate))).iter().any(|f| f.contains("Full certainty")));
    assert!(report::gate(&r, Some(&base(full, r.ask_rate - 0.009))).is_empty(), "within a point");
    assert!(report::gate(&r, Some(&base(full, r.ask_rate - 0.011))).iter().any(|f| f.contains("ask rate")));
}

#[test]
fn the_guard_misses_what_the_engine_catches() {
    let corpus = Corpus::load(&reins_replay::default_corpus()).expect("the corpus loads");
    let guard = report::run(&corpus, &Policy::default(), EngineKind::Guard);
    let effects = report::run(&corpus, &Policy::default(), EngineKind::Effects);
    assert!(effects.recall > guard.recall, "{} vs {}", effects.recall, guard.recall);
    assert!(effects.asks < guard.asks);
}

#[test]
fn overnight_gate() {
    let corpus = Corpus::load(&reins_replay::default_corpus()).expect("the corpus loads");
    let policy: Policy =
        serde_json::from_slice(&std::fs::read(reins_replay::default_policy()).expect("the policy file"))
            .expect("a policy");
    let r = report::run(&corpus, &policy, EngineKind::Effects);
    let o = r.overnight.as_ref().expect("the effects engine replays Overnight");
    assert_eq!(o.real_risk_auto, 0, "no labelled risk runs in Overnight");
    assert_eq!(o.denied, 0, "Overnight never denies without a rule");
    // Overnight parks at most what Auto asks (its looser L1 for the owner's own work), never more but for people:
    // the default matrix already asks every Work and Outside L2.
    assert!(o.parks <= r.asks, "{} parks > {} asks", o.parks, r.asks);
    assert!(o.agent_nights > 0);
    let baseline: Baseline =
        serde_json::from_slice(&std::fs::read(reins_replay::default_baseline()).expect("baseline.json"))
            .expect("a baseline");
    let mean = baseline.overnight_decisions_mean.expect("the baseline records Overnight's decisions");
    assert!(o.decisions_mean <= mean + report::OVERNIGHT_SLACK + 1e-9, "{} > {mean}", o.decisions_mean);
}
