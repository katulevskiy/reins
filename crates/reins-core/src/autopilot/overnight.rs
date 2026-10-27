//! Overnight's one gate (`docs/autopilot-risk.md` §6). An approval in Overnight is carried out only with a [`Proof`],
//! and a proof is made only by [`gate`], for one request, when every condition holds:
//!
//! - the core read the request with certainty and no tripwire fired ([`Outcome::Read`]);
//! - a verified package declares the risk output, and the model returned a risk vector of four finite probabilities
//!   in 0..1 that sum to 1;
//! - the package's threshold is a finite number in (0, 1], and P(catastrophic) is strictly below it (compared as
//!   `f32`; a NaN, an infinity or equality holds);
//! - Auto's checks other than the floor pass: no floor Overnight keeps, not a just-paired AI, an Approve verdict, the
//!   class unlocked (by Overnight's own numbers, or Auto would approve the request on its own), not paused (Auto's
//!   rate limit and Overnight's valves).
//!
//! Anything else is a [`Hold`], with its reason. The engine refuses an Overnight approval without a proof for that
//! very request (`Engine::ap_conclude`). That gate decides only until Auto's setup is done; after it, Overnight is the
//! matrix's (Decisions v2, 01 §11).
//!
//! Overnight never denies (07 §2): whatever it doesn't run is **parked** for the morning, a Deferred ticket whose window
//! runs [`DEFAULT_EXPIRY_HOURS`] past the morning ([`park_deadline`]), and the owner gets one digest ([`digest`]).

use super::risk::Outcome;
use super::types::{DigestGroup, OvernightDigest, Verdict};

/// How long a parked request waits past the morning, by default (01 §11), and the bounds of the owner's setting.
pub const DEFAULT_EXPIRY_HOURS: u32 = 12;
pub const MIN_EXPIRY_HOURS: u32 = 1;
pub const MAX_EXPIRY_HOURS: u32 = 48;
/// What the agent is told of a park (the ticket's note).
pub const PARKED_NOTE: &str = "Parked until morning";

/// The morning a request parked at `now` waits for (`overnight_until`, when the Overnight has an end still to come,
/// else now: the owner sees it when they open Reins), and its deadline, `expiry_secs` later, within the ticket windows
/// (60 s to 7 days from now).
#[must_use]
pub fn park_deadline(overnight_until: Option<i64>, now: i64, expiry_secs: i64) -> (i64, i64) {
    let morning = overnight_until.filter(|u| *u != i64::MAX && *u > now).unwrap_or(now);
    let max = i64::try_from(reins_proto::ticket::DEFERRED_MAX_SECS).unwrap_or(604_800);
    let deadline = morning.saturating_add(expiry_secs).clamp(now + 60, now.saturating_add(max));
    (morning.min(deadline), deadline)
}

/// One parked request, as the digest groups it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParkedItem {
    pub request_id: String,
    pub connection_id: String,
    pub connection_label: String,
    pub task_id: Option<String>,
    pub task_title: Option<String>,
    pub until: i64,
    pub created_at: i64,
    pub batchable: bool,
}

