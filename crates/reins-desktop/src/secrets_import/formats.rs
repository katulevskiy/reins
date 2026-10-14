//! The two files a project commits (design `docs/secrets.md`, 2.3), written exactly as the design shows them:
//! `reins.toml` (the project, its default environment, per-environment settings) and `.env.reins` (each variable's
//! reference, or a literal for a value that is not a secret). Neither ever holds a secret.
//!
//! The one place these are written for import. Reins Secrets core owns the formats and their parser; when it lands,
//! import writes through it instead.

use std::fmt::Write as _;

use zeroize::Zeroizing;

/// One `[env.<name>]` section.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnvSettings {
    pub name: String,
    /// How long a release may be held (`8h`, `15m`).
    pub lease: Option<String>,
    /// `confirm = "always"`: never covered by a grant.
    pub confirm_always: bool,
}

impl EnvSettings {
    /// The defaults import writes: production asks every time and is held a short while; the rest are held a day's
    /// work.
    #[must_use]
    pub fn for_env(name: &str) -> Self {
        let production = matches!(name, "production" | "prod");
        Self {
            name: name.to_owned(),
            lease: Some(
                if production {
                    "15m"
                } else {
                    "8h"
                }
                .to_owned(),
            ),
            confirm_always: production,
        }
    }
}

/// A TOML string.
fn quoted(s: &str) -> String {
    toml::Value::String(s.to_owned()).to_string()
}

/// A TOML key: bare when it can be.
fn key(s: &str) -> String {
    if !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
        s.to_owned()
    } else {
        quoted(s)
    }
}

/// `reins.toml` for a new project; `origin`: its git origin (`github.com/acme/api`), so its name stays tied to it.
#[must_use]
pub fn reins_toml(project: &str, default_env: &str, envs: &[EnvSettings], origin: Option<&str>) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "[project]");
    let _ = writeln!(out, "name = {}", quoted(project));
    if let Some(o) = origin {
        let _ = writeln!(out, "origin = {}", quoted(o));
    }
    let _ = writeln!(out, "default_env = {}", quoted(default_env));
    for e in envs {
        let _ = writeln!(out, "\n[env.{}]", key(&e.name));
        if let Some(lease) = &e.lease {
            let _ = writeln!(out, "lease = {}", quoted(lease));
        }
        if e.confirm_always {
            let _ = writeln!(out, "confirm = \"always\"");
        }
    }
    out
}

/// A line of `.env.reins` (or of a `.env` file's rest).
#[derive(Clone, PartialEq, Eq)]
pub enum EnvLine {
    /// `NAME=vault:<project>/<env>/NAME`.
    Reference {
        name: String,
        reference: String,
    },
    /// A value kept as it was (a setting; or, in a `.env` file's rest, a value not moved).
    Literal {
        name: String,
        value: Zeroizing<String>,
    },
}

impl std::fmt::Debug for EnvLine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Reference {
                name,
                reference,
            } => f.debug_struct("Reference").field("name", name).field("reference", reference).finish(),
            // Never the value.
            Self::Literal {
                name,
                ..
            } => f.debug_struct("Literal").field("name", name).finish_non_exhaustive(),
        }
    }
}

impl EnvLine {
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::Reference {
                name,
                ..
            }
            | Self::Literal {
                name,
                ..
            } => name,
        }
    }
}

/// The reference to a variable of an environment.
#[must_use]
pub fn reference(project: &str, env: &str, name: &str) -> String {
    format!("vault:{project}/{env}/{name}")
}

/// A dotenv value, quoted when it has to be.
fn dotenv_value(value: &str) -> Zeroizing<String> {
    if value.is_empty()
        || value.bytes().all(|b| b.is_ascii_graphic() && !matches!(b, b'#' | b'"' | b'\'' | b'\\' | b'`'))
    {
        Zeroizing::new(value.to_owned())
    } else {
        let mut out = Zeroizing::new(String::with_capacity(value.len() + 8));
        out.push('"');
        for c in value.chars() {
            match c {
                '\\' => out.push_str("\\\\"),
                '"' => out.push_str("\\\""),
                '\n' => out.push_str("\\n"),
                c => out.push(c),
            }
        }
        out.push('"');
        out
    }
}

