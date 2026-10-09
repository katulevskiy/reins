//! Passkeys that open the account's vault (`reins_proto::vault_passkey`): the sealed copies of the account secret the
//! phones keep here, one per passkey. Reading them only needs a session (they are useless without the passkey);
//! adding or removing one changes how the vault can be opened, so it takes the master password hash of the account
//! secret, like taking the approval role over, with the same limit on wrong attempts.

use reins_proto::vault_passkey::{
    MAX_CREDENTIAL_ID_CHARS, MAX_NAME_CHARS, MAX_PASSKEYS, MAX_WRAPPED_CHARS, NewVaultPasskey, RemoveVaultPasskey,
    VaultPasskey, VaultPasskeys, codes, is_base64url,
};
use rocket::{Data, Route, http::Status, serde::json::Json};

use super::{
    device_api::{PhoneResult, api_err, bad_request, internal, rate_limited, read_body_limited, user_key},
    now_unix,
};
use crate::{
    auth::Headers,
    db::{DbConn, models::ReinsVaultPasskey},
};

/// Largest request body accepted.
const MAX_BODY_BYTES: u64 = 4096;

pub fn routes() -> Vec<Route> {
    routes![get_passkeys, post_passkey, post_remove]
}

async fn list(headers: &Headers, conn: &DbConn) -> Json<VaultPasskeys> {
    let passkeys = ReinsVaultPasskey::find_by_user(&headers.user.uuid, conn)
        .await
        .into_iter()
        .map(|p| VaultPasskey {
            credential_id: p.credential_id,
            wrapped: p.wrapped,
            name: p.name,
            created_at: p.created_at,
        })
        .collect();
    Json(VaultPasskeys {
        passkeys,
    })
}

/// Only someone holding the account secret can make its master password hash. Wrong hashes count against the account
/// (`limits::DEVICE_PROOFS`), together with wrong takeover proofs.
fn check_proof(headers: &Headers, master_password_hash: &str) -> PhoneResult<()> {
    let user = user_key(headers);
    if let Err(wait) = super::limits::DEVICE_PROOFS.check(&user) {
        return Err(rate_limited("too many wrong recovery codes or passwords for this account", wait));
    }
    if !master_password_hash.is_empty() && headers.user.check_valid_password(master_password_hash) {
        return Ok(());
    }
    super::limits::DEVICE_PROOFS.fail(&user);
    warn!("Reins: a device of user {user} sent a wrong proof to change the vault's passkeys");
    Err(api_err(
        Status::Forbidden,
        reins_proto::device::codes::WRONG_PROOF,
        "Only a phone that can open the vault can change its passkeys.",
    ))
}

fn credential_id(raw: &str) -> PhoneResult<&str> {
    if is_base64url(raw, MAX_CREDENTIAL_ID_CHARS) {
        Ok(raw)
    } else {
        Err(bad_request("invalid credential id"))
    }
}

#[get("/reins/api/vault-passkeys")]
async fn get_passkeys(headers: Headers, conn: DbConn) -> Json<VaultPasskeys> {
    list(&headers, &conn).await
}

#[post("/reins/api/vault-passkeys", data = "<data>")]
async fn post_passkey(data: Data<'_>, headers: Headers, conn: DbConn) -> PhoneResult<Json<VaultPasskeys>> {
    let body = read_body_limited(data, MAX_BODY_BYTES).await?;
    let new: NewVaultPasskey = serde_json::from_slice(&body).map_err(|e| bad_request(format!("invalid body: {e}")))?;
    let id = credential_id(&new.credential_id)?.to_owned();
    if !is_base64url(&new.wrapped, MAX_WRAPPED_CHARS) {
        return Err(bad_request("invalid sealed secret"));
    }
    let name: String = new.name.trim().chars().filter(|c| !c.is_control()).take(MAX_NAME_CHARS).collect();
    check_proof(&headers, &new.master_password_hash)?;
    let row = ReinsVaultPasskey::new(
        headers.user.uuid.clone(),
        id,
        new.wrapped,
        if name.is_empty() {
            "Passkey".to_owned()
        } else {
            name
        },
        now_unix(),
    );
    if !row.save(MAX_PASSKEYS, &conn).await.map_err(|e| internal(&e))? {
        return Err(api_err(
            Status::Conflict,
            codes::TOO_MANY,
            format!("an account keeps at most {MAX_PASSKEYS} passkeys"),
        ));
    }
    Ok(list(&headers, &conn).await)
}

#[post("/reins/api/vault-passkeys/remove", data = "<data>")]
async fn post_remove(data: Data<'_>, headers: Headers, conn: DbConn) -> PhoneResult<Json<VaultPasskeys>> {
    let body = read_body_limited(data, MAX_BODY_BYTES).await?;
    let remove: RemoveVaultPasskey =
        serde_json::from_slice(&body).map_err(|e| bad_request(format!("invalid body: {e}")))?;
    let id = credential_id(&remove.credential_id)?;
    check_proof(&headers, &remove.master_password_hash)?;
    ReinsVaultPasskey::delete(&headers.user.uuid, id, &conn).await.map_err(|e| internal(&e))?;
    Ok(list(&headers, &conn).await)
}
