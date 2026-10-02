//! GitHub: issues, labels, milestones, pull requests, reviews, search and discussions.

use std::fmt::Write;

use reqwest::Method;
use rewarden_proto::connector::ConnectorCall;
use serde_json::{Map, Value, json};

use super::{GitHub, MAX_BODY, Options, Preview, owner_ok, parents, ref_ok, repo_arg, resource, resource_label};
use crate::connector::Item;
use crate::connector::calendar::{limit, segment};
use crate::{CoreError, text};

/// The most characters of patches or of a diff handed over.
const MAX_DIFF: usize = 60_000;
/// The most characters of AI-supplied text repeated in a preview.
const PREVIEW_TEXT: usize = 1_500;
/// The most inline comments in one review.
const MAX_INLINE: usize = 50;

/// The operations that change something.
const WRITES: &[&str] = &[
    "comment",
    "create_issue",
    "issue_update",
    "issue_lock",
    "issue_unlock",
    "issue_comment_edit",
    "issue_comment_delete",
    "issue_reaction_add",
    "issue_label_add",
    "issue_label_remove",
    "issue_assignee_add",
    "issue_assignee_remove",
    "issue_transfer",
    "label_create",
    "label_update",
    "label_delete",
    "milestone_create",
    "milestone_update",
    "milestone_delete",
    "pr_create",
    "pr_update",
    "pr_ready",
    "pr_draft",
    "pr_reviewers_request",
    "pr_reviewers_remove",
    "pr_update_branch",
    "pr_merge",
    "pr_review_create",
    "pr_review_dismiss",
    "pr_review_comment_create",
    "pr_review_comment_edit",
    "pr_review_comment_delete",
];

/// Lists and reads. `None` when the operation is not one of this area's.
pub(super) async fn fetch(gh: &GitHub, token: &str, call: &ConnectorCall) -> Option<Result<Vec<Item>, CoreError>> {
    Some(match call.op.as_str() {
        "list_issues" => list_issues(gh, token, call).await,
        "get_issue" => get_issue(gh, token, call).await,
        "search" => search(gh, token, call).await,
        "issue_comment_list" => issue_comment_list(gh, token, call).await,
        "issue_reaction_list" => issue_reaction_list(gh, token, call).await,
        "issue_timeline" => issue_timeline(gh, token, call).await,
        "assignee_list" => assignee_list(gh, token, call).await,
        "label_list" => label_list(gh, token, call).await,
        "milestone_list" => milestone_list(gh, token, call).await,
        "pr_list" => pr_list(gh, token, call).await,
        "pr_get" => pr_get(gh, token, call).await,
        "pr_files" => pr_files(gh, token, call).await,
        "pr_diff" => pr_diff(gh, token, call).await,
        "pr_commits" => pr_commits(gh, token, call).await,
        "pr_review_list" => pr_review_list(gh, token, call).await,
        "pr_review_comment_list" => pr_review_comment_list(gh, token, call).await,
        "discussion_list" => discussion_list(gh, token, call).await,
        "discussion_get" => discussion_get(gh, token, call).await,
        "search_code" => search_code(gh, token, call).await,
        "search_repos" => search_repos(gh, token, call).await,
        "search_commits" => search_commits(gh, token, call).await,
        "search_users" => search_users(gh, token, call).await,
        "search_topics" => search_topics(gh, token, call).await,
        "search_labels" => search_labels(gh, token, call).await,
        _ => return None,
    })
}

/// What a write would do. `None` when the operation is not one of this area's.
pub(super) async fn preview(gh: &GitHub, token: &str, call: &ConnectorCall) -> Option<Result<Preview, CoreError>> {
    if !WRITES.contains(&call.op.as_str()) {
        return None;
    }
    Some(match call.op.as_str() {
        "pr_merge" => preview_merge(gh, token, call).await,
        "pr_update_branch" => preview_update_branch(gh, token, call).await,
        "pr_ready" | "pr_draft" => preview_draft(gh, token, call).await,
        "issue_transfer" => preview_transfer(gh, token, call).await,
        _ => preview_plain(gh, token, call).await,
    })
}

/// Does the write. `None` when the operation is not one of this area's.
pub(super) async fn perform(gh: &GitHub, token: &str, call: &ConnectorCall) -> Option<Result<Value, CoreError>> {
    if !WRITES.contains(&call.op.as_str()) {
        return None;
    }
    Some(match call.op.as_str() {
        "pr_merge" => perform_merge(gh, token, call).await,
        "pr_ready" | "pr_draft" => perform_draft(gh, token, call).await,
        "issue_transfer" => perform_transfer(gh, token, call).await,
        _ => perform_plain(gh, token, call).await,
    })
}

// ---- Small helpers --------------------------------------------------------------------------------------------------

fn fail(message: impl Into<String>) -> CoreError {
    CoreError::service(message)
}

/// A required positive number argument (`number`, `comment_id`, ...).
fn id_arg(call: &ConnectorCall, name: &str) -> Result<i64, CoreError> {
    match call.int_arg(name) {
        Some(n) if n > 0 => Ok(n),
        _ => Err(fail(format!("`{name}` must be a positive number."))),
    }
}

/// At most `max` characters, and whether something was cut.
fn cut(s: &str, max: usize) -> (String, bool) {
    let t = text::truncate_chars(s, max);
    let cut = t.len() < s.len();
    (t, cut)
}

/// AI-supplied text as a preview shows it: in full up to a limit, with a note when it goes on.
fn shown(s: &str) -> String {
    let (t, cut) = cut(s, PREVIEW_TEXT);
    if cut {
        format!("{t}\n… (the text goes on; {} characters in all)", s.chars().count())
    } else {
        t
    }
}

fn when(v: &Value) -> i64 {
    v.as_str().and_then(text::parse_when).unwrap_or(0)
}

fn line1(s: &str) -> String {
    text::truncate_chars(&text::one_line(s.lines().next().unwrap_or_default()), 160)
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or_default()
}

fn title_of(v: &Value) -> String {
    text::one_line(v["title"].as_str().unwrap_or("(no title)"))
}

fn map(v: &Value) -> Map<String, Value> {
    v.as_object().cloned().unwrap_or_default()
}

fn names(v: &Value, key: &str) -> Vec<String> {
    v.as_array().map(|a| a.iter().filter_map(|x| x[key].as_str().map(text::one_line)).collect()).unwrap_or_default()
}

fn or_none(list: &[String]) -> String {
    if list.is_empty() {
        "(none)".to_owned()
    } else {
        list.join(", ")
    }
}

fn strs(call: &ConnectorCall, name: &str) -> Vec<String> {
    call.list_arg(name).into_iter().map(str::to_owned).collect()
}

/// Logins (users or teams) from a list argument, each checked.
fn logins_arg(call: &ConnectorCall, name: &str) -> Result<Vec<String>, CoreError> {
    let list = strs(call, name);
    match list.iter().find(|l| !owner_ok(l)) {
        Some(bad) => Err(fail(format!("`{name}` has an entry that is not a login: {}", text::one_line(bad)))),
        None => Ok(list),
    }
}

fn sha_ok(sha: &str) -> bool {
    (7..=40).contains(&sha.len()) && sha.bytes().all(|b| b.is_ascii_hexdigit())
}

fn sha_arg(call: &ConnectorCall, name: &str) -> Result<Option<String>, CoreError> {
    match call.str_arg(name) {
        None => Ok(None),
        Some(sha) if sha_ok(sha) => Ok(Some(sha.to_owned())),
        Some(_) => Err(fail(format!("`{name}` must be a commit sha (7 to 40 hex digits)."))),
    }
}

/// A file path inside a repository, as it goes into a request body.
fn path_ok(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 1024
        && !path.starts_with('/')
        && !path.contains('\\')
        && !path.chars().any(char::is_control)
        && !path.split('/').any(|part| part == "..")
}

fn color_arg(call: &ConnectorCall) -> Result<Option<String>, CoreError> {
    let Some(color) = call.str_arg("color") else {
        return Ok(None);
    };
    let hex = color.trim_start_matches('#');
    if hex.len() == 6 && hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(Some(hex.to_ascii_lowercase()))
    } else {
        Err(fail("`color` must be a six-digit hex colour such as d73a4a."))
    }
}

fn branch_arg(call: &ConnectorCall, name: &str) -> Result<String, CoreError> {
    let branch = call.str_arg(name).unwrap_or_default();
    if ref_ok(branch) {
        Ok(branch.to_owned())
    } else {
        Err(fail(format!("`{name}` is not a valid branch name.")))
    }
}

/// The head of a pull request: `branch`, or `owner:branch` for a fork.
fn head_arg(call: &ConnectorCall) -> Result<String, CoreError> {
    let head = call.str_arg("head").unwrap_or_default();
    let (owner, branch) = head.split_once(':').map_or((None, head), |(o, b)| (Some(o), b));
    if ref_ok(branch) && owner.is_none_or(owner_ok) {
        Ok(head.to_owned())
    } else {
        Err(fail("`head` must be a branch name, or owner:branch for a fork."))
    }
}

/// A list entry of a listing about `repo`.
fn entry(repo: &str, id: String, title: &str) -> Item {
    Item {
        id,
        resource: repo.to_owned(),
        resource_label: repo.to_owned(),
        title: text::one_line(title),
        parents: parents(repo, None),
        ..Item::default()
    }
}

/// Other people's text goes into `body`, cut, with `truncated` in `extra` when it was.
fn set_body(item: &mut Item, content: &str) {
    let (t, was_cut) = cut(content, MAX_BODY);
    if !t.is_empty() {
        item.body = Some(t);
    }
    if was_cut {
        item.extra.insert("truncated".to_owned(), json!(true));
    }
}

fn scoped(repo: &str, lines: Vec<String>) -> Preview {
    Preview {
        resource: repo.to_owned(),
        resource_label: repo.to_owned(),
        lines,
        parents: parents(repo, None),
        ..Preview::default()
    }
}

fn user_of(v: &Value) -> String {
    v["user"]["login"].as_str().or_else(|| v["author"]["login"].as_str()).unwrap_or_default().to_owned()
}

async fn subject(gh: &GitHub, token: &str, repo: &str, number: i64, pull: bool) -> Result<Value, CoreError> {
    let kind = if pull {
        "pulls"
    } else {
        "issues"
    };
    gh.call(token, Method::GET, &format!("/repos/{repo}/{kind}/{number}"), &[], None).await
}

