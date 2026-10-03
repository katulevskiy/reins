//! `rewarden mcp`: stdio JSON-RPC to the Rewarden server's `/mcp` with the app's session, against a mock server.

mod d1_mock;

use d1_mock::{Mock, logged_in, logged_out};
use rewarden_desktop::config::Paths;
use rewarden_desktop::mcp_bridge;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _};

fn init(id: u64) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": "initialize", "params": {
        "protocolVersion": "2025-06-18",
        "capabilities": {},
        "clientInfo": {"name": "claude-code", "version": "2.0.0"},
    }})
}

fn initialized() -> Value {
    json!({"jsonrpc": "2.0", "method": "notifications/initialized"})
}

/// Runs the bridge over `lines` (then end of input) and returns what it wrote, one message per line.
async fn bridge(paths: &Paths, via: Option<&str>, lines: &[String]) -> Vec<Value> {
    let mut input = lines.join("\n");
    input.push('\n');
    let (out_w, mut out_r) = tokio::io::duplex(1 << 20);
    mcp_bridge::run(paths, via, tokio::io::BufReader::new(input.as_bytes()), out_w).await.unwrap();
    let mut out = String::new();
    out_r.read_to_string(&mut out).await.unwrap();
    out.lines().map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("{e}: {l}"))).collect()
}

fn by_id(out: &[Value], id: u64) -> &Value {
    out.iter().find(|m| m["id"] == id).unwrap_or_else(|| panic!("no answer to {id}: {out:?}"))
}

fn lines(msgs: &[Value]) -> Vec<String> {
    msgs.iter().map(Value::to_string).collect()
}

