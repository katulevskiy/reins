//! MCP JSON-RPC protocol handling (spec §4.2, Decisions 13-14): a dual-era dispatcher.
//! Modern (2026-07-28, ChatGPT) requests carry their version in `_meta`; legacy
//! (2025-11-25, Claude) clients open with `initialize`. Everything here is pure; the
//! relay and HTTP live in `mcp_routes.rs`.

use data_encoding::BASE64;
use reins_proto::remote_mcp::McpServerReport;
use serde_json::{Map, Value, json};

use super::tools;

pub const MODERN_VERSION: &str = "2026-07-28";
pub const LEGACY_VERSION: &str = "2025-11-25";
/// Legacy revisions we answer `initialize` for.
pub const LEGACY_VERSIONS: [&str; 3] = ["2025-11-25", "2025-06-18", "2025-03-26"];
pub const SERVER_NAME: &str = "reins";
pub const INSTRUCTIONS: &str = "Reins gives you access to the user's Gmail through their phone: search, read and send, \
and organizing the mailbox (archive, labels, spam, Trash, drafts, attachments, filters). Every request may need the \
user's approval in the Reins app: if a result says the request is waiting or the device is offline, tell the user \
and call reins_get_result with the given request_id after they confirm.";

const META_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
const META_SERVER_INFO: &str = "io.modelcontextprotocol/serverInfo";
/// How long clients may cache `tools/list` (modern era).
const LIST_TTL_MS: u64 = 60_000;

pub const PARSE_ERROR: i64 = -32700;
pub const INVALID_REQUEST: i64 = -32600;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;
pub const HEADER_MISMATCH: i64 = -32020;
pub const UNSUPPORTED_VERSION: i64 = -32022;
/// Server-defined JSON-RPC error: a rate limit; the message says when to try again.
pub const RATE_LIMITED: i64 = -32029;

/// The `MCP-*` request headers relevant to validation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct McpHeaders {
    pub protocol_version: Option<String>,
    pub method: Option<String>,
    pub name: Option<String>,
}

/// What the HTTP layer must do next.
#[derive(Debug, PartialEq)]
pub enum Action {
    /// Answered locally: HTTP status and JSON body (`None` = 202 with no body).
    Reply {
        status: u16,
        body: Option<Value>,
    },
    /// A `tools/call` to run; the reply is built with [`tool_reply`] / [`rpc_error`].
    CallTool {
        id: Value,
        name: String,
        arguments: Value,
        modern: bool,
    },
}

fn server_info() -> Value {
    json!({"name": SERVER_NAME, "version": env!("CARGO_PKG_VERSION")})
}

pub fn rpc_error(id: &Value, code: i64, message: &str, data: Option<Value>) -> Value {
    let mut error = json!({"code": code, "message": message});
    if let (Some(data), Some(obj)) = (data, error.as_object_mut()) {
        obj.insert("data".to_owned(), data);
    }
    json!({"jsonrpc": "2.0", "id": id, "error": error})
}

fn rpc_result(id: &Value, result: Value) -> Value {
    let mut obj = Map::new();
    obj.insert("jsonrpc".to_owned(), json!("2.0"));
    obj.insert("id".to_owned(), id.clone());
    obj.insert("result".to_owned(), result);
    Value::Object(obj)
}

fn error_reply(status: u16, id: &Value, code: i64, message: &str, data: Option<Value>) -> Action {
    Action::Reply {
        status,
        body: Some(rpc_error(id, code, message, data)),
    }
}

/// Modern results carry `resultType` and server info in `_meta`.
fn modern_result(mut result: Value) -> Value {
    if let Some(obj) = result.as_object_mut() {
        obj.insert("resultType".to_owned(), json!("complete"));
        let meta = obj.entry("_meta").or_insert_with(|| json!({}));
        if let Some(meta) = meta.as_object_mut() {
            meta.insert(META_SERVER_INFO.to_owned(), server_info());
        }
    }
    result
}