/// One GraphQL call; GraphQL answers 200 with `errors`, which must not look like success.
async fn graphql(gh: &GitHub, token: &str, query: &str, variables: Value) -> Result<Value, CoreError> {
    let body = json!({"query": query, "variables": variables});
    let reply = gh.call(token, Method::POST, "/graphql", &[], Some(&body)).await?;
    if let Some(first) = reply["errors"].as_array().and_then(|e| e.first()) {
        let message = text::truncate_chars(&text::one_line(first["message"].as_str().unwrap_or("unknown error")), 200);
        return Err(fail(format!("GitHub refused: {message}")));
    }
    if reply["data"].is_null() {
        return Err(fail("GitHub gave no answer to the request."));
    }
    Ok(reply["data"].clone())
}

// ---- Issues: reading -------------------------------------------------------------------------------------------------

pub(super) fn issue_item(repo: &str, issue: &Value) -> Item {
    let number = issue["number"].as_i64().unwrap_or(0);
    let labels: Vec<&str> =
        issue["labels"].as_array().map(|l| l.iter().filter_map(|x| x["name"].as_str()).collect()).unwrap_or_default();
    let state = issue["state"].as_str().unwrap_or_default();
    let is_pr = issue.get("pull_request").is_some();
    let mut extra = Map::new();
    extra.insert("number".to_owned(), json!(number));
    extra.insert("state".to_owned(), json!(state));
    extra.insert("pull_request".to_owned(), json!(is_pr));
    if !labels.is_empty() {
        extra.insert("labels".to_owned(), json!(labels));
    }
    if let Some(url) = issue["html_url"].as_str() {
        extra.insert("url".to_owned(), json!(url));
    }
    let assignees = names(&issue["assignees"], "login");
    if !assignees.is_empty() {
        extra.insert("assignees".to_owned(), json!(assignees));
    }
    if let Some(milestone) = issue["milestone"]["title"].as_str() {
        extra.insert("milestone".to_owned(), json!(milestone));
    }
    if let Some(comments) = issue["comments"].as_i64() {
        extra.insert("comments".to_owned(), json!(comments));
    }
    if issue["locked"].as_bool() == Some(true) {
        extra.insert("locked".to_owned(), json!(true));
    }
    Item {
        id: format!("{repo}#{number}"),
        resource: repo.to_owned(),
        resource_label: repo.to_owned(),
        from: issue["user"]["login"].as_str().unwrap_or_default().to_owned(),
        title: text::one_line(issue["title"].as_str().unwrap_or("(no title)")),
        snippet: format!(
            "#{number} · {}{state}{}",
            if is_pr {
                "pull request · "
            } else {
                ""
            },
            if labels.is_empty() {
                String::new()
            } else {
                format!(" · {}", labels.join(", "))
            }
        ),
        date: issue["updated_at"].as_str().and_then(text::parse_when).unwrap_or(0),
        body: None,
        sensitive: false,
        secret: false,
        parents: parents(repo, None),
        extra,
    }
}

async fn list_issues(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
    let repo = repo_arg(call)?;
    let mut query =
        vec![("state", call.str_arg("state").unwrap_or("open").to_owned()), ("per_page", limit(call).to_string())];
    if let Some(assignee) = call.str_arg("assignee") {
        if !(assignee == "*" || assignee == "none" || owner_ok(assignee)) {
            return Err(fail("`assignee` must be a login, `none` or `*`."));
        }
        query.push(("assignee", assignee.to_owned()));
    }
    if let Some(creator) = call.str_arg("creator") {
        if !owner_ok(creator) {
            return Err(fail("`creator` must be a login."));
        }
        query.push(("creator", creator.to_owned()));
    }
    if let Some(milestone) = call.str_arg("milestone") {
        if !(milestone == "*" || milestone == "none" || milestone.bytes().all(|b| b.is_ascii_digit())) {
            return Err(fail("`milestone` must be a number, `*` or `none`."));
        }
        query.push(("milestone", milestone.to_owned()));
    }
    let labels = call.list_arg("labels");
    if !labels.is_empty() {
        query.push(("labels", labels.join(",")));
    }
    for key in ["sort", "direction"] {
        if let Some(v) = call.str_arg(key) {
            query.push((key, v.to_owned()));
        }
    }
    let v = gh.call(token, Method::GET, &format!("/repos/{repo}/issues"), &query, None).await?;
    Ok(v.as_array().map(|issues| issues.iter().map(|i| issue_item(&repo, i)).collect()).unwrap_or_default())
}

async fn get_issue(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
    let repo = repo_arg(call)?;
    let number = id_arg(call, "number")?;
    let issue = gh.call(token, Method::GET, &format!("/repos/{repo}/issues/{number}"), &[], None).await?;
    let comments = gh
        .call(
            token,
            Method::GET,
            &format!("/repos/{repo}/issues/{number}/comments"),
            &[("per_page", "30".to_owned())],
            None,
        )
        .await?;
    let mut body = issue["body"].as_str().unwrap_or_default().to_owned();
    for c in comments.as_array().into_iter().flatten() {
        write!(
            body,
            "\n\n@{} ({}):\n{}",
            c["user"]["login"].as_str().unwrap_or("?"),
            c["created_at"].as_str().unwrap_or_default(),
            c["body"].as_str().unwrap_or_default()
        )
        .ok();
    }
    let mut item = issue_item(&repo, &issue);
    set_body(&mut item, &body);
    Ok(vec![item])
}

async fn search(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
    let per_page = limit(call).to_string();
    let mut q = call.str_arg("query").unwrap_or_default().to_owned();
    if call.str_arg("repo").is_some() {
        let repo = repo_arg(call)?;
        q = format!("{q} repo:{repo}");
    }
    let v = gh.call(token, Method::GET, "/search/issues", &[("q", q), ("per_page", per_page)], None).await?;
    Ok(v["items"]
        .as_array()
        .map(|issues| {
            issues
                .iter()
                .map(|i| {
                    let repo = i["repository_url"].as_str().and_then(|u| u.split("/repos/").nth(1)).unwrap_or_default();
                    issue_item(repo, i)
                })
                .collect()
        })
        .unwrap_or_default())
}

async fn issue_comment_list(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
    let repo = repo_arg(call)?;
    let number = id_arg(call, "number")?;
    let v = gh
        .call(
            token,
            Method::GET,
            &format!("/repos/{repo}/issues/{number}/comments"),
            &[("per_page", limit(call).to_string())],
            None,
        )
        .await?;
    Ok(v.as_array()
        .into_iter()
        .flatten()
        .map(|c| {
            let id = c["id"].as_i64().unwrap_or(0);
            let mut item = entry(&repo, id.to_string(), &format!("Comment {id} on #{number}"));
            item.from = user_of(c);
            item.snippet = line1(s(&c["body"]));
            item.date = when(&c["created_at"]);
            set_body(&mut item, s(&c["body"]));
            item.extra = map(&json!({"id": id, "number": number, "url": c["html_url"],
                "author_association": c["author_association"], "created_at": c["created_at"], "updated_at": c["updated_at"]}));
            item
        })
        .collect())
}

/// Where reactions live: on an issue (`number`) or on one of its comments (`comment_id`), exactly one of them.
fn reaction_path(call: &ConnectorCall, repo: &str) -> Result<String, CoreError> {
    match (call.int_arg("number"), call.int_arg("comment_id")) {
        (Some(n), None) if n > 0 => Ok(format!("/repos/{repo}/issues/{n}/reactions")),
        (None, Some(c)) if c > 0 => Ok(format!("/repos/{repo}/issues/comments/{c}/reactions")),
        _ => Err(fail("Give exactly one of `number` (an issue or pull request) or `comment_id` (a comment).")),
    }
}

async fn issue_reaction_list(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
    let repo = repo_arg(call)?;
    let path = reaction_path(call, &repo)?;
    let v = gh.call(token, Method::GET, &path, &[("per_page", limit(call).to_string())], None).await?;
    Ok(v.as_array()
        .into_iter()
        .flatten()
        .map(|r| {
            let id = r["id"].as_i64().unwrap_or(0);
            let content = s(&r["content"]);
            let mut item = entry(&repo, id.to_string(), content);
            item.from = user_of(r);
            item.snippet = format!("{content} by {}", user_of(r));
            item.date = when(&r["created_at"]);
            item.extra = map(&json!({"id": id, "content": content}));
            item
        })
        .collect())
}

/// What a timeline event says in a line, and the text it carries (other people's words), if any.
fn timeline_detail(e: &Value) -> (String, Option<String>) {
    let event = s(&e["event"]);
    let name = |v: &Value| text::one_line(v.as_str().unwrap_or_default());
    match event {
        "labeled" | "unlabeled" => (name(&e["label"]["name"]), None),
        "assigned" | "unassigned" => (name(&e["assignee"]["login"]), None),
        "milestoned" | "demilestoned" => (name(&e["milestone"]["title"]), None),
        "review_requested" | "review_request_removed" => {
            (name(&e["requested_reviewer"]["login"]) + &name(&e["requested_team"]["name"]), None)
        }
        "renamed" => (format!("'{}' → '{}'", name(&e["rename"]["from"]), name(&e["rename"]["to"])), None),
        "closed" | "merged" | "referenced" => (name(&e["state_reason"]) + &name(&e["commit_id"]), None),
        "cross-referenced" => (
            format!(
                "{}#{}",
                name(&e["source"]["issue"]["repository"]["full_name"]),
                e["source"]["issue"]["number"].as_i64().unwrap_or(0)
            ),
            None,
        ),
        "commented" => (line1(s(&e["body"])), Some(s(&e["body"]).to_owned())),
        "reviewed" => (format!("{} {}", name(&e["state"]), line1(s(&e["body"]))), Some(s(&e["body"]).to_owned())),
        "committed" => (line1(s(&e["message"])), Some(s(&e["message"]).to_owned())),
        _ => (String::new(), None),
    }
}

