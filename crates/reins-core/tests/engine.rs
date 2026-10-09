//! End-to-end behaviour of the core against fake Reins and Gmail servers.

mod common;

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::task::{Context, Waker};
use std::time::Duration;

use common::{FakeGoogle, FakeKeys, RecordingNotifier, TOKEN_NEEDS_CONSENT, gmail_message};
use reins_core::{
    ApprovalChoice, ApprovalKind, CoreConfig, CoreError, GmailStatus, GoogleTokenProvider, GrantScopeChoice, Notifier,
    ReinsCore, StandingGrant, StartingPolicy,
};
use serde_json::{Value, json};
use wiremock::matchers::{method, path, path_regex, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const EMAIL: &str = "me@example.com";
const GMAIL: &str = "me@gmail.com";

struct Env {
    server: MockServer,
    gmail: MockServer,
    core: Arc<ReinsCore>,
    google: Arc<FakeGoogle>,
    notifier: Arc<RecordingNotifier>,
    dir: tempfile::TempDir,
}

fn scope() -> GrantScopeChoice {
    GrantScopeChoice {
        all_mail: false,
        selected_messages_only: false,
        sender_addresses: vec![],
        sender_domains: vec![],
        subject_pattern: None,
        recipient_addresses: vec![],
        recipient_domains: vec![],
        resources: vec![],
        classes: vec![],
    }
}

fn choice(selected: &[&str], standing: Option<StandingGrant>) -> ApprovalChoice {
    ApprovalChoice {
        selected_message_ids: selected.iter().map(|s| (*s).to_owned()).collect(),
        standing,
    }
}

fn open_core(
    dir: &std::path::Path,
    gmail: &MockServer,
    google: &Arc<FakeGoogle>,
    notifier: &Arc<RecordingNotifier>,
) -> Arc<ReinsCore> {
    let cfg = CoreConfig {
        gmail_base: gmail.uri(),
        backoff_base: Duration::from_millis(1),
        ..CoreConfig::default()
    };
    let google: Arc<dyn GoogleTokenProvider> = Arc::<FakeGoogle>::clone(google);
    let notifier: Arc<dyn Notifier> = Arc::<RecordingNotifier>::clone(notifier);
    ReinsCore::with_config(dir.to_str().unwrap(), &FakeKeys, google, notifier, cfg).unwrap()
}

async fn mount_identity(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/identity/accounts/prelogin"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"kdf": 0, "kdfIterations": 5000})))
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path("/identity/connect/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": common::account_token(), "refresh_token": "REFRESH", "expires_in": 7200})))
        .mount(server)
        .await;
}

async fn env() -> Env {
    let server = MockServer::start().await;
    let gmail = MockServer::start().await;
    mount_identity(&server).await;
    let dir = tempfile::tempdir().unwrap();
    let google = Arc::new(FakeGoogle::new());
    let notifier = Arc::new(RecordingNotifier::default());
    let core = open_core(dir.path(), &gmail, &google, &notifier);
    common::mount_account_vault(&server, EMAIL, "hunter2").await;
    core.login(server.uri(), EMAIL.to_owned(), "hunter2".to_owned(), None).await.unwrap();
    // One Gmail account is connected, as on a phone that has been set up.
    Mock::given(method("GET"))
        .and(path("/users/me/profile"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"emailAddress": GMAIL})))
        .with_priority(200)
        .mount(&gmail)
        .await;
    core.add_account(GMAIL.to_owned()).await.unwrap();
    Env {
        server,
        gmail,
        core,
        google,
        notifier,
        dir,
    }
}

fn search_request(id: &str, conn: &str, label: &str, query: &str) -> Value {
    json!({"v": 1, "id": id, "connection_id": conn, "connection_label": label, "created_at": 100,
           "call": {"tool": "gmail_search", "query": query, "max_results": 10}})
}

fn read_request(id: &str, conn: &str, ids: &[&str]) -> Value {
    json!({"v": 1, "id": id, "connection_id": conn, "connection_label": "Claude", "created_at": 100,
           "call": {"tool": "gmail_read", "message_ids": ids}})
}

fn send_request(id: &str, conn: &str, to: &str) -> Value {
    json!({"v": 1, "id": id, "connection_id": conn, "connection_label": "ChatGPT", "created_at": 100,
           "call": {"tool": "gmail_send", "email": {"to": [to], "subject": "Hello", "body": "Body"}}})
}

/// The server hands out `requests` on the next long-poll (and accepts their answers).
async fn serve_pending(env: &Env, requests: &[Value], pairings: &[Value]) {
    Mock::given(method("GET"))
        .and(path("/reins/api/pending"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"requests": requests, "pairings": pairings})))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&env.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/reins/api/pending"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"requests": [], "pairings": []})))
        .mount(&env.server)
        .await;
    Mock::given(method("POST"))
        .and(path_regex(r"^/reins/api/(requests|pairings)/[^/]+/response$"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&env.server)
        .await;
}

async fn serve_gmail_search(env: &Env, messages: &[(&str, &str)]) {
    let ids: Vec<Value> = messages.iter().map(|(id, _)| json!({"id": id})).collect();
    Mock::given(method("GET"))
        .and(path("/users/me/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"messages": ids})))
        .mount(&env.gmail)
        .await;
    for (id, from) in messages {
        Mock::given(method("GET"))
            .and(path(format!("/users/me/messages/{id}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(gmail_message(
                id,
                from,
                &format!("Subject {id}"),
                Some("Full text"),
            )))
            .mount(&env.gmail)
            .await;
    }
}

/// Bodies POSTed to the request-response endpoint, in order: (request id, JSON).
async fn answers(env: &Env) -> Vec<(String, Value)> {
    env.server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| {
            r.method.as_str() == "POST" && r.url.path().ends_with("/response") && r.url.path().contains("/requests/")
        })
        .map(|r| {
            let id = r.url.path().split('/').nth_back(1).unwrap().to_owned();
            (id, serde_json::from_slice(&r.body).unwrap())
        })
        .collect()
}

fn bank_grant() -> StandingGrant {
    let mut s = scope();
    s.sender_domains = vec!["bank.com".to_owned()];
    StandingGrant {
        duration_secs: None,
        max_uses: None,
        scope: s,
    }
}

#[tokio::test]
async fn an_uncovered_search_is_parked_then_approved_with_a_standing_grant() {
    let env = env().await;
    serve_gmail_search(&env, &[("m1", "Bank <alerts@bank.com>")]).await;
    serve_pending(&env, &[search_request("r1", "c1", "Chat\u{202E}GPT", "from:bank")], &[]).await;

    let items = env.core.sync(0).await.unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].id, "r1");
    assert_eq!(items[0].title, "ChatGPT wants to search your Gmail", "bidi controls removed from the label");
    assert_eq!(env.notifier.pending.lock().unwrap().len(), 1);
    assert!(answers(&env).await.is_empty(), "nothing is released before the user decides");

    let view = env.core.approval_view("r1".to_owned()).await.unwrap();
    assert_eq!((view.kind, view.query.as_deref(), view.messages.len()), (ApprovalKind::Search, Some("from:bank"), 1));
    assert_eq!((view.messages[0].from.as_str(), view.messages[0].covered_by_grant), ("Bank <alerts@bank.com>", false));

    env.core.approve("r1".to_owned(), choice(&["m1"], Some(bank_grant()))).await.unwrap();
    let sent = answers(&env).await;
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].1["outcome"], "result");
    assert_eq!(sent[0].1["result"]["kind"], "search");
    assert_eq!(sent[0].1["result"]["messages"][0]["from"], "alerts@bank.com");
    assert_eq!(env.core.pending().await.unwrap().len(), 0);
    assert_eq!(env.notifier.resolved.lock().unwrap().as_slice(), ["r1"]);
    let grants = env.core.grants().await.unwrap();
    assert_eq!(grants.len(), 1);
    assert_eq!(grants[0].connection_label, "ChatGPT");
    assert_eq!(grants[0].summary, "Read emails from @bank.com");
    let activity = env.core.activity(10).await.unwrap();
    assert_eq!((activity[0].outcome.as_str(), activity[0].action.as_str()), ("released", "search"));
    assert!(activity[0].detail.starts_with("approved: 1 message"));

    // The next search by the same connection is covered: answered without asking.
    serve_pending(&env, &[search_request("r2", "c1", "ChatGPT", "from:bank")], &[]).await;
    env.core.sync(0).await.unwrap();
    let sent = answers(&env).await;
    assert_eq!(sent.len(), 2, "answered automatically");
    assert_eq!(sent[1].0, "r2");
    assert_eq!(env.core.pending().await.unwrap().len(), 0);
    assert_eq!(env.notifier.pending.lock().unwrap().len(), 1, "no second prompt");
    // A different connection is not covered by that grant.
    serve_pending(&env, &[search_request("r3", "other", "Claude", "from:bank")], &[]).await;
    assert_eq!(env.core.sync(0).await.unwrap().len(), 1);
}

