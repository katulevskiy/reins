//! Real exported calls exercise the local account boundary and encrypted network transport together.
mod common;
use data_encoding::BASE64URL_NOPAD;
use reins_core::crypto::{Kdf, master_key};
use reins_core::{CoreConfig, CoreError, GoogleTokenProvider, Notifier, ReinsCore};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

fn token(email: &str) -> String {
    format!("h.{}.s", BASE64URL_NOPAD.encode(json!({"sub":email,"email":email}).to_string().as_bytes()))
}
fn owner(request: &Request) -> String {
    let auth = request.headers.get("authorization").unwrap().to_str().unwrap();
    let payload = auth.strip_prefix("Bearer ").unwrap().split('.').nth(1).unwrap();
    serde_json::from_slice::<Value>(&BASE64URL_NOPAD.decode(payload.as_bytes()).unwrap()).unwrap()["sub"]
        .as_str()
        .unwrap()
        .to_owned()
}
struct Identity;
impl Respond for Identity {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let form = url::form_urlencoded::parse(&request.body).into_owned().collect::<HashMap<_, _>>();
        let email = &form["username"];
        ResponseTemplate::new(200)
            .set_body_json(json!({"access_token":token(email),"refresh_token":"REFRESH","expires_in":7200}))
    }
}
struct VaultProfile;
impl Respond for VaultProfile {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let email = owner(request);
        let key = master_key(
            "pw",
            &email,
            Kdf::Pbkdf2 {
                iterations: 5000,
            },
        )
        .unwrap()
        .stretch()
        .encrypt(&[7; 64])
        .unwrap();
        ResponseTemplate::new(200).set_body_json(json!({"profile":{"id":email,"key":key}}))
    }
}
#[derive(Clone, Default)]
struct Cloud(Arc<Mutex<HashMap<String, Value>>>);
impl Respond for Cloud {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let owner = owner(request);
        let mut states = self.0.lock().unwrap();
        let state = states.entry(owner).or_insert_with(|| json!({"revision":0,"ciphertext":null}));
        if request.method.as_str() == "PUT" {
            let update: Value = serde_json::from_slice(&request.body).unwrap();
            if update["revision"] != state["revision"] {
                return ResponseTemplate::new(409);
            }
            *state = json!({"revision":state["revision"].as_u64().unwrap()+1,"ciphertext":update["ciphertext"]});
        }
        ResponseTemplate::new(200).set_body_json(state.clone())
    }
}
fn core(dir: &std::path::Path) -> Arc<ReinsCore> {
    let google: Arc<dyn GoogleTokenProvider> = Arc::new(common::FakeGoogle::new());
    let notifier: Arc<dyn Notifier> = Arc::new(common::RecordingNotifier::default());
    ReinsCore::with_config(dir.to_str().unwrap(), &common::FakeKeys, google, notifier, CoreConfig::default()).unwrap()
}

