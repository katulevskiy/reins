//! The parsers, one per kind of file: each turns a file's text into its entries (a variable, a token field, a
//! password), every value in a [`Zeroizing`] string. The scanner classifies and masks them and drops them; import
//! parses the file again with the same parser to move exactly what the review showed.

use data_encoding::BASE64;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

/// One value in a file.
pub struct RawEntry {
    /// The variable or field (`OPENAI_API_KEY`, `aws_secret_access_key`, `_authToken`, a user name).
    pub name: String,
    /// What it belongs to within the file: an AWS profile, a registry, a git host, a kube user.
    pub group: Option<String>,
    /// 1-based.
    pub line: u32,
    pub value: Zeroizing<String>,
    /// The file's format says this field holds a credential.
    pub credential: bool,
}

impl std::fmt::Debug for RawEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never the value.
        f.debug_struct("RawEntry")
            .field("name", &self.name)
            .field("group", &self.group)
            .field("line", &self.line)
            .finish_non_exhaustive()
    }
}

fn entry(name: &str, group: Option<&str>, line: usize, value: &str, credential: bool) -> RawEntry {
    RawEntry {
        name: name.to_owned(),
        group: group.map(str::to_owned),
        line: u32::try_from(line + 1).unwrap_or(u32::MAX),
        value: Zeroizing::new(value.to_owned()),
        credential,
    }
}

/// The kinds of file the scanner reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Dotenv,
    SshKey,
    GhCli,
    GitCredentials,
    Npm,
    #[serde(rename = "pypi")]
    PyPi,
    Cargo,
    Docker,
    Aws,
    Gcp,
    Kube,
    Shell,
    Netrc,
}

impl Source {
    pub const ALL: [Self; 13] = [
        Self::Dotenv,
        Self::SshKey,
        Self::GhCli,
        Self::GitCredentials,
        Self::Npm,
        Self::PyPi,
        Self::Cargo,
        Self::Docker,
        Self::Aws,
        Self::Gcp,
        Self::Kube,
        Self::Shell,
        Self::Netrc,
    ];

    /// The id prefix of its findings.
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::Dotenv => "dotenv",
            Self::SshKey => "ssh",
            Self::GhCli => "gh",
            Self::GitCredentials => "git",
            Self::Npm => "npm",
            Self::PyPi => "pypi",
            Self::Cargo => "cargo",
            Self::Docker => "docker",
            Self::Aws => "aws",
            Self::Gcp => "gcp",
            Self::Kube => "kube",
            Self::Shell => "shell",
            Self::Netrc => "netrc",
        }
    }

    /// What the review calls a finding of it, when the file name says too little.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Dotenv => ".env file",
            Self::SshKey => "SSH key",
            Self::GhCli => "GitHub CLI token",
            Self::GitCredentials => "git credentials",
            Self::Npm => "npm token",
            Self::PyPi => "PyPI token",
            Self::Cargo => "crates.io token",
            Self::Docker => "Docker logins",
            Self::Aws => "AWS profiles",
            Self::Gcp => "Google Cloud credentials",
            Self::Kube => "kubeconfig",
            Self::Shell => "Shell exports",
            Self::Netrc => "netrc logins",
        }
    }
}

/// The text of a line without its end-of-line comment (` #…` outside quotes).
fn strip_comment(value: &str) -> &str {
    let bytes = value.as_bytes();
    for (i, b) in bytes.iter().enumerate() {
        if *b == b'#' && (i == 0 || bytes[i - 1].is_ascii_whitespace()) {
            return &value[..i];
        }
    }
    value
}

fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
        && name.len() <= 200
}

