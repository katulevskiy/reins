//! Vault: Sends (text and files shared by link) and the password, passphrase and username generator.
//!
//! A Send is encrypted on the phone the way the Bitwarden apps do it: a random 16-byte send key is stretched with
//! HKDF-SHA256 (salt `bitwarden-send`, info `send`) into an AES/HMAC key that protects the name, notes, text and file;
//! the send key itself is stored encrypted with the user key, and is the secret in the link. A password is sent as
//! `base64(PBKDF2-SHA256(password, salt = send key, 100 000 rounds))`; the server hashes that again.
//!
//! Nothing secret (the text, the link, a password, a generated value) is ever written to a preview line, a title, a
//! snippet or an error: those go only into an item's `body`, which the approval never shows and the activity never keeps.

use std::num::NonZeroU32;
use std::time::{SystemTime, UNIX_EPOCH};

use data_encoding::{BASE64, BASE64_NOPAD, BASE64URL_NOPAD};
use reins_proto::connector::ConnectorCall;
use reqwest::Method;
use reqwest::multipart::{Form, Part};
use ring::rand::{SecureRandom, SystemRandom};
use ring::{hkdf, pbkdf2};
use serde_json::{Map, Value, json};
use zeroize::Zeroizing;

use super::{Loaded, Vault};
use crate::connector::calendar::limit;
use crate::connector::{Item, Preview};
use crate::crypto::{self, VaultKey};
use crate::{CoreError, text};

/// The resource of every Send; a Send is `sends/{id}`.
const SENDS: &str = "sends";
const SENDS_LABEL: &str = "All your Sends";
const GENERATOR: &str = "generator";
const GENERATOR_LABEL: &str = "Generator";
/// The most a Send's file may weigh once decoded.
const MAX_FILE: usize = 2 * 1024 * 1024;
const DEFAULT_DELETE_DAYS: i64 = 7;
/// The vault server refuses a deletion date further than this from now.
const MAX_KEEP_SECS: i64 = 31 * 86_400;
/// How much of a text or notes a preview shows.
const PREVIEW_CHARS: usize = 300;
const PASSWORD_ROUNDS: NonZeroU32 = NonZeroU32::new(100_000).unwrap();

const EFF_WORDLIST: &str = include_str!("eff_large_wordlist.txt");
const UPPER: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
const LOWER: &str = "abcdefghijklmnopqrstuvwxyz";
const DIGITS: &str = "0123456789";
const SYMBOLS: &str = "!@#$%^&*()-_=+[]{};:,.?";
const AMBIGUOUS: &str = "Il1O0";

/// Lists and reads. `None` when the operation is not one of this area's.
pub(super) async fn fetch(vault: &Vault, account: &str, call: &ConnectorCall) -> Option<Result<Vec<Item>, CoreError>> {
    Some(match call.op.as_str() {
        "send_list" => list(vault, account, call).await,
        "send_get" => get(vault, account, call).await,
        "generate_password" => generate_password(&mut Secure::new(), call).map(|v| vec![v]),
        "generate_passphrase" => generate_passphrase(&mut Secure::new(), call).map(|v| vec![v]),
        "generate_username" => generate_username(&mut Secure::new(), call).map(|v| vec![v]),
        "generate_check_password" => check_password(call).map(|v| vec![v]),
        _ => return None,
    })
}

/// What a write would do. `None` when the operation is not one of this area's.
pub(super) async fn preview(vault: &Vault, account: &str, call: &ConnectorCall) -> Option<Result<Preview, CoreError>> {
    Some(match call.op.as_str() {
        "send_create" => preview_create(call),
        "send_update" => preview_update(vault, account, call).await,
        "send_remove_password" => preview_remove_password(vault, account, call).await,
        "send_delete" => preview_delete(vault, account, call).await,
        _ => return None,
    })
}

/// Does the write. `None` when the operation is not one of this area's.
pub(super) async fn perform(vault: &Vault, account: &str, call: &ConnectorCall) -> Option<Result<Value, CoreError>> {
    Some(match call.op.as_str() {
        "send_create" => create(vault, account, call).await,
        "send_update" => update(vault, account, call).await,
        "send_remove_password" => remove_password(vault, call).await,
        "send_delete" => delete(vault, call).await,
        _ => return None,
    })
}

// ---- Send crypto ----

struct KeyLen(usize);

impl hkdf::KeyType for KeyLen {
    fn len(&self) -> usize {
        self.0
    }
}

/// The keys a Send's fields are encrypted with, from the send key.
fn derive(send_key: &[u8]) -> Result<VaultKey, CoreError> {
    let fail = || CoreError::invalid("the Send key could not be stretched");
    let prk = hkdf::Salt::new(hkdf::HKDF_SHA256, b"bitwarden-send").extract(send_key);
    let info = [b"send".as_slice()];
    let okm = prk.expand(&info, KeyLen(64)).map_err(|_| fail())?;
    let mut out = Zeroizing::new([0u8; 64]);
    okm.fill(&mut *out).map_err(|_| fail())?;
    VaultKey::from_bytes(&*out)
}

/// `base64(PBKDF2-SHA256(password, salt = send key, 100 000 rounds, 32 bytes))`, what a client sends as the password.
fn password_hash(password: &str, send_key: &[u8]) -> String {
    let mut out = [0u8; 32];
    pbkdf2::derive(pbkdf2::PBKDF2_HMAC_SHA256, PASSWORD_ROUNDS, send_key, password.as_bytes(), &mut out);
    BASE64.encode(&out)
}

/// The same off the async threads (100 000 rounds take a while on a phone).
async fn password_hash_blocking(password: &str, send_key: &[u8]) -> Result<String, CoreError> {
    let (password, key) = (Zeroizing::new(password.to_owned()), Zeroizing::new(send_key.to_vec()));
    tokio::task::spawn_blocking(move || password_hash(&password, &key))
        .await
        .map_err(|_| CoreError::service("The password could not be prepared."))
}

/// The send key and the keys derived from it.
struct SendKeys {
    raw: Zeroizing<Vec<u8>>,
    keys: VaultKey,
}

impl SendKeys {
    fn from_raw(raw: Zeroizing<Vec<u8>>) -> Result<Self, CoreError> {
        let keys = derive(&raw)?;
        Ok(Self {
            raw,
            keys,
        })
    }

    fn generate() -> Result<Self, CoreError> {
        Self::from_raw(Zeroizing::new(crypto::random_bytes::<16>()?.to_vec()))
    }

    /// The keys of an existing Send (its `key` is the send key encrypted with the user key).
    fn open(user: &VaultKey, send: &Value) -> Result<Self, CoreError> {
        let unreadable = || CoreError::service("That Send could not be decrypted.");
        let wrapped = send["key"].as_str().ok_or_else(unreadable)?;
        Self::from_raw(Zeroizing::new(user.decrypt(wrapped).map_err(|_| unreadable())?.to_vec()))
            .map_err(|_| unreadable())
    }

    fn link(&self, base: &str, access_id: &str) -> String {
        format!("{base}/#/send/{access_id}/{}", BASE64URL_NOPAD.encode(&self.raw))
    }

    fn encrypt(&self, plain: &str) -> Result<Value, CoreError> {
        self.keys.encrypt(plain.as_bytes()).map(Value::String)
    }

    fn decrypt(&self, field: &Value) -> Option<String> {
        self.keys.decrypt_text(field.as_str()?).ok().map(|t| t.to_string())
    }

    /// Encrypted data as the file upload wants it: `0x02 || iv || mac || ciphertext`.
    fn encrypt_buffer(&self, plain: &[u8]) -> Result<Vec<u8>, CoreError> {
        let broken = || CoreError::invalid("encryption produced an unexpected form");
        let sealed = self.keys.encrypt(plain)?;
        let mut parts = sealed.strip_prefix("2.").ok_or_else(broken)?.split('|');
        let (Some(iv), Some(data), Some(mac), None) = (parts.next(), parts.next(), parts.next(), parts.next()) else {
            return Err(broken());
        };
        let [iv, data, mac] = [iv, data, mac].map(|p| BASE64.decode(p.as_bytes()));
        let (iv, data, mac) = (iv.map_err(|_| broken())?, data.map_err(|_| broken())?, mac.map_err(|_| broken())?);
        let mut out = Vec::with_capacity(1 + iv.len() + mac.len() + data.len());
        out.push(2);
        out.extend_from_slice(&iv);
        out.extend_from_slice(&mac);
        out.extend_from_slice(&data);
        Ok(out)
    }
}

