//! Per-tenant keys behind a KMS seam (`docs/design/decisions-v2/06-trust-and-compliance.md` §3–§4, `08` R12).
//!
//! Each org has its own:
//! - **issuer key** (Ed25519): signs its policy snapshots and its roster;
//! - **decision service key** (Ed25519): countersigns decisions and signs audit checkpoints and export heads;
//! - **data key** (AES-256-GCM): seals request bodies and results at rest (envelope encryption).
//!
//! Each is stored sealed by the [`Kms`], bound to its org, purpose and key id (the associated data), so a sealed key
//! copied to another tenant's row doesn't open. [`LocalKms`] seals with a 32-byte master key:
//! - with `REINS_ORG_KMS_PROVIDER=local` (Reins Cloud, self-hosted) the master key is kept in a file outside the
//!   database;
//! - with `aws`, `gcp` or `azure` (BYOK, [`super::kms_byok`]) that file holds the master key *wrapped* by the
//!   customer's KMS key, unwrapped once at launch and installed with [`install_kms`]. Until then [`kms`] fails
//!   closed: it never falls back to a local key.
//!
//! BYOK's limit: once unwrapped, the master key and the org signing keys it opens live in the server's memory while
//! it runs, so revoking the KMS key stops the next launch, not the running one. Signing inside an HSM is later. With
//! keys in the server's memory the decision service and the issuer key share a process: R5's split between the
//! decision service and the issuer holds fully only with a KMS that enforces the two-signature rule itself.

use std::{
    io::Write,
    sync::{Arc, LazyLock, RwLock},
};

use reins_proto::{
    canon::{decode_payload, encode_payload, encode_sig},
    managed::{Envelope, EnvelopeKind, canonical_json, envelope_message},
};
use ring::{
    aead::{AES_256_GCM, Aad, LessSafeKey, NONCE_LEN, Nonce, UnboundKey},
    rand::{SecureRandom, SystemRandom},
    signature::{Ed25519KeyPair, KeyPair},
};
use zeroize::Zeroizing;

use super::OrgError;
use crate::{
    CONFIG,
    db::{
        DbConnInner,
        models::reins_org_keys::{self, ReinsOrgKey},
    },
};

/// What a tenant key is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyPurpose {
    Issuer,
    Decision,
    Data,
}

impl KeyPurpose {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Issuer => "issuer",
            Self::Decision => "decision",
            Self::Data => "data",
        }
    }

    fn prefix(self) -> &'static str {
        match self {
            Self::Issuer => "iss",
            Self::Decision => "dec",
            Self::Data => "dat",
        }
    }
}

/// Seals and opens key material. `aad` binds a sealed value to where it belongs.
pub trait Kms: Send + Sync {
    fn seal(&self, aad: &[u8], plain: &[u8]) -> Result<String, String>;
    fn open(&self, aad: &[u8], sealed: &str) -> Result<Zeroizing<Vec<u8>>, String>;
    /// Which KMS (and master key) sealed it, for the console.
    fn id(&self) -> String;
}

fn gcm_seal(key: &[u8; 32], aad: &[u8], plain: &[u8]) -> Result<String, String> {
    let k = LessSafeKey::new(UnboundKey::new(&AES_256_GCM, key).map_err(|_| "bad key")?);
    let mut nonce = [0u8; NONCE_LEN];
    SystemRandom::new().fill(&mut nonce).map_err(|_| "no randomness")?;
    let mut buf = plain.to_vec();
    k.seal_in_place_append_tag(Nonce::assume_unique_for_key(nonce), Aad::from(aad), &mut buf)
        .map_err(|_| "seal failed")?;
    let mut out = nonce.to_vec();
    out.extend_from_slice(&buf);
    Ok(encode_payload(&out))
}

fn gcm_open(key: &[u8; 32], aad: &[u8], sealed: &str) -> Result<Zeroizing<Vec<u8>>, String> {
    let k = LessSafeKey::new(UnboundKey::new(&AES_256_GCM, key).map_err(|_| "bad key")?);
    let raw = decode_payload(sealed).ok_or("not sealed data")?;
    if raw.len() < NONCE_LEN + 16 {
        return Err("sealed data too short".to_owned());
    }
    let (nonce, ct) = raw.split_at(NONCE_LEN);
    let nonce = Nonce::try_assume_unique_for_key(nonce).map_err(|_| "bad nonce")?;
    let mut buf = Zeroizing::new(ct.to_vec());
    let n = k.open_in_place(nonce, Aad::from(aad), &mut buf).map_err(|_| "does not open")?.len();
    buf.truncate(n);
    Ok(buf)
}

