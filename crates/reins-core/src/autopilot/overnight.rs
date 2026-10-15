//! Overnight's one gate (`docs/autopilot-risk.md` §6). An approval in Overnight is carried out only with a [`Proof`],
//! and a proof is made only by [`gate`], for one request, when every condition holds:
//!
//! - the core read the request with certainty and no tripwire fired ([`Outcome::Read`]);
//! - a verified package declares the risk output, and the model returned a risk vector of four finite probabilities
//!   in 0..1 that sum to 1;
//! - the package's threshold is a finite number in (0, 1], and P(catastrophic) is strictly below it (compared as
//!   `f32`; a NaN, an infinity or equality holds);
//! - Auto's checks other than the floor pass: no floor Overnight keeps, not a just-paired AI, an Approve verdict, the
//!   class unlocked, not paused (Auto's rate limit and Overnight's valves).
//!
//! Anything else is a [`Hold`], with its reason. The engine refuses an Overnight approval without a proof for that
//! very request (`Engine::ap_conclude`).

use super::risk::Outcome;
use super::types::Verdict;

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
    /// The class lets Autopilot approve / deny on its own (earned, or unlocked by hand).
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
            why: "Autopilot has not earned this kind of request yet".to_owned(),
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
