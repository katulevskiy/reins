//! The org's GitHub App. Every action gets its own installation token, narrowed to the one repository it names and the
//! one permission it needs, and revoked right after; a lease is such a token handed (sealed) to the requester and
//! revoked when the server says so.

use reins_proto::managed::LeaseAccess;
use ring::signature::RsaKeyPair;
use serde_json::{Map, Value, json};
use zeroize::Zeroizing;

use super::{Connector, arg_list, arg_str, http_client, jwt_rs256, only_args, parse_rfc3339, required_str};

/// The tools the GitHub App connector runs.
pub const TOOLS: [&str; 4] = ["github_api_read", "github_api_write", "github_create_issue", "github_comment"];

/// How long an App JWT is valid (GitHub allows 10 minutes); issued a minute back for clock drift.
const JWT_SECS: i64 = 540;
const JWT_BACKDATE_SECS: i64 = 60;
/// The longest answer passed back to the agent.
const MAX_ANSWER: usize = 2 * 1024 * 1024;

pub struct GithubApp {
    app_id: u64,
    installation_id: u64,
    key: RsaKeyPair,
    api_base: String,
    http: reqwest::Client,
}

/// `owner/name`, both of GitHub's name characters.
fn split_repo(repo: &str) -> Result<(&str, &str), String> {
    let ok = |s: &str| {
        !s.is_empty()
            && s.len() <= 100
            && s != "."
            && s != ".."
            && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    };
    match repo.split_once('/') {
        Some((o, n)) if ok(o) && ok(n) => Ok((o, n)),
        _ => Err(format!("{repo:?} is not an owner/name repository")),
    }
}

/// The repository a REST path is under and the permission (`issues`, `contents`, …) its section needs. The org
/// executor only calls repository endpoints, and none of the administrative ones.
fn path_scope(path: &str) -> Result<(String, &'static str), String> {
    let clean = path.starts_with("/repos/")
        && !path.contains("..")
        && !path.contains("//")
        && !path.bytes().any(|b| b.is_ascii_whitespace() || b.is_ascii_control() || matches!(b, b'?' | b'#' | b'%'));
    if !clean {
        return Err("the path must be a plain /repos/<owner>/<name>/… path".to_owned());
    }
    let mut parts = path["/repos/".len()..].split('/');
    let (owner, name) = (parts.next().unwrap_or_default(), parts.next().unwrap_or_default());
    let repo = format!("{owner}/{name}");
    split_repo(&repo)?;
    let permission = match parts.next().unwrap_or_default() {
        "" => "metadata",
        "issues" | "labels" | "milestones" | "assignees" => "issues",
        "pulls" => "pull_requests",
        "contents" | "git" | "commits" | "branches" | "merges" | "compare" | "tags" | "releases" | "readme" => {
            "contents"
        }
        "actions" => "actions",
        "deployments" | "environments" => "deployments",
        "statuses" => "statuses",
        "check-runs" | "check-suites" => "checks",
        other => return Err(format!("the org executor does not call /repos/…/{other}")),
    };
    Ok((repo, permission))
}

fn query_pairs(args: &Value) -> Result<Vec<(String, String)>, String> {
    match args.get("query") {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Object(m)) => m
            .iter()
            .map(|(k, v)| match v {
                Value::String(s) => Ok((k.clone(), s.clone())),
                Value::Number(n) => Ok((k.clone(), n.to_string())),
                Value::Bool(b) => Ok((k.clone(), b.to_string())),
                _ => Err(format!("query `{k}` must be a string")),
            })
            .collect(),
        Some(_) => Err("`query` must be an object".to_owned()),
    }
}

/// What one tool call does: the repository and permission its token is narrowed to, and the request.
struct Call {
    repo: String,
    permissions: Value,
    method: reqwest::Method,
    path: String,
    query: Vec<(String, String)>,
    body: Option<Value>,
}

