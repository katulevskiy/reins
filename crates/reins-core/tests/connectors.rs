//! Calls to the integrations besides Gmail, driven through a fake Telegram: listing, reading, searching and sending,
//! the approvals they need, the grants those leave, and how everything is logged.

mod common;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{FakeGoogle, FakeKeys, RecordingNotifier};
use reins_core::connector::{Connector, Item, Preview};
use reins_core::{
    ApprovalChoice, ApprovalKind, CoreConfig, CoreError, GmailStatus, GoogleTokenProvider, GrantScopeChoice, Notifier,
    ReinsCore, StandingGrant,
};
use reins_proto::connector::ConnectorCall;
use serde_json::{Value, json};
use wiremock::matchers::{method, path, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A Telegram that keeps its chats in memory.
#[derive(Default)]
struct FakeTelegram {
    chats: Mutex<Vec<(String, String)>>,
    messages: Mutex<BTreeMap<String, Vec<Item>>>,
    sent: Mutex<Vec<(String, String)>>,
    fail: Mutex<bool>,
}

impl FakeTelegram {
    fn with_chats() -> Arc<Self> {
        let t = Self::default();
        *t.chats.lock().unwrap() = vec![("100".into(), "Family".into()), ("200".into(), "Work".into())];
        let message = |chat: &str, label: &str, n: u32, from: &str, text: &str, sensitive: bool| Item {
            id: format!("{chat}:{n}"),
            resource: chat.into(),
            resource_label: label.into(),
            from: from.into(),
            snippet: text.chars().take(20).collect(),
            body: Some(text.into()),
            date: 1_700_000_000 + i64::from(n),
            sensitive,
            ..Item::default()
        };
        t.messages.lock().unwrap().insert(
            "100".into(),
            vec![
                message("100", "Family", 2, "Anna", "Dinner at eight?", false),
                message("100", "Family", 1, "Bob", "I am late", false),
            ],
        );
        t.messages.lock().unwrap().insert(
            "200".into(),
            vec![
                message("200", "Work", 1, "Boss", "Your code is 481516", true),
                message("200", "Work", 2, "Boss", "Report by Friday", false),
            ],
        );
        Arc::new(t)
    }

    fn chat(&self, wanted: &str) -> Option<(String, String)> {
        self.chats
            .lock()
            .unwrap()
            .iter()
            .find(|(id, title)| id == wanted || title.eq_ignore_ascii_case(wanted))
            .cloned()
    }
}

#[async_trait::async_trait]
impl Connector for FakeTelegram {
    fn service(&self) -> &'static str {
        "telegram"
    }

    async fn fetch(&self, _account: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
        if *self.fail.lock().unwrap() {
            return Err(CoreError::service("Telegram is having a bad day"));
        }
        match call.op.as_str() {
            "list_chats" => Ok(self
                .chats
                .lock()
                .unwrap()
                .iter()
                .map(|(id, title)| Item {
                    id: id.clone(),
                    resource: id.clone(),
                    resource_label: title.clone(),
                    title: title.clone(),
                    from: "chat".into(),
                    ..Item::default()
                })
                .collect()),
            "read" => {
                let (id, _) =
                    self.chat(call.str_arg("chat").unwrap()).ok_or_else(|| CoreError::service("no such chat"))?;
                Ok(self.messages.lock().unwrap().get(&id).cloned().unwrap_or_default())
            }
            "search" => Ok(self
                .messages
                .lock()
                .unwrap()
                .values()
                .flatten()
                .filter(|m| {
                    m.body
                        .as_deref()
                        .unwrap_or("")
                        .to_lowercase()
                        .contains(&call.str_arg("query").unwrap().to_lowercase())
                })
                .cloned()
                .collect()),
            other => Err(CoreError::service(format!("unknown {other}"))),
        }
    }

    async fn preview(&self, _account: &str, call: &ConnectorCall) -> Result<Preview, CoreError> {
        let (id, title) = self.chat(call.str_arg("chat").unwrap()).ok_or_else(|| CoreError::service("no such chat"))?;
        Ok(Preview {
            resource: id,
            resource_label: title.clone(),
            lines: vec![format!("To {title}"), call.str_arg("text").unwrap().to_owned()],
            ..Preview::default()
        })
    }

    async fn perform(&self, _account: &str, call: &ConnectorCall) -> Result<Value, CoreError> {
        if *self.fail.lock().unwrap() {
            return Err(CoreError::service("Telegram is having a bad day"));
        }
        let (id, _) = self.chat(call.str_arg("chat").unwrap()).unwrap();
        self.sent.lock().unwrap().push((id, call.str_arg("text").unwrap().to_owned()));
        Ok(json!({"sent": true, "message_id": self.sent.lock().unwrap().len()}))
    }

    async fn status(&self, _account: &str) -> GmailStatus {
        GmailStatus::Ready
    }
}

