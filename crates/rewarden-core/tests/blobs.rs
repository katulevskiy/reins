//! Large files through the Rewarden server, under the phone's control: upload links for file tools, the file shown
//! with the write and sent on by the server, uploads the user decides on (`rewarden_upload`), and large results handed
//! over as download links once released. A mock plays the Rewarden server (its blob endpoints), the vault and GitHub.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use aes::Aes256;
use cbc::cipher::block_padding::Pkcs7;
use cbc::cipher::{BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use common::{FakeGoogle, FakeKeys, RecordingNotifier};
use data_encoding::BASE64;
use rewarden_core::crypto::{Kdf, VaultKey, master_key};
use rewarden_core::{
    ApprovalChoice, ApprovalKind, CoreConfig, GoogleTokenProvider, GrantScopeChoice, Notifier, PendingKind,
    RewardenCore, StandingGrant,
};
use rewarden_proto::blob::MAX_BLOB_BYTES;
use ring::hmac;
use serde_json::{Value, json};
use wiremock::matchers::{method, path, path_regex};
use wiremock::{Match, Mock, MockServer, Request, ResponseTemplate};

const PASSWORD: &str = "correct horse battery";
const EMAIL: &str = "me@example.com";
const GH_TOKEN: &str = "ghp_BLOB-TEST-TOKEN";
const SLOT: &str = "slot-0123456789abcdef";
const UPLOADED: &str = "blob-uploaded-00000001";

struct Env {
    server: MockServer,
    github: MockServer,
    core: Arc<RewardenCore>,
    notifier: Arc<RecordingNotifier>,
    user: VaultKey,
    counter: AtomicU32,
    _dir: tempfile::TempDir,
}

fn now() -> i64 {
    i64::try_from(SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()).unwrap()
}

fn t(key: &VaultKey, s: &str) -> Value {
    json!(key.encrypt(s.as_bytes()).unwrap())
}

/// Encrypts a file the way Bitwarden clients do: `0x02 || iv || mac || ciphertext`.
fn buffer_encrypt(key: &[u8], data: &[u8]) -> Vec<u8> {
    let iv = [5u8; 16];
    let ct = cbc::Encryptor::<Aes256>::new_from_slices(&key[..32], &iv).unwrap().encrypt_padded_vec_mut::<Pkcs7>(data);
    let mut signed = iv.to_vec();
    signed.extend_from_slice(&ct);
    let mac = hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, &key[32..]), &signed);
    let mut out = vec![2u8];
    out.extend_from_slice(&iv);
    out.extend_from_slice(mac.as_ref());
    out.extend_from_slice(&ct);
    out
}

fn buffer_decrypt(key: &[u8], blob: &[u8]) -> Vec<u8> {
    let (iv, rest) = blob[1..].split_at(16);
    let (mac, ct) = rest.split_at(32);
    let mut signed = iv.to_vec();
    signed.extend_from_slice(ct);
    hmac::verify(&hmac::Key::new(hmac::HMAC_SHA256, &key[32..]), &signed, mac).expect("MAC");
    cbc::Decryptor::<Aes256>::new_from_slices(&key[..32], iv).unwrap().decrypt_padded_vec_mut::<Pkcs7>(ct).unwrap()
}

/// The data part of a multipart upload.
fn upload_part(request: &Request) -> Vec<u8> {
    let content_type = request.headers.get("content-type").unwrap().to_str().unwrap().to_owned();
    let boundary = content_type.split("boundary=").nth(1).unwrap().trim_matches('"').to_owned();
    let body = &request.body;
    let marker = format!("--{boundary}");
    let text = String::from_utf8_lossy(body);
    let start = text.find(&marker).unwrap();
    let data_start = text[start..].find("\r\n\r\n").unwrap() + start + 4;
    let data_end = body.windows(marker.len() + 2).rposition(|w| w == format!("\r\n{marker}").as_bytes()).unwrap();
    body[data_start..data_end].to_vec()
}

struct NotGet;

impl Match for NotGet {
    fn matches(&self, request: &Request) -> bool {
        request.method.as_str() != "GET"
    }
}

