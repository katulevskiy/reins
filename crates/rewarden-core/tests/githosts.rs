//! Git for GitLab, Codeberg and Bitbucket through the Rewarden desktop app: token sign-in checked against each host's
//! API, fetches and pushes answered with the token sealed to the app's key, with the user name each host expects for
//! git over HTTPS. Fake hosts answer only for the right credentials.

mod common;

use common::desktop::{DESK, Desk, REFUSED, call, choice, desk_with, mount_rewarden, standing};
use data_encoding::BASE64;
use rewarden_core::store::unix_now;
use rewarden_core::{ApprovalKind, CoreConfig, CoreError};
use rewarden_proto::desktop::{CredentialGrant, PUSH_LEASE_SECS, PushSummary, RefChange, RefUpdate, ZERO_OID};
use serde_json::{Value, json};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const GITLAB_TOKEN: &str = "glpat-SECRET-one";
const GITLAB_TOKEN2: &str = "glpat-SECRET-two";
const CODEBERG_TOKEN: &str = "cb-SECRET-token";
const BITBUCKET_EMAIL: &str = "ann@example.com";
const BITBUCKET_TOKEN: &str = "ATATT-SECRET-token";
const TOKENS: &[&str] = &[GITLAB_TOKEN, GITLAB_TOKEN2, CODEBERG_TOKEN, BITBUCKET_TOKEN];
const DIGEST: &str = "d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1";

struct Hosts {
    desk: Desk,
    gitlab: MockServer,
    // Kept for the whole test: wiremock pools its servers, and one dropped early goes back to the pool, where another
    // test running at the same time takes it and resets its mocks (the host then answers 404 to everything).
    _codeberg: MockServer,
    _bitbucket: MockServer,
}

fn basic() -> String {
    format!("Basic {}", BASE64.encode(format!("{BITBUCKET_EMAIL}:{BITBUCKET_TOKEN}").as_bytes()))
}

async fn answers(server: &MockServer, auth: &str, at: &str, body: Value) {
    Mock::given(method("GET"))
        .and(path(at))
        .and(header("authorization", auth))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .with_priority(1)
        .mount(server)
        .await;
}

async fn refuses_the_rest(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/user"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({"message": "401 Unauthorized"})))
        .with_priority(5)
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({"message": "404 Not Found"})))
        .with_priority(9)
        .mount(server)
        .await;
}

async fn hosts() -> Hosts {
    let server = MockServer::start().await;
    mount_rewarden(&server).await;
    let gitlab = MockServer::start().await;
    let codeberg = MockServer::start().await;
    let bitbucket = MockServer::start().await;
    let lab1 = format!("Bearer {GITLAB_TOKEN}");
    let lab2 = format!("Bearer {GITLAB_TOKEN2}");
    answers(&gitlab, &lab1, "/user", json!({"username": "Ann", "id": 1})).await;
    answers(&gitlab, &lab2, "/user", json!({"username": "work", "id": 2})).await;
    answers(&gitlab, &lab1, "/projects/grp%2Fsub%2Fapp", json!({"visibility": "private"})).await;
    answers(&gitlab, &lab2, "/projects/work%2Fapp", json!({"visibility": "public"})).await;
    refuses_the_rest(&gitlab).await;
    let berg = format!("token {CODEBERG_TOKEN}");
    answers(&codeberg, &berg, "/user", json!({"login": "Ann", "id": 7})).await;
    answers(&codeberg, &berg, "/repos/ann/app", json!({"private": false})).await;
    refuses_the_rest(&codeberg).await;
    answers(&bitbucket, &basic(), "/user", json!({"username": "ann-b", "account_id": "x"})).await;
    answers(&bitbucket, &basic(), "/repositories/ws/app", json!({"is_private": true})).await;
    refuses_the_rest(&bitbucket).await;
    let desk = desk_with(
        server,
        "me@example.com",
        "pw",
        CoreConfig {
            gitlab_base: gitlab.uri(),
            codeberg_base: codeberg.uri(),
            bitbucket_base: bitbucket.uri(),
            ..CoreConfig::default()
        },
    )
    .await;
    let core = &desk.core;
    assert_eq!(core.add_token_account("gitlab".into(), GITLAB_TOKEN.into()).await.unwrap().account, "ann");
    assert_eq!(core.add_token_account("codeberg".into(), format!(" {CODEBERG_TOKEN} ")).await.unwrap().account, "ann");
    let bucket = core.add_token_account("bitbucket".into(), format!("{BITBUCKET_EMAIL}:{BITBUCKET_TOKEN}")).await;
    assert_eq!(bucket.unwrap().account, "ann-b");
    Hosts {
        desk,
        gitlab,
        _codeberg: codeberg,
        _bitbucket: bitbucket,
    }
}

