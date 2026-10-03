//! `reins git setup`: points git at the proxy with `url.<proxy>.insteadOf` rules for each enabled git host's URL
//! forms (`https://gitlab.com/`, `git@gitlab.com:`, `ssh://git@gitlab.com/`), in the global git config or one
//! repository's. Setting up twice changes nothing; `unsetup` removes exactly those rules (for any loopback proxy
//! address, so a changed port does not leave old rules behind) and nothing else.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Command;

use crate::config::Config;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scope {
    Global,
    /// One repository (`git -C DIR config --local`).
    Repo(PathBuf),
}

/// How git is run: extra environment for tests (a temporary `HOME`).
#[derive(Clone, Debug, Default)]
pub struct Git {
    envs: Vec<(OsString, OsString)>,
}

/// The remote URL prefixes rewritten for `host`.
#[must_use]
pub fn sources(host: &str) -> [String; 3] {
    [format!("https://{host}/"), format!("git@{host}:"), format!("ssh://git@{host}/")]
}

impl Git {
    #[must_use]
    pub fn env(mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        self.envs.push((key.into(), value.into()));
        self
    }

    fn run(&self, scope: &Scope, args: &[&str]) -> Result<(bool, String), String> {
        let mut cmd = Command::new("git");
        cmd.envs(self.envs.iter().map(|(k, v)| (k, v)));
        match scope {
            Scope::Global => cmd.args(["config", "--global"]),
            Scope::Repo(dir) => cmd.arg("-C").arg(dir).args(["config", "--local"]),
        };
        let out = cmd.args(args).output().map_err(|e| format!("cannot run git: {e}"))?;
        // Exit 1 from `git config --get…` means "nothing found".
        match out.status.code() {
            Some(0) => Ok((true, String::from_utf8_lossy(&out.stdout).into_owned())),
            Some(1) if args.first().is_some_and(|a| a.starts_with("--get")) => Ok((false, String::new())),
            _ => Err(format!("git config {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim())),
        }
    }

    /// `(proxy base, rewritten prefix)` of every rule this app made (for any loopback proxy address).
    fn ours(&self, scope: &Scope, host: &str) -> Result<Vec<(String, String)>, String> {
        let (_, listing) = self.run(scope, &["--get-regexp", r"^url\..*\.insteadof$"])?;
        let sources = sources(host);
        let suffix = format!("/{host}/");
        Ok(listing
            .lines()
            .filter_map(|line| {
                let (key, value) = line.split_once(' ')?;
                let base = key.strip_prefix("url.")?.strip_suffix(".insteadof")?;
                let addr = base.strip_prefix("http://")?.strip_suffix(&suffix)?;
                let loopback = addr.parse::<std::net::SocketAddr>().is_ok_and(|a| a.ip().is_loopback())
                    || addr.strip_prefix("localhost:").is_some_and(|p| p.parse::<u16>().is_ok());
                (loopback && sources.iter().any(|s| s == value)).then(|| (base.to_owned(), value.to_owned()))
            })
            .collect())
    }

    /// Whether git is sent through this app (any of its rules is present).
    pub fn is_set_up(&self, scope: &Scope, host: &str) -> Result<bool, String> {
        Ok(!self.ours(scope, host)?.is_empty())
    }

    /// Adds the missing rules; returns the prefixes it added.
    pub fn setup(&self, scope: &Scope, proxy_base: &str, host: &str) -> Result<Vec<String>, String> {
        let key = format!("url.{proxy_base}.insteadOf");
        let (_, existing) = self.run(scope, &["--get-all", &key])?;
        let existing: Vec<&str> = existing.lines().collect();
        let mut added = Vec::new();
        for source in sources(host) {
            if !existing.contains(&source.as_str()) {
                self.run(scope, &["--add", &key, &source])?;
                added.push(source);
            }
        }
        Ok(added)
    }

    /// Removes this app's rules; returns how many.
    pub fn unsetup(&self, scope: &Scope, host: &str) -> Result<usize, String> {
        let ours = self.ours(scope, host)?;
        let mut bases: Vec<&str> = Vec::new();
        for (base, value) in &ours {
            self.run(scope, &["--fixed-value", "--unset-all", &format!("url.{base}.insteadOf"), value])?;
            if !bases.contains(&base.as_str()) {
                bases.push(base);
            }
        }
        // git leaves an empty `[url "…"]` header behind; drop it when nothing else is in that section.
        let (_, listing) = self.run(scope, &["--get-regexp", r"^url\."])?;
        for base in bases {
            let prefix = format!("url.{base}.");
            if !listing.lines().any(|l| l.starts_with(&prefix)) {
                self.run(scope, &["--remove-section", &format!("url.{base}")]).ok();
            }
        }
        Ok(ours.len())
    }

    /// Adds the rules of every enabled host of `config` and removes those of the disabled ones; returns the prefixes
    /// added and how many rules were removed.
    pub fn setup_hosts(&self, scope: &Scope, config: &Config) -> Result<(Vec<String>, usize), String> {
        let mut added = Vec::new();
        let mut removed = 0;
        for h in config.git_hosts()? {
            if h.enabled {
                added.extend(self.setup(scope, &config.proxy_base_for(&h.host), &h.host)?);
            } else {
                removed += self.unsetup(scope, &h.host)?;
            }
        }
        Ok((added, removed))
    }

    /// Removes this app's rules for every host of `config`, enabled or not; returns how many.
    pub fn unsetup_hosts(&self, scope: &Scope, config: &Config) -> Result<usize, String> {
        let mut removed = 0;
        for h in config.git_hosts()? {
            removed += self.unsetup(scope, &h.host)?;
        }
        Ok(removed)
    }

    /// The hosts of `config` git is sent through this app for (any of their rules is present).
    pub fn hosts_set_up(&self, scope: &Scope, config: &Config) -> Result<Vec<String>, String> {
        let mut hosts = Vec::new();
        for h in config.git_hosts()? {
            if self.is_set_up(scope, &h.host)? {
                hosts.push(h.host);
            }
        }
        Ok(hosts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(home: &std::path::Path) -> Git {
        Git::default()
            .env("HOME", home)
            .env("XDG_CONFIG_HOME", home.join(".config"))
            .env("GIT_CONFIG_GLOBAL", home.join(".gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
    }

    #[test]
    fn setup_is_idempotent_and_unsetup_removes_exactly_its_rules() {
        let home = tempfile::tempdir().unwrap();
        let g = git(home.path());
        let user = "[user]\n\tname = Me\n[url \"https://example.com/\"]\n\tinsteadOf = ex:\n";
        std::fs::write(home.path().join(".gitconfig"), user).unwrap();
        let base = "http://127.0.0.1:7457/github.com/";
        assert!(!g.is_set_up(&Scope::Global, "github.com").unwrap());
        assert_eq!(g.setup(&Scope::Global, base, "github.com").unwrap().len(), 3);
        assert!(g.is_set_up(&Scope::Global, "github.com").unwrap());
        assert!(g.setup(&Scope::Global, base, "github.com").unwrap().is_empty());
        // An older port's rule for one form, and a user rule for the same base that is not ours.
        g.setup(&Scope::Global, "http://127.0.0.1:9000/github.com/", "github.com").unwrap();
        g.run(&Scope::Global, &["--add", "url.http://127.0.0.1:9000/github.com/.pushInsteadOf", "mine:"]).unwrap();
        let text = std::fs::read_to_string(home.path().join(".gitconfig")).unwrap();
        assert_eq!(text.matches("insteadOf = https://github.com/").count(), 2);
        assert_eq!(text.matches("insteadOf = git@github.com:").count(), 2);
        assert_eq!(g.unsetup(&Scope::Global, "github.com").unwrap(), 6);
        assert_eq!(g.unsetup(&Scope::Global, "github.com").unwrap(), 0);
        let text = std::fs::read_to_string(home.path().join(".gitconfig")).unwrap();
        assert!(!text.contains("7457"), "{text}");
        assert!(text.contains("pushInsteadOf = mine:"), "{text}");
        assert!(text.contains("insteadOf = ex:") && text.contains("name = Me"), "{text}");
    }

    #[test]
    fn a_repository_scope_touches_only_that_repository() {
        let home = tempfile::tempdir().unwrap();
        let g = git(home.path());
        let repo = home.path().join("r");
        let ok = Command::new("git").args(["init", "-q"]).arg(&repo).env("HOME", home.path()).status().unwrap();
        assert!(ok.success());
        let scope = Scope::Repo(repo.clone());
        assert_eq!(g.setup(&scope, "http://[::1]:7457/github.com/", "github.com").unwrap().len(), 3);
        assert!(!home.path().join(".gitconfig").exists());
        let local = std::fs::read_to_string(repo.join(".git/config")).unwrap();
        assert!(local.contains("[url \"http://[::1]:7457/github.com/\"]"), "{local}");
        assert_eq!(g.unsetup(&scope, "github.com").unwrap(), 3);
        assert!(!std::fs::read_to_string(repo.join(".git/config")).unwrap().contains("url"));
    }
}
