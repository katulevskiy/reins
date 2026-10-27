//! The console of a Family (P7): its guardians and members (minor, leave), invitations, the notice, consents and the
//! changes waiting for a guardian. Shown in place of a company's overview when the tenant's tier is Family; roles read
//! "Guardian" and "Family member" (`family_role_label`). Each POST re-checks the guardian's capability and the CSRF
//! token.

#![allow(
    clippy::format_collect,
    reason = "small server-rendered pages: building them as readable format strings beats saving allocations"
)]

use reins_proto::{
    managed::OAuthProvider,
    org::{Capability, FamilyLeaveView, family_role_label},
};
use rocket::{
    Route,
    form::{Form, FromForm},
    http::CookieJar,
    response::Redirect,
};

use super::{
    OrgError, can,
    console::{Page, Signed, admin_of, base, check_csrf, email_of, flash, run, signed, to_home, to_org},
    console_pages::{self as pages, Frame, Tab, hidden_csrf},
    family, family_connect,
};
use crate::{
    CONFIG,
    api::reins::{now_unix, pages::escape_html as e},
    db::{
        DbConn, DbPool,
        models::{reins_family, reins_family_conn, reins_org},
    },
};

pub fn routes() -> Vec<Route> {
    routes![invite, cancel_invite, consent, notice, minor, connect, connect_wait, oauth_callback, revoke]
}

/// One member as the family page lists them.
pub struct MemberRow {
    pub user: String,
    pub email: String,
    pub label: &'static str,
    pub guardian: bool,
    pub minor: bool,
    pub minor_proposed: bool,
    pub leave: Option<FamilyLeaveView>,
    /// `(provider, state, account)` of each account a guardian connected for them.
    pub connections: Vec<(OAuthProvider, String, Option<String>)>,
}

#[allow(clippy::struct_excessive_bools, reason = "independent facts the page shows")]
pub struct InviteRow {
    pub id: String,
    pub email: String,
    pub minor: bool,
    pub guardian: bool,
    pub approved: bool,
    pub open: bool,
}

pub struct FamilyView {
    pub members: Vec<MemberRow>,
    pub invites: Vec<InviteRow>,
    pub notice_days: i32,
    pub can_manage: bool,
    pub can_connect: bool,
}

/// The link a new member opens on their phone: `reins-family://<server host>/<secret>`.
#[must_use]
pub fn invite_link(token: &str) -> String {
    let domain = CONFIG.domain();
    let host = domain.trim_end_matches('/').rsplit("://").next().unwrap_or_default();
    format!("reins-family://{host}/{token}")
}

fn load(c: &mut crate::db::DbConnInner, org: &str, viewer: &str, now: i64) -> Result<FamilyView, OrgError> {
    let mut members = Vec::new();
    for m in reins_org::q_members(c, org)? {
        let user = m.user_uuid.to_string();
        if super::member_of(c, org, &user)?.is_none() {
            continue;
        }
        let roles = reins_org::q_roles_of(c, org, m.uuid.as_ref())?;
        let guardian = roles.iter().any(|r| reins_proto::org::GUARDIAN_ROLES.contains(r));
        let row = reins_family::q_member(c, org, &user)?;
        let leave = reins_family::q_leave(c, org, &user)?.map(|l| FamilyLeaveView {
            requested_at: l.requested_at,
            effective_at: l.effective_at,
            needs_consent: l.effective_at.is_none(),
        });
        let connections = reins_family_conn::q_connections(c, org, Some(&user))?
            .into_iter()
            .filter_map(|r| OAuthProvider::parse(&r.provider).map(|p| (p, r.state, r.account)))
            .collect();
        members.push(MemberRow {
            connections,
            email: email_of(c, &user)?,
            label: family_role_label(if guardian {
                reins_proto::org::Role::OrgOwner
            } else {
                reins_proto::org::Role::Member
            }),
            guardian,
            minor: row.as_ref().is_some_and(|r| r.minor),
            minor_proposed: row.as_ref().is_some_and(|r| r.minor_proposed_at.is_some()),
            leave,
            user,
        });
    }
    members.sort_by(|a, b| (!a.guardian, &a.email).cmp(&(!b.guardian, &b.email)));
    let invites = reins_family::q_invites(c, org)?
        .into_iter()
        .take(30)
        .map(|i| InviteRow {
            open: i.open(now),
            id: i.id,
            email: i.email,
            minor: i.minor,
            guardian: i.guardian,
            approved: i.approved,
        })
        .collect();
    Ok(FamilyView {
        members,
        invites,
        notice_days: reins_family::q_family(c, org)?.map_or(family::DEFAULT_NOTICE_DAYS, |f| f.notice_days),
        can_manage: can(c, org, viewer, Capability::ManageMembers)?,
        can_connect: can(c, org, viewer, Capability::ManageConnections)?,
    })
}

