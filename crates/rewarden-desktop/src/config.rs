//! Where the app keeps things, and its settings (`config.toml`).
//!
//! Config (settings the user edits) lives in `$REWARDEN_CONFIG_DIR`, else `$XDG_CONFIG_HOME/rewarden`, else
//! `~/.config/rewarden`. State (the identity key, the server session, the control token) lives in
//! `$REWARDEN_STATE_DIR`, else `$XDG_STATE_HOME/rewarden`, else `~/.local/state/rewarden`. Both are created 0700 and
//! every secret file 0600.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const DEFAULT_PORT: u16 = 7457;

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("cannot find the home directory; set REWARDEN_CONFIG_DIR and REWARDEN_STATE_DIR")]
    NoHome,
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{path}: {message}")]
    Invalid {
        path: PathBuf,
        message: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Paths {
    pub config_dir: PathBuf,
    pub state_dir: PathBuf,
}

fn env_dir(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from)
}

impl Paths {
    /// From the environment (see the module docs).
    pub fn from_env() -> Result<Self, ConfigError> {
        let home = env_dir("HOME").or_else(|| env_dir("USERPROFILE"));
        let config_dir = env_dir("REWARDEN_CONFIG_DIR")
            .or_else(|| env_dir("XDG_CONFIG_HOME").map(|d| d.join("rewarden")))
            .or_else(|| home.as_ref().map(|h| h.join(".config").join("rewarden")))
            .ok_or(ConfigError::NoHome)?;
        let state_dir = env_dir("REWARDEN_STATE_DIR")
            .or_else(|| env_dir("XDG_STATE_HOME").map(|d| d.join("rewarden")))
            .or_else(|| home.as_ref().map(|h| h.join(".local").join("state").join("rewarden")))
            .ok_or(ConfigError::NoHome)?;
        Ok(Self {
            config_dir,
            state_dir,
        })
    }

    /// Both directories under one root (tests).
    #[must_use]
    pub fn under(root: &Path) -> Self {
        Self {
            config_dir: root.join("config"),
            state_dir: root.join("state"),
        }
    }

    #[must_use]
    pub fn config_file(&self) -> PathBuf {
        self.config_dir.join("config.toml")
    }

    /// The app's X25519 key (see [`crate::identity`]).
    #[must_use]
    pub fn identity_file(&self) -> PathBuf {
        self.state_dir.join("identity.key")
    }

    /// The Rewarden server session (see [`crate::server`]).
    #[must_use]
    pub fn session_file(&self) -> PathBuf {
        self.state_dir.join("session.json")
    }

    /// The secret the CLI sends to the daemon's control API.
    #[must_use]
    pub fn control_token_file(&self) -> PathBuf {
        self.state_dir.join("control.token")
    }

    /// Creates both directories (0700 on Unix).
    pub fn ensure(&self) -> Result<(), ConfigError> {
        for dir in [&self.config_dir, &self.state_dir] {
            std::fs::create_dir_all(dir).map_err(|source| ConfigError::Io {
                path: dir.clone(),
                source,
            })?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).map_err(|source| {
                    ConfigError::Io {
                        path: dir.clone(),
                        source,
                    }
                })?;
            }
        }
        Ok(())
    }
}

/// Writes `bytes` to `path` readable by the owner only, replacing it atomically.
pub fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        tmp.as_file().set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    std::io::Write::write_all(&mut tmp, bytes)?;
    tmp.as_file().sync_all()?;
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(())
}

/// Who decides and where credentials come from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// The phone when the app is logged in to a Rewarden server, else the local policy.
    #[default]
    Auto,
    /// The local policy and a local GitHub token.
    Local,
    /// The phone, through the Rewarden server; nothing works while logged out.
    Rewarden,
}

/// What the local policy does with a kind of request.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rule {
    Allow,
    Ask,
    Deny,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GithubConfig {
    /// Host part of the proxy paths and of remote URLs (`github.com`).
    pub host: String,
    /// Where git traffic goes.
    pub git_base: String,
    /// The REST API, used to describe pushes.
    pub api_base: String,
    /// Rewarden mode: which of the phone's GitHub accounts to use (optional with one).
    pub account: Option<String>,
    /// Local mode: where the token comes from. `gh` (the GitHub CLI's `gh auth token`), `env:NAME`, or `file:PATH`.
    pub token: String,
}

impl Default for GithubConfig {
    fn default() -> Self {
        Self {
            host: "github.com".to_owned(),
            git_base: "https://github.com".to_owned(),
            api_base: "https://api.github.com".to_owned(),
            account: None,
            token: "gh".to_owned(),
        }
    }
}

