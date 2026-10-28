//! Microsoft Sentinel through the Azure Monitor Logs Ingestion API: a client-credentials token for
//! `https://monitor.azure.com`, then `POST <endpoint>/dataCollectionRules/<dcr>/streams/<stream>` with one record per
//! audit event (the stream's columns: `TimeGenerated`, `Org`, `Seq`, `Kind`, `Actor`, `Subject`, `Detail`,
//! `PrevHash`, `Hash`).

use reins_proto::managed::AuditEvent;
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{OrgError, Transport, field_ok, headers, refused, url_ok};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub tenant_id: String,
    pub client_id: String,
    pub client_secret: String,
    /// The data collection endpoint's logs ingestion URL.
    pub endpoint: String,
    /// The data collection rule's immutable id (`dcr-…`).
    pub dcr_id: String,
    /// The rule's stream (`Custom-ReinsAudit`).
    pub stream: String,
}

fn id_ok(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

impl Config {
    pub fn check(&self) -> Result<(), OrgError> {
        url_ok(&self.endpoint)?;
        let stream_ok = self.stream.starts_with("Custom-")
            && self.stream.len() <= 100
            && self.stream.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
        if !id_ok(&self.tenant_id)
            || !id_ok(&self.client_id)
            || !self.dcr_id.starts_with("dcr-")
            || !id_ok(&self.dcr_id)
            || !stream_ok
            || !field_ok(&self.client_secret, 256)
        {
            return Err(OrgError::Invalid("Fill in every field (the stream starts with Custom-)."));
        }
        Ok(())
    }
}

/// Microsoft's sign-in (a test base only with `REINS_TEST_ALLOW_LOOPBACK`).
fn login_base() -> String {
    crate::api::reins::org::channels::api_base("https://login.microsoftonline.com", "REINS_TEST_ENTRA_BASE")
}

#[must_use]
pub fn body(events: &[AuditEvent]) -> Vec<u8> {
    let records: Vec<Value> = events
        .iter()
        .map(|e| {
            json!({"TimeGenerated": chrono::DateTime::from_timestamp(e.at, 0).unwrap_or_default().to_rfc3339(),
                   "Org": e.org, "Seq": e.seq, "Kind": e.kind, "Actor": e.actor, "Subject": e.subject,
                   "Detail": e.detail, "PrevHash": e.prev_hash, "Hash": e.hash})
        })
        .collect();
    serde_json::to_vec(&records).unwrap_or_default()
}

async fn token(t: &dyn Transport, cfg: &Config, login: &str) -> Result<String, String> {
    let url = format!("{login}/{}/oauth2/v2.0/token", cfg.tenant_id);
    let form = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("grant_type", "client_credentials")
        .append_pair("client_id", &cfg.client_id)
        .append_pair("client_secret", &cfg.client_secret)
        .append_pair("scope", "https://monitor.azure.com//.default")
        .finish();
    let h = headers(&[("content-type", "application/x-www-form-urlencoded".into())])?;
    let (status, answer) = t.send(Method::POST, &url, h, form.into_bytes()).await?;
    if !(200..300).contains(&status) {
        return Err(format!("Microsoft sign-in for the sink answered {status}"));
    }
    serde_json::from_slice::<Value>(&answer)
        .ok()
        .and_then(|v| v.get("access_token").and_then(Value::as_str).map(str::to_owned))
        .ok_or_else(|| "Microsoft sign-in gave no token".to_owned())
}

pub async fn deliver(t: &dyn Transport, cfg: &Config, events: &[AuditEvent]) -> Result<(), String> {
    deliver_via(t, cfg, events, &login_base()).await
}

async fn deliver_via(t: &dyn Transport, cfg: &Config, events: &[AuditEvent], login: &str) -> Result<(), String> {
    let bearer = token(t, cfg, login).await?;
    let url = format!(
        "{}/dataCollectionRules/{}/streams/{}?api-version=2023-01-01",
        cfg.endpoint.trim_end_matches('/'),
        cfg.dcr_id,
        cfg.stream
    );
    let h = headers(&[("authorization", format!("Bearer {bearer}")), ("content-type", "application/json".into())])?;
    let (status, _) = t.send(Method::POST, &url, h, body(events)).await?;
    if (200..300).contains(&status) {
        Ok(())
    } else {
        Err(refused(&url, status))
    }
}

#[cfg(test)]
mod tests {
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_string_contains, header, method, path},
    };

    use super::*;
    use crate::api::reins::org::sinks::testing::{Plain, events};

    #[tokio::test]
    async fn sentinel_gets_a_token_then_posts_records() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/tenant-1/oauth2/v2.0/token"))
            .and(body_string_contains("grant_type=client_credentials"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"access_token": "tok"})))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/dataCollectionRules/dcr-abc/streams/Custom-ReinsAudit"))
            .and(header("authorization", "Bearer tok"))
            .respond_with(ResponseTemplate::new(204))
            .mount(&server)
            .await;
        let cfg = Config {
            tenant_id: "tenant-1".into(),
            client_id: "client".into(),
            client_secret: "secret".into(),
            endpoint: server.uri(),
            dcr_id: "dcr-abc".into(),
            stream: "Custom-ReinsAudit".into(),
        };
        deliver_via(&Plain, &cfg, &events("o1", 2), &server.uri()).await.unwrap();
        let posted = server.received_requests().await.unwrap();
        let records: Vec<Value> = serde_json::from_slice(&posted[1].body).unwrap();
        assert_eq!((records.len(), records[1]["Seq"].as_i64()), (2, Some(2)));
        assert!(records[0]["TimeGenerated"].as_str().unwrap().starts_with("2026-10-27"));
        assert!(
            String::from_utf8(posted[0].body.clone())
                .unwrap()
                .contains("scope=https%3A%2F%2Fmonitor.azure.com%2F%2F.default")
        );
        // A refused sign-in stops there.
        let down = MockServer::start().await;
        Mock::given(method("POST")).respond_with(ResponseTemplate::new(401)).mount(&down).await;
        assert!(deliver_via(&Plain, &cfg, &events("o1", 1), &down.uri()).await.unwrap_err().contains("401"));
    }
}
