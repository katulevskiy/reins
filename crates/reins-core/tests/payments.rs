//! Payments against a mock Vaultwarden (a card and an identity with an address) and a fake Privacy.com API: masked
//! lists, the purchase screen, approvals with each kind of payment method, budgets that refuse, spend limits that
//! approve, the ledger, and that card numbers never reach the activity log or the ledger.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use common::{FakeGoogle, FakeKeys, RecordingNotifier};
use crypto_box::SecretKey;
use crypto_box::aead::OsRng;
use reins_core::connector::payments::mandate;
use reins_core::connector::payments::{BudgetView, LimitPeriod, PurchaseChoice, SpendLimitInput};
use reins_core::crypto::{Kdf, VaultKey, master_key};
use reins_core::{ApprovalChoice, CoreConfig, GoogleTokenProvider, Notifier, ReinsCore, StandingGrant};
use serde_json::{Value, json};
use wiremock::matchers::{body_partial_json, header, method, path, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

const PASSWORD: &str = "correct horse battery";
const EMAIL: &str = "me@example.com";
const VAULT_CARD: &str = "4111111111111111";
const VAULT_CODE: &str = "737";
const VIRTUAL_PAN: &str = "4000123412341234";
const VIRTUAL_CVV: &str = "321";
const PRIVACY_KEY: &str = "test-privacy-key";

struct Env {
    server: MockServer,
    privacy: MockServer,
    core: Arc<ReinsCore>,
    counter: AtomicU32,
    notes: Arc<RecordingNotifier>,
    _dir: tempfile::TempDir,
}

fn t(key: &VaultKey, s: &str) -> Value {
    json!(key.encrypt(s.as_bytes()).unwrap())
}

fn cipher(key: &VaultKey, id: &str, kind: i64, name: &str) -> Value {
    json!({"id": id, "type": kind, "name": t(key, name), "notes": null, "key": null, "folderId": null,
        "favorite": false, "deletedDate": null, "archivedDate": null, "organizationId": null,
        "creationDate": "2026-01-01T00:00:00.000000Z", "revisionDate": "2026-02-02T10:00:00.000000Z",
        "reprompt": 0, "fields": [], "passwordHistory": [], "attachments": null,
        "login": null, "secureNote": null, "card": null, "identity": null, "sshKey": null})
}

async fn env() -> Env {
    let server = MockServer::start().await;
    let privacy = MockServer::start().await;
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
    let user = VaultKey::from_bytes(&[7u8; 64]).unwrap();
    let wrapped = master_key(
        PASSWORD,
        EMAIL,
        Kdf::Pbkdf2 {
            iterations: 5000,
        },
    )
    .unwrap()
    .stretch()
    .encrypt(&user.to_bytes())
    .unwrap();
    let mut card = cipher(&user, "card1", 3, "Everyday Visa");
    card["card"] = json!({"cardholderName": t(&user, "Anna Lee"), "brand": t(&user, "Visa"),
        "number": t(&user, VAULT_CARD), "expMonth": t(&user, "12"), "expYear": t(&user, "2040"),
        "code": t(&user, VAULT_CODE)});
    let mut home = cipher(&user, "home", 4, "Home");
    home["identity"] = json!({"firstName": t(&user, "Anna"), "lastName": t(&user, "Lee"),
        "address1": t(&user, "12 Harbour Street"), "city": t(&user, "Oslo"), "postalCode": t(&user, "0150"),
        "country": t(&user, "NO"), "phone": t(&user, "555 0100")});
    let passport = cipher(&user, "passport", 4, "Passport only");
    Mock::given(method("GET"))
        .and(path("/api/sync"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"profile": {"id": "account-1", "key": wrapped}, "ciphers": [card, home, passport], "folders": []}),
        ))
        .mount(&server)
        .await;

    mount_privacy(&privacy).await;

    let dir = tempfile::tempdir().unwrap();
    let google: Arc<dyn GoogleTokenProvider> = Arc::new(FakeGoogle::new());
    let notes = Arc::new(RecordingNotifier::default());
    let notifier: Arc<dyn Notifier> = Arc::<RecordingNotifier>::clone(&notes);
    let cfg = CoreConfig {
        privacy_base: privacy.uri(),
        privacy_sandbox_base: format!("{}/sandbox", privacy.uri()),
        ..CoreConfig::default()
    };
    let core =
        ReinsCore::with_connectors(dir.path().to_str().unwrap(), &FakeKeys, google, notifier, cfg, vec![]).unwrap();
    core.login(server.uri(), EMAIL.to_owned(), PASSWORD.to_owned(), None).await.unwrap();
    core.add_token_account("vault".into(), PASSWORD.into()).await.unwrap();
    core.add_service_account("payments".into(), String::new()).await.unwrap();
    Mock::given(method("POST"))
        .and(path_regex(r"^/reins/api/(requests|pairings)/[^/]+/response$"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    Env {
        server,
        privacy,
        core,
        counter: AtomicU32::new(0),
        notes,
        _dir: dir,
    }
}

/// Privacy.com: the key check, new cards, their charges, and closing them.
async fn mount_privacy(privacy: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/cards"))
        .and(header("authorization", format!("api-key {PRIVACY_KEY}").as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [], "page": 1})))
        .mount(privacy)
        .await;
    Mock::given(method("GET"))
        .and(path("/cards"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({"message": "Invalid API key"})))
        .with_priority(9)
        .mount(privacy)
        .await;
    Mock::given(method("POST"))
        .and(path("/cards"))
        .and(header("authorization", format!("api-key {PRIVACY_KEY}").as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "token": "7ef7d65c-9023-4da3-b113-3b8583fd7951", "pan": VIRTUAL_PAN, "cvv": VIRTUAL_CVV,
            "exp_month": "10", "exp_year": "2031", "last_four": "1234", "state": "OPEN", "type": "MERCHANT_LOCKED"})))
        .mount(privacy)
        .await;
    Mock::given(method("GET"))
        .and(path("/transactions"))
        .respond_with(charges("AMZN Mktp US*2K3AB1CD2"))
        .with_priority(9)
        .mount(privacy)
        .await;
    Mock::given(method("PATCH"))
        .and(path_regex(r"^/cards/[0-9a-f-]+$"))
        .and(body_partial_json(json!({"state": "CLOSED"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"state": "CLOSED"})))
        .mount(privacy)
        .await;
}

/// Privacy.com's list of approved charges, all by `descriptor`.
fn charges(descriptor: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({"data": [{"token": "t1", "result": "APPROVED", "status": "SETTLED",
        "amount": 2497, "merchant": {"descriptor": descriptor, "city": "SEATTLE", "country": "USA", "mcc": "5942"}}]}))
}

fn now() -> i64 {
    i64::try_from(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()).unwrap()
}

fn request(id: &str, connection: &str, op: &str, args: &Value) -> Value {
    request_at(id, connection, op, args, now())
}

fn request_at(id: &str, connection: &str, op: &str, args: &Value, created_at: i64) -> Value {
    json!({"v": 1, "id": id, "connection_id": connection, "connection_label": "Claude", "created_at": created_at,
           "call": {"tool": "connector", "service": "payments", "op": op, "args": args}})
}

/// Sends a request from connection `c1` (or `connection`). Returns its id and whether it waits for the user.
async fn ask_from(env: &Env, connection: &str, op: &str, args: &Value) -> (String, bool) {
    let id = format!("p{}", env.counter.fetch_add(1, Ordering::SeqCst) + 1);
    Mock::given(method("GET"))
        .and(path("/reins/api/pending"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"requests": [request(&id, connection, op, args)], "pairings": []})),
        )
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&env.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/reins/api/pending"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"requests": [], "pairings": []})))
        .mount(&env.server)
        .await;
    let parked = env.core.sync(0).await.unwrap().iter().any(|p| p.id == id);
    (id, parked)
}

