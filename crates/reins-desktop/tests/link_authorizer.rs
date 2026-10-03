//! `RewardenAuthorizer` against a mock Rewarden server whose phone answers from a script: approved reads and pushes,
//! answers that do not match the request (refused), denials, errors, waiting, and retries of an unanswered request.

mod link_mock;

use std::sync::Arc;

use link_mock::{App, Mock, Step, config, logged_in, push_to, repo, tag_push};
use rewarden_desktop::auth::rewarden::RewardenAuthorizer;
use rewarden_desktop::auth::{Authorizer as _, Refusal};
use rewarden_proto::desktop::{GIT_FETCH_TOOL, GIT_PUSH_TOOL, GIT_TAG_PUSH_TOOL};

const DIGEST: &str = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08";

async fn setup(timeout_secs: u64) -> (Mock, App, RewardenAuthorizer) {
    let mock = Mock::start().await;
    let app = logged_in(&mock).await;
    let auth = RewardenAuthorizer::new(&app.paths, Arc::clone(&app.identity), &config(timeout_secs)).unwrap();
    (mock, app, auth)
}

fn unavailable(r: Result<rewarden_desktop::auth::Credential, Refusal>) -> String {
    match r {
        Err(Refusal::Unavailable(m)) => m,
        other => panic!("expected Unavailable, got {other:?}"),
    }
}

#[tokio::test]
async fn an_approved_read_is_a_credential_cached_until_it_expires() {
    let (mock, app, auth) = setup(5).await;
    assert_eq!(auth.describe(), format!("your phone, through {}", mock.base));

    let c = auth.read(&repo("octo/hello")).await.unwrap();
    assert_eq!(c.username, "x-access-token");
    assert_eq!(c.token.as_str(), "ghs-req-1");
    assert!(c.is_live(rewarden_desktop::now_unix()));
    let calls = mock
        .with(|s| s.calls.iter().map(|c| (c.tool.clone(), c.arguments.clone(), c.account.clone())).collect::<Vec<_>>());
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, GIT_FETCH_TOOL);
    assert_eq!(calls[0].1["repo"], "octo/hello");
    assert_eq!(calls[0].1["client_key"], app.identity.public_key());
    assert_eq!(calls[0].1["nonce"].as_str().unwrap().len(), 22, "128 random bits, base64url");
    assert_eq!(calls[0].2, "octo", "the account from the config");
    assert!(!format!("{c:?}").contains("ghs-"), "the token never shows in debug output");

    // Cached: no second question.
    let again = auth.read(&repo("octo/hello")).await.unwrap();
    assert_eq!(again.token.as_str(), c.token.as_str());
    assert_eq!(mock.with(|s| s.calls.len()), 1);

    // Another repository is another question.
    auth.read(&repo("octo/other")).await.unwrap();
    assert_eq!(mock.with(|s| s.calls.len()), 2);
}

#[tokio::test]
async fn an_expired_read_lease_is_asked_again() {
    let (mock, _app, auth) = setup(5).await;
    mock.plan(&[Step::Tamper(|g| g.expires_at = rewarden_desktop::now_unix() + 2)]);
    auth.read(&repo("octo/hello")).await.unwrap();
    auth.read(&repo("octo/hello")).await.unwrap();
    assert_eq!(mock.with(|s| s.calls.len()), 1);
    tokio::time::sleep(std::time::Duration::from_millis(2100)).await;
    auth.read(&repo("octo/hello")).await.unwrap();
    assert_eq!(mock.with(|s| s.calls.len()), 2);
}

#[tokio::test]
async fn an_approved_push_is_bound_to_its_digest_and_never_cached() {
    let (mock, _app, auth) = setup(5).await;
    let summary = push_to("main");
    let c = auth.push(&repo("octo/hello"), &summary, DIGEST).await.unwrap();
    assert!(c.is_live(rewarden_desktop::now_unix()));
    let (tool, args) = mock.with(|s| (s.calls[0].tool.clone(), s.calls[0].arguments.clone()));
    assert_eq!(tool, GIT_PUSH_TOOL);
    assert_eq!(args["digest"], DIGEST);
    assert_eq!(
        serde_json::from_value::<rewarden_proto::desktop::PushSummary>(args["summary"].clone()).unwrap(),
        summary
    );

    auth.push(&repo("octo/hello"), &summary, DIGEST).await.unwrap();
    assert_eq!(mock.with(|s| s.calls.len()), 2, "a push credential is for one push");

    auth.push(&repo("octo/hello"), &tag_push(), DIGEST).await.unwrap();
    assert_eq!(mock.with(|s| s.calls[2].tool.clone()), GIT_TAG_PUSH_TOOL);
}