async fn issue_timeline(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
    let repo = repo_arg(call)?;
    let number = id_arg(call, "number")?;
    let v = gh
        .call(
            token,
            Method::GET,
            &format!("/repos/{repo}/issues/{number}/timeline"),
            &[("per_page", limit(call).to_string())],
            None,
        )
        .await?;
    Ok(v.as_array()
        .into_iter()
        .flatten()
        .enumerate()
        .map(|(i, e)| {
            let event = s(&e["event"]);
            let (snippet, body) = timeline_detail(e);
            let actor = e["actor"]["login"]
                .as_str()
                .or_else(|| e["user"]["login"].as_str())
                .unwrap_or_else(|| e["author"]["name"].as_str().unwrap_or_default());
            let mut item = entry(&repo, format!("{i}:{event}"), event);
            item.from = text::one_line(actor);
            item.snippet = snippet;
            item.date = when(
                e.get("created_at")
                    .or_else(|| e.get("submitted_at"))
                    .or_else(|| e["author"].get("date"))
                    .unwrap_or(&Value::Null),
            );
            if let Some(body) = body {
                set_body(&mut item, &body);
            }
            item.extra = map(&json!({"event": event, "number": number}));
            item
        })
        .collect())
}

async fn assignee_list(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
    let repo = repo_arg(call)?;
    let v = gh
        .call(token, Method::GET, &format!("/repos/{repo}/assignees"), &[("per_page", limit(call).to_string())], None)
        .await?;
    Ok(v.as_array()
        .into_iter()
        .flatten()
        .map(|u| {
            let login = s(&u["login"]);
            let mut item = entry(&repo, login.to_owned(), login);
            "can be assigned".clone_into(&mut item.snippet);
            item
        })
        .collect())
}

async fn label_list(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
    let repo = repo_arg(call)?;
    let v = gh
        .call(token, Method::GET, &format!("/repos/{repo}/labels"), &[("per_page", limit(call).to_string())], None)
        .await?;
    Ok(v.as_array()
        .into_iter()
        .flatten()
        .map(|l| {
            let name = s(&l["name"]);
            let mut item = entry(&repo, name.to_owned(), name);
            item.snippet = text::one_line(s(&l["description"]));
            item.extra = map(&json!({"color": l["color"], "default": l["default"]}));
            item
        })
        .collect())
}

async fn milestone_list(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
    let repo = repo_arg(call)?;
    let query = [("state", call.str_arg("state").unwrap_or("open").to_owned()), ("per_page", limit(call).to_string())];
    let v = gh.call(token, Method::GET, &format!("/repos/{repo}/milestones"), &query, None).await?;
    Ok(v.as_array()
        .into_iter()
        .flatten()
        .map(|m| {
            let number = m["number"].as_i64().unwrap_or(0);
            let mut item = entry(&repo, number.to_string(), s(&m["title"]));
            item.snippet = format!(
                "#{number} · {} · {} open, {} closed{}",
                s(&m["state"]),
                m["open_issues"].as_i64().unwrap_or(0),
                m["closed_issues"].as_i64().unwrap_or(0),
                m["due_on"].as_str().map_or_else(String::new, |d| format!(" · due {d}"))
            );
            item.date = when(&m["updated_at"]);
            item.extra = map(&json!({"number": number, "state": m["state"], "due_on": m["due_on"],
                "open_issues": m["open_issues"], "closed_issues": m["closed_issues"], "url": m["html_url"]}));
            if let Some(d) = m["description"].as_str() {
                set_body(&mut item, d);
            }
            item
        })
        .collect())
}

// ---- Pull requests: reading -----------------------------------------------------------------------------------------

fn pr_state(pr: &Value) -> &'static str {
    if pr["merged"].as_bool() == Some(true) || !pr["merged_at"].is_null() {
        "merged"
    } else if pr["draft"].as_bool() == Some(true) {
        "draft"
    } else if pr["state"] == "closed" {
        "closed"
    } else {
        "open"
    }
}

fn pr_item(repo: &str, pr: &Value) -> Item {
    let number = pr["number"].as_i64().unwrap_or(0);
    let state = pr_state(pr);
    let head = s(&pr["head"]["ref"]);
    let base = s(&pr["base"]["ref"]);
    let mut item = entry(repo, format!("{repo}#{number}"), s(&pr["title"]));
    item.from = user_of(pr);
    item.snippet = format!("#{number} · {state} · {head} → {base}");
    item.date = when(&pr["updated_at"]);
    item.extra = map(&json!({"number": number, "state": state, "draft": pr["draft"], "head": head,
        "head_sha": pr["head"]["sha"], "base": base, "url": pr["html_url"],
        "labels": names(&pr["labels"], "name"), "created_at": pr["created_at"]}));
    item
}

async fn pr_list(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
    let repo = repo_arg(call)?;
    let mut query =
        vec![("state", call.str_arg("state").unwrap_or("open").to_owned()), ("per_page", limit(call).to_string())];
    if call.str_arg("head").is_some() {
        query.push(("head", head_arg(call)?));
    }
    if call.str_arg("base").is_some() {
        query.push(("base", branch_arg(call, "base")?));
    }
    for key in ["sort", "direction"] {
        if let Some(v) = call.str_arg(key) {
            query.push((key, v.to_owned()));
        }
    }
    let v = gh.call(token, Method::GET, &format!("/repos/{repo}/pulls"), &query, None).await?;
    Ok(v.as_array().into_iter().flatten().map(|pr| pr_item(&repo, pr)).collect())
}

/// How the checks of a commit stand: counts of check runs and statuses, and one word for them all.
async fn checks_summary(gh: &GitHub, token: &str, repo: &str, sha: &str) -> Value {
    if !sha_ok(sha) {
        return json!({"overall": "unknown"});
    }
    let runs = gh
        .call(
            token,
            Method::GET,
            &format!("/repos/{repo}/commits/{sha}/check-runs"),
            &[("per_page", "100".to_owned())],
            None,
        )
        .await
        .ok();
    let statuses = gh.call(token, Method::GET, &format!("/repos/{repo}/commits/{sha}/status"), &[], None).await.ok();
    if runs.is_none() && statuses.is_none() {
        return json!({"overall": "unknown"});
    }
    let (mut passed, mut failed, mut pending) = (0, 0, 0);
    for run in runs.iter().flat_map(|r| r["check_runs"].as_array().into_iter().flatten()) {
        match (s(&run["status"]), s(&run["conclusion"])) {
            ("completed", "success" | "neutral" | "skipped") => passed += 1,
            ("completed", _) => failed += 1,
            _ => pending += 1,
        }
    }
    for status in statuses.iter().flat_map(|r| r["statuses"].as_array().into_iter().flatten()) {
        match s(&status["state"]) {
            "success" => passed += 1,
            "failure" | "error" => failed += 1,
            _ => pending += 1,
        }
    }
    let overall = if failed > 0 {
        "failing"
    } else if pending > 0 {
        "pending"
    } else if passed > 0 {
        "passing"
    } else {
        "none"
    };
    json!({"overall": overall, "passed": passed, "failed": failed, "pending": pending})
}

async fn pr_get(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
    let repo = repo_arg(call)?;
    let number = id_arg(call, "number")?;
    let pr = subject(gh, token, &repo, number, true).await?;
    let checks = checks_summary(gh, token, &repo, s(&pr["head"]["sha"])).await;
    let mut item = pr_item(&repo, &pr);
    item.extra.extend(map(&json!({
        "merged": pr_state(&pr) == "merged",
        "mergeable": pr["mergeable"], "mergeable_state": pr["mergeable_state"],
        "head_repo": pr["head"]["repo"]["full_name"], "commits": pr["commits"],
        "additions": pr["additions"], "deletions": pr["deletions"], "changed_files": pr["changed_files"],
        "comments": pr["comments"], "review_comments": pr["review_comments"],
        "assignees": names(&pr["assignees"], "login"),
        "requested_reviewers": names(&pr["requested_reviewers"], "login"),
        "requested_teams": names(&pr["requested_teams"], "slug"),
        "milestone": pr["milestone"]["title"], "auto_merge": !pr["auto_merge"].is_null(),
        "maintainer_can_modify": pr["maintainer_can_modify"], "checks": checks,
    })));
    set_body(&mut item, s(&pr["body"]));
    Ok(vec![item])
}

async fn pr_files(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
    let repo = repo_arg(call)?;
    let number = id_arg(call, "number")?;
    let v = gh
        .call(
            token,
            Method::GET,
            &format!("/repos/{repo}/pulls/{number}/files"),
            &[("per_page", limit(call).to_string())],
            None,
        )
        .await?;
    let mut budget = MAX_DIFF;
    let mut items = Vec::new();
    for f in v.as_array().into_iter().flatten() {
        let name = s(&f["filename"]);
        let mut item = entry(&repo, name.to_owned(), name);
        item.snippet = format!(
            "{} · +{} −{}",
            s(&f["status"]),
            f["additions"].as_i64().unwrap_or(0),
            f["deletions"].as_i64().unwrap_or(0)
        );
        item.extra = map(&json!({"status": f["status"], "additions": f["additions"], "deletions": f["deletions"],
            "changes": f["changes"], "sha": f["sha"], "previous_filename": f["previous_filename"]}));
        match f["patch"].as_str() {
            Some(patch) => {
                let (t, was_cut) = cut(patch, budget);
                budget -= t.chars().count();
                if !t.is_empty() {
                    item.body = Some(t);
                }
                if was_cut {
                    item.extra.insert("truncated".to_owned(), json!(true));
                }
            }
            None => {
                item.extra.insert("patch_available".to_owned(), json!(false));
            }
        }
        items.push(item);
    }
    Ok(items)
}

async fn pr_diff(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
    let repo = repo_arg(call)?;
    let number = id_arg(call, "number")?;
    let max_text = MAX_DIFF * 4;
    let options = Options {
        accept: Some("application/vnd.github.diff"),
        max_text,
        ..Options::default()
    };
    let reply = gh.send(token, Method::GET, &format!("/repos/{repo}/pulls/{number}"), &[], None, &options).await?;
    let (diff, was_cut) = cut(&reply.text, MAX_DIFF);
    let mut item = entry(&repo, format!("{repo}#{number}.diff"), &format!("Diff of #{number}"));
    item.snippet = format!("{} characters", reply.text.chars().count());
    item.body = Some(diff).filter(|d| !d.is_empty());
    item.extra = map(&json!({"number": number, "truncated": was_cut || reply.text.len() >= max_text}));
    Ok(vec![item])
}

