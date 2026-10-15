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
    /// What a phone's MAC proves when it enrolls without a tap ([`crate::join::enroll_proof_message`]).
    pub const ENROLL_PROOF: &str = "reins/enroll-proof/v1";
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
            contexts::ENROLL_PROOF,
        ];
        let set: std::collections::BTreeSet<_> = all.iter().collect();
        assert_eq!(set.len(), all.len());
        assert!(all.iter().all(|c| c.starts_with("reins/") && c.ends_with("/v1")));
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
