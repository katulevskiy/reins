//! The desktop app's API (`/reins/desktop/calls`) and its pairing key, against the real server.

use std::time::{Duration, Instant};

use reins_proto::desktop::{GIT_FETCH_TOOL, encode_key, key_fingerprint};
use reqwest::StatusCode;
use serde_json::{Value, json};

use crate::harness::{Phone, Server, Tokens, client};

/// Status, `WWW-Authenticate` and JSON body of one desktop API response.
type Answer = (StatusCode, Option<String>, Value);

async fn read(response: reqwest::Response) -> Answer {
    let status = response.status();
    let challenge = response.headers().get("www-authenticate").and_then(|v| v.to_str().ok()).map(str::to_owned);
    let text = response.text().await.unwrap_or_default();
    (status, challenge, serde_json::from_str(&text).unwrap_or(Value::Null))
}

async fn post_call(server: &Server, token: Option<&str>, body: &Value) -> Answer {
    let mut request = client().post(server.url("/reins/desktop/calls")).json(body);
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    read(request.send().await.expect("desktop call")).await
}

async fn get_call(server: &Server, token: Option<&str>, id: &str) -> Answer {
    let mut request = client().get(server.url(&format!("/reins/desktop/calls/{id}")));
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    read(request.send().await.expect("desktop call lookup")).await
}

fn app_key() -> String {
    encode_key(&[42u8; 32])
}

fn fetch(repo: &str) -> Value {
    json!({"tool": GIT_FETCH_TOOL, "arguments": {"repo": repo, "client_key": app_key(), "nonce": "nonce-1"},
        "account": null})
}

fn sealed_answer() -> Value {
    json!({"v": 1, "outcome": "result", "result": {"kind": "connector",
        "data": {"items": [{"id": "octo/app", "sealed": "c2VhbGVk", "expires_at": 1_900_000_000}]}}})
}

/// Waits for the next relay request on the phone and answers it.
async fn answer_next(phone: &Phone, response: Value) -> Value {
    let (status, pending) = phone.get("/pending?wait=10").await;
    assert_eq!(status, StatusCode::OK, "{pending}");
    let request = pending["requests"][0].clone();
    assert!(request.is_object(), "no request arrived: {pending}");
    let id = request["id"].as_str().expect("request id").to_owned();
    let (status, body) = phone.post(&format!("/requests/{id}/response"), &response).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    request
}

/// A server, a phone and a desktop app paired with its key.
async fn paired() -> (Server, Phone, Tokens) {
    let server = Server::start().await;
    let phone = server.phone("dev@example.com").await;
    phone.register_device().await;
    let key = app_key();
    let (tokens, _, _) =
        server.connect_with(&phone, "dev@example.com", Some("Laptop"), &[("reins_client_key", key.as_str())]).await;
    (server, phone, tokens)
}

/// `GET /reins/desktop/phone` as the app with `token`.
async fn phone_status(server: &Server, token: &str) -> Value {
    let (status, _, body) =
        read(client().get(server.url("/reins/desktop/phone")).bearer_auth(token).send().await.unwrap()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

/// A11: the phone leaves a computer the name the user gave it, sealed (the server only keeps it); the computer reads
/// it back, nobody else does, and a body that is not a sealed name is refused.
#[tokio::test]
async fn a_computer_gets_the_sealed_name_its_phone_left_and_no_one_else_does() {
    let (server, phone, tokens) = paired().await;
    assert_eq!(phone_status(&server, &tokens.access).await["sealed_name"], Value::Null, "none yet");
    let id = &tokens.connection_id;
    let sealed = "A".repeat(598);
    let (status, body) = phone.put(&format!("/connections/{id}/name"), &json!({"sealed": sealed})).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    assert_eq!(phone_status(&server, &tokens.access).await["sealed_name"], sealed.as_str());

    for bad in [json!({"sealed": ""}), json!({"sealed": "not base64!"}), json!({"sealed": "A".repeat(1025)})] {
        let (status, body) = phone.put(&format!("/connections/{id}/name"), &bad).await;
        assert_eq!((status, body["error"].as_str()), (StatusCode::BAD_REQUEST, Some("bad_request")), "{bad}");
    }
    let (status, _) = phone.put("/connections/nope/name", &json!({"sealed": "QQ"})).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // Another device of the account that is not the approval device cannot either.
    let second = server.second_device("dev@example.com").await;
    let (status, body) = second.put(&format!("/connections/{id}/name"), &json!({"sealed": "QQ"})).await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::FORBIDDEN, Some("not_approval_device")));
    assert_eq!(phone_status(&server, &tokens.access).await["sealed_name"], sealed.as_str(), "left as it was");
    // Another account cannot name this computer.
    let other = server.phone("eve@example.com").await;
    other.register_device().await;
    let (status, _) = other.put(&format!("/connections/{id}/name"), &json!({"sealed": "QQ"})).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // Another AI of the account does not see it.
    let ai = server.connect_ai(&phone, "dev@example.com", None).await;
    assert_eq!(phone_status(&server, &ai.access).await["sealed_name"], Value::Null);
    // Cleared: the computer goes by its own name again.
    let (status, _) = phone.put(&format!("/connections/{id}/name"), &json!({"sealed": null})).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(phone_status(&server, &tokens.access).await["sealed_name"], Value::Null);
}