/// Decodes the `=?base64?…?=` form MCP uses for header values that are not header-safe.
fn decode_header_value(raw: &str) -> String {
    raw.strip_prefix("=?base64?")
        .and_then(|rest| rest.strip_suffix("?="))
        .and_then(|b64| BASE64.decode(b64.as_bytes()).ok())
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .unwrap_or_else(|| raw.to_owned())
}

fn meta_version(params: &Value) -> Option<&str> {
    params.get("_meta").and_then(|m| m.get(META_VERSION)).and_then(Value::as_str)
}

/// Decision 13: a request is modern by `_meta`, by header, or by being `server/discover`.
fn is_modern(headers: &McpHeaders, method: &str, params: &Value) -> bool {
    meta_version(params).is_some()
        || headers.protocol_version.as_deref() == Some(MODERN_VERSION)
        || method == "server/discover"
}

fn unsupported_version(id: &Value, requested: &str) -> Action {
    error_reply(
        400,
        id,
        UNSUPPORTED_VERSION,
        &format!("Unsupported protocol version `{requested}`"),
        Some(json!({"supported": [MODERN_VERSION, LEGACY_VERSION]})),
    )
}

/// Header/body consistency (MCP transport spec): only present headers are compared.
fn check_headers(headers: &McpHeaders, method: &str, params: &Value, id: &Value) -> Result<(), Action> {
    let mismatch = |what: &str| Err(error_reply(400, id, HEADER_MISMATCH, &format!("Header mismatch: {what}"), None));
    if let Some(h) = headers.method.as_deref()
        && decode_header_value(h) != method
    {
        return mismatch("Mcp-Method does not match the request method");
    }
    if method == "tools/call"
        && let Some(h) = headers.name.as_deref()
        && params.get("name").and_then(Value::as_str) != Some(decode_header_value(h).as_str())
    {
        return mismatch("Mcp-Name does not match the tool name");
    }
    if let (Some(h), Some(m)) = (headers.protocol_version.as_deref(), meta_version(params))
        && h != m
    {
        return mismatch("MCP-Protocol-Version does not match the protocol version in _meta");
    }
    Ok(())
}

