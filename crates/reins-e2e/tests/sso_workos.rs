//! Passwordless sign-in through WorkOS AuthKit (a fake WorkOS), the keyless vault, and the WorkOS lifecycle sync,
//! with the real server and real phone cores.

use std::time::Duration;

use reins_core::{AccountKeys, CoreError};
use reins_e2e::workos::{FakeWorkos, User};
use reins_e2e::{Phone, Server};
use serde_json::{Value, json};

#[tokio::test(flavor = "multi_thread")]
async fn mobile_logout_ends_only_its_device_and_next_login_requires_fresh_authentication() {
    let workos = FakeWorkos::start().await;
    let server = Server::start_with_env(5, 3, &workos.server_env()).await;
    let http = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().unwrap();
    assert_eq!(http.post(format!("{}/reins/api/logout", server.base)).send().await.unwrap().status(), 401);

    let alice = User {
        id: "user_logout_alice".to_owned(),
        email: "logout-alice@example.com".to_owned(),
    };
    workos.sign_in_as(&alice);
    let first = Phone::signed_out(|_| {}).await;
    first.sso_sign_in(&server.base).await.unwrap();
    first.core.register_device(None).await.unwrap();
    let recovery = first.core.account_recovery_code().await.unwrap();
    let second = Phone::signed_out(|_| {}).await;
    second.sso_sign_in(&server.base).await.unwrap();
    second.core.unlock_account(recovery).await.unwrap();
    workos.sign_in_as(&User {
        id: "user_logout_bob".to_owned(),
        email: "logout-bob@example.com".to_owned(),
    });
    let other_account = Phone::signed_out(|_| {}).await;
    other_account.sso_sign_in(&server.base).await.unwrap();
    other_account.core.register_device(None).await.unwrap();
    let db = rusqlite::Connection::open(server.database_path()).unwrap();
    let first_device: String = db
        .query_row("SELECT device_uuid FROM reins_sso_sessions WHERE session_id=?1", [&workos.sessions()[0]], |row| {
            row.get(0)
        })
        .unwrap();

    let logout = first.core.logout_with_browser().await.unwrap().expect("provider logout URL");
    let url = url::Url::parse(&logout).unwrap();
    assert_eq!(url.path(), "/user_management/sessions/logout");
    let query: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
    assert_eq!(query["session_id"], workos.sessions()[0]);
    assert_eq!(query["return_to"], format!("{}/reins/signed-out", server.base));
    assert!(first.core.session().await.is_none());
    let count: i64 =
        db.query_row("SELECT COUNT(*) FROM devices WHERE uuid=?1", [&first_device], |row| row.get(0)).unwrap();
    assert_eq!(count, 0, "server-side access and refresh tokens must be revoked too");
    other_account.core.register_device(None).await.expect("other account remains signed in");
    second.core.register_device(None).await.expect("other phone of the same account remains signed in");
    let page = http.get(&query["return_to"]).send().await.unwrap();
    assert_eq!(page.status(), 200);
    assert!(page.text().await.unwrap().contains("com.reins2fa.app://signed-out"));

    // The real server's browser redirect requests fresh authentication, even if its logout tab was closed.
    let begin = first.core.sso_begin(server.base.clone()).await.unwrap();
    let redirect = http.get(&begin.url).send().await.unwrap();
    let provider = url::Url::parse(redirect.headers()["location"].to_str().unwrap()).unwrap();
    let query: std::collections::HashMap<_, _> = provider.query_pairs().into_owned().collect();
    assert_eq!(query["prompt"], "login");
    assert_eq!(query["max_age"], "0");
}

