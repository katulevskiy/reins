//! The trainable, per-profile part (spec §5.3): logistic regression with L2, trained with Adam in plain Rust over
//! `[embedding, base logits (3), neighbour vote (2), class (32, hashed one-hot), novelty (1)]`, one for the facts side
//! and one for the full side; then Platt scaling of the blend on a holdout once there are enough examples.

use data_encoding::BASE64;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::decide::{blend, softmax};
use super::memory::{Knn, MemoryRow, Variant, knn};

/// Hashed one-hot buckets for the class key.
pub const CLASS_BUCKETS: usize = 32;
/// Examples (with both answers among them) before an adapter is trained.
pub const MIN_EXAMPLES: usize = 10;
/// Examples before the blend is recalibrated (Platt scaling on a holdout).
pub const PLATT_MIN: usize = 30;
/// The newest examples trained on.
pub const TRAIN_WINDOW: usize = 1_000;
/// Retrain after this many new examples.
pub const RETRAIN_EVERY: usize = 5;
const STEPS: usize = 300;
const LEARNING_RATE: f32 = 0.05;
const L2: f32 = 1e-3;
const PLATT_RIDGE: f32 = 1e-2;
/// The largest Newton step of the Platt fit.
const PLATT_STEP: f32 = 5.0;

/// FNV-1a: stable across builds and platforms.
fn bucket(class_key: &str) -> usize {
    let mut h: u32 = 0x811c_9dc5;
    for b in class_key.bytes() {
        h ^= u32::from(b);
        h = h.wrapping_mul(0x0100_0193);
    }
    (h as usize) % CLASS_BUCKETS
}

/// The adapter's input for one situation.
pub fn features(e: &[f32], logits: [f32; 3], vote: &Knn, class_key: &str, novel: bool) -> Vec<f32> {
    let mut x = Vec::with_capacity(e.len() + 3 + 2 + CLASS_BUCKETS + 1);
    x.extend_from_slice(e);
    x.extend(logits);
    x.push(vote.p_approve - 0.5);
    x.push(vote.max_sim);
    let mut onehot = [0.0_f32; CLASS_BUCKETS];
    onehot[bucket(class_key)] = 1.0;
    x.extend(onehot);
    x.push(if novel {
        1.0
    } else {
        0.0
    });
    x
}

fn sigmoid(z: f32) -> f32 {
    1.0 / (1.0 + (-z).exp())
}

/// Floats as base64 of their little-endian bytes (compact in the sealed JSON).
mod floats {
    use super::{BASE64, Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &[f32], s: S) -> Result<S::Ok, S::Error> {
        let bytes: Vec<u8> = v.iter().flat_map(|x| x.to_le_bytes()).collect();
        s.serialize_str(&BASE64.encode(&bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<f32>, D::Error> {
        let text = String::deserialize(d)?;
        let bytes = BASE64.decode(text.as_bytes()).map_err(serde::de::Error::custom)?;
        if bytes.len() % 4 != 0 {
            return Err(serde::de::Error::custom("not a list of floats"));
        }
        Ok(bytes.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect())
    }
}

/// P(approve) = σ(w·x + b).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Logistic {
    #[serde(with = "floats")]
    pub w: Vec<f32>,
    pub b: f32,
}

impl Logistic {
    /// 0.5 when `x` does not fit (another model's embedding).
    pub fn predict(&self, x: &[f32]) -> f32 {
        if x.len() != self.w.len() {
            return 0.5;
        }
        sigmoid(self.w.iter().zip(x).map(|(w, x)| w * x).sum::<f32>() + self.b)
    }

    /// Full-batch Adam on the weighted log loss with L2 on the weights. Deterministic.
    pub fn train(xs: &[Vec<f32>], ys: &[f32], weights: &[f32]) -> Self {
        let dim = xs.first().map_or(0, Vec::len);
        let total: f32 = weights.iter().sum::<f32>().max(f32::EPSILON);
        // Parameters: the weights, then the bias.
        let mut theta = vec![0.0_f32; dim + 1];
        let (mut moment1, mut moment2) = (vec![0.0_f32; dim + 1], vec![0.0_f32; dim + 1]);
        let (beta1, beta2, eps) = (0.9_f32, 0.999_f32, 1e-8_f32);
        let mut grad = vec![0.0_f32; dim + 1];
        for step in 1..=STEPS {
            grad.fill(0.0);
            for ((x, y), sample) in xs.iter().zip(ys).zip(weights) {
                let z = theta.iter().zip(x).map(|(c, xi)| c * xi).sum::<f32>() + theta[dim];
                let err = (sigmoid(z) - y) * sample / total;
                for (g, xi) in grad.iter_mut().zip(x) {
                    *g += err * xi;
                }
                grad[dim] += err;
            }
            for (g, c) in grad.iter_mut().zip(&theta[..dim]) {
                *g += L2 * c;
            }
            let round = i32::try_from(step).unwrap_or(i32::MAX);
            let (c1, c2) = (1.0 - beta1.powi(round), 1.0 - beta2.powi(round));
            for i in 0..=dim {
                moment1[i] = beta1 * moment1[i] + (1.0 - beta1) * grad[i];
                moment2[i] = beta2 * moment2[i] + (1.0 - beta2) * grad[i] * grad[i];
                theta[i] -= LEARNING_RATE * (moment1[i] / c1) / ((moment2[i] / c2).sqrt() + eps);
            }
        }
        let b = theta.pop().unwrap_or_default();
        Self {
            w: theta,
            b,
        }
    }
}

/// Recalibration of a probability: σ(a·logit(p) + b).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Platt {
    pub a: f32,
    pub b: f32,
}

fn logit(p: f32) -> f32 {
    let p = p.clamp(1e-4, 1.0 - 1e-4);
    (p / (1.0 - p)).ln()
}

impl Platt {
    pub fn apply(&self, p: f32) -> f32 {
        sigmoid(self.a * logit(p) + self.b)
    }

