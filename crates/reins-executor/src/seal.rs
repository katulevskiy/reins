//! Sealing a leased credential to its requester: only the holder of the box key the request named can open it, so the
//! server that relays it never sees the token.
//!
//! An ephemeral X25519 agreement with the recipient's key, HKDF-SHA256 (salt: both public keys; info
//! `reins/org-lease/v1`) to an AES-256-GCM key, a random nonce. The sealed form is `base64url(epk ‖ nonce ‖ ct)`.

use ring::aead::{AES_256_GCM, Aad, LessSafeKey, NONCE_LEN, Nonce, UnboundKey};
use ring::agreement::{self, EphemeralPrivateKey, UnparsedPublicKey, X25519};
use ring::hkdf::{HKDF_SHA256, Salt};
use ring::rand::{SecureRandom, SystemRandom};
use zeroize::Zeroizing;

/// The HKDF info of a lease seal.
pub const SEAL_INFO: &[u8] = b"reins/org-lease/v1";

const KEY_LEN: usize = 32;

fn aead_key(shared: &[u8], epk: &[u8], recipient: &[u8]) -> Result<LessSafeKey, String> {
    let mut salt = epk.to_vec();
    salt.extend_from_slice(recipient);
    let prk = Salt::new(HKDF_SHA256, &salt).extract(shared);
    let okm = prk.expand(&[SEAL_INFO], &AES_256_GCM).map_err(|_| "key derivation failed".to_owned())?;
    Ok(LessSafeKey::new(UnboundKey::from(okm)))
}

fn decode_key(b64: &str) -> Result<Vec<u8>, String> {
    let key = data_encoding::BASE64URL_NOPAD
        .decode(b64.trim().as_bytes())
        .map_err(|_| "the box key is not base64url".to_owned())?;
    if key.len() == KEY_LEN {
        Ok(key)
    } else {
        Err("the box key is not an X25519 public key".to_owned())
    }
}

/// Seals `plaintext` to the X25519 public key `recipient_b64` (base64url, no padding).
pub fn seal(recipient_b64: &str, plaintext: &[u8]) -> Result<String, String> {
    let recipient = decode_key(recipient_b64)?;
    let rng = SystemRandom::new();
    let private = EphemeralPrivateKey::generate(&X25519, &rng).map_err(|_| "no randomness".to_owned())?;
    let epk = private.compute_public_key().map_err(|_| "key generation failed".to_owned())?;
    let epk = epk.as_ref().to_vec();
    let key = agreement::agree_ephemeral(private, &UnparsedPublicKey::new(&X25519, &recipient), |shared| {
        aead_key(shared, &epk, &recipient)
    })
    .map_err(|_| "the box key is not usable".to_owned())??;
    let mut nonce = [0u8; NONCE_LEN];
    rng.fill(&mut nonce).map_err(|_| "no randomness".to_owned())?;
    let mut ct = plaintext.to_vec();
    key.seal_in_place_append_tag(Nonce::assume_unique_for_key(nonce), Aad::empty(), &mut ct)
        .map_err(|_| "sealing failed".to_owned())?;
    let mut out = epk;
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    Ok(data_encoding::BASE64URL_NOPAD.encode(&out))
}

/// The requester's side: an ephemeral box key whose public half goes in the request, used once to open the answer.
pub struct Opener {
    private: EphemeralPrivateKey,
    public: Vec<u8>,
}

impl Opener {
    /// A new key pair, and its public key (base64url, no padding) to send.
    pub fn new() -> Result<(Self, String), String> {
        let private =
            EphemeralPrivateKey::generate(&X25519, &SystemRandom::new()).map_err(|_| "no randomness".to_owned())?;
        let public = private.compute_public_key().map_err(|_| "key generation failed".to_owned())?.as_ref().to_vec();
        let b64 = data_encoding::BASE64URL_NOPAD.encode(&public);
        Ok((
            Self {
                private,
                public,
            },
            b64,
        ))
    }

    /// Opens what [`seal`] sealed to this key.
    pub fn open(self, sealed: &str) -> Result<Zeroizing<Vec<u8>>, String> {
        let raw = data_encoding::BASE64URL_NOPAD
            .decode(sealed.trim().as_bytes())
            .map_err(|_| "the sealed credential is not base64url".to_owned())?;
        if raw.len() < KEY_LEN + NONCE_LEN + AES_256_GCM.tag_len() {
            return Err("the sealed credential is too short".to_owned());
        }
        let (epk, rest) = raw.split_at(KEY_LEN);
        let (nonce, ct) = rest.split_at(NONCE_LEN);
        let public = self.public;
        let key = agreement::agree_ephemeral(self.private, &UnparsedPublicKey::new(&X25519, epk), |shared| {
            aead_key(shared, epk, &public)
        })
        .map_err(|_| "the sealed credential is not usable".to_owned())??;
        let nonce = Nonce::try_assume_unique_for_key(nonce).map_err(|_| "bad nonce".to_owned())?;
        let mut buf = Zeroizing::new(ct.to_vec());
        let len = key
            .open_in_place(nonce, Aad::empty(), &mut buf)
            .map_err(|_| "the sealed credential does not open with this key".to_owned())?
            .len();
        buf.truncate(len);
        Ok(buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sealed_opens_only_with_its_key() {
        let (opener, public) = Opener::new().unwrap();
        let (other, _) = Opener::new().unwrap();
        let sealed = seal(&public, b"ghs_token").unwrap();
        assert!(!sealed.contains("ghs_token"));
        // Another key can't open it.
        assert!(other.open(&sealed).is_err());
        // A flipped byte doesn't open.
        let (again, public2) = Opener::new().unwrap();
        let mut raw = data_encoding::BASE64URL_NOPAD.decode(seal(&public2, b"x").unwrap().as_bytes()).unwrap();
        let last = raw.len() - 1;
        raw[last] ^= 1;
        assert!(again.open(&data_encoding::BASE64URL_NOPAD.encode(&raw)).is_err());
        // Its own key does.
        assert_eq!(opener.open(&sealed).unwrap().as_slice(), b"ghs_token");
        // Two seals of the same token differ (ephemeral key, random nonce).
        assert_ne!(seal(&public, b"t").unwrap(), seal(&public, b"t").unwrap());
        assert!(seal("not a key", b"t").is_err());
    }
}
