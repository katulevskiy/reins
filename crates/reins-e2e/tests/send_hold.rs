//! The email send hold (Decisions v2, 01 §5, 08 R10), end to end: the real server binary, the real phone core, an AI
//! client and a loopback push service that records the server's wake-ups.
//!
//! - send, Undo: nothing is sent, and the server's ciphertext is gone;
//! - send, release: the server wakes the phone, which sends exactly once, and the ciphertext is gone;
//! - an offline phone that wakes past the limit asks again instead of sending;
//! - the agent's `reins_cancel` and the desktop's Undo delete it; another account can't;
//! - while held, the server keeps only a ciphertext.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use reins_core::HoldState;
use reins_e2e::{AiClient, Phone, Server};
use serde_json::{Value, json};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const EMAIL: &str = "user@example.com";
const HOLD_SECS: u32 = 2;
const SUBJECT: &str = "Quarterly numbers";
const BODY: &str = "The draft is attached in spirit.";
const TO: &str = "ada@example.org";

/// The loopback push service: every push the server sent, `(t, id)`.
#[derive(Clone, Default)]
struct Pushes(Arc<Mutex<Vec<(String, String)>>>);

impl Respond for Pushes {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let body: Value = serde_json::from_slice(&request.body).unwrap_or_default();
        let kind = body["t"].as_str().unwrap_or_default().to_owned();
        let id = body["id"].as_str().unwrap_or_default().to_owned();
        self.0.lock().unwrap().push((kind, id));
        ResponseTemplate::new(200)
    }
}

