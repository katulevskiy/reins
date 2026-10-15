//! "Add another phone": a phone signed in to an account whose keys it cannot open asks the account's approval device
//! for the account secret (see `reins-core`'s `sso` module), and the server relays the answer without being able to
//! read it.
//!
//! 1. The new phone makes an X25519 key pair and posts [`NewJoin`] (`POST /reins/api/joins`): its name and public
//!    key. It shows [`join_code`] of its key.
//! 2. The server parks a [`JoinRequest`] for the account and wakes the approval device (push `join`).
//! 3. The approval device shows "Add <device name>?" with the same [`join_code`], computed from the key it received.
//!    The user compares the codes and approves with biometrics; the phone seals a [`SealedSecret`] to the new
//!    phone's key (crypto_box sealed box) and posts [`JoinAnswer`].
//! 4. The new phone polls [`JoinState`] (`GET /reins/api/joins/<id>`), opens the sealed box and checks that it
//!    names this join and that the secret opens the account's keys.
//!
//! A server that swapped the key would show a different code on the approval device, and the user would deny. The
//! sealed secret names the join it answers, so it cannot be replayed to another one. The limit: [`join_code`] covers
//! one key that its sender picks freely, so a server able to try about a million key pairs offline could find one
//! with the same six digits. Enrollment below does not have that weakness.
//!
//! **Enrolling as a viewer** ([`crate::roster`]): the request also carries the phone's device signing key
//! (`sign_pub`; `public_key` is then its box key) and a commitment to a nonce, and the codes come from the
//! commit-then-reveal exchange of [`crate::enroll`] (`challenge` in [`JoinState`], `reveal` in [`JoinRequest`]),
//! never from [`join_code`]. The primary answers by appending a roster entry that lists the phone as a viewer, and with
//! a signed, sealed answer. The server lists a phone as a member only with the device key it asked with.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Longest device name kept (characters).
pub const MAX_DEVICE_NAME_CHARS: usize = 64;
/// Largest sealed answer accepted (bytes of its base64 text).
pub const MAX_SEALED_CHARS: usize = 1024;
/// Largest sealed answer to an enrollment (bytes of its base64 text): the view key, the roster and the primary's
/// signature ride along.
pub const MAX_ENROLL_SEALED_CHARS: usize = 32 * 1024;

/// Step 1, the new phone's request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewJoin {
    pub v: u32,
    pub device_name: String,
    /// X25519 public key, base64url without padding (32 bytes).
    pub public_key: String,
    /// Enrolling as a viewer: the phone's device signing key (Ed25519, base64url); `public_key` is its box key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sign_pub: Option<String>,
    /// Enrolling as a viewer: [`crate::enroll::commitment`] to the phone's nonce, hex.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commitment: Option<String>,
}

/// What the server answers to [`NewJoin`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JoinCreated {
    pub id: String,
    /// Unix seconds after which the request is gone.
    pub expires_at: i64,
}

/// Step 2, what the approval device sees.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JoinRequest {
    pub v: u32,
    pub id: String,
    pub device_name: String,
    pub public_key: String,
    pub created_at: i64,
    /// The asking phone's Vaultwarden device id: the `device_id` the roster lists it under when it enrolls.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
    /// As in [`NewJoin`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sign_pub: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commitment: Option<String>,
    /// The joiner's nonce (and proof), once it revealed it ([`crate::enroll`] step 3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reveal: Option<crate::enroll::EnrollReveal>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JoinStatus {
    Waiting,
    Approved,
    Denied,
    /// Unknown or past its time.
    Expired,
}

/// Step 4, what the new phone polls.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JoinState {
    pub status: JoinStatus,
    /// The sealed [`SealedSecret`] (base64), with `Approved`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sealed: Option<String>,
    /// Enrolling: the primary's challenge, once posted ([`crate::enroll`] step 2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub challenge: Option<crate::enroll::EnrollChallenge>,
}

/// Step 3, the approval device's answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JoinAnswer {
    pub v: u32,
    pub approve: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sealed: Option<String>,
}

/// The plaintext inside the sealed box.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SealedSecret {
    pub v: u32,
    /// The join this answers.
    pub join_id: String,
    /// The account secret, base64url without padding (32 bytes).
    pub secret: String,
}

impl std::fmt::Debug for SealedSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SealedSecret").field("join_id", &self.join_id).finish_non_exhaustive()
    }
}

/// Six digits both phones show for the new phone's key ("482 193"); `None` for a key that is not 32 bytes.
#[must_use]
pub fn join_code(public_key: &str) -> Option<String> {
    let raw = crate::desktop::decode_key(public_key)?;
    let mut h = Sha256::new();
    h.update(b"reins-join-key/1");
    h.update(raw);
    let d = h.finalize();
    let n = u32::from_be_bytes([0, d[0], d[1], d[2]]) % 1_000_000;
    Some(format!("{:03} {:03}", n / 1000, n % 1000))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::encode_key;

    #[test]
    fn a_join_from_before_roles_still_parses() {
        let b = encode_key(&[3u8; 32]);
        let old: NewJoin =
            serde_json::from_value(serde_json::json!({"v": 1, "device_name": "x", "public_key": b})).unwrap();
        assert_eq!((old.sign_pub, old.commitment), (None, None));
        let state = JoinState {
            status: JoinStatus::Waiting,
            sealed: None,
            challenge: None,
        };
        assert_eq!(serde_json::to_value(&state).unwrap(), serde_json::json!({"status": "waiting"}));
    }

    #[test]
    fn codes_are_six_stable_digits() {
        let key = encode_key(&[3u8; 32]);
        let code = join_code(&key).unwrap();
        assert_eq!(code.len(), 7);
        assert!(code.chars().all(|c| c.is_ascii_digit() || c == ' '));
        assert_eq!(join_code(&key), Some(code.clone()));
        assert_ne!(join_code(&encode_key(&[4u8; 32])), Some(code));
        assert_eq!(join_code("short"), None);
    }

    #[test]
    fn wire_format() {
        let state = JoinState {
            status: JoinStatus::Waiting,
            sealed: None,
            challenge: None,
        };
        assert_eq!(serde_json::to_string(&state).unwrap(), r#"{"status":"waiting"}"#);
        let secret = SealedSecret {
            v: 1,
            join_id: "j1".into(),
            secret: "c2VjcmV0".into(),
        };
        assert!(!format!("{secret:?}").contains("c2VjcmV0"));
    }
}
