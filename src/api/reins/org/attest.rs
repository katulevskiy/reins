//! Verifying the platform attestations a phone brings when it enrolls its org device key (06 §6, O2), so posture
//! gating can require one ([`super::posture`]: `PostureRule.attested` needs [`attestation::VERIFIED`]).
//!
//! Each attestation must vouch for this enrollment of this key: [`reins_proto::org::attestation_binding`] of the
//! challenge and the enrolled key (SPKI DER), kept with the attestations as `{"binding": hex, "items": [...]}`.
//!
//! - **Android key attestation** ([`verify_android`]): the key's certificate chain, leaf first. Each certificate is
//!   signed by the next; the last is, or is signed by, a pinned root key (Google's hardware attestation roots, or
//!   `REINS_ATTEST_ANDROID_ROOTS`); the leaf certifies the enrolled key; its KeyDescription says the key lives in a TEE
//!   or StrongBox and carries this enrollment's challenge. The app can't attest the binding itself (the key doesn't
//!   exist until the attestation is made), so it attests the enrollment message, and the binding is checked as
//!   SHA-256(attestationChallenge ‖ leaf key), which is the same thing. Revocation (Google's status list) is not
//!   checked yet.
//! - **App Attest** ([`verify_app_attest`]), per Apple's "Validating apps that connect to your server": the app's
//!   client data hash is the binding itself. Needs `REINS_APP_ATTEST_APP_ID`; without it App Attest is recorded but
//!   not verified (counted as none).
//! - **Play Integrity** is recorded, not verified (the Google API verifier comes later).
//!
//! Certificate validity periods are not checked: attestation leaves carry the key's own dates (often long past or
//! far ahead), and the roots are pinned by key.

use std::sync::LazyLock;

use data_encoding::{BASE64, HEXLOWER};
use reins_proto::org::{Attestation, AttestationKind, enrollment_message};
use ring::signature::{self, UnparsedPublicKey, VerificationAlgorithm};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use x509_parser::{
    certificate::X509Certificate,
    der_parser::asn1_rs::{Any, Class, FromDer, Tag},
    extensions::X509Extension,
    parse_x509_certificate,
    pem::Pem,
    x509::SubjectPublicKeyInfo,
};

use super::enabled;
use crate::{
    CONFIG,
    db::{
        DbConnInner, DbPool,
        models::reins_org::{self, attestation},
    },
};

/// Android's KeyDescription extension.
const OID_KEY_DESCRIPTION: &str = "1.3.6.1.4.1.11129.2.1.17";
/// App Attest's nonce extension.
const OID_APP_ATTEST_NONCE: &str = "1.2.840.113635.100.8.2";
const OID_EC_KEY: &str = "1.2.840.10045.2.1";
const OID_RSA_KEY: &str = "1.2.840.113549.1.1.1";
const OID_P256: &str = "1.2.840.10045.3.1.7";
const OID_P384: &str = "1.3.132.0.34";
const OID_ECDSA_SHA256: &str = "1.2.840.10045.4.3.2";
const OID_ECDSA_SHA384: &str = "1.2.840.10045.4.3.3";
const OID_RSA_SHA256: &str = "1.2.840.113549.1.1.11";

/// The longest certificate chain read.
const MAX_CHAIN: usize = 8;
/// RootOfTrust's tag in an AuthorizationList.
const TAG_ROOT_OF_TRUST: u32 = 704;

/// Google's hardware attestation roots, from https://developer.android.com/privacy-and-security/security-key-attestation
/// ("Root certificates"): the RSA root (one key, reissued several times; this is the 2022–2042 certificate) and
/// "Key Attestation CA1", the ECDSA P-384 root new in 2025. Pinned by key.
const GOOGLE_ROOTS_PEM: &str = "-----BEGIN CERTIFICATE-----
MIIFHDCCAwSgAwIBAgIJAPHBcqaZ6vUdMA0GCSqGSIb3DQEBCwUAMBsxGTAXBgNV
BAUTEGY5MjAwOWU4NTNiNmIwNDUwHhcNMjIwMzIwMTgwNzQ4WhcNNDIwMzE1MTgw
NzQ4WjAbMRkwFwYDVQQFExBmOTIwMDllODUzYjZiMDQ1MIICIjANBgkqhkiG9w0B
AQEFAAOCAg8AMIICCgKCAgEAr7bHgiuxpwHsK7Qui8xUFmOr75gvMsd/dTEDDJdS
Sxtf6An7xyqpRR90PL2abxM1dEqlXnf2tqw1Ne4Xwl5jlRfdnJLmN0pTy/4lj4/7
tv0Sk3iiKkypnEUtR6WfMgH0QZfKHM1+di+y9TFRtv6y//0rb+T+W8a9nsNL/ggj
nar86461qO0rOs2cXjp3kOG1FEJ5MVmFmBGtnrKpa73XpXyTqRxB/M0n1n/W9nGq
C4FSYa04T6N5RIZGBN2z2MT5IKGbFlbC8UrW0DxW7AYImQQcHtGl/m00QLVWutHQ
oVJYnFPlXTcHYvASLu+RhhsbDmxMgJJ0mcDpvsC4PjvB+TxywElgS70vE0XmLD+O
JtvsBslHZvPBKCOdT0MS+tgSOIfga+z1Z1g7+DVagf7quvmag8jfPioyKvxnK/Eg
sTUVi2ghzq8wm27ud/mIM7AY2qEORR8Go3TVB4HzWQgpZrt3i5MIlCaY504LzSRi
igHCzAPlHws+W0rB5N+er5/2pJKnfBSDiCiFAVtCLOZ7gLiMm0jhO2B6tUXHI/+M
RPjy02i59lINMRRev56GKtcd9qO/0kUJWdZTdA2XoS82ixPvZtXQpUpuL12ab+9E
aDK8Z4RHJYYfCT3Q5vNAXaiWQ+8PTWm2QgBR/bkwSWc+NpUFgNPN9PvQi8WEg5Um
AGMCAwEAAaNjMGEwHQYDVR0OBBYEFDZh4QB8iAUJUYtEbEf/GkzJ6k8SMB8GA1Ud
IwQYMBaAFDZh4QB8iAUJUYtEbEf/GkzJ6k8SMA8GA1UdEwEB/wQFMAMBAf8wDgYD
VR0PAQH/BAQDAgIEMA0GCSqGSIb3DQEBCwUAA4ICAQB8cMqTllHc8U+qCrOlg3H7
174lmaCsbo/bJ0C17JEgMLb4kvrqsXZs01U3mB/qABg/1t5Pd5AORHARs1hhqGIC
W/nKMav574f9rZN4PC2ZlufGXb7sIdJpGiO9ctRhiLuYuly10JccUZGEHpHSYM2G
tkgYbZba6lsCPYAAP83cyDV+1aOkTf1RCp/lM0PKvmxYN10RYsK631jrleGdcdkx
oSK//mSQbgcWnmAEZrzHoF1/0gso1HZgIn0YLzVhLSA/iXCX4QT2h3J5z3znluKG
1nv8NQdxei2DIIhASWfu804CA96cQKTTlaae2fweqXjdN1/v2nqOhngNyz1361mF
mr4XmaKH/ItTwOe72NI9ZcwS1lVaCvsIkTDCEXdm9rCNPAY10iTunIHFXRh+7KPz
lHGewCq/8TOohBRn0/NNfh7uRslOSZ/xKbN9tMBtw37Z8d2vvnXq/YWdsm1+JLVw
n6yYD/yacNJBlwpddla8eaVMjsF6nBnIgQOf9zKSe06nSTqvgwUHosgOECZJZ1Eu
zbH4yswbt02tKtKEFhx+v+OTge/06V+jGsqTWLsfrOCNLuA8H++z+pUENmpqnnHo
vaI47gC+TNpkgYGkkBT6B/m/U01BuOBBTzhIlMEZq9qkDWuM2cA5kW5V3FJUcfHn
w1IdYIg2Wxg7yHcQZemFQg==
-----END CERTIFICATE-----
-----BEGIN CERTIFICATE-----
MIICIjCCAaigAwIBAgIRAISp0Cl7DrWK5/8OgN52BgUwCgYIKoZIzj0EAwMwUjEc
MBoGA1UEAwwTS2V5IEF0dGVzdGF0aW9uIENBMTEQMA4GA1UECwwHQW5kcm9pZDET
MBEGA1UECgwKR29vZ2xlIExMQzELMAkGA1UEBhMCVVMwHhcNMjUwNzE3MjIzMjE4
WhcNMzUwNzE1MjIzMjE4WjBSMRwwGgYDVQQDDBNLZXkgQXR0ZXN0YXRpb24gQ0Ex
MRAwDgYDVQQLDAdBbmRyb2lkMRMwEQYDVQQKDApHb29nbGUgTExDMQswCQYDVQQG
EwJVUzB2MBAGByqGSM49AgEGBSuBBAAiA2IABCPaI3FO3z5bBQo8cuiEas4HjqCt
G/mLFfRT0MsIssPBEEU5Cfbt6sH5yOAxqEi5QagpU1yX4HwnGb7OtBYpDTB57uH5
Eczm34A5FNijV3s0/f0UPl7zbJcTx6xwqMIRq6NCMEAwDwYDVR0TAQH/BAUwAwEB
/zAOBgNVHQ8BAf8EBAMCAQYwHQYDVR0OBBYEFFIyuyz7RkOb3NaBqQ5lZuA0QepA
MAoGCCqGSM49BAMDA2gAMGUCMETfjPO/HwqReR2CS7p0ZWoD/LHs6hDi422opifH
EUaYLxwGlT9SLdjkVpz0UUOR5wIxAIoGyxGKRHVTpqpGRFiJtQEOOTp/+s1GcxeY
uR2zh/80lQyu9vAFCj6E4AXc+osmRg==
-----END CERTIFICATE-----
";

