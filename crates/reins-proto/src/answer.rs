//! Answers signed by the primary: to a relayed request (A4), a pairing (A6) or a join. Once an account has a roster,
//! the server takes an A4 answer only as a [`SignedAnswer`] whose signature verifies against the primary's
//! `sign_pub`, and a desktop verifies the same signature inside the sealed answer it opens (sign-then-seal), chained to
//! the owner key it pinned. A server can then neither forge a "yes" nor move one answer to another request.
//!
//! The answer travels as the exact bytes that were signed (`answer`, base64url): the body the endpoint takes without a
//! roster (a [`crate::relay::RelayResponse`] for A4). Nothing is re-serialized before verifying, so no JSON
//! canonicalization is needed.
//!
//! Signed message ([`crate::canon`], context `reins/answer/v1`): `v` (u32), `account` (string), `kind` (string),
//! `id` (string), `epoch` (u64), SHA-256 of the decoded `answer` (32 bytes).

use serde::{Deserialize, Serialize};

use crate::{
    ValidationError,
    canon::{Canon, contexts, decode_payload, parse_sig, sha256},
    validate::invalid,
};

/// Largest answer (decoded bytes): an A4 body may carry up to 20 MiB of results.
pub const MAX_ANSWER_BYTES: usize = 20 * 1024 * 1024;

/// What is answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnswerKind {
    /// A relayed tool call (A4); `id` is the request id.
    Request,
    /// An AI or computer connecting (A6); `id` is the pairing id.
    Pairing,
    /// A phone asking to join; `id` is the join id.
    Join,
}

impl AnswerKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Request => "request",
            Self::Pairing => "pairing",
            Self::Join => "join",
        }
    }
}

/// An answer and the primary's signature over it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedAnswer {
    pub v: u32,
    /// The roster epoch the primary signed at.
    pub epoch: u64,
    /// The answer's exact bytes, base64url without padding.
    pub answer: String,
    /// Ed25519 by the primary's `sign_pub` at `epoch`, base64url.
    pub sig: String,
}

impl SignedAnswer {
    /// The answer's bytes, after checking the shape.
    pub fn answer_bytes(&self) -> Result<Vec<u8>, ValidationError> {
        crate::check_version(self.v)?;
        let bytes =
            decode_payload(&self.answer).ok_or_else(|| invalid("answer", "must be base64url without padding"))?;
        if bytes.is_empty() || bytes.len() > MAX_ANSWER_BYTES {
            return Err(invalid("answer", format!("must be 1 to {MAX_ANSWER_BYTES} bytes")));
        }
        parse_sig(&self.sig).ok_or_else(|| invalid("sig", "must be 64 bytes, base64url without padding"))?;
        Ok(bytes)
    }
}

/// The message the primary signs to answer item `id` of `kind` in `account` at `epoch` with `answer` (its exact
/// bytes). See the module docs for the layout.
#[must_use]
pub fn answer_message(account: &str, kind: AnswerKind, id: &str, epoch: u64, answer: &[u8]) -> Vec<u8> {
    let mut c = Canon::new(contexts::ANSWER);
    c.u32(crate::PROTOCOL_VERSION).str(account).str(kind.as_str()).str(id).u64(epoch).key(&sha256(answer));
    c.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canon::{encode_payload, encode_sig};

    #[test]
    fn the_message_binds_account_kind_id_epoch_and_answer() {
        let base = answer_message("acct", AnswerKind::Request, "r1", 3, b"{\"v\":1}");
        for other in [
            answer_message("acct2", AnswerKind::Request, "r1", 3, b"{\"v\":1}"),
            answer_message("acct", AnswerKind::Pairing, "r1", 3, b"{\"v\":1}"),
            answer_message("acct", AnswerKind::Request, "r2", 3, b"{\"v\":1}"),
            answer_message("acct", AnswerKind::Request, "r1", 4, b"{\"v\":1}"),
            answer_message("acct", AnswerKind::Request, "r1", 3, b"{\"v\": 1}"),
        ] {
            assert_ne!(other, base);
        }
    }

    #[test]
    fn shape_and_wire_format() {
        let signed = SignedAnswer {
            v: 1,
            epoch: 2,
            answer: encode_payload(br#"{"v":1,"outcome":"denied","reason":null}"#),
            sig: encode_sig(&[1; 64]),
        };
        assert_eq!(signed.answer_bytes().unwrap(), br#"{"v":1,"outcome":"denied","reason":null}"#);
        let v = serde_json::to_value(&signed).unwrap();
        assert_eq!(v["epoch"], 2);
        assert_eq!(serde_json::from_value::<SignedAnswer>(v).unwrap(), signed);
        let empty = SignedAnswer {
            answer: String::new(),
            ..signed.clone()
        };
        assert!(empty.answer_bytes().is_err());
        let bad_sig = SignedAnswer {
            sig: "x".to_owned(),
            ..signed.clone()
        };
        assert!(bad_sig.answer_bytes().is_err());
        let v2 = SignedAnswer {
            v: 2,
            ..signed
        };
        assert!(v2.answer_bytes().is_err());
    }
}