// ---- What the server knows about a Send ----

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

fn parents() -> Vec<(String, String)> {
    vec![(SENDS.to_owned(), SENDS_LABEL.to_owned())]
}

fn send_resource(id: &str) -> String {
    format!("{SENDS}/{id}")
}

fn id_ok(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn send_id(call: &ConnectorCall) -> Result<&str, CoreError> {
    call.str_arg("send")
        .filter(|id| id_ok(id))
        .ok_or_else(|| CoreError::service("`send` must be the id of a Send, from vault_send_list."))
}

fn sends_of(sync: &Value) -> &[Value] {
    sync["sends"].as_array().or_else(|| sync["Sends"].as_array()).map_or(&[], Vec::as_slice)
}

fn find_send<'a>(loaded: &'a Loaded, id: &str) -> Result<&'a Value, CoreError> {
    sends_of(&loaded.sync)
        .iter()
        .find(|s| s["id"].as_str() == Some(id))
        .ok_or_else(|| CoreError::service("No Send with that id. Use vault_send_list to find it."))
}

/// What a listing shows of a Send (never its content or link).
#[allow(clippy::struct_excessive_bools, reason = "a Send has this many independent switches")]
struct Meta {
    id: String,
    file: bool,
    name: String,
    access_count: i64,
    max_access_count: Option<i64>,
    expiration: Option<String>,
    deletion: String,
    disabled: bool,
    hide_email: bool,
    has_password: bool,
    revision: i64,
}

impl Meta {
    fn new(send: &Value, keys: Option<&SendKeys>) -> Self {
        let name = keys
            .and_then(|k| k.decrypt(&send["name"]))
            .map(|n| text::one_line(&n))
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| "(unreadable name)".to_owned());
        Self {
            id: send["id"].as_str().unwrap_or_default().to_owned(),
            file: send["type"].as_i64() == Some(1),
            name,
            access_count: send["accessCount"].as_i64().unwrap_or(0),
            max_access_count: send["maxAccessCount"].as_i64(),
            expiration: send["expirationDate"].as_str().map(str::to_owned),
            deletion: send["deletionDate"].as_str().unwrap_or_default().to_owned(),
            disabled: send["disabled"].as_bool().unwrap_or(false),
            hide_email: send["hideEmail"].as_bool().unwrap_or(false),
            has_password: !send["password"].is_null() || send["authType"].as_i64() == Some(1),
            revision: send["revisionDate"].as_str().and_then(text::parse_when).unwrap_or(0),
        }
    }

    fn kind(&self) -> &'static str {
        if self.file {
            "file"
        } else {
            "text"
        }
    }

    fn describe(&self) -> String {
        let mut parts = vec![format!(
            "{} Send",
            if self.file {
                "File"
            } else {
                "Text"
            }
        )];
        parts.push(match self.max_access_count {
            Some(max) => format!("opened {} of {max} times", self.access_count),
            None => format!("opened {} times", self.access_count),
        });
        if let Some(e) = &self.expiration {
            parts.push(format!("expires {}", day(e)));
        }
        parts.push(format!("deleted {}", day(&self.deletion)));
        for (on, what) in [
            (self.has_password, "password protected"),
            (self.hide_email, "hides your email"),
            (self.disabled, "disabled"),
        ] {
            if on {
                parts.push(what.to_owned());
            }
        }
        parts.join(", ")
    }

    fn extra(&self) -> Map<String, Value> {
        let mut extra = Map::new();
        extra.insert("type".to_owned(), json!(self.kind()));
        extra.insert("access_count".to_owned(), json!(self.access_count));
        extra.insert("max_access_count".to_owned(), json!(self.max_access_count));
        extra.insert("expiration_date".to_owned(), json!(self.expiration.as_deref().map(moment_text)));
        extra.insert("deletion_date".to_owned(), json!(moment_text(&self.deletion)));
        extra.insert("disabled".to_owned(), json!(self.disabled));
        extra.insert("password_protected".to_owned(), json!(self.has_password));
        extra.insert("hide_email".to_owned(), json!(self.hide_email));
        extra
    }
}

/// `2026-10-05` from a server date.
fn day(date: &str) -> &str {
    date.get(..10).unwrap_or(date)
}

/// A server date the way the AI is given dates everywhere: `2026-10-05T14:30:00Z`.
fn moment_text(date: &str) -> String {
    text::parse_when(date).map_or_else(|| date.to_owned(), text::iso_utc)
}

fn item_of(meta: &Meta, snippet: String) -> Item {
    Item {
        id: meta.id.clone(),
        resource: send_resource(&meta.id),
        resource_label: meta.name.clone(),
        title: meta.name.clone(),
        snippet,
        date: meta.revision,
        parents: parents(),
        extra: meta.extra(),
        ..Item::default()
    }
}

async fn list(vault: &Vault, account: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
    let loaded = vault.load(account).await?;
    let query = call.str_arg("query").unwrap_or_default().to_lowercase();
    let mut items: Vec<Item> = sends_of(&loaded.sync)
        .iter()
        .map(|send| {
            let keys = SendKeys::open(&loaded.key, send).ok();
            Meta::new(send, keys.as_ref())
        })
        .filter(|m| m.name.to_lowercase().contains(&query))
        .map(|m| {
            let snippet = m.describe();
            item_of(&m, snippet)
        })
        .collect();
    items.sort_by_key(|i| i.title.to_lowercase());
    items.truncate(limit(call));
    Ok(items)
}

async fn get(vault: &Vault, account: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
    let id = send_id(call)?;
    let loaded = vault.load(account).await?;
    let send = find_send(&loaded, id)?;
    let keys = SendKeys::open(&loaded.key, send)?;
    let meta = Meta::new(send, Some(&keys));
    let access_id = access_id(send)?;
    let link = keys.link(loaded.session.server.as_str(), &access_id);
    let mut body = format!("Link: {link}\n");
    if meta.file {
        let file = &send["file"];
        let name = keys.decrypt(&file["fileName"]).map(|n| text::one_line(&n)).unwrap_or_default();
        let size = match (file["sizeName"].as_str(), &file["size"]) {
            (Some(shown), _) => shown.to_owned(),
            (None, Value::String(s)) => format!("{s} bytes"),
            (None, Value::Number(n)) => format!("{n} bytes"),
            _ => "unknown size".to_owned(),
        };
        body = format!("{body}File: {name} ({size})\n");
    } else {
        let content = keys.decrypt(&send["text"]["text"]).unwrap_or_default();
        body.push_str("Text:\n");
        body.push_str(&content);
    }
    if meta.has_password {
        body.push_str("\nThe Send has a password; it cannot be read back from the vault.");
    }
    Ok(vec![Item {
        snippet: format!("Link and content of the Send \"{}\"", meta.name),
        body: Some(body),
        sensitive: true,
        secret: true,
        ..item_of(&meta, String::new())
    }])
}

/// The id recipients use in the link; the server names it, else it is the Send's id as bytes.
fn access_id(send: &Value) -> Result<String, CoreError> {
    if let Some(a) = send["accessId"].as_str().filter(|a| id_ok(a)) {
        return Ok(a.to_owned());
    }
    send["id"]
        .as_str()
        .and_then(|id| uuid::Uuid::parse_str(id).ok())
        .map(|u| BASE64URL_NOPAD.encode(u.as_bytes()))
        .ok_or_else(|| CoreError::service("The vault did not say how to reach that Send."))
}

// ---- Arguments of writes ----

/// A change to an optional value.
enum Set<T> {
    Keep,
    Clear,
    To(T),
}

