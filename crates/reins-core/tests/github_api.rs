//! `github_api_read` and `github_api_write` through the phone: what a path is about (resource, kind of change,
//! asked every time), which paths are refused, how answers come back, and the permissions that cover them.

mod common;

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{FakeGoogle, FakeKeys, RecordingNotifier};
use reins_core::connector::github::GitHub;
use reins_core::connector::github::api::target;
use reins_core::connector::{Connector, Preview};
use reins_core::{
    ApprovalChoice, CoreConfig, CoreError, GmailStatus, GoogleTokenProvider, GrantScopeChoice, Notifier, ReinsCore,
    StandingGrant,
};
use reins_proto::connector::ConnectorCall;
use serde_json::{Value, json};
use wiremock::matchers::{body_json, method, path, path_regex, query_param};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

const GH_TOKEN: &str = "ghp_API-TEST-TOKEN";

struct Env {
    server: MockServer,
    github: MockServer,
    core: Arc<ReinsCore>,
    counter: AtomicU32,
    _dir: tempfile::TempDir,
}

async fn env() -> Env {
    let server = MockServer::start().await;
    let github = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/identity/accounts/prelogin"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"kdf": 0, "kdfIterations": 5000})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/identity/connect/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": common::account_token(), "refresh_token": "REFRESH", "expires_in": 7200})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path_regex(r"^/reins/api/requests/[^/]+/response$"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/reins/api/blobs/fetch"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "blob-archive-0000001",
            "download_url": "https://rw.example/reins/blob/DL", "name": "main", "size": 12_345,
            "sha256": "ef".repeat(32), "content_type": "application/zip", "expires_at": 4_000_000_000_i64})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/user"))
        .and(wiremock::matchers::header("authorization", format!("Bearer {GH_TOKEN}").as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"login": "octo"})))
        .with_priority(1)
        .mount(&github)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let google: Arc<dyn GoogleTokenProvider> = Arc::new(FakeGoogle::new());
    let notifier: Arc<dyn Notifier> = Arc::new(RecordingNotifier::default());
    let cfg = CoreConfig {
        github_base: github.uri(),
        backoff_base: Duration::from_millis(1),
        ..CoreConfig::default()
    };
    let core = ReinsCore::with_config(dir.path().to_str().unwrap(), &FakeKeys, google, notifier, cfg).unwrap();
    common::mount_account_vault(&server, "me@example.com", "pw").await;
    core.login(server.uri(), "me@example.com".to_owned(), "pw".to_owned(), None).await.unwrap();
    core.add_token_account("github".into(), GH_TOKEN.into()).await.unwrap();
    Env {
        server,
        github,
        core,
        counter: AtomicU32::new(0),
        _dir: dir,
    }
}

impl Env {
    async fn ask(&self, op: &str, args: &Value) -> (String, bool) {
        let id = format!("r{}", self.counter.fetch_add(1, Ordering::SeqCst) + 1);
        let request = json!({"v": 1, "id": id, "connection_id": "c1", "connection_label": "Claude",
            "created_at": 100, "call": {"tool": "connector", "service": "github", "op": op, "args": args}});
        Mock::given(method("GET"))
            .and(path("/reins/api/pending"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"requests": [request], "pairings": []})))
            .up_to_n_times(1)
            .with_priority(1)
            .mount(&self.server)
            .await;
        Mock::given(method("GET"))
            .and(path("/reins/api/pending"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"requests": [], "pairings": []})))
            .mount(&self.server)
            .await;
        let parked = self.core.sync(0).await.unwrap().iter().any(|p| p.id == id);
        (id, parked)
    }

    async fn told(&self) -> Value {
        let last = self
            .server
            .received_requests()
            .await
            .unwrap()
            .into_iter()
            .rfind(|r| r.method.as_str() == "POST" && r.url.path().ends_with("/response"))
            .unwrap();
        serde_json::from_slice(&last.body).unwrap()
    }

    /// What GitHub saw besides the sign-in.
    async fn github_saw(&self) -> Vec<Request> {
        self.github
            .received_requests()
            .await
            .unwrap()
            .into_iter()
            .filter(|r| r.url.path() != "/user" && r.method.as_str() != "HEAD")
            .collect()
    }
}

fn choice(ids: &[String], standing: Option<StandingGrant>) -> ApprovalChoice {
    ApprovalChoice {
        selected_message_ids: ids.to_vec(),
        standing,
    }
}

