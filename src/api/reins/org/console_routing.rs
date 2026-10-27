//! The console's O3 routes: routing (group backups and windows, managers on the Team tier, the GitHub reader,
//! on-call sources), channels (Slack, Teams), the SLA report, and templates. Each page re-checks the viewer's
//! capability; each POST carries the session's CSRF token; changes to who approves go through two-person
//! `admin::propose`, and secrets are sealed as posted and never shown again.

use reins_proto::org::{AdminAction, Capability, ChannelKind, OnCallProvider};
use rocket::{
    Route, State,
    form::{Form, FromForm},
    http::CookieJar,
    response::Redirect,
};
use serde_json::{Value, json};

use super::{
    OrgError, channels,
    console::{
        self, Page, Signed, admin_of, check_csrf, flash, not_found, org_name, outcome_text, propose, run, to_org,
    },
    console_pages::{Frame, Tab},
    console_routing_pages as pages, github_reader, policy,
    routing::{oncall, sla},
    templates::{self, Template, TemplateParams},
};
use crate::{
    api::reins::now_unix,
    db::{
        DbConn, DbConnInner, DbPool,
        models::{reins_org, reins_org_github_reader, reins_org_routing as store},
    },
};

pub fn routes() -> Vec<Route> {
    routes![
        routing,
        group,
        manager,
        reader,
        reader_key,
        oncall_source,
        oncall_key,
        channels_page,
        install,
        toggle,
        sla_page,
        templates_page,
        template_post
    ]
}

const CONNECTIONS: &[Capability] = &[Capability::ManageConnections];
const REPORTS: &[Capability] = &[Capability::ViewAdminEvents];
const AUTHORS: &[Capability] = &[Capability::AuthorPolicy];

fn frame<'a>(s: &'a Signed, org: &'a str, name: &'a str, base: &'a str) -> Frame<'a> {
    Frame {
        base,
        email: &s.email,
        csrf: &s.csrf,
        org: Some((org, name)),
        tab: Tab::Routing,
        flash: s.flash.as_ref().map(|(t, b)| (t.as_str(), *b)),
    }
}

/// Everything the routing page shows.
pub fn routing_view(c: &mut DbConnInner, org: &str) -> Result<pages::RoutingView, OrgError> {
    let groups = reins_org::q_groups(c, org)?;
    let settings = store::q_group_settings(c, org)?;
    let group_rows = groups
        .iter()
        .map(|(id, name, _)| {
            let s = settings.iter().find(|s| s.group_uuid == *id);
            let backup = s
                .and_then(|s| s.backup_group.as_ref())
                .and_then(|b| groups.iter().find(|g| g.0 == *b))
                .map(|g| g.1.clone());
            (id.clone(), name.clone(), backup, s.and_then(|s| s.window_secs).map(|w| w / 60))
        })
        .collect();
    let mut managers = Vec::new();
    for (user, (manager, source)) in super::routing::manager_links(c, org)? {
        managers.push((super::reviews::email_of_user(c, &user)?, super::reviews::email_of_user(c, &manager)?, source));
    }
    let members = reins_org::q_members(c, org)?
        .into_iter()
        .map(|m| (m.uuid.to_string(), m.user_uuid.to_string()))
        .collect::<Vec<_>>();
    let mut member_rows = Vec::new();
    for (membership, user) in members {
        if super::member_of(c, org, &user)?.is_some() {
            member_rows.push((membership, super::reviews::email_of_user(c, &user)?));
        }
    }
    let reader =
        reins_org_github_reader::q_github_reader(c, org)?.map(|r| (r.app_id, r.installation_id, r.key_id.is_some()));
    let linked = store::q_people(c, org)?.iter().filter(|p| p.github_verified_via.is_some()).count();
    Ok(pages::RoutingView {
        groups: group_rows,
        group_choices: groups.into_iter().map(|(id, name, _)| (id, name)).collect(),
        managers,
        members: member_rows,
        team_tier: !reins_org::q_org(c, org)?.is_some_and(|t| t.tier().scim()),
        reader,
        linked_logins: linked,
        oncall: store::q_oncalls(c, org)?,
    })
}

