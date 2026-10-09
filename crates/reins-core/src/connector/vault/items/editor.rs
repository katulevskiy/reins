//! The phone's own vault screens ([`crate::vault_editor`]): the list, one item's fields, and the changes the user makes
//! on the phone. A change is planned and carried out by the same code as the vault tools (`item_create`, `item_update`,
//! `item_delete`), without asking: the user is the one acting.

use reins_proto::connector::{ConnectorCall, VAULT};
use serde_json::{Map, Value, json};
use ssh_key::private::{Ed25519Keypair, KeypairData};
use ssh_key::{HashAlg, LineEnding, PrivateKey};
use zeroize::Zeroizing;

use super::host_of;
use super::model::{Entry, Kind, Snapshot, State};
use super::write::{carry_out, defs, plan_with};
use crate::connector::vault::Vault;
use crate::store::Store;
use crate::vault_editor::{
    VaultField, VaultFieldInput, VaultItemDetail, VaultItemInput, VaultItemKind, VaultItemSummary, VaultSshKey,
    VaultUse,
};
use crate::{CoreError, crypto, text};

/// The prefix of a custom field's key.
const CUSTOM: &str = "custom:";
const RUN_HINT: &str = "reins run --env, and secret = in an [[api]] block";

fn kind_view(kind: Kind) -> VaultItemKind {
    match kind {
        Kind::Login => VaultItemKind::Login,
        Kind::Note => VaultItemKind::Note,
        Kind::Card => VaultItemKind::Card,
        Kind::Identity => VaultItemKind::Identity,
        Kind::SshKey => VaultItemKind::SshKey,
    }
}

fn kind_of(kind: VaultItemKind) -> Kind {
    match kind {
        VaultItemKind::Login => Kind::Login,
        VaultItemKind::Note => Kind::Note,
        VaultItemKind::Card => Kind::Card,
        VaultItemKind::Identity => Kind::Identity,
        VaultItemKind::SshKey => Kind::SshKey,
    }
}

fn pointer(kind: Kind, key: &str) -> String {
    format!("/{}/{key}", kind.object())
}

/// The line under the name in the list.
fn subtitle(entry: &Entry) -> String {
    match entry.kind {
        Kind::Login => {
            let user = entry.line("/login/username");
            if user.is_empty() {
                entry.uris().first().map(|u| host_of(u)).unwrap_or_default()
            } else {
                user
            }
        }
        Kind::Card => {
            let holder = entry.holder();
            let last4: String = entry
                .dec("/card/number")
                .map(|n| n.chars().filter(char::is_ascii_digit).collect::<String>())
                .filter(|d| d.len() >= 4)
                .map(|d| d[d.len() - 4..].to_owned())
                .unwrap_or_default();
            match (holder.is_empty(), last4.is_empty()) {
                (_, true) => holder,
                (true, false) => format!("•••• {last4}"),
                (false, false) => format!("{holder} · •••• {last4}"),
            }
        }
        Kind::Identity => entry.holder(),
        Kind::SshKey => entry.line("/sshKey/keyFingerprint"),
        Kind::Note => String::new(),
    }
}

fn summary(entry: &Entry) -> VaultItemSummary {
    VaultItemSummary {
        id: entry.id.clone(),
        name: entry.name(),
        kind: kind_view(entry.kind),
        subtitle: subtitle(entry),
        favorite: entry.favorite(),
    }
}

/// The items in use whose name, username, holder or website contains `query`, by name.
pub(crate) async fn list(vault: &Vault, account: &str, query: &str) -> Result<Vec<VaultItemSummary>, CoreError> {
    let snap = Snapshot::load(vault, account).await?;
    let query = query.trim().to_lowercase();
    let mut found: Vec<VaultItemSummary> = snap
        .entries
        .iter()
        .filter(|e| e.state() == State::Active)
        .filter(|e| {
            query.is_empty()
                || e.name().to_lowercase().contains(&query)
                || e.holder().to_lowercase().contains(&query)
                || e.uris().iter().any(|u| u.to_lowercase().contains(&query))
        })
        .map(summary)
        .collect();
    found.sort_by_key(|i| (i.name.to_lowercase(), i.id.clone()));
    Ok(found)
}

/// An item that is not in the trash.
fn active<'a>(snap: &'a Snapshot, id: &str) -> Result<&'a Entry, CoreError> {
    snap.entries
        .iter()
        .find(|e| e.id == id && e.state() != State::Trash)
        .ok_or_else(|| CoreError::service("That item is no longer in the vault."))
}

fn multiline(key: &str) -> bool {
    matches!(key, "notes" | "uris" | "private_key" | "public_key")
}

