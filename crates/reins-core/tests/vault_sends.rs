//! Sends and the generator of the vault, against a mock Vaultwarden that serves a vault encrypted the way Bitwarden
//! clients do. What the phone writes is decrypted here with keys derived independently, and the links it returns are
//! opened the way another client would.

mod common;

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use common::{FakeGoogle, FakeKeys, RecordingNotifier};
use data_encoding::{BASE64, BASE64URL_NOPAD};
use reins_core::crypto::{Kdf, VaultKey, master_key};
use reins_core::text::parse_when;
use reins_core::{
    ApprovalChoice, CoreConfig, GoogleTokenProvider, GrantScopeChoice, Notifier, ReinsCore, StandingGrant,
};
use ring::{hkdf, pbkdf2};
use serde_json::{Value, json};
use wiremock::matchers::{method, path, path_regex};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

const PASSWORD: &str = "correct horse battery";
const EMAIL: &str = "me@example.com";
const WORDS: &str = include_str!("../src/connector/vault/eff_large_wordlist.txt");
const TEXT_ID: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const FILE_ID: &str = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
const NEW_ID: &str = "cccccccc-cccc-4ccc-8ccc-cccccccccccc";

struct Env {
    server: MockServer,
    core: Arc<ReinsCore>,
    _dir: tempfile::TempDir,
}

fn user_key() -> VaultKey {
    VaultKey::from_bytes(&[7u8; 64]).unwrap()
}

struct Len64;

impl hkdf::KeyType for Len64 {
    fn len(&self) -> usize {
        64
    }
}

/// The keys of a Send, written out again here on purpose (HKDF-SHA256, salt "bitwarden-send", info "send").
fn derive(send_key: &[u8]) -> VaultKey {
    let prk = hkdf::Salt::new(hkdf::HKDF_SHA256, b"bitwarden-send").extract(send_key);
    let mut out = [0u8; 64];
    prk.expand(&[b"send".as_slice()], Len64).unwrap().fill(&mut out).unwrap();
    VaultKey::from_bytes(&out).unwrap()
}

fn expected_password_hash(password: &str, send_key: &[u8]) -> String {
    let mut out = [0u8; 32];
    pbkdf2::derive(
        pbkdf2::PBKDF2_HMAC_SHA256,
        std::num::NonZeroU32::new(100_000).unwrap(),
        send_key,
        password.as_bytes(),
        &mut out,
    );
    BASE64.encode(&out)
}

fn access_id(uuid: &str) -> String {
    BASE64URL_NOPAD.encode(uuid::Uuid::parse_str(uuid).unwrap().as_bytes())
}

fn enc(key: &VaultKey, s: &str) -> Value {
    json!(key.encrypt(s.as_bytes()).unwrap())
}

/// A text Send the way the server lists it in `/api/sync`. The send key is 16 x `fill`.
fn text_send(id: &str, fill: u8, name: &str, text: &str, extra: &Value) -> Value {
    let raw = [fill; 16];
    let keys = derive(&raw);
    let mut send = json!({
        "id": id, "accessId": access_id(id), "type": 0, "name": enc(&keys, name), "notes": enc(&keys, "private notes"),
        "text": {"text": enc(&keys, text), "hidden": false}, "file": null,
        "key": user_key().encrypt(&raw).unwrap(),
        "maxAccessCount": null, "accessCount": 2, "password": null, "authType": 2, "disabled": false,
        "hideEmail": false, "revisionDate": "2026-09-01T10:00:00.000000Z", "expirationDate": null,
        "deletionDate": "2026-10-05T10:00:00.000000Z", "object": "send"
    });
    for (k, v) in extra.as_object().unwrap() {
        send[k] = v.clone();
    }
    send
}

fn file_send(id: &str, fill: u8, name: &str, file_name: &str) -> Value {
    let raw = [fill; 16];
    let keys = derive(&raw);
    json!({
        "id": id, "accessId": access_id(id), "type": 1, "name": enc(&keys, name), "notes": null, "text": null,
        "file": {"id": "fileid", "fileName": enc(&keys, file_name), "size": "1024", "sizeName": "1.00 KB"},
        "key": user_key().encrypt(&raw).unwrap(),
        "maxAccessCount": 5, "accessCount": 0, "password": "hash", "authType": 1, "disabled": true,
        "hideEmail": true, "revisionDate": "2026-09-02T10:00:00.000000Z", "expirationDate": "2026-10-01T10:00:00.000000Z",
        "deletionDate": "2026-10-09T10:00:00.000000Z", "object": "send"
    })
}

async fn env(sends: Vec<Value>) -> Env {
    let server = MockServer::start().await;
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
    let master = master_key(
        PASSWORD,
        EMAIL,
        Kdf::Pbkdf2 {
            iterations: 5000,
        },
    )
    .unwrap();
    let wrapped = master.stretch().encrypt(&user_key().to_bytes()).unwrap();
    Mock::given(method("GET"))
        .and(path("/api/sync"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "profile": {"key": wrapped}, "ciphers": [], "sends": sends})))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let google: Arc<dyn GoogleTokenProvider> = Arc::new(FakeGoogle::new());
    let notifier: Arc<dyn Notifier> = Arc::new(RecordingNotifier::default());
    let core = ReinsCore::with_connectors(
        dir.path().to_str().unwrap(),
        &FakeKeys,
        google,
        notifier,
        CoreConfig::default(),
        vec![],
    )
    .unwrap();
    common::mount_account_vault(&server, EMAIL, PASSWORD).await;
    core.login(server.uri(), EMAIL.to_owned(), PASSWORD.to_owned(), None).await.unwrap();
    core.add_token_account("vault".into(), PASSWORD.into()).await.unwrap();
    Env {
        server,
        core,
        _dir: dir,
    }
}

fn request(id: &str, op: &str, args: &Value) -> Value {
    json!({"v": 1, "id": id, "connection_id": "c1", "connection_label": "Claude", "created_at": 100,
           "call": {"tool": "connector", "service": "vault", "op": op, "args": args}})
}

async fn serve_pending(env: &Env, requests: &[Value]) {
    Mock::given(method("GET"))
        .and(path("/reins/api/pending"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"requests": requests, "pairings": []})))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&env.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/reins/api/pending"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"requests": [], "pairings": []})))
        .mount(&env.server)
        .await;
    Mock::given(method("POST"))
        .and(path_regex(r"^/reins/api/(requests|pairings)/[^/]+/response$"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&env.server)
        .await;
}

async fn answers(env: &Env) -> Vec<Value> {
    env.server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| {
            r.method.as_str() == "POST" && r.url.path().contains("/requests/") && r.url.path().ends_with("/response")
        })
        .map(|r| serde_json::from_slice(&r.body).unwrap())
        .collect()
}

fn choice(ids: &[&str], standing: Option<StandingGrant>) -> ApprovalChoice {
    ApprovalChoice {
        selected_message_ids: ids.iter().map(|s| (*s).to_owned()).collect(),
        standing,
    }
}

fn hour(resources: &[&str], classes: &[&str]) -> Option<StandingGrant> {
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

/// Sends the request, has the user approve it (nothing ticked: a write), and returns what the AI was told.
async fn approved(env: &Env, id: &str, op: &str, args: &Value) -> Value {
    serve_pending(env, &[request(id, op, args)]).await;
    assert_eq!(env.core.sync(0).await.unwrap().len(), 1, "{op} waits for the user");
    env.core.approve(id.to_owned(), choice(&[], None)).await.unwrap();
    answers(env).await.pop().unwrap()
}

/// The message the AI gets when the call fails before anything is asked.
async fn refused(env: &Env, id: &str, op: &str, args: &Value) -> String {
    serve_pending(env, &[request(id, op, args)]).await;
    assert!(env.core.sync(0).await.unwrap().is_empty(), "{op} must fail before the user is asked");
    let told = answers(env).await.pop().unwrap();
    assert_eq!(told["outcome"], "error", "{told}");
    told["message"].as_str().unwrap().to_owned()
}

async fn sent(env: &Env, verb: &str, url_path: &str) -> Vec<Request> {
    env.server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.method.as_str() == verb && r.url.path() == url_path)
        .collect()
}

