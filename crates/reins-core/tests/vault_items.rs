//! The vault items area against a mock Vaultwarden that serves a vault encrypted the way Bitwarden clients do (one item
//! with a key of its own) and records what the phone sends. Every write is checked by decrypting what was sent.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use aes::Aes256;
use cbc::cipher::block_padding::Pkcs7;
use cbc::cipher::{BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use common::{FakeGoogle, FakeKeys, RecordingNotifier};
use data_encoding::BASE64;
use reins_core::crypto::{Kdf, VaultKey, master_key};
use reins_core::{
    ApprovalChoice, ApprovalKind, ApprovalView, CoreConfig, GoogleTokenProvider, Notifier, ReinsCore, StandingGrant,
};
use reins_proto::connector::{ConnectorCall, Effect};
use ring::hmac;
use serde_json::{Map, Value, json};
use wiremock::matchers::{header_exists, method, path, path_regex};
use wiremock::{Match, Mock, MockServer, Request, ResponseTemplate};

const PASSWORD: &str = "correct horse battery";
const EMAIL: &str = "me@example.com";

/// Everything that must never appear outside `secret` items and previews' "set (N characters)" lines.
const SECRETS: &[&str] = &[
    "hunter2!",
    "s3cret",
    "4111111111111111",
    "737",
    "078-05-1120",
    "P1234567",
    "D9876543",
    "FAKEPRIVATEKEYMATERIAL",
    "4321",
    "deploy notes",
    "old-pw-1",
    "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ",
    "seed phrase words",
];

const PUBLIC_KEY: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIHj2DRUTdkNas40IazQKs/dKxpKmBL2To4E+7OtctO45 test@reins";
const FINGERPRINT: &str = "SHA256:LEmZRdeu5A7eN2mINUqQkbgDak8Mr5bWYY1PXxCWkNc";
const PRIVATE_KEY: &str =
    "-----BEGIN OPENSSH PRIVATE KEY-----\nFAKEPRIVATEKEYMATERIAL\n-----END OPENSSH PRIVATE KEY-----";

struct Env {
    server: MockServer,
    core: Arc<ReinsCore>,
    user: VaultKey,
    own: VaultKey,
    att_key: Vec<u8>,
    counter: AtomicU32,
    _dir: tempfile::TempDir,
}

fn t(key: &VaultKey, s: &str) -> Value {
    json!(key.encrypt(s.as_bytes()).unwrap())
}

fn dec(key: &VaultKey, v: &Value) -> String {
    key.decrypt_text(v.as_str().unwrap_or_else(|| panic!("not an EncString: {v}"))).unwrap().to_string()
}

fn cipher(key: &VaultKey, id: &str, kind: i64, name: &str) -> Value {
    json!({"id": id, "type": kind, "name": t(key, name), "notes": null, "key": null, "folderId": null,
        "favorite": false, "deletedDate": null, "archivedDate": null, "organizationId": null,
        "creationDate": "2026-01-01T00:00:00.000000Z", "revisionDate": "2026-02-02T10:00:00.000000Z",
        "reprompt": 0, "fields": [], "passwordHistory": [], "attachments": null,
        "login": null, "secureNote": null, "card": null, "identity": null, "sshKey": null})
}

/// Encrypts a file the way Bitwarden clients do: `0x02 || iv || mac || ciphertext` (an independent implementation).
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
    assert_eq!(blob[0], 2, "EncArrayBuffer type");
    let (iv, rest) = blob[1..].split_at(16);
    let (mac, ct) = rest.split_at(32);
    let mut signed = iv.to_vec();
    signed.extend_from_slice(ct);
    hmac::verify(&hmac::Key::new(hmac::HMAC_SHA256, &key[32..]), &signed, mac).expect("MAC");
    cbc::Decryptor::<Aes256>::new_from_slices(&key[..32], iv).unwrap().decrypt_padded_vec_mut::<Pkcs7>(ct).unwrap()
}

/// Matches every request that changes something.
struct NotGet;

impl Match for NotGet {
    fn matches(&self, request: &Request) -> bool {
        request.method.as_str() != "GET"
    }
}

fn vault_ciphers(user: &VaultKey, own: &VaultKey, att_key: &[u8], server: &str) -> Vec<Value> {
    let mut git = cipher(user, "git", 1, "GitHub");
    git["folderId"] = json!("f1");
    git["favorite"] = json!(true);
    git["notes"] = t(user, "deploy notes");
    git["login"] = json!({"username": t(user, "octo"), "password": t(user, "hunter2!"),
        "totp": t(user, "otpauth://totp/GitHub:octo?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ&digits=6"),
        "uris": [{"uri": t(user, "https://github.com/login"), "match": 1},
                 {"uri": t(user, "https://gist.github.com"), "match": null}],
        "passwordRevisionDate": "2026-01-05T00:00:00Z", "uri": t(user, "https://github.com/login")});
    git["fields"] = json!([
        {"name": t(user, "env"), "value": t(user, "prod"), "type": 0, "linkedId": null},
        {"name": t(user, "pin"), "value": t(user, "4321"), "type": 1, "linkedId": null},
        {"name": t(user, "admin"), "value": t(user, "true"), "type": 2, "linkedId": null}]);
    git["passwordHistory"] = json!([{"password": t(user, "old-pw-1"), "lastUsedDate": "2025-12-01T00:00:00.000000Z"}]);
    git["attachments"] = json!([
        {"id": "att1", "fileName": t(user, "report.txt"), "size": "20", "key": null,
         "url": format!("{server}/attachments/git/att1?token=T"), "object": "attachment"}]);
    // The attachment key is 64 raw bytes: stored as the encryption of those bytes.
    git["attachments"][0]["key"] = json!(user.encrypt(att_key).unwrap());
    git["attachments"].as_array_mut().unwrap().push(json!({"id": "att2", "fileName": t(user, "huge.bin"),
        "size": "3000000", "key": user.encrypt(att_key).unwrap(), "object": "attachment"}));
    git["attachments"].as_array_mut().unwrap().push(json!({"id": "att3", "fileName": t(user, "elsewhere.txt"),
        "size": "20", "key": user.encrypt(att_key).unwrap(), "object": "attachment"}));

    let mut bank = cipher(own, "bank", 1, "Bank");
    bank["key"] = json!(user.encrypt(&own.to_bytes()).unwrap());
    bank["login"] = json!({"username": t(own, "anna"), "password": t(own, "s3cret"), "totp": null, "uris": null});
    bank["fields"] = json!([{"name": t(own, "branch"), "value": t(own, "north"), "type": 0, "linkedId": null}]);
    bank["notes"] = t(own, "seed phrase words");

    let mut card = cipher(user, "card1", 3, "Visa");
    card["folderId"] = json!("f2");
    card["card"] = json!({"cardholderName": t(user, "Anna Lee"), "brand": t(user, "Visa"),
        "number": t(user, "4111111111111111"), "expMonth": t(user, "12"), "expYear": t(user, "2030"),
        "code": t(user, "737")});

    let mut ident = cipher(user, "ident", 4, "Passport identity");
    ident["identity"] = json!({"title": t(user, "Ms"), "firstName": t(user, "Anna"), "middleName": null,
        "lastName": t(user, "Lee"), "email": t(user, "anna@example.com"), "company": t(user, "Acme"),
        "city": t(user, "Oslo"), "country": t(user, "NO"), "phone": t(user, "555 0100"),
        "address1": t(user, "Street 1"), "ssn": t(user, "078-05-1120"), "passportNumber": t(user, "P1234567"),
        "licenseNumber": t(user, "D9876543")});

    let mut ssh = cipher(user, "ssh", 5, "Deploy key");
    ssh["sshKey"] = json!({"privateKey": t(user, PRIVATE_KEY), "publicKey": t(user, PUBLIC_KEY),
        "keyFingerprint": t(user, FINGERPRINT)});

    let mut note = cipher(user, "note", 2, "Wifi");
    note["folderId"] = json!("f1");
    note["notes"] = t(user, "deploy notes");
    note["secureNote"] = json!({"type": 0});

    let mut old = cipher(user, "old", 1, "Old forum");
    old["folderId"] = json!("f2");
    old["deletedDate"] = json!("2026-03-01T00:00:00.000000Z");
    old["login"] =
        json!({"username": t(user, "olduser"), "password": t(user, "old-secret"), "totp": null, "uris": null});

    let mut arch = cipher(user, "arch", 1, "Archived shop");
    arch["folderId"] = json!("f2");
    arch["archivedDate"] = json!("2026-03-02T00:00:00.000000Z");
    arch["login"] = json!({"username": t(user, "shopper"), "password": t(user, "shop-pw"), "totp": null,
        "uris": [{"uri": t(user, "https://shop.example"), "match": null}]});

    let mut org = cipher(user, "org", 1, "Shared GitHub");
    org["organizationId"] = json!("o1");
    org["login"] = json!({"username": null, "password": null, "totp": null, "uris": null});

    vec![git, bank, card, ident, ssh, note, old, arch, org]
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
    let user = VaultKey::from_bytes(&[7u8; 64]).unwrap();
    let own = VaultKey::from_bytes(&[9u8; 64]).unwrap();
    let att_key = vec![11u8; 64];
    let wrapped = master.stretch().encrypt(&user.to_bytes()).unwrap();
    let ciphers = vault_ciphers(&user, &own, &att_key, &server.uri());
    let without_ssh: Vec<Value> = ciphers.iter().filter(|c| c["type"] != 5).cloned().collect();
    let folders = json!([{"id": "f1", "name": t(&user, "Work")}, {"id": "f2", "name": t(&user, "Personal")}]);
    // Vaultwarden leaves SSH keys out of a sync that does not say which client version asks.
    Mock::given(method("GET"))
        .and(path("/api/sync"))
        .and(header_exists("Bitwarden-Client-Version"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"profile": {"key": wrapped}, "ciphers": ciphers, "folders": folders})),
        )
        .with_priority(3)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/sync"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"profile": {"key": wrapped}, "ciphers": without_ssh, "folders": folders})),
        )
        .with_priority(4)
        .mount(&server)
        .await;
    // Attachments: metadata with the download address, the file, and one that lives on another host.
    Mock::given(method("GET"))
        .and(path("/api/ciphers/git/attachment/att1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "att1", "object": "attachment",
            "url": format!("{}/attachments/git/att1?token=T", server.uri())})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/ciphers/git/attachment/att3"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "att3", "object": "attachment",
            "url": "http://evil.example/attachments/git/att3?token=T"})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/attachments/git/att1"))
        .respond_with(
            ResponseTemplate::new(200).set_body_bytes(buffer_encrypt(&att_key, b"quarterly report\nline 2\n")),
        )
        .mount(&server)
        .await;
    // Attachments v2: the record, then the upload.
    Mock::given(method("POST"))
        .and(path_regex(r"^/api/ciphers/[^/]+/attachment/v2$"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"object": "attachment-fileUpload",
            "attachmentId": "att-new", "url": "/ciphers/git/attachment/att-new", "fileUploadType": 0})))
        .with_priority(1)
        .mount(&server)
        .await;
    // Everything that changes something is accepted and answers with an id.
    Mock::given(NotGet)
        .and(path_regex(r"^/api/(ciphers|folders)(/.*)?$"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "new-id"})))
        .with_priority(9)
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
        user,
        own,
        att_key,
        counter: AtomicU32::new(0),
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

