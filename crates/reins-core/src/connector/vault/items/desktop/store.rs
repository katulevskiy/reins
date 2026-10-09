//! `reins vault add` and `reins vault list`: the desktop app saves a value from the computer into the vault, and lists
//! the items' names.
//!
//! - `phone_key`: the public half of this phone's inbox key ([`Store::inbox_key`]), sealed to the computer. The user
//!   approves it once, the approval shows its twelve digits ([`phone_key_fingerprint`]), the user types them on the
//!   computer, and the computer keeps the key. The key never travels in the clear, so a server that answered with its
//!   own key could not know which digits to aim for.
//! - `secret_store`: the value, in a box from the computer's pinned key to that key, so the server can neither read nor
//!   replace it. The box also says when it was made, the item, the field, the kind of a new item and whether an
//!   existing item may be changed; a box older than [`SECRET_STORE_TTL_SECS`], or one seen before, is refused, so a
//!   captured request cannot be sent again. The phone shows what will be saved (never the value, but four check digits
//!   the terminal shows too) and, once approved, makes the item, or changes the field of the item with that name when
//!   the box says `replace`, keeping the earlier value. The item the approval was about must still be the one changed.
//! - `names`: the items' names and kinds, asked for every time.
//!
//! What goes back to the computer after the key is pinned (a saved value's acknowledgement, the names) is boxed from the
//! phone's inbox key to the computer's key, so the computer knows its pinned phone wrote it.
//!
//! [`Store::inbox_key`]: crate::store::Store::inbox_key

use std::collections::BTreeMap;

