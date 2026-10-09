//! How fast a request travels, hop by hop: the AI calls a tool, the server queues it and pushes, the phone fetches and
//! shows it, the user approves, the AI has the answer. The real server binary and phone core, a loopback "push"
//! (`REINS_TEST_PUSH_URL`) that wakes the phone like FCM would, and a fake Gmail. Prints p50/p95 per hop
//! (`cargo test -p reins-e2e --test round_trip -- --nocapture`); the asserts are loose bounds that catch a hop
//! regressing to "seconds".

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use reins_e2e::{AiClient, Phone, Server};
use serde_json::{Value, json};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const EMAIL: &str = "user@example.com";
const ROUNDS: usize = 20;

/// The loopback push service: records each push with the moment it arrived.
#[derive(Clone, Default)]
struct Pushes(Arc<Mutex<Vec<(Instant, String, String)>>>);

impl Respond for Pushes {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let body: Value = serde_json::from_slice(&request.body).unwrap_or_default();
        let kind = body["t"].as_str().unwrap_or_default().to_owned();
        let id = body["id"].as_str().unwrap_or_default().to_owned();
        self.0.lock().unwrap().push((Instant::now(), kind, id));
        ResponseTemplate::new(200)
    }
}

impl Pushes {
    /// The next push of `kind` after `since`, waiting up to 10 s.
    async fn next(&self, kind: &str, since: Instant) -> (Instant, String) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some((at, _, id)) = self.0.lock().unwrap().iter().find(|(at, k, _)| *at >= since && k == kind) {
                return (*at, id.clone());
            }
            assert!(Instant::now() < deadline, "no {kind} push arrived");
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    }
}

async fn push_service() -> (MockServer, Pushes) {
    let server = MockServer::start().await;
    let pushes = Pushes::default();
    Mock::given(wiremock::matchers::method("POST")).respond_with(pushes.clone()).mount(&server).await;
    (server, pushes)
}

