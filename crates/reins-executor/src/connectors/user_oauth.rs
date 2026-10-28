//! A family's connections (Decisions v2 · P7, 08 R7): accounts a guardian authorized for one member, held here.
//!
//! - **Connect** ([`UserOAuth::begin`], [`UserOAuth::finish`]): the authorization-code flow with PKCE. The verifier and
//!   `state` stay in `state_dir/connect/<state>` (10 minutes); the client id and secret are this executor's
//!   configuration. The server only relays the provider's one-time code, which it can't redeem.
//! - **Act** ([`Connector::run`]): the member's own token (Google: refreshed from the refresh token; GitHub: the OAuth
//!   token) for exactly the requesting member, as the signed roster names them, and the same calls and minimal
//!   results as the org's connectors.
//! - **Revoke** ([`UserOAuth::revoke`]): upstream (Google `oauth2/revoke`; GitHub `DELETE /applications/{client}/
//!   grant`), then forgotten.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use reins_proto::managed::OAuthProvider;
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use zeroize::Zeroizing;

use super::{
    Connector,
    github::{call_as, minimal, plan},
    google::{Api, TokenSource},
    http_client,
};
use crate::user_tokens::{UserToken, UserTokens};

/// How long a begun connection may take.
pub const PENDING_SECS: i64 = 600;
/// What a Google connection may do: what the org's Google connector does, and who the account is.
pub const GOOGLE_SCOPES: &str = "openid email https://www.googleapis.com/auth/gmail.send \
https://www.googleapis.com/auth/calendar.readonly https://www.googleapis.com/auth/calendar.events";
pub const GITHUB_SCOPES: &str = "repo read:user";

/// One provider's OAuth client, as this executor holds it.
pub struct Client {
    pub provider: OAuthProvider,
    pub client_id: String,
    pub client_secret: Zeroizing<String>,
    pub auth_url: String,
    pub token_url: String,
    /// Google's revocation endpoint (GitHub revokes through its API).
    pub revoke_url: String,
    /// Google's userinfo endpoint (the account's address).
    pub userinfo_url: String,
    pub api_base: String,
}

impl Client {
    /// Google's endpoints; `api_base`: `https://www.googleapis.com`.
    #[must_use]
    pub fn google(client_id: &str, client_secret: Zeroizing<String>) -> Self {
        Self {
            provider: OAuthProvider::Google,
            client_id: client_id.to_owned(),
            client_secret,
            auth_url: "https://accounts.google.com/o/oauth2/v2/auth".to_owned(),
            token_url: "https://oauth2.googleapis.com/token".to_owned(),
            revoke_url: "https://oauth2.googleapis.com/revoke".to_owned(),
            userinfo_url: "https://openidconnect.googleapis.com/v1/userinfo".to_owned(),
            api_base: "https://www.googleapis.com".to_owned(),
        }
    }

    #[must_use]
    pub fn github(client_id: &str, client_secret: Zeroizing<String>) -> Self {
        Self {
            provider: OAuthProvider::Github,
            client_id: client_id.to_owned(),
            client_secret,
            auth_url: "https://github.com/login/oauth/authorize".to_owned(),
            token_url: "https://github.com/login/oauth/access_token".to_owned(),
            revoke_url: String::new(),
            userinfo_url: String::new(),
            api_base: "https://api.github.com".to_owned(),
        }
    }

    /// Every endpoint at `base` (a test's mock server).
    #[must_use]
    pub fn at(mut self, base: &str) -> Self {
        let base = base.trim_end_matches('/');
        self.auth_url = format!("{base}/authorize");
        self.token_url = format!("{base}/token");
        self.revoke_url = format!("{base}/revoke");
        self.userinfo_url = format!("{base}/userinfo");
        base.clone_into(&mut self.api_base);
        self
    }
}

/// What a begun connection keeps until the guardian comes back.
#[derive(Serialize, Deserialize)]
struct Pending {
    member: String,
    provider: OAuthProvider,
    verifier: String,
    redirect_uri: String,
    at: i64,
}