/// The vault requests that changed something so far: (method, path).
async fn changes(env: &Env) -> Vec<(String, String)> {
    env.server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.method.as_str() != "GET" && r.url.path().starts_with("/api/"))
        .map(|r| (r.method.to_string(), r.url.path().to_owned()))
        .collect()
}

/// The JSON bodies the vault received for a method and path.
async fn sent(env: &Env, verb: &str, on: &str) -> Vec<Value> {
    env.server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.method.as_str() == verb && r.url.path() == on)
        .map(|r| serde_json::from_slice(&r.body).unwrap_or(Value::Null))
        .collect()
}

fn all_selected(view: &ApprovalView) -> ApprovalChoice {
    ApprovalChoice {
        selected_message_ids: view.messages.iter().map(|m| m.id.clone()).collect(),
        standing: None,
    }
}

fn nothing_selected() -> ApprovalChoice {
    ApprovalChoice {
        selected_message_ids: vec![],
        standing: None,
    }
}

/// Sends a request to the phone. Returns its id and whether it was parked for the user.
async fn ask(env: &Env, op: &str, args: Value) -> (String, bool) {
    let id = format!("r{}", env.counter.fetch_add(1, Ordering::SeqCst) + 1);
    serve_pending(env, &[request(&id, op, &args)]).await;
    let parked = !env.core.sync(0).await.unwrap().is_empty();
    (id, parked)
}

/// A read: the user ticks everything offered. Returns the items the AI got.
async fn read(env: &Env, op: &str, args: Value) -> Vec<Value> {
    let (id, parked) = ask(env, op, args).await;
    assert!(parked, "{op} was not parked: {:?}", answers(env).await.last());
    let view = env.core.approval_view(id.clone()).await.unwrap();
    env.core.approve(id, all_selected(&view)).await.unwrap();
    answers(env).await.last().unwrap()["result"]["data"]["items"].as_array().unwrap().clone()
}

/// A write, approved once. Returns what the user saw and what the AI got.
async fn write(env: &Env, op: &str, args: Value) -> (ApprovalView, Value) {
    let (id, parked) = ask(env, op, args).await;
    assert!(parked, "{op} was not parked: {:?}", answers(env).await.last());
    let view = env.core.approval_view(id.clone()).await.unwrap();
    assert_eq!(view.kind, ApprovalKind::Write);
    env.core.approve(id, nothing_selected()).await.unwrap();
    (view, answers(env).await.last().unwrap()["result"]["data"].clone())
}

/// A call the phone refuses (at the preview): what the AI is told.
async fn refused(env: &Env, op: &str, args: Value) -> String {
    let shown = args.to_string().chars().take(120).collect::<String>();
    let (_, parked) = ask(env, op, args).await;
    assert!(!parked, "{op} should have been refused: {shown}");
    let last = answers(env).await.last().unwrap().clone();
    assert_eq!(last["outcome"], "error", "{last}");
    last["message"].as_str().unwrap().to_owned()
}

fn no_secrets(what: &str, shown: &str) {
    for secret in SECRETS {
        assert!(!shown.contains(secret), "{what} leaks {secret}: {shown}");
    }
}

fn ids(items: &[Value]) -> Vec<String> {
    items.iter().map(|i| i["id"].as_str().unwrap().to_owned()).collect()
}

fn call_of(op: &str) -> ConnectorCall {
    ConnectorCall {
        service: "vault".to_owned(),
        op: op.to_owned(),
        args: Map::new(),
    }
}

// ---- registry ---------------------------------------------------------------------------------------------------

#[test]
fn every_change_has_a_class_and_only_the_permanent_ones_are_asked_every_time() {
    let once = ["item_delete", "trash_empty"];
    let organize = [
        "item_favorite",
        "item_move",
        "item_trash",
        "item_restore",
        "item_archive",
        "item_unarchive",
        "folder_create",
        "folder_rename",
        "folder_delete",
    ];
    let items = ["item_create", "item_update", "attachment_add", "attachment_delete", "item_clone"];
    for op in once.iter().chain(&organize).chain(&items) {
        let spec = call_of(op).spec().unwrap_or_else(|| panic!("{op}"));
        assert_eq!(spec.effect, Effect::Write, "{op}");
        assert_eq!(spec.once_only, once.contains(op), "{op}");
        let expected = if items.contains(op) {
            "items"
        } else {
            "organize"
        };
        assert_eq!(spec.class, expected, "{op}");
    }
    for op in ["search", "folders_list", "item_view", "get", "attachment_get"] {
        let spec = call_of(op).spec().unwrap();
        assert_ne!(spec.effect, Effect::Write, "{op}");
        assert_eq!(spec.class, "");
    }
    let spec = call_of("get").spec().unwrap();
    assert!(spec.parse(&json!({"item": "x", "field": "card_number"})).is_ok());
    assert!(spec.parse(&json!({"item": "x", "field": "nope"})).is_err());
}

// ---- reading ----------------------------------------------------------------------------------------------------