impl Pushes {
    /// Waits up to `within` for a `hold` push for `id`.
    async fn hold(&self, id: &str, within: Duration) -> bool {
        let deadline = Instant::now() + within;
        while Instant::now() < deadline {
            if self.0.lock().unwrap().iter().any(|(t, i)| t == "hold" && i == id) {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        false
    }
}

struct World {
    server: Server,
    phone: Phone,
    ai: AiClient,
    pushes: Pushes,
    _push_service: MockServer,
}

async fn pair(server: &Server, email: &str) -> (Phone, AiClient) {
    server.register(email).await;
    let phone = Phone::sign_in(&server.base, email).await;
    let mut ai = AiClient::new(&server.base);
    ai.register_client().await;
    let wait_url = ai.start_authorization(email).await;
    let code = ai.browser_code(&wait_url).await;
    let item = phone.wait_for_item(Duration::from_secs(20)).await;
    phone.core.answer_pairing(item.id, true, Some(code), Some("Claude".to_owned()), true).await.unwrap();
    ai.finish_authorization(&wait_url).await;
    phone.accept_sends().await;
    phone.core.set_send_hold_secs(HOLD_SECS).await.unwrap();
    (phone, ai)
}

async fn world() -> World {
    let push_service = MockServer::start().await;
    let pushes = Pushes::default();
    Mock::given(wiremock::matchers::method("POST")).respond_with(pushes.clone()).mount(&push_service).await;
    let env = [("REINS_TEST_PUSH_URL".to_owned(), push_service.uri())];
    let server = Server::start_with_env(20, 8, &env).await;
    let (phone, ai) = pair(&server, EMAIL).await;
    World {
        server,
        phone,
        ai,
        pushes,
        _push_service: push_service,
    }
}

fn db(server: &Server) -> rusqlite::Connection {
    rusqlite::Connection::open(server.database_path()).unwrap()
}

/// `(id, sealed)` of every hold the server keeps.
fn kept(server: &Server) -> Vec<(String, String)> {
    let db = db(server);
    let mut q = db.prepare("SELECT id, sealed FROM reins_send_holds").unwrap();
    q.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(Result::unwrap).collect()
}

/// The agent sends an email and the owner approves it: the agent hears it is held. The hold id and the ticket.
async fn held_ticket(w: &World) -> (String, String) {
    let args = json!({"to": [TO], "subject": SUBJECT, "body": BODY});
    let call = w.ai.tool("gmail_send", &args);
    let owner = async {
        let item = w.phone.wait_for_item(Duration::from_secs(20)).await;
        w.phone
            .core
            .approve(
                item.id,
                reins_core::ApprovalChoice {
                    selected_message_ids: vec![],
                    standing: None,
                },
            )
            .await
            .unwrap();
    };
    let (result, ()) = tokio::join!(call, owner);
    assert_eq!(result["isError"], false, "{result}");
    let held = &result["structuredContent"];
    assert_eq!(held["status"], "held", "{result}");
    assert!(held["note"].as_str().unwrap().contains(&format!("in {HOLD_SECS} s unless undone")), "{held}");
    assert!(held["ticket"].as_str().unwrap().starts_with("rt_"));
    assert_eq!(w.phone.sent_emails().await.len(), 0, "nothing goes during the hold");
    (held["hold"].as_str().unwrap().to_owned(), held["ticket"].as_str().unwrap().to_owned())
}

async fn held(w: &World) -> String {
    held_ticket(w).await.0
}

async fn send_rows(phone: &Phone) -> Vec<(String, Option<HoldState>)> {
    let mut rows: Vec<_> = phone
        .core
        .activity(50)
        .await
        .unwrap()
        .into_iter()
        .filter(|e| e.action == "send")
        .map(|e| (e.outcome, e.hold.map(|h| h.state)))
        .collect();
    rows.reverse();
    rows
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn while_held_the_server_keeps_only_an_opaque_ciphertext() {
    let w = world().await;
    let id = held(&w).await;
    let rows = kept(&w.server);
    assert_eq!(rows.len(), 1);
    let (kept_id, sealed) = &rows[0];
    assert_eq!(kept_id, &id);
    for clear in [TO, SUBJECT, BODY, "gmail_send"] {
        assert!(!sealed.contains(clear), "{clear} is on the server in clear");
    }
    let bytes = data_encoding::BASE64URL_NOPAD.decode(sealed.as_bytes()).expect("base64url");
    for clear in [TO, SUBJECT, BODY] {
        assert!(!String::from_utf8_lossy(&bytes).contains(clear), "{clear} is on the server in clear");
    }
    // Nothing else of the email is kept either: the row is ids, times and the ciphertext.
    let db = db(&w.server);
    let row: String = db
        .query_row(
            "SELECT id || user_uuid || device_uuid || connection_uuid || request_id || release_at || created_at \
             FROM reins_send_holds",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!row.contains(TO) && !row.contains(SUBJECT));
    assert_eq!(
        send_rows(&w.phone).await,
        [("held".to_owned(), Some(HoldState::Waiting))],
        "Activity: Sending in 2 s · Undo"
    );
    assert!(w.phone.notes.pending.lock().unwrap().iter().all(|i| !i.title.contains("Send")), "a hold is no ask");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn send_undo_nothing_is_sent() {
    let w = world().await;
    let id = held(&w).await;
    w.phone.core.undo_send(id.clone()).await.unwrap();
    assert!(kept(&w.server).is_empty(), "Undo deleted the ciphertext");
    // Past the release, every way of waking the phone sends nothing.
    tokio::time::sleep(Duration::from_secs(u64::from(HOLD_SECS) + 1)).await;
    w.phone.core.handle_push("hold".to_owned(), id.clone()).await.unwrap();
    w.phone.core.sync(0).await.unwrap();
    assert_eq!(w.phone.core.release_holds().await.unwrap().sent, 0);
    assert_eq!(w.phone.sent_emails().await.len(), 0);
    assert_eq!(send_rows(&w.phone).await, [("held".to_owned(), None), ("undone".to_owned(), None)]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn send_release_sent_exactly_once() {
    let w = world().await;
    let id = held(&w).await;
    // The server wakes the phone at the release time, silently.
    assert!(w.pushes.hold(&id, Duration::from_secs(u64::from(HOLD_SECS) + 8)).await, "the server woke the phone");
    // Everything that could release it at once: the push, a sync and the app's own timer.
    let (a, b, c) = tokio::join!(
        w.phone.core.handle_push("hold".to_owned(), id.clone()),
        w.phone.core.sync(0),
        w.phone.core.release_holds(),
    );
    a.unwrap();
    b.unwrap();
    c.unwrap();
    w.phone.core.release_holds().await.unwrap();
    let sent = w.phone.sent_emails().await;
    assert_eq!(sent.len(), 1, "sent exactly once");
    let raw: Value = serde_json::from_slice(&sent[0].body).unwrap();
    let mime = data_encoding::BASE64URL
        .decode(raw["raw"].as_str().unwrap().as_bytes())
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .unwrap_or_default();
    assert!(mime.contains(TO), "the email as approved went out");
    assert!(kept(&w.server).is_empty(), "the claim deleted the ciphertext");
    assert_eq!(send_rows(&w.phone).await, [("held".to_owned(), None), ("sent".to_owned(), None)]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_offline_phone_that_wakes_late_asks_again() {
    let w = world().await;
    let id = held(&w).await;
    // The phone was offline long past the release (the server's clock says 20 minutes).
    db(&w.server)
        .execute("UPDATE reins_send_holds SET release_at = release_at - 1200, woken = 1 WHERE id = ?1", [&id])
        .unwrap();
    tokio::time::sleep(Duration::from_secs(u64::from(HOLD_SECS) + 1)).await;
    let released = w.phone.core.release_holds().await.unwrap();
    assert_eq!((released.sent, released.late), (0, 1));
    assert_eq!(w.phone.sent_emails().await.len(), 0, "never sent late");
    assert!(kept(&w.server).is_empty(), "claimed: the server has nothing left to release");
    assert_eq!(
        send_rows(&w.phone).await,
        [("held".to_owned(), None), ("not_sent".to_owned(), Some(HoldState::Late))],
        "Activity: Still send?"
    );
    // The owner says send: it goes now, once.
    w.phone.core.send_late(id.clone()).await.unwrap();
    assert!(w.phone.core.send_late(id).await.is_err());
    assert_eq!(w.phone.sent_emails().await.len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_agent_and_the_desktop_can_undo_but_another_account_cannot() {
    let w = world().await;

    // The agent withdraws its own held send.
    let (id, ticket) = held_ticket(&w).await;
    let cancelled = w.ai.tool("reins_cancel", &json!({"ticket": ticket})).await;
    assert_eq!(cancelled["structuredContent"]["status"], "cancelled", "{cancelled}");
    assert!(kept(&w.server).is_empty());
    assert!(w.pushes.hold(&id, Duration::from_secs(5)).await, "the phone hears of it");
    w.phone.core.handle_push("hold".to_owned(), id).await.unwrap();

    // The desktop's Undo (as the connection that asked).
    let id = held(&w).await;
    reins_e2e::init_tls();
    let http = reqwest::Client::new();
    // Another account's agent can't see or touch it.
    let (_stranger_phone, stranger) = pair(&w.server, "other@example.com").await;
    let undo = |token: &str| {
        http.delete(w.server.url(&format!("/reins/desktop/holds/{id}"))).bearer_auth(token.to_owned()).send()
    };
    assert_eq!(undo(&stranger.access).await.unwrap().status(), 404, "another account's hold is not there for it");
    assert_eq!(kept(&w.server).len(), 1);
    assert_eq!(undo(&w.ai.access).await.unwrap().status(), 200);
    assert_eq!(undo(&w.ai.access).await.unwrap().status(), 404, "once");
    assert!(kept(&w.server).is_empty());
    tokio::time::sleep(Duration::from_secs(u64::from(HOLD_SECS) + 1)).await;
    w.phone.core.release_holds().await.unwrap();
    assert_eq!(w.phone.sent_emails().await.len(), 0, "nothing undone was sent");
    let rows = send_rows(&w.phone).await;
    assert_eq!(rows.iter().filter(|(o, _)| o == "undone").count(), 2, "{rows:?}");
}
