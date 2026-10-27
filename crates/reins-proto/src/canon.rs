//! The canonical byte encoding that phone roles sign ([`crate::roster`], [`crate::oplog`], [`crate::answer`]).
//!
//! JSON is how these messages travel, never what is signed: two serializers (or two versions of one) may order keys,
//! escape characters or write numbers differently. Every signed message is instead this encoding of its fields, in
//! the order its type documents, written by [`Canon`]:
//!
//! | Value | Bytes |
//! |---|---|
//! | context (first, once) | as a string: what the message is, e.g. `reins/op/v1` |
//! | `u32` | 4 bytes, big-endian |
//! | `u64` | 8 bytes, big-endian |
//! | `i64` | 8 bytes, big-endian two's complement |
//! | `bool` | one byte, `0x00` or `0x01` |
//! | string, bytes | length as `u32`, then the bytes (strings as UTF-8, unnormalized) |
//! | 32-byte key or hash | the 32 bytes, no length |
//! | optional value | `0x00` when absent; `0x01` and the value when present |
//! | list | count as `u32`, then each item |
//! | enum | its JSON name, as a string |
//!
//! Keys travel as base64url without padding and hashes as lowercase hex; both are encoded here as their raw 32 bytes,
//! so a message has one encoding whatever spelling carried it. Large payloads (op bodies, checkpoints, answers) enter
//! as their SHA-256, so a signature covers them without signing megabytes.
//!
//! Domain separation: the context string differs per kind of message and carries its version, and every message also
//! names the account it belongs to, so a signature made for one kind, version or account verifies for no other.
//! Contexts are listed in [`contexts`].

use sha2::{Digest, Sha256};

/// The context strings of every signed or hashed message. Each is used for exactly one kind of message.
pub mod contexts {
    /// A roster entry, signed by the owner key ([`crate::roster::RosterEntry`]).
    pub const ROSTER_ENTRY: &str = "reins/roster-entry/v1";
    /// The hash that chains roster entries (`prev_hash`).
    pub const ROSTER_HASH: &str = "reins/roster-hash/v1";
    /// An op, signed by its author ([`crate::oplog::Op`]).
    pub const OP: &str = "reins/op/v1";
    /// A checkpoint, signed by the primary ([`crate::oplog::Checkpoint`]).
    pub const CHECKPOINT: &str = "reins/checkpoint/v1";
    /// A heartbeat, signed by the primary ([`crate::oplog::Heartbeat`]).
    pub const HEARTBEAT: &str = "reins/heartbeat/v1";
    /// One step of the running hash of applied ops ([`crate::oplog::applied_hash_step`]).
    pub const APPLIED: &str = "reins/applied/v1";
    /// An answer to a request, pairing or join, signed by the primary ([`crate::answer::SignedAnswer`]).
    pub const ANSWER: &str = "reins/answer/v1";
    /// A joining phone's commitment to its nonce ([`crate::enroll::commitment`]).
    pub const ENROLL_COMMIT: &str = "reins/enroll-commit/v1";
    /// The six digits both phones compare when one enrolls ([`crate::enroll::Transcript::code`]).
    pub const ENROLL_CODE: &str = "reins/enroll-code/v1";
    /// What a phone's MAC proves when it enrolls without a tap ([`crate::enroll::Transcript::proof_message`]).
    pub const ENROLL_PROOF: &str = "reins/enroll-proof/v1";
    /// The key that MAC is made with ([`crate::enroll::proof_key`]).
    pub const ENROLL_PROOF_KEY: &str = "reins/enroll-proof-key/v1";
    /// The plaintext of an op's body ([`crate::oplog::OpBody`]), the associated data its seal binds.
    pub const OP_BODY: &str = "reins/op-body/v1";
    /// A viewer's request to become primary, signed by its device key ([`crate::roster::HandoverSignature`]).
    pub const HANDOVER_REQUEST: &str = "reins/handover-request/v1";
    /// A viewer's supported policy versions, independently bound to its exact handover request.
    pub const HANDOVER_POLICY_SUPPORT: &str = "reins/handover-policy-support/v1";
    /// One event of an organization's admin log, chained to the one before ([`crate::org::event_hash`]).
    pub const ORG_EVENT: &str = "reins/org-event/v1";
    /// A Managed request's digest ([`crate::managed::request_digest`]).
    pub const ORG_REQUEST: &str = "reins/org-request/v1";
    /// A reviewer's decision, signed by their enrolled device ([`crate::managed::review_message`]).
    pub const ORG_REVIEW: &str = "reins/org-review/v1";
    /// A pending admin change's digest ([`crate::managed::change_digest`]).
    pub const ORG_CHANGE: &str = "reins/org-change/v1";
    /// A body signed by an org key or an executor ([`crate::managed::envelope_message`]).
    pub const ORG_ENVELOPE: &str = "reins/org-envelope/v1";
    /// An executor's call to the server ([`crate::managed::exec_request_message`]).
    pub const ORG_EXEC: &str = "reins/org-exec/v1";
    /// A member's own policy, signed by their device ([`crate::managed::member_policy_message`]).
    pub const ORG_MEMBER_POLICY: &str = "reins/org-member-policy/v1";
}

