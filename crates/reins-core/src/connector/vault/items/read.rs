//! Reading items: searching, the folders, the overview of one item, its fields and its attachments.

use std::collections::BTreeMap;

use data_encoding::BASE64;
use reins_proto::connector::ConnectorCall;
use serde_json::{Map, Value, json};
use url::Url;
use zeroize::Zeroizing;

use super::model::{Entry, Kind, Snapshot, State, decrypt_buffer};
use super::wallet::last_four;
use super::{MAX_FILE_BYTES, host_of};
use crate::connector::Item;
use crate::connector::calendar::{limit, segment};
use crate::connector::vault::Vault;
use crate::connector::vault::totp::{totp_code, unix_seconds};
use crate::crypto::VaultKey;
use crate::{CoreError, text};

/// An item of a listing: who, where, what kind.
fn listed(snap: &Snapshot, entry: &Entry) -> Item {
    let folder = entry.folder();
    let name = entry.name();
    let snippet = match entry.kind {
        Kind::Login => entry.uris().first().map(|u| host_of(u)),
        _ => None,
    };
    let mut extra = Map::new();
    extra.insert("type".to_owned(), json!(entry.kind.slug()));
    extra.insert("folder_id".to_owned(), json!(folder));
    extra.insert("folder".to_owned(), json!(snap.folder_name(folder)));
    extra.insert("favorite".to_owned(), json!(entry.favorite()));
    extra.insert("state".to_owned(), json!(entry.state().word()));
    Item {
        id: entry.id.clone(),
        resource: entry.resource(),
        resource_label: name.clone(),
        title: name,
        from: entry.holder(),
        snippet: snippet.unwrap_or_else(|| entry.kind.label().to_owned()),
        extra,
        parents: snap.parents(folder, entry.kind),
        ..Item::default()
    }
}

pub(super) async fn search(vault: &Vault, account: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
    let snap = Snapshot::load(vault, account).await?;
    let query = call.str_arg("query").unwrap_or_default().to_lowercase();
    // Without `type` only logins are searched, as this tool always did.
    let kind = match call.str_arg("type") {
        None => Some(Kind::Login),
        Some("any") => None,
        Some(slug) => Some(Kind::from_slug(slug).ok_or_else(|| CoreError::service("Unknown item type."))?),
    };
    let state = match call.str_arg("state").unwrap_or("active") {
        "trash" => Some(State::Trash),
        "archived" => Some(State::Archived),
        "all" => None,
        _ => Some(State::Active),
    };
    let folder = call
        .str_arg("folder")
        .map(|f| snap.folder_ref(f).map(|f| f.map_or_else(|| "none".to_owned(), |f| f.id.clone())))
        .transpose()?;
    let favorite = call.bool_arg("favorite");
    let mut found: Vec<Item> = snap
        .entries
        .iter()
        .filter(|e| kind.is_none_or(|k| e.kind == k))
        .filter(|e| state.is_none_or(|s| e.state() == s))
        .filter(|e| folder.as_deref().is_none_or(|f| e.folder() == f))
        .filter(|e| favorite.is_none_or(|f| e.favorite() == f))
        .filter(|e| {
            query.is_empty()
                || e.name().to_lowercase().contains(&query)
                || e.holder().to_lowercase().contains(&query)
                || e.uris().iter().any(|u| u.to_lowercase().contains(&query))
        })
        .map(|e| listed(&snap, e))
        .collect();
    found.sort_by_key(|i| i.title.to_lowercase());
    found.truncate(limit(call));
    if snap.organization_items > 0
        && let Some(first) = found.first_mut()
    {
        first.extra.insert("organization_items_skipped".to_owned(), json!(snap.organization_items));
    }
    Ok(found)
}

