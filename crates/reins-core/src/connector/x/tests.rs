//! The X connector against a fake X (wiremock): no request leaves the machine.

use std::sync::{Arc, Mutex, Once};

use reins_proto::connector::ConnectorCall;
use serde_json::{Map, Value, json};
use wiremock::matchers::{body_json, body_string_contains, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::http::client;

const ME: &str = "1001";
const ACCESS: &str = "access-AAAA-secret";
const REFRESH: &str = "refresh-RRRR-secret";

struct Fixture {
    server: MockServer,
    x: XApi,
    _dir: tempfile::TempDir,
}

async fn fixture() -> Fixture {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(crate::store::tests::open(dir.path()));
    let x = XApi::new(
        client().unwrap(),
        &server.uri(),
        &format!("{}/i/oauth2/authorize", server.uri()),
        Some("client-123".into()),
        store,
    );
    Fixture {
        server,
        x,
        _dir: dir,
    }
}

/// `@alice` connected, with tokens valid for `valid_for` more seconds.
fn connect(f: &Fixture, valid_for: i64) {
    let tokens = Tokens {
        access_token: ACCESS.into(),
        refresh_token: Some(REFRESH.into()),
        expires_at: unix_now() + valid_for,
        user_id: ME.into(),
        username: "alice".into(),
    };
    f.x.save("alice", &tokens).unwrap();
}

fn call(op: &str, args: Value) -> ConnectorCall {
    let args: Map<String, Value> = serde_json::from_value(args).unwrap();
    ConnectorCall {
        service: X.into(),
        op: op.into(),
        args,
    }
}

fn ok(body: Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(body)
}

/// The post X has as 42, by @bob.
async fn bob_post(f: &Fixture) {
    Mock::given(method("GET"))
        .and(path("/2/tweets/42"))
        .respond_with(ok(json!({
            "data": {"id": "42", "author_id": "2002", "text": "hello world", "created_at": "2026-01-02T03:04:05.000Z"},
            "includes": {"users": [{"id": "2002", "name": "Bob", "username": "bob"}]},
        })))
        .mount(&f.server)
        .await;
}

async fn bob_user(f: &Fixture) {
    Mock::given(method("GET"))
        .and(path("/2/users/by/username/bob"))
        .respond_with(ok(json!({"data": {"id": "2002", "name": "Bob", "username": "bob"}})))
        .mount(&f.server)
        .await;
}

#[test]
fn the_manifest_lists_every_x_action_with_its_risk() {
    let manifest = reins_proto::connector::manifest::manifest(X).unwrap();
    assert_eq!(manifest.auth.kind(), "oauth");
    let tools: Vec<&str> = (manifest.tools)().iter().map(|t| t.tool).collect();
    for tool in [
        "x_post",
        "x_reply",
        "x_thread",
        "x_post_delete",
        "x_like",
        "x_unlike",
        "x_repost",
        "x_unrepost",
        "x_bookmark_add",
        "x_bookmark_remove",
        "x_bookmarks",
        "x_follow",
        "x_unfollow",
        "x_mute",
        "x_unmute",
        "x_block",
        "x_unblock",
        "x_dm_send",
        "x_dm_list",
        "x_home_timeline",
        "x_mentions",
        "x_search",
        "x_user",
    ] {
        assert!(tools.contains(&tool), "{tool} is missing");
    }
    let risk = |tool: &str| reins_proto::connector::manifest::action(tool).unwrap().risk;
    assert_eq!(risk("x_post"), reins_proto::connector::RiskLevel::High);
    assert_eq!(risk("x_dm_send"), reins_proto::connector::RiskLevel::High);
    assert_eq!(risk("x_search"), reins_proto::connector::RiskLevel::Low);
}

#[tokio::test]
async fn sign_in_uses_pkce_and_checks_the_state() {
    let f = fixture().await;
    let url = url::Url::parse(&f.x.begin().unwrap()).unwrap();
    let q: HashMap<String, String> = url.query_pairs().into_owned().collect();
    assert_eq!(url.path(), "/i/oauth2/authorize");
    assert_eq!(q["response_type"], "code");
    assert_eq!(q["client_id"], "client-123");
    assert_eq!(q["redirect_uri"], REDIRECT_URI);
    assert_eq!(q["code_challenge_method"], "S256");
    assert!(q["scope"].split(' ').any(|s| s == "offline.access"));
    assert!(q["scope"].split(' ').any(|s| s == "tweet.write"));
    let state = q["state"].clone();

    // A callback for another sign-in is refused, and the sign-in must start again.
    let wrong = f.x.finish(&format!("{REDIRECT_URI}?state=other&code=c")).await.unwrap_err();
    assert!(wrong.to_string().contains("not for this sign-in"), "{wrong}");
    assert!(f.x.finish(&format!("{REDIRECT_URI}?state={state}&code=c")).await.is_err());

    // The real thing: the code and the verifier are traded for tokens.
    let url = url::Url::parse(&f.x.begin().unwrap()).unwrap();
    let state = url.query_pairs().find(|(k, _)| k == "state").unwrap().1.into_owned();
    Mock::given(method("POST"))
        .and(path("/2/oauth2/token"))
        .and(body_string_contains("grant_type=authorization_code"))
        .and(body_string_contains("code=the-code"))
        .and(body_string_contains("code_verifier="))
        .and(body_string_contains("client_id=client-123"))
        .respond_with(ok(json!({"access_token": ACCESS, "refresh_token": REFRESH, "expires_in": 7200})))
        .expect(1)
        .mount(&f.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/2/users/me"))
        .and(header("authorization", format!("Bearer {ACCESS}").as_str()))
        .respond_with(ok(json!({"data": {"id": ME, "name": "Alice", "username": "Alice"}})))
        .mount(&f.server)
        .await;
    let account = f.x.finish(&format!("{REDIRECT_URI}?state={state}&code=the-code")).await.unwrap();
    assert_eq!(account, "alice");
    assert_eq!(f.x.status("alice").await, GmailStatus::Ready);
    assert_eq!(f.x.load("alice").unwrap().user_id, ME);
}

#[tokio::test]
async fn without_a_client_id_x_is_unavailable() {
    let dir = tempfile::tempdir().unwrap();
    let x = XApi::new(
        client().unwrap(),
        "http://127.0.0.1:9",
        "http://127.0.0.1:9/a",
        None,
        Arc::new(crate::store::tests::open(dir.path())),
    );
    assert!(x.unavailable().unwrap().contains("REINS_X_CLIENT_ID"));
    assert!(matches!(x.status("alice").await, GmailStatus::Unavailable { .. }));
    assert!(x.begin().is_err());
}

#[tokio::test]
async fn tokens_are_refreshed_before_they_run_out_and_after_a_401() {
    let f = fixture().await;
    connect(&f, 10);
    Mock::given(method("POST"))
        .and(path("/2/oauth2/token"))
        .and(body_string_contains("grant_type=refresh_token"))
        .and(body_string_contains(format!("refresh_token={REFRESH}").as_str()))
        .respond_with(ok(json!({"access_token": "fresh-1", "expires_in": 7200})))
        .expect(1)
        .mount(&f.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/2/users/me"))
        .and(header("authorization", "Bearer fresh-1"))
        .respond_with(ok(json!({"data": {"id": ME, "name": "Alice", "username": "alice"}})))
        .mount(&f.server)
        .await;
    let items = f.x.fetch("alice", &call("me", json!({}))).await.unwrap();
    assert_eq!(items[0].title, "@alice (Alice)");
    let kept = f.x.load("alice").unwrap();
    assert_eq!(kept.access_token, "fresh-1");
    assert_eq!(kept.refresh_token.as_deref(), Some(REFRESH), "the old refresh token is kept when none comes back");

    // A token X no longer takes is refreshed once, and the call tried again.
    let f = fixture().await;
    connect(&f, 3600);
    Mock::given(method("GET"))
        .and(path("/2/users/me"))
        .and(header("authorization", format!("Bearer {ACCESS}").as_str()))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({"title": "Unauthorized"})))
        .expect(1)
        .mount(&f.server)
        .await;
    Mock::given(method("POST"))
        .and(path("/2/oauth2/token"))
        .respond_with(ok(json!({"access_token": "fresh-2", "refresh_token": "refresh-2", "expires_in": 7200})))
        .expect(1)
        .mount(&f.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/2/users/me"))
        .and(header("authorization", "Bearer fresh-2"))
        .respond_with(ok(json!({"data": {"id": ME, "name": "Alice", "username": "alice"}})))
        .mount(&f.server)
        .await;
    f.x.fetch("alice", &call("me", json!({}))).await.unwrap();
    assert_eq!(f.x.load("alice").unwrap().refresh_token.as_deref(), Some("refresh-2"));
}

#[tokio::test]
async fn a_refused_refresh_asks_for_the_account_again() {
    let f = fixture().await;
    connect(&f, 0);
    Mock::given(method("POST"))
        .and(path("/2/oauth2/token"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({"error": "invalid_request"})))
        .mount(&f.server)
        .await;
    let err = f.x.fetch("alice", &call("me", json!({}))).await.unwrap_err();
    assert!(matches!(err, CoreError::ServiceNeedsAttention { .. }), "{err:?}");
}

#[tokio::test]
async fn a_used_up_rate_limit_says_when_it_resets() {
    let f = fixture().await;
    connect(&f, 3600);
    let reset = unix_now() + 300;
    Mock::given(method("GET"))
        .and(path("/2/tweets/search/recent"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("x-rate-limit-limit", "300")
                .insert_header("x-rate-limit-remaining", "0")
                .insert_header("x-rate-limit-reset", reset.to_string().as_str())
                .set_body_json(json!({"title": "Too Many Requests"})),
        )
        .mount(&f.server)
        .await;
    let err = f.x.fetch("alice", &call("search", json!({"query": "rust"}))).await.unwrap_err().to_string();
    assert!(err.contains("rate limit for this is used up"), "{err}");
    assert!(err.contains(&text::iso_utc(reset)), "{err}");
    assert!(err.contains("in 5 minutes"), "{err}");

    let mut headers = HeaderMap::new();
    headers.insert("x-rate-limit-reset", "100".parse().unwrap());
    headers.insert("x-user-limit-24hour-remaining", "0".parse().unwrap());
    headers.insert("x-user-limit-24hour-reset", "4000".parse().unwrap());
    let limits = RateLimit::of(&headers);
    assert!(limits.used_up(40).contains("daily limit"), "{}", limits.used_up(40));
}

#[tokio::test]
async fn api_errors_are_explained() {
    let limits = RateLimit::default();
    let refused = answer(StatusCode::FORBIDDEN, &limits, json!({"detail": "You are not permitted"})).unwrap_err();
    assert!(refused.to_string().contains("X refused this (403): You are not permitted"), "{refused}");
    let broke = answer(StatusCode::PAYMENT_REQUIRED, &limits, json!({})).unwrap_err();
    assert!(broke.to_string().contains("no API credits left"), "{broke}");
    let only_errors = answer(StatusCode::OK, &limits, json!({"errors": [{"detail": "Could not find tweet"}]}));
    assert!(only_errors.unwrap_err().to_string().contains("Could not find tweet"));
}

#[tokio::test]
async fn reads_hand_over_posts_profiles_and_sensitive_messages() {
    let f = fixture().await;
    connect(&f, 3600);
    bob_post(&f).await;
    bob_user(&f).await;
    let page = json!({
        "data": [{"id": "7", "author_id": "2002", "text": "first"}, {"id": "8", "author_id": "2002", "text": "second"}],
        "includes": {"users": [{"id": "2002", "name": "Bob", "username": "bob"}]},
    });
    for p in [
        format!("/2/users/{ME}/timelines/reverse_chronological"),
        format!("/2/users/{ME}/mentions"),
        format!("/2/users/{ME}/bookmarks"),
        "/2/tweets/search/recent".to_owned(),
    ] {
        Mock::given(method("GET")).and(path(p.as_str())).respond_with(ok(page.clone())).mount(&f.server).await;
    }
    Mock::given(method("GET"))
        .and(path("/2/dm_conversations/with/2002/dm_events"))
        .respond_with(ok(json!({
            "data": [{"id": "d1", "text": "psst", "sender_id": "2002", "dm_conversation_id": "1001-2002"}],
            "includes": {"users": [{"id": "2002", "name": "Bob", "username": "bob"}]},
        })))
        .mount(&f.server)
        .await;

    let post =
        f.x.fetch("alice", &call("post_get", json!({"post": "https://x.com/bob/status/42?s=20"}))).await.unwrap();
    assert_eq!((post[0].id.as_str(), post[0].body.as_deref()), ("42", Some("hello world")));
    assert_eq!(post[0].extra["url"], "https://x.com/bob/status/42");
    let user = f.x.fetch("alice", &call("user", json!({"user": "@Bob"}))).await.unwrap();
    assert_eq!(user[0].id, "2002");
    for op in ["home_timeline", "mentions", "bookmarks"] {
        let items = f.x.fetch("alice", &call(op, json!({"limit": 1}))).await.unwrap();
        assert_eq!(items.len(), 1, "{op}");
        assert_eq!(items[0].from, "Bob (@bob)");
    }
    let found = f.x.fetch("alice", &call("search", json!({"query": "from:bob"}))).await.unwrap();
    assert_eq!(found.len(), 2);
    let dms = f.x.fetch("alice", &call("dm_list", json!({"user": "bob"}))).await.unwrap();
    assert!(dms[0].sensitive, "direct messages are ticked by the user");
    assert_eq!(dms[0].from, "@bob");
}

#[tokio::test]
async fn posts_replies_and_threads_are_shown_exactly_then_published() {
    let f = fixture().await;
    connect(&f, 3600);
    bob_post(&f).await;

    let post = call("post", json!({"text": "Hello X", "media": ["555"], "quote": "42"}));
    let preview = f.x.preview("alice", &post).await.unwrap();
    assert_eq!(
        preview.lines,
        ["Post from @alice on X", "Quoting @bob's post 42: \"hello world\"", "Media: 1 attached (555)", "Hello X",]
    );
    assert_eq!(preview.resource, "alice");
    Mock::given(method("POST"))
        .and(path("/2/tweets"))
        .and(body_json(json!({"text": "Hello X", "media": {"media_ids": ["555"]}, "quote_tweet_id": "42"})))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"data": {"id": "900", "text": "Hello X"}})))
        .expect(1)
        .mount(&f.server)
        .await;
    assert_eq!(
        f.x.perform("alice", &post).await.unwrap(),
        json!({"id": "900", "url": "https://x.com/alice/status/900"})
    );

    let reply = call("reply", json!({"reply_to": "42", "text": "Hi Bob"}));
    let preview = f.x.preview("alice", &reply).await.unwrap();
    assert_eq!(preview.lines, ["Reply from @alice on X", "In reply to @bob's post 42: \"hello world\"", "Hi Bob"]);
    assert_eq!((preview.resource.as_str(), preview.resource_label.as_str()), ("2002", "@bob"));

    let long = "a".repeat(281);
    let preview = f.x.preview("alice", &call("post", json!({"text": long}))).await.unwrap();
    assert!(preview.lines.iter().any(|l| l.contains("X Premium")));

    let thread = call("thread", json!({"posts": ["one", "two"]}));
    let preview = f.x.preview("alice", &thread).await.unwrap();
    assert_eq!(preview.lines, ["Thread of 2 posts from @alice on X", "1/2: one", "2/2: two"]);
    Mock::given(method("POST"))
        .and(path("/2/tweets"))
        .and(body_json(json!({"text": "one"})))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"data": {"id": "901"}})))
        .expect(1)
        .mount(&f.server)
        .await;
    Mock::given(method("POST"))
        .and(path("/2/tweets"))
        .and(body_json(json!({"text": "two", "reply": {"in_reply_to_tweet_id": "901"}})))
        .respond_with(ResponseTemplate::new(503))
        .mount(&f.server)
        .await;
    let partial = f.x.perform("alice", &thread).await.unwrap_err().to_string();
    assert!(partial.contains("stopped after 1 of its posts"), "{partial}");
    assert!(partial.contains("https://x.com/alice/status/901"), "{partial}");

    assert!(f.x.preview("alice", &call("thread", json!({"posts": ["only"]}))).await.is_err());
    assert!(f.x.preview("alice", &call("post", json!({"text": " "}))).await.is_err());
}

