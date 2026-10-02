//! Master-password key derivation (contracts §B) and the data-encryption key
//! (DEK) protecting secrets in the local store (contracts §E).

use std::fmt;
use std::num::NonZeroU32;

use aes::Aes256;
use argon2::{Algorithm, Argon2, Params, Version};
use cbc::cipher::block_padding::Pkcs7;
use cbc::cipher::{BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use data_encoding::BASE64;
use ring::aead::{AES_256_GCM, Aad, LessSafeKey, NONCE_LEN, Nonce, UnboundKey};
use ring::rand::{SecureRandom, SystemRandom};
use ring::{digest, hmac, pbkdf2};
use zeroize::Zeroizing;

use crate::CoreError;

/// Bounds accepted from the server. The lower bounds are the Bitwarden SDK's
/// minimums; the upper bounds stop a hostile server from exhausting the phone.
const PBKDF2_ITERATIONS: std::ops::RangeInclusive<u32> = 5_000..=10_000_000;
const ARGON2_ITERATIONS: std::ops::RangeInclusive<u32> = 1..=10;
const ARGON2_MEMORY_MIB: std::ops::RangeInclusive<u32> = 15..=1024;
const ARGON2_PARALLELISM: std::ops::RangeInclusive<u32> = 1..=16;

/// Length of the store's data-encryption key.
pub const DEK_LEN: usize = 32;

/// An account's KDF, as reported by `/identity/accounts/prelogin`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kdf {
    Pbkdf2 {
        iterations: u32,
    },
    /// `memory_mib` is in MiB, as the server reports it.
    Argon2id {
        iterations: u32,
        memory_mib: u32,
        parallelism: u32,
    },
}

impl Kdf {
    /// Builds a KDF from prelogin values (`kdf`: 0 = PBKDF2-SHA256, 1 = Argon2id).
    pub fn from_prelogin(
        kdf: i32,
        iterations: u32,
        memory_mib: Option<u32>,
        parallelism: Option<u32>,
    ) -> Result<Self, CoreError> {
        let unsupported = || CoreError::invalid("the server reported unsupported key-derivation settings");
        match kdf {
            0 if PBKDF2_ITERATIONS.contains(&iterations) => Ok(Self::Pbkdf2 {
                iterations,
            }),
            1 => {
                let memory_mib = memory_mib.ok_or_else(unsupported)?;
                let parallelism = parallelism.ok_or_else(unsupported)?;
                if ARGON2_ITERATIONS.contains(&iterations)
                    && ARGON2_MEMORY_MIB.contains(&memory_mib)
                    && ARGON2_PARALLELISM.contains(&parallelism)
                {
                    Ok(Self::Argon2id {
                        iterations,
                        memory_mib,
                        parallelism,
                    })
                } else {
                    Err(unsupported())
                }
            }
            _ => Err(unsupported()),
        }
    }
}

/// The 32-byte master key. Zeroized on drop; never printed.
pub struct MasterKey(Zeroizing<[u8; 32]>);

impl fmt::Debug for MasterKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("MasterKey(<redacted>)")
    }
}

/// Derives the master key. The salt is the trimmed, lower-cased email
/// (Bitwarden SDK behaviour); Argon2id salts with its SHA-256.
/// CPU-heavy: call from `tokio::task::spawn_blocking`.
pub fn master_key(password: &str, email: &str, kdf: Kdf) -> Result<MasterKey, CoreError> {
    let salt = email.trim().to_lowercase();
    let mut out = Zeroizing::new([0u8; 32]);
    match kdf {
        Kdf::Pbkdf2 {
            iterations,
        } => {
            let iterations = NonZeroU32::new(iterations).ok_or_else(|| CoreError::invalid("zero KDF iterations"))?;
            pbkdf2::derive(pbkdf2::PBKDF2_HMAC_SHA256, iterations, salt.as_bytes(), password.as_bytes(), &mut *out);
        }
        Kdf::Argon2id {
            iterations,
            memory_mib,
            parallelism,
        } => {
            let salt = digest::digest(&digest::SHA256, salt.as_bytes());
            let params = Params::new(memory_mib * 1024, iterations, parallelism, Some(32))
                .map_err(|e| CoreError::invalid(format!("Argon2 parameters: {e}")))?;
            Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
                .hash_password_into(password.as_bytes(), salt.as_ref(), &mut *out)
                .map_err(|e| CoreError::invalid(format!("Argon2: {e}")))?;
        }
    }
    Ok(MasterKey(out))
}