pub(super) async fn folders_list(vault: &Vault, account: &str) -> Result<Vec<Item>, CoreError> {
    let snap = Snapshot::load(vault, account).await?;
    let mut ids: Vec<(String, String)> = vec![("none".to_owned(), "No folder".to_owned())];
    let mut named: Vec<&super::model::Folder> = snap.folders.iter().collect();
    named.sort_by_key(|f| f.name.to_lowercase());
    ids.extend(named.iter().map(|f| (f.id.clone(), f.name.clone())));
    Ok(ids
        .into_iter()
        .map(|(id, name)| {
            let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
            let (mut trash, mut archived) = (0, 0);
            for e in snap.entries.iter().filter(|e| e.folder() == id) {
                match e.state() {
                    State::Trash => trash += 1,
                    State::Archived => {
                        archived += 1;
                        *counts.entry(e.kind.slug()).or_default() += 1;
                    }
                    State::Active => *counts.entry(e.kind.slug()).or_default() += 1,
                }
            }
            let total: usize = counts.values().sum();
            let mut extra = Map::new();
            extra.insert("items".to_owned(), json!(total));
            extra.insert("by_type".to_owned(), json!(counts));
            extra.insert("in_trash".to_owned(), json!(trash));
            extra.insert("archived".to_owned(), json!(archived));
            if id == "none" && snap.organization_items > 0 {
                extra.insert("organization_items_skipped".to_owned(), json!(snap.organization_items));
            }
            Item {
                resource: id.clone(),
                resource_label: name.clone(),
                title: name,
                snippet: format!("{total} items"),
                id,
                extra,
                ..Item::default()
            }
        })
        .collect())
}

pub(super) async fn item_view(vault: &Vault, account: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
    let snap = Snapshot::load(vault, account).await?;
    let entry = snap.find(call.str_arg("item").unwrap_or_default())?;
    let mut item = listed(&snap, entry);
    let mut lines = vec![format!("{}: {}", entry.kind.label(), item.title)];
    let folder = snap.folder_name(entry.folder());
    lines.push(format!("Folder: {folder}"));
    lines.push(format!(
        "Favorite: {}",
        if entry.favorite() {
            "yes"
        } else {
            "no"
        }
    ));
    lines.push(format!("State: {}", entry.state().word()));
    let extra = &mut item.extra;
    extra.insert("id".to_owned(), json!(entry.id));
    extra.insert("name".to_owned(), json!(item.title));
    for (key, pointer) in [
        ("created", "/creationDate"),
        ("modified", "/revisionDate"),
        ("deleted", "/deletedDate"),
        ("archived", "/archivedDate"),
    ] {
        if let Some(date) = entry.raw.pointer(pointer).and_then(Value::as_str) {
            extra.insert(key.to_owned(), json!(text::one_line(date)));
            lines.push(format!("{}: {}", key.replace('_', " "), text::one_line(date)));
        }
    }
    extra.insert("requires_master_password".to_owned(), json!(entry.raw["reprompt"].as_i64().unwrap_or(0) == 1));
    let notes = entry.dec("/notes").map_or(0, |n| n.chars().count());
    extra.insert("notes_length".to_owned(), json!(notes));
    lines.push(format!(
        "Notes: {}",
        if notes == 0 {
            "none".to_owned()
        } else {
            format!("{notes} characters")
        }
    ));
    let put = |extra: &mut Map<String, Value>, lines: &mut Vec<String>, key: &str, label: &str, value: String| {
        if !value.is_empty() {
            lines.push(format!("{label}: {value}"));
            extra.insert(key.to_owned(), json!(value));
        }
    };
    match entry.kind {
        Kind::Login => {
            let uris = entry.uris();
            for uri in &uris {
                lines.push(format!("Website: {uri}"));
            }
            extra.insert("websites".to_owned(), json!(uris));
            put(extra, &mut lines, "username", "Username", entry.line("/login/username"));
            let (has_password, has_totp) = (entry.dec("/login/password").is_some(), entry.dec("/login/totp").is_some());
            extra.insert("has_password".to_owned(), json!(has_password));
            extra.insert("has_one_time_code".to_owned(), json!(has_totp));
            lines.push(format!(
                "Password: {}",
                if has_password {
                    "set"
                } else {
                    "none"
                }
            ));
            lines.push(format!(
                "One-time code: {}",
                if has_totp {
                    "set up"
                } else {
                    "none"
                }
            ));
        }
        Kind::Card => {
            put(extra, &mut lines, "holder", "Holder", entry.line("/card/cardholderName"));
            put(extra, &mut lines, "brand", "Brand", entry.line("/card/brand"));
            if let Some(last4) = entry.dec("/card/number").and_then(|n| last_four(&n)) {
                lines.push(format!("Number ends in: {last4}"));
                extra.insert("last4".to_owned(), json!(last4));
            }
            put(extra, &mut lines, "exp_month", "Expiry month", entry.line("/card/expMonth"));
            put(extra, &mut lines, "exp_year", "Expiry year", entry.line("/card/expYear"));
        }
        Kind::Identity => {
            for (key, label, field) in [
                ("title", "Title", "title"),
                ("first_name", "First name", "firstName"),
                ("middle_name", "Middle name", "middleName"),
                ("last_name", "Last name", "lastName"),
                ("email", "Email", "email"),
                ("company", "Company", "company"),
                ("city", "City", "city"),
                ("country", "Country", "country"),
            ] {
                put(extra, &mut lines, key, label, entry.line(&format!("/identity/{field}")));
            }
        }
        Kind::SshKey => {
            put(extra, &mut lines, "public_key", "Public key", entry.line("/sshKey/publicKey"));
            put(extra, &mut lines, "fingerprint", "Fingerprint", entry.line("/sshKey/keyFingerprint"));
        }
        Kind::Note => {}
    }
    let custom: Vec<Value> =
        entry.custom_fields().iter().map(|c| json!({"name": c.name, "type": c.kind_word()})).collect();
    for c in entry.custom_fields() {
        lines.push(format!("Custom field: {} ({})", c.name, c.kind_word()));
    }
    extra.insert("custom_fields".to_owned(), json!(custom));
    let attachments: Vec<Value> =
        entry.attachments().iter().map(|a| json!({"id": a.id, "name": a.name, "size": a.size})).collect();
    for a in entry.attachments() {
        lines.push(format!("Attachment: {} ({} bytes, id {})", a.name, a.size, a.id));
    }
    extra.insert("attachments".to_owned(), json!(attachments));
    extra.insert("password_history_count".to_owned(), json!(entry.history_len()));
    if entry.history_len() > 0 {
        lines.push(format!("Old passwords kept: {}", entry.history_len()));
    }
    item.snippet = format!("{} · {folder}", entry.kind.label());
    item.body = Some(lines.join("\n"));
    Ok(vec![item])
}

