//! `github_api_read` and `github_api_write`: any GitHub REST call no dedicated tool covers, under the usual
//! permissions. The phone works out from the method and the path what the call touches (a repository, one branch of
//! it, an organization, the account, or GitHub at large) and, for a change, what kind of change it is; far-reaching
//! changes are asked for every time.
//!
//! The path is checked before anything else: it is a plain absolute path on the API host (no scheme, host, `//`, `..`
//! or `.` segment, no encoded slash or dot, no query or fragment, no control or non-ASCII character).

use reqwest::Method;
use rewarden_proto::connector::ConnectorCall;
use serde_json::{Map, Value, json};

use super::{GitHub, Options, Preview, owner_ok, parents, ref_ok, repo_ok, resource, resource_label};
use crate::connector::Item;
use crate::{CoreError, text};

/// The most characters of an answer handed to the AI.
const MAX_ANSWER_CHARS: usize = 100_000;
/// The most characters of a body the preview shows.
const MAX_BODY_SHOWN: usize = 2_000;
pub const ACCOUNT: &str = "account";
const ACCOUNT_LABEL: &str = "Your GitHub account";
pub const GITHUB_WIDE: &str = "github";
const GITHUB_WIDE_LABEL: &str = "GitHub (not one repository)";
/// Segments whose writes are asked for every time: who has access, hooks, keys, secrets, branch protection.
const ONCE_SEGMENTS: &[&str] = &[
    "collaborators",
    "invitations",
    "repository_invitations",
    "teams",
    "hooks",
    "secrets",
    "protection",
    "rulesets",
    "transfer",
];

/// What a call touches, for permissions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    /// `owner/repo`, `owner/repo@branch`, an organization, [`ACCOUNT`] or [`GITHUB_WIDE`].
    pub resource: String,
    pub label: String,
    /// The wider things it is part of, nearest first.
    pub parents: Vec<(String, String)>,
    /// For a change: issues, pulls, code, releases, actions, settings or account.
    pub class: &'static str,
    /// For a change: asked for every time.
    pub once: bool,
    /// For a read: the answer may hold credentials or private addresses (keys, emails), so it is never covered by a
    /// permission.
    pub sensitive: bool,
}

fn decode(segment: &str) -> Result<String, String> {
    let bytes = segment.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = segment.get(i + 1..i + 3).filter(|h| h.bytes().all(|b| b.is_ascii_hexdigit()));
            let value =
                hex.and_then(|h| u8::from_str_radix(h, 16).ok()).ok_or("a % must be followed by two hex digits")?;
            out.push(value);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| "an encoded character is not UTF-8".to_owned())
}

/// The segments of a path the AI gave, decoded, or why the path is refused.
pub fn path_segments(path: &str) -> Result<Vec<String>, String> {
    let refuse = |why: &str| Err(format!("The path is not allowed: {why}."));
    if !path.starts_with('/') {
        return refuse("it must start with /, like /repos/owner/name/pulls");
    }
    if path.len() > 500 {
        return refuse("it is longer than 500 characters");
    }
    if !path.bytes().all(|b| b.is_ascii_graphic()) {
        return refuse("it has spaces, control or non-ASCII characters (encode them)");
    }
    if path.contains(['?', '#']) {
        return refuse("put the query in `query`, and no # fragment");
    }
    if path.contains("://") || path.contains('\\') {
        return refuse("it must be a path on the GitHub API, without a scheme or host");
    }
    if path.contains("//") {
        return refuse("it has an empty segment (//)");
    }
    let mut segments = Vec::new();
    for raw in path[1..].split('/') {
        if raw.is_empty() {
            return refuse("it has an empty segment");
        }
        let decoded = decode(raw).map_err(|why| format!("The path is not allowed: {why}."))?;
        if decoded == "." || decoded == ".." {
            return refuse("`.` and `..` segments");
        }
        if decoded.contains(['/', '\\']) || decoded.chars().any(char::is_control) {
            return refuse("an encoded slash, backslash or control character");
        }
        if raw.ends_with(':') {
            return refuse("it must be a path on the GitHub API, without a scheme or host");
        }
        segments.push(decoded);
    }
    Ok(segments)
}

