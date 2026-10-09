//! Payments against a mock Vaultwarden (a card and an identity with an address) and a fake Privacy.com API: masked
//! lists, the purchase screen, approvals with each kind of payment method, budgets that refuse, spend limits that
//! approve, the ledger, and that card numbers never reach the activity log or the ledger.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use common::{FakeGoogle, FakeKeys, RecordingNotifier};
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
    let refused = desk.core.approve_purchase("s3".into(), choose("virtual_card")).await.unwrap_err();
    assert!(refused.to_string().contains("sealed only"), "{refused}");
    desk.core.approve_purchase("s3".into(), choose("merchant_account")).await.unwrap();
    assert_eq!(desk.data("s3").await["payment"]["kind"], "merchant_account");

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
    assert!(env.core.sync(0).await.unwrap().iter().any(|p| p.id == id), "an old request waits for the user");
    // Approved once, the same request is refused if it is ever relayed again (the ledger keeps it).
    env.core.approve_purchase(id.into(), choose("virtual_card")).await.unwrap();
    assert!(env.core.approve_purchase(id.into(), choose("virtual_card")).await.is_err());
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