fn plan(tool: &str, args: &Value) -> Result<Call, String> {
    match tool {
        "github_create_issue" => {
            only_args(args, &["repo", "title", "body", "labels", "assignees", "milestone"])?;
            let repo = required_str(args, "repo")?;
            split_repo(repo)?;
            let mut body = Map::new();
            body.insert("title".into(), json!(required_str(args, "title")?));
            if let Some(b) = arg_str(args, "body") {
                body.insert("body".into(), json!(b));
            }
            for list in ["labels", "assignees"] {
                let items = arg_list(args, list)?;
                if !items.is_empty() {
                    body.insert(list.into(), json!(items));
                }
            }
            if let Some(m) = args.get("milestone").filter(|v| !v.is_null()) {
                body.insert("milestone".into(), json!(m.as_i64().ok_or("`milestone` must be a number")?));
            }
            Ok(Call {
                repo: repo.to_owned(),
                permissions: json!({"issues": "write"}),
                method: reqwest::Method::POST,
                path: format!("/repos/{repo}/issues"),
                query: Vec::new(),
                body: Some(Value::Object(body)),
            })
        }
        "github_comment" => {
            only_args(args, &["repo", "number", "body"])?;
            let repo = required_str(args, "repo")?;
            split_repo(repo)?;
            let number = args.get("number").and_then(Value::as_i64).filter(|n| *n > 0).ok_or("`number` is required")?;
            Ok(Call {
                repo: repo.to_owned(),
                permissions: json!({"issues": "write"}),
                method: reqwest::Method::POST,
                path: format!("/repos/{repo}/issues/{number}/comments"),
                query: Vec::new(),
                body: Some(json!({"body": required_str(args, "body")?})),
            })
        }
        "github_api_read" => {
            only_args(args, &["path", "query"])?;
            let path = required_str(args, "path")?;
            let (repo, permission) = path_scope(path)?;
            Ok(Call {
                repo,
                permissions: json!({ permission: "read" }),
                method: reqwest::Method::GET,
                path: path.to_owned(),
                query: query_pairs(args)?,
                body: None,
            })
        }
        "github_api_write" => {
            only_args(args, &["method", "path", "query", "body"])?;
            let method = match required_str(args, "method")? {
                "post" => reqwest::Method::POST,
                "put" => reqwest::Method::PUT,
                "patch" => reqwest::Method::PATCH,
                "delete" => reqwest::Method::DELETE,
                _ => return Err("`method` must be post, put, patch or delete".to_owned()),
            };
            let path = required_str(args, "path")?;
            let (repo, permission) = path_scope(path)?;
            if permission == "metadata" {
                return Err("the org executor does not change repository settings".to_owned());
            }
            Ok(Call {
                repo,
                permissions: json!({ permission: "write" }),
                method,
                path: path.to_owned(),
                query: query_pairs(args)?,
                body: args.get("body").filter(|v| !v.is_null()).cloned(),
            })
        }
        other => Err(format!("the GitHub App does not run {other}")),
    }
}

async fn error_of(what: &str, resp: reqwest::Response) -> String {
    let status = resp.status();
    let message = resp
        .json::<Value>()
        .await
        .ok()
        .and_then(|v| v.get("message").and_then(Value::as_str).map(str::to_owned))
        .unwrap_or_default();
    format!("{what}: GitHub answered {status} {message}").trim_end().to_owned()
}

impl GithubApp {
    /// `key_pem`: the App's private key; `api_base`: `https://api.github.com` (or a GitHub Enterprise API root).
    pub fn new(app_id: u64, installation_id: u64, key_pem: &str, api_base: &str) -> Result<Self, String> {
        Ok(Self {
            app_id,
            installation_id,
            key: super::rsa_from_pem(key_pem)?,
            api_base: api_base.trim_end_matches('/').to_owned(),
            http: http_client()?,
        })
    }

    /// The App's JWT (RS256, `iss` = the App id, nine minutes from a minute ago).
    pub fn jwt(&self, now: i64) -> Result<String, String> {
        jwt_rs256(
            &self.key,
            &json!({"iat": now - JWT_BACKDATE_SECS, "exp": now + JWT_SECS, "iss": self.app_id.to_string()}),
        )
    }

    fn request(&self, method: reqwest::Method, path: &str, bearer: &str) -> reqwest::RequestBuilder {
        self.http
            .request(method, format!("{}{path}", self.api_base))
            .bearer_auth(bearer)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
    }

