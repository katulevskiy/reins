//! Two phones on one account, through the real exported calls and an encrypted-state server with compare-and-swap.
//! What one phone changed while the other moved the server ahead is merged; it never keeps the phone out.
mod common;
use data_encoding::BASE64URL_NOPAD;
use reins_core::crypto::{Kdf, master_key};
use reins_core::{CoreConfig, GoogleTokenProvider, Notifier, ReinsCore};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
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
        // A sign-in names the user; a refresh brings the refresh token this mock handed out, which names them too.
        let email = form.get("username").cloned().unwrap_or_else(|| form["refresh_token"].replace("REFRESH:", ""));
        ResponseTemplate::new(200).set_body_json(
            json!({"access_token":token(&email),"refresh_token":format!("REFRESH:{email}"),"expires_in":7200}),
        )
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

/// The server's account state: compare-and-swap on the revision, as `src/api/reins/account_state.rs`. While
/// `refuse` is set every upload is refused as from a phone that is not the approval device (403).
#[derive(Clone, Default)]
struct Cloud {
    states: Arc<Mutex<HashMap<String, Value>>>,
    refuse: Arc<AtomicBool>,
    refused: Arc<AtomicUsize>,
    conflicts: Arc<AtomicUsize>,
}
impl Cloud {
    fn revision(&self) -> u64 {
        self.states.lock().unwrap().values().next().map_or(0, |s| s["revision"].as_u64().unwrap())
    }
    /// Waits until no upload has changed the revision for a while: whatever the phones were uploading is done.
    async fn settle(&self) {
        let mut last = self.revision();
        loop {
            tokio::time::sleep(Duration::from_millis(400)).await;
            let now = self.revision();
            if now == last {
                return;
            }
            last = now;
        }
    }
    async fn wait_for(&self, what: &str, done: impl Fn(&Self) -> bool) {
        tokio::time::timeout(Duration::from_secs(10), async {
            while !done(self) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for {what}"));
    }
}
impl Respond for Cloud {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let owner = owner(request);
        let mut states = self.states.lock().unwrap();
        let state = states.entry(owner).or_insert_with(|| json!({"revision":0,"ciphertext":null}));
        if request.method.as_str() == "PUT" {
            if self.refuse.load(Ordering::SeqCst) {
                self.refused.fetch_add(1, Ordering::SeqCst);
                return ResponseTemplate::new(403)
                    .set_body_json(json!({"code":"not_approval_device","message":"not the approval device"}));
            }
            let update: Value = serde_json::from_slice(&request.body).unwrap();
            if update["revision"] != state["revision"] {
                self.conflicts.fetch_add(1, Ordering::SeqCst);
                return ResponseTemplate::new(409)
                    .set_body_json(json!({"code":"state_conflict","message":"Account state changed"}));
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
async fn server(cloud: &Cloud) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/identity/accounts/prelogin"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"kdf":0,"kdfIterations":5000})))
        .mount(&server)
        .await;
    Mock::given(method("POST")).and(path("/identity/connect/token")).respond_with(Identity).mount(&server).await;
    Mock::given(method("GET")).and(path("/api/sync")).respond_with(VaultProfile).mount(&server).await;
    Mock::given(path("/reins/api/account-state")).respond_with(cloud.clone()).mount(&server).await;
    server
}
async fn sign_in(server: &MockServer, dir: &std::path::Path) -> Arc<ReinsCore> {
    let app = core(dir);
    app.login(server.uri(), "alice@example.com".to_owned(), "pw".to_owned(), None).await.unwrap();
    app
}
/// Connects an integration on `app` and waits until the server's revision passes `after`.
async fn connect_and_upload(app: &ReinsCore, cloud: &Cloud, account: &str) {
    let before = cloud.revision();
    app.engine().register_account("github", account).unwrap();
    app.accounts().await.unwrap(); // a call that finishes uploads what changed, in the background
    cloud.wait_for("the upload", |c| c.revision() > before).await;
}
async fn integrations(app: &ReinsCore) -> Vec<String> {
    let mut names: Vec<String> =
        app.accounts().await.unwrap().into_iter().filter(|a| a.service == "github").map(|a| a.account).collect();
    names.sort();
    names
}

/// The user's phones: the iPhone moved the server ahead while the Android phone had a change the server refused
/// (it was not the approval device, or its upload lost a race). Then the Android app updated and started again.
/// Before the fix its sign-in failed with "This account has unsynced changes on this phone and newer changes on
/// another phone", every time, and the app stayed on the welcome screen.
#[tokio::test]
async fn a_phone_whose_changes_never_reached_the_server_still_opens_and_keeps_both_phones_changes() {
    let cloud = Cloud::default();
    let server = server(&cloud).await;
    let (iphone_dir, android_dir) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let iphone = sign_in(&server, iphone_dir.path()).await;
    connect_and_upload(&iphone, &cloud, "from-iphone-1").await;
    let android = sign_in(&server, android_dir.path()).await;
    assert_eq!(integrations(&android).await, ["from-iphone-1"]);
    cloud.settle().await;
    // The iPhone starts again too, and is up to date with what the Android phone uploaded while signing in.
    drop(iphone);
    let iphone = core(iphone_dir.path());
    assert!(iphone.session().await.is_some());
    cloud.settle().await;

    // Android writes while the server refuses its uploads.
    cloud.refuse.store(true, Ordering::SeqCst);
    let refused = cloud.refused.load(Ordering::SeqCst);
    android.engine().register_account("github", "from-android").unwrap();
    android.accounts().await.unwrap();
    cloud.wait_for("the refused upload", |c| c.refused.load(Ordering::SeqCst) > refused).await;
    cloud.refuse.store(false, Ordering::SeqCst);
    // The iPhone moves the server ahead.
    connect_and_upload(&iphone, &cloud, "from-iphone-2").await;

    // The Android app is updated: a new process opens the same data.
    drop(android);
    let android = core(android_dir.path());
    assert!(android.session().await.is_some(), "the phone opens signed in, never on the welcome screen");
    assert_eq!(integrations(&android).await, ["from-android", "from-iphone-1", "from-iphone-2"]);
    // Signing in again (the welcome screen's Continue) works too.
    drop(android);
    let android = core(android_dir.path());
    let again = android.login(server.uri(), "alice@example.com".to_owned(), "pw".to_owned(), None).await;
    assert!(again.is_ok(), "{again:?}");
    assert_eq!(integrations(&android).await, ["from-android", "from-iphone-1", "from-iphone-2"]);

    // The merge reaches the server: a phone signing in now sees all three.
    cloud.settle().await;
    let third = sign_in(&server, tempfile::tempdir().unwrap().path()).await;
    assert_eq!(integrations(&third).await, ["from-android", "from-iphone-1", "from-iphone-2"]);
}

/// Two phones upload at once: the one whose compare-and-swap loses pulls, merges and tries again.
#[tokio::test]
async fn an_upload_that_loses_the_race_merges_and_retries() {
    let cloud = Cloud::default();
    let server = server(&cloud).await;
    let (a_dir, b_dir) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let a = sign_in(&server, a_dir.path()).await;
    connect_and_upload(&a, &cloud, "first").await;
    let b = sign_in(&server, b_dir.path()).await;
    integrations(&b).await;
    cloud.settle().await;
    // B changes something without uploading yet; A moves the server ahead meanwhile.
    b.engine().register_account("github", "from-b").unwrap();
    connect_and_upload(&a, &cloud, "from-a").await;
    let conflicts = cloud.conflicts.load(Ordering::SeqCst);
    let before = cloud.revision();
    b.accounts().await.unwrap();
    cloud.wait_for("B's merged upload", |c| c.revision() > before).await;
    assert!(cloud.conflicts.load(Ordering::SeqCst) > conflicts, "B's first try lost the race");
    assert_eq!(integrations(&b).await, ["first", "from-a", "from-b"]);
    let c = sign_in(&server, tempfile::tempdir().unwrap().path()).await;
    assert_eq!(integrations(&c).await, ["first", "from-a", "from-b"]);
}