fn body(request: &Request) -> Value {
    serde_json::from_slice(&request.body).unwrap()
}

fn unix_now() -> i64 {
    i64::try_from(SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()).unwrap()
}

async fn mock_created_text_send(env: &Env) {
    Mock::given(method("POST"))
        .and(path("/api/sends"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": NEW_ID, "accessId": access_id(NEW_ID), "type": 0, "object": "send",
            "expirationDate": null, "deletionDate": "2026-10-07T10:00:00.000000Z"})))
        .mount(&env.server)
        .await;
}

/// The send key in a link, and the id in it.
fn open_link(link: &str, server: &str) -> (String, Vec<u8>) {
    let rest = link.strip_prefix(&format!("{server}/#/send/")).unwrap_or_else(|| panic!("{link} is not on {server}"));
    let (id, key) = rest.split_once('/').unwrap();
    (id.to_owned(), BASE64URL_NOPAD.decode(key.as_bytes()).unwrap())
}

/// The send key the phone wrote into a request body, opened with the user key.
fn send_key_of(sent_body: &Value) -> Vec<u8> {
    user_key().decrypt(sent_body["key"].as_str().unwrap()).unwrap().to_vec()
}

// ---- multipart ----

fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    hay.get(from..)?.windows(needle.len()).position(|w| w == needle).map(|p| p + from)
}

/// The parts of a multipart body: (headers, data).
fn multipart(request: &Request) -> Vec<(String, Vec<u8>)> {
    let content_type = request.headers.get("content-type").unwrap().to_str().unwrap().to_owned();
    let boundary = content_type.split("boundary=").nth(1).unwrap().trim_matches('"').to_owned();
    let delimiter = format!("--{boundary}").into_bytes();
    let mut parts = Vec::new();
    let mut at = find(&request.body, &delimiter, 0).unwrap();
    loop {
        let start = at + delimiter.len();
        if request.body[start..].starts_with(b"--") {
            break;
        }
        let end = find(&request.body, &delimiter, start).unwrap();
        let part = &request.body[start + 2..end - 2]; // the line breaks around the delimiters
        let split = find(part, b"\r\n\r\n", 0).unwrap();
        parts.push((String::from_utf8(part[..split].to_vec()).unwrap(), part[split + 4..].to_vec()));
        at = end;
    }
    parts
}

fn filename_of(headers: &str) -> String {
    let after = headers.split("filename=\"").nth(1).unwrap();
    after[..after.find('"').unwrap()].to_owned()
}

/// `0x02 || iv || mac || ciphertext` back to an EncString, then decrypted.
fn open_buffer(keys: &VaultKey, data: &[u8]) -> Vec<u8> {
    assert_eq!(data[0], 2, "AesCbc256_HmacSha256");
    let (iv, rest) = data[1..].split_at(16);
    let (mac, cipher) = rest.split_at(32);
    let sealed = format!("2.{}|{}|{}", BASE64.encode(iv), BASE64.encode(cipher), BASE64.encode(mac));
    keys.decrypt(&sealed).unwrap().to_vec()
}

// ================= creating =================

#[tokio::test]
async fn a_text_send_is_encrypted_with_its_own_key_and_the_link_opens_it_for_anyone() {
    let env = env(vec![]).await;
    mock_created_text_send(&env).await;
    let before = unix_now();
    let args = json!({"type": "text", "name": "Wifi", "notes": "for guests", "text": "the code is 4711", "hidden": true,
        "password": "open sesame", "max_access_count": 3, "expires_in_hours": 12, "delete_in_days": 10,
        "disabled": false, "hide_email": true});
    serve_pending(&env, &[request("r1", "send_create", &args)]).await;
    env.core.sync(0).await.unwrap();

    // What the user sees: the summary, the limits, never the password or a hidden text.
    let view = env.core.approval_view("r1".to_owned()).await.unwrap();
    let shown = view.preview.join("\n");
    assert_eq!(view.class, "sends");
    assert!(shown.starts_with("Create a text Send \"Wifi\""), "{shown}");
    assert!(shown.contains("Opens at most 3 times") && shown.contains("Protected by a password"), "{shown}");
    assert!(shown.contains("Hides your email"), "{shown}");
    assert!(!shown.contains("open sesame") && !shown.contains("4711"), "hidden text and password stay hidden: {shown}");
    assert!(!view.no_standing);
    assert!(env.server.received_requests().await.unwrap().iter().all(|r| r.url.path() != "/api/sends"));

    env.core.approve("r1".to_owned(), choice(&[], None)).await.unwrap();
    let after = unix_now();

    // The request the server received.
    let requests = sent(&env, "POST", "/api/sends").await;
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].headers.get("authorization").unwrap().to_str().unwrap(),
        format!("Bearer {}", common::account_token()).as_str()
    );
    let wire = body(&requests[0]);
    let raw = send_key_of(&wire);
    assert_eq!(raw.len(), 16, "a 16-byte send key, wrapped with the user key");
    let keys = derive(&raw);
    let field = |v: &Value| keys.decrypt_text(v.as_str().unwrap()).unwrap().to_string();
    assert_eq!(wire["type"], 0);
    assert_eq!(field(&wire["name"]), "Wifi");
    assert_eq!(field(&wire["notes"]), "for guests");
    assert_eq!(field(&wire["text"]["text"]), "the code is 4711");
    assert_eq!(wire["text"]["hidden"], true);
    assert!(wire["file"].is_null() && wire["fileLength"].is_null());
    assert_eq!(wire["password"], expected_password_hash("open sesame", &raw));
    assert_eq!(wire["maxAccessCount"], 3);
    assert_eq!((wire["disabled"].clone(), wire["hideEmail"].clone()), (json!(false), json!(true)));
    let expires = parse_when(wire["expirationDate"].as_str().unwrap()).unwrap();
    assert!((before + 12 * 3_600..=after + 12 * 3_600).contains(&expires), "12 hours from now");
    let deletes = parse_when(wire["deletionDate"].as_str().unwrap()).unwrap();
    assert!((before + 10 * 86_400..=after + 10 * 86_400).contains(&deletes), "10 days from now");
    let raw_text = String::from_utf8_lossy(&requests[0].body).into_owned();
    for plain in ["Wifi", "guests", "4711", "open sesame"] {
        assert!(!raw_text.contains(plain), "{plain} must not travel in the clear");
    }

    // What the AI is told: the link, which opens the Send for another client that only has the link.
    let told = answers(&env).await.pop().unwrap();
    let data = &told["result"]["data"];
    assert_eq!(
        (data["created"].clone(), data["id"].clone(), data["password_protected"].clone()),
        (json!(true), json!(NEW_ID), json!(true))
    );
    let (link_id, link_key) = open_link(data["link"].as_str().unwrap(), &env.server.uri());
    assert_eq!(link_id, access_id(NEW_ID));
    assert_eq!(link_key, raw);
    let as_recipient = derive(&link_key);
    assert_eq!(as_recipient.decrypt_text(wire["text"]["text"].as_str().unwrap()).unwrap().as_str(), "the code is 4711");
    assert_eq!(data["deletion_date"], "2026-10-07T10:00:00Z");

    // The log keeps what happened, not the link or the text.
    let log = format!("{:?}", env.core.activity(5).await.unwrap());
    assert!(!log.contains("4711") && !log.contains("open sesame") && !log.contains("#/send/"), "{log}");
    assert!(log.contains("Create a text Send"), "{log}");
}