fn grant(resources: &[&str], classes: &[&str]) -> Option<StandingGrant> {
    Some(StandingGrant {
        duration_secs: Some(3_600),
        max_uses: None,
        scope: GrantScopeChoice {
            all_mail: false,
            selected_messages_only: false,
            sender_addresses: vec![],
            sender_domains: vec![],
            subject_pattern: None,
            recipient_addresses: vec![],
            recipient_domains: vec![],
            resources: resources.iter().map(|r| (*r).to_owned()).collect(),
            classes: classes.iter().map(|c| (*c).to_owned()).collect(),
        },
    })
}

/// What `target` says, as (resource, class, asked every time).
fn derived(method_name: &str, p: &str, query: &[(&str, &str)], body: &Value) -> (String, &'static str, bool) {
    let query: Vec<(String, String)> = query.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect();
    let body = (!body.is_null()).then_some(body);
    let t = target(method_name, p, &query, body).unwrap_or_else(|e| panic!("{method_name} {p}: {e}"));
    (t.resource, t.class, t.once)
}

/// method, path, query, body → resource, kind of change, asked every time.
type Row =
    (&'static str, &'static str, &'static [(&'static str, &'static str)], Value, &'static str, &'static str, bool);

#[test]
fn a_call_is_about_what_its_path_names_and_far_reaching_changes_are_asked_every_time() {
    // method, path, query, body → resource, kind of change, asked every time
    let table: &[Row] = &[
        ("POST", "/repos/octo/cat/issues", &[], json!({"title": "Bug"}), "octo/cat", "issues", false),
        ("POST", "/repos/octo/cat/issues/3/comments", &[], json!({"body": "x"}), "octo/cat", "issues", false),
        ("POST", "/repos/octo/cat/labels", &[], json!({"name": "x"}), "octo/cat", "issues", false),
        ("PUT", "/repos/octo/cat/pulls/4/merge", &[], Value::Null, "octo/cat", "pulls", false),
        ("PATCH", "/repos/octo/cat/git/refs/heads/main", &[], json!({"sha": "a"}), "octo/cat@main", "code", false),
        ("PUT", "/repos/octo/cat/contents/a.txt", &[], json!({"branch": "dev"}), "octo/cat@dev", "code", false),
        (
            "DELETE",
            "/repos/octo/cat/contents/a.txt",
            &[("ref", "x")],
            json!({"branch": "dev"}),
            "octo/cat@x",
            "code",
            false,
        ),
        ("POST", "/repos/octo/cat/merges", &[], json!({"base": "main"}), "octo/cat", "code", false),
        ("POST", "/repos/octo/cat/releases", &[], json!({"tag_name": "v1"}), "octo/cat", "releases", false),
        ("POST", "/repos/octo/cat/git/tags", &[], json!({}), "octo/cat", "releases", false),
        ("DELETE", "/repos/octo/cat/git/refs/tags/v1", &[], Value::Null, "octo/cat", "releases", false),
        ("POST", "/repos/octo/cat/actions/runs/9/rerun", &[], Value::Null, "octo/cat", "actions", false),
        ("POST", "/repos/octo/cat/dispatches", &[], json!({"event_type": "x"}), "octo/cat", "actions", false),
        ("PUT", "/repos/octo/cat/topics", &[], json!({"names": []}), "octo/cat", "settings", false),
        ("PATCH", "/repos/octo/cat", &[], json!({"description": "d"}), "octo/cat", "settings", false),
        ("PATCH", "/repos/octo/cat", &[], json!({"private": false}), "octo/cat", "settings", true),
        ("PATCH", "/repos/octo/cat", &[], json!({"visibility": "public"}), "octo/cat", "settings", true),
        ("PATCH", "/repos/octo/cat", &[], json!({"archived": true}), "octo/cat", "settings", true),
        ("PATCH", "/repos/octo/cat", &[], json!({"name": "dog"}), "octo/cat", "settings", true),
        ("DELETE", "/repos/octo/cat", &[], Value::Null, "octo/cat", "settings", true),
        ("POST", "/repos/octo/cat/transfer", &[], json!({"new_owner": "x"}), "octo/cat", "settings", true),
        ("PUT", "/repos/octo/cat/collaborators/eve", &[], Value::Null, "octo/cat", "settings", true),
        ("DELETE", "/repos/octo/cat/invitations/5", &[], Value::Null, "octo/cat", "settings", true),
        ("PUT", "/orgs/acme/teams/core/repos/octo/cat", &[], Value::Null, "acme", "settings", true),
        ("POST", "/repos/octo/cat/hooks", &[], json!({"config": {}}), "octo/cat", "settings", true),
        ("POST", "/repos/octo/cat/keys", &[], json!({"key": "ssh-ed25519 A"}), "octo/cat", "settings", true),
        ("PUT", "/repos/octo/cat/actions/secrets/TOKEN", &[], json!({}), "octo/cat", "actions", true),
        ("PUT", "/repos/octo/cat/branches/main/protection", &[], json!({}), "octo/cat@main", "settings", true),
        ("POST", "/repos/octo/cat/rulesets", &[], json!({}), "octo/cat", "settings", true),
        ("DELETE", "/orgs/acme/memberships/eve", &[], Value::Null, "acme", "settings", true),
        ("PATCH", "/orgs/acme", &[], json!({"blog": "x"}), "acme", "settings", false),
        ("PATCH", "/user", &[], json!({"bio": "hi"}), "account", "account", false),
        ("POST", "/gists", &[], json!({"files": {}}), "account", "account", false),
        ("PUT", "/notifications", &[], Value::Null, "account", "account", false),
        ("POST", "/user/keys", &[], json!({"key": "k"}), "account", "account", true),
        ("POST", "/markdown", &[], json!({"text": "x"}), "github", "account", false),
        // Reads.
        ("GET", "/repos/octo/cat/branches/feature", &[], Value::Null, "octo/cat@feature", "code", false),
        ("GET", "/repos/octo/cat/collaborators", &[], Value::Null, "octo/cat", "settings", false),
        ("GET", "/orgs/acme/repos", &[], Value::Null, "acme", "settings", false),
        ("GET", "/search/issues", &[("q", "x")], Value::Null, "github", "account", false),
    ];
    for (method_name, p, query, body, resource, class, once) in table {
        assert_eq!(
            derived(method_name, p, query, body),
            ((*resource).to_owned(), *class, *once),
            "{method_name} {p} {body}"
        );
    }
    // A branch is part of its repository, which is part of its owner; an organization stands alone.
    let branch = target("DELETE", "/repos/octo/cat/git/refs/heads/feature/x", &[], None).unwrap();
    assert_eq!(branch.resource, "octo/cat@feature/x");
    let parents: Vec<&str> = branch.parents.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(parents, ["octo/cat", "octo"]);
    assert_eq!(target("GET", "/orgs/acme", &[], None).unwrap().parents.len(), 0);
    // Keys and addresses are never read under a permission.
    assert!(target("GET", "/user/emails", &[], None).unwrap().sensitive);
    assert!(target("GET", "/repos/octo/cat/keys", &[], None).unwrap().sensitive);
    assert!(!target("GET", "/repos/octo/cat/issues", &[], None).unwrap().sensitive);
}

const BAD_PATHS: &[&str] = &[
    "repos/octo/cat",
    "/repos/octo/cat/../../user",
    "/repos/octo/cat/%2e%2e/x",
    "/repos/octo/cat/%2E",
    "/repos/octo/cat/a%2Fb",
    "/repos/octo/cat/a%5cb",
    "//evil.example/repos",
    "/https://evil.example/x",
    "/http:/x",
    "/repos/octo/cat/issues?state=all",
    "/repos/octo/cat/issues#top",
    "/repos/octo/cat/\u{e9}",
    "/repos/octo/cat/a\\b",
    "/repos/octo/cat/%0a",
    "/repos/octo/cat/%zz",
    "/repos/octo/cat/./x",
    "/repos/octo",
    "/repos/octo/ca$t",
    "/orgs",
    "/repos/octo/cat/",
];

#[test]
fn paths_that_could_leave_the_api_or_smuggle_segments_are_refused() {
    for bad in BAD_PATHS {
        for m in ["GET", "POST", "DELETE"] {
            let refused = target(m, bad, &[], None).unwrap_err();
            assert!(refused.contains("not allowed"), "{m} {bad}: {refused}");
        }
    }
    assert!(target("GET", &format!("/repos/octo/cat/{}", "a".repeat(500)), &[], None).is_err(), "too long");
    for fine in ["/repos/octo/cat/compare/main...dev", "/repos/octo/cat/contents/dir%20x/a.txt", "/rate_limit"] {
        assert!(target("GET", fine, &[], None).is_ok(), "{fine}");
    }
}

#[tokio::test]
async fn refused_paths_never_reach_github() {
    let env = env().await;
    // Paths the server would relay (no whitespace, no control characters).
    for bad in BAD_PATHS.iter().filter(|p| !p.contains(['\n', ' '])) {
        let (_, parked) = env.ask("api_read", &json!({"path": bad})).await;
        assert!(!parked, "{bad}");
        let told = env.told().await;
        assert_eq!(told["outcome"], "error", "{bad}: {told}");
        assert!(told["message"].as_str().unwrap().contains("not allowed"), "{bad}: {told}");
    }
    let (_, parked) = env.ask("api_read", &json!({"path": "/repos/octo/cat/issues", "query": {"bad key": "1"}})).await;
    assert!(!parked);
    assert!(env.told().await["message"].as_str().unwrap().contains("query parameter"));
    assert!(env.github_saw().await.is_empty(), "GitHub was never called");
}

/// The GitHub connector on its own, signed in against the fake GitHub.
async fn direct(env: &Env) -> (GitHub, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(reins_core::store::Store::open(dir.path(), &FakeKeys).unwrap());
    let gh = GitHub::new(reins_core::http::client().unwrap(), &env.github.uri(), store, Duration::from_millis(1));
    gh.sign_in(GH_TOKEN).await.unwrap();
    (gh, dir)
}

fn write_call(args: &Value) -> ConnectorCall {
    ConnectorCall {
        service: "github".into(),
        op: "api_write".into(),
        args: args.as_object().unwrap().clone(),
    }
}

#[tokio::test]
async fn a_change_shows_method_path_query_and_body_and_is_sent_as_shown() {
    let env = env().await;
    let (gh, _dir) = direct(&env).await;
    let body = json!({"title": "Crash on start", "labels": ["bug"], "long": "y".repeat(3_000)});
    // The method as the tool's choice normalizes it.
    let call = write_call(&json!({"method": "post", "path": "/repos/octo/cat/issues", "query": {"magic": "1"},
        "body": body}));
    let preview = gh.preview("octo", &call).await.unwrap();
    assert_eq!(preview.lines[0], "POST /repos/octo/cat/issues");
    assert_eq!(preview.lines[1], "Query: magic=1");
    assert!(preview.lines[2].starts_with("Body:\n{\n  \"labels\": [\n    \"bug\"\n  ],"), "{}", preview.lines[2]);
    assert!(preview.lines[2].ends_with("characters in all)") && preview.lines[2].len() < 2_200, "the body is cut");
    assert_eq!(
        (preview.resource.as_str(), preview.class.as_deref(), preview.once_only),
        ("octo/cat", Some("issues"), false)
    );
    assert_eq!(preview.parents, [("octo".to_owned(), "Every repository of octo".to_owned())]);
    assert!(env.github_saw().await.is_empty(), "a preview never calls GitHub");

    Mock::given(method("POST"))
        .and(path("/repos/octo/cat/issues"))
        .and(query_param("magic", "1"))
        .and(body_json(&body))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"number": 12, "html_url": "u"})))
        .expect(1)
        .mount(&env.github)
        .await;
    let done = gh.perform("octo", &call).await.unwrap();
    assert_eq!(
        (done["status"].as_u64(), done["response"]["number"].as_u64(), done["method"].as_str()),
        (Some(201), Some(12), Some("POST"))
    );
    let sent = env.github_saw().await;
    assert_eq!(sent[0].headers.get("authorization").unwrap(), format!("Bearer {GH_TOKEN}").as_str());

    // GitHub's refusal is an error, and a far-reaching change says so in its preview.
    Mock::given(method("DELETE"))
        .and(path("/repos/octo/cat"))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({"message": "Must have admin rights"})))
        .mount(&env.github)
        .await;
    let delete = write_call(&json!({"method": "DELETE", "path": "/repos/octo/cat"}));
    assert!(gh.preview("octo", &delete).await.unwrap().once_only);
    let refused = gh.perform("octo", &delete).await.unwrap_err().to_string();
    assert!(refused.contains("Must have admin rights") && !refused.contains(GH_TOKEN), "{refused}");
    let odd = write_call(&json!({"method": "GET", "path": "/repos/octo/cat"}));
    assert!(gh.preview("octo", &odd).await.is_err(), "only POST, PUT, PATCH and DELETE change things");
}

