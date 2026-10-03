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
    fn long_commands_are_cut_for_the_phone() {
        assert_eq!(cut("abc", 5), "abc");
        assert_eq!(cut("abcdef", 5), "abcd…");
        assert_eq!(cut(&"x".repeat(600), MAX_COMMAND).chars().count(), MAX_COMMAND);
    }
}