/// Seals with a 32-byte master key: from a file (created 0600 on first use), or unwrapped by a BYOK KMS.
pub struct LocalKms {
    master: Zeroizing<[u8; 32]>,
    /// The id the console shows instead of `local:…` (a BYOK master: `<provider>:<key hash>`).
    id: Option<String>,
}

impl LocalKms {
    #[must_use]
    pub fn from_key(master: [u8; 32]) -> Self {
        Self {
            master: Zeroizing::new(master),
            id: None,
        }
    }

    /// A master key that came from somewhere else (a BYOK unwrap), named `id`.
    #[must_use]
    pub fn with_id(master: [u8; 32], id: String) -> Self {
        Self {
            master: Zeroizing::new(master),
            id: Some(id),
        }
    }

    /// Reads the master key at `path`, or creates it.
    pub fn from_file(path: &str) -> Result<Self, String> {
        match std::fs::read(path) {
            Ok(bytes) => {
                let key: [u8; 32] = bytes.as_slice().try_into().map_err(|_| format!("{path} is not a 32-byte key"))?;
                Ok(Self::from_key(key))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let mut key = [0u8; 32];
                SystemRandom::new().fill(&mut key).map_err(|_| "no randomness")?;
                let mut options = std::fs::OpenOptions::new();
                options.write(true).create_new(true);
                #[cfg(unix)]
                std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
                let mut f = options.open(path).map_err(|e| format!("{path}: {e}"))?;
                f.write_all(&key).map_err(|e| format!("{path}: {e}"))?;
                Ok(Self::from_key(key))
            }
            Err(e) => Err(format!("{path}: {e}")),
        }
    }
}

impl Kms for LocalKms {
    fn seal(&self, aad: &[u8], plain: &[u8]) -> Result<String, String> {
        gcm_seal(&self.master, aad, plain)
    }

    fn open(&self, aad: &[u8], sealed: &str) -> Result<Zeroizing<Vec<u8>>, String> {
        gcm_open(&self.master, aad, sealed)
    }

    fn id(&self) -> String {
        if let Some(id) = &self.id {
            return id.clone();
        }
        let h = reins_proto::canon::sha256(self.master.as_ref());
        format!("local:{}", &reins_proto::canon::hex32(&h)[..8])
    }
}

static KMS: LazyLock<RwLock<Option<Arc<dyn Kms>>>> = LazyLock::new(|| RwLock::new(None));

/// Whether `provider` (`REINS_ORG_KMS_PROVIDER`) is the local key file.
#[must_use]
pub fn provider_is_local(provider: &str) -> bool {
    provider.trim().eq_ignore_ascii_case("local")
}

/// The configured KMS: the installed one, else (provider `local` only) [`LocalKms`] over `REINS_ORG_KMS_KEY_FILE`.
pub fn kms() -> Result<Arc<dyn Kms>, OrgError> {
    resolve(&KMS, &CONFIG.reins_org_kms_provider(), &CONFIG.reins_org_kms_key_file())
}

/// [`kms`] over a given slot and settings. A cloud provider with nothing installed is an error, never a local key.
pub(super) fn resolve(
    slot: &RwLock<Option<Arc<dyn Kms>>>,
    provider: &str,
    key_file: &str,
) -> Result<Arc<dyn Kms>, OrgError> {
    if let Some(k) = slot.read().unwrap_or_else(std::sync::PoisonError::into_inner).clone() {
        return Ok(k);
    }
    if !provider_is_local(provider) {
        return Err(OrgError::Internal("the org KMS is not unlocked".into()));
    }
    let local: Arc<dyn Kms> = Arc::new(LocalKms::from_file(key_file).map_err(OrgError::Internal)?);
    let mut w = slot.write().unwrap_or_else(std::sync::PoisonError::into_inner);
    Ok(Arc::clone(w.get_or_insert(local)))
}