    /// An installation token for `repo` (`owner/name`) only, with only `permissions`; and when it expires.
    pub async fn installation_token(
        &self,
        repo: &str,
        permissions: &Value,
    ) -> Result<(Zeroizing<String>, i64), String> {
        let (_, name) = split_repo(repo)?;
        let jwt = self.jwt(crate::now())?;
        let resp = self
            .request(reqwest::Method::POST, &format!("/app/installations/{}/access_tokens", self.installation_id), &jwt)
            .json(&json!({"repositories": [name], "permissions": permissions}))
            .send()
            .await
            .map_err(|e| format!("GitHub: {}", e.without_url()))?;
        if !resp.status().is_success() {
            return Err(error_of("minting a token", resp).await);
        }
        let v: Value = resp.json().await.map_err(|_| "GitHub's token answer is not JSON".to_owned())?;
        let token = v.get("token").and_then(Value::as_str).ok_or("GitHub's token answer has no token")?;
        let expires_at =
            v.get("expires_at").and_then(Value::as_str).and_then(parse_rfc3339).unwrap_or_else(|| crate::now() + 3600);
        Ok((Zeroizing::new(token.to_owned()), expires_at))
    }

    /// A credential for git: read (`contents: read`) or push (`contents: write`) on `repo` only.
    pub async fn lease(&self, repo: &str, access: LeaseAccess) -> Result<(Zeroizing<String>, i64), String> {
        let level = match access {
            LeaseAccess::Read => "read",
            LeaseAccess::Push => "write",
        };
        self.installation_token(repo, &json!({"contents": level})).await
    }

    /// Revokes an installation token upstream (`DELETE /installation/token`, authenticated by the token itself). A
    /// token GitHub no longer accepts is already dead.
    pub async fn revoke(&self, token: &str) -> Result<(), String> {
        let resp = self
            .request(reqwest::Method::DELETE, "/installation/token", token)
            .send()
            .await
            .map_err(|e| format!("GitHub: {}", e.without_url()))?;
        match resp.status().as_u16() {
            204 | 401 => Ok(()),
            _ => Err(error_of("revoking a token", resp).await),
        }
    }

    async fn call(&self, call: &Call, token: &str) -> Result<Value, String> {
        let mut req = self.request(call.method.clone(), &call.path, token);
        if !call.query.is_empty() {
            req = req.query(&call.query);
        }
        if let Some(body) = &call.body {
            req = req.json(body);
        }
        let resp = req.send().await.map_err(|e| format!("GitHub: {}", e.without_url()))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(error_of("the call", resp).await);
        }
        let bytes = resp.bytes().await.map_err(|e| format!("GitHub: {}", e.without_url()))?;
        if bytes.len() > MAX_ANSWER {
            return Err("GitHub's answer is too large".to_owned());
        }
        Ok(if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or(Value::Null)
        })
    }
}

/// The minimal result of a change: its number or id and link.
fn minimal(tool: &str, answer: &Value) -> Value {
    let pick = |keys: &[&str]| {
        let mut out = Map::new();
        for k in keys {
            if let Some(v) = answer.get(*k) {
                out.insert((*k).to_owned(), v.clone());
            }
        }
        Value::Object(out)
    };
    match tool {
        "github_create_issue" => pick(&["number", "html_url"]),
        "github_comment" => pick(&["id", "html_url"]),
        "github_api_write" => pick(&["id", "number", "html_url", "sha"]),
        _ => answer.clone(),
    }
}

#[async_trait::async_trait]
impl Connector for GithubApp {
    fn handles(&self, tool: &str) -> bool {
        TOOLS.contains(&tool)
    }

    async fn run(&self, _member_email: &str, tool: &str, args: &Value) -> Result<Value, String> {
        let call = plan(tool, args)?;
        let (token, _) = self.installation_token(&call.repo, &call.permissions).await?;
        let answer = self.call(&call, &token).await;
        // The token was for this one call.
        if let Err(e) = self.revoke(&token).await {
            log::warn!("revoking a one-call GitHub token: {e}");
        }
        answer.map(|a| minimal(tool, &a))
    }
}

#[cfg(test)]
mod tests {
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::connectors::testing::{jwt_claims, test_pem};

    async fn github() -> (MockServer, GithubApp) {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/app/installations/77/access_tokens"))
            .respond_with(
                ResponseTemplate::new(201)
                    .set_body_json(json!({"token": "ghs_narrow", "expires_at": "2026-10-27T11:00:00Z"})),
            )
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/installation/token"))
            .respond_with(ResponseTemplate::new(204))
            .mount(&server)
            .await;
        let app = GithubApp::new(123, 77, &test_pem(), &server.uri()).unwrap();
        (server, app)
    }

    async fn token_requests(server: &MockServer) -> Vec<wiremock::Request> {
        server
            .received_requests()
            .await
            .unwrap()
            .into_iter()
            .filter(|r| r.url.path() == "/app/installations/77/access_tokens")
            .collect()
    }

