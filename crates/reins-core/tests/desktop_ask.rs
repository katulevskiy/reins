//! `desktop_ask`: a yes-or-no question from the paired desktop app (`reins ask`, a harness hook). Only the app whose
//! key was pinned at pairing may ask; the user sees the question, its detail and topic; yes is an `AskAnswer` sealed
//! to the app's key with its nonce, no is the ordinary denial, and a standing answer covers one topic.

mod common;

use common::desktop::{DESK, Desk, REFUSED, call, choice, desk_with, mount_reins, standing};
use reins_core::{ApprovalKind, CoreConfig, CoreError};
use reins_proto::desktop::{AskAnswer, encode_key};
use serde_json::{Value, json};
use wiremock::MockServer;

async fn desk() -> Desk {
    let server = MockServer::start().await;
    mount_reins(&server).await;
    desk_with(server, "me@example.com", "pw", CoreConfig::default()).await
}

fn ask(desk: &Desk, id: &str, question: &str, topic: Option<&str>) -> Value {
    let mut args = json!({"question": question, "detail": "git push --force origin main\nrewrites 3 commits",
        "client_key": desk.public(), "nonce": format!("nonce-{id}")});
    if let Some(t) = topic {
        args["topic"] = json!(t);
    }
    call(id, DESK, "desktop", "ask", &args)
}

#[tokio::test]
async fn only_the_paired_app_may_ask() {
    let desk = desk().await;
    desk.send(&[ask(&desk, "r0", "Push?", None)]).await;
    assert_eq!(desk.error("r0").await, REFUSED, "nothing pinned yet");
    desk.pair().await;
    let other = encode_key(crypto_box::SecretKey::generate(&mut crypto_box::aead::OsRng).public_key().as_bytes());
    let mut wrong_key = ask(&desk, "r1", "Push?", None);
    wrong_key["call"]["args"]["client_key"] = json!(other);
    let mut wrong_connection = ask(&desk, "r2", "Push?", None);
    wrong_connection["connection_id"] = json!("claude");
    desk.send(&[wrong_key, wrong_connection]).await;
    assert_eq!(desk.error("r1").await, REFUSED);
    assert_eq!(desk.error("r2").await, REFUSED);
    assert!(desk.waiting().await.is_empty());
}

#[tokio::test]
async fn a_question_is_shown_and_yes_is_sealed_to_the_app_with_its_nonce() {
    let desk = desk().await;
    desk.pair().await;
    desk.send(&[ask(&desk, "r1", "Force push to main?", Some("command:git push --force"))]).await;
    assert_eq!(desk.waiting().await, ["r1"]);
    let item = desk.core.pending().await.unwrap().remove(0);
    assert_eq!((item.service.as_str(), item.op_title.as_str()), ("desktop", "Ask you on your phone"));

    let view = desk.core.approval_view("r1".to_owned()).await.unwrap();
    assert_eq!(view.kind, ApprovalKind::Write);
    let asked = view.ask.clone().expect("the question");
    assert_eq!(asked.question, "Force push to main?");
    assert_eq!(asked.detail.as_deref(), Some("git push --force origin main\nrewrites 3 commits"));
    assert_eq!(asked.topic.as_deref(), Some("command:git push --force"));
    assert_eq!(view.preview, ["Force push to main?", "About: command:git push --force"]);
    let resources: Vec<_> = view.resources.iter().map(|r| (r.id.as_str(), r.label.as_str())).collect();
    assert_eq!(resources, [("command:git push --force", "Questions about command:git push --force")]);
    assert!(view.git.is_none() && view.secrets.is_none() && view.ssh.is_none());

    desk.core.approve("r1".to_owned(), choice(&[], None)).await.unwrap();
    let data = desk.data("r1").await;
    assert_eq!(data.as_object().unwrap().len(), 1, "only the sealed answer: {data}");
    let answer: AskAnswer = desk.open(&data["sealed"]);
    assert_eq!(
        answer,
        AskAnswer {
            v: 1,
            nonce: "nonce-r1".to_owned(),
            approved: true
        }
    );
    let entry = desk.core.activity(10).await.unwrap().into_iter().find(|a| a.op == "ask").unwrap();
    assert_eq!((entry.outcome.as_str(), entry.service.as_str()), ("sent", "desktop"));
}

#[tokio::test]
async fn no_is_the_ordinary_denial() {
    let desk = desk().await;
    desk.pair().await;
    desk.send(&[ask(&desk, "r1", "Delete the database?", None)]).await;
    desk.core.deny("r1".to_owned()).await.unwrap();
    let answer = desk.answer("r1").await.unwrap();
    assert_eq!(answer["outcome"], "denied", "{answer}");
    assert!(answer.get("result").is_none());
    assert!(desk.waiting().await.is_empty());
}

#[tokio::test]
async fn a_standing_answer_covers_its_topic_only() {
    let desk = desk().await;
    desk.pair().await;
    desk.send(&[ask(&desk, "r1", "Run the tests?", Some("command:cargo test"))]).await;
    desk.core.approve("r1".to_owned(), choice(&[], standing(&["command:cargo test"], &[]))).await.unwrap();

    desk.send(&[
        ask(&desk, "r2", "Run the tests again?", Some("command:cargo test")),
        ask(&desk, "r3", "Publish?", Some("command:cargo publish")),
        ask(&desk, "r4", "Anything?", None),
    ])
    .await;
    assert_eq!(desk.waiting().await, ["r3", "r4"]);
    let answer: AskAnswer = desk.open(&desk.data("r2").await["sealed"]);
    assert_eq!((answer.nonce.as_str(), answer.approved), ("nonce-r2", true));
    let plain = desk.core.approval_view("r4".to_owned()).await.unwrap();
    assert_eq!(plain.resources[0].id, "ask", "a question without a topic is its own thing");
    assert!(matches!(
        desk.core.approve("r3".to_owned(), choice(&[], standing(&["command:cargo test"], &[]))).await,
        Err(CoreError::Invalid { .. })
    ));
}

#[tokio::test]
async fn questions_and_topics_are_checked() {
    let desk = desk().await;
    desk.pair().await;
    let mut no_nonce = ask(&desk, "r2", "Ok?", None);
    no_nonce["call"]["args"]["nonce"] = json!("has space");
    desk.send(&[ask(&desk, "r1", "  ", None), no_nonce]).await;
    assert!(!desk.error("r1").await.is_empty(), "a blank question is refused");
    assert!(desk.error("r2").await.contains("nonce"));
    assert!(desk.waiting().await.is_empty());
}
