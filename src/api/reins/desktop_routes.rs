//! `{domain}/reins/desktop/calls` (desktop git proxy spec, S): the Reins desktop app asks the phone for git access
//! with the same OAuth access token an AI uses for `/mcp`. Only desktop-only tools are accepted, and only here; a call
//! is relayed exactly like an MCP tool call, and the phone's answer (a credential sealed to the app's key, which the
//! server cannot open) comes back as the relay stored it.
//!
//! Arguments are never logged: a push summary describes private code.

use std::{io::Cursor, time::Duration};

use reins_proto::{
    connector::{self, ConnectorCall, normalize_account_name},
    gmail::ToolCall,
    ids::{ConnectionId, RequestId},
};
use rocket::{
    Data, Request, Route, State,
    data::ToByteUnit,
    http::{ContentType, Status},
    response::{Responder, Response},
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{
    HUB,
    limits::{rate_limited_text, retry_secs},
    mcp_routes::{
        ACCOUNT_CALLS_LIMITED, CONNECTION_REQUESTS_LIMITED, CONNECTION_WAITING_LIMITED, McpRequest, QUEUED_TEXT,
        SubmitError, Unauthorized, authenticate, check_connection_rate, enter_waiting, submit_to_phone,
    },
    now_unix,
    relay::WaitResult,
};
use crate::db::{DbConn, DbPool, models::ReinsConnection};

/// A push summary is capped at 200 000 bytes by its tool spec; the rest of a call is small.
pub const MAX_BODY_BYTES: u64 = 512 * 1024;
/// Request ids are UUIDs; anything longer is not one of ours.
const MAX_REQUEST_ID_BYTES: usize = 64;
/// Longest tool name echoed back in an error.
const MAX_ECHOED_TOOL_CHARS: usize = 64;

pub fn routes() -> Vec<Route> {
    routes![post_call, get_call, get_phone, delete_connection]
}

// ---------------------------------------------------------------------------------------
// Responses
// ---------------------------------------------------------------------------------------

/// A JSON answer, never cached (an answered call carries a sealed credential).
#[derive(Debug, PartialEq)]
pub struct DesktopResponse {
    status: Status,
    body: Value,
    challenge: Option<String>,
    /// Seconds for `Retry-After` (rate limits).
    retry_after: Option<u64>,
}

impl DesktopResponse {
    fn json(status: Status, body: Value) -> Self {
        Self {
            status,
            body,
            challenge: None,
            retry_after: None,
        }
    }

    /// 429: `what` was limited; try again after `wait`.
    fn rate_limited(what: &str, wait: Duration) -> Self {
        Self {
            retry_after: Some(retry_secs(wait)),
            ..Self::error(Status::TooManyRequests, "rate_limited", &rate_limited_text(what, wait))
        }
    }

    fn error(status: Status, error: &str, message: &str) -> Self {
        Self::json(status, json!({"error": error, "message": message}))
    }

    fn not_found() -> Self {
        Self::json(Status::NotFound, json!({"error": "not_found"}))
    }

    fn unauthorized(refused: Unauthorized) -> Self {
        let error = if refused.invalid_token {
            "invalid_token"
        } else {
            "unauthorized"
        };
        Self {
            status: Status::Unauthorized,
            body: json!({"error": error}),
            challenge: Some(refused.challenge()),
            retry_after: None,
        }
    }
}

impl<'r> Responder<'r, 'static> for DesktopResponse {
    fn respond_to(self, _: &'r Request<'_>) -> Result<Response<'static>, Status> {
        let text = self.body.to_string();
        let mut response = Response::build();
        response
            .status(self.status)
            .header(ContentType::JSON)
            .raw_header("Cache-Control", "no-store")
            .sized_body(text.len(), Cursor::new(text));
        if let Some(challenge) = self.challenge {
            response.raw_header("WWW-Authenticate", challenge);
        }
        if let Some(seconds) = self.retry_after {
            response.raw_header("Retry-After", seconds.to_string());
        }
        response.ok()
    }
}

