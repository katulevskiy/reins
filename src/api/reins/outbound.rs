//! Requests the server makes for the phone (`BlobSend`, `BlobFetch`, `ProxyCall`) to URLs the phone names.
//!
//! The URLs come from an AI's tool call, so the server must not become a way into its own network (SSRF): https only,
//! every address a name resolves to must be public (no private, loopback, link-local, shared or metadata addresses),
//! checked in the resolver itself so a second DNS answer cannot differ from the checked one, and redirects are
//! followed by hand (at most [`MAX_REDIRECTS`]), each target checked again, the phone's headers dropped as soon as the
//! origin changes. Headers and bodies are never logged.

use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::{Arc, LazyLock},
    time::Duration,
};

use reqwest::{
    Client, Method, Response, StatusCode,
    dns::{Addrs, Name, Resolve, Resolving},
    header::{HeaderMap, HeaderName, HeaderValue},
    redirect::Policy,
};
use url::{Host, Url};

/// Redirects followed per request.
pub const MAX_REDIRECTS: usize = 3;
/// Most headers the phone may hand over for one request.
pub const MAX_HEADERS: usize = 50;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// No byte for this long: the other side is gone.
const READ_TIMEOUT: Duration = Duration::from_secs(60);
/// Upper bound for a whole request, long enough for a 1 GiB transfer on a slow link.
const TOTAL_TIMEOUT: Duration = Duration::from_hours(1);

/// Test-only: lets the integration tests use `http://127.0.0.1` mock servers. Never set in production; it is read
/// from the environment because it is not a setting anybody should find in the admin page.
pub const ALLOW_LOOPBACK_ENV: &str = "REINS_TEST_ALLOW_LOOPBACK";

static ALLOW_LOOPBACK: LazyLock<bool> =
    LazyLock::new(|| std::env::var(ALLOW_LOOPBACK_ENV).is_ok_and(|v| matches!(v.trim(), "true" | "1")));

pub fn allow_loopback() -> bool {
    *ALLOW_LOOPBACK
}

/// Why a request was not made or failed. The texts name hosts and statuses, never headers or bodies.
#[derive(Debug, PartialEq, Eq)]
pub enum OutboundError {
    /// The URL or a header was refused before anything was sent.
    Refused(String),
    /// The destination could not be reached or broke off.
    Failed(String),
}

impl std::fmt::Display for OutboundError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refused(m) | Self::Failed(m) => f.write_str(m),
        }
    }
}

/// Addresses known to serve cloud instance metadata; all of them are non-global as well, named here to be explicit.
fn is_metadata(ip: IpAddr) -> bool {
    const AWS_V6: Ipv6Addr = Ipv6Addr::new(0xfd00, 0xec2, 0, 0, 0, 0, 0, 0x254);
    match ip {
        IpAddr::V4(v4) => v4 == Ipv4Addr::new(169, 254, 169, 254) || v4 == Ipv4Addr::new(100, 100, 100, 200),
        IpAddr::V6(v6) => v6 == AWS_V6,
    }
}

/// Whether the server may connect to `ip` for the phone: public unicast only (loopback too in the test mode).
///
/// IPv6 forms that reach IPv4 through the network are judged by the IPv4 address inside: NAT64 (`64:ff9b::/96`,
/// RFC 6052) must embed a public one, and the deprecated IPv4-compatible form (`::a.b.c.d`) is never used. IPv4-mapped
/// addresses are made canonical first; 6to4 and Teredo are not global, so they are refused too.
pub fn address_allowed(ip: IpAddr, allow_loopback: bool) -> bool {
    let ip = ip.to_canonical();
    if is_metadata(ip) {
        return false;
    }
    if ip.is_loopback() {
        return allow_loopback;
    }
    if let IpAddr::V6(v6) = ip {
        let segments = v6.segments();
        let octets = v6.octets();
        let embedded = Ipv4Addr::new(octets[12], octets[13], octets[14], octets[15]);
        match segments[..6] {
            [0, 0, 0, 0, 0, 0] => return false,
            [0x64, 0xff9b, 0, 0, 0, 0] => return address_allowed(IpAddr::V4(embedded), false),
            _ => {}
        }
    }
    !ip.is_multicast() && crate::util::is_global(ip)
}

