//! `POST {domain}/mcp` (spec §4.2-4.3): bearer authentication, Origin check and the relay
//! of `tools/call` to the user's approval device. Protocol logic is in `mcp.rs`/`tools.rs`.

use std::{convert::Infallible, io::Cursor, time::Duration};

use rewarden_proto::{
    gmail::ToolCall,
    ids::{ConnectionId, RequestId},
    pairing::{PushKind, PushMessage},
    remote_mcp::McpServerReport,
};
use rocket::{
    Data, Request, Route, State,
    data::ToByteUnit,
    http::{ContentType, Status},
    request::{FromRequest, Outcome},
    response::{Responder, Response},
};
use serde_json::{Value, json};

use super::{
    HUB, fcm,
    limits::{self, Admitted, rate_limited_text, retry_secs},
    mcp::{self, Action, INVALID_PARAMS, INVALID_REQUEST, McpHeaders, RATE_LIMITED},
    now_unix,
    oauth::canonical_mcp_url,
    relay::QueueFull,
    tools::{self, ToolArgError, ToolInvocation},
};
use crate::{
    CONFIG,
    auth::rewarden::decode_access_token,
    db::{
        DbConn, DbPool,
        models::{RewardenConnection, RewardenDevice, User, UserId},
    },
};

const MAX_BODY_BYTES: u64 = 1024 * 1024;
/// AI clients that may send an `Origin` header (server-to-server calls send none).
const ALLOWED_ORIGINS: [&str; 4] =
    ["https://claude.ai", "https://claude.com", "https://chatgpt.com", "https://chat.openai.com"];

pub fn routes() -> Vec<Route> {
    routes![mcp_post, mcp_get, mcp_delete]
}

// ---------------------------------------------------------------------------------------
// Request/response plumbing
// ---------------------------------------------------------------------------------------

/// The headers `/mcp` cares about; extraction never fails.
pub struct McpRequest {
    pub authorization: Option<String>,
    pub origin: Option<String>,
    pub headers: McpHeaders,
    /// `X-Rewarden-Via`, sanitized: which app on the connection's machine is asking ("Claude Code").
    pub via: Option<String>,
}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for McpRequest {
    type Error = Infallible;

    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        let get = |name: &str| request.headers().get_one(name).map(str::to_owned);
        Outcome::Success(Self {
            authorization: get("Authorization"),
            origin: get("Origin"),
            headers: McpHeaders {
                protocol_version: get("MCP-Protocol-Version"),
                method: get("Mcp-Method"),
                name: get("Mcp-Name"),
            },
            via: request.headers().get_one(VIA_HEADER).and_then(sanitize_via),
        })
    }
}

/// Header naming the app behind a shared connection (the desktop app's `rewarden mcp --via`).
pub const VIA_HEADER: &str = "X-Rewarden-Via";
/// Longest `X-Rewarden-Via` kept.
pub const MAX_VIA_CHARS: usize = 40;

/// The `X-Rewarden-Via` value as shown on the phone: one line, single spaces, at most [`MAX_VIA_CHARS`] characters;
/// `None` when nothing is left. It is untrusted (any client can send it) and only ever shown next to the label.
pub fn sanitize_via(raw: &str) -> Option<String> {
    let words: Vec<&str> = raw.split(|c: char| c.is_whitespace() || c.is_control()).filter(|w| !w.is_empty()).collect();
    let via: String = words.join(" ").chars().filter(|c| !c.is_control()).take(MAX_VIA_CHARS).collect();
    let via = via.trim_end().to_owned();
    (!via.is_empty()).then_some(via)
}

/// The connection label the phone shows for a request: "Laptop · Claude Code" when the client said who it is.
pub fn label_with_via(label: &str, via: Option<&str>) -> String {
    match via {
        Some(via) => format!("{label} \u{b7} {via}"),
        None => label.to_owned(),
    }
}

/// A JSON-RPC response (or an empty one) with optional extra headers.
pub struct McpResponse {
    status: Status,
    body: Option<Value>,
    headers: Vec<(&'static str, String)>,
}

impl McpResponse {
    fn json(status: u16, body: Option<Value>) -> Self {
        Self {
            status: Status::from_code(status).unwrap_or(Status::BadRequest),
            body,
            headers: Vec::new(),
        }
    }