/// The Apple App Attestation Root CA, from
/// https://www.apple.com/certificateauthority/Apple_App_Attestation_Root_CA.pem (SHA-256 fingerprint
/// 1C:B9:82:3B:A2:8B:A6:AD:2D:33:A0:06:94:1D:E2:AE:4F:51:3E:F1:D4:E8:31:B9:F7:E0:FA:7B:62:42:C9:32).
const APPLE_APP_ATTEST_ROOT_PEM: &str = "-----BEGIN CERTIFICATE-----
MIICITCCAaegAwIBAgIQC/O+DvHN0uD7jG5yH2IXmDAKBggqhkjOPQQDAzBSMSYw
JAYDVQQDDB1BcHBsZSBBcHAgQXR0ZXN0YXRpb24gUm9vdCBDQTETMBEGA1UECgwK
QXBwbGUgSW5jLjETMBEGA1UECAwKQ2FsaWZvcm5pYTAeFw0yMDAzMTgxODMyNTNa
Fw00NTAzMTUwMDAwMDBaMFIxJjAkBgNVBAMMHUFwcGxlIEFwcCBBdHRlc3RhdGlv
biBSb290IENBMRMwEQYDVQQKDApBcHBsZSBJbmMuMRMwEQYDVQQIDApDYWxpZm9y
bmlhMHYwEAYHKoZIzj0CAQYFK4EEACIDYgAERTHhmLW07ATaFQIEVwTtT4dyctdh
NbJhFs/Ii2FdCgAHGbpphY3+d8qjuDngIN3WVhQUBHAoMeQ/cLiP1sOUtgjqK9au
Yen1mMEvRq9Sk3Jm5X8U62H+xTD3FE9TgS41o0IwQDAPBgNVHRMBAf8EBTADAQH/
MB0GA1UdDgQWBBSskRBTM72+aEH/pwyp5frq5eWKoTAOBgNVHQ8BAf8EBAMCAQYw
CgYIKoZIzj0EAwMDaAAwZQIwQgFGnByvsiVbpTKwSga0kP0e8EeDS4+sQmTvb7vn
53O5+FRXgeLhpJ06ysC5PrOyAjEAp5U4xDgEgllF7En3VcE3iexZZtKeYnpqtijV
oyFraWVIyd/dganmrduC1bmTBGwD
-----END CERTIFICATE-----
";

/// Where the attested key lives.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SecurityLevel {
    Software,
    Tee,
    Strongbox,
    SecureEnclave,
}

/// What a verified attestation says.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Attested {
    /// `android` or `ios`.
    pub platform: &'static str,
    pub security: SecurityLevel,
    /// Android's verified boot state is `Verified` (RootOfTrust); `None` when not attested.
    pub verified_boot: Option<bool>,
    /// Android's bootloader is locked (RootOfTrust); `None` when not attested.
    pub locked: Option<bool>,
}

/// The roots and app id attestations are checked against.
pub struct Trust {
    /// Android key attestation roots (SPKI DER).
    pub android_roots: Vec<Vec<u8>>,
    /// App Attest roots (SPKI DER).
    pub apple_roots: Vec<Vec<u8>>,
    /// `TEAMID.bundle`; `None`: App Attest is not verified.
    pub app_id: Option<String>,
}

impl Trust {
    /// From the configuration, read once (a change needs a restart). An unreadable roots file pins nothing, so
    /// Android attestations then fail rather than pass.
    fn configured() -> Self {
        let path = CONFIG.reins_attest_android_roots();
        let android_roots = if path.trim().is_empty() {
            android_roots()
        } else {
            match std::fs::read(path.trim()) {
                Ok(pem) => spkis_of_pem(&pem),
                Err(e) => {
                    error!(
                        "REINS_ATTEST_ANDROID_ROOTS ({path}) is unreadable, so no Android attestation verifies: {e}"
                    );
                    Vec::new()
                }
            }
        };
        if android_roots.is_empty() {
            warn!("No Android attestation roots: Android key attestations will be recorded as failed");
        }
        let app_id = CONFIG.reins_app_attest_app_id();
        Self {
            android_roots,
            apple_roots: spkis_of_pem(APPLE_APP_ATTEST_ROOT_PEM.as_bytes()),
            app_id: Some(app_id.trim().to_owned()).filter(|a| !a.is_empty()),
        }
    }
}

static TRUST: LazyLock<Trust> = LazyLock::new(Trust::configured);

/// Google's hardware attestation root keys (SPKI DER).
pub fn android_roots() -> Vec<Vec<u8>> {
    spkis_of_pem(GOOGLE_ROOTS_PEM.as_bytes())
}

