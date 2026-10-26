//! Autopilot from the desktop app: what the phone applies at once (only what makes Reins stricter), what it asks its
//! owner (anything looser, never decided by a mode: not Autopilot, not a bypass, not Lockdown), who decided an answer
//! (inside the answer), and the modes reported for the desktop apps.

mod common;

use common::desktop::{DESK, Desk, call, choice, desk_with, mount_reins};
use crypto_box::aead::{Aead as _, AeadCore as _, OsRng};
use crypto_box::{PublicKey, SalsaBox, SecretKey};
use data_encoding::BASE64URL_NOPAD;
use reins_core::{AutopilotMode, CoreConfig, CoreError};
use reins_proto::autopilot::{ASK_DIR, DesktopAsk};
use reins_proto::desktop::{PhoneKey, decode_key};
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

/// The phone's key, as the app gets it once (approved on the phone).
async fn phone_key(desk: &Desk) -> String {
    desk.send(&[call("k1", DESK, "vault", "phone_key", &json!({"client_key": desk.public(), "nonce": "nk"}))]).await;
    desk.core.approve("k1".to_owned(), choice(&[], None)).await.unwrap();
    let answer: PhoneKey = desk.open(&desk.data("k1").await["sealed"]);
    answer.public_key
}

/// A [`DesktopAsk`] for `op` with `args` and `nonce`, boxed from `from` to the phone's key.
fn proof(from: &SecretKey, phone: &str, op: &str, args: &Value, nonce: &str, created_at: i64) -> String {
    let ask = DesktopAsk {
        v: 1,
        dir: ASK_DIR.to_owned(),
        op: op.to_owned(),
        args: DesktopAsk::covered(args.as_object().unwrap()),
        nonce: nonce.to_owned(),
        created_at,
    };
    let cipher = SalsaBox::new(&PublicKey::from(decode_key(phone).unwrap()), from);
    let n = SalsaBox::generate_nonce(&mut OsRng);
    let mut out = n.to_vec();
    out.extend(cipher.encrypt(&n, serde_json::to_vec(&ask).unwrap().as_slice()).unwrap());
    BASE64URL_NOPAD.encode(&out)
}

/// The app's call `id` for `op` with `args`: its key and nonce, and, given the phone's key, its proof.
fn asked(desk: &Desk, phone: Option<&str>, id: &str, op: &str, args: &Value) -> Value {
    let nonce = format!("nonce-{id}");
    let mut args = args.clone();
    args["client_key"] = json!(desk.public());
    args["nonce"] = json!(nonce);
    if let Some(phone) = phone {
        args["proof"] = json!(proof(&desk.key, phone, op, &args, &nonce, reins_core::store::unix_now()));
    }
    call(id, DESK, "desktop", op, &args)
}

fn mode(desk: &Desk, phone: Option<&str>, id: &str, args: &Value) -> Value {
    asked(desk, phone, id, "autopilot_mode", args)
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
    // Stricter needs no proof: it can only tighten.
    desk.send(&[mode(&desk, None, "m1", &json!({"mode": "manual"}))]).await;
    assert!(desk.waiting().await.is_empty(), "back to Manual is never asked");
    assert_eq!(desk.data("m1").await["applied"], true);
    assert_eq!(global(&desk).await, AutopilotMode::Manual);
    desk.send(&[mode(&desk, None, "m2", &json!({"mode": "lockdown"}))]).await;
    assert!(desk.waiting().await.is_empty(), "Lockdown is the brake");
    assert_eq!(global(&desk).await, AutopilotMode::Lockdown);
    assert_eq!(reported(&desk).await["mode"], "lockdown", "the desktop apps hear of it");
}