#[tokio::test]
async fn defaults_a_week_to_live_no_password_and_a_fresh_key_every_time() {
    let env = env(vec![]).await;
    mock_created_text_send(&env).await;
    let before = unix_now();
    let first = approved(&env, "r1", "send_create", &json!({"type": "text", "name": "A", "text": "one"})).await;
    approved(&env, "r2", "send_create", &json!({"type": "text", "name": "A", "text": "one"})).await;
    let requests = sent(&env, "POST", "/api/sends").await;
    let (a, b) = (body(&requests[0]), body(&requests[1]));
    assert_ne!(send_key_of(&a), send_key_of(&b), "each Send gets its own key");
    assert_ne!(a["name"], b["name"], "and its own IVs");
    assert!(a["password"].is_null() && a["notes"].is_null() && a["expirationDate"].is_null());
    assert!(a["maxAccessCount"].is_null());
    assert_eq!(
        (a["disabled"].clone(), a["hideEmail"].clone(), a["text"]["hidden"].clone()),
        (json!(false), json!(false), json!(false))
    );
    let deletes = parse_when(a["deletionDate"].as_str().unwrap()).unwrap();
    assert!((before + 7 * 86_400 - 5..=unix_now() + 7 * 86_400).contains(&deletes));
    assert_eq!(first["result"]["data"]["password_protected"], false);
}

#[tokio::test]
async fn a_file_send_is_created_then_its_encrypted_content_uploaded() {
    let env = env(vec![]).await;
    Mock::given(method("POST"))
        .and(path("/api/sends/file/v2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "fileUploadType": 0, "object": "send-fileUpload", "url": format!("/sends/{NEW_ID}/file/FILE123"),
            "sendResponse": {"id": NEW_ID, "accessId": access_id(NEW_ID), "type": 1, "expirationDate": null,
                             "deletionDate": "2026-10-07T10:00:00.000000Z"}})))
        .mount(&env.server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("/api/sends/{NEW_ID}/file/FILE123")))
        .respond_with(ResponseTemplate::new(200))
        .mount(&env.server)
        .await;
    let content: Vec<u8> = (0..=255u8).cycle().take(10_000).collect();
    let args = json!({"type": "file", "name": "Report", "file_name": "report/2026.bin",
        "content_base64": BASE64.encode(&content), "password": "pw", "max_access_count": 1});
    serve_pending(&env, &[request("r1", "send_create", &args)]).await;
    env.core.sync(0).await.unwrap();
    let shown = env.core.approval_view("r1".to_owned()).await.unwrap().preview.join("\n");
    assert!(shown.starts_with("Create a file Send \"Report\" for report/2026.bin (9.8 KB)"), "{shown}");
    env.core.approve("r1".to_owned(), choice(&[], None)).await.unwrap();

    let step1 = sent(&env, "POST", "/api/sends/file/v2").await;
    assert_eq!(step1.len(), 1);
    let wire = body(&step1[0]);
    let raw = send_key_of(&wire);
    let keys = derive(&raw);
    assert_eq!(wire["type"], 1);
    assert_eq!(keys.decrypt_text(wire["name"].as_str().unwrap()).unwrap().as_str(), "Report");
    let encrypted_name = wire["file"]["fileName"].as_str().unwrap();
    assert_eq!(keys.decrypt_text(encrypted_name).unwrap().as_str(), "report/2026.bin");
    assert!(wire["text"].is_null());
    assert_eq!(wire["password"], expected_password_hash("pw", &raw));
    assert_eq!(wire["maxAccessCount"], 1);

    let uploads = sent(&env, "POST", &format!("/api/sends/{NEW_ID}/file/FILE123")).await;
    assert_eq!(uploads.len(), 1, "one upload");
    assert_eq!(
        uploads[0].headers.get("authorization").unwrap().to_str().unwrap(),
        format!("Bearer {}", common::account_token()).as_str()
    );
    let parts = multipart(&uploads[0]);
    assert_eq!(parts.len(), 1);
    let (headers, data) = &parts[0];
    assert!(headers.contains("name=\"data\""), "{headers}");
    assert_eq!(filename_of(headers), encrypted_name, "the part is named like the encrypted file name, byte for byte");
    assert_eq!(wire["fileLength"], data.len(), "the server checks the announced size against the upload");
    assert_eq!(open_buffer(&keys, data), content, "the recipient recovers the file");
    assert!(find(data, &content[..64], 0).is_none(), "the file is not sent in the clear");

    let told = answers(&env).await.pop().unwrap();
    let link = told["result"]["data"]["link"].as_str().unwrap();
    let (id, key) = open_link(link, &env.server.uri());
    assert_eq!((id, key), (access_id(NEW_ID), raw));
    assert_eq!(told["result"]["data"]["type"], "file");
}

#[tokio::test]
async fn a_failed_upload_deletes_the_empty_send_and_tells_the_ai() {
    let env = env(vec![]).await;
    Mock::given(method("POST"))
        .and(path("/api/sends/file/v2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "fileUploadType": 0, "url": format!("/sends/{NEW_ID}/file/F1"),
            "sendResponse": {"id": NEW_ID, "accessId": access_id(NEW_ID)}})))
        .mount(&env.server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("/api/sends/{NEW_ID}/file/F1")))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({"message": "Send file size does not match."})))
        .mount(&env.server)
        .await;
    Mock::given(method("DELETE"))
        .and(path(format!("/api/sends/{NEW_ID}")))
        .respond_with(ResponseTemplate::new(200))
        .mount(&env.server)
        .await;
    let args = json!({"type": "file", "name": "R", "file_name": "a.txt", "content_base64": BASE64.encode(b"hello")});
    serve_pending(&env, &[request("r1", "send_create", &args)]).await;
    env.core.sync(0).await.unwrap();
    assert!(env.core.approve("r1".to_owned(), choice(&[], None)).await.is_err());
    assert_eq!(
        sent(&env, "DELETE", &format!("/api/sends/{NEW_ID}")).await.len(),
        1,
        "no half-made Send is left behind"
    );
}

#[tokio::test]
async fn a_server_that_keeps_files_elsewhere_or_answers_oddly_is_not_followed() {
    let env = env(vec![]).await;
    Mock::given(method("POST"))
        .and(path("/api/sends/file/v2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "fileUploadType": 1, "url": "https://evil.example/upload",
            "sendResponse": {"id": NEW_ID, "accessId": access_id(NEW_ID)}})))
        .mount(&env.server)
        .await;
    Mock::given(method("DELETE"))
        .and(path(format!("/api/sends/{NEW_ID}")))
        .respond_with(ResponseTemplate::new(200))
        .mount(&env.server)
        .await;
    let args = json!({"type": "file", "name": "R", "file_name": "a.txt", "content_base64": BASE64.encode(b"hello")});
    serve_pending(&env, &[request("r1", "send_create", &args)]).await;
    env.core.sync(0).await.unwrap();
    assert!(env.core.approve("r1".to_owned(), choice(&[], None)).await.is_err());
    let all = env.server.received_requests().await.unwrap();
    assert!(all.iter().all(|r| r.url.host_str() != Some("evil.example")));
    assert!(all.iter().all(|r| !r.url.path().contains("/file/") || r.url.path() == "/api/sends/file/v2"));
}

