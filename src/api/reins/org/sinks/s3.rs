//! S3 (and S3-compatible storage: GCS through `https://storage.googleapis.com` with HMAC keys) with object lock: one
//! object per segment of the chain between two signed checkpoints, `<prefix><org>/<after>-<to>.jsonl`, each a whole
//! export that `reins-audit verify --anchor <the checkpoint at after>` checks alone. With `lock_days`, objects are
//! written in COMPLIANCE mode until then (the bucket needs Object Lock on; GCS uses a locked bucket retention policy
//! instead, so set `lock_days` to 0 there).

use reqwest::Method;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use super::{OrgError, Transport, field_ok, headers, refused, url_ok};
use crate::api::reins::org::sigv4::{self, AwsCreds};

/// The longest object lock (100 years).
pub const MAX_LOCK_DAYS: u32 = 36_500;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// `https://s3.<region>.amazonaws.com` when absent; `https://storage.googleapis.com` for GCS.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    pub region: String,
    pub bucket: String,
    #[serde(default)]
    pub prefix: String,
    pub access_key_id: String,
    pub secret_access_key: String,
    #[serde(default)]
    pub lock_days: u32,
}

impl Config {
    pub fn check(&self) -> Result<(), OrgError> {
        if let Some(e) = &self.endpoint {
            url_ok(e)?;
        }
        let region_ok = !self.region.is_empty()
            && self.region.len() <= 32
            && self.region.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        let bucket_ok = (3..=63).contains(&self.bucket.len())
            && self.bucket.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.');
        let prefix_ok = self.prefix.len() <= 200
            && !self.prefix.contains("..")
            && !self.prefix.starts_with('/')
            && self.prefix.bytes().all(|b| b.is_ascii_alphanumeric() || b"/_.-".contains(&b));
        if !region_ok || !bucket_ok || !prefix_ok {
            return Err(OrgError::Invalid("Check the region, bucket and prefix."));
        }
        if !field_ok(&self.access_key_id, 128) || !field_ok(&self.secret_access_key, 256) {
            return Err(OrgError::Invalid("Fill in every field."));
        }
        if self.lock_days > MAX_LOCK_DAYS {
            return Err(OrgError::Invalid("Lock objects for at most 100 years."));
        }
        Ok(())
    }

    fn base(&self) -> String {
        self.endpoint.clone().unwrap_or_else(|| format!("https://s3.{}.amazonaws.com", self.region))
    }

    /// The object of the segment after the checkpoint at `after` up to the one at `to`.
    #[must_use]
    pub fn object_url(&self, org: &str, after: i64, to: i64) -> String {
        format!("{}/{}/{}{org}/{after:012}-{to:012}.jsonl", self.base().trim_end_matches('/'), self.bucket, self.prefix)
    }
}

/// `YYYY-MM-DDTHH:MM:SSZ`.
fn rfc3339(unix: i64) -> String {
    chrono::DateTime::from_timestamp(unix, 0).unwrap_or_default().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

#[allow(clippy::too_many_arguments, reason = "one segment and where it goes")]
pub async fn deliver(
    t: &dyn Transport,
    cfg: &Config,
    org: &str,
    after: i64,
    to: i64,
    text: &str,
    now: i64,
) -> Result<(), String> {
    let url = cfg.object_url(org, after, to);
    let parsed = url::Url::parse(&url).map_err(|_| "not a URL".to_owned())?;
    let body = text.as_bytes().to_vec();
    let md5 = openssl::hash::hash(openssl::hash::MessageDigest::md5(), &body).map_err(|_| "no MD5")?;
    let sha = sigv4::hex_sha256(&body);
    let mut h: Vec<(String, String)> = vec![
        ("content-type".into(), "application/x-ndjson".into()),
        ("content-md5".into(), data_encoding::BASE64.encode(&md5)),
        ("x-amz-content-sha256".into(), sha.clone()),
    ];
    if cfg.lock_days > 0 {
        h.push(("x-amz-object-lock-mode".into(), "COMPLIANCE".into()));
        h.push(("x-amz-object-lock-retain-until-date".into(), rfc3339(now + i64::from(cfg.lock_days) * 86_400)));
    }
    let creds = AwsCreds {
        access_key_id: cfg.access_key_id.clone(),
        secret_access_key: Zeroizing::new(cfg.secret_access_key.clone()),
        session_token: None,
    };
    sigv4::sign("PUT", &parsed, &mut h, &sha, &cfg.region, "s3", &creds, &sigv4::amz_date(now));
    let pairs: Vec<(&str, String)> =
        h.iter().filter(|(k, _)| k != "host").map(|(k, v)| (k.as_str(), v.clone())).collect();
    let (status, _) = t.send(Method::PUT, &url, headers(&pairs)?, body).await?;
    if (200..300).contains(&status) {
        Ok(())
    } else {
        Err(refused(&url, status))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> Config {
        Config {
            endpoint: None,
            region: "eu-west-1".into(),
            bucket: "acme-audit".into(),
            prefix: "reins/".into(),
            access_key_id: "AKID".into(),
            secret_access_key: "secret".into(),
            lock_days: 365,
        }
    }

    #[test]
    fn objects_are_named_by_their_segment() {
        assert_eq!(
            cfg().object_url("o1", 0, 42),
            "https://s3.eu-west-1.amazonaws.com/acme-audit/reins/o1/000000000000-000000000042.jsonl"
        );
        let mut bad = cfg();
        bad.prefix = "../x".into();
        assert!(bad.check().is_err());
        bad = cfg();
        bad.bucket = "Acme".into();
        assert!(bad.check().is_err());
        bad = cfg();
        bad.lock_days = MAX_LOCK_DAYS + 1;
        assert!(bad.check().is_err());
        assert!(cfg().check().is_ok());
        assert_eq!(rfc3339(1_793_102_400), "2026-10-27T12:00:00Z");
    }
}