use reins_proto::connector::ConnectorCall;
use reins_proto::desktop::{
    PHONE_KEY_CHANGED, PhoneKey, SEALED_FIELD, SECRET_STORE_TTL_SECS, SecretToStore, StoreAck, TO_DESKTOP, TO_PHONE,
    VaultNames, phone_key_fingerprint, value_check,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use zeroize::Zeroizing;

use super::super::editor::{args_for, inbox_public_key, public_of_private};
use super::super::model::{Entry, Kind, Snapshot, State};
use super::super::write::{carry_out, plan_with, ssh_fingerprint};
use super::bad;
use crate::connector::Preview;
use crate::connector::sealed::{box_from_phone, client_key, nonce_arg, open_from, seal};
use crate::connector::vault::Vault;
use crate::store::{Store, unix_now};
use crate::vault_editor::{VaultFieldInput, VaultItemInput, VaultItemKind};
use crate::{CoreError, text};

/// The resource of the phone's key and of the list of names.
const PHONE_KEY: &str = "phone_key";
const NAMES: &str = "names";
/// Where the requests seen are remembered (a `meta` key): nonce → what the approval was about, and whether it was done.
const SEEN: &str = "vault.secret-store.seen";
/// A box made this far ahead of the phone's clock is refused too.
const CLOCK_SKEW_SECS: i64 = 5 * 60;

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
            "The computer asks you to type these twelve digits once you approve.".to_owned(),
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
struct Request {
    nonce: String,
    created_at: i64,
    name: String,
    /// `api-key`, `login`, `note` or `ssh`.
    kind_arg: String,
    /// The kind of a new item.
    kind: Kind,
    /// `password`, `username`, `notes`, `private_key`, or a custom field's name.
    field: String,
    replace: bool,
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

/// Opens the value boxed to this phone and checks it answers this very call: its nonce, a recent time, and the item,
/// field and kind the call names.
fn open(vault: &Vault, call: &ConnectorCall, now: i64) -> Result<Request, CoreError> {
    let desktop = client_key(call)?;
    let nonce = nonce_arg(call)?;
    let name = text::one_line(call.str_arg("name").unwrap_or_default()).trim().to_owned();
    if name.is_empty() {
        return Err(bad("The item needs a name."));
    }
    let kind_given = call.str_arg("kind").unwrap_or_default().to_owned();
    let kind = kind_arg(&kind_given)?;
    let field = field_arg(call.str_arg("field").unwrap_or_default());
    if field.is_empty() || field.chars().any(char::is_control) {
        return Err(bad("`field` must name a field."));
    }
    // A box from the app's key, which the flow checked is the one pinned for this connection: the server can neither
    // read the value nor put another one in its place. A box that does not open was made for another phone's key (a
    // new phone, the app reinstalled), or by someone else.
    let unreadable = || bad(format!("{PHONE_KEY_CHANGED} If this is a new phone, run reins vault add --new-phone."));
    let plain = open_from(desktop, &*vault.store.inbox_key()?, call.str_arg("sealed").unwrap_or_default())
        .ok_or_else(unreadable)?;
    let opened: SecretToStore = serde_json::from_slice(&plain).map_err(|_| unreadable())?;
    if opened.dir != TO_PHONE || opened.nonce != nonce {
        return Err(bad("The value was sealed for another request; refused."));
    }
    if opened.created_at < now - SECRET_STORE_TTL_SECS || opened.created_at > now + CLOCK_SKEW_SECS {
        return Err(bad(
            "The value was sealed too long ago (or the computer's clock is wrong); refused. Run reins vault add again.",
        ));
    }
    if text::one_line(&opened.name).trim() != name || field_arg(&opened.field) != field || opened.kind != kind_given {
        return Err(bad("The value was sealed for another item, field or kind; refused."));
    }
    if opened.value.is_empty() {
        return Err(bad("The value is empty."));
    }
    Ok(Request {
        value: Zeroizing::new(opened.value.clone()),
        nonce,
        created_at: opened.created_at,
        name,
        kind_arg: kind_given,
        kind,
        field,
        replace: opened.replace,
    })
}

/// One request the phone has seen: what its approval was about (`new`, or an item at a revision), and whether it was
/// carried out.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Seen {
    target: String,
    until: i64,
    done: bool,
}

fn seen_all(store: &Store, now: i64) -> Result<BTreeMap<String, Seen>, CoreError> {
    let mut all: BTreeMap<String, Seen> =
        store.meta_get(SEEN)?.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    all.retain(|_, s| s.until >= now);
    Ok(all)
}

fn seen_save(store: &Store, all: &BTreeMap<String, Seen>) -> Result<(), CoreError> {
    store.meta_set(SEEN, &serde_json::to_string(all).map_err(|_| CoreError::storage("cannot record the request"))?)
}

const ALREADY: &str =
    "This request was seen already; a request is never shown or saved twice. Run reins vault add again.";

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

/// What the approval is about: a new item, or this item as it is now.
fn target_of(found: Option<&Entry>) -> String {
    found.map_or_else(|| "new".to_owned(), |e| format!("{}@{}", e.id, e.raw["revisionDate"].as_str().unwrap_or("")))
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

/// The value the field has now, if any.
fn current(entry: &Entry, field: &str) -> Option<Zeroizing<String>> {
    let value = match field {
        "notes" => entry.dec("/notes"),
        "password" => entry.dec("/login/password"),
        "username" => entry.dec("/login/username"),
        "private_key" => entry.dec("/sshKey/privateKey"),
        custom => entry.custom_fields().into_iter().find(|c| c.name == custom).map(|c| c.value),
    };
    value.filter(|v| !v.is_empty())
}

/// A name for the hidden field that keeps an earlier value, free on the item: "Notes before 2026-10-09".
fn backup_name(entry: &Entry, field: &str, now: i64) -> String {
    let what = match field {
        "notes" => "Notes",
        "username" => "Username",
        custom => custom,
    };
    let day = text::iso_utc(now).chars().take(10).collect::<String>();
    let taken: Vec<String> = entry.custom_fields().into_iter().map(|c| c.name).collect();
    let base = format!("{what} before {day}");
    (1..1_000)
        .map(|n| {
            if n == 1 {
                base.clone()
            } else {
                format!("{base} ({n})")
            }
        })
        .find(|candidate| !taken.contains(candidate))
        .unwrap_or(base)
}

/// The change to make, the lines that describe it, what the approval is about and where an earlier value goes.
struct Planned {
    call: ConnectorCall,
    lines: Vec<String>,
    target: String,
    kept: Option<String>,
}

fn plan(snap: &Snapshot, req: &Request, now: i64) -> Result<Planned, CoreError> {
    let found = existing(snap, &req.name)?;
    if let Some(e) = found {
        if !req.replace {
            return Err(bad(format!(
                "{} is already in your vault. To change its {}, run reins vault add {} --replace.",
                req.name,
                label(&req.field),
                shell_word(&req.name)
            )));
        }
        if e.kind == Kind::SshKey {
            return Err(bad(format!(
                "{} is an SSH key, which is not replaced: add the new key under another name, then delete the old one \
                 in the Reins app.",
                req.name
            )));
        }
    }
    let kind = found.map_or(req.kind, |e| e.kind);
    if !holds(kind, &req.field) {
        return Err(bad(match found {
            Some(e) => format!(
                "{} is a {}, which has no {}. Use another field or another name.",
                req.name,
                e.kind.label().to_lowercase(),
                label(&req.field)
            ),
            None => format!("A new {} has no {}.", kind_word(&req.kind_arg, kind), label(&req.field)),
        }));
    }
    let custom = !matches!(req.field.as_str(), "password" | "username" | "notes" | "private_key");
    let mut fields = vec![VaultFieldInput {
        key: if custom {
            format!("custom:{}", req.field)
        } else {
            req.field.clone()
        },
        value: req.value.to_string(),
    }];
    // The earlier value is kept: a login's password goes to its history, anything else to a hidden field.
    let mut kept = None;
    if let Some(e) = found
        && let Some(old) = current(e, &req.field)
    {
        if req.field == "password" {
            kept = Some("the item's password history".to_owned());
        } else {
            let backup = backup_name(e, &req.field, now);
            fields.push(VaultFieldInput {
                key: format!("custom:{backup}"),
                value: old.to_string(),
            });
            kept = Some(format!("the hidden field \u{201c}{backup}\u{201d}"));
        }
    }
    let input = VaultItemInput {
        kind: VaultItemKind::Login,
        name: req.name.clone(),
        fields,
    };
    let mut args: Map<String, Value> = args_for(kind, &input, found)?;
    // A value sent this way is a secret: a custom field holding it is hidden, whatever it was before.
    if let Some(Value::Array(list)) = args.get_mut("custom_fields") {
        for c in list {
            c["type"] = json!("hidden");
        }
    }
    let check = value_check(&req.value);
    let mut lines = vec![match found {
        None => format!("Save a new {} \u{201c}{}\u{201d} in your vault", kind_word(&req.kind_arg, kind), req.name),
        Some(_) => format!("Replace the {} of \u{201c}{}\u{201d} in your vault", label(&req.field), req.name),
    }];
    if kind == Kind::SshKey {
        let fingerprint = ssh_fingerprint(&public_of_private(&req.value)?).unwrap_or_default();
        lines.push(format!(
            "SSH key {fingerprint}, sent by reins vault add on the computer (the private key is not shown)"
        ));
    } else {
        lines.push(format!(
            "The {}: {} characters, sent by reins vault add on the computer (typed or piped in; not shown)",
            label(&req.field),
            req.value.chars().count()
        ));
    }
    lines.push(format!("Check: {check}. The terminal shows the same four digits; deny if it does not."));
    if let Some(where_) = &kept {
        lines.push(format!("The earlier value is kept in {where_}"));
    }
    lines.push(if kind == Kind::SshKey {
        "The SSH agent of the desktop app (reins ssh setup) offers it; signing stays on the phone".to_owned()
    } else {
        format!("Use it as vault:{}/{}", req.name, req.field)
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
        target: target_of(found),
        kept,
    })
}

/// A name as the user would type it after `reins vault add`.
fn shell_word(name: &str) -> String {
    if name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/')) {
        name.to_owned()
    } else {
        format!("'{}'", name.replace('\'', r"'\''"))
    }
}

pub(super) async fn preview_store(vault: &Vault, account: &str, call: &ConnectorCall) -> Result<Preview, CoreError> {
    let now = unix_now();
    let req = open(vault, call, now)?;
    let mut seen = seen_all(&vault.store, now)?;
    // One preview per request: a box sent again must not move what the approval is about.
    if seen.contains_key(&req.nonce) {
        return Err(bad(ALREADY));
    }
    let snap = Snapshot::load(vault, account).await?;
    let planned = plan(&snap, &req, now)?;
    // The vault tools' own checks, and the resource the change is about.
    let inner = plan_with(&snap, &planned.call)?.preview;
    seen.insert(
        req.nonce.clone(),
        Seen {
            target: planned.target,
            until: req.created_at + SECRET_STORE_TTL_SECS,
            done: false,
        },
    );
    seen_save(&vault.store, &seen)?;
    Ok(Preview {
        lines: planned.lines,
        once_only: true,
        ..inner
    })
}

pub(super) async fn perform_store(vault: &Vault, account: &str, call: &ConnectorCall) -> Result<Value, CoreError> {
    let now = unix_now();
    let req = open(vault, call, now)?;
    let mut seen = seen_all(&vault.store, now)?;
    let Some(asked) = seen.get(&req.nonce).filter(|s| !s.done).cloned() else {
        return Err(bad(ALREADY));
    };
    let snap = Snapshot::load(vault, account).await?;
    let planned = plan(&snap, &req, now)?;
    // What happens must be what the user approved: not another item, nor the same one changed since.
    if planned.target != asked.target {
        return Err(bad("The vault changed since you were asked; nothing was saved. Run reins vault add again."));
    }
    let created = planned.call.op == "item_create";
    carry_out(vault, &snap.session, plan_with(&snap, &planned.call)?).await?;
    seen.insert(
        req.nonce.clone(),
        Seen {
            done: true,
            ..asked
        },
    );
    seen_save(&vault.store, &seen)?;
    let ack = StoreAck {
        v: 1,
        dir: TO_DESKTOP.to_owned(),
        nonce: req.nonce,
        name: req.name,
        field: req.field,
        created,
        kept: planned.kept,
    };
    Ok(json!({ SEALED_FIELD: box_from_phone(client_key(call)?, &*vault.store.inbox_key()?, &ack)? }))
}

/// The names and kinds of the items in use, by name.
async fn names_of(vault: &Vault, account: &str) -> Result<Vec<(String, String)>, CoreError> {
    let snap = Snapshot::load(vault, account).await?;
    let mut items: Vec<(String, String)> = snap
        .entries
        .iter()
        .filter(|e| e.state() == State::Active)
        .map(|e| (e.name(), e.kind.slug().to_owned()))
        .collect();
    items.sort_by_key(|(name, _)| name.to_lowercase());
    Ok(items)
}

pub(super) async fn preview_names(vault: &Vault, account: &str, call: &ConnectorCall) -> Result<Preview, CoreError> {
    client_key(call)?;
    nonce_arg(call)?;
    let count = names_of(vault, account).await?.len();
    Ok(Preview {
        resource: NAMES.to_owned(),
        resource_label: "Names of your vault items".to_owned(),
        lines: vec![
            format!("Show the names of your {count} vault items on the computer (reins vault list)"),
            "Only names and kinds: no passwords or other values".to_owned(),
        ],
        parents: Vec::new(),
        once_only: true,
        ..Preview::default()
    })
}

pub(super) async fn perform_names(vault: &Vault, account: &str, call: &ConnectorCall) -> Result<Value, CoreError> {
    let answer = VaultNames {
        v: 1,
        dir: TO_DESKTOP.to_owned(),
        nonce: nonce_arg(call)?,
        items: names_of(vault, account).await?,
    };
    Ok(json!({ SEALED_FIELD: box_from_phone(client_key(call)?, &*vault.store.inbox_key()?, &answer)? }))
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
        assert_eq!(shell_word("OpenAI"), "OpenAI");
        assert_eq!(shell_word("AWS prod"), "'AWS prod'");
    }

    #[test]
    fn the_requests_seen_survive_a_restart_until_they_expire() {
        let dir = tempfile::tempdir().unwrap();
        let seen = Seen {
            target: "bank@2026-01-01".to_owned(),
            until: 2_000,
            done: true,
        };
        {
            let store = crate::store::tests::open(dir.path());
            seen_save(&store, &BTreeMap::from([("n1".to_owned(), seen.clone())])).unwrap();
        }
        let store = crate::store::tests::open(dir.path());
        assert_eq!(seen_all(&store, 1_000).unwrap().get("n1"), Some(&seen), "kept across a restart");
        assert!(seen_all(&store, 2_001).unwrap().is_empty(), "forgotten once it could no longer be used");
    }
}
