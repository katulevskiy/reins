//! Deferred decisions end to end (Decisions v2, 03): the real server, the real phone core and an AI client. Tickets
//! are sealed to the phone, wait past the relay's ten minutes, are decided later, and their outcome is collected.

use std::time::Duration;

use reins_core::ApprovalChoice;
use reins_e2e::{AiClient, Phone, Server};
use serde_json::{Value, json};

const EMAIL: &str = "user@example.com";

async fn connected() -> (Server, Phone, AiClient) {
    let server = Server::start(6, 3).await;
    server.register(EMAIL).await;
    let phone = Phone::sign_in(&server.base, EMAIL).await;
    let mut ai = AiClient::new(&server.base);
    ai.register_client().await;
    let wait_url = ai.start_authorization(EMAIL).await;
    let code = ai.browser_code(&wait_url).await;
    let item = phone.wait_for_item(Duration::from_secs(20)).await;
    phone.core.answer_pairing(item.id, true, Some(code), Some("Claude".to_owned()), true).await.unwrap();
    ai.finish_authorization(&wait_url).await;
    (server, phone, ai)
}

/// A `tools/call` that asks for a ticket at once.
async fn deferred(ai: &AiClient, name: &str, arguments: &Value) -> Value {
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": {"name": name, "arguments": arguments, "_meta": {"dev.reins/defer": true}}});
    ai.rpc(&body).await.1["result"].clone()
}

fn none() -> ApprovalChoice {
    ApprovalChoice {
        selected_message_ids: vec![],
        standing: None,
    }
}