fn when(t: i64) -> String {
    chrono::DateTime::from_timestamp(t, 0).map(|d| d.format("%Y-%m-%d %H:%M UTC").to_string()).unwrap_or_default()
}

/// The family's page (what `GET /reins/org/<org>` shows for a family).
pub(super) async fn page(org: &str, s: &Signed, conn: &DbConn, overview: pages::OverviewView) -> Page {
    let (o, u, now) = (org.to_owned(), s.user.clone(), now_unix());
    let Ok(view) = run(conn, move |c| load(c, &o, &u, now)).await else {
        return super::console::not_found();
    };
    let base = base();
    let frame = Frame {
        base: &base,
        email: &s.email,
        csrf: &s.csrf,
        org: Some((org, &overview.name)),
        tab: Tab::Overview,
        flash: s.flash.as_ref().map(|(t, b)| (t.as_str(), *b)),
    };
    Page(render(&frame, &overview, &view))
}

#[must_use]
pub fn render(frame: &Frame<'_>, overview: &pages::OverviewView, v: &FamilyView) -> String {
    let csrf = hidden_csrf(frame.csrf);
    let at = format!("{}{}/family", e(frame.base), e(&overview.org_id));
    let members: String = v
        .members
        .iter()
        .map(|m| {
            let mut pills = String::new();
            if m.minor {
                pills.push_str("<span class=\"pill\">Minor</span>");
            } else if m.minor_proposed {
                pills.push_str("<span class=\"pill\">Minor · waiting for them</span>");
            }
            let leave = match &m.leave {
                Some(l) if l.needs_consent => {
                    let button = if v.can_manage {
                        format!(
                            "<form method=\"post\" action=\"{at}/consent\">{csrf}<input type=\"hidden\" name=\"member\" \
value=\"{}\"><button type=\"submit\">Allow</button></form>",
                            e(&m.user)
                        )
                    } else {
                        String::new()
                    };
                    format!("<span class=\"pill bad\">Asked to leave</span>{button}")
                }
                Some(l) => format!(
                    "<span class=\"pill\">Leaves {}</span>",
                    e(&l.effective_at.map(when).unwrap_or_default())
                ),
                None => String::new(),
            };
            let mark = if v.can_manage && !m.guardian && !m.minor && !m.minor_proposed {
                format!(
                    "<form method=\"post\" action=\"{at}/minor\">{csrf}<input type=\"hidden\" name=\"member\" \
value=\"{}\"><input type=\"hidden\" name=\"on\" value=\"true\"><button class=\"quiet\" type=\"submit\">Mark minor\
</button></form>",
                    e(&m.user)
                )
            } else {
                String::new()
            };
            let accounts: String = OAuthProvider::ALL
                .iter()
                .filter(|_| v.can_connect && !m.guardian)
                .map(|p| {
                    let now = m.connections.iter().find(|(q, ..)| q == p);
                    match now {
                        Some((_, state, account)) if state == "connected" => format!(
                            "<form method=\"post\" action=\"{at}/revoke\">{csrf}<input type=\"hidden\" name=\"member\" \
value=\"{u}\"><input type=\"hidden\" name=\"provider\" value=\"{pv}\"><span class=\"pill v\">{label} · {acct}</span>\
<button class=\"quiet\" type=\"submit\">Disconnect</button></form>",
                            u = e(&m.user),
                            pv = p.as_str(),
                            label = p.label(),
                            acct = e(account.as_deref().unwrap_or_default()),
                        ),
                        _ => format!(
                            "<form method=\"post\" action=\"{at}/connect\">{csrf}<input type=\"hidden\" name=\"member\" \
value=\"{u}\"><input type=\"hidden\" name=\"provider\" value=\"{pv}\"><button class=\"quiet\" type=\"submit\">\
Connect {label}</button></form>",
                            u = e(&m.user),
                            pv = p.as_str(),
                            label = p.label(),
                        ),
                    }
                })
                .collect();
            format!(
                "<tr><td>{} {pills}</td><td>{}</td><td>{leave}{mark}{accounts}</td></tr>",
                e(&m.email),
                m.label
            )
        })
        .collect();
    let invites: String = v
        .invites
        .iter()
        .filter(|i| i.open)
        .map(|i| {
            let what = match (i.guardian, i.minor) {
                (true, _) => "Guardian",
                (false, true) => "Minor",
                (false, false) => "Family member",
            };
            let wait = if i.approved {
                ""
            } else {
                " <span class=\"meta\">Waiting for a second guardian</span>"
            };
            let cancel = if v.can_manage {
                format!(
                    "<form method=\"post\" action=\"{at}/invites/cancel\">{csrf}<input type=\"hidden\" name=\"id\" \
value=\"{}\"><button class=\"quiet\" type=\"submit\">Cancel</button></form>",
                    e(&i.id)
                )
            } else {
                String::new()
            };
            format!("<tr><td>{}</td><td>{what}{wait}</td><td>{cancel}</td></tr>", e(&i.email))
        })
        .collect();
    let invite_form = if v.can_manage {
        format!(
            "<form method=\"post\" action=\"{at}/invites\">{csrf}<input name=\"email\" type=\"email\" \
placeholder=\"New address\" required><label><input type=\"checkbox\" name=\"minor\" value=\"true\"> Minor</label>\
<label><input type=\"checkbox\" name=\"guardian\" value=\"true\"> Guardian</label><button type=\"submit\">Invite\
</button></form>"
        )
    } else {
        String::new()
    };
    let notice = if v.can_manage {
        let opts: String = family::NOTICE_DAYS
            .iter()
            .map(|d| {
                let sel = if *d == v.notice_days {
                    " selected"
                } else {
                    ""
                };
                format!("<option value=\"{d}\"{sel}>{d} days</option>")
            })
            .collect();
        format!(
            "<form method=\"post\" action=\"{at}/notice\">{csrf}<select name=\"days\">{opts}</select>\
<button class=\"quiet\" type=\"submit\">Save</button></form>"
        )
    } else {
        format!("<p class=\"meta\">{} days</p>", v.notice_days)
    };
    let invites = if invites.is_empty() {
        String::new()
    } else {
        format!("<table>{invites}</table>")
    };
    let body = format!(
        "<div class=\"row\"><h1>{name}</h1><span class=\"pill v\">Family</span></div>\
<section><h2>Waiting for approval</h2>{changes}</section>\
<section><h2>Family</h2><table>{members}</table></section>\
<section><h2>Invite</h2>{invite_form}{invites}</section>\
<section><h2>Notice to leave</h2>{notice}</section>",
        name = e(&overview.name),
        changes = pages::changes_table(frame, overview),
    );
    pages::page(&overview.name, Some(frame), &body, frame.base)
}

