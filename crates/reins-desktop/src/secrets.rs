//! Secrets from the phone's vault, for `reins run` and the API proxy: references (`vault:Item/field`), the
//! `vault_secret_release` question, and the checks on the sealed [`SecretGrant`] (this request's nonce, every requested
//! secret in order, not expired). Values are held in [`Zeroizing`] strings and never logged.

use reins_proto::desktop::SecretGrant;
use serde_json::json;
use zeroize::{Zeroize as _, Zeroizing};

use crate::auth::Refusal;
use crate::phone::{Phone, unavailable};

pub const TOOL: &str = "vault_secret_release";
/// At most this many secrets in one release (the tool's limit).
pub const MAX_SECRETS: usize = 10;
pub const MIN_LEASE_SECS: u32 = 60;
pub const MAX_LEASE_SECS: u32 = 86_400;
const MAX_REFERENCE: usize = 300;
const MAX_COMMAND: usize = 500;
const MAX_PURPOSE: usize = 300;

/// One secret in the vault: an item (its id or exact name) and a field (`password`, `username`, `totp`, `notes`, `uri`
/// or a custom field's name).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SecretRef {
    pub item: String,
    pub field: String,
}

impl SecretRef {
    /// `vault:Item/field`; the field is after the last `/`, so item names may contain `/`.
    pub fn parse(s: &str) -> Result<Self, String> {
        let rest = s
            .strip_prefix("vault:")
            .ok_or_else(|| format!("`{s}` is not a vault reference; write `vault:Item/field`"))?;
        let (item, field) =
            rest.rsplit_once('/').ok_or_else(|| format!("`{s}` names no field; write `vault:Item/field`"))?;
        let (item, field) = (item.trim(), field.trim());
        if item.is_empty() || field.is_empty() {
            return Err(format!("`{s}` needs an item and a field: `vault:Item/field`"));
        }
        if s.chars().any(char::is_control) {
            return Err("a vault reference must not contain control characters".to_owned());
        }
        let r = Self {
            item: item.to_owned(),
            field: field.to_owned(),
        };
        if r.reference().chars().count() > MAX_REFERENCE {
            return Err(format!("a vault reference is at most {MAX_REFERENCE} characters"));
        }
        Ok(r)
    }

    /// `Item/field`: what the phone is asked for, and what its answer echoes.
    #[must_use]
    pub fn reference(&self) -> String {
        format!("{}/{}", self.item, self.field)
    }
}

impl std::fmt::Display for SecretRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "vault:{}/{}", self.item, self.field)
    }
}

impl SecretRef {
    /// The variable of an environment this names, when the item is `project/env` (both valid names) and the field a
    /// variable name: `vault:acme-web/development/OPENAI_API_KEY`.
    #[must_use]
    pub fn as_var(&self) -> Option<VarRef> {
        let (project, env) = self.item.split_once('/')?;
        let env = EnvRef::new(project, env).ok()?;
        check_var(&self.field).ok()?;
        Some(VarRef {
            env,
            name: self.field.clone(),
        })
    }
}

/// The vault folder all of Reins Secrets lives in; a project is the folder `Reins/<project>`.
pub const SECRETS_FOLDER: &str = reins_proto::desktop::SECRETS_FOLDER;
const MAX_NAME: usize = 64;

/// A project or environment name: 1 to 64 lowercase letters, digits and `-`, not starting or ending with `-`. Names
/// never contain a `/`, so `project/env` is one item name and `project/env/NAME` one variable.
pub fn check_name(what: &str, name: &str) -> Result<(), String> {
    if reins_proto::desktop::name_ok(name) {
        Ok(())
    } else {
        Err(format!(
            "`{}` is not a {what} name: 1 to {MAX_NAME} lowercase letters, digits and `-`, not starting or ending with `-`",
            cut(name, 80)
        ))
    }
}

/// An environment variable name: a letter or `_`, then letters, digits and `_`, at most 100.
pub fn check_var(name: &str) -> Result<(), String> {
    if reins_proto::desktop::var_ok(name) {
        Ok(())
    } else {
        Err(format!("`{}` is not an environment variable name (letters, digits, `_`)", cut(name, 80)))
    }
}

/// One environment of a project: the Secure Note `project/env` in the vault folder `Reins/project`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EnvRef {
    pub project: String,
    pub env: String,
}

impl EnvRef {
    pub fn new(project: &str, env: &str) -> Result<Self, String> {
        check_name("project", project)?;
        check_name("environment", env)?;
        Ok(Self {
            project: project.to_owned(),
            env: env.to_owned(),
        })
    }

    /// `project/env`: the vault item's name.
    #[must_use]
    pub fn item(&self) -> String {
        format!("{}/{}", self.project, self.env)
    }