/// A quoted value that starts at `rest` (after its opening quote `q`), maybe over several lines; the value and how
/// many more lines it took.
fn quoted(q: char, rest: &str, more: &mut dyn Iterator<Item = &str>) -> (Zeroizing<String>, usize) {
    let mut out = Zeroizing::new(String::new());
    let mut extra = 0;
    let mut text: Zeroizing<String> = Zeroizing::new(rest.to_owned());
    loop {
        let mut chars = text.chars();
        let mut closed = false;
        while let Some(c) = chars.next() {
            if c == q {
                closed = true;
                break;
            }
            if c == '\\' && q == '"' {
                match chars.next() {
                    Some('n') => out.push('\n'),
                    Some('t') => out.push('\t'),
                    Some('r') => out.push('\r'),
                    Some(other) => out.push(other),
                    None => {}
                }
                continue;
            }
            out.push(c);
        }
        if closed {
            return (out, extra);
        }
        match more.next() {
            Some(next) if extra < 200 => {
                out.push('\n');
                extra += 1;
                text = Zeroizing::new(next.to_owned());
            }
            _ => return (out, extra),
        }
    }
}

/// `KEY=value` lines of a dotenv file (`export` allowed; quotes, `#` comments and multi-line quoted values as
/// dotenv libraries read them).
#[must_use]
pub fn dotenv(text: &str) -> Vec<RawEntry> {
    let mut out = Vec::new();
    let mut lines = text.trim_start_matches('\u{feff}').lines().enumerate();
    while let Some((n, line)) = lines.next() {
        let line = line.trim_start();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").map_or(line, str::trim_start);
        let Some((name, rest)) = line.split_once('=') else {
            continue;
        };
        let name = name.trim();
        if !valid_name(name) {
            continue;
        }
        let rest = rest.trim_start();
        let value = match rest.chars().next() {
            Some(q @ ('"' | '\'' | '`')) => {
                let mut more = lines.by_ref().map(|(_, l)| l);
                quoted(q, &rest[1..], &mut more).0
            }
            _ => Zeroizing::new(strip_comment(rest).trim_end().to_owned()),
        };
        out.push(RawEntry {
            name: name.to_owned(),
            group: None,
            line: u32::try_from(n + 1).unwrap_or(u32::MAX),
            value,
            credential: false,
        });
    }
    out
}

/// `export NAME=value` (bash, zsh, sh) and `set -x NAME value` (fish) lines of a shell startup file. Values that
/// expand something (`$HOME`, `$(cmd)`) are left out: they are not stored there.
#[must_use]
pub fn shell(text: &str) -> Vec<RawEntry> {
    let mut out = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("export ") {
            let Some((name, value)) = rest.trim_start().split_once('=') else {
                continue;
            };
            if !valid_name(name) || name.contains(['.', '-']) {
                continue;
            }
            if let Some(v) = shell_value(value) {
                out.push(entry(name, None, n, &v, false));
            }
        } else if let Some(rest) = line.strip_prefix("set ") {
            let words: Vec<&str> = rest.split_whitespace().collect();
            let flags: Vec<&&str> = words.iter().take_while(|w| w.starts_with('-')).collect();
            if !flags.iter().any(|f| f.contains('x')) {
                continue;
            }
            let rest_words = &words[flags.len()..];
            let (Some(name), Some(_)) = (rest_words.first(), rest_words.get(1)) else {
                continue;
            };
            let value_start = rest.find(rest_words[1]).unwrap_or(rest.len());
            if valid_name(name)
                && let Some(v) = shell_value(&rest[value_start..])
            {
                out.push(entry(name, None, n, &v, false));
            }
        }
    }
    out
}

/// A shell word's value: single quotes literal, double quotes and bare words only without `$` or backticks.
fn shell_value(raw: &str) -> Option<Zeroizing<String>> {
    let raw = raw.trim();
    if let Some(rest) = raw.strip_prefix('\'') {
        return rest.split_once('\'').map(|(v, _)| Zeroizing::new(v.to_owned()));
    }
    let v = if let Some(rest) = raw.strip_prefix('"') {
        rest.split_once('"')?.0
    } else {
        strip_comment(raw).split(';').next().unwrap_or("").trim()
    };
    (!v.contains('$') && !v.contains('`') && !v.is_empty()).then(|| Zeroizing::new(v.to_owned()))
}

