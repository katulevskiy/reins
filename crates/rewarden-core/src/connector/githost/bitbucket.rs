//! Bitbucket (bitbucket.org). App passwords are retired; Rewarden uses an Atlassian API token for Bitbucket with the
//! scopes `read:user:bitbucket` (who it is), `read:repository:bitbucket` (fetch) and `write:repository:bitbucket`
//! (push). The REST API takes it with HTTP basic auth and the Atlassian account email, so it is pasted as
//! `email:token`. Git over HTTPS takes it with the fixed user name `x-bitbucket-api-token-auth` (Bitbucket's own user
//! names are case sensitive there; the fixed one is not a guess).

use rewarden_proto::connector::BITBUCKET;
use serde_json::Value;
use zeroize::Zeroizing;

use super::{Auth, Credentials, Forge, GitHost};
use crate::CoreError;

pub struct Bucket;

/// Bitbucket, as a connector.
pub type Bitbucket = GitHost<Bucket>;

/// The git user name that goes with an API token.
const GIT_USERNAME: &str = "x-bitbucket-api-token-auth";

impl Forge for Bucket {
    const SERVICE: &'static str = BITBUCKET;
    const NAME: &'static str = "Bitbucket";
    const NESTED: bool = false;

    /// `email:token` (or the two separated by a space).
    fn parse(pasted: &str) -> Result<Credentials, CoreError> {
        let wrong = || {
            CoreError::invalid(
                "Paste your Atlassian account email and a Bitbucket API token as email:token (app passwords no longer work)",
            )
        };
        let pasted = pasted.trim();
        let (email, token) = pasted.split_once(|c: char| c == ':' || c.is_whitespace()).ok_or_else(wrong)?;
        let (email, token) = (email.trim(), token.trim());
        let email_ok = email.len() <= 254
            && email.split_once('@').is_some_and(|(user, domain)| !user.is_empty() && domain.contains('.'))
            && !email.chars().any(|c| c.is_control() || c.is_whitespace());
        let token_ok = !token.is_empty()
            && token.len() <= 500
            && !token.chars().any(|c| c.is_control() || c.is_whitespace() || c == ':');
        if !email_ok || !token_ok {
            return Err(wrong());
        }
        Ok(Credentials {
            token: Zeroizing::new(token.to_owned()),
            login: String::new(),
            email: Some(email.to_lowercase()),
        })
    }

    fn auth(creds: &Credentials) -> Auth<'_> {
        Auth::Basic(creds.email.as_deref().unwrap_or_default(), &creds.token)
    }

    fn login(me: &Value) -> Option<String> {
        me["username"].as_str().map(str::to_owned)
    }

    fn repo_path(repo: &str) -> String {
        format!("/repositories/{repo}")
    }

    fn private(found: &Value) -> bool {
        found["is_private"].as_bool() != Some(false)
    }

    fn git_username(_creds: &Credentials) -> String {
        GIT_USERNAME.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_email_and_a_token_are_pasted_together() {
        let c = Bucket::parse(" Ann@Example.com:ATATT3x=abc ").unwrap();
        assert_eq!((c.email.as_deref(), c.token.as_str()), (Some("ann@example.com"), "ATATT3x=abc"));
        assert_eq!(Bucket::parse("ann@example.com ATATT").unwrap().token.as_str(), "ATATT");
        for bad in ["ATATT3x", "ann:ATATT", "ann@example.com:", "@x.y:t", "a@b.c:t t"] {
            assert!(Bucket::parse(bad).is_err(), "{bad}");
        }
    }
}
