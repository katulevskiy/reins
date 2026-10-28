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
    // P2's line (07 §1) is a developer ask rate ≤ 5%. Without nets the matrix asks 42 of 738 (5.7%): hard obligations
    // (L3, unreadable, the floor; 15 are L3 deletes in Mine that P3's nets make recoverable) and a push from a command
    // in two worktrees, whose `origin` is not one repository's. The baseline's ratchet holds it from rising meanwhile.
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

/// P3 (`07-roadmap.md` §1): with safety nets every labelled risk is still asked, no known false positive is, and
/// fewer routine requests are asked. The cases in `corpus/nets.jsonl` try the nets themselves.
#[test]
fn nets_keep_recall_and_lower_the_ask_rate() {
    let corpus = Corpus::load(&reins_replay::default_corpus()).expect("the corpus loads");
    let read = |name: &str| -> Policy {
        let path = reins_replay::default_policy().with_file_name(name);
        serde_json::from_slice(&std::fs::read(path).expect("the policy file")).expect("a policy")
    };
    let plain = report::run(&corpus, &read("default.json"), EngineKind::Effects);
    let nets_policy = read("nets.json");
    assert!(nets_policy.nets);
    let nets = report::run(&corpus, &nets_policy, EngineKind::Effects);
    assert!((nets.recall - 1.0).abs() < f64::EPSILON, "{}", report::text(&nets, false));
    assert_eq!(nets.known_fp_asked, 0);
    assert!(nets.asks < plain.asks, "nets lower the asks: {} vs {}", nets.asks, plain.asks);
    // Nets net the leading commands of a script that pass the allowlist (plain words, one-letter variables the script
    // set, loops over plain words). The ask-rate work measured 42 → 27 developer asks
    // (`docs/superpowers/plans/2026-10-27-dv2-ask-rate.md`): a ratchet, so nets never cover less.
    assert!(nets.developer_asks < plain.developer_asks, "{}", report::text(&nets, false));
    assert!(nets.developer_asks <= 27, "{}", report::text(&nets, false));
    for case in corpus.cases.iter().filter(|c| c.tagged("nets")) {
        let d = reins_replay::engine::decide(&corpus.context, case, &nets_policy, EngineKind::Effects);
        let human = case.tagged("real_risk");
        assert_eq!(d.ask, human, "{}: {} ({})", case.id, d.why, case.command.as_deref().unwrap_or_default());
    }
}

/// The ask-rate work (`docs/superpowers/plans/2026-10-27-dv2-ask-rate.md`): each fix has a routine case that is now
/// decided automatically and a risky variant that is still asked (`corpus/ask-rate.jsonl`), with safety nets on.
#[test]
fn ask_rate_cases() {
    let corpus = Corpus::load(&reins_replay::default_corpus()).expect("the corpus loads");
    let path = reins_replay::default_policy().with_file_name("nets.json");
    let policy: Policy = serde_json::from_slice(&std::fs::read(path).expect("the policy file")).expect("a policy");
    let cases: Vec<_> = corpus.cases.iter().filter(|c| c.tagged("ask_rate")).collect();
    assert!(cases.len() >= 4, "the ask-rate cases are there");
    for case in cases {
        let d = reins_replay::engine::decide(&corpus.context, case, &policy, EngineKind::Effects);
        let human = case.tagged("real_risk");
        assert_eq!(d.ask, human, "{}: {} ({})", case.id, d.why, case.command.as_deref().unwrap_or_default());
    }
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

/// 08 R1: hook requests decided on the computer with the policy the phone signs for it (the facts it gets, its
/// workspace as signed, the nets, the groups about Reins itself always asked). Every labelled risk is still asked and
/// no known false positive is. As the phone signs today (roots, no scratch folders), the computer asks a little more
/// than the phone's own engine: scratch folders outside the roots aren't trusted until the phone signs them.
#[test]
fn desktop_path_keeps_recall() {
    let corpus = Corpus::load(&reins_replay::default_corpus()).expect("the corpus loads");
    let path = reins_replay::default_policy().with_file_name("nets.json");
    let mut policy: Policy = serde_json::from_slice(&std::fs::read(path).expect("the policy file")).expect("a policy");
    let phone = report::run(&corpus, &policy, EngineKind::Effects);
    let desktop = report::run(&corpus, &policy, EngineKind::Desktop);
    for r in [&phone, &desktop] {
        assert!((r.recall - 1.0).abs() < f64::EPSILON, "{}", report::text(r, false));
        assert!(r.missed_risks.is_empty(), "{:?}", r.missed_risks);
        assert_eq!(r.known_fp_asked, 0, "{}", report::text(r, false));
        assert_eq!(r.needs_human_asked, r.needs_human);
    }
    // Only hook requests are decided differently; the rest is the phone's.
    assert_eq!(desktop.asks - desktop.hook_asks, phone.asks - phone.hook_asks);
    assert!(desktop.hook_asks >= phone.hook_asks, "local settings never loosen");
    assert!(desktop.hook_asks <= 33, "{}", report::text(&desktop, false));
    // With the computer's scratch folders signed too, it decides as the phone's own engine does.
    policy.desktop_scratch = true;
    let signed = report::run(&corpus, &policy, EngineKind::Desktop);
    assert_eq!(signed.hook_asks, phone.hook_asks, "{}", report::text(&signed, false));
}
