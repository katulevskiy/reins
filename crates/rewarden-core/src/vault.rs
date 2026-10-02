//! Vaultwarden login (contracts §B): prelogin, master-password hash, password
//! grant with TOTP, refresh.

use std::fmt;

use serde::Deserialize;
use zeroize::Zeroizing;

use crate::CoreError;
use crate::crypto::{Kdf, NewAccountKeys, master_key, master_password_hash};
use crate::http::{ServerUrl, error_text};

/// Bitwarden device type "Android".
pub const DEVICE_TYPE_ANDROID: &str = "0";
pub const DEVICE_NAME: &str = "Rewarden";
const CLIENT_ID: &str = "mobile";
const SCOPE: &str = "api offline_access";
/// Two-factor provider id of authenticator apps (TOTP).
const TOTP_PROVIDER: &str = "0";

/// Tokens from `/identity/connect/token`. Redacted in `Debug`.
pub struct Tokens {
    pub access_token: Zeroizing<String>,
    pub refresh_token: Zeroizing<String>,
    /// Seconds.
    pub expires_in: i64,
}

impl fmt::Debug for Tokens {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Tokens").field("expires_in", &self.expires_in).finish_non_exhaustive()
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Prelogin {
    kdf: i32,
    kdf_iterations: u32,
    kdf_memory: Option<u32>,
    kdf_parallelism: Option<u32>,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: String,
    expires_in: i64,
}

#[derive(Deserialize)]
struct TwoFactorBody {
    #[serde(rename = "TwoFactorProviders")]
    providers: Vec<String>,
}

enum TokenOutcome {
    Tokens(Tokens),
    TwoFactor(Vec<String>),
    Rejected {
        status: u16,
        message: String,
    },
}

pub struct VaultClient<'a> {
    http: &'a reqwest::Client,
    server: &'a ServerUrl,
}

impl<'a> VaultClient<'a> {
    pub fn new(http: &'a reqwest::Client, server: &'a ServerUrl) -> Self {
        Self {
            http,
            server,
        }
    }

    pub async fn prelogin(&self, email: &str) -> Result<Kdf, CoreError> {
        let resp = self
            .http
            .post(self.server.join("/identity/accounts/prelogin"))
            .json(&serde_json::json!({ "email": email.trim().to_lowercase() }))
            .send()
            .await?;
        let status = resp.status();
        let body = resp.text().await?;
        if !status.is_success() {
            let (_, message) = error_text(status, &body);
            return Err(CoreError::Server {
                status: status.as_u16(),
                reason: message,
            });
        }
        let p: Prelogin = serde_json::from_str(&body).map_err(|_| CoreError::Network {
            reason: "invalid prelogin response".to_owned(),
        })?;
        Kdf::from_prelogin(p.kdf, p.kdf_iterations, p.kdf_memory, p.kdf_parallelism)
    }

    /// Creates the account `email` with these keys (made by [`crate::crypto::new_account_keys`] with `kdf`), the way current Bitwarden
    /// clients do: `register/send-verification-email` hands back a token when the server does not verify emails, and
    /// `register/finish` creates the account with it. A server that verifies emails (it answers 204), or one older than
    /// that flow (404), takes the account at `/accounts/register`, the endpoint older clients use. `email` is
    /// normalized (trimmed, lower case).
    pub async fn register(&self, email: &str, keys: &NewAccountKeys, kdf: Kdf) -> Result<(), CoreError> {
        let (kdf_type, iterations, memory, parallelism) = match kdf {
            Kdf::Pbkdf2 {
                iterations,
            } => (0, iterations, None, None),
            Kdf::Argon2id {
                iterations,
                memory_mib,
                parallelism,
            } => (1, iterations, Some(memory_mib), Some(parallelism)),
        };
        let mut body = serde_json::json!({
            "email": email,
            "name": null,
            "masterPasswordHash": keys.master_password_hash.as_str(),
            "masterPasswordHint": null,
            "key": keys.key,
            "keys": {"publicKey": keys.public_key, "encryptedPrivateKey": keys.encrypted_private_key},
            "kdf": kdf_type,
            "kdfIterations": iterations,
            "kdfMemory": memory,
            "kdfParallelism": parallelism,
        });
        let resp = self
            .http
            .post(self.server.join("/identity/accounts/register/send-verification-email"))
            .header(reqwest::header::ACCEPT, "application/json")
            .json(&serde_json::json!({"email": email, "name": null, "receiveMarketingEmails": false}))
            .send()
            .await?;
        let status = resp.status();
        let text = Zeroizing::new(resp.text().await?);
        let path = match status.as_u16() {
            200 => {
                // The token, as a JSON string (or plain text from a server that ignores `Accept`).
                let token = serde_json::from_str::<String>(&text).unwrap_or_else(|_| text.trim().to_owned());
                if token.is_empty() {
                    return Err(CoreError::Network {
                        reason: "invalid registration response".to_owned(),
                    });
                }
                body["emailVerificationToken"] = token.into();
                "/identity/accounts/register/finish"
            }
            204 | 404 | 405 => "/identity/accounts/register",
            _ => return Err(registration_refused(status, &text, false)),
        };
        let resp = self.http.post(self.server.join(path)).json(&body).send().await?;
        let status = resp.status();
        if status.is_success() {
            return Ok(());
        }
        Err(registration_refused(status, &resp.text().await?, true))
    }