    /// `Reins/project`: the vault folder the item is in.
    #[must_use]
    pub fn folder(&self) -> String {
        format!("{SECRETS_FOLDER}/{}", self.project)
    }

    /// The variable `name` of this environment.
    #[must_use]
    pub fn var(&self, name: &str) -> VarRef {
        VarRef {
            env: self.clone(),
            name: name.to_owned(),
        }
    }
}

impl std::fmt::Display for EnvRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "vault:{}/{}", self.project, self.env)
    }
}

/// One variable of an environment: `vault:project/env/NAME`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct VarRef {
    pub env: EnvRef,
    pub name: String,
}

impl VarRef {
    /// The same variable as an `item/field` reference.
    #[must_use]
    pub fn secret_ref(&self) -> SecretRef {
        SecretRef {
            item: self.env.item(),
            field: self.name.clone(),
        }
    }
}

impl std::fmt::Display for VarRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "vault:{}/{}/{}", self.env.project, self.env.env, self.name)
    }
}

/// A reference as written in `.env.reins`, a `--env` pair, a run profile or on the command line.
///
/// - `vault:Item/field`: one field of any vault item (split at the last `/`). `vault:acme-web/development/KEY` is the
///   variable `KEY` of that environment ([`SecretRef::as_var`]).
/// - `vault:project/env`: every variable of an environment (only where a set of variables is expected:
///   [`Reference::parse_env`]; as a single value it is `vault:Item/field`, as before).
/// - `env:ENV/NAME` and `env:ENV`: the same inside a project (a `reins.toml` above the working directory), which
///   supplies the project ([`Reference::resolve`]).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Reference {
    Field(SecretRef),
    /// `env:ENV/NAME`.
    ProjectVar {
        env: String,
        name: String,
    },
    /// `vault:project/env`.
    Env(EnvRef),
    /// `env:ENV`.
    ProjectEnv {
        env: String,
    },
}

/// A reference with its project filled in.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Resolved {
    /// A field of an item that is not an environment variable of this project.
    Field(SecretRef),
    Var(VarRef),
    Env(EnvRef),
}

impl Reference {
    /// Whether `s` is written as a reference (`vault:…` or `env:…`) rather than a literal value.
    #[must_use]
    pub fn looks_like(s: &str) -> bool {
        s.starts_with("vault:") || s.starts_with("env:")
    }

    /// A reference to one value: `vault:Item/field` or `env:ENV/NAME`.
    pub fn parse_value(s: &str) -> Result<Self, String> {
        if let Some(rest) = s.strip_prefix("env:") {
            let (env, name) = rest.split_once('/').ok_or_else(|| {
                format!("`{}` names a whole environment; write `env:ENV/NAME` for one value", cut(s, 80))
            })?;
            check_name("environment", env)?;
            check_var(name)?;
            return Ok(Self::ProjectVar {
                env: env.to_owned(),
                name: name.to_owned(),
            });
        }
        SecretRef::parse(s).map(Self::Field)
    }

    /// A reference to a whole environment: `vault:project/env` or `env:ENV`.
    pub fn parse_env(s: &str) -> Result<Self, String> {
        if let Some(env) = s.strip_prefix("env:") {
            check_name("environment", env)?;
            return Ok(Self::ProjectEnv {
                env: env.to_owned(),
            });
        }
        let rest = s
            .strip_prefix("vault:")
            .ok_or_else(|| format!("`{}` is not a reference; write `vault:project/env` or `env:ENV`", cut(s, 80)))?;
        let (project, env) = rest
            .split_once('/')
            .ok_or_else(|| format!("`{}` needs a project and an environment: `vault:project/env`", cut(s, 80)))?;
        EnvRef::new(project, env).map(Self::Env)
    }

    /// Fills in the project for `env:` references (an error without one), and recognizes `vault:project/env/NAME`.
    pub fn resolve(&self, project: Option<&str>) -> Result<Resolved, String> {
        let need = |what: &str| {
            project.ok_or_else(|| {
                format!("`{what}` needs a project: run it in a folder with a reins.toml, or give --project")
            })
        };
        Ok(match self {
            Self::Field(r) => r.as_var().map_or_else(|| Resolved::Field(r.clone()), Resolved::Var),
            Self::ProjectVar {
                env,
                name,
            } => Resolved::Var(EnvRef::new(need(&self.to_string())?, env)?.var(name)),
            Self::Env(e) => Resolved::Env(e.clone()),
            Self::ProjectEnv {
                env,
            } => Resolved::Env(EnvRef::new(need(&self.to_string())?, env)?),
        })
    }
}

