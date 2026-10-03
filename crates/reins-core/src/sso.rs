//! Signing in without a password, and the vault that needs none.
//!
//! **Sign-in.** The server's SSO (Vaultwarden's flow; passkeys through WorkOS AuthKit on app.reins2fa.com) runs in the
//! system browser: the app opens [`begin`]'s URL in ASWebAuthenticationSession (iOS) or a
//! Custom Tab (Android), the server sends the browser on to the identity provider and back, and finally to
//! [`REDIRECT_URI`] with `code` and `state`. The app hands that URL to `sso_finish`, which exchanges the code at
//! `/identity/connect/token` (`grant_type=authorization_code`, PKCE: the server forwards the challenge to the provider,
//! so the verifier only ever leaves this phone for the server's token endpoint).
//!
//! **Keys.** A new SSO account has no master password and so no vault keys. The phone makes them, exactly as a
//! Bitwarden client does for a master password, with a random 256-bit *account secret* in the password's place
//! (`/api/accounts/set-password`). The secret stays in this phone's store (sealed with the key the OS keystore
//! wraps); it is never typed. Its recovery code (the secret in base32, in groups of four) must be recorded in setup,
//! and a new phone gets the secret from this one or from that code.
//!
//! The secret derives the master key with PBKDF2-SHA256 over a fixed salt ([`SECRET_SALT`], [`SECRET_KDF`]) instead
//! of the email: WorkOS may change the account's email (`api::reins::workos_sync` on the server), and a salt that
//! follows the email would lock the vault. The secret has 256 bits of entropy, so the iterations only satisfy the
//! server's floor. A Bitwarden client cannot open such a vault with the code; Reins can.

use std::fmt;

use data_encoding::{BASE32_NOPAD, BASE64URL_NOPAD};
use reqwest::header::ACCEPT;
use ring::digest;
use serde::Deserialize;
use url::Url;
use zeroize::Zeroizing;

use crate::CoreError;
use crate::crypto::{self, Kdf, MasterKey, NewAccountKeys, VaultKey, master_key, master_password_hash, random_bytes};
use crate::http::{ServerUrl, error_text};
use crate::vault::{DEVICE_NAME, DEVICE_TYPE_ANDROID, Tokens};

/// Where the server sends the browser at the end (`client_id=mobile`; the server allows exactly this one).
pub const REDIRECT_URI: &str = "com.reins2fa.app://sso-callback";
/// The URL scheme the app's browser session waits for.
pub const CALLBACK_SCHEME: &str = "com.reins2fa.app";
const CLIENT_ID: &str = "mobile";
const SCOPE: &str = "api offline_access";

/// What the account secret's master key is salted with (in place of the email).
pub const SECRET_SALT: &str = "reins-account-secret-v1";
/// The account secret's KDF: the server's floor for PBKDF2 (the secret itself has 256 bits of entropy).
pub const SECRET_KDF: Kdf = Kdf::Pbkdf2 {
    iterations: 100_000,
};
/// The store's service name for the account secret (account: the server's user id).
pub const SECRET_SERVICE: &str = "reins.account-secret";

/// One sign-in in the browser: the URL to open and what `finish` checks the answer against. The app keeps `state`
/// and `verifier` until the browser comes back (Android may restart the app meanwhile).
pub struct Start {
    pub url: String,
    pub state: String,
    pub verifier: Zeroizing<String>,
}

impl fmt::Debug for Start {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Start").field("url", &self.url).finish_non_exhaustive()
    }
}

/// The authorize URL for a sign-in at `server`, with a fresh state and PKCE verifier.
pub fn begin(server: &ServerUrl) -> Result<Start, CoreError> {
    let state = BASE64URL_NOPAD.encode(&random_bytes::<16>()?);
    let verifier = Zeroizing::new(BASE64URL_NOPAD.encode(&random_bytes::<32>()?));
    let challenge = BASE64URL_NOPAD.encode(digest::digest(&digest::SHA256, verifier.as_bytes()).as_ref());
    let mut url = Url::parse(&server.join("/identity/connect/authorize"))
        .map_err(|_| CoreError::invalid("server URL is not a valid URL"))?;
    url.query_pairs_mut()
        .append_pair("client_id", CLIENT_ID)
        .append_pair("redirect_uri", REDIRECT_URI)
        .append_pair("response_type", "code")
        .append_pair("response_mode", "query")
        .append_pair("scope", SCOPE)
        .append_pair("state", &state)
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256");
    Ok(Start {
        url: url.into(),
        state,
        verifier,
    })
}