/// How the desktop app names the item's values: `vault:Name/field` for `reins run` and the API proxy, the fingerprint
/// for the SSH agent.
fn uses(entry: &Entry) -> Vec<VaultUse> {
    let name = entry.name();
    let reference = |field: &str| format!("vault:{name}/{field}");
    let mut out = Vec::new();
    let mut add = |field: &str, hint: &str| {
        out.push(VaultUse {
            reference: reference(field),
            hint: hint.to_owned(),
        });
    };
    match entry.kind {
        Kind::Login => {
            if entry.dec("/login/password").is_some_and(|p| !p.is_empty()) {
                add("password", RUN_HINT);
            }
            if entry.dec("/login/username").is_some_and(|u| !u.is_empty()) {
                add("username", RUN_HINT);
            }
            if entry.dec("/login/totp").is_some_and(|t| !t.is_empty()) {
                add("totp", "the current one-time code, for reins run");
            }
        }
        Kind::Note => {
            if entry.dec("/notes").is_some_and(|n| !n.is_empty()) {
                add("notes", RUN_HINT);
            }
        }
        Kind::SshKey => {
            let fingerprint = entry.line("/sshKey/keyFingerprint");
            if !fingerprint.is_empty() {
                out.push(VaultUse {
                    reference: fingerprint,
                    hint: "Offered by the SSH agent of the desktop app (reins ssh setup); the private key stays on \
                           the phone"
                        .to_owned(),
                });
            }
        }
        Kind::Card | Kind::Identity => {}
    }
    for c in entry.custom_fields() {
        out.push(VaultUse {
            reference: reference(&c.name),
            hint: RUN_HINT.to_owned(),
        });
    }
    out
}

/// What keeps the desktop app from finding the item by name, if anything.
fn warning(snap: &Snapshot, entry: &Entry) -> Option<String> {
    let name = entry.name();
    if name.trim() != name {
        return Some("The name starts or ends with a space, which vault:… references leave out. Remove it.".to_owned());
    }
    let same = snap.entries.iter().filter(|e| e.state() != State::Trash && e.name() == name).count();
    (same > 1).then(|| {
        format!("{same} items are named \u{201c}{name}\u{201d}, so vault:{name}/… could mean either. Rename one.")
    })
}

pub(crate) async fn detail(vault: &Vault, account: &str, id: &str) -> Result<VaultItemDetail, CoreError> {
    let snap = Snapshot::load(vault, account).await?;
    let entry = active(&snap, id)?;
    let mut fields = Vec::new();
    for d in defs(entry.kind) {
        let Some(value) = entry.dec(&pointer(entry.kind, d.key)).filter(|v| !v.is_empty()) else {
            continue;
        };
        fields.push(VaultField {
            key: d.arg.to_owned(),
            label: d.label.to_owned(),
            value: (!d.secret).then(|| value.to_string()),
            secret: d.secret,
            multiline: multiline(d.arg),
        });
    }
    let uris = entry.uris();
    if !uris.is_empty() {
        fields.push(VaultField {
            key: "uris".to_owned(),
            label: if uris.len() == 1 {
                "Website"
            } else {
                "Websites"
            }
            .to_owned(),
            value: Some(uris.join("\n")),
            secret: false,
            multiline: uris.len() > 1,
        });
    }
    for c in entry.custom_fields() {
        // Hidden and linked fields are revealed; text and yes/no fields are shown.
        let secret = !matches!(c.kind, 0 | 2);
        fields.push(VaultField {
            key: format!("{CUSTOM}{}", c.name),
            label: c.name.clone(),
            value: (!secret).then(|| c.value.to_string()),
            secret,
            multiline: false,
        });
    }
    if entry.dec("/notes").is_some_and(|n| !n.is_empty()) {
        fields.push(VaultField {
            key: "notes".to_owned(),
            label: "Notes".to_owned(),
            value: None,
            secret: true,
            multiline: true,
        });
    }
    Ok(VaultItemDetail {
        id: entry.id.clone(),
        name: entry.name(),
        kind: kind_view(entry.kind),
        fields,
        uses: uses(entry),
        warning: warning(&snap, entry),
        favorite: entry.favorite(),
    })
}

/// One field's value, for the user who asked to see or copy it.
pub(crate) async fn reveal(vault: &Vault, account: &str, id: &str, key: &str) -> Result<String, CoreError> {
    let snap = Snapshot::load(vault, account).await?;
    let entry = active(&snap, id)?;
    let missing = || CoreError::service("That item has no such field.");
    let value: Zeroizing<String> = if key == "notes" {
        entry.dec("/notes").ok_or_else(missing)?
    } else if key == "uris" {
        Zeroizing::new(entry.uris().join("\n"))
    } else if let Some(name) = key.strip_prefix(CUSTOM) {
        entry.custom_fields().into_iter().find(|c| c.name == name).map(|c| c.value).ok_or_else(missing)?
    } else {
        let d = defs(entry.kind).iter().find(|d| d.arg == key).ok_or_else(missing)?;
        entry.dec(&pointer(entry.kind, d.key)).ok_or_else(missing)?
    };
    if value.is_empty() {
        return Err(missing());
    }
    Ok(value.to_string())
}

