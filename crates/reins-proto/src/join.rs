//! "Add another phone": a phone signed in to an account whose keys it cannot open asks the account's approval device
//! for the account secret (see `rewarden-core`'s `sso` module), and the server relays the answer without being able to
//! read it.
//!
//! 1. The new phone makes an X25519 key pair and posts [`NewJoin`] (`POST /rewarden/api/joins`): its name and public
//!    key. It shows [`join_code`] of its key.
//! 2. The server parks a [`JoinRequest`] for the account and wakes the approval device (push `join`).
//! 3. The approval device shows "Add <device name>?" with the same [`join_code`], computed from the key it received.
//!    The user compares the codes and approves with biometrics; the phone seals a [`SealedSecret`] to the new
//!    phone's key (crypto_box sealed box) and posts [`JoinAnswer`].
//! 4. The new phone polls [`JoinState`] (`GET /rewarden/api/joins/<id>`), opens the sealed box and checks that it
//!    names this join and that the secret opens the account's keys.
//!
//! A server that swapped the key would show a different code on the approval device, and the user would deny. The
//! sealed secret names the join it answers, so it cannot be replayed to another one.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Longest device name kept (characters).
pub const MAX_DEVICE_NAME_CHARS: usize = 64;
/// Largest sealed answer accepted (bytes of its base64 text).
pub const MAX_SEALED_CHARS: usize = 1024;

/// Step 1, the new phone's request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewJoin {
    pub v: u32,
    pub device_name: String,
    /// X25519 public key, base64url without padding (32 bytes).
    pub public_key: String,
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
