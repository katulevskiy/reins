//! Telegram, as the user's own account. The connector holds everything that decides what an AI may see and do (how a chat
//! is found, what counts as sensitive, what a message becomes); a [`TelegramBackend`] does the talking to Telegram.

use std::sync::Arc;

use rewarden_proto::connector::{ConnectorCall, TELEGRAM};
use serde_json::{Map, json};

use super::{Connector, Item, LoginProgress, Preview, looks_like_code};
use crate::types::GmailStatus;
use crate::{CoreError, text};

/// Telegram's own service account: login codes and security notices come from it.
pub const TELEGRAM_SERVICE_ID: i64 = 777_000;
/// How many dialogs are looked through to find a chat by name.
const DIALOG_SCAN: usize = 200;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Chat {
    /// The Bot API dialog id, as a string (negative for groups and channels).
    pub id: String,
    pub title: String,
    /// "user", "bot", "group" or "channel".
    pub kind: String,
    pub username: Option<String>,
    /// The last message, one line.
    pub last: String,
    pub date: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Msg {
    pub id: i64,
    pub chat: String,
    pub chat_title: String,
    pub from: String,
    /// The Bot API id of the sender, 0 when unknown.
    pub from_id: i64,
    pub text: String,
    pub date: i64,
}

#[async_trait::async_trait]
pub trait TelegramBackend: Send + Sync {
    async fn dialogs(&self, account: &str, limit: usize) -> Result<Vec<Chat>, CoreError>;
    async fn resolve_username(&self, account: &str, username: &str) -> Result<Option<Chat>, CoreError>;
    async fn messages(&self, account: &str, chat: &Chat, limit: usize) -> Result<Vec<Msg>, CoreError>;
    async fn search(
        &self,
        account: &str,
        chat: Option<&Chat>,
        query: &str,
        limit: usize,
    ) -> Result<Vec<Msg>, CoreError>;
    async fn send(&self, account: &str, chat: &Chat, text: &str, reply_to: Option<i32>) -> Result<i64, CoreError>;
    async fn status(&self, account: &str) -> GmailStatus;
    /// Whether this build carries the application credentials Telegram requires.
    fn configured(&self) -> bool {
        true
    }
    /// Asks Telegram to send a login code to `phone`.
    async fn request_code(&self, phone: &str) -> Result<(), CoreError>;
    async fn submit_code(&self, code: &str) -> Result<LoginProgress, CoreError>;
    async fn submit_password(&self, password: &str) -> Result<String, CoreError>;
    /// Signs the account out of Telegram and forgets its session.
    async fn sign_out(&self, account: &str) -> Result<(), CoreError>;
}

pub struct Telegram {
    backend: Arc<dyn TelegramBackend>,
}

impl Telegram {
    pub fn new(backend: Arc<dyn TelegramBackend>) -> Self {
        Self {
            backend,
        }
    }

    pub fn backend(&self) -> &Arc<dyn TelegramBackend> {
        &self.backend
    }

    /// The chat an AI meant: an id, an @username, or the exact name (which must be unambiguous).
    async fn resolve(&self, account: &str, wanted: &str) -> Result<Chat, CoreError> {
        let wanted = wanted.trim();
        if let Some(username) = wanted.strip_prefix('@') {
            return self
                .backend
                .resolve_username(account, username)
                .await?
                .ok_or_else(|| CoreError::service("Telegram has no chat with that username."));
        }
        let dialogs = self.backend.dialogs(account, DIALOG_SCAN).await?;
        if let Some(chat) = dialogs.iter().find(|c| c.id == wanted) {
            return Ok(chat.clone());
        }
        let mut same_name = dialogs.iter().filter(|c| c.title.eq_ignore_ascii_case(wanted));
        match (same_name.next(), same_name.next()) {
            (Some(chat), None) => Ok(chat.clone()),
            (Some(_), Some(_)) => Err(CoreError::service(
                "More than one chat has that name. Use the id from telegram_list_chats, or an @username.",
            )),
            _ => Err(CoreError::service("No chat with that id or exact name. Use telegram_list_chats to find it.")),
        }
    }

    fn message_item(chat_title: &str, m: &Msg) -> Item {
        let mut extra = Map::new();
        extra.insert("message_id".to_owned(), json!(m.id));
        Item {
            id: format!("{}:{}", m.chat, m.id),
            resource: m.chat.clone(),
            resource_label: text::one_line(if m.chat_title.is_empty() {
                chat_title
            } else {
                &m.chat_title
            }),
            from: text::one_line(&m.from),
            title: String::new(),
            snippet: text::truncate_chars(&text::one_line(&m.text), 160),
            date: m.date,
            body: Some(m.text.clone()),
            sensitive: m.from_id == TELEGRAM_SERVICE_ID || looks_like_code(&m.text),
            secret: false,
            parents: Vec::new(),
            extra,
        }
    }
}

#[async_trait::async_trait]
impl Connector for Telegram {
    fn service(&self) -> &'static str {
        TELEGRAM
    }

    async fn fetch(&self, account: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
        let limit = usize::try_from(call.int_arg("limit").unwrap_or(20).clamp(1, 50)).unwrap_or(20);
        match call.op.as_str() {
            "list_chats" => {
                let query = call.str_arg("query").map(str::to_lowercase);
                let dialogs = self.backend.dialogs(account, DIALOG_SCAN).await?;
                Ok(dialogs
                    .into_iter()
                    .filter(|c| query.as_deref().is_none_or(|q| c.title.to_lowercase().contains(q)))
                    .take(limit)
                    .map(|c| {
                        let mut extra = Map::new();
                        if let Some(u) = &c.username {
                            extra.insert("username".to_owned(), json!(format!("@{u}")));
                        }
                        Item {
                            id: c.id.clone(),
                            resource: c.id,
                            resource_label: text::one_line(&c.title),
                            from: c.kind,
                            title: text::one_line(&c.title),
                            snippet: text::truncate_chars(&text::one_line(&c.last), 120),
                            date: c.date,
                            extra,
                            ..Item::default()
                        }
                    })
                    .collect())
            }
            "read" => {
                let chat = self.resolve(account, call.str_arg("chat").unwrap_or_default()).await?;
                let messages = self.backend.messages(account, &chat, limit).await?;
                Ok(messages.iter().map(|m| Self::message_item(&chat.title, m)).collect())
            }
            "search" => {
                let chat = match call.str_arg("chat") {
                    Some(wanted) => Some(self.resolve(account, wanted).await?),
                    None => None,
                };
                let query = call.str_arg("query").unwrap_or_default();
                let messages = self.backend.search(account, chat.as_ref(), query, limit).await?;
                Ok(messages
                    .iter()
                    .map(|m| Self::message_item(chat.as_ref().map_or("", |c| c.title.as_str()), m))
                    .collect())
            }
            other => Err(CoreError::service(format!("Telegram cannot {other}"))),
        }
    }

    async fn preview(&self, account: &str, call: &ConnectorCall) -> Result<Preview, CoreError> {
        if call.op != "send" {
            return Err(CoreError::service("Telegram cannot do that"));
        }
        let chat = self.resolve(account, call.str_arg("chat").unwrap_or_default()).await?;
        let mut lines =
            vec![format!("To {} ({})", chat.title, chat.kind), call.str_arg("text").unwrap_or_default().to_owned()];
        if let Some(reply) = call.int_arg("reply_to") {
            lines.push(format!("In reply to message {reply}"));
        }
        Ok(Preview {
            resource: chat.id,
            resource_label: chat.title,
            lines,
            ..Preview::default()
        })
    }

    async fn perform(&self, account: &str, call: &ConnectorCall) -> Result<serde_json::Value, CoreError> {
        if call.op != "send" {
            return Err(CoreError::service("Telegram cannot do that"));
        }
        let chat = self.resolve(account, call.str_arg("chat").unwrap_or_default()).await?;
        let reply = call.int_arg("reply_to").and_then(|r| i32::try_from(r).ok());
        let id = self.backend.send(account, &chat, call.str_arg("text").unwrap_or_default(), reply).await?;
        Ok(json!({"sent": true, "message_id": id, "chat": chat.title}))
    }

    async fn status(&self, account: &str) -> GmailStatus {
        self.backend.status(account).await
    }

    fn unavailable(&self) -> Option<String> {
        (!self.backend.configured()).then(|| "This build has no Telegram application credentials.".to_owned())
    }

    async fn forget(&self, account: &str) {
        self.backend.sign_out(account).await.ok();
    }

    async fn login_begin(&self, phone: &str) -> Result<(), CoreError> {
        self.backend.request_code(phone).await
    }

    async fn login_code(&self, code: &str) -> Result<LoginProgress, CoreError> {
        self.backend.submit_code(code).await
    }

    async fn login_password(&self, password: &str) -> Result<String, CoreError> {
        self.backend.submit_password(password).await
    }
}
