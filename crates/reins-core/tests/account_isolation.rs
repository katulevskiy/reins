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
async fn logout_cancels_an_in_flight_accounts_response() {
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
    let reading = Arc::clone(&app);
    let task = tokio::spawn(async move { reading.accounts().await });
    tokio::time::timeout(std::time::Duration::from_secs(3), started.notified()).await.unwrap();
    app.logout().await.unwrap();
    let result = tokio::time::timeout(std::time::Duration::from_secs(1), task).await.unwrap().unwrap();
    assert_eq!(result.unwrap_err(), CoreError::NotLoggedIn);
    assert_eq!(app.accounts().await.unwrap().len(), 0);
}
