//! Large files through the server under the phone's control, and an MCP server added on the phone: the real server
//! binary, the real phone core, a simulated AI client, fake GitHub and MCP servers.

use std::time::Duration;

use rewarden_core::{ApprovalChoice, PendingKind};
use rewarden_e2e::{AiClient, Phone, Server};
use serde_json::{Value, json};
use wiremock::matchers::{body_partial_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const EMAIL: &str = "files@example.com";
const GITHUB_TOKEN: &str = "ghp_E2eFilesToken0123456789";

fn once() -> ApprovalChoice {
    ApprovalChoice {
        selected_message_ids: vec![],
        standing: None,
    }
}

async fn connected(github_base: Option<&str>) -> (Server, Phone, AiClient) {
    let server = Server::start(20, 10).await;
    server.register(EMAIL).await;
    let phone = match github_base {
        Some(base) => Phone::sign_in_with_github(&server.base, EMAIL, base).await,
        None => Phone::sign_in(&server.base, EMAIL).await,
    };
    let mut ai = AiClient::new(&server.base);
    ai.register_client().await;
    let wait_url = ai.start_authorization(EMAIL).await;
    let code = ai.browser_code(&wait_url).await;
    let item = phone.wait_for_item(Duration::from_secs(20)).await;
    phone.core.answer_pairing(item.id, true, Some(code), Some("Claude".to_owned())).await.unwrap();
    ai.finish_authorization(&wait_url).await;
    (server, phone, ai)
}

/// Keeps the app "open" (long-polling) until `until` finishes, answering whatever needs no user.
async fn with_app_open<T>(phone: &Phone, until: impl Future<Output = T>) -> T {
    let mut until = std::pin::pin!(until);
    loop {
        tokio::select! {
            r = &mut until => return r,
            _ = phone.core.sync(2) => {}
        }
    }
}

fn http() -> reqwest::Client {
    reqwest::Client::new()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_ai_uploads_a_file_the_user_approves_it_and_the_link_works() {
    rewarden_e2e::init_tls();
    let (server, phone, ai) = connected(None).await;
    let content = "quarterly numbers\n".repeat(20_000).into_bytes();

    // The AI asks to share a file: the phone answers at once with where to upload it.
    let args = json!({"name": "report.txt", "size": content.len(), "reason": "attach the report to the issue"});
    let ready = with_app_open(&phone, ai.tool("rewarden_upload", &args)).await;
    assert_eq!(ready["isError"], false, "{ready}");
    let data = &ready["structuredContent"];
    assert_eq!(data["status"], "upload_ready", "{ready}");
    let upload_url = data["upload_url"].as_str().unwrap().to_owned();
    let download_url = data["download_url"].as_str().unwrap().to_owned();

    // Before the user decides, nobody can download it.
    let put = http().put(&upload_url).body(content.clone()).send().await.unwrap();
    assert!(put.status().is_success(), "{}", put.text().await.unwrap_or_default());
    assert!(!http().get(&download_url).send().await.unwrap().status().is_success());

    // The phone shows the upload with a preview; the user approves.
    let item = phone.wait_for_item(Duration::from_secs(20)).await;
    assert_eq!(item.kind, PendingKind::Blob, "{item:?}");
    let view = phone.core.blob_view(item.id.clone()).await.unwrap();
    assert_eq!(view.name, "report.txt");
    assert_eq!(view.size, content.len() as u64);
    assert!(view.preview_text.as_deref().is_some_and(|t| t.starts_with("quarterly numbers")), "{view:?}");
    phone.core.answer_blob(item.id, true).await.unwrap();

    let got = http().get(&download_url).send().await.unwrap();
    assert!(got.status().is_success());
    assert_eq!(got.bytes().await.unwrap().as_ref(), content.as_slice());
    assert!(!server.log().contains("panicked"), "{}", server.log());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_large_file_reaches_github_through_the_server_after_the_user_approves() {
    rewarden_e2e::init_tls();
    let github = MockServer::start().await;
    for (at, body) in [
        ("/user", json!({"login": "me", "id": 1})),
        ("/repos/me/app", json!({"full_name": "me/app", "private": true, "default_branch": "main"})),
        (
            "/repos/me/app/git/ref/heads/main",
            json!({"ref": "refs/heads/main", "object": {"sha": "abc123", "type": "commit"}}),
        ),
    ] {
        Mock::given(method("GET"))
            .and(path(at))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&github)
            .await;
    }
    Mock::given(method("GET"))
        .and(path("/repos/me/app/contents/big.bin"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({"message": "Not Found"})))
        .mount(&github)
        .await;
    let content: Vec<u8> = (0..3_000_000u32).map(|i| u8::try_from(i % 251).unwrap()).collect();
    let encoded = data_encoding::BASE64.encode(&content);
    Mock::given(method("PUT"))
        .and(path("/repos/me/app/contents/big.bin"))
        .and(header("authorization", format!("Bearer {GITHUB_TOKEN}").as_str()))
        .and(body_partial_json(json!({"message": "Add big.bin", "content": encoded})))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "commit": {"sha": "c0ffee", "html_url": "https://github.com/me/app/commit/c0ffee"},
            "content": {"sha": "b10b"}})))
        .expect(1)
        .mount(&github)
        .await;
    let (_server, phone, ai) = connected(Some(&github.uri())).await;
    phone.core.add_token_account("github".to_owned(), GITHUB_TOKEN.to_owned()).await.unwrap();

    // Too big for a tool call: the phone answers with an upload link instead of an error.
    let first = json!({"repo": "me/app", "path": "big.bin", "message": "Add big.bin", "branch": "main"});
    let answer = with_app_open(&phone, ai.tool("github_file_put", &first)).await;
    let data = &answer["structuredContent"];
    assert_eq!(data["status"], "upload_required", "{answer}");
    let blob = data["blob"].as_str().unwrap().to_owned();
    let put = http().put(data["upload_url"].as_str().unwrap()).body(content.clone()).send().await.unwrap();
    assert!(put.status().is_success());

    // The AI calls again with the blob; the user sees the file in the approval and approves.
    let mut second = first.clone();
    second["blob"] = json!(blob);
    let call = ai.tool("github_file_put", &second);
    let user = async {
        let item = phone.wait_for_item(Duration::from_secs(30)).await;
        let view = phone.core.approval_view(item.id.clone()).await.unwrap();
        let file = view.blob.clone().expect("the file is shown in the approval");
        assert_eq!(file.size, content.len() as u64);
        phone.core.approve(item.id, once()).await.unwrap();
    };
    let (result, ()) = tokio::join!(call, user);
    assert_eq!(result["isError"], false, "{result}");
    assert_eq!(result["structuredContent"]["commit_sha"], "c0ffee", "{result}");
}

