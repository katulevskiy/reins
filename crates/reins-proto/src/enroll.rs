//! Enrolling a phone as a viewer ([`crate::roster`]) over the server, which relays every message and may lie.
//!
//! Six digits compared by a person are only safe if neither side's keys can be chosen after seeing the other side's
//! contribution: a server that swaps a key could otherwise try a million key pairs offline until the digits match (the
//! plain [`crate::join::join_code`], over one key, has exactly that weakness). So the comparison is a
//! commit-then-reveal over the whole transcript:
//!
//! 1. **Joiner** (`POST /reins/api/joins`, [`crate::join::NewJoin`]): its box key, signing key and [`commitment`]
//!    to a fresh random 32-byte nonce `nj`, without the nonce.
//! 2. **Primary** (`POST /reins/api/joins/<id>/challenge`, [`EnrollChallenge`]): only after it holds the commitment,
//!    a fresh random nonce `np`, with its own signing key, the owner key and the epoch it is at.
//! 3. **Joiner** (`POST /reins/api/joins/<id>/reveal`, [`EnrollReveal`]): only after it holds the challenge, `nj`
//!    (and, when it held the account key before, a [`proof`](make_proof)).
//! 4. **Primary**: checks `nj` against the commitment ([`check_commitment`], constant time). Both phones show
//!    [`Transcript::code`]; the user compares them, and the primary approves (a roster entry listing the viewer, and
//!    an answer signed by its signing key, [`crate::answer::SignedAnswer`] of kind `join`, sealed to the joiner's box
//!    key: sign-then-seal, so the anonymous sealed box is authenticated by the key the code covered).
//!
//! Whoever swaps keys must fix its substitute keys (behind a commitment, or in a challenge) before the other side's
//! nonce is known, so the digits it gets are random: one chance in a million per attempt, and each attempt is a join
//! the user sees. The server enforces the order (one challenge, then one reveal, per join) and every message names
//! the join, the account and the device, so nothing carries over to another join.
//!
//! **Without a tap** (a phone that held the account key before an update): the reveal carries an HMAC-SHA256 (full 32
//! bytes) over [`Transcript::proof_message`], under [`proof_key`] derived from the account key. The transcript includes
//! the primary's fresh nonce and the server's join id, so a proof answers one challenge only; the primary checks it in
//! constant time ([`check_proof`]).

use serde::{Deserialize, Serialize};

use crate::{
    ValidationError,
    canon::{Canon, contexts, decode_payload, encode_payload, hex32, parse_hex32, sha256},
    desktop::decode_key,
    validate::invalid,
};

/// Bytes of each side's nonce.
pub const NONCE_BYTES: usize = 32;
/// Bytes of an enrollment proof (HMAC-SHA256, never truncated).
pub const PROOF_BYTES: usize = 32;

/// Step 2, the primary's contribution. `nonce` is 32 random bytes, base64url.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnrollChallenge {
    pub v: u32,
    pub nonce: String,
    /// The primary's device signing key: what the approval's signature must verify against.
    pub primary_sign_pub: String,
    /// The roster's owner key at `epoch`: what the joiner pins.
    pub owner_pub: String,
    pub epoch: u64,
    /// Unix seconds after which the reveal (and a no-tap proof over this challenge) is refused; at most
    /// [`MAX_CHALLENGE_SECS`] ahead.
    pub expires_at: i64,
}

/// The longest a challenge may stay open (seconds).
pub const MAX_CHALLENGE_SECS: i64 = 300;

/// Step 3, the joiner's nonce (base64url, 32 bytes), and its proof when it enrolls without a tap.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnrollReveal {
    pub v: u32,
    pub nonce: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proof: Option<String>,
}

/// A nonce as it travels; `None` unless exactly [`NONCE_BYTES`].
#[must_use]
pub fn parse_nonce(nonce: &str) -> Option<[u8; NONCE_BYTES]> {
    decode_payload(nonce)?.try_into().ok()
}

#[must_use]
pub fn encode_nonce(nonce: &[u8; NONCE_BYTES]) -> String {
    encode_payload(nonce)
}