#[tokio::test]
async fn searching_filters_by_type_folder_state_and_favorite_and_never_shows_a_secret() {
    let env = env().await;
    // Logins in use, as always.
    let logins = read(&env, "search", json!({"query": ""})).await;
    assert_eq!(ids(&logins), ["bank", "git"]);
    let git = &logins[1];
    assert_eq!(
        (git["title"].as_str(), git["from"].as_str(), git["text"].as_str()),
        (Some("GitHub"), Some("octo"), Some("github.com"))
    );
    assert_eq!(git["in"]["id"], "f1/login/git");
    assert_eq!(git["folder"], "Work");
    assert_eq!(
        (git["type"].as_str(), git["state"].as_str(), git["favorite"].as_bool()),
        (Some("login"), Some("active"), Some(true))
    );
    assert_eq!(logins[0]["in"]["id"], "none/login/bank");
    assert_eq!(logins[0]["organization_items_skipped"], 1, "the shared item is skipped and counted");
    no_secrets("search", &serde_json::to_string(&logins).unwrap());

    let everything = read(&env, "search", json!({"type": "any", "state": "all"})).await;
    let mut got = ids(&everything);
    got.sort();
    assert_eq!(got, ["arch", "bank", "card1", "git", "ident", "note", "old", "ssh"], "SSH keys are listed too");
    let shown = serde_json::to_string(&everything).unwrap();
    no_secrets("search", &shown);
    let by_id = |id: &str| everything.iter().find(|i| i["id"] == id).unwrap().clone();
    assert_eq!((by_id("card1")["from"].as_str(), by_id("card1")["text"].as_str()), (Some("Anna Lee"), Some("Card")));
    assert_eq!(by_id("card1")["in"]["id"], "f2/card/card1");
    assert_eq!(by_id("ident")["from"], "Anna Lee");
    assert_eq!(by_id("ssh")["in"]["id"], "none/ssh_key/ssh");
    assert_eq!(by_id("old")["state"], "trash");
    assert_eq!(by_id("arch")["state"], "archived");

    assert_eq!(ids(&read(&env, "search", json!({"type": "any", "state": "trash"})).await), ["old"]);
    assert_eq!(ids(&read(&env, "search", json!({"type": "login", "state": "archived"})).await), ["arch"]);
    assert_eq!(ids(&read(&env, "search", json!({"type": "any", "folder": "Work"})).await), ["git", "note"]);
    assert_eq!(ids(&read(&env, "search", json!({"type": "any", "folder": "f2"})).await), ["card1"]);
    assert_eq!(ids(&read(&env, "search", json!({"type": "any", "folder": "none"})).await), ["bank", "ssh", "ident"]);
    assert_eq!(ids(&read(&env, "search", json!({"type": "any", "favorite": true})).await), ["git"]);
    assert_eq!(ids(&read(&env, "search", json!({"type": "any", "favorite": false, "folder": "none"})).await).len(), 3);
    assert_eq!(ids(&read(&env, "search", json!({"type": "card"})).await), ["card1"]);
    assert_eq!(ids(&read(&env, "search", json!({"type": "any", "query": "anna"})).await), ["bank", "ident", "card1"]);
    assert_eq!(ids(&read(&env, "search", json!({"query": "GIST.github"})).await), ["git"], "any website of a login");
    assert_eq!(ids(&read(&env, "search", json!({"type": "any", "state": "all", "limit": 2})).await).len(), 2);

    let message = refused(&env, "search", json!({"folder": "Nowhere"})).await;
    assert!(message.contains("No folder with that id or name"), "{message}");
}

#[tokio::test]
async fn folders_are_listed_with_their_counts() {
    let env = env().await;
    let folders = read(&env, "folders_list", json!({})).await;
    assert_eq!(ids(&folders), ["none", "f2", "f1"], "no folder first, then by name (Personal, Work)");
    let by_id = |id: &str| folders.iter().find(|f| f["id"] == id).unwrap().clone();
    assert_eq!(by_id("f1")["title"], "Work");
    assert_eq!(by_id("f1")["by_type"], json!({"login": 1, "note": 1}));
    assert_eq!(by_id("f2")["by_type"], json!({"card": 1, "login": 1}));
    assert_eq!((by_id("f2")["in_trash"].as_i64(), by_id("f2")["archived"].as_i64()), (Some(1), Some(1)));
    assert_eq!(by_id("none")["by_type"], json!({"identity": 1, "login": 1, "ssh_key": 1}));
    assert_eq!(by_id("none")["organization_items_skipped"], 1);
    no_secrets("folders", &serde_json::to_string(&folders).unwrap());
}

#[tokio::test]
async fn an_item_view_shows_the_facts_of_every_type_and_no_secret() {
    let env = env().await;
    let git = read(&env, "item_view", json!({"item": "git"})).await.remove(0);
    let shown = git.to_string();
    no_secrets("login view", &shown);
    assert_eq!(git["in"]["id"], "f1/login/git");
    assert_eq!(
        (git["type"].as_str(), git["folder"].as_str(), git["state"].as_str()),
        (Some("login"), Some("Work"), Some("active"))
    );
    assert_eq!(git["websites"], json!(["https://github.com/login", "https://gist.github.com"]));
    assert_eq!(git["username"], "octo");
    assert_eq!((git["has_password"].as_bool(), git["has_one_time_code"].as_bool()), (Some(true), Some(true)));
    assert_eq!(git["notes_length"], 12);
    assert_eq!(
        git["custom_fields"],
        json!([{"name": "env", "type": "text"}, {"name": "pin", "type": "hidden"}, {"name": "admin", "type": "boolean"}])
    );
    assert_eq!(git["attachments"][0], json!({"id": "att1", "name": "report.txt", "size": 20}));
    assert_eq!(git["password_history_count"], 1);
    assert_eq!(git["modified"], "2026-02-02T10:00:00.000000Z");
    assert!(git["text"].as_str().unwrap().contains("Website: https://gist.github.com"));

    let card = read(&env, "item_view", json!({"item": "card1"})).await.remove(0);
    no_secrets("card view", &card.to_string());
    assert_eq!(
        (card["brand"].as_str(), card["last4"].as_str(), card["exp_month"].as_str(), card["exp_year"].as_str()),
        (Some("Visa"), Some("1111"), Some("12"), Some("2030"))
    );

    let ident = read(&env, "item_view", json!({"item": "ident"})).await.remove(0);
    no_secrets("identity view", &ident.to_string());
    assert_eq!(
        (ident["email"].as_str(), ident["company"].as_str(), ident["city"].as_str()),
        (Some("anna@example.com"), Some("Acme"), Some("Oslo"))
    );
    assert!(ident.get("phone").is_none() && ident.get("ssn").is_none(), "{ident}");

    let ssh = read(&env, "item_view", json!({"item": "ssh"})).await.remove(0);
    no_secrets("ssh view", &ssh.to_string());
    assert_eq!((ssh["public_key"].as_str(), ssh["fingerprint"].as_str()), (Some(PUBLIC_KEY), Some(FINGERPRINT)));

    let old = read(&env, "item_view", json!({"item": "old"})).await.remove(0);
    assert_eq!(old["state"], "trash");
    assert!(old["deleted"].is_string());

    let message = refused(&env, "item_view", json!({"item": "org"})).await;
    assert!(message.contains("No item with that id"), "{message}");
}

#[tokio::test]
async fn every_field_is_fetched_as_a_secret_except_the_ordinary_ones() {
    let env = env().await;
    // (item, field, custom field, what the AI gets, is a secret)
    let cases: &[(&str, &str, Option<&str>, &str, bool)] = &[
        ("git", "username", None, "octo", true),
        ("git", "password", None, "hunter2!", true),
        ("git", "notes", None, "deploy notes", true),
        ("git", "uri", None, "https://github.com/login\nhttps://gist.github.com", false),
        ("git", "custom", Some("pin"), "4321", true),
        ("git", "custom", Some("ENV"), "prod", true),
        ("git", "custom", None, "env: prod\npin: 4321\nadmin: true", true),
        ("git", "password_history", None, "2025-12-01T00:00:00.000000Z: old-pw-1", true),
        ("bank", "password", None, "s3cret", true),
        ("bank", "notes", None, "seed phrase words", true),
        ("card1", "card_number", None, "4111111111111111", true),
        ("card1", "card_code", None, "737", true),
        ("card1", "card_holder", None, "Anna Lee", false),
        ("card1", "card_expiry", None, "12/2030", false),
        ("ident", "identity_ssn", None, "078-05-1120", true),
        ("ident", "identity_passport", None, "P1234567", true),
        ("ident", "identity_license", None, "D9876543", true),
        ("ssh", "ssh_public_key", None, PUBLIC_KEY, false),
        ("ssh", "ssh_fingerprint", None, FINGERPRINT, false),
        ("arch", "password", None, "shop-pw", true),
    ];
    for (item, field, custom, expected, secret) in cases {
        let mut args = json!({"item": item, "field": field});
        if let Some(c) = custom {
            args["custom_field"] = json!(c);
        }
        let (id, parked) = ask(&env, "get", args).await;
        assert!(parked, "{item} {field}: {:?}", answers(&env).await.last());
        let view = env.core.approval_view(id.clone()).await.unwrap();
        let message = &view.messages[0];
        assert_eq!(message.sensitive, *secret, "{item} {field}");
        assert_eq!(view.no_standing, *secret, "{item} {field}");
        if *secret {
            assert!(message.snippet.contains(" for "), "{}", message.snippet);
            no_secrets("the approval", &format!("{view:?}"));
            if *field != "username" {
                // (A username is what the list shows next to the name anyway.)
                assert!(!format!("{view:?}").contains(expected), "{item} {field}: the approval shows the value");
            }
        }
        env.core.approve(id, all_selected(&view)).await.unwrap();
        let told = answers(&env).await.last().unwrap().clone();
        assert_eq!(told["result"]["data"]["items"][0]["text"], *expected, "{item} {field}");
    }
    // The private half of an SSH key is never handed out, approved or not: the desktop app's SSH agent has the phone
    // sign instead.
    // (The field is not one the tool offers, so the call is refused before anything is read.)
    let told = refused(&env, "get", json!({"item": "ssh", "field": "ssh_private_key"})).await;
    assert!(told.contains("malformed") || told.contains("never given out"), "{told}");
    no_secrets("the refusal", &told);
    let identity = read(&env, "get", json!({"item": "ident", "field": "identity"})).await;
    let block = identity[0]["text"].as_str().unwrap();
    for line in [
        "Title: Ms",
        "First name: Anna",
        "Last name: Lee",
        "Email: anna@example.com",
        "Social security number: 078-05-1120",
        "License number: D9876543",
    ] {
        assert!(block.contains(line), "{block}");
    }
    assert!(!block.contains("Middle name"));
    let totp = read(&env, "get", json!({"item": "git", "field": "totp"})).await;
    assert_eq!(totp[0]["text"].as_str().unwrap().len(), 6);
}