/// An SSH private key file: its type, whether a passphrase protects it, and its public key (an OpenSSH key carries it
/// in the clear).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SshKey {
    /// `ssh-ed25519`, `ssh-rsa`, `ecdsa-sha2-nistp256`, or the PEM label (`RSA`, `EC`).
    pub kind: String,
    pub encrypted: bool,
    /// The SSH public key blob, when the file has it.
    pub public: Option<Vec<u8>>,
}

/// Reads an SSH private key's header (never its secret part).
#[must_use]
pub fn ssh_key(text: &str) -> Option<SshKey> {
    let text = text.trim_start();
    let first = text.lines().next()?.trim();
    if first == "-----BEGIN OPENSSH PRIVATE KEY-----" {
        let body: Zeroizing<String> = Zeroizing::new(
            text.lines().skip(1).take_while(|l| !l.starts_with("-----END")).map(str::trim).collect::<String>(),
        );
        let bytes = Zeroizing::new(BASE64.decode(body.as_bytes()).ok()?);
        let mut r = crate::ssh_agent::wire::Reader::new(bytes.strip_prefix(b"openssh-key-v1\0")?);
        let cipher = r.str()?.to_owned();
        let _kdf = r.string()?;
        let _kdf_options = r.string()?;
        let keys = r.u32()?;
        if keys == 0 {
            return None;
        }
        let public = r.string()?.to_vec();
        let kind = crate::ssh_agent::keys::key_type(&public).unwrap_or("ssh key").to_owned();
        return Some(SshKey {
            kind,
            encrypted: cipher != "none",
            public: Some(public),
        });
    }
    let label = first.strip_prefix("-----BEGIN ")?.strip_suffix(" PRIVATE KEY-----")?;
    let (kind, encrypted) = match label {
        "ENCRYPTED" => ("PKCS#8".to_owned(), true),
        "" => ("PKCS#8".to_owned(), false),
        "RSA" | "EC" | "DSA" => (label.to_owned(), text.lines().take(4).any(|l| l.contains("ENCRYPTED"))),
        _ => return None,
    };
    Some(SshKey {
        kind,
        encrypted,
        public: None,
    })
}

/// `~/.config/gh/hosts.yml`: each host's `oauth_token` (at the host, or per user under `users:`). The hosts without
/// one keep their token in the system keychain: returned as the second list.
#[must_use]
pub fn gh_hosts(text: &str) -> (Vec<RawEntry>, Vec<(String, usize)>) {
    let mut out = Vec::new();
    let mut hosts: Vec<(String, usize, bool)> = Vec::new();
    for (n, line) in text.lines().enumerate() {
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        if !line.starts_with([' ', '\t']) {
            if let Some(host) = line.trim().strip_suffix(':') {
                hosts.push((host.trim_matches(['"', '\'']).to_owned(), n, false));
            }
            continue;
        }
        if let Some((key, value)) = line.trim().split_once(':')
            && key.trim() == "oauth_token"
            && let Some(host) = hosts.last_mut()
        {
            let value = value.trim().trim_matches(['"', '\'']);
            if !value.is_empty() {
                host.2 = true;
                out.push(entry("oauth_token", Some(&host.0), n, value, true));
            }
        }
    }
    let keychain = hosts.into_iter().filter(|h| !h.2).map(|(h, n, _)| (h, n + 1)).collect();
    (out, keychain)
}

/// Percent-decoding of a URL's user or password.
fn percent_decode(s: &str) -> Zeroizing<String> {
    let bytes = s.as_bytes();
    let mut out = Zeroizing::new(Vec::with_capacity(bytes.len()));
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len() + 1
            && let Some(hex) = s.get(i + 1..i + 3)
            && let Ok(b) = u8::from_str_radix(hex, 16)
        {
            out.push(b);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    Zeroizing::new(String::from_utf8_lossy(&out).into_owned())
}

/// `~/.git-credentials` (the `store` helper's file): `https://user:token@host` per line.
#[must_use]
pub fn git_credentials(text: &str) -> Vec<RawEntry> {
    let mut out = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        let Some((_scheme, rest)) = line.split_once("://") else {
            continue;
        };
        let Some((userinfo, host)) = rest.rsplit_once('@') else {
            continue;
        };
        let host = host.split('/').next().unwrap_or(host);
        let (user, password) = userinfo.split_once(':').unwrap_or((userinfo, ""));
        if password.is_empty() {
            continue;
        }
        let user = percent_decode(user);
        out.push(RawEntry {
            name: user.to_string(),
            group: Some(host.to_owned()),
            line: u32::try_from(n + 1).unwrap_or(u32::MAX),
            value: percent_decode(password),
            credential: true,
        });
    }
    out
}