const MANAGE: &[Capability] = &[Capability::ManageMembers];

#[derive(FromForm)]
struct InviteForm {
    csrf: String,
    email: String,
    #[field(default = false)]
    minor: bool,
    #[field(default = false)]
    guardian: bool,
}

#[post("/reins/org/<org>/family/invites", data = "<form>")]
async fn invite(org: &str, form: Form<InviteForm>, cookies: &CookieJar<'_>, conn: DbConn) -> Redirect {
    let Some(s) = admin_of(cookies, &conn, org, MANAGE).await else {
        return to_home();
    };
    if check_csrf(&s, &form.csrf) {
        let (o, u, now) = (org.to_owned(), s.user.clone(), now_unix());
        let (email, minor, guardian) = (form.email.clone(), form.minor, form.guardian);
        match run(&conn, move |c| family::invite(c, &o, &u, &email, minor, guardian, now)).await {
            Ok(created) => {
                let wait = if created.waiting {
                    " It counts once another guardian approves."
                } else {
                    ""
                };
                // Shown once: only its hash is kept.
                flash(&s.sid, &format!("Send this link (shown once): {}{wait}", invite_link(&created.token)), false);
            }
            Err(e) => flash(&s.sid, e.message(), true),
        }
    }
    to_org(org, "")
}