fn has_zone(when: &str) -> bool {
    let Some((_, clock)) = when.split_once(['T', 't', ' ']) else {
        return true;
    };
    clock.ends_with(['Z', 'z']) || clock.rfind(['+', '-']).is_some_and(|i| i > 0)
}

fn moment(name: &str, raw: &str) -> Result<i64, CoreError> {
    if !has_zone(raw) {
        return Err(CoreError::service(format!(
            "`{name}` needs a time zone: write it like 2026-10-05T14:00:00+02:00 or ...Z."
        )));
    }
    text::parse_when(raw).ok_or_else(|| {
        CoreError::service(format!("`{name}` is not a date: write 2026-10-05 or 2026-10-05T14:00:00+02:00."))
    })
}

struct Timing {
    expiration: Set<i64>,
    /// `None`: not given (keep it when updating).
    deletion: Option<i64>,
}

fn timing(call: &ConnectorCall, now: i64, creating: bool) -> Result<Timing, CoreError> {
    let (hours, expires) = (call.int_arg("expires_in_hours"), call.str_arg("expiration_date"));
    if hours.is_some() && expires.is_some() {
        return Err(CoreError::service("Give either `expires_in_hours` or `expiration_date`, not both."));
    }
    let expiration = match (hours, expires) {
        (Some(0), _) => Set::Clear,
        (Some(h), _) => Set::To(now.saturating_add(h.saturating_mul(3_600))),
        (None, Some(date)) => Set::To(moment("expiration_date", date)?),
        (None, None) => Set::Keep,
    };
    if let Set::To(t) = expiration
        && t <= now
    {
        return Err(CoreError::service("The expiration must be in the future."));
    }
    let (days, date) = (call.int_arg("delete_in_days"), call.str_arg("deletion_date"));
    if days.is_some() && date.is_some() {
        return Err(CoreError::service("Give either `delete_in_days` or `deletion_date`, not both."));
    }
    let deletion = match (days, date) {
        (Some(d), _) => Some(now.saturating_add(d.saturating_mul(86_400))),
        (None, Some(date)) => Some(moment("deletion_date", date)?),
        (None, None) => creating.then(|| now + DEFAULT_DELETE_DAYS * 86_400),
    };
    if let Some(d) = deletion {
        if d <= now {
            return Err(CoreError::service("The deletion date must be in the future."));
        }
        if d > now + MAX_KEEP_SECS {
            return Err(CoreError::service(
                "A Send can be kept for 31 days at most: choose a deletion date less than 31 days from now.",
            ));
        }
        if let Set::To(e) = expiration
            && e > d
        {
            return Err(CoreError::service("The Send would be deleted before it expires: move the deletion later."));
        }
    }
    Ok(Timing {
        expiration,
        deletion,
    })
}

/// The bytes of a base64 file (padded or not, with or without line breaks).
fn decode_file(encoded: &str) -> Result<Zeroizing<Vec<u8>>, CoreError> {
    let cleaned: String = encoded.chars().filter(|c| !c.is_whitespace()).collect();
    if cleaned.len() > MAX_FILE / 3 * 4 + 8 {
        return Err(CoreError::service("The file is larger than 2 MB."));
    }
    let bytes = BASE64
        .decode(cleaned.as_bytes())
        .or_else(|_| BASE64_NOPAD.decode(cleaned.as_bytes()))
        .map_err(|_| CoreError::service("`content_base64` is not valid base64."))?;
    if bytes.is_empty() {
        return Err(CoreError::service("The file is empty."));
    }
    if bytes.len() > MAX_FILE {
        return Err(CoreError::service("The file is larger than 2 MB."));
    }
    Ok(Zeroizing::new(bytes))
}

#[allow(clippy::struct_excessive_bools, reason = "a Send has this many independent switches")]
struct Create {
    file: bool,
    name: String,
    notes: Option<String>,
    text: Option<Zeroizing<String>>,
    hidden: bool,
    file_name: Option<String>,
    data: Option<Zeroizing<Vec<u8>>>,
    /// The file was uploaded through the server instead (`blob`), read when the Send is made.
    blob: Option<String>,
    password: Option<Zeroizing<String>>,
    max_access_count: Option<i64>,
    timing: Timing,
    disabled: bool,
    hide_email: bool,
}

fn create_spec(call: &ConnectorCall, now: i64) -> Result<Create, CoreError> {
    let file = call.str_arg("type") == Some("file");
    let given = |name: &str| call.args.contains_key(name);
    let (text_given, file_given) =
        (given("text") || given("hidden"), given("file_name") || given("content_base64") || given("blob"));
    let blob = call.str_arg("blob").filter(|b| !b.is_empty()).map(str::to_owned);
    let (text, file_name, data) = if file {
        if text_given {
            return Err(CoreError::service(
                "`text` and `hidden` belong to a text Send; a file Send takes `file_name` and `content_base64`.",
            ));
        }
        let name = call.str_arg("file_name").filter(|n| !n.is_empty());
        let content = call.str_arg("content_base64").filter(|c| !c.is_empty());
        match (name, content, &blob) {
            (Some(_), Some(_), Some(_)) => {
                return Err(CoreError::service("Give either `content_base64` or `blob`, not both."));
            }
            (Some(name), None, Some(_)) => (None, Some(text::one_line(name)), None),
            (Some(name), Some(content), None) => (None, Some(text::one_line(name)), Some(decode_file(content)?)),
            _ => return Err(CoreError::service("A file Send needs `file_name` and `content_base64`.")),
        }
    } else {
        if file_given {
            return Err(CoreError::service(
                "`file_name` and `content_base64` belong to a file Send; a text Send takes `text`.",
            ));
        }
        let content = call.str_arg("text").filter(|t| !t.is_empty());
        let Some(content) = content else {
            return Err(CoreError::service("A text Send needs `text`."));
        };
        (Some(Zeroizing::new(content.to_owned())), None, None)
    };
    let password = call.str_arg("password").filter(|p| !p.is_empty()).map(|p| Zeroizing::new(p.to_owned()));
    Ok(Create {
        file,
        name: call
            .str_arg("name")
            .map(text::one_line)
            .filter(|n| !n.is_empty())
            .ok_or_else(|| CoreError::service("A Send needs a `name`."))?,
        notes: call.str_arg("notes").filter(|n| !n.is_empty()).map(str::to_owned),
        text,
        hidden: call.bool_arg("hidden").unwrap_or(false),
        file_name,
        data,
        blob,
        password,
        max_access_count: call.int_arg("max_access_count"),
        timing: timing(call, now, true)?,
        disabled: call.bool_arg("disabled").unwrap_or(false),
        hide_email: call.bool_arg("hide_email").unwrap_or(false),
    })
}

/// `text` shortened for a preview, one line, with how much was left out.
fn excerpt(s: &str) -> String {
    let shown = text::one_line(&text::truncate_chars(s, PREVIEW_CHARS));
    let total = s.chars().count();
    if total > PREVIEW_CHARS {
        format!("{shown}... ({} more characters)", total - PREVIEW_CHARS)
    } else {
        shown
    }
}

fn size_text(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes} bytes")
    } else {
        format!("{:.1} KB", f64::from(u32::try_from(bytes).unwrap_or(u32::MAX)) / 1024.0)
    }
}