/// The public keys (SPKI DER) of the certificates in `pem`; what doesn't parse is skipped.
fn spkis_of_pem(pem: &[u8]) -> Vec<Vec<u8>> {
    Pem::iter_from_buffer(pem)
        .filter_map(Result::ok)
        .filter_map(|p| parse_x509_certificate(&p.contents).ok().map(|(_, c)| c.public_key().raw.to_vec()))
        .collect()
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    reins_proto::canon::sha256(bytes)
}

// ---------------------------------------------------------------------------------------------------------------
// Certificates
// ---------------------------------------------------------------------------------------------------------------

fn parse_cert(der: &[u8]) -> Result<X509Certificate<'_>, String> {
    match parse_x509_certificate(der) {
        Ok(([], cert)) => Ok(cert),
        _ => Err("a certificate does not parse".to_owned()),
    }
}

/// Whether `cert` is signed by the key `issuer_spki` (ECDSA P-256/P-384 with SHA-256/384, RSA PKCS#1 SHA-256).
fn signed_by(cert: &X509Certificate<'_>, issuer_spki: &[u8]) -> Result<(), String> {
    let Ok(([], spki)) = SubjectPublicKeyInfo::from_der(issuer_spki) else {
        return Err("an issuer key does not parse".to_owned());
    };
    let key = spki.algorithm.algorithm.to_id_string();
    let sig = cert.signature_algorithm.algorithm.to_id_string();
    let algorithm: &'static dyn VerificationAlgorithm = if key == OID_EC_KEY {
        let curve = spki.algorithm.parameters.as_ref().and_then(|p| p.as_oid().ok()).map(|o| o.to_id_string());
        match (curve.as_deref(), sig.as_str()) {
            (Some(OID_P256), OID_ECDSA_SHA256) => &signature::ECDSA_P256_SHA256_ASN1,
            (Some(OID_P256), OID_ECDSA_SHA384) => &signature::ECDSA_P256_SHA384_ASN1,
            (Some(OID_P384), OID_ECDSA_SHA256) => &signature::ECDSA_P384_SHA256_ASN1,
            (Some(OID_P384), OID_ECDSA_SHA384) => &signature::ECDSA_P384_SHA384_ASN1,
            _ => return Err(format!("unsupported signature {sig} by an EC key")),
        }
    } else if key == OID_RSA_KEY && sig == OID_RSA_SHA256 {
        &signature::RSA_PKCS1_2048_8192_SHA256
    } else {
        return Err(format!("unsupported signature {sig} by a {key} key"));
    };
    UnparsedPublicKey::new(algorithm, &spki.subject_public_key.data)
        .verify(cert.tbs_certificate.as_ref(), &cert.signature_value.data)
        .map_err(|_| "a certificate's signature does not verify".to_owned())
}

/// Parses `certs` (leaf first) and checks that each is signed by the next, and the last is, or is signed by, one of
/// `roots` (SPKI DER).
fn verify_chain<'a>(certs: &'a [Vec<u8>], roots: &[Vec<u8>]) -> Result<Vec<X509Certificate<'a>>, String> {
    if certs.is_empty() || certs.len() > MAX_CHAIN {
        return Err(format!("the chain has {} certificates", certs.len()));
    }
    let chain = certs.iter().map(|c| parse_cert(c)).collect::<Result<Vec<_>, _>>()?;
    for pair in chain.windows(2) {
        signed_by(&pair[0], pair[1].public_key().raw)?;
    }
    let last = &chain[chain.len() - 1];
    let pinned =
        roots.iter().any(|r| r.as_slice() == last.public_key().raw) || roots.iter().any(|r| signed_by(last, r).is_ok());
    if !pinned {
        return Err("the chain does not end in a pinned root".to_owned());
    }
    Ok(chain)
}

/// The extension `oid` of `cert`, if it has one (exactly one).
fn extension<'c>(cert: &'c X509Certificate<'_>, oid: &str) -> Result<Option<&'c X509Extension<'c>>, String> {
    let mut found = cert.extensions().iter().filter(|e| e.oid.to_id_string() == oid);
    let first = found.next();
    if found.next().is_some() {
        return Err(format!("the extension {oid} appears twice"));
    }
    Ok(first)
}

// ---------------------------------------------------------------------------------------------------------------
// DER
// ---------------------------------------------------------------------------------------------------------------

fn der_items(mut data: &[u8]) -> Result<Vec<Any<'_>>, String> {
    let mut items = Vec::new();
    while !data.is_empty() {
        let (rest, item) = Any::from_der(data).map_err(|_| "bad DER".to_owned())?;
        items.push(item);
        data = rest;
    }
    Ok(items)
}

/// The one DER object in `bytes`.
fn der_one(bytes: &[u8]) -> Result<Any<'_>, String> {
    match Any::from_der(bytes) {
        Ok(([], any)) => Ok(any),
        _ => Err("bad DER".to_owned()),
    }
}

/// The items of the SEQUENCE `bytes`.
fn der_sequence(bytes: &[u8]) -> Result<Vec<Any<'_>>, String> {
    let any = der_one(bytes)?;
    if any.header.class() != Class::Universal || any.header.tag() != Tag::Sequence {
        return Err("expected a SEQUENCE".to_owned());
    }
    der_items(any.data)
}

/// A small non-negative INTEGER or ENUMERATED.
fn der_small(any: &Any<'_>, tag: Tag) -> Result<u64, String> {
    if any.header.class() != Class::Universal || any.header.tag() != tag || any.data.is_empty() || any.data.len() > 8 {
        return Err(format!("expected a small {tag}"));
    }
    if any.data[0] & 0x80 != 0 {
        return Err(format!("a negative {tag}"));
    }
    Ok(any.data.iter().fold(0, |n, b| (n << 8) | u64::from(*b)))
}

fn der_octets<'a>(any: &Any<'a>) -> Result<&'a [u8], String> {
    if any.header.class() != Class::Universal || any.header.tag() != Tag::OctetString {
        return Err("expected an OCTET STRING".to_owned());
    }
    Ok(any.data)
}

// ---------------------------------------------------------------------------------------------------------------
// Android key attestation
// ---------------------------------------------------------------------------------------------------------------

/// What the KeyDescription says that matters here.
struct KeyDescription {
    attestation_security: u64,
    key_security: u64,
    challenge: Vec<u8>,
    /// (deviceLocked, verifiedBootState) from RootOfTrust in `hardwareEnforced`.
    root_of_trust: Option<(bool, u64)>,
}

/// KeyDescription ::= SEQUENCE { attestationVersion INTEGER, attestationSecurityLevel SecurityLevel,
/// keyMintVersion INTEGER, keyMintSecurityLevel SecurityLevel, attestationChallenge OCTET STRING, uniqueId OCTET
/// STRING, softwareEnforced AuthorizationList, hardwareEnforced AuthorizationList, ... }
fn key_description(der: &[u8]) -> Result<KeyDescription, String> {
    let items = der_sequence(der)?;
    if items.len() < 8 {
        return Err("the KeyDescription is short".to_owned());
    }
    let attestation_security = der_small(&items[1], Tag::Enumerated)?;
    let key_security = der_small(&items[3], Tag::Enumerated)?;
    let challenge = der_octets(&items[4])?.to_vec();
    let hardware = &items[7];
    if hardware.header.class() != Class::Universal || hardware.header.tag() != Tag::Sequence {
        return Err("the hardware-enforced list is not a SEQUENCE".to_owned());
    }
    let mut root_of_trust = None;
    for item in der_items(hardware.data)? {
        if item.header.class() == Class::ContextSpecific && item.header.tag().0 == TAG_ROOT_OF_TRUST {
            // RootOfTrust ::= SEQUENCE { verifiedBootKey OCTET STRING, deviceLocked BOOLEAN,
            //   verifiedBootState ENUMERATED, ... }
            let rot = der_sequence(item.data)?;
            if rot.len() < 3 || rot[1].header.tag() != Tag::Boolean || rot[1].data.len() != 1 {
                return Err("the RootOfTrust does not parse".to_owned());
            }
            root_of_trust = Some((rot[1].data[0] != 0, der_small(&rot[2], Tag::Enumerated)?));
        }
    }
    Ok(KeyDescription {
        attestation_security,
        key_security,
        challenge,
        root_of_trust,
    })
}

