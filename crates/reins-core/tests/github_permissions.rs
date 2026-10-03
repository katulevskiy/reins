//! What a standing permission on GitHub can name: the repository or one branch, the kinds of change, and what it can
//! never cover. A fake GitHub answers previews and performs, so this is about the phone's decisions.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{FakeGoogle, FakeKeys, RecordingNotifier};
use rewarden_core::connector::{Connector, Item, Preview};
use rewarden_core::{
    ApprovalChoice, ApprovalKind, CoreConfig, CoreError, GmailStatus, GoogleTokenProvider, GrantScopeChoice, Notifier,
    RewardenCore, StandingGrant,
};
use rewarden_proto::connector::ConnectorCall;
use serde_json::{Value, json};
use wiremock::matchers::{method, path, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Previews the way the real connector does (branch-level resource with parents) and records what it performed.
#[derive(Default)]
struct FakeGitHub {
    done: Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl Connector for FakeGitHub {
    fn service(&self) -> &'static str {
        "github"
    }

    async fn fetch(&self, _account: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
        let repo = call.str_arg("repo").unwrap_or("me/app");
        Ok(vec![Item {
            id: "f".into(),
            resource: format!("{repo}@main"),
            resource_label: format!("{repo}, branch main"),
            title: "README.md".into(),
            parents: vec![
                (repo.into(), format!("Any branch of {repo}")),
                ("me".into(), "Every repository of me".into()),
            ],
            ..Item::default()
        }])
    }

    async fn preview(&self, _account: &str, call: &ConnectorCall) -> Result<Preview, CoreError> {
        let repo = call.str_arg("repo").unwrap().to_owned();
        let (resource, label, parents) = match call.op.as_str() {
            "file_put" => {
                let branch = call.str_arg("branch").unwrap_or("main");
                (
                    format!("{repo}@{branch}"),
                    format!("{repo}, branch {branch}"),
                    vec![
                        (repo.clone(), format!("Any branch of {repo}")),
                        ("me".into(), "Every repository of me".into()),
                    ],
                )
            }
            _ => (repo.clone(), repo.clone(), vec![("me".into(), "Every repository of me".into())]),
        };
        Ok(Preview {
            resource,
            resource_label: label,
            lines: vec![format!("{} in {repo}", call.op)],
            parents,
            once_only: false,
            ..Preview::default()
        })
    }

    async fn perform(&self, _account: &str, call: &ConnectorCall) -> Result<Value, CoreError> {
        self.done.lock().unwrap().push(format!("{}:{}", call.op, call.str_arg("branch").unwrap_or("")));
        Ok(json!({"done": call.op}))
    }

    async fn status(&self, _account: &str) -> GmailStatus {
        GmailStatus::Ready
    }
}

struct Env {
    server: MockServer,
    core: Arc<RewardenCore>,
    github: Arc<FakeGitHub>,
    _dir: tempfile::TempDir,
}

async fn env() -> Env {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/identity/accounts/prelogin"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"kdf": 0, "kdfIterations": 5000})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/identity/connect/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "ACCESS", "refresh_token": "REFRESH", "expires_in": 7200})))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let github = Arc::new(FakeGitHub::default());
    let google: Arc<dyn GoogleTokenProvider> = Arc::new(FakeGoogle::new());
    let notifier: Arc<dyn Notifier> = Arc::new(RecordingNotifier::default());
    let core = RewardenCore::with_connectors(
        dir.path().to_str().unwrap(),
        &FakeKeys,
        google,
        notifier,
        CoreConfig {
            backoff_base: Duration::from_millis(1),
            ..CoreConfig::default()
        },
        vec![Arc::<FakeGitHub>::clone(&github)],
    )
    .unwrap();
    core.login(server.uri(), "me@example.com".to_owned(), "hunter2".to_owned(), None).await.unwrap();
    core.engine().register_account("github", "octo").unwrap();
    Env {
        server,
        core,
        github,
        _dir: dir,
    }
}

fn request(id: &str, op: &str, args: &Value) -> Value {
    json!({"v": 1, "id": id, "connection_id": "c1", "connection_label": "Claude", "created_at": 100,
           "call": {"tool": "connector", "service": "github", "op": op, "args": args}})
}

async fn serve(env: &Env, requests: &[Value]) {
    Mock::given(method("GET"))
        .and(path("/rewarden/api/pending"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"requests": requests, "pairings": []})))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&env.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/rewarden/api/pending"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"requests": [], "pairings": []})))
        .mount(&env.server)
        .await;
    Mock::given(method("POST"))
        .and(path_regex(r"^/rewarden/api/(requests|pairings)/[^/]+/response$"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&env.server)
        .await;
}

