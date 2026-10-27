//! The minimal admin console (O1) at `{DOMAIN}/reins/org/`: org setup, domains, sign-in (SSO, SCIM), members and
//! groups, roles, the changes waiting for a second admin, and the admin events.
//!
//! - **Sign-in** reuses the apps' SSO path: the browser goes to `/identity/connect/authorize` with client id
//!   `reins-console` (so the browser-binding cookie and PKCE apply), comes back to `/reins/org/callback`, and the code
//!   is exchanged here. The org gate and the passkey rule apply as for the apps; only an existing Reins account gets
//!   in (sign in on the phone first). The console has no two-factor step, so an account with two-factor sign-in
//!   enabled is refused here rather than let in on SSO alone. A GitHub sign-in's token stays in the server-side
//!   session, for GitHub's word that the admin owns the GitHub organization they connect.
//! - **Sessions** live in memory: a random id in an HttpOnly cookie (`SameSite=Lax`, so the sign-in's redirect brings
//!   it back), 30 minutes idle, 8 hours at most, a new id once signed in. Every form carries the session's CSRF token.
//! - **Authority** is checked again on every page and every action, from the database: nothing in the session says
//!   what the admin may do.

use std::{
    collections::HashMap,
    io::Cursor,
    sync::{LazyLock, Mutex},
};

use reins_proto::org::{AdminAction, Capability, IdentityKind, Role, Subject, Tier};
use rocket::{
    Request, Response, Route, State,
    form::{Form, FromForm},
    http::{ContentType, Cookie, CookieJar, SameSite},
    response::{self, Redirect, Responder},
};

use super::{
    OrgError, admin, can, console_pages as pages, deprovision, dns, enabled, member_of, random_token, tenant,
    workos_api,
};
use crate::{
    CONFIG,
    api::reins::now_unix,
    auth::Secure,
    db::{
        DbConn, DbConnInner, DbPool,
        models::{
            SsoUser, reins_org,
            reins_org_event::{self, q_events},
        },
        schema::users,
    },
};

const COOKIE: &str = "reins_console";
const IDLE_SECS: i64 = 30 * 60;
const MAX_SECS: i64 = 8 * 3600;
/// Console sessions kept at once (the oldest go first).
const MAX_SESSIONS: usize = 10_000;

pub fn routes() -> Vec<Route> {
    routes![
        home,
        login,
        callback,
        logout,
        create,
        overview,
        add_domain,
        verify_domain,
        remove_domain,
        set_identity,
        set_tier,
        set_two_person,
        confirm_held,
        members,
        roles,
        remove_member,
        approve_change,
        reject_change,
        events
    ]
}

#[derive(Clone)]
struct Session {
    user: Option<String>,
    email: String,
    csrf: String,
    /// While signing in: the state and PKCE verifier sent to `/identity/connect/authorize`.
    pending: Option<(String, String)>,
    /// The GitHub token of a GitHub sign-in (never sent to the browser).
    github_token: Option<String>,
    flash: Option<(String, bool)>,
    created: i64,
    last: i64,
}

static SESSIONS: LazyLock<Mutex<HashMap<String, Session>>> = LazyLock::new(Mutex::default);

fn sessions() -> std::sync::MutexGuard<'static, HashMap<String, Session>> {
    SESSIONS.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn base() -> String {
    format!("{}/reins/org/", CONFIG.domain_path())
}

fn put_cookie(cookies: &CookieJar<'_>, sid: String, https: bool) {
    cookies.add(
        Cookie::build((COOKIE, sid))
            .path(format!("{}/reins/org", CONFIG.domain_path()))
            .http_only(true)
            .same_site(SameSite::Lax)
            .secure(https)
            .build(),
    );
}

/// A new session; `None` when the store is full of signed-in sessions. Only sessions still signing in are ever pushed
/// out to make room (the oldest first), never a signed-in admin's.
fn new_session(now: i64) -> Option<(String, Session)> {
    let mut map = sessions();
    map.retain(|_, s| now - s.last < IDLE_SECS && now - s.created < MAX_SECS);
    if map.len() >= MAX_SESSIONS {
        let oldest_pending =
            map.iter().filter(|(_, s)| s.user.is_none()).min_by_key(|(_, s)| s.last).map(|(k, _)| k.clone());
        map.remove(&oldest_pending?);
    }
    let session = Session {
        user: None,
        email: String::new(),
        csrf: random_token(),
        pending: None,
        github_token: None,
        flash: None,
        created: now,
        last: now,
    };
    let sid = random_token();
    map.insert(sid.clone(), session.clone());
    Some((sid, session))
}

/// Takes the session `sid` if it is still signing in with exactly `state`, and gives its PKCE verifier; anything else
/// (no session, another state, a signed-in session) is left as it was.
fn take_pending(sid: &str, state: &str) -> Option<String> {
    let mut map = sessions();
    let matches = map.get(sid).and_then(|s| s.pending.as_ref()).is_some_and(|(expected, _)| s_eq(expected, state));
    if !matches {
        return None;
    }
    map.remove(sid).and_then(|s| s.pending).map(|(_, verifier)| verifier)
}

fn s_eq(a: &str, b: &str) -> bool {
    crate::crypto::ct_eq(a, b)
}

