//! The API proxy: an agent calls `http://127.0.0.1:7457/api/<name>/<path>` without any key, and the daemon forwards to
//! `<base>/<path>` with the configured header filled with a secret the phone released for that API. The secret is kept
//! in memory until its lease ends (or the API answers 401), never logged, never put in an answer. Requests and answers
//! stream in both directions. The daemon's Host and Origin checks apply before a request gets here.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use http_body_util::BodyExt as _;
use hyper::body::Incoming;
use hyper::header::{self, HeaderMap, HeaderName, HeaderValue};
use hyper::{Request, Response, StatusCode};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::auth::Refusal;
use crate::journal::{Entry, Kind};
use crate::notice::{self, Peer};
use crate::phone::{PhoneLink, awaiting, teller};
use crate::proxy::{Body, BoxError, text};
use crate::secrets::{self, MAX_LEASE_SECS, MIN_LEASE_SECS, SecretRef};
use crate::stats::Stats;

/// Requests under this path go to the API proxy.
pub const PREFIX: &str = "/api/";
const DEFAULT_HEADER: &str = "Authorization: Bearer {secret}";
const PLACEHOLDER: &str = "{secret}";
const DEFAULT_LEASE_SECS: u32 = 3_600;

/// Headers that belong to one connection, never forwarded.
const HOP_BY_HOP: [&str; 9] = [
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "proxy-connection",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];

/// One `[[api]]` entry in `config.toml`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApiConfig {
    /// The path part after `/api/` (letters, digits, `-`, `_`).
    pub name: String,
    /// Where requests go (https; http only for localhost).
    pub base: String,
    /// The header added, with `{secret}` where the secret goes.
    #[serde(default = "default_header")]
    pub header: String,
    /// `vault:Item/field`.
    pub secret: String,
    /// How long the daemon keeps the secret (60 to 86400 seconds).
    #[serde(default = "default_lease")]
    pub lease_secs: u32,
}

fn default_header() -> String {
    DEFAULT_HEADER.to_owned()
}

fn default_lease() -> u32 {
    DEFAULT_LEASE_SECS
}

fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 40 && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// The header's name and the parts of its value around `{secret}`.
fn split_header(template: &str) -> Result<(HeaderName, String, String), String> {
    let (name, value) = template.split_once(':').ok_or("must look like `Name: value with {secret}`")?;
    let name = HeaderName::from_bytes(name.trim().as_bytes()).map_err(|_| "has an invalid header name")?;
    if HOP_BY_HOP.contains(&name.as_str()) || name == header::HOST || name == header::CONTENT_LENGTH {
        return Err(format!("cannot set `{name}`"));
    }
    let value = value.trim();
    if value.matches(PLACEHOLDER).count() != 1 {
        return Err("must contain `{secret}` once".to_owned());
    }
    if value.chars().any(char::is_control) {
        return Err("must not contain control characters".to_owned());
    }
    let (before, after) = value.split_once(PLACEHOLDER).unwrap_or((value, ""));
    Ok((name, before.to_owned(), after.to_owned()))
}

/// Checks every `[[api]]` entry (when the config loads).
pub fn validate(apis: &[ApiConfig]) -> Result<(), String> {
    let mut names = std::collections::HashSet::new();
    for a in apis {
        if !valid_name(&a.name) {
            return Err(format!("`api.name` `{}`: 1 to 40 letters, digits, `-` or `_`", a.name));
        }
        if !names.insert(a.name.to_ascii_lowercase()) {
            return Err(format!("`api.name` `{}` appears twice", a.name));
        }
        let base = url::Url::parse(&a.base).map_err(|e| format!("`api.base` of {}: {e}", a.name))?;
        crate::server::check_url(&base).map_err(|e| format!("`api.base` of {}: {e}", a.name))?;
        if base.query().is_some() || base.fragment().is_some() {
            return Err(format!("`api.base` of {}: no query or fragment", a.name));
        }
        split_header(&a.header).map_err(|e| format!("`api.header` of {}: {e}", a.name))?;
        SecretRef::parse(&a.secret).map_err(|e| format!("`api.secret` of {}: {e}", a.name))?;
        if !(MIN_LEASE_SECS..=MAX_LEASE_SECS).contains(&a.lease_secs) {
            return Err(format!("`api.lease_secs` of {}: {MIN_LEASE_SECS} to {MAX_LEASE_SECS}", a.name));
        }
    }
    Ok(())
}