    /// Full password login. `totp` is only sent when the server asks for it.
    pub async fn login(
        &self,
        email: &str,
        password: Zeroizing<String>,
        totp: Option<&str>,
        device_id: &str,
    ) -> Result<Tokens, CoreError> {
        let email = email.trim().to_lowercase();
        let totp = totp.map(normalize_totp).transpose()?;
        let kdf = self.prelogin(&email).await?;
        let hash_email = email.clone();
        let hash = tokio::task::spawn_blocking(move || -> Result<Zeroizing<String>, CoreError> {
            let key = master_key(&password, &hash_email, kdf)?;
            Ok(master_password_hash(&key, &password))
        })
        .await
        .map_err(|_| CoreError::storage("key derivation was interrupted"))??;

        let mut form: Vec<(&str, &str)> = vec![
            ("grant_type", "password"),
            ("username", &email),
            ("password", &hash),
            ("scope", SCOPE),
            ("client_id", CLIENT_ID),
            ("deviceType", DEVICE_TYPE_ANDROID),
            ("deviceIdentifier", device_id),
            ("deviceName", DEVICE_NAME),
        ];
        match self.token(&form).await? {
            TokenOutcome::Tokens(t) => Ok(t),
            TokenOutcome::TwoFactor(providers) if !providers.iter().any(|p| p == TOTP_PROVIDER) => {
                Err(CoreError::UnsupportedTwoFactor)
            }
            TokenOutcome::TwoFactor(_) => {
                let Some(code) = totp.as_deref() else {
                    return Err(CoreError::TwoFactorRequired);
                };
                form.extend([
                    ("twoFactorToken", code),
                    ("twoFactorProvider", TOTP_PROVIDER),
                    ("twoFactorRemember", "0"),
                ]);
                match self.token(&form).await? {
                    TokenOutcome::Tokens(t) => Ok(t),
                    TokenOutcome::TwoFactor(_) => Err(CoreError::InvalidCredentials),
                    TokenOutcome::Rejected {
                        status,
                        message,
                    } => Err(rejected(status, message, true)),
                }
            }
            TokenOutcome::Rejected {
                status,
                message,
            } => Err(rejected(status, message, false)),
        }
    }

    /// Exchanges a refresh token. A rejected refresh token → `NotLoggedIn`.
    pub async fn refresh(&self, refresh_token: &str) -> Result<Tokens, CoreError> {
        let form = [("grant_type", "refresh_token"), ("client_id", CLIENT_ID), ("refresh_token", refresh_token)];
        match self.token(&form).await? {
            TokenOutcome::Tokens(t) => Ok(t),
            TokenOutcome::Rejected {
                status: 400 | 401,
                ..
            }
            | TokenOutcome::TwoFactor(_) => Err(CoreError::NotLoggedIn),
            TokenOutcome::Rejected {
                status,
                message,
            } => Err(CoreError::Server {
                status,
                reason: message,
            }),
        }
    }