pub struct UserOAuth {
    clients: Vec<Client>,
    tokens: Arc<UserTokens>,
    pending_dir: PathBuf,
    http: reqwest::Client,
}

fn random_b64(n: usize) -> Result<String, String> {
    let mut raw = vec![0u8; n];
    SystemRandom::new().fill(&mut raw).map_err(|_| "no randomness".to_owned())?;
    Ok(data_encoding::BASE64URL_NOPAD.encode(&raw))
}

/// A `state` this executor made: 43 base64url characters (and so a safe file name).
fn state_ok(state: &str) -> bool {
    state.len() == 43 && state.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

impl UserOAuth {
    pub fn new(clients: Vec<Client>, tokens: Arc<UserTokens>, state_dir: &Path) -> Result<Self, String> {
        let pending_dir = state_dir.join("connect");
        std::fs::create_dir_all(&pending_dir).map_err(|e| format!("creating {}: {e}", pending_dir.display()))?;
        Ok(Self {
            clients,
            tokens,
            pending_dir,
            http: http_client()?,
        })
    }

    fn client(&self, provider: OAuthProvider) -> Result<&Client, String> {
        self.clients
            .iter()
            .find(|c| c.provider == provider)
            .ok_or_else(|| format!("this executor has no {} client for family connections", provider.label()))
    }

    /// Starts connecting `member`'s account: a PKCE verifier and a `state`, kept for ten minutes. Returns the
    /// authorization URL and the state.
    pub fn begin(&self, provider: OAuthProvider, member: &str, redirect_uri: &str) -> Result<(String, String), String> {
        let client = self.client(provider)?;
        let verifier = Zeroizing::new(random_b64(32)?);
        let challenge = data_encoding::BASE64URL_NOPAD
            .encode(ring::digest::digest(&ring::digest::SHA256, verifier.as_bytes()).as_ref());
        let state = random_b64(32)?;
        let pending = Pending {
            member: member.to_owned(),
            provider,
            verifier: verifier.to_string(),
            redirect_uri: redirect_uri.to_owned(),
            at: crate::now(),
        };
        let text = Zeroizing::new(serde_json::to_vec(&pending).map_err(|e| e.to_string())?);
        crate::leases::write_private(&self.pending_dir.join(&state), &text)?;
        let mut url =
            reqwest::Url::parse(&client.auth_url).map_err(|_| "the authorization URL is not a URL".to_owned())?;
        {
            let mut q = url.query_pairs_mut();
            q.append_pair("response_type", "code")
                .append_pair("client_id", &client.client_id)
                .append_pair("redirect_uri", redirect_uri)
                .append_pair("state", &state)
                .append_pair("code_challenge", &challenge)
                .append_pair("code_challenge_method", "S256");
            match provider {
                OAuthProvider::Google => {
                    q.append_pair("scope", GOOGLE_SCOPES)
                        .append_pair("access_type", "offline")
                        .append_pair("prompt", "consent")
                        .append_pair("login_hint", member);
                }
                OAuthProvider::Github => {
                    q.append_pair("scope", GITHUB_SCOPES).append_pair("allow_signup", "false");
                }
            }
        }
        Ok((url.to_string(), state))
    }

    /// Redeems `code` for `state` with its kept verifier and this executor's client secret, keeps the member's token,
    /// and returns the account. The job's member and provider must be the ones the state was made for.
    pub async fn finish(
        &self,
        provider: OAuthProvider,
        member: &str,
        code: &str,
        state: &str,
    ) -> Result<String, String> {
        if !state_ok(state) {
            return Err("not a state this executor made".to_owned());
        }
        let path = self.pending_dir.join(state);
        let text =
            Zeroizing::new(std::fs::read(&path).map_err(|_| "no connection is waiting for that state".to_owned())?);
        // Once, whatever happens next.
        if let Err(e) = std::fs::remove_file(&path) {
            log::warn!("removing a pending connection: {e}");
        }
        let pending: Pending = serde_json::from_slice(&text).map_err(|_| "a damaged pending connection".to_owned())?;
        let verifier = Zeroizing::new(pending.verifier.clone());
        if pending.provider != provider || !pending.member.eq_ignore_ascii_case(member) {
            return Err("that state was made for another connection".to_owned());
        }
        if crate::now() - pending.at > PENDING_SECS {
            return Err("the connection took too long; start again".to_owned());
        }
        let client = self.client(provider)?;
        let resp = self
            .http
            .post(&client.token_url)
            .header("Accept", "application/json")
            .form(&[
                ("grant_type", "authorization_code"),
                ("code", code),
                ("redirect_uri", pending.redirect_uri.as_str()),
                ("client_id", client.client_id.as_str()),
                ("client_secret", client.client_secret.as_str()),
                ("code_verifier", verifier.as_str()),
            ])
            .send()
            .await
            .map_err(|e| format!("{}: {}", provider.label(), e.without_url()))?;
        if !resp.status().is_success() {
            return Err(format!("{} refused the code ({})", provider.label(), resp.status()));
        }
        let v: Value = resp.json().await.map_err(|_| "the token answer is not JSON".to_owned())?;
        let field = |k: &str| v.get(k).and_then(Value::as_str).map(|t| Zeroizing::new(t.to_owned()));
        let (keep, access) = match provider {
            OAuthProvider::Google => (
                field("refresh_token").ok_or("Google gave no refresh token (consent was not offline)")?,
                field("access_token").ok_or("Google gave no access token")?,
            ),
            OAuthProvider::Github => {
                let t = field("access_token").ok_or("GitHub gave no token")?;
                (t.clone(), t)
            }
        };
        let account = self.account(client, &access).await?;
        self.tokens.put(
            provider,
            member,
            UserToken {
                token: keep.to_string(),
                account: account.clone(),
                at: crate::now(),
            },
        )?;
        Ok(account)
    }

    /// Who the connected account is: Google's address, GitHub's login.
    async fn account(&self, client: &Client, access: &str) -> Result<String, String> {
        let (url, key) = match client.provider {
            OAuthProvider::Google => (client.userinfo_url.clone(), "email"),
            OAuthProvider::Github => (format!("{}/user", client.api_base), "login"),
        };
        let resp = self
            .http
            .get(url)
            .bearer_auth(access)
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| format!("{}: {}", client.provider.label(), e.without_url()))?;
        let v: Value = resp.json().await.unwrap_or(Value::Null);
        v.get(key)
            .and_then(Value::as_str)
            .map(|s| s.chars().take(254).collect())
            .ok_or_else(|| format!("{} didn't say whose account it is", client.provider.label()))
    }

    /// Revokes `member`'s grant upstream and forgets it. Nothing kept is nothing to revoke.
    pub async fn revoke(&self, provider: OAuthProvider, member: &str) -> Result<(), String> {
        let Some(held) = self.tokens.get(provider, member) else {
            return Ok(());
        };
        let client = self.client(provider)?;
        let resp = match provider {
            OAuthProvider::Google => {
                self.http.post(&client.revoke_url).form(&[("token", held.token.as_str())]).send().await
            }
            OAuthProvider::Github => {
                self.http
                    .delete(format!("{}/applications/{}/grant", client.api_base, client.client_id))
                    .basic_auth(&client.client_id, Some(client.client_secret.as_str()))
                    .header("Accept", "application/vnd.github+json")
                    .json(&serde_json::json!({"access_token": held.token}))
                    .send()
                    .await
            }
        }
        .map_err(|e| format!("{}: {}", provider.label(), e.without_url()))?;
        // Already gone upstream (400 from Google, 404/422 from GitHub) is revoked too.
        if !(resp.status().is_success() || matches!(resp.status().as_u16(), 400 | 404 | 422)) {
            return Err(format!("{} refused the revocation ({})", provider.label(), resp.status()));
        }
        self.tokens.remove(provider, member)?;
        Ok(())
    }

    /// An access token of `member`'s own Google account (refreshed now).
    async fn google_access(&self, member: &str) -> Result<Zeroizing<String>, String> {
        let held = self
            .tokens
            .get(OAuthProvider::Google, member)
            .ok_or("this family member has no Google account connected")?;
        let client = self.client(OAuthProvider::Google)?;
        let resp = self
            .http
            .post(&client.token_url)
            .form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", held.token.as_str()),
                ("client_id", client.client_id.as_str()),
                ("client_secret", client.client_secret.as_str()),
            ])
            .send()
            .await
            .map_err(|e| format!("Google: {}", e.without_url()))?;
        if !resp.status().is_success() {
            return Err(format!("Google refused the member's grant ({}): connect it again", resp.status()));
        }
        let v: Value = resp.json().await.map_err(|_| "Google's token answer is not JSON".to_owned())?;
        v.get("access_token")
            .and_then(Value::as_str)
            .map(|t| Zeroizing::new(t.to_owned()))
            .ok_or_else(|| "Google's token answer has no access token".to_owned())
    }
}

