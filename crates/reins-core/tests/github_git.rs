//! Git through the Reins desktop app: the phone pins the app's key when it is paired, answers git fetches and
//! pushes only for that app, shows what a push does, and seals the GitHub token to the app's key. A fake Reins
//! server relays the calls and a fake GitHub answers for the repositories.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{FakeGoogle, FakeKeys, RecordingNotifier};
use crypto_box::SecretKey;
use crypto_box::aead::OsRng;
use data_encoding::BASE64URL_NOPAD;
use reins_core::store::unix_now;
use reins_core::{
    ApprovalChoice, ApprovalKind, CoreConfig, CoreError, GoogleTokenProvider, GrantScopeChoice, Notifier, ReinsCore,
    StandingGrant,
};
use reins_proto::desktop::{
    CommitInfo, CredentialGrant, FETCH_LEASE_SECS, FileChange, FileStatus, PUSH_LEASE_SECS, PushSummary, RefChange,
    RefUpdate, ZERO_OID, encode_key, key_fingerprint,
};
use serde_json::{Value, json};
use wiremock::matchers::{header, method, path, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TOKEN: &str = "ghp_gitSECRETone111";
const TOKEN2: &str = "ghp_gitSECRETtwo222";
const DESK: &str = "desk1";
const DIGEST: &str = "d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1";
const REFUSED: &str = "This must come from the Reins desktop app paired with this phone.";

fn oid(c: char) -> String {
    c.to_string().repeat(40)
}

struct Env {
    server: MockServer,
    github: MockServer,
    core: Arc<ReinsCore>,
    key: SecretKey,
    _dir: tempfile::TempDir,
}

impl Env {
    fn public(&self) -> String {
        encode_key(self.key.public_key().as_bytes())
    }
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
        .and(path_regex(r"^/reins/api/(requests|pairings)/[^/]+/response$"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    for (token, login) in [(TOKEN, "me"), (TOKEN2, "work")] {
        Mock::given(method("GET"))
            .and(path("/user"))
            .and(header("authorization", format!("Bearer {token}").as_str()))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"login": login})))
            .mount(&github)
            .await;
    }
    // `me` can see me/app (private); `work` can see work/app; nobody sees anything else.
    Mock::given(method("GET"))
        .and(path("/repos/me/app"))
        .and(header("authorization", format!("Bearer {TOKEN}").as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"full_name": "me/app", "private": true})))
        .with_priority(1)
        .mount(&github)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/work/app"))
        .and(header("authorization", format!("Bearer {TOKEN2}").as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"full_name": "work/app", "private": false})))
        .with_priority(1)
        .mount(&github)
        .await;
    Mock::given(path_regex("^/repos/"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({"message": "Not Found"})))
        .with_priority(5)
        .mount(&github)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let google: Arc<dyn GoogleTokenProvider> = Arc::new(FakeGoogle::new());
    let notifier: Arc<dyn Notifier> = Arc::new(RecordingNotifier::default());
    let core = ReinsCore::with_connectors(
        dir.path().to_str().unwrap(),
        &FakeKeys,
        google,
        notifier,
        CoreConfig {
            github_base: github.uri(),
            backoff_base: Duration::from_millis(1),
            ..CoreConfig::default()
        },
        vec![],
    )
    .unwrap();
    common::mount_account_vault(&server, "me@example.com", "hunter2").await;
    core.login(server.uri(), "me@example.com".to_owned(), "hunter2".to_owned(), None).await.unwrap();
    core.add_token_account("github".to_owned(), TOKEN.to_owned()).await.unwrap();
    Env {
        server,
        github,
        core,
        key: SecretKey::generate(&mut OsRng),
        _dir: dir,
    }
}

async fn serve(env: &Env, requests: &[Value], pairings: &[Value]) {
    Mock::given(method("GET"))
        .and(path("/reins/api/pending"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"requests": requests, "pairings": pairings})))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&env.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/reins/api/pending"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"requests": [], "pairings": []})))
        .mount(&env.server)
        .await;
}