#[tokio::test]
async fn the_app_key_reaches_the_phone_and_its_fingerprint_both_pages() {
    let server = Server::start().await;
    let phone = server.phone("kai@example.com").await;
    phone.register_device().await;
    let key = app_key();
    let shown = format!("Desktop app key: <strong class=\"key\">{}</strong>", key_fingerprint(&key).unwrap());

    let client_id = server.register_client().await;
    let authorize = server.authorize_url_with(&client_id, &server.url("/mcp"), &[("reins_client_key", key.as_str())]);
    let page = client().get(&authorize).send().await.unwrap().text().await.unwrap();
    assert!(page.contains(&shown), "{page}");

    let (_, pairing, wait_page) =
        server.connect_with(&phone, "kai@example.com", None, &[("reins_client_key", key.as_str())]).await;
    assert_eq!(pairing["client_key"], key);
    assert!(wait_page.contains(&shown), "{wait_page}");

    // An AI client has no key: nothing is shown and the pairing carries none.
    let (_, pairing, wait_page) = server.connect_with(&phone, "kai@example.com", None, &[]).await;
    assert!(pairing.get("client_key").is_none(), "{pairing}");
    assert!(!wait_page.contains("Desktop app key"));
}

#[tokio::test]
async fn an_invalid_app_key_is_refused_before_any_pairing() {
    let server = Server::start().await;
    let phone = server.phone("lea@example.com").await;
    phone.register_device().await;
    let client_id = server.register_client().await;
    for bad in ["", "not-a-key", &encode_key(&[1u8; 32])[..20]] {
        let url = server.authorize_url_with(&client_id, &server.url("/mcp"), &[("reins_client_key", bad)]);
        let r = client().get(&url).send().await.unwrap();
        assert_eq!(r.status(), StatusCode::BAD_REQUEST, "{bad:?}");
        assert!(r.text().await.unwrap().contains("The desktop app key is not valid."), "{bad:?}");
    }
    let (_, pending) = phone.get("/pending?wait=0").await;
    assert_eq!(pending["pairings"], json!([]));
}

#[tokio::test]
async fn a_desktop_call_reaches_the_phone_and_the_sealed_answer_comes_back() {
    let (server, phone, tokens) = paired().await;
    let call = json!({"tool": GIT_FETCH_TOOL, "arguments": {"repo": "octo/app", "client_key": app_key(), "nonce": "n-7"},
        "account": " Octo-Cat "});
    let app = post_call(&server, Some(&tokens.access), &call);
    let (answer, request) = tokio::join!(app, answer_next(&phone, sealed_answer()));
    let (status, _, body) = answer;
    assert_eq!(status, StatusCode::OK, "{body}");
    let request_id = request["id"].as_str().unwrap();
    assert_eq!(body["request_id"], request_id);
    assert_eq!(body["status"], "answered");
    assert_eq!(body["outcome"], json!({"outcome": "result", "result": sealed_answer()["result"]}));
    // What the phone saw: the normalized connector call, attributed to the desktop app's connection.
    assert_eq!(request["connection_id"], tokens.connection_id);
    assert_eq!(request["connection_label"], "Laptop");
    assert_eq!(request["account"], "octo-cat");
    assert_eq!(
        request["call"],
        json!({"tool": "connector", "service": "github", "op": "git_fetch",
            "args": {"repo": "octo/app", "client_key": app_key(), "nonce": "n-7"}})
    );
    // A denial is an answer too.
    let again = fetch("octo/app");
    let app = post_call(&server, Some(&tokens.access), &again);
    let denied = json!({"v": 1, "outcome": "denied", "reason": "Not now"});
    let ((status, _, body), _) = tokio::join!(app, answer_next(&phone, denied));
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["outcome"], json!({"outcome": "denied", "reason": "Not now"}));
}

