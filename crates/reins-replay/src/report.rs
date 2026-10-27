//! A replay's numbers (01 §13, 07 §3): ask rate, real-risk recall, known false positives, certainty, and the
//! decisions that changed against the guard.

use serde::{Deserialize, Serialize};

use crate::corpus::{Case, Corpus, Label};
use crate::engine::{self, Decision, EngineKind, Policy};

/// A case whose decision differs between two engines.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Changed {
    pub id: String,
    /// `ask → auto` or `auto → ask`.
    pub change: String,
    pub why: String,
    pub label: Label,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// The first effect lines.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lines: Vec<String>,
}

/// One case that a gate cares about.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Miss {
    pub id: String,
    pub why: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lines: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub engine: String,
    pub policy: String,
    pub cases: usize,
    /// Recorded requests (not adversarial): the rates below are theirs.
    pub history: usize,
    pub asks: usize,
    pub adversarial: usize,
    pub adversarial_asks: usize,
    pub ask_rate: f64,
    /// Asks among developer requests (commands, files, git).
    pub developer_cases: usize,
    pub developer_asks: usize,
    pub developer_ask_rate: f64,
    pub real_risk: usize,
    pub real_risk_asked: usize,
    /// Labelled risks the matrix would have let Auto do (P2's success line: none).
    pub real_risk_auto: usize,
    pub recall: f64,
    pub needs_human: usize,
    pub needs_human_asked: usize,
    pub known_fp: usize,
    pub known_fp_asked: usize,
    pub routine: usize,
    pub routine_asked: usize,
    /// Share of developer cases the engine read with `Full` certainty (`None` for the guard engine).
    #[serde(default)]
    pub full_certainty: Option<f64>,
    pub partial: usize,
    pub unreadable: usize,
    /// Days the recorded cases span, and asks per day over them.
    pub days: f64,
    pub asks_per_day: f64,
    /// Real risks that were not asked: must be empty.
    pub missed_risks: Vec<Miss>,
    /// Known false positives that were still asked: must be empty.
    pub false_alarms: Vec<Miss>,
    /// Routine cases asked (beyond the known false positives).
    pub routine_asks: Vec<Miss>,
    pub changed: Vec<Changed>,
}

fn rate(n: usize, d: usize) -> f64 {
    if d == 0 {
        return 0.0;
    }
    #[expect(clippy::cast_precision_loss, reason = "counts far below 2^52")]
    let r = n as f64 / d as f64;
    r
}

fn lines(d: &Decision) -> Vec<String> {
    d.analysis.as_ref().map(|a| a.lines().into_iter().take(4).collect()).unwrap_or_default()
}

/// Replays `corpus` through `engine` with `policy`, comparing with the guard.
#[must_use]
pub fn run(corpus: &Corpus, policy: &Policy, engine: EngineKind) -> Report {
    let mut r = Report {
        engine: format!("{engine:?}").to_lowercase(),
        policy: policy.name.clone(),
        cases: corpus.cases.len(),
        ..Report::default()
    };
    let mut full = 0;
    let (mut first, mut last) = (i64::MAX, i64::MIN);
    for case in &corpus.cases {
        let d = engine::decide(&corpus.context, case, policy, engine);
        let base = engine::decide(&corpus.context, case, policy, EngineKind::Guard);
        if case.at > 0 && !case.tagged("adversarial") {
            first = first.min(case.at);
            last = last.max(case.at);
        }
        tally(&mut r, case, &d);
        if let Some(a) = d.analysis.as_ref().filter(|_| !case.tagged("adversarial")) {
            match a.certainty {
                reins_policy::effects::Certainty::Full if case.kind.developer() => full += 1,
                reins_policy::effects::Certainty::Partial => r.partial += 1,
                reins_policy::effects::Certainty::None => r.unreadable += 1,
                reins_policy::effects::Certainty::Full => {}
            }
        }
        if d.ask != base.ask {
            r.changed.push(Changed {
                id: case.id.clone(),
                change: if d.ask {
                    "auto → ask".to_owned()
                } else {
                    "ask → auto".to_owned()
                },
                why: d.why.clone(),
                label: case.label,
                tags: case.tags.clone(),
                lines: lines(&d),
            });
        }
    }
    r.ask_rate = rate(r.asks, r.history);
    r.developer_ask_rate = rate(r.developer_asks, r.developer_cases);
    r.recall = if r.real_risk == 0 {
        1.0
    } else {
        rate(r.real_risk_asked, r.real_risk)
    };
    if engine == EngineKind::Effects {
        r.full_certainty = Some(rate(full, r.developer_cases));
    }
    if last > first {
        #[expect(clippy::cast_precision_loss, reason = "seconds in a few weeks")]
        let days = ((last - first) as f64 / 86_400.0).max(1.0);
        r.days = (days * 10.0).round() / 10.0;
        #[expect(clippy::cast_precision_loss, reason = "counts far below 2^52")]
        let per_day = r.asks as f64 / days;
        r.asks_per_day = (per_day * 10.0).round() / 10.0;
    }
    r
}