#[tokio::test]
async fn bad_arguments_are_explained_before_the_user_is_asked() {
    let env = env(vec![]).await;
    let long_text = "x".repeat(400);
    let big = BASE64.encode(&vec![7u8; 2 * 1024 * 1024 + 1]);
    let ok_file = BASE64.encode(b"hello");
    for (id, args, needle) in [
        ("e1", json!({"type": "text", "name": "N", "text": "t", "deletion_date": "2099-01-01"}), "31 days at most"),
        (
            "e2",
            json!({"type": "text", "name": "N", "text": "t", "deletion_date": "2020-01-01"}),
            "must be in the future",
        ),
        (
            "e3",
            json!({"type": "text", "name": "N", "text": "t", "deletion_date": "2026-10-05T10:00:00"}),
            "needs a time zone",
        ),
        ("e4", json!({"type": "text", "name": "N", "text": "t", "deletion_date": "soon"}), "not a date"),
        (
            "e5",
            json!({"type": "text", "name": "N", "text": "t", "delete_in_days": 3, "deletion_date": "2026-10-05"}),
            "not both",
        ),
        (
            "e6",
            json!({"type": "text", "name": "N", "text": "t", "expires_in_hours": 5, "expiration_date": "2026-10-05"}),
            "not both",
        ),
        ("e7", json!({"type": "text", "name": "N", "text": "t", "expiration_date": "2020-01-01"}), "in the future"),
        (
            "e8",
            json!({"type": "text", "name": "N", "text": "t", "delete_in_days": 1, "expires_in_hours": 48}),
            "deleted before it expires",
        ),
        ("e9", json!({"type": "text", "name": "N"}), "needs `text`"),
        ("e10", json!({"type": "text", "name": "N", "text": "t", "file_name": "a"}), "belong to a file Send"),
        ("e11", json!({"type": "file", "name": "N", "text": "t"}), "belong to a text Send"),
        (
            "e13",
            json!({"type": "file", "name": "N", "file_name": "a", "content_base64": "not base64!!"}),
            "not valid base64",
        ),
        ("e14", json!({"type": "file", "name": "N", "file_name": "a", "content_base64": big}), "larger than 2 MB"),
        (
            "e15",
            json!({"type": "file", "name": "N", "file_name": "a", "content_base64": ""}),
            "needs `file_name` and `content_base64`",
        ),
        ("e16", json!({"type": "file", "name": "N", "file_name": "a", "content_base64": "\n"}), "empty"),
    ] {
        let message = refused(&env, id, "send_create", &args).await;
        assert!(message.contains(needle), "{id}: {message}");
    }
    assert!(
        sent(&env, "POST", "/api/sends").await.is_empty() && sent(&env, "POST", "/api/sends/file/v2").await.is_empty()
    );
    // A file Send without its content is told where to upload the file (see tests/blobs.rs), not refused.
    Mock::given(method("POST"))
        .and(path("/reins/api/blobs"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "blob-for-a-send-0001",
            "upload_url": "https://rw.example/reins/blob/up", "expires_at": 4_000_000_000_i64})))
        .mount(&env.server)
        .await;
    let args = json!({"type": "file", "name": "N", "file_name": "a"});
    serve_pending(&env, &[request("e12", "send_create", &args)]).await;
    assert!(env.core.sync(0).await.unwrap().is_empty(), "answered at once");
    let told = answers(&env).await.pop().unwrap();
    assert_eq!(told["result"]["data"]["status"], "upload_required", "{told}");

    // The edge of the file limit is accepted, and unpadded or wrapped base64 too.
    let exactly = BASE64.encode(&vec![7u8; 2 * 1024 * 1024]);
    serve_pending(
        &env,
        &[request(
            "ok1",
            "send_create",
            &json!({"type": "file", "name": "N", "file_name": "a", "content_base64": exactly}),
        )],
    )
    .await;
    assert_eq!(env.core.sync(0).await.unwrap().len(), 1);
    let unpadded = ok_file.trim_end_matches('=').to_owned();
    serve_pending(
        &env,
        &[request(
            "ok2",
            "send_create",
            &json!({"type": "file", "name": "N", "file_name": "a", "content_base64": unpadded}),
        )],
    )
    .await;
    assert_eq!(env.core.sync(0).await.unwrap().len(), 2);

    // A long text is shown in part only.
    serve_pending(&env, &[request("ok3", "send_create", &json!({"type": "text", "name": "N", "text": long_text}))])
        .await;
    env.core.sync(0).await.unwrap();
    let shown = env.core.approval_view("ok3".to_owned()).await.unwrap().preview.join("\n");
    assert!(shown.contains("(400 characters)") && shown.contains("100 more characters"), "{shown}");
    assert!(!shown.contains(&"x".repeat(301)), "at most 300 characters of the text");
}

#[tokio::test]
async fn the_server_refusing_is_passed_on_in_its_own_words_and_nothing_is_claimed() {
    let env = env(vec![]).await;
    Mock::given(method("POST"))
        .and(path("/api/sends"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "message": "Due to an Enterprise Policy, you are only able to delete an existing Send."})))
        .mount(&env.server)
        .await;
    let args = json!({"type": "text", "name": "N", "text": "secret-text-123"});
    serve_pending(&env, &[request("r1", "send_create", &args)]).await;
    env.core.sync(0).await.unwrap();
    let error = env.core.approve("r1".to_owned(), choice(&[], None)).await.unwrap_err();
    assert!(error.to_string().contains("Enterprise Policy"), "{error}");
    assert!(!error.to_string().contains("secret-text-123"));
    assert_eq!(env.core.pending().await.unwrap().len(), 1, "still waiting: the AI was not told it worked");
}

#[tokio::test]
async fn server_errors_map_to_messages_the_ai_can_act_on() {
    let env = env(vec![text_send(TEXT_ID, 1, "Old", "body", &json!({}))]).await;
    Mock::given(method("DELETE"))
        .and(path(format!("/api/sends/{TEXT_ID}")))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({"message": "Send not found"})))
        .mount(&env.server)
        .await;
    Mock::given(method("PUT"))
        .and(path(format!("/api/sends/{TEXT_ID}")))
        .respond_with(ResponseTemplate::new(500).set_body_string("panic: token=abc123"))
        .mount(&env.server)
        .await;
    serve_pending(&env, &[request("r1", "send_delete", &json!({"send": TEXT_ID}))]).await;
    env.core.sync(0).await.unwrap();
    let error = env.core.approve("r1".to_owned(), choice(&[], None)).await.unwrap_err().to_string();
    assert!(error.contains("Send not found"), "{error}");
    serve_pending(&env, &[request("r2", "send_update", &json!({"send": TEXT_ID, "name": "New"}))]).await;
    env.core.sync(0).await.unwrap();
    let error = env.core.approve("r2".to_owned(), choice(&[], None)).await.unwrap_err().to_string();
    assert!(!error.contains("abc123"), "the server's raw body is never passed on: {error}");
}

// ================= reading =================

#[tokio::test]
async fn the_list_shows_what_each_send_is_and_never_a_link_or_content() {
    let env = env(vec![
        text_send(TEXT_ID, 1, "Wifi code", "hunter2-in-a-send", &json!({"maxAccessCount": 5})),
        file_send(FILE_ID, 2, "Contract", "contract.pdf"),
    ])
    .await;
    serve_pending(&env, &[request("r1", "send_list", &json!({}))]).await;
    env.core.sync(0).await.unwrap();
    let view = env.core.approval_view("r1".to_owned()).await.unwrap();
    assert_eq!(view.action, "list");
    let names: Vec<&str> = view.messages.iter().map(|m| m.subject.as_str()).collect();
    assert_eq!(names, ["Contract", "Wifi code"]);
    assert!(view.messages.iter().all(|m| !m.sensitive));
    assert_eq!(
        view.messages[0].snippet,
        "File Send, opened 0 of 5 times, expires 2026-10-01, deleted 2026-10-09, password protected, hides your email, disabled"
    );
    assert_eq!(view.messages[1].snippet, "Text Send, opened 2 of 5 times, deleted 2026-10-05");
    env.core.approve("r1".to_owned(), choice(&[TEXT_ID, FILE_ID], None)).await.unwrap();
    let told = answers(&env).await.pop().unwrap();
    let everything = told.to_string();
    assert!(
        !everything.contains("hunter2") && !everything.contains("#/send/") && !everything.contains("contract.pdf"),
        "{everything}"
    );
    let items = told["result"]["data"]["items"].as_array().unwrap();
    let wifi = items.iter().find(|i| i["id"] == TEXT_ID).unwrap();
    assert_eq!(wifi["title"], "Wifi code");
    assert_eq!(
        (
            wifi["type"].clone(),
            wifi["access_count"].clone(),
            wifi["max_access_count"].clone(),
            wifi["password_protected"].clone()
        ),
        (json!("text"), json!(2), json!(5), json!(false))
    );
    assert_eq!(wifi["deletion_date"], "2026-10-05T10:00:00Z");
    let contract = items.iter().find(|i| i["id"] == FILE_ID).unwrap();
    assert_eq!(
        (
            contract["type"].clone(),
            contract["disabled"].clone(),
            contract["hide_email"].clone(),
            contract["password_protected"].clone()
        ),
        (json!("file"), json!(true), json!(true), json!(true))
    );

    serve_pending(&env, &[request("r2", "send_list", &json!({"query": "wifi"}))]).await;
    env.core.sync(0).await.unwrap();
    let view = env.core.approval_view("r2".to_owned()).await.unwrap();
    assert_eq!(view.messages.len(), 1);
    assert_eq!(
        view.resources.iter().map(|r| (r.id.clone(), r.wider)).collect::<Vec<_>>(),
        [(format!("sends/{TEXT_ID}"), false), ("sends".to_owned(), true)]
    );
}

