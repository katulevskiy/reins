//! Autopilot end to end: the real server binary, the real phone core with a fake model (downloaded and verified like
//! the real one), a simulated AI client calling the tools of an MCP server added on the phone. The phone side only
//! syncs, as the push worker would; nobody taps unless a test says so.

use std::sync::Arc;
use std::time::Duration;

use reins_core::autopilot::testing::{self, FakeRuntime, logits};
use reins_core::{ApprovalChoice, AutopilotMode, DownloadProgress, ModelRuntime, ModelState, PendingKind, Verdict};
use reins_e2e::{AiClient, Phone, Server};
use serde_json::{Value, json};
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn once() -> ApprovalChoice {
    ApprovalChoice {
        selected_message_ids: vec![],
        standing: None,
    }
}

/// A minimal MCP server (JSON answers): one read-only tool and one that changes things.
async fn fake_mcp() -> MockServer {
    let mcp = MockServer::start().await;
    let answer = |method_name: &str, result: Value| {
        Mock::given(method("POST")).and(body_partial_json(json!({"method": method_name}))).respond_with(
            move |req: &wiremock::Request| {
                let body: Value = serde_json::from_slice(&req.body).unwrap_or(Value::Null);
                ResponseTemplate::new(200)
                    .insert_header("content-type", "application/json")
                    .set_body_json(json!({"jsonrpc": "2.0", "id": body["id"], "result": result.clone()}))
            },
        )
    };
    answer(
        "initialize",
        json!({"protocolVersion": "2025-06-18", "capabilities": {"tools": {}}, "serverInfo": {"name": "notes", "version": "1"}}),
    )
    .mount(&mcp)
    .await;
    Mock::given(method("POST"))
        .and(body_partial_json(json!({"method": "notifications/initialized"})))
        .respond_with(ResponseTemplate::new(202))
        .mount(&mcp)
        .await;
    answer(
        "tools/list",
        json!({"tools": [
            {"name": "list_notes", "description": "Lists notes.", "inputSchema": {"type": "object"},
             "annotations": {"readOnlyHint": true}},
            {"name": "add_note", "description": "Adds a note.", "inputSchema": {"type": "object",
             "properties": {"text": {"type": "string"}}, "required": ["text"]},
             "annotations": {"readOnlyHint": false, "destructiveHint": false}}]}),
    )
    .mount(&mcp)
    .await;
    answer("tools/call", json!({"content": [{"type": "text", "text": "Saved note 7."}], "isError": false}))
        .mount(&mcp)
        .await;
    mcp
}

struct Setup {
    server: Server,
    phone: Phone,
    ai: AiClient,
    model: Arc<FakeRuntime>,
    connection: String,
    list_notes: String,
    add_note: String,
    _mcp: MockServer,
    _models: MockServer,
}