/// The signed-in session of this browser: `(sid, user, email, csrf)`.
struct Signed {
    sid: String,
    user: String,
    email: String,
    csrf: String,
    github_token: Option<String>,
    flash: Option<(String, bool)>,
}

fn signed(cookies: &CookieJar<'_>, now: i64) -> Option<Signed> {
    let sid = cookies.get(COOKIE)?.value().to_owned();
    let mut map = sessions();
    let s = map.get_mut(&sid)?;
    if now - s.last >= IDLE_SECS || now - s.created >= MAX_SECS {
        map.remove(&sid);
        return None;
    }
    s.last = now;
    let user = s.user.clone()?;
    Some(Signed {
        sid,
        user,
        email: s.email.clone(),
        csrf: s.csrf.clone(),
        github_token: s.github_token.clone(),
        flash: s.flash.take(),
    })
}

fn flash(sid: &str, text: &str, bad: bool) {
    if let Some(s) = sessions().get_mut(sid) {
        s.flash = Some((text.to_owned(), bad));
    }
}

/// An HTML page with the console's own, stricter headers.
pub struct Page(pub String);

impl<'r> Responder<'r, 'static> for Page {
    fn respond_to(self, _: &'r Request<'_>) -> response::Result<'static> {
        Response::build()
            .header(ContentType::HTML)
            .raw_header("Cache-Control", "no-store")
            .raw_header("Referrer-Policy", "no-referrer")
            .raw_header("X-Frame-Options", "DENY")
            .sized_body(self.0.len(), Cursor::new(self.0))
            .ok()
    }
}

type Answer = Result<Page, Redirect>;

fn to_home() -> Redirect {
    Redirect::to(base().trim_end_matches('/').to_owned())
}

fn to_org(org: &str, tail: &str) -> Redirect {
    Redirect::to(format!("{}{}{}", base(), org, tail))
}

fn check_csrf(s: &Signed, given: &str) -> bool {
    crate::crypto::ct_eq(&s.csrf, given)
}

fn not_found() -> Page {
    Page(pages::page("Not found", None, "<section><h1>Not found</h1></section>", &base()))
}

async fn run<T: Send + 'static>(
    conn: &DbConn,
    f: impl FnOnce(&mut DbConnInner) -> Result<T, OrgError> + Send + 'static,
) -> Result<T, OrgError> {
    conn.run(f).await
}

/// Flash text for an outcome.
fn outcome_text(r: &Result<admin::Proposed, OrgError>, done: &str) -> (String, bool) {
    match r {
        Ok(admin::Proposed::Applied) => (done.to_owned(), false),
        Ok(admin::Proposed::Pending(_)) => ("Waiting for a second admin to approve.".to_owned(), false),
        Err(e) => (e.message().to_owned(), true),
    }
}

async fn propose(
    conn: &DbConn,
    pool: &DbPool,
    org: &str,
    user: &str,
    action: AdminAction,
    binding: Option<workos_api::VerifiedBinding>,
) -> Result<admin::Proposed, OrgError> {
    let (o, u, now) = (org.to_owned(), user.to_owned(), now_unix());
    let (proposed, followups) = run(conn, move |c| admin::propose(c, &o, &u, &action, binding.as_ref(), now)).await?;
    for removal in followups {
        deprovision::finish(removal, Some(pool)).await;
    }
    Ok(proposed)
}

// ---------------------------------------------------------------------------------------------------------------
// Sign-in
// ---------------------------------------------------------------------------------------------------------------

#[get("/reins/org")]
async fn home(cookies: &CookieJar<'_>, conn: DbConn) -> Page {
    if !enabled() {
        return not_found();
    }
    let Some(s) = signed(cookies, now_unix()) else {
        return Page(pages::signed_out(&base(), None));
    };
    let (user, email) = (s.user.clone(), s.email.clone());
    let view = run(&conn, move |c| {
        let orgs = reins_org::q_orgs_of_user(c, &user)?
            .into_iter()
            .map(|o| {
                let name = reins_org::q_org_name(c, &o.org_uuid)?.unwrap_or_default();
                Ok((o.org_uuid.clone(), name, o.tier()))
            })
            .collect::<Result<Vec<_>, OrgError>>()?;
        let in_any = !reins_org::q_orgs_of_user_any(c, &user)?.is_empty();
        let domain = reins_proto::org::email_domain(&email);
        let free = match &domain {
            Some(d) => reins_org::q_verified_owner(c, d)?.is_none(),
            None => false,
        };
        Ok((orgs, (!in_any && free).then_some(domain).flatten()))
    })
    .await;
    let Ok((orgs, create)) = view else {
        return Page(pages::signed_out(&base(), Some("Something went wrong. Try again.")));
    };
    let flash = s.flash.as_ref().map(|(t, b)| (t.as_str(), *b));
    let base = base();
    let frame = pages::Frame {
        base: &base,
        email: &s.email,
        csrf: &s.csrf,
        org: None,
        tab: pages::Tab::Home,
        flash,
    };
    Page(pages::home(
        &frame,
        &pages::HomeView {
            orgs,
            can_create: create.as_deref(),
        },
    ))
}

