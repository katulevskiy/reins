//! Git hosts besides GitHub — GitLab, Codeberg and Bitbucket — for git on the user's computer through the Reins
//! desktop app. Each is added with a pasted token that is checked against the host's API (who it belongs to) and kept
//! sealed in the local store, per account. A fetch looks the repository up with the token first; a push is previewed
//! from the summary the app sent. The token reaches the app only sealed to its key (see `connector::git`), with the
//! HTTP user name the host expects for git over HTTPS.
//!
//! What differs per host is in `gitlab`, `codeberg` and `bitbucket`: how the pasted text becomes credentials, how the
//! API is authorized, who the token belongs to, where a repository is looked up, and the git user name.

use std::marker::PhantomData;
use std::sync::Arc;
use std::time::Duration;

use reins_proto::connector::ConnectorCall;
use reins_proto::desktop::GIT_FETCH_OP;
use reqwest::StatusCode;
use serde_json::{Value, json};
use zeroize::Zeroizing;

use super::{Connector, Item, Preview, git};
use crate::store::Store;
use crate::types::GmailStatus;
use crate::{CoreError, text};

mod bitbucket;
mod codeberg;
mod gitlab;

pub use bitbucket::Bitbucket;
pub use codeberg::Codeberg;
pub use gitlab::GitLab;

pub const GITLAB_BASE: &str = "https://gitlab.com/api/v4";
pub const CODEBERG_BASE: &str = "https://codeberg.org/api/v1";
pub const BITBUCKET_BASE: &str = "https://api.bitbucket.org/2.0";

const MAX_RETRIES: u32 = 2;
/// The most of an answer that is read.
const MAX_REPLY_BYTES: usize = 1024 * 1024;

/// How a request to a host's API is authorized.
pub enum Auth<'a> {
    /// `Authorization: Bearer <token>`.
    Bearer(&'a str),
    /// `Authorization: token <token>` (Gitea, Forgejo).
    Token(&'a str),
    /// HTTP basic: user (an email) and token.
    Basic(&'a str, &'a str),
}

/// What is kept for an account: the token, the login it belongs to (as the host spells it), and for hosts that need
/// it the email that goes with the token.
pub struct Credentials {
    pub token: Zeroizing<String>,
    pub login: String,
    pub email: Option<String>,
}

impl Credentials {
    fn to_bytes(&self) -> Zeroizing<Vec<u8>> {
        Zeroizing::new(
            serde_json::to_vec(&json!({"token": self.token.as_str(), "login": self.login, "email": self.email}))
                .unwrap_or_default(),
        )
    }

    fn from_bytes(raw: &[u8]) -> Option<Self> {
        let Ok(Value::Object(mut parsed)) = serde_json::from_slice::<Value>(raw) else {
            return None;
        };
        // The token moves out of the parsed JSON into wiped memory.
        let token = match parsed.remove("token") {
            Some(Value::String(t)) => Zeroizing::new(t),
            _ => return None,
        };
        Some(Self {
            token,
            login: parsed.get("login")?.as_str()?.to_owned(),
            email: parsed.get("email").and_then(Value::as_str).map(str::to_owned),
        })
    }
}

/// What makes one git host different from another.
pub trait Forge: Send + Sync + 'static {
    const SERVICE: &'static str;
    const NAME: &'static str;
    /// Repository paths may have groups above the repository (`group/subgroup/name`).
    const NESTED: bool;

    /// The pasted text as a token (and, for hosts that need one, the email that goes with it). The login is filled
    /// in once the host said who it is.
    fn parse(pasted: &str) -> Result<Credentials, CoreError>;

    fn auth(creds: &Credentials) -> Auth<'_>;

    /// The login of the token's owner, from the answer to `GET /user`.
    fn login(me: &Value) -> Option<String>;

    /// The API path of a repository (already checked by `git::repo_arg`).
    fn repo_path(repo: &str) -> String;

    /// Whether the repository the API returned is private.
    fn private(found: &Value) -> bool;

    /// The HTTP basic user name git sends with the token.
    fn git_username(creds: &Credentials) -> String;
}

