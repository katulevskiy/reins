//! Auto's cold start against the real Laya package, when `REINS_LAYA_PKG` names its directory (skipped otherwise):
//! the hand-labelled risk cases of `tools/laya/golden.json` (`risk.cases`, severity in context) judged as the phone
//! judges them, then decided by Auto's verdict ([`decide::verdict`]) with none of the user's answers yet.
//!
//! What has to hold: nothing labelled risky, severe or catastrophic is approved, at day one (every target new, so
//! nothing at all), once the user approved the target once (the kind's first answer), and after 10 answers. Prints
//! the table it measured.

use std::path::{Path, PathBuf};

use reins_core::autopilot::decide::{self, Evidence, Probs};
use reins_core::{Preset, Verdict};
use reins_laya::Judge;
use serde_json::Value;

fn package() -> Option<PathBuf> {
    let Some(dir) = std::env::var_os("REINS_LAYA_PKG").map(PathBuf::from) else {
        eprintln!("skipped: set REINS_LAYA_PKG to a Laya package directory to run this test");
        return None;
    };
    assert!(dir.join("model.onnx").exists(), "REINS_LAYA_PKG={} has no model.onnx", dir.display());
    Some(dir)
}

/// One golden case as Auto sees it.
struct Case {
    id: String,
    connector: String,
    severity: usize,
    probs: Probs,
    escalate: f32,
    risk: Option<[f32; 4]>,
    /// The core's own reading holds it (a tripwire, or what it could not read with certainty).
    rules_hold: bool,
}

/// The approval head reads the request without the context line (only the risk reads it).
fn without_context(text: &str) -> String {
    text.lines().filter(|l| !l.starts_with("context: ")).collect::<Vec<_>>().join("\n")
}

fn judge_case(judge: &Judge, c: &Value) -> Case {
    let s = |k: &str| c[k].as_str().unwrap_or_default().to_owned();
    let (facts, full, neutral) = (s("s_facts"), s("s_full"), s("s_neutral"));
    let (a_facts, a_full) = (without_context(&facts), without_context(&full));
    let mut states = vec![a_facts.as_str(), a_full.as_str(), facts.as_str(), full.as_str()];
    if !neutral.is_empty() && neutral != facts && neutral != full {
        states.push(neutral.as_str());
    }
    let passes = judge.laya.run(&judge.runtime, &states).expect("the model runs");
    let p = |i: usize| Probs {
        approve: passes[i].probs[0],
        deny: passes[i].probs[1],
        ask: passes[i].probs[2],
    };
    let probs = decide::combine(p(0), p(1));
    // as the engine combines the risk: facts with full, then with the neutral view
    let risk =
        passes[2].risk.zip(passes[3].risk).map(|(f, u)| decide::risk_combine(f, u)).and_then(|r| match passes.get(4) {
            None => Some(r),
            Some(n) => n.risk.map(|n| decide::risk_combine(r, n)),
        });
    let severity = [c["risk_facts"].as_u64(), c["risk_full"].as_u64()].into_iter().flatten().max().unwrap_or(3);
    Case {
        id: s("id"),
        connector: s("connector"),
        severity: usize::try_from(severity).unwrap_or(3),
        probs,
        escalate: passes[0].escalate.max(passes[1].escalate),
        risk,
        rules_hold: c["rules_hold"].as_bool().unwrap_or(true),
    }
}

fn verdict(c: &Case, class_examples: usize, novel: bool) -> Verdict {
    let evidence = Evidence {
        class_examples,
        unlocked_by_hand: false,
        novel,
        benign: !c.rules_hold && decide::clearly_benign(c.risk),
    };
    decide::verdict(c.probs, c.escalate, Preset::Balanced.thresholds(), evidence)
}

#[test]
fn with_no_answers_auto_approves_only_what_is_benign() {
    let Some(dir) = package() else {
        return;
    };
    let judge = Judge::open(&dir).expect("open the package");
    let golden: Value = serde_json::from_str(
        &std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/laya/golden.json")).unwrap(),
    )
    .unwrap();
    let cases: Vec<Case> = golden["risk"]["cases"].as_array().unwrap().iter().map(|c| judge_case(&judge, c)).collect();
    if let Some(out) = std::env::var_os("REINS_COLD_DUMP") {
        let rows: Vec<Value> = cases
            .iter()
            .map(|c| {
                serde_json::json!({"id": c.id, "connector": c.connector, "severity": c.severity,
                    "approve": c.probs.approve, "deny": c.probs.deny, "escalate": c.escalate, "risk": c.risk,
                    "rules_hold": c.rules_hold})
            })
            .collect();
        std::fs::write(out, serde_json::to_string(&rows).unwrap()).unwrap();
    }
    // the floor: these never reach the model in the engine (secrets, sign-ins, purchases); counted apart
    let floor = |c: &Case| matches!(c.connector.as_str(), "secrets" | "keys" | "payments");
    let mut unsafe_approved = Vec::new();
    eprintln!("scenario                        benign approved   risky+ approved   floor approved   deny (cat.)");
    for (name, examples, novel) in
        [("day one (every target new)", 0, true), ("target approved once", 1, false), ("10 answers", 10, false)]
    {
        let benign: Vec<&Case> = cases.iter().filter(|c| c.severity == 0 && !floor(c)).collect();
        let approved = |c: &Case| verdict(c, examples, novel) == Verdict::Approve;
        let benign_ok = benign.iter().filter(|c| approved(c)).count();
        let worse: Vec<&Case> = cases.iter().filter(|c| c.severity > 0 && approved(c)).collect();
        let floors = cases.iter().filter(|c| floor(c) && approved(c)).count();
        let cat_deny = cases
            .iter()
            .filter(|c| verdict(c, examples, novel) == Verdict::Deny && decide::surely_catastrophic(c.risk))
            .map(|c| (c.severity, c.id.clone()))
            .collect::<Vec<_>>();
        eprintln!(
            "{name:<31} {benign_ok:>3} / {:<3}         {:>3} / {:<3}         {floors:>2}             {} (labels {:?})",
            benign.len(),
            worse.len(),
            cases.iter().filter(|c| c.severity > 0).count(),
            cat_deny.len(),
            cat_deny.iter().map(|(s, _)| *s).collect::<Vec<_>>(),
        );
        for c in &worse {
            eprintln!("  approved {} (severity {}): {:?} risk {:?}", c.id, c.severity, c.probs, c.risk);
        }
        unsafe_approved.extend(worse.iter().map(|c| c.id.clone()));
        assert_eq!(floors, 0, "{name}: a floor case would be approved");
    }
    for c in cases.iter().filter(|c| c.severity == 0 && verdict(c, 1, false) != Verdict::Approve) {
        eprintln!(
            "  benign asked {} ({}): approve {:.3} risk {:?} held {}",
            c.id, c.connector, c.probs.approve, c.risk, c.rules_hold
        );
    }
    assert!(unsafe_approved.is_empty(), "risky or worse approved with no answers: {unsafe_approved:?}");
}
