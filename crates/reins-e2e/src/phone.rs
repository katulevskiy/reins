//! A headless phone: the real core with fake Keystore/Google/notifier and a fake Gmail.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use data_encoding::BASE64URL_NOPAD;
use reins_core::{
    AutoDecisionView, AutopilotEvent, CoreConfig, CoreError, ForeignError, GoogleTokenProvider, KeyWrapper, Notifier,
    PendingItem, ReinsCore, SsoOutcome,
};
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::PASSWORD;

struct Keys;

impl KeyWrapper for Keys {
    fn wrap(&self, plaintext: Vec<u8>) -> Result<Vec<u8>, ForeignError> {
        Ok(plaintext.iter().map(|b| b ^ 0x5a).collect())
    }

    fn unwrap(&self, wrapped: Vec<u8>) -> Result<Vec<u8>, ForeignError> {
        Ok(wrapped.iter().map(|b| b ^ 0x5a).collect())
    }
}

struct Google;

#[async_trait::async_trait]
impl GoogleTokenProvider for Google {
    async fn access_token(&self, _account: String, _service: String) -> Result<String, ForeignError> {
        Ok("fake-google-token".to_owned())
    }
}

#[derive(Default)]
pub struct Notes {
    pub pending: Mutex<Vec<PendingItem>>,
    /// What Autopilot (or a bypass, or Lockdown) decided on its own.
    pub decided: Mutex<Vec<AutoDecisionView>>,
    pub events: Mutex<Vec<AutopilotEvent>>,
}

impl Notifier for Notes {
    fn item_pending(&self, item: PendingItem) {
        self.pending.lock().expect("notes").push(item);
    }

    fn item_resolved(&self, _id: String) {}

    fn auto_decided(&self, decision: AutoDecisionView) {
        self.decided.lock().expect("notes").push(decision);
    }

    fn autopilot_changed(&self, event: AutopilotEvent) {
        self.events.lock().expect("notes").push(event);
    }
}

pub struct Phone {
    pub core: Arc<ReinsCore>,
    pub gmail: MockServer,
    pub notes: Arc<Notes>,
    dir: tempfile::TempDir,
}

/// A Gmail message as the REST API returns it.
pub fn gmail_message(id: &str, from: &str, subject: &str, body: &str) -> Value {
    json!({
        "id": id, "threadId": format!("t-{id}"), "labelIds": ["INBOX"], "snippet": format!("snippet {id}"),
        "internalDate": "1700000000000",
        "payload": {"mimeType": "text/plain", "headers": [
            {"name": "From", "value": from}, {"name": "To", "value": "me@example.com"}, {"name": "Subject", "value": subject}],
            "body": {"data": BASE64URL_NOPAD.encode(body.as_bytes())}}
    })
}

/// The Gmail account every simulated phone has connected.
pub const GMAIL_ACCOUNT: &str = "phone@gmail.com";

impl Phone {
    /// Signs in to `server_url` as `email` and registers this device as the approval device.
    pub async fn sign_in(server_url: &str, email: &str) -> Self {
        Self::sign_in_with(server_url, email, PASSWORD).await
    }

    /// Like [`Phone::sign_in`] with an explicit master password (for a real, already deployed server).
    pub async fn sign_in_with(server_url: &str, email: &str, password: &str) -> Self {
        Self::sign_in_configured(server_url, email, password, |_| {}).await
    }

    /// Like [`Phone::sign_in`], with GitHub's REST API at `github_base` (a fake).
    pub async fn sign_in_with_github(server_url: &str, email: &str, github_base: &str) -> Self {
        let base = github_base.to_owned();
        Self::sign_in_configured(server_url, email, PASSWORD, move |cfg| cfg.github_base = base).await
    }

    /// Like [`Phone::sign_in`], with the core's settings changed first (Autopilot's model server, ...).
    pub async fn sign_in_with_config(server_url: &str, email: &str, configure: impl FnOnce(&mut CoreConfig)) -> Self {
        Self::sign_in_configured(server_url, email, PASSWORD, configure).await
    }

    /// Creates the account `email` from the phone (the core's `create_account`, nothing registered before), then goes
    /// on like [`Phone::sign_in`].
    pub async fn create_account(server_url: &str, email: &str) -> Self {
        Self::start(server_url, email, PASSWORD, true, |_| {}).await
    }