    /// Weighted maximum likelihood (Newton steps, a small ridge toward the identity); `a` stays positive: the blend
    /// may be sharpened or softened, never inverted.
    pub fn fit(scores: &[f32], ys: &[f32], weights: &[f32]) -> Self {
        let total: f32 = weights.iter().sum::<f32>().max(f32::EPSILON);
        let (mut a, mut b) = (1.0_f32, 0.0_f32);
        for _ in 0..50 {
            let (mut ga, mut gb) = (PLATT_RIDGE * (a - 1.0), PLATT_RIDGE * b);
            let (mut haa, mut hab, mut hbb) = (PLATT_RIDGE, 0.0_f32, PLATT_RIDGE);
            for ((score, label), weight) in scores.iter().zip(ys).zip(weights) {
                let z = logit(*score);
                let p = sigmoid(a * z + b);
                let (err, curve) = ((p - label) * weight / total, p * (1.0 - p) * weight / total);
                ga += err * z;
                gb += err;
                haa += curve * z * z;
                hab += curve * z;
                hbb += curve;
            }
            let det = haa * hbb - hab * hab;
            if det <= f32::EPSILON {
                break;
            }
            let da = ((hbb * ga - hab * gb) / det).clamp(-PLATT_STEP, PLATT_STEP);
            let db = ((haa * gb - hab * ga) / det).clamp(-PLATT_STEP, PLATT_STEP);
            a = (a - da).max(0.05);
            b -= db;
            if da.abs() < 1e-5 && db.abs() < 1e-5 {
                break;
            }
        }
        Self {
            a,
            b,
        }
    }
}

/// One side's adapter.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AdapterVariant {
    pub lr: Logistic,
    pub platt: Option<Platt>,
}

/// A profile's trained adapter, for one model.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Adapter {
    pub model_id: String,
    pub dim: usize,
    pub facts: AdapterVariant,
    pub full: AdapterVariant,
    pub examples: usize,
    pub trained_at: i64,
}

impl Adapter {
    pub fn side(&self, v: Variant) -> &AdapterVariant {
        match v {
            Variant::Facts => &self.facts,
            Variant::Full => &self.full,
        }
    }
}

/// Training examples of one side: features with the row itself left out of its neighbours.
fn examples(rows: &[&MemoryRow], v: Variant) -> Vec<Vec<f32>> {
    let units: Vec<Vec<f32>> = rows.iter().map(|r| r.embedding(v).vector()).collect();
    rows.iter()
        .enumerate()
        .map(|(i, r)| {
            let vote = knn(&units[i], rows, v, &r.target_key, Some(i));
            features(&units[i], r.base(v), &vote, &r.class_key, r.novel)
        })
        .collect()
}

fn class_counts<'a>(rows: &[&'a MemoryRow]) -> std::collections::HashMap<&'a str, usize> {
    let mut counts = std::collections::HashMap::new();
    for r in rows {
        *counts.entry(r.class_key.as_str()).or_insert(0) += 1;
    }
    counts
}

fn train_side(rows: &[&MemoryRow], v: Variant) -> AdapterVariant {
    let xs = examples(rows, v);
    let ys: Vec<f32> = rows
        .iter()
        .map(|r| {
            if r.approved() {
                1.0
            } else {
                0.0
            }
        })
        .collect();
    let ws: Vec<f32> = rows.iter().map(|r| r.weight).collect();
    let lr = Logistic::train(&xs, &ys, &ws);
    // Recalibration on a holdout: the newest fifth, predicted by an adapter trained on the rest (rows are newest
    // first), and blended as an evaluation would blend it.
    let platt = (rows.len() >= PLATT_MIN).then(|| {
        let hold = rows.len() / 5;
        let rest = Logistic::train(&xs[hold..], &ys[hold..], &ws[hold..]);
        let counts = class_counts(&rows[hold..]);
        let scores: Vec<f32> = (0..hold)
            .map(|i| {
                let unit = rows[i].embedding(v).vector();
                let vote = knn(&unit, &rows[hold..], v, &rows[i].target_key, None);
                let q = rest.predict(&features(&unit, rows[i].base(v), &vote, &rows[i].class_key, rows[i].novel));
                let n = counts.get(rows[i].class_key.as_str()).copied().unwrap_or(0);
                let p = blend(softmax(rows[i].base(v)), vote.p_approve, Some(q), n);
                p.approve / (p.approve + p.deny).max(f32::EPSILON)
            })
            .collect();
        Platt::fit(&scores, &ys[..hold], &ws[..hold])
    });
    AdapterVariant {
        lr,
        platt,
    }
}

