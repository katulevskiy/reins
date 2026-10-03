//! Live check of passwordless sign-in against real WorkOS (a staging environment), headless: the local server with
//! `SSO_PROVIDER=workos`, two phone cores, and headless Chrome on AuthKit's hosted page (`authkit/sign-in.mjs`).
//! Run it through `scripts/workos-live.sh`, which reads the WorkOS credentials and installs the browser driver.
//!
//! 1. A WorkOS test user is made through the API (verified email, random password).
//! 2. Phone A: "Continue", AuthKit sign-in, the account and its keys are made silently, the recovery code exists.
//! 3. Phone B: the same person; the keys are locked until the recovery code opens them.
//! 4. WorkOS changes the email (verified): the account follows (WorkOS events → the server's sync).
//! 5. WorkOS revokes phone A's session: phone A is signed out, phone B is not.
//! 6. WorkOS deletes the user: the account is deleted.
//!
//! Needs `http://localhost:8765/identity/connect/oidc-signin` among the WorkOS redirect URIs (the script adds it).

use std::process::Command;
use std::time::Duration;

use reins_core::{AccountKeys, CoreError};
use reins_e2e::{Phone, Server};
use serde_json::{Value, json};

const BASE: &str = "http://localhost:8765";
const WORKOS: &str = "https://api.workos.com";

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("set {name}"))
}

struct Workos {
    http: reqwest::Client,
    key: String,
}

impl Workos {
    async fn call(&self, method: reqwest::Method, path: &str, body: Option<Value>) -> Value {
        let mut req = self.http.request(method, format!("{WORKOS}{path}")).bearer_auth(&self.key);
        if let Some(body) = body {
            req = req.json(&body);
        }
        let resp = req.send().await.expect("WorkOS API");
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        assert!(status.is_success(), "WorkOS {path}: {status} {text}");
        serde_json::from_str(&text).unwrap_or(Value::Null)
    }
}

/// Signs in on AuthKit in headless Chrome and returns the app's callback URL.
fn browser_sign_in(url: &str, scheme: &str, email: &str, password: &str) -> String {
    let out = Command::new(std::env::var("NODE").unwrap_or_else(|_| "node".to_owned()))
        .arg(env("AUTHKIT_DRIVER"))
        .arg(url)
        .arg(scheme)
        .env("AUTHKIT_EMAIL", email)
        .env("AUTHKIT_PASSWORD", password)
        .output()
        .expect("run the AuthKit driver");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "AuthKit sign-in failed: {}", String::from_utf8_lossy(&out.stderr));
    stdout.lines().find_map(|l| l.strip_prefix("CALLBACK ")).expect("callback").trim().to_owned()
}

async fn sso(phone: &Phone, email: &str, password: &str) -> reins_core::SsoOutcome {
    let start = phone.core.sso_begin(BASE.to_owned()).await.expect("sso_begin");
    let (url, scheme, email, password) =
        (start.url.clone(), start.callback_scheme.clone(), email.to_owned(), password.to_owned());
    let callback =
        tokio::task::spawn_blocking(move || browser_sign_in(&url, &scheme, &email, &password)).await.expect("browser");
    phone.core.sso_finish(BASE.to_owned(), callback, start.state, start.verifier).await.expect("sso_finish")
}

async fn prelogin_iterations(email: &str) -> u64 {
    let r: Value = reqwest::Client::new()
        .post(format!("{BASE}/identity/accounts/prelogin"))
        .json(&json!({"email": email}))
        .send()
        .await
        .expect("prelogin")
        .json()
        .await
        .expect("prelogin json");
    r["kdfIterations"].as_u64().expect("iterations")
}