/// `ssh-ed25519 AAAA… comment` of an OpenSSH private key (no passphrase).
pub(super) fn public_of_private(pem: &str) -> Result<String, CoreError> {
    let private = PrivateKey::from_openssh(pem.trim().as_bytes())
        .map_err(|_| CoreError::service("That is not an OpenSSH private key (-----BEGIN OPENSSH PRIVATE KEY-----)."))?;
    if private.is_encrypted() {
        return Err(CoreError::service(
            "The key has a passphrase. Remove it first (ssh-keygen -p -N \"\" -f KEY), or make a new key on the phone.",
        ));
    }
    private.public_key().to_openssh().map_err(|_| CoreError::service("The SSH key could not be read."))
}

/// The arguments of `item_create` or `item_update` for what the user entered. `existing` is the item being changed.
pub(super) fn args_for(
    kind: Kind,
    input: &VaultItemInput,
    existing: Option<&Entry>,
) -> Result<Map<String, Value>, CoreError> {
    let mut args = Map::new();
    let mut fields = Map::new();
    let mut custom = Vec::new();
    let mut removed = Vec::new();
    for VaultFieldInput {
        key,
        value,
    } in &input.fields
    {
        if key == "notes" {
            args.insert("notes".to_owned(), json!(value));
        } else if key == "uris" {
            let list: Vec<&str> = value.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
            fields.insert("uris".to_owned(), json!(list));
        } else if let Some(name) = key.strip_prefix(CUSTOM) {
            let kept = existing.and_then(|e| e.custom_fields().into_iter().find(|c| c.name == name));
            if value.is_empty() {
                if kept.is_some() {
                    removed.push(json!(name));
                }
                continue;
            }
            let kind = match kept.map(|c| c.kind) {
                Some(0) => "text",
                Some(2) => "boolean",
                _ => "hidden",
            };
            custom.push(json!({"name": name, "value": value, "type": kind}));
        } else {
            fields.insert(key.clone(), json!(value));
        }
    }
    if kind == Kind::SshKey
        && let Some(pem) = fields.get("private_key").and_then(Value::as_str).filter(|p| !p.trim().is_empty())
    {
        // The public half and the fingerprint follow from the private key.
        let public = public_of_private(pem)?;
        fields.insert("private_key".to_owned(), json!(pem.trim()));
        fields.insert("public_key".to_owned(), json!(public));
        fields.remove("fingerprint");
    }
    args.insert("type".to_owned(), json!(kind.slug()));
    args.insert("name".to_owned(), json!(input.name.trim()));
    if !fields.is_empty() {
        args.insert("fields".to_owned(), Value::Object(fields));
    }
    if !custom.is_empty() {
        args.insert("custom_fields".to_owned(), Value::Array(custom));
    }
    if !removed.is_empty() {
        args.insert("remove_custom_fields".to_owned(), Value::Array(removed));
    }
    Ok(args)
}

fn call(op: &str, args: Map<String, Value>) -> ConnectorCall {
    ConnectorCall {
        service: VAULT.to_owned(),
        op: op.to_owned(),
        args,
    }
}

/// Plans and makes a change on a vault already read; the vault's answer.
async fn change(vault: &Vault, snap: &Snapshot, call: &ConnectorCall) -> Result<Value, CoreError> {
    let plan = plan_with(snap, call)?;
    carry_out(vault, &snap.session, plan).await
}

/// Creates an item; its id.
pub(crate) async fn create(vault: &Vault, account: &str, input: &VaultItemInput) -> Result<String, CoreError> {
    if input.name.trim().is_empty() {
        return Err(CoreError::service("Give the item a name."));
    }
    let snap = Snapshot::load(vault, account).await?;
    let args = args_for(kind_of(input.kind), input, None)?;
    let done = change(vault, &snap, &call("item_create", args)).await?;
    done["id"].as_str().map(str::to_owned).ok_or_else(|| CoreError::service("The vault did not say what it created."))
}