/// The large attachment of the vault item `git`.
fn big_attachment() -> Vec<u8> {
    (0..300_000u32).map(|i| u8::try_from(i % 251).unwrap()).collect()
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
            "access_token": "ACCESS", "refresh_token": "REFRESH", "expires_in": 7200})))
        .mount(&server)
        .await;
    // The vault: one login with one large attachment.
    let master = master_key(
        PASSWORD,
        EMAIL,
        Kdf::Pbkdf2 {
            iterations: 5000,
        },
    )
    .unwrap();
    let user = VaultKey::from_bytes(&[7u8; 64]).unwrap();
    let att_key = vec![11u8; 64];
    let wrapped = master.stretch().encrypt(&user.to_bytes()).unwrap();
    let git = json!({"id": "git", "type": 1, "name": t(&user, "GitHub"), "notes": null, "key": null,
        "folderId": null, "favorite": false, "deletedDate": null, "archivedDate": null, "organizationId": null,
        "creationDate": "2026-01-01T00:00:00.000000Z", "revisionDate": "2026-02-02T10:00:00.000000Z",
        "reprompt": 0, "fields": [], "passwordHistory": [], "login": {"username": t(&user, "octo")},
        "secureNote": null, "card": null, "identity": null, "sshKey": null,
        "attachments": [{"id": "att9", "fileName": t(&user, "big.bin"), "size": "300000",
            "key": user.encrypt(&att_key).unwrap(), "object": "attachment"}]});
    Mock::given(method("GET"))
        .and(path("/api/sync"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"profile": {"key": wrapped}, "ciphers": [git], "folders": []})),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/ciphers/git/attachment/att9"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "att9",
            "url": format!("{}/attachments/git/att9?token=T", server.uri())})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/attachments/git/att9"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(buffer_encrypt(&att_key, &big_attachment())))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path_regex(r"^/api/ciphers/[^/]+/attachment/v2$"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"attachmentId": "att-new",
            "url": "/ciphers/git/attachment/att-new", "fileUploadType": 0})))
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(NotGet)
        .and(path_regex(r"^/api/ciphers(/.*)?$"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "new-id"})))
        .with_priority(9)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path_regex(r"^/rewarden/api/requests/[^/]+/response$"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/user"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"login": "octo"})))
        .mount(&github)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let google: Arc<dyn GoogleTokenProvider> = Arc::new(FakeGoogle::new());
    let notifier = Arc::new(RecordingNotifier::default());
    let dyn_notifier: Arc<dyn Notifier> = Arc::<RecordingNotifier>::clone(&notifier);
    let cfg = CoreConfig {
        github_base: github.uri(),
        backoff_base: Duration::from_millis(1),
        ..CoreConfig::default()
    };
    let core = RewardenCore::with_config(dir.path().to_str().unwrap(), &FakeKeys, google, dyn_notifier, cfg).unwrap();
    core.login(server.uri(), EMAIL.to_owned(), PASSWORD.to_owned(), None).await.unwrap();
    core.add_token_account("vault".into(), PASSWORD.into()).await.unwrap();
    core.add_token_account("github".into(), GH_TOKEN.into()).await.unwrap();
    let env = Env {
        server,
        github,
        core,
        notifier,
        user,
        counter: AtomicU32::new(0),
        _dir: dir,
    };
    env.blob_endpoints().await;
    env
}

/// What the server says about a file it holds.
fn info(id: &str, connection: &str, tool: &str, size: u64, preview: &Value) -> Value {
    json!({"v": 1, "id": id, "connection_id": connection, "connection_label": "Claude", "request_id": null,
        "name": "app.zip", "purpose": {"kind": "tool_input", "tool": tool}, "state": "uploaded", "size": size,
        "sha256": "ab".repeat(32), "content_type": "application/zip", "preview": preview,
        "created_at": now() - 60, "expires_at": now() + 1_200})
}

fn download(id: &str, name: &str, size: u64) -> Value {
    json!({"id": id, "download_url": format!("https://rw.example/rewarden/blob/DOWNLOAD-{id}"), "name": name,
        "size": size, "sha256": "cd".repeat(32), "content_type": "application/octet-stream",
        "expires_at": now() + 1_800})
}

fn choice(ids: &[String], standing: Option<StandingGrant>) -> ApprovalChoice {
    ApprovalChoice {
        selected_message_ids: ids.to_vec(),
        standing,
    }
}

fn read_grant(resources: &[&str]) -> Option<StandingGrant> {
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
            classes: vec![],
        },
    })
}