/// The answer to one wait on a relay request: answered (with the outcome as the relay serializes it), pending or
/// offline. `None` for an unknown or expired request, or another connection's.
pub fn wait_body(wait: &WaitResult, id: &RequestId) -> Option<Value> {
    match wait {
        WaitResult::Answered(outcome) => Some(json!({"request_id": id.0, "status": "answered", "outcome": outcome})),
        WaitResult::Pending => Some(json!({"request_id": id.0, "status": "pending"})),
        WaitResult::Offline => Some(json!({"request_id": id.0, "status": "offline"})),
        WaitResult::NotFound => None,
    }
}

fn wait_response(wait: &WaitResult, id: &RequestId) -> DesktopResponse {
    wait_body(wait, id).map_or_else(DesktopResponse::not_found, |body| DesktopResponse::json(Status::Ok, body))
}

// ---------------------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CallBody {
    tool: String,
    #[serde(default)]
    arguments: Value,
    #[serde(default)]
    account: Option<String>,
}

/// A validated desktop call, ready to relay.
#[derive(Debug, PartialEq, Eq)]
pub struct DesktopCall {
    pub tool: &'static str,
    pub call: ConnectorCall,
    pub account: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum CallError {
    /// Not JSON, or not the `{"tool", "arguments", "account"}` shape.
    InvalidRequest(String),
    /// Not a desktop-only tool (every AI tool is unknown here, as desktop tools are on `/mcp`).
    UnknownTool(String),
    /// Refused by the tool's spec.
    InvalidArguments(String),
}

impl CallError {
    fn response(&self) -> DesktopResponse {
        match self {
            Self::InvalidRequest(m) => DesktopResponse::error(Status::BadRequest, "invalid_request", m),
            Self::UnknownTool(m) => DesktopResponse::error(Status::BadRequest, "unknown_tool", m),
            Self::InvalidArguments(m) => DesktopResponse::error(Status::BadRequest, "invalid_arguments", m),
        }
    }
}

/// Validates a call body: a desktop-only tool, arguments its spec accepts, and the account normalized like an MCP
/// call's. The account may sit at the top of the body or among the arguments, but not differ between the two.
pub fn parse_call(body: &[u8]) -> Result<DesktopCall, CallError> {
    let body: CallBody = serde_json::from_slice(body).map_err(|e| {
        CallError::InvalidRequest(format!("The body must be {{\"tool\", \"arguments\", \"account\"}}: {e}"))
    })?;
    let Some(spec) = connector::spec_for_tool(&body.tool).filter(|s| s.desktop_only) else {
        let shown: String = body.tool.chars().filter(|c| !c.is_control()).take(MAX_ECHOED_TOOL_CHARS).collect();
        return Err(CallError::UnknownTool(format!("Unknown desktop tool: {shown}")));
    };
    let (call, in_arguments) = spec.parse(&body.arguments).map_err(CallError::InvalidArguments)?;
    let top = body.account.as_deref().map(normalize_account_name).transpose().map_err(CallError::InvalidArguments)?;
    let account = match (top, in_arguments) {
        (Some(a), Some(b)) if a != b => {
            return Err(CallError::InvalidArguments("`account` is given twice with different values.".to_owned()));
        }
        (a, b) => a.or(b),
    };
    Ok(DesktopCall {
        tool: spec.tool,
        call,
        account,
    })
}

// ---------------------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------------------

/// The desktop app talks to the server directly; a browser never has a reason to call this API.
fn refuse_browsers(request: &McpRequest) -> Option<DesktopResponse> {
    request.origin.is_some().then(|| {
        DesktopResponse::error(Status::Forbidden, "forbidden_origin", "Browsers may not call the desktop API.")
    })
}

async fn authenticated(request: &McpRequest, conn: &DbConn) -> Result<ReinsConnection, DesktopResponse> {
    if let Some(refused) = refuse_browsers(request) {
        return Err(refused);
    }
    let connection =
        authenticate(request.authorization.as_deref(), conn).await.map_err(DesktopResponse::unauthorized)?;
    check_connection_rate(&connection)
        .map_err(|wait| DesktopResponse::rate_limited(CONNECTION_REQUESTS_LIMITED, wait))?;
    if let Err(e) = ReinsConnection::touch(&connection.uuid, now_unix(), conn).await {
        warn!("Could not record Reins connection use: {e:?}");
    }
    Ok(connection)
}

/// Relays a desktop call to the phone and waits like an MCP tool call.
#[post("/reins/desktop/calls", data = "<data>")]
async fn post_call(data: Data<'_>, request: McpRequest, conn: DbConn, pool: &State<DbPool>) -> DesktopResponse {
    let connection = match authenticated(&request, &conn).await {
        Ok(c) => c,
        Err(response) => return response,
    };
    let body = match data.open(MAX_BODY_BYTES.bytes()).into_bytes().await {
        Ok(b) if b.is_complete() => b.into_inner(),
        Ok(_) => {
            return DesktopResponse::error(
                Status::PayloadTooLarge,
                "too_large",
                &format!("The body is larger than {MAX_BODY_BYTES} bytes."),
            );
        }
        Err(_) => return DesktopResponse::error(Status::BadRequest, "invalid_request", "Unreadable body."),
    };
    let call = match parse_call(&body) {
        Ok(c) => c,
        Err(e) => return e.response(),
    };
    debug!("Desktop call {} ({} bytes)", call.tool, body.len());
    let _waiting = match enter_waiting(&connection) {
        Ok(place) => place,
        Err(wait) => return DesktopResponse::rate_limited(CONNECTION_WAITING_LIMITED, wait),
    };
    let request_id =
        match submit_to_phone(&connection, None, ToolCall::Connector(call.call), call.account, &conn, pool.inner())
            .await
        {
            Ok(request_id) => request_id,
            Err(SubmitError::Queued) => {
                return DesktopResponse::error(Status::TooManyRequests, "rate_limited", QUEUED_TEXT);
            }
            Err(SubmitError::Busy) => {
                return DesktopResponse::error(
                    Status::ServiceUnavailable,
                    "busy",
                    "Reins is busy. Try again in a minute.",
                );
            }
            Err(SubmitError::RateLimited(wait)) => {
                return DesktopResponse::rate_limited(ACCOUNT_CALLS_LIMITED, wait);
            }
        };
    // Never hold a pooled DB connection while waiting for the phone.
    drop(conn);
    let waited = HUB.relay.wait(&request_id, &ConnectionId(connection.uuid)).await;
    wait_response(&waited, &request_id)
}

/// Waits again on an earlier call of the same connection (an approval can come after the first wait ran out).
#[get("/reins/desktop/calls/<id>")]
async fn get_call(id: &str, request: McpRequest, conn: DbConn) -> DesktopResponse {
    let connection = match authenticated(&request, &conn).await {
        Ok(c) => c,
        Err(response) => return response,
    };
    drop(conn);
    if id.is_empty() || id.len() > MAX_REQUEST_ID_BYTES {
        return DesktopResponse::not_found();
    }
    let _waiting = match enter_waiting(&connection) {
        Ok(place) => place,
        Err(wait) => return DesktopResponse::rate_limited(CONNECTION_WAITING_LIMITED, wait),
    };
    let request_id = RequestId::from(id);
    let waited = HUB.relay.wait(&request_id, &ConnectionId(connection.uuid)).await;
    wait_response(&waited, &request_id)
}

/// When the approval phone last asked this server for work (`null`: not since the server started), the integrations
/// it reported having an account for (`null`: not reported since the server started), and the server's clock: for
/// `reins doctor`, and for `reins setup`/`resume`, which leave git direct for a host the phone cannot serve yet. A
/// phone asleep in a pocket polls rarely; push wakes it, so an old time is no fault.
#[get("/reins/desktop/phone")]
async fn get_phone(request: McpRequest, conn: DbConn) -> DesktopResponse {
    let connection = match authenticated(&request, &conn).await {
        Ok(c) => c,
        Err(response) => return response,
    };
    drop(conn);
    let user = connection.user_uuid.to_string();
    let services = HUB.services_of(&user);
    let last_seen = HUB.phone_seen(&user);
    DesktopResponse::json(Status::Ok, json!({"last_seen": last_seen, "services": services, "server_time": now_unix()}))
}

/// `reins logout` and `reins uninstall`: this computer's connection ends itself, its refresh tokens with it. The phone's
/// list of connections no longer shows it, and its access token stops working at once (every call re-checks).
#[delete("/reins/desktop/connection")]
async fn delete_connection(request: McpRequest, conn: DbConn) -> DesktopResponse {
    let connection = match authenticated(&request, &conn).await {
        Ok(c) => c,
        Err(response) => return response,
    };
    match connection.delete(&conn).await {
        Ok(()) => {
            info!("Desktop connection {} removed by the desktop app", connection.uuid);
            DesktopResponse::json(Status::Ok, json!({"removed": true}))
        }
        Err(e) => {
            warn!("Could not remove desktop connection: {e:?}");
            DesktopResponse::error(Status::InternalServerError, "server_error", "The connection could not be removed.")
        }
    }
}

#[cfg(test)]
mod tests {
    use reins_proto::{
        desktop::{self, GIT_FETCH_TOOL, GIT_PUSH_TOOL},
        relay::{RelayOutcome, ToolResult},
    };