fn fetch(desk: &Desk, id: &str, service: &str, repo: &str) -> Value {
    call(
        id,
        DESK,
        service,
        "git_fetch",
        &json!({"repo": repo, "client_key": desk.public(), "nonce": format!("nonce-{id}")}),
    )
}

fn update(name: &str, old: &str, new: &str, fast_forward: Option<bool>) -> RefUpdate {
    let change = match (old == ZERO_OID, new == ZERO_OID) {
        (true, _) => RefChange::Create,
        (_, true) => RefChange::Delete,
        _ => RefChange::Update,
    };
    RefUpdate {
        name: name.to_owned(),
        change,
        old: old.to_owned(),
        new: new.to_owned(),
        fast_forward,
        commit_count: 1,
        commits: vec![],
        files_changed: 0,
        files: vec![],
        additions: None,
        deletions: None,
    }
}

fn summary(updates: Vec<RefUpdate>) -> Value {
    serde_json::to_value(PushSummary {
        updates,
        pack_bytes: 512,
        push_options: vec![],
        notes: vec![],
    })
    .unwrap()
}

fn push(desk: &Desk, id: &str, service: &str, op: &str, repo: &str, summary: &Value) -> Value {
    call(
        id,
        DESK,
        service,
        op,
        &json!({"repo": repo, "client_key": desk.public(), "nonce": format!("nonce-{id}"), "digest": DIGEST,
                "summary": summary}),
    )
}

fn main_ff() -> Value {
    summary(vec![update("refs/heads/main", &"a".repeat(40), &"b".repeat(40), Some(true))])
}

#[tokio::test]
async fn each_host_signs_in_with_a_token_its_api_accepts_and_is_offered_with_token_sign_in() {
    let hosts = hosts().await;
    let core = &hosts.desk.core;
    for (service, bad) in [
        ("gitlab", "glpat-wrong".to_owned()),
        ("codeberg", "wrong".to_owned()),
        ("bitbucket", format!("{BITBUCKET_EMAIL}:wrong")),
    ] {
        assert!(core.add_token_account(service.into(), bad).await.is_err(), "{service}");
    }
    assert!(matches!(
        core.add_token_account("bitbucket".into(), BITBUCKET_TOKEN.into()).await,
        Err(CoreError::Invalid { .. })
    ));
    let services = core.services().await.unwrap();
    for (service, name, account) in
        [("gitlab", "GitLab", "ann"), ("codeberg", "Codeberg", "ann"), ("bitbucket", "Bitbucket", "ann-b")]
    {
        let s = services.iter().find(|s| s.service == service).unwrap();
        assert_eq!((s.name.as_str(), s.kind.as_str(), s.available), (name, "token", true));
        assert_eq!(s.accounts.iter().map(|a| a.account.as_str()).collect::<Vec<_>>(), [account]);
        assert_eq!(
            core.service_account_status(service.into(), account.into()).await,
            rewarden_core::GmailStatus::Ready,
            "{service}"
        );
    }
}

#[tokio::test]
async fn only_the_paired_app_gets_credentials() {
    let hosts = hosts().await;
    let desk = &hosts.desk;
    desk.send(&[
        fetch(desk, "r1", "gitlab", "grp/sub/app"),
        push(desk, "r2", "codeberg", "git_push", "ann/app", &main_ff()),
    ])
    .await;
    assert_eq!(desk.error("r1").await, REFUSED);
    assert_eq!(desk.error("r2").await, REFUSED);
}

