use reqwest::StatusCode;
use serde_json::{Value, json};

use crate::harness::{REDIRECT, Server, VERIFIER, between, client, query_param};

#[tokio::test]
async fn discovery_documents_describe_this_server() {
    let server = Server::start().await;
    let prm: Value =
        client().get(server.url("/.well-known/oauth-protected-resource")).send().await.unwrap().json().await.unwrap();
    assert_eq!(prm["resource"], server.url("/mcp"));
    assert_eq!(prm["authorization_servers"], json!([server.base]));
    let via_path: Value = client()
        .get(server.url("/.well-known/oauth-protected-resource/mcp"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(via_path, prm);
    let wrong = client().get(server.url("/.well-known/oauth-protected-resource/other")).send().await.unwrap();
    assert_eq!(wrong.status(), StatusCode::NOT_FOUND);

    let meta: Value =
        client().get(server.url("/.well-known/oauth-authorization-server")).send().await.unwrap().json().await.unwrap();
    assert_eq!(meta["issuer"], server.base);
    assert_eq!(meta["token_endpoint"], server.url("/rewarden/oauth/token"));
    assert_eq!(meta["code_challenge_methods_supported"], json!(["S256"]));
    assert_eq!(meta["authorization_response_iss_parameter_supported"], json!(true));
}

#[tokio::test]
async fn registration_accepts_public_clients_only() {
    let server = Server::start().await;
    assert!(!server.register_client().await.is_empty());
    let bad = json!({"redirect_uris": ["http://evil.example/cb"]});
    let r = client().post(server.url("/rewarden/oauth/register")).json(&bad).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    let body: Value = r.json().await.unwrap();
    assert_eq!(body["error"], "invalid_redirect_uri");
    let secret = json!({"redirect_uris": [REDIRECT], "token_endpoint_auth_method": "client_secret_basic"});
    let r = client().post(server.url("/rewarden/oauth/register")).json(&secret).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn authorize_rejects_bad_requests_without_redirecting_to_untrusted_uris() {
    let server = Server::start().await;
    let client_id = server.register_client().await;
    // Unregistered redirect URI: an error page, never a redirect.
    let url = server.authorize_url(&client_id).replace("claude.ai", "evil.example");
    let r = client().get(url).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    assert!(r.headers().get("location").is_none());
    // Unknown client.
    let r = client().get(server.authorize_url("no-such-client")).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    // Missing PKCE: redirected back to the registered URI with an error and the state.
    let no_pkce = server.authorize_url(&client_id).replace("code_challenge_method=S256", "code_challenge_method=plain");
    let r = client().get(no_pkce).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::SEE_OTHER);
    let location = r.headers()["location"].to_str().unwrap().to_owned();
    assert!(location.starts_with(REDIRECT), "{location}");
    assert_eq!(query_param(&location, "error").as_deref(), Some("invalid_request"));
    assert_eq!(query_param(&location, "state").as_deref(), Some("st-1"));
    assert_eq!(query_param(&location, "iss").as_deref(), Some(server.base.as_str()));
    // Wrong resource.
    let wrong = server.authorize_url_for(&client_id, "https://other.example/mcp");
    let r = client().get(wrong).send().await.unwrap();
    assert_eq!(query_param(r.headers()["location"].to_str().unwrap(), "error").as_deref(), Some("invalid_target"));
}

#[tokio::test]
async fn full_flow_with_replay_and_refresh_rotation() {
    let server = Server::start().await;
    let phone = server.phone("erin@example.com").await;
    phone.register_device().await;
    let tokens = server.connect_ai(&phone, "erin@example.com", Some("Work Claude")).await;
    assert!(!tokens.client_id.is_empty());
    assert!(!tokens.access.is_empty() && tokens.access.matches('.').count() == 2, "access token is a JWT");

    // The connection is listed under the chosen label.
    let (_, list) = phone.get("/connections").await;
    assert_eq!(list["connections"][0]["label"], "Work Claude");
    assert_eq!(list["connections"][0]["id"], tokens.connection_id);
    assert_eq!(list["connections"][0]["client_host"], "claude.ai");

    // Refresh rotates; the old refresh token dies at once.
    let refresh_form = [("grant_type", "refresh_token"), ("refresh_token", tokens.refresh.as_str())];
    let (status, rotated) = server.token(&refresh_form).await;
    assert_eq!(status, StatusCode::OK, "{rotated}");
    assert_ne!(rotated["refresh_token"], tokens.refresh);
    let (status, replay) = server.token(&refresh_form).await;
    assert_eq!((status, replay["error"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_grant")));
}

async fn exchange(
    server: &Server,
    client_id: &str,
    code: &str,
    verifier: &str,
    redirect: &str,
    resource: &str,
) -> (StatusCode, Value) {
    server
        .token(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", redirect),
            ("client_id", client_id),
            ("code_verifier", verifier),
            ("resource", resource),
        ])
        .await
}

#[tokio::test]
async fn authorization_codes_are_single_use_and_bound() {
    let server = Server::start().await;
    let phone = server.phone("frank@example.com").await;
    phone.register_device().await;
    let client_id = server.register_client().await;

    let mut codes = Vec::new();
    for _ in 0..3 {
        let wait_url = server.start_authorization(&client_id, "frank@example.com").await;
        let html = client().get(&wait_url).send().await.unwrap().text().await.unwrap();
        let code: u8 = between(&html, "aria-label=\"code\">", "<").parse().unwrap();
        let (_, pending) = phone.get("/pending?wait=5").await;
        let id = pending["pairings"][0]["id"].as_str().unwrap().to_owned();
        let (status, _) = phone
            .post(
                &format!("/pairings/{id}/response"),
                &json!({"v": 1, "approved": true, "chosen_code": code, "label": null}),
            )
            .await;
        assert_eq!(status, StatusCode::OK);
        let done = client().get(&wait_url).send().await.unwrap();
        let location = done.headers()["location"].to_str().unwrap().to_owned();
        codes.push(query_param(&location, "code").unwrap());
        // A reload of the wait page after completion cannot mint another code.
        let again = client().get(&wait_url).send().await.unwrap();
        assert_eq!(again.status(), StatusCode::BAD_REQUEST);
    }
    let mcp = server.url("/mcp");
    // Wrong verifier burns the code.
    let (status, body) = exchange(&server, &client_id, &codes[0], "x".repeat(43).as_str(), REDIRECT, &mcp).await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_grant")));
    let (_, body) = exchange(&server, &client_id, &codes[0], VERIFIER, REDIRECT, &mcp).await;
    assert_eq!(body["error"], "invalid_grant", "a code that failed once cannot be retried");
    // Wrong redirect_uri.
    let (_, body) = exchange(&server, &client_id, &codes[1], VERIFIER, "https://claude.ai/other", &mcp).await;
    assert_eq!(body["error"], "invalid_grant");
    // Wrong resource.
    let (_, body) = exchange(&server, &client_id, &codes[2], VERIFIER, REDIRECT, "https://other.example/mcp").await;
    assert_eq!(body["error"], "invalid_target");
    // Garbage.
    let (_, body) = exchange(&server, &client_id, "not-a-code", VERIFIER, REDIRECT, &mcp).await;
    assert_eq!(body["error"], "invalid_grant");
}

#[tokio::test]
async fn a_wrong_code_on_the_phone_cancels_the_connection() {
    let server = Server::start().await;
    let phone = server.phone("gina@example.com").await;
    phone.register_device().await;
    let client_id = server.register_client().await;
    let wait_url = server.start_authorization(&client_id, "gina@example.com").await;
    let html = client().get(&wait_url).send().await.unwrap().text().await.unwrap();
    let shown: u64 = between(&html, "aria-label=\"code\">", "<").parse().unwrap();
    let (_, pending) = phone.get("/pending?wait=5").await;
    let pairing = &pending["pairings"][0];
    let id = pairing["id"].as_str().unwrap().to_owned();
    assert_eq!(pairing["client_name"], "Claude");
    assert_eq!(pairing["client_host"], "claude.ai");
    let wrong =
        pairing["choices"].as_array().unwrap().iter().map(|c| c.as_u64().unwrap()).find(|c| *c != shown).unwrap();
    let (status, body) = phone
        .post(
            &format!("/pairings/{id}/response"),
            &json!({"v": 1, "approved": true, "chosen_code": wrong, "label": null}),
        )
        .await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::CONFLICT, Some("wrong_code")));
    // The browser is sent back with access_denied and never gets a code.
    let done = client().get(&wait_url).send().await.unwrap();
    assert_eq!(done.status(), StatusCode::SEE_OTHER);
    let location = done.headers()["location"].to_str().unwrap().to_owned();
    assert_eq!(query_param(&location, "error").as_deref(), Some("access_denied"));
    assert!(query_param(&location, "code").is_none());
    let (_, list) = phone.get("/connections").await;
    assert_eq!(list["connections"], json!([]));
}

#[tokio::test]
async fn unknown_emails_get_the_same_page_and_nothing_to_approve() {
    let server = Server::start().await;
    let client_id = server.register_client().await;
    let wait_url = server.start_authorization(&client_id, "nobody@example.com").await;
    let html = client().get(&wait_url).send().await.unwrap().text().await.unwrap();
    let code = between(&html, "aria-label=\"code\">", "<");
    assert_eq!(code.len(), 2, "a code is shown exactly as for a real account");
    assert!(html.contains("http-equiv=\"refresh\""));
    // A registered user without an approval device looks identical.
    let phone = server.phone("henry@example.com").await;
    drop(phone);
    let wait_url = server.start_authorization(&client_id, "henry@example.com").await;
    let html = client().get(&wait_url).send().await.unwrap().text().await.unwrap();
    assert_eq!(between(&html, "aria-label=\"code\">", "<").len(), 2);
}

/// Anyone can name a client metadata URL: it is fetched only from public addresses, never from this host's network.
#[tokio::test]
async fn client_metadata_documents_are_never_fetched_from_private_addresses() {
    let server = Server::start().await;
    let mock = crate::mock::MockServer::start(|_| {
        crate::mock::Reply::json(200, &json!({"client_id": "x", "redirect_uris": [REDIRECT]}))
    })
    .await;
    for client_id in [
        format!("https://127.0.0.1:{}/client.json", mock.port),
        format!("https://localhost:{}/client.json", mock.port),
        "https://169.254.169.254/latest/meta-data/client.json".to_owned(),
        "https://[::ffff:10.0.0.1]/client.json".to_owned(),
    ] {
        let r = client().get(server.authorize_url(&client_id)).send().await.unwrap();
        assert_eq!(r.status(), StatusCode::BAD_REQUEST, "{client_id}");
        let page = r.text().await.unwrap();
        assert!(page.contains("not allowed"), "{client_id}: {page}");
    }
    assert!(mock.requests().is_empty(), "nothing reached the local server");
}