/// Installs the KMS every org key is sealed by (at launch, after a BYOK unwrap).
pub fn install_kms(kms: Arc<dyn Kms>) {
    *KMS.write().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(kms);
}

/// Replaces the KMS (tests).
#[cfg(test)]
pub fn set_kms(kms: Arc<dyn Kms>) {
    install_kms(kms);
}

fn key_aad(org: &str, purpose: KeyPurpose, key_id: &str) -> Vec<u8> {
    let mut c = reins_proto::canon::Canon::new("reins/org-kms-key/v1");
    c.str(org).str(purpose.as_str()).str(key_id);
    c.finish()
}

/// An org's Ed25519 key, opened for one use.
pub struct OrgSigner {
    pub key_id: String,
    pub public: [u8; 32],
    pair: Ed25519KeyPair,
}

impl OrgSigner {
    /// Ed25519 over `msg`, base64url without padding.
    #[must_use]
    pub fn sign(&self, msg: &[u8]) -> String {
        let sig = self.pair.sign(msg);
        let bytes: [u8; 64] = sig.as_ref().try_into().unwrap_or([0u8; 64]);
        encode_sig(&bytes)
    }

    #[must_use]
    pub fn public_b64(&self) -> String {
        encode_payload(&self.public)
    }
}

fn new_key_id(purpose: KeyPurpose) -> String {
    format!("{}-{}", purpose.prefix(), crate::crypto::encode_random_bytes::<9>(&data_encoding::HEXLOWER))
}

/// `org`'s signing key for `purpose` (issuer or decision), created on first use (`key.created` in audit).
pub fn signing_key(c: &mut DbConnInner, org: &str, purpose: KeyPurpose, now: i64) -> Result<OrgSigner, OrgError> {
    debug_assert_ne!(purpose, KeyPurpose::Data);
    let kms = kms()?;
    if let Some(row) = reins_org_keys::q_key(c, org, purpose.as_str())? {
        let pkcs8 = kms.open(&key_aad(org, purpose, &row.key_id), &row.sealed).map_err(OrgError::Internal)?;
        let pair =
            Ed25519KeyPair::from_pkcs8(&pkcs8).map_err(|_| OrgError::Internal("a stored key is not Ed25519".into()))?;
        let public: [u8; 32] = pair.public_key().as_ref().try_into().map_err(|_| OrgError::Internal("key".into()))?;
        return Ok(OrgSigner {
            key_id: row.key_id,
            public,
            pair,
        });
    }
    let key_id = new_key_id(purpose);
    let pkcs8 =
        Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).map_err(|_| OrgError::Internal("keygen".into()))?;
    let pair = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).map_err(|_| OrgError::Internal("keygen".into()))?;
    let public: [u8; 32] = pair.public_key().as_ref().try_into().map_err(|_| OrgError::Internal("key".into()))?;
    let sealed = kms.seal(&key_aad(org, purpose, &key_id), pkcs8.as_ref()).map_err(OrgError::Internal)?;
    reins_org_keys::q_insert_key(
        c,
        &ReinsOrgKey {
            org_uuid: org.to_owned(),
            key_id: key_id.clone(),
            purpose: purpose.as_str().to_owned(),
            public_key: encode_payload(&public),
            sealed,
            created_at: now,
            retired_at: None,
        },
    )?;
    super::event(
        c,
        org,
        "key.created",
        None,
        Some(&key_id),
        &serde_json::json!({"purpose": purpose.as_str(), "public_key": encode_payload(&public), "kms": kms.id()}),
        now,
    )?;
    Ok(OrgSigner {
        key_id,
        public,
        pair,
    })
}

/// Signs `body` (as canonical JSON) with `org`'s `purpose` key into an envelope of `kind`.
pub fn envelope(
    c: &mut DbConnInner,
    org: &str,
    purpose: KeyPurpose,
    kind: EnvelopeKind,
    body: &serde_json::Value,
    now: i64,
) -> Result<Envelope, OrgError> {
    let signer = signing_key(c, org, purpose, now)?;
    let body = canonical_json(body);
    let sig = signer.sign(&envelope_message(kind, org, &signer.key_id, &body));
    Ok(Envelope {
        kind,
        org: org.to_owned(),
        body,
        key_id: signer.key_id,
        sig,
    })
}

