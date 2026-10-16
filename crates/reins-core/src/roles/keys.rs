//! The keys of phone roles ([`reins_proto::roster`]):
//!
//! | Key | What | Where |
//! |---|---|---|
//! | device signing key | Ed25519; signs ops, answers, heartbeats, checkpoints and handover bundles | this installation (`reins.sign-key`), never copied |
//! | device box key | X25519; the view key, enrollment answers and handover bundles are sealed to it | this installation (`reins.box-key`), never copied |
//! | owner seed | 32 random bytes; derives the owner signing key, the credential key and the view keys | the primary (`reins.owner-seed`); the recovery wrap on the server |
//! | view key | AES-256-GCM; seals ops and checkpoints | every member (`reins.view-keys`, a keyring by key id) |
//!
//! All of them live in the device store, which is sealed with the data key the platform keystore wraps
//! ([`crate::KeyWrapper`]), and none is in an account snapshot.
//!
//! **Derivations.** Every key the owner seed derives is HMAC-SHA256 keyed by the seed over a context of its own
//! ([`OwnerSeed`]): the owner signing key's Ed25519 seed, the credential key's X25519 secret, and the view key of each
//! key id (`kid`), the roster epoch at which that view key came in (epoch 1 at `create`, a removal's or a recovery's
//! epoch after). So the recovery wrap carries the seed, and with it every view key of that owner's tenure; a removed
//! member, which never held the seed, learns no view key that came in after it left.
//!
//! **Sealed formats.** A view-key seal is `kid` (8 bytes, big-endian) ‖ nonce (12) ‖ AES-256-GCM ciphertext and tag,
//! with the caller's associated data (an op's signed envelope, [`reins_proto::oplog::body_associated_data`]). Box seals
//! are libsodium sealed boxes (`crypto_box`), signed inside first wherever the opener must know who sealed (sign-then-
//! seal).

use std::collections::BTreeMap;
use std::fmt;

use data_encoding::BASE64URL_NOPAD;
use reins_proto::canon::{Canon, strict_ed25519_key};
use reins_proto::desktop::{decode_key, encode_key};
use ring::aead::{AES_256_GCM, Aad, LessSafeKey, NONCE_LEN, Nonce, UnboundKey};
use ring::hmac;
use ring::signature::{ED25519, Ed25519KeyPair, KeyPair, UnparsedPublicKey};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::CoreError;
use crate::crypto::random_bytes;

/// Context strings of what the core derives, signs or seals for phone roles (besides [`reins_proto::canon::contexts`],
/// which they never equal: a test checks).
pub mod contexts {
    pub const OWNER_SIGN_KEY: &str = "reins/owner-sign-key/v1";
    pub const CREDENTIAL_KEY: &str = "reins/credential-key/v1";
    pub const VIEW_KEY: &str = "reins/view-key/v1";
    pub const RECOVERY_WRAP_KEY: &str = "reins/recovery-wrap-key/v1";
    pub const RECOVERY_WRAP: &str = "reins/recovery-wrap/v1";
    pub const HANDOVER_BUNDLE: &str = "reins/handover-bundle/v1";
    pub const VIEW_KEY_GRANT: &str = "reins/view-key-grant/v1";
    pub const CHECKPOINT_BODY: &str = "reins/checkpoint-body/v1";
}

/// Where this installation keeps its keys (store secrets: service, account).
pub(crate) const SIGN_KEY: (&str, &str) = ("reins.sign-key", "this");
pub(crate) const BOX_KEY: (&str, &str) = ("reins.box-key", "this");
/// Per account (account: [`crate::store::Owner::id`]).
pub(crate) const OWNER_SEED: &str = "reins.owner-seed";
pub(crate) const VIEW_KEYS: &str = "reins.view-keys";

/// Ed25519 verification as every phone-roles check does it: the key must be a strict one (canonical, not of small
/// order, [`strict_ed25519_key`]) and the signature must verify under ring's (strict) Ed25519.
pub fn ed25519_verify(public_key: &[u8; 32], message: &[u8], signature: &[u8; 64]) -> bool {
    strict_ed25519_key(public_key) && UnparsedPublicKey::new(&ED25519, public_key).verify(message, signature).is_ok()
}

fn hmac32(key: &[u8], message: &[u8]) -> Zeroizing<[u8; 32]> {
    let mut out = Zeroizing::new([0u8; 32]);
    out.copy_from_slice(hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, key), message).as_ref());
    out
}

