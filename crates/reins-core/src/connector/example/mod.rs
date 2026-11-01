//! A template connector: copy this directory to start a new one (`docs/connectors/adding-a-connector.md`).
//!
//! It is compiled only for tests. It is not in the manifests, so it is never registered, never on the
//! Integrations screen and never listed to an AI. It talks to a made-up notes service over HTTP with an access token
//! the user pastes: one read (`notes_list`) and one write (`notes_add`) that the user approves on the phone from its
//! [`Preview`].
//!
//! How it handles the token is the part to keep when copying it:
//! - [`Connector::sign_in_token`] checks the token with the service before keeping it, and keeps it only in the
//!   phone's secret store ([`Store::secret_put`], under this service and the account it belongs to). The connector
//!   itself holds no credential.
//! - Each request reads it back from the store ([`ExampleNotes::token`]) into memory that is wiped after use, and
//!   sends it in a header, never in a URL.
//! - Errors say what went wrong by the HTTP status alone: never the token, and never the service's reply, which can
//!   echo what was sent.
//! - [`Connector::forget`] deletes it when the user disconnects the account.

use std::sync::Arc;

use reins_proto::connector::{
    AuthType, ConnectorCall, ConnectorManifest, Effect, Kind, MAX_ACCOUNT_LEN, Param, ToolSpec,
};
use reqwest::StatusCode;
use serde_json::{Value, json};
use zeroize::Zeroizing;

use super::{Connector, Item, Preview};
use crate::CoreError;
use crate::store::Store;
use crate::text;
use crate::types::GmailStatus;

/// The service id: the `service` of its tools' calls, the key of its connector in the registry and of its secrets.
pub const SERVICE: &str = "example_notes";
const NAME: &str = "Example notes";

/// What the connector is. A real one goes in `reins_proto::connector::manifest::MANIFESTS`, with its tools in
/// `reins_proto::connector`.
pub const MANIFEST: ConnectorManifest = ConnectorManifest {
    service: SERVICE,
    name: NAME,
    auth: AuthType::Token,
    scopes: &["notes.read", "notes.write"],
    classes: &[],
    screen: None,
    tools,
};

/// Its tools, as `tools/list` would show them. A real connector builds them with `tool(...)` in `reins_proto`; that
/// builder is private to the crate, so the template spells the struct out.
pub fn tools() -> Vec<ToolSpec> {
    let spec = |tool, op, effect, title, description, params, scopes| ToolSpec {
        tool,
        service: SERVICE,
        op,
        effect,
        title,
        description,
        params,
        resource_param: None,
        class: "",
        once_only: false,
        desktop_only: false,
        check: None,
        scopes,
    };
    vec![
        spec("notes_list", "list", Effect::List, "List notes", "Lists the user's notes.", vec![], &["notes.read"]),
        spec(
            "notes_add",
            "add",
            Effect::Write,
            "Add a note",
            "Adds a note. The user approves it on their phone.",
            vec![Param {
                name: "text",
                kind: Kind::Str {
                    max: 1_000,
                },
                required: true,
                description: "The note.",
            }],
            &["notes.write"],
        ),
    ]
}

/// The connector. It holds no credential itself: each account's token is in the phone's secret store.
pub struct ExampleNotes {
    http: reqwest::Client,
    base: String,
    store: Arc<Store>,
}

impl ExampleNotes {
    pub fn new(http: reqwest::Client, base: &str, store: Arc<Store>) -> Self {
        Self {
            http,
            base: base.trim_end_matches('/').to_owned(),
            store,
        }
    }

    /// The account's token, from the secret store, in memory that is wiped when it is dropped.
    fn token(&self, account: &str) -> Result<Zeroizing<String>, CoreError> {
        let raw = Zeroizing::new(
            self.store
                .secret_get(SERVICE, account)?
                .ok_or_else(|| CoreError::needs_attention(format!("{NAME} is not connected")))?,
        );
        let token = std::str::from_utf8(&raw).map_err(|_| CoreError::storage(format!("corrupt {NAME} token")))?;
        Ok(Zeroizing::new(token.to_owned()))
    }

    /// What a failed answer means. The service's own message is not passed on: it could echo what was sent.
    fn explain(status: StatusCode) -> CoreError {
        match status {
            StatusCode::UNAUTHORIZED => CoreError::needs_attention(format!(
                "{NAME} does not accept the token: it is mistyped, expired or revoked"
            )),
            StatusCode::FORBIDDEN => CoreError::service(format!("{NAME} refused: the token may lack a permission")),
            other => CoreError::service(format!("{NAME} answered {}", other.as_u16())),
        }
    }