#[tokio::test]
async fn an_absent_phone_is_offline_then_pending_and_a_late_answer_is_fetched_by_id() {
    let (server, phone, tokens) = paired().await;
    let start = Instant::now();
    let (status, _, offline) = post_call(&server, Some(&tokens.access), &fetch("octo/app")).await;
    let elapsed = start.elapsed();
    assert_eq!(status, StatusCode::OK, "{offline}");
    assert_eq!(offline["status"], "offline");
    assert!(elapsed >= Duration::from_millis(1800) && elapsed < Duration::from_millis(3900), "{elapsed:?}");
    let request_id = offline["request_id"].as_str().expect("request_id").to_owned();
    assert_eq!(offline, json!({"request_id": request_id, "status": "offline", "last_seen": "less than a minute ago"}));

    // The phone opens the request but does not decide: pending after the full relay wait.
    let (status, _) = phone.get(&format!("/requests/{request_id}")).await;
    assert_eq!(status, StatusCode::OK);
    let start = Instant::now();
    let (status, _, pending) = get_call(&server, Some(&tokens.access), &request_id).await;
    assert!(start.elapsed() >= Duration::from_millis(3800));
    assert_eq!((status, pending), (StatusCode::OK, json!({"request_id": request_id, "status": "pending"})));

    // The user approves later; the app asks again and gets it, as often as it asks until it expires.
    let (status, _) = phone.post(&format!("/requests/{request_id}/response"), &sealed_answer()).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    for _ in 0..2 {
        let (status, _, late) = get_call(&server, Some(&tokens.access), &request_id).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(late["status"], "answered");
        assert_eq!(late["outcome"]["result"]["data"]["items"][0]["sealed"], "c2VhbGVk");
    }

    // Another connection of the same user, and unknown ids, get nothing.
    let other = server.connect_ai(&phone, "dev@example.com", Some("Claude")).await;
    let (status, _, body) = get_call(&server, Some(&other.access), &request_id).await;
    assert_eq!((status, body), (StatusCode::NOT_FOUND, json!({"error": "not_found"})));
    let (status, _, _) = get_call(&server, Some(&tokens.access), "no-such-request").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _, _) = get_call(&server, Some(&tokens.access), &"x".repeat(65)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn only_desktop_tools_with_valid_arguments_are_relayed() {
    let (server, phone, tokens) = paired().await;
    let token = Some(tokens.access.as_str());
    let ai_tool = json!({"tool": "github_list_repos", "arguments": {}});
    let (status, _, body) = post_call(&server, token, &ai_tool).await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::BAD_REQUEST, Some("unknown_tool")), "{body}");
    assert!(body["message"].is_string());

    let missing_key = json!({"tool": GIT_FETCH_TOOL, "arguments": {"repo": "octo/app", "nonce": "n"}});
    let (status, _, body) = post_call(&server, token, &missing_key).await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_arguments")), "{body}");
    assert!(body["message"].as_str().unwrap().contains("client_key"), "{body}");

    let extra = json!({"tool": GIT_FETCH_TOOL, "arguments": {"repo": "o/r", "client_key": app_key(), "nonce": "n",
        "token": "ghp_x"}});
    assert_eq!(post_call(&server, token, &extra).await.2["error"], "invalid_arguments");

    let r = client()
        .post(server.url("/reins/desktop/calls"))
        .bearer_auth(&tokens.access)
        .body("not json")
        .send()
        .await
        .unwrap();
    let (status, _, body) = read(r).await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_request")));

    let huge = json!({"tool": GIT_FETCH_TOOL, "arguments": {"repo": "x".repeat(600 * 1024)}});
    let (status, _, body) = post_call(&server, token, &huge).await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::PAYLOAD_TOO_LARGE, Some("too_large")), "{body}");

    let (_, pending) = phone.get("/pending?wait=0").await;
    assert_eq!(pending["requests"], json!([]), "refused calls must not be relayed");
}

