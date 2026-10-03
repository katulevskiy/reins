//! GitHub: workflows and their runs, variables and secrets, security alerts, and the user's own account.

#![allow(clippy::assigning_clones, reason = "items are filled field by field from JSON text")]

use std::fmt::Write as _;

use data_encoding::BASE64;
use reins_proto::connector::ConnectorCall;
use reqwest::Method;
use serde_json::{Map, Value, json};

use super::{GitHub, Options, Preview, owner_ok, parents, ref_ok, repo_arg, repo_ok};
use crate::connector::Item;
use crate::connector::calendar::{limit, segment};
use crate::{CoreError, text};

type Res<T> = Result<T, CoreError>;
type Query = Vec<(&'static str, String)>;

/// The most characters of a log, a dependency list or a raw answer handed over.
const LOG_CHARS: usize = 60_000;
/// The most characters of one file of a gist.
const GIST_FILE_CHARS: usize = 20_000;
/// The most of a log kept in memory while it downloads; the rest of the start is dropped.
const LOG_KEEP_BYTES: usize = 2 * 1024 * 1024;
/// The most text of the AI's own words shown in a preview.
const SHOWN_CHARS: usize = 1_500;
const ACCOUNT: &str = "account";
const ACCOUNT_LABEL: &str = "Your GitHub account";

/// Lists and reads. `None` when the operation is not one of this area's.
pub(super) async fn fetch(gh: &GitHub, token: &str, call: &ConnectorCall) -> Option<Result<Vec<Item>, CoreError>> {
    Some(match call.op.as_str() {
        "workflow_list" => workflow_list(gh, token, call).await,
        "workflow_get" => workflow_get(gh, token, call).await,
        "workflow_usage" => workflow_usage(gh, token, call).await,
        "run_list" => run_list(gh, token, call).await,
        "run_get" => run_get(gh, token, call).await,
        "run_attempts" => run_attempts(gh, token, call).await,
        "run_jobs" => run_jobs(gh, token, call).await,
        "job_get" => job_get(gh, token, call).await,
        "job_logs" => job_logs(gh, token, call).await,
        "run_logs" => run_logs(gh, token, call).await,
        "artifact_list" => artifact_list(gh, token, call).await,
        "artifact_download_url" => artifact_download_url(gh, token, call).await,
        "run_pending_deployments" => pending_deployments(gh, token, call).await,
        "variable_list" => variable_list(gh, token, call).await,
        "variable_get" => variable_get(gh, token, call).await,
        "secret_list" => secret_list(gh, token, call).await,
        "environment_list" => environment_list(gh, token, call).await,
        "environment_get" => environment_get(gh, token, call).await,
        "cache_list" => cache_list(gh, token, call).await,
        "dependabot_alert_list" => alert_list(gh, token, call, Alerts::Dependabot).await,
        "dependabot_alert_get" => alert_get(gh, token, call, Alerts::Dependabot).await,
        "code_scanning_alert_list" => alert_list(gh, token, call, Alerts::CodeScanning).await,
        "code_scanning_alert_get" => alert_get(gh, token, call, Alerts::CodeScanning).await,
        "secret_scanning_alert_list" => alert_list(gh, token, call, Alerts::SecretScanning).await,
        "secret_scanning_alert_get" => alert_get(gh, token, call, Alerts::SecretScanning).await,
        "security_advisory_list" => advisory_list(gh, token, call).await,
        "dependency_sbom" => dependency_sbom(gh, token, call).await,
        "user_me" => user(gh, token, Ok("/user".to_owned())).await,
        "user_get" => user(gh, token, name_arg(call, "username").map(|n| format!("/users/{n}"))).await,
        "org_get" => user(gh, token, name_arg(call, "org").map(|n| format!("/orgs/{n}"))).await,
        "org_list" => account_list(gh, token, call, Ok("/user/orgs".to_owned()), Listing::Login).await,
        "org_repos" => {
            account_list(gh, token, call, name_arg(call, "org").map(|n| format!("/orgs/{n}/repos")), Listing::Repo)
                .await
        }
        "org_members" => {
            account_list(gh, token, call, name_arg(call, "org").map(|n| format!("/orgs/{n}/members")), Listing::Login)
                .await
        }
        "org_teams" => {
            account_list(gh, token, call, name_arg(call, "org").map(|n| format!("/orgs/{n}/teams")), Listing::Team)
                .await
        }
        "starred_list" => starred_list(gh, token, call).await,
        "ssh_key_list" => account_list(gh, token, call, Ok("/user/keys".to_owned()), Listing::SshKey).await,
        "gpg_key_list" => account_list(gh, token, call, Ok("/user/gpg_keys".to_owned()), Listing::GpgKey).await,
        "rate_limit" => rate_limit(gh, token).await,
        "notification_list" => notification_list(gh, token, call).await,
        "gist_list" => gist_list(gh, token, call).await,
        "gist_get" => gist_get(gh, token, call).await,
        "request_read" => request_read(gh, token, call).await,
        _ => return None,
    })
}

/// What a write would do. `None` when the operation is not one of this area's.
pub(super) async fn preview(gh: &GitHub, token: &str, call: &ConnectorCall) -> Option<Result<Preview, CoreError>> {
    let plan = plan(gh, token, call, false).await?;
    Some(plan.map(|p| p.preview()))
}

/// Does the write. `None` when the operation is not one of this area's.
pub(super) async fn perform(gh: &GitHub, token: &str, call: &ConnectorCall) -> Option<Result<Value, CoreError>> {
    let plan = plan(gh, token, call, true).await?;
    match plan {
        Ok(p) => Some(p.run(gh, token).await),
        Err(e) => Some(Err(e)),
    }
}

// ---- Small helpers ------------------------------------------------------------------------------------------------

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or_default()
}

fn when(v: &Value) -> i64 {
    v.as_str().and_then(text::parse_when).unwrap_or(0)
}

/// The text of the AI or of the network for a preview line: one bounded piece, said when shortened.
fn shown(raw: &str) -> String {
    let cut = text::truncate_chars(raw, SHOWN_CHARS);
    if cut.len() < raw.len() {
        format!("{cut}\n(shortened)")
    } else {
        cut
    }
}

fn short(raw: &str, max: usize) -> String {
    let line = text::one_line(raw);
    if line.chars().count() > max {
        format!("{}...", text::truncate_chars(&line, max))
    } else {
        line
    }
}

