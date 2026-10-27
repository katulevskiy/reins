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