fn is_loopback_host(host: &Host<&str>) -> bool {
    match host {
        Host::Domain(d) => d.eq_ignore_ascii_case("localhost"),
        Host::Ipv4(ip) => ip.is_loopback(),
        Host::Ipv6(ip) => ip.is_loopback(),
    }
}

/// Parses and checks a destination URL: https (http only to loopback in the test mode), no credentials, a host that
/// is not a refused address literal. Names are checked again, address by address, when they are resolved.
pub fn check_url(raw: &str, allow_loopback: bool) -> Result<Url, OutboundError> {
    let refuse = |why: &str| Err(OutboundError::Refused(why.to_owned()));
    let Ok(mut url) = Url::parse(raw.trim()) else {
        return refuse("The URL is not valid.");
    };
    let Some(host) = url.host() else {
        return refuse("The URL has no host.");
    };
    let loopback = is_loopback_host(&host);
    match url.scheme() {
        "https" => {}
        "http" if allow_loopback && loopback => {}
        _ => return refuse("Only https URLs are allowed."),
    }
    if !url.username().is_empty() || url.password().is_some() {
        return refuse("The URL may not carry credentials.");
    }
    let literal = match host {
        Host::Ipv4(ip) => Some(IpAddr::V4(ip)),
        Host::Ipv6(ip) => Some(IpAddr::V6(ip)),
        Host::Domain(d) => {
            let d = d.trim_end_matches('.').to_ascii_lowercase();
            if (d == "localhost" || d.ends_with(".localhost")) && !allow_loopback {
                return refuse("That host is not allowed.");
            }
            None
        }
    };
    if literal.is_some_and(|ip| !address_allowed(ip, allow_loopback)) {
        return refuse("That address is not allowed.");
    }
    url.set_fragment(None);
    Ok(url)
}

/// The same origin: scheme, host and port. Headers meant for one origin never go to another.
pub fn same_origin(a: &Url, b: &Url) -> bool {
    a.scheme() == b.scheme() && a.host_str() == b.host_str() && a.port_or_known_default() == b.port_or_known_default()
}

/// Headers the server sets itself or that would change how a message is framed.
fn reserved_header(name: &HeaderName) -> bool {
    const RESERVED: [&str; 11] = [
        "host",
        "content-length",
        "transfer-encoding",
        "connection",
        "keep-alive",
        "te",
        "trailer",
        "upgrade",
        "expect",
        "proxy-authorization",
        "proxy-connection",
    ];
    RESERVED.contains(&name.as_str())
}

/// The phone's headers for one request, checked: valid names and values, no framing headers, `skip` left out (the
/// server sets those itself).
pub fn header_map(headers: &[(String, String)], skip: &[&str]) -> Result<HeaderMap, OutboundError> {
    if headers.len() > MAX_HEADERS {
        return Err(OutboundError::Refused(format!("At most {MAX_HEADERS} headers.")));
    }
    let mut map = HeaderMap::new();
    for (name, value) in headers {
        let Ok(name) = HeaderName::from_bytes(name.trim().as_bytes()) else {
            return Err(OutboundError::Refused("A header name is not valid.".to_owned()));
        };
        let Ok(value) = HeaderValue::from_str(value) else {
            return Err(OutboundError::Refused(format!("The value of header {name} is not valid.")));
        };
        if reserved_header(&name) {
            return Err(OutboundError::Refused(format!("Header {name} is set by the server.")));
        }
        if skip.contains(&name.as_str()) {
            continue;
        }
        map.append(name, value);
    }
    Ok(map)
}

/// Resolves names and keeps only allowed addresses, so the address connected to is the address checked.
struct GuardedResolver {
    allow_loopback: bool,
}

#[derive(Debug)]
struct RefusedAddress(String);

impl std::fmt::Display for RefusedAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} resolves only to addresses that are not allowed", self.0)
    }
}

