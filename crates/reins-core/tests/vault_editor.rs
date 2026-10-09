//! The vault on the phone: the list, an item's fields (secrets only when asked), adding, changing and deleting items,
//! SSH keys made on the phone; and `reins vault add`, whose value the desktop app seals to the phone's own key so that
//! the server relays it without being able to read it. A mock Vaultwarden serves an encrypted vault.

mod common;

use common::desktop::{DESK, Desk, REFUSED, call, choice, desk_with, mount_reins};
use crypto_box::aead::{Aead as _, AeadCore as _, OsRng};
use crypto_box::{PublicKey, SalsaBox, SecretKey};
use data_encoding::BASE64URL_NOPAD;
use reins_core::crypto::{Kdf, VaultKey, master_key};
use reins_core::vault_editor::{VaultFieldInput, VaultItemInput, VaultItemKind};
use reins_core::{ApprovalKind, CoreConfig};
use reins_proto::desktop::{PHONE_KEY_CHANGED, PhoneKey, SecretToStore, VaultNames, decode_key, phone_key_fingerprint};
use serde_json::{Value, json};
use ssh_key::PrivateKey;
use wiremock::matchers::{header_exists, method, path, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

const EMAIL: &str = "me@example.com";
const PASSWORD: &str = "correct horse battery";

fn t(key: &VaultKey, s: &str) -> Value {
    json!(key.encrypt(s.as_bytes()).unwrap())
}

fn cipher(key: &VaultKey, id: &str, kind: i64, name: &str) -> Value {
    json!({"id": id, "type": kind, "name": t(key, name), "notes": null, "key": null, "folderId": null,
        "favorite": false, "deletedDate": null, "archivedDate": null, "organizationId": null,
        "reprompt": 0, "fields": [], "passwordHistory": [], "attachments": null, "revisionDate": "2026-01-01T00:00:00Z",
        "login": null, "secureNote": null, "card": null, "identity": null, "sshKey": null})
}

fn login(key: &VaultKey, id: &str, name: &str, username: &str, password: &str) -> Value {
    let mut c = cipher(key, id, 1, name);
    c["login"] = json!({"username": t(key, username), "password": t(key, password), "totp": null, "uris": null});
    c
}

fn user_key() -> VaultKey {
    VaultKey::from_bytes(&[7u8; 64]).unwrap()
}

fn ciphers(user: &VaultKey) -> Vec<Value> {
    let mut openai = login(user, "openai", "OpenAI", "", "sk-live-123");
    openai["login"]["uris"] = json!([{"uri": t(user, "https://platform.openai.com"), "match": null}]);
    openai["fields"] = json!([{"name": t(user, "Org"), "value": t(user, "org-9"), "type": 1, "linkedId": null}]);
    let mut note = cipher(user, "note", 2, "Stripe");
    note["secureNote"] = json!({"type": 0});
    note["notes"] = t(user, "rk_live_456");
    let mut card = cipher(user, "card", 3, "Visa");
    card["card"] = json!({"cardholderName": t(user, "Anna Smith"), "number": t(user, "4111111111111234"),
        "brand": null, "expMonth": t(user, "4"), "expYear": t(user, "2030"), "code": t(user, "123")});
    let mut old = login(user, "old", "Old", "x", "old-pw");
    old["deletedDate"] = json!("2026-03-01T00:00:00.000000Z");
    vec![
        openai,
        login(user, "bank", "Bank", "anna", "s3cret"),
        note,
        card,
        login(user, "twin1", "Twin", "a", "pw-a"),
        login(user, "twin2", "Twin", "b", "pw-b"),
        old,
    ]
}

async fn desk() -> Desk {
    let server = MockServer::start().await;
    mount_reins(&server).await;
    let master = master_key(
        PASSWORD,
        EMAIL,
        Kdf::Pbkdf2 {
            iterations: 5000,
        },
    )
    .unwrap();
    let user = user_key();
    let wrapped = master.stretch().encrypt(&user.to_bytes()).unwrap();
    Mock::given(method("GET"))
        .and(path("/api/sync"))
        .and(header_exists("Bitwarden-Client-Version"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"profile": {"key": wrapped}, "ciphers": ciphers(&user), "folders": []})),
        )
        .with_priority(3)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/sync"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"profile": {"key": wrapped}, "ciphers": [], "folders": []})),
        )
        .with_priority(4)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/ciphers"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "new-item"})))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path_regex(r"^/api/ciphers/[a-z0-9-]+$"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path_regex(r"^/api/ciphers/[a-z0-9-]+$"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let desk = desk_with(server, EMAIL, PASSWORD, CoreConfig::default()).await;
    desk.core.add_token_account("vault".to_owned(), PASSWORD.to_owned()).await.unwrap();
    desk
}