/// Trains on `rows` (one model's, newest first). `None` while there are too few examples or only one kind of answer.
pub fn train(rows: &[&MemoryRow], model_id: &str, now: i64) -> Option<Adapter> {
    let rows = &rows[..rows.len().min(TRAIN_WINDOW)];
    let dim = rows.first()?.e_facts.dim();
    let rows: Vec<&MemoryRow> =
        rows.iter().copied().filter(|r| r.e_facts.dim() == dim && r.e_full.dim() == dim).collect();
    let approved = rows.iter().filter(|r| r.approved()).count();
    if rows.len() < MIN_EXAMPLES || approved == 0 || approved == rows.len() {
        return None;
    }
    Some(Adapter {
        model_id: model_id.to_owned(),
        dim,
        facts: train_side(&rows, Variant::Facts),
        full: train_side(&rows, Variant::Full),
        examples: rows.len(),
        trained_at: now,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::autopilot::memory::normalized;
    use crate::autopilot::memory::tests::row;
    use crate::autopilot::types::Verdict;

    #[test]
    fn logistic_regression_learns_a_separable_pattern() {
        // Approve when the first coordinate is positive, deny otherwise; the rest is noise.
        let mut xs = Vec::new();
        let mut ys = Vec::new();
        for i in 0..40 {
            let s = if i % 2 == 0 {
                1.0
            } else {
                -1.0
            };
            let noise = f32::from(u8::try_from((i * 37) % 11).unwrap()) / 11.0 - 0.5;
            xs.push(vec![s * (0.5 + noise.abs()), noise, -noise]);
            ys.push(if s > 0.0 {
                1.0
            } else {
                0.0
            });
        }
        let lr = Logistic::train(&xs, &ys, &vec![1.0; xs.len()]);
        assert!(lr.predict(&[0.8, 0.2, 0.0]) > 0.9);
        assert!(lr.predict(&[-0.8, 0.2, 0.0]) < 0.1);
        assert!((lr.predict(&[1.0]) - 0.5).abs() < f32::EPSILON, "a wrong size is no opinion");
        let json = serde_json::to_string(&lr).unwrap();
        assert_eq!(serde_json::from_str::<Logistic>(&json).unwrap(), lr);
    }

    #[test]
    fn platt_scaling_fixes_an_overconfident_score() {
        // Scores of 0.99 that are right only 70% of the time.
        let scores = vec![0.99; 100];
        let ys: Vec<f32> = (0..100)
            .map(|i| {
                if i % 10 < 7 {
                    1.0
                } else {
                    0.0
                }
            })
            .collect();
        let platt = Platt::fit(&scores, &ys, &[1.0; 100]);
        assert!((platt.apply(0.99) - 0.7).abs() < 0.05, "{platt:?} -> {}", platt.apply(0.99));
    }

    #[test]
    fn the_adapter_separates_what_the_user_approves_from_what_they_deny() {
        let mut rows = Vec::new();
        for i in 0..40_u8 {
            let approve = i % 2 == 0;
            let jitter = f32::from(i % 7) / 50.0;
            let e = if approve {
                [1.0, jitter, 0.1, 0.0]
            } else {
                [0.0, jitter, 0.1, 1.0]
            };
            let mut r = row(
                if approve {
                    Verdict::Approve
                } else {
                    Verdict::Deny
                },
                &e,
            );
            r.at = i64::from(i);
            r.class_key = "github/write/push".to_owned();
            rows.push(r);
        }
        rows.reverse();
        let refs: Vec<&MemoryRow> = rows.iter().collect();
        let adapter = train(&refs, "m", 1).expect("trained");
        assert_eq!((adapter.examples, adapter.dim), (40, 4));
        assert!(adapter.facts.platt.is_some(), "recalibrated from 30 examples on");
        let score = |e: [f32; 4]| {
            let unit = normalized(&e);
            let vote = knn(&unit, &refs, Variant::Facts, "", None);
            adapter.facts.lr.predict(&features(&unit, [0.0; 3], &vote, "github/write/push", false))
        };
        assert!(score([1.0, 0.05, 0.1, 0.0]) > 0.8);
        assert!(score([0.0, 0.05, 0.1, 1.0]) < 0.2);
        // Too few examples, or only approvals: nothing to learn.
        assert!(train(&refs[..5], "m", 1).is_none());
        let yes: Vec<&MemoryRow> = refs.iter().copied().filter(|r| r.approved()).collect();
        assert!(train(&yes, "m", 1).is_none());
    }
}