#[tokio::test]
async fn an_all_mail_grant_answers_any_later_search_without_asking_until_it_expires() {
    let env = env().await;
    serve_gmail_search(&env, &[("m1", "Bank <alerts@bank.com>"), ("m2", "Friend <pal@example.org>")]).await;
    serve_pending(&env, &[search_request("r1", "c1", "Claude", "in:inbox")], &[]).await;
    assert_eq!(env.core.sync(0).await.unwrap().len(), 1);

    // One tap: release both messages and allow this connection to read anything for an hour.
    let mut all = scope();
    all.all_mail = true;
    let hour = StandingGrant {
        duration_secs: Some(3600),
        max_uses: None,
        scope: all,
    };
    env.core.approve("r1".to_owned(), choice(&["m1", "m2"], Some(hour))).await.unwrap();
    let grants = env.core.grants().await.unwrap();
    assert_eq!(grants.len(), 1);
    assert_eq!(grants[0].summary, "Read any email");
    assert!(grants[0].expires_at.is_some());

    // A later search with a completely different query is answered on the phone, with no prompt.
    serve_pending(&env, &[search_request("r2", "c1", "Claude", "from:someone-else")], &[]).await;
    assert!(env.core.sync(0).await.unwrap().is_empty(), "nothing waits for the user");
    let sent = answers(&env).await;
    assert_eq!((sent.len(), sent[1].0.as_str()), (2, "r2"));
    assert_eq!(sent[1].1["result"]["messages"].as_array().unwrap().len(), 2);

    // Another connection is not covered.
    serve_pending(&env, &[search_request("r3", "other", "ChatGPT", "in:inbox")], &[]).await;
    assert_eq!(env.core.sync(0).await.unwrap().len(), 1);
}

#[tokio::test]
async fn an_all_mail_grant_without_a_time_limit_is_refused_and_nothing_is_released() {
    let env = env().await;
    serve_gmail_search(&env, &[("m1", "a@bank.com")]).await;
    serve_pending(&env, &[search_request("r1", "c1", "Claude", "in:inbox")], &[]).await;
    env.core.sync(0).await.unwrap();
    let mut all = scope();
    all.all_mail = true;
    let forever = StandingGrant {
        duration_secs: None,
        max_uses: None,
        scope: all,
    };
    assert!(env.core.approve("r1".to_owned(), choice(&["m1"], Some(forever))).await.is_err());
    assert!(answers(&env).await.is_empty(), "the refusal happens before anything is released");
    assert_eq!(env.core.pending().await.unwrap().len(), 1, "the request is still waiting");
}

fn grant_request(id: &str, conn: &str, label: &str, grant: &Value) -> Value {
    json!({"v": 1, "id": id, "connection_id": conn, "connection_label": label, "created_at": 100,
           "call": {"tool": "request_grant", "grant": grant}})
}

fn bank_ask(duration: u64) -> Value {
    json!({"action": "read", "duration_secs": duration, "reason": "Summarise the bank statements",
           "from": ["@bank.com"], "subject_contains": null, "recipients": []})
}

async fn serve_connections(env: &Env) {
    Mock::given(method("GET"))
        .and(path("/reins/api/connections"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"connections": [
            {"id": "c1", "label": "Claude", "client_name": "Claude", "client_host": "claude.ai",
             "created_at": 1, "last_used_at": null}]})))
        .mount(&env.server)
        .await;
}

#[tokio::test]
async fn an_ai_can_ask_for_a_narrow_permission_and_the_user_can_only_shorten_it() {
    let env = env().await;
    serve_gmail_search(&env, &[("m1", "Bank <alerts@bank.com>")]).await;
    serve_pending(&env, &[grant_request("g-r1", "c1", "Claude", &bank_ask(3600))], &[]).await;
    let items = env.core.sync(0).await.unwrap();
    assert_eq!((items.len(), items[0].action.as_str(), items[0].count), (1, "grant", 1));

    let view = env.core.approval_view("g-r1".to_owned()).await.unwrap();
    assert_eq!(view.kind, ApprovalKind::Grant);
    let ask = view.grant.expect("the request is spelled out");
    assert_eq!((ask.action.as_str(), ask.breadth.as_str(), ask.duration_secs), ("read", "broad", 3600));
    assert_eq!(ask.lines, ["From @bank.com"]);
    assert_eq!(ask.reason, "Summarise the bank statements");

    // The user allows it, but for ten minutes only (asking for a longer time is impossible).
    let shorter = StandingGrant {
        duration_secs: Some(600),
        max_uses: None,
        scope: scope(),
    };
    env.core.approve("g-r1".to_owned(), choice(&[], Some(shorter))).await.unwrap();
    let sent = answers(&env).await;
    assert_eq!(sent[0].1["result"]["kind"], "granted");
    let grants = env.core.grants().await.unwrap();
    assert_eq!((grants.len(), grants[0].origin.as_str(), grants[0].active), (1, "ai_request", true));
    let window = grants[0].expires_at.unwrap() - grants[0].created_at;
    assert_eq!(window, 600);

    // The permission now answers a matching search without a prompt.
    serve_pending(&env, &[search_request("r2", "c1", "Claude", "from:bank")], &[]).await;
    assert_eq!(env.core.sync(0).await.unwrap().len(), 0);
    assert_eq!(answers(&env).await.len(), 2);
    let activity = env.core.activity(10).await.unwrap();
    assert!(activity.iter().any(|a| a.action == "grant" && a.outcome == "granted"));
}

#[tokio::test]
async fn a_refused_permission_request_creates_nothing() {
    let env = env().await;
    serve_pending(&env, &[grant_request("g-r1", "c1", "Claude", &bank_ask(3600))], &[]).await;
    env.core.sync(0).await.unwrap();
    env.core.deny("g-r1".to_owned()).await.unwrap();
    assert_eq!(answers(&env).await[0].1["outcome"], "denied");
    assert_eq!(env.core.grants().await.unwrap().len(), 0);
}

#[tokio::test]
async fn approving_after_the_ai_stopped_waiting_leaves_a_one_time_retry_pass() {
    let env = env().await;
    serve_gmail_search(&env, &[("m1", "Bank <alerts@bank.com>")]).await;
    let mut late = search_request("r1", "c1", "Claude", "from:bank");
    late["wait_until"] = json!(101); // long past: the AI gave up waiting
    serve_pending(&env, &[late], &[]).await;
    env.core.sync(0).await.unwrap();
    env.core.approve("r1".to_owned(), choice(&["m1"], None)).await.unwrap();

    let grants = env.core.grants().await.unwrap();
    assert_eq!((grants.len(), grants[0].origin.as_str(), grants[0].max_uses), (1, "retry", Some(1)));
    let activity = env.core.activity(5).await.unwrap();
    assert!(activity[0].info.note.as_deref().unwrap().contains("stopped waiting"));

    // The AI asks again: covered, once. The third time it has to ask.
    serve_pending(&env, &[search_request("r2", "c1", "Claude", "from:bank")], &[]).await;
    assert!(env.core.sync(0).await.unwrap().is_empty(), "the pass answers the retry");
    serve_pending(&env, &[search_request("r3", "c1", "Claude", "from:bank")], &[]).await;
    assert_eq!(env.core.sync(0).await.unwrap().len(), 1, "the pass is spent");
}

#[tokio::test]
async fn a_send_approved_late_is_sent_once_and_no_pass_is_created() {
    let env = env().await;
    Mock::given(method("POST"))
        .and(path("/users/me/messages/send"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "s1", "threadId": "t1"})))
        .mount(&env.gmail)
        .await;
    let mut late = send_request("r1", "c1", "boss@work.com");
    late["wait_until"] = json!(101);
    serve_pending(&env, &[late], &[]).await;
    env.core.sync(0).await.unwrap();
    env.core.approve("r1".to_owned(), choice(&[], None)).await.unwrap();
    assert!(env.core.grants().await.unwrap().is_empty(), "repeating a send would only duplicate it");
    assert_eq!(answers(&env).await[0].1["result"]["kind"], "sent");
}

#[tokio::test]
async fn a_permission_can_be_created_ahead_of_time_and_used_once() {
    let env = env().await;
    serve_connections(&env).await;
    serve_gmail_search(&env, &[("m1", "Bank <alerts@bank.com>")]).await;
    let mut s = scope();
    s.sender_domains = vec!["bank.com".to_owned()];
    let one_time = StandingGrant {
        duration_secs: None,
        max_uses: Some(1),
        scope: s,
    };
    env.core.create_grant("c1".to_owned(), GMAIL.to_owned(), ApprovalKind::Read, one_time).await.unwrap();
    let grants = env.core.grants().await.unwrap();
    assert_eq!((grants[0].origin.as_str(), grants[0].connection_label.as_str(), grants[0].uses), ("user", "Claude", 0));

    serve_pending(&env, &[search_request("r1", "c1", "Claude", "from:bank")], &[]).await;
    assert_eq!(env.core.sync(0).await.unwrap().len(), 0);
    let after = env.core.grants().await.unwrap();
    assert_eq!((after[0].uses, after[0].active), (1, false), "used up");
    assert!(after[0].last_used_at.is_some());

    // Unknown connections and sending "to anyone" are refused.
    assert!(
        env.core.create_grant("nope".to_owned(), GMAIL.to_owned(), ApprovalKind::Read, bank_grant()).await.is_err()
    );
    assert!(env.core.create_grant("c1".to_owned(), GMAIL.to_owned(), ApprovalKind::Grant, bank_grant()).await.is_err());
}

