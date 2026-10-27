//! Work profiles on this computer (Decisions v2 · O1, 06 §2, 08 R6): one identity and session per profile, and git
//! routed by the remote's owner.
//!
//! ```toml
//! [[profiles]]
//! name = "acme"              # reins login --profile acme
//! org = "Acme"
//! github_orgs = ["acme"]     # github.com/acme/* goes through this profile
//! hosts = ["git.acme.com"]   # and every repository on these hosts
//! ```
//!
//! The Personal profile is the computer's own state directory, as before. A work profile keeps its own X25519
//! identity and server session under `<state>/profiles/<name>/`, paired with the work account's phone.
//!
//! **Routing.** A remote that belongs to an org (its GitHub organization, or one of its hosts, compared after
//! normalizing the remote: `reins_proto::org::Remote`) goes through that work profile only. With the work profile
//! not signed in, it is refused with what to run; it never falls back to the Personal phone or the local policy.
//! Every other remote stays Personal.

use reins_proto::org::{OrgRoutes, ProfileRef, Remote, route_remote};
use serde::{Deserialize, Serialize};

use crate::{
    auth::Repo,
    config::{Config, Paths},
};

/// One `[[profiles]]` entry.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileConfig {
    /// `[a-z0-9-]`, 1–32: the folder and `--profile` name.
    pub name: String,
    /// The org's name, for messages.
    #[serde(default)]
    pub org: String,
    #[serde(default)]
    pub github_orgs: Vec<String>,
    #[serde(default)]
    pub hosts: Vec<String>,
}

/// Whether `name` can name a profile.
#[must_use]
pub fn valid_name(name: &str) -> bool {
    (1..=32).contains(&name.len()) && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

impl ProfileConfig {
    /// The org's name, or the profile's.
    #[must_use]
    pub fn display(&self) -> &str {
        if self.org.trim().is_empty() {
            &self.name
        } else {
            self.org.trim()
        }
    }
}

/// Checks the `[[profiles]]` entries (called from `Config::validate`).
pub fn validate(profiles: &[ProfileConfig]) -> Result<(), String> {
    let mut seen = std::collections::HashSet::new();
    for p in profiles {
        if !valid_name(&p.name) {
            return Err(format!("profile name `{}`: use 1 to 32 lowercase letters, digits and dashes", p.name));
        }
        if !seen.insert(p.name.as_str()) {
            return Err(format!("profile `{}` is listed twice", p.name));
        }
        if p.github_orgs.is_empty() && p.hosts.is_empty() {
            return Err(format!("profile `{}` needs `github_orgs` or `hosts`", p.name));
        }
    }
    Ok(())
}

/// The profile a repository belongs to.
#[must_use]
pub fn route(config: &Config, repo: &Repo) -> ProfileRef {
    let bindings: Vec<(String, OrgRoutes)> = config
        .profiles
        .iter()
        .map(|p| {
            (
                p.name.clone(),
                OrgRoutes {
                    github_orgs: p.github_orgs.clone(),
                    hosts: p.hosts.clone(),
                },
            )
        })
        .collect();
    route_remote(&bindings, &Remote::from_parts(&repo.host, &repo.path))
}

/// Who decides for a repository.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Choice {
    Personal,
    /// The work profile `name`, at these paths, signed in to `server`.
    Managed {
        name: String,
        paths: Paths,
        server: String,
    },
    /// The repository is an org's and its work profile is not signed in: refused with this message.
    Refuse(String),
}

