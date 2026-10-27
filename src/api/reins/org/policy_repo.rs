//! The org's policy repo (`docs/design/decisions-v2/05-organizations.md` §7.3, `02` §7): policy as code on GitHub.
//!
//! - **Binding** is a two-person admin change (`AdminAction::BindPolicyRepo`); the org's GitHub reader must reach the
//!   repo, and its default branch is recorded.
//! - **`POST /reins/org/github/policy-webhook`** takes the reader App's webhooks. Each is verified
//!   (`X-Hub-Signature-256`, HMAC-SHA256 with the org's sealed webhook secret, in constant time), must come from the
//!   org's installation and its bound repo, and is acted on once (`X-GitHub-Delivery`, a day).
//! - **A PR** to the default branch: the repo at the PR's head is read (≤ 200 policy files, ≤ 1 MiB), compiled, saved
//!   as a draft and simulated; the `reins/policy` check run carries the compile errors (annotated by file and line) or
//!   the simulation line.
//! - **A push** to the default branch (a merge): the same, then an `ActivatePolicy` is *proposed* by the system. It
//!   applies nothing: a first Security admin confirms it on their phone, and a second approves it.
//!
//! The repo is input, never authority.

use std::collections::BTreeMap;

use diesel::prelude::*;
use reins_policy::{
    org::OrgPolicy,
    simulate::SimulationReport,
    source::{self, Compiled, SourceError},
};
use reins_proto::org::{AdminAction, Capability};
use rocket::{Data, Request, Route, State, http::Status, request::FromRequest, request::Outcome};
use serde_json::{Value, json};

use super::{
    OrgError, admin, audit, enabled,
    github_reader::{self, GithubReader},
    policy, reviews,
    simulation::{self, SYSTEM_ACTOR},
};
use crate::{
    CONFIG,
    api::reins::{device_api::read_body_limited, now_unix},
    db::{
        DbConn, DbConnInner, DbPool,
        models::{
            reins_org,
            reins_org_policy_code::{self as code, ReinsOrgPolicyRepo},
        },
        schema::users,
    },
};

/// The check run's name.
pub const CHECK_NAME: &str = "reins/policy";
/// Largest webhook body taken.
const MAX_WEBHOOK: u64 = 4 * 1024 * 1024;
/// GitHub takes at most 50 annotations per request.
const MAX_ANNOTATIONS: usize = 50;

pub fn routes() -> Vec<Route> {
    routes![policy_webhook]
}

fn repo_ok(repo: &str) -> bool {
    let Some((owner, name)) = repo.split_once('/') else {
        return false;
    };
    let part = |s: &str, max: usize| {
        !s.is_empty()
            && s.len() <= max
            && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    };
    part(owner, 39) && part(name, 100) && !name.starts_with('.')
}

fn branch_ok(b: &str) -> bool {
    !b.is_empty() && b.len() <= 255 && !b.contains("..") && !b.bytes().any(|x| x.is_ascii_control() || x == b' ')
}

/// Whether `org` may bind `repo` (lowercased) with `default_branch`; an empty `repo` unbinds (a binding must exist).
pub fn check_binding(c: &mut DbConnInner, org: &str, repo: &str, default_branch: &str) -> Result<(), OrgError> {
    if repo.is_empty() {
        return code::q_policy_repo(c, org)?.map(drop).ok_or(OrgError::NotFound);
    }
    let repo = repo.to_ascii_lowercase();
    if !repo_ok(&repo) {
        return Err(OrgError::Invalid("Name the repo as owner/name."));
    }
    if !branch_ok(default_branch) {
        return Err(OrgError::Invalid("The repo's default branch couldn't be read."));
    }
    if github_reader::for_org(c, org)?.is_none() {
        return Err(OrgError::Invalid("Set up the GitHub reader first."));
    }
    if code::q_policy_repo_by_name(c, &repo)?.is_some_and(|r| r.org_uuid != org) {
        return Err(OrgError::Taken("That repo is another organization's policy repo."));
    }
    Ok(())
}

