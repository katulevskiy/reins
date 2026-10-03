//! The GitHub `actions` area against a fake server: workflows, runs, logs, variables, secrets, environments, security
//! alerts, the user's own account and the generic request. For every tool: what is sent and what comes back.

#![allow(clippy::format_collect, reason = "test logs are built from formatted lines")]

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::FakeKeys;
use crypto_box::aead::OsRng;
use data_encoding::BASE64;
use rewarden_core::connector::github::GitHub;
use rewarden_core::connector::{Connector, Item};
use rewarden_core::store::Store;
use rewarden_core::{CoreError, GmailStatus};
use rewarden_proto::connector::{ConnectorCall, spec_for_tool};
use serde_json::{Value, json};
use wiremock::matchers::{body_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const ME: &str = "octo-cat";

fn call(tool: &str, args: &Value) -> ConnectorCall {
    spec_for_tool(tool)
        .unwrap_or_else(|| panic!("no tool {tool}"))
        .parse(args)
        .unwrap_or_else(|e| panic!("{tool}: {e}"))
        .0
}

async fn github(server: &MockServer) -> (GitHub, tempfile::TempDir) {
    Mock::given(method("GET"))
        .and(path("/user"))
        .and(header("authorization", "Bearer ghp_secret"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"login": "Octo-Cat"})))
        .mount(server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(dir.path(), &FakeKeys).unwrap());
    let github = GitHub::new(rewarden_core::http::client().unwrap(), &server.uri(), store, Duration::from_millis(1));
    assert_eq!(github.sign_in(" ghp_secret ").await.unwrap(), ME);
    (github, dir)
}

async fn json_at(server: &MockServer, p: &str, body: &Value) {
    Mock::given(method("GET"))
        .and(path(p))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

async fn status_at(server: &MockServer, verb: &str, p: &str, status: u16, body: &Value) {
    Mock::given(method(verb))
        .and(path(p))
        .respond_with(ResponseTemplate::new(status).set_body_json(body))
        .mount(server)
        .await;
}

async fn fetch(gh: &GitHub, tool: &str, args: &Value) -> Vec<Item> {
    gh.fetch(ME, &call(tool, args)).await.unwrap_or_else(|e| panic!("{tool}: {e}"))
}

async fn fetch_err(gh: &GitHub, tool: &str, args: &Value) -> CoreError {
    gh.fetch(ME, &call(tool, args)).await.expect_err(tool)
}

/// Requests the fake server got besides the sign-in.
async fn requests(server: &MockServer) -> Vec<wiremock::Request> {
    server.received_requests().await.unwrap().into_iter().filter(|r| r.url.path() != "/user").collect()
}

fn text(e: &CoreError) -> String {
    e.to_string()
}

// ---- Reads ---------------------------------------------------------------------------------------------------------

struct Read {
    tool: &'static str,
    args: Value,
    path: &'static str,
    query: Vec<(&'static str, &'static str)>,
    reply: Value,
    id: &'static str,
    title: &'static str,
    snippet: &'static str,
    resource: &'static str,
}

async fn check_read(r: Read) -> Vec<Item> {
    let server = MockServer::start().await;
    let (gh, _dir) = github(&server).await;
    let mut mock = Mock::given(method("GET")).and(path(r.path));
    for (k, v) in &r.query {
        mock = mock.and(query_param(*k, *v));
    }
    mock.respond_with(ResponseTemplate::new(200).set_body_json(&r.reply))
        .with_priority(1)
        .expect(1)
        .mount(&server)
        .await;
    let items = fetch(&gh, r.tool, &r.args).await;
    let first = items.first().unwrap_or_else(|| panic!("{}: no items", r.tool));
    assert_eq!(first.id, r.id, "{}", r.tool);
    assert!(first.title.contains(r.title), "{}: title {:?}", r.tool, first.title);
    assert!(first.snippet.contains(r.snippet), "{}: snippet {:?}", r.tool, first.snippet);
    assert_eq!(first.resource, r.resource, "{}", r.tool);
    assert!(!first.resource_label.is_empty(), "{}", r.tool);
    for item in &items {
        assert!(!item.sensitive, "{} items are ordinary", r.tool);
    }
    items
}

fn run_json(id: i64) -> Value {
    json!({"id": id, "name": "CI", "display_title": "Fix the build", "run_number": 12, "run_attempt": 1, "workflow_id": 161_335,
        "status": "completed", "conclusion": "failure", "head_branch": "main", "head_sha": "abc", "event": "push",
        "actor": {"login": "ann"}, "html_url": "https://github.com/octo/cat/actions/runs/77",
        "created_at": "2026-09-01T10:00:00Z", "run_started_at": "2026-09-01T10:00:05Z", "updated_at": "2026-09-01T10:05:00Z",
        "head_commit": {"message": "Fix\n\nlonger"}})
}

fn job_json(id: i64) -> Value {
    json!({"id": id, "run_id": 77, "run_attempt": 1, "name": "build", "status": "completed", "conclusion": "failure",
        "started_at": "2026-09-01T10:00:00Z", "completed_at": "2026-09-01T10:04:00Z", "runner_name": "GitHub Actions 3",
        "labels": ["ubuntu-latest"], "html_url": "https://github.com/octo/cat/actions/runs/77/job/5",
        "steps": [{"number": 1, "name": "Checkout", "status": "completed", "conclusion": "success"},
                  {"number": 2, "name": "Build", "status": "completed", "conclusion": "failure"}]})
}

#[tokio::test]
async fn workflows_are_listed_read_and_their_usage_shown() {
    let workflow = json!({"id": 161_335, "name": "CI", "path": ".github/workflows/ci.yml", "state": "active",
        "updated_at": "2026-09-01T10:00:00Z", "html_url": "https://github.com/octo/cat/blob/main/.github/workflows/ci.yml"});
    let items = check_read(Read {
        tool: "github_workflow_list",
        args: json!({"repo": "octo/cat"}),
        path: "/repos/octo/cat/actions/workflows",
        query: vec![("per_page", "20")],
        reply: json!({"total_count": 1, "workflows": [workflow]}),
        id: "161335",
        title: "CI",
        snippet: ".github/workflows/ci.yml · active",
        resource: "octo/cat",
    })
    .await;
    assert_eq!(items[0].extra["state"], "active");
    assert_eq!(items[0].parents, [("octo".to_owned(), "Every repository of octo".to_owned())]);
    assert!(items[0].date > 0);
    check_read(Read {
        tool: "github_workflow_get",
        args: json!({"repo": "octo/cat", "workflow": "ci.yml"}),
        path: "/repos/octo/cat/actions/workflows/ci.yml",
        query: vec![],
        reply: workflow,
        id: "161335",
        title: "CI",
        snippet: "active",
        resource: "octo/cat",
    })
    .await;
    let usage = check_read(Read {
        tool: "github_workflow_usage",
        args: json!({"repo": "octo/cat", "workflow": "161335"}),
        path: "/repos/octo/cat/actions/workflows/161335/timing",
        query: vec![],
        reply: json!({"billable": {"UBUNTU": {"total_ms": 120_000}}}),
        id: "usage-161335",
        title: "Usage of workflow 161335",
        snippet: "UBUNTU: 120000 ms",
        resource: "octo/cat",
    })
    .await;
    assert_eq!(usage[0].extra["billable"]["UBUNTU"]["total_ms"], 120_000);
}

#[tokio::test]
async fn runs_are_listed_with_their_filters_and_read_by_attempt() {
    let items = check_read(Read {
        tool: "github_run_list",
        args: json!({"repo": "octo/cat", "workflow": "ci.yml", "branch": "main", "event": "push", "status": "failure",
            "actor": "ann", "limit": 5}),
        path: "/repos/octo/cat/actions/workflows/ci.yml/runs",
        query: vec![("branch", "main"), ("event", "push"), ("status", "failure"), ("actor", "ann"), ("per_page", "5")],
        reply: json!({"total_count": 1, "workflow_runs": [run_json(77)]}),
        id: "77",
        title: "Fix the build",
        snippet: "#12 · CI · failure · main · push",
        resource: "octo/cat",
    })
    .await;
    assert_eq!((items[0].from.as_str(), items[0].extra["run_number"].as_i64()), ("ann", Some(12)));
    assert_eq!(items[0].extra["head_sha"], "abc");
    check_read(Read {
        tool: "github_run_list",
        args: json!({"repo": "octo/cat"}),
        path: "/repos/octo/cat/actions/runs",
        query: vec![("per_page", "20")],
        reply: json!({"workflow_runs": [run_json(78)]}),
        id: "78",
        title: "Fix the build",
        snippet: "failure",
        resource: "octo/cat",
    })
    .await;
    let got = check_read(Read {
        tool: "github_run_get",
        args: json!({"repo": "octo/cat", "run_id": 77}),
        path: "/repos/octo/cat/actions/runs/77",
        query: vec![],
        reply: run_json(77),
        id: "77",
        title: "Fix the build",
        snippet: "failure",
        resource: "octo/cat",
    })
    .await;
    assert_eq!(got[0].extra["commit_message"], "Fix longer");
    check_read(Read {
        tool: "github_run_get",
        args: json!({"repo": "octo/cat", "run_id": 77, "attempt": 2}),
        path: "/repos/octo/cat/actions/runs/77/attempts/2",
        query: vec![],
        reply: run_json(77),
        id: "77",
        title: "Fix the build",
        snippet: "failure",
        resource: "octo/cat",
    })
    .await;
}

#[tokio::test]
async fn every_attempt_of_a_run_is_listed_newest_first() {
    let server = MockServer::start().await;
    let (gh, _dir) = github(&server).await;
    let mut latest = run_json(77);
    latest["run_attempt"] = json!(3);
    json_at(&server, "/repos/octo/cat/actions/runs/77", &latest).await;
    for attempt in [1, 2] {
        let mut r = run_json(77);
        r["run_attempt"] = json!(attempt);
        json_at(&server, &format!("/repos/octo/cat/actions/runs/77/attempts/{attempt}"), &r).await;
    }
    let items = fetch(&gh, "github_run_attempts", &json!({"repo": "octo/cat", "run_id": 77})).await;
    assert_eq!(items.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), ["77-3", "77-2", "77-1"]);
    assert_eq!(items[2].extra["run_attempt"], 1);
}

#[tokio::test]
async fn jobs_show_their_steps_and_which_failed() {
    let items = check_read(Read {
        tool: "github_run_jobs",
        args: json!({"repo": "octo/cat", "run_id": 77, "filter": "all"}),
        path: "/repos/octo/cat/actions/runs/77/jobs",
        query: vec![("filter", "all"), ("per_page", "20")],
        reply: json!({"total_count": 1, "jobs": [job_json(5)]}),
        id: "5",
        title: "build",
        snippet: "failure · runner GitHub Actions 3 · failed step: Build",
        resource: "octo/cat",
    })
    .await;
    assert_eq!(items[0].extra["steps"][1]["conclusion"], "failure");
    check_read(Read {
        tool: "github_run_jobs",
        args: json!({"repo": "octo/cat", "run_id": 77}),
        path: "/repos/octo/cat/actions/runs/77/jobs",
        query: vec![("filter", "latest")],
        reply: json!({"jobs": [job_json(5)]}),
        id: "5",
        title: "build",
        snippet: "failure",
        resource: "octo/cat",
    })
    .await;
    check_read(Read {
        tool: "github_job_get",
        args: json!({"repo": "octo/cat", "job_id": 5}),
        path: "/repos/octo/cat/actions/jobs/5",
        query: vec![],
        reply: job_json(5),
        id: "5",
        title: "build",
        snippet: "failed step: Build",
        resource: "octo/cat",
    })
    .await;
}