fn data_key(
    c: &mut DbConnInner,
    org: &str,
    key_id: Option<&str>,
    now: i64,
) -> Result<(String, Zeroizing<Vec<u8>>), OrgError> {
    let kms = kms()?;
    let row = match key_id {
        Some(id) => reins_org_keys::q_key_by_id(c, org, id)?.filter(|k| k.purpose == KeyPurpose::Data.as_str()),
        None => reins_org_keys::q_key(c, org, KeyPurpose::Data.as_str())?,
    };
    if let Some(row) = row {
        let key = kms.open(&key_aad(org, KeyPurpose::Data, &row.key_id), &row.sealed).map_err(OrgError::Internal)?;
        return Ok((row.key_id, key));
    }
    if key_id.is_some() {
        return Err(OrgError::NotFound);
    }
    let key_id = new_key_id(KeyPurpose::Data);
    let mut key = Zeroizing::new(vec![0u8; 32]);
    SystemRandom::new().fill(&mut key).map_err(|_| OrgError::Internal("no randomness".into()))?;
    let sealed = kms.seal(&key_aad(org, KeyPurpose::Data, &key_id), &key).map_err(OrgError::Internal)?;
    reins_org_keys::q_insert_key(
        c,
        &ReinsOrgKey {
            org_uuid: org.to_owned(),
            key_id: key_id.clone(),
            purpose: KeyPurpose::Data.as_str().to_owned(),
            public_key: String::new(),
            sealed,
            created_at: now,
            retired_at: None,
        },
    )?;
    Ok((key_id, key))
}

fn as_key(k: &[u8]) -> Result<[u8; 32], OrgError> {
    k.try_into().map_err(|_| OrgError::Internal("a data key is not 32 bytes".into()))
}

/// Seals `plain` under `org`'s data key, bound to `aad` (the request and part). Returns (key id, sealed).
pub fn seal_data(
    c: &mut DbConnInner,
    org: &str,
    aad: &str,
    plain: &[u8],
    now: i64,
) -> Result<(String, String), OrgError> {
    let (key_id, key) = data_key(c, org, None, now)?;
    let sealed = gcm_seal(&as_key(&key)?, aad.as_bytes(), plain).map_err(OrgError::Internal)?;
    Ok((key_id, sealed))
}

/// Opens what [`seal_data`] sealed for `org`.
pub fn open_data(
    c: &mut DbConnInner,
    org: &str,
    key_id: &str,
    aad: &str,
    sealed: &str,
) -> Result<Zeroizing<Vec<u8>>, OrgError> {
    let (_, key) = data_key(c, org, Some(key_id), 0)?;
    gcm_open(&as_key(&key)?, aad.as_bytes(), sealed).map_err(OrgError::Internal)
}

/// An org's public keys, as the console shows them and an executor pins them: (key id, base64url).
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OrgKeys {
    pub issuer: (String, String),
    pub decision: (String, String),
}

pub fn public_keys(c: &mut DbConnInner, org: &str, now: i64) -> Result<OrgKeys, OrgError> {
    let issuer = signing_key(c, org, KeyPurpose::Issuer, now)?;
    let decision = signing_key(c, org, KeyPurpose::Decision, now)?;
    Ok(OrgKeys {
        issuer: (issuer.key_id.clone(), issuer.public_b64()),
        decision: (decision.key_id.clone(), decision.public_b64()),
    })
}

#[cfg(all(test, sqlite))]
pub(crate) mod tests {
    use reins_audit::{TrustedKey, verify_envelope};
    use serde_json::json;

    use super::*;

    /// A fixed in-memory KMS for every org test.
    pub fn use_test_kms() {
        set_kms(Arc::new(LocalKms::from_key([7u8; 32])));
    }

    pub fn trusted(signer: &OrgSigner) -> TrustedKey {
        TrustedKey {
            key_id: signer.key_id.clone(),
            public: signer.public,
        }
    }

    #[test]
    fn tenants_get_distinct_keys() {
        use_test_kms();
        let mut c = super::super::testing::db();
        let a_iss = signing_key(&mut c, "o1", KeyPurpose::Issuer, 1).unwrap();
        let a_dec = signing_key(&mut c, "o1", KeyPurpose::Decision, 1).unwrap();
        let b_iss = signing_key(&mut c, "o2", KeyPurpose::Issuer, 1).unwrap();
        assert_ne!(a_iss.public, a_dec.public);
        assert_ne!(a_iss.public, b_iss.public);
        // The same key the next time.
        assert_eq!(signing_key(&mut c, "o1", KeyPurpose::Issuer, 2).unwrap().public, a_iss.public);
        let keys = public_keys(&mut c, "o1", 3).unwrap();
        assert_eq!(keys.issuer.0, a_iss.key_id);
    }