async fn ask(env: &Env, op: &str, args: &Value) -> (String, bool) {
    ask_from(env, "c1", op, args).await
}

/// The server relays `request` (once). Whether it waits for the user.
async fn deliver(env: &Env, request: Value) -> bool {
    let id = request["id"].as_str().unwrap().to_owned();
    Mock::given(method("GET"))
        .and(path("/reins/api/pending"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"requests": [request], "pairings": []})))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&env.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/reins/api/pending"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"requests": [], "pairings": []})))
        .mount(&env.server)
        .await;
    env.core.sync(0).await.unwrap().iter().any(|p| p.id == id)
}

/// A purchase request from `connection`, under the name `label`.
fn named_request(id: &str, connection: &str, label: &str, args: &Value) -> Value {
    let mut r = request(id, connection, "purchase_request", args);
    r["connection_label"] = json!(label);
    r
}

/// A Reins desktop app named `label` is paired with this phone on `connection`, with a key of its own.
async fn pair_desktop(env: &Env, connection: &str, label: &str) -> SecretKey {
    let key = SecretKey::generate(&mut OsRng);
    let id = format!("pair-{}", env.counter.fetch_add(1, Ordering::SeqCst));
    let pairing = json!({"v": 1, "id": id, "client_name": label, "client_host": "127.0.0.1", "choices": [12, 47, 83],
        "created_at": now(), "client_key": reins_proto::desktop::encode_key(key.public_key().as_bytes())});
    Mock::given(method("GET"))
        .and(path("/reins/api/pending"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"requests": [], "pairings": [pairing]})))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&env.server)
        .await;
    env.core.sync(0).await.unwrap();
    Mock::given(method("POST"))
        .and(path(format!("/reins/api/pairings/{id}/response")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"connection_id": connection})))
        .with_priority(1)
        .mount(&env.server)
        .await;
    env.core.answer_pairing(id, true, Some(47), None).await.unwrap();
    key
}

/// A cart asked for by the desktop app with `key`, sealed with `nonce`.
fn sealed_cart(key: &SecretKey, nonce: &str, changes: &Value) -> Value {
    let mut c = cart_with(changes);
    c["client_key"] = json!(reins_proto::desktop::encode_key(key.public_key().as_bytes()));
    c["nonce"] = json!(nonce);
    c
}

fn unseal(key: &SecretKey, sealed: &Value) -> Value {
    let bytes = data_encoding::BASE64URL_NOPAD.decode(sealed.as_str().unwrap().as_bytes()).unwrap();
    serde_json::from_slice(&key.unseal(&bytes).unwrap()).unwrap()
}

/// The phone's answer to request `id`.
async fn answer(env: &Env, id: &str) -> Value {
    let found = env
        .server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .rev()
        .find(|r| r.method.as_str() == "POST" && r.url.path() == format!("/reins/api/requests/{id}/response"));
    serde_json::from_slice(&found.unwrap_or_else(|| panic!("{id} was not answered")).body).unwrap()
}

fn cart() -> Value {
    json!({
        "merchant": "Amazon",
        "merchant_url": "https://www.amazon.com/dp/B0EXAMPLE",
        "items": [{"name": "USB-C cable", "quantity": 2, "unit_price": "9.99"}],
        "shipping": "4.99",
        "currency": "USD",
        "total": "24.97",
        "note": "The user asked for two spare cables."
    })
}

fn cart_with(changes: &Value) -> Value {
    let mut c = cart();
    for (k, v) in changes.as_object().unwrap() {
        c[k] = v.clone();
    }
    c
}

fn choose(method: &str) -> PurchaseChoice {
    PurchaseChoice {
        method_id: Some(method.to_owned()),
        address_id: None,
        limit: None,
    }
}

async fn privacy_requests(env: &Env, verb: &str) -> Vec<Value> {
    env.privacy
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.method.as_str() == verb)
        .map(|r| serde_json::from_slice(&r.body).unwrap_or(Value::Null))
        .collect()
}

/// Nothing the phone keeps (activity, ledger, settings) holds a card number or a security code.
async fn nothing_kept_holds_a_card_number(env: &Env) {
    let activity = format!("{:?}", env.core.activity(100).await.unwrap());
    let spending = format!("{:?}", env.core.payments_spending(0).await.unwrap());
    let overview = format!("{:?}", env.core.payments_overview().await.unwrap());
    for kept in [activity, spending, overview] {
        for secret in [VAULT_CARD, VIRTUAL_PAN, PRIVACY_KEY] {
            assert!(!kept.contains(secret), "{secret} kept: {kept}");
        }
    }
}

#[tokio::test]
async fn lists_show_masked_methods_and_addresses_only() {
    let env = env().await;
    env.core.payments_set_method("card:card1".into(), true).await.unwrap();
    let (id, parked) = ask(&env, "methods_list", &json!({})).await;
    assert!(parked, "a list waits for the user like any other");
    let view = env.core.approval_view(id.clone()).await.unwrap();
    let picked = view.messages.iter().map(|m| m.id.clone()).collect();
    env.core
        .approve(
            id.clone(),
            ApprovalChoice {
                selected_message_ids: picked,
                standing: None,
            },
        )
        .await
        .unwrap();
    let shown = answer(&env, &id).await["result"]["data"].to_string();
    assert!(shown.contains("card:card1") && shown.contains("\"last4\":\"1111\"") && shown.contains("12/40"));
    assert!(shown.contains("merchant_account") && shown.contains("pay_on_phone"), "{shown}");
    assert!(!shown.contains(VAULT_CARD) && !shown.contains(VAULT_CODE), "{shown}");

    let (id, _) = ask(&env, "addresses_list", &json!({})).await;
    let view = env.core.approval_view(id.clone()).await.unwrap();
    assert_eq!(view.messages.len(), 1, "an identity without an address is not an address: {view:?}");
    env.core
        .approve(
            id.clone(),
            ApprovalChoice {
                selected_message_ids: vec!["home".into()],
                standing: None,
            },
        )
        .await
        .unwrap();
    let shown = answer(&env, &id).await["result"]["data"].to_string();
    assert!(shown.contains("Oslo") && shown.contains("Home"), "{shown}");
    assert!(!shown.contains("Harbour") && !shown.contains("0150") && !shown.contains("555 0100"), "{shown}");
}

