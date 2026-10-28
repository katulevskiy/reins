//! Guardianship end to end (Decisions v2 P7, 04 §4, 08 R7): a ward and a guardian on one real server, two phone cores,
//! an AI connected to the ward. Notice periods, minors and consent, the ward's view, and what the guardian is told
//! (checked, or "Reported by the server").
//!
//! The exact notice boundary (in force at `effective_at - 1`, not at `effective_at`) is pinned by pure tests in
//! `reins_proto::circle` and `reins_core::circles::guardian`; here a 3-day notice is shown still binding, and a 0-day
//! one ending at once.

use std::{sync::Arc, time::Duration};

use reins_core::{
    AgeSignalInput, ApprovalChoice, CircleDraft, CircleInviteProgress, CircleJoinStage, CircleLinkState,
    CircleReviewState, CircleReviewView, CircleRole, CircleScopePreset, PendingItem,
};
use reins_e2e::{AiClient, PASSWORD, Phone, Server};
use serde_json::{Value, json};

/// The owner check the apps run (biometric): `true` confirms, `false` is a cancel.
struct Owner(bool);
#[async_trait::async_trait]
impl reins_core::OwnerCheck for Owner {
    async fn confirm(&self, _reason: String) -> Result<bool, reins_core::ForeignError> {
        Ok(self.0)
    }
}

fn owner(yes: bool) -> Arc<dyn reins_core::OwnerCheck> {
    Arc::new(Owner(yes))
}

const WARD: &str = "sam@example.com";
const GUARDIAN: &str = "mom@example.com";

struct World {
    server: Server,
    sam: Phone,
    mom: Phone,
    ai: AiClient,
}

async fn world() -> World {
    let server = Server::start(6, 3).await;
    server.register(WARD).await;
    server.register(GUARDIAN).await;
    let sam = with_roster(&server, WARD).await;
    let mom = with_roster(&server, GUARDIAN).await;
    let mut ai = AiClient::new(&server.base);
    ai.register_client().await;
    let wait_url = ai.start_authorization(WARD).await;
    let code = ai.browser_code(&wait_url).await;
    let item = sam.wait_for_item(Duration::from_secs(20)).await;
    sam.core.answer_pairing(item.id, true, Some(code), Some("Claude".to_owned()), true).await.unwrap();
    ai.finish_authorization(&wait_url).await;
    sam.accept_sends().await;
    World {
        server,
        sam,
        mom,
        ai,
    }
}

async fn with_roster(server: &Server, email: &str) -> Phone {
    let phone = Phone::sign_in_with_existing_gmail(&server.base, email).await;
    phone.core.roster_create("Phone".to_owned(), Some(PASSWORD.to_owned())).await.expect("roster");
    phone
}

fn guardian(notice_days: Option<u8>, deferred: bool) -> CircleDraft {
    CircleDraft {
        role: CircleRole::Guardian,
        preset: CircleScopePreset::Everything,
        deferred,
        window_secs: None,
        fall_back_to_me: false,
        full_cards: false,
        notice_days,
    }
}

fn none() -> ApprovalChoice {
    ApprovalChoice {
        selected_message_ids: vec![],
        standing: None,
    }
}

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

/// Sam (the ward) invites Mom as guardian; the notice is chosen by Sam and shown to Mom before the code.
async fn invite(w: &World, draft: CircleDraft) -> String {
    let days = draft.notice_days;
    let start = w.sam.core.circle_invite_create(draft).await.expect("invite");
    let opened = w.mom.core.circle_join_open(start.link.clone()).await.expect("open");
    poll(async || match w.sam.core.circle_invite_poll(start.id.clone()).await.expect("poll") {
        CircleInviteProgress::Compare {
            code,
            ..
        } => Some(code),
        _ => None,
    })
    .await;
    let view = poll(async || {
        let v = w.mom.core.circle_join_poll(opened.id.clone()).await.expect("join poll");
        (v.stage == CircleJoinStage::Compare).then_some(v)
    })
    .await;
    assert_eq!(view.role, CircleRole::Guardian);
    assert_eq!(view.notice_days, days, "the guardian sees the notice the ward chose");
    w.mom.core.circle_join_accept(opened.id.clone()).await.expect("accept");
    poll(async || {
        matches!(
            w.sam.core.circle_invite_poll(start.id.clone()).await.expect("poll"),
            CircleInviteProgress::Accepted { .. }
        )
        .then_some(())
    })
    .await;
    let person = w.sam.core.circle_invite_confirm(start.id.clone()).await.expect("confirm");
    poll(async || {
        let people = w.mom.core.circle_people(true).await.expect("people");
        people.into_iter().find(|p| p.link_id == person.link_id && p.state == CircleLinkState::Active)
    })
    .await;
    person.link_id
}