    #[tokio::test]
    async fn installation_tokens_are_narrowed_to_the_repo_and_action() {
        let (server, app) = github().await;
        Mock::given(method("POST"))
            .and(path("/repos/acme/api/issues"))
            .and(header("authorization", "Bearer ghs_narrow"))
            .respond_with(
                ResponseTemplate::new(201).set_body_json(json!({"number": 7, "html_url": "u", "body": "secret"})),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/repos/acme/web/pulls"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([{"number": 1}])))
            .expect(1)
            .mount(&server)
            .await;
        let args = json!({"repo": "acme/api", "title": "Flaky", "body": "x", "labels": ["bug"]});
        let out = app.run("lee@acme.com", "github_create_issue", &args).await.unwrap();
        assert_eq!(out, json!({"number": 7, "html_url": "u"}));
        let read = app
            .run(
                "lee@acme.com",
                "github_api_read",
                &json!({"path": "/repos/acme/web/pulls", "query": {"state": "open"}}),
            )
            .await
            .unwrap();
        assert_eq!(read, json!([{"number": 1}]));

        let tokens = token_requests(&server).await;
        assert_eq!(tokens.len(), 2);
        let body = |i: usize| serde_json::from_slice::<Value>(&tokens[i].body).unwrap();
        assert_eq!(body(0), json!({"repositories": ["api"], "permissions": {"issues": "write"}}));
        assert_eq!(body(1), json!({"repositories": ["web"], "permissions": {"pull_requests": "read"}}));
        let issue =
            server.received_requests().await.unwrap().into_iter().find(|r| r.url.path() == "/repos/acme/api/issues");
        assert_eq!(
            serde_json::from_slice::<Value>(&issue.unwrap().body).unwrap(),
            json!({"title": "Flaky", "body": "x", "labels": ["bug"]})
        );
        // Each one-call token is revoked after its call.
        let revoked =
            server.received_requests().await.unwrap().iter().filter(|r| r.method.as_str() == "DELETE").count();
        assert_eq!(revoked, 2);

        // Paths outside a repository, or administrative ones, are refused before anything is minted.
        for (tool, a) in [
            ("github_api_read", json!({"path": "/user"})),
            ("github_api_read", json!({"path": "/repos/acme/api/../../user"})),
            ("github_api_write", json!({"method": "put", "path": "/repos/acme/api/collaborators/eve"})),
            ("github_api_write", json!({"method": "delete", "path": "/repos/acme/api"})),
            ("github_create_issue", json!({"repo": "acme/api", "title": "t", "extra": 1})),
        ] {
            assert!(app.run("lee@acme.com", tool, &a).await.is_err(), "{a}");
        }
        assert_eq!(token_requests(&server).await.len(), 2);
    }

    #[tokio::test]
    async fn the_jwt_is_rs256_with_the_app_id() {
        let (server, app) = github().await;
        let before = crate::now();
        app.installation_token("acme/api", &json!({"issues": "read"})).await.unwrap();
        let req = &token_requests(&server).await[0];
        let auth = req.headers.get("authorization").unwrap().to_str().unwrap();
        let claims = jwt_claims(auth.strip_prefix("Bearer ").unwrap());
        assert_eq!(claims["iss"], "123");
        let (iat, exp) = (claims["iat"].as_i64().unwrap(), claims["exp"].as_i64().unwrap());
        assert!(iat <= before - 59 && iat >= before - 61, "{claims}");
        assert_eq!(exp - iat, 600);
    }

    #[tokio::test]
    async fn leases_are_revoked_with_delete_installation_token() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/app/installations/77/access_tokens"))
            .respond_with(
                ResponseTemplate::new(201)
                    .set_body_json(json!({"token": "ghs_lease", "expires_at": "2026-10-27T11:00:00Z"})),
            )
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/installation/token"))
            .and(header("authorization", "Bearer ghs_lease"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
        let app = GithubApp::new(123, 77, &test_pem(), &server.uri()).unwrap();
        let (token, expires) = app.lease("acme/api", LeaseAccess::Push).await.unwrap();
        assert_eq!(token.as_str(), "ghs_lease");
        assert_eq!(expires, parse_rfc3339("2026-10-27T11:00:00Z").unwrap());
        let minted = &token_requests(&server).await[0];
        assert_eq!(
            serde_json::from_slice::<Value>(&minted.body).unwrap(),
            json!({"repositories": ["api"], "permissions": {"contents": "write"}})
        );
        app.revoke(&token).await.unwrap();
    }
}
