//! GitHub, with a token the user pasted (a fine-grained token limits it to the repositories they chose). The token is
//! kept sealed in the local store, per account (the login it belongs to).

#![allow(dead_code, reason = "helpers shared by the tool areas, used as the areas grow")]

use std::sync::Arc;
use std::time::Duration;

use reins_proto::connector::{ConnectorCall, GITHUB};
use reqwest::{Method, StatusCode};
use serde_json::Value;

use super::{Connector, Item, Preview};
use crate::CoreError;
use crate::store::Store;
use crate::types::GmailStatus;

mod actions;
pub mod api;
mod code;
pub(crate) mod git;
mod issues;
mod repos;

pub const GITHUB_BASE: &str = "https://api.github.com";
const MAX_RETRIES: u32 = 2;
/// The most text of an issue with its comments handed over.
pub(super) const MAX_BODY: usize = 20_000;
/// The most of an answer that is kept or parsed.
const MAX_REPLY_BYTES: usize = 8 * 1024 * 1024;

/// What a failed answer means, in words for the user and the AI: GitHub's own message (short, one line) helps
/// ("Reference already exists", "Required status check is failing") and never carries a secret.
fn explain(status: StatusCode, body: &str) -> CoreError {
    let message = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v["message"].as_str().map(|m| crate::text::truncate_chars(&crate::text::one_line(m), 200)))
        .filter(|m| !m.is_empty());
    let with = |what: &str| match &message {
        Some(m) => CoreError::service(format!("{what}: {m}")),
        None => CoreError::service(what),
    };
    match status {
        StatusCode::UNAUTHORIZED => {
            CoreError::needs_attention("GitHub does not accept the token: it is mistyped, expired or revoked")
        }
        StatusCode::FORBIDDEN => with("GitHub refused: the token may not reach that, or the rate limit is used up"),
        StatusCode::NOT_FOUND => with("GitHub says that does not exist, or the token cannot see it"),
        StatusCode::UNPROCESSABLE_ENTITY => with("GitHub did not accept that"),
        StatusCode::CONFLICT | StatusCode::METHOD_NOT_ALLOWED => with("GitHub cannot do that right now"),
        other => with(&format!("GitHub answered {}", other.as_u16())),
    }
}

/// What every request in the account's name carries (`Accept` aside).
fn fetch_headers(token: &str) -> Vec<(String, String)> {
    vec![
        ("Authorization".to_owned(), format!("Bearer {token}")),
        ("X-GitHub-Api-Version".to_owned(), "2022-11-28".to_owned()),
        ("User-Agent".to_owned(), "reins".to_owned()),
    ]
}

pub struct GitHub {
    http: reqwest::Client,
    base: String,
    /// Where release assets are uploaded (`uploads.github.com` in production).
    upload_base: String,
    store: Arc<Store>,
    backoff: Duration,
}

/// `owner/name` with only the characters GitHub allows.
pub(super) fn repo_ok(repo: &str) -> bool {
    let mut parts = repo.split('/');
    let valid = |s: &str| {
        !s.is_empty()
            && s.len() <= 100
            && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    };
    matches!((parts.next(), parts.next(), parts.next()), (Some(o), Some(n), None) if valid(o) && valid(n))
        && !repo.contains("..")
}

pub(super) fn repo_arg(call: &ConnectorCall) -> Result<String, CoreError> {
    let repo = call.str_arg("repo").unwrap_or_default();
    if repo_ok(repo) {
        Ok(repo.to_owned())
    } else {
        Err(CoreError::service("`repo` must look like owner/name."))
    }
}

/// A user or organisation name (`owner`, `org`, `username` arguments).
pub(super) fn owner_ok(name: &str) -> bool {
    !name.is_empty() && name.len() <= 100 && name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}

/// A branch, tag or ref name git allows, so that it can go into a path or a query without surprises: no `..`, no
/// spaces or control characters, none of `~^:?*[\`, no leading `-` or `/`, no trailing `/`, `.` or `.lock`, no `@{`.
pub(super) fn ref_ok(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 250
        && !name.starts_with(['-', '/'])
        && !name.ends_with(['/', '.'])
        && !name.to_ascii_lowercase().ends_with(".lock")
        && !name.contains("..")
        && !name.contains("//")
        && !name.contains("@{")
        && name != "@"
        && !name
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || matches!(c, '~' | '^' | ':' | '?' | '*' | '[' | '\\'))
}