#[tokio::test]
async fn only_your_own_posts_are_deleted_and_each_time_is_asked() {
    let f = fixture().await;
    connect(&f, 3600);
    bob_post(&f).await;
    Mock::given(method("GET"))
        .and(path("/2/tweets/77"))
        .respond_with(ok(json!({
            "data": {"id": "77", "author_id": ME, "text": "oops"},
            "includes": {"users": [{"id": ME, "name": "Alice", "username": "alice"}]},
        })))
        .mount(&f.server)
        .await;
    assert!(f.x.preview("alice", &call("post_delete", json!({"post": "42"}))).await.is_err());
    let delete = call("post_delete", json!({"post": "77"}));
    let preview = f.x.preview("alice", &delete).await.unwrap();
    assert_eq!(preview.lines, ["Delete @alice's post 77 on X for good", "oops"]);
    assert!(preview.once_only);
    Mock::given(method("DELETE"))
        .and(path("/2/tweets/77"))
        .respond_with(ok(json!({"data": {"deleted": true}})))
        .expect(1)
        .mount(&f.server)
        .await;
    assert_eq!(f.x.perform("alice", &delete).await.unwrap(), json!({"deleted": true}));
}

#[tokio::test]
async fn likes_reposts_and_bookmarks_name_the_post() {
    let f = fixture().await;
    connect(&f, 3600);
    bob_post(&f).await;
    for (op, line, verb, p) in [
        ("like", "Like @bob's post 42: \"hello world\", as @alice", "POST", format!("/2/users/{ME}/likes")),
        (
            "unlike",
            "Take back @alice's like of @bob's post 42: \"hello world\"",
            "DELETE",
            format!("/2/users/{ME}/likes/42"),
        ),
        (
            "repost",
            "Repost @bob's post 42: \"hello world\" to @alice's followers",
            "POST",
            format!("/2/users/{ME}/retweets"),
        ),
        (
            "unrepost",
            "Undo @alice's repost of @bob's post 42: \"hello world\"",
            "DELETE",
            format!("/2/users/{ME}/retweets/42"),
        ),
        (
            "bookmark_add",
            "Bookmark @bob's post 42: \"hello world\" (only @alice sees bookmarks)",
            "POST",
            format!("/2/users/{ME}/bookmarks"),
        ),
        (
            "bookmark_remove",
            "Remove @bob's post 42: \"hello world\" from @alice's bookmarks",
            "DELETE",
            format!("/2/users/{ME}/bookmarks/42"),
        ),
    ] {
        let c = call(op, json!({"post": "42"}));
        let preview = f.x.preview("alice", &c).await.unwrap();
        assert_eq!(preview.lines, [line], "{op}");
        assert_eq!(preview.resource, "2002", "{op}");
        let mut mock = Mock::given(method(verb)).and(path(p.as_str()));
        if verb == "POST" {
            mock = mock.and(body_json(json!({"tweet_id": "42"})));
        }
        mock.respond_with(ok(json!({"data": {"done": true}}))).expect(1).mount(&f.server).await;
        assert_eq!(f.x.perform("alice", &c).await.unwrap(), json!({"done": true}), "{op}");
    }
}

