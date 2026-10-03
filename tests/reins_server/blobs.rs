//! The blob store against the real server: upload slots, decisions, downloads, sending and fetching through a local
//! mock, limits, expiry and owner checks.

use std::time::Duration;

use data_encoding::BASE64;
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

use crate::{
    harness::{Options, Phone, Server, Tokens, client},
    mock::{MockServer, Reply},
};

async fn setup(allow_loopback: bool) -> (Server, Phone, Tokens) {
    let server = Server::start_with(Options {
        allow_loopback,
        ..Options::default()
    })
    .await;
    let phone = server.phone("files@example.com").await;
    phone.register_device().await;
    let tokens = server.connect_ai(&phone, "files@example.com", Some("Claude")).await;
    (server, phone, tokens)
}

fn slot_request(connection_id: &str, name: &str, max_bytes: u64, purpose: &Value) -> Value {
    json!({"v": 1, "connection_id": connection_id, "name": name, "max_bytes": max_bytes, "purpose": purpose,
        "ttl_secs": 600})
}

async fn open_slot(phone: &Phone, request: &Value) -> Value {
    let (status, slot) = phone.post("/blobs", request).await;
    assert_eq!(status, StatusCode::OK, "{slot}");
    slot
}

async fn put(url: &str, body: Vec<u8>) -> (StatusCode, Value) {
    let r = client().put(url).body(body).send().await.expect("upload");
    let status = r.status();
    (status, r.json().await.unwrap_or(Value::Null))
}

async fn download(url: &str) -> reqwest::Response {
    client().get(url).send().await.expect("download")
}

fn secret_of(url: &str) -> &str {
    url.rsplit('/').next().unwrap()
}