#[derive(FromForm)]
struct IdForm {
    csrf: String,
    id: String,
}

#[post("/reins/org/<org>/family/invites/cancel", data = "<form>")]
async fn cancel_invite(org: &str, form: Form<IdForm>, cookies: &CookieJar<'_>, conn: DbConn) -> Redirect {
    let Some(s) = admin_of(cookies, &conn, org, MANAGE).await else {
        return to_home();
    };
    if check_csrf(&s, &form.csrf) {
        let (o, u, id, now) = (org.to_owned(), s.user.clone(), form.id.clone(), now_unix());
        if let Err(e) = run(&conn, move |c| family::cancel_invite(c, &o, &u, &id, now)).await {
            flash(&s.sid, e.message(), true);
        }
    }
    to_org(org, "")
}

#[derive(FromForm)]
struct MemberForm {
    csrf: String,
    member: String,
    #[field(default = false)]
    on: bool,
}

#[post("/reins/org/<org>/family/consent", data = "<form>")]
async fn consent(
    org: &str,
    form: Form<MemberForm>,
    cookies: &CookieJar<'_>,
    conn: DbConn,
    pool: &rocket::State<DbPool>,
) -> Redirect {
    let Some(s) = admin_of(cookies, &conn, org, MANAGE).await else {
        return to_home();
    };
    if check_csrf(&s, &form.csrf) {
        let (org_id, user, member, now) = (org.to_owned(), s.user.clone(), form.member.clone(), now_unix());
        match run(&conn, move |c| family::consent_leave(c, &org_id, &user, &member, now)).await {
            Ok(()) => {
                family::sweep_now(&conn, pool.inner()).await;
                flash(&s.sid, "They left the family.", false);
            }
            Err(e) => flash(&s.sid, e.message(), true),
        }
    }
    to_org(org, "")
}

#[post("/reins/org/<org>/family/minor", data = "<form>")]
async fn minor(
    org: &str,
    form: Form<MemberForm>,
    cookies: &CookieJar<'_>,
    conn: DbConn,
    pool: &rocket::State<DbPool>,
) -> Redirect {
    let Some(s) = admin_of(cookies, &conn, org, MANAGE).await else {
        return to_home();
    };
    if check_csrf(&s, &form.csrf) {
        let (org_id, user, member, on, now) =
            (org.to_owned(), s.user.clone(), form.member.clone(), form.on, now_unix());
        let target = member.clone();
        match run(&conn, move |c| family::set_minor(c, &org_id, &user, &member, on, now)).await {
            Ok(true) => {
                super::reviews::notify(pool.inner(), &[target], "family:minor");
                flash(&s.sid, "It counts once they accept on their phone.", false);
            }
            Ok(false) => {}
            Err(e) => flash(&s.sid, e.message(), true),
        }
    }
    to_org(org, "")
}

#[derive(FromForm)]
struct NoticeForm {
    csrf: String,
    days: i32,
}

#[post("/reins/org/<org>/family/notice", data = "<form>")]
async fn notice(org: &str, form: Form<NoticeForm>, cookies: &CookieJar<'_>, conn: DbConn) -> Redirect {
    let Some(s) = admin_of(cookies, &conn, org, MANAGE).await else {
        return to_home();
    };
    if check_csrf(&s, &form.csrf) {
        let (o, u, d, now) = (org.to_owned(), s.user.clone(), form.days, now_unix());
        if let Err(e) = run(&conn, move |c| family::set_notice(c, &o, &u, d, now)).await {
            flash(&s.sid, e.message(), true);
        }
    }
    to_org(org, "")
}

const CONNECT: &[Capability] = &[Capability::ManageConnections];

#[derive(FromForm)]
struct ProviderForm {
    csrf: String,
    member: String,
    provider: String,
}

