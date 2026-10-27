//! The tokens a family's guardians connected for its members (P7): kept only here, in `user_tokens.json` in the state
//! directory, encrypted with AES-256-GCM under a key derived (HKDF-SHA256, info `reins/user-tokens/v1`) from the
//! executor's own key file, owner-only. Each token is filed under its provider and member: one member's token never
//! serves another.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Mutex,
};

use reins_proto::managed::OAuthProvider;
use ring::{
    aead::{AES_256_GCM, Aad, LessSafeKey, NONCE_LEN, Nonce, UnboundKey},
    hkdf::{HKDF_SHA256, KeyType, Salt},
    rand::{SecureRandom, SystemRandom},
};
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};

/// The derivation's info, and the file's associated data.
pub const INFO: &[u8] = b"reins/user-tokens/v1";
const FILE: &str = "user_tokens.json";

/// One member's grant.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserToken {
    /// Google: the refresh token. GitHub: the OAuth token.
    pub token: String,
    /// The connected account (an address or a login).
    pub account: String,
    pub at: i64,
}

impl Drop for UserToken {
    fn drop(&mut self) {
        self.token.zeroize();
    }
}

#[derive(Default, Serialize, Deserialize)]
struct Stored {
    #[serde(default)]
    tokens: BTreeMap<String, UserToken>,
}

pub struct UserTokens {
    path: PathBuf,
    key: LessSafeKey,
    map: Mutex<BTreeMap<String, UserToken>>,
}

struct Len32;

impl KeyType for Len32 {
    fn len(&self) -> usize {
        32
    }
}

/// The file key: HKDF-SHA256 of `secret` (the executor's key file), info [`INFO`].
fn derive(secret: &[u8]) -> Result<LessSafeKey, String> {
    let prk = Salt::new(HKDF_SHA256, &[]).extract(secret);
    let okm = prk.expand(&[INFO], Len32).map_err(|_| "deriving the token key failed".to_owned())?;
    let mut raw = Zeroizing::new([0u8; 32]);
    okm.fill(raw.as_mut()).map_err(|_| "deriving the token key failed".to_owned())?;
    let key = UnboundKey::new(&AES_256_GCM, raw.as_ref()).map_err(|_| "the token key is not usable".to_owned())?;
    Ok(LessSafeKey::new(key))
}

fn slot(provider: OAuthProvider, member: &str) -> String {
    format!("{}/{}", provider.as_str(), member.trim().to_lowercase())
}

impl UserTokens {
    /// Opens (or starts) the store in `state_dir`, keyed by `secret` (the executor's key file's bytes).
    pub fn open(state_dir: &Path, secret: &[u8]) -> Result<Self, String> {
        let key = derive(secret)?;
        let path = state_dir.join(FILE);
        let map = match std::fs::read(&path) {
            Ok(bytes) => {
                if bytes.len() < NONCE_LEN {
                    return Err(format!("{} is damaged", path.display()));
                }
                let (nonce, sealed) = bytes.split_at(NONCE_LEN);
                let mut buf = Zeroizing::new(sealed.to_vec());
                let nonce = Nonce::try_assume_unique_for_key(nonce).map_err(|_| "bad nonce".to_owned())?;
                let plain = key
                    .open_in_place(nonce, Aad::from(INFO), buf.as_mut())
                    .map_err(|_| format!("{} doesn't open with this executor's key", path.display()))?;
                serde_json::from_slice::<Stored>(plain).map_err(|_| format!("{} is damaged", path.display()))?.tokens
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(e) => return Err(format!("reading {}: {e}", path.display())),
        };
        Ok(Self {
            path,
            key,
            map: Mutex::new(map),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, UserToken>> {
        self.map.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn save(&self, map: &BTreeMap<String, UserToken>) -> Result<(), String> {
        let stored = Stored {
            tokens: map.clone(),
        };
        let mut buf = Zeroizing::new(serde_json::to_vec(&stored).map_err(|e| e.to_string())?);
        let mut nonce = [0u8; NONCE_LEN];
        SystemRandom::new().fill(&mut nonce).map_err(|_| "no randomness".to_owned())?;
        let mut out = nonce.to_vec();
        let mut sealed = buf.to_vec();
        buf.zeroize();
        self.key
            .seal_in_place_append_tag(Nonce::assume_unique_for_key(nonce), Aad::from(INFO), &mut sealed)
            .map_err(|_| "sealing the tokens failed".to_owned())?;
        out.extend_from_slice(&sealed);
        crate::leases::write_private(&self.path, &out)
    }

    /// `member`'s token for `provider`.
    pub fn get(&self, provider: OAuthProvider, member: &str) -> Option<UserToken> {
        self.lock().get(&slot(provider, member)).cloned()
    }

    pub fn put(&self, provider: OAuthProvider, member: &str, token: UserToken) -> Result<(), String> {
        let mut map = self.lock();
        map.insert(slot(provider, member), token);
        self.save(&map)
    }

    /// Forgets `member`'s token for `provider` (returned, to revoke it upstream).
    pub fn remove(&self, provider: OAuthProvider, member: &str) -> Result<Option<UserToken>, String> {
        let mut map = self.lock();
        let gone = map.remove(&slot(provider, member));
        if gone.is_some() {
            self.save(&map)?;
        }
        Ok(gone)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token(t: &str) -> UserToken {
        UserToken {
            token: t.to_owned(),
            account: "sam@example.com".to_owned(),
            at: 1,
        }
    }

    #[test]
    fn tokens_at_rest_are_encrypted() {
        let dir = tempfile::tempdir().unwrap();
        let store = UserTokens::open(dir.path(), b"executor key bytes").unwrap();
        store.put(OAuthProvider::Google, "Sam@Example.com", token("1//refresh-SECRET")).unwrap();
        let raw = std::fs::read(dir.path().join(FILE)).unwrap();
        assert!(!raw.windows(6).any(|w| w == b"SECRET"), "never in clear");
        assert!(!raw.windows(7).any(|w| w == b"example"), "not even whose it is");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join(FILE)).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        // It opens again with the same key, and not with another.
        let again = UserTokens::open(dir.path(), b"executor key bytes").unwrap();
        assert_eq!(again.get(OAuthProvider::Google, "sam@example.com").unwrap().token, "1//refresh-SECRET");
        assert!(UserTokens::open(dir.path(), b"another key").is_err());
    }

    #[test]
    fn a_token_is_filed_under_its_member_and_provider() {
        let dir = tempfile::tempdir().unwrap();
        let store = UserTokens::open(dir.path(), b"k").unwrap();
        store.put(OAuthProvider::Google, "sam@example.com", token("a")).unwrap();
        assert!(store.get(OAuthProvider::Google, "ann@example.com").is_none());
        assert!(store.get(OAuthProvider::Github, "sam@example.com").is_none());
        assert_eq!(store.remove(OAuthProvider::Google, "sam@example.com").unwrap().unwrap().token, "a");
        assert!(store.get(OAuthProvider::Google, "sam@example.com").is_none());
    }
}