#[get("/reins/org/login")]
fn login(cookies: &CookieJar<'_>, secure: Secure, ip: crate::auth::ClientIp) -> Redirect {
    if !enabled() || !CONFIG.sso_enabled() || crate::ratelimit::check_limit_login(&ip.ip).is_err() {
        return to_home();
    }
    let now = now_unix();
    let Some((sid, _)) = new_session(now) else {
        return to_home();
    };
    let state = random_token();
    let verifier = random_token();
    let challenge = data_encoding::BASE64URL_NOPAD
        .encode(ring::digest::digest(&ring::digest::SHA256, verifier.as_bytes()).as_ref());
    if let Some(s) = sessions().get_mut(&sid) {
        s.pending = Some((state.clone(), verifier));
    }
    put_cookie(cookies, sid, secure.https);
    let Ok(mut url) = url::Url::parse(&format!("{}/identity/connect/authorize", CONFIG.domain())) else {
        return to_home();
    };
    url.query_pairs_mut()
        .append_pair("client_id", CONSOLE_CLIENT)
        .append_pair("redirect_uri", &callback_url())
        .append_pair("response_type", "code")
        .append_pair("scope", "api offline_access")
        .append_pair("state", &state)
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256");
    Redirect::to(String::from(url))
}

/// The client id `sso::authorize_url` knows the console by.
pub const CONSOLE_CLIENT: &str = "reins-console";

/// Where the console's sign-in comes back.
pub fn callback_url() -> String {
    format!("{}/reins/org/callback", CONFIG.domain())
}

#[get("/reins/org/callback?<code>&<state>")]
async fn callback(code: String, state: String, cookies: &CookieJar<'_>, secure: Secure, conn: DbConn) -> Answer {
    if !enabled() {
        return Ok(not_found());
    }
    let refused = |m: &str| Ok(Page(pages::signed_out(&base(), Some(m))));
    let Some(sid) = cookies.get(COOKIE).map(|c| c.value().to_owned()) else {
        return refused("Your sign-in expired. Sign in again.");
    };
    // Only the sign-in this callback answers is taken: a forged callback (another state) leaves every session alone.
    let pending = take_pending(&sid, &state);
    let Some(verifier) = pending else {
        return refused("Your sign-in expired. Sign in again.");
    };
    let Ok(verifier) = serde_json::from_value(serde_json::Value::String(verifier)) else {
        return refused("Your sign-in expired. Sign in again.");
    };
    let code = crate::sso::OIDCCode::from(code);
    let (sso_auth, user_infos) = match crate::sso::exchange_code(&code, verifier, &conn).await {
        Ok(x) => x,
        Err(e) => {
            warn!("Console sign-in: {e:?}");
            return refused("The sign-in didn't work. Try again.");
        }
    };
    if let Err(e) = sso_auth.delete(&conn).await {
        warn!("Console sign-in: could not delete the SSO state: {e:?}");
    }
    let Some((user, _)) = SsoUser::find_by_identifier(&user_infos.identifier, &conn).await else {
        return refused("Sign in to Reins on your phone first, then come back.");
    };
    if !user.enabled {
        return refused("This account can't sign in.");
    }
    if two_factor_refused(&crate::db::models::TwoFactor::find_by_user(&user.uuid, &conn).await) {
        return refused(
            "This account uses two-step sign-in, which the console can't ask for yet. Use an admin account without it.",
        );
    }
    let decision = match super::gate::check(&user_infos, Some(&user), &conn).await {
        Ok(d) => d,
        Err(e) => return refused(e.message()),
    };
    if let Err(e) = crate::sso_workos::check_passkey(
        &user_infos,
        crate::sso_workos::passkeys_required(),
        decision.through_org_identity(),
    ) {
        return refused(e.message());
    }
    // A new session id once signed in.
    let now = now_unix();
    let Some((new_sid, _)) = new_session(now) else {
        return refused("The console is busy. Try again in a minute.");
    };
    let github_token = user_infos
        .workos
        .as_ref()
        .filter(|w| w.method.as_deref() == Some("GitHubOAuth"))
        .and_then(|w| w.provider_token.clone());
    if let Some(s) = sessions().get_mut(&new_sid) {
        s.user = Some(user.uuid.to_string());
        s.email.clone_from(&user.email);
        s.github_token = github_token;
    }
    put_cookie(cookies, new_sid, secure.https);
    Err(to_home())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_forged_callback_never_ends_a_session() {
        let now = 1_000;
        let (sid, _) = new_session(now).unwrap();
        sessions().get_mut(&sid).unwrap().pending = Some(("right-state".to_owned(), "verifier".to_owned()));
        assert_eq!(take_pending(&sid, "forged"), None);
        assert!(sessions().contains_key(&sid), "untouched");
        assert_eq!(take_pending(&sid, "right-state").as_deref(), Some("verifier"));
        assert!(!sessions().contains_key(&sid));
        // A signed-in session can't be taken as a pending one.
        let (admin, _) = new_session(now).unwrap();
        sessions().get_mut(&admin).unwrap().user = Some("u1".to_owned());
        assert_eq!(take_pending(&admin, ""), None);
        assert!(sessions().contains_key(&admin));
        sessions().remove(&admin);
    }

    #[test]
    fn accounts_with_two_step_sign_in_are_refused() {
        use crate::db::models::{TwoFactor, TwoFactorType};
        let user = crate::db::models::UserId::from("u1".to_owned());
        assert!(!two_factor_refused(&[]));
        let mut totp = TwoFactor::new(user, TwoFactorType::Authenticator, String::new());
        assert!(two_factor_refused(std::slice::from_ref(&totp)));
        totp.enabled = false;
        assert!(!two_factor_refused(&[totp]));
    }
}