#[tokio::test]
async fn items_and_activity_carry_the_connector_account_counts_and_details() {
    let env = env().await;
    Mock::given(method("GET"))
        .and(path("/users/me/profile"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"emailAddress": "Me@Gmail.com"})))
        .mount(&env.gmail)
        .await;
    serve_gmail_search(&env, &[("m1", "Bank <alerts@bank.com>"), ("m2", "Pal <pal@example.org>")]).await;
    serve_pending(&env, &[search_request("r1", "c1", "Claude", "in:inbox")], &[]).await;
    let items = env.core.sync(0).await.unwrap();
    assert_eq!(
        (items[0].service.as_str(), items[0].account.as_deref(), items[0].count, items[0].connection_id.as_str()),
        ("gmail", Some("me@gmail.com"), 2, "c1")
    );
    env.core.approve("r1".to_owned(), choice(&["m1"], None)).await.unwrap();
    let entry = &env.core.activity(5).await.unwrap()[0];
    assert_eq!((entry.action.as_str(), entry.count, entry.account.as_deref()), ("search", 1, Some("me@gmail.com")));
    assert_eq!(entry.info.query.as_deref(), Some("in:inbox"));
    assert_eq!(entry.info.messages.len(), 1);
    assert_eq!(entry.info.messages[0].subject, "Subject m1");

    // A send keeps the whole email for the details screen.
    Mock::given(method("POST"))
        .and(path("/users/me/messages/send"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "s1", "threadId": "t1"})))
        .mount(&env.gmail)
        .await;
    serve_pending(&env, &[send_request("r2", "c1", "boss@work.com")], &[]).await;
    env.core.sync(0).await.unwrap();
    env.core.approve("r2".to_owned(), choice(&[], None)).await.unwrap();
    let sent = &env.core.activity(5).await.unwrap()[0];
    let email = sent.info.email.as_ref().expect("the email is kept");
    assert_eq!(
        (email.to.as_slice(), email.subject.as_str(), email.body.as_str()),
        (["boss@work.com".to_owned()].as_slice(), "Hello", "Body")
    );
    assert!(env.core.activity(5).await.unwrap()[0].id > env.core.activity(5).await.unwrap()[1].id);
}

#[tokio::test]
async fn reading_one_email_is_counted_as_one_everywhere() {
    let env = env().await;
    serve_gmail_search(&env, &[("m1", "Bank <alerts@bank.com>")]).await;
    serve_pending(&env, &[read_request("r1", "c1", &["m1"])], &[]).await;
    let items = env.core.sync(0).await.unwrap();
    assert_eq!((items[0].action.as_str(), items[0].count), ("read", 1));
    let view = env.core.approval_view("r1".to_owned()).await.unwrap();
    assert_eq!((view.messages.len(), view.count), (1, 1));
    env.core.approve("r1".to_owned(), choice(&["m1"], None)).await.unwrap();
    let entry = &env.core.activity(5).await.unwrap()[0];
    assert_eq!((entry.action.as_str(), entry.outcome.as_str(), entry.count), ("read", "released", 1));
}

#[tokio::test]
async fn reading_an_email_that_does_not_exist_is_an_error_that_still_counts_what_was_asked() {
    let env = env().await;
    Mock::given(method("GET"))
        .and(path("/users/me/messages/gone"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&env.gmail)
        .await;
    serve_pending(&env, &[read_request("r1", "c1", &["gone"])], &[]).await;
    assert!(env.core.sync(0).await.unwrap().is_empty(), "nothing to approve");
    let sent = answers(&env).await;
    assert_eq!(sent[0].1["outcome"], "error");
    assert!(sent[0].1["message"].as_str().unwrap().contains("not found"));
    let entry = &env.core.activity(5).await.unwrap()[0];
    assert_eq!((entry.action.as_str(), entry.outcome.as_str(), entry.count), ("read", "error", 1));
}

#[tokio::test]
async fn a_connection_icon_choice_is_remembered_and_forgotten_with_the_connection() {
    let env = env().await;
    serve_connections(&env).await;
    assert_eq!(env.core.connections().await.unwrap()[0].icon, None);
    env.core.set_connection_icon("c1".to_owned(), Some("claude".to_owned())).await.unwrap();
    assert_eq!(env.core.connections().await.unwrap()[0].icon.as_deref(), Some("claude"));
    env.core.set_connection_icon("c1".to_owned(), None).await.unwrap();
    assert_eq!(env.core.connections().await.unwrap()[0].icon, None);
}

#[tokio::test]
async fn covered_messages_are_always_released_and_the_rest_needs_a_pick() {
    let env = env().await;
    serve_gmail_search(&env, &[("m1", "a@bank.com"), ("m2", "eve@evil.com")]).await;
    env.core.approve("nope".to_owned(), choice(&[], None)).await.unwrap_err();
    // Create the bank grant through an earlier approval.
    serve_pending(&env, &[search_request("r0", "c1", "AI", "x")], &[]).await;
    env.core.sync(0).await.unwrap();
    env.core.approve("r0".to_owned(), choice(&["m1", "m2"], Some(bank_grant()))).await.unwrap();

    serve_pending(&env, &[search_request("r1", "c1", "AI", "x")], &[]).await;
    env.core.sync(0).await.unwrap();
    let view = env.core.approval_view("r1".to_owned()).await.unwrap();
    let covered: Vec<(&str, bool)> = view.messages.iter().map(|m| (m.id.as_str(), m.covered_by_grant)).collect();
    assert_eq!(covered, [("m1", true), ("m2", false)]);
    // Picking nothing still releases the covered message.
    env.core.approve("r1".to_owned(), choice(&[], None)).await.unwrap();
    let sent = answers(&env).await;
    let released: Vec<&str> =
        sent[1].1["result"]["messages"].as_array().unwrap().iter().map(|m| m["id"].as_str().unwrap()).collect();
    assert_eq!(released, ["m1"]);
    // Selecting a message that was never shown is refused.
    serve_pending(&env, &[search_request("r2", "c1", "AI", "x")], &[]).await;
    env.core.sync(0).await.unwrap();
    let err = env.core.approve("r2".to_owned(), choice(&["zzz"], None)).await.unwrap_err();
    assert!(matches!(err, CoreError::Invalid { .. }));
    assert_eq!(env.core.pending().await.unwrap().len(), 1, "still waiting for a valid decision");
}

#[tokio::test]
async fn deny_answers_denied_and_is_logged() {
    let env = env().await;
    serve_gmail_search(&env, &[("m1", "a@x.com")]).await;
    serve_pending(&env, &[search_request("r1", "c1", "AI", "x")], &[]).await;
    env.core.sync(0).await.unwrap();
    env.core.deny("r1".to_owned()).await.unwrap();
    assert_eq!(answers(&env).await[0].1, json!({"v": 1, "outcome": "denied", "reason": null}));
    assert_eq!(env.core.activity(5).await.unwrap()[0].outcome, "denied");
    assert_eq!(env.core.deny("r1".to_owned()).await.unwrap_err(), CoreError::NotFound, "cannot decide twice");
    assert_eq!(env.core.grants().await.unwrap().len(), 0);
}

#[tokio::test]
async fn a_single_use_grant_is_spent_once_even_when_requests_race() {
    let env = env().await;
    serve_gmail_search(&env, &[("m1", "a@bank.com")]).await;
    // One approval creates a grant with a single use, consumed by a later request.
    serve_pending(&env, &[search_request("r0", "c1", "AI", "x")], &[]).await;
    env.core.sync(0).await.unwrap();
    let mut single = bank_grant();
    single.max_uses = Some(1);
    env.core.approve("r0".to_owned(), choice(&["m1"], Some(single))).await.unwrap();
    assert_eq!(env.core.grants().await.unwrap()[0].uses, 0, "the approval itself spends no use");

    let requests: Vec<Value> = (1..=6).map(|i| search_request(&format!("race{i}"), "c1", "AI", "x")).collect();
    serve_pending(&env, &requests, &[]).await;
    let parked = env.core.sync(0).await.unwrap();
    assert_eq!(parked.len(), 5, "exactly one of six was answered automatically");
    assert_eq!(answers(&env).await.len(), 2, "the approval's answer plus one automatic answer");
    let grants = env.core.grants().await.unwrap();
    assert_eq!(grants[0].uses, 1);
}

#[tokio::test]
async fn the_same_request_by_push_and_by_poll_is_processed_once() {
    let env = env().await;
    serve_gmail_search(&env, &[("m1", "a@bank.com")]).await;
    let request = search_request("dup1", "c1", "AI", "x");
    Mock::given(method("GET"))
        .and(path("/reins/api/requests/dup1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(request.clone()))
        .mount(&env.server)
        .await;
    serve_pending(&env, &[request], &[]).await;
    let (a, b) = tokio::join!(env.core.handle_push("req".to_owned(), "dup1".to_owned()), env.core.sync(0));
    a.unwrap();
    b.unwrap();
    env.core.handle_push("req".to_owned(), "dup1".to_owned()).await.unwrap();
    assert_eq!(env.notifier.pending.lock().unwrap().len(), 1, "one prompt");
    assert_eq!(env.core.pending().await.unwrap().len(), 1);
    // After it is answered, a late duplicate does nothing either.
    env.core.deny("dup1".to_owned()).await.unwrap();
    env.core.handle_push("req".to_owned(), "dup1".to_owned()).await.unwrap();
    assert_eq!(answers(&env).await.len(), 1);
    assert_eq!(env.core.pending().await.unwrap().len(), 0);
}

#[tokio::test]
async fn cancelling_sync_after_the_server_delivered_still_parks_the_item() {
    let env = env().await;
    serve_gmail_search(&env, &[("m1", "a@bank.com")]).await;
    serve_pending(&env, &[search_request("late1", "c1", "AI", "x")], &[]).await;
    let mut fut = Box::pin(env.core.sync(0));
    let mut cx = Context::from_waker(Waker::noop());
    assert!(fut.as_mut().poll(&mut cx).is_pending());
    drop(fut); // the Kotlin coroutine is cancelled
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(env.core.pending().await.unwrap().len(), 1, "work continued after the caller left");
    assert_eq!(env.notifier.pending.lock().unwrap().len(), 1);
}

fn send_grant(domain: &str) -> StandingGrant {
    let mut s = scope();
    s.recipient_domains = vec![domain.to_owned()];
    StandingGrant {
        duration_secs: Some(3600),
        max_uses: Some(1),
        scope: s,
    }
}

#[tokio::test]
async fn sending_needs_approval_then_a_standing_grant_and_failures_refund() {
    let env = env().await;
    Mock::given(method("POST"))
        .and(path("/users/me/messages/send"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "sent1", "threadId": "t1"})))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&env.gmail)
        .await;
    Mock::given(method("POST"))
        .and(path("/users/me/messages/send"))
        .respond_with(ResponseTemplate::new(400))
        .mount(&env.gmail)
        .await;
    serve_pending(&env, &[send_request("s1", "c1", "boss@work.com")], &[]).await;
    env.core.sync(0).await.unwrap();
    let view = env.core.approval_view("s1".to_owned()).await.unwrap();
    let email = view.email.unwrap();
    assert_eq!(
        (view.kind, email.to, email.subject.as_str(), email.body.as_str()),
        (ApprovalKind::Send, vec!["boss@work.com".to_owned()], "Hello", "Body")
    );

    env.core.approve("s1".to_owned(), choice(&[], Some(send_grant("work.com")))).await.unwrap();
    let sent = answers(&env).await;
    assert_eq!((sent[0].1["outcome"].as_str(), sent[0].1["result"]["kind"].as_str()), (Some("result"), Some("sent")));
    let grant = &env.core.grants().await.unwrap()[0];
    assert_eq!((grant.action.as_str(), grant.uses, grant.max_uses), ("send", 0, Some(1)));

    // Covered by the grant, but Gmail now fails: the AI gets an error and the use is refunded.
    serve_pending(&env, &[send_request("s2", "c1", "peer@work.com")], &[]).await;
    env.core.sync(0).await.unwrap();
    let sent = answers(&env).await;
    assert_eq!(sent[1].0, "s2");
    assert_eq!(sent[1].1["outcome"], "error");
    assert!(sent[1].1["message"].as_str().unwrap().starts_with("Gmail could not complete the request"));
    assert_eq!(env.core.grants().await.unwrap()[0].uses, 0, "nothing was sent, so the use is back");
    // Outside the granted domain it still needs approval.
    serve_pending(&env, &[send_request("s3", "c1", "x@evil.com")], &[]).await;
    assert_eq!(env.core.sync(0).await.unwrap().len(), 1);
}

