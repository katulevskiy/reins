//! The signed web approval (05 §6: "Slack and Teams buttons can be enabled for L0–L2 reviews. They open a signed web
//! approval that requires SSO plus device-bound passkeys. L3 always needs the app.").
//!
//! - **The link** carries a one-time token: 32 random bytes, stored as SHA-256, bound to one member and one request,
//!   valid until the review's deadline or an hour, whichever is first. It's issued only for L0–L2 and only to a member
//!   the review is routed to ([`issue_token`]).
//! - **The page** needs that member's own console SSO session (anyone else is told "This review is for someone else").
//!   Approve and Deny each ask the member's passkey to sign `webauthn_challenge(review_message(…))`, so the assertion
//!   binds the org, the request, its digest, the decision, the policy snapshot and the time.
//! - **The passkey** must be device-bound (not backup eligible), user-verified, registered in the console and
//!   confirmed on the member's enrolled phone ([`confirm_passkey`]); only then is it in the issuer-signed roster,
//!   where executors check it again (`reins-executor` `verify`).
//! - Using the token and recording the answer happen in one transaction: a refused answer leaves the link usable, a
//!   recorded one spends it.

use reins_proto::managed::{
    ReviewDecision, ReviewSubject, WebAuthnProof, channel_allowed, passkey_confirm_message, passkey_fingerprint,
    passkey_spki_hash, review_message, webauthn_challenge,
};
use rocket::{
    Route,
    form::{Form, FromForm},
    http::{CookieJar, Status},
    response::{Redirect, status::Custom},
    serde::json::Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{
    OrgError, audit,
    console::{self, Page, check_csrf, not_found, signed},
    enabled, member_of,
    reviews::{self, Answered, WebAnswer},
    routing::{self, Directory},
    web_approval_pages as pages,
};
use crate::{
    CONFIG,
    api::reins::now_unix,
    auth::Headers,
    db::{
        DbConn, DbConnInner, DbPool,
        models::{
            UserId,
            reins_org_request::{self, request_state},
            reins_org_routing::{self as store, ReinsOrgWebPasskey, ReinsOrgWebToken},
        },
    },
};

/// The longest a link lives.
pub const TOKEN_SECS: i64 = 3600;

pub fn routes() -> Vec<Route> {
    routes![
        approve_page,
        approve_post,
        passkey_page,
        passkey_post,
        passkey_revoke,
        api_passkeys,
        api_confirm,
        api_delete
    ]
}

fn token_hash(token: &str) -> String {
    reins_proto::canon::hex32(&reins_proto::canon::sha256(token.as_bytes()))
}

/// The web approval link of a token.
#[must_use]
pub fn token_url(token: &str) -> String {
    format!("{}/reins/org/approve/{token}", CONFIG.domain())
}

/// A one-time link for `member` to review `request`, when it may be reviewed on the web (L0–L2) and is routed to them.
pub fn issue_token(
    c: &mut DbConnInner,
    org: &str,
    request: &str,
    member: &str,
    now: i64,
) -> Result<Option<String>, OrgError> {
    let Some(row) = reins_org_request::q_request(c, org, request)? else {
        return Ok(None);
    };
    let trace: Value = serde_json::from_str(&row.trace).unwrap_or_default();
    if row.state != request_state::REVIEWING
        || row.expires_at <= now
        || !channel_allowed(trace["level"].as_str().unwrap_or_default())
        || member == row.user_uuid
    {
        return Ok(None);
    }
    let obligations: Vec<reins_policy::org::Obligation> = serde_json::from_str(&row.obligations).unwrap_or_default();
    let resolved = routing::stored(row.resolved.as_deref());
    let dir = Directory::load(c, org, now)?;
    if !obligations.iter().any(|o| dir.eligible(o, &row.user_uuid, resolved.get(&o.id)).contains(member)) {
        return Ok(None);
    }
    let token = super::random_token();
    store::q_insert_token(
        c,
        &ReinsOrgWebToken {
            token_hash: token_hash(&token),
            org_uuid: org.to_owned(),
            request_uuid: request.to_owned(),
            user_uuid: member.to_owned(),
            expires_at: row.expires_at.min(now + TOKEN_SECS),
            used_at: None,
        },
    )?;
    Ok(Some(token))
}

/// What a link opens, for its member.
#[derive(Debug)]
pub enum Opened {
    /// Not a link (or used, or expired).
    Gone,
    /// Someone else's review.
    NotYours,
    /// L3, or no longer reviewable on the web.
    UseTheApp,
    Card(Box<pages::Card>),
}

/// Reads the link `token` for the signed-in `user`.
pub fn open(c: &mut DbConnInner, token: &str, user: &str, now: i64) -> Result<Opened, OrgError> {
    let Some(t) = store::q_token(c, &token_hash(token))? else {
        return Ok(Opened::Gone);
    };
    if t.used_at.is_some() || t.expires_at <= now {
        return Ok(Opened::Gone);
    }
    if !crate::crypto::ct_eq(&t.user_uuid, user) {
        return Ok(Opened::NotYours);
    }
    let Some(row) = reins_org_request::q_request(c, &t.org_uuid, &t.request_uuid)? else {
        return Ok(Opened::Gone);
    };
    let trace: Value = serde_json::from_str(&row.trace).unwrap_or_default();
    if row.state != request_state::REVIEWING {
        return Ok(Opened::Gone);
    }
    if !channel_allowed(trace["level"].as_str().unwrap_or_default()) {
        return Ok(Opened::UseTheApp);
    }
    let item = reviews::items_for(c, &t.org_uuid, user, now)?.into_iter().find(|i| i.id == row.uuid);
    let Some(item) = item else {
        return Ok(Opened::Gone);
    };
    let passkeys: Vec<String> =
        store::q_usable_passkeys(c, &t.org_uuid, user)?.into_iter().map(|p| p.credential_id).collect();
    let challenge = |d: ReviewDecision| {
        webauthn_challenge(&review_message(
            &t.org_uuid,
            ReviewSubject::Request,
            &row.uuid,
            &row.digest,
            d,
            &row.snapshot,
            now,
        ))
    };
    let lines = trace["lines"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_owned).collect();
    Ok(Opened::Card(Box::new(pages::Card {
        org_name: item.org_name,
        requester: item.requester,
        connection: item.connection,
        lines,
        needs: item.needs,
        progress: item.progress,
        level: item.level,
        reason: item.reason,
        expires_at: row.expires_at,
        at: now,
        approve_challenge: challenge(ReviewDecision::Approve),
        deny_challenge: challenge(ReviewDecision::Deny),
        passkeys,
        rp_id: reviews::web_origin().1,
    })))
}

/// The assertion the page posts.
#[derive(Clone, Debug, Default, FromForm)]
pub struct AssertionForm {
    pub csrf: String,
    pub decision: String,
    pub at: i64,
    pub credential_id: String,
    /// base64url, as the browser gave them.
    pub authenticator_data: String,
    pub client_data_json: String,
    pub signature: String,
}

/// Answers the review behind `token` as the signed-in `user`: the token is spent only if the answer is recorded.
pub fn answer(
    c: &mut DbConnInner,
    token: &str,
    user: &str,
    form: &AssertionForm,
    now: i64,
) -> Result<(Answered, String), OrgError> {
    use diesel::Connection;
    let decision = match form.decision.as_str() {
        "approve" => ReviewDecision::Approve,
        "deny" => ReviewDecision::Deny,
        _ => return Err(OrgError::Invalid("Approve or deny.")),
    };
    let sig = data_encoding::BASE64URL_NOPAD
        .decode(form.signature.trim_end_matches('=').as_bytes())
        .map_err(|_| OrgError::Invalid("The passkey didn't sign this review."))?;
    let ans = WebAnswer {
        decision,
        at: form.at,
        signature: data_encoding::BASE64.encode(&sig),
        proof: WebAuthnProof {
            authenticator_data: form.authenticator_data.clone(),
            client_data_json: form.client_data_json.clone(),
        },
    };
    c.transaction(|c| {
        let hash = token_hash(token);
        let t = store::q_token(c, &hash)?.ok_or(OrgError::NotFound)?;
        if !crate::crypto::ct_eq(&t.user_uuid, user) {
            return Err(OrgError::Forbidden);
        }
        if !store::q_use_token(c, &hash, now)? {
            return Err(OrgError::Invalid("This link was used or expired."));
        }
        let answered = reviews::answer_web(c, &t.org_uuid, user, &form.credential_id, &t.request_uuid, &ans, now)?;
        audit::record(
            c,
            &t.org_uuid,
            audit::kinds::WEB_APPROVAL,
            Some(user),
            Some(&t.request_uuid),
            &json!({"channel": "web", "decision": decision.as_str(), "passkey": form.credential_id}),
            now,
        )?;
        Ok((answered, t.request_uuid))
    })
}

// ---------------------------------------------------------------------------------------------------------------
// Passkeys
// ---------------------------------------------------------------------------------------------------------------

const P256_SPKI_PREFIX: [u8; 26] = [
    0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08, 0x2a, 0x86, 0x48, 0xce,
    0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00,
];

/// Reads a WebAuthn registration: `(credential id, P-256 SPKI base64)`. Refuses another challenge, origin or relying
/// party, a missing user verification, a key other than ES256, and a synced (backup eligible) passkey.
pub fn parse_registration(
    credential_id: &str,
    client_data_b64: &str,
    attestation_b64: &str,
    challenge: &str,
    origin: &str,
    rp_id: &str,
) -> Result<(String, String), &'static str> {
    use serde_cbor_2::Value as C;
    let b64 = data_encoding::BASE64URL_NOPAD;
    let bad = "The passkey's answer can't be read.";
    let client = b64.decode(client_data_b64.trim_end_matches('=').as_bytes()).map_err(|_| bad)?;
    let data: Value = serde_json::from_slice(&client).map_err(|_| bad)?;
    if data["type"] != "webauthn.create" || data["challenge"].as_str() != Some(challenge) {
        return Err("That passkey answer is for another request. Try again.");
    }
    if data["origin"].as_str() != Some(origin) {
        return Err("That passkey answer is for another site.");
    }
    let att = b64.decode(attestation_b64.trim_end_matches('=').as_bytes()).map_err(|_| bad)?;
    let C::Map(obj) = serde_cbor_2::from_slice::<C>(&att).map_err(|_| bad)? else {
        return Err(bad);
    };
    let Some(C::Bytes(auth)) = obj.get(&C::Text("authData".into())) else {
        return Err(bad);
    };
    if auth.len() < 55 || auth[..32] != reins_proto::canon::sha256(rp_id.as_bytes()) {
        return Err("That passkey is for another site.");
    }
    let flags = auth[32];
    if flags & 0x01 == 0 || flags & 0x04 == 0 {
        return Err("Unlock your passkey to add it.");
    }
    if flags & 0x08 != 0 {
        return Err("Use a security key or a device-bound passkey.");
    }
    if flags & 0x40 == 0 {
        return Err(bad);
    }
    let len = usize::from(u16::from_be_bytes([auth[53], auth[54]]));
    let id = auth.get(55..55 + len).ok_or(bad)?;
    if b64.encode(id) != credential_id.trim_end_matches('=') {
        return Err(bad);
    }
    let mut cose = serde_cbor_2::Deserializer::from_slice(&auth[55 + len..]).into_iter::<C>();
    let Some(Ok(C::Map(key))) = cose.next() else {
        return Err(bad);
    };
    let int = |k: i128| key.get(&C::Integer(k));
    let ok = int(1) == Some(&C::Integer(2)) && int(3) == Some(&C::Integer(-7)) && int(-1) == Some(&C::Integer(1));
    let (Some(C::Bytes(x)), Some(C::Bytes(y))) = (int(-2), int(-3)) else {
        return Err("Use a passkey with a P-256 key.");
    };
    if !ok || x.len() != 32 || y.len() != 32 {
        return Err("Use a passkey with a P-256 key.");
    }
    let mut spki = P256_SPKI_PREFIX.to_vec();
    spki.push(0x04);
    spki.extend_from_slice(x);
    spki.extend_from_slice(y);
    Ok((b64.encode(id), data_encoding::BASE64.encode(&spki)))
}

