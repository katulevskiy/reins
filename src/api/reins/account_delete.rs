//! In-app account deletion, `POST {domain_path}/reins/api/account/delete` (App Store guideline 5.1.1(v), Google Play's
//! account deletion policy): the phone deletes its account without a password, an email round-trip or the web vault.
//!
//! Passwordless accounts have no password to ask for, and the server has no re-authentication of its own. What the
//! request must carry instead:
//!
//! - the account's email, typed by the user (`confirm_email`): a deliberate act, for this account;
//! - an access token issued in the last [`DELETION_TOKEN_MAX_AGE_SECS`]: the phone refreshes its token first, so only
//!   the holder of the refresh token (the signed-in app), not a copied bearer token, deletes the account;
//! - the approval device's device key, like every other approval-device call. Another device (or a sign-in to the
//!   identity alone) brings the master password hash of the account secret or master password, as for taking the
//!   approval role. An account with no approval device yet needs neither.
//!
//! Wrong emails and proofs count against the account ([`super::limits::DEVICE_PROOFS`]). The last owner of an
//! organization is refused before anything is deleted, as is everyone when WorkOS cannot delete the user; then the
//! WorkOS user goes (which ends its sessions, so the identity cannot sign in to a new, empty account by itself),
//! and the account with it. Every step tolerates what an earlier, interrupted attempt already deleted; a deletion the
//! database refused is finished by the WorkOS sync ([`super::workos_sync`]).

use reins_proto::device::{AccountDeletion, DELETION_REFUSED, DELETION_TOKEN_MAX_AGE_SECS, codes};
use rocket::{
    Data, Request, Route,
    http::Status,
    request::{FromRequest, Outcome},
};

use super::{
    HUB, account_state,
    device_api::{
        DeviceKey, PhoneResult, api_err, bad_request, internal, is_caller, rate_limited, read_body_limited, user_key,
    },
    limits::DEVICE_PROOFS,
    now_unix, workos_sync,
};
use crate::{
    api::{EmptyResult, core::log_event},
    auth::{Headers, decode_login},
    db::{
        DbConn,
        models::{EventType, Membership, MembershipType, ReinsDevice, ReinsSetting, User, UserId, reins_workos},
    },
    sso_workos,
};

pub fn routes() -> Vec<Route> {
    routes![post_delete_account]
}

/// When the request's access token was issued (its `nbf`); `0` when there is none to read (`Headers` refuses those).
pub struct TokenIssuedAt(i64);

#[rocket::async_trait]
impl<'r> FromRequest<'r> for TokenIssuedAt {
    type Error = ();

    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, ()> {
        let issued = request
            .headers()
            .get_one("Authorization")
            .and_then(|h| h.strip_prefix("Bearer "))
            .and_then(|token| decode_login(token).ok())
            .map_or(0, |claims| claims.nbf);
        Outcome::Success(Self(issued))
    }
}

/// Whether `typed` names the account `email` (case and surrounding spaces do not count).
fn confirms(typed: &str, email: &str) -> bool {
    let typed = typed.trim();
    !typed.is_empty() && typed.eq_ignore_ascii_case(email.trim())
}

/// The approval device, any device of an account without one, or a device with the master password hash.
async fn check_proof(headers: &Headers, key: &DeviceKey, hash: Option<&str>, conn: &DbConn) -> PhoneResult<()> {
    match ReinsDevice::find_by_user(&headers.user.uuid, conn).await {
        None => return Ok(()),
        Some(row) if is_caller(&row, headers, key) => return Ok(()),
        Some(_) => {}
    }
    let Some(hash) = hash.filter(|h| !h.is_empty()) else {
        return Err(api_err(Status::Forbidden, codes::PROOF_REQUIRED, DELETION_REFUSED));
    };
    if headers.user.check_valid_password(hash) {
        return Ok(());
    }
    DEVICE_PROOFS.fail(&user_key(headers));
    warn!("Reins: a device of user {} sent a wrong proof to delete the account", headers.user.uuid);
    Err(api_err(Status::Forbidden, codes::WRONG_PROOF, DELETION_REFUSED))
}

/// Whether the account is the only owner of an organization (Vaultwarden refuses to delete it then).
pub async fn last_owner(uuid: &UserId, conn: &DbConn) -> bool {
    for member in Membership::find_confirmed_by_user(uuid, conn).await {
        if member.atype == MembershipType::Owner
            && Membership::count_confirmed_by_org_and_type(&member.org_uuid, MembershipType::Owner, conn).await <= 1
        {
            return true;
        }
    }
    false
}

