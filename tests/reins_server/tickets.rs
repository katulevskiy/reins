//! Deferred decisions (Decisions v2, 03): tickets end to end against the real server, with a phone that publishes its
//! keys, opens sealed tickets and signs its transitions.

use data_encoding::BASE64URL_NOPAD;
use reins_proto::{
    ids::ConnectionId,
    relay::RelayRequest,
    ticket::{SealedTicketBody, SignedTicketEvent, TicketEvent, TicketState, digest_of, event_message},
};
use reqwest::StatusCode;
use ring::signature::{Ed25519KeyPair, KeyPair};
use serde_json::{Value, json};

use crate::harness::{Phone, Server, Tokens, client};

struct Keys {
    sign: Ed25519KeyPair,
    boxed: crypto_box::SecretKey,
}

impl Keys {
    fn new() -> Self {
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
        Self {
            sign: Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap(),
            boxed: crypto_box::SecretKey::generate(&mut crypto_box::aead::OsRng),
        }
    }

    fn sign_pub(&self) -> String {
        BASE64URL_NOPAD.encode(self.sign.public_key().as_ref())
    }

    fn box_pub(&self) -> String {
        BASE64URL_NOPAD.encode(self.boxed.public_key().as_bytes())
    }

    fn open(&self, sealed: &str) -> Vec<u8> {
        self.boxed.unseal(&BASE64URL_NOPAD.decode(sealed.as_bytes()).unwrap()).unwrap()
    }

    fn request(&self, item: &Value) -> RelayRequest {
        let body: SealedTicketBody = serde_json::from_slice(&self.open(item["sealed"].as_str().unwrap())).unwrap();
        body.request
    }

    fn signed(&self, request: &RelayRequest, state: TicketState, extra: impl FnOnce(&mut TicketEvent)) -> Value {
        let mut event = TicketEvent {
            v: 1,
            ticket: format!("rt_{}", request.id),
            from: TicketState::Deferred,
            state,
            at: 0,
            digest: digest_of(request),
            connection_id: ConnectionId(request.connection_id.0.clone()),
            deadline: None,
            note: None,
            result: None,
            stale: None,
        };
        extra(&mut event);
        let sig = BASE64URL_NOPAD.encode(self.sign.sign(&event_message(&event)).as_ref());
        serde_json::to_value(SignedTicketEvent {
            event,
            key: self.sign_pub(),
            sig,
        })
        .unwrap()
    }
}

async fn mcp(server: &Server, token: &str, body: &Value) -> Value {
    let response = client().post(server.url("/mcp")).bearer_auth(token).json(body).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    response.json::<Value>().await.unwrap()
}

/// A `tools/call` that asks for a ticket at once.
async fn deferred_call(server: &Server, token: &str, name: &str, arguments: Value) -> Value {
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": {"name": name, "arguments": arguments, "_meta": {"dev.reins/defer": true}}});
    mcp(server, token, &body).await["result"].clone()
}

async fn tool(server: &Server, token: &str, name: &str, arguments: Value) -> Value {
    let body =
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": name, "arguments": arguments}});
    mcp(server, token, &body).await["result"].clone()
}

fn send_args(subject: &str) -> Value {
    json!({"to": ["ann@acme.com"], "subject": subject, "body": "Numbers attached"})
}

async fn setup(keys: Option<&Keys>) -> (Server, Phone, Tokens) {
    let server = Server::start().await;
    let phone = server.phone("tia@example.com").await;
    let registration = match keys {
        Some(k) => json!({"fcm_token": null, "sign_pub": k.sign_pub(), "box_pub": k.box_pub()}),
        None => json!({"fcm_token": null}),
    };
    let (status, body) = phone.put("/device", &registration).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let tokens = server.connect_ai(&phone, "tia@example.com", Some("Claude")).await;
    (server, phone, tokens)
}

