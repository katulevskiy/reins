//! The vault as the items area sees it: the ciphers exactly as the server sent them (so an edit can resend
//! everything it did not change), each with the key that decrypts it, and the folders.

use std::sync::Arc;

use aes::Aes256;
use cbc::cipher::block_padding::Pkcs7;
use cbc::cipher::{BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use ring::hmac;
use serde_json::Value;
use zeroize::Zeroizing;

use super::super::Vault;
use crate::crypto::{self, VaultKey};
use crate::session::Session;
use crate::{CoreError, text};

/// The version a Bitwarden client announces; Vaultwarden leaves SSH keys out of a sync without one that is recent.
const CLIENT_VERSION: &str = "2025.1.0";

/// The kind of an item (the Bitwarden cipher `type`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Login,
    Note,
    Card,
    Identity,
    SshKey,
}

impl Kind {
    pub fn from_code(code: i64) -> Option<Self> {
        Some(match code {
            1 => Self::Login,
            2 => Self::Note,
            3 => Self::Card,
            4 => Self::Identity,
            5 => Self::SshKey,
            _ => return None,
        })
    }

    pub fn from_slug(slug: &str) -> Option<Self> {
        Some(match slug {
            "login" => Self::Login,
            "note" => Self::Note,
            "card" => Self::Card,
            "identity" => Self::Identity,
            "ssh_key" => Self::SshKey,
            _ => return None,
        })
    }

    pub fn code(self) -> i64 {
        match self {
            Self::Login => 1,
            Self::Note => 2,
            Self::Card => 3,
            Self::Identity => 4,
            Self::SshKey => 5,
        }
    }

    /// The name in resources and tool arguments.
    pub fn slug(self) -> &'static str {
        match self {
            Self::Login => "login",
            Self::Note => "note",
            Self::Card => "card",
            Self::Identity => "identity",
            Self::SshKey => "ssh_key",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Login => "Login",
            Self::Note => "Secure note",
            Self::Card => "Card",
            Self::Identity => "Identity",
            Self::SshKey => "SSH key",
        }
    }

    pub fn plural(self) -> &'static str {
        match self {
            Self::Login => "Logins",
            Self::Note => "Secure notes",
            Self::Card => "Cards",
            Self::Identity => "Identities",
            Self::SshKey => "SSH keys",
        }
    }

    /// The key that holds the type's content in the cipher JSON.
    pub fn object(self) -> &'static str {
        match self {
            Self::Login => "login",
            Self::Note => "secureNote",
            Self::Card => "card",
            Self::Identity => "identity",
            Self::SshKey => "sshKey",
        }
    }
}

/// Where an item is: in use, in the trash, or archived.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum State {
    Active,
    Trash,
    Archived,
}

impl State {
    pub fn word(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Trash => "trash",
            Self::Archived => "archived",
        }
    }
}

/// One custom field of an item.
pub(super) struct Custom {
    pub name: String,
    pub value: Zeroizing<String>,
    /// 0 text, 1 hidden, 2 boolean, 3 linked.
    pub kind: i64,
}

impl Custom {
    pub fn kind_word(&self) -> &'static str {
        match self.kind {
            0 => "text",
            2 => "boolean",
            3 => "linked",
            _ => "hidden",
        }
    }
}

/// One attachment of an item, its name decrypted.
pub(super) struct Attachment {
    pub id: String,
    pub name: String,
    pub size: u64,
    /// The attachment's own key, encrypted with the item's key (older attachments have none).
    pub key: Option<String>,
}

/// One of the user's own items with the key that opens it.
pub(super) struct Entry {
    /// The cipher exactly as the server sent it.
    pub raw: Value,
    pub id: String,
    pub kind: Kind,
    /// The item's own key when it has one, else the user key.
    pub key: VaultKey,
}

