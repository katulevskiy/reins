//! Passwordless sign-in through WorkOS AuthKit (a fake WorkOS), the keyless vault, and the WorkOS lifecycle sync,
//! with the real server and real phone cores.

use std::time::Duration;

use rewarden_core::{AccountKeys, CoreError};
use rewarden_e2e::workos::{FakeWorkos, User};
use rewarden_e2e::{Phone, Server};
use serde_json::{Value, json};

/// The KDF iterations prelogin reports for `email`: 100 000 for a keyless account, the server's default (600 000)
/// for an email it does not know.
async fn prelogin_iterations(server: &Server, email: &str) -> u64 {
    rewarden_e2e::init_tls();
    let r: Value = reqwest::Client::new()
        .post(server.url("/identity/accounts/prelogin"))
        .json(&json!({"email": email}))
        .send()
        .await
        .expect("prelogin")
        .json()
        .await
        .expect("prelogin json");
    r["kdfIterations"].as_u64().or_else(|| r["KdfIterations"].as_u64()).expect("iterations")
}

async fn eventually(what: &str, mut check: impl AsyncFnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    while !check().await {
        assert!(tokio::time::Instant::now() < deadline, "{what} did not happen");
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn sso_sign_in_makes_a_keyless_vault_that_follows_workos() {
    let workos = FakeWorkos::start().await;
    let server = Server::start_with_env(5, 3, &workos.server_env()).await;
    let ada = User {
        id: "user_01ADA".to_owned(),
        email: "ada@example.com".to_owned(),
    };

    // First phone: "Continue", AuthKit signs Ada in, the account and its keys are made silently.
    let first = Phone::signed_out(|_| {}).await;
    workos.sign_in_as(&ada);
    let outcome = first.sso_sign_in(&server.base).await.expect("first sign-in");
    assert_eq!(outcome.session.email, "ada@example.com");
    assert_eq!(outcome.keys, AccountKeys::Created);
    first.core.register_device(None).await.expect("the first phone is the approval device");
    assert_eq!(first.core.account_keys().await.unwrap(), AccountKeys::Unlocked);
    let accounts = first.core.accounts().await.unwrap();
    assert!(accounts.iter().any(|a| a.service == "vault" && a.account == "ada@example.com"), "the vault is connected");
    let code = first.core.account_recovery_code().await.expect("recovery code");
    assert_eq!(code.split('-').count(), 13, "{code}");
    assert_eq!(prelogin_iterations(&server, "ada@example.com").await, 100_000);

    // Password sign-in is off on this server (SSO_ONLY).
    let err = first
        .core
        .login(server.base.clone(), "ada@example.com".to_owned(), "anything at all".to_owned(), None)
        .await
        .unwrap_err();
    assert!(!matches!(err, CoreError::NotLoggedIn), "{err:?}");

    // Second phone: the same person signs in; the keys are there but locked until the recovery code opens them.
    let second = Phone::signed_out(|_| {}).await;
    let outcome = second.sso_sign_in(&server.base).await.expect("second sign-in");
    assert_eq!(outcome.keys, AccountKeys::Locked);
    assert!(second.core.unlock_account("AAAA-BBBB".to_owned()).await.is_err());
    second.core.unlock_account(code.to_lowercase().replace('-', " ")).await.expect("the recovery code opens it");
    assert_eq!(second.core.account_keys().await.unwrap(), AccountKeys::Unlocked);
    assert_eq!(second.core.account_recovery_code().await.unwrap(), code);

    // WorkOS changes Ada's email: an unverified one is not taken, the verified one is.
    workos.emit("user.updated", json!({"id": ada.id, "email": "ada@lovelace.dev", "email_verified": false}));
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(prelogin_iterations(&server, "ada@lovelace.dev").await, 600_000);
    workos.emit(
        "user.updated",
        json!({"id": ada.id, "email": "Ada@Lovelace.dev", "email_verified": true, "first_name": "Ada"}),
    );
    eventually("the email change", async || prelogin_iterations(&server, "ada@lovelace.dev").await == 100_000).await;
    assert_eq!(prelogin_iterations(&server, "ada@example.com").await, 600_000);
    // The vault still opens: its key does not depend on the email.
    let third = Phone::signed_out(|_| {}).await;
    workos.sign_in_as(&User {
        id: ada.id.clone(),
        email: "ada@lovelace.dev".to_owned(),
    });
    let outcome = third.sso_sign_in(&server.base).await.expect("sign-in after the email change");
    assert_eq!((outcome.session.email.as_str(), outcome.keys), ("ada@lovelace.dev", AccountKeys::Locked));
    third.core.unlock_account(code.clone()).await.expect("the same recovery code");

    // WorkOS revokes the first phone's session: that phone is signed out, the others are not.
    let sessions = workos.sessions();
    workos.emit("session.revoked", json!({"id": sessions[0], "user_id": ada.id, "status": "inactive"}));
    eventually("the session revocation", async || {
        matches!(first.core.register_device(None).await, Err(CoreError::NotLoggedIn))
    })
    .await;
    second.core.register_device(None).await.expect("the second phone is still signed in");

    // WorkOS deletes Ada: the account goes, with everything Reins kept for it.
    workos.emit("user.deleted", json!({"id": ada.id, "email": "ada@lovelace.dev"}));
    eventually("the account deletion", async || prelogin_iterations(&server, "ada@lovelace.dev").await == 600_000)
        .await;
    assert!(matches!(second.core.register_device(None).await, Err(CoreError::NotLoggedIn)));
    let log = server.log();
    assert!(log.contains("was deleted; deleting account"), "{log}");
}

#[tokio::test(flavor = "multi_thread")]
async fn another_phone_gets_the_keys_from_the_approval_device() {
    use rewarden_core::{JoinProgress, PendingKind};

    let workos = FakeWorkos::start().await;
    let server = Server::start_with_env(5, 3, &workos.server_env()).await;
    let grace = User {
        id: "user_01GRACE".to_owned(),
        email: "grace@example.com".to_owned(),
    };
    workos.sign_in_as(&grace);
    let first = Phone::signed_out(|_| {}).await;
    assert_eq!(first.sso_sign_in(&server.base).await.unwrap().keys, AccountKeys::Created);
    first.core.register_device(None).await.unwrap();

    let second = Phone::signed_out(|_| {}).await;
    assert_eq!(second.sso_sign_in(&server.base).await.unwrap().keys, AccountKeys::Locked);
    assert_eq!(second.core.register_device(None).await.unwrap_err(), CoreError::OtherApprovalDevice);

    // A refusal first: the new phone learns it, and nothing was handed over.
    let asked = second.core.join_begin("Pixel 9".to_owned()).await.expect("join");
    let item = first.wait_for_item(Duration::from_secs(10)).await;
    assert_eq!((item.kind, item.id.as_str()), (PendingKind::Join, asked.id.as_str()));
    first.core.answer_join(item.id, false).await.unwrap();
    assert_eq!(second.core.join_poll().await.unwrap(), JoinProgress::Denied);
    assert_eq!(second.core.account_keys().await.unwrap(), AccountKeys::Locked);
    assert_eq!(second.core.register_device(None).await.unwrap_err(), CoreError::OtherApprovalDevice);

    // Then an approval: both phones show the same code, and the keys open on the new phone.
    let asked = second.core.join_begin("Pixel 9".to_owned()).await.expect("join");
    assert_eq!(second.core.join_poll().await.unwrap(), JoinProgress::Waiting);
    let item = first.wait_for_item(Duration::from_secs(10)).await;
    let view = first.core.join_view(item.id.clone()).await.unwrap();
    assert_eq!((view.device_name.as_str(), view.code.as_str()), ("Pixel 9", asked.code.as_str()));
    first.core.answer_join(item.id, true).await.unwrap();
    assert_eq!(second.core.join_poll().await.unwrap(), JoinProgress::Joined);
    assert_eq!(second.core.account_keys().await.unwrap(), AccountKeys::Unlocked);
    assert_eq!(second.core.account_recovery_code().await.unwrap(), first.core.account_recovery_code().await.unwrap());
    assert!(matches!(second.core.join_poll().await, Err(CoreError::NotFound)), "the request is over");

    // The approval device itself cannot ask, and the new phone takes over approvals when it registers: the approval
    // was its proof.
    assert!(first.core.join_begin("again".to_owned()).await.is_err());
    second.core.register_device(None).await.unwrap();
    assert!(first.core.sync(0).await.is_err(), "the first phone no longer approves");
}

/// Whoever controls the identity Grace signs in with (her Google account, her email) gets a signed-in phone, but not
/// the approval role: that takes the recovery code or her phone's yes.
#[tokio::test(flavor = "multi_thread")]
async fn a_phone_signed_in_to_the_identity_does_not_get_the_approval_role() {
    let workos = FakeWorkos::start().await;
    let server = Server::start_with_env(5, 3, &workos.server_env()).await;
    let grace = User {
        id: "user_01GRACE".to_owned(),
        email: "grace@example.com".to_owned(),
    };
    workos.sign_in_as(&grace);
    let phone = Phone::signed_out(|_| {}).await;
    assert_eq!(phone.sso_sign_in(&server.base).await.unwrap().keys, AccountKeys::Created);
    phone.core.register_device(None).await.expect("the first phone needs no proof");
    phone.core.register_device(Some("fcm-1".to_owned())).await.expect("nor does it to register again");
    let code = phone.core.account_recovery_code().await.unwrap();

    // Someone else signs in as Grace: the keys stay locked, and the server will not make that phone approve.
    let other = Phone::signed_out(|_| {}).await;
    assert_eq!(other.sso_sign_in(&server.base).await.unwrap().keys, AccountKeys::Locked);
    let refused = other.core.register_device(None).await.unwrap_err();
    assert_eq!(refused, CoreError::OtherApprovalDevice);
    assert_eq!(
        refused.to_string(),
        "This account already has a phone for approvals. Approve this phone from it, or enter your recovery code."
    );
    assert!(other.core.sync(0).await.is_err(), "it gets no requests");
    phone.core.sync(0).await.expect("Grace's phone still approves");

    // With the recovery code (Grace's new phone, say), the phone proves itself and takes over.
    other.core.unlock_account(code).await.unwrap();
    other.core.register_device(None).await.expect("the account secret is the proof");
    other.core.sync(0).await.expect("the new phone approves");
    assert!(phone.core.sync(0).await.is_err(), "the old one no longer does");
    // The old phone keeps the secret too, so "Use this phone" there moves the role back without asking.
    phone.core.register_device(None).await.expect("its secret proves it");
    phone.core.sync(0).await.unwrap();
}
