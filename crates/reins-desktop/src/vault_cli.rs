//! `reins vault add NAME` and `reins vault list`: put a secret into the vault on the phone without handing it to an AI
//! (an account signed in with a passkey or SSO has no other way to add one), and see what the items are called.
//!
//! The value is read here without echo (or from stdin when piped), put in a box from this app's key to the phone's
//! own key, and sent to the phone, which shows "Save NAME in your vault?" and stores it encrypted with the vault key
//! once approved. The box also carries when it was made (the phone refuses an old or repeated one), the item, the
//! field, the kind and whether an existing item may be changed (`--replace`; the phone keeps the earlier value).
//! The terminal and the phone show the same four check digits of the value. The server relays the box without being
//! able to open or replace it, and nothing here keeps the value.
//!
//! The phone's key comes from the phone the first time (`vault_phone_key`, sealed to this app): the approval shows its
//! twelve digits, the user types them here, and the key is kept (`phone-key.json`) only when they match the key that
//! arrived; the digits that arrived are never shown, and this app's own pairing digits are refused. The key never
//! travels in the clear, so a server that answered with its own key could not know which digits to aim for. What the
//! phone sends back afterwards (that it saved, the names) is boxed from that key, so only the pinned phone can have
//! written it. A new phone, or the app reinstalled, cannot open the box; this app then says so and the user checks the
//! new key with `--new-phone`, never by itself.