    /// Sends `request` with `token` in its `Authorization` header.
    async fn send(&self, token: &str, request: reqwest::RequestBuilder) -> Result<Value, CoreError> {
        let response = request.bearer_auth(token).send().await?;
        let status = response.status();
        if !status.is_success() {
            return Err(Self::explain(status));
        }
        response.json().await.map_err(|_| CoreError::service(format!("{NAME} sent an answer it cannot read")))
    }

    /// The account a token belongs to, as the service says: one line, lower case.
    async fn whoami(&self, token: &str) -> Result<String, CoreError> {
        let me = self.send(token, self.http.get(format!("{}/me", self.base))).await?;
        me["login"]
            .as_str()
            .map(text::one_line)
            .filter(|l| !l.is_empty() && l.len() <= MAX_ACCOUNT_LEN)
            .map(|l| l.to_lowercase())
            .ok_or_else(|| CoreError::service(format!("{NAME} did not say who that is")))
    }
}

#[async_trait::async_trait]
impl Connector for ExampleNotes {
    fn service(&self) -> &'static str {
        SERVICE
    }

    /// Checks the pasted token with the service, then keeps it for the account it belongs to.
    async fn sign_in_token(&self, token: &str) -> Result<String, CoreError> {
        let account = self.whoami(token).await?;
        self.store.secret_put(SERVICE, &account, token.as_bytes())?;
        Ok(account)
    }

    async fn fetch(&self, account: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
        if call.op != "list" {
            return Err(CoreError::invalid(format!("{NAME} cannot {}", call.op)));
        }
        let token = self.token(account)?;
        let notes = self.send(&token, self.http.get(format!("{}/notes", self.base))).await?;
        Ok(notes["notes"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|note| Item {
                id: note["id"].as_str().unwrap_or_default().to_owned(),
                resource: account.to_owned(),
                resource_label: account.to_owned(),
                title: note["text"].as_str().unwrap_or_default().to_owned(),
                snippet: note["text"].as_str().unwrap_or_default().to_owned(),
                ..Item::default()
            })
            .collect())
    }

    /// The approval screen: the account and the exact text, before anything is sent.
    async fn preview(&self, account: &str, call: &ConnectorCall) -> Result<Preview, CoreError> {
        let text = call.str_arg("text").ok_or_else(|| CoreError::invalid("text is required"))?;
        // A write to an account that is not connected fails here, not after the user approved it.
        self.token(account)?;
        Ok(Preview {
            resource: account.to_owned(),
            resource_label: account.to_owned(),
            lines: vec![format!("Account: {account}"), format!("Note: {text}")],
            ..Preview::default()
        })
    }

    /// Runs only after the user approved the preview.
    async fn perform(&self, account: &str, call: &ConnectorCall) -> Result<Value, CoreError> {
        let text = call.str_arg("text").ok_or_else(|| CoreError::invalid("text is required"))?;
        let token = self.token(account)?;
        let created =
            self.send(&token, self.http.post(format!("{}/notes", self.base)).json(&json!({"text": text}))).await?;
        Ok(json!({"id": created["id"]}))
    }

    async fn forget(&self, account: &str) {
        self.store.secret_delete(SERVICE, account).ok();
    }

    async fn status(&self, account: &str) -> GmailStatus {
        let Ok(token) = self.token(account) else {
            return GmailStatus::NeedsConsent;
        };
        match self.whoami(&token).await {
            Ok(_) => GmailStatus::Ready,
            Err(CoreError::ServiceNeedsAttention {
                ..
            }) => GmailStatus::NeedsConsent,
            Err(e) => GmailStatus::Unavailable {
                message: e.to_string(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::Map;
    use wiremock::matchers::{body_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::connector::Registry;
    use crate::http::client;

    const TOKEN: &str = "notes-TOKEN-SECRET";

    fn call(op: &str, args: Value) -> ConnectorCall {
        let args: Map<String, Value> = serde_json::from_value(args).unwrap();
        ConnectorCall {
            service: SERVICE.into(),
            op: op.into(),
            args,
        }
    }

    fn notes(server: &MockServer) -> (tempfile::TempDir, Arc<Store>, ExampleNotes) {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(crate::store::tests::open(dir.path()));
        let notes = ExampleNotes::new(client().unwrap(), &server.uri(), Arc::clone(&store));
        (dir, store, notes)
    }

    async fn me(server: &MockServer) {
        Mock::given(method("GET"))
            .and(path("/me"))
            .and(header("authorization", format!("Bearer {TOKEN}").as_str()))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"login": "Ada"})))
            .mount(server)
            .await;
    }

    #[test]
    fn the_manifest_describes_each_tool() {
        let tools = tools();
        let shape: Vec<_> = tools.iter().map(|t| (t.tool, t.effect)).collect();
        assert_eq!(shape, [("notes_list", Effect::List), ("notes_add", Effect::Write)]);
        for tool in &tools {
            assert_eq!(tool.service, SERVICE);
            assert!(tool.scopes.iter().all(|s| MANIFEST.scopes.contains(s)), "{}", tool.tool);
        }
        assert_eq!(MANIFEST.auth.kind(), "token");
    }

    #[test]
    fn it_is_not_a_real_connector() {
        assert!(reins_proto::connector::manifest::manifest(SERVICE).is_none());
        assert!(reins_proto::connector::spec_for_tool("notes_add").is_none());
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(crate::store::tests::open(dir.path()));
        let mut registry = Registry::default();
        registry.add(Arc::new(ExampleNotes::new(client().unwrap(), "http://127.0.0.1:9", store)));
        assert!(registry.get(SERVICE).is_some());
    }

    #[tokio::test]
    async fn signs_in_reads_previews_and_writes_with_the_kept_token() {
        let server = MockServer::start().await;
        me(&server).await;
        let bearer = format!("Bearer {TOKEN}");
        Mock::given(method("GET"))
            .and(path("/notes"))
            .and(header("authorization", bearer.as_str()))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"notes": [{"id": "n1", "text": "milk"}]})))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/notes"))
            .and(header("authorization", bearer.as_str()))
            .and(body_json(json!({"text": "eggs"})))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({"id": "n2"})))
            .expect(1)
            .mount(&server)
            .await;
        let (_dir, store, notes) = notes(&server);

        let account = notes.sign_in_token(TOKEN).await.unwrap();
        assert_eq!(account, "ada");
        assert_eq!(store.secret_get(SERVICE, "ada").unwrap().as_deref(), Some(TOKEN.as_bytes()));
        assert_eq!(notes.status("ada").await, GmailStatus::Ready);

        let items = notes.fetch("ada", &call("list", json!({}))).await.unwrap();
        assert_eq!((items[0].id.as_str(), items[0].title.as_str()), ("n1", "milk"));

        let add = call("add", json!({"text": "eggs"}));
        let preview = notes.preview("ada", &add).await.unwrap();
        assert_eq!(preview.lines, ["Account: ada", "Note: eggs"]);
        assert_eq!(notes.perform("ada", &add).await.unwrap(), json!({"id": "n2"}));

        notes.forget("ada").await;
        assert_eq!(store.secret_get(SERVICE, "ada").unwrap(), None, "disconnecting deletes the token");
        assert_eq!(notes.status("ada").await, GmailStatus::NeedsConsent);
    }

    #[tokio::test]
    async fn a_rejected_token_is_not_kept_and_never_shown() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/me"))
            .respond_with(ResponseTemplate::new(401).set_body_json(json!({"error": format!("bad token {TOKEN}")})))
            .mount(&server)
            .await;
        let (_dir, store, notes) = notes(&server);
        let error = notes.sign_in_token(TOKEN).await.unwrap_err();
        assert!(matches!(error, CoreError::ServiceNeedsAttention { .. }), "{error:?}");
        assert!(!format!("{error} {error:?}").contains(TOKEN), "the token leaked into the error: {error:?}");
        assert!(store.secret_get(SERVICE, "ada").unwrap().is_none());
    }

    #[tokio::test]
    async fn an_account_that_is_not_connected_asks_nothing_of_the_service() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).respond_with(ResponseTemplate::new(200)).expect(0).mount(&server).await;
        let (_dir, _store, notes) = notes(&server);
        assert!(notes.fetch("ada", &call("list", json!({}))).await.is_err());
        let add = call("add", json!({"text": "eggs"}));
        assert!(matches!(notes.preview("ada", &add).await, Err(CoreError::ServiceNeedsAttention { .. })));
    }

    #[tokio::test]
    async fn a_failed_request_is_an_error() {
        let server = MockServer::start().await;
        me(&server).await;
        Mock::given(method("GET")).and(path("/notes")).respond_with(ResponseTemplate::new(500)).mount(&server).await;
        let (_dir, _store, notes) = notes(&server);
        notes.sign_in_token(TOKEN).await.unwrap();
        assert!(notes.fetch("ada", &call("list", json!({}))).await.is_err());
    }
}
