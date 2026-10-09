//! A passkey added on the phone that holds the account secret opens the vault on a fresh install, where the account
//! is `Locked`: in place of the other phone or the recovery code.
mod common;

use std::sync::{Arc, Mutex};

use data_encoding::BASE64URL_NOPAD;
use reins_core::sso::AccountSecret;
use reins_core::{AccountKeys, CoreConfig, CoreError, GoogleTokenProvider, Notifier, ReinsCore};
use reins_proto::vault_passkey::{NewVaultPasskey, RemoveVaultPasskey, VaultPasskey, VaultPasskeys};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

const USER_ID: &str = "0b5c-user";
const EMAIL: &str = "me@example.com";
const CREDENTIAL: &[u8] = b"credential-1";
const PRF: [u8; 32] = [42; 32];

fn token() -> String {
    let claims = json!({"sub": USER_ID, "email": EMAIL, "exp": 4_000_000_000_i64}).to_string();
    format!("h.{}.s", BASE64URL_NOPAD.encode(claims.as_bytes()))
}

/// The server's side: the account's keys (made with `secret`) and the vault passkeys, checked like the real routes.
struct Account {
    wrapped_key: String,
    proof: String,
    passkeys: Mutex<Vec<VaultPasskey>>,
}

async fn serve(server: &MockServer, secret: &AccountSecret) -> Arc<Account> {
    let master = secret.master_key().unwrap();
    let account = Arc::new(Account {
        wrapped_key: reins_core::sso::new_keys(secret).unwrap().key,
        proof: secret.master_password_hash(&master).to_string(),
        passkeys: Mutex::default(),
    });
    let key = account.wrapped_key.clone();
    Mock::given(method("POST"))
        .and(path("/identity/connect/token"))
        .respond_with(move |_: &Request| {
            ResponseTemplate::new(200).set_body_json(json!({
                "access_token": token(), "refresh_token": "REFRESH", "expires_in": 7200, "Key": key}))
        })
        .mount(server)
        .await;
    let key = account.wrapped_key.clone();
    Mock::given(method("GET"))
        .and(path("/api/accounts/profile"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": USER_ID, "email": EMAIL, "key": key})))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/reins/api/account-state"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"revision": 0, "ciphertext": null})))
        .mount(server)
        .await;
    let list = |a: &Account| VaultPasskeys {
        passkeys: a.passkeys.lock().unwrap().clone(),
    };
    let a = Arc::clone(&account);
    Mock::given(method("GET"))
        .and(path("/reins/api/vault-passkeys"))
        .respond_with(move |_: &Request| ResponseTemplate::new(200).set_body_json(list(&a)))
        .mount(server)
        .await;
    let a = Arc::clone(&account);
    Mock::given(method("POST"))
        .and(path("/reins/api/vault-passkeys"))
        .respond_with(move |request: &Request| {
            let new: NewVaultPasskey = serde_json::from_slice(&request.body).unwrap();
            if new.master_password_hash != a.proof {
                return ResponseTemplate::new(403).set_body_json(json!({"error": "wrong_proof", "message": "no"}));
            }
            let mut passkeys = a.passkeys.lock().unwrap();
            passkeys.retain(|p| p.credential_id != new.credential_id);
            passkeys.push(VaultPasskey {
                credential_id: new.credential_id,
                wrapped: new.wrapped,
                name: new.name,
                created_at: 1,
            });
            drop(passkeys);
            ResponseTemplate::new(200).set_body_json(list(&a))
        })
        .mount(server)
        .await;
    let a = Arc::clone(&account);
    Mock::given(method("POST"))
        .and(path("/reins/api/vault-passkeys/remove"))
        .respond_with(move |request: &Request| {
            let remove: RemoveVaultPasskey = serde_json::from_slice(&request.body).unwrap();
            if remove.master_password_hash != a.proof {
                return ResponseTemplate::new(403).set_body_json(json!({"error": "wrong_proof", "message": "no"}));
            }
            a.passkeys.lock().unwrap().retain(|p| p.credential_id != remove.credential_id);
            ResponseTemplate::new(200).set_body_json(list(&a))
        })
        .mount(server)
        .await;
    account
}