use std::io::{IsTerminal as _, Read as _, Write as _};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use clap::{Subcommand, ValueEnum};
use reins_proto::desktop::{
    PHONE_KEY_CHANGED, PhoneKey, SEALED_FIELD, SecretToStore, StoreAck, TO_DESKTOP, TO_PHONE, VaultNames, decode_key,
    phone_key_fingerprint, printable, value_check,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use zeroize::Zeroizing;

use crate::auth::Refusal;
use crate::config::{Config, Mode, Paths};
use crate::identity::Identity;
use crate::journal::{Entry, Journal, Kind};
use crate::phone::{Phone, awaiting, teller};

pub const KEY_TOOL: &str = "vault_phone_key";
pub const STORE_TOOL: &str = "vault_secret_store";
pub const NAMES_TOOL: &str = "vault_names";
/// The largest value read (an RSA private key is a few kilobytes).
const MAX_VALUE: usize = 32 * 1024;

#[derive(Clone, Debug, Subcommand)]
pub enum VaultCmd {
    /// Save a secret in the vault on your phone. It is typed here without echo (or piped in), sealed so that only
    /// your phone can open it, and saved once you approve on the phone. An item that exists is changed only with
    /// --replace (its earlier value is kept).
    Add {
        /// The item's name: what `vault:NAME/password` refers to.
        name: String,
        /// What kind of item a new one is.
        #[arg(long, value_enum, default_value_t = ItemKind::ApiKey)]
        kind: ItemKind,
        /// The field to set (default: password; notes for a note; the private key for ssh). Any other name makes a
        /// custom field (hidden).
        #[arg(long)]
        field: Option<String>,
        /// Change the field of the item with this name if there is one (its earlier value is kept in the item).
        #[arg(long)]
        replace: bool,
        /// Your phone is new (or Reins was installed again): check its key again.
        #[arg(long)]
        new_phone: bool,
        /// The twelve digits your phone shows for its key, when this terminal cannot ask (the first time only).
        #[arg(long, value_name = "DIGITS")]
        phone_key: Option<String>,
    },
    /// The names of the items in the vault on your phone (never a value). Asked on the phone every time.
    List {
        /// The twelve digits your phone shows for its key, when this terminal cannot ask (the first time only).
        #[arg(long, value_name = "DIGITS")]
        phone_key: Option<String>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum ItemKind {
    /// A login with only the key, in its password: `vault:NAME/password`.
    ApiKey,
    Login,
    /// A secure note: `vault:NAME/notes`.
    Note,
    /// An SSH private key (OpenSSH format), for the SSH agent. Usually piped in: `< ~/.ssh/id_ed25519`.
    Ssh,
}

impl ItemKind {
    fn arg(self) -> &'static str {
        match self {
            Self::ApiKey => "api-key",
            Self::Login => "login",
            Self::Note => "note",
            Self::Ssh => "ssh",
        }
    }

    fn default_field(self) -> &'static str {
        match self {
            Self::ApiKey | Self::Login => "password",
            Self::Note => "notes",
            Self::Ssh => "private_key",
        }
    }
}

/// The phone key this app checked with the user, for the server it is logged in to.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
struct PinnedKey {
    server: String,
    public_key: String,
}

fn pin_file(paths: &Paths) -> PathBuf {
    paths.state_dir.join("phone-key.json")
}

fn pinned(paths: &Paths, server: &str) -> Option<String> {
    let text = std::fs::read_to_string(pin_file(paths)).ok()?;
    let pin: PinnedKey = serde_json::from_str(&text).ok()?;
    (pin.server == server && decode_key(&pin.public_key).is_some()).then_some(pin.public_key)
}

fn pin(paths: &Paths, server: &str, public_key: &str) -> Result<(), String> {
    let pin = PinnedKey {
        server: server.to_owned(),
        public_key: public_key.to_owned(),
    };
    let bytes = serde_json::to_vec_pretty(&pin).map_err(|e| e.to_string())?;
    crate::config::write_private(&pin_file(paths), &bytes).map_err(|e| format!("{}: {e}", pin_file(paths).display()))
}

fn unpin(paths: &Paths) {
    std::fs::remove_file(pin_file(paths)).ok();
}

/// Only the digits of what was typed ("4821-9930-1274", "4821 9930 1274", "482199301274").
fn digits_of(s: &str) -> String {
    s.chars().filter(char::is_ascii_digit).collect()
}

/// Whether the typed digits are the phone key's. This app's own pairing digits are refused outright: they are the
/// other number a user may find (in `reins status`, on the phone's computer list), and the one a server could aim for.
fn check_typed(typed: &str, phone_digits: &str, own_digits: &str) -> Result<(), String> {
    let typed = digits_of(typed);
    if !own_digits.is_empty() && typed == digits_of(own_digits) {
        return Err("Those are this computer's own key digits, not your phone's. Type the twelve digits your phone \
                    shows for its key (the bottom of the Vault page in the Reins app)."
            .to_owned());
    }
    if typed != digits_of(phone_digits) {
        // The digits that arrived are not shown: whoever sent them would only have the user copy them.
        return Err(
            "Not saved: the key this computer received is not the one your phone shows. Something between your \
                    phone and this computer changed it. Check that you typed your phone's twelve digits, and which \
                    Reins server you are logged in to (reins status)."
                .to_owned(),
        );
    }
    Ok(())
}

/// What the phone answered, as the server relayed it: the server could have written it, so it is said where it came
/// from and printed without control characters.
fn relayed(r: &Refusal) -> String {
    match r {
        // Our own words (no answer in time).
        Refusal::Waiting(m) => printable(m),
        Refusal::Denied(m) | Refusal::Unavailable(m) => format!("phone (via the Reins server): {}", printable(m)),
    }
}

/// Whether this process can ask at the terminal.
fn has_terminal() -> bool {
    #[cfg(unix)]
    return std::fs::OpenOptions::new().read(true).write(true).open("/dev/tty").is_ok();
    #[cfg(windows)]
    return std::fs::OpenOptions::new().read(true).write(true).open("CONIN$").is_ok();
    #[cfg(not(any(unix, windows)))]
    return false;
}

/// `--phone-key` is for a run without a terminal, and the first time only: with a terminal the digits are typed when
/// asked (so that no text printed earlier, which the server may have written, can supply them), and a new phone's key
/// is always typed.
fn phone_key_arg(given: Option<&str>, new_phone: bool, terminal: bool) -> Result<Option<&str>, String> {
    match given {
        Some(_) if new_phone => {
            Err("--phone-key cannot go with --new-phone: type the new phone's digits when asked, reading \
                                      them on the phone (the bottom of the Vault page)."
                .to_owned())
        }
        Some(_) if terminal => {
            Err("--phone-key is for runs without a terminal; here, type the digits when asked, reading \
                                     them on your phone (the bottom of the Vault page)."
                .to_owned())
        }
        other => Ok(other),
    }
}

/// A line typed by the user at the terminal, even when stdin is piped (`None` without one).
fn ask_terminal(prompt: &str) -> Option<String> {
    #[cfg(unix)]
    let tty = std::fs::OpenOptions::new().read(true).write(true).open("/dev/tty").ok()?;
    #[cfg(windows)]
    let tty = std::fs::OpenOptions::new().read(true).write(true).open("CONIN$").ok()?;
    #[cfg(not(any(unix, windows)))]
    return None;
    eprint!("{prompt}");
    std::io::stderr().flush().ok();
    let mut line = String::new();
    std::io::BufRead::read_line(&mut std::io::BufReader::new(tty), &mut line).ok()?;
    Some(line.trim().to_owned())
}

/// Asks the phone for its key and keeps it once the user has typed its digits.
async fn check_phone_key(phone: &Phone, paths: &Paths, given: Option<&str>) -> Result<String, String> {
    eprintln!(
        "First, your phone hands this computer its key. Approve \"Let the computer save secrets\" on your phone, and \
         note the twelve digits it shows."
    );
    let answer = phone
        .ask(KEY_TOOL, None, |_| json!({}), "No answer from your phone in time. Approve it, then run again.")
        .await
        .map_err(|r| relayed(&r))?;
    let key: PhoneKey = phone.open(&answer.data).map_err(|r| r.message().to_owned())?;
    if key.nonce != answer.nonce {
        return Err("The phone's answer is for another request; refused.".to_owned());
    }
    let digits = phone_key_fingerprint(&key.public_key).ok_or("The phone sent a malformed key; refused.")?;
    // Typed, not a yes: the user has to read the number on the phone.
    let typed = match given {
        Some(typed) => typed.to_owned(),
        None => ask_terminal(
            "Type the twelve digits your phone shows for its key (also at the bottom of the Vault page in the Reins app): ",
        )
        .ok_or(
            "Check your phone's key once: run reins vault add in a terminal, or pass --phone-key with the twelve \
             digits your phone shows.",
        )?,
    };
    check_typed(&typed, &digits, &phone.identity().fingerprint())?;
    pin(paths, phone.server(), &key.public_key)?;
    eprintln!("Your phone's key is kept on this computer; it is not asked for again.");
    Ok(key.public_key)
}

/// The value to save: piped in on stdin, or typed at the terminal without echo. Read into a buffer that is wiped and
/// never grows (so no copy is left behind).
fn read_value(name: &str, field: &str, kind: ItemKind) -> Result<Zeroizing<String>, String> {
    let mut value = if std::io::stdin().is_terminal() {
        if kind == ItemKind::Ssh {
            return Err("pipe the private key in: reins vault add NAME --kind ssh < ~/.ssh/id_ed25519".to_owned());
        }
        let typed = Zeroizing::new(
            rpassword::prompt_password(format!("{field} for {name} (not shown, Enter to finish): "))
                .map_err(|e| format!("reading from the terminal: {e}"))?,
        );
        Zeroizing::new(typed.to_string())
    } else {
        let mut bytes = Zeroizing::new(Vec::with_capacity(MAX_VALUE + 1));
        std::io::stdin()
            .take(MAX_VALUE as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| format!("reading stdin: {e}"))?;
        if bytes.len() > MAX_VALUE {
            return Err(format!("the value is longer than {} KB", MAX_VALUE / 1024));
        }
        let text = std::str::from_utf8(&bytes).map_err(|_| "the value is not text (UTF-8)".to_owned())?;
        let mut value = Zeroizing::new(String::with_capacity(text.len()));
        value.push_str(text);
        value
    };
    // What `echo` and editors add, not part of the value; a key keeps its own layout.
    if kind != ItemKind::Ssh {
        let kept = value.trim_end_matches(['\n', '\r']).len();
        value.truncate(kept);
    }
    if value.trim().is_empty() {
        return Err("nothing to save: the value is empty".to_owned());
    }
    Ok(value)
}

/// What is sent, in a box from this app's key to the phone's.
struct Outgoing<'a> {
    name: &'a str,
    field: &'a str,
    kind: ItemKind,
    replace: bool,
    value: &'a str,
}