fn preview_create(call: &ConnectorCall) -> Result<Preview, CoreError> {
    let spec = create_spec(call, now())?;
    let mut lines = vec![match (&spec.file_name, &spec.data) {
        (Some(file), Some(data)) => {
            format!("Create a file Send \"{}\" for {file} ({})", spec.name, size_text(data.len()))
        }
        (Some(file), None) => format!("Create a file Send \"{}\" for {file} (the uploaded file, below)", spec.name),
        _ => format!("Create a text Send \"{}\"", spec.name),
    }];
    if let Some(content) = &spec.text {
        let chars = content.chars().count();
        lines.push(if spec.hidden {
            format!("Text: {chars} characters, hidden from recipients until they show it (not shown here)")
        } else {
            format!("Text ({chars} characters): {}", excerpt(content))
        });
    }
    if let Some(notes) = &spec.notes {
        lines.push(format!("Notes: {}", excerpt(notes)));
    }
    if let Some(d) = spec.timing.deletion {
        lines.push(format!("Deleted on {}", text::iso_utc(d)));
    }
    lines.push(match spec.timing.expiration {
        Set::To(e) => format!("Stops opening on {}", text::iso_utc(e)),
        _ => "Never expires before it is deleted".to_owned(),
    });
    lines.push(match spec.max_access_count {
        Some(n) => format!("Opens at most {n} times"),
        None => "No limit on how often it is opened".to_owned(),
    });
    lines.push(if spec.password.is_some() {
        "Protected by a password (not shown here)".to_owned()
    } else {
        "No password: anyone with the link can open it".to_owned()
    });
    if spec.hide_email {
        lines.push("Hides your email address from recipients".to_owned());
    }
    if spec.disabled {
        lines.push("Created switched off: nobody can open it yet".to_owned());
    }
    lines.push("The link is given to the AI once it is created".to_owned());
    Ok(Preview {
        resource: SENDS.to_owned(),
        resource_label: SENDS_LABEL.to_owned(),
        lines,
        parents: Vec::new(),
        once_only: false,
        ..Preview::default()
    })
}

fn set_iso(change: &Set<i64>, current: &Value) -> Value {
    match change {
        Set::Keep => current.clone(),
        Set::Clear => Value::Null,
        Set::To(t) => json!(text::iso_utc(*t)),
    }
}

async fn create(vault: &Vault, account: &str, call: &ConnectorCall) -> Result<Value, CoreError> {
    let spec = create_spec(call, now())?;
    let session = vault.session()?;
    let user = vault.key(account)?;
    let keys = SendKeys::generate()?;
    let password = match &spec.password {
        Some(p) => Some(password_hash_blocking(p, &keys.raw).await?),
        None => None,
    };
    let mut body = json!({
        "type": i32::from(spec.file),
        "key": user.encrypt(&keys.raw)?,
        "name": keys.encrypt(&spec.name)?,
        "notes": spec.notes.as_deref().map(|n| keys.encrypt(n)).transpose()?,
        "text": Value::Null,
        "file": Value::Null,
        "fileLength": Value::Null,
        "password": password,
        "maxAccessCount": spec.max_access_count,
        "expirationDate": set_iso(&spec.timing.expiration, &Value::Null),
        "deletionDate": spec.timing.deletion.map(text::iso_utc),
        "disabled": spec.disabled,
        "hideEmail": spec.hide_email,
    });
    let uploaded = match &spec.blob {
        Some(id) if spec.file => Some(crate::blob::read(id, crate::blob::MAX_READ_BYTES).await?),
        _ => None,
    };
    let sent = if let (Some(file_name), Some(data)) = (&spec.file_name, spec.data.as_ref().or(uploaded.as_ref())) {
        let encrypted_name = keys.encrypt(file_name)?;
        let buffer = keys.encrypt_buffer(data)?;
        body["file"] = json!({"fileName": encrypted_name});
        body["fileLength"] = json!(buffer.len());
        let created = vault.api(&session, Method::POST, "/api/sends/file/v2", Some(&body)).await?;
        let sent = created["sendResponse"].clone();
        let path = upload_path(&created, &sent)?;
        let name = encrypted_name.as_str().unwrap_or_default().to_owned();
        let uploaded = vault
            .api_form(&session, &path, || {
                Form::new().percent_encode_noop().part("data", Part::bytes(buffer.clone()).file_name(name.clone()))
            })
            .await;
        if let Err(e) = uploaded {
            // Do not leave a Send without its file behind.
            if let Some(id) = sent["id"].as_str().filter(|id| id_ok(id)) {
                vault.api(&session, Method::DELETE, &format!("/api/sends/{id}"), None).await.ok();
            }
            return Err(e);
        }
        if let Some(id) = &spec.blob {
            crate::blob::used(id).await;
        }
        sent
    } else {
        body["text"] = json!({
            "text": keys.encrypt(spec.text.as_deref().map_or("", String::as_str))?,
            "hidden": spec.hidden,
        });
        vault.api(&session, Method::POST, "/api/sends", Some(&body)).await?
    };
    let id = sent["id"]
        .as_str()
        .filter(|id| id_ok(id))
        .ok_or_else(|| CoreError::service("The vault did not create the Send."))?;
    let link = keys.link(session.server.as_str(), &access_id(&sent)?);
    Ok(json!({
        "created": true,
        "id": id,
        "type": if spec.file { "file" } else { "text" },
        "name": spec.name,
        "link": link,
        "expiration_date": sent["expirationDate"].as_str().map(moment_text),
        "deletion_date": sent["deletionDate"].as_str().map(moment_text),
        "max_access_count": spec.max_access_count,
        "password_protected": spec.password.is_some(),
    }))
}

/// Where the file goes, from the server's answer to the first step; only a direct upload to the same server.
fn upload_path(created: &Value, sent: &Value) -> Result<String, CoreError> {
    let wrong = || CoreError::service("The vault answered in a way Reins cannot upload files to.");
    if created["fileUploadType"].as_i64().unwrap_or(0) != 0 {
        return Err(CoreError::service("This vault keeps Send files somewhere else, which Reins cannot upload to."));
    }
    let send_id = sent["id"].as_str().filter(|id| id_ok(id)).ok_or_else(wrong)?;
    let url = created["url"].as_str().ok_or_else(wrong)?;
    let parts: Vec<&str> = url.trim_start_matches('/').split('/').collect();
    match parts.as_slice() {
        ["sends", id, "file", file_id] if *id == send_id && id_ok(file_id) => {
            Ok(format!("/api/sends/{send_id}/file/{file_id}"))
        }
        _ => Err(wrong()),
    }
}

// ---- Update ----

struct UpdatePlan {
    name: Option<String>,
    /// An empty string removes the notes.
    notes: Option<String>,
    text: Option<Zeroizing<String>>,
    hidden: Option<bool>,
    /// 0 removes the limit.
    max_access_count: Option<i64>,
    timing: Timing,
    password: Option<Zeroizing<String>>,
    disabled: Option<bool>,
    hide_email: Option<bool>,
}

fn update_plan(call: &ConnectorCall, now: i64) -> Result<UpdatePlan, CoreError> {
    let text = match call.str_arg("text") {
        Some("") => return Err(CoreError::service("A text Send cannot have an empty text.")),
        other => other.map(|t| Zeroizing::new(t.to_owned())),
    };
    let password = match call.str_arg("password") {
        Some("") => {
            return Err(CoreError::service(
                "The password cannot be empty; use vault_send_remove_password to remove it.",
            ));
        }
        other => other.map(|p| Zeroizing::new(p.to_owned())),
    };
    let plan = UpdatePlan {
        name: call.str_arg("name").map(text::one_line).filter(|n| !n.is_empty()),
        notes: call.str_arg("notes").map(str::to_owned),
        text,
        hidden: call.bool_arg("hidden"),
        max_access_count: call.int_arg("max_access_count"),
        timing: timing(call, now, false)?,
        password,
        disabled: call.bool_arg("disabled"),
        hide_email: call.bool_arg("hide_email"),
    };
    let nothing = plan.name.is_none()
        && plan.notes.is_none()
        && plan.text.is_none()
        && plan.hidden.is_none()
        && plan.max_access_count.is_none()
        && matches!(plan.timing.expiration, Set::Keep)
        && plan.timing.deletion.is_none()
        && plan.password.is_none()
        && plan.disabled.is_none()
        && plan.hide_email.is_none();
    if nothing || (call.args.contains_key("name") && plan.name.is_none()) {
        return Err(CoreError::service("Give at least one field to change (and a `name` cannot be empty)."));
    }
    Ok(plan)
}

fn check_update_fits(plan: &UpdatePlan, file: bool) -> Result<(), CoreError> {
    if file && (plan.text.is_some() || plan.hidden.is_some()) {
        return Err(CoreError::service("A file Send has no text to change, and its file cannot be replaced."));
    }
    Ok(())
}