/// The phone's next ticket.
async fn next_ticket(phone: &Phone) -> Value {
    let (status, pending) = phone.get("/pending?wait=5").await;
    assert_eq!(status, StatusCode::OK, "{pending}");
    let item = pending["tickets"][0].clone();
    assert!(item.is_object(), "no ticket arrived: {pending}");
    item
}

fn database_bytes(server: &Server) -> Vec<u8> {
    let mut all = Vec::new();
    for name in ["db.sqlite3", "db.sqlite3-wal"] {
        if let Ok(bytes) = std::fs::read(server.data_dir().join(name)) {
            all.extend(bytes);
        }
    }
    all
}

fn contains(haystack: &[u8], needle: &str) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle.as_bytes())
}

#[tokio::test]
async fn a_deferred_send_is_stored_sealed_approved_and_collected_as_a_minimal_result() {
    let keys = Keys::new();
    let (server, phone, tokens) = setup(Some(&keys)).await;
    let result = deferred_call(&server, &tokens.access, "gmail_send", send_args("Quarterly-7c1e")).await;
    assert_eq!(result["isError"], false, "{result}");
    let parked = &result["structuredContent"];
    assert_eq!(parked["status"], "pending_review");
    assert!(parked["summary"].as_str().unwrap().starts_with("Send email to ann@acme.com"));
    let ticket = parked["ticket"].as_str().unwrap().to_owned();
    assert!(ticket.starts_with("rt_"));

    let item = next_ticket(&phone).await;
    assert_eq!((item["state"].as_str(), item["connection_label"].as_str()), (Some("deferred"), Some("Claude")));
    let request = keys.request(&item);
    assert_eq!(format!("rt_{}", request.id), ticket);
    assert_eq!(item["digest"], digest_of(&request), "the clear digest is the sealed body's");
    let db = database_bytes(&server);
    assert!(contains(&db, &request.id.0), "the ticket is stored");
    for clear in ["Quarterly-7c1e", "ann@acme.com", "Numbers attached"] {
        assert!(!contains(&db, clear), "{clear} is stored in clear");
    }

    // Still waiting: a second identical call shares the ticket instead of asking twice.
    let again = deferred_call(&server, &tokens.access, "gmail_send", send_args("Quarterly-7c1e")).await;
    assert_eq!(again["structuredContent"]["ticket"], ticket.as_str());

    let response = json!({"v": 1, "outcome": "result", "result": {"kind": "sent", "id": "m1", "thread_id": "t1"},
    "events": [keys.signed(&request, TicketState::Approved, |_| {}),
               keys.signed(&request, TicketState::Done, |e| {
                   e.from = TicketState::Approved;
                   e.result = Some(json!({"id": "m1", "body": "x"}));
               })]});
    let (status, body) = phone.post(&format!("/requests/{}/response", request.id), &response).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");

    let done = tool(&server, &tokens.access, "reins_get_result", json!({"ticket": ticket})).await;
    assert_eq!(done["structuredContent"]["status"], "done", "{done}");
    assert_eq!(done["structuredContent"]["result"], json!({"id": "m1", "thread_id": "t1"}));

    let listed = tool(&server, &tokens.access, "reins_tickets", json!({})).await;
    assert_eq!(listed["structuredContent"]["tickets"][0]["state"], "done", "{listed}");

    // Another connection of the same account learns nothing about it.
    let other = server.connect_ai(&phone, "tia@example.com", Some("ChatGPT")).await;
    let foreign = tool(&server, &other.access, "reins_get_result", json!({"ticket": ticket})).await;
    assert_eq!(foreign["isError"], true, "{foreign}");
    assert!(foreign["content"][0]["text"].as_str().unwrap().contains("Unknown or expired"));
    let listed = tool(&server, &other.access, "reins_tickets", json!({})).await;
    assert_eq!(listed["structuredContent"]["tickets"], json!([]));
}

