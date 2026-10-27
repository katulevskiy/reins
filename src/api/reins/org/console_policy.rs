//! The console's O4 routes under `/reins/org/<org>/policy`: drafts (from the repo, the console or a template) and
//! their simulations, the staged rollout and its controls, retained samples (an audited read), the history export, and
//! the policy repo with its GitHub reader. Every page re-checks the viewer's capability; every POST carries CSRF.

use std::collections::{BTreeMap, HashMap};

use reins_policy::{
    org::OrgPolicy,
    source::{self, RolloutPlanSource, SourceError},
};
use reins_proto::{
    org::{AdminAction, Capability},
    rollout::Stage,
};
use rocket::{
    Route, State,
    form::{Form, FromForm},
    http::CookieJar,
    response::Redirect,
};

use super::{
    OrgError, activation, can,
    console::{
        self, Page, admin_of, check_csrf, flash, not_found, org_name, outcome_text, propose, run, to_home, to_org,
    },
    console_pages::{Frame, Tab},
    console_policy_pages as pages, github_reader, policy, policy_repo,
    rollout::{self, Control},
    simulation,
};
use crate::{
    api::reins::now_unix,
    db::{
        DbConn, DbPool,
        models::{reins_org_github_reader, reins_org_keys, reins_org_policy_code as code},
    },
};

pub fn routes() -> Vec<Route> {
    routes![
        drafts,
        simulate,
        new_draft,
        save_new_draft,
        simulation_page,
        sample,
        history,
        rollout_page,
        control,
        repo_page,
        set_reader,
        set_reader_secrets,
        bind_repo,
        unbind_repo
    ]
}

const VIEW: &[Capability] = &[Capability::AuthorPolicy, Capability::ActivatePolicy, Capability::ViewAdminEvents];
const REPO_VIEW: &[Capability] = &[Capability::SetUpOrg, Capability::ManageConnections, Capability::ActivatePolicy];

fn when(t: i64) -> String {
    chrono::DateTime::from_timestamp(t, 0).map(|d| d.format("%Y-%m-%d %H:%M UTC").to_string()).unwrap_or_default()
}

macro_rules! frame {
    ($s:expr, $org:expr, $name:expr, $base:expr) => {
        Frame {
            base: &$base,
            email: &$s.email,
            csrf: &$s.csrf,
            org: Some(($org, &$name)),
            tab: Tab::Policy,
            flash: $s.flash.as_ref().map(|(t, b)| (t.as_str(), *b)),
        }
    };
}

#[get("/reins/org/<org>/policy/drafts")]
async fn drafts(org: &str, cookies: &CookieJar<'_>, conn: DbConn) -> Page {
    let Some(s) = admin_of(cookies, &conn, org, VIEW).await else {
        return not_found();
    };
    let (o, u, now) = (org.to_owned(), s.user.clone(), now_unix());
    let loaded = run(&conn, move |c| {
        let mut items = Vec::new();
        for r in code::q_drafts(c, &o, 30)? {
            let sim = match code::q_last_simulation(c, &o, &r.hash)? {
                Some(row) => {
                    let report: reins_policy::simulate::SimulationReport =
                        serde_json::from_str(&row.report).map_err(|e| OrgError::Internal(e.to_string()))?;
                    let fresh = row.ran_at >= now - activation::EVIDENCE_SECS && report.drift == 0;
                    Some((row.report_hash, report.line(), fresh))
                }
                None => None,
            };
            items.push(pages::DraftItem {
                revision: r.revision,
                hash: r.hash,
                state: r.state,
                source: r.source_ref.unwrap_or_default(),
                simulation: sim,
            });
        }
        Ok((items, can(c, &o, &u, Capability::AuthorPolicy)?, can(c, &o, &u, Capability::ActivatePolicy)?))
    })
    .await;
    let (Ok((items, can_author, can_activate)), Ok(name)) = (loaded, org_name(&conn, org).await) else {
        return not_found();
    };
    let base = console::base();
    Page(pages::drafts(
        &frame!(s, org, name, base),
        &pages::DraftsView {
            drafts: items,
            can_author,
            can_activate,
        },
    ))
}

#[derive(FromForm)]
struct CsrfForm {
    csrf: String,
}

