mod common;

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use common::{FakeGoogle, TOKEN_FAILS, TOKEN_NEEDS_CONSENT, gmail_message};
use data_encoding::BASE64URL;
use rewarden_core::CoreError;
use rewarden_core::gmail::{DEFAULT_BASE, GmailClient, Probe};
use rewarden_core::http::client;
use rewarden_proto::gmail::OutgoingEmail;
use serde_json::json;
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

fn gmail(server: &MockServer, google: Arc<FakeGoogle>) -> GmailClient {
    GmailClient::new(client().unwrap(), &server.uri(), google, "", Duration::from_millis(1))
}

fn email() -> OutgoingEmail {
    OutgoingEmail {
        to: vec!["alice@work.com".to_owned()],
        cc: vec![],
        subject: "Hello".to_owned(),
        body: "Body text".to_owned(),
        reply_to_message_id: None,
    }
}

#[test]
fn default_base_is_the_real_gmail_api() {
    assert_eq!(DEFAULT_BASE, "https://gmail.googleapis.com/gmail/v1");
}

#[tokio::test]
async fn list_and_fetch_keep_order_and_skip_vanished_messages() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/users/me/messages"))
        .and(query_param("q", "from:bank"))
        .and(query_param("maxResults", "10"))
        .and(header("authorization", "Bearer google-token-1"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"messages": [{"id": "m3"}, {"id": "m1"}, {"id": "m2"}]})),
        )
        .mount(&server)
        .await;
    for (id, delay) in [("m3", 60), ("m1", 0), ("m2", 30)] {
        Mock::given(method("GET"))
            .and(path(format!("/users/me/messages/{id}")))
            .and(query_param("format", "metadata"))
            .and(query_param("metadataHeaders", "From"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(gmail_message(id, "Bank <a@bank.com>", &format!("Subject {id}"), None))
                    .set_delay(Duration::from_millis(delay)),
            )
            .mount(&server)
            .await;
    }
    Mock::given(method("GET"))
        .and(path("/users/me/messages/gone"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({"error": {"code": 404}})))
        .mount(&server)
        .await;
    let g = gmail(&server, Arc::new(FakeGoogle::new()));
    let ids = g.list("from:bank", 10).await.unwrap();
    assert_eq!(ids, ["m3", "m1", "m2"]);
    let mut with_gone = ids.clone();
    with_gone.insert(1, "gone".to_owned());
    let parsed = g.fetch(&with_gone, false).await.unwrap();
    let order: Vec<&str> = parsed.iter().map(|m| m.summary.id.as_str()).collect();
    assert_eq!(order, ["m3", "m1", "m2"], "order follows the list, vanished message skipped");
    assert_eq!(parsed[0].summary.from, "a@bank.com");
    assert!(parsed[0].body.is_none());
}

#[tokio::test]
async fn full_fetch_returns_bodies() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/users/me/messages/m1"))
        .and(query_param("format", "full"))
        .respond_with(ResponseTemplate::new(200).set_body_json(gmail_message(
            "m1",
            "a@bank.com",
            "S",
            Some("Full body"),
        )))
        .mount(&server)
        .await;
    let g = gmail(&server, Arc::new(FakeGoogle::new()));
    let parsed = g.fetch(&["m1".to_owned()], true).await.unwrap();
    assert_eq!(parsed[0].body.as_deref(), Some("Full body"));
    assert_eq!(parsed[0].clone().into_full().body_text, "Full body");
}