struct Api {
    name: String,
    base: url::Url,
    header: HeaderName,
    before: String,
    after: String,
    secret: SecretRef,
    lease_secs: u32,
}

struct Lease {
    value: Zeroizing<String>,
    expires_at: i64,
}

pub struct ApiProxy {
    apis: HashMap<String, Api>,
    phone: Arc<PhoneLink>,
    http: reqwest::Client,
    leases: Mutex<HashMap<String, Lease>>,
    /// One question per API at a time: concurrent requests wait for the same answer.
    turns: HashMap<String, tokio::sync::Mutex<()>>,
    stats: Arc<Stats>,
}

/// `rest` of `/api/<name>/<rest>` when it is a plain path: no `.`/`..` segments (also percent-encoded), backslashes,
/// or control characters.
fn safe_rest(rest: &str) -> bool {
    rest.split('/').all(|seg| {
        let lower = seg.to_ascii_lowercase().replace("%2e", ".");
        lower != "." && lower != ".."
    }) && !rest.contains('\\')
        && !rest.to_ascii_lowercase().contains("%2f")
        && !rest.to_ascii_lowercase().contains("%5c")
        && !rest.chars().any(char::is_control)
}

fn refusal(r: &Refusal) -> Response<Body> {
    let (status, prefix) = match r {
        Refusal::Denied(_) => (StatusCode::FORBIDDEN, "Denied"),
        Refusal::Waiting(_) => (StatusCode::SERVICE_UNAVAILABLE, "Waiting"),
        Refusal::Unavailable(_) => (StatusCode::SERVICE_UNAVAILABLE, "Unavailable"),
    };
    let mut resp = text(status, &format!("Reins: {prefix}: {}", r.message()));
    if matches!(r, Refusal::Waiting(_)) {
        resp.headers_mut().insert(header::RETRY_AFTER, HeaderValue::from_static("10"));
    }
    resp
}

impl ApiProxy {
    pub fn new(apis: &[ApiConfig], phone: Arc<PhoneLink>, stats: Arc<Stats>) -> Result<Self, String> {
        validate(apis)?;
        let mut map = HashMap::new();
        let mut turns = HashMap::new();
        for a in apis {
            let (header, before, after) = split_header(&a.header)?;
            let base = url::Url::parse(a.base.trim_end_matches('/')).map_err(|e| e.to_string())?;
            let key = a.name.to_ascii_lowercase();
            turns.insert(key.clone(), tokio::sync::Mutex::new(()));
            map.insert(
                key,
                Api {
                    name: a.name.clone(),
                    base,
                    header,
                    before,
                    after,
                    secret: SecretRef::parse(&a.secret)?,
                    lease_secs: a.lease_secs,
                },
            );
        }
        Ok(Self {
            apis: map,
            phone,
            http: crate::http::client(None)?,
            leases: Mutex::new(HashMap::new()),
            turns,
            stats,
        })
    }

    fn cached(&self, key: &str) -> Option<Zeroizing<String>> {
        let mut leases = self.leases.lock().unwrap_or_else(PoisonError::into_inner);
        let now = crate::now_unix();
        leases.retain(|_, l| now < l.expires_at);
        leases.get(key).map(|l| l.value.clone())
    }

    /// The APIs, with when the key the phone released for each runs out (`None`: none held). Never the key.
    #[must_use]
    pub fn leases(&self) -> Vec<(String, String, Option<i64>)> {
        let mut leases = self.leases.lock().unwrap_or_else(PoisonError::into_inner);
        let now = crate::now_unix();
        leases.retain(|_, l| now < l.expires_at);
        let mut v: Vec<(String, String, Option<i64>)> = self
            .apis
            .iter()
            .map(|(key, a)| (a.name.clone(), a.base.to_string(), leases.get(key).map(|l| l.expires_at)))
            .collect();
        v.sort();
        v
    }

    fn forget(&self, key: &str) {
        self.leases.lock().unwrap_or_else(PoisonError::into_inner).remove(key);
    }