fn box_value(identity: &Identity, phone_key: &str, nonce: &str, out: &Outgoing<'_>, now: i64) -> Option<String> {
    let secret = SecretToStore {
        v: 1,
        dir: TO_PHONE.to_owned(),
        nonce: nonce.to_owned(),
        created_at: now,
        name: out.name.to_owned(),
        field: out.field.to_owned(),
        kind: out.kind.arg().to_owned(),
        replace: out.replace,
        value: out.value.to_owned(),
    };
    // Room for the worst escaping up front, so the buffer never moves and leaves no copy behind.
    let mut plain = Zeroizing::new(Vec::with_capacity(out.value.len() * 6 + 1_024));
    serde_json::to_writer(&mut *plain, &secret).ok()?;
    identity.box_to(phone_key, &plain)
}

/// Opens what the pinned phone boxed for this app, and checks it answers this request.
fn open_from_phone<T: DeserializeOwned>(phone: &Phone, phone_key: &str, data: &Value) -> Result<T, String> {
    let refused = || "The answer is not from the phone this computer knows; refused.".to_owned();
    let boxed = data.get(SEALED_FIELD).and_then(Value::as_str).ok_or_else(refused)?;
    let plain = phone.identity().open_box_from(phone_key, boxed).map_err(|_| refused())?;
    serde_json::from_slice(&plain).map_err(|_| refused())
}