#[tokio::test]
async fn artifacts_deployments_and_caches_are_listed() {
    let artifact = json!({"id": 9, "name": "dist", "size_in_bytes": 1_572_864, "expired": false, "created_at": "2026-09-01T10:00:00Z",
        "expires_at": "2026-12-01T10:00:00Z", "workflow_run": {"id": 77, "head_branch": "main"}});
    let items = check_read(Read {
        tool: "github_artifact_list",
        args: json!({"repo": "octo/cat", "run_id": 77, "name": "dist"}),
        path: "/repos/octo/cat/actions/runs/77/artifacts",
        query: vec![("name", "dist")],
        reply: json!({"artifacts": [artifact]}),
        id: "9",
        title: "dist",
        snippet: "1.5 MB · expires 2026-12-01T10:00:00Z · main",
        resource: "octo/cat",
    })
    .await;
    assert_eq!(items[0].extra["run_id"], 77);
    check_read(Read {
        tool: "github_artifact_list",
        args: json!({"repo": "octo/cat"}),
        path: "/repos/octo/cat/actions/artifacts",
        query: vec![("per_page", "20")],
        reply: json!({"artifacts": [{"id": 10, "name": "old", "size_in_bytes": 5, "expired": true}]}),
        id: "10",
        title: "old",
        snippet: "5 B · expired",
        resource: "octo/cat",
    })
    .await;
    let waiting = check_read(Read {
        tool: "github_run_pending_deployments",
        args: json!({"repo": "octo/cat", "run_id": 77}),
        path: "/repos/octo/cat/actions/runs/77/pending_deployments",
        query: vec![],
        reply: json!([{"environment": {"id": 161_088_068, "name": "staging"}, "wait_timer": 0, "current_user_can_approve": true,
            "reviewers": [{"type": "User", "reviewer": {"login": "ann"}}, {"type": "Team", "reviewer": {"slug": "ops"}}]}]),
        id: "161088068",
        title: "staging",
        snippet: "you can review it",
        resource: "octo/cat",
    })
    .await;
    assert_eq!(waiting[0].extra["reviewers"], json!(["ann", "ops"]));
    let caches = check_read(Read {
        tool: "github_cache_list",
        args: json!({"repo": "octo/cat", "key": "npm-", "ref": "refs/heads/main"}),
        path: "/repos/octo/cat/actions/caches",
        query: vec![("key", "npm-"), ("ref", "refs/heads/main"), ("per_page", "20")],
        reply: json!({"total_count": 1, "actions_caches": [{"id": 5, "key": "npm-abc", "ref": "refs/heads/main", "size_in_bytes": 2048,
            "last_accessed_at": "2026-09-01T10:00:00Z"}]}),
        id: "5",
        title: "npm-abc",
        snippet: "2.0 KB · refs/heads/main",
        resource: "octo/cat",
    })
    .await;
    assert_eq!(caches[0].extra["key"], "npm-abc");
}

#[tokio::test]
async fn variables_show_values_and_secrets_never_do() {
    let var = json!({"name": "DEPLOY_TARGET", "value": "eu-west", "updated_at": "2026-09-01T10:00:00Z"});
    let items = check_read(Read {
        tool: "github_variable_list",
        args: json!({"repo": "octo/cat"}),
        path: "/repos/octo/cat/actions/variables",
        query: vec![("per_page", "20")],
        reply: json!({"total_count": 1, "variables": [var]}),
        id: "DEPLOY_TARGET",
        title: "DEPLOY_TARGET",
        snippet: "eu-west",
        resource: "octo/cat",
    })
    .await;
    assert_eq!(items[0].body.as_deref(), Some("eu-west"));
    check_read(Read {
        tool: "github_variable_list",
        args: json!({"repo": "octo/cat", "environment": "prod env"}),
        path: "/repos/octo/cat/environments/prod%20env/variables",
        query: vec![],
        reply: json!({"variables": [var]}),
        id: "DEPLOY_TARGET",
        title: "DEPLOY_TARGET",
        snippet: "eu-west",
        resource: "octo/cat",
    })
    .await;
    let one = check_read(Read {
        tool: "github_variable_get",
        args: json!({"repo": "octo/cat", "name": "DEPLOY_TARGET"}),
        path: "/repos/octo/cat/actions/variables/DEPLOY_TARGET",
        query: vec![],
        reply: var,
        id: "DEPLOY_TARGET",
        title: "DEPLOY_TARGET",
        snippet: "eu-west",
        resource: "octo/cat",
    })
    .await;
    assert_eq!(one[0].body.as_deref(), Some("eu-west"));
    let secrets = check_read(Read {
        tool: "github_secret_list",
        args: json!({"repo": "octo/cat"}),
        path: "/repos/octo/cat/actions/secrets",
        query: vec![("per_page", "20")],
        reply: json!({"total_count": 1, "secrets": [{"name": "API_KEY", "updated_at": "2026-09-01T10:00:00Z", "value": "must-not-appear"}]}),
        id: "API_KEY",
        title: "API_KEY",
        snippet: "never shown",
        resource: "octo/cat",
    })
    .await;
    assert!(!serde_json::to_string(&secrets).unwrap().contains("must-not-appear"));
    check_read(Read {
        tool: "github_secret_list",
        args: json!({"repo": "octo/cat", "environment": "prod"}),
        path: "/repos/octo/cat/environments/prod/secrets",
        query: vec![],
        reply: json!({"secrets": [{"name": "TOKEN"}]}),
        id: "TOKEN",
        title: "TOKEN",
        snippet: "never shown",
        resource: "octo/cat",
    })
    .await;
}

#[tokio::test]
async fn environments_are_listed_and_read() {
    let env = json!({"name": "staging", "protection_rules": [{"type": "required_reviewers"}, {"type": "wait_timer"}],
        "deployment_branch_policy": {"protected_branches": true, "custom_branch_policies": false}, "can_admins_bypass": true});
    check_read(Read {
        tool: "github_environment_list",
        args: json!({"repo": "octo/cat"}),
        path: "/repos/octo/cat/environments",
        query: vec![("per_page", "20")],
        reply: json!({"total_count": 1, "environments": [env]}),
        id: "staging",
        title: "staging",
        snippet: "required_reviewers, wait_timer",
        resource: "octo/cat",
    })
    .await;
    let got = check_read(Read {
        tool: "github_environment_get",
        args: json!({"repo": "octo/cat", "environment": "staging"}),
        path: "/repos/octo/cat/environments/staging",
        query: vec![],
        reply: env,
        id: "staging",
        title: "staging",
        snippet: "required_reviewers",
        resource: "octo/cat",
    })
    .await;
    assert_eq!(got[0].extra["deployment_branch_policy"]["protected_branches"], true);
}

#[tokio::test]
async fn security_alerts_are_listed_and_read() {
    let dependabot = json!({"number": 3, "state": "open", "html_url": "https://github.com/octo/cat/security/dependabot/3",
        "created_at": "2026-09-01T10:00:00Z", "dependency": {"package": {"name": "lodash", "ecosystem": "npm"}, "manifest_path": "package.json"},
        "security_advisory": {"ghsa_id": "GHSA-xxxx", "cve_id": "CVE-2026-1", "severity": "high", "summary": "Prototype pollution",
            "description": "Long advisory text"},
        "security_vulnerability": {"vulnerable_version_range": "< 4.17.21", "first_patched_version": {"identifier": "4.17.21"}}});
    let list = check_read(Read {
        tool: "github_dependabot_alert_list",
        args: json!({"repo": "octo/cat", "severity": "high", "package": "lodash"}),
        path: "/repos/octo/cat/dependabot/alerts",
        query: vec![("state", "open"), ("severity", "high"), ("package", "lodash"), ("per_page", "20")],
        reply: json!([dependabot]),
        id: "3",
        title: "Prototype pollution",
        snippet: "#3 · high · open · lodash (npm)",
        resource: "octo/cat",
    })
    .await;
    assert_eq!(list[0].extra["first_patched_version"], "4.17.21");
    let one = check_read(Read {
        tool: "github_dependabot_alert_get",
        args: json!({"repo": "octo/cat", "number": 3}),
        path: "/repos/octo/cat/dependabot/alerts/3",
        query: vec![],
        reply: dependabot,
        id: "3",
        title: "Prototype pollution",
        snippet: "lodash",
        resource: "octo/cat",
    })
    .await;
    assert_eq!(one[0].body.as_deref(), Some("Long advisory text"));

    let code = json!({"number": 4, "state": "open", "html_url": "u", "created_at": "2026-09-01T10:00:00Z",
        "rule": {"id": "js/sql-injection", "description": "SQL injection", "severity": "error", "security_severity_level": "high", "help": "Use parameters"},
        "tool": {"name": "CodeQL"},
        "most_recent_instance": {"ref": "refs/heads/main", "message": {"text": "This query depends on user input"}, "location": {"path": "src/db.js", "start_line": 42, "end_line": 43}}});
    check_read(Read {
        tool: "github_code_scanning_alert_list",
        args: json!({"repo": "octo/cat", "state": "dismissed", "ref": "refs/heads/main", "severity": "high"}),
        path: "/repos/octo/cat/code-scanning/alerts",
        query: vec![("state", "dismissed"), ("ref", "refs/heads/main"), ("severity", "high")],
        reply: json!([code]),
        id: "4",
        title: "SQL injection",
        snippet: "#4 · high · open · src/db.js:42",
        resource: "octo/cat",
    })
    .await;
    let one = check_read(Read {
        tool: "github_code_scanning_alert_get",
        args: json!({"repo": "octo/cat", "number": 4}),
        path: "/repos/octo/cat/code-scanning/alerts/4",
        query: vec![],
        reply: code,
        id: "4",
        title: "SQL injection",
        snippet: "src/db.js",
        resource: "octo/cat",
    })
    .await;
    assert!(one[0].body.as_deref().unwrap().contains("This query depends on user input"));

    let advisory = json!({"ghsa_id": "GHSA-yyyy", "cve_id": null, "severity": "critical", "state": "published", "summary": "Bad thing",
        "description": "details", "html_url": "u", "published_at": "2026-09-01T10:00:00Z"});
    check_read(Read {
        tool: "github_security_advisory_list",
        args: json!({"repo": "octo/cat", "state": "published"}),
        path: "/repos/octo/cat/security-advisories",
        query: vec![("state", "published")],
        reply: json!([advisory]),
        id: "GHSA-yyyy",
        title: "Bad thing",
        snippet: "GHSA-yyyy · critical · published",
        resource: "octo/cat",
    })
    .await;
}