/// What `vault_get` found.
struct Field {
    label: String,
    value: Zeroizing<String>,
    /// Ordinary reads (a website, a public key) are not secrets.
    plain: bool,
    id_suffix: String,
    extra: Map<String, Value>,
}

impl Field {
    fn secret(label: &str, value: Zeroizing<String>) -> Self {
        Self::new(label, value, false)
    }

    fn plain(label: &str, value: Zeroizing<String>) -> Self {
        Self::new(label, value, true)
    }

    fn new(label: &str, value: Zeroizing<String>, plain: bool) -> Self {
        Self {
            label: label.to_owned(),
            value,
            plain,
            id_suffix: String::new(),
            extra: Map::new(),
        }
    }
}

const IDENTITY_FIELDS: [(&str, &str); 18] = [
    ("Title", "title"),
    ("First name", "firstName"),
    ("Middle name", "middleName"),
    ("Last name", "lastName"),
    ("Company", "company"),
    ("Email", "email"),
    ("Phone", "phone"),
    ("Address 1", "address1"),
    ("Address 2", "address2"),
    ("Address 3", "address3"),
    ("City", "city"),
    ("State", "state"),
    ("Postal code", "postalCode"),
    ("Country", "country"),
    ("Username", "username"),
    ("Social security number", "ssn"),
    ("Passport number", "passportNumber"),
    ("License number", "licenseNumber"),
];

