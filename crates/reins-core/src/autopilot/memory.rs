//! Decision memory (spec §5.2): what the user decided on requests Autopilot evaluated, and the nearest neighbours of
//! a new request among them.
//!
//! Rows are sealed with the data key (`autopilot.memory`). Embeddings are kept as int8 with one scale per vector
//! (1 byte per dimension instead of 4): plenty for cosine similarity and the adapter.

use data_encoding::BASE64;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::types::Verdict;

/// Rows kept per profile; the oldest go first.
pub const MEMORY_CAP: usize = 5_000;
/// Neighbours voting.
pub const K: usize = 8;
/// Temperature of the similarity-weighted vote.
pub const TAU: f32 = 0.1;
/// The weight of a correction ("this should have been denied") against an ordinary decision.
pub const CORRECTION_WEIGHT: f32 = 3.0;

/// An L2-normalised vector, quantized to int8.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Embedding {
    pub scale: f32,
    pub q: Vec<i8>,
}

#[expect(clippy::cast_possible_truncation, reason = "values are clamped to the i8 range first")]
fn quantize_one(x: f32, inv: f32) -> i8 {
    (x * inv).round().clamp(-127.0, 127.0) as i8
}

impl Embedding {
    /// Normalises `v` and quantizes it (a zero vector stays zero).
    pub fn from_vector(v: &[f32]) -> Self {
        let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        if !norm.is_finite() || norm <= f32::EPSILON {
            return Self {
                scale: 0.0,
                q: vec![0; v.len()],
            };
        }
        let max = v.iter().fold(0.0_f32, |m, x| m.max((x / norm).abs()));
        let scale = max / 127.0;
        let inv = if scale > 0.0 {
            1.0 / scale
        } else {
            0.0
        };
        Self {
            scale,
            q: v.iter().map(|x| quantize_one(x / norm, inv)).collect(),
        }
    }

    pub fn dim(&self) -> usize {
        self.q.len()
    }

    /// Back to floats (unit length up to rounding).
    pub fn vector(&self) -> Vec<f32> {
        self.q.iter().map(|x| f32::from(*x) * self.scale).collect()
    }

    /// Cosine similarity with a unit vector of the same dimension (0 when they differ).
    pub fn cosine(&self, unit: &[f32]) -> f32 {
        if unit.len() != self.q.len() {
            return 0.0;
        }
        self.q.iter().zip(unit).map(|(a, b)| f32::from(*a) * b).sum::<f32>() * self.scale
    }
}

impl Serialize for Embedding {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let bytes: Vec<u8> = self.q.iter().map(|x| x.cast_unsigned()).collect();
        (self.scale, BASE64.encode(&bytes)).serialize(s)
    }
}

impl<'de> Deserialize<'de> for Embedding {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let (scale, text): (f32, String) = Deserialize::deserialize(d)?;
        let bytes = BASE64.decode(text.as_bytes()).map_err(serde::de::Error::custom)?;
        Ok(Self {
            scale,
            q: bytes.into_iter().map(u8::cast_signed).collect(),
        })
    }
}

/// L2-normalised copy of `v`.
pub fn normalized(v: &[f32]) -> Vec<f32> {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if !norm.is_finite() || norm <= f32::EPSILON {
        return vec![0.0; v.len()];
    }
    v.iter().map(|x| x / norm).collect()
}

/// Where a remembered decision came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Source {
    /// The user approved or denied the request.
    User,
    /// "This was wrong" in Activity.
    Correction,
}

/// What one evaluation saw: from `S_facts` or from `S_full`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Variant {
    Facts,
    Full,
}

/// One remembered human decision. Never an automatic one.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MemoryRow {
    pub request_id: String,
    pub at: i64,
    pub connection_id: String,
    pub class_key: String,
    /// `Approve` or `Deny`.
    pub label: Verdict,
    pub weight: f32,
    pub source: Source,
    /// What Autopilot would have done (shadow accuracy, spec §5.4).
    pub would: Verdict,
    /// Shown for neighbours.
    pub short_label: String,
    /// `S_facts` (so a new model can learn from it again).
    pub facts: String,
    pub novel: bool,
    /// Keyed hash of (connection, target): neighbours with the same target count first.
    #[serde(default)]
    pub target_key: String,
    /// The model the embeddings come from; rows of another model are not compared.
    pub model_id: String,
    /// Calibrated logits (approve, deny, ask) of `S_facts` and `S_full`.
    pub base_facts: [f32; 3],
    pub base_full: [f32; 3],
    pub e_facts: Embedding,
    pub e_full: Embedding,
}

impl MemoryRow {
    pub fn embedding(&self, v: Variant) -> &Embedding {
        match v {
            Variant::Facts => &self.e_facts,
            Variant::Full => &self.e_full,
        }
    }

    pub fn base(&self, v: Variant) -> [f32; 3] {
        match v {
            Variant::Facts => self.base_facts,
            Variant::Full => self.base_full,
        }
    }

    pub fn approved(&self) -> bool {
        self.label == Verdict::Approve
    }
}

/// The neighbours' vote.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Knn {
    /// Similarity-weighted share of approvals among the neighbours (0.5 with none).
    pub p_approve: f32,
    /// The highest similarity (0 with none).
    pub max_sim: f32,
    /// (row index, similarity), most similar first.
    pub neighbours: Vec<(usize, f32)>,
}

/// How much more a remembered decision about the very same target counts: situations that differ only in their
/// target (one recipient or another) are nearly identical texts, and the target is what the user decided on.
pub const SAME_TARGET_BONUS: f32 = 0.5;

