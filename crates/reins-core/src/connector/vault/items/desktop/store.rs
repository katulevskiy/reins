//! `reins vault add` and `reins vault list`: the desktop app saves a value typed on the computer into the vault, and
//! lists the items' names.
//!
//! - `phone_key`: the public half of this phone's inbox key ([`Store::inbox_key`]), sealed to the computer. The user
//!   approves it once, the approval shows its eight digits ([`phone_key_fingerprint`]), the user types them on the
//!   computer, and the computer keeps the key. The key never travels in the clear, so a server that answered with its
//!   own key could not know which digits to aim for.
//! - `secret_store`: the value, in a box from the computer's pinned key to that key, so the server can neither read
//!   nor replace it. The phone opens it,
//!   shows what will be saved (never the value) and, once approved, creates the item or changes the field of the item
//!   with that name, encrypted with the vault key like any change.
//! - `names`: the items' names and kinds, sealed to the computer.
//!
//! [`Store::inbox_key`]: crate::store::Store::inbox_key

use reins_proto::connector::ConnectorCall;
use reins_proto::desktop::{
    PHONE_KEY_CHANGED, PhoneKey, SEALED_FIELD, SecretToStore, VaultNames, phone_key_fingerprint,
};
use serde_json::{Map, Value, json};
use zeroize::Zeroizing;

use super::super::editor::{inbox_public_key, public_of_private};
use super::super::model::{Entry, Kind, Snapshot, State};
use super::super::write::{carry_out, plan_with};
use super::bad;
use crate::connector::sealed::{client_key, nonce_arg, open_from, seal};
use crate::connector::vault::Vault;
use crate::connector::{Item, Preview};
use crate::vault_editor::{VaultFieldInput, VaultItemInput, VaultItemKind};
use crate::{CoreError, text};

/// The resource of the phone's key and of the list of names.
const PHONE_KEY: &str = "phone_key";
const NAMES: &str = "names";

pub(super) fn preview_phone_key(vault: &Vault, call: &ConnectorCall) -> Result<Preview, CoreError> {
    client_key(call)?;
    nonce_arg(call)?;
    let public = inbox_public_key(&vault.store)?;
    let digits = phone_key_fingerprint(&public).unwrap_or_default();
    Ok(Preview {
        resource: PHONE_KEY.to_owned(),
        resource_label: "This phone's key".to_owned(),
        lines: vec![
            "Let this computer save secrets in your vault (reins vault add)".to_owned(),
            format!("This phone's key: {digits}"),
            "The computer asks you to type this number once you approve.".to_owned(),
        ],
        parents: Vec::new(),
        once_only: true,
        ..Preview::default()
    })
}

pub(super) fn perform_phone_key(vault: &Vault, call: &ConnectorCall) -> Result<Value, CoreError> {
    let answer = PhoneKey {
        v: 1,
        nonce: nonce_arg(call)?,
        public_key: inbox_public_key(&vault.store)?,
    };
    Ok(json!({ SEALED_FIELD: seal(client_key(call)?, &answer)? }))
}

/// What the computer asked to save.
struct Store {
    name: String,
    /// The kind of a new item.
    kind: Kind,
    /// `password`, `username`, `notes`, `private_key`, or a custom field's name.
    field: String,
    value: Zeroizing<String>,
}

/// The kind of a new item from `--kind`; an API key is a login with only a password.
fn kind_arg(kind: &str) -> Result<Kind, CoreError> {
    Ok(match kind {
        "api-key" | "login" => Kind::Login,
        "note" => Kind::Note,
        "ssh" => Kind::SshKey,
        _ => return Err(bad("`kind` must be api-key, login, note or ssh.")),
    })
}

fn kind_word(kind: &str, given: Kind) -> &'static str {
    match (kind, given) {
        ("api-key", _) => "API key",
        (_, Kind::Note) => "secure note",
        (_, Kind::SshKey) => "SSH key",
        _ => "login",
    }
}

/// The field, as the vault tools name it: the built-in ones in lower case, any other a custom field.
fn field_arg(field: &str) -> String {
    let lower = field.trim().to_ascii_lowercase();
    match lower.as_str() {
        "password" | "username" | "notes" | "private_key" => lower,
        _ => field.trim().to_owned(),
    }
}