/// The files `credential.helper store --file <path>` names in a git config (besides `~/.git-credentials`).
#[must_use]
pub fn credential_store_files(gitconfig: &str) -> Vec<String> {
    gitconfig
        .lines()
        .filter_map(|l| {
            let (key, value) = l.trim().split_once('=')?;
            (key.trim() == "helper").then_some(value.trim().trim_matches('"'))
        })
        .filter_map(|v| {
            let mut words = v.split_whitespace();
            (words.next()? == "store").then_some(())?;
            let mut file = None;
            while let Some(w) = words.next() {
                if let Some(f) = w.strip_prefix("--file=") {
                    file = Some(f.to_owned());
                } else if w == "--file" {
                    file = words.next().map(str::to_owned);
                }
            }
            file
        })
        .collect()
}

/// `.npmrc`: `//registry/:_authToken=…`, `_auth`, `_password` (global or per registry).
#[must_use]
pub fn npmrc(text: &str) -> Vec<RawEntry> {
    let mut out = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.starts_with(['#', ';']) {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let (registry, field) = match key.rsplit_once(':') {
            Some((r, f)) if r.starts_with("//") => (Some(r.trim_start_matches('/').trim_end_matches('/')), f),
            _ => (None, key),
        };
        if !matches!(field, "_authToken" | "_auth" | "_password") {
            continue;
        }
        let value = value.trim().trim_matches('"');
        if !value.is_empty() {
            out.push(entry(field, Some(registry.unwrap_or("registry.npmjs.org")), n, value, true));
        }
    }
    out
}

/// `[section]` / `key = value` files (`~/.pypirc`, `~/.aws/credentials`): every key with its section and line.
fn ini(text: &str) -> Vec<(String, String, Zeroizing<String>, usize)> {
    let mut section = String::new();
    let mut out = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with(['#', ';']) {
            continue;
        }
        if let Some(s) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            s.trim().clone_into(&mut section);
            continue;
        }
        if let Some((k, v)) = line.split_once(['=', ':']) {
            out.push((section.clone(), k.trim().to_owned(), Zeroizing::new(v.trim().to_owned()), n));
        }
    }
    out
}

/// `~/.pypirc`: each repository's `password`.
#[must_use]
pub fn pypirc(text: &str) -> Vec<RawEntry> {
    ini(text)
        .into_iter()
        .filter(|(s, k, v, _)| k == "password" && !v.is_empty() && s != "distutils")
        .map(|(s, k, v, n)| entry(&k, Some(&s), n, &v, true))
        .collect()
}

/// `~/.aws/credentials`: each profile's keys.
#[must_use]
pub fn aws_credentials(text: &str) -> Vec<RawEntry> {
    ini(text)
        .into_iter()
        .filter(|(_, k, v, _)| {
            !v.is_empty() && matches!(k.as_str(), "aws_access_key_id" | "aws_secret_access_key" | "aws_session_token")
        })
        .map(|(s, k, v, n)| {
            let credential = k != "aws_access_key_id";
            entry(&k, Some(&s), n, &v, credential)
        })
        .collect()
}

/// The line (0-based) where `needle` first appears, else 0.
fn line_of(text: &str, needle: &str) -> usize {
    text.lines().position(|l| l.contains(needle)).unwrap_or(0)
}