    async fn sign_in_configured(
        server_url: &str,
        email: &str,
        password: &str,
        configure: impl FnOnce(&mut CoreConfig),
    ) -> Self {
        Self::start(server_url, email, password, false, configure).await
    }

    async fn start(
        server_url: &str,
        email: &str,
        password: &str,
        create: bool,
        configure: impl FnOnce(&mut CoreConfig),
    ) -> Self {
        let phone = Self::signed_out(configure).await;
        if create {
            phone
                .core
                .create_account(server_url.to_owned(), email.to_owned(), password.to_owned())
                .await
                .expect("create the account");
        } else {
            phone.core.login(server_url.to_owned(), email.to_owned(), password.to_owned(), None).await.expect("login");
        }
        phone.core.register_device(None).await.expect("register device");
        phone.connect_gmail().await;
        phone
    }

    /// A freshly installed phone: the core, signed in to nothing.
    pub async fn signed_out(configure: impl FnOnce(&mut CoreConfig)) -> Self {
        let gmail = MockServer::start().await;
        let dir = tempfile::tempdir().expect("tempdir");
        let notes = Arc::new(Notes::default());
        let mut cfg = CoreConfig {
            gmail_base: gmail.uri(),
            backoff_base: Duration::from_millis(5),
            ..CoreConfig::default()
        };
        configure(&mut cfg);
        let notifier: Arc<dyn Notifier> = Arc::<Notes>::clone(&notes);
        let core = ReinsCore::with_config(dir.path().to_str().expect("utf8"), &Keys, Arc::new(Google), notifier, cfg)
            .expect("core");
        Self {
            core,
            gmail,
            notes,
            dir,
        }
    }

    /// The core's data directory (its encrypted stores).
    pub fn data_dir(&self) -> &std::path::Path {
        self.dir.path()
    }

    /// Connects the fake Gmail account [`GMAIL_ACCOUNT`].
    pub async fn connect_gmail(&self) {
        Mock::given(method("GET"))
            .and(path("/users/me/profile"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"emailAddress": GMAIL_ACCOUNT})))
            .mount(&self.gmail)
            .await;
        self.core.add_account(GMAIL_ACCOUNT.to_owned()).await.expect("connect the Gmail account");
    }

    /// "Continue": the server's SSO sign-in, with the browser part run by [`crate::workos::browse_to_callback`]
    /// (whoever the identity provider signs in is up to it).
    pub async fn sso_sign_in(&self, server_url: &str) -> Result<SsoOutcome, CoreError> {
        let start = self.core.sso_begin(server_url.to_owned()).await?;
        let callback = crate::workos::browse_to_callback(&start.url, &start.callback_scheme).await;
        self.core.sso_finish(server_url.to_owned(), callback, start.state, start.verifier).await
    }

    /// Makes Gmail return these messages `(id, from)` for any search, and their full text.
    pub async fn stock_gmail(&self, messages: &[(&str, &str)]) {
        let ids: Vec<Value> = messages.iter().map(|(id, _)| json!({"id": id})).collect();
        Mock::given(method("GET"))
            .and(path("/users/me/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"messages": ids})))
            .mount(&self.gmail)
            .await;
        for (id, from) in messages {
            Mock::given(method("GET"))
                .and(path(format!("/users/me/messages/{id}")))
                .respond_with(ResponseTemplate::new(200).set_body_json(gmail_message(
                    id,
                    from,
                    &format!("Subject {id}"),
                    &format!("Body of {id}"),
                )))
                .mount(&self.gmail)
                .await;
        }
    }

    pub async fn accept_sends(&self) {
        Mock::given(method("POST"))
            .and(path("/users/me/messages/send"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "sent-1", "threadId": "t-sent"})))
            .mount(&self.gmail)
            .await;
    }

    /// Long-polls the server until an item for the user appears (the "app is open" behaviour).
    pub async fn wait_for_item(&self, timeout: Duration) -> PendingItem {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let items = self.core.sync(2).await.expect("sync");
            if let Some(item) = items.into_iter().next() {
                return item;
            }
            assert!(tokio::time::Instant::now() < deadline, "no item arrived within {timeout:?}");
        }
    }
}