impl Env {
    /// The blob endpoints of the Rewarden server, answering every call.
    async fn blob_endpoints(&self) {
        Mock::given(method("POST"))
            .and(path("/rewarden/api/blobs"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": SLOT,
                "upload_url": "https://rw.example/rewarden/blob/UPLOAD-SECRET",
                "download_url": "https://rw.example/rewarden/blob/DOWNLOAD-SECRET",
                "expires_at": now() + 1_800})))
            .mount(&self.server)
            .await;
        Mock::given(method("DELETE"))
            .and(path_regex(r"^/rewarden/api/blobs/[^/]+$"))
            .respond_with(ResponseTemplate::new(204))
            .mount(&self.server)
            .await;
        Mock::given(method("POST"))
            .and(path_regex(r"^/rewarden/api/blobs/[^/]+/decision$"))
            .respond_with(ResponseTemplate::new(204))
            .mount(&self.server)
            .await;
        Mock::given(method("POST"))
            .and(path("/rewarden/api/blobs/fetch"))
            .respond_with(ResponseTemplate::new(200).set_body_json(download(
                "blob-fetched-0000001",
                "big.iso",
                5_000_000,
            )))
            .mount(&self.server)
            .await;
        Mock::given(method("PUT"))
            .and(path("/rewarden/api/blobs/output"))
            .respond_with(ResponseTemplate::new(200).set_body_json(download(
                "blob-output-00000001",
                "big.bin",
                300_000,
            )))
            .mount(&self.server)
            .await;
    }

    /// The server holds this file.
    async fn holds(&self, info: &Value) {
        Mock::given(method("GET"))
            .and(path(format!("/rewarden/api/blobs/{}", info["id"].as_str().unwrap())))
            .respond_with(ResponseTemplate::new(200).set_body_json(info))
            .mount(&self.server)
            .await;
    }

    /// The server sends a file on and the destination answers `status` with `body`.
    async fn sends_on(&self, id: &str, status: u16, body: &Value) {
        Mock::given(method("POST"))
            .and(path(format!("/rewarden/api/blobs/{id}/send")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": status,
                "headers": [["content-type", "application/json"]], "body": body.to_string(), "truncated": false})))
            .mount(&self.server)
            .await;
    }

    async fn gh(&self, verb: &str, p: &str, body: &Value) {
        Mock::given(method(verb))
            .and(path(p))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&self.github)
            .await;
    }

    /// Relays one call to the phone. Returns its id and whether it was parked for the user.
    async fn ask(&self, service: &str, op: &str, args: &Value) -> (String, bool) {
        let id = format!("r{}", self.counter.fetch_add(1, Ordering::SeqCst) + 1);
        let call = json!({"tool": "connector", "service": service, "op": op, "args": args});
        self.relay(&id, &call).await;
        let parked = self.core.pending().await.unwrap().iter().any(|p| p.id == id);
        (id, parked)
    }

    async fn relay(&self, id: &str, call: &Value) {
        let request = json!({"v": 1, "id": id, "connection_id": "c1", "connection_label": "Claude",
            "created_at": now(), "call": call});
        self.pending(&json!({"requests": [request], "pairings": []})).await;
        self.core.sync(0).await.unwrap();
    }

    async fn pending(&self, body: &Value) {
        Mock::given(method("GET"))
            .and(path("/rewarden/api/pending"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .up_to_n_times(1)
            .with_priority(1)
            .mount(&self.server)
            .await;
        Mock::given(method("GET"))
            .and(path("/rewarden/api/pending"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"requests": [], "pairings": []})))
            .mount(&self.server)
            .await;
    }

    /// The last answer the AI got.
    async fn told(&self) -> Value {
        self.requests("POST", "/response").await.pop().map(|r| serde_json::from_slice(&r.body).unwrap()).unwrap()
    }

    /// Requests the server saw with this method whose path ends with `suffix`.
    async fn requests(&self, verb: &str, suffix: &str) -> Vec<Request> {
        self.server
            .received_requests()
            .await
            .unwrap()
            .into_iter()
            .filter(|r| r.method.as_str() == verb && r.url.path().ends_with(suffix))
            .collect()
    }

    async fn json_of(&self, verb: &str, suffix: &str) -> Vec<Value> {
        self.requests(verb, suffix).await.iter().map(|r| serde_json::from_slice(&r.body).unwrap()).collect()
    }
}

fn header<'a>(send: &'a Value, name: &str) -> Option<&'a str> {
    send["headers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|h| h[0].as_str().is_some_and(|n| n.eq_ignore_ascii_case(name)))
        .and_then(|h| h[1].as_str())
}

// ---- file tools without content -------------------------------------------------------------------------------------