/// Parses and dispatches one POST body.
/// `services`: the integrations the user's phone reported (`None`: every tool is listed); `mcp`: the MCP servers the
/// user added on the phone, whose tools are listed too.
pub fn handle(headers: &McpHeaders, body: &[u8], services: Option<&[String]>, mcp: &[McpServerReport]) -> Action {
    let null = Value::Null;
    let Ok(value) = serde_json::from_slice::<Value>(body) else {
        return error_reply(400, &null, PARSE_ERROR, "Parse error", None);
    };
    let Some(obj) = value.as_object() else {
        return error_reply(400, &null, INVALID_REQUEST, "Batch requests are not supported", None);
    };
    let id = obj.get("id").cloned();
    let reply_id = id.clone().unwrap_or(Value::Null);
    let Some(method) = obj.get("method").and_then(Value::as_str) else {
        return error_reply(400, &reply_id, INVALID_REQUEST, "Missing method", None);
    };
    if obj.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return error_reply(400, &reply_id, INVALID_REQUEST, "jsonrpc must be \"2.0\"", None);
    }
    let params = obj.get("params").cloned().unwrap_or_else(|| Value::Object(Map::new()));

    // Notifications (no id) never get a body.
    if id.is_none() {
        return if method.starts_with("notifications/") {
            Action::Reply {
                status: 202,
                body: None,
            }
        } else {
            error_reply(400, &null, INVALID_REQUEST, "Requests need an id", None)
        };
    }

    if let Err(reply) = check_headers(headers, method, &params, &reply_id) {
        return reply;
    }
    let modern = is_modern(headers, method, &params);
    if modern {
        let requested = meta_version(&params).or(headers.protocol_version.as_deref());
        if let Some(requested) = requested
            && requested != MODERN_VERSION
        {
            return unsupported_version(&reply_id, requested);
        }
    }
    let finish = |result: Value| Action::Reply {
        status: 200,
        body: Some(rpc_result(
            &reply_id,
            if modern {
                modern_result(result)
            } else {
                result
            },
        )),
    };

    match method {
        "initialize" => {
            let requested = params.get("protocolVersion").and_then(Value::as_str).unwrap_or(LEGACY_VERSION);
            let negotiated = if LEGACY_VERSIONS.contains(&requested) {
                requested
            } else {
                LEGACY_VERSION
            };
            finish(json!({
                "protocolVersion": negotiated,
                "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": server_info(),
                "instructions": INSTRUCTIONS
            }))
        }
        "server/discover" => finish(json!({
            "supportedVersions": [MODERN_VERSION, LEGACY_VERSION],
            "capabilities": {"tools": {"listChanged": false}},
            "serverInfo": server_info(),
            "instructions": INSTRUCTIONS
        })),
        "ping" => finish(json!({})),
        "tools/list" => {
            let mut result = json!({"tools": tools::tool_definitions_with(services, mcp)});
            if modern && let Some(obj) = result.as_object_mut() {
                obj.insert("ttlMs".to_owned(), json!(LIST_TTL_MS));
                obj.insert("cacheScope".to_owned(), json!("private"));
            }
            finish(result)
        }
        "tools/call" => {
            let Some(name) = params.get("name").and_then(Value::as_str) else {
                return error_reply(400, &reply_id, INVALID_PARAMS, "tools/call needs a tool name", None);
            };
            Action::CallTool {
                id: reply_id,
                name: name.to_owned(),
                arguments: params.get("arguments").cloned().unwrap_or_else(|| Value::Object(Map::new())),
                modern,
            }
        }
        _ => error_reply(404, &reply_id, METHOD_NOT_FOUND, &format!("Method not found: {method}"), None),
    }
}