async fn pr_commits(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
    let repo = repo_arg(call)?;
    let number = id_arg(call, "number")?;
    let v = gh
        .call(
            token,
            Method::GET,
            &format!("/repos/{repo}/pulls/{number}/commits"),
            &[("per_page", limit(call).to_string())],
            None,
        )
        .await?;
    Ok(v.as_array()
        .into_iter()
        .flatten()
        .map(|c| {
            let sha = s(&c["sha"]);
            let mut item = entry(&repo, sha.to_owned(), &line1(s(&c["commit"]["message"])));
            item.from = c["author"]["login"]
                .as_str()
                .or_else(|| c["commit"]["author"]["name"].as_str())
                .map(text::one_line)
                .unwrap_or_default();
            item.snippet = sha.chars().take(7).collect();
            item.date = when(&c["commit"]["author"]["date"]);
            set_body(&mut item, s(&c["commit"]["message"]));
            item.extra = map(&json!({"sha": sha, "url": c["html_url"]}));
            item
        })
        .collect())
}

async fn pr_review_list(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
    let repo = repo_arg(call)?;
    let number = id_arg(call, "number")?;
    let v = gh
        .call(
            token,
            Method::GET,
            &format!("/repos/{repo}/pulls/{number}/reviews"),
            &[("per_page", limit(call).to_string())],
            None,
        )
        .await?;
    Ok(v.as_array()
        .into_iter()
        .flatten()
        .map(|r| {
            let id = r["id"].as_i64().unwrap_or(0);
            let mut item = entry(&repo, id.to_string(), s(&r["state"]));
            item.from = user_of(r);
            item.snippet = line1(s(&r["body"]));
            item.date = when(&r["submitted_at"]);
            set_body(&mut item, s(&r["body"]));
            item.extra =
                map(&json!({"id": id, "state": r["state"], "commit_id": r["commit_id"], "url": r["html_url"]}));
            item
        })
        .collect())
}

async fn pr_review_comment_list(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
    let repo = repo_arg(call)?;
    let number = id_arg(call, "number")?;
    let v = gh
        .call(
            token,
            Method::GET,
            &format!("/repos/{repo}/pulls/{number}/comments"),
            &[("per_page", limit(call).to_string())],
            None,
        )
        .await?;
    Ok(v.as_array()
        .into_iter()
        .flatten()
        .map(|c| {
            let id = c["id"].as_i64().unwrap_or(0);
            let line = c["line"].as_i64().or_else(|| c["original_line"].as_i64()).unwrap_or(0);
            let mut item = entry(&repo, id.to_string(), &format!("{}:{line}", s(&c["path"])));
            item.from = user_of(c);
            item.snippet = line1(s(&c["body"]));
            item.date = when(&c["created_at"]);
            set_body(&mut item, s(&c["body"]));
            item.extra = map(&json!({"id": id, "path": c["path"], "line": line, "side": c["side"],
                "in_reply_to_id": c["in_reply_to_id"], "commit_id": c["commit_id"], "url": c["html_url"],
                "diff_hunk": text::truncate_chars(s(&c["diff_hunk"]), 1_000)}));
            item
        })
        .collect())
}

// ---- Discussions (GraphQL) -------------------------------------------------------------------------------------------

fn owner_and_name(repo: &str) -> (&str, &str) {
    repo.split_once('/').unwrap_or((repo, ""))
}

async fn discussion_list(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
    const QUERY: &str = "query($owner: String!, $name: String!, $first: Int!) { repository(owner: $owner, name: $name) \
        { discussions(first: $first, orderBy: {field: UPDATED_AT, direction: DESC}) { nodes { number title url \
        createdAt updatedAt author { login } category { name } comments { totalCount } answerChosenAt } } } }";
    let repo = repo_arg(call)?;
    let (owner, name) = owner_and_name(&repo);
    let data = graphql(gh, token, QUERY, json!({"owner": owner, "name": name, "first": limit(call)})).await?;
    Ok(data["repository"]["discussions"]["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|d| {
            let number = d["number"].as_i64().unwrap_or(0);
            let mut item = entry(&repo, format!("{repo}#{number}"), s(&d["title"]));
            item.from = text::one_line(s(&d["author"]["login"]));
            item.snippet = format!("#{number} · {}", text::one_line(s(&d["category"]["name"])));
            item.date = when(&d["updatedAt"]);
            item.extra = map(&json!({"number": number, "url": d["url"], "comments": d["comments"]["totalCount"],
                "answered": !d["answerChosenAt"].is_null(), "category": d["category"]["name"]}));
            item
        })
        .collect())
}

async fn discussion_get(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
    const QUERY: &str = "query($owner: String!, $name: String!, $number: Int!) { repository(owner: $owner, name: $name) \
        { discussion(number: $number) { number title body url createdAt author { login } category { name } \
        answerChosenAt comments(first: 20) { totalCount nodes { author { login } body createdAt } } } } }";
    let repo = repo_arg(call)?;
    let number = id_arg(call, "number")?;
    let (owner, name) = owner_and_name(&repo);
    let data = graphql(gh, token, QUERY, json!({"owner": owner, "name": name, "number": number})).await?;
    let d = &data["repository"]["discussion"];
    if d.is_null() {
        return Err(fail("GitHub says that discussion does not exist, or discussions are off for the repository."));
    }
    let mut body = s(&d["body"]).to_owned();
    for c in d["comments"]["nodes"].as_array().into_iter().flatten() {
        write!(body, "\n\n@{} ({}):\n{}", s(&c["author"]["login"]), s(&c["createdAt"]), s(&c["body"])).ok();
    }
    let mut item = entry(&repo, format!("{repo}#{number}"), s(&d["title"]));
    item.from = text::one_line(s(&d["author"]["login"]));
    item.snippet = format!("#{number} · {}", text::one_line(s(&d["category"]["name"])));
    item.date = when(&d["createdAt"]);
    item.extra = map(&json!({"number": number, "url": d["url"], "comments": d["comments"]["totalCount"],
        "answered": !d["answerChosenAt"].is_null()}));
    set_body(&mut item, &body);
    Ok(vec![item])
}

// ---- Search -----------------------------------------------------------------------------------------------------------

/// The search text with the repository qualifier added when `repo` is given.
fn search_query(call: &ConnectorCall) -> Result<String, CoreError> {
    let q = call.str_arg("query").unwrap_or_default();
    if call.str_arg("repo").is_some() {
        Ok(format!("{q} repo:{}", repo_arg(call)?))
    } else {
        Ok(q.to_owned())
    }
}

async fn search_code(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
    let options = Options {
        accept: Some("application/vnd.github.text-match+json"),
        ..Options::default()
    };
    let query = [("q", search_query(call)?), ("per_page", limit(call).to_string())];
    let v = gh.send(token, Method::GET, "/search/code", &query, None, &options).await?.json;
    Ok(v["items"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|r| {
            let repo = s(&r["repository"]["full_name"]);
            let path = s(&r["path"]);
            let fragments: Vec<&str> =
                r["text_matches"].as_array().into_iter().flatten().filter_map(|m| m["fragment"].as_str()).collect();
            let mut item = entry(repo, format!("{repo}:{path}"), path);
            repo.clone_into(&mut item.from);
            item.snippet = fragments.first().map(|f| text::truncate_chars(&text::one_line(f), 200)).unwrap_or_default();
            item.extra = map(&json!({"path": path, "repo": repo, "sha": r["sha"], "url": r["html_url"]}));
            set_body(&mut item, &fragments.join("\n…\n"));
            item
        })
        .collect())
}

async fn search_repos(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
    let query = [("q", search_query(call)?), ("per_page", limit(call).to_string())];
    let v = gh.call(token, Method::GET, "/search/repositories", &query, None).await?;
    Ok(v["items"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|r| {
            let name = s(&r["full_name"]);
            let mut item = entry(name, name.to_owned(), name);
            item.from = text::one_line(s(&r["owner"]["login"]));
            item.snippet = text::truncate_chars(&text::one_line(s(&r["description"])), 200);
            item.date = when(&r["pushed_at"]);
            item.extra = map(&json!({"stars": r["stargazers_count"], "language": r["language"], "fork": r["fork"],
                "default_branch": r["default_branch"], "topics": r["topics"], "url": r["html_url"],
                "private": r["private"]}));
            item
        })
        .collect())
}

async fn search_commits(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
    let query = [("q", search_query(call)?), ("per_page", limit(call).to_string())];
    let v = gh.call(token, Method::GET, "/search/commits", &query, None).await?;
    Ok(v["items"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| {
            let repo = s(&c["repository"]["full_name"]);
            let sha = s(&c["sha"]);
            let mut item = entry(repo, format!("{repo}@{sha}"), &line1(s(&c["commit"]["message"])));
            item.from = c["author"]["login"]
                .as_str()
                .or_else(|| c["commit"]["author"]["name"].as_str())
                .map(text::one_line)
                .unwrap_or_default();
            item.snippet = format!("{repo} {}", sha.chars().take(7).collect::<String>());
            item.date = when(&c["commit"]["author"]["date"]);
            set_body(&mut item, s(&c["commit"]["message"]));
            item.extra = map(&json!({"sha": sha, "repo": repo, "url": c["html_url"]}));
            item
        })
        .collect())
}

async fn search_users(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
    let query = [("q", search_query(call)?), ("per_page", limit(call).to_string())];
    let v = gh.call(token, Method::GET, "/search/users", &query, None).await?;
    Ok(v["items"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|u| {
            let login = s(&u["login"]);
            let mut item = entry(login, login.to_owned(), login);
            item.parents = Vec::new();
            item.snippet = text::one_line(s(&u["type"]));
            item.extra = map(&json!({"type": u["type"], "url": u["html_url"]}));
            item
        })
        .collect())
}

async fn search_topics(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
    let query = [("q", search_query(call)?), ("per_page", limit(call).to_string())];
    let v = gh.call(token, Method::GET, "/search/topics", &query, None).await?;
    Ok(v["items"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|t| {
            let name = s(&t["name"]);
            let mut item = entry("search:topics", name.to_owned(), name);
            "GitHub topics".clone_into(&mut item.resource_label);
            item.parents = Vec::new();
            item.snippet = text::truncate_chars(&text::one_line(s(&t["short_description"])), 200);
            item.extra = map(&json!({"display_name": t["display_name"], "featured": t["featured"],
                "curated": t["curated"], "released": t["released"]}));
            item
        })
        .collect())
}