#[tokio::test]
async fn initialize_list_and_call_go_through_with_the_session_and_the_via_header() {
    let mock = Mock::start().await;
    let app = logged_in(&mock, 3600);
    let out = bridge(
        &app.paths,
        Some("Claude Code"),
        &lines(&[
            init(1),
            initialized(),
            json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
            json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "echo", "arguments": {"x": 1}}}),
            json!({"jsonrpc": "2.0", "id": "p", "method": "ping"}),
        ]),
    )
    .await;
    assert_eq!(by_id(&out, 1)["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(by_id(&out, 2)["result"]["tools"][0]["name"], "echo");
    assert_eq!(by_id(&out, 3)["result"]["content"][0]["text"], "echo {\"x\":1}");
    assert!(out.iter().any(|m| m["id"] == "p" && m["result"] == json!({})));
    // The server's notification in the event stream is passed on; the client's notification gets no answer.
    assert!(out.iter().any(|m| m["method"] == "notifications/message"), "{out:?}");
    assert_eq!(out.len(), 5, "{out:?}");

    let seen = mock.with(|s| s.mcp.clone());
    assert_eq!(seen.len(), 5);
    assert_eq!(seen[0].method, "initialize");
    assert_eq!(seen[0].session, None);
    assert_eq!(seen[0].protocol, None);
    for s in &seen {
        assert_eq!(s.via.as_deref(), Some("Claude Code"));
        assert_eq!(s.authorization.as_deref(), Some("Bearer at-1"));
        assert_eq!(s.accept.as_deref(), Some("application/json, text/event-stream"));
    }
    for s in &seen[1..] {
        assert_eq!(s.session.as_deref(), Some("s-2"), "{}", s.method);
        assert_eq!(s.protocol.as_deref(), Some("2025-06-18"), "{}", s.method);
    }
    assert!(seen.iter().any(|s| s.method == "notifications/initialized"));
    // The session is ended when the harness goes away.
    assert_eq!(mock.with(|s| s.deleted_sessions.clone()), vec!["s-2"]);
}

#[tokio::test]
async fn a_refused_token_is_renewed_and_the_request_sent_again() {
    let mock = Mock::start().await;
    let app = logged_in(&mock, 3600);
    mock.with(|s| s.reject_bearer = 1);
    let out = bridge(&app.paths, None, &lines(&[init(1), json!({"jsonrpc": "2.0", "id": 2, "method": "ping"})])).await;
    assert!(by_id(&out, 1).get("result").is_some(), "{out:?}");
    assert!(by_id(&out, 2).get("result").is_some(), "{out:?}");
    let seen = mock.with(|s| s.mcp.clone());
    assert_eq!(mock.with(|s| s.refreshes), 1);
    // The refused attempt never reached the MCP handler; the one that did carries the renewed token.
    assert_eq!(seen.iter().map(|s| s.method.as_str()).collect::<Vec<_>>(), ["initialize", "ping"]);
    for s in &seen {
        assert!(s.authorization.as_deref().is_some_and(|a| a.starts_with("Bearer at-") && a != "Bearer at-1"));
        assert_eq!(s.via, None);
    }
    // The renewed session was saved: the session file has the new token, never printed.
    let saved = std::fs::read_to_string(app.paths.session_file()).unwrap();
    assert!(!saved.contains("\"at-1\""));
    assert!(!out.iter().any(|m| m.to_string().contains("at-")), "no token in the output");
}

#[tokio::test]
async fn an_expired_token_is_renewed_before_use() {
    let mock = Mock::start().await;
    let app = logged_in(&mock, -5);
    let out = bridge(&app.paths, None, &lines(&[init(1)])).await;
    assert!(by_id(&out, 1).get("result").is_some());
    assert_eq!(mock.with(|s| s.refreshes), 1);
    assert_eq!(mock.with(|s| s.mcp.len()), 1);
}

#[tokio::test]
async fn without_a_session_every_request_gets_a_clear_error() {
    let app = logged_out();
    let out = bridge(
        &app.paths,
        Some("Codex"),
        &lines(&[init(1), initialized(), json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"})]),
    )
    .await;
    assert_eq!(out.len(), 2, "{out:?}");
    for id in [1, 2] {
        let e = &by_id(&out, id)["error"];
        assert_eq!(e["code"], -32000);
        assert!(e["message"].as_str().unwrap().contains("rewarden login"), "{e}");
    }
}

#[tokio::test]
async fn a_session_the_server_ended_says_log_in_again() {
    let mock = Mock::start().await;
    let app = logged_in(&mock, 3600);
    mock.with(|s| {
        s.reject_bearer = 1;
        s.refuse_refresh = true;
    });
    let out = bridge(&app.paths, None, &lines(&[init(7)])).await;
    let message = by_id(&out, 7)["error"]["message"].as_str().unwrap().to_owned();
    assert!(message.contains("ended this app's session") && message.contains("rewarden login"), "{message}");
    assert!(!app.paths.session_file().exists(), "logged out");
}

#[tokio::test]
async fn a_forgotten_mcp_session_is_started_again() {
    let mock = Mock::start().await;
    let app = logged_in(&mock, 3600);
    // The server restarts after the client initialized: the next request finds the session unknown.
    let state = mock.clone();
    let input = format!("{}\n", init(1));
    let (mut in_w, in_r) = tokio::io::duplex(1 << 16);
    let (out_w, mut out_r) = tokio::io::duplex(1 << 20);
    let paths = app.paths.clone();
    let task = tokio::spawn(async move {
        mcp_bridge::run(&paths, None, tokio::io::BufReader::new(in_r), out_w).await.unwrap();
    });
    in_w.write_all(input.as_bytes()).await.unwrap();
    let mut reader = tokio::io::BufReader::new(&mut out_r);
    let mut first = String::new();
    reader.read_line(&mut first).await.unwrap();
    assert!(first.contains("protocolVersion"), "{first}");
    state.with(d1_mock::State::forget_sessions);
    in_w.write_all(format!("{}\n", json!({"jsonrpc": "2.0", "id": 2, "method": "ping"})).as_bytes()).await.unwrap();
    let mut second = String::new();
    reader.read_line(&mut second).await.unwrap();
    let second: Value = serde_json::from_str(&second).unwrap();
    assert_eq!(second["id"], 2);
    assert_eq!(second["result"], json!({}), "{second}");
    drop(in_w);
    task.await.unwrap();
    let methods: Vec<String> = mock.with(|s| s.mcp.iter().map(|m| m.method.clone()).collect());
    assert_eq!(methods, ["initialize", "ping", "initialize", "notifications/initialized", "ping"]);
}

#[tokio::test]
async fn lines_that_are_not_json_get_a_parse_error() {
    let mock = Mock::start().await;
    let app = logged_in(&mock, 3600);
    let out = bridge(&app.paths, None, &["{not json".to_owned(), String::new(), init(1).to_string()]).await;
    assert!(out.iter().any(|m| m["error"]["code"] == -32700), "{out:?}");
    assert!(by_id(&out, 1).get("result").is_some());
}

#[test]
fn the_command_line_bridges_stdin_and_stdout() {
    use std::io::Write as _;
    let app = logged_out();
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_rewarden"))
        .args(["mcp", "--via", "Gemini CLI"])
        .env("REWARDEN_CONFIG_DIR", &app.paths.config_dir)
        .env("REWARDEN_STATE_DIR", &app.paths.state_dir)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(format!("{}\n", init(1)).as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    let answer: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(answer["id"], 1);
    assert!(answer["error"]["message"].as_str().unwrap().contains("rewarden login"));
    assert!(String::from_utf8_lossy(&out.stderr).contains("not logged in"));
}