/// Opens the value sealed to this phone, after checking the call names this phone's key and echoes its nonce.
fn open(vault: &Vault, call: &ConnectorCall) -> Result<Store, CoreError> {
    client_key(call)?;
    let nonce = nonce_arg(call)?;
    let name = text::one_line(call.str_arg("name").unwrap_or_default()).trim().to_owned();
    if name.is_empty() {
        return Err(bad("The item needs a name."));
    }
    let kind = kind_arg(call.str_arg("kind").unwrap_or_default())?;
    let field = field_arg(call.str_arg("field").unwrap_or_default());
    if field.is_empty() || field.chars().any(char::is_control) {
        return Err(bad("`field` must name a field."));
    }
    // A box from the app's key, which the flow checked is the one pinned for this connection: the server can neither
    // read the value nor put another one in its place.
    // A box for another key: the computer kept the key of a phone this is not (a new phone, the app reinstalled).
    let unreadable = || bad(format!("{PHONE_KEY_CHANGED} Run reins vault add again: it asks for this phone's key."));
    let plain = open_from(client_key(call)?, &*vault.store.inbox_key()?, call.str_arg("sealed").unwrap_or_default())
        .ok_or_else(unreadable)?;
    let opened: SecretToStore = serde_json::from_slice(&plain).map_err(|_| unreadable())?;
    let value = Zeroizing::new(opened.value);
    if opened.nonce != nonce {
        return Err(bad("The value was sealed for another request; refused."));
    }
    if text::one_line(&opened.name).trim() != name
        || opened.field.trim() != call.str_arg("field").unwrap_or_default().trim()
    {
        return Err(bad("The value was sealed for another item or field; refused."));
    }
    if value.is_empty() {
        return Err(bad("The value is empty."));
    }
    Ok(Store {
        name,
        kind,
        field,
        value,
    })
}

/// The one item in use with this exact name, if any.
fn existing<'a>(snap: &'a Snapshot, name: &str) -> Result<Option<&'a Entry>, CoreError> {
    let mut same = snap.entries.iter().filter(|e| e.state() != State::Trash && e.name() == name);
    match (same.next(), same.next()) {
        (None, _) => Ok(None),
        (Some(e), None) => Ok(Some(e)),
        (Some(_), Some(_)) => Err(bad(format!(
            "Several vault items are named {name}. Rename one in the Reins app (Vault) and try again."
        ))),
    }
}

/// Whether an item of `kind` can hold `field`.
fn holds(kind: Kind, field: &str) -> bool {
    match field {
        "password" | "username" => kind == Kind::Login,
        "private_key" => kind == Kind::SshKey,
        _ => true,
    }
}

fn label(field: &str) -> String {
    match field {
        "private_key" => "private key".to_owned(),
        "notes" | "password" | "username" => field.to_owned(),
        custom => format!("field \u{201c}{custom}\u{201d}"),
    }
}

/// The change to make, and the lines that describe it.
struct Planned {
    call: ConnectorCall,
    lines: Vec<String>,
}