async fn search_labels(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
    let repo = repo_arg(call)?;
    let info = gh.call(token, Method::GET, &format!("/repos/{repo}"), &[], None).await?;
    let id = info["id"].as_i64().ok_or_else(|| fail("GitHub did not say which repository that is."))?;
    let query = [
        ("repository_id", id.to_string()),
        ("q", call.str_arg("query").unwrap_or_default().to_owned()),
        ("per_page", limit(call).to_string()),
    ];
    let v = gh.call(token, Method::GET, "/search/labels", &query, None).await?;
    Ok(v["items"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|l| {
            let name = s(&l["name"]);
            let mut item = entry(&repo, name.to_owned(), name);
            item.snippet = text::one_line(s(&l["description"]));
            item.extra = map(&json!({"color": l["color"]}));
            item
        })
        .collect())
}

// ---- Writes: the requests --------------------------------------------------------------------------------------------

/// One REST request a write consists of.
struct Req {
    method: Method,
    path: String,
    body: Option<Value>,
}

fn req(method: Method, path: String, body: Option<Value>) -> Req {
    Req {
        method,
        path,
        body,
    }
}

/// The arguments named in `keys` that are present, as a JSON object of the same names.
fn pick(call: &ConnectorCall, keys: &[&str]) -> Map<String, Value> {
    keys.iter().filter_map(|k| call.args.get(*k).map(|v| ((*k).to_owned(), v.clone()))).collect()
}

fn at_least_one(body: &Map<String, Value>, fields: &str) -> Result<(), CoreError> {
    if body.is_empty() {
        Err(fail(format!("Give at least one field to change: {fields}.")))
    } else {
        Ok(())
    }
}

/// Inline comments of a review, checked and reduced to the fields GitHub knows.
fn review_comments(call: &ConnectorCall) -> Result<Vec<Value>, CoreError> {
    let Some(raw) = call.args.get("comments") else {
        return Ok(Vec::new());
    };
    let list = raw.as_array().ok_or_else(|| fail("`comments` must be a list of {path, body, line}."))?;
    if list.len() > MAX_INLINE {
        return Err(fail(format!("`comments` may have at most {MAX_INLINE} entries.")));
    }
    let mut out = Vec::new();
    for c in list {
        let path = c["path"]
            .as_str()
            .filter(|p| path_ok(p))
            .ok_or_else(|| fail("Each inline comment needs a valid `path`."))?;
        let body = c["body"]
            .as_str()
            .filter(|b| !b.trim().is_empty())
            .ok_or_else(|| fail("Each inline comment needs a `body`."))?;
        let line = c["line"]
            .as_i64()
            .filter(|l| *l > 0)
            .ok_or_else(|| fail("Each inline comment needs a `line` (a positive number)."))?;
        let mut one = json!({"path": path, "body": body, "line": line});
        for key in ["side", "start_side"] {
            if let Some(side) = c.get(key).and_then(Value::as_str) {
                let side = side.to_ascii_uppercase();
                if side != "LEFT" && side != "RIGHT" {
                    return Err(fail(format!("`{key}` of an inline comment must be LEFT or RIGHT.")));
                }
                one[key] = json!(side);
            }
        }
        if let Some(start) = c.get("start_line") {
            one["start_line"] = json!(
                start.as_i64().filter(|l| *l > 0).ok_or_else(|| fail("`start_line` must be a positive number."))?
            );
        }
        out.push(one);
    }
    Ok(out)
}

/// The request of a simple write. `head_sha` is the pull request's head, needed to comment on its code; a preview
/// passes a stand-in.
#[allow(clippy::too_many_lines, reason = "one arm per operation")]
fn plan(call: &ConnectorCall, head_sha: Option<&str>) -> Result<Req, CoreError> {
    let repo = repo_arg(call)?;
    let base = format!("/repos/{repo}");
    let issue = |suffix: &str| -> Result<String, CoreError> {
        Ok(format!("{base}/issues/{}{suffix}", id_arg(call, "number")?))
    };
    let pull =
        |suffix: &str| -> Result<String, CoreError> { Ok(format!("{base}/pulls/{}{suffix}", id_arg(call, "number")?)) };
    let s_arg = |name: &str| call.str_arg(name).unwrap_or_default().to_owned();
    Ok(match call.op.as_str() {
        "create_issue" => {
            logins_arg(call, "assignees")?;
            req(
                Method::POST,
                format!("{base}/issues"),
                Some(Value::Object(pick(call, &["title", "body", "labels", "assignees", "milestone"]))),
            )
        }
        "comment" => req(Method::POST, issue("/comments")?, Some(Value::Object(pick(call, &["body"])))),
        "issue_update" => {
            logins_arg(call, "assignees")?;
            let mut body = pick(call, &["title", "body", "state", "state_reason", "labels", "assignees"]);
            if let Some(m) = call.int_arg("milestone") {
                body.insert(
                    "milestone".to_owned(),
                    if m == 0 {
                        Value::Null
                    } else {
                        json!(m)
                    },
                );
            }
            at_least_one(&body, "title, body, state, labels, assignees or milestone")?;
            if body.contains_key("state_reason") && !body.contains_key("state") {
                return Err(fail("`state_reason` goes together with `state`."));
            }
            req(Method::PATCH, issue("")?, Some(Value::Object(body)))
        }
        "issue_lock" => {
            let body = call.str_arg("reason").map_or_else(|| json!({}), |r| json!({"lock_reason": r}));
            req(Method::PUT, issue("/lock")?, Some(body))
        }
        "issue_unlock" => req(Method::DELETE, issue("/lock")?, None),
        "issue_comment_edit" => req(
            Method::PATCH,
            format!("{base}/issues/comments/{}", id_arg(call, "comment_id")?),
            Some(Value::Object(pick(call, &["body"]))),
        ),
        "issue_comment_delete" => {
            req(Method::DELETE, format!("{base}/issues/comments/{}", id_arg(call, "comment_id")?), None)
        }
        "issue_reaction_add" => {
            req(Method::POST, reaction_path(call, &repo)?, Some(json!({"content": s_arg("content")})))
        }
        "label_create" => {
            let mut body = pick(call, &["name", "description"]);
            if let Some(color) = color_arg(call)? {
                body.insert("color".to_owned(), json!(color));
            }
            req(Method::POST, format!("{base}/labels"), Some(Value::Object(body)))
        }
        "label_update" => {
            let mut body = pick(call, &["description"]);
            if let Some(n) = call.str_arg("new_name") {
                body.insert("new_name".to_owned(), json!(n));
            }
            if let Some(color) = color_arg(call)? {
                body.insert("color".to_owned(), json!(color));
            }
            at_least_one(&body, "new_name, color or description")?;
            req(Method::PATCH, format!("{base}/labels/{}", segment(&s_arg("name"))), Some(Value::Object(body)))
        }
        "label_delete" => req(Method::DELETE, format!("{base}/labels/{}", segment(&s_arg("name"))), None),
        "issue_label_add" => {
            let labels = strs(call, "labels");
            if labels.is_empty() {
                return Err(fail("Give at least one label."));
            }
            req(Method::POST, issue("/labels")?, Some(json!({"labels": labels})))
        }
        "issue_label_remove" => req(Method::DELETE, issue(&format!("/labels/{}", segment(&s_arg("label"))))?, None),
        "issue_assignee_add" | "issue_assignee_remove" => {
            let assignees = logins_arg(call, "assignees")?;
            if assignees.is_empty() {
                return Err(fail("Give at least one login."));
            }
            let method = if call.op == "issue_assignee_add" {
                Method::POST
            } else {
                Method::DELETE
            };
            req(method, issue("/assignees")?, Some(json!({"assignees": assignees})))
        }
        "milestone_create" => {
            let mut body = pick(call, &["title", "description", "state"]);
            if let Some(due) = call.str_arg("due_on") {
                body.insert("due_on".to_owned(), json!(due_iso(due)?));
            }
            req(Method::POST, format!("{base}/milestones"), Some(Value::Object(body)))
        }
        "milestone_update" => {
            let mut body = pick(call, &["title", "description", "state"]);
            if let Some(due) = call.str_arg("due_on") {
                body.insert(
                    "due_on".to_owned(),
                    if due.eq_ignore_ascii_case("none") {
                        Value::Null
                    } else {
                        json!(due_iso(due)?)
                    },
                );
            }
            at_least_one(&body, "title, description, due_on or state")?;
            req(Method::PATCH, format!("{base}/milestones/{}", id_arg(call, "milestone")?), Some(Value::Object(body)))
        }
        "milestone_delete" => req(Method::DELETE, format!("{base}/milestones/{}", id_arg(call, "milestone")?), None),
        "pr_create" => {
            let mut body = pick(call, &["title", "body", "draft", "maintainer_can_modify"]);
            body.insert("head".to_owned(), json!(head_arg(call)?));
            body.insert("base".to_owned(), json!(branch_arg(call, "base")?));
            req(Method::POST, format!("{base}/pulls"), Some(Value::Object(body)))
        }
        "pr_update" => {
            let mut body = pick(call, &["title", "body", "state", "maintainer_can_modify"]);
            if call.str_arg("base").is_some() {
                body.insert("base".to_owned(), json!(branch_arg(call, "base")?));
            }
            at_least_one(&body, "title, body, base, state or maintainer_can_modify")?;
            req(Method::PATCH, pull("")?, Some(Value::Object(body)))
        }
        "pr_review_create" => {
            let event = s_arg("event");
            let comments = review_comments(call)?;
            let has_text = call.str_arg("body").is_some_and(|b| !b.trim().is_empty());
            if event == "request_changes" && !has_text {
                return Err(fail("A review that requests changes needs a `body`."));
            }
            if event == "comment" && !has_text && comments.is_empty() {
                return Err(fail("A comment review needs a `body` or inline `comments`."));
            }
            let mut body = pick(call, &["body"]);
            body.insert("event".to_owned(), json!(event.to_ascii_uppercase()));
            if let Some(sha) = sha_arg(call, "commit_id")? {
                body.insert("commit_id".to_owned(), json!(sha));
            }
            if !comments.is_empty() {
                body.insert("comments".to_owned(), json!(comments));
            }
            req(Method::POST, pull("/reviews")?, Some(Value::Object(body)))
        }
        "pr_review_dismiss" => req(
            Method::PUT,
            pull(&format!("/reviews/{}/dismissals", id_arg(call, "review_id")?))?,
            Some(json!({"message": s_arg("message")})),
        ),
        "pr_review_comment_create" => {
            let body = s_arg("body");
            if let Some(reply_to) = call.int_arg("in_reply_to") {
                if reply_to <= 0 {
                    return Err(fail("`in_reply_to` must be a positive number."));
                }
                return Ok(req(
                    Method::POST,
                    pull(&format!("/comments/{reply_to}/replies"))?,
                    Some(json!({"body": body})),
                ));
            }
            let path = call
                .str_arg("path")
                .filter(|p| path_ok(p))
                .ok_or_else(|| fail("Give the `path` of the file, or `in_reply_to`."))?;
            let line = id_arg(call, "line").map_err(|_| fail("Give the `line` to comment on, or `in_reply_to`."))?;
            let commit = match sha_arg(call, "commit_id")? {
                Some(sha) => sha,
                None => head_sha.ok_or_else(|| fail("The pull request's head commit is unknown."))?.to_owned(),
            };
            let mut payload = json!({"body": body, "path": path, "line": line, "commit_id": commit,
                "side": call.str_arg("side").unwrap_or("right").to_ascii_uppercase()});
            if let Some(start) = call.int_arg("start_line") {
                payload["start_line"] = json!(start);
                payload["start_side"] =
                    json!(call.str_arg("start_side").or(call.str_arg("side")).unwrap_or("right").to_ascii_uppercase());
            }
            req(Method::POST, pull("/comments")?, Some(payload))
        }
        "pr_review_comment_edit" => req(
            Method::PATCH,
            format!("{base}/pulls/comments/{}", id_arg(call, "comment_id")?),
            Some(Value::Object(pick(call, &["body"]))),
        ),
        "pr_review_comment_delete" => {
            req(Method::DELETE, format!("{base}/pulls/comments/{}", id_arg(call, "comment_id")?), None)
        }
        "pr_reviewers_request" | "pr_reviewers_remove" => {
            let reviewers = logins_arg(call, "reviewers")?;
            let teams = logins_arg(call, "team_reviewers")?;
            if reviewers.is_empty() && teams.is_empty() {
                return Err(fail("Give `reviewers` or `team_reviewers`."));
            }
            let method = if call.op == "pr_reviewers_request" {
                Method::POST
            } else {
                Method::DELETE
            };
            req(method, pull("/requested_reviewers")?, Some(json!({"reviewers": reviewers, "team_reviewers": teams})))
        }
        "pr_update_branch" => {
            let body =
                sha_arg(call, "expected_head_sha")?.map_or_else(|| json!({}), |sha| json!({"expected_head_sha": sha}));
            req(Method::PUT, pull("/update-branch")?, Some(body))
        }
        "pr_merge" => {
            let method = call.str_arg("merge_method").unwrap_or("merge");
            if method == "rebase"
                && (call.str_arg("commit_title").is_some() || call.str_arg("commit_message").is_some())
            {
                return Err(fail(
                    "A rebase merge has no commit title or message of its own; leave them out or use merge or squash.",
                ));
            }
            let mut body = pick(call, &["commit_title", "commit_message"]);
            body.insert("merge_method".to_owned(), json!(method));
            if let Some(sha) = sha_arg(call, "sha")? {
                body.insert("sha".to_owned(), json!(sha));
            }
            req(Method::PUT, pull("/merge")?, Some(Value::Object(body)))
        }
        other => return Err(fail(format!("GitHub cannot {other}"))),
    })
}

