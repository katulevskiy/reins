//! What a value is: compiled-in rules in the style of gitleaks (a known prefix and shape: OpenAI, Anthropic, Stripe,
//! GitHub, Slack, AWS, Google, npm, PyPI, crates.io, private keys, database URLs with a password, JWTs), then the
//! variable's name and the value's randomness for the rest. Each answer has a confidence, so the review pre-ticks real
//! secrets (`High`, `Medium`) and leaves `LOG_LEVEL=debug` alone.
//!
//! The guards against false positives: placeholders (`changeme`, `your-api-key`, `${VAR}`, `<token>`, `xxxx`), values
//! that are words, numbers, booleans, paths, versions or plain URLs, names that are known settings (`PORT`,
//! `NODE_ENV`), and names that front-end tools publish (`NEXT_PUBLIC_*`), which only a known rule flags.

use std::sync::LazyLock;

use data_encoding::BASE64URL_NOPAD;
use regex::Regex;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

/// How sure the scanner is that a value is a secret.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    /// Not a secret (a setting, a placeholder).
    None,
    /// Could be one; not pre-ticked.
    Low,
    /// Very likely one: pre-ticked.
    Medium,
    /// A known kind of secret: pre-ticked.
    High,
}

impl Confidence {
    /// Whether the review ticks it before the user does.
    #[must_use]
    pub fn suggested(self) -> bool {
        self >= Self::Medium
    }
}

/// The kind of secret a rule recognized.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Detector {
    #[serde(rename = "openai")]
    OpenAi,
    Anthropic,
    StripeLive,
    StripeTest,
    GithubPat,
    GithubFineGrained,
    GithubApp,
    GithubOauth,
    Slack,
    SlackWebhook,
    AwsAccessKeyId,
    AwsSecretKey,
    GoogleApiKey,
    SendGrid,
    HuggingFace,
    Npm,
    #[serde(rename = "pypi")]
    PyPi,
    CratesIo,
    PrivateKey,
    Postgres,
    #[serde(rename = "mysql")]
    MySql,
    #[serde(rename = "mongodb")]
    MongoDb,
    Redis,
    SupabaseService,
    Jwt,
    /// A field the file's format says holds a credential (`_authToken`, `oauth_token`, a netrc password).
    Credential,
    /// A name that says "password".
    Password,
    /// A secret-looking name and a random-looking value, or only a very random value.
    Generic,
}

impl Detector {
    /// What the review calls it.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::OpenAi => "OpenAI key",
            Self::Anthropic => "Anthropic key",
            Self::StripeLive => "Stripe live key",
            Self::StripeTest => "Stripe test key",
            Self::GithubPat => "GitHub token",
            Self::GithubFineGrained => "GitHub fine-grained token",
            Self::GithubApp => "GitHub app token",
            Self::GithubOauth => "GitHub OAuth token",
            Self::Slack => "Slack token",
            Self::SlackWebhook => "Slack webhook",
            Self::AwsAccessKeyId => "AWS access key ID",
            Self::AwsSecretKey => "AWS secret key",
            Self::GoogleApiKey => "Google API key",
            Self::SendGrid => "SendGrid key",
            Self::HuggingFace => "Hugging Face token",
            Self::Npm => "npm token",
            Self::PyPi => "PyPI token",
            Self::CratesIo => "crates.io token",
            Self::PrivateKey => "Private key",
            Self::Postgres => "Postgres password",
            Self::MySql => "MySQL password",
            Self::MongoDb => "MongoDB password",
            Self::Redis => "Redis password",
            Self::SupabaseService => "Supabase service key",
            Self::Jwt => "JWT",
            Self::Credential => "Token",
            Self::Password => "Password",
            Self::Generic => "Secret",
        }
    }
}

/// A rule: the whole value matches `pattern` (`find`: somewhere in it).
struct Rule {
    detector: Detector,
    confidence: Confidence,
    pattern: &'static str,
    find: bool,
}

