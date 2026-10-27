//! The replay gate (Decisions v2 P1 acceptance, `docs/design/decisions-v2/07-roadmap.md` §1): on the committed corpus,
//! every labelled real risk is asked, no known false positive is, developer requests are read with Full certainty at
//! least 95% of the time, and nothing is worse than `baseline.json`.

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
    assert!(
        r.real_risk >= 9 && r.known_fp >= 64,
        "the labelled set shrank: {} risks, {} known false positives",
        r.real_risk,
        r.known_fp
    );
}

#[test]
fn the_guard_misses_what_the_engine_catches() {
    let corpus = Corpus::load(&reins_replay::default_corpus()).expect("the corpus loads");
    let guard = report::run(&corpus, &Policy::default(), EngineKind::Guard);
    let effects = report::run(&corpus, &Policy::default(), EngineKind::Effects);
    assert!(effects.recall > guard.recall, "{} vs {}", effects.recall, guard.recall);
    assert!(effects.asks < guard.asks);
}
