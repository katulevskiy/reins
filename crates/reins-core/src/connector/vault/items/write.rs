//! Changing items: what a change would do (for the user) and doing it. Every string of an item is encrypted with the
//! item's key here, on the phone; the server only ever sees ciphertext. Nothing secret reaches a preview.

use std::sync::Arc;

use data_encoding::{BASE64, BASE64_NOPAD};
use reins_proto::connector::ConnectorCall;
use reqwest::Method;
use ring::digest;
use serde_json::{Map, Value, json};

use super::model::{Custom, Entry, Folder, Kind, Snapshot, State, encrypt_buffer};
use super::read::attachment_of;
use super::{MAX_FILE_BYTES, host_of};
use crate::connector::Preview;
use crate::connector::calendar::segment;
use crate::connector::vault::Vault;
use crate::connector::vault::totp::{totp_storable, unix_seconds};
use crate::crypto::{self, VaultKey};
use crate::session::Session;
use crate::{CoreError, text};

/// A type-specific field an AI may set.
pub(super) struct Def {
    /// The name in `fields`.
    pub arg: &'static str,
    /// The name in the cipher JSON.
    pub key: &'static str,
    pub label: &'static str,
    pub secret: bool,
}

const fn def(arg: &'static str, key: &'static str, label: &'static str, secret: bool) -> Def {
    Def {
        arg,
        key,
        label,
        secret,
    }
}

const LOGIN: &[Def] = &[
    def("username", "username", "Username", false),
    def("password", "password", "Password", true),
    def("totp", "totp", "One-time code", true),
];
const CARD: &[Def] = &[
    def("holder", "cardholderName", "Holder", false),
    def("brand", "brand", "Brand", false),
    def("number", "number", "Number", true),
    def("exp_month", "expMonth", "Expiry month", false),
    def("exp_year", "expYear", "Expiry year", false),
    def("code", "code", "Security code", true),
];
const IDENTITY: &[Def] = &[
    def("title", "title", "Title", false),
    def("first_name", "firstName", "First name", false),
    def("middle_name", "middleName", "Middle name", false),
    def("last_name", "lastName", "Last name", false),
    def("address1", "address1", "Address 1", false),
    def("address2", "address2", "Address 2", false),
    def("address3", "address3", "Address 3", false),
    def("city", "city", "City", false),
    def("state", "state", "State", false),
    def("postal_code", "postalCode", "Postal code", false),
    def("country", "country", "Country", false),
    def("company", "company", "Company", false),
    def("email", "email", "Email", false),
    def("phone", "phone", "Phone", false),
    def("ssn", "ssn", "Social security number", true),
    def("username", "username", "Username", false),
    def("passport_number", "passportNumber", "Passport number", true),
    def("license_number", "licenseNumber", "License number", true),
];
const SSH: &[Def] = &[
    def("private_key", "privateKey", "Private key", true),
    def("public_key", "publicKey", "Public key", false),
    def("fingerprint", "keyFingerprint", "Fingerprint", false),
];

pub(super) fn defs(kind: Kind) -> &'static [Def] {
    match kind {
        Kind::Login => LOGIN,
        Kind::Card => CARD,
        Kind::Identity => IDENTITY,
        Kind::SshKey => SSH,
        Kind::Note => &[],
    }
}

/// The longest value of one field (an SSH private key is the biggest thing here).
const MAX_VALUE: usize = 16_000;
const MAX_URIS: usize = 20;
const MAX_CUSTOM: usize = 50;
/// The most old passwords an item keeps, like the Bitwarden apps.
const MAX_HISTORY: usize = 5;

/// The `fields` of a call, checked.
#[derive(Default)]
struct Parsed {
    values: Vec<(&'static Def, String)>,
    uris: Option<Vec<String>>,
}

fn clean(what: &str, value: &str, max: usize) -> Result<(), CoreError> {
    if value.chars().count() > max {
        return Err(CoreError::service(format!("{what} is longer than {max} characters.")));
    }
    if value.chars().any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t')) {
        return Err(CoreError::service(format!("{what} contains control characters.")));
    }
    Ok(())
}

/// A JSON argument that an AI client may have sent as a string holding JSON.
fn json_arg(call: &ConnectorCall, name: &str) -> Result<Option<Value>, CoreError> {
    Ok(match call.args.get(name) {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => {
            Some(serde_json::from_str(s).map_err(|_| CoreError::service(format!("`{name}` must be a JSON value.")))?)
        }
        Some(v) => Some(v.clone()),
    })
}

fn parse_fields(kind: Kind, call: &ConnectorCall) -> Result<Parsed, CoreError> {
    let Some(value) = json_arg(call, "fields")? else {
        return Ok(Parsed::default());
    };
    let map = value.as_object().ok_or_else(|| CoreError::service("`fields` must be an object."))?;
    let mut parsed = Parsed::default();
    for (arg, v) in map {
        if arg == "uris" && kind == Kind::Login {
            let list = v.as_array().ok_or_else(|| CoreError::service("`fields.uris` must be a list of addresses."))?;
            if list.len() > MAX_URIS {
                return Err(CoreError::service(format!("`fields.uris` may have at most {MAX_URIS} addresses.")));
            }
            let mut uris = Vec::new();
            for u in list {
                let u = u.as_str().ok_or_else(|| CoreError::service("`fields.uris` must contain only text."))?.trim();
                if u.is_empty() {
                    continue;
                }
                clean("A website address", u, 2_000)?;
                uris.push(u.to_owned());
            }
            parsed.uris = Some(uris);
            continue;
        }
        let Some(d) = defs(kind).iter().find(|d| d.arg == arg) else {
            let mut allowed: Vec<&str> = defs(kind).iter().map(|d| d.arg).collect();
            if kind == Kind::Login {
                allowed.push("uris");
            }
            return Err(CoreError::service(if allowed.is_empty() {
                format!("A {} has no `fields`; its text goes in `notes`.", kind.label().to_lowercase())
            } else {
                format!("Unknown field `{arg}` for a {}. Allowed: {}.", kind.label().to_lowercase(), allowed.join(", "))
            }));
        };
        let text = match v {
            Value::String(s) => s.clone(),
            Value::Number(n) if matches!(d.arg, "exp_month" | "exp_year") => n.to_string(),
            Value::Null => String::new(),
            _ => return Err(CoreError::service(format!("`fields.{arg}` must be text."))),
        };
        clean(&format!("`fields.{arg}`"), &text, MAX_VALUE)?;
        parsed.values.push((d, text));
    }
    Ok(parsed)
}