/// The branch a repository path is about, when it names one.
fn branch_of(rest: &[String], query: &[(String, String)], body: Option<&Value>) -> Option<String> {
    let named = match rest {
        [b, n, ..] if b == "branches" => Some(n.clone()),
        [g, r, h, name @ ..] if g == "git" && (r == "refs" || r == "ref") && h == "heads" && !name.is_empty() => {
            Some(name.join("/"))
        }
        [c, ..] if c == "contents" => query
            .iter()
            .find(|(k, _)| k == "ref")
            .map(|(_, v)| v.clone())
            .or_else(|| body.and_then(|b| b["branch"].as_str()).map(str::to_owned)),
        _ => None,
    };
    named.filter(|b| ref_ok(b))
}

/// The kind of change a write to a repository path is.
fn repo_class(rest: &[String]) -> &'static str {
    let first = rest.first().map(String::as_str);
    let second = rest.get(1).map(String::as_str);
    match first {
        Some("issues" | "labels" | "milestones" | "assignees") => "issues",
        Some("pulls") => "pulls",
        Some("releases" | "tags") => "releases",
        Some("git") if second == Some("tags") => "releases",
        Some("git") if rest.get(2).map(String::as_str) == Some("tags") => "releases",
        Some("branches") if rest.iter().any(|s| s == "protection") => "settings",
        Some("git" | "contents" | "commits" | "merges" | "merge-upstream" | "branches" | "comments") => "code",
        Some(
            "actions" | "environments" | "deployments" | "check-runs" | "check-suites" | "statuses" | "dispatches",
        ) => "actions",
        // The repository itself, who has access, hooks, keys, pages, topics, ...
        _ => "settings",
    }
}

/// What a call with `method` on `path` touches. `Err` says why the path is refused.
pub fn target(method: &str, path: &str, query: &[(String, String)], body: Option<&Value>) -> Result<Target, String> {
    let segs = path_segments(path)?;
    let root = segs.first().map(|s| s.to_ascii_lowercase()).unwrap_or_default();
    let write = method != "GET";
    let keyish = |s: &String| s.ends_with("keys") || s == "emails" || s == "public_emails";
    let mut once = write && segs.iter().any(|s| ONCE_SEGMENTS.contains(&s.as_str()) || keyish(s));
    let sensitive = !write && segs.iter().any(keyish);
    let (resource, label, parents_of, class) = match root.as_str() {
        "repos" => {
            let (Some(owner), Some(name)) = (segs.get(1), segs.get(2)) else {
                return Err("The path is not allowed: /repos/ must be followed by owner/name.".to_owned());
            };
            let repo = format!("{owner}/{name}");
            if !repo_ok(&repo) {
                return Err("The path is not allowed: /repos/ must be followed by owner/name.".to_owned());
            }
            let rest = &segs[3..];
            if write && rest.is_empty() {
                // Deleting the repository, or renaming it, changing who sees it, archiving it.
                let far = ["name", "visibility", "private", "archived"];
                once |= method == "DELETE"
                    || body.and_then(Value::as_object).is_some_and(|o| far.iter().any(|k| o.contains_key(*k)));
            }
            let branch = branch_of(rest, query, body);
            (
                resource(&repo, branch.as_deref()),
                resource_label(&repo, branch.as_deref()),
                parents(&repo, branch.as_deref()),
                repo_class(rest),
            )
        }
        "orgs" => {
            let Some(org) = segs.get(1).filter(|o| owner_ok(o)) else {
                return Err("The path is not allowed: /orgs/ must be followed by an organization name.".to_owned());
            };
            once |= method == "DELETE";
            (org.clone(), format!("Organization {org}"), Vec::new(), "settings")
        }
        "user" | "gists" | "notifications" => (ACCOUNT.to_owned(), ACCOUNT_LABEL.to_owned(), Vec::new(), "account"),
        _ => (GITHUB_WIDE.to_owned(), GITHUB_WIDE_LABEL.to_owned(), Vec::new(), "account"),
    };
    Ok(Target {
        resource,
        label,
        parents: parents_of,
        class,
        once,
        sensitive,
    })
}

