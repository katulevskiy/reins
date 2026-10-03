//! The app's own key: an X25519 key pair made on first start and kept in the state directory (0600). The public half
//! is pinned on the phone at pairing; the phone seals every credential to it, so only this app can open them.

use std::path::Path;

use crypto_box::SecretKey;
use crypto_box::aead::OsRng;
use data_encoding::BASE64URL_NOPAD;
use reins_proto::desktop::{CredentialGrant, encode_key, key_fingerprint};
use zeroize::Zeroizing;

#[derive(Debug, thiserror::Error)]
pub enum IdentityError {
    #[error("identity key {path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
    #[error("identity key {0} is damaged; delete it and log in again")]
    Damaged(String),
    #[error("the phone's answer could not be opened with this app's key")]
    Unseal,
    #[error("the phone's answer is malformed")]
    Malformed,
}

pub struct Identity {
    secret: SecretKey,
}

impl std::fmt::Debug for Identity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Identity").field("public_key", &self.public_key()).finish_non_exhaustive()
    }
}

impl Identity {
    /// A fresh key that is not saved (tests).
    #[must_use]
    pub fn generate() -> Self {
        Self {
            secret: SecretKey::generate(&mut OsRng),
        }
    }

    /// The key at `path`, made (and saved 0600) when there is none.
    pub fn load_or_create(path: &Path) -> Result<Self, IdentityError> {
        let shown = path.display().to_string();
        match std::fs::read_to_string(path) {
            Ok(text) => {
                let raw = Zeroizing::new(
                    BASE64URL_NOPAD
                        .decode(text.trim().as_bytes())
                        .map_err(|_| IdentityError::Damaged(shown.clone()))?,
                );
                let bytes: [u8; 32] = raw.as_slice().try_into().map_err(|_| IdentityError::Damaged(shown))?;
                Ok(Self {
                    secret: SecretKey::from(bytes),
                })
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let id = Self::generate();
                let text = Zeroizing::new(BASE64URL_NOPAD.encode(&id.secret.to_bytes()));
                crate::config::write_private(path, text.as_bytes()).map_err(|source| IdentityError::Io {
                    path: shown,
                    source,
                })?;
                Ok(id)
            }
            Err(source) => Err(IdentityError::Io {
                path: shown,
                source,
            }),
        }
    }

    /// base64url, 32 bytes, no padding: what the phone pins.
    #[must_use]
    pub fn public_key(&self) -> String {
        encode_key(self.secret.public_key().as_bytes())
    }

    /// The eight digits the user compares with the phone ("4821 9930").
    #[must_use]
    pub fn fingerprint(&self) -> String {
        key_fingerprint(&self.public_key()).unwrap_or_default()
    }

    /// Opens a sealed box addressed to this app (base64url, no padding).
    pub fn unseal(&self, sealed: &str) -> Result<Zeroizing<Vec<u8>>, IdentityError> {
        let bytes = BASE64URL_NOPAD.decode(sealed.trim().as_bytes()).map_err(|_| IdentityError::Malformed)?;
        self.secret.unseal(&bytes).map(Zeroizing::new).map_err(|_| IdentityError::Unseal)
    }

    /// Opens the credential the phone sealed to this app.
    pub fn open_grant(&self, sealed: &str) -> Result<CredentialGrant, IdentityError> {
        let plain = self.unseal(sealed)?;
        serde_json::from_slice(&plain).map_err(|_| IdentityError::Malformed)
    }
}

/// Seals `plaintext` to a public key the way the phone does (tests and fakes).
pub fn seal_to(public_key: &str, plaintext: &[u8]) -> Option<String> {
    let raw = reins_proto::desktop::decode_key(public_key)?;
    let public = crypto_box::PublicKey::from(raw);
    public.seal(&mut OsRng, plaintext).ok().map(|b| BASE64URL_NOPAD.encode(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_key_survives_a_restart_and_opens_what_is_sealed_to_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("identity.key");
        let a = Identity::load_or_create(&path).unwrap();
        let b = Identity::load_or_create(&path).unwrap();
        assert_eq!(a.public_key(), b.public_key());
        assert_eq!(a.fingerprint().len(), 9);
        let grant = CredentialGrant {
            v: 1,
            nonce: "n1".into(),
            digest: None,
            repo: "o/r".into(),
            access: "read".into(),
            username: "x-access-token".into(),
            token: "t0k".into(),
            expires_at: 5,
        };
        let sealed = seal_to(&a.public_key(), &serde_json::to_vec(&grant).unwrap()).unwrap();
        assert_eq!(b.open_grant(&sealed).unwrap(), grant);
        assert!(matches!(Identity::generate().open_grant(&sealed), Err(IdentityError::Unseal)));
        assert!(matches!(a.open_grant("!!"), Err(IdentityError::Malformed)));
    }

    #[test]
    fn a_damaged_key_file_is_reported_not_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("identity.key");
        std::fs::write(&path, "nope").unwrap();
        assert!(matches!(Identity::load_or_create(&path), Err(IdentityError::Damaged(_))));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "nope");
    }
}