#[tokio::test]
async fn a_disconnected_gmail_is_explained_to_the_ai() {
    let env = env().await;
    env.google.set_mode(TOKEN_NEEDS_CONSENT);
    serve_pending(&env, &[search_request("g1", "c1", "AI", "x")], &[]).await;
    assert!(env.core.sync(0).await.unwrap().is_empty(), "nothing to approve");
    let sent = answers(&env).await;
    assert_eq!(sent[0].1["outcome"], "error");
    assert!(sent[0].1["message"].as_str().unwrap().contains("connect Gmail"));
    assert_eq!(env.core.activity(1).await.unwrap()[0].outcome, "error");
    assert_eq!(env.core.gmail_status().await, GmailStatus::NeedsConsent);
    env.google.set_mode(0);
    Mock::given(method("GET"))
        .and(path("/users/me/profile"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&env.gmail)
        .await;
    assert_eq!(env.core.gmail_status().await, GmailStatus::Ready);
    assert!(env.google.calls.load(Ordering::SeqCst) >= 2);
}

#[tokio::test]
async fn malformed_and_hostile_requests_are_rejected_not_executed() {
    let env = env().await;
    Mock::given(wiremock::matchers::any())
        .and(path_regex("^/users/"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&env.gmail)
        .await;
    let bad_version = json!({"v": 2, "id": "b1", "connection_id": "c1", "connection_label": "AI", "created_at": 1,
        "call": {"tool": "gmail_search", "query": "x", "max_results": 10}});
    let injected = json!({"v": 1, "id": "b2", "connection_id": "c1", "connection_label": "AI", "created_at": 1,
        "call": {"tool": "gmail_send", "email": {"to": ["a@b.com\r\nBcc: eve@evil.com"], "subject": "s", "body": "b"}}});
    let too_many = json!({"v": 1, "id": "b3", "connection_id": "c1", "connection_label": "AI", "created_at": 1,
        "call": {"tool": "gmail_search", "query": "x", "max_results": 5000}});
    serve_pending(&env, &[bad_version, injected, too_many], &[]).await;
    assert_eq!(env.core.sync(0).await.unwrap().len(), 0);
    let sent = answers(&env).await;
    assert_eq!(sent.len(), 3);
    assert!(sent.iter().all(|(_, body)| body["outcome"] == "error"));
}

#[tokio::test]
async fn pairing_requests_are_cleaned_and_answered() {
    let env = env().await;
    let pairing = json!({"v": 1, "id": "p1", "client_name": "Cla\u{202E}ude", "client_host": "claude.ai",
        "choices": [12, 47, 83], "created_at": 50});
    serve_pending(&env, &[], &[pairing]).await;
    let items = env.core.sync(0).await.unwrap();
    assert_eq!(items[0].title, "Connect Claude to Reins?");
    let view = env.core.pairing_view("p1".to_owned()).await.unwrap();
    assert_eq!((view.client_name.as_str(), view.choices.clone()), ("Claude", vec![12, 47, 83]));
    // Not answerable with a number that was not offered, or without a number.
    let err = env.core.answer_pairing("p1".to_owned(), true, Some(99), None).await.unwrap_err();
    assert!(matches!(err, CoreError::Invalid { .. }));
    assert!(env.core.answer_pairing("p1".to_owned(), true, None, None).await.is_err());

    Mock::given(method("POST"))
        .and(path("/reins/api/pairings/p1/response"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"connection_id": "c9"})))
        .with_priority(1)
        .mount(&env.server)
        .await;
    env.core.answer_pairing("p1".to_owned(), true, Some(47), Some("  Work \u{202E}Claude ".to_owned())).await.unwrap();
    let posted: Vec<Value> = env
        .server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path() == "/reins/api/pairings/p1/response")
        .map(|r| serde_json::from_slice(&r.body).unwrap())
        .collect();
    assert_eq!(posted[0], json!({"v": 1, "approved": true, "chosen_code": 47, "label": "Work Claude"}));
    assert_eq!(env.core.pending().await.unwrap().len(), 0);
    assert_eq!(env.core.activity(1).await.unwrap()[0].action, "pair");
}

#[tokio::test]
async fn a_wrong_number_cancels_the_pairing_and_reports_it() {
    let env = env().await;
    let pairing = json!({"v": 1, "id": "p2", "client_name": "ChatGPT", "client_host": "chatgpt.com", "choices": [10, 20, 30], "created_at": 1});
    serve_pending(&env, &[], &[pairing]).await;
    env.core.sync(0).await.unwrap();
    Mock::given(method("POST"))
        .and(path("/reins/api/pairings/p2/response"))
        .respond_with(ResponseTemplate::new(409).set_body_json(json!({"error": "wrong_code", "message": "m"})))
        .with_priority(1)
        .mount(&env.server)
        .await;
    let err = env.core.answer_pairing("p2".to_owned(), true, Some(20), None).await.unwrap_err();
    assert_eq!(
        err,
        CoreError::Server {
            status: 409,
            reason: "wrong_code".to_owned()
        }
    );
    assert!(env.core.pending().await.unwrap().is_empty(), "a cancelled pairing is gone");
    // Refusing is a valid answer too.
    let again = json!({"v": 1, "id": "p3", "client_name": "X", "client_host": "x.com", "choices": [10, 20, 30], "created_at": 2});
    serve_pending(&env, &[], &[again]).await;
    env.core.sync(0).await.unwrap();
    Mock::given(method("POST"))
        .and(path("/reins/api/pairings/p3/response"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"connection_id": null})))
        .with_priority(1)
        .mount(&env.server)
        .await;
    env.core.answer_pairing("p3".to_owned(), false, None, None).await.unwrap();
    assert_eq!(env.core.activity(1).await.unwrap()[0].outcome, "denied");
}

