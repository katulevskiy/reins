//! A client for Google's REST APIs, authorised with tokens from the Kotlin `GoogleTokenProvider`: bounded retries with
//! backoff, one fresh token after a 401, and errors that never carry a token or a URL. Gmail, Calendar and Contacts
//! each use it with their own base address and scopes.

use std::sync::Arc;
use std::time::Duration;

use reqwest::{Method, StatusCode};

use crate::{CoreError, ForeignError, GoogleTokenProvider};

const MAX_RETRIES: u32 = 3;
const MAX_BACKOFF: Duration = Duration::from_secs(4);

pub struct GoogleApi {
    http: reqwest::Client,
    base: String,
    token: Arc<dyn GoogleTokenProvider>,
    /// The Google account to act as; empty means the phone's default one (accounts not yet known).
    account: String,
    /// "gmail", "gcalendar", "gcontacts": which scopes the token is asked for.
    service: &'static str,
    backoff_base: Duration,
}

impl GoogleApi {
    pub fn new(
        http: reqwest::Client,
        base: &str,
        token: Arc<dyn GoogleTokenProvider>,
        account: &str,
        service: &'static str,
        backoff_base: Duration,
    ) -> Self {
        Self {
            http,
            base: base.trim_end_matches('/').to_owned(),
            token,
            account: account.to_owned(),
            service,
            backoff_base,
        }
    }

    fn error(&self, message: String) -> CoreError {
        if self.service == "gmail" {
            CoreError::gmail(message)
        } else {
            CoreError::service(message)
        }
    }

    /// What a failed answer means for the user, from the reason Google gives (never its text, which can carry a
    /// project number and a link).
    fn explain(&self, status: StatusCode, body: &str) -> String {
        let label = self.label();
        let reason = serde_json::from_str::<serde_json::Value>(body).ok().map(|v| {
            let error = &v["error"];
            format!(
                "{} {}",
                error["status"].as_str().unwrap_or_default(),
                error["errors"][0]["reason"].as_str().unwrap_or_default()
            )
        });
        let reason = reason.unwrap_or_default();
        if reason.contains("SERVICE_DISABLED") || reason.contains("accessNotConfigured") {
            format!("The {label} API is not switched on in this app's Google Cloud project.")
        } else if status == StatusCode::FORBIDDEN && reason.contains("PERMISSION_DENIED") {
            format!(
                "Google did not let this app use {label}. If the app is in testing, add your account as a test user."
            )
        } else if status == StatusCode::NOT_FOUND && self.service != "gmail" {
            format!("{label} could not find that.")
        } else {
            format!("{label} answered {}", status.as_u16())
        }
    }

    fn label(&self) -> &'static str {
        match self.service {
            "gmail" => "Gmail",
            "gcalendar" => "Google Calendar",
            "gcontacts" => "Google Contacts",
            _ => "Google",
        }
    }

    async fn access_token(&self) -> Result<String, CoreError> {
        self.token.access_token(self.account.clone(), self.service.to_owned()).await.map_err(|e| match e {
            ForeignError::NeedsUserInteraction => CoreError::GmailNeedsConsent,
            ForeignError::Failed {
                reason: message,
            } => self.error(format!("could not get a Google token: {message}")),
        })
    }

    /// Safe reads use bounded backoff. Mutations never retry ambiguous 429/5xx responses: the provider may
    /// already have applied them. A definite 401 rejection gets one fresh token, then `GmailNeedsConsent`.
    pub async fn request(
        &self,
        method: &Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<&serde_json::Value>,
    ) -> Result<String, CoreError> {
        self.request_checked(method, path, query, body, || std::future::ready(Ok(()))).await
    }

    /// Rechecks the caller's live execution authority after token acquisition and before every HTTP invocation.
    pub(crate) async fn request_checked<F, Fut>(
        &self,
        method: &Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<&serde_json::Value>,
        mut check: F,
    ) -> Result<String, CoreError>
    where
        F: FnMut() -> Fut + Send,
        Fut: Future<Output = Result<(), CoreError>> + Send,
    {
        let mut token = self.access_token().await?;
        let mut refreshed = false;
        let mut attempt = 0;
        loop {
            let mut req =
                self.http.request(method.clone(), format!("{}{path}", self.base)).bearer_auth(&token).query(query);
            if let Some(body) = body {
                req = req.json(body);
            }
            check().await?;
            let resp = req.send().await?;
            let status = resp.status();
            let text = resp.text().await?;
            if status.is_success() {
                return Ok(text);
            }
            if status == StatusCode::UNAUTHORIZED {
                if refreshed {
                    return Err(CoreError::GmailNeedsConsent);
                }
                refreshed = true;
                token = self.access_token().await?;
                continue;
            }
            if method.is_safe() && retryable(status, &text) && attempt < MAX_RETRIES {
                let delay = (self.backoff_base * 2u32.pow(attempt)).min(MAX_BACKOFF);
                tokio::time::sleep(delay).await;
                attempt += 1;
                continue;
            }
            if status == StatusCode::FORBIDDEN && text.contains("insufficientPermissions") {
                return Err(CoreError::GmailNeedsConsent);
            }
            return Err(self.error(self.explain(status, &text)));
        }
    }
}

fn retryable(status: StatusCode, body: &str) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS
        || status.is_server_error()
        || (status == StatusCode::FORBIDDEN
            && (body.contains("rateLimitExceeded")
                || body.contains("userRateLimitExceeded")
                || body.contains("quotaExceeded")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use tokio::sync::Notify;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    struct GatedToken {
        entered: Arc<Notify>,
        release: Arc<Notify>,
    }
    #[async_trait::async_trait]
    impl GoogleTokenProvider for GatedToken {
        async fn access_token(&self, _account: String, _service: String) -> Result<String, ForeignError> {
            self.entered.notify_one();
            self.release.notified().await;
            Ok("token".into())
        }
    }

    #[tokio::test]
    async fn revoked_authority_during_token_acquisition_never_reaches_provider() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/send"))
            .respond_with(ResponseTemplate::new(200).set_body_string("{}"))
            .expect(0)
            .mount(&server)
            .await;
        let entered = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let token: Arc<dyn GoogleTokenProvider> = Arc::new(GatedToken {
            entered: Arc::clone(&entered),
            release: Arc::clone(&release),
        });
        let api =
            GoogleApi::new(crate::http::client().unwrap(), &server.uri(), token, "account", "gmail", Duration::ZERO);
        let allowed = Arc::new(AtomicBool::new(true));
        let check = Arc::clone(&allowed);
        let invocation = tokio::spawn(async move {
            api.request_checked(&Method::POST, "/send", &[], None, || {
                std::future::ready(if check.load(Ordering::Acquire) {
                    Ok(())
                } else {
                    Err(CoreError::invalid("permission revoked"))
                })
            })
            .await
        });
        entered.notified().await;
        allowed.store(false, Ordering::Release);
        release.notify_one();
        assert!(invocation.await.unwrap().is_err());
        assert!(server.received_requests().await.unwrap().is_empty());
    }
}
