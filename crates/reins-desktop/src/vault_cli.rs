//! `reins vault add NAME` and `reins vault list`: put a secret into the vault on the phone without handing it to an AI
//! (an account signed in with a passkey or SSO has no other way to add one), and see what the items are called.
//!
//! The value is read here without echo (or from stdin when piped), put in a box from this app's key to the phone's
//! own key, and sent to the phone, which shows "Save NAME in your vault?" and stores it encrypted with the vault key
//! once approved. The server relays the box without being able to open or replace it, and nothing here keeps the
//! value.
//!
//! The phone's key comes from the phone the first time (`vault_phone_key`, sealed to this app): the approval shows its
//! eight digits, the user types them here, and the key is kept (`phone-key.json`) only when they match the key that
//! arrived. The key never travels in the clear, so a server that answered with its own key could not know which digits
//! to aim for. A new phone, or the app reinstalled, has a new key: the phone then cannot open the box, and the key is
//! asked for and checked again.

use std::io::{IsTerminal as _, Read as _, Write as _};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use clap::{Subcommand, ValueEnum};
use reins_proto::desktop::{PHONE_KEY_CHANGED, PhoneKey, SecretToStore, VaultNames, decode_key, phone_key_fingerprint};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use zeroize::Zeroizing;

use crate::auth::Refusal;
use crate::config::{Config, Mode, Paths};
use crate::identity::Identity;
use crate::journal::{Entry, Journal, Kind};
use crate::phone::{Phone, awaiting, teller, unavailable};

pub const KEY_TOOL: &str = "vault_phone_key";
pub const STORE_TOOL: &str = "vault_secret_store";
pub const NAMES_TOOL: &str = "vault_names";
/// The largest value read (an RSA private key is a few kilobytes).
const MAX_VALUE: u64 = 32 * 1024;

