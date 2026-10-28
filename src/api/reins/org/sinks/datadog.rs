//! Datadog: the Logs intake (`POST https://http-intake.logs.<site>/api/v2/logs`, `DD-API-KEY`), one log per audit
//! event, `ddsource` `reins`, the event under `reins`.

use reins_proto::managed::AuditEvent;
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{OrgError, Transport, field_ok, headers, refused};

/// Datadog's sites (an API key belongs to one).
pub const SITES: [&str; 6] =
    ["datadoghq.com", "us3.datadoghq.com", "us5.datadoghq.com", "datadoghq.eu", "ap1.datadoghq.com", "ddog-gov.com"];

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub site: String,
    pub api_key: String,
}

impl Config {
    pub fn check(&self) -> Result<(), OrgError> {
        if !SITES.contains(&self.site.as_str()) {
            return Err(OrgError::Invalid("That isn't a Datadog site."));
        }
        if !field_ok(&self.api_key, 128) {
            return Err(OrgError::Invalid("Fill in every field."));
        }
        Ok(())
    }

    fn url(&self) -> String {
        let base = crate::api::reins::org::channels::api_base(
            &format!("https://http-intake.logs.{}", self.site),
            "REINS_TEST_DATADOG_BASE",
        );
        format!("{base}/api/v2/logs")
    }
}

#[must_use]
pub fn body(org: &str, events: &[AuditEvent]) -> Vec<u8> {
    let logs: Vec<_> = events
        .iter()
        .map(|e| {
            json!({"ddsource": "reins", "service": "reins-audit", "hostname": "reins",
                   "ddtags": format!("org:{org}"), "message": e.kind, "reins": e})
        })
        .collect();
    serde_json::to_vec(&logs).unwrap_or_default()
}

pub async fn deliver(t: &dyn Transport, cfg: &Config, org: &str, events: &[AuditEvent]) -> Result<(), String> {
    let url = cfg.url();
    let h = headers(&[("dd-api-key", cfg.api_key.clone()), ("content-type", "application/json".into())])?;
    let (status, _) = t.send(Method::POST, &url, h, body(org, events)).await?;
    if (200..300).contains(&status) {
        Ok(())
    } else {
        Err(refused(&url, status))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn datadog_refuses_an_unknown_site() {
        let cfg = |site: &str| Config {
            site: site.into(),
            api_key: "k".into(),
        };
        assert!(cfg("datadoghq.eu").check().is_ok());
        for bad in ["evil.example.com", "datadoghq.com.evil.net", ""] {
            assert!(cfg(bad).check().is_err(), "{bad}");
        }
        assert_eq!(cfg("datadoghq.eu").url(), "https://http-intake.logs.datadoghq.eu/api/v2/logs");
    }
}