#[tokio::test]
async fn fetching_the_wrong_thing_is_explained() {
    let env = env().await;
    for (args, needle) in [
        (json!({"item": "card1", "field": "password"}), "That item is a card, which has no password"),
        (json!({"item": "git", "field": "card_number"}), "That item is a login, which has no number"),
        (json!({"item": "bank", "field": "uri"}), "That login has no website"),
        (json!({"item": "bank", "field": "totp"}), "no one-time code"),
        (json!({"item": "bank", "field": "password_history"}), "keeps no earlier passwords"),
        (json!({"item": "note", "field": "custom"}), "no custom fields"),
        (json!({"item": "git", "field": "custom", "custom_field": "zzz"}), "The item has: env, pin, admin"),
        (json!({"item": "ident", "field": "notes"}), "no notes"),
        (json!({"item": "old", "field": "password"}), "No login with that id"),
        (json!({"item": "org", "field": "notes"}), "No item with that id"),
        (json!({"item": "nope", "field": "identity"}), "No item with that id"),
    ] {
        let message = refused(&env, "get", args.clone()).await;
        assert!(message.contains(needle), "{args}: {message}");
        no_secrets("an error", &message);
    }
}

#[tokio::test]
async fn an_attachment_is_downloaded_decrypted_and_only_from_the_vaults_own_host() {
    let env = env().await;
    let (id, parked) = ask(&env, "attachment_get", json!({"item": "git", "attachment": "report.txt"})).await;
    assert!(parked);
    let view = env.core.approval_view(id.clone()).await.unwrap();
    assert!(view.no_standing && view.messages[0].sensitive);
    assert!(view.messages[0].snippet.contains("report.txt"));
    assert!(!format!("{view:?}").contains("quarterly"));
    env.core.approve(id, all_selected(&view)).await.unwrap();
    let item = answers(&env).await.last().unwrap()["result"]["data"]["items"][0].clone();
    assert_eq!(item["encoding"], "base64");
    assert_eq!(BASE64.decode(item["text"].as_str().unwrap().as_bytes()).unwrap(), b"quarterly report\nline 2\n");
    assert_eq!(item["file_name"], "report.txt");
    // By id as well.
    let again = read(&env, "attachment_get", json!({"item": "git", "attachment": "att1"})).await;
    assert_eq!(again[0]["text"], item["text"]);
    // The token URL was fetched without the bearer token.
    let downloads: Vec<_> = env
        .server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.url.path() == "/attachments/git/att1")
        .collect();
    assert_eq!(downloads.len(), 2);
    assert!(downloads.iter().all(|r| !r.headers.contains_key("authorization")));

    // Too large: only the facts.
    let big = read(&env, "attachment_get", json!({"item": "git", "attachment": "huge.bin"})).await;
    assert_eq!(big[0]["too_large"], true);
    assert!(big[0]["text"].as_str().unwrap().contains("too large"));
    // Another host is never contacted.
    let message = refused(&env, "attachment_get", json!({"item": "git", "attachment": "elsewhere.txt"})).await;
    assert!(message.contains("another host"), "{message}");
    let message = refused(&env, "attachment_get", json!({"item": "git", "attachment": "missing.txt"})).await;
    assert!(message.contains("no attachment with that id or name"), "{message}");
}

// ---- creating ---------------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_login_is_created_encrypted_with_everything_and_the_preview_hides_the_secrets() {
    let env = env().await;
    let args = json!({"type": "login", "name": "Example", "notes": "seed phrase words", "favorite": true,
        "folder": "Work",
        "fields": {"username": "me@example.com", "password": "Sup3r-secret!", "totp": "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ",
                   "uris": ["https://example.com/login", "https://www.example.com"]},
        "custom_fields": [{"name": "pin", "value": "9876", "type": "hidden"}, {"name": "site", "value": "eu"},
                          {"name": "admin", "value": "true", "type": "boolean"}]});
    let (view, done) = write(&env, "item_create", args).await;
    // What the user saw.
    let shown = view.preview.join("\n");
    assert!(view.preview[0].starts_with("Create login \"Example\" in Work"), "{shown}");
    for line in [
        "Username: me@example.com",
        "Website: https://example.com/login",
        "Password: set (13 characters)",
        "One-time code: set (32 characters)",
        "Notes: set (17 characters)",
        "Favorite: yes",
        "Custom field \"site\" (text): eu",
        "Custom field \"pin\" (hidden): set (4 characters)",
    ] {
        assert!(shown.contains(line), "missing {line:?} in\n{shown}");
    }
    for secret in ["Sup3r-secret!", "GEZDGNBV", "seed phrase words", "9876"] {
        assert!(!shown.contains(secret) && !format!("{view:?}").contains(secret), "the preview leaks {secret}");
    }
    assert_eq!(view.resources[0].id, "f1/login");
    assert_eq!((view.class.as_str(), view.no_standing), ("items", false));
    assert_eq!(view.classes.len(), 5, "items, organize, sends, secrets, ssh");
    // What the AI got.
    assert_eq!(
        (done["created"].as_bool(), done["id"].as_str(), done["type"].as_str()),
        (Some(true), Some("new-id"), Some("login"))
    );
    // What the vault got.
    let bodies = sent(&env, "POST", "/api/ciphers").await;
    assert_eq!(bodies.len(), 1);
    let body = &bodies[0];
    let raw = body.to_string();
    for secret in ["Sup3r-secret!", "me@example.com", "Example", "seed phrase words", "9876", "GEZDGNBV"] {
        assert!(!raw.contains(secret), "the vault got {secret} in the clear");
    }
    let u = &env.user;
    assert_eq!(
        (body["type"].as_i64(), body["folderId"].as_str(), body["favorite"].as_bool()),
        (Some(1), Some("f1"), Some(true))
    );
    assert!(body["key"].is_null() && body["organizationId"].is_null() && body["archivedDate"].is_null());
    assert_eq!(body["reprompt"], 0);
    assert_eq!(dec(u, &body["name"]), "Example");
    assert_eq!(dec(u, &body["notes"]), "seed phrase words");
    let login = &body["login"];
    assert_eq!(dec(u, &login["username"]), "me@example.com");
    assert_eq!(dec(u, &login["password"]), "Sup3r-secret!");
    assert_eq!(dec(u, &login["totp"]), "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ");
    let uris: Vec<String> = login["uris"].as_array().unwrap().iter().map(|x| dec(u, &x["uri"])).collect();
    assert_eq!(uris, ["https://example.com/login", "https://www.example.com"]);
    assert!(login["uris"][0]["match"].is_null());
    let fields: Vec<(String, String, i64)> = body["fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| (dec(u, &f["name"]), dec(u, &f["value"]), f["type"].as_i64().unwrap()))
        .collect();
    assert_eq!(
        fields,
        [("pin".into(), "9876".into(), 1), ("site".into(), "eu".into(), 0), ("admin".into(), "true".into(), 2)]
    );
    for other in ["secureNote", "card", "identity", "sshKey"] {
        assert!(body.get(other).is_none());
    }
    assert_eq!(changes(&env).await, [("POST".to_owned(), "/api/ciphers".to_owned())]);
}

