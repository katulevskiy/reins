//! The email send hold (Decisions v2, 01 §5, 08 R10).
//!
//! The Gmail API has no scheduled send, so an approved send waits on the server as a ciphertext only the approval
//! device can open:
//!
//! 1. The phone seals the prepared email to its own key and gives the server the ciphertext and the hold's length
//!    ([`HoldCreate`]). The server picks the release time on its own clock ([`HoldCreated`]).
//! 2. **Undo** before then deletes the ciphertext (the phone, the desktop app, or the agent's `reins_cancel`).
//! 3. At the release time the server wakes the phone ([`crate::pairing::PushKind::Hold`]). The phone claims the hold
//!    ([`HoldClaimed`]): one claim, compare-and-swap, which also deletes the ciphertext. Then it checks the request
//!    again and sends.
//! 4. A phone that wakes more than [`LATE_SECS`] after the release time asks the owner again instead of sending.
//!
//! The server never holds the email in clear and cannot send it: it has no Gmail token and no key to the ciphertext.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::ids::RequestId;

/// The hold the phone applies unless the owner changes it.
pub const DEFAULT_SECS: u32 = 30;
/// The longest hold the owner may set (0 turns the hold off).
pub const MAX_SECS: u32 = 120;
/// How late past the release time a woken phone may still send; later, it asks the owner again (08 R10).
pub const LATE_SECS: i64 = 15 * 60;
/// Largest ciphertext (base64url characters): an email body is at most 1 MiB, sealed and encoded.
pub const MAX_SEALED_CHARS: usize = 2 * 1024 * 1024;
/// Holds one account may have waiting at once.
pub const MAX_PER_ACCOUNT: i64 = 20;
/// Ciphertext one account may have waiting at once (base64url characters).
pub const MAX_BYTES_PER_ACCOUNT: i64 = 8 * 1024 * 1024;
/// How long the server keeps a hold nobody claimed (the phone never woke): then it is gone, never sent.
pub const KEEP_SECS: i64 = 7 * 86_400;
/// Longest hold id.
pub const MAX_ID_LEN: usize = 64;

/// `POST /reins/api/holds`: hold a send.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HoldCreate {
    pub v: u32,
    /// The request the send answers (one hold per request).
    pub request_id: RequestId,
    /// The connection that asked: the only one that may cancel it besides the approval device.
    pub connection_id: String,
    /// The prepared email, sealed to the approval device's own key (base64url). Opaque to the server.
    pub sealed: String,
    /// How long to hold it, 1 to [`MAX_SECS`] seconds.
    pub secs: u32,
}

/// The answer to [`HoldCreate`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HoldCreated {
    pub id: String,
    /// When it may be released (server clock, Unix seconds).
    pub release_at: i64,
    /// The server's clock when it answered.
    pub now: i64,
}

/// One waiting hold, as `GET /reins/api/holds` lists it (never its ciphertext).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HoldItem {
    pub id: String,
    pub request_id: RequestId,
    pub release_at: i64,
}

/// `GET /reins/api/holds`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HoldList {
    pub holds: Vec<HoldItem>,
    pub now: i64,
}

/// The answer to `POST /reins/api/holds/<id>/claim`: the ciphertext, which the server no longer has.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HoldClaimed {
    pub id: String,
    pub request_id: RequestId,
    pub sealed: String,
    pub release_at: i64,
    pub now: i64,
    /// Claimed more than [`LATE_SECS`] after `release_at`: the phone asks again instead of sending.
    pub late: bool,
}

/// Whether a claim at `now` of a hold released at `release_at` is too late to send without asking.
#[must_use]
pub fn is_late(release_at: i64, now: i64) -> bool {
    now.saturating_sub(release_at) > LATE_SECS
}

/// A hold id as the server makes them: a UUID's characters only.
#[must_use]
pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= MAX_ID_LEN && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// What the agent gets for a held send (a connector result): when it goes, and how to stop it.
#[must_use]
pub fn held_result(hold: &str, ticket: &str, send_at: &str, secs: u32) -> Value {
    json!({
        "status": "held",
        "hold": hold,
        "ticket": ticket,
        "send_at": send_at,
        "note": format!("Will send in {secs} s unless undone. Do not send it again; reins_cancel with this ticket stops it."),
    })
}

/// The hold id and the release time (RFC 3339) of a held send's result ([`held_result`]); `None` for any other result.
#[must_use]
pub fn held_of(result: &Value) -> Option<(String, String)> {
    if result.get("status").and_then(Value::as_str) != Some("held") {
        return None;
    }
    let hold = result.get("hold").and_then(Value::as_str).filter(|h| valid_id(h))?;
    let send_at = result.get("send_at").and_then(Value::as_str)?;
    Some((hold.to_owned(), send_at.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn late_is_strictly_past_the_limit() {
        assert!(!is_late(100, 100 + LATE_SECS));
        assert!(is_late(100, 101 + LATE_SECS));
        assert!(!is_late(100, 50), "a clock behind the release is never late");
    }

    #[test]
    fn held_results_round_trip_and_nothing_else_reads_as_one() {
        let r = held_result("h-1", "rt_r1", "2026-10-27T10:00:30Z", 30);
        assert_eq!(held_of(&r), Some(("h-1".to_owned(), "2026-10-27T10:00:30Z".to_owned())));
        assert!(r["note"].as_str().unwrap().contains("in 30 s unless undone"));
        assert_eq!(held_of(&json!({"id": "m1", "thread_id": "t1"})), None);
        assert_eq!(held_of(&json!({"status": "held", "hold": "../x", "send_at": "t"})), None);
    }

    #[test]
    fn ids_are_plain() {
        assert!(valid_id("0b8c2d4e-1f2a-4b3c-8d9e-0a1b2c3d4e5f"));
        assert!(!valid_id(""));
        assert!(!valid_id("a/b"));
        assert!(!valid_id(&"a".repeat(65)));
    }
}