/// The desktop app asks to be paired with `key`; the user approves (the server answers with `connection`) or denies.
async fn pair(env: &Env, id: &str, key: Option<&str>, connection: Option<&str>, approve: bool) {
    let mut pairing = json!({"v": 1, "id": id, "client_name": "Reins desktop app", "client_host": "127.0.0.1",
        "choices": [12, 47, 83], "created_at": 50});
    if let Some(key) = key {
        pairing["client_key"] = json!(key);
    }
    serve(env, &[], &[pairing]).await;
    env.core.sync(0).await.unwrap();
    Mock::given(method("POST"))
        .and(path(format!("/reins/api/pairings/{id}/response")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"connection_id": connection})))
        .with_priority(1)
        .mount(&env.server)
        .await;
    let code = approve.then_some(47);
    env.core.answer_pairing(id.to_owned(), approve, code, None).await.unwrap();
}

fn call(id: &str, connection: &str, op: &str, args: &Value) -> Value {
    json!({"v": 1, "id": id, "connection_id": connection, "connection_label": "Reins desktop app",
           "created_at": 100, "call": {"tool": "connector", "service": "github", "op": op, "args": args}})
}

fn fetch(id: &str, connection: &str, key: &str, repo: &str) -> Value {
    call(id, connection, "git_fetch", &json!({"repo": repo, "client_key": key, "nonce": format!("nonce-{id}")}))
}

fn push(id: &str, key: &str, op: &str, summary: &Value) -> Value {
    call(
        id,
        DESK,
        op,
        &json!({"repo": "me/app", "client_key": key, "nonce": format!("nonce-{id}"), "digest": DIGEST,
                "summary": summary}),
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
        commit_count: 0,
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
        pack_bytes: 2048,
        push_options: vec![],
        notes: vec![],
    })
    .unwrap()
}

/// A fast-forward of main by three commits touching two files.
fn fast_forward() -> Value {
    let mut u = update("refs/heads/main", &oid('a'), &oid('b'), Some(true));
    u.commit_count = 3;
    u.commits = ["Fix the parser", "Add tests", "Bump version"]
        .iter()
        .enumerate()
        .map(|(i, s)| CommitInfo {
            sha: oid(char::from(b'1' + u8::try_from(i).unwrap())),
            subject: (*s).to_owned(),
            author: "Ann <ann@example.com>".to_owned(),
            date: 1_700_000_000,
        })
        .collect();
    u.files_changed = 2;
    u.files = vec![
        FileChange {
            path: "src/parse.rs".to_owned(),
            status: FileStatus::Modified,
            additions: Some(10),
            deletions: Some(2),
            binary: false,
        },
        FileChange {
            path: "logo.png".to_owned(),
            status: FileStatus::Added,
            additions: None,
            deletions: None,
            binary: true,
        },
    ];
    u.additions = Some(10);
    u.deletions = Some(2);
    let mut s: PushSummary = serde_json::from_value(summary(vec![u])).unwrap();
    s.notes = vec!["File list unavailable: one object was too large".to_owned()];
    serde_json::to_value(s).unwrap()
}

/// What the phone answered, by request id.
async fn answer(env: &Env, id: &str) -> Option<Value> {
    env.server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .rev()
        .find(|r| r.method.as_str() == "POST" && r.url.path() == format!("/reins/api/requests/{id}/response"))
        .map(|r| serde_json::from_slice(&r.body).unwrap())
}

async fn waiting(env: &Env) -> Vec<String> {
    env.core.pending().await.unwrap().into_iter().map(|p| p.id).collect()
}

fn open(key: &SecretKey, sealed: &Value) -> CredentialGrant {
    let bytes = BASE64URL_NOPAD.decode(sealed.as_str().unwrap().as_bytes()).unwrap();
    serde_json::from_slice(&key.unseal(&bytes).unwrap()).unwrap()
}

