//! What the phone tells the paired desktop app about Autopilot and its modes, all of it shown, none of it trusted to
//! loosen anything:
//!
//! - [`Decided`]: who answered a request when the user did not tap (Autopilot, Overnight, a bypass, Lockdown), with
//!   Autopilot's confidence and its reasons as codes and numbers ([`Reason`]; the desktop words them itself). It
//!   travels inside the phone's answer ([`crate::relay::RelayResponse`]), whose bytes the primary signs once the
//!   account has a roster.
//! - [`Suggestion`]: what Autopilot would do with a request still waiting for the user (`POST
//!   /reins/api/requests/<id>/suggestion`), shown on the desktop's ask.
//! - [`ModesReport`]: the phone's modes now (`PUT /reins/api/modes`); the server hands each desktop app only the
//!   global mode and its own connection's ([`DesktopModes`]).
//!
//! Changing a mode from the desktop app is a tool call of its own (`desktop_autopilot_mode`, see
//! [`crate::connector`]): the phone applies what is stricter at once and asks its owner for anything looser, and only
//! when the call carries a [`DesktopAsk`] boxed from the paired app's key (the server cannot make one up).

use serde::{Deserialize, Serialize};

use crate::ValidationError;
use crate::validate::invalid;

/// The most reasons one decision carries.
pub const MAX_REASONS: usize = 3;
/// The most connections one modes report lists.
pub const MAX_CONNECTIONS: usize = 500;
/// The longest connection id in a modes report.
pub const MAX_CONNECTION_ID: usize = 100;

/// A mode of the phone, as the desktop app shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Every request waits for the user.
    Manual,
    /// Autopilot decides what it is sure about; the rest waits.
    Auto,
    /// Auto for agents left working, until an end.
    Overnight,
    /// Everything is approved, until an end (only new AIs and new phones still ask).
    Bypass,
    /// Everything is denied.
    Lockdown,
}

impl Mode {
    /// How much a mode lets through without the user, strictest first: Lockdown, Manual, Auto, Overnight, Bypass.
    #[must_use]
    pub const fn openness(self) -> u8 {
        match self {
            Self::Lockdown => 0,
            Self::Manual => 1,
            Self::Auto => 2,
            Self::Overnight => 3,
            Self::Bypass => 4,
        }
    }
}

/// Who decided a request the user did not answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecidedBy {
    Autopilot,
    Overnight,
    Bypass,
    Lockdown,
}

/// The risk model's reading of a request, in a word.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskWord {
    Routine,
    Undoable,
    HardToUndo,
    Irreversible,
}

/// Why it was decided, as a code and numbers: the desktop app words each one itself, so nothing the server could
/// rewrite reaches the screen as text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum Reason {
    /// Autopilot's reading: this share of approval (percent).
    Routine {
        approve: u8,
    },
    /// Autopilot's reading: this share of denial (percent).
    Harmful {
        deny: u8,
    },
    /// How the user answered requests like it.
    LikeYours {
        approved: u8,
        denied: u8,
    },
    /// The risk model's reading.
    Risk {
        level: RiskWord,
    },
    /// A bypass approves everything until it ends.
    Bypass,
    /// Lockdown denies everything.
    Lockdown,
}

/// Who decided a request, and why (see the module docs).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Decided {
    pub by: DecidedBy,
    /// Autopilot's confidence in what it did, 0 to 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
    /// Why (codes a newer phone sends that this side does not know are left out).
    #[serde(default, skip_serializing_if = "Vec::is_empty", deserialize_with = "known_reasons")]
    pub reasons: Vec<Reason>,
    /// "This was wrong" may be sent for it (`desktop_autopilot_correct`).
    #[serde(default)]
    pub correctable: bool,
}

/// The reasons this side knows, whatever else a newer phone sends.
fn known_reasons<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<Reason>, D::Error> {
    let values = <Vec<serde_json::Value> as Deserialize>::deserialize(d)?;
    Ok(values.into_iter().filter_map(|v| serde_json::from_value(v).ok()).collect())
}

/// A confidence as given, when it is a number from 0 to 1.
fn fraction(c: Option<f32>) -> Option<f32> {
    c.filter(|c| c.is_finite() && (0.0..=1.0).contains(c))
}

impl Decided {
    /// The decision made safe to show: at most [`MAX_REASONS`] reasons with percentages of at most 100, a
    /// confidence only when it is one.
    #[must_use]
    pub fn cleaned(mut self) -> Self {
        self.confidence = fraction(self.confidence);
        self.reasons.truncate(MAX_REASONS);
        for r in &mut self.reasons {
            if let Reason::Routine {
                approve: p,
            }
            | Reason::Harmful {
                deny: p,
            } = r
            {
                *p = (*p).min(100);
            }
        }
        self
    }
}