/// Sends the boxed value; what the phone answered, and the nonce the request carried.
async fn store(phone: &Phone, phone_key: &str, out: &Outgoing<'_>) -> Result<(Value, String), Refusal> {
    let answer = phone
        .ask(
            STORE_TOOL,
            None,
            |nonce| {
                json!({"name": out.name, "kind": out.kind.arg(), "field": out.field,
                    "sealed": box_value(phone.identity(), phone_key, nonce, out, crate::now_unix()).unwrap_or_default()})
            },
            "No answer from your phone in time. Approve it, then run again.",
        )
        .await?;
    Ok((answer.data, answer.nonce))
}

fn logged_in(paths: &Paths, config: &Config) -> Result<Phone, String> {
    if config.mode == Mode::Local {
        return Err("the vault is on your phone, and this app is in local mode (`mode = \"local\"` in config.toml); \
                    set `mode = \"auto\"` and run `reins login`"
            .to_owned());
    }
    if crate::server::oauth::logged_in_server(paths).is_none() {
        return Err("the vault is on your phone: run `reins login` to pair this app with it".to_owned());
    }
    // The value passes through this process: no other process of the user may read its memory.
    crate::harden::harden()?;
    let identity = Arc::new(Identity::load_or_create(&paths.identity_file()).map_err(|e| e.to_string())?);
    Phone::new(paths, identity, Duration::from_secs(config.approval_timeout_secs))
}

/// The pinned phone key, asking for it the first time (or again with `new_phone`).
async fn phone_key_for(phone: &Phone, paths: &Paths, given: Option<&str>, new_phone: bool) -> Result<String, String> {
    let given = phone_key_arg(given, new_phone, has_terminal())?;
    if new_phone {
        unpin(paths);
    }
    match pinned(paths, phone.server()) {
        Some(key) => Ok(key),
        None => check_phone_key(phone, paths, given).await,
    }
}

struct AddArgs<'a> {
    name: &'a str,
    kind: ItemKind,
    field: Option<&'a str>,
    replace: bool,
    new_phone: bool,
    given: Option<&'a str>,
}