/// A plain token as pasted: one word of visible characters.
pub(crate) fn plain_token(pasted: &str, what: &str) -> Result<Credentials, CoreError> {
    let token = pasted.trim();
    if token.is_empty() || token.len() > 500 || token.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err(CoreError::invalid(format!("that does not look like a {what} token")));
    }
    Ok(Credentials {
        token: Zeroizing::new(token.to_owned()),
        login: String::new(),
        email: None,
    })
}

/// A git host with tokens, for one [`Forge`].
pub struct GitHost<F: Forge> {
    http: reqwest::Client,
    base: String,
    store: Arc<Store>,
    backoff: Duration,
    forge: PhantomData<F>,
}

/// One answer from a host's API.
struct Reply {
    status: u16,
    json: Value,
}

impl<F: Forge> GitHost<F> {
    pub fn new(http: reqwest::Client, base: &str, store: Arc<Store>, backoff: Duration) -> Self {
        Self {
            http,
            base: base.trim_end_matches('/').to_owned(),
            store,
            backoff,
            forge: PhantomData,
        }
    }

    fn credentials(&self, account: &str) -> Result<Credentials, CoreError> {
        let raw = Zeroizing::new(
            self.store
                .secret_get(F::SERVICE, account)?
                .ok_or_else(|| CoreError::needs_attention(format!("{} is not connected", F::NAME)))?,
        );
        Credentials::from_bytes(&raw).ok_or_else(|| CoreError::storage(format!("corrupt {} token", F::NAME)))
    }

    /// What a failed answer means, in words for the user and the AI (the host's own message is not passed on: it is
    /// shaped differently by every host).
    fn explain(status: StatusCode) -> CoreError {
        let name = F::NAME;
        match status {
            StatusCode::UNAUTHORIZED => CoreError::needs_attention(format!("{name} no longer accepts the token")),
            StatusCode::FORBIDDEN => CoreError::service(format!(
                "{name} refused: the token may lack a permission for that, or the rate limit is used up"
            )),
            StatusCode::NOT_FOUND => {
                CoreError::service(format!("{name} says that does not exist, or the token cannot see it"))
            }
            other => CoreError::service(format!("{name} answered {}", other.as_u16())),
        }
    }

    /// `GET {base}{path}`. Statuses in `allow` are answers, not errors.
    async fn get(&self, creds: &Credentials, path: &str, allow: &[u16]) -> Result<Reply, CoreError> {
        let mut attempt = 0;
        loop {
            let req = self
                .http
                .get(format!("{}{path}", self.base))
                .header("Accept", "application/json")
                .header("User-Agent", "reins");
            let req = match F::auth(creds) {
                Auth::Bearer(t) => req.bearer_auth(t),
                Auth::Token(t) => req.header("Authorization", format!("token {t}")),
                Auth::Basic(user, t) => req.basic_auth(user, Some(t)),
            };
            let resp = req.send().await?;
            let status = resp.status();
            if status.is_success() || allow.contains(&status.as_u16()) {
                let bytes = resp.bytes().await?;
                let json = if bytes.len() > MAX_REPLY_BYTES {
                    Value::Null
                } else {
                    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
                };
                return Ok(Reply {
                    status: status.as_u16(),
                    json,
                });
            }
            if (status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error()) && attempt < MAX_RETRIES {
                tokio::time::sleep(self.backoff * 2u32.pow(attempt)).await;
                attempt += 1;
                continue;
            }
            return Err(Self::explain(status));
        }
    }

    /// Checks pasted credentials and keeps them. Returns the account: the login they belong to, in lower case.
    pub async fn sign_in(&self, pasted: &str) -> Result<String, CoreError> {
        let mut creds = F::parse(pasted)?;
        let me = self.get(&creds, "/user", &[]).await?;
        let login = F::login(&me.json)
            .map(|l| text::one_line(&l))
            .filter(|l| !l.is_empty() && l.len() <= reins_proto::connector::MAX_ACCOUNT_LEN)
            .ok_or_else(|| CoreError::service(format!("{} did not say who that is", F::NAME)))?;
        creds.login.clone_from(&login);
        let account = login.to_lowercase();
        self.store.secret_put(F::SERVICE, &account, &creds.to_bytes())?;
        Ok(account)
    }