#[tokio::test]
async fn follows_mutes_and_blocks_name_the_account() {
    let f = fixture().await;
    connect(&f, 3600);
    bob_user(&f).await;
    for (op, verb, p) in [
        ("follow", "POST", format!("/2/users/{ME}/following")),
        ("unfollow", "DELETE", format!("/2/users/{ME}/following/2002")),
        ("mute", "POST", format!("/2/users/{ME}/muting")),
        ("unmute", "DELETE", format!("/2/users/{ME}/muting/2002")),
        ("block", "POST", format!("/2/users/{ME}/blocking")),
        ("unblock", "DELETE", format!("/2/users/{ME}/blocking/2002")),
    ] {
        let c = call(op, json!({"user": "@bob"}));
        let preview = f.x.preview("alice", &c).await.unwrap();
        assert!(preview.lines[0].contains("@bob (Bob)"), "{op}: {:?}", preview.lines);
        assert_eq!(preview.resource, "2002");
        assert_eq!(preview.once_only, op == "block", "{op}");
        let mut mock = Mock::given(method(verb)).and(path(p.as_str()));
        if verb == "POST" {
            mock = mock.and(body_json(json!({"target_user_id": "2002"})));
        }
        mock.respond_with(ok(json!({"data": {"ok": op}}))).expect(1).mount(&f.server).await;
        assert_eq!(f.x.perform("alice", &c).await.unwrap(), json!({"ok": op}));
    }
}