impl std::error::Error for RefusedAddress {}

impl Resolve for GuardedResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let allow_loopback = self.allow_loopback;
        Box::pin(async move {
            let host = name.as_str().to_owned();
            let found = tokio::net::lookup_host((host.as_str(), 0)).await?;
            let allowed: Vec<SocketAddr> = found.filter(|a| address_allowed(a.ip(), allow_loopback)).collect();
            if allowed.is_empty() {
                let refused: Box<dyn std::error::Error + Send + Sync> = Box::new(RefusedAddress(host));
                return Err(refused);
            }
            let addrs: Addrs = Box::new(allowed.into_iter());
            Ok(addrs)
        })
    }
}

fn build_client(allow_loopback: bool) -> Client {
    Client::builder()
        .redirect(Policy::none())
        .no_proxy()
        .no_gzip()
        .no_brotli()
        .no_deflate()
        .no_zstd()
        .dns_resolver(Arc::new(GuardedResolver {
            allow_loopback,
        }))
        .connect_timeout(CONNECT_TIMEOUT)
        .read_timeout(READ_TIMEOUT)
        .timeout(TOTAL_TIMEOUT)
        .user_agent(concat!("Reins/", env!("CARGO_PKG_VERSION")))
        .build()
        .expect("the outbound HTTP client builds")
}

fn client() -> &'static Client {
    static CLIENT: LazyLock<Client> = LazyLock::new(|| build_client(allow_loopback()));
    &CLIENT
}

/// How a request body is made again for each hop (a redirect may need it twice).
pub trait BodySource {
    /// The body and its exact length, or `None` for no body.
    fn body(&self) -> impl Future<Output = Result<Option<(reqwest::Body, u64)>, OutboundError>> + Send;
}

/// No body (a GET).
pub struct NoBody;

impl BodySource for NoBody {
    fn body(&self) -> impl Future<Output = Result<Option<(reqwest::Body, u64)>, OutboundError>> + Send {
        std::future::ready(Ok(None))
    }
}

/// A small body held in memory.
pub struct BytesBody(pub bytes::Bytes);

impl BodySource for BytesBody {
    fn body(&self) -> impl Future<Output = Result<Option<(reqwest::Body, u64)>, OutboundError>> + Send {
        std::future::ready(Ok(Some((reqwest::Body::from(self.0.clone()), self.0.len() as u64))))
    }
}

fn is_redirect(status: StatusCode) -> bool {
    matches!(status.as_u16(), 301 | 302 | 303 | 307 | 308)
}

/// Makes one request for the phone, following redirects by hand.
///
/// `headers` go only to the first origin; `fixed` (content type, accept) go everywhere. 307/308 repeat the method and
/// body; 301/302/303 turn into a body-less GET unless `follow_method_change` is false, in which case that response is
/// returned as it is.
pub async fn execute<B: BodySource>(
    method: Method,
    url: &str,
    headers: &HeaderMap,
    fixed: &HeaderMap,
    body: &B,
    follow_method_change: bool,
) -> Result<Response, OutboundError> {
    let allow = allow_loopback();
    let mut url = check_url(url, allow)?;
    let mut method = method;
    let mut send_body = true;
    let mut send_headers = true;
    let mut hops = 0;
    loop {
        let mut request = client().request(method.clone(), url.clone());
        if send_headers {
            request = request.headers(headers.clone());
        }
        request = request.headers(fixed.clone());
        if send_body && let Some((body, length)) = body.body().await? {
            request = request.header(reqwest::header::CONTENT_LENGTH, length).body(body);
        }
        let host = url.host_str().unwrap_or_default().to_owned();
        let response = request.send().await.map_err(|e| describe(&host, &e))?;
        let status = response.status();
        if !is_redirect(status) || hops == MAX_REDIRECTS {
            return Ok(response);
        }
        let changes_method = !matches!(status.as_u16(), 307 | 308);
        if changes_method && !follow_method_change {
            return Ok(response);
        }
        let Some(location) = response.headers().get(reqwest::header::LOCATION).and_then(|v| v.to_str().ok()) else {
            return Ok(response);
        };
        let next =
            url.join(location).map_err(|_| OutboundError::Failed(format!("{host} redirected to an invalid URL.")))?;
        let next = check_url(next.as_str(), allow)?;
        if !same_origin(&url, &next) {
            send_headers = false;
        }
        if changes_method && method != Method::GET {
            method = Method::GET;
            send_body = false;
        }
        url = next;
        hops += 1;
    }
}