/// Checks that a value is what its field needs (without ever quoting a secret in the message).
fn check_value(kind: Kind, d: &Def, value: &str) -> Result<(), CoreError> {
    if value.is_empty() {
        return Ok(());
    }
    let bad = |why: &str| Err(CoreError::service(format!("`fields.{}` {why}.", d.arg)));
    match (kind, d.arg) {
        (Kind::Login, "totp") if !totp_storable(value) => bad("must be a base32 secret or an otpauth:// address"),
        (Kind::Card, "exp_month") if !value.parse::<u8>().is_ok_and(|m| (1..=12).contains(&m)) => {
            bad("must be a month from 1 to 12")
        }
        (Kind::Card, "exp_year") if !(value.len() == 4 && value.bytes().all(|b| b.is_ascii_digit())) => {
            bad("must be a four-digit year")
        }
        _ => Ok(()),
    }
}

/// One custom field asked for.
struct CustomIn {
    name: String,
    value: String,
    kind: i64,
}

fn parse_custom(call: &ConnectorCall) -> Result<Vec<CustomIn>, CoreError> {
    let Some(value) = json_arg(call, "custom_fields")? else {
        return Ok(Vec::new());
    };
    let list = value.as_array().ok_or_else(|| CoreError::service("`custom_fields` must be a list."))?;
    if list.len() > MAX_CUSTOM {
        return Err(CoreError::service(format!("`custom_fields` may have at most {MAX_CUSTOM} entries.")));
    }
    let mut out: Vec<CustomIn> = Vec::new();
    for entry in list {
        let map = entry
            .as_object()
            .ok_or_else(|| CoreError::service("Each custom field must be an object {name, value, type}."))?;
        if let Some(extra) = map.keys().find(|k| !matches!(k.as_str(), "name" | "value" | "type")) {
            return Err(CoreError::service(format!("A custom field has no `{extra}`; use name, value and type.")));
        }
        let name = map.get("name").and_then(Value::as_str).map(str::trim).filter(|n| !n.is_empty());
        let name = name.ok_or_else(|| CoreError::service("Each custom field needs a `name`."))?;
        clean("A custom field name", name, 100)?;
        let kind = match map.get("type").and_then(Value::as_str).unwrap_or("text") {
            "text" => 0,
            "hidden" => 1,
            "boolean" => 2,
            _ => return Err(CoreError::service("A custom field's type must be text, hidden or boolean.")),
        };
        let value = match map.get("value") {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Bool(b)) => b.to_string(),
            Some(Value::Number(n)) => n.to_string(),
            None | Some(Value::Null) => String::new(),
            Some(_) => return Err(CoreError::service("A custom field's value must be text.")),
        };
        clean("A custom field value", &value, MAX_VALUE)?;
        if kind == 2 && !matches!(value.as_str(), "true" | "false") {
            return Err(CoreError::service("A boolean custom field's value must be true or false."));
        }
        if out.iter().any(|c| c.name == name) {
            return Err(CoreError::service(format!("The custom field \"{name}\" is given twice.")));
        }
        out.push(CustomIn {
            name: name.to_owned(),
            value,
            kind,
        });
    }
    Ok(out)
}

/// `SHA256:...` of an OpenSSH public key, as `ssh-keygen -l` prints it.
pub(super) fn ssh_fingerprint(public_key: &str) -> Option<String> {
    let blob = public_key.split_whitespace().nth(1)?;
    let raw = BASE64.decode(blob.as_bytes()).ok()?;
    Some(format!("SHA256:{}", BASE64_NOPAD.encode(digest::digest(&digest::SHA256, &raw).as_ref())))
}

fn seal(key: &VaultKey, value: &str) -> Result<Value, CoreError> {
    Ok(json!(key.encrypt(value.as_bytes())?))
}

/// Encrypted, or null for nothing (the apps write null, not an empty string).
fn seal_opt(key: &VaultKey, value: &str) -> Result<Value, CoreError> {
    if value.is_empty() {
        Ok(Value::Null)
    } else {
        seal(key, value)
    }
}

/// A value for the preview: one line, cut.
fn shown(value: &str) -> String {
    let line = text::one_line(value);
    if line.chars().count() > 300 {
        format!("{}…", text::truncate_chars(&line, 300))
    } else {
        line
    }
}

fn or_none(value: &str) -> String {
    if value.is_empty() {
        "(none)".to_owned()
    } else {
        shown(value)
    }
}

fn secret_line(label: &str, value: &str) -> String {
    format!("{label}: set ({} characters)", value.chars().count())
}

fn now() -> String {
    text::iso_utc(i64::try_from(unix_seconds()).unwrap_or(0))
}

/// What a plan does when approved.
enum Act {
    Call {
        method: Method,
        path: String,
        body: Option<Value>,
    },
    /// Attachments v2: announce the file, then upload it.
    Attach {
        cipher_id: String,
        key: VaultKey,
        file_name: String,
        data: FileData,
    },
}

