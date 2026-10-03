//! `reins login`: OAuth 2.1 with PKCE and a loopback redirect; the phone approves the pairing and pins this app's key.
//!
//! The session (`session.json`, 0600) keeps the server, the client id, the token endpoint, the access token with its
//! expiry, and the refresh token. Refresh tokens rotate; a refused refresh means the user (or the server) ended the
//! connection, so the session is dropped and the app is logged out.

use std::collections::HashMap;
use std::convert::Infallible;
use std::time::Duration;

use data_encoding::BASE64URL_NOPAD;
use http_body_util::Full;
use hyper::body::Bytes;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use tokio::net::TcpListener;
use zeroize::Zeroizing;

use super::{LinkError, check_url, error_text, random_token, read_limited, server_base};
use crate::config::{Paths, write_private};
use crate::identity::Identity;

/// How long `reins login` waits for the browser to come back (the phone has to approve in between).
const CALLBACK_WAIT: Duration = Duration::from_mins(15);
/// An access token is refreshed this long before it expires, so it does not expire in flight.
const EXPIRY_MARGIN_SECS: i64 = 60;
pub(super) const MAX_ANSWER_BYTES: usize = 64 * 1024;

/// What `session.json` holds.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Session {
    pub server: String,
    pub client_id: String,
    pub token_endpoint: String,
    pub access_token: String,
    /// Unix seconds.
    pub access_expires_at: i64,
    #[serde(default)]
    pub refresh_token: Option<String>,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("server", &self.server)
            .field("client_id", &self.client_id)
            .field("access_expires_at", &self.access_expires_at)
            .finish_non_exhaustive()
    }
}

impl Session {
    fn resource(&self) -> String {
        format!("{}/mcp", self.server)
    }
}