fn standing(resources: &[&str], classes: &[&str]) -> Option<StandingGrant> {
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

fn choice(selected: &[&str], standing: Option<StandingGrant>) -> ApprovalChoice {
    ApprovalChoice {
        selected_message_ids: selected.iter().map(|s| (*s).to_owned()).collect(),
        standing,
    }
}

async fn repo_lookups(env: &Env) -> usize {
    env.github.received_requests().await.unwrap().iter().filter(|r| r.url.path().starts_with("/repos/")).count()
}

fn assert_refused(answer: Option<Value>) {
    let answer = answer.expect("answered");
    assert_eq!(answer["outcome"], "error", "{answer}");
    assert_eq!(answer["message"], REFUSED);
}

// ---- pairing --------------------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_desktop_pairing_shows_the_key_fingerprint_and_approving_it_pins_the_key() {
    let env = env().await;
    let key = env.public();
    let pairing = json!({"v": 1, "id": "p1", "client_name": "Reins desktop app", "client_host": "127.0.0.1",
        "choices": [12, 47, 83], "created_at": 50, "client_key": key});
    serve(&env, &[], &[pairing]).await;
    let items = env.core.sync(0).await.unwrap();
    assert_eq!(items[0].title, "Connect Reins desktop app to Reins?", "the title stays generic");
    let view = env.core.pairing_view("p1".to_owned()).await.unwrap();
    assert_eq!(view.key_fingerprint, key_fingerprint(&key));
    assert!(view.key_fingerprint.is_some());

    Mock::given(method("POST"))
        .and(path("/reins/api/pairings/p1/response"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"connection_id": DESK})))
        .with_priority(1)
        .mount(&env.server)
        .await;
    env.core.answer_pairing("p1".to_owned(), true, Some(47), None).await.unwrap();

    serve(&env, &[fetch("r1", DESK, &key, "me/app")], &[]).await;
    env.core.sync(0).await.unwrap();
    assert_eq!(waiting(&env).await, ["r1"], "the pinned app may ask");
}

#[tokio::test]
async fn a_denied_pairing_or_an_unusable_key_pins_nothing() {
    let env = env().await;
    let key = env.public();
    pair(&env, "p1", Some(&key), None, false).await;
    // A key that is not 32 bytes of base64url: no fingerprint, nothing pinned.
    let pairing = json!({"v": 1, "id": "p2", "client_name": "X", "client_host": "127.0.0.1",
        "choices": [12, 47, 83], "created_at": 50, "client_key": "not-a-key"});
    serve(&env, &[], &[pairing]).await;
    env.core.sync(0).await.unwrap();
    assert_eq!(env.core.pairing_view("p2".to_owned()).await.unwrap().key_fingerprint, None);
    Mock::given(method("POST"))
        .and(path("/reins/api/pairings/p2/response"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"connection_id": "desk2"})))
        .with_priority(1)
        .mount(&env.server)
        .await;
    env.core.answer_pairing("p2".to_owned(), true, Some(47), None).await.unwrap();
    // An ordinary AI pairing shows no fingerprint either.
    let plain = json!({"v": 1, "id": "p3", "client_name": "Claude", "client_host": "claude.ai",
        "choices": [12, 47, 83], "created_at": 50});
    serve(&env, &[], &[plain]).await;
    env.core.sync(0).await.unwrap();
    assert_eq!(env.core.pairing_view("p3".to_owned()).await.unwrap().key_fingerprint, None);

    serve(&env, &[fetch("r1", DESK, &key, "me/app"), fetch("r2", "desk2", "not-a-key", "me/app")], &[]).await;
    env.core.sync(0).await.unwrap();
    assert_refused(answer(&env, "r1").await);
    assert_refused(answer(&env, "r2").await);
    assert_eq!(repo_lookups(&env).await, 0, "GitHub is never asked");
}

#[tokio::test]
async fn only_the_pinned_key_of_the_same_connection_is_answered() {
    let env = env().await;
    let key = env.public();
    let other = encode_key(SecretKey::generate(&mut OsRng).public_key().as_bytes());
    // Nothing pinned yet.
    serve(&env, &[fetch("r0", DESK, &key, "me/app")], &[]).await;
    env.core.sync(0).await.unwrap();
    assert_refused(answer(&env, "r0").await);

    pair(&env, "p1", Some(&key), Some(DESK), true).await;
    serve(
        &env,
        &[
            fetch("r1", DESK, &other, "me/app"),
            fetch("r2", "claude", &key, "me/app"),
            push("r3", &other, "git_push", &fast_forward()),
        ],
        &[],
    )
    .await;
    env.core.sync(0).await.unwrap();
    for id in ["r1", "r2", "r3"] {
        assert_refused(answer(&env, id).await);
    }
    assert_eq!(waiting(&env).await.len(), 0);
    assert_eq!(repo_lookups(&env).await, 0);
    let activity = env.core.activity(10).await.unwrap();
    assert!(activity.iter().filter(|a| a.outcome == "error").count() >= 3);
}

