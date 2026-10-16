//! From the model's two passes, the memory and the adapter to probabilities and a verdict (spec §3, §5.3).
//!
//! **Prompt-injection rule.** Everything is computed twice, once from `S_facts` (its pass, its embedding, the memory's
//! facts embeddings, the facts adapter) and once from `S_full`; the approve probability used is the smaller of the two,
//! the deny probability the larger. Text the AI wrote can only move a request toward deny or ask. The risk of the two
//! passes is combined the same way ([`risk_combine`]): it can only rise.

use super::adapter::{AdapterVariant, features};
use super::memory::{Knn, MemoryRow, Variant, knn};
use super::types::Verdict;

/// Examples of a class from which memory and adapter outweigh the base model.
pub const FULL_WEIGHT_EXAMPLES: usize = 20;
/// The act head's escalate probability above which Autopilot never approves on its own.
pub const ESCALATE_LIMIT: f32 = 0.5;
/// The approve probability Auto needs in a class with no answers of the user yet (the cold start); it relaxes to the
/// profile's own threshold by [`FULL_WEIGHT_EXAMPLES`] answers ([`approve_bar`]). A profile stricter than this keeps
/// its own.
pub const COLD_APPROVE: f32 = 0.97;
/// While the bar is not the profile's own, the risk head must call the request clearly benign: P(benign) at least
/// this…
pub const COLD_BENIGN: f32 = 0.9;
/// …and P(severe) + P(catastrophic) below this: just above where the head rests on harmless requests (up to about
/// 0.049 on the golden set of `tools/laya`), so any more sign of harm asks.
pub const COLD_SEVERE_MAX: f32 = 0.05;
/// Auto denies on its own, in a class that has not earned auto-deny, only what the risk head rates catastrophic at
/// least this sure (and the deny probability reaches the profile's threshold).
pub const CATASTROPHIC_DENY: f32 = 0.9;

/// One forward pass of the model over one situation text.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pass {
    /// Calibrated logits (raw / temperature) of approve, deny, ask.
    pub logits: [f32; 3],
    /// Their softmax.
    pub probs: [f32; 3],
    /// P(escalate) from the act head.
    pub escalate: f32,
    /// The embedding, unit length.
    pub pooled: Vec<f32>,
    /// Calibrated severity probabilities (benign, risky, severe, catastrophic) when the model rates risk.
    pub risk: Option<[f32; 4]>,
}

pub fn softmax(logits: [f32; 3]) -> [f32; 3] {
    let max = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let e = logits.map(|l| (l - max).exp());
    let sum: f32 = e.iter().sum();
    e.map(|x| x / sum)
}

pub fn softmax4(logits: [f32; 4]) -> [f32; 4] {
    let max = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let e = logits.map(|l| (l - max).exp());
    let sum: f32 = e.iter().sum();
    e.map(|x| x / sum)
}

/// The prompt-injection rule for risk: per class k, P(class ≥ k) is the larger of the two passes. The result is a
/// distribution again (it sums to 1); text the AI wrote can only move it toward the severe end.
pub fn risk_combine(facts: [f32; 4], full: [f32; 4]) -> [f32; 4] {
    let at_least = |p: [f32; 4]| [1.0, p[1] + p[2] + p[3], p[2] + p[3], p[3]];
    let (a, b) = (at_least(facts), at_least(full));
    let c: [f32; 4] = std::array::from_fn(|k| a[k].max(b[k]).clamp(0.0, 1.0));
    [(c[0] - c[1]).max(0.0), (c[1] - c[2]).max(0.0), (c[2] - c[3]).max(0.0), c[3]]
}

/// (approve, deny, ask), summing to 1.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Probs {
    pub approve: f32,
    pub deny: f32,
    pub ask: f32,
}