/// The query of a call: names and values as given, names checked.
fn query_of(call: &ConnectorCall) -> Result<Vec<(String, String)>, CoreError> {
    let mut out = Vec::new();
    if let Some(Value::Object(map)) = call.args.get("query") {
        for (k, v) in map {
            let ok = !k.is_empty()
                && k.len() <= 100
                && k.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b'[' | b']'));
            let value = v.as_str().unwrap_or_default();
            if !ok || value.chars().any(char::is_control) {
                return Err(CoreError::service(format!(
                    "The query parameter `{}` is not allowed.",
                    text::truncate_chars(&text::one_line(k), 40)
                )));
            }
            out.push((k.clone(), value.to_owned()));
        }
    }
    Ok(out)
}

fn body_of(call: &ConnectorCall) -> Option<&Value> {
    call.args.get("body").filter(|b| !b.is_null())
}

/// The method of a change, in either case (the tool's choice is normalized to lower case).
fn method_of(call: &ConnectorCall) -> Result<Method, CoreError> {
    match call.str_arg("method").unwrap_or_default().to_ascii_uppercase().as_str() {
        "POST" => Ok(Method::POST),
        "PUT" => Ok(Method::PUT),
        "PATCH" => Ok(Method::PATCH),
        "DELETE" => Ok(Method::DELETE),
        _ => Err(CoreError::service("`method` must be POST, PUT, PATCH or DELETE.")),
    }
}

type Query = Vec<(String, String)>;

/// The call's path, query and target.
fn parts(call: &ConnectorCall, method: &Method) -> Result<(String, Query, Target), CoreError> {
    let path = call.str_arg("path").unwrap_or_default().to_owned();
    let query = query_of(call)?;
    let target = target(method.as_str(), &path, &query, body_of(call)).map_err(CoreError::service)?;
    Ok((path, query, target))
}

/// Text for the AI: at most [`MAX_ANSWER_CHARS`], and whether it was cut.
fn bounded(raw: &str) -> (String, bool) {
    let cut = text::truncate_chars(raw, MAX_ANSWER_CHARS);
    let was_cut = cut.len() < raw.len();
    (cut, was_cut)
}

fn cut_note(total: usize) -> String {
    format!(
        "The answer was cut at {MAX_ANSWER_CHARS} of {total} characters; ask for less (per_page, page, or a narrower \
         path)."
    )
}

/// `github_api_read`.
pub(super) async fn fetch(gh: &GitHub, token: &str, call: &ConnectorCall) -> Option<Result<Vec<Item>, CoreError>> {
    (call.op == "api_read").then_some(())?;
    Some(read(gh, token, call).await)
}

async fn read(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
    let (path, query, target) = parts(call, &Method::GET)?;
    let q: Vec<(&str, String)> = query.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
    let options = Options {
        // A redirect leads to a download (an archive, a log): handed over as a link.
        allow: &[301, 302, 303, 307, 308],
        ..Options::default()
    };
    let reply = gh.send(token, Method::GET, &path, &q, None, &options).await?;
    let mut item = Item {
        id: "api".to_owned(),
        resource: target.resource,
        resource_label: target.label,
        parents: target.parents,
        title: format!("GET {}", text::truncate_chars(&path, 200)),
        sensitive: target.sensitive,
        ..Item::default()
    };
    let mut extra = Map::new();
    extra.insert("status".to_owned(), json!(reply.status));
    let binary = (300..400).contains(&reply.status)
        || (reply.json.is_null() && (reply.text.contains('\0') || reply.text.contains('\u{FFFD}')));
    if binary {
        if crate::blob::linking() {
            let mut url = url::Url::parse(&format!("{}{path}", gh.base))
                .map_err(|_| CoreError::service("That path cannot be downloaded."))?;
            for (k, v) in &query {
                url.query_pairs_mut().append_pair(k, v);
            }
            let name = path.rsplit('/').next().unwrap_or("download");
            let marker = crate::blob::fetch_marker(url.as_str(), "", name, u64::MAX);
            extra.insert(crate::blob::DELIVER_KEY.to_owned(), marker);
            item.snippet = format!("answered {} · a file", reply.status);
        } else {
            item.snippet = format!("answered {} · a file, not shown here", reply.status);
            extra.insert("binary".to_owned(), json!(true));
        }
    } else {
        let raw = if reply.json.is_null() {
            reply.text.clone()
        } else {
            serde_json::to_string_pretty(&reply.json).unwrap_or_default()
        };
        let total = raw.chars().count();
        let (body, cut) = bounded(&raw);
        item.snippet = format!("answered {} · {total} characters", reply.status);
        item.body = Some(body);
        extra.insert("truncated".to_owned(), json!(cut));
        if cut {
            extra.insert("note".to_owned(), json!(cut_note(total)));
        }
    }
    if let Some(next) = reply.next {
        extra.insert("next_path".to_owned(), json!(next));
    }
    item.extra = extra;
    Ok(vec![item])
}

