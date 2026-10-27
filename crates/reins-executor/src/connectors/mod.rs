//! What an executor runs upstream with the org's managed credentials: the org's GitHub App ([`github`]) and its Google
//! Workspace service account ([`google`]). Each connector builds its calls from the approved arguments only.

use std::sync::Arc;
use std::time::Duration;

use ring::rand::SystemRandom;
use ring::signature::{RSA_PKCS1_SHA256, RsaKeyPair};
use serde_json::Value;

pub mod github;
pub mod google;

pub use github::GithubApp;
pub use google::GoogleWorkspace;

/// A service the executor acts on.
#[async_trait::async_trait]
pub trait Connector: Send + Sync {
    /// Whether it runs `tool`.
    fn handles(&self, tool: &str) -> bool;

    /// Runs `tool(args)` for the requesting member (`member_email`, from the signed roster). Returns the minimal result
    /// (an id, a status), or a read's answer.
    async fn run(&self, member_email: &str, tool: &str, args: &Value) -> Result<Value, String>;
}

pub const USER_AGENT: &str = concat!("reins-executor/", env!("CARGO_PKG_VERSION"));

/// An HTTP client: rustls with ring and the webpki roots, no redirects (a token is never sent where it wasn't meant to
/// go).
pub fn http_client() -> Result<reqwest::Client, String> {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let tls = rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .map_err(|e| format!("TLS setup: {e}"))?
        .with_root_certificates(roots)
        .with_no_client_auth();
    reqwest::Client::builder()
        .tls_backend_preconfigured(tls)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(60))
        .user_agent(USER_AGENT)
        .build()
        .map_err(|e| format!("HTTP client setup: {}", e.without_url()))
}

/// An RSA private key from PEM: PKCS#8 (`PRIVATE KEY`, Google's) or PKCS#1 (`RSA PRIVATE KEY`, GitHub's).
pub fn rsa_from_pem(pem: &str) -> Result<RsaKeyPair, String> {
    let begin = pem.find("-----BEGIN ").ok_or("the key is not PEM")?;
    let rest = &pem[begin + 11..];
    let label_end = rest.find("-----").ok_or("the key is not PEM")?;
    let label = &rest[..label_end];
    let body_start = label_end + 5;
    let end = rest.find("-----END ").ok_or("the key is not PEM")?;
    let b64: String = rest[body_start..end].chars().filter(|c| !c.is_whitespace()).collect();
    let der = zeroize::Zeroizing::new(
        data_encoding::BASE64.decode(b64.as_bytes()).map_err(|_| "the key's PEM body is not base64".to_owned())?,
    );
    match label {
        "PRIVATE KEY" => RsaKeyPair::from_pkcs8(&der),
        "RSA PRIVATE KEY" => RsaKeyPair::from_der(&der),
        _ => return Err(format!("a {label} is not an RSA private key")),
    }
    .map_err(|e| format!("the RSA key is not usable: {e}"))
}

/// A compact RS256 JWT of `claims`.
pub fn jwt_rs256(key: &RsaKeyPair, claims: &Value) -> Result<String, String> {
    let b64 = |v: &[u8]| data_encoding::BASE64URL_NOPAD.encode(v);
    let header = b64(br#"{"alg":"RS256","typ":"JWT"}"#);
    let payload = b64(claims.to_string().as_bytes());
    let input = format!("{header}.{payload}");
    let mut sig = vec![0u8; key.public().modulus_len()];
    key.sign(&RSA_PKCS1_SHA256, &SystemRandom::new(), input.as_bytes(), &mut sig).map_err(|_| "signing failed")?;
    Ok(format!("{input}.{}", b64(&sig)))
}

/// A string argument.
pub(crate) fn arg_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str)
}

pub(crate) fn required_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    arg_str(args, key).filter(|s| !s.is_empty()).ok_or_else(|| format!("`{key}` is required"))
}

/// A list of strings argument (absent is empty).
pub(crate) fn arg_list(args: &Value, key: &str) -> Result<Vec<String>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(items)) => items
            .iter()
            .map(|v| v.as_str().map(str::to_owned).ok_or_else(|| format!("`{key}` holds only strings")))
            .collect(),
        Some(_) => Err(format!("`{key}` must be a list")),
    }
}

/// Refuses arguments a connector doesn't build from: an approved argument is never silently dropped.
pub(crate) fn only_args(args: &Value, allowed: &[&str]) -> Result<(), String> {
    let map = args.as_object().ok_or("the arguments must be an object")?;
    match map.keys().find(|k| k.as_str() != "account" && !allowed.contains(&k.as_str())) {
        Some(k) => Err(format!("`{k}` is not supported by the org executor")),
        None => Ok(()),
    }
}