const RULES: &[Rule] = &[
    rule(Detector::Anthropic, r"sk-ant-(?:api|admin|sid)[0-9]{2}-[A-Za-z0-9_\-]{32,}"),
    rule(Detector::OpenAi, r"sk-(?:proj|svcacct|admin|None)-[A-Za-z0-9_\-]{20,}"),
    rule(Detector::OpenAi, r"sk-[A-Za-z0-9]{20}T3BlbkFJ[A-Za-z0-9]{20}"),
    rule(Detector::OpenAi, r"sk-[A-Za-z0-9]{48}"),
    rule(Detector::StripeLive, r"(?:sk|rk)_live_[A-Za-z0-9]{20,}"),
    Rule {
        detector: Detector::StripeTest,
        confidence: Confidence::Medium,
        pattern: r"(?:sk|rk)_test_[A-Za-z0-9]{20,}",
        find: false,
    },
    rule(Detector::GithubPat, r"ghp_[A-Za-z0-9]{36}"),
    rule(Detector::GithubFineGrained, r"github_pat_[A-Za-z0-9_]{50,}"),
    rule(Detector::GithubApp, r"gh[su]_[A-Za-z0-9]{36}"),
    rule(Detector::GithubOauth, r"gh[or]_[A-Za-z0-9]{36}"),
    rule(Detector::Slack, r"xox[abposr]-[A-Za-z0-9\-]{10,}"),
    rule(Detector::SlackWebhook, r"https://hooks\.slack\.com/services/T[A-Za-z0-9_]+/B[A-Za-z0-9_]+/[A-Za-z0-9_]{16,}"),
    rule(Detector::AwsAccessKeyId, r"(?:AKIA|ASIA|ABIA|ACCA)[A-Z0-9]{16}"),
    rule(Detector::GoogleApiKey, r"AIza[0-9A-Za-z_\-]{35}"),
    rule(Detector::SendGrid, r"SG\.[A-Za-z0-9_\-]{22}\.[A-Za-z0-9_\-]{43}"),
    rule(Detector::HuggingFace, r"hf_[A-Za-z0-9]{34,}"),
    rule(Detector::Npm, r"npm_[A-Za-z0-9]{36}"),
    rule(Detector::PyPi, r"pypi-AgEIcHlwaS5vcmc[A-Za-z0-9_\-]{50,}"),
    rule(Detector::CratesIo, r"cio[A-Za-z0-9]{32}"),
    Rule {
        detector: Detector::PrivateKey,
        confidence: Confidence::High,
        pattern: r"-----BEGIN (?:RSA |EC |DSA |OPENSSH |ENCRYPTED |PGP )?PRIVATE KEY(?: BLOCK)?-----",
        find: true,
    },
];

const fn rule(detector: Detector, pattern: &'static str) -> Rule {
    Rule {
        detector,
        confidence: Confidence::High,
        pattern,
        find: false,
    }
}

static COMPILED: LazyLock<Vec<(Regex, &'static Rule)>> = LazyLock::new(|| {
    RULES
        .iter()
        .map(|r| {
            let anchored = if r.find {
                r.pattern.to_owned()
            } else {
                format!("^(?:{})$", r.pattern)
            };
            (Regex::new(&anchored).expect("the built-in rules compile"), r)
        })
        .collect()
});

/// `scheme://user:password@host`: a database URL with a password.
static DB_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^(postgres|postgresql|mysql|mariadb|mongodb|mongodb\+srv|redis|rediss)://([^:/@\s]*):([^@\s]+)@[^\s]+$",
    )
    .expect("compiles")
});

static JWT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^eyJ[A-Za-z0-9_\-]{8,}\.eyJ[A-Za-z0-9_\-]{8,}\.[A-Za-z0-9_\-]{8,}$").expect("compiles")
});

static AWS_SECRET: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z0-9/+]{40}$").expect("compiles"));

/// What a value is, as far as the rules can tell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Verdict {
    pub detector: Option<Detector>,
    pub confidence: Confidence,
}

impl Verdict {
    const NONE: Self = Self {
        detector: None,
        confidence: Confidence::None,
    };

    fn of(detector: Detector, confidence: Confidence) -> Self {
        Self {
            detector: Some(detector),
            confidence,
        }
    }

