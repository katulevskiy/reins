//! A passkey that opens the account's vault (see `reins_proto::vault_passkey`). The apps do the WebAuthn part with
//! the platform's passkey UI (Credential Manager, AuthenticationServices) from [`VaultPasskeyOptions`] and hand back
//! the credential id and its PRF output; everything about keys happens here.
//!
//! The PRF output never leaves the phone: a key derived from it (HKDF-SHA256, bound to the account) seals the account
//! secret, and only that sealed copy goes to the server, bound to the account and the credential.

use data_encoding::BASE64URL_NOPAD;
use reins_proto::vault_passkey::{
    MAX_CREDENTIAL_ID_CHARS, MAX_NAME_CHARS, MAX_PASSKEYS, NewVaultPasskey, PRF_SALT_LABEL, RemoveVaultPasskey,
    VaultPasskeys, codes,
};
use reqwest::Method;
use ring::{digest, hkdf};
use zeroize::Zeroizing;

use crate::CoreError;
use crate::crypto::{self, Dek};
use crate::engine::Engine;
use crate::phone_api::{ApiFailure, PhoneApi};
use crate::session::{Session, api_call};
use crate::sso::AccountSecret;

/// Everything the app needs to ask the platform for a passkey (to make one, or to use one).
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct VaultPasskeyOptions {
    /// The relying party: the server's host (`app.reins2fa.com`), which lists the app in its association files.
    pub rp_id: String,
    /// WebAuthn `user.id`: the account's server id.
    pub user_handle: Vec<u8>,
    /// WebAuthn `user.name` and `displayName`: the account's email.
    pub user_name: String,
    /// A fresh challenge. Nothing verifies the assertion (the sealed copy is useless without the PRF output), but the
    /// platform wants one.
    pub challenge: Vec<u8>,
    /// The PRF input (`eval.first`), the same for every passkey.
    pub prf_salt: Vec<u8>,
    /// The passkeys the account has for its vault already: excluded when making one, allowed when unlocking.
    pub credential_ids: Vec<Vec<u8>>,
}

/// One passkey that opens the vault, for Settings.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct VaultPasskeyView {
    pub credential_id: Vec<u8>,
    pub name: String,
    pub created_at: i64,
}

/// The PRF input: SHA-256 of [`PRF_SALT_LABEL`].
pub fn prf_salt() -> Vec<u8> {
    digest::digest(&digest::SHA256, PRF_SALT_LABEL.as_bytes()).as_ref().to_vec()
}

/// The key that seals the account secret for one passkey, from its PRF output (32 bytes), bound to the account.
fn passkey_key(prf: &[u8], user_id: &str) -> Result<Dek, CoreError> {
    if prf.len() != 32 {
        return Err(CoreError::invalid("This passkey did not give the secret Reins needs. Try another passkey."));
    }
    let prk = hkdf::Salt::new(hkdf::HKDF_SHA256, b"reins-vault-passkey/v1").extract(prf);
    let info = [user_id.as_bytes()];
    let mut raw = Zeroizing::new([0u8; 32]);
    prk.expand(&info, hkdf::HKDF_SHA256)
        .and_then(|okm| okm.fill(&mut raw[..]))
        .map_err(|_| CoreError::storage("could not derive the passkey's key"))?;
    Dek::from_bytes(&raw[..])
}

/// Binds a sealed copy to the account and the credential it was made for: a copy moved to another passkey or another
/// account does not open.
fn aad(user_id: &str, credential_id: &str) -> String {
    format!("reins.vault-passkey.v1:{user_id}:{credential_id}")
}

pub(crate) fn seal_secret(
    secret: &AccountSecret,
    prf: &[u8],
    user_id: &str,
    credential_id: &str,
) -> Result<String, CoreError> {
    let sealed = passkey_key(prf, user_id)?.seal(&aad(user_id, credential_id), secret.as_bytes())?;
    Ok(BASE64URL_NOPAD.encode(&sealed))
}

