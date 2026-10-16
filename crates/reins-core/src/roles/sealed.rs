//! What phone roles seal, and how each is authenticated:
//!
//! | Message | Sealed with | Authenticated by |
//! |---|---|---|
//! | recovery wrap | AES-256-GCM under a key the master key derives, associated data `reins/recovery-wrap/v1` and the account | the AEAD (only the master key opens or makes it); the roster names its hash |
//! | handover bundle | a sealed box to the new primary's box key | the old primary's signature inside, over the account, the handover entry's epoch and hash and the payload's hash (sign-then-seal) |
//! | view-key grant | a sealed box to one member's box key | the primary's signature inside, over the account, the member, the key id and the payload's hash |
//! | enrollment grant | a sealed box to the joiner's box key | a [`SignedAnswer`] of kind `join` inside, by the primary key the six digits covered |
//!
//! A sealed box alone says nothing about who sealed it (anyone can seal to a public key), so every one carries a
//! signature its opener checks against a key it already trusts.

use std::fmt;

use data_encoding::BASE64URL_NOPAD;
use reins_proto::PROTOCOL_VERSION;
use reins_proto::answer::{AnswerKind, SignedAnswer, answer_message};
use reins_proto::canon::{Canon, decode_payload, encode_payload, parse_sig, sha256};
use reins_proto::roster::{Member, RosterEntry, RosterReason};
use ring::aead::{AES_256_GCM, Aad, LessSafeKey, NONCE_LEN, Nonce, UnboundKey};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use super::keys::{
    BoxKey, OwnerSeed, SigningKey, ViewKey, ViewKeyEntry, ViewKeyring, contexts, ed25519_verify, seal_to,
};
use crate::CoreError;
use crate::crypto::{MasterKey, random_bytes};

fn bad(what: &str) -> CoreError {
    CoreError::invalid(format!("{what} could not be opened"))
}

fn aead(key: &[u8; 32]) -> Result<LessSafeKey, CoreError> {
    UnboundKey::new(&AES_256_GCM, key).map(LessSafeKey::new).map_err(|_| CoreError::storage("invalid key"))
}

fn json<T: Serialize>(value: &T) -> Result<Zeroizing<Vec<u8>>, CoreError> {
    serde_json::to_vec(value).map(Zeroizing::new).map_err(|e| CoreError::storage(e.to_string()))
}

// ---- recovery wrap ----------------------------------------------------------------------------------------------

/// The key the recovery wrap of `account` is sealed with: derived from the master key (the recovery code's, or the
/// master password's), so whoever can sign in with it can open the wrap, and nobody else.
pub fn recovery_wrap_key(master: &MasterKey, account: &str) -> Zeroizing<[u8; 32]> {
    let mut c = Canon::new(contexts::RECOVERY_WRAP_KEY);
    c.str(account);
    master.derive(&c.finish())
}

fn wrap_aad(account: &str) -> Vec<u8> {
    let mut c = Canon::new(contexts::RECOVERY_WRAP);
    c.str(account);
    c.finish()
}

#[derive(Serialize, Deserialize)]
struct WrapPlain {
    v: u32,
    owner_seed: String,
    /// View keys of earlier owners (a recovery's new primary needs them until its first checkpoint); this owner's
    /// own view keys come from the seed.
    #[serde(default)]
    view_keys: Vec<ViewKeyEntry>,
}

/// The recovery wrap (base64url: nonce ‖ AES-256-GCM): the owner seed and the view keys `carried`.
pub fn seal_recovery_wrap(
    key: &[u8; 32],
    account: &str,
    seed: &OwnerSeed,
    carried: &ViewKeyring,
) -> Result<String, CoreError> {
    let plain = json(&WrapPlain {
        v: PROTOCOL_VERSION,
        owner_seed: BASE64URL_NOPAD.encode(seed.as_bytes()),
        view_keys: carried.entries(),
    })?;
    let nonce = random_bytes::<NONCE_LEN>()?;
    let mut buf = plain.to_vec();
    aead(key)?
        .seal_in_place_append_tag(Nonce::assume_unique_for_key(nonce), Aad::from(wrap_aad(account)), &mut buf)
        .map_err(|_| CoreError::storage("encryption failed"))?;
    let mut out = nonce.to_vec();
    out.extend_from_slice(&buf);
    Ok(encode_payload(&out))
}