/// The bodies sent with `verb` to `on`.
async fn sent(desk: &Desk, verb: &str, on: &str) -> Vec<Value> {
    desk.server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.method.as_str() == verb && r.url.path() == on)
        .map(|r| serde_json::from_slice(&r.body).unwrap_or(Value::Null))
        .collect()
}

fn dec(v: &Value) -> String {
    user_key().decrypt_text(v.as_str().unwrap()).unwrap().to_string()
}

fn field(key: &str, value: &str) -> VaultFieldInput {
    VaultFieldInput {
        key: key.to_owned(),
        value: value.to_owned(),
    }
}

// ---- the phone's own screens ----------------------------------------------------------------------------------------

#[tokio::test]
async fn the_list_is_searchable_by_name_and_leaves_the_trash_out() {
    let desk = desk().await;
    let all = desk.core.vault_items(String::new()).await.unwrap();
    let names: Vec<&str> = all.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(names, ["Bank", "OpenAI", "Stripe", "Twin", "Twin", "Visa"]);
    let visa = all.iter().find(|i| i.name == "Visa").unwrap();
    assert_eq!((visa.kind, visa.subtitle.as_str()), (VaultItemKind::Card, "Anna Smith · •••• 1234"));
    assert_eq!(all.iter().find(|i| i.name == "OpenAI").unwrap().subtitle, "platform.openai.com");
    let found = desk.core.vault_items("anna".to_owned()).await.unwrap();
    assert_eq!(found.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), ["Bank", "Visa"]);
    let found = desk.core.vault_items("OPENAI.com".to_owned()).await.unwrap();
    assert_eq!(found.len(), 1);
}

#[tokio::test]
async fn an_item_shows_plain_fields_hides_secrets_and_says_how_the_computer_names_it() {
    let desk = desk().await;
    let item = desk.core.vault_item("openai".to_owned()).await.unwrap();
    let fields: Vec<(&str, Option<&str>, bool)> =
        item.fields.iter().map(|f| (f.key.as_str(), f.value.as_deref(), f.secret)).collect();
    assert_eq!(
        fields,
        [("password", None, true), ("uris", Some("https://platform.openai.com"), false), ("custom:Org", None, true)]
    );
    let uses: Vec<&str> = item.uses.iter().map(|u| u.reference.as_str()).collect();
    assert_eq!(uses, ["vault:OpenAI/password", "vault:OpenAI/Org"]);
    assert_eq!(item.warning, None);
    assert!(!format!("{item:?}").contains("sk-live-123"));

    assert_eq!(desk.core.vault_reveal("openai".to_owned(), "password".to_owned()).await.unwrap(), "sk-live-123");
    assert_eq!(desk.core.vault_reveal("openai".to_owned(), "custom:Org".to_owned()).await.unwrap(), "org-9");
    assert!(desk.core.vault_reveal("openai".to_owned(), "username".to_owned()).await.is_err(), "not set");
    assert!(desk.core.vault_reveal("old".to_owned(), "password".to_owned()).await.is_err(), "in the trash");

    let note = desk.core.vault_item("note".to_owned()).await.unwrap();
    assert_eq!(note.uses[0].reference, "vault:Stripe/notes");
    assert_eq!(desk.core.vault_reveal("note".to_owned(), "notes".to_owned()).await.unwrap(), "rk_live_456");
    let twin = desk.core.vault_item("twin1".to_owned()).await.unwrap();
    assert!(twin.warning.unwrap().starts_with("2 items are named"));
    let card = desk.core.vault_item("card".to_owned()).await.unwrap();
    let card_fields: Vec<(&str, bool)> = card.fields.iter().map(|f| (f.key.as_str(), f.secret)).collect();
    assert_eq!(
        card_fields,
        [("holder", false), ("number", true), ("exp_month", false), ("exp_year", false), ("code", true)]
    );
}