async fn review_for(mom: &Phone) -> CircleReviewView {
    poll(async || {
        mom.core.sync(1).await.expect("sync");
        mom.core
            .circle_reviews(true)
            .await
            .expect("reviews")
            .into_iter()
            .find(|r| r.state == CircleReviewState::Waiting)
    })
    .await
}

/// The next waiting request on Sam's phone, and whether the guardian is on it.
async fn next_item(sam: &Phone) -> PendingItem {
    poll(async || sam.core.sync(1).await.expect("sync").into_iter().next()).await
}

fn send_args(subject: &str) -> Value {
    json!({"to": ["ann@acme.com"], "subject": subject, "body": "Hello."})
}

async fn sent(sam: &Phone) -> usize {
    sam.sent_emails().await.len()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn guardian_end_to_end() {
    let w = world().await;
    let link = invite(&w, guardian(Some(3), true)).await;
    // A request waits for the guardian (Deferred: a ticket), and runs once when both approved.
    let args_one = send_args("One");
    let (parked, item) = tokio::join!(w.ai.tool("gmail_send", &args_one), next_item(&w.sam));
    assert_eq!(parked["structuredContent"]["status"], "pending_review", "{parked}");
    assert!(item.circle.is_some(), "the guardian is on it");
    w.sam.core.approve(item.id.clone(), none()).await.unwrap();
    assert_eq!(sent(&w.sam).await, 0, "the guardian's review binds");
    let review = review_for(&w.mom).await;
    assert_eq!(review.role, CircleRole::Guardian);
    assert_eq!(review.detail, None, "a guardian sees summaries");
    w.mom.core.circle_answer(review.id, true, Some("Fine".to_owned())).await.unwrap();
    poll(async || {
        w.sam.core.sync(1).await.unwrap();
        (sent(&w.sam).await == 1).then_some(())
    })
    .await;
    // The ward sees the decision and the note.
    let view = w.sam.core.guardian_ward_view(link.clone()).await.unwrap();
    assert_eq!(view.notice_days, 3);
    assert_eq!(view.decisions.len(), 1);
    assert!(view.decisions[0].approved);
    assert_eq!(view.decisions[0].note.as_deref(), Some("Fine"));
    assert!(!view.digest, "the weekly summary is off until the ward turns it on");

    // A ward can't pause a guardianship.
    assert!(w.sam.core.circle_pause(link.clone(), true).await.is_err());
    // Ending starts the notice: the guardian is told, and its review still applies.
    w.sam.core.circle_end(link.clone()).await.unwrap();
    let mine = w.sam.core.circle_people(true).await.unwrap().into_iter().find(|p| p.link_id == link).unwrap();
    assert_eq!(mine.state, CircleLinkState::Active);
    assert!(mine.ending.is_some_and(|at| at > 2 * 86_400 + reins_now()), "{:?}", mine.ending);
    let told = poll(async || {
        w.mom.core.sync(1).await.unwrap();
        w.mom.core.guardian_notices(true).await.unwrap().into_iter().find(|n| n.link_id == link)
    })
    .await;
    assert_eq!(told.line, "Leaving after the notice");
    assert!(told.verified, "a signed change is checked by the guardian's phone");
    let args_two = send_args("Two");
    let (_sent, ()) = tokio::join!(w.ai.tool("gmail_send", &args_two), async {
        let item = next_item(&w.sam).await;
        assert!(item.circle.is_some(), "during the notice the guardian still reviews");
        w.sam.core.approve(item.id.clone(), none()).await.unwrap();
        let review = review_for(&w.mom).await;
        w.mom.core.circle_answer(review.id, false, Some("Not now".to_owned())).await.unwrap();
        poll(async || {
            let items = w.sam.core.sync(1).await.unwrap();
            items.iter().all(|i| i.id != item.id).then_some(())
        })
        .await;
    });
    assert_eq!(sent(&w.sam).await, 1, "the guardian's deny wins during the notice");

    // The ward deletes the account: the guardian is told before the rows go, as reported by the server.
    w.sam.core.delete_account(WARD.to_owned()).await.expect("deleted");
    let lines = poll(async || {
        w.mom.core.sync(1).await.ok();
        let notices = w.mom.core.guardian_notices(true).await.unwrap();
        notices.iter().any(|n| n.line == "Deleted the account").then_some(notices)
    })
    .await;
    let deleted = lines.iter().find(|n| n.line == "Deleted the account").unwrap();
    assert!(!deleted.verified, "nobody signed it: reported by the server");
    drop(w.server);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_minors_end_waits_for_consent() {
    let w = world().await;
    let link = invite(&w, guardian(Some(7), false)).await;
    // The guardian proposes; an adult's signal refuses; the ward's acceptance marks it.
    w.mom.core.guardian_propose_minor(link.clone()).await.unwrap();
    let view = poll(async || {
        w.sam.core.sync(1).await.unwrap();
        w.sam.core.circle_people(true).await.ok();
        let v = w.sam.core.guardian_ward_view(link.clone()).await.unwrap();
        v.minor_proposed.then_some(v)
    })
    .await;
    assert!(!view.minor, "a guardian alone marks nothing");
    assert!(w.sam.core.guardian_accept_minor(link.clone(), AgeSignalInput::Adult, owner(true)).await.is_err());
    // Without the owner's confirmation nothing is marked.
    assert!(w.sam.core.guardian_accept_minor(link.clone(), AgeSignalInput::Unavailable, owner(false)).await.is_err());
    assert!(!w.sam.core.guardian_ward_view(link.clone()).await.unwrap().minor);
    w.sam.core.guardian_accept_minor(link.clone(), AgeSignalInput::Unavailable, owner(true)).await.unwrap();
    assert!(w.sam.core.guardian_ward_view(link.clone()).await.unwrap().minor);
    // A minor's end waits; the guardian is asked.
    w.sam.core.circle_end(link.clone()).await.unwrap();
    let view = w.sam.core.guardian_ward_view(link.clone()).await.unwrap();
    assert!(view.needs_consent);
    assert_eq!(view.ending, None);
    let ask = poll(async || {
        w.mom.core.sync(1).await.unwrap();
        w.mom.core.guardian_notices(true).await.unwrap().into_iter().find(|n| n.needs_consent)
    })
    .await;
    assert_eq!(ask.line, "Asked to leave");
    // Still in force until then.
    let args_three = send_args("Three");
    let (_, ()) = tokio::join!(w.ai.tool("gmail_send", &args_three), async {
        let item = next_item(&w.sam).await;
        assert!(item.circle.is_some(), "a minor's request changes nothing until consent");
        w.sam.core.deny(item.id.clone()).await.unwrap();
    });
    // The guardian's consent needs the owner check, in the core: a cancel consents to nothing.
    assert!(w.mom.core.guardian_consent_end(link.clone(), owner(false)).await.is_err());
    assert!(w.sam.core.guardian_ward_view(link.clone()).await.unwrap().needs_consent);
    w.mom.core.guardian_consent_end(link.clone(), owner(true)).await.unwrap();
    poll(async || {
        w.sam.core.sync(1).await.unwrap();
        w.sam
            .core
            .circle_people(true)
            .await
            .unwrap()
            .into_iter()
            .find(|p| p.link_id == link && p.state == CircleLinkState::Ended)
    })
    .await;
    // Ended: the next request is the ward's alone.
    let args_four = send_args("Four");
    let (_, ()) = tokio::join!(w.ai.tool("gmail_send", &args_four), async {
        let item = next_item(&w.sam).await;
        assert!(item.circle.is_none(), "after consent the guardian is gone");
        w.sam.core.approve(item.id.clone(), none()).await.unwrap();
    });
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn no_notice_ends_at_once_and_only_the_ward_sets_the_digest() {
    let w = world().await;
    // A guardianship needs its notice chosen up front.
    assert!(w.sam.core.circle_invite_create(guardian(None, false)).await.is_err());
    let link = invite(&w, guardian(Some(0), false)).await;
    assert!(w.mom.core.guardian_set_digest(link.clone(), true).await.is_err(), "the digest is the ward's");
    w.sam.core.guardian_set_digest(link.clone(), true).await.unwrap();
    assert!(w.sam.core.guardian_ward_view(link.clone()).await.unwrap().digest);
    w.sam.core.circle_end(link.clone()).await.unwrap();
    let mine = w.sam.core.circle_people(false).await.unwrap().into_iter().find(|p| p.link_id == link).unwrap();
    assert_eq!(mine.state, CircleLinkState::Ended, "0 days: it ends at once");
    let args_five = send_args("Five");
    let (_, ()) = tokio::join!(w.ai.tool("gmail_send", &args_five), async {
        let item = next_item(&w.sam).await;
        assert!(item.circle.is_none());
        w.sam.core.approve(item.id.clone(), none()).await.unwrap();
    });
    poll(async || {
        w.mom.core.sync(1).await.unwrap();
        w.mom
            .core
            .circle_people(true)
            .await
            .unwrap()
            .into_iter()
            .find(|p| p.link_id == link && p.state == CircleLinkState::Ended)
    })
    .await;
}

fn reins_now() -> i64 {
    i64::try_from(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).expect("time").as_secs())
        .expect("i64")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_guardian_gone_from_the_server_never_suspends_and_the_ward_still_leaves() {
    let w = world().await;
    let link = invite(&w, guardian(Some(0), false)).await;
    // The guardian deletes the account: the link vanishes from the server.
    w.mom.core.delete_account(GUARDIAN.to_owned()).await.expect("deleted");
    for _ in 0..3 {
        w.sam.core.sync(1).await.unwrap();
        w.sam.core.circle_people(true).await.unwrap();
    }
    // Absence alone proves nothing: not suspended (which would end at once), still active.
    let mine = w.sam.core.circle_people(false).await.unwrap().into_iter().find(|p| p.link_id == link).unwrap();
    assert_eq!(mine.state, CircleLinkState::Active, "a missing link never suspends a ward's guardianship");
    // The ward still leaves: its end request is kept on its phone and takes the notice it chose (none here).
    w.sam.core.circle_end(link.clone()).await.expect("ended");
    let ended = w.sam.core.circle_people(false).await.unwrap().into_iter().find(|p| p.link_id == link).unwrap();
    assert_eq!(ended.state, CircleLinkState::Ended);
    let args_six = send_args("Six");
    let (_, ()) = tokio::join!(w.ai.tool("gmail_send", &args_six), async {
        let item = next_item(&w.sam).await;
        assert!(item.circle.is_none(), "no request waits on a guardianship that ended");
        w.sam.core.approve(item.id.clone(), none()).await.unwrap();
    });
    poll(async || {
        w.sam.core.sync(1).await.unwrap();
        (sent(&w.sam).await == 1).then_some(())
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_withheld_change_holds_the_later_ones() {
    let w = world().await;
    let link = invite(&w, guardian(Some(7), false)).await;
    // Two steps of the ward: the digest on (1), then the end request (2).
    w.sam.core.guardian_set_digest(link.clone(), true).await.unwrap();
    w.sam.core.circle_end(link.clone()).await.unwrap();
    // The server withholds step 1 from the guardian (and its notice).
    let db = rusqlite::Connection::open(w.server.database_path()).unwrap();
    db.execute("DELETE FROM reins_circle_changes WHERE link_id = ?1 AND seq = 1", [&link]).unwrap();
    db.execute("DELETE FROM reins_guardian_notices WHERE link_id = ?1", [&link]).unwrap();
    let notices = poll(async || {
        w.mom.core.sync(1).await.unwrap();
        let n = w.mom.core.guardian_notices(true).await.unwrap();
        n.iter().any(|n| n.line == "Waiting for earlier changes").then_some(n)
    })
    .await;
    assert!(
        !notices.iter().any(|n| n.line == "Leaving after the notice"),
        "step 2 waits until step 1 is shown: {notices:?}"
    );
    let theirs = w.mom.core.circle_people(false).await.unwrap().into_iter().find(|p| p.link_id == link).unwrap();
    assert_eq!(theirs.ending, None, "the end request isn't counted out of order");
}