#[tokio::test]
async fn file_tools_called_without_content_are_told_where_to_upload_it() {
    let env = env().await;
    let args = json!({"repo": "octo/cat", "release_id": 7, "name": "app.apk",
        "content_type": "application/vnd.android.package-archive"});
    let (id, parked) = env.ask("github", "release_asset_upload", &args).await;
    assert!(!parked, "nothing to decide yet");
    let told = env.told().await;
    assert_eq!(told["outcome"], "result", "an answer, not an error: {told}");
    let data = &told["result"]["data"];
    assert_eq!(
        (data["status"].as_str(), data["blob"].as_str(), data["upload_url"].as_str()),
        (Some("upload_required"), Some(SLOT), Some("https://rw.example/rewarden/blob/UPLOAD-SECRET"))
    );
    assert_eq!(data["max_bytes"], MAX_BLOB_BYTES);
    let next = data["next"].as_str().unwrap();
    for needle in ["curl -T", "UPLOAD-SECRET", "github_release_asset_upload", SLOT, "\"blob\""] {
        assert!(next.contains(needle), "{needle} missing in {next}");
    }
    // The slot the phone opened: this connection, this request, this tool only.
    let slot = env.json_of("POST", "/rewarden/api/blobs").await.pop().unwrap();
    assert_eq!(
        slot,
        json!({"v": 1, "connection_id": "c1", "request_id": id, "name": "app.apk",
            "content_type": "application/vnd.android.package-archive", "max_bytes": MAX_BLOB_BYTES,
            "purpose": {"kind": "tool_input", "tool": "github_release_asset_upload"}, "ttl_secs": 1_800})
    );
    let entry = &env.core.activity(1).await.unwrap()[0];
    assert_eq!((entry.outcome.as_str(), entry.service.as_str()), ("released", "github"));
    assert!(entry.detail.contains("upload link"), "{}", entry.detail);

    // The contents API and the vault get slots of their own, named after the file.
    env.ask("github", "file_put", &json!({"repo": "octo/cat", "path": "docs/big.pdf", "message": "Add"})).await;
    let slot = env.json_of("POST", "/rewarden/api/blobs").await.pop().unwrap();
    assert_eq!((slot["name"].as_str(), slot["max_bytes"].as_u64()), (Some("big.pdf"), Some(100 * 1024 * 1024)));
    assert_eq!(slot["purpose"]["tool"], "github_file_put");
    env.ask("vault", "attachment_add", &json!({"item": "git", "file_name": "scan.pdf"})).await;
    let slot = env.json_of("POST", "/rewarden/api/blobs").await.pop().unwrap();
    assert_eq!(
        (slot["name"].as_str(), slot["purpose"]["tool"].as_str()),
        (Some("scan.pdf"), Some("vault_attachment_add"))
    );
    assert_eq!(env.told().await["result"]["data"]["status"], "upload_required");

    // Content and an upload at once is a mistake.
    let both =
        json!({"repo": "octo/cat", "release_id": 7, "name": "a.zip", "content_base64": "eA==", "blob": UPLOADED});
    let (_, parked) = env.ask("github", "release_asset_upload", &both).await;
    assert!(!parked);
    let told = env.told().await;
    assert_eq!(told["outcome"], "error");
    assert!(told["message"].as_str().unwrap().contains("not both"), "{told}");
}

// ---- writes that use an upload --------------------------------------------------------------------------------------

#[tokio::test]
async fn an_uploaded_asset_is_shown_with_the_write_and_streamed_to_github_by_the_server() {
    let env = env().await;
    let preview = json!({"kind": "binary", "description": "ZIP archive"});
    env.holds(&info(UPLOADED, "c1", "github_release_asset_upload", 5_000_000, &preview)).await;
    env.gh(
        "GET",
        "/repos/octo/cat/releases/7",
        &json!({"id": 7, "name": "v1", "tag_name": "v1.0", "draft": false,
        "assets": []}),
    )
    .await;
    env.sends_on(
        UPLOADED,
        201,
        &json!({"id": 99, "name": "app.zip", "size": 5_000_000, "state": "uploaded",
        "browser_download_url": "https://github.com/octo/cat/releases/download/v1.0/app.zip"}),
    )
    .await;
    let args = json!({"repo": "octo/cat", "release_id": 7, "name": "app.zip", "content_type": "application/zip",
        "blob": UPLOADED});
    let (id, parked) = env.ask("github", "release_asset_upload", &args).await;
    assert!(parked);
    let view = env.core.approval_view(id.clone()).await.unwrap();
    assert_eq!((view.kind, view.class.as_str()), (ApprovalKind::Write, "releases"));
    let lines = view.preview.join("\n");
    for needle in [
        "Attach `app.zip` to the release",
        "the uploaded file",
        "Uploaded file: app.zip · 4.8 MB · application/zip",
        &format!("SHA-256: {}", "ab".repeat(32)),
        "Looks like: ZIP archive",
    ] {
        assert!(lines.contains(needle), "{needle} missing in\n{lines}");
    }
    let blob = view.blob.expect("the file is part of the approval");
    assert_eq!(
        (blob.id.as_str(), blob.name.as_str(), blob.size, blob.purpose.as_str()),
        (UPLOADED, "app.zip", 5_000_000, "For github_release_asset_upload")
    );
    assert_eq!((blob.preview_text.as_deref(), blob.preview_image), (Some("ZIP archive"), None));
    assert!(env.requests("POST", "/send").await.is_empty(), "nothing moves before the user approved");

    env.core.approve(id, choice(&[], None)).await.unwrap();
    let send = env.json_of("POST", &format!("/rewarden/api/blobs/{UPLOADED}/send")).await.pop().unwrap();
    assert_eq!(
        (send["method"].as_str(), send["url"].as_str(), &send["body"]),
        (
            Some("POST"),
            Some("https://uploads.github.com/repos/octo/cat/releases/7/assets?name=app.zip"),
            &json!({"kind": "raw"})
        )
    );
    assert_eq!(header(&send, "Authorization"), Some(format!("Bearer {GH_TOKEN}").as_str()));
    assert_eq!(header(&send, "Content-Type"), Some("application/zip"));
    assert_eq!(header(&send, "X-GitHub-Api-Version"), Some("2022-11-28"));
    let told = env.told().await;
    assert_eq!(
        (told["result"]["data"]["uploaded"].as_bool(), told["result"]["data"]["id"].as_i64()),
        (Some(true), Some(99))
    );
    assert_eq!(env.requests("DELETE", &format!("/rewarden/api/blobs/{UPLOADED}")).await.len(), 1, "used, then deleted");
}

