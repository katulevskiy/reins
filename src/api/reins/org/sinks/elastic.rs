//! Elastic: the bulk API (`POST <url>/_bulk`, `Authorization: ApiKey <key>`), `create` with `_id` `<org>-<seq>` so a
//! resend is a duplicate (409) Elastic keeps once.

use reins_proto::managed::AuditEvent;
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{OrgError, Transport, field_ok, headers, host_of, refused, url_ok};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub url: String,
    pub api_key: String,
    pub index: String,
}

impl Config {
    pub fn check(&self) -> Result<(), OrgError> {
        url_ok(&self.url)?;
        let index_ok = !self.index.is_empty()
            && self.index.len() <= 100
            && self.index.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"-_.".contains(&b))
            && !self.index.starts_with(['-', '_', '.']);
        if !field_ok(&self.api_key, 256) || !index_ok {
            return Err(OrgError::Invalid("Fill in every field (the index in lowercase)."));
        }
        Ok(())
    }
}

#[must_use]
pub fn body(cfg: &Config, events: &[AuditEvent]) -> Vec<u8> {
    let mut out = Vec::new();
    for e in events {
        let action = json!({"create": {"_index": cfg.index, "_id": format!("{}-{}", e.org, e.seq)}});
        out.extend(serde_json::to_vec(&action).unwrap_or_default());
        out.push(b'\n');
        let mut doc = serde_json::to_value(e).unwrap_or_default();
        doc["@timestamp"] = json!(chrono::DateTime::from_timestamp(e.at, 0).unwrap_or_default().to_rfc3339());
        out.extend(serde_json::to_vec(&doc).unwrap_or_default());
        out.push(b'\n');
    }
    out
}

/// Whether a bulk answer took every event: no errors, or only duplicates (409).
#[must_use]
pub fn all_taken(answer: &Value) -> bool {
    if answer.get("errors") == Some(&Value::Bool(false)) {
        return true;
    }
    answer.get("items").and_then(Value::as_array).is_some_and(|items| {
        items.iter().all(|i| {
            let status = i.get("create").and_then(|c| c.get("status")).and_then(Value::as_u64);
            matches!(status, Some(200..=299 | 409))
        })
    })
}

pub async fn deliver(t: &dyn Transport, cfg: &Config, events: &[AuditEvent]) -> Result<(), String> {
    let url = format!("{}/_bulk", cfg.url.trim_end_matches('/'));
    let h = headers(&[
        ("authorization", format!("ApiKey {}", cfg.api_key)),
        ("content-type", "application/x-ndjson".into()),
    ])?;
    let (status, answer) = t.send(Method::POST, &url, h, body(cfg, events)).await?;
    if !(200..300).contains(&status) {
        return Err(refused(&url, status));
    }
    let answer: Value =
        serde_json::from_slice(&answer).map_err(|_| format!("{} answered something else", host_of(&url)))?;
    if all_taken(&answer) {
        Ok(())
    } else {
        Err(format!("{} refused some events", host_of(&url)))
    }
}

#[cfg(test)]
mod tests {
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{header, method, path},
    };

    use super::*;
    use crate::api::reins::org::sinks::testing::{Plain, events};

    fn cfg(url: &str) -> Config {
        Config {
            url: url.into(),
            api_key: "k".into(),
            index: "reins-audit".into(),
        }
    }

    #[tokio::test]
    async fn elastic_duplicates_count_as_delivered() {
        let server = MockServer::start().await;
        let answer = json!({"errors": true, "items": [{"create": {"status": 201}}, {"create": {"status": 409}}]});
        Mock::given(method("POST"))
            .and(path("/_bulk"))
            .and(header("content-type", "application/x-ndjson"))
            .and(header("authorization", "ApiKey k"))
            .respond_with(ResponseTemplate::new(200).set_body_json(answer))
            .mount(&server)
            .await;
        deliver(&Plain, &cfg(&server.uri()), &events("o1", 2)).await.unwrap();
        let body = String::from_utf8(server.received_requests().await.unwrap()[0].body.clone()).unwrap();
        assert!(body.contains("\"_id\":\"o1-2\""), "{body}");
        assert_eq!(body.lines().count(), 4);
    }

    #[tokio::test]
    async fn elastic_other_item_errors_do_not() {
        let server = MockServer::start().await;
        let answer = json!({"errors": true, "items": [{"create": {"status": 201}}, {"create": {"status": 400}}]});
        Mock::given(method("POST")).respond_with(ResponseTemplate::new(200).set_body_json(answer)).mount(&server).await;
        assert!(deliver(&Plain, &cfg(&server.uri()), &events("o1", 2)).await.is_err());
        let server = MockServer::start().await;
        Mock::given(method("POST")).respond_with(ResponseTemplate::new(401)).mount(&server).await;
        let e = deliver(&Plain, &cfg(&server.uri()), &events("o1", 1)).await.unwrap_err();
        assert!(e.ends_with("answered 401"), "{e}");
    }
}
