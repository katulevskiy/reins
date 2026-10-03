//! Review Focus: secrets never reach log output, `Debug` output or error text.
//! One test per binary: it installs a process-wide capturing logger.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{FakeGoogle, FakeKeys, RecordingNotifier};
use rewarden_core::http::{ServerUrl, client};
use rewarden_core::vault::VaultClient;
use rewarden_core::{ApprovalChoice, CoreConfig, RewardenCore};
use serde_json::{Value, json};
use wiremock::matchers::{body_string_contains, method, path, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};
use zeroize::Zeroizing;

const PASSWORD: &str = "hunter2-master-password";
const GITHUB_TOKEN: &str = "ghp_GIT-TOKEN-SECRET";
const GITLAB_TOKEN: &str = "glpat-LOG-TOKEN-SECRET";
const VAULT_PASSWORD: &str = "VAULT-ITEM-PASSWORD-SECRET";
const VAULT_FIELD: &str = "VAULT-CUSTOM-FIELD-SECRET";
const MCP_TOKENS: common::mcp_mock::AuthTokens<'static> = common::mcp_mock::AuthTokens {
    access: ["MCP-ACCESS-SECRET-1", "MCP-ACCESS-SECRET-2"],
    refresh: ["MCP-REFRESH-SECRET-1", "MCP-REFRESH-SECRET-2"],
    client_secret: Some("MCP-CLIENT-SECRET"),
};
const MCP_STATIC_TOKEN: &str = "MCP-STATIC-SECRET";
const SECRETS: [&str; 14] = [
    "ACCESS-SECRET-1",
    "REFRESH-SECRET-1",
    PASSWORD,
    "REFRESH-OLD-SECRET",
    GITHUB_TOKEN,
    GITLAB_TOKEN,
    VAULT_PASSWORD,
    VAULT_FIELD,
    "OPENSSH PRIVATE KEY",
    "MCP-ACCESS-SECRET",
    "MCP-REFRESH-SECRET",
    "MCP-CLIENT-SECRET",
    MCP_STATIC_TOKEN,
    "code_verifier",
];

struct Capture(Mutex<String>);

impl log::Log for Capture {
    fn enabled(&self, _: &log::Metadata<'_>) -> bool {
        true
    }

    fn log(&self, record: &log::Record<'_>) {
        let line = format!("{} {}\n", record.target(), record.args());
        self.0.lock().unwrap().push_str(&line);
    }

    fn flush(&self) {}
}

fn assert_clean(what: &str, text: &str) {
    for secret in SECRETS {
        assert!(!text.contains(secret), "{what} leaks {secret:?}: {text}");
    }
}

#[tokio::test]
async fn login_and_refresh_never_leak_secrets() {
    let capture: &'static Capture = Box::leak(Box::new(Capture(Mutex::new(String::new()))));
    log::set_logger(capture).unwrap();
    log::set_max_level(log::LevelFilter::Trace);

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/identity/accounts/prelogin"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"kdf": 0, "kdfIterations": 5000})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/identity/connect/token"))
        .and(body_string_contains("twoFactorToken=123456"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "ACCESS-SECRET-1", "refresh_token": "REFRESH-SECRET-1", "expires_in": 7200
        })))
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/identity/connect/token"))
        .and(body_string_contains("grant_type=refresh_token"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({"error": "invalid_grant"})))
        .with_priority(2)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/identity/connect/token"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({"TwoFactorProviders": ["0"]})))
        .mount(&server)
        .await;

    let http = client().unwrap();
    let url = ServerUrl::parse(&server.uri()).unwrap();
    let vault = VaultClient::new(&http, &url);
    let err = vault.login("me@example.com", Zeroizing::new(PASSWORD.to_owned()), None, "d1").await.unwrap_err();
    assert_clean("2FA error", &format!("{err} {err:?}"));
    let tokens =
        vault.login("me@example.com", Zeroizing::new(PASSWORD.to_owned()), Some("123456"), "d1").await.unwrap();
    assert_clean("tokens Debug", &format!("{tokens:?}"));
    let err = vault.refresh("REFRESH-OLD-SECRET").await.unwrap_err();
    assert_clean("refresh error", &format!("{err} {err:?}"));

    let shown = git_through_the_desktop_app().await;
    assert_clean("git views and activity", &shown);
    let shown = desktop_secrets_ssh_and_git_hosts().await;
    assert_clean("desktop secrets, SSH and git hosts", &shown);

    let shown = mcp_through_the_phone().await;
    assert_clean("MCP views, activity, reports and answers", &shown);

    let shown = files_through_the_server().await;
    assert_clean("file views, activity, errors and answers", &shown);

    let logged = capture.0.lock().unwrap().clone();
    assert_clean("log output", &logged);
    assert!(!logged.contains("CAPABILITY"), "the upload and download links are never logged: {logged}");
}