#[tokio::test]
async fn items_are_created_changed_and_deleted_encrypted() {
    let desk = desk().await;
    let input = VaultItemInput {
        kind: VaultItemKind::Login,
        name: "Anthropic".to_owned(),
        fields: vec![field("password", "sk-ant-1"), field("uris", "https://console.example.com")],
    };
    assert_eq!(desk.core.vault_create(input).await.unwrap(), "new-item");
    let created = &sent(&desk, "POST", "/api/ciphers").await[0];
    assert_eq!(created["type"], 1);
    assert_eq!(dec(&created["name"]), "Anthropic");
    assert_eq!(dec(&created["login"]["password"]), "sk-ant-1");
    assert_eq!(dec(&created["login"]["uris"][0]["uri"]), "https://console.example.com");
    assert!(!created.to_string().contains("sk-ant-1"), "only ciphertext reaches the server");

    let change = VaultItemInput {
        kind: VaultItemKind::Login,
        name: "Bank of Anna".to_owned(),
        fields: vec![field("password", "n3w"), field("custom:PIN", "4321")],
    };
    desk.core.vault_update("bank".to_owned(), change).await.unwrap();
    let updated = &sent(&desk, "PUT", "/api/ciphers/bank").await[0];
    assert_eq!(dec(&updated["name"]), "Bank of Anna");
    assert_eq!(dec(&updated["login"]["password"]), "n3w");
    assert_eq!(dec(&updated["login"]["username"]), "anna", "what was not given stays");
    assert_eq!(dec(&updated["passwordHistory"][0]["password"]), "s3cret");
    assert_eq!((dec(&updated["fields"][0]["name"]).as_str(), updated["fields"][0]["type"].as_i64()), ("PIN", Some(1)));

    desk.core.vault_delete("note".to_owned()).await.unwrap();
    assert_eq!(sent(&desk, "DELETE", "/api/ciphers/note").await.len(), 1);
    assert!(desk.core.vault_delete("old".to_owned()).await.is_err(), "already in the trash");
    let nameless = VaultItemInput {
        kind: VaultItemKind::Note,
        name: "  ".to_owned(),
        fields: vec![],
    };
    assert!(desk.core.vault_create(nameless).await.is_err());
}

#[tokio::test]
async fn an_ssh_key_is_made_on_the_phone_and_only_its_public_half_comes_back() {
    let desk = desk().await;
    let made = desk.core.vault_generate_ssh_key("Deploy".to_owned()).await.unwrap();
    assert!(made.public_key.starts_with("ssh-ed25519 AAAA") && made.public_key.ends_with(" Deploy"));
    assert!(made.fingerprint.starts_with("SHA256:"));
    let body = &sent(&desk, "POST", "/api/ciphers").await[0];
    assert_eq!(body["type"], 5);
    let private = PrivateKey::from_openssh(dec(&body["sshKey"]["privateKey"])).unwrap();
    assert_eq!(private.public_key().to_openssh().unwrap(), made.public_key);
    assert_eq!(dec(&body["sshKey"]["publicKey"]), made.public_key);
    assert_eq!(dec(&body["sshKey"]["keyFingerprint"]), made.fingerprint);
}

// ---- reins vault add / list ------------------------------------------------------------------------------------------