/// The morning digest of `items` at `now`: grouped by agent and task, oldest first; `None` when nothing waits.
#[must_use]
pub fn digest(items: &[ParkedItem], now: i64) -> Option<OvernightDigest> {
    if items.is_empty() {
        return None;
    }
    let mut sorted: Vec<&ParkedItem> = items.iter().collect();
    sorted.sort_by_key(|i| (i.created_at, i.request_id.clone()));
    let mut groups: Vec<DigestGroup> = Vec::new();
    for i in sorted {
        match groups.iter_mut().find(|g| g.connection_id == i.connection_id && g.task_id == i.task_id) {
            Some(g) => {
                g.request_ids.push(i.request_id.clone());
                g.batchable += u32::from(i.batchable);
            }
            None => groups.push(DigestGroup {
                connection_id: i.connection_id.clone(),
                connection_label: i.connection_label.clone(),
                task_id: i.task_id.clone(),
                task_title: i.task_title.clone(),
                request_ids: vec![i.request_id.clone()],
                batchable: u32::from(i.batchable),
            }),
        }
    }
    let total = u32::try_from(items.len()).unwrap_or(u32::MAX);
    let body = groups
        .iter()
        .map(|g| {
            let who = if g.connection_label.is_empty() {
                "an AI"
            } else {
                g.connection_label.as_str()
            };
            match &g.task_title {
                Some(t) => format!("{} from {who} ({t})", g.request_ids.len()),
                None => format!("{} from {who}", g.request_ids.len()),
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    Some(OvernightDigest {
        at: items.iter().map(|i| i.until).filter(|u| *u > now).min(),
        total,
        title: format!("{total} waiting"),
        body,
        groups,
    })
}

/// That Overnight may approve one request. Its fields are private: only [`gate`] makes one.
#[derive(Debug, PartialEq)]
pub(crate) struct Proof {
    request_id: String,
    p_catastrophic: f32,
}

impl Proof {
    /// The request it was made for: it approves that one only.
    pub(crate) fn request_id(&self) -> &str {
        &self.request_id
    }

    pub(crate) fn p_catastrophic(&self) -> f32 {
        self.p_catastrophic
    }
}

/// Why Overnight does not approve a request.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Hold {
    /// A tripwire fired, or the model rated it catastrophic: it waits, or is denied, as the user chose.
    Catastrophic(String),
    /// Not read with certainty, or no rating Overnight may act on: it waits, whatever the setting.
    Uncertain(String),
    /// Auto's rules leave it to the user (`deny`: Auto itself would deny it).
    Auto {
        why: String,
        deny: bool,
    },
    /// Paused for this AI (Auto's rate limit, Overnight's valves): it waits.
    Paused(&'static str),
}

#[derive(Debug, PartialEq)]
pub(crate) enum Gate {
    Approve(Proof),
    Hold(Hold),
}

/// Auto's checks other than the floor, as the engine worked them out for this request.
#[derive(Clone, Debug)]
pub(crate) struct AutoChecks {
    /// The model's verdict (Auto's thresholds, a new target asks).
    pub verdict: Verdict,
    /// The class lets Autopilot approve / deny on its own (earned, unlocked by hand, or, for approving, Auto would
    /// approve this very request on its own).
    pub may_approve: bool,
    pub may_deny: bool,
    /// A just-paired AI.
    pub young: bool,
    /// A floor Overnight does not weigh.
    pub kept_floor: Option<String>,
    /// Paused: Overnight's valves, else Auto's rate limit (`"unusual volume"`).
    pub paused: Option<&'static str>,
}

/// One request, as Overnight sees it.
#[derive(Clone, Debug)]
pub(crate) struct Request<'a> {
    pub request_id: &'a str,
    /// What the core read of it (`None`: it was not read for risk).
    pub outcome: Option<&'a Outcome>,
    /// The verified package that judged it declares the risk output.
    pub risk_declared: bool,
    /// The model's risk (after the two-pass rule), as it came.
    pub risk: Option<&'a [f32]>,
    /// The package's Overnight threshold.
    pub threshold: Option<f32>,
    /// A floor Overnight weighed: only ever approved by it; Auto's deny leaves it to the user.
    pub weighed_floor: bool,
    /// The first phrase of the context line (what a catastrophic rating names).
    pub why: Option<&'a str>,
    pub auto: AutoChecks,
}

/// Four finite probabilities in 0..1 that sum to 1.
fn risk_vector(risk: Option<&[f32]>) -> Result<[f32; 4], String> {
    let r = risk.ok_or("the risk could not be read")?;
    let p: [f32; 4] = r.try_into().map_err(|_| "the risk could not be read (not four classes)".to_owned())?;
    if !p.iter().all(|x| x.is_finite() && (0.0..=1.0).contains(x)) {
        return Err("the risk could not be read (not a probability)".to_owned());
    }
    if (p.iter().sum::<f32>() - 1.0).abs() >= 1e-3 {
        return Err("the risk could not be read (does not sum to one)".to_owned());
    }
    Ok(p)
}

/// The decision: approve (with its proof) or hold (with its reason). See the module documentation.
pub(crate) fn gate(r: &Request<'_>) -> Gate {
    let hold = |h: Hold| Gate::Hold(h);
    if !r.risk_declared {
        return hold(Hold::Uncertain("no verified model that rates risk".to_owned()));
    }
    if let Some(floor) = &r.auto.kept_floor {
        return hold(Hold::Auto {
            why: format!("Always asks: {floor}"),
            deny: false,
        });
    }
    if r.auto.young {
        return hold(Hold::Auto {
            why: "Always asks: a connection paired less than 10 minutes ago".to_owned(),
            deny: false,
        });
    }
    match r.outcome {
        None => return hold(Hold::Uncertain("the request was not read for risk".to_owned())),
        Some(Outcome::Tripwire(line)) => return hold(Hold::Catastrophic(line.clone())),
        Some(Outcome::Uncertain(why)) => return hold(Hold::Uncertain(why.clone())),
        Some(Outcome::Read) => {}
    }
    let p = match risk_vector(r.risk) {
        Ok(p) => p,
        Err(why) => return hold(Hold::Uncertain(why)),
    };
    let Some(theta) = r.threshold.filter(|t| t.is_finite() && *t > 0.0 && *t <= 1.0) else {
        return hold(Hold::Uncertain("the model has no threshold Overnight may use".to_owned()));
    };
    let p_cat = p[3];
    // Strictly below: equality holds, and so would a NaN (incomparable).
    if p_cat.partial_cmp(&theta) != Some(std::cmp::Ordering::Less) {
        return hold(Hold::Catastrophic(r.why.map_or_else(|| "rated catastrophic".to_owned(), str::to_owned)));
    }
    match r.auto.verdict {
        Verdict::Approve => {}
        Verdict::Deny => {
            return hold(Hold::Auto {
                why: "the model would deny it".to_owned(),
                deny: r.auto.may_deny && !r.weighed_floor,
            });
        }
        Verdict::Ask => {
            return hold(Hold::Auto {
                why: "the model is not sure".to_owned(),
                deny: false,
            });
        }
    }
    if !r.auto.may_approve {
        return hold(Hold::Auto {
            why: "still learning this kind of request".to_owned(),
            deny: false,
        });
    }
    if let Some(why) = r.auto.paused {
        return hold(Hold::Paused(why));
    }
    Gate::Approve(Proof {
        request_id: r.request_id.to_owned(),
        p_catastrophic: p_cat,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parked(id: &str, conn: &str, task: Option<&str>, at: i64, batchable: bool) -> ParkedItem {
        ParkedItem {
            request_id: id.to_owned(),
            connection_id: conn.to_owned(),
            connection_label: if conn == "c1" {
                "Claude Code"
            } else {
                "ChatGPT"
            }
            .to_owned(),
            task_id: task.map(str::to_owned),
            task_title: task.map(|_| "Fix CI".to_owned()),
            until: 10_000,
            created_at: at,
            batchable,
        }
    }

    #[test]
    fn park_window_clamps() {
        // Until a morning 8 h away, 12 h after it.
        assert_eq!(park_deadline(Some(28_800), 0, 43_200), (28_800, 72_000));
        // No end (until turned off), or one already past: from now.
        assert_eq!(park_deadline(Some(i64::MAX), 100, 43_200), (100, 43_300));
        assert_eq!(park_deadline(None, 100, 43_200), (100, 43_300));
        assert_eq!(park_deadline(Some(50), 100, 43_200), (100, 43_300));
        // Never less than a minute, never more than 7 days.
        assert_eq!(park_deadline(None, 0, 0).1, 60);
        assert_eq!(park_deadline(Some(30 * 86_400), 0, 43_200).1, 7 * 86_400);
    }

    #[test]
    fn digest_groups_by_agent_and_task() {
        let items = [
            parked("r1", "c1", Some("t1"), 1, true),
            parked("r2", "c2", None, 2, false),
            parked("r3", "c1", Some("t1"), 3, false),
        ];
        let d = digest(&items, 0).unwrap();
        assert_eq!(d.total, 3);
        assert_eq!(d.title, "3 waiting");
        assert_eq!(d.body, "2 from Claude Code (Fix CI), 1 from ChatGPT");
        assert_eq!(d.groups.len(), 2);
        assert_eq!(d.groups[0].request_ids, vec!["r1", "r3"]);
        assert_eq!(d.groups[0].batchable, 1);
        assert_eq!(d.at, Some(10_000));
        assert_eq!(digest(&items, 20_000).unwrap().at, None, "the morning came: shown when opened");
        assert_eq!(digest(&[], 0), None, "nothing waits, nothing to tell");
    }

    const CALM: [f32; 4] = [0.9, 0.05, 0.04, 0.01];

    fn auto() -> AutoChecks {
        AutoChecks {
            verdict: Verdict::Approve,
            may_approve: true,
            may_deny: true,
            young: false,
            kept_floor: None,
            paused: None,
        }
    }

    fn with<'a>(outcome: &'a Outcome, risk: &'a [f32], threshold: Option<f32>) -> Request<'a> {
        Request {
            request_id: "r1",
            outcome: Some(outcome),
            risk_declared: true,
            risk: Some(risk),
            threshold,
            weighed_floor: true,
            why: Some("deletes inside the project \"build/\""),
            auto: auto(),
        }
    }

    fn held(r: &Request<'_>) -> Hold {
        match gate(r) {
            Gate::Hold(h) => h,
            Gate::Approve(p) => panic!("approved: {p:?} for {r:?}"),
        }
    }

    #[test]
    fn it_approves_only_when_everything_holds() {
        let read = Outcome::Read;
        let Gate::Approve(proof) = gate(&with(&read, &CALM, Some(0.5))) else {
            panic!("a calm, certain, unlocked request");
        };
        assert_eq!(proof.request_id(), "r1");
        assert!((proof.p_catastrophic() - 0.01).abs() < 1e-6);
    }

    #[test]
    fn every_failure_holds() {
        let read = Outcome::Read;
        // the risk vector: NaN, infinity, out of range, not summing to one, the wrong length, missing
        for bad in [
            vec![f32::NAN, 0.0, 0.0, 0.0],
            vec![0.9, 0.05, 0.04, f32::NAN],
            vec![0.9, 0.05, 0.04, f32::INFINITY],
            vec![1.5, -0.5, 0.0, 0.0],
            vec![0.5, 0.1, 0.1, 0.1],
            vec![0.9, 0.05, 0.05],
            vec![0.9, 0.05, 0.04, 0.005, 0.005],
            vec![],
        ] {
            assert!(matches!(held(&with(&read, &bad, Some(0.5))), Hold::Uncertain(_)), "{bad:?}");
        }
        let mut none = with(&read, &CALM, Some(0.5));
        none.risk = None;
        assert!(matches!(held(&none), Hold::Uncertain(_)), "no risk (a runtime that does not read it)");
        // the threshold: missing, NaN, infinite, zero, negative, above one
        for t in [None, Some(f32::NAN), Some(f32::INFINITY), Some(0.0), Some(-0.1), Some(1.5)] {
            assert!(matches!(held(&with(&read, &CALM, t)), Hold::Uncertain(_)), "{t:?}");
        }
        // equality holds, so does anything above
        assert!(matches!(held(&with(&read, &CALM, Some(0.01))), Hold::Catastrophic(_)), "equal");
        assert!(matches!(held(&with(&read, &CALM, Some(0.009))), Hold::Catastrophic(_)), "above");
        // what the core read: a tripwire, uncertain, not read at all
        let trip = Outcome::Tripwire("deletes the home directory".to_owned());
        assert_eq!(held(&with(&trip, &CALM, Some(0.5))), Hold::Catastrophic("deletes the home directory".to_owned()));
        let unsure = Outcome::Uncertain("the command could not be read with certainty".to_owned());
        assert!(matches!(held(&with(&unsure, &CALM, Some(0.5))), Hold::Uncertain(_)));
        let mut unread = with(&read, &CALM, Some(0.5));
        unread.outcome = None;
        assert!(matches!(held(&unread), Hold::Uncertain(_)));
        // a package that does not declare the risk output (or was not verified to)
        let mut undeclared = with(&read, &CALM, Some(0.5));
        undeclared.risk_declared = false;
        assert!(matches!(held(&undeclared), Hold::Uncertain(_)));
        // Auto's checks
        let mut r = with(&read, &CALM, Some(0.5));
        r.auto.verdict = Verdict::Ask;
        assert!(matches!(
            held(&r),
            Hold::Auto {
                deny: false,
                ..
            }
        ));
        r.auto.verdict = Verdict::Deny;
        assert!(
            matches!(
                held(&r),
                Hold::Auto {
                    deny: false,
                    ..
                }
            ),
            "a weighed floor is never denied by the verdict"
        );
        r.weighed_floor = false;
        assert!(
            matches!(
                held(&r),
                Hold::Auto {
                    deny: true,
                    ..
                }
            ),
            "as Auto would deny it"
        );
        let mut r = with(&read, &CALM, Some(0.5));
        r.auto.may_approve = false;
        assert!(matches!(held(&r), Hold::Auto { .. }), "a class still learning");
        let mut r = with(&read, &CALM, Some(0.5));
        r.auto.young = true;
        assert!(matches!(held(&r), Hold::Auto { .. }), "a just-paired AI");
        let mut r = with(&read, &CALM, Some(0.5));
        r.auto.kept_floor = Some("a standing permission is only ever given by you".to_owned());
        assert!(matches!(held(&r), Hold::Auto { .. }), "a floor Overnight keeps");
        let mut r = with(&read, &CALM, Some(0.5));
        r.auto.paused = Some("unusual volume");
        assert_eq!(held(&r), Hold::Paused("unusual volume"));
    }
}
