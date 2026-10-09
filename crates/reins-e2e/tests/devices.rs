//! Settings > Devices against the real server: the approval phone lists the account's devices and signs a lost one
//! out, whose sign-in then stops working at once (it can no longer read the vault or answer for the account). Only
//! the approval device may do it, and not to itself.

use reins_core::{CoreError, DeviceKind};
use reins_e2e::{PASSWORD, Phone, Server};

const EMAIL: &str = "lost@example.com";

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_approval_phone_signs_a_lost_phone_out_and_its_sign_in_stops_working() {
    let server = Server::start(20, 8).await;
    server.register(EMAIL).await;
    let approval = Phone::sign_in(&server.base, EMAIL).await;
    // Another phone signed in to the same account: it reads the vault, but is not the approval device.
    let lost = Phone::signed_out(|_| {}).await;
    lost.core.login(server.base.clone(), EMAIL.to_owned(), PASSWORD.to_owned(), None).await.expect("login");
    lost.core.add_token_account("vault".to_owned(), PASSWORD.to_owned()).await.expect("unlock the vault");
    lost.core.vault_items(String::new()).await.expect("the other phone reads the vault");
    assert!(lost.core.devices().await.is_err(), "only the approval device lists the devices");

    let devices = approval.core.devices().await.unwrap();
    assert_eq!(devices.len(), 2, "{devices:?}");
    let me = &devices[0];
    assert!(me.this_device && me.approval, "this phone first: {me:?}");
    let other = devices.iter().find(|d| !d.this_device).unwrap();
    assert!(!other.approval);
    assert_eq!(other.kind, DeviceKind::Phone);
    assert!(other.last_seen_at > 0);

    assert!(approval.core.sign_out_device(me.id.clone()).await.is_err(), "not this phone: that is Sign out");
    approval.core.sign_out_device(other.id.clone()).await.unwrap();

    let refused = lost.core.vault_items(String::new()).await;
    assert!(matches!(refused, Err(CoreError::NotLoggedIn)), "the lost phone's sign-in is over: {refused:?}");
    let left = approval.core.devices().await.unwrap();
    assert_eq!(left.iter().map(|d| d.this_device).collect::<Vec<_>>(), [true]);
    // Already gone is fine; the approval phone keeps working.
    approval.core.sign_out_device(other.id.clone()).await.unwrap();
    approval.core.sync(0).await.expect("the approval phone still syncs");
}