fn due_iso(due: &str) -> Result<String, CoreError> {
    text::parse_when(due)
        .map(text::iso_utc)
        .ok_or_else(|| fail("`due_on` is not a date (2026-10-05 or 2026-10-05T14:00:00+02:00)."))
}

async fn perform_plain(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Value, CoreError> {
    let repo = repo_arg(call)?;
    let head = if call.op == "pr_review_comment_create"
        && call.int_arg("in_reply_to").is_none()
        && call.str_arg("commit_id").is_none()
    {
        let pr = subject(gh, token, &repo, id_arg(call, "number")?, true).await?;
        Some(s(&pr["head"]["sha"]).to_owned())
    } else {
        None
    };
    let r = plan(call, head.as_deref())?;
    let reply = gh.send(token, r.method, &r.path, &[], r.body.as_ref(), &Options::default()).await?;
    Ok(outcome(call, &reply.json))
}

/// The answer the AI gets after a simple write.
#[allow(clippy::too_many_lines, reason = "one arm per operation")]
fn outcome(call: &ConnectorCall, j: &Value) -> Value {
    let number = call.int_arg("number");
    match call.op.as_str() {
        "create_issue" => json!({"created": true, "number": j["number"], "url": j["html_url"], "id": j["id"]}),
        "comment" => json!({"commented": true, "id": j["id"], "url": j["html_url"]}),
        "issue_update" | "pr_update" => {
            json!({"updated": true, "number": j["number"], "state": j["state"], "url": j["html_url"]})
        }
        "issue_lock" => json!({"locked": true, "number": number}),
        "issue_unlock" => json!({"unlocked": true, "number": number}),
        "issue_comment_edit" | "pr_review_comment_edit" => {
            json!({"updated": true, "id": j["id"], "url": j["html_url"]})
        }
        "issue_comment_delete" | "pr_review_comment_delete" => {
            json!({"deleted": true, "comment_id": call.int_arg("comment_id")})
        }
        "issue_reaction_add" => json!({"reacted": true, "id": j["id"], "content": j["content"]}),
        "label_create" => json!({"created": true, "name": j["name"], "color": j["color"]}),
        "label_update" => json!({"updated": true, "name": j["name"], "color": j["color"]}),
        "label_delete" => json!({"deleted": true, "name": call.str_arg("name")}),
        "issue_label_add" => json!({"added": true, "labels": names(j, "name")}),
        "issue_label_remove" => json!({"removed": true, "labels": names(j, "name")}),
        "issue_assignee_add" => json!({"added": true, "assignees": names(&j["assignees"], "login")}),
        "issue_assignee_remove" => json!({"removed": true, "assignees": names(&j["assignees"], "login")}),
        "milestone_create" => json!({"created": true, "number": j["number"], "url": j["html_url"]}),
        "milestone_update" => json!({"updated": true, "number": j["number"], "state": j["state"]}),
        "milestone_delete" => json!({"deleted": true, "milestone": call.int_arg("milestone")}),
        "pr_create" => json!({"created": true, "number": j["number"], "url": j["html_url"], "draft": j["draft"]}),
        "pr_review_create" => json!({"reviewed": true, "id": j["id"], "state": j["state"], "url": j["html_url"]}),
        "pr_review_dismiss" => json!({"dismissed": true, "id": j["id"], "state": j["state"]}),
        "pr_review_comment_create" => json!({"created": true, "id": j["id"], "url": j["html_url"]}),
        "pr_reviewers_request" => {
            json!({"requested": true, "reviewers": names(&j["requested_reviewers"], "login"), "url": j["html_url"]})
        }
        "pr_reviewers_remove" => {
            json!({"removed": true, "reviewers": names(&j["requested_reviewers"], "login"), "url": j["html_url"]})
        }
        "pr_update_branch" => json!({"updating": true, "number": number, "message": j["message"]}),
        _ => json!({"done": true}),
    }
}

// ---- Writes: previews ------------------------------------------------------------------------------------------------

fn change(lines: &mut Vec<String>, what: &str, old: &str, new: &str) {
    lines.push(format!("{what}: '{}' → '{}'", text::one_line(old), text::one_line(new)));
}

#[allow(clippy::too_many_lines, reason = "one arm per operation")]
async fn preview_plain(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Preview, CoreError> {
    let repo = repo_arg(call)?;
    plan(call, Some("HEAD"))?;
    let n = call.int_arg("number").unwrap_or(0);
    let body = call.str_arg("body").unwrap_or_default();
    let mut lines = Vec::new();
    match call.op.as_str() {
        "create_issue" => {
            lines.push(format!("New issue in {repo}: {}", call.str_arg("title").unwrap_or_default()));
            for (label, key) in [("Labels", "labels"), ("Assignees", "assignees")] {
                if !call.list_arg(key).is_empty() {
                    lines.push(format!("{label}: {}", call.list_arg(key).join(", ")));
                }
            }
            if let Some(m) = call.int_arg("milestone") {
                lines.push(format!("Milestone: #{m}"));
            }
            if call.str_arg("body").is_some() {
                lines.push(shown(body));
            }
        }
        "comment" => {
            let it = subject(gh, token, &repo, n, false).await?;
            lines.push(format!("Comment on {repo}#{n}: {}", title_of(&it)));
            lines.push(shown(body));
        }
        "issue_update" | "pr_update" => {
            let pull = call.op == "pr_update";
            let it = subject(gh, token, &repo, n, pull).await?;
            lines.push(format!(
                "Update {}{repo}#{n}: {}",
                if pull {
                    "pull request "
                } else {
                    ""
                },
                title_of(&it)
            ));
            if let Some(t) = call.str_arg("title") {
                change(&mut lines, "title", s(&it["title"]), t);
            }
            if call.str_arg("body").is_some() {
                lines.push(format!(
                    "body replaced (was {} characters, now {}):",
                    s(&it["body"]).chars().count(),
                    body.chars().count()
                ));
                lines.push(shown(body));
            }
            if let Some(state) = call.str_arg("state") {
                let reason = call.str_arg("state_reason").map_or_else(String::new, |r| format!(" ({r})"));
                lines.push(format!("state: {} → {state}{reason}", s(&it["state"])));
            }
            if let Some(b) = call.str_arg("base") {
                change(&mut lines, "base branch", s(&it["base"]["ref"]), b);
            }
            if call.args.contains_key("labels") {
                lines.push(format!(
                    "labels: {} → {}",
                    or_none(&names(&it["labels"], "name")),
                    or_none(&strs(call, "labels"))
                ));
            }
            if call.args.contains_key("assignees") {
                lines.push(format!(
                    "assignees: {} → {}",
                    or_none(&names(&it["assignees"], "login")),
                    or_none(&strs(call, "assignees"))
                ));
            }
            if let Some(m) = call.int_arg("milestone") {
                let old = it["milestone"]["title"].as_str().unwrap_or("none");
                lines.push(format!(
                    "milestone: {} → {}",
                    text::one_line(old),
                    if m == 0 {
                        "none".to_owned()
                    } else {
                        format!("#{m}")
                    }
                ));
            }
            if let Some(v) = call.bool_arg("maintainer_can_modify") {
                lines.push(format!("maintainers can modify the head branch: {v}"));
            }
        }
        "issue_lock" | "issue_unlock" => {
            let it = subject(gh, token, &repo, n, false).await?;
            let verb = if call.op == "issue_lock" {
                "Lock"
            } else {
                "Unlock"
            };
            let reason = call.str_arg("reason").map_or_else(String::new, |r| format!(" (reason: {r})"));
            lines.push(format!("{verb} {repo}#{n}: {}{reason}", title_of(&it)));
        }
        "issue_comment_edit" | "issue_comment_delete" => {
            let id = id_arg(call, "comment_id")?;
            let c = gh.call(token, Method::GET, &format!("/repos/{repo}/issues/comments/{id}"), &[], None).await?;
            let verb = if call.op == "issue_comment_edit" {
                "Edit"
            } else {
                "Delete"
            };
            lines.push(format!(
                "{verb} comment {id} by @{} on {}",
                user_of(&c),
                s(&c["issue_url"]).rsplit('/').next().map_or_else(String::new, |i| format!("{repo}#{i}"))
            ));
            lines.push(format!("Current text: {}", shown(&text::truncate_chars(s(&c["body"]), 400))));
            if call.op == "issue_comment_edit" {
                lines.push(format!("New text: {}", shown(body)));
            }
        }
        "issue_reaction_add" => {
            let content = call.str_arg("content").unwrap_or_default();
            if let Some(id) = call.int_arg("comment_id") {
                lines.push(format!("React {content} to comment {id} in {repo}"));
            } else {
                let it = subject(gh, token, &repo, n, false).await?;
                lines.push(format!("React {content} to {repo}#{n}: {}", title_of(&it)));
            }
        }
        "issue_label_add" | "issue_label_remove" | "issue_assignee_add" | "issue_assignee_remove" => {
            let it = subject(gh, token, &repo, n, false).await?;
            let (verb, what, list, now) = match call.op.as_str() {
                "issue_label_add" => ("Add labels", "labels", strs(call, "labels"), names(&it["labels"], "name")),
                "issue_label_remove" => ("Remove label", "labels", strs(call, "label"), names(&it["labels"], "name")),
                "issue_assignee_add" => {
                    ("Assign", "assignees", strs(call, "assignees"), names(&it["assignees"], "login"))
                }
                _ => ("Unassign", "assignees", strs(call, "assignees"), names(&it["assignees"], "login")),
            };
            let list = if list.is_empty() {
                vec![call.str_arg("label").unwrap_or_default().to_owned()]
            } else {
                list
            };
            lines.push(format!("{verb} {} on {repo}#{n}: {}", list.join(", "), title_of(&it)));
            lines.push(format!("Current {what}: {}", or_none(&now)));
        }
        "label_create" => {
            lines.push(format!("Create label '{}' in {repo}", call.str_arg("name").unwrap_or_default()));
            if let Some(c) = color_arg(call)? {
                lines.push(format!("colour: {c}"));
            }
            if let Some(d) = call.str_arg("description") {
                lines.push(format!("description: {}", text::one_line(d)));
            }
        }
        "label_update" | "label_delete" => {
            let name = call.str_arg("name").unwrap_or_default();
            let l = gh.call(token, Method::GET, &format!("/repos/{repo}/labels/{}", segment(name)), &[], None).await?;
            if call.op == "label_delete" {
                lines.push(format!(
                    "Delete label '{}' from {repo} and from every issue that has it",
                    text::one_line(name)
                ));
                lines.push(format!("colour {}, description: {}", s(&l["color"]), text::one_line(s(&l["description"]))));
            } else {
                lines.push(format!("Edit label '{}' in {repo}", text::one_line(name)));
                if let Some(v) = call.str_arg("new_name") {
                    change(&mut lines, "name", name, v);
                }
                if let Some(v) = color_arg(call)? {
                    change(&mut lines, "colour", s(&l["color"]), &v);
                }
                if let Some(v) = call.str_arg("description") {
                    change(&mut lines, "description", s(&l["description"]), v);
                }
            }
        }
        "milestone_create" => {
            lines.push(format!("Create milestone '{}' in {repo}", call.str_arg("title").unwrap_or_default()));
            if let Some(d) = call.str_arg("due_on") {
                lines.push(format!("due: {}", due_iso(d)?));
            }
            if let Some(d) = call.str_arg("description") {
                lines.push(shown(d));
            }
        }
        "milestone_update" | "milestone_delete" => {
            let id = id_arg(call, "milestone")?;
            let m = gh.call(token, Method::GET, &format!("/repos/{repo}/milestones/{id}"), &[], None).await?;
            if call.op == "milestone_delete" {
                lines.push(format!(
                    "Delete milestone #{id} '{}' of {repo} ({} open, {} closed issues keep existing but lose it)",
                    title_of(&m),
                    m["open_issues"],
                    m["closed_issues"]
                ));
            } else {
                lines.push(format!("Edit milestone #{id} '{}' of {repo}", title_of(&m)));
                if let Some(v) = call.str_arg("title") {
                    change(&mut lines, "title", s(&m["title"]), v);
                }
                if let Some(v) = call.str_arg("state") {
                    change(&mut lines, "state", s(&m["state"]), v);
                }
                if let Some(v) = call.str_arg("due_on") {
                    change(&mut lines, "due", m["due_on"].as_str().unwrap_or("none"), v);
                }
                if let Some(v) = call.str_arg("description") {
                    lines.push("description replaced:".to_owned());
                    lines.push(shown(v));
                }
            }
        }
        "pr_create" => {
            let draft = if call.bool_arg("draft") == Some(true) {
                " (draft)"
            } else {
                ""
            };
            lines.push(format!("Open pull request in {repo}: {}{draft}", call.str_arg("title").unwrap_or_default()));
            lines.push(format!("{} → {}", head_arg(call)?, branch_arg(call, "base")?));
            if call.str_arg("body").is_some() {
                lines.push(shown(body));
            }
        }
        "pr_review_create" => {
            let it = subject(gh, token, &repo, n, true).await?;
            let verdict = match call.str_arg("event").unwrap_or_default() {
                "approve" => "approve",
                "request_changes" => "request changes",
                _ => "comment",
            };
            lines.push(format!("Review {repo}#{n} '{}': {verdict}", title_of(&it)));
            if call.str_arg("body").is_some() {
                lines.push(shown(body));
            }
            for c in review_comments(call)? {
                lines.push(format!(
                    "{}:{}: {}",
                    s(&c["path"]),
                    c["line"],
                    shown(&text::truncate_chars(s(&c["body"]), 400))
                ));
            }
        }
        "pr_review_dismiss" => {
            let id = id_arg(call, "review_id")?;
            let it = subject(gh, token, &repo, n, true).await?;
            let r = gh.call(token, Method::GET, &format!("/repos/{repo}/pulls/{n}/reviews/{id}"), &[], None).await?;
            lines.push(format!(
                "Dismiss the {} review {id} by @{} on {repo}#{n} '{}'",
                s(&r["state"]),
                user_of(&r),
                title_of(&it)
            ));
            lines.push(format!("Message: {}", shown(call.str_arg("message").unwrap_or_default())));
        }
        "pr_review_comment_create" => {
            let it = subject(gh, token, &repo, n, true).await?;
            if let Some(reply) = call.int_arg("in_reply_to") {
                lines.push(format!("Reply to review comment {reply} on {repo}#{n} '{}'", title_of(&it)));
            } else {
                lines.push(format!(
                    "Comment on {}:{} of {repo}#{n} '{}'",
                    call.str_arg("path").unwrap_or_default(),
                    call.int_arg("line").unwrap_or(0),
                    title_of(&it)
                ));
            }
            lines.push(shown(body));
        }
        "pr_review_comment_edit" | "pr_review_comment_delete" => {
            let id = id_arg(call, "comment_id")?;
            let c = gh.call(token, Method::GET, &format!("/repos/{repo}/pulls/comments/{id}"), &[], None).await?;
            let verb = if call.op == "pr_review_comment_edit" {
                "Edit"
            } else {
                "Delete"
            };
            lines.push(format!("{verb} review comment {id} by @{} on {}", user_of(&c), s(&c["path"])));
            lines.push(format!("Current text: {}", shown(&text::truncate_chars(s(&c["body"]), 400))));
            if call.op == "pr_review_comment_edit" {
                lines.push(format!("New text: {}", shown(body)));
            }
        }
        "pr_reviewers_request" | "pr_reviewers_remove" => {
            let it = subject(gh, token, &repo, n, true).await?;
            let verb = if call.op == "pr_reviewers_request" {
                "Request review from"
            } else {
                "Withdraw review requests from"
            };
            let mut who = strs(call, "reviewers");
            who.extend(strs(call, "team_reviewers").into_iter().map(|t| format!("team {t}")));
            lines.push(format!("{verb} {} on {repo}#{n} '{}'", who.join(", "), title_of(&it)));
            lines.push(format!("Currently requested: {}", or_none(&names(&it["requested_reviewers"], "login"))));
        }
        other => return Err(fail(format!("GitHub cannot {other}"))),
    }
    Ok(scoped(&repo, lines))
}

// ---- Draft state (GraphQL) -------------------------------------------------------------------------------------------

async fn preview_draft(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Preview, CoreError> {
    let repo = repo_arg(call)?;
    let n = id_arg(call, "number")?;
    let pr = subject(gh, token, &repo, n, true).await?;
    let ready = call.op == "pr_ready";
    let is_draft = pr["draft"].as_bool() == Some(true);
    if ready && !is_draft {
        return Err(fail(format!("{repo}#{n} is not a draft.")));
    }
    if !ready && (is_draft || pr["state"] != "open") {
        return Err(fail(format!("{repo}#{n} is already a draft or is not open.")));
    }
    let what = if ready {
        "Mark as ready for review"
    } else {
        "Convert to draft"
    };
    Ok(scoped(&repo, vec![format!("{what}: {repo}#{n} '{}'", title_of(&pr))]))
}

async fn perform_draft(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Value, CoreError> {
    let repo = repo_arg(call)?;
    let n = id_arg(call, "number")?;
    let pr = subject(gh, token, &repo, n, true).await?;
    let id = pr["node_id"].as_str().ok_or_else(|| fail("GitHub did not identify that pull request."))?;
    let (query, field, key) = if call.op == "pr_ready" {
        (
            "mutation($id: ID!) { markPullRequestReadyForReview(input: {pullRequestId: $id}) { pullRequest { number isDraft url } } }",
            "markPullRequestReadyForReview",
            "ready_for_review",
        )
    } else {
        (
            "mutation($id: ID!) { convertPullRequestToDraft(input: {pullRequestId: $id}) { pullRequest { number isDraft url } } }",
            "convertPullRequestToDraft",
            "draft",
        )
    };
    let data = graphql(gh, token, query, json!({"id": id})).await?;
    let done = &data[field]["pullRequest"];
    Ok(json!({key: true, "number": done["number"], "is_draft": done["isDraft"], "url": done["url"]}))
}

// ---- Transfer (GraphQL) ----------------------------------------------------------------------------------------------

fn to_repo_arg(call: &ConnectorCall) -> Result<String, CoreError> {
    let to = call.str_arg("to_repo").unwrap_or_default();
    if super::repo_ok(to) {
        Ok(to.to_owned())
    } else {
        Err(fail("`to_repo` must look like owner/name."))
    }
}

/// The issue and the destination repository, checked (a pull request cannot move; a repository cannot move to itself).
async fn transfer_parts(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<(String, Value, Value), CoreError> {
    let repo = repo_arg(call)?;
    let to = to_repo_arg(call)?;
    if to.eq_ignore_ascii_case(&repo) {
        return Err(fail("The destination is the repository the issue is already in."));
    }
    let n = id_arg(call, "number")?;
    let issue = subject(gh, token, &repo, n, false).await?;
    if issue.get("pull_request").is_some() {
        return Err(fail("A pull request cannot be transferred, only an issue."));
    }
    let dest = gh.call(token, Method::GET, &format!("/repos/{to}"), &[], None).await?;
    Ok((repo, issue, dest))
}

async fn preview_transfer(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Preview, CoreError> {
    let (repo, issue, dest) = transfer_parts(gh, token, call).await?;
    let n = id_arg(call, "number")?;
    let mut p = scoped(
        &repo,
        vec![
            format!("Transfer issue {repo}#{n} '{}' to {}", title_of(&issue), s(&dest["full_name"])),
            "The issue leaves this repository; labels and milestone that do not exist there are dropped.".to_owned(),
        ],
    );
    p.once_only = true;
    Ok(p)
}

async fn perform_transfer(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Value, CoreError> {
    const QUERY: &str = "mutation($issue: ID!, $repo: ID!) { transferIssue(input: {issueId: $issue, repositoryId: $repo}) \
        { issue { number url } } }";
    let (_, issue, dest) = transfer_parts(gh, token, call).await?;
    let (Some(issue_id), Some(repo_id)) = (issue["node_id"].as_str(), dest["node_id"].as_str()) else {
        return Err(fail("GitHub did not identify the issue or the destination."));
    };
    let data = graphql(gh, token, QUERY, json!({"issue": issue_id, "repo": repo_id})).await?;
    let moved = &data["transferIssue"]["issue"];
    Ok(json!({"transferred": true, "to": dest["full_name"], "number": moved["number"], "url": moved["url"]}))
}

// ---- Merging and updating the branch ---------------------------------------------------------------------------------

/// Everything a preview of merge or update-branch says about the pull request; the base branch is what it is about.
struct MergeView {
    pr: Value,
    base: String,
    default_branch: String,
}

async fn merge_view(gh: &GitHub, token: &str, repo: &str, n: i64) -> Result<MergeView, CoreError> {
    let pr = subject(gh, token, repo, n, true).await?;
    if pr["merged"].as_bool() == Some(true) {
        return Err(fail(format!("{repo}#{n} is already merged.")));
    }
    if pr["state"] != "open" {
        return Err(fail(format!("{repo}#{n} is closed.")));
    }
    let base = s(&pr["base"]["ref"]).to_owned();
    let default_branch = match pr["base"]["repo"]["default_branch"].as_str() {
        Some(d) => d.to_owned(),
        None => {
            s(&gh.call(token, Method::GET, &format!("/repos/{repo}"), &[], None).await?["default_branch"]).to_owned()
        }
    };
    Ok(MergeView {
        pr,
        base,
        default_branch,
    })
}

/// What the mergeable state means, in words.
fn mergeable_note(state: &str) -> &'static str {
    match state {
        "clean" => "ready to merge",
        "dirty" => "has merge conflicts",
        "blocked" => "blocked by required reviews or checks",
        "behind" => "the head branch is behind the base",
        "unstable" => "some non-required checks are failing",
        "draft" => "still a draft",
        "has_hooks" => "mergeable with passing checks and hooks",
        _ => "not computed yet",
    }
}

fn base_preview(repo: &str, view: &MergeView, lines: Vec<String>) -> Preview {
    let base = Some(view.base.as_str());
    Preview {
        resource: resource(repo, base),
        resource_label: resource_label(repo, base),
        lines,
        parents: parents(repo, base),
        once_only: false,
        ..Preview::default()
    }
}

fn direction(pr: &Value) -> String {
    let head_repo = s(&pr["head"]["repo"]["full_name"]);
    let base_repo = s(&pr["base"]["repo"]["full_name"]);
    let head = if head_repo.is_empty() || head_repo == base_repo {
        text::one_line(s(&pr["head"]["ref"]))
    } else {
        format!("{}:{}", text::one_line(head_repo), text::one_line(s(&pr["head"]["ref"])))
    };
    format!("{head} → {}", text::one_line(s(&pr["base"]["ref"])))
}

async fn preview_merge(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Preview, CoreError> {
    let repo = repo_arg(call)?;
    let n = id_arg(call, "number")?;
    plan(call, None)?;
    let view = merge_view(gh, token, &repo, n).await?;
    let pr = &view.pr;
    let head_sha = s(&pr["head"]["sha"]);
    if let Some(expected) = call.str_arg("sha").filter(|e| !head_sha.starts_with(e)) {
        return Err(fail(format!(
            "The head of {repo}#{n} is now {}, not {expected}: it changed since it was reviewed.",
            head_sha.chars().take(12).collect::<String>()
        )));
    }
    let checks = checks_summary(gh, token, &repo, head_sha).await;
    let method = call.str_arg("merge_method").unwrap_or("merge");
    let mut lines = vec![format!("Merge {repo}#{n} '{}' ({method}): {}", title_of(pr), direction(pr))];
    if view.base == view.default_branch {
        lines.push(format!("The base is the default branch ({}).", view.default_branch));
    } else {
        lines.push(format!("The base is {}, not the default branch ({}).", view.base, view.default_branch));
    }
    let mstate = s(&pr["mergeable_state"]);
    lines.push(format!(
        "Mergeable: {} ({})",
        if mstate.is_empty() {
            "unknown"
        } else {
            mstate
        },
        mergeable_note(mstate)
    ));
    lines.push(format!(
        "Checks: {} ({} passed, {} failed, {} pending)",
        checks["overall"].as_str().unwrap_or("unknown"),
        checks["passed"].as_i64().unwrap_or(0),
        checks["failed"].as_i64().unwrap_or(0),
        checks["pending"].as_i64().unwrap_or(0)
    ));
    lines.push(format!(
        "Head commit {}{}; {} commits, +{} −{} in {} files",
        head_sha.chars().take(12).collect::<String>(),
        if call.str_arg("sha").is_some() {
            " (matches the reviewed one)"
        } else {
            " (not pinned: anything pushed before the merge is included)"
        },
        pr["commits"].as_i64().unwrap_or(0),
        pr["additions"].as_i64().unwrap_or(0),
        pr["deletions"].as_i64().unwrap_or(0),
        pr["changed_files"].as_i64().unwrap_or(0)
    ));
    if let Some(t) = call.str_arg("commit_title") {
        lines.push(format!("Commit title: {}", text::one_line(t)));
    }
    if let Some(m) = call.str_arg("commit_message") {
        lines.push(format!("Commit message: {}", shown(m)));
    }
    Ok(base_preview(&repo, &view, lines))
}

async fn perform_merge(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Value, CoreError> {
    let repo = repo_arg(call)?;
    let n = id_arg(call, "number")?;
    let r = plan(call, None)?;
    let options = Options {
        allow: &[405, 409],
        ..Options::default()
    };
    let reply = gh.send(token, r.method, &r.path, &[], r.body.as_ref(), &options).await?;
    if reply.status == 200 && reply.json["merged"].as_bool() != Some(false) {
        return Ok(json!({"merged": true, "number": n, "sha": reply.json["sha"], "message": reply.json["message"],
            "method": call.str_arg("merge_method").unwrap_or("merge")}));
    }
    let message = text::truncate_chars(&text::one_line(s(&reply.json["message"])), 200);
    let reason = match reply.status {
        405 => "it cannot be merged right now (conflicts, missing reviews or checks, a draft, or branch rules)",
        409 => "the head branch changed since it was reviewed, or the merge conflicts",
        _ => "GitHub did not merge it",
    };
    Err(fail(format!("{repo}#{n} was not merged: {reason}. GitHub says: {message}")))
}

async fn preview_update_branch(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Preview, CoreError> {
    let repo = repo_arg(call)?;
    let n = id_arg(call, "number")?;
    plan(call, None)?;
    let view = merge_view(gh, token, &repo, n).await?;
    let pr = &view.pr;
    let lines = vec![
        format!("Update the branch of {repo}#{n} '{}': {}", title_of(pr), direction(pr)),
        format!(
            "Merges {} into {} with a new commit on {}.",
            view.base,
            text::one_line(s(&pr["head"]["ref"])),
            text::one_line(s(&pr["head"]["ref"]))
        ),
        format!("Head commit now {}", s(&pr["head"]["sha"]).chars().take(12).collect::<String>()),
    ];
    Ok(base_preview(&repo, &view, lines))
}