    /// The API's secret: the leased one, or a new release from the phone.
    async fn secret(&self, key: &str, api: &Api, peer: Option<Peer>) -> Result<Zeroizing<String>, Refusal> {
        if let Some(v) = self.cached(key) {
            return Ok(v);
        }
        let _turn = match self.turns.get(key) {
            Some(t) => Some(t.lock().await),
            None => None,
        };
        if let Some(v) = self.cached(key) {
            return Ok(v);
        }
        let phone = self.phone.phone()?;
        let command = format!("API {}", api.name);
        let purpose = format!("Requests to {} through the Reins proxy", api.base.host_str().unwrap_or("the API"));
        let refs = [api.secret.clone()];
        let request = secrets::Request {
            secrets: &refs,
            command: &command,
            purpose: Some(&purpose),
            lease_secs: api.lease_secs,
        };
        let tell = peer.map(|p| teller(move |line: &str| notice::tell(p, line)));
        let reuse = format!("api:{key}");
        let what = format!("the key for API {}", api.name);
        let entry = Entry::new(Kind::Api, &what)
            .service(Some(&api.name))
            .detail(Some(&format!("{purpose}\nSecret: {}\nLease: {} s", api.secret, api.lease_secs)));
        let mut released = awaiting(
            self.phone.journal(),
            entry,
            tell,
            self.phone.timeout(),
            secrets::release(
                &phone,
                &request,
                Some(&reuse),
                "Waiting for approval on your phone. Approve it, then send the request again.",
            ),
        )
        .await?;
        let value =
            released.values.pop().ok_or_else(|| Refusal::Unavailable("The phone released nothing.".to_owned()))?;
        let expires_at = released.expires_at.min(crate::now_unix() + i64::from(api.lease_secs));
        self.leases.lock().unwrap_or_else(PoisonError::into_inner).insert(
            key.to_owned(),
            Lease {
                value: value.clone(),
                expires_at,
            },
        );
        Ok(value)
    }

    /// Handles a request under [`PREFIX`].
    pub async fn handle(&self, req: Request<Incoming>) -> Response<Body> {
        let path = req.uri().path().to_owned();
        let Some(after) = path.strip_prefix(PREFIX) else {
            return text(StatusCode::NOT_FOUND, "Not an API path.");
        };
        let (name, rest) = after.split_once('/').unwrap_or((after, ""));
        let key = name.to_ascii_lowercase();
        let Some(api) = self.apis.get(&key) else {
            return text(StatusCode::NOT_FOUND, &format!("No API `{name}` in config.toml (`[[api]]`)."));
        };
        if !safe_rest(rest) {
            return text(StatusCode::BAD_REQUEST, "Refused: that path leaves the API.");
        }
        let mut url = api.base.clone();
        let base_path = api.base.path().trim_end_matches('/').to_owned();
        url.set_path(&format!("{base_path}/{rest}"));
        url.set_query(req.uri().query());
        let method = req.method().clone();
        let peer = req.extensions().get::<Peer>().copied();
        let secret = match self.secret(&key, api, peer).await {
            Ok(s) => s,
            Err(r) => {
                log::info!("{method} api {}: secret {}", api.name, kind(&r));
                self.stats.record("api", &api.name, kind(&r));
                return refusal(&r);
            }
        };
        let (parts, body) = req.into_parts();
        let mut out = self.http.request(method.clone(), url);
        let mut headers = HeaderMap::new();
        for (n, v) in &parts.headers {
            if HOP_BY_HOP.contains(&n.as_str()) || *n == header::HOST || *n == api.header {
                continue;
            }
            headers.append(n.clone(), v.clone());
        }
        let filled = Zeroizing::new(format!("{}{}{}", api.before, secret.as_str(), api.after));
        drop(secret);
        let Ok(mut value) = HeaderValue::from_str(&filled) else {
            return text(StatusCode::BAD_GATEWAY, "The released secret cannot go in an HTTP header.");
        };
        drop(filled);
        value.set_sensitive(true);
        headers.insert(api.header.clone(), value);
        let has_body =
            parts.headers.contains_key(header::CONTENT_LENGTH) || parts.headers.contains_key(header::TRANSFER_ENCODING);
        out = out.headers(headers);
        if has_body {
            out = out.body(reqwest::Body::wrap_stream(body.into_data_stream()));
        }
        let answer = match out.send().await {
            Ok(r) => r,
            Err(e) => {
                let e = e.without_url();
                log::warn!("{method} api {}: upstream: {e}", api.name);
                self.stats.record("api", &api.name, "unreachable");
                return text(StatusCode::BAD_GATEWAY, &format!("Cannot reach the API: {e}"));
            }
        };
        let status = answer.status();
        log::info!("{method} api {}: {}", api.name, status.as_u16());
        self.stats.record("api", &api.name, &format!("{method} {}", status.as_u16()));
        if status == StatusCode::UNAUTHORIZED {
            // A revoked or rotated key: the next request asks the phone again.
            self.forget(&key);
        }
        forward(answer)
    }
}

