//! The vault on the phone itself: the list, an item's fields, and adding, changing and deleting items. This is the
//! user's own hand on their vault, so nothing here is asked for: the screens behind it unlock with the phone's screen
//! lock. The work is done by the same code as the vault tools ([`crate::connector::vault`]), which encrypts every string
//! on the phone before it reaches the server.
//!
//! Each item also says how the desktop app refers to it (`vault:OpenAI/password` for `reins run` and the API proxy, the
//! SSH agent for SSH keys), so that people know what to name things.

use std::sync::Arc;

use reins_proto::connector::VAULT;

use crate::CoreError;
use crate::connector::vault::{Vault, editor};
use crate::engine::Engine;

/// The kind of a vault item.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum VaultItemKind {
    Login,
    Note,
    Card,
    Identity,
    SshKey,
}

/// One item of the list.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct VaultItemSummary {
    pub id: String,
    pub name: String,
    pub kind: VaultItemKind,
    /// A login's username or website, a card's holder, an SSH key's fingerprint; empty when there is none.
    pub subtitle: String,
    pub favorite: bool,
}

/// One field of an item as the phone shows it.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct VaultField {
    /// What [`VaultFieldInput::key`] and `vault_reveal` call it: `username`, `password`, `notes`, `uris`,
    /// `private_key`, `number`, ..., or `custom:<name>` for a custom field.
    pub key: String,
    pub label: String,
    /// The value of a field that is not secret; `None` for a secret one (`vault_reveal` gives it).
    pub value: Option<String>,
    pub secret: bool,
    /// Several lines (notes, websites, a key).
    pub multiline: bool,
}

/// A way the desktop app uses the item: what to write, and where.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct VaultUse {
    /// `vault:OpenAI/password`, or an SSH key's fingerprint.
    pub reference: String,
    /// "reins run --env and [[api]] secret".
    pub hint: String,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct VaultItemDetail {
    pub id: String,
    pub name: String,
    pub kind: VaultItemKind,
    /// The fields that are set, in the order the item's kind has them; then the custom fields and the notes.
    pub fields: Vec<VaultField>,
    pub uses: Vec<VaultUse>,
    /// Something to fix for the desktop app to find the item ("2 items are named OpenAI"), when there is one.
    pub warning: Option<String>,
    pub favorite: bool,
}

/// A field to set. An empty value clears it.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct VaultFieldInput {
    pub key: String,
    pub value: String,
}

/// A new item, or the changes to one: only the fields given are changed.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct VaultItemInput {
    pub kind: VaultItemKind,
    pub name: String,
    pub fields: Vec<VaultFieldInput>,
}

/// An SSH key made on the phone: its item, and the public half to put on servers.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct VaultSshKey {
    pub id: String,
    /// `ssh-ed25519 AAAA… name`, to paste into `authorized_keys` or a Git host's settings.
    pub public_key: String,
    /// `SHA256:…`.
    pub fingerprint: String,
}

impl Engine {
    /// The vault connector and the account it is unlocked for.
    fn vault_editor(&self) -> Result<(Vault, String), CoreError> {
        self.ensure_active()?;
        let account = self
            .accounts_of(VAULT)
            .into_iter()
            .next()
            .ok_or_else(|| CoreError::needs_attention("Unlock the vault first: Integrations, Password vault."))?;
        Ok((Vault::new(self.vault_session(), Arc::clone(&self.store)), account))
    }

    pub async fn vault_items(&self, query: &str) -> Result<Vec<VaultItemSummary>, CoreError> {
        let (vault, account) = self.vault_editor()?;
        editor::list(&vault, &account, query).await
    }

    pub async fn vault_item(&self, id: &str) -> Result<VaultItemDetail, CoreError> {
        let (vault, account) = self.vault_editor()?;
        editor::detail(&vault, &account, id).await
    }

    pub async fn vault_reveal(&self, id: &str, key: &str) -> Result<String, CoreError> {
        let (vault, account) = self.vault_editor()?;
        editor::reveal(&vault, &account, id, key).await
    }

    pub async fn vault_create(&self, input: &VaultItemInput) -> Result<String, CoreError> {
        let (vault, account) = self.vault_editor()?;
        editor::create(&vault, &account, input).await
    }

    pub async fn vault_update(&self, id: &str, input: &VaultItemInput) -> Result<(), CoreError> {
        let (vault, account) = self.vault_editor()?;
        editor::update(&vault, &account, id, input).await
    }

    pub async fn vault_delete(&self, id: &str) -> Result<(), CoreError> {
        let (vault, account) = self.vault_editor()?;
        editor::delete(&vault, &account, id).await
    }

    pub async fn vault_generate_ssh_key(&self, name: &str) -> Result<VaultSshKey, CoreError> {
        let (vault, account) = self.vault_editor()?;
        editor::generate_ssh_key(&vault, &account, name).await
    }

    /// The eight digits of this phone's inbox key, which `reins vault add` asks the user to compare once.
    pub fn phone_key_fingerprint(&self) -> Result<String, CoreError> {
        let public = editor::inbox_public_key(&self.store)?;
        reins_proto::desktop::phone_key_fingerprint(&public)
            .ok_or_else(|| CoreError::storage("the phone key is invalid"))
    }
}