/// The code in the URL the browser came back with, after checking that it answers the sign-in that sent `state`.
pub fn callback_code(callback: &str, state: &str) -> Result<Zeroizing<String>, CoreError> {
    let bad = || CoreError::invalid("The sign-in did not come back as expected. Try again.");
    let url = Url::parse(callback.trim()).map_err(|_| bad())?;
    if url.scheme() != CALLBACK_SCHEME || url.host_str() != Some("sso-callback") {
        return Err(bad());
    }
    let mut code = None;
    let mut got_state = None;
    let mut error = None;
    for (k, v) in url.query_pairs() {
        match k.as_ref() {
            "code" => code = Some(Zeroizing::new(v.into_owned())),
            "state" => got_state = Some(v.into_owned()),
            "error" => error = Some(v.into_owned()),
            _ => {}
        }
    }
    if let Some(error) = error {
        return Err(CoreError::invalid(format!("The sign-in was refused: {error}")));
    }
    if got_state.as_deref() != Some(state) {
        return Err(bad());
    }
    code.filter(|c| !c.is_empty()).ok_or_else(bad)
}

/// What `/identity/connect/token` answered to the code: the tokens, the account's wrapped user key (none yet for a
/// new account) and who signed in.
pub struct SignedIn {
    pub tokens: Tokens,
    /// `profile.key`: the user key wrapped with the master key; `None` before the account has keys.
    pub key: Option<String>,
    pub email: String,
    /// The server's id of the account.
    pub user_id: String,
}

impl fmt::Debug for SignedIn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SignedIn").field("email", &self.email).field("user_id", &self.user_id).finish_non_exhaustive()
    }
}

#[derive(Deserialize)]
struct TokenAnswer {
    access_token: String,
    refresh_token: String,
    expires_in: i64,
    #[serde(rename = "Key", default)]
    key: Option<String>,
}

#[derive(Deserialize)]
struct AccessClaims {
    sub: String,
    email: String,
}

/// The claims of an access token from the server (read, not verified: it came from the server over TLS).
pub fn access_claims(token: &str) -> Result<(String, String), CoreError> {
    let bad = || CoreError::Network {
        reason: "invalid token response".to_owned(),
    };
    let payload = token.split('.').nth(1).ok_or_else(bad)?;
    let bytes = BASE64URL_NOPAD.decode(payload.trim_end_matches('=').as_bytes()).map_err(|_| bad())?;
    let claims: AccessClaims = serde_json::from_slice(&bytes).map_err(|_| bad())?;
    Ok((claims.sub, claims.email.to_lowercase()))
}

/// Exchanges the code for this device's tokens.
pub async fn exchange(
    http: &reqwest::Client,
    server: &ServerUrl,
    code: &str,
    verifier: &str,
    device_id: &str,
) -> Result<SignedIn, CoreError> {
    let form = [
        ("grant_type", "authorization_code"),
        ("code", code),
        ("code_verifier", verifier),
        ("redirect_uri", REDIRECT_URI),
        ("client_id", CLIENT_ID),
        ("scope", SCOPE),
        ("deviceType", DEVICE_TYPE_ANDROID),
        ("deviceIdentifier", device_id),
        ("deviceName", DEVICE_NAME),
    ];
    let resp = http
        .post(server.join("/identity/connect/token"))
        .header("Device-Type", DEVICE_TYPE_ANDROID)
        .form(&form)
        .send()
        .await?;
    let status = resp.status();
    let body = Zeroizing::new(resp.text().await?);
    if !status.is_success() {
        if body.contains("TwoFactorProviders") {
            return Err(CoreError::UnsupportedTwoFactor);
        }
        let (_, message) = error_text(status, &body);
        return Err(if status.is_client_error() {
            CoreError::invalid(format!("The server did not sign you in: {message}"))
        } else {
            CoreError::Server {
                status: status.as_u16(),
                reason: message,
            }
        });
    }
    let answer: TokenAnswer = serde_json::from_str(&body).map_err(|_| CoreError::Network {
        reason: "invalid token response".to_owned(),
    })?;
    let (user_id, email) = access_claims(&answer.access_token)?;
    Ok(SignedIn {
        tokens: Tokens {
            access_token: Zeroizing::new(answer.access_token),
            refresh_token: Zeroizing::new(answer.refresh_token),
            expires_in: answer.expires_in,
        },
        key: answer.key.filter(|k| !k.is_empty()),
        email,
        user_id,
    })
}