#[derive(FromForm)]
struct CsrfOnly {
    csrf: String,
}

#[post("/reins/org/logout", data = "<form>")]
fn logout(form: Form<CsrfOnly>, cookies: &CookieJar<'_>) -> Redirect {
    let form = form.into_inner();
    if let Some(s) = signed(cookies, now_unix())
        && check_csrf(&s, &form.csrf)
    {
        sessions().remove(&s.sid);
        cookies.remove(Cookie::build(COOKIE).path(format!("{}/reins/org", CONFIG.domain_path())).build());
    }
    to_home()
}

// ---------------------------------------------------------------------------------------------------------------
// Orgs
// ---------------------------------------------------------------------------------------------------------------

#[derive(FromForm)]
struct NewOrg {
    csrf: String,
    name: String,
    tier: String,
}

#[post("/reins/org/new", data = "<form>")]
async fn create(form: Form<NewOrg>, cookies: &CookieJar<'_>, conn: DbConn) -> Redirect {
    let Some(s) = signed(cookies, now_unix()) else {
        return to_home();
    };
    if !enabled() || !check_csrf(&s, &form.csrf) {
        return to_home();
    }
    let tier = Tier::parse(&form.tier).unwrap_or(Tier::Business);
    let (user, email, name, now) = (s.user.clone(), s.email.clone(), form.name.clone(), now_unix());
    match run(&conn, move |c| tenant::create_org(c, &user, &email, &name, tier, now)).await {
        Ok((org, _)) => {
            flash(&s.sid, "Created. Add the TXT record, then Verify.", false);
            to_org(&org, "")
        }
        Err(e) => {
            flash(&s.sid, e.message(), true);
            to_home()
        }
    }
}

/// The signed-in admin's way into `org`: refused (as not found) unless an active member holding one of `caps`.
async fn admin_of(cookies: &CookieJar<'_>, conn: &DbConn, org: &str, caps: &'static [Capability]) -> Option<Signed> {
    if !enabled() {
        return None;
    }
    let s = signed(cookies, now_unix())?;
    let (o, u) = (org.to_owned(), s.user.clone());
    let ok = run(conn, move |c| {
        if member_of(c, &o, &u)?.is_none() {
            return Ok(false);
        }
        for cap in caps {
            if can(c, &o, &u, *cap)? {
                return Ok(true);
            }
        }
        Ok(false)
    })
    .await
    .unwrap_or(false);
    ok.then_some(s)
}

const ADMIN: &[Capability] =
    &[Capability::SetUpOrg, Capability::AssignRoles, Capability::ViewMembers, Capability::ViewAdminEvents];

fn email_of(c: &mut DbConnInner, user: &str) -> Result<String, OrgError> {
    use diesel::prelude::*;
    Ok(users::table
        .filter(users::uuid.eq(user))
        .select(users::email)
        .first::<String>(c)
        .optional()?
        .unwrap_or_default())
}

#[get("/reins/org/<org>")]
async fn overview(org: &str, cookies: &CookieJar<'_>, conn: DbConn) -> Page {
    let Some(s) = admin_of(cookies, &conn, org, ADMIN).await else {
        return not_found();
    };
    let (o, u) = (org.to_owned(), s.user.clone());
    let view = run(&conn, move |c| {
        let t = reins_org::q_org(c, &o)?.ok_or(OrgError::NotFound)?;
        let name = reins_org::q_org_name(c, &o)?.unwrap_or_default();
        let domains = reins_org::q_domains(c, &o)?
            .into_iter()
            .map(|d| pages::DomainView {
                verified: d.verified_at.is_some(),
                record_name: tenant::record_name(&d.domain),
                record_value: tenant::record_value(&d.token),
                domain: d.domain,
            })
            .collect();
        let mut names = HashMap::new();
        let mut roles = HashMap::new();
        for m in reins_org::q_members(c, &o)? {
            names.insert(m.uuid.to_string(), email_of(c, m.user_uuid.as_ref())?);
            roles.insert(m.uuid.to_string(), reins_org::q_granted_roles(c, &o, m.uuid.as_ref())?);
        }
        for (id, gname, _) in reins_org::q_groups(c, &o)? {
            roles.insert(id.clone(), reins_org::q_subject_roles(c, &o, reins_org::SUBJECT_GROUP, &id)?);
            names.insert(id, gname);
        }
        let changes = admin::pending_for(c, &o, &u)?
            .into_iter()
            .map(|p| {
                Ok(pages::ChangeView {
                    id: p.change.uuid.clone(),
                    proposed_by: email_of(c, &p.change.proposed_by)?,
                    can_approve: p.can_approve,
                    can_reject: p.can_reject,
                    action: p.action,
                })
            })
            .collect::<Result<Vec<_>, OrgError>>()?;
        Ok(pages::OverviewView {
            org_id: o.clone(),
            name,
            tier: t.tier(),
            identity: t.identity(),
            workos_org_id: t.workos_org_id.clone(),
            directory_id: t.directory_id.clone(),
            github_org: t.github_org.clone(),
            two_person: t.two_person,
            owner_recovery: super::invariants::owner_recovery_shown(c, &o)?,
            domains,
            changes,
            can_set_up: can(c, &o, &u, Capability::SetUpOrg)?,
            names,
            roles,
            held: super::scim::q_held(c, &o)?.map(|h| (h.external_ids.len(), h.digest)),
            can_confirm_held: can(c, &o, &u, Capability::ManageMembers)?,
        })
    })
    .await;
    let Ok(view) = view else {
        return not_found();
    };
    let flash = s.flash.as_ref().map(|(t, b)| (t.as_str(), *b));
    let base = base();
    let frame = pages::Frame {
        base: &base,
        email: &s.email,
        csrf: &s.csrf,
        org: Some((org, &view.name)),
        tab: pages::Tab::Overview,
        flash,
    };
    Page(pages::overview(&frame, &view))
}

