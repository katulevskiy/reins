//! Resetting the vault, for an account whose keys no phone and no recovery code can open any more (a reinstalled app
//! with the code lost): without it that account is locked out for good.
//!
//! What the lost keys protected goes: the vault (items, folders, sends), the keys themselves, the encrypted account
//! state, emergency access and organization memberships (their keys were wrapped with the lost ones), the approval
//! device and every other device's sign-in. So do the AI and desktop connections: whoever resets becomes the approval
//! device, and must not get requests from AIs someone else connected. The account stays: its WorkOS identity, email
//! and two-step login. The phone then makes new keys exactly as for a new account (`/api/accounts/set-password`).
//!
//! The proof is a fresh sign-in: the calling device must have finished a phone app's WorkOS sign-in (for which WorkOS
//! asks for the sign-in method again) within [`RESET_SIGN_IN_TTL_SECS`], and the request must carry an access token
//! issued by that sign-in or later, not one the device held before it. One sign-in allows one reset ([`note_sign_in`],
//! called from `sso::redeem`). Holding a signed-in phone, or a copy of its token, is not enough.

use std::{
    collections::HashMap,
    sync::{LazyLock, Mutex, PoisonError},
};

use reins_proto::{
    device::{RESET_SIGN_IN_TTL_SECS, codes},
    pairing::{PushKind, PushMessage},
};
use rocket::{Route, State, http::Status, serde::json::Json};
use serde_json::{Value, json};

use super::{
    HUB, account_delete, account_state,
    device_api::{PhoneResult, api_err, internal},
    now_unix, push,
};
use crate::{
    auth::Headers,
    crypto,
    db::{
        DbConn, DbPool,
        models::{
            Cipher, DeviceId, EmergencyAccess, Favorite, Folder, Membership, ReinsDevice, ReinsVaultPasskey, Send,
            TwoFactorIncomplete, UserId, reins_workos,
        },
    },
};

pub fn routes() -> Vec<Route> {
    routes![post_reset]
}

/// When each (user, device) last finished an SSO sign-in. In memory: a restart only means signing in again.
static SIGN_INS: LazyLock<Mutex<HashMap<(String, String), i64>>> = LazyLock::new(Mutex::default);

/// A device just finished an SSO sign-in to `user`'s account.
pub fn note_sign_in(user: &UserId, device: &DeviceId) {
    let (user, device) = (user.to_string(), device.to_string());
    record_sign_in(&mut SIGN_INS.lock().unwrap_or_else(PoisonError::into_inner), &user, &device, now_unix());
}

fn record_sign_in(map: &mut HashMap<(String, String), i64>, user: &str, device: &str, now: i64) {
    map.retain(|_, at| now - *at <= RESET_SIGN_IN_TTL_SECS);
    map.insert((user.to_owned(), device.to_owned()), now);
}

/// How much earlier than the sign-in's record its access token may say it was issued (WorkOS issues the token a
/// moment before the server records the sign-in, and clocks differ a little).
const ISSUED_SLACK_SECS: i64 = 60;

/// Spends the device's sign-in on a reset: true when it was recent enough and the request's access token (issued at
/// `issued`) came from it or later.
fn take_sign_in(map: &mut HashMap<(String, String), i64>, user: &str, device: &str, issued: i64, now: i64) -> bool {
    let Some(at) = map.get(&(user.to_owned(), device.to_owned())).copied() else {
        return false;
    };
    if issued < at - ISSUED_SLACK_SECS {
        // A token from before the sign-in: refused, and the sign-in stays for the token it issued.
        return false;
    }
    map.remove(&(user.to_owned(), device.to_owned()));
    (0..=RESET_SIGN_IN_TTL_SECS).contains(&(now - at))
}

