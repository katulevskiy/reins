//! Signing in to X: OAuth 2.0 Authorization Code with PKCE, as a public client (no client secret on the phone)
//! (<https://docs.x.com/resources/fundamentals/authentication/oauth-2-0/authorization-code>).
//!
//! The phone opens [`XApi::begin`]'s address in the browser; X sends the browser back to [`REDIRECT_URI`] with a code,
//! and [`XApi::finish`] trades it (with the PKCE verifier) for an access token, valid two hours, and a refresh token
//! (`offline.access`). Both are kept sealed in the phone's store, never logged and never shown.

use std::collections::HashMap;
use std::sync::PoisonError;

use reins_proto::connector::X;
use reins_proto::connector::x::SCOPES;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use zeroize::{Zeroize, Zeroizing};

use super::XApi;
use crate::mcp::oauth::{code_challenge, random_token};
use crate::store::unix_now;
use crate::{CoreError, text};

/// Where X sends the browser back to: registered as the callback URL of the X developer app (`docs/connectors/x.md`).
pub const REDIRECT_URI: &str = "com.reins2fa.app://x-oauth";
/// A sign-in left this long unanswered is started again (X's authorization codes themselves last 30 seconds).
const SIGN_IN_TTL_SECS: i64 = 600;
/// Tokens are refreshed this long before they run out.
const REFRESH_EARLY_SECS: i64 = 60;

/// The account's tokens, as kept in the store (JSON). Wiped from memory when dropped; never printed.
#[derive(Clone, Serialize, Deserialize)]
pub struct Tokens {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    /// Unix seconds.
    pub expires_at: i64,
    /// The account's numeric id, which every `/2/users/{id}/…` call needs.
    pub user_id: String,
    pub username: String,
}

impl std::fmt::Debug for Tokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tokens")
            .field("access_token", &"<redacted>")
            .field("refresh_token", &self.refresh_token.as_ref().map(|_| "<redacted>"))
            .field("expires_at", &self.expires_at)
            .field("user_id", &self.user_id)
            .field("username", &self.username)
            .finish()
    }
}

impl Drop for Tokens {
    fn drop(&mut self) {
        self.access_token.zeroize();
        if let Some(refresh) = &mut self.refresh_token {
            refresh.zeroize();
        }
    }
}

/// A sign-in started on this phone and not answered yet.
pub(super) struct Pending {
    verifier: String,
    state: String,
    started: i64,
}

impl std::fmt::Debug for Pending {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pending").field("started", &self.started).finish_non_exhaustive()
    }
}

impl Drop for Pending {
    fn drop(&mut self) {
        self.verifier.zeroize();
        self.state.zeroize();
    }
}

/// X's answer to a token request (<https://docs.x.com/resources/fundamentals/authentication/oauth-2-0/authorization-code>).
#[derive(Deserialize)]
struct TokenReply {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
}

impl Drop for TokenReply {
    fn drop(&mut self) {
        self.access_token.zeroize();
        if let Some(refresh) = &mut self.refresh_token {
            refresh.zeroize();
        }
    }
}

fn clean(s: &str) -> String {
    text::truncate_chars(&text::one_line(&text::neutralize(s)), 200)
}

fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

impl XApi {
    fn client_id(&self) -> Result<&str, CoreError> {
        self.client_id.as_deref().filter(|id| !id.is_empty()).ok_or_else(|| CoreError::invalid(super::NO_CLIENT_ID))
    }

    /// Step 1: the address to open in the browser, where the user signs in to X and allows Reins.
    pub(super) fn begin(&self) -> Result<String, CoreError> {
        let client_id = self.client_id()?;
        let verifier = random_token(48)?;
        let state = random_token(24)?;
        let mut url = url::Url::parse(&self.authorize_url).map_err(|_| CoreError::storage("bad X sign-in address"))?;
        url.query_pairs_mut()
            .append_pair("response_type", "code")
            .append_pair("client_id", client_id)
            .append_pair("redirect_uri", REDIRECT_URI)
            .append_pair("scope", &SCOPES.join(" "))
            .append_pair("state", &state)
            .append_pair("code_challenge", &code_challenge(&verifier))
            .append_pair("code_challenge_method", "S256");
        *self.pending.lock().unwrap_or_else(PoisonError::into_inner) = Some(Pending {
            verifier,
            state,
            started: unix_now(),
        });
        Ok(url.into())
    }