fn yes_no(on: bool) -> &'static str {
    if on {
        "yes"
    } else {
        "no"
    }
}

async fn preview_update(vault: &Vault, account: &str, call: &ConnectorCall) -> Result<Preview, CoreError> {
    let id = send_id(call)?;
    let plan = update_plan(call, now())?;
    let loaded = vault.load(account).await?;
    let send = find_send(&loaded, id)?;
    let keys = SendKeys::open(&loaded.key, send)?;
    let meta = Meta::new(send, Some(&keys));
    check_update_fits(&plan, meta.file)?;
    let mut lines = vec![format!("Change the Send \"{}\"", meta.name)];
    if let Some(name) = &plan.name {
        lines.push(format!("Name: \"{}\" -> \"{name}\"", meta.name));
    }
    if let Some(notes) = &plan.notes {
        lines.push(if notes.is_empty() {
            "Notes: removed".to_owned()
        } else {
            format!("Notes: {}", excerpt(notes))
        });
    }
    let hidden_now = plan.hidden.unwrap_or_else(|| send["text"]["hidden"].as_bool().unwrap_or(false));
    if let Some(content) = &plan.text {
        let chars = content.chars().count();
        lines.push(if hidden_now {
            format!("Text: replaced by {chars} characters, hidden from recipients until they show it (not shown here)")
        } else {
            format!("Text: replaced by {chars} characters: {}", excerpt(content))
        });
    }
    if let Some(hidden) = plan.hidden {
        lines.push(format!("Text hidden until recipients show it: {}", yes_no(hidden)));
    }
    if let Some(max) = plan.max_access_count {
        let old = meta.max_access_count.map_or("no limit".to_owned(), |m| m.to_string());
        lines.push(format!(
            "Most views: {old} -> {}",
            if max == 0 {
                "no limit".to_owned()
            } else {
                max.to_string()
            }
        ));
    }
    match plan.timing.expiration {
        Set::Keep => {}
        Set::Clear => lines
            .push(format!("Expiration: {} -> none", meta.expiration.as_deref().map_or("none".to_owned(), moment_text))),
        Set::To(t) => lines.push(format!(
            "Expiration: {} -> {}",
            meta.expiration.as_deref().map_or("none".to_owned(), moment_text),
            text::iso_utc(t)
        )),
    }
    if let Some(d) = plan.timing.deletion {
        lines.push(format!("Deletion: {} -> {}", moment_text(&meta.deletion), text::iso_utc(d)));
    }
    if plan.password.is_some() {
        lines.push(if meta.has_password {
            "Password: changed (not shown here)".to_owned()
        } else {
            "Password: set (not shown here)".to_owned()
        });
    }
    if let Some(disabled) = plan.disabled {
        lines.push(format!("Disabled: {} -> {}", yes_no(meta.disabled), yes_no(disabled)));
    }
    if let Some(hide) = plan.hide_email {
        lines.push(format!("Hide your email: {} -> {}", yes_no(meta.hide_email), yes_no(hide)));
    }
    lines.push("The link stays the same".to_owned());
    Ok(Preview {
        resource: send_resource(id),
        resource_label: meta.name,
        lines,
        parents: parents(),
        once_only: false,
        ..Preview::default()
    })
}

async fn update(vault: &Vault, account: &str, call: &ConnectorCall) -> Result<Value, CoreError> {
    let id = send_id(call)?;
    let plan = update_plan(call, now())?;
    let loaded = vault.load(account).await?;
    let send = find_send(&loaded, id)?;
    let keys = SendKeys::open(&loaded.key, send)?;
    let file = send["type"].as_i64() == Some(1);
    check_update_fits(&plan, file)?;
    let notes = match plan.notes.as_deref() {
        Some("") => Value::Null,
        Some(n) => keys.encrypt(n)?,
        None => send["notes"].clone(),
    };
    let content = if file {
        Value::Null
    } else {
        let stored = &send["text"]["text"];
        let text_field = match &plan.text {
            Some(t) => keys.encrypt(t)?,
            None if stored.is_string() => stored.clone(),
            None => return Err(CoreError::service("That Send has no text.")),
        };
        json!({
            "text": text_field,
            "hidden": plan.hidden.unwrap_or_else(|| send["text"]["hidden"].as_bool().unwrap_or(false)),
        })
    };
    let password = match &plan.password {
        Some(p) => Some(password_hash_blocking(p, &keys.raw).await?),
        None => None,
    };
    let body = json!({
        "type": send["type"],
        "key": send["key"],
        "name": match &plan.name { Some(n) => keys.encrypt(n)?, None => send["name"].clone() },
        "notes": notes,
        "text": content,
        "file": Value::Null,
        "fileLength": Value::Null,
        "password": password,
        "maxAccessCount": match plan.max_access_count {
            Some(0) => Value::Null,
            Some(n) => json!(n),
            None => send["maxAccessCount"].clone(),
        },
        "expirationDate": set_iso(&plan.timing.expiration, &send["expirationDate"]),
        "deletionDate": plan.timing.deletion.map_or_else(|| send["deletionDate"].clone(), |d| json!(text::iso_utc(d))),
        "disabled": plan.disabled.unwrap_or_else(|| send["disabled"].as_bool().unwrap_or(false)),
        "hideEmail": plan.hide_email.unwrap_or_else(|| send["hideEmail"].as_bool().unwrap_or(false)),
    });
    let updated = vault.api(&loaded.session, Method::PUT, &format!("/api/sends/{id}"), Some(&body)).await?;
    Ok(json!({
        "updated": true,
        "id": id,
        "expiration_date": updated["expirationDate"].as_str().map(moment_text),
        "deletion_date": updated["deletionDate"].as_str().map(moment_text),
        "password_protected": !updated["password"].is_null() || updated["authType"].as_i64() == Some(1),
    }))
}

// ---- Remove the password, delete ----

async fn preview_remove_password(vault: &Vault, account: &str, call: &ConnectorCall) -> Result<Preview, CoreError> {
    let id = send_id(call)?;
    let loaded = vault.load(account).await?;
    let send = find_send(&loaded, id)?;
    let meta = Meta::new(send, SendKeys::open(&loaded.key, send).ok().as_ref());
    if !meta.has_password {
        return Err(CoreError::service("That Send has no password."));
    }
    Ok(Preview {
        resource: send_resource(id),
        resource_label: meta.name.clone(),
        lines: vec![
            format!("Remove the password of the Send \"{}\"", meta.name),
            "Anyone with the link will be able to open it".to_owned(),
        ],
        parents: parents(),
        once_only: false,
        ..Preview::default()
    })
}

async fn remove_password(vault: &Vault, call: &ConnectorCall) -> Result<Value, CoreError> {
    let id = send_id(call)?;
    let session = vault.session()?;
    vault.api(&session, Method::PUT, &format!("/api/sends/{id}/remove-password"), None).await?;
    Ok(json!({"password_removed": true, "id": id}))
}

async fn preview_delete(vault: &Vault, account: &str, call: &ConnectorCall) -> Result<Preview, CoreError> {
    let id = send_id(call)?;
    let loaded = vault.load(account).await?;
    let send = find_send(&loaded, id)?;
    let meta = Meta::new(send, SendKeys::open(&loaded.key, send).ok().as_ref());
    Ok(Preview {
        resource: send_resource(id),
        resource_label: meta.name.clone(),
        lines: vec![
            format!("Delete the Send \"{}\" for good", meta.name),
            format!(
                "{}, opened {} times so far",
                if meta.file {
                    "File Send"
                } else {
                    "Text Send"
                },
                meta.access_count
            ),
            "Its link stops working".to_owned(),
        ],
        parents: parents(),
        once_only: false,
        ..Preview::default()
    })
}

async fn delete(vault: &Vault, call: &ConnectorCall) -> Result<Value, CoreError> {
    let id = send_id(call)?;
    let session = vault.session()?;
    vault.api(&session, Method::DELETE, &format!("/api/sends/{id}"), None).await?;
    Ok(json!({"deleted": true, "id": id}))
}

