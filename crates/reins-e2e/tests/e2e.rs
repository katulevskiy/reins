//! The whole system: real server binary + real phone core + simulated AI client.

use std::time::Duration;

use reins_core::{ApprovalChoice, ApprovalKind, GrantScopeChoice, StandingGrant};
use reins_e2e::{AiClient, Phone, Server};
use reqwest::StatusCode;
use serde_json::{Value, json};

const EMAIL: &str = "user@example.com";

fn scope() -> GrantScopeChoice {
    GrantScopeChoice {
        all_mail: false,
        selected_messages_only: false,
        sender_addresses: vec![],
        sender_domains: vec![],
        subject_pattern: None,
        recipient_addresses: vec![],
        resources: vec![],
        classes: vec![],
        recipient_domains: vec![],
    }
}

fn pick(ids: &[&str], standing: Option<StandingGrant>) -> ApprovalChoice {
    ApprovalChoice {
        selected_message_ids: ids.iter().map(|s| (*s).to_owned()).collect(),
        standing,
    }
}

/// Registers the account, signs the phone in and connects an AI client end to end.
async fn connected(relay_wait: u64, offline: u64) -> (Server, Phone, AiClient) {
    let server = Server::start(relay_wait, offline).await;
    server.register(EMAIL).await;
    let phone = Phone::sign_in(&server.base, EMAIL).await;
    let mut ai = AiClient::new(&server.base);
    ai.register_client().await;
    let wait_url = ai.start_authorization(EMAIL).await;
    let code = ai.browser_code(&wait_url).await;
    // The user opens the app: the connection request is waiting; they pick the number shown in the browser.
    let item = phone.wait_for_item(Duration::from_secs(20)).await;
    assert_eq!(item.title, "Connect Claude to Reins?");
    let view = phone.core.pairing_view(item.id.clone()).await.unwrap();
    assert!(view.choices.contains(&code));
    phone.core.answer_pairing(item.id, true, Some(code), Some("My Claude".to_owned())).await.unwrap();
    ai.finish_authorization(&wait_url).await;
    (server, phone, ai)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn connect_search_approve_with_a_standing_grant_then_auto_allow() {
    let (server, phone, ai) = connected(20, 8).await;
    phone.stock_gmail(&[("m1", "Bank <alerts@bank.com>"), ("m2", "Spam <x@spam.example>")]).await;

    // The AI searches; nothing covers it, so the phone asks the user.
    let query = json!({"query": "from:bank"});
    let search = ai.tool("gmail_search", &query);
    let user = async {
        let item = phone.wait_for_item(Duration::from_secs(20)).await;
        assert_eq!(item.title, "My Claude wants to search your Gmail");
        let view = phone.core.approval_view(item.id.clone()).await.unwrap();
        assert_eq!((view.kind, view.messages.len()), (ApprovalKind::Search, 2));
        let mut only_bank = scope();
        only_bank.sender_domains = vec!["bank.com".to_owned()];
        let standing = StandingGrant {
            duration_secs: Some(3600),
            max_uses: None,
            scope: only_bank,
        };
        // The user shares just the bank message and allows the bank from now on.
        phone.core.approve(item.id, pick(&["m1"], Some(standing))).await.unwrap();
    };
    let (result, ()) = tokio::join!(search, user);
    assert_eq!(result["isError"], false, "{result}");
    let ids: Vec<&str> =
        result["structuredContent"]["messages"].as_array().unwrap().iter().map(|m| m["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["m1"], "the unapproved message never left the phone");

    // Now bank mail is covered: the same kind of search is answered without any prompt.
    phone.gmail.reset().await;
    phone.stock_gmail(&[("m1", "Bank <alerts@bank.com>")]).await;
    let query = json!({"query": "from:bank"});
    let again = ai.tool("gmail_search", &query);
    // The app is open (long-polling): it receives the request and answers it by itself.
    let app_open = async { phone.core.sync(10).await.unwrap() };
    let (again, still_pending) = tokio::join!(again, app_open);
    assert!(still_pending.is_empty(), "nothing needed the user");
    assert_eq!(again["isError"], false, "{again}");
    assert!(phone.core.pending().await.unwrap().is_empty());
    assert_eq!(phone.core.grants().await.unwrap().len(), 1);
    assert!(!server.log().contains("panicked"), "server log: {}", server.log());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sending_denial_and_offline_behaviour() {
    let (_server, phone, ai) = connected(6, 3).await;
    phone.accept_sends().await;

    // The user denies a send.
    let args = json!({"to": ["boss@work.com"], "subject": "Report", "body": "Attached soon."});
    let call = ai.tool("gmail_send", &args);
    let user = async {
        let item = phone.wait_for_item(Duration::from_secs(20)).await;
        let view = phone.core.approval_view(item.id.clone()).await.unwrap();
        assert_eq!(view.email.unwrap().to, ["boss@work.com"]);
        phone.core.deny(item.id).await.unwrap();
    };
    let (result, ()) = tokio::join!(call, user);
    assert_eq!(result["isError"], true);
    assert_eq!(result["content"][0]["text"], "Denied by the user on their Reins device.");

    // The user approves a send.
    let call = ai.tool("gmail_send", &args);
    let user = async {
        let item = phone.wait_for_item(Duration::from_secs(20)).await;
        phone.core.approve(item.id, pick(&[], None)).await.unwrap();
    };
    let (result, ()) = tokio::join!(call, user);
    assert_eq!(result["isError"], false, "{result}");
    assert_eq!(result["structuredContent"]["id"], "sent-1");

    // The app is closed: the AI is told the device is offline, and can collect the answer later.
    let started = std::time::Instant::now();
    let offline = ai.tool("gmail_search", &json!({"query": "anything"})).await;
    assert!(started.elapsed() < Duration::from_secs(5), "offline is reported after the short threshold");
    let text = offline["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("your approval device is offline"), "{text}");
    let request_id = text.split("request_id=").nth(1).unwrap().trim_end_matches('.').to_owned();

    phone.stock_gmail(&[("m9", "Someone <s@x.com>")]).await;
    let item = phone.wait_for_item(Duration::from_secs(20)).await;
    phone.core.approve(item.id, pick(&["m9"], None)).await.unwrap();
    let late = ai.tool("reins_get_result", &json!({"request_id": request_id})).await;
    assert_eq!(late["isError"], false, "{late}");
    assert_eq!(late["structuredContent"]["messages"][0]["id"], "m9");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn revoking_the_connection_cuts_the_ai_off() {
    let (_server, phone, ai) = connected(6, 3).await;
    let connections = phone.core.connections().await.unwrap();
    assert_eq!((connections.len(), connections[0].label.as_str()), (1, "My Claude"));
    assert_eq!(connections[0].key_fingerprint, None, "an AI app pins no key");
    let (status, _) = ai.rpc(&json!({"jsonrpc": "2.0", "id": 1, "method": "ping"})).await;
    assert_eq!(status, StatusCode::OK);
    phone.core.revoke_connection(connections[0].id.clone()).await.unwrap();
    let (status, _) = ai.rpc(&json!({"jsonrpc": "2.0", "id": 2, "method": "ping"})).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(phone.core.connections().await.unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_ai_asks_for_a_narrow_permission_and_then_reads_without_prompts() {
    let (server, phone, ai) = connected(20, 8).await;
    phone.stock_gmail(&[("m1", "Bank <alerts@bank.com>")]).await;

    let ask = json!({
        "action": "read", "duration_seconds": 1800, "reason": "Summarise this week's bank statements",
        "from": ["@bank.com"]
    });
    let request = ai.tool("reins_request_access", &ask);
    let user = async {
        let item = phone.wait_for_item(Duration::from_secs(20)).await;
        assert_eq!(item.action, "grant");
        let view = phone.core.approval_view(item.id.clone()).await.unwrap();
        assert_eq!(view.kind, ApprovalKind::Grant);
        assert_eq!(view.grant.unwrap().lines, ["From @bank.com"]);
        phone.core.approve(item.id, pick(&[], None)).await.unwrap();
    };
    let (result, ()) = tokio::join!(request, user);
    assert_eq!(result["isError"], false, "{result}");
    assert_eq!(result["structuredContent"]["granted"], true);

    // The permission covers the search: the phone answers by itself while the app is open.
    let query = json!({"query": "from:bank"});
    let search = ai.tool("gmail_search", &query);
    let app_open = async { phone.core.sync(10).await.unwrap() };
    let (search, waiting) = tokio::join!(search, app_open);
    assert!(waiting.is_empty(), "nothing needed the user");
    assert_eq!(search["isError"], false, "{search}");
    assert!(!server.log().contains("panicked"), "server log: {}", server.log());

    // Asking for something unreasonable is refused before the user is ever bothered.
    let greedy = ai
        .rpc(&json!({"jsonrpc": "2.0", "id": 9, "method": "tools/call", "params": {"name": "reins_request_access",
            "arguments": {"action": "send", "duration_seconds": 600, "reason": "x", "any": true}}}))
        .await
        .1;
    assert_eq!(greedy["result"]["isError"], true);
    assert!(phone.core.pending().await.unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_ai_sees_the_integrations_and_gets_their_accounts_only_when_the_user_allows_it() {
    let (server, phone, ai) = connected(20, 8).await;
    phone.stock_gmail(&[("m1", "Bank <alerts@bank.com>")]).await;

    // The integrations need no approval and show no accounts.
    let none = json!({});
    let listing = ai.tool("reins_list_accounts", &none);
    let app_open = async { phone.core.sync(10).await.unwrap() };
    let (listing, waiting) = tokio::join!(listing, app_open);
    assert!(waiting.is_empty(), "nothing needed the user");
    assert_eq!(listing["isError"], false, "{listing}");
    // In the order they were first connected: the vault and Gmail can register in either order here.
    let mut integrations = listing["structuredContent"]["integrations"].as_array().cloned().unwrap_or_default();
    integrations.sort_by_key(|i| i["service"].as_str().unwrap_or_default().to_owned());
    assert_eq!(
        Value::Array(integrations),
        json!([
            {"service": "gmail", "name": "Gmail"},
            {"service": "vault", "name": "Password vault"}
        ])
    );
    assert!(!listing.to_string().contains(reins_e2e::phone::GMAIL_ACCOUNT), "{listing}");

    // The accounts wait for the user, who allows them for a month.
    let gmail = json!({"service": "gmail"});
    let accounts = ai.tool("reins_list_accounts", &gmail);
    let user = async {
        let item = phone.wait_for_item(Duration::from_secs(20)).await;
        assert_eq!((item.action.as_str(), item.count), ("accounts", 1));
        let view = phone.core.approval_view(item.id.clone()).await.unwrap();
        assert_eq!(view.accounts, [reins_e2e::phone::GMAIL_ACCOUNT]);
        let month = StandingGrant {
            duration_secs: Some(30 * 86_400),
            max_uses: None,
            scope: scope(),
        };
        phone.core.approve(item.id, pick(&[reins_e2e::phone::GMAIL_ACCOUNT], Some(month))).await.unwrap();
    };
    let (accounts, ()) = tokio::join!(accounts, user);
    assert_eq!(accounts["isError"], false, "{accounts}");
    assert_eq!(
        accounts["structuredContent"]["accounts"],
        json!([{"service": "gmail", "account": reins_e2e::phone::GMAIL_ACCOUNT}])
    );

    // The month-long grant answers the next request by itself.
    let again = ai.tool("reins_list_accounts", &gmail);
    let app_open = async { phone.core.sync(10).await.unwrap() };
    let (again, waiting) = tokio::join!(again, app_open);
    assert!(waiting.is_empty());
    assert_eq!(again["isError"], false, "{again}");

    // Naming an account that is not connected is refused without revealing the ones that are.
    let elsewhere = json!({"query": "x", "account": "someone@else.com"});
    let wrong = ai.tool("gmail_search", &elsewhere);
    let app_open = async { phone.core.sync(10).await.unwrap() };
    let (wrong, _) = tokio::join!(wrong, app_open);
    assert_eq!(wrong["isError"], true);
    let text = wrong["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("not connected") && !text.contains(reins_e2e::phone::GMAIL_ACCOUNT), "{wrong}");
    assert!(!server.log().contains("panicked"), "server log: {}", server.log());
}