fn security_level(level: u64) -> Result<SecurityLevel, String> {
    match level {
        0 => Ok(SecurityLevel::Software),
        1 => Ok(SecurityLevel::Tee),
        2 => Ok(SecurityLevel::Strongbox),
        other => Err(format!("unknown security level {other}")),
    }
}

/// Whether an Android attestation challenge vouches for `binding`: the binding itself, or (what the app attests) the
/// enrollment message, which with the enrolled key hashes to the binding.
fn challenge_vouches(challenge: &[u8], binding: &[u8; 32], device_spki: &[u8]) -> bool {
    if challenge == binding {
        return true;
    }
    if !challenge.starts_with(&enrollment_message("")) {
        return false;
    }
    let mut m = challenge.to_vec();
    m.extend_from_slice(device_spki);
    sha256(&m) == *binding
}

/// Verifies an Android key attestation chain (DER certificates, base64, joined with `,`, leaf first) for the enrolled
/// key `device_spki` and `binding`, against `roots` (SPKI DER).
pub fn verify_android(
    chain_b64: &str,
    binding: &[u8; 32],
    device_spki: &[u8],
    roots: &[Vec<u8>],
) -> Result<Attested, String> {
    let certs = chain_b64
        .split(',')
        .map(|c| BASE64.decode(c.trim().as_bytes()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "a certificate is not base64".to_owned())?;
    let chain = verify_chain(&certs, roots)?;
    let leaf = &chain[0];
    if leaf.public_key().raw != device_spki {
        return Err("the attested key is not the enrolled key".to_owned());
    }
    // Only the leaf may be an attestation: a key that was itself attested could otherwise sign a made-up one.
    for issuer in &chain[1..] {
        if extension(issuer, OID_KEY_DESCRIPTION)?.is_some() {
            return Err("an issuer carries a key attestation".to_owned());
        }
    }
    let ext = extension(leaf, OID_KEY_DESCRIPTION)?.ok_or("the leaf has no key attestation")?;
    let description = key_description(ext.value)?;
    if !challenge_vouches(&description.challenge, binding, device_spki) {
        return Err("the attestation is not for this enrollment".to_owned());
    }
    let attestation_security = security_level(description.attestation_security)?;
    let security = security_level(description.key_security)?;
    if attestation_security == SecurityLevel::Software || security == SecurityLevel::Software {
        return Err("the key is attested by software, not secure hardware".to_owned());
    }
    Ok(Attested {
        platform: "android",
        security,
        verified_boot: description.root_of_trust.map(|(_, state)| state == 0),
        locked: description.root_of_trust.map(|(locked, _)| locked),
    })
}

// ---------------------------------------------------------------------------------------------------------------
// App Attest
// ---------------------------------------------------------------------------------------------------------------

/// The nonce in App Attest's extension: SEQUENCE { [1] EXPLICIT OCTET STRING }.
fn app_attest_nonce(der: &[u8]) -> Result<&[u8], String> {
    for item in der_sequence(der)? {
        if item.header.class() == Class::ContextSpecific && item.header.tag().0 == 1 {
            return der_octets(&der_one(item.data)?);
        }
    }
    Err("the nonce extension has no nonce".to_owned())
}

/// Verifies an App Attest attestation object (CBOR, base64) whose client data hash is `binding`, for the app
/// `app_id` (`TEAMID.bundle`), against `roots` (SPKI DER).
pub fn verify_app_attest(
    object_b64: &str,
    binding: &[u8; 32],
    app_id: &str,
    roots: &[Vec<u8>],
) -> Result<Attested, String> {
    let raw = BASE64.decode(object_b64.trim().as_bytes()).map_err(|_| "the attestation is not base64".to_owned())?;
    let object = Cbor::parse(&raw)?;
    if object.get("fmt").and_then(Cbor::text) != Some("apple-appattest") {
        return Err("not an App Attest attestation".to_owned());
    }
    let statement = object.get("attStmt").ok_or("the attestation has no statement")?;
    let x5c = statement
        .get("x5c")
        .and_then(Cbor::array)
        .ok_or("the attestation has no certificates")?
        .iter()
        .map(|c| c.bytes().map(<[u8]>::to_vec).ok_or("a certificate is not a byte string"))
        .collect::<Result<Vec<_>, _>>()?;
    let auth = object.get("authData").and_then(Cbor::bytes).ok_or("the attestation has no authenticator data")?;

    // 1. The certificates chain to Apple's App Attestation root.
    let chain = verify_chain(&x5c, roots)?;
    let leaf = &chain[0];
    // 2–4. The nonce is SHA-256(authData ‖ clientDataHash).
    let mut signed = auth.to_vec();
    signed.extend_from_slice(binding);
    let ext = extension(leaf, OID_APP_ATTEST_NONCE)?.ok_or("the certificate has no nonce")?;
    if app_attest_nonce(ext.value)? != sha256(&signed) {
        return Err("the attestation is not for this enrollment".to_owned());
    }
    // authData: rpIdHash (32) ‖ flags (1) ‖ counter (4) ‖ aaguid (16) ‖ credentialId length (2) ‖ credentialId ‖
    // credentialPublicKey (COSE).
    if auth.len() < 55 {
        return Err("the authenticator data is short".to_owned());
    }
    // 6. The app.
    if auth[..32] != sha256(app_id.as_bytes()) {
        return Err("the attestation is for another app".to_owned());
    }
    // 7. A new key.
    if auth[33..37] != [0, 0, 0, 0] {
        return Err("the attestation's counter is not 0".to_owned());
    }
    // 8. Production or development App Attest.
    if &auth[37..53] != b"appattest\0\0\0\0\0\0\0" && &auth[37..53] != b"appattestdevelop" {
        return Err("the attestation is not from App Attest".to_owned());
    }
    // 5, 9. The credential is the certificate's key (its key id is SHA-256 of the uncompressed point).
    let id_len = usize::from(u16::from_be_bytes([auth[53], auth[54]]));
    let id = auth.get(55..55 + id_len).ok_or("the authenticator data is short")?;
    let point: &[u8] = &leaf.public_key().subject_public_key.data;
    if id != sha256(point) {
        return Err("the credential is not the certified key".to_owned());
    }
    let (cose, _) = Cbor::parse_prefix(&auth[55 + id_len..])?;
    let coordinate = |label| cose.get_int(label).and_then(Cbor::bytes).unwrap_or_default();
    if [&[4][..], coordinate(-2), coordinate(-3)].concat() != point {
        return Err("the credential's public key is not the certified key".to_owned());
    }
    Ok(Attested {
        platform: "ios",
        security: SecurityLevel::SecureEnclave,
        verified_boot: None,
        locked: None,
    })
}

// ---------------------------------------------------------------------------------------------------------------
// CBOR (what App Attest uses: maps, arrays, byte and text strings, integers; definite lengths only)
// ---------------------------------------------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Cbor<'a> {
    Int(i128),
    Bytes(&'a [u8]),
    Text(&'a str),
    Array(Vec<Cbor<'a>>),
    Map(Vec<(Cbor<'a>, Cbor<'a>)>),
}

/// How deep a CBOR value may nest.
const MAX_CBOR_DEPTH: usize = 16;

impl<'a> Cbor<'a> {
    /// The one CBOR value in `bytes`.
    pub(crate) fn parse(bytes: &'a [u8]) -> Result<Self, String> {
        match Self::parse_prefix(bytes)? {
            (value, []) => Ok(value),
            _ => Err("bytes after the CBOR value".to_owned()),
        }
    }

    /// The CBOR value at the start of `bytes`, and what follows it.
    pub(crate) fn parse_prefix(bytes: &'a [u8]) -> Result<(Self, &'a [u8]), String> {
        Self::read(bytes, 0)
    }

    fn read(bytes: &'a [u8], depth: usize) -> Result<(Self, &'a [u8]), String> {
        const TRUNCATED: &str = "truncated CBOR";
        if depth > MAX_CBOR_DEPTH {
            return Err("CBOR nested too deep".to_owned());
        }
        let (&first, rest) = bytes.split_first().ok_or(TRUNCATED)?;
        let (major, info) = (first >> 5, first & 0x1f);
        let size = match info {
            0..=23 => 0,
            24 => 1,
            25 => 2,
            26 => 4,
            27 => 8,
            _ => return Err("indefinite or reserved CBOR lengths are not read".to_owned()),
        };
        let (arg, mut rest) = if size == 0 {
            (u64::from(info), rest)
        } else {
            let (arg, rest) = rest.split_at_checked(size).ok_or(TRUNCATED)?;
            (arg.iter().fold(0, |n, b| (n << 8) | u64::from(*b)), rest)
        };
        let len = || usize::try_from(arg).ok().filter(|n| *n <= bytes.len()).ok_or(TRUNCATED);
        let value = match major {
            0 => Self::Int(i128::from(arg)),
            1 => Self::Int(-1 - i128::from(arg)),
            2 | 3 => {
                let (data, after) = rest.split_at_checked(len()?).ok_or(TRUNCATED)?;
                rest = after;
                if major == 2 {
                    Self::Bytes(data)
                } else {
                    Self::Text(std::str::from_utf8(data).map_err(|_| "CBOR text is not UTF-8".to_owned())?)
                }
            }
            4 => {
                let n = len()?;
                let mut items = Vec::with_capacity(n);
                for _ in 0..n {
                    let (item, after) = Self::read(rest, depth + 1)?;
                    items.push(item);
                    rest = after;
                }
                Self::Array(items)
            }
            5 => {
                let n = len()?;
                let mut entries = Vec::with_capacity(n);
                for _ in 0..n {
                    let (key, after) = Self::read(rest, depth + 1)?;
                    let (value, after) = Self::read(after, depth + 1)?;
                    entries.push((key, value));
                    rest = after;
                }
                Self::Map(entries)
            }
            _ => return Err("unsupported CBOR type".to_owned()),
        };
        Ok((value, rest))
    }

    fn entry(&self, matches: impl Fn(&Self) -> bool) -> Option<&Self> {
        match self {
            Self::Map(entries) => entries.iter().find(|(k, _)| matches(k)).map(|(_, v)| v),
            _ => None,
        }
    }

    /// The value of the text key `key`, in a map.
    pub(crate) fn get(&self, key: &str) -> Option<&Self> {
        self.entry(|k| matches!(k, Self::Text(t) if *t == key))
    }

    /// The value of the integer key `key`, in a map.
    pub(crate) fn get_int(&self, key: i128) -> Option<&Self> {
        self.entry(|k| matches!(k, Self::Int(i) if *i == key))
    }

    pub(crate) fn bytes(&self) -> Option<&'a [u8]> {
        match self {
            Self::Bytes(b) => Some(b),
            _ => None,
        }
    }

    pub(crate) fn text(&self) -> Option<&'a str> {
        match self {
            Self::Text(t) => Some(t),
            _ => None,
        }
    }

    pub(crate) fn array(&self) -> Option<&[Self]> {
        match self {
            Self::Array(items) => Some(items),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Enrollment
// ---------------------------------------------------------------------------------------------------------------

/// What the attestations of an enrollment show: [`attestation::VERIFIED`] when one verifies, [`attestation::FAILED`]
/// when one is present but fails, [`attestation::NONE`] otherwise; and per attestation what was found.
pub fn verify_enrollment(atts: &[Attestation], binding: &[u8; 32], device_spki: &[u8]) -> (&'static str, Value) {
    verify_with(atts, binding, device_spki, &TRUST)
}

fn verify_with(atts: &[Attestation], binding: &[u8; 32], device_spki: &[u8], trust: &Trust) -> (&'static str, Value) {
    let mut attested = None;
    let mut failed = false;
    let mut items = Vec::new();
    for a in atts {
        let result = match a.kind {
            AttestationKind::AndroidKeyAttestation => {
                Some(verify_android(&a.data, binding, device_spki, &trust.android_roots))
            }
            AttestationKind::AppAttest => {
                trust.app_id.as_deref().map(|app_id| verify_app_attest(&a.data, binding, app_id, &trust.apple_roots))
            }
            AttestationKind::PlayIntegrity | AttestationKind::None => None,
        };
        items.push(match result {
            Some(Ok(found)) => {
                let item = json!({"kind": a.kind, "verified": found});
                attested.get_or_insert(found);
                item
            }
            Some(Err(error)) => {
                failed = true;
                json!({"kind": a.kind, "error": error})
            }
            None if a.kind == AttestationKind::None => json!({"kind": a.kind}),
            None => json!({"kind": a.kind, "unverified": a.kind}),
        });
    }
    let state = if attested.is_some() {
        attestation::VERIFIED
    } else if failed {
        attestation::FAILED
    } else {
        attestation::NONE
    };
    (state, json!({"attested": attested, "items": items}))
}

/// The binding and attestations an enrollment kept (`{"binding": hex, "items": [...]}`).
fn stored(attestations: &str) -> Option<([u8; 32], Vec<Attestation>)> {
    #[derive(Deserialize)]
    struct Stored {
        binding: String,
        #[serde(default)]
        items: Vec<Attestation>,
    }
    let stored: Stored = serde_json::from_str(attestations).ok()?;
    let binding = HEXLOWER.decode(stored.binding.as_bytes()).ok()?.try_into().ok()?;
    Some((binding, stored.items))
}

/// Verifies the devices enrolled before attestations were verified; how many it settled.
fn backfill_with(c: &mut DbConnInner, trust: &Trust) -> Result<usize, diesel::result::Error> {
    let mut settled = 0;
    for device in reins_org::q_unverified_devices(c)? {
        let Some((binding, items)) = stored(&device.attestations) else {
            continue;
        };
        let Ok(spki) = BASE64.decode(device.public_key.trim().as_bytes()) else {
            continue;
        };
        let (state, detail) = verify_with(&items, &binding, &spki, trust);
        if reins_org::q_set_attestation(c, &device, state, &detail.to_string())? {
            settled += 1;
        }
    }
    Ok(settled)
}

/// Once at launch: verifies the attestations of devices still `unverified` that kept their binding.
pub async fn backfill(pool: DbPool) {
    if !enabled() {
        return;
    }
    let conn = match pool.get().await {
        Ok(conn) => conn,
        Err(e) => {
            warn!("Org attestation backfill: no database connection: {e:?}");
            return;
        }
    };
    match conn.run(|c| backfill_with(c, &TRUST)).await {
        Ok(0) => {}
        Ok(n) => info!("Org attestation backfill: verified {n} enrolled devices"),
        Err(e) => warn!("Org attestation backfill: {e:?}"),
    }
}

#[cfg(test)]
mod tests {
    use openssl::{
        asn1::{Asn1Integer, Asn1Object, Asn1OctetString, Asn1Time},
        bn::{BigNum, BigNumContext},
        ec::{EcGroup, EcKey, PointConversionForm},
        hash::MessageDigest,
        nid::Nid,
        pkey::{PKey, Private},
        rsa::Rsa,
        x509::{X509, X509Builder, X509Extension, X509NameBuilder},
    };
    use reins_proto::org::attestation_binding;

    use super::*;

    const CHALLENGE: &str = "challenge-1";
    const APP_ID: &str = "TEAM123456.dev.reins.app";

    fn ec_key() -> PKey<Private> {
        let group = EcGroup::from_curve_name(Nid::X9_62_PRIME256V1).unwrap();
        PKey::from_ec_key(EcKey::generate(&group).unwrap()).unwrap()
    }

    fn rsa_key() -> PKey<Private> {
        PKey::from_rsa(Rsa::generate(2048).unwrap()).unwrap()
    }

    fn spki(key: &PKey<Private>) -> Vec<u8> {
        key.public_key_to_der().unwrap()
    }

    /// A certificate for `key` named `cn`, signed by `issuer` (self-signed without one), with `extensions` (OID, DER).
    fn cert(
        key: &PKey<Private>,
        cn: &str,
        issuer: Option<(&X509, &PKey<Private>)>,
        extensions: &[(&str, &[u8])],
    ) -> X509 {
        let mut name = X509NameBuilder::new().unwrap();
        name.append_entry_by_nid(Nid::COMMONNAME, cn).unwrap();
        let name = name.build();
        let mut b = X509Builder::new().unwrap();
        b.set_version(2).unwrap();
        let serial = Asn1Integer::from_bn(&BigNum::from_u32(7).unwrap()).unwrap();
        b.set_serial_number(&serial).unwrap();
        b.set_subject_name(&name).unwrap();
        b.set_issuer_name(issuer.map_or(&*name, |(c, _)| c.subject_name())).unwrap();
        b.set_pubkey(key).unwrap();
        b.set_not_before(&Asn1Time::days_from_now(0).unwrap()).unwrap();
        b.set_not_after(&Asn1Time::days_from_now(30).unwrap()).unwrap();
        for (oid, der) in extensions {
            let ext = X509Extension::new_from_der(
                &Asn1Object::from_str(oid).unwrap(),
                false,
                &Asn1OctetString::new_from_bytes(der).unwrap(),
            )
            .unwrap();
            b.append_extension(ext).unwrap();
        }
        b.sign(issuer.map_or(key, |(_, k)| k), MessageDigest::sha256()).unwrap();
        b.build()
    }

    fn tlv(tag: &[u8], content: &[u8]) -> Vec<u8> {
        let len = content.len();
        let mut out = tag.to_vec();
        if len < 0x80 {
            out.push(u8::try_from(len).unwrap());
        } else {
            out.extend_from_slice(&[0x82, u8::try_from(len >> 8).unwrap(), u8::try_from(len & 0xff).unwrap()]);
        }
        out.extend_from_slice(content);
        out
    }

    /// A KeyDescription with `challenge` at `security`, with a locked, verified boot.
    fn key_description_der(challenge: &[u8], security: u8) -> Vec<u8> {
        let root_of_trust = tlv(
            &[0x30],
            &[tlv(&[0x04], &[7; 32]), tlv(&[0x01], &[0xff]), tlv(&[0x0a], &[0]), tlv(&[0x04], &[9; 32])].concat(),
        );
        let hardware = tlv(&[0x30], &tlv(&[0xbf, 0x85, 0x40], &root_of_trust));
        tlv(
            &[0x30],
            &[
                tlv(&[0x02], &[100]),
                tlv(&[0x0a], &[security]),
                tlv(&[0x02], &[100]),
                tlv(&[0x0a], &[security]),
                tlv(&[0x04], challenge),
                tlv(&[0x04], &[]),
                tlv(&[0x30], &[]),
                hardware,
            ]
            .concat(),
        )
    }

    fn b64(certs: &[&X509]) -> String {
        certs.iter().map(|c| BASE64.encode(&c.to_der().unwrap())).collect::<Vec<_>>().join(",")
    }

    /// An Android-like chain: an RSA root, an EC intermediate, and a leaf for `leaf_key` attesting `challenge`.
    struct Android {
        root: PKey<Private>,
        leaf_key: PKey<Private>,
        chain: String,
    }

    fn android(challenge: &[u8], security: u8) -> Android {
        let root = rsa_key();
        let root_cert = cert(&root, "Test Root", None, &[]);
        let inter = ec_key();
        let inter_cert = cert(&inter, "Test Intermediate", Some((&root_cert, &root)), &[]);
        let leaf_key = ec_key();
        let ext = key_description_der(challenge, security);
        let leaf = cert(&leaf_key, "Android Keystore Key", Some((&inter_cert, &inter)), &[(OID_KEY_DESCRIPTION, &ext)]);
        Android {
            chain: b64(&[&leaf, &inter_cert, &root_cert]),
            root,
            leaf_key,
        }
    }

    /// What the app attests: the enrollment message of the challenge.
    fn app_challenge(challenge: &str) -> Vec<u8> {
        enrollment_message(challenge)
    }

    #[test]
    fn google_and_apple_roots_parse() {
        assert_eq!(android_roots().len(), 2);
        assert_eq!(spkis_of_pem(APPLE_APP_ATTEST_ROOT_PEM.as_bytes()).len(), 1);
    }

    #[test]
    fn android_a_chain_for_this_key_and_challenge_verifies() {
        let a = android(&app_challenge(CHALLENGE), 1);
        let key = spki(&a.leaf_key);
        let binding = attestation_binding(CHALLENGE, &key);
        let attested = verify_android(&a.chain, &binding, &key, &[spki(&a.root)]).unwrap();
        assert_eq!(
            attested,
            Attested {
                platform: "android",
                security: SecurityLevel::Tee,
                verified_boot: Some(true),
                locked: Some(true),
            }
        );
        // The binding itself as the challenge vouches too.
        let leaf_key = a.leaf_key;
        let root = rsa_key();
        let root_cert = cert(&root, "Root", None, &[]);
        let ext = key_description_der(&binding, 2);
        let leaf = cert(&leaf_key, "Key", Some((&root_cert, &root)), &[(OID_KEY_DESCRIPTION, &ext)]);
        let attested = verify_android(&b64(&[&leaf]), &binding, &key, &[spki(&root)]).unwrap();
        assert_eq!(attested.security, SecurityLevel::Strongbox);
    }

    #[test]
    fn android_another_challenge_fails() {
        let a = android(&app_challenge("challenge-2"), 1);
        let key = spki(&a.leaf_key);
        let err = verify_android(&a.chain, &attestation_binding(CHALLENGE, &key), &key, &[spki(&a.root)]).unwrap_err();
        assert!(err.contains("not for this enrollment"), "{err}");
    }

    #[test]
    fn android_another_leaf_key_fails() {
        let a = android(&app_challenge(CHALLENGE), 1);
        let other = spki(&ec_key());
        let err =
            verify_android(&a.chain, &attestation_binding(CHALLENGE, &other), &other, &[spki(&a.root)]).unwrap_err();
        assert!(err.contains("not the enrolled key"), "{err}");
    }

    #[test]
    fn android_an_unpinned_root_fails() {
        let a = android(&app_challenge(CHALLENGE), 1);
        let key = spki(&a.leaf_key);
        let binding = attestation_binding(CHALLENGE, &key);
        let err = verify_android(&a.chain, &binding, &key, &[spki(&rsa_key())]).unwrap_err();
        assert!(err.contains("pinned root"), "{err}");
        assert!(verify_android(&a.chain, &binding, &key, &[]).is_err());
        assert!(verify_android(&a.chain, &binding, &key, &android_roots()).is_err());
    }

    #[test]
    fn android_software_attestation_is_not_hardware() {
        let a = android(&app_challenge(CHALLENGE), 0);
        let key = spki(&a.leaf_key);
        let err = verify_android(&a.chain, &attestation_binding(CHALLENGE, &key), &key, &[spki(&a.root)]).unwrap_err();
        assert!(err.contains("software"), "{err}");
    }

    #[test]
    fn android_a_leaf_signed_by_an_attested_key_fails() {
        // A genuine attested key signs a made-up attestation for another key.
        let a = android(&app_challenge("anything"), 1);
        let genuine = BASE64.decode(a.chain.split(',').next().unwrap().as_bytes()).unwrap();
        let genuine = X509::from_der(&genuine).unwrap();
        let forged_key = ec_key();
        let key = spki(&forged_key);
        let ext = key_description_der(&app_challenge(CHALLENGE), 2);
        let forged = cert(&forged_key, "Forged", Some((&genuine, &a.leaf_key)), &[(OID_KEY_DESCRIPTION, &ext)]);
        let chain = format!("{},{}", b64(&[&forged]), a.chain);
        let err = verify_android(&chain, &attestation_binding(CHALLENGE, &key), &key, &[spki(&a.root)]).unwrap_err();
        assert!(err.contains("issuer carries"), "{err}");
    }

    // A minimal CBOR writer for the fixtures.
    fn cb_head(major: u8, n: u64) -> Vec<u8> {
        let m = major << 5;
        match n {
            0..24 => vec![m | u8::try_from(n).unwrap()],
            24..256 => vec![m | 0x18, u8::try_from(n).unwrap()],
            256..65536 => [&[m | 0x19][..], &u16::try_from(n).unwrap().to_be_bytes()].concat(),
            _ => [&[m | 0x1a][..], &u32::try_from(n).unwrap().to_be_bytes()].concat(),
        }
    }
    fn cb_int(i: i64) -> Vec<u8> {
        if i >= 0 {
            cb_head(0, i.unsigned_abs())
        } else {
            cb_head(1, (-1 - i).unsigned_abs())
        }
    }
    fn cb_bytes(b: &[u8]) -> Vec<u8> {
        [cb_head(2, b.len() as u64), b.to_vec()].concat()
    }
    fn cb_text(t: &str) -> Vec<u8> {
        [cb_head(3, t.len() as u64), t.as_bytes().to_vec()].concat()
    }
    fn cb_array(items: &[Vec<u8>]) -> Vec<u8> {
        [cb_head(4, items.len() as u64), items.concat()].concat()
    }
    fn cb_map(entries: &[(Vec<u8>, Vec<u8>)]) -> Vec<u8> {
        let mut out = cb_head(5, entries.len() as u64);
        for (k, v) in entries {
            out.extend_from_slice(k);
            out.extend_from_slice(v);
        }
        out
    }

    #[test]
    fn cbor_reader_reads_maps_bytes_and_text() {
        let long = "x".repeat(300);
        let encoded = cb_map(&[
            (cb_text("fmt"), cb_text("apple-appattest")),
            (cb_text("b"), cb_bytes(&[1, 2, 3])),
            (cb_int(-2), cb_array(&[cb_int(0), cb_int(23), cb_int(24), cb_int(70_000), cb_int(-7)])),
            (cb_int(300), cb_text(&long)),
        ]);
        let v = Cbor::parse(&encoded).unwrap();
        assert_eq!(v.get("fmt").and_then(Cbor::text), Some("apple-appattest"));
        assert_eq!(v.get("b").and_then(Cbor::bytes), Some(&[1u8, 2, 3][..]));
        assert_eq!(
            v.get_int(-2).and_then(Cbor::array).unwrap(),
            &[Cbor::Int(0), Cbor::Int(23), Cbor::Int(24), Cbor::Int(70_000), Cbor::Int(-7)]
        );
        assert_eq!(v.get_int(300).and_then(Cbor::text), Some(long.as_str()));
        assert_eq!(v.get("missing"), None);
        // Trailing bytes, truncation, indefinite lengths and a huge claimed length are refused.
        assert!(Cbor::parse(&[encoded.clone(), vec![0]].concat()).is_err());
        assert!(Cbor::parse(&encoded[..encoded.len() - 1]).is_err());
        assert!(Cbor::parse(&[0x9f, 0x01, 0xff]).is_err());
        assert!(Cbor::parse(&[0x5b, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]).is_err());
        assert!(Cbor::parse(&[0x9b, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]).is_err());
        assert!(Cbor::parse(&[0x81; 64]).is_err());
        let (first, rest) = Cbor::parse_prefix(&[0x01, 0x02]).unwrap();
        assert_eq!((first, rest), (Cbor::Int(1), &[0x02u8][..]));
    }

    /// An App Attest object made for `binding` and `app_id`, and its root's key.
    fn app_attest(binding: &[u8; 32], app_id: &str) -> (String, Vec<u8>) {
        let root = ec_key();
        let root_cert = cert(&root, "Test App Attestation Root CA", None, &[]);
        let inter = ec_key();
        let inter_cert = cert(&inter, "Test App Attestation CA 1", Some((&root_cert, &root)), &[]);
        let leaf_key = ec_key();
        let ec = leaf_key.ec_key().unwrap();
        let mut ctx = BigNumContext::new().unwrap();
        let point = ec.public_key().to_bytes(ec.group(), PointConversionForm::UNCOMPRESSED, &mut ctx).unwrap();
        let key_id = sha256(&point);
        let cose = cb_map(&[
            (cb_int(1), cb_int(2)),
            (cb_int(3), cb_int(-7)),
            (cb_int(-1), cb_int(1)),
            (cb_int(-2), cb_bytes(&point[1..33])),
            (cb_int(-3), cb_bytes(&point[33..])),
        ]);
        let auth =
            [&sha256(app_id.as_bytes())[..], &[0x40], &[0, 0, 0, 0], b"appattestdevelop", &[0, 32], &key_id, &cose]
                .concat();
        let nonce = sha256(&[&auth[..], binding].concat());
        let ext = tlv(&[0x30], &tlv(&[0xa1], &tlv(&[0x04], &nonce)));
        let leaf = cert(&leaf_key, "key id", Some((&inter_cert, &inter)), &[(OID_APP_ATTEST_NONCE, &ext)]);
        let object = cb_map(&[
            (cb_text("fmt"), cb_text("apple-appattest")),
            (
                cb_text("attStmt"),
                cb_map(&[
                    (
                        cb_text("x5c"),
                        cb_array(&[cb_bytes(&leaf.to_der().unwrap()), cb_bytes(&inter_cert.to_der().unwrap())]),
                    ),
                    (cb_text("receipt"), cb_bytes(b"receipt")),
                ]),
            ),
            (cb_text("authData"), cb_bytes(&auth)),
        ]);
        (BASE64.encode(&object), spki(&root))
    }

    #[test]
    fn app_attest_an_object_for_this_enrollment_verifies() {
        let binding = attestation_binding(CHALLENGE, &spki(&ec_key()));
        let (object, root) = app_attest(&binding, APP_ID);
        let attested = verify_app_attest(&object, &binding, APP_ID, std::slice::from_ref(&root)).unwrap();
        assert_eq!(attested.security, SecurityLevel::SecureEnclave);
        let err = verify_app_attest(&object, &binding, "TEAM123456.other", &[root]).unwrap_err();
        assert!(err.contains("another app"), "{err}");
        let err = verify_app_attest(&object, &binding, APP_ID, &[spki(&ec_key())]).unwrap_err();
        assert!(err.contains("pinned root"), "{err}");
    }

    #[test]
    fn app_attest_wrong_nonce_fails() {
        let key = spki(&ec_key());
        let (object, root) = app_attest(&attestation_binding("challenge-2", &key), APP_ID);
        let err = verify_app_attest(&object, &attestation_binding(CHALLENGE, &key), APP_ID, &[root]).unwrap_err();
        assert!(err.contains("not for this enrollment"), "{err}");
    }

    #[test]
    fn app_attest_bad_cbor_fails() {
        let binding = [1; 32];
        let roots = spkis_of_pem(APPLE_APP_ATTEST_ROOT_PEM.as_bytes());
        for object in
            [&b"not cbor at all"[..], &[0xa1, 0x63, b'f', b'm'], &cb_map(&[(cb_text("fmt"), cb_text("packed"))]), &[]]
        {
            assert!(verify_app_attest(&BASE64.encode(object), &binding, APP_ID, &roots).is_err());
        }
        assert!(verify_app_attest("%%%", &binding, APP_ID, &roots).is_err());
    }

    fn test_trust(android_root: Vec<u8>, apple_root: Vec<u8>, app_id: Option<&str>) -> Trust {
        Trust {
            android_roots: vec![android_root],
            apple_roots: vec![apple_root],
            app_id: app_id.map(str::to_owned),
        }
    }

    #[test]
    fn enrollment_state_follows_the_attestations() {
        let a = android(&app_challenge(CHALLENGE), 1);
        let key = spki(&a.leaf_key);
        let binding = attestation_binding(CHALLENGE, &key);
        let trust = test_trust(spki(&a.root), Vec::new(), None);
        let att = |kind, data: &str| Attestation {
            kind,
            data: data.to_owned(),
        };

        let (state, detail) =
            verify_with(&[att(AttestationKind::AndroidKeyAttestation, &a.chain)], &binding, &key, &trust);
        assert_eq!(state, attestation::VERIFIED);
        assert_eq!(detail["attested"]["security"], "tee");
        assert_eq!(detail["attested"]["locked"], true);

        let other = spki(&ec_key());
        let (state, detail) =
            verify_with(&[att(AttestationKind::AndroidKeyAttestation, &a.chain)], &binding, &other, &trust);
        assert_eq!(state, attestation::FAILED);
        assert!(detail["items"][0]["error"].is_string());

        let (state, detail) = verify_with(&[att(AttestationKind::PlayIntegrity, "token")], &binding, &key, &trust);
        assert_eq!(state, attestation::NONE);
        assert_eq!(detail["items"][0]["unverified"], "play_integrity");

        // App Attest without an app id is recorded, not verified.
        let (state, _) = verify_with(&[att(AttestationKind::AppAttest, "garbage")], &binding, &key, &trust);
        assert_eq!(state, attestation::NONE);
        let (state, _) = verify_with(&[att(AttestationKind::None, "")], &binding, &key, &trust);
        assert_eq!(state, attestation::NONE);
        let (state, _) = verify_with(&[], &binding, &key, &trust);
        assert_eq!(state, attestation::NONE);

        let (object, apple_root) = app_attest(&binding, APP_ID);
        let trust = test_trust(spki(&a.root), apple_root, Some(APP_ID));
        let (state, _) = verify_with(&[att(AttestationKind::AppAttest, &object)], &binding, &key, &trust);
        assert_eq!(state, attestation::VERIFIED);
        let (state, _) = verify_with(&[att(AttestationKind::AppAttest, "garbage")], &binding, &key, &trust);
        assert_eq!(state, attestation::FAILED);
    }

    #[cfg(sqlite)]
    #[test]
    fn backfill_verifies_devices_enrolled_before_verification() {
        use crate::db::models::reins_org::ReinsOrgDevice;

        let mut c = crate::api::reins::org::testing::db();
        crate::api::reins::org::testing::add_org(&mut c, "o1");
        let a = android(&app_challenge(CHALLENGE), 1);
        let key = spki(&a.leaf_key);
        let binding = attestation_binding(CHALLENGE, &key);
        let items = vec![Attestation {
            kind: AttestationKind::AndroidKeyAttestation,
            data: a.chain.clone(),
        }];
        let device = |id: &str, attestations: String| ReinsOrgDevice {
            org_uuid: "o1".to_owned(),
            user_uuid: "u1".to_owned(),
            device_uuid: id.to_owned(),
            key_algorithm: "p256".to_owned(),
            public_key: BASE64.encode(&key),
            hardware: true,
            attestations,
            posture: "{}".to_owned(),
            posture_at: 1,
            enrolled_at: 1,
            revoked_at: None,
            attestation_state: attestation::UNVERIFIED.to_owned(),
            attestation_detail: None,
        };
        let kept = json!({"binding": HEXLOWER.encode(&binding), "items": items}).to_string();
        for row in [device("d1", kept), device("d2", "[]".to_owned())] {
            assert!(reins_org::q_enroll_device(&mut c, &row).unwrap());
        }
        let trust = test_trust(spki(&a.root), Vec::new(), None);
        assert_eq!(backfill_with(&mut c, &trust).unwrap(), 1);
        let d1 = reins_org::q_device(&mut c, "o1", "u1", "d1").unwrap().unwrap();
        assert_eq!(d1.attestation_state, attestation::VERIFIED);
        assert!(d1.attestation_detail.unwrap().contains("\"tee\""));
        // Without a kept binding there is nothing to check against: it stays unverified.
        let d2 = reins_org::q_device(&mut c, "o1", "u1", "d2").unwrap().unwrap();
        assert_eq!(d2.attestation_state, attestation::UNVERIFIED);
        assert_eq!(backfill_with(&mut c, &trust).unwrap(), 0);
    }
}