#[tokio::test]
async fn empty_search_and_malformed_ids() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/users/me/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"resultSizeEstimate": 0})))
        .mount(&server)
        .await;
    Mock::given(wiremock::matchers::path_regex(r"^/users/me/messages/.+"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    let g = gmail(&server, Arc::new(FakeGoogle::new()));
    assert!(g.list("nothing", 5).await.unwrap().is_empty());
    for bad in ["", "../profile", "a/b", "a?x=1", "m 1"] {
        let err = g.fetch(&[bad.to_owned()], false).await.unwrap_err();
        assert!(matches!(err, CoreError::Invalid { .. }), "{bad:?}: {err:?}");
    }
}

#[tokio::test]
async fn a_401_gets_one_fresh_token_then_needs_consent() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/users/me/profile"))
        .and(header("authorization", "Bearer google-token-2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"emailAddress": "me@example.com"})))
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/users/me/profile"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;
    let google = Arc::new(FakeGoogle::new());
    let g = gmail(&server, Arc::clone(&google));
    assert_eq!(g.probe().await, Probe::Ready);
    assert_eq!(google.calls.load(Ordering::SeqCst), 2, "the stale token was replaced once");

    let always_401 = MockServer::start().await;
    Mock::given(method("GET")).respond_with(ResponseTemplate::new(401)).mount(&always_401).await;
    let google = Arc::new(FakeGoogle::new());
    let g = gmail(&always_401, Arc::clone(&google));
    assert_eq!(g.list("x", 5).await.unwrap_err(), CoreError::GmailNeedsConsent);
    assert_eq!(google.calls.load(Ordering::SeqCst), 2, "no refresh loop");
    assert_eq!(g.probe().await, Probe::NeedsConsent);
}

#[tokio::test]
async fn google_token_problems_map_to_consent_or_gmail_errors() {
    let server = MockServer::start().await;
    Mock::given(wiremock::matchers::any()).respond_with(ResponseTemplate::new(200)).expect(0).mount(&server).await;
    let google = Arc::new(FakeGoogle::new());
    let g = gmail(&server, Arc::clone(&google));
    google.set_mode(TOKEN_NEEDS_CONSENT);
    assert_eq!(g.list("x", 5).await.unwrap_err(), CoreError::GmailNeedsConsent);
    assert_eq!(g.probe().await, Probe::NeedsConsent);
    google.set_mode(TOKEN_FAILS);
    assert!(matches!(g.list("x", 5).await.unwrap_err(), CoreError::Gmail { .. }));
    assert!(matches!(g.probe().await, Probe::Unavailable(_)));
}

#[tokio::test]
async fn transient_errors_back_off_and_then_succeed_or_give_up() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/users/me/messages"))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(2)
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/users/me/messages"))
        .respond_with(ResponseTemplate::new(429))
        .up_to_n_times(1)
        .with_priority(2)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/users/me/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"messages": [{"id": "m1"}]})))
        .with_priority(3)
        .mount(&server)
        .await;
    let g = gmail(&server, Arc::new(FakeGoogle::new()));
    assert_eq!(g.list("x", 5).await.unwrap(), ["m1"], "503, 503, 429, then 200 within 3 retries");

    let rate = MockServer::start().await;
    let body = json!({"error": {"errors": [{"reason": "rateLimitExceeded"}]}});
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(403).set_body_json(body))
        .expect(4)
        .mount(&rate)
        .await;
    let g = gmail(&rate, Arc::new(FakeGoogle::new()));
    let err = g.list("x", 5).await.unwrap_err();
    assert!(matches!(&err, CoreError::Gmail { reason: message } if message.contains("403")), "{err:?}");

    let denied = MockServer::start().await;
    let body = json!({"error": {"errors": [{"reason": "insufficientPermissions"}]}});
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(403).set_body_json(body))
        .expect(1)
        .mount(&denied)
        .await;
    let g = gmail(&denied, Arc::new(FakeGoogle::new()));
    assert_eq!(g.list("x", 5).await.unwrap_err(), CoreError::GmailNeedsConsent);
}

fn sent_raw(req: &Request) -> String {
    let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap();
    String::from_utf8(BASE64URL.decode(body["raw"].as_str().unwrap().as_bytes()).unwrap()).unwrap()
}

#[tokio::test]
async fn send_posts_a_raw_message_and_threads_replies() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/users/me/messages/send"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "sent1", "threadId": "t9"})))
        .mount(&server)
        .await;
    let g = gmail(&server, Arc::new(FakeGoogle::new()));
    let sent = g.send(&email()).await.unwrap();
    assert_eq!((sent.id.as_str(), sent.thread_id.as_str()), ("sent1", "t9"));
    let requests = server.received_requests().await.unwrap();
    let post = requests.iter().find(|r| r.method.as_str() == "POST").unwrap();
    let raw = sent_raw(post);
    assert!(raw.contains("To: alice@work.com") && raw.contains("Subject: Hello"), "{raw}");
    assert!(!raw.contains("In-Reply-To"));
    assert!(serde_json::from_slice::<serde_json::Value>(&post.body).unwrap().get("threadId").is_none());

    // A reply reads the original's Message-ID/threadId first.
    let reply_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/users/me/messages/orig1"))
        .and(query_param("metadataHeaders", "Message-ID"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "orig1", "threadId": "thread-42",
            "payload": {"headers": [{"name": "Message-ID", "value": "<orig@mail.example.com>"}]}})))
        .mount(&reply_server)
        .await;
    Mock::given(method("POST"))
        .and(path("/users/me/messages/send"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "sent2", "threadId": "thread-42"})))
        .mount(&reply_server)
        .await;
    let g = gmail(&reply_server, Arc::new(FakeGoogle::new()));
    let mut reply = email();
    reply.reply_to_message_id = Some("orig1".to_owned());
    g.send(&reply).await.unwrap();
    let requests = reply_server.received_requests().await.unwrap();
    let post = requests.iter().find(|r| r.method.as_str() == "POST").unwrap();
    let body: serde_json::Value = serde_json::from_slice(&post.body).unwrap();
    assert_eq!(body["threadId"], "thread-42");
    assert!(sent_raw(post).contains("In-Reply-To: <orig@mail.example.com>"));
}

#[tokio::test]
async fn send_refuses_unvalidated_emails() {
    let server = MockServer::start().await;
    Mock::given(wiremock::matchers::any()).respond_with(ResponseTemplate::new(200)).expect(0).mount(&server).await;
    let g = gmail(&server, Arc::new(FakeGoogle::new()));
    let mut bad = email();
    bad.to = vec!["a@b.com\r\nBcc: eve@evil.com".to_owned()];
    assert!(matches!(g.send(&bad).await.unwrap_err(), CoreError::Invalid { .. }));
}