#[tokio::test]
async fn a_purchase_shows_like_a_receipt_and_pays_with_the_vault_card_once_approved() {
    let env = env().await;
    env.core.payments_set_method("card:card1".into(), true).await.unwrap();
    let (id, parked) = ask(&env, "purchase_request", &cart()).await;
    assert!(parked);
    let pending = env.core.pending().await.unwrap();
    assert_eq!(pending[0].title, "Claude wants to buy 2 items at amazon.com for $24.97");
    let view = env.core.approval_view(id.clone()).await.unwrap();
    assert!(view.no_standing, "never remembered");
    assert_eq!(
        reins_core::autopilot::gates::request_floor(&view),
        Some("purchases are only ever approved by you, or within a spend limit you set"),
        "Autopilot never approves a purchase, in any mode"
    );
    let p = view.purchase.expect("the purchase screen");
    assert_eq!((p.merchant.as_str(), p.domain.as_str(), p.total.as_str()), ("Amazon", "amazon.com", "$24.97"));
    assert_eq!((p.items[0].quantity, p.items[0].line_total.as_str()), (2, "$19.98"));
    assert_eq!(p.shipping.as_deref(), Some("$4.99"));
    assert_eq!(p.address_id.as_deref(), Some("home"), "the only address is picked");
    assert_eq!(p.addresses[0].lines, ["Anna Lee", "12 Harbour Street", "0150 Oslo", "NO"]);
    assert!(p.methods.iter().any(|m| m.id == "card:card1" && m.last4.as_deref() == Some("1111")));
    assert!(p.warnings.iter().any(|w| w.contains("first purchase at amazon.com")), "{:?}", p.warnings);
    assert!(p.limit_methods.is_empty(), "only a virtual card goes in a spend limit: {:?}", p.limit_methods);

    // A standing permission is refused; the purchase screen's approval pays.
    let standing = env
        .core
        .approve(
            id.clone(),
            ApprovalChoice {
                selected_message_ids: vec![],
                standing: Some(StandingGrant {
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
                        resources: vec!["amazon.com".into()],
                        classes: vec![],
                    },
                }),
            },
        )
        .await;
    assert!(standing.unwrap_err().to_string().contains("every time"));
    // A plain approval (an older app, a generic button) never pays with defaults: the purchase screen does.
    let plain = env.core.approve(
        id.clone(),
        ApprovalChoice {
            selected_message_ids: vec![],
            standing: None,
        },
    );
    assert!(plain.await.unwrap_err().to_string().contains("Open the purchase"));
    env.core.approve_purchase(id.clone(), choose("card:card1")).await.unwrap();
    let data = answer(&env, &id).await["result"]["data"].clone();
    assert_eq!(data["status"], "approved");
    assert_eq!(data["payment"]["kind"], "card");
    assert_eq!(
        (data["payment"]["number"].as_str(), data["payment"]["code"].as_str()),
        (Some(VAULT_CARD), Some(VAULT_CODE))
    );
    assert_eq!(data["ship_to"]["line1"], "12 Harbour Street");
    let pid = data["purchase_id"].as_str().unwrap().to_owned();
    assert_ne!(pid, id, "the phone makes its own purchase id");
    let (mandate, kid) = mandate::verify(data["mandate"].as_str().unwrap()).unwrap();
    assert_eq!(mandate.purchase_id, pid);
    // The mandate binds the whole address with a salted hash.
    let bound = mandate.ship_to.clone().unwrap();
    let text = format!("{}\n{}", bound.salt, ["Anna Lee", "12 Harbour Street", "0150 Oslo", "NO"].join("\n"));
    let digest = ring::digest::digest(&ring::digest::SHA256, text.as_bytes());
    assert_eq!(bound.address_sha256, data_encoding::BASE64URL_NOPAD.encode(digest.as_ref()));
    assert_eq!((bound.label.as_str(), bound.country.as_str()), ("Home", "NO"));
    assert_eq!((mandate.amounts["total"].as_str(), mandate.payment.last4.as_deref()), (Some("24.97"), Some("1111")));
    assert_eq!((mandate.agent.as_str(), mandate.approved_by.as_str()), ("Claude", "you"));
    assert_eq!(kid, env.core.payments_overview().await.unwrap().mandate_key);

    // The AI reports the order; the ledger has it, the activity too, and neither has the number.
    let elsewhere = json!({"purchase_id": pid, "status": "completed", "receipt_url": "https://phish.example/r"});
    let (rid, _) = ask(&env, "purchase_complete", &elsewhere).await;
    assert!(answer(&env, &rid).await["message"].as_str().unwrap().contains("must be a page of amazon.com"));
    let report = json!({"purchase_id": pid, "status": "completed", "order_id": "111-222", "charged_total": "24.97",
        "currency": "USD", "receipt_url": "https://www.amazon.com/gp/your-account/order-details?orderID=111-222"});
    let (rid, parked) = ask(&env, "purchase_complete", &report).await;
    assert!(!parked, "a report never asks");
    assert_eq!(answer(&env, &rid).await["result"]["data"]["recorded"], true);
    let spending = env.core.payments_spending(0).await.unwrap();
    assert_eq!(spending.totals[0].text, "$24.97");
    let record = &spending.purchases[0];
    assert_eq!((record.status.as_str(), record.order_id.as_deref()), ("completed", Some("111-222")));
    assert_eq!(record.method_label, "Visa \u{2022}\u{2022} 1111");
    nothing_kept_holds_a_card_number(&env).await;

    // Only the AI that bought may report on it.
    let (other, _) = ask_from(&env, "c2", "purchase_complete", &report).await;
    assert_eq!(answer(&env, &other).await["outcome"], "error");
}

#[tokio::test]
async fn a_virtual_card_is_made_for_the_cart_capped_and_closed_when_the_purchase_fails() {
    let env = env().await;
    assert!(env.core.payments_connect_provider("privacy".into(), "wrong".into(), false, false).await.is_err());
    env.core.payments_connect_provider("privacy".into(), PRIVACY_KEY.into(), false, false).await.unwrap();
    let (id, parked) = ask(&env, "purchase_request", &cart_with(&json!({"payment_method": "virtual_card"}))).await;
    assert!(parked);
    let view = env.core.approval_view(id.clone()).await.unwrap().purchase.unwrap();
    assert_eq!(view.method_id.as_deref(), Some("virtual_card"));
    assert!(view.methods[0].detail.contains("$27.47"), "10% tolerance, at least $1: {}", view.methods[0].detail);
    env.core.approve_purchase(id.clone(), choose("virtual_card")).await.unwrap();
    let made = privacy_requests(&env, "POST").await;
    assert_eq!(made.len(), 1);
    assert_eq!(made[0]["type"], "MERCHANT_LOCKED");
    assert_eq!(made[0]["spend_limit"], 2_747, "the total plus the tolerance, in cents");
    assert_eq!(made[0]["spend_limit_duration"], "FOREVER");
    let data = answer(&env, &id).await["result"]["data"].clone();
    assert_eq!(
        (data["payment"]["kind"].as_str(), data["payment"]["number"].as_str()),
        (Some("virtual_card"), Some(VIRTUAL_PAN))
    );
    assert_eq!(
        (data["payment"]["code"].as_str(), data["payment"]["limit"].as_str()),
        (Some(VIRTUAL_CVV), Some("27.47"))
    );
    assert_eq!(data["payment"]["holder"], "Anna Lee");
    let open = env.core.payments_spending(0).await.unwrap();
    assert!(open.purchases[0].card_open);
    assert_eq!(open.purchases[0].card_last4.as_deref(), Some("1234"));

    // Privacy.com says the card was never charged.
    Mock::given(method("GET"))
        .and(path("/transactions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": []})))
        .with_priority(1)
        .mount(&env.privacy)
        .await;
    let pid = data["purchase_id"].clone();
    let (rid, _) = ask(&env, "purchase_complete", &json!({"purchase_id": pid, "status": "failed"})).await;
    assert_eq!(answer(&env, &rid).await["result"]["data"]["virtual_card"], "closed");
    assert_eq!(privacy_requests(&env, "PATCH").await, [json!({"state": "CLOSED"})]);
    let spending = env.core.payments_spending(0).await.unwrap();
    assert!(!spending.purchases[0].card_open);
    assert_eq!(spending.totals[0].minor, 0, "a failed purchase whose card was never charged spends nothing");
    nothing_kept_holds_a_card_number(&env).await;

    // A virtual card cannot pay in another currency: the AI is told at once.
    let (eur, parked) =
        ask(&env, "purchase_request", &cart_with(&json!({"payment_method": "virtual_card", "currency": "EUR"}))).await;
    assert!(!parked);
    assert!(answer(&env, &eur).await["message"].as_str().unwrap().contains("US dollars"));
}