#[derive(FromForm)]
struct DomainForm {
    csrf: String,
    domain: String,
}

#[post("/reins/org/<org>/domains", data = "<form>")]
async fn add_domain(org: &str, form: Form<DomainForm>, cookies: &CookieJar<'_>, conn: DbConn) -> Redirect {
    let Some(s) = admin_of(cookies, &conn, org, &[Capability::SetUpOrg]).await else {
        return to_home();
    };
    if check_csrf(&s, &form.csrf) {
        let (org_id, user, domain, now) = (org.to_owned(), s.user.clone(), form.domain.clone(), now_unix());
        match run(&conn, move |c| tenant::add_domain(c, &org_id, &domain, &user, now)).await {
            Ok(_) => flash(&s.sid, "Added. Publish the TXT record, then Verify.", false),
            Err(e) => flash(&s.sid, e.message(), true),
        }
    }
    to_org(org, "")
}

#[post("/reins/org/<org>/domains/verify", data = "<form>")]
async fn verify_domain(org: &str, form: Form<DomainForm>, cookies: &CookieJar<'_>, conn: DbConn) -> Redirect {
    let Some(s) = admin_of(cookies, &conn, org, &[Capability::SetUpOrg]).await else {
        return to_home();
    };
    if check_csrf(&s, &form.csrf) {
        let resolver = dns::resolver();
        match tenant::verify_domain(&conn, org, &form.domain, &s.user, resolver.as_ref(), now_unix()).await {
            Ok(true) => flash(&s.sid, "Verified.", false),
            Ok(false) => flash(&s.sid, "The TXT record isn't there yet. DNS can take a few minutes.", true),
            Err(e) => flash(&s.sid, e.message(), true),
        }
    }
    to_org(org, "")
}

#[post("/reins/org/<org>/domains/remove", data = "<form>")]
async fn remove_domain(
    org: &str,
    form: Form<DomainForm>,
    cookies: &CookieJar<'_>,
    conn: DbConn,
    pool: &State<DbPool>,
) -> Redirect {
    let Some(s) = admin_of(cookies, &conn, org, &[Capability::SetUpOrg]).await else {
        return to_home();
    };
    if check_csrf(&s, &form.csrf) {
        let action = AdminAction::RemoveDomain {
            domain: form.domain.trim().to_lowercase(),
        };
        let r = propose(&conn, pool, org, &s.user, action, None).await;
        let (t, bad) = outcome_text(&r, "Removed.");
        flash(&s.sid, &t, bad);
    }
    to_org(org, "")
}

#[derive(FromForm)]
struct IdentityForm {
    csrf: String,
    identity: String,
    workos_org_id: String,
    directory_id: String,
    github_org: String,
}

fn opt(s: &str) -> Option<String> {
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_owned())
}

/// Whether an account with these second factors is refused by the console (it has no two-factor step yet).
fn two_factor_refused(factors: &[crate::db::models::TwoFactor]) -> bool {
    factors.iter().any(|f| f.enabled)
}

/// WorkOS's (and for a GitHub org, GitHub's) word that the binding of a `SetIdentity` is this tenant's
/// ([`workos_api`]); `None` for other actions. `github_token`: the console user's GitHub sign-in.
async fn binding_for(
    conn: &DbConn,
    org: &str,
    action: &AdminAction,
    github_token: Option<&str>,
) -> Result<Option<workos_api::VerifiedBinding>, OrgError> {
    let AdminAction::SetIdentity {
        identity,
        workos_org_id: Some(workos_org_id),
        directory_id,
        github_org,
    } = action
    else {
        return Ok(None);
    };
    if *identity == IdentityKind::None {
        return Ok(None);
    }
    let o = org.to_owned();
    let (domains, scim) = run(conn, move |c| {
        let t = reins_org::q_org(c, &o)?.ok_or(OrgError::NotFound)?;
        let d: Vec<String> =
            reins_org::q_domains(c, &o)?.into_iter().filter(|d| d.verified_at.is_some()).map(|d| d.domain).collect();
        Ok::<_, OrgError>((d, t.tier().scim()))
    })
    .await?;
    let directory = directory_id.as_deref().filter(|_| *identity == IdentityKind::Sso && scim);
    let github = match (identity, github_org.as_deref()) {
        (IdentityKind::GithubOrg, Some(gh)) => {
            let answer = match github_token {
                Some(token) => super::github::checker().membership(token, gh).await,
                None => super::github::GithubAnswer::Unknown,
            };
            Some((gh, answer))
        }
        (IdentityKind::GithubOrg, None) => return Err(OrgError::Invalid("Name the GitHub organization (its login).")),
        _ => None,
    };
    workos_api::verify_binding(workos_api::api().as_ref(), workos_org_id, directory, &domains, github).await.map(Some)
}