#[tokio::test]
async fn revoking_drops_the_key_and_returning_to_its_owner_restores_the_pin() {
    let env = env().await;
    let key = env.public();
    pair(&env, "p1", Some(&key), Some(DESK), true).await;
    Mock::given(method("DELETE"))
        .and(path(format!("/reins/api/connections/{DESK}")))
        .respond_with(ResponseTemplate::new(204))
        .mount(&env.server)
        .await;
    env.core.revoke_connection(DESK.to_owned()).await.unwrap();
    serve(&env, &[fetch("r1", DESK, &key, "me/app")], &[]).await;
    env.core.sync(0).await.unwrap();
    assert_refused(answer(&env, "r1").await);

    pair(&env, "p2", Some(&key), Some(DESK), true).await;
    env.core.logout().await.unwrap();
    env.core.login(env.server.uri(), "me@example.com".to_owned(), "hunter2".to_owned(), None).await.unwrap();
    serve(&env, &[fetch("r2", DESK, &key, "me/app")], &[]).await;
    env.core.sync(0).await.unwrap();
    assert!(answer(&env, "r2").await.is_none(), "the owner keeps its pinned computer; sharing still needs approval");
    assert!(env.core.pending().await.unwrap().iter().any(|p| p.id == "r2"));
}

// ---- fetch ----------------------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_fetch_is_parked_then_answered_with_a_sealed_read_credential_and_a_read_grant_covers_the_next() {
    let env = env().await;
    let key = env.public();
    pair(&env, "p1", Some(&key), Some(DESK), true).await;
    serve(&env, &[fetch("r1", DESK, &key, "me/app")], &[]).await;
    env.core.sync(0).await.unwrap();
    let item = env.core.pending().await.unwrap().into_iter().find(|p| p.id == "r1").unwrap();
    assert_eq!((item.op_title.as_str(), item.account.as_deref()), ("Clone and fetch with git", Some("me")));

    let view = env.core.approval_view("r1".to_owned()).await.unwrap();
    assert_eq!(view.kind, ApprovalKind::Fetch);
    assert!(view.git.is_none(), "a fetch shows the repository as its item");
    assert_eq!(view.messages.len(), 1);
    assert_eq!(view.messages[0].id, "me/app");
    assert_eq!(view.messages[0].subject, "Clone and fetch me/app");
    assert_eq!(view.messages[0].snippet, "Git on your computer can read this repository for 1 hour.");
    assert!(!view.messages[0].sensitive);
    let resources: Vec<_> = view.resources.iter().map(|r| (r.id.as_str(), r.label.as_str(), r.wider)).collect();
    assert_eq!(resources, [("me/app", "me/app (private)", false), ("me", "Every repository of me", true)]);

    let before = unix_now();
    env.core.approve("r1".to_owned(), choice(&["me/app"], standing(&["me/app"], &[]))).await.unwrap();
    let answered = answer(&env, "r1").await.unwrap();
    assert_eq!(answered["outcome"], "result", "{answered}");
    let data = &answered["result"]["data"];
    let first = &data["items"][0];
    let grant = open(&env.key, &first["sealed"]);
    assert_eq!(
        (grant.v, grant.nonce.as_str(), grant.repo.as_str(), grant.access.as_str(), grant.username.as_str()),
        (1, "nonce-r1", "me/app", "read", "x-access-token")
    );
    assert_eq!(grant.token, TOKEN);
    assert_eq!(grant.digest, None);
    // A second of slack: the lease can be stamped in the second before `before` was read.
    assert!((before - 1 + FETCH_LEASE_SECS..=unix_now() + FETCH_LEASE_SECS).contains(&grant.expires_at));
    assert_eq!(first["expires_at"], grant.expires_at);
    assert!(!answered.to_string().contains(TOKEN), "the token only travels sealed");

    // The read permission on the repository answers the next fetch at once.
    serve(&env, &[fetch("r2", DESK, &key, "me/app")], &[]).await;
    env.core.sync(0).await.unwrap();
    assert_eq!(waiting(&env).await.len(), 0);
    let next = answer(&env, "r2").await.unwrap();
    let grant = open(&env.key, &next["result"]["data"]["items"][0]["sealed"]);
    assert_eq!((grant.nonce.as_str(), grant.access.as_str()), ("nonce-r2", "read"));
}