/// Asks for the phone's key, approves it, and returns it as the app opens it.
async fn phone_key(desk: &Desk) -> String {
    desk.send(&[call("k1", DESK, "vault", "phone_key", &json!({"client_key": desk.public(), "nonce": "nk"}))]).await;
    let view = desk.core.approval_view("k1".to_owned()).await.unwrap();
    let digits = desk.core.phone_key_fingerprint().await.unwrap();
    assert_eq!(view.preview[1], format!("This phone's key: {digits}"));
    desk.core.approve("k1".to_owned(), choice(&[], None)).await.unwrap();
    let answer: PhoneKey = desk.open(&desk.data("k1").await["sealed"]);
    assert_eq!(answer.nonce, "nk");
    assert_eq!(phone_key_fingerprint(&answer.public_key).unwrap(), digits, "the computer shows the same digits");
    answer.public_key
}

/// The value in a box from `from` (the app's key, or a forger's) to the phone's key, as `reins vault add` sends it.
fn box_from(from: &SecretKey, phone: &str, nonce: &str, name: &str, field: &str, value: &str) -> String {
    let secret = SecretToStore {
        v: 1,
        nonce: nonce.to_owned(),
        name: name.to_owned(),
        field: field.to_owned(),
        value: value.to_owned(),
    };
    let cipher = SalsaBox::new(&PublicKey::from(decode_key(phone).unwrap()), from);
    let nonce = SalsaBox::generate_nonce(&mut OsRng);
    let mut out = nonce.to_vec();
    out.extend(cipher.encrypt(&nonce, serde_json::to_vec(&secret).unwrap().as_slice()).unwrap());
    BASE64URL_NOPAD.encode(&out)
}

fn store(desk: &Desk, id: &str, phone: &str, name: &str, kind: &str, field: &str, value: &str) -> Value {
    let nonce = format!("nonce-{id}");
    let boxed = box_from(&desk.key, phone, &nonce, name, field, value);
    store_boxed(desk, id, name, kind, field, &boxed)
}

fn store_boxed(desk: &Desk, id: &str, name: &str, kind: &str, field: &str, boxed: &str) -> Value {
    call(
        id,
        DESK,
        "vault",
        "secret_store",
        &json!({"name": name, "kind": kind, "field": field, "sealed": boxed,
            "client_key": desk.public(), "nonce": format!("nonce-{id}")}),
    )
}

#[tokio::test]
async fn a_secret_typed_on_the_computer_is_saved_without_the_server_seeing_it() {
    let desk = desk().await;
    desk.pair().await;
    let phone = phone_key(&desk).await;
    desk.send(&[store(&desk, "s1", &phone, "Groq", "api-key", "password", "gsk_typed_42")]).await;
    let view = desk.core.approval_view("s1".to_owned()).await.unwrap();
    assert_eq!(view.kind, ApprovalKind::Write);
    assert_eq!(
        view.preview,
        [
            "Save a new API key \u{201c}Groq\u{201d} in your vault",
            "The password: 12 characters, typed on the computer (not shown)",
            "Use it as vault:Groq/password",
        ]
    );
    desk.core.approve("s1".to_owned(), choice(&[], None)).await.unwrap();
    let data = desk.data("s1").await;
    assert_eq!((data["saved"].as_bool(), data["created"].as_bool()), (Some(true), Some(true)));
    let created = &sent(&desk, "POST", "/api/ciphers").await[0];
    assert_eq!((dec(&created["name"]).as_str(), created["type"].as_i64()), ("Groq", Some(1)));
    assert_eq!(dec(&created["login"]["password"]), "gsk_typed_42");
    let shown = desk.everything_shown(&["k1", "s1"]).await;
    assert!(!shown.contains("gsk_typed_42"), "never in the clear");
    assert!(!shown.contains(&phone), "the phone's key travels sealed only, so nobody can aim for its digits");

    // The same name again changes that item's field.
    desk.send(&[store(&desk, "s2", &phone, "Bank", "api-key", "password", "rotated")]).await;
    let view = desk.core.approval_view("s2".to_owned()).await.unwrap();
    assert_eq!(view.preview[0], "Change the password of \u{201c}Bank\u{201d} in your vault");
    desk.core.approve("s2".to_owned(), choice(&[], None)).await.unwrap();
    assert_eq!(dec(&sent(&desk, "PUT", "/api/ciphers/bank").await[0]["login"]["password"]), "rotated");
}

