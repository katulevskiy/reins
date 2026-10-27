//! Circles end to end (Decisions v2, 04): two real accounts on one real server, two phone cores and an AI connected to
//! the first. Invitations with a safety code, co-sign Instant and Deferred, revocation, and phones that are offline
//! while the other side decides.

use std::time::Duration;

use reins_core::{
    ApprovalChoice, CircleDraft, CircleInviteProgress, CircleJoinStage, CircleLinkState, CircleReviewState,
    CircleReviewView, CircleRole, CircleScopePreset, PendingItem,
};
use reins_e2e::{AiClient, PASSWORD, Phone, Server};
use serde_json::{Value, json};

const DANA: &str = "dana@example.com";
const ALEX: &str = "alex@example.com";

struct World {
    server: Server,
    dana: Phone,
    alex: Phone,
    ai: AiClient,
}

/// Two accounts with phone roles; an AI connected to Dana's.
async fn world() -> World {
    let server = Server::start(6, 3).await;
    server.register(DANA).await;
    server.register(ALEX).await;
    let dana = with_roster(&server, DANA).await;
    let alex = with_roster(&server, ALEX).await;
    let mut ai = AiClient::new(&server.base);
    ai.register_client().await;
    let wait_url = ai.start_authorization(DANA).await;
    let code = ai.browser_code(&wait_url).await;
    let item = dana.wait_for_item(Duration::from_secs(20)).await;
    dana.core.answer_pairing(item.id, true, Some(code), Some("Claude".to_owned()), true).await.unwrap();
    ai.finish_authorization(&wait_url).await;
    dana.accept_sends().await;
    World {
        server,
        dana,
        alex,
        ai,
    }
}

/// A phone signed in to `email` that is its account's primary (phone roles on). Gmail is connected the pre-upgrade
/// way (no managed permission defaults): on an account with phone roles a managed send is refused before any Circle
/// looks at it, which these tests don't exercise.
async fn with_roster(server: &Server, email: &str) -> Phone {
    let phone = Phone::sign_in_with_existing_gmail(&server.base, email).await;
    phone.core.roster_create("Phone".to_owned(), Some(PASSWORD.to_owned())).await.expect("roster");
    phone
}

fn cosign(deferred: bool) -> CircleDraft {
    CircleDraft {
        role: CircleRole::Cosign,
        preset: CircleScopePreset::Everything,
        deferred,
        window_secs: None,
        fall_back_to_me: false,
        full_cards: true,
        notice_days: None,
    }
}

fn none() -> ApprovalChoice {
    ApprovalChoice {
        selected_message_ids: vec![],
        standing: None,
    }
}

/// Dana invites Alex; both phones show the same code; Alex accepts, Dana confirms. Returns the link id.
async fn invite(w: &World, draft: CircleDraft) -> String {
    let start = w.dana.core.circle_invite_create(draft).await.expect("invite");
    assert!(start.link.starts_with("reins-circle://"), "{}", start.link);
    let opened = w.alex.core.circle_join_open(start.link.clone()).await.expect("open the link");
    assert_eq!(opened.inviter, DANA);
    assert_eq!(opened.stage, CircleJoinStage::Opened);
    // The owner's phone reveals once Alex answered (here when its screen polls).
    let owner_code = poll(async || match w.dana.core.circle_invite_poll(start.id.clone()).await.expect("poll") {
        CircleInviteProgress::Compare {
            code,
            helper,
        } => {
            assert_eq!(helper, ALEX);
            Some(code)
        }
        _ => None,
    })
    .await;
    let view = poll(async || {
        let v = w.alex.core.circle_join_poll(opened.id.clone()).await.expect("join poll");
        (v.stage == CircleJoinStage::Compare).then_some(v)
    })
    .await;
    assert_eq!(view.code.as_deref(), Some(owner_code.as_str()), "both phones show the same six digits");
    assert_eq!(view.scope_line, "Everything");
    w.alex.core.circle_join_accept(opened.id.clone()).await.expect("accept");
    poll(async || {
        matches!(
            w.dana.core.circle_invite_poll(start.id.clone()).await.expect("poll"),
            CircleInviteProgress::Accepted { .. }
        )
        .then_some(())
    })
    .await;
    let person = w.dana.core.circle_invite_confirm(start.id.clone()).await.expect("confirm");
    assert!(person.mine);
    assert_eq!(person.peer, ALEX);
    assert_eq!(person.state, CircleLinkState::Active);
    let theirs = poll(async || {
        let people = w.alex.core.circle_people(true).await.expect("people");
        people.into_iter().find(|p| p.link_id == person.link_id && p.state == CircleLinkState::Active)
    })
    .await;
    assert!(!theirs.mine);
    assert_eq!(theirs.peer, DANA);
    assert_eq!(theirs.role, CircleRole::Cosign);
    person.link_id
}