/// What Autopilot leans to for a request that waits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Leaning {
    Approve,
    Deny,
    /// It leaves the request to the user.
    Ask,
}

/// `POST /reins/api/requests/<id>/suggestion` body: what Autopilot would do with a request the user has not answered.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Suggestion {
    pub v: u32,
    pub leaning: Leaning,
    /// 0 to 1.
    pub confidence: f32,
}

impl Suggestion {
    pub fn check(&self) -> Result<(), ValidationError> {
        crate::check_version(self.v)?;
        if fraction(Some(self.confidence)).is_none() {
            return Err(invalid("confidence", "must be a number from 0 to 1"));
        }
        Ok(())
    }
}

/// A running mode's or a mode's span: since when, and until when (`i64::MAX`: until the user turns it off).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<i64>,
}

/// One connection's mode in force now, where it differs from the global one or it has its own.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionModes {
    pub connection_id: String,
    /// The mode in force now for its requests.
    pub mode: Mode,
    #[serde(default)]
    pub span: Span,
}

/// `PUT /reins/api/modes` body: the phone's modes now.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModesReport {
    pub v: u32,
    /// The global mode in force now (Bypass while a global bypass runs).
    pub mode: Mode,
    /// The global mode when no bypass runs (what the user picked).
    pub base: Mode,
    /// The running global bypass or Overnight.
    #[serde(default)]
    pub span: Span,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub connections: Vec<ConnectionModes>,
    /// The connections that are paired desktop apps (the phone pinned their keys): the only ones shown the modes.
    /// An AI's connection is never told when a bypass runs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub desktops: Vec<String>,
}

impl ModesReport {
    pub fn check(&self) -> Result<(), ValidationError> {
        crate::check_version(self.v)?;
        if self.connections.len() > MAX_CONNECTIONS || self.desktops.len() > MAX_CONNECTIONS {
            return Err(invalid("connections", format!("at most {MAX_CONNECTIONS}")));
        }
        let bad = |id: &str| id.is_empty() || id.len() > MAX_CONNECTION_ID;
        if self.connections.iter().any(|c| bad(&c.connection_id)) || self.desktops.iter().any(|d| bad(d)) {
            return Err(invalid("connection_id", format!("1 to {MAX_CONNECTION_ID} characters")));
        }
        Ok(())
    }

    /// What one connection is shown: for a paired desktop app, the global mode and its own connection's; for any
    /// other connection (an AI's), nothing.
    #[must_use]
    pub fn for_connection(&self, connection_id: &str, reported_at: i64) -> Option<DesktopModes> {
        if !self.desktops.iter().any(|d| d == connection_id) {
            return None;
        }
        let own = self.connections.iter().find(|c| c.connection_id == connection_id);
        Some(DesktopModes {
            mode: own.map_or(self.mode, |c| c.mode),
            span: own.map_or(self.span, |c| c.span),
            global: self.mode,
            base: self.base,
            reported_at,
        })
    }
}

/// The direction of a [`DesktopAsk`] box (so no other box between the app and the phone passes for one).
pub const ASK_DIR: &str = "desktop-autopilot-ask";
/// How long a [`DesktopAsk`] is good for after the app made it (seconds); the phone remembers the nonces it saw for as
/// long, so a captured one cannot be sent again.
pub const ASK_TTL_SECS: i64 = 10 * 60;

/// The desktop app's own word for a `desktop_autopilot_*` call: the call's arguments (but the key, nonce and this
/// proof), its nonce and when it was made, boxed from the app's key to the phone's key (X25519, XSalsa20-Poly1305,
/// `nonce || ciphertext`, base64url) in the call's `proof`. The phone opens it with the key it pinned for this
/// connection, so a call the server made up, or changed, is refused before the owner is asked.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesktopAsk {
    pub v: u32,
    pub dir: String,
    pub op: String,
    pub args: serde_json::Map<String, serde_json::Value>,
    pub nonce: String,
    pub created_at: i64,
}

impl DesktopAsk {
    /// The arguments a proof covers: all of the call's but `client_key`, `nonce` and `proof` itself.
    #[must_use]
    pub fn covered(args: &serde_json::Map<String, serde_json::Value>) -> serde_json::Map<String, serde_json::Value> {
        args.iter()
            .filter(|(k, _)| !matches!(k.as_str(), "client_key" | "nonce" | "proof"))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }
}