fn tally(r: &mut Report, case: &Case, d: &Decision) {
    let miss = || Miss {
        id: case.id.clone(),
        why: d.why.clone(),
        lines: lines(d),
        note: case.note.clone(),
    };
    // Rates and certainty are the recorded history's; adversarial cases count toward recall and labels only.
    if case.tagged("adversarial") {
        r.adversarial += 1;
        r.adversarial_asks += usize::from(d.ask);
    } else {
        r.history += 1;
        r.asks += usize::from(d.ask);
        if case.kind.developer() {
            r.developer_cases += 1;
            r.developer_asks += usize::from(d.ask);
        }
    }
    if case.real_risk() {
        r.real_risk += 1;
        if d.ask {
            r.real_risk_asked += 1;
        } else {
            r.missed_risks.push(miss());
        }
        if d.evaluation.as_ref().is_some_and(|e| e.outcome == reins_policy::matrix::Outcome::Auto) {
            r.real_risk_auto += 1;
        }
    }
    match case.label {
        Label::NeedsHuman => {
            r.needs_human += 1;
            r.needs_human_asked += usize::from(d.ask);
        }
        Label::Routine => {
            r.routine += 1;
            r.routine_asked += usize::from(d.ask);
        }
    }
    if case.tagged("known_fp") {
        r.known_fp += 1;
        if d.ask {
            r.known_fp_asked += 1;
            r.false_alarms.push(miss());
        }
    } else if case.label == Label::Routine && d.ask {
        r.routine_asks.push(miss());
    }
}

/// The numbers a later run must hold (committed as `baseline.json`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Baseline {
    pub recall: f64,
    pub known_fp_asked: usize,
    pub full_certainty: f64,
    pub asks: usize,
    /// Asks among the recorded requests, as a share.
    #[serde(default)]
    pub ask_rate: f64,
    /// Asks ÷ developer requests (P2 on).
    #[serde(default)]
    pub developer_ask_rate: Option<f64>,
}

/// How far Full certainty may fall below the baseline's (a ratchet, not a fixed bar).
pub const FULL_CERTAINTY_SLACK: f64 = 0.005;
/// How far the ask rate may rise above the baseline's.
pub const ASK_RATE_SLACK: f64 = 0.01;