/// Deletes account `uuid` and everything kept for it: what the relay holds in memory and its files, the approval
/// device, the AI and desktop connections with their refresh tokens, the SSO sessions and identity, the encrypted
/// account state, then the Vaultwarden account (vault, folders, sends, devices and their push tokens, two-step
/// login, memberships). What is already gone is skipped, so it can run again after a failure.
///
/// The Reins data goes even when Vaultwarden then refuses (the last owner of an organization): callers that must not
/// lose it check [`last_owner`] first, as [`delete_user`] does.
pub async fn erase(uuid: &UserId, conn: &DbConn) -> EmptyResult {
    HUB.forget_user(uuid.as_ref());
    reins_workos::delete_reins_data(uuid, conn).await?;
    if let Err(e) = account_state::forget(uuid.as_ref()) {
        err!(format!("Could not delete the account state: {e}"));
    }
    if let Some(user) = reins_workos::user_by_id(uuid, conn).await? {
        user.delete(conn).await?;
    }
    Ok(())
}

/// Vaultwarden's own account deletions (the web vault, the emailed deletion link, the admin page): [`erase`], so
/// that what Reins keeps goes too, but never for the last owner of an organization.
pub async fn delete_user(user: User, conn: &DbConn) -> EmptyResult {
    if last_owner(&user.uuid, conn).await {
        err!("Can't delete last owner")
    }
    erase(&user.uuid, conn).await
}

/// Deletes the caller's account (see the module documentation); 204 when it is gone.
#[post("/reins/api/account/delete", data = "<data>")]
async fn post_delete_account(
    data: Data<'_>,
    headers: Headers,
    key: DeviceKey,
    issued: TokenIssuedAt,
    conn: DbConn,
) -> PhoneResult<Status> {
    let user = user_key(&headers);
    if let Err(wait) = DEVICE_PROOFS.check(&user) {
        return Err(rate_limited("too many wrong confirmations for this account", wait));
    }
    if now_unix().saturating_sub(issued.0) > DELETION_TOKEN_MAX_AGE_SECS {
        return Err(api_err(
            Status::Forbidden,
            codes::FRESH_TOKEN_REQUIRED,
            "Refresh the access token, then send the request again",
        ));
    }
    let deletion: AccountDeletion = serde_json::from_slice(&read_body_limited(data, 4096).await?)
        .map_err(|e| bad_request(format!("invalid body: {e}")))?;
    if !confirms(&deletion.confirm_email, &headers.user.email) {
        DEVICE_PROOFS.fail(&user);
        return Err(api_err(
            Status::BadRequest,
            codes::CONFIRMATION_MISMATCH,
            "Type this account's email address to confirm",
        ));
    }
    check_proof(&headers, &key, deletion.master_password_hash.as_deref(), &conn).await?;
    let uuid = headers.user.uuid.clone();
    if last_owner(&uuid, &conn).await {
        return Err(api_err(
            Status::Conflict,
            codes::LAST_OWNER,
            "This account is the only owner of an organization. Give it another owner or delete it in the web \
             vault first.",
        ));
    }

    // WorkOS first: if it cannot delete the user, nothing is deleted here either and the phone may try again.
    let workos_user = if sso_workos::enabled() {
        let identifier = reins_workos::identifier_of(&uuid, &conn).await.map_err(|e| internal(&e))?;
        identifier.as_deref().and_then(sso_workos::user_id_of).map(str::to_owned)
    } else {
        None
    };
    if let Some(id) = &workos_user
        && let Err(e) = sso_workos::delete_user(id).await
    {
        error!("Reins: account {uuid} could not be deleted: {e}");
        return Err(api_err(
            Status::BadGateway,
            codes::PROVIDER_UNAVAILABLE,
            "The sign-in service could not delete your account right now; nothing was deleted. Try again later.",
        ));
    }

    // The WorkOS user is gone; keep the account's id until it is gone too, so the sync finishes a deletion the
    // database refused now.
    let pending =
        workos_user.is_some().then(|| workos_sync::pending_deletion(&uuid)).filter(|_| workos_sync::enabled());
    if let Some(pending) = &pending {
        ReinsSetting::set(pending, uuid.as_ref(), &conn).await.map_err(|e| internal(&e))?;
    }
    let memberships = Membership::find_any_state_by_user(&uuid, &conn).await;
    erase(&uuid, &conn).await.map_err(|e| internal(&e))?;
    if let Some(pending) = &pending {
        ReinsSetting::remove(pending, &conn).await.map_err(|e| internal(&e))?;
    }
    for membership in memberships {
        log_event(
            EventType::OrganizationUserDeleted,
            &membership.uuid,
            &membership.org_uuid,
            &uuid,
            headers.device.atype,
            &headers.ip.ip,
            &conn,
        )
        .await;
    }
    info!(
        "Reins: account {uuid} was deleted at its owner's request from device {} ({}){}",
        headers.device.uuid,
        headers.ip.ip,
        if workos_user.is_some() {
            ", with its WorkOS user"
        } else {
            ""
        }
    );
    Ok(Status::NoContent)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_typed_email_must_name_the_account() {
        assert!(confirms("ada@example.com", "ada@example.com"));
        assert!(confirms("  Ada@Example.COM \n", "ada@example.com"));
        assert!(!confirms("", ""));
        assert!(!confirms(" ", "ada@example.com"));
        assert!(!confirms("ada@example.co", "ada@example.com"));
        assert!(!confirms("DELETE", "ada@example.com"));
    }
}