pub(crate) fn open_secret(
    wrapped: &str,
    prf: &[u8],
    user_id: &str,
    credential_id: &str,
) -> Result<AccountSecret, CoreError> {
    let sealed = BASE64URL_NOPAD
        .decode(wrapped.as_bytes())
        .map_err(|_| CoreError::invalid("The server sent a damaged passkey copy."))?;
    let raw = Zeroizing::new(
        passkey_key(prf, user_id)?
            .open(&aad(user_id, credential_id), &sealed)
            .map_err(|_| CoreError::invalid("This passkey does not open this account's vault."))?,
    );
    AccountSecret::from_bytes(&raw)
}

impl PhoneApi<'_> {
    async fn vault_passkeys(&self) -> Result<VaultPasskeys, ApiFailure> {
        Self::json(self.request(Method::GET, "/vault-passkeys")).await
    }

    async fn add_vault_passkey(&self, new: &NewVaultPasskey) -> Result<VaultPasskeys, ApiFailure> {
        Self::json(self.request(Method::POST, "/vault-passkeys").json(new)).await
    }

    async fn remove_vault_passkey(&self, remove: &RemoveVaultPasskey) -> Result<VaultPasskeys, ApiFailure> {
        Self::json(self.request(Method::POST, "/vault-passkeys/remove").json(remove)).await
    }
}

fn credential_text(credential_id: &[u8]) -> Result<String, CoreError> {
    let text = BASE64URL_NOPAD.encode(credential_id);
    if credential_id.is_empty() || text.len() > MAX_CREDENTIAL_ID_CHARS {
        return Err(CoreError::invalid("That is not a passkey Reins can use."));
    }
    Ok(text)
}

fn views(list: VaultPasskeys) -> Vec<VaultPasskeyView> {
    list.passkeys
        .into_iter()
        .filter_map(|p| {
            Some(VaultPasskeyView {
                credential_id: BASE64URL_NOPAD.decode(p.credential_id.as_bytes()).ok()?,
                name: p.name,
                created_at: p.created_at,
            })
        })
        .collect()
}

impl Engine {
    async fn passkey_list(&self, session: &Session) -> Result<VaultPasskeys, CoreError> {
        api_call!(session, |api| api.vault_passkeys()).map_err(ApiFailure::into_core)
    }

    /// The master password hash the account secret derives: the proof adding or removing a passkey takes.
    async fn secret_proof(secret: AccountSecret) -> Result<Zeroizing<String>, CoreError> {
        tokio::task::spawn_blocking(move || -> Result<Zeroizing<String>, CoreError> {
            let master = secret.master_key()?;
            Ok(secret.master_password_hash(&master))
        })
        .await
        .map_err(|_| CoreError::storage("key derivation was interrupted"))?
    }

    fn held_secret(&self, user_id: &str) -> Result<AccountSecret, CoreError> {
        self.secret_of(user_id)?.ok_or_else(|| {
            CoreError::invalid(
                "Open the vault on this phone first: a passkey can only be added where the vault is open.",
            )
        })
    }

    pub async fn vault_passkey_options(&self) -> Result<VaultPasskeyOptions, CoreError> {
        let session = self.session()?;
        let user_id = session.account_user_id().await?;
        let list = self.passkey_list(&session).await?;
        let rp_id = url::Url::parse(session.server.as_str())
            .ok()
            .and_then(|u| u.host_str().map(str::to_owned))
            .ok_or_else(|| CoreError::invalid("This server has no host name a passkey can belong to."))?;
        Ok(VaultPasskeyOptions {
            rp_id,
            user_handle: user_id.into_bytes(),
            user_name: session.email(),
            challenge: crypto::random_bytes::<32>()?.to_vec(),
            prf_salt: prf_salt(),
            credential_ids: views(list).into_iter().map(|v| v.credential_id).collect(),
        })
    }

    pub async fn vault_passkeys(&self) -> Result<Vec<VaultPasskeyView>, CoreError> {
        let session = self.session()?;
        Ok(views(self.passkey_list(&session).await?))
    }

