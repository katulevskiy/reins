//! The executor's configuration file (TOML). Secrets never sit in it: they are files or environment variables it names
//! ([`SecretSource`]).
//!
//! ```toml
//! server = "https://reins.example.com"
//! org = "org_…"
//! executor_id = "ex-1"
//! mode = "cloud"                 # cloud | dedicated | self_hosted
//! key_file = "/var/lib/reins-executor/executor.key"
//! state_dir = "/var/lib/reins-executor"
//!
//! [pins]                         # written by `reins-executor pin --accept`
//! issuer = "iss-…=<base64url>"
//! decision = "dec-…=<base64url>"
//!
//! [github]
//! app_id = 12345
//! installation_id = 67890
//! private_key = { file = "/etc/reins-executor/github-app.pem" }
//!
//! [google]
//! service_account = { env = "REINS_GOOGLE_SERVICE_ACCOUNT" }
//! ```

use std::path::{Path, PathBuf};

use reins_audit::TrustedKey;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

/// The longest a signed roster is trusted after it was issued (seconds).
pub const MAX_ROSTER_AGE_SECS: i64 = 900;

/// Where an executor runs (05 §5). The same binary in every mode; nothing else differs in code in O2.
///
/// Kept here rather than in `reins-proto` so the executor stands alone; the server has its own copy of the names.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutorMode {
    /// Run by Reins, one per tenant (a `reins-executor@<org>.service` unit with its own state and key).
    #[default]
    Cloud,
    /// Run by Reins on hardware dedicated to the org.
    Dedicated,
    /// Run by the org in its own network.
    SelfHosted,
}

/// A secret, named by where it is read from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretSource {
    /// A file's contents.
    File(PathBuf),
    /// An environment variable's value.
    Env(String),
}

impl SecretSource {
    pub fn read(&self) -> Result<Zeroizing<String>, String> {
        match self {
            Self::File(p) => std::fs::read_to_string(p)
                .map(Zeroizing::new)
                .map_err(|e| format!("reading the secret file {}: {e}", p.display())),
            Self::Env(v) => {
                std::env::var(v).map(Zeroizing::new).map_err(|_| format!("the environment variable {v} is not set"))
            }
        }
    }
}

/// The org's keys the executor trusts, as `<key id>=<base64url>` ([`TrustedKey::parse`]). Empty until pinned.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PinsConfig {
    #[serde(default)]
    pub issuer: Option<String>,
    #[serde(default)]
    pub decision: Option<String>,
}

/// The org's GitHub App installation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubConfig {
    pub app_id: u64,
    pub installation_id: u64,
    /// The App's private key (PEM, PKCS#1 as GitHub hands it out, or PKCS#8).
    pub private_key: SecretSource,
    #[serde(default = "github_api")]
    pub api_base: String,
}

fn github_api() -> String {
    "https://api.github.com".to_owned()
}

/// The org's Google Workspace service account (domain-wide delegation).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoogleConfig {
    /// The service account's JSON key (`client_email`, `private_key`, `token_uri`).
    pub service_account: SecretSource,
    #[serde(default = "google_api")]
    pub api_base: String,
    /// Overrides the key's `token_uri`.
    #[serde(default)]
    pub token_uri: Option<String>,
}

fn google_api() -> String {
    "https://www.googleapis.com".to_owned()
}

const fn max_roster_age() -> i64 {
    MAX_ROSTER_AGE_SECS
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutorConfig {
    /// The Reins server's origin (`https://…`).
    pub server: String,
    pub org: String,
    pub executor_id: String,
    #[serde(default)]
    pub mode: ExecutorMode,
    /// The executor's Ed25519 key (`reins-executor keygen`).
    pub key_file: PathBuf,
    /// Leases and the idempotency log.
    pub state_dir: PathBuf,
    #[serde(default)]
    pub pins: PinsConfig,
    #[serde(default)]
    pub github: Option<GithubConfig>,
    #[serde(default)]
    pub google: Option<GoogleConfig>,
    #[serde(default = "max_roster_age")]
    pub max_roster_age_secs: i64,
}

impl ExecutorConfig {
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("reading {}: {e}", path.display()))?;
        Self::parse(&text)
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        let cfg: Self = toml::from_str(text).map_err(|e| format!("the configuration is not valid: {e}"))?;
        if cfg.org.is_empty() || cfg.executor_id.is_empty() {
            return Err("`org` and `executor_id` are required".to_owned());
        }
        if cfg.max_roster_age_secs <= 0 {
            return Err("`max_roster_age_secs` must be positive".to_owned());
        }
        Ok(cfg)
    }

    /// The pinned keys; an error until `reins-executor pin --accept` has run.
    pub fn pins(&self) -> Result<Pins, String> {
        let parse = |what: &str, v: &Option<String>| {
            v.as_deref()
                .and_then(TrustedKey::parse)
                .ok_or_else(|| format!("no valid {what} key is pinned: run `reins-executor pin --config … --accept`"))
        };
        Ok(Pins {
            org: self.org.clone(),
            issuer: parse("issuer", &self.pins.issuer)?,
            decision: parse("decision", &self.pins.decision)?,
        })
    }
}

/// The org and the keys an executor trusts for it: the issuer (policies, rosters) and the decision service
/// (countersigned decisions). Pinned once; never taken from the server after that.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pins {
    pub org: String,
    pub issuer: TrustedKey,
    pub decision: TrustedKey,
}

/// `<key id>=<base64url>`, as the pins are written.
#[must_use]
pub fn pin_text(key: &TrustedKey) -> String {
    format!("{}={}", key.key_id, data_encoding::BASE64URL_NOPAD.encode(&key.public))
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = r#"
server = "https://reins.example"
org = "o1"
executor_id = "ex-1"
mode = "self_hosted"
key_file = "/k"
state_dir = "/s"
"#;

    #[test]
    fn a_config_loads_with_its_defaults() {
        let cfg = ExecutorConfig::parse(BASE).unwrap();
        assert_eq!(cfg.mode, ExecutorMode::SelfHosted);
        assert_eq!(cfg.max_roster_age_secs, MAX_ROSTER_AGE_SECS);
        assert!(cfg.github.is_none());
        assert!(cfg.pins().unwrap_err().contains("pin"));
    }

    #[test]
    fn secrets_are_named_never_inlined() {
        let key = TrustedKey {
            key_id: "iss-1".into(),
            public: [3; 32],
        };
        let text = format!(
            "{BASE}\n[pins]\nissuer = \"{}\"\ndecision = \"{}\"\n\n[github]\napp_id = 1\ninstallation_id = 2\n\
             private_key = {{ file = \"/etc/app.pem\" }}\n\n[google]\nservice_account = {{ env = \"SA\" }}\n",
            pin_text(&key),
            pin_text(&key),
        );
        let cfg = ExecutorConfig::parse(&text).unwrap();
        let gh = cfg.github.as_ref().unwrap();
        assert_eq!(gh.private_key, SecretSource::File("/etc/app.pem".into()));
        assert_eq!(gh.api_base, "https://api.github.com");
        assert_eq!(cfg.google.as_ref().unwrap().service_account, SecretSource::Env("SA".into()));
        assert_eq!(cfg.pins().unwrap().issuer, key);
        assert!(ExecutorConfig::parse(&BASE.replace("self_hosted", "on_a_boat")).is_err());
    }
}