/// Opens a recovery wrap of `account`: its owner seed and carried view keys. Fails for a wrong key (not this
/// account's master key), another account's wrap, or anything tampered with.
pub fn open_recovery_wrap(key: &[u8; 32], account: &str, wrap: &str) -> Result<(OwnerSeed, ViewKeyring), CoreError> {
    let fail = || bad("The recovery wrap");
    let raw = decode_payload(wrap).ok_or_else(fail)?;
    if raw.len() < NONCE_LEN + AES_256_GCM.tag_len() {
        return Err(fail());
    }
    let nonce = Nonce::try_assume_unique_for_key(&raw[..NONCE_LEN]).map_err(|_| fail())?;
    let mut buf = Zeroizing::new(raw[NONCE_LEN..].to_vec());
    let plain = aead(key)?.open_in_place(nonce, Aad::from(wrap_aad(account)), &mut buf).map_err(|_| fail())?;
    let parsed: WrapPlain = serde_json::from_slice(plain).map_err(|_| fail())?;
    if parsed.v != PROTOCOL_VERSION {
        return Err(fail());
    }
    let seed = Zeroizing::new(BASE64URL_NOPAD.decode(parsed.owner_seed.as_bytes()).map_err(|_| fail())?);
    let mut ring = ViewKeyring::default();
    ring.extend(&parsed.view_keys)?;
    Ok((OwnerSeed::from_bytes(&seed)?, ring))
}

// ---- signed, then sealed ----------------------------------------------------------------------------------------

/// What travels inside a sealed box: a payload and its signature.
#[derive(Serialize, Deserialize)]
struct Signed {
    payload: String,
    sig: String,
}

fn seal_signed(to: &str, payload: &[u8], sig: String) -> Result<String, CoreError> {
    let plain = json(&Signed {
        payload: encode_payload(payload),
        sig,
    })?;
    Ok(encode_payload(&seal_to(to, &plain)?))
}

/// Opens a [`seal_signed`] box: the payload and the signature (both still to be checked by the caller).
fn open_signed(key: &BoxKey, sealed: &str, what: &str) -> Result<(Zeroizing<Vec<u8>>, [u8; 64]), CoreError> {
    let raw = decode_payload(sealed).ok_or_else(|| bad(what))?;
    let plain = key.open(&raw).map_err(|_| bad(what))?;
    let signed: Signed = serde_json::from_slice(&plain).map_err(|_| bad(what))?;
    let payload = Zeroizing::new(decode_payload(&signed.payload).ok_or_else(|| bad(what))?);
    let sig = parse_sig(&signed.sig).ok_or_else(|| bad(what))?;
    Ok((payload, sig))
}

// ---- handover bundle --------------------------------------------------------------------------------------------

/// A device secret that moves with the primary role (the card provider's key, `payments.provider`).
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CarriedSecret {
    pub service: String,
    pub account: String,
    /// base64url.
    pub value: String,
}

impl fmt::Debug for CarriedSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CarriedSecret")
            .field("service", &self.service)
            .field("account", &self.account)
            .finish_non_exhaustive()
    }
}

/// What the old primary hands the new one.
#[derive(Clone, Serialize, Deserialize)]
pub struct BundlePayload {
    pub v: u32,
    /// base64url, 32 bytes.
    pub owner_seed: String,
    #[serde(default)]
    pub view_keys: Vec<ViewKeyEntry>,
    /// The account secret, base64url (Q1 = B: viewers hold it already; it rides along for one that lost it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_secret: Option<String>,
    #[serde(default)]
    pub device_secrets: Vec<CarriedSecret>,
}

impl fmt::Debug for BundlePayload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BundlePayload").field("device_secrets", &self.device_secrets).finish_non_exhaustive()
    }
}

fn bundle_message(account: &str, entry: &RosterEntry, payload: &[u8]) -> Result<Vec<u8>, CoreError> {
    let hash = entry.hash().map_err(|e| CoreError::invalid(e.to_string()))?;
    let mut c = Canon::new(contexts::HANDOVER_BUNDLE);
    c.str(account).u64(entry.epoch).key(&hash).key(&sha256(payload));
    Ok(c.finish())
}

/// The bundle for the `handover` entry `entry` (signed by the old primary's device key, sealed to the new primary's
/// box key), base64url.
pub fn seal_bundle(
    signer: &SigningKey,
    account: &str,
    entry: &RosterEntry,
    payload: &BundlePayload,
) -> Result<String, CoreError> {
    let bytes = json(payload)?;
    let sig = signer.sign_b64(&bundle_message(account, entry, &bytes)?);
    seal_signed(&entry.primary.box_pub, &bytes, sig)
}