    /// The review's words for it ("OpenAI key", "not a secret").
    #[must_use]
    pub fn label(&self) -> &'static str {
        match (self.detector, self.confidence) {
            (_, Confidence::None) => "not a secret",
            (Some(d), _) => d.label(),
            (None, _) => "maybe a secret",
        }
    }
}

/// Words in a name that say it holds a secret.
const SECRET_WORDS: &[&str] = &[
    "SECRET",
    "TOKEN",
    "KEY",
    "PASSWORD",
    "PASSWD",
    "PASS",
    "PWD",
    "CREDENTIAL",
    "CREDENTIALS",
    "AUTH",
    "PRIVATE",
    "APIKEY",
    "DSN",
    "SALT",
    "SIGNATURE",
    "CERT",
    "WEBHOOK",
    "SESSION",
    "COOKIE",
];

/// Names that hold settings, never secrets.
const SETTING_NAMES: &[&str] = &[
    "NODE_ENV",
    "RAILS_ENV",
    "RACK_ENV",
    "APP_ENV",
    "ENV",
    "ENVIRONMENT",
    "PORT",
    "HOST",
    "HOSTNAME",
    "LOG_LEVEL",
    "LOGLEVEL",
    "DEBUG",
    "TZ",
    "LANG",
    "LC_ALL",
    "PATH",
    "HOME",
    "EDITOR",
    "SHELL",
    "TERM",
    "PUBLIC_KEY",
    "KEY_ID",
    "SSH_AUTH_SOCK",
    "GPG_TTY",
    "KEYBOARD",
    "KEYMAP",
];

/// Prefixes of variables front-end build tools put into the published bundle: public by design.
const PUBLIC_PREFIXES: &[&str] =
    &["NEXT_PUBLIC_", "VITE_", "REACT_APP_", "EXPO_PUBLIC_", "PUBLIC_", "GATSBY_", "NUXT_PUBLIC_"];

/// Words of placeholders people leave in files (a whole word of the value: `your-api-key`, `changeme`).
const PLACEHOLDER_WORDS: &[&str] = &[
    "changeme",
    "replaceme",
    "placeholder",
    "example",
    "your",
    "yourkey",
    "todo",
    "fixme",
    "dummy",
    "redacted",
    "insert",
    "null",
    "none",
    "nil",
    "undefined",
    "password",
];

/// Placeholders made of several words, or of no letters.
const PLACEHOLDER_PARTS: &[&str] = &["change_me", "change-me", "replace_me", "replace-me", "****", "...", "•"];

fn upper(name: &str) -> String {
    name.trim().to_ascii_uppercase().replace(['-', '.'], "_")
}

/// The words of a name (`STRIPE_SECRET_KEY` → STRIPE, SECRET, KEY; `apiKey` → API, KEY).
fn name_words(name: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut prev_lower = false;
    for c in name.chars() {
        if !c.is_ascii_alphanumeric() {
            if !word.is_empty() {
                words.push(std::mem::take(&mut word));
            }
            prev_lower = false;
            continue;
        }
        if c.is_ascii_uppercase() && prev_lower && !word.is_empty() {
            words.push(std::mem::take(&mut word));
        }
        prev_lower = c.is_ascii_lowercase();
        word.push(c.to_ascii_uppercase());
    }
    if !word.is_empty() {
        words.push(word);
    }
    words
}

/// Whether a variable's name says it holds a secret.
#[must_use]
pub fn secret_name(name: &str) -> bool {
    let up = upper(name);
    if SETTING_NAMES.contains(&up.as_str()) {
        return false;
    }
    let words = name_words(name);
    words.iter().any(|w| SECRET_WORDS.contains(&w.as_str()))
        || ["APIKEY", "SECRETKEY", "ACCESSKEY", "AUTHTOKEN", "PASSWORD", "PASSWD"].iter().any(|w| up.contains(w))
}

fn password_name(name: &str) -> bool {
    let words = name_words(name);
    words.iter().any(|w| matches!(w.as_str(), "PASSWORD" | "PASSWD" | "PASS" | "PWD"))
        || upper(name).contains("PASSWORD")
}

