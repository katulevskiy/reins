//! The API proxy in a daemon in this process, against a mock upstream API and a mock phone: the key is added (and
//! the agent's own header replaced), leased and reused, refused when the phone says no, dropped after a 401, never
//! logged; bodies stream both ways; browsers and foreign hosts are kept out.

mod d2_support;

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use d2_support::{App, Mock, Step, capture_logs, logged_in, logs};
use http_body_util::{BodyExt as _, Full};
use rewarden_desktop::api_proxy::ApiConfig;
use rewarden_desktop::auth::prompt::NoPrompter;
use rewarden_desktop::config::{Config, Mode};
use rewarden_desktop::daemon::{Daemon, Options, Running};

const SECRET: &str = "sk-live-5ecret-0penai";

#[derive(Clone, Debug)]
struct Seen {
    method: String,
    path: String,
    query: Option<String>,
    authorization: Option<String>,
    agent_header: Option<String>,
    body_len: usize,
}

/// An upstream API: `/v1/echo` streams the body back, `/v1/expired` answers 401, anything else a small JSON.
struct Upstream {
    base: String,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl Upstream {
    async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://127.0.0.1:{}/v1", listener.local_addr().unwrap().port());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let shared = Arc::clone(&seen);
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    continue;
                };
                let seen = Arc::clone(&shared);
                tokio::spawn(async move {
                    let service = hyper::service::service_fn(move |req: hyper::Request<hyper::body::Incoming>| {
                        let seen = Arc::clone(&seen);
                        async move {
                            let header = |n: &str| req.headers().get(n).map(|v| v.to_str().unwrap().to_owned());
                            let mut s = Seen {
                                method: req.method().to_string(),
                                path: req.uri().path().to_owned(),
                                query: req.uri().query().map(str::to_owned),
                                authorization: header("authorization"),
                                agent_header: header("x-agent"),
                                body_len: 0,
                            };
                            let path = s.path.clone();
                            let body = req.into_body().collect().await.unwrap().to_bytes();
                            s.body_len = body.len();
                            seen.lock().unwrap().push(s);
                            let resp = match path.as_str() {
                                "/v1/echo" => hyper::Response::builder()
                                    .header("content-type", "application/octet-stream")
                                    .body(Full::new(body))
                                    .unwrap(),
                                "/v1/expired" => hyper::Response::builder()
                                    .status(401)
                                    .body(Full::new(Bytes::from_static(b"{\"error\":\"invalid_api_key\"}")))
                                    .unwrap(),
                                _ => hyper::Response::builder()
                                    .header("content-type", "application/json")
                                    .header("x-upstream", "yes")
                                    .body(Full::new(Bytes::from_static(b"{\"ok\":true}")))
                                    .unwrap(),
                            };
                            Ok::<_, Infallible>(resp)
                        }
                    });
                    let _done = hyper::server::conn::http1::Builder::new()
                        .serve_connection(hyper_util::rt::TokioIo::new(stream), service)
                        .await;
                });
            }
        });
        Self {
            base,
            seen,
        }
    }

    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}

struct Setup {
    mock: Mock,
    _app: App,
    upstream: Upstream,
    running: Running,
    http: reqwest::Client,
}

impl Setup {
    async fn start(mode: Mode, timeout_secs: u64) -> Self {
        capture_logs();
        let mock = Mock::start().await;
        let app = logged_in(&mock);
        let upstream = Upstream::start().await;
        let mut config = Config {
            listen: "127.0.0.1:0".parse().unwrap(),
            mode,
            approval_timeout_secs: timeout_secs,
            ..Config::default()
        };
        config.ssh.enabled = false;
        config.api = vec![ApiConfig {
            name: "openai".to_owned(),
            base: upstream.base.clone(),
            header: "Authorization: Bearer {secret}".to_owned(),
            secret: "vault:OpenAI/password".to_owned(),
            lease_secs: 3600,
        }];
        let daemon = Daemon::bind(
            &app.paths,
            config,
            Options {
                authorizer: None,
                prompter: Arc::new(NoPrompter),
                harden: false,
            },
        )
        .await
        .unwrap();
        Self {
            mock,
            _app: app,
            upstream,
            running: daemon.spawn(),
            http: rewarden_desktop::http::client(Some(std::time::Duration::from_secs(60))).unwrap(),
        }
    }

    fn addr(&self) -> SocketAddr {
        self.running.addr
    }

    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.addr())
    }
}

#[tokio::test]
async fn the_key_is_added_leased_and_never_logged() {
    let s = Setup::start(Mode::Rewarden, 10).await;
    let resp = s
        .http
        .get(s.url("/api/openai/models?limit=2"))
        .header("Authorization", "Bearer agent-made-this-up")
        .header("X-Agent", "kept")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.headers()["x-upstream"], "yes");
    assert_eq!(resp.text().await.unwrap(), "{\"ok\":true}");
    let seen = s.upstream.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, "GET");
    assert_eq!(seen[0].path, "/v1/models");
    assert_eq!(seen[0].query.as_deref(), Some("limit=2"));
    assert_eq!(seen[0].authorization.as_deref(), Some(&*format!("Bearer {SECRET}")));
    assert_eq!(seen[0].agent_header.as_deref(), Some("kept"));

    let calls = s.mock.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].tool, "vault_secret_release");
    assert_eq!(calls[0].arguments["secrets"], serde_json::json!(["OpenAI/password"]));
    assert_eq!(calls[0].arguments["command"], "API openai");
    assert_eq!(calls[0].arguments["lease_secs"], 3600);

    // Leased: the next requests do not ask again.
    for path in ["/api/openai/chat/completions", "/api/openai"] {
        let resp = s.http.post(s.url(path)).body("{}").send().await.unwrap();
        assert_eq!(resp.status(), 200);
    }
    assert_eq!(s.mock.calls().len(), 1);
    let seen = s.upstream.seen();
    assert_eq!(seen[1].path, "/v1/chat/completions");
    assert_eq!(seen[1].body_len, 2);
    assert_eq!(seen[2].path, "/v1/");
    assert!(seen.iter().all(|x| x.authorization.as_deref() == Some(&*format!("Bearer {SECRET}"))));
    let logged = logs();
    assert!(logged.contains("api openai: 200"), "{logged}");
    assert!(!logged.contains(SECRET) && !logged.contains("5ecret"), "{logged}");
}