/// Records a new passkey of `user`, unconfirmed until their enrolled phone signs for it.
pub fn register(
    c: &mut DbConnInner,
    org: &str,
    user: &str,
    credential_id: &str,
    spki: &str,
    now: i64,
) -> Result<(), OrgError> {
    if member_of(c, org, user)?.is_none() {
        return Err(OrgError::NotFound);
    }
    if store::q_passkeys_of(c, org, user)?.len() >= 10 {
        return Err(OrgError::Invalid("You have ten passkeys already. Remove one first."));
    }
    let fresh = store::q_insert_passkey(
        c,
        &ReinsOrgWebPasskey {
            credential_id: credential_id.to_owned(),
            org_uuid: org.to_owned(),
            user_uuid: user.to_owned(),
            passkey: spki.to_owned(),
            created_at: now,
            confirmed_at: None,
            confirmed_device: None,
            revoked_at: None,
        },
    )?;
    if !fresh {
        return Err(OrgError::Invalid("That passkey is already registered."));
    }
    audit::record(c, org, audit::kinds::PASSKEY_ADDED, Some(user), Some(credential_id), &json!({}), now)?;
    Ok(())
}

/// `POST /reins/api/org/passkeys/<id>/confirm`: the member's enrolled phone confirms (or refuses) a passkey.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PasskeyConfirm {
    pub approve: bool,
    pub at: i64,
    /// DER ECDSA P-256 over [`passkey_confirm_message`], base64.
    pub signature: String,
}