impl Entry {
    fn new(raw: &Value, user: &VaultKey) -> Option<Self> {
        if !raw["organizationId"].is_null() {
            return None;
        }
        let kind = Kind::from_code(raw["type"].as_i64()?)?;
        let key = match raw["key"].as_str() {
            Some(own) => VaultKey::from_bytes(&user.decrypt(own).ok()?).ok()?,
            None => VaultKey::from_bytes(&user.to_bytes()).ok()?,
        };
        Some(Self {
            raw: raw.clone(),
            id: raw["id"].as_str()?.to_owned(),
            kind,
            key,
        })
    }

    /// Whether the item has a key of its own (an edit must keep it).
    pub fn has_own_key(&self) -> bool {
        self.raw["key"].is_string()
    }

    /// The decrypted string at a JSON pointer (`/login/username`); `None` when absent, null or unreadable.
    pub fn dec(&self, pointer: &str) -> Option<Zeroizing<String>> {
        self.key.decrypt_text(self.raw.pointer(pointer)?.as_str()?).ok()
    }

    /// Like [`Entry::dec`] for text that may be shown: one line, empty when absent.
    pub fn line(&self, pointer: &str) -> String {
        self.dec(pointer).map(|t| text::one_line(&t)).unwrap_or_default()
    }

    pub fn name(&self) -> String {
        self.line("/name")
    }

    pub fn folder(&self) -> &str {
        self.raw["folderId"].as_str().filter(|f| !f.is_empty()).unwrap_or("none")
    }

    /// The resource of the item: `{folder}/{type}/{item}`.
    pub fn resource(&self) -> String {
        format!("{}/{}/{}", self.folder(), self.kind.slug(), self.id)
    }

    pub fn favorite(&self) -> bool {
        self.raw["favorite"].as_bool().unwrap_or(false)
    }

    pub fn state(&self) -> State {
        if !self.raw["deletedDate"].is_null() {
            State::Trash
        } else if !self.raw["archivedDate"].is_null() {
            State::Archived
        } else {
            State::Active
        }
    }

    /// Every website of a login, decrypted.
    pub fn uris(&self) -> Vec<String> {
        self.raw
            .pointer("/login/uris")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|u| self.key.decrypt_text(u["uri"].as_str()?).ok())
            .map(|u| text::one_line(&u))
            .filter(|u| !u.is_empty())
            .collect()
    }

    /// Who the item is for, as one line: a login's username, a card's holder, an identity's name.
    pub fn holder(&self) -> String {
        match self.kind {
            Kind::Login => self.line("/login/username"),
            Kind::Card => self.line("/card/cardholderName"),
            Kind::Identity => {
                let name: Vec<String> = ["firstName", "middleName", "lastName"]
                    .iter()
                    .map(|k| self.line(&format!("/identity/{k}")))
                    .filter(|p| !p.is_empty())
                    .collect();
                name.join(" ")
            }
            Kind::Note | Kind::SshKey => String::new(),
        }
    }

    /// One slot per custom field of the cipher, in order (`None` for one that cannot be read).
    pub fn custom_slots(&self) -> Vec<Option<Custom>> {
        self.raw["fields"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|f| {
                let name = self.key.decrypt_text(f["name"].as_str()?).ok()?;
                let value = f["value"].as_str().and_then(|v| self.key.decrypt_text(v).ok()).unwrap_or_default();
                Some(Custom {
                    name: text::one_line(&name),
                    value,
                    kind: f["type"].as_i64().unwrap_or(1),
                })
            })
            .collect()
    }

    pub fn custom_fields(&self) -> Vec<Custom> {
        self.custom_slots().into_iter().flatten().collect()
    }

    pub fn attachments(&self) -> Vec<Attachment> {
        self.raw["attachments"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|a| {
                Some(Attachment {
                    id: a["id"].as_str()?.to_owned(),
                    name: a["fileName"]
                        .as_str()
                        .and_then(|n| self.key.decrypt_text(n).ok())
                        .map_or_else(|| "(unreadable name)".to_owned(), |n| text::one_line(&n)),
                    size: a["size"].as_str().and_then(|s| s.parse().ok()).or_else(|| a["size"].as_u64()).unwrap_or(0),
                    key: a["key"].as_str().map(str::to_owned),
                })
            })
            .collect()
    }

    /// How many old passwords are kept.
    pub fn history_len(&self) -> usize {
        self.raw["passwordHistory"].as_array().map_or(0, Vec::len)
    }
}