fn load_session(paths: &Paths) -> Result<Option<Session>, String> {
    let path = paths.session_file();
    match std::fs::read(&path) {
        Ok(bytes) => {
            let bytes = Zeroizing::new(bytes);
            serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|_| format!("{} is damaged; run `reins login` again", path.display()))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

pub(super) fn save_session(paths: &Paths, session: &Session) -> Result<(), String> {
    paths.ensure().map_err(|e| e.to_string())?;
    let json = Zeroizing::new(serde_json::to_vec_pretty(session).map_err(|e| e.to_string())?);
    write_private(&paths.session_file(), &json).map_err(|e| format!("{}: {e}", paths.session_file().display()))
}

/// Forgets the saved session. `Ok(false)` when there was none.
pub fn logout(paths: &Paths) -> Result<bool, String> {
    match std::fs::remove_file(paths.session_file()) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(format!("{}: {e}", paths.session_file().display())),
    }
}

/// The server the app is logged in to, if any.
#[must_use]
pub fn logged_in_server(paths: &Paths) -> Option<String> {
    load_session(paths).ok().flatten().map(|s| s.server)
}

/// Logs in to the Reins server at `server_url`: registers this app, opens the browser (when `open_browser`) at the
/// server's sign-in page, prints the link and the key fingerprint the phone will show, waits for the approval and saves
/// the session. Returns the server's base URL.
pub async fn login(paths: &Paths, identity: &Identity, server_url: &str, open_browser: bool) -> Result<String, String> {
    login_with_browser(paths, identity, server_url, |url| {
        if open_browser {
            open_in_browser(url);
        }
    })
    .await
}

/// [`login`], with `browser` called with the sign-in link once the app is ready for the callback (tests drive the
/// sign-in with it).
pub async fn login_with_browser(
    paths: &Paths,
    identity: &Identity,
    server_url: &str,
    browser: impl FnOnce(&str),
) -> Result<String, String> {
    let server = server_base(server_url)?;
    let http = crate::http::client(Some(Duration::from_secs(30)))?;
    let meta = discover(&http, &server).await?;
    let client_id = register(&http, &meta).await?;

    let listener = TcpListener::bind(("127.0.0.1", 0)).await.map_err(|e| format!("cannot listen on loopback: {e}"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let redirect_uri = format!("http://127.0.0.1:{port}/callback");
    let verifier = Zeroizing::new(random_token(32));
    let state = random_token(16);
    let resource = format!("{server}/mcp");
    let mut authorize = url::Url::parse(&meta.authorization_endpoint).map_err(|e| e.to_string())?;
    authorize
        .query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", &client_id)
        .append_pair("redirect_uri", &redirect_uri)
        .append_pair("code_challenge", &pkce_challenge(&verifier))
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", &state)
        .append_pair("resource", &resource)
        .append_pair("reins_client_key", &identity.public_key());

    println!("Open this link to log in to {server}:\n\n  {authorize}\n");
    println!("Your phone will show the key {}. Approve only if it matches.", identity.fingerprint());
    browser(authorize.as_str());

    let code = match wait_for_callback(listener, &state, CALLBACK_WAIT).await? {
        Callback::Code {
            code,
            iss,
        } => {
            // RFC 9207: a code from another authorization server must not be sent to this one.
            if let (Some(iss), Some(issuer)) = (iss, meta.issuer.as_deref())
                && iss.trim_end_matches('/') != issuer.trim_end_matches('/')
            {
                return Err(format!("the sign-in came back from {iss}, not {issuer}; not logged in"));
            }
            Zeroizing::new(code)
        }
        Callback::Refused {
            error,
            description,
        } => {
            return Err(match description {
                Some(d) => format!("not logged in: {d} ({error})"),
                None => format!("not logged in: {error}"),
            });
        }
    };

    let form = [
        ("grant_type", "authorization_code"),
        ("code", code.as_str()),
        ("redirect_uri", redirect_uri.as_str()),
        ("client_id", client_id.as_str()),
        ("code_verifier", verifier.as_str()),
        ("resource", resource.as_str()),
    ];
    let tokens = token_request(&http, &meta.token_endpoint, &form).await.map_err(|e| match e {
        TokenFailure::Refused(m) | TokenFailure::Failed(m) => format!("the server did not issue a session: {m}"),
    })?;
    let session = Session {
        server: server.clone(),
        client_id,
        token_endpoint: meta.token_endpoint,
        access_expires_at: crate::now_unix().saturating_add(tokens.expires_in),
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
    };
    save_session(paths, &session)?;
    log::info!("logged in to {server}");
    Ok(server)
}

fn pkce_challenge(verifier: &str) -> String {
    BASE64URL_NOPAD.encode(&Sha256::digest(verifier.as_bytes()))
}

fn open_in_browser(url: &str) {
    let mut cmd = if cfg!(target_os = "macos") {
        tokio::process::Command::new("open")
    } else if cfg!(windows) {
        // Not `cmd /C start`: cmd would split the link at `&`.
        let mut c = tokio::process::Command::new(crate::win::system32("rundll32.exe"));
        c.arg("url.dll,FileProtocolHandler");
        c
    } else {
        tokio::process::Command::new("xdg-open")
    };
    cmd.arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    if cmd.spawn().is_err() {
        println!("(Could not open a browser; open the link yourself.)");
    }
}

#[derive(Deserialize)]
pub(super) struct Metadata {
    #[serde(default)]
    issuer: Option<String>,
    authorization_endpoint: String,
    pub(super) token_endpoint: String,
    #[serde(default)]
    registration_endpoint: Option<String>,
    /// RFC 8628: where a QR code to scan with the phone is asked for (servers since the device flow).
    #[serde(default)]
    pub(super) device_authorization_endpoint: Option<String>,
}

/// The `grant_type` that redeems a device code (RFC 8628 §3.4).
pub(super) const DEVICE_CODE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";

async fn get_json<T: serde::de::DeserializeOwned>(http: &reqwest::Client, url: &str) -> Result<Option<T>, String> {
    let resp = http.get(url).send().await.map_err(|e| format!("cannot reach {url}: {}", e.without_url()))?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let status = resp.status();
    let body = read_limited(resp, MAX_ANSWER_BYTES).await?;
    if !status.is_success() {
        return Err(format!("{url}: {}", error_text(status, &body)));
    }
    serde_json::from_slice(&body).map(Some).map_err(|e| format!("{url}: unexpected answer: {e}"))
}

/// RFC 8414 metadata: at `<server>/.well-known/…`, or path-inserted for a server below a path.
pub(super) async fn discover(http: &reqwest::Client, server: &str) -> Result<Metadata, String> {
    let base = url::Url::parse(server).map_err(|e| e.to_string())?;
    let mut candidates = vec![format!("{server}/.well-known/oauth-authorization-server")];
    let path = base.path().trim_end_matches('/');
    if !path.is_empty() {
        candidates
            .push(format!("{}/.well-known/oauth-authorization-server{path}", base.origin().ascii_serialization()));
    }
    for url in candidates {
        if let Some(meta) = get_json::<Metadata>(http, &url).await? {
            for endpoint in [
                Some(&meta.authorization_endpoint),
                Some(&meta.token_endpoint),
                meta.registration_endpoint.as_ref(),
                meta.device_authorization_endpoint.as_ref(),
            ]
            .into_iter()
            .flatten()
            {
                let parsed = url::Url::parse(endpoint).map_err(|e| format!("{url}: bad endpoint {endpoint}: {e}"))?;
                check_url(&parsed).map_err(|e| format!("{url}: endpoint {endpoint} {e}"))?;
            }
            return Ok(meta);
        }
    }
    Err(format!("{server} is not a Reins server (no OAuth metadata)"))
}

fn client_name() -> String {
    let host = hostname::get().map(|h| h.to_string_lossy().into_owned()).unwrap_or_default();
    let host: String = host.chars().filter(|c| !c.is_control()).take(64).collect();
    let host = host.trim();
    format!(
        "Reins desktop app on {}",
        if host.is_empty() {
            "this computer"
        } else {
            host
        }
    )
}

/// RFC 7591 dynamic registration as a public client with a loopback redirect (any port, RFC 8252).
pub(super) async fn register(http: &reqwest::Client, meta: &Metadata) -> Result<String, String> {
    #[derive(Deserialize)]
    struct Registered {
        client_id: String,
    }
    let endpoint =
        meta.registration_endpoint.as_deref().ok_or("the server does not accept new apps (no registration)")?;
    let body = serde_json::json!({
        "client_name": client_name(),
        "redirect_uris": ["http://127.0.0.1/callback"],
        "grant_types": ["authorization_code", "refresh_token", DEVICE_CODE_GRANT],
        "response_types": ["code"],
        "token_endpoint_auth_method": "none",
    });
    let resp = http
        .post(endpoint)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("cannot register with the server: {}", e.without_url()))?;
    let status = resp.status();
    let body = read_limited(resp, MAX_ANSWER_BYTES).await?;
    if !status.is_success() {
        return Err(format!("the server refused to register this app: {}", error_text(status, &body)));
    }
    let r: Registered =
        serde_json::from_slice(&body).map_err(|e| format!("unexpected registration answer from the server: {e}"))?;
    if r.client_id.is_empty() {
        return Err("the server registered this app without a client id".to_owned());
    }
    Ok(r.client_id)
}

#[derive(Debug, PartialEq, Eq)]
enum Callback {
    Code {
        code: String,
        iss: Option<String>,
    },
    Refused {
        error: String,
        description: Option<String>,
    },
}

/// What a request to the loopback listener means: `Err` is the status and page to answer a request that is not the
/// callback of this login (a wrong state is ignored: it is not ours, and must not end the login).
fn read_callback(path: &str, query: Option<&str>, state: &str) -> Result<Callback, (u16, &'static str)> {
    if path != "/callback" {
        return Err((404, "Not found."));
    }
    let params: HashMap<String, String> =
        url::form_urlencoded::parse(query.unwrap_or_default().as_bytes()).into_owned().collect();
    if params.get("state").map(String::as_str) != Some(state) {
        return Err((400, "This link does not belong to the login running in your terminal."));
    }
    if let Some(error) = params.get("error") {
        return Ok(Callback::Refused {
            error: error.chars().filter(|c| !c.is_control()).take(100).collect(),
            description: params
                .get("error_description")
                .map(|d| d.chars().filter(|c| !c.is_control()).take(300).collect()),
        });
    }
    match params.get("code").filter(|c| !c.is_empty()) {
        Some(code) => Ok(Callback::Code {
            code: code.clone(),
            iss: params.get("iss").cloned(),
        }),
        None => Err((400, "The sign-in did not return a code.")),
    }
}

fn page(title: &str, message: &str) -> String {
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>Reins</title></head>\
         <body style=\"font-family:sans-serif;max-width:32em;margin:4em auto\"><h1>{title}</h1><p>{message}</p></body></html>"
    )
}

async fn wait_for_callback(listener: TcpListener, state: &str, wait: Duration) -> Result<Callback, String> {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Callback>(1);
    let deadline = tokio::time::sleep(wait);
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            got = rx.recv() => return got.ok_or_else(|| "the login was interrupted".to_owned()),
            () = &mut deadline => return Err("timed out waiting for the sign-in to finish; run `reins login` again".to_owned()),
            accepted = listener.accept() => {
                let Ok((stream, _)) = accepted else { continue };
                let tx = tx.clone();
                let state = state.to_owned();
                tokio::spawn(async move {
                    let service = hyper::service::service_fn(move |req: hyper::Request<hyper::body::Incoming>| {
                        let answer = match read_callback(req.uri().path(), req.uri().query(), &state) {
                            Ok(cb) => {
                                let shown = match &cb {
                                    Callback::Code { .. } => (200, page("Logged in", "The Reins desktop app is connected. You can close this tab.")),
                                    Callback::Refused { .. } => (200, page("Not logged in", "The login was not completed. You can close this tab; your terminal says why.")),
                                };
                                let _sent = tx.try_send(cb);
                                shown
                            }
                            Err((status, message)) => (status, page("Reins", message)),
                        };
                        let resp = hyper::Response::builder()
                            .status(answer.0)
                            .header("Content-Type", "text/html; charset=utf-8")
                            .header("Cache-Control", "no-store")
                            .body(Full::new(Bytes::from(answer.1)));
                        async move { Ok::<_, Infallible>(resp.unwrap_or_default()) }
                    });
                    let io = hyper_util::rt::TokioIo::new(stream);
                    let _done = hyper::server::conn::http1::Builder::new().serve_connection(io, service).await;
                });
            }
        }
    }
}