#[tokio::test]
async fn leaked_secrets_are_sensitive_secret_and_kept_out_of_the_text() {
    let server = MockServer::start().await;
    let (gh, _dir) = github(&server).await;
    let alert = json!({"number": 5, "state": "open", "secret_type": "github_personal_access_token",
        "secret_type_display_name": "GitHub Personal Access Token", "secret": "ghp_LEAKEDLEAKEDLEAKED", "validity": "active",
        "html_url": "u", "created_at": "2026-09-01T10:00:00Z"});
    Mock::given(method("GET"))
        .and(path("/repos/octo/cat/secret-scanning/alerts"))
        .and(query_param("state", "open"))
        .and(query_param("secret_type", "github_personal_access_token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([alert])))
        .expect(1)
        .mount(&server)
        .await;
    json_at(&server, "/repos/octo/cat/secret-scanning/alerts/5", &alert).await;
    for items in [
        fetch(
            &gh,
            "github_secret_scanning_alert_list",
            &json!({"repo": "octo/cat", "secret_type": "github_personal_access_token"}),
        )
        .await,
        fetch(&gh, "github_secret_scanning_alert_get", &json!({"repo": "octo/cat", "number": 5})).await,
    ] {
        let item = &items[0];
        assert!(item.sensitive && item.secret);
        assert_eq!(item.body.as_deref(), Some("ghp_LEAKEDLEAKEDLEAKED"));
        assert!(item.snippet.contains("GitHub Personal Access Token") && !item.snippet.contains("ghp_"));
        assert!(!item.title.contains("ghp_"));
        assert!(!item.text().contains("ghp_LEAKED"), "the approval and the log show only the snippet");
        assert!(!serde_json::to_string(&item.extra).unwrap().contains("ghp_LEAKED"));
        assert_eq!(item.resource, "octo/cat");
    }
}

#[tokio::test]
async fn the_dependency_list_is_cut_at_the_limit() {
    let server = MockServer::start().await;
    let (gh, _dir) = github(&server).await;
    let packages: Vec<Value> =
        (0..5_000).map(|i| json!({"name": format!("npm:package-number-{i}"), "versionInfo": "1.0.0"})).collect();
    json_at(&server, "/repos/octo/cat/dependency-graph/sbom", &json!({"sbom": {"packages": packages}})).await;
    let items = fetch(&gh, "github_dependency_sbom", &json!({"repo": "octo/cat"})).await;
    let body = items[0].body.as_deref().unwrap();
    assert!(body.chars().count() <= 60_000 && body.starts_with("npm:package-number-0 1.0.0\n"));
    assert_eq!(
        (items[0].extra["truncated"].clone(), items[0].extra["package_count"].clone()),
        (json!(true), json!(5_000))
    );
    assert_eq!(items[0].snippet, "5000 packages");
    let short = MockServer::start().await;
    let (gh2, _d) = github(&short).await;
    json_at(
        &short,
        "/repos/octo/cat/dependency-graph/sbom",
        &json!({"sbom": {"packages": [{"name": "npm:a", "versionInfo": "2"}]}}),
    )
    .await;
    let items = fetch(&gh2, "github_dependency_sbom", &json!({"repo": "octo/cat"})).await;
    assert_eq!((items[0].body.as_deref(), &items[0].extra["truncated"]), (Some("npm:a 2\n"), &json!(false)));
}

// ---- The account ---------------------------------------------------------------------------------------------------

#[tokio::test]
async fn the_account_reads_are_about_the_account_resource() {
    let profile = json!({"login": "octo-cat", "id": 1, "name": "Octo Cat", "type": "User", "public_repos": 8, "followers": 3,
        "following": 2, "plan": {"name": "pro"}, "html_url": "u", "created_at": "2020-01-01T00:00:00Z", "bio": "Hi there"});
    let me = check_read(Read {
        tool: "github_user_me",
        args: json!({}),
        path: "/user",
        query: vec![],
        reply: profile.clone(),
        id: "octo-cat",
        title: "octo-cat",
        snippet: "Octo Cat",
        resource: "account",
    })
    .await;
    assert_eq!((me[0].resource_label.as_str(), me[0].extra["plan"].clone()), ("Your GitHub account", json!("pro")));
    assert!(me[0].parents.is_empty());
    check_read(Read {
        tool: "github_user_get",
        args: json!({"username": "ann"}),
        path: "/users/ann",
        query: vec![],
        reply: profile.clone(),
        id: "octo-cat",
        title: "octo-cat",
        snippet: "Octo Cat",
        resource: "account",
    })
    .await;
    check_read(Read {
        tool: "github_org_get",
        args: json!({"org": "acme"}),
        path: "/orgs/acme",
        query: vec![],
        reply: json!({"login": "acme", "type": "Organization", "description": "Anvils"}),
        id: "acme",
        title: "acme",
        snippet: "Anvils",
        resource: "account",
    })
    .await;
    check_read(Read {
        tool: "github_org_list",
        args: json!({}),
        path: "/user/orgs",
        query: vec![("per_page", "20")],
        reply: json!([{"login": "acme", "description": "Anvils"}]),
        id: "acme",
        title: "acme",
        snippet: "Anvils",
        resource: "account",
    })
    .await;
    let repos = check_read(Read {
        tool: "github_org_repos",
        args: json!({"org": "acme"}),
        path: "/orgs/acme/repos",
        query: vec![("per_page", "20")],
        reply: json!([{"full_name": "acme/anvil", "description": "Heavy", "private": true, "default_branch": "main", "stargazers_count": 9}]),
        id: "acme/anvil",
        title: "acme/anvil",
        snippet: "Heavy",
        resource: "account",
    })
    .await;
    assert_eq!((repos[0].extra["private"].clone(), repos[0].extra["stars"].clone()), (json!(true), json!(9)));
    check_read(Read {
        tool: "github_org_members",
        args: json!({"org": "acme", "limit": 3}),
        path: "/orgs/acme/members",
        query: vec![("per_page", "3")],
        reply: json!([{"login": "ann", "type": "User"}]),
        id: "ann",
        title: "ann",
        snippet: "",
        resource: "account",
    })
    .await;
    check_read(Read {
        tool: "github_org_teams",
        args: json!({"org": "acme"}),
        path: "/orgs/acme/teams",
        query: vec![],
        reply: json!([{"slug": "ops", "name": "Operations", "privacy": "closed", "description": "Run things"}]),
        id: "ops",
        title: "Operations",
        snippet: "closed · Run things",
        resource: "account",
    })
    .await;
    for (args, p) in [(json!({}), "/user/starred"), (json!({"username": "ann"}), "/users/ann/starred")] {
        check_read(Read {
            tool: "github_starred_list",
            args,
            path: p,
            query: vec![("per_page", "20")],
            reply: json!([{"full_name": "octo/cat", "description": "A cat"}]),
            id: "octo/cat",
            title: "octo/cat",
            snippet: "A cat",
            resource: "account",
        })
        .await;
    }
}

#[tokio::test]
async fn ssh_and_gpg_keys_are_listed_by_name_only() {
    let ssh = check_read(Read {
        tool: "github_ssh_key_list",
        args: json!({}),
        path: "/user/keys",
        query: vec![("per_page", "20")],
        reply: json!([{"id": 7, "title": "laptop", "key": "ssh-ed25519 AAAAKEYMATERIAL", "created_at": "2026-01-01T00:00:00Z"}]),
        id: "7",
        title: "laptop",
        snippet: "SSH key 7",
        resource: "account",
    })
    .await;
    assert!(!serde_json::to_string(&ssh).unwrap().contains("KEYMATERIAL"));
    let gpg = check_read(Read {
        tool: "github_gpg_key_list",
        args: json!({}),
        path: "/user/gpg_keys",
        query: vec![],
        reply: json!([{"id": 8, "key_id": "3262EFF25BA0D270", "public_key": "-----BEGIN PGP PUBLIC KEY BLOCK-----KEYMATERIAL",
            "emails": [{"email": "octo@example.com", "verified": true}], "created_at": "2026-01-01T00:00:00Z"}]),
        id: "8",
        title: "3262EFF25BA0D270",
        snippet: "octo@example.com",
        resource: "account",
    })
    .await;
    assert!(!serde_json::to_string(&gpg).unwrap().contains("KEYMATERIAL"));
}

#[tokio::test]
async fn rate_limits_notifications_and_gists_are_read() {
    let limits = check_read(Read {
        tool: "github_rate_limit",
        args: json!({}),
        path: "/rate_limit",
        query: vec![],
        reply: json!({"resources": {"core": {"limit": 5000, "remaining": 4990, "reset": 1_790_000_000},
            "search": {"limit": 30, "remaining": 29, "reset": 1_790_000_000}}}),
        id: "rate_limit",
        title: "rate limits",
        snippet: "core 4990/5000",
        resource: "account",
    })
    .await;
    assert_eq!(limits[0].extra["search"]["remaining"], 29);
    assert!(limits[0].extra["core"]["resets_at"].as_str().unwrap().starts_with("2026-"));

    let note = json!({"id": "123", "reason": "mention", "unread": true, "updated_at": "2026-09-01T10:00:00Z",
        "subject": {"title": "Crash on start", "type": "Issue", "url": "https://api.github.com/repos/octo/cat/issues/7"},
        "repository": {"full_name": "octo/cat"}});
    let notes = check_read(Read {
        tool: "github_notification_list",
        args: json!({"all": true, "participating": true}),
        path: "/notifications",
        query: vec![("all", "true"), ("participating", "true"), ("per_page", "20")],
        reply: json!([note]),
        id: "123",
        title: "Crash on start",
        snippet: "mention · Issue · octo/cat · unread",
        resource: "account",
    })
    .await;
    assert_eq!(notes[0].extra["repository"], "octo/cat");
    check_read(Read {
        tool: "github_notification_list",
        args: json!({"repo": "octo/cat"}),
        path: "/repos/octo/cat/notifications",
        query: vec![],
        reply: json!([note]),
        id: "123",
        title: "Crash on start",
        snippet: "mention",
        resource: "account",
    })
    .await;

    let gist = json!({"id": "abc123", "description": "notes", "public": false, "updated_at": "2026-09-01T10:00:00Z",
        "html_url": "u", "files": {"a.txt": {"content": "hello"}, "b.md": {"content": "x".repeat(25_000), "truncated": false}}});
    let listed = check_read(Read {
        tool: "github_gist_list",
        args: json!({}),
        path: "/gists",
        query: vec![("per_page", "20")],
        reply: json!([gist]),
        id: "abc123",
        title: "notes",
        snippet: "secret · a.txt, b.md",
        resource: "account",
    })
    .await;
    assert_eq!(listed[0].extra["files"], json!(["a.txt", "b.md"]));
    check_read(Read {
        tool: "github_gist_list",
        args: json!({"username": "ann"}),
        path: "/users/ann/gists",
        query: vec![],
        reply: json!([gist]),
        id: "abc123",
        title: "notes",
        snippet: "a.txt",
        resource: "account",
    })
    .await;
    let got = check_read(Read {
        tool: "github_gist_get",
        args: json!({"gist_id": "abc123"}),
        path: "/gists/abc123",
        query: vec![],
        reply: gist,
        id: "abc123",
        title: "notes",
        snippet: "secret",
        resource: "account",
    })
    .await;
    let body = got[0].body.as_deref().unwrap();
    assert!(body.contains("--- a.txt ---\nhello") && body.contains("--- b.md ---"));
    assert_eq!(got[0].extra["truncated"], true, "a file over 20000 characters is cut");
    assert!(body.chars().count() < 21_000);
}

// ---- Writes ----------------------------------------------------------------------------------------------------------

/// One write: what the preview says, what request the change sends, what the AI is told.
struct Write {
    tool: &'static str,
    args: Value,
    /// Looked up (GET) to describe the change.
    lookups: Vec<(&'static str, Value)>,
    verb: &'static str,
    path: &'static str,
    query: Vec<(&'static str, &'static str)>,
    /// The exact JSON body, or `None` when it is not checked.
    body: Option<Value>,
    status: u16,
    reply: Value,
    first_line: &'static str,
    once: bool,
    resource: &'static str,
    result: Value,
}

impl Write {
    fn new(tool: &'static str, args: Value, verb: &'static str, path: &'static str, first_line: &'static str) -> Self {
        Self {
            tool,
            args,
            lookups: Vec::new(),
            verb,
            path,
            query: Vec::new(),
            body: None,
            status: 204,
            reply: Value::Null,
            first_line,
            once: false,
            resource: "octo/cat",
            result: json!({}),
        }
    }

    fn account(mut self) -> Self {
        self.resource = "account";
        self
    }

    fn once(mut self) -> Self {
        self.once = true;
        self
    }

    fn body(mut self, body: Value) -> Self {
        self.body = Some(body);
        self
    }

    fn lookup(mut self, p: &'static str, v: Value) -> Self {
        self.lookups.push((p, v));
        self
    }

    fn status(mut self, status: u16, reply: Value) -> Self {
        self.status = status;
        self.reply = reply;
        self
    }

    fn result(mut self, result: Value) -> Self {
        self.result = result;
        self
    }

    fn query(mut self, q: &[(&'static str, &'static str)]) -> Self {
        self.query = q.to_vec();
        self
    }
}

/// Every key of `expected` is in `actual` with that value.
fn holds(actual: &Value, expected: &Value) {
    for (k, v) in expected.as_object().unwrap() {
        assert_eq!(&actual[k], v, "{k} in {actual}");
    }
}

async fn check_write(w: Write) -> Value {
    let server = MockServer::start().await;
    let (gh, _dir) = github(&server).await;
    for (p, v) in &w.lookups {
        json_at(&server, p, v).await;
    }
    let mut mock = Mock::given(method(w.verb)).and(path(w.path));
    for (k, v) in &w.query {
        mock = mock.and(query_param(*k, *v));
    }
    if let Some(b) = &w.body {
        mock = mock.and(body_json(b));
    }
    let response = if w.reply.is_null() {
        ResponseTemplate::new(w.status)
    } else {
        ResponseTemplate::new(w.status).set_body_json(&w.reply)
    };
    // Exactly one call: the preview changes nothing, the change happens once.
    mock.respond_with(response).expect(1).mount(&server).await;
    let c = call(w.tool, &w.args);
    let preview = gh.preview(ME, &c).await.unwrap_or_else(|e| panic!("{} preview: {e}", w.tool));
    assert!(preview.lines[0].contains(w.first_line), "{}: {:?}", w.tool, preview.lines);
    assert_eq!(preview.once_only, w.once, "{} once-only", w.tool);
    assert_eq!(preview.resource, w.resource, "{}", w.tool);
    assert!(!preview.resource_label.is_empty());
    if w.resource == "account" {
        assert_eq!((preview.resource_label.as_str(), preview.parents.is_empty()), ("Your GitHub account", true));
    } else {
        assert_eq!(preview.parents, [("octo".to_owned(), "Every repository of octo".to_owned())]);
    }
    let spec = spec_for_tool(w.tool).unwrap();
    assert_eq!(spec.once_only, w.once, "{} tool flag agrees with the preview", w.tool);
    let done = gh.perform(ME, &c).await.unwrap_or_else(|e| panic!("{} perform: {e}", w.tool));
    holds(&done, &w.result);
    done
}

#[tokio::test]
async fn workflows_are_dispatched_enabled_and_disabled() {
    let ci = json!({"id": 161_335, "name": "CI", "state": "active"});
    let done = check_write(
        Write::new(
            "github_workflow_dispatch",
            json!({"repo": "octo/cat", "workflow": "ci.yml", "ref": "main", "inputs": {"level": "debug", "n": "3"}}),
            "POST",
            "/repos/octo/cat/actions/workflows/ci.yml/dispatches",
            "Run workflow CI in octo/cat on main",
        )
        .lookup("/repos/octo/cat/actions/workflows/ci.yml", ci.clone())
        .body(json!({"ref": "main", "inputs": {"level": "debug", "n": "3"}}))
        .result(json!({"dispatched": true, "workflow": "ci.yml", "ref": "main"})),
    )
    .await;
    assert_eq!(done.as_object().unwrap().len(), 3);
    check_write(
        Write::new(
            "github_workflow_dispatch",
            json!({"repo": "octo/cat", "workflow": "161335", "ref": "v1.0"}),
            "POST",
            "/repos/octo/cat/actions/workflows/161335/dispatches",
            "Run workflow 161335 in octo/cat on v1.0",
        )
        .body(json!({"ref": "v1.0"}))
        .lookup("/repos/octo/cat/actions/workflows/161335", json!({})),
    )
    .await;
    check_write(
        Write::new(
            "github_workflow_enable",
            json!({"repo": "octo/cat", "workflow": "ci.yml"}),
            "PUT",
            "/repos/octo/cat/actions/workflows/ci.yml/enable",
            "Enable workflow CI in octo/cat",
        )
        .lookup("/repos/octo/cat/actions/workflows/ci.yml", ci.clone())
        .result(json!({"workflow": "ci.yml", "state": "active"})),
    )
    .await;
    check_write(
        Write::new(
            "github_workflow_disable",
            json!({"repo": "octo/cat", "workflow": "ci.yml"}),
            "PUT",
            "/repos/octo/cat/actions/workflows/ci.yml/disable",
            "Disable workflow CI in octo/cat",
        )
        .lookup("/repos/octo/cat/actions/workflows/ci.yml", ci)
        .result(json!({"workflow": "ci.yml", "state": "disabled_manually"})),
    )
    .await;
}

#[tokio::test]
async fn runs_are_rerun_cancelled_deleted_and_approved() {
    let run = run_json(77);
    let p = "/repos/octo/cat/actions/runs/77";
    let line = "run #12 'Fix the build' (main, failure) in octo/cat";
    let done = check_write(
        Write::new(
            "github_run_rerun",
            json!({"repo": "octo/cat", "run_id": 77, "enable_debug_logging": true}),
            "POST",
            "/repos/octo/cat/actions/runs/77/rerun",
            "Re-run run #12",
        )
        .lookup(p, run.clone())
        .body(json!({"enable_debug_logging": true}))
        .status(201, json!({}))
        .result(json!({"rerun": true, "run_id": 77})),
    )
    .await;
    assert_eq!(done["run_id"], 77);
    check_write(
        Write::new(
            "github_run_rerun",
            json!({"repo": "octo/cat", "run_id": 77}),
            "POST",
            "/repos/octo/cat/actions/runs/77/rerun",
            line,
        )
        .lookup(p, run.clone())
        .status(201, json!({})),
    )
    .await;
    check_write(
        Write::new(
            "github_run_rerun_failed",
            json!({"repo": "octo/cat", "run_id": 77}),
            "POST",
            "/repos/octo/cat/actions/runs/77/rerun-failed-jobs",
            "Re-run the failed jobs of run #12",
        )
        .lookup(p, run.clone())
        .status(201, json!({}))
        .result(json!({"rerun_failed": true})),
    )
    .await;
    check_write(
        Write::new(
            "github_run_cancel",
            json!({"repo": "octo/cat", "run_id": 77}),
            "POST",
            "/repos/octo/cat/actions/runs/77/cancel",
            "Cancel run #12",
        )
        .lookup(p, run.clone())
        .status(202, json!({}))
        .result(json!({"cancel_requested": true})),
    )
    .await;
    check_write(
        Write::new(
            "github_run_force_cancel",
            json!({"repo": "octo/cat", "run_id": 77}),
            "POST",
            "/repos/octo/cat/actions/runs/77/force-cancel",
            "Force-cancel run #12",
        )
        .lookup(p, run.clone())
        .once()
        .status(202, json!({}))
        .result(json!({"force_cancel_requested": true})),
    )
    .await;
    check_write(
        Write::new(
            "github_run_delete",
            json!({"repo": "octo/cat", "run_id": 77}),
            "DELETE",
            "/repos/octo/cat/actions/runs/77",
            "Delete run #12",
        )
        .lookup(p, run.clone())
        .result(json!({"deleted": true})),
    )
    .await;
    check_write(
        Write::new(
            "github_run_approve",
            json!({"repo": "octo/cat", "run_id": 77}),
            "POST",
            "/repos/octo/cat/actions/runs/77/approve",
            "Approve (and so let it run the code of a fork) run #12",
        )
        .lookup(p, run)
        .once()
        .status(201, json!({}))
        .result(json!({"approved": true})),
    )
    .await;
    check_write(
        Write::new(
            "github_job_rerun",
            json!({"repo": "octo/cat", "job_id": 5, "enable_debug_logging": true}),
            "POST",
            "/repos/octo/cat/actions/jobs/5/rerun",
            "Re-run job 5 'build' (failure) in octo/cat",
        )
        .lookup("/repos/octo/cat/actions/jobs/5", job_json(5))
        .body(json!({"enable_debug_logging": true}))
        .status(201, json!({}))
        .result(json!({"rerun": true, "job_id": 5})),
    )
    .await;
    check_write(
        Write::new(
            "github_artifact_delete",
            json!({"repo": "octo/cat", "artifact_id": 9}),
            "DELETE",
            "/repos/octo/cat/actions/artifacts/9",
            "Delete artifact 9 'dist' (1.0 KB)",
        )
        .lookup("/repos/octo/cat/actions/artifacts/9", json!({"name": "dist", "size_in_bytes": 1024}))
        .result(json!({"deleted": true, "artifact_id": 9})),
    )
    .await;
}

#[tokio::test]
async fn a_deployment_is_reviewed_for_the_named_environments_only_when_they_wait() {
    let waiting = json!([
        {"environment": {"id": 11, "name": "staging"}, "current_user_can_approve": true},
        {"environment": {"id": 12, "name": "prod"}, "current_user_can_approve": true}
    ]);
    check_write(
        Write::new(
            "github_deployment_review",
            json!({"repo": "octo/cat", "run_id": 77, "environments": ["Staging"], "state": "approved", "comment": "looks good"}),
            "POST",
            "/repos/octo/cat/actions/runs/77/pending_deployments",
            "Approve the deployment of run 77 in octo/cat to staging",
        )
        .lookup("/repos/octo/cat/actions/runs/77/pending_deployments", waiting.clone())
        .body(json!({"environment_ids": [11], "state": "approved", "comment": "looks good"}))
        .once()
        .status(200, json!([]))
        .result(json!({"reviewed": true, "state": "approved", "environments": ["staging"]})),
    )
    .await;
    let server = MockServer::start().await;
    let (gh, _dir) = github(&server).await;
    json_at(&server, "/repos/octo/cat/actions/runs/77/pending_deployments", &waiting).await;
    let unknown = call(
        "github_deployment_review",
        &json!({"repo": "octo/cat", "run_id": 77, "environments": ["qa"], "state": "rejected", "comment": "no"}),
    );
    let err = gh.preview(ME, &unknown).await.unwrap_err();
    assert!(text(&err).contains("not waiting for 'qa'") && text(&err).contains("staging, prod"), "{err}");
    let cannot = json!([{"environment": {"id": 11, "name": "staging"}, "current_user_can_approve": false}]);
    let server = MockServer::start().await;
    let (gh, _dir) = github(&server).await;
    json_at(&server, "/repos/octo/cat/actions/runs/77/pending_deployments", &cannot).await;
    let c = call(
        "github_deployment_review",
        &json!({"repo": "octo/cat", "run_id": 77, "environments": ["staging"], "state": "approved", "comment": "x"}),
    );
    assert!(text(&gh.perform(ME, &c).await.unwrap_err()).contains("may not review"));
    assert!(requests(&server).await.iter().all(|r| r.method.as_str() == "GET"), "nothing was sent");
}

#[tokio::test]
async fn variables_are_created_changed_and_deleted_in_a_repository_or_an_environment() {
    check_write(
        Write::new(
            "github_variable_create",
            json!({"repo": "octo/cat", "name": "REGION", "value": "eu"}),
            "POST",
            "/repos/octo/cat/actions/variables",
            "Create Actions variable REGION in octo/cat",
        )
        .body(json!({"name": "REGION", "value": "eu"}))
        .status(201, json!({}))
        .result(json!({"created": true, "name": "REGION"})),
    )
    .await;
    let done = check_write(
        Write::new(
            "github_variable_create",
            json!({"repo": "octo/cat", "environment": "prod", "name": "REGION", "value": "us"}),
            "POST",
            "/repos/octo/cat/environments/prod/variables",
            "in environment 'prod' of octo/cat",
        )
        .body(json!({"name": "REGION", "value": "us"}))
        .status(201, json!({})),
    )
    .await;
    assert_eq!(done["environment"], "prod");
    let server = MockServer::start().await;
    let (gh, _dir) = github(&server).await;
    json_at(&server, "/repos/octo/cat/actions/variables/REGION", &json!({"name": "REGION", "value": "eu-old"})).await;
    let update = call("github_variable_update", &json!({"repo": "octo/cat", "name": "REGION", "value": "eu-new"}));
    let preview = gh.preview(ME, &update).await.unwrap();
    assert_eq!(preview.lines[1..], ["Old value: eu-old", "New value: eu-new"]);
    check_write(
        Write::new(
            "github_variable_update",
            json!({"repo": "octo/cat", "name": "REGION", "value": "eu-new"}),
            "PATCH",
            "/repos/octo/cat/actions/variables/REGION",
            "Change Actions variable REGION",
        )
        .lookup("/repos/octo/cat/actions/variables/REGION", json!({"value": "eu-old"}))
        .body(json!({"name": "REGION", "value": "eu-new"}))
        .result(json!({"updated": true})),
    )
    .await;
    check_write(
        Write::new(
            "github_variable_delete",
            json!({"repo": "octo/cat", "name": "REGION"}),
            "DELETE",
            "/repos/octo/cat/actions/variables/REGION",
            "Delete Actions variable REGION from octo/cat",
        )
        .lookup("/repos/octo/cat/actions/variables/REGION", json!({"value": "eu"}))
        .result(json!({"deleted": true})),
    )
    .await;
}

#[tokio::test]
async fn environments_are_created_changed_and_deleted_every_time_asking() {
    let server = MockServer::start().await;
    let (gh, _dir) = github(&server).await;
    status_at(&server, "GET", "/repos/octo/cat/environments/staging", 404, &json!({"message": "Not Found"})).await;
    json_at(&server, "/users/ann", &json!({"id": 42, "login": "ann"})).await;
    let expected = json!({"wait_timer": 10, "prevent_self_review": true,
        "deployment_branch_policy": {"protected_branches": true, "custom_branch_policies": false},
        "reviewers": [{"type": "User", "id": 42}]});
    Mock::given(method("PUT"))
        .and(path("/repos/octo/cat/environments/staging"))
        .and(body_json(&expected))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"name": "staging"})))
        .expect(1)
        .mount(&server)
        .await;
    let c = call(
        "github_environment_set",
        &json!({"repo": "octo/cat", "environment": "staging", "wait_timer": 10, "prevent_self_review": true,
            "branch_policy": "protected", "reviewers": ["ann"]}),
    );
    let preview = gh.preview(ME, &c).await.unwrap();
    assert!(
        preview.once_only && preview.lines[0].contains("Create environment 'staging' in octo/cat"),
        "{:?}",
        preview.lines
    );
    assert!(preview.lines.iter().any(|l| l.contains("Wait timer: 10 minutes")));
    assert!(preview.lines.iter().any(|l| l.contains("Required reviewers (replacing the current ones): ann")));
    holds(&gh.perform(ME, &c).await.unwrap(), &json!({"saved": true, "environment": "staging"}));

    let existing = MockServer::start().await;
    let (gh, _dir) = github(&existing).await;
    json_at(
        &existing,
        "/repos/octo/cat/environments/staging",
        &json!({"name": "staging", "protection_rules": [{"type": "wait_timer"}]}),
    )
    .await;
    let c =
        call("github_environment_set", &json!({"repo": "octo/cat", "environment": "staging", "branch_policy": "any"}));
    let preview = gh.preview(ME, &c).await.unwrap();
    assert!(
        preview.lines[0].starts_with("Change environment 'staging'")
            && preview.lines.iter().any(|l| l.contains("1 protection rule"))
    );
    Mock::given(method("PUT"))
        .and(path("/repos/octo/cat/environments/staging"))
        .and(body_json(json!({"deployment_branch_policy": null})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(1)
        .mount(&existing)
        .await;
    gh.perform(ME, &c).await.unwrap();
    let bad =
        call("github_environment_set", &json!({"repo": "octo/cat", "environment": "staging", "reviewers": ["a/b"]}));
    assert!(text(&gh.preview(ME, &bad).await.unwrap_err()).contains("not a valid GitHub login"));

    check_write(
        Write::new(
            "github_environment_delete",
            json!({"repo": "octo/cat", "environment": "staging"}),
            "DELETE",
            "/repos/octo/cat/environments/staging",
            "Delete environment 'staging' of octo/cat",
        )
        .once()
        .result(json!({"deleted": true, "environment": "staging"})),
    )
    .await;
}

#[tokio::test]
async fn caches_are_deleted_by_id_or_by_key() {
    check_write(
        Write::new(
            "github_cache_delete",
            json!({"repo": "octo/cat", "cache_id": 5}),
            "DELETE",
            "/repos/octo/cat/actions/caches/5",
            "Delete Actions cache 5 of octo/cat",
        )
        .result(json!({"deleted": true, "cache_id": 5})),
    )
    .await;
    let done = check_write(
        Write::new(
            "github_cache_delete",
            json!({"repo": "octo/cat", "key": "npm-abc", "ref": "refs/heads/main"}),
            "DELETE",
            "/repos/octo/cat/actions/caches",
            "Delete every Actions cache with key 'npm-abc' on refs/heads/main in octo/cat",
        )
        .query(&[("key", "npm-abc"), ("ref", "refs/heads/main")])
        .lookup("/repos/octo/cat/actions/caches", json!({"actions_caches": [{"id": 1}, {"id": 2}]}))
        .status(200, json!({"total_count": 2, "actions_caches": []}))
        .result(json!({"deleted": true, "key": "npm-abc", "total_count": 2})),
    )
    .await;
    assert!(done.get("actions_caches").is_none());
}

#[tokio::test]
async fn alerts_are_dismissed_and_reopened_with_a_reason() {
    check_write(
        Write::new(
            "github_dependabot_alert_update",
            json!({"repo": "octo/cat", "number": 3, "state": "dismissed", "reason": "not_used", "comment": "tests only"}),
            "PATCH",
            "/repos/octo/cat/dependabot/alerts/3",
            "Mark alert #3 of octo/cat (Prototype pollution) as dismissed",
        )
        .lookup("/repos/octo/cat/dependabot/alerts/3", json!({"state": "open", "security_advisory": {"summary": "Prototype pollution"}}))
        .body(json!({"state": "dismissed", "dismissed_reason": "not_used", "dismissed_comment": "tests only"}))
        .status(200, json!({}))
        .result(json!({"updated": true, "number": 3, "state": "dismissed"})),
    )
    .await;
    check_write(
        Write::new(
            "github_code_scanning_alert_update",
            json!({"repo": "octo/cat", "number": 4, "state": "dismissed", "reason": "false positive"}),
            "PATCH",
            "/repos/octo/cat/code-scanning/alerts/4",
            "Mark alert #4 of octo/cat (SQL injection) as dismissed",
        )
        .lookup(
            "/repos/octo/cat/code-scanning/alerts/4",
            json!({"state": "open", "rule": {"description": "SQL injection"}}),
        )
        .body(json!({"state": "dismissed", "dismissed_reason": "false positive"}))
        .status(200, json!({})),
    )
    .await;
    check_write(
        Write::new(
            "github_code_scanning_alert_update",
            json!({"repo": "octo/cat", "number": 4, "state": "open"}),
            "PATCH",
            "/repos/octo/cat/code-scanning/alerts/4",
            "as open",
        )
        .lookup("/repos/octo/cat/code-scanning/alerts/4", json!({"state": "dismissed"}))
        .body(json!({"state": "open"}))
        .status(200, json!({})),
    )
    .await;
    let server = MockServer::start().await;
    let (gh, _dir) = github(&server).await;
    json_at(
        &server,
        "/repos/octo/cat/secret-scanning/alerts/5",
        &json!({"state": "open", "secret": "ghp_LEAKEDLEAKED", "secret_type_display_name": "GitHub Token"}),
    )
    .await;
    Mock::given(method("PATCH"))
        .and(path("/repos/octo/cat/secret-scanning/alerts/5"))
        .and(body_json(json!({"state": "resolved", "resolution": "revoked", "resolution_comment": "rotated"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(1)
        .mount(&server)
        .await;
    let c = call(
        "github_secret_scanning_alert_update",
        &json!({"repo": "octo/cat", "number": 5, "state": "resolved", "resolution": "revoked", "comment": "rotated"}),
    );
    let preview = gh.preview(ME, &c).await.unwrap();
    assert!(preview.lines[0].contains("(GitHub Token) as resolved"));
    assert!(!format!("{preview:?}").contains("LEAKED"), "the preview never carries the leaked secret");
    holds(&gh.perform(ME, &c).await.unwrap(), &json!({"updated": true, "state": "resolved"}));
}

#[tokio::test]
async fn account_changes_go_to_the_account_and_send_the_right_requests() {
    let cases = vec![
        Write::new("github_star_add", json!({"repo": "octo/cat"}), "PUT", "/user/starred/octo/cat", "Star octo/cat")
            .result(json!({"starred": true, "repo": "octo/cat"})),
        Write::new(
            "github_star_remove",
            json!({"repo": "octo/cat"}),
            "DELETE",
            "/user/starred/octo/cat",
            "Remove your star from octo/cat",
        )
        .result(json!({"starred": false})),
        Write::new(
            "github_watch_add",
            json!({"repo": "octo/cat"}),
            "PUT",
            "/repos/octo/cat/subscription",
            "Watch octo/cat",
        )
        .body(json!({"subscribed": true, "ignored": false}))
        .status(200, json!({}))
        .result(json!({"watching": true})),
        Write::new(
            "github_watch_add",
            json!({"repo": "octo/cat", "ignored": true}),
            "PUT",
            "/repos/octo/cat/subscription",
            "Ignore all notifications of octo/cat",
        )
        .body(json!({"subscribed": false, "ignored": true}))
        .status(200, json!({}))
        .result(json!({"watching": false, "ignored": true})),
        Write::new(
            "github_watch_remove",
            json!({"repo": "octo/cat"}),
            "DELETE",
            "/repos/octo/cat/subscription",
            "Stop watching octo/cat",
        )
        .result(json!({"watching": false})),
        Write::new("github_follow_add", json!({"username": "ann"}), "PUT", "/user/following/ann", "Follow ann")
            .result(json!({"following": true, "user": "ann"})),
        Write::new("github_follow_remove", json!({"username": "ann"}), "DELETE", "/user/following/ann", "Unfollow ann")
            .result(json!({"following": false})),
        Write::new(
            "github_notification_mark_read",
            json!({"thread_id": "123"}),
            "PATCH",
            "/notifications/threads/123",
            "Mark notification thread 123 as read",
        )
        .status(205, Value::Null)
        .result(json!({"marked_read": true, "thread_id": "123"})),
        Write::new(
            "github_notification_mark_read",
            json!({}),
            "PUT",
            "/notifications",
            "Mark all your GitHub notifications as read",
        )
        .body(json!({"read": true}))
        .status(202, json!({}))
        .result(json!({"marked_read": true})),
        Write::new(
            "github_notification_mark_read",
            json!({"repo": "octo/cat"}),
            "PUT",
            "/repos/octo/cat/notifications",
            "Mark all notifications of octo/cat as read",
        )
        .body(json!({"read": true}))
        .status(205, Value::Null),
        Write::new("github_gist_star", json!({"gist_id": "abc123"}), "PUT", "/gists/abc123/star", "Star gist abc123")
            .result(json!({"starred": true})),
        Write::new(
            "github_gist_star",
            json!({"gist_id": "abc123", "unstar": true}),
            "DELETE",
            "/gists/abc123/star",
            "Remove your star from gist abc123",
        )
        .result(json!({"starred": false})),
        Write::new(
            "github_gist_delete",
            json!({"gist_id": "abc123"}),
            "DELETE",
            "/gists/abc123",
            "Delete gist abc123 'notes' for good",
        )
        .lookup("/gists/abc123", json!({"description": "notes", "files": {"a.txt": {}}}))
        .result(json!({"deleted": true, "gist_id": "abc123"})),
    ];
    for w in cases {
        check_write(w.account()).await;
    }
}

#[tokio::test]
async fn gists_are_created_and_changed_with_their_files_shown() {
    let done = check_write(
        Write::new(
            "github_gist_create",
            json!({"description": "d", "files": {"a.txt": "hello"}, "public": true}),
            "POST",
            "/gists",
            "Create a PUBLIC gist with 1 file(s)",
        )
        .account()
        .body(json!({"description": "d", "public": true, "files": {"a.txt": {"content": "hello"}}}))
        .status(201, json!({"id": "abc", "html_url": "https://gist.github.com/abc", "owner": {"login": "octo-cat"}}))
        .result(json!({"created": true, "public": true, "id": "abc", "html_url": "https://gist.github.com/abc"})),
    )
    .await;
    assert!(done.get("owner").is_none(), "only ids and links are handed on");
    check_write(
        Write::new("github_gist_create", json!({"files": {"a.txt": "x"}}), "POST", "/gists", "Create a secret gist")
            .account()
            .body(json!({"description": "", "public": false, "files": {"a.txt": {"content": "x"}}}))
            .status(201, json!({"id": "def"})),
    )
    .await;
    check_write(
        Write::new(
            "github_gist_update",
            json!({"gist_id": "abc", "description": "n", "files": {"a.txt": "new text", "old.txt": null}}),
            "PATCH",
            "/gists/abc",
            "Change gist abc 'notes'",
        )
        .account()
        .lookup("/gists/abc", json!({"description": "notes"}))
        .body(json!({"description": "n", "files": {"a.txt": {"content": "new text"}, "old.txt": null}}))
        .status(200, json!({"html_url": "u"}))
        .result(json!({"updated": true, "gist_id": "abc", "html_url": "u"})),
    )
    .await;
    let server = MockServer::start().await;
    let (gh, _dir) = github(&server).await;
    let c = call("github_gist_create", &json!({"files": {"big.txt": "y".repeat(2_000)}}));
    let preview = gh.preview(ME, &c).await.unwrap();
    assert!(
        preview.lines[1].starts_with("File big.txt: 2000 characters") && preview.lines[1].len() < 400,
        "{:?}",
        preview.lines
    );
    for bad in [
        json!({"files": {}}),
        json!({"files": {"a/b.txt": "x"}}),
        json!({"files": {"a.txt": ""}}),
        json!({"files": {"a.txt": 5}}),
        json!({"files": ["a"]}),
        json!({"description": "no files"}),
    ] {
        let Ok((c, _)) = spec_for_tool("github_gist_create").unwrap().parse(&bad) else {
            continue;
        };
        assert!(gh.preview(ME, &c).await.is_err(), "{bad}");
    }
    let c = call("github_gist_update", &json!({"gist_id": "abc"}));
    assert!(text(&gh.preview(ME, &c).await.unwrap_err()).contains("Give a new"));
    let c = call("github_gist_update", &json!({"gist_id": "abc", "files": {"a.txt": null}}));
    json_at(&server, "/gists/abc", &json!({"description": "x"})).await;
    assert!(gh.preview(ME, &c).await.unwrap().lines.iter().any(|l| l == "Delete file a.txt"));
    assert!(requests(&server).await.iter().all(|r| r.method.as_str() == "GET"));
}

// ---- Secrets ---------------------------------------------------------------------------------------------------------

const VALUE: &str = "s3cr3t-value-123";

#[tokio::test]
async fn a_secret_is_sealed_with_the_public_key_and_never_sent_or_shown_in_the_clear() {
    let server = MockServer::start().await;
    let (gh, _dir) = github(&server).await;
    let secret_key = crypto_box::SecretKey::generate(&mut OsRng);
    let key_id = "568250167242549743";
    json_at(
        &server,
        "/repos/octo/cat/actions/secrets/public-key",
        &json!({"key_id": key_id, "key": BASE64.encode(secret_key.public_key().as_bytes())}),
    )
    .await;
    status_at(&server, "GET", "/repos/octo/cat/actions/secrets/API_KEY", 404, &json!({"message": "Not Found"})).await;
    Mock::given(method("PUT"))
        .and(path("/repos/octo/cat/actions/secrets/API_KEY"))
        .respond_with(ResponseTemplate::new(201))
        .expect(2)
        .mount(&server)
        .await;
    let c = call("github_secret_set", &json!({"repo": "octo/cat", "name": "API_KEY", "value": VALUE}));

    let preview = gh.preview(ME, &c).await.unwrap();
    assert!(preview.once_only);
    assert_eq!(preview.lines[0], "Set Actions secret API_KEY in octo/cat");
    assert!(preview.lines.iter().any(|l| l.starts_with("value set (16 characters)")), "{:?}", preview.lines);
    assert!(preview.lines.iter().any(|l| l == "Creates a new secret"));
    assert!(!format!("{preview:?}").contains(VALUE));
    let sent: Vec<_> = requests(&server).await.iter().map(|r| format!("{} {}", r.method, r.url.path())).collect();
    assert_eq!(
        sent,
        ["GET /repos/octo/cat/actions/secrets/API_KEY"],
        "a preview neither fetches the key nor sends anything"
    );

    let done = gh.perform(ME, &c).await.unwrap();
    assert_eq!(done, json!({"set": true, "name": "API_KEY", "environment": null}));
    assert!(!done.to_string().contains(VALUE));
    let all = requests(&server).await;
    let put = all.iter().find(|r| r.method.as_str() == "PUT").expect("the secret was put");
    let body: Value = serde_json::from_slice(&put.body).unwrap();
    assert_eq!(body["key_id"], key_id);
    let sealed = BASE64.decode(body["encrypted_value"].as_str().unwrap().as_bytes()).unwrap();
    assert_eq!(secret_key.unseal(&sealed).unwrap(), VALUE.as_bytes(), "the key holder can read what was sent");
    assert_eq!(body.as_object().unwrap().len(), 2);
    for r in &all {
        assert!(!String::from_utf8_lossy(&r.body).contains(VALUE) && !r.url.to_string().contains(VALUE));
    }
    // A second seal of the same value differs (a fresh ephemeral key each time).
    gh.perform(ME, &c).await.unwrap();
    let puts: Vec<Vec<u8>> =
        requests(&server).await.into_iter().filter(|r| r.method.as_str() == "PUT").map(|r| r.body).collect();
    assert_eq!(puts.len(), 2);
    assert_ne!(puts[0], puts[1]);
}

#[tokio::test]
async fn an_environment_secret_uses_the_environment_key_and_replacing_is_said() {
    let server = MockServer::start().await;
    let (gh, _dir) = github(&server).await;
    let secret_key = crypto_box::SecretKey::generate(&mut OsRng);
    json_at(
        &server,
        "/repos/octo/cat/environments/prod/secrets/public-key",
        &json!({"key_id": "k1", "key": BASE64.encode(secret_key.public_key().as_bytes())}),
    )
    .await;
    json_at(
        &server,
        "/repos/octo/cat/environments/prod/secrets/DB_PASS",
        &json!({"name": "DB_PASS", "updated_at": "2026-09-01T10:00:00Z"}),
    )
    .await;
    Mock::given(method("PUT"))
        .and(path("/repos/octo/cat/environments/prod/secrets/DB_PASS"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    let c = call(
        "github_secret_set",
        &json!({"repo": "octo/cat", "environment": "prod", "name": "DB_PASS", "value": "pässwörd"}),
    );
    let preview = gh.preview(ME, &c).await.unwrap();
    assert!(preview.lines[0].contains("in environment 'prod' of octo/cat"));
    assert!(
        preview.lines.iter().any(|l| l.starts_with("value set (8 characters)")),
        "characters, not bytes: {:?}",
        preview.lines
    );
    assert!(preview.lines.iter().any(|l| l == "Replaces the existing secret (last changed 2026-09-01T10:00:00Z)"));
    holds(&gh.perform(ME, &c).await.unwrap(), &json!({"set": true, "environment": "prod"}));
    let put = requests(&server).await.into_iter().find(|r| r.method.as_str() == "PUT").unwrap();
    let body: Value = serde_json::from_slice(&put.body).unwrap();
    let sealed = BASE64.decode(body["encrypted_value"].as_str().unwrap().as_bytes()).unwrap();
    assert_eq!(secret_key.unseal(&sealed).unwrap(), "pässwörd".as_bytes());
}

#[tokio::test]
async fn secret_failures_never_carry_the_value_and_bad_names_and_keys_are_refused() {
    let server = MockServer::start().await;
    let (gh, _dir) = github(&server).await;
    let secret_key = crypto_box::SecretKey::generate(&mut OsRng);
    json_at(
        &server,
        "/repos/octo/cat/actions/secrets/public-key",
        &json!({"key_id": "k", "key": BASE64.encode(secret_key.public_key().as_bytes())}),
    )
    .await;
    status_at(&server, "PUT", "/repos/octo/cat/actions/secrets/API_KEY", 422, &json!({"message": "Bad key_id"})).await;
    let c = call("github_secret_set", &json!({"repo": "octo/cat", "name": "API_KEY", "value": VALUE}));
    let err = gh.perform(ME, &c).await.unwrap_err();
    assert!(
        text(&err).contains("Bad key_id") && !text(&err).contains(VALUE) && !text(&err).contains("ghp_secret"),
        "{err}"
    );

    for name in ["GITHUB_TOKEN", "github_x", "bad-name", "a b", "a/b", "../x", ""] {
        let Ok((c, _)) = spec_for_tool("github_secret_set")
            .unwrap()
            .parse(&json!({"repo": "octo/cat", "name": name, "value": VALUE}))
        else {
            continue;
        };
        assert!(gh.preview(ME, &c).await.is_err(), "{name:?}");
    }

    let other = MockServer::start().await;
    let (gh, _dir) = github(&other).await;
    json_at(&other, "/repos/octo/cat/actions/secrets/public-key", &json!({"key_id": "k", "key": "AAAA"})).await;
    let err = gh.perform(ME, &c).await.unwrap_err();
    assert!(text(&err).contains("public key that cannot be used") && !text(&err).contains(VALUE), "{err}");
    json_at(&other, "/repos/octo/cat/actions/secrets/public-key", &json!({})).await;
    assert!(requests(&other).await.iter().all(|r| r.method.as_str() == "GET"));
}

#[tokio::test]
async fn deleting_a_secret_says_whether_it_exists_and_asks_every_time() {
    let (_d, _s) = (
        check_write(
            Write::new(
                "github_secret_delete",
                json!({"repo": "octo/cat", "name": "API_KEY"}),
                "DELETE",
                "/repos/octo/cat/actions/secrets/API_KEY",
                "Delete Actions secret API_KEY from octo/cat",
            )
            .lookup(
                "/repos/octo/cat/actions/secrets/API_KEY",
                json!({"name": "API_KEY", "updated_at": "2026-09-01T10:00:00Z"}),
            )
            .once()
            .result(json!({"deleted": true, "name": "API_KEY"})),
        )
        .await,
        (),
    );
    check_write(
        Write::new(
            "github_secret_delete",
            json!({"repo": "octo/cat", "environment": "prod", "name": "API_KEY"}),
            "DELETE",
            "/repos/octo/cat/environments/prod/secrets/API_KEY",
            "from environment 'prod' of octo/cat",
        )
        .once(),
    )
    .await;
    let server = MockServer::start().await;
    let (gh, _dir) = github(&server).await;
    status_at(&server, "GET", "/repos/octo/cat/actions/secrets/NOPE", 404, &json!({"message": "Not Found"})).await;
    let preview =
        gh.preview(ME, &call("github_secret_delete", &json!({"repo": "octo/cat", "name": "NOPE"}))).await.unwrap();
    assert!(preview.lines.iter().any(|l| l == "No secret with that name exists") && preview.once_only);
}

// ---- Logs and downloads that redirect -----------------------------------------------------------------------------

async fn redirect(server: &MockServer, from: &str, to: &str) {
    Mock::given(method("GET"))
        .and(path(from))
        .and(header("authorization", "Bearer ghp_secret"))
        .respond_with(ResponseTemplate::new(302).insert_header("Location", to))
        .expect(1)
        .mount(server)
        .await;
}

async fn text_at(server: &MockServer, p: &str, body: &str) {
    Mock::given(method("GET"))
        .and(path(p))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .mount(server)
        .await;
}

#[tokio::test]
async fn a_job_log_is_followed_through_the_redirect_without_the_token() {
    let server = MockServer::start().await;
    let (gh, _dir) = github(&server).await;
    redirect(&server, "/repos/octo/cat/actions/jobs/5/logs", &format!("{}/blob/job5.txt?sig=abc", server.uri())).await;
    text_at(
        &server,
        "/blob/job5.txt",
        "\u{feff}2026-09-01T10:00:00Z \u{1b}[31;1merror\u{1b}[0m: it broke\r\nsecond line\n",
    )
    .await;
    let items = fetch(&gh, "github_job_logs", &json!({"repo": "octo/cat", "job_id": 5})).await;
    assert_eq!(items[0].body.as_deref(), Some("2026-09-01T10:00:00Z error: it broke\nsecond line\n"));
    assert_eq!((items[0].id.as_str(), items[0].extra["truncated"].clone()), ("5", json!(false)));
    assert_eq!(items[0].resource, "octo/cat");
    assert!(!items[0].sensitive && !items[0].secret);
    let all = requests(&server).await;
    let api = all.iter().find(|r| r.url.path().ends_with("/logs")).unwrap();
    assert_eq!(api.headers.get("authorization").unwrap().to_str().unwrap(), "Bearer ghp_secret");
    let blob = all.iter().find(|r| r.url.path() == "/blob/job5.txt").unwrap();
    assert!(blob.headers.get("authorization").is_none(), "the token is never sent to where the redirect leads");
    assert_eq!(blob.url.query(), Some("sig=abc"));
}

#[tokio::test]
async fn a_long_log_keeps_its_end_from_a_line_start() {
    let server = MockServer::start().await;
    let (gh, _dir) = github(&server).await;
    redirect(&server, "/repos/octo/cat/actions/jobs/5/logs", &format!("{}/blob/big.txt", server.uri())).await;
    let log: String = (0..9_000).map(|i| format!("line {i:05} of the very long build log\n")).collect();
    assert!(log.len() > 300_000);
    text_at(&server, "/blob/big.txt", &log).await;
    let items = fetch(&gh, "github_job_logs", &json!({"repo": "octo/cat", "job_id": 5})).await;
    let body = items[0].body.as_deref().unwrap();
    assert!(body.chars().count() <= 60_000 && body.chars().count() > 50_000, "{}", body.chars().count());
    assert!(body.ends_with("line 08999 of the very long build log\n"), "the end is kept");
    assert!(body.starts_with("line "), "a partial first line is dropped");
    assert_eq!(items[0].extra["truncated"], true);
    assert!(items[0].snippet.contains("(end of the log)"));
}

#[tokio::test]
async fn only_the_last_lines_are_returned_when_asked() {
    let server = MockServer::start().await;
    let (gh, _dir) = github(&server).await;
    redirect(&server, "/repos/octo/cat/actions/jobs/5/logs", &format!("{}/blob/x.txt", server.uri())).await;
    text_at(&server, "/blob/x.txt", "one\ntwo\nthree\nfour\n").await;
    let items = fetch(&gh, "github_job_logs", &json!({"repo": "octo/cat", "job_id": 5, "tail_lines": 2})).await;
    assert_eq!(items[0].body.as_deref(), Some("three\nfour"));
    assert_eq!(items[0].extra["truncated"], true);
}

#[tokio::test]
async fn a_log_without_a_redirect_is_read_and_bad_redirects_and_missing_logs_are_reported() {
    let server = MockServer::start().await;
    let (gh, _dir) = github(&server).await;
    text_at(&server, "/repos/octo/cat/actions/jobs/1/logs", "direct\n").await;
    assert_eq!(
        fetch(&gh, "github_job_logs", &json!({"repo": "octo/cat", "job_id": 1})).await[0].body.as_deref(),
        Some("direct\n")
    );
    for (job, to) in [
        (2, "http://evil.example/steal"),
        (3, "https://127.0.0.1/x"),
        (4, "https://user:pw@evil.example/x"),
        (6, "file:///etc/passwd"),
    ] {
        redirect(&server, &format!("/repos/octo/cat/actions/jobs/{job}/logs"), to).await;
        let err = fetch_err(&gh, "github_job_logs", &json!({"repo": "octo/cat", "job_id": job})).await;
        assert!(text(&err).contains("not allowed"), "{to}: {err}");
    }
    status_at(&server, "GET", "/repos/octo/cat/actions/jobs/7/logs", 404, &json!({"message": "Not Found"})).await;
    assert!(
        text(&fetch_err(&gh, "github_job_logs", &json!({"repo": "octo/cat", "job_id": 7})).await)
            .contains("does not exist")
    );
    redirect(&server, "/repos/octo/cat/actions/jobs/8/logs", &format!("{}/blob/gone.txt", server.uri())).await;
    status_at(&server, "GET", "/blob/gone.txt", 404, &json!({})).await;
    let err = fetch_err(&gh, "github_job_logs", &json!({"repo": "octo/cat", "job_id": 8})).await;
    assert!(text(&err).contains("no longer available") && text(&err).contains("404"), "{err}");
    assert!(!requests(&server).await.iter().any(|r| r.url.host_str() == Some("evil.example")));
}

#[tokio::test]
async fn the_logs_of_a_run_come_job_by_job_failed_first() {
    let server = MockServer::start().await;
    let (gh, _dir) = github(&server).await;
    let jobs = json!({"jobs": [
        {"id": 6, "name": "test", "status": "completed", "conclusion": "success"},
        {"id": 5, "name": "build", "status": "completed", "conclusion": "failure", "steps": [{"name": "Build", "conclusion": "failure"}]},
        {"id": 7, "name": "deploy", "status": "completed", "conclusion": "skipped"}
    ]});
    Mock::given(method("GET"))
        .and(path("/repos/octo/cat/actions/runs/77/jobs"))
        .and(query_param("filter", "latest"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&jobs))
        .mount(&server)
        .await;
    for (id, log) in [(5, "build failed\n"), (6, "tests ok\n")] {
        redirect(
            &server,
            &format!("/repos/octo/cat/actions/jobs/{id}/logs"),
            &format!("{}/blob/{id}.txt", server.uri()),
        )
        .await;
        text_at(&server, &format!("/blob/{id}.txt"), log).await;
    }
    status_at(&server, "GET", "/repos/octo/cat/actions/jobs/7/logs", 404, &json!({"message": "Not Found"})).await;
    let items = fetch(&gh, "github_run_logs", &json!({"repo": "octo/cat", "run_id": 77})).await;
    assert_eq!(items.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), ["5", "6", "7"]);
    assert_eq!(items[0].body.as_deref(), Some("build failed\n"));
    assert!(items[0].snippet.contains("failed step: Build"));
    assert_eq!(items[1].body.as_deref(), Some("tests ok\n"));
    assert!(items[2].body.is_none() && items[2].snippet.contains("no log:"), "{:?}", items[2].snippet);
}

#[tokio::test]
async fn the_logs_of_a_run_share_the_character_budget() {
    let server = MockServer::start().await;
    let (gh, _dir) = github(&server).await;
    let jobs: Vec<Value> =
        (1..=3).map(|id| json!({"id": id, "name": format!("job{id}"), "conclusion": "failure"})).collect();
    json_at(&server, "/repos/octo/cat/actions/runs/77/jobs", &json!({"jobs": jobs})).await;
    let log: String = (0..5_000).map(|i| format!("row {i:05} some text\n")).collect();
    for id in 1..=3 {
        redirect(
            &server,
            &format!("/repos/octo/cat/actions/jobs/{id}/logs"),
            &format!("{}/blob/{id}.txt", server.uri()),
        )
        .await;
        text_at(&server, &format!("/blob/{id}.txt"), &log).await;
    }
    let items = fetch(&gh, "github_run_logs", &json!({"repo": "octo/cat", "run_id": 77})).await;
    let total: usize = items.iter().map(|i| i.body.as_deref().unwrap().chars().count()).sum();
    assert!(total <= 60_000 && items.iter().all(|i| i.extra["truncated"] == true), "{total}");
    assert!(items[0].body.as_deref().unwrap().ends_with("row 04999 some text\n"));
}

#[tokio::test]
async fn an_artifact_link_is_returned_but_not_fetched_and_kept_out_of_the_approval() {
    let server = MockServer::start().await;
    let (gh, _dir) = github(&server).await;
    let link = "https://productionresultssa1.blob.core.windows.net/actions-results/x.zip?sig=SIGNATURE";
    redirect(&server, "/repos/octo/cat/actions/artifacts/9/zip", link).await;
    let items = fetch(&gh, "github_artifact_download_url", &json!({"repo": "octo/cat", "artifact_id": 9})).await;
    assert_eq!(items[0].body.as_deref(), Some(link));
    assert!(items[0].secret && !items[0].sensitive);
    assert!(
        !items[0].text().contains("SIGNATURE")
            && !items[0].snippet.contains("SIGNATURE")
            && !items[0].title.contains("SIGNATURE")
    );
    assert_eq!(requests(&server).await.len(), 1, "only the API call was made");

    redirect(&server, "/repos/octo/cat/actions/artifacts/10/zip", "http://evil.example/x.zip").await;
    assert!(
        text(&fetch_err(&gh, "github_artifact_download_url", &json!({"repo": "octo/cat", "artifact_id": 10})).await)
            .contains("not allowed")
    );
    text_at(&server, "/repos/octo/cat/actions/artifacts/11/zip", "PK...").await;
    assert!(
        text(&fetch_err(&gh, "github_artifact_download_url", &json!({"repo": "octo/cat", "artifact_id": 11})).await)
            .contains("download link")
    );
    status_at(
        &server,
        "GET",
        "/repos/octo/cat/actions/artifacts/12/zip",
        410,
        &json!({"message": "Artifact has expired"}),
    )
    .await;
    let err = fetch_err(&gh, "github_artifact_download_url", &json!({"repo": "octo/cat", "artifact_id": 12})).await;
    assert!(text(&err).contains("410") && text(&err).contains("expired"), "{err}");
}

// ---- The generic request -------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_raw_read_under_a_repository_is_an_ordinary_read() {
    let server = MockServer::start().await;
    let (gh, _dir) = github(&server).await;
    Mock::given(method("GET"))
        .and(path("/repos/octo/cat/issues"))
        .and(query_param("state", "all"))
        .and(query_param("labels", "bug&x=1#frag"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([{"number": 1}])))
        .expect(1)
        .mount(&server)
        .await;
    let items = fetch(
        &gh,
        "github_request_read",
        &json!({"path": "/repos/octo/cat/issues", "query": {"state": "all", "labels": "bug&x=1#frag"}}),
    )
    .await;
    assert_eq!(items.len(), 1);
    let item = &items[0];
    assert_eq!((item.resource.as_str(), item.sensitive, item.secret), ("octo/cat", false, false));
    assert_eq!(item.title, "GET /repos/octo/cat/issues");
    assert!(item.body.as_deref().unwrap().contains("\"number\": 1"));
    assert_eq!((item.extra["status"].clone(), item.extra["truncated"].clone()), (json!(200), json!(false)));
    let sent = requests(&server).await;
    assert_eq!(
        sent[0].url.query_pairs().find(|(k, _)| k == "labels").unwrap().1,
        "bug&x=1#frag",
        "values are encoded, never smuggled"
    );
}

#[tokio::test]
async fn a_raw_read_outside_repositories_is_sensitive_and_about_the_account() {
    let server = MockServer::start().await;
    let (gh, _dir) = github(&server).await;
    json_at(&server, "/user/orgs", &json!([{"login": "acme"}])).await;
    text_at(&server, "/rate_limit", "plain text answer").await;
    let items = fetch(&gh, "github_request_read", &json!({"path": "/user/orgs"})).await;
    assert_eq!(
        (items[0].resource.as_str(), items[0].resource_label.as_str(), items[0].sensitive),
        ("account", "Your GitHub account", true)
    );
    assert!(items[0].body.as_deref().unwrap().contains("acme"));
    let items = fetch(&gh, "github_request_read", &json!({"path": "/rate_limit"})).await;
    assert_eq!(items[0].body.as_deref(), Some("plain text answer"));
    assert!(items[0].sensitive);
}

#[tokio::test]
async fn a_huge_raw_answer_is_cut() {
    let server = MockServer::start().await;
    let (gh, _dir) = github(&server).await;
    let big: Vec<Value> = (0..20_000).map(|i| json!({"n": i, "text": "some words to fill the answer"})).collect();
    json_at(&server, "/repos/octo/cat/git/trees/abc", &Value::Array(big)).await;
    let items = fetch(&gh, "github_request_read", &json!({"path": "/repos/octo/cat/git/trees/abc"})).await;
    assert!(items[0].body.as_deref().unwrap().chars().count() <= 60_000);
    assert_eq!(items[0].extra["truncated"], true);
}

#[tokio::test]
async fn raw_paths_that_could_escape_never_reach_the_network() {
    let server = MockServer::start().await;
    let (gh, _dir) = github(&server).await;
    let bad = [
        "/repos/octo/cat/../../user/emails",
        "/repos/octo/cat/%2e%2e/x",
        "/repos/octo/cat/a%2fb",
        "/repos/octo/cat/a%2Fb",
        "/repos/octo/cat/issues?state=all",
        "/repos/octo/cat/issues#x",
        "/graphql",
        "/app",
        "/app/installations",
        "/authorizations",
        "/applications/abc/token",
        "/user/keys",
        "/user/gpg_keys",
        "/user/emails",
        "/user/tokens",
        "/user/keys/5",
        "/USER/EMAILS",
        "//repos/octo/cat",
        "repos/octo/cat",
        "/repos/octo",
        "/repos/octo/cat/",
        "/repos/octo/cat//issues",
        "/repos/octo/cat/a\\b",
        "/orgs",
        "/users",
        "/somewhere/else",
        "/repos/octo%2fcat/x",
        "/repos/octo/cat/%00",
        "/repos/octo/cat/a b",
    ];
    for p in bad {
        let Ok((c, _)) = spec_for_tool("github_request_read").unwrap().parse(&json!({"path": p})) else {
            continue;
        };
        let err = gh.fetch(ME, &c).await.expect_err(p);
        assert!(text(&err).contains("path is not allowed"), "{p}: {err}");
        let Ok((c, _)) = spec_for_tool("github_request_write").unwrap().parse(&json!({"path": p, "method": "post"}))
        else {
            continue;
        };
        assert!(gh.preview(ME, &c).await.is_err() && gh.perform(ME, &c).await.is_err(), "{p}");
    }
    assert!(requests(&server).await.is_empty(), "nothing was sent");
    let c = call("github_request_read", &json!({"path": "/repos/octo/cat/issues", "query": {"a b": "1"}}));
    assert!(text(&gh.fetch(ME, &c).await.unwrap_err()).contains("query name"));
    assert!(requests(&server).await.is_empty());
}

#[tokio::test]
async fn a_raw_write_is_previewed_in_full_asked_every_time_and_sends_only_what_was_shown() {
    let server = MockServer::start().await;
    let (gh, _dir) = github(&server).await;
    Mock::given(method("POST"))
        .and(path("/repos/octo/cat/labels"))
        .and(query_param("dry", "no"))
        .and(body_json(json!({"name": "bug", "color": "ff0000"})))
        .respond_with(
            ResponseTemplate::new(201).set_body_json(json!({"id": 1, "name": "bug", "secret_field": "hidden"})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let c = call(
        "github_request_write",
        &json!({"method": "post", "path": "/repos/octo/cat/labels", "query": {"dry": "no"}, "body": {"name": "bug", "color": "ff0000"}}),
    );
    let preview = gh.preview(ME, &c).await.unwrap();
    assert!(preview.once_only);
    assert_eq!((preview.resource.as_str(), preview.resource_label.as_str()), ("octo/cat", "octo/cat"));
    assert_eq!(preview.lines[0], "Send POST /repos/octo/cat/labels to the GitHub API");
    assert!(preview.lines.iter().any(|l| l == "Query dry = no"));
    assert!(preview.lines.iter().any(|l| l.starts_with("Body:\n") && l.contains("\"color\": \"ff0000\"")));
    assert!(requests(&server).await.is_empty(), "the preview sent nothing");
    let done = gh.perform(ME, &c).await.unwrap();
    assert_eq!(done, json!({"sent": true, "method": "POST", "path": "/repos/octo/cat/labels"}));
    assert!(!done.to_string().contains("hidden"), "the answer body is not handed on");
    let spec = spec_for_tool("github_request_write").unwrap();
    assert_eq!((spec.once_only, spec.class), (true, "settings"));

    status_at(&server, "DELETE", "/user/starred/octo/cat", 204, &Value::Null).await;
    let c = call("github_request_write", &json!({"method": "delete", "path": "/user/starred/octo/cat"}));
    let preview = gh.preview(ME, &c).await.unwrap();
    assert_eq!(
        (preview.resource.as_str(), preview.resource_label.as_str(), preview.once_only),
        ("account", "Your GitHub account", true)
    );
    assert_eq!(gh.perform(ME, &c).await.unwrap()["method"], "DELETE");
    for verb in ["put", "patch"] {
        let c = call("github_request_write", &json!({"method": verb, "path": "/user"}));
        assert_eq!(
            gh.preview(ME, &c).await.unwrap().lines[0],
            format!("Send {} /user to the GitHub API", verb.to_uppercase())
        );
    }
    let long = "x".repeat(10_000);
    let c = call(
        "github_request_write",
        &json!({"method": "post", "path": "/repos/octo/cat/issues", "body": {"body": long}}),
    );
    let preview = gh.preview(ME, &c).await.unwrap();
    assert!(preview.lines.iter().any(|l| l.ends_with("(shortened)")) && preview.lines.iter().all(|l| l.len() < 1_700));
}

// ---- Refusals and errors ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn bad_arguments_are_refused_before_anything_is_sent() {
    let server = MockServer::start().await;
    let (gh, _dir) = github(&server).await;
    let reads = [
        ("github_workflow_get", json!({"repo": "octo/cat", "workflow": "../x.yml"})),
        ("github_workflow_get", json!({"repo": "octo/cat", "workflow": "ci"})),
        ("github_workflow_list", json!({"repo": "octo"})),
        ("github_workflow_usage", json!({"repo": "octo/cat", "workflow": "a/b.yml"})),
        ("github_run_list", json!({"repo": "octo/cat", "workflow": "a/b.yml"})),
        ("github_run_list", json!({"repo": "octo/cat", "branch": "bad..name"})),
        ("github_run_get", json!({"repo": "../../etc", "run_id": 1})),
        ("github_job_logs", json!({"repo": "a/b/c", "job_id": 1})),
        ("github_variable_get", json!({"repo": "octo/cat", "name": "a-b"})),
        ("github_variable_get", json!({"repo": "octo/cat", "name": "../x"})),
        ("github_user_get", json!({"username": "a/b"})),
        ("github_org_get", json!({"org": "x y"})),
        ("github_org_members", json!({"org": "../x"})),
        ("github_starred_list", json!({"username": "a/b"})),
        ("github_gist_list", json!({"username": "a?b"})),
        ("github_gist_get", json!({"gist_id": "../x"})),
        ("github_notification_list", json!({"repo": "nope"})),
        ("github_dependabot_alert_get", json!({"repo": "octo/cat/x", "number": 1})),
        ("github_environment_get", json!({"repo": "octo/cat"})),
    ];
    for (tool, args) in reads {
        let Ok((c, _)) = spec_for_tool(tool).unwrap().parse(&args) else {
            continue;
        };
        assert!(gh.fetch(ME, &c).await.is_err(), "{tool} {args}");
    }
    let writes = [
        ("github_workflow_dispatch", json!({"repo": "octo/cat", "workflow": "ci.yml", "ref": "bad..ref"})),
        ("github_workflow_dispatch", json!({"repo": "octo/cat", "workflow": "x/y.yml", "ref": "main"})),
        ("github_workflow_enable", json!({"repo": "octo/cat", "workflow": "ci"})),
        ("github_variable_create", json!({"repo": "octo/cat", "name": "GITHUB_X", "value": "1"})),
        ("github_variable_create", json!({"repo": "octo/cat", "name": "a b", "value": "1"})),
        ("github_variable_update", json!({"repo": "octo/cat", "name": "../x", "value": "1"})),
        ("github_secret_delete", json!({"repo": "octo/cat", "name": "a-b"})),
        ("github_cache_delete", json!({"repo": "octo/cat"})),
        ("github_cache_delete", json!({"repo": "octo/cat", "cache_id": 1, "key": "k"})),
        ("github_dependabot_alert_update", json!({"repo": "octo/cat", "number": 1, "state": "dismissed"})),
        (
            "github_dependabot_alert_update",
            json!({"repo": "octo/cat", "number": 1, "state": "open", "reason": "not_used"}),
        ),
        (
            "github_dependabot_alert_update",
            json!({"repo": "octo/cat", "number": 1, "state": "dismissed", "reason": "not_used", "comment": "x".repeat(281)}),
        ),
        ("github_secret_scanning_alert_update", json!({"repo": "octo/cat", "number": 1, "state": "resolved"})),
        (
            "github_code_scanning_alert_update",
            json!({"repo": "octo/cat", "number": 1, "state": "open", "comment": "c"}),
        ),
        ("github_star_add", json!({"repo": "nope"})),
        ("github_watch_remove", json!({"repo": "a/b/c"})),
        ("github_follow_add", json!({"username": "a/b"})),
        ("github_notification_mark_read", json!({"thread_id": "12/3"})),
        ("github_notification_mark_read", json!({"repo": "nope"})),
        ("github_gist_star", json!({"gist_id": "a/b"})),
        ("github_gist_delete", json!({"gist_id": "../x"})),
        ("github_run_cancel", json!({"repo": "nope", "run_id": 1})),
        ("github_artifact_delete", json!({"repo": "../x/y", "artifact_id": 1})),
        (
            "github_deployment_review",
            json!({"repo": "nope", "run_id": 1, "environments": ["a"], "state": "approved", "comment": "c"}),
        ),
    ];
    for (tool, args) in writes {
        let Ok((c, _)) = spec_for_tool(tool).unwrap().parse(&args) else {
            continue;
        };
        assert!(gh.preview(ME, &c).await.is_err(), "{tool} {args} previews");
        assert!(gh.perform(ME, &c).await.is_err(), "{tool} {args} performs");
    }
    assert!(requests(&server).await.is_empty(), "nothing was sent");
    // The proto layer refuses what the schema forbids.
    for (tool, args) in [
        ("github_run_get", json!({"repo": "octo/cat"})),
        ("github_variable_create", json!({"repo": "octo/cat", "name": "A"})),
        (
            "github_workflow_dispatch",
            json!({"repo": "octo/cat", "workflow": "ci.yml", "ref": "main", "inputs": {"a": 1}}),
        ),
        ("github_request_write", json!({"path": "/user", "method": "get"})),
        ("github_gist_create", json!({})),
    ] {
        assert!(spec_for_tool(tool).unwrap().parse(&args).is_err(), "{tool} {args}");
    }
}

#[tokio::test]
async fn failures_are_explained_without_the_token_and_a_failed_change_is_an_error() {
    let server = MockServer::start().await;
    let (gh, _dir) = github(&server).await;
    status_at(&server, "GET", "/repos/octo/cat/actions/workflows", 404, &json!({"message": "Not Found"})).await;
    status_at(
        &server,
        "GET",
        "/repos/octo/cat/actions/runs",
        403,
        &json!({"message": "Resource not accessible by personal access token"}),
    )
    .await;
    status_at(&server, "GET", "/repos/octo/cat/actions/caches", 401, &json!({"message": "Bad credentials"})).await;
    status_at(&server, "GET", "/repos/octo/cat/actions/artifacts", 422, &json!({"message": "Validation Failed"})).await;
    let err = fetch_err(&gh, "github_workflow_list", &json!({"repo": "octo/cat"})).await;
    assert!(text(&err).contains("does not exist") && !text(&err).contains("ghp_secret"), "{err}");
    let err = fetch_err(&gh, "github_run_list", &json!({"repo": "octo/cat"})).await;
    assert!(text(&err).contains("Resource not accessible"), "{err}");
    let err = fetch_err(&gh, "github_cache_list", &json!({"repo": "octo/cat"})).await;
    assert!(matches!(err, CoreError::ServiceNeedsAttention { .. }), "{err:?}");
    assert!(
        text(&fetch_err(&gh, "github_artifact_list", &json!({"repo": "octo/cat"})).await).contains("Validation Failed")
    );

    Mock::given(method("GET"))
        .and(path("/repos/octo/cat/actions/runs/1/jobs"))
        .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
        .expect(3)
        .mount(&server)
        .await;
    assert!(
        text(&fetch_err(&gh, "github_run_jobs", &json!({"repo": "octo/cat", "run_id": 1})).await).contains("500"),
        "retried, then reported"
    );

    status_at(
        &server,
        "POST",
        "/repos/octo/cat/actions/runs/2/cancel",
        409,
        &json!({"message": "Cannot cancel a workflow run that is completed."}),
    )
    .await;
    json_at(&server, "/repos/octo/cat/actions/runs/2", &run_json(2)).await;
    let c = call("github_run_cancel", &json!({"repo": "octo/cat", "run_id": 2}));
    let err = gh.perform(ME, &c).await.unwrap_err();
    assert!(text(&err).contains("cannot do that right now") && text(&err).contains("completed"), "{err}");
    status_at(&server, "PUT", "/user/starred/octo/cat", 404, &json!({"message": "Not Found"})).await;
    assert!(gh.perform(ME, &call("github_star_add", &json!({"repo": "octo/cat"}))).await.is_err());
    assert_eq!(gh.status(ME).await, GmailStatus::Ready);
}