impl EnrollChallenge {
    /// The shape, the keys (strict Ed25519) and an expiry within [`MAX_CHALLENGE_SECS`] of `now`.
    pub fn validate(&self, now: i64) -> Result<(), ValidationError> {
        crate::check_version(self.v)?;
        parse_nonce(&self.nonce).ok_or_else(|| invalid("nonce", "must be 32 bytes, base64url"))?;
        for (field, key) in [("primary_sign_pub", &self.primary_sign_pub), ("owner_pub", &self.owner_pub)] {
            let raw = decode_key(key).ok_or_else(|| invalid(field, "must be 32 bytes"))?;
            if !crate::canon::strict_ed25519_key(&raw) {
                return Err(invalid(field, "is a small-order or non-canonical Ed25519 key"));
            }
        }
        if self.epoch == 0 {
            return Err(invalid("epoch", "starts at 1"));
        }
        if self.expires_at <= now || self.expires_at > now.saturating_add(MAX_CHALLENGE_SECS) {
            return Err(invalid("expires_at", format!("must be within {MAX_CHALLENGE_SECS} s from now")));
        }
        Ok(())
    }
}

impl EnrollReveal {
    pub fn validate(&self) -> Result<(), ValidationError> {
        crate::check_version(self.v)?;
        parse_nonce(&self.nonce).ok_or_else(|| invalid("nonce", "must be 32 bytes, base64url"))?;
        if let Some(proof) = &self.proof
            && decode_payload(proof).is_none_or(|p| p.len() != PROOF_BYTES)
        {
            return Err(invalid("proof", "must be 32 bytes, base64url"));
        }
        Ok(())
    }
}

/// Step 1's commitment, hex: SHA-256 over context `reins/enroll-commit/v1`, `account` (string), `device_id` (string),
/// `box_pub` (32 bytes), `sign_pub` (32 bytes), the joiner's nonce (32 bytes). `None` unless both keys are 32 bytes.
#[must_use]
pub fn commitment(account: &str, device_id: &str, box_pub: &str, sign_pub: &str, nonce: &[u8; 32]) -> Option<String> {
    let mut c = Canon::new(contexts::ENROLL_COMMIT);
    c.str(account).str(device_id).key(&decode_key(box_pub)?).key(&decode_key(sign_pub)?).key(nonce);
    Some(hex32(&sha256(&c.finish())))
}

/// Whether the revealed nonce opens `committed` for these keys (constant time in the hash).
#[must_use]
pub fn check_commitment(
    committed: &str,
    account: &str,
    device_id: &str,
    box_pub: &str,
    sign_pub: &str,
    nonce: &[u8; 32],
) -> bool {
    let (Some(want), Some(got)) = (
        parse_hex32(committed),
        commitment(account, device_id, box_pub, sign_pub, nonce).and_then(|h| parse_hex32(&h)),
    ) else {
        return false;
    };
    ct_eq(&want, &got)
}

/// Everything both phones agree on when the codes match.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transcript<'a> {
    pub account: &'a str,
    /// The server's id of the join.
    pub join_id: &'a str,
    /// The joiner's Vaultwarden device id.
    pub device_id: &'a str,
    pub box_pub: &'a str,
    pub sign_pub: &'a str,
    pub primary_sign_pub: &'a str,
    pub owner_pub: &'a str,
    pub epoch: u64,
    /// The challenge's `expires_at`.
    pub expires_at: i64,
    pub joiner_nonce: [u8; 32],
    pub primary_nonce: [u8; 32],
}

impl Transcript<'_> {
    /// The transcript's bytes (no context; each use adds its own): `account`, `join_id`, `device_id` (strings),
    /// `box_pub`, `sign_pub`, `primary_sign_pub`, `owner_pub` (32 bytes each), `epoch` (u64), `expires_at` (i64), the
    /// joiner's then the primary's nonce (32 bytes each).
    fn write(&self, c: &mut Canon) -> Result<(), ValidationError> {
        let key = |field: &'static str, v: &str| decode_key(v).ok_or_else(|| invalid(field, "must be 32 bytes"));
        c.str(self.account)
            .str(self.join_id)
            .str(self.device_id)
            .key(&key("box_pub", self.box_pub)?)
            .key(&key("sign_pub", self.sign_pub)?)
            .key(&key("primary_sign_pub", self.primary_sign_pub)?)
            .key(&key("owner_pub", self.owner_pub)?)
            .u64(self.epoch)
            .i64(self.expires_at)
            .key(&self.joiner_nonce)
            .key(&self.primary_nonce);
        Ok(())
    }

    /// The six digits both phones show ("482 193"): SHA-256 over context `reins/enroll-code/v1` and the transcript.
    pub fn code(&self) -> Result<String, ValidationError> {
        let mut c = Canon::new(contexts::ENROLL_CODE);
        self.write(&mut c)?;
        let d = sha256(&c.finish());
        let n = u64::from_be_bytes([d[0], d[1], d[2], d[3], d[4], d[5], d[6], d[7]]) % 1_000_000;
        Ok(format!("{:03} {:03}", n / 1000, n % 1000))
    }

    /// What a no-tap proof MACs: context `reins/enroll-proof/v1` and the transcript.
    pub fn proof_message(&self) -> Result<Vec<u8>, ValidationError> {
        let mut c = Canon::new(contexts::ENROLL_PROOF);
        self.write(&mut c)?;
        Ok(c.finish())
    }
}

