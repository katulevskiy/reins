//! Codeberg (codeberg.org, Forgejo with the Gitea API). An access token with the scopes `read:user` (who it is) and
//! `read:repository` / `write:repository` (fetch, push). The API takes it as `Authorization: token …`; git over HTTPS
//! takes it as the password of the account name.

use reins_proto::connector::CODEBERG;
use serde_json::Value;

use super::{Auth, Credentials, Forge, GitHost, plain_token};
use crate::CoreError;

pub struct Berg;

/// Codeberg, as a connector.
pub type Codeberg = GitHost<Berg>;

impl Forge for Berg {
    const SERVICE: &'static str = CODEBERG;
    const NAME: &'static str = "Codeberg";
    const NESTED: bool = false;

    fn parse(pasted: &str) -> Result<Credentials, CoreError> {
        plain_token(pasted, "Codeberg")
    }

    fn auth(creds: &Credentials) -> Auth<'_> {
        Auth::Token(&creds.token)
    }

    fn login(me: &Value) -> Option<String> {
        me["login"].as_str().map(str::to_owned)
    }

    fn repo_path(repo: &str) -> String {
        format!("/repos/{repo}")
    }

    fn private(found: &Value) -> bool {
        found["private"].as_bool() == Some(true)
    }

    fn git_username(creds: &Credentials) -> String {
        creds.login.clone()
    }
}