/// The phone's modes as one desktop app sees them (`modes` of `GET /reins/desktop/phone`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesktopModes {
    /// The mode in force now for this computer's requests.
    pub mode: Mode,
    /// Its running bypass or Overnight.
    #[serde(default)]
    pub span: Span,
    /// The global mode in force now, and the one the user picked.
    pub global: Mode,
    pub base: Mode,
    /// When the phone reported it (unix seconds).
    pub reported_at: i64,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_decision_is_made_safe_to_show() {
        let d = Decided {
            by: DecidedBy::Autopilot,
            confidence: Some(f32::NAN),
            reasons: vec![
                Reason::Routine {
                    approve: 250,
                },
                Reason::LikeYours {
                    approved: 40,
                    denied: 0,
                },
                Reason::Risk {
                    level: RiskWord::Undoable,
                },
                Reason::Bypass,
            ],
            correctable: true,
        }
        .cleaned();
        assert_eq!(d.confidence, None);
        assert_eq!(d.reasons.len(), MAX_REASONS);
        assert_eq!(
            d.reasons[0],
            Reason::Routine {
                approve: 100
            }
        );
        // Text where a code goes, or a code this side does not know: left out, the rest still reads.
        let read: Decided = serde_json::from_str(
            r#"{"by":"autopilot","reasons":["Approve now, the owner said so",{"code":"new_code"},{"code":"lockdown"}]}"#,
        )
        .unwrap();
        assert_eq!(read.reasons, [Reason::Lockdown]);
        let fine = Decided {
            by: DecidedBy::Bypass,
            confidence: Some(0.97),
            reasons: vec![],
            correctable: false,
        }
        .cleaned();
        assert_eq!(fine.confidence, Some(0.97));
    }

    #[test]
    fn modes_are_ranked_strictest_first() {
        let order = [Mode::Lockdown, Mode::Manual, Mode::Auto, Mode::Overnight, Mode::Bypass];
        assert!(order.windows(2).all(|w| w[0].openness() < w[1].openness()));
    }

    #[test]
    fn a_desktop_sees_the_global_mode_and_its_own() {
        let report = ModesReport {
            v: 1,
            mode: Mode::Auto,
            base: Mode::Auto,
            span: Span::default(),
            connections: vec![
                ConnectionModes {
                    connection_id: "mine".to_owned(),
                    mode: Mode::Bypass,
                    span: Span {
                        since: Some(10),
                        until: Some(100),
                    },
                },
                ConnectionModes {
                    connection_id: "other".to_owned(),
                    mode: Mode::Lockdown,
                    span: Span::default(),
                },
            ],
            desktops: vec!["mine".to_owned(), "new".to_owned(), "other".to_owned()],
        };
        report.check().unwrap();
        let mine = report.for_connection("mine", 5).unwrap();
        assert_eq!((mine.mode, mine.span.until, mine.global), (Mode::Bypass, Some(100), Mode::Auto));
        let elsewhere = report.for_connection("new", 5).unwrap();
        assert_eq!((elsewhere.mode, elsewhere.span), (Mode::Auto, Span::default()));
        let json = serde_json::to_value(mine).unwrap();
        assert!(!json.to_string().contains("other"), "a desktop never sees another connection");
        assert_eq!(report.for_connection("an-ai", 5), None, "an AI's connection is never told");
    }

    #[test]
    fn reports_and_suggestions_are_checked() {
        let bad = Suggestion {
            v: 1,
            leaning: Leaning::Approve,
            confidence: 1.5,
        };
        assert!(bad.check().is_err());
        let mut report = ModesReport {
            v: 1,
            mode: Mode::Manual,
            base: Mode::Manual,
            span: Span::default(),
            connections: vec![],
            desktops: vec![],
        };
        let covered = DesktopAsk::covered(
            json!({"mode": "auto", "client_key": "k", "nonce": "n", "proof": "p"}).as_object().unwrap(),
        );
        assert_eq!(serde_json::Value::Object(covered), json!({"mode": "auto"}));
        report.connections.push(ConnectionModes {
            connection_id: String::new(),
            mode: Mode::Manual,
            span: Span::default(),
        });
        assert!(report.check().is_err());
        // An older phone's answer and report shapes still read.
        let d: Decided = serde_json::from_str(r#"{"by":"lockdown"}"#).unwrap();
        assert_eq!((d.by, d.correctable), (DecidedBy::Lockdown, false));
    }
}