struct Env {
    server: MockServer,
    core: Arc<ReinsCore>,
    telegram: Arc<FakeTelegram>,
    _dir: tempfile::TempDir,
}

async fn env() -> Env {
    let server = MockServer::start().await;
    let gmail = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/identity/accounts/prelogin"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"kdf": 0, "kdfIterations": 5000})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/identity/connect/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "ACCESS", "refresh_token": "REFRESH", "expires_in": 7200})))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let telegram = FakeTelegram::with_chats();
    let cfg = CoreConfig {
        gmail_base: gmail.uri(),
        backoff_base: Duration::from_millis(1),
        ..CoreConfig::default()
    };
    let google: Arc<dyn GoogleTokenProvider> = Arc::new(FakeGoogle::new());
    let notifier: Arc<dyn Notifier> = Arc::new(RecordingNotifier::default());
    let core = ReinsCore::with_connectors(
        dir.path().to_str().unwrap(),
        &FakeKeys,
        google,
        notifier,
        cfg,
        vec![Arc::<FakeTelegram>::clone(&telegram)],
    )
    .unwrap();
    core.login(server.uri(), "me@example.com".to_owned(), "hunter2".to_owned(), None).await.unwrap();
    core.engine().register_account("telegram", "+15550100").unwrap();
    Env {
        server,
        core,
        telegram,
        _dir: dir,
    }
}

fn call_request(id: &str, conn: &str, op: &str, args: &Value) -> Value {
    json!({"v": 1, "id": id, "connection_id": conn, "connection_label": "Claude", "created_at": 100,
           "call": {"tool": "connector", "service": "telegram", "op": op, "args": args}})
}

async fn serve_pending(env: &Env, requests: &[Value]) {
    Mock::given(method("GET"))
        .and(path("/reins/api/pending"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"requests": requests, "pairings": []})))
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

async fn answers(env: &Env) -> Vec<Value> {
    env.server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| {
            r.method.as_str() == "POST" && r.url.path().ends_with("/response") && r.url.path().contains("/requests/")
        })
        .map(|r| serde_json::from_slice(&r.body).unwrap())
        .collect()
}

fn scope(resources: &[&str], any: bool) -> GrantScopeChoice {
    GrantScopeChoice {
        all_mail: any,
        selected_messages_only: false,
        sender_addresses: vec![],
        sender_domains: vec![],
        subject_pattern: None,
        recipient_addresses: vec![],
        recipient_domains: vec![],
        resources: resources.iter().map(|r| (*r).to_owned()).collect(),
        classes: vec![],
    }
}

fn hour(resources: &[&str]) -> Option<StandingGrant> {
    Some(StandingGrant {
        duration_secs: Some(3_600),
        max_uses: None,
        scope: scope(resources, false),
    })
}

fn choice(ids: &[&str], standing: Option<StandingGrant>) -> ApprovalChoice {
    ApprovalChoice {
        selected_message_ids: ids.iter().map(|s| (*s).to_owned()).collect(),
        standing,
    }
}

