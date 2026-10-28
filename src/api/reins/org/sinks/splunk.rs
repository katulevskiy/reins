//! Splunk: the HTTP Event Collector (`POST <url>/services/collector/event`, `Authorization: Splunk <token>`), one
//! event object per audit event, `sourcetype` `reins:audit`.

use reins_proto::managed::AuditEvent;
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{OrgError, Transport, field_ok, headers, refused, url_ok};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// The collector's base URL (`https://splunk.example.com:8088`).
    pub url: String,
    pub token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<String>,
}

impl Config {
    pub fn check(&self) -> Result<(), OrgError> {
        url_ok(&self.url)?;
        if !field_ok(&self.token, 256) || self.index.as_deref().is_some_and(|i| !field_ok(i, 80)) {
            return Err(OrgError::Invalid("Fill in every field."));
        }
        Ok(())
    }
}

/// The collector's body: one JSON object per event, concatenated.
#[must_use]
pub fn body(cfg: &Config, events: &[AuditEvent]) -> Vec<u8> {
    let mut out = Vec::new();
    for e in events {
        let mut ev = json!({"time": e.at, "source": "reins", "sourcetype": "reins:audit", "event": e});
        if let Some(i) = &cfg.index {
            ev["index"] = json!(i);
        }
        out.extend(serde_json::to_vec(&ev).unwrap_or_default());
        out.push(b'\n');
    }
    out
}

pub async fn deliver(t: &dyn Transport, cfg: &Config, events: &[AuditEvent]) -> Result<(), String> {
    let url = format!("{}/services/collector/event", cfg.url.trim_end_matches('/'));
    let h =
        headers(&[("authorization", format!("Splunk {}", cfg.token)), ("content-type", "application/json".into())])?;
    let (status, _) = t.send(Method::POST, &url, h, body(cfg, events)).await?;
    if (200..300).contains(&status) {
        Ok(())
    } else {
        Err(refused(&url, status))
    }
}