/// Wipes the caller's vault and keys (see the module docs). 403 `reauth_required` without a fresh sign-in on this
/// device; 409 when the account is the last owner of an organization (its other members would lose it).
#[post("/reins/api/account/reset")]
async fn post_reset(
    headers: Headers,
    issued: account_delete::TokenIssuedAt,
    conn: DbConn,
    pool: &State<DbPool>,
) -> PhoneResult<Json<Value>> {
    let mut user = headers.user;
    let device = headers.device;

    if account_delete::last_owner(&user.uuid, &conn).await {
        return Err(api_err(
            Status::Conflict,
            codes::LAST_OWNER,
            "This account is the only owner of an organization. Give it another owner in the web vault first.",
        ));
    }

    let fresh = take_sign_in(
        &mut SIGN_INS.lock().unwrap_or_else(PoisonError::into_inner),
        &user.uuid.to_string(),
        &device.uuid.to_string(),
        issued.0,
        now_unix(),
    );
    if !fresh {
        return Err(api_err(Status::Forbidden, codes::REAUTH_REQUIRED, "Sign in again to confirm the reset."));
    }

    let approval = ReinsDevice::find_by_user(&user.uuid, &conn).await;

    let wipe = async {
        Send::delete_all_by_user(&user.uuid, &conn).await?;
        EmergencyAccess::delete_all_by_user(&user.uuid, &conn).await?;
        EmergencyAccess::delete_all_by_grantee_email(&user.email, &conn).await?;
        Membership::delete_all_by_user(&user.uuid, &conn).await?;
        Cipher::delete_all_by_user(&user.uuid, &conn).await?;
        Favorite::delete_all_by_user(&user.uuid, &conn).await?;
        Folder::delete_all_by_user(&user.uuid, &conn).await?;
        TwoFactorIncomplete::delete_all_by_user(&user.uuid, &conn).await?;
        reins_workos::reset_devices(&user.uuid, &device.uuid, &conn).await?;
        // They sealed the old account secret, which opens nothing now.
        ReinsVaultPasskey::delete_all(&user.uuid, &conn).await?;

        // As a new SSO account: no master password and no keys, so `set-password` takes new ones.
        user.password_hash = Vec::new();
        user.salt = crypto::get_random_bytes::<64>().to_vec();
        user.password_hint = None;
        user.akey = String::new();
        user.private_key = None;
        user.public_key = None;
        user.save(&conn).await
    };
    wipe.await.map_err(|e| internal(&e))?;

    if let Err(e) = account_state::forget(user.uuid.as_ref()) {
        error!("Reins: could not delete the account state of {} after a reset: {e}", user.uuid);
        return Err(api_err(Status::InternalServerError, codes::INTERNAL, "Server error, please retry"));
    }
    HUB.forget_user(user.uuid.as_ref());

    // The phone that approved for the account is signed out already; tell it so it says why.
    if let Some(old) = approval.filter(|old| old.device_uuid != device.uuid) {
        push::spawn_push(
            pool.inner().clone(),
            user.uuid.clone(),
            old.fcm_token,
            PushMessage {
                t: PushKind::Replaced,
                id: String::new(),
            },
        );
    }
    warn!("Reins: user {} reset their vault from device {}", user.uuid, device.uuid);
    Ok(Json(json!({})))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reset_needs_a_recent_sign_in_of_this_device_and_spends_it() {
        let mut map = HashMap::new();
        assert!(!take_sign_in(&mut map, "u1", "d1", 100, 100), "never signed in");
        record_sign_in(&mut map, "u1", "d1", 1000);
        assert!(!take_sign_in(&mut map, "u1", "d2", 1000, 1001), "another device");
        assert!(!take_sign_in(&mut map, "u2", "d1", 1000, 1001), "another account");
        assert!(!take_sign_in(&mut map, "u1", "d1", 900, 1001), "a token from before the sign-in");
        assert!(take_sign_in(&mut map, "u1", "d1", 1000 - ISSUED_SLACK_SECS, 1000 + RESET_SIGN_IN_TTL_SECS));
        assert!(!take_sign_in(&mut map, "u1", "d1", 1001, 1001), "once per sign-in");

        record_sign_in(&mut map, "u1", "d1", 2000);
        assert!(!take_sign_in(&mut map, "u1", "d1", 2001, 2001 + RESET_SIGN_IN_TTL_SECS), "too late");
    }

    #[test]
    fn old_sign_ins_are_forgotten() {
        let mut map = HashMap::new();
        record_sign_in(&mut map, "u1", "d1", 100);
        record_sign_in(&mut map, "u2", "d2", 101 + RESET_SIGN_IN_TTL_SECS);
        assert_eq!(map.len(), 1);
    }
}