/// The file of an attachment: given in the call, or uploaded through the server (read when the change is made).
enum FileData {
    Bytes(Vec<u8>),
    Blob(String),
}

/// A change, ready to be previewed and performed.
pub(super) struct Plan {
    pub preview: Preview,
    act: Act,
    /// The answer for the AI.
    done: Map<String, Value>,
    /// The new item's id is in the vault's answer.
    new_id: bool,
}

impl Plan {
    fn call(preview: Preview, method: Method, path: String, body: Option<Value>) -> Self {
        Self {
            preview,
            act: Act::Call {
                method,
                path,
                body,
            },
            done: Map::new(),
            new_id: false,
        }
    }

    fn done(mut self, key: &str, value: Value) -> Self {
        self.done.insert(key.to_owned(), value);
        self
    }
}

fn item_preview(snap: &Snapshot, entry: &Entry, lines: Vec<String>) -> Preview {
    Preview {
        resource: entry.resource(),
        resource_label: entry.name(),
        lines,
        parents: snap.parents(entry.folder(), entry.kind),
        once_only: false,
        ..Preview::default()
    }
}

fn describe(entry: &Entry) -> String {
    format!("{} \"{}\"", entry.kind.label().to_lowercase(), shown(&entry.name()))
}

fn cipher_path(entry: &Entry, tail: &str) -> String {
    format!("/api/ciphers/{}{tail}", segment(&entry.id))
}

/// The item in the call, which must not be in the trash unless `allow_trash`.
fn target<'a>(snap: &'a Snapshot, call: &ConnectorCall) -> Result<&'a Entry, CoreError> {
    snap.find(call.str_arg("item").unwrap_or_default())
}

fn not_in_trash(entry: &Entry) -> Result<(), CoreError> {
    if entry.state() == State::Trash {
        Err(CoreError::service("That item is in the trash. Restore it first (vault_item_restore)."))
    } else {
        Ok(())
    }
}

fn folder_id(folder: Option<&Folder>) -> Value {
    folder.map_or(Value::Null, |f| json!(f.id))
}

fn folder_label(snap: &Snapshot, folder: Option<&Folder>) -> String {
    snap.folder_name(folder.map_or("none", |f| f.id.as_str()))
}

// ---- create ----------------------------------------------------------------------------------------------------

/// The content object of a new item, and the lines describing it.
fn new_object(kind: Kind, key: &VaultKey, parsed: &Parsed, lines: &mut Vec<String>) -> Result<Value, CoreError> {
    let mut object = Map::new();
    let mut values: Vec<(&Def, String)> = parsed.values.iter().map(|(d, v)| (*d, v.clone())).collect();
    if kind == Kind::SshKey {
        let has = |arg: &str| values.iter().any(|(d, v)| d.arg == arg && !v.is_empty());
        if !has("private_key") || !has("public_key") {
            return Err(CoreError::service("An SSH key needs `fields.private_key` and `fields.public_key`."));
        }
        if !has("fingerprint") {
            let public = values.iter().find(|(d, _)| d.arg == "public_key").map(|(_, v)| v.clone()).unwrap_or_default();
            let fingerprint = ssh_fingerprint(&public).ok_or_else(|| {
                CoreError::service("`fields.public_key` is not an OpenSSH public key (ssh-ed25519 AAAA...).")
            })?;
            values.retain(|(d, _)| d.arg != "fingerprint");
            values.push((&SSH[2], fingerprint));
        }
    }
    for d in defs(kind) {
        let value = values.iter().find(|(x, _)| x.arg == d.arg).map_or("", |(_, v)| v.as_str());
        check_value(kind, d, value)?;
        object.insert(d.key.to_owned(), seal_opt(key, value)?);
        if !value.is_empty() {
            lines.push(if d.secret {
                secret_line(d.label, value)
            } else {
                format!("{}: {}", d.label, shown(value))
            });
        }
    }
    match kind {
        Kind::Login => {
            let uris = parsed.uris.clone().unwrap_or_default();
            let mut list = Vec::new();
            for uri in &uris {
                lines.push(format!("Website: {}", shown(uri)));
                list.push(json!({"uri": seal(key, uri)?, "match": null}));
            }
            object.insert("uris".to_owned(), Value::Array(list));
            object.insert("passwordRevisionDate".to_owned(), Value::Null);
        }
        Kind::Note => {
            object.insert("type".to_owned(), json!(0));
        }
        _ => {}
    }
    Ok(Value::Object(object))
}

fn custom_json(key: &VaultKey, c: &CustomIn) -> Result<Value, CoreError> {
    Ok(json!({"name": seal(key, &c.name)?, "value": seal(key, &c.value)?, "type": c.kind, "linkedId": null}))
}

fn custom_line(c: &CustomIn) -> String {
    let what = match c.kind {
        1 => secret_line("", &c.value).trim_start_matches(':').trim().to_owned(),
        _ => or_none(&c.value),
    };
    format!(
        "Custom field \"{}\" ({}): {what}",
        shown(&c.name),
        ["text", "hidden", "boolean"][usize::try_from(c.kind).unwrap_or(1).min(2)]
    )
}