#[tokio::test]
async fn a_file_put_from_an_upload_goes_as_base64_inside_the_contents_api_body() {
    let env = env().await;
    let png = [0x89u8, b'P', b'N', b'G', 1, 2, 3];
    let preview = json!({"kind": "image", "mime": "image/png", "data_base64": BASE64.encode(&png)});
    let id = "blob-image-000000001";
    env.holds(&info(id, "c1", "github_file_put", 700_000, &preview)).await;
    env.gh("GET", "/repos/octo/cat", &json!({"default_branch": "main"})).await;
    env.gh("GET", "/repos/octo/cat/git/ref/heads/main", &json!({"object": {"sha": "1".repeat(40)}})).await;
    env.sends_on(
        id,
        201,
        &json!({"commit": {"sha": "c0ffee", "html_url": "https://github.com/octo/cat/commit/c0ffee"},
        "content": {"sha": "f11e"}}),
    )
    .await;
    let args = json!({"repo": "octo/cat", "path": "docs/logo.png", "message": "Add the logo", "blob": id});
    let (request_id, parked) = env.ask("github", "file_put", &args).await;
    assert!(parked);
    let view = env.core.approval_view(request_id.clone()).await.unwrap();
    assert!(view.preview.iter().any(|l| l == "An image (shown below)"), "{:?}", view.preview);
    assert_eq!(view.blob.unwrap().preview_image.as_deref(), Some(&png[..]));
    assert_eq!(view.resources[0].id, "octo/cat@main");

    env.core.approve(request_id, choice(&[], None)).await.unwrap();
    let send = env.json_of("POST", &format!("/rewarden/api/blobs/{id}/send")).await.pop().unwrap();
    assert_eq!(send["method"], "PUT");
    assert_eq!(send["url"], format!("{}/repos/octo/cat/contents/docs/logo.png", env.github.uri()));
    assert_eq!(
        send["body"],
        json!({"kind": "json_base64", "json": {"message": "Add the logo", "branch": "main"}, "field": "content"})
    );
    let done = env.told().await["result"]["data"].clone();
    assert_eq!((done["committed"].as_bool(), done["commit_sha"].as_str()), (Some(true), Some("c0ffee")));
    assert!(
        !env.github.received_requests().await.unwrap().iter().any(|r| r.method.as_str() == "PUT"),
        "the phone itself never sends the file"
    );
}

#[tokio::test]
async fn an_upload_of_another_connection_an_expired_one_or_one_for_another_tool_is_refused() {
    let env = env().await;
    let none = json!({"kind": "none"});
    env.holds(&info("blob-other-conn-00001", "c2", "github_release_asset_upload", 10, &none)).await;
    env.holds(&info("blob-other-tool-00001", "c1", "github_file_put", 10, &none)).await;
    let mut expired = info("blob-expired-0000001", "c1", "github_release_asset_upload", 10, &none);
    expired["expires_at"] = json!(now() - 5);
    env.holds(&expired).await;
    let mut waiting = info("blob-waiting-0000001", "c1", "github_release_asset_upload", 0, &none);
    waiting["state"] = json!("waiting");
    env.holds(&waiting).await;
    for (blob, needle) in [
        ("blob-other-conn-00001", "another connection"),
        ("blob-other-tool-00001", "made for something else"),
        ("blob-expired-0000001", "expired"),
        ("blob-waiting-0000001", "not arrived yet"),
        ("blob-unknown-0000001", "no such upload"),
        ("short", "not the id of an upload"),
    ] {
        let args = json!({"repo": "octo/cat", "release_id": 7, "name": "a.zip", "blob": blob});
        let (_, parked) = env.ask("github", "release_asset_upload", &args).await;
        assert!(!parked, "{blob}");
        let told = env.told().await;
        assert_eq!(told["outcome"], "error", "{blob}: {told}");
        assert!(told["message"].as_str().unwrap().contains(needle), "{blob}: {told}");
    }
    assert!(env.requests("POST", "/send").await.is_empty());
    assert!(
        env.github
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|r| r.url.path() == "/user" || r.method.as_str() == "HEAD")
    );
}