/// The 256-bit secret that stands in for a master password.
pub struct AccountSecret(Zeroizing<[u8; 32]>);

impl fmt::Debug for AccountSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AccountSecret(<redacted>)")
    }
}

impl AccountSecret {
    pub fn generate() -> Result<Self, CoreError> {
        Ok(Self(Zeroizing::new(random_bytes::<32>()?)))
    }

    pub fn from_bytes(raw: &[u8]) -> Result<Self, CoreError> {
        let bytes = <[u8; 32]>::try_from(raw).map_err(|_| CoreError::invalid("not an account secret"))?;
        Ok(Self(Zeroizing::new(bytes)))
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0[..]
    }

    /// From a recovery code as people type it: case, spaces and dashes do not count. `None` when it is not one.
    pub fn from_recovery_code(code: &str) -> Option<Self> {
        let clean: Zeroizing<String> =
            Zeroizing::new(code.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_uppercase()).collect());
        if clean.len() != 52 {
            return None;
        }
        let bytes = Zeroizing::new(BASE32_NOPAD.decode(clean.as_bytes()).ok()?);
        Self::from_bytes(&bytes).ok()
    }

    /// The secret in base32, in thirteen groups of four (`ABCD-EFGH-...`).
    pub fn recovery_code(&self) -> Zeroizing<String> {
        let plain = self.password();
        let mut out = Zeroizing::new(String::with_capacity(64));
        for (i, c) in plain.chars().enumerate() {
            if i > 0 && i % 4 == 0 {
                out.push('-');
            }
            out.push(c);
        }
        out
    }

    /// What goes through the KDF in the master password's place: the code without dashes.
    fn password(&self) -> Zeroizing<String> {
        Zeroizing::new(BASE32_NOPAD.encode(&self.0[..]))
    }

    /// The master key (CPU-heavy: call from `spawn_blocking`).
    pub fn master_key(&self) -> Result<MasterKey, CoreError> {
        master_key(&self.password(), SECRET_SALT, SECRET_KDF)
    }

    /// The hash the server stores for this "master password".
    pub fn master_password_hash(&self, master: &MasterKey) -> Zeroizing<String> {
        master_password_hash(master, &self.password())
    }

    /// The user key, unwrapped from the account's `profile.key` (CPU-heavy). An error means this is not the
    /// account's secret.
    pub fn open_user_key(&self, wrapped: &str) -> Result<VaultKey, CoreError> {
        let master = self.master_key()?;
        let raw = master
            .stretch()
            .decrypt(wrapped)
            .map_err(|_| CoreError::invalid("That recovery code does not open this account."))?;
        VaultKey::from_bytes(&raw)
    }
}

/// A new account's keys with `secret` in the master password's place: a random user key wrapped with the secret's
/// master key, and an RSA-2048 key pair whose private half is wrapped with the user key, as the Bitwarden clients
/// make them ([`crypto::new_account_keys`]). CPU-heavy: call from `spawn_blocking`.
pub fn new_keys(secret: &AccountSecret) -> Result<NewAccountKeys, CoreError> {
    crypto::new_account_keys(&secret.password(), SECRET_SALT, SECRET_KDF)
}