/// A file uploaded for a release asset (the server sends it on, GitHub refuses) and a large asset downloaded through
/// the server: the token travels only inside the requests that need it (`send`, `fetch`), never into a view, the
/// activity, an error or an answer. Returns everything the app or the AI can see.
async fn files_through_the_server() -> String {
    let server = MockServer::start().await;
    let github = MockServer::start().await;
    let ok = |body: Value| ResponseTemplate::new(200).set_body_json(body);
    for (verb, p, body) in [
        ("POST", "/identity/accounts/prelogin", json!({"kdf": 0, "kdfIterations": 5000})),
        (
            "POST",
            "/identity/connect/token",
            json!({"access_token": "ACCESS", "refresh_token": "REFRESH", "expires_in": 7200}),
        ),
        (
            "POST",
            "/rewarden/api/blobs",
            json!({"id": "slot-0123456789abcdef",
            "upload_url": "https://rw.example/rewarden/blob/UP-CAPABILITY", "expires_at": 4_000_000_000_i64}),
        ),
        (
            "GET",
            "/rewarden/api/blobs/blob-for-asset-000001",
            json!({"v": 1, "id": "blob-for-asset-000001",
            "connection_id": "c1", "connection_label": "Claude", "name": "app.zip",
            "purpose": {"kind": "tool_input", "tool": "github_release_asset_upload"}, "state": "uploaded",
            "size": 9, "sha256": "ab", "content_type": "application/zip", "preview": {"kind": "none"},
            "created_at": 1, "expires_at": 4_000_000_000_i64}),
        ),
        (
            "POST",
            "/rewarden/api/blobs/blob-for-asset-000001/send",
            json!({"status": 401,
            "headers": [], "body": "{\"message\": \"Bad credentials\"}", "truncated": false}),
        ),
        (
            "POST",
            "/rewarden/api/blobs/fetch",
            json!({"id": "blob-fetched-0000001",
            "download_url": "https://rw.example/rewarden/blob/DOWN-CAPABILITY", "name": "big.iso", "size": 5_000_000,
            "sha256": "cd", "content_type": "application/octet-stream", "expires_at": 4_000_000_000_i64}),
        ),
    ] {
        Mock::given(method(verb)).and(path(p)).respond_with(ok(body)).mount(&server).await;
    }
    Mock::given(method("POST"))
        .and(path_regex(r"^/rewarden/api/requests/[^/]+/response$"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path_regex(r"^/rewarden/api/blobs/[^/]+$"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    for (p, body) in [
        ("/user", json!({"login": "me"})),
        ("/repos/me/app/releases/7", json!({"id": 7, "tag_name": "v1", "draft": true, "assets": []})),
        ("/repos/me/app/releases/assets/3", json!({"id": 3, "name": "big.iso", "size": 5_000_000})),
    ] {
        Mock::given(method("GET")).and(path(p)).respond_with(ok(body)).mount(&github).await;
    }
    let dir = tempfile::tempdir().unwrap();
    let core = RewardenCore::with_connectors(
        dir.path().to_str().unwrap(),
        &FakeKeys,
        Arc::new(FakeGoogle::new()),
        Arc::new(RecordingNotifier::default()),
        CoreConfig {
            github_base: github.uri(),
            backoff_base: Duration::from_millis(1),
            ..CoreConfig::default()
        },
        vec![],
    )
    .unwrap();
    core.login(server.uri(), "me@example.com".to_owned(), "pw".to_owned(), None).await.unwrap();
    core.add_token_account("github".to_owned(), GITHUB_TOKEN.to_owned()).await.unwrap();
    let request = |id: &str, op: &str, args: Value| -> Value {
        json!({"v": 1, "id": id, "connection_id": "c1", "connection_label": "Claude", "created_at": 1,
            "call": {"tool": "connector", "service": "github", "op": op, "args": args}})
    };
    let upload = json!({"repo": "me/app", "release_id": 7, "name": "app.zip"});
    let mut with_blob = upload.clone();
    with_blob["blob"] = json!("blob-for-asset-000001");
    let requests = vec![
        request("f1", "release_asset_upload", upload),
        request("f2", "release_asset_upload", with_blob),
        request("f3", "release_asset_download", json!({"repo": "me/app", "asset_id": 3})),
    ];
    Mock::given(method("GET"))
        .and(path("/rewarden/api/pending"))
        .respond_with(ok(json!({"requests": requests, "pairings": []})))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    core.sync(0).await.unwrap();
    let views = format!(
        "{:?} {:?} {:?}",
        core.pending().await.unwrap(),
        core.approval_view("f2".to_owned()).await.unwrap(),
        core.approval_view("f3".to_owned()).await.unwrap()
    );
    let pick = |ids: &[&str]| ApprovalChoice {
        selected_message_ids: ids.iter().map(|i| (*i).to_owned()).collect(),
        standing: None,
    };
    let refused = core.approve("f2".to_owned(), pick(&[])).await.unwrap_err();
    assert!(refused.to_string().contains("no longer accepts the token"), "{refused}");
    core.deny("f2".to_owned()).await.unwrap();
    core.approve("f3".to_owned(), pick(&["me/app:asset:3"])).await.unwrap();
    let after =
        format!("{refused} {refused:?} {:?} {:?}", core.activity(50).await.unwrap(), core.grants().await.unwrap());
    let (mut answers, mut carried) = (String::new(), String::new());
    for r in server.received_requests().await.unwrap() {
        let body = String::from_utf8_lossy(&r.body).into_owned();
        if r.url.path().ends_with("/send") || r.url.path().ends_with("/blobs/fetch") {
            carried.push_str(&body);
        } else {
            answers.push_str(&body);
        }
    }
    assert!(carried.contains(GITHUB_TOKEN), "the server got the token for the one request that needed it");
    assert!(answers.contains("DOWN-CAPABILITY") && answers.contains("upload_required"), "the AI got its links");
    format!("{views} {after} {answers}")
}

/// A git fetch and a push from the desktop app, refused, parked, approved and covered: the token only ever leaves
/// sealed. Returns everything the app can show about them.
async fn git_through_the_desktop_app() -> String {
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
            "access_token": "ACCESS", "refresh_token": "REFRESH", "expires_in": 7200})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path_regex(r"^/rewarden/api/requests/[^/]+/response$"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/rewarden/api/pairings/p1/response"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"connection_id": "desk"})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/user"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"login": "me"})))
        .mount(&github)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/me/app"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"private": true})))
        .mount(&github)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let core = RewardenCore::with_connectors(
        dir.path().to_str().unwrap(),
        &FakeKeys,
        Arc::new(FakeGoogle::new()),
        Arc::new(RecordingNotifier::default()),
        CoreConfig {
            github_base: github.uri(),
            backoff_base: Duration::from_millis(1),
            ..CoreConfig::default()
        },
        vec![],
    )
    .unwrap();
    core.login(server.uri(), "me@example.com".to_owned(), "pw".to_owned(), None).await.unwrap();
    core.add_token_account("github".to_owned(), GITHUB_TOKEN.to_owned()).await.unwrap();
    let key = rewarden_proto::desktop::encode_key(&[9u8; 32]);
    let a = "a".repeat(40);
    let b = "b".repeat(40);
    let summary = json!({"updates": [{"name": "refs/heads/main", "change": "update", "old": a, "new": b,
        "fast_forward": true, "commit_count": 1, "files_changed": 0}], "pack_bytes": 10});
    let call = |id: &str, op: &str, key: &str| -> Value {
        json!({"v": 1, "id": id, "connection_id": "desk", "connection_label": "Desktop", "created_at": 1,
            "call": {"tool": "connector", "service": "github", "op": op, "args": {"repo": "me/app",
            "client_key": key, "nonce": "n", "digest": "c".repeat(64), "summary": summary}}})
    };
    let fetch = |id: &str, key: &str| -> Value {
        let mut v = call(id, "git_fetch", key);
        v["call"]["args"] = json!({"repo": "me/app", "client_key": key, "nonce": "n"});
        v
    };
    let pairing = json!({"v": 1, "id": "p1", "client_name": "Rewarden desktop app", "client_host": "127.0.0.1",
        "choices": [1, 2, 3], "created_at": 1, "client_key": key});
    let wrong = rewarden_proto::desktop::encode_key(&[8u8; 32]);
    for (requests, pairings) in [
        (vec![fetch("r0", &key)], vec![pairing]),
        (vec![fetch("r1", &key), call("r2", "git_push", &key), call("r3", "git_push", &wrong)], vec![]),
    ] {
        Mock::given(method("GET"))
            .and(path("/rewarden/api/pending"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"requests": requests, "pairings": pairings})))
            .up_to_n_times(1)
            .with_priority(1)
            .mount(&server)
            .await;
        core.sync(0).await.unwrap();
        if core.pairing_view("p1".to_owned()).await.is_ok() {
            core.answer_pairing("p1".to_owned(), true, Some(2), None).await.unwrap();
        }
    }
    let before = format!(
        "{:?} {:?} {:?}",
        core.pending().await.unwrap(),
        core.approval_view("r1".to_owned()).await.unwrap(),
        core.approval_view("r2".to_owned()).await.unwrap()
    );
    let pick = |ids: &[&str]| ApprovalChoice {
        selected_message_ids: ids.iter().map(|i| (*i).to_owned()).collect(),
        standing: None,
    };
    core.approve("r1".to_owned(), pick(&["me/app"])).await.unwrap();
    core.approve("r2".to_owned(), pick(&[])).await.unwrap();
    let after = format!("{:?} {:?}", core.activity(50).await.unwrap(), core.grants().await.unwrap());
    let sent: String = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|r| String::from_utf8_lossy(&r.body).into_owned())
        .collect();
    let shown = format!("{before} {after} {sent}");
    assert!(shown.contains("sealed"), "the answers were sent");
    shown
}