#[tokio::test]
async fn an_upload_arrives_with_a_preview_waits_for_the_user_and_then_downloads() {
    let (server, phone, tokens) = setup(false).await;
    let purpose = json!({"kind": "upload", "reason": "Share the notes with the reviewer"});
    let slot = open_slot(&phone, &slot_request(&tokens.connection_id, "notes.txt", 1000, &purpose)).await;
    let upload_url = slot["upload_url"].as_str().unwrap().to_owned();
    let download_url = slot["download_url"].as_str().unwrap().to_owned();
    let id = slot["id"].as_str().unwrap().to_owned();
    assert!(upload_url.starts_with(&server.url("/reins/blob/")), "{upload_url}");
    assert_ne!(upload_url, download_url);
    assert_eq!(download(&download_url).await.status(), StatusCode::NOT_FOUND, "nothing to download yet");

    let (status, info) = phone.get(&format!("/blobs/{id}")).await;
    assert_eq!(
        (status, info["state"].as_str(), &info["preview"]),
        (StatusCode::OK, Some("waiting"), &json!({"kind": "none"}))
    );

    let (status, answer) = put(&upload_url, b"hello blob\nsecond line\n".to_vec()).await;
    assert_eq!(status, StatusCode::CREATED, "{answer}");
    assert_eq!((answer["blob"].as_str(), answer["size"].as_u64()), (Some(id.as_str()), Some(23)));
    assert_eq!(answer["sha256"].as_str().map(str::len), Some(64));
    assert!(answer["next"].as_str().unwrap().contains("approve"), "{answer}");
    let (status, _) = put(&upload_url, b"again".to_vec()).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "an upload link works once");

    // The phone hears about the upload once, with the preview.
    let (_, pending) = phone.get("/pending?wait=5").await;
    let blob = &pending["blobs"][0];
    assert_eq!(blob["id"], id.as_str());
    assert_eq!((blob["state"].as_str(), blob["connection_label"].as_str()), (Some("uploaded"), Some("Claude")));
    assert_eq!(blob["content_type"], "text/plain; charset=utf-8");
    assert_eq!(blob["preview"], json!({"kind": "text", "head": "hello blob\nsecond line\n", "truncated": false}));
    assert_eq!(blob["purpose"], purpose);
    let (_, again) = phone.get("/pending?wait=0").await;
    assert!(again.get("blobs").is_none(), "delivered once: {again}");
    assert_eq!(download(&download_url).await.status(), StatusCode::NOT_FOUND, "not approved yet");

    // Another user's phone sees nothing of it.
    let other = server.phone("other@example.com").await;
    other.register_device().await;
    assert_eq!(other.get(&format!("/blobs/{id}")).await.0, StatusCode::NOT_FOUND);
    assert_eq!(
        other.post(&format!("/blobs/{id}/decision"), &json!({"v": 1, "approved": true})).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(other.get(&format!("/blobs/{id}/content")).await.0, StatusCode::NOT_FOUND);
    assert_eq!(other.delete(&format!("/blobs/{id}")).await.0, StatusCode::NOT_FOUND);

    let (status, decided) = phone.post(&format!("/blobs/{id}/decision"), &json!({"v": 1, "approved": true})).await;
    assert_eq!((status, decided["state"].as_str()), (StatusCode::OK, Some("approved")), "{decided}");
    let (status, body) = phone.post(&format!("/blobs/{id}/decision"), &json!({"v": 1, "approved": false})).await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::CONFLICT, Some("already_answered")));

    let r = download(&download_url).await;
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(
        r.headers()["content-disposition"].to_str().unwrap(),
        "attachment; filename=\"notes.txt\"; filename*=UTF-8''notes.txt"
    );
    assert_eq!(r.headers()["content-type"], "text/plain; charset=utf-8");
    assert_eq!(r.headers()["x-content-type-options"], "nosniff");
    assert_eq!(r.bytes().await.unwrap().as_ref(), b"hello blob\nsecond line\n");

    // The phone reads the bytes itself, whole or a range.
    let r = phone.raw(Method::GET, &format!("/blobs/{id}/content")).header("Range", "bytes=0-4").send().await.unwrap();
    assert_eq!(r.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(r.headers()["content-range"], "bytes 0-4/23");
    assert_eq!(r.bytes().await.unwrap().as_ref(), b"hello");
    let r = phone.raw(Method::GET, &format!("/blobs/{id}/content")).header("Range", "bytes=50-").send().await.unwrap();
    assert_eq!(r.status(), StatusCode::RANGE_NOT_SATISFIABLE);

    // Capability secrets never reach the log.
    let log = server.log();
    assert!(log.contains("/reins/blob/"), "uploads are logged, without their secret");
    assert!(!log.contains(secret_of(&upload_url)) && !log.contains(secret_of(&download_url)), "secret in the log");

    assert_eq!(phone.delete(&format!("/blobs/{id}")).await.0, StatusCode::NO_CONTENT);
    assert_eq!(download(&download_url).await.status(), StatusCode::NOT_FOUND);
    let files = std::fs::read_dir(server.data_dir().join("reins-blobs")).unwrap().count();
    assert_eq!(files, 0, "deleting a blob deletes its file");
}