#[post("/reins/org/<org>/policy/drafts/<revision>/simulate", data = "<form>")]
async fn simulate(org: &str, revision: u64, form: Form<CsrfForm>, cookies: &CookieJar<'_>, conn: DbConn) -> Redirect {
    let Some(s) = admin_of(cookies, &conn, org, &[Capability::AuthorPolicy]).await else {
        return to_home();
    };
    if !check_csrf(&s, &form.csrf) {
        return to_org(org, "/policy/drafts");
    }
    let (o, u, now) = (org.to_owned(), s.user.clone(), now_unix());
    match run(&conn, move |c| simulation::run(c, &o, &u, revision, None, None, Some("console"), now)).await {
        Ok(r) => flash(&s.sid, &r.line(), false),
        Err(e) => flash(&s.sid, e.message(), true),
    }
    to_org(org, "/policy/drafts")
}

/// The active policy (or the newest draft), rendered as policy repo files with names for ids.
fn starting_files(
    c: &mut crate::db::DbConnInner,
    org: &str,
    from: Option<i64>,
) -> Result<Vec<(String, String)>, OrgError> {
    let row = match from {
        Some(rev) => reins_org_keys::q_policy(c, org, rev)?,
        None => reins_org_keys::q_active_policy(c, org)?,
    };
    let (mut p, plan) = if let Some(r) = row {
        let p: OrgPolicy = serde_json::from_str(&r.body).map_err(|e| OrgError::Internal(e.to_string()))?;
        let plan: RolloutPlanSource =
            r.rollout_plan.as_deref().and_then(|t| serde_json::from_str(t).ok()).unwrap_or_default();
        (p, plan)
    } else {
        let mut p: OrgPolicy =
            serde_json::from_str(policy::DEVELOPER_BASELINE).map_err(|e| OrgError::Internal(e.to_string()))?;
        org.clone_into(&mut p.org);
        (p, RolloutPlanSource::default())
    };
    policy_repo::names_of(c, org, &mut p)?;
    // The org's slug in the files: the id is put back when the draft is saved.
    org.clone_into(&mut p.org);
    for r in &mut p.rules {
        r.rule.issuer = reins_policy::matrix::Issuer::Org(p.org.clone());
    }
    Ok(source::render(&p, &plan).into_iter().collect())
}

#[get("/reins/org/<org>/policy/drafts/new?<from>")]
async fn new_draft(org: &str, from: Option<i64>, cookies: &CookieJar<'_>, conn: DbConn) -> Page {
    let Some(s) = admin_of(cookies, &conn, org, &[Capability::AuthorPolicy]).await else {
        return not_found();
    };
    let o = org.to_owned();
    let (Ok(files), Ok(name)) = (run(&conn, move |c| starting_files(c, &o, from)).await, org_name(&conn, org).await)
    else {
        return not_found();
    };
    let base = console::base();
    Page(pages::new_draft(&frame!(s, org, name, base), &files, &[]))
}

#[derive(FromForm)]
struct NewDraftForm {
    csrf: String,
    files: HashMap<String, String>,
    new_path: Option<String>,
    new_text: Option<String>,
}

#[post("/reins/org/<org>/policy/drafts/new", data = "<form>")]
async fn save_new_draft(
    org: &str,
    form: Form<NewDraftForm>,
    cookies: &CookieJar<'_>,
    conn: DbConn,
) -> Result<Redirect, Page> {
    let Some(s) = admin_of(cookies, &conn, org, &[Capability::AuthorPolicy]).await else {
        return Ok(to_home());
    };
    if !check_csrf(&s, &form.csrf) {
        return Ok(to_org(org, "/policy/drafts"));
    }
    let mut files: BTreeMap<String, String> = form
        .files
        .iter()
        .filter(|(_, t)| !t.trim().is_empty())
        .map(|(p, t)| (p.clone(), t.replace("\r\n", "\n")))
        .collect();
    if let (Some(p), Some(t)) = (form.new_path.as_deref(), form.new_text.as_deref())
        && !p.trim().is_empty()
        && !t.trim().is_empty()
    {
        files.insert(p.trim().to_owned(), t.replace("\r\n", "\n"));
    }
    let (o, u, now) = (org.to_owned(), s.user.clone(), now_unix());
    let kept = files.clone();
    let saved = run(&conn, move |c| -> Result<Result<u64, Vec<SourceError>>, OrgError> {
        if let Err(m) = source::check_size(&files) {
            return Ok(Err(vec![SourceError {
                file: source::MAIN_FILE.to_owned(),
                line: 0,
                message: m,
            }]));
        }
        let mut compiled = match source::compile(&files) {
            Ok(c) => c,
            Err(e) => return Ok(Err(e)),
        };
        if let Err(e) = policy_repo::resolve_names(c, &o, &mut compiled) {
            return Ok(Err(e));
        }
        let body = serde_json::to_string(&compiled.policy).map_err(|e| OrgError::Internal(e.to_string()))?;
        let (rev, _) = policy::save_draft_with(c, &o, &u, &body, Some("console"), Some(&compiled.plan), now)?;
        Ok(Ok(rev))
    })
    .await;
    match saved {
        Ok(Ok(rev)) => {
            flash(&s.sid, &format!("Saved as revision {rev}. Simulate it next."), false);
            Ok(to_org(org, "/policy/drafts"))
        }
        Ok(Err(errors)) => {
            let Ok(name) = org_name(&conn, org).await else {
                return Err(not_found());
            };
            let base = console::base();
            let files: Vec<(String, String)> = kept.into_iter().collect();
            Err(Page(pages::new_draft(&frame!(s, org, name, base), &files, &errors)))
        }
        Err(e) => {
            flash(&s.sid, e.message(), true);
            Ok(to_org(org, "/policy/drafts"))
        }
    }
}