fn size(bytes: i64) -> String {
    #[allow(clippy::cast_precision_loss, reason = "a display size")]
    let b = bytes as f64;
    if bytes >= 1 << 20 {
        format!("{:.1} MB", b / f64::from(1 << 20))
    } else if bytes >= 1 << 10 {
        format!("{:.1} KB", b / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

fn id_arg(call: &ConnectorCall, name: &str) -> Res<i64> {
    match call.int_arg(name) {
        Some(n) if n > 0 => Ok(n),
        _ => Err(CoreError::service(format!("`{name}` must be a positive number."))),
    }
}

/// A user, organisation or team login, checked.
fn name_arg(call: &ConnectorCall, name: &str) -> Res<String> {
    match call.str_arg(name) {
        Some(n) if owner_ok(n) => Ok(n.to_owned()),
        _ => Err(CoreError::service(format!("`{name}` is not a valid GitHub login."))),
    }
}

/// A workflow id or file name, checked and made safe for a path.
fn workflow_ok(w: &str) -> bool {
    let file = |f: &str| {
        matches!(f.rsplit_once('.'), Some((stem, "yml" | "yaml")) if !stem.is_empty())
            && !f.contains("..")
            && f.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    };
    !w.is_empty() && w.len() <= 100 && (w.bytes().all(|b| b.is_ascii_digit()) || file(w))
}

fn workflow_arg(call: &ConnectorCall) -> Res<String> {
    match call.str_arg("workflow") {
        Some(w) if workflow_ok(w) => Ok(segment(w)),
        _ => Err(CoreError::service("`workflow` must be a workflow id or a file name like ci.yml.")),
    }
}

/// The name of a variable or secret.
fn var_name(call: &ConnectorCall, fresh: bool) -> Res<String> {
    let n = call.str_arg("name").unwrap_or_default();
    let ok = !n.is_empty() && n.len() <= 100 && n.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
    if !ok {
        return Err(CoreError::service("`name` may only have letters, digits and underscores."));
    }
    if fresh && n.to_ascii_uppercase().starts_with("GITHUB_") {
        return Err(CoreError::service("Names starting with GITHUB_ are reserved by GitHub."));
    }
    Ok(n.to_owned())
}

/// The optional `environment` argument.
fn env_arg(call: &ConnectorCall) -> Option<String> {
    call.str_arg("environment").filter(|e| !e.is_empty()).map(str::to_owned)
}

fn env_required(call: &ConnectorCall) -> Res<String> {
    env_arg(call).ok_or_else(|| CoreError::service("`environment` is required."))
}

/// `/repos/{repo}` or `/repos/{repo}/environments/{env}`, then `/variables` or `/secrets`.
fn scoped(repo: &str, env: Option<&str>, kind: &str) -> String {
    match env {
        Some(e) => format!("/repos/{repo}/environments/{}/{kind}", segment(e)),
        None => format!("/repos/{repo}/actions/{kind}"),
    }
}

fn scope_label(repo: &str, env: Option<&str>) -> String {
    match env {
        Some(e) => format!("environment '{}' of {repo}", short(e, 80)),
        None => repo.to_owned(),
    }
}

fn repo_base(repo: &str) -> Item {
    Item {
        resource: repo.to_owned(),
        resource_label: repo.to_owned(),
        parents: parents(repo, None),
        ..Item::default()
    }
}

fn account_base() -> Item {
    Item {
        resource: ACCOUNT.to_owned(),
        resource_label: ACCOUNT_LABEL.to_owned(),
        ..Item::default()
    }
}

/// Adds the structured fields of an item, leaving out those GitHub left empty.
fn with(mut item: Item, extra: &Value) -> Item {
    if let Value::Object(map) = extra {
        item.extra = map.iter().filter(|(_, v)| !v.is_null()).map(|(k, v)| (k.clone(), v.clone())).collect();
    }
    item
}

fn state_of(v: &Value) -> String {
    let conclusion = s(&v["conclusion"]);
    if conclusion.is_empty() {
        s(&v["status"]).to_owned()
    } else {
        conclusion.to_owned()
    }
}

/// A page of a listing: `limit` items.
async fn list(
    gh: &GitHub,
    token: &str,
    call: &ConnectorCall,
    path: &str,
    mut query: Query,
    key: Option<&str>,
) -> Res<Vec<Value>> {
    let n = limit(call);
    query.push(("per_page", n.min(100).to_string()));
    gh.pages(token, path, &query, key, n).await
}

fn opt(query: &mut Query, call: &ConnectorCall, arg: &str, name: &'static str) {
    if let Some(v) = call.str_arg(arg) {
        query.push((name, v.to_owned()));
    }
}

async fn get(gh: &GitHub, token: &str, path: &str) -> Res<Value> {
    gh.call(token, Method::GET, path, &[], None).await
}

// ---- Downloads that redirect (logs, artifacts) --------------------------------------------------------------------

/// Where a redirect may lead: the API host itself, or any other https host by name. The token is never sent there.
fn location_ok(base: &str, location: &str) -> Res<String> {
    let refuse = || CoreError::service("GitHub pointed to a download location that is not allowed.");
    let base = url::Url::parse(base).map_err(|_| refuse())?;
    let url = base.join(location).map_err(|_| refuse())?;
    let same_origin = url.scheme() == base.scheme()
        && url.host_str() == base.host_str()
        && url.port_or_known_default() == base.port_or_known_default();
    let public_https = url.scheme() == "https"
        && matches!(url.host(), Some(url::Host::Domain(d)) if d != "localhost" && d.rsplit('.').next() != Some("local"));
    if (same_origin || public_https) && url.username().is_empty() && url.password().is_none() {
        Ok(url.to_string())
    } else {
        Err(refuse())
    }
}

/// The API call of a download: GitHub answers with a redirect, which is not followed here (the client never follows
/// one); `Ok(Err(location))` is where it points, `Ok(Ok(response))` the answer when there was no redirect.
async fn download_start(gh: &GitHub, token: &str, path: &str) -> Res<Result<reqwest::Response, String>> {
    let resp = gh
        .http
        .get(format!("{}{path}", gh.base))
        .bearer_auth(token)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .header("User-Agent", "reins")
        .send()
        .await?;
    let status = resp.status();
    if status.is_redirection() {
        let location = resp
            .headers()
            .get("location")
            .and_then(|h| h.to_str().ok())
            .ok_or_else(|| CoreError::service("GitHub did not say where the download is"))?;
        return Ok(Err(location_ok(&gh.base, location)?));
    }
    if status.is_success() {
        return Ok(Ok(resp));
    }
    let body = resp.text().await?;
    Err(super::explain(status, &body))
}

/// Just the link of a download (an artifact zip), which is not fetched.
async fn download_link(gh: &GitHub, token: &str, path: &str) -> Res<String> {
    match download_start(gh, token, path).await? {
        Err(location) => Ok(location),
        Ok(_) => Err(CoreError::service("GitHub answered with the file itself and not with a download link")),
    }
}

/// The text behind `path`, following the redirect to where the log is kept. The token goes to the API host only:
/// the second request carries no credentials. Returns the text (the start dropped when huge) and whether it was cut.
async fn download_text(gh: &GitHub, token: &str, path: &str) -> Res<(String, bool)> {
    let mut resp = match download_start(gh, token, path).await? {
        Ok(resp) => resp,
        Err(location) => {
            let resp = gh.http.get(location).header("User-Agent", "reins").send().await?;
            if !resp.status().is_success() {
                return Err(CoreError::service(format!(
                    "The log is no longer available (the download answered {}); GitHub keeps logs for a limited time.",
                    resp.status().as_u16()
                )));
            }
            resp
        }
    };
    let mut buf: Vec<u8> = Vec::new();
    let mut dropped = false;
    while let Some(chunk) = resp.chunk().await? {
        buf.extend_from_slice(&chunk);
        if buf.len() > 2 * LOG_KEEP_BYTES {
            let extra = buf.len() - LOG_KEEP_BYTES;
            buf.drain(..extra);
            dropped = true;
        }
    }
    Ok((String::from_utf8_lossy(&buf).into_owned(), dropped))
}

/// Terminal colour codes and a byte-order mark removed.
fn strip_ansi(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\u{feff}' => {}
            '\u{1b}' => {
                if chars.peek() == Some(&'[') {
                    chars.next();
                    for n in chars.by_ref() {
                        if ('@'..='~').contains(&n) {
                            break;
                        }
                    }
                }
            }
            _ => out.push(c),
        }
    }
    out
}

/// The end of a log: the last `tail_lines` lines if asked, at most `max_chars` characters, starting at a line start.
/// The flag says whether anything of the start was cut.
fn log_tail(raw: &str, tail_lines: Option<usize>, max_chars: usize) -> (String, bool) {
    let clean = strip_ansi(raw).replace("\r\n", "\n");
    let mut cut = false;
    let mut kept = clean.as_str();
    let by_lines;
    if let Some(n) = tail_lines {
        let lines: Vec<&str> = clean.lines().collect();
        if lines.len() > n {
            cut = true;
            by_lines = lines[lines.len() - n..].join("\n");
            kept = &by_lines;
        }
    }
    let count = kept.chars().count();
    if count > max_chars {
        cut = true;
        let start = kept.char_indices().nth(count - max_chars).map_or(0, |(i, _)| i);
        let tail = &kept[start..];
        let at_line_start = kept[..start].ends_with('\n');
        let tail = if at_line_start {
            tail
        } else {
            tail.split_once('\n').map_or(tail, |(_, rest)| rest)
        };
        return (tail.to_owned(), cut);
    }
    (kept.to_owned(), cut)
}

// ---- Workflows ----------------------------------------------------------------------------------------------------

fn workflow_item(repo: &str, w: &Value) -> Item {
    let mut item = repo_base(repo);
    item.id = w["id"].to_string();
    item.title = short(s(&w["name"]), 200);
    item.snippet = format!("{} · {}", s(&w["path"]), s(&w["state"]));
    item.date = when(&w["updated_at"]);
    with(
        item,
        &json!({"id": w["id"], "name": w["name"], "path": w["path"], "state": w["state"], "url": w["html_url"],
            "created_at": w["created_at"], "updated_at": w["updated_at"]}),
    )
}

async fn workflow_list(gh: &GitHub, token: &str, call: &ConnectorCall) -> Res<Vec<Item>> {
    let repo = repo_arg(call)?;
    let items =
        list(gh, token, call, &format!("/repos/{repo}/actions/workflows"), Vec::new(), Some("workflows")).await?;
    Ok(items.iter().map(|w| workflow_item(&repo, w)).collect())
}

async fn workflow_get(gh: &GitHub, token: &str, call: &ConnectorCall) -> Res<Vec<Item>> {
    let repo = repo_arg(call)?;
    let w = get(gh, token, &format!("/repos/{repo}/actions/workflows/{}", workflow_arg(call)?)).await?;
    Ok(vec![workflow_item(&repo, &w)])
}

async fn workflow_usage(gh: &GitHub, token: &str, call: &ConnectorCall) -> Res<Vec<Item>> {
    let repo = repo_arg(call)?;
    let w = workflow_arg(call)?;
    let v = get(gh, token, &format!("/repos/{repo}/actions/workflows/{w}/timing")).await?;
    let mut item = repo_base(&repo);
    item.id = format!("usage-{w}");
    item.title = format!("Usage of workflow {}", call.str_arg("workflow").unwrap_or_default());
    let mut parts = Vec::new();
    for (os, t) in v["billable"].as_object().into_iter().flatten() {
        parts.push(format!("{os}: {} ms", t["total_ms"].as_i64().unwrap_or(0)));
    }
    item.snippet = if parts.is_empty() {
        "No billable time reported".to_owned()
    } else {
        parts.join(" · ")
    };
    Ok(vec![with(item, &json!({"billable": v["billable"]}))])
}

// ---- Runs and jobs ------------------------------------------------------------------------------------------------

fn run_item(repo: &str, r: &Value) -> Item {
    let mut item = repo_base(repo);
    item.id = r["id"].to_string();
    let name = if s(&r["display_title"]).is_empty() {
        s(&r["name"])
    } else {
        s(&r["display_title"])
    };
    item.title = short(name, 200);
    item.from = s(&r["actor"]["login"]).to_owned();
    item.snippet = format!(
        "#{} · {} · {} · {} · {}",
        r["run_number"],
        short(s(&r["name"]), 60),
        state_of(r),
        s(&r["head_branch"]),
        s(&r["event"])
    );
    item.date = when(&r["updated_at"]);
    with(
        item,
        &json!({"id": r["id"], "run_number": r["run_number"], "run_attempt": r["run_attempt"], "workflow_id": r["workflow_id"],
            "workflow": r["name"], "status": r["status"], "conclusion": r["conclusion"], "branch": r["head_branch"],
            "head_sha": r["head_sha"], "event": r["event"], "actor": r["actor"]["login"], "url": r["html_url"],
            "created_at": r["created_at"], "started_at": r["run_started_at"], "updated_at": r["updated_at"]}),
    )
}

async fn run_list(gh: &GitHub, token: &str, call: &ConnectorCall) -> Res<Vec<Item>> {
    let repo = repo_arg(call)?;
    let path = match call.str_arg("workflow") {
        Some(w) if workflow_ok(w) => format!("/repos/{repo}/actions/workflows/{}/runs", segment(w)),
        Some(_) => return Err(CoreError::service("`workflow` must be a workflow id or a file name like ci.yml.")),
        None => format!("/repos/{repo}/actions/runs"),
    };
    let mut query = Query::new();
    if call.str_arg("branch").is_some() {
        if ref_arg_ok(call, "branch") {
            opt(&mut query, call, "branch", "branch");
        } else {
            return Err(CoreError::service("`branch` is not a valid branch name."));
        }
    }
    opt(&mut query, call, "event", "event");
    opt(&mut query, call, "status", "status");
    opt(&mut query, call, "actor", "actor");
    let runs = list(gh, token, call, &path, query, Some("workflow_runs")).await?;
    Ok(runs.iter().map(|r| run_item(&repo, r)).collect())
}

fn ref_arg_ok(call: &ConnectorCall, name: &str) -> bool {
    call.str_arg(name).is_some_and(ref_ok)
}

async fn run_get(gh: &GitHub, token: &str, call: &ConnectorCall) -> Res<Vec<Item>> {
    let repo = repo_arg(call)?;
    let run = id_arg(call, "run_id")?;
    let path = match call.int_arg("attempt") {
        Some(n) if n > 0 => format!("/repos/{repo}/actions/runs/{run}/attempts/{n}"),
        _ => format!("/repos/{repo}/actions/runs/{run}"),
    };
    let r = get(gh, token, &path).await?;
    let mut item = run_item(&repo, &r);
    item.extra.insert("commit_message".to_owned(), json!(short(s(&r["head_commit"]["message"]), 300)));
    Ok(vec![item])
}

async fn run_attempts(gh: &GitHub, token: &str, call: &ConnectorCall) -> Res<Vec<Item>> {
    let repo = repo_arg(call)?;
    let run = id_arg(call, "run_id")?;
    let latest = get(gh, token, &format!("/repos/{repo}/actions/runs/{run}")).await?;
    let last = latest["run_attempt"].as_i64().unwrap_or(1).max(1);
    let mut items = Vec::new();
    for attempt in (1.max(last - 19)..=last).rev() {
        let r = if attempt == last {
            latest.clone()
        } else {
            get(gh, token, &format!("/repos/{repo}/actions/runs/{run}/attempts/{attempt}")).await?
        };
        let mut item = run_item(&repo, &r);
        item.id = format!("{run}-{attempt}");
        items.push(item);
    }
    Ok(items)
}

fn job_item(repo: &str, j: &Value) -> Item {
    let mut item = repo_base(repo);
    item.id = j["id"].to_string();
    item.title = short(s(&j["name"]), 200);
    let steps: Vec<Value> = j["steps"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|st| json!({"number": st["number"], "name": st["name"], "status": st["status"], "conclusion": st["conclusion"]}))
        .collect();
    let failed: Vec<&str> = j["steps"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|st| st["conclusion"] == "failure")
        .map(|st| s(&st["name"]))
        .collect();
    item.snippet = format!(
        "{}{}{}",
        state_of(j),
        if s(&j["runner_name"]).is_empty() {
            String::new()
        } else {
            format!(" · runner {}", s(&j["runner_name"]))
        },
        if failed.is_empty() {
            String::new()
        } else {
            format!(" · failed step: {}", short(&failed.join(", "), 120))
        }
    );
    item.date = when(if j["completed_at"].is_null() {
        &j["started_at"]
    } else {
        &j["completed_at"]
    });
    with(
        item,
        &json!({"id": j["id"], "run_id": j["run_id"], "run_attempt": j["run_attempt"], "status": j["status"],
            "conclusion": j["conclusion"], "started_at": j["started_at"], "completed_at": j["completed_at"],
            "runner_name": j["runner_name"], "labels": j["labels"], "steps": steps, "url": j["html_url"]}),
    )
}

async fn run_jobs(gh: &GitHub, token: &str, call: &ConnectorCall) -> Res<Vec<Item>> {
    let repo = repo_arg(call)?;
    let run = id_arg(call, "run_id")?;
    let query = vec![("filter", call.str_arg("filter").unwrap_or("latest").to_owned())];
    let jobs = list(gh, token, call, &format!("/repos/{repo}/actions/runs/{run}/jobs"), query, Some("jobs")).await?;
    Ok(jobs.iter().map(|j| job_item(&repo, j)).collect())
}

async fn job_get(gh: &GitHub, token: &str, call: &ConnectorCall) -> Res<Vec<Item>> {
    let repo = repo_arg(call)?;
    let job = id_arg(call, "job_id")?;
    let j = get(gh, token, &format!("/repos/{repo}/actions/jobs/{job}")).await?;
    Ok(vec![job_item(&repo, &j)])
}

fn tail_arg(call: &ConnectorCall) -> Option<usize> {
    call.int_arg("tail_lines").and_then(|n| usize::try_from(n).ok()).filter(|n| *n > 0)
}

async fn job_logs(gh: &GitHub, token: &str, call: &ConnectorCall) -> Res<Vec<Item>> {
    let repo = repo_arg(call)?;
    let job = id_arg(call, "job_id")?;
    let (raw, dropped) = download_text(gh, token, &format!("/repos/{repo}/actions/jobs/{job}/logs")).await?;
    let (body, cut) = log_tail(&raw, tail_arg(call), LOG_CHARS);
    let mut item = repo_base(&repo);
    item.id = job.to_string();
    item.title = format!("Log of job {job}");
    item.snippet = format!(
        "{} characters{}",
        body.chars().count(),
        if cut || dropped {
            " (end of the log)"
        } else {
            ""
        }
    );
    item.body = Some(body);
    let mut item = with(item, &json!({"job_id": job, "truncated": cut || dropped}));
    // The whole log as a download link, once released, when only its end fits here.
    if (cut || dropped) && crate::blob::linking() {
        let url = format!("{}/repos/{repo}/actions/jobs/{job}/logs", gh.base);
        let marker =
            crate::blob::fetch_marker(&url, "application/vnd.github+json", &format!("job-{job}.log"), u64::MAX);
        item.extra.insert(crate::blob::DELIVER_KEY.to_owned(), marker);
    }
    Ok(vec![item])
}

async fn run_logs(gh: &GitHub, token: &str, call: &ConnectorCall) -> Res<Vec<Item>> {
    let repo = repo_arg(call)?;
    let run = id_arg(call, "run_id")?;
    let query = vec![("filter", "latest".to_owned())];
    let mut jobs = gh.pages(token, &format!("/repos/{repo}/actions/runs/{run}/jobs"), &query, Some("jobs"), 30).await?;
    // Failed jobs first: that is where the reader looks.
    jobs.sort_by_key(|j| j["conclusion"] != "failure");
    let budget = (LOG_CHARS / jobs.len().max(1)).clamp(2_000, LOG_CHARS);
    let mut items = Vec::new();
    for j in &jobs {
        let mut item = job_item(&repo, j);
        let id = j["id"].as_i64().unwrap_or(0);
        match download_text(gh, token, &format!("/repos/{repo}/actions/jobs/{id}/logs")).await {
            Ok((raw, dropped)) => {
                let (body, cut) = log_tail(&raw, tail_arg(call), budget);
                item.extra.insert("truncated".to_owned(), json!(cut || dropped));
                item.body = Some(body);
            }
            Err(
                e @ CoreError::ServiceNeedsAttention {
                    ..
                },
            ) => return Err(e),
            Err(e) => {
                item.snippet = format!("{} · no log: {}", item.snippet, short(&e.to_string(), 120));
            }
        }
        items.push(item);
    }
    Ok(items)
}

// ---- Artifacts, deployments, caches --------------------------------------------------------------------------------

async fn artifact_list(gh: &GitHub, token: &str, call: &ConnectorCall) -> Res<Vec<Item>> {
    let repo = repo_arg(call)?;
    let path = match call.int_arg("run_id") {
        Some(run) if run > 0 => format!("/repos/{repo}/actions/runs/{run}/artifacts"),
        _ => format!("/repos/{repo}/actions/artifacts"),
    };
    let mut query = Query::new();
    opt(&mut query, call, "name", "name");
    let artifacts = list(gh, token, call, &path, query, Some("artifacts")).await?;
    Ok(artifacts
        .iter()
        .map(|a| {
            let mut item = repo_base(&repo);
            item.id = a["id"].to_string();
            item.title = short(s(&a["name"]), 200);
            item.snippet = format!(
                "{} · {}{}",
                size(a["size_in_bytes"].as_i64().unwrap_or(0)),
                if a["expired"] == true {
                    "expired".to_owned()
                } else {
                    format!("expires {}", s(&a["expires_at"]))
                },
                a["workflow_run"]["head_branch"].as_str().map(|b| format!(" · {b}")).unwrap_or_default()
            );
            item.date = when(&a["created_at"]);
            with(
                item,
                &json!({"id": a["id"], "name": a["name"], "size_in_bytes": a["size_in_bytes"], "expired": a["expired"],
                    "created_at": a["created_at"], "expires_at": a["expires_at"], "run_id": a["workflow_run"]["id"],
                    "branch": a["workflow_run"]["head_branch"]}),
            )
        })
        .collect())
}

async fn artifact_download_url(gh: &GitHub, token: &str, call: &ConnectorCall) -> Res<Vec<Item>> {
    let repo = repo_arg(call)?;
    let artifact = id_arg(call, "artifact_id")?;
    let link = download_link(gh, token, &format!("/repos/{repo}/actions/artifacts/{artifact}/zip")).await?;
    let mut item = repo_base(&repo);
    item.id = artifact.to_string();
    item.title = format!("Download link for artifact {artifact}");
    item.snippet = "Temporary download link (valid for about a minute)".to_owned();
    item.body = Some(link);
    item.secret = true;
    Ok(vec![with(item, &json!({"artifact_id": artifact}))])
}

async fn pending_deployments(gh: &GitHub, token: &str, call: &ConnectorCall) -> Res<Vec<Item>> {
    let repo = repo_arg(call)?;
    let run = id_arg(call, "run_id")?;
    let v = get(gh, token, &format!("/repos/{repo}/actions/runs/{run}/pending_deployments")).await?;
    Ok(v.as_array()
        .into_iter()
        .flatten()
        .map(|d| {
            let mut item = repo_base(&repo);
            item.id = d["environment"]["id"].to_string();
            item.title = short(s(&d["environment"]["name"]), 200);
            item.snippet = format!(
                "waiting for review · {}",
                if d["current_user_can_approve"] == true {
                    "you can review it"
                } else {
                    "you cannot review it"
                }
            );
            let reviewers: Vec<&str> = d["reviewers"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|r| {
                    if r["reviewer"]["login"].is_string() {
                        s(&r["reviewer"]["login"])
                    } else {
                        s(&r["reviewer"]["slug"])
                    }
                })
                .collect();
            with(
                item,
                &json!({"environment_id": d["environment"]["id"], "environment": d["environment"]["name"],
                    "wait_timer": d["wait_timer"], "current_user_can_approve": d["current_user_can_approve"],
                    "reviewers": reviewers}),
            )
        })
        .collect())
}

async fn cache_list(gh: &GitHub, token: &str, call: &ConnectorCall) -> Res<Vec<Item>> {
    let repo = repo_arg(call)?;
    let mut query = Query::new();
    opt(&mut query, call, "key", "key");
    opt(&mut query, call, "ref", "ref");
    let caches = list(gh, token, call, &format!("/repos/{repo}/actions/caches"), query, Some("actions_caches")).await?;
    Ok(caches
        .iter()
        .map(|c| {
            let mut item = repo_base(&repo);
            item.id = c["id"].to_string();
            item.title = short(s(&c["key"]), 200);
            item.snippet = format!("{} · {}", size(c["size_in_bytes"].as_i64().unwrap_or(0)), s(&c["ref"]));
            item.date = when(&c["last_accessed_at"]);
            with(
                item,
                &json!({"id": c["id"], "key": c["key"], "ref": c["ref"], "size_in_bytes": c["size_in_bytes"],
                    "created_at": c["created_at"], "last_accessed_at": c["last_accessed_at"]}),
            )
        })
        .collect())
}

// ---- Variables, secrets, environments -----------------------------------------------------------------------------

fn variable_item(repo: &str, env: Option<&str>, v: &Value) -> Item {
    let mut item = repo_base(repo);
    item.id = s(&v["name"]).to_owned();
    item.title = s(&v["name"]).to_owned();
    item.snippet = short(s(&v["value"]), 200);
    item.body = v["value"].as_str().map(str::to_owned);
    item.date = when(&v["updated_at"]);
    with(
        item,
        &json!({"name": v["name"], "environment": env, "created_at": v["created_at"], "updated_at": v["updated_at"]}),
    )
}

async fn variable_list(gh: &GitHub, token: &str, call: &ConnectorCall) -> Res<Vec<Item>> {
    let repo = repo_arg(call)?;
    let env = env_arg(call);
    let vars =
        list(gh, token, call, &scoped(&repo, env.as_deref(), "variables"), Query::new(), Some("variables")).await?;
    Ok(vars.iter().map(|v| variable_item(&repo, env.as_deref(), v)).collect())
}

async fn variable_get(gh: &GitHub, token: &str, call: &ConnectorCall) -> Res<Vec<Item>> {
    let repo = repo_arg(call)?;
    let env = env_arg(call);
    let name = var_name(call, false)?;
    let v = get(gh, token, &format!("{}/{name}", scoped(&repo, env.as_deref(), "variables"))).await?;
    Ok(vec![variable_item(&repo, env.as_deref(), &v)])
}

async fn secret_list(gh: &GitHub, token: &str, call: &ConnectorCall) -> Res<Vec<Item>> {
    let repo = repo_arg(call)?;
    let env = env_arg(call);
    let secrets =
        list(gh, token, call, &scoped(&repo, env.as_deref(), "secrets"), Query::new(), Some("secrets")).await?;
    Ok(secrets
        .iter()
        .map(|v| {
            let mut item = repo_base(&repo);
            item.id = s(&v["name"]).to_owned();
            item.title = s(&v["name"]).to_owned();
            item.snippet = "secret (the value is never shown)".to_owned();
            item.date = when(&v["updated_at"]);
            with(item, &json!({"name": v["name"], "environment": env, "updated_at": v["updated_at"]}))
        })
        .collect())
}

fn environment_item(repo: &str, e: &Value) -> Item {
    let mut item = repo_base(repo);
    item.id = s(&e["name"]).to_owned();
    item.title = short(s(&e["name"]), 200);
    let rules: Vec<&str> = e["protection_rules"].as_array().into_iter().flatten().map(|r| s(&r["type"])).collect();
    item.snippet = if rules.is_empty() {
        "no protection rules".to_owned()
    } else {
        rules.join(", ")
    };
    item.date = when(&e["updated_at"]);
    with(
        item,
        &json!({"name": e["name"], "protection_rules": e["protection_rules"],
            "deployment_branch_policy": e["deployment_branch_policy"], "can_admins_bypass": e["can_admins_bypass"],
            "url": e["html_url"], "created_at": e["created_at"], "updated_at": e["updated_at"]}),
    )
}

async fn environment_list(gh: &GitHub, token: &str, call: &ConnectorCall) -> Res<Vec<Item>> {
    let repo = repo_arg(call)?;
    let envs =
        list(gh, token, call, &format!("/repos/{repo}/environments"), Query::new(), Some("environments")).await?;
    Ok(envs.iter().map(|e| environment_item(&repo, e)).collect())
}

async fn environment_get(gh: &GitHub, token: &str, call: &ConnectorCall) -> Res<Vec<Item>> {
    let repo = repo_arg(call)?;
    let env = env_required(call)?;
    let e = get(gh, token, &format!("/repos/{repo}/environments/{}", segment(&env))).await?;
    Ok(vec![environment_item(&repo, &e)])
}

// ---- Security alerts ----------------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Alerts {
    Dependabot,
    CodeScanning,
    SecretScanning,
}