/// The KDF iterations prelogin reports for `email`: 100 000 for a keyless account, the server's default (600 000)
/// for an email it does not know.
async fn prelogin_iterations(server: &Server, email: &str) -> u64 {
    reins_e2e::init_tls();
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
async fn workos_requires_passkeys_even_when_sso_only_is_unset() {
    let workos = FakeWorkos::start().await;
    let mut env = workos.server_env();
    env.retain(|(key, _)| key != "SSO_ONLY");
    env.push(("REINS_WORKOS_REQUIRE_PASSKEY".to_owned(), "true".to_owned()));
    let server = Server::start_with_env(5, 3, &env).await;
    let user = User {
        id: "user_PASSKEY".to_owned(),
        email: "passkey@example.com".to_owned(),
    };
    workos.sign_in_as(&user);
    let phone = Phone::signed_out(|_| {}).await;
    for method in ["GoogleOAuth", "MagicAuth", "Password"] {
        workos.authenticate_with(method);
        let error = phone.sso_sign_in(&server.base).await.unwrap_err();
        assert!(error.to_string().contains("passkey is required"), "{method}: {error}");
    }
    workos.authenticate_with("Passkey");
    let outcome = phone.sso_sign_in(&server.base).await.expect("passkey sign-in");
    assert_eq!(outcome.keys, AccountKeys::Created);
    let error = phone.core.login(server.base.clone(), user.email, "password".to_owned(), None).await.unwrap_err();
    assert!(error.to_string().contains("SSO sign-in is required"), "{error}");
}

#[tokio::test(flavor = "multi_thread")]
async fn workos_email_and_social_login_work_by_default_and_sessions_are_revocable() {
    let workos = FakeWorkos::start().await;
    let server = Server::start_with_env(5, 3, &workos.server_env()).await;
    for method in ["GoogleOAuth", "MagicAuth", "Password", "GitHubOAuth", "MicrosoftOAuth", "AppleOAuth", "Passkey"] {
        let user = User {
            id: format!("user_{method}"),
            email: format!("{method}@example.com").to_lowercase(),
        };
        workos.sign_in_as(&user);
        workos.authenticate_with(method);
        let phone = Phone::signed_out(|_| {}).await;
        let outcome = phone.sso_sign_in(&server.base).await.expect("WorkOS sign-in");
        assert_eq!(outcome.keys, AccountKeys::Created, "{method}");
        phone.core.register_device(None).await.unwrap();
        // Identity login cannot enable the local master-password route.
        let error = phone.core.login(server.base.clone(), user.email, "password".to_owned(), None).await.unwrap_err();
        assert!(error.to_string().contains("SSO sign-in is required"), "{method}: {error}");
        let session = workos.sessions().last().unwrap().clone();
        workos.emit("session.revoked", json!({"id": session}));
        eventually("WorkOS session revocation", async || {
            matches!(phone.core.register_device(None).await, Err(CoreError::NotLoggedIn))
        })
        .await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn failed_account_cleanup_retries_without_blocking_paginated_revocations() {
    let workos = FakeWorkos::start().await;
    let server = Server::start_with_env(5, 3, &workos.server_env()).await;
    let deleted = User {
        id: "user_DELETE".to_owned(),
        email: "delete@example.com".to_owned(),
    };
    workos.sign_in_as(&deleted);
    let first = Phone::signed_out(|_| {}).await;
    first.sso_sign_in(&server.base).await.unwrap();
    first.core.register_device(None).await.unwrap();
    let other = User {
        id: "user_OTHER".to_owned(),
        email: "other@example.com".to_owned(),
    };
    workos.sign_in_as(&other);
    let second = Phone::signed_out(|_| {}).await;
    second.sso_sign_in(&server.base).await.unwrap();
    second.core.register_device(None).await.unwrap();
    let db = rusqlite::Connection::open(server.database_path()).unwrap();
    db.busy_timeout(Duration::from_secs(5)).unwrap();
    db.execute_batch("CREATE TRIGGER fail_account_delete BEFORE DELETE ON users BEGIN SELECT RAISE(ABORT, 'temporary deletion failure'); END").unwrap();
    workos.emit("user.deleted", json!({"id": deleted.id}));
    // More than one Events API page. An unrelated blocked deletion must not delay revocation on page two.
    for i in 0..105 {
        workos.emit("user.updated", json!({"id": format!("unknown_{i}"), "first_name": "Ignore"}));
    }
    workos.emit("session.revoked", json!({"id": workos.sessions()[1]}));
    eventually("the first phone's invalidation", async || {
        matches!(first.core.register_device(None).await, Err(CoreError::NotLoggedIn))
    })
    .await;
    eventually("page-two session revocation", async || {
        matches!(second.core.register_device(None).await, Err(CoreError::NotLoggedIn))
    })
    .await;
    let pending: i64 = db
        .query_row("SELECT COUNT(*) FROM reins_settings WHERE name LIKE 'workos.delete.%'", [], |row| row.get(0))
        .unwrap();
    assert_eq!(pending, 1, "failed cleanup is persisted for retry");
    let enabled: bool =
        db.query_row("SELECT enabled FROM users WHERE email='delete@example.com'", [], |row| row.get(0)).unwrap();
    assert!(!enabled);
    db.execute_batch("DROP TRIGGER fail_account_delete").unwrap();
    eventually("deferred cleanup", async || {
        db.query_row("SELECT COUNT(*) FROM users WHERE email='delete@example.com'", [], |row| row.get::<_, i64>(0))
            .unwrap()
            == 0
            && db
                .query_row("SELECT COUNT(*) FROM reins_settings WHERE name LIKE 'workos.delete.%'", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap()
                == 0
    })
    .await;
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
    // The already signed-in approval phone learns the change through its usual poll, before token expiry.
    first.core.sync(0).await.expect("poll current WorkOS metadata");
    assert_eq!(first.core.session().await.unwrap().email, "ada@lovelace.dev");
    let accounts = first.core.accounts().await.unwrap();
    assert!(accounts.iter().any(|a| a.service == "vault" && a.account == "ada@lovelace.dev"));
    assert!(!accounts.iter().any(|a| a.service == "vault" && a.account == "ada@example.com"));
    assert_eq!(first.core.account_recovery_code().await.unwrap(), code);
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
    use reins_core::{JoinProgress, PendingKind};

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

/// A lost phone signed out from the approval phone (with the recovery code typed then): its WorkOS session ends at
/// WorkOS, so the sign-in it kept no longer works, and a sign-in with its device id is refused.
#[tokio::test(flavor = "multi_thread")]
async fn signing_a_phone_out_ends_its_workos_session_and_it_cannot_sign_back_in() {
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
    let lost = Phone::signed_out(|_| {}).await;
    lost.sso_sign_in(&server.base).await.unwrap();
    let lost_session = workos.sessions().last().unwrap().clone();

    let other = first.core.devices().await.unwrap().into_iter().find(|d| !d.this_device).expect("the other phone");
    let code = first.core.account_recovery_code().await.unwrap();
    assert!(first.core.sign_out_device(other.id.clone(), "AAAA-BBBB".to_owned()).await.is_err(), "a wrong code");
    first.core.sign_out_device(other.id, code).await.unwrap();
    assert_eq!(workos.revoked_sessions(), [lost_session], "only the lost phone's session ends");
    assert!(lost.core.account_keys().await.is_err(), "its sign-in is over");
    let back = lost.sso_sign_in(&server.base).await;
    assert!(format!("{back:?}").contains("signed out of the account"), "{back:?}");
    first.core.sync(0).await.expect("the approval phone still works");
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

fn sealed_accounts(dir: &std::path::Path) -> usize {
    std::fs::read_dir(dir.join("accounts"))
        .map_or(0, |d| d.filter(|e| e.as_ref().unwrap().path().extension().is_some_and(|x| x == "sealed")).count())
}

/// "Delete account" in the app: the account and its WorkOS user go together, only from a phone that may approve for
/// it, and not at all while WorkOS cannot delete the user.
#[tokio::test(flavor = "multi_thread")]
async fn deleting_the_account_in_the_app_deletes_its_workos_user_too() {
    let workos = FakeWorkos::start().await;
    let server = Server::start_with_env(5, 3, &workos.server_env()).await;
    let hedy = User {
        id: "user_01HEDY".to_owned(),
        email: "hedy@example.com".to_owned(),
    };
    workos.sign_in_as(&hedy);
    let phone = Phone::signed_out(|_| {}).await;
    assert_eq!(phone.sso_sign_in(&server.base).await.unwrap().keys, AccountKeys::Created);
    phone.core.register_device(None).await.unwrap();
    assert_eq!(sealed_accounts(phone.data_dir()), 1);

    // A phone signed in to the identity alone (its keys locked) cannot delete the account.
    let other = Phone::signed_out(|_| {}).await;
    assert_eq!(other.sso_sign_in(&server.base).await.unwrap().keys, AccountKeys::Locked);
    let refused = other.core.delete_account(hedy.email.clone()).await.unwrap_err();
    assert!(
        matches!(&refused, CoreError::Invalid { reason } if reason.contains("Another phone approves")),
        "{refused:?}"
    );

    // A typo, and WorkOS being down, change nothing.
    let typo = phone.core.delete_account("hedy@example.org".to_owned()).await.unwrap_err();
    assert!(matches!(&typo, CoreError::Invalid { reason } if reason.contains("not this account's email")), "{typo:?}");
    workos.fail_deletes(true);
    let down = phone.core.delete_account(hedy.email.clone()).await.unwrap_err();
    assert!(matches!(&down, CoreError::Invalid { reason } if reason.contains("nothing was deleted")), "{down:?}");
    assert_eq!(workos.deleted_users(), Vec::<String>::new());
    assert_eq!(prelogin_iterations(&server, &hedy.email).await, 100_000, "the account is still there");
    assert!(phone.core.session().await.is_some());
    phone.core.sync(0).await.expect("still the approval device");

    workos.fail_deletes(false);
    phone.core.delete_account(" Hedy@Example.com ".to_owned()).await.expect("deleted");
    assert_eq!(workos.deleted_users(), [hedy.id.as_str()]);
    assert_eq!(prelogin_iterations(&server, &hedy.email).await, 600_000, "the account is gone");
    assert!(phone.core.session().await.is_none());
    assert_eq!(sealed_accounts(phone.data_dir()), 0, "and so is the phone's encrypted copy");
    assert!(matches!(other.core.register_device(None).await, Err(CoreError::NotLoggedIn)));
    let db = rusqlite::Connection::open(server.database_path()).unwrap();
    for table in ["sso_users", "reins_devices", "reins_sso_sessions", "devices"] {
        let rows: i64 = db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| row.get(0)).unwrap();
        assert_eq!(rows, 0, "{table}");
    }
    let pending: i64 = db
        .query_row("SELECT COUNT(*) FROM reins_settings WHERE name LIKE 'workos.delete.%'", [], |row| row.get(0))
        .unwrap();
    assert_eq!(pending, 0);
    assert!(server.log().contains("was deleted at its owner's request"), "audit log");

    // WorkOS reports the deletion it made; the sync finds nothing left to do.
    workos.emit("user.deleted", json!({"id": hedy.id}));
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert!(!server.log().contains("WorkOS sync failed"), "{}", server.log());
}

/// Grace reinstalls the app and has neither her old phone nor her recovery code: signing in again to the same account
/// resets the vault, the reinstalled phone makes new keys and takes the approval role, and the old phone is out.
#[tokio::test(flavor = "multi_thread")]
async fn a_fresh_sign_in_resets_a_vault_nothing_can_open() {
    let workos = FakeWorkos::start().await;
    let server = Server::start_with_env(5, 3, &workos.server_env()).await;
    let grace = User {
        id: "user_01GRACE".to_owned(),
        email: "grace@example.com".to_owned(),
    };
    workos.sign_in_as(&grace);
    let old = Phone::signed_out(|_| {}).await;
    assert_eq!(old.sso_sign_in(&server.base).await.unwrap().keys, AccountKeys::Created);
    old.core.register_device(None).await.unwrap();
    let old_code = old.core.account_recovery_code().await.unwrap();
    // Its account state is on the server, sealed with the keys that are about to be lost.
    old.core.engine().register_account("github", "grace-integration").unwrap();
    let states = server.database_path().with_file_name("reins-account-state");
    let saved = || {
        std::fs::read_dir(&states)
            .map_or(0, |d| d.flatten().filter(|e| e.path().extension().is_some_and(|x| x == "json")).count())
    };
    eventually("the account state upload", async || {
        old.core.accounts().await.unwrap();
        saved() == 1
    })
    .await;

    let reinstalled = Phone::signed_out(|_| {}).await;
    assert_eq!(reinstalled.sso_sign_in(&server.base).await.unwrap().keys, AccountKeys::Locked);
    assert_eq!(reinstalled.core.register_device(None).await.unwrap_err(), CoreError::OtherApprovalDevice);

    // Signing in to another account does not confirm the reset of this one.
    workos.sign_in_as(&User {
        id: "user_01MALLORY".to_owned(),
        email: "mallory@example.com".to_owned(),
    });
    let start = reinstalled.core.sso_begin(server.base.clone()).await.unwrap();
    let callback = reins_e2e::workos::browse_to_callback(&start.url, &start.callback_scheme).await;
    let refused = reinstalled.core.reset_account(server.base.clone(), callback, start.state, start.verifier).await;
    assert!(
        matches!(&refused, Err(CoreError::Invalid { reason }) if reason.contains("mallory@example.com")),
        "{refused:?}"
    );
    assert_eq!(saved(), 1, "nothing was reset");

    workos.sign_in_as(&grace);
    let start = reinstalled.core.sso_begin(server.base.clone()).await.unwrap();
    let callback = reins_e2e::workos::browse_to_callback(&start.url, &start.callback_scheme).await;
    let outcome = reinstalled.core.reset_account(server.base.clone(), callback, start.state, start.verifier).await;
    let outcome = outcome.expect("the reset");
    assert_eq!((outcome.session.email.as_str(), outcome.keys), ("grace@example.com", AccountKeys::Created));
    let new_code = reinstalled.core.account_recovery_code().await.unwrap();
    assert_ne!(new_code, old_code, "new keys, a new recovery code");
    assert_eq!(reinstalled.core.account_keys().await.unwrap(), AccountKeys::Unlocked);
    assert!(!reinstalled.core.accounts().await.unwrap().iter().any(|a| a.account == "grace-integration"));
    reinstalled.core.register_device(None).await.expect("the approval role is free");
    reinstalled.core.sync(0).await.expect("the reinstalled phone approves");
    assert!(matches!(old.core.register_device(None).await, Err(CoreError::NotLoggedIn)), "the old phone is out");

    // The new keys are the account's: another phone opens them with the new code only.
    let next = Phone::signed_out(|_| {}).await;
    assert_eq!(next.sso_sign_in(&server.base).await.unwrap().keys, AccountKeys::Locked);
    assert!(next.core.unlock_account(old_code).await.is_err());
    next.core.unlock_account(new_code).await.expect("the new code opens the new keys");
}
