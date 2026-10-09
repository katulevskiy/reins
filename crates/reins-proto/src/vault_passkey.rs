//! A passkey that opens the account's vault: "Unlock with passkey" on a phone signed in to an account whose keys it
//! cannot open (a reinstall, a new phone), in place of the other phone or the recovery code.
//!
//! The passkey's PRF extension gives the phone a secret only that passkey can reproduce (WebAuthn `prf`, from the
//! same salt every time: [`PRF_SALT_LABEL`]). A key derived from it seals the account secret (`reins-core`'s `sso`
//! module), and the server keeps the sealed copy per passkey ([`VaultPasskey`]): useless without the passkey, so any
//! signed-in session may read it. Adding or removing one changes how the vault can be opened, so it takes the same
//! proof as taking the approval role over: the master password hash the account secret derives.

use serde::{Deserialize, Serialize};

/// The PRF salt is SHA-256 of this: one fixed input, so unlocking needs no lookup before asking the passkey.
pub const PRF_SALT_LABEL: &str = "reins-vault-prf/v1";
/// Passkeys an account may keep for its vault.
pub const MAX_PASSKEYS: usize = 10;
/// Longest credential id accepted (bytes of its base64url text; WebAuthn ids are at most 1023 bytes).
pub const MAX_CREDENTIAL_ID_CHARS: usize = 1400;
/// Longest sealed secret accepted (bytes of its base64url text).
pub const MAX_WRAPPED_CHARS: usize = 512;
/// Longest name kept (characters).
pub const MAX_NAME_CHARS: usize = 64;

/// One passkey that opens the vault, as the server keeps it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultPasskey {
    /// The WebAuthn credential id, base64url without padding.
    pub credential_id: String,
    /// The account secret sealed with the key the passkey's PRF output derives, base64url without padding.
    pub wrapped: String,
    /// Where it was made ("Pixel 9"), for the list in Settings.
    pub name: String,
    pub created_at: i64,
}

/// `GET /reins/api/vault-passkeys`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultPasskeys {
    pub passkeys: Vec<VaultPasskey>,
}

/// `POST /reins/api/vault-passkeys`: adds (or replaces) the copy for one passkey.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewVaultPasskey {
    pub credential_id: String,
    pub wrapped: String,
    pub name: String,
    /// The account's master password hash, which only someone holding the account secret can make.
    pub master_password_hash: String,
}

/// `POST /reins/api/vault-passkeys/remove`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoveVaultPasskey {
    pub credential_id: String,
    pub master_password_hash: String,
}

/// Error codes of these routes.
pub mod codes {
    /// The account already keeps [`super::MAX_PASSKEYS`].
    pub const TOO_MANY: &str = "too_many_passkeys";
}

/// Whether `s` is base64url without padding and at most `max` characters, as ids and sealed copies are sent.
#[must_use]
pub fn is_base64url(s: &str, max: usize) -> bool {
    !s.is_empty() && s.len() <= max && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_unpadded_base64url_within_the_limit_is_accepted() {
        assert!(is_base64url("AbC-_09", 10));
        assert!(!is_base64url("", 10));
        assert!(!is_base64url("AbC=", 10));
        assert!(!is_base64url("a/b", 10));
        assert!(!is_base64url("abcdef", 5));
    }
}