/// Opens the bundle of the `handover` entry `entry` (verified already), whose previous primary had signing key
/// `old_primary`: refused unless that key signed it for exactly this entry, and its owner seed is the entry's.
pub fn open_bundle(
    key: &BoxKey,
    sealed: &str,
    account: &str,
    entry: &RosterEntry,
    old_primary: &[u8; 32],
) -> Result<(BundlePayload, OwnerSeed), CoreError> {
    let what = "The handover bundle";
    if entry.reason != RosterReason::Handover {
        return Err(bad(what));
    }
    let (payload, sig) = open_signed(key, sealed, what)?;
    if !ed25519_verify(old_primary, &bundle_message(account, entry, &payload)?, &sig) {
        return Err(CoreError::invalid("The handover bundle is not signed by the old primary"));
    }
    let parsed: BundlePayload = serde_json::from_slice(&payload).map_err(|_| bad(what))?;
    if parsed.v != PROTOCOL_VERSION {
        return Err(bad(what));
    }
    let seed = OwnerSeed::from_bytes(&Zeroizing::new(
        BASE64URL_NOPAD.decode(parsed.owner_seed.as_bytes()).map_err(|_| bad(what))?,
    ))?;
    if seed.owner_key()?.public_b64() != entry.owner_pub {
        return Err(CoreError::invalid("The handover bundle carries another owner key"));
    }
    Ok((parsed, seed))
}

// ---- view-key grants --------------------------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
struct GrantPlain {
    v: u32,
    account: String,
    device_id: String,
    kid: u64,
    key: String,
}

fn grant_message(account: &str, device_id: &str, kid: u64, payload: &[u8]) -> Vec<u8> {
    let mut c = Canon::new(contexts::VIEW_KEY_GRANT);
    c.str(account).str(device_id).u64(kid).key(&sha256(payload));
    c.finish()
}

/// `key` for `member` (a rotation after a removal or a recovery): signed by the primary's device key, sealed to the
/// member's box key, base64url.
pub fn seal_view_grant(
    signer: &SigningKey,
    account: &str,
    member: &Member,
    key: &ViewKey,
) -> Result<String, CoreError> {
    let bytes = json(&GrantPlain {
        v: PROTOCOL_VERSION,
        account: account.to_owned(),
        device_id: member.device_id.clone(),
        kid: key.kid,
        key: BASE64URL_NOPAD.encode(key.as_bytes()),
    })?;
    let sig = signer.sign_b64(&grant_message(account, &member.device_id, key.kid, &bytes));
    seal_signed(&member.box_pub, &bytes, sig)
}

/// Opens a grant sealed to this phone (`device_id`, box key `key`). `signer_at`: the signing key of the primary at a
/// key id's epoch, from the verified roster (`None`: no such epoch). Refused unless that primary signed it for this
/// account and this phone.
pub fn open_view_grant(
    key: &BoxKey,
    sealed: &str,
    account: &str,
    device_id: &str,
    signer_at: impl Fn(u64) -> Option<[u8; 32]>,
) -> Result<ViewKey, CoreError> {
    let what = "A view key";
    let (payload, sig) = open_signed(key, sealed, what)?;
    let parsed: GrantPlain = serde_json::from_slice(&payload).map_err(|_| bad(what))?;
    if parsed.v != PROTOCOL_VERSION || parsed.account != account || parsed.device_id != device_id || parsed.kid == 0 {
        return Err(bad(what));
    }
    let signer = signer_at(parsed.kid).ok_or_else(|| bad(what))?;
    if !ed25519_verify(&signer, &grant_message(account, device_id, parsed.kid, &payload), &sig) {
        return Err(CoreError::invalid("A view key is not signed by the primary"));
    }
    let raw = Zeroizing::new(BASE64URL_NOPAD.decode(parsed.key.as_bytes()).map_err(|_| bad(what))?);
    ViewKey::from_bytes(parsed.kid, &raw)
}

// ---- answers ----------------------------------------------------------------------------------------------------

/// `answer` (its exact bytes) signed by the primary's device key, as the server and desktops verify it.
pub fn sign_answer(
    signer: &SigningKey,
    account: &str,
    kind: AnswerKind,
    id: &str,
    epoch: u64,
    answer: &[u8],
) -> SignedAnswer {
    SignedAnswer {
        v: PROTOCOL_VERSION,
        epoch,
        answer: encode_payload(answer),
        sig: signer.sign_b64(&answer_message(account, kind, id, epoch, answer)),
    }
}