// ---- Randomness ----

/// A source of random bytes; the phone's system generator, or a fixed one in tests.
trait Entropy {
    fn fill(&mut self, buf: &mut [u8]) -> Result<(), CoreError>;
}

struct Secure(SystemRandom);

impl Secure {
    fn new() -> Self {
        Self(SystemRandom::new())
    }
}

impl Entropy for Secure {
    fn fill(&mut self, buf: &mut [u8]) -> Result<(), CoreError> {
        self.0.fill(buf).map_err(|_| CoreError::service("The phone's random number generator failed."))
    }
}

/// A uniform number in `0..n`. Numbers from the part of the 32-bit range that does not divide evenly into `n` are
/// thrown away, so no value is more likely than another (no modulo bias).
fn below<R: Entropy>(rng: &mut R, n: usize) -> Result<usize, CoreError> {
    let n = u32::try_from(n).ok().filter(|n| *n > 0).ok_or_else(|| CoreError::invalid("nothing to choose from"))?;
    let zone = (u32::MAX / n) * n;
    loop {
        let mut bytes = [0u8; 4];
        rng.fill(&mut bytes)?;
        let r = u32::from_le_bytes(bytes);
        if r < zone {
            return usize::try_from(r % n).map_err(|_| CoreError::invalid("number out of range"));
        }
    }
}

fn pick<'a, T, R: Entropy>(rng: &mut R, from: &'a [T]) -> Result<&'a T, CoreError> {
    Ok(&from[below(rng, from.len())?])
}

/// Fisher-Yates.
fn shuffle<T, R: Entropy>(rng: &mut R, items: &mut [T]) -> Result<(), CoreError> {
    for i in (1..items.len()).rev() {
        items.swap(i, below(rng, i + 1)?);
    }
    Ok(())
}

fn generated(id: &str, title: &str, snippet: String, value: String) -> Item {
    Item {
        id: id.to_owned(),
        resource: GENERATOR.to_owned(),
        resource_label: GENERATOR_LABEL.to_owned(),
        title: title.to_owned(),
        snippet,
        body: Some(value),
        // Not from the vault, so asking is enough; but the value is never shown or kept in the log.
        sensitive: false,
        secret: true,
        ..Item::default()
    }
}

// ---- Password ----

struct Class {
    name: &'static str,
    chars: Vec<char>,
    min: usize,
}

fn password_classes(call: &ConnectorCall, length: usize) -> Result<Vec<Class>, CoreError> {
    let mut excluded: Vec<char> = call.str_arg("exclude").unwrap_or_default().chars().collect();
    if call.bool_arg("avoid_ambiguous").unwrap_or(false) {
        excluded.extend(AMBIGUOUS.chars());
    }
    let mut classes = Vec::new();
    for (name, set, min_name) in [
        ("uppercase", UPPER, "min_uppercase"),
        ("lowercase", LOWER, "min_lowercase"),
        ("numbers", DIGITS, "min_numbers"),
        ("symbols", SYMBOLS, "min_symbols"),
    ] {
        let min = usize::try_from(call.int_arg(min_name).unwrap_or(0)).unwrap_or(0);
        if !call.bool_arg(name).unwrap_or(true) {
            if min > 0 {
                return Err(CoreError::service(format!("`{min_name}` needs `{name}` to be on.")));
            }
            continue;
        }
        let chars: Vec<char> = set.chars().filter(|c| !excluded.contains(c)).collect();
        if chars.is_empty() {
            return Err(CoreError::service(format!(
                "Nothing is left of `{name}` after `exclude` and `avoid_ambiguous`."
            )));
        }
        classes.push(Class {
            name,
            chars,
            min,
        });
    }
    if classes.is_empty() {
        return Err(CoreError::service("Turn on at least one of uppercase, lowercase, numbers and symbols."));
    }
    let asked: usize = classes.iter().map(|c| c.min).sum();
    if asked > length {
        return Err(CoreError::service(format!("The minimums add up to {asked}, more than the length {length}.")));
    }
    // Every class that is on shows up at least once, as far as the length allows.
    let mut total = asked;
    for class in &mut classes {
        if class.min == 0 && total < length {
            class.min = 1;
            total += 1;
        }
    }
    Ok(classes)
}

fn generate_password<R: Entropy>(rng: &mut R, call: &ConnectorCall) -> Result<Item, CoreError> {
    let length = usize::try_from(call.int_arg("length").unwrap_or(16)).unwrap_or(16).clamp(5, 128);
    let classes = password_classes(call, length)?;
    let mut chars: Vec<char> = Vec::with_capacity(length);
    for class in &classes {
        for _ in 0..class.min {
            chars.push(*pick(rng, &class.chars)?);
        }
    }
    let all: Vec<char> = classes.iter().flat_map(|c| c.chars.iter().copied()).collect();
    while chars.len() < length {
        chars.push(*pick(rng, &all)?);
    }
    shuffle(rng, &mut chars)?;
    let value: Zeroizing<String> = Zeroizing::new(chars.into_iter().collect());
    let kinds: Vec<&str> = classes.iter().map(|c| c.name).collect();
    Ok(generated(
        "password",
        "Generated password",
        format!("Generated password: {length} characters ({})", kinds.join(", ")),
        value.to_string(),
    ))
}

// ---- Passphrase and username ----

/// The EFF large wordlist, `dice<TAB>word` per line.
fn wordlist() -> &'static [&'static str] {
    static WORDS: std::sync::OnceLock<Vec<&'static str>> = std::sync::OnceLock::new();
    WORDS.get_or_init(|| EFF_WORDLIST.lines().filter_map(|l| l.split_once('\t').map(|(_, w)| w)).collect())
}

fn capitalized(word: &str) -> String {
    let mut chars = word.chars();
    chars.next().map_or_else(String::new, |first| first.to_uppercase().chain(chars).collect())
}

fn generate_passphrase<R: Entropy>(rng: &mut R, call: &ConnectorCall) -> Result<Item, CoreError> {
    let count = usize::try_from(call.int_arg("words").unwrap_or(6)).unwrap_or(6).clamp(3, 20);
    let separator = call.str_arg("separator").unwrap_or("-");
    if separator.chars().any(char::is_control) {
        return Err(CoreError::service("The separator cannot contain control characters or line breaks."));
    }
    let capitalize = call.bool_arg("capitalize").unwrap_or(false);
    let mut words = Vec::with_capacity(count);
    for _ in 0..count {
        let word = *pick(rng, wordlist())?;
        words.push(if capitalize {
            capitalized(word)
        } else {
            word.to_owned()
        });
    }
    if call.bool_arg("include_number").unwrap_or(false) {
        let at = below(rng, count)?;
        words[at].push_str(&below(rng, 10)?.to_string());
    }
    Ok(generated(
        "passphrase",
        "Generated passphrase",
        format!("Generated passphrase: {count} words"),
        words.join(separator),
    ))
}

fn domain_ok(domain: &str) -> bool {
    let labels: Vec<&str> = domain.split('.').collect();
    domain.len() <= 253
        && labels.len() >= 2
        && labels.iter().all(|l| {
            (1..=63).contains(&l.len())
                && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
                && !l.starts_with('-')
                && !l.ends_with('-')
        })
}

/// Eight random lowercase letters and digits.
fn random_tag<R: Entropy>(rng: &mut R) -> Result<String, CoreError> {
    let alphabet: Vec<char> = LOWER.chars().chain(DIGITS.chars()).collect();
    (0..8).map(|_| pick(rng, &alphabet).copied()).collect()
}

