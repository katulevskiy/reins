//! Payments end to end: the real server lists the payment tools once the phone has Payments on; an AI asks to buy a
//! cart over MCP; the user approves it on the purchase screen; a virtual card is made at a fake Privacy.com for that
//! cart; the AI gets the card and a mandate it can verify, and reports the order, which lands in the ledger.

use std::time::Duration;

use reins_core::connector::payments::{PurchaseChoice, mandate};
use reins_e2e::{AiClient, Phone, Server};
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const EMAIL: &str = "buyer@example.com";
const PAN: &str = "4000123412341234";

async fn fake_privacy() -> MockServer {
    let privacy = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/cards"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [], "page": 1})))
        .mount(&privacy)
        .await;
    Mock::given(method("POST"))
        .and(path("/cards"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "token": "0f3c1d1e-5b7a-4f00-9a51-2b4b5f0f6a11", "pan": PAN, "cvv": "321", "exp_month": "10",
            "exp_year": "2031", "last_four": "1234", "state": "OPEN", "type": "MERCHANT_LOCKED"})))
        .mount(&privacy)
        .await;
    privacy
}

async fn tool_names(ai: &AiClient) -> Vec<String> {
    let (_, reply) = ai.rpc(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {}})).await;
    reply["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_owned()).collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_ai_buys_a_cart_the_user_approved_with_a_virtual_card() {
    let server = Server::start(20, 10).await;
    let privacy = fake_privacy().await;
    server.register(EMAIL).await;
    let base = privacy.uri();
    let phone = Phone::sign_in_with_config(&server.base, EMAIL, move |cfg| cfg.privacy_base = base).await;
    let mut ai = AiClient::new(&server.base);
    ai.register_client().await;
    let wait_url = ai.start_authorization(EMAIL).await;
    let code = ai.browser_code(&wait_url).await;
    let item = phone.wait_for_item(Duration::from_secs(20)).await;
    phone.core.answer_pairing(item.id, true, Some(code), Some("My Claude".to_owned())).await.unwrap();
    ai.finish_authorization(&wait_url).await;

    // Payments is off: the AI has no payment tools. The user switches it on with a Privacy.com key.
    assert!(!tool_names(&ai).await.iter().any(|t| t.starts_with("payments_")));
    phone.core.add_service_account("payments".to_owned(), String::new()).await.unwrap();
    phone.core.payments_connect_provider("privacy".into(), "personal-key".into(), false, false).await.unwrap();
    phone.core.sync(0).await.unwrap();
    let names = tool_names(&ai).await;
    for tool in
        ["payments_methods_list", "payments_addresses_list", "payments_purchase_request", "payments_purchase_complete"]
    {
        assert!(names.iter().any(|n| n == tool), "{tool} is listed once Payments is on");
    }

    // The server checks the cart before the phone sees it.
    let wrong = json!({"merchant": "Example Books", "merchant_url": "https://books.example.com/b/42",
        "items": [{"name": "Ebook", "unit_price": "12.00"}], "currency": "USD", "total": "10.00"});
    let refused = ai.tool("payments_purchase_request", &wrong).await;
    assert_eq!(refused["isError"], true);
    assert!(refused.to_string().contains("make 12.00"), "{refused}");

    let cart = json!({"merchant": "Example Books", "merchant_url": "https://books.example.com/b/42",
        "items": [{"name": "Ebook", "unit_price": "12.00"}], "currency": "USD", "total": "12.00",
        "ship_to": "none", "payment_method": "virtual_card", "note": "The book the user asked for."});
    let buying = ai.tool("payments_purchase_request", &cart);
    let user = async {
        let item = phone.wait_for_item(Duration::from_secs(20)).await;
        assert_eq!(item.title, "My Claude wants to buy 1 item at books.example.com for $12.00");
        let view = phone.core.approval_view(item.id.clone()).await.unwrap();
        let purchase = view.purchase.expect("the purchase screen");
        assert_eq!((purchase.total.as_str(), purchase.ships), ("$12.00", false));
        assert_eq!(purchase.note.as_deref(), Some("The book the user asked for."));
        let choice = PurchaseChoice {
            method_id: None,
            address_id: None,
            limit: None,
        };
        phone.core.approve_purchase(item.id.clone(), choice).await.unwrap();
        item.id
    };
    let (result, purchase_id) = tokio::join!(buying, user);
    assert_eq!(result["isError"], false, "{result}");
    let data: &Value = &result["structuredContent"];
    assert_eq!(data["purchase_id"], purchase_id);
    assert_eq!(
        (data["payment"]["kind"].as_str(), data["payment"]["number"].as_str()),
        (Some("virtual_card"), Some(PAN))
    );
    assert_eq!(data["payment"]["limit"], "13.20", "the total plus 10%");
    let made = privacy.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&made.iter().find(|r| r.method.as_str() == "POST").unwrap().body).unwrap();
    assert_eq!((body["spend_limit"].as_i64(), body["type"].as_str()), (Some(1_320), Some("MERCHANT_LOCKED")));
    let (signed, kid) = mandate::verify(data["mandate"].as_str().unwrap()).unwrap();
    assert_eq!((signed.purchase_id.as_str(), signed.agent.as_str()), (purchase_id.as_str(), "My Claude"));
    assert_eq!(kid, phone.core.payments_overview().await.unwrap().mandate_key);

    // The AI reports the order; the phone records it without asking (the app is open).
    let report = json!({"purchase_id": purchase_id, "status": "completed", "order_id": "EB-1001",
        "charged_total": "12.00", "currency": "USD"});
    let (reported, _) = tokio::join!(ai.tool("payments_purchase_complete", &report), phone.core.sync(10));
    assert_eq!(reported["isError"], false, "{reported}");
    let spending = phone.core.payments_spending(0).await.unwrap();
    assert_eq!((spending.purchases[0].status.as_str(), spending.totals[0].text.as_str()), ("completed", "$12.00"));
    assert_eq!(spending.by_ai[0].connection_label, "My Claude");
    assert!(!server.log().contains("panicked"), "server log: {}", server.log());
    assert!(!server.log().contains(PAN), "the server never logs a card number");
}