    fn with_header(mut self, name: &'static str, value: String) -> Self {
        self.headers.push((name, value));
        self
    }
}

impl<'r> Responder<'r, 'static> for McpResponse {
    fn respond_to(self, _: &'r Request<'_>) -> Result<Response<'static>, Status> {
        let mut response = Response::build();
        response.status(self.status);
        for (name, value) in self.headers {
            response.raw_header(name, value);
        }
        if let Some(body) = self.body {
            let text = body.to_string();
            response.header(ContentType::JSON).sized_body(text.len(), Cursor::new(text));
        }
        response.ok()
    }
}

// ---------------------------------------------------------------------------------------
// Pure helpers
// ---------------------------------------------------------------------------------------

/// Decision 12: no `Origin` (server-to-server) or one of the known AI clients / this server.
pub fn origin_allowed(origin: Option<&str>, own_origin: &str) -> bool {
    match origin {
        None => true,
        Some(o) => {
            let o = o.trim().trim_end_matches('/');
            ALLOWED_ORIGINS.iter().any(|a| a.eq_ignore_ascii_case(o))
                || o.eq_ignore_ascii_case(own_origin.trim_end_matches('/'))
        }
    }
}

/// The token of an `Authorization: Bearer …` header.
pub fn bearer_token(authorization: Option<&str>) -> Option<&str> {
    let value = authorization?.trim();
    let (scheme, token) = value.split_once(' ')?;
    (scheme.eq_ignore_ascii_case("bearer") && !token.trim().is_empty()).then(|| token.trim())
}

/// RFC 9728 §3.1 path-inserted metadata URL for `{origin}{domain_path}/mcp`.
pub fn resource_metadata_url(origin: &str, domain_path: &str) -> String {
    format!(
        "{}/.well-known/oauth-protected-resource{}/mcp",
        origin.trim_end_matches('/'),
        domain_path.trim_end_matches('/')
    )
}

/// `WWW-Authenticate` value for a 401 (RFC 6750 + RFC 9728).
pub fn bearer_challenge(metadata_url: &str, invalid_token: bool) -> String {
    if invalid_token {
        format!("Bearer error=\"invalid_token\", resource_metadata=\"{metadata_url}\"")
    } else {
        format!("Bearer resource_metadata=\"{metadata_url}\"")
    }
}

/// A refused bearer token. Each API renders its own body; the `WWW-Authenticate` challenge is the same everywhere.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Unauthorized {
    /// A token was sent but is not (or no longer) valid, as opposed to no token at all.
    pub invalid_token: bool,
}

impl Unauthorized {
    pub fn challenge(self) -> String {
        bearer_challenge(&resource_metadata_url(&CONFIG.domain_origin(), &CONFIG.domain_path()), self.invalid_token)
    }
}

fn unauthorized(refused: Unauthorized) -> McpResponse {
    McpResponse::json(401, Some(mcp::rpc_error(&Value::Null, -32001, "Authentication required", None)))
        .with_header("WWW-Authenticate", refused.challenge())
}

fn mcp_url() -> String {
    canonical_mcp_url(&CONFIG.domain())
}

// ---------------------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------------------

/// Validates the bearer token and that its connection (and user) still exist. Also guards the desktop API, which uses
/// the same access tokens.
pub async fn authenticate(authorization: Option<&str>, conn: &DbConn) -> Result<RewardenConnection, Unauthorized> {
    let Some(token) = bearer_token(authorization) else {
        return Err(Unauthorized {
            invalid_token: false,
        });
    };
    let invalid = Unauthorized {
        invalid_token: true,
    };
    let Ok(claims) = decode_access_token(token, &mcp_url()) else {
        return Err(invalid);
    };
    let user_uuid = UserId::from(claims.sub);
    let user_ok = User::find_by_uuid(&user_uuid, conn).await.is_some_and(|u| u.enabled);
    let connection = if user_ok {
        RewardenConnection::find_by_uuid_and_user(&claims.cid, &user_uuid, conn).await
    } else {
        None
    };
    connection.ok_or(invalid)
}