/// A vault key: the AES-256-CBC key and the HMAC-SHA256 key that protect Bitwarden "EncStrings".
pub struct VaultKey {
    enc: Zeroizing<[u8; 32]>,
    mac: Zeroizing<[u8; 32]>,
}

impl fmt::Debug for VaultKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("VaultKey(<redacted>)")
    }
}

impl VaultKey {
    /// From the 64 bytes `enc || mac` (the form the user key has once decrypted).
    pub fn from_bytes(raw: &[u8]) -> Result<Self, CoreError> {
        let (enc, mac) = raw.split_at_checked(32).ok_or_else(|| CoreError::invalid("unsupported vault key"))?;
        let (Ok(enc), Ok(mac)) = (<[u8; 32]>::try_from(enc), <[u8; 32]>::try_from(mac)) else {
            return Err(CoreError::invalid("unsupported vault key"));
        };
        Ok(Self {
            enc: Zeroizing::new(enc),
            mac: Zeroizing::new(mac),
        })
    }

    /// `enc || mac`, to keep the key sealed in the store.
    pub fn to_bytes(&self) -> Zeroizing<Vec<u8>> {
        let mut out = Zeroizing::new(Vec::with_capacity(64));
        out.extend_from_slice(&self.enc[..]);
        out.extend_from_slice(&self.mac[..]);
        out
    }

    /// Decrypts `2.<iv>|<data>|<mac>` (AES-256-CBC + HMAC-SHA256, the only form current clients write). The MAC is
    /// checked first, in constant time.
    pub fn decrypt(&self, enc_string: &str) -> Result<Zeroizing<Vec<u8>>, CoreError> {
        let bad = || CoreError::invalid("the vault could not be decrypted");
        let rest = enc_string.strip_prefix("2.").ok_or_else(bad)?;
        let mut parts = rest.split('|');
        let (Some(iv), Some(data), Some(mac), None) = (parts.next(), parts.next(), parts.next(), parts.next()) else {
            return Err(bad());
        };
        let (iv, data, mac) = (
            BASE64.decode(iv.as_bytes()).map_err(|_| bad())?,
            BASE64.decode(data.as_bytes()).map_err(|_| bad())?,
            BASE64.decode(mac.as_bytes()).map_err(|_| bad())?,
        );
        let mut signed = iv.clone();
        signed.extend_from_slice(&data);
        hmac::verify(&hmac::Key::new(hmac::HMAC_SHA256, &self.mac[..]), &signed, &mac).map_err(|_| bad())?;
        let plain = cbc::Decryptor::<Aes256>::new_from_slices(&self.enc[..], &iv)
            .map_err(|_| bad())?
            .decrypt_padded_vec_mut::<Pkcs7>(&data)
            .map_err(|_| bad())?;
        Ok(Zeroizing::new(plain))
    }

    /// Decrypts to text.
    pub fn decrypt_text(&self, enc_string: &str) -> Result<Zeroizing<String>, CoreError> {
        let plain = self.decrypt(enc_string)?;
        String::from_utf8(plain.to_vec())
            .map(Zeroizing::new)
            .map_err(|_| CoreError::invalid("the vault could not be decrypted"))
    }

    /// Encrypts into `2.<iv>|<data>|<mac>`.
    pub fn encrypt(&self, plaintext: &[u8]) -> Result<String, CoreError> {
        let iv = random_bytes::<16>()?;
        let data = cbc::Encryptor::<Aes256>::new_from_slices(&self.enc[..], &iv)
            .map_err(|_| CoreError::invalid("unsupported vault key"))?
            .encrypt_padded_vec_mut::<Pkcs7>(plaintext);
        let mut signed = iv.to_vec();
        signed.extend_from_slice(&data);
        let mac = hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, &self.mac[..]), &signed);
        Ok(format!("2.{}|{}|{}", BASE64.encode(&iv), BASE64.encode(&data), BASE64.encode(mac.as_ref())))
    }
}