#[tokio::test]
async fn a_direct_message_names_the_recipient() {
    let f = fixture().await;
    connect(&f, 3600);
    bob_user(&f).await;
    let dm = call("dm_send", json!({"user": "bob", "text": "see you at 6", "media": ["556"]}));
    let preview = f.x.preview("alice", &dm).await.unwrap();
    assert_eq!(
        preview.lines,
        ["Direct message from @alice to @bob (Bob) on X", "Media: 1 attached (556)", "see you at 6"]
    );
    assert_eq!(preview.resource, "2002");
    Mock::given(method("POST"))
        .and(path("/2/dm_conversations/with/2002/messages"))
        .and(body_json(json!({"text": "see you at 6", "attachments": [{"media_id": "556"}]})))
        .respond_with(
            ResponseTemplate::new(201)
                .set_body_json(json!({"data": {"dm_event_id": "e1", "dm_conversation_id": "1001-2002"}})),
        )
        .expect(1)
        .mount(&f.server)
        .await;
    assert_eq!(f.x.perform("alice", &dm).await.unwrap(), json!({"dm_event_id": "e1", "conversation_id": "1001-2002"}));
}

#[tokio::test]
async fn media_is_checked_before_it_is_uploaded() {
    let f = fixture().await;
    connect(&f, 3600);
    let png = BASE64.encode(b"\x89PNG\r\n\x1a\n0000");
    let upload = call("media_upload", json!({"data": png, "media_type": "image/png"}));
    let preview = f.x.preview("alice", &upload).await.unwrap();
    assert_eq!(preview.lines[0], "Upload a PNG image (1 KB) to @alice's X media, for a post");
    Mock::given(method("POST"))
        .and(path("/2/media/upload"))
        .and(body_json(json!({"media": png, "media_category": "tweet_image"})))
        .respond_with(ok(json!({"data": {"id": "555", "media_key": "3_555"}})))
        .expect(1)
        .mount(&f.server)
        .await;
    assert_eq!(f.x.perform("alice", &upload).await.unwrap(), json!({"media_id": "555"}));

    let lie = call("media_upload", json!({"data": png, "media_type": "image/gif"}));
    assert!(f.x.preview("alice", &lie).await.is_err());
    let junk = call("media_upload", json!({"data": BASE64.encode(b"MZ executable"), "media_type": "image/png"}));
    assert!(f.x.preview("alice", &junk).await.is_err());
    let five = call("post", json!({"text": "t", "media": ["1", "2", "3", "4", "5"]}));
    assert!(f.x.preview("alice", &five).await.is_err());
}