/// Applies an approved binding (or unbinding).
pub fn apply_binding(
    c: &mut DbConnInner,
    org: &str,
    repo: &str,
    default_branch: &str,
    actor: &str,
    now: i64,
) -> Result<(), OrgError> {
    check_binding(c, org, repo, default_branch)?;
    if repo.is_empty() {
        code::q_unbind_policy_repo(c, org)?;
    } else if !code::q_bind_policy_repo(
        c,
        &ReinsOrgPolicyRepo {
            org_uuid: org.to_owned(),
            repo: repo.to_ascii_lowercase(),
            default_branch: default_branch.to_owned(),
            bound_by: actor.to_owned(),
            bound_at: now,
        },
    )? {
        return Err(OrgError::Taken("That repo is another organization's policy repo."));
    }
    audit::record(
        c,
        org,
        audit::kinds::POLICY_REPO_BOUND,
        Some(actor),
        None,
        &json!({"repo": repo.to_ascii_lowercase(), "default_branch": default_branch}),
        now,
    )
    .map(drop)
}

/// Whether `sig` (`sha256=<hex>`) is the HMAC-SHA256 of `body` under `secret`, checked in constant time.
#[must_use]
pub fn signature_ok(secret: &[u8], body: &[u8], sig: &str) -> bool {
    let Some(hex) = sig.trim().strip_prefix("sha256=") else {
        return false;
    };
    let Ok(tag) = data_encoding::HEXLOWER_PERMISSIVE.decode(hex.as_bytes()) else {
        return false;
    };
    let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, secret);
    ring::hmac::verify(&key, body, &tag).is_ok()
}

/// The webhook headers.
pub struct HookHeaders {
    event: Option<String>,
    delivery: Option<String>,
    signature: Option<String>,
}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for HookHeaders {
    type Error = ();

    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        let h = |n: &str| request.headers().get_one(n).map(str::to_owned);
        Outcome::Success(Self {
            event: h("X-GitHub-Event"),
            delivery: h("X-GitHub-Delivery"),
            signature: h("X-Hub-Signature-256"),
        })
    }
}

/// What a verified webhook asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Work {
    /// A PR's head to check: (number, head sha).
    PullRequest {
        number: u64,
        sha: String,
    },
    /// The default branch moved: (branch, sha).
    Push {
        branch: String,
        sha: String,
    },
    /// Nothing to do (another event, branch or action).
    Ignore,
}

/// Which org a webhook is for: its installation's org, and only for that org's bound repo.
pub fn route(c: &mut DbConnInner, payload: &Value) -> Result<Option<(String, ReinsOrgPolicyRepo)>, OrgError> {
    let Some(installation) = payload["installation"]["id"].as_i64() else {
        return Ok(None);
    };
    let Some(reader) = code::q_github_reader_by_installation(c, installation)? else {
        return Ok(None);
    };
    let Some(bound) = code::q_policy_repo(c, &reader.org_uuid)? else {
        return Ok(None);
    };
    let repo = payload["repository"]["full_name"].as_str().unwrap_or_default().to_ascii_lowercase();
    if repo != bound.repo {
        return Ok(None);
    }
    Ok(Some((reader.org_uuid, bound)))
}

/// What a webhook event asks for, given the bound repo.
#[must_use]
pub fn work_of(event: &str, payload: &Value, bound: &ReinsOrgPolicyRepo) -> Work {
    match event {
        "pull_request" => {
            let action = payload["action"].as_str().unwrap_or_default();
            let pr = &payload["pull_request"];
            let base = pr["base"]["ref"].as_str().unwrap_or_default();
            let sha = pr["head"]["sha"].as_str().unwrap_or_default();
            let number = payload["number"].as_u64().or_else(|| pr["number"].as_u64()).unwrap_or(0);
            let interesting = matches!(action, "opened" | "synchronize" | "reopened" | "ready_for_review");
            if interesting && base == bound.default_branch && sha_ok(sha) && number > 0 {
                Work::PullRequest {
                    number,
                    sha: sha.to_owned(),
                }
            } else {
                Work::Ignore
            }
        }
        "push" => {
            let r = payload["ref"].as_str().unwrap_or_default();
            let after = payload["after"].as_str().unwrap_or_default();
            let deleted = payload["deleted"].as_bool().unwrap_or(false);
            if r == format!("refs/heads/{}", bound.default_branch) && sha_ok(after) && !deleted {
                Work::Push {
                    branch: bound.default_branch.clone(),
                    sha: after.to_owned(),
                }
            } else {
                Work::Ignore
            }
        }
        _ => Work::Ignore,
    }
}

fn sha_ok(s: &str) -> bool {
    s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit()) && s.bytes().any(|b| b != b'0')
}