/// The value at a pointer, or the error the AI gets when the item has none.
fn need(entry: &Entry, pointer: &str, what: &str) -> Result<Zeroizing<String>, CoreError> {
    entry
        .dec(pointer)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| CoreError::service(format!("That {} has no {what}.", entry.kind.label().to_lowercase())))
}

fn expect(entry: &Entry, kind: Kind, what: &str) -> Result<(), CoreError> {
    if entry.kind == kind {
        Ok(())
    } else {
        Err(CoreError::service(format!("That item is a {}, which has no {what}.", entry.kind.label().to_lowercase())))
    }
}

fn read_field(entry: &Entry, field: &str, custom: Option<&str>) -> Result<Field, CoreError> {
    Ok(match field {
        "username" => {
            expect(entry, Kind::Login, "username")?;
            Field::secret("Username", need(entry, "/login/username", "username")?)
        }
        "password" => {
            expect(entry, Kind::Login, "password")?;
            Field::secret("Password", need(entry, "/login/password", "password")?)
        }
        "totp" => {
            expect(entry, Kind::Login, "one-time code")?;
            let stored = entry
                .dec("/login/totp")
                .filter(|t| !t.is_empty())
                .ok_or_else(|| CoreError::service("That login has no one-time code set up."))?;
            let (code, valid_for) = totp_code(&stored, unix_seconds())?;
            let mut f = Field::secret("One-time code", Zeroizing::new(code));
            f.extra.insert("valid_for_seconds".to_owned(), json!(valid_for));
            f
        }
        "notes" => Field::secret("Notes", need(entry, "/notes", "notes")?),
        "uri" => {
            expect(entry, Kind::Login, "website")?;
            let uris = entry.uris();
            if uris.is_empty() {
                return Err(CoreError::service("That login has no website."));
            }
            let mut f = Field::plain("Website", Zeroizing::new(uris.join("\n")));
            f.extra.insert("websites".to_owned(), json!(uris));
            f
        }
        "card_number" => {
            expect(entry, Kind::Card, "number")?;
            Field::secret("Card number", need(entry, "/card/number", "number")?)
        }
        "card_code" => {
            expect(entry, Kind::Card, "security code")?;
            Field::secret("Card security code", need(entry, "/card/code", "security code")?)
        }
        "card_holder" => {
            expect(entry, Kind::Card, "holder")?;
            Field::plain("Card holder", need(entry, "/card/cardholderName", "holder")?)
        }
        "card_expiry" => {
            expect(entry, Kind::Card, "expiry")?;
            let (month, year) = (entry.line("/card/expMonth"), entry.line("/card/expYear"));
            if month.is_empty() && year.is_empty() {
                return Err(CoreError::service("That card has no expiry date."));
            }
            Field::plain("Card expiry", Zeroizing::new(format!("{month}/{year}")))
        }
        "identity" => {
            expect(entry, Kind::Identity, "identity details")?;
            let block: Vec<String> = IDENTITY_FIELDS
                .iter()
                .filter_map(|(label, key)| {
                    let v = entry.dec(&format!("/identity/{key}")).filter(|v| !v.is_empty())?;
                    Some(format!("{label}: {}", *v))
                })
                .collect();
            if block.is_empty() {
                return Err(CoreError::service("That identity is empty."));
            }
            Field::secret("Identity details", Zeroizing::new(block.join("\n")))
        }
        "identity_ssn" => {
            expect(entry, Kind::Identity, "social security number")?;
            Field::secret("Social security number", need(entry, "/identity/ssn", "social security number")?)
        }
        "identity_passport" => {
            expect(entry, Kind::Identity, "passport number")?;
            Field::secret("Passport number", need(entry, "/identity/passportNumber", "passport number")?)
        }
        "identity_license" => {
            expect(entry, Kind::Identity, "license number")?;
            Field::secret("Driver's license number", need(entry, "/identity/licenseNumber", "license number")?)
        }
        // The SSH agent of the desktop app has the phone sign; the private key itself never leaves the vault.
        "ssh_private_key" => {
            return Err(CoreError::service(
                "An SSH key's private key is never given out. Use the Reins desktop app's SSH agent: the phone signs.",
            ));
        }
        "ssh_public_key" => {
            expect(entry, Kind::SshKey, "public key")?;
            Field::plain("SSH public key", need(entry, "/sshKey/publicKey", "public key")?)
        }
        "ssh_fingerprint" => {
            expect(entry, Kind::SshKey, "fingerprint")?;
            Field::plain("SSH fingerprint", need(entry, "/sshKey/keyFingerprint", "fingerprint")?)
        }
        "custom" => {
            let fields = entry.custom_fields();
            if fields.is_empty() {
                return Err(CoreError::service("That item has no custom fields."));
            }
            if let Some(name) = custom {
                {
                    let c = fields
                        .iter()
                        .find(|c| c.name == name)
                        .or_else(|| fields.iter().find(|c| c.name.eq_ignore_ascii_case(name)))
                        .ok_or_else(|| {
                            let names: Vec<&str> = fields.iter().map(|c| c.name.as_str()).collect();
                            CoreError::service(format!(
                                "No custom field named that. The item has: {}.",
                                names.join(", ")
                            ))
                        })?;
                    let mut f = Field::secret(&format!("Custom field \"{}\"", c.name), c.value.clone());
                    f.id_suffix = format!(":{}", c.name);
                    f.extra.insert("type".to_owned(), json!(c.kind_word()));
                    f
                }
            } else {
                let block: Vec<String> = fields.iter().map(|c| format!("{}: {}", c.name, *c.value)).collect();
                Field::secret("Custom fields", Zeroizing::new(block.join("\n")))
            }
        }
        "password_history" => {
            let block: Vec<String> = entry.raw["passwordHistory"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|h| {
                    let password = entry.key.decrypt_text(h["password"].as_str()?).ok()?;
                    Some(format!("{}: {}", text::one_line(h["lastUsedDate"].as_str().unwrap_or("")), *password))
                })
                .collect();
            if block.is_empty() {
                return Err(CoreError::service("That item keeps no earlier passwords."));
            }
            Field::secret("Password history", Zeroizing::new(block.join("\n")))
        }
        _ => return Err(CoreError::service("Unknown field.")),
    })
}

