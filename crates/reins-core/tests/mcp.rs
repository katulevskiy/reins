//! Universal MCP against a fake MCP server, a fake OAuth authorization server and a fake Reins server: adding a
//! server (OAuth or a token), the tools in the services report, calls covered by a grant, parked and approved, asked
//! every time, refused, large results and heavy tools, refreshing tokens and removing a server.

mod common;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::mcp_mock::{AuthTokens, McpState, bodies, mcp_request, mount_auth, mount_mcp, mount_reins};
use common::{FakeGoogle, FakeKeys, RecordingNotifier};
use data_encoding::BASE64URL_NOPAD;
use reins_core::{
    ApprovalChoice, ApprovalKind, CoreConfig, CoreError, GrantScopeChoice, McpAddStep, McpServerView, ReinsCore,
    StandingGrant,
};
use serde_json::{Value, json};
use url::Url;
use wiremock::MockServer;

const TOKENS: AuthTokens<'static> = AuthTokens {
    access: ["MCP-ACCESS-1", "MCP-ACCESS-2"],
    refresh: ["MCP-REFRESH-1", "MCP-REFRESH-2"],
    client_secret: None,
};

struct World {
    rw: MockServer,
    mcp: MockServer,
    auth: MockServer,
    state: Arc<Mutex<McpState>>,
    core: Arc<ReinsCore>,
    _dir: tempfile::TempDir,
}

impl World {
    async fn new(configure: impl FnOnce(&mut McpState)) -> Self {
        let (rw, mcp, auth) = (MockServer::start().await, MockServer::start().await, MockServer::start().await);
        let state = Arc::new(Mutex::new(McpState::default()));
        configure(&mut state.lock().unwrap());
        mount_reins(&rw).await;
        mount_mcp(&mcp, &auth, &state).await;
        mount_auth(&auth, &TOKENS).await;
        let dir = tempfile::tempdir().unwrap();
        let core = ReinsCore::with_connectors(
            dir.path().to_str().unwrap(),
            &FakeKeys,
            Arc::new(FakeGoogle::new()),
            Arc::new(RecordingNotifier::default()),
            CoreConfig {
                backoff_base: Duration::from_millis(1),
                ..CoreConfig::default()
            },
            vec![],
        )
        .unwrap();
        core.login(rw.uri(), "me@example.com".to_owned(), "pw".to_owned(), None).await.unwrap();
        Self {
            rw,
            mcp,
            auth,
            state,
            core,
            _dir: dir,
        }
    }

    fn url(&self) -> String {
        format!("{}/mcp", self.mcp.uri())
    }

    /// Adds the server, which needs no sign-in.
    async fn add_open(&self) -> McpServerView {
        match self.core.mcp_add(self.url(), Some("Tracker".to_owned())).await.unwrap() {
            McpAddStep::Added {
                server,
            } => server,
            other @ McpAddStep::NeedsSignIn {
                ..
            } => panic!("{other:?}"),
        }
    }

    /// Starts adding the server, which needs an OAuth sign-in: its id and the sign-in page.
    async fn start_sign_in(&self) -> (String, Url) {
        match self.core.mcp_add(self.url(), Some("Tracker".to_owned())).await.unwrap() {
            McpAddStep::NeedsSignIn {
                server_id,
                authorize_url,
            } => (server_id, Url::parse(&authorize_url).unwrap()),
            other @ McpAddStep::Added {
                ..
            } => panic!("{other:?}"),
        }
    }

    /// The sign-in page "redirects back" with code CODE-1.
    async fn finish_sign_in(&self, id: &str, page: &Url) -> McpServerView {
        let state = query(page)["state"].clone();
        let redirect = format!("com.reins2fa.app://mcp-oauth?code=CODE-1&state={state}&iss={}", self.auth.uri());
        self.core.mcp_finish_sign_in(id.to_owned(), redirect).await.unwrap()
    }

    async fn relay(&self, id: &str, tool: &str, args: &Value) {
        self.relay_to(id, "tracker", tool, args).await;
    }

    async fn relay_to(&self, id: &str, server: &str, tool: &str, args: &Value) {
        self.core.engine().process_relayed(mcp_request(id, server, tool, args)).await.unwrap();
    }