#[tokio::test]
async fn revoking_a_connection_revokes_its_grants_and_drops_its_parked_requests() {
    let env = env().await;
    serve_gmail_search(&env, &[("m1", "a@bank.com")]).await;
    serve_pending(&env, &[search_request("r0", "c1", "AI", "x")], &[]).await;
    env.core.sync(0).await.unwrap();
    env.core.approve("r0".to_owned(), choice(&["m1"], Some(bank_grant()))).await.unwrap();
    serve_pending(&env, &[search_request("r1", "other", "Other", "x")], &[]).await;
    env.core.sync(0).await.unwrap();
    assert_eq!(env.core.pending().await.unwrap().len(), 1);
    Mock::given(method("GET"))
        .and(path("/reins/api/connections"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"connections": [
            {"id": "c1", "label": "Claude", "client_name": "Claude", "client_host": "claude.ai", "created_at": 1, "last_used_at": 5}]})))
        .mount(&env.server)
        .await;
    let listed = env.core.connections().await.unwrap();
    assert_eq!(
        (listed[0].id.as_str(), listed[0].client_host.as_str(), listed[0].last_used_at),
        ("c1", "claude.ai", Some(5))
    );
    Mock::given(method("DELETE"))
        .and(path("/reins/api/connections/c1"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&env.server)
        .await;
    env.core.revoke_connection("c1".to_owned()).await.unwrap();
    assert!(env.core.grants().await.unwrap().is_empty(), "nothing is left to resume for a disconnected AI");
    assert_eq!(env.core.pending().await.unwrap().len(), 1, "another connection's request stays");
    Mock::given(method("DELETE"))
        .and(path("/reins/api/connections/other"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&env.server)
        .await;
    env.core.revoke_connection("other".to_owned()).await.unwrap();
    assert_eq!(env.core.pending().await.unwrap().len(), 0);
    assert!(matches!(env.core.revoke_connection("../x".to_owned()).await, Err(CoreError::Invalid { .. })));
}

#[tokio::test]
async fn grants_can_be_revoked_and_unknown_ids_are_not_found() {
    let env = env().await;
    serve_gmail_search(&env, &[("m1", "a@bank.com")]).await;
    serve_pending(&env, &[search_request("r0", "c1", "AI", "x")], &[]).await;
    env.core.sync(0).await.unwrap();
    env.core.approve("r0".to_owned(), choice(&["m1"], Some(bank_grant()))).await.unwrap();
    let id = env.core.grants().await.unwrap()[0].id.clone();
    env.core.revoke_grant(id.clone()).await.unwrap();
    let listed = env.core.grants().await.unwrap();
    assert_eq!((listed.len(), listed[0].active, listed[0].state.as_str()), (1, false, "revoked"));
    assert_eq!(env.core.revoke_grant("nope".to_owned()).await.unwrap_err(), CoreError::NotFound);
    serve_pending(&env, &[search_request("r1", "c1", "AI", "x")], &[]).await;
    assert_eq!(env.core.sync(0).await.unwrap().len(), 1, "a revoked grant no longer covers anything");
    env.core.deny("r1".to_owned()).await.unwrap();

    // A deleted grant can be started again for a while, and then covers what it covered before.
    assert!(matches!(env.core.resume_grant(id.clone(), 5).await, Err(CoreError::Invalid { .. })), "too short");
    assert_eq!(env.core.resume_grant("nope".to_owned(), 3600).await.unwrap_err(), CoreError::NotFound);
    env.core.resume_grant(id.clone(), 3600).await.unwrap();
    let resumed = &env.core.grants().await.unwrap()[0];
    assert_eq!((resumed.active, resumed.state.as_str(), resumed.uses), (true, "active", 0));
    assert!(resumed.expires_at.is_some_and(|t| t > resumed.created_at));
    assert!(matches!(env.core.resume_grant(id.clone(), 3600).await, Err(CoreError::Invalid { .. })), "already running");
    serve_pending(&env, &[search_request("r2", "c1", "AI", "x")], &[]).await;
    assert!(env.core.sync(0).await.unwrap().is_empty(), "the resumed grant answers by itself");

    // A running grant is revoked first; an ended one can then be removed for good.
    assert!(matches!(env.core.delete_grant(id.clone()).await, Err(CoreError::Invalid { .. })));
    env.core.revoke_grant(id.clone()).await.unwrap();
    env.core.delete_grant(id.clone()).await.unwrap();
    assert_eq!(env.core.grants().await.unwrap().len(), 0);
    assert_eq!(env.core.delete_grant(id).await.unwrap_err(), CoreError::NotFound);
}

#[tokio::test]
async fn nothing_works_signed_out_and_state_survives_a_restart() {
    let env = env().await;
    serve_gmail_search(&env, &[("m1", "a@x.com")]).await;
    serve_pending(&env, &[search_request("r1", "c1", "AI", "x")], &[]).await;
    env.core.sync(0).await.unwrap();
    let info = env.core.session().await.unwrap();
    assert_eq!((info.email.as_str(), info.server_url.as_str()), (EMAIL, env.server.uri().as_str()));

    // A new process on the same data directory: session and parked item are back.
    let restarted = open_core(env.dir.path(), &env.gmail, &env.google, &env.notifier);
    assert_eq!(restarted.session().await.unwrap().email, EMAIL);
    assert_eq!(restarted.pending().await.unwrap().len(), 1);
    Mock::given(method("POST"))
        .and(path("/identity/connect/token"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"access_token": "A2", "refresh_token": "R2", "expires_in": 7200})),
        )
        .mount(&env.server)
        .await;
    restarted.deny("r1".to_owned()).await.unwrap();

    restarted.logout().await.unwrap();
    assert!(restarted.session().await.is_none());
    assert_eq!(restarted.sync(0).await.unwrap_err(), CoreError::NotLoggedIn);
    assert_eq!(restarted.register_device(None).await.unwrap_err(), CoreError::NotLoggedIn);
    assert_eq!(restarted.connections().await.unwrap_err(), CoreError::NotLoggedIn);
    assert_eq!(restarted.handle_push("req".to_owned(), "x".to_owned()).await.unwrap_err(), CoreError::NotLoggedIn);
    assert_eq!(restarted.activity(5).await.unwrap().len(), 0, "signed-out views have no decrypted account history");
    assert!(matches!(restarted.handle_push("bogus".to_owned(), "x".to_owned()).await, Err(CoreError::Invalid { .. })));
}

#[tokio::test]
async fn register_device_sends_the_fcm_token_and_reports_server_refusals() {
    let env = env().await;
    Mock::given(method("PUT"))
        .and(path("/reins/api/device"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"replaced_previous": false})))
        .mount(&env.server)
        .await;
    env.core.register_device(Some("fcm-token-1".to_owned())).await.unwrap();
    let put = env
        .server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .find(|r| r.method.as_str() == "PUT" && r.url.path() == "/reins/api/device")
        .unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&put.body).unwrap(), json!({"fcm_token": "fcm-token-1"}));
    Mock::given(method("GET"))
        .and(path("/reins/api/pending"))
        .and(query_param("wait", "25"))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({"error": "not_approval_device", "message": "m"})))
        .mount(&env.server)
        .await;
    assert_eq!(
        env.core.sync(99).await.unwrap_err(),
        CoreError::Server {
            status: 403,
            reason: "not_approval_device".to_owned()
        }
    );
    env.core.handle_push("replaced".to_owned(), String::new()).await.unwrap();
}

// ---- several accounts -----------------------------------------------------------------------------------------

const WORK: &str = "work@gmail.com";

fn for_account(mut request: Value, account: &str) -> Value {
    request["account"] = json!(account);
    request
}

/// Gmail answers with `messages` only to tokens issued for `account`.
async fn serve_gmail_account(env: &Env, account: &str, messages: &[(&str, &str)]) {
    let token = format!("Bearer google-token-{}-\\d+", account.replace('.', "\\."));
    let ids: Vec<Value> = messages.iter().map(|(id, _)| json!({"id": id})).collect();
    Mock::given(method("GET"))
        .and(path("/users/me/messages"))
        .and(wiremock::matchers::header_regex("authorization", token.as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"messages": ids})))
        .mount(&env.gmail)
        .await;
    for (id, from) in messages {
        Mock::given(method("GET"))
            .and(path(format!("/users/me/messages/{id}")))
            .and(wiremock::matchers::header_regex("authorization", token.as_str()))
            .respond_with(ResponseTemplate::new(200).set_body_json(gmail_message(id, from, "Subject", Some("Text"))))
            .mount(&env.gmail)
            .await;
    }
}

async fn add_work_account(env: &Env) {
    Mock::given(method("GET"))
        .and(path("/users/me/profile"))
        .and(wiremock::matchers::header_regex("authorization", r"Bearer google-token-work@gmail\.com-\d+"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"emailAddress": "Work@Gmail.com"})))
        .with_priority(1)
        .mount(&env.gmail)
        .await;
    let added = env.core.add_account("work@gmail.com".to_owned()).await.unwrap();
    assert_eq!((added.service.as_str(), added.account.as_str()), ("gmail", WORK));
}