#[tokio::test]
async fn budgets_refuse_before_the_user_is_asked() {
    let env = env().await;
    let budget = |per_purchase: Option<i64>, merchants: Vec<String>| BudgetView {
        connection_id: String::new(),
        connection_label: String::new(),
        currency: "USD".into(),
        per_purchase,
        per_day: None,
        per_month: None,
        merchants,
        lines: vec![],
    };
    env.core.payments_set_budget(budget(Some(2_000), vec![])).await.unwrap();
    let (id, parked) = ask(&env, "purchase_request", &cart()).await;
    assert!(!parked);
    let refused = answer(&env, &id).await;
    assert_eq!(refused["outcome"], "denied");
    assert!(refused["reason"].as_str().unwrap().contains("$20.00 per purchase"), "{refused}");

    env.core.payments_set_budget(budget(None, vec!["https://www.ebay.com".into()])).await.unwrap();
    let (id, parked) = ask(&env, "purchase_request", &cart()).await;
    assert!(!parked);
    assert!(answer(&env, &id).await["reason"].as_str().unwrap().contains("only allows purchases at ebay.com"));

    env.core.payments_set_budget(budget(None, vec![])).await.unwrap();
    assert!(env.core.payments_overview().await.unwrap().budgets.is_empty(), "an empty budget is removed");
    assert!(ask(&env, "purchase_request", &cart()).await.1);
}

#[tokio::test]
async fn a_spend_limit_approves_what_it_covers_and_asks_about_the_rest() {
    let env = env().await;
    env.core.payments_connect_provider("privacy".into(), PRIVACY_KEY.into(), false, false).await.unwrap();
    let named = json!({"payment_method": "virtual_card", "ship_to": "home"});
    let (first, _) = ask(&env, "purchase_request", &cart_with(&named)).await;
    // Approved by hand, with a limit for more like it: up to $30 a purchase and $50 a day at amazon.com.
    let limit = SpendLimitInput {
        connection_id: String::new(),
        merchants: vec!["amazon.com".into()],
        method: "virtual_card".into(),
        currency: "USD".into(),
        per_purchase: 3_000,
        per_period: 6_000,
        period: LimitPeriod::Day,
        duration_secs: 7 * 86_400,
    };
    let mut with_card_limit = choose("virtual_card");
    with_card_limit.limit = Some(SpendLimitInput {
        method: "card:card1".into(),
        ..limit.clone()
    });
    assert!(env.core.approve_purchase(first.clone(), with_card_limit).await.is_err(), "never a vault card");
    let mut with_store_limit = choose("virtual_card");
    with_store_limit.limit = Some(SpendLimitInput {
        method: "merchant_account".into(),
        ..limit.clone()
    });
    assert!(
        env.core.approve_purchase(first.clone(), with_store_limit).await.is_err(),
        "nothing caps the store's saved card"
    );
    let mut with_limit = choose("virtual_card");
    with_limit.limit = Some(limit);
    env.core.approve_purchase(first, with_limit).await.unwrap();
    let limits = env.core.payments_overview().await.unwrap().limits;
    assert_eq!(limits.len(), 1);
    assert_eq!(limits[0].summary, "Up to $30.00 a purchase and $60.00 a day at amazon.com");
    assert_eq!(limits[0].spent, 2_747, "an open card counts its cap: the total and the tolerance");

    // The second one fits ($24.97 + $24.97 <= $50): approved without asking.
    let (second, parked) = ask(&env, "purchase_request", &cart_with(&named)).await;
    assert!(!parked);
    let data = answer(&env, &second).await["result"]["data"].clone();
    assert_eq!(data["approved_by"], "spend limit");
    assert_eq!(mandate::verify(data["mandate"].as_str().unwrap()).unwrap().0.approved_by, "spend limit");
    // The third would go over the day: it asks. So does one that leaves the address to the user, and one elsewhere.
    assert!(ask(&env, "purchase_request", &cart_with(&named)).await.1);
    let small = json!({"items": [{"name": "Sticker", "unit_price": "1.00"}], "shipping": "0", "total": "1.00"});
    let mut small_unnamed = cart_with(&small);
    small_unnamed["payment_method"] = json!("virtual_card");
    assert!(ask(&env, "purchase_request", &small_unnamed).await.1, "the address is left to the user");
    let mut elsewhere = cart_with(&small);
    elsewhere["merchant"] = json!("eBay");
    elsewhere["merchant_url"] = json!("https://www.ebay.com/itm/1");
    elsewhere["payment_method"] = json!("virtual_card");
    elsewhere["ship_to"] = json!("home");
    assert!(ask(&env, "purchase_request", &elsewhere).await.1);
    // Another AI is not covered by Claude's limit.
    let mut tiny = cart_with(&small);
    tiny["payment_method"] = json!("virtual_card");
    tiny["ship_to"] = json!("home");
    assert!(ask_from(&env, "c2", "purchase_request", &tiny).await.1);
    // Lockdown stops limits too.
    env.core.set_autopilot_mode(None, Some(reins_core::AutopilotMode::Lockdown), None).await.unwrap();
    assert!(!ask(&env, "purchase_request", &tiny).await.1, "denied by Lockdown with everything else");

    let id = env.core.payments_overview().await.unwrap().limits[0].id.clone();
    env.core.payments_remove_limit(id).await.unwrap();
    assert!(env.core.payments_overview().await.unwrap().limits.is_empty());
    nothing_kept_holds_a_card_number(&env).await;
}

#[tokio::test]
async fn paying_on_the_phone_and_at_the_store_hand_over_nothing_to_pay_with() {
    let env = env().await;
    let (id, _) = ask(
        &env,
        "purchase_request",
        &cart_with(&json!({"checkout_url": "https://www.amazon.com/checkout/p/1", "ship_to": "none"})),
    )
    .await;
    let view = env.core.approval_view(id.clone()).await.unwrap().purchase.unwrap();
    assert!(!view.ships && view.address_id.is_none());
    assert_eq!(view.checkout_url, "https://www.amazon.com/checkout/p/1");
    env.core.approve_purchase(id.clone(), choose("pay_on_phone")).await.unwrap();
    let data = answer(&env, &id).await["result"]["data"].clone();
    assert_eq!(
        (data["payment"]["kind"].as_str(), data["payment"]["status"].as_str()),
        (Some("pay_on_phone"), Some("handed_off"))
    );
    assert!(data["payment"].get("number").is_none() && data["ship_to"].is_null());

    let (id, _) = ask(&env, "purchase_request", &cart()).await;
    env.core.approve_purchase(id.clone(), choose("merchant_account")).await.unwrap();
    let data = answer(&env, &id).await["result"]["data"].clone();
    assert_eq!(data["payment"]["kind"], "merchant_account");
    assert!(data["payment"]["instructions"].as_str().unwrap().contains("amazon.com"));

    // Switched off while it waited: refused, and the request still waits.
    let (id, _) = ask(&env, "purchase_request", &cart()).await;
    env.core.payments_set_method("pay_on_phone".into(), false).await.unwrap();
    assert!(env.core.approve_purchase(id.clone(), choose("pay_on_phone")).await.is_err());
    assert!(env.core.pending().await.unwrap().iter().any(|p| p.id == id));
}