#[tokio::test]
async fn a_value_for_another_key_or_request_or_an_unpaired_app_is_refused() {
    let desk = desk().await;
    let stranger = PublicKey::from(SecretKey::generate(&mut OsRng).public_key().to_bytes());
    let other = reins_proto::desktop::encode_key(stranger.as_bytes());
    desk.send(&[store(&desk, "u1", &other, "X", "api-key", "password", "v")]).await;
    assert_eq!(desk.error("u1").await, REFUSED);

    desk.pair().await;
    let phone = phone_key(&desk).await;
    let mut replayed = store(&desk, "w2", &phone, "X", "api-key", "password", "v");
    replayed["call"]["args"]["nonce"] = json!("another");
    let cases = [
        ("w1", store(&desk, "w1", &other, "X", "api-key", "password", "v"), PHONE_KEY_CHANGED),
        ("w2", replayed, "another request"),
        ("w3", store(&desk, "w3", &phone, "Twin", "api-key", "password", "v"), "Several vault items are named Twin"),
        ("w4", store(&desk, "w4", &phone, "Stripe", "api-key", "password", "v"), "which has no password"),
        ("w5", store(&desk, "w5", &phone, "K", "ssh", "private_key", "not a key"), "not an OpenSSH private key"),
        ("w6", store(&desk, "w6", &phone, "N", "note", "password", "v"), "has no password"),
    ];
    // The server knows both public keys: what it boxes itself, or a box moved to another item, does not open.
    let server_key = SecretKey::generate(&mut OsRng);
    let forged = box_from(&server_key, &phone, "nonce-w7", "X", "password", "attacker-key");
    let moved = box_from(&desk.key, &phone, "nonce-w8", "Other", "password", "v");
    let cases = [
        cases.to_vec(),
        vec![
            ("w7", store_boxed(&desk, "w7", "X", "api-key", "password", &forged), PHONE_KEY_CHANGED),
            ("w8", store_boxed(&desk, "w8", "X", "api-key", "password", &moved), "another item or field"),
        ],
    ]
    .concat();
    let requests: Vec<Value> = cases.iter().map(|(_, r, _)| r.clone()).collect();
    desk.send(&requests).await;
    for (id, _, says) in cases {
        let message = desk.error(id).await;
        assert!(message.contains(says), "{id}: {message}");
    }
    assert!(desk.waiting().await.is_empty());
}

#[tokio::test]
async fn the_computer_lists_names_only_sealed_to_it() {
    let desk = desk().await;
    desk.pair().await;
    desk.send(&[call("n1", DESK, "vault", "names", &json!({"client_key": desk.public(), "nonce": "nn"}))]).await;
    let view = desk.core.approval_view("n1".to_owned()).await.unwrap();
    let ids: Vec<String> = view.messages.iter().map(|m| m.id.clone()).collect();
    desk.core
        .approve("n1".to_owned(), choice(&ids.iter().map(String::as_str).collect::<Vec<_>>(), None))
        .await
        .unwrap();
    let data = desk.data("n1").await;
    let names: VaultNames = desk.open(&data["items"][0]["sealed"]);
    assert_eq!(names.nonce, "nn");
    let listed: Vec<(&str, &str)> = names.items.iter().map(|(n, k)| (n.as_str(), k.as_str())).collect();
    assert_eq!(
        listed,
        [
            ("Bank", "login"),
            ("OpenAI", "login"),
            ("Stripe", "note"),
            ("Twin", "login"),
            ("Twin", "login"),
            ("Visa", "card")
        ]
    );
    assert!(!data.to_string().contains("OpenAI"), "the names travel sealed");
}