fn generate_username<R: Entropy>(rng: &mut R, call: &ConnectorCall) -> Result<Item, CoreError> {
    let (value, what) = match call.str_arg("type").unwrap_or_default() {
        "random_word" => {
            let word = *pick(rng, wordlist())?;
            let mut name = if call.bool_arg("capitalize").unwrap_or(false) {
                capitalized(word)
            } else {
                word.to_owned()
            };
            if call.bool_arg("include_number").unwrap_or(false) {
                for _ in 0..4 {
                    name.push_str(&below(rng, 10)?.to_string());
                }
            }
            (name, "random word")
        }
        "plus_addressed_email" => {
            let email = call.str_arg("email").unwrap_or_default();
            let (local, domain) = email.split_once('@').unwrap_or_default();
            if local.is_empty()
                || local.len() > 64
                || local.contains(['@', '+', ' '])
                || !local.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
                || !domain_ok(domain)
            {
                return Err(CoreError::service(
                    "`email` must be the user's plain address (name@example.com, without a +tag).",
                ));
            }
            (format!("{local}+{}@{domain}", random_tag(rng)?), "plus-addressed email")
        }
        "catch_all_email" => {
            let domain = call.str_arg("domain").unwrap_or_default();
            if !domain_ok(domain) {
                return Err(CoreError::service("`domain` must be a domain name such as example.com."));
            }
            (format!("{}@{domain}", random_tag(rng)?), "catch-all email")
        }
        _ => return Err(CoreError::service("`type` must be random_word, plus_addressed_email or catch_all_email.")),
    };
    Ok(generated("username", "Generated username", format!("Generated username ({what})"), value))
}

// ---- Password strength ----

/// Very common passwords (lowercase, letters only unless a number is the password).
const COMMON: &[&str] = &[
    "password",
    "123456",
    "12345678",
    "123456789",
    "12345",
    "1234567",
    "1234567890",
    "qwerty",
    "qwertyuiop",
    "abc123",
    "monkey",
    "letmein",
    "dragon",
    "111111",
    "baseball",
    "iloveyou",
    "trustno",
    "sunshine",
    "master",
    "welcome",
    "shadow",
    "ashley",
    "football",
    "jesus",
    "michael",
    "ninja",
    "mustang",
    "admin",
    "administrator",
    "login",
    "princess",
    "starwars",
    "hello",
    "freedom",
    "whatever",
    "qazwsx",
    "654321",
    "superman",
    "batman",
    "changeme",
    "secret",
    "computer",
    "000000",
    "121212",
    "flower",
    "loveme",
    "cheese",
    "pokemon",
    "charlie",
    "donald",
    "access",
    "lovely",
    "696969",
    "hunter",
    "test",
    "guest",
    "root",
    "passw",
    "pass",
    "default",
    "google",
    "internet",
    "summer",
    "winter",
    "spring",
    "autumn",
    "soccer",
    "hockey",
    "killer",
    "george",
    "harley",
    "ranger",
    "jordan",
    "tigger",
    "buster",
    "thomas",
    "robert",
    "jennifer",
    "hannah",
    "andrew",
    "matrix",
    "asdfgh",
    "asdfghjkl",
    "zxcvbn",
    "zxcvbnm",
    "azerty",
    "1q2w3e",
    "1q2w3e4r",
    "q1w2e3r4",
    "letmein1",
    "welcome1",
    "iloveu",
    "biteme",
    "fuckyou",
    "blink",
    "abcdef",
    "abcdefg",
    "aaaaaa",
    "qwe123",
    "pass123",
    "user",
    "temp",
    "love",
    "angel",
    "money",
    "prince",
];

struct Assessment {
    length: usize,
    kinds: Vec<&'static str>,
    bits: f64,
    common: bool,
}

impl Assessment {
    fn rating(&self) -> &'static str {
        match self.bits {
            b if b < 28.0 => "very weak",
            b if b < 36.0 => "weak",
            b if b < 60.0 => "reasonable",
            b if b < 80.0 => "strong",
            _ => "very strong",
        }
    }
}

/// The password with look-alike symbols read as the letters they stand for, cut at the first letter-free tail.
fn is_common(password: &str) -> bool {
    let plain = password.to_lowercase();
    let read: String = plain
        .chars()
        .map(|c| match c {
            '0' => 'o',
            '1' => 'i',
            '3' => 'e',
            '4' | '@' => 'a',
            '5' | '$' => 's',
            '7' => 't',
            other => other,
        })
        .collect();
    let stripped = |s: &str| s.trim_end_matches(|c: char| !c.is_alphabetic()).to_owned();
    let starts_with_common = |s: &str| !s.is_empty() && COMMON.contains(&s);
    starts_with_common(&plain)
        || starts_with_common(&stripped(&plain))
        || starts_with_common(&read)
        || starts_with_common(&stripped(&read))
        || plain.chars().all(|c| c.is_ascii_digit())
            && plain.chars().collect::<std::collections::BTreeSet<_>>().len() <= 2
}

fn assess(password: &str) -> Assessment {
    let chars: Vec<char> = password.chars().collect();
    let (mut lower, mut upper, mut digit, mut symbol, mut other) = (false, false, false, false, false);
    for c in &chars {
        match c {
            'a'..='z' => lower = true,
            'A'..='Z' => upper = true,
            '0'..='9' => digit = true,
            c if c.is_ascii() => symbol = true,
            _ => other = true,
        }
    }
    let pool: f64 = [(lower, 26.0), (upper, 26.0), (digit, 10.0), (symbol, 33.0), (other, 100.0)]
        .iter()
        .filter(|(on, _)| *on)
        .map(|(_, n)| n)
        .sum();
    // Repeated characters and runs like abc or 321 add little.
    let mut seen = std::collections::BTreeSet::new();
    let mut effective = 0.0;
    for (i, c) in chars.iter().enumerate() {
        let run = i >= 2 && {
            let (a, b, c) =
                (i64::from(u32::from(chars[i - 2])), i64::from(u32::from(chars[i - 1])), i64::from(u32::from(*c)));
            b - a == c - b && (c - b).abs() == 1
        };
        effective += if !seen.insert(*c) || run {
            0.25
        } else {
            1.0
        };
    }
    let common = is_common(password);
    let mut bits = if pool > 0.0 {
        effective * pool.log2()
    } else {
        0.0
    };
    if common {
        bits = bits.min(15.0);
    }
    let kinds = [
        (lower, "lowercase"),
        (upper, "uppercase"),
        (digit, "numbers"),
        (symbol, "symbols"),
        (other, "other characters"),
    ]
    .iter()
    .filter(|(on, _)| *on)
    .map(|(_, name)| *name)
    .collect();
    Assessment {
        length: chars.len(),
        kinds,
        bits,
        common,
    }
}