#[tokio::test]
async fn bad_carts_and_unknown_choices_are_refused_with_a_reason() {
    let env = env().await;
    // The server refuses these with the reason before relaying (reins-proto's tests); the phone checks again.
    for args in [cart_with(&json!({"total": "20.00"})), cart_with(&json!({"merchant_url": "http://www.amazon.com/"}))] {
        let (id, parked) = ask(&env, "purchase_request", &args).await;
        assert!(!parked);
        assert_eq!(answer(&env, &id).await["outcome"], "error");
    }
    for (args, says) in [
        (cart_with(&json!({"ship_to": "nowhere"})), "not one of the user's addresses"),
        (cart_with(&json!({"payment_method": "card:card1"})), "not one the user allows"),
        (cart_with(&json!({"client_key": "AAAA", "nonce": "n"})), "desktop app"),
    ] {
        let (id, parked) = ask(&env, "purchase_request", &args).await;
        assert!(!parked);
        let refused = answer(&env, &id).await;
        let message = refused["message"].as_str().unwrap_or_default();
        assert!(message.contains(says), "{says}: {refused}");
    }
}

#[tokio::test]
async fn a_card_charged_by_another_store_pauses_limits_until_the_user_has_seen_it() {
    let env = env().await;
    env.core.payments_connect_provider("privacy".into(), PRIVACY_KEY.into(), false, false).await.unwrap();
    let named = cart_with(&json!({"payment_method": "virtual_card", "ship_to": "home"}));
    let (first, _) = ask(&env, "purchase_request", &named).await;
    let mut with_limit = choose("virtual_card");
    with_limit.limit = Some(SpendLimitInput {
        connection_id: String::new(),
        merchants: vec!["amazon.com".into()],
        method: "virtual_card".into(),
        currency: "USD".into(),
        per_purchase: 5_000,
        per_period: 20_000,
        period: LimitPeriod::Week,
        duration_secs: 86_400,
    });
    env.core.approve_purchase(first.clone(), with_limit).await.unwrap();
    let first = answer(&env, &first).await["result"]["data"]["purchase_id"].as_str().unwrap().to_owned();

    // The card made for amazon.com is charged by someone else: the next purchase asks, and says why.
    Mock::given(method("GET"))
        .and(path("/transactions"))
        .respond_with(charges("CHEAP WATCHES LTD"))
        .with_priority(5)
        .mount(&env.privacy)
        .await;
    let (second, parked) = ask(&env, "purchase_request", &named).await;
    assert!(parked, "no spend limit while a foreign charge is unseen");
    let warnings = env.core.approval_view(second.clone()).await.unwrap().purchase.unwrap().warnings;
    assert!(warnings.iter().any(|w| w.contains("charged by CHEAP WATCHES LTD")), "{warnings:?}");
    assert_eq!(privacy_requests(&env, "PATCH").await.len(), 1, "the card is closed at once");
    let spending = env.core.payments_spending(0).await.unwrap();
    let flagged = spending.purchases.iter().find(|p| p.id == first).unwrap();
    assert_eq!(flagged.mismatch.as_deref(), Some("CHEAP WATCHES LTD"));
    env.core.deny(second).await.unwrap();

    // Seen: the limit approves again (later charges are by Amazon).
    env.core.payments_acknowledge_charge(first.clone()).await.unwrap();
    Mock::given(method("GET"))
        .and(path("/transactions"))
        .respond_with(charges("AMZN Mktp US*9ZZ"))
        .with_priority(1)
        .mount(&env.privacy)
        .await;
    let (third, parked) = ask(&env, "purchase_request", &named).await;
    assert!(!parked);
    assert_eq!(answer(&env, &third).await["result"]["data"]["approved_by"], "spend limit");
    assert_eq!(
        env.core.payments_spending(0).await.unwrap().purchases.iter().find(|p| p.id == first).unwrap().mismatch,
        None
    );
}

#[tokio::test]
async fn card_details_for_the_desktop_app_are_sealed_to_its_pinned_key() {
    let server = MockServer::start().await;
    common::desktop::mount_reins(&server).await;
    let privacy = MockServer::start().await;
    mount_privacy(&privacy).await;
    let cfg = CoreConfig {
        privacy_base: privacy.uri(),
        ..CoreConfig::default()
    };
    let desk = common::desktop::desk_with(server, EMAIL, PASSWORD, cfg).await;
    desk.pair().await;
    desk.core.add_service_account("payments".into(), String::new()).await.unwrap();
    desk.core.payments_connect_provider("privacy".into(), PRIVACY_KEY.into(), false, false).await.unwrap();
    let digital = cart_with(&json!({"ship_to": "none", "payment_method": "virtual_card", "client_key": desk.public(),
        "nonce": "n-1"}));
    desk.send(&[request("s1", common::desktop::DESK, "purchase_request", &digital)]).await;
    desk.core.approve_purchase("s1".into(), choose("virtual_card")).await.unwrap();
    let data = desk.data("s1").await;
    assert_eq!(data["payment"]["kind"], "virtual_card");
    assert!(data["payment"].get("number").is_none(), "the server never sees the number: {data}");
    let opened: Value = desk.open(&data["payment"]["sealed"]);
    // Signed by the phone's payment key, so that the server cannot seal a card of its own to the app.
    let (inner, kid) = mandate::verify_json(opened["jws"].as_str().unwrap(), mandate::SEALED_JWS_TYPE).unwrap();
    assert_eq!(kid, desk.core.payments_overview().await.unwrap().mandate_key);
    assert_eq!((inner["nonce"].as_str(), &inner["purchase_id"]), (Some("n-1"), &data["purchase_id"]));
    assert_eq!(inner["payment"]["number"], VIRTUAL_PAN);

    // The desktop app's connection without a key: card details are refused (a server that strips the key gets
    // nothing); paying at the store still works.
    let mut stripped = digital.clone();
    stripped.as_object_mut().unwrap().remove("client_key");
    stripped.as_object_mut().unwrap().remove("nonce");
    desk.send(&[request("s3", common::desktop::DESK, "purchase_request", &stripped)]).await;
    assert!(desk.error("s3").await.contains("without your desktop app's key"), "it asked for a virtual card");
    stripped.as_object_mut().unwrap().remove("payment_method");
    desk.send(&[request("s4", common::desktop::DESK, "purchase_request", &stripped)]).await;
    let refused = desk.core.approve_purchase("s4".into(), choose("virtual_card")).await.unwrap_err();
    assert!(refused.to_string().contains("without your desktop app's key"), "{refused}");
    desk.core.approve_purchase("s4".into(), choose("merchant_account")).await.unwrap();
    assert_eq!(desk.data("s4").await["payment"]["kind"], "merchant_account");

    // Another key on the desktop app's connection is refused.
    let mut wrong = digital.clone();
    wrong["client_key"] = json!(reins_proto::desktop::encode_key(&[9; 32]));
    desk.send(&[request("s2", common::desktop::DESK, "purchase_request", &wrong)]).await;
    assert_eq!(desk.error("s2").await, common::desktop::REFUSED);
}

#[tokio::test]
async fn a_spend_limit_approves_nothing_while_the_cards_charges_cannot_be_read() {
    let env = env().await;
    env.core.payments_connect_provider("privacy".into(), PRIVACY_KEY.into(), false, false).await.unwrap();
    let named = cart_with(&json!({"payment_method": "virtual_card", "ship_to": "home"}));
    let (first, _) = ask(&env, "purchase_request", &named).await;
    let mut with_limit = choose("virtual_card");
    with_limit.limit = Some(SpendLimitInput {
        connection_id: String::new(),
        merchants: vec![],
        method: "virtual_card".into(),
        currency: "USD".into(),
        per_purchase: 5_000,
        per_period: 50_000,
        period: LimitPeriod::Month,
        duration_secs: 86_400,
    });
    env.core.approve_purchase(first, with_limit).await.unwrap();
    Mock::given(method("GET"))
        .and(path("/transactions"))
        .respond_with(ResponseTemplate::new(503))
        .with_priority(1)
        .mount(&env.privacy)
        .await;
    assert!(ask(&env, "purchase_request", &named).await.1, "it asks when Privacy.com cannot say who charged");
}