#[tokio::test]
async fn a_send_is_read_with_its_link_only_after_approval_every_time_and_never_logged() {
    let env =
        env(vec![text_send(TEXT_ID, 9, "Wifi code", "the-real-content", &json!({"password": "x", "authType": 1}))])
            .await;
    serve_pending(&env, &[request("r1", "send_get", &json!({"send": TEXT_ID}))]).await;
    env.core.sync(0).await.unwrap();
    let view = env.core.approval_view("r1".to_owned()).await.unwrap();
    assert!(view.no_standing, "a Send's link is never remembered");
    assert!(view.messages[0].sensitive && !view.messages[0].covered_by_grant);
    assert_eq!(view.messages[0].snippet, "Link and content of the Send \"Wifi code\"");
    assert!(!format!("{view:?}").contains("the-real-content") && !format!("{view:?}").contains("#/send/"));
    assert!(env.core.approve("r1".to_owned(), ChoiceWithGrant::hour()).await.is_err(), "no standing permission for it");
    env.core.approve("r1".to_owned(), choice(&[TEXT_ID], None)).await.unwrap();
    let told = answers(&env).await.pop().unwrap();
    let item = &told["result"]["data"]["items"][0];
    let text = item["text"].as_str().unwrap();
    let expected_link =
        format!("{}/#/send/{}/{}", env.server.uri(), access_id(TEXT_ID), BASE64URL_NOPAD.encode(&[9u8; 16]));
    assert!(text.starts_with(&format!("Link: {expected_link}\nText:\nthe-real-content")), "{text}");
    assert!(text.contains("has a password"), "{text}");
    assert_eq!(item["password_protected"], true);
    let log = format!("{:?}", env.core.activity(5).await.unwrap());
    assert!(!log.contains("the-real-content") && !log.contains("#/send/"), "{log}");

    // A link from the answer is enough to decrypt the same Send elsewhere.
    let (_, key) = open_link(text.lines().next().unwrap().strip_prefix("Link: ").unwrap(), &env.server.uri());
    let sync_send = text_send(TEXT_ID, 9, "Wifi code", "the-real-content", &json!({}));
    assert_eq!(derive(&key).decrypt_text(sync_send["name"].as_str().unwrap()).unwrap().as_str(), "Wifi code");
}

/// A choice that asks to remember the permission.
struct ChoiceWithGrant;

impl ChoiceWithGrant {
    fn hour() -> ApprovalChoice {
        choice(&[TEXT_ID], hour(&[&format!("sends/{TEXT_ID}")], &[]))
    }
}

#[tokio::test]
async fn a_file_send_is_read_as_its_name_and_size_never_the_file() {
    let env = env(vec![file_send(FILE_ID, 4, "Contract", "contract.pdf")]).await;
    serve_pending(&env, &[request("r1", "send_get", &json!({"send": FILE_ID}))]).await;
    env.core.sync(0).await.unwrap();
    env.core.approve("r1".to_owned(), choice(&[FILE_ID], None)).await.unwrap();
    let told = answers(&env).await.pop().unwrap();
    let text = told["result"]["data"]["items"][0]["text"].as_str().unwrap().to_owned();
    assert!(text.contains("File: contract.pdf (1.00 KB)") && text.starts_with("Link: "), "{text}");
    assert_eq!(told["result"]["data"]["items"][0]["type"], "file");
}

#[tokio::test]
async fn unknown_or_malformed_ids_are_refused_without_a_request_to_the_server() {
    let env = env(vec![text_send(TEXT_ID, 1, "Old", "body", &json!({}))]).await;
    for (id, op, send) in [
        ("m1", "send_get", "nope"),
        ("m2", "send_delete", "nope"),
        ("m3", "send_remove_password", "nope"),
        ("m4", "send_delete", "../ciphers/1"),
        ("m5", "send_get", "a b"),
    ] {
        let message = refused(&env, id, op, &json!({"send": send})).await;
        assert!(message.contains("No Send with that id") || message.contains("must be the id"), "{id}: {message}");
    }
    assert_eq!(sent(&env, "DELETE", "/api/sends/nope").await.len(), 0);
}

// ================= changing =================

#[tokio::test]
async fn an_update_sends_the_whole_send_with_only_the_given_fields_changed() {
    let original = text_send(
        TEXT_ID,
        3,
        "Old name",
        "old text",
        &json!({"maxAccessCount": 4, "hideEmail": true, "disabled": false,
        "expirationDate": "2026-10-01T10:00:00.000000Z"}),
    );
    let env = env(vec![original.clone()]).await;
    Mock::given(method("PUT"))
        .and(path(format!("/api/sends/{TEXT_ID}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": TEXT_ID, "password": null, "authType": 2,
            "deletionDate": "2026-10-05T10:00:00.000000Z"})))
        .mount(&env.server)
        .await;
    let args = json!({"send": TEXT_ID, "name": "New name", "text": "new text", "password": "fresh", "disabled": true});
    serve_pending(&env, &[request("r1", "send_update", &args)]).await;
    env.core.sync(0).await.unwrap();
    let view = env.core.approval_view("r1".to_owned()).await.unwrap();
    let shown = view.preview.join("\n");
    assert_eq!((view.class.as_str(), view.resources[0].id.clone()), ("sends", format!("sends/{TEXT_ID}")));
    assert!(
        view.resources.iter().any(|r| r.id == "sends" && r.wider),
        "the Send is part of all Sends: {:?}",
        view.resources
    );
    assert!(shown.starts_with("Change the Send \"Old name\""), "{shown}");
    assert!(shown.contains("Name: \"Old name\" -> \"New name\""), "{shown}");
    assert!(shown.contains("Text: replaced by 8 characters: new text"), "{shown}");
    assert!(shown.contains("Password: set (not shown here)") && shown.contains("Disabled: no -> yes"), "{shown}");
    assert!(!shown.contains("fresh"), "{shown}");
    assert_eq!(sent(&env, "PUT", &format!("/api/sends/{TEXT_ID}")).await.len(), 0);
    env.core.approve("r1".to_owned(), choice(&[], None)).await.unwrap();

    let wire = body(&sent(&env, "PUT", &format!("/api/sends/{TEXT_ID}")).await[0]);
    let raw = [3u8; 16];
    let keys = derive(&raw);
    assert_eq!(wire["key"], original["key"], "the key, hence the link, stays");
    assert_eq!(keys.decrypt_text(wire["name"].as_str().unwrap()).unwrap().as_str(), "New name");
    assert_eq!(keys.decrypt_text(wire["text"]["text"].as_str().unwrap()).unwrap().as_str(), "new text");
    assert_eq!(wire["text"]["hidden"], false);
    assert_eq!(wire["notes"], original["notes"], "untouched fields go back exactly as they were");
    assert_eq!(wire["password"], expected_password_hash("fresh", &raw));
    assert_eq!(wire["disabled"], true);
    assert_eq!((wire["maxAccessCount"].clone(), wire["hideEmail"].clone()), (json!(4), json!(true)));
    assert_eq!(wire["expirationDate"], original["expirationDate"]);
    assert_eq!(wire["deletionDate"], original["deletionDate"]);
    assert_eq!(wire["type"], 0);
    let told = answers(&env).await.pop().unwrap();
    assert_eq!(told["result"]["data"]["updated"], true);
    assert!(!told.to_string().contains("fresh"));
}