async fn eventually(what: &str, mut check: impl AsyncFnMut() -> bool) {
    let started = tokio::time::Instant::now();
    while !check().await {
        assert!(started.elapsed() < Duration::from_secs(180), "{what} did not happen within 3 minutes");
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    println!("ok: {what} ({}s)", started.elapsed().as_secs());
}

async fn run(workos: &Workos, user_id: &str, email: &str, password: &str) {
    let client_id = env("WORKOS_CLIENT_ID");
    let server_env: Vec<(String, String)> = [
        ("SSO_ENABLED", "true"),
        ("SSO_ONLY", "true"),
        // This headless staging harness exercises Magic Auth, not a platform authenticator.
        ("REINS_WORKOS_REQUIRE_PASSKEY", "false"),
        ("SSO_CLIENT_ID", client_id.as_str()),
        ("SSO_CLIENT_SECRET", workos.key.as_str()),
        ("SSO_AUTH_ONLY_NOT_SESSION", "true"),
        ("REINS_WORKOS_SYNC_SECS", "2"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v.to_owned()))
    .chain([("SSO_AUTHORITY".to_owned(), format!("{WORKOS}/user_management/{client_id}"))])
    .collect();
    let server = Server::start_at(BASE, 10, 5, &server_env).await;
    println!("server: {BASE} (WorkOS {client_id})");

    let a = Phone::signed_out(|_| {}).await;
    let outcome = sso(&a, email, password).await;
    assert_eq!((outcome.session.email.as_str(), outcome.keys), (email, AccountKeys::Created));
    a.core.register_device(None).await.expect("phone A is the approval device");
    let code = a.core.account_recovery_code().await.expect("recovery code");
    assert_eq!(code.split('-').count(), 13);
    println!("ok: phone A signed in through AuthKit; the account, its keys and the recovery code were made");

    let b = Phone::signed_out(|_| {}).await;
    let outcome = sso(&b, email, password).await;
    assert_eq!(outcome.keys, AccountKeys::Locked);
    b.core.unlock_account(code.clone()).await.expect("the recovery code opens the vault");
    assert_eq!(b.core.account_keys().await.expect("keys"), AccountKeys::Unlocked);
    println!("ok: phone B signed in; locked until the recovery code opened it");

    let new_email = email.replace("e2e-", "e2e-moved-");
    workos
        .call(
            reqwest::Method::PUT,
            &format!("/user_management/users/{user_id}"),
            Some(json!({"email": new_email, "email_verified": true})),
        )
        .await;
    eventually("the account follows the WorkOS email change", async || {
        prelogin_iterations(&new_email).await == 100_000
    })
    .await;

    let sessions = workos.call(reqwest::Method::GET, &format!("/user_management/users/{user_id}/sessions"), None).await;
    let mut list: Vec<(String, String)> = sessions["data"]
        .as_array()
        .expect("sessions")
        .iter()
        .map(|s| {
            (s["created_at"].as_str().unwrap_or_default().to_owned(), s["id"].as_str().unwrap_or_default().to_owned())
        })
        .collect();
    list.sort();
    let first_session = list.first().expect("phone A's session").1.clone();
    workos
        .call(reqwest::Method::POST, "/user_management/sessions/revoke", Some(json!({"session_id": first_session})))
        .await;
    eventually("phone A is signed out after WorkOS revoked its session", async || {
        matches!(a.core.register_device(None).await, Err(CoreError::NotLoggedIn))
    })
    .await;
    b.core.account_keys().await.expect("phone B is still signed in");

    workos.call(reqwest::Method::DELETE, &format!("/user_management/users/{user_id}"), None).await;
    eventually("the account is deleted after WorkOS deleted the user", async || {
        prelogin_iterations(&new_email).await == 600_000
    })
    .await;
    assert!(matches!(b.core.register_device(None).await, Err(CoreError::NotLoggedIn)));
    println!("ok: phone B is signed out too");
    drop(server);
}

#[tokio::main]
async fn main() {
    reins_e2e::init_tls();
    let workos = Workos {
        http: reqwest::Client::new(),
        key: env("WORKOS_API_KEY"),
    };
    let tag = uuid::Uuid::new_v4().simple().to_string();
    let email = format!("e2e-{}@test.reins2fa.com", &tag[..10]);
    let password = format!("Reins-{tag}");
    let user = workos
        .call(
            reqwest::Method::POST,
            "/user_management/users",
            Some(json!({"email": email, "password": password, "email_verified": true,
                        "first_name": "Reins", "last_name": "Live"})),
        )
        .await;
    let user_id = user["id"].as_str().expect("user id").to_owned();
    println!("WorkOS test user {email} ({user_id})");

    let outcome = {
        let (id, email, password) = (user_id.clone(), email.clone(), password.clone());
        let workos = Workos {
            http: workos.http.clone(),
            key: workos.key.clone(),
        };
        // Its own thread and runtime, so that a failed assertion still lets the cleanup below run.
        std::thread::spawn(move || {
            tokio::runtime::Runtime::new().expect("runtime").block_on(run(&workos, &id, &email, &password));
        })
        .join()
    };
    // Whatever happened, the test user goes (it may be gone already).
    drop(workos.http.delete(format!("{WORKOS}/user_management/users/{user_id}")).bearer_auth(&workos.key).send().await);
    if outcome.is_err() {
        eprintln!("FAILED (see the panic above)");
        std::process::exit(1);
    }
    println!("PASS: WorkOS live sign-in, keyless vault and lifecycle sync");
}