async fn add(paths: &Paths, config: &Config, args: &AddArgs<'_>) -> Result<Vec<String>, String> {
    let name = args.name.trim();
    if name.is_empty() || name.chars().any(char::is_control) {
        return Err("give the item a name: reins vault add NAME".to_owned());
    }
    let kind = args.kind;
    let field = args.field.map_or(kind.default_field(), str::trim);
    let phone = logged_in(paths, config)?;
    let phone_key = phone_key_for(&phone, paths, args.given, args.new_phone).await?;
    let value = read_value(name, field, kind)?;
    eprintln!("Check: {}. Your phone shows the same four digits; approve only if it does.", value_check(&value));
    let journal = Journal::new(paths);
    let timeout = Duration::from_secs(config.approval_timeout_secs);
    let what = format!("save {name} in the vault");
    let entry = Entry::new(Kind::Secrets, &what).source(Some("reins vault add"));
    let tell = teller(|line: &str| eprintln!("{line}"));
    let out = Outgoing {
        name,
        field,
        kind,
        replace: args.replace,
        value: &value,
    };
    let sent = awaiting(&journal, entry, Some(tell), timeout, store(&phone, &phone_key, &out)).await;
    drop(value);
    let (data, nonce) = sent.map_err(|r| refusal_message(name, &r))?;
    let ack: StoreAck = open_from_phone(&phone, &phone_key, &data)?;
    if ack.dir != TO_DESKTOP || ack.nonce != nonce || ack.name != name {
        return Err("The phone's answer is for another request; refused.".to_owned());
    }
    let shown_field = if field == "private_key" {
        "private key"
    } else {
        field
    };
    let mut lines = vec![if ack.created {
        format!("Saved {name} in your vault.")
    } else {
        format!("Replaced the {shown_field} of {name} in your vault.")
    }];
    if let Some(kept) = &ack.kept {
        lines.push(format!("The earlier value is kept in {kept}."));
    }
    lines.push(if kind == ItemKind::Ssh {
        "The SSH agent offers it (reins ssh setup); signing happens on your phone.".to_owned()
    } else {
        format!("Use it as vault:{name}/{field}, for example: reins run --env API_KEY=vault:{name}/{field} -- ...")
    });
    Ok(lines)
}

/// What `reins vault add` says when the phone did not save. A phone that cannot open the box is never acted on by
/// itself (the server could say so too): the pinned key stays, and the user checks a new phone's key on purpose.
fn refusal_message(name: &str, r: &Refusal) -> String {
    match r {
        Refusal::Unavailable(m) if m.starts_with(PHONE_KEY_CHANGED) => format!(
            "Nothing was saved: your phone could not open it, so it is not the phone this computer knows. If you have a \
             new phone or installed Reins again, run reins vault add {} --new-phone and type the digits the new phone \
             shows. Otherwise something between your phone and this computer is interfering.",
            printable(name)
        ),
        other => relayed(other),
    }
}

async fn list(paths: &Paths, config: &Config, given: Option<&str>) -> Result<Vec<String>, String> {
    let phone = logged_in(paths, config)?;
    let phone_key = phone_key_for(&phone, paths, given, false).await?;
    let journal = Journal::new(paths);
    let entry = Entry::new(Kind::Secrets, "the names of the vault items").source(Some("reins vault list"));
    let tell = teller(|line: &str| eprintln!("{line}"));
    let asked =
        phone.ask(NAMES_TOOL, None, |_| json!({}), "No answer from your phone in time. Approve it, then run again.");
    let timeout = Duration::from_secs(config.approval_timeout_secs);
    let answer = awaiting(&journal, entry, Some(tell), timeout, asked).await.map_err(|r| relayed(&r))?;
    let names: VaultNames = open_from_phone(&phone, &phone_key, &answer.data)?;
    if names.dir != TO_DESKTOP || names.nonce != answer.nonce {
        return Err("The phone's answer is for another request; refused.".to_owned());
    }
    Ok(listing(&names.items))
}

/// One line per item: the name, and the kind when it is not a login.
fn listing(items: &[(String, String)]) -> Vec<String> {
    if items.is_empty() {
        return vec!["The vault is empty. Add a secret with: reins vault add NAME".to_owned()];
    }
    let width = items.iter().map(|(n, _)| n.chars().count()).max().unwrap_or(0).min(40);
    let mut lines: Vec<String> = items
        .iter()
        .map(|(name, kind)| {
            let kind = match kind.as_str() {
                "login" => "login",
                "note" => "note",
                "card" => "card",
                "identity" => "identity",
                "ssh_key" => "SSH key",
                _ => "",
            };
            format!("{name:<width$}  {kind}")
        })
        .collect();
    lines.push(String::new());
    lines
        .push("Use an item as vault:NAME/password (or /username, /notes, /totp, or a custom field's name).".to_owned());
    lines
}