/// The optional branch/tag/ref argument `param` of a call, checked.
pub(super) fn ref_arg(call: &ConnectorCall, param: &str) -> Result<Option<String>, CoreError> {
    match call.str_arg(param) {
        None => Ok(None),
        Some(name) if ref_ok(name) => Ok(Some(name.to_owned())),
        Some(_) => Err(CoreError::service(format!("`{param}` is not a valid branch, tag or ref name."))),
    }
}

/// The permission resource for something in a repository: `owner/repo`, or `owner/repo@branch` when the operation is
/// about one branch. A permission for the repository covers its branches; one for a branch covers only that branch (and
/// branches under it, `feature` covers `feature/x`).
pub(super) fn resource(repo: &str, branch: Option<&str>) -> String {
    branch.map_or_else(|| repo.to_owned(), |b| format!("{repo}@{b}"))
}

/// The wider things `resource(repo, branch)` belongs to, nearest first, for offering broader permissions.
pub(super) fn parents(repo: &str, branch: Option<&str>) -> Vec<(String, String)> {
    let mut out = Vec::new();
    if branch.is_some() {
        out.push((repo.to_owned(), format!("Any branch of {repo}")));
    }
    if let Some((owner, _)) = repo.split_once('/') {
        out.push((owner.to_owned(), format!("Every repository of {owner}")));
    }
    out
}

/// The label of `resource(repo, branch)`.
pub(super) fn resource_label(repo: &str, branch: Option<&str>) -> String {
    branch.map_or_else(|| repo.to_owned(), |b| format!("{repo}, branch {b}"))
}

/// One answer from GitHub.
pub(super) struct Reply {
    pub status: u16,
    /// The body as JSON (`Null` when it is empty or not JSON).
    pub json: Value,
    /// The body as text, as far as `max_text` allowed.
    pub text: String,
    /// The `next` page of a listing, as a path and query.
    pub next: Option<String>,
}

/// How a request is sent; `Default` is a JSON API call.
#[derive(Default)]
pub(super) struct Options<'a> {
    /// The `Accept` header (`application/vnd.github.diff` for a diff, `application/vnd.github.raw+json` for file text).
    pub accept: Option<&'a str>,
    /// A raw body with its content type (a release asset), instead of JSON.
    pub raw: Option<(&'a str, Vec<u8>)>,
    /// Send to the uploads host instead of the API host.
    pub upload: bool,
    /// Follow a redirect to plain text (job logs) without the token; at most this many bytes of text are kept.
    pub max_text: usize,
    /// Statuses that are an answer, not an error (a check for existence: 404; a merge that cannot happen: 405, 409).
    pub allow: &'a [u16],
}

impl GitHub {
    pub fn new(http: reqwest::Client, base: &str, store: Arc<Store>, backoff: Duration) -> Self {
        Self {
            http,
            base: base.trim_end_matches('/').to_owned(),
            upload_base: "https://uploads.github.com".to_owned(),
            store,
            backoff,
        }
    }

    /// Uploads go to `base` (tests point this at a fake server).
    #[must_use]
    pub fn with_upload_base(mut self, base: &str) -> Self {
        base.trim_end_matches('/').clone_into(&mut self.upload_base);
        self
    }

    /// The headers of a request in the account's name, for the server to send for this one request ([`crate::blob`]).
    pub(super) fn headers(token: &str, accept: &str) -> Vec<(String, String)> {
        let mut headers = fetch_headers(token);
        headers.push(("Accept".to_owned(), accept.to_owned()));
        headers
    }

    fn token(&self, account: &str) -> Result<String, CoreError> {
        let raw = self
            .store
            .secret_get(GITHUB, account)?
            .ok_or_else(|| CoreError::needs_attention("GitHub is not connected"))?;
        String::from_utf8(raw).map_err(|_| CoreError::storage("corrupt GitHub token"))
    }

    /// A JSON API call. `path` starts with `/` and has been built from validated parts only; `query` is encoded here.
    pub(super) async fn call(
        &self,
        token: &str,
        method: Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<&Value>,
    ) -> Result<Value, CoreError> {
        Ok(self.send(token, method, path, query, body, &Options::default()).await?.json)
    }