/// Gives the signed-in account its keys (`/api/accounts/set-password`, the call Bitwarden clients make after a
/// first SSO sign-in). The server refuses an account that has keys already.
pub async fn set_keys(
    http: &reqwest::Client,
    server: &ServerUrl,
    access_token: &str,
    keys: &NewAccountKeys,
) -> Result<(), CoreError> {
    let Kdf::Pbkdf2 {
        iterations,
    } = SECRET_KDF
    else {
        unreachable!("the account secret's KDF is PBKDF2")
    };
    let body = serde_json::json!({
        "masterPasswordHash": keys.master_password_hash.as_str(),
        "masterPasswordHint": null,
        "key": keys.key,
        "keys": {"publicKey": keys.public_key, "encryptedPrivateKey": keys.encrypted_private_key},
        "kdf": 0,
        "kdfIterations": iterations,
        "kdfMemory": null,
        "kdfParallelism": null,
        "orgIdentifier": null,
    });
    let resp = http
        .post(server.join("/api/accounts/set-password"))
        .bearer_auth(access_token)
        .header(ACCEPT, "application/json")
        .json(&body)
        .send()
        .await?;
    let status = resp.status();
    if status.is_success() {
        return Ok(());
    }
    let (_, message) = error_text(status, &resp.text().await?);
    Err(CoreError::Server {
        status: status.as_u16(),
        reason: message,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authorize_urls_carry_pkce_and_the_app_redirect() {
        let server = ServerUrl::parse("https://app.reins2fa.com").unwrap();
        let start = begin(&server).unwrap();
        let url = Url::parse(&start.url).unwrap();
        assert_eq!(url.path(), "/identity/connect/authorize");
        let q: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(q["client_id"], "mobile");
        assert_eq!(q["redirect_uri"], REDIRECT_URI);
        assert_eq!(q["state"], start.state);
        assert_eq!(q["code_challenge_method"], "S256");
        let expected = BASE64URL_NOPAD.encode(digest::digest(&digest::SHA256, start.verifier.as_bytes()).as_ref());
        assert_eq!(q["code_challenge"], expected);
        assert_eq!(start.verifier.len(), 43, "RFC 7636 allows 43 to 128 characters");
        assert_ne!(begin(&server).unwrap().state, start.state);
    }

    #[test]
    fn callbacks_must_answer_this_sign_in() {
        let ok = "com.reins2fa.app://sso-callback?code=CODE-1&state=S1&scope=api%20offline_access&iss=x";
        assert_eq!(callback_code(ok, "S1").unwrap().as_str(), "CODE-1");
        assert!(callback_code(ok, "S2").is_err());
        assert!(callback_code("bitwarden://sso-callback?code=C&state=S1", "S1").is_err());
        assert!(callback_code("com.reins2fa.app://other?code=C&state=S1", "S1").is_err());
        assert!(callback_code("com.reins2fa.app://sso-callback?state=S1", "S1").is_err());
        let refused = callback_code("com.reins2fa.app://sso-callback?error=access_denied&state=S1", "S1");
        assert!(format!("{:?}", refused.unwrap_err()).contains("access_denied"));
    }

    #[test]
    fn recovery_codes_round_trip_however_they_are_typed() {
        let secret = AccountSecret::generate().unwrap();
        let code = secret.recovery_code();
        assert_eq!(code.len(), 52 + 12);
        assert_eq!(code.split('-').count(), 13);
        let typed = code.to_lowercase().replace('-', " ");
        let back = AccountSecret::from_recovery_code(&typed).unwrap();
        assert_eq!(back.as_bytes(), secret.as_bytes());
        assert!(AccountSecret::from_recovery_code(&code[..60]).is_none());
        assert!(AccountSecret::from_recovery_code("correct horse battery staple").is_none());
        assert!(!format!("{secret:?}").contains(&code[..4]));
    }

    #[test]
    fn keyless_keys_open_with_the_secret_only() {
        use rsa::pkcs8::DecodePrivateKey;

        let secret = AccountSecret::generate().unwrap();
        let keys = new_keys(&secret).unwrap();
        // The same secret (from its recovery code, on another phone) opens the user key, and the user key opens
        // the private key.
        let again = AccountSecret::from_recovery_code(&secret.recovery_code()).unwrap();
        let user = again.open_user_key(&keys.key).unwrap();
        assert_eq!(*user.to_bytes(), *keys.user_key.to_bytes());
        let private = user.decrypt(&keys.encrypted_private_key).unwrap();
        assert!(rsa::RsaPrivateKey::from_pkcs8_der(&private).is_ok());
        // The hash the server checks is the one the secret derives.
        let master = again.master_key().unwrap();
        assert_eq!(*again.master_password_hash(&master), *keys.master_password_hash);
        // Another secret opens nothing.
        let other = AccountSecret::generate().unwrap();
        assert!(other.open_user_key(&keys.key).is_err());
    }

    #[test]
    fn access_claims_name_the_account() {
        let payload = BASE64URL_NOPAD.encode(br#"{"sub":"0b5c","email":"Me@Example.com","exp":1}"#);
        assert_eq!(access_claims(&format!("h.{payload}.s")).unwrap(), ("0b5c".to_owned(), "me@example.com".to_owned()));
        assert!(access_claims("garbage").is_err());
    }
}