/// Writes one canonical message. Start with [`Canon::new`] and its context, add the fields in their documented order,
/// then take [`Canon::finish`].
#[derive(Clone, Debug)]
pub struct Canon {
    out: Vec<u8>,
}

impl Canon {
    /// A message of the kind `context` (one of [`contexts`]).
    #[must_use]
    pub fn new(context: &str) -> Self {
        let mut c = Self {
            out: Vec::with_capacity(256),
        };
        c.str(context);
        c
    }

    pub fn u32(&mut self, v: u32) -> &mut Self {
        self.out.extend_from_slice(&v.to_be_bytes());
        self
    }

    pub fn u64(&mut self, v: u64) -> &mut Self {
        self.out.extend_from_slice(&v.to_be_bytes());
        self
    }

    pub fn i64(&mut self, v: i64) -> &mut Self {
        self.out.extend_from_slice(&v.to_be_bytes());
        self
    }

    pub fn bool(&mut self, v: bool) -> &mut Self {
        self.out.push(u8::from(v));
        self
    }

    /// Length-prefixed bytes. Panics past 4 GiB, which no message comes near (every field is bounded far below).
    pub fn bytes(&mut self, v: &[u8]) -> &mut Self {
        let len = u32::try_from(v.len()).expect("canonical field longer than 4 GiB");
        self.u32(len);
        self.out.extend_from_slice(v);
        self
    }

    pub fn str(&mut self, v: &str) -> &mut Self {
        self.bytes(v.as_bytes())
    }

    /// A 32-byte key or hash, without a length.
    pub fn key(&mut self, v: &[u8; 32]) -> &mut Self {
        self.out.extend_from_slice(v);
        self
    }

    /// An optional 32-byte key or hash.
    pub fn opt_key(&mut self, v: Option<&[u8; 32]>) -> &mut Self {
        match v {
            None => self.bool(false),
            Some(k) => self.bool(true).key(k),
        }
    }

    /// The count of a list; the caller then adds each item.
    pub fn count(&mut self, n: usize) -> &mut Self {
        let n = u32::try_from(n).expect("canonical list longer than 4 billion items");
        self.u32(n)
    }

    #[must_use]
    pub fn finish(self) -> Vec<u8> {
        self.out
    }
}

/// Encodings of the Ed25519 points of small order (orders 1, 2, 4 and 8), with the sign bit cleared, as libsodium
/// lists them; and the non-canonical encodings of 0 and 1 (`p` and `p + 1`).
const SMALL_ORDER: [[u8; 32]; 7] = [
    [0; 32],
    {
        let mut k = [0u8; 32];
        k[0] = 1;
        k
    },
    [
        0x26, 0xe8, 0x95, 0x8f, 0xc2, 0xb2, 0x27, 0xb0, 0x45, 0xc3, 0xf4, 0x89, 0xf2, 0xef, 0x98, 0xf0, 0xd5, 0xdf,
        0xac, 0x05, 0xd3, 0xc6, 0x33, 0x39, 0xb1, 0x38, 0x02, 0x88, 0x6d, 0x53, 0xfc, 0x05,
    ],
    [
        0xc7, 0x17, 0x6a, 0x70, 0x3d, 0x4d, 0xd8, 0x4f, 0xba, 0x3c, 0x0b, 0x76, 0x0d, 0x10, 0x67, 0x0f, 0x2a, 0x20,
        0x53, 0xfa, 0x2c, 0x39, 0xcc, 0xc6, 0x4e, 0xc7, 0xfd, 0x77, 0x92, 0xac, 0x03, 0x7a,
    ],
    {
        let mut k = [0xffu8; 32];
        k[0] = 0xec;
        k[31] = 0x7f;
        k
    },
    {
        let mut k = [0xffu8; 32];
        k[0] = 0xed;
        k[31] = 0x7f;
        k
    },
    {
        let mut k = [0xffu8; 32];
        k[0] = 0xee;
        k[31] = 0x7f;
        k
    },
];

/// Whether `key` is an Ed25519 public key a signature can be trusted under: its `y` coordinate canonical (below
/// 2^255 - 19) and the point not of small order. A small-order key verifies signatures nobody had to make, and verifiers
/// differ on such keys (ring does not refuse them), so every key a roster lists passes this first.
#[must_use]
pub fn strict_ed25519_key(key: &[u8; 32]) -> bool {
    let mut y = *key;
    y[31] &= 0x7f;
    // y >= p = 2^255 - 19: little-endian ed ff .. ff 7f and above.
    let non_canonical = y[31] == 0x7f && y[1..31].iter().all(|b| *b == 0xff) && y[0] >= 0xed;
    !non_canonical && !SMALL_ORDER.contains(&y)
}

/// SHA-256 of `bytes`.
#[must_use]
pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

/// A 32-byte hash as it travels: lowercase hex.
#[must_use]
pub fn hex32(hash: &[u8; 32]) -> String {
    data_encoding::HEXLOWER.encode(hash)
}