#[tokio::test]
async fn chats_are_listed_only_as_far_as_the_user_allows_and_then_remembered() {
    let env = env().await;
    serve_pending(&env, &[call_request("r1", "c1", "list_chats", &json!({"limit": 20}))]).await;
    let items = env.core.sync(0).await.unwrap();
    assert_eq!(
        (items[0].action.as_str(), items[0].op.as_str(), items[0].service.as_str(), items[0].count),
        ("list", "list_chats", "telegram", 2)
    );
    let view = env.core.approval_view("r1".to_owned()).await.unwrap();
    assert_eq!(view.kind, ApprovalKind::Fetch);
    assert_eq!(view.messages.iter().map(|m| m.subject.as_str()).collect::<Vec<_>>(), ["Family", "Work"]);
    assert_eq!(view.resources.iter().map(|r| r.label.as_str()).collect::<Vec<_>>(), ["Family", "Work"]);
    assert_eq!(view.account.as_deref(), Some("+15550100"));
    assert!(answers(&env).await.is_empty(), "nothing is told before the user decides");

    // Only Family is shown, and remembered for an hour.
    env.core.approve("r1".to_owned(), choice(&["100"], hour(&["100"]))).await.unwrap();
    let told = answers(&env).await;
    assert_eq!(told[0]["result"]["kind"], "connector");
    assert_eq!(told[0]["result"]["data"]["items"].as_array().unwrap().len(), 1);
    assert_eq!(told[0]["result"]["data"]["items"][0]["title"], "Family");
    let grant = env.core.grants().await.unwrap().remove(0);
    assert_eq!(
        (grant.action.as_str(), grant.service.as_str(), grant.summary.as_str()),
        ("list", "telegram", "List Telegram: Family")
    );
    let entry = &env.core.activity(1).await.unwrap()[0];
    assert_eq!(
        (entry.action.as_str(), entry.op.as_str(), entry.service.as_str(), entry.count),
        ("list", "list_chats", "telegram", 1)
    );

    // The next listing: Family is already covered (ticked), Work still needs the user.
    serve_pending(&env, &[call_request("r2", "c1", "list_chats", &json!({"limit": 20}))]).await;
    let items = env.core.sync(0).await.unwrap();
    assert_eq!(items[0].count, 2);
    let view = env.core.approval_view("r2".to_owned()).await.unwrap();
    assert_eq!(
        view.messages.iter().map(|m| (m.subject.as_str(), m.covered_by_grant)).collect::<Vec<_>>(),
        [("Family", true), ("Work", false)]
    );
    env.core.approve("r2".to_owned(), choice(&["200"], None)).await.unwrap();
    assert_eq!(answers(&env).await[1]["result"]["data"]["items"].as_array().unwrap().len(), 2, "covered and picked");
}