#[tokio::test]
async fn accounts_are_added_listed_and_removed() {
    let env = env().await;
    assert_eq!(env.core.accounts().await.unwrap().len(), 2, "the owning vault and the Gmail account of the setup");
    add_work_account(&env).await;
    let names: Vec<_> =
        env.core.accounts().await.unwrap().into_iter().filter(|a| a.service == "gmail").map(|a| a.account).collect();
    assert_eq!(names, [GMAIL, WORK]);
    assert!(matches!(env.core.add_account("not an address".to_owned()).await, Err(CoreError::Invalid { .. })));
    assert_eq!(env.core.account_status(WORK.to_owned()).await, GmailStatus::Ready);
    env.core.remove_account("WORK@gmail.com".to_owned()).await.unwrap();
    assert_eq!(env.core.remove_account(WORK.to_owned()).await.unwrap_err(), CoreError::NotFound);
    assert_eq!(env.core.accounts().await.unwrap().len(), 2);
}

fn list_request(id: &str, conn: &str, service: Option<&str>) -> Value {
    list_request_for(id, conn, service, false)
}

fn list_request_for(id: &str, conn: &str, service: Option<&str>, ask_for_more: bool) -> Value {
    let mut call = json!({"tool": "list_accounts", "ask_for_more": ask_for_more});
    if let Some(service) = service {
        call["service"] = json!(service);
    }
    json!({"v": 1, "id": id, "connection_id": conn, "connection_label": "Claude", "created_at": 100, "call": call})
}

#[tokio::test]
async fn the_ai_must_name_an_account_but_is_not_told_which_ones_exist() {
    let env = env().await;
    add_work_account(&env).await;
    serve_pending(&env, &[search_request("r1", "c1", "Claude", "in:inbox")], &[]).await;
    assert!(env.core.sync(0).await.unwrap().is_empty(), "nothing to approve yet");
    let told = answers(&env).await;
    assert_eq!(told[0].1["outcome"], "error");
    let message = told[0].1["message"].as_str().unwrap();
    assert!(
        message.contains("reins_list_accounts") && !message.contains(WORK) && !message.contains(GMAIL),
        "{message}"
    );
    let entry = &env.core.activity(1).await.unwrap()[0];
    assert_eq!((entry.action.as_str(), entry.outcome.as_str()), ("search", "error"));

    serve_pending(&env, &[for_account(search_request("r2", "c1", "Claude", "x"), "nobody@gmail.com")], &[]).await;
    env.core.sync(0).await.unwrap();
    let told = answers(&env).await;
    let message = told[1].1["message"].as_str().unwrap();
    assert!(message.contains("not connected") && !message.contains(WORK) && !message.contains(GMAIL), "{message}");
}

#[tokio::test]
async fn the_ai_sees_the_integrations_freely_but_their_accounts_only_when_the_user_allows_it() {
    let env = env().await;
    add_work_account(&env).await;

    // Integrations: no accounts in the answer, nothing to approve.
    serve_pending(&env, &[list_request("r1", "c1", None)], &[]).await;
    assert_eq!(env.core.sync(0).await.unwrap().len(), 0);
    let told = answers(&env).await;
    assert_eq!(told[0].1["result"]["kind"], "integrations");
    assert_eq!(
        told[0].1["result"]["integrations"],
        json!([{"service": "vault", "name": "Password vault"}, {"service": "gmail", "name": "Gmail"}])
    );
    assert!(!told[0].1.to_string().contains('@'), "no address in the answer: {}", told[0].1);

    // Accounts of an integration: waits for the user, who is shown every account and picks the ones to share.
    serve_pending(&env, &[list_request("r2", "c1", Some("gmail"))], &[]).await;
    let items = env.core.sync(0).await.unwrap();
    assert_eq!((items[0].action.as_str(), items[0].count, items[0].service.as_str()), ("accounts", 2, "gmail"));
    let view = env.core.approval_view("r2".to_owned()).await.unwrap();
    assert_eq!((view.kind, view.accounts.clone()), (ApprovalKind::Accounts, vec![GMAIL.to_owned(), WORK.to_owned()]));
    assert_eq!(view.shared_accounts.len(), 0);
    assert_eq!(answers(&env).await.len(), 1, "nothing was told yet");
    let month = || StandingGrant {
        duration_secs: Some(30 * 86_400),
        max_uses: None,
        scope: scope(),
    };
    assert!(
        matches!(env.core.approve("r2".to_owned(), choice(&[], Some(month()))).await, Err(CoreError::Invalid { .. })),
        "at least one account has to be picked"
    );
    assert!(
        matches!(
            env.core.approve("r2".to_owned(), choice(&["stranger@gmail.com"], Some(month()))).await,
            Err(CoreError::Invalid { .. })
        ),
        "only what was offered can be picked"
    );

    // Only the first account is shared, for a month: the AI learns that one more exists, not which.
    env.core.approve("r2".to_owned(), choice(&[GMAIL], Some(month()))).await.unwrap();
    let told = answers(&env).await;
    assert_eq!(told[1].1["result"]["kind"], "accounts");
    assert_eq!(told[1].1["result"]["accounts"], json!([{"service": "gmail", "account": GMAIL}]));
    assert_eq!(told[1].1["result"]["withheld"], 1);
    assert!(!told[1].1.to_string().contains(WORK), "the private account is not named: {}", told[1].1);
    let entry = &env.core.activity(1).await.unwrap()[0];
    assert_eq!((entry.action.as_str(), entry.count, entry.service.as_str()), ("accounts", 1, "gmail"));
    assert_eq!(entry.info.accounts, [GMAIL], "the log says which accounts were shown");
    let grant = env.core.grants().await.unwrap().remove(0);
    assert_eq!(
        (grant.action.as_str(), grant.active, grant.summary.as_str()),
        ("accounts", true, "See 1 of your Gmail accounts")
    );
    assert_eq!(grant.lines, [GMAIL, "Not what is in them"]);
    assert!(grant.expires_at.is_some_and(|t| t - grant.created_at == 30 * 86_400));
    assert!(grant.editable_scope.is_none());

    // From now on this AI is told the same without asking, still with the count of what is withheld.
    serve_pending(&env, &[list_request("r3", "c1", Some("gmail"))], &[]).await;
    assert!(env.core.sync(0).await.unwrap().is_empty(), "covered by the grant");
    let told = answers(&env).await;
    assert_eq!(
        (told[2].1["result"]["kind"].as_str(), told[2].1["result"]["withheld"].as_u64()),
        (Some("accounts"), Some(1))
    );
    assert_eq!(env.core.activity(1).await.unwrap()[0].grant_id.as_deref(), Some(grant.id.as_str()));

    // Asking for more shows the account that is still private (the shared one is already ticked and locked).
    serve_pending(&env, &[list_request_for("r5", "c1", Some("gmail"), true)], &[]).await;
    let items = env.core.sync(0).await.unwrap();
    assert_eq!((items[0].action.as_str(), items[0].count), ("accounts", 1));
    let view = env.core.approval_view("r5".to_owned()).await.unwrap();
    assert_eq!(
        (view.accounts.clone(), view.shared_accounts.clone()),
        (vec![GMAIL.to_owned(), WORK.to_owned()], vec![GMAIL.to_owned()])
    );
    assert!(
        matches!(
            env.core.approve("r5".to_owned(), choice(&[GMAIL], Some(month()))).await,
            Err(CoreError::Invalid { .. })
        ),
        "what was already shared is not a new pick"
    );
    env.core.approve("r5".to_owned(), choice(&[WORK], Some(month()))).await.unwrap();
    let told = answers(&env).await;
    assert_eq!(
        told[3].1["result"]["accounts"],
        json!([{"service": "gmail", "account": GMAIL}, {"service": "gmail", "account": WORK}])
    );
    assert_eq!(told[3].1["result"]["withheld"], 0);
    assert_eq!(env.core.grants().await.unwrap().len(), 2, "one grant per decision");

    // Everything is covered now; another AI still has to ask.
    serve_pending(&env, &[list_request_for("r6", "c1", Some("gmail"), true)], &[]).await;
    assert!(env.core.sync(0).await.unwrap().is_empty(), "nothing left to ask about");
    assert_eq!(answers(&env).await[4].1["result"]["withheld"], 0);
    serve_pending(&env, &[list_request("r4", "c2", Some("gmail"))], &[]).await;
    assert_eq!(env.core.sync(0).await.unwrap().len(), 1);
    env.core.deny("r4".to_owned()).await.unwrap();
    assert_eq!(answers(&env).await[5].1["outcome"], "denied");
    let denied = env.core.activity(1).await.unwrap().remove(0);
    assert_eq!((denied.action.as_str(), denied.outcome.as_str()), ("accounts", "denied"));
}

#[tokio::test]
async fn accounts_can_also_be_shown_just_once_and_only_of_a_connected_integration() {
    let env = env().await;
    serve_pending(&env, &[list_request("r1", "c1", Some("gmail"))], &[]).await;
    env.core.sync(0).await.unwrap();
    env.core.approve("r1".to_owned(), choice(&[GMAIL], None)).await.unwrap();
    assert_eq!(answers(&env).await[0].1["result"]["kind"], "accounts");
    assert!(env.core.grants().await.unwrap().is_empty(), "once means once");
    serve_pending(&env, &[list_request("r2", "c1", Some("drive"))], &[]).await;
    assert_eq!(env.core.sync(0).await.unwrap().len(), 0);
    let told = answers(&env).await;
    assert_eq!(told[1].1["outcome"], "error");
    assert!(told[1].1["message"].as_str().unwrap().contains("not connected"));
}

