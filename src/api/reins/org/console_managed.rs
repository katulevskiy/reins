//! The console's O2 routes: the org policy (draft, activate through a two-person device-signed change), retention,
//! recent decisions and the 05 §10 basics, the audit chain and its export, and executors. Each page re-checks the
//! viewer's capability; each POST carries the session's CSRF token.

use reins_proto::org::{AdminAction, Capability, ExecutorMode};
use rocket::{
    Request, Response, Route, State,
    form::{Form, FromForm},
    http::{ContentType, CookieJar},
    response::{self, Redirect, Responder},
};

use super::{
    OrgError, audit, can,
    console::{
        self, Page, admin_of, check_csrf, flash, not_found, org_name, outcome_text, propose, run, to_home, to_org,
    },
    console_managed_pages as pages,
    console_pages::{Frame, Tab},
    kms, policy,
};
use crate::{
    api::reins::now_unix,
    db::{
        DbConn, DbPool,
        models::{reins_org, reins_org_event, reins_org_exec, reins_org_keys, reins_org_request},
    },
};

pub fn routes() -> Vec<Route> {
    routes![
        policy_page,
        save_draft,
        activate,
        retention,
        requests,
        audit_page,
        export,
        executors,
        register_executor,
        remove_executor
    ]
}

const POLICY_VIEW: &[Capability] = &[Capability::AuthorPolicy, Capability::ActivatePolicy, Capability::ViewAdminEvents];
const AUDIT_VIEW: &[Capability] = &[Capability::ViewAdminEvents];
const EXEC_VIEW: &[Capability] = &[Capability::ManageConnections, Capability::ViewAdminEvents];

fn when(t: i64) -> String {
    chrono::DateTime::from_timestamp(t, 0).map(|d| d.format("%Y-%m-%d %H:%M UTC").to_string()).unwrap_or_default()
}

async fn frame_parts(conn: &DbConn, org: &str) -> Option<String> {
    org_name(conn, org).await.ok()
}

macro_rules! frame {
    ($s:expr, $org:expr, $name:expr, $base:expr, $tab:expr) => {
        Frame {
            base: &$base,
            email: &$s.email,
            csrf: &$s.csrf,
            org: Some(($org, &$name)),
            tab: $tab,
            flash: $s.flash.as_ref().map(|(t, b)| (t.as_str(), *b)),
        }
    };
}

#[get("/reins/org/<org>/policy")]
async fn policy_page(org: &str, cookies: &CookieJar<'_>, conn: DbConn) -> Page {
    let Some(s) = admin_of(cookies, &conn, org, POLICY_VIEW).await else {
        return not_found();
    };
    let (o, u) = (org.to_owned(), s.user.clone());
    let loaded = run(&conn, move |c| {
        let rows = reins_org_keys::q_policies(c, &o, 20)?;
        let active =
            rows.iter().find(|r| r.state == reins_org_keys::policy_state::ACTIVE).map(|r| pages::ActivePolicy {
                revision: r.revision,
                hash: r.hash.clone(),
                activated_at: r.activated_at.map(when).unwrap_or_default(),
            });
        let draft_text = rows
            .iter()
            .find(|r| r.state == reins_org_keys::policy_state::DRAFT)
            .or(rows.first())
            .map_or_else(|| policy::DEVELOPER_BASELINE.to_owned(), |r| pretty(&r.body));
        let drafts = rows
            .into_iter()
            .map(|r| pages::DraftRow {
                revision: r.revision,
                hash: r.hash,
                state: r.state,
            })
            .collect();
        let days = reins_org::q_org(c, &o)?.map_or(30, |t| u32::try_from(t.body_retention_days).unwrap_or(30));
        Ok((
            active,
            drafts,
            draft_text,
            days,
            can(c, &o, &u, Capability::AuthorPolicy)?,
            can(c, &o, &u, Capability::ActivatePolicy)?,
            super::console_policy::summary(c, &o, now_unix())?,
        ))
    })
    .await;
    let (Ok((active, drafts, draft_text, days, can_author, can_activate, rollout_line)), Some(name)) =
        (loaded, frame_parts(&conn, org).await)
    else {
        return not_found();
    };
    let base = console::base();
    let frame = frame!(s, org, name, base, Tab::Policy);
    Page(pages::policy(
        &frame,
        &pages::PolicyView {
            active,
            drafts,
            draft_text: &draft_text,
            retention_days: days,
            can_author,
            can_activate,
            rollout_line,
        },
    ))
}