#[tokio::test]
async fn a_refused_write_deletes_its_upload() {
    let env = env().await;
    env.holds(&info(UPLOADED, "c1", "github_release_asset_upload", 10, &json!({"kind": "none"}))).await;
    env.gh("GET", "/repos/octo/cat/releases/7", &json!({"id": 7, "tag_name": "v1", "draft": true, "assets": []})).await;
    let args = json!({"repo": "octo/cat", "release_id": 7, "name": "a.zip", "blob": UPLOADED});
    let (id, parked) = env.ask("github", "release_asset_upload", &args).await;
    assert!(parked);
    env.core.deny(id).await.unwrap();
    assert_eq!(env.told().await["outcome"], "denied");
    assert_eq!(env.requests("DELETE", &format!("/rewarden/api/blobs/{UPLOADED}")).await.len(), 1);
}

#[tokio::test]
async fn a_vault_attachment_from_an_upload_is_read_encrypted_on_the_phone_and_then_deleted() {
    let env = env().await;
    let content = b"scanned contract, many pages".to_vec();
    let id = "blob-for-the-vault-01";
    let mut held = info(
        id,
        "c1",
        "vault_attachment_add",
        content.len() as u64,
        &json!({"kind": "text",
        "head": "scanned contract", "truncated": true}),
    );
    held["name"] = json!("scan.pdf");
    env.holds(&held).await;
    Mock::given(method("GET"))
        .and(path(format!("/rewarden/api/blobs/{id}/content")))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(content.clone()))
        .mount(&env.server)
        .await;
    let (request_id, parked) =
        env.ask("vault", "attachment_add", &json!({"item": "git", "file_name": "scan.pdf", "blob": id})).await;
    assert!(parked);
    let view = env.core.approval_view(request_id.clone()).await.unwrap();
    let lines = view.preview.join("\n");
    assert!(lines.contains("Attach \"scan.pdf\" (the uploaded file, below)"), "{lines}");
    assert!(lines.contains("Starts with:\nscanned contract\n…"), "{lines}");
    assert!(env.requests("GET", "/content").await.is_empty(), "read only once approved");

    env.core.approve(request_id, choice(&[], None)).await.unwrap();
    let announce = env.json_of("POST", "/api/ciphers/git/attachment/v2").await.pop().unwrap();
    let file_key = env.user.decrypt(announce["key"].as_str().unwrap()).unwrap();
    let upload = env.requests("POST", "/api/ciphers/git/attachment/att-new").await.pop().expect("uploaded");
    assert_eq!(buffer_decrypt(&file_key, &upload_part(&upload)), content, "encrypted on the phone");
    assert_eq!(env.told().await["result"]["data"]["attachment"], "att-new");
    assert_eq!(env.requests("DELETE", &format!("/rewarden/api/blobs/{id}")).await.len(), 1);
}

// ---- rewarden_upload ------------------------------------------------------------------------------------------------