/// `github_api_write`: what it would do.
pub(super) fn preview(call: &ConnectorCall) -> Option<Result<Preview, CoreError>> {
    (call.op == "api_write").then_some(())?;
    Some(write_preview(call))
}

fn write_preview(call: &ConnectorCall) -> Result<Preview, CoreError> {
    let method = method_of(call)?;
    let (path, query, target) = parts(call, &method)?;
    let mut lines = vec![format!("{method} {path}")];
    if !query.is_empty() {
        let shown: Vec<String> = query.iter().map(|(k, v)| format!("{k}={}", text::one_line(v))).collect();
        lines.push(format!("Query: {}", text::truncate_chars(&shown.join("&"), 500)));
    }
    if let Some(body) = body_of(call) {
        let pretty = serde_json::to_string_pretty(body).unwrap_or_default();
        let total = pretty.chars().count();
        let shown = text::truncate_chars(&pretty, MAX_BODY_SHOWN);
        lines.push(if total > MAX_BODY_SHOWN {
            format!("Body:\n{shown}\n… ({total} characters in all)")
        } else {
            format!("Body:\n{shown}")
        });
    }
    Ok(Preview {
        resource: target.resource,
        resource_label: target.label,
        lines,
        parents: target.parents,
        once_only: target.once,
        class: Some(target.class.to_owned()),
        blob: None,
    })
}

/// `github_api_write`: does it and hands back GitHub's answer.
pub(super) async fn perform(gh: &GitHub, token: &str, call: &ConnectorCall) -> Option<Result<Value, CoreError>> {
    (call.op == "api_write").then_some(())?;
    Some(write(gh, token, call).await)
}

async fn write(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Value, CoreError> {
    let method = method_of(call)?;
    let (path, query, _) = parts(call, &method)?;
    let q: Vec<(&str, String)> = query.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
    let reply = gh.send(token, method.clone(), &path, &q, body_of(call), &Options::default()).await?;
    let mut out = json!({"done": true, "method": method.as_str(), "path": path, "status": reply.status});
    if !reply.json.is_null() {
        let raw = serde_json::to_string(&reply.json).unwrap_or_default();
        if raw.chars().count() > MAX_ANSWER_CHARS {
            let (cut, _) = bounded(&raw);
            out["response_text"] = json!(cut);
            out["truncated"] = json!(true);
            out["note"] = json!(cut_note(raw.chars().count()));
        } else {
            out["response"] = reply.json;
        }
    } else if !reply.text.is_empty() {
        let (cut, was_cut) = bounded(&reply.text);
        out["response_text"] = json!(cut);
        out["truncated"] = json!(was_cut);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(method: &str, path: &str) -> Target {
        target(method, path, &[], None).unwrap()
    }

    #[test]
    fn resources_follow_the_path() {
        assert_eq!(t("GET", "/repos/octo/cat/pulls/3").resource, "octo/cat");
        assert_eq!(t("GET", "/repos/octo/cat/branches/main/protection").resource, "octo/cat@main");
        assert_eq!(t("PATCH", "/repos/octo/cat/git/refs/heads/feature/x").resource, "octo/cat@feature/x");
        assert_eq!(t("GET", "/orgs/acme/teams").resource, "acme");
        assert_eq!(t("GET", "/user/starred").resource, ACCOUNT);
        assert_eq!(t("GET", "/search/code").resource, GITHUB_WIDE);
        let q = vec![("ref".to_owned(), "dev".to_owned())];
        assert_eq!(target("GET", "/repos/o/r/contents/a.txt", &q, None).unwrap().resource, "o/r@dev");
    }

    #[test]
    fn segments_cannot_smuggle() {
        for bad in ["/repos/o/r/%2e%2e/x", "/repos/o/r/a%2Fb", "/x/%zz", "/a/https:", "/a/%00"] {
            assert!(path_segments(bad).is_err(), "{bad}");
        }
        assert_eq!(path_segments("/repos/o/r/compare/main...dev").unwrap().len(), 5);
    }
}