/// Tries `f` every half second for up to 30 s.
async fn poll<T>(f: impl AsyncFn() -> Option<T>) -> T {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(v) = f().await {
            return v;
        }
        assert!(tokio::time::Instant::now() < deadline, "timed out");
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// Alex's phone (open) until a review waits.
async fn review_for(alex: &Phone) -> CircleReviewView {
    poll(async || {
        alex.core.sync(1).await.expect("sync");
        alex.core
            .circle_reviews(true)
            .await
            .expect("reviews")
            .into_iter()
            .find(|r| r.state == CircleReviewState::Waiting)
    })
    .await
}

/// Dana's phone (open) until the request shows its Circle line.
async fn circled_item(dana: &Phone) -> PendingItem {
    poll(async || {
        let items = dana.core.sync(1).await.expect("sync");
        items.into_iter().find(|i| i.circle.is_some())
    })
    .await
}

async fn sent(dana: &Phone) -> usize {
    dana.sent_emails().await.len()
}

fn send_args() -> Value {
    json!({"to": ["ann@acme.com"], "subject": "Quarterly numbers", "body": "Attached, as discussed."})
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn invite_shows_one_code_and_activates_on_both_phones() {
    let w = world().await;
    let link = invite(&w, cosign(false)).await;
    // A second attempt by someone else to answer the same link would need the secret; the link is spent anyway.
    let people = w.dana.core.circle_people(true).await.unwrap();
    assert_eq!(people.len(), 1);
    assert_eq!(people[0].link_id, link);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cosign_instant_needs_both_and_runs_once() {
    let w = world().await;
    invite(&w, cosign(false)).await;
    let args = send_args();
    let call = w.ai.tool("gmail_send", &args);
    let phones = async {
        let item = circled_item(&w.dana).await;
        let progress = item.circle.clone().unwrap();
        assert_eq!(progress.line, format!("Needs {ALEX} too"));
        assert!(progress.waiting_for_you);
        // Dana approves first: nothing runs yet.
        w.dana.core.approve(item.id.clone(), none()).await.unwrap();
        assert_eq!(sent(&w.dana).await, 0, "a co-signed request waits for the co-signer");
        let waiting = w.dana.core.pending().await.unwrap();
        let card = waiting.iter().find(|i| i.id == item.id).expect("still waiting");
        assert!(card.circle.as_ref().unwrap().you_approved);
        // Alex sees the card (no raw request), and approves.
        let review = review_for(&w.alex).await;
        assert_eq!(review.person, DANA);
        assert_eq!(review.agent, "Claude");
        w.alex.core.circle_answer(review.id.clone(), true, None).await.unwrap();
        // Dana's phone takes the answer and runs it.
        poll(async || {
            w.dana.core.sync(1).await.unwrap();
            (sent(&w.dana).await == 1).then_some(())
        })
        .await;
    };
    let (result, ()) = tokio::join!(call, phones);
    assert!(result.to_string().contains("sent") || result["isError"] == false, "{result}");
    // Once only, whatever syncs after.
    w.dana.core.sync(1).await.unwrap();
    assert_eq!(sent(&w.dana).await, 1);
    let receipts = w.alex.core.activity(20).await.unwrap();
    assert!(receipts.iter().any(|e| e.detail.contains(DANA)), "Alex's Activity keeps a receipt");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn either_deny_wins() {
    let w = world().await;
    invite(&w, cosign(false)).await;
    let args = send_args();
    let call = w.ai.tool("gmail_send", &args);
    let phones = async {
        let item = circled_item(&w.dana).await;
        w.dana.core.approve(item.id.clone(), none()).await.unwrap();
        let review = review_for(&w.alex).await;
        w.alex.core.circle_answer(review.id, false, Some("Not this week".to_owned())).await.unwrap();
        poll(async || {
            let items = w.dana.core.sync(1).await.unwrap();
            items.iter().all(|i| i.id != item.id).then_some(())
        })
        .await;
    };
    let (result, ()) = tokio::join!(call, phones);
    assert_eq!(sent(&w.dana).await, 0);
    assert!(result.to_string().contains("Not this week"), "the agent hears the co-signer's note: {result}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cosign_deferred_is_a_ticket_and_offline_phones_catch_up() {
    let w = world().await;
    invite(&w, cosign(true)).await;
    // The agent doesn't wait: a Deferred arrangement makes a ticket at once.
    let args = send_args();
    let call = w.ai.tool("gmail_send", &args);
    let first = async {
        let item = circled_item(&w.dana).await;
        assert!(item.circle.as_ref().unwrap().deferred);
        item
    };
    let (parked, item) = tokio::join!(call, first);
    assert_eq!(parked["structuredContent"]["status"], "pending_review", "{parked}");
    let ticket = parked["structuredContent"]["ticket"].as_str().unwrap().to_owned();
    // Dana approves, then her phone goes quiet (no sync).
    w.dana.core.approve(item.id.clone(), none()).await.unwrap();
    assert_eq!(sent(&w.dana).await, 0);
    // Alex was offline all along; his review waited on the server. He approves now.
    let review = review_for(&w.alex).await;
    w.alex.core.circle_answer(review.id, true, None).await.unwrap();
    assert_eq!(sent(&w.dana).await, 0, "nothing runs on the server's word");
    // Dana's phone wakes: it verifies Alex's answer and runs it.
    poll(async || {
        w.dana.core.sync(1).await.unwrap();
        (sent(&w.dana).await == 1).then_some(())
    })
    .await;
    let done = w.ai.tool("reins_get_result", &json!({"ticket": ticket})).await;
    assert_eq!(done["structuredContent"]["status"], "done", "{done}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn revoke_stops_late_answers() {
    let w = world().await;
    let link = invite(&w, cosign(true)).await;
    let args = send_args();
    let call = w.ai.tool("gmail_send", &args);
    let first = async { circled_item(&w.dana).await };
    let (_parked, item) = tokio::join!(call, first);
    // Alex answers; before Dana's phone takes it, Dana ends the arrangement.
    let review = review_for(&w.alex).await;
    w.alex.core.circle_answer(review.id, true, None).await.unwrap();
    w.dana.core.circle_end(link.clone()).await.unwrap();
    w.dana.core.sync(1).await.unwrap();
    assert_eq!(sent(&w.dana).await, 0, "a late answer from an ended arrangement never runs anything");
    let card = w.dana.core.pending().await.unwrap().into_iter().find(|i| i.id == item.id).expect("still waiting");
    assert!(card.circle.is_none(), "it waits for Dana alone now");
    // Alex's side ends too.
    let theirs = poll(async || {
        w.alex.core.sync(1).await.unwrap();
        w.alex.core.circle_people(true).await.unwrap().into_iter().find(|p| p.state == CircleLinkState::Ended)
    })
    .await;
    assert_eq!(theirs.link_id, link);
    // Dana decides alone.
    w.dana.core.approve(item.id, none()).await.unwrap();
    assert_eq!(sent(&w.dana).await, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_lying_server_changes_nothing() {
    let w = world().await;
    let mut draft = cosign(true);
    // Summary cards: the helper sees the effects and the sentence, not who it goes to.
    draft.full_cards = false;
    let link = invite(&w, draft).await;
    let db = || rusqlite::Connection::open(w.server.database_path()).unwrap();

    // A first request, co-signed for real.
    let args = send_args();
    let (_, first) = tokio::join!(w.ai.tool("gmail_send", &args), circled_item(&w.dana));
    w.dana.core.approve(first.id.clone(), none()).await.unwrap();
    let review = review_for(&w.alex).await;
    assert!(review.detail.is_none() && review.subtitle.is_empty(), "a summary card carries no detail: {review:?}");
    w.alex.core.circle_answer(review.id.clone(), true, None).await.unwrap();
    poll(async || {
        w.dana.core.sync(1).await.unwrap();
        (sent(&w.dana).await == 1).then_some(())
    })
    .await;
    let old_answer: String = db()
        .query_row("SELECT sealed_answer FROM reins_circle_packets WHERE id = ?1", [&review.id], |r| r.get(0))
        .unwrap();

    // A second request: Dana approves; the server replays Alex's old answer as the answer to this one.
    let second_args = json!({"to": ["ann@acme.com"], "subject": "Another", "body": "Second"});
    let (_, second) = tokio::join!(
        w.ai.tool("gmail_send", &second_args),
        poll(async || {
            let items = w.dana.core.sync(1).await.unwrap();
            items.into_iter().find(|i| i.circle.is_some() && i.id != first.id)
        })
    );
    w.dana.core.approve(second.id.clone(), none()).await.unwrap();
    let packet: String = poll(async || {
        db().query_row("SELECT id FROM reins_circle_packets WHERE state = 'waiting' AND kind = 'review'", [], |r| {
            r.get(0)
        })
        .ok()
    })
    .await;
    db().execute(
        "UPDATE reins_circle_packets SET state = 'answered', sealed_answer = ?1, owner_seen = 0 WHERE id = ?2",
        rusqlite::params![old_answer, packet],
    )
    .unwrap();
    // It also claims the arrangement ended (an unsigned change) and plants an arrangement Dana never made.
    db().execute(
        "UPDATE reins_circle_links SET last_change = ?1 WHERE id = ?2",
        rusqlite::params![
            json!({"body": json!({"v": 1, "link": link, "arrangement": "0".repeat(64), "seq": 9, "state": "ended", "by": "helper", "at": 1}).to_string(), "sig": "A".repeat(86)}).to_string(),
            link
        ],
    )
    .unwrap();
    let (dana_id, alex_id): (String, String) = {
        let c = db();
        let id = |email: &str| c.query_row("SELECT uuid FROM users WHERE email = ?1", [email], |r| r.get(0)).unwrap();
        (id(DANA), id(ALEX))
    };
    db().execute(
        "INSERT INTO reins_circle_links VALUES (?1, ?2, ?3, 'delegate', 'active', 'AAAA', 'AAAA', NULL, 1, 1, NULL)",
        rusqlite::params!["f".repeat(32), dana_id, alex_id],
    )
    .unwrap();
    for _ in 0..3 {
        w.dana.core.sync(1).await.unwrap();
    }
    assert_eq!(sent(&w.dana).await, 1, "a replayed answer never runs the second request");
    let card = w.dana.core.pending().await.unwrap().into_iter().find(|i| i.id == second.id).expect("still waiting");
    assert_eq!(card.circle.unwrap().line, format!("Waiting for {ALEX}"));
    let people = w.dana.core.circle_people(true).await.unwrap();
    assert_eq!(people.len(), 1, "a planted arrangement is ignored: {people:?}");
    assert_eq!(people[0].state, CircleLinkState::Active, "an unsigned end is ignored");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn nobody_shares_control_with_themself() {
    let w = world().await;
    let start = w.dana.core.circle_invite_create(cosign(false)).await.unwrap();
    let err = w.dana.core.circle_join_open(start.link).await.unwrap_err();
    assert!(err.to_string().contains("own invitation"), "{err}");
}