    /// The phone's answer to request `id`.
    async fn answer(&self, id: &str) -> Value {
        bodies(&self.rw, "POST", &format!("/reins/api/requests/{id}/response")).await.pop().expect("answered")
    }

    async fn answered(&self, id: &str) -> bool {
        !bodies(&self.rw, "POST", &format!("/reins/api/requests/{id}/response")).await.is_empty()
    }

    /// The last services report.
    async fn report(&self) -> Value {
        bodies(&self.rw, "PUT", "/reins/api/services").await.pop().expect("reported")
    }

    async fn is_pending(&self, id: &str) -> bool {
        self.core.pending().await.unwrap().iter().any(|p| p.id == id)
    }
}

fn query(url: &Url) -> HashMap<String, String> {
    url.query_pairs().into_owned().collect()
}

fn once() -> ApprovalChoice {
    ApprovalChoice {
        selected_message_ids: Vec::new(),
        standing: None,
    }
}

fn remember(resource: &str) -> ApprovalChoice {
    ApprovalChoice {
        selected_message_ids: Vec::new(),
        standing: Some(StandingGrant {
            duration_secs: Some(3_600),
            max_uses: None,
            scope: GrantScopeChoice {
                all_mail: false,
                selected_messages_only: false,
                sender_addresses: Vec::new(),
                sender_domains: Vec::new(),
                subject_pattern: None,
                recipient_addresses: Vec::new(),
                recipient_domains: Vec::new(),
                resources: vec![resource.to_owned()],
                classes: Vec::new(),
            },
        }),
    }
}

fn tool_names(server: &McpServerView) -> Vec<&str> {
    server.tools.iter().map(|t| t.name.as_str()).collect()
}

#[tokio::test]
async fn a_server_is_added_with_an_oauth_sign_in_and_its_tools_are_reported() {
    let w = World::new(|s| s.valid_token = Some("MCP-ACCESS-1".to_owned())).await;
    let (id, page) = w.start_sign_in().await;
    assert_eq!(id, "tracker");

    // The sign-in page: the registered client, the app's redirect, PKCE and the resource.
    assert_eq!(
        format!("{}{}", page.origin().ascii_serialization(), page.path()),
        format!("{}/authorize", w.auth.uri())
    );
    let q = query(&page);
    assert_eq!(q["response_type"], "code");
    assert_eq!(q["client_id"], "client-1");
    assert_eq!(q["redirect_uri"], "com.reins2fa.app://mcp-oauth");
    assert_eq!(q["code_challenge_method"], "S256");
    assert_eq!(q["code_challenge"].len(), 43);
    assert!(q["state"].len() >= 16);
    assert_eq!(q["resource"], w.url());
    assert_eq!(q["scope"], "issues", "the scopes of the resource metadata");
    let registration = bodies(&w.auth, "POST", "/register").await.pop().unwrap();
    assert_eq!(registration["redirect_uris"], json!(["com.reins2fa.app://mcp-oauth"]));
    assert_eq!(registration["token_endpoint_auth_method"], "none");
    let waiting = w.core.mcp_servers().await.unwrap();
    assert_eq!((waiting[0].status.as_str(), waiting[0].tools.len()), ("needs_sign_in", 0));
    assert!(
        bodies(&w.rw, "PUT", "/reins/api/services").await.iter().all(|r| r.get("mcp").is_none()),
        "nothing to report yet"
    );

    // An answer for another sign-in, or another app's redirect, is refused.
    let forged = format!("com.reins2fa.app://mcp-oauth?code=CODE-1&state={}", "x".repeat(22));
    assert!(matches!(w.core.mcp_finish_sign_in(id.clone(), forged).await, Err(CoreError::Invalid { .. })));
    let elsewhere = format!("https://evil.example/cb?code=CODE-1&state={}", q["state"]);
    assert!(matches!(w.core.mcp_finish_sign_in(id.clone(), elsewhere).await, Err(CoreError::Invalid { .. })));

    let server = w.finish_sign_in(&id, &page).await;
    assert_eq!(server.status, "ok");
    assert_eq!(tool_names(&server), ["search", "create_issue", "delete_issue", "export"], "both pages");
    let search = &server.tools[0];
    assert_eq!((search.title.as_str(), search.read_only, search.destructive), ("Search issues", true, false));
    let delete = &server.tools[2];
    assert!(!delete.read_only && delete.destructive, "no hints: a write that is asked every time");

    // PKCE: the verifier sent to the token endpoint is the one the challenge was made from.
    let token_request =
        w.auth.received_requests().await.unwrap().into_iter().rfind(|r| r.url.path() == "/token").unwrap();
    let form: HashMap<String, String> = url::form_urlencoded::parse(&token_request.body).into_owned().collect();
    let verifier = &form["code_verifier"];
    let challenge = BASE64URL_NOPAD.encode(ring::digest::digest(&ring::digest::SHA256, verifier.as_bytes()).as_ref());
    assert_eq!(challenge, q["code_challenge"]);
    assert_eq!((form["grant_type"].as_str(), form["resource"].as_str()), ("authorization_code", w.url().as_str()));

    // The token works and stays on the phone.
    assert!(w.state.lock().unwrap().tokens_seen.iter().any(|t| t == "Bearer MCP-ACCESS-1"));
    let report = w.report().await;
    assert_eq!(report["mcp"][0]["id"], "tracker");
    assert_eq!(report["mcp"][0]["name"], "Tracker");
    assert_eq!(report["mcp"][0]["tools"].as_array().unwrap().len(), 4);
    assert_eq!(report["mcp"][0]["tools"][0]["read_only"], true);
    let everything = format!("{report} {:?}", w.core.mcp_servers().await.unwrap());
    assert!(!everything.contains("MCP-ACCESS") && !everything.contains("MCP-REFRESH"), "{everything}");

    // Adding it again is refused; the started sign-in is over.
    assert!(w.core.mcp_add(w.url(), None).await.is_err());
    let again = format!("com.reins2fa.app://mcp-oauth?code=CODE-1&state={}", q["state"]);
    assert!(w.core.mcp_finish_sign_in(id, again).await.is_err());
}