#[derive(Clone, Debug, Subcommand)]
pub enum VaultCmd {
    /// Save a secret in the vault on your phone. It is typed here without echo (or piped in), sealed so that only
    /// your phone can open it, and saved once you approve on the phone. An item with that name gets the field
    /// changed; otherwise a new item is made.
    Add {
        /// The item's name: what `vault:NAME/password` refers to.
        name: String,
        /// What kind of item a new one is.
        #[arg(long, value_enum, default_value_t = ItemKind::ApiKey)]
        kind: ItemKind,
        /// The field to set (default: password; notes for a note; the private key for ssh). Any other name makes a
        /// custom field.
        #[arg(long)]
        field: Option<String>,
        /// The eight digits your phone showed for its key, when this terminal cannot ask (the first time only).
        #[arg(long, value_name = "DIGITS")]
        phone_key: Option<String>,
    },
    /// The names of the items in the vault on your phone (never a value).
    List,
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

/// Digits as typed ("4821 9930", "48219930") and as shown, compared without spaces.
fn same_digits(typed: &str, shown: &str) -> bool {
    let digits = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
    digits(typed) == digits(shown)
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

/// Asks the phone for its key and keeps it once the user has compared the digits.
async fn check_phone_key(phone: &Phone, paths: &Paths, given: Option<&str>) -> Result<String, String> {
    eprintln!(
        "First, your phone hands this computer its key: approve \"Let the computer save secrets\" on your phone."
    );
    let answer = phone
        .ask(KEY_TOOL, None, |_| json!({}), "No answer from your phone in time. Approve it, then run again.")
        .await
        .map_err(|r| r.message().to_owned())?;
    let key: PhoneKey = phone.open(&answer.data).map_err(|r| r.message().to_owned())?;
    if key.nonce != answer.nonce {
        return Err("The phone's answer is for another request; refused.".to_owned());
    }
    let digits = phone_key_fingerprint(&key.public_key).ok_or("The phone sent a malformed key; refused.")?;
    // Typed, not a yes: the user has to read the number on the phone.
    let typed = match given {
        Some(typed) => typed.to_owned(),
        None => ask_terminal(
            "Type the eight digits your phone showed for its key (also in the Reins app: Settings, Devices): ",
        )
        .ok_or(
            "Check your phone's key once: run reins vault add in a terminal, or pass --phone-key with the eight digits \
             your phone showed.",
        )?,
    };
    if !same_digits(&typed, &digits) {
        return Err(format!(
            "Not saved: this computer received a key with the digits {digits}, not the ones you typed. If your phone \
             shows {digits}, run reins vault add again and type them; if it does not, something between your phone \
             and this computer changed the key (check the server: reins status)."
        ));
    }
    pin(paths, phone.server(), &key.public_key)?;
    eprintln!("Your phone's key is kept on this computer; it is not asked for again.");
    Ok(key.public_key)
}

/// The value to save: piped in on stdin, or typed at the terminal without echo.
fn read_value(name: &str, field: &str, kind: ItemKind) -> Result<Zeroizing<String>, String> {
    let mut value = Zeroizing::new(String::new());
    if std::io::stdin().is_terminal() {
        if kind == ItemKind::Ssh {
            return Err("pipe the private key in: reins vault add NAME --kind ssh < ~/.ssh/id_ed25519".to_owned());
        }
        let typed = rpassword::prompt_password(format!("{field} for {name} (not shown, Enter to finish): "))
            .map_err(|e| format!("reading from the terminal: {e}"))?;
        value.push_str(&typed);
        drop(Zeroizing::new(typed));
    } else {
        std::io::stdin().take(MAX_VALUE + 1).read_to_string(&mut value).map_err(|e| format!("reading stdin: {e}"))?;
        if value.len() as u64 > MAX_VALUE {
            return Err(format!("the value is longer than {} KB", MAX_VALUE / 1024));
        }
        // What `echo` and editors add, not part of the value; a key keeps its own layout.
        if kind != ItemKind::Ssh {
            let kept = value.trim_end_matches(['\n', '\r']).len();
            value.truncate(kept);
        }
    }
    if value.trim().is_empty() {
        return Err("nothing to save: the value is empty".to_owned());
    }
    Ok(value)
}

/// What is sent: the value with the request's nonce, the name and the field, in a box from this app's key to the
/// phone's.
fn box_value(
    identity: &Identity,
    phone_key: &str,
    nonce: &str,
    name: &str,
    field: &str,
    value: &str,
) -> Option<String> {
    let plain = Zeroizing::new(
        serde_json::to_vec(&SecretToStore {
            v: 1,
            nonce: nonce.to_owned(),
            name: name.to_owned(),
            field: field.to_owned(),
            value: value.to_owned(),
        })
        .ok()?,
    );
    identity.box_to(phone_key, &plain)
}

/// Sends the boxed value; what the phone answered.
async fn store(
    phone: &Phone,
    phone_key: &str,
    name: &str,
    kind: ItemKind,
    field: &str,
    value: &Zeroizing<String>,
) -> Result<Value, Refusal> {
    let answer = phone
        .ask(
            STORE_TOOL,
            None,
            |nonce| {
                json!({"name": name, "kind": kind.arg(), "field": field,
                    "sealed": box_value(phone.identity(), phone_key, nonce, name, field, value).unwrap_or_default()})
            },
            "No answer from your phone in time. Approve it, then run again.",
        )
        .await?;
    Ok(answer.data)
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
    let identity = Arc::new(Identity::load_or_create(&paths.identity_file()).map_err(|e| e.to_string())?);
    Phone::new(paths, identity, Duration::from_secs(config.approval_timeout_secs))
}

async fn add(
    paths: &Paths,
    config: &Config,
    name: &str,
    kind: ItemKind,
    field: Option<&str>,
    given: Option<&str>,
) -> Result<Vec<String>, String> {
    let name = name.trim();
    if name.is_empty() || name.chars().any(char::is_control) {
        return Err("give the item a name: reins vault add NAME".to_owned());
    }
    let field = field.map_or(kind.default_field(), str::trim);
    let phone = logged_in(paths, config)?;
    let mut phone_key = match pinned(paths, phone.server()) {
        Some(key) => key,
        None => check_phone_key(&phone, paths, given).await?,
    };
    let value = read_value(name, field, kind)?;
    let journal = Journal::new(paths);
    let timeout = Duration::from_secs(config.approval_timeout_secs);
    let what = format!("save {name} in the vault");
    let mut retried = false;
    let data = loop {
        let entry = Entry::new(Kind::Secrets, &what).source(Some("reins vault add"));
        let tell = teller(|line: &str| eprintln!("{line}"));
        let sent = awaiting(&journal, entry, Some(tell), timeout, store(&phone, &phone_key, name, kind, field, &value));
        match sent.await {
            Ok(data) => break data,
            Err(Refusal::Unavailable(m)) if m.starts_with(PHONE_KEY_CHANGED) && !retried => {
                retried = true;
                unpin(paths);
                eprintln!("Your phone has a new key (a new phone, or the app was installed again).");
                phone_key = check_phone_key(&phone, paths, None).await?;
            }
            Err(r) => return Err(r.message().to_owned()),
        }
    };
    drop(value);
    let created = data.get("created").and_then(Value::as_bool).unwrap_or(false);
    let shown_field = if field == "private_key" {
        "private key"
    } else {
        field
    };
    let mut lines = vec![if created {
        format!("Saved {name} in your vault.")
    } else {
        format!("Changed the {shown_field} of {name} in your vault.")
    }];
    lines.push(if kind == ItemKind::Ssh {
        "The SSH agent offers it (reins ssh setup); signing happens on your phone.".to_owned()
    } else {
        format!("Use it as vault:{name}/{field}, for example: reins run --env API_KEY=vault:{name}/{field} -- ...")
    });
    Ok(lines)
}

async fn list(paths: &Paths, config: &Config) -> Result<Vec<String>, String> {
    let phone = logged_in(paths, config)?;
    let journal = Journal::new(paths);
    let entry = Entry::new(Kind::Secrets, "the names of the vault items").source(Some("reins vault list"));
    let tell = teller(|line: &str| eprintln!("{line}"));
    let asked =
        phone.ask(NAMES_TOOL, None, |_| json!({}), "No answer from your phone in time. Approve it, then run again.");
    let timeout = Duration::from_secs(config.approval_timeout_secs);
    let answer = awaiting(&journal, entry, Some(tell), timeout, asked).await.map_err(|r| r.message().to_owned())?;
    let item = answer.data.get("items").and_then(|i| i.get(0)).ok_or("The phone's answer is empty.")?;
    let names: VaultNames = phone.open(item).map_err(|r| r.message().to_owned())?;
    if names.nonce != answer.nonce {
        return Err(unavailable("The phone's answer is for another request; refused.").message().to_owned());
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
            phone_key,
        } => add(paths, config, name, *kind, field.as_deref(), phone_key.as_deref()).await,
        VaultCmd::List => list(paths, config).await,
    }
}

#[cfg(test)]
mod tests {
    use crypto_box::aead::OsRng;
    use data_encoding::BASE64URL_NOPAD;