/// Confirms (or refuses) passkey `id` of `user`, signed on `device`, an enrolled phone that may decide.
pub fn confirm_passkey(
    c: &mut DbConnInner,
    org: &str,
    user: &str,
    device: &str,
    id: &str,
    ans: &PasskeyConfirm,
    now: i64,
) -> Result<(), OrgError> {
    if (now - ans.at).abs() > reviews::SKEW_SECS {
        return Err(OrgError::Invalid("This phone's clock is off. Check the time and try again."));
    }
    let dev = reviews::deciding_device(c, org, user, device, now)?;
    let key = store::q_passkey(c, org, id)?.filter(|k| k.user_uuid == user).ok_or(OrgError::NotFound)?;
    // The phone signs the exact key and registration it was shown (its fingerprint), not just an id.
    let msg = passkey_confirm_message(org, id, &passkey_spki_hash(&key.passkey), key.created_at, ans.at);
    if !reviews::device_sig_ok(&dev.public_key, &ans.signature, &msg) {
        return Err(OrgError::Invalid("The answer isn't signed by this device."));
    }
    let done = if ans.approve {
        store::q_confirm_passkey(c, org, user, id, device, now)?
    } else {
        store::q_revoke_passkey(c, org, user, id, now)?
    };
    if !done {
        return Err(OrgError::Invalid("That passkey was already answered."));
    }
    let kind = if ans.approve {
        audit::kinds::PASSKEY_CONFIRMED
    } else {
        audit::kinds::PASSKEY_REVOKED
    };
    audit::record(c, org, kind, Some(user), Some(id), &json!({"device": device}), now)?;
    Ok(())
}

/// Revokes passkey `id` of `user`.
pub fn revoke_passkey(c: &mut DbConnInner, org: &str, user: &str, id: &str, now: i64) -> Result<(), OrgError> {
    if !store::q_revoke_passkey(c, org, user, id, now)? {
        return Err(OrgError::NotFound);
    }
    audit::record(c, org, audit::kinds::PASSKEY_REVOKED, Some(user), Some(id), &json!({}), now)?;
    Ok(())
}

/// One passkey as its member's phone lists it: what the confirmation signs, and the fingerprint the console shows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PasskeyView {
    pub credential_id: String,
    pub created_at: i64,
    pub confirmed: bool,
    /// SHA-256 (hex) of the key's SPKI.
    pub spki_hash: String,
    /// `abcd-ef01`, as the console shows it.
    pub fingerprint: String,
}