impl Alerts {
    fn path(self) -> &'static str {
        match self {
            Self::Dependabot => "dependabot/alerts",
            Self::CodeScanning => "code-scanning/alerts",
            Self::SecretScanning => "secret-scanning/alerts",
        }
    }
}

fn alert_item(repo: &str, kind: Alerts, a: &Value) -> Item {
    let mut item = repo_base(repo);
    item.id = a["number"].to_string();
    item.date = when(&a["created_at"]);
    match kind {
        Alerts::Dependabot => {
            let adv = &a["security_advisory"];
            item.title = short(s(&adv["summary"]), 200);
            item.snippet = format!(
                "#{} · {} · {} · {} ({})",
                a["number"],
                s(&adv["severity"]),
                s(&a["state"]),
                s(&a["dependency"]["package"]["name"]),
                s(&a["dependency"]["package"]["ecosystem"])
            );
            with(
                item,
                &json!({"number": a["number"], "state": a["state"], "severity": adv["severity"], "ghsa_id": adv["ghsa_id"],
                    "cve_id": adv["cve_id"], "package": a["dependency"]["package"]["name"],
                    "ecosystem": a["dependency"]["package"]["ecosystem"], "manifest": a["dependency"]["manifest_path"],
                    "vulnerable_version_range": a["security_vulnerability"]["vulnerable_version_range"],
                    "first_patched_version": a["security_vulnerability"]["first_patched_version"]["identifier"],
                    "url": a["html_url"], "created_at": a["created_at"], "fixed_at": a["fixed_at"],
                    "dismissed_reason": a["dismissed_reason"]}),
            )
        }
        Alerts::CodeScanning => {
            let loc = &a["most_recent_instance"]["location"];
            item.title = short(
                if s(&a["rule"]["description"]).is_empty() {
                    s(&a["rule"]["id"])
                } else {
                    s(&a["rule"]["description"])
                },
                200,
            );
            item.snippet = format!(
                "#{} · {} · {} · {}:{}",
                a["number"],
                if s(&a["rule"]["security_severity_level"]).is_empty() {
                    s(&a["rule"]["severity"])
                } else {
                    s(&a["rule"]["security_severity_level"])
                },
                s(&a["state"]),
                s(&loc["path"]),
                loc["start_line"]
            );
            with(
                item,
                &json!({"number": a["number"], "state": a["state"], "rule": a["rule"]["id"], "severity": a["rule"]["severity"],
                    "security_severity": a["rule"]["security_severity_level"], "tool": a["tool"]["name"],
                    "ref": a["most_recent_instance"]["ref"], "path": loc["path"], "start_line": loc["start_line"],
                    "end_line": loc["end_line"], "url": a["html_url"], "created_at": a["created_at"],
                    "dismissed_reason": a["dismissed_reason"]}),
            )
        }
        Alerts::SecretScanning => {
            let kind_name = if s(&a["secret_type_display_name"]).is_empty() {
                s(&a["secret_type"])
            } else {
                s(&a["secret_type_display_name"])
            };
            item.title = format!("Exposed secret: {}", short(kind_name, 150));
            item.snippet = format!(
                "#{} · {} · {} (the secret itself is withheld from the approval)",
                a["number"],
                kind_name,
                s(&a["state"])
            );
            item.sensitive = true;
            item.secret = true;
            item.body = a["secret"].as_str().map(str::to_owned);
            with(
                item,
                &json!({"number": a["number"], "state": a["state"], "secret_type": a["secret_type"],
                    "validity": a["validity"], "resolution": a["resolution"], "url": a["html_url"],
                    "created_at": a["created_at"], "publicly_leaked": a["publicly_leaked"]}),
            )
        }
    }
}