fn public_name(name: &str) -> bool {
    let up = upper(name);
    PUBLIC_PREFIXES.iter().any(|p| up.starts_with(p))
}

/// Whether a value is a placeholder or a reference to something else, not a value.
#[must_use]
pub fn placeholder(value: &str) -> bool {
    let v = value.trim();
    if v.is_empty() {
        return true;
    }
    let lower = Zeroizing::new(v.to_ascii_lowercase());
    // References to another variable or a command's output, and Reins' own references.
    if v.starts_with('$') || v.contains("${") || v.contains("$(") || v.contains('`') {
        return true;
    }
    if ["vault:", "env:", "op://", "infisical:", "doppler:", "sm://"].iter().any(|p| lower.starts_with(p)) {
        return true;
    }
    if (v.contains('<') && v.contains('>')) || PLACEHOLDER_PARTS.iter().any(|p| lower.contains(p)) {
        return true;
    }
    // A word of a placeholder, or a word that is one character over and over (`sk-xxxxxxxx`, `0000`).
    lower
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .any(|w| PLACEHOLDER_WORDS.contains(&w) || (w.len() >= 4 && w.bytes().all(|b| b == w.as_bytes()[0])))
}

/// Shannon entropy in bits per character.
#[must_use]
#[allow(clippy::cast_precision_loss, reason = "counts of a short string")]
pub fn entropy(value: &str) -> f64 {
    let mut counts = [0u32; 256];
    let mut n = 0u32;
    for b in value.bytes() {
        counts[usize::from(b)] += 1;
        n += 1;
    }
    if n == 0 {
        return 0.0;
    }
    let n = f64::from(n);
    counts.iter().filter(|&&c| c > 0).map(|&c| f64::from(c) / n).map(|p| -p * p.log2()).sum()
}

/// Whether a value looks made by a machine: long enough, several kinds of characters, random.
fn random_looking(value: &str, min_len: usize, min_entropy: f64) -> bool {
    let v = value.trim();
    if v.len() < min_len || v.len() > 8_192 || v.contains(char::is_whitespace) {
        return false;
    }
    let classes = [
        v.bytes().any(|b| b.is_ascii_lowercase()),
        v.bytes().any(|b| b.is_ascii_uppercase()),
        v.bytes().any(|b| b.is_ascii_digit()),
        v.bytes().any(|b| !b.is_ascii_alphanumeric()),
    ]
    .iter()
    .filter(|&&c| c)
    .count();
    classes >= 2 && entropy(v) >= min_entropy
}

/// Values that are plainly not secrets: numbers, booleans, versions, colours, paths, URLs without credentials, words.
fn plain_value(value: &str) -> bool {
    let v = value.trim();
    let lower = v.to_ascii_lowercase();
    if matches!(lower.as_str(), "true" | "false" | "yes" | "no" | "on" | "off" | "1" | "0") {
        return true;
    }
    if v.bytes().all(|b| b.is_ascii_digit() || b == b'.' || b == b'-' || b == b'_' || b == b':') {
        return true;
    }
    if v.starts_with('#') && v.len() <= 9 && v[1..].bytes().all(|b| b.is_ascii_hexdigit()) {
        return true;
    }
    if v.starts_with('/') || v.starts_with("./") || v.starts_with("../") || v.starts_with("~/") {
        return true;
    }
    if let Some((scheme, rest)) = v.split_once("://")
        && scheme.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'+')
        && !rest.split('/').next().unwrap_or("").contains('@')
        && !rest.contains('?')
    {
        return true;
    }
    // Words, with spaces or dashes (`debug`, `us-east-1`, `Acme Web`).
    v.bytes().all(|b| b.is_ascii_lowercase() || b == b'-' || b == b'_' || b == b' ' || b == b'.' || b.is_ascii_digit())
        && v.bytes().filter(u8::is_ascii_digit).count() * 3 < v.len()
}

