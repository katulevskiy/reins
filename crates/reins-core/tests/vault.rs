//! The password vault: a mock Vaultwarden serves a vault encrypted the way Bitwarden clients do, and the phone unlocks
//! it once with the master password, then hands over one field at a time, each time with the user's approval.

mod common;

use std::sync::Arc;

use common::{FakeGoogle, FakeKeys, RecordingNotifier};
use reins_core::crypto::{Kdf, VaultKey, master_key};
use reins_core::{
    ApprovalChoice, CoreConfig, CoreError, GmailStatus, GoogleTokenProvider, GrantScopeChoice, Notifier, ReinsCore,
    StandingGrant,
};
use serde_json::{Value, json};
use wiremock::matchers::{header, method, path, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

const PASSWORD: &str = "correct horse battery";
const EMAIL: &str = "me@example.com";

struct Env {
    server: MockServer,
    core: Arc<ReinsCore>,
    _dir: tempfile::TempDir,
}

fn text(key: &VaultKey, s: &str) -> Value {
    json!(key.encrypt(s.as_bytes()).unwrap())
}

async fn env() -> Env {
    let server = MockServer::start().await;
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

    let master = master_key(
        PASSWORD,
        EMAIL,
        Kdf::Pbkdf2 {
            iterations: 5000,
        },
    )
    .unwrap();
    let user = VaultKey::from_bytes(&[7u8; 64]).unwrap();
    let wrapped = master.stretch().encrypt(&user.to_bytes()).unwrap();
    let own = VaultKey::from_bytes(&[9u8; 64]).unwrap();
    let ciphers = json!([
        {"id": "git", "type": 1, "name": text(&user, "GitHub"), "key": null, "deletedDate": null, "organizationId": null,
         "login": {"username": text(&user, "octo"), "password": text(&user, "hunter2!"),
                   "totp": text(&user, "otpauth://totp/GitHub:octo?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ&digits=6"),
                   "uris": [{"uri": text(&user, "https://github.com/login"), "match": null}]}},
        {"id": "bank", "type": 1, "name": text(&own, "Bank"), "key": null,
         "deletedDate": null, "organizationId": null,
         "login": {"username": text(&own, "anna"), "password": text(&own, "s3cret"), "totp": null, "uris": null}},
        {"id": "gone", "type": 1, "name": text(&user, "GitHub old"), "key": null, "deletedDate": "2026-01-01T00:00:00Z",
         "organizationId": null, "login": {"username": null, "password": null, "totp": null, "uris": null}},
        {"id": "org", "type": 1, "name": text(&user, "GitHub shared"), "key": null, "deletedDate": null,
         "organizationId": "o1", "login": {"username": null, "password": null, "totp": null, "uris": null}},
        {"id": "note", "type": 2, "name": text(&user, "GitHub note"), "key": null, "deletedDate": null,
         "organizationId": null, "login": null}
    ]);
    // The bank item has a key of its own, itself encrypted with the user key.
    let mut ciphers = ciphers;
    ciphers[1]["key"] = json!(user.encrypt(&own.to_bytes()).unwrap());
    Mock::given(method("GET"))
        .and(path("/api/sync"))
        .and(header("authorization", "Bearer ACCESS"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"profile": {"key": wrapped}, "ciphers": ciphers})),
        )
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let google: Arc<dyn GoogleTokenProvider> = Arc::new(FakeGoogle::new());
    let notifier: Arc<dyn Notifier> = Arc::new(RecordingNotifier::default());
    let core = ReinsCore::with_connectors(
        dir.path().to_str().unwrap(),
        &FakeKeys,
        google,
        notifier,
        CoreConfig::default(),
        vec![],
    )
    .unwrap();
    core.login(server.uri(), EMAIL.to_owned(), PASSWORD.to_owned(), None).await.unwrap();
    Env {
        server,
        core,
        _dir: dir,
    }
}

fn request(id: &str, op: &str, args: &Value) -> Value {
    json!({"v": 1, "id": id, "connection_id": "c1", "connection_label": "Claude", "created_at": 100,
           "call": {"tool": "connector", "service": "vault", "op": op, "args": args}})
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
            r.method.as_str() == "POST" && r.url.path().contains("/requests/") && r.url.path().ends_with("/response")
        })
        .map(|r| serde_json::from_slice(&r.body).unwrap())
        .collect()
}

fn choice(ids: &[&str], standing: Option<StandingGrant>) -> ApprovalChoice {
    ApprovalChoice {
        selected_message_ids: ids.iter().map(|s| (*s).to_owned()).collect(),
        standing,
    }
}

#[tokio::test]
async fn the_master_password_unlocks_the_vault_once_and_a_wrong_one_does_not() {
    let env = env().await;
    assert!(matches!(env.core.add_token_account("vault".into(), "wrong".into()).await, Err(CoreError::Invalid { .. })));
    assert!(env.core.accounts().await.unwrap().is_empty());
    let added = env.core.add_token_account("vault".into(), PASSWORD.into()).await.unwrap();
    assert_eq!((added.service.as_str(), added.account.as_str()), ("vault", EMAIL));
    assert_eq!(env.core.service_account_status("vault".into(), EMAIL.into()).await, GmailStatus::Ready);
    env.core.remove_service_account("vault".into(), EMAIL.into()).await.unwrap();
    assert_eq!(env.core.service_account_status("vault".into(), EMAIL.into()).await, GmailStatus::NeedsConsent);
}