/// `.env.reins`: one line per variable, in order.
#[must_use]
pub fn env_reins(lines: &[EnvLine]) -> Zeroizing<String> {
    let mut out = Zeroizing::new(String::new());
    for l in lines {
        match l {
            EnvLine::Reference {
                name,
                reference,
            } => {
                let _ = writeln!(out, "{name}={reference}");
            }
            EnvLine::Literal {
                name,
                value,
            } => {
                out.push_str(name);
                out.push('=');
                out.push_str(&dotenv_value(value));
                out.push('\n');
            }
        }
    }
    out
}

/// `existing` (an `.env.reins` already there) with `lines` added: a variable already in it is replaced where it is,
/// a new one is added at the end; its other lines (comments too) stay.
#[must_use]
pub fn merge_env_reins(existing: &str, lines: &[EnvLine]) -> Zeroizing<String> {
    let mut out = Zeroizing::new(String::new());
    let mut written: Vec<&str> = Vec::new();
    for raw in existing.lines() {
        let name =
            raw.trim_start().strip_prefix("export ").unwrap_or(raw.trim_start()).split('=').next().unwrap_or("").trim();
        if let Some(l) = lines.iter().find(|l| !raw.trim_start().starts_with('#') && l.name() == name) {
            out.push_str(&env_reins(std::slice::from_ref(l)));
            written.push(l.name());
        } else {
            out.push_str(raw);
            out.push('\n');
        }
    }
    let rest: Vec<EnvLine> = lines.iter().filter(|l| !written.contains(&l.name())).cloned().collect();
    out.push_str(&env_reins(&rest));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reins_toml_is_the_designs() {
        let text = reins_toml(
            "acme-web",
            "development",
            &[EnvSettings::for_env("development"), EnvSettings::for_env("production")],
            Some("github.com/acme/web"),
        );
        assert_eq!(
            text,
            "[project]\nname = \"acme-web\"\norigin = \"github.com/acme/web\"\ndefault_env = \"development\"\n\n\
             [env.development]\nlease = \"8h\"\n\n[env.production]\nlease = \"15m\"\nconfirm = \"always\"\n"
        );
        let parsed: toml::Value = toml::from_str(&text).unwrap();
        assert_eq!(parsed["project"]["name"].as_str(), Some("acme-web"));
        assert_eq!(parsed["project"]["origin"].as_str(), Some("github.com/acme/web"));
        assert_eq!(parsed["env"]["production"]["confirm"].as_str(), Some("always"));
        // Names that are not bare keys are quoted; no origin, no line.
        let odd = reins_toml("my \"app\"", "dev", &[EnvSettings::for_env("qa env")], None);
        assert!(!odd.contains("origin"));
        let parsed: toml::Value = toml::from_str(&odd).unwrap();
        assert_eq!(parsed["project"]["name"].as_str(), Some("my \"app\""));
        assert!(parsed["env"].get("qa env").is_some());
    }

    #[test]
    fn env_reins_is_the_designs_and_merges() {
        let lines = [
            EnvLine::Reference {
                name: "OPENAI_API_KEY".to_owned(),
                reference: reference("acme-web", "development", "OPENAI_API_KEY"),
            },
            EnvLine::Literal {
                name: "LOG_LEVEL".to_owned(),
                value: Zeroizing::new("debug".to_owned()),
            },
            EnvLine::Literal {
                name: "APP_NAME".to_owned(),
                value: Zeroizing::new("Acme Web # 1".to_owned()),
            },
        ];
        assert_eq!(
            env_reins(&lines).as_str(),
            "OPENAI_API_KEY=vault:acme-web/development/OPENAI_API_KEY\nLOG_LEVEL=debug\nAPP_NAME=\"Acme Web # 1\"\n"
        );
        let merged = merge_env_reins("# kept\nLOG_LEVEL=info\nOTHER=vault:x/y/OTHER\n", &lines);
        assert_eq!(
            merged.as_str(),
            "# kept\nLOG_LEVEL=debug\nOTHER=vault:x/y/OTHER\nOPENAI_API_KEY=vault:acme-web/development/OPENAI_API_KEY\n\
             APP_NAME=\"Acme Web # 1\"\n"
        );
        // What the scanner's parser reads back is what was written.
        let back = crate::secrets_scan::sources::dotenv(&env_reins(&lines));
        assert_eq!(back[2].value.as_str(), "Acme Web # 1");
    }
}