    use super::*;

    fn key() -> String {
        desktop::encode_key(&[5u8; 32])
    }

    fn fetch_body(extra: &Value) -> Vec<u8> {
        let mut body =
            json!({"tool": GIT_FETCH_TOOL, "arguments": {"repo": "octo/app", "client_key": key(), "nonce": "n-1"}});
        if let Value::Object(map) = extra {
            for (k, v) in map {
                body[k] = v.clone();
            }
        }
        body.to_string().into_bytes()
    }

    fn message(result: Result<DesktopCall, CallError>) -> CallError {
        result.expect_err("the call should be refused")
    }

    #[test]
    fn a_fetch_is_parsed_into_a_connector_call() {
        let call = parse_call(&fetch_body(&json!({}))).unwrap();
        assert_eq!(call.tool, GIT_FETCH_TOOL);
        assert_eq!((call.call.service.as_str(), call.call.op.as_str()), ("github", desktop::GIT_FETCH_OP));
        assert_eq!(call.call.args["repo"], "octo/app");
        assert_eq!(call.call.args["client_key"], key());
        assert_eq!(call.account, None);
        let with_account = parse_call(&fetch_body(&json!({"account": " Octo-Cat "}))).unwrap();
        assert_eq!(with_account.account.as_deref(), Some("octo-cat"));
        let null_account = parse_call(&fetch_body(&json!({"account": null}))).unwrap();
        assert_eq!(null_account.account, None);
    }