/// A spend limit for Claude with a virtual card, made alongside a first purchase approved by hand.
async fn limit_for(env: &Env, per_purchase: i64, per_period: i64, merchants: Vec<String>) {
    let named = cart_with(&json!({"payment_method": "virtual_card", "ship_to": "home"}));
    let (first, _) = ask(env, "purchase_request", &named).await;
    let mut with_limit = choose("virtual_card");
    with_limit.limit = Some(SpendLimitInput {
        connection_id: String::new(),
        merchants,
        method: "virtual_card".into(),
        currency: "USD".into(),
        per_purchase,
        per_period,
        period: LimitPeriod::Day,
        duration_secs: 86_400,
    });
    env.core.approve_purchase(first, with_limit).await.unwrap();
}

fn tiny() -> Value {
    cart_with(&json!({"items": [{"name": "Sticker", "unit_price": "0.01"}], "shipping": "0", "total": "0.01",
        "payment_method": "virtual_card", "ship_to": "home"}))
}

#[tokio::test]
async fn tiny_carts_count_their_cards_cap_and_limits_approve_a_few_an_hour_at_most() {
    let env = env().await;
    env.core.payments_connect_provider("privacy".into(), PRIVACY_KEY.into(), false, false).await.unwrap();
    limit_for(&env, 5_000, 1_000_000, vec![]).await;
    let made_before = privacy_requests(&env, "POST").await.len();
    let mut approved = 0;
    for _ in 0..6 {
        if !ask(&env, "purchase_request", &tiny()).await.1 {
            approved += 1;
        }
    }
    assert_eq!(approved, 3, "a spend limit approves at most three purchases an hour for one AI");
    assert_eq!(privacy_requests(&env, "POST").await.len() - made_before, 3);
    let limits = env.core.payments_overview().await.unwrap().limits;
    // The first card (24.97 + 2.50) and three 0.01 carts, each on a card that can be charged 1.01.
    assert_eq!(limits[0].spent, 2_747 + 3 * 101, "every open card counts its cap, not the cart's total");
    let told = env.notes.decided.lock().unwrap().clone();
    assert_eq!(told.len(), 3, "the user is told of every purchase a limit approves");
    assert!(told.iter().all(|d| d.decided_by == "spend limit" && d.title.starts_with("Bought 1 item at amazon.com")));

    // A limit counts the card's cap against the amount a purchase: 24.97 + 2.50 is over 26.00.
    let env = self::env().await;
    env.core.payments_connect_provider("privacy".into(), PRIVACY_KEY.into(), false, false).await.unwrap();
    limit_for(&env, 2_600, 1_000_000, vec![]).await;
    assert!(
        ask(&env, "purchase_request", &cart_with(&json!({"payment_method": "virtual_card", "ship_to": "home"})))
            .await
            .1
    );
}

#[tokio::test]
async fn an_old_request_is_never_approved_by_a_limit_and_a_paid_one_never_twice() {
    let env = env().await;
    env.core.payments_connect_provider("privacy".into(), PRIVACY_KEY.into(), false, false).await.unwrap();
    limit_for(&env, 5_000, 1_000_000, vec![]).await;
    // Relayed again an hour after it was made: it asks.
    let id = "stale-1";
    Mock::given(method("GET"))
        .and(path("/reins/api/pending"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"requests": [request_at(id, "c1", "purchase_request", &tiny(), now() - 3_600)], "pairings": []}),
        ))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&env.server)
        .await;
    let made_before = privacy_requests(&env, "POST").await.len();
    env.core.sync(0).await.unwrap();
    assert_eq!(privacy_requests(&env, "POST").await.len(), made_before, "no card for an old request");
    assert!(
        env.server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|r| r.url.path() != format!("/reins/api/requests/{id}/response")
                || serde_json::from_slice::<Value>(&r.body).unwrap()["outcome"] != "result"),
        "never approved on its own"
    );
    // Approved once by hand, the same request is refused if it is ever asked again (the ledger keeps it).
    let mut by_hand = tiny();
    by_hand.as_object_mut().unwrap().remove("ship_to");
    let (fresh, parked) = ask(&env, "purchase_request", &by_hand).await;
    assert!(parked);
    env.core.approve_purchase(fresh.clone(), choose("virtual_card")).await.unwrap();
    assert!(env.core.approve_purchase(fresh, choose("virtual_card")).await.is_err());
}

#[tokio::test]
async fn a_purchase_not_paid_with_a_virtual_card_counts_until_the_user_clears_it() {
    let env = env().await;
    let (id, _) = ask(&env, "purchase_request", &cart()).await;
    env.core.approve_purchase(id.clone(), choose("merchant_account")).await.unwrap();
    let pid = answer(&env, &id).await["result"]["data"]["purchase_id"].as_str().unwrap().to_owned();
    let (rid, _) = ask(&env, "purchase_complete", &json!({"purchase_id": pid, "status": "failed"})).await;
    assert_eq!(answer(&env, &rid).await["result"]["data"]["recorded"], true);
    let spending = env.core.payments_spending(0).await.unwrap();
    assert_eq!(spending.totals[0].minor, 2_497, "the AI saying it failed does not take it off the budget");
    assert!(spending.purchases[0].clearable);
    env.core.payments_clear_purchase(pid.clone()).await.unwrap();
    let spending = env.core.payments_spending(0).await.unwrap();
    assert_eq!(spending.totals[0].minor, 0, "the user said nothing was charged");
    assert!(!spending.purchases[0].clearable);
}

#[tokio::test]
async fn many_open_cards_make_a_limit_ask_rather_than_hold_everything_up() {
    let env = env().await;
    env.core.payments_connect_provider("privacy".into(), PRIVACY_KEY.into(), false, false).await.unwrap();
    limit_for(&env, 5_000, 10_000_000, vec![]).await;
    // Eight more cards approved by hand: nine open cards, more than are read before a limit may approve.
    let mut by_hand = tiny();
    by_hand.as_object_mut().unwrap().remove("ship_to");
    for _ in 0..8 {
        let (id, parked) = ask(&env, "purchase_request", &by_hand).await;
        assert!(parked, "the address is left to the user");
        env.core.approve_purchase(id, choose("virtual_card")).await.unwrap();
    }
    let reads_before =
        env.privacy.received_requests().await.unwrap().iter().filter(|r| r.url.path() == "/transactions").count();
    assert!(ask(&env, "purchase_request", &tiny()).await.1, "with charges left unread, it asks");
    let reads =
        env.privacy.received_requests().await.unwrap().iter().filter(|r| r.url.path() == "/transactions").count();
    assert!(reads - reads_before <= 8, "{} reads", reads - reads_before);
}

#[tokio::test]
async fn disconnecting_the_provider_closes_its_cards_first() {
    let env = env().await;
    env.core.payments_connect_provider("privacy".into(), PRIVACY_KEY.into(), false, false).await.unwrap();
    limit_for(&env, 5_000, 10_000, vec![]).await;
    // Privacy.com does not answer: the cards stay open, so the key stays.
    Mock::given(method("PATCH"))
        .and(path_regex(r"^/cards/.+$"))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&env.privacy)
        .await;
    assert!(env.core.payments_disconnect_provider().await.unwrap_err().to_string().contains("could not be closed"));
    assert!(env.core.payments_overview().await.unwrap().provider.is_some());
    // Switching Payments off with a card Privacy.com will not close: the limits go, the key stays to close it later.
    Mock::given(method("PATCH"))
        .and(path_regex(r"^/cards/.+$"))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&env.privacy)
        .await;
    env.core.remove_service_account("payments".into(), "this phone".into()).await.unwrap();
    let off = env.core.payments_overview().await.unwrap();
    assert!(!off.enabled && off.limits.is_empty(), "no AI can buy any more");
    assert!(off.provider.is_some(), "the key stays while a card is open");
    env.core.add_service_account("payments".into(), String::new()).await.unwrap();
    // Next time it does: closed, then the key goes.
    env.core.payments_disconnect_provider().await.unwrap();
    let overview = env.core.payments_overview().await.unwrap();
    assert!(overview.provider.is_none() && overview.limits.is_empty());
    assert!(!env.core.payments_spending(0).await.unwrap().purchases[0].card_open);
}