fn now() -> i64 {
    i64::try_from(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_deferred_send_waits_until_its_deadline_and_is_collected_as_a_minimal_result() {
    let (_server, phone, ai) = connected().await;
    phone.accept_sends().await;
    let args = json!({"to": ["boss@work.com"], "subject": "Report", "body": "Attached"});
    let parked = deferred(&ai, "gmail_send", &args).await;
    assert_eq!(parked["structuredContent"]["status"], "pending_review", "{parked}");
    let ticket = parked["structuredContent"]["ticket"].as_str().unwrap().to_owned();

    let item = phone.wait_for_item(Duration::from_secs(20)).await;
    let view = item.ticket.clone().expect("shown as a ticket");
    assert_eq!(view.ticket, ticket);
    assert!(view.deadline > now() + 3 * 3600, "a Deferred window, not the relay's ten minutes");
    assert_eq!(item.wait_until, Some(view.deadline));
    assert!(!item.can_defer, "already a ticket");

    phone.core.approve(item.id, none()).await.unwrap();
    let done = ai.tool("reins_get_result", &json!({"ticket": ticket})).await;
    assert_eq!(done["structuredContent"]["status"], "done", "{done}");
    assert_eq!(done["structuredContent"]["result"], json!({"id": "sent-1", "thread_id": "t-sent"}));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn later_parks_an_instant_card_and_a_denial_note_reaches_the_agent() {
    let (_server, phone, ai) = connected().await;
    let args = json!({"to": ["ann@acme.com"], "subject": "Hi", "body": "Hello"});
    let call = ai.tool("gmail_send", &args);
    let user = async {
        let item = phone.wait_for_item(Duration::from_secs(20)).await;
        assert!(item.can_defer && item.ticket.is_none(), "an Instant card");
        phone.core.decide_later(item.id.clone()).await.unwrap();
        item.id
    };
    let (result, id) = tokio::join!(call, user);
    assert_eq!(result["isError"], false, "the agent is told at once: {result}");
    assert_eq!(result["structuredContent"]["status"], "pending_review");
    let ticket = result["structuredContent"]["ticket"].as_str().unwrap().to_owned();
    let items = phone.core.pending().await.unwrap();
    let waiting = items.iter().find(|i| i.id == id).expect("still waiting for the user");
    assert_eq!(waiting.ticket.as_ref().map(|t| t.state.as_str()), Some("deferred"));

    phone.core.ask_why(id.clone(), "Why Acme?".to_owned()).await.unwrap();
    let asked = ai.tool("reins_get_result", &json!({"ticket": ticket})).await;
    assert_eq!(asked["structuredContent"]["question"], "Why Acme?", "{asked}");
    let replied = ai.tool("reins_reply", &json!({"ticket": ticket, "text": "They asked for the numbers"})).await;
    assert_eq!(replied["isError"], false, "{replied}");
    let mut reply = None;
    for _ in 0..10 {
        let items = phone.core.sync(1).await.unwrap();
        reply = items.iter().find(|i| i.id == id).and_then(|i| i.ticket.as_ref()?.agent_reply.clone());
        if reply.is_some() {
            break;
        }
    }
    assert_eq!(reply.as_deref(), Some("They asked for the numbers"));

    phone.core.deny_with_note(id, Some("Use the staging list".to_owned())).await.unwrap();
    let denied = ai.tool("reins_get_result", &json!({"ticket": ticket})).await;
    assert_eq!(denied["isError"], true);
    assert_eq!(
        denied["content"][0]["text"],
        "Denied by the user on their Reins device. Note: \"Use the staging list\""
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_approved_read_runs_only_when_the_agent_collects_it() {
    let (_server, phone, ai) = connected().await;
    phone.stock_gmail(&[("m1", "Bank <alerts@bank.com>"), ("m2", "Bank <alerts@bank.com>")]).await;
    let parked = deferred(&ai, "gmail_search", &json!({"query": "from:bank"})).await;
    let ticket = parked["structuredContent"]["ticket"].as_str().unwrap().to_owned();
    let item = phone.wait_for_item(Duration::from_secs(20)).await;
    phone
        .core
        .approve(
            item.id,
            ApprovalChoice {
                selected_message_ids: vec!["m1".to_owned()],
                standing: None,
            },
        )
        .await
        .unwrap();
    let listed = ai.tool("reins_tickets", &json!({})).await;
    assert_eq!(listed["structuredContent"]["tickets"][0]["state"], "approved", "only the decision is kept: {listed}");

    // The agent collects: the phone (its app open) runs the search now and the result passes straight through.
    let args = json!({"ticket": ticket});
    let collect = ai.tool("reins_get_result", &args);
    let app = async {
        for _ in 0..15 {
            phone.core.sync(1).await.unwrap();
        }
    };
    let (result, ()) = tokio::join!(collect, app);
    assert_eq!(result["isError"], false, "{result}");
    let ids: Vec<&str> =
        result["structuredContent"]["messages"].as_array().unwrap().iter().filter_map(|m| m["id"].as_str()).collect();
    assert_eq!(ids, ["m1"], "only what the owner released, never m2: {result}");

    // One use: the approval is spent; the ticket is closed and a later collect runs nothing.
    let mut state = Value::Null;
    for _ in 0..10 {
        let listed = ai.tool("reins_tickets", &json!({})).await;
        state = listed["structuredContent"]["tickets"][0]["state"].clone();
        if state == "done" {
            break;
        }
        phone.core.sync(1).await.unwrap();
    }
    assert_eq!(state, "done");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_ticket_withdrawn_before_the_approval_never_runs() {
    let (server, phone, ai) = connected().await;
    phone.accept_sends().await;
    let args = json!({"to": ["boss@work.com"], "subject": "Report", "body": "Attached"});
    let ticket = deferred(&ai, "gmail_send", &args).await["structuredContent"]["ticket"].as_str().unwrap().to_owned();
    let item = phone.wait_for_item(Duration::from_secs(20)).await;
    let cancelled = ai.tool("reins_cancel", &json!({"ticket": ticket})).await;
    assert_eq!(cancelled["structuredContent"]["status"], "cancelled", "{cancelled}");
    // The phone has not heard yet: its approval is refused by the server before anything is sent.
    assert!(phone.core.approve(item.id, none()).await.is_err());
    assert!(phone.sent_emails().await.is_empty(), "nothing was sent");
    drop(server);
}