fn pretty(json: &str) -> String {
    serde_json::from_str::<serde_json::Value>(json)
        .ok()
        .and_then(|v| serde_json::to_string_pretty(&v).ok())
        .unwrap_or_else(|| json.to_owned())
}

#[derive(FromForm)]
struct DraftForm {
    csrf: String,
    body: String,
}

#[post("/reins/org/<org>/policy/draft", data = "<form>")]
async fn save_draft(org: &str, form: Form<DraftForm>, cookies: &CookieJar<'_>, conn: DbConn) -> Redirect {
    let Some(s) = admin_of(cookies, &conn, org, &[Capability::AuthorPolicy]).await else {
        return to_home();
    };
    if !check_csrf(&s, &form.csrf) {
        return to_org(org, "/policy");
    }
    let (o, u, body, now) = (org.to_owned(), s.user.clone(), form.body.clone(), now_unix());
    match run(&conn, move |c| policy::save_draft(c, &o, &u, &body, now)).await {
        Ok((rev, _)) => flash(&s.sid, &format!("Saved as revision {rev}."), false),
        Err(e) => flash(&s.sid, e.message(), true),
    }
    to_org(org, "/policy")
}

#[derive(FromForm)]
struct ActivateForm {
    csrf: String,
    revision: u64,
    hash: String,
    /// The simulation report it rests on; the draft's newest when not named.
    report: Option<String>,
}

#[post("/reins/org/<org>/policy/activate", data = "<form>")]
async fn activate(
    org: &str,
    form: Form<ActivateForm>,
    cookies: &CookieJar<'_>,
    conn: DbConn,
    pool: &State<DbPool>,
) -> Redirect {
    let Some(s) = admin_of(cookies, &conn, org, &[Capability::ActivatePolicy]).await else {
        return to_home();
    };
    if !check_csrf(&s, &form.csrf) {
        return to_org(org, "/policy");
    }
    let mut report = form.report.clone().filter(|r| !r.is_empty());
    if report.is_none() {
        let (o, h) = (org.to_owned(), form.hash.clone());
        report = conn
            .run(move |c| crate::db::models::reins_org_policy_code::q_last_simulation(c, &o, &h))
            .await
            .ok()
            .flatten()
            .map(|r| r.report_hash);
    }
    let action = AdminAction::ActivatePolicy {
        revision: form.revision,
        hash: form.hash.clone(),
        report,
    };
    signed_change(&conn, pool, org, &s, action).await;
    to_org(org, "/policy")
}

/// Proposes a change that needs device signatures and tells the proposer to confirm it on their phone.
async fn signed_change(conn: &DbConn, pool: &DbPool, org: &str, s: &console::Signed, action: AdminAction) {
    let r = propose(conn, pool, org, &s.user, action, None).await;
    if let Ok(super::admin::Proposed::Pending(id)) = &r {
        super::reviews::notify(pool, std::slice::from_ref(&s.user), id);
        flash(&s.sid, "Confirm it on your phone. A second admin then approves on theirs.", false);
        return;
    }
    let (text, bad) = outcome_text(&r, "Done.");
    flash(&s.sid, &text, bad);
}

#[derive(FromForm)]
struct RetentionForm {
    csrf: String,
    days: u32,
}

#[post("/reins/org/<org>/retention", data = "<form>")]
async fn retention(
    org: &str,
    form: Form<RetentionForm>,
    cookies: &CookieJar<'_>,
    conn: DbConn,
    pool: &State<DbPool>,
) -> Redirect {
    let Some(s) = admin_of(cookies, &conn, org, &[Capability::ActivatePolicy]).await else {
        return to_home();
    };
    if !check_csrf(&s, &form.csrf) {
        return to_org(org, "/policy");
    }
    signed_change(
        &conn,
        pool,
        org,
        &s,
        AdminAction::SetRetention {
            days: form.days,
        },
    )
    .await;
    to_org(org, "/policy")
}