#[tokio::test]
async fn reading_a_chat_shows_the_messages_to_tick_and_a_grant_answers_the_next_read() {
    let env = env().await;
    serve_pending(&env, &[call_request("r1", "c1", "read", &json!({"chat": "family", "limit": 20}))]).await;
    let items = env.core.sync(0).await.unwrap();
    assert_eq!((items[0].action.as_str(), items[0].count), ("read", 2));
    let view = env.core.approval_view("r1".to_owned()).await.unwrap();
    assert_eq!(view.messages[0].from, "Anna");
    assert_eq!(view.messages[0].snippet, "Dinner at eight?");
    assert!(!view.no_standing);
    assert!(matches!(env.core.approve("r1".to_owned(), choice(&[], None)).await, Err(CoreError::Invalid { .. })));
    assert!(matches!(
        env.core.approve("r1".to_owned(), choice(&["999:9"], None)).await,
        Err(CoreError::Invalid { .. })
    ));
    // The standing permission can only cover what the request touched.
    assert!(matches!(
        env.core.approve("r1".to_owned(), choice(&["100:2"], hour(&["200"]))).await,
        Err(CoreError::Invalid { .. })
    ));

    env.core.approve("r1".to_owned(), choice(&["100:2"], hour(&["100"]))).await.unwrap();
    let told = answers(&env).await;
    assert_eq!(told[0]["result"]["data"]["items"][0]["text"], "Dinner at eight?");
    assert_eq!(told[0]["result"]["data"]["items"][0]["in"], json!({"id": "100", "name": "Family"}));
    assert_eq!(told[0]["result"]["data"]["items"].as_array().unwrap().len(), 1, "only the ticked message");
    let entry = &env.core.activity(1).await.unwrap()[0];
    assert_eq!(entry.info.messages[0].text, "Dinner at eight?", "the log keeps what was shared");

    // Now the whole chat is covered for an hour; another chat is not.
    serve_pending(&env, &[call_request("r2", "c1", "read", &json!({"chat": "Family"}))]).await;
    assert!(env.core.sync(0).await.unwrap().is_empty(), "answered by the grant");
    assert_eq!(answers(&env).await[1]["result"]["data"]["items"].as_array().unwrap().len(), 2);
    assert!(env.core.activity(1).await.unwrap()[0].grant_id.is_some());
    serve_pending(&env, &[call_request("r3", "c1", "read", &json!({"chat": "Work"}))]).await;
    assert_eq!(env.core.sync(0).await.unwrap().len(), 1);
    // Another AI is not covered either.
    serve_pending(&env, &[call_request("r4", "c2", "read", &json!({"chat": "Family"}))]).await;
    assert_eq!(env.core.sync(0).await.unwrap().len(), 2);
}

#[tokio::test]
async fn anything_sensitive_is_never_covered_by_a_grant_and_starts_unticked() {
    let env = env().await;
    // A grant covering the whole Work chat exists.
    serve_pending(&env, &[call_request("r0", "c1", "read", &json!({"chat": "Work"}))]).await;
    env.core.sync(0).await.unwrap();
    let view = env.core.approval_view("r0".to_owned()).await.unwrap();
    assert_eq!(view.messages.iter().map(|m| m.sensitive).collect::<Vec<_>>(), [true, false]);
    env.core.approve("r0".to_owned(), choice(&["200:2"], hour(&["200"]))).await.unwrap();
    // Reading again: the code message is still held back for the user, though the chat is covered.
    serve_pending(&env, &[call_request("r1", "c1", "read", &json!({"chat": "Work"}))]).await;
    assert_eq!(env.core.sync(0).await.unwrap().len(), 1, "the sensitive message needs a decision");
    let view = env.core.approval_view("r1".to_owned()).await.unwrap();
    assert_eq!(
        view.messages.iter().map(|m| (m.sensitive, m.covered_by_grant)).collect::<Vec<_>>(),
        [(true, false), (false, true)]
    );
    env.core.approve("r1".to_owned(), choice(&[], None)).await.unwrap();
    let told = answers(&env).await;
    assert_eq!(told[1]["result"]["data"]["items"].as_array().unwrap().len(), 1, "only the covered message");
}