/// The server with the loopback push, the phone signed in, and an AI connected (the pairing woken by push too).
async fn connected(relay_wait: u64, offline: u64, pushes: &MockServer) -> (Server, Phone, AiClient) {
    let env = [("REINS_TEST_PUSH_URL".to_owned(), pushes.uri())];
    let server = Server::start_with_env(relay_wait, offline, &env).await;
    server.register(EMAIL).await;
    let phone = Phone::sign_in(&server.base, EMAIL).await;
    let mut ai = AiClient::new(&server.base);
    ai.register_client().await;
    let wait_url = ai.start_authorization(EMAIL).await;
    let code = ai.browser_code(&wait_url).await;
    let item = phone.wait_for_item(Duration::from_secs(20)).await;
    phone.core.answer_pairing(item.id, true, Some(code), Some("Claude".to_owned())).await.unwrap();
    ai.finish_authorization(&wait_url).await;
    (server, phone, ai)
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// `(p50, p95)` in milliseconds.
fn percentiles(samples: &mut [Duration]) -> (f64, f64) {
    samples.sort_unstable();
    let at = |q: f64| {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss,
            reason = "a small index"
        )]
        let i = ((samples.len() as f64 - 1.0) * q).round() as usize;
        ms(samples[i])
    };
    (at(0.5), at(0.95))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_request_reaches_the_phone_and_the_answer_reaches_the_ai_in_milliseconds() {
    let (push_service, push_log) = push_service().await;
    let (server, phone, ai) = connected(20, 8, &push_service).await;
    phone.stock_gmail(&[("m1", "Bank <alerts@bank.com>"), ("m2", "Friend <pal@example.org>")]).await;
    let (mut to_push, mut to_shown, mut to_answer, mut total) = (vec![], vec![], vec![], vec![]);
    let query = json!({"query": "from:bank"});
    for _ in 0..ROUNDS {
        let asked = Instant::now();
        let call = ai.tool("gmail_search", &query);
        let phone_side = async {
            // The push wakes the phone, which fetches the request, looks it up in Gmail and shows it.
            let (pushed, id) = push_log.next("req", asked).await;
            phone.core.handle_push("req".to_owned(), id.clone()).await.unwrap();
            let shown = Instant::now();
            assert!(phone.core.pending().await.unwrap().iter().any(|i| i.id == id), "parked and shown");
            let approved = Instant::now();
            phone.core.approve_quick(id).await.unwrap();
            (pushed, shown, approved)
        };
        let (result, (pushed, shown, approved)) = tokio::join!(call, phone_side);
        let answered = Instant::now();
        assert_eq!(result["isError"], false, "{result}");
        to_push.push(pushed - asked);
        to_shown.push(shown - pushed);
        to_answer.push(answered - approved);
        total.push((answered - asked).saturating_sub(approved - shown));
    }
    let rows = [
        ("AI call -> push sent", percentiles(&mut to_push)),
        ("push -> shown on the phone", percentiles(&mut to_shown)),
        ("approve -> AI has the answer", percentiles(&mut to_answer)),
        ("total, without the user", percentiles(&mut total)),
    ];
    println!("\nround trip over loopback, {ROUNDS} rounds (p50 / p95 ms):");
    for (hop, (p50, p95)) in rows {
        println!("  {hop:<32} {p50:>8.1} {p95:>8.1}");
    }
    for (hop, (_, p95)) in rows {
        assert!(p95 < 1_500.0, "{hop} took {p95} ms at p95");
    }
    assert!(!server.log().contains("panicked"), "server log: {}", server.log());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_phone_that_cannot_be_woken_is_reported_at_once_with_when_it_was_last_seen() {
    // Pushes go nowhere (nothing listens on the port): the push fails, as it does for an uninstalled app.
    let dead = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = dead.local_addr().unwrap().port();
    drop(dead);
    let env = [("REINS_TEST_PUSH_URL".to_owned(), format!("http://127.0.0.1:{port}/push"))];
    let server = Server::start_with_env(20, 8, &env).await;
    server.register(EMAIL).await;
    let phone = Phone::sign_in(&server.base, EMAIL).await;
    let mut ai = AiClient::new(&server.base);
    ai.register_client().await;
    let wait_url = ai.start_authorization(EMAIL).await;
    let code = ai.browser_code(&wait_url).await;
    let item = phone.wait_for_item(Duration::from_secs(20)).await;
    phone.core.answer_pairing(item.id, true, Some(code), Some("Claude".to_owned())).await.unwrap();
    ai.finish_authorization(&wait_url).await;
    // The app is closed now: nothing heard from the phone for longer than an open app ever waits between polls.
    tokio::time::sleep(Duration::from_secs(11)).await;

    let asked = Instant::now();
    let result = ai.tool("gmail_search", &json!({"query": "anything"})).await;
    let took = asked.elapsed();
    let text = result["content"][0]["text"].as_str().unwrap();
    println!("\nunreachable phone: the AI hears it after {:.0} ms (the offline threshold is 8 s)", ms(took));
    assert!(text.contains("your approval device is offline (last seen"), "{text}");
    assert!(text.contains("waits there for up to 10 minutes"), "{text}");
    assert!(took < Duration::from_secs(3), "reported at once, not after the threshold: {took:?}");

    // The user opens the app later: the request is still there, once. The AI's retry is that same request, and the
    // answer reaches it.
    let id = text.split("request_id=").nth(1).unwrap().trim_end_matches('.').to_owned();
    phone.stock_gmail(&[("m1", "Bank <alerts@bank.com>")]).await;
    let query = json!({"query": "anything"});
    let retry = async {
        tokio::time::sleep(Duration::from_millis(500)).await;
        ai.tool("gmail_search", &query).await
    };
    let app = async {
        let item = phone.wait_for_item(Duration::from_secs(20)).await;
        assert_eq!(item.id, id, "the request waited on the server");
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert_eq!(phone.core.pending().await.unwrap().len(), 1, "the retry added nothing");
        phone.core.deny(item.id).await.unwrap();
    };
    let (again, ()) = tokio::join!(retry, app);
    assert_eq!(again["content"][0]["text"], "Denied by the user on their Reins device.", "{again}");
    assert!(!server.log().contains("panicked"), "server log: {}", server.log());
}