#[post("/reins/org/<org>/identity", data = "<form>")]
async fn set_identity(
    org: &str,
    form: Form<IdentityForm>,
    cookies: &CookieJar<'_>,
    conn: DbConn,
    pool: &State<DbPool>,
) -> Redirect {
    let Some(s) = admin_of(cookies, &conn, org, &[Capability::SetUpOrg]).await else {
        return to_home();
    };
    if !check_csrf(&s, &form.csrf) {
        return to_org(org, "");
    }
    let Some(identity) = IdentityKind::parse(&form.identity) else {
        return to_org(org, "");
    };
    let action = AdminAction::SetIdentity {
        identity,
        workos_org_id: opt(&form.workos_org_id),
        directory_id: opt(&form.directory_id),
        github_org: opt(&form.github_org),
    };
    let r = match binding_for(&conn, org, &action, s.github_token.as_deref()).await {
        Ok(binding) => propose(&conn, pool, org, &s.user, action, binding).await,
        Err(e) => Err(e),
    };
    if matches!(r, Ok(admin::Proposed::Applied)) {
        // A newly bound directory: its users and groups come from listing it.
        super::scim::backfill_soon();
    }
    let (t, bad) = outcome_text(&r, "Saved.");
    flash(&s.sid, &t, bad);
    to_org(org, "")
}

#[derive(FromForm)]
struct TierForm {
    csrf: String,
    tier: String,
}

#[post("/reins/org/<org>/tier", data = "<form>")]
async fn set_tier(
    org: &str,
    form: Form<TierForm>,
    cookies: &CookieJar<'_>,
    conn: DbConn,
    pool: &State<DbPool>,
) -> Redirect {
    let Some(s) = admin_of(cookies, &conn, org, &[Capability::SetUpOrg]).await else {
        return to_home();
    };
    if check_csrf(&s, &form.csrf)
        && let Some(tier) = Tier::parse(&form.tier)
    {
        let r = propose(
            &conn,
            pool,
            org,
            &s.user,
            AdminAction::SetTier {
                tier,
            },
            None,
        )
        .await;
        let (t, bad) = outcome_text(&r, "Plan changed.");
        flash(&s.sid, &t, bad);
    }
    to_org(org, "")
}

#[derive(FromForm)]
struct HeldForm {
    csrf: String,
    held: String,
}

/// Confirms the removals the directory sync held back (a two-person change like any removal).
#[post("/reins/org/<org>/directory/confirm", data = "<form>")]
async fn confirm_held(
    org: &str,
    form: Form<HeldForm>,
    cookies: &CookieJar<'_>,
    conn: DbConn,
    pool: &State<DbPool>,
) -> Redirect {
    let Some(s) = admin_of(cookies, &conn, org, &[Capability::ManageMembers]).await else {
        return to_home();
    };
    if check_csrf(&s, &form.csrf) {
        let action = AdminAction::ConfirmDirectoryRemovals {
            held: form.held.clone(),
        };
        let r = propose(&conn, pool, org, &s.user, action, None).await;
        if matches!(r, Ok(admin::Proposed::Applied)) {
            super::scim::backfill_soon();
        }
        let (t, bad) = outcome_text(&r, "Confirmed. They're removed at the next directory sync.");
        flash(&s.sid, &t, bad);
    }
    to_org(org, "")
}

#[derive(FromForm)]
struct TwoPersonForm {
    csrf: String,
    on: bool,
}

#[post("/reins/org/<org>/two-person", data = "<form>")]
async fn set_two_person(
    org: &str,
    form: Form<TwoPersonForm>,
    cookies: &CookieJar<'_>,
    conn: DbConn,
    pool: &State<DbPool>,
) -> Redirect {
    let Some(s) = admin_of(cookies, &conn, org, &[Capability::SetUpOrg]).await else {
        return to_home();
    };
    if check_csrf(&s, &form.csrf) {
        let r = propose(
            &conn,
            pool,
            org,
            &s.user,
            AdminAction::SetTwoPerson {
                on: form.on,
            },
            None,
        )
        .await;
        let (t, bad) = outcome_text(&r, "Saved.");
        flash(&s.sid, &t, bad);
    }
    to_org(org, "")
}

// ---------------------------------------------------------------------------------------------------------------
// Members, roles, changes, events
// ---------------------------------------------------------------------------------------------------------------