/// Reads the repo's policy files at `sha` (the tree, then each blob), within the caps.
pub async fn read_tree(gh: &GithubReader, repo: &str, sha: &str) -> Result<BTreeMap<String, String>, String> {
    let tree = gh.get_json(&format!("/repos/{repo}/git/trees/{sha}?recursive=1")).await?;
    if tree["truncated"].as_bool().unwrap_or(false) {
        return Err("the repo is too large to read".to_owned());
    }
    let mut wanted = Vec::new();
    let mut bytes = 0usize;
    for e in tree["tree"].as_array().into_iter().flatten() {
        let path = e["path"].as_str().unwrap_or_default();
        if e["type"].as_str() != Some("blob") || !source::is_policy_file(path) {
            continue;
        }
        bytes += usize::try_from(e["size"].as_u64().unwrap_or(0)).unwrap_or(usize::MAX);
        wanted.push((path.to_owned(), e["sha"].as_str().unwrap_or_default().to_owned()));
    }
    if wanted.len() > source::MAX_FILES {
        return Err(format!("more than {} policy files", source::MAX_FILES));
    }
    if bytes > source::MAX_BYTES {
        return Err("policy files over 1 MiB".to_owned());
    }
    let mut files = BTreeMap::new();
    for (path, blob) in wanted {
        if !blob.bytes().all(|b| b.is_ascii_hexdigit()) || blob.is_empty() {
            return Err(format!("{path}: unreadable"));
        }
        let b = gh.get_json(&format!("/repos/{repo}/git/blobs/{blob}")).await?;
        let content: String =
            b["content"].as_str().unwrap_or_default().chars().filter(|c| !c.is_whitespace()).collect();
        let raw = data_encoding::BASE64.decode(content.as_bytes()).map_err(|_| format!("{path}: unreadable"))?;
        files.insert(path.clone(), String::from_utf8(raw).map_err(|_| format!("{path}: not UTF-8"))?);
    }
    source::check_size(&files)?;
    Ok(files)
}

/// The compiled policy with group names and people's emails mapped to the org's ids; errors as source errors.
pub fn resolve_names(c: &mut DbConnInner, org: &str, compiled: &mut Compiled) -> Result<(), Vec<SourceError>> {
    let groups: BTreeMap<String, String> = reins_org::q_groups(c, org)
        .unwrap_or_default()
        .into_iter()
        .map(|(id, name, _)| (name.to_ascii_lowercase(), id))
        .collect();
    let people = member_emails(c, org).unwrap_or_default();
    let mapped = source::map_reviewers(
        &mut compiled.policy,
        |g| groups.get(&g.to_ascii_lowercase()).cloned().ok_or_else(|| format!("there is no group {g:?}")),
        |m| people.get(&m.to_ascii_lowercase()).cloned().ok_or_else(|| format!("{m} isn't a member")),
    );
    let mut errs = Vec::new();
    if let Err(message) = mapped {
        errs.push(SourceError {
            file: source::RULES_DIR.trim_end_matches('/').to_owned(),
            line: 1,
            message,
        });
    }
    if let Some(g) = &compiled.plan.pilot_group
        && !groups.contains_key(&g.to_ascii_lowercase())
    {
        errs.push(SourceError {
            file: source::MAIN_FILE.to_owned(),
            line: 1,
            message: format!("the pilot group {g:?} isn't in this organization"),
        });
    }
    if errs.is_empty() {
        Ok(())
    } else {
        Err(errs)
    }
}

/// Active members' emails (lowercase) to user ids.
pub fn member_emails(c: &mut DbConnInner, org: &str) -> Result<BTreeMap<String, String>, OrgError> {
    let mut out = BTreeMap::new();
    for m in reins_org::q_members(c, org)? {
        let user = m.user_uuid.to_string();
        if super::member_of(c, org, &user)?.is_none() {
            continue;
        }
        if let Some(email) =
            users::table.filter(users::uuid.eq(&user)).select(users::email).first::<String>(c).optional()?
        {
            out.insert(email.to_ascii_lowercase(), user);
        }
    }
    Ok(out)
}

/// The org's ids mapped back to names and emails, so a policy renders as people wrote it.
pub fn names_of(c: &mut DbConnInner, org: &str, policy: &mut OrgPolicy) -> Result<(), OrgError> {
    let groups: BTreeMap<String, String> =
        reins_org::q_groups(c, org)?.into_iter().map(|(id, name, _)| (id, name)).collect();
    let people: BTreeMap<String, String> = member_emails(c, org)?.into_iter().map(|(e, u)| (u, e)).collect();
    source::map_reviewers(
        policy,
        |g| Ok(groups.get(g).cloned().unwrap_or_else(|| g.to_owned())),
        |m| Ok(people.get(m).cloned().unwrap_or_else(|| m.to_owned())),
    )
    .map_err(|e| OrgError::Internal(e))
}