/// An Ed25519 key pair made from a 32-byte seed.
pub struct SigningKey {
    pair: Ed25519KeyPair,
    public: [u8; 32],
}

impl fmt::Debug for SigningKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SigningKey").field("public", &encode_key(&self.public)).finish_non_exhaustive()
    }
}

impl SigningKey {
    pub fn from_seed(seed: &[u8; 32]) -> Result<Self, CoreError> {
        let pair = Ed25519KeyPair::from_seed_unchecked(seed).map_err(|_| CoreError::storage("invalid signing key"))?;
        let public: [u8; 32] =
            pair.public_key().as_ref().try_into().map_err(|_| CoreError::storage("invalid signing key"))?;
        // One chance in about 2^250 for a random seed; a key verifiers would refuse is never used.
        if !strict_ed25519_key(&public) {
            return Err(CoreError::storage("invalid signing key"));
        }
        Ok(Self {
            pair,
            public,
        })
    }

    pub fn public(&self) -> [u8; 32] {
        self.public
    }

    /// The public key as rosters carry it (base64url).
    pub fn public_b64(&self) -> String {
        encode_key(&self.public)
    }

    pub fn sign(&self, message: &[u8]) -> [u8; 64] {
        let mut out = [0u8; 64];
        out.copy_from_slice(self.pair.sign(message).as_ref());
        out
    }

    /// [`SigningKey::sign`] as signatures travel (base64url).
    pub fn sign_b64(&self, message: &[u8]) -> String {
        reins_proto::canon::encode_sig(&self.sign(message))
    }
}

/// An X25519 secret key (the device box key, or the credential key).
pub struct BoxKey(crypto_box::SecretKey);

impl fmt::Debug for BoxKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("BoxKey(<redacted>)")
    }
}

impl BoxKey {
    pub fn from_bytes(raw: [u8; 32]) -> Self {
        Self(crypto_box::SecretKey::from(raw))
    }

    pub fn public(&self) -> [u8; 32] {
        *self.0.public_key().as_bytes()
    }

    pub fn public_b64(&self) -> String {
        encode_key(&self.public())
    }

    /// Opens a sealed box made for this key.
    pub fn open(&self, sealed: &[u8]) -> Result<Zeroizing<Vec<u8>>, CoreError> {
        self.0
            .unseal(sealed)
            .map(Zeroizing::new)
            .map_err(|_| CoreError::invalid("a sealed message could not be opened"))
    }
}

/// Seals `plain` to the X25519 public key `to` (base64url): an anonymous sealed box.
pub fn seal_to(to: &str, plain: &[u8]) -> Result<Vec<u8>, CoreError> {
    let key = decode_key(to).ok_or_else(|| CoreError::invalid("not an X25519 public key"))?;
    crypto_box::PublicKey::from(key)
        .seal(&mut crypto_box::aead::OsRng, plain)
        .map_err(|_| CoreError::storage("could not seal"))
}

/// The 32 random bytes that make an account's owner key, credential key and view keys.
pub struct OwnerSeed(Zeroizing<[u8; 32]>);

impl fmt::Debug for OwnerSeed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("OwnerSeed(<redacted>)")
    }
}

impl OwnerSeed {
    pub fn generate() -> Result<Self, CoreError> {
        Ok(Self(Zeroizing::new(random_bytes::<32>()?)))
    }

    pub fn from_bytes(raw: &[u8]) -> Result<Self, CoreError> {
        <[u8; 32]>::try_from(raw).map(|b| Self(Zeroizing::new(b))).map_err(|_| CoreError::invalid("not an owner seed"))
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    fn derive(&self, canon: Canon) -> Zeroizing<[u8; 32]> {
        hmac32(&self.0[..], &canon.finish())
    }

    /// The owner key, which signs the roster.
    pub fn owner_key(&self) -> Result<SigningKey, CoreError> {
        SigningKey::from_seed(&self.derive(Canon::new(contexts::OWNER_SIGN_KEY)))
    }

    /// The credential key, which tokens and proposals' payloads are sealed to.
    pub fn credential_key(&self) -> BoxKey {
        BoxKey::from_bytes(*self.derive(Canon::new(contexts::CREDENTIAL_KEY)))
    }

    /// The view key that came in at roster epoch `kid` of `account`.
    pub fn view_key(&self, account: &str, kid: u64) -> ViewKey {
        let mut c = Canon::new(contexts::VIEW_KEY);
        c.str(account).u64(kid);
        ViewKey {
            kid,
            key: self.derive(c),
        }
    }
}

/// One view key and its id (the roster epoch it came in at).
#[derive(Clone)]
pub struct ViewKey {
    pub kid: u64,
    key: Zeroizing<[u8; 32]>,
}

impl fmt::Debug for ViewKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ViewKey").field("kid", &self.kid).finish_non_exhaustive()
    }
}

