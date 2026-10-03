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

    /// Sends a request with bounded retries. A 401 gets one fresh token, then `GmailNeedsConsent`; 429/5xx and
    /// rate-limit 403s back off exponentially.
    pub async fn request(
        &self,
        method: &Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<&serde_json::Value>,
    ) -> Result<String, CoreError> {
        let mut token = self.access_token().await?;
        let mut refreshed = false;
        let mut attempt = 0;
        loop {
            let mut req =
                self.http.request(method.clone(), format!("{}{path}", self.base)).bearer_auth(&token).query(query);
            if let Some(body) = body {
                req = req.json(body);
            }
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
            if retryable(status, &text) && attempt < MAX_RETRIES {
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