/// A GitHub whose writes say which kind of change they are through their preview (as `github_api_write` does).
#[derive(Default)]
struct ClassFromPreview {
    done: Mutex<u32>,
}

#[async_trait::async_trait]
impl Connector for ClassFromPreview {
    fn service(&self) -> &'static str {
        "github"
    }

    async fn preview(&self, _account: &str, call: &ConnectorCall) -> Result<Preview, CoreError> {
        let class = call.str_arg("body").unwrap().to_owned();
        Ok(Preview {
            resource: "octo/cat".into(),
            resource_label: "octo/cat".into(),
            lines: vec![format!("a {class} change")],
            parents: vec![("octo".into(), "Every repository of octo".into())],
            once_only: class == "settings",
            class: Some(class),
            ..Preview::default()
        })
    }

    async fn perform(&self, _account: &str, _call: &ConnectorCall) -> Result<Value, CoreError> {
        *self.done.lock().unwrap() += 1;
        Ok(json!({"done": true}))
    }

    async fn status(&self, _account: &str) -> GmailStatus {
        GmailStatus::Ready
    }
}

#[tokio::test]
async fn the_kind_of_change_a_preview_names_is_what_permissions_are_checked_against() {
    let server = MockServer::start().await;
    for (verb, p, body) in [
        ("POST", "/identity/accounts/prelogin", json!({"kdf": 0, "kdfIterations": 5000})),
        (
            "POST",
            "/identity/connect/token",
            json!({"access_token": common::account_token(), "refresh_token": "R", "expires_in": 7200}),
        ),
    ] {
        Mock::given(method(verb))
            .and(path(p))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;
    }
    Mock::given(method("POST"))
        .and(path_regex(r"^/reins/api/requests/[^/]+/response$"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let fake = Arc::new(ClassFromPreview::default());
    let google: Arc<dyn GoogleTokenProvider> = Arc::new(FakeGoogle::new());
    let notifier: Arc<dyn Notifier> = Arc::new(RecordingNotifier::default());
    let core = ReinsCore::with_connectors(
        dir.path().to_str().unwrap(),
        &FakeKeys,
        google,
        notifier,
        CoreConfig::default(),
        vec![Arc::<ClassFromPreview>::clone(&fake)],
    )
    .unwrap();
    common::mount_account_vault(&server, "me@example.com", "pw").await;
    core.login(server.uri(), "me@example.com".to_owned(), "pw".to_owned(), None).await.unwrap();
    core.engine().register_account("github", "octo").unwrap();
    let env = Env {
        server,
        github: MockServer::start().await,
        core,
        counter: AtomicU32::new(0),
        _dir: dir,
    };
    // `github_comment` is an "issues" change by its tool; this one's preview says "pulls".
    let comment = |class: &str| json!({"repo": "octo/cat", "number": 1, "body": class});
    let (id, parked) = env.ask("comment", &comment("pulls")).await;
    assert!(parked);
    let view = env.core.approval_view(id.clone()).await.unwrap();
    assert_eq!(view.class, "pulls");
    env.core.approve(id, choice(&[], grant(&["octo/cat"], &[]))).await.unwrap();
    let granted = env.core.grants().await.unwrap();
    assert!(granted[0].lines.contains(&"Only: Pull requests and reviews".to_owned()), "{:?}", granted[0].lines);

    let (_, parked) = env.ask("comment", &comment("pulls")).await;
    assert!(!parked, "covered: the permission is for the kind the preview named");
    let (_, parked) = env.ask("comment", &comment("issues")).await;
    assert!(parked, "not covered: the tool's own kind is not what counts");
    assert_eq!(*fake.done.lock().unwrap(), 2);
    // A preview that says "every time" is never remembered.
    let (id, _) = env.ask("comment", &comment("settings")).await;
    assert!(env.core.approval_view(id.clone()).await.unwrap().no_standing);
    assert!(env.core.approve(id, choice(&[], grant(&["octo/cat"], &["settings"]))).await.is_err());
}

#[tokio::test]
async fn reads_return_githubs_json_cut_when_large_and_files_as_links() {
    let env = env().await;
    Mock::given(method("GET"))
        .and(path("/repos/octo/cat/issues"))
        .and(query_param("state", "all"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!([{"number": 1, "title": "One"}]))
                .insert_header("link", format!("<{}/repos/octo/cat/issues?page=2>; rel=\"next\"", env.github.uri())),
        )
        .mount(&env.github)
        .await;
    let (id, parked) = env.ask("api_read", &json!({"path": "/repos/octo/cat/issues", "query": {"state": "all"}})).await;
    assert!(parked);
    let view = env.core.approval_view(id.clone()).await.unwrap();
    assert_eq!(view.resources[0].id, "octo/cat");
    assert_eq!(view.messages[0].subject, "GET /repos/octo/cat/issues");
    let ids: Vec<String> = view.messages.iter().map(|m| m.id.clone()).collect();
    env.core.approve(id, choice(&ids, None)).await.unwrap();
    let item = env.told().await["result"]["data"]["items"][0].clone();
    let shown: Value = serde_json::from_str(item["text"].as_str().unwrap()).unwrap();
    assert_eq!(shown, json!([{"number": 1, "title": "One"}]));
    assert_eq!((item["status"].as_u64(), item["truncated"].as_bool()), (Some(200), Some(false)));
    assert_eq!(item["next_path"], "/repos/octo/cat/issues?page=2");

    // A large answer is cut, with a note.
    let many: Vec<Value> = (0..3_000).map(|n| json!({"number": n, "title": "x".repeat(40)})).collect();
    Mock::given(method("GET"))
        .and(path("/repos/octo/cat/pulls"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!(many)))
        .mount(&env.github)
        .await;
    let (id, _) = env.ask("api_read", &json!({"path": "/repos/octo/cat/pulls"})).await;
    let ids: Vec<String> =
        env.core.approval_view(id.clone()).await.unwrap().messages.iter().map(|m| m.id.clone()).collect();
    env.core.approve(id, choice(&ids, None)).await.unwrap();
    let item = env.told().await["result"]["data"]["items"][0].clone();
    assert_eq!(item["truncated"], true);
    assert_eq!(item["text"].as_str().unwrap().chars().count(), 100_000);
    assert!(item["note"].as_str().unwrap().contains("cut"), "{}", item["note"]);

    // A download (GitHub redirects to it) is fetched by the server once released.
    Mock::given(method("GET"))
        .and(path("/repos/octo/cat/zipball/main"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", "https://codeload.example/x.zip"))
        .mount(&env.github)
        .await;
    let (id, _) = env.ask("api_read", &json!({"path": "/repos/octo/cat/zipball/main"})).await;
    let fetches = async || {
        env.server
            .received_requests()
            .await
            .unwrap()
            .into_iter()
            .filter(|r| r.url.path() == "/reins/api/blobs/fetch")
            .count()
    };
    assert_eq!(fetches().await, 0);
    let ids: Vec<String> =
        env.core.approval_view(id.clone()).await.unwrap().messages.iter().map(|m| m.id.clone()).collect();
    env.core.approve(id, choice(&ids, None)).await.unwrap();
    assert_eq!(fetches().await, 1);
    let item = env.told().await["result"]["data"]["items"][0].clone();
    assert_eq!(
        (item["download_url"].as_str(), item["encoding"].as_str()),
        (Some("https://rw.example/reins/blob/DL"), Some("link"))
    );
}

#[tokio::test]
async fn a_read_permission_for_a_repository_covers_api_reads_of_it_and_only_of_it() {
    let env = env().await;
    Mock::given(method("GET"))
        .and(path_regex(r"^/(repos|user).*$"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .mount(&env.github)
        .await;
    let (id, parked) = env.ask("api_read", &json!({"path": "/repos/octo/cat/actions/runs"})).await;
    assert!(parked);
    let view = env.core.approval_view(id.clone()).await.unwrap();
    let ids: Vec<String> = view.messages.iter().map(|m| m.id.clone()).collect();
    env.core.approve(id, choice(&ids, grant(&["octo/cat"], &[]))).await.unwrap();

    let (_, parked) = env.ask("api_read", &json!({"path": "/repos/octo/cat/issues/3"})).await;
    assert!(!parked, "covered by the read permission for octo/cat");
    assert_eq!(env.told().await["result"]["data"]["items"][0]["status"], 200);
    let (_, parked) = env.ask("api_read", &json!({"path": "/repos/octo/cat/branches/main"})).await;
    assert!(!parked, "a branch of it too");
    let (_, parked) = env.ask("api_read", &json!({"path": "/repos/octo/dog/issues"})).await;
    assert!(parked, "another repository is not");
    let (_, parked) = env.ask("api_read", &json!({"path": "/user/repos"})).await;
    assert!(parked, "the account is not");
    // Keys and addresses are asked for every time, whatever is allowed.
    let (id, parked) = env.ask("api_read", &json!({"path": "/user/emails"})).await;
    assert!(parked);
    let view = env.core.approval_view(id).await.unwrap();
    assert!(view.messages[0].sensitive && view.no_standing);
}