#[get("/reins/org/<org>/policy/simulations/<report>")]
async fn simulation_page(org: &str, report: &str, cookies: &CookieJar<'_>, conn: DbConn) -> Page {
    let Some(s) = admin_of(cookies, &conn, org, VIEW).await else {
        return not_found();
    };
    let (o, h) = (org.to_owned(), report.to_owned());
    let (Ok(Some((r, row))), Ok(name)) =
        (run(&conn, move |c| simulation::report(c, &o, &h)).await, org_name(&conn, org).await)
    else {
        return not_found();
    };
    let base = console::base();
    Page(pages::simulation(&frame!(s, org, name, base), report, row.revision, &when(row.ran_at), &r))
}

#[get("/reins/org/<org>/policy/samples/<request>")]
async fn sample(org: &str, request: &str, cookies: &CookieJar<'_>, conn: DbConn) -> Page {
    let Some(s) = admin_of(cookies, &conn, org, &[Capability::ViewAdminEvents]).await else {
        return not_found();
    };
    let (o, u, r, now) = (org.to_owned(), s.user.clone(), request.to_owned(), now_unix());
    // The read is recorded before anything is shown (and only Auditors and Security admins may).
    let (Ok(case), Ok(name)) =
        (run(&conn, move |c| simulation::sample(c, &o, &u, &r, now)).await, org_name(&conn, org).await)
    else {
        return not_found();
    };
    let base = console::base();
    let json = serde_json::to_string_pretty(&case).unwrap_or_default();
    Page(pages::sample(&frame!(s, org, name, base), request, &json))
}

#[get("/reins/org/<org>/policy/history?<days>")]
async fn history(
    org: &str,
    days: Option<i64>,
    cookies: &CookieJar<'_>,
    conn: DbConn,
) -> Result<console_jsonl::Jsonl, Page> {
    let Some(s) = admin_of(cookies, &conn, org, &[Capability::ViewAdminEvents]).await else {
        return Err(not_found());
    };
    let now = now_unix();
    let since = now - days.unwrap_or(simulation::DEFAULT_DAYS).clamp(1, simulation::MAX_DAYS) * 86_400;
    let (o, u) = (org.to_owned(), s.user.clone());
    let body = run(&conn, move |c| simulation::export(c, &o, &u, since, now, now)).await.map_err(|_| not_found())?;
    Ok(console_jsonl::Jsonl {
        name: format!("reins-history-{}-{now}.jsonl", org.get(..8).unwrap_or(org)),
        body,
    })
}

/// The JSON Lines download (as the audit export's).
mod console_jsonl {
    use rocket::{
        Request, Response,
        http::ContentType,
        response::{self, Responder},
    };

