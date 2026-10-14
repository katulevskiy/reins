//! Writing to Reins Secrets from this computer: what `reins secrets set/push/…` and import use to put values into the
//! vault on the phone.
//!
//! [`VaultWriter`] is the interface; `PhoneVaultWriter` (next to the release client) carries it out through the phone
//! (`vault_secrets_store` and `integration_token_store`, each value in a box only the phone opens, one approval per
//! call), and [`MemoryWriter`] keeps what it is given, for tests.
//!
//! The caller shows the check digits before it stores ([`check_digits`]): the phone shows the same four, so that a
//! batch an AI on the computer sent in the user's place does not pass for theirs.

use std::sync::Mutex;

pub use reins_proto::desktop::{EnvChanges, EnvStored};
use zeroize::Zeroizing;

/// What the phone did with a store, environment by environment.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StoreReceipt {
    pub envs: Vec<EnvStored>,
}

/// A git host's token for the phone's own integration (`github`, `gitlab`, `codeberg`, `bitbucket`).
#[derive(Clone, PartialEq, Eq)]
pub struct IntegrationToken {
    pub service: String,
    /// The host it was found for (`github.com`).
    pub host: String,
    /// What is known of its scopes (`repo`, `read:org`); shown on the phone, not trusted.
    pub scopes: Vec<String>,
    pub token: Zeroizing<String>,
}

impl std::fmt::Debug for IntegrationToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IntegrationToken")
            .field("service", &self.service)
            .field("host", &self.host)
            .field("scopes", &self.scopes)
            .finish_non_exhaustive()
    }
}

/// The account the phone added the token as.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenReceipt {
    pub service: String,
    pub account: String,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum StoreError {
    /// Denied on the phone (with the reason it gave, labelled as relayed).
    #[error("{0}")]
    Denied(String),
    /// No answer in time; approving on the phone and trying again works.
    #[error("{0}")]
    Waiting(String),
    /// The phone's key has never been checked on this computer, and this process cannot ask the user to type its
    /// digits (no terminal, no digits given).
    #[error(
        "this computer has not checked your phone's key yet: run `reins vault list` once in a terminal, or pass the \
         twelve digits your phone shows"
    )]
    PhoneKeyNeeded,
    /// The phone could not open the box: a new phone, or Reins installed again (`reins vault add --new-phone`).
    #[error(
        "your phone could not open it, so it is not the phone this computer knows. With a new phone (or Reins installed \
         again), run `reins vault list --new-phone`"
    )]
    PhoneChanged,
    /// Anything else (not logged in, the changes are not valid, the server or the phone failed).
    #[error("{0}")]
    Failed(String),
}

/// Puts values into the vault on the phone. Each call is one approval.
#[async_trait::async_trait]
pub trait VaultWriter: Send + Sync {
    /// Sets, removes, renames and copies variables of one or more environments (folders and items are made when
    /// missing; replaced and removed values stay in the item's history).
    async fn store_envs(&self, envs: &[EnvChanges]) -> Result<StoreReceipt, StoreError>;

    /// Hands a git host's token to the phone's integration of that host.
    async fn store_integration_token(&self, token: &IntegrationToken) -> Result<TokenReceipt, StoreError>;
}

/// The four check digits of a store, to show before it is sent ([`reins_proto::desktop::secrets_check`]).
#[must_use]
pub fn check_digits(envs: &[EnvChanges]) -> String {
    reins_proto::desktop::secrets_check(envs)
}

/// A [`VaultWriter`] that keeps what it is given (tests, previews).
#[derive(Debug, Default)]
pub struct MemoryWriter {
    pub stored: Mutex<Vec<EnvChanges>>,
    pub tokens: Mutex<Vec<IntegrationToken>>,
}

#[async_trait::async_trait]
impl VaultWriter for MemoryWriter {
    async fn store_envs(&self, envs: &[EnvChanges]) -> Result<StoreReceipt, StoreError> {
        reins_proto::desktop::validate_store(envs).map_err(StoreError::Failed)?;
        let receipt = StoreReceipt {
            envs: envs
                .iter()
                .map(|e| EnvStored {
                    project: e.project.clone(),
                    env: e.env.clone(),
                    added: u32::try_from(e.set.len()).unwrap_or(u32::MAX),
                    removed: u32::try_from(e.unset.len()).unwrap_or(u32::MAX),
                    renamed: u32::try_from(e.rename.len()).unwrap_or(u32::MAX),
                    ..EnvStored::default()
                })
                .collect(),
        };
        self.stored.lock().unwrap_or_else(std::sync::PoisonError::into_inner).extend(envs.iter().cloned());
        Ok(receipt)
    }

    async fn store_integration_token(&self, token: &IntegrationToken) -> Result<TokenReceipt, StoreError> {
        self.tokens.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(token.clone());
        Ok(TokenReceipt {
            service: token.service.clone(),
            account: "memory".to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_memory_writer_checks_and_keeps_what_it_is_given() {
        let w = MemoryWriter::default();
        let mut env = EnvChanges::new("acme", "dev");
        env.set.push(("A".into(), "1".into()));
        let r = w.store_envs(std::slice::from_ref(&env)).await.unwrap();
        assert_eq!((r.envs[0].env.as_str(), r.envs[0].added), ("dev", 1));
        assert_eq!(w.stored.lock().unwrap().len(), 1);
        assert!(matches!(w.store_envs(&[]).await, Err(StoreError::Failed(_))));
        assert_eq!(check_digits(&[env]).len(), 4);
        let token = IntegrationToken {
            service: "github".into(),
            host: "github.com".into(),
            scopes: vec!["repo".into()],
            token: Zeroizing::new("ghp_x".into()),
        };
        assert!(!format!("{token:?}").contains("ghp_x"));
        assert_eq!(w.store_integration_token(&token).await.unwrap().service, "github");
    }
}