/// What checking one commit came to.
#[derive(Debug)]
pub enum Checked {
    Errors(Vec<SourceError>),
    /// Saved as `revision` (`hash`) and simulated.
    Simulated {
        revision: u64,
        hash: String,
        report: SimulationReport,
        report_hash: String,
    },
}

/// Compiles, saves and simulates `files` for `org` from `source_ref`.
pub fn check_files(
    c: &mut DbConnInner,
    org: &str,
    files: &BTreeMap<String, String>,
    source_ref: &str,
    now: i64,
) -> Result<Checked, OrgError> {
    let mut compiled = match source::compile(files) {
        Ok(c) => c,
        Err(errs) => return Ok(Checked::Errors(errs)),
    };
    if let Err(errs) = resolve_names(c, org, &mut compiled) {
        return Ok(Checked::Errors(errs));
    }
    let body = serde_json::to_string(&compiled.policy).map_err(|e| OrgError::Internal(e.to_string()))?;
    let (revision, hash) =
        match policy::save_draft_with(c, org, SYSTEM_ACTOR, &body, Some(source_ref), Some(&compiled.plan), now) {
            Ok(saved) => saved,
            Err(OrgError::Invalid(m)) => {
                return Ok(Checked::Errors(vec![SourceError {
                    file: source::MAIN_FILE.to_owned(),
                    line: 1,
                    message: m.to_owned(),
                }]));
            }
            Err(e) => return Err(e),
        };
    let report = simulation::run(c, org, SYSTEM_ACTOR, revision, None, None, Some(source_ref), now)?;
    let report_hash = report.hash();
    Ok(Checked::Simulated {
        revision,
        hash,
        report,
        report_hash,
    })
}

/// The check run for a checked commit.
#[must_use]
pub fn check_run(org: &str, sha: &str, checked: &Checked) -> Value {
    match checked {
        Checked::Errors(errs) => {
            let annotations: Vec<Value> = errs
                .iter()
                .take(MAX_ANNOTATIONS)
                .map(|e| {
                    let line = e.line.max(1);
                    json!({"path": e.file, "start_line": line, "end_line": line, "annotation_level": "failure",
                           "message": e.message})
                })
                .collect();
            let summary: Vec<String> = errs.iter().map(ToString::to_string).collect();
            json!({"name": CHECK_NAME, "head_sha": sha, "status": "completed", "conclusion": "failure",
                   "output": {"title": format!("{} error{}", errs.len(), if errs.len() == 1 { "" } else { "s" }),
                              "summary": summary.join("\n"), "annotations": annotations}})
        }
        Checked::Simulated {
            revision,
            report,
            report_hash,
            ..
        } => {
            let link =
                format!("{}/reins/org/{org}/policy/simulations/{report_hash}", CONFIG.domain().trim_end_matches('/'));
            let tops = |v: &[(String, u32)]| v.iter().map(|(k, n)| format!("{k} {n}")).collect::<Vec<_>>().join(", ");
            let mut summary = format!(
                "{}\n\n- Cases: {} (last {} days), skipped {}\n- Auto → Ask {}, Ask → Auto {}, → Deny {}, Deny → {}\n- Drift: {}\n",
                report.line(),
                report.cases,
                (report.until - report.since).max(0) / 86_400,
                report.skipped,
                report.auto_to_review,
                report.review_to_auto,
                report.to_deny,
                report.from_deny,
                report.drift
            );
            if !report.top_groups.is_empty() {
                summary.push_str(&format!("- Top groups: {}\n", tops(&report.top_groups)));
            }
            if !report.top_rules.is_empty() {
                summary.push_str(&format!("- Top rules: {}\n", tops(&report.top_rules)));
            }
            summary.push_str(&format!("\nDraft revision {revision}. [Report]({link})"));
            json!({"name": CHECK_NAME, "head_sha": sha, "status": "completed", "conclusion": "neutral",
                   "details_url": link,
                   "output": {"title": report.line(), "summary": summary}})
        }
    }
}