#[tokio::test]
async fn answers_that_do_not_match_the_request_are_refused() {
    let (mock, _app, auth) = setup(5).await;
    let cases: [(Step, &str); 7] = [
        (Step::Tamper(|g| g.nonce = "replayed".to_owned()), "nonce"),
        (Step::Tamper(|g| g.repo = "octo/elsewhere".to_owned()), "octo/elsewhere"),
        (Step::Tamper(|g| g.access = "write".to_owned()), "write access"),
        (Step::Tamper(|g| g.expires_at = rewarden_desktop::now_unix() - 1), "expired"),
        (Step::OtherKey, "not sealed to this app's key"),
        (Step::Garbage, "malformed"),
        (Step::Tamper(|g| g.token = String::new()), "no token"),
    ];
    for (step, expected) in cases {
        mock.plan(&[step]);
        let m = unavailable(auth.read(&repo("octo/hello")).await);
        assert!(m.contains(expected), "{expected}: {m}");
    }

    let summary = push_to("main");
    for (step, expected) in [
        (Step::Tamper(|g| g.digest = Some("0".repeat(64))), "digest"),
        (Step::Tamper(|g| g.digest = None), "digest"),
        (Step::Tamper(|g| g.access = "read".to_owned()), "read access"),
        (Step::Tamper(|g| g.nonce = "replayed".to_owned()), "nonce"),
        (Step::Tamper(|g| g.expires_at = 0), "expired"),
    ] {
        mock.plan(&[step]);
        let m = unavailable(auth.push(&repo("octo/hello"), &summary, DIGEST).await);
        assert!(m.contains(expected), "{expected}: {m}");
    }
    // Nothing refused was cached: the next read asks again.
    let asked = mock.with(|s| s.calls.len());
    auth.read(&repo("octo/hello")).await.unwrap();
    assert_eq!(mock.with(|s| s.calls.len()), asked + 1);
}

#[tokio::test]
async fn denials_and_errors_from_the_phone_become_refusals() {
    let (mock, _app, auth) = setup(5).await;
    mock.plan(&[Step::Denied(None)]);
    mock.plan(&[Step::Denied(Some("Not this repository."))]);
    mock.plan(&[Step::Error("GitHub said 404 for octo/hello.")]);
    assert_eq!(auth.read(&repo("octo/hello")).await.unwrap_err(), Refusal::Denied("Denied on your phone.".to_owned()));
    assert_eq!(
        auth.push(&repo("octo/hello"), &push_to("main"), DIGEST).await.unwrap_err(),
        Refusal::Denied("Not this repository.".to_owned())
    );
    assert_eq!(
        auth.read(&repo("octo/hello")).await.unwrap_err(),
        Refusal::Unavailable("GitHub said 404 for octo/hello.".to_owned())
    );
    // A denial is final: the next attempt is a new question.
    auth.read(&repo("octo/hello")).await.unwrap();
    assert_eq!(mock.with(|s| s.calls.len()), 4);
}

#[tokio::test]
async fn a_pending_request_is_polled_until_answered() {
    let (mock, _app, auth) = setup(10).await;
    mock.plan(&[Step::Pending, Step::Offline, Step::Approve]);
    auth.push(&repo("octo/hello"), &push_to("main"), DIGEST).await.unwrap();
    assert_eq!(mock.with(|s| s.calls.len()), 1);
    assert_eq!(mock.with(|s| s.polls.clone()), ["req-1", "req-1"]);
}

#[tokio::test]
async fn an_unanswered_push_waits_and_its_retry_reuses_the_request() {
    let (mock, _app, auth) = setup(1).await;
    mock.plan(&[Step::Offline, Step::Pending, Step::Approve]);
    let summary = push_to("main");
    let first = auth.push(&repo("octo/hello"), &summary, DIGEST).await.unwrap_err();
    assert_eq!(
        first,
        Refusal::Waiting("Waiting for approval on your phone. Approve it, then run git push again.".to_owned())
    );

    // A different push is a different question.
    mock.plan(&[Step::Approve]);
    auth.push(&repo("octo/hello"), &summary, &"e".repeat(64)).await.unwrap();
    assert_eq!(mock.with(|s| s.calls.len()), 2);

    // git push again: the same digest polls the same request (and its nonce still matches).
    let mut tries = 0;
    let c = loop {
        tries += 1;
        match auth.push(&repo("octo/hello"), &summary, DIGEST).await {
            Ok(c) => break c,
            Err(Refusal::Waiting(_)) if tries < 5 => {}
            Err(other) => panic!("{other:?}"),
        }
    };
    assert!(c.token.starts_with("ghs-req-1"), "the answer to the first request");
    assert_eq!(mock.with(|s| s.calls.len()), 2, "never asked twice");
    assert!(mock.with(|s| s.polls.iter().all(|p| p == "req-1")));
}

#[tokio::test]
async fn an_unanswered_read_waits_and_a_forgotten_request_is_asked_again() {
    let (mock, _app, auth) = setup(1).await;
    mock.plan(&[Step::Pending]);
    let Err(Refusal::Waiting(m)) = auth.read(&repo("octo/hello")).await else {
        panic!("expected waiting")
    };
    assert!(m.contains("Approve it"), "{m}");

    // The server forgot the request (expired): the retry asks anew instead of failing.
    let forgotten = mock.with(|s| s.calls.len());
    mock.with(link_mock::State::forget_requests);
    auth.read(&repo("octo/hello")).await.unwrap();
    assert_eq!(mock.with(|s| s.calls.len()), forgotten + 1);
    let polls = mock.with(|s| s.polls.clone());
    assert!(polls.contains(&"req-1".to_owned()), "the earlier request was tried first");
    assert!(polls.iter().all(|p| p == "req-1"), "{polls:?}");
}

#[tokio::test]
async fn a_phone_that_pinned_another_key_answers_with_an_error() {
    let (mock, _app, auth) = setup(5).await;
    mock.with(|s| s.pinned_key = Some(rewarden_desktop::identity::Identity::generate().public_key()));
    let m = unavailable(auth.read(&repo("octo/hello")).await);
    assert!(m.contains("paired with this phone"), "{m}");
}