    async fn git_fetch(&self, account: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
        let repo = git::repo_arg(call, F::NESTED)?;
        git::check_fetch(call)?;
        let creds = self.credentials(account)?;
        // The token must reach the repository; otherwise the usual error says why.
        let found = self.get(&creds, &F::repo_path(&repo), &[]).await?;
        Ok(vec![git::fetch_item(call, &repo, F::private(&found.json), &F::git_username(&creds), &creds.token)?])
    }
}

#[async_trait::async_trait]
impl<F: Forge> Connector for GitHost<F> {
    fn service(&self) -> &'static str {
        F::SERVICE
    }

    async fn fetch(&self, account: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
        if call.op == GIT_FETCH_OP {
            return self.git_fetch(account, call).await;
        }
        Err(CoreError::service(format!("{} cannot {}", F::NAME, call.op)))
    }

    async fn preview(&self, account: &str, call: &ConnectorCall) -> Result<Preview, CoreError> {
        if !git::is_push(call) {
            return Err(CoreError::service(format!("{} cannot {}", F::NAME, call.op)));
        }
        self.credentials(account)?;
        git::preview_push(call, &git::repo_arg(call, F::NESTED)?)
    }

    async fn perform(&self, account: &str, call: &ConnectorCall) -> Result<Value, CoreError> {
        if !git::is_push(call) {
            return Err(CoreError::service(format!("{} cannot {}", F::NAME, call.op)));
        }
        let creds = self.credentials(account)?;
        git::perform_push(call, &git::repo_arg(call, F::NESTED)?, &F::git_username(&creds), &creds.token)
    }

    async fn sign_in_token(&self, token: &str) -> Result<String, CoreError> {
        self.sign_in(token).await
    }

    /// Git through the desktop app names no account: the first whose token can see the repository.
    async fn choose_account(&self, accounts: &[String], call: &ConnectorCall) -> Result<Option<String>, CoreError> {
        if !call.spec().is_some_and(|s| s.desktop_only) {
            return Ok(None);
        }
        let repo = git::repo_arg(call, F::NESTED)?;
        for account in accounts {
            let Ok(creds) = self.credentials(account) else {
                continue;
            };
            let reply = self.get(&creds, &F::repo_path(&repo), &[401, 403, 404]).await?;
            if (200..300).contains(&reply.status) {
                return Ok(Some(account.clone()));
            }
        }
        Err(CoreError::service(format!("None of the {} accounts on the user's phone can see {repo}.", F::NAME)))
    }

    async fn forget(&self, account: &str) {
        self.store.secret_delete(F::SERVICE, account).ok();
    }

    async fn status(&self, account: &str) -> GmailStatus {
        let Ok(creds) = self.credentials(account) else {
            return GmailStatus::NeedsConsent;
        };
        match self.get(&creds, "/user", &[]).await {
            Ok(_) => GmailStatus::Ready,
            Err(CoreError::ServiceNeedsAttention {
                ..
            }) => GmailStatus::NeedsConsent,
            Err(e) => GmailStatus::Unavailable {
                message: e.to_string(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_round_trip_and_plain_tokens_are_one_word() {
        let creds = Credentials {
            token: Zeroizing::new("t0k".to_owned()),
            login: "Ann".to_owned(),
            email: Some("a@b.c".to_owned()),
        };
        let back = Credentials::from_bytes(&creds.to_bytes()).unwrap();
        assert_eq!((back.token.as_str(), back.login.as_str(), back.email.as_deref()), ("t0k", "Ann", Some("a@b.c")));
        assert!(Credentials::from_bytes(b"garbage").is_none());
        assert!(plain_token("a b", "X").is_err());
        assert!(plain_token("", "X").is_err());
        assert_eq!(plain_token("  glpat-1 ", "X").unwrap().token.as_str(), "glpat-1");
    }
}