/// The git hosts' services, as the phone's tools name them (`gitlab_git_fetch`).
pub const GIT_SERVICES: [&str; 4] = ["github", "gitlab", "codeberg", "bitbucket"];

/// "GitLab" for `gitlab`, for messages.
#[must_use]
pub fn service_label(service: &str) -> &'static str {
    match service {
        "github" => "GitHub",
        "gitlab" => "GitLab",
        "codeberg" => "Codeberg",
        "bitbucket" => "Bitbucket",
        _ => "the git host",
    }
}

/// One `[[git.hosts]]` entry as written. Only `host` is required; what is left out comes from the built-in host of
/// that name (github.com, gitlab.com, codeberg.org, bitbucket.org), or for another host from its `service`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostEntry {
    /// Host part of the proxy paths and of remote URLs (`gitlab.com`).
    pub host: String,
    /// One of [`GIT_SERVICES`]: whose phone tools are asked. Required for a host that is not built in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service: Option<String>,
    /// Where git traffic goes (default `https://<host>`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_base: Option<String>,
    /// The host's API (only GitHub's is used, to describe pushes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_base: Option<String>,
    /// Whether git is sent through Rewarden for this host. Built in: github.com yes, the others no; another host: yes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// Rewarden mode: which of the phone's accounts for this service to use (optional with one).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    /// Local mode: where the token comes from (`env:NAME`, `file:PATH`; `gh` for GitHub).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    /// Local mode: the HTTP user name sent with the token (GitHub `x-access-token`, GitLab `oauth2`, ...).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
}

/// `[git]`: the hosts besides (or overriding) the built-in ones.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GitConfig {
    pub hosts: Vec<HostEntry>,
}

/// A git host with everything filled in (see [`Config::git_hosts`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHost {
    pub host: String,
    pub service: String,
    pub git_base: String,
    pub api_base: String,
    pub enabled: bool,
    pub account: Option<String>,
    pub token: Option<String>,
    pub username: String,
}

impl GitHost {
    fn new(host: &str, service: &str, git_base: String, api_base: String, enabled: bool) -> Self {
        Self {
            host: host.to_owned(),
            service: service.to_owned(),
            git_base,
            api_base,
            enabled,
            account: None,
            token: None,
            username: match service {
                "gitlab" => "oauth2",
                "bitbucket" => "x-token-auth",
                _ => "x-access-token",
            }
            .to_owned(),
        }
    }

    /// "GitLab", for messages.
    #[must_use]
    pub fn label(&self) -> &'static str {
        service_label(&self.service)
    }

    fn apply(&mut self, e: &HostEntry) {
        let set = |field: &mut String, value: &Option<String>| {
            if let Some(v) = value {
                field.clone_from(v);
            }
        };
        set(&mut self.git_base, &e.git_base);
        set(&mut self.api_base, &e.api_base);
        set(&mut self.username, &e.username);
        if let Some(enabled) = e.enabled {
            self.enabled = enabled;
        }
        if e.account.is_some() {
            self.account.clone_from(&e.account);
        }
        if e.token.is_some() {
            self.token.clone_from(&e.token);
        }
    }
}

fn valid_host(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 253
        && host.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-'))
        && !host.starts_with(['.', '-'])
        && !host.ends_with(['.', '-'])
        && !host.contains("..")
}