#[tokio::test]
async fn bodies_stream_both_ways() {
    let s = Setup::start(Mode::Rewarden, 10).await;
    let big: Vec<u8> = (0..3_000_000u32).map(|i| u8::try_from(i % 251).unwrap()).collect();
    let chunks: Vec<Result<Bytes, std::io::Error>> =
        big.chunks(65_536).map(|c| Ok(Bytes::copy_from_slice(c))).collect();
    let resp = s
        .http
        .post(s.url("/api/openai/echo"))
        .body(reqwest::Body::wrap_stream(futures_util::stream::iter(chunks)))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.bytes().await.unwrap().as_ref(), big.as_slice());
    let resp = s.http.put(s.url("/api/openai/echo")).body(big.clone()).send().await.unwrap();
    assert_eq!(resp.bytes().await.unwrap().len(), big.len());
}

#[tokio::test]
async fn a_refusal_reaches_the_agent_and_nothing_goes_upstream() {
    let s = Setup::start(Mode::Rewarden, 10).await;
    s.mock.plan(&[Step::Denied("Not this API")]);
    let resp = s.http.get(s.url("/api/openai/models")).send().await.unwrap();
    assert_eq!(resp.status(), 403);
    assert!(resp.text().await.unwrap().contains("Not this API"));
    assert!(s.upstream.seen().is_empty());
    // A denial is not remembered: the next request asks again.
    let resp = s.http.get(s.url("/api/openai/models")).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(s.mock.calls().len(), 2);

    // A forged answer is refused the same way.
    let s = Setup::start(Mode::Rewarden, 10).await;
    s.mock.plan(&[Step::ForgedNonce]);
    let resp = s.http.get(s.url("/api/openai/models")).send().await.unwrap();
    assert_eq!(resp.status(), 503);
    assert!(resp.text().await.unwrap().contains("nonce"));
    assert!(s.upstream.seen().is_empty());
}

#[tokio::test]
async fn a_401_from_the_api_drops_the_lease() {
    let s = Setup::start(Mode::Rewarden, 10).await;
    assert_eq!(s.http.get(s.url("/api/openai/expired")).send().await.unwrap().status(), 401);
    assert_eq!(s.http.get(s.url("/api/openai/models")).send().await.unwrap().status(), 200);
    assert_eq!(s.mock.calls().len(), 2, "asked again after the 401");
}

#[tokio::test]
async fn an_unanswered_request_is_polled_again_on_retry() {
    let s = Setup::start(Mode::Rewarden, 5).await;
    s.mock.plan(&[Step::Pending]);
    let resp = s.http.get(s.url("/api/openai/models")).send().await.unwrap();
    assert_eq!(resp.status(), 503);
    assert_eq!(resp.headers()["retry-after"], "10");
    assert!(resp.text().await.unwrap().contains("Waiting"));
    // The user approves meanwhile; the retry picks the same request up.
    s.mock.with(|st| st.set_steps("req-1", &[Step::Approve]));
    let resp = s.http.get(s.url("/api/openai/models")).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(s.mock.calls().len(), 1, "no second question");
    assert!(s.mock.with(|st| st.polls.iter().filter(|p| *p == "req-1").count()) >= 2);
}

#[tokio::test]
async fn browsers_paths_out_of_the_api_and_unknown_apis_are_refused() {
    let s = Setup::start(Mode::Rewarden, 10).await;
    let origin = s.http.get(s.url("/api/openai/models")).header("Origin", "https://evil.example").send().await.unwrap();
    assert_eq!(origin.status(), 403);
    let raw =
        d2_raw(s.addr(), "GET /api/openai/models HTTP/1.1\r\nHost: evil.example\r\nConnection: close\r\n\r\n").await;
    assert!(raw.starts_with("HTTP/1.1 403"), "{raw}");
    // Raw: an HTTP client would resolve the dots itself.
    for path in ["/api/openai/v1/%2e%2e/admin", "/api/openai/../x", "/api/openai/a%2Fb"] {
        let request = format!("GET {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n", s.addr());
        let raw = d2_raw(s.addr(), &request).await;
        assert!(raw.starts_with("HTTP/1.1 400"), "{path}: {raw}");
    }
    let unknown = s.http.get(s.url("/api/other/x")).send().await.unwrap();
    assert_eq!(unknown.status(), 404);
    assert!(s.upstream.seen().is_empty());
    assert!(s.mock.calls().is_empty());
}

#[tokio::test]
async fn local_mode_says_secrets_live_on_the_phone() {
    let s = Setup::start(Mode::Local, 10).await;
    let resp = s.http.get(s.url("/api/openai/models")).send().await.unwrap();
    assert_eq!(resp.status(), 503);
    assert!(resp.text().await.unwrap().contains("local mode"));
    assert!(s.mock.calls().is_empty());
}

async fn d2_raw(addr: SocketAddr, request: &str) -> String {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
    s.write_all(request.as_bytes()).await.unwrap();
    let mut out = Vec::new();
    s.read_to_end(&mut out).await.unwrap();
    String::from_utf8_lossy(&out).into_owned()
}