fn plan_create(snap: &Snapshot, call: &ConnectorCall) -> Result<Plan, CoreError> {
    let kind = Kind::from_slug(call.str_arg("type").unwrap_or_default())
        .ok_or_else(|| CoreError::service("Unknown item type."))?;
    let name = call.str_arg("name").unwrap_or_default();
    if name.is_empty() {
        return Err(CoreError::service("The item needs a `name`."));
    }
    clean("`name`", name, 300)?;
    let folder = call.str_arg("folder").map(|f| snap.folder_ref(f)).transpose()?.flatten();
    let folder_key = folder.map_or("none", |f| f.id.as_str());
    let parsed = parse_fields(kind, call)?;
    let custom = parse_custom(call)?;
    let notes = call.str_arg("notes").unwrap_or_default();
    clean("`notes`", notes, 10_000)?;
    let favorite = call.bool_arg("favorite").unwrap_or(false);
    let key = &snap.user;
    let mut lines = vec![
        format!("Create {} \"{}\" in {}", kind.label().to_lowercase(), shown(name), folder_label(snap, folder)),
        format!("Name: {}", shown(name)),
    ];
    let object = new_object(kind, key, &parsed, &mut lines)?;
    if !notes.is_empty() {
        lines.push(secret_line("Notes", notes));
    }
    if favorite {
        lines.push("Favorite: yes".to_owned());
    }
    let reprompt = call.bool_arg("reprompt").unwrap_or(false);
    if reprompt {
        lines.push("Asks for the master password before showing it in a Bitwarden app".to_owned());
    }
    let mut fields = Vec::new();
    for c in &custom {
        lines.push(custom_line(c));
        fields.push(custom_json(key, c)?);
    }
    let mut body = json!({
        "type": kind.code(),
        "folderId": folder_id(folder),
        "organizationId": null,
        "key": null,
        "name": seal(key, name)?,
        "notes": seal_opt(key, notes)?,
        "favorite": favorite,
        "reprompt": i32::from(reprompt),
        "fields": fields,
        "passwordHistory": null,
        "archivedDate": null,
    });
    body[kind.object()] = object;
    let preview = Preview {
        resource: format!("{folder_key}/{}", kind.slug()),
        resource_label: format!("{} in {}", kind.plural(), snap.folder_name(folder_key)),
        lines,
        parents: vec![(folder_key.to_owned(), snap.folder_name(folder_key))],
        once_only: false,
        ..Preview::default()
    };
    let mut plan = Plan::call(preview, Method::POST, "/api/ciphers".to_owned(), Some(body))
        .done("created", json!(true))
        .done("type", json!(kind.slug()))
        .done("name", json!(shown(name)))
        .done("folder", json!(folder_key));
    plan.new_id = true;
    Ok(plan)
}

// ---- update ----------------------------------------------------------------------------------------------------