#[get("/reins/org/<org>/routing")]
async fn routing(org: &str, cookies: &CookieJar<'_>, conn: DbConn) -> Page {
    let Some(s) = admin_of(cookies, &conn, org, CONNECTIONS).await else {
        return not_found();
    };
    let o = org.to_owned();
    let (Ok(view), Ok(name)) = (run(&conn, move |c| routing_view(c, &o)).await, org_name(&conn, org).await) else {
        return not_found();
    };
    let base = console::base();
    Page(pages::routing(&frame(&s, org, &name, &base), &view))
}

async fn proposed(
    org: &str,
    s: &Signed,
    conn: &DbConn,
    pool: &DbPool,
    action: AdminAction,
    done: &str,
    back: &str,
) -> Redirect {
    let r = propose(conn, pool, org, &s.user, action, None).await;
    let (text, bad) = outcome_text(&r, done);
    flash(&s.sid, &text, bad);
    to_org(org, back)
}

async fn checked(
    cookies: &CookieJar<'_>,
    conn: &DbConn,
    org: &str,
    csrf: &str,
    caps: &'static [Capability],
) -> Option<Signed> {
    admin_of(cookies, conn, org, caps).await.filter(|s| check_csrf(s, csrf))
}

#[derive(FromForm)]
struct GroupForm {
    csrf: String,
    group: String,
    backup: String,
    window_mins: String,
}

#[post("/reins/org/<org>/routing/group", data = "<form>")]
async fn group(
    org: &str,
    form: Form<GroupForm>,
    cookies: &CookieJar<'_>,
    conn: DbConn,
    pool: &State<DbPool>,
) -> Redirect {
    let f = form.into_inner();
    let Some(s) = checked(cookies, &conn, org, &f.csrf, CONNECTIONS).await else {
        return console::to_home();
    };
    let action = AdminAction::SetGroupRouting {
        group: f.group,
        backup: Some(f.backup).filter(|b| !b.is_empty()),
        window_secs: f.window_mins.trim().parse::<u64>().ok().map(|m| m * 60),
    };
    proposed(org, &s, &conn, pool, action, "Saved.", "/routing").await
}

#[derive(FromForm)]
struct ManagerForm {
    csrf: String,
    member: String,
    manager: String,
}

#[post("/reins/org/<org>/routing/manager", data = "<form>")]
async fn manager(
    org: &str,
    form: Form<ManagerForm>,
    cookies: &CookieJar<'_>,
    conn: DbConn,
    pool: &State<DbPool>,
) -> Redirect {
    let f = form.into_inner();
    let caps: &'static [Capability] = &[Capability::ManageMembers];
    let Some(s) = checked(cookies, &conn, org, &f.csrf, caps).await else {
        return console::to_home();
    };
    let action = AdminAction::SetManager {
        member: f.member,
        manager: Some(f.manager).filter(|m| !m.is_empty()),
    };
    proposed(org, &s, &conn, pool, action, "Saved.", "/routing").await
}

#[derive(FromForm)]
struct ReaderForm {
    csrf: String,
    app_id: u64,
    installation_id: u64,
}

#[post("/reins/org/<org>/routing/reader", data = "<form>")]
async fn reader(
    org: &str,
    form: Form<ReaderForm>,
    cookies: &CookieJar<'_>,
    conn: DbConn,
    pool: &State<DbPool>,
) -> Redirect {
    let f = form.into_inner();
    let Some(s) = checked(cookies, &conn, org, &f.csrf, CONNECTIONS).await else {
        return console::to_home();
    };
    let action = AdminAction::SetGithubReader {
        app_id: f.app_id,
        installation_id: f.installation_id,
    };
    proposed(org, &s, &conn, pool, action, "Saved.", "/routing").await
}

#[derive(FromForm)]
struct ReaderKeyForm {
    csrf: String,
    pem: String,
    webhook_secret: String,
}

#[post("/reins/org/<org>/routing/reader/key", data = "<form>")]
async fn reader_key(org: &str, form: Form<ReaderKeyForm>, cookies: &CookieJar<'_>, conn: DbConn) -> Redirect {
    let f = form.into_inner();
    let Some(s) = checked(cookies, &conn, org, &f.csrf, CONNECTIONS).await else {
        return console::to_home();
    };
    let (o, now) = (org.to_owned(), now_unix());
    let r = run(&conn, move |c| set_reader_key(c, &o, &f.pem, &f.webhook_secret, now)).await;
    flash(&s.sid, r.as_ref().map_or_else(OrgError::message, |()| "Key saved."), r.is_err());
    to_org(org, "/routing")
}