#[get("/reins/org/<org>/requests")]
async fn requests(org: &str, cookies: &CookieJar<'_>, conn: DbConn) -> Page {
    let Some(s) = admin_of(cookies, &conn, org, AUDIT_VIEW).await else {
        return not_found();
    };
    let o = org.to_owned();
    let now = now_unix();
    let loaded = run(&conn, move |c| {
        let mut rows = Vec::new();
        for r in reins_org_request::q_requests_of_org(c, &o, None, 100)? {
            let trace: serde_json::Value = serde_json::from_str(&r.trace).unwrap_or_default();
            let word = match r.outcome.as_str() {
                "auto" => "Auto",
                "deny" => "Deny",
                _ => "Ask",
            };
            let line = format!(
                "{word} · {} · {}",
                trace["zone"].as_str().unwrap_or("?"),
                trace["level"].as_str().unwrap_or("?")
            );
            rows.push(pages::RequestRow {
                at: when(r.created_at),
                member: console::email_of(c, &r.user_uuid)?,
                agent: reins_org_request_label(c, &r.connection_uuid)?,
                tool: r.tool,
                line,
                state: r.state,
            });
        }
        let k = reins_org_request::q_counts_since(c, &o, now - 7 * 86_400)?;
        Ok((
            rows,
            pages::Counts {
                total: k.total,
                auto: k.auto,
                reviewed: k.reviewed,
                denied: k.denied,
                expired: k.expired,
                median: k.median_decision_secs,
            },
        ))
    })
    .await;
    let (Ok((rows, counts)), Some(name)) = (loaded, frame_parts(&conn, org).await) else {
        return not_found();
    };
    let base = console::base();
    let frame = frame!(s, org, name, base, Tab::Requests);
    Page(pages::requests(&frame, &rows, &counts))
}

fn reins_org_request_label(c: &mut crate::db::DbConnInner, connection: &str) -> Result<String, OrgError> {
    use diesel::prelude::*;

    use crate::db::schema::reins_connections;
    Ok(reins_connections::table
        .filter(reins_connections::uuid.eq(connection))
        .select(reins_connections::label)
        .first::<String>(c)
        .optional()?
        .unwrap_or_else(|| "removed".to_owned()))
}

#[get("/reins/org/<org>/audit")]
async fn audit_page(org: &str, cookies: &CookieJar<'_>, conn: DbConn) -> Page {
    let Some(s) = admin_of(cookies, &conn, org, AUDIT_VIEW).await else {
        return not_found();
    };
    let o = org.to_owned();
    let now = now_unix();
    let loaded = run(&conn, move |c| {
        let keys = kms::public_keys(c, &o, now)?;
        Ok(pages::AuditView {
            intact: audit::intact(c, &o)?,
            head: reins_org_event::q_head(c, &o)?.map(|(seq, _)| seq),
            checkpoint: reins_org_exec::q_last_checkpoint(c, &o)?.map(|cp| (cp.seq, when(cp.at))),
            decision_key: keys.decision,
            issuer_key: keys.issuer,
            kms: kms::kms()?.id(),
        })
    })
    .await;
    let (Ok(view), Some(name)) = (loaded, frame_parts(&conn, org).await) else {
        return not_found();
    };
    let base = console::base();
    let frame = frame!(s, org, name, base, Tab::Audit);
    Page(pages::audit(&frame, &view))
}

/// The audit export download (JSON Lines).
pub struct Jsonl {
    name: String,
    body: String,
}

impl<'r> Responder<'r, 'static> for Jsonl {
    fn respond_to(self, _: &'r Request<'_>) -> response::Result<'static> {
        Response::build()
            .header(ContentType::new("application", "x-ndjson"))
            .raw_header("Content-Disposition", format!("attachment; filename=\"{}\"", self.name))
            .raw_header("Cache-Control", "no-store")
            .sized_body(self.body.len(), std::io::Cursor::new(self.body))
            .ok()
    }
}