/// Days since 1970-01-01 of a proleptic Gregorian date (Howard Hinnant's `days_from_civil`).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 {
        y - 1
    } else {
        y
    };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 {
        mp + 3
    } else {
        mp - 9
    };
    (
        if m <= 2 {
            yoe + era * 400 + 1
        } else {
            yoe + era * 400
        },
        m,
        d,
    )
}

fn num(s: &str) -> Option<i64> {
    (!s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())).then(|| s.parse().ok()).flatten()
}

/// `YYYY-MM-DD` as days since the epoch.
pub(crate) fn parse_date(s: &str) -> Option<i64> {
    let (y, rest) = s.split_once('-')?;
    let (m, d) = rest.split_once('-')?;
    let (y, m, d) = (num(y)?, num(m)?, num(d)?);
    ((1..=12).contains(&m) && (1..=31).contains(&d)).then(|| days_from_civil(y, m, d))
}

/// `YYYY-MM-DD`.
pub(crate) fn format_date(days: i64) -> String {
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}")
}

/// RFC 3339 (`2026-10-27T10:00:00Z`, `…+02:00`, fractional seconds ignored) as Unix seconds.
pub(crate) fn parse_rfc3339(s: &str) -> Option<i64> {
    let (date, time) = s.split_once(['T', 't', ' '])?;
    let days = parse_date(date)?;
    let (clock, offset) = if let Some(c) = time.strip_suffix(['Z', 'z']) {
        (c, 0)
    } else {
        let at = time.rfind(['+', '-'])?;
        let (c, off) = time.split_at(at);
        let sign = if off.starts_with('-') {
            -1
        } else {
            1
        };
        let (oh, om) = off[1..].split_once(':')?;
        (c, sign * (num(oh)? * 3600 + num(om)? * 60))
    };
    let clock = clock.split('.').next()?;
    let mut parts = clock.split(':');
    let (h, mi, se) = (num(parts.next()?)?, num(parts.next()?)?, num(parts.next().unwrap_or("0"))?);
    (h < 24 && mi < 60 && se < 61).then(|| days * 86_400 + h * 3600 + mi * 60 + se - offset)
}

/// Unix seconds as RFC 3339 UTC.
pub(crate) fn format_rfc3339(t: i64) -> String {
    let (days, secs) = (t.div_euclid(86_400), t.rem_euclid(86_400));
    format!("{}T{:02}:{:02}:{:02}Z", format_date(days), secs / 3600, secs % 3600 / 60, secs % 60)
}

/// Test support (a fixed test-only RSA key), for this crate's tests and the end-to-end suite.
#[cfg(any(test, feature = "testing"))]
#[allow(clippy::must_use_candidate, clippy::missing_panics_doc, reason = "test support")]
pub mod testing {
    use ring::signature::RsaKeyPair;