#[tokio::test]
async fn a_fetch_of_a_repository_the_token_cannot_see_fails() {
    let env = env().await;
    let key = env.public();
    pair(&env, "p1", Some(&key), Some(DESK), true).await;
    serve(&env, &[fetch("r1", DESK, &key, "other/secret")], &[]).await;
    env.core.sync(0).await.unwrap();
    let answered = answer(&env, "r1").await.unwrap();
    assert_eq!(answered["outcome"], "error");
    assert!(answered["message"].as_str().unwrap().contains("does not exist"), "{answered}");
    assert_eq!(waiting(&env).await.len(), 0);
}

#[tokio::test]
async fn with_several_accounts_the_named_one_or_the_first_that_sees_the_repository_is_used() {
    let env = env().await;
    let key = env.public();
    env.core.add_token_account("github".to_owned(), TOKEN2.to_owned()).await.unwrap();
    pair(&env, "p1", Some(&key), Some(DESK), true).await;
    let mut named = fetch("r2", DESK, &key, "me/app");
    named["account"] = json!("me");
    serve(&env, &[fetch("r1", DESK, &key, "work/app"), named, fetch("r3", DESK, &key, "nobody/app")], &[]).await;
    env.core.sync(0).await.unwrap();
    let pending = env.core.pending().await.unwrap();
    let account = |id: &str| pending.iter().find(|p| p.id == id).and_then(|p| p.account.clone());
    assert_eq!(account("r1").as_deref(), Some("work"));
    assert_eq!(account("r2").as_deref(), Some("me"));
    let none = answer(&env, "r3").await.unwrap();
    assert_eq!(none["outcome"], "error");
    assert!(none["message"].as_str().unwrap().contains("nobody/app"), "{none}");

    env.core.approve("r1".to_owned(), choice(&["work/app"], None)).await.unwrap();
    let grant = open(&env.key, &answer(&env, "r1").await.unwrap()["result"]["data"]["items"][0]["sealed"]);
    assert_eq!((grant.token.as_str(), grant.repo.as_str()), (TOKEN2, "work/app"));
}

// ---- push -----------------------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_push_is_parked_with_what_it_does_and_answered_with_a_write_credential_bound_to_the_digest() {
    let env = env().await;
    let key = env.public();
    pair(&env, "p1", Some(&key), Some(DESK), true).await;
    serve(&env, &[push("r1", &key, "git_push", &fast_forward())], &[]).await;
    env.core.sync(0).await.unwrap();
    assert_eq!(waiting(&env).await, ["r1"]);

    let view = env.core.approval_view("r1".to_owned()).await.unwrap();
    assert_eq!(view.kind, ApprovalKind::Write);
    assert_eq!((view.op_title.as_str(), view.class.as_str()), ("Push with git", "code"));
    assert!(!view.no_standing);
    assert_eq!(
        view.preview,
        ["Push 3 commits to main", "Fix the parser", "Add tests", "Bump version", "+10 \u{2212}2 in 2 files"]
    );
    let resources: Vec<_> = view.resources.iter().map(|r| (r.id.as_str(), r.label.as_str(), r.wider)).collect();
    assert_eq!(
        resources,
        [
            ("me/app@main", "me/app, branch main", false),
            ("me/app", "Any branch of me/app", true),
            ("me", "Every repository of me", true)
        ]
    );
    let git = view.git.expect("a push shows its refs");
    assert_eq!((git.repo.as_str(), git.pack_bytes), ("me/app", 2048));
    assert_eq!(git.notes, ["File list unavailable: one object was too large"]);
    assert_eq!(git.refs.len(), 1);
    let r = &git.refs[0];
    assert_eq!(
        (r.name.as_str(), r.kind.as_str(), r.short_name.as_str(), r.change.as_str()),
        ("refs/heads/main", "branch", "main", "update")
    );
    assert_eq!((r.force, r.force_unknown, r.commit_count, r.files_changed), (false, false, 3, 2));
    assert_eq!((r.additions, r.deletions), (Some(10), Some(2)));
    assert_eq!(
        (r.commits[0].short_sha.as_str(), r.commits[0].subject.as_str(), r.commits[0].author.as_str()),
        ("1111111", "Fix the parser", "Ann <ann@example.com>")
    );
    let files: Vec<_> =
        r.files.iter().map(|f| (f.path.as_str(), f.status.as_str(), f.additions, f.deletions, f.binary)).collect();
    assert_eq!(
        files,
        [("src/parse.rs", "modified", Some(10), Some(2), false), ("logo.png", "added", None, None, true)]
    );

    let before = unix_now();
    env.core.approve("r1".to_owned(), choice(&[], None)).await.unwrap();
    let answered = answer(&env, "r1").await.unwrap();
    let data = &answered["result"]["data"];
    assert_eq!(data.as_object().unwrap().len(), 1, "only the sealed credential: {data}");
    let grant = open(&env.key, &data["sealed"]);
    assert_eq!(
        (grant.nonce.as_str(), grant.repo.as_str(), grant.access.as_str(), grant.digest.as_deref()),
        ("nonce-r1", "me/app", "write", Some(DIGEST))
    );
    assert_eq!((grant.username.as_str(), grant.token.as_str()), ("x-access-token", TOKEN));
    assert!((before + PUSH_LEASE_SECS..=unix_now() + PUSH_LEASE_SECS).contains(&grant.expires_at));
    assert!(!answered.to_string().contains(TOKEN));
}