#[tokio::test]
async fn a_server_is_added_with_a_token_and_speaks_sse_sessions_and_the_older_protocol() {
    let w = World::new(|s| {
        s.sse = true;
        s.reject_latest = true;
        s.valid_token = Some("STATIC-TOKEN".to_owned());
    })
    .await;
    let refused = w.core.mcp_add_with_token(w.url(), "WRONG".to_owned(), Some("Tracker".to_owned())).await;
    assert!(matches!(refused, Err(CoreError::Invalid { .. })), "{refused:?}");
    assert!(w.core.mcp_servers().await.unwrap().is_empty(), "a refused token adds nothing");

    let server =
        w.core.mcp_add_with_token(w.url(), " STATIC-TOKEN ".to_owned(), Some("Tracker".to_owned())).await.unwrap();
    assert_eq!(tool_names(&server).len(), 4);
    {
        let st = w.state.lock().unwrap();
        assert_eq!(st.initialized, 1, "notifications/initialized was sent");
        assert!(!st.protocols.is_empty() && st.protocols.iter().all(|p| p == "2025-03-26"), "{:?}", st.protocols);
    }

    // A call in the session; then the server ends the session and the next call opens a new one.
    w.relay("r1", "create_issue", &json!({"title": "Fix login"})).await;
    w.core.approve("r1".to_owned(), once()).await.unwrap();
    assert_eq!(w.answer("r1").await["result"]["result"]["content"][0]["text"], "created ISS-2");
    w.state.lock().unwrap().session = "ended".to_owned();
    w.relay("r2", "create_issue", &json!({"title": "Again"})).await;
    w.core.approve("r2".to_owned(), once()).await.unwrap();
    assert_eq!(w.answer("r2").await["outcome"], "result");
    let st = w.state.lock().unwrap();
    assert_eq!(st.calls.len(), 2);
    assert!(st.inits >= 3, "initialize again after the session ended (and once refused for the version)");
}

