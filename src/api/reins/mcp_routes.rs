//! `POST {domain}/mcp` (spec §4.2-4.3): bearer authentication, Origin check and the relay
//! of `tools/call` to the user's approval device. Protocol logic is in `mcp.rs`/`tools.rs`.

use std::{convert::Infallible, io::Cursor, time::Duration};

use reins_proto::ticket::{TicketState, request_of, ticket_of};
use reins_proto::{
    batch::BatchRef,
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
    response::{
        Responder, Response,
        stream::{Event, EventStream},
    },
};
use serde_json::{Value, json};

use super::{
    HUB,
    limits::{self, Admitted, rate_limited_text, retry_secs},
    mcp::{self, Action, INVALID_PARAMS, INVALID_REQUEST, McpHeaders, RATE_LIMITED},
    now_unix,
    oauth::canonical_mcp_url,
    presence::PhoneSeen,
    push,
    relay::{QueueFull, WaitResult},
    tools::{self, ToolArgError, ToolInvocation},
};
use crate::{
    CONFIG,
    auth::reins::decode_access_token,
    db::{
        DbConn, DbPool,
        models::{ReinsConnection, ReinsDevice, ReinsTicket, User, UserId},
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
    /// `X-Reins-Via`, sanitized: which app on the connection's machine is asking ("Claude Code").
    pub via: Option<String>,
    /// `X-Reins-Wait`: how long the caller keeps asking for the answer (the desktop app polls past one relay wait);
    /// the phone counts that down. Capped by the relay to the request's lifetime.
    pub waits: Option<Duration>,
    /// `X-Reins-Defer: 1`: the caller asks not to wait (it has other work): the call becomes a ticket at once.
    pub defer: bool,
    /// `Accept`, for `GET /mcp` (an event stream).
    pub accept: Option<String>,
}

/// The header a caller states its own deadline in, in seconds (see [`McpRequest::waits`]).
pub const WAIT_HEADER: &str = "X-Reins-Wait";
/// The header a caller asks for a ticket at once with (see [`McpRequest::defer`]).
pub const DEFER_HEADER: &str = "X-Reins-Defer";

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
            waits: request
                .headers()
                .get_one(WAIT_HEADER)
                .and_then(|v| v.trim().parse::<u64>().ok())
                .filter(|&s| s > 0)
                .map(Duration::from_secs),
            defer: request.headers().get_one(DEFER_HEADER).is_some_and(|v| matches!(v.trim(), "1" | "true")),
            accept: get("Accept"),
        })
    }
}

/// Header naming the app behind a shared connection (the desktop app's `reins mcp --via`).
pub const VIA_HEADER: &str = "X-Reins-Via";
/// Longest `X-Reins-Via` kept.
pub const MAX_VIA_CHARS: usize = 40;

