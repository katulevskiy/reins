//! The org's GitHub reader (shared by O3 and O4; the interface is O3 plan Task 4's): a second, read-mostly GitHub App the
//! org installs (`contents`, `members`, `pull_requests`: read; `checks`: write) so the server can read the org's policy
//! repo and code owners and post check runs. It is not the executor's App: neither key ever reaches the other side.
//!
//! - Its app and installation ids are set by a two-person admin change (`AdminAction::SetGithubReader`); its private
//!   key and webhook secret are posted apart and sealed under the org's data key.
//! - It authenticates as the App (an RS256 JWT) and exchanges that for an installation token, cached for 50 minutes.
//! - The API base is GitHub's; tests may point it at a loopback mock only with `REINS_TEST_ALLOW_LOOPBACK`.

use std::{
    collections::HashMap,
    sync::{LazyLock, Mutex},
    time::Duration,
};

use jsonwebtoken::{Algorithm, EncodingKey, Header};
use serde_json::{Value, json};
use zeroize::Zeroizing;

use super::{OrgError, kms};
use crate::db::{DbConnInner, models::reins_org_policy_code as code};

/// GitHub's REST API.
pub const GITHUB_API: &str = "https://api.github.com";
/// Test-only override of the API base, honoured only with `REINS_TEST_ALLOW_LOOPBACK`.
pub const API_ENV: &str = "REINS_TEST_GITHUB_API";
/// How long an installation token is reused (GitHub's live an hour).
const TOKEN_SECS: i64 = 50 * 60;
const TIMEOUT: Duration = Duration::from_secs(30);

fn aad(org: &str, part: &str) -> String {
    format!("{org}/github-reader/{part}")
}

/// The API base in use.
#[must_use]
pub fn api_base() -> String {
    if crate::api::reins::outbound::allow_loopback()
        && let Ok(base) = std::env::var(API_ENV)
        && !base.trim().is_empty()
    {
        return base.trim().trim_end_matches('/').to_owned();
    }
    GITHUB_API.to_owned()
}

/// The org's reader, ready to call GitHub.
pub struct GithubReader {
    pub org: String,
    pub app_id: i64,
    pub installation_id: i64,
    key_pem: Zeroizing<String>,
    base: String,
}

impl std::fmt::Debug for GithubReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GithubReader")
            .field("org", &self.org)
            .field("app_id", &self.app_id)
            .field("installation_id", &self.installation_id)
            .finish_non_exhaustive()
    }
}

/// The org's reader, once its ids are set and its key posted.
pub fn for_org(c: &mut DbConnInner, org: &str) -> Result<Option<GithubReader>, OrgError> {
    let Some(row) = code::q_github_reader(c, org)? else {
        return Ok(None);
    };
    let (Some(key_id), Some(sealed)) = (row.key_id.as_deref(), row.sealed.as_deref()) else {
        return Ok(None);
    };
    let pem = kms::open_data(c, org, key_id, &aad(org, "key"), sealed)?;
    let pem = Zeroizing::new(
        String::from_utf8(pem.to_vec()).map_err(|_| OrgError::Internal("reader key isn't text".into()))?,
    );
    Ok(Some(GithubReader {
        org: org.to_owned(),
        app_id: row.app_id,
        installation_id: row.installation_id,
        key_pem: pem,
        base: api_base(),
    }))
}

/// The reader's webhook secret, when posted.
pub fn webhook_secret(c: &mut DbConnInner, org: &str) -> Result<Option<Zeroizing<Vec<u8>>>, OrgError> {
    let Some(row) = code::q_github_reader(c, org)? else {
        return Ok(None);
    };
    match (row.webhook_key_id.as_deref(), row.webhook_sealed.as_deref()) {
        (Some(k), Some(s)) => Ok(Some(kms::open_data(c, org, k, &aad(org, "webhook"), s)?)),
        _ => Ok(None),
    }
}

/// Seals and stores the reader's private key (PEM) and webhook secret; the App's ids must be set already.
pub fn set_secrets(c: &mut DbConnInner, org: &str, pem: &str, webhook_secret: &str, now: i64) -> Result<(), OrgError> {
    let pem = pem.trim();
    if openssl::rsa::Rsa::private_key_from_pem(pem.as_bytes()).is_err() {
        return Err(OrgError::Invalid("That isn't the App's private key (PEM)."));
    }
    if webhook_secret.trim().len() < 16 {
        return Err(OrgError::Invalid("Use a webhook secret of at least 16 characters."));
    }
    let key = kms::seal_data(c, org, &aad(org, "key"), pem.as_bytes(), now)?;
    let hook = kms::seal_data(c, org, &aad(org, "webhook"), webhook_secret.trim().as_bytes(), now)?;
    if !code::q_set_github_reader_secrets(c, org, (&key.0, &key.1), (&hook.0, &hook.1))? {
        return Err(OrgError::Invalid("Set up the GitHub reader's app and installation first."));
    }
    Ok(())
}