#[tokio::test]
async fn fetches_are_sealed_with_the_user_name_each_host_expects() {
    let hosts = hosts().await;
    let desk = &hosts.desk;
    desk.pair().await;
    let cases = [
        ("r1", "gitlab", "grp/sub/app", "oauth2", GITLAB_TOKEN, "grp/sub/app (private)"),
        ("r2", "codeberg", "ann/app", "Ann", CODEBERG_TOKEN, "ann/app"),
        ("r3", "bitbucket", "ws/app", "x-bitbucket-api-token-auth", BITBUCKET_TOKEN, "ws/app (private)"),
    ];
    let requests: Vec<Value> = cases.iter().map(|(id, service, repo, ..)| fetch(desk, id, service, repo)).collect();
    desk.send(&requests).await;
    assert_eq!(desk.waiting().await, ["r1", "r2", "r3"]);
    for (id, service, repo, username, token, label) in cases {
        let view = desk.core.approval_view(id.to_owned()).await.unwrap();
        assert_eq!((view.kind, view.op_title.as_str()), (ApprovalKind::Fetch, "Clone and fetch with git"));
        assert_eq!((view.resources[0].id.as_str(), view.resources[0].label.as_str()), (repo, label), "{service}");
        desk.core.approve(id.to_owned(), choice(&[repo], standing(&[repo], &[]))).await.unwrap();
        let grant: CredentialGrant = desk.open(&desk.data(id).await["items"][0]["sealed"]);
        assert_eq!(
            (grant.nonce.as_str(), grant.repo.as_str(), grant.access.as_str(), grant.username.as_str()),
            (format!("nonce-{id}").as_str(), repo, "read", username),
            "{service}"
        );
        assert_eq!(grant.token, token, "{service}");
    }
    // The read permission answers the next fetch at once.
    desk.send(&[fetch(desk, "r4", "gitlab", "grp/sub/app")]).await;
    assert!(desk.waiting().await.is_empty());
    let grant: CredentialGrant = desk.open(&desk.data("r4").await["items"][0]["sealed"]);
    assert_eq!(grant.username, "oauth2");
}

#[tokio::test]
async fn nested_gitlab_groups_can_each_be_given_a_permission() {
    let hosts = hosts().await;
    let desk = &hosts.desk;
    desk.pair().await;
    desk.send(&[push(desk, "r1", "gitlab", "git_push", "grp/sub/app", &main_ff())]).await;
    let view = desk.core.approval_view("r1".to_owned()).await.unwrap();
    let resources: Vec<_> = view.resources.iter().map(|r| (r.id.as_str(), r.wider)).collect();
    assert_eq!(resources, [("grp/sub/app@main", false), ("grp/sub/app", true), ("grp/sub", true), ("grp", true)]);
    desk.core.approve("r1".to_owned(), choice(&[], standing(&["grp"], &["code"]))).await.unwrap();
    desk.send(&[push(desk, "r2", "gitlab", "git_push", "grp/sub/app", &main_ff())]).await;
    assert!(desk.waiting().await.is_empty(), "the group's permission covers the push");
    let grant: CredentialGrant = desk.open(&desk.data("r2").await["sealed"]);
    assert_eq!((grant.access.as_str(), grant.digest.as_deref()), ("write", Some(DIGEST)));
}

#[tokio::test]
async fn pushes_are_shown_ref_by_ref_and_sealed_bound_to_the_digest() {
    let hosts = hosts().await;
    let desk = &hosts.desk;
    desk.pair().await;
    let tags = summary(vec![update("refs/tags/v1", ZERO_OID, &"c".repeat(40), None)]);
    let cases = [
        ("r1", "gitlab", "git_push", "grp/sub/app", main_ff(), "oauth2", GITLAB_TOKEN, "code"),
        ("r2", "codeberg", "git_tag_push", "ann/app", tags.clone(), "Ann", CODEBERG_TOKEN, "releases"),
        ("r3", "bitbucket", "git_push", "ws/app", main_ff(), "x-bitbucket-api-token-auth", BITBUCKET_TOKEN, "code"),
    ];
    let requests: Vec<Value> =
        cases.iter().map(|(id, service, op, repo, s, ..)| push(desk, id, service, op, repo, s)).collect();
    desk.send(&requests).await;
    assert_eq!(desk.waiting().await, ["r1", "r2", "r3"]);
    for (id, service, _, repo, _, username, token, class) in cases {
        let view = desk.core.approval_view(id.to_owned()).await.unwrap();
        assert_eq!((view.kind, view.class.as_str()), (ApprovalKind::Write, class), "{service}");
        let git = view.git.expect("a push shows its refs");
        assert_eq!((git.repo.as_str(), git.pack_bytes, git.refs.len()), (repo, 512, 1));
        let before = unix_now();
        desk.core.approve(id.to_owned(), choice(&[], None)).await.unwrap();
        let data = desk.data(id).await;
        assert_eq!(data.as_object().unwrap().len(), 1);
        let grant: CredentialGrant = desk.open(&data["sealed"]);
        assert_eq!(
            (grant.repo.as_str(), grant.access.as_str(), grant.digest.as_deref(), grant.username.as_str()),
            (repo, "write", Some(DIGEST), username),
            "{service}"
        );
        assert_eq!(grant.token, token);
        assert!((before + PUSH_LEASE_SECS..=unix_now() + PUSH_LEASE_SECS).contains(&grant.expires_at));
    }
    let codeberg = desk.answer("r2").await.unwrap();
    assert!(!codeberg.to_string().contains(CODEBERG_TOKEN), "the token only travels sealed");
}