fn plan(snap: &Snapshot, store: &Store, kind_given: &str) -> Result<Planned, CoreError> {
    let found = existing(snap, &store.name)?;
    let kind = found.map_or(store.kind, |e| e.kind);
    if !holds(kind, &store.field) {
        return Err(bad(match found {
            Some(e) => format!(
                "{} is a {}, which has no {}. Use another field or another name.",
                store.name,
                e.kind.label().to_lowercase(),
                label(&store.field)
            ),
            None => format!("A new {} has no {}.", kind_word(kind_given, kind), label(&store.field)),
        }));
    }
    let key = match store.field.as_str() {
        "password" | "username" | "notes" | "private_key" => store.field.clone(),
        custom => format!("custom:{custom}"),
    };
    let input = VaultItemInput {
        kind: VaultItemKind::Login,
        name: store.name.clone(),
        fields: vec![VaultFieldInput {
            key,
            value: store.value.to_string(),
        }],
    };
    let mut args: Map<String, Value> = super::super::editor::args_for(kind, &input, found)?;
    let what = if kind == Kind::SshKey {
        let public = public_of_private(&store.value)?;
        let fingerprint = super::super::write::ssh_fingerprint(&public).unwrap_or_default();
        format!("SSH key {fingerprint}, typed on the computer (the private key is not shown)")
    } else {
        format!(
            "The {}: {} characters, typed on the computer (not shown)",
            label(&store.field),
            store.value.chars().count()
        )
    };
    let mut lines = vec![match found {
        None => format!("Save a new {} \u{201c}{}\u{201d} in your vault", kind_word(kind_given, kind), store.name),
        Some(_) => format!("Change the {} of \u{201c}{}\u{201d} in your vault", label(&store.field), store.name),
    }];
    lines.push(what);
    lines.push(if kind == Kind::SshKey {
        "The SSH agent of the desktop app (reins ssh setup) offers it; signing stays on the phone".to_owned()
    } else {
        format!("Use it as vault:{}/{}", store.name, store.field)
    });
    let op = match found {
        None => "item_create",
        Some(e) => {
            args.remove("type");
            args.insert("item".to_owned(), json!(e.id));
            "item_update"
        }
    };
    Ok(Planned {
        call: ConnectorCall {
            service: reins_proto::connector::VAULT.to_owned(),
            op: op.to_owned(),
            args,
        },
        lines,
    })
}

pub(super) async fn preview_store(vault: &Vault, account: &str, call: &ConnectorCall) -> Result<Preview, CoreError> {
    let store = open(vault, call)?;
    let snap = Snapshot::load(vault, account).await?;
    let planned = plan(&snap, &store, call.str_arg("kind").unwrap_or_default())?;
    // The vault tools' own checks, and the resource the change is about.
    let inner = plan_with(&snap, &planned.call)?.preview;
    Ok(Preview {
        lines: planned.lines,
        once_only: true,
        ..inner
    })
}

pub(super) async fn perform_store(vault: &Vault, account: &str, call: &ConnectorCall) -> Result<Value, CoreError> {
    let store = open(vault, call)?;
    let snap = Snapshot::load(vault, account).await?;
    let planned = plan(&snap, &store, call.str_arg("kind").unwrap_or_default())?;
    let created = planned.call.op == "item_create";
    let done = carry_out(vault, &snap.session, plan_with(&snap, &planned.call)?).await?;
    Ok(json!({
        "saved": true,
        "created": created,
        "id": done["id"],
        "name": store.name,
        "field": store.field,
    }))
}

pub(super) async fn names(vault: &Vault, account: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
    client_key(call)?;
    let snap = Snapshot::load(vault, account).await?;
    let mut items: Vec<(String, String)> = snap
        .entries
        .iter()
        .filter(|e| e.state() == State::Active)
        .map(|e| (e.name(), e.kind.slug().to_owned()))
        .collect();
    items.sort_by_key(|(name, _)| name.to_lowercase());
    let count = items.len();
    let answer = VaultNames {
        v: 1,
        nonce: nonce_arg(call)?,
        items,
    };
    let mut extra = Map::new();
    extra.insert(SEALED_FIELD.to_owned(), json!(seal(client_key(call)?, &answer)?));
    let title = format!("The names of your {count} vault items (no passwords or other values)");
    Ok(vec![Item {
        id: NAMES.to_owned(),
        resource: NAMES.to_owned(),
        resource_label: "Names of your vault items".to_owned(),
        title: title.clone(),
        snippet: title,
        extra,
        ..Item::default()
    }])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_and_fields_are_named_as_the_vault_tools_name_them() {
        assert_eq!(kind_arg("api-key").unwrap(), Kind::Login);
        assert_eq!(kind_arg("ssh").unwrap(), Kind::SshKey);
        assert!(kind_arg("card").is_err());
        assert_eq!(field_arg("Password"), "password");
        assert_eq!(field_arg(" API key "), "API key");
        assert!(holds(Kind::Login, "password") && !holds(Kind::Note, "password"));
        assert!(holds(Kind::Note, "notes") && holds(Kind::Note, "Token"));
        assert!(holds(Kind::SshKey, "private_key") && !holds(Kind::Login, "private_key"));
    }
}
