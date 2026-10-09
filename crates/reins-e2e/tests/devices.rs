//! Settings > Devices against the real server: the approval phone lists the account's devices and signs a lost one
//! out, with the master password (or recovery code) typed then. The lost phone's sign-in stops working at once (it can
//! no longer read the vault or answer for the account), and it cannot sign back in with the same device id. The
//! approval device may not sign itself out this way, a wrong proof does nothing, and a device id the phone could not
//! name is refused at sign-in.

use reins_core::crypto::{Kdf, master_key, master_password_hash};
use reins_core::{CoreError, DeviceKind};
use reins_e2e::{PASSWORD, Phone, Server};

const EMAIL: &str = "lost@example.com";

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_approval_phone_signs_a_lost_phone_out_and_it_cannot_come_back() {
    let server = Server::start(20, 8).await;
    server.register(EMAIL).await;
    let approval = Phone::sign_in(&server.base, EMAIL).await;
    // Another phone signed in to the same account: it reads the vault, but is not the approval device.
    let lost = Phone::signed_out(|_| {}).await;
    lost.core.login(server.base.clone(), EMAIL.to_owned(), PASSWORD.to_owned(), None).await.expect("login");
    lost.core.add_token_account("vault".to_owned(), PASSWORD.to_owned()).await.expect("unlock the vault");
    lost.core.vault_items(String::new()).await.expect("the other phone reads the vault");
    assert!(lost.core.devices().await.is_err(), "the Reins call is for the approval device");

    let devices = approval.core.devices().await.unwrap();
    assert_eq!(devices.len(), 2, "{devices:?}");
    let me = &devices[0];
    assert!(me.this_device && me.approval, "this phone first: {me:?}");
    let other = devices.iter().find(|d| !d.this_device).unwrap();
    assert!(!other.approval);
    assert_eq!(other.kind, DeviceKind::Phone);
    assert!(other.last_seen_at > 0);

    // Not this phone (that is Sign out), and not without the master password typed now.
    assert!(approval.core.sign_out_device(me.id.clone(), PASSWORD.to_owned()).await.is_err());
    let wrong = approval.core.sign_out_device(other.id.clone(), "not the password".to_owned()).await;
    assert!(matches!(wrong, Err(CoreError::Invalid { .. })), "{wrong:?}");
    lost.core.vault_items(String::new()).await.expect("a wrong proof changes nothing");

    approval.core.sign_out_device(other.id.clone(), PASSWORD.to_owned()).await.unwrap();
    let refused = lost.core.vault_items(String::new()).await;
    assert!(matches!(refused, Err(CoreError::NotLoggedIn)), "the lost phone's sign-in is over: {refused:?}");
    let left = approval.core.devices().await.unwrap();
    assert_eq!(left.iter().map(|d| d.this_device).collect::<Vec<_>>(), [true]);

    // The lost phone signs in again with the keys it kept and the same device id: refused, and told by whom.
    let back = lost.core.login(server.base.clone(), EMAIL.to_owned(), PASSWORD.to_owned(), None).await;
    let told = format!("{back:?}");
    assert!(back.is_err() && told.contains("signed out of the account"), "{told}");

    // Already gone is fine; the approval phone keeps working.
    approval.core.sign_out_device(other.id.clone(), PASSWORD.to_owned()).await.unwrap();
    approval.core.sync(0).await.expect("the approval phone still syncs");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_device_id_the_phone_could_not_sign_out_is_refused_at_sign_in() {
    let server = Server::start(20, 8).await;
    server.register(EMAIL).await;
    let hash = tokio::task::spawn_blocking(|| {
        let key = master_key(
            PASSWORD,
            EMAIL,
            Kdf::Pbkdf2 {
                iterations: 600_000,
            },
        )
        .unwrap();
        master_password_hash(&key, PASSWORD).to_string()
    })
    .await
    .unwrap();
    let client = reqwest::Client::new();
    for id in ["evil.dev", &"a".repeat(65)] {
        let form = [
            ("grant_type", "password"),
            ("username", EMAIL),
            ("password", hash.as_str()),
            ("scope", "api offline_access"),
            ("client_id", "mobile"),
            ("deviceType", "0"),
            ("deviceIdentifier", id),
            ("deviceName", "Phone"),
        ];
        let r = client.post(server.url("/identity/connect/token")).form(&form).send().await.unwrap();
        let status = r.status();
        let body = r.text().await.unwrap_or_default();
        assert!(!status.is_success() && body.contains("device id"), "{id}: {status} {body}");
    }
}