#[get("/reins/org/<org>/audit/export?<from>")]
async fn export(org: &str, from: Option<i64>, cookies: &CookieJar<'_>, conn: DbConn) -> Result<Jsonl, Page> {
    let Some(s) = admin_of(cookies, &conn, org, AUDIT_VIEW).await else {
        return Err(not_found());
    };
    let (o, u, now) = (org.to_owned(), s.user.clone(), now_unix());
    let body = run(&conn, move |c| audit::export(c, &o, from, Some(&u), now)).await.map_err(|_| not_found())?;
    Ok(Jsonl {
        name: format!("reins-audit-{}-{now}.jsonl", org.get(..8).unwrap_or(org)),
        body,
    })
}

#[get("/reins/org/<org>/executors")]
async fn executors(org: &str, cookies: &CookieJar<'_>, conn: DbConn) -> Page {
    let Some(s) = admin_of(cookies, &conn, org, EXEC_VIEW).await else {
        return not_found();
    };
    let (o, u, now) = (org.to_owned(), s.user.clone(), now_unix());
    let loaded = run(&conn, move |c| {
        let rows = reins_org_exec::q_executors(c, &o)?
            .into_iter()
            .map(|e| pages::ExecutorRow {
                id: e.executor_id,
                mode: ExecutorMode::parse(&e.mode)
                    .map_or("?", |m| match m {
                        ExecutorMode::Cloud => "Reins Cloud",
                        ExecutorMode::Dedicated => "Dedicated",
                        ExecutorMode::SelfHosted => "Self-hosted",
                    })
                    .to_owned(),
                last_seen: e.last_seen.map_or_else(|| "never".to_owned(), when),
                removed: e.revoked_at.is_some(),
            })
            .collect::<Vec<_>>();
        let keys = kms::public_keys(c, &o, now)?;
        Ok((
            rows,
            can(c, &o, &u, Capability::ManageConnections)?,
            (keys.issuer.0, keys.issuer.1, keys.decision.0, keys.decision.1),
        ))
    })
    .await;
    let (Ok((rows, can_manage, pins)), Some(name)) = (loaded, frame_parts(&conn, org).await) else {
        return not_found();
    };
    let base = console::base();
    let frame = frame!(s, org, name, base, Tab::Executors);
    Page(pages::executors(&frame, &rows, can_manage, &pins))
}

#[derive(FromForm)]
struct ExecutorForm {
    csrf: String,
    executor_id: String,
    mode: String,
    public_key: String,
}

#[post("/reins/org/<org>/executors", data = "<form>")]
async fn register_executor(
    org: &str,
    form: Form<ExecutorForm>,
    cookies: &CookieJar<'_>,
    conn: DbConn,
    pool: &State<DbPool>,
) -> Redirect {
    let Some(s) = admin_of(cookies, &conn, org, &[Capability::ManageConnections]).await else {
        return to_home();
    };
    if !check_csrf(&s, &form.csrf) {
        return to_org(org, "/executors");
    }
    let Some(mode) = ExecutorMode::parse(&form.mode) else {
        flash(&s.sid, "Choose where the executor runs.", true);
        return to_org(org, "/executors");
    };
    let action = AdminAction::RegisterExecutor {
        executor_id: form.executor_id.trim().to_owned(),
        mode,
        public_key: form.public_key.trim().to_owned(),
    };
    signed_change(&conn, pool, org, &s, action).await;
    to_org(org, "/executors")
}

#[derive(FromForm)]
struct RemoveExecutorForm {
    csrf: String,
    executor_id: String,
}

#[post("/reins/org/<org>/executors/remove", data = "<form>")]
async fn remove_executor(
    org: &str,
    form: Form<RemoveExecutorForm>,
    cookies: &CookieJar<'_>,
    conn: DbConn,
    pool: &State<DbPool>,
) -> Redirect {
    let Some(s) = admin_of(cookies, &conn, org, &[Capability::ManageConnections]).await else {
        return to_home();
    };
    if !check_csrf(&s, &form.csrf) {
        return to_org(org, "/executors");
    }
    signed_change(
        &conn,
        pool,
        org,
        &s,
        AdminAction::RemoveExecutor {
            executor_id: form.executor_id.clone(),
        },
    )
    .await;
    to_org(org, "/executors")
}