async fn alert_list(gh: &GitHub, token: &str, call: &ConnectorCall, kind: Alerts) -> Res<Vec<Item>> {
    let repo = repo_arg(call)?;
    let mut query = Query::new();
    query.push(("state", call.str_arg("state").unwrap_or("open").to_owned()));
    opt(&mut query, call, "severity", "severity");
    opt(&mut query, call, "package", "package");
    opt(&mut query, call, "secret_type", "secret_type");
    if kind == Alerts::CodeScanning && call.str_arg("ref").is_some() {
        opt(&mut query, call, "ref", "ref");
    }
    let alerts = list(gh, token, call, &format!("/repos/{repo}/{}", kind.path()), query, None).await?;
    Ok(alerts.iter().map(|a| alert_item(&repo, kind, a)).collect())
}

async fn alert_get(gh: &GitHub, token: &str, call: &ConnectorCall, kind: Alerts) -> Res<Vec<Item>> {
    let repo = repo_arg(call)?;
    let number = id_arg(call, "number")?;
    let a = get(gh, token, &format!("/repos/{repo}/{}/{number}", kind.path())).await?;
    let mut item = alert_item(&repo, kind, &a);
    match kind {
        Alerts::Dependabot => {
            item.body =
                Some(text::truncate_chars(s(&a["security_advisory"]["description"]), 20_000)).filter(|b| !b.is_empty());
        }
        Alerts::CodeScanning => {
            let message = s(&a["most_recent_instance"]["message"]["text"]);
            let help = s(&a["rule"]["help"]);
            item.body =
                Some(text::truncate_chars(&format!("{message}\n\n{help}"), 20_000)).filter(|b| !b.trim().is_empty());
        }
        Alerts::SecretScanning => {}
    }
    Ok(vec![item])
}

async fn advisory_list(gh: &GitHub, token: &str, call: &ConnectorCall) -> Res<Vec<Item>> {
    let repo = repo_arg(call)?;
    let mut query = Query::new();
    opt(&mut query, call, "state", "state");
    let advisories = list(gh, token, call, &format!("/repos/{repo}/security-advisories"), query, None).await?;
    Ok(advisories
        .iter()
        .map(|a| {
            let mut item = repo_base(&repo);
            item.id = s(&a["ghsa_id"]).to_owned();
            item.title = short(s(&a["summary"]), 200);
            item.snippet = format!("{} · {} · {}", s(&a["ghsa_id"]), s(&a["severity"]), s(&a["state"]));
            item.date = when(&a["published_at"]);
            item.body = Some(text::truncate_chars(s(&a["description"]), 10_000)).filter(|b| !b.is_empty());
            with(
                item,
                &json!({"ghsa_id": a["ghsa_id"], "cve_id": a["cve_id"], "severity": a["severity"], "state": a["state"],
                    "url": a["html_url"], "published_at": a["published_at"]}),
            )
        })
        .collect())
}

async fn dependency_sbom(gh: &GitHub, token: &str, call: &ConnectorCall) -> Res<Vec<Item>> {
    let repo = repo_arg(call)?;
    let v = get(gh, token, &format!("/repos/{repo}/dependency-graph/sbom")).await?;
    let packages = v["sbom"]["packages"].as_array().cloned().unwrap_or_default();
    let mut body = String::new();
    let mut cut = false;
    for p in &packages {
        let line = format!("{} {}\n", s(&p["name"]), s(&p["versionInfo"]));
        if body.chars().count() + line.chars().count() > LOG_CHARS {
            cut = true;
            break;
        }
        body.push_str(&line);
    }
    let mut item = repo_base(&repo);
    item.id = "sbom".to_owned();
    item.title = format!("Dependencies of {repo}");
    item.snippet = format!("{} packages", packages.len());
    item.body = Some(body);
    Ok(vec![with(item, &json!({"package_count": packages.len(), "truncated": cut}))])
}

// ---- The account --------------------------------------------------------------------------------------------------

async fn user(gh: &GitHub, token: &str, path: Res<String>) -> Res<Vec<Item>> {
    let u = get(gh, token, &path?).await?;
    let mut item = account_base();
    item.id = s(&u["login"]).to_owned();
    item.title = s(&u["login"]).to_owned();
    item.snippet = short(&format!("{} {}", s(&u["name"]), s(&u["description"])), 200);
    item.date = when(&u["created_at"]);
    item.body = u["bio"].as_str().map(|b| text::truncate_chars(b, 2_000)).filter(|b| !b.is_empty());
    Ok(vec![with(
        item,
        &json!({"login": u["login"], "id": u["id"], "type": u["type"], "name": u["name"], "company": u["company"],
            "location": u["location"], "blog": u["blog"], "public_repos": u["public_repos"],
            "followers": u["followers"], "following": u["following"], "plan": u["plan"]["name"],
            "url": u["html_url"], "created_at": u["created_at"]}),
    )])
}

#[derive(Clone, Copy)]
enum Listing {
    Login,
    Repo,
    Team,
    SshKey,
    GpgKey,
}

async fn account_list(
    gh: &GitHub,
    token: &str,
    call: &ConnectorCall,
    path: Res<String>,
    kind: Listing,
) -> Res<Vec<Item>> {
    let entries = list(gh, token, call, &path?, Vec::new(), None).await?;
    Ok(entries.iter().map(|e| listing_item(kind, e)).collect())
}

fn listing_item(kind: Listing, e: &Value) -> Item {
    let mut item = account_base();
    match kind {
        Listing::Login => {
            item.id = s(&e["login"]).to_owned();
            item.title = s(&e["login"]).to_owned();
            item.snippet = short(s(&e["description"]), 200);
            with(item, &json!({"login": e["login"], "type": e["type"], "url": e["html_url"]}))
        }
        Listing::Repo => {
            item.id = s(&e["full_name"]).to_owned();
            item.title = s(&e["full_name"]).to_owned();
            item.snippet = short(s(&e["description"]), 200);
            item.date = when(&e["pushed_at"]);
            with(
                item,
                &json!({"full_name": e["full_name"], "private": e["private"], "archived": e["archived"],
                    "default_branch": e["default_branch"], "language": e["language"],
                    "stars": e["stargazers_count"], "url": e["html_url"]}),
            )
        }
        Listing::Team => {
            item.id = s(&e["slug"]).to_owned();
            item.title = short(s(&e["name"]), 200);
            item.snippet = format!("{} · {}", s(&e["privacy"]), short(s(&e["description"]), 150));
            with(
                item,
                &json!({"slug": e["slug"], "privacy": e["privacy"], "permission": e["permission"], "url": e["html_url"]}),
            )
        }
        Listing::SshKey => {
            item.id = e["id"].to_string();
            item.title = short(s(&e["title"]), 200);
            item.snippet = format!("SSH key {}", e["id"]);
            item.date = when(&e["created_at"]);
            with(item, &json!({"id": e["id"], "title": e["title"], "created_at": e["created_at"]}))
        }
        Listing::GpgKey => {
            let emails: Vec<&str> = e["emails"].as_array().into_iter().flatten().map(|m| s(&m["email"])).collect();
            item.id = e["id"].to_string();
            item.title = s(&e["key_id"]).to_owned();
            item.snippet = short(&emails.join(", "), 200);
            item.date = when(&e["created_at"]);
            with(item, &json!({"id": e["id"], "key_id": e["key_id"], "emails": emails, "expires_at": e["expires_at"]}))
        }
    }
}

async fn starred_list(gh: &GitHub, token: &str, call: &ConnectorCall) -> Res<Vec<Item>> {
    let path = match call.str_arg("username") {
        Some(_) => format!("/users/{}/starred", name_arg(call, "username")?),
        None => "/user/starred".to_owned(),
    };
    account_list(gh, token, call, Ok(path), Listing::Repo).await
}

async fn rate_limit(gh: &GitHub, token: &str) -> Res<Vec<Item>> {
    let v = get(gh, token, "/rate_limit").await?;
    let mut limits = Map::new();
    let mut parts = Vec::new();
    for (name, l) in v["resources"].as_object().into_iter().flatten() {
        if let (Some(max), Some(left)) = (l["limit"].as_i64(), l["remaining"].as_i64()) {
            parts.push(format!("{name} {left}/{max}"));
            limits.insert(
                name.clone(),
                json!({"limit": max, "remaining": left, "resets_at": l["reset"].as_i64().map(text::iso_utc)}),
            );
        }
    }
    let mut item = account_base();
    item.id = "rate_limit".to_owned();
    item.title = "GitHub rate limits".to_owned();
    item.snippet = parts.join(" · ");
    item.extra = limits;
    Ok(vec![item])
}

async fn notification_list(gh: &GitHub, token: &str, call: &ConnectorCall) -> Res<Vec<Item>> {
    let path = if call.str_arg("repo").is_some() {
        format!("/repos/{}/notifications", repo_arg(call)?)
    } else {
        "/notifications".to_owned()
    };
    let mut query = Query::new();
    if call.bool_arg("all") == Some(true) {
        query.push(("all", "true".to_owned()));
    }
    if call.bool_arg("participating") == Some(true) {
        query.push(("participating", "true".to_owned()));
    }
    let entries = list(gh, token, call, &path, query, None).await?;
    Ok(entries
        .iter()
        .map(|n| {
            let mut item = account_base();
            item.id = s(&n["id"]).to_owned();
            item.title = short(s(&n["subject"]["title"]), 200);
            item.snippet = format!(
                "{} · {} · {}{}",
                s(&n["reason"]),
                s(&n["subject"]["type"]),
                s(&n["repository"]["full_name"]),
                if n["unread"] == true { " · unread" } else { "" }
            );
            item.date = when(&n["updated_at"]);
            with(
                item,
                &json!({"id": n["id"], "reason": n["reason"], "unread": n["unread"], "repository": n["repository"]["full_name"],
                    "subject_type": n["subject"]["type"], "subject_api_url": n["subject"]["url"],
                    "last_read_at": n["last_read_at"]}),
            )
        })
        .collect())
}