#[cfg(test)]
impl GithubReader {
    /// A reader for `org` with `pem`, calling `base` (a mock).
    pub fn for_tests(org: &str, installation_id: i64, pem: &str, base: &str) -> Self {
        Self {
            org: org.to_owned(),
            app_id: 1,
            installation_id,
            key_pem: Zeroizing::new(pem.to_owned()),
            base: base.trim_end_matches('/').to_owned(),
        }
    }
}

/// Installation tokens by (org, installation): (token, reused until).
type TokenCache = HashMap<(String, i64), (String, i64)>;

static TOKENS: LazyLock<Mutex<TokenCache>> = LazyLock::new(Mutex::default);

#[derive(serde::Serialize)]
struct AppClaims {
    iat: i64,
    exp: i64,
    iss: String,
}

impl GithubReader {
    fn client() -> Result<reqwest::Client, String> {
        crate::http_client::get_reqwest_client_builder(false)
            .timeout(TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| e.to_string())
    }

    fn app_jwt(&self, now: i64) -> Result<String, String> {
        let rsa = openssl::rsa::Rsa::private_key_from_pem(self.key_pem.as_bytes()).map_err(|_| "bad reader key")?;
        let pkcs1 = Zeroizing::new(rsa.private_key_to_pem().map_err(|_| "bad reader key")?);
        let key = EncodingKey::from_rsa_pem(&pkcs1).map_err(|_| "bad reader key")?;
        let claims = AppClaims {
            iat: now - 60,
            exp: now + 9 * 60,
            iss: self.app_id.to_string(),
        };
        jsonwebtoken::encode(&Header::new(Algorithm::RS256), &claims, &key).map_err(|e| e.to_string())
    }

    async fn token(&self) -> Result<String, String> {
        let now = crate::api::reins::now_unix();
        let k = (self.org.clone(), self.installation_id);
        if let Some((t, exp)) = TOKENS.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(&k)
            && *exp > now
        {
            return Ok(t.clone());
        }
        let url = format!("{}/app/installations/{}/access_tokens", self.base, self.installation_id);
        let resp = Self::client()?
            .post(url)
            .bearer_auth(self.app_jwt(now)?)
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "reins-server")
            .send()
            .await
            .map_err(|e| format!("GitHub: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("GitHub installation token: {}", resp.status()));
        }
        let v: Value = resp.json().await.map_err(|e| e.to_string())?;
        let t = v["token"].as_str().ok_or("GitHub sent no token")?.to_owned();
        TOKENS.lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert(k, (t.clone(), now + TOKEN_SECS));
        Ok(t)
    }

    async fn call(&self, method: reqwest::Method, path: &str, body: Option<&Value>) -> Result<Value, String> {
        if !path.starts_with('/') || path.contains("..") || path.contains("://") {
            return Err("bad GitHub path".to_owned());
        }
        let token = self.token().await?;
        let mut req = Self::client()?
            .request(method, format!("{}{path}", self.base))
            .bearer_auth(token)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header("User-Agent", "reins-server");
        if let Some(b) = body {
            req = req.json(b);
        }
        let resp = req.send().await.map_err(|e| format!("GitHub: {e}"))?;
        let status = resp.status();
        let v: Value = resp.json().await.unwrap_or(Value::Null);
        if status.is_success() {
            Ok(v)
        } else {
            Err(format!("GitHub {path}: {status}"))
        }
    }

    pub async fn get_json(&self, path: &str) -> Result<Value, String> {
        self.call(reqwest::Method::GET, path, None).await
    }

    pub async fn post_json(&self, path: &str, body: &Value) -> Result<Value, String> {
        self.call(reqwest::Method::POST, path, Some(body)).await
    }

    #[allow(dead_code, reason = "part of the shared reader interface: O3's routing uses it")]
    pub async fn patch_json(&self, path: &str, body: &Value) -> Result<Value, String> {
        self.call(reqwest::Method::PATCH, path, Some(body)).await
    }

    #[allow(dead_code, reason = "part of the shared reader interface: O3's SAML links use it")]
    pub async fn graphql(&self, query: &str, vars: Value) -> Result<Value, String> {
        self.post_json("/graphql", &json!({"query": query, "variables": vars})).await
    }
}