#[tokio::test]
async fn a_read_only_call_is_answered_by_a_grant_after_one_approval() {
    let w = World::new(|_| {}).await;
    w.add_open().await;
    w.relay("r1", "search", &json!({"query": "login"})).await;
    assert!(w.is_pending("r1").await);
    let view = w.core.approval_view("r1".to_owned()).await.unwrap();
    assert_eq!((view.kind, view.action.as_str(), view.service.as_str()), (ApprovalKind::Write, "read", "mcp:tracker"));
    assert!(!view.no_standing);
    assert_eq!(view.resources.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["search"]);
    let mcp = view.mcp.unwrap();
    assert_eq!(
        (mcp.server_name.as_str(), mcp.tool.as_str(), mcp.title.as_str()),
        ("Tracker", "search", "Search issues")
    );
    assert_eq!(mcp.server_url, w.url());
    assert!(mcp.read_only && !mcp.destructive);
    assert!(mcp.arguments_json.contains("\"query\": \"login\""), "{}", mcp.arguments_json);

    w.core.approve("r1".to_owned(), remember("search")).await.unwrap();
    let answer = w.answer("r1").await;
    assert_eq!(answer["result"]["kind"], "mcp");
    assert_eq!(
        answer["result"]["result"],
        json!({"content": [{"type": "text", "text": "found 1 issue for \"login\""}], "structuredContent": {"issues": [{"id": "ISS-1"}]}}),
        "passed through as the server gave it"
    );

    // Covered now: answered without asking.
    w.relay("r2", "search", &json!({"query": "crash"})).await;
    assert!(!w.is_pending("r2").await);
    assert_eq!(w.answer("r2").await["result"]["result"]["content"][0]["text"], "found 1 issue for \"crash\"");
    let grants = w.core.grants().await.unwrap();
    assert_eq!(grants.len(), 1);
    assert_eq!((grants[0].action.as_str(), grants[0].uses), ("read", 1));
    let activity = w.core.activity(10).await.unwrap();
    assert_eq!((activity[0].service.as_str(), activity[0].op.as_str()), ("mcp:tracker", "search"));
    assert_eq!(activity[0].grant_id.as_deref(), Some(grants[0].id.as_str()));

    // A grant to read does not cover a write.
    w.relay("r3", "create_issue", &json!({"title": "x"})).await;
    assert!(w.is_pending("r3").await);
}

#[tokio::test]
async fn a_write_is_parked_shown_and_done_once_approved() {
    let w = World::new(|_| {}).await;
    w.add_open().await;
    w.relay("r1", "create_issue", &json!({"title": "Fix login", "labels": ["bug"]})).await;
    assert!(w.is_pending("r1").await);
    assert!(w.state.lock().unwrap().calls.is_empty(), "nothing happens before the user decides");
    let view = w.core.approval_view("r1".to_owned()).await.unwrap();
    assert_eq!(view.action, "write");
    let mcp = view.mcp.unwrap();
    assert!(!mcp.read_only && !mcp.destructive);
    assert_eq!(mcp.description, "Creates an issue");
    assert!(mcp.arguments_json.contains("\"title\": \"Fix login\"") && mcp.arguments_json.contains('\n'), "pretty");

    w.core.approve("r1".to_owned(), remember("create_issue")).await.unwrap();
    assert_eq!(w.answer("r1").await["result"]["result"]["content"][0]["text"], "created ISS-2");
    assert_eq!(
        w.state.lock().unwrap().calls[0],
        ("create_issue".to_owned(), json!({"title": "Fix login", "labels": ["bug"]}))
    );
    let entry = &w.core.activity(5).await.unwrap()[0];
    assert_eq!(
        (entry.action.as_str(), entry.outcome.as_str(), entry.service.as_str()),
        ("write", "sent", "mcp:tracker")
    );

    // The permission covers the next write of that tool.
    w.relay("r2", "create_issue", &json!({"title": "Next"})).await;
    assert!(!w.is_pending("r2").await);
    assert_eq!(w.state.lock().unwrap().calls.len(), 2);

    // Denied: nothing happens and the AI is told.
    w.relay("r3", "delete_issue", &json!({"id": "ISS-1"})).await;
    w.core.deny("r3".to_owned()).await.unwrap();
    assert_eq!(w.answer("r3").await["outcome"], "denied");
    assert_eq!(w.state.lock().unwrap().calls.len(), 2);
    let entry = &w.core.activity(5).await.unwrap()[0];
    assert_eq!(
        (entry.outcome.as_str(), entry.service.as_str(), entry.op.as_str()),
        ("denied", "mcp:tracker", "delete_issue")
    );
}

