use serde::{Deserialize, Serialize};

use crate::ids::PairingId;

/// Shown on the phone when an AI client asks to connect to the account.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairingRequest {
    pub v: u32,
    pub id: PairingId,
    /// Client-declared name (e.g. "ChatGPT"). Untrusted: always shown with `client_host`.
    pub client_name: String,
    /// Host of the OAuth redirect URI (e.g. "chatgpt.com"). Server-verified.
    pub client_host: String,
    /// Three two-digit codes; exactly one equals the code shown in the browser.
    pub choices: [u8; 3],
    /// Unix seconds.
    pub created_at: i64,
    /// The Reins desktop app's public key (base64url, 32 bytes), when the client is that app. The phone shows its
    /// [`crate::desktop::key_fingerprint`] for the user to compare and pins it to the new connection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_key: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairingResponse {
    pub v: u32,
    pub approved: bool,
    /// The code the user tapped; required when `approved`.
    pub chosen_code: Option<u8>,
    /// Label for the new connection; defaults to `client_name`.
    pub label: Option<String>,
}

/// The letters of a pairing code (the RFC 8628 "user code" a computer shows as a QR code): consonants only, so no code
/// spells a word, and none that are easily confused (no vowels, so no `O`/`0` or `I`/`1`).
pub const USER_CODE_ALPHABET: &str = "BCDFGHJKLMNPQRSTVWXZ";
/// Letters in a pairing code: 20^8, about 2.6 * 10^10 codes.
pub const USER_CODE_LEN: usize = 8;

/// A pairing code as people and links write it (`bcdf ghjk`, `BCDF-GHJK`): case, spaces and dashes do not count.
/// `None` when it cannot be one. The result is the form the server shows and expects, `BCDF-GHJK`.
#[must_use]
pub fn normalize_user_code(raw: &str) -> Option<String> {
    let letters: Vec<char> =
        raw.chars().filter(|c| !c.is_whitespace() && *c != '-').map(|c| c.to_ascii_uppercase()).collect();
    if letters.len() != USER_CODE_LEN || !letters.iter().all(|c| USER_CODE_ALPHABET.contains(*c)) {
        return None;
    }
    let (first, second) = letters.split_at(USER_CODE_LEN / 2);
    Some(format!("{}-{}", first.iter().collect::<String>(), second.iter().collect::<String>()))
}

/// A6b `POST /pairings/claim` body: the phone scanned (or opened a link with) the code a computer shows, and asks for
/// the pairing it stands for. The answer is a [`PairingRequest`], answered like any other (A6).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairingClaim {
    pub v: u32,
    pub user_code: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PushKind {
    Req,
    Pair,
    /// The device's push registration was replaced; carries an empty id.
    Replaced,
    /// An upload arrived (the id is the blob's).
    Blob,
    /// Another phone of the account asks for its secret (the id is the join's).
    Join,
    /// The account's op log or roster changed (to its other members): the id is [`PushMessage::sync`]'s, the new head
    /// `seq` and whether to alert. Fetch `GET /reins/api/sync`.
    Sync,
    /// A handover concerns this phone (the id is the handover's): to the primary when a viewer asks to decide, to the
    /// new primary when the roster made it primary (`GET /reins/api/handover-bundle`).
    Handover,
    /// An org review waits for this member (the id is the review's: an org request or an admin change). Fetch
    /// `GET /reins/api/org/reviews`.
    OrgReview,
    /// Something of a Circle concerns this phone (the id is a packet's, an invitation's or a link's): a review for
    /// another account, an answer, an invitation's next step. Fetch `GET /reins/api/circles/*`.
    Circle,
    /// O3: a web passkey this member registered waits for their enrolled phone's confirmation (the id is the
    /// credential's). Fetch `GET /reins/api/org/passkeys`.
    OrgPasskey,
    /// A held email send is due (the id is the hold's): claim it with `POST /reins/api/holds/<id>/claim`, or, when it
    /// is gone (undone), drop it. Silent: an automatic hold never notifies (08 R10).
    Hold,
}

/// FCM data payload. Carries only an id; content is fetched over HTTPS.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushMessage {
    pub t: PushKind,
    pub id: String,
    /// Which account it is for ([`push_profile_tag`] of the receiving user), so a phone with a Personal and a work
    /// profile hands it to the right one. Older servers send none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p: Option<String>,
}

/// The tag a push carries for the account `user_uuid` (16 hex of SHA-256 of a fixed context and the id): stable, and
/// meaningless to anyone without the id.
#[must_use]
pub fn push_profile_tag(user_uuid: &str) -> String {
    let digest = crate::canon::sha256(format!("reins/push-profile/v1\n{user_uuid}").as_bytes());
    crate::canon::hex32(&digest)[..16].to_owned()
}

/// The suffix of a [`PushKind::Sync`] id that asks for an alert (FCM data values are strings, so the flag rides in the
/// id).
const ALERT_SUFFIX: &str = ":alert";