    /// Keeps a sealed copy of the account secret for the passkey just made (or used): from now on it opens the vault.
    pub async fn add_vault_passkey(
        &self,
        credential_id: &[u8],
        prf: &[u8],
        name: &str,
    ) -> Result<Vec<VaultPasskeyView>, CoreError> {
        let session = self.session()?;
        let user_id = session.account_user_id().await?;
        let secret = self.held_secret(&user_id)?;
        let credential_id = credential_text(credential_id)?;
        let wrapped = seal_secret(&secret, prf, &user_id, &credential_id)?;
        // Check the copy opens before the server keeps it.
        open_secret(&wrapped, prf, &user_id, &credential_id)?;
        let proof = Self::secret_proof(secret).await?;
        let name: String = name.trim().chars().take(MAX_NAME_CHARS).collect();
        let new = NewVaultPasskey {
            credential_id,
            wrapped,
            name: if name.is_empty() {
                "Passkey".to_owned()
            } else {
                name
            },
            master_password_hash: proof.to_string(),
        };
        let list = api_call!(&session, |api| api.add_vault_passkey(&new)).map_err(|e| match e {
            ApiFailure::Status {
                code,
                ..
            } if code == codes::TOO_MANY => {
                CoreError::invalid(format!("An account can keep {MAX_PASSKEYS} passkeys. Remove one first."))
            }
            other => other.into_core(),
        })?;
        Ok(views(list))
    }

    pub async fn remove_vault_passkey(&self, credential_id: &[u8]) -> Result<Vec<VaultPasskeyView>, CoreError> {
        let session = self.session()?;
        let user_id = session.account_user_id().await?;
        let secret = self.held_secret(&user_id)?;
        let remove = RemoveVaultPasskey {
            credential_id: credential_text(credential_id)?,
            master_password_hash: Self::secret_proof(secret).await?.to_string(),
        };
        let list = api_call!(&session, |api| api.remove_vault_passkey(&remove)).map_err(ApiFailure::into_core)?;
        Ok(views(list))
    }

    /// Opens the vault on a phone that cannot yet, with a passkey the account added before: the copy the server keeps
    /// for it gives back the account secret, which then works as the recovery code does.
    pub async fn unlock_with_vault_passkey(&self, credential_id: &[u8], prf: &[u8]) -> Result<(), CoreError> {
        let session = self.session()?;
        let user_id = session.account_user_id().await?;
        let credential_id = credential_text(credential_id)?;
        let list = self.passkey_list(&session).await?;
        let saved = list
            .passkeys
            .into_iter()
            .find(|p| p.credential_id == credential_id)
            .ok_or_else(|| CoreError::invalid("This passkey was not added to open this account's vault."))?;
        let secret = open_secret(&saved.wrapped, prf, &user_id, &credential_id)?;
        self.adopt_secret(secret).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secret() -> AccountSecret {
        AccountSecret::from_bytes(&[9; 32]).unwrap()
    }

    #[test]
    fn a_sealed_secret_opens_only_with_its_passkey_account_and_credential() {
        let prf = [1u8; 32];
        let wrapped = seal_secret(&secret(), &prf, "user-a", "cred-1").unwrap();
        assert_eq!(open_secret(&wrapped, &prf, "user-a", "cred-1").unwrap().as_bytes(), secret().as_bytes());
        assert!(open_secret(&wrapped, &[2u8; 32], "user-a", "cred-1").is_err(), "another passkey's output");
        assert!(open_secret(&wrapped, &prf, "user-b", "cred-1").is_err(), "another account");
        assert!(open_secret(&wrapped, &prf, "user-a", "cred-2").is_err(), "moved to another credential");
        assert!(seal_secret(&secret(), &[1u8; 16], "user-a", "cred-1").is_err(), "a PRF output of the wrong size");
    }

    #[test]
    fn the_prf_salt_is_fixed() {
        assert_eq!(prf_salt(), digest::digest(&digest::SHA256, b"reins-vault-prf/v1").as_ref());
        assert_eq!(prf_salt().len(), 32);
    }
}