/// Posts the reader's key: only into a reader with none yet. A key in place is replaced only by setting the reader
/// again (a two-person `SetGithubReader` for another app, which drops its secrets).
pub fn set_reader_key(
    c: &mut DbConnInner,
    org: &str,
    pem: &str,
    webhook_secret: &str,
    now: i64,
) -> Result<(), OrgError> {
    if reins_org_github_reader::q_github_reader(c, org)?.is_some_and(|r| r.key_id.is_some()) {
        return Err(OrgError::Invalid("A key is set. Set the reader again to replace it."));
    }
    github_reader::set_secrets(c, org, pem, webhook_secret, now)
}

#[derive(FromForm)]
struct OnCallForm {
    csrf: String,
    alias: String,
    provider: String,
    schedule_id: String,
}

#[post("/reins/org/<org>/routing/oncall", data = "<form>")]
async fn oncall_source(
    org: &str,
    form: Form<OnCallForm>,
    cookies: &CookieJar<'_>,
    conn: DbConn,
    pool: &State<DbPool>,
) -> Redirect {
    let f = form.into_inner();
    let Some(s) = checked(cookies, &conn, org, &f.csrf, CONNECTIONS).await else {
        return console::to_home();
    };
    let Some(provider) = OnCallProvider::parse(&f.provider) else {
        flash(&s.sid, "Pick a provider.", true);
        return to_org(org, "/routing");
    };
    let action = AdminAction::SetOnCallSource {
        alias: f.alias.trim().to_ascii_lowercase(),
        provider,
        schedule_id: f.schedule_id.trim().to_owned(),
    };
    proposed(org, &s, &conn, pool, action, "Saved.", "/routing").await
}

#[derive(FromForm)]
struct OnCallKeyForm {
    csrf: String,
    alias: String,
    key: String,
}

#[post("/reins/org/<org>/routing/oncall/key", data = "<form>")]
async fn oncall_key(org: &str, form: Form<OnCallKeyForm>, cookies: &CookieJar<'_>, conn: DbConn) -> Redirect {
    let f = form.into_inner();
    let Some(s) = checked(cookies, &conn, org, &f.csrf, CONNECTIONS).await else {
        return console::to_home();
    };
    let (o, now) = (org.to_owned(), now_unix());
    let r = run(&conn, move |c| oncall::set_key(c, &o, &f.alias, &f.key, now)).await;
    flash(&s.sid, r.as_ref().map_or_else(OrgError::message, |()| "Key saved."), r.is_err());
    to_org(org, "/routing")
}

#[get("/reins/org/<org>/channels")]
async fn channels_page(org: &str, cookies: &CookieJar<'_>, conn: DbConn) -> Page {
    let Some(s) = admin_of(cookies, &conn, org, CONNECTIONS).await else {
        return not_found();
    };
    let o = org.to_owned();
    let (Ok(rows), Ok(name)) = (run(&conn, move |c| Ok(store::q_channels(c, &o)?)).await, org_name(&conn, org).await)
    else {
        return not_found();
    };
    let base = console::base();
    Page(pages::channels(
        &frame(&s, org, &name, &base),
        &rows,
        &console::callback_url().replace("/reins/org/callback", ""),
    ))
}

#[derive(FromForm)]
struct InstallForm {
    csrf: String,
    kind: String,
    bot_token: Option<String>,
    signing_secret: Option<String>,
    app_id: Option<String>,
    app_secret: Option<String>,
    tenant_id: Option<String>,
    teams_app_id: Option<String>,
}

#[post("/reins/org/<org>/channels/install", data = "<form>")]
async fn install(org: &str, form: Form<InstallForm>, cookies: &CookieJar<'_>, conn: DbConn) -> Redirect {
    let f = form.into_inner();
    let Some(s) = checked(cookies, &conn, org, &f.csrf, CONNECTIONS).await else {
        return console::to_home();
    };
    let Some(kind) = ChannelKind::parse(&f.kind) else {
        return to_org(org, "/channels");
    };
    let config = match kind {
        ChannelKind::Slack => json!({"bot_token": f.bot_token, "signing_secret": f.signing_secret}),
        ChannelKind::Teams => json!({"app_id": f.app_id, "app_secret": f.app_secret, "tenant_id": f.tenant_id,
                                     "teams_app_id": f.teams_app_id}),
    };
    // A Teams bot is claimed only by proving its secret to Microsoft first.
    let proof = match kind {
        ChannelKind::Teams => match channels::teams::TeamsConfig::from_value(&config) {
            Some(cfg) => match channels::teams::verify(cfg).await {
                Ok(p) => Some(p),
                Err(e) => {
                    info!("Org {org}: Teams install: {e}");
                    flash(&s.sid, "Microsoft didn't accept that bot's id and secret.", true);
                    return to_org(org, "/channels");
                }
            },
            None => None,
        },
        ChannelKind::Slack => None,
    };
    let (o, now) = (org.to_owned(), now_unix());
    let r = run(&conn, move |c| channels::install(c, &o, kind, &config, proof.as_ref(), now)).await;
    flash(&s.sid, r.as_ref().map_or_else(OrgError::message, |()| "Installed. Turn it on to use it."), r.is_err());
    to_org(org, "/channels")
}