/// HMAC-SHA256 (RFC 2104).
#[must_use]
pub fn hmac_sha256(key: &[u8], msg: &[u8]) -> [u8; 32] {
    const BLOCK: usize = 64;
    let mut block = [0u8; BLOCK];
    if key.len() > BLOCK {
        block[..32].copy_from_slice(&sha256(key));
    } else {
        block[..key.len()].copy_from_slice(key);
    }
    let mut inner = Vec::with_capacity(BLOCK + msg.len());
    inner.extend(block.iter().map(|b| b ^ 0x36));
    inner.extend_from_slice(msg);
    let mut outer = Vec::with_capacity(BLOCK + 32);
    outer.extend(block.iter().map(|b| b ^ 0x5c));
    outer.extend_from_slice(&sha256(&inner));
    sha256(&outer)
}

/// The key a no-tap proof is made with: HMAC-SHA256 keyed by the account key over context `reins/enroll-proof-key/v1`
/// and the account id, so it is used for nothing else.
#[must_use]
pub fn proof_key(account_key: &[u8], account: &str) -> [u8; 32] {
    let mut c = Canon::new(contexts::ENROLL_PROOF_KEY);
    c.str(account);
    hmac_sha256(account_key, &c.finish())
}

/// The proof for `transcript`, base64url (32 bytes).
pub fn make_proof(key: &[u8; 32], transcript: &Transcript<'_>) -> Result<String, ValidationError> {
    Ok(encode_payload(&hmac_sha256(key, &transcript.proof_message()?)))
}

/// Whether `proof` is [`make_proof`]'s for `transcript`, checked at `now`: before the challenge expired, exactly 32
/// bytes, compared in constant time.
#[must_use]
pub fn check_proof(key: &[u8; 32], transcript: &Transcript<'_>, proof: &str, now: i64) -> bool {
    if now >= transcript.expires_at {
        return false;
    }
    let (Some(given), Ok(msg)) = (decode_payload(proof), transcript.proof_message()) else {
        return false;
    };
    given.len() == PROOF_BYTES && ct_eq(&given, &hmac_sha256(key, &msg))
}