/// The role in a JWT's payload (`service_role`, `anon`), if it says.
fn jwt_role(value: &str) -> Option<String> {
    let payload = value.split('.').nth(1)?;
    let bytes = Zeroizing::new(BASE64URL_NOPAD.decode(payload.trim_end_matches('=').as_bytes()).ok()?);
    let claims: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    claims.get("role").and_then(serde_json::Value::as_str).map(str::to_owned)
}

/// What `value` of the variable `name` is. `credential`: the file's format says the field holds a credential.
#[must_use]
pub fn classify(name: &str, value: &str, credential: bool) -> Verdict {
    let v = value.trim();
    if placeholder(v) {
        return Verdict::NONE;
    }
    for (re, r) in COMPILED.iter() {
        if re.is_match(v) {
            return Verdict::of(r.detector, r.confidence);
        }
    }
    if let Some(c) = DB_URL.captures(v) {
        let password = c.get(3).map_or("", |m| m.as_str());
        if placeholder(password) || password.len() < 3 {
            return Verdict::NONE;
        }
        let detector = match c.get(1).map_or("", |m| m.as_str()) {
            "postgres" | "postgresql" => Detector::Postgres,
            "mysql" | "mariadb" => Detector::MySql,
            "mongodb" | "mongodb+srv" => Detector::MongoDb,
            _ => Detector::Redis,
        };
        return Verdict::of(detector, Confidence::High);
    }
    let up = upper(name);
    if JWT.is_match(v) {
        return match jwt_role(v).as_deref() {
            Some("service_role") => Verdict::of(Detector::SupabaseService, Confidence::High),
            // Supabase's anon key is published in every client.
            Some("anon") => Verdict::of(Detector::Jwt, Confidence::Low),
            _ if secret_name(name) => Verdict::of(Detector::Jwt, Confidence::High),
            _ => Verdict::of(Detector::Jwt, Confidence::Medium),
        };
    }
    if (up.contains("AWS") || up.contains("SECRET_ACCESS")) && up.contains("SECRET") && AWS_SECRET.is_match(v) {
        return Verdict::of(Detector::AwsSecretKey, Confidence::High);
    }
    if credential {
        return Verdict::of(Detector::Credential, Confidence::High);
    }
    if public_name(name) {
        return Verdict::NONE;
    }
    if SETTING_NAMES.contains(&up.as_str()) {
        return Verdict::NONE;
    }
    if password_name(name) && v.len() >= 4 && !plain_value(v) {
        return Verdict::of(Detector::Password, Confidence::Medium);
    }
    if secret_name(name) && !plain_value(v) && random_looking(v, 12, 3.0) {
        return Verdict::of(Detector::Generic, Confidence::Medium);
    }
    if !plain_value(v) && random_looking(v, 24, 4.2) {
        return Verdict::of(Detector::Generic, Confidence::Low);
    }
    Verdict::NONE
}

/// The bullets that stand for the hidden part of a value (always the same number: the length is not shown).
pub const MASK: &str = "••••••••";

