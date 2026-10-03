//! Onboarding without a web vault and without typing addresses, against the real server binary: the phone creates the
//! account itself (Bitwarden-compatible vault keys), and the desktop app pairs by a QR code the phone scans (OAuth
//! device authorization, RFC 8628) with its key pinned exactly as after the browser login.
//!
//! It lives here rather than in `reins-desktop` so the desktop crate (Apache-2.0) has no dependency on the AGPL-3.0
//! crates. See LICENSING.md.

use std::time::Duration;

use data_encoding::BASE64;
use reins_core::crypto::{Kdf, VaultKey, master_key};
use reins_core::http::ServerUrl;
use reins_core::vault::VaultClient;
use reins_core::{ApprovalChoice, CoreError};
use reins_desktop::config::Paths;
use reins_desktop::identity::Identity;
use reins_desktop::server::device::{DevicePairing, DeviceStatus};
use reins_e2e::{AiClient, PASSWORD, Phone, Server};
use serde_json::{Value, json};
use zeroize::Zeroizing;

const EMAIL: &str = "new@example.com";

fn http() -> reqwest::Client {
    reins_e2e::init_tls();
    reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().unwrap()
}

/// The vault as a Bitwarden client sees it: a fresh login with the master password, then `/api/sync`.
async fn sync_as_a_bitwarden_client(server: &Server) -> (reqwest::Client, String, Value) {
    let http = http();
    let url = ServerUrl::parse(&server.base).unwrap();
    let (tokens, _) = VaultClient::new(&http, &url)
        .login(EMAIL, Zeroizing::new(PASSWORD.to_owned()), None, "another-client")
        .await
        .expect("a second client signs in with the same password");
    let token = tokens.access_token.to_string();
    let sync: Value = http
        .get(server.url("/api/sync?excludeDomains=true"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    (http, token, sync)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_phone_creates_the_account_and_its_vault_works_like_a_bitwarden_vault() {
    let server = Server::start(20, 10).await;
    let phone = Phone::create_account(&server.base, EMAIL).await;
    assert_eq!(phone.core.session().await.unwrap().email, EMAIL);

    // The keys on the server open with the master password, as in any Bitwarden client.
    let (http, token, sync) = sync_as_a_bitwarden_client(&server).await;
    let profile = &sync["profile"];
    assert_eq!(profile["email"], EMAIL);
    let master = tokio::task::spawn_blocking(|| {
        master_key(
            PASSWORD,
            EMAIL,
            Kdf::Pbkdf2 {
                iterations: 600_000,
            },
        )
        .unwrap()
    })
    .await
    .unwrap();
    let user = VaultKey::from_bytes(&master.stretch().decrypt(profile["key"].as_str().unwrap()).unwrap()).unwrap();
    let private = user.decrypt(profile["privateKey"].as_str().unwrap()).unwrap();
    assert_eq!(private[0], 0x30, "the private key is PKCS#8 DER (a SEQUENCE)");
    assert!(private.len() > 1100, "an RSA-2048 key: {} bytes", private.len());
    let keys_url = server.url(&format!("/api/users/{}/public-key", profile["id"].as_str().unwrap()));
    let public: Value = http.get(keys_url).bearer_auth(&token).send().await.unwrap().json().await.unwrap();
    let public = BASE64.decode(public["publicKey"].as_str().unwrap().as_bytes()).unwrap();
    assert_eq!(public[0], 0x30, "SubjectPublicKeyInfo DER");

    // A login saved by that client...
    let text = |s: &str| user.encrypt(s.as_bytes()).unwrap();
    let cipher = json!({
        "type": 1, "name": text("GitHub"), "notes": null, "favorite": false, "folderId": null, "organizationId": null,
        "login": {"username": text("octo"), "password": text("hunter2!"), "totp": null,
                  "uris": [{"uri": text("https://github.com/login"), "match": null}]}
    });
    let saved = http.post(server.url("/api/ciphers")).bearer_auth(&token).json(&cipher).send().await.unwrap();
    assert!(saved.status().is_success(), "{}", saved.text().await.unwrap_or_default());

    // ...is in the vault the phone unlocks with the master password, and an AI finds it once the user agrees.
    assert!(phone.core.add_token_account("vault".to_owned(), "not the password".to_owned()).await.is_err());
    phone.core.add_token_account("vault".to_owned(), PASSWORD.to_owned()).await.expect("the vault unlocks");
    let mut ai = AiClient::new(&server.base);
    ai.register_client().await;
    let wait_url = ai.start_authorization(EMAIL).await;
    let code = ai.browser_code(&wait_url).await;
    let item = phone.wait_for_item(Duration::from_secs(20)).await;
    phone.core.answer_pairing(item.id, true, Some(code), Some("Claude".to_owned())).await.unwrap();
    ai.finish_authorization(&wait_url).await;
    let query = json!({"query": "git"});
    let search = ai.tool("vault_search", &query);
    let user_agrees = async {
        let item = phone.wait_for_item(Duration::from_secs(20)).await;
        let view = phone.core.approval_view(item.id.clone()).await.unwrap();
        let ids: Vec<String> = view.messages.iter().map(|m| m.id.clone()).collect();
        assert_eq!(ids.len(), 1, "{view:?}");
        let choice = ApprovalChoice {
            selected_message_ids: ids,
            standing: None,
        };
        phone.core.approve(item.id, choice).await.unwrap();
    };
    let (result, ()) = tokio::join!(search, user_agrees);
    assert_eq!(result["isError"], false, "{result}");
    let shown = result.to_string();
    assert!(shown.contains("GitHub") && shown.contains("octo"), "{shown}");
    assert!(!shown.contains("hunter2!"), "a search never shows the password");

    // The email is taken now; a short master password is refused before anything is sent.
    phone.core.logout().await.unwrap();
    let again = phone.core.create_account(server.base.clone(), EMAIL.to_owned(), PASSWORD.to_owned()).await;
    assert_eq!(again.unwrap_err(), CoreError::invalid("An account with this email already exists. Sign in instead."));
    let short =
        phone.core.create_account(server.base.clone(), "other@example.com".to_owned(), "short".to_owned()).await;
    assert!(matches!(short, Err(CoreError::Invalid { reason }) if reason.contains("12 characters")));
    assert!(!server.log().contains("panicked"), "server log: {}", server.log());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_desktop_app_pairs_by_a_qr_code_the_phone_scans() {
    let server = Server::start(20, 10).await;
    let phone = Phone::create_account(&server.base, EMAIL).await;
    let state = tempfile::tempdir().unwrap();
    let paths = Paths::under(state.path());
    paths.ensure().unwrap();
    let identity = Identity::load_or_create(&paths.identity_file()).unwrap();

    // `reins login`: the QR code links to the server's pairing page with the code.
    let mut pairing = DevicePairing::start(&identity, &server.base).await.expect("a QR code");
    assert_eq!(pairing.qr_url, format!("{}/pair?code={}", server.base, pairing.user_code));
    assert_eq!(pairing.verification_uri, format!("{}/pair", server.base));
    let confirm = pairing.confirm_code.expect("the number to tap");
    let page = http().get(&pairing.qr_url).send().await.unwrap().text().await.unwrap();
    assert!(page.contains(&pairing.user_code) && page.contains("reins://pair?code="), "{page}");
    assert_eq!(pairing.poll(&paths).await.unwrap(), DeviceStatus::Waiting);

    // The phone scans it: the pairing view shows this app's key and offers the number among three.
    let scanned = pairing.user_code.to_lowercase().replace('-', " ");
    let view = phone.core.pairing_by_code(scanned.clone()).await.expect("the phone claims the code");
    assert_eq!(view.key_fingerprint.as_deref(), Some(identity.fingerprint().as_str()));
    assert!(view.choices.contains(&confirm), "{view:?}");
    assert_eq!(phone.core.pairing_by_code(scanned).await.unwrap().id, view.id, "scanning twice is the same pairing");
    assert_eq!(phone.core.pairing_view(view.id.clone()).await.unwrap(), view, "parked like a pushed pairing");
    phone.core.answer_pairing(view.id, true, Some(confirm), Some("Laptop".to_owned())).await.unwrap();

    // The next poll logs the app in; the key is pinned: the phone answers its question, sealed to that key.
    let logged_in = tokio::time::timeout(Duration::from_secs(30), pairing.wait(&paths)).await.unwrap();
    assert_eq!(logged_in.unwrap(), server.base);
    assert!(phone.core.connections().await.unwrap().iter().any(|c| c.label == "Laptop"));
    let question = reins_desktop::ask::Question {
        question: "Deploy to prod?".to_owned(),
        detail: None,
        topic: None,
    };
    let asked = reins_desktop::ask::ask_phone(&paths, &identity, &question, Duration::from_secs(60));
    let user = async {
        let item = phone.wait_for_item(Duration::from_secs(30)).await;
        let choice = ApprovalChoice {
            selected_message_ids: vec![],
            standing: None,
        };
        phone.core.approve(item.id, choice).await.unwrap();
    };
    let (answer, ()) = tokio::join!(asked, user);
    assert_eq!(answer, reins_desktop::ask::Answer::Yes);

    // A used code is gone; a denied pairing ends the login with a clear message.
    let spent = phone.core.pairing_by_code(pairing.user_code.clone()).await;
    assert_eq!(spent.unwrap_err(), CoreError::NotFound);
    let mut denied = DevicePairing::start(&identity, &server.base).await.unwrap();
    let view = phone.core.pairing_by_code(denied.user_code.clone()).await.unwrap();
    phone.core.answer_pairing(view.id, false, None, None).await.unwrap();
    let refused = tokio::time::timeout(Duration::from_secs(30), denied.wait(&paths)).await.unwrap();
    assert!(refused.unwrap_err().contains("denied"));
    assert!(matches!(phone.core.pairing_by_code("not a code".to_owned()).await, Err(CoreError::Invalid { .. })));
    assert!(!server.log().contains("panicked"), "server log: {}", server.log());
}