fn device_line(d: &reins_org::ReinsOrgDevice) -> String {
    if d.revoked_at.is_some() {
        return "Removed device".to_owned();
    }
    let p: Option<reins_proto::org::Posture> = serde_json::from_str(&d.posture).ok();
    let key = if d.hardware {
        "Hardware key"
    } else {
        "Software key"
    };
    match p {
        Some(p) => format!(
            "{key} · {} {} · {} · app {}{}",
            p.os_name,
            p.os_version,
            if p.screen_lock {
                "screen lock"
            } else {
                "no screen lock"
            },
            p.app_version,
            if p.rooted == Some(true) {
                " · rooted"
            } else {
                ""
            }
        ),
        None => key.to_owned(),
    }
}

#[get("/reins/org/<org>/members")]
async fn members(org: &str, cookies: &CookieJar<'_>, conn: DbConn) -> Page {
    let Some(s) = admin_of(cookies, &conn, org, &[Capability::ViewMembers]).await else {
        return not_found();
    };
    let (o, u) = (org.to_owned(), s.user.clone());
    let view = run(&conn, move |c| {
        let devices = reins_org::q_devices_of_org(c, &o)?;
        let groups_all = reins_org::q_groups(c, &o)?;
        let mut members = Vec::new();
        for m in reins_org::q_members(c, &o)? {
            let user = m.user_uuid.to_string();
            let st = reins_org::q_member_state(c, &o, &user)?;
            let active = st.as_ref().is_some_and(reins_org::ReinsOrgMember::is_active);
            let groups = reins_org::q_groups_of(c, m.uuid.as_ref())?;
            members.push(pages::MemberRow {
                membership: m.uuid.to_string(),
                email: email_of(c, &user)?,
                active,
                from_directory: m.external_id.is_some(),
                roles: reins_org::q_granted_roles(c, &o, m.uuid.as_ref())?,
                groups: groups_all.iter().filter(|(id, _, _)| groups.contains(id)).map(|(_, n, _)| n.clone()).collect(),
                devices: devices.iter().filter(|d| d.user_uuid == user).map(device_line).collect(),
            });
        }
        members.sort_by(|a, b| b.active.cmp(&a.active).then(a.email.cmp(&b.email)));
        let mut groups = Vec::new();
        for (id, name, external) in groups_all {
            groups.push(pages::GroupRow {
                members: reins_org::q_group_members(c, &id)?.len(),
                roles: reins_org::q_subject_roles(c, &o, reins_org::SUBJECT_GROUP, &id)?,
                from_directory: external.is_some(),
                id,
                name,
            });
        }
        Ok(pages::MembersView {
            org_id: o.clone(),
            members,
            groups,
            can_assign: can(c, &o, &u, Capability::AssignRoles)?,
            can_remove: can(c, &o, &u, Capability::ManageMembers)?,
        })
    })
    .await;
    let (Ok(view), Ok(name)) = (view, org_name(&conn, org).await) else {
        return not_found();
    };
    let flash = s.flash.as_ref().map(|(t, b)| (t.as_str(), *b));
    let base = base();
    let frame = pages::Frame {
        base: &base,
        email: &s.email,
        csrf: &s.csrf,
        org: Some((org, &name)),
        tab: pages::Tab::Members,
        flash,
    };
    Page(pages::members(&frame, &view))
}

async fn org_name(conn: &DbConn, org: &str) -> Result<String, OrgError> {
    let o = org.to_owned();
    run(conn, move |c| Ok(reins_org::q_org_name(c, &o)?.unwrap_or_default())).await
}

#[derive(FromForm)]
struct RoleForm {
    csrf: String,
    /// `member:<membership>` or `group:<group>`.
    subject: String,
    role: String,
    op: String,
}

#[post("/reins/org/<org>/roles", data = "<form>")]
async fn roles(
    org: &str,
    form: Form<RoleForm>,
    cookies: &CookieJar<'_>,
    conn: DbConn,
    pool: &State<DbPool>,
) -> Redirect {
    let Some(s) = admin_of(cookies, &conn, org, &[Capability::AssignRoles]).await else {
        return to_home();
    };
    let subject = match form.subject.split_once(':') {
        Some(("member", id)) => Some(Subject::Member(id.to_owned())),
        Some(("group", id)) => Some(Subject::Group(id.to_owned())),
        _ => None,
    };
    if check_csrf(&s, &form.csrf)
        && let (Some(subject), Some(role)) = (subject, Role::parse(&form.role))
    {
        let action = if form.op == "revoke" {
            AdminAction::RevokeRole {
                subject,
                role,
            }
        } else {
            AdminAction::GrantRole {
                subject,
                role,
            }
        };
        let r = propose(&conn, pool, org, &s.user, action, None).await;
        let (t, bad) = outcome_text(&r, "Done.");
        flash(&s.sid, &t, bad);
    }
    to_org(org, "/members")
}

#[derive(FromForm)]
struct MemberForm {
    csrf: String,
    member: String,
}

#[post("/reins/org/<org>/members/remove", data = "<form>")]
async fn remove_member(
    org: &str,
    form: Form<MemberForm>,
    cookies: &CookieJar<'_>,
    conn: DbConn,
    pool: &State<DbPool>,
) -> Redirect {
    let Some(s) = admin_of(cookies, &conn, org, &[Capability::ManageMembers]).await else {
        return to_home();
    };
    if check_csrf(&s, &form.csrf) {
        let action = AdminAction::RemoveMember {
            member: form.member.clone(),
        };
        let r = propose(&conn, pool, org, &s.user, action, None).await;
        let (t, bad) = outcome_text(&r, "Removed.");
        flash(&s.sid, &t, bad);
    }
    to_org(org, "/members")
}