/// Secrets released to the desktop app, an SSH sign-in, and a GitLab fetch, each approved, plus a few refused: the
/// values and tokens only ever leave sealed. Returns everything the app can show and everything that was sent.
async fn desktop_secrets_ssh_and_git_hosts() -> String {
    use common::desktop::{DESK, call, choice, desk_with, mount_rewarden};
    use data_encoding::BASE64;
    use rewarden_core::crypto::{Kdf, VaultKey, master_key};
    use wiremock::matchers::header;

    const EMAIL: &str = "me@example.com";
    const MASTER: &str = "vault master pw";
    let server = MockServer::start().await;
    mount_rewarden(&server).await;
    let gitlab = MockServer::start().await;
    for at in ["/user", "/projects/me%2Fapp"] {
        Mock::given(method("GET"))
            .and(path(at))
            .and(header("authorization", format!("Bearer {GITLAB_TOKEN}").as_str()))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"username": "me", "visibility": "private"})))
            .mount(&gitlab)
            .await;
    }
    let user = VaultKey::from_bytes(&[5u8; 64]).unwrap();
    let t = |s: &str| json!(user.encrypt(s.as_bytes()).unwrap());
    let master = master_key(
        MASTER,
        EMAIL,
        Kdf::Pbkdf2 {
            iterations: 5000,
        },
    )
    .unwrap();
    let ed_private = include_str!("fixtures/ssh/k_ed");
    let ed_public = include_str!("fixtures/ssh/k_ed.pub");
    let base = json!({"notes": null, "key": null, "folderId": null, "favorite": false, "deletedDate": null,
        "archivedDate": null, "organizationId": null, "reprompt": 0, "passwordHistory": [], "attachments": null,
        "login": null, "secureNote": null, "card": null, "identity": null, "sshKey": null, "fields": []});
    let mut deploy = base.clone();
    deploy["id"] = json!("deploy");
    deploy["type"] = json!(1);
    deploy["name"] = t("Deploy");
    deploy["login"] = json!({"username": t("ci"), "password": t(VAULT_PASSWORD), "totp": null, "uris": null});
    deploy["fields"] = json!([{"name": t("token"), "value": t(VAULT_FIELD), "type": 1, "linkedId": null}]);
    let mut key = base;
    key["id"] = json!("key");
    key["type"] = json!(5);
    key["name"] = t("Laptop key");
    key["sshKey"] = json!({"privateKey": t(ed_private), "publicKey": t(ed_public), "keyFingerprint": t("x")});
    Mock::given(method("GET"))
        .and(path("/api/sync"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "profile": {"key": master.stretch().encrypt(&user.to_bytes()).unwrap()},
            "ciphers": [deploy, key], "folders": []})))
        .mount(&server)
        .await;
    let desk = desk_with(
        server,
        EMAIL,
        MASTER,
        CoreConfig {
            gitlab_base: gitlab.uri(),
            ..CoreConfig::default()
        },
    )
    .await;
    desk.core.add_token_account("vault".to_owned(), MASTER.to_owned()).await.unwrap();
    desk.core.add_token_account("gitlab".to_owned(), GITLAB_TOKEN.to_owned()).await.unwrap();
    // Refused before pairing, then paired.
    let release = |id: &str, secrets: &[&str]| {
        call(
            id,
            DESK,
            "vault",
            "secret_release",
            &json!({"secrets": secrets, "command": "deploy",
            "client_key": desk.public(), "nonce": "n"}),
        )
    };
    desk.send(&[release("r0", &["Deploy/password"])]).await;
    desk.pair().await;
    let blob = BASE64.decode(ed_public.split_whitespace().nth(1).unwrap().as_bytes()).unwrap();
    // An SSH user authentication request for the key: strings, with the message type and "has signature" bytes.
    let string = |out: &mut Vec<u8>, s: &[u8]| {
        out.extend_from_slice(&u32::try_from(s.len()).unwrap().to_be_bytes());
        out.extend_from_slice(s);
    };
    let mut data = Vec::new();
    string(&mut data, &[1u8; 32]);
    data.push(50);
    for part in [&b"git"[..], b"ssh-connection", b"publickey"] {
        string(&mut data, part);
    }
    data.push(1);
    string(&mut data, b"ssh-ed25519");
    string(&mut data, &blob);
    let fingerprint = "SHA256:59UqiUdGtW4OqO6NNXeMEo3goCDvszgSgq1R0tab5n0";
    desk.send(&[
        release("r1", &["Deploy/password", "Deploy/token"]),
        release("r2", &["Deploy/colour"]),
        call(
            "r3",
            DESK,
            "vault",
            "ssh_sign",
            &json!({"key": fingerprint, "data_base64": BASE64.encode(&data),
            "host": "example.org", "client_key": desk.public(), "nonce": "n"}),
        ),
        call("r4", DESK, "gitlab", "git_fetch", &json!({"repo": "me/app", "client_key": desk.public(), "nonce": "n"})),
    ])
    .await;
    let before = desk.everything_shown(&["r1", "r3", "r4"]).await;
    desk.core.approve("r1".to_owned(), choice(&[], None)).await.unwrap();
    desk.core.approve("r3".to_owned(), choice(&[], None)).await.unwrap();
    desk.core.approve("r4".to_owned(), choice(&["me/app"], None)).await.unwrap();
    let after = desk.everything_shown(&[]).await;
    assert_eq!(desk.data("r1").await.as_object().unwrap().len(), 2, "sealed and expires_at");
    assert!(desk.data("r3").await["sealed"].is_string());
    format!("{before} {after}")
}