/// Picks who decides for `repo`; `logged_in` reads a profile's session (the server it is signed in to).
pub fn choose(config: &Config, paths: &Paths, repo: &Repo, logged_in: impl Fn(&Paths) -> Option<String>) -> Choice {
    let ProfileRef::Managed(name) = route(config, repo) else {
        return Choice::Personal;
    };
    let profile = config.profiles.iter().find(|p| p.name == name);
    let display = profile.map_or(name.as_str(), ProfileConfig::display);
    let profile_paths = paths.for_profile(&name);
    match logged_in(&profile_paths) {
        Some(server) => Choice::Managed {
            name,
            paths: profile_paths,
            server,
        },
        None => Choice::Refuse(format!(
            "{}/{} belongs to {display}. Sign in to that profile: reins login --profile {name}",
            repo.host, repo.path
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::Identity;

    fn config() -> Config {
        Config {
            profiles: vec![ProfileConfig {
                name: "acme".to_owned(),
                org: "Acme".to_owned(),
                github_orgs: vec!["acme".to_owned()],
                hosts: vec!["git.acme.com".to_owned()],
            }],
            ..Config::default()
        }
    }

    fn repo(host: &str, path: &str) -> Repo {
        Repo {
            host: host.to_owned(),
            service: "github".to_owned(),
            path: path.to_owned(),
        }
    }

    #[test]
    fn an_org_remote_never_goes_to_the_personal_profile() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        // Only the Personal profile is signed in: the org's repositories are refused, never sent to it.
        let personal_only = |p: &Paths| (*p == paths).then(|| "https://reins.example".to_owned());
        for (host, path) in [("github.com", "acme/api"), ("github.com", "ACME/web"), ("git.acme.com", "team/repo")] {
            match choose(&config(), &paths, &repo(host, path), personal_only) {
                Choice::Refuse(m) => assert!(m.contains("reins login --profile acme") && m.contains("Acme"), "{m}"),
                other => panic!("{host}/{path}: {other:?}"),
            }
        }
        // Signed in, they go through the work profile, at its own paths.
        let both = |_: &Paths| Some("https://reins.example".to_owned());
        match choose(&config(), &paths, &repo("github.com", "acme/api"), both) {
            Choice::Managed {
                name,
                paths: p,
                ..
            } => {
                assert_eq!(name, "acme");
                assert_eq!(p, paths.for_profile("acme"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn personal_remotes_stay_personal() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        let both = |_: &Paths| Some("https://reins.example".to_owned());
        for (host, path) in [("github.com", "dkat/reins"), ("github.com.evil.io", "acme/api"), ("gitlab.com", "acme/x")]
        {
            assert_eq!(choose(&config(), &paths, &repo(host, path), both), Choice::Personal, "{host}/{path}");
        }
        assert_eq!(choose(&Config::default(), &paths, &repo("github.com", "acme/api"), both), Choice::Personal);
    }

    #[test]
    fn each_profile_keeps_its_own_identity_and_session() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        let work = paths.for_profile("acme");
        assert_eq!(work.config_dir, paths.config_dir);
        assert_ne!(work.identity_file(), paths.identity_file());
        assert_ne!(work.session_file(), paths.session_file());
        assert!(work.state_dir.starts_with(&paths.state_dir));
        std::fs::create_dir_all(&paths.state_dir).unwrap();
        std::fs::create_dir_all(&work.state_dir).unwrap();
        let personal = Identity::load_or_create(&paths.identity_file()).unwrap();
        let acme = Identity::load_or_create(&work.identity_file()).unwrap();
        assert_ne!(personal.fingerprint(), acme.fingerprint());
    }

    #[test]
    fn profile_entries_are_checked() {
        let ok = config().profiles;
        assert!(validate(&ok).is_ok());
        let mut twice = ok.clone();
        twice.push(ok[0].clone());
        assert!(validate(&twice).is_err());
        for bad in ["", "Acme", "a/b", "..", &"a".repeat(33)] {
            let p = ProfileConfig {
                name: bad.to_owned(),
                github_orgs: vec!["x".to_owned()],
                ..ProfileConfig::default()
            };
            assert!(validate(&[p]).is_err(), "{bad}");
        }
        let routeless = ProfileConfig {
            name: "x".to_owned(),
            ..ProfileConfig::default()
        };
        assert!(validate(&[routeless]).is_err());
    }
}