fn check_url(name: &str, url: &str) -> Result<(), String> {
    let parsed = url::Url::parse(url).map_err(|e| format!("`{name}`: {e}"))?;
    let local = matches!(parsed.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"));
    if parsed.scheme() != "https" && !(parsed.scheme() == "http" && local) {
        return Err(format!("`{name}` must be https (http only for localhost)"));
    }
    Ok(())
}

/// One local rule; the first rule whose `repo` (and `host` and `branch`, when set) matches decides, else the defaults.
/// Patterns: `*` matches any run of characters within a path part, `**` across parts (`owner/*`, `group/**`,
/// `feature/**`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyRule {
    /// The git host (`gitlab.com`, `*.example.com`); any host when not set.
    #[serde(default)]
    pub host: Option<String>,
    /// The repository path: `owner/name`, on GitLab `group/subgroup/name`.
    pub repo: String,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub read: Option<Rule>,
    #[serde(default)]
    pub push: Option<Rule>,
    /// Force pushes, deletions, moved tags.
    #[serde(default)]
    pub risky: Option<Rule>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PolicyConfig {
    pub read: Rule,
    pub push: Rule,
    pub risky: Rule,
    pub rules: Vec<PolicyRule>,
}

impl Default for PolicyConfig {
    fn default() -> Self {
        Self {
            read: Rule::Allow,
            push: Rule::Ask,
            risky: Rule::Ask,
            rules: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Loopback only.
    pub listen: SocketAddr,
    pub mode: Mode,
    /// How long git is kept waiting for an approval before the push or fetch is refused (it can be retried; an
    /// approval given later still counts for the same push).
    pub approval_timeout_secs: u64,
    /// The GitHub host (kept from before `[[git.hosts]]`; an entry for the same host there wins).
    pub github: GithubConfig,
    pub git: GitConfig,
    pub policy: PolicyConfig,
    /// Where `rewarden update` looks for releases.
    pub releases: String,
    /// What harness hooks ask about (`rewarden hook`).
    pub guard: crate::guard::GuardConfig,
    /// `rewarden run --profile` sets (`[run.profiles.<name>]`).
    pub run: crate::run::RunConfig,
    /// APIs the daemon calls with a key from the phone (`[[api]]`).
    pub api: Vec<crate::api_proxy::ApiConfig>,
    /// The SSH agent (`[ssh]`).
    pub ssh: crate::ssh_agent::SshConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen: SocketAddr::from(([127, 0, 0, 1], DEFAULT_PORT)),
            mode: Mode::Auto,
            approval_timeout_secs: 120,
            github: GithubConfig::default(),
            git: GitConfig::default(),
            policy: PolicyConfig::default(),
            releases: crate::update::DEFAULT_RELEASES.to_owned(),
            guard: crate::guard::GuardConfig::default(),
            run: crate::run::RunConfig::default(),
            api: Vec::new(),
            ssh: crate::ssh_agent::SshConfig::default(),
        }
    }
}

impl Config {
    /// The config file, or the defaults when there is none.
    pub fn load(paths: &Paths) -> Result<Self, ConfigError> {
        let path = paths.config_file();
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(source) => {
                return Err(ConfigError::Io {
                    path,
                    source,
                });
            }
        };
        let config: Self = toml::from_str(&text).map_err(|e| ConfigError::Invalid {
            path: path.clone(),
            message: e.to_string(),
        })?;
        config.validate().map_err(|message| ConfigError::Invalid {
            path,
            message,
        })?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), String> {
        if !self.listen.ip().is_loopback() {
            return Err("`listen` must be a loopback address (127.0.0.1 or ::1)".to_owned());
        }
        if !(5..=3_600).contains(&self.approval_timeout_secs) {
            return Err("`approval_timeout_secs` must be between 5 and 3600".to_owned());
        }
        for (name, url) in [
            ("github.git_base", &self.github.git_base),
            ("github.api_base", &self.github.api_base),
            ("releases", &self.releases),
        ] {
            check_url(name, url)?;
        }
        if !valid_host(&self.github.host) {
            return Err("`github.host` must be a host name".to_owned());
        }
        self.guard.validate()?;
        self.run.validate()?;
        crate::api_proxy::validate(&self.api)?;
        self.ssh.validate()?;
        self.git_hosts().map(drop)
    }

    /// Every git host: the built-in ones (github.com from `[github]`, gitlab.com, codeberg.org, bitbucket.org) with
    /// the `[[git.hosts]]` entries applied, then the hosts those add. Fails on an entry that does not make sense.
    pub fn git_hosts(&self) -> Result<Vec<GitHost>, String> {
        let gh = &self.github;
        let mut github = GitHost::new(&gh.host, "github", gh.git_base.clone(), gh.api_base.clone(), true);
        github.account.clone_from(&gh.account);
        github.token = Some(gh.token.clone());
        let mut hosts = vec![github];
        for (host, service, api) in [
            ("gitlab.com", "gitlab", "https://gitlab.com/api/v4"),
            ("codeberg.org", "codeberg", "https://codeberg.org/api/v1"),
            ("bitbucket.org", "bitbucket", "https://api.bitbucket.org/2.0"),
        ] {
            if !host.eq_ignore_ascii_case(&gh.host) {
                hosts.push(GitHost::new(host, service, format!("https://{host}"), api.to_owned(), false));
            }
        }
        let mut seen = std::collections::HashSet::new();
        for e in &self.git.hosts {
            if !valid_host(&e.host) {
                return Err(format!("`git.hosts`: `{}` must be a host name (like gitlab.com)", e.host));
            }
            let key = e.host.to_ascii_lowercase();
            if !seen.insert(key.clone()) {
                return Err(format!("`git.hosts`: {key} is listed more than once"));
            }
            if let Some(service) = &e.service
                && !GIT_SERVICES.contains(&service.as_str())
            {
                return Err(format!(
                    "`git.hosts`: {key}: the service `{service}` is not one of {}",
                    GIT_SERVICES.join(", ")
                ));
            }
            if let Some(h) = hosts.iter_mut().find(|h| h.host.eq_ignore_ascii_case(&key)) {
                if e.service.as_ref().is_some_and(|s| *s != h.service) {
                    return Err(format!("`git.hosts`: {key} is a {} host; its service cannot change", h.label()));
                }
                h.apply(e);
            } else {
                let service = e.service.as_deref().ok_or_else(|| {
                    format!("`git.hosts`: {key} is not built in; give its `service` ({})", GIT_SERVICES.join(", "))
                })?;
                let git_base = e.git_base.clone().unwrap_or_else(|| format!("https://{key}"));
                let api_base = match service {
                    "github" => format!("{}/api/v3", git_base.trim_end_matches('/')),
                    "gitlab" => format!("{}/api/v4", git_base.trim_end_matches('/')),
                    "codeberg" => format!("{}/api/v1", git_base.trim_end_matches('/')),
                    _ => git_base.clone(),
                };
                let mut h = GitHost::new(&key, service, git_base, api_base, true);
                h.apply(e);
                hosts.push(h);
            }
        }
        for h in &hosts {
            check_url(&format!("git.hosts: {} git_base", h.host), &h.git_base)?;
            check_url(&format!("git.hosts: {} api_base", h.host), &h.api_base)?;
            if h.username.is_empty() || h.username.contains([':', '\n', '\r']) {
                return Err(format!("`git.hosts`: {}: the username must be a non-empty name without `:`", h.host));
            }
        }
        Ok(hosts)
    }

    /// The hosts git is sent through Rewarden for.
    pub fn enabled_hosts(&self) -> Result<Vec<GitHost>, String> {
        Ok(self.git_hosts()?.into_iter().filter(|h| h.enabled).collect())
    }

    /// The base URL GitHub remotes are rewritten to: `http://127.0.0.1:7457/github.com/`.
    #[must_use]
    pub fn proxy_base(&self) -> String {
        self.proxy_base_for(&self.github.host)
    }

    /// The base URL remotes of `host` are rewritten to: `http://127.0.0.1:7457/gitlab.com/`.
    #[must_use]
    pub fn proxy_base_for(&self, host: &str) -> String {
        format!("http://{}/{host}/", self.listen)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid_and_a_file_can_override_parts() {
        Config::default().validate().unwrap();
        let c: Config = toml::from_str(
            "listen = \"127.0.0.1:9000\"\n[policy]\npush = \"allow\"\n[[policy.rules]]\nrepo = \"me/*\"\nbranch = \"main\"\npush = \"ask\"\n",
        )
        .unwrap();
        c.validate().unwrap();
        assert_eq!(c.listen.port(), 9000);
        assert_eq!(c.policy.push, Rule::Allow);
        assert_eq!(c.policy.risky, Rule::Ask);
        assert_eq!(c.policy.rules[0].push, Some(Rule::Ask));
        assert_eq!(c.proxy_base(), "http://127.0.0.1:9000/github.com/");
    }

    #[test]
    fn listening_beyond_loopback_or_plain_http_upstreams_are_refused() {
        let open = Config {
            listen: "0.0.0.0:7457".parse().unwrap(),
            ..Config::default()
        };
        assert!(open.validate().is_err());
        let mut c = Config::default();
        c.github.api_base = "http://api.github.com".to_owned();
        assert!(c.validate().is_err());
        c.github.api_base = "http://127.0.0.1:9999".to_owned();
        c.validate().unwrap();
        assert!(toml::from_str::<Config>("unknown = 1").is_err());
    }

    #[test]
    fn private_files_are_owner_only() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        paths.ensure().unwrap();
        let f = paths.session_file();
        write_private(&f, b"x").unwrap();
        write_private(&f, b"yz").unwrap();
        assert_eq!(std::fs::read(&f).unwrap(), b"yz");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(std::fs::metadata(&f).unwrap().permissions().mode() & 0o777, 0o600);
            assert_eq!(std::fs::metadata(&paths.state_dir).unwrap().permissions().mode() & 0o777, 0o700);
        }
    }
}