#[derive(FromForm)]
struct ToggleForm {
    csrf: String,
    kind: String,
    enabled: bool,
}

#[post("/reins/org/<org>/channels/toggle", data = "<form>")]
async fn toggle(
    org: &str,
    form: Form<ToggleForm>,
    cookies: &CookieJar<'_>,
    conn: DbConn,
    pool: &State<DbPool>,
) -> Redirect {
    let f = form.into_inner();
    let Some(s) = checked(cookies, &conn, org, &f.csrf, CONNECTIONS).await else {
        return console::to_home();
    };
    let Some(channel) = ChannelKind::parse(&f.kind) else {
        return to_org(org, "/channels");
    };
    let (o, enabled) = (org.to_owned(), f.enabled);
    let config = if enabled {
        run(&conn, move |c| channels::config_digest(c, &o, channel)).await.ok().flatten()
    } else {
        None
    };
    let action = AdminAction::SetApprovalChannel {
        channel,
        enabled,
        config,
    };
    proposed(org, &s, &conn, pool, action, "Saved.", "/channels").await
}

#[get("/reins/org/<org>/reports/sla?<days>")]
async fn sla_page(org: &str, days: Option<i64>, cookies: &CookieJar<'_>, conn: DbConn) -> Page {
    let Some(s) = admin_of(cookies, &conn, org, REPORTS).await else {
        return not_found();
    };
    let days = if days == Some(30) {
        30
    } else {
        7
    };
    let (o, now) = (org.to_owned(), now_unix());
    let (Ok((overall, groups)), Ok(name)) =
        (run(&conn, move |c| sla::sla(c, &o, now - days * 86_400, now)).await, org_name(&conn, org).await)
    else {
        return not_found();
    };
    let base = console::base();
    Page(pages::sla(&frame(&s, org, &name, &base), days, &overall, &groups))
}

#[get("/reins/org/<org>/templates")]
async fn templates_page(org: &str, cookies: &CookieJar<'_>, conn: DbConn) -> Page {
    let Some(s) = admin_of(cookies, &conn, org, AUTHORS).await else {
        return not_found();
    };
    let Ok(name) = org_name(&conn, org).await else {
        return not_found();
    };
    let base = console::base();
    let params = serde_json::to_string_pretty(&TemplateParams::default()).unwrap_or_default();
    Page(pages::templates(&frame(&s, org, &name, &base), Template::DeveloperBaseline, &params, None))
}

/// A template's rules merged into a copy of the active policy (or O2's baseline), as draft text.
pub fn template_draft(
    c: &mut DbConnInner,
    org: &str,
    t: Template,
    params: &TemplateParams,
) -> Result<String, OrgError> {
    let base = match policy::active(c, org)? {
        Some((p, _)) => p,
        None => serde_json::from_str::<reins_policy::org::OrgPolicy>(policy::DEVELOPER_BASELINE)
            .map_err(|e| OrgError::Internal(e.to_string()))?,
    };
    let merged = templates::apply(&base, templates::build(t, params));
    serde_json::to_string_pretty(&merged).map_err(|e| OrgError::Internal(e.to_string()))
}

/// Saves a template as the next draft; needs [`Capability::AuthorPolicy`] (O2's `save_draft` checks it).
pub fn draft_template(
    c: &mut DbConnInner,
    org: &str,
    author: &str,
    t: Template,
    params: &TemplateParams,
    now: i64,
) -> Result<(u64, String), OrgError> {
    super::require(c, org, author, Capability::AuthorPolicy)?;
    let text = template_draft(c, org, t, params)?;
    policy::save_draft(c, org, author, &text, now)
}