#[tokio::test]
async fn searching_shows_names_and_addresses_but_no_secrets_and_skips_deleted_and_shared_items() {
    let env = env().await;
    env.core.add_token_account("vault".into(), PASSWORD.into()).await.unwrap();
    serve_pending(&env, &[request("r1", "search", &json!({"query": "github"}))]).await;
    let pending = env.core.sync(0).await.unwrap();
    assert_eq!((pending[0].action.as_str(), pending[0].count), ("search", 1));
    let view = env.core.approval_view("r1".to_owned()).await.unwrap();
    assert_eq!(view.messages[0].subject, "GitHub");
    assert_eq!(view.messages[0].from, "octo");
    assert_eq!(view.messages[0].snippet, "github.com");
    env.core.approve("r1".to_owned(), choice(&["git"], None)).await.unwrap();
    let told = answers(&env).await;
    let item = &told[0]["result"]["data"]["items"][0];
    assert_eq!(
        (item["id"].as_str(), item["title"].as_str(), item["from"].as_str()),
        (Some("git"), Some("GitHub"), Some("octo"))
    );
    let everything = told[0].to_string();
    assert!(!everything.contains("hunter2") && !everything.contains("s3cret"), "{everything}");

    serve_pending(&env, &[request("r2", "search", &json!({"query": "bank"}))]).await;
    env.core.sync(0).await.unwrap();
    let view = env.core.approval_view("r2".to_owned()).await.unwrap();
    assert_eq!(
        (view.messages[0].subject.as_str(), view.messages[0].from.as_str()),
        ("Bank", "anna"),
        "an item with its own key"
    );
}

#[tokio::test]
async fn a_field_is_handed_over_only_after_approval_every_time_and_never_written_to_the_activity() {
    let env = env().await;
    env.core.add_token_account("vault".into(), PASSWORD.into()).await.unwrap();
    serve_pending(&env, &[request("r1", "get", &json!({"item": "git", "field": "password"}))]).await;
    let pending = env.core.sync(0).await.unwrap();
    assert_eq!(pending.len(), 1);
    let view = env.core.approval_view("r1".to_owned()).await.unwrap();
    assert!(view.no_standing, "a password cannot be remembered");
    assert_eq!(view.messages[0].snippet, "Password for GitHub", "the user sees what, not the value");
    assert!(!view.messages[0].covered_by_grant && view.messages[0].sensitive);
    assert!(!format!("{view:?}").contains("hunter2"));

    // Asking for a standing permission alongside is refused.
    let scope = GrantScopeChoice {
        all_mail: false,
        selected_messages_only: false,
        sender_addresses: vec![],
        sender_domains: vec![],
        subject_pattern: None,
        recipient_addresses: vec![],
        recipient_domains: vec![],
        resources: vec!["git".into()],
        classes: vec![],
    };
    let standing = StandingGrant {
        duration_secs: Some(3_600),
        max_uses: None,
        scope,
    };
    assert!(env.core.approve("r1".to_owned(), choice(&["git:password"], Some(standing))).await.is_err());
    assert!(env.core.grants().await.unwrap().is_empty());

    env.core.approve("r1".to_owned(), choice(&["git:password"], None)).await.unwrap();
    let told = answers(&env).await;
    assert_eq!(told[0]["result"]["data"]["items"][0]["text"], "hunter2!");
    let entry = &env.core.activity(1).await.unwrap()[0];
    assert!(!format!("{entry:?}").contains("hunter2"), "the log never keeps a password: {entry:?}");
    assert!(env.core.grants().await.unwrap().is_empty());

    serve_pending(&env, &[request("r2", "get", &json!({"item": "git", "field": "password"}))]).await;
    assert_eq!(env.core.sync(0).await.unwrap().len(), 1, "asked again every time");
}

#[tokio::test]
async fn usernames_and_one_time_codes_are_fetched_and_mistakes_are_explained() {
    let env = env().await;
    env.core.add_token_account("vault".into(), PASSWORD.into()).await.unwrap();
    serve_pending(&env, &[request("r1", "get", &json!({"item": "git", "field": "totp"}))]).await;
    env.core.sync(0).await.unwrap();
    env.core.approve("r1".to_owned(), choice(&["git:totp"], None)).await.unwrap();
    let told = answers(&env).await;
    let item = &told[0]["result"]["data"]["items"][0];
    let code = item["text"].as_str().unwrap();
    assert!(code.len() == 6 && code.bytes().all(|b| b.is_ascii_digit()), "{code}");
    assert!(item["valid_for_seconds"].as_u64().unwrap() <= 30);

    for (id, args, needle) in [
        ("r2", json!({"item": "bank", "field": "totp"}), "no one-time code"),
        ("r3", json!({"item": "nope", "field": "password"}), "No login with that id"),
        ("r4", json!({"item": "gone", "field": "password"}), "No login with that id"),
        ("r5", json!({"item": "org", "field": "password"}), "No login with that id"),
    ] {
        serve_pending(&env, &[request(id, "get", &args)]).await;
        assert!(env.core.sync(0).await.unwrap().is_empty());
        let told = answers(&env).await;
        let message = told.last().unwrap()["message"].as_str().unwrap().to_owned();
        assert!(message.contains(needle), "{id}: {message}");
    }
}