#[tokio::test]
async fn mcp_never_lists_or_runs_desktop_tools() {
    let (server, phone, tokens) = paired().await;
    let rpc = |body: Value| client().post(server.url("/mcp")).bearer_auth(&tokens.access).json(&body).send();
    let list: Value =
        rpc(json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"})).await.unwrap().json().await.unwrap();
    let names: Vec<&str> =
        list["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"github_list_repos"));
    assert!(!names.iter().any(|n| n.starts_with("github_git_")), "{names:?}");

    let call = json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": {"name": GIT_FETCH_TOOL, "arguments": fetch("octo/app")["arguments"]}});
    let r = rpc(call).await.unwrap();
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    let body: Value = r.json().await.unwrap();
    assert_eq!(body["error"]["code"], -32602, "{body}");
    let (_, pending) = phone.get("/pending?wait=0").await;
    assert_eq!(pending["requests"], json!([]));
}

#[tokio::test]
async fn bad_bearers_get_the_mcp_challenge_and_browsers_are_refused() {
    let (server, phone, tokens) = paired().await;
    for (status, challenge, body) in
        [post_call(&server, None, &fetch("o/r")).await, get_call(&server, None, "some-id").await]
    {
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        let challenge = challenge.expect("WWW-Authenticate");
        assert!(challenge.starts_with("Bearer resource_metadata=\""), "{challenge}");
        assert!(challenge.contains("/.well-known/oauth-protected-resource/mcp"), "{challenge}");
        assert_eq!(body["error"], "unauthorized");
    }
    // Garbage, and a regular Vaultwarden login token, are invalid tokens.
    for token in ["garbage", phone.token.as_str()] {
        let (status, challenge, body) = post_call(&server, Some(token), &fetch("o/r")).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert!(challenge.unwrap().contains("invalid_token"));
        assert_eq!(body["error"], "invalid_token");
    }
    // A revoked connection's token dies at once.
    let (status, _) = phone.delete(&format!("/connections/{}", tokens.connection_id)).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(get_call(&server, Some(&tokens.access), "some-id").await.0, StatusCode::UNAUTHORIZED);

    let other = server.connect_ai(&phone, "dev@example.com", None).await;
    let r = client()
        .post(server.url("/reins/desktop/calls"))
        .bearer_auth(&other.access)
        .header("Origin", "https://evil.example")
        .json(&fetch("o/r"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
}

/// The phone reports its Autopilot modes (the approval device only); each computer is shown the global mode and its
/// own connection's, nothing of another's, and nothing at all before the phone reported.
#[tokio::test]
async fn a_computer_sees_the_phones_modes_its_own_and_no_other_connections() {
    let (server, phone, tokens) = paired().await;
    assert_eq!(phone_status(&server, &tokens.access).await["modes"], Value::Null, "not reported yet");
    let other = server.connect_ai(&phone, "dev@example.com", Some("Claude")).await;
    let report = json!({"v": 1, "mode": "auto", "base": "auto", "span": {},
        "connections": [
            {"connection_id": tokens.connection_id, "mode": "bypass", "span": {"since": 100, "until": 1900}},
            {"connection_id": other.connection_id, "mode": "lockdown"}],
        "desktops": [tokens.connection_id]});
    let (status, body) = phone.put("/modes", &report).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let status = phone_status(&server, &tokens.access).await;
    let mine = &status["modes"];
    assert_eq!((mine["mode"].as_str(), mine["global"].as_str()), (Some("bypass"), Some("auto")), "{mine}");
    assert_eq!(mine["span"]["until"], 1900);
    assert!(mine["reported_at"].as_i64().unwrap() > 0);
    assert!(!mine.to_string().contains(&other.connection_id), "never another connection's: {mine}");
    assert!(!status.to_string().contains("desktops"), "nor which connections are desktop apps");
    assert_eq!(phone_status(&server, &other.access).await["modes"], Value::Null, "an AI's connection is never told");

    // Only the approval device reports, with a body that keeps to the contract.
    let second = server.second_device("dev@example.com").await;
    let (status, body) = second.put("/modes", &json!({"v": 1, "mode": "bypass", "base": "bypass"})).await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::FORBIDDEN, Some("not_approval_device")));
    let long = json!({"v": 1, "mode": "manual", "base": "manual",
        "connections": [{"connection_id": "x".repeat(101), "mode": "manual"}]});
    for bad in [json!({"v": 1, "mode": "sideways", "base": "manual"}), long, json!({"mode": "manual"})] {
        let (status, _) = phone.put("/modes", &bad).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
    }
    assert_eq!(phone_status(&server, &tokens.access).await["modes"]["mode"], "bypass", "left as it was");
    // Another account's computer sees nothing of it.
    let eve = server.phone("eve@example.com").await;
    eve.register_device().await;
    let eves = server.connect_ai(&eve, "eve@example.com", None).await;
    assert_eq!(phone_status(&server, &eves.access).await["modes"], Value::Null);
}

/// Where Autopilot leans reaches the computer while its call waits; who decided comes with the answer. An older
/// phone's answer, without either, keeps the old shape.
#[tokio::test]
async fn the_computer_hears_where_autopilot_leans_and_who_decided() {
    let (server, phone, tokens) = paired().await;
    report_desktops(&phone, &[&tokens.connection_id]).await;
    let (_, _, offline) = post_call(&server, Some(&tokens.access), &fetch("octo/app")).await;
    let id = offline["request_id"].as_str().unwrap().to_owned();
    let lean = json!({"v": 1, "leaning": "approve", "confidence": 0.93});
    let second = server.second_device("dev@example.com").await;
    let (status, _) = second.post(&format!("/requests/{id}/suggestion"), &lean).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "the approval device only");
    let (status, _) = phone.post("/requests/no-such/suggestion", &lean).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = phone
        .post(&format!("/requests/{id}/suggestion"), &json!({"v": 1, "leaning": "approve", "confidence": 7}))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, body) = phone.post(&format!("/requests/{id}/suggestion"), &lean).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    phone.get(&format!("/requests/{id}")).await;
    let (_, _, pending) = get_call(&server, Some(&tokens.access), &id).await;
    assert_eq!(
        (pending["status"].as_str(), pending["suggestion"]["leaning"].as_str()),
        (Some("pending"), Some("approve"))
    );

    let mut answer = sealed_answer();
    answer["decided"] = json!({"by": "autopilot", "confidence": 0.9, "reasons": [{"code": "routine", "approve": 97}],
        "correctable": true});
    let (status, body) = phone.post(&format!("/requests/{id}/response"), &answer).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let (_, _, late) = get_call(&server, Some(&tokens.access), &id).await;
    assert_eq!(late["decided"]["by"], "autopilot", "{late}");
    assert!(late.get("suggestion").is_none(), "answered: it leans no more");
    let (status, _) = phone.post(&format!("/requests/{id}/suggestion"), &lean).await;
    assert_eq!(status, StatusCode::CONFLICT);

    let other_call = fetch("octo/other");
    let app = post_call(&server, Some(&tokens.access), &other_call);
    let ((_, _, plain), _) = tokio::join!(app, answer_next(&phone, sealed_answer()));
    assert!(plain.get("decided").is_none() && plain.get("suggestion").is_none(), "{plain}");
}

