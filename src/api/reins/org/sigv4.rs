//! AWS Signature Version 4 (`Authorization: AWS4-HMAC-SHA256 …`), for the S3 audit sink (and S3-compatible storage:
//! GCS with HMAC keys, MinIO) and the AWS KMS key wrapper. Only what those need: header signing, a payload hash the
//! caller gives, path segments already safe (the callers build them from `[A-Za-z0-9._/-]`).

use ring::{digest, hmac};
use url::Url;
use zeroize::Zeroizing;

/// AWS credentials (static, or a role's temporary ones with their session token).
#[derive(Clone)]
pub struct AwsCreds {
    pub access_key_id: String,
    pub secret_access_key: Zeroizing<String>,
    pub session_token: Option<String>,
}

impl std::fmt::Debug for AwsCreds {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AwsCreds").field("access_key_id", &self.access_key_id).finish_non_exhaustive()
    }
}

/// SHA-256 of `bytes`, lowercase hex.
#[must_use]
pub fn hex_sha256(bytes: &[u8]) -> String {
    data_encoding::HEXLOWER.encode(digest::digest(&digest::SHA256, bytes).as_ref())
}

fn hmac_sha256(key: &[u8], msg: &[u8]) -> Vec<u8> {
    hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, key), msg).as_ref().to_vec()
}

/// RFC 3986 percent-encoding of everything but the unreserved characters (AWS's `UriEncode`).
fn uri_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(b));
        } else {
            {
                use std::fmt::Write as _;
                let _ = write!(out, "%{b:02X}");
            }
        }
    }
    out
}

fn canonical_query(url: &Url) -> String {
    let mut pairs: Vec<(String, String)> = url.query_pairs().map(|(k, v)| (uri_encode(&k), uri_encode(&v))).collect();
    pairs.sort();
    pairs.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&")
}

fn host_header(url: &Url) -> String {
    let host = url.host_str().unwrap_or_default();
    match url.port() {
        Some(p) => format!("{host}:{p}"),
        None => host.to_owned(),
    }
}

/// Signs a request: adds `host` (when missing), `x-amz-date`, `x-amz-security-token` (with temporary credentials)
/// and `authorization` to `headers`. Every header in `headers` is signed. `payload_sha256`: the body's hex SHA-256
/// (callers add `x-amz-content-sha256` themselves where the service wants it, as S3 does). `amz_date`:
/// `YYYYMMDDTHHMMSSZ`.
#[allow(clippy::too_many_arguments, reason = "the inputs of AWS's signing algorithm, each one distinct")]
pub fn sign(
    method: &str,
    url: &Url,
    headers: &mut Vec<(String, String)>,
    payload_sha256: &str,
    region: &str,
    service: &str,
    creds: &AwsCreds,
    amz_date: &str,
) {
    let has = |hs: &[(String, String)], n: &str| hs.iter().any(|(k, _)| k.eq_ignore_ascii_case(n));
    if !has(headers, "host") {
        headers.push(("host".to_owned(), host_header(url)));
    }
    headers.push(("x-amz-date".to_owned(), amz_date.to_owned()));
    if let Some(t) = &creds.session_token {
        headers.push(("x-amz-security-token".to_owned(), t.clone()));
    }
    let mut canon: Vec<(String, String)> = headers
        .iter()
        .map(|(k, v)| (k.to_ascii_lowercase(), v.split_whitespace().collect::<Vec<_>>().join(" ")))
        .collect();
    canon.sort();
    let signed: String = canon.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>().join(";");
    let mut canonical_headers = String::new();
    for (k, v) in &canon {
        canonical_headers.push_str(k);
        canonical_headers.push(':');
        canonical_headers.push_str(v);
        canonical_headers.push('\n');
    }
    let path = if url.path().is_empty() {
        "/"
    } else {
        url.path()
    };
    let request =
        format!("{method}\n{path}\n{}\n{canonical_headers}\n{signed}\n{payload_sha256}", canonical_query(url));
    let date = amz_date.get(..8).unwrap_or_default();
    let scope = format!("{date}/{region}/{service}/aws4_request");
    let to_sign = format!("AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}", hex_sha256(request.as_bytes()));
    let mut key = hmac_sha256(format!("AWS4{}", creds.secret_access_key.as_str()).as_bytes(), date.as_bytes());
    for part in [region, service, "aws4_request"] {
        key = hmac_sha256(&key, part.as_bytes());
    }
    let signature = data_encoding::HEXLOWER.encode(&hmac_sha256(&key, to_sign.as_bytes()));
    headers.push((
        "authorization".to_owned(),
        format!(
            "AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed}, Signature={signature}",
            creds.access_key_id
        ),
    ));
}

/// `YYYYMMDDTHHMMSSZ` for a Unix time.
#[must_use]
pub fn amz_date(unix: i64) -> String {
    chrono::DateTime::from_timestamp(unix, 0).unwrap_or_default().format("%Y%m%dT%H%M%SZ").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn example_creds() -> AwsCreds {
        AwsCreds {
            access_key_id: "AKIDEXAMPLE".into(),
            secret_access_key: Zeroizing::new("wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY".into()),
            session_token: None,
        }
    }

    #[test]
    fn sigv4_matches_the_aws_example() {
        // AWS's documented example ("Examples of the complete Signature Version 4 signing process", IAM ListUsers).
        let url = Url::parse("https://iam.amazonaws.com/?Action=ListUsers&Version=2010-05-08").unwrap();
        let mut headers =
            vec![("Content-Type".to_owned(), "application/x-www-form-urlencoded; charset=utf-8".to_owned())];
        sign("GET", &url, &mut headers, &hex_sha256(b""), "us-east-1", "iam", &example_creds(), "20150830T123600Z");
        let auth = &headers.iter().find(|(k, _)| k == "authorization").unwrap().1;
        assert_eq!(
            auth,
            "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/iam/aws4_request, \
             SignedHeaders=content-type;host;x-amz-date, \
             Signature=5d672d79c15b13162d9279b0855cfba6789a8edb4c82c400e06b5924a6f2b5d7"
        );
    }

    #[test]
    fn query_and_path_are_canonical() {
        let url = Url::parse("https://s3.example.com:9000/bucket/a/b.jsonl?z=1&a=x%20y&a=b").unwrap();
        assert_eq!(canonical_query(&url), "a=b&a=x%20y&z=1");
        assert_eq!(host_header(&url), "s3.example.com:9000");
        let mut temp = example_creds();
        temp.session_token = Some("tok".into());
        let mut headers = Vec::new();
        sign("PUT", &url, &mut headers, &hex_sha256(b"x"), "eu-west-1", "s3", &temp, "20261027T120000Z");
        let names: Vec<&str> = headers.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(names, ["host", "x-amz-date", "x-amz-security-token", "authorization"]);
        assert!(headers[3].1.contains("SignedHeaders=host;x-amz-date;x-amz-security-token"));
        assert_eq!(amz_date(1_793_102_400), "20261027T120000Z");
    }
}