fn gist_id_ok(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric())
}

fn gist_arg(call: &ConnectorCall) -> Res<String> {
    match call.str_arg("gist_id") {
        Some(g) if gist_id_ok(g) => Ok(g.to_owned()),
        _ => Err(CoreError::service("`gist_id` is not a valid gist id.")),
    }
}

fn gist_item(g: &Value) -> Item {
    let mut item = account_base();
    let names: Vec<&str> = g["files"].as_object().into_iter().flatten().map(|(n, _)| n.as_str()).collect();
    item.id = s(&g["id"]).to_owned();
    item.title = short(
        if s(&g["description"]).is_empty() {
            names.first().copied().unwrap_or("(empty gist)")
        } else {
            s(&g["description"])
        },
        200,
    );
    item.snippet = format!(
        "{} · {}",
        if g["public"] == true {
            "public"
        } else {
            "secret"
        },
        short(&names.join(", "), 150)
    );
    item.date = when(&g["updated_at"]);
    with(
        item,
        &json!({"id": g["id"], "public": g["public"], "files": names, "url": g["html_url"], "owner": g["owner"]["login"]}),
    )
}

async fn gist_list(gh: &GitHub, token: &str, call: &ConnectorCall) -> Res<Vec<Item>> {
    let path = match call.str_arg("username") {
        Some(_) => format!("/users/{}/gists", name_arg(call, "username")?),
        None => "/gists".to_owned(),
    };
    let entries = list(gh, token, call, &path, Vec::new(), None).await?;
    Ok(entries.iter().map(gist_item).collect())
}

async fn gist_get(gh: &GitHub, token: &str, call: &ConnectorCall) -> Res<Vec<Item>> {
    let g = get(gh, token, &format!("/gists/{}", gist_arg(call)?)).await?;
    let mut item = gist_item(&g);
    let mut body = String::new();
    let mut cut = false;
    for (name, f) in g["files"].as_object().into_iter().flatten() {
        let content = s(&f["content"]);
        let part = text::truncate_chars(content, GIST_FILE_CHARS);
        cut |= part.len() < content.len() || f["truncated"] == true;
        write!(body, "--- {name} ---\n{part}\n\n").ok();
    }
    item.body = Some(text::truncate_chars(&body, LOG_CHARS));
    item.extra.insert("truncated".to_owned(), json!(cut));
    Ok(vec![item])
}

// ---- The generic request -------------------------------------------------------------------------------------------

/// Second segments of `/user/...` that are never reachable: credentials, keys, addresses, installations.
const DENIED_USER: &[&str] = &[
    "keys",
    "gpg_keys",
    "emails",
    "public_emails",
    "tokens",
    "ssh_signing_keys",
    "installations",
    "codespaces",
    "migrations",
];
const ROOTS: &[&str] = &["repos", "user", "users", "orgs", "search", "gists", "notifications", "rate_limit"];

/// Checks a path for the generic request. `Ok(Some(repo))` for a path under `/repos/owner/name`, `Ok(None)` for the
/// other allowed roots; `Err` says why the path is refused. Refused: anything but a plain absolute path of ASCII
/// path characters, `?` and `#`, `..` and `.` segments, empty segments, encoded slashes, dots, backslashes, question
/// marks, hashes and percent signs (double encoding) and control characters, roots outside the allowed list, and the
/// credential paths of `/user`.
fn request_path_ok(path: &str) -> Result<Option<String>, String> {
    let refuse = |why: &str| Err(format!("The path is not allowed: {why}."));
    if path.is_empty() || path.len() > 1_000 {
        return refuse("it must be 1..=1000 characters");
    }
    if !path.starts_with('/') {
        return refuse("it must start with /");
    }
    if path.contains(['?', '#']) {
        return refuse("use `query` for the query string, and no # fragment");
    }
    if !path.bytes().all(|b| {
        b.is_ascii_alphanumeric()
            || matches!(b, b'-' | b'_' | b'.' | b'~' | b'/' | b'%' | b':' | b'@' | b'+' | b',' | b'=')
    }) {
        return refuse("it has characters that do not belong in a path");
    }
    let bytes = path.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = path.get(i + 1..i + 3).filter(|h| h.bytes().all(|b| b.is_ascii_hexdigit()));
            let Some(decoded) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) else {
                return refuse("a % must be followed by two hex digits");
            };
            if decoded < 0x20 || decoded == 0x7f || matches!(decoded, b'/' | b'\\' | b'.' | b'?' | b'#' | b'%') {
                return refuse("an encoded slash, dot, question mark, percent or control character");
            }
            i += 3;
        } else {
            i += 1;
        }
    }
    let segments: Vec<&str> = path[1..].split('/').collect();
    if segments.iter().any(|seg| seg.is_empty() || *seg == "." || *seg == "..") {
        return refuse("empty, `.` and `..` segments");
    }
    let root = segments[0].to_ascii_lowercase();
    if !ROOTS.contains(&root.as_str()) {
        return refuse(
            "it must start with /repos/, /user, /users/, /orgs/, /search/, /gists, /notifications or /rate_limit",
        );
    }
    match root.as_str() {
        "repos" => {
            let repo = format!("{}/{}", segments.get(1).unwrap_or(&""), segments.get(2).unwrap_or(&""));
            if segments.len() < 3 || !repo_ok(&repo) {
                return refuse("/repos/ must be followed by owner/name");
            }
            Ok(Some(repo))
        }
        "user" => {
            if segments.get(1).is_some_and(|s| DENIED_USER.contains(&s.to_ascii_lowercase().as_str())) {
                return refuse("credentials, keys, addresses and installations are never reachable");
            }
            Ok(None)
        }
        "users" | "orgs" | "search" if segments.len() < 2 => refuse("it needs more than the root"),
        _ => Ok(None),
    }
}

/// The query of a generic request: names and values as given, names checked.
fn request_query(call: &ConnectorCall) -> Res<Vec<(String, String)>> {
    let mut out = Vec::new();
    if let Some(Value::Object(map)) = call.args.get("query") {
        for (k, v) in map {
            let ok = !k.is_empty()
                && k.len() <= 100
                && k.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b'[' | b']'));
            if !ok {
                return Err(CoreError::service(format!("The query name '{}' is not allowed.", short(k, 40))));
            }
            out.push((k.clone(), v.as_str().unwrap_or_default().to_owned()));
        }
    }
    Ok(out)
}

fn request_path(call: &ConnectorCall) -> Res<(String, Option<String>)> {
    let path = call.str_arg("path").unwrap_or_default();
    let repo = request_path_ok(path).map_err(CoreError::service)?;
    Ok((path.to_owned(), repo))
}

async fn request_read(gh: &GitHub, token: &str, call: &ConnectorCall) -> Res<Vec<Item>> {
    let (path, repo) = request_path(call)?;
    let query = request_query(call)?;
    let query: Vec<(&str, String)> = query.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
    let reply = gh.send(token, Method::GET, &path, &query, None, &Options::default()).await?;
    let raw = if reply.json.is_null() {
        reply.text.clone()
    } else {
        serde_json::to_string_pretty(&reply.json).unwrap_or_default()
    };
    let body = text::truncate_chars(&raw, LOG_CHARS);
    let mut item = match &repo {
        Some(r) => repo_base(r),
        None => account_base(),
    };
    item.id = "request".to_owned();
    item.title = format!("GET {}", short(&path, 200));
    item.snippet = format!("answered {} · {} characters", reply.status, raw.chars().count());
    item.sensitive = repo.is_none();
    let cut = body.len() < raw.len();
    item.body = Some(body);
    item.extra.insert("status".to_owned(), json!(reply.status));
    item.extra.insert("truncated".to_owned(), json!(cut));
    if let Some(next) = reply.next {
        item.extra.insert("next_path".to_owned(), json!(next));
    }
    Ok(vec![item])
}

// ---- Writes: one plan for the preview and the change ---------------------------------------------------------------

/// One change, described once: the preview shows it, `run` does it. Holds the body of the request, which may carry a
/// secret: it has no `Debug` and is never put into a preview or an error.
struct Plan {
    method: Method,
    path: String,
    query: Vec<(String, String)>,
    body: Option<Value>,
    resource: String,
    label: String,
    parents: Vec<(String, String)>,
    lines: Vec<String>,
    once: bool,
    /// The fixed part of the answer to the AI.
    done: Map<String, Value>,
    /// Fields of the GitHub answer to hand on (ids, links).
    pick: &'static [&'static str],
}

impl Plan {
    fn repo(method: Method, path: String, repo: &str, summary: String) -> Self {
        Self {
            method,
            path,
            query: Vec::new(),
            body: None,
            resource: repo.to_owned(),
            label: repo.to_owned(),
            parents: parents(repo, None),
            lines: vec![summary],
            once: false,
            done: Map::new(),
            pick: &[],
        }
    }

    fn account(method: Method, path: String, summary: String) -> Self {
        Self {
            resource: ACCOUNT.to_owned(),
            label: ACCOUNT_LABEL.to_owned(),
            parents: Vec::new(),
            ..Self::repo(method, path, ACCOUNT, summary)
        }
    }

    fn body(mut self, body: Value) -> Self {
        self.body = Some(body);
        self
    }

    fn line(mut self, line: impl Into<String>) -> Self {
        self.lines.push(line.into());
        self
    }

    fn once(mut self) -> Self {
        self.once = true;
        self
    }

    fn done(mut self, fields: &Value) -> Self {
        if let Value::Object(m) = fields {
            self.done.extend(m.iter().map(|(k, v)| (k.clone(), v.clone())));
        }
        self
    }

    fn pick(mut self, keys: &'static [&'static str]) -> Self {
        self.pick = keys;
        self
    }

    fn preview(&self) -> Preview {
        Preview {
            resource: self.resource.clone(),
            resource_label: self.label.clone(),
            lines: self.lines.clone(),
            parents: self.parents.clone(),
            once_only: self.once,
            ..Preview::default()
        }
    }

    async fn run(self, gh: &GitHub, token: &str) -> Res<Value> {
        let query: Vec<(&str, String)> = self.query.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
        let reply = gh.send(token, self.method, &self.path, &query, self.body.as_ref(), &Options::default()).await?;
        let mut out = self.done;
        for key in self.pick {
            if !reply.json[*key].is_null() {
                out.insert((*key).to_owned(), reply.json[*key].clone());
            }
        }
        Ok(Value::Object(out))
    }
}

/// The current state of something for a preview; nothing for the change itself.
async fn lookup(gh: &GitHub, token: &str, perform: bool, path: &str) -> Res<Value> {
    if perform {
        Ok(Value::Null)
    } else {
        get(gh, token, path).await
    }
}

/// Like `lookup`, but a missing thing is an answer (`None`).
async fn lookup_opt(gh: &GitHub, token: &str, perform: bool, path: &str) -> Res<Option<Value>> {
    if perform {
        return Ok(None);
    }
    let reply = gh
        .send(
            token,
            Method::GET,
            path,
            &[],
            None,
            &Options {
                allow: &[404],
                ..Options::default()
            },
        )
        .await?;
    Ok((reply.status != 404).then_some(reply.json))
}

/// The write behind an operation. `None` when the operation is not one of this area's writes. `perform` is false for
/// a preview (which looks things up to describe them) and true for the change itself.
async fn plan(gh: &GitHub, token: &str, call: &ConnectorCall, perform: bool) -> Option<Res<Plan>> {
    let (g, t, p) = (gh, token, perform);
    Some(match call.op.as_str() {
        "workflow_dispatch" => workflow_dispatch(g, t, call, p).await,
        "workflow_enable" => workflow_toggle(g, t, call, p, "enable").await,
        "workflow_disable" => workflow_toggle(g, t, call, p, "disable").await,
        "run_rerun" | "run_rerun_failed" | "run_cancel" | "run_force_cancel" | "run_delete" | "run_approve" => {
            run_change(g, t, call, p).await
        }
        "job_rerun" => job_rerun(g, t, call, p).await,
        "artifact_delete" => artifact_delete(g, t, call, p).await,
        "deployment_review" => deployment_review(g, t, call).await,
        "variable_create" | "variable_update" | "variable_delete" => variable_change(g, t, call, p).await,
        "secret_set" => secret_set(g, t, call, p).await,
        "secret_delete" => secret_delete(g, t, call, p).await,
        "environment_set" => environment_set(g, t, call, p).await,
        "environment_delete" => environment_delete(call),
        "cache_delete" => cache_delete(g, t, call, p).await,
        "dependabot_alert_update" => alert_update(g, t, call, p, Alerts::Dependabot).await,
        "code_scanning_alert_update" => alert_update(g, t, call, p, Alerts::CodeScanning).await,
        "secret_scanning_alert_update" => alert_update(g, t, call, p, Alerts::SecretScanning).await,
        "star_add" | "star_remove" | "watch_add" | "watch_remove" => repo_account_change(call),
        "follow_add" | "follow_remove" => follow_change(call),
        "notification_mark_read" => notification_mark_read(call),
        "gist_create" => gist_create(call),
        "gist_update" => gist_update(g, t, call, p).await,
        "gist_delete" => gist_delete(g, t, call, p).await,
        "gist_star" => gist_star(call),
        "request_write" => request_write(call),
        _ => return None,
    })
}