/// Why a call was not queued for the phone.
#[derive(Debug, PartialEq, Eq)]
pub enum SubmitError {
    /// The relay holds as many requests as it may.
    Busy,
    /// This account has as many unanswered requests queued for its phone as it may.
    Queued,
    /// The account's phone got too many calls lately; try again after this long.
    RateLimited(Duration),
}

/// What a caller is told when the account's phone has too many unanswered requests.
pub const QUEUED_TEXT: &str = "Rate limited: too many requests are waiting for the user's phone. Ask the user to open \
the Rewarden app and answer them, then try again.";
/// What a caller over the account's call rate is told.
pub const ACCOUNT_CALLS_LIMITED: &str = "too many calls to this account's phone";
/// What a caller over the connection's request rate is told.
pub const CONNECTION_REQUESTS_LIMITED: &str = "too many requests on this connection";
/// What a caller with too many calls waiting for the phone is told.
pub const CONNECTION_WAITING_LIMITED: &str = "too many calls of this connection are waiting for the phone";

/// Counts one request of `connection` against its rate (`/mcp` and the desktop API share it).
pub fn check_connection_rate(connection: &RewardenConnection) -> Result<(), Duration> {
    limits::CONNECTION_REQUESTS.check(&connection.uuid)
}

/// A place among the calls of `connection` waiting for the phone; `Err` with how long the oldest may still wait.
pub fn enter_waiting(connection: &RewardenConnection) -> Result<Admitted<String>, Duration> {
    limits::CONNECTION_WAITING
        .try_enter(&connection.uuid)
        .ok_or_else(|| Duration::from_secs(CONFIG.rewarden_relay_wait_secs()))
}

/// Queues a validated call for the user's approval device and wakes it with a push. Shared with the desktop API so
/// both relay exactly the same way, under the same per-account rate. `via`: the sanitized `X-Rewarden-Via`, appended
/// to the label.
pub async fn submit_to_phone(
    connection: &RewardenConnection,
    via: Option<&str>,
    call: ToolCall,
    account: Option<String>,
    conn: &DbConn,
    pool: &DbPool,
) -> Result<RequestId, SubmitError> {
    let user = connection.user_uuid.to_string();
    limits::ACCOUNT_CALLS.check(&user).map_err(SubmitError::RateLimited)?;
    let request = HUB
        .relay
        .submit(
            &user,
            &ConnectionId(connection.uuid.clone()),
            &label_with_via(&connection.label, via),
            call,
            account,
            now_unix(),
        )
        .map_err(|full| match full {
            QueueFull::Server => SubmitError::Busy,
            QueueFull::Account => SubmitError::Queued,
        })?;
    if let Some(device) = RewardenDevice::find_by_user(&connection.user_uuid, conn).await {
        fcm::spawn_push(
            pool.clone(),
            connection.user_uuid.clone(),
            device.fcm_token,
            PushMessage {
                t: PushKind::Req,
                id: request.id.0.clone(),
            },
        );
    }
    Ok(request.id)
}

/// One `tools/call` as the client sent it.
struct ToolRun {
    id: Value,
    name: String,
    arguments: Value,
    modern: bool,
    via: Option<String>,
}