#[tokio::test]
async fn a_destructive_tool_is_asked_every_time() {
    let w = World::new(|_| {}).await;
    w.add_open().await;
    w.relay("r1", "delete_issue", &json!({"id": "ISS-1"})).await;
    let view = w.core.approval_view("r1".to_owned()).await.unwrap();
    assert!(view.no_standing && view.mcp.unwrap().destructive);
    let refused = w.core.approve("r1".to_owned(), remember("delete_issue")).await;
    assert!(matches!(refused, Err(CoreError::Invalid { .. })), "{refused:?}");
    w.core.approve("r1".to_owned(), once()).await.unwrap();
    assert_eq!(w.answer("r1").await["result"]["result"]["content"][0]["text"], "deleted");
    w.relay("r2", "delete_issue", &json!({"id": "ISS-2"})).await;
    assert!(w.is_pending("r2").await, "asked again");
    assert!(w.core.grants().await.unwrap().is_empty());
}

#[tokio::test]
async fn unknown_servers_and_tools_and_server_errors_are_answered_with_an_error() {
    let w = World::new(|_| {}).await;
    w.add_open().await;
    w.relay("r1", "nope", &json!({})).await;
    let answer = w.answer("r1").await;
    assert_eq!(answer["outcome"], "error");
    assert!(answer["message"].as_str().unwrap().contains("no tool named nope"), "{answer}");
    w.relay_to("r2", "ghost", "search", &json!({})).await;
    assert!(w.answer("r2").await["message"].as_str().unwrap().contains("not added"));
    assert!(w.state.lock().unwrap().calls.is_empty());
    assert_eq!(w.core.activity(5).await.unwrap()[0].outcome, "error");

    // The server says the arguments are wrong: the AI is told, nothing stays waiting.
    w.relay("r3", "search", &json!({"query": 5})).await;
    w.core.approve("r3".to_owned(), once()).await.unwrap();
    let answer = w.answer("r3").await;
    assert_eq!(answer["outcome"], "error");
    assert!(answer["message"].as_str().unwrap().contains("query must be a string"), "{answer}");
    assert!(!w.is_pending("r3").await);
}

#[tokio::test]
async fn an_expired_token_is_refreshed_and_a_refused_refresh_asks_for_a_new_sign_in() {
    let w = World::new(|s| s.valid_token = Some("MCP-ACCESS-1".to_owned())).await;
    let (id, page) = w.start_sign_in().await;
    w.finish_sign_in(&id, &page).await;

    // The server stops taking the first token: 401, refresh, the same call again.
    w.state.lock().unwrap().valid_token = Some("MCP-ACCESS-2".to_owned());
    w.relay("r1", "create_issue", &json!({"title": "a"})).await;
    w.core.approve("r1".to_owned(), once()).await.unwrap();
    assert_eq!(w.answer("r1").await["outcome"], "result");
    let refresh = w.auth.received_requests().await.unwrap().into_iter().rfind(|r| r.url.path() == "/token").unwrap();
    let form: HashMap<String, String> = url::form_urlencoded::parse(&refresh.body).into_owned().collect();
    assert_eq!((form["grant_type"].as_str(), form["refresh_token"].as_str()), ("refresh_token", "MCP-REFRESH-1"));
    assert_eq!(form["resource"], w.url());

    // Then neither the token nor the (rotated) refresh token works: the approval waits for a new sign-in.
    w.state.lock().unwrap().valid_token = Some("MCP-ACCESS-3".to_owned());
    w.relay("r2", "create_issue", &json!({"title": "b"})).await;
    let failed = w.core.approve("r2".to_owned(), once()).await;
    assert!(matches!(failed, Err(CoreError::ServiceNeedsAttention { .. })), "{failed:?}");
    assert!(w.is_pending("r2").await, "kept for another try");
    assert_eq!(w.core.mcp_servers().await.unwrap()[0].status, "needs_sign_in");
    let McpAddStep::NeedsSignIn {
        authorize_url,
        ..
    } = w.core.mcp_refresh(id).await.unwrap()
    else {
        panic!("a new sign-in is started");
    };
    assert!(authorize_url.contains("client_id=client-1"), "the registered client is reused");
}