/// Runs `reins vault ...`; the lines to print.
pub async fn run(cmd: &VaultCmd, paths: &Paths, config: &Config) -> Result<Vec<String>, String> {
    match cmd {
        VaultCmd::Add {
            name,
            kind,
            field,
            replace,
            new_phone,
            phone_key,
        } => {
            let args = AddArgs {
                name,
                kind: *kind,
                field: field.as_deref(),
                replace: *replace,
                new_phone: *new_phone,
                given: phone_key.as_deref(),
            };
            add(paths, config, &args).await
        }
        VaultCmd::List {
            phone_key,
        } => list(paths, config, phone_key.as_deref()).await,
    }
}

#[cfg(test)]
mod tests {
    use crypto_box::aead::OsRng;
    use data_encoding::BASE64URL_NOPAD;

    use super::*;

    #[test]
    fn the_typed_digits_must_be_the_phones_and_never_this_computers_own() {
        assert!(check_typed("4821-9930-1274", "4821-9930-1274", "1111 2222").is_ok());
        assert!(check_typed("4821 9930 1274", "4821-9930-1274", "1111 2222").is_ok());
        assert!(check_typed("482199301274", "4821-9930-1274", "1111 2222").is_ok());
        // The other number the user may find: refused, and said why.
        let own = check_typed("1111 2222", "1111-2222-0000", "1111 2222").unwrap_err();
        assert!(own.contains("this computer's own key"), "{own}");
        // A mismatch never shows the digits that arrived, so the user is not led to type them.
        let wrong = check_typed("4821-9930-1275", "9999-8888-7777", "1111 2222").unwrap_err();
        assert!(!wrong.contains("9999") && !wrong.contains("8888") && !wrong.contains("7777"), "{wrong}");
        assert!(check_typed("", "4821-9930-1274", "").is_err());
    }

    #[test]
    fn the_value_opens_with_the_phone_key_and_proves_this_app_sent_it() {
        use crypto_box::aead::Aead as _;
        let app = Identity::generate();
        let phone = crypto_box::SecretKey::generate(&mut OsRng);
        let public = reins_proto::desktop::encode_key(phone.public_key().as_bytes());
        let out = Outgoing {
            name: "OpenAI",
            field: "password",
            kind: ItemKind::ApiKey,
            replace: true,
            value: "sk-123",
        };
        let boxed = box_value(&app, &public, "n1", &out, 1_700_000_000).unwrap();
        assert!(!boxed.contains("sk-123"));
        let bytes = BASE64URL_NOPAD.decode(boxed.as_bytes()).unwrap();
        let (nonce, ciphertext) = bytes.split_at(24);
        let nonce = crypto_box::Nonce::from(<[u8; 24]>::try_from(nonce).unwrap());
        let app_key = crypto_box::PublicKey::from(decode_key(&app.public_key()).unwrap());
        let plain = crypto_box::SalsaBox::new(&app_key, &phone).decrypt(&nonce, ciphertext).unwrap();
        let opened: SecretToStore = serde_json::from_slice(&plain).unwrap();
        assert_eq!((opened.nonce.as_str(), opened.name.as_str(), opened.value.as_str()), ("n1", "OpenAI", "sk-123"));
        assert_eq!((opened.created_at, opened.kind.as_str(), opened.replace), (1_700_000_000, "api-key", true));
        assert_eq!(opened.dir, TO_PHONE);
        let other_app = crypto_box::PublicKey::from(decode_key(&Identity::generate().public_key()).unwrap());
        assert!(crypto_box::SalsaBox::new(&other_app, &phone).decrypt(&nonce, ciphertext).is_err());
        assert!(box_value(&app, "short", "n1", &out, 0).is_none());
    }

