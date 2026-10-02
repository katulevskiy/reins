//! `rewarden ask` with the phone: the question goes out as `desktop_ask`, and only a decision sealed to this app's key
//! that echoes the question's nonce counts.

mod d1_mock;

use std::time::Duration;

use d1_mock::{Mock, Step, logged_in, logged_out};
use rewarden_desktop::ask::{Answer, Question, ask, ask_phone};
use rewarden_desktop::auth::prompt::NoPrompter;
use rewarden_desktop::config::{Config, Mode};

fn q() -> Question {
    Question::new("Deploy to production?", Some("make deploy\nenv=prod"), Some("command:make deploy")).unwrap()
}

const T: Duration = Duration::from_secs(10);

#[tokio::test]
async fn an_approval_sealed_for_this_question_is_a_yes() {
    let mock = Mock::start().await;
    let app = logged_in(&mock, 3600);
    assert_eq!(ask_phone(&app.paths, &app.identity, &q(), T).await, Answer::Yes);
    let args = mock.with(|s| s.calls.pop().unwrap());
    assert_eq!(args["question"], "Deploy to production?");
    assert_eq!(args["detail"], "make deploy\nenv=prod");
    assert_eq!(args["topic"], "command:make deploy");
    assert_eq!(args["client_key"], app.identity.public_key());
    assert_eq!(args["nonce"].as_str().unwrap().len(), 22);
    // Every question has its own nonce.
    ask_phone(&app.paths, &app.identity, &q(), T).await;
    let again = mock.with(|s| s.calls.pop().unwrap());
    assert_ne!(again["nonce"], args["nonce"]);
}

#[tokio::test]
async fn a_denial_or_a_sealed_no_is_a_no() {
    let mock = Mock::start().await;
    let app = logged_in(&mock, 3600);
    mock.plan(&[Step::Denied(Some("Not now."))]);
    assert_eq!(ask_phone(&app.paths, &app.identity, &q(), T).await, Answer::No("Not now.".into()));
    mock.plan(&[Step::Denied(None)]);
    assert_eq!(ask_phone(&app.paths, &app.identity, &q(), T).await, Answer::No("Denied on your phone.".into()));
    mock.plan(&[Step::Refuse]);
    assert_eq!(ask_phone(&app.paths, &app.identity, &q(), T).await.exit_code(), 1);
}

#[tokio::test]
async fn forged_or_unreadable_answers_are_not_a_yes() {
    let mock = Mock::start().await;
    let app = logged_in(&mock, 3600);
    for (step, says) in [
        (Step::ForgeNonce, "nonce differs"),
        (Step::OtherKey, "not sealed to this app's key"),
        (Step::Garbage, "malformed"),
        (Step::Error("The phone is locked."), "The phone is locked."),
    ] {
        mock.plan(&[step]);
        let a = ask_phone(&app.paths, &app.identity, &q(), T).await;
        let Answer::Unanswered(why) = &a else {
            panic!("{step:?}: {a:?}")
        };
        assert!(why.contains(says), "{step:?}: {why}");
        assert_eq!(a.exit_code(), 2);
    }
}

#[tokio::test]
async fn the_answer_is_polled_for_until_it_comes_or_the_time_is_up() {
    let mock = Mock::start().await;
    let app = logged_in(&mock, 3600);
    mock.plan(&[Step::Offline, Step::Pending, Step::Approve]);
    assert_eq!(ask_phone(&app.paths, &app.identity, &q(), T).await, Answer::Yes);
    assert_eq!(mock.with(|s| s.polls.len()), 2);

    mock.plan(&[Step::Pending]);
    let started = std::time::Instant::now();
    let a = ask_phone(&app.paths, &app.identity, &q(), Duration::from_secs(2)).await;
    assert!(matches!(&a, Answer::Unanswered(why) if why.contains("within 2 s")), "{a:?}");
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[tokio::test]
async fn an_expired_or_refused_token_is_renewed_once() {
    let mock = Mock::start().await;
    let app = logged_in(&mock, -10);
    assert_eq!(ask_phone(&app.paths, &app.identity, &q(), T).await, Answer::Yes);
    assert_eq!(mock.with(|s| s.refreshes), 1);
    mock.with(|s| s.reject_bearer = 1);
    assert_eq!(ask_phone(&app.paths, &app.identity, &q(), T).await, Answer::Yes);
    assert_eq!(mock.with(|s| s.refreshes), 2);
    mock.with(|s| {
        s.reject_bearer = 1;
        s.refuse_refresh = true;
    });
    let a = ask_phone(&app.paths, &app.identity, &q(), T).await;
    assert!(matches!(&a, Answer::Unanswered(why) if why.contains("rewarden login")), "{a:?}");
}

#[tokio::test]
async fn the_mode_decides_who_is_asked() {
    let mock = Mock::start().await;
    let app = logged_in(&mock, 3600);
    let mut config = Config::default();
    // Logged in: the phone, even without a terminal or a desktop prompt.
    assert_eq!(ask(&app.paths, &config, &q(), T, false, &NoPrompter).await, Answer::Yes);
    // Local: the desktop prompt, which here never answers.
    config.mode = Mode::Local;
    let a = ask(&app.paths, &config, &q(), T, false, &NoPrompter).await;
    assert_eq!(a.exit_code(), 2, "{a:?}");
    assert_eq!(mock.with(|s| s.calls.len()), 1);
    // Phone only, not logged in: no answer, and says why.
    let out = logged_out();
    config.mode = Mode::Rewarden;
    let a = ask(&out.paths, &config, &q(), T, false, &NoPrompter).await;
    assert!(matches!(&a, Answer::Unanswered(why) if why.contains("rewarden login")), "{a:?}");
}

/// The binary's exit codes: 0 yes, 1 no, 2 no answer.
#[tokio::test]
async fn the_command_line_exits_0_1_or_2() {
    let mock = Mock::start().await;
    let app = logged_in(&mock, 3600);
    let paths = app.paths.clone();
    let run = move |args: &[&str]| {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_rewarden"))
            .arg("ask")
            .args(args)
            .env("REWARDEN_CONFIG_DIR", &paths.config_dir)
            .env("REWARDEN_STATE_DIR", &paths.state_dir)
            .stdin(std::process::Stdio::null())
            .output()
            .unwrap();
        (out.status.code().unwrap(), String::from_utf8_lossy(&out.stderr).into_owned())
    };
    let (code, said) = tokio::task::spawn_blocking(move || {
        let yes = run(&["Ship it?", "--topic", "ship"]);
        mock.plan(&[Step::Denied(Some("No."))]);
        let no = run(&["Ship it?"]);
        mock.plan(&[Step::Pending]);
        let none = run(&["Ship it?", "--timeout", "1"]);
        let empty = run(&[" "]);
        (vec![yes.0, no.0, none.0, empty.0], vec![yes.1, no.1, none.1, empty.1])
    })
    .await
    .unwrap();
    assert_eq!(code, vec![0, 1, 2, 2], "{said:?}");
    assert!(said[0].contains("Asking on your phone") && said[0].contains("Yes."), "{}", said[0]);
    assert!(said[1].contains("No: No."), "{}", said[1]);
    assert!(said[2].contains("No answer"), "{}", said[2]);
    assert!(said.iter().all(|s| !s.contains("at-")), "no token is ever printed");
}