/// The phone's modes report, naming `desktops` as its paired desktop apps.
async fn report_desktops(phone: &Phone, desktops: &[&str]) {
    let (status, body) =
        phone.put("/modes", &json!({"v": 1, "mode": "auto", "base": "auto", "desktops": desktops})).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
}

/// Who decided and where Autopilot leans are for the phone's paired desktop apps only: an AI's connection waiting on its
/// own call hears neither, and a new leaning does not end its wait early (its timing says nothing either); the desktop
/// app's wait ends at once with it.
#[tokio::test]
async fn an_ai_never_hears_where_autopilot_leans_or_who_decided() {
    let (server, phone, tokens) = paired().await;
    let ai = server.connect_ai(&phone, "dev@example.com", Some("Claude")).await;
    report_desktops(&phone, &[&tokens.connection_id]).await;
    let lean = |c: f64| json!({"v": 1, "leaning": "approve", "confidence": c});
    // Each asks once and the phone opens it; neither has an answer yet.
    let mut ids = Vec::new();
    for (token, repo) in [(&tokens.access, "octo/desk"), (&ai.access, "octo/ai")] {
        let (status, _, first) = post_call(&server, Some(token), &fetch(repo)).await;
        assert_eq!(status, StatusCode::OK, "{first}");
        let id = first["request_id"].as_str().unwrap().to_owned();
        phone.get(&format!("/requests/{id}")).await;
        ids.push(id);
    }
    let (desk_id, ai_id) = (ids[0].clone(), ids[1].clone());

    // A leaning arriving a second into each wait: the desktop's ends then, with it; the AI's runs its course, plain.
    let wait_with_lean = |token: String, id: String| {
        let server = &server;
        let phone = &phone;
        async move {
            let start = Instant::now();
            let (answer, ()) = tokio::join!(get_call(server, Some(&token), &id), async {
                tokio::time::sleep(Duration::from_millis(1000)).await;
                let (status, _) = phone.post(&format!("/requests/{id}/suggestion"), &lean(0.91)).await;
                assert_eq!(status, StatusCode::NO_CONTENT);
            });
            (answer, start.elapsed())
        }
    };
    let ((_, _, desk), took) = wait_with_lean(tokens.access.clone(), desk_id.clone()).await;
    assert_eq!((desk["status"].as_str(), desk["suggestion"]["leaning"].as_str()), (Some("pending"), Some("approve")));
    assert!(took < Duration::from_millis(3000), "the desktop app's wait ended with the leaning: {took:?}");
    let ((_, _, plain), took) = wait_with_lean(ai.access.clone(), ai_id.clone()).await;
    assert_eq!(plain, json!({"request_id": ai_id, "status": "pending"}), "an AI hears nothing of it");
    assert!(took >= Duration::from_millis(3800), "nor does its wait end early: {took:?}");

    // Answered by Autopilot: only the desktop app is told who decided.
    let mut decided = sealed_answer();
    decided["decided"] = json!({"by": "autopilot", "confidence": 0.9, "reasons": [{"code": "routine", "approve": 97}],
        "correctable": true});
    for id in [&desk_id, &ai_id] {
        let (status, body) = phone.post(&format!("/requests/{id}/response"), &decided).await;
        assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    }
    let (_, _, desk) = get_call(&server, Some(&tokens.access), &desk_id).await;
    assert_eq!(desk["decided"]["by"], "autopilot");
    let (_, _, ai_answer) = get_call(&server, Some(&ai.access), &ai_id).await;
    assert_eq!(ai_answer["status"], "answered");
    assert!(ai_answer.get("decided").is_none() && ai_answer.get("suggestion").is_none(), "{ai_answer}");
    // A peek answers at once, with the same rule.
    let (_, _, desk_peek) = get_call(&server, Some(&tokens.access), &format!("{desk_id}?peek=true")).await;
    assert_eq!(desk_peek["decided"]["by"], "autopilot");
    let (_, _, ai_peek) = get_call(&server, Some(&ai.access), &format!("{ai_id}?peek=true")).await;
    assert_eq!(ai_peek["status"], "answered");
    assert!(ai_peek.get("decided").is_none(), "{ai_peek}");

    // Before the phone has said which connections are its desktop apps, nobody is told.
    let (server, phone, tokens) = paired().await;
    let (_, _, first) = post_call(&server, Some(&tokens.access), &fetch("octo/app")).await;
    let id = first["request_id"].as_str().unwrap().to_owned();
    phone.post(&format!("/requests/{id}/response"), &decided).await;
    let (_, _, answer) = get_call(&server, Some(&tokens.access), &id).await;
    assert!(answer.get("decided").is_none(), "{answer}");
}