#[tokio::test]
async fn a_request_is_served_from_the_account_it_names_and_grants_stay_with_that_account() {
    let env = env().await;
    add_work_account(&env).await;
    serve_gmail_account(&env, GMAIL, &[("p1", "friend@example.org")]).await;
    serve_gmail_account(&env, WORK, &[("w1", "boss@corp.com")]).await;

    serve_pending(&env, &[for_account(search_request("r1", "c1", "Claude", "in:inbox"), WORK)], &[]).await;
    let items = env.core.sync(0).await.unwrap();
    assert_eq!(items[0].account.as_deref(), Some(WORK));
    let view = env.core.approval_view("r1".to_owned()).await.unwrap();
    assert_eq!(view.account.as_deref(), Some(WORK));
    assert_eq!(
        view.messages.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        ["w1"],
        "the work mailbox was searched"
    );
    let standing = StandingGrant {
        duration_secs: Some(3600),
        max_uses: None,
        scope: GrantScopeChoice {
            sender_domains: vec!["corp.com".to_owned()],
            ..scope()
        },
    };
    env.core.approve("r1".to_owned(), choice(&["w1"], Some(standing))).await.unwrap();
    let grant = &env.core.grants().await.unwrap()[0];
    assert_eq!(grant.account.as_deref(), Some(WORK));
    let entry = &env.core.activity(1).await.unwrap()[0];
    assert_eq!(entry.account.as_deref(), Some(WORK));

    // The same kind of request is now answered for the work account and still asked for the personal one.
    serve_pending(&env, &[for_account(search_request("r2", "c1", "Claude", "again"), WORK)], &[]).await;
    assert!(env.core.sync(0).await.unwrap().is_empty(), "covered by the work grant");
    serve_gmail_account(&env, GMAIL, &[("p2", "boss@corp.com")]).await;
    serve_pending(&env, &[for_account(search_request("r3", "c1", "Claude", "again"), GMAIL)], &[]).await;
    assert_eq!(env.core.sync(0).await.unwrap().len(), 1, "the grant does not reach the personal account");

    // Removing the work account takes its grant along.
    env.core.remove_account(WORK.to_owned()).await.unwrap();
    assert_eq!(env.core.grants().await.unwrap().len(), 0);
}

#[tokio::test]
async fn removing_an_account_answers_the_requests_waiting_for_it() {
    let env = env().await;
    add_work_account(&env).await;
    serve_gmail_account(&env, WORK, &[("w1", "boss@corp.com")]).await;
    serve_pending(&env, &[for_account(search_request("r1", "c1", "Claude", "x"), WORK)], &[]).await;
    assert_eq!(env.core.sync(0).await.unwrap().len(), 1);
    env.core.remove_account(WORK.to_owned()).await.unwrap();
    assert_eq!(env.core.pending().await.unwrap().len(), 0);
    let told = answers(&env).await;
    assert_eq!(told[0].1["outcome"], "error");
    assert!(told[0].1["message"].as_str().unwrap().contains("disconnected"));
}

#[tokio::test]
async fn a_new_grant_is_made_for_one_of_the_connected_accounts() {
    let env = env().await;
    serve_connections(&env).await;
    add_work_account(&env).await;
    let one_time = StandingGrant {
        duration_secs: Some(3600),
        max_uses: Some(1),
        scope: GrantScopeChoice {
            sender_domains: vec!["corp.com".to_owned()],
            ..scope()
        },
    };
    let clone = || one_time.clone();
    assert!(matches!(
        env.core.create_grant("c1".to_owned(), "nobody@gmail.com".to_owned(), ApprovalKind::Read, clone()).await,
        Err(CoreError::Invalid { .. })
    ));
    env.core.create_grant("c1".to_owned(), "WORK@gmail.com".to_owned(), ApprovalKind::Read, clone()).await.unwrap();
    assert_eq!(env.core.grants().await.unwrap()[0].account.as_deref(), Some(WORK));
    assert_eq!(env.core.activity(1).await.unwrap()[0].account.as_deref(), Some(WORK));
}

#[tokio::test]
async fn an_ended_grant_can_be_resumed_with_a_new_period_new_limits_and_a_changed_scope() {
    let env = env().await;
    serve_gmail_search(&env, &[("m1", "a@bank.com")]).await;
    serve_pending(&env, &[search_request("r0", "c1", "AI", "x")], &[]).await;
    env.core.sync(0).await.unwrap();
    env.core.approve("r0".to_owned(), choice(&["m1"], Some(bank_grant()))).await.unwrap();
    let before = env.core.grants().await.unwrap().remove(0);
    let editable = before.editable_scope.clone().expect("a sender rule can be edited");
    assert_eq!((editable.sender_domains.clone(), editable.all_mail), (vec!["bank.com".to_owned()], false));
    env.core.revoke_grant(before.id.clone()).await.unwrap();

    let changed = StandingGrant {
        duration_secs: Some(7_200),
        max_uses: Some(2),
        scope: GrantScopeChoice {
            sender_domains: vec!["statements.example".to_owned()],
            sender_addresses: vec!["Boss@Corp.com".to_owned()],
            subject_pattern: Some("invoice".to_owned()),
            ..scope()
        },
    };
    env.core.resume_grant_edited(before.id.clone(), changed).await.unwrap();
    let after = env.core.grants().await.unwrap().remove(0);
    assert_eq!((after.id.as_str(), after.active, after.uses, after.max_uses), (before.id.as_str(), true, 0, Some(2)));
    assert_eq!(after.expires_at.map(|t| t - after.created_at), Some(7_200));
    let now_covered = after.editable_scope.unwrap();
    assert_eq!(now_covered.sender_domains, ["statements.example"]);
    assert_eq!(now_covered.sender_addresses, ["boss@corp.com"]);
    assert_eq!(now_covered.subject_pattern.as_deref(), Some("invoice"));
    assert_eq!(after.account, before.account);

    // Unusable edits are refused and leave the grant as it was.
    env.core.revoke_grant(after.id.clone()).await.unwrap();
    let nothing = StandingGrant {
        duration_secs: Some(3_600),
        max_uses: None,
        scope: scope_all_empty(),
    };
    assert!(matches!(env.core.resume_grant_edited(after.id.clone(), nothing).await, Err(CoreError::Invalid { .. })));
    let forever = StandingGrant {
        duration_secs: None,
        max_uses: None,
        scope: GrantScopeChoice {
            sender_domains: vec!["bank.com".to_owned()],
            ..scope()
        },
    };
    assert!(matches!(env.core.resume_grant_edited(after.id.clone(), forever).await, Err(CoreError::Invalid { .. })));
    let month_of_everything = StandingGrant {
        duration_secs: Some(30 * 86_400),
        max_uses: None,
        scope: GrantScopeChoice {
            all_mail: true,
            ..scope()
        },
    };
    assert!(matches!(
        env.core.resume_grant_edited(after.id.clone(), month_of_everything).await,
        Err(CoreError::Invalid { .. })
    ));
    let ok = StandingGrant {
        duration_secs: Some(3_600),
        ..bank_grant()
    };
    assert_eq!(env.core.resume_grant_edited("nope".to_owned(), ok).await.unwrap_err(), CoreError::NotFound);
    assert!(!env.core.grants().await.unwrap()[0].active);
}

fn scope_all_empty() -> GrantScopeChoice {
    scope()
}