    /// Any API call, with the answer's status, text and next page.
    pub(super) async fn send(
        &self,
        token: &str,
        method: Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<&Value>,
        options: &Options<'_>,
    ) -> Result<Reply, CoreError> {
        let mut attempt = 0;
        loop {
            let base = if options.upload {
                &self.upload_base
            } else {
                &self.base
            };
            let mut req = self
                .http
                .request(method.clone(), format!("{base}{path}"))
                .bearer_auth(token)
                .header("Accept", options.accept.unwrap_or("application/vnd.github+json"))
                .header("X-GitHub-Api-Version", "2022-11-28")
                .header("User-Agent", "reins")
                .query(query);
            if let Some((content_type, bytes)) = &options.raw {
                req = req.header("Content-Type", *content_type).body(bytes.clone());
            } else if let Some(body) = body {
                req = req.json(body);
            }
            let resp = req.send().await?;
            let status = resp.status();
            let next = resp
                .headers()
                .get("link")
                .and_then(|h| h.to_str().ok())
                .and_then(|h| h.split(',').find(|p| p.contains("rel=\"next\"")))
                .and_then(|p| p.split('<').nth(1))
                .and_then(|p| p.split('>').next())
                .and_then(|url| url.strip_prefix(self.base.as_str()))
                .map(str::to_owned);
            let bytes = resp.bytes().await?;
            let cap = if options.max_text == 0 {
                MAX_REPLY_BYTES
            } else {
                options.max_text
            };
            let text = String::from_utf8_lossy(&bytes[..bytes.len().min(cap)]).into_owned();
            if status.is_success() || options.allow.contains(&status.as_u16()) {
                return Ok(Reply {
                    status: status.as_u16(),
                    json: if bytes.len() > MAX_REPLY_BYTES {
                        Value::Null
                    } else {
                        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
                    },
                    text,
                    next,
                });
            }
            if (status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error()) && attempt < MAX_RETRIES {
                tokio::time::sleep(self.backoff * 2u32.pow(attempt)).await;
                attempt += 1;
                continue;
            }
            return Err(explain(status, &text));
        }
    }

    /// Every item of a listing, following the pages, at most `max` of them. `key` names the array in the answer when
    /// it is wrapped (`workflow_runs`, `items`); `None` when the answer is the array itself.
    pub(super) async fn pages(
        &self,
        token: &str,
        path: &str,
        query: &[(&str, String)],
        key: Option<&str>,
        max: usize,
    ) -> Result<Vec<Value>, CoreError> {
        let mut out = Vec::new();
        let mut reply = self.send(token, Method::GET, path, query, None, &Options::default()).await?;
        loop {
            let page = match key {
                Some(k) => reply.json[k].as_array().cloned(),
                None => reply.json.as_array().cloned(),
            }
            .unwrap_or_default();
            out.extend(page);
            if out.len() >= max {
                out.truncate(max);
                return Ok(out);
            }
            let Some(next) = reply.next.take() else {
                return Ok(out);
            };
            reply = self.send(token, Method::GET, &next, &[], None, &Options::default()).await?;
        }
    }

    /// Checks a pasted token and keeps it. Returns the account: the login it belongs to.
    pub async fn sign_in(&self, token: &str) -> Result<String, CoreError> {
        let token = token.trim();
        if token.is_empty() || token.len() > 300 || token.chars().any(|c| c.is_control() || c.is_whitespace()) {
            return Err(CoreError::invalid("that does not look like a GitHub token"));
        }
        let me = self.call(token, Method::GET, "/user", &[], None).await?;
        let login =
            me["login"].as_str().ok_or_else(|| CoreError::service("GitHub did not say who that is"))?.to_lowercase();
        self.store.secret_put(GITHUB, &login, token.as_bytes())?;
        Ok(login)
    }
}