impl std::fmt::Display for Reference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Field(r) => r.fmt(f),
            Self::ProjectVar {
                env,
                name,
            } => write!(f, "env:{env}/{name}"),
            Self::Env(e) => e.fmt(f),
            Self::ProjectEnv {
                env,
            } => write!(f, "env:{env}"),
        }
    }
}

/// Released secrets, in the order asked for.
pub struct Released {
    pub values: Vec<Zeroizing<String>>,
    /// Unix seconds; not used after it.
    pub expires_at: i64,
}

impl std::fmt::Debug for Released {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Released").field("count", &self.values.len()).field("expires_at", &self.expires_at).finish()
    }
}

/// What the phone is asked: the secrets, the command or route using them (shown to the user), why, and for how long.
pub struct Request<'a> {
    pub secrets: &'a [SecretRef],
    pub command: &'a str,
    pub purpose: Option<&'a str>,
    pub lease_secs: u32,
}

/// `s`, shortened to `max` characters with an ellipsis.
pub(crate) fn cut(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_owned();
    }
    let mut out: String = s.chars().take(max - 1).collect();
    out.push('…');
    out
}

/// Asks the phone to release `request.secrets`. `reuse`: a key under which an unanswered earlier question is polled
/// again instead of asked again.
pub async fn release(
    phone: &Phone,
    request: &Request<'_>,
    reuse: Option<&str>,
    waiting: &str,
) -> Result<Released, Refusal> {
    if request.secrets.is_empty() || request.secrets.len() > MAX_SECRETS {
        return Err(unavailable("Between 1 and 10 secrets can be released at once."));
    }
    let references: Vec<String> = request.secrets.iter().map(SecretRef::reference).collect();
    let command = cut(request.command, MAX_COMMAND);
    let purpose = request.purpose.map(|p| cut(p, MAX_PURPOSE));
    let lease = request.lease_secs.clamp(MIN_LEASE_SECS, MAX_LEASE_SECS);
    let answer = phone
        .ask(
            TOOL,
            reuse,
            |_| {
                let mut args = json!({"secrets": references, "command": command, "lease_secs": lease});
                if let Some(p) = &purpose {
                    args["purpose"] = json!(p);
                }
                args
            },
            waiting,
        )
        .await?;
    let grant: SecretGrant = phone.open(&answer.data)?;
    check(grant, &answer.nonce, &references, crate::now_unix())
}