/// The server's copy of the account's encrypted state, shared by the user's phones.
#[derive(Clone, Default)]
struct Cloud(Arc<std::sync::Mutex<Value>>);

impl wiremock::Respond for Cloud {
    fn respond(&self, request: &wiremock::Request) -> ResponseTemplate {
        let mut state = self.0.lock().unwrap();
        if state.is_null() {
            *state = json!({"revision": 0, "ciphertext": null});
        }
        if request.method.as_str() == "PUT" {
            let update: Value = serde_json::from_slice(&request.body).unwrap();
            if update["revision"] != state["revision"] {
                return ResponseTemplate::new(409);
            }
            *state = json!({"revision": state["revision"].as_u64().unwrap() + 1, "ciphertext": update["ciphertext"]});
        }
        ResponseTemplate::new(200).set_body_json(state.clone())
    }
}

#[tokio::test]
async fn a_request_already_paid_for_is_refused_when_it_arrives_again() {
    let env = env().await;
    let cloud = Cloud::default();
    Mock::given(path("/reins/api/account-state")).respond_with(cloud.clone()).mount(&env.server).await;
    let paid = request("again-1", "c1", "purchase_request", &cart());
    assert!(deliver(&env, paid.clone()).await);
    env.core.approve_purchase("again-1".into(), choose("merchant_account")).await.unwrap();
    assert_eq!(answer(&env, "again-1").await["outcome"], "result");
    // The ledger travels with the account's encrypted state.
    for _ in 0..100 {
        env.core.accounts().await.unwrap();
        if cloud.0.lock().unwrap()["revision"].as_u64() > Some(0) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(cloud.0.lock().unwrap()["revision"].as_u64() > Some(0), "the state was uploaded");

    // The same request relayed to the user's other phone, which never answered it: its ledger knows it was paid.
    let dir = tempfile::tempdir().unwrap();
    let other = ReinsCore::with_connectors(
        dir.path().to_str().unwrap(),
        &FakeKeys,
        Arc::new(FakeGoogle::new()),
        Arc::new(RecordingNotifier::default()),
        CoreConfig {
            privacy_base: env.privacy.uri(),
            ..CoreConfig::default()
        },
        vec![],
    )
    .unwrap();
    other.login(env.server.uri(), EMAIL.to_owned(), PASSWORD.to_owned(), None).await.unwrap();
    assert_eq!(other.payments_spending(0).await.unwrap().purchases.len(), 1, "the other phone has the ledger");
    Mock::given(method("GET"))
        .and(path("/reins/api/pending"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"requests": [paid], "pairings": []})))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&env.server)
        .await;
    assert!(other.sync(0).await.unwrap().is_empty(), "it does not wait for the user");
    let again = answer(&env, "again-1").await;
    assert_eq!(again["outcome"], "error", "{again}");
    assert!(again["message"].as_str().unwrap().contains("already paid"), "{again}");
    assert_eq!(other.payments_spending(0).await.unwrap().purchases.len(), 1, "paid once");
}

#[tokio::test]
async fn while_a_desktop_app_is_paired_a_card_from_the_vault_goes_only_to_it() {
    let env = env().await;
    env.core.payments_set_method("card:card1".into(), true).await.unwrap();
    env.core.payments_connect_provider("privacy".into(), PRIVACY_KEY.into(), false, false).await.unwrap();
    // Without a desktop app, a card goes in the answer, through the server, and the purchase screen says so.
    assert!(deliver(&env, request("plain-1", "c1", "purchase_request", &cart())).await);
    let p = env.core.approval_view("plain-1".into()).await.unwrap().purchase.unwrap();
    assert!(p.delivery.as_deref().unwrap().contains("through your Reins server"), "{:?}", p.delivery);
    assert!(p.methods.iter().any(|m| m.id == "card:card1" && m.unavailable.is_none()));

    let desk = pair_desktop(&env, "desk1", "Work laptop").await;
    // The server moves the desktop app's purchase to another connection, under the app's name, without its key.
    assert!(deliver(&env, named_request("moved-1", "c2", "Work laptop", &cart())).await);
    let p = env.core.approval_view("moved-1".into()).await.unwrap().purchase.unwrap();
    let card = p.methods.iter().find(|m| m.id == "card:card1").unwrap();
    assert!(card.unavailable.as_deref().unwrap().contains("only to your paired desktop app"), "{card:?}");
    assert!(p.delivery.as_deref().unwrap().starts_with("Not from your paired desktop app"), "{:?}", p.delivery);
    assert!(
        p.warnings.iter().any(|w| w.contains("\u{201c}Work laptop\u{201d}, the name of your desktop app")),
        "{:?}",
        p.warnings
    );
    let refused = env.core.approve_purchase("moved-1".into(), choose("card:card1")).await.unwrap_err();
    assert!(refused.to_string().contains("only to your paired desktop app"), "{refused}");
    // A virtual card, capped and locked to the store, still pays.
    env.core.approve_purchase("moved-1".into(), choose("virtual_card")).await.unwrap();
    assert_eq!(answer(&env, "moved-1").await["result"]["data"]["payment"]["number"], VIRTUAL_PAN);
    // An AI that names the vault card is told at once.
    let named = cart_with(&json!({"payment_method": "card:card1"}));
    assert!(!deliver(&env, request("moved-2", "c2", "purchase_request", &named)).await);
    assert!(answer(&env, "moved-2").await["message"].as_str().unwrap().contains("only to your paired desktop app"));
    // A request that waited from before the pairing is decided again when approved.
    let late = env.core.approve_purchase("plain-1".into(), choose("card:card1")).await.unwrap_err();
    assert!(late.to_string().contains("only to your paired desktop app"), "{late}");

    // The desktop app itself gets the vault card, sealed to its key, and the whole answer signed with its nonce.
    assert!(
        deliver(&env, named_request("desk-1", "desk1", "Work laptop", &sealed_cart(&desk, "n-7", &json!({})))).await
    );
    let p = env.core.approval_view("desk-1".into()).await.unwrap().purchase.unwrap();
    assert!(p.methods.iter().any(|m| m.id == "card:card1" && m.unavailable.is_none()));
    let delivery = p.delivery.unwrap();
    assert!(delivery.starts_with("Card details go sealed to \u{201c}Work laptop\u{201d}, your desktop app paired on"));
    env.core.approve_purchase("desk-1".into(), choose("card:card1")).await.unwrap();
    let data = answer(&env, "desk-1").await["result"]["data"].clone();
    assert!(!data.to_string().contains(VAULT_CARD), "the server never sees the number");
    let (signed, kid) = mandate::verify_json(data["signed"].as_str().unwrap(), mandate::ANSWER_JWS_TYPE).unwrap();
    assert_eq!(kid, env.core.payments_overview().await.unwrap().mandate_key);
    assert_eq!(signed["nonce"], "n-7");
    assert_eq!(signed["answer"]["purchase_id"], data["purchase_id"]);
    assert_eq!(signed["answer"]["payment"], data["payment"], "what the server relays is what was signed");
    let opened = unseal(&desk, &data["payment"]["sealed"]);
    let (inner, _) = mandate::verify_json(opened["jws"].as_str().unwrap(), mandate::SEALED_JWS_TYPE).unwrap();
    assert_eq!(inner["payment"]["number"], VAULT_CARD);
    // Paying at the store, the desktop app gets a signed answer too.
    assert!(
        deliver(&env, named_request("desk-2", "desk1", "Work laptop", &sealed_cart(&desk, "n-8", &json!({})))).await
    );
    env.core.approve_purchase("desk-2".into(), choose("merchant_account")).await.unwrap();
    let data = answer(&env, "desk-2").await["result"]["data"].clone();
    let (signed, _) = mandate::verify_json(data["signed"].as_str().unwrap(), mandate::ANSWER_JWS_TYPE).unwrap();
    assert_eq!(
        (signed["nonce"].as_str(), &signed["answer"]["payment"]["kind"]),
        (Some("n-8"), &json!("merchant_account"))
    );
    nothing_kept_holds_a_card_number(&env).await;
}