#[tokio::test]
async fn unavailable_browser_logout_still_locks_local_account_and_retires_old_calls() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/identity/accounts/prelogin"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"kdf":0,"kdfIterations":5000})))
        .mount(&server)
        .await;
    Mock::given(method("POST")).and(path("/identity/connect/token")).respond_with(Identity).mount(&server).await;
    Mock::given(method("GET")).and(path("/api/sync")).respond_with(VaultProfile).mount(&server).await;
    Mock::given(path("/reins/api/account-state")).respond_with(Cloud::default()).mount(&server).await;
    Mock::given(method("POST"))
        .and(path("/reins/api/logout"))
        .respond_with(ResponseTemplate::new(503))
        .expect(1)
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let app = core(dir.path());
    app.login(server.uri(), "alice@example.com".to_owned(), "pw".to_owned(), None).await.unwrap();
    let old = app.engine();
    old.register_account("github", "alice-private-integration").unwrap();
    assert_eq!(app.logout_with_browser().await.unwrap(), None);
    assert!(app.session().await.is_none());
    assert_eq!(old.register_account("github", "late").unwrap_err(), CoreError::NotLoggedIn);
    assert_eq!(app.accounts().await.unwrap().len(), 0);
}
#[tokio::test]
async fn switching_accounts_locks_previous_runtime_and_restores_only_own_integrations_on_each_device() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/identity/accounts/prelogin"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"kdf":0,"kdfIterations":5000})))
        .mount(&server)
        .await;
    Mock::given(method("POST")).and(path("/identity/connect/token")).respond_with(Identity).mount(&server).await;
    Mock::given(method("GET")).and(path("/api/sync")).respond_with(VaultProfile).mount(&server).await;
    let cloud = Cloud::default();
    Mock::given(path("/reins/api/account-state")).respond_with(cloud.clone()).mount(&server).await;
    let dir = tempfile::tempdir().unwrap();
    let a = core(dir.path());
    a.login(server.uri(), "alice@example.com".to_owned(), "pw".to_owned(), None).await.unwrap();
    let retired = a.engine();
    retired.register_account("github", "alice-private-integration").unwrap();
    assert!(a.accounts().await.unwrap().iter().any(|a| a.account == "alice-private-integration"));
    a.logout().await.unwrap();
    assert_eq!(retired.register_account("github", "late-result").unwrap_err(), CoreError::NotLoggedIn);
    assert_eq!(a.accounts().await.unwrap().len(), 0);
    a.login(server.uri(), "bob@example.com".to_owned(), "pw".to_owned(), None).await.unwrap();
    assert!(!a.accounts().await.unwrap().iter().any(|a| a.account.contains("alice")));
    a.engine().register_account("github", "bob-private-integration").unwrap();
    a.accounts().await.unwrap(); // commits the encrypted snapshot to the network mock
    a.logout().await.unwrap();
    a.login(server.uri(), "alice@example.com".to_owned(), "pw".to_owned(), None).await.unwrap();
    let accounts = a.accounts().await.unwrap();
    assert!(accounts.iter().any(|a| a.account == "alice-private-integration"));
    assert!(!accounts.iter().any(|a| a.account.contains("bob") || a.account == "late-result"));
    let second = tempfile::tempdir().unwrap();
    let b = core(second.path());
    b.login(server.uri(), "alice@example.com".to_owned(), "pw".to_owned(), None).await.unwrap();
    assert!(b.accounts().await.unwrap().iter().any(|a| a.account == "alice-private-integration"));
    for state in cloud.0.lock().unwrap().values() {
        let ciphertext = state["ciphertext"].as_str().unwrap();
        let bytes = BASE64URL_NOPAD.decode(ciphertext.as_bytes()).unwrap();
        assert!(!bytes.windows("private-integration".len()).any(|w| w == b"private-integration"));
    }
}

#[tokio::test]
async fn a_slow_upload_neither_delays_calls_nor_blocks_logout() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/identity/accounts/prelogin"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"kdf":0,"kdfIterations":5000})))
        .mount(&server)
        .await;
    Mock::given(method("POST")).and(path("/identity/connect/token")).respond_with(Identity).mount(&server).await;
    Mock::given(method("GET")).and(path("/api/sync")).respond_with(VaultProfile).mount(&server).await;
    Mock::given(method("GET"))
        .and(path("/reins/api/account-state"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"revision":0,"ciphertext":null})))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let app = core(dir.path());
    app.login(server.uri(), "alice@example.com".to_owned(), "pw".to_owned(), None).await.unwrap();
    let started = Arc::new(tokio::sync::Notify::new());
    let notify = Arc::clone(&started);
    Mock::given(method("PUT"))
        .and(path("/reins/api/account-state"))
        .respond_with(move |_: &Request| {
            notify.notify_one();
            ResponseTemplate::new(200).set_delay(std::time::Duration::from_secs(30))
        })
        .mount(&server)
        .await;
    // The upload runs in the background: the call that started it answers without waiting for the network.
    let answered = tokio::time::timeout(std::time::Duration::from_secs(1), app.accounts()).await.unwrap();
    assert!(answered.is_ok());
    tokio::time::timeout(std::time::Duration::from_secs(3), started.notified()).await.unwrap();
    // Logout gives the stuck upload a bounded wait, then retires it with the account.
    tokio::time::timeout(std::time::Duration::from_secs(5), app.logout()).await.unwrap().unwrap();
    assert_eq!(app.accounts().await.unwrap().len(), 0);
}

/// Identity that issues a new access token (`n` counts them) for every sign-in and refresh of Alice.
#[derive(Clone, Default)]
struct CountingIdentity(Arc<Mutex<u32>>);
impl Respond for CountingIdentity {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        let mut n = self.0.lock().unwrap();
        *n += 1;
        let claims = json!({"sub": "alice@example.com", "email": "alice@example.com", "n": *n});
        let token = format!("h.{}.s", BASE64URL_NOPAD.encode(claims.to_string().as_bytes()));
        ResponseTemplate::new(200)
            .set_body_json(json!({"access_token": token, "refresh_token": "REFRESH", "expires_in": 7200}))
    }
}