/// The answer's bytes when `signer` signed it for (`account`, `kind`, `id`) at its epoch.
pub fn verify_answer(
    signed: &SignedAnswer,
    primary: &[u8; 32],
    account: &str,
    kind: AnswerKind,
    id: &str,
) -> Result<Vec<u8>, CoreError> {
    let bytes = signed.answer_bytes().map_err(|e| CoreError::invalid(e.to_string()))?;
    let sig = parse_sig(&signed.sig).ok_or_else(|| CoreError::invalid("not a signature"))?;
    if !ed25519_verify(primary, &answer_message(account, kind, id, signed.epoch, &bytes), &sig) {
        return Err(CoreError::invalid("The answer is not signed by the primary"));
    }
    Ok(bytes)
}

// ---- enrollment grant -------------------------------------------------------------------------------------------

/// What an enrolling phone gets once the primary approves (inside a [`SignedAnswer`] of kind `join`, sealed to its box
/// key).
#[derive(Clone, Serialize, Deserialize)]
pub struct EnrollGrant {
    pub v: u32,
    pub join_id: String,
    /// [`RosterEntry::hash`] of the `enroll` entry that lists the phone, hex.
    pub roster_hash: String,
    pub view_keys: Vec<ViewKeyEntry>,
    /// The account secret, base64url (Q1 = B: a viewer keeps it, as a second phone does today); `None` for an
    /// account made with a master password, whose phones sign in with it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_secret: Option<String>,
}

impl fmt::Debug for EnrollGrant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EnrollGrant").field("join_id", &self.join_id).finish_non_exhaustive()
    }
}

/// The grant, signed at `epoch` as the answer to join `grant.join_id`, sealed to `to` (the joiner's box key).
pub fn seal_enroll_grant(
    signer: &SigningKey,
    account: &str,
    epoch: u64,
    to: &str,
    grant: &EnrollGrant,
) -> Result<String, CoreError> {
    let bytes = json(grant)?;
    let answer = sign_answer(signer, account, AnswerKind::Join, &grant.join_id, epoch, &bytes);
    Ok(encode_payload(&seal_to(to, &json(&answer)?)?))
}