fn kind(r: &Refusal) -> &'static str {
    match r {
        Refusal::Denied(_) => "denied",
        Refusal::Waiting(_) => "waiting",
        Refusal::Unavailable(_) => "unavailable",
    }
}

fn forward(resp: reqwest::Response) -> Response<Body> {
    let status = resp.status();
    let mut headers = HeaderMap::new();
    for (n, v) in resp.headers() {
        if !HOP_BY_HOP.contains(&n.as_str()) {
            headers.append(n.clone(), v.clone());
        }
    }
    let body = reqwest::Body::from(resp).map_err(BoxError::from).boxed_unsync();
    let mut r = Response::new(body);
    *r.status_mut() = status;
    *r.headers_mut() = headers;
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    fn api(name: &str, base: &str, header: &str) -> ApiConfig {
        ApiConfig {
            name: name.to_owned(),
            base: base.to_owned(),
            header: header.to_owned(),
            secret: "vault:OpenAI/password".to_owned(),
            lease_secs: 3600,
        }
    }

    #[test]
    fn api_entries_are_checked() {
        validate(&[api("openai", "https://api.openai.com/v1", DEFAULT_HEADER)]).unwrap();
        validate(&[api("local", "http://127.0.0.1:9000", "X-Api-Key: {secret}")]).unwrap();
        for bad in [
            vec![api("open ai", "https://a.example", DEFAULT_HEADER)],
            vec![api("a", "http://a.example", DEFAULT_HEADER)],
            vec![api("a", "https://a.example/?q=1", DEFAULT_HEADER)],
            vec![api("a", "https://a.example", "Authorization: Bearer")],
            vec![api("a", "https://a.example", "Authorization {secret}")],
            vec![api("a", "https://a.example", "Host: {secret}")],
            vec![api("a", "https://a.example", "X: {secret}{secret}")],
            vec![api("a", "https://a.example", DEFAULT_HEADER), api("A", "https://b.example", DEFAULT_HEADER)],
        ] {
            assert!(validate(&bad).is_err(), "{bad:?}");
        }
        let mut short = api("a", "https://a.example", DEFAULT_HEADER);
        short.lease_secs = 10;
        assert!(validate(&[short]).is_err());
        let toml_entry: crate::config::Config = toml::from_str(
            "[[api]]\nname = \"openai\"\nbase = \"https://api.openai.com\"\nsecret = \"vault:OpenAI/password\"\n",
        )
        .unwrap();
        assert_eq!(toml_entry.api[0].header, DEFAULT_HEADER);
        assert_eq!(toml_entry.api[0].lease_secs, DEFAULT_LEASE_SECS);
        toml_entry.validate().unwrap();
    }

    #[test]
    fn the_header_template_splits_around_the_secret() {
        let (n, b, a) = split_header("Authorization: Bearer {secret}").unwrap();
        assert_eq!((n.as_str(), b.as_str(), a.as_str()), ("authorization", "Bearer ", ""));
        let (n, b, a) = split_header("x-goog-api-key:{secret}").unwrap();
        assert_eq!((n.as_str(), b.as_str(), a.as_str()), ("x-goog-api-key", "", ""));
    }

    #[test]
    fn paths_cannot_climb_out_of_the_base() {
        for ok in ["", "v1/models", "v1/chat/completions", "a.b/c-d_e", "files/%20x"] {
            assert!(safe_rest(ok), "{ok}");
        }
        for bad in ["..", "v1/../x", "v1/%2e%2e/x", "v1/%2E./x", ".", "a\\b", "a%2fb", "a%5Cb", "a\nb"] {
            assert!(!safe_rest(bad), "{bad}");
        }
    }
}