#[tokio::test]
async fn rewarden_upload_answers_with_links_and_the_user_decides_when_the_file_arrives() {
    let env = env().await;
    let call = json!({"tool": "request_upload", "name": "report.pdf", "size": 1_000,
        "content_type": "application/pdf", "reason": "for the review"});
    env.relay("u1", &call).await;
    let told = env.told().await;
    let data = &told["result"]["data"];
    assert_eq!(data["status"], "upload_ready");
    assert_eq!(
        (data["blob"].as_str(), data["download_url"].as_str()),
        (Some(SLOT), Some("https://rw.example/rewarden/blob/DOWNLOAD-SECRET"))
    );
    let next = data["next"].as_str().unwrap();
    assert!(next.contains("curl -T") && next.contains("asked on their phone"), "{next}");
    let slot = env.json_of("POST", "/rewarden/api/blobs").await.pop().unwrap();
    assert_eq!(
        (slot["max_bytes"].as_u64(), &slot["purpose"], slot["request_id"].as_str()),
        (Some(1_100), &json!({"kind": "upload", "reason": "for the review"}), Some("u1"))
    );
    assert!(env.core.pending().await.unwrap().is_empty());

    // The file arrives: the server lists it once.
    let mut arrived = info(
        "blob-report-00000001",
        "c1",
        "x",
        1_000,
        &json!({"kind": "text", "head": "%PDF-1.7 hello",
        "truncated": true}),
    );
    arrived["purpose"] = json!({"kind": "upload", "reason": "for the review"});
    arrived["name"] = json!("report.pdf");
    let mut early = arrived.clone();
    early["id"] = json!("blob-still-waiting-01");
    early["state"] = json!("waiting");
    env.pending(&json!({"requests": [], "pairings": [], "blobs": [arrived, early]})).await;
    let items = env.core.sync(0).await.unwrap();
    assert_eq!(items.len(), 1, "only an uploaded file waits for a decision");
    assert_eq!(
        (items[0].kind, items[0].id.as_str(), items[0].action.as_str()),
        (PendingKind::Blob, "blob-report-00000001", "upload")
    );
    assert_eq!(items[0].title, "Claude wants to share a file: report.pdf");
    assert_eq!(env.notifier.pending.lock().unwrap().last().unwrap().kind, PendingKind::Blob);
    let view = env.core.blob_view("blob-report-00000001".into()).await.unwrap();
    assert_eq!(
        (view.purpose.as_str(), view.preview_text.as_deref(), view.size, view.connection_label.as_str()),
        ("for the review", Some("%PDF-1.7 hello"), 1_000, "Claude")
    );

    env.core.answer_blob("blob-report-00000001".into(), true).await.unwrap();
    assert_eq!(
        env.json_of("POST", "/rewarden/api/blobs/blob-report-00000001/decision").await,
        [json!({"v": 1, "approved": true})]
    );
    assert!(env.core.pending().await.unwrap().is_empty());
    assert!(env.notifier.resolved.lock().unwrap().contains(&"blob-report-00000001".to_owned()));
    let entry = &env.core.activity(1).await.unwrap()[0];
    assert_eq!(
        (entry.action.as_str(), entry.outcome.as_str(), entry.service.as_str()),
        ("upload", "released", "files")
    );
    // Listed again by mistake: it was decided already.
    env.pending(&json!({"requests": [], "pairings": [], "blobs": [arrived]})).await;
    assert!(env.core.sync(0).await.unwrap().is_empty());

    // By push, refused.
    let mut pushed = arrived.clone();
    pushed["id"] = json!("blob-pushed-00000001");
    env.holds(&pushed).await;
    env.core.handle_push("blob".into(), "blob-pushed-00000001".into()).await.unwrap();
    assert_eq!(env.core.pending().await.unwrap()[0].kind, PendingKind::Blob);
    env.core.answer_blob("blob-pushed-00000001".into(), false).await.unwrap();
    assert_eq!(
        env.json_of("POST", "/rewarden/api/blobs/blob-pushed-00000001/decision").await,
        [json!({"v": 1, "approved": false})]
    );
    assert_eq!(env.core.activity(1).await.unwrap()[0].outcome, "denied");
    assert!(env.core.blob_view("blob-pushed-00000001".into()).await.is_err());
    assert!(env.core.handle_push("blob".into(), "../../x".into()).await.is_err());
}

// ---- large results --------------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_large_release_asset_is_fetched_by_the_server_only_once_released_and_handed_over_as_a_link() {
    let env = env().await;
    env.gh(
        "GET",
        "/repos/octo/cat/releases/assets/3",
        &json!({"id": 3, "name": "big.iso", "size": 5_000_000,
        "content_type": "application/octet-stream", "state": "uploaded", "download_count": 1,
        "browser_download_url": "https://github.com/x"}),
    )
    .await;
    let (id, parked) = env.ask("github", "release_asset_download", &json!({"repo": "octo/cat", "asset_id": 3})).await;
    assert!(parked);
    let view = env.core.approval_view(id.clone()).await.unwrap();
    assert!(env.requests("POST", "/blobs/fetch").await.is_empty(), "nothing is fetched before the user decides");
    assert!(!format!("{view:?}").contains("_rewarden"));

    // Released, and the read of octo/cat remembered.
    let ids: Vec<String> = view.messages.iter().map(|m| m.id.clone()).collect();
    env.core.approve(id.clone(), choice(&ids, read_grant(&["octo/cat"]))).await.unwrap();
    let fetch = env.json_of("POST", "/rewarden/api/blobs/fetch").await.pop().unwrap();
    assert_eq!(
        (fetch["connection_id"].as_str(), fetch["request_id"].as_str(), fetch["name"].as_str()),
        (Some("c1"), Some(id.as_str()), Some("big.iso"))
    );
    assert_eq!(fetch["url"], format!("{}/repos/octo/cat/releases/assets/3", env.github.uri()));
    assert_eq!(fetch["max_bytes"], 5_000_000);
    assert_eq!(header(&fetch, "Authorization"), Some(format!("Bearer {GH_TOKEN}").as_str()));
    assert_eq!(header(&fetch, "Accept"), Some("application/octet-stream"));
    let item = env.told().await["result"]["data"]["items"][0].clone();
    assert_eq!(item["download_url"], "https://rw.example/rewarden/blob/DOWNLOAD-blob-fetched-0000001");
    assert_eq!((item["encoding"].as_str(), item["sha256"].as_str()), (Some("link"), Some("cd".repeat(32).as_str())));
    assert!(item.get("_rewarden_deliver").is_none() && item.get("content_base64").is_none(), "{item}");

    // The next download is covered by the permission: fetched and answered at once.
    let (_, parked) = env.ask("github", "release_asset_download", &json!({"repo": "octo/cat", "asset_id": 3})).await;
    assert!(!parked);
    assert_eq!(env.requests("POST", "/blobs/fetch").await.len(), 2);
    assert_eq!(env.told().await["result"]["data"]["items"][0]["encoding"], "link");
}