/// `~/.cargo/credentials.toml`: `[registry] token` and `[registries.<name>] token`.
#[must_use]
pub fn cargo_credentials(text: &str) -> Vec<RawEntry> {
    ini(text)
        .into_iter()
        .filter(|(_, k, _, _)| k == "token")
        .filter_map(|(section, k, v, n)| {
            let registry = match section.as_str() {
                "registry" => "crates.io".to_owned(),
                s => s.strip_prefix("registries.")?.trim_matches('"').to_owned(),
            };
            let value = v.trim().trim_matches(['"', '\'']);
            (!value.is_empty()).then(|| entry(&k, Some(&registry), n, value, true))
        })
        .collect()
}

/// `~/.docker/config.json`: each registry's `auth` (base64 `user:password`) or token; the credential helpers the file
/// names (their secrets are in the keychain, not here).
#[must_use]
pub fn docker_config(text: &str) -> (Vec<RawEntry>, Vec<String>) {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else {
        return (Vec::new(), Vec::new());
    };
    let mut out = Vec::new();
    if let Some(auths) = v.get("auths").and_then(serde_json::Value::as_object) {
        for (registry, a) in auths {
            for field in ["auth", "identitytoken", "registrytoken"] {
                if let Some(value) = a.get(field).and_then(serde_json::Value::as_str).filter(|s| !s.is_empty()) {
                    out.push(entry(field, Some(registry), line_of(text, &format!("\"{registry}\"")), value, true));
                }
            }
        }
    }
    let mut helpers: Vec<String> = Vec::new();
    if let Some(store) = v.get("credsStore").and_then(serde_json::Value::as_str) {
        helpers.push(store.to_owned());
    }
    if let Some(map) = v.get("credHelpers").and_then(serde_json::Value::as_object) {
        helpers.extend(map.values().filter_map(serde_json::Value::as_str).map(str::to_owned));
    }
    helpers.sort();
    helpers.dedup();
    (out, helpers)
}

/// Google Cloud's application default credentials: a user's refresh token and client secret, or a service account's
/// private key.
#[must_use]
pub fn gcp_adc(text: &str) -> Vec<RawEntry> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else {
        return Vec::new();
    };
    let group = v.get("type").and_then(serde_json::Value::as_str).unwrap_or("credentials");
    ["refresh_token", "client_secret", "private_key"]
        .iter()
        .filter_map(|field| {
            let value = v.get(*field)?.as_str()?;
            (!value.is_empty()).then(|| entry(field, Some(group), line_of(text, &format!("\"{field}\"")), value, true))
        })
        .collect()
}

/// `~/.kube/config`: each user's `token`, `password` and `client-key-data`; the users that run an exec plugin
/// instead (nothing stored here) are returned as the second list.
#[must_use]
pub fn kubeconfig(text: &str) -> (Vec<RawEntry>, Vec<String>) {
    let mut out = Vec::new();
    let mut exec = Vec::new();
    let mut in_users = false;
    let mut user: Option<String> = None;
    for (n, line) in text.lines().enumerate() {
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        if !line.starts_with([' ', '\t', '-']) {
            in_users = line.trim() == "users:";
            user = None;
            continue;
        }
        if !in_users {
            continue;
        }
        let t = line.trim().trim_start_matches("- ").trim();
        let Some((key, value)) = t.split_once(':') else {
            continue;
        };
        let value = value.trim().trim_matches(['"', '\'']);
        match key.trim() {
            "name" if line.trim_start().starts_with("- ") => user = Some(value.to_owned()),
            "token" | "password" | "client-key-data" if !value.is_empty() => {
                out.push(entry(key.trim(), user.as_deref(), n, value, true));
            }
            "exec" | "auth-provider" => exec.extend(user.clone()),
            _ => {}
        }
    }
    (out, exec)
}