pub(super) async fn get(vault: &Vault, account: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
    let snap = Snapshot::load(vault, account).await?;
    let wanted = call.str_arg("item").unwrap_or_default();
    let field = call.str_arg("field").unwrap_or_default();
    let entry = snap.entries.iter().find(|e| e.id == wanted && e.state() != State::Trash).ok_or_else(|| {
        if matches!(field, "username" | "password" | "totp") {
            CoreError::service("No login with that id. Use vault_search to find it.")
        } else {
            CoreError::service("No item with that id. Use vault_search to find it.")
        }
    })?;
    let got = read_field(entry, field, call.str_arg("custom_field"))?;
    let name = entry.name();
    Ok(vec![Item {
        id: format!("{}:{field}{}", entry.id, got.id_suffix),
        resource: entry.resource(),
        resource_label: name.clone(),
        title: name.clone(),
        from: entry.holder(),
        snippet: format!("{} for {name}", got.label),
        body: Some(got.value.to_string()),
        // Every secret is asked for again each time, whatever it is.
        sensitive: !got.plain,
        secret: !got.plain,
        extra: got.extra,
        parents: snap.parents(entry.folder(), entry.kind),
        ..Item::default()
    }])
}

/// Finds an attachment by id or exact file name.
pub(super) fn attachment_of<'a>(
    attachments: &'a [super::model::Attachment],
    given: &str,
) -> Result<&'a super::model::Attachment, CoreError> {
    attachments.iter().find(|a| a.id == given).or_else(|| attachments.iter().find(|a| a.name == given)).ok_or_else(
        || CoreError::service("That item has no attachment with that id or name. Use vault_item_view to see them."),
    )
}