/// Opens the grant answering `join_id`, signed by `primary` (the key the enrollment code covered): the epoch it was
/// signed at and the grant.
pub fn open_enroll_grant(
    key: &BoxKey,
    sealed: &str,
    account: &str,
    join_id: &str,
    primary: &[u8; 32],
) -> Result<(u64, EnrollGrant), CoreError> {
    let what = "The other phone's answer";
    let raw = decode_payload(sealed).ok_or_else(|| bad(what))?;
    let plain = key.open(&raw).map_err(|_| bad(what))?;
    let signed: SignedAnswer = serde_json::from_slice(&plain).map_err(|_| bad(what))?;
    let bytes = Zeroizing::new(verify_answer(&signed, primary, account, AnswerKind::Join, join_id)?);
    let grant: EnrollGrant = serde_json::from_slice(&bytes).map_err(|_| bad(what))?;
    if grant.v != PROTOCOL_VERSION || grant.join_id != join_id {
        return Err(bad(what));
    }
    Ok((signed.epoch, grant))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::{Kdf, master_key};

    fn master(password: &str) -> MasterKey {
        master_key(
            password,
            "salt",
            Kdf::Pbkdf2 {
                iterations: 5_000,
            },
        )
        .unwrap()
    }

    fn member(id: &str, box_key: &BoxKey, sign: &SigningKey) -> Member {
        Member {
            device_id: id.to_owned(),
            name: format!("Phone {id}"),
            sign_pub: sign.public_b64(),
            box_pub: box_key.public_b64(),
        }
    }

    #[test]
    fn the_recovery_wrap_opens_only_with_the_accounts_master_key() {
        let seed = OwnerSeed::generate().unwrap();
        let mut carried = ViewKeyring::default();
        carried.insert(OwnerSeed::generate().unwrap().view_key("acct", 3));
        let key = recovery_wrap_key(&master("pw"), "acct");
        let wrap = seal_recovery_wrap(&key, "acct", &seed, &carried).unwrap();
        let (back, ring) = open_recovery_wrap(&key, "acct", &wrap).unwrap();
        assert_eq!(back.as_bytes(), seed.as_bytes());
        assert_eq!(ring.kids(), [3]);
        // Another password, another account, the same master key for another account: nothing opens.
        assert!(open_recovery_wrap(&recovery_wrap_key(&master("other"), "acct"), "acct", &wrap).is_err());
        assert!(open_recovery_wrap(&key, "acct2", &wrap).is_err());
        assert!(open_recovery_wrap(&recovery_wrap_key(&master("pw"), "acct2"), "acct2", &wrap).is_err());
        let mut raw = decode_payload(&wrap).unwrap();
        *raw.last_mut().unwrap() ^= 1;
        assert!(open_recovery_wrap(&key, "acct", &encode_payload(&raw)).is_err(), "tampered");
        assert!(open_recovery_wrap(&key, "acct", "AAAA").is_err(), "truncated");
        assert!(open_recovery_wrap(&key, "acct", "not base64!").is_err());
    }

    fn handover_entry(old: &Member, new: &Member, seed: &OwnerSeed) -> RosterEntry {
        let owner = seed.owner_key().unwrap();
        let genesis = RosterEntry::genesis(
            "acct",
            old.clone(),
            owner.public_b64(),
            seed.credential_key().public_b64(),
            reins_proto::canon::hex32(&[1; 32]),
            1,
        );
        genesis.successor(RosterReason::Handover, new.clone(), vec![old.clone()], 2).unwrap()
    }

    #[test]
    fn a_bundle_opens_for_its_entry_signed_by_the_old_primary_with_its_owner_seed() {
        let (old_sign, new_sign) = (SigningKey::from_seed(&[1; 32]).unwrap(), SigningKey::from_seed(&[2; 32]).unwrap());
        let (old_box, new_box) = (BoxKey::from_bytes([3; 32]), BoxKey::from_bytes([4; 32]));
        let (old, new) = (member("p", &old_box, &old_sign), member("v", &new_box, &new_sign));
        let seed = OwnerSeed::generate().unwrap();
        let entry = handover_entry(&old, &new, &seed);
        let payload = BundlePayload {
            v: 1,
            owner_seed: BASE64URL_NOPAD.encode(seed.as_bytes()),
            view_keys: vec![],
            account_secret: None,
            device_secrets: vec![CarriedSecret {
                service: "payments.provider".to_owned(),
                account: "privacy".to_owned(),
                value: "c2VjcmV0".to_owned(),
            }],
        };
        let sealed = seal_bundle(&old_sign, "acct", &entry, &payload).unwrap();
        let (opened, got) = open_bundle(&new_box, &sealed, "acct", &entry, &old_sign.public()).unwrap();
        assert_eq!(got.as_bytes(), seed.as_bytes());
        assert_eq!(opened.device_secrets, payload.device_secrets);
        assert!(!format!("{opened:?}").contains("c2VjcmV0"));
        // Signed by anyone else (the server sealing its own bundle with its own key), for another entry, opened by
        // another phone, or carrying another owner seed: refused.
        let forged = seal_bundle(&new_sign, "acct", &entry, &payload).unwrap();
        assert!(open_bundle(&new_box, &forged, "acct", &entry, &old_sign.public()).is_err());
        let mut later = entry.clone();
        later.at += 1;
        assert!(open_bundle(&new_box, &sealed, "acct", &later, &old_sign.public()).is_err());
        assert!(open_bundle(&new_box, &sealed, "acct2", &entry, &old_sign.public()).is_err());
        assert!(open_bundle(&old_box, &sealed, "acct", &entry, &old_sign.public()).is_err());
        let other_seed = BundlePayload {
            owner_seed: BASE64URL_NOPAD.encode(OwnerSeed::generate().unwrap().as_bytes()),
            ..payload.clone()
        };
        let wrong = seal_bundle(&old_sign, "acct", &entry, &other_seed).unwrap();
        assert!(open_bundle(&new_box, &wrong, "acct", &entry, &old_sign.public()).is_err());
        let mut not_handover = entry;
        not_handover.reason = RosterReason::Enroll;
        let sealed_enroll = seal_bundle(&old_sign, "acct", &not_handover, &payload).unwrap();
        assert!(open_bundle(&new_box, &sealed_enroll, "acct", &not_handover, &old_sign.public()).is_err());
    }

    #[test]
    fn a_view_grant_opens_for_its_member_signed_by_the_primary() {
        let primary = SigningKey::from_seed(&[1; 32]).unwrap();
        let (vbox, vsign) = (BoxKey::from_bytes([5; 32]), SigningKey::from_seed(&[6; 32]).unwrap());
        let viewer = member("v", &vbox, &vsign);
        let key = OwnerSeed::generate().unwrap().view_key("acct", 4);
        let sealed = seal_view_grant(&primary, "acct", &viewer, &key).unwrap();
        let signer = |kid| (kid == 4).then(|| primary.public());
        let opened = open_view_grant(&vbox, &sealed, "acct", "v", signer).unwrap();
        assert_eq!((opened.kid, opened.as_bytes()), (4, key.as_bytes()));
        // For another phone or account, signed by a viewer, or at an epoch the roster lacks: refused.
        assert!(open_view_grant(&vbox, &sealed, "acct", "w", signer).is_err());
        assert!(open_view_grant(&vbox, &sealed, "acct2", "v", signer).is_err());
        let forged = seal_view_grant(&vsign, "acct", &viewer, &key).unwrap();
        assert!(open_view_grant(&vbox, &forged, "acct", "v", signer).is_err());
        assert!(open_view_grant(&vbox, &sealed, "acct", "v", |_| None).is_err());
        assert!(open_view_grant(&BoxKey::from_bytes([7; 32]), &sealed, "acct", "v", signer).is_err());
    }

    #[test]
    fn answers_verify_for_their_signer_item_and_bytes_only() {
        let primary = SigningKey::from_seed(&[1; 32]).unwrap();
        let signed = sign_answer(&primary, "acct", AnswerKind::Request, "r1", 3, b"{\"v\":1}");
        assert_eq!(verify_answer(&signed, &primary.public(), "acct", AnswerKind::Request, "r1").unwrap(), b"{\"v\":1}");
        let other = SigningKey::from_seed(&[2; 32]).unwrap();
        assert!(verify_answer(&signed, &other.public(), "acct", AnswerKind::Request, "r1").is_err());
        assert!(verify_answer(&signed, &primary.public(), "acct", AnswerKind::Request, "r2").is_err());
        assert!(verify_answer(&signed, &primary.public(), "acct", AnswerKind::Pairing, "r1").is_err());
        let mut moved = signed.clone();
        moved.epoch = 4;
        assert!(verify_answer(&moved, &primary.public(), "acct", AnswerKind::Request, "r1").is_err());
        let mut swapped = signed;
        swapped.answer = encode_payload(b"{\"v\":2}");
        assert!(verify_answer(&swapped, &primary.public(), "acct", AnswerKind::Request, "r1").is_err());
    }

    #[test]
    fn an_enrollment_grant_opens_for_its_join_signed_by_the_compared_primary() {
        let primary = SigningKey::from_seed(&[1; 32]).unwrap();
        let joiner = BoxKey::from_bytes([8; 32]);
        let grant = EnrollGrant {
            v: 1,
            join_id: "j1".to_owned(),
            roster_hash: reins_proto::canon::hex32(&[2; 32]),
            view_keys: vec![],
            account_secret: Some("c2VjcmV0".to_owned()),
        };
        let sealed = seal_enroll_grant(&primary, "acct", 2, &joiner.public_b64(), &grant).unwrap();
        let (epoch, got) = open_enroll_grant(&joiner, &sealed, "acct", "j1", &primary.public()).unwrap();
        assert_eq!((epoch, got.account_secret.as_deref()), (2, Some("c2VjcmV0")));
        assert!(!format!("{got:?}").contains("c2VjcmV0"));
        // Replayed to another join, signed by another key (the server's), or opened elsewhere: refused.
        assert!(open_enroll_grant(&joiner, &sealed, "acct", "j2", &primary.public()).is_err());
        let server = SigningKey::from_seed(&[9; 32]).unwrap();
        let forged = seal_enroll_grant(&server, "acct", 2, &joiner.public_b64(), &grant).unwrap();
        assert!(open_enroll_grant(&joiner, &forged, "acct", "j1", &primary.public()).is_err());
        assert!(open_enroll_grant(&BoxKey::from_bytes([9; 32]), &sealed, "acct", "j1", &primary.public()).is_err());
        let mismatched = EnrollGrant {
            join_id: "j2".to_owned(),
            ..grant
        };
        let inner = seal_enroll_grant(&primary, "acct", 2, &joiner.public_b64(), &mismatched).unwrap();
        assert!(open_enroll_grant(&joiner, &inner, "acct", "j1", &primary.public()).is_err(), "signed for j2");
    }
}