#[tokio::test]
async fn an_email_in_the_activity_can_be_opened_in_full_from_gmail() {
    let env = env().await;
    serve_gmail_search(&env, &[("m1", "Bank <alerts@bank.com>"), ("m2", "Pal <pal@example.org>")]).await;
    serve_pending(&env, &[search_request("r1", "c1", "Claude", "in:inbox")], &[]).await;
    env.core.sync(0).await.unwrap();
    env.core.approve("r1".to_owned(), choice(&["m1"], None)).await.unwrap();
    let entry = &env.core.activity(1).await.unwrap()[0];
    assert_eq!(
        entry.info.messages.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        ["m1"],
        "the id is kept, not the text"
    );
    let email = env.core.fetch_email(entry.account.clone(), "m1".to_owned()).await.unwrap();
    assert_eq!((email.id.as_str(), email.subject.as_str(), email.body.as_str()), ("m1", "Subject m1", "Full text"));
    assert!(email.from.contains("alerts@bank.com"), "{}", email.from);
    Mock::given(method("GET"))
        .and(path("/users/me/messages/gone"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&env.gmail)
        .await;
    assert!(
        env.core.fetch_email(None, "gone".to_owned()).await.is_err(),
        "an email deleted since is reported, not invented"
    );
    assert!(matches!(env.core.fetch_email(None, "../x".to_owned()).await, Err(CoreError::Invalid { .. })));
}

#[tokio::test]
async fn a_search_is_one_tap_and_repeats_offer_a_longer_permission_for_the_same_account() {
    let env = env().await;
    serve_gmail_search(&env, &[("m1", "Bank <alerts@bank.com>"), ("m2", "Friend <pal@example.org>")]).await;
    serve_pending(&env, &[search_request("r1", "c1", "Claude", "from:bank")], &[]).await;
    let items = env.core.sync(0).await.unwrap();
    assert!(items[0].quick, "a search can be approved from the notification");
    assert_eq!(items[0].headline, "Claude gets the 2 emails found for \"from:bank\".");
    let view = env.core.approval_view("r1".to_owned()).await.unwrap();
    assert_eq!(view.headline, items[0].headline);
    let quick = view.quick.unwrap();
    assert!(quick.from_notification);
    assert_eq!((quick.repeats, quick.allow_what.as_str()), (0, "searching and reading me@gmail.com"));
    let allow = quick.allow.unwrap();
    assert_eq!((allow.duration_secs, allow.scope.all_mail), (Some(3_600), true));

    // From the notification: everything found is released, nothing is remembered.
    env.core.approve_quick("r1".to_owned()).await.unwrap();
    let sent = answers(&env).await;
    assert_eq!(sent[0].1["result"]["messages"].as_array().unwrap().len(), 2);
    assert_eq!(env.core.grants().await.unwrap().len(), 0);

    // The same again: counted; the third time offers eight hours.
    serve_pending(&env, &[search_request("r2", "c1", "Claude", "from:bank")], &[]).await;
    env.core.sync(0).await.unwrap();
    assert_eq!(env.core.approval_view("r2".to_owned()).await.unwrap().quick.unwrap().repeats, 1);
    env.core.approve_quick("r2".to_owned()).await.unwrap();
    serve_pending(&env, &[search_request("r3", "c1", "Claude", "in:inbox")], &[]).await;
    env.core.sync(0).await.unwrap();
    let quick = env.core.approval_view("r3".to_owned()).await.unwrap().quick.unwrap();
    assert_eq!(quick.repeats, 2, "a different query to the same account is the same thing");
    assert_eq!(quick.allow.as_ref().unwrap().duration_secs, Some(8 * 3_600));
    // Another AI's approvals do not count for this one.
    serve_pending(&env, &[search_request("r4", "c2", "ChatGPT", "from:bank")], &[]).await;
    env.core.sync(0).await.unwrap();
    assert_eq!(env.core.approval_view("r4".to_owned()).await.unwrap().quick.unwrap().repeats, 0);

    // "Approve and allow for 8 hours": the next search by this AI is answered without asking.
    env.core.approve("r3".to_owned(), choice(&["m1", "m2"], quick.allow)).await.unwrap();
    let grant = &env.core.grants().await.unwrap()[0];
    assert_eq!((grant.summary.as_str(), grant.account.as_deref()), ("Read any email", Some("me@gmail.com")));
    serve_pending(&env, &[search_request("r5", "c1", "Claude", "anything")], &[]).await;
    assert_eq!(env.core.sync(0).await.unwrap().len(), 1, "only ChatGPT's request still waits");
}

#[tokio::test]
async fn a_send_offers_to_allow_exactly_its_recipients() {
    let env = env().await;
    Mock::given(method("POST"))
        .and(path("/users/me/messages/send"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "sent1", "threadId": "t1"})))
        .mount(&env.gmail)
        .await;
    serve_pending(&env, &[send_request("s1", "c1", "Boss@Work.com")], &[]).await;
    let items = env.core.sync(0).await.unwrap();
    assert_eq!(items[0].headline, "An email to boss@work.com goes out from me@gmail.com.");
    let quick = env.core.approval_view("s1".to_owned()).await.unwrap().quick.unwrap();
    assert_eq!(quick.allow_what, "emails to boss@work.com");
    let allow = quick.allow.unwrap();
    assert_eq!(allow.scope.recipient_addresses, ["boss@work.com"]);
    assert!(allow.scope.recipient_domains.is_empty(), "never a whole domain");
    env.core.approve("s1".to_owned(), choice(&[], Some(allow))).await.unwrap();

    serve_pending(&env, &[send_request("s2", "c1", "boss@work.com")], &[]).await;
    assert!(env.core.sync(0).await.unwrap().is_empty(), "the same recipient is covered");
    serve_pending(&env, &[send_request("s3", "c1", "peer@work.com")], &[]).await;
    assert_eq!(env.core.sync(0).await.unwrap().len(), 1, "a colleague is not");
}

#[tokio::test]
async fn a_permission_request_is_never_answered_in_one_tap() {
    let env = env().await;
    serve_pending(&env, &[grant_request("g1", "c1", "Claude", &bank_ask(3_600))], &[]).await;
    let items = env.core.sync(0).await.unwrap();
    assert!(!items[0].quick);
    assert!(items[0].headline.starts_with("Claude may read emails from"), "{}", items[0].headline);
    assert!(env.core.approval_view("g1".to_owned()).await.unwrap().quick.is_none());
    assert!(matches!(env.core.approve_quick("g1".to_owned()).await, Err(CoreError::Invalid { .. })));
    assert!(answers(&env).await.is_empty() && env.core.grants().await.unwrap().is_empty());
    assert_eq!(env.core.pending().await.unwrap().len(), 1, "it still waits for the user");
}

/// Answers the pairing `id` with a new connection `connection`.
async fn pair_as(env: &Env, id: &str, connection: &str) {
    let pairing = json!({"v": 1, "id": id, "client_name": "Claude", "client_host": "claude.ai", "choices": [12, 47, 83],
        "created_at": 50});
    serve_pending(env, &[], &[pairing]).await;
    env.core.sync(0).await.unwrap();
    Mock::given(method("POST"))
        .and(path(format!("/reins/api/pairings/{id}/response")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"connection_id": connection})))
        .with_priority(1)
        .mount(&env.server)
        .await;
    env.core.answer_pairing(id.to_owned(), true, Some(47), Some("Claude".to_owned())).await.unwrap();
}

#[tokio::test]
async fn a_new_ai_asks_for_everything_until_the_starting_rule_says_otherwise() {
    let env = env().await;
    env.core.engine().register_account("gmail", "me@gmail.com").unwrap();
    assert_eq!(env.core.starting_policy().await.unwrap(), None, "not chosen yet");
    pair_as(&env, "p1", "c8").await;
    assert!(env.core.grants().await.unwrap().is_empty(), "nothing is given without the user's choice");
}

#[tokio::test]
async fn reads_for_a_day_lets_a_new_ai_read_but_never_send_or_see_codes() {
    let env = env().await;
    env.core.engine().register_account("gmail", "me@gmail.com").unwrap();
    env.core.engine().register_account("telegram", "+15550100").unwrap();
    env.core.engine().register_account("vault", "me@example.com").unwrap();
    env.core.set_starting_policy(StartingPolicy::ReadsForADay).await.unwrap();
    assert_eq!(env.core.starting_policy().await.unwrap(), Some(StartingPolicy::ReadsForADay));
    let before = reins_core::store::unix_now();
    pair_as(&env, "p1", "c9").await;

    let grants = env.core.grants().await.unwrap();
    let mut shown: Vec<(&str, &str, &str)> =
        grants.iter().map(|g| (g.service.as_str(), g.action.as_str(), g.origin.as_str())).collect();
    shown.sort_unstable();
    assert_eq!(shown, [("gmail", "read", "starter"), ("telegram", "read", "starter")], "never the vault");
    for g in &grants {
        assert_eq!(g.connection_id, "c9");
        let left = g.expires_at.unwrap() - before;
        assert!((86_000..=86_500).contains(&left), "a day: {left}");
    }
    let logged = &env.core.activity(1).await.unwrap()[0];
    assert_eq!((logged.action.as_str(), logged.outcome.as_str()), ("grant", "granted"));

    // A search is answered without asking, except the email that looks like a login code.
    let ids = json!({"messages": [{"id": "m1"}, {"id": "m2"}]});
    Mock::given(method("GET"))
        .and(path("/users/me/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(ids))
        .mount(&env.gmail)
        .await;
    for (id, subject) in [("m1", "Lunch on Friday"), ("m2", "Your sign in code is 481516")] {
        Mock::given(method("GET"))
            .and(path(format!("/users/me/messages/{id}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(gmail_message(
                id,
                "a@example.com",
                subject,
                Some("x"),
            )))
            .mount(&env.gmail)
            .await;
    }
    serve_pending(&env, &[search_request("r1", "c9", "Claude", "anything")], &[]).await;
    let waiting = env.core.sync(0).await.unwrap();
    assert_eq!(waiting.len(), 1, "the code waits for the user");
    let view = env.core.approval_view("r1".to_owned()).await.unwrap();
    let messages: Vec<_> = view.messages.iter().map(|m| (m.id.as_str(), m.covered_by_grant, m.sensitive)).collect();
    assert_eq!(messages, [("m1", true, false), ("m2", false, true)]);
    assert!(!waiting[0].quick, "a code needs a look, not a notification button");
    // Approving untouched releases what the grant covers, never the code.
    env.core.approve_quick("r1".to_owned()).await.unwrap();
    let sent = answers(&env).await;
    let released: Vec<_> =
        sent[0].1["result"]["messages"].as_array().unwrap().iter().map(|m| m["id"].clone()).collect();
    assert_eq!(released, [json!("m1")]);

    // Sending still asks.
    serve_pending(&env, &[send_request("s1", "c9", "boss@work.com")], &[]).await;
    assert_eq!(env.core.sync(0).await.unwrap().len(), 1);
    // Another AI connected before has nothing.
    serve_pending(&env, &[search_request("r2", "c1", "ChatGPT", "anything")], &[]).await;
    assert_eq!(env.core.sync(0).await.unwrap().len(), 2);
}