struct Tokens {
    access_token: String,
    expires_in: i64,
    refresh_token: Option<String>,
}

enum TokenFailure {
    /// The server answered with an OAuth error (a used or revoked refresh token, a bad code).
    Refused(String),
    /// Network trouble or a server error.
    Failed(String),
}

async fn token_request(http: &reqwest::Client, endpoint: &str, form: &[(&str, &str)]) -> Result<Tokens, TokenFailure> {
    #[derive(Deserialize)]
    struct Answer {
        access_token: String,
        #[serde(default)]
        token_type: Option<String>,
        #[serde(default)]
        expires_in: Option<i64>,
        #[serde(default)]
        refresh_token: Option<String>,
    }
    let resp = http
        .post(endpoint)
        .form(form)
        .send()
        .await
        .map_err(|e| TokenFailure::Failed(format!("cannot reach the server: {}", e.without_url())))?;
    let status = resp.status();
    let body = Zeroizing::new(read_limited(resp, MAX_ANSWER_BYTES).await.map_err(TokenFailure::Failed)?);
    if status.is_client_error() && status != reqwest::StatusCode::TOO_MANY_REQUESTS {
        return Err(TokenFailure::Refused(error_text(status, &body)));
    }
    if !status.is_success() {
        return Err(TokenFailure::Failed(error_text(status, &body)));
    }
    let a: Answer =
        serde_json::from_slice(&body).map_err(|_| TokenFailure::Failed("unexpected token answer".to_owned()))?;
    if a.access_token.is_empty() || a.token_type.as_deref().is_some_and(|t| !t.eq_ignore_ascii_case("bearer")) {
        return Err(TokenFailure::Failed("the server issued no bearer token".to_owned()));
    }
    Ok(Tokens {
        access_token: a.access_token,
        expires_in: a.expires_in.unwrap_or(3600).clamp(0, 365 * 86_400),
        refresh_token: a.refresh_token.filter(|r| !r.is_empty()),
    })
}