#[tokio::test]
async fn a_denied_upload_is_gone_and_sizes_and_slots_are_checked() {
    let (server, phone, tokens) = setup(false).await;
    let purpose = json!({"kind": "upload", "reason": "r"});
    let slot = open_slot(&phone, &slot_request(&tokens.connection_id, "pic.png", 300, &purpose)).await;
    let url = slot["upload_url"].as_str().unwrap();
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.resize(301, 0);
    let (status, body) = put(url, png.clone()).await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::PAYLOAD_TOO_LARGE, Some("too_large")));
    png.truncate(300);
    let (status, _) = put(url, png.clone()).await;
    assert_eq!(status, StatusCode::CREATED, "a failed upload may be retried");
    let id = slot["id"].as_str().unwrap();
    let (_, info) = phone.get(&format!("/blobs/{id}")).await;
    assert_eq!(info["preview"], json!({"kind": "image", "mime": "image/png", "data_base64": BASE64.encode(&png)}));
    let (status, denied) = phone.post(&format!("/blobs/{id}/decision"), &json!({"v": 1, "approved": false})).await;
    assert_eq!((status, denied["state"].as_str()), (StatusCode::OK, Some("denied")));
    assert_eq!(download(slot["download_url"].as_str().unwrap()).await.status(), StatusCode::NOT_FOUND);
    assert_eq!(std::fs::read_dir(server.data_dir().join("reins-blobs")).unwrap().count(), 0);

    // Raw POST uploads work too; a tool input is never downloadable and not decided on its own.
    let tool = json!({"kind": "tool_input", "tool": "github_release_asset_upload"});
    let slot = open_slot(&phone, &slot_request(&tokens.connection_id, "app.zip", 10, &tool)).await;
    assert!(slot["download_url"].is_null());
    let r = client().post(slot["upload_url"].as_str().unwrap()).body("PK\x03\x04data").send().await.unwrap();
    assert_eq!(r.status(), StatusCode::CREATED);
    let answer: Value = r.json().await.unwrap();
    assert!(answer["next"].as_str().unwrap().contains("github_release_asset_upload"), "{answer}");
    let id = slot["id"].as_str().unwrap();
    let (status, _) = phone.post(&format!("/blobs/{id}/decision"), &json!({"v": 1, "approved": true})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (_, info) = phone.get(&format!("/blobs/{id}")).await;
    assert_eq!(info["preview"], json!({"kind": "binary", "description": "ZIP archive"}));

    // Bad slots.
    for (request, why) in [
        (slot_request(&tokens.connection_id, "../x", 10, &purpose), "path in the name"),
        (slot_request(&tokens.connection_id, "x", 0, &purpose), "no size"),
        (slot_request(&tokens.connection_id, "x", (1 << 30) + 1, &purpose), "too large"),
        (slot_request(&tokens.connection_id, "x", 10, &json!({"kind": "output"})), "outputs are not uploaded"),
    ] {
        assert_eq!(phone.post("/blobs", &request).await.0, StatusCode::BAD_REQUEST, "{why}");
    }
    let (status, body) = phone.post("/blobs", &slot_request("no-such-connection", "x", 10, &purpose)).await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::NOT_FOUND, Some("not_found")));
    let mut versioned = slot_request(&tokens.connection_id, "x", 10, &purpose);
    versioned["v"] = json!(2);
    assert_eq!(phone.post("/blobs", &versioned).await.1["error"], "bad_version");
    // A connection of another user cannot be named.
    let other = server.phone("other2@example.com").await;
    other.register_device().await;
    assert_eq!(
        other.post("/blobs", &slot_request(&tokens.connection_id, "x", 10, &purpose)).await.0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn a_tool_input_is_sent_on_as_base64_json_and_as_a_raw_body_across_a_redirect() {
    let (_server, phone, tokens) = setup(true).await;
    let github = MockServer::start(|r| match r.path.as_str() {
        "/repos/o/r/contents/bin/tool" => Reply::json(201, &json!({"content": {"sha": "abc123"}})),
        _ => Reply::new(404, "text/plain", "not found"),
    })
    .await;
    let tool = json!({"kind": "tool_input", "tool": "github_file_put"});
    let slot = open_slot(&phone, &slot_request(&tokens.connection_id, "tool", 100_000, &tool)).await;
    let id = slot["id"].as_str().unwrap();
    let data: Vec<u8> = (0..70_000u32).map(|i| u8::try_from(i % 253).unwrap()).collect();
    assert_eq!(put(slot["upload_url"].as_str().unwrap(), data.clone()).await.0, StatusCode::CREATED);

    let send = json!({"v": 1, "method": "PUT", "url": github.url("/repos/o/r/contents/bin/tool"),
        "headers": [["Authorization", "Bearer gh-secret"], ["Accept", "application/vnd.github+json"]],
        "body": {"kind": "json_base64", "json": {"message": "Add tool", "branch": "main"}, "field": "content"}});
    let (status, result) = phone.post(&format!("/blobs/{id}/send"), &send).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["status"], 201);
    assert_eq!(result["truncated"], false);
    assert_eq!(serde_json::from_str::<Value>(result["body"].as_str().unwrap()).unwrap()["content"]["sha"], "abc123");
    assert_eq!(result["headers"], json!([["content-type", "application/json"]]));
    let seen = &github.requests()[0];
    assert_eq!((seen.method.as_str(), seen.header("authorization")), ("PUT", Some("Bearer gh-secret")));
    assert_eq!(seen.header("content-type"), Some("application/json"));
    assert_eq!(seen.header("content-length").map(str::to_owned), Some(seen.body.len().to_string()));
    let forwarded: Value = serde_json::from_slice(&seen.body).unwrap();
    assert_eq!((forwarded["message"].as_str(), forwarded["branch"].as_str()), (Some("Add tool"), Some("main")));
    assert_eq!(BASE64.decode(forwarded["content"].as_str().unwrap().as_bytes()).unwrap(), data);

    // A raw upload redirected (307) to another host: the body follows, the phone's headers do not.
    let uploads = MockServer::start(|_| Reply::json(201, &json!({"id": 9}))).await;
    let target = format!("http://localhost:{}/assets?name=tool", uploads.port);
    let redirector = MockServer::start(move |_| Reply::redirect(307, &target)).await;
    let send = json!({"v": 1, "method": "POST", "url": redirector.url("/upload"),
        "headers": [["Authorization", "Bearer gh-secret"], ["Content-Type", "application/octet-stream"]]});
    let (status, result) = phone.post(&format!("/blobs/{id}/send"), &send).await;
    assert_eq!((status, result["status"].as_u64()), (StatusCode::OK, Some(201)), "{result}");
    assert_eq!(redirector.requests()[0].header("authorization"), Some("Bearer gh-secret"));
    let landed = &uploads.requests()[0];
    assert_eq!((landed.method.as_str(), landed.path.as_str()), ("POST", "/assets?name=tool"));
    assert_eq!(landed.header("authorization"), None, "headers never follow a redirect to another host");
    assert_eq!(landed.header("content-type"), Some("application/octet-stream"));
    assert_eq!(landed.body, data);

    // Refused before anything is sent.
    for (send, why) in [
        (json!({"v": 1, "method": "GET", "url": github.url("/x"), "headers": []}), "method"),
        (json!({"v": 1, "method": "PUT", "url": "http://10.0.0.1/x", "headers": []}), "private address"),
        (json!({"v": 1, "method": "PUT", "url": "ftp://example.com/x", "headers": []}), "scheme"),
        (json!({"v": 1, "method": "PUT", "url": github.url("/x"), "headers": [["Host", "evil"]]}), "host header"),
    ] {
        assert_eq!(phone.post(&format!("/blobs/{id}/send"), &send).await.0, StatusCode::BAD_REQUEST, "{why}");
    }
    let waiting = open_slot(&phone, &slot_request(&tokens.connection_id, "later", 10, &tool)).await;
    let send = json!({"v": 1, "method": "PUT", "url": github.url("/x"), "headers": []});
    let (status, _) = phone.post(&format!("/blobs/{}/send", waiting["id"].as_str().unwrap()), &send).await;
    assert_eq!(status, StatusCode::CONFLICT, "nothing to send before the upload");
}