fn plan_update(snap: &Snapshot, call: &ConnectorCall) -> Result<Plan, CoreError> {
    let entry = target(snap, call)?;
    not_in_trash(entry)?;
    let kind = entry.kind;
    let key = &entry.key;
    let given = ["name", "notes", "favorite", "folder", "fields", "custom_fields", "remove_custom_fields", "reprompt"];
    if !given.iter().any(|g| call.args.contains_key(*g)) {
        return Err(CoreError::service(
            "Nothing to change: give at least one of name, notes, favorite, folder, fields, custom_fields, \
             remove_custom_fields or reprompt.",
        ));
    }
    let parsed = parse_fields(kind, call)?;
    let custom = parse_custom(call)?;
    let mut lines = vec![format!("Edit {}", describe(entry))];
    let mut body = json!({
        "id": entry.id,
        "type": kind.code(),
        "folderId": entry.raw["folderId"].clone(),
        "organizationId": null,
        "key": entry.raw["key"].clone(),
        "name": entry.raw["name"].clone(),
        "notes": entry.raw["notes"].clone(),
        "favorite": entry.favorite(),
        "reprompt": entry.raw["reprompt"].as_i64().unwrap_or(0),
        "fields": entry.raw["fields"].clone(),
        "passwordHistory": entry.raw["passwordHistory"].clone(),
        "archivedDate": entry.raw["archivedDate"].clone(),
        "lastKnownRevisionDate": entry.raw["revisionDate"].clone(),
    });
    if body["fields"].is_null() {
        body["fields"] = json!([]);
    }
    if body["passwordHistory"].is_null() {
        body["passwordHistory"] = json!([]);
    }
    let mut object = entry.raw[kind.object()].clone();
    if !object.is_object() {
        object = json!({});
    }
    if let Some(map) = object.as_object_mut() {
        // The server works the first address out again.
        map.remove("uri");
    }

    if let Some(name) = call.str_arg("name") {
        if name.is_empty() {
            return Err(CoreError::service("The name cannot be empty."));
        }
        clean("`name`", name, 300)?;
        lines.push(format!("Name: {} → {}", or_none(&entry.name()), shown(name)));
        body["name"] = seal(key, name)?;
    }
    if let Some(notes) = call.str_arg("notes") {
        clean("`notes`", notes, 10_000)?;
        lines.push(if notes.is_empty() {
            "Notes: cleared".to_owned()
        } else {
            format!("Notes: change ({} characters)", notes.chars().count())
        });
        body["notes"] = seal_opt(key, notes)?;
    }
    if let Some(favorite) = call.bool_arg("favorite") {
        lines.push(format!("Favorite: {} → {}", yes_no(entry.favorite()), yes_no(favorite)));
        body["favorite"] = json!(favorite);
    }
    if let Some(given) = call.str_arg("folder") {
        let target = snap.folder_ref(given)?;
        lines.push(format!("Folder: {} → {}", snap.folder_name(entry.folder()), folder_label(snap, target)));
        body["folderId"] = folder_id(target);
    }
    if let Some(reprompt) = call.bool_arg("reprompt") {
        lines.push(format!("Asks for the master password: {}", yes_no(reprompt)));
        body["reprompt"] = json!(i32::from(reprompt));
    }
    for (d, value) in &parsed.values {
        check_value(kind, d, value)?;
        let pointer = format!("/{}/{}", kind.object(), d.key);
        let old = entry.dec(&pointer).map(|v| v.to_string()).unwrap_or_default();
        if kind == Kind::SshKey && d.arg == "private_key" && value.is_empty() {
            return Err(CoreError::service("An SSH key cannot lose its private key."));
        }
        if *value == old {
            continue;
        }
        if d.secret {
            lines.push(if value.is_empty() {
                format!("{}: removed", d.label)
            } else {
                format!("{}: changes ({} characters)", d.label, value.chars().count())
            });
        } else {
            lines.push(format!("{}: {} → {}", d.label, or_none(&entry.line(&pointer)), or_none(value)));
        }
        object[d.key] = seal_opt(key, value)?;
        if kind == Kind::Login && d.arg == "password" {
            if !old.is_empty() {
                // The old password stays in the history, as the apps do (it is already encrypted with this key).
                let mut history = body["passwordHistory"].as_array().cloned().unwrap_or_default();
                history.insert(0, json!({"password": entry.raw["login"]["password"].clone(), "lastUsedDate": now()}));
                history.truncate(MAX_HISTORY);
                body["passwordHistory"] = Value::Array(history);
                lines.push("The old password is kept in the item's password history".to_owned());
            }
            object["passwordRevisionDate"] = json!(now());
        }
    }
    if kind == Kind::SshKey
        && parsed.values.iter().any(|(d, v)| d.arg == "public_key" && !v.is_empty())
        && !parsed.values.iter().any(|(d, _)| d.arg == "fingerprint")
    {
        let public = parsed.values.iter().find(|(d, _)| d.arg == "public_key").map_or("", |(_, v)| v.as_str());
        let fingerprint = ssh_fingerprint(public).ok_or_else(|| {
            CoreError::service("`fields.public_key` is not an OpenSSH public key (ssh-ed25519 AAAA...).")
        })?;
        lines.push(format!("Fingerprint: {} → {fingerprint}", or_none(&entry.line("/sshKey/keyFingerprint"))));
        object["keyFingerprint"] = seal(key, &fingerprint)?;
    }
    if let Some(uris) = &parsed.uris {
        let old = entry.uris();
        lines.push(format!("Websites: {} → {}", or_none(&old.join(", ")), or_none(&uris.join(", "))));
        let previous = entry.raw.pointer("/login/uris").and_then(Value::as_array).cloned().unwrap_or_default();
        let mut list = Vec::new();
        for uri in uris {
            // An address that stays keeps its matching rule.
            let kept = previous
                .iter()
                .find(|p| p["uri"].as_str().and_then(|u| key.decrypt_text(u).ok()).is_some_and(|u| *u == *uri))
                .map(|p| p["match"].clone());
            list.push(json!({"uri": seal(key, uri)?, "match": kept.unwrap_or(Value::Null)}));
        }
        object["uris"] = Value::Array(list);
    }
    body[kind.object()] = object;

    // Custom fields: change or add by name, remove by name.
    let existing = entry.custom_slots();
    let mut fields = body["fields"].as_array().cloned().unwrap_or_default();
    for c in &custom {
        let json = custom_json(key, c)?;
        if let Some(i) = position_of(&existing, &c.name) {
            lines.push(match existing[i].as_ref() {
                Some(old) if c.kind != 1 && old.kind != 1 => {
                    format!("Custom field \"{}\": {} → {}", shown(&c.name), or_none(&old.value), or_none(&c.value))
                }
                _ => format!("Custom field \"{}\": changes", shown(&c.name)),
            });
            fields[i] = json;
        } else {
            lines.push(format!("Add {}", custom_line(c)));
            fields.push(json);
        }
    }
    let mut removed = Vec::new();
    for name in call.list_arg("remove_custom_fields") {
        let i = position_of(&existing, name)
            .ok_or_else(|| CoreError::service(format!("The item has no custom field \"{}\".", shown(name))))?;
        removed.push(i);
        lines.push(format!("Remove custom field \"{}\"", shown(name)));
    }
    if !removed.is_empty() {
        // Indexes follow the existing order, which `fields` keeps.
        let keep: Vec<Value> =
            fields.iter().enumerate().filter(|(i, _)| !removed.contains(i)).map(|(_, f)| f.clone()).collect();
        fields = keep;
    }
    body["fields"] = Value::Array(fields);
    let preview = item_preview(snap, entry, lines);
    Ok(Plan::call(preview, Method::PUT, cipher_path(entry, ""), Some(body))
        .done("updated", json!(true))
        .done("id", json!(entry.id)))
}

/// Where in the item's custom fields the one with this name is.
fn position_of(slots: &[Option<Custom>], name: &str) -> Option<usize> {
    slots.iter().position(|c| c.as_ref().is_some_and(|c| c.name == name))
}

fn yes_no(b: bool) -> &'static str {
    if b {
        "yes"
    } else {
        "no"
    }
}

// ---- clone -----------------------------------------------------------------------------------------------------