/// The session as the desktop API client uses it: the access token, refreshed when it expires or is rejected. The
/// file is read on every use, so `reins login` / `logout` take effect in a running daemon.
pub(crate) struct SessionTokens {
    paths: Paths,
    http: reqwest::Client,
    /// One refresh at a time: refresh tokens rotate, a second use of the old one would be refused.
    lock: tokio::sync::Mutex<()>,
}

/// A live access token and the server it is for.
pub(crate) struct Access {
    pub server: String,
    pub token: Zeroizing<String>,
}

fn not_logged_in() -> LinkError {
    LinkError::LoggedOut("not logged in to a Reins server; run `reins login`".to_owned())
}

impl SessionTokens {
    pub fn new(paths: &Paths, http: reqwest::Client) -> Self {
        Self {
            paths: paths.clone(),
            http,
            lock: tokio::sync::Mutex::new(()),
        }
    }

    fn load(&self) -> Result<Session, LinkError> {
        load_session(&self.paths).map_err(LinkError::LoggedOut)?.ok_or_else(not_logged_in)
    }

    /// A token that is not about to expire.
    pub async fn access(&self) -> Result<Access, LinkError> {
        let _one = self.lock.lock().await;
        let session = self.load()?;
        if crate::now_unix() < session.access_expires_at.saturating_sub(EXPIRY_MARGIN_SECS) {
            return Ok(Access {
                server: session.server,
                token: Zeroizing::new(session.access_token),
            });
        }
        self.refresh(session).await
    }

