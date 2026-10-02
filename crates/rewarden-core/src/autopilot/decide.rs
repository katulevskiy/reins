//! From the model's two passes, the memory and the adapter to probabilities and a verdict (spec §3, §5.3).
//!
//! **Prompt-injection rule.** Everything is computed twice, once from `S_facts` (its pass, its embedding, the memory's
//! facts embeddings, the facts adapter) and once from `S_full`; the approve probability used is the smaller of the two,
//! the deny probability the larger. Text the AI wrote can only move a request toward deny or ask.

use super::adapter::{AdapterVariant, features};
use super::memory::{Knn, MemoryRow, Variant, knn};
use super::types::Verdict;

/// Examples of a class from which memory and adapter outweigh the base model.
pub const FULL_WEIGHT_EXAMPLES: usize = 20;
/// The act head's escalate probability above which Autopilot never approves on its own.
pub const ESCALATE_LIMIT: f32 = 0.5;

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
}

pub fn softmax(logits: [f32; 3]) -> [f32; 3] {
    let max = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let e = logits.map(|l| (l - max).exp());
    let sum: f32 = e.iter().sum();
    e.map(|x| x / sum)
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

/// What Autopilot would do in Auto mode, the class gates aside: approve only a target seen approved before, with
/// the act head not asking to escalate.
pub fn would(p: Probs, novel: bool, escalate: f32, thresholds: (f32, f32)) -> Verdict {
    if p.approve >= thresholds.0 && !novel && escalate < ESCALATE_LIMIT {
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
    fn verdicts_follow_thresholds_and_novelty() {
        let t = (0.95, 0.90);
        assert_eq!(would(p(0.96, 0.0), false, 0.0, t), Verdict::Approve);
        assert_eq!(would(p(0.94, 0.0), false, 0.0, t), Verdict::Ask);
        assert_eq!(would(p(0.96, 0.0), true, 0.0, t), Verdict::Ask, "a new target always asks");
        assert_eq!(would(p(0.96, 0.0), false, 0.7, t), Verdict::Ask, "the model asked to escalate");
        assert_eq!(would(p(0.0, 0.91), true, 0.0, t), Verdict::Deny, "deny still allowed on a new target");
        assert_eq!(would(p(0.0, 0.89), false, 0.0, t), Verdict::Ask);
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
