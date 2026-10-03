//! Whole-system check against a deployed server: a headless phone (real core, fake Gmail) signs in with a real
//! account, a simulated AI client connects through OAuth + the phone-code match, searches mail, and the phone
//! approves. Everything the user's phone and Claude would do, minus real Gmail.
//!
//! ```text
//! REINS_PASSWORD='...' cargo run -p reins-e2e --example remote_smoke -- https://reins.example.com you@example.com
//! ```
//!
//! It leaves nothing behind except this headless device as the approval device (sign in on your phone to replace it)
//! and disconnects the simulated AI when done.

use std::time::Duration;

use reins_core::ApprovalChoice;
use reins_e2e::{AiClient, Phone};
use serde_json::json;

#[tokio::main]
async fn main() {
    reins_e2e::init_tls();
    let mut args = std::env::args().skip(1);
    let (Some(server), Some(email)) = (args.next(), args.next()) else {
        eprintln!("usage: remote_smoke <server-url> <email>   (password in REINS_PASSWORD)");
        std::process::exit(2);
    };
    let password = std::env::var("REINS_PASSWORD").expect("set REINS_PASSWORD");
    let server = server.trim_end_matches('/').to_owned();

    let phone = Phone::sign_in_with(&server, &email, &password).await;
    println!("phone: signed in and registered");
    phone.stock_gmail(&[("m1", "Bank <alerts@bank.com>"), ("m2", "Spam <x@spam.example>")]).await;

    let mut ai = AiClient::new(&server);
    ai.register_client().await;
    let wait_url = ai.start_authorization(&email).await;
    let code = ai.browser_code(&wait_url).await;
    let item = phone.wait_for_item(Duration::from_secs(30)).await;
    let view = phone.core.pairing_view(item.id.clone()).await.expect("pairing view");
    assert!(view.choices.contains(&code), "the browser code is one of the phone's three choices");
    phone.core.answer_pairing(item.id, true, Some(code), Some("remote smoke".to_owned())).await.expect("pair");
    ai.finish_when_approved(&wait_url, Duration::from_secs(30)).await;
    println!("ai: connected (code {code} matched)");

    let query = json!({"query": "from:bank"});
    let search = ai.tool("gmail_search", &query);
    let user = async {
        let item = phone.wait_for_item(Duration::from_secs(30)).await;
        let view = phone.core.approval_view(item.id.clone()).await.expect("approval view");
        println!("phone: asked to approve `{}` with {} messages", item.title, view.messages.len());
        let choice = ApprovalChoice {
            selected_message_ids: vec!["m1".to_owned()],
            standing: None,
        };
        phone.core.approve(item.id, choice).await.expect("approve");
    };
    let (result, ()) = tokio::join!(search, user);
    assert_eq!(result["isError"], false, "{result}");
    let ids: Vec<&str> = result["structuredContent"]["messages"]
        .as_array()
        .expect("messages")
        .iter()
        .filter_map(|m| m["id"].as_str())
        .collect();
    assert_eq!(ids, ["m1"], "only the approved message reached the AI");
    println!("ai: received exactly the approved message");

    for connection in phone.core.connections().await.expect("connections") {
        phone.core.revoke_connection(connection.id).await.expect("revoke");
    }
    println!("OK: end-to-end through {server}");
}