fn plan_clone(snap: &Snapshot, call: &ConnectorCall) -> Result<Plan, CoreError> {
    let entry = target(snap, call)?;
    not_in_trash(entry)?;
    let kind = entry.kind;
    let name = match call.str_arg("name") {
        Some(n) => n.to_owned(),
        None => format!("{} - Clone", entry.name()),
    };
    clean("`name`", &name, 300)?;
    let folder = match call.str_arg("folder") {
        Some(f) => snap.folder_ref(f)?,
        None => snap.folders.iter().find(|f| f.id == entry.folder()),
    };
    let folder_key = folder.map_or("none", |f| f.id.as_str());
    let mut object = entry.raw[kind.object()].clone();
    if let Some(map) = object.as_object_mut() {
        map.remove("uri");
        // A passkey cannot be duplicated.
        map.remove("fido2Credentials");
    }
    let mut body = json!({
        "type": kind.code(),
        "folderId": folder_id(folder),
        "organizationId": null,
        "key": entry.raw["key"].clone(),
        // With its own key an item's copy shares it, so the copied strings stay valid.
        "name": seal(&entry.key, &name)?,
        "notes": entry.raw["notes"].clone(),
        "favorite": false,
        "reprompt": entry.raw["reprompt"].as_i64().unwrap_or(0),
        "fields": if entry.raw["fields"].is_null() { json!([]) } else { entry.raw["fields"].clone() },
        "passwordHistory": null,
        "archivedDate": null,
    });
    body[kind.object()] = object;
    let mut lines = vec![
        format!("Copy {} as \"{}\" in {}", describe(entry), shown(&name), folder_label(snap, folder)),
        "Passwords, codes, notes and custom fields are copied on the phone without being shown.".to_owned(),
        "Attachments and old passwords are not copied.".to_owned(),
    ];
    if kind == Kind::Login {
        lines.push(format!("Username: {}", or_none(&entry.line("/login/username"))));
        for uri in entry.uris() {
            lines.push(format!("Website: {}", shown(&host_of(&uri))));
        }
    }
    let preview = Preview {
        resource: format!("{folder_key}/{}", kind.slug()),
        resource_label: format!("{} in {}", kind.plural(), snap.folder_name(folder_key)),
        lines,
        parents: vec![(folder_key.to_owned(), snap.folder_name(folder_key))],
        once_only: false,
        ..Preview::default()
    };
    let mut plan = Plan::call(preview, Method::POST, "/api/ciphers".to_owned(), Some(body))
        .done("created", json!(true))
        .done("copied_from", json!(entry.id))
        .done("type", json!(kind.slug()))
        .done("name", json!(shown(&name)))
        .done("folder", json!(folder_key));
    plan.new_id = true;
    Ok(plan)
}

// ---- states, favorite, move, delete ----------------------------------------------------------------------------

fn plan_state(snap: &Snapshot, call: &ConnectorCall) -> Result<Plan, CoreError> {
    let entry = target(snap, call)?;
    let state = entry.state();
    let (verb, path, done, ok) = match call.op.as_str() {
        "item_trash" => ("Move to the trash", "/delete", "trashed", state != State::Trash),
        "item_restore" => ("Restore from the trash", "/restore", "restored", state == State::Trash),
        "item_archive" => ("Archive", "/archive", "archived", state == State::Active),
        _ => ("Take out of the archive", "/unarchive", "unarchived", state == State::Archived),
    };
    if !ok {
        return Err(CoreError::service(format!(
            "That item is {} already or not in a state for this.",
            match state {
                State::Trash => "in the trash",
                State::Archived => "archived",
                State::Active => "active",
            }
        )));
    }
    let preview = item_preview(snap, entry, vec![format!("{verb}: {}", describe(entry))]);
    Ok(Plan::call(preview, Method::PUT, cipher_path(entry, path), None)
        .done(done, json!(true))
        .done("id", json!(entry.id)))
}

fn plan_partial(snap: &Snapshot, call: &ConnectorCall) -> Result<Plan, CoreError> {
    let entry = target(snap, call)?;
    let (mut folder, mut favorite) = (entry.raw["folderId"].clone(), entry.favorite());
    let (line, done) = if call.op == "item_favorite" {
        favorite = call.bool_arg("favorite").unwrap_or(true);
        (
            if favorite {
                format!("Mark {} as a favorite", describe(entry))
            } else {
                format!("Remove the favorite mark from {}", describe(entry))
            },
            "favorite",
        )
    } else {
        let target = snap.folder_ref(call.str_arg("folder").unwrap_or_default())?;
        folder = folder_id(target);
        (
            format!(
                "Move {} from {} to {}",
                describe(entry),
                snap.folder_name(entry.folder()),
                folder_label(snap, target)
            ),
            "moved",
        )
    };
    let preview = item_preview(snap, entry, vec![line]);
    // The server sets folder and favorite together, so both are sent.
    let body = json!({"folderId": folder, "favorite": favorite});
    let mut plan =
        Plan::call(preview, Method::PUT, cipher_path(entry, "/partial"), Some(body)).done("id", json!(entry.id));
    plan = if done == "favorite" {
        plan.done("favorite", json!(favorite))
    } else {
        plan.done("moved", json!(true))
    };
    Ok(plan)
}

fn plan_delete(snap: &Snapshot, call: &ConnectorCall) -> Result<Plan, CoreError> {
    let entry = target(snap, call)?;
    let mut lines = vec![format!("Delete {} for good", describe(entry)), "It cannot be brought back.".to_owned()];
    let files = entry.attachments().len();
    if files > 0 {
        lines.push(format!("Its {files} attached file(s) are deleted too."));
    }
    let mut preview = item_preview(snap, entry, lines);
    preview.once_only = true;
    Ok(Plan::call(preview, Method::DELETE, cipher_path(entry, ""), None)
        .done("deleted", json!(true))
        .done("id", json!(entry.id)))
}

fn plan_trash_empty(snap: &Snapshot) -> Result<Plan, CoreError> {
    let trashed: Vec<&Entry> = snap.entries.iter().filter(|e| e.state() == State::Trash).collect();
    if trashed.is_empty() {
        return Err(CoreError::service("The trash is empty."));
    }
    let mut lines = vec![
        format!("Delete {} item(s) in the trash for good", trashed.len()),
        "They cannot be brought back.".to_owned(),
    ];
    for e in trashed.iter().take(20) {
        lines.push(format!("- {}", describe(e)));
    }
    if trashed.len() > 20 {
        lines.push(format!("… and {} more", trashed.len() - 20));
    }
    let ids: Vec<&str> = trashed.iter().map(|e| e.id.as_str()).collect();
    let preview = Preview {
        resource: "trash".to_owned(),
        resource_label: "Vault trash".to_owned(),
        lines,
        parents: Vec::new(),
        once_only: true,
        ..Preview::default()
    };
    Ok(Plan::call(preview, Method::DELETE, "/api/ciphers".to_owned(), Some(json!({"ids": ids})))
        .done("emptied", json!(true))
        .done("deleted_items", json!(trashed.len())))
}