#[derive(FromForm)]
struct ChangeForm {
    csrf: String,
    change: String,
}

#[post("/reins/org/<org>/changes/approve", data = "<form>")]
async fn approve_change(
    org: &str,
    form: Form<ChangeForm>,
    cookies: &CookieJar<'_>,
    conn: DbConn,
    pool: &State<DbPool>,
) -> Redirect {
    let Some(s) = admin_of(cookies, &conn, org, ADMIN).await else {
        return to_home();
    };
    if !check_csrf(&s, &form.csrf) {
        return to_org(org, "");
    }
    let (o, ch) = (org.to_owned(), form.change.clone());
    let action = run(&conn, move |c| {
        let row = reins_org::q_change(c, &o, &ch)?.ok_or(OrgError::NotFound)?;
        serde_json::from_str::<AdminAction>(&row.action).map_err(|e| OrgError::Internal(e.to_string()))
    })
    .await;
    let result = match action {
        Ok(action) => match binding_for(&conn, org, &action, s.github_token.as_deref()).await {
            Ok(binding) => {
                let (o, ch, u, now) = (org.to_owned(), form.change.clone(), s.user.clone(), now_unix());
                match run(&conn, move |c| admin::approve_from_console(c, &o, &ch, &u, binding.as_ref(), now)).await {
                    Ok(followups) => {
                        for removal in followups {
                            deprovision::finish(removal, Some(pool)).await;
                        }
                        super::scim::backfill_soon();
                        Ok(())
                    }
                    Err(e) => Err(e),
                }
            }
            Err(e) => Err(e),
        },
        Err(e) => Err(e),
    };
    match result {
        Ok(()) => flash(&s.sid, "Approved and applied.", false),
        Err(OrgError::Invalid("Approve it on your phone first.")) => {
            // The approval is the phone's: wake it.
            super::reviews::notify(pool.inner(), std::slice::from_ref(&s.user), &form.change);
            flash(&s.sid, "Approve it on your phone first. We sent it there.", true);
        }
        Err(e) => flash(&s.sid, e.message(), true),
    }
    to_org(org, "")
}

#[post("/reins/org/<org>/changes/reject", data = "<form>")]
async fn reject_change(org: &str, form: Form<ChangeForm>, cookies: &CookieJar<'_>, conn: DbConn) -> Redirect {
    let Some(s) = admin_of(cookies, &conn, org, ADMIN).await else {
        return to_home();
    };
    if check_csrf(&s, &form.csrf) {
        let (o, ch, u, now) = (org.to_owned(), form.change.clone(), s.user.clone(), now_unix());
        match run(&conn, move |c| admin::reject(c, &o, &ch, &u, now)).await {
            Ok(()) => flash(&s.sid, "Rejected.", false),
            Err(e) => flash(&s.sid, e.message(), true),
        }
    }
    to_org(org, "")
}

/// How many events the console reads to check the chain.
const CHAIN_CHECK_MAX: i64 = 5_000;

#[get("/reins/org/<org>/events?<before>")]
async fn events(org: &str, before: Option<i64>, cookies: &CookieJar<'_>, conn: DbConn) -> Page {
    let Some(s) = admin_of(cookies, &conn, org, &[Capability::ViewAdminEvents]).await else {
        return not_found();
    };
    let o = org.to_owned();
    let loaded = run(&conn, move |c| {
        let page = q_events(c, &o, before, 100)?;
        let mut rows = Vec::new();
        for ev in &page {
            let actor = match &ev.actor {
                Some(a) if reins_org::q_membership(c, &o, a)?.is_some() => Some(email_of(c, a)?),
                Some(_) => Some("—".to_owned()),
                None => None,
            };
            rows.push(pages::EventRow {
                seq: ev.seq,
                at: chrono::DateTime::from_timestamp(ev.at, 0)
                    .map(|t| t.format("%Y-%m-%d %H:%M UTC").to_string())
                    .unwrap_or_default(),
                kind: ev.kind.clone(),
                actor,
                subject: ev.subject.clone(),
                detail: ev.detail.clone(),
            });
        }
        let older = page.last().filter(|e| e.seq > 1).map(|e| e.seq);
        let head = page.first().map_or(0, |e| e.seq);
        let chain = if before.is_none() && head <= CHAIN_CHECK_MAX {
            let mut all = q_events(c, &o, None, CHAIN_CHECK_MAX)?;
            all.reverse();
            Some(reins_org_event::verify_chain(&all))
        } else {
            None
        };
        Ok((rows, older, chain))
    })
    .await;
    let (Ok((rows, older, chain)), Ok(name)) = (loaded, org_name(&conn, org).await) else {
        return not_found();
    };
    let flash = s.flash.as_ref().map(|(t, b)| (t.as_str(), *b));
    let base = base();
    let frame = pages::Frame {
        base: &base,
        email: &s.email,
        csrf: &s.csrf,
        org: Some((org, &name)),
        tab: pages::Tab::Events,
        flash,
    };
    Page(pages::events(&frame, &rows, older, chain))
}