pub(super) async fn attachment_get(vault: &Vault, account: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
    let snap = Snapshot::load(vault, account).await?;
    let entry = snap.find(call.str_arg("item").unwrap_or_default())?;
    let attachments = entry.attachments();
    let attachment = attachment_of(&attachments, call.str_arg("attachment").unwrap_or_default())?;
    let name = entry.name();
    let mut item = Item {
        id: format!("{}:attachment:{}", entry.id, attachment.id),
        resource: entry.resource(),
        resource_label: name.clone(),
        title: name.clone(),
        from: attachment.name.clone(),
        snippet: format!("Attachment \"{}\" of {name}", attachment.name),
        parents: snap.parents(entry.folder(), entry.kind),
        ..Item::default()
    };
    item.extra.insert("file_name".to_owned(), json!(attachment.name));
    item.extra.insert("size".to_owned(), json!(attachment.size));
    if attachment.size > MAX_FILE_BYTES as u64 {
        item.extra.insert("too_large".to_owned(), json!(true));
        item.extra.insert(
            "note".to_owned(),
            json!("The file is larger than 2 MB, so its content is not sent. The user can download it in the vault."),
        );
        item.snippet =
            format!("Attachment \"{}\" of {name} is too large to send ({} bytes)", attachment.name, attachment.size);
        return Ok(vec![item]);
    }
    let meta = vault
        .api(
            &snap.session,
            reqwest::Method::GET,
            &format!("/api/ciphers/{}/attachment/{}", segment(&entry.id), segment(&attachment.id)),
            None,
        )
        .await?;
    let url = meta["url"].as_str().ok_or_else(|| CoreError::service("The vault gave no download address."))?;
    let blob = download(&snap, url).await?;
    let file_key = match &attachment.key {
        Some(k) => VaultKey::from_bytes(&entry.key.decrypt(k)?)?,
        None => VaultKey::from_bytes(&entry.key.to_bytes())?,
    };
    let data = decrypt_buffer(&file_key, &blob)?;
    if data.len() > MAX_FILE_BYTES {
        return Err(CoreError::service("The attachment is larger than 2 MB, so it is not sent."));
    }
    item.body = Some(BASE64.encode(&data));
    item.sensitive = true;
    item.secret = true;
    item.extra.insert("encoding".to_owned(), json!("base64"));
    item.extra.insert("size".to_owned(), json!(data.len()));
    // Too large to go inline: uploaded to the server as a download link once the user released it, not before.
    if crate::blob::as_link(data.len() as u64) {
        item.extra.insert(crate::blob::DELIVER_KEY.to_owned(), crate::blob::output_marker(&attachment.name));
        item.snippet = format!("{} ({} bytes)", item.snippet, data.len());
    }
    Ok(vec![item])
}

/// Downloads an attachment's encrypted bytes from the address the vault gave, when it is the vault's own host.
async fn download(snap: &Snapshot, url: &str) -> Result<Vec<u8>, CoreError> {
    let (Ok(target), Ok(home)) = (Url::parse(url), Url::parse(&snap.session.server.join("/"))) else {
        return Err(CoreError::service("The vault gave an unusable download address."));
    };
    let same = target.scheme() == home.scheme()
        && target.host_str() == home.host_str()
        && target.port_or_known_default() == home.port_or_known_default();
    if !same {
        return Err(CoreError::service(
            "The vault gave a download address on another host, so nothing was downloaded.",
        ));
    }
    let response = snap.session.http.get(target).send().await?;
    if !response.status().is_success() {
        return Err(CoreError::service(format!(
            "The vault could not send the file (status {}).",
            response.status().as_u16()
        )));
    }
    if response.content_length().is_some_and(|n| n > (MAX_FILE_BYTES as u64) + 1_024) {
        return Err(CoreError::service("The attachment is larger than 2 MB, so it is not sent."));
    }
    let bytes = response.bytes().await?;
    if bytes.len() > MAX_FILE_BYTES + 1_024 {
        return Err(CoreError::service("The attachment is larger than 2 MB, so it is not sent."));
    }
    Ok(bytes.to_vec())
}