/// A guardian connects a member's account: the family's executor prepares the sign-in, then the browser goes there.
#[post("/reins/org/<org>/family/connect", data = "<form>")]
async fn connect(org: &str, form: Form<ProviderForm>, cookies: &CookieJar<'_>, conn: DbConn) -> Redirect {
    let Some(s) = admin_of(cookies, &conn, org, CONNECT).await else {
        return to_home();
    };
    let Some(provider) = OAuthProvider::parse(&form.provider) else {
        return to_org(org, "");
    };
    if !check_csrf(&s, &form.csrf) {
        return to_org(org, "");
    }
    let (org_id, user, member, now) = (org.to_owned(), s.user.clone(), form.member.clone(), now_unix());
    match run(&conn, move |c| family_connect::begin(c, &org_id, &user, &member, provider, now)).await {
        Ok(job) => to_org(org, &format!("/family/connect/{job}")),
        Err(e) => {
            flash(&s.sid, e.message(), true);
            to_org(org, "")
        }
    }
}

/// Waits (refreshing) until the executor has the provider's sign-in ready, then sends the browser there.
#[get("/reins/org/<org>/family/connect/<job>")]
async fn connect_wait(org: &str, job: &str, cookies: &CookieJar<'_>, conn: DbConn) -> Result<Redirect, Page> {
    let Some(s) = admin_of(cookies, &conn, org, CONNECT).await else {
        return Err(super::console::not_found());
    };
    let (org_id, user, job_id) = (org.to_owned(), s.user.clone(), job.to_owned());
    match run(&conn, move |c| family_connect::auth_url(c, &org_id, &user, &job_id)).await {
        Ok(Some(url)) => Ok(Redirect::to(url)),
        Ok(None) => {
            let body = "<meta http-equiv=\"refresh\" content=\"2\"><section><h1>Connecting</h1>\
<p class=\"meta\">Waiting for the family's executor.</p></section>";
            Err(Page(pages::page("Connecting", None, body, &base())))
        }
        Err(e) => {
            flash(&s.sid, e.message(), true);
            Ok(to_org(org, ""))
        }
    }
}

/// The provider sends the guardian back here. Only the guardian who started it, in this browser, finishes it.
#[get("/reins/org/family/oauth/callback?<code>&<state>&<error>")]
async fn oauth_callback(
    code: Option<&str>,
    state: Option<&str>,
    error: Option<&str>,
    cookies: &CookieJar<'_>,
    conn: DbConn,
) -> Redirect {
    let Some(s) = signed(cookies, now_unix()) else {
        return to_home();
    };
    let (Some(code), Some(state), None) = (code, state, error) else {
        flash(&s.sid, "The sign-in was cancelled.", true);
        return to_home();
    };
    let (user, code, state, now) = (s.user.clone(), code.to_owned(), state.to_owned(), now_unix());
    match run(&conn, move |c| family_connect::callback(c, &user, &state, &code, now)).await {
        Ok(org) => {
            flash(&s.sid, "Connecting. It shows here in a moment.", false);
            to_org(&org, "")
        }
        Err(e) => {
            flash(&s.sid, e.message(), true);
            to_home()
        }
    }
}

#[post("/reins/org/<org>/family/revoke", data = "<form>")]
async fn revoke(org: &str, form: Form<ProviderForm>, cookies: &CookieJar<'_>, conn: DbConn) -> Redirect {
    let Some(s) = admin_of(cookies, &conn, org, CONNECT).await else {
        return to_home();
    };
    if let (true, Some(provider)) = (check_csrf(&s, &form.csrf), OAuthProvider::parse(&form.provider)) {
        let (org_id, user, member, now) = (org.to_owned(), s.user.clone(), form.member.clone(), now_unix());
        if let Err(e) = run(&conn, move |c| family_connect::revoke(c, &org_id, &user, &member, provider, now)).await {
            flash(&s.sid, e.message(), true);
        }
    }
    to_org(org, "")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_carry_the_server_and_the_secret() {
        let link = invite_link("abc");
        assert!(link.starts_with("reins-family://"), "{link}");
        assert!(link.ends_with("/abc"), "{link}");
    }
}