    /// Step 2: the address X sent the browser back to. Returns the account (the @username, lowercase).
    pub(super) async fn finish(&self, callback: &str) -> Result<String, CoreError> {
        let client_id = self.client_id()?.to_owned();
        let callback = callback.trim();
        let url = url::Url::parse(callback)
            .ok()
            .filter(|_| callback.starts_with(REDIRECT_URI))
            .ok_or_else(|| CoreError::invalid("that is not the address X sent back"))?;
        let answer: HashMap<String, Zeroizing<String>> =
            url.query_pairs().map(|(k, v)| (k.into_owned(), Zeroizing::new(v.into_owned()))).collect();
        let pending = self
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
            .ok_or_else(|| CoreError::invalid("start the X sign-in again"))?;
        if !answer.get("state").is_some_and(|s| same(s, &pending.state)) {
            return Err(CoreError::invalid("this X sign-in answer is not for this sign-in; start again"));
        }
        if let Some(error) = answer.get("error") {
            return Err(CoreError::invalid(format!("X did not allow it: {}", clean(error))));
        }
        if unix_now() - pending.started > SIGN_IN_TTL_SECS {
            return Err(CoreError::invalid("the X sign-in took too long; start again"));
        }
        let code = answer
            .get("code")
            .filter(|c| !c.is_empty())
            .ok_or_else(|| CoreError::invalid("X sent back no sign-in code"))?;
        let reply = self
            .token_request(&[
                ("grant_type", "authorization_code"),
                ("code", code),
                ("redirect_uri", REDIRECT_URI),
                ("code_verifier", &pending.verifier),
                ("client_id", &client_id),
            ])
            .await?;
        let me = self.send(&reply.access_token, reqwest::Method::GET, "/2/users/me", &[], None).await?;
        let user_id = me["data"]["id"].as_str().unwrap_or_default().to_owned();
        let username = me["data"]["username"].as_str().unwrap_or_default().to_lowercase();
        if user_id.is_empty() || !super::is_username(&username) {
            return Err(CoreError::service("X did not say who signed in"));
        }
        let tokens = Self::tokens_of(&reply, None, user_id, username.clone());
        self.save(&username, &tokens)?;
        Ok(username)
    }

    fn tokens_of(reply: &TokenReply, old_refresh: Option<&str>, user_id: String, username: String) -> Tokens {
        Tokens {
            access_token: reply.access_token.clone(),
            refresh_token: reply.refresh_token.clone().or_else(|| old_refresh.map(str::to_owned)),
            expires_at: unix_now() + reply.expires_in.unwrap_or(7_200).max(0),
            user_id,
            username,
        }
    }

    /// `POST /2/oauth2/token`, form-encoded; a public client names itself with `client_id`.
    async fn token_request(&self, form: &[(&str, &str)]) -> Result<TokenReply, CoreError> {
        let response = self
            .http
            .post(format!("{}/2/oauth2/token", self.base))
            .form(form)
            .send()
            .await
            .map_err(|e| CoreError::service(format!("X could not be reached: {}", e.without_url())))?;
        let status = response.status();
        log::debug!("x: token request answered {status}");
        let body = Zeroizing::new(response.text().await.unwrap_or_default());
        if !status.is_success() {
            let json: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
            let what = json["error_description"].as_str().or_else(|| json["error"].as_str()).unwrap_or("refused");
            let message = format!("X refused the sign-in ({status}): {}", clean(what));
            return Err(if status.is_client_error() {
                CoreError::needs_attention(message)
            } else {
                CoreError::service(message)
            });
        }
        serde_json::from_str(&body).map_err(|_| CoreError::service("X's sign-in answer was not understood"))
    }

    pub(super) fn load(&self, account: &str) -> Result<Tokens, CoreError> {
        let raw = Zeroizing::new(
            self.store.secret_get(X, account)?.ok_or_else(|| CoreError::needs_attention("X is not connected"))?,
        );
        serde_json::from_slice(&raw).map_err(|_| CoreError::storage("corrupt X tokens"))
    }

    pub(super) fn save(&self, account: &str, tokens: &Tokens) -> Result<(), CoreError> {
        let raw = Zeroizing::new(serde_json::to_vec(tokens).map_err(|_| CoreError::storage("X tokens"))?);
        self.store.secret_put(X, account, &raw)
    }

    /// The account's tokens, refreshed first when they are about to run out.
    pub(super) async fn tokens(&self, account: &str) -> Result<Tokens, CoreError> {
        let tokens = self.load(account)?;
        if tokens.expires_at - REFRESH_EARLY_SECS > unix_now() {
            return Ok(tokens);
        }
        self.refresh(account, &tokens).await
    }

    /// New tokens for `stale` (X hands out a new refresh token each time). One refresh at a time: a caller that
    /// waited finds the tokens already renewed and uses them.
    pub(super) async fn refresh(&self, account: &str, stale: &Tokens) -> Result<Tokens, CoreError> {
        let _one_at_a_time = self.refreshing.lock().await;
        let current = self.load(account)?;
        if !same(&current.access_token, &stale.access_token) && current.expires_at - REFRESH_EARLY_SECS > unix_now() {
            return Ok(current);
        }
        let refresh = current
            .refresh_token
            .clone()
            .map(Zeroizing::new)
            .ok_or_else(|| CoreError::needs_attention("X needs the account signed in again"))?;
        let client_id = self.client_id()?.to_owned();
        let reply = self
            .token_request(&[("grant_type", "refresh_token"), ("refresh_token", &refresh), ("client_id", &client_id)])
            .await
            .map_err(|e| match e {
                CoreError::ServiceNeedsAttention {
                    ..
                } => CoreError::needs_attention("X needs the account signed in again"),
                other => other,
            })?;
        let tokens = Self::tokens_of(&reply, Some(&refresh), current.user_id.clone(), current.username.clone());
        self.save(account, &tokens)?;
        Ok(tokens)
    }
}