impl Probs {
    /// 1 − normalised entropy.
    pub fn confidence(&self) -> f32 {
        let h: f32 = [self.approve, self.deny, self.ask].iter().filter(|p| **p > 0.0).map(|p| -p * p.ln()).sum();
        (1.0 - h / 3.0_f32.ln()).clamp(0.0, 1.0)
    }
}

/// Weights (base, neighbours, adapter): base only without examples of the class, mostly memory from
/// [`FULL_WEIGHT_EXAMPLES`] on.
#[expect(clippy::cast_precision_loss, reason = "a count of at most 20")]
pub fn weights(class_examples: usize, adapter: bool) -> (f32, f32, f32) {
    if class_examples == 0 {
        return (1.0, 0.0, 0.0);
    }
    let m = class_examples.min(FULL_WEIGHT_EXAMPLES) as f32 / FULL_WEIGHT_EXAMPLES as f32;
    let (b, k, a) = (1.0 - 0.8 * m, 0.4 * m, 0.4 * m);
    if adapter {
        (b, k, a)
    } else {
        (b, k + a, 0.0)
    }
}

/// The blend of the base distribution with the neighbours' and the adapter's approve probabilities.
pub fn blend(base: [f32; 3], q_knn: f32, q_adapter: Option<f32>, class_examples: usize) -> Probs {
    let (wb, wk, wa) = weights(class_examples, q_adapter.is_some());
    let qa = q_adapter.unwrap_or(q_knn);
    Probs {
        approve: wb * base[0] + wk * q_knn + wa * qa,
        deny: wb * base[1] + wk * (1.0 - q_knn) + wa * (1.0 - qa),
        ask: wb * base[2],
    }
}

/// One side (facts or full) of an evaluation.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Side {
    pub probs: Probs,
    pub knn: Knn,
}

/// What a request is, for scoring it against the memory.
#[derive(Clone, Copy, Debug)]
pub struct Subject<'a> {
    pub class_key: &'a str,
    /// Keyed hash of the target ("" without one).
    pub target_key: &'a str,
    pub novel: bool,
    /// Remembered decisions of its class.
    pub class_examples: usize,
}

/// Scores one pass against the profile's memory and adapter.
pub fn side(
    pass: &Pass,
    rows: &[&MemoryRow],
    variant: Variant,
    adapter: Option<&AdapterVariant>,
    subject: Subject<'_>,
) -> Side {
    let vote = knn(&pass.pooled, rows, variant, subject.target_key, None);
    let q_adapter =
        adapter.map(|a| a.lr.predict(&features(&pass.pooled, pass.logits, &vote, subject.class_key, subject.novel)));
    let mut probs = blend(pass.probs, vote.p_approve, q_adapter, subject.class_examples);
    if let Some(platt) = adapter.and_then(|a| a.platt.as_ref()) {
        let decided = probs.approve + probs.deny;
        if decided > f32::EPSILON {
            let p = platt.apply(probs.approve / decided);
            probs.approve = p * decided;
            probs.deny = (1.0 - p) * decided;
        }
    }
    Side {
        probs,
        knn: vote,
    }
}

/// The prompt-injection rule: the smaller approve, the larger deny.
pub fn combine(facts: Probs, full: Probs) -> Probs {
    let approve = facts.approve.min(full.approve).clamp(0.0, 1.0);
    let deny = facts.deny.max(full.deny).clamp(0.0, 1.0 - approve);
    Probs {
        approve,
        deny,
        ask: (1.0 - approve - deny).max(0.0),
    }
}

/// The rule before the cold start, which Overnight keeps: approve only a target seen approved before, at the
/// profile's thresholds, with the act head not asking to escalate. Auto decides with [`verdict`].
pub fn would(p: Probs, novel: bool, escalate: f32, thresholds: (f32, f32)) -> Verdict {
    if p.approve >= thresholds.0 && !novel && escalate < ESCALATE_LIMIT {
        Verdict::Approve
    } else if p.deny >= thresholds.1 {
        Verdict::Deny
    } else {
        Verdict::Ask
    }
}