#[test]
fn ids_and_names_are_read_strictly() {
    assert_eq!(parse_post_id("https://x.com/bob/status/123/photo/1").as_deref(), Some("123"));
    assert_eq!(parse_post_id(" 456 ").as_deref(), Some("456"));
    assert_eq!(parse_post_id("https://x.com/bob"), None);
    assert_eq!(parse_post_id("12a"), None);
    assert!(matches!(who("@Some_One").unwrap(), Who::Name(n) if n == "some_one"));
    assert!(matches!(who("2002").unwrap(), Who::Id(n) if n == "2002"));
    assert!(who("../etc").is_err());
    assert!(who("@far_too_long_a_username").is_err());
}

#[test]
fn tokens_never_print() {
    let tokens = Tokens {
        access_token: ACCESS.into(),
        refresh_token: Some(REFRESH.into()),
        expires_at: 1,
        user_id: ME.into(),
        username: "alice".into(),
    };
    let printed = format!("{tokens:?}");
    assert!(!printed.contains(ACCESS) && !printed.contains(REFRESH), "{printed}");
}

static LOGS: Mutex<Vec<String>> = Mutex::new(Vec::new());

struct Capture;

impl log::Log for Capture {
    fn enabled(&self, _: &log::Metadata<'_>) -> bool {
        true
    }