/// The grant must answer this request (its nonce), give every requested secret in order, and not have expired.
pub fn check(mut grant: SecretGrant, nonce: &str, references: &[String], now: i64) -> Result<Released, Refusal> {
    let result = (|| {
        if grant.nonce != nonce {
            return Err(unavailable("The phone's answer is for another request (the nonce differs); refused."));
        }
        if grant.expires_at <= now {
            return Err(unavailable("The phone's answer has expired; refused. Try again."));
        }
        if grant.secrets.len() != references.len()
            || grant.secrets.iter().zip(references).any(|((name, _), wanted)| name != wanted)
        {
            return Err(unavailable("The phone's answer does not hold the secrets asked for; refused."));
        }
        Ok(Released {
            values: grant.secrets.iter_mut().map(|(_, v)| Zeroizing::new(std::mem::take(v))).collect(),
            expires_at: grant.expires_at,
        })
    })();
    for (_, v) in &mut grant.secrets {
        v.zeroize();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn references_name_an_item_and_a_field() {
        let r = SecretRef::parse("vault:OpenAI/password").unwrap();
        assert_eq!((r.item.as_str(), r.field.as_str()), ("OpenAI", "password"));
        assert_eq!(r.reference(), "OpenAI/password");
        assert_eq!(r.to_string(), "vault:OpenAI/password");
        let r = SecretRef::parse("vault:Work/AWS prod/Access key").unwrap();
        assert_eq!((r.item.as_str(), r.field.as_str()), ("Work/AWS prod", "Access key"));
        for bad in ["OpenAI/password", "vault:OpenAI", "vault:/password", "vault:OpenAI/", "vault:a\n/b", ""] {
            assert!(SecretRef::parse(bad).is_err(), "{bad}");
        }
        assert!(SecretRef::parse(&format!("vault:{}/x", "a".repeat(400))).is_err());
    }

    fn grant(nonce: &str, secrets: &[(&str, &str)], expires_at: i64) -> SecretGrant {
        SecretGrant {
            v: 1,
            nonce: nonce.to_owned(),
            secrets: secrets.iter().map(|(a, b)| ((*a).to_owned(), (*b).to_owned())).collect(),
            expires_at,
            project: None,
            env: None,
            revision: None,
            runner: None,
            root: None,
            command: None,
        }
    }

    #[test]
    fn a_grant_must_answer_exactly_this_request() {
        let refs = vec!["A/password".to_owned(), "B/token".to_owned()];
        let ok = check(grant("n1", &[("A/password", "pw"), ("B/token", "tk")], 200), "n1", &refs, 100).unwrap();
        assert_eq!(ok.values.iter().map(|v| v.as_str()).collect::<Vec<_>>(), ["pw", "tk"]);
        assert_eq!(ok.expires_at, 200);
        assert!(!format!("{ok:?}").contains("pw"));
        for (g, why) in [
            (grant("n2", &[("A/password", "pw"), ("B/token", "tk")], 200), "nonce"),
            (grant("n1", &[("A/password", "pw"), ("B/token", "tk")], 100), "expired"),
            (grant("n1", &[("A/password", "pw")], 200), "missing"),
            (grant("n1", &[("B/token", "tk"), ("A/password", "pw")], 200), "order"),
            (grant("n1", &[("A/password", "pw"), ("C/token", "tk")], 200), "other"),
        ] {
            let r = check(g, "n1", &refs, 100);
            assert!(matches!(r, Err(Refusal::Unavailable(_))), "{why}");
        }
    }

    #[test]
    fn env_references_name_a_project_an_environment_and_a_variable() {
        let v = Reference::parse_value("vault:acme-web/development/OPENAI_API_KEY").unwrap();
        let Resolved::Var(var) = v.resolve(None).unwrap() else {
            panic!("a variable expected")
        };
        assert_eq!(
            (var.env.project.as_str(), var.env.env.as_str(), var.name.as_str()),
            ("acme-web", "development", "OPENAI_API_KEY")
        );
        assert_eq!(var.to_string(), "vault:acme-web/development/OPENAI_API_KEY");
        assert_eq!(var.secret_ref().reference(), "acme-web/development/OPENAI_API_KEY");
        assert_eq!(var.env.folder(), "Reins/acme-web");
        // Not an environment: an item with a space, or a field that is no variable name, stays item/field.
        for legacy in ["vault:OpenAI/password", "vault:Work/AWS prod/Access key", "vault:a/b/Access key"] {
            assert!(matches!(Reference::parse_value(legacy).unwrap().resolve(None).unwrap(), Resolved::Field(_)));
        }
        let short = Reference::parse_value("env:production/STRIPE_KEY").unwrap();
        assert_eq!(short.to_string(), "env:production/STRIPE_KEY");
        assert!(short.resolve(None).unwrap_err().contains("reins.toml"));
        assert_eq!(
            short.resolve(Some("acme-web")).unwrap(),
            Resolved::Var(EnvRef::new("acme-web", "production").unwrap().var("STRIPE_KEY"))
        );
        for bad in ["env:production", "env:/X", "env:prod/1X", "env:.x/X", "OpenAI"] {
            assert!(Reference::parse_value(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn whole_environments_are_named_only_where_a_set_is_expected() {
        assert_eq!(
            Reference::parse_env("vault:acme-web/development").unwrap().resolve(None).unwrap(),
            Resolved::Env(EnvRef::new("acme-web", "development").unwrap())
        );
        assert_eq!(
            Reference::parse_env("env:staging").unwrap().resolve(Some("acme-web")).unwrap(),
            Resolved::Env(EnvRef::new("acme-web", "staging").unwrap())
        );
        // As a single value the same text is item/field, as it always was.
        assert!(matches!(Reference::parse_value("vault:acme-web/development").unwrap(), Reference::Field(_)));
        for bad in ["vault:acme-web", "vault:a/b/c", "vault:a b/c", "env:", "env:a/b", "x"] {
            assert!(Reference::parse_env(bad).is_err(), "{bad}");
        }
        assert!(Reference::looks_like("env:x") && Reference::looks_like("vault:x") && !Reference::looks_like("debug"));
    }

    #[test]
    fn names_are_checked() {
        for ok in ["acme-web", "a", "web2", "machine-danas-macbook-3f9a", &"x".repeat(64)] {
            check_name("project", ok).unwrap();
        }
        for bad in ["", ".hidden", "-x", "x-", "a/b", "a b", "é", "Acme", "web_2.0", "a.b", &"x".repeat(65)] {
            assert!(check_name("project", bad).is_err(), "{bad}");
        }
        check_var("_A1").unwrap();
        assert!(check_var("1A").is_err() && check_var("A-B").is_err() && check_var("").is_err());
    }

    #[test]
    fn long_commands_are_cut_for_the_phone() {
        assert_eq!(cut("abc", 5), "abc");
        assert_eq!(cut("abcdef", 5), "abcd…");
        assert_eq!(cut(&"x".repeat(600), MAX_COMMAND).chars().count(), MAX_COMMAND);
    }
}