/// A hash written by [`hex32`]; `None` unless it is exactly 64 lowercase hex digits.
#[must_use]
pub fn parse_hex32(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != 64 {
        return None;
    }
    data_encoding::HEXLOWER.decode(hex.as_bytes()).ok()?.try_into().ok()
}

/// A 64-byte Ed25519 signature as it travels: base64url without padding. `None` for anything else, including a
/// non-canonical spelling of 64 bytes.
#[must_use]
pub fn parse_sig(sig: &str) -> Option<[u8; 64]> {
    data_encoding::BASE64URL_NOPAD.decode(sig.as_bytes()).ok()?.try_into().ok()
}

/// A 64-byte signature, as [`parse_sig`] reads it.
#[must_use]
pub fn encode_sig(sig: &[u8; 64]) -> String {
    data_encoding::BASE64URL_NOPAD.encode(sig)
}

/// A payload as it travels (op bodies, checkpoints, sealed bundles, answers): base64url without padding.
#[must_use]
pub fn decode_payload(payload: &str) -> Option<Vec<u8>> {
    data_encoding::BASE64URL_NOPAD.decode(payload.as_bytes()).ok()
}

#[must_use]
pub fn encode_payload(bytes: &[u8]) -> String {
    data_encoding::BASE64URL_NOPAD.encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_encoding_is_the_documented_one() {
        let mut c = Canon::new("ctx");
        c.u32(1).u64(2).i64(-1).bool(true).str("é").key(&[7; 32]).opt_key(None).opt_key(Some(&[8; 32])).count(0);
        let mut want = vec![0, 0, 0, 3, b'c', b't', b'x', 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 2];
        want.extend([0xff; 8]);
        want.push(1);
        want.extend([0, 0, 0, 2, 0xc3, 0xa9]);
        want.extend([7; 32]);
        want.push(0);
        want.push(1);
        want.extend([8; 32]);
        want.extend([0, 0, 0, 0]);
        assert_eq!(c.finish(), want);
    }

    #[test]
    fn lengths_keep_adjacent_fields_apart() {
        let a = {
            let mut c = Canon::new("ctx");
            c.str("ab").str("c");
            c.finish()
        };
        let b = {
            let mut c = Canon::new("ctx");
            c.str("a").str("bc");
            c.finish()
        };
        assert_ne!(a, b);
        assert_ne!(Canon::new("reins/op/v1").finish(), Canon::new("reins/op/v2").finish());
    }

    #[test]
    fn contexts_are_distinct() {
        let all = [
            contexts::ROSTER_ENTRY,
            contexts::ROSTER_HASH,
            contexts::OP,
            contexts::CHECKPOINT,
            contexts::HEARTBEAT,
            contexts::APPLIED,
            contexts::ANSWER,
            contexts::ENROLL_COMMIT,
            contexts::ENROLL_CODE,
            contexts::ENROLL_PROOF,
            contexts::ENROLL_PROOF_KEY,
            contexts::OP_BODY,
            contexts::HANDOVER_REQUEST,
            contexts::HANDOVER_POLICY_SUPPORT,
        ];
        let set: std::collections::BTreeSet<_> = all.iter().collect();
        assert_eq!(set.len(), all.len());
        assert!(all.iter().all(|c| c.starts_with("reins/") && c.ends_with("/v1")));
    }

    #[test]
    fn small_order_and_non_canonical_keys_are_refused() {
        for bad in SMALL_ORDER {
            assert!(!strict_ed25519_key(&bad));
            let mut signed = bad;
            signed[31] |= 0x80;
            assert!(!strict_ed25519_key(&signed), "either sign");
        }
        let mut above_p = [0xffu8; 32];
        above_p[31] = 0x7f;
        assert!(!strict_ed25519_key(&above_p));
        // RFC 8032 test 1's public key.
        let good = data_encoding::HEXLOWER
            .decode(b"d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a")
            .unwrap();
        assert!(strict_ed25519_key(&good.try_into().unwrap()));
    }

    #[test]
    fn hashes_and_signatures_have_one_spelling() {
        let h = [0xab; 32];
        assert_eq!(parse_hex32(&hex32(&h)), Some(h));
        assert_eq!(parse_hex32(&hex32(&h).to_uppercase()), None);
        assert_eq!(parse_hex32("ab"), None);
        let s = [3; 64];
        assert_eq!(parse_sig(&encode_sig(&s)), Some(s));
        assert_eq!(parse_sig(&encode_payload(&[3; 63])), None);
        assert_eq!(parse_sig("not base64!"), None);
        // The last character carries 2 spare bits: a set spare bit is another spelling, refused.
        let mut other = encode_sig(&s);
        let last = other.pop().unwrap();
        let alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let idx = alphabet.find(last).unwrap();
        other.push(alphabet.as_bytes()[idx | 1] as char);
        if other != encode_sig(&s) {
            assert_eq!(parse_sig(&other), None);
        }
        assert_eq!(decode_payload(&encode_payload(b"x")), Some(b"x".to_vec()));
    }
}