/// A minimal MCP server (JSON answers): one read-only tool and one that changes things.
async fn fake_mcp() -> MockServer {
    let mcp = MockServer::start().await;
    let rpc = |id: &Value, result: Value| json!({"jsonrpc": "2.0", "id": id, "result": result});
    let answer = move |method_name: &str, result: Value| {
        let rpc = rpc;
        Mock::given(method("POST")).and(body_partial_json(json!({"method": method_name}))).respond_with(
            move |req: &wiremock::Request| {
                let body: Value = serde_json::from_slice(&req.body).unwrap_or(Value::Null);
                ResponseTemplate::new(200)
                    .insert_header("content-type", "application/json")
                    .set_body_json(rpc(&body["id"], result.clone()))
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_mcp_server_added_on_the_phone_is_offered_to_the_ai_and_runs_after_approval() {
    rewarden_e2e::init_tls();
    let mcp = fake_mcp().await;
    let (_server, phone, ai) = connected(None).await;
    let added = phone
        .core
        .mcp_add_with_token(format!("{}/mcp", mcp.uri()), "notes-token".to_owned(), Some("Notes".to_owned()))
        .await
        .unwrap();
    assert_eq!(added.tools.len(), 2, "{added:?}");
    phone.core.sync(1).await.unwrap();

    let (_, listed) = ai.rpc(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {}})).await;
    let names: Vec<String> =
        listed["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_owned()).collect();
    let add_note = rewarden_core_name(&added.id, "add_note");
    assert!(names.contains(&add_note), "{names:?}");

    let note = json!({"text": "buy milk"});
    let call = ai.tool(&add_note, &note);
    let user = async {
        let item = phone.wait_for_item(Duration::from_secs(30)).await;
        let view = phone.core.approval_view(item.id.clone()).await.unwrap();
        let shown = view.mcp.clone().expect("an MCP call view");
        assert_eq!(shown.tool, "add_note");
        assert!(shown.arguments_json.contains("buy milk"));
        phone.core.approve(item.id, once()).await.unwrap();
    };
    let (result, ()) = tokio::join!(call, user);
    assert_eq!(result["content"][0]["text"], "Saved note 7.", "the server's result passes through: {result}");
}

fn rewarden_core_name(server: &str, tool: &str) -> String {
    format!("{server}__{tool}")
}