/// The `K` rows most similar to `query` (a unit vector), and their vote. Rows about the same target (`target_key`,
/// when there is one) rank and weigh first. `skip` leaves one row out (training).
pub fn knn(query: &[f32], rows: &[&MemoryRow], variant: Variant, target_key: &str, skip: Option<usize>) -> Knn {
    // (row, cosine, ranking score)
    let mut sims: Vec<(usize, f32, f32)> = rows
        .iter()
        .enumerate()
        .filter(|(i, _)| Some(*i) != skip)
        .map(|(i, r)| {
            let cos = r.embedding(variant).cosine(query);
            let same = !target_key.is_empty() && r.target_key == target_key;
            (
                i,
                cos,
                if same {
                    cos + SAME_TARGET_BONUS
                } else {
                    cos
                },
            )
        })
        .collect();
    sims.sort_by(|a, b| b.2.total_cmp(&a.2).then(a.0.cmp(&b.0)));
    sims.truncate(K);
    let Some(top) = sims.first().map(|s| s.2) else {
        return Knn {
            p_approve: 0.5,
            max_sim: 0.0,
            neighbours: Vec::new(),
        };
    };
    let (mut yes, mut all) = (0.0_f32, 0.0_f32);
    for (i, _, score) in &sims {
        let w = ((score - top) / TAU).exp() * rows[*i].weight;
        all += w;
        if rows[*i].approved() {
            yes += w;
        }
    }
    let sims: Vec<(usize, f32)> = sims.into_iter().map(|(i, cos, _)| (i, cos)).collect();
    Knn {
        p_approve: if all > 0.0 {
            yes / all
        } else {
            0.5
        },
        max_sim: sims[0].1,
        neighbours: sims,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn row(label: Verdict, e: &[f32]) -> MemoryRow {
        MemoryRow {
            request_id: "r".to_owned(),
            at: 0,
            connection_id: "c".to_owned(),
            class_key: "x/y".to_owned(),
            label,
            weight: 1.0,
            source: Source::User,
            would: Verdict::Ask,
            short_label: String::new(),
            facts: String::new(),
            novel: false,
            target_key: String::new(),
            model_id: "m".to_owned(),
            base_facts: [0.0; 3],
            base_full: [0.0; 3],
            e_facts: Embedding::from_vector(e),
            e_full: Embedding::from_vector(e),
        }
    }

    #[test]
    fn embeddings_round_trip_closely() {
        let v = [0.3, -1.2, 0.0, 2.5, 0.01];
        let e = Embedding::from_vector(&v);
        let unit = normalized(&v);
        assert!((e.cosine(&unit) - 1.0).abs() < 0.01);
        let back = e.vector();
        for (a, b) in back.iter().zip(&unit) {
            assert!((a - b).abs() < 0.01);
        }
        let json = serde_json::to_string(&e).unwrap();
        assert_eq!(serde_json::from_str::<Embedding>(&json).unwrap(), e);
        assert!(Embedding::from_vector(&[0.0, 0.0]).cosine(&[1.0, 0.0]).abs() < f32::EPSILON);
        assert!(e.cosine(&[1.0]).abs() < f32::EPSILON, "other dimensions never match");
    }

    #[test]
    fn the_nearest_decide_the_vote() {
        let rows = [
            row(Verdict::Approve, &[1.0, 0.0, 0.0]),
            row(Verdict::Approve, &[0.9, 0.1, 0.0]),
            row(Verdict::Deny, &[0.0, 0.0, 1.0]),
        ];
        let refs: Vec<&MemoryRow> = rows.iter().collect();
        let near_yes = knn(&normalized(&[1.0, 0.05, 0.0]), &refs, Variant::Facts, "", None);
        assert!(near_yes.p_approve > 0.99, "{near_yes:?}");
        assert_eq!(near_yes.neighbours[0].0, 0);
        let near_no = knn(&normalized(&[0.0, 0.1, 1.0]), &refs, Variant::Full, "", None);
        assert!(near_no.p_approve < 0.01, "{near_no:?}");
        let left_out = knn(&normalized(&[1.0, 0.0, 0.0]), &refs, Variant::Facts, "", Some(0));
        assert_eq!(left_out.neighbours[0].0, 1);
        let none = knn(&[1.0, 0.0, 0.0], &[], Variant::Facts, "", None);
        assert!((none.p_approve - 0.5).abs() < f32::EPSILON && none.neighbours.is_empty());
    }

    #[test]
    fn corrections_weigh_more() {
        let mut wrong = row(Verdict::Deny, &[1.0, 0.0]);
        wrong.weight = CORRECTION_WEIGHT;
        let rows = [row(Verdict::Approve, &[1.0, 0.0]), row(Verdict::Approve, &[1.0, 0.0]), wrong];
        let refs: Vec<&MemoryRow> = rows.iter().collect();
        let vote = knn(&[1.0, 0.0], &refs, Variant::Facts, "", None);
        assert!((vote.p_approve - 0.4).abs() < 0.01, "{vote:?}");
    }

    #[test]
    fn the_same_target_counts_first() {
        // Nearly identical texts: the one decision about this very target outweighs the closer ones.
        let mut same = row(Verdict::Approve, &[0.9, 0.44]);
        same.target_key = "t-friend".to_owned();
        let rows = [row(Verdict::Deny, &[1.0, 0.0]), row(Verdict::Deny, &[0.99, 0.1]), same];
        let refs: Vec<&MemoryRow> = rows.iter().collect();
        let query = normalized(&[1.0, 0.02]);
        assert!(knn(&query, &refs, Variant::Facts, "", None).p_approve < 0.5);
        let vote = knn(&query, &refs, Variant::Facts, "t-friend", None);
        assert!(vote.p_approve > 0.95, "{vote:?}");
        assert_eq!(vote.neighbours[0].0, 2);
        assert!(vote.neighbours[0].1 < 1.0, "the cosine itself is reported");
    }
}