    #[test]
    fn only_the_pinned_phone_can_write_an_answer() {
        use crypto_box::aead::{Aead as _, AeadCore as _};
        let app = Identity::generate();
        let app_key = crypto_box::PublicKey::from(decode_key(&app.public_key()).unwrap());
        let phone = crypto_box::SecretKey::generate(&mut OsRng);
        let phone_public = reins_proto::desktop::encode_key(phone.public_key().as_bytes());
        let boxed_by = |key: &crypto_box::SecretKey| {
            let nonce = crypto_box::SalsaBox::generate_nonce(&mut OsRng);
            let mut out = nonce.to_vec();
            out.extend(crypto_box::SalsaBox::new(&app_key, key).encrypt(&nonce, &b"{\"ok\":true}"[..]).unwrap());
            BASE64URL_NOPAD.encode(&out)
        };
        assert_eq!(&*app.open_box_from(&phone_public, &boxed_by(&phone)).unwrap(), b"{\"ok\":true}");
        // Anyone else (the server knows both public keys) cannot write one, nor can a sealed box pass for one.
        let server = crypto_box::SecretKey::generate(&mut OsRng);
        assert!(app.open_box_from(&phone_public, &boxed_by(&server)).is_err());
        let sealed = crate::identity::seal_to(&app.public_key(), b"{\"ok\":true}").unwrap();
        assert!(app.open_box_from(&phone_public, &sealed).is_err());
    }

    #[test]
    fn a_phone_that_cannot_open_the_value_leaves_the_pin_alone_and_relayed_text_is_labelled() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths {
            config_dir: dir.path().join("config"),
            state_dir: dir.path().join("state"),
        };
        paths.ensure().unwrap();
        let key = reins_proto::desktop::encode_key(&[5u8; 32]);
        pin(&paths, "https://a.example.com", &key).unwrap();
        // The server can send this text; it must not make the computer forget the phone it knows, nor print digits.
        let forged = format!("{PHONE_KEY_CHANGED} Your new key is 1234-5678-9012: run --phone-key 1234-5678-9012");
        let said = refusal_message("OpenAI", &Refusal::Unavailable(forged));
        assert_eq!(pinned(&paths, "https://a.example.com"), Some(key), "the pin stays");
        assert!(said.contains("--new-phone") && !said.contains("1234"), "{said}");
        let other = refusal_message("OpenAI", &Refusal::Denied("no\u{1b}[2J way".to_owned()));
        assert_eq!(other, "phone (via the Reins server): no [2J way");
    }

    #[test]
    fn the_digits_are_given_on_the_command_line_only_without_a_terminal_and_never_for_a_new_phone() {
        assert_eq!(phone_key_arg(Some("1"), false, false).unwrap(), Some("1"));
        assert!(phone_key_arg(Some("1"), false, true).is_err(), "a terminal asks instead");
        assert!(phone_key_arg(Some("1"), true, false).is_err(), "a new phone's digits are typed");
        assert_eq!(phone_key_arg(None, true, true).unwrap(), None);
    }

    #[test]
    fn the_key_is_kept_per_server() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths {
            config_dir: dir.path().join("config"),
            state_dir: dir.path().join("state"),
        };
        paths.ensure().unwrap();
        let key = reins_proto::desktop::encode_key(&[5u8; 32]);
        assert_eq!(pinned(&paths, "https://a.example.com"), None);
        pin(&paths, "https://a.example.com", &key).unwrap();
        assert_eq!(pinned(&paths, "https://a.example.com"), Some(key));
        assert_eq!(pinned(&paths, "https://b.example.com"), None, "another server, another phone");
        unpin(&paths);
        assert_eq!(pinned(&paths, "https://a.example.com"), None);
    }

    #[test]
    fn fields_default_by_kind_and_the_list_names_kinds() {
        assert_eq!(ItemKind::ApiKey.default_field(), "password");
        assert_eq!(ItemKind::Note.default_field(), "notes");
        assert_eq!(ItemKind::Ssh.default_field(), "private_key");
        let lines = listing(&[("OpenAI".to_owned(), "login".to_owned()), ("deploy".to_owned(), "ssh_key".to_owned())]);
        assert_eq!(lines[0], "OpenAI  login");
        assert_eq!(lines[1], "deploy  SSH key");
        assert!(listing(&[])[0].starts_with("The vault is empty"));
    }
}