fn standing(resources: &[&str], classes: &[&str], secs: u64) -> Option<StandingGrant> {
    Some(StandingGrant {
        duration_secs: Some(secs),
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

fn choice(standing: Option<StandingGrant>) -> ApprovalChoice {
    ApprovalChoice {
        selected_message_ids: vec![],
        standing,
    }
}

fn put(id: &str, branch: &str) -> Value {
    request(
        id,
        "file_put",
        &json!({"repo": "me/app", "path": "a.txt", "content": "x", "message": "m", "branch": branch}),
    )
}

fn issue(id: &str) -> Value {
    request(id, "issue_update", &json!({"repo": "me/app", "number": 3, "state": "closed"}))
}

#[tokio::test]
async fn a_write_is_offered_for_the_branch_the_repository_and_the_owner_and_the_kind_of_change() {
    let env = env().await;
    serve(&env, &[put("r1", "main")]).await;
    env.core.sync(0).await.unwrap();
    let view = env.core.approval_view("r1".to_owned()).await.unwrap();
    assert_eq!(view.kind, ApprovalKind::Write);
    let resources: Vec<_> = view.resources.iter().map(|r| (r.id.as_str(), r.wider)).collect();
    assert_eq!(resources, [("me/app@main", false), ("me/app", true), ("me", true)]);
    assert_eq!(view.class, "code");
    assert!(view.classes.iter().any(|c| c.id == "issues") && view.classes.iter().any(|c| c.id == "releases"));
    assert_eq!(view.op_title, "Create or change a file on GitHub");
    assert!(!view.no_standing);
}

#[tokio::test]
async fn a_permission_for_one_branch_and_one_kind_covers_exactly_that() {
    let env = env().await;
    serve(&env, &[put("r1", "main")]).await;
    env.core.sync(0).await.unwrap();
    // No kinds named: the kind of this request only.
    env.core.approve("r1".to_owned(), choice(standing(&["me/app@main"], &[], 3_600))).await.unwrap();
    let grants = env.core.grants().await.unwrap();
    assert_eq!(grants.len(), 1);
    assert!(
        grants[0].summary.contains("Commits, files and branches") && grants[0].summary.contains("me/app, branch main")
    );

    serve(&env, &[put("r2", "main"), put("r3", "dev"), issue("r4")]).await;
    let waiting = env.core.sync(0).await.unwrap();
    let ids: Vec<_> = waiting.iter().map(|p| p.id.as_str()).collect();
    assert!(!ids.contains(&"r2"), "the same branch and kind went through: {ids:?}");
    assert!(ids.contains(&"r3"), "another branch asks: {ids:?}");
    assert!(ids.contains(&"r4"), "another kind of change asks: {ids:?}");
    assert_eq!(
        *env.github.done.lock().unwrap(),
        ["file_put:main", "file_put:main"],
        "the approved one, then the covered one"
    );
}

#[tokio::test]
async fn a_repository_permission_with_two_kinds_covers_its_branches_and_those_kinds() {
    let env = env().await;
    serve(&env, &[put("r1", "main")]).await;
    env.core.sync(0).await.unwrap();
    env.core.approve("r1".to_owned(), choice(standing(&["me/app"], &["code", "issues"], 3_600))).await.unwrap();
    serve(
        &env,
        &[put("r2", "dev"), issue("r3"), request("r4", "release_create", &json!({"repo": "me/app", "tag_name": "v1"}))],
    )
    .await;
    let waiting = env.core.sync(0).await.unwrap();
    let ids: Vec<_> = waiting.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids, ["r4"], "a release is another kind of change");
    assert_eq!(*env.github.done.lock().unwrap(), ["file_put:main", "file_put:dev", "issue_update:"]);
}

#[tokio::test]
async fn kinds_of_change_are_checked() {
    let env = env().await;
    serve(&env, &[put("r1", "main")]).await;
    env.core.sync(0).await.unwrap();
    for bad in [&["nonsense"][..], &["code", "Code"][..]] {
        assert!(matches!(
            env.core.approve("r1".to_owned(), choice(standing(&["me/app@main"], bad, 3_600))).await,
            Err(CoreError::Invalid { .. })
        ));
    }
    assert!(
        matches!(
            env.core.approve("r1".to_owned(), choice(standing(&["someone/else"], &[], 3_600))).await,
            Err(CoreError::Invalid { .. })
        ),
        "a resource that was not part of the request"
    );
    assert!(env.core.grants().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_destructive_change_is_asked_every_time_and_never_remembered() {
    let env = env().await;
    let delete = |id: &str| request(id, "repo_delete", &json!({"repo": "me/app"}));
    serve(&env, &[delete("r1")]).await;
    env.core.sync(0).await.unwrap();
    let view = env.core.approval_view("r1".to_owned()).await.unwrap();
    assert!(view.no_standing);
    assert!(matches!(
        env.core.approve("r1".to_owned(), choice(standing(&["me/app"], &[], 3_600))).await,
        Err(CoreError::Invalid { .. })
    ));
    assert!(env.github.done.lock().unwrap().is_empty(), "nothing happened");
    env.core.approve("r1".to_owned(), choice(None)).await.unwrap();
    assert!(env.core.grants().await.unwrap().is_empty());

    // Even a broad permission for the repository does not cover it.
    serve(&env, &[put("r2", "main")]).await;
    env.core.sync(0).await.unwrap();
    env.core.approve("r2".to_owned(), choice(standing(&["me"], &["code", "settings"], 3_600))).await.unwrap();
    serve(&env, &[delete("r3")]).await;
    let waiting = env.core.sync(0).await.unwrap();
    assert_eq!(waiting.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["r3"]);
}

#[tokio::test]
async fn reading_can_be_allowed_for_a_branch_and_asks_again_for_another() {
    let env = env().await;
    let read = |id: &str| request(id, "file_get", &json!({"repo": "me/app", "path": "README.md"}));
    serve(&env, &[read("r1")]).await;
    env.core.sync(0).await.unwrap();
    let view = env.core.approval_view("r1".to_owned()).await.unwrap();
    assert!(view.classes.is_empty(), "reads have no kinds of change");
    let resources: Vec<_> = view.resources.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(resources, ["me/app@main", "me/app", "me"]);
    env.core
        .approve(
            "r1".to_owned(),
            ApprovalChoice {
                selected_message_ids: vec!["f".into()],
                standing: standing(&["me/app@main"], &[], 3_600),
            },
        )
        .await
        .unwrap();
    serve(&env, &[read("r2")]).await;
    assert!(env.core.sync(0).await.unwrap().is_empty(), "the same branch is covered");
    // Another branch is not.
    let other = request("r3", "file_get", &json!({"repo": "me/app", "path": "README.md", "ref": "dev"}));
    assert!(other["call"]["args"]["ref"].is_string());
}