impl MasterKey {
    /// HKDF-Expand of the master key into the two keys that protect the user key (`info` = "enc" and "mac"; the
    /// output is one block long, so it is a single HMAC).
    pub fn stretch(&self) -> VaultKey {
        let prk = hmac::Key::new(hmac::HMAC_SHA256, &self.0[..]);
        let expand = |info: &[u8]| {
            let mut input = info.to_vec();
            input.push(1);
            let mut out = Zeroizing::new([0u8; 32]);
            out.copy_from_slice(hmac::sign(&prk, &input).as_ref());
            out
        };
        VaultKey {
            enc: expand(b"enc"),
            mac: expand(b"mac"),
        }
    }
}

/// `base64(PBKDF2-SHA256(key = master_key, salt = password, 1 iteration))`,
/// the value sent as `password` to `/identity/connect/token` (contracts §B.3).
pub fn master_password_hash(key: &MasterKey, password: &str) -> Zeroizing<String> {
    let mut out = Zeroizing::new([0u8; 32]);
    pbkdf2::derive(pbkdf2::PBKDF2_HMAC_SHA256, NonZeroU32::MIN, password.as_bytes(), &key.0[..], &mut *out);
    Zeroizing::new(BASE64.encode(&out[..]))
}

/// Cryptographically secure random bytes.
pub fn random_bytes<const N: usize>() -> Result<[u8; N], CoreError> {
    let mut out = [0u8; N];
    SystemRandom::new().fill(&mut out).map_err(|_| CoreError::storage("system random generator failed"))?;
    Ok(out)
}

/// AES-256-GCM data-encryption key for secrets in the local store.
///
/// A sealed blob is `nonce (12 bytes) || ciphertext || tag (16 bytes)`. The AAD
/// names the column (and row) the blob belongs to, so blobs cannot be moved
/// between places.
pub struct Dek(LessSafeKey);

impl fmt::Debug for Dek {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Dek(<redacted>)")
    }
}

impl Dek {
    /// A fresh random key and its raw bytes (to be wrapped by `KeyWrapper`).
    pub fn generate() -> Result<(Self, Zeroizing<Vec<u8>>), CoreError> {
        let raw = Zeroizing::new(random_bytes::<DEK_LEN>()?.to_vec());
        Ok((Self::from_bytes(&raw)?, raw))
    }

    pub fn from_bytes(raw: &[u8]) -> Result<Self, CoreError> {
        if raw.len() != DEK_LEN {
            return Err(CoreError::storage("data key has the wrong length"));
        }
        let key = UnboundKey::new(&AES_256_GCM, raw).map_err(|_| CoreError::storage("invalid data key"))?;
        Ok(Self(LessSafeKey::new(key)))
    }