/// `GET /reins/desktop/policy` as the app with `token`.
async fn get_policy(server: &Server, token: &str, query: &str) -> Answer {
    read(client().get(server.url(&format!("/reins/desktop/policy{query}"))).bearer_auth(token).send().await.unwrap())
        .await
}

/// Decisions v2 (08 R1): the phone posts the signed policy sealed to each computer; that computer picks up what is
/// newer than it holds, waiting for news; another account's computer, or an AI, gets nothing.
#[tokio::test]
async fn a_computer_picks_up_the_policy_its_phone_sealed_to_it() {
    let (server, phone, tokens) = paired().await;
    let id = tokens.connection_id.clone();
    let item = |version: u64, kind: &str, sealed: &str| json!({"connection_id": id, "version": version, "kind": kind, "sealed": sealed});
    let (status, _, empty) = get_policy(&server, &tokens.access, "").await;
    assert_eq!((status, empty["items"].clone()), (StatusCode::OK, json!([])), "nothing yet, no wait asked");

    let post = json!({"items": [item(10, "snapshot", "c25hcA"), item(11, "tighten", "dGln"),
        {"connection_id": "someone-else", "version": 1, "kind": "snapshot", "sealed": "eA"}]});
    let (status, body) = phone.post("/desktop/policy", &post).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let (status, _, mail) = get_policy(&server, &tokens.access, "?after=0").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        mail["items"],
        json!([{"version": 10, "kind": "snapshot", "sealed": "c25hcA"},
            {"version": 11, "kind": "tighten", "sealed": "dGln"}])
    );
    let (_, _, newer) = get_policy(&server, &tokens.access, "?after=10").await;
    assert_eq!(newer["items"].as_array().map(Vec::len), Some(1));

    // A wait ends with the next post.
    let start = Instant::now();
    let waiting = get_policy(&server, &tokens.access, "?after=11&wait=20");
    let later = async {
        tokio::time::sleep(Duration::from_millis(500)).await;
        phone.post("/desktop/policy", &json!({"items": [item(12, "snapshot", "bmV3")]})).await
    };
    let ((status, _, woke), (posted, _)) = tokio::join!(waiting, later);
    assert_eq!((status, posted), (StatusCode::OK, StatusCode::NO_CONTENT));
    assert_eq!(woke["items"], json!([{"version": 12, "kind": "snapshot", "sealed": "bmV3"}]));
    assert!(start.elapsed() < Duration::from_secs(10), "{:?}", start.elapsed());

    // Too many items at once is refused; another account cannot post for this computer.
    let many: Vec<Value> =
        (0..=reins_proto::desktop::MAX_POLICY_ITEMS as u64).map(|v| item(100 + v, "tighten", "eA")).collect();
    let (status, _) = phone.post("/desktop/policy", &json!({"items": many})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    // An item the server can't keep (empty, oversized) is an error, so the phone keeps the change and sends it again.
    let (status, _) = phone.post("/desktop/policy", &json!({"items": [item(13, "tighten", "")]})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "a refused item is never a silent 204");
    let other = server.phone("eve@example.com").await;
    other.register_device().await;
    let (status, _) = other.post("/desktop/policy", &json!({"items": [item(50, "snapshot", "ZXZl")]})).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "skipped silently");
    let (_, _, after) = get_policy(&server, &tokens.access, "?after=12").await;
    assert_eq!(after["items"], json!([]));
    // An AI's connection has no policy of its own; a missing token is refused.
    let ai = server.connect_ai(&phone, "dev@example.com", None).await;
    assert_eq!(get_policy(&server, &ai.access, "").await.2["items"], json!([]));
    let (status, _, _) = read(client().get(server.url("/reins/desktop/policy")).send().await.unwrap()).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