#[tokio::test]
async fn wrong_tools_paths_and_unseen_repositories_are_refused() {
    let hosts = hosts().await;
    let desk = &hosts.desk;
    desk.pair().await;
    let tags = summary(vec![update("refs/tags/v1", ZERO_OID, &"c".repeat(40), None)]);
    desk.send(&[
        push(desk, "r1", "gitlab", "git_push", "grp/sub/app", &tags),
        fetch(desk, "r2", "codeberg", "ann/sub/app"),
        fetch(desk, "r3", "gitlab", "other/secret"),
        fetch(desk, "r4", "bitbucket", "../etc"),
    ])
    .await;
    assert!(desk.error("r1").await.contains("gitlab_git_tag_push"));
    assert!(desk.error("r2").await.contains("owner/name"));
    assert!(desk.error("r3").await.contains("GitLab says that does not exist"));
    desk.error("r4").await;
    assert!(desk.waiting().await.is_empty());
}

#[tokio::test]
async fn with_several_accounts_the_one_that_sees_the_repository_is_used() {
    let hosts = hosts().await;
    let desk = &hosts.desk;
    desk.core.add_token_account("gitlab".into(), GITLAB_TOKEN2.into()).await.unwrap();
    desk.pair().await;
    desk.send(&[fetch(desk, "r1", "gitlab", "work/app"), fetch(desk, "r2", "gitlab", "nobody/app")]).await;
    let pending = desk.core.pending().await.unwrap();
    assert_eq!(pending.iter().find(|p| p.id == "r1").and_then(|p| p.account.clone()).as_deref(), Some("work"));
    assert!(desk.error("r2").await.contains("None of the GitLab accounts"));
    desk.core.approve("r1".to_owned(), choice(&["work/app"], None)).await.unwrap();
    let grant: CredentialGrant = desk.open(&desk.data("r1").await["items"][0]["sealed"]);
    assert_eq!(grant.token, GITLAB_TOKEN2);
    assert!(hosts.gitlab.received_requests().await.unwrap().iter().any(|r| r.url.path() == "/projects/work%2Fapp"));
}

#[tokio::test]
async fn tokens_never_appear_in_views_activity_or_what_is_sent() {
    let hosts = hosts().await;
    let desk = &hosts.desk;
    desk.pair().await;
    desk.send(&[
        fetch(desk, "r1", "gitlab", "grp/sub/app"),
        push(desk, "r2", "codeberg", "git_push", "ann/app", &main_ff()),
        push(desk, "r3", "bitbucket", "git_push", "ws/app", &main_ff()),
    ])
    .await;
    let before = desk.everything_shown(&["r1", "r2", "r3"]).await;
    desk.core.approve("r1".to_owned(), choice(&["grp/sub/app"], None)).await.unwrap();
    desk.core.approve("r2".to_owned(), choice(&[], None)).await.unwrap();
    desk.core.deny("r3".to_owned()).await.unwrap();
    let shown = format!("{before} {}", desk.everything_shown(&[]).await);
    for token in TOKENS {
        assert!(!shown.contains(token), "{token} leaks");
    }
    assert!(shown.contains("sealed"));
}
