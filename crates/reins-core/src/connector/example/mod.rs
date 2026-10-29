//! A template connector: copy this directory to start a new one (`docs/connectors/adding-a-connector.md`).
//!
//! It is compiled only for tests. It is not in the manifests, so it is never registered, never on the
//! Integrations screen and never listed to an AI. It talks to a made-up notes service over HTTP: one read
//! (`notes_list`) and one write (`notes_add`) that the user approves on the phone from its [`Preview`].

use reins_proto::connector::{AuthType, ConnectorCall, ConnectorManifest, Effect, Kind, Param, RiskLevel, ToolSpec};
use serde_json::{Value, json};

use super::{Connector, Item, Preview};
use crate::CoreError;
use crate::types::GmailStatus;

/// The service id: the `service` of its tools' calls and the key of its connector in the registry.
pub const SERVICE: &str = "example_notes";

/// What the connector is. A real one goes in `reins_proto::connector::manifest::MANIFESTS`, with its tools in
/// `reins_proto::connector`.
pub const MANIFEST: ConnectorManifest = ConnectorManifest {
    service: SERVICE,
    name: "Example notes",
    auth: AuthType::Token,
    scopes: &["notes.read", "notes.write"],
    classes: &[],
    screen: None,
    tools,
};

/// Its tools, as `tools/list` would show them.
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
        risk: None,
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
        )
        .risk(RiskLevel::Medium),
    ]
}

/// The connector. It holds no credential itself: a real one reads the account's token from the phone's store.
pub struct ExampleNotes {
    http: reqwest::Client,
    base: String,
}

impl ExampleNotes {
    pub fn new(http: reqwest::Client, base: &str) -> Self {
        Self {
            http,
            base: base.trim_end_matches('/').to_owned(),
        }
    }

    async fn send(&self, request: reqwest::RequestBuilder) -> Result<Value, CoreError> {
        let response = request.send().await.map_err(|e| CoreError::service(format!("notes: {e}")))?;
        let status = response.status();
        if !status.is_success() {
            return Err(CoreError::service(format!("notes answered {status}")));
        }
        response.json().await.map_err(|e| CoreError::service(format!("notes: {e}")))
    }
}

#[async_trait::async_trait]
impl Connector for ExampleNotes {
    fn service(&self) -> &'static str {
        SERVICE
    }

    fn manifest(&self) -> Option<&'static ConnectorManifest> {
        Some(&MANIFEST)
    }

    async fn fetch(&self, account: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
        if call.op != "list" {
            return Err(CoreError::invalid(format!("notes cannot {}", call.op)));
        }
        let notes = self.send(self.http.get(format!("{}/notes", self.base))).await?;
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

    async fn preview(&self, account: &str, call: &ConnectorCall) -> Result<Preview, CoreError> {
        let text = call.str_arg("text").ok_or_else(|| CoreError::invalid("text is required"))?;
        Ok(Preview {
            resource: account.to_owned(),
            resource_label: account.to_owned(),
            lines: vec![format!("Account: {account}"), format!("Note: {text}")],
            ..Preview::default()
        })
    }

    async fn perform(&self, _account: &str, call: &ConnectorCall) -> Result<Value, CoreError> {
        let text = call.str_arg("text").ok_or_else(|| CoreError::invalid("text is required"))?;
        let created = self.send(self.http.post(format!("{}/notes", self.base)).json(&json!({"text": text}))).await?;
        Ok(json!({"id": created["id"]}))
    }

    async fn status(&self, _account: &str) -> GmailStatus {
        GmailStatus::Ready
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use reins_proto::connector::ActionManifest;
    use serde_json::Map;
    use wiremock::matchers::{body_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::connector::Registry;
    use crate::http::client;

    fn call(op: &str, args: Value) -> ConnectorCall {
        let args: Map<String, Value> = serde_json::from_value(args).unwrap();
        ConnectorCall {
            service: SERVICE.into(),
            op: op.into(),
            args,
        }
    }

    #[test]
    fn the_manifest_describes_each_action() {
        let specs: &'static [ToolSpec] = tools().leak();
        let actions: Vec<ActionManifest> = specs.iter().map(ActionManifest::of).collect();
        assert_eq!(actions.len(), 2);
        assert_eq!((actions[0].tool, actions[0].risk, actions[0].writes()), ("notes_list", RiskLevel::Low, false));
        assert_eq!((actions[1].tool, actions[1].risk, actions[1].writes()), ("notes_add", RiskLevel::Medium, true));
        assert_eq!(actions[1].scopes, ["notes.write"]);
        assert_eq!(MANIFEST.auth.kind(), "token");
    }

    #[test]
    fn it_is_not_a_real_connector() {
        assert!(reins_proto::connector::manifest::manifest(SERVICE).is_none());
        assert!(reins_proto::connector::spec_for_tool("notes_add").is_none());
        let mut registry = Registry::default();
        registry.add(Arc::new(ExampleNotes::new(client().unwrap(), "http://127.0.0.1:9")));
        assert!(registry.get(SERVICE).is_some());
        assert!(registry.manifests().is_empty(), "only manifested connectors are listed");
    }

    #[tokio::test]
    async fn reads_previews_and_writes_over_http() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/notes"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"notes": [{"id": "n1", "text": "milk"}]})))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/notes"))
            .and(body_json(json!({"text": "eggs"})))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({"id": "n2"})))
            .expect(1)
            .mount(&server)
            .await;
        let notes = ExampleNotes::new(client().unwrap(), &server.uri());

        let items = notes.fetch("me", &call("list", json!({}))).await.unwrap();
        assert_eq!((items[0].id.as_str(), items[0].title.as_str()), ("n1", "milk"));

        let add = call("add", json!({"text": "eggs"}));
        let preview = notes.preview("me", &add).await.unwrap();
        assert_eq!(preview.lines, ["Account: me", "Note: eggs"]);
        assert_eq!(notes.perform("me", &add).await.unwrap(), json!({"id": "n2"}));
    }

    #[tokio::test]
    async fn a_failed_request_is_an_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).respond_with(ResponseTemplate::new(500)).mount(&server).await;
        let notes = ExampleNotes::new(client().unwrap(), &server.uri());
        assert!(notes.fetch("me", &call("list", json!({}))).await.is_err());
    }
}