/// The approve probability Auto needs after `class_examples` answers of the user in a class: [`COLD_APPROVE`] (or
/// the profile's own threshold `normal` if stricter) with none, down to `normal` in a straight line by
/// [`FULL_WEIGHT_EXAMPLES`].
///
/// The line is the one [`weights`] moves on: the extra margin is in proportion to the share of the blend that still
/// rests on the base model alone (all of it with no answers, its floor of 0.2 from 20 on). Memory and adapter earn
/// trust exactly as fast as they earn weight, and a class is never held to a stricter bar than its evidence warrants
/// once the user's answers dominate it.
#[expect(clippy::cast_precision_loss, reason = "a count of at most 20")]
pub fn approve_bar(class_examples: usize, normal: f32) -> f32 {
    let cold = COLD_APPROVE.max(normal);
    let left = 1.0 - class_examples.min(FULL_WEIGHT_EXAMPLES) as f32 / FULL_WEIGHT_EXAMPLES as f32;
    normal + (cold - normal) * left
}

/// The risk head calls a request clearly benign (after the two-pass rule): P(benign) ≥ [`COLD_BENIGN`] and
/// P(severe) + P(catastrophic) < [`COLD_SEVERE_MAX`]. Not without a readable risk.
pub fn clearly_benign(risk: Option<[f32; 4]>) -> bool {
    risk.is_some_and(|r| {
        r.iter().all(|p| p.is_finite() && (0.0..=1.0).contains(p))
            && (r.iter().sum::<f32>() - 1.0).abs() < 1e-3
            && r[0] >= COLD_BENIGN
            && r[2] + r[3] < COLD_SEVERE_MAX
    })
}

/// The risk head rates a request catastrophic at least [`CATASTROPHIC_DENY`] sure.
pub fn surely_catastrophic(risk: Option<[f32; 4]>) -> bool {
    risk.is_some_and(|r| r[3].is_finite() && r[3] >= CATASTROPHIC_DENY)
}

/// What Auto's verdict on a request rests on besides the probabilities (kept with an evaluation, so that it can be
/// replayed at other thresholds).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Evidence {
    /// The user's answers remembered for its class (with this model): the bar relaxes as they grow.
    pub class_examples: usize,
    /// The user unlocked the class by hand: the profile's own threshold from the first request.
    pub unlocked_by_hand: bool,
    /// Its target was never approved for this connection: it always asks, reads included.
    pub novel: bool,
    /// Read with certainty, no tripwire, and the risk head calls it clearly benign ([`clearly_benign`]).
    pub benign: bool,
}

impl Evidence {
    /// The user's answers dominate the class (or the user unlocked it): the profile's own threshold applies.
    pub fn warm(&self) -> bool {
        self.unlocked_by_hand || self.class_examples >= FULL_WEIGHT_EXAMPLES
    }
}

