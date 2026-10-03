//! Gmail REST v1 client (contracts §D, spec §5.3): search, fetch, send and a
//! status probe, authorised with tokens from the Kotlin `GoogleTokenProvider`.

pub mod compose;
pub mod model;
pub mod parse;

use std::sync::Arc;
use std::time::Duration;

use futures::stream::{self, StreamExt};
use reins_proto::gmail::{OutgoingEmail, SentMessage};
use reqwest::Method;
use serde::de::DeserializeOwned;
use serde_json::json;

use self::compose::ReplyContext;
use self::model::{GmailMessage, ListResponse, SendResponse};
use self::parse::ParsedMessage;
use crate::google::GoogleApi;
use crate::{CoreError, GoogleTokenProvider};

pub const DEFAULT_BASE: &str = "https://gmail.googleapis.com/gmail/v1";
/// Concurrent message fetches (spec §5.3).
pub const FETCH_CONCURRENCY: usize = 8;
const METADATA_HEADERS: [&str; 4] = ["From", "To", "Cc", "Subject"];

/// Outcome of the status probe (`users/me/profile`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Probe {
    Ready,
    NeedsConsent,
    Unavailable(String),
}

pub struct GmailClient {
    api: GoogleApi,
}

impl GmailClient {
    pub fn new(
        http: reqwest::Client,
        base: &str,
        token: Arc<dyn GoogleTokenProvider>,
        account: &str,
        backoff_base: Duration,
    ) -> Self {
        Self {
            api: GoogleApi::new(http, base, token, account, "gmail", backoff_base),
        }
    }

    async fn request(
        &self,
        method: &Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<&serde_json::Value>,
    ) -> Result<String, CoreError> {
        self.api.request(method, path, query, body).await
    }

    async fn json<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<&serde_json::Value>,
    ) -> Result<T, CoreError> {
        let text = self.request(&method, path, query, body).await?;
        serde_json::from_str(&text).map_err(|_| CoreError::gmail("Gmail sent an unexpected response"))
    }

    /// `users/me/profile`: is Gmail usable right now?
    pub async fn probe(&self) -> Probe {
        match self.request(&Method::GET, "/users/me/profile", &[], None).await {
            Ok(_) => Probe::Ready,
            Err(CoreError::GmailNeedsConsent) => Probe::NeedsConsent,
            Err(e) => Probe::Unavailable(e.to_string()),
        }
    }

    /// The address of the connected Gmail account (`users/me/profile`).
    pub async fn account(&self) -> Result<Option<String>, CoreError> {
        #[derive(serde::Deserialize)]
        struct Profile {
            #[serde(rename = "emailAddress")]
            email_address: Option<String>,
        }
        let profile: Profile = self.json(Method::GET, "/users/me/profile", &[], None).await?;
        Ok(profile.email_address.map(|a| a.trim().to_ascii_lowercase()).filter(|a| !a.is_empty() && a.len() <= 254))
    }

    /// Message ids matching a Gmail search, newest first.
    pub async fn list(&self, query: &str, max_results: u32) -> Result<Vec<String>, CoreError> {
        let q = [("q", query.to_owned()), ("maxResults", max_results.to_string())];
        let list: ListResponse = self.json(Method::GET, "/users/me/messages", &q, None).await?;
        Ok(list.messages.into_iter().map(|m| m.id).collect())
    }

    async fn get(&self, id: &str, full: bool) -> Result<GmailMessage, CoreError> {
        if id.is_empty() || !id.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return Err(CoreError::invalid("malformed message id"));
        }
        let mut query = vec![(
            "format",
            if full {
                "full"
            } else {
                "metadata"
            }
            .to_owned(),
        )];
        if !full {
            query.extend(METADATA_HEADERS.iter().map(|h| ("metadataHeaders", (*h).to_owned())));
        }
        self.json(Method::GET, &format!("/users/me/messages/{id}"), &query, None).await
    }

    /// One fetch, tagged with its position so the caller can restore the order.
    async fn fetch_one(&self, index: usize, id: String, full: bool) -> (usize, Result<GmailMessage, CoreError>) {
        (index, self.get(&id, full).await)
    }

    /// Fetches and parses messages concurrently (at most [`FETCH_CONCURRENCY`]),
    /// keeping the input order. Messages that vanished (404) are skipped.
    pub async fn fetch(&self, ids: &[String], full: bool) -> Result<Vec<ParsedMessage>, CoreError> {
        let fetches: Vec<_> = ids.iter().cloned().enumerate().map(|(i, id)| self.fetch_one(i, id, full)).collect();
        let results: Vec<(usize, Result<GmailMessage, CoreError>)> =
            stream::iter(fetches).buffer_unordered(FETCH_CONCURRENCY).collect().await;
        let mut ordered: Vec<Option<GmailMessage>> = (0..ids.len()).map(|_| None).collect();
        for (i, result) in results {
            match result {
                Ok(message) => ordered[i] = Some(message),
                Err(CoreError::Gmail {
                    reason: message,
                }) if message.ends_with("404") => {}
                Err(e) => return Err(e),
            }
        }
        Ok(ordered.into_iter().flatten().map(|m| parse::parse_message(&m, full)).collect())
    }

    /// The threading data of the message being replied to.
    pub async fn reply_context(&self, id: &str) -> Option<ReplyContext> {
        if id.is_empty() || !id.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return None;
        }
        let query = [
            ("format", "metadata".to_owned()),
            ("metadataHeaders", "Message-ID".to_owned()),
            ("metadataHeaders", "References".to_owned()),
        ];
        let message: GmailMessage =
            self.json(Method::GET, &format!("/users/me/messages/{id}"), &query, None).await.ok()?;
        let headers = message.payload.as_ref().map(|p| p.headers.as_slice()).unwrap_or_default();
        let find = |name: &str| headers.iter().find(|h| h.name.eq_ignore_ascii_case(name)).map(|h| h.value.clone());
        let message_id = find("Message-ID")?;
        compose::valid_message_id(&message_id)?;
        Some(ReplyContext {
            thread_id: message.thread_id,
            message_id,
            references: find("References"),
        })
    }

    /// Sends `email` (already approved). Replies stay in their conversation when
    /// the original's threading headers can be read.
    pub async fn send(&self, email: &OutgoingEmail) -> Result<SentMessage, CoreError> {
        let reply = match &email.reply_to_message_id {
            Some(id) => self.reply_context(id).await,
            None => None,
        };
        let raw = compose::raw(email, reply.as_ref())?;
        let mut body = json!({ "raw": raw });
        if let (Some(reply), Some(obj)) = (&reply, body.as_object_mut()) {
            obj.insert("threadId".to_owned(), json!(reply.thread_id));
        }
        let sent: SendResponse = self.json(Method::POST, "/users/me/messages/send", &[], Some(&body)).await?;
        Ok(SentMessage {
            id: sent.id,
            thread_id: sent.thread_id,
        })
    }
}
