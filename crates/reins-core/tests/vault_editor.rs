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
use reins_proto::desktop::{
    PHONE_KEY_CHANGED, PhoneKey, SecretToStore, StoreAck, VaultNames, decode_key, phone_key_fingerprint, value_check,
};
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
    let mut bank = login(user, "bank", "Bank", "anna", "s3cret");
    bank["fields"] = json!([{"name": t(user, "Region"), "value": t(user, "eu"), "type": 0, "linkedId": null}]);
    let mut ssh = cipher(user, "ssh", 5, "Deploy key");
    ssh["sshKey"] = json!({"privateKey": t(user, "-----BEGIN OPENSSH PRIVATE KEY-----"),
        "publicKey": t(user, "ssh-ed25519 AAAA"), "keyFingerprint": t(user, "SHA256:x")});
    vec![
        openai,
        bank,
        ssh,
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
async fn sent_bodies(desk: &Desk, verb: &str, on: &str) -> Vec<Value> {
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
    assert_eq!(names, ["Bank", "Deploy key", "OpenAI", "Stripe", "Twin", "Twin", "Visa"]);
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
    let created = &sent_bodies(&desk, "POST", "/api/ciphers").await[0];
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
    let updated = &sent_bodies(&desk, "PUT", "/api/ciphers/bank").await[0];
    assert_eq!(dec(&updated["name"]), "Bank of Anna");
    assert_eq!(dec(&updated["login"]["password"]), "n3w");
    assert_eq!(dec(&updated["login"]["username"]), "anna", "what was not given stays");
    assert_eq!(dec(&updated["passwordHistory"][0]["password"]), "s3cret");
    assert_eq!((dec(&updated["fields"][1]["name"]).as_str(), updated["fields"][1]["type"].as_i64()), ("PIN", Some(1)));

    desk.core.vault_delete("note".to_owned()).await.unwrap();
    assert_eq!(sent_bodies(&desk, "DELETE", "/api/ciphers/note").await.len(), 1);
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
    let body = &sent_bodies(&desk, "POST", "/api/ciphers").await[0];
    assert_eq!(body["type"], 5);
    let private = PrivateKey::from_openssh(dec(&body["sshKey"]["privateKey"])).unwrap();
    assert_eq!(private.public_key().to_openssh().unwrap(), made.public_key);
    assert_eq!(dec(&body["sshKey"]["publicKey"]), made.public_key);
    assert_eq!(dec(&body["sshKey"]["keyFingerprint"]), made.fingerprint);
    // The phone signs with it; nothing hands the private key out, not even to the phone's own screens.
    let refused = desk.core.vault_reveal(made.id, "private_key".to_owned()).await.unwrap_err();
    assert!(refused.to_string().contains("not shown"), "{refused}");
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

/// What `reins vault add` puts in the box.
struct Sent<'a> {
    name: &'a str,
    field: &'a str,
    kind: &'a str,
    replace: bool,
    value: &'a str,
    created_at: i64,
}

impl<'a> Sent<'a> {
    fn new(name: &'a str, kind: &'a str, field: &'a str, value: &'a str) -> Self {
        Self {
            name,
            field,
            kind,
            replace: false,
            value,
            created_at: reins_core::store::unix_now(),
        }
    }

    fn replace(mut self) -> Self {
        self.replace = true;
        self
    }
}

/// The value in a box from `from` (the app's key, or a forger's) to the phone's key, as `reins vault add` sends it.
fn box_from(from: &SecretKey, phone: &str, nonce: &str, sent: &Sent<'_>) -> String {
    let secret = SecretToStore {
        v: 1,
        dir: reins_proto::desktop::TO_PHONE.to_owned(),
        nonce: nonce.to_owned(),
        created_at: sent.created_at,
        name: sent.name.to_owned(),
        field: sent.field.to_owned(),
        kind: sent.kind.to_owned(),
        replace: sent.replace,
        value: sent.value.to_owned(),
    };
    let cipher = SalsaBox::new(&PublicKey::from(decode_key(phone).unwrap()), from);
    let nonce = SalsaBox::generate_nonce(&mut OsRng);
    let mut out = nonce.to_vec();
    out.extend(cipher.encrypt(&nonce, serde_json::to_vec(&secret).unwrap().as_slice()).unwrap());
    BASE64URL_NOPAD.encode(&out)
}

fn store(desk: &Desk, id: &str, phone: &str, sent: &Sent<'_>) -> Value {
    let boxed = box_from(&desk.key, phone, &format!("nonce-{id}"), sent);
    store_boxed(desk, id, &format!("nonce-{id}"), sent, &boxed)
}

fn store_boxed(desk: &Desk, id: &str, nonce: &str, sent: &Sent<'_>, boxed: &str) -> Value {
    call(
        id,
        DESK,
        "vault",
        "secret_store",
        &json!({"name": sent.name, "kind": sent.kind, "field": sent.field, "sealed": boxed,
            "client_key": desk.public(), "nonce": nonce}),
    )
}

/// Opens what the phone boxed for the app from its own key (an acknowledgement, the names).
fn open_from_phone<T: serde::de::DeserializeOwned>(desk: &Desk, phone: &str, boxed: &Value) -> T {
    let bytes = BASE64URL_NOPAD.decode(boxed.as_str().unwrap().as_bytes()).unwrap();
    let (nonce, ciphertext) = bytes.split_at(24);
    let nonce = crypto_box::Nonce::from(<[u8; 24]>::try_from(nonce).unwrap());
    let plain = SalsaBox::new(&PublicKey::from(decode_key(phone).unwrap()), &desk.key)
        .decrypt(&nonce, ciphertext)
        .expect("boxed by the pinned phone");
    serde_json::from_slice(&plain).unwrap()
}

#[tokio::test]
async fn a_secret_sent_from_the_computer_is_saved_without_the_server_seeing_it() {
    let desk = desk().await;
    desk.pair().await;
    let phone = phone_key(&desk).await;
    assert_eq!(phone_key_fingerprint(&phone).unwrap().len(), 14, "twelve digits in three groups");
    let sent = Sent::new("Groq", "api-key", "password", "gsk_typed_42");
    desk.send(&[store(&desk, "s1", &phone, &sent)]).await;
    let view = desk.core.approval_view("s1".to_owned()).await.unwrap();
    assert_eq!(view.kind, ApprovalKind::Write);
    assert_eq!(
        view.preview,
        [
            "Save a new API key \u{201c}Groq\u{201d} in your vault".to_owned(),
            "The password: 12 characters, sent by reins vault add on the computer (typed or piped in; not shown)"
                .to_owned(),
            format!(
                "Check: {}. The terminal shows the same four digits; deny if it does not.",
                value_check("gsk_typed_42")
            ),
            "Use it as vault:Groq/password".to_owned(),
        ]
    );
    desk.core.approve("s1".to_owned(), choice(&[], None)).await.unwrap();
    let ack: StoreAck = open_from_phone(&desk, &phone, &desk.data("s1").await["sealed"]);
    assert_eq!((ack.nonce.as_str(), ack.name.as_str(), ack.created, ack.kept), ("nonce-s1", "Groq", true, None));
    let created = &sent_bodies(&desk, "POST", "/api/ciphers").await[0];
    assert_eq!((dec(&created["name"]).as_str(), created["type"].as_i64()), ("Groq", Some(1)));
    assert_eq!(dec(&created["login"]["password"]), "gsk_typed_42");
    let shown = desk.everything_shown(&["k1", "s1"]).await;
    assert!(!shown.contains("gsk_typed_42"), "never in the clear");
    assert!(!shown.contains(&phone), "the phone's key travels sealed only, so nobody can aim for its digits");

    // The same request again (the server keeps a copy) is never saved twice.
    desk.send(&[store_boxed(&desk, "s1b", "nonce-s1", &sent, &box_from(&desk.key, &phone, "nonce-s1", &sent))]).await;
    let again = desk.error("s1b").await;
    assert!(again.contains("seen already"), "{again}");
}

#[tokio::test]
async fn an_existing_item_is_changed_only_when_asked_and_its_earlier_value_is_kept() {
    let desk = desk().await;
    desk.pair().await;
    let phone = phone_key(&desk).await;
    // Without --replace an item that exists is refused before the user is asked.
    desk.send(&[store(&desk, "r1", &phone, &Sent::new("Bank", "api-key", "password", "attacker"))]).await;
    let told = desk.error("r1").await;
    assert!(told.contains("Bank is already in your vault") && told.contains("--replace"), "{told}");

    desk.send(&[store(&desk, "r2", &phone, &Sent::new("Bank", "api-key", "password", "rotated").replace())]).await;
    let view = desk.core.approval_view("r2".to_owned()).await.unwrap();
    assert_eq!(view.preview[0], "Replace the password of \u{201c}Bank\u{201d} in your vault");
    assert!(view.preview.contains(&"The earlier value is kept in the item's password history".to_owned()));
    desk.core.approve("r2".to_owned(), choice(&[], None)).await.unwrap();
    let ack: StoreAck = open_from_phone(&desk, &phone, &desk.data("r2").await["sealed"]);
    assert!(!ack.created);
    let put = &sent_bodies(&desk, "PUT", "/api/ciphers/bank").await[0];
    assert_eq!(dec(&put["login"]["password"]), "rotated");
    assert_eq!(dec(&put["passwordHistory"][0]["password"]), "s3cret");

    // A note or a custom field keeps the earlier value in a hidden field; the new value is hidden too.
    desk.send(&[
        store(&desk, "r3", &phone, &Sent::new("Stripe", "note", "notes", "rk_new").replace()),
        store(&desk, "r4", &phone, &Sent::new("OpenAI", "api-key", "Org", "org-new").replace()),
    ])
    .await;
    for id in ["r3", "r4"] {
        desk.core.approve(id.to_owned(), choice(&[], None)).await.unwrap();
    }
    let note = &sent_bodies(&desk, "PUT", "/api/ciphers/note").await[0];
    assert_eq!(dec(&note["notes"]), "rk_new");
    let kept = &note["fields"][0];
    assert!(dec(&kept["name"]).starts_with("Notes before "), "{kept}");
    assert_eq!((dec(&kept["value"]).as_str(), kept["type"].as_i64()), ("rk_live_456", Some(1)));
    let openai = &sent_bodies(&desk, "PUT", "/api/ciphers/openai").await[0];
    let fields: Vec<(String, String, i64)> = openai["fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| (dec(&f["name"]), dec(&f["value"]), f["type"].as_i64().unwrap()))
        .collect();
    assert_eq!(fields[0], ("Org".to_owned(), "org-new".to_owned(), 1));
    assert!(fields[1].0.starts_with("Org before ") && fields[1].1 == "org-9" && fields[1].2 == 1, "{fields:?}");

    // A text field that receives a secret becomes hidden, and keeps its earlier value hidden too.
    desk.send(&[store(&desk, "r5", &phone, &Sent::new("Bank", "api-key", "Region", "secret-region").replace())]).await;
    desk.core.approve("r5".to_owned(), choice(&[], None)).await.unwrap();
    let bank = sent_bodies(&desk, "PUT", "/api/ciphers/bank").await.pop().unwrap();
    let region = &bank["fields"][0];
    assert_eq!((dec(&region["name"]).as_str(), region["type"].as_i64()), ("Region", Some(1)), "{region}");

    // An SSH key is never replaced, even when asked.
    desk.send(&[store(&desk, "r6", &phone, &Sent::new("Deploy key", "ssh", "private_key", "x").replace())]).await;
    let told = desk.error("r6").await;
    assert!(told.contains("is an SSH key, which is not replaced"), "{told}");
}

#[tokio::test]
async fn the_vault_changing_between_the_approval_and_the_save_saves_nothing() {
    let desk = desk().await;
    desk.pair().await;
    let phone = phone_key(&desk).await;
    desk.send(&[store(&desk, "c1", &phone, &Sent::new("Bank", "api-key", "password", "rotated").replace())]).await;
    assert_eq!(desk.waiting().await, ["c1"]);
    // Before the user approves, the item changes (another device saved it).
    let user = user_key();
    let mut changed = ciphers(&user);
    changed[1]["revisionDate"] = json!("2026-10-09T00:00:00Z");
    Mock::given(method("GET"))
        .and(path("/api/sync"))
        .and(header_exists("Bitwarden-Client-Version"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"profile": {"key": "x"}, "ciphers": changed, "folders": []})),
        )
        .with_priority(1)
        .mount(&desk.server)
        .await;
    let refused = desk.core.approve("c1".to_owned(), choice(&[], None)).await.unwrap_err();
    assert!(refused.to_string().contains("The vault changed since you were asked"), "{refused}");
    assert!(sent_bodies(&desk, "PUT", "/api/ciphers/bank").await.is_empty());
}