// ---- attachments -----------------------------------------------------------------------------------------------

/// The bytes of a base64 argument, with or without padding and line breaks.
fn decode_file(call: &ConnectorCall) -> Result<Vec<u8>, CoreError> {
    let raw: String = call.str_arg("content_base64").unwrap_or_default().split_whitespace().collect();
    let bytes = BASE64
        .decode(raw.as_bytes())
        .or_else(|_| BASE64_NOPAD.decode(raw.trim_end_matches('=').as_bytes()))
        .map_err(|_| CoreError::service("`content_base64` is not valid base64."))?;
    if bytes.is_empty() {
        return Err(CoreError::service("The file is empty."));
    }
    if bytes.len() > MAX_FILE_BYTES {
        return Err(CoreError::service("The file is larger than 2 MB."));
    }
    Ok(bytes)
}

fn plan_attachment_add(snap: &Snapshot, call: &ConnectorCall) -> Result<Plan, CoreError> {
    let entry = target(snap, call)?;
    not_in_trash(entry)?;
    let file_name = call.str_arg("file_name").unwrap_or_default();
    if file_name.is_empty() || file_name.contains(['/', '\\']) {
        return Err(CoreError::service("`file_name` must be a file name without a path."));
    }
    clean("`file_name`", file_name, 200)?;
    let (data, size) = match call.str_arg("blob").filter(|b| !b.is_empty()) {
        Some(_) if call.str_arg("content_base64").is_some() => {
            return Err(CoreError::service("Give either `content_base64` or `blob`, not both."));
        }
        Some(id) => (FileData::Blob(id.to_owned()), "the uploaded file, below".to_owned()),
        None => {
            let bytes = decode_file(call)?;
            let size = format!("{} bytes", bytes.len());
            (FileData::Bytes(bytes), size)
        }
    };
    let preview =
        item_preview(snap, entry, vec![format!("Attach \"{}\" ({size}) to {}", shown(file_name), describe(entry))]);
    Ok(Plan {
        preview,
        act: Act::Attach {
            cipher_id: entry.id.clone(),
            key: VaultKey::from_bytes(&entry.key.to_bytes())?,
            file_name: file_name.to_owned(),
            data,
        },
        done: Map::from_iter([
            ("attached".to_owned(), json!(true)),
            ("id".to_owned(), json!(entry.id)),
            ("file_name".to_owned(), json!(shown(file_name))),
        ]),
        new_id: false,
    })
}

fn plan_attachment_delete(snap: &Snapshot, call: &ConnectorCall) -> Result<Plan, CoreError> {
    let entry = target(snap, call)?;
    let attachments = entry.attachments();
    let a = attachment_of(&attachments, call.str_arg("attachment").unwrap_or_default())?;
    let preview = item_preview(
        snap,
        entry,
        vec![format!(
            "Delete the attachment \"{}\" ({} bytes) of {} for good",
            shown(&a.name),
            a.size,
            describe(entry)
        )],
    );
    Ok(Plan::call(preview, Method::DELETE, cipher_path(entry, &format!("/attachment/{}", segment(&a.id))), None)
        .done("deleted", json!(true))
        .done("id", json!(entry.id))
        .done("attachment", json!(a.id)))
}

// ---- folders ---------------------------------------------------------------------------------------------------

fn folder_name_arg(snap: &Snapshot, call: &ConnectorCall, except: Option<&str>) -> Result<String, CoreError> {
    let name = call.str_arg("name").unwrap_or_default();
    if name.is_empty() {
        return Err(CoreError::service("The folder needs a `name`."));
    }
    clean("`name`", name, 100)?;
    if name.eq_ignore_ascii_case("none") {
        return Err(CoreError::service("`none` is reserved for items without a folder; pick another name."));
    }
    if snap.folders.iter().any(|f| Some(f.id.as_str()) != except && f.name.eq_ignore_ascii_case(name)) {
        return Err(CoreError::service("A folder with that name exists already."));
    }
    Ok(name.to_owned())
}

fn existing_folder<'a>(snap: &'a Snapshot, call: &ConnectorCall) -> Result<&'a Folder, CoreError> {
    snap.folder_ref(call.str_arg("folder").unwrap_or_default())?
        .ok_or_else(|| CoreError::service("`none` is not a folder. Use vault_folders_list to see the folders."))
}

fn folder_preview(folder: &Folder, lines: Vec<String>) -> Preview {
    Preview {
        resource: folder.id.clone(),
        resource_label: folder.name.clone(),
        lines,
        parents: Vec::new(),
        once_only: false,
        ..Preview::default()
    }
}