#[tokio::test]
async fn when_everything_found_is_sensitive_nothing_can_be_remembered() {
    let env = env().await;
    env.telegram.messages.lock().unwrap().insert(
        "200".into(),
        vec![Item {
            id: "200:9".into(),
            resource: "200".into(),
            resource_label: "Work".into(),
            snippet: "code 1".into(),
            sensitive: true,
            ..Item::default()
        }],
    );
    serve_pending(&env, &[call_request("r1", "c1", "read", &json!({"chat": "Work"}))]).await;
    env.core.sync(0).await.unwrap();
    assert!(env.core.approval_view("r1".to_owned()).await.unwrap().no_standing);
    assert!(matches!(
        env.core.approve("r1".to_owned(), choice(&["200:9"], hour(&["200"]))).await,
        Err(CoreError::Invalid { .. })
    ));
    env.core.approve("r1".to_owned(), choice(&["200:9"], None)).await.unwrap();
    assert!(env.core.grants().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_message_is_previewed_sent_once_and_remembered_for_that_chat_only() {
    let env = env().await;
    let send = |id: &str, chat: &str, text: &str| call_request(id, "c1", "send", &json!({"chat": chat, "text": text}));
    serve_pending(&env, &[send("r1", "Family", "On my way")]).await;
    let items = env.core.sync(0).await.unwrap();
    assert_eq!((items[0].action.as_str(), items[0].op.as_str()), ("send", "send"));
    let view = env.core.approval_view("r1".to_owned()).await.unwrap();
    assert_eq!(view.kind, ApprovalKind::Write);
    assert_eq!(view.preview, ["To Family", "On my way"]);
    assert!(env.telegram.sent.lock().unwrap().is_empty(), "nothing is sent before the user agrees");

    env.core.approve("r1".to_owned(), choice(&[], hour(&["100"]))).await.unwrap();
    assert_eq!(*env.telegram.sent.lock().unwrap(), [("100".to_owned(), "On my way".to_owned())]);
    assert_eq!(answers(&env).await[0]["result"]["data"]["sent"], true);
    let grant = env.core.grants().await.unwrap().remove(0);
    assert_eq!((grant.action.as_str(), grant.summary.as_str()), ("write", "Write to Telegram: Family"));
    let entry = &env.core.activity(1).await.unwrap()[0];
    assert_eq!((entry.action.as_str(), entry.outcome.as_str(), entry.op.as_str()), ("send", "sent", "send"));

    serve_pending(&env, &[send("r2", "Family", "Again")]).await;
    assert!(env.core.sync(0).await.unwrap().is_empty(), "covered for this chat");
    assert_eq!(env.telegram.sent.lock().unwrap().len(), 2);
    serve_pending(&env, &[send("r3", "Work", "Boss?")]).await;
    assert_eq!(env.core.sync(0).await.unwrap().len(), 1, "another chat needs its own decision");
    assert_eq!(env.telegram.sent.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn a_failed_send_keeps_the_request_and_gives_a_used_grant_back() {
    let env = env().await;
    serve_pending(&env, &[call_request("r1", "c1", "send", &json!({"chat": "Family", "text": "hi"}))]).await;
    env.core.sync(0).await.unwrap();
    *env.telegram.fail.lock().unwrap() = true;
    assert!(env.core.approve("r1".to_owned(), choice(&[], None)).await.is_err());
    assert_eq!(env.core.pending().await.unwrap().len(), 1, "still waiting for another try");
    *env.telegram.fail.lock().unwrap() = false;
    env.core.approve("r1".to_owned(), choice(&[], None)).await.unwrap();
    assert_eq!(env.telegram.sent.lock().unwrap().len(), 1);

    // With a one-use grant, a send that fails does not spend it.
    let one = Some(StandingGrant {
        duration_secs: Some(3_600),
        max_uses: Some(1),
        scope: scope(&["100"], false),
    });
    serve_pending(&env, &[call_request("r2", "c1", "send", &json!({"chat": "Family", "text": "again"}))]).await;
    env.core.sync(0).await.unwrap();
    env.core.approve("r2".to_owned(), choice(&[], one)).await.unwrap();
    *env.telegram.fail.lock().unwrap() = true;
    serve_pending(&env, &[call_request("r3", "c1", "send", &json!({"chat": "Family", "text": "third"}))]).await;
    // The grant was spent by the send that created it? No: creating a grant does not use it.
    assert!(env.core.sync(0).await.unwrap().is_empty(), "the grant answers, and the failure goes to the AI");
    let told = answers(&env).await;
    assert_eq!(told.last().unwrap()["outcome"], "error");
    assert!(told.last().unwrap()["message"].as_str().unwrap().contains("bad day"));
    assert_eq!(env.core.grants().await.unwrap()[0].uses, 0, "the use was given back");
}

#[tokio::test]
async fn a_search_across_chats_is_covered_only_when_every_chat_it_touches_is() {
    let env = env().await;
    serve_pending(&env, &[call_request("r0", "c1", "read", &json!({"chat": "Family"}))]).await;
    env.core.sync(0).await.unwrap();
    env.core.approve("r0".to_owned(), choice(&["100:1"], hour(&["100"]))).await.unwrap();
    // "dinner" is only in Family: covered. "report" is only in Work: not.
    serve_pending(&env, &[call_request("r1", "c1", "search", &json!({"query": "dinner"}))]).await;
    assert!(env.core.sync(0).await.unwrap().is_empty());
    serve_pending(&env, &[call_request("r2", "c1", "search", &json!({"query": "report"}))]).await;
    assert_eq!(env.core.sync(0).await.unwrap().len(), 1);
    // Nothing found: nothing to ask.
    serve_pending(&env, &[call_request("r3", "c1", "search", &json!({"query": "zebra"}))]).await;
    assert_eq!(env.core.sync(0).await.unwrap().len(), 1, "only the earlier one still waits");
    assert_eq!(answers(&env).await.last().unwrap()["result"]["data"]["items"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn a_refusal_lists_what_would_have_been_shared_and_errors_reach_the_ai() {
    let env = env().await;
    serve_pending(&env, &[call_request("r1", "c1", "read", &json!({"chat": "Family"}))]).await;
    env.core.sync(0).await.unwrap();
    env.core.deny("r1".to_owned()).await.unwrap();
    assert_eq!(answers(&env).await[0]["outcome"], "denied");
    let entry = &env.core.activity(1).await.unwrap()[0];
    assert_eq!((entry.outcome.as_str(), entry.service.as_str(), entry.count), ("denied", "telegram", 2));
    assert_eq!(entry.info.messages.len(), 2);

    *env.telegram.fail.lock().unwrap() = true;
    serve_pending(&env, &[call_request("r2", "c1", "read", &json!({"chat": "Family"}))]).await;
    assert!(env.core.sync(0).await.unwrap().is_empty());
    let last = answers(&env).await.pop().unwrap();
    assert_eq!(last["outcome"], "error");
    assert!(last["message"].as_str().unwrap().contains("bad day"));
    let entry = &env.core.activity(1).await.unwrap()[0];
    assert_eq!((entry.outcome.as_str(), entry.action.as_str(), entry.op.as_str()), ("error", "read", "read"));
}

#[tokio::test]
async fn accounts_are_chosen_by_name_and_never_revealed_by_errors() {
    let env = env().await;
    env.core.engine().register_account("telegram", "+15550199").unwrap();
    serve_pending(&env, &[call_request("r1", "c1", "read", &json!({"chat": "Family"}))]).await;
    assert!(env.core.sync(0).await.unwrap().is_empty());
    let message = answers(&env).await[0]["message"].as_str().unwrap().to_owned();
    assert!(message.contains("reins_list_accounts") && !message.contains("+1555"), "{message}");

    let mut named = call_request("r2", "c1", "read", &json!({"chat": "Family"}));
    named["account"] = json!("+15550199");
    serve_pending(&env, &[named]).await;
    let items = env.core.sync(0).await.unwrap();
    assert_eq!(items[0].account.as_deref(), Some("+15550199"));

    let mut wrong = call_request("r3", "c1", "read", &json!({"chat": "Family"}));
    wrong["account"] = json!("+19999999");
    serve_pending(&env, &[wrong]).await;
    env.core.sync(0).await.unwrap();
    let message = answers(&env).await[1]["message"].as_str().unwrap().to_owned();
    assert!(message.contains("not connected") && !message.contains("+1555"), "{message}");
}

#[tokio::test]
async fn an_integration_that_is_not_there_or_not_connected_says_so() {
    let env = env().await;
    let github = json!({"v": 1, "id": "r1", "connection_id": "c1", "connection_label": "Claude", "created_at": 100,
        "call": {"tool": "connector", "service": "github", "op": "list_repos", "args": {"limit": 20}}});
    serve_pending(&env, &[github]).await;
    assert!(env.core.sync(0).await.unwrap().is_empty());
    let said = answers(&env).await[0]["message"].as_str().unwrap().to_owned();
    assert!(said.contains("not connected"), "{said}");
    let vault = json!({"v": 1, "id": "r0", "connection_id": "c1", "connection_label": "Claude", "created_at": 100,
        "call": {"tool": "connector", "service": "sms", "op": "list_threads", "args": {}}});
    serve_pending(&env, &[vault]).await;
    env.core.sync(0).await.unwrap();
    let said = answers(&env).await[1]["message"].as_str().unwrap().to_owned();
    assert!(said.contains("not available"), "{said}");
    let bad = json!({"v": 1, "id": "r2", "connection_id": "c1", "connection_label": "Claude", "created_at": 100,
        "call": {"tool": "connector", "service": "telegram", "op": "read", "args": {"chat": "x", "wat": 1}}});
    serve_pending(&env, &[bad]).await;
    env.core.sync(0).await.unwrap();
    assert_eq!(answers(&env).await[2]["outcome"], "error");
}

#[tokio::test]
async fn a_read_approved_after_the_ai_stopped_waiting_leaves_a_one_time_pass_for_the_chat() {
    let env = env().await;
    let mut late = call_request("r1", "c1", "read", &json!({"chat": "Family"}));
    late["wait_until"] = json!(1);
    serve_pending(&env, &[late]).await;
    env.core.sync(0).await.unwrap();
    env.core.approve("r1".to_owned(), choice(&["100:2"], None)).await.unwrap();
    let pass = env.core.grants().await.unwrap().remove(0);
    assert_eq!((pass.origin.as_str(), pass.max_uses, pass.action.as_str()), ("retry", Some(1), "read"));
    serve_pending(&env, &[call_request("r2", "c1", "read", &json!({"chat": "Family"}))]).await;
    assert!(env.core.sync(0).await.unwrap().is_empty(), "asking again goes through");
}

#[tokio::test]
async fn everything_of_an_integration_can_be_allowed_for_a_short_time_but_not_for_writing() {
    let env = env().await;
    serve_pending(&env, &[call_request("r1", "c1", "read", &json!({"chat": "Family"}))]).await;
    env.core.sync(0).await.unwrap();
    let everything = |secs| {
        Some(StandingGrant {
            duration_secs: Some(secs),
            max_uses: None,
            scope: scope(&[], true),
        })
    };
    assert!(
        matches!(
            env.core.approve("r1".to_owned(), choice(&["100:2"], everything(30 * 86_400))).await,
            Err(CoreError::Invalid { .. })
        ),
        "too long"
    );
    env.core.approve("r1".to_owned(), choice(&["100:2"], everything(86_400))).await.unwrap();
    serve_pending(&env, &[call_request("r2", "c1", "read", &json!({"chat": "Work"}))]).await;
    let pending = env.core.sync(0).await.unwrap();
    assert_eq!(pending.len(), 1, "the sensitive code still needs a decision, though every chat is allowed");
    serve_pending(&env, &[call_request("r3", "c1", "read", &json!({"chat": "Family"}))]).await;
    assert!(env.core.sync(0).await.unwrap().len() == 1, "Family is answered, Work still waits");
    serve_pending(&env, &[call_request("r4", "c1", "send", &json!({"chat": "Family", "text": "x"}))]).await;
    env.core.sync(0).await.unwrap();
    assert!(
        matches!(
            env.core.approve("r4".to_owned(), choice(&[], everything(3_600))).await,
            Err(CoreError::Invalid { .. })
        ),
        "writing cannot be allowed everywhere"
    );
}