fn approve_once() -> ApprovalChoice {
    ApprovalChoice {
        selected_message_ids: Vec::new(),
        standing: None,
    }
}

/// An MCP server added with an OAuth sign-in (with a client secret) and another with a token; calls approved, a token
/// refreshed, a refresh refused. Returns everything the app can show and everything reported or answered to the server
/// (except the proxied call, which carries the token by design).
async fn mcp_through_the_phone() -> String {
    use common::mcp_mock::{McpState, mcp_request, mount_auth, mount_mcp, mount_rewarden};
    use rewarden_core::McpAddStep;

    let (rw, mcp, auth, other) =
        (MockServer::start().await, MockServer::start().await, MockServer::start().await, MockServer::start().await);
    let state = Arc::new(Mutex::new(McpState {
        valid_token: Some(MCP_TOKENS.access[0].to_owned()),
        ..McpState::default()
    }));
    let static_state = Arc::new(Mutex::new(McpState {
        valid_token: Some(MCP_STATIC_TOKEN.to_owned()),
        ..McpState::default()
    }));
    mount_rewarden(&rw).await;
    mount_mcp(&mcp, &auth, &state).await;
    mount_mcp(&other, &auth, &static_state).await;
    mount_auth(&auth, &MCP_TOKENS).await;
    let dir = tempfile::tempdir().unwrap();
    let core = RewardenCore::with_connectors(
        dir.path().to_str().unwrap(),
        &FakeKeys,
        Arc::new(FakeGoogle::new()),
        Arc::new(RecordingNotifier::default()),
        CoreConfig {
            backoff_base: Duration::from_millis(1),
            ..CoreConfig::default()
        },
        vec![],
    )
    .unwrap();
    core.login(rw.uri(), "me@example.com".to_owned(), "pw".to_owned(), None).await.unwrap();
    let McpAddStep::NeedsSignIn {
        server_id,
        authorize_url,
    } = core.mcp_add(format!("{}/mcp", mcp.uri()), Some("Tracker".to_owned())).await.unwrap()
    else {
        panic!("a sign-in is needed");
    };
    let page = url::Url::parse(&authorize_url).unwrap();
    let state_param = page.query_pairs().find(|(k, _)| k == "state").unwrap().1.into_owned();
    let redirect = format!("com.reins2fa.app://mcp-oauth?code=CODE-1&state={state_param}");
    core.mcp_finish_sign_in(server_id, redirect).await.unwrap();
    let other_url = format!("{}/mcp", other.uri());
    let wrong = core.mcp_add_with_token(other_url.clone(), "WRONG".to_owned(), None).await.unwrap_err();
    core.mcp_add_with_token(other_url, MCP_STATIC_TOKEN.to_owned(), Some("Other".to_owned())).await.unwrap();

    let relay = |id: &str, server: &str| mcp_request(id, server, "create_issue", &json!({"title": "t"}));
    core.engine().process_relayed(relay("m1", "tracker")).await.unwrap();
    core.engine().process_relayed(relay("m2", "other")).await.unwrap();
    let views = format!(
        "{:?} {:?} {:?} {:?}",
        core.pending().await.unwrap(),
        core.approval_view("m1".to_owned()).await.unwrap(),
        core.approval_view("m2".to_owned()).await.unwrap(),
        core.mcp_servers().await.unwrap()
    );
    // The first token is no longer taken: refreshed with the client secret.
    state.lock().unwrap().valid_token = Some(MCP_TOKENS.access[1].to_owned());
    core.approve("m1".to_owned(), approve_once()).await.unwrap();
    core.approve("m2".to_owned(), approve_once()).await.unwrap();
    // Then nothing works any more.
    state.lock().unwrap().valid_token = Some("nobody".to_owned());
    core.engine().process_relayed(relay("m3", "tracker")).await.unwrap();
    let failed = core.approve("m3".to_owned(), approve_once()).await.unwrap_err();
    let refreshed = core.mcp_refresh("tracker".to_owned()).await.unwrap();
    let after = format!(
        "{wrong} {wrong:?} {failed} {failed:?} {refreshed:?} {:?} {:?} {:?}",
        core.activity(50).await.unwrap(),
        core.grants().await.unwrap(),
        core.mcp_servers().await.unwrap()
    );
    let token_form = auth
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path() == "/token")
        .map(|r| String::from_utf8_lossy(&r.body).into_owned())
        .collect::<String>();
    assert!(token_form.contains("client_secret=MCP-CLIENT-SECRET"), "the client secret was used");
    let sent: String = rw
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path() != "/rewarden/api/mcp/call")
        .map(|r| String::from_utf8_lossy(&r.body).into_owned())
        .collect();
    assert!(sent.contains("created ISS-2"), "the answers were sent");
    assert!(sent.contains(r#""mcp":[{"#), "the tools were reported");
    format!("{views} {after} {sent}")
}