pub(super) struct Folder {
    pub id: String,
    pub name: String,
}

/// The vault, read once for one call.
pub(super) struct Snapshot {
    pub session: Arc<Session>,
    pub user: VaultKey,
    pub entries: Vec<Entry>,
    pub folders: Vec<Folder>,
    /// Items that belong to an organization: not handled here.
    pub organization_items: usize,
}

impl Snapshot {
    /// Reads the whole vault. The client version header makes the server include SSH keys.
    pub async fn load(vault: &Vault, account: &str) -> Result<Self, CoreError> {
        let session = vault.session()?;
        let user = vault.key(account)?;
        let mut retried = false;
        let sync: Value = loop {
            let token = session.access_token().await?;
            let response = session
                .http
                .get(session.server.join("/api/sync?excludeDomains=true"))
                .bearer_auth(token.as_str())
                .header("Bitwarden-Client-Version", CLIENT_VERSION)
                .send()
                .await?;
            let status = response.status();
            if status.as_u16() == 401 && !retried {
                retried = true;
                session.invalidate().await;
                continue;
            }
            if !status.is_success() {
                return Err(match status.as_u16() {
                    401 => CoreError::NotLoggedIn,
                    code => CoreError::Server {
                        status: code,
                        reason: "the vault could not be read".to_owned(),
                    },
                });
            }
            let bytes = Zeroizing::new(response.bytes().await?.to_vec());
            break serde_json::from_slice(&bytes).map_err(|_| CoreError::Network {
                reason: "invalid vault response".to_owned(),
            })?;
        };
        let ciphers = sync["ciphers"].as_array().map_or(&[][..], Vec::as_slice);
        let organization_items = ciphers.iter().filter(|c| !c["organizationId"].is_null()).count();
        let entries = ciphers.iter().filter_map(|c| Entry::new(c, &user)).collect();
        let folders = sync["folders"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|f| {
                Some(Folder {
                    id: f["id"].as_str()?.to_owned(),
                    name: f["name"]
                        .as_str()
                        .and_then(|n| user.decrypt_text(n).ok())
                        .map_or_else(|| "(unreadable name)".to_owned(), |n| text::one_line(&n)),
                })
            })
            .collect();
        Ok(Self {
            session,
            user,
            entries,
            folders,
            organization_items,
        })
    }

    /// One item by id, whatever its state.
    pub fn find(&self, id: &str) -> Result<&Entry, CoreError> {
        self.entries
            .iter()
            .find(|e| e.id == id)
            .ok_or_else(|| CoreError::service("No item with that id. Use vault_search to find it."))
    }

    /// A folder named by the AI: `none`, an id or an exact name. `None` is "no folder".
    pub fn folder_ref(&self, given: &str) -> Result<Option<&Folder>, CoreError> {
        let given = given.trim();
        if given.eq_ignore_ascii_case("none") {
            return Ok(None);
        }
        if let Some(f) = self.folders.iter().find(|f| f.id == given) {
            return Ok(Some(f));
        }
        let mut same = self.folders.iter().filter(|f| f.name.eq_ignore_ascii_case(given));
        match (same.next(), same.next()) {
            (Some(f), None) => Ok(Some(f)),
            (Some(_), Some(_)) => {
                Err(CoreError::service("Several folders have that name. Use the folder's id from vault_folders_list."))
            }
            _ => Err(CoreError::service("No folder with that id or name. Use vault_folders_list to see the folders.")),
        }
    }

    /// The name of a folder by id (`none` is "No folder").
    pub fn folder_name(&self, id: &str) -> String {
        if id == "none" {
            return "No folder".to_owned();
        }
        self.folders.iter().find(|f| f.id == id).map_or_else(|| "Folder".to_owned(), |f| f.name.clone())
    }