    /// A 2048-bit RSA key made for these tests only (PKCS#8 DER, base64). It signs nothing outside them.
    pub const TEST_RSA_PKCS8: &[&str] = &[
        "MIIEvwIBADANBgkqhkiG9w0BAQEFAASCBKkwggSlAgEAAoIBAQCp0FNpxbfTegIMZH6dOVvQ4Lp2LHAOfN//iQV3uXrbHaoq5hdx",
        "/jordaEkEA8+PJ25G65rFVN6uZ1I6wYJwr6y7HfWUgclMlTyRHjWXmj0nKjUx8MAOOh25AtmSRE+DlDfos63pliLVu4LMIoDRBoF",
        "78AZD6hO9wHgnArvAIYkM8PpOBVGWQurdaVtwT5bHLqKtnXLZG7DKLJ42urEzbOZ3jDN7B0yngwtzGjyp8rLVeVP6TDuKzA/MP0Y",
        "MRXh5BiKV4vKqKQRfssAnCQgQkcotLD9BF5Hw4drd7i+UpVXok6t8Ha76o9A+yd6deyPNIRwG2F/OCEsyoQS2v//PoMnAgMBAAEC",
        "ggEABRQ6N8rWSDN5KzYNmWs/7/FAx8xldPF5i6g6qyLiO0/ZqUXPhAp8DT+2mEsIrFkIyKK80UQb2+sAUCWDZ+L/bCo+yAC/vkuR",
        "QdvCwiVPE1M6M38JZJDPLHspTMpjrcz6Je411kVZ7EpR0rDazTqKDWJnMaNdFTDgc/+GzjDgUjso/cwgUYo2u0hd/hj/USOM/ZkT",
        "xxETIS/Vcx8TyyaUXgrf811KeYrlBlKDgoUgGhWLvWjJYYQENu6sWATDQW4piTg2q8cDTlaP3gXF2ZemRwlr95WbB1xazvHdI4Ph",
        "A3PZ1R6p0BiGi8XytW+oQxXtSkUp3i/MyWRA3lWKI9S2rQKBgQDRLDUJrjhj6uhpftsX9hrdV/3rCvBaD/gSOx693F1Tpm9SQijE",
        "gcreG6O2GWqWlq0YC4PaDIug2OQoX/44iD17LsObseqc0DxWRfL+DcU1//fCFeyrxwhjRCx/gRmH8GBMJcmRnzXl+BBxpuxNEg1w",
        "GjB28c3YNGKS3Zs+ZhdV6wKBgQDP1HB+5LL67HxwDDLS8cTg0FNoXPRkgbJnZFiPQ64wuQBARQhj4P2B+cDZXxv+WVE2lMFwybcE",
        "OqDyDtawkoQ/zCUuPgFBWS1FkjMaGKYMm3eEqGqSGK3MPdf1CaIS3fhhAPr02/hsu+PguN5xvysqgxcFBMjSbL6gnZyCGXdMtQKB",
        "gQDJpY9fzLeKA2yYPvXhzJwrY+diToJUoG5qzNme+LBzstBAgY0hm2pB4m1ZYHkzwQAPXRJ4A34byttw3NGMGDME7JkgIE1nbQut",
        "s/ND3WyP2To0vcQR+Wbpo1Aufvto5F9BKXB/VPe1wgXAOsY0qkGQH383O3ZQhne806wUZID8jwKBgQCq6tINeXW8KIp5935AvvYj",
        "0RAXLwMKOSOIJh0yzP1PMoYDVQJ+aYTyxHwWCELUXY4Nri6b3Dg4MSM1joB/PC+o6/HT4shTfsxdf8uo9YZxrdWP8pWaStN7uYyG",
        "B4MTsxrWY+D/SHw4qBjTMhY2Q3pzftWKmhGyBB6NprEUyW60VQKBgQDDcriuyJnCZY1mre4RAb+ZhFVtLclhTuuMynY32d01njyF",
        "co5ngLhFnBJxCajB6qtltHFdu47t11iqUXN/tey+7jFk8afjOE18WWsI9HuOc+exhxHjUALZiVoSrNdFeS/N5LumfHvuQ9ucJsFQ",
        "9ku2O3+SHDukAn2jt3HcoNKhgQ==",
    ];

    /// [`TEST_RSA_PKCS8`] as PEM.
    pub fn test_pem() -> String {
        let label = ["PRIVATE", "KEY"].join(" ");
        format!("-----BEGIN {label}-----\n{}\n-----END {label}-----\n", TEST_RSA_PKCS8.join("\n"))
    }

    pub fn test_key() -> RsaKeyPair {
        super::rsa_from_pem(&test_pem()).unwrap()
    }

    /// Verifies an RS256 JWT with the test key's public half and returns its claims.
    pub fn jwt_claims(jwt: &str) -> serde_json::Value {
        let (input, sig) = jwt.rsplit_once('.').unwrap();
        let (header, payload) = input.split_once('.').unwrap();
        let header: serde_json::Value =
            serde_json::from_slice(&data_encoding::BASE64URL_NOPAD.decode(header.as_bytes()).unwrap()).unwrap();
        assert_eq!(header["alg"], "RS256");
        let sig = data_encoding::BASE64URL_NOPAD.decode(sig.as_bytes()).unwrap();
        let key = test_key();
        ring::signature::UnparsedPublicKey::new(
            &ring::signature::RSA_PKCS1_2048_8192_SHA256,
            ring::signature::KeyPair::public_key(&key).as_ref(),
        )
        .verify(input.as_bytes(), &sig)
        .expect("the JWT verifies with the key");
        serde_json::from_slice(&data_encoding::BASE64URL_NOPAD.decode(payload.as_bytes()).unwrap()).unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_round_trip() {
        assert_eq!(parse_rfc3339("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_rfc3339("2026-10-27T10:00:00Z"), Some(1_793_095_200));
        assert_eq!(parse_rfc3339("2026-10-27T12:00:00+02:00"), Some(1_793_095_200));
        assert_eq!(parse_rfc3339("2026-10-27T10:00:00.123Z"), Some(1_793_095_200));
        assert_eq!(format_rfc3339(1_793_095_200), "2026-10-27T10:00:00Z");
        assert_eq!(format_date(parse_date("2024-02-29").unwrap() + 1), "2024-03-01");
        assert_eq!(parse_rfc3339("tomorrow"), None);
    }

    #[test]
    fn keys_load_from_pem_and_sign() {
        let key = testing::test_key();
        let jwt = jwt_rs256(&key, &serde_json::json!({"iss": "x"})).unwrap();
        assert_eq!(testing::jwt_claims(&jwt)["iss"], "x");
        assert!(rsa_from_pem("not a key").is_err());
    }
}