/// Alice signed in on a fresh phone, with an integration kept in her encrypted account file.
async fn alice(server: &MockServer, dir: &std::path::Path) -> Arc<ReinsCore> {
    Mock::given(method("POST"))
        .and(path("/identity/accounts/prelogin"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"kdf":0,"kdfIterations":5000})))
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path("/identity/connect/token"))
        .respond_with(CountingIdentity::default())
        .mount(server)
        .await;
    Mock::given(method("GET")).and(path("/api/sync")).respond_with(VaultProfile).mount(server).await;
    Mock::given(path("/reins/api/account-state")).respond_with(Cloud::default()).mount(server).await;
    let app = core(dir);
    app.login(server.uri(), "alice@example.com".to_owned(), "pw".to_owned(), None).await.unwrap();
    app.engine().register_account("github", "alice-private-integration").unwrap();
    app.accounts().await.unwrap();
    // The upload that call started runs in the background: count from when it is done.
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while uploads(server).await == 0 {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the account state is uploaded");
    app
}

fn sealed_accounts(dir: &std::path::Path) -> usize {
    std::fs::read_dir(dir.join("accounts"))
        .map_or(0, |d| d.filter(|e| e.as_ref().unwrap().path().extension().is_some_and(|x| x == "sealed")).count())
}

async fn uploads(server: &MockServer) -> usize {
    server.received_requests().await.unwrap().iter().filter(|r| r.method.as_str() == "PUT").count()
}

#[tokio::test]
async fn deleting_the_account_refreshes_an_old_token_then_signs_out_and_forgets_the_phones_copy() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let app = alice(&server, dir.path()).await;
    assert_eq!(sealed_accounts(dir.path()), 1);
    let tokens = Arc::new(Mutex::new(Vec::<String>::new()));
    let seen = Arc::clone(&tokens);
    Mock::given(method("POST"))
        .and(path("/reins/api/account/delete"))
        .respond_with(move |request: &Request| {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            assert_eq!(body, json!({"confirm_email": "Alice@Example.com"}), "trimmed, and no proof needed");
            let mut seen = seen.lock().unwrap();
            seen.push(request.headers.get("authorization").unwrap().to_str().unwrap().to_owned());
            if seen.len() == 1 {
                ResponseTemplate::new(403).set_body_json(json!({"error": "fresh_token_required", "message": "m"}))
            } else {
                ResponseTemplate::new(204)
            }
        })
        .expect(2)
        .mount(&server)
        .await;
    let old = app.engine();
    let uploaded = uploads(&server).await;
    app.delete_account("  Alice@Example.com ".to_owned()).await.unwrap();
    let tokens = tokens.lock().unwrap().clone();
    assert_ne!(tokens[0], tokens[1], "the second attempt carries a newly issued token");
    assert!(app.session().await.is_none());
    assert_eq!(old.register_account("github", "late").unwrap_err(), CoreError::NotLoggedIn);
    assert_eq!(sealed_accounts(dir.path()), 0, "the encrypted account file is gone");
    assert_eq!(app.accounts().await.unwrap().len(), 0);
    assert_eq!(uploads(&server).await, uploaded, "nothing is uploaded for a deleted account");
}

#[tokio::test]
async fn a_refused_deletion_leaves_the_account_signed_in_and_says_why() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let app = alice(&server, dir.path()).await;
    for (status, code, says) in [
        (400, "confirmation_mismatch", "not this account's email"),
        (409, "last_owner", "only owner of an organization"),
        (502, "provider_unavailable", "nothing was deleted"),
        (404, "not_found", "cannot delete accounts from the app"),
    ] {
        Mock::given(method("POST"))
            .and(path("/reins/api/account/delete"))
            .respond_with(ResponseTemplate::new(status).set_body_json(json!({"error": code, "message": "m"})))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        let error = app.delete_account("alice@example.com".to_owned()).await.unwrap_err();
        assert!(matches!(&error, CoreError::Invalid { reason } if reason.contains(says)), "{code}: {error:?}");
        assert!(app.session().await.is_some(), "{code}: still signed in");
        assert!(app.accounts().await.unwrap().iter().any(|a| a.account == "alice-private-integration"));
    }
    assert_eq!(sealed_accounts(dir.path()), 1);
}

#[tokio::test]
async fn another_phone_proves_itself_with_the_password_it_signed_in_with() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let app = alice(&server, dir.path()).await;
    Mock::given(method("POST"))
        .and(path("/reins/api/account/delete"))
        .respond_with(|request: &Request| {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            if body["master_password_hash"].as_str().is_some_and(|h| !h.is_empty()) {
                ResponseTemplate::new(204)
            } else {
                ResponseTemplate::new(403).set_body_json(json!({"error": "proof_required", "message": "m"}))
            }
        })
        .expect(2)
        .mount(&server)
        .await;
    app.delete_account("alice@example.com".to_owned()).await.unwrap();
    assert!(app.session().await.is_none());
}
