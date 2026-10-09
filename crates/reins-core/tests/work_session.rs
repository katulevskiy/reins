//! Work sessions from the desktop app: one request, asked every time, that becomes ordinary grants of the app's
//! connection on approval (origin `session`, ending together); ending one is done at once and touches only its own
//! grants.

mod common;

use common::desktop::{DESK, Desk, REFUSED, call, choice, desk_with, mount_reins, standing};
use reins_core::{CoreConfig, CoreError};
use serde_json::{Value, json};
use wiremock::MockServer;

async fn desk() -> Desk {
    let server = MockServer::start().await;
    mount_reins(&server).await;
    desk_with(server, "me@example.com", "pw", CoreConfig::default()).await
}

fn start(desk: &Desk, id: &str, args: &Value) -> Value {
    let mut args = args.clone();
    args["client_key"] = json!(desk.public());
    args["nonce"] = json!(format!("nonce-{id}"));
    call(id, DESK, "desktop", "session", &args)
}

fn end(desk: &Desk, id: &str, grants: &[String]) -> Value {
    call(
        id,
        DESK,
        "desktop",
        "session_end",
        &json!({"grants": grants, "client_key": desk.public(), "nonce": format!("nonce-{id}")}),
    )
}

#[tokio::test]
async fn a_session_is_one_request_and_becomes_grants_that_end_together() {
    let desk = desk().await;
    desk.pair().await;
    desk.send(&[start(
        &desk,
        "s1",
        &json!({"duration_secs": 7200, "reason": "Fix the login bug", "push": ["github:me/app@feature/login"]}),
    )])
    .await;
    assert_eq!(desk.waiting().await, ["s1"], "a session is always asked");
    let view = desk.core.approval_view("s1".to_owned()).await.unwrap();
    // What the user reads: where it came from, how long, every permission it makes, what stays asked.
    assert_eq!(view.preview[0], "Work session: \u{201c}Fix the login bug\u{201d}");
    assert_eq!(view.preview[1], "Asked from this computer. Didn't start it? Deny.");
    assert_eq!(view.preview[2], "For 2 h, all of it ending together:");
    assert!(
        view.preview.contains(&"• Push to GitHub me/app, branch feature/login (git only)".to_owned()),
        "{:?}",
        view.preview
    );
    assert!(view.preview.contains(&"• Fetch GitHub me/app (git only)".to_owned()), "{:?}", view.preview);
    assert!(view.preview.iter().any(|l| l.starts_with("Always asked")), "{:?}", view.preview);
    // Never answered from a notification, by "Approve all" or by Autopilot: the sheet has to be opened.
    assert!(view.no_standing && view.quick.is_none(), "{view:?}");
    // The session itself cannot be remembered.
    assert!(matches!(
        desk.core.approve("s1".to_owned(), choice(&[], standing(&["session"], &[]))).await,
        Err(CoreError::Invalid { .. })
    ));
    desk.core.approve("s1".to_owned(), choice(&[], None)).await.unwrap();
    let data = desk.data("s1").await;
    let ids: Vec<String> =
        data["grants"].as_array().unwrap().iter().map(|g| g["id"].as_str().unwrap().to_owned()).collect();
    assert_eq!(ids.len(), 2, "the push, and reading that repository: {data}");
    let expires = data["expires_at"].as_i64().unwrap();

    let grants = desk.core.grants().await.unwrap();
    let session: Vec<_> = grants.iter().filter(|g| ids.contains(&g.id)).collect();
    assert_eq!(session.len(), 2);
    assert!(session.iter().all(|g| g.origin == "session" && g.active && g.expires_at == Some(expires)));
    assert!(session.iter().any(|g| g.action == "write" && g.service == "github"), "{session:?}");

    // Ending: answered at once, only this connection's session grants.
    desk.send(&[end(&desk, "e1", &ids)]).await;
    assert!(desk.waiting().await.is_empty(), "ending is never asked");
    assert_eq!(desk.data("e1").await["ended"], 2);
    let after = desk.core.grants().await.unwrap();
    assert!(after.iter().filter(|g| ids.contains(&g.id)).all(|g| !g.active), "{after:?}");
}

#[tokio::test]
async fn a_session_never_covers_the_hard_floor_and_only_the_paired_app_may_start_one() {
    let desk = desk().await;
    desk.send(&[start(&desk, "s0", &json!({"duration_secs": 3600, "reason": "x", "read": ["gmail"]}))]).await;
    assert_eq!(desk.error("s0").await, REFUSED, "nothing pinned yet");
    desk.pair().await;
    desk.send(&[
        start(&desk, "s1", &json!({"duration_secs": 3600, "reason": "x", "read": ["vault"]})),
        start(&desk, "s2", &json!({"duration_secs": 3600, "reason": "x", "push": ["github:me/app@main~1"]})),
        start(&desk, "s3", &json!({"duration_secs": 3600, "reason": "x"})),
    ])
    .await;
    assert!(desk.error("s1").await.contains("never covers"));
    assert!(!desk.error("s2").await.is_empty());
    assert!(!desk.error("s3").await.is_empty());
    assert!(desk.waiting().await.is_empty());
    // Ending grants that are not a session's (or not this app's) does nothing.
    desk.send(&[end(&desk, "e1", &["not-a-grant".to_owned()])]).await;
    assert_eq!(desk.data("e1").await["ended"], 0);
}