#[tokio::test]
async fn a_permission_for_the_branch_covers_the_next_fast_forward_but_never_a_force_push() {
    let env = env().await;
    let key = env.public();
    pair(&env, "p1", Some(&key), Some(DESK), true).await;
    serve(&env, &[push("r1", &key, "git_push", &fast_forward())], &[]).await;
    env.core.sync(0).await.unwrap();
    env.core.approve("r1".to_owned(), choice(&[], standing(&["me/app@main"], &[]))).await.unwrap();

    let force = summary(vec![update("refs/heads/main", &oid('b'), &oid('c'), Some(false))]);
    let other = summary(vec![update("refs/heads/dev", &oid('b'), &oid('c'), Some(true))]);
    serve(
        &env,
        &[
            push("r2", &key, "git_push", &fast_forward()),
            push("r3", &key, "git_push", &force),
            push("r4", &key, "git_push", &other),
        ],
        &[],
    )
    .await;
    env.core.sync(0).await.unwrap();
    assert_eq!(waiting(&env).await, ["r3", "r4"], "the same branch went through");
    let grant = open(&env.key, &answer(&env, "r2").await.unwrap()["result"]["data"]["sealed"]);
    assert_eq!((grant.nonce.as_str(), grant.access.as_str()), ("nonce-r2", "write"));

    let view = env.core.approval_view("r3".to_owned()).await.unwrap();
    assert!(view.no_standing);
    assert_eq!(view.preview[0], "Force push to main: rewrites history");
    let r = &view.git.unwrap().refs[0];
    assert!(r.force && !r.force_unknown);
    assert!(matches!(
        env.core.approve("r3".to_owned(), choice(&[], standing(&["me/app@main"], &[]))).await,
        Err(CoreError::Invalid { .. })
    ));
    env.core.approve("r3".to_owned(), choice(&[], None)).await.unwrap();
    assert_eq!(answer(&env, "r3").await.unwrap()["outcome"], "result");
}

#[tokio::test]
async fn force_pushes_deletions_moved_tags_and_unchecked_history_are_asked_every_time() {
    let env = env().await;
    let key = env.public();
    pair(&env, "p1", Some(&key), Some(DESK), true).await;
    // A wide permission: every repository of me, for code and releases.
    serve(&env, &[push("r0", &key, "git_push", &fast_forward())], &[]).await;
    env.core.sync(0).await.unwrap();
    env.core.approve("r0".to_owned(), choice(&[], standing(&["me"], &["code", "releases"]))).await.unwrap();

    let cases = [
        (
            "r1",
            "git_push",
            summary(vec![update("refs/heads/main", &oid('a'), &oid('b'), Some(false))]),
            "Force push to main: rewrites history",
        ),
        (
            "r2",
            "git_push",
            summary(vec![update("refs/heads/main", &oid('a'), &oid('b'), None)]),
            "Force push to main: may rewrite history",
        ),
        ("r3", "git_push", summary(vec![update("refs/heads/old", &oid('a'), ZERO_OID, None)]), "Delete branch old"),
        (
            "r4",
            "git_tag_push",
            summary(vec![update("refs/tags/v1", &oid('a'), &oid('b'), Some(true))]),
            "Move tag v1 \u{2192} bbbbbbb",
        ),
        (
            "r5",
            "git_push",
            summary(vec![
                update("refs/heads/a", ZERO_OID, &oid('b'), None),
                update("refs/heads/b", ZERO_OID, &oid('b'), None),
            ]),
            "Create branch a with 0 commits",
        ),
    ];
    let requests: Vec<Value> = cases.iter().map(|(id, op, s, _)| push(id, &key, op, s)).collect();
    serve(&env, &requests, &[]).await;
    env.core.sync(0).await.unwrap();
    assert_eq!(waiting(&env).await, ["r1", "r2", "r3", "r4", "r5"]);
    for (id, _, _, line) in cases {
        let view = env.core.approval_view(id.to_owned()).await.unwrap();
        assert!(view.no_standing, "{id}");
        assert_eq!(view.preview[0], line, "{id}");
    }
    let unknown = env.core.approval_view("r2".to_owned()).await.unwrap().git.unwrap();
    assert!(unknown.refs[0].force && unknown.refs[0].force_unknown);
    let deleted = env.core.approval_view("r3".to_owned()).await.unwrap().git.unwrap();
    assert_eq!((deleted.refs[0].change.as_str(), deleted.refs[0].force), ("delete", false));
}