#[tokio::test]
async fn anything_looser_is_asked_only_with_the_apps_own_proof() {
    let desk = desk().await;
    let phone = phone_key(&desk).await;
    // Without a proof, or with one the server could make (another key), or made for another call: refused.
    desk.send(&[mode(&desk, None, "m0", &json!({"mode": "auto"}))]).await;
    assert!(desk.error("m0").await.contains("could not be verified"), "no proof");
    let not_the_app = SecretKey::generate(&mut OsRng);
    let mut forged = json!({"mode": "auto", "client_key": desk.public(), "nonce": "nonce-f"});
    forged["proof"] =
        json!(proof(&not_the_app, &phone, "autopilot_mode", &forged, "nonce-f", reins_core::store::unix_now()));
    desk.send(&[call("f1", DESK, "desktop", "autopilot_mode", &forged)]).await;
    assert!(desk.error("f1").await.contains("could not be verified"), "not the app's key");
    let mut swapped = json!({"mode": "manual", "client_key": desk.public(), "nonce": "nonce-s"});
    swapped["proof"] =
        json!(proof(&desk.key, &phone, "autopilot_mode", &swapped, "nonce-s", reins_core::store::unix_now()));
    swapped["mode"] = json!("auto");
    desk.send(&[call("s1", DESK, "desktop", "autopilot_mode", &swapped)]).await;
    assert!(desk.error("s1").await.contains("could not be verified"), "made for another call");
    let mut old = json!({"mode": "auto", "client_key": desk.public(), "nonce": "nonce-o"});
    old["proof"] =
        json!(proof(&desk.key, &phone, "autopilot_mode", &old, "nonce-o", reins_core::store::unix_now() - 3_600));
    desk.send(&[call("o1", DESK, "desktop", "autopilot_mode", &old)]).await;
    assert!(desk.error("o1").await.contains("could not be verified"), "too old");
    assert!(desk.waiting().await.is_empty(), "none of them reached the owner");

    // The app's own: asked, worded by the phone, held, never quick, never remembered; once.
    let real = mode(&desk, Some(&phone), "m1", &json!({"mode": "auto"}));
    desk.send(std::slice::from_ref(&real)).await;
    assert_eq!(desk.waiting().await, ["m1"], "Auto from Manual is asked");
    let mut again = real;
    again["id"] = json!("m1-again");
    desk.send(&[again]).await;
    assert!(desk.error("m1-again").await.contains("sent before"), "a captured ask cannot be sent twice");
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

    // A bypass and Overnight start on the phone only.
    for (id, which) in [("b1", "bypass"), ("b2", "overnight")] {
        desk.send(&[mode(&desk, Some(&phone), id, &json!({"mode": which}))]).await;
        assert!(!desk.error(id).await.is_empty(), "{which} is not the computer's to start");
    }
    assert!(desk.waiting().await.is_empty());
}

#[tokio::test]
async fn no_mode_decides_a_change_of_autopilot_and_a_lockdown_denies_what_waited() {
    let desk = desk().await;
    let phone = phone_key(&desk).await;
    // A bypass approves everything else at once, never a change of Autopilot.
    desk.core.start_bypass(None, Some(30)).await.unwrap();
    desk.send(&[mode(&desk, Some(&phone), "a1", &json!({"mode": "auto"}))]).await;
    assert_eq!(desk.waiting().await, ["a1"], "the bypass did not approve it");
    // A lockdown that starts denies what a computer asked before it...
    desk.core.set_autopilot_mode(None, Some(AutopilotMode::Lockdown), None).await.unwrap();
    assert!(desk.waiting().await.is_empty(), "denied by the lockdown");
    assert_eq!(desk.answer("a1").await.unwrap()["outcome"], "denied");
    // ...and never what is asked after it: the way out waits for the owner, worded from the phone's state.
    desk.send(&[mode(&desk, Some(&phone), "m1", &json!({"mode": "manual"}))]).await;
    assert_eq!(desk.waiting().await, ["m1"], "leaving Lockdown is asked");
    let shown = desk.core.approval_view("m1".to_owned()).await.unwrap().ask.unwrap();
    assert_eq!(shown.question, "Set every AI to Manual and leave Lockdown?");
    assert_eq!(global(&desk).await, AutopilotMode::Lockdown);
    desk.core.deny("m1".to_owned()).await.unwrap();
    assert_eq!(global(&desk).await, AutopilotMode::Lockdown);
}