fn plan_folder(snap: &Snapshot, call: &ConnectorCall) -> Result<Plan, CoreError> {
    match call.op.as_str() {
        "folder_create" => {
            let name = folder_name_arg(snap, call, None)?;
            let preview = Preview {
                resource: "folders".to_owned(),
                resource_label: "Vault folders".to_owned(),
                lines: vec![format!("Create the folder \"{}\"", shown(&name))],
                parents: Vec::new(),
                once_only: false,
                ..Preview::default()
            };
            let mut plan = Plan::call(
                preview,
                Method::POST,
                "/api/folders".to_owned(),
                Some(json!({"name": seal(&snap.user, &name)?})),
            )
            .done("created", json!(true))
            .done("name", json!(shown(&name)));
            plan.new_id = true;
            Ok(plan)
        }
        "folder_rename" => {
            let folder = existing_folder(snap, call)?;
            let name = folder_name_arg(snap, call, Some(&folder.id))?;
            let preview = folder_preview(
                folder,
                vec![format!("Rename the folder \"{}\" to \"{}\"", shown(&folder.name), shown(&name))],
            );
            Ok(Plan::call(
                preview,
                Method::PUT,
                format!("/api/folders/{}", segment(&folder.id)),
                Some(json!({"name": seal(&snap.user, &name)?})),
            )
            .done("renamed", json!(true))
            .done("id", json!(folder.id)))
        }
        _ => {
            let folder = existing_folder(snap, call)?;
            let count = snap.entries.iter().filter(|e| e.folder() == folder.id).count();
            let preview = folder_preview(
                folder,
                vec![
                    format!("Delete the folder \"{}\"", shown(&folder.name)),
                    format!("Its {count} item(s) are kept and end up without a folder."),
                ],
            );
            Ok(Plan::call(preview, Method::DELETE, format!("/api/folders/{}", segment(&folder.id)), None)
                .done("deleted", json!(true))
                .done("id", json!(folder.id))
                .done("items_unfiled", json!(count)))
        }
    }
}

// ---- entry points ----------------------------------------------------------------------------------------------

/// The plan of a write of this area, with the session to carry it out; `None` when the operation is not one of them.
pub(super) async fn plan(
    vault: &Vault,
    account: &str,
    call: &ConnectorCall,
) -> Option<Result<(Arc<Session>, Plan), CoreError>> {
    if !matches!(
        call.op.as_str(),
        "item_create"
            | "item_update"
            | "item_clone"
            | "item_trash"
            | "item_restore"
            | "item_archive"
            | "item_unarchive"
            | "item_favorite"
            | "item_move"
            | "item_delete"
            | "trash_empty"
            | "attachment_add"
            | "attachment_delete"
            | "folder_create"
            | "folder_rename"
            | "folder_delete"
    ) {
        return None;
    }
    Some(plan_of(vault, account, call).await)
}

async fn plan_of(vault: &Vault, account: &str, call: &ConnectorCall) -> Result<(Arc<Session>, Plan), CoreError> {
    let snap = Snapshot::load(vault, account).await?;
    let plan = plan_with(&snap, call)?;
    Ok((snap.session, plan))
}

/// The plan of a write of this area on a vault already read.
pub(super) fn plan_with(snap: &Snapshot, call: &ConnectorCall) -> Result<Plan, CoreError> {
    match call.op.as_str() {
        "item_create" => plan_create(snap, call),
        "item_update" => plan_update(snap, call),
        "item_clone" => plan_clone(snap, call),
        "item_trash" | "item_restore" | "item_archive" | "item_unarchive" => plan_state(snap, call),
        "item_favorite" | "item_move" => plan_partial(snap, call),
        "item_delete" => plan_delete(snap, call),
        "trash_empty" => plan_trash_empty(snap),
        "attachment_add" => plan_attachment_add(snap, call),
        "attachment_delete" => plan_attachment_delete(snap, call),
        _ => plan_folder(snap, call),
    }
}

/// Does what the plan says and answers the AI with what happened.
pub(super) async fn carry_out(vault: &Vault, session: &Session, plan: Plan) -> Result<Value, CoreError> {
    let mut done = plan.done;
    match plan.act {
        Act::Call {
            method,
            path,
            body,
        } => {
            let answer = vault.api(session, method, &path, body.as_ref()).await?;
            if plan.new_id {
                let id = answer["id"]
                    .as_str()
                    .ok_or_else(|| CoreError::service("The vault did not say what it created."))?;
                done.insert("id".to_owned(), json!(id));
            }
        }
        Act::Attach {
            cipher_id,
            key,
            file_name,
            data,
        } => {
            let (data, upload) = match data {
                FileData::Bytes(bytes) => (zeroize::Zeroizing::new(bytes), None),
                FileData::Blob(id) => (crate::blob::read(&id, crate::blob::MAX_READ_BYTES).await?, Some(id)),
            };
            // A fresh key for the file, kept with the item encrypted by the item's key.
            let mut file_key = crypto::random_bytes::<32>()?.to_vec();
            file_key.extend_from_slice(&crypto::random_bytes::<32>()?);
            let file_key = zeroize::Zeroizing::new(file_key);
            let encrypted = encrypt_buffer(&VaultKey::from_bytes(&file_key)?, &data)?;
            let enc_name = key.encrypt(file_name.as_bytes())?;
            let announce = json!({"key": key.encrypt(&file_key)?, "fileName": enc_name,
                "fileSize": encrypted.len(), "adminRequest": false});
            let base = format!("/api/ciphers/{}", segment(&cipher_id));
            let created = vault.api(session, Method::POST, &format!("{base}/attachment/v2"), Some(&announce)).await?;
            let attachment_id = created["attachmentId"]
                .as_str()
                .ok_or_else(|| CoreError::service("The vault did not accept the attachment."))?
                .to_owned();
            let attachment_path = format!("{base}/attachment/{}", segment(&attachment_id));
            let uploaded = vault
                .api_form(session, &attachment_path, || {
                    reqwest::multipart::Form::new()
                        .part("data", reqwest::multipart::Part::bytes(encrypted.clone()).file_name(enc_name.clone()))
                })
                .await;
            if let Err(e) = uploaded {
                // Do not leave an empty attachment record behind.
                vault.api(session, Method::DELETE, &attachment_path, None).await.ok();
                return Err(e);
            }
            if let Some(id) = upload {
                crate::blob::used(&id).await;
            }
            done.insert("attachment".to_owned(), json!(attachment_id));
        }
    }
    Ok(Value::Object(done))
}