#[tokio::test]
async fn tag_pushes_and_pushes_must_match_the_refs_they_change() {
    let env = env().await;
    let key = env.public();
    pair(&env, "p1", Some(&key), Some(DESK), true).await;
    let tags = summary(vec![update("refs/tags/v1.2", ZERO_OID, &oid('1'), None)]);
    let branch = summary(vec![update("refs/heads/main", &oid('a'), &oid('b'), Some(true))]);
    let notes = summary(vec![update("refs/notes/commits", &oid('a'), &oid('b'), Some(true))]);
    let mixed = summary(vec![
        update("refs/heads/main", &oid('a'), &oid('b'), Some(true)),
        update("refs/tags/v2", ZERO_OID, &oid('b'), None),
    ]);
    serve(
        &env,
        &[
            push("r1", &key, "git_push", &tags),
            push("r2", &key, "git_tag_push", &branch),
            push("r3", &key, "git_push", &notes),
            push("r4", &key, "git_tag_push", &tags),
            push("r5", &key, "git_push", &mixed),
        ],
        &[],
    )
    .await;
    env.core.sync(0).await.unwrap();
    for id in ["r1", "r2", "r3"] {
        let a = answer(&env, id).await.unwrap();
        assert_eq!(a["outcome"], "error", "{id}: {a}");
    }
    assert_eq!(waiting(&env).await, ["r4", "r5"]);
    let tag = env.core.approval_view("r4".to_owned()).await.unwrap();
    assert_eq!((tag.class.as_str(), tag.op_title.as_str()), ("releases", "Push tags with git"));
    assert_eq!(tag.preview, ["Tag v1.2 \u{2192} 1111111"]);
    let resources: Vec<_> = tag.resources.iter().map(|r| (r.id.as_str(), r.wider)).collect();
    assert_eq!(resources, [("me/app", false), ("me", true)]);
    let git = tag.git.unwrap();
    assert_eq!(
        (git.refs[0].kind.as_str(), git.refs[0].short_name.as_str(), git.refs[0].change.as_str()),
        ("tag", "v1.2", "create")
    );
    let both = env.core.approval_view("r5".to_owned()).await.unwrap();
    assert_eq!(both.resources[0].id, "me/app", "a branch and a tag: the whole repository");
    assert_eq!(both.preview, ["Push 0 commits to main", "Tag v2 \u{2192} bbbbbbb"]);
}

#[tokio::test]
async fn an_invalid_summary_or_digest_is_refused() {
    let env = env().await;
    let key = env.public();
    pair(&env, "p1", Some(&key), Some(DESK), true).await;
    let mut lying = update("refs/heads/main", &oid('a'), &oid('b'), Some(true));
    lying.change = RefChange::Create;
    let mut bad_digest = push("r4", &key, "git_push", &fast_forward());
    bad_digest["call"]["args"]["digest"] = json!("not-hex");
    serve(
        &env,
        &[
            push("r1", &key, "git_push", &summary(vec![])),
            push("r2", &key, "git_push", &summary(vec![lying])),
            push("r3", &key, "git_push", &json!({"updates": "nope"})),
            bad_digest,
            push("r5", &key, "git_push", &json!(fast_forward().to_string())),
        ],
        &[],
    )
    .await;
    env.core.sync(0).await.unwrap();
    for id in ["r1", "r2", "r3", "r4"] {
        let a = answer(&env, id).await.unwrap();
        assert_eq!(a["outcome"], "error", "{id}: {a}");
    }
    assert_eq!(waiting(&env).await, ["r5"], "a summary sent as a JSON string is read too");
}