    use super::*;

    #[test]
    fn digits_are_compared_without_spaces() {
        assert!(same_digits("48219930", "4821 9930"));
        assert!(same_digits(" 4821 9930 ", "4821 9930"));
        assert!(!same_digits("4821 9931", "4821 9930"));
        assert!(!same_digits("", "4821 9930"));
    }

    #[test]
    fn the_value_opens_with_the_phone_key_and_proves_this_app_sent_it() {
        use crypto_box::aead::Aead as _;
        let app = Identity::generate();
        let phone = crypto_box::SecretKey::generate(&mut OsRng);
        let public = reins_proto::desktop::encode_key(phone.public_key().as_bytes());
        let boxed = box_value(&app, &public, "n1", "OpenAI", "password", "sk-123").unwrap();
        assert!(!boxed.contains("sk-123"));
        let bytes = BASE64URL_NOPAD.decode(boxed.as_bytes()).unwrap();
        let (nonce, ciphertext) = bytes.split_at(24);
        let nonce = crypto_box::Nonce::from(<[u8; 24]>::try_from(nonce).unwrap());
        let app_key = crypto_box::PublicKey::from(decode_key(&app.public_key()).unwrap());
        let plain = crypto_box::SalsaBox::new(&app_key, &phone).decrypt(&nonce, ciphertext).unwrap();
        let opened: SecretToStore = serde_json::from_slice(&plain).unwrap();
        assert_eq!((opened.nonce.as_str(), opened.name.as_str(), opened.value.as_str()), ("n1", "OpenAI", "sk-123"));
        let other_app = crypto_box::PublicKey::from(decode_key(&Identity::generate().public_key()).unwrap());
        assert!(crypto_box::SalsaBox::new(&other_app, &phone).decrypt(&nonce, ciphertext).is_err());
        assert!(box_value(&app, "short", "n1", "x", "y", "z").is_none());
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