#[async_trait::async_trait]
impl Connector for GitHub {
    fn service(&self) -> &'static str {
        GITHUB
    }

    async fn fetch(&self, account: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
        let token = self.token(account)?;
        let t = token.as_str();
        if let Some(result) = api::fetch(self, t, call).await {
            return result;
        }
        if let Some(result) = git::fetch(self, t, call).await {
            return result;
        }
        if let Some(result) = repos::fetch(self, t, call).await {
            return result;
        }
        if let Some(result) = code::fetch(self, t, call).await {
            return result;
        }
        if let Some(result) = issues::fetch(self, t, call).await {
            return result;
        }
        if let Some(result) = actions::fetch(self, t, call).await {
            return result;
        }
        Err(CoreError::service(format!("GitHub cannot {}", call.op)))
    }

    async fn preview(&self, account: &str, call: &ConnectorCall) -> Result<Preview, CoreError> {
        let token = self.token(account)?;
        let t = token.as_str();
        if let Some(result) = api::preview(call) {
            return result;
        }
        if let Some(result) = git::preview(call) {
            return result;
        }
        if let Some(result) = repos::preview(self, t, call).await {
            return result;
        }
        if let Some(result) = code::preview(self, t, call).await {
            return result;
        }
        if let Some(result) = issues::preview(self, t, call).await {
            return result;
        }
        if let Some(result) = actions::preview(self, t, call).await {
            return result;
        }
        Err(CoreError::service(format!("GitHub cannot {}", call.op)))
    }

    async fn perform(&self, account: &str, call: &ConnectorCall) -> Result<Value, CoreError> {
        let token = self.token(account)?;
        let t = token.as_str();
        if let Some(result) = api::perform(self, t, call).await {
            return result;
        }
        if let Some(result) = git::perform(t, call) {
            return result;
        }
        if let Some(result) = repos::perform(self, t, call).await {
            return result;
        }
        if let Some(result) = code::perform(self, t, call).await {
            return result;
        }
        if let Some(result) = issues::perform(self, t, call).await {
            return result;
        }
        if let Some(result) = actions::perform(self, t, call).await {
            return result;
        }
        Err(CoreError::service(format!("GitHub cannot {}", call.op)))
    }

    async fn sign_in_token(&self, token: &str) -> Result<String, CoreError> {
        self.sign_in(token).await
    }

    async fn fetch_headers(&self, account: &str) -> Result<Vec<(String, String)>, CoreError> {
        Ok(fetch_headers(&self.token(account)?))
    }

    /// Git through the desktop app names no account: the first whose token can see the repository.
    async fn choose_account(&self, accounts: &[String], call: &ConnectorCall) -> Result<Option<String>, CoreError> {
        if !call.spec().is_some_and(|s| s.desktop_only) {
            return Ok(None);
        }
        let repo = repo_arg(call)?;
        let unseen = Options {
            allow: &[401, 403, 404],
            ..Options::default()
        };
        for account in accounts {
            let Ok(token) = self.token(account) else {
                continue;
            };
            let reply = self.send(&token, Method::GET, &format!("/repos/{repo}"), &[], None, &unseen).await?;
            if (200..300).contains(&reply.status) {
                return Ok(Some(account.clone()));
            }
        }
        Err(CoreError::service(format!("None of the GitHub accounts on the user's phone can see {repo}.")))
    }

    async fn forget(&self, account: &str) {
        self.store.secret_delete(GITHUB, account).ok();
    }

    async fn status(&self, account: &str) -> GmailStatus {
        let Ok(token) = self.token(account) else {
            return GmailStatus::NeedsConsent;
        };
        match self.call(&token, Method::GET, "/user", &[], None).await {
            Ok(_) => GmailStatus::Ready,
            Err(CoreError::ServiceNeedsAttention {
                ..
            }) => GmailStatus::NeedsConsent,
            Err(e) => GmailStatus::Unavailable {
                message: e.to_string(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repository_names_are_checked_strictly() {
        for ok in ["octo/cat", "a-b/c_d.e", "Org/Repo.js"] {
            assert!(repo_ok(ok), "{ok}");
        }
        for bad in ["", "octo", "octo/", "/cat", "a/b/c", "a b/c", "../../etc", "a/..", "a/b?x=1", "a/b#1", "a%2Fb/c"] {
            assert!(!repo_ok(bad), "{bad:?}");
        }
        assert_eq!(super::super::calendar::segment("a b"), "a%20b");
    }

    #[test]
    fn branch_and_tag_names_cannot_smuggle_anything_into_a_path() {
        for ok in ["main", "feature/login", "release-1.2.3", "v1.0", "user/topic_2", "a@b"] {
            assert!(ref_ok(ok), "{ok}");
        }
        for bad in [
            "", "-x", "/x", "x/", "x.", "x.lock", "a..b", "a//b", "a b", "a~b", "a^b", "a:b", "a?b", "a*b", "a[b",
            "a\\b", "a@{b", "@", "a\nb",
        ] {
            assert!(!ref_ok(bad), "{bad:?}");
        }
    }

    #[test]
    fn resources_name_the_repository_and_the_branch_and_their_parents() {
        assert_eq!(resource("me/app", None), "me/app");
        assert_eq!(resource("me/app", Some("main")), "me/app@main");
        assert_eq!(
            parents("me/app", Some("main")),
            [
                ("me/app".to_owned(), "Any branch of me/app".to_owned()),
                ("me".to_owned(), "Every repository of me".to_owned())
            ]
        );
        assert_eq!(parents("me/app", None), [("me".to_owned(), "Every repository of me".to_owned())]);
        for (granted, thing) in [("me/app", "me/app@main"), ("me", "me/app"), ("me", "me/app@main")] {
            assert!(reins_policy::resource_covers(granted, thing));
        }
    }
}