#[tokio::test]
async fn a_desktop_key_from_a_connection_without_that_app_is_refused() {
    let env = env().await;
    let desk = pair_desktop(&env, "desk1", "Work laptop").await;
    // The desktop app's key and nonce, on a connection that is not the app's (the server moved them there).
    for connection in ["c1", "c9"] {
        let id = format!("keyed-{connection}");
        assert!(
            !deliver(&env, request(&id, connection, "purchase_request", &sealed_cart(&desk, "n-1", &json!({})))).await
        );
        let refused = answer(&env, &id).await;
        assert_eq!(refused["outcome"], "error", "{refused}");
        assert!(refused["message"].as_str().unwrap().contains("desktop app paired with this phone"), "{refused}");
    }
}

#[tokio::test]
async fn a_desktop_app_unpaired_or_paired_again_while_a_purchase_waits_gets_nothing() {
    let env = env().await;
    env.core.payments_connect_provider("privacy".into(), PRIVACY_KEY.into(), false, false).await.unwrap();
    let old = pair_desktop(&env, "desk1", "Work laptop").await;
    assert!(deliver(&env, request("w-1", "desk1", "purchase_request", &sealed_cart(&old, "n-1", &json!({})))).await);
    // Paired again (another key on the same connection) before the user approved: the old key gets nothing.
    let new = pair_desktop(&env, "desk1", "Work laptop").await;
    let refused = env.core.approve_purchase("w-1".into(), choose("virtual_card")).await.unwrap_err();
    assert!(refused.to_string().contains("no longer paired"), "{refused}");
    assert!(privacy_requests(&env, "POST").await.is_empty(), "no card was made");
    // Unpaired: what it asked for goes with it.
    assert!(deliver(&env, request("w-2", "desk1", "purchase_request", &sealed_cart(&new, "n-2", &json!({})))).await);
    Mock::given(method("DELETE"))
        .and(path("/reins/api/connections/desk1"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&env.server)
        .await;
    env.core.revoke_connection("desk1".into()).await.unwrap();
    assert!(env.core.approve_purchase("w-2".into(), choose("virtual_card")).await.is_err());
    assert!(privacy_requests(&env, "POST").await.is_empty(), "no card was made");
}

#[tokio::test]
async fn lockdown_switched_on_while_the_charges_are_read_stops_a_spend_limit() {
    let env = env().await;
    env.core.payments_connect_provider("privacy".into(), PRIVACY_KEY.into(), false, false).await.unwrap();
    limit_for(&env, 5_000, 1_000_000, vec![]).await;
    let made_before = privacy_requests(&env, "POST").await.len();
    // Reading the earlier card's charges takes a while; the user switches Lockdown on meanwhile.
    Mock::given(method("GET"))
        .and(path("/transactions"))
        .respond_with(charges("AMZN Mktp US*2K3AB1CD2").set_delay(std::time::Duration::from_millis(800)))
        .with_priority(1)
        .mount(&env.privacy)
        .await;
    let core = Arc::clone(&env.core);
    let lock = tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        core.set_autopilot_mode(None, Some(reins_core::AutopilotMode::Lockdown), None).await.unwrap();
    });
    let (id, _) = ask(&env, "purchase_request", &tiny()).await;
    lock.await.unwrap();
    assert_eq!(privacy_requests(&env, "POST").await.len(), made_before, "no card was made");
    let said = answer(&env, &id).await;
    assert_ne!(said["outcome"], "result", "{said}");
}

#[tokio::test]
async fn charges_are_read_after_a_card_is_closed_and_every_page_counts() {
    let env = env().await;
    env.core.payments_connect_provider("privacy".into(), PRIVACY_KEY.into(), false, false).await.unwrap();
    let (id, _) = ask(&env, "purchase_request", &cart_with(&json!({"payment_method": "virtual_card"}))).await;
    env.core.approve_purchase(id.clone(), choose("virtual_card")).await.unwrap();
    let pid = answer(&env, &id).await["result"]["data"]["purchase_id"].clone();
    // Two pages of charges, made by the store while the AI reports that checkout failed.
    let page = |n: u32, amount: i64| {
        ResponseTemplate::new(200).set_body_json(json!({"page": n, "total_pages": 2, "total_entries": 2, "data": [
            {"token": format!("t{n}"), "result": "APPROVED", "amount": amount, "merchant": {"descriptor": "AMZN Mktp US"}}]}))
    };
    Mock::given(method("GET"))
        .and(path("/transactions"))
        .and(wiremock::matchers::query_param("page", "1"))
        .respond_with(page(1, 1_500))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&env.privacy)
        .await;
    Mock::given(method("GET"))
        .and(path("/transactions"))
        .and(wiremock::matchers::query_param("page", "2"))
        .respond_with(page(2, 997))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&env.privacy)
        .await;
    let (rid, _) = ask(&env, "purchase_complete", &json!({"purchase_id": pid, "status": "failed"})).await;
    assert_eq!(answer(&env, &rid).await["result"]["data"]["virtual_card"], "closed");
    // The card was closed before its charges were read, so the read is final: both pages count.
    let calls = env.privacy.received_requests().await.unwrap();
    let closed = calls.iter().position(|r| r.method.as_str() == "PATCH").unwrap();
    let last_read = calls.iter().rposition(|r| r.url.path() == "/transactions").unwrap();
    assert!(closed < last_read, "closed, then read");
    let spending = env.core.payments_spending(0).await.unwrap();
    assert_eq!(spending.purchases[0].charged_text.as_deref(), None);
    assert_eq!(spending.totals[0].minor, 2_497, "what both pages say was charged");

    // A card the user closes counts its cap until its charges are read after the closing.
    let (id, _) = ask(&env, "purchase_request", &cart_with(&json!({"payment_method": "virtual_card"}))).await;
    env.core.approve_purchase(id.clone(), choose("virtual_card")).await.unwrap();
    let pid = answer(&env, &id).await["result"]["data"]["purchase_id"].as_str().unwrap().to_owned();
    Mock::given(method("GET"))
        .and(path("/transactions"))
        .respond_with(ResponseTemplate::new(503))
        .with_priority(1)
        .mount(&env.privacy)
        .await;
    env.core.payments_close_card(pid.clone()).await.unwrap();
    let spending = env.core.payments_spending(0).await.unwrap();
    let closed = spending.purchases.iter().find(|p| p.id == pid).unwrap();
    assert!(!closed.card_open);
    assert_eq!(closed.counted_text, "$27.47", "its cap, while what was charged is unknown");
}