#[tokio::test]
async fn a_value_for_another_key_request_time_or_kind_or_an_unpaired_app_is_refused() {
    let desk = desk().await;
    let stranger = PublicKey::from(SecretKey::generate(&mut OsRng).public_key().to_bytes());
    let other = reins_proto::desktop::encode_key(stranger.as_bytes());
    let x = Sent::new("X", "api-key", "password", "v");
    desk.send(&[store(&desk, "u1", &other, &x)]).await;
    assert_eq!(desk.error("u1").await, REFUSED);

    desk.pair().await;
    let phone = phone_key(&desk).await;
    let mut renonced = store(&desk, "w2", &phone, &x);
    renonced["call"]["args"]["nonce"] = json!("another");
    let mut rekinded = store(&desk, "w9", &phone, &x);
    rekinded["call"]["args"]["kind"] = json!("note");
    let old = Sent {
        created_at: reins_core::store::unix_now() - 3_600,
        ..Sent::new("X", "api-key", "password", "v")
    };
    let cases = [
        ("w1", store(&desk, "w1", &other, &x), PHONE_KEY_CHANGED),
        ("w2", renonced, "another request"),
        (
            "w3",
            store(&desk, "w3", &phone, &Sent::new("Twin", "api-key", "password", "v").replace()),
            "Several vault items are named Twin",
        ),
        (
            "w4",
            store(&desk, "w4", &phone, &Sent::new("Stripe", "api-key", "password", "v").replace()),
            "which has no password",
        ),
        (
            "w5",
            store(&desk, "w5", &phone, &Sent::new("K", "ssh", "private_key", "not a key")),
            "not an OpenSSH private key",
        ),
        ("w6", store(&desk, "w6", &phone, &Sent::new("N", "note", "password", "v")), "has no password"),
        ("w9", rekinded, "another item, field or kind"),
        ("w10", store(&desk, "w10", &phone, &old), "too long ago"),
    ];
    // The server knows both public keys: what it boxes itself, or a box moved to another item, does not open.
    let server_key = SecretKey::generate(&mut OsRng);
    let forged = box_from(&server_key, &phone, "nonce-w7", &Sent::new("X", "api-key", "password", "attacker-key"));
    let moved = box_from(&desk.key, &phone, "nonce-w8", &Sent::new("Other", "api-key", "password", "v"));
    let cases = [
        cases.to_vec(),
        vec![
            ("w7", store_boxed(&desk, "w7", "nonce-w7", &x, &forged), PHONE_KEY_CHANGED),
            ("w8", store_boxed(&desk, "w8", "nonce-w8", &x, &moved), "another item, field or kind"),
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
async fn the_computer_lists_names_only_when_asked_each_time_boxed_by_the_phone() {
    let desk = desk().await;
    desk.pair().await;
    let phone = phone_key(&desk).await;
    desk.send(&[call("n1", DESK, "vault", "names", &json!({"client_key": desk.public(), "nonce": "nn"}))]).await;
    let view = desk.core.approval_view("n1".to_owned()).await.unwrap();
    assert!(view.no_standing, "asked for every time");
    desk.core.approve("n1".to_owned(), choice(&[], None)).await.unwrap();
    let data = desk.data("n1").await;
    let names: VaultNames = open_from_phone(&desk, &phone, &data["sealed"]);
    assert_eq!(names.nonce, "nn");
    let listed: Vec<(&str, &str)> = names.items.iter().map(|(n, k)| (n.as_str(), k.as_str())).collect();
    assert_eq!(
        listed,
        [
            ("Bank", "login"),
            ("Deploy key", "ssh_key"),
            ("OpenAI", "login"),
            ("Stripe", "note"),
            ("Twin", "login"),
            ("Twin", "login"),
            ("Visa", "card")
        ]
    );
    assert!(!data.to_string().contains("OpenAI"), "the names travel boxed");
}