/// A value as the review may show it: at most its first 4 and last 4 characters, and never more than a quarter of it,
/// the rest as [`MASK`]. Line breaks and control characters are never shown.
#[must_use]
pub fn mask(value: &str) -> String {
    let chars: Vec<char> = value.trim().chars().collect();
    let shown = (chars.len() / 4).min(8);
    let head = shown.div_ceil(2).min(4);
    let tail = shown - head;
    let visible = |c: &char| {
        if c.is_control() {
            '·'
        } else {
            *c
        }
    };
    let mut out: String = chars[..head].iter().map(visible).collect();
    out.push_str(MASK);
    out.extend(chars[chars.len() - tail..].iter().map(visible));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::secrets_scan::tests::fake;

    fn kind(name: &str, value: &str) -> (Option<Detector>, Confidence) {
        let v = classify(name, value, false);
        (v.detector, v.confidence)
    }

    #[test]
    fn known_kinds_are_recognized_whatever_the_name() {
        let cases = [
            (fake("sk-proj-", 48), Detector::OpenAi),
            (fake("sk-ant-api03-", 90), Detector::Anthropic),
            (fake(&["sk", "_live_"].concat(), 24), Detector::StripeLive),
            (fake(&["gh", "p_"].concat(), 36), Detector::GithubPat),
            (fake("github_pat_", 82), Detector::GithubFineGrained),
            (fake(&["gh", "s_"].concat(), 36), Detector::GithubApp),
            (fake(&["gh", "o_"].concat(), 36), Detector::GithubOauth),
            (fake(&["xo", "xb-"].concat(), 40), Detector::Slack),
            (fake(&["AK", "IA"].concat(), 16).to_ascii_uppercase(), Detector::AwsAccessKeyId),
            (fake(&["AI", "za"].concat(), 35), Detector::GoogleApiKey),
            (fake("npm_", 36), Detector::Npm),
            (fake("pypi-AgEIcHlwaS5vcmc", 60), Detector::PyPi),
            (fake("cio", 32), Detector::CratesIo),
            (fake("hf_", 34), Detector::HuggingFace),
        ];
        for (value, want) in cases {
            assert_eq!(kind("X", &value), (Some(want), Confidence::High), "{want:?}");
        }
        assert_eq!(kind("X", &fake(&["sk", "_test_"].concat(), 24)), (Some(Detector::StripeTest), Confidence::Medium));
        let pem =
            format!("-----BEGIN OPENSSH PRIVATE KEY-----\n{}\n-----END OPENSSH PRIVATE KEY-----", fake("b3Bl", 60));
        assert_eq!(kind("DEPLOY_KEY", &pem).0, Some(Detector::PrivateKey));
    }

    #[test]
    fn database_urls_count_only_with_a_real_password() {
        let pw = fake("", 16);
        assert_eq!(kind("DATABASE_URL", &format!("postgres://app:{pw}@db.acme.dev/app")).0, Some(Detector::Postgres));
        assert_eq!(kind("X", &format!("mongodb+srv://u:{pw}@c.mongodb.net")).0, Some(Detector::MongoDb));
        assert_eq!(kind("X", &format!("rediss://:{pw}@cache:6379")).0, Some(Detector::Redis));
        assert_eq!(kind("X", &format!("mysql://root:{pw}@localhost/db")).0, Some(Detector::MySql));
        assert_eq!(kind("DATABASE_URL", "postgres://localhost/app").1, Confidence::None);
        assert_eq!(kind("DATABASE_URL", "postgres://app:${DB_PASSWORD}@db/app").1, Confidence::None);
        assert_eq!(kind("DATABASE_URL", "postgres://app:password@localhost/app").1, Confidence::None);
    }

    #[test]
    fn jwts_by_their_role_and_name() {
        let part = |json: &str| BASE64URL_NOPAD.encode(json.as_bytes());
        let jwt = |role: &str| {
            format!(
                "{}.{}.{}",
                part(r#"{"alg":"HS256","typ":"JWT"}"#),
                part(&format!(r#"{{"iss":"supabase","role":"{role}"}}"#)),
                fake("", 43)
            )
        };
        assert_eq!(
            kind("SUPABASE_SERVICE_KEY", &jwt("service_role")),
            (Some(Detector::SupabaseService), Confidence::High)
        );
        assert_eq!(kind("SUPABASE_ANON_KEY", &jwt("anon")).1, Confidence::Low);
        assert_eq!(kind("SESSION_TOKEN", &jwt("user")), (Some(Detector::Jwt), Confidence::High));
        assert_eq!(kind("X", &jwt("user")), (Some(Detector::Jwt), Confidence::Medium));
    }

    #[test]
    fn settings_and_placeholders_are_not_secrets() {
        for (name, value) in [
            ("LOG_LEVEL", "debug"),
            ("NODE_ENV", "production"),
            ("PORT", "3000"),
            ("NEXT_PUBLIC_SITE_URL", "https://acme.dev"),
            ("API_URL", "https://api.acme.dev/v1"),
            ("FEATURE_FLAG", "true"),
            ("OPENAI_API_KEY", "your-api-key-here"),
            ("OPENAI_API_KEY", "sk-xxxxxxxxxxxxxxxxxxxx"),
            ("STRIPE_SECRET_KEY", "${STRIPE_SECRET_KEY}"),
            ("SECRET", "changeme"),
            ("API_KEY", "<your key>"),
            ("TOKEN", "vault:acme-web/development/TOKEN"),
            ("DB_PASSWORD", ""),
            ("REGION", "us-east-1"),
            ("APP_NAME", "Acme Web"),
            ("VERSION", "1.2.3"),
            ("THEME_COLOR", "#ff8800"),
            ("CACHE_DIR", "/var/cache/acme"),
            ("PUBLIC_KEY", &fake("", 40)),
            ("NEXT_PUBLIC_ANALYTICS_ID", &fake("", 30)),
        ] {
            assert_eq!(kind(name, value).1, Confidence::None, "{name}={value}");
        }
    }

    #[test]
    fn names_and_randomness_decide_the_rest() {
        assert_eq!(kind("DB_PASSWORD", "hunter2!x"), (Some(Detector::Password), Confidence::Medium));
        assert_eq!(kind("WEBHOOK_SECRET", &fake("whsec_", 32)), (Some(Detector::Generic), Confidence::Medium));
        assert_eq!(kind("apiKey", &fake("", 32)), (Some(Detector::Generic), Confidence::Medium));
        // Very random but nothing in the name: shown, not pre-ticked.
        assert_eq!(kind("BUILD_ID", &fake("", 40)), (Some(Detector::Generic), Confidence::Low));
        // A short or plain value under a secret name is not enough.
        assert_eq!(kind("SECRET_NAME", "billing").1, Confidence::None);
        // AWS's secret key needs its name (40 base64 characters alone could be anything).
        let aws = fake("", 40);
        assert_eq!(kind("AWS_SECRET_ACCESS_KEY", &aws), (Some(Detector::AwsSecretKey), Confidence::High));
        // A field the format calls a credential is one.
        assert_eq!(classify("_authToken", &fake("", 20), true).detector, Some(Detector::Credential));
        assert_eq!(classify("_authToken", "${NPM_TOKEN}", true).confidence, Confidence::None);
    }

    #[test]
    fn names_split_into_words() {
        assert_eq!(name_words("STRIPE_SECRET_KEY"), ["STRIPE", "SECRET", "KEY"]);
        assert_eq!(name_words("apiKey"), ["API", "KEY"]);
        assert!(secret_name("githubToken") && secret_name("DB_PASS") && !secret_name("KEYBOARD"));
        assert!(!secret_name("MONKEY") && !secret_name("PORT"));
    }

    #[test]
    fn masks_show_at_most_four_and_four_and_a_quarter() {
        for len in 0..300 {
            let value = fake("", len);
            let masked = mask(&value);
            let shown: String = masked.replace(MASK, "");
            assert!(shown.chars().count() <= 8, "{len}: {masked}");
            assert!(shown.chars().count() * 4 <= len.max(1) || shown.is_empty(), "{len}: {masked}");
            let (head, tail) = masked.split_once(MASK).unwrap();
            assert!(head.chars().count() <= 4 && tail.chars().count() <= 4, "{len}: {masked}");
            assert!(value.starts_with(head) && value.ends_with(tail), "{len}: {masked}");
        }
        assert_eq!(mask(&fake("sk-proj-", 48)).chars().take(4).collect::<String>(), "sk-p");
        assert_eq!(mask("abc"), MASK);
        assert_eq!(mask("debug"), format!("d{MASK}"));
        // Line breaks never show, nor does the length.
        assert!(!mask("ab\ncdefghijklmnopqrstuvw\nxyz").contains('\n'));
        assert_eq!(mask(&"a".repeat(40)).len(), mask(&"a".repeat(400)).len());
    }

    #[test]
    fn entropy_tells_words_from_noise() {
        assert!(entropy("aaaaaaaa") < 0.1);
        assert!(entropy(&fake("", 40)) > 4.0);
        assert!(placeholder("xxxxxxxx") && placeholder("  ") && !placeholder(&fake("", 20)));
    }
}