/// `~/.netrc`: `machine host login user password secret`, over one line or several.
#[must_use]
pub fn netrc(text: &str) -> Vec<RawEntry> {
    let mut out = Vec::new();
    let mut host: Option<String> = None;
    let mut login = String::new();
    let mut words = text.lines().enumerate().flat_map(|(n, l)| {
        let l = if l.trim_start().starts_with('#') {
            ""
        } else {
            l
        };
        l.split_whitespace().map(move |w| (n, w))
    });
    while let Some((n, w)) = words.next() {
        match w {
            "machine" => {
                host = words.next().map(|(_, h)| h.to_owned());
                login.clear();
            }
            "default" => {
                host = Some("default".to_owned());
                login.clear();
            }
            "login" => login = words.next().map(|(_, l)| l.to_owned()).unwrap_or_default(),
            "password" | "account" => {
                if let (Some(h), Some((_, value))) = (&host, words.next()) {
                    let name = if login.is_empty() {
                        w.to_owned()
                    } else {
                        login.clone()
                    };
                    out.push(entry(&name, Some(h), n, value, true));
                }
            }
            "macdef" => break,
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(entries: &[RawEntry]) -> Vec<String> {
        entries
            .iter()
            .map(|e| {
                format!("{}{}@{}", e.group.as_deref().map(|g| format!("{g}/")).unwrap_or_default(), e.name, e.line)
            })
            .collect()
    }

    #[test]
    fn dotenv_reads_quotes_comments_exports_and_multiline() {
        let text = "\u{feff}# comment\nA=1\nexport B = two # note\nC=\"q # not a comment\"\nD='lit\\n'\nE=\"line1\\nline2\"\n\
                    F=\"-----BEGIN KEY-----\nabc\n-----END KEY-----\"\n G=after\nbad line\n1X=no\nH=\n";
        let e = dotenv(text);
        assert_eq!(names(&e), ["A@2", "B@3", "C@4", "D@5", "E@6", "F@7", "G@10", "H@13"]);
        let v: Vec<&str> = e.iter().map(|e| e.value.as_str()).collect();
        assert_eq!(
            v,
            [
                "1",
                "two",
                "q # not a comment",
                "lit\\n",
                "line1\nline2",
                "-----BEGIN KEY-----\nabc\n-----END KEY-----",
                "after",
                ""
            ]
        );
    }

    #[test]
    fn shell_reads_exports_and_fish_and_skips_expansions() {
        let text = "export OPENAI_API_KEY=sk-abc\nexport PATH=\"$HOME/bin:$PATH\"\nexport TOKEN='a$b'\nalias x=y\n\
                    set -gx GITHUB_TOKEN tok123\nset fish_greeting\nexport X=\"q\" # c\n";
        let e = shell(text);
        assert_eq!(names(&e), ["OPENAI_API_KEY@1", "TOKEN@3", "GITHUB_TOKEN@5", "X@7"]);
        assert_eq!(e[1].value.as_str(), "a$b");
        assert_eq!(e[2].value.as_str(), "tok123");
    }

    #[test]
    fn gh_hosts_finds_tokens_and_keychain_hosts() {
        let text = "github.com:\n    user: dana\n    oauth_token: gho_abc\n    git_protocol: https\n    users:\n        dana:\n            oauth_token: gho_abc\nghe.acme.dev:\n    user: dana\n    git_protocol: ssh\n";
        let (e, keychain) = gh_hosts(text);
        assert_eq!(names(&e), ["github.com/oauth_token@3", "github.com/oauth_token@7"]);
        assert_eq!(keychain, [("ghe.acme.dev".to_owned(), 8)]);
    }

    #[test]
    fn git_credentials_per_host_decoded() {
        let e =
            git_credentials("https://dana:t%40k@github.com\nhttps://x@nopass.dev\nhttps://ci:abc@gitlab.com/group\n");
        assert_eq!(names(&e), ["github.com/dana@1", "gitlab.com/ci@3"]);
        assert_eq!(e[0].value.as_str(), "t@k");
        let files =
            credential_store_files("[credential]\n\thelper = store --file ~/.my-creds\n\thelper = osxkeychain\n");
        assert_eq!(files, ["~/.my-creds"]);
    }

    #[test]
    fn npmrc_pypirc_cargo_aws() {
        let e = npmrc(
            "registry=https://registry.npmjs.org/\n//registry.npmjs.org/:_authToken=abc\n//npm.pkg.github.com/:_authToken=${GH}\n_auth=xyz\n",
        );
        assert_eq!(
            names(&e),
            ["registry.npmjs.org/_authToken@2", "npm.pkg.github.com/_authToken@3", "registry.npmjs.org/_auth@4"]
        );
        let e = pypirc("[distutils]\nindex-servers = pypi\n[pypi]\nusername = __token__\npassword = pypi-abc\n");
        assert_eq!(names(&e), ["pypi/password@5"]);
        let e = cargo_credentials("[registry]\ntoken = \"cio123\"\n[registries.acme]\ntoken = \"t2\"\n");
        assert_eq!(names(&e), ["crates.io/token@2", "acme/token@4"]);
        let e = aws_credentials(
            "[default]\naws_access_key_id = AK\naws_secret_access_key = s\n[prod]\naws_access_key_id=AK2\naws_secret_access_key=s2\nregion=x\n",
        );
        assert_eq!(
            names(&e),
            [
                "default/aws_access_key_id@2",
                "default/aws_secret_access_key@3",
                "prod/aws_access_key_id@5",
                "prod/aws_secret_access_key@6"
            ]
        );
        assert!(!e[0].credential && e[1].credential);
    }

    #[test]
    fn docker_gcp_kube_netrc() {
        let (e, helpers) = docker_config(
            "{\n \"auths\": {\n  \"ghcr.io\": {\"auth\": \"dTpw\"},\n  \"docker.io\": {}\n },\n \"credsStore\": \"desktop\"\n}",
        );
        assert_eq!(names(&e), ["ghcr.io/auth@3"]);
        assert_eq!(helpers, ["desktop"]);
        let e =
            gcp_adc("{\n \"type\": \"authorized_user\",\n \"client_secret\": \"cs\",\n \"refresh_token\": \"rt\"\n}");
        assert_eq!(names(&e), ["authorized_user/refresh_token@4", "authorized_user/client_secret@3"]);
        let kube = "apiVersion: v1\nclusters:\n- cluster:\n    server: https://k\n  name: prod\nusers:\n- name: admin\n  user:\n    token: abc\n- name: eks\n  user:\n    exec:\n      command: aws\n";
        let (e, exec) = kubeconfig(kube);
        assert_eq!(names(&e), ["admin/token@9"]);
        assert_eq!(exec, ["eks"]);
        let e = netrc("machine api.heroku.com\n  login dana\n  password hk1\nmachine x.dev login a password p2\n");
        assert_eq!(names(&e), ["api.heroku.com/dana@3", "x.dev/a@4"]);
    }

    #[test]
    fn ssh_keys_say_their_type_and_passphrase() {
        // An OpenSSH key's container: magic, cipher, kdf, kdf options, one key, its public blob (the secret part is
        // not read).
        let blob = {
            let mut w = crate::ssh_agent::wire::Writer::new();
            w.string(b"ssh-ed25519").string(&[7u8; 32]);
            w.buf
        };
        let file = |cipher: &str| {
            let mut w = crate::ssh_agent::wire::Writer::new();
            w.string(cipher.as_bytes()).string(b"none").string(b"").u32(1).string(&blob).string(b"secret part");
            let mut bytes = b"openssh-key-v1\0".to_vec();
            bytes.extend(w.buf);
            format!(
                "-----BEGIN OPENSSH PRIVATE KEY-----\n{}\n-----END OPENSSH PRIVATE KEY-----\n",
                BASE64.encode(&bytes)
            )
        };
        let plain = ssh_key(&file("none")).unwrap();
        assert_eq!(
            (plain.kind.as_str(), plain.encrypted, plain.public.as_deref()),
            ("ssh-ed25519", false, Some(&blob[..]))
        );
        assert!(ssh_key(&file("aes256-ctr")).unwrap().encrypted);
        let pem = ssh_key("-----BEGIN RSA PRIVATE KEY-----\nProc-Type: 4,ENCRYPTED\nabc\n").unwrap();
        assert_eq!((pem.kind.as_str(), pem.encrypted), ("RSA", true));
        assert!(!ssh_key("-----BEGIN EC PRIVATE KEY-----\nabc\n").unwrap().encrypted);
        assert!(ssh_key("ssh-ed25519 AAAA dana@mac\n").is_none());
    }
}