/// Changes the fields given (and the name).
pub(crate) async fn update(vault: &Vault, account: &str, id: &str, input: &VaultItemInput) -> Result<(), CoreError> {
    if input.name.trim().is_empty() {
        return Err(CoreError::service("Give the item a name."));
    }
    let snap = Snapshot::load(vault, account).await?;
    let entry = active(&snap, id)?;
    let mut args = args_for(entry.kind, input, Some(entry))?;
    args.remove("type");
    args.insert("item".to_owned(), json!(id));
    change(vault, &snap, &call("item_update", args)).await.map(drop)
}

/// Deletes an item for good.
pub(crate) async fn delete(vault: &Vault, account: &str, id: &str) -> Result<(), CoreError> {
    let snap = Snapshot::load(vault, account).await?;
    active(&snap, id)?;
    let args = Map::from_iter([("item".to_owned(), json!(id))]);
    change(vault, &snap, &call("item_delete", args)).await.map(drop)
}

/// A new Ed25519 key, made here; its private half goes into a new SSH key item and never leaves the vault.
pub(super) fn new_ed25519(comment: &str) -> Result<(Zeroizing<String>, String, String), CoreError> {
    let seed = Zeroizing::new(crypto::random_bytes::<32>()?);
    let keypair = Ed25519Keypair::from_seed(&seed);
    let private = PrivateKey::new(KeypairData::Ed25519(keypair), text::one_line(comment))
        .map_err(|_| CoreError::service("The SSH key could not be made."))?;
    let pem = private.to_openssh(LineEnding::LF).map_err(|_| CoreError::service("The SSH key could not be made."))?;
    let public = private.public_key().to_openssh().map_err(|_| CoreError::service("The SSH key could not be made."))?;
    let fingerprint = private.public_key().fingerprint(HashAlg::Sha256).to_string();
    Ok((Zeroizing::new(pem.to_string()), public, fingerprint))
}

pub(crate) async fn generate_ssh_key(vault: &Vault, account: &str, name: &str) -> Result<VaultSshKey, CoreError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(CoreError::service("Give the key a name."));
    }
    let (pem, public, fingerprint) = new_ed25519(name)?;
    let input = VaultItemInput {
        kind: VaultItemKind::SshKey,
        name: name.to_owned(),
        fields: vec![VaultFieldInput {
            key: "private_key".to_owned(),
            value: pem.to_string(),
        }],
    };
    let id = create(vault, account, &input).await?;
    Ok(VaultSshKey {
        id,
        public_key: public,
        fingerprint,
    })
}

/// This phone's inbox public key (base64url), which the desktop app seals secrets to.
pub(crate) fn inbox_public_key(store: &Store) -> Result<String, CoreError> {
    let secret = crypto_box::SecretKey::from(*store.inbox_key()?);
    Ok(reins_proto::desktop::encode_key(secret.public_key().as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_made_key_reads_back_as_its_public_half() {
        let (pem, public, fingerprint) = new_ed25519("deploy key").unwrap();
        assert!(pem.starts_with("-----BEGIN OPENSSH PRIVATE KEY-----"));
        assert!(public.starts_with("ssh-ed25519 AAAA") && public.ends_with(" deploy key"), "{public}");
        assert!(fingerprint.starts_with("SHA256:"));
        assert_eq!(public_of_private(&pem).unwrap(), public);
        assert!(public_of_private("ssh-ed25519 AAAA").is_err());
        let other = new_ed25519("x").unwrap();
        assert_ne!(other.1.split(' ').nth(1), public.split(' ').nth(1));
    }

    #[test]
    fn what_the_user_entered_becomes_tool_arguments() {
        let input = VaultItemInput {
            kind: VaultItemKind::Login,
            name: " OpenAI ".to_owned(),
            fields: vec![
                VaultFieldInput {
                    key: "password".to_owned(),
                    value: "sk-1".to_owned(),
                },
                VaultFieldInput {
                    key: "uris".to_owned(),
                    value: "https://platform.openai.com\n\n".to_owned(),
                },
                VaultFieldInput {
                    key: "custom:Org".to_owned(),
                    value: "org-1".to_owned(),
                },
                VaultFieldInput {
                    key: "notes".to_owned(),
                    value: "billing".to_owned(),
                },
            ],
        };
        let args = args_for(Kind::Login, &input, None).unwrap();
        assert_eq!(
            Value::Object(args),
            json!({"type": "login", "name": "OpenAI", "notes": "billing",
                "fields": {"password": "sk-1", "uris": ["https://platform.openai.com"]},
                "custom_fields": [{"name": "Org", "value": "org-1", "type": "hidden"}]})
        );
        let ssh = VaultItemInput {
            kind: VaultItemKind::SshKey,
            name: "k".to_owned(),
            fields: vec![VaultFieldInput {
                key: "private_key".to_owned(),
                value: "not a key".to_owned(),
            }],
        };
        assert!(args_for(Kind::SshKey, &ssh, None).is_err());
    }
}