    pub fn seal(&self, aad: &str, plaintext: &[u8]) -> Result<Vec<u8>, CoreError> {
        let nonce = random_bytes::<NONCE_LEN>()?;
        let mut buf = plaintext.to_vec();
        self.0
            .seal_in_place_append_tag(Nonce::assume_unique_for_key(nonce), Aad::from(aad.as_bytes()), &mut buf)
            .map_err(|_| CoreError::storage("encryption failed"))?;
        let mut out = Vec::with_capacity(NONCE_LEN + buf.len());
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&buf);
        Ok(out)
    }

    pub fn open(&self, aad: &str, blob: &[u8]) -> Result<Vec<u8>, CoreError> {
        let fail = || CoreError::storage("stored data cannot be decrypted");
        if blob.len() < NONCE_LEN + AES_256_GCM.tag_len() {
            return Err(fail());
        }
        let (nonce, sealed) = blob.split_at(NONCE_LEN);
        let nonce = Nonce::try_assume_unique_for_key(nonce).map_err(|_| fail())?;
        let mut buf = sealed.to_vec();
        let plain = self.0.open_in_place(nonce, Aad::from(aad.as_bytes()), &mut buf).map_err(|_| fail())?;
        Ok(plain.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Vectors from the Bitwarden SDK (bitwarden-crypto: keys/kdf.rs and
    // keys/master_key.rs tests), reproduced independently on 2026-09-29.
    const PBKDF2_MASTER_KEY: [u8; 32] = [
        31, 79, 104, 226, 150, 71, 177, 90, 194, 80, 172, 209, 17, 129, 132, 81, 138, 167, 69, 167, 254, 149, 2, 27,
        39, 197, 64, 42, 22, 195, 86, 75,
    ];
    const ARGON2_MASTER_KEY: [u8; 32] = [
        207, 240, 225, 177, 162, 19, 163, 76, 98, 106, 179, 175, 224, 9, 17, 240, 20, 147, 237, 47, 246, 150, 141, 184,
        62, 225, 131, 242, 51, 53, 225, 242,
    ];

    fn argon(iterations: u32, memory_mib: u32, parallelism: u32) -> Kdf {
        Kdf::Argon2id {
            iterations,
            memory_mib,
            parallelism,
        }
    }

    #[test]
    fn pbkdf2_master_key_vector() {
        let key = master_key(
            "67t9b5g67$%Dh89n",
            "test_key",
            Kdf::Pbkdf2 {
                iterations: 10_000,
            },
        )
        .unwrap();
        assert_eq!(*key.0, PBKDF2_MASTER_KEY);
    }

    #[test]
    fn argon2id_master_key_vector() {
        let key = master_key("67t9b5g67$%Dh89n", "test_key", argon(4, 32, 2)).unwrap();
        assert_eq!(*key.0, ARGON2_MASTER_KEY);
    }

    #[test]
    fn pbkdf2_password_hash_vector_and_email_normalization() {
        for email in ["test@bitwarden.com", "TEST@bitwarden.com", " test@bitwarden.com"] {
            let key = master_key(
                "asdfasdf",
                email,
                Kdf::Pbkdf2 {
                    iterations: 100_000,
                },
            )
            .unwrap();
            assert_eq!(&*master_password_hash(&key, "asdfasdf"), "wmyadRMyBZOH7P/a/ucTCbSghKgdzDpPqUnu/DAVtSw=");
        }
    }

    #[test]
    fn argon2id_password_hash_vector() {
        let key = master_key("asdfasdf", "test_salt", argon(4, 32, 2)).unwrap();
        assert_eq!(&*master_password_hash(&key, "asdfasdf"), "PR6UjYmjmppTYcdyTiNbAhPJuQQOmynKbdEl1oyi/iQ=");
    }

    #[test]
    fn prelogin_parameters_are_bounded() {
        assert_eq!(
            Kdf::from_prelogin(0, 600_000, None, None).unwrap(),
            Kdf::Pbkdf2 {
                iterations: 600_000
            }
        );
        assert_eq!(Kdf::from_prelogin(1, 3, Some(64), Some(4)).unwrap(), argon(3, 64, 4));
        for (kdf, it, mem, par) in [
            (0, 4_999, None, None),
            (0, 10_000_001, None, None),
            (1, 3, None, Some(4)),
            (1, 3, Some(64), None),
            (1, 0, Some(64), Some(4)),
            (1, 3, Some(2048), Some(4)),
            (1, 3, Some(64), Some(17)),
            (2, 600_000, None, None),
        ] {
            assert!(Kdf::from_prelogin(kdf, it, mem, par).is_err(), "accepted {kdf} {it} {mem:?} {par:?}");
        }
    }

    #[test]
    fn secrets_are_redacted_in_debug() {
        let key = master_key(
            "pw",
            "a@b.com",
            Kdf::Pbkdf2 {
                iterations: 5_000,
            },
        )
        .unwrap();
        assert_eq!(format!("{key:?}"), "MasterKey(<redacted>)");
        let (dek, _) = Dek::generate().unwrap();
        assert_eq!(format!("{dek:?}"), "Dek(<redacted>)");
    }

    #[test]
    fn dek_round_trip_and_binding() {
        let (dek, raw) = Dek::generate().unwrap();
        let blob = dek.seal("session.refresh_token", b"secret").unwrap();
        assert_eq!(dek.open("session.refresh_token", &blob).unwrap(), b"secret");
        assert_ne!(dek.seal("session.refresh_token", b"secret").unwrap(), blob, "fresh nonce per seal");
        assert!(dek.open("audit.detail", &blob).is_err(), "AAD binds the blob to its column");
        let mut tampered = blob.clone();
        *tampered.last_mut().unwrap() ^= 1;
        assert!(dek.open("session.refresh_token", &tampered).is_err());
        assert!(dek.open("session.refresh_token", &blob[..20]).is_err());
        let (other, _) = Dek::generate().unwrap();
        assert!(other.open("session.refresh_token", &blob).is_err(), "another key never decrypts");
        let again = Dek::from_bytes(&raw).unwrap();
        assert_eq!(again.open("session.refresh_token", &blob).unwrap(), b"secret");
        assert!(Dek::from_bytes(&raw[..31]).is_err());
    }
}