#[derive(FromForm)]
struct TemplateForm {
    csrf: String,
    template: String,
    params: String,
    op: String,
}

#[post("/reins/org/<org>/templates", data = "<form>")]
async fn template_post(
    org: &str,
    form: Form<TemplateForm>,
    cookies: &CookieJar<'_>,
    conn: DbConn,
) -> Result<Page, Redirect> {
    let input = form.into_inner();
    let Some(s) = checked(cookies, &conn, org, &input.csrf, AUTHORS).await else {
        return Err(console::to_home());
    };
    let base = console::base();
    let Ok(name) = org_name(&conn, org).await else {
        return Ok(not_found());
    };
    let template = Template::parse(&input.template).unwrap_or(Template::DeveloperBaseline);
    let parsed: Result<TemplateParams, _> = serde_json::from_str(&input.params);
    let Ok(params) = parsed else {
        flash(&s.sid, "Those parameters aren't valid JSON.", true);
        return Ok(Page(pages::templates(&frame(&s, org, &name, &base), template, &input.params, None)));
    };
    let (o, u, now) = (org.to_owned(), s.user.clone(), now_unix());
    if input.op == "draft" {
        let r = run(&conn, move |c| draft_template(c, &o, &u, template, &params, now)).await;
        let text = r.as_ref().map_or_else(|e| e.message().to_owned(), |(rev, _)| format!("Saved as revision {rev}."));
        flash(&s.sid, &text, r.is_err());
        return Err(to_org(org, "/policy"));
    }
    let p2 = params.clone();
    let preview = run(&conn, move |c| template_draft(c, &o, template, &p2)).await.unwrap_or_default();
    let shown: Value = serde_json::to_value(&params).unwrap_or_default();
    let text = serde_json::to_string_pretty(&shown).unwrap_or_default();
    Ok(Page(pages::templates(&frame(&s, org, &name, &base), template, &text, Some(&preview))))
}

#[cfg(all(test, sqlite))]
mod tests {
    use super::*;
    use crate::api::reins::org::{
        kms::tests::use_test_kms,
        testing::{add_org, db, member},
    };
    use reins_proto::org::Role;

    #[test]
    fn templates_need_author_policy() {
        use_test_kms();
        let mut c = db();
        add_org(&mut c, "o1");
        member(&mut c, "o1", "ann", &[Role::PolicyAuthor]);
        member(&mut c, "o1", "lee", &[]);
        member(&mut c, "o1", "ida", &[Role::ConnectionAdmin]);
        let p = TemplateParams::default();
        for who in ["lee", "ida"] {
            assert_eq!(draft_template(&mut c, "o1", who, Template::Finance, &p, 5), Err(OrgError::Forbidden));
        }
        let (rev, _) = draft_template(&mut c, "o1", "ann", Template::Finance, &p, 5).unwrap();
        let row =
            crate::db::models::reins_org_keys::q_policy(&mut c, "o1", i64::try_from(rev).unwrap()).unwrap().unwrap();
        assert!(row.body.contains("tpl.finance.1"));
        // A draft, not active: activation stays two-person.
        assert!(policy::active(&mut c, "o1").unwrap().is_none());
    }

    #[test]
    fn a_reader_key_is_replaced_only_through_a_two_person_change() {
        use_test_kms();
        let mut c = db();
        add_org(&mut c, "o1");
        let pem = String::from_utf8(openssl::rsa::Rsa::generate(2048).unwrap().private_key_to_pem().unwrap()).unwrap();
        assert!(set_reader_key(&mut c, "o1", &pem, "a-webhook-secret-123", 5).is_err(), "no reader set up");
        reins_org_github_reader::q_set_github_reader(&mut c, "o1", 7, 8, 5).unwrap();
        set_reader_key(&mut c, "o1", &pem, "a-webhook-secret-123", 6).unwrap();
        assert_eq!(
            set_reader_key(&mut c, "o1", &pem, "another-secret-4567", 7),
            Err(OrgError::Invalid("A key is set. Set the reader again to replace it."))
        );
        // Setting the reader again (another app, after its two-person change) drops the key.
        reins_org_github_reader::q_set_github_reader(&mut c, "o1", 70, 9, 8).unwrap();
        set_reader_key(&mut c, "o1", &pem, "another-secret-4567", 9).unwrap();
    }
}