impl PasskeyView {
    #[must_use]
    pub fn of(k: &ReinsOrgWebPasskey) -> Self {
        Self {
            credential_id: k.credential_id.clone(),
            created_at: k.created_at,
            confirmed: k.confirmed_at.is_some(),
            spki_hash: passkey_spki_hash(&k.passkey),
            fingerprint: passkey_fingerprint(&k.passkey),
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Routes
// ---------------------------------------------------------------------------------------------------------------

fn login_to(path: &str) -> Redirect {
    let mut url = format!("{}login?next=", console::base());
    url.extend(url::form_urlencoded::byte_serialize(path.as_bytes()));
    Redirect::to(url)
}

fn token_ok(token: &str) -> bool {
    (40..=64).contains(&token.len()) && token.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}

#[get("/reins/org/approve/<token>")]
async fn approve_page(token: &str, cookies: &CookieJar<'_>, conn: DbConn) -> Result<Custom<Page>, Redirect> {
    if !enabled() || !token_ok(token) {
        return Ok(Custom(Status::NotFound, not_found()));
    }
    let Some(s) = signed(cookies, now_unix()) else {
        return Err(login_to(&format!("{}/reins/org/approve/{token}", CONFIG.domain_path())));
    };
    let (t, u, now) = (token.to_owned(), s.user.clone(), now_unix());
    let opened = conn.run(move |c| open(c, &t, &u, now)).await;
    let base = console::base();
    Ok(match opened {
        Ok(Opened::Card(card)) => Custom(Status::Ok, Page(pages::card(&base, &s.csrf, token, &card))),
        Ok(Opened::NotYours) => {
            Custom(Status::Forbidden, Page(pages::message(&base, "This review is for someone else")))
        }
        Ok(Opened::UseTheApp) => Custom(Status::Ok, Page(pages::message(&base, "Open the Reins app"))),
        Ok(Opened::Gone) | Err(OrgError::NotFound) => {
            Custom(Status::NotFound, Page(pages::message(&base, "This link expired")))
        }
        Err(e) => Custom(Status::Ok, Page(pages::message(&base, e.message()))),
    })
}

#[post("/reins/org/approve/<token>", data = "<form>")]
async fn approve_post(
    token: &str,
    form: Form<AssertionForm>,
    cookies: &CookieJar<'_>,
    conn: DbConn,
    pool: &rocket::State<DbPool>,
) -> Result<Custom<Page>, Redirect> {
    let form = form.into_inner();
    let base = console::base();
    let Some(s) = signed(cookies, now_unix()).filter(|_| enabled() && token_ok(token)) else {
        return Err(console::to_home());
    };
    if !check_csrf(&s, &form.csrf) {
        return Ok(Custom(Status::Forbidden, Page(pages::message(&base, "Open the link again"))));
    }
    let (t, u, now) = (token.to_owned(), s.user.clone(), now_unix());
    let answered = conn.run(move |c| answer(c, &t, &u, &form, now)).await;
    if let Ok((Answered::Approved | Answered::Denied, request)) = &answered {
        super::channels::spawn_update(pool.inner(), request, None);
    }
    let answered = answered.map(|(a, _)| a);
    Ok(match answered {
        Ok(Answered::Approved | Answered::Recorded | Answered::Applied(_)) => {
            Custom(Status::Ok, Page(pages::done(&base, true)))
        }
        Ok(Answered::Denied) => Custom(Status::Ok, Page(pages::done(&base, false))),
        Err(OrgError::Forbidden) => {
            Custom(Status::Forbidden, Page(pages::message(&base, "This review is for someone else")))
        }
        Err(e) => Custom(Status::Ok, Page(pages::message(&base, e.message()))),
    })
}

#[get("/reins/org/passkeys/new")]
async fn passkey_page(cookies: &CookieJar<'_>, conn: DbConn) -> Result<Page, Redirect> {
    if !enabled() {
        return Ok(not_found());
    }
    let Some(s) = signed(cookies, now_unix()) else {
        return Err(login_to(&format!("{}/reins/org/passkeys/new", CONFIG.domain_path())));
    };
    let u = s.user.clone();
    let keys = conn
        .run(move |c| -> Result<Option<Vec<ReinsOrgWebPasskey>>, OrgError> {
            match super::leases::managed_org(c, &u)? {
                Some(org) => Ok(Some(store::q_passkeys_of(c, &org, &u)?)),
                None => Ok(None),
            }
        })
        .await;
    let Ok(Some(keys)) = keys else {
        return Ok(not_found());
    };
    let challenge = super::random_token();
    console::put_challenge(&s.sid, &challenge);
    Ok(Page(pages::passkeys(&console::base(), &s.csrf, &s.email, &s.user, &challenge, &reviews::web_origin().1, &keys)))
}

#[derive(FromForm)]
struct RegisterForm {
    csrf: String,
    credential_id: String,
    client_data_json: String,
    attestation_object: String,
}

#[post("/reins/org/passkeys/new", data = "<form>")]
async fn passkey_post(
    form: Form<RegisterForm>,
    cookies: &CookieJar<'_>,
    conn: DbConn,
    pool: &rocket::State<DbPool>,
) -> Result<Page, Redirect> {
    let form = form.into_inner();
    let base = console::base();
    let Some(s) = signed(cookies, now_unix()).filter(|_| enabled()) else {
        return Err(console::to_home());
    };
    if !check_csrf(&s, &form.csrf) {
        return Ok(Page(pages::message(&base, "Try again")));
    }
    let Some(challenge) = console::take_challenge(&s.sid) else {
        return Ok(Page(pages::message(&base, "Try again")));
    };
    let (origin, rp_id) = reviews::web_origin();
    let parsed = parse_registration(
        &form.credential_id,
        &form.client_data_json,
        &form.attestation_object,
        &challenge,
        &origin,
        &rp_id,
    );
    let (id, spki) = match parsed {
        Ok(x) => x,
        Err(m) => return Ok(Page(pages::message(&base, m))),
    };
    let (u, now, cred) = (s.user.clone(), now_unix(), id.clone());
    let done = conn
        .run(move |c| {
            let org = super::leases::managed_org(c, &u)?.ok_or(OrgError::NotFound)?;
            register(c, &org, &u, &cred, &spki, now)
        })
        .await;
    match done {
        Ok(()) => {
            notify_phone(pool.inner(), &s.user, &id);
            Ok(Page(pages::message(&base, "Confirm it on your phone")))
        }
        Err(e) => Ok(Page(pages::message(&base, e.message()))),
    }
}

/// Asks the member's enrolled phone to confirm a passkey (they must act, so this notifies).
fn notify_phone(pool: &DbPool, user: &str, credential: &str) {
    crate::api::reins::push::spawn_push(
        pool.clone(),
        UserId::from(user.to_owned()),
        None,
        reins_proto::pairing::PushMessage {
            t: reins_proto::pairing::PushKind::OrgPasskey,
            id: credential.to_owned(),
            p: Some(reins_proto::pairing::push_profile_tag(user)),
        },
    );
}

#[derive(FromForm)]
struct RevokeForm {
    csrf: String,
}

#[post("/reins/org/passkeys/<id>/revoke", data = "<form>")]
async fn passkey_revoke(id: &str, form: Form<RevokeForm>, cookies: &CookieJar<'_>, conn: DbConn) -> Redirect {
    if let Some(s) = signed(cookies, now_unix()).filter(|_| enabled())
        && check_csrf(&s, &form.csrf)
    {
        let (u, id, now) = (s.user.clone(), id.to_owned(), now_unix());
        let done = conn
            .run(move |c| {
                let org = super::leases::managed_org(c, &u)?.ok_or(OrgError::NotFound)?;
                revoke_passkey(c, &org, &u, &id, now)
            })
            .await;
        console::flash(
            &s.sid,
            if done.is_ok() {
                "Removed."
            } else {
                "Not found."
            },
            done.is_err(),
        );
    }
    Redirect::to(format!("{}passkeys/new", console::base()))
}

async fn org_of(user: &str, conn: &DbConn) -> Result<String, Status> {
    let u = user.to_owned();
    conn.run(move |c| super::leases::managed_org(c, &u)).await.ok().flatten().ok_or(Status::NotFound)
}

/// `GET /reins/api/org/passkeys`: this member's passkeys, for the phone that confirms them.
#[get("/reins/api/org/passkeys")]
async fn api_passkeys(headers: Headers, conn: DbConn) -> Result<Json<Vec<PasskeyView>>, Status> {
    if !enabled() {
        return Err(Status::NotFound);
    }
    let user = headers.user.uuid.to_string();
    let org = org_of(&user, &conn).await?;
    conn.run(move |c| store::q_passkeys_of(c, &org, &user))
        .await
        .map(|keys| Json(keys.iter().map(PasskeyView::of).collect()))
        .map_err(|_| Status::InternalServerError)
}

#[post("/reins/api/org/passkeys/<id>/confirm", data = "<body>")]
async fn api_confirm(
    id: &str,
    body: Json<PasskeyConfirm>,
    headers: Headers,
    conn: DbConn,
) -> Result<Status, Custom<Json<Value>>> {
    let err = |s: Status, m: &str| Custom(s, Json(json!({"error": m})));
    if !enabled() {
        return Err(err(Status::NotFound, "Not found."));
    }
    let (user, device) = (headers.user.uuid.to_string(), headers.device.uuid.to_string());
    let org = org_of(&user, &conn).await.map_err(|s| err(s, "Not found."))?;
    let (id, ans, now) = (id.to_owned(), body.into_inner(), now_unix());
    conn.run(move |c| confirm_passkey(c, &org, &user, &device, &id, &ans, now))
        .await
        .map(|()| Status::NoContent)
        .map_err(|e| err(Status::Conflict, e.message()))
}

#[delete("/reins/api/org/passkeys/<id>")]
async fn api_delete(id: &str, headers: Headers, conn: DbConn) -> Status {
    if !enabled() {
        return Status::NotFound;
    }
    let user = headers.user.uuid.to_string();
    let Ok(org) = org_of(&user, &conn).await else {
        return Status::NotFound;
    };
    let (id, now) = (id.to_owned(), now_unix());
    match conn.run(move |c| revoke_passkey(c, &org, &user, &id, now)).await {
        Ok(()) => Status::NoContent,
        Err(_) => Status::NotFound,
    }
}

#[cfg(all(test, sqlite))]
pub(crate) mod tests {
    use ring::{
        rand::SystemRandom,
        signature::{ECDSA_P256_SHA256_ASN1_SIGNING, EcdsaKeyPair, KeyPair},
    };
    use serde_cbor_2::Value as C;

    use super::*;
    use crate::api::reins::org::{
        routing::{
            Dynamic,
            tests::{Phones, ask_with, routed_org},
        },
        testing::db,
    };

    /// A soft WebAuthn authenticator with one P-256 credential.
    pub struct Authenticator {
        pub id: Vec<u8>,
        pkcs8: Vec<u8>,
    }

    impl Authenticator {
        pub fn new(id: &[u8]) -> Self {
            let rng = SystemRandom::new();
            Self {
                id: id.to_vec(),
                pkcs8: EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng).unwrap().as_ref().to_vec(),
            }
        }

        fn key(&self) -> EcdsaKeyPair {
            EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &self.pkcs8, &SystemRandom::new()).unwrap()
        }

        pub fn credential_id(&self) -> String {
            data_encoding::BASE64URL_NOPAD.encode(&self.id)
        }

        /// A `create()` answer: (client data, attestation object), base64url.
        pub fn register(&self, challenge: &str, origin: &str, rp_id: &str, flags: u8) -> (String, String) {
            let b64 = data_encoding::BASE64URL_NOPAD;
            let point = self.key().public_key().as_ref().to_vec();
            let cose: std::collections::BTreeMap<C, C> = [
                (C::Integer(1), C::Integer(2)),
                (C::Integer(3), C::Integer(-7)),
                (C::Integer(-1), C::Integer(1)),
                (C::Integer(-2), C::Bytes(point[1..33].to_vec())),
                (C::Integer(-3), C::Bytes(point[33..].to_vec())),
            ]
            .into();
            let mut auth = reins_proto::canon::sha256(rp_id.as_bytes()).to_vec();
            auth.push(flags | 0x40);
            auth.extend_from_slice(&[0, 0, 0, 0]);
            auth.extend_from_slice(&[0; 16]);
            auth.extend_from_slice(&u16::try_from(self.id.len()).unwrap().to_be_bytes());
            auth.extend_from_slice(&self.id);
            auth.extend(serde_cbor_2::to_vec(&C::Map(cose)).unwrap());
            let obj: std::collections::BTreeMap<C, C> = [
                (C::Text("fmt".into()), C::Text("none".into())),
                (C::Text("attStmt".into()), C::Map(std::collections::BTreeMap::new())),
                (C::Text("authData".into()), C::Bytes(auth)),
            ]
            .into();
            let client = json!({"type": "webauthn.create", "challenge": challenge, "origin": origin}).to_string();
            (b64.encode(client.as_bytes()), b64.encode(&serde_cbor_2::to_vec(&C::Map(obj)).unwrap()))
        }

        /// A `get()` answer for `challenge`: the form the page posts.
        pub fn assert(&self, challenge: &str, origin: &str, rp_id: &str, flags: u8) -> AssertionForm {
            let b64 = data_encoding::BASE64URL_NOPAD;
            let mut auth = reins_proto::canon::sha256(rp_id.as_bytes()).to_vec();
            auth.push(flags);
            auth.extend_from_slice(&[0, 0, 0, 1]);
            let client = json!({"type": "webauthn.get", "challenge": challenge, "origin": origin}).to_string();
            let mut signed = auth.clone();
            signed.extend_from_slice(&reins_proto::canon::sha256(client.as_bytes()));
            let sig = self.key().sign(&SystemRandom::new(), &signed).unwrap();
            AssertionForm {
                csrf: String::new(),
                decision: "approve".into(),
                at: 0,
                credential_id: self.credential_id(),
                authenticator_data: b64.encode(&auth),
                client_data_json: b64.encode(client.as_bytes()),
                signature: b64.encode(sig.as_ref()),
            }
        }
    }

    /// Registers `auth` for `user` and confirms it on their phone.
    pub fn add_passkey(c: &mut DbConnInner, phones: &Phones, user: &str, auth: &Authenticator, now: i64) {
        let (origin, rp) = reviews::web_origin();
        let (client, att) = auth.register("ch", &origin, &rp, 0x05);
        let (id, spki) = parse_registration(&auth.credential_id(), &client, &att, "ch", &origin, &rp).unwrap();
        register(c, "o1", user, &id, &spki, now).unwrap();
        let ans = PasskeyConfirm {
            approve: true,
            at: now,
            signature: phones[user].sign(&passkey_confirm_message("o1", &id, &passkey_spki_hash(&spki), now, now)),
        };
        confirm_passkey(c, "o1", user, &phones[user].device, &id, &ans, now).unwrap();
    }

    /// Kim's assertion of `decision` on request `id` at `at`.
    pub fn assertion(
        c: &mut DbConnInner,
        auth: &Authenticator,
        id: &str,
        decision: ReviewDecision,
        at: i64,
        flags: u8,
    ) -> AssertionForm {
        let row = reins_org_request::q_request(c, "o1", id).unwrap().unwrap();
        let msg = review_message("o1", ReviewSubject::Request, id, &row.digest, decision, &row.snapshot, at);
        let (origin, rp) = reviews::web_origin();
        let mut f = auth.assert(&webauthn_challenge(&msg), &origin, &rp, flags);
        f.decision = decision.as_str().into();
        f.at = at;
        f
    }

    #[test]
    fn a_token_works_once_for_its_member_only() {
        let mut c = db();
        let phones = routed_org(&mut c);
        let kim = Authenticator::new(b"kim-key");
        add_passkey(&mut c, &phones, "kim", &kim, 150);
        let s = ask_with(&mut c, "acme/prod", &Dynamic::new(), 200);
        // Only a member it's routed to gets a link; never the requester.
        assert!(issue_token(&mut c, "o1", &s.id, "lee", 201).unwrap().is_none());
        assert!(issue_token(&mut c, "o1", &s.id, "mia", 201).unwrap().is_none());
        let token = issue_token(&mut c, "o1", &s.id, "kim", 201).unwrap().unwrap();
        // Forwarded to someone else: refused, and it changes nothing.
        assert!(matches!(open(&mut c, &token, "lee", 202).unwrap(), Opened::NotYours));
        let form = assertion(&mut c, &kim, &s.id, ReviewDecision::Approve, 202, 0x05);
        assert_eq!(answer(&mut c, &token, "lee", &form, 202).unwrap_err(), OrgError::Forbidden);
        assert!(matches!(open(&mut c, &token, "kim", 202).unwrap(), Opened::Card(_)));
        assert!(matches!(answer(&mut c, &token, "kim", &form, 202).unwrap().0, Answered::Approved));
        // Once.
        assert!(matches!(open(&mut c, &token, "kim", 203).unwrap(), Opened::Gone));
        assert!(answer(&mut c, &token, "kim", &form, 203).is_err());
        let row = reins_org_request::q_request(&mut c, "o1", &s.id).unwrap().unwrap();
        assert_eq!(row.state, request_state::APPROVED);
        let record: reins_proto::managed::DecisionRecord =
            serde_json::from_str::<reins_proto::managed::Envelope>(row.decision.as_deref().unwrap())
                .unwrap()
                .parse()
                .unwrap();
        assert!(record.approvals[0].webauthn.is_some());
        assert_eq!(record.approvals[0].device, kim.credential_id());
    }

    #[test]
    fn a_synced_passkey_is_refused() {
        let (origin, rp) = reviews::web_origin();
        let synced = Authenticator::new(b"synced");
        let (client, att) = synced.register("ch", &origin, &rp, 0x05 | 0x08);
        assert_eq!(
            parse_registration(&synced.credential_id(), &client, &att, "ch", &origin, &rp),
            Err("Use a security key or a device-bound passkey.")
        );
        // Nor without user verification, for another challenge, site or relying party.
        let (client, att) = synced.register("ch", &origin, &rp, 0x01);
        assert!(parse_registration(&synced.credential_id(), &client, &att, "ch", &origin, &rp).is_err());
        let (client, att) = synced.register("other", &origin, &rp, 0x05);
        assert!(parse_registration(&synced.credential_id(), &client, &att, "ch", &origin, &rp).is_err());
        let (client, att) = synced.register("ch", "https://evil.example", &rp, 0x05);
        assert!(parse_registration(&synced.credential_id(), &client, &att, "ch", &origin, &rp).is_err());
        let (client, att) = synced.register("ch", &origin, "evil.example", 0x05);
        assert!(parse_registration(&synced.credential_id(), &client, &att, "ch", &origin, &rp).is_err());
        // An assertion that says backup eligible is refused too (a passkey that became synced).
        let mut c = db();
        let phones = routed_org(&mut c);
        let kim = Authenticator::new(b"kim-key");
        add_passkey(&mut c, &phones, "kim", &kim, 150);
        let s = ask_with(&mut c, "acme/prod", &Dynamic::new(), 200);
        let token = issue_token(&mut c, "o1", &s.id, "kim", 201).unwrap().unwrap();
        let form = assertion(&mut c, &kim, &s.id, ReviewDecision::Approve, 202, 0x0d);
        assert_eq!(
            answer(&mut c, &token, "kim", &form, 202).unwrap_err(),
            OrgError::Invalid("Use a security key or a device-bound passkey.")
        );
        // A refused answer leaves the link usable.
        let form = assertion(&mut c, &kim, &s.id, ReviewDecision::Approve, 203, 0x05);
        assert!(answer(&mut c, &token, "kim", &form, 203).is_ok());
    }

    #[test]
    fn l3_is_refused_on_the_web() {
        let mut c = db();
        let phones = routed_org(&mut c);
        let kim = Authenticator::new(b"kim-key");
        add_passkey(&mut c, &phones, "kim", &kim, 150);
        let s = ask_with(&mut c, "acme/prod", &Dynamic::new(), 200);
        // The same review made L3 (as a deletion would be): no link, and an old link opens "use the app".
        let token = issue_token(&mut c, "o1", &s.id, "kim", 201).unwrap().unwrap();
        let row = reins_org_request::q_request(&mut c, "o1", &s.id).unwrap().unwrap();
        let mut trace: Value = serde_json::from_str(&row.trace).unwrap();
        trace["level"] = json!("L3");
        {
            use crate::db::schema::reins_org_requests as r;
            use diesel::prelude::*;
            diesel::update(r::table.filter(r::uuid.eq(&s.id)))
                .set(r::trace.eq(trace.to_string()))
                .execute(&mut c)
                .unwrap();
        }
        assert!(issue_token(&mut c, "o1", &s.id, "kim", 202).unwrap().is_none());
        assert!(matches!(open(&mut c, &token, "kim", 202).unwrap(), Opened::UseTheApp));
        let form = assertion(&mut c, &kim, &s.id, ReviewDecision::Approve, 202, 0x05);
        assert_eq!(
            answer(&mut c, &token, "kim", &form, 202).unwrap_err(),
            OrgError::Invalid("Open the Reins app to decide this.")
        );
    }

    #[test]
    fn registration_needs_the_enrolled_phone() {
        let mut c = db();
        let phones = routed_org(&mut c);
        let (origin, rp) = reviews::web_origin();
        let kim = Authenticator::new(b"kim-key");
        let (client, att) = kim.register("ch", &origin, &rp, 0x05);
        let (id, spki) = parse_registration(&kim.credential_id(), &client, &att, "ch", &origin, &rp).unwrap();
        register(&mut c, "o1", "kim", &id, &spki, 150).unwrap();
        // Unconfirmed: not in the roster, not usable.
        let roster = crate::api::reins::org::roster::build(&mut c, "o1", None, 151).unwrap();
        assert!(roster.member("kim").unwrap().passkeys.is_empty());
        let s = ask_with(&mut c, "acme/prod", &Dynamic::new(), 200);
        let token = issue_token(&mut c, "o1", &s.id, "kim", 201).unwrap().unwrap();
        let form = assertion(&mut c, &kim, &s.id, ReviewDecision::Approve, 202, 0x05);
        assert_eq!(
            answer(&mut c, &token, "kim", &form, 202).unwrap_err(),
            OrgError::Invalid("Confirm this passkey on your phone first.")
        );
        // Someone else's phone, or Kim's signing something else, can't confirm it.
        let hash = passkey_spki_hash(&spki);
        let wrong = PasskeyConfirm {
            approve: true,
            at: 203,
            signature: phones["lee"].sign(&passkey_confirm_message("o1", &id, &hash, 150, 203)),
        };
        assert!(confirm_passkey(&mut c, "o1", "kim", &phones["kim"].device, &id, &wrong, 203).is_err());
        assert!(confirm_passkey(&mut c, "o1", "lee", &phones["lee"].device, &id, &wrong, 203).is_err(), "not Lee's");
        let other = PasskeyConfirm {
            approve: true,
            at: 203,
            signature: phones["kim"].sign(&passkey_confirm_message("o1", "other", &hash, 150, 203)),
        };
        assert!(confirm_passkey(&mut c, "o1", "kim", &phones["kim"].device, &id, &other, 203).is_err());
        let ok = PasskeyConfirm {
            approve: true,
            at: 203,
            signature: phones["kim"].sign(&passkey_confirm_message("o1", &id, &hash, 150, 203)),
        };
        confirm_passkey(&mut c, "o1", "kim", &phones["kim"].device, &id, &ok, 203).unwrap();
        let roster = crate::api::reins::org::roster::build(&mut c, "o1", None, 204).unwrap();
        assert_eq!(roster.member("kim").unwrap().passkeys[0].credential_id, id);
        let form = assertion(&mut c, &kim, &s.id, ReviewDecision::Approve, 204, 0x05);
        assert!(answer(&mut c, &token, "kim", &form, 204).is_ok());
    }

    #[test]
    fn the_assertion_binds_digest_decision_and_snapshot() {
        let mut c = db();
        let phones = routed_org(&mut c);
        let kim = Authenticator::new(b"kim-key");
        add_passkey(&mut c, &phones, "kim", &kim, 150);
        let s = ask_with(&mut c, "acme/prod", &Dynamic::new(), 200);
        let token = issue_token(&mut c, "o1", &s.id, "kim", 201).unwrap().unwrap();
        let row = reins_org_request::q_request(&mut c, "o1", &s.id).unwrap().unwrap();
        let (origin, rp) = reviews::web_origin();
        let forged = |digest: &str, d: ReviewDecision, snapshot: &str, at: i64| {
            let msg = review_message("o1", ReviewSubject::Request, &s.id, digest, d, snapshot, at);
            let mut f = kim.assert(&webauthn_challenge(&msg), &origin, &rp, 0x05);
            f.decision = "approve".into();
            f.at = 202;
            f
        };
        for f in [
            forged("other", ReviewDecision::Approve, &row.snapshot, 202),
            forged(&row.digest, ReviewDecision::Deny, &row.snapshot, 202),
            forged(&row.digest, ReviewDecision::Approve, "other", 202),
            forged(&row.digest, ReviewDecision::Approve, &row.snapshot, 203),
        ] {
            assert!(answer(&mut c, &token, "kim", &f, 202).is_err());
        }
        let good = forged(&row.digest, ReviewDecision::Approve, &row.snapshot, 202);
        assert!(matches!(answer(&mut c, &token, "kim", &good, 202).unwrap().0, Answered::Approved));
    }

    #[test]
    fn a_confirmation_binds_the_exact_key_shown() {
        let mut c = db();
        let phones = routed_org(&mut c);
        let (origin, rp) = reviews::web_origin();
        // Two keys that share the first characters of their id: the phone signs the key's hash and registration
        // time, so a confirmation for one never confirms the other.
        let mine = Authenticator::new(b"shared-prefix-0001-mine");
        let theirs = Authenticator::new(b"shared-prefix-0001-theirs");
        let mut reg = |a: &Authenticator, at: i64| {
            let (client, att) = a.register("ch", &origin, &rp, 0x05);
            let (id, spki) = parse_registration(&a.credential_id(), &client, &att, "ch", &origin, &rp).unwrap();
            register(&mut c, "o1", "kim", &id, &spki, at).unwrap();
            (id, spki)
        };
        let (mine_id, mine_spki) = reg(&mine, 150);
        let (theirs_id, theirs_spki) = reg(&theirs, 151);
        let views: Vec<PasskeyView> =
            store::q_passkeys_of(&mut c, "o1", "kim").unwrap().iter().map(PasskeyView::of).collect();
        assert_eq!(views[0].fingerprint, passkey_fingerprint(&mine_spki));
        assert_ne!(views[0].fingerprint, views[1].fingerprint);
        // Signed for my key, presented for theirs: refused. A stale registration time: refused.
        let for_mine = |at: i64, registered: i64| PasskeyConfirm {
            approve: true,
            at,
            signature: phones["kim"].sign(&passkey_confirm_message(
                "o1",
                &theirs_id,
                &passkey_spki_hash(&mine_spki),
                registered,
                at,
            )),
        };
        assert!(
            confirm_passkey(&mut c, "o1", "kim", &phones["kim"].device, &theirs_id, &for_mine(200, 151), 200).is_err()
        );
        let stale = PasskeyConfirm {
            approve: true,
            at: 200,
            signature: phones["kim"].sign(&passkey_confirm_message(
                "o1",
                &theirs_id,
                &passkey_spki_hash(&theirs_spki),
                99,
                200,
            )),
        };
        assert!(confirm_passkey(&mut c, "o1", "kim", &phones["kim"].device, &theirs_id, &stale, 200).is_err());
        let exact = PasskeyConfirm {
            approve: true,
            at: 200,
            signature: phones["kim"].sign(&passkey_confirm_message(
                "o1",
                &mine_id,
                &passkey_spki_hash(&mine_spki),
                150,
                200,
            )),
        };
        confirm_passkey(&mut c, "o1", "kim", &phones["kim"].device, &mine_id, &exact, 200).unwrap();
        let usable: Vec<String> =
            store::q_usable_passkeys(&mut c, "o1", "kim").unwrap().into_iter().map(|k| k.credential_id).collect();
        assert_eq!(usable, [mine_id]);
    }

    #[test]
    fn device_binding_is_as_the_browser_reports() {
        // With attestation `none`, BE/UV are the authenticator's own word: a registration that claims device-bound is
        // accepted on that word (it can't be proven), and the page says so.
        let (origin, rp) = reviews::web_origin();
        let a = Authenticator::new(b"claims-bound");
        let (client, att) = a.register("ch", &origin, &rp, 0x05);
        assert!(parse_registration(&a.credential_id(), &client, &att, "ch", &origin, &rp).is_ok());
        let html = pages::passkeys("/reins/org/", "c", "kim@acme.com", "kim", "ch", &rp, &[]);
        assert!(html.contains("as your browser reports"), "{html}");
        assert!(html.contains("attestation:'none'"));
    }
}