async fn workflow_dispatch(gh: &GitHub, token: &str, call: &ConnectorCall, perform: bool) -> Res<Plan> {
    let repo = repo_arg(call)?;
    let w = workflow_arg(call)?;
    let git_ref = match call.str_arg("ref") {
        Some(r) if ref_ok(r) => r.to_owned(),
        _ => return Err(CoreError::service("`ref` must be a valid branch or tag name.")),
    };
    let info = lookup(gh, token, perform, &format!("/repos/{repo}/actions/workflows/{w}")).await?;
    let name = short(s(&info["name"]), 100);
    let mut body = json!({"ref": git_ref});
    let mut plan = Plan::repo(
        Method::POST,
        format!("/repos/{repo}/actions/workflows/{w}/dispatches"),
        &repo,
        format!(
            "Run workflow {} in {repo} on {git_ref}",
            if name.is_empty() {
                call.str_arg("workflow").unwrap_or_default()
            } else {
                &name
            }
        ),
    );
    if let Some(Value::Object(inputs)) =
        call.args.get("inputs").filter(|i| i.as_object().is_some_and(|o| !o.is_empty()))
    {
        body["inputs"] = Value::Object(inputs.clone());
        plan = plan.line("Inputs:");
        for (k, v) in inputs {
            plan = plan.line(format!("  {} = {}", short(k, 60), short(s(v), 300)));
        }
    }
    Ok(plan.body(body).done(&json!({"dispatched": true, "workflow": call.str_arg("workflow"), "ref": git_ref})))
}

async fn workflow_toggle(gh: &GitHub, token: &str, call: &ConnectorCall, perform: bool, verb: &str) -> Res<Plan> {
    let repo = repo_arg(call)?;
    let w = workflow_arg(call)?;
    let info = lookup(gh, token, perform, &format!("/repos/{repo}/actions/workflows/{w}")).await?;
    let name = short(s(&info["name"]), 100);
    let what = if name.is_empty() {
        call.str_arg("workflow").unwrap_or_default().to_owned()
    } else {
        name
    };
    let mut plan = Plan::repo(
        Method::PUT,
        format!("/repos/{repo}/actions/workflows/{w}/{verb}"),
        &repo,
        format!(
            "{} workflow {what} in {repo}",
            if verb == "enable" {
                "Enable"
            } else {
                "Disable"
            }
        ),
    );
    if !info["state"].is_null() {
        plan = plan.line(format!("It is now {}", s(&info["state"])));
    }
    Ok(plan.done(&json!({"workflow": call.str_arg("workflow"), "state": if verb == "enable" { "active" } else { "disabled_manually" }})))
}

async fn run_change(gh: &GitHub, token: &str, call: &ConnectorCall, perform: bool) -> Res<Plan> {
    let repo = repo_arg(call)?;
    let run = id_arg(call, "run_id")?;
    let base = format!("/repos/{repo}/actions/runs/{run}");
    let info = lookup(gh, token, perform, &base).await?;
    let described = format!(
        "run #{} '{}' ({}, {})",
        if info["run_number"].is_null() {
            run.to_string()
        } else {
            info["run_number"].to_string()
        },
        short(
            if s(&info["display_title"]).is_empty() {
                s(&info["name"])
            } else {
                s(&info["display_title"])
            },
            80
        ),
        s(&info["head_branch"]),
        state_of(&info)
    );
    let (method, suffix, verb, once, result) = match call.op.as_str() {
        "run_rerun" => (Method::POST, "/rerun", "Re-run", false, json!({"rerun": true})),
        "run_rerun_failed" => {
            (Method::POST, "/rerun-failed-jobs", "Re-run the failed jobs of", false, json!({"rerun_failed": true}))
        }
        "run_cancel" => (Method::POST, "/cancel", "Cancel", false, json!({"cancel_requested": true})),
        "run_force_cancel" => {
            (Method::POST, "/force-cancel", "Force-cancel", true, json!({"force_cancel_requested": true}))
        }
        "run_approve" => (
            Method::POST,
            "/approve",
            "Approve (and so let it run the code of a fork)",
            true,
            json!({"approved": true}),
        ),
        _ => (Method::DELETE, "", "Delete", false, json!({"deleted": true})),
    };
    let mut plan = Plan::repo(method, format!("{base}{suffix}"), &repo, format!("{verb} {described} in {repo}"));
    if matches!(call.op.as_str(), "run_rerun" | "run_rerun_failed")
        && call.bool_arg("enable_debug_logging") == Some(true)
    {
        plan = plan.body(json!({"enable_debug_logging": true})).line("With debug logging on");
    }
    if once {
        plan = plan.once();
    }
    Ok(plan.done(&result).done(&json!({"run_id": run})))
}

async fn job_rerun(gh: &GitHub, token: &str, call: &ConnectorCall, perform: bool) -> Res<Plan> {
    let repo = repo_arg(call)?;
    let job = id_arg(call, "job_id")?;
    let path = format!("/repos/{repo}/actions/jobs/{job}");
    let info = lookup(gh, token, perform, &path).await?;
    let mut plan = Plan::repo(
        Method::POST,
        format!("{path}/rerun"),
        &repo,
        format!("Re-run job {job} '{}' ({}) in {repo}", short(s(&info["name"]), 80), state_of(&info)),
    );
    if call.bool_arg("enable_debug_logging") == Some(true) {
        plan = plan.body(json!({"enable_debug_logging": true})).line("With debug logging on");
    }
    Ok(plan.done(&json!({"rerun": true, "job_id": job})))
}

async fn artifact_delete(gh: &GitHub, token: &str, call: &ConnectorCall, perform: bool) -> Res<Plan> {
    let repo = repo_arg(call)?;
    let artifact = id_arg(call, "artifact_id")?;
    let path = format!("/repos/{repo}/actions/artifacts/{artifact}");
    let info = lookup(gh, token, perform, &path).await?;
    let plan = Plan::repo(
        Method::DELETE,
        path,
        &repo,
        format!(
            "Delete artifact {artifact} '{}' ({}) of {repo}",
            short(s(&info["name"]), 80),
            size(info["size_in_bytes"].as_i64().unwrap_or(0))
        ),
    );
    Ok(plan.done(&json!({"deleted": true, "artifact_id": artifact})))
}

async fn deployment_review(gh: &GitHub, token: &str, call: &ConnectorCall) -> Res<Plan> {
    let repo = repo_arg(call)?;
    let run = id_arg(call, "run_id")?;
    let state = call.str_arg("state").unwrap_or_default();
    if !matches!(state, "approved" | "rejected") {
        return Err(CoreError::service("`state` must be approved or rejected."));
    }
    let comment = call.str_arg("comment").unwrap_or_default();
    if comment.is_empty() {
        return Err(CoreError::service("`comment` is required."));
    }
    let base = format!("/repos/{repo}/actions/runs/{run}");
    let pending = get(gh, token, &format!("{base}/pending_deployments")).await?;
    let pending = pending.as_array().cloned().unwrap_or_default();
    let mut ids = Vec::new();
    let mut names = Vec::new();
    for wanted in call.list_arg("environments") {
        let found = pending.iter().find(|d| s(&d["environment"]["name"]).eq_ignore_ascii_case(wanted));
        let Some(d) = found else {
            let waiting: Vec<&str> = pending.iter().map(|d| s(&d["environment"]["name"])).collect();
            return Err(CoreError::service(format!(
                "The run is not waiting for '{}'. Waiting: {}.",
                short(wanted, 60),
                if waiting.is_empty() {
                    "nothing".to_owned()
                } else {
                    waiting.join(", ")
                }
            )));
        };
        if d["current_user_can_approve"] != true {
            return Err(CoreError::service(format!(
                "The token's user may not review the deployment to '{}'.",
                short(wanted, 60)
            )));
        }
        ids.push(d["environment"]["id"].clone());
        names.push(short(s(&d["environment"]["name"]), 60));
    }
    if ids.is_empty() {
        return Err(CoreError::service("Name at least one environment."));
    }
    let plan = Plan::repo(
        Method::POST,
        format!("{base}/pending_deployments"),
        &repo,
        format!(
            "{} the deployment of run {run} in {repo} to {}",
            if state == "approved" {
                "Approve"
            } else {
                "Reject"
            },
            names.join(", ")
        ),
    )
    .line(format!("Comment: {}", shown(comment)))
    .body(json!({"environment_ids": ids, "state": state, "comment": comment}))
    .once();
    Ok(plan.done(&json!({"reviewed": true, "state": state, "environments": names})))
}

async fn variable_change(gh: &GitHub, token: &str, call: &ConnectorCall, perform: bool) -> Res<Plan> {
    let repo = repo_arg(call)?;
    let env = env_arg(call);
    let name = var_name(call, call.op == "variable_create")?;
    let base = scoped(&repo, env.as_deref(), "variables");
    let place = scope_label(&repo, env.as_deref());
    let value = call.str_arg("value").unwrap_or_default();
    let done = json!({"name": name, "environment": env});
    let plan = match call.op.as_str() {
        "variable_create" => {
            Plan::repo(Method::POST, base, &repo, format!("Create Actions variable {name} in {place}"))
                .line(format!("Value: {}", shown(value)))
                .body(json!({"name": name, "value": value}))
                .done(&done)
                .done(&json!({"created": true}))
        }
        "variable_update" => {
            let old = lookup(gh, token, perform, &format!("{base}/{name}")).await?;
            Plan::repo(
                Method::PATCH,
                format!("{base}/{name}"),
                &repo,
                format!("Change Actions variable {name} in {place}"),
            )
            .line(format!("Old value: {}", shown(s(&old["value"]))))
            .line(format!("New value: {}", shown(value)))
            .body(json!({"name": name, "value": value}))
            .done(&done)
            .done(&json!({"updated": true}))
        }
        _ => {
            let old = lookup(gh, token, perform, &format!("{base}/{name}")).await?;
            Plan::repo(
                Method::DELETE,
                format!("{base}/{name}"),
                &repo,
                format!("Delete Actions variable {name} from {place}"),
            )
            .line(format!("Current value: {}", shown(s(&old["value"]))))
            .done(&done)
            .done(&json!({"deleted": true}))
        }
    };
    Ok(plan)
}

/// Encrypts `value` for GitHub with the public key `key` (base64): a libsodium sealed box.
fn seal_secret(key: &str, value: &str) -> Res<String> {
    let unusable = || CoreError::service("GitHub sent a public key that cannot be used");
    let raw = BASE64.decode(key.as_bytes()).map_err(|_| unusable())?;
    let bytes: [u8; 32] = raw.try_into().map_err(|_| unusable())?;
    let public = crypto_box::PublicKey::from_bytes(bytes);
    let sealed = public
        .seal(&mut crypto_box::aead::OsRng, value.as_bytes())
        .map_err(|_| CoreError::service("The secret could not be encrypted"))?;
    Ok(BASE64.encode(&sealed))
}