/// Handles a verified webhook's work; on a push, returns the pending change and whom to tell.
pub async fn handle(
    pool: &DbPool,
    conn: &DbConn,
    org: String,
    bound: ReinsOrgPolicyRepo,
    work: Work,
) -> Result<(), OrgError> {
    let (sha, source_ref, is_push) = match &work {
        Work::PullRequest {
            number,
            sha,
        } => (sha.clone(), format!("pr#{number}@{sha}"), false),
        Work::Push {
            branch,
            sha,
        } => (sha.clone(), format!("{branch}@{sha}"), true),
        Work::Ignore => return Ok(()),
    };
    let o = org.clone();
    let Some(gh) = conn.run(move |c| github_reader::for_org(c, &o)).await? else {
        return Err(OrgError::Invalid("The GitHub reader isn't set up."));
    };
    let read = read_tree(&gh, &bound.repo, &sha).await;
    let (o, r) = (org.clone(), source_ref.clone());
    let checked = conn
        .run(move |c| -> Result<Checked, OrgError> {
            let now = now_unix();
            let checked = match read {
                Ok(files) => check_files(c, &o, &files, &r, now)?,
                Err(m) => Checked::Errors(vec![SourceError {
                    file: source::MAIN_FILE.to_owned(),
                    line: 0,
                    message: m,
                }]),
            };
            let detail = match &checked {
                Checked::Errors(e) => json!({"source": r, "errors": e.len()}),
                Checked::Simulated {
                    revision,
                    report_hash,
                    ..
                } => json!({"source": r, "revision": revision, "report": report_hash}),
            };
            audit::record(c, &o, audit::kinds::POLICY_REPO_CHECKED, Some(SYSTEM_ACTOR), None, &detail, now)?;
            Ok(checked)
        })
        .await?;
    if !is_push {
        let body = check_run(&org, &sha, &checked);
        if let Err(e) = gh.post_json(&format!("/repos/{}/check-runs", bound.repo), &body).await {
            warn!("Org {org}: check run for {source_ref}: {e}");
        }
        return Ok(());
    }
    let Checked::Simulated {
        revision,
        hash,
        report_hash,
        ..
    } = checked
    else {
        return Ok(());
    };
    let action = AdminAction::ActivatePolicy {
        revision,
        hash,
        report: Some(report_hash),
    };
    let o = org.clone();
    let (change, notify) = conn
        .run(move |c| -> Result<(String, Vec<String>), OrgError> {
            let id = admin::propose_as_system(c, &o, &action, now_unix())?;
            Ok((id, reins_org::q_holders(c, &o, Capability::ActivatePolicy)?))
        })
        .await?;
    reviews::notify(pool, &notify, &change);
    Ok(())
}

#[post("/reins/org/github/policy-webhook", data = "<data>")]
async fn policy_webhook(h: HookHeaders, data: Data<'_>, conn: DbConn, pool: &State<DbPool>) -> Status {
    if !enabled() {
        return Status::NotFound;
    }
    let Ok(body) = read_body_limited(data, MAX_WEBHOOK).await else {
        return Status::PayloadTooLarge;
    };
    let (Some(event), Some(delivery), Some(sig)) = (h.event, h.delivery, h.signature) else {
        return Status::Unauthorized;
    };
    if delivery.is_empty() || delivery.len() > 64 {
        return Status::BadRequest;
    }
    let Ok(payload) = serde_json::from_slice::<Value>(&body) else {
        return Status::BadRequest;
    };
    // Which org (from the installation) and its webhook secret; then the signature, before anything is acted on.
    let p = payload.clone();
    let routed = conn
        .run(move |c| -> Result<Option<(String, ReinsOrgPolicyRepo, Option<zeroize::Zeroizing<Vec<u8>>>)>, OrgError> {
            let Some((org, bound)) = route(c, &p)? else {
                return Ok(None);
            };
            let secret = github_reader::webhook_secret(c, &org)?;
            Ok(Some((org, bound, secret)))
        })
        .await;
    let (org, bound) = match routed {
        Ok(Some((org, bound, Some(secret)))) if signature_ok(&secret, &body, &sig) => (org, bound),
        Ok(Some(_)) => return Status::Unauthorized,
        Ok(None) => return Status::NotFound,
        Err(e) => {
            warn!("Policy webhook: {e:?}");
            return Status::InternalServerError;
        }
    };
    let first = conn.run(move |c| code::q_first_delivery(c, &delivery, now_unix())).await;
    match first {
        Ok(true) => {}
        Ok(false) => return Status::Ok,
        Err(e) => {
            warn!("Policy webhook: {e:?}");
            return Status::InternalServerError;
        }
    }
    let work = work_of(&event, &payload, &bound);
    if work == Work::Ignore {
        return Status::NoContent;
    }
    match handle(pool, &conn, org.clone(), bound, work).await {
        Ok(()) => Status::Accepted,
        Err(e) => {
            warn!("Policy webhook for {org}: {e:?}");
            Status::Accepted
        }
    }
}