/// The `X-Reins-Via` value as shown on the phone: one line, single spaces, at most [`MAX_VIA_CHARS`] characters;
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
pub async fn authenticate(authorization: Option<&str>, conn: &DbConn) -> Result<ReinsConnection, Unauthorized> {
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
        ReinsConnection::find_by_uuid_and_user(&claims.cid, &user_uuid, conn).await
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
the Reins app and answer them, then try again.";
/// What a caller over the account's call rate is told.
pub const ACCOUNT_CALLS_LIMITED: &str = "too many calls to this account's phone";
/// What a caller over the connection's request rate is told.
pub const CONNECTION_REQUESTS_LIMITED: &str = "too many requests on this connection";
/// What a caller with too many calls waiting for the phone is told.
pub const CONNECTION_WAITING_LIMITED: &str = "too many calls of this connection are waiting for the phone";

/// Counts one request of `connection` against its rate (`/mcp` and the desktop API share it).
pub fn check_connection_rate(connection: &ReinsConnection) -> Result<(), Duration> {
    limits::CONNECTION_REQUESTS.check(&connection.uuid)
}

/// A place among the calls of `connection` waiting for the phone; `Err` with how long the oldest may still wait.
pub fn enter_waiting(connection: &ReinsConnection) -> Result<Admitted<String>, Duration> {
    limits::CONNECTION_WAITING
        .try_enter(&connection.uuid)
        .ok_or_else(|| Duration::from_secs(CONFIG.reins_relay_wait_secs()))
}

/// Queues a validated call for the user's approval device and wakes it with a push. Shared with the desktop API so
/// both relay exactly the same way, under the same per-account rate. `via`: the sanitized `X-Reins-Via`, appended
/// to the label.
pub async fn submit_to_phone(
    connection: &ReinsConnection,
    via: Option<&str>,
    call: ToolCall,
    account: Option<String>,
    conn: &DbConn,
    pool: &DbPool,
) -> Result<RequestId, SubmitError> {
    submit_to_phone_waiting(connection, via, call, account, conn, pool, None).await
}

/// Like [`submit_to_phone`] for a caller that keeps asking for up to `waits` (the desktop app); the phone shows that
/// deadline. When nothing can wake the phone (no push, its app not open) or the push fails, the first wait says so
/// at once instead of after the offline threshold.
pub async fn submit_to_phone_waiting(
    connection: &ReinsConnection,
    via: Option<&str>,
    call: ToolCall,
    account: Option<String>,
    conn: &DbConn,
    pool: &DbPool,
    waits: Option<Duration>,
) -> Result<RequestId, SubmitError> {
    submit_to_phone_in(connection, via, call, account, conn, pool, Submit::Waiting(waits)).await
}

/// How a call is queued: alone (waited on for up to the given time) or as one call of a `reins_batch`.
enum Submit {
    Waiting(Option<Duration>),
    Batch(BatchRef),
}

async fn submit_to_phone_in(
    connection: &ReinsConnection,
    via: Option<&str>,
    call: ToolCall,
    account: Option<String>,
    conn: &DbConn,
    pool: &DbPool,
    how: Submit,
) -> Result<RequestId, SubmitError> {
    let user = connection.user_uuid.to_string();
    limits::ACCOUNT_CALLS.check(&user).map_err(SubmitError::RateLimited)?;
    let (connection_id, label) = (ConnectionId(connection.uuid.clone()), label_with_via(&connection.label, via));
    let request = match how {
        Submit::Waiting(waits) => {
            HUB.relay.submit_waiting(&user, &connection_id, &label, call, account, now_unix(), waits)
        }
        Submit::Batch(batch) => {
            HUB.relay.submit_in_batch(&user, &connection_id, &label, call, account, now_unix(), batch)
        }
    }
    .map_err(|full| match full {
        QueueFull::Server => SubmitError::Busy,
        QueueFull::Account => SubmitError::Queued,
    })?;
    let pushing = match ReinsDevice::find_by_user(&connection.user_uuid, conn).await {
        Some(device) => {
            let (id, user) = (request.id.clone(), user.clone());
            push::spawn_push_then(
                pool.clone(),
                connection.user_uuid.clone(),
                device.fcm_token,
                PushMessage {
                    t: PushKind::Req,
                    id: request.id.0.clone(),
                    p: None,
                },
                move || {
                    if !HUB.presence.status(&user, now_unix()).recent() {
                        HUB.relay.mark_unreachable(&id);
                    }
                },
            )
        }
        None => false,
    };
    if !pushing && !HUB.presence.status(&user, now_unix()).recent() {
        HUB.relay.mark_unreachable(&request.id);
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
    /// The client asked for a ticket at once (`_meta` or `X-Reins-Defer`).
    defer: bool,
}

/// What a caller over the open-ticket cap is told.
pub const TOO_MANY_TICKETS: &str = "Too many of your requests are waiting for review. Wait for some to be decided \
(reins_tickets), then try again.";

async fn run_tool(
    run: ToolRun,
    mcp_servers: &[McpServerReport],
    connection: ReinsConnection,
    conn: DbConn,
    pool: &State<DbPool>,
) -> McpResponse {
    let ToolRun {
        id,
        name,
        arguments,
        modern,
        via,
        defer,
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
    let (request_id, first, summary) = match invocation {
        ToolInvocation::GetResult(request_id) => (request_id, false, None),
        ToolInvocation::Wait(request_id, secs) => {
            drop(conn);
            return ok(wait_ticket(&request_id, &connection, Duration::from_secs(secs), pool).await);
        }
        ToolInvocation::Tickets => return ok(list_tickets(&connection, &conn).await),
        ToolInvocation::Cancel(request_id) => return ok(cancel(&request_id, &connection, &conn, pool.inner()).await),
        ToolInvocation::Reply(request_id, text) => {
            return ok(reply(&request_id, &text, &connection, &conn, pool.inner()).await);
        }
        ToolInvocation::Batch(calls) => {
            return ok(run_batch(calls, &connection, via.as_deref(), conn, pool.inner()).await);
        }
        ToolInvocation::Relay(call, account) => {
            let summary = super::tickets::summary_of(&call);
            if let Some(open) = super::tickets::open_for(&connection.uuid, account.as_deref(), &call, &conn).await {
                return ok(tools::success(&super::tickets::pending_review(&open, Some(&summary))));
            }
            match submit_to_phone(&connection, via.as_deref(), call, account, &conn, pool.inner()).await {
                Ok(request_id) => (request_id, true, Some(summary)),
                Err(e) => return ok(submit_failure(&e)),
            }
        }
    };
    let user = connection.user_uuid.to_string();
    if defer
        && first
        && let Some(relayed) = HUB.relay.unanswered(&user, &request_id)
    {
        match super::tickets::park(&user, &relayed, TicketState::Deferred, None, &conn, pool.inner()).await {
            Ok(row) => return ok(tools::success(&super::tickets::pending_review(&row, summary.as_deref()))),
            Err(super::tickets::ParkError::TooMany) => return ok(tools::failure(TOO_MANY_TICKETS)),
            // No key to seal it to: it waits as an Instant request.
            Err(_) => {}
        }
    }
    // Never hold a pooled DB connection while waiting for the phone.
    drop(conn);
    let waited = if first {
        HUB.relay.wait_first(&request_id, &connection_id).await
    } else {
        HUB.relay.wait(&request_id, &connection_id).await
    };
    match waited {
        WaitResult::Deferred(ref ticket) => {
            let Ok(conn) = pool.get().await else {
                return ok(tools::render_wait(&waited, &request_id, PhoneSeen::Never));
            };
            // A duplicate shares the ticket of the first call.
            let parked = request_of(ticket).unwrap_or_else(|| request_id.clone());
            match ReinsTicket::find_for_connection(&parked.0, &connection.uuid, &conn).await {
                Some(row) => ok(tools::success(&super::tickets::pending_review(&row, summary.as_deref()))),
                None => ok(tools::render_wait(&waited, &request_id, PhoneSeen::Never)),
            }
        }
        // Not in the relay: a ticket by now (or never one of this connection's).
        WaitResult::NotFound if !first => {
            ok(wait_ticket(&request_id, &connection, Duration::from_secs(CONFIG.reins_relay_wait_secs()), pool).await)
        }
        _ => {
            let seen = HUB.presence.status(&user, now_unix());
            ok(tools::render_wait(&waited, &request_id, seen))
        }
    }
}

/// What the AI is told when a call could not be queued.
fn submit_failure(e: &SubmitError) -> Value {
    match e {
        SubmitError::Busy => tools::failure("Reins is busy right now. Try again in a minute."),
        SubmitError::Queued => tools::failure(QUEUED_TEXT),
        SubmitError::RateLimited(wait) => tools::failure(&rate_limited_text(ACCOUNT_CALLS_LIMITED, *wait)),
    }
}

/// `reins_batch`: queues every valid call under one batch id (the phone shows them as one queue, each approved or
/// denied on its own and audited on its own), waits for all of them together, and answers with one result per
/// call in order. A refused call fails alone.
async fn run_batch(
    calls: Vec<tools::BatchCall>,
    connection: &ReinsConnection,
    via: Option<&str>,
    conn: DbConn,
    pool: &DbPool,
) -> Value {
    let batch_id = crate::util::get_uuid();
    let size = u32::try_from(calls.len()).unwrap_or(u32::MAX);
    let mut queued = Vec::with_capacity(calls.len());
    for (index, item) in (0u32..).zip(calls) {
        let outcome = match item.call {
            Err(why) => Err(tools::failure(&why)),
            Ok((call, account)) => {
                let batch = BatchRef {
                    id: batch_id.clone(),
                    index,
                    size,
                };
                submit_to_phone_in(connection, via, call, account, &conn, pool, Submit::Batch(batch))
                    .await
                    .map_err(|e| submit_failure(&e))
            }
        };
        queued.push((item.name, outcome));
    }
    // Never hold a pooled DB connection while waiting for the phone.
    drop(conn);
    let connection_id = ConnectionId(connection.uuid.clone());
    let user = connection.user_uuid.to_string();
    let results =
        futures::future::join_all(queued.iter().map(|(_, outcome)| batch_result(outcome, &connection_id, &user))).await;
    let items = queued.into_iter().zip(results).map(|((name, outcome), result)| (name, outcome.ok(), result)).collect();
    tools::render_batch(&batch_id, items)
}

/// One call of a batch: its answer once the phone has decided (or how it still waits), or why it was not queued.
async fn batch_result(outcome: &Result<RequestId, Value>, connection_id: &ConnectionId, user: &str) -> Value {
    match outcome {
        Ok(request_id) => {
            let waited = HUB.relay.wait_first(request_id, connection_id).await;
            tools::render_wait(&waited, request_id, HUB.presence.status(user, now_unix()))
        }
        Err(failure) => failure.clone(),
    }
}

/// `reins_get_result` / `reins_wait` on a ticket: waits up to `max`, collecting an approved read.
async fn wait_ticket(id: &RequestId, connection: &ReinsConnection, max: Duration, pool: &State<DbPool>) -> Value {
    // Still Instant: the relay's own wait first.
    let connection_id = ConnectionId(connection.uuid.clone());
    if !matches!(HUB.relay.peek(id, &connection_id), WaitResult::NotFound | WaitResult::Deferred(_)) {
        let waited = HUB.relay.wait(id, &connection_id).await;
        if !matches!(waited, WaitResult::NotFound | WaitResult::Deferred(_)) {
            let seen = HUB.presence.status(&connection.user_uuid.to_string(), now_unix());
            return tools::render_wait(&waited, id, seen);
        }
    }
    match super::tickets::agent_wait(&id.0, &connection.uuid, max, true, pool.inner()).await {
        Some(view) => tools::render_ticket(&view),
        None => tools::render_wait(&WaitResult::NotFound, id, PhoneSeen::Never),
    }
}

async fn list_tickets(connection: &ReinsConnection, conn: &DbConn) -> Value {
    let since = now_unix() - super::ticket_routes::RECENT_SECS;
    let rows = ReinsTicket::list_for_connection(&connection.uuid, since, conn).await;
    let tickets: Vec<Value> = rows.iter().map(super::tickets::status_value).collect();
    tools::success(&json!({"tickets": tickets}))
}

async fn cancel(id: &RequestId, connection: &ReinsConnection, conn: &DbConn, pool: &DbPool) -> Value {
    let cancelled = json!({"status": "cancelled", "ticket": ticket_of(id)});
    // A send waiting out its hold (08 R10): withdrawing it deletes the email before it goes.
    if super::holds::cancel_request(&id.0, connection, conn, pool).await {
        return tools::success(
            &json!({"status": "cancelled", "ticket": ticket_of(id), "note": "The email was not sent."}),
        );
    }
    match HUB.relay.cancel(id, &ConnectionId(connection.uuid.clone())) {
        super::relay::Cancel::Cancelled => return tools::success(&cancelled),
        super::relay::Cancel::PhoneHasIt => {
            return tools::failure(
                "The user's phone already has this request and may be running it; it can no longer be withdrawn. \
Call reins_get_result to learn what happened.",
            );
        }
        super::relay::Cancel::Answered => return tools::failure("This request was already answered."),
        super::relay::Cancel::NotFound => {}
    }
    let Some(mut row) = ReinsTicket::find_for_connection(&id.0, &connection.uuid, conn).await else {
        return tools::failure("Unknown or expired ticket.");
    };
    if super::tickets::transition(&mut row, TicketState::Cancelled, "agent", &json!({}), |_| {}, conn).await {
        tools::success(&cancelled)
    } else {
        tools::failure(&format!("This ticket can no longer be withdrawn: it is {}.", row.state))
    }
}

async fn reply(id: &RequestId, text: &str, connection: &ReinsConnection, conn: &DbConn, pool: &DbPool) -> Value {
    let Some(mut row) = ReinsTicket::find_for_connection(&id.0, &connection.uuid, conn).await else {
        return tools::failure("Unknown or expired ticket.");
    };
    if row.state != TicketState::NeedsInfo.as_str() {
        return tools::failure("Nobody asked a question on this ticket.");
    }
    let Some(keys) = super::tickets::device_keys(&row.user_uuid, conn).await else {
        return tools::failure("The user's phone cannot receive an answer right now.");
    };
    let Ok(sealed) = super::tickets::seal_bytes(&keys.box_pub, text.as_bytes()) else {
        return tools::failure("The answer could not be sent.");
    };
    if !super::tickets::transition(
        &mut row,
        TicketState::Deferred,
        "agent",
        &json!({"replied": true}),
        |t| {
            t.sealed_reply = Some(sealed);
        },
        conn,
    )
    .await
    {
        return tools::failure("The ticket moved on meanwhile.");
    }
    super::tickets::wake_phone(&row.user_uuid, &row.id, conn, pool).await;
    tools::success(&super::tickets::pending_review(&row, None))
}

/// `resources/*` for tickets: this connection's only.
async fn resource(method: &str, params: &Value, connection: &ReinsConnection, conn: &DbConn) -> Result<Value, String> {
    let uri_ticket = || {
        params
            .get("uri")
            .and_then(Value::as_str)
            .and_then(|u| u.strip_prefix(mcp::TICKET_URI))
            .and_then(request_of)
            .ok_or_else(|| "Unknown resource.".to_owned())
    };
    match method {
        "resources/list" => {
            let rows = ReinsTicket::list_for_connection(&connection.uuid, now_unix(), conn).await;
            let resources: Vec<Value> = rows
                .iter()
                .map(|r| {
                    let ticket = ticket_of(&RequestId(r.id.clone()));
                    json!({"uri": format!("{}{ticket}", mcp::TICKET_URI), "name": ticket,
                           "description": format!("A request waiting for review ({})", r.state),
                           "mimeType": "application/json"})
                })
                .collect();
            Ok(json!({"resources": resources}))
        }
        "resources/templates/list" => Ok(json!({"resourceTemplates": [{
            "uriTemplate": format!("{}{{ticket}}", mcp::TICKET_URI), "name": "Ticket",
            "description": "A request waiting for review, by its ticket (rt_...).", "mimeType": "application/json"}]})),
        "resources/read" => {
            let id = uri_ticket()?;
            let row = ReinsTicket::find_for_connection(&id.0, &connection.uuid, conn)
                .await
                .ok_or_else(|| "Unknown resource.".to_owned())?;
            let uri = params.get("uri").cloned().unwrap_or(Value::Null);
            Ok(json!({"contents": [{"uri": uri, "mimeType": "application/json",
                                    "text": super::tickets::status_value(&row).to_string()}]}))
        }
        "resources/subscribe" | "resources/unsubscribe" => {
            let id = uri_ticket()?;
            ReinsTicket::find_for_connection(&id.0, &connection.uuid, conn)
                .await
                .ok_or_else(|| "Unknown resource.".to_owned())?;
            HUB.tickets.subscribe(&connection.uuid, &id.0, method == "resources/subscribe");
            Ok(json!({}))
        }
        _ => Err("Method not found".to_owned()),
    }
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
    // A Managed account's calls go to its org's decision service, never to a phone (Decisions v2 · O2).
    if let Some(org) = super::org::managed_mcp::org_of(&connection, &conn).await {
        let (status, body) =
            super::org::managed_mcp::post(&org, &request.headers, &body, &connection, conn, pool.inner()).await;
        return McpResponse::json(status, body);
    }
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
    // Written at most once a minute: a recent use needs no write before the call goes to the phone.
    let now = now_unix();
    if connection.needs_touch(now)
        && let Err(e) = ReinsConnection::touch(&connection.uuid, now, &conn).await
    {
        warn!("Could not record Reins connection use: {e:?}");
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
            defer,
            ..
        } => {
            let run = ToolRun {
                id,
                name,
                arguments,
                modern,
                via: request.via,
                defer: defer || request.defer,
            };
            run_tool(run, &mcp_servers, connection, conn, pool).await
        }
        Action::Resource {
            id,
            method,
            params,
            modern,
        } => match resource(&method, &params, &connection, &conn).await {
            Ok(result) => McpResponse::json(200, Some(mcp::resource_reply(&id, modern, result))),
            Err(message) => McpResponse::json(200, Some(mcp::rpc_error(&id, INVALID_PARAMS, &message, None))),
        },
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
        Action::Resource {
            id,
            ..
        } => McpResponse::json(429, Some(mcp::rpc_error(&id, RATE_LIMITED, &text, None))),
    };
    response.with_header("Retry-After", retry_secs(wait).to_string())
}

fn method_not_allowed() -> McpResponse {
    McpResponse::json(405, Some(json!({"error": "method_not_allowed"}))).with_header("Allow", "POST".to_owned())
}

/// The server-to-client stream (Streamable HTTP `GET`), for clients that subscribed to tickets: it carries
/// `notifications/resources/updated` for this connection's subscribed tickets, and nothing else. Without
/// `Accept: text/event-stream` (or without a valid token), GET is not supported, as before.
#[get("/mcp")]
#[allow(tail_expr_drop_order, reason = "the stream's temporaries hold nothing whose drop order matters")]
async fn mcp_get(request: McpRequest, conn: DbConn) -> Result<EventStream![], McpResponse> {
    if !request.accept.as_deref().is_some_and(|a| a.contains("text/event-stream")) {
        return Err(method_not_allowed());
    }
    if !origin_allowed(request.origin.as_deref(), &CONFIG.domain_origin()) {
        return Err(McpResponse::json(
            403,
            Some(mcp::rpc_error(&Value::Null, INVALID_REQUEST, "Origin not allowed", None)),
        ));
    }
    let connection = match authenticate(request.authorization.as_deref(), &conn).await {
        Ok(c) => c,
        Err(refused) => return Err(unauthorized(refused)),
    };
    drop(conn);
    let Some(slot) = HUB.tickets.open_stream(&connection.uuid) else {
        return Err(McpResponse::json(429, Some(mcp::rpc_error(&Value::Null, RATE_LIMITED, "Too many streams", None))));
    };
    let mut updates = HUB.tickets.updates();
    let until = tokio::time::Instant::now() + STREAM_LIFETIME;
    let mine = connection.uuid;
    let stream = EventStream! {
        let _slot = slot;
        loop {
            let next = tokio::time::timeout_at(until, updates.recv()).await;
            let Ok(next) = next else { break };
            let (who, ticket) = match next {
                Ok(update) => update,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            };
            if who != mine || !HUB.tickets.subscribed(&mine, &ticket) {
                continue;
            }
            let uri = format!("{}{}", mcp::TICKET_URI, ticket_of(&RequestId(ticket)));
            yield Event::json(&json!({"jsonrpc": "2.0", "method": "notifications/resources/updated",
                                      "params": {"uri": uri}}));
        }
    };
    Ok(stream.heartbeat(Duration::from_secs(25)))
}

/// How long one event stream stays open; clients open another.
const STREAM_LIFETIME: Duration = Duration::from_mins(25);

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
            defer: false,
            reason: None,
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