#[tokio::test]
async fn an_update_can_clear_limits_move_dates_and_keep_the_password() {
    let original = text_send(
        TEXT_ID,
        3,
        "N",
        "t",
        &json!({"maxAccessCount": 4, "expirationDate": "2026-10-01T10:00:00.000000Z",
        "notes": null, "password": "h", "authType": 1}),
    );
    let env = env(vec![original.clone()]).await;
    Mock::given(method("PUT"))
        .and(path(format!("/api/sends/{TEXT_ID}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&env.server)
        .await;
    let before = unix_now();
    approved(&env, "r1", "send_update", &json!({"send": TEXT_ID, "max_access_count": 0, "expires_in_hours": 0, "delete_in_days": 20, "notes": "n2", "hidden": true, "hide_email": false})).await;
    let wire = body(&sent(&env, "PUT", &format!("/api/sends/{TEXT_ID}")).await[0]);
    let keys = derive(&[3u8; 16]);
    assert!(wire["maxAccessCount"].is_null() && wire["expirationDate"].is_null());
    assert!(wire["password"].is_null(), "no password field: the server keeps the one it has");
    assert_eq!(wire["text"]["text"], original["text"]["text"]);
    assert_eq!(wire["text"]["hidden"], true);
    assert_eq!(keys.decrypt_text(wire["notes"].as_str().unwrap()).unwrap().as_str(), "n2");
    assert_eq!(wire["hideEmail"], false);
    let deletes = parse_when(wire["deletionDate"].as_str().unwrap()).unwrap();
    assert!((before + 20 * 86_400..=unix_now() + 20 * 86_400).contains(&deletes));

    approved(&env, "r2", "send_update", &json!({"send": TEXT_ID, "notes": ""})).await;
    let wire = body(&sent(&env, "PUT", &format!("/api/sends/{TEXT_ID}")).await[1]);
    assert!(wire["notes"].is_null(), "an empty string removes the notes");
}

#[tokio::test]
async fn updates_that_make_no_sense_are_refused_before_the_user_is_asked() {
    let env = env(vec![text_send(TEXT_ID, 1, "T", "t", &json!({})), file_send(FILE_ID, 2, "F", "f.bin")]).await;
    for (id, args, needle) in [
        ("u1", json!({"send": TEXT_ID}), "at least one field"),
        ("u2", json!({"send": TEXT_ID, "text": ""}), "empty text"),
        ("u3", json!({"send": TEXT_ID, "password": ""}), "vault_send_remove_password"),
        ("u4", json!({"send": TEXT_ID, "deletion_date": "2099-01-01"}), "31 days at most"),
        ("u5", json!({"send": FILE_ID, "text": "x"}), "file Send has no text"),
        ("u6", json!({"send": FILE_ID, "hidden": true}), "file Send has no text"),
        ("u7", json!({"send": "unknown", "name": "x"}), "No Send with that id"),
    ] {
        let message = refused(&env, id, "send_update", &args).await;
        assert!(message.contains(needle), "{id}: {message}");
    }
    assert_eq!(sent(&env, "PUT", &format!("/api/sends/{TEXT_ID}")).await.len(), 0);
}

#[tokio::test]
async fn updating_a_file_send_leaves_the_file_alone() {
    let original = file_send(FILE_ID, 2, "F", "f.bin");
    let env = env(vec![original.clone()]).await;
    Mock::given(method("PUT"))
        .and(path(format!("/api/sends/{FILE_ID}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&env.server)
        .await;
    approved(&env, "r1", "send_update", &json!({"send": FILE_ID, "name": "Renamed", "disabled": false})).await;
    let wire = body(&sent(&env, "PUT", &format!("/api/sends/{FILE_ID}")).await[0]);
    assert_eq!(wire["type"], 1);
    assert!(
        wire["file"].is_null() && wire["text"].is_null() && wire["fileLength"].is_null(),
        "the server ignores the file of an update"
    );
    assert_eq!(derive(&[2u8; 16]).decrypt_text(wire["name"].as_str().unwrap()).unwrap().as_str(), "Renamed");
    assert_eq!(wire["maxAccessCount"], 5);
    assert_eq!(wire["disabled"], false);
}

#[tokio::test]
async fn the_password_of_a_send_can_be_removed_but_only_if_it_has_one() {
    let env =
        env(vec![text_send(TEXT_ID, 1, "Plain", "t", &json!({})), file_send(FILE_ID, 2, "Locked", "f.bin")]).await;
    Mock::given(method("PUT"))
        .and(path(format!("/api/sends/{FILE_ID}/remove-password")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&env.server)
        .await;
    assert!(refused(&env, "r0", "send_remove_password", &json!({"send": TEXT_ID})).await.contains("no password"));
    serve_pending(&env, &[request("r1", "send_remove_password", &json!({"send": FILE_ID}))]).await;
    env.core.sync(0).await.unwrap();
    let view = env.core.approval_view("r1".to_owned()).await.unwrap();
    assert_eq!(view.class, "sends");
    assert_eq!(
        view.preview,
        ["Remove the password of the Send \"Locked\"", "Anyone with the link will be able to open it"]
    );
    assert_eq!(sent(&env, "PUT", &format!("/api/sends/{FILE_ID}/remove-password")).await.len(), 0);
    env.core.approve("r1".to_owned(), choice(&[], None)).await.unwrap();
    let requests = sent(&env, "PUT", &format!("/api/sends/{FILE_ID}/remove-password")).await;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].body.len(), 0);
    assert_eq!(answers(&env).await.pop().unwrap()["result"]["data"], json!({"password_removed": true, "id": FILE_ID}));
}

#[tokio::test]
async fn a_send_is_deleted_only_after_the_user_sees_which() {
    let env = env(vec![text_send(TEXT_ID, 1, "Wifi", "t", &json!({}))]).await;
    Mock::given(method("DELETE"))
        .and(path(format!("/api/sends/{TEXT_ID}")))
        .respond_with(ResponseTemplate::new(200))
        .mount(&env.server)
        .await;
    serve_pending(&env, &[request("r1", "send_delete", &json!({"send": TEXT_ID}))]).await;
    env.core.sync(0).await.unwrap();
    let view = env.core.approval_view("r1".to_owned()).await.unwrap();
    assert_eq!(view.preview[0], "Delete the Send \"Wifi\" for good");
    assert!(view.preview.iter().any(|l| l.contains("opened 2 times")));
    assert!(!view.no_standing, "deleting a Send is not once-only: a standing permission may cover it");
    assert_eq!(sent(&env, "DELETE", &format!("/api/sends/{TEXT_ID}")).await.len(), 0);
    env.core.approve("r1".to_owned(), choice(&[], None)).await.unwrap();
    assert_eq!(sent(&env, "DELETE", &format!("/api/sends/{TEXT_ID}")).await.len(), 1);
    assert_eq!(answers(&env).await.pop().unwrap()["result"]["data"], json!({"deleted": true, "id": TEXT_ID}));
}

// ================= permissions =================

#[tokio::test]
async fn a_permission_for_sends_covers_creating_and_changing_them_but_only_for_that_class() {
    let env = env(vec![text_send(TEXT_ID, 1, "Wifi", "t", &json!({}))]).await;
    mock_created_text_send(&env).await;
    Mock::given(method("DELETE"))
        .and(path(format!("/api/sends/{TEXT_ID}")))
        .respond_with(ResponseTemplate::new(200))
        .mount(&env.server)
        .await;
    serve_pending(&env, &[request("r1", "send_create", &json!({"type": "text", "name": "A", "text": "one"}))]).await;
    env.core.sync(0).await.unwrap();
    let view = env.core.approval_view("r1".to_owned()).await.unwrap();
    assert_eq!(view.resources.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["sends"]);
    assert!(view.classes.iter().any(|c| c.id == "sends"));
    env.core.approve("r1".to_owned(), choice(&[], hour(&["sends"], &["sends"]))).await.unwrap();
    assert_eq!(env.core.grants().await.unwrap().len(), 1);

    serve_pending(&env, &[request("r2", "send_create", &json!({"type": "text", "name": "B", "text": "two"}))]).await;
    assert!(env.core.sync(0).await.unwrap().is_empty(), "covered");
    assert_eq!(sent(&env, "POST", "/api/sends").await.len(), 2);
    assert!(answers(&env).await.pop().unwrap()["result"]["data"]["link"].as_str().is_some());
    serve_pending(&env, &[request("r3", "send_delete", &json!({"send": TEXT_ID}))]).await;
    assert!(env.core.sync(0).await.unwrap().is_empty(), "`sends` covers a Send");
    assert_eq!(sent(&env, "DELETE", &format!("/api/sends/{TEXT_ID}")).await.len(), 1);
    // A grant to read the list is not a grant to write.
    serve_pending(&env, &[request("r4", "send_list", &json!({}))]).await;
    assert_eq!(env.core.sync(0).await.unwrap().len(), 1, "writing is not reading");
}

#[tokio::test]
async fn a_permission_for_one_send_does_not_cover_another() {
    let env = env(vec![text_send(TEXT_ID, 1, "One", "t", &json!({})), file_send(FILE_ID, 2, "Two", "f")]).await;
    Mock::given(method("DELETE")).respond_with(ResponseTemplate::new(200)).mount(&env.server).await;
    serve_pending(&env, &[request("r1", "send_delete", &json!({"send": TEXT_ID}))]).await;
    env.core.sync(0).await.unwrap();
    env.core.approve("r1".to_owned(), choice(&[], hour(&[&format!("sends/{TEXT_ID}")], &["sends"]))).await.unwrap();
    serve_pending(&env, &[request("r2", "send_delete", &json!({"send": FILE_ID}))]).await;
    assert_eq!(env.core.sync(0).await.unwrap().len(), 1, "another Send needs its own decision");
}

// ================= the generator =================

/// The generator is asked once with a standing permission; after that every call is answered at once.
async fn generator(op: &str, args: &Value, env: &Env, n: usize) -> String {
    let id = format!("g{n}");
    serve_pending(env, &[request(&id, op, args)]).await;
    let pending = env.core.sync(0).await.unwrap();
    if !pending.is_empty() {
        env.core.approve(id.clone(), choice(&[item_id(op)], hour(&["generator"], &[]))).await.unwrap();
    }
    let told = answers(env).await.pop().unwrap();
    assert_eq!(told["outcome"], "result", "{told}");
    told["result"]["data"]["items"][0]["text"].as_str().unwrap().to_owned()
}

fn item_id(op: &str) -> &'static str {
    match op {
        "generate_password" => "password",
        "generate_passphrase" => "passphrase",
        "generate_username" => "username",
        _ => "check",
    }
}

static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(100_000);

async fn passwords(env: &Env, args: &Value, count: usize) -> Vec<String> {
    let mut out = Vec::new();
    for _ in 0..count {
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        out.push(generator("generate_password", args, env, n).await);
    }
    out
}

fn count(value: &str, set: &str) -> usize {
    value.chars().filter(|c| set.contains(*c)).count()
}

const UPPER: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
const LOWER: &str = "abcdefghijklmnopqrstuvwxyz";
const DIGITS: &str = "0123456789";
const SYMBOLS: &str = "!@#$%^&*()-_=+[]{};:,.?";

#[tokio::test]
async fn generated_passwords_have_the_length_and_classes_asked_for_and_the_minimums_always() {
    let env = env(vec![]).await;
    let defaults = passwords(&env, &json!({}), 15).await;
    for p in &defaults {
        assert_eq!(p.chars().count(), 16, "{p}");
        for set in [UPPER, LOWER, DIGITS, SYMBOLS] {
            assert!(count(p, set) >= 1, "every class shows up: {p}");
        }
        assert_eq!(count(p, &format!("{UPPER}{LOWER}{DIGITS}{SYMBOLS}")), 16);
    }
    assert!(defaults.iter().collect::<std::collections::BTreeSet<_>>().len() > 1, "random");

    for p in passwords(&env, &json!({"length": 5}), 10).await {
        assert_eq!(p.len(), 5);
        assert!([UPPER, LOWER, DIGITS, SYMBOLS].iter().all(|s| count(&p, s) >= 1), "{p}");
    }
    for p in passwords(&env, &json!({"length": 128, "symbols": false}), 5).await {
        assert_eq!(p.len(), 128);
        assert_eq!(count(&p, SYMBOLS), 0, "{p}");
        assert!([UPPER, LOWER, DIGITS].iter().all(|s| count(&p, s) >= 1));
    }
    for p in passwords(&env, &json!({"length": 20, "uppercase": false, "lowercase": false, "symbols": false}), 5).await
    {
        assert!(p.len() == 20 && p.chars().all(|c| c.is_ascii_digit()), "{p}");
    }
    let minimums = json!({"length": 30, "min_uppercase": 5, "min_lowercase": 6, "min_numbers": 7, "min_symbols": 8});
    for p in passwords(&env, &minimums, 15).await {
        assert!(
            count(&p, UPPER) >= 5 && count(&p, LOWER) >= 6 && count(&p, DIGITS) >= 7 && count(&p, SYMBOLS) >= 8,
            "{p}"
        );
        assert_eq!(p.len(), 30);
        // With the minimums adding up to the length, the counts are exact.
        let exact = json!({"length": 26, "min_uppercase": 5, "min_lowercase": 6, "min_numbers": 7, "min_symbols": 8});
        let q =
            generator("generate_password", &exact, &env, NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst)).await;
        assert_eq!((count(&q, UPPER), count(&q, LOWER), count(&q, DIGITS), count(&q, SYMBOLS)), (5, 6, 7, 8), "{q}");
    }
}

#[tokio::test]
async fn ambiguous_and_excluded_characters_never_appear() {
    let env = env(vec![]).await;
    for p in passwords(&env, &json!({"length": 128, "avoid_ambiguous": true}), 8).await {
        assert_eq!(count(&p, "Il1O0"), 0, "{p}");
    }
    for p in passwords(&env, &json!({"length": 128, "exclude": "aeiouAEIOU!@#", "symbols": true}), 8).await {
        assert_eq!(count(&p, "aeiouAEIOU!@#"), 0, "{p}");
        assert!(count(&p, SYMBOLS) >= 1);
    }
    // Every digit gets used when nothing is excluded, and 128 characters of digits show them all.
    let digits =
        passwords(&env, &json!({"length": 128, "uppercase": false, "lowercase": false, "symbols": false}), 4).await;
    let seen: std::collections::BTreeSet<char> = digits.iter().flat_map(|p| p.chars()).collect();
    assert_eq!(seen.len(), 10);
}

#[tokio::test]
async fn impossible_password_requests_are_explained() {
    let env = env(vec![]).await;
    for (n, args, needle) in [
        (1, json!({"uppercase": false, "lowercase": false, "numbers": false, "symbols": false}), "at least one"),
        (2, json!({"length": 10, "min_numbers": 6, "min_symbols": 5}), "add up to 11"),
        (3, json!({"numbers": false, "min_numbers": 2}), "needs `numbers`"),
        (
            4,
            json!({"uppercase": false, "lowercase": false, "symbols": false, "exclude": "0123456789"}),
            "Nothing is left",
        ),
    ] {
        let message = refused(&env, &format!("x{n}"), "generate_password", &args).await;
        assert!(message.contains(needle), "{n}: {message}");
    }
}

#[tokio::test]
async fn the_user_sees_that_a_password_was_generated_and_not_which_one() {
    let env = env(vec![]).await;
    serve_pending(&env, &[request("g1", "generate_password", &json!({"length": 24}))]).await;
    env.core.sync(0).await.unwrap();
    let view = env.core.approval_view("g1".to_owned()).await.unwrap();
    assert_eq!(view.resources[0].id, "generator");
    assert_eq!(view.messages[0].snippet, "Generated password: 24 characters (uppercase, lowercase, numbers, symbols)");
    assert!(!view.messages[0].sensitive, "not from the vault: a standing permission may cover the generator");
    env.core.approve("g1".to_owned(), choice(&["password"], None)).await.unwrap();
    let told = answers(&env).await.pop().unwrap();
    let value = told["result"]["data"]["items"][0]["text"].as_str().unwrap().to_owned();
    assert_eq!(value.len(), 24);
    let log = format!("{:?}", env.core.activity(5).await.unwrap());
    assert!(!log.contains(&value), "the log never keeps a generated value: {log}");
    assert!(log.contains("Generated password"));
}

#[tokio::test]
async fn passphrases_are_words_of_the_eff_list_with_the_separator_and_options_asked_for() {
    let env = env(vec![]).await;
    let words: std::collections::BTreeSet<&str> = WORDS.lines().map(|l| l.split_once('\t').unwrap().1).collect();
    assert_eq!(words.len(), 7776);
    let default = generator("generate_passphrase", &json!({}), &env, 1).await;
    assert!(default.split('-').count() >= 6, "6 words with '-' (some words contain a hyphen themselves): {default}");
    let mut seen = std::collections::BTreeSet::new();
    for n in 0..20 {
        let p = generator("generate_passphrase", &json!({"words": 4, "separator": " "}), &env, 10 + n).await;
        let list: Vec<&str> = p.split(' ').collect();
        assert_eq!(list.len(), 4, "{p}");
        assert!(list.iter().all(|w| words.contains(w)), "{p}");
        seen.insert(p);
    }
    assert!(seen.len() > 15, "random");
    let p = generator("generate_passphrase", &json!({"words": 20, "separator": ""}), &env, 100).await;
    assert!(p.len() >= 60 && p.chars().all(|c| c.is_ascii_lowercase() || c == '-'), "{p}");
    let p =
        generator("generate_passphrase", &json!({"words": 5, "separator": "_", "capitalize": true}), &env, 101).await;
    let list: Vec<&str> = p.split('_').collect();
    assert_eq!(list.len(), 5);
    for w in list {
        let mut chars = w.chars();
        let first = chars.next().unwrap();
        assert!(first.is_ascii_uppercase(), "{p}");
        assert!(words.contains(format!("{}{}", first.to_ascii_lowercase(), chars.as_str()).as_str()), "{p}");
    }
    for n in 0..10 {
        let p = generator(
            "generate_passphrase",
            &json!({"words": 3, "separator": "+", "include_number": true}),
            &env,
            200 + n,
        )
        .await;
        assert_eq!(p.chars().filter(char::is_ascii_digit).count(), 1, "exactly one digit: {p}");
        assert!(
            p.split('+').filter(|w| w.ends_with(|c: char| c.is_ascii_digit())).count() == 1,
            "at the end of one word: {p}"
        );
    }
    let message = refused(&env, "bad", "generate_passphrase", &json!({"separator": "a\nb"})).await;
    assert!(message.contains("separator"), "{message}");
}

#[tokio::test]
async fn every_word_of_the_list_can_come_up() {
    // 7776 words: draw enough to see well over half of them, which a biased or stuck generator would not.
    let env = env(vec![]).await;
    let mut seen = std::collections::BTreeSet::new();
    for n in 0..60 {
        let p = generator("generate_passphrase", &json!({"words": 20, "separator": " "}), &env, 300 + n).await;
        seen.extend(p.split(' ').map(str::to_owned));
    }
    assert!(seen.len() > 1_000, "{} distinct words in 1200 draws", seen.len());
}

#[tokio::test]
async fn usernames_are_a_word_a_plus_address_or_a_catch_all_address() {
    let env = env(vec![]).await;
    let words: std::collections::BTreeSet<&str> = WORDS.lines().map(|l| l.split_once('\t').unwrap().1).collect();
    let word = generator("generate_username", &json!({"type": "random_word"}), &env, 1).await;
    assert!(words.contains(word.as_str()), "{word}");
    let word = generator(
        "generate_username",
        &json!({"type": "random_word", "capitalize": true, "include_number": true}),
        &env,
        2,
    )
    .await;
    let (head, tail) = word.split_at(word.len() - 4);
    assert!(tail.chars().all(|c| c.is_ascii_digit()), "{word}");
    assert!(
        head.chars().next().unwrap().is_ascii_uppercase() && words.contains(head.to_lowercase().as_str()),
        "{word}"
    );

    let mut tags = std::collections::BTreeSet::new();
    for n in 0..10 {
        let plus = generator(
            "generate_username",
            &json!({"type": "plus_addressed_email", "email": "Anna.Smith@example.co.uk"}),
            &env,
            10 + n,
        )
        .await;
        let (local, domain) = plus.split_once('@').unwrap();
        let (name, tag) = local.split_once('+').unwrap();
        assert_eq!((name, domain), ("Anna.Smith", "example.co.uk"), "{plus}");
        assert!(tag.len() == 8 && tag.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()), "{plus}");
        tags.insert(tag.to_owned());
        let catch_all = generator(
            "generate_username",
            &json!({"type": "catch_all_email", "domain": "my-domain.org"}),
            &env,
            40 + n,
        )
        .await;
        let (tag, domain) = catch_all.split_once('@').unwrap();
        assert!(
            domain == "my-domain.org"
                && tag.len() == 8
                && tag.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()),
            "{catch_all}"
        );
        tags.insert(tag.to_owned());
    }
    assert!(tags.len() > 15, "random");

    for (n, args, needle) in [
        (1, json!({"type": "plus_addressed_email"}), "plain address"),
        (2, json!({"type": "plus_addressed_email", "email": "a+b@example.com"}), "plain address"),
        (3, json!({"type": "plus_addressed_email", "email": "a@localhost"}), "plain address"),
        (4, json!({"type": "plus_addressed_email", "email": "not an email"}), "plain address"),
        (5, json!({"type": "catch_all_email"}), "`domain`"),
        (6, json!({"type": "catch_all_email", "domain": "-bad-.com"}), "`domain`"),
        (7, json!({"type": "catch_all_email", "domain": "a@b.com"}), "`domain`"),
    ] {
        let message = refused(&env, &format!("bad{n}"), "generate_username", &args).await;
        assert!(message.contains(needle), "{n}: {message}");
    }
}

#[tokio::test]
async fn the_strength_check_rates_passwords_and_never_repeats_or_sends_them() {
    let env = env(vec![]).await;
    let calls_before = env.server.received_requests().await.unwrap().len();
    let weak = generator("generate_check_password", &json!({"password": "Password1!"}), &env, 1).await;
    assert!(weak.contains("Strength: very weak") && weak.contains("Very common password: yes"), "{weak}");
    let repetitive = generator("generate_check_password", &json!({"password": "aaaaaaaa"}), &env, 2).await;
    assert!(repetitive.contains("Strength: very weak") && repetitive.contains("Length: 8 characters"), "{repetitive}");
    let strong = generator("generate_check_password", &json!({"password": "vK9#mQ2$xL7!pR4&zN8@"}), &env, 3).await;
    assert!(
        strong.contains("Strength: very strong") && strong.contains("lowercase, uppercase, numbers, symbols"),
        "{strong}"
    );
    assert!(strong.contains("Very common password: no"), "{strong}");
    let mid = generator("generate_check_password", &json!({"password": "correcthorsebattery"}), &env, 4).await;
    assert!(mid.contains("Strength: strong") || mid.contains("Strength: reasonable"), "{mid}");
    for shown in [&weak, &repetitive, &strong, &mid] {
        for password in ["Password1!", "aaaaaaaa", "vK9#mQ2$xL7!pR4&zN8@", "correcthorsebattery"] {
            assert!(!shown.contains(password), "the answer repeats the password: {shown}");
        }
    }
    let told = answers(&env).await;
    assert!(!format!("{told:?}").contains("vK9#mQ2$xL7!pR4&zN8@"), "the password is not in what the phone answers");
    let log = format!("{:?}", env.core.activity(10).await.unwrap());
    assert!(!log.contains("vK9#mQ2") && !log.contains("Password1"), "{log}");
    let later = env.server.received_requests().await.unwrap();
    assert!(
        later[calls_before..].iter().all(|r| !r.url.path().starts_with("/api/")),
        "the generator does not touch the vault"
    );
}