#[tokio::test]
async fn forged_replayed_and_out_of_order_events_are_refused() {
    let keys = Keys::new();
    let (server, phone, tokens) = setup(Some(&keys)).await;
    deferred_call(&server, &tokens.access, "gmail_send", send_args("x")).await;
    let request = keys.request(&next_ticket(&phone).await);
    let events = |e: Value| json!({"v": 1, "events": [e]});
    let path = format!("/tickets/{}/events", request.id);

    let stranger = Keys::new();
    let (status, _) = phone.post(&path, &events(stranger.signed(&request, TicketState::Approved, |_| {}))).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "another key");
    let mut tampered = keys.signed(&request, TicketState::Approved, |_| {});
    tampered["event"]["state"] = json!("denied");
    let (status, _) = phone.post(&path, &events(tampered)).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "the signature covers the state");
    let mut other_digest = request.clone();
    other_digest.account = Some("else@x.com".to_owned());
    let (status, _) = phone.post(&path, &events(keys.signed(&other_digest, TicketState::Approved, |_| {}))).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "another request's approval never transfers");
    let (status, _) = phone.post(&path, &events(keys.signed(&request, TicketState::Done, |_| {}))).await;
    assert_eq!(status, StatusCode::CONFLICT, "done before approved");

    let denied = keys.signed(&request, TicketState::Denied, |_| {});
    let (status, body) = phone.post(&path, &events(denied.clone())).await;
    assert_eq!((status, body["state"].as_str()), (StatusCode::OK, Some("denied")));
    let (status, _) = phone.post(&path, &events(denied)).await;
    assert_eq!(status, StatusCode::CONFLICT, "a replay");
    let (status, _) = phone.post(&path, &events(keys.signed(&request, TicketState::Approved, |_| {}))).await;
    assert_eq!(status, StatusCode::CONFLICT, "decided is decided");
}

#[tokio::test]
async fn why_reply_and_a_denial_note_reach_each_side() {
    let keys = Keys::new();
    let (server, phone, tokens) = setup(Some(&keys)).await;
    let ticket =
        deferred_call(&server, &tokens.access, "gmail_send", send_args("y")).await["structuredContent"]["ticket"]
            .as_str()
            .unwrap()
            .to_owned();
    let request = keys.request(&next_ticket(&phone).await);
    let path = format!("/tickets/{}/events", request.id);
    let why = keys.signed(&request, TicketState::NeedsInfo, |e| e.note = Some("Why Acme?".to_owned()));
    let (status, _) = phone.post(&path, &json!({"v": 1, "events": [why]})).await;
    assert_eq!(status, StatusCode::OK);

    let asked = tool(&server, &tokens.access, "reins_get_result", json!({"ticket": ticket})).await;
    assert_eq!(asked["structuredContent"]["status"], "needs_info", "{asked}");
    assert_eq!(asked["structuredContent"]["question"], "Why Acme?");
    let replied =
        tool(&server, &tokens.access, "reins_reply", json!({"ticket": ticket, "text": "They asked for it"})).await;
    assert_eq!(replied["isError"], false, "{replied}");

    let item = next_ticket(&phone).await;
    assert_eq!(item["state"], "deferred");
    assert_eq!(keys.open(item["sealed_reply"].as_str().unwrap()), b"They asked for it");
    assert!(!contains(&database_bytes(&server), "They asked for it"), "the reply is stored sealed");

    let deny = keys.signed(&request, TicketState::Denied, |e| e.note = Some("Use the staging list".to_owned()));
    let (status, _) = phone.post(&path, &json!({"v": 1, "events": [deny]})).await;
    assert_eq!(status, StatusCode::OK);
    let denied = tool(&server, &tokens.access, "reins_wait", json!({"ticket": ticket, "max_seconds": 1})).await;
    assert_eq!(denied["isError"], true);
    assert!(denied["content"][0]["text"].as_str().unwrap().ends_with("Note: \"Use the staging list\""), "{denied}");
}