    async fn token(&self, form: &[(&str, &str)]) -> Result<TokenOutcome, CoreError> {
        let resp = self
            .http
            .post(self.server.join("/identity/connect/token"))
            .header("Device-Type", DEVICE_TYPE_ANDROID)
            .form(form)
            .send()
            .await?;
        let status = resp.status();
        let body = Zeroizing::new(resp.text().await?);
        if status.is_success() {
            let t: TokenResponse = serde_json::from_str(&body).map_err(|_| CoreError::Network {
                reason: "invalid token response".to_owned(),
            })?;
            return Ok(TokenOutcome::Tokens(Tokens {
                access_token: Zeroizing::new(t.access_token),
                refresh_token: Zeroizing::new(t.refresh_token),
                expires_in: t.expires_in,
            }));
        }
        if status.as_u16() == 400
            && let Ok(tf) = serde_json::from_str::<TwoFactorBody>(&body)
        {
            return Ok(TokenOutcome::TwoFactor(tf.providers));
        }
        let (_, message) = error_text(status, &body);
        Ok(TokenOutcome::Rejected {
            status: status.as_u16(),
            message,
        })
    }
}

/// Why the server did not create the account, in words for people. Vaultwarden says "Registration not allowed or user
/// already exists" for both cases: before the account step (`creating` false) only closed sign-ups refuse an email,
/// so at the account step it means the account is there already.
fn registration_refused(status: reqwest::StatusCode, body: &str, creating: bool) -> CoreError {
    let (_, message) = error_text(status, body);
    let lower = message.to_lowercase();
    if lower.contains("not allowed or user already exists") {
        return CoreError::invalid(if creating {
            "An account with this email already exists. Sign in instead."
        } else {
            "This server does not take new accounts for this email. Sign in, or ask the server's owner."
        });
    }
    if lower.contains("already exists") || lower.contains("already taken") {
        return CoreError::invalid("An account with this email already exists. Sign in instead.");
    }
    if status.as_u16() == 429 {
        return CoreError::invalid("Too many attempts. Wait a minute and try again.");
    }
    if status.is_client_error() {
        return CoreError::invalid(format!("The server did not create the account: {message}"));
    }
    CoreError::Server {
        status: status.as_u16(),
        reason: message,
    }
}

fn rejected(status: u16, message: String, sent_totp: bool) -> CoreError {
    let lower = message.to_lowercase();
    if status == 400 && (sent_totp || lower.contains("incorrect") || lower.contains("invalid totp")) {
        CoreError::InvalidCredentials
    } else {
        CoreError::Server {
            status,
            reason: message,
        }
    }
}

