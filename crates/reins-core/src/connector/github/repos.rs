//! GitHub: repositories, branches, and repository settings and access.

use std::fmt::{Display, Write as _};

use data_encoding::BASE64;
use reins_proto::connector::ConnectorCall;
use reqwest::Method;
use serde_json::{Map, Value, json};

use super::{GitHub, Options, Preview, owner_ok, parents, ref_arg, repo_arg, repo_ok, resource, resource_label};
use crate::connector::Item;
use crate::connector::calendar::{limit, segment};
use crate::{CoreError, text};

/// The most characters of a README handed over.
const MAX_README: usize = 100_000;
/// The most characters of AI-supplied text repeated in a preview.
const MAX_SHOWN: usize = 1_500;

/// Lists and reads. `None` when the operation is not one of this area's.
pub(super) async fn fetch(gh: &GitHub, token: &str, call: &ConnectorCall) -> Option<Result<Vec<Item>, CoreError>> {
    Some(match call.op.as_str() {
        "list_repos" => list_repos(gh, token, call).await,
        "org_repo_list" => org_repo_list(gh, token, call).await,
        "repo_get" => repo_get(gh, token, call).await,
        "repo_forks_list" => repo_forks_list(gh, token, call).await,
        "repo_languages" => repo_languages(gh, token, call).await,
        "repo_contributors" => repo_contributors(gh, token, call).await,
        "repo_topics_get" => repo_topics_get(gh, token, call).await,
        "repo_readme_get" => repo_readme_get(gh, token, call).await,
        "branch_list" => branch_list(gh, token, call).await,
        "branch_get" => branch_get(gh, token, call).await,
        "branch_protection_get" => branch_protection_get(gh, token, call).await,
        "ruleset_list" => ruleset_list(gh, token, call).await,
        "ruleset_get" => ruleset_get(gh, token, call).await,
        "collaborator_list" => collaborator_list(gh, token, call).await,
        "invitation_list" => invitation_list(gh, token, call).await,
        "repo_team_list" => repo_team_list(gh, token, call).await,
        "webhook_list" => webhook_list(gh, token, call).await,
        "webhook_get" => webhook_get(gh, token, call).await,
        "deploy_key_list" => deploy_key_list(gh, token, call).await,
        "traffic_clones" | "traffic_views" => traffic(gh, token, call).await,
        "commit_activity" => commit_activity(gh, token, call).await,
        _ => return None,
    })
}

/// What a write would do. `None` when the operation is not one of this area's.
pub(super) async fn preview(gh: &GitHub, token: &str, call: &ConnectorCall) -> Option<Result<Preview, CoreError>> {
    Some(match call.op.as_str() {
        "repo_create" => preview_repo_create(gh, token, call).await,
        "repo_update" => preview_repo_update(gh, token, call).await,
        "repo_fork" => preview_repo_fork(gh, token, call).await,
        "repo_delete" => preview_repo_delete(gh, token, call).await,
        "repo_transfer" => preview_repo_transfer(gh, token, call).await,
        "repo_visibility_set" => preview_visibility(gh, token, call).await,
        "repo_topics_set" => preview_topics_set(gh, token, call).await,
        "branch_create" => preview_branch_create(gh, token, call).await,
        "branch_delete" => preview_branch_delete(gh, token, call).await,
        "branch_rename" => preview_branch_rename(gh, token, call).await,
        "branch_merge" => preview_branch_merge(gh, token, call).await,
        "branch_protection_set" => preview_protection_set(gh, token, call).await,
        "branch_protection_delete" => preview_protection_delete(gh, token, call).await,
        "collaborator_add" => preview_collaborator_add(gh, token, call).await,
        "collaborator_remove" => preview_collaborator_remove(gh, token, call).await,
        "invitation_cancel" => preview_invitation_cancel(gh, token, call).await,
        "repo_team_add" => preview_team(gh, token, call, true).await,
        "repo_team_remove" => preview_team(gh, token, call, false).await,
        "webhook_create" => preview_webhook_create(call),
        "webhook_update" => preview_webhook_update(gh, token, call).await,
        "webhook_delete" => preview_webhook_touch(gh, token, call, "Delete").await,
        "webhook_ping" => preview_webhook_touch(gh, token, call, "Send a test delivery to").await,
        "deploy_key_add" => preview_deploy_key_add(call),
        "deploy_key_delete" => preview_deploy_key_delete(gh, token, call).await,
        _ => return None,
    })
}

/// Does the write. `None` when the operation is not one of this area's.
pub(super) async fn perform(gh: &GitHub, token: &str, call: &ConnectorCall) -> Option<Result<Value, CoreError>> {
    Some(match call.op.as_str() {
        "repo_create" => perform_repo_create(gh, token, call).await,
        "repo_update" => perform_repo_update(gh, token, call).await,
        "repo_fork" => perform_repo_fork(gh, token, call).await,
        "repo_delete" => perform_repo_delete(gh, token, call).await,
        "repo_transfer" => perform_repo_transfer(gh, token, call).await,
        "repo_visibility_set" => perform_visibility(gh, token, call).await,
        "repo_topics_set" => perform_topics_set(gh, token, call).await,
        "branch_create" => perform_branch_create(gh, token, call).await,
        "branch_delete" => perform_branch_delete(gh, token, call).await,
        "branch_rename" => perform_branch_rename(gh, token, call).await,
        "branch_merge" => perform_branch_merge(gh, token, call).await,
        "branch_protection_set" => perform_protection_set(gh, token, call).await,
        "branch_protection_delete" => perform_protection_delete(gh, token, call).await,
        "collaborator_add" => perform_collaborator_add(gh, token, call).await,
        "collaborator_remove" => perform_collaborator_remove(gh, token, call).await,
        "invitation_cancel" => perform_invitation_cancel(gh, token, call).await,
        "repo_team_add" => perform_team(gh, token, call, true).await,
        "repo_team_remove" => perform_team(gh, token, call, false).await,
        "webhook_create" => perform_webhook_create(gh, token, call).await,
        "webhook_update" => perform_webhook_update(gh, token, call).await,
        "webhook_delete" => perform_webhook_delete(gh, token, call).await,
        "webhook_ping" => perform_webhook_ping(gh, token, call).await,
        "deploy_key_add" => perform_deploy_key_add(gh, token, call).await,
        "deploy_key_delete" => perform_deploy_key_delete(gh, token, call).await,
        _ => return None,
    })
}

// ---- helpers -------------------------------------------------------------------------------------------------------

fn fail(message: impl Into<String>) -> CoreError {
    CoreError::service(message)
}

/// A string field of an API object ("" when missing).
fn s<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or_default()
}

fn count(v: &Value, key: &str) -> i64 {
    v[key].as_i64().unwrap_or(0)
}

fn short(sha: &str) -> String {
    sha.chars().take(7).collect()
}

/// The first line of a commit message, for one display line.
fn subject(message: &str) -> String {
    text::truncate_chars(&text::one_line(message.lines().next().unwrap_or_default()), 120)
}

/// AI-supplied text repeated for the user, cut when long.
fn shown(value: &str) -> String {
    let cut = text::truncate_chars(value, MAX_SHOWN);
    if cut.len() < value.len() {
        format!("{cut} [shortened]")
    } else {
        cut
    }
}

fn object(v: Value) -> Map<String, Value> {
    match v {
        Value::Object(m) => m,
        _ => Map::new(),
    }
}

fn strings(v: &Value) -> Vec<&str> {
    v.as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default()
}

/// A required login or organization name argument.
fn login_arg(call: &ConnectorCall, name: &str) -> Result<String, CoreError> {
    match call.str_arg(name) {
        Some(v) if owner_ok(v) => Ok(v.to_owned()),
        Some(_) => Err(fail(format!("`{name}` is not a valid GitHub user or organization name."))),
        None => Err(fail(format!("`{name}` is required."))),
    }
}

fn opt_login(call: &ConnectorCall, name: &str) -> Result<Option<String>, CoreError> {
    call.str_arg(name).map(|_| login_arg(call, name)).transpose()
}

/// A required branch/tag/ref argument.
fn ref_req(call: &ConnectorCall, name: &str) -> Result<String, CoreError> {
    ref_arg(call, name)?.ok_or_else(|| fail(format!("`{name}` is required.")))
}

/// A positive number argument (an id).
fn id_arg(call: &ConnectorCall, name: &str) -> Result<i64, CoreError> {
    call.int_arg(name).filter(|n| *n > 0).ok_or_else(|| fail(format!("`{name}` must be a positive number.")))
}

/// A ref name as the tail of a `git/ref/heads/...` path: every part encoded, the slashes kept.
fn ref_path(name: &str) -> String {
    name.split('/').map(segment).collect::<Vec<_>>().join("/")
}

fn repo_preview(repo: &str, lines: Vec<String>) -> Preview {
    Preview {
        resource: resource(repo, None),
        resource_label: resource_label(repo, None),
        lines,
        parents: parents(repo, None),
        once_only: false,
        ..Preview::default()
    }
}

fn branch_preview(repo: &str, branch: &str, lines: Vec<String>) -> Preview {
    Preview {
        resource: resource(repo, Some(branch)),
        resource_label: resource_label(repo, Some(branch)),
        lines,
        parents: parents(repo, Some(branch)),
        once_only: false,
        ..Preview::default()
    }
}

/// A row of a repository-level listing or read.
fn repo_row(repo: &str, id: String, title: String) -> Item {
    Item {
        id,
        resource: resource(repo, None),
        resource_label: resource_label(repo, None),
        title,
        parents: parents(repo, None),
        ..Item::default()
    }
}

fn branch_row(repo: &str, branch: &str) -> Item {
    Item {
        id: branch.to_owned(),
        resource: resource(repo, Some(branch)),
        resource_label: resource_label(repo, Some(branch)),
        title: text::one_line(branch),
        parents: parents(repo, Some(branch)),
        ..Item::default()
    }
}

async fn get(gh: &GitHub, token: &str, path: &str) -> Result<Value, CoreError> {
    gh.call(token, Method::GET, path, &[], None).await
}

/// A read whose 404 is an answer: `None` when it does not exist.
async fn get_opt(gh: &GitHub, token: &str, path: &str) -> Result<Option<Value>, CoreError> {
    let options = Options {
        allow: &[404],
        ..Options::default()
    };
    let reply = gh.send(token, Method::GET, path, &[], None, &options).await?;
    Ok((reply.status != 404).then_some(reply.json))
}