/// A phone with the fake model downloaded and an AI paired (the 10-minute rule shortened to nothing) and an MCP server.
async fn setup(email: &str) -> Setup {
    reins_e2e::init_tls();
    let mcp = fake_mcp().await;
    let models = MockServer::start().await;
    let files = testing::package();
    let known = testing::known_model("fake-laya-e2e", &files);
    for (name, bytes) in &files {
        Mock::given(method("GET"))
            .and(path(format!("/models/fake-laya-e2e/{name}")))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(bytes.clone()))
            .mount(&models)
            .await;
    }
    let server = Server::start(20, 10).await;
    server.register(email).await;
    let models_base = format!("{}/models", models.uri());
    let phone = Phone::sign_in_with_config(&server.base, email, move |cfg| {
        cfg.models = vec![known];
        cfg.models_base = models_base;
        cfg.new_connection_secs = 0;
    })
    .await;
    let model = Arc::new(FakeRuntime::default());
    let runtime: Arc<dyn ModelRuntime> = Arc::<FakeRuntime>::clone(&model);
    phone.core.set_model_runtime(runtime);
    let progress: Arc<dyn DownloadProgress> = Arc::new(testing::Progress::default());
    let status = phone.core.download_model(progress).await.expect("download the model");
    assert_eq!(status.state, ModelState::Installed);

    let mut ai = AiClient::new(&server.base);
    ai.register_client().await;
    let wait_url = ai.start_authorization(email).await;
    let code = ai.browser_code(&wait_url).await;
    let item = phone.wait_for_item(Duration::from_secs(20)).await;
    assert_eq!(item.kind, PendingKind::Pairing);
    phone.core.answer_pairing(item.id, true, Some(code), Some("Claude".to_owned())).await.unwrap();
    ai.finish_authorization(&wait_url).await;
    let added = phone
        .core
        .mcp_add_with_token(format!("{}/mcp", mcp.uri()), "notes-token".to_owned(), Some("Notes".to_owned()))
        .await
        .unwrap();
    phone.core.sync(1).await.unwrap();
    let connection = phone.core.connections().await.unwrap().remove(0).id;
    Setup {
        server,
        phone,
        ai,
        model,
        connection,
        list_notes: format!("{}__list_notes", added.id),
        add_note: format!("{}__add_note", added.id),
        _mcp: mcp,
        _models: models,
    }
}

/// Keeps the phone syncing (as its push worker would) until `until` finishes; nobody taps anything.
async fn with_app_open<T>(phone: &Phone, until: impl Future<Output = T>) -> T {
    let mut until = std::pin::pin!(until);
    loop {
        tokio::select! {
            r = &mut until => return r,
            _ = phone.core.sync(2) => {}
        }
    }
}

fn decided(phone: &Phone) -> Vec<(Verdict, String)> {
    phone.notes.decided.lock().unwrap().iter().map(|d| (d.verdict, d.decided_by.clone())).collect()
}