    pub struct Jsonl {
        pub name: String,
        pub body: String,
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
}

#[get("/reins/org/<org>/policy/rollout")]
async fn rollout_page(org: &str, cookies: &CookieJar<'_>, conn: DbConn) -> Page {
    let Some(s) = admin_of(cookies, &conn, org, VIEW).await else {
        return not_found();
    };
    let (o, u, now) = (org.to_owned(), s.user.clone(), now_unix());
    let loaded = run(&conn, move |c| {
        let active = reins_org_keys::q_active_policy(c, &o)?.map(|r| (r.revision, r.hash));
        let row = code::q_recent_rollouts(c, &o, now - rollout::RECENT_SECS)?.into_iter().next();
        let view = match row {
            Some(r) => {
                let plan = rollout::plan(&r)?;
                let (cand, stable) = rollout::numbers(c, &r)?;
                let cohort = |x: rollout::Cohort| pages::CohortView {
                    decisions: x.decisions,
                    ask_rate: x.ask_rate,
                    median_ttd: x.median_ttd,
                };
                Some(pages::RolloutView {
                    id: r.rollout_id.clone(),
                    state: r.state.clone(),
                    revision: r.revision,
                    stable: r.stable.clone(),
                    candidate: r.candidate.clone(),
                    stages: plan.stages.iter().map(Stage::label).collect(),
                    published: usize::try_from(r.published_seq).unwrap_or(0),
                    since: when(r.stage_started_at),
                    candidate_numbers: cohort(cand),
                    stable_numbers: cohort(stable),
                    min_decisions: plan.guards.min_decisions,
                    reason: r.reason.clone(),
                    can_control: can(c, &o, &u, Capability::ActivatePolicy)?,
                })
            }
            None => None,
        };
        Ok((active, view))
    })
    .await;
    let (Ok((active, view)), Ok(name)) = (loaded, org_name(&conn, org).await) else {
        return not_found();
    };
    let base = console::base();
    Page(pages::rollout(&frame!(s, org, name, base), active.as_ref().map(|(r, h)| (*r, h.as_str())), view.as_ref()))
}

#[post("/reins/org/<org>/rollouts/<id>/<what>", data = "<form>")]
async fn control(
    org: &str,
    id: &str,
    what: &str,
    form: Form<CsrfForm>,
    cookies: &CookieJar<'_>,
    conn: DbConn,
) -> Redirect {
    let Some(s) = admin_of(cookies, &conn, org, &[Capability::ActivatePolicy]).await else {
        return to_home();
    };
    let Some(what) = Control::parse(what) else {
        return to_org(org, "/policy/rollout");
    };
    if !check_csrf(&s, &form.csrf) {
        return to_org(org, "/policy/rollout");
    }
    let (o, user, rollout_id, now) = (org.to_owned(), s.user.clone(), id.to_owned(), now_unix());
    match run(&conn, move |c| rollout::control(c, &o, &user, &rollout_id, what, now)).await {
        Ok(()) => flash(&s.sid, "Done.", false),
        Err(e) => flash(&s.sid, e.message(), true),
    }
    to_org(org, "/policy/rollout")
}

#[get("/reins/org/<org>/policy/repo")]
async fn repo_page(org: &str, cookies: &CookieJar<'_>, conn: DbConn) -> Page {
    let Some(s) = admin_of(cookies, &conn, org, REPO_VIEW).await else {
        return not_found();
    };
    let (o, u) = (org.to_owned(), s.user.clone());
    let loaded = run(&conn, move |c| {
        let reader = reins_org_github_reader::q_github_reader(c, &o)?
            .map(|r| (r.app_id, r.installation_id, r.sealed.is_some() && r.webhook_sealed.is_some()));
        let repo = code::q_policy_repo(c, &o)?.map(|r| (r.repo, r.default_branch));
        Ok(pages::RepoView {
            reader,
            repo,
            can_setup: can(c, &o, &u, Capability::SetUpOrg)?,
            can_connect: can(c, &o, &u, Capability::ManageConnections)?,
        })
    })
    .await;
    let (Ok(view), Ok(name)) = (loaded, org_name(&conn, org).await) else {
        return not_found();
    };
    let base = console::base();
    Page(pages::repo(&frame!(s, org, name, base), &view))
}

#[derive(FromForm)]
struct ReaderForm {
    csrf: String,
    app_id: u64,
    installation_id: u64,
}

#[post("/reins/org/<org>/policy/reader", data = "<form>")]
async fn set_reader(
    org: &str,
    form: Form<ReaderForm>,
    cookies: &CookieJar<'_>,
    conn: DbConn,
    pool: &State<DbPool>,
) -> Redirect {
    let Some(s) = admin_of(cookies, &conn, org, &[Capability::ManageConnections]).await else {
        return to_home();
    };
    if !check_csrf(&s, &form.csrf) {
        return to_org(org, "/policy/repo");
    }
    let action = AdminAction::SetGithubReader {
        app_id: form.app_id,
        installation_id: form.installation_id,
    };
    let r = propose(&conn, pool, org, &s.user, action, None).await;
    let (text, bad) = outcome_text(&r, "Set. Now its key and webhook secret.");
    flash(&s.sid, &text, bad);
    to_org(org, "/policy/repo")
}

#[derive(FromForm)]
struct SecretsForm {
    csrf: String,
    pem: String,
    webhook_secret: String,
}

#[post("/reins/org/<org>/policy/reader/secrets", data = "<form>")]
async fn set_reader_secrets(org: &str, form: Form<SecretsForm>, cookies: &CookieJar<'_>, conn: DbConn) -> Redirect {
    let Some(s) = admin_of(cookies, &conn, org, &[Capability::ManageConnections]).await else {
        return to_home();
    };
    if !check_csrf(&s, &form.csrf) {
        return to_org(org, "/policy/repo");
    }
    let (o, user, pem, secret, now) =
        (org.to_owned(), s.user.clone(), form.pem.clone(), form.webhook_secret.clone(), now_unix());
    let r = run(&conn, move |c| {
        github_reader::set_secrets(c, &o, &pem, &secret, now)?;
        super::audit::record(
            c,
            &o,
            super::audit::kinds::GITHUB_READER_SET,
            Some(&user),
            None,
            &serde_json::json!({"secrets": "replaced"}),
            now,
        )
        .map(drop)
    })
    .await;
    match r {
        Ok(()) => flash(&s.sid, "Saved.", false),
        Err(e) => flash(&s.sid, e.message(), true),
    }
    to_org(org, "/policy/repo")
}

#[derive(FromForm)]
struct RepoForm {
    csrf: String,
    repo: String,
}

#[post("/reins/org/<org>/policy/repo", data = "<form>")]
async fn bind_repo(
    org: &str,
    form: Form<RepoForm>,
    cookies: &CookieJar<'_>,
    conn: DbConn,
    pool: &State<DbPool>,
) -> Redirect {
    let Some(s) = admin_of(cookies, &conn, org, &[Capability::SetUpOrg]).await else {
        return to_home();
    };
    if !check_csrf(&s, &form.csrf) {
        return to_org(org, "/policy/repo");
    }
    let repo = form.repo.trim().to_ascii_lowercase();
    let o = org.to_owned();
    // The reader must reach the repo: its default branch is what's approved.
    let branch = match run(&conn, move |c| github_reader::for_org(c, &o)).await {
        Ok(Some(gh)) if repo.split('/').count() == 2 => gh
            .get_json(&format!("/repos/{repo}"))
            .await
            .ok()
            .and_then(|v| v["default_branch"].as_str().map(str::to_owned)),
        _ => None,
    };
    let Some(default_branch) = branch else {
        flash(&s.sid, "The GitHub reader can't reach that repo.", true);
        return to_org(org, "/policy/repo");
    };
    let r = propose(
        &conn,
        pool,
        org,
        &s.user,
        AdminAction::BindPolicyRepo {
            repo,
            default_branch,
        },
        None,
    )
    .await;
    let (text, bad) = outcome_text(&r, "Bound.");
    flash(&s.sid, &text, bad);
    to_org(org, "/policy/repo")
}

#[post("/reins/org/<org>/policy/repo/unbind", data = "<form>")]
async fn unbind_repo(
    org: &str,
    form: Form<CsrfForm>,
    cookies: &CookieJar<'_>,
    conn: DbConn,
    pool: &State<DbPool>,
) -> Redirect {
    let Some(s) = admin_of(cookies, &conn, org, &[Capability::SetUpOrg]).await else {
        return to_home();
    };
    if !check_csrf(&s, &form.csrf) {
        return to_org(org, "/policy/repo");
    }
    let action = AdminAction::BindPolicyRepo {
        repo: String::new(),
        default_branch: String::new(),
    };
    let r = propose(&conn, pool, org, &s.user, action, None).await;
    let (text, bad) = outcome_text(&r, "Unbound.");
    flash(&s.sid, &text, bad);
    to_org(org, "/policy/repo")
}

/// The rollout summary O2's policy page shows: the stages, the one published, and the state.
pub fn summary(c: &mut crate::db::DbConnInner, org: &str, now: i64) -> Result<Option<String>, OrgError> {
    let Some(r) = code::q_recent_rollouts(c, org, now - rollout::RECENT_SECS)?.into_iter().next() else {
        return Ok(None);
    };
    let plan = rollout::plan(&r)?;
    let stage =
        plan.stages.get(usize::try_from(r.published_seq - 1).unwrap_or(0)).map(Stage::label).unwrap_or_default();
    let words = match r.state.as_str() {
        code::rollout_state::RUNNING => format!("Rolling out revision {}: {stage}", r.revision),
        code::rollout_state::PAUSED => format!("Paused at {stage}, revision {}", r.revision),
        code::rollout_state::ROLLED_BACK => format!("Revision {} rolled back", r.revision),
        _ => format!("Revision {} rolled out", r.revision),
    };
    Ok(Some(words))
}