    #[test]
    fn envelopes_verify_with_reins_audit() {
        use_test_kms();
        let mut c = super::super::testing::db();
        let env = envelope(&mut c, "o1", KeyPurpose::Decision, EnvelopeKind::Checkpoint, &json!({"b": 1, "a": 2}), 1)
            .unwrap();
        assert_eq!(env.body, r#"{"a":2,"b":1}"#);
        let dec = signing_key(&mut c, "o1", KeyPurpose::Decision, 1).unwrap();
        let iss = signing_key(&mut c, "o1", KeyPurpose::Issuer, 1).unwrap();
        assert!(verify_envelope(&env, &[trusted(&dec)]));
        assert!(!verify_envelope(&env, &[trusted(&iss)]), "the issuer key didn't sign it");
    }

    #[test]
    fn a_sealed_key_moved_to_another_tenant_does_not_open() {
        use_test_kms();
        let mut c = super::super::testing::db();
        signing_key(&mut c, "o1", KeyPurpose::Issuer, 1).unwrap();
        let row = reins_org_keys::q_key(&mut c, "o1", "issuer").unwrap().unwrap();
        reins_org_keys::q_insert_key(
            &mut c,
            &ReinsOrgKey {
                org_uuid: "o2".to_owned(),
                ..row
            },
        )
        .unwrap();
        assert!(signing_key(&mut c, "o2", KeyPurpose::Issuer, 2).is_err());
    }

    #[test]
    fn data_sealed_for_one_org_does_not_open_for_another() {
        use_test_kms();
        let mut c = super::super::testing::db();
        let (key_id, sealed) = seal_data(&mut c, "o1", "or_1/request", b"secret body", 1).unwrap();
        assert_eq!(&*open_data(&mut c, "o1", &key_id, "or_1/request", &sealed).unwrap(), b"secret body");
        assert!(open_data(&mut c, "o1", &key_id, "or_2/request", &sealed).is_err(), "bound to its request");
        assert!(open_data(&mut c, "o2", &key_id, "or_1/request", &sealed).is_err(), "another tenant");
    }

    #[test]
    fn a_wrong_master_key_cannot_open() {
        let a = LocalKms::from_key([1u8; 32]);
        let b = LocalKms::from_key([2u8; 32]);
        let sealed = a.seal(b"aad", b"x").unwrap();
        assert!(b.open(b"aad", &sealed).is_err());
        assert!(a.open(b"other", &sealed).is_err());
        assert_eq!(&*a.open(b"aad", &sealed).unwrap(), b"x");
        assert_ne!(a.id(), b.id());
        assert_eq!(LocalKms::with_id([1u8; 32], "aws:0123abcd".into()).id(), "aws:0123abcd");
    }

    fn test_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("reins-kms-{name}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_cloud_provider_with_nothing_installed_fails_closed() {
        let dir = test_dir("closed");
        let path = dir.join("master.key");
        let path = path.to_str().unwrap();
        let slot: RwLock<Option<Arc<dyn Kms>>> = RwLock::new(None);
        for provider in ["aws", "gcp", "azure", "unknown"] {
            let Err(OrgError::Internal(e)) = resolve(&slot, provider, path) else {
                panic!("{provider}: expected an error");
            };
            assert_eq!(e, "the org KMS is not unlocked");
        }
        assert!(!std::path::Path::new(path).exists(), "no local key was created");
        // Installed: used whatever the provider.
        *slot.write().unwrap() = Some(Arc::new(LocalKms::with_id([3u8; 32], "aws:x".into())));
        assert_eq!(resolve(&slot, "aws", path).unwrap().id(), "aws:x");
        // Local: the key file, created once.
        let local = RwLock::new(None);
        let id = resolve(&local, " Local ", path).unwrap().id();
        assert!(id.starts_with("local:"));
        assert_eq!(std::fs::read(path).unwrap().len(), 32);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