/// The repository, the state every preview of a repository-wide change starts from.
async fn repo_state(gh: &GitHub, token: &str, repo: &str) -> Result<Value, CoreError> {
    get(gh, token, &format!("/repos/{repo}")).await
}

fn visibility_of(r: &Value) -> &str {
    if r["private"].as_bool() == Some(true) {
        "private"
    } else if r["visibility"].as_str() == Some("internal") {
        "internal"
    } else {
        "public"
    }
}

fn change(lines: &mut Vec<String>, label: &str, old: impl Display, new: impl Display) {
    lines.push(format!("{label}: {old} \u{2192} {new}"));
}

fn quoted(v: &str) -> String {
    format!("'{}'", text::one_line(v))
}

fn on_off(b: bool) -> &'static str {
    if b {
        "on"
    } else {
        "off"
    }
}

fn date_of(v: &Value, key: &str) -> i64 {
    v[key].as_str().and_then(text::parse_when).unwrap_or(0)
}

/// An address for display: scheme and host only, since the path of a webhook address may carry a secret.
fn redact_url(raw: &str) -> String {
    let Ok(u) = url::Url::parse(raw) else {
        return "(unreadable address)".to_owned();
    };
    let Some(host) = u.host_str() else {
        return "(unreadable address)".to_owned();
    };
    let port = u.port().map(|p| format!(":{p}")).unwrap_or_default();
    let more = if u.path() == "/" && u.query().is_none() {
        ""
    } else {
        "/\u{2026}"
    };
    format!("{}://{host}{port}{more}", u.scheme())
}

/// A webhook address the AI gave: https, with a host and no credentials inside.
fn https_url(raw: &str) -> Result<String, CoreError> {
    let bad = || fail("`url` must be an https address such as https://example.com/hook.");
    let u = url::Url::parse(raw).map_err(|_| bad())?;
    if u.scheme() != "https" || u.host_str().is_none() || !u.username().is_empty() || u.password().is_some() {
        return Err(bad());
    }
    Ok(raw.to_owned())
}

fn events_arg(call: &ConnectorCall) -> Result<Option<Vec<String>>, CoreError> {
    if !call.args.contains_key("events") {
        return Ok(None);
    }
    let events = call.list_arg("events");
    if events.is_empty() {
        return Err(fail("`events` needs at least one event name."));
    }
    for e in &events {
        if *e != "*" && !e.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_') {
            return Err(fail(format!("`{}` is not an event name.", text::one_line(e))));
        }
    }
    Ok(Some(events.into_iter().map(str::to_owned).collect()))
}

// ---- repositories: reads ---------------------------------------------------------------------------------------------

fn repo_item(r: &Value) -> Item {
    let name = s(r, "full_name");
    let mut item = repo_row(name, name.to_owned(), name.to_owned());
    visibility_of(r).clone_into(&mut item.from);
    item.snippet = text::one_line(s(r, "description"));
    item.date = date_of(r, "pushed_at");
    item.extra = object(json!({
        "private": r["private"], "fork": r["fork"], "archived": r["archived"],
        "default_branch": r["default_branch"], "language": r["language"],
        "stars": r["stargazers_count"], "url": r["html_url"],
    }));
    item
}

fn filtered_repos(repos: &[Value], call: &ConnectorCall, visibility: Option<&str>) -> Vec<Item> {
    let query = call.str_arg("query").map(str::to_lowercase);
    repos
        .iter()
        .filter(|r| query.as_deref().is_none_or(|q| s(r, "full_name").to_lowercase().contains(q)))
        .filter(|r| match visibility {
            Some("private") => r["private"].as_bool() == Some(true),
            Some("public") => r["private"].as_bool() != Some(true),
            _ => true,
        })
        .take(limit(call))
        .map(repo_item)
        .collect()
}

async fn list_repos(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
    let org = opt_login(call, "org")?;
    let owner = opt_login(call, "owner")?;
    if org.is_some() && owner.is_some() {
        return Err(fail("Give `org` or `owner`, not both."));
    }
    let visibility = call.str_arg("visibility").filter(|v| *v != "all");
    let mut query = vec![("per_page", "100".to_owned()), ("sort", "pushed".to_owned())];
    let path = if let Some(org) = &org {
        if let Some(v) = visibility {
            query.push(("type", v.to_owned()));
        }
        format!("/orgs/{org}/repos")
    } else if let Some(owner) = &owner {
        format!("/users/{owner}/repos")
    } else {
        if let Some(v) = visibility {
            query.push(("visibility", v.to_owned()));
        }
        if let Some(a) = call.str_arg("affiliation") {
            query.push(("affiliation", a.to_owned()));
        }
        "/user/repos".to_owned()
    };
    let filtering = call.str_arg("query").is_some() || visibility.is_some();
    let max = if filtering {
        500
    } else {
        limit(call)
    };
    let repos = gh.pages(token, &path, &query, None, max).await?;
    Ok(filtered_repos(&repos, call, visibility))
}

async fn org_repo_list(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
    let org = login_arg(call, "org")?;
    let query = [
        ("per_page", "100".to_owned()),
        ("sort", "pushed".to_owned()),
        ("type", call.str_arg("type").unwrap_or("all").to_owned()),
    ];
    let max = if call.str_arg("query").is_some() {
        500
    } else {
        limit(call)
    };
    let repos = gh.pages(token, &format!("/orgs/{org}/repos"), &query, None, max).await?;
    Ok(filtered_repos(&repos, call, None))
}

fn features(r: &Value) -> Value {
    json!({
        "issues": r["has_issues"], "projects": r["has_projects"], "wiki": r["has_wiki"], "pages": r["has_pages"],
        "discussions": r["has_discussions"], "downloads": r["has_downloads"],
        "merge_commit": r["allow_merge_commit"], "squash_merge": r["allow_squash_merge"],
        "rebase_merge": r["allow_rebase_merge"], "auto_merge": r["allow_auto_merge"],
        "delete_branch_on_merge": r["delete_branch_on_merge"],
    })
}