/// Equality of two byte strings in time that depends only on their lengths.
#[must_use]
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let diff = a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y));
    std::hint::black_box(diff) == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::encode_key;

    fn transcript(keys: &[String; 4], nj: u8, np: u8) -> Transcript<'_> {
        Transcript {
            account: "acct",
            join_id: "j1",
            device_id: "dev",
            box_pub: &keys[0],
            sign_pub: &keys[1],
            primary_sign_pub: &keys[2],
            owner_pub: &keys[3],
            epoch: 2,
            expires_at: 1_000,
            joiner_nonce: [nj; 32],
            primary_nonce: [np; 32],
        }
    }

    fn keys() -> [String; 4] {
        [1, 2, 3, 4].map(|n| encode_key(&[n; 32]))
    }

    #[test]
    fn hmac_matches_rfc_4231() {
        // Test case 2.
        let mac = hmac_sha256(b"Jefe", b"what do ya want for nothing?");
        assert_eq!(hex32(&mac), "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843");
        // Test case 6: a key longer than the block.
        let mac = hmac_sha256(&[0xaa; 131], b"Test Using Larger Than Block-Size Key - Hash Key First");
        assert_eq!(hex32(&mac), "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54");
    }

    #[test]
    fn a_commitment_opens_only_with_its_nonce_and_keys() {
        let k = keys();
        let c = commitment("acct", "dev", &k[0], &k[1], &[7; 32]).unwrap();
        assert!(check_commitment(&c, "acct", "dev", &k[0], &k[1], &[7; 32]));
        assert!(!check_commitment(&c, "acct", "dev", &k[0], &k[1], &[8; 32]));
        assert!(!check_commitment(&c, "acct", "dev", &k[2], &k[1], &[7; 32]), "a swapped box key");
        assert!(!check_commitment(&c, "acct", "dev", &k[0], &k[2], &[7; 32]), "a swapped signing key");
        assert!(!check_commitment(&c, "acct", "dev2", &k[0], &k[1], &[7; 32]));
        assert!(!check_commitment(&c, "acct2", "dev", &k[0], &k[1], &[7; 32]));
        assert!(!check_commitment("zz", "acct", "dev", &k[0], &k[1], &[7; 32]));
    }

    #[test]
    fn the_code_covers_both_sides_keys_and_nonces() {
        let k = keys();
        let base = transcript(&k, 1, 2).code().unwrap();
        assert_eq!(base.len(), 7);
        let other_keys = [5, 6, 7, 8].map(|n| encode_key(&[n; 32]));
        let mut changed = Vec::new();
        for i in 0..4 {
            let mut kk = k.clone();
            kk[i].clone_from(&other_keys[i]);
            changed.push(transcript(&kk, 1, 2).code().unwrap());
        }
        changed.push(transcript(&k, 9, 2).code().unwrap());
        changed.push(transcript(&k, 1, 9).code().unwrap());
        let mut t = transcript(&k, 1, 2);
        t.join_id = "j2";
        changed.push(t.code().unwrap());
        let mut t = transcript(&k, 1, 2);
        t.epoch = 3;
        changed.push(t.code().unwrap());
        let mut t = transcript(&k, 1, 2);
        t.expires_at += 1;
        changed.push(t.code().unwrap());
        // Six digits collide by chance one time in a million; these inputs are fixed, so the test is deterministic.
        assert!(changed.iter().all(|c| *c != base), "{base} {changed:?}");
    }

    #[test]
    fn a_proof_is_full_length_bound_to_its_transcript_and_not_replayable() {
        let k = keys();
        let key = proof_key(b"the account key", "acct");
        assert_ne!(key, proof_key(b"the account key", "acct2"), "per account");
        let t = transcript(&k, 1, 2);
        let proof = make_proof(&key, &t).unwrap();
        assert_eq!(decode_payload(&proof).unwrap().len(), 32);
        assert!(check_proof(&key, &t, &proof, 900));
        assert!(!check_proof(&key, &t, &proof, 1_000), "not once the challenge expired");
        // Replayed to the next join (a new primary nonce, a new join id), or bound to other keys: refused.
        assert!(!check_proof(&key, &transcript(&k, 1, 3), &proof, 900));
        let mut t2 = transcript(&k, 1, 2);
        t2.join_id = "j2";
        assert!(!check_proof(&key, &t2, &proof, 900));
        let mut swapped = k.clone();
        swapped[1] = encode_key(&[9; 32]);
        assert!(!check_proof(&key, &transcript(&swapped, 1, 2), &proof, 900));
        assert!(!check_proof(&proof_key(b"another key", "acct"), &t, &proof, 900));
        // A truncated proof never verifies, even when its bytes are the right prefix.
        let short = encode_payload(&decode_payload(&proof).unwrap()[..16]);
        assert!(!check_proof(&key, &t, &short, 900));
        let reveal = EnrollReveal {
            v: 1,
            nonce: encode_nonce(&[1; 32]),
            proof: Some(short),
        };
        assert!(reveal.validate().is_err());
    }

    #[test]
    fn messages_are_checked() {
        let k = keys();
        let ch = EnrollChallenge {
            v: 1,
            nonce: encode_nonce(&[1; 32]),
            primary_sign_pub: k[2].clone(),
            owner_pub: k[3].clone(),
            epoch: 1,
            expires_at: 200,
        };
        ch.validate(100).unwrap();
        assert!(ch.validate(200).is_err(), "expired");
        assert!(ch.validate(200 - MAX_CHALLENGE_SECS - 1).is_err(), "too far ahead");
        let weak = EnrollChallenge {
            owner_pub: encode_key(&[0; 32]),
            ..ch.clone()
        };
        assert!(weak.validate(100).is_err(), "a small-order owner key");
        let short = EnrollChallenge {
            nonce: encode_payload(&[1; 16]),
            ..ch
        };
        assert!(short.validate(100).is_err(), "nonces are 32 bytes");
        assert!(ct_eq(b"abc", b"abc") && !ct_eq(b"abc", b"abd") && !ct_eq(b"abc", b"ab"));
    }
}