#[tokio::test]
async fn a_key_replaced_while_a_push_waits_cannot_receive_it() {
    let env = env().await;
    let key = env.public();
    pair(&env, "p1", Some(&key), Some(DESK), true).await;
    serve(&env, &[push("r1", &key, "git_push", &fast_forward())], &[]).await;
    env.core.sync(0).await.unwrap();
    let new_key = encode_key(SecretKey::generate(&mut OsRng).public_key().as_bytes());
    pair(&env, "p2", Some(&new_key), Some(DESK), true).await;
    assert!(env.core.approve("r1".to_owned(), choice(&[], None)).await.is_err());
    assert!(answer(&env, "r1").await.is_none_or(|a| a["outcome"] != "result"));
}

#[tokio::test]
async fn the_activity_and_the_views_never_hold_the_token() {
    let env = env().await;
    let key = env.public();
    pair(&env, "p1", Some(&key), Some(DESK), true).await;
    serve(&env, &[fetch("r1", DESK, &key, "me/app"), push("r2", &key, "git_push", &fast_forward())], &[]).await;
    env.core.sync(0).await.unwrap();
    let views = format!(
        "{:?} {:?} {:?}",
        env.core.pending().await.unwrap(),
        env.core.approval_view("r1".to_owned()).await.unwrap(),
        env.core.approval_view("r2".to_owned()).await.unwrap()
    );
    assert!(!views.contains(TOKEN), "{views}");
    env.core.approve("r1".to_owned(), choice(&["me/app"], standing(&["me/app"], &[]))).await.unwrap();
    env.core.approve("r2".to_owned(), choice(&[], standing(&["me/app@main"], &[]))).await.unwrap();
    serve(&env, &[fetch("r3", DESK, &key, "me/app"), push("r4", &key, "git_push", &fast_forward())], &[]).await;
    env.core.sync(0).await.unwrap();
    let activity = env.core.activity(50).await.unwrap();
    assert!(activity.len() >= 4);
    let logged = format!("{activity:?} {:?}", env.core.grants().await.unwrap());
    assert!(!logged.contains(TOKEN), "{logged}");
    let entry = activity.iter().find(|a| a.op == "git_push").unwrap();
    assert_eq!(entry.op_title, "Push with git");
}

#[tokio::test]
async fn a_fast_forward_is_one_tap_and_a_force_push_never_is() {
    let env = env().await;
    let key = env.public();
    pair(&env, "p1", Some(&key), Some(DESK), true).await;
    let force = summary(vec![update("refs/heads/main", &oid('b'), &oid('c'), Some(false))]);
    serve(&env, &[push("r1", &key, "git_push", &fast_forward()), push("r2", &key, "git_push", &force)], &[]).await;
    env.core.sync(0).await.unwrap();
    let items = env.core.pending().await.unwrap();
    let item = |id: &str| items.iter().find(|i| i.id == id).unwrap().clone();
    assert!(item("r1").quick && !item("r2").quick);
    assert_eq!(item("r1").headline, "Pushes 3 commits to main in me/app.");
    assert_eq!(item("r2").headline, "Rewrites the history of main in me/app.");

    let quick = env.core.approval_view("r1".to_owned()).await.unwrap().quick.unwrap();
    assert_eq!(quick.allow_what, "push with git: me/app, branch main");
    assert_eq!(quick.allow.as_ref().unwrap().scope.resources, ["me/app@main"], "the branch, not the repository");
    assert!(env.core.approval_view("r2".to_owned()).await.unwrap().quick.is_none());
    assert!(matches!(env.core.approve_quick("r2".to_owned()).await, Err(CoreError::Invalid { .. })));
    assert!(answer(&env, "r2").await.is_none(), "a force push is not approved without being opened");

    env.core.approve_quick("r1".to_owned()).await.unwrap();
    let grant = open(&env.key, &answer(&env, "r1").await.unwrap()["result"]["data"]["sealed"]);
    assert_eq!((grant.nonce.as_str(), grant.digest.as_deref()), ("nonce-r1", Some(DIGEST)));
    assert_eq!(env.core.grants().await.unwrap().len(), 0);
}