async fn run_tool(
    run: ToolRun,
    mcp_servers: &[McpServerReport],
    connection: RewardenConnection,
    conn: DbConn,
    pool: &State<DbPool>,
) -> McpResponse {
    let ToolRun {
        id,
        name,
        arguments,
        modern,
        via,
    } = run;
    let ok = |result: Value| McpResponse::json(200, Some(mcp::tool_reply(&id, modern, result)));
    let invocation = match tools::parse_invocation_with(&name, &arguments, mcp_servers) {
        Ok(i) => i,
        Err(ToolArgError::UnknownTool(tool)) => {
            return McpResponse::json(
                400,
                Some(mcp::rpc_error(&id, INVALID_PARAMS, &format!("Unknown tool: {tool}"), None)),
            );
        }
        Err(ToolArgError::Invalid(message)) => return ok(tools::failure(&message)),
    };
    let connection_id = ConnectionId(connection.uuid.clone());
    let _waiting = match enter_waiting(&connection) {
        Ok(place) => place,
        Err(wait) => return ok(tools::failure(&rate_limited_text(CONNECTION_WAITING_LIMITED, wait))),
    };
    let request_id = match invocation {
        ToolInvocation::GetResult(request_id) => request_id,
        ToolInvocation::Relay(call, account) => {
            match submit_to_phone(&connection, via.as_deref(), call, account, &conn, pool.inner()).await {
                Ok(request_id) => request_id,
                Err(SubmitError::Busy) => {
                    return ok(tools::failure("Rewarden is busy right now. Try again in a minute."));
                }
                Err(SubmitError::Queued) => return ok(tools::failure(QUEUED_TEXT)),
                Err(SubmitError::RateLimited(wait)) => {
                    return ok(tools::failure(&rate_limited_text(ACCOUNT_CALLS_LIMITED, wait)));
                }
            }
        }
    };
    // Never hold a pooled DB connection while waiting for the phone.
    drop(conn);
    let waited = HUB.relay.wait(&request_id, &connection_id).await;
    ok(tools::render_wait(&waited, &request_id))
}

#[post("/mcp", data = "<data>")]
async fn mcp_post(data: Data<'_>, request: McpRequest, conn: DbConn, pool: &State<DbPool>) -> McpResponse {
    if !origin_allowed(request.origin.as_deref(), &CONFIG.domain_origin()) {
        return McpResponse::json(403, Some(mcp::rpc_error(&Value::Null, INVALID_REQUEST, "Origin not allowed", None)));
    }
    let connection = match authenticate(request.authorization.as_deref(), &conn).await {
        Ok(c) => c,
        Err(refused) => return unauthorized(refused),
    };
    let body = match data.open(MAX_BODY_BYTES.bytes()).into_bytes().await {
        Ok(b) if b.is_complete() => b.into_inner(),
        _ => {
            return McpResponse::json(
                400,
                Some(mcp::rpc_error(&Value::Null, INVALID_REQUEST, "Request body too large", None)),
            );
        }
    };
    let user = connection.user_uuid.to_string();
    let services = HUB.services_of(&user);
    let mcp_servers = HUB.mcp_servers_of(&user);
    let action = mcp::handle(&request.headers, &body, services.as_deref(), &mcp_servers);
    // Notifications are answered with an empty 202 and cost nothing; every other request counts.
    let notification = matches!(
        action,
        Action::Reply {
            body: None,
            ..
        }
    );
    if !notification && let Err(wait) = check_connection_rate(&connection) {
        return rate_limited_reply(action, wait);
    }
    if let Err(e) = RewardenConnection::touch(&connection.uuid, now_unix(), &conn).await {
        warn!("Could not record Rewarden connection use: {e:?}");
    }
    match action {
        Action::Reply {
            status,
            body,
        } => McpResponse::json(status, body),
        Action::CallTool {
            id,
            name,
            arguments,
            modern,
        } => {
            let run = ToolRun {
                id,
                name,
                arguments,
                modern,
                via: request.via,
            };
            run_tool(run, &mcp_servers, connection, conn, pool).await
        }
    }
}

/// The answer to a request over the connection's rate: a tool error the model can read for `tools/call`, a JSON-RPC
/// error with HTTP 429 for anything else; both say when to try again (also in `Retry-After`).
pub fn rate_limited_reply(action: Action, wait: Duration) -> McpResponse {
    let text = rate_limited_text(CONNECTION_REQUESTS_LIMITED, wait);
    let response = match action {
        Action::CallTool {
            id,
            modern,
            ..
        } => McpResponse::json(200, Some(mcp::tool_reply(&id, modern, tools::failure(&text)))),
        Action::Reply {
            body,
            ..
        } => {
            let id = body.as_ref().and_then(|b| b.get("id")).cloned().unwrap_or(Value::Null);
            McpResponse::json(429, Some(mcp::rpc_error(&id, RATE_LIMITED, &text, None)))
        }
    };
    response.with_header("Retry-After", retry_secs(wait).to_string())
}

fn method_not_allowed() -> McpResponse {
    McpResponse::json(405, Some(json!({"error": "method_not_allowed"}))).with_header("Allow", "POST".to_owned())
}