fn check_password(call: &ConnectorCall) -> Result<Item, CoreError> {
    let password = Zeroizing::new(call.str_arg("password").unwrap_or_default().to_owned());
    if password.is_empty() {
        return Err(CoreError::service("`password` is required."));
    }
    let a = assess(&password);
    let bits = a.bits.round();
    let mut lines = vec![
        format!("Strength: {} (about {bits} bits)", a.rating()),
        format!("Length: {} characters", a.length),
        format!("Character types: {}", a.kinds.join(", ")),
        format!(
            "Very common password: {}",
            if a.common {
                "yes, choose another"
            } else {
                "no"
            }
        ),
    ];
    if a.length < 12 && !a.common {
        lines.push("Advice: 12 characters or more is much harder to guess.".to_owned());
    }
    lines.push("Checked on the phone only; the password was not sent or stored anywhere.".to_owned());
    Ok(Item {
        id: "check".to_owned(),
        resource: GENERATOR.to_owned(),
        resource_label: GENERATOR_LABEL.to_owned(),
        title: "Password strength".to_owned(),
        snippet: format!("Password strength: {} (about {bits} bits)", a.rating()),
        body: Some(lines.join("\n")),
        ..Item::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Replays fixed bytes, then panics: to show what `below` throws away.
    struct Fixed(std::collections::VecDeque<u8>);

    impl Entropy for Fixed {
        fn fill(&mut self, buf: &mut [u8]) -> Result<(), CoreError> {
            for b in buf {
                *b = self.0.pop_front().expect("ran out of test entropy");
            }
            Ok(())
        }
    }

    fn call(op: &str, args: &Value) -> ConnectorCall {
        ConnectorCall {
            service: "vault".to_owned(),
            op: op.to_owned(),
            args: args.as_object().unwrap().clone(),
        }
    }

    #[test]
    fn the_wordlist_is_the_eff_large_list() {
        let lines: Vec<&str> = EFF_WORDLIST.lines().collect();
        assert_eq!(lines.len(), 7776);
        assert_eq!(wordlist().len(), 7776);
        let mut dice = std::collections::BTreeSet::new();
        for line in lines {
            let (d, w) = line.split_once('\t').unwrap();
            assert!(d.len() == 5 && d.bytes().all(|b| (b'1'..=b'6').contains(&b)), "{line}");
            assert!(!w.is_empty() && w.chars().all(|c| c.is_ascii_lowercase() || c == '-'), "{line}");
            assert!(dice.insert(d), "{d} twice");
        }
        assert_eq!(wordlist()[0], "abacus");
        assert_eq!(wordlist()[7775], "zoom");
    }

    #[test]
    fn values_in_the_uneven_tail_of_the_range_are_thrown_away() {
        // n = 6: u32::MAX / 6 * 6 = 4_294_967_292; the last three values (…292, 293, 294) would favor 0..=2 for 4_294_967_295.
        let zone = (u32::MAX / 6) * 6;
        let mut bytes = Vec::new();
        for r in [u32::MAX, u32::MAX - 1, zone, 7] {
            bytes.extend_from_slice(&r.to_le_bytes());
        }
        let mut rng = Fixed(bytes.into());
        assert_eq!(below(&mut rng, 6).unwrap(), 1, "7 % 6 after the three rejected values");
        assert!(rng.0.is_empty(), "all four values were drawn");
        let mut just_below = Fixed((zone - 1).to_le_bytes().to_vec().into());
        assert_eq!(below(&mut just_below, 6).unwrap(), 5);
        assert!(below(&mut Fixed(std::collections::VecDeque::new()), 0).is_err());
    }

    #[test]
    fn choices_are_evenly_spread() {
        let mut rng = Secure::new();
        for n in [2usize, 3, 7, 10, 62] {
            let draws = 20_000 * n;
            let mut counts = vec![0usize; n];
            for _ in 0..draws {
                counts[below(&mut rng, n).unwrap()] += 1;
            }
            let as_f64 = |x: usize| f64::from(u32::try_from(x).unwrap());
            let expected = as_f64(draws) / as_f64(n);
            let chi: f64 = counts.iter().map(|c| (as_f64(*c) - expected).powi(2) / expected).sum();
            // Far above the 0.001 critical value for every n here (df <= 61: 100), so a fair generator never trips it.
            assert!(chi < 110.0, "n={n} chi={chi} counts={counts:?}");
        }
    }

    #[test]
    fn shuffling_moves_things_and_keeps_them() {
        let mut rng = Secure::new();
        let mut first_position = [0usize; 5];
        for _ in 0..5_000 {
            let mut v = [0, 1, 2, 3, 4];
            shuffle(&mut rng, &mut v).unwrap();
            first_position[v[0]] += 1;
            v.sort_unstable();
            assert_eq!(v, [0, 1, 2, 3, 4]);
        }
        assert!(first_position.iter().all(|c| (800..1_200).contains(c)), "{first_position:?}");
    }

    #[test]
    fn a_common_password_is_recognized_through_simple_disguises() {
        for p in ["password", "Password1!", "P@ssw0rd", "qwerty123", "123456", "111111", "iloveyou2020"] {
            assert!(assess(p).common, "{p}");
        }
        assert!(!assess("tr0ub4dor&3xylophone").common);
    }

    #[test]
    fn strength_grows_with_length_and_variety() {
        let weak = assess("abc").bits;
        let repeated = assess("aaaaaaaaaaaa").bits;
        let plain = assess("correcthorse").bits;
        let mixed = assess("C0rrect-Horse-Battery!").bits;
        assert!(weak < repeated && repeated < plain && plain < mixed, "{weak} {repeated} {plain} {mixed}");
        assert_eq!(assess("Zq7!kP2#vN9$xL4@").rating(), "very strong");
        assert_eq!(assess("abc").rating(), "very weak");
    }

    #[test]
    fn the_check_never_repeats_the_password() {
        let item = check_password(&call("generate_check_password", &json!({"password": "Tr0ub4dor&3-zebra"}))).unwrap();
        let everything = format!("{item:?}");
        assert!(!everything.contains("Tr0ub4dor") && !everything.contains("zebra"), "{everything}");
        assert!(!item.secret && !item.sensitive);
    }

    fn f(x: usize) -> f64 {
        f64::from(u32::try_from(x).unwrap())
    }

    fn password(args: &Value) -> String {
        generate_password(&mut Secure::new(), &call("generate_password", args)).unwrap().body.unwrap()
    }

    #[test]
    fn generated_characters_are_evenly_spread_and_the_guaranteed_ones_land_anywhere() {
        // Digits only: 10 symbols, 40 x 128 draws; chi-square with 9 degrees of freedom, critical value at p = 0.00001 is 39.2, so a fair generator never trips it.
        let only_digits = json!({"length": 128, "uppercase": false, "lowercase": false, "symbols": false});
        let mut counts = [0usize; 10];
        for _ in 0..200 {
            for c in password(&only_digits).chars() {
                counts[usize::try_from(c.to_digit(10).unwrap()).unwrap()] += 1;
            }
        }
        let total: usize = counts.iter().sum();
        assert_eq!(total, 200 * 128);
        let expected = f(total) / 10.0;
        let chi: f64 = counts.iter().map(|c| (f(*c) - expected).powi(2) / expected).sum();
        assert!(chi < 40.0, "chi={chi} {counts:?}");

        // Where the guaranteed capitals land: every place is equally likely (they are not stuck at the front).
        let args = json!({"length": 8, "min_uppercase": 4, "numbers": false, "symbols": false});
        let mut places = [0usize; 8];
        for _ in 0..4_000 {
            for (i, c) in password(&args).chars().enumerate() {
                if c.is_ascii_uppercase() {
                    places[i] += 1;
                }
            }
        }
        // 4 guaranteed capitals, 1 guaranteed lowercase letter and 3 free draws: 5.5 capitals in 8 places, 2 750 each.
        assert!(places.iter().all(|c| (2_550..2_950).contains(c)), "{places:?}");
    }

    #[test]
    fn the_minimums_hold_for_every_password_however_tight() {
        for _ in 0..500 {
            let p = password(
                &json!({"length": 10, "min_uppercase": 3, "min_lowercase": 3, "min_numbers": 2, "min_symbols": 2}),
            );
            let n = |set: &str| p.chars().filter(|c| set.contains(*c)).count();
            assert_eq!((n(UPPER), n(LOWER), n(DIGITS), n(SYMBOLS)), (3, 3, 2, 2), "{p}");
        }
        // Classes that are on appear once even with no minimum, at the smallest length.
        for _ in 0..500 {
            let p = password(&json!({"length": 5, "avoid_ambiguous": true}));
            assert!(p.chars().any(|c| UPPER.contains(c)) && p.chars().any(|c| LOWER.contains(c)), "{p}");
            assert!(p.chars().any(|c| DIGITS.contains(c)) && p.chars().any(|c| SYMBOLS.contains(c)), "{p}");
            assert!(!p.contains(['I', 'l', '1', 'O', '0']), "{p}");
        }
    }

    #[test]
    fn the_send_key_is_stretched_the_way_bitwarden_does() {
        // HKDF-SHA256(ikm = 16 x 0x01, salt = "bitwarden-send", info = "send", L = 64), computed independently.
        let keys = derive(&[1u8; 16]).unwrap();
        let sealed = keys.encrypt(b"x").unwrap();
        assert_eq!(keys.decrypt_text(&sealed).unwrap().as_str(), "x");
        let other = derive(&[2u8; 16]).unwrap();
        assert!(other.decrypt(&sealed).is_err());
        assert_eq!(keys.to_bytes().len(), 64);
    }
}