fn normalize_totp(raw: &str) -> Result<String, CoreError> {
    let code: String = raw.chars().filter(|c| !c.is_whitespace()).collect();
    if (6..=8).contains(&code.len()) && code.bytes().all(|b| b.is_ascii_digit()) {
        Ok(code)
    } else {
        Err(CoreError::invalid("the two-factor code must be 6 to 8 digits"))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wiremock::matchers::{body_string_contains, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::http::client;

    const EMAIL: &str = "Me@Example.com";
    const PASSWORD: &str = "correct horse battery";

    fn expected_hash() -> String {
        let key = master_key(
            PASSWORD,
            EMAIL,
            Kdf::Pbkdf2 {
                iterations: 5_000,
            },
        )
        .unwrap();
        url::form_urlencoded::byte_serialize(master_password_hash(&key, PASSWORD).as_bytes()).collect()
    }

    async fn server() -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/identity/accounts/prelogin"))
            .and(body_string_contains("me@example.com"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"kdf": 0, "kdfIterations": 5000, "kdfMemory": null, "kdfParallelism": null})),
            )
            .mount(&server)
            .await;
        server
    }

    fn tokens() -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "ACCESS-1", "refresh_token": "REFRESH-1", "expires_in": 7200, "token_type": "Bearer"
        }))
    }

    fn two_factor(providers: &[&str]) -> ResponseTemplate {
        ResponseTemplate::new(400).set_body_json(json!({
            "error": "invalid_grant", "error_description": "Two factor required.",
            "TwoFactorProviders": providers, "TwoFactorProviders2": {}
        }))
    }

    async fn login(server: &MockServer, totp: Option<&str>) -> Result<Tokens, CoreError> {
        let http = client().unwrap();
        let url = ServerUrl::parse(&server.uri()).unwrap();
        VaultClient::new(&http, &url).login(EMAIL, Zeroizing::new(PASSWORD.to_owned()), totp, "dev-1").await
    }

    #[tokio::test]
    async fn password_login_sends_contract_form() {
        let server = server().await;
        Mock::given(method("POST"))
            .and(path("/identity/connect/token"))
            .and(header("Device-Type", "0"))
            .and(body_string_contains("grant_type=password"))
            .and(body_string_contains("username=me%40example.com"))
            .and(body_string_contains(format!("password={}", expected_hash())))
            .and(body_string_contains("scope=api+offline_access"))
            .and(body_string_contains("client_id=mobile"))
            .and(body_string_contains("deviceType=0"))
            .and(body_string_contains("deviceIdentifier=dev-1"))
            .and(body_string_contains("deviceName=Rewarden"))
            .respond_with(tokens())
            .expect(1)
            .mount(&server)
            .await;
        let t = login(&server, None).await.unwrap();
        assert_eq!((t.access_token.as_str(), t.refresh_token.as_str(), t.expires_in), ("ACCESS-1", "REFRESH-1", 7200));
        assert!(!format!("{t:?}").contains("ACCESS-1"));
    }

    #[tokio::test]
    async fn totp_flow() {
        let server = server().await;
        Mock::given(method("POST"))
            .and(path("/identity/connect/token"))
            .and(body_string_contains("twoFactorToken=123456"))
            .and(body_string_contains("twoFactorProvider=0"))
            .and(body_string_contains("twoFactorRemember=0"))
            .respond_with(tokens())
            .with_priority(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/identity/connect/token"))
            .respond_with(two_factor(&["0", "1"]))
            .mount(&server)
            .await;
        assert_eq!(login(&server, None).await.unwrap_err(), CoreError::TwoFactorRequired);
        assert_eq!(login(&server, Some("123 456")).await.unwrap().access_token.as_str(), "ACCESS-1");
        assert!(matches!(login(&server, Some("12ab")).await, Err(CoreError::Invalid { .. })));
    }

    #[tokio::test]
    async fn wrong_totp_is_invalid_credentials() {
        let server = server().await;
        Mock::given(method("POST"))
            .and(path("/identity/connect/token"))
            .and(body_string_contains("twoFactorToken="))
            .respond_with(
                ResponseTemplate::new(400).set_body_json(json!({"message": "Invalid TOTP code! Server time: x"})),
            )
            .with_priority(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/identity/connect/token"))
            .respond_with(two_factor(&["0"]))
            .mount(&server)
            .await;
        assert_eq!(login(&server, Some("000000")).await.unwrap_err(), CoreError::InvalidCredentials);
    }

    #[tokio::test]
    async fn other_two_factor_methods_are_unsupported() {
        let server = server().await;
        Mock::given(method("POST"))
            .and(path("/identity/connect/token"))
            .respond_with(two_factor(&["1", "7"]))
            .mount(&server)
            .await;
        assert_eq!(login(&server, Some("123456")).await.unwrap_err(), CoreError::UnsupportedTwoFactor);
    }

    #[tokio::test]
    async fn wrong_password_and_server_errors() {
        let server = server().await;
        Mock::given(method("POST"))
            .and(path("/identity/connect/token"))
            .respond_with(ResponseTemplate::new(400).set_body_json(json!({
                "message": "Username or password is incorrect. Try again", "error": "", "error_description": ""
            })))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        assert_eq!(login(&server, None).await.unwrap_err(), CoreError::InvalidCredentials);
        Mock::given(method("POST"))
            .and(path("/identity/connect/token"))
            .respond_with(
                ResponseTemplate::new(400)
                    .set_body_json(json!({"message": "Please verify your email before trying again."})),
            )
            .mount(&server)
            .await;
        assert_eq!(
            login(&server, None).await.unwrap_err(),
            CoreError::Server {
                status: 400,
                reason: "Please verify your email before trying again.".to_owned()
            }
        );
    }

    #[tokio::test]
    async fn refresh() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/identity/connect/token"))
            .and(body_string_contains("grant_type=refresh_token"))
            .and(body_string_contains("refresh_token=REFRESH-OLD"))
            .and(body_string_contains("client_id=mobile"))
            .respond_with(tokens())
            .with_priority(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/identity/connect/token"))
            .respond_with(ResponseTemplate::new(400).set_body_json(json!({"error": "invalid_grant"})))
            .mount(&server)
            .await;
        let http = client().unwrap();
        let url = ServerUrl::parse(&server.uri()).unwrap();
        let vault = VaultClient::new(&http, &url);
        assert_eq!(vault.refresh("REFRESH-OLD").await.unwrap().refresh_token.as_str(), "REFRESH-1");
        assert_eq!(vault.refresh("REFRESH-DEAD").await.unwrap_err(), CoreError::NotLoggedIn);
    }

    fn new_keys() -> NewAccountKeys {
        crate::crypto::new_account_keys(
            PASSWORD,
            "me@example.com",
            Kdf::Pbkdf2 {
                iterations: 5_000,
            },
        )
        .unwrap()
    }

    async fn register(server: &MockServer) -> Result<(), CoreError> {
        let http = client().unwrap();
        let url = ServerUrl::parse(&server.uri()).unwrap();
        let kdf = Kdf::Pbkdf2 {
            iterations: 5_000,
        };
        VaultClient::new(&http, &url).register("me@example.com", &new_keys(), kdf).await
    }

    fn verification(response: ResponseTemplate) -> Mock {
        Mock::given(method("POST"))
            .and(path("/identity/accounts/register/send-verification-email"))
            .and(header("accept", "application/json"))
            .and(body_string_contains("\"email\":\"me@example.com\""))
            .respond_with(response)
    }

    fn refused() -> ResponseTemplate {
        ResponseTemplate::new(400).set_body_json(json!({"message": "Registration not allowed or user already exists"}))
    }

    #[tokio::test]
    async fn registration_finishes_with_the_token_the_server_hands_back() {
        let server = MockServer::start().await;
        verification(ResponseTemplate::new(200).set_body_json(json!("TOKEN-1"))).expect(1).mount(&server).await;
        Mock::given(method("POST"))
            .and(path("/identity/accounts/register/finish"))
            .and(body_string_contains("\"emailVerificationToken\":\"TOKEN-1\""))
            .and(body_string_contains("\"kdf\":0"))
            .and(body_string_contains("\"kdfIterations\":5000"))
            .and(body_string_contains("\"masterPasswordHash\":\""))
            .and(body_string_contains("\"key\":\"2."))
            .and(body_string_contains("\"encryptedPrivateKey\":\"2."))
            .and(body_string_contains("\"publicKey\":\"MII"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"object": "register"})))
            .expect(1)
            .mount(&server)
            .await;
        register(&server).await.unwrap();
    }

    #[tokio::test]
    async fn servers_that_verify_emails_or_predate_the_token_take_the_older_endpoint() {
        for first in [ResponseTemplate::new(204), ResponseTemplate::new(404)] {
            let server = MockServer::start().await;
            verification(first).mount(&server).await;
            Mock::given(method("POST"))
                .and(path("/identity/accounts/register"))
                .and(body_string_contains("\"kdfIterations\":5000"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({"object": "register"})))
                .expect(1)
                .mount(&server)
                .await;
            register(&server).await.unwrap();
        }
    }

    #[tokio::test]
    async fn closed_sign_ups_and_taken_emails_say_so() {
        let server = MockServer::start().await;
        verification(refused()).mount(&server).await;
        let CoreError::Invalid {
            reason,
        } = register(&server).await.unwrap_err()
        else {
            panic!("not Invalid")
        };
        assert!(reason.contains("does not take new accounts"), "{reason}");

        let server = MockServer::start().await;
        verification(ResponseTemplate::new(200).set_body_string("TOKEN-2")).mount(&server).await;
        Mock::given(method("POST"))
            .and(path("/identity/accounts/register/finish"))
            .and(body_string_contains("TOKEN-2"))
            .respond_with(refused())
            .mount(&server)
            .await;
        assert_eq!(
            register(&server).await.unwrap_err(),
            CoreError::invalid("An account with this email already exists. Sign in instead.")
        );

        let server = MockServer::start().await;
        verification(ResponseTemplate::new(500)).mount(&server).await;
        assert!(matches!(
            register(&server).await.unwrap_err(),
            CoreError::Server {
                status: 500,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn unreachable_server_is_a_network_error_without_url() {
        let http = client().unwrap();
        let url = ServerUrl::parse("http://127.0.0.1:9").unwrap();
        let err = VaultClient::new(&http, &url).prelogin(EMAIL).await.unwrap_err();
        let CoreError::Network {
            reason: message,
        } = err
        else {
            panic!("{err:?}")
        };
        assert!(!message.contains("127.0.0.1"), "{message}");
    }
}