async fn secret_set(gh: &GitHub, token: &str, call: &ConnectorCall, perform: bool) -> Res<Plan> {
    let repo = repo_arg(call)?;
    let env = env_arg(call);
    let name = var_name(call, true)?;
    let value = call.str_arg("value").unwrap_or_default();
    if value.is_empty() {
        return Err(CoreError::service("`value` must not be empty."));
    }
    let base = scoped(&repo, env.as_deref(), "secrets");
    let place = scope_label(&repo, env.as_deref());
    let existing = lookup_opt(gh, token, perform, &format!("{base}/{name}")).await?;
    let mut plan =
        Plan::repo(Method::PUT, format!("{base}/{name}"), &repo, format!("Set Actions secret {name} in {place}"))
            .line(format!(
                "value set ({} characters); it is encrypted on the phone and never shown",
                value.chars().count()
            ))
            .line(match &existing {
                Some(e) => format!("Replaces the existing secret (last changed {})", s(&e["updated_at"])),
                None if perform => String::new(),
                None => "Creates a new secret".to_owned(),
            })
            .once()
            .done(&json!({"set": true, "name": name, "environment": env}));
    plan.lines.retain(|l| !l.is_empty());
    if perform {
        let key = get(gh, token, &format!("{base}/public-key")).await?;
        let (key_id, public) = (s(&key["key_id"]), s(&key["key"]));
        if key_id.is_empty() || public.is_empty() {
            return Err(CoreError::service("GitHub did not send the repository's public key"));
        }
        plan = plan.body(json!({"encrypted_value": seal_secret(public, value)?, "key_id": key_id}));
    }
    Ok(plan)
}

async fn secret_delete(gh: &GitHub, token: &str, call: &ConnectorCall, perform: bool) -> Res<Plan> {
    let repo = repo_arg(call)?;
    let env = env_arg(call);
    let name = var_name(call, false)?;
    let base = scoped(&repo, env.as_deref(), "secrets");
    let existing = lookup_opt(gh, token, perform, &format!("{base}/{name}")).await?;
    let mut plan = Plan::repo(
        Method::DELETE,
        format!("{base}/{name}"),
        &repo,
        format!("Delete Actions secret {name} from {}", scope_label(&repo, env.as_deref())),
    )
    .once()
    .done(&json!({"deleted": true, "name": name, "environment": env}));
    if let Some(e) = existing {
        plan = plan.line(format!("Last changed {}; workflows that use it will fail", s(&e["updated_at"])));
    } else if !perform {
        plan = plan.line("No secret with that name exists");
    }
    Ok(plan)
}

async fn environment_set(gh: &GitHub, token: &str, call: &ConnectorCall, perform: bool) -> Res<Plan> {
    let repo = repo_arg(call)?;
    let env = env_required(call)?;
    let path = format!("/repos/{repo}/environments/{}", segment(&env));
    let existing = lookup_opt(gh, token, perform, &path).await?;
    let mut body = Map::new();
    let mut lines = Vec::new();
    if let Some(n) = call.int_arg("wait_timer") {
        body.insert("wait_timer".to_owned(), json!(n));
        lines.push(format!("Wait timer: {n} minutes"));
    }
    if let Some(b) = call.bool_arg("prevent_self_review") {
        body.insert("prevent_self_review".to_owned(), json!(b));
        lines.push(format!("Prevent self-review: {b}"));
    }
    if let Some(policy) = call.str_arg("branch_policy") {
        let value = match policy {
            "protected" => json!({"protected_branches": true, "custom_branch_policies": false}),
            "custom" => json!({"protected_branches": false, "custom_branch_policies": true}),
            _ => Value::Null,
        };
        body.insert("deployment_branch_policy".to_owned(), value);
        lines.push(format!("Deploy from: {policy} branches"));
    }
    let logins = call.list_arg("reviewers");
    if !logins.is_empty() {
        let mut reviewers = Vec::new();
        for login in &logins {
            if !owner_ok(login) {
                return Err(CoreError::service(format!("'{}' is not a valid GitHub login.", short(login, 40))));
            }
            let u = get(gh, token, &format!("/users/{login}")).await?;
            reviewers.push(json!({"type": "User", "id": u["id"]}));
        }
        body.insert("reviewers".to_owned(), json!(reviewers));
        lines.push(format!("Required reviewers (replacing the current ones): {}", logins.join(", ")));
    }
    let summary = if existing.is_some() || perform {
        format!("Change environment '{}' of {repo}", short(&env, 80))
    } else {
        format!("Create environment '{}' in {repo}", short(&env, 80))
    };
    let mut plan = Plan::repo(Method::PUT, path, &repo, summary)
        .once()
        .body(Value::Object(body))
        .done(&json!({"environment": env, "saved": true}));
    for line in lines {
        plan = plan.line(line);
    }
    if let Some(e) = existing {
        plan = plan
            .line(format!("It has {} protection rule(s) now", e["protection_rules"].as_array().map_or(0, Vec::len)));
    }
    Ok(plan)
}

fn environment_delete(call: &ConnectorCall) -> Res<Plan> {
    let repo = repo_arg(call)?;
    let env = env_required(call)?;
    Ok(Plan::repo(
        Method::DELETE,
        format!("/repos/{repo}/environments/{}", segment(&env)),
        &repo,
        format!("Delete environment '{}' of {repo}, with its secrets and variables", short(&env, 80)),
    )
    .once()
    .done(&json!({"deleted": true, "environment": env})))
}

async fn cache_delete(gh: &GitHub, token: &str, call: &ConnectorCall, perform: bool) -> Res<Plan> {
    let repo = repo_arg(call)?;
    let base = format!("/repos/{repo}/actions/caches");
    match (call.int_arg("cache_id"), call.str_arg("key")) {
        (Some(id), None) if id > 0 => Ok(Plan::repo(
            Method::DELETE,
            format!("{base}/{id}"),
            &repo,
            format!("Delete Actions cache {id} of {repo}"),
        )
        .done(&json!({"deleted": true, "cache_id": id}))),
        (None, Some(key)) => {
            let mut query = vec![("key", key.to_owned())];
            if let Some(r) = call.str_arg("ref") {
                query.push(("ref", r.to_owned()));
            }
            let scope = call.str_arg("ref").map(|r| format!(" on {}", short(r, 80))).unwrap_or_default();
            let mut plan = Plan::repo(
                Method::DELETE,
                base.clone(),
                &repo,
                format!("Delete every Actions cache with key '{}'{scope} in {repo}", short(key, 100)),
            );
            if !perform {
                let mut q = query.clone();
                q.push(("per_page", "100".to_owned()));
                let found = gh.call(token, Method::GET, &base, &q, None).await?;
                let count = found["actions_caches"].as_array().map_or(0, Vec::len);
                plan = plan.line(format!("{count} cache(s) match now"));
            }
            plan.query = query.into_iter().map(|(k, v)| (k.to_owned(), v)).collect();
            Ok(plan.done(&json!({"deleted": true, "key": key})).pick(&["total_count"]))
        }
        _ => Err(CoreError::service("Give either `cache_id` or `key`.")),
    }
}

async fn alert_update(gh: &GitHub, token: &str, call: &ConnectorCall, perform: bool, kind: Alerts) -> Res<Plan> {
    let repo = repo_arg(call)?;
    let number = id_arg(call, "number")?;
    let state = call.str_arg("state").unwrap_or_default();
    let (closed, reason_arg) = match kind {
        Alerts::Dependabot | Alerts::CodeScanning => ("dismissed", "reason"),
        Alerts::SecretScanning => ("resolved", "resolution"),
    };
    if state != "open" && state != closed {
        return Err(CoreError::service(format!("`state` must be open or {closed}.")));
    }
    let reason = call.str_arg(reason_arg);
    let comment = call.str_arg("comment");
    if state == closed && reason.is_none() {
        return Err(CoreError::service(format!("`{reason_arg}` is required to make an alert {closed}.")));
    }
    if state == "open" && (reason.is_some() || comment.is_some()) {
        return Err(CoreError::service(format!("`{reason_arg}` and `comment` only go with {closed}.")));
    }
    if comment.is_some_and(|c| c.chars().count() > 280) {
        return Err(CoreError::service("`comment` may have at most 280 characters."));
    }
    let path = format!("/repos/{repo}/{}/{number}", kind.path());
    let info = lookup(gh, token, perform, &path).await?;
    let what = match kind {
        Alerts::Dependabot => short(s(&info["security_advisory"]["summary"]), 100),
        Alerts::CodeScanning => short(s(&info["rule"]["description"]), 100),
        // The alert carries the secret itself: only the kind of secret is named.
        Alerts::SecretScanning => short(s(&info["secret_type_display_name"]), 100),
    };
    let mut body = Map::new();
    body.insert("state".to_owned(), json!(state));
    if let Some(r) = reason {
        body.insert(
            if kind == Alerts::SecretScanning {
                "resolution"
            } else {
                "dismissed_reason"
            }
            .to_owned(),
            json!(r),
        );
    }
    if let Some(c) = comment {
        body.insert(
            if kind == Alerts::SecretScanning {
                "resolution_comment"
            } else {
                "dismissed_comment"
            }
            .to_owned(),
            json!(c),
        );
    }
    let mut plan = Plan::repo(
        Method::PATCH,
        path,
        &repo,
        format!(
            "Mark alert #{number} of {repo}{} as {state}",
            if what.is_empty() {
                String::new()
            } else {
                format!(" ({what})")
            }
        ),
    )
    .body(Value::Object(body))
    .done(&json!({"updated": true, "number": number, "state": state}));
    if let Some(r) = reason {
        plan = plan.line(format!("Reason: {r}"));
    }
    if let Some(c) = comment {
        plan = plan.line(format!("Comment: {c}"));
    }
    if !info["state"].is_null() {
        plan = plan.line(format!("It is now {}", s(&info["state"])));
    }
    Ok(plan)
}

fn repo_account_change(call: &ConnectorCall) -> Res<Plan> {
    let repo = repo_arg(call)?;
    Ok(match call.op.as_str() {
        "star_add" => Plan::account(Method::PUT, format!("/user/starred/{repo}"), format!("Star {repo}"))
            .done(&json!({"starred": true, "repo": repo})),
        "star_remove" => {
            Plan::account(Method::DELETE, format!("/user/starred/{repo}"), format!("Remove your star from {repo}"))
                .done(&json!({"starred": false, "repo": repo}))
        }
        "watch_add" => {
            let ignored = call.bool_arg("ignored") == Some(true);
            Plan::account(
                Method::PUT,
                format!("/repos/{repo}/subscription"),
                if ignored {
                    format!("Ignore all notifications of {repo}")
                } else {
                    format!("Watch {repo}")
                },
            )
            .body(json!({"subscribed": !ignored, "ignored": ignored}))
            .done(&json!({"watching": !ignored, "ignored": ignored, "repo": repo}))
        }
        _ => Plan::account(Method::DELETE, format!("/repos/{repo}/subscription"), format!("Stop watching {repo}"))
            .done(&json!({"watching": false, "repo": repo})),
    })
}

fn follow_change(call: &ConnectorCall) -> Res<Plan> {
    let user = name_arg(call, "username")?;
    Ok(if call.op == "follow_add" {
        Plan::account(Method::PUT, format!("/user/following/{user}"), format!("Follow {user}"))
            .done(&json!({"following": true, "user": user}))
    } else {
        Plan::account(Method::DELETE, format!("/user/following/{user}"), format!("Unfollow {user}"))
            .done(&json!({"following": false, "user": user}))
    })
}

fn notification_mark_read(call: &ConnectorCall) -> Res<Plan> {
    if let Some(thread) = call.str_arg("thread_id") {
        if thread.is_empty() || thread.len() > 30 || !thread.bytes().all(|b| b.is_ascii_digit()) {
            return Err(CoreError::service("`thread_id` is not a valid notification thread id."));
        }
        return Ok(Plan::account(
            Method::PATCH,
            format!("/notifications/threads/{thread}"),
            format!("Mark notification thread {thread} as read"),
        )
        .done(&json!({"marked_read": true, "thread_id": thread})));
    }
    let (path, what) = if call.str_arg("repo").is_some() {
        let repo = repo_arg(call)?;
        (format!("/repos/{repo}/notifications"), format!("Mark all notifications of {repo} as read"))
    } else {
        ("/notifications".to_owned(), "Mark all your GitHub notifications as read".to_owned())
    };
    Ok(Plan::account(Method::PUT, path, what).body(json!({"read": true})).done(&json!({"marked_read": true})))
}