/// What Autopilot would do in Auto mode, the class gates aside (spec §5.4, from the first request):
///
/// - approve a target this connection had approved before, when the approve probability reaches [`approve_bar`] and
///   the act head does not ask to escalate; while the class is not warm, only what the risk head calls clearly
///   benign. A target never approved always asks once, whatever it is (the user is never surprised by a first time:
///   an agent reading their email for the first time asks);
/// - else deny at the profile's deny threshold;
/// - else ask.
pub fn verdict(p: Probs, escalate: f32, thresholds: (f32, f32), e: Evidence) -> Verdict {
    let bar = if e.unlocked_by_hand {
        thresholds.0
    } else {
        approve_bar(e.class_examples, thresholds.0)
    };
    if p.approve >= bar && escalate < ESCALATE_LIMIT && !e.novel && (e.warm() || e.benign) {
        Verdict::Approve
    } else if p.deny >= thresholds.1 {
        Verdict::Deny
    } else {
        Verdict::Ask
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(approve: f32, deny: f32) -> Probs {
        Probs {
            approve,
            deny,
            ask: 1.0 - approve - deny,
        }
    }

    #[test]
    fn ai_text_can_only_lower_approval() {
        // The facts look like an ask; the AI's text makes the full pass look perfectly routine.
        let combined = combine(p(0.40, 0.10), p(0.99, 0.0));
        assert!((combined.approve - 0.40).abs() < 1e-6 && (combined.deny - 0.10).abs() < 1e-6);
        // The AI's text makes it look harmful: deny rises, approve falls.
        let harmful = combine(p(0.97, 0.01), p(0.05, 0.90));
        assert!((harmful.approve - 0.05).abs() < 1e-6 && (harmful.deny - 0.90).abs() < 1e-6);
        assert!(harmful.ask >= 0.0);
    }

    #[test]
    fn ai_text_can_only_raise_risk() {
        let calm = [0.9, 0.08, 0.015, 0.005];
        let alarming = [0.05, 0.05, 0.1, 0.8];
        for (f, u) in [(calm, alarming), (alarming, calm)] {
            let c = risk_combine(f, u);
            assert!((c[3] - 0.8).abs() < 1e-6, "the larger P(catastrophic) either way: {c:?}");
            assert!((c.iter().sum::<f32>() - 1.0).abs() < 1e-5);
        }
        // Per class: P(at least severe) is the larger too, even when neither pass alone has the most of it.
        let c = risk_combine([0.1, 0.1, 0.7, 0.1], [0.1, 0.0, 0.0, 0.9]);
        assert!((c[2] + c[3] - 0.9).abs() < 1e-6 && (c[3] - 0.9).abs() < 1e-6, "{c:?}");
        let same = risk_combine(calm, calm);
        assert!(same.iter().zip(calm).all(|(a, b)| (a - b).abs() < 1e-6), "{same:?}");
    }

    #[test]
    fn verdicts_follow_thresholds_and_novelty() {
        let t = (0.95, 0.90);
        assert_eq!(would(p(0.96, 0.0), false, 0.0, t), Verdict::Approve);
        assert_eq!(would(p(0.94, 0.0), false, 0.0, t), Verdict::Ask);
        assert_eq!(would(p(0.96, 0.0), true, 0.0, t), Verdict::Ask, "a new target always asks");
        assert_eq!(would(p(0.96, 0.0), false, 0.7, t), Verdict::Ask, "the model asked to escalate");
        assert_eq!(would(p(0.0, 0.91), true, 0.0, t), Verdict::Deny, "deny still allowed on a new target");
        assert_eq!(would(p(0.0, 0.89), false, 0.0, t), Verdict::Ask);
    }

    const CALM: [f32; 4] = [0.95, 0.02, 0.02, 0.01];

    fn seen(class_examples: usize, benign: bool) -> Evidence {
        Evidence {
            class_examples,
            unlocked_by_hand: false,
            novel: false,
            benign,
        }
    }

    #[test]
    fn the_bar_starts_cold_and_relaxes_in_a_line_to_the_profiles_own() {
        assert!((approve_bar(0, 0.95) - 0.97).abs() < 1e-6);
        assert!((approve_bar(10, 0.95) - 0.96).abs() < 1e-6, "halfway, as the blend's base weight");
        assert!((approve_bar(20, 0.95) - 0.95).abs() < 1e-6);
        assert!((approve_bar(500, 0.95) - 0.95).abs() < 1e-6);
        assert!((approve_bar(0, 0.98) - 0.98).abs() < 1e-6, "a stricter profile keeps its own");
        assert!((approve_bar(0, 0.90) - 0.97).abs() < 1e-6, "Relaxed starts as strict as the rest");
        let bars: Vec<f32> = (0..=20).map(|n| approve_bar(n, 0.90)).collect();
        assert!(bars.windows(2).all(|w| w[1] < w[0]), "strictly relaxing: {bars:?}");
    }

    #[test]
    fn clearly_benign_needs_a_calm_readable_risk() {
        assert!(clearly_benign(Some(CALM)));
        assert!(!clearly_benign(None), "no risk head, nothing vouches");
        assert!(!clearly_benign(Some([0.85, 0.13, 0.01, 0.01])), "P(benign) under 0.9");
        assert!(!clearly_benign(Some([0.90, 0.04, 0.03, 0.03])), "severe + catastrophic at 0.06");
        assert!(!clearly_benign(Some([f32::NAN, 0.0, 0.0, 0.0])));
        assert!(!clearly_benign(Some([0.95, 0.0, 0.0, 0.0])), "does not sum to one");
        assert!(surely_catastrophic(Some([0.0, 0.0, 0.05, 0.95])));
        assert!(!surely_catastrophic(Some([0.0, 0.0, 0.2, 0.8])) && !surely_catastrophic(None));
    }

    #[test]
    fn cold_auto_approves_a_benign_request_to_a_known_target_and_nothing_else() {
        let t = (0.95, 0.90);
        assert_eq!(verdict(p(0.975, 0.0), 0.0, t, seen(0, true)), Verdict::Approve, "benign, sure, target known");
        assert_eq!(verdict(p(0.965, 0.0), 0.0, t, seen(0, true)), Verdict::Ask, "under the cold bar");
        assert_eq!(verdict(p(0.965, 0.0), 0.0, t, seen(10, true)), Verdict::Approve, "the bar relaxed to 0.96");
        assert_eq!(verdict(p(0.999, 0.0), 0.0, t, seen(0, false)), Verdict::Ask, "not clearly benign: asks");
        assert_eq!(verdict(p(0.999, 0.0), 0.0, t, seen(19, false)), Verdict::Ask, "still cold at 19");
        assert_eq!(verdict(p(0.96, 0.0), 0.0, t, seen(20, false)), Verdict::Approve, "warm: the user's answers rule");
        assert_eq!(verdict(p(0.999, 0.0), 0.7, t, seen(0, true)), Verdict::Ask, "the model asked to escalate");
        let novel = Evidence {
            novel: true,
            ..seen(500, true)
        };
        assert_eq!(verdict(p(0.999, 0.0), 0.0, t, novel), Verdict::Ask, "a target never approved asks, reads too");
        let by_hand = Evidence {
            unlocked_by_hand: true,
            ..seen(0, false)
        };
        assert_eq!(verdict(p(0.951, 0.0), 0.0, t, by_hand), Verdict::Approve, "unlocked by hand: the profile's bar");
        assert_eq!(verdict(p(0.0, 0.91), 0.0, t, novel), Verdict::Deny, "the deny verdict is as before");
        assert_eq!(verdict(p(0.0, 0.89), 0.0, t, seen(0, true)), Verdict::Ask);
    }

    #[test]
    fn weights_move_from_base_to_memory() {
        assert_eq!(weights(0, true), (1.0, 0.0, 0.0));
        let (b, k, a) = weights(20, true);
        assert!((b - 0.2).abs() < 1e-6 && (k - 0.4).abs() < 1e-6 && (a - 0.4).abs() < 1e-6);
        let (b, k, a) = weights(100, false);
        assert!((b - 0.2).abs() < 1e-6 && (k - 0.8).abs() < 1e-6 && a == 0.0);
        let mixed = blend([0.8, 0.1, 0.1], 1.0, None, 20);
        assert!((mixed.approve - 0.96).abs() < 1e-5, "{mixed:?}");
        let sum = mixed.approve + mixed.deny + mixed.ask;
        assert!((sum - 1.0).abs() < 1e-5);
    }

    #[test]
    fn confidence_is_one_minus_normalised_entropy() {
        assert!((p(1.0, 0.0).confidence() - 1.0).abs() < 1e-6);
        let flat = Probs {
            approve: 1.0 / 3.0,
            deny: 1.0 / 3.0,
            ask: 1.0 / 3.0,
        };
        assert!(flat.confidence() < 1e-5);
    }
}