    #[test]
    fn a_push_carries_its_summary_and_digest() {
        let body = json!({"tool": GIT_PUSH_TOOL, "arguments": {"repo": "octo/app", "client_key": key(), "nonce": "n",
            "digest": "ab".repeat(32), "summary": {"updates": [], "pack_bytes": 0}}, "account": null});
        let call = parse_call(body.to_string().as_bytes()).unwrap();
        assert_eq!(call.call.op, desktop::GIT_PUSH_OP);
        assert_eq!(call.call.args["summary"]["pack_bytes"], 0);
    }

    #[test]
    fn only_desktop_tools_are_accepted() {
        for tool in ["github_list_repos", "gmail_search", "reins_get_result", "nope", ""] {
            let body = json!({"tool": tool, "arguments": {}});
            assert!(
                matches!(message(parse_call(body.to_string().as_bytes())), CallError::UnknownTool(_)),
                "{tool} must be unknown here"
            );
        }
        let long = json!({"tool": format!("x\u{7}{}", "y".repeat(500)), "arguments": {}});
        let CallError::UnknownTool(text) = message(parse_call(long.to_string().as_bytes())) else {
            panic!("unknown tool expected")
        };
        assert!(text.len() < 100 && !text.contains('\u{7}'), "{text}");
    }

    #[test]
    fn arguments_are_checked_by_the_tool_spec() {
        let missing = json!({"tool": GIT_FETCH_TOOL, "arguments": {"repo": "octo/app"}});
        let CallError::InvalidArguments(text) = message(parse_call(missing.to_string().as_bytes())) else {
            panic!("invalid arguments expected")
        };
        assert!(text.contains("client_key"), "{text}");
        let extra = fetch_body(&json!({"arguments": {"repo": "o/r", "client_key": key(), "nonce": "n", "token": "x"}}));
        assert!(
            matches!(message(parse_call(&extra)), CallError::InvalidArguments(m) if m.contains("Unknown property"))
        );
        let not_object = json!({"tool": GIT_FETCH_TOOL, "arguments": "repo"});
        assert!(matches!(message(parse_call(not_object.to_string().as_bytes())), CallError::InvalidArguments(_)));
        let no_arguments = json!({"tool": GIT_FETCH_TOOL});
        assert!(matches!(message(parse_call(no_arguments.to_string().as_bytes())), CallError::InvalidArguments(_)));
    }