/// The automatic decisions once there are `n` (the AI hears the answer a moment before the phone has noted it).
async fn decided_soon(phone: &Phone, n: usize) -> Vec<(Verdict, String)> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while decided(phone).len() < n && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    decided(phone)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn in_bypass_an_ai_tool_call_completes_without_a_tap() {
    let s = setup("bypass@example.com").await;
    s.phone.core.set_autopilot_mode(Some(s.connection.clone()), Some(AutopilotMode::Bypass), Some(15)).await.unwrap();
    let result = with_app_open(&s.phone, s.ai.tool(&s.add_note, &json!({"text": "buy milk"}))).await;
    assert_eq!(result["isError"], false, "{result}");
    assert_eq!(result["content"][0]["text"], "Saved note 7.", "{result}");
    assert_eq!(decided_soon(&s.phone, 1).await, [(Verdict::Approve, "bypass".to_owned())]);
    assert!(s.phone.core.pending().await.unwrap().is_empty());
    let entry = s.phone.core.activity(5).await.unwrap().into_iter().find(|e| e.decided_by == "bypass").unwrap();
    assert_eq!(entry.outcome, "sent");
    assert!(s.phone.core.grants().await.unwrap().is_empty(), "approved once: no standing grant");
    assert!(!s.server.log().contains("panicked"), "{}", s.server.log());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn in_auto_mode_autopilot_learns_from_the_user_then_approves_alone_and_still_asks_for_something_new() {
    let s = setup("auto@example.com").await;
    s.phone.core.set_autopilot_mode(None, Some(AutopilotMode::Auto), None).await.unwrap();
    s.model.rule("list_notes", logits(0.97, 0.01, 0.02));
    s.model.rule("add_note", logits(0.99, 0.005, 0.005));

    // The user answers twenty times; Autopilot only watches (the class has not earned it yet).
    for i in 0..20 {
        let no_args = json!({});
        let call = s.ai.tool(&s.list_notes, &no_args);
        let user = async {
            let item = s.phone.wait_for_item(Duration::from_secs(30)).await;
            let suggestion = s.phone.core.autopilot_suggestion(item.id.clone()).await.unwrap().expect("evaluated");
            assert!(suggestion.judged, "{i}: {suggestion:?}");
            if i > 0 {
                assert_eq!(suggestion.verdict, Verdict::Approve, "{i}: {suggestion:?}");
                assert!(item.suggestion.as_deref().is_some_and(|l| l.starts_with("Autopilot would approve")));
            }
            s.phone.core.approve(item.id, once()).await.unwrap();
        };
        let (result, ()) = tokio::join!(call, user);
        assert_eq!(result["isError"], false, "{result}");
    }
    assert!(decided(&s.phone).is_empty());

    // Earned: the same kind of request now completes with nobody tapping.
    let result = with_app_open(&s.phone, s.ai.tool(&s.list_notes, &json!({}))).await;
    assert_eq!(result["isError"], false, "{result}");
    assert_eq!(decided_soon(&s.phone, 1).await, [(Verdict::Approve, "autopilot".to_owned())]);
    let profiles = s.phone.core.autopilot_profiles().await.unwrap();
    let class = profiles[0].classes.iter().find(|c| c.class_key.ends_with("/read/list_notes")).unwrap();
    assert!(class.auto_approve && class.decisions == 20, "{class:?}");

    // A tool never approved before waits, however sure the model is; the user says no.
    let args = json!({"text": "transfer everything"});
    let call = s.ai.tool(&s.add_note, &args);
    let user = async {
        let item = s.phone.wait_for_item(Duration::from_secs(30)).await;
        let suggestion = s.phone.core.autopilot_suggestion(item.id.clone()).await.unwrap().unwrap();
        assert!(suggestion.novel && suggestion.verdict == Verdict::Ask, "{suggestion:?}");
        s.phone.core.deny(item.id).await.unwrap();
    };
    let (result, ()) = tokio::join!(call, user);
    assert_eq!(result["isError"], true, "{result}");
    assert_eq!(decided(&s.phone).len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn lockdown_denies_at_once() {
    let s = setup("lockdown@example.com").await;
    s.phone.core.set_autopilot_mode(None, Some(AutopilotMode::Lockdown), None).await.unwrap();
    let result = with_app_open(&s.phone, s.ai.tool(&s.list_notes, &json!({}))).await;
    assert_eq!(result["isError"], true, "{result}");
    assert_eq!(decided_soon(&s.phone, 1).await, [(Verdict::Deny, "lockdown".to_owned())]);
    let entry = s.phone.core.activity(5).await.unwrap().into_iter().find(|e| e.decided_by == "lockdown").unwrap();
    assert_eq!(entry.outcome, "denied");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_pairing_is_never_approved_automatically() {
    let s = setup("pairing@example.com").await;
    s.phone.core.set_autopilot_mode(None, Some(AutopilotMode::Bypass), Some(15)).await.unwrap();
    s.model.set_default(logits(0.9999, 0.00005, 0.00005));
    let mut other = AiClient::new(&s.server.base);
    other.register_client().await;
    let wait_url = other.start_authorization("pairing@example.com").await;
    let code = other.browser_code(&wait_url).await;
    let item = s.phone.wait_for_item(Duration::from_secs(20)).await;
    assert_eq!(item.kind, PendingKind::Pairing);
    assert_eq!(item.suggestion.as_deref(), Some("Autopilot always asks you for this"));
    // Synced again and again, it still waits for the user.
    for _ in 0..3 {
        s.phone.core.sync(1).await.unwrap();
    }
    assert!(decided(&s.phone).is_empty());
    assert_eq!(s.phone.core.pending().await.unwrap().len(), 1);
    s.phone.core.answer_pairing(item.id, true, Some(code), Some("Second".to_owned())).await.unwrap();
    other.finish_authorization(&wait_url).await;
    assert_eq!(s.phone.core.connections().await.unwrap().len(), 2);
}