/// Bytes before the ciphertext of a view-key seal: the key id and the nonce.
const VIEW_HEADER: usize = 8 + NONCE_LEN;

impl ViewKey {
    pub fn from_bytes(kid: u64, raw: &[u8]) -> Result<Self, CoreError> {
        let key = <[u8; 32]>::try_from(raw).map_err(|_| CoreError::invalid("not a view key"))?;
        Ok(Self {
            kid,
            key: Zeroizing::new(key),
        })
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.key
    }

    fn aead(&self) -> Result<LessSafeKey, CoreError> {
        UnboundKey::new(&AES_256_GCM, &self.key[..])
            .map(LessSafeKey::new)
            .map_err(|_| CoreError::storage("invalid view key"))
    }

    /// `kid ‖ nonce ‖ AES-256-GCM(plain, aad)`.
    pub fn seal(&self, aad: &[u8], plain: &[u8]) -> Result<Vec<u8>, CoreError> {
        let nonce = random_bytes::<NONCE_LEN>()?;
        let mut buf = plain.to_vec();
        self.aead()?
            .seal_in_place_append_tag(Nonce::assume_unique_for_key(nonce), Aad::from(aad), &mut buf)
            .map_err(|_| CoreError::storage("encryption failed"))?;
        let mut out = Vec::with_capacity(VIEW_HEADER + buf.len());
        out.extend_from_slice(&self.kid.to_be_bytes());
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&buf);
        Ok(out)
    }

    fn open(&self, aad: &[u8], sealed: &[u8]) -> Result<Zeroizing<Vec<u8>>, CoreError> {
        let fail = || CoreError::invalid("a sealed record could not be opened");
        if sealed.len() < VIEW_HEADER + AES_256_GCM.tag_len() {
            return Err(fail());
        }
        let nonce = Nonce::try_assume_unique_for_key(&sealed[8..VIEW_HEADER]).map_err(|_| fail())?;
        let mut buf = Zeroizing::new(sealed[VIEW_HEADER..].to_vec());
        let len = self.aead()?.open_in_place(nonce, Aad::from(aad), &mut buf).map_err(|_| fail())?.len();
        buf.truncate(len);
        Ok(buf)
    }
}

/// The key id a view-key seal names (its first 8 bytes).
pub fn sealed_kid(sealed: &[u8]) -> Option<u64> {
    sealed.get(..8).and_then(|b| b.try_into().ok()).map(u64::from_be_bytes)
}

/// Every view key this phone holds, by key id.
#[derive(Clone, Default)]
pub struct ViewKeyring {
    keys: BTreeMap<u64, ViewKey>,
}

impl fmt::Debug for ViewKeyring {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ViewKeyring").field("kids", &self.keys.keys().collect::<Vec<_>>()).finish()
    }
}

/// One view key as it travels inside sealed messages and the keyring's stored form.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ViewKeyEntry {
    pub kid: u64,
    /// base64url, 32 bytes.
    pub key: String,
}

impl fmt::Debug for ViewKeyEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ViewKeyEntry").field("kid", &self.kid).finish_non_exhaustive()
    }
}

/// Most view keys a sealed message (the recovery wrap, a bundle) carries: the newest ones.
pub const MAX_CARRIED_VIEW_KEYS: usize = 16;

impl ViewKeyring {
    pub fn insert(&mut self, key: ViewKey) {
        self.keys.insert(key.kid, key);
    }

    pub fn get(&self, kid: u64) -> Option<&ViewKey> {
        self.keys.get(&kid)
    }