    #[test]
    fn the_account_is_normalized_and_may_not_contradict_itself() {
        assert!(matches!(
            message(parse_call(&fetch_body(&json!({"account": "a\nb"})))),
            CallError::InvalidArguments(m) if m.contains("account")
        ));
        let both = fetch_body(&json!({"account": "Octo",
            "arguments": {"repo": "o/r", "client_key": key(), "nonce": "n", "account": "octo"}}));
        assert_eq!(parse_call(&both).unwrap().account.as_deref(), Some("octo"));
        let differ = fetch_body(&json!({"account": "octo",
            "arguments": {"repo": "o/r", "client_key": key(), "nonce": "n", "account": "other"}}));
        assert!(matches!(message(parse_call(&differ)), CallError::InvalidArguments(m) if m.contains("twice")));
        let inner_only =
            fetch_body(&json!({"arguments": {"repo": "o/r", "client_key": key(), "nonce": "n", "account": "Me"}}));
        assert_eq!(parse_call(&inner_only).unwrap().account.as_deref(), Some("me"));
    }

    #[test]
    fn malformed_bodies_are_invalid_requests() {
        for body in
            [&b"not json"[..], b"[]", b"{}", br#"{"tool": 5, "arguments": {}}"#, br#"{"tool": "x", "extra": 1}"#]
        {
            assert!(
                matches!(message(parse_call(body)), CallError::InvalidRequest(_)),
                "{}",
                String::from_utf8_lossy(body)
            );
        }
        assert!(matches!(message(parse_call(&fetch_body(&json!({"account": 5})))), CallError::InvalidRequest(_)));
    }

    #[test]
    fn errors_are_json_with_a_code_and_a_message() {
        let r = CallError::UnknownTool("Unknown desktop tool: x".into()).response();
        assert_eq!(r.status, Status::BadRequest);
        assert_eq!(r.body, json!({"error": "unknown_tool", "message": "Unknown desktop tool: x"}));
        assert_eq!(CallError::InvalidArguments("m".into()).response().body["error"], "invalid_arguments");
        assert_eq!(CallError::InvalidRequest("m".into()).response().body["error"], "invalid_request");
        assert_eq!(DesktopResponse::not_found().body, json!({"error": "not_found"}));
        let limited = DesktopResponse::rate_limited("too many requests on this connection", Duration::from_secs(3));
        assert_eq!((limited.status, limited.retry_after), (Status::TooManyRequests, Some(3)));
        assert_eq!(
            limited.body,
            json!({"error": "rate_limited",
                "message": "Rate limited: too many requests on this connection; try again in 3 s."})
        );
    }

    #[test]
    fn wait_results_have_one_shape() {
        let id = RequestId("req-1".into());
        let outcome = RelayOutcome::Result {
            result: ToolResult::Connector {
                data: json!({"sealed": "abc"}),
            },
        };
        assert_eq!(
            wait_body(&WaitResult::Answered(outcome), &id),
            Some(json!({"request_id": "req-1", "status": "answered",
                "outcome": {"outcome": "result", "result": {"kind": "connector", "data": {"sealed": "abc"}}}}))
        );
        let denied = RelayOutcome::Denied {
            reason: Some("no".into()),
        };
        assert_eq!(
            wait_body(&WaitResult::Answered(denied), &id).unwrap()["outcome"],
            json!({"outcome": "denied", "reason": "no"})
        );
        assert_eq!(wait_body(&WaitResult::Pending, &id), Some(json!({"request_id": "req-1", "status": "pending"})));
        assert_eq!(wait_body(&WaitResult::Offline, &id), Some(json!({"request_id": "req-1", "status": "offline"})));
        assert_eq!(wait_body(&WaitResult::NotFound, &id), None);
        assert_eq!(wait_response(&WaitResult::NotFound, &id).status, Status::NotFound);
    }
}