impl PushMessage {
    /// A [`PushKind::Sync`] push for head `seq`: an alert (iOS: mutable content, decrypted by the notification
    /// extension) when an op asked for one ([`crate::oplog::Op::notify`]), else silent. The id is `"<seq>"` or
    /// `"<seq>:alert"`.
    #[must_use]
    pub fn sync(seq: u64, alert: bool) -> Self {
        Self {
            t: PushKind::Sync,
            id: if alert {
                format!("{seq}{ALERT_SUFFIX}")
            } else {
                seq.to_string()
            },
            p: None,
        }
    }

    /// The head `seq` and alert flag of a [`PushKind::Sync`] push; `None` for any other push or a malformed id.
    #[must_use]
    pub fn sync_head(&self) -> Option<(u64, bool)> {
        if self.t != PushKind::Sync {
            return None;
        }
        match self.id.strip_suffix(ALERT_SUFFIX) {
            Some(seq) => seq.parse().ok().map(|s| (s, true)),
            None => self.id.parse().ok().map(|s| (s, false)),
        }
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn pushes_name_their_account_by_a_tag() {
        let tag = push_profile_tag("2c4b2a6e-0000-4000-8000-000000000001");
        assert_eq!(tag.len(), 16);
        assert_eq!(tag, push_profile_tag("2c4b2a6e-0000-4000-8000-000000000001"));
        assert_ne!(tag, push_profile_tag("2c4b2a6e-0000-4000-8000-000000000002"));
        let mut push = PushMessage::sync(3, false);
        assert_eq!(serde_json::to_value(&push).unwrap(), serde_json::json!({"t": "sync", "id": "3"}));
        push.p = Some(tag.clone());
        assert_eq!(serde_json::to_value(&push).unwrap()["p"], serde_json::json!(tag));
        // Older servers' pushes still read.
        let old: PushMessage = serde_json::from_str(r#"{"t":"req","id":"r1"}"#).unwrap();
        assert_eq!(old.p, None);
    }

    use serde_json::json;

    use super::*;

    #[test]
    fn push_wire_format() {
        let p = PushMessage {
            t: PushKind::Req,
            id: "r1".into(),
            p: None,
        };
        assert_eq!(serde_json::to_value(&p).unwrap(), json!({"t": "req", "id": "r1"}));
    }

    #[test]
    fn replaced_push_wire_format() {
        let p = PushMessage {
            t: PushKind::Replaced,
            id: String::new(),
            p: None,
        };
        assert_eq!(serde_json::to_value(&p).unwrap(), json!({"t": "replaced", "id": ""}));
        assert_eq!(serde_json::from_value::<PushMessage>(json!({"t": "replaced", "id": ""})).unwrap(), p);
    }

    #[test]
    fn sync_and_handover_pushes() {
        let silent = PushMessage::sync(42, false);
        assert_eq!(serde_json::to_value(&silent).unwrap(), json!({"t": "sync", "id": "42"}));
        assert_eq!(silent.sync_head(), Some((42, false)));
        let alert = PushMessage::sync(43, true);
        assert_eq!(serde_json::to_value(&alert).unwrap(), json!({"t": "sync", "id": "43:alert"}));
        assert_eq!(alert.sync_head(), Some((43, true)));
        let handover: PushMessage = serde_json::from_value(json!({"t": "handover", "id": "h1"})).unwrap();
        assert_eq!(handover.t, PushKind::Handover);
        let review: PushMessage = serde_json::from_value(json!({"t": "orgreview", "id": "or_1"})).unwrap();
        assert_eq!(review.t, PushKind::OrgReview);
        assert_eq!(handover.sync_head(), None);
        for bad in ["", "x", "1:alertx", ":alert", "-1"] {
            let p = PushMessage {
                t: PushKind::Sync,
                id: bad.to_owned(),
                p: None,
            };
            assert_eq!(p.sync_head(), None, "{bad}");
        }
    }

    #[test]
    fn user_codes_ignore_case_spaces_and_dashes() {
        assert_eq!(normalize_user_code("BCDF-GHJK").as_deref(), Some("BCDF-GHJK"));
        assert_eq!(normalize_user_code(" bcdf ghjk ").as_deref(), Some("BCDF-GHJK"));
        assert_eq!(normalize_user_code("bcdfghjk").as_deref(), Some("BCDF-GHJK"));
        assert_eq!(normalize_user_code("B-C-D-F-G-H-J-K").as_deref(), Some("BCDF-GHJK"));
        for bad in ["", "BCDF-GHJ", "BCDF-GHJKL", "ABCD-EFGH", "BCDF-GHJ1", "BCDF_GHJK", "BCDF-GHJ\u{e9}"] {
            assert_eq!(normalize_user_code(bad), None, "{bad:?}");
        }
        assert_eq!(USER_CODE_ALPHABET.len(), 20);
    }

    #[test]
    fn pairing_round_trips() {
        let req = PairingRequest {
            v: 1,
            id: "p1".into(),
            client_name: "ChatGPT".into(),
            client_host: "chatgpt.com".into(),
            choices: [12, 47, 83],
            created_at: 1,
            client_key: None,
        };
        let v = serde_json::to_value(&req).unwrap();
        assert_eq!(v["choices"], json!([12, 47, 83]));
        assert_eq!(serde_json::from_value::<PairingRequest>(v).unwrap(), req);
    }
}