#[async_trait::async_trait]
impl TokenSource for UserOAuth {
    async fn token(&self, member: &str, _scope: &str) -> Result<Zeroizing<String>, String> {
        self.google_access(member).await
    }
}

#[async_trait::async_trait]
impl Connector for UserOAuth {
    fn handles(&self, tool: &str) -> bool {
        self.clients.iter().any(|c| c.provider.tools().contains(&tool))
    }

    async fn run(&self, member_email: &str, tool: &str, args: &Value) -> Result<Value, String> {
        if OAuthProvider::Google.tools().contains(&tool) {
            let client = self.client(OAuthProvider::Google)?;
            return Api {
                http: &self.http,
                api_base: &client.api_base,
                tokens: self,
            }
            .run(member_email, tool, args)
            .await;
        }
        let client = self.client(OAuthProvider::Github)?;
        let held = self
            .tokens
            .get(OAuthProvider::Github, member_email)
            .ok_or("this family member has no GitHub account connected")?;
        let call = plan(tool, args)?;
        call_as(&self.http, &client.api_base, &call, &held.token).await.map(|a| minimal(tool, &a))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wiremock::matchers::{body_string_contains, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    struct World {
        server: MockServer,
        oauth: UserOAuth,
        tokens: Arc<UserTokens>,
        _dir: tempfile::TempDir,
    }

    async fn world() -> World {
        let server = MockServer::start().await;
        let dir = tempfile::tempdir().unwrap();
        let tokens = Arc::new(UserTokens::open(dir.path(), b"executor key").unwrap());
        let clients = vec![
            Client::google("gid", Zeroizing::new("gsecret".into())).at(&server.uri()),
            Client::github("hid", Zeroizing::new("hsecret".into())).at(&server.uri()),
        ];
        let oauth = UserOAuth::new(clients, Arc::clone(&tokens), dir.path()).unwrap();
        World {
            server,
            oauth,
            tokens,
            _dir: dir,
        }
    }

    fn query(url: &str, key: &str) -> String {
        reqwest::Url::parse(url).unwrap().query_pairs().find(|(k, _)| k == key).unwrap().1.into_owned()
    }

    async fn google_token_mocks(server: &MockServer) {
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(body_string_contains("grant_type=authorization_code"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"access_token": "ya29.first", "refresh_token": "1//refresh"})),
            )
            .mount(server)
            .await;
        Mock::given(method("GET"))
            .and(path("/userinfo"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"email": "sam.kid@gmail.com"})))
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn connect_exchanges_the_code_with_its_own_verifier() {
        let w = world().await;
        google_token_mocks(&w.server).await;
        let (url, state) = w.oauth.begin(OAuthProvider::Google, "sam@example.com", "https://reins.example/cb").unwrap();
        assert_eq!(query(&url, "state"), state);
        assert_eq!(query(&url, "code_challenge_method"), "S256");
        // Another member's job can't use this state.
        assert!(w.oauth.finish(OAuthProvider::Google, "ann@example.com", "CODE", &state).await.is_err());
        let (url, state) = w.oauth.begin(OAuthProvider::Google, "sam@example.com", "https://reins.example/cb").unwrap();
        let account = w.oauth.finish(OAuthProvider::Google, "sam@example.com", "CODE", &state).await.unwrap();
        assert_eq!(account, "sam.kid@gmail.com");
        // The exchange carried the verifier whose hash the URL committed to, and the client secret.
        let reqs = w.server.received_requests().await.unwrap();
        let exchange = reqs.iter().find(|r| r.url.path() == "/token").unwrap();
        let form: Vec<(String, String)> =
            reqwest::Url::parse(&format!("http://x/?{}", String::from_utf8_lossy(&exchange.body)))
                .unwrap()
                .query_pairs()
                .into_owned()
                .collect();
        let get = |k: &str| form.iter().find(|(a, _)| a == k).map(|(_, v)| v.clone()).unwrap();
        assert_eq!(get("code"), "CODE");
        assert_eq!(get("client_secret"), "gsecret");
        let verifier = get("code_verifier");
        assert_eq!(
            data_encoding::BASE64URL_NOPAD
                .encode(ring::digest::digest(&ring::digest::SHA256, verifier.as_bytes()).as_ref()),
            query(&url, "code_challenge"),
        );
        assert_eq!(w.tokens.get(OAuthProvider::Google, "sam@example.com").unwrap().token, "1//refresh");
        // A state works once.
        assert!(w.oauth.finish(OAuthProvider::Google, "sam@example.com", "CODE", &state).await.is_err());
        assert!(w.oauth.finish(OAuthProvider::Google, "sam@example.com", "CODE", "../../etc/passwd").await.is_err());
    }

    #[tokio::test]
    async fn a_token_serves_only_its_member() {
        let w = world().await;
        w.tokens
            .put(
                OAuthProvider::Google,
                "sam@example.com",
                UserToken {
                    token: "1//sam".into(),
                    account: "sam.kid@gmail.com".into(),
                    at: 1,
                },
            )
            .unwrap();
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(body_string_contains("refresh_token=1%2F%2Fsam"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"access_token": "ya29.sam"})))
            .mount(&w.server)
            .await;
        Mock::given(method("POST"))
            .and(path("/gmail/v1/users/me/messages/send"))
            .and(header("authorization", "Bearer ya29.sam"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "m1", "threadId": "t1"})))
            .mount(&w.server)
            .await;
        let args = json!({"to": ["gran@example.com"], "subject": "Hi", "body": "Hello"});
        let sent = w.oauth.run("sam@example.com", "gmail_send", &args).await.unwrap();
        assert_eq!(sent["id"], "m1");
        // Ann has nothing connected: nothing of Sam's is used for her.
        let err = w.oauth.run("ann@example.com", "gmail_send", &args).await.unwrap_err();
        assert!(err.contains("no Google account"), "{err}");
        assert!(
            w.oauth.run("ann@example.com", "github_create_issue", &json!({"repo": "a/b", "title": "x"})).await.is_err()
        );
    }

    #[tokio::test]
    async fn revoking_revokes_upstream_and_forgets() {
        let w = world().await;
        for (p, t) in [(OAuthProvider::Google, "1//g"), (OAuthProvider::Github, "gho_h")] {
            w.tokens
                .put(
                    p,
                    "sam@example.com",
                    UserToken {
                        token: t.into(),
                        account: "sam".into(),
                        at: 1,
                    },
                )
                .unwrap();
        }
        Mock::given(method("POST"))
            .and(path("/revoke"))
            .and(body_string_contains("token=1%2F%2Fg"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&w.server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/applications/hid/grant"))
            .and(body_string_contains("gho_h"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&w.server)
            .await;
        w.oauth.revoke(OAuthProvider::Google, "sam@example.com").await.unwrap();
        w.oauth.revoke(OAuthProvider::Github, "sam@example.com").await.unwrap();
        assert!(w.tokens.get(OAuthProvider::Google, "sam@example.com").is_none());
        assert!(w.tokens.get(OAuthProvider::Github, "sam@example.com").is_none());
        // Nothing left: a second revocation calls nobody.
        w.oauth.revoke(OAuthProvider::Google, "sam@example.com").await.unwrap();
    }
}