#[tokio::test]
async fn notes_cards_identities_and_ssh_keys_are_created_with_their_own_objects() {
    let env = env().await;
    let u = VaultKey::from_bytes(&[7u8; 64]).unwrap();

    let (view, _) =
        write(&env, "item_create", json!({"type": "note", "name": "Wifi code", "notes": "the code is 1234"})).await;
    assert_eq!(view.resources[0].id, "none/note");
    assert!(!view.preview.join("\n").contains("1234"));
    let bodies = sent(&env, "POST", "/api/ciphers").await;
    assert_eq!(bodies[0]["type"], 2);
    assert_eq!(bodies[0]["secureNote"], json!({"type": 0}));
    assert_eq!(dec(&u, &bodies[0]["notes"]), "the code is 1234");
    assert!(bodies[0]["folderId"].is_null());

    let (view, _) = write(
        &env,
        "item_create",
        json!({"type": "card", "name": "Travel card", "folder": "f2", "fields": {"holder": "Anna Lee", "brand": "Mastercard",
            "number": "5555555555554444", "exp_month": 3, "exp_year": "2031", "code": "999"}}),
    )
    .await;
    let shown = view.preview.join("\n");
    assert!(shown.contains("Holder: Anna Lee") && shown.contains("Brand: Mastercard"), "{shown}");
    assert!(
        shown.contains("Number: set (16 characters)") && shown.contains("Security code: set (3 characters)"),
        "{shown}"
    );
    assert!(shown.contains("Expiry month: 3") && shown.contains("Expiry year: 2031"), "{shown}");
    assert!(!shown.contains("5555555555554444") && !shown.contains("999"));
    let card = &sent(&env, "POST", "/api/ciphers").await[1];
    assert_eq!((card["type"].as_i64(), card["folderId"].as_str()), (Some(3), Some("f2")));
    let c = &card["card"];
    assert_eq!(
        [
            dec(&u, &c["cardholderName"]),
            dec(&u, &c["brand"]),
            dec(&u, &c["number"]),
            dec(&u, &c["expMonth"]),
            dec(&u, &c["expYear"]),
            dec(&u, &c["code"])
        ],
        ["Anna Lee", "Mastercard", "5555555555554444", "3", "2031", "999"]
    );

    let (view, _) = write(
        &env,
        "item_create",
        json!({"type": "identity", "name": "Home", "fields": {"title": "Ms", "first_name": "Anna", "last_name": "Lee",
            "address1": "Street 1", "postal_code": "0150", "email": "anna@example.com", "ssn": "078-05-1120",
            "passport_number": "P1234567", "license_number": "D9876543", "username": "annal"}}),
    )
    .await;
    let shown = view.preview.join("\n");
    assert!(shown.contains("First name: Anna") && shown.contains("Postal code: 0150"), "{shown}");
    assert!(shown.contains("Social security number: set (11 characters)"), "{shown}");
    no_secrets("the identity preview", &shown);
    let ident = &sent(&env, "POST", "/api/ciphers").await[2];
    let i = &ident["identity"];
    assert_eq!(ident["type"], 4);
    assert_eq!(dec(&u, &i["firstName"]), "Anna");
    assert_eq!(dec(&u, &i["postalCode"]), "0150");
    assert_eq!(dec(&u, &i["ssn"]), "078-05-1120");
    assert_eq!(dec(&u, &i["passportNumber"]), "P1234567");
    assert_eq!(dec(&u, &i["licenseNumber"]), "D9876543");
    assert_eq!(dec(&u, &i["username"]), "annal");
    assert!(i["middleName"].is_null() && i["phone"].is_null());

    // An SSH key: the fingerprint is worked out from the public key.
    let (view, _) = write(
        &env,
        "item_create",
        json!({"type": "ssh_key", "name": "Deploy", "fields": {"private_key": PRIVATE_KEY, "public_key": PUBLIC_KEY}}),
    )
    .await;
    let shown = view.preview.join("\n");
    assert!(shown.contains(&format!("Fingerprint: {FINGERPRINT}")) && shown.contains("Private key: set ("), "{shown}");
    assert!(!shown.contains("FAKEPRIVATEKEYMATERIAL"));
    let ssh = &sent(&env, "POST", "/api/ciphers").await[3];
    let k = &ssh["sshKey"];
    assert_eq!(
        (
            ssh["type"].as_i64(),
            dec(&u, &k["privateKey"]).as_str(),
            dec(&u, &k["publicKey"]).as_str(),
            dec(&u, &k["keyFingerprint"]).as_str()
        ),
        (Some(5), PRIVATE_KEY, PUBLIC_KEY, FINGERPRINT)
    );
}

#[tokio::test]
async fn creating_with_bad_input_is_refused_before_anything_is_sent() {
    let env = env().await;
    for (args, needle) in [
        (json!({"type": "login", "name": "x", "fields": {"pasword": "x"}}), "Unknown field `pasword`"),
        (json!({"type": "note", "name": "x", "fields": {"username": "x"}}), "has no `fields`"),
        (json!({"type": "login", "name": "x", "fields": {"totp": "not base32 1"}}), "base32"),
        (json!({"type": "card", "name": "x", "fields": {"exp_month": "13"}}), "month from 1 to 12"),
        (json!({"type": "card", "name": "x", "fields": {"exp_year": "99"}}), "four-digit year"),
        (
            json!({"type": "ssh_key", "name": "x", "fields": {"private_key": "k"}}),
            "needs `fields.private_key` and `fields.public_key`",
        ),
        (
            json!({"type": "ssh_key", "name": "x", "fields": {"private_key": "k", "public_key": "junk"}}),
            "not an OpenSSH public key",
        ),
        (json!({"type": "login", "name": "x", "folder": "Nope"}), "No folder with that id or name"),
        (json!({"type": "login", "name": "x", "fields": "[1]"}), "must be an object"),
        (
            json!({"type": "login", "name": "x", "custom_fields": [{"name": "a", "type": "boolean", "value": "maybe"}]}),
            "true or false",
        ),
        (json!({"type": "login", "name": "x", "custom_fields": [{"name": "a"}, {"name": "a"}]}), "given twice"),
        (json!({"type": "login", "name": "x", "custom_fields": [{"name": "a", "colour": "red"}]}), "no `colour`"),
        (
            json!({"type": "login", "name": "x", "custom_fields": [{"name": "a", "type": "secret"}]}),
            "text, hidden or boolean",
        ),
    ] {
        let message = refused(&env, "item_create", args.clone()).await;
        assert!(message.contains(needle), "{args}: {message}");
    }
    assert_eq!(changes(&env).await.len(), 0);
    // The registry itself refuses malformed arguments.
    let spec = call_of("item_create").spec().unwrap();
    assert!(spec.parse(&json!({"name": "x"})).unwrap_err().contains("`type` is required"));
    assert!(spec.parse(&json!({"type": "wallet", "name": "x"})).is_err());
    assert!(spec.parse(&json!({"type": "login", "name": "x", "extra": 1})).is_err());
}

// ---- updating ---------------------------------------------------------------------------------------------------