/// Streamable HTTP without a server-initiated stream: GET is not supported.
#[get("/mcp")]
fn mcp_get() -> McpResponse {
    method_not_allowed()
}

/// There are no sessions to end.
#[delete("/mcp")]
fn mcp_delete() -> McpResponse {
    method_not_allowed()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_limited_tool_calls_are_tool_errors_and_other_requests_429() {
        let wait = Duration::from_millis(7300);
        let call = Action::CallTool {
            id: json!(4),
            name: "gmail_search".to_owned(),
            arguments: json!({}),
            modern: false,
        };
        let r = rate_limited_reply(call, wait);
        assert_eq!(r.status, Status::Ok);
        let body = r.body.unwrap();
        assert_eq!((&body["id"], &body["result"]["isError"]), (&json!(4), &json!(true)));
        assert_eq!(
            body["result"]["content"][0]["text"],
            "Rate limited: too many requests on this connection; try again in 8 s."
        );
        assert_eq!(r.headers, [("Retry-After", "8".to_owned())]);
        let list = Action::Reply {
            status: 200,
            body: Some(json!({"jsonrpc": "2.0", "id": 9, "result": {}})),
        };
        let r = rate_limited_reply(list, wait);
        assert_eq!(r.status, Status::TooManyRequests);
        let body = r.body.unwrap();
        assert_eq!((&body["id"], body["error"]["code"].as_i64()), (&json!(9), Some(RATE_LIMITED)));
        assert!(body["error"]["message"].as_str().unwrap().contains("try again in 8 s"));
    }

    #[test]
    fn origins() {
        let own = "https://rw.example.com";
        assert!(origin_allowed(None, own));
        for ok in [
            "https://claude.ai",
            "https://chatgpt.com",
            "https://chat.openai.com",
            "https://claude.com",
            "https://CLAUDE.ai/",
            own,
            "https://rw.example.com/",
        ] {
            assert!(origin_allowed(Some(ok), own), "{ok}");
        }
        for bad in ["https://evil.example", "http://claude.ai", "https://claude.ai.evil.example", "null", ""] {
            assert!(!origin_allowed(Some(bad), own), "{bad}");
        }
    }

    #[test]
    fn via_is_one_short_line_appended_to_the_label() {
        assert_eq!(sanitize_via("Claude Code").as_deref(), Some("Claude Code"));
        assert_eq!(sanitize_via("  Claude \t  Code\r\n").as_deref(), Some("Claude Code"));
        assert_eq!(sanitize_via("a\u{0}b\u{7}c").as_deref(), Some("a b c"));
        assert_eq!(sanitize_via(&"x".repeat(100)).map(|v| v.chars().count()), Some(MAX_VIA_CHARS));
        assert_eq!(sanitize_via(" \t\n"), None);
        assert_eq!(sanitize_via(""), None);
        assert_eq!(label_with_via("Laptop", Some("Claude Code")), "Laptop \u{b7} Claude Code");
        assert_eq!(label_with_via("Laptop", None), "Laptop");
    }

    #[test]
    fn bearer_tokens() {
        assert_eq!(bearer_token(Some("Bearer abc.def")), Some("abc.def"));
        assert_eq!(bearer_token(Some("bearer  abc ")), Some("abc"));
        assert_eq!(bearer_token(Some("BEARER x")), Some("x"));
        for bad in [None, Some(""), Some("Bearer"), Some("Bearer "), Some("Basic abc"), Some("abc")] {
            assert_eq!(bearer_token(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn challenge_points_at_the_path_inserted_metadata() {
        let url = resource_metadata_url("https://rw.example.com", "");
        assert_eq!(url, "https://rw.example.com/.well-known/oauth-protected-resource/mcp");
        assert_eq!(
            resource_metadata_url("https://rw.example.com/", "/vw"),
            "https://rw.example.com/.well-known/oauth-protected-resource/vw/mcp"
        );
        assert_eq!(bearer_challenge(&url, false), format!("Bearer resource_metadata=\"{url}\""));
        assert!(bearer_challenge(&url, true).starts_with("Bearer error=\"invalid_token\""));
    }
}