/// Why a report fails the gate (empty: it passes). Hard: every labelled risk and every case for a person asked, no
/// known false positive asked, the labelled set no smaller than the minimums. Against `baseline.json`: Full certainty
/// on developer requests no more than half a point below it, the ask rate no more than a point above it.
#[must_use]
pub fn gate(r: &Report, baseline: Option<&Baseline>) -> Vec<String> {
    let mut fails = Vec::new();
    // A corpus that lost its labelled cases would pass anything.
    if r.real_risk < MIN_REAL_RISK || r.known_fp < MIN_KNOWN_FP {
        fails.push(format!(
            "the labelled set shrank: {} risks (at least {MIN_REAL_RISK}), {} known false positives (at least {MIN_KNOWN_FP})",
            r.real_risk, r.known_fp
        ));
    }
    if r.needs_human_asked < r.needs_human {
        fails.push(format!(
            "{} case(s) labelled for a person decided automatically",
            r.needs_human - r.needs_human_asked
        ));
    }
    if r.real_risk_asked < r.real_risk {
        fails.push(format!(
            "real-risk recall {:.3}: {} labelled risk(s) not asked",
            r.recall,
            r.real_risk - r.real_risk_asked
        ));
    }
    if r.real_risk_auto > 0 {
        fails.push(format!("{} labelled risk(s) would be Auto", r.real_risk_auto));
    }
    if r.known_fp_asked > 0 {
        fails.push(format!("{} known false positive(s) still asked", r.known_fp_asked));
    }
    if let Some(b) = baseline {
        if r.recall < b.recall {
            fails.push(format!("recall dropped from {:.3} to {:.3}", b.recall, r.recall));
        }
        if let Some(rate) = b.developer_ask_rate
            && r.developer_ask_rate > rate + 1e-9
        {
            fails.push(format!(
                "developer ask rate rose from {:.2}% to {:.2}%",
                rate * 100.0,
                r.developer_ask_rate * 100.0
            ));
        }
        if r.known_fp_asked > b.known_fp_asked {
            fails.push(format!("known false positives asked rose from {} to {}", b.known_fp_asked, r.known_fp_asked));
        }
        if let Some(full) = r.full_certainty
            && full + FULL_CERTAINTY_SLACK + 1e-9 < b.full_certainty
        {
            fails.push(format!(
                "Full certainty on developer requests dropped from {:.1}% to {:.1}% (more than {:.1} points)",
                b.full_certainty * 100.0,
                full * 100.0,
                FULL_CERTAINTY_SLACK * 100.0
            ));
        }
        if r.ask_rate > b.ask_rate + ASK_RATE_SLACK + 1e-9 {
            fails.push(format!(
                "the ask rate rose from {:.1}% to {:.1}% (more than {:.0} point)",
                b.ask_rate * 100.0,
                r.ask_rate * 100.0,
                ASK_RATE_SLACK * 100.0
            ));
        }
    }
    fails
}

/// The fewest labelled real risks and known false positives a corpus the gate trusts has.
pub const MIN_REAL_RISK: usize = 9;
pub const MIN_KNOWN_FP: usize = 64;

/// The report as text.
#[must_use]
pub fn text(r: &Report, verbose: bool) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();
    let pct = |x: f64| format!("{:.1}%", x * 100.0);
    let _w = writeln!(s, "engine {} · policy {}", r.engine, r.policy);
    let _w = writeln!(
        s,
        "recorded requests {} · asks {} ({}) · {} asks/day over {} days · adversarial cases {} (asked {})",
        r.history,
        r.asks,
        pct(r.ask_rate),
        r.asks_per_day,
        r.days,
        r.adversarial,
        r.adversarial_asks
    );
    let _w = writeln!(
        s,
        "developer requests {} · asks {} ({})",
        r.developer_cases,
        r.developer_asks,
        pct(r.developer_ask_rate)
    );
    let _w = writeln!(
        s,
        "real-risk recall {}/{} ({}) · labelled risks Auto {}",
        r.real_risk_asked,
        r.real_risk,
        pct(r.recall),
        r.real_risk_auto
    );
    let _w = writeln!(
        s,
        "needs a human: asked {}/{} · routine asked {}/{}",
        r.needs_human_asked, r.needs_human, r.routine_asked, r.routine
    );
    let _w = writeln!(s, "known false positives asked {}/{}", r.known_fp_asked, r.known_fp);
    if let Some(f) = r.full_certainty {
        let _w = writeln!(
            s,
            "Full certainty on developer requests {} · partial {} · unreadable {}",
            pct(f),
            r.partial,
            r.unreadable
        );
    }
    let ask_to_auto = r.changed.iter().filter(|c| c.change.starts_with("ask")).count();
    let _w = writeln!(
        s,
        "changed against the guard: {} ask → auto, {} auto → ask",
        ask_to_auto,
        r.changed.len() - ask_to_auto
    );
    let list = |s: &mut String, title: &str, items: &[Miss]| {
        if !items.is_empty() {
            let _w = writeln!(s, "\n{title}:");
            for m in items {
                let _w = writeln!(s, "  {} · {} · {}", m.id, m.why, m.lines.join(" | "));
            }
        }
    };
    list(&mut s, "MISSED REAL RISKS", &r.missed_risks);
    list(&mut s, "KNOWN FALSE POSITIVES STILL ASKED", &r.false_alarms);
    if verbose {
        list(&mut s, "routine cases asked", &r.routine_asks);
        let _w = writeln!(s, "\nchanged:");
        for c in &r.changed {
            let _w = writeln!(s, "  {} · {} · {:?} · {} · {}", c.id, c.change, c.label, c.why, c.lines.join(" | "));
        }
    }
    s
}