/// `POST /reins/desktop/folders` as the app with `token`.
async fn post_folders(server: &Server, token: &str, body: &Value) -> Answer {
    read(client().post(server.url("/reins/desktop/folders")).bearer_auth(token).json(body).send().await.unwrap()).await
}

/// Decisions v2: a computer proposes its folders; its own phone reads them (the latest only), another account's phone
/// never does, and nothing wakes the phone for it.
#[tokio::test]
async fn a_computer_proposes_its_folders_to_its_own_phone() {
    let (server, phone, tokens) = paired().await;
    let (status, empty) = phone.get("/desktop/folders").await;
    assert_eq!((status, empty["items"].clone()), (StatusCode::OK, json!([])));
    let first =
        json!({"home": "/home/me", "folders": [{"path": "/home/me/.claude/projects/*/memory", "kind": "scratch"}]});
    let (status, _, _) = post_folders(&server, &tokens.access, &first).await;
    assert_eq!(status, StatusCode::OK);
    let latest = json!({"home": "/home/me", "folders": [{"path": "/mnt/data/dev", "kind": "root"}]});
    post_folders(&server, &tokens.access, &latest).await;
    let (_, read_back) = phone.get("/desktop/folders").await;
    let items = read_back["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["connection_id"], json!(tokens.connection_id));
    assert_eq!(items[0]["proposal"], latest, "the latest replaces the one before");
    // Too many folders, or not a proposal: refused.
    let many: Vec<Value> =
        (0..=reins_proto::desktop::MAX_FOLDERS).map(|i| json!({"path": format!("/x/y/{i}"), "kind": "root"})).collect();
    let (status, _, _) = post_folders(&server, &tokens.access, &json!({"home": "/home/me", "folders": many})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _, _) = post_folders(&server, &tokens.access, &json!({"roots": []})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    // Another account's phone sees none of it; a missing token is refused.
    let other = server.phone("eve@example.com").await;
    other.register_device().await;
    assert_eq!(other.get("/desktop/folders").await.1["items"], json!([]));
    let (status, _, _) =
        read(client().post(server.url("/reins/desktop/folders")).json(&latest).send().await.unwrap()).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}