fn core(dir: &std::path::Path) -> Arc<ReinsCore> {
    let google: Arc<dyn GoogleTokenProvider> = Arc::new(common::FakeGoogle::new());
    let notifier: Arc<dyn Notifier> = Arc::new(common::RecordingNotifier::default());
    ReinsCore::with_config(dir.to_str().unwrap(), &common::FakeKeys, google, notifier, CoreConfig::default()).unwrap()
}

/// A phone signed in through the browser; the account's keys are not open on it yet.
async fn signed_in(server: &MockServer, dir: &std::path::Path) -> Arc<ReinsCore> {
    let app = core(dir);
    let start = app.sso_begin(server.uri()).await.unwrap();
    let callback = format!("com.reins2fa.app://sso-callback?code=c&state={}", start.state);
    let outcome = app.sso_finish(server.uri(), callback, start.state, start.verifier).await.unwrap();
    assert_eq!(outcome.keys, AccountKeys::Locked);
    app
}

#[tokio::test]
async fn a_passkey_added_on_one_phone_opens_the_vault_on_a_fresh_install() {
    let server = MockServer::start().await;
    let secret = AccountSecret::generate().unwrap();
    let account = serve(&server, &secret).await;

    // The phone that holds the secret (here it was opened with the recovery code) adds the passkey.
    let first = tempfile::tempdir().unwrap();
    let phone = signed_in(&server, first.path()).await;
    phone.unlock_account(secret.recovery_code().to_string()).await.unwrap();
    let options = phone.vault_passkey_options().await.unwrap();
    assert_eq!(options.rp_id, "127.0.0.1");
    assert_eq!(options.user_handle, USER_ID.as_bytes());
    assert_eq!(options.prf_salt.len(), 32);
    assert!(options.credential_ids.is_empty());
    let list = phone.add_vault_passkey(CREDENTIAL.to_vec(), PRF.to_vec(), "Pixel 9".to_owned()).await.unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!((list[0].credential_id.as_slice(), list[0].name.as_str()), (CREDENTIAL, "Pixel 9"));
    let kept = account.passkeys.lock().unwrap()[0].wrapped.clone();
    assert!(!kept.contains(&BASE64URL_NOPAD.encode(secret.as_bytes())), "the server never sees the secret");

    // A fresh install of the same account: locked, and its passkey is offered.
    let second = tempfile::tempdir().unwrap();
    let fresh = signed_in(&server, second.path()).await;
    assert_eq!(fresh.vault_passkey_options().await.unwrap().credential_ids, vec![CREDENTIAL.to_vec()]);
    let wrong = fresh.unlock_with_vault_passkey(CREDENTIAL.to_vec(), [7; 32].to_vec()).await.unwrap_err();
    assert!(matches!(&wrong, CoreError::Invalid { .. }), "{wrong:?}");
    assert_eq!(fresh.account_keys().await.unwrap(), AccountKeys::Locked, "another passkey's output opens nothing");
    fresh.unlock_with_vault_passkey(CREDENTIAL.to_vec(), PRF.to_vec()).await.unwrap();
    assert_eq!(fresh.account_keys().await.unwrap(), AccountKeys::Unlocked);
    assert_eq!(fresh.account_recovery_code().await.unwrap(), secret.recovery_code().to_string(), "as with the code");

    // The phone that opened it may manage them too; removing it leaves nothing to unlock with.
    assert!(fresh.remove_vault_passkey(CREDENTIAL.to_vec()).await.unwrap().is_empty());
    assert!(phone.vault_passkeys().await.unwrap().is_empty());
}

#[tokio::test]
async fn only_a_phone_that_holds_the_secret_can_add_one() {
    let server = MockServer::start().await;
    let secret = AccountSecret::generate().unwrap();
    let account = serve(&server, &secret).await;
    let dir = tempfile::tempdir().unwrap();
    let locked = signed_in(&server, dir.path()).await;
    let err = locked.add_vault_passkey(CREDENTIAL.to_vec(), PRF.to_vec(), "Pixel".to_owned()).await.unwrap_err();
    assert!(matches!(&err, CoreError::Invalid { reason } if reason.contains("Open the vault")), "{err:?}");
    assert!(account.passkeys.lock().unwrap().is_empty());
    let err = locked.unlock_with_vault_passkey(CREDENTIAL.to_vec(), PRF.to_vec()).await.unwrap_err();
    assert!(matches!(&err, CoreError::Invalid { reason } if reason.contains("was not added")), "{err:?}");
}