    /// After the server rejected `rejected` (401): a refreshed token, unless another task refreshed already.
    pub async fn after_rejection(&self, rejected: &str) -> Result<Access, LinkError> {
        let _one = self.lock.lock().await;
        let session = self.load()?;
        if session.access_token != rejected {
            return Ok(Access {
                server: session.server,
                token: Zeroizing::new(session.access_token),
            });
        }
        self.refresh(session).await
    }

    async fn refresh(&self, session: Session) -> Result<Access, LinkError> {
        let Some(refresh) = session.refresh_token.clone() else {
            self.drop_session(&session);
            return Err(LinkError::LoggedOut(format!(
                "the session with {} has expired; run `reins login {}` again",
                session.server, session.server
            )));
        };
        let resource = session.resource();
        let form = [
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh.as_str()),
            ("client_id", session.client_id.as_str()),
            ("resource", resource.as_str()),
        ];
        match token_request(&self.http, &session.token_endpoint, &form).await {
            Ok(tokens) => {
                let renewed = Session {
                    access_expires_at: crate::now_unix().saturating_add(tokens.expires_in),
                    access_token: tokens.access_token,
                    refresh_token: tokens.refresh_token.or(Some(refresh)),
                    ..session
                };
                save_session(&self.paths, &renewed).map_err(LinkError::Failed)?;
                log::info!("renewed the session with {}", renewed.server);
                Ok(Access {
                    server: renewed.server,
                    token: Zeroizing::new(renewed.access_token),
                })
            }
            Err(TokenFailure::Refused(why)) => {
                log::warn!("the Reins server ended the session: {why}");
                self.drop_session(&session);
                Err(LinkError::LoggedOut(format!(
                    "{} ended this app's session ({why}); run `reins login {}` again",
                    session.server, session.server
                )))
            }
            Err(TokenFailure::Failed(why)) => Err(LinkError::Failed(format!("cannot renew the session: {why}"))),
        }
    }

    /// Logs out, unless a new login replaced the session meanwhile.
    fn drop_session(&self, ended: &Session) {
        let current = load_session(&self.paths).ok().flatten();
        if current.is_some_and(|c| c.refresh_token == ended.refresh_token && c.access_token == ended.access_token) {
            let _gone = logout(&self.paths);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_matches_the_rfc_7636_example() {
        assert_eq!(
            pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn only_the_callback_with_our_state_counts() {
        assert_eq!(read_callback("/favicon.ico", None, "s"), Err((404, "Not found.")));
        assert_eq!(read_callback("/callback", Some("code=c&state=x"), "s").unwrap_err().0, 400);
        assert_eq!(read_callback("/callback", Some("code=c"), "s").unwrap_err().0, 400);
        assert_eq!(read_callback("/callback", Some("state=s"), "s").unwrap_err().0, 400);
        assert_eq!(
            read_callback("/callback", Some("code=c%2B1&state=s&iss=https%3A%2F%2Frw"), "s"),
            Ok(Callback::Code {
                code: "c+1".into(),
                iss: Some("https://rw".into())
            })
        );
        assert_eq!(
            read_callback("/callback", Some("error=access_denied&error_description=Denied+on+the+phone&state=s"), "s"),
            Ok(Callback::Refused {
                error: "access_denied".into(),
                description: Some("Denied on the phone".into())
            })
        );
    }

    #[test]
    fn the_session_file_never_shows_up_in_debug_output() {
        let s = Session {
            server: "https://rw".into(),
            client_id: "c".into(),
            token_endpoint: "https://rw/t".into(),
            access_token: "AT-secret".into(),
            access_expires_at: 1,
            refresh_token: Some("RT-secret".into()),
        };
        let shown = format!("{s:?}");
        assert!(!shown.contains("secret"), "{shown}");
    }

    #[test]
    fn logout_without_a_session_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        assert_eq!(logout(&paths), Ok(false));
        assert_eq!(logged_in_server(&paths), None);
        std::fs::create_dir_all(&paths.state_dir).unwrap();
        std::fs::write(paths.session_file(), "{").unwrap();
        assert_eq!(logged_in_server(&paths), None, "a damaged session is not a login");
        assert_eq!(logout(&paths), Ok(true));
    }

    #[test]
    fn the_app_names_itself_after_this_computer() {
        assert!(client_name().starts_with("Reins desktop app on "));
    }
}