/// One GET under the same guard, without following redirects (a redirect comes back as it is) and with a total
/// `timeout`: for small documents fetched for unauthenticated callers (OAuth client metadata documents).
pub async fn get_once(url: &str, accept: &str, timeout: Duration) -> Result<Response, OutboundError> {
    let url = check_url(url, allow_loopback())?;
    let host = url.host_str().unwrap_or_default().to_owned();
    client()
        .get(url)
        .header(reqwest::header::ACCEPT, accept)
        .timeout(timeout)
        .send()
        .await
        .map_err(|e| describe(&host, &e))
}

/// A transport error in words that name the host but never the request.
fn describe(host: &str, e: &reqwest::Error) -> OutboundError {
    let mut source: Option<&dyn std::error::Error> = Some(e);
    while let Some(err) = source {
        if err.downcast_ref::<RefusedAddress>().is_some() {
            return OutboundError::Refused(format!("{host} resolves only to addresses that are not allowed."));
        }
        source = err.source();
    }
    if e.is_timeout() {
        OutboundError::Failed(format!("{host} did not answer in time."))
    } else if e.is_connect() {
        OutboundError::Failed(format!("Could not connect to {host}."))
    } else {
        OutboundError::Failed(format!("The request to {host} failed."))
    }
}

/// Reads a response body up to `limit` bytes; `true` when more was left unread.
pub async fn read_limited(mut response: Response, limit: usize) -> Result<(Vec<u8>, bool), OutboundError> {
    let mut out = Vec::new();
    while let Some(chunk) =
        response.chunk().await.map_err(|_| OutboundError::Failed("The answer broke off.".to_owned()))?
    {
        let room = limit - out.len();
        if chunk.len() > room {
            out.extend_from_slice(&chunk[..room]);
            return Ok((out, true));
        }
        out.extend_from_slice(&chunk);
    }
    Ok((out, false))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn only_public_addresses_are_allowed() {
        for ok in ["1.1.1.1", "140.82.112.3", "2606:4700:4700::1111"] {
            assert!(address_allowed(ip(ok), false), "{ok}");
        }
        for bad in [
            "127.0.0.1",
            "10.0.0.1",
            "172.16.5.4",
            "192.168.1.1",
            "169.254.169.254",
            "100.100.100.200",
            "100.64.0.1",
            "0.0.0.0",
            "224.0.0.1",
            "255.255.255.255",
            "::1",
            "::",
            "fe80::1",
            "fc00::1",
            "fd00:ec2::254",
            "::ffff:10.0.0.1",
            "::ffff:127.0.0.1",
            "ff02::1",
        ] {
            assert!(!address_allowed(ip(bad), false), "{bad}");
        }
        // Translated IPv6 is judged by the IPv4 inside; the test mode never opens it.
        for bad in ["64:ff9b::a00:1", "64:ff9b::a9fe:a9fe", "64:ff9b::7f00:1", "::a00:1", "::101:101", "2002:a00:1::1"]
        {
            assert!(!address_allowed(ip(bad), false), "{bad}");
            assert!(!address_allowed(ip(bad), true), "{bad} in the test mode");
        }
        assert!(address_allowed(ip("64:ff9b::101:101"), false), "NAT64 to a public address");
        assert!(address_allowed(ip("127.0.0.1"), true));
        assert!(address_allowed(ip("::ffff:127.0.0.1"), true));
        assert!(!address_allowed(ip("10.0.0.1"), true), "the test mode opens loopback only");
        assert!(!address_allowed(ip("169.254.169.254"), true));
    }

    #[test]
    fn urls_must_be_https_to_a_public_host() {
        assert_eq!(
            check_url("https://uploads.github.com/x?y=1#frag", false).unwrap().as_str(),
            "https://uploads.github.com/x?y=1"
        );
        for (bad, allow) in [
            ("http://example.com/", false),
            ("http://example.com/", true),
            ("ftp://example.com/", false),
            ("file:///etc/passwd", false),
            ("https://user:pw@example.com/", false),
            ("https://127.0.0.1/", false),
            ("https://[::1]/", false),
            ("https://169.254.169.254/latest/meta-data", true),
            ("https://10.1.2.3/", false),
            ("https://localhost/", false),
            ("https://a.localhost/", false),
            ("http://127.0.0.1:8080/", false),
            ("not a url", false),
        ] {
            assert!(matches!(check_url(bad, allow), Err(OutboundError::Refused(_))), "{bad} allow={allow}");
        }
        assert!(check_url("http://127.0.0.1:8080/x", true).is_ok());
        assert!(check_url("http://localhost:8080/x", true).is_ok());
    }

    /// DNS rebinding: whatever a name resolves to is checked again, address by address, where the connection is made.
    #[tokio::test]
    async fn names_resolving_to_refused_addresses_are_refused_at_connect_time() {
        let resolve = |allow_loopback: bool| {
            GuardedResolver {
                allow_loopback,
            }
            .resolve("localhost".parse().unwrap())
        };
        let refused = resolve(false).await.err().expect("localhost resolves to loopback only");
        assert!(refused.to_string().contains("not allowed"), "{refused}");
        let allowed: Vec<SocketAddr> = resolve(true).await.expect("the test mode opens loopback").collect();
        assert!(!allowed.is_empty() && allowed.iter().all(|a| a.ip().is_loopback()), "{allowed:?}");
    }

    #[tokio::test]
    async fn requests_to_refused_hosts_never_leave() {
        let none = HeaderMap::new();
        for url in [
            "https://10.0.0.1/x",
            "https://[::ffff:169.254.169.254]/latest",
            "https://[64:ff9b::a00:1]/",
            "http://example.com/",
        ] {
            let err = execute(Method::GET, url, &none, &none, &NoBody, true).await.unwrap_err();
            assert!(matches!(err, OutboundError::Refused(_)), "{url}: {err:?}");
            let err = get_once(url, "application/json", Duration::from_secs(1)).await.unwrap_err();
            assert!(matches!(err, OutboundError::Refused(_)), "{url}: {err:?}");
        }
    }

    #[test]
    fn origins_compare_scheme_host_and_port() {
        let u = |s: &str| Url::parse(s).unwrap();
        assert!(same_origin(&u("https://a.com/x"), &u("https://a.com:443/y")));
        assert!(!same_origin(&u("https://a.com/x"), &u("https://b.a.com/x")));
        assert!(!same_origin(&u("https://a.com/x"), &u("https://a.com:8443/x")));
        assert!(!same_origin(&u("http://a.com/x"), &u("https://a.com/x")));
    }

    #[test]
    fn phone_headers_are_checked() {
        let h = |pairs: &[(&str, &str)]| -> Vec<(String, String)> {
            pairs.iter().map(|(a, b)| ((*a).to_owned(), (*b).to_owned())).collect()
        };
        let map = header_map(&h(&[("Authorization", "Bearer x"), ("Content-Type", "text/plain")]), &["content-type"])
            .unwrap();
        assert_eq!(map.len(), 1);
        assert_eq!(map["authorization"], "Bearer x");
        for bad in [("Host", "evil"), ("Content-Length", "5"), ("Transfer-Encoding", "chunked"), ("bad name", "x")] {
            assert!(header_map(&h(&[bad]), &[]).is_err(), "{bad:?}");
        }
        assert!(header_map(&h(&[("X-A", "a\r\nInjected: 1")]), &[]).is_err());
        let many: Vec<(String, String)> = (0..=MAX_HEADERS).map(|i| (format!("x-{i}"), "v".to_owned())).collect();
        assert!(header_map(&many, &[]).is_err());
    }
}