/// The JSON-RPC reply for a finished `tools/call`.
pub fn tool_reply(id: &Value, modern: bool, tool_result: Value) -> Value {
    rpc_result(
        id,
        if modern {
            modern_result(tool_result)
        } else {
            tool_result
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[expect(clippy::needless_pass_by_value, reason = "test helper; keeps call sites terse")]
    fn call(headers: &McpHeaders, body: Value) -> Action {
        handle(headers, body.to_string().as_bytes(), None, &[])
    }

    fn reply(action: Action) -> (u16, Value) {
        match action {
            Action::Reply {
                status,
                body,
            } => (status, body.unwrap_or(Value::Null)),
            other @ Action::CallTool {
                ..
            } => panic!("expected a reply, got {other:?}"),
        }
    }

    #[expect(clippy::needless_pass_by_value, reason = "test helper; keeps call sites terse")]
    fn rpc(id: u64, method: &str, params: Value) -> Value {
        json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
    }

    #[test]
    fn legacy_initialize_negotiates_a_known_version() {
        let h = McpHeaders::default();
        let (status, body) = reply(call(
            &h,
            rpc(
                1,
                "initialize",
                json!({"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "claude-ai"}}),
            ),
        ));
        assert_eq!(status, 200);
        assert_eq!(body["id"], 1);
        assert_eq!(body["result"]["protocolVersion"], "2025-11-25");
        assert_eq!(body["result"]["serverInfo"]["name"], "reins");
        assert!(body["result"]["capabilities"]["tools"].is_object());
        assert!(body["result"].get("resultType").is_none(), "legacy results stay legacy-shaped");
        let (_, older) = reply(call(&h, rpc(2, "initialize", json!({"protocolVersion": "2025-03-26"}))));
        assert_eq!(older["result"]["protocolVersion"], "2025-03-26");
        let (_, unknown) = reply(call(&h, rpc(3, "initialize", json!({"protocolVersion": "1999-01-01"}))));
        assert_eq!(unknown["result"]["protocolVersion"], "2025-11-25");
    }

    #[test]
    fn modern_discover_lists_supported_versions() {
        let h = McpHeaders {
            protocol_version: Some(MODERN_VERSION.to_owned()),
            method: Some("server/discover".to_owned()),
            name: None,
        };
        let params = json!({"_meta": {META_VERSION: MODERN_VERSION, "io.modelcontextprotocol/clientCapabilities": {}}});
        let (status, body) = reply(call(&h, rpc(1, "server/discover", params)));
        assert_eq!(status, 200);
        assert_eq!(body["result"]["supportedVersions"], json!([MODERN_VERSION, LEGACY_VERSION]));
        assert_eq!(body["result"]["resultType"], "complete");
        assert_eq!(body["result"]["_meta"][META_SERVER_INFO]["name"], "reins");
        // server/discover without any version information is still treated as modern.
        let (_, bare) = reply(call(&McpHeaders::default(), rpc(2, "server/discover", json!({}))));
        assert_eq!(bare["result"]["resultType"], "complete");
    }

    #[test]
    fn tools_list_is_deterministic_and_era_shaped() {
        let (_, legacy) = reply(call(&McpHeaders::default(), rpc(1, "tools/list", json!({}))));
        let names: Vec<&str> =
            legacy["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(
            names,
            [
                "gmail_read",
                "gmail_search",
                "gmail_send",
                "reins_request_access",
                "reins_list_accounts",
                "reins_get_result",
                "reins_upload"
            ]
            .into_iter()
            .chain(reins_proto::connector::specs().iter().filter(|s| !s.desktop_only).map(|s| s.tool))
            .collect::<Vec<_>>()
        );
        assert!(legacy["result"].get("ttlMs").is_none());
        let params = json!({"_meta": {META_VERSION: MODERN_VERSION}});
        let (_, modern) = reply(call(&McpHeaders::default(), rpc(2, "tools/list", params)));
        assert_eq!(
            (modern["result"]["ttlMs"].as_u64(), modern["result"]["cacheScope"].as_str()),
            (Some(60_000), Some("private"))
        );
        assert_eq!(modern["result"]["resultType"], "complete");
        assert_eq!(modern["result"]["tools"], legacy["result"]["tools"]);
    }

    #[test]
    fn notifications_get_202_and_no_body() {
        let n = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
        assert_eq!(
            call(&McpHeaders::default(), n),
            Action::Reply {
                status: 202,
                body: None
            }
        );
        let not_a_notification = json!({"jsonrpc": "2.0", "method": "tools/list"});
        assert_eq!(reply(call(&McpHeaders::default(), not_a_notification)).0, 400);
    }

    #[test]
    fn ping_is_answered_in_both_eras() {
        let (status, body) = reply(call(&McpHeaders::default(), rpc(9, "ping", json!({}))));
        assert_eq!((status, &body["result"]), (200, &json!({})));
    }

    #[test]
    fn malformed_requests_get_jsonrpc_errors_never_422() {
        let h = McpHeaders::default();
        let (status, body) = reply(handle(&h, b"not json", None, &[]));
        assert_eq!((status, body["error"]["code"].as_i64()), (400, Some(PARSE_ERROR)));
        let (status, body) = reply(handle(&h, b"[]", None, &[]));
        assert_eq!((status, body["error"]["code"].as_i64()), (400, Some(INVALID_REQUEST)));
        let (status, body) = reply(call(&h, json!({"jsonrpc": "1.0", "id": 1, "method": "ping"})));
        assert_eq!((status, body["error"]["code"].as_i64()), (400, Some(INVALID_REQUEST)));
        let (status, body) = reply(call(&h, json!({"jsonrpc": "2.0", "id": 1})));
        assert_eq!((status, body["error"]["code"].as_i64()), (400, Some(INVALID_REQUEST)));
        let (status, body) = reply(call(&h, rpc(5, "no/such/method", json!({}))));
        assert_eq!((status, body["error"]["code"].as_i64(), &body["id"]), (404, Some(METHOD_NOT_FOUND), &json!(5)));
    }

    #[test]
    fn unsupported_modern_versions_list_the_supported_ones() {
        let params = json!({"_meta": {META_VERSION: "2099-01-01"}});
        let (status, body) = reply(call(&McpHeaders::default(), rpc(1, "tools/list", params)));
        assert_eq!((status, body["error"]["code"].as_i64()), (400, Some(UNSUPPORTED_VERSION)));
        assert_eq!(body["error"]["data"]["supported"], json!([MODERN_VERSION, LEGACY_VERSION]));
        let h = McpHeaders {
            protocol_version: Some("2099-01-01".to_owned()),
            ..McpHeaders::default()
        };
        // A header alone with an unknown value is not modern: legacy clients send their own version.
        assert_eq!(reply(call(&h, rpc(2, "tools/list", json!({})))).0, 200);
    }

    #[test]
    fn headers_must_match_the_body_when_present() {
        let mismatch = |h: McpHeaders, body: Value| {
            let (status, b) = reply(call(&h, body));
            assert_eq!((status, b["error"]["code"].as_i64()), (400, Some(HEADER_MISMATCH)), "{b}");
        };
        mismatch(
            McpHeaders {
                method: Some("tools/list".into()),
                ..Default::default()
            },
            rpc(1, "ping", json!({})),
        );
        mismatch(
            McpHeaders {
                name: Some("gmail_send".into()),
                ..Default::default()
            },
            rpc(1, "tools/call", json!({"name": "gmail_read", "arguments": {}})),
        );
        mismatch(
            McpHeaders {
                protocol_version: Some("2025-11-25".into()),
                ..Default::default()
            },
            rpc(1, "tools/list", json!({"_meta": {META_VERSION: MODERN_VERSION}})),
        );
        // Matching headers, including the base64 form, pass.
        let encoded = format!("=?base64?{}?=", BASE64.encode(b"gmail_read"));
        let ok = McpHeaders {
            method: Some("tools/call".into()),
            name: Some(encoded),
            ..Default::default()
        };
        assert!(matches!(
            call(&ok, rpc(1, "tools/call", json!({"name": "gmail_read", "arguments": {}}))),
            Action::CallTool { .. }
        ));
    }

    #[test]
    fn tools_call_is_handed_to_the_relay() {
        let params = json!({"name": "gmail_search", "arguments": {"query": "from:bank"}, "_meta": {META_VERSION: MODERN_VERSION}});
        match call(&McpHeaders::default(), rpc(7, "tools/call", params)) {
            Action::CallTool {
                id,
                name,
                arguments,
                modern,
            } => {
                assert_eq!((id, name.as_str(), modern), (json!(7), "gmail_search", true));
                assert_eq!(arguments, json!({"query": "from:bank"}));
            }
            other @ Action::Reply {
                ..
            } => panic!("{other:?}"),
        }
        match call(&McpHeaders::default(), rpc(8, "tools/call", json!({"name": "gmail_read"}))) {
            Action::CallTool {
                arguments,
                modern,
                ..
            } => assert_eq!((arguments, modern), (json!({}), false)),
            other @ Action::Reply {
                ..
            } => panic!("{other:?}"),
        }
        let (status, body) = reply(call(&McpHeaders::default(), rpc(9, "tools/call", json!({"arguments": {}}))));
        assert_eq!((status, body["error"]["code"].as_i64()), (400, Some(INVALID_PARAMS)));
    }

    #[test]
    fn tool_replies_are_era_shaped() {
        let result = json!({"content": [{"type": "text", "text": "hi"}], "isError": false});
        let legacy = tool_reply(&json!(1), false, result.clone());
        assert!(legacy["result"].get("resultType").is_none());
        let modern = tool_reply(&json!(1), true, result);
        assert_eq!(modern["result"]["resultType"], "complete");
        assert_eq!(modern["result"]["isError"], false);
    }
}