#[tokio::test]
async fn a_large_result_marks_the_tool_heavy_and_becomes_a_download_link() {
    let w = World::new(|s| s.big = 1_200_000).await;
    w.add_open().await;
    w.relay("r1", "export", &json!({})).await;
    w.core.approve("r1".to_owned(), once()).await.unwrap();
    let result = w.answer("r1").await["result"]["result"].clone();
    let link = &result["content"][0];
    assert_eq!(link["type"], "resource_link");
    assert_eq!(link["uri"], "https://rw.example/reins/blob/dl-export-1.txt");
    assert_eq!(
        (link["mimeType"].as_str(), link["size"].as_u64()),
        (Some("text/plain; charset=utf-8"), Some(1_200_000))
    );
    assert_eq!(result["content"][1], json!({"type": "text", "text": "done"}), "small items stay");
    let upload =
        w.rw.received_requests()
            .await
            .unwrap()
            .into_iter()
            .find(|r| r.url.path() == "/reins/api/blobs/output")
            .unwrap();
    let q: HashMap<String, String> = upload.url.query_pairs().into_owned().collect();
    assert_eq!(
        (q["connection_id"].as_str(), q["name"].as_str(), q["ttl_secs"].as_str()),
        ("c1", "export-1.txt", "1800")
    );
    assert_eq!(upload.body.len(), 1_200_000);
    let export = w.core.mcp_servers().await.unwrap()[0].tools.iter().find(|t| t.name == "export").cloned().unwrap();
    assert!(export.heavy);
}

#[tokio::test]
async fn a_heavy_tool_is_called_through_the_reins_server() {
    let w = World::new(|s| s.valid_token = Some("STATIC-TOKEN".to_owned())).await;
    w.core.mcp_add_with_token(w.url(), "STATIC-TOKEN".to_owned(), Some("Tracker".to_owned())).await.unwrap();
    assert!(w.core.mcp_set_heavy("tracker".to_owned(), "nope".to_owned(), true).await.is_err());
    w.core.mcp_set_heavy("tracker".to_owned(), "export".to_owned(), true).await.unwrap();
    w.relay("r1", "export", &json!({"format": "csv"})).await;
    w.core.approve("r1".to_owned(), once()).await.unwrap();
    assert_eq!(w.answer("r1").await["result"]["result"]["content"][0]["uri"], "https://rw.example/reins/blob/proxied");
    assert!(w.state.lock().unwrap().calls.is_empty(), "the phone did not call it itself");
    let proxied = bodies(&w.rw, "POST", "/reins/api/mcp/call").await.pop().unwrap();
    assert_eq!(proxied["endpoint"], w.url());
    assert_eq!(proxied["connection_id"], "c1");
    assert_eq!(proxied["request"]["method"], "tools/call");
    assert_eq!(proxied["request"]["params"], json!({"name": "export", "arguments": {"format": "csv"}}));
    let headers: HashMap<String, String> =
        serde_json::from_value::<Vec<(String, String)>>(proxied["headers"].clone()).unwrap().into_iter().collect();
    assert_eq!(headers["Authorization"], "Bearer STATIC-TOKEN");
    assert_eq!(headers["Mcp-Session-Id"], "sess-1");
    assert_eq!(headers["MCP-Protocol-Version"], "2025-06-18");
    // Back to the phone.
    w.core.mcp_set_heavy("tracker".to_owned(), "export".to_owned(), false).await.unwrap();
    assert!(!w.core.mcp_servers().await.unwrap()[0].tools[3].heavy);
}

#[tokio::test]
async fn removing_a_server_drops_its_tools_grants_and_waiting_calls() {
    let w = World::new(|_| {}).await;
    w.add_open().await;
    assert_eq!(w.report().await["mcp"][0]["tools"].as_array().unwrap().len(), 4);
    w.relay("r1", "search", &json!({"query": "a"})).await;
    w.core.approve("r1".to_owned(), remember("search")).await.unwrap();
    w.relay("r2", "create_issue", &json!({"title": "b"})).await;
    assert!(w.is_pending("r2").await);

    w.core.mcp_remove("tracker".to_owned()).await.unwrap();
    assert!(w.report().await.get("mcp").is_none(), "{}", w.report().await);
    assert!(w.core.mcp_servers().await.unwrap().is_empty());
    assert!(w.core.grants().await.unwrap().is_empty(), "its permissions are gone");
    assert!(!w.is_pending("r2").await);
    assert_eq!(w.answer("r2").await["outcome"], "error");
    assert!(matches!(w.core.mcp_remove("tracker".to_owned()).await, Err(CoreError::NotFound)));
    w.relay("r3", "search", &json!({"query": "a"})).await;
    assert!(w.answered("r3").await && w.answer("r3").await["outcome"] == "error");
}
