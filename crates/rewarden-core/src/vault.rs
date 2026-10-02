//! Vaultwarden login (contracts §B): prelogin, master-password hash, password
//! grant with TOTP, refresh.

use std::fmt;

use serde::Deserialize;
use zeroize::Zeroizing;

use crate::CoreError;
use crate::crypto::{Kdf, master_key, master_password_hash};
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