#[tokio::test]
async fn a_computer_in_its_own_lockdown_changes_nothing() {
    let desk = desk().await;
    let phone = phone_key(&desk).await;
    desk.core.set_autopilot_mode(None, Some(AutopilotMode::Auto), None).await.unwrap();
    desk.core.set_autopilot_mode(Some(DESK.to_owned()), Some(AutopilotMode::Lockdown), None).await.unwrap();
    for (id, which) in [("m1", "manual"), ("m2", "auto")] {
        desk.send(&[mode(&desk, Some(&phone), id, &json!({"mode": which}))]).await;
        assert!(desk.error(id).await.contains("Lockdown"), "{which}");
    }
    assert!(desk.waiting().await.is_empty());
    assert_eq!(global(&desk).await, AutopilotMode::Auto);
}

#[tokio::test]
async fn an_answer_says_which_mode_decided_it() {
    let desk = desk().await;
    desk.core.start_bypass(None, Some(30)).await.unwrap();
    desk.send(&[ask(&desk, "a1")]).await;
    let answer = desk.answer("a1").await.unwrap();
    assert_eq!(answer["outcome"], "result");
    assert_eq!(answer["decided"]["by"], "bypass", "{answer}");
    assert_eq!(answer["decided"]["reasons"], json!([{"code": "bypass"}]), "codes, never text");
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
async fn a_correction_names_this_computers_own_decision_from_the_phone() {
    let desk = desk().await;
    let phone = phone_key(&desk).await;
    let correct = |id: &str, request: &str, should: &str| {
        asked(&desk, Some(&phone), id, "autopilot_correct", &json!({"request_id": request, "should_have": should}))
    };
    // Nothing Autopilot decided for this computer under that id: refused, either way, before anyone is asked.
    for (id, should) in [("c1", "deny"), ("c2", "approve")] {
        desk.send(&[correct(id, "someone-elses", should)]).await;
        assert!(desk.error(id).await.contains("did not decide"), "{should}");
    }
    assert!(desk.waiting().await.is_empty());
    // A request a bypass approved for this computer.
    desk.core.start_bypass(None, Some(30)).await.unwrap();
    desk.send(&[ask(&desk, "a1")]).await;
    desk.core.set_autopilot_mode(None, Some(AutopilotMode::Manual), None).await.unwrap();
    // Teaching it to approve is asked, naming the request from the phone's own activity.
    desk.send(&[correct("c3", "a1", "approve")]).await;
    assert_eq!(desk.waiting().await, ["c3"]);
    let shown = desk.core.approval_view("c3".to_owned()).await.unwrap().ask.unwrap();
    assert!(shown.question.contains("approve") && shown.destructive, "{shown:?}");
    let detail = shown.detail.unwrap();
    assert!(detail.contains("Deploy now?") && detail.contains("allowed"), "{detail}");
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

#[tokio::test]
async fn an_ai_connection_cannot_change_a_mode_even_to_a_stricter_one() {
    let desk = desk().await;
    desk.core.set_autopilot_mode(None, Some(AutopilotMode::Auto), None).await.unwrap();
    // The same call from a connection whose desktop key the phone never pinned (an AI's): refused before anything.
    let args = json!({"mode": "lockdown", "client_key": desk.public(), "nonce": "nonce-ai"});
    desk.send(&[call("ai1", "ai-connection", "desktop", "autopilot_mode", &args)]).await;
    assert!(desk.waiting().await.is_empty(), "nothing is asked of the owner");
    assert!(!desk.error("ai1").await.is_empty(), "refused");
    assert_eq!(global(&desk).await, AutopilotMode::Auto, "nothing changed");
    let correct = call(
        "ai2",
        "ai-connection",
        "desktop",
        "autopilot_correct",
        &json!({"request_id": "r1", "should_have": "approve", "client_key": desk.public(), "nonce": "nonce-ai2"}),
    );
    desk.send(&[correct]).await;
    assert!(desk.waiting().await.is_empty() && !desk.error("ai2").await.is_empty());
}