#[tokio::test]
async fn an_update_resends_the_whole_item_keeps_its_own_key_and_changes_only_what_was_given() {
    let env = env().await;
    let (view, done) = write(
        &env,
        "item_update",
        json!({"item": "bank", "fields": {"username": "anna2", "password": "n3w-pass!"},
               "custom_fields": [{"name": "branch", "value": "south"}, {"name": "iban", "value": "NO93 0000", "type": "hidden"}],
               "favorite": true, "folder": "Personal"}),
    )
    .await;
    let shown = view.preview.join("\n");
    assert!(shown.contains("Username: anna → anna2"), "{shown}");
    assert!(shown.contains("Password: changes (9 characters)"), "{shown}");
    assert!(shown.contains("Favorite: no → yes") && shown.contains("Folder: No folder → Personal"), "{shown}");
    assert!(shown.contains("Custom field \"branch\": north → south"), "{shown}");
    assert!(shown.contains("Add Custom field \"iban\" (hidden): set (9 characters)"), "{shown}");
    for secret in ["s3cret", "n3w-pass!", "NO93 0000"] {
        assert!(!shown.contains(secret));
    }
    assert_eq!(view.resources[0].id, "none/login/bank");
    assert_eq!((done["updated"].as_bool(), done["id"].as_str()), (Some(true), Some("bank")));

    let body = &sent(&env, "PUT", "/api/ciphers/bank").await[0];
    // The item's own key is sent back as it was, and everything is encrypted with it, not with the user key.
    assert_eq!(env.user.decrypt(body["key"].as_str().unwrap()).unwrap().to_vec(), env.own.to_bytes().to_vec());
    assert!(env.user.decrypt_text(body["name"].as_str().unwrap()).is_err(), "not the user key");
    let o = &env.own;
    assert_eq!(dec(o, &body["name"]), "Bank");
    assert_eq!(dec(o, &body["notes"]), "seed phrase words", "what was not given is resent");
    assert_eq!(dec(o, &body["login"]["username"]), "anna2");
    assert_eq!(dec(o, &body["login"]["password"]), "n3w-pass!");
    let history = body["passwordHistory"].as_array().unwrap();
    assert_eq!((history.len(), dec(o, &history[0]["password"])), (1, "s3cret".to_owned()), "the old password is kept");
    assert!(history[0]["lastUsedDate"].as_str().unwrap().ends_with('Z'));
    assert!(body["login"]["passwordRevisionDate"].as_str().unwrap().starts_with("20"));
    let fields: Vec<(String, String, i64)> = body["fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| (dec(o, &f["name"]), dec(o, &f["value"]), f["type"].as_i64().unwrap()))
        .collect();
    assert_eq!(fields, [("branch".into(), "south".into(), 0), ("iban".into(), "NO93 0000".into(), 1)]);
    assert_eq!((body["favorite"].as_bool(), body["folderId"].as_str()), (Some(true), Some("f2")));
    assert_eq!(body["lastKnownRevisionDate"], "2026-02-02T10:00:00.000000Z");
    assert_eq!((body["type"].as_i64(), body["id"].as_str()), (Some(1), Some("bank")));
    assert!(body["organizationId"].is_null() && body["archivedDate"].is_null());
    assert!(body["login"].get("uri").is_none(), "the server works the first address out again");
}

#[tokio::test]
async fn an_update_keeps_the_archive_the_websites_rules_the_history_and_can_remove_things() {
    let env = env().await;
    // An archived item stays archived (the server unarchives on an update without the date).
    write(&env, "item_update", json!({"item": "arch", "name": "Shop"})).await;
    let body = &sent(&env, "PUT", "/api/ciphers/arch").await[0];
    assert_eq!(body["archivedDate"], "2026-03-02T00:00:00.000000Z");
    assert!(body["key"].is_null(), "an item without its own key gets none");
    assert_eq!(dec(&env.user, &body["name"]), "Shop");
    assert_eq!(dec(&env.user, &body["login"]["password"]), "shop-pw");

    // Websites are replaced; an address that stays keeps its matching rule; the history grows by one; fields go.
    let (view, _) = write(
        &env,
        "item_update",
        json!({"item": "git", "fields": {"uris": ["https://gist.github.com", "https://git.example"], "password": "hunter3!"},
               "remove_custom_fields": ["env"], "notes": ""}),
    )
    .await;
    let shown = view.preview.join("\n");
    assert!(
        shown.contains(
            "Websites: https://github.com/login, https://gist.github.com → https://gist.github.com, https://git.example"
        ),
        "{shown}"
    );
    assert!(shown.contains("Remove custom field \"env\"") && shown.contains("Notes: cleared"), "{shown}");
    assert!(shown.contains("The old password is kept"), "{shown}");
    let body = &sent(&env, "PUT", "/api/ciphers/git").await[0];
    let u = &env.user;
    assert_eq!(dec(u, &body["login"]["uris"][0]["uri"]), "https://gist.github.com");
    assert!(body["login"]["uris"][0]["match"].is_null());
    assert_eq!(dec(u, &body["login"]["uris"][1]["uri"]), "https://git.example");
    assert!(body["notes"].is_null());
    let names: Vec<String> = body["fields"].as_array().unwrap().iter().map(|f| dec(u, &f["name"])).collect();
    assert_eq!(names, ["pin", "admin"]);
    let history = body["passwordHistory"].as_array().unwrap();
    assert_eq!(
        (dec(u, &history[0]["password"]), dec(u, &history[1]["password"])),
        ("hunter2!".to_owned(), "old-pw-1".to_owned())
    );
    assert_eq!(
        dec(u, &body["login"]["totp"]),
        "otpauth://totp/GitHub:octo?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ&digits=6"
    );
    assert_eq!(body["folderId"], "f1");
    assert_eq!(body["favorite"], true);

    // A name and an SSH key's fingerprint follow the public key.
    write(
        &env,
        "item_update",
        json!({"item": "ssh", "fields": {"public_key": PUBLIC_KEY.replace("test@reins", "renamed")}}),
    )
    .await;
    let body = &sent(&env, "PUT", "/api/ciphers/ssh").await[0];
    assert_eq!(dec(u, &body["sshKey"]["keyFingerprint"]), FINGERPRINT);
    assert_eq!(dec(u, &body["sshKey"]["privateKey"]), PRIVATE_KEY);
}

#[tokio::test]
async fn updates_that_make_no_sense_are_refused_and_nothing_is_sent() {
    let env = env().await;
    for (args, needle) in [
        (json!({"item": "git"}), "Nothing to change"),
        (json!({"item": "old", "name": "x"}), "in the trash"),
        (json!({"item": "nope", "name": "x"}), "No item with that id"),
        (json!({"item": "org", "name": "x"}), "No item with that id"),
        (json!({"item": "card1", "fields": {"password": "x"}}), "Unknown field `password` for a card"),
        (json!({"item": "git", "remove_custom_fields": ["zzz"]}), "no custom field \"zzz\""),
        (json!({"item": "ssh", "fields": {"private_key": ""}}), "cannot lose its private key"),
        (json!({"item": "git", "name": ""}), "empty"),
        (json!({"item": "card1", "fields": {"exp_month": 13}}), "month from 1 to 12"),
    ] {
        let message = refused(&env, "item_update", args.clone()).await;
        assert!(message.contains(needle), "{args}: {message}");
    }
    assert_eq!(changes(&env).await.len(), 0);
}

#[tokio::test]
async fn a_conflicting_update_and_server_errors_reach_the_ai_in_words() {
    let env = env().await;
    Mock::given(method("PUT"))
        .and(path("/api/ciphers/git"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({"message":
            "The client copy of this cipher is out of date. Resync the client and try again.", "Message": "x"})))
        .with_priority(1)
        .mount(&env.server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/api/ciphers/bank/partial"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({"message": "Invalid folder"})))
        .with_priority(1)
        .mount(&env.server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/api/ciphers/note/delete"))
        .respond_with(ResponseTemplate::new(500).set_body_string("boom secret-token-123"))
        .with_priority(1)
        .mount(&env.server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/api/ciphers/card1/archive"))
        .respond_with(ResponseTemplate::new(404).set_body_string("{}"))
        .with_priority(1)
        .mount(&env.server)
        .await;

    for (op, args, needle) in [
        ("item_update", json!({"item": "git", "name": "x"}), "out of date"),
        ("item_move", json!({"item": "bank", "folder": "Work"}), "The vault refused: Invalid folder"),
        ("item_trash", json!({"item": "note"}), "server error 500"),
        ("item_archive", json!({"item": "card1"}), "does not have that"),
    ] {
        let (id, parked) = ask(&env, op, args).await;
        assert!(parked);
        // The action failed: the request stays parked and the user is told why.
        let message = env.core.approve(id, nothing_selected()).await.unwrap_err().to_string();
        assert!(message.contains(needle), "{op}: {message}");
        assert!(!message.contains("secret-token"), "{message}");
    }
}

// ---- attachments ------------------------------------------------------------------------------------------------

/// The bytes and the file name of the `data` part of a multipart body.
fn upload_part(request: &Request) -> (Vec<u8>, String) {
    let content_type = request.headers.get("content-type").unwrap().to_str().unwrap().to_owned();
    let boundary = content_type.split("boundary=").nth(1).unwrap().trim_matches('"').to_owned();
    let body = &request.body;
    let marker = format!("--{boundary}");
    let text = String::from_utf8_lossy(body);
    let start = text.find(&marker).unwrap();
    let head_end = text[start..].find("\r\n\r\n").unwrap() + start;
    let head = text[start..head_end].to_owned();
    let data_start = head_end + 4;
    let data_end = body.windows(marker.len() + 2).rposition(|w| w == format!("\r\n{marker}").as_bytes()).unwrap();
    assert!(head.contains("name=\"data\""), "{head}");
    let name = head.split("filename=\"").nth(1).unwrap().split('"').next().unwrap().to_owned();
    (body[data_start..data_end].to_vec(), name)
}

#[tokio::test]
async fn an_attachment_is_encrypted_with_a_fresh_key_that_the_item_key_protects() {
    let env = env().await;
    let content = b"binary \x00\x01\x02 data".to_vec();
    let (view, done) = write(
        &env,
        "attachment_add",
        json!({"item": "git", "file_name": "notes.bin", "content_base64": BASE64.encode(&content)}),
    )
    .await;
    assert_eq!(view.preview, ["Attach \"notes.bin\" (15 bytes) to login \"GitHub\""]);
    assert_eq!(view.class, "items");
    assert_eq!((done["attached"].as_bool(), done["attachment"].as_str()), (Some(true), Some("att-new")));

    let announce = &sent(&env, "POST", "/api/ciphers/git/attachment/v2").await[0];
    let u = &env.user;
    assert_eq!(dec(u, &announce["fileName"]), "notes.bin");
    let file_key = u.decrypt(announce["key"].as_str().unwrap()).unwrap();
    assert_eq!(file_key.len(), 64, "the file key is 64 random bytes");
    assert_ne!(*file_key, env.att_key);
    assert_eq!(announce["adminRequest"], false);

    let upload = env
        .server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .find(|r| r.method.as_str() == "POST" && r.url.path() == "/api/ciphers/git/attachment/att-new")
        .expect("the file is uploaded");
    assert_eq!(upload.headers.get("authorization").unwrap(), format!("Bearer {}", common::account_token()).as_str());
    let (encrypted, part_name) = upload_part(&upload);
    assert_eq!(
        announce["fileSize"].as_u64().unwrap(),
        encrypted.len() as u64,
        "the size announced is the encrypted size"
    );
    assert_eq!(part_name, announce["fileName"].as_str().unwrap(), "the part is named by the encrypted file name");
    assert_eq!(buffer_decrypt(&file_key, &encrypted), content);
    assert!(!String::from_utf8_lossy(&upload.body).contains("binary"));

    // An item with its own key protects the file key with that key.
    write(&env, "attachment_add", json!({"item": "bank", "file_name": "a.txt", "content_base64": BASE64.encode(b"x")}))
        .await;
    let announce = &sent(&env, "POST", "/api/ciphers/bank/attachment/v2").await[0];
    assert_eq!(dec(&env.own, &announce["fileName"]), "a.txt");
    assert_eq!(env.own.decrypt(announce["key"].as_str().unwrap()).unwrap().len(), 64);
}

#[tokio::test]
async fn a_failed_upload_removes_the_empty_attachment_and_bad_files_are_refused() {
    let env = env().await;
    Mock::given(method("POST"))
        .and(path("/api/ciphers/git/attachment/att-new"))
        .respond_with(
            ResponseTemplate::new(400)
                .set_body_json(json!({"message": "Attachment storage limit exceeded with this file"})),
        )
        .with_priority(1)
        .mount(&env.server)
        .await;
    let (id, parked) = ask(
        &env,
        "attachment_add",
        json!({"item": "git", "file_name": "big.bin", "content_base64": BASE64.encode(b"data")}),
    )
    .await;
    assert!(parked);
    let message = env.core.approve(id.clone(), nothing_selected()).await.unwrap_err().to_string();
    assert!(message.contains("storage limit exceeded"), "{message}");
    // The request stays parked for the user; they decline it.
    env.core.deny(id).await.unwrap();
    let changes = changes(&env).await;
    assert!(changes.contains(&("DELETE".to_owned(), "/api/ciphers/git/attachment/att-new".to_owned())), "{changes:?}");

    let too_big = BASE64.encode(&vec![0u8; 2 * 1024 * 1024 + 1]);
    let exactly = BASE64.encode(&vec![0u8; 2 * 1024 * 1024]);
    for (args, needle) in [
        (json!({"item": "git", "file_name": "a.txt", "content_base64": "!!!"}), "not valid base64"),
        (json!({"item": "git", "file_name": "a.txt", "content_base64": too_big}), "larger than 2 MB"),
        (json!({"item": "git", "file_name": "../a.txt", "content_base64": "eA=="}), "without a path"),
        (json!({"item": "old", "file_name": "a.txt", "content_base64": "eA=="}), "in the trash"),
        (json!({"item": "nope", "file_name": "a.txt", "content_base64": "eA=="}), "No item with that id"),
    ] {
        let message = refused(&env, "attachment_add", args.clone()).await;
        assert!(message.contains(needle), "{args:.80}: {message}");
    }
    let (_, parked) =
        ask(&env, "attachment_add", json!({"item": "git", "file_name": "max.bin", "content_base64": exactly})).await;
    assert!(parked, "exactly 2 MB is fine");
}

#[tokio::test]
async fn an_attachment_is_deleted_by_id_or_by_name() {
    let env = env().await;
    let (view, done) = write(&env, "attachment_delete", json!({"item": "git", "attachment": "report.txt"})).await;
    assert_eq!(view.preview, ["Delete the attachment \"report.txt\" (20 bytes) of login \"GitHub\" for good"]);
    assert_eq!((done["deleted"].as_bool(), done["attachment"].as_str()), (Some(true), Some("att1")));
    write(&env, "attachment_delete", json!({"item": "git", "attachment": "att2"})).await;
    let deletes: Vec<_> = changes(&env).await;
    assert_eq!(
        deletes,
        [
            ("DELETE".to_owned(), "/api/ciphers/git/attachment/att1".to_owned()),
            ("DELETE".to_owned(), "/api/ciphers/git/attachment/att2".to_owned())
        ]
    );
    let message = refused(&env, "attachment_delete", json!({"item": "git", "attachment": "nothing.txt"})).await;
    assert!(message.contains("no attachment"), "{message}");
}

// ---- organizing -------------------------------------------------------------------------------------------------

#[tokio::test]
async fn favorite_move_trash_restore_archive_and_unarchive_call_the_right_endpoints() {
    let env = env().await;
    let (view, _) = write(&env, "item_favorite", json!({"item": "bank"})).await;
    assert_eq!(view.class, "organize");
    assert_eq!(view.preview, ["Mark login \"Bank\" as a favorite"]);
    // The server sets folder and favorite together: the folder is resent as it is.
    assert_eq!(sent(&env, "PUT", "/api/ciphers/bank/partial").await[0], json!({"folderId": null, "favorite": true}));

    write(&env, "item_favorite", json!({"item": "git", "favorite": false})).await;
    assert_eq!(sent(&env, "PUT", "/api/ciphers/git/partial").await[0], json!({"folderId": "f1", "favorite": false}));

    let (view, _) = write(&env, "item_move", json!({"item": "git", "folder": "Personal"})).await;
    assert_eq!(view.preview, ["Move login \"GitHub\" from Work to Personal"]);
    assert_eq!(sent(&env, "PUT", "/api/ciphers/git/partial").await[1], json!({"folderId": "f2", "favorite": true}));
    write(&env, "item_move", json!({"item": "git", "folder": "none"})).await;
    assert_eq!(sent(&env, "PUT", "/api/ciphers/git/partial").await[2], json!({"folderId": null, "favorite": true}));

    let (view, done) = write(&env, "item_trash", json!({"item": "git"})).await;
    assert_eq!(view.preview, ["Move to the trash: login \"GitHub\""]);
    assert_eq!(done["trashed"], true);
    write(&env, "item_restore", json!({"item": "old"})).await;
    write(&env, "item_archive", json!({"item": "git"})).await;
    write(&env, "item_unarchive", json!({"item": "arch"})).await;
    assert_eq!(
        changes(&env).await.iter().skip(4).cloned().collect::<Vec<_>>(),
        [
            ("PUT".to_owned(), "/api/ciphers/git/delete".to_owned()),
            ("PUT".to_owned(), "/api/ciphers/old/restore".to_owned()),
            ("PUT".to_owned(), "/api/ciphers/git/archive".to_owned()),
            ("PUT".to_owned(), "/api/ciphers/arch/unarchive".to_owned()),
        ]
    );
    // The resource follows the folder and the type, and offers the wider ones.
    let (view, _) = write(&env, "item_trash", json!({"item": "card1"})).await;
    let resources: Vec<(String, bool)> = view.resources.iter().map(|r| (r.id.clone(), r.wider)).collect();
    assert_eq!(resources, [("f2/card/card1".to_owned(), false), ("f2/card".to_owned(), true), ("f2".to_owned(), true)]);
    assert_eq!(view.resources[1].label, "Cards in Personal");
    assert_eq!(view.resources[2].label, "Personal");
}

#[tokio::test]
async fn state_changes_that_do_not_apply_are_refused() {
    let env = env().await;
    for (op, item, needle) in [
        ("item_trash", "old", "in the trash already"),
        ("item_restore", "git", "not in a state"),
        ("item_archive", "arch", "archived already"),
        ("item_unarchive", "git", "not in a state"),
        ("item_trash", "org", "No item with that id"),
    ] {
        let message = refused(&env, op, json!({"item": item})).await;
        assert!(message.contains(needle), "{op} {item}: {message}");
    }
    let message = refused(&env, "item_move", json!({"item": "git", "folder": "Nowhere"})).await;
    assert!(message.contains("No folder with that id or name"), "{message}");
    assert_eq!(changes(&env).await.len(), 0);
}

#[tokio::test]
async fn folders_are_created_renamed_and_deleted_with_encrypted_names() {
    let env = env().await;
    let (view, done) = write(&env, "folder_create", json!({"name": "Travel"})).await;
    assert_eq!(
        (view.class.as_str(), view.preview.clone()),
        ("organize", vec!["Create the folder \"Travel\"".to_owned()])
    );
    assert_eq!(view.resources[0].id, "folders");
    assert_eq!((done["created"].as_bool(), done["id"].as_str()), (Some(true), Some("new-id")));
    let body = &sent(&env, "POST", "/api/folders").await[0];
    assert_eq!(dec(&env.user, &body["name"]), "Travel");
    assert!(!body.to_string().contains("Travel"));

    let (view, _) = write(&env, "folder_rename", json!({"folder": "Work", "name": "Office"})).await;
    assert_eq!(view.preview, ["Rename the folder \"Work\" to \"Office\""]);
    assert_eq!(view.resources[0].id, "f1");
    assert_eq!(dec(&env.user, &sent(&env, "PUT", "/api/folders/f1").await[0]["name"]), "Office");

    let (view, done) = write(&env, "folder_delete", json!({"folder": "f2"})).await;
    assert_eq!(view.preview, ["Delete the folder \"Personal\"", "Its 3 item(s) are kept and end up without a folder."]);
    assert_eq!((done["deleted"].as_bool(), done["items_unfiled"].as_i64()), (Some(true), Some(3)));
    assert_eq!(changes(&env).await.last().unwrap(), &("DELETE".to_owned(), "/api/folders/f2".to_owned()));

    for (op, args, needle) in [
        ("folder_create", json!({"name": "work"}), "exists already"),
        ("folder_create", json!({"name": "None"}), "reserved"),
        ("folder_rename", json!({"folder": "Work", "name": "Personal"}), "exists already"),
        ("folder_rename", json!({"folder": "none", "name": "X"}), "not a folder"),
        ("folder_delete", json!({"folder": "Nowhere"}), "No folder with that id or name"),
    ] {
        let message = refused(&env, op, args).await;
        assert!(message.contains(needle), "{op}: {message}");
    }
}

// ---- permanent deletion -----------------------------------------------------------------------------------------

#[tokio::test]
async fn deleting_for_good_and_emptying_the_trash_are_asked_every_time() {
    let env = env().await;
    let standing = StandingGrant {
        duration_secs: Some(3_600),
        max_uses: None,
        scope: reins_core::GrantScopeChoice {
            all_mail: false,
            selected_messages_only: false,
            sender_addresses: vec![],
            sender_domains: vec![],
            subject_pattern: None,
            recipient_addresses: vec![],
            recipient_domains: vec![],
            resources: vec!["none".into(), "f2".into()],
            classes: vec!["organize".into()],
        },
    };
    let (id, parked) = ask(&env, "item_delete", json!({"item": "old"})).await;
    assert!(parked);
    let view = env.core.approval_view(id.clone()).await.unwrap();
    assert!(view.no_standing, "no standing permission for a permanent deletion");
    assert_eq!(view.preview[0], "Delete login \"Old forum\" for good");
    assert!(view.preview[1].contains("cannot be brought back"));
    assert!(
        env.core
            .approve(
                id.clone(),
                ApprovalChoice {
                    selected_message_ids: vec![],
                    standing: Some(standing.clone()),
                }
            )
            .await
            .is_err()
    );
    assert_eq!(env.core.grants().await.unwrap().len(), 0);
    assert!(changes(&env).await.is_empty(), "nothing is deleted until the user agrees");
    env.core.approve(id, nothing_selected()).await.unwrap();
    assert_eq!(changes(&env).await, [("DELETE".to_owned(), "/api/ciphers/old".to_owned())]);
    assert_eq!(answers(&env).await.last().unwrap()["result"]["data"], json!({"deleted": true, "id": "old"}));

    // Asked again the next time, whatever was granted.
    let (_, parked) = ask(&env, "item_delete", json!({"item": "old"})).await;
    assert!(parked);

    // Emptying the trash: only the user's own trashed items, listed by name, one request.
    let (id, parked) = ask(&env, "trash_empty", json!({})).await;
    assert!(parked);
    let view = env.core.approval_view(id.clone()).await.unwrap();
    assert!(view.no_standing);
    assert_eq!(view.preview[0], "Delete 1 item(s) in the trash for good");
    assert!(view.preview.contains(&"- login \"Old forum\"".to_owned()));
    no_secrets("the trash preview", &view.preview.join("\n"));
    env.core.approve(id, nothing_selected()).await.unwrap();
    let deleted = sent(&env, "DELETE", "/api/ciphers").await;
    assert_eq!(deleted, [json!({"ids": ["old"]})]);
    assert_eq!(answers(&env).await.last().unwrap()["result"]["data"], json!({"emptied": true, "deleted_items": 1}));
}

#[tokio::test]
async fn an_empty_trash_and_an_unknown_item_are_reported() {
    let env = env().await;
    // Nothing in the trash of this vault once the only trashed item is gone: use a vault without one.
    let message = refused(&env, "item_delete", json!({"item": "nope"})).await;
    assert!(message.contains("No item with that id"), "{message}");
    let message = refused(&env, "item_delete", json!({"item": "org"})).await;
    assert!(message.contains("No item with that id"), "{message}");
    assert_eq!(changes(&env).await.len(), 0);
}

// ---- cloning ----------------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_clone_copies_the_secrets_on_the_phone_without_showing_them() {
    let env = env().await;
    let (view, done) = write(&env, "item_clone", json!({"item": "bank"})).await;
    let shown = view.preview.join("\n");
    assert!(shown.starts_with("Copy login \"Bank\" as \"Bank - Clone\" in No folder"), "{shown}");
    assert!(shown.contains("without being shown") && shown.contains("Username: anna"), "{shown}");
    no_secrets("the clone preview", &shown);
    assert_eq!(view.class, "items");
    assert_eq!(view.resources[0].id, "none/login");
    assert_eq!(
        (done["created"].as_bool(), done["copied_from"].as_str(), done["id"].as_str()),
        (Some(true), Some("bank"), Some("new-id"))
    );
    let body = &sent(&env, "POST", "/api/ciphers").await[0];
    // The copy shares the item's key, so its copied strings stay readable; the new name is encrypted with it.
    assert_eq!(env.user.decrypt(body["key"].as_str().unwrap()).unwrap().to_vec(), env.own.to_bytes().to_vec());
    let o = &env.own;
    assert_eq!(dec(o, &body["name"]), "Bank - Clone");
    assert_eq!(dec(o, &body["login"]["password"]), "s3cret");
    assert_eq!(dec(o, &body["login"]["username"]), "anna");
    assert_eq!(dec(o, &body["notes"]), "seed phrase words");
    assert_eq!(dec(o, &body["fields"][0]["value"]), "north");
    assert_eq!((body["favorite"].as_bool(), body["type"].as_i64()), (Some(false), Some(1)));
    assert!(body["passwordHistory"].is_null() && body["archivedDate"].is_null() && body.get("id").is_none());

    let (view, _) =
        write(&env, "item_clone", json!({"item": "git", "name": "GitHub (work)", "folder": "Personal"})).await;
    assert!(view.preview[0].contains("\"GitHub (work)\" in Personal"), "{:?}", view.preview);
    assert_eq!(view.resources[0].id, "f2/login");
    let body = &sent(&env, "POST", "/api/ciphers").await[1];
    assert_eq!((body["folderId"].as_str(), body["key"].is_null()), (Some("f2"), true));
    assert_eq!(dec(&env.user, &body["name"]), "GitHub (work)");
    assert_eq!(dec(&env.user, &body["login"]["password"]), "hunter2!");
    assert_eq!(body["fields"].as_array().unwrap().len(), 3, "custom fields are copied");
    assert!(body["passwordHistory"].is_null());

    let message = refused(&env, "item_clone", json!({"item": "old"})).await;
    assert!(message.contains("in the trash"), "{message}");
}

// ---- nothing changes before the user agrees ---------------------------------------------------------------------

#[tokio::test]
async fn previews_change_nothing_and_a_denied_write_sends_nothing() {
    let env = env().await;
    for (op, args) in [
        ("item_create", json!({"type": "note", "name": "x"})),
        ("item_update", json!({"item": "git", "name": "x"})),
        ("item_trash", json!({"item": "git"})),
        ("attachment_add", json!({"item": "git", "file_name": "a", "content_base64": "eA=="})),
        ("folder_create", json!({"name": "Y"})),
        ("item_clone", json!({"item": "git"})),
    ] {
        let (id, parked) = ask(&env, op, args).await;
        assert!(parked, "{op}");
        let view = env.core.approval_view(id.clone()).await.unwrap();
        assert_ne!(view.preview.len(), 0);
        env.core.deny(id).await.unwrap();
    }
    assert_eq!(changes(&env).await.len(), 0);
}

#[tokio::test]
async fn an_empty_trash_is_reported_and_nothing_is_sent() {
    let env = env().await;
    // A vault whose trash has been emptied.
    let ciphers: Vec<Value> = vault_ciphers(&env.user, &env.own, &env.att_key, &env.server.uri())
        .into_iter()
        .filter(|c| c["deletedDate"].is_null())
        .collect();
    Mock::given(method("GET"))
        .and(path("/api/sync"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ciphers": ciphers, "folders": []})))
        .with_priority(1)
        .mount(&env.server)
        .await;
    let message = refused(&env, "trash_empty", json!({})).await;
    assert!(message.contains("trash is empty"), "{message}");
    assert_eq!(changes(&env).await.len(), 0);
}

#[tokio::test]
async fn a_standing_permission_for_a_folder_covers_that_folder_and_that_kind_of_change_only() {
    let env = env().await;
    let standing = StandingGrant {
        duration_secs: Some(3_600),
        max_uses: None,
        scope: reins_core::GrantScopeChoice {
            all_mail: false,
            selected_messages_only: false,
            sender_addresses: vec![],
            sender_domains: vec![],
            subject_pattern: None,
            recipient_addresses: vec![],
            recipient_domains: vec![],
            resources: vec!["f1".into()],
            classes: vec!["organize".into()],
        },
    };
    let (id, parked) = ask(&env, "item_favorite", json!({"item": "git", "favorite": false})).await;
    assert!(parked);
    let view = env.core.approval_view(id.clone()).await.unwrap();
    assert!(view.resources.iter().any(|r| r.id == "f1" && r.wider), "the folder can be granted: {:?}", view.resources);
    env.core
        .approve(
            id,
            ApprovalChoice {
                selected_message_ids: vec![],
                standing: Some(standing),
            },
        )
        .await
        .unwrap();
    // Another item of the folder, the same kind of change: no question.
    let (_, parked) = ask(&env, "item_trash", json!({"item": "note"})).await;
    assert!(!parked);
    assert!(changes(&env).await.contains(&("PUT".to_owned(), "/api/ciphers/note/delete".to_owned())));
    // Another folder, or another kind of change: asked.
    assert!(ask(&env, "item_trash", json!({"item": "card1"})).await.1);
    assert!(ask(&env, "item_update", json!({"item": "git", "name": "x"})).await.1);
    // A permanent deletion is never covered.
    assert!(ask(&env, "item_delete", json!({"item": "git"})).await.1);
}