#[tokio::test]
async fn without_the_test_switch_loopback_and_http_are_refused() {
    let (_server, phone, tokens) = setup(false).await;
    let mock = MockServer::start(|_| Reply::new(200, "text/plain", "x")).await;
    for url in [mock.url("/a"), format!("https://127.0.0.1:{}/a", mock.port), "https://localhost/a".to_owned()] {
        let fetch = json!({"v": 1, "connection_id": tokens.connection_id, "url": url, "headers": [], "name": "a",
            "max_bytes": 10, "ttl_secs": 60});
        let (status, body) = phone.post("/blobs/fetch", &fetch).await;
        assert_eq!((status, body["error"].as_str()), (StatusCode::BAD_REQUEST, Some("bad_request")), "{url}: {body}");
    }
    assert!(mock.requests().is_empty(), "nothing reached the local server");
}

#[tokio::test]
async fn a_fetched_result_becomes_a_download_link_with_limits_and_an_expiry() {
    let (_server, phone, tokens) = setup(true).await;
    let asset: Vec<u8> = b"%PDF-1.7\n".iter().copied().chain(std::iter::repeat_n(b'x', 5000)).collect();
    let served = asset.clone();
    let host = MockServer::start(move |r| match r.path.as_str() {
        "/asset.pdf" => Reply::new(200, "application/octet-stream", served.clone()),
        "/moved" => Reply::redirect(302, "/asset.pdf"),
        _ => Reply::new(404, "text/plain", "missing"),
    })
    .await;
    let fetch = |path: &str, max_bytes: u64, ttl_secs: u32| {
        json!({"v": 1, "connection_id": tokens.connection_id, "url": host.url(path),
            "headers": [["Authorization", "token gh-secret"]], "name": "report.pdf", "max_bytes": max_bytes,
            "ttl_secs": ttl_secs})
    };
    let (status, got) = phone.post("/blobs/fetch", &fetch("/moved", 10_000, 600)).await;
    assert_eq!(status, StatusCode::OK, "{got}");
    assert_eq!((got["name"].as_str(), got["size"].as_u64()), (Some("report.pdf"), Some(asset.len() as u64)));
    assert_eq!(got["content_type"], "application/pdf", "sniffed, not taken from the server");
    let requests = host.requests();
    assert_eq!(requests.len(), 2);
    assert!(requests.iter().all(|r| r.header("authorization") == Some("token gh-secret")), "same origin keeps headers");
    let url = got["download_url"].as_str().unwrap().to_owned();
    for _ in 0..20 {
        let r = download(&url).await;
        assert_eq!(r.status(), StatusCode::OK);
        assert_eq!(r.bytes().await.unwrap().as_ref(), asset.as_slice());
    }
    assert_eq!(download(&url).await.status(), StatusCode::NOT_FOUND, "at most 20 downloads");
    let (_, info) = phone.get(&format!("/blobs/{}", got["id"].as_str().unwrap())).await;
    assert_eq!((info["purpose"]["kind"].as_str(), info["state"].as_str()), (Some("output"), Some("approved")));

    let (status, body) = phone.post("/blobs/fetch", &fetch("/asset.pdf", 100, 600)).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{body}");
    let (status, body) = phone.post("/blobs/fetch", &fetch("/nope", 10_000, 600)).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY, "{body}");
    assert!(body["message"].as_str().unwrap().contains("404"), "{body}");

    let (_, short) = phone.post("/blobs/fetch", &fetch("/asset.pdf", 10_000, 1)).await;
    let url = short["download_url"].as_str().unwrap().to_owned();
    tokio::time::sleep(Duration::from_millis(2100)).await;
    assert_eq!(download(&url).await.status(), StatusCode::NOT_FOUND, "expired");
}

#[tokio::test]
async fn the_phone_publishes_its_own_result_as_an_output() {
    let (_server, phone, tokens) = setup(false).await;
    let path = format!("/blobs/output?connection_id={}&name=secret%20notes.txt&ttl_secs=60", tokens.connection_id);
    let r = phone.raw(Method::PUT, &path).body("decrypted attachment").send().await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let got: Value = r.json().await.unwrap();
    assert_eq!((got["name"].as_str(), got["size"].as_u64()), (Some("secret notes.txt"), Some(20)));
    let r = download(got["download_url"].as_str().unwrap()).await;
    assert_eq!(
        r.headers()["content-disposition"].to_str().unwrap(),
        "attachment; filename=\"secret_notes.txt\"; filename*=UTF-8''secret%20notes.txt"
    );
    assert_eq!(r.text().await.unwrap(), "decrypted attachment");
    let r = phone.raw(Method::PUT, "/blobs/output?name=x").body("x").send().await.unwrap();
    assert_eq!(r.status(), StatusCode::BAD_REQUEST, "the connection is required");
    let r = phone.raw(Method::PUT, "/blobs/output?connection_id=nope&name=x").body("x").send().await.unwrap();
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
}