#[tokio::test]
async fn an_agent_can_withdraw_an_open_ticket_and_read_it_as_a_resource() {
    let keys = Keys::new();
    let (server, phone, tokens) = setup(Some(&keys)).await;
    let ticket =
        deferred_call(&server, &tokens.access, "gmail_send", send_args("z")).await["structuredContent"]["ticket"]
            .as_str()
            .unwrap()
            .to_owned();
    let uri = format!("reins://tickets/{ticket}");
    let listed = mcp(&server, &tokens.access, &json!({"jsonrpc": "2.0", "id": 3, "method": "resources/list"})).await;
    assert_eq!(listed["result"]["resources"][0]["uri"], uri.as_str(), "{listed}");
    let read = mcp(
        &server,
        &tokens.access,
        &json!({"jsonrpc": "2.0", "id": 4, "method": "resources/read", "params": {"uri": uri}}),
    )
    .await;
    let text: Value = serde_json::from_str(read["result"]["contents"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(text["state"], "deferred");

    let cancelled = tool(&server, &tokens.access, "reins_cancel", json!({"ticket": ticket})).await;
    assert_eq!(cancelled["structuredContent"]["status"], "cancelled", "{cancelled}");
    let item = next_ticket(&phone).await;
    assert_eq!(item["state"], "cancelled", "the phone drops it from its inbox");
    let again = tool(&server, &tokens.access, "reins_cancel", json!({"ticket": ticket})).await;
    assert_eq!(again["isError"], true);
}

#[tokio::test]
async fn without_a_box_key_nothing_is_deferred() {
    let (server, phone, tokens) = setup(None).await;
    let result = deferred_call(&server, &tokens.access, "gmail_send", send_args("w")).await;
    assert_ne!(result["structuredContent"]["status"], "pending_review", "{result}");
    let (_, pending) = phone.get("/pending?wait=1").await;
    assert_eq!(pending["requests"].as_array().map(Vec::len), Some(1), "an ordinary Instant request: {pending}");
    assert!(pending.get("tickets").is_none());
}

#[tokio::test]
async fn an_approval_cannot_be_replayed_after_a_later_transition() {
    let keys = Keys::new();
    let (server, phone, tokens) = setup(Some(&keys)).await;
    let ticket = deferred_call(&server, &tokens.access, "gmail_search", json!({"query": "from:bank"})).await
        ["structuredContent"]["ticket"]
        .as_str()
        .unwrap()
        .to_owned();
    let request = keys.request(&next_ticket(&phone).await);
    let path = format!("/tickets/{}/events", request.id);
    let approved = keys.signed(&request, TicketState::Approved, |_| {});
    let (status, _) = phone.post(&path, &json!({"v": 1, "events": [approved.clone()]})).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) =
        phone.put("/modes", &json!({"v": 1, "mode": "auto", "base": "auto", "desktops": [tokens.connection_id]})).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    // The world moved (a push's remote ref): Stale, asked again.
    let stale = client()
        .post(server.url(&format!("/reins/desktop/tickets/{ticket}/stale")))
        .bearer_auth(&tokens.access)
        .json(&json!({"diff": "refs/heads/main moved"}))
        .send()
        .await
        .unwrap();
    assert_eq!(stale.status(), StatusCode::OK);
    let (status, _) = phone.post(&path, &json!({"v": 1, "events": [approved]})).await;
    assert_eq!(status, StatusCode::CONFLICT, "the old approval was for another point of the ticket's history");
    let forged = keys.signed(&request, TicketState::Approved, |e| {
        e.from = TicketState::Stale;
        e.at = -1;
    });
    let (status, _) = phone.post(&path, &json!({"v": 1, "events": [forged]})).await;
    assert_eq!(status, StatusCode::CONFLICT, "time never goes back");
    let fresh = keys.signed(&request, TicketState::Approved, |e| {
        e.from = TicketState::Stale;
        e.at = 1;
    });
    let (status, _) = phone.post(&path, &json!({"v": 1, "events": [fresh]})).await;
    assert_eq!(status, StatusCode::OK, "a new review, signed anew, counts");
}

#[tokio::test]
async fn an_unsigned_answer_never_moves_an_open_ticket() {
    let keys = Keys::new();
    let (server, phone, tokens) = setup(Some(&keys)).await;
    let ticket =
        deferred_call(&server, &tokens.access, "gmail_send", send_args("race")).await["structuredContent"]["ticket"]
            .as_str()
            .unwrap()
            .to_owned();
    let request = keys.request(&next_ticket(&phone).await);
    let response = json!({"v": 1, "outcome": "result", "result": {"kind": "sent", "id": "m7", "thread_id": "t7"}});
    let (status, _) = phone.post(&format!("/requests/{}/response", request.id), &response).await;
    assert_eq!(status, StatusCode::CONFLICT, "only the device's signed approval moves it");
    let denied = json!({"v": 1, "outcome": "denied", "reason": null});
    let (status, _) = phone.post(&format!("/requests/{}/response", request.id), &denied).await;
    assert_eq!(status, StatusCode::CONFLICT);
    let listed = tool(&server, &tokens.access, "reins_tickets", json!({})).await;
    assert_eq!(listed["structuredContent"]["tickets"][0]["state"], "deferred", "{listed}");
    assert_eq!(listed["structuredContent"]["tickets"][0]["ticket"], ticket.as_str());
}

#[tokio::test]
async fn stale_comes_only_from_the_desktop_app_and_asks_again_with_a_new_window() {
    let keys = Keys::new();
    let (server, phone, tokens) = setup(Some(&keys)).await;
    let ticket = deferred_call(&server, &tokens.access, "gmail_search", json!({"query": "from:bank"})).await
        ["structuredContent"]["ticket"]
        .as_str()
        .unwrap()
        .to_owned();
    let first = next_ticket(&phone).await;
    let request = keys.request(&first);
    let approved = keys.signed(&request, TicketState::Approved, |_| {});
    let (status, _) =
        phone.post(&format!("/tickets/{}/events", request.id), &json!({"v": 1, "events": [approved]})).await;
    assert_eq!(status, StatusCode::OK);
    let report = |token: &str| {
        client()
            .post(server.url(&format!("/reins/desktop/tickets/{ticket}/stale")))
            .bearer_auth(token.to_owned())
            .json(&json!({"diff": "refs/heads/main moved from 1a2b3c4 to 5d6e7f8"}))
            .send()
    };
    assert_eq!(report(&tokens.access).await.unwrap().status(), StatusCode::FORBIDDEN, "an AI's connection");
    let (status, _) =
        phone.put("/modes", &json!({"v": 1, "mode": "auto", "base": "auto", "desktops": [tokens.connection_id]})).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(report(&tokens.access).await.unwrap().status(), StatusCode::OK, "the paired desktop app");
    let again = next_ticket(&phone).await;
    assert_eq!(again["state"], "stale");
    assert_eq!(again["stale"], "refs/heads/main moved from 1a2b3c4 to 5d6e7f8");
    assert!(again["deferred_at"].as_i64() >= first["deferred_at"].as_i64(), "a new window");
    assert!(again["deadline"].as_i64() >= first["deadline"].as_i64());
}

#[tokio::test]
async fn an_identical_call_shares_the_open_ticket_instead_of_asking_twice() {
    let keys = Keys::new();
    let (server, phone, tokens) = setup(Some(&keys)).await;
    let first = deferred_call(&server, &tokens.access, "gmail_send", send_args("twice")).await;
    let started = std::time::Instant::now();
    let again = tool(&server, &tokens.access, "gmail_send", send_args("twice")).await;
    assert!(started.elapsed() < std::time::Duration::from_secs(2), "no Instant wait: {again}");
    assert_eq!(again["structuredContent"]["status"], "pending_review");
    assert_eq!(again["structuredContent"]["ticket"], first["structuredContent"]["ticket"]);
    let (_, pending) = phone.get("/pending?wait=1").await;
    assert!(pending.get("requests").and_then(Value::as_array).is_none_or(Vec::is_empty), "no second card: {pending}");
}

#[tokio::test]
async fn a_request_the_phone_has_cannot_be_withdrawn_or_parked_behind_its_back() {
    let keys = Keys::new();
    let (server, phone, tokens) = setup(Some(&keys)).await;
    let call = tool(&server, &tokens.access, "gmail_send", send_args("instant"));
    let user = async {
        let (_, pending) = phone.get("/pending?wait=5").await;
        pending["requests"][0]["id"].as_str().unwrap().to_owned()
    };
    let (_, id) = tokio::join!(call, user);
    let refused = tool(&server, &tokens.access, "reins_cancel", json!({"ticket": id})).await;
    assert_eq!(refused["isError"], true);
    assert!(refused["content"][0]["text"].as_str().unwrap().contains("already has"), "{refused}");
}

#[tokio::test]
async fn a_new_approval_phone_closes_what_was_sealed_to_the_old_one() {
    let keys = Keys::new();
    let (server, phone, tokens) = setup(Some(&keys)).await;
    deferred_call(&server, &tokens.access, "gmail_send", send_args("old phone")).await;
    let other = Keys::new();
    let (status, _) = phone
        .put("/device", &json!({"fcm_token": null, "sign_pub": other.sign_pub(), "box_pub": other.box_pub()}))
        .await;
    assert_eq!(status, StatusCode::OK);
    let listed = tool(&server, &tokens.access, "reins_tickets", json!({})).await;
    assert_eq!(listed["structuredContent"]["tickets"][0]["state"], "expired", "{listed}");
}

#[tokio::test]
async fn a_collected_result_nobody_waited_for_is_kept_until_an_agent_takes_it() {
    let keys = Keys::new();
    let (server, phone, tokens) = setup(Some(&keys)).await;
    let ticket = deferred_call(&server, &tokens.access, "gmail_search", json!({"query": "from:bank"})).await
        ["structuredContent"]["ticket"]
        .as_str()
        .unwrap()
        .to_owned();
    let request = keys.request(&next_ticket(&phone).await);
    let approved = keys.signed(&request, TicketState::Approved, |_| {});
    let (status, _) =
        phone.post(&format!("/tickets/{}/events", request.id), &json!({"v": 1, "events": [approved]})).await;
    assert_eq!(status, StatusCode::OK);
    // The agent asks for it and gives up quickly; the phone runs the read after that.
    let gave_up = tool(&server, &tokens.access, "reins_wait", json!({"ticket": ticket, "max_seconds": 1})).await;
    assert_eq!(gave_up["structuredContent"]["status"], "approved", "{gave_up}");
    let found = json!({"v": 1, "outcome": "result", "result": {"kind": "search", "messages": [{"id": "m1",
        "thread_id": "t1", "from": "a@bank.com", "to": [], "subject": "S", "date": 0, "snippet": "x"}]}});
    let (status, _) = phone.post(&format!("/requests/{}/response", request.id), &found).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let listed = tool(&server, &tokens.access, "reins_tickets", json!({})).await;
    assert_eq!(listed["structuredContent"]["tickets"][0]["state"], "approved", "not spent until received");
    let got = tool(&server, &tokens.access, "reins_get_result", json!({"ticket": ticket})).await;
    assert_eq!(got["structuredContent"]["messages"][0]["id"], "m1", "{got}");
    let listed = tool(&server, &tokens.access, "reins_tickets", json!({})).await;
    assert_eq!(listed["structuredContent"]["tickets"][0]["state"], "done", "received: spent");
}