/// One line per language, biggest first, with its share.
#[allow(clippy::cast_precision_loss, reason = "byte counts shown as a rounded percentage")]
fn languages_text(languages: &Value) -> String {
    let mut rows: Vec<(&String, i64)> = languages
        .as_object()
        .map(|m| m.iter().map(|(k, v)| (k, v.as_i64().unwrap_or(0))).collect())
        .unwrap_or_default();
    rows.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    let total: i64 = rows.iter().map(|(_, n)| n).sum();
    rows.iter()
        .map(|(name, n)| {
            let percent = if total > 0 {
                *n as f64 * 100.0 / total as f64
            } else {
                0.0
            };
            format!("{}: {percent:.1}% ({n} bytes)", text::one_line(name))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

async fn repo_get(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
    let repo = repo_arg(call)?;
    let r = repo_state(gh, token, &repo).await?;
    let languages = get(gh, token, &format!("/repos/{repo}/languages")).await?;
    let mut lines = vec![format!("{} ({})", s(&r, "full_name"), visibility_of(&r))];
    for (label, key) in [("Description", "description"), ("Homepage", "homepage")] {
        if !s(&r, key).is_empty() {
            lines.push(format!("{label}: {}", text::one_line(s(&r, key))));
        }
    }
    lines.push(format!("Default branch: {}", text::one_line(s(&r, "default_branch"))));
    let topics = strings(&r["topics"]).join(", ");
    if !topics.is_empty() {
        lines.push(format!("Topics: {}", text::one_line(&topics)));
    }
    if let Some(parent) = r["parent"]["full_name"].as_str() {
        lines.push(format!("Fork of {}", text::one_line(parent)));
    }
    lines.push(format!(
        "Stars {}, forks {}, watchers {}, open issues {}, size {} KB",
        count(&r, "stargazers_count"),
        count(&r, "forks_count"),
        count(&r, "subscribers_count"),
        count(&r, "open_issues_count"),
        count(&r, "size")
    ));
    if let Some(license) = r["license"]["spdx_id"].as_str() {
        lines.push(format!("License: {license}"));
    }
    if r["archived"].as_bool() == Some(true) {
        lines.push("Archived (read-only)".to_owned());
    }
    let by_language = languages_text(&languages);
    if !by_language.is_empty() {
        lines.push(format!("Languages:\n{by_language}"));
    }
    let mut item = repo_item(&r);
    item.body = Some(lines.join("\n"));
    item.extra = object(json!({
        "full_name": r["full_name"], "description": r["description"], "homepage": r["homepage"],
        "topics": r["topics"], "default_branch": r["default_branch"], "visibility": visibility_of(&r),
        "private": r["private"], "archived": r["archived"], "disabled": r["disabled"], "fork": r["fork"],
        "parent": r["parent"]["full_name"], "source": r["source"]["full_name"],
        "size_kb": r["size"], "stars": r["stargazers_count"], "forks": r["forks_count"],
        "watchers": r["subscribers_count"], "open_issues": r["open_issues_count"],
        "license": r["license"]["spdx_id"], "languages": languages, "features": features(&r),
        "created_at": r["created_at"], "pushed_at": r["pushed_at"], "url": r["html_url"],
        "permissions": r["permissions"],
    }));
    Ok(vec![item])
}

async fn repo_forks_list(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
    let repo = repo_arg(call)?;
    let query = [("per_page", "100".to_owned()), ("sort", call.str_arg("sort").unwrap_or("newest").to_owned())];
    let forks = gh.pages(token, &format!("/repos/{repo}/forks"), &query, None, limit(call)).await?;
    Ok(filtered_repos(&forks, call, None))
}

async fn repo_languages(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
    let repo = repo_arg(call)?;
    let languages = get(gh, token, &format!("/repos/{repo}/languages")).await?;
    let mut item = repo_row(&repo, format!("{repo}#languages"), format!("Languages of {repo}"));
    item.body = Some(languages_text(&languages)).filter(|t| !t.is_empty());
    item.snippet = format!("{} languages", languages.as_object().map_or(0, Map::len));
    item.extra = object(json!({"languages": languages}));
    Ok(vec![item])
}

async fn repo_contributors(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
    let repo = repo_arg(call)?;
    let query = [("per_page", "100".to_owned())];
    let people = gh.pages(token, &format!("/repos/{repo}/contributors"), &query, None, limit(call)).await?;
    Ok(people
        .iter()
        .map(|p| {
            let login = s(p, "login");
            let mut item = repo_row(&repo, login.to_owned(), text::one_line(login));
            s(p, "type").clone_into(&mut item.from);
            item.snippet = format!("{} commits", count(p, "contributions"));
            item.extra =
                object(json!({"login": p["login"], "contributions": p["contributions"], "url": p["html_url"]}));
            item
        })
        .collect())
}

async fn repo_topics_get(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
    let repo = repo_arg(call)?;
    let v = get(gh, token, &format!("/repos/{repo}/topics")).await?;
    let names = strings(&v["names"]);
    let mut item = repo_row(&repo, format!("{repo}#topics"), format!("Topics of {repo}"));
    item.snippet = text::one_line(&names.join(", "));
    item.body = Some(names.join(", ")).filter(|t| !t.is_empty());
    item.extra = object(json!({"topics": names}));
    Ok(vec![item])
}

async fn repo_readme_get(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
    let repo = repo_arg(call)?;
    let path = format!("/repos/{repo}/readme");
    let meta = get(gh, token, &path).await?;
    let decoded = meta["content"]
        .as_str()
        .map(|c| c.chars().filter(|c| !c.is_whitespace()).collect::<String>())
        .filter(|c| !c.is_empty() && meta["encoding"].as_str() == Some("base64"))
        .and_then(|c| BASE64.decode(c.as_bytes()).ok())
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
    let content = if let Some(t) = decoded {
        t
    } else {
        let options = Options {
            accept: Some("application/vnd.github.raw+json"),
            max_text: 4 * MAX_README,
            ..Options::default()
        };
        gh.send(token, Method::GET, &path, &[], None, &options).await?.text
    };
    let truncated = content.chars().count() > MAX_README;
    let name = s(&meta, "path");
    let mut item = repo_row(&repo, format!("{repo}#{name}"), text::one_line(name));
    item.snippet = format!("README of {repo}, {} bytes", count(&meta, "size"));
    item.body = Some(text::truncate_chars(&content, MAX_README));
    item.extra =
        object(json!({"path": meta["path"], "size": meta["size"], "truncated": truncated, "url": meta["html_url"]}));
    Ok(vec![item])
}

// ---- repositories: writes ---------------------------------------------------------------------------------------------

/// A repository name (without the owner).
fn name_ok(name: &str) -> bool {
    name != "." && repo_ok(&format!("x/{name}"))
}

fn name_arg(call: &ConnectorCall, param: &str) -> Result<Option<String>, CoreError> {
    match call.str_arg(param) {
        None => Ok(None),
        Some(n) if name_ok(n) => Ok(Some(n.to_owned())),
        Some(_) => Err(fail(format!("`{param}` is not a valid repository name."))),
    }
}

/// Where a new repository goes: the organization named, else the user the token belongs to.
async fn new_owner(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<(String, Option<String>), CoreError> {
    if let Some(org) = opt_login(call, "org")? {
        return Ok((org.clone(), Some(org)));
    }
    let me = get(gh, token, "/user").await?;
    let login = s(&me, "login");
    if login.is_empty() {
        return Err(fail("GitHub did not say who the token belongs to."));
    }
    Ok((login.to_owned(), None))
}

fn create_body(call: &ConnectorCall) -> Result<Value, CoreError> {
    let name = name_arg(call, "name")?.ok_or_else(|| fail("`name` is required."))?;
    let mut body = json!({"name": name, "private": call.bool_arg("private").unwrap_or(true)});
    for key in ["description", "homepage", "gitignore_template", "license_template"] {
        if let Some(v) = call.str_arg(key) {
            body[key] = json!(v);
        }
    }
    if let Some(v) = call.bool_arg("auto_init") {
        body["auto_init"] = json!(v);
    }
    Ok(body)
}

async fn preview_repo_create(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Preview, CoreError> {
    let body = create_body(call)?;
    let (owner, _) = new_owner(gh, token, call).await?;
    let full = format!("{owner}/{}", s(&body, "name"));
    if get_opt(gh, token, &format!("/repos/{full}")).await?.is_some() {
        return Err(fail(format!("{full} already exists.")));
    }
    let kind = if body["private"] == json!(true) {
        "private"
    } else {
        "PUBLIC"
    };
    let mut lines = vec![format!("Create the {kind} repository {full}")];
    for (label, key) in [("Description", "description"), ("Homepage", "homepage")] {
        if let Some(v) = body[key].as_str() {
            lines.push(format!("{label}: {}", shown(v)));
        }
    }
    if body["auto_init"] == json!(true) {
        lines.push("With an initial commit (README)".to_owned());
    }
    for (label, key) in [(".gitignore", "gitignore_template"), ("License", "license_template")] {
        if let Some(v) = body[key].as_str() {
            lines.push(format!("{label}: {}", text::one_line(v)));
        }
    }
    Ok(repo_preview(&full, lines))
}

async fn perform_repo_create(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Value, CoreError> {
    let body = create_body(call)?;
    let path = match opt_login(call, "org")? {
        Some(org) => format!("/orgs/{org}/repos"),
        None => "/user/repos".to_owned(),
    };
    let made = gh.call(token, Method::POST, &path, &[], Some(&body)).await?;
    Ok(json!({
        "created": true, "full_name": made["full_name"], "url": made["html_url"],
        "private": made["private"], "default_branch": made["default_branch"],
    }))
}

const TEXT_SETTINGS: [(&str, &str); 2] = [("description", "Description"), ("homepage", "Homepage")];
const FLAG_SETTINGS: [(&str, &str); 8] = [
    ("has_issues", "Issues"),
    ("has_projects", "Projects"),
    ("has_wiki", "Wiki"),
    ("allow_merge_commit", "Merge commits"),
    ("allow_squash_merge", "Squash merging"),
    ("allow_rebase_merge", "Rebase merging"),
    ("allow_auto_merge", "Auto-merge"),
    ("delete_branch_on_merge", "Delete branch on merge"),
];

/// The settings to change, from the arguments given (and only those).
fn update_body(call: &ConnectorCall) -> Result<Map<String, Value>, CoreError> {
    let mut body = Map::new();
    for (key, _) in TEXT_SETTINGS {
        if let Some(v) = call.str_arg(key) {
            body.insert(key.to_owned(), json!(v));
        }
    }
    if let Some(branch) = ref_arg(call, "default_branch")? {
        body.insert("default_branch".to_owned(), json!(branch));
    }
    for (key, _) in FLAG_SETTINGS {
        if let Some(v) = call.bool_arg(key) {
            body.insert(key.to_owned(), json!(v));
        }
    }
    match call.bool_arg("archived") {
        Some(true) => {
            body.insert("archived".to_owned(), json!(true));
        }
        Some(false) => {
            return Err(fail("GitHub cannot unarchive a repository through its API; do that on github.com."));
        }
        None => {}
    }
    if body.is_empty() {
        return Err(fail("Give at least one setting to change."));
    }
    Ok(body)
}

async fn preview_repo_update(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Preview, CoreError> {
    let repo = repo_arg(call)?;
    let body = update_body(call)?;
    let now = repo_state(gh, token, &repo).await?;
    let mut lines = vec![format!("Change the settings of {repo}")];
    for (key, label) in TEXT_SETTINGS {
        if let Some(v) = body.get(key).and_then(Value::as_str) {
            change(&mut lines, label, quoted(s(&now, key)), quoted(&shown(v)));
        }
    }
    if let Some(v) = body.get("default_branch").and_then(Value::as_str) {
        change(&mut lines, "Default branch", s(&now, "default_branch"), v);
    }
    for (key, label) in FLAG_SETTINGS {
        if let Some(v) = body.get(key).and_then(Value::as_bool) {
            change(&mut lines, label, on_off(now[key].as_bool().unwrap_or(false)), on_off(v));
        }
    }
    if body.contains_key("archived") {
        lines.push("Archive the repository: it becomes read-only, and only github.com can undo it.".to_owned());
    }
    Ok(repo_preview(&repo, lines))
}

async fn perform_repo_update(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Value, CoreError> {
    let repo = repo_arg(call)?;
    let body = Value::Object(update_body(call)?);
    let done = gh.call(token, Method::PATCH, &format!("/repos/{repo}"), &[], Some(&body)).await?;
    let changed: Vec<&String> = body.as_object().map(|m| m.keys().collect()).unwrap_or_default();
    Ok(json!({"updated": true, "repo": repo, "changed": changed, "url": done["html_url"]}))
}

fn fork_body(call: &ConnectorCall) -> Result<Value, CoreError> {
    let mut body = json!({});
    if let Some(org) = opt_login(call, "org")? {
        body["organization"] = json!(org);
    }
    if let Some(name) = name_arg(call, "name")? {
        body["name"] = json!(name);
    }
    if let Some(v) = call.bool_arg("default_branch_only") {
        body["default_branch_only"] = json!(v);
    }
    Ok(body)
}

async fn preview_repo_fork(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Preview, CoreError> {
    let repo = repo_arg(call)?;
    let body = fork_body(call)?;
    let source = repo_state(gh, token, &repo).await?;
    let target = match body["organization"].as_str() {
        Some(org) => format!("the organization {org}"),
        None => "your account".to_owned(),
    };
    let mut lines = vec![format!("Fork {repo} ({}) into {target}", visibility_of(&source))];
    if let Some(name) = body["name"].as_str() {
        lines.push(format!("Named {name}"));
    }
    if body["default_branch_only"] == json!(true) {
        lines.push("Only the default branch is copied".to_owned());
    }
    Ok(repo_preview(&repo, lines))
}

async fn perform_repo_fork(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Value, CoreError> {
    let repo = repo_arg(call)?;
    let body = fork_body(call)?;
    let made = gh.call(token, Method::POST, &format!("/repos/{repo}/forks"), &[], Some(&body)).await?;
    Ok(json!({"forking": true, "full_name": made["full_name"], "url": made["html_url"]}))
}

async fn preview_repo_delete(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Preview, CoreError> {
    let repo = repo_arg(call)?;
    let now = repo_state(gh, token, &repo).await?;
    let mut lines = vec![format!("PERMANENTLY DELETE the {} repository {repo}", visibility_of(&now))];
    lines.push("All its code, history, issues, pull requests and wiki are lost. This cannot be undone.".to_owned());
    lines.push(format!(
        "{} stars, {} forks, {} open issues, {} KB, last push {}",
        count(&now, "stargazers_count"),
        count(&now, "forks_count"),
        count(&now, "open_issues_count"),
        count(&now, "size"),
        s(&now, "pushed_at")
    ));
    if !s(&now, "description").is_empty() {
        lines.push(format!("Description: {}", text::one_line(s(&now, "description"))));
    }
    let mut preview = repo_preview(&repo, lines);
    preview.once_only = true;
    Ok(preview)
}

async fn perform_repo_delete(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Value, CoreError> {
    let repo = repo_arg(call)?;
    gh.call(token, Method::DELETE, &format!("/repos/{repo}"), &[], None).await?;
    Ok(json!({"deleted": true, "repo": repo}))
}

fn transfer_body(call: &ConnectorCall) -> Result<Value, CoreError> {
    let mut body = json!({"new_owner": login_arg(call, "new_owner")?});
    if let Some(name) = name_arg(call, "new_name")? {
        body["new_name"] = json!(name);
    }
    Ok(body)
}

async fn preview_repo_transfer(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Preview, CoreError> {
    let repo = repo_arg(call)?;
    let body = transfer_body(call)?;
    let now = repo_state(gh, token, &repo).await?;
    let mut lines =
        vec![format!("Transfer the {} repository {repo} to {}", visibility_of(&now), s(&body, "new_owner"))];
    if let Some(name) = body["new_name"].as_str() {
        lines.push(format!("It will be renamed {name}"));
    }
    lines.push(
        "You lose control of it unless the new owner gives it back; the new owner may have to accept.".to_owned(),
    );
    let mut preview = repo_preview(&repo, lines);
    preview.once_only = true;
    Ok(preview)
}

async fn perform_repo_transfer(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Value, CoreError> {
    let repo = repo_arg(call)?;
    let body = transfer_body(call)?;
    let made = gh.call(token, Method::POST, &format!("/repos/{repo}/transfer"), &[], Some(&body)).await?;
    Ok(json!({"transfer_started": true, "new_owner": body["new_owner"], "url": made["html_url"]}))
}

fn visibility_arg(call: &ConnectorCall) -> Result<&str, CoreError> {
    match call.str_arg("visibility") {
        Some(v @ ("public" | "private")) => Ok(v),
        _ => Err(fail("`visibility` must be public or private.")),
    }
}

async fn preview_visibility(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Preview, CoreError> {
    let repo = repo_arg(call)?;
    let wanted = visibility_arg(call)?;
    let now = repo_state(gh, token, &repo).await?;
    if visibility_of(&now) == wanted {
        return Err(fail(format!("{repo} is already {wanted}.")));
    }
    let mut lines = vec![format!("Make {repo} {}: it is {} now", wanted.to_uppercase(), visibility_of(&now))];
    if wanted == "public" {
        lines.push("Everyone on the internet can then read all its code and history.".to_owned());
    } else {
        lines.push("Only people with access can then see it; stars and watchers may be lost.".to_owned());
    }
    let mut preview = repo_preview(&repo, lines);
    preview.once_only = true;
    Ok(preview)
}

async fn perform_visibility(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Value, CoreError> {
    let repo = repo_arg(call)?;
    let wanted = visibility_arg(call)?;
    let body = json!({"visibility": wanted});
    gh.call(token, Method::PATCH, &format!("/repos/{repo}"), &[], Some(&body)).await?;
    Ok(json!({"updated": true, "repo": repo, "visibility": wanted}))
}

fn topic_ok(t: &str) -> bool {
    t.len() <= 50
        && t.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        && t.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn topics_arg(call: &ConnectorCall) -> Result<Vec<String>, CoreError> {
    if !call.args.contains_key("topics") {
        return Err(fail("`topics` is required (an empty list removes all topics)."));
    }
    let topics = call.list_arg("topics");
    if let Some(bad) = topics.iter().find(|t| !topic_ok(t)) {
        return Err(fail(format!(
            "`{}` is not a valid topic: use lowercase letters, digits and hyphens, at most 50 characters.",
            text::one_line(bad)
        )));
    }
    Ok(topics.into_iter().map(str::to_owned).collect())
}

async fn preview_topics_set(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Preview, CoreError> {
    let repo = repo_arg(call)?;
    let topics = topics_arg(call)?;
    let now = get(gh, token, &format!("/repos/{repo}/topics")).await?;
    let old = strings(&now["names"]).join(", ");
    let mut lines = vec![format!("Set the topics of {repo}")];
    change(&mut lines, "Topics", format!("[{old}]"), format!("[{}]", topics.join(", ")));
    Ok(repo_preview(&repo, lines))
}

async fn perform_topics_set(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Value, CoreError> {
    let repo = repo_arg(call)?;
    let topics = topics_arg(call)?;
    let body = json!({"names": topics});
    let done = gh.call(token, Method::PUT, &format!("/repos/{repo}/topics"), &[], Some(&body)).await?;
    Ok(json!({"updated": true, "repo": repo, "topics": done["names"]}))
}

// ---- branches ------------------------------------------------------------------------------------------------------

async fn branch_list(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
    let repo = repo_arg(call)?;
    let mut query = vec![("per_page", "100".to_owned())];
    if let Some(p) = call.bool_arg("protected") {
        query.push(("protected", p.to_string()));
    }
    let branches = gh.pages(token, &format!("/repos/{repo}/branches"), &query, None, limit(call)).await?;
    Ok(branches
        .iter()
        .map(|b| {
            let mut item = branch_row(&repo, s(b, "name"));
            let protected = b["protected"].as_bool().unwrap_or(false);
            item.from = s(&b["commit"], "sha").chars().take(7).collect();
            item.snippet = String::from(if protected {
                "protected"
            } else {
                "not protected"
            });
            item.extra = object(json!({"sha": b["commit"]["sha"], "protected": protected}));
            item
        })
        .collect())
}

async fn branch_state(gh: &GitHub, token: &str, repo: &str, branch: &str) -> Result<Value, CoreError> {
    get(gh, token, &format!("/repos/{repo}/branches/{}", segment(branch))).await
}

/// `abc1234: subject (author)` for the head of a branch answer.
fn head_line(b: &Value) -> String {
    let commit = &b["commit"];
    let author = commit["commit"]["author"]["name"].as_str().unwrap_or_default();
    format!("{}: {} ({})", short(s(commit, "sha")), subject(s(&commit["commit"], "message")), text::one_line(author))
}

async fn branch_get(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
    let repo = repo_arg(call)?;
    let branch = ref_req(call, "branch")?;
    let b = branch_state(gh, token, &repo, &branch).await?;
    let protected = b["protected"].as_bool().unwrap_or(false);
    let mut item = branch_row(&repo, &branch);
    item.from = short(s(&b["commit"], "sha"));
    item.snippet = subject(s(&b["commit"]["commit"], "message"));
    item.date = date_of(&b["commit"]["commit"]["author"], "date");
    item.body = Some(format!(
        "Branch {branch} of {repo}, {}\nHead {}",
        if protected {
            "protected"
        } else {
            "not protected"
        },
        head_line(&b)
    ));
    item.extra = object(json!({
        "sha": b["commit"]["sha"], "protected": protected, "message": b["commit"]["commit"]["message"],
        "author": b["commit"]["commit"]["author"]["name"], "date": b["commit"]["commit"]["author"]["date"],
        "url": b["_links"]["html"],
    }));
    Ok(vec![item])
}

fn flag_of(p: &Value, key: &str) -> bool {
    p[key]["enabled"].as_bool().unwrap_or(false)
}

/// The rules of a branch protection answer, one per line.
fn protection_lines(p: &Value) -> Vec<String> {
    let mut lines = Vec::new();
    let checks = &p["required_status_checks"];
    if checks.is_object() {
        let contexts = strings(&checks["contexts"]).join(", ");
        lines.push(format!(
            "Required checks: {}{}",
            if contexts.is_empty() {
                "(none named)"
            } else {
                &contexts
            },
            if checks["strict"].as_bool() == Some(true) {
                ", branch must be up to date"
            } else {
                ""
            }
        ));
    }
    let reviews = &p["required_pull_request_reviews"];
    if reviews.is_object() {
        lines.push(format!(
            "Pull request reviews: {} approving, stale approvals dismissed {}, code owners {}",
            count(reviews, "required_approving_review_count"),
            on_off(reviews["dismiss_stale_reviews"].as_bool().unwrap_or(false)),
            on_off(reviews["require_code_owner_reviews"].as_bool().unwrap_or(false)),
        ));
    }
    for (key, label) in [
        ("enforce_admins", "Applies to administrators"),
        ("required_linear_history", "Linear history required"),
        ("allow_force_pushes", "Force pushes allowed"),
        ("allow_deletions", "Deleting the branch allowed"),
        ("required_conversation_resolution", "Conversations must be resolved"),
        ("lock_branch", "Branch locked (read-only)"),
        ("block_creations", "Creating matching branches blocked"),
    ] {
        if flag_of(p, key) {
            lines.push(label.to_owned());
        }
    }
    let restrictions = &p["restrictions"];
    if restrictions.is_object() {
        let users: Vec<&str> = restrictions["users"]
            .as_array()
            .map(|a| a.iter().filter_map(|u| u["login"].as_str()).collect())
            .unwrap_or_default();
        let teams: Vec<&str> = restrictions["teams"]
            .as_array()
            .map(|a| a.iter().filter_map(|t| t["slug"].as_str()).collect())
            .unwrap_or_default();
        lines.push(format!("Only these may push: users [{}], teams [{}]", users.join(", "), teams.join(", ")));
    }
    if lines.is_empty() {
        lines.push("Protected, with no extra rules".to_owned());
    }
    lines.iter().map(|l| text::one_line(l)).collect()
}

/// The protection of a branch; `None` when it has none.
async fn protection_state(gh: &GitHub, token: &str, repo: &str, branch: &str) -> Result<Option<Value>, CoreError> {
    let path = format!("/repos/{repo}/branches/{}/protection", segment(branch));
    let options = Options {
        allow: &[404],
        ..Options::default()
    };
    let reply = gh.send(token, Method::GET, &path, &[], None, &options).await?;
    if reply.status != 404 {
        return Ok(Some(reply.json));
    }
    let message = s(&reply.json, "message");
    if message.to_lowercase().contains("not protected") {
        Ok(None)
    } else {
        Err(fail(format!(
            "GitHub says that does not exist, or the token cannot see it: {}",
            text::truncate_chars(&text::one_line(message), 200)
        )))
    }
}

async fn branch_protection_get(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
    let repo = repo_arg(call)?;
    let branch = ref_req(call, "branch")?;
    let state = protection_state(gh, token, &repo, &branch).await?;
    let mut item = branch_row(&repo, &branch);
    item.id = format!("{branch}#protection");
    item.title = format!("Protection of {}", text::one_line(&branch));
    match state {
        None => {
            "not protected".clone_into(&mut item.snippet);
            item.body = Some(format!("The branch {branch} of {repo} has no protection rules."));
            item.extra = object(json!({"protected": false}));
        }
        Some(p) => {
            let lines = protection_lines(&p);
            "protected".clone_into(&mut item.snippet);
            item.body = Some(lines.join("\n"));
            item.extra = object(json!({
                "protected": true,
                "required_status_checks": p["required_status_checks"],
                "required_pull_request_reviews": p["required_pull_request_reviews"],
                "enforce_admins": flag_of(&p, "enforce_admins"),
                "required_linear_history": flag_of(&p, "required_linear_history"),
                "allow_force_pushes": flag_of(&p, "allow_force_pushes"),
                "allow_deletions": flag_of(&p, "allow_deletions"),
                "required_conversation_resolution": flag_of(&p, "required_conversation_resolution"),
                "lock_branch": flag_of(&p, "lock_branch"),
                "restrictions": p["restrictions"],
            }));
        }
    }
    Ok(vec![item])
}

async fn ruleset_list(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
    let repo = repo_arg(call)?;
    let query = [("per_page", "100".to_owned())];
    let sets = gh.pages(token, &format!("/repos/{repo}/rulesets"), &query, None, limit(call)).await?;
    Ok(sets
        .iter()
        .map(|r| {
            let mut item = repo_row(&repo, count(r, "id").to_string(), text::one_line(s(r, "name")));
            s(r, "source_type").clone_into(&mut item.from);
            item.snippet = format!("{} · {}", s(r, "target"), s(r, "enforcement"));
            item.date = date_of(r, "updated_at");
            item.extra = object(json!({
                "id": r["id"], "name": r["name"], "target": r["target"], "enforcement": r["enforcement"],
                "source_type": r["source_type"], "source": r["source"],
            }));
            item
        })
        .collect())
}

async fn ruleset_get(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
    let repo = repo_arg(call)?;
    let id = id_arg(call, "ruleset_id")?;
    let r = get(gh, token, &format!("/repos/{repo}/rulesets/{id}")).await?;
    let rules: Vec<&str> =
        r["rules"].as_array().map(|a| a.iter().filter_map(|x| x["type"].as_str()).collect()).unwrap_or_default();
    let include = strings(&r["conditions"]["ref_name"]["include"]).join(", ");
    let exclude = strings(&r["conditions"]["ref_name"]["exclude"]).join(", ");
    let mut item = repo_row(&repo, id.to_string(), text::one_line(s(&r, "name")));
    item.snippet = format!("{} · {}", s(&r, "target"), s(&r, "enforcement"));
    item.body = Some(format!(
        "Ruleset {} ({}, {})\nApplies to: {}{}\nRules: {}\nBypass actors: {}",
        text::one_line(s(&r, "name")),
        s(&r, "target"),
        s(&r, "enforcement"),
        text::one_line(&include),
        if exclude.is_empty() {
            String::new()
        } else {
            format!(" (except {})", text::one_line(&exclude))
        },
        rules.join(", "),
        r["bypass_actors"].as_array().map_or(0, Vec::len),
    ));
    item.extra = object(json!({
        "id": r["id"], "name": r["name"], "target": r["target"], "enforcement": r["enforcement"],
        "conditions": r["conditions"], "rules": r["rules"], "bypass_actors": r["bypass_actors"],
    }));
    Ok(vec![item])
}

/// Where a branch, tag or commit points: the sha of the commit and how to call it.
struct Source {
    sha: String,
    label: String,
}

fn is_sha(name: &str) -> bool {
    matches!(name.len(), 40 | 64) && name.bytes().all(|b| b.is_ascii_hexdigit())
}

async fn resolve_source(gh: &GitHub, token: &str, repo: &str, from: Option<&str>) -> Result<Source, CoreError> {
    let name = match from {
        Some(f) => f.to_owned(),
        None => s(&repo_state(gh, token, repo).await?, "default_branch").to_owned(),
    };
    if is_sha(&name) {
        let commit = get_opt(gh, token, &format!("/repos/{repo}/git/commits/{name}"))
            .await?
            .ok_or_else(|| fail(format!("{repo} has no commit {}.", short(&name))))?;
        return Ok(Source {
            sha: name.clone(),
            label: format!("commit {}: {}", short(&name), subject(s(&commit, "message"))),
        });
    }
    for kind in ["heads", "tags"] {
        let Some(found) = get_opt(gh, token, &format!("/repos/{repo}/git/ref/{kind}/{}", ref_path(&name))).await?
        else {
            continue;
        };
        let mut sha = s(&found["object"], "sha").to_owned();
        if found["object"]["type"].as_str() == Some("tag") {
            let tag = get(gh, token, &format!("/repos/{repo}/git/tags/{sha}")).await?;
            s(&tag["object"], "sha").clone_into(&mut sha);
        }
        let what = if kind == "heads" {
            "branch"
        } else {
            "tag"
        };
        return Ok(Source {
            label: format!("{what} {} ({})", text::one_line(&name), short(&sha)),
            sha,
        });
    }
    Err(fail(format!("{repo} has no branch or tag named {}.", text::one_line(&name))))
}

async fn branch_exists(gh: &GitHub, token: &str, repo: &str, branch: &str) -> Result<bool, CoreError> {
    Ok(get_opt(gh, token, &format!("/repos/{repo}/git/ref/heads/{}", ref_path(branch))).await?.is_some())
}

async fn preview_branch_create(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Preview, CoreError> {
    let repo = repo_arg(call)?;
    let branch = ref_req(call, "branch")?;
    let from = ref_arg(call, "from")?;
    if branch_exists(gh, token, &repo, &branch).await? {
        return Err(fail(format!("{repo} already has a branch named {}.", text::one_line(&branch))));
    }
    let source = resolve_source(gh, token, &repo, from.as_deref()).await?;
    Ok(branch_preview(
        &repo,
        &branch,
        vec![
            format!("Create the branch {} in {repo}", text::one_line(&branch)),
            format!("Starting at {}", source.label),
        ],
    ))
}

async fn perform_branch_create(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Value, CoreError> {
    let repo = repo_arg(call)?;
    let branch = ref_req(call, "branch")?;
    let from = ref_arg(call, "from")?;
    let source = resolve_source(gh, token, &repo, from.as_deref()).await?;
    let body = json!({"ref": format!("refs/heads/{branch}"), "sha": source.sha});
    let made = gh.call(token, Method::POST, &format!("/repos/{repo}/git/refs"), &[], Some(&body)).await?;
    Ok(json!({"created": true, "branch": branch, "sha": source.sha, "ref": made["ref"]}))
}

async fn preview_branch_delete(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Preview, CoreError> {
    let repo = repo_arg(call)?;
    let branch = ref_req(call, "branch")?;
    let b = branch_state(gh, token, &repo, &branch).await?;
    let default = s(&repo_state(gh, token, &repo).await?, "default_branch").to_owned();
    if default == branch {
        return Err(fail(format!(
            "{} is the default branch of {repo}; it cannot be deleted.",
            text::one_line(&branch)
        )));
    }
    let mut lines = vec![
        format!("Delete the branch {} of {repo}", text::one_line(&branch)),
        format!("Its head is {}", head_line(&b)),
    ];
    if b["protected"].as_bool() == Some(true) {
        lines.push("The branch is protected: GitHub refuses to delete it until the protection is removed.".to_owned());
    }
    Ok(branch_preview(&repo, &branch, lines))
}

async fn perform_branch_delete(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Value, CoreError> {
    let repo = repo_arg(call)?;
    let branch = ref_req(call, "branch")?;
    let head = get_opt(gh, token, &format!("/repos/{repo}/git/ref/heads/{}", ref_path(&branch))).await?;
    gh.call(token, Method::DELETE, &format!("/repos/{repo}/git/refs/heads/{}", ref_path(&branch)), &[], None).await?;
    Ok(json!({"deleted": true, "branch": branch, "last_sha": head.map(|h| h["object"]["sha"].clone())}))
}

async fn preview_branch_rename(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Preview, CoreError> {
    let repo = repo_arg(call)?;
    let branch = ref_req(call, "branch")?;
    let new_name = ref_req(call, "new_name")?;
    if new_name == branch {
        return Err(fail("The new name is the same as the old one."));
    }
    let b = branch_state(gh, token, &repo, &branch).await?;
    if branch_exists(gh, token, &repo, &new_name).await? {
        return Err(fail(format!("{repo} already has a branch named {}.", text::one_line(&new_name))));
    }
    let default = s(&repo_state(gh, token, &repo).await?, "default_branch") == branch;
    let mut lines = vec![
        format!("Rename the branch {} of {repo} to {}", text::one_line(&branch), text::one_line(&new_name)),
        format!("Its head is {}", head_line(&b)),
    ];
    if default {
        lines.push("It is the default branch: clones, links and workflows that name it will break.".to_owned());
    }
    let mut preview = branch_preview(&repo, &branch, lines);
    preview.once_only = default;
    Ok(preview)
}

async fn perform_branch_rename(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Value, CoreError> {
    let repo = repo_arg(call)?;
    let branch = ref_req(call, "branch")?;
    let new_name = ref_req(call, "new_name")?;
    let body = json!({"new_name": new_name});
    let path = format!("/repos/{repo}/branches/{}/rename", segment(&branch));
    let done = gh.call(token, Method::POST, &path, &[], Some(&body)).await?;
    Ok(json!({"renamed": true, "old_name": branch, "new_name": done["name"], "sha": done["commit"]["sha"]}))
}

fn compare_path(repo: &str, base: &str, head: &str) -> String {
    format!("/repos/{repo}/compare/{}...{}", segment(base), segment(head))
}

async fn preview_branch_merge(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Preview, CoreError> {
    let repo = repo_arg(call)?;
    let base = ref_req(call, "base")?;
    let head = ref_req(call, "head")?;
    let cmp = get(gh, token, &compare_path(&repo, &base, &head)).await?;
    let default = s(&repo_state(gh, token, &repo).await?, "default_branch") == base;
    let ahead = count(&cmp, "ahead_by");
    let mut lines = vec![format!(
        "Merge {} into {} of {repo}{}",
        text::one_line(&head),
        text::one_line(&base),
        if default {
            " (the default branch)"
        } else {
            ""
        }
    )];
    if ahead == 0 {
        lines.push(format!("Nothing to merge: {} already contains {}.", text::one_line(&base), text::one_line(&head)));
    } else {
        lines.push(format!(
            "{ahead} commit(s) would be merged; {} is {} behind.",
            text::one_line(&head),
            count(&cmp, "behind_by")
        ));
        for c in cmp["commits"].as_array().into_iter().flatten().take(10) {
            let author = c["commit"]["author"]["name"].as_str().unwrap_or_default();
            lines.push(format!(
                "- {} {} ({})",
                short(s(c, "sha")),
                subject(s(&c["commit"], "message")),
                text::one_line(author)
            ));
        }
        if ahead > 10 {
            lines.push(format!("... and {} more", ahead - 10));
        }
        if let Some(files) = cmp["files"].as_array() {
            lines.push(format!("{} file(s) changed", files.len()));
        }
    }
    if let Some(m) = call.str_arg("commit_message") {
        lines.push(format!("Merge commit message: {}", shown(m)));
    }
    Ok(branch_preview(&repo, &base, lines))
}

async fn perform_branch_merge(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Value, CoreError> {
    let repo = repo_arg(call)?;
    let base = ref_req(call, "base")?;
    let head = ref_req(call, "head")?;
    let mut body = json!({"base": base, "head": head});
    if let Some(m) = call.str_arg("commit_message") {
        body["commit_message"] = json!(m);
    }
    let reply =
        gh.send(token, Method::POST, &format!("/repos/{repo}/merges"), &[], Some(&body), &Options::default()).await?;
    if reply.status == 204 {
        return Ok(json!({"merged": false, "already_up_to_date": true, "base": base, "head": head}));
    }
    Ok(json!({"merged": true, "sha": reply.json["sha"], "url": reply.json["html_url"], "base": base, "head": head}))
}

/// The protection rules asked for, as the body GitHub wants (which replaces the whole protection), and their
/// description for the user.
fn protection_body(call: &ConnectorCall) -> Result<(Value, Vec<String>), CoreError> {
    let contexts = call.list_arg("status_check_contexts");
    let strict = call.bool_arg("strict_status_checks");
    let approvals = call.int_arg("required_approvals").unwrap_or(0);
    let flag = |k: &str| call.bool_arg(k).unwrap_or(false);
    if approvals == 0
        && (call.bool_arg("dismiss_stale_reviews").is_some() || call.bool_arg("require_code_owner_reviews").is_some())
    {
        return Err(fail(
            "`dismiss_stale_reviews` and `require_code_owner_reviews` need `required_approvals` of 1 or more.",
        ));
    }
    let logins = |name: &str| -> Result<Vec<String>, CoreError> {
        call.list_arg(name)
            .into_iter()
            .map(|l| {
                if owner_ok(l) {
                    Ok(l.to_owned())
                } else {
                    Err(fail(format!("`{name}` has an invalid name.")))
                }
            })
            .collect()
    };
    let users = logins("restrict_push_users")?;
    let teams = logins("restrict_push_teams")?;
    let checks = if contexts.is_empty() && strict.is_none() {
        Value::Null
    } else {
        json!({"strict": strict.unwrap_or(false), "contexts": contexts})
    };
    let reviews = if approvals > 0 {
        json!({
            "required_approving_review_count": approvals,
            "dismiss_stale_reviews": flag("dismiss_stale_reviews"),
            "require_code_owner_reviews": flag("require_code_owner_reviews"),
        })
    } else {
        Value::Null
    };
    let restrictions = if users.is_empty() && teams.is_empty() {
        Value::Null
    } else {
        json!({"users": users, "teams": teams, "apps": []})
    };
    let body = json!({
        "required_status_checks": checks,
        "enforce_admins": flag("enforce_admins"),
        "required_pull_request_reviews": reviews,
        "restrictions": restrictions,
        "required_linear_history": flag("required_linear_history"),
        "allow_force_pushes": flag("allow_force_pushes"),
        "allow_deletions": flag("allow_deletions"),
        "required_conversation_resolution": flag("required_conversation_resolution"),
        "lock_branch": flag("lock_branch"),
    });
    // Describe it the way an existing protection is described.
    let as_answer = json!({
        "required_status_checks": body["required_status_checks"],
        "required_pull_request_reviews": body["required_pull_request_reviews"],
        "enforce_admins": {"enabled": body["enforce_admins"]},
        "required_linear_history": {"enabled": body["required_linear_history"]},
        "allow_force_pushes": {"enabled": body["allow_force_pushes"]},
        "allow_deletions": {"enabled": body["allow_deletions"]},
        "required_conversation_resolution": {"enabled": body["required_conversation_resolution"]},
        "lock_branch": {"enabled": body["lock_branch"]},
        "restrictions": if users.is_empty() && teams.is_empty() {
            Value::Null
        } else {
            json!({"users": users.iter().map(|u| json!({"login": u})).collect::<Vec<_>>(),
                   "teams": teams.iter().map(|t| json!({"slug": t})).collect::<Vec<_>>()})
        },
    });
    Ok((body, protection_lines(&as_answer)))
}

async fn preview_protection_set(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Preview, CoreError> {
    let repo = repo_arg(call)?;
    let branch = ref_req(call, "branch")?;
    let (_, new_rules) = protection_body(call)?;
    let old = protection_state(gh, token, &repo, &branch).await?;
    let mut lines = vec![format!("Replace the protection of the branch {} of {repo}", text::one_line(&branch))];
    lines.push("Now:".to_owned());
    match old {
        Some(p) => lines.extend(protection_lines(&p).into_iter().map(|l| format!("  {l}"))),
        None => lines.push("  not protected".to_owned()),
    }
    lines.push("After:".to_owned());
    lines.extend(new_rules.into_iter().map(|l| format!("  {l}")));
    let mut preview = branch_preview(&repo, &branch, lines);
    preview.once_only = true;
    Ok(preview)
}

async fn perform_protection_set(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Value, CoreError> {
    let repo = repo_arg(call)?;
    let branch = ref_req(call, "branch")?;
    let (body, rules) = protection_body(call)?;
    let path = format!("/repos/{repo}/branches/{}/protection", segment(&branch));
    gh.call(token, Method::PUT, &path, &[], Some(&body)).await?;
    Ok(json!({"updated": true, "branch": branch, "rules": rules}))
}

async fn preview_protection_delete(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Preview, CoreError> {
    let repo = repo_arg(call)?;
    let branch = ref_req(call, "branch")?;
    let Some(p) = protection_state(gh, token, &repo, &branch).await? else {
        return Err(fail(format!("The branch {} of {repo} has no protection to remove.", text::one_line(&branch))));
    };
    let mut lines = vec![format!("Remove ALL protection from the branch {} of {repo}", text::one_line(&branch))];
    lines.push("Rules that go away:".to_owned());
    lines.extend(protection_lines(&p).into_iter().map(|l| format!("  {l}")));
    let mut preview = branch_preview(&repo, &branch, lines);
    preview.once_only = true;
    Ok(preview)
}

async fn perform_protection_delete(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Value, CoreError> {
    let repo = repo_arg(call)?;
    let branch = ref_req(call, "branch")?;
    let path = format!("/repos/{repo}/branches/{}/protection", segment(&branch));
    gh.call(token, Method::DELETE, &path, &[], None).await?;
    Ok(json!({"removed": true, "branch": branch}))
}

// ---- access ----------------------------------------------------------------------------------------------------------

fn permission_arg(call: &ConnectorCall) -> Result<&str, CoreError> {
    match call.str_arg("permission") {
        Some(p @ ("pull" | "triage" | "push" | "maintain" | "admin")) => Ok(p),
        _ => Err(fail("`permission` must be pull, triage, push, maintain or admin.")),
    }
}

async fn collaborator_list(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
    let repo = repo_arg(call)?;
    let query =
        [("per_page", "100".to_owned()), ("affiliation", call.str_arg("affiliation").unwrap_or("all").to_owned())];
    let people = gh.pages(token, &format!("/repos/{repo}/collaborators"), &query, None, limit(call)).await?;
    Ok(people
        .iter()
        .map(|p| {
            let login = s(p, "login");
            let role = s(p, "role_name");
            let mut item = repo_row(&repo, login.to_owned(), text::one_line(login));
            role.clone_into(&mut item.from);
            item.snippet = format!("{role} access");
            item.extra = object(json!({
                "login": p["login"], "role_name": p["role_name"], "permissions": p["permissions"],
                "site_admin": p["site_admin"], "url": p["html_url"],
            }));
            item
        })
        .collect())
}

async fn preview_collaborator_add(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Preview, CoreError> {
    let repo = repo_arg(call)?;
    let user = login_arg(call, "username")?;
    let permission = permission_arg(call)?;
    let account = get(gh, token, &format!("/users/{user}")).await?;
    let already = get_opt(gh, token, &format!("/repos/{repo}/collaborators/{user}")).await?.is_some();
    let name = s(&account, "name");
    let who = if name.is_empty() {
        user.clone()
    } else {
        format!("{user} ({})", text::one_line(name))
    };
    let mut lines = vec![format!("Give {who} {permission} access to {repo}")];
    if permission == "admin" {
        lines.push("Admin access includes changing settings, adding people and deleting the repository.".to_owned());
    }
    lines.push(
        if already {
            "They already have access: this changes their permission."
        } else {
            "They get an invitation by email and must accept it."
        }
        .to_owned(),
    );
    let mut preview = repo_preview(&repo, lines);
    preview.once_only = true;
    Ok(preview)
}

async fn perform_collaborator_add(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Value, CoreError> {
    let repo = repo_arg(call)?;
    let user = login_arg(call, "username")?;
    let permission = permission_arg(call)?;
    let body = json!({"permission": permission});
    let path = format!("/repos/{repo}/collaborators/{user}");
    let reply = gh.send(token, Method::PUT, &path, &[], Some(&body), &Options::default()).await?;
    if reply.status == 201 {
        Ok(json!({"invited": true, "invitation_id": reply.json["id"], "username": user, "permission": permission}))
    } else {
        Ok(json!({"added": true, "username": user, "permission": permission}))
    }
}

async fn preview_collaborator_remove(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Preview, CoreError> {
    let repo = repo_arg(call)?;
    let user = login_arg(call, "username")?;
    let Some(now) = get_opt(gh, token, &format!("/repos/{repo}/collaborators/{user}/permission")).await? else {
        return Err(fail(format!("{user} is not a collaborator of {repo}.")));
    };
    let role = if s(&now, "role_name").is_empty() {
        s(&now, "permission")
    } else {
        s(&now, "role_name")
    };
    let mut preview = repo_preview(
        &repo,
        vec![
            format!("Remove {user} from {repo}"),
            format!("Their current permission: {}", text::one_line(role)),
            "They lose their direct access (access through a team or as the owner stays).".to_owned(),
        ],
    );
    preview.once_only = true;
    Ok(preview)
}

async fn perform_collaborator_remove(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Value, CoreError> {
    let repo = repo_arg(call)?;
    let user = login_arg(call, "username")?;
    gh.call(token, Method::DELETE, &format!("/repos/{repo}/collaborators/{user}"), &[], None).await?;
    Ok(json!({"removed": true, "username": user}))
}

async fn pending_invitations(gh: &GitHub, token: &str, repo: &str, max: usize) -> Result<Vec<Value>, CoreError> {
    let query = [("per_page", "100".to_owned())];
    gh.pages(token, &format!("/repos/{repo}/invitations"), &query, None, max).await
}

async fn invitation_list(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
    let repo = repo_arg(call)?;
    let invitations = pending_invitations(gh, token, &repo, limit(call)).await?;
    Ok(invitations
        .iter()
        .map(|i| {
            let invitee = s(&i["invitee"], "login");
            let mut item = repo_row(&repo, count(i, "id").to_string(), text::one_line(invitee));
            s(&i["inviter"], "login").clone_into(&mut item.from);
            item.snippet = format!("{} · invited by {}", s(i, "permissions"), text::one_line(&item.from));
            item.date = date_of(i, "created_at");
            item.extra = object(json!({
                "id": i["id"], "invitee": i["invitee"]["login"], "inviter": i["inviter"]["login"],
                "permissions": i["permissions"], "expired": i["expired"], "url": i["html_url"],
            }));
            item
        })
        .collect())
}

async fn preview_invitation_cancel(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Preview, CoreError> {
    let repo = repo_arg(call)?;
    let id = id_arg(call, "invitation_id")?;
    let invitations = pending_invitations(gh, token, &repo, 500).await?;
    let found = invitations
        .iter()
        .find(|i| i["id"].as_i64() == Some(id))
        .ok_or_else(|| fail(format!("{repo} has no pending invitation {id}.")))?;
    let mut preview = repo_preview(
        &repo,
        vec![format!(
            "Cancel the invitation of {} to {repo} ({} access)",
            text::one_line(s(&found["invitee"], "login")),
            s(found, "permissions")
        )],
    );
    preview.once_only = true;
    Ok(preview)
}

async fn perform_invitation_cancel(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Value, CoreError> {
    let repo = repo_arg(call)?;
    let id = id_arg(call, "invitation_id")?;
    gh.call(token, Method::DELETE, &format!("/repos/{repo}/invitations/{id}"), &[], None).await?;
    Ok(json!({"cancelled": true, "invitation_id": id}))
}

async fn repo_team_list(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
    let repo = repo_arg(call)?;
    let query = [("per_page", "100".to_owned())];
    let teams = gh.pages(token, &format!("/repos/{repo}/teams"), &query, None, limit(call)).await?;
    Ok(teams
        .iter()
        .map(|t| {
            let slug = s(t, "slug");
            let mut item = repo_row(&repo, slug.to_owned(), text::one_line(s(t, "name")));
            s(t, "permission").clone_into(&mut item.from);
            item.snippet = format!("{} access · {}", s(t, "permission"), s(t, "privacy"));
            item.extra = object(json!({
                "slug": t["slug"], "name": t["name"], "permission": t["permission"], "privacy": t["privacy"],
                "url": t["html_url"],
            }));
            item
        })
        .collect())
}

/// The organization that owns the repository, and the team's slug.
fn team_target(call: &ConnectorCall, repo: &str) -> Result<(String, String), CoreError> {
    let team = login_arg(call, "team")?;
    let org = repo.split_once('/').map(|(o, _)| o.to_owned()).unwrap_or_default();
    Ok((org, team))
}

async fn preview_team(gh: &GitHub, token: &str, call: &ConnectorCall, add: bool) -> Result<Preview, CoreError> {
    let repo = repo_arg(call)?;
    let (org, slug) = team_target(call, &repo)?;
    let permission = if add {
        Some(permission_arg(call)?)
    } else {
        None
    };
    let team = get(gh, token, &format!("/orgs/{org}/teams/{slug}")).await?;
    let name = text::one_line(s(&team, "name"));
    let line = match permission {
        Some(p) => format!("Give the team {org}/{slug} ({name}) {p} access to {repo}"),
        None => format!("Remove the team {org}/{slug} ({name}) from {repo}"),
    };
    let mut lines = vec![line];
    lines.push(format!("The team has {} member(s).", count(&team, "members_count")));
    let mut preview = repo_preview(&repo, lines);
    preview.once_only = true;
    Ok(preview)
}

async fn perform_team(gh: &GitHub, token: &str, call: &ConnectorCall, add: bool) -> Result<Value, CoreError> {
    let repo = repo_arg(call)?;
    let (org, slug) = team_target(call, &repo)?;
    let path = format!("/orgs/{org}/teams/{slug}/repos/{repo}");
    if add {
        let permission = permission_arg(call)?;
        gh.call(token, Method::PUT, &path, &[], Some(&json!({"permission": permission}))).await?;
        Ok(json!({"added": true, "team": slug, "permission": permission}))
    } else {
        gh.call(token, Method::DELETE, &path, &[], None).await?;
        Ok(json!({"removed": true, "team": slug}))
    }
}

// ---- integrations ----------------------------------------------------------------------------------------------------

fn hook_row(repo: &str, h: &Value) -> Item {
    let id = count(h, "id");
    let host = redact_url(s(&h["config"], "url"));
    let events = strings(&h["events"]).join(", ");
    let active = h["active"].as_bool().unwrap_or(false);
    let mut item = repo_row(repo, id.to_string(), format!("Webhook {id}"));
    item.snippet = format!(
        "{host} · {} · {}",
        text::one_line(&events),
        if active {
            "active"
        } else {
            "inactive"
        }
    );
    item.date = date_of(h, "updated_at");
    item.extra = object(json!({
        "id": h["id"], "events": h["events"], "active": active, "address": host,
        "content_type": h["config"]["content_type"], "insecure_ssl": h["config"]["insecure_ssl"],
        "last_response": {
            "code": h["last_response"]["code"], "status": h["last_response"]["status"],
            "message": h["last_response"]["message"],
        },
    }));
    item
}

async fn webhook_list(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
    let repo = repo_arg(call)?;
    let query = [("per_page", "100".to_owned())];
    let hooks = gh.pages(token, &format!("/repos/{repo}/hooks"), &query, None, limit(call)).await?;
    Ok(hooks.iter().map(|h| hook_row(&repo, h)).collect())
}

async fn webhook_get(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
    let repo = repo_arg(call)?;
    let id = id_arg(call, "hook_id")?;
    let h = get(gh, token, &format!("/repos/{repo}/hooks/{id}")).await?;
    let mut item = hook_row(&repo, &h);
    let last = &h["last_response"];
    item.body = Some(format!(
        "Webhook {id} of {repo}\nDelivers to {}\nEvents: {}\nContent type: {}\nActive: {}\nLast delivery: {} {}",
        redact_url(s(&h["config"], "url")),
        strings(&h["events"]).join(", "),
        s(&h["config"], "content_type"),
        h["active"].as_bool().unwrap_or(false),
        last["code"].as_i64().map_or_else(|| "none".to_owned(), |c| c.to_string()),
        text::one_line(s(last, "message")),
    ));
    Ok(vec![item])
}

fn preview_webhook_create(call: &ConnectorCall) -> Result<Preview, CoreError> {
    let repo = repo_arg(call)?;
    let url = https_url(call.str_arg("url").ok_or_else(|| fail("`url` is required."))?)?;
    let events = events_arg(call)?.unwrap_or_else(|| vec!["push".to_owned()]);
    let mut lines =
        vec![format!("Create a webhook in {repo}"), format!("Sends {} to {}", events.join(", "), text::one_line(&url))];
    lines.push(format!("Content type: {}", call.str_arg("content_type").unwrap_or("json")));
    lines.push(
        if call.str_arg("secret").is_some() {
            "Deliveries are signed with a secret (not shown here)."
        } else {
            "No secret: the receiver cannot check who sent a delivery."
        }
        .to_owned(),
    );
    if call.bool_arg("insecure_ssl") == Some(true) {
        lines.push("The address's TLS certificate will NOT be checked.".to_owned());
    }
    if call.bool_arg("active") == Some(false) {
        lines.push("Created inactive: nothing is sent until it is turned on.".to_owned());
    }
    let mut preview = repo_preview(&repo, lines);
    preview.once_only = true;
    Ok(preview)
}

async fn perform_webhook_create(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Value, CoreError> {
    let repo = repo_arg(call)?;
    let url = https_url(call.str_arg("url").ok_or_else(|| fail("`url` is required."))?)?;
    let events = events_arg(call)?.unwrap_or_else(|| vec!["push".to_owned()]);
    let mut config = json!({
        "url": url,
        "content_type": call.str_arg("content_type").unwrap_or("json"),
        "insecure_ssl": if call.bool_arg("insecure_ssl") == Some(true) { "1" } else { "0" },
    });
    if let Some(secret) = call.str_arg("secret") {
        config["secret"] = json!(secret);
    }
    let body = json!({
        "name": "web", "active": call.bool_arg("active").unwrap_or(true), "events": events, "config": config,
    });
    let made = gh.call(token, Method::POST, &format!("/repos/{repo}/hooks"), &[], Some(&body)).await?;
    Ok(json!({
        "created": true, "hook_id": made["id"], "events": made["events"], "active": made["active"],
        "address": redact_url(&url),
    }))
}

const HOOK_FIELDS: [&str; 6] = ["url", "events", "content_type", "secret", "insecure_ssl", "active"];

async fn preview_webhook_update(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Preview, CoreError> {
    let repo = repo_arg(call)?;
    let id = id_arg(call, "hook_id")?;
    if !HOOK_FIELDS.iter().any(|f| call.args.contains_key(*f)) {
        return Err(fail("Give at least one thing to change."));
    }
    let url = call.str_arg("url").map(https_url).transpose()?;
    let events = events_arg(call)?;
    let now = get(gh, token, &format!("/repos/{repo}/hooks/{id}")).await?;
    let mut lines = vec![format!("Change the webhook {id} of {repo}")];
    if let Some(u) = &url {
        change(&mut lines, "Address", redact_url(s(&now["config"], "url")), text::one_line(u));
    }
    if let Some(e) = &events {
        change(&mut lines, "Events", strings(&now["events"]).join(", "), e.join(", "));
    }
    if let Some(c) = call.str_arg("content_type") {
        change(&mut lines, "Content type", s(&now["config"], "content_type"), c);
    }
    if call.str_arg("secret").is_some() {
        lines.push("The signing secret is replaced (not shown here).".to_owned());
    }
    if let Some(v) = call.bool_arg("insecure_ssl") {
        lines.push(format!(
            "Certificate check: {}",
            if v {
                "OFF"
            } else {
                "on"
            }
        ));
    }
    if let Some(v) = call.bool_arg("active") {
        change(&mut lines, "Active", now["active"].as_bool().unwrap_or(false), v);
    }
    let mut preview = repo_preview(&repo, lines);
    preview.once_only = true;
    Ok(preview)
}

async fn perform_webhook_update(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Value, CoreError> {
    let repo = repo_arg(call)?;
    let id = id_arg(call, "hook_id")?;
    let path = format!("/repos/{repo}/hooks/{id}");
    let url = call.str_arg("url").map(https_url).transpose()?;
    let events = events_arg(call)?;
    let now = get(gh, token, &path).await?;
    let mut body = Map::new();
    if let Some(e) = events {
        body.insert("events".to_owned(), json!(e));
    }
    if let Some(a) = call.bool_arg("active") {
        body.insert("active".to_owned(), json!(a));
    }
    if ["url", "content_type", "secret", "insecure_ssl"].iter().any(|f| call.args.contains_key(*f)) {
        // GitHub replaces the whole configuration; the current one (its secret is masked and kept) is the base.
        let mut config = now["config"].as_object().cloned().unwrap_or_default();
        if let Some(u) = url {
            config.insert("url".to_owned(), json!(u));
        }
        if let Some(c) = call.str_arg("content_type") {
            config.insert("content_type".to_owned(), json!(c));
        }
        if let Some(sec) = call.str_arg("secret") {
            config.insert("secret".to_owned(), json!(sec));
        }
        if let Some(v) = call.bool_arg("insecure_ssl") {
            config.insert(
                "insecure_ssl".to_owned(),
                json!(if v {
                    "1"
                } else {
                    "0"
                }),
            );
        }
        body.insert("config".to_owned(), Value::Object(config));
    }
    let done = gh.call(token, Method::PATCH, &path, &[], Some(&Value::Object(body))).await?;
    Ok(json!({"updated": true, "hook_id": id, "events": done["events"], "active": done["active"]}))
}

async fn preview_webhook_touch(
    gh: &GitHub,
    token: &str,
    call: &ConnectorCall,
    verb: &str,
) -> Result<Preview, CoreError> {
    let repo = repo_arg(call)?;
    let id = id_arg(call, "hook_id")?;
    let now = get(gh, token, &format!("/repos/{repo}/hooks/{id}")).await?;
    let mut preview = repo_preview(
        &repo,
        vec![
            format!("{verb} the webhook {id} of {repo}"),
            format!("It sends {} to {}", strings(&now["events"]).join(", "), redact_url(s(&now["config"], "url"))),
        ],
    );
    preview.once_only = true;
    Ok(preview)
}

async fn perform_webhook_delete(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Value, CoreError> {
    let repo = repo_arg(call)?;
    let id = id_arg(call, "hook_id")?;
    gh.call(token, Method::DELETE, &format!("/repos/{repo}/hooks/{id}"), &[], None).await?;
    Ok(json!({"deleted": true, "hook_id": id}))
}

async fn perform_webhook_ping(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Value, CoreError> {
    let repo = repo_arg(call)?;
    let id = id_arg(call, "hook_id")?;
    gh.call(token, Method::POST, &format!("/repos/{repo}/hooks/{id}/pings"), &[], None).await?;
    Ok(json!({"pinged": true, "hook_id": id}))
}

async fn deploy_key_list(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
    let repo = repo_arg(call)?;
    let query = [("per_page", "100".to_owned())];
    let keys = gh.pages(token, &format!("/repos/{repo}/keys"), &query, None, limit(call)).await?;
    Ok(keys
        .iter()
        .map(|k| {
            let read_only = k["read_only"].as_bool().unwrap_or(true);
            let mut item = repo_row(&repo, count(k, "id").to_string(), text::one_line(s(k, "title")));
            item.snippet = String::from(if read_only {
                "read-only"
            } else {
                "read and write"
            });
            item.date = date_of(k, "created_at");
            item.extra = object(json!({
                "id": k["id"], "title": k["title"], "read_only": read_only, "key": k["key"],
                "verified": k["verified"],
            }));
            item
        })
        .collect())
}

/// The public key the AI gave: one line, in the OpenSSH format.
fn deploy_key_arg(call: &ConnectorCall) -> Result<&str, CoreError> {
    let key = call.str_arg("key").unwrap_or_default();
    let known = ["ssh-", "ecdsa-", "sk-ssh-", "sk-ecdsa-"].iter().any(|p| key.starts_with(p));
    if !known || key.contains(['\n', '\r']) || key.split_whitespace().count() < 2 {
        return Err(fail("`key` must be one public key line such as `ssh-ed25519 AAAA... comment`."));
    }
    Ok(key)
}

fn preview_deploy_key_add(call: &ConnectorCall) -> Result<Preview, CoreError> {
    let repo = repo_arg(call)?;
    let key = deploy_key_arg(call)?;
    let title = call.str_arg("title").ok_or_else(|| fail("`title` is required."))?;
    let read_only = call.bool_arg("read_only").unwrap_or(true);
    let mut parts = key.split_whitespace();
    let kind = parts.next().unwrap_or_default();
    let material: String = parts.next().unwrap_or_default().chars().take(24).collect();
    let mut preview = repo_preview(
        &repo,
        vec![
            format!(
                "Add the deploy key '{}' to {repo} ({})",
                text::one_line(title),
                if read_only {
                    "read-only"
                } else {
                    "can also PUSH"
                }
            ),
            format!("Key: {kind} {material}..."),
        ],
    );
    preview.once_only = true;
    Ok(preview)
}

async fn perform_deploy_key_add(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Value, CoreError> {
    let repo = repo_arg(call)?;
    let key = deploy_key_arg(call)?;
    let body = json!({
        "title": call.str_arg("title").ok_or_else(|| fail("`title` is required."))?,
        "key": key,
        "read_only": call.bool_arg("read_only").unwrap_or(true),
    });
    let made = gh.call(token, Method::POST, &format!("/repos/{repo}/keys"), &[], Some(&body)).await?;
    Ok(json!({"added": true, "key_id": made["id"], "read_only": made["read_only"]}))
}

async fn preview_deploy_key_delete(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Preview, CoreError> {
    let repo = repo_arg(call)?;
    let id = id_arg(call, "key_id")?;
    let now = get(gh, token, &format!("/repos/{repo}/keys/{id}")).await?;
    let mut preview = repo_preview(
        &repo,
        vec![
            format!("Delete the deploy key '{}' of {repo}", text::one_line(s(&now, "title"))),
            if now["read_only"].as_bool().unwrap_or(true) {
                "It could only read.".to_owned()
            } else {
                "It could also push.".to_owned()
            },
        ],
    );
    preview.once_only = true;
    Ok(preview)
}

async fn perform_deploy_key_delete(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Value, CoreError> {
    let repo = repo_arg(call)?;
    let id = id_arg(call, "key_id")?;
    gh.call(token, Method::DELETE, &format!("/repos/{repo}/keys/{id}"), &[], None).await?;
    Ok(json!({"deleted": true, "key_id": id}))
}

// ---- traffic and statistics -------------------------------------------------------------------------------------------

async fn traffic(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
    let repo = repo_arg(call)?;
    let (kind, rows_key) = if call.op == "traffic_clones" {
        ("clones", "clones")
    } else {
        ("views", "views")
    };
    let per = call.str_arg("per").unwrap_or("day").to_owned();
    let v =
        gh.call(token, Method::GET, &format!("/repos/{repo}/traffic/{kind}"), &[("per", per.clone())], None).await?;
    let mut body =
        format!("{kind} of {repo} in the last 14 days: {} total, {} unique", count(&v, "count"), count(&v, "uniques"));
    let rows = v[rows_key].as_array().cloned().unwrap_or_default();
    for r in &rows {
        write!(body, "\n{} {}: {} ({} unique)", s(r, "timestamp"), per, count(r, "count"), count(r, "uniques")).ok();
    }
    let mut item = repo_row(&repo, format!("{repo}#{kind}"), format!("{kind} of {repo}"));
    item.snippet = format!("{} total, {} unique", count(&v, "count"), count(&v, "uniques"));
    item.body = Some(body);
    item.extra = object(json!({"count": v["count"], "uniques": v["uniques"], "per": per, "days": rows}));
    Ok(vec![item])
}

async fn commit_activity(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
    let repo = repo_arg(call)?;
    let reply = gh
        .send(token, Method::GET, &format!("/repos/{repo}/stats/commit_activity"), &[], None, &Options::default())
        .await?;
    let mut item = repo_row(&repo, format!("{repo}#commit_activity"), format!("Commit activity of {repo}"));
    let Some(weeks) = reply.json.as_array().filter(|_| reply.status == 200) else {
        "being computed".clone_into(&mut item.snippet);
        item.body = Some("GitHub is still computing these statistics. Ask again in a moment.".to_owned());
        item.extra = object(json!({"ready": false}));
        return Ok(vec![item]);
    };
    let total: i64 = weeks.iter().map(|w| count(w, "total")).sum();
    let rows: Vec<Value> =
        weeks.iter().map(|w| json!({"week": text::iso_utc(count(w, "week")), "total": w["total"]})).collect();
    let mut body = format!("{total} commits in the last {} weeks (week starting: commits)", weeks.len());
    for w in weeks.iter().rev().take(12).rev() {
        write!(body, "\n{}: {}", text::iso_utc(count(w, "week")).get(..10).unwrap_or_default(), count(w, "total")).ok();
    }
    item.snippet = format!("{total} commits in {} weeks", weeks.len());
    item.body = Some(body);
    item.extra = object(json!({"ready": true, "total": total, "weeks": rows}));
    Ok(vec![item])
}