    fn log(&self, record: &log::Record<'_>) {
        // Reins' own lines only: the fake server and the HTTP stack log on their own.
        if !record.target().starts_with("reins_core") {
            return;
        }
        LOGS.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(record.args().to_string());
    }

    fn flush(&self) {}
}

#[tokio::test]
async fn no_token_code_or_verifier_reaches_the_logs() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        if log::set_logger(&Capture).is_ok() {
            log::set_max_level(log::LevelFilter::Trace);
        }
    });
    let f = fixture().await;
    let url = url::Url::parse(&f.x.begin().unwrap()).unwrap();
    let state = url.query_pairs().find(|(k, _)| k == "state").unwrap().1.into_owned();
    Mock::given(method("POST"))
        .and(path("/2/oauth2/token"))
        .respond_with(ok(json!({"access_token": ACCESS, "refresh_token": REFRESH, "expires_in": 7200})))
        .mount(&f.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/2/users/me"))
        .respond_with(ok(json!({"data": {"id": ME, "name": "Alice", "username": "alice"}})))
        .mount(&f.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/2/tweets/search/recent"))
        .and(query_param("query", "rust"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&f.server)
        .await;
    f.x.finish(&format!("{REDIRECT_URI}?state={state}&code=the-secret-code")).await.unwrap();
    f.x.fetch("alice", &call("search", json!({"query": "rust"}))).await.unwrap_err();

    let logs = LOGS.lock().unwrap_or_else(std::sync::PoisonError::into_inner).join("\n");
    assert!(logs.contains("x: GET /2/users/me answered 200"), "the X requests are logged: {logs}");
    for secret in [ACCESS, REFRESH, "the-secret-code", state.as_str(), "client-123"] {
        assert!(!logs.contains(secret), "{secret} was logged");
    }
}