    /// The wider things an item's resource belongs to, nearest first.
    pub fn parents(&self, folder: &str, kind: Kind) -> Vec<(String, String)> {
        let name = self.folder_name(folder);
        vec![(format!("{folder}/{}", kind.slug()), format!("{} in {name}", kind.plural())), (folder.to_owned(), name)]
    }
}

/// Encrypts a file the way Bitwarden does: `0x02 || iv || mac || ciphertext`.
pub(super) fn encrypt_buffer(key: &VaultKey, data: &[u8]) -> Result<Vec<u8>, CoreError> {
    let bytes = key.to_bytes();
    let (enc, mac_key) = bytes.split_at(32);
    let iv = crypto::random_bytes::<16>()?;
    let ciphertext = cbc::Encryptor::<Aes256>::new_from_slices(enc, &iv)
        .map_err(|_| CoreError::invalid("unsupported vault key"))?
        .encrypt_padded_vec_mut::<Pkcs7>(data);
    let mut signed = iv.to_vec();
    signed.extend_from_slice(&ciphertext);
    let mac = hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, mac_key), &signed);
    let mut out = Vec::with_capacity(1 + 16 + 32 + ciphertext.len());
    out.push(2);
    out.extend_from_slice(&iv);
    out.extend_from_slice(mac.as_ref());
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

/// The inverse of [`encrypt_buffer`]; the MAC is checked first.
pub(super) fn decrypt_buffer(key: &VaultKey, blob: &[u8]) -> Result<Zeroizing<Vec<u8>>, CoreError> {
    let bad = || CoreError::service("The attachment could not be decrypted.");
    let rest = blob.strip_prefix(&[2]).ok_or_else(bad)?;
    if rest.len() < 48 {
        return Err(bad());
    }
    let (iv, rest) = rest.split_at(16);
    let (mac, ciphertext) = rest.split_at(32);
    let bytes = key.to_bytes();
    let (enc, mac_key) = bytes.split_at(32);
    let mut signed = iv.to_vec();
    signed.extend_from_slice(ciphertext);
    hmac::verify(&hmac::Key::new(hmac::HMAC_SHA256, mac_key), &signed, mac).map_err(|_| bad())?;
    let plain = cbc::Decryptor::<Aes256>::new_from_slices(enc, iv)
        .map_err(|_| bad())?
        .decrypt_padded_vec_mut::<Pkcs7>(ciphertext)
        .map_err(|_| bad())?;
    Ok(Zeroizing::new(plain))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_round_trip_and_tampering_is_caught() {
        let key = VaultKey::from_bytes(&[3u8; 64]).unwrap();
        let blob = encrypt_buffer(&key, b"hello attachment").unwrap();
        assert_eq!(blob[0], 2);
        assert_eq!(&*decrypt_buffer(&key, &blob).unwrap(), b"hello attachment");
        let mut bad = blob.clone();
        *bad.last_mut().unwrap() ^= 1;
        assert!(decrypt_buffer(&key, &bad).is_err());
        assert!(decrypt_buffer(&key, &[2, 1, 2]).is_err());
        assert!(decrypt_buffer(&VaultKey::from_bytes(&[4u8; 64]).unwrap(), &blob).is_err());
    }

    #[test]
    fn the_folder_of_an_item_defaults_to_none() {
        let user = VaultKey::from_bytes(&[3u8; 64]).unwrap();
        let raw = serde_json::json!({"id": "a", "type": 2, "folderId": null, "organizationId": null});
        let entry = Entry::new(&raw, &user).unwrap();
        assert_eq!((entry.folder(), entry.state(), entry.kind), ("none", State::Active, Kind::Note));
        assert!(Entry::new(&serde_json::json!({"id": "a", "type": 2, "organizationId": "o"}), &user).is_none());
        assert!(Entry::new(&serde_json::json!({"id": "a", "type": 9, "organizationId": null}), &user).is_none());
    }
}
