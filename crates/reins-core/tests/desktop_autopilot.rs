//! Autopilot from the desktop app: what the phone applies at once (only what makes Reins stricter), what it asks its
//! owner (anything looser, never decided by a mode: not Autopilot, not a bypass, not Lockdown), who decided an answer
//! (inside the answer), and the modes reported for the desktop apps.

mod common;

use common::desktop::{DESK, Desk, call, choice, desk_with, mount_reins};
use reins_core::{AutopilotMode, CoreConfig, CoreError};
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn desk() -> Desk {
    let server = MockServer::start().await;
    mount_reins(&server).await;
    Mock::given(method("PUT"))
        .and(path("/reins/api/modes"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    let desk = desk_with(server, "me@example.com", "pw", CoreConfig::default()).await;
    desk.pair().await;
    desk
}

fn mode(desk: &Desk, id: &str, args: &Value) -> Value {
    let mut args = args.clone();
    args["client_key"] = json!(desk.public());
    args["nonce"] = json!(format!("nonce-{id}"));
    call(id, DESK, "desktop", "autopilot_mode", &args)
}

fn ask(desk: &Desk, id: &str) -> Value {
    call(
        id,
        DESK,
        "desktop",
        "ask",
        &json!({"question": "Deploy now?", "client_key": desk.public(), "nonce": format!("nonce-{id}")}),
    )
}

async fn global(desk: &Desk) -> AutopilotMode {
    desk.core.autopilot_settings().await.unwrap().mode
}

/// The last modes report the phone sent the server.
async fn reported(desk: &Desk) -> Value {
    let r = desk.server.received_requests().await.unwrap();
    let put =
        r.iter().rev().find(|r| r.method.as_str() == "PUT" && r.url.path() == "/reins/api/modes").expect("reported");
    serde_json::from_slice(&put.body).unwrap()
}

#[tokio::test]
async fn what_only_makes_reins_stricter_applies_at_once() {
    let desk = desk().await;
    desk.core.set_autopilot_mode(None, Some(AutopilotMode::Auto), None).await.unwrap();
    desk.send(&[mode(&desk, "m1", &json!({"mode": "manual"}))]).await;
    assert!(desk.waiting().await.is_empty(), "back to Manual is never asked");
    assert_eq!(desk.data("m1").await["applied"], true);
    assert_eq!(global(&desk).await, AutopilotMode::Manual);
    desk.send(&[mode(&desk, "m2", &json!({"mode": "lockdown"}))]).await;
    assert!(desk.waiting().await.is_empty(), "Lockdown is the brake");
    assert_eq!(global(&desk).await, AutopilotMode::Lockdown);
    assert_eq!(reported(&desk).await["mode"], "lockdown", "the desktop apps hear of it");
}

#[tokio::test]
async fn anything_looser_is_asked_held_and_worded_by_the_phone() {
    let desk = desk().await;
    desk.send(&[mode(&desk, "m1", &json!({"mode": "auto"}))]).await;
    assert_eq!(desk.waiting().await, ["m1"], "Auto from Manual is asked");
    assert_eq!(global(&desk).await, AutopilotMode::Manual, "nothing changed yet");
    let view = desk.core.approval_view("m1".to_owned()).await.unwrap();
    let shown = view.ask.clone().expect("shown as a question");
    assert_eq!(shown.question, "Turn on Auto for every AI?");
    assert!(shown.destructive, "held to approve");
    assert!(view.quick.is_none() && view.no_standing, "never one tap, never remembered: {view:?}");
    assert!(matches!(desk.core.approve_quick("m1".to_owned()).await, Err(CoreError::Invalid { .. })));
    desk.core.approve("m1".to_owned(), choice(&[], None)).await.unwrap();
    assert_eq!(global(&desk).await, AutopilotMode::Auto, "applied once the owner approved");
    assert_eq!(desk.data("m1").await["applied"], true);

    // A bypass from the computer always ends, and is asked.
    desk.send(&[mode(&desk, "b1", &json!({"mode": "bypass"}))]).await;
    assert!(desk.waiting().await.is_empty());
    assert!(!desk.error("b1").await.is_empty(), "a bypass without an end is refused");
    desk.send(&[mode(&desk, "b2", &json!({"mode": "overnight"}))]).await;
    assert!(!desk.error("b2").await.is_empty(), "Overnight starts on the phone only");
}

#[tokio::test]
async fn no_mode_decides_a_change_of_autopilot() {
    let desk = desk().await;
    // A bypass approves everything else at once, never its own extension.
    desk.core.start_bypass(None, Some(30)).await.unwrap();
    desk.send(&[mode(&desk, "b1", &json!({"mode": "bypass", "minutes": 600}))]).await;
    assert_eq!(desk.waiting().await, ["b1"], "the bypass did not approve it");
    // Lockdown denies everything else at once, never the way out of it.
    desk.core.set_autopilot_mode(None, Some(AutopilotMode::Lockdown), None).await.unwrap();
    assert_eq!(desk.waiting().await, ["b1"], "Lockdown did not deny it either");
    desk.send(&[mode(&desk, "m1", &json!({"mode": "manual"}))]).await;
    let mut waiting = desk.waiting().await;
    waiting.sort();
    assert_eq!(waiting, ["b1", "m1"], "leaving Lockdown is asked");
    assert_eq!(global(&desk).await, AutopilotMode::Lockdown);
    // The owner says no: nothing changes.
    desk.core.deny("m1".to_owned()).await.unwrap();
    assert_eq!(global(&desk).await, AutopilotMode::Lockdown);
}

#[tokio::test]
async fn an_answer_says_which_mode_decided_it() {
    let desk = desk().await;
    desk.core.start_bypass(None, Some(30)).await.unwrap();
    desk.send(&[ask(&desk, "a1")]).await;
    let answer = desk.answer("a1").await.unwrap();
    assert_eq!(answer["outcome"], "result");
    assert_eq!(answer["decided"]["by"], "bypass", "{answer}");
    desk.core.set_autopilot_mode(None, Some(AutopilotMode::Lockdown), None).await.unwrap();
    desk.send(&[ask(&desk, "a2")]).await;
    let answer = desk.answer("a2").await.unwrap();
    assert_eq!((answer["outcome"].as_str(), answer["decided"]["by"].as_str()), (Some("denied"), Some("lockdown")));
    assert_eq!(answer["decided"]["correctable"], false, "nothing to learn from Lockdown");
    // The owner's own answers say nothing of the kind.
    desk.core.set_autopilot_mode(None, Some(AutopilotMode::Manual), None).await.unwrap();
    desk.send(&[ask(&desk, "a3")]).await;
    desk.core.approve("a3".to_owned(), choice(&[], None)).await.unwrap();
    assert!(desk.answer("a3").await.unwrap().get("decided").is_none());
}

#[tokio::test]
async fn a_correction_concerns_only_this_computers_decisions() {
    let desk = desk().await;
    let correct = |id: &str, request: &str, should: &str| {
        call(
            id,
            DESK,
            "desktop",
            "autopilot_correct",
            &json!({"request_id": request, "should_have": should, "client_key": desk.public(),
                    "nonce": format!("nonce-{id}")}),
        )
    };
    // Teaching it to deny is applied at once, but only to a decision of this computer's.
    desk.send(&[correct("c1", "someone-elses", "deny")]).await;
    assert!(desk.error("c1").await.contains("did not decide"), "nothing of another connection is found");
    // Teaching it to approve is asked first.
    desk.send(&[correct("c2", "r9", "approve")]).await;
    assert_eq!(desk.waiting().await, ["c2"]);
    let shown = desk.core.approval_view("c2".to_owned()).await.unwrap().ask.unwrap();
    assert!(shown.question.contains("approve") && shown.destructive, "{shown:?}");
}

#[tokio::test]
async fn the_modes_are_reported_for_the_desktop_apps() {
    let desk = desk().await;
    desk.core.start_bypass(None, Some(30)).await.unwrap();
    let report = reported(&desk).await;
    assert_eq!((report["mode"].as_str(), report["base"].as_str()), (Some("bypass"), Some("manual")));
    let (since, until) = (report["span"]["since"].as_i64().unwrap(), report["span"]["until"].as_i64().unwrap());
    assert_eq!(until - since, 30 * 60);
    assert_eq!(report["desktops"], json!([DESK]), "only the paired desktop app is to be shown them");
}