#[tokio::test]
async fn small_results_stay_inline() {
    let env = env().await;
    let small = b"tiny".to_vec();
    env.gh("GET", "/repos/octo/cat/releases/assets/4", &json!({"id": 4, "name": "t.txt", "size": small.len()})).await;
    Mock::given(method("GET"))
        .and(path("/repos/octo/cat/releases/assets/4"))
        .and(wiremock::matchers::header("accept", "application/octet-stream"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", format!("{}/dl/t.txt", env.github.uri())))
        .with_priority(1)
        .mount(&env.github)
        .await;
    Mock::given(method("GET"))
        .and(path("/dl/t.txt"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(small))
        .mount(&env.github)
        .await;
    let (id, _) = env.ask("github", "release_asset_download", &json!({"repo": "octo/cat", "asset_id": 4})).await;
    let view = env.core.approval_view(id.clone()).await.unwrap();
    let ids: Vec<String> = view.messages.iter().map(|m| m.id.clone()).collect();
    env.core.approve(id, choice(&ids, None)).await.unwrap();
    assert_eq!(env.told().await["result"]["data"]["items"][0]["text"], "tiny");
    assert!(env.requests("POST", "/blobs/fetch").await.is_empty());
}

#[tokio::test]
async fn a_large_vault_attachment_is_decrypted_on_the_phone_and_handed_over_as_a_link() {
    let env = env().await;
    let (id, parked) = env.ask("vault", "attachment_get", &json!({"item": "git", "attachment": "big.bin"})).await;
    assert!(parked);
    let view = env.core.approval_view(id.clone()).await.unwrap();
    assert!(view.messages[0].sensitive);
    assert!(env.requests("PUT", "/blobs/output").await.is_empty(), "nothing leaves the phone before the user decides");
    let ids: Vec<String> = view.messages.iter().map(|m| m.id.clone()).collect();
    env.core.approve(id, choice(&ids, None)).await.unwrap();

    let output = env.requests("PUT", "/rewarden/api/blobs/output").await.pop().expect("uploaded once released");
    assert_eq!(output.body, big_attachment(), "the decrypted file");
    let query: Vec<(String, String)> =
        output.url.query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect();
    assert!(
        query.contains(&("connection_id".into(), "c1".into())) && query.contains(&("name".into(), "big.bin".into()))
    );
    let item = env.told().await["result"]["data"]["items"][0].clone();
    assert_eq!(item["download_url"], "https://rw.example/rewarden/blob/DOWNLOAD-blob-output-00000001");
    assert_eq!(item["encoding"], "link");
    assert!(!item.to_string().contains(&BASE64.encode(&big_attachment()[..64])), "the content is not inline");
}

#[tokio::test]
async fn a_long_job_log_keeps_its_end_inline_and_the_whole_log_comes_as_a_link() {
    let env = env().await;
    let log = (0..6_000).fold(String::new(), |mut log, n| {
        std::fmt::Write::write_fmt(&mut log, format_args!("step {n:05} ok\n")).unwrap();
        log
    });
    Mock::given(method("GET"))
        .and(path("/repos/octo/cat/actions/jobs/5/logs"))
        .respond_with(ResponseTemplate::new(200).set_body_string(log))
        .mount(&env.github)
        .await;
    let (id, parked) = env.ask("github", "job_logs", &json!({"repo": "octo/cat", "job_id": 5})).await;
    assert!(parked);
    let view = env.core.approval_view(id.clone()).await.unwrap();
    let ids: Vec<String> = view.messages.iter().map(|m| m.id.clone()).collect();
    env.core.approve(id, choice(&ids, None)).await.unwrap();
    let item = env.told().await["result"]["data"]["items"][0].clone();
    assert!(item["text"].as_str().unwrap().trim_end().ends_with("step 05999 ok"), "the end of the log is inline");
    assert_eq!((item["truncated"].as_bool(), item["encoding"].as_str()), (Some(true), Some("link")));
    let fetch = env.json_of("POST", "/rewarden/api/blobs/fetch").await.pop().unwrap();
    assert_eq!(fetch["url"], format!("{}/repos/octo/cat/actions/jobs/5/logs", env.github.uri()));
    assert_eq!(fetch["name"], "job-5.log");
}