    /// The newest key: the one new seals use.
    pub fn current(&self) -> Option<&ViewKey> {
        self.keys.values().next_back()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    pub fn kids(&self) -> Vec<u64> {
        self.keys.keys().copied().collect()
    }

    /// Seals with the newest key.
    pub fn seal(&self, aad: &[u8], plain: &[u8]) -> Result<Vec<u8>, CoreError> {
        self.current().ok_or_else(|| CoreError::invalid("this phone holds no view key"))?.seal(aad, plain)
    }

    /// Opens a seal with the key it names.
    pub fn open(&self, aad: &[u8], sealed: &[u8]) -> Result<Zeroizing<Vec<u8>>, CoreError> {
        let kid = sealed_kid(sealed).ok_or_else(|| CoreError::invalid("a sealed record could not be opened"))?;
        self.get(kid)
            .ok_or_else(|| CoreError::invalid("a record is sealed with a view key this phone lacks"))?
            .open(aad, sealed)
    }

    /// The newest [`MAX_CARRIED_VIEW_KEYS`] keys, as they travel.
    pub fn entries(&self) -> Vec<ViewKeyEntry> {
        let skip = self.keys.len().saturating_sub(MAX_CARRIED_VIEW_KEYS);
        self.keys
            .values()
            .skip(skip)
            .map(|k| ViewKeyEntry {
                kid: k.kid,
                key: BASE64URL_NOPAD.encode(&k.key[..]),
            })
            .collect()
    }

    /// Adds keys as they travel; a malformed one fails the whole call.
    pub fn extend(&mut self, entries: &[ViewKeyEntry]) -> Result<(), CoreError> {
        for e in entries {
            let raw = Zeroizing::new(
                BASE64URL_NOPAD.decode(e.key.as_bytes()).map_err(|_| CoreError::invalid("not a view key"))?,
            );
            if e.kid == 0 {
                return Err(CoreError::invalid("view key ids start at 1"));
            }
            self.insert(ViewKey::from_bytes(e.kid, &raw)?);
        }
        Ok(())
    }

    /// The stored form (JSON of [`ViewKeyring::all_entries`]).
    pub fn to_bytes(&self) -> Result<Zeroizing<Vec<u8>>, CoreError> {
        serde_json::to_vec(&self.all_entries()).map(Zeroizing::new).map_err(|e| CoreError::storage(e.to_string()))
    }

    pub fn from_bytes(raw: &[u8]) -> Result<Self, CoreError> {
        let entries: Vec<ViewKeyEntry> =
            serde_json::from_slice(raw).map_err(|_| CoreError::storage("corrupt view keys"))?;
        let mut ring = Self::default();
        ring.extend(&entries)?;
        Ok(ring)
    }

    fn all_entries(&self) -> Vec<ViewKeyEntry> {
        self.keys
            .values()
            .map(|k| ViewKeyEntry {
                kid: k.kid,
                key: BASE64URL_NOPAD.encode(&k.key[..]),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signatures_verify_only_for_their_key_and_message() {
        let key = SigningKey::from_seed(&[7; 32]).unwrap();
        let sig = key.sign(b"message");
        assert!(ed25519_verify(&key.public(), b"message", &sig));
        assert!(!ed25519_verify(&key.public(), b"messagE", &sig));
        let other = SigningKey::from_seed(&[8; 32]).unwrap();
        assert!(!ed25519_verify(&other.public(), b"message", &sig));
        let mut bad = sig;
        bad[0] ^= 1;
        assert!(!ed25519_verify(&key.public(), b"message", &bad));
        // A small-order key is refused before ring sees it (ring alone would accept some signatures under it).
        let mut identity = [0u8; 32];
        identity[0] = 1;
        assert!(!ed25519_verify(&identity, b"message", &sig));
        assert!(!format!("{key:?}").contains("seed"));
    }

    #[test]
    fn the_owner_seed_derives_distinct_stable_keys() {
        let seed = OwnerSeed::from_bytes(&[1; 32]).unwrap();
        let again = OwnerSeed::from_bytes(&[1; 32]).unwrap();
        assert_eq!(seed.owner_key().unwrap().public(), again.owner_key().unwrap().public());
        assert_eq!(seed.credential_key().public(), again.credential_key().public());
        assert_ne!(seed.owner_key().unwrap().public(), seed.credential_key().public());
        assert_eq!(seed.view_key("a", 1).as_bytes(), again.view_key("a", 1).as_bytes());
        assert_ne!(seed.view_key("a", 1).as_bytes(), seed.view_key("a", 2).as_bytes(), "per key id");
        assert_ne!(seed.view_key("a", 1).as_bytes(), seed.view_key("b", 1).as_bytes(), "per account");
        let other = OwnerSeed::from_bytes(&[2; 32]).unwrap();
        assert_ne!(seed.view_key("a", 1).as_bytes(), other.view_key("a", 1).as_bytes());
        assert_ne!(seed.owner_key().unwrap().public(), other.owner_key().unwrap().public());
        assert!(OwnerSeed::from_bytes(&[1; 31]).is_err());
        assert_eq!(format!("{seed:?}"), "OwnerSeed(<redacted>)");
    }

    #[test]
    fn view_seals_open_only_with_their_key_and_associated_data() {
        let seed = OwnerSeed::from_bytes(&[3; 32]).unwrap();
        let mut ring = ViewKeyring::default();
        assert!(ring.seal(b"aad", b"x").is_err(), "no key yet");
        ring.insert(seed.view_key("a", 1));
        let sealed = ring.seal(b"aad", b"record").unwrap();
        assert_eq!(sealed_kid(&sealed), Some(1));
        assert_eq!(&*ring.open(b"aad", &sealed).unwrap(), b"record");
        assert!(ring.open(b"other aad", &sealed).is_err(), "the envelope is bound");
        let mut tampered = sealed.clone();
        *tampered.last_mut().unwrap() ^= 1;
        assert!(ring.open(b"aad", &tampered).is_err());
        assert!(ring.open(b"aad", &sealed[..20]).is_err());
        // Relabelled with another key id: no such key, or the wrong one.
        let mut relabelled = sealed.clone();
        relabelled[7] = 2;
        assert!(ring.open(b"aad", &relabelled).is_err());
        ring.insert(seed.view_key("a", 2));
        assert!(ring.open(b"aad", &relabelled).is_err());
        // New seals use the newest key; old ones still open.
        assert_eq!(sealed_kid(&ring.seal(b"aad", b"x").unwrap()), Some(2));
        assert_eq!(&*ring.open(b"aad", &sealed).unwrap(), b"record");
        // Another owner's key of the same id opens nothing.
        let mut stranger = ViewKeyring::default();
        stranger.insert(OwnerSeed::from_bytes(&[4; 32]).unwrap().view_key("a", 1));
        assert!(stranger.open(b"aad", &sealed).is_err());
    }

    #[test]
    fn keyrings_round_trip_and_carry_the_newest_keys() {
        let seed = OwnerSeed::from_bytes(&[5; 32]).unwrap();
        let mut ring = ViewKeyring::default();
        for kid in 1..=20 {
            ring.insert(seed.view_key("a", kid));
        }
        let back = ViewKeyring::from_bytes(&ring.to_bytes().unwrap()).unwrap();
        assert_eq!(back.kids(), ring.kids());
        let carried = ring.entries();
        assert_eq!(carried.len(), MAX_CARRIED_VIEW_KEYS);
        assert_eq!(carried.last().unwrap().kid, 20);
        let mut bad = ViewKeyring::default();
        assert!(
            bad.extend(&[ViewKeyEntry {
                kid: 0,
                key: BASE64URL_NOPAD.encode(&[0; 32])
            }])
            .is_err()
        );
        assert!(
            bad.extend(&[ViewKeyEntry {
                kid: 1,
                key: BASE64URL_NOPAD.encode(&[0; 31])
            }])
            .is_err()
        );
        assert!(!format!("{:?}", carried[0]).contains(&carried[0].key));
    }

    #[test]
    fn sealed_boxes_open_only_with_their_key() {
        let key = BoxKey::from_bytes([9; 32]);
        let sealed = seal_to(&key.public_b64(), b"secret").unwrap();
        assert_eq!(&*key.open(&sealed).unwrap(), b"secret");
        assert!(BoxKey::from_bytes([10; 32]).open(&sealed).is_err());
        assert!(seal_to("short", b"x").is_err());
    }

    #[test]
    fn contexts_are_distinct_from_each_other_and_from_the_protocols() {
        use reins_proto::canon::contexts as p;
        let ours = [
            contexts::OWNER_SIGN_KEY,
            contexts::CREDENTIAL_KEY,
            contexts::VIEW_KEY,
            contexts::RECOVERY_WRAP_KEY,
            contexts::RECOVERY_WRAP,
            contexts::HANDOVER_BUNDLE,
            contexts::VIEW_KEY_GRANT,
            contexts::CHECKPOINT_BODY,
        ];
        let theirs = [
            p::ROSTER_ENTRY,
            p::ROSTER_HASH,
            p::OP,
            p::CHECKPOINT,
            p::HEARTBEAT,
            p::APPLIED,
            p::ANSWER,
            p::ENROLL_COMMIT,
            p::ENROLL_CODE,
            p::ENROLL_PROOF,
            p::ENROLL_PROOF_KEY,
            p::OP_BODY,
        ];
        let all: std::collections::BTreeSet<&str> = ours.iter().chain(theirs.iter()).copied().collect();
        assert_eq!(all.len(), ours.len() + theirs.len());
    }
}