/// The `files` argument of a gist: name to text (or, when `allow_delete`, to null).
fn gist_files(call: &ConnectorCall, allow_delete: bool) -> Res<Vec<(String, Option<String>)>> {
    let Some(files) = call.args.get("files") else {
        return Ok(Vec::new());
    };
    let map = files.as_object().ok_or_else(|| CoreError::service("`files` must be an object of file name to text."))?;
    if map.len() > 50 {
        return Err(CoreError::service("A gist may have at most 50 files."));
    }
    let mut out = Vec::new();
    for (name, content) in map {
        let name_ok = !name.trim().is_empty()
            && name.len() <= 255
            && !name.contains(['/', '\\'])
            && !name.chars().any(char::is_control);
        if !name_ok {
            return Err(CoreError::service(format!("'{}' is not a valid file name.", short(name, 40))));
        }
        match content {
            Value::String(c) if !c.is_empty() && c.len() <= 300_000 => out.push((name.clone(), Some(c.clone()))),
            Value::Null if allow_delete => out.push((name.clone(), None)),
            _ => {
                return Err(CoreError::service(format!(
                    "File '{}' needs text (not empty, at most 300000 bytes).",
                    short(name, 40)
                )));
            }
        }
    }
    Ok(out)
}

fn gist_files_body(files: &[(String, Option<String>)]) -> Value {
    Value::Object(
        files
            .iter()
            .map(|(name, content)| (name.clone(), content.as_ref().map_or(Value::Null, |c| json!({"content": c}))))
            .collect(),
    )
}

fn gist_file_lines(mut plan: Plan, files: &[(String, Option<String>)]) -> Plan {
    for (name, content) in files {
        plan = match content {
            Some(c) => plan.line(format!(
                "File {}: {} characters\n{}",
                short(name, 80),
                c.chars().count(),
                text::truncate_chars(c, 300)
            )),
            None => plan.line(format!("Delete file {}", short(name, 80))),
        };
    }
    plan
}

fn gist_create(call: &ConnectorCall) -> Res<Plan> {
    let files = gist_files(call, false)?;
    if files.is_empty() {
        return Err(CoreError::service("`files` needs at least one file."));
    }
    let public = call.bool_arg("public") == Some(true);
    let description = call.str_arg("description").unwrap_or_default();
    let plan = Plan::account(
        Method::POST,
        "/gists".to_owned(),
        format!(
            "Create a {} gist with {} file(s)",
            if public {
                "PUBLIC"
            } else {
                "secret"
            },
            files.len()
        ),
    );
    let plan = if description.is_empty() {
        plan
    } else {
        plan.line(format!("Description: {}", shown(description)))
    };
    Ok(gist_file_lines(plan, &files)
        .body(json!({"description": description, "public": public, "files": gist_files_body(&files)}))
        .pick(&["id", "html_url"])
        .done(&json!({"created": true, "public": public})))
}

async fn gist_update(gh: &GitHub, token: &str, call: &ConnectorCall, perform: bool) -> Res<Plan> {
    let id = gist_arg(call)?;
    let files = gist_files(call, true)?;
    let description = call.str_arg("description");
    if files.is_empty() && description.is_none() {
        return Err(CoreError::service("Give a new `description` or some `files` to change."));
    }
    let info = lookup(gh, token, perform, &format!("/gists/{id}")).await?;
    let mut body = Map::new();
    if let Some(d) = description {
        body.insert("description".to_owned(), json!(d));
    }
    if !files.is_empty() {
        body.insert("files".to_owned(), gist_files_body(&files));
    }
    let mut plan = Plan::account(
        Method::PATCH,
        format!("/gists/{id}"),
        format!("Change gist {id} '{}'", short(s(&info["description"]), 80)),
    );
    if let Some(d) = description {
        plan = plan.line(format!("New description: {}", shown(d)));
    }
    Ok(gist_file_lines(plan, &files)
        .body(Value::Object(body))
        .pick(&["html_url"])
        .done(&json!({"updated": true, "gist_id": id})))
}

async fn gist_delete(gh: &GitHub, token: &str, call: &ConnectorCall, perform: bool) -> Res<Plan> {
    let id = gist_arg(call)?;
    let info = lookup(gh, token, perform, &format!("/gists/{id}")).await?;
    let names: Vec<&str> = info["files"].as_object().into_iter().flatten().map(|(n, _)| n.as_str()).collect();
    let mut plan = Plan::account(
        Method::DELETE,
        format!("/gists/{id}"),
        format!("Delete gist {id} '{}' for good", short(s(&info["description"]), 80)),
    )
    .done(&json!({"deleted": true, "gist_id": id}));
    if !names.is_empty() {
        plan = plan.line(format!("Files: {}", short(&names.join(", "), 200)));
    }
    Ok(plan)
}

fn gist_star(call: &ConnectorCall) -> Res<Plan> {
    let id = gist_arg(call)?;
    Ok(if call.bool_arg("unstar") == Some(true) {
        Plan::account(Method::DELETE, format!("/gists/{id}/star"), format!("Remove your star from gist {id}"))
            .done(&json!({"starred": false, "gist_id": id}))
    } else {
        Plan::account(Method::PUT, format!("/gists/{id}/star"), format!("Star gist {id}"))
            .done(&json!({"starred": true, "gist_id": id}))
    })
}

fn request_write(call: &ConnectorCall) -> Res<Plan> {
    let (path, repo) = request_path(call)?;
    let method = match call.str_arg("method").unwrap_or_default() {
        "post" => Method::POST,
        "put" => Method::PUT,
        "patch" => Method::PATCH,
        "delete" => Method::DELETE,
        _ => return Err(CoreError::service("`method` must be post, put, patch or delete.")),
    };
    let query = request_query(call)?;
    let summary = format!("Send {method} {} to the GitHub API", short(&path, 300));
    let mut plan = match &repo {
        Some(r) => Plan::repo(method.clone(), path.clone(), r, summary),
        None => Plan::account(method.clone(), path.clone(), summary),
    }
    .once();
    for (k, v) in &query {
        plan = plan.line(format!("Query {} = {}", short(k, 60), short(v, 300)));
    }
    if let Some(body) = call.args.get("body").filter(|b| !b.is_null()) {
        plan = plan
            .line(format!("Body:\n{}", shown(&serde_json::to_string_pretty(body).unwrap_or_default())))
            .body(body.clone());
    }
    plan.query = query;
    Ok(plan.done(&json!({"sent": true, "method": method.as_str(), "path": path})))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_request_paths_are_allowed_only_under_the_known_roots() {
        assert_eq!(request_path_ok("/repos/octo/cat/issues").unwrap().as_deref(), Some("octo/cat"));
        assert_eq!(request_path_ok("/repos/octo/cat").unwrap().as_deref(), Some("octo/cat"));
        assert_eq!(request_path_ok("/repos/octo/cat/contents/dir%20x/a.txt").unwrap().as_deref(), Some("octo/cat"));
        for ok in [
            "/user",
            "/user/starred",
            "/users/octo",
            "/orgs/acme/teams",
            "/search/code",
            "/gists",
            "/gists/public",
            "/notifications",
            "/rate_limit",
            "/user/orgs",
        ] {
            assert_eq!(request_path_ok(ok), Ok(None), "{ok}");
        }
    }

    #[test]
    fn raw_request_paths_cannot_traverse_smuggle_or_reach_credentials() {
        for bad in [
            "",
            "repos/octo/cat",
            "/",
            "//repos/octo/cat",
            "/repos",
            "/repos/octo",
            "/repos/octo/",
            "/repos/octo/cat/",
            "/repos/octo/cat/../../../user",
            "/repos/octo/cat/./issues",
            "/repos/octo/cat//issues",
            "/repos/../user",
            "/repos/octo/cat/%2e%2e/x",
            "/repos/octo/cat/%2E%2E/x",
            "/repos/octo/cat/%2f",
            "/repos/octo/cat/a%2Fb",
            "/repos/octo/cat/a%5Cb",
            "/repos/octo/cat/a\\b",
            "/repos/octo/cat/a%3fb",
            "/repos/octo/cat/a%23b",
            "/repos/octo/cat/a%252fb",
            "/repos/octo/cat/a%00b",
            "/repos/octo/cat/a%0ab",
            "/repos/octo/cat/a%7f",
            "/repos/octo/cat/%zz",
            "/repos/octo/cat/%2",
            "/repos/octo/cat/%",
            "/repos/octo/cat/issues?state=all",
            "/repos/octo/cat/issues#x",
            "/repos/octo/cat/issues?",
            "/repos/octo/cat/a b",
            "/repos/octo/cat/a\nb",
            "/repos/octo/cat/\u{e9}",
            "/repos/oc%74o/cat",
            "/repos/octo/ca%74",
            "/repos/octo/cat/a;b",
            "/repos/octo/cat/a<b",
            "/graphql",
            "/app",
            "/app/installations",
            "/apps/x",
            "/authorizations",
            "/applications/x/token",
            "/user/keys",
            "/user/keys/1",
            "/user/gpg_keys",
            "/user/emails",
            "/user/public_emails",
            "/user/tokens",
            "/user/ssh_signing_keys",
            "/user/installations",
            "/user/codespaces/secrets",
            "/User/Keys",
            "/USER/emails",
            "/users",
            "/orgs",
            "/search",
            "/userx",
            "/usersx/octo",
            "/repositories",
            "/enterprises/x",
            "/admin/users",
            "/authorizations/1",
            "/marketplace_listing",
            "/gists/../user/emails",
            "/users/octo/../../user/keys",
        ] {
            assert!(request_path_ok(bad).is_err(), "{bad:?} must be refused");
        }
        assert!(request_path_ok(&format!("/repos/octo/cat/{}", "a".repeat(1_000))).is_err());
    }

    #[test]
    fn logs_keep_the_end_from_a_line_start_and_lose_terminal_colours() {
        let raw = "2026-01-01T00:00:00Z \u{1b}[36;1mstep one\u{1b}[0m\nline two\r\nline three\n";
        assert_eq!(
            log_tail(raw, None, 1_000),
            ("2026-01-01T00:00:00Z step one\nline two\nline three\n".to_owned(), false)
        );
        let (tail, cut) = log_tail(raw, Some(2), 1_000);
        assert_eq!((tail.as_str(), cut), ("line two\nline three", true));
        let long = format!("{}\nshort tail\n", "x".repeat(100));
        let (tail, cut) = log_tail(&long, None, 20);
        assert_eq!((tail.as_str(), cut), ("short tail\n", true));
    }

    #[test]
    fn redirects_may_lead_to_the_api_host_or_a_public_https_host_only() {
        let base = "https://api.github.com";
        assert!(location_ok(base, "https://pipelines.actions.githubusercontent.com/x?sig=1").is_ok());
        assert!(location_ok(base, "/logs/1").is_ok());
        for bad in [
            "http://evil.example/x",
            "https://127.0.0.1/x",
            "https://localhost/x",
            "https://user:pw@evil.example/x",
            "https://[::1]/x",
            "https://printer.local/x",
            "ftp://evil.example/x",
            "file:///etc/passwd",
        ] {
            assert!(location_ok(base, bad).is_err(), "{bad}");
        }
        assert!(location_ok("http://127.0.0.1:9000", "http://127.0.0.1:9000/blob").is_ok());
        assert!(location_ok("http://127.0.0.1:9000", "http://127.0.0.1:9001/blob").is_err());
    }

    #[test]
    fn workflow_and_variable_names_are_checked() {
        for ok in ["123", "ci.yml", "build-and_test.yaml"] {
            assert!(workflow_ok(ok), "{ok}");
        }
        for bad in ["", "ci", "../ci.yml", "a/b.yml", "ci.yml?x", "a b.yml", "..yml", "a..b.yml"] {
            assert!(!workflow_ok(bad), "{bad}");
        }
    }

    #[test]
    fn a_secret_is_sealed_for_the_key_and_a_bad_key_is_refused() {
        let secret_key = crypto_box::SecretKey::generate(&mut crypto_box::aead::OsRng);
        let key = BASE64.encode(secret_key.public_key().as_bytes());
        let sealed = seal_secret(&key, "hunter2").unwrap();
        let raw = BASE64.decode(sealed.as_bytes()).unwrap();
        assert_eq!(secret_key.unseal(&raw).unwrap(), b"hunter2");
        assert!(seal_secret("not base64!", "x").is_err());
        assert!(seal_secret(&BASE64.encode(&[1u8; 31]), "x").is_err());
    }
}
