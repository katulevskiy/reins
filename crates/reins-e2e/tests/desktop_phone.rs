//! The whole chain with the phone deciding: the real Reins server binary, the real phone core (GitHub token on the
//! phone, a fake GitHub REST API), this app logged in through the server's OAuth with the phone pinning its key, and
//! real git cloning and pushing through the daemon to a `git http-backend` upstream.
//!
//! It lives here rather than in `reins-desktop` so the desktop crate (Apache-2.0) has no dependency, not even a
//! dev-dependency, on the AGPL-3.0 crates (`reins-core`, `reins-e2e`). See LICENSING.md.

#[path = "../../reins-desktop/tests/proxy_support/mod.rs"]
mod proxy_support;

use std::sync::Arc;
use std::time::Duration;

use proxy_support::{Home, TOKEN, Upstream, run_git};
use reins_core::{ApprovalChoice, GrantScopeChoice, PendingItem, StandingGrant};
use reins_desktop::auth::prompt::NoPrompter;
use reins_desktop::config::{Config, Mode, Paths};
use reins_desktop::daemon::{Daemon, Options};
use reins_desktop::identity::Identity;
use reins_e2e::{Phone, Server};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const EMAIL: &str = "dev@example.com";
const REPO: &str = "me/app";
const PUSH_FEATURE: [&str; 4] = ["push", "-q", "origin", "feature"];
const FORCE_FEATURE: [&str; 5] = ["push", "-q", "--force", "origin", "feature"];

fn scope(resources: &[&str]) -> GrantScopeChoice {
    GrantScopeChoice {
        all_mail: false,
        selected_messages_only: false,
        sender_addresses: vec![],
        sender_domains: vec![],
        subject_pattern: None,
        recipient_addresses: vec![],
        resources: resources.iter().map(|r| (*r).to_owned()).collect(),
        classes: vec![],
        recipient_domains: vec![],
    }
}

fn hour(resources: &[&str]) -> Option<StandingGrant> {
    Some(StandingGrant {
        duration_secs: Some(3600),
        max_uses: None,
        scope: scope(resources),
    })
}

fn between<'a>(haystack: &'a str, before: &str, after: &str) -> Option<&'a str> {
    let start = haystack.find(before)? + before.len();
    let end = haystack[start..].find(after)?;
    Some(&haystack[start..start + end])
}

/// GitHub's REST API as the phone sees it: whose token this is, and the private repository.
async fn fake_github_api() -> MockServer {
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/user"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"login": "me", "id": 1})))
        .mount(&api)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/repos/{REPO}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"full_name": REPO, "private": true, "default_branch": "main", "permissions": {"push": true}}),
        ))
        .mount(&api)
        .await;
    api
}

/// Plays the browser of `reins login`: the email form, the code the phone must pick, the redirect to the app.
async fn browse(authorize: String, code_out: tokio::sync::oneshot::Sender<u8>) {
    let http = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().unwrap();
    let page = http.get(&authorize).send().await.unwrap().text().await.unwrap();
    let session = between(&page, "name=\"session\" value=\"", "\"").expect("session field").to_owned();
    let origin = {
        let u = url::Url::parse(&authorize).unwrap();
        format!("{}://{}", u.scheme(), u.host_str().unwrap()) + &u.port().map(|p| format!(":{p}")).unwrap_or_default()
    };
    let posted = http
        .post(format!("{origin}/reins/oauth/authorize"))
        .form(&[("session", session.as_str()), ("email", EMAIL)])
        .send()
        .await
        .unwrap();
    let wait = format!("{origin}{}", posted.headers()["location"].to_str().unwrap());
    let mut code_out = Some(code_out);
    for _ in 0..120 {
        let r = http.get(&wait).send().await.unwrap();
        if r.status() == reqwest::StatusCode::SEE_OTHER {
            let back = r.headers()["location"].to_str().unwrap().to_owned();
            assert!(back.starts_with("http://127.0.0.1:"), "redirected to {back}");
            http.get(&back).send().await.unwrap();
            return;
        }
        let html = r.text().await.unwrap();
        if let Some(tx) = code_out.take() {
            let code = between(&html, "aria-label=\"code\">", "<").expect("code on the wait page").parse().unwrap();
            tx.send(code).unwrap();
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    panic!("the phone never approved the desktop app");
}

/// The next request on the phone (the app is open and long-polling).
async fn next_item(phone: &Phone) -> PendingItem {
    phone.wait_for_item(Duration::from_secs(60)).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn the_phone_approves_git_reads_and_pushes_and_refuses_a_force_push() {
    reins_e2e::init_tls();
    let server = Server::start(20, 10).await;
    server.register(EMAIL).await;
    let api = fake_github_api().await;
    let phone = Phone::sign_in_with_github(&server.base, EMAIL, &api.uri()).await;
    phone.core.add_token_account("github".to_owned(), TOKEN.to_owned()).await.expect("GitHub on the phone");

    let upstream = Upstream::start().await;
    upstream.create(REPO, true);

    // ---- reins login: the phone shows this app's key fingerprint and pins the key ----
    let state = tempfile::tempdir().unwrap();
    let paths = Paths::under(state.path());
    paths.ensure().unwrap();
    let identity = Identity::load_or_create(&paths.identity_file()).unwrap();
    let fingerprint = identity.fingerprint();
    let (url_tx, url_rx) = tokio::sync::oneshot::channel::<String>();
    let (code_tx, code_rx) = tokio::sync::oneshot::channel::<u8>();
    let login = reins_desktop::server::oauth::login_with_browser(&paths, &identity, &server.base, move |url| {
        url_tx.send(url.to_owned()).unwrap();
    });
    let browser = async {
        let authorize = url_rx.await.unwrap();
        assert!(authorize.contains("reins_client_key="), "{authorize}");
        browse(authorize, code_tx).await;
    };
    let user = async {
        let item = next_item(&phone).await;
        let view = phone.core.pairing_view(item.id.clone()).await.unwrap();
        assert_eq!(view.key_fingerprint.as_deref(), Some(fingerprint.as_str()), "the phone shows the app's key");
        let code = code_rx.await.unwrap();
        phone.core.answer_pairing(item.id, true, Some(code), Some("Laptop".to_owned())).await.unwrap();
    };
    let (logged_in, (), ()) = tokio::join!(login, browser, user);
    assert_eq!(logged_in.unwrap(), server.base);

    // ---- reins doctor's view of the phone: it polled while approving the pairing ----
    let seen = reins_desktop::server::client::DesktopClient::new(&paths).unwrap().phone().await.unwrap();
    let last = seen.last_seen.expect("the phone polled");
    assert!((0..120).contains(&(seen.server_time - last)), "{seen:?}");
    let doctor = reins_desktop::doctor::phone_check(seen.last_seen, seen.server_time);
    assert_eq!(doctor.level, reins_desktop::doctor::Level::Ok, "{doctor:?}");

    // ---- the daemon, deciding with the phone ----
    let mut config = Config {
        listen: "127.0.0.1:0".parse().unwrap(),
        mode: Mode::Reins,
        approval_timeout_secs: 60,
        ..Config::default()
    };
    config.github.git_base = upstream.url();
    config.github.api_base = format!("{}/api", upstream.url());
    let daemon = Daemon::bind(
        &paths,
        config.clone(),
        Options {
            authorizer: None,
            prompter: Arc::new(NoPrompter),
            harden: false,
        },
    )
    .await
    .unwrap();
    let running = daemon.spawn();
    let home = Home::new(running.addr);

    // ---- clone: the phone asks once, the user allows reading this repository for an hour ----
    let remote = format!("https://github.com/{REPO}.git");
    let clone_args = ["clone", "-q", remote.as_str(), "app"];
    let clone = home.git("", &clone_args);
    let user = async {
        let item = next_item(&phone).await;
        assert_eq!(item.op, "git_fetch", "{item:?}");
        let view = phone.core.approval_view(item.id.clone()).await.unwrap();
        assert!(view.git.is_none());
        phone
            .core
            .approve(
                item.id,
                ApprovalChoice {
                    selected_message_ids: vec![REPO.to_owned()],
                    standing: hour(&[REPO]),
                },
            )
            .await
            .unwrap();
    };
    let (clone, ()) = tokio::join!(clone, user);
    assert!(clone.ok, "{}", clone.all());
    let git_config = std::fs::read_to_string(home.path("app/.git/config")).unwrap();
    assert!(!git_config.contains(TOKEN) && !clone.all().contains(TOKEN), "the agent never sees the token");

    // ---- push a new branch: the phone shows exactly what is pushed; the user allows this branch for an hour ----
    run_git(&home.path("app"), &[], &["checkout", "-q", "-b", "feature"]);
    let first = home.commit("app", "hello.txt", b"hello\n", "Add hello").await;
    let push = home.git("app", &PUSH_FEATURE);
    let user = async {
        let item = next_item(&phone).await;
        assert_eq!(item.op, "git_push", "{item:?}");
        let view = phone.core.approval_view(item.id.clone()).await.unwrap();
        let git = view.git.clone().expect("a git view");
        assert_eq!(git.repo, REPO);
        assert_eq!((git.refs[0].short_name.as_str(), git.refs[0].change.as_str()), ("feature", "create"));
        assert!(git.refs[0].commits.iter().any(|c| c.subject == "Add hello"), "{git:?}");
        assert!(view.resources.iter().any(|r| r.id == format!("{REPO}@feature")), "{:?}", view.resources);
        phone
            .core
            .approve(
                item.id,
                ApprovalChoice {
                    selected_message_ids: vec![],
                    standing: hour(&[&format!("{REPO}@feature")]),
                },
            )
            .await
            .unwrap();
    };
    let (push, ()) = tokio::join!(push, user);
    assert!(push.ok, "{}", push.all());
    assert_eq!(upstream.head(REPO, "feature").as_deref(), Some(first.as_str()));

    // ---- a fast-forward to the same branch is covered by the grant: nobody is asked ----
    let second = home.commit("app", "hello.txt", b"hello again\n", "Say it again").await;
    let push = home.git("app", &PUSH_FEATURE);
    let app_open = async {
        let mut asked = Vec::new();
        for _ in 0..3 {
            asked.extend(phone.core.sync(5).await.unwrap());
            if !asked.is_empty() {
                break;
            }
        }
        asked
    };
    let (push, asked) = tokio::join!(push, app_open);
    assert!(push.ok, "{}", push.all());
    assert!(asked.is_empty(), "the branch grant covers a fast-forward: {asked:?}");
    assert_eq!(upstream.head(REPO, "feature").as_deref(), Some(second.as_str()));

    // ---- a force push is asked every time, cannot be remembered, and the user denies it ----
    // An explicit identity: run_git does not use the test HOME's git config (CI runners have no global identity).
    run_git(
        &home.path("app"),
        &[],
        &["-c", "user.name=Me", "-c", "user.email=me@example.com", "commit", "-q", "--amend", "-m", "Rewritten"],
    );
    let force = home.git("app", &FORCE_FEATURE);
    let user = async {
        let item = next_item(&phone).await;
        let view = phone.core.approval_view(item.id.clone()).await.unwrap();
        assert!(view.no_standing, "a force push is asked every time");
        assert!(view.git.as_ref().unwrap().refs[0].force, "{:?}", view.git);
        phone.core.deny(item.id).await.unwrap();
    };
    let (force, ()) = tokio::join!(force, user);
    assert!(!force.ok);
    assert!(force.all().contains("remote rejected"), "{}", force.all());
    assert_eq!(upstream.head(REPO, "feature").as_deref(), Some(second.as_str()), "nothing changed upstream");

    // ---- fetching again within the hour needs nobody ----
    let fetch = home.git("app", &["fetch", "-q", "origin"]).await;
    assert!(fetch.ok, "{}", fetch.all());
    assert!(phone.core.pending().await.unwrap().is_empty());

    // ---- reins ask: a yes from the phone, then a no ----
    let question = |q: &str| reins_desktop::ask::Question {
        question: q.to_owned(),
        detail: Some("terraform apply -auto-approve".to_owned()),
        topic: Some("command:terraform apply".to_owned()),
    };
    // The third: a long command, cut to 300 characters with an ellipsis (more than 300 bytes), and a detail of
    // multi-byte text, both within the limits counted in characters.
    let long = format!("Claude Code wants to run: {}…", "x".repeat(273));
    assert_eq!(long.chars().count(), 300);
    for (approve, text) in [(true, "Deploy to prod?"), (false, "Drop the staging database?"), (true, long.as_str())] {
        let mut q = question(text);
        if text == long {
            q.detail = Some(format!("Command:\n{}", "é".repeat(7_000)));
        }
        let asked = reins_desktop::ask::ask_phone(&paths, &identity, &q, Duration::from_secs(60));
        let user = async {
            let item = next_item(&phone).await;
            let view = phone.core.approval_view(item.id.clone()).await.unwrap();
            let shown = view.ask.clone().expect("an ask view");
            assert_eq!(shown.question, q.question);
            if approve {
                phone
                    .core
                    .approve(
                        item.id,
                        ApprovalChoice {
                            selected_message_ids: vec![],
                            standing: None,
                        },
                    )
                    .await
                    .unwrap();
            } else {
                phone.core.deny(item.id).await.unwrap();
            }
        };
        let (answer, ()) = tokio::join!(asked, user);
        match (approve, answer) {
            (true, reins_desktop::ask::Answer::Yes) | (false, reins_desktop::ask::Answer::No(_)) => {}
            (_, other) => panic!("approve={approve} answered {other:?}"),
        }
    }
    // ---- a work session: one approval on the phone becomes its permissions; ending it needs none ----
    let request = reins_desktop::work_session::Request {
        secs: 3_600,
        reason: "Work on dev".to_owned(),
        from_cli: true,
        read: vec![],
        push: vec![reins_desktop::work_session::Branch {
            service: "github".to_owned(),
            host: "github.com".to_owned(),
            repo: REPO.to_owned(),
            branch: "dev".to_owned(),
        }],
    };
    let starting = reins_desktop::work_session::start(&paths, &config, &request, None);
    let user = async {
        let item = next_item(&phone).await;
        let view = phone.core.approval_view(item.id.clone()).await.unwrap();
        assert_eq!(view.preview[0], "Work session: Work on dev");
        assert!(view.preview[1].contains("command line"), "{:?}", view.preview);
        assert_eq!(view.preview[2], "For 1 h, all of it ending together:");
        assert!(view.no_standing, "the session itself is never remembered");
        phone
            .core
            .approve(
                item.id,
                ApprovalChoice {
                    selected_message_ids: vec![],
                    standing: None,
                },
            )
            .await
            .unwrap();
    };
    let (started, ()) = tokio::join!(starting, user);
    let started = started.unwrap();
    assert_eq!(started.grants.len(), 2, "{started:?}");
    assert_eq!(reins_desktop::work_session::current(&paths).as_ref(), Some(&started));
    let grants = phone.core.grants().await.unwrap();
    assert_eq!(grants.iter().filter(|g| g.origin == "session" && g.active).count(), 2);
    // The phone app is open (polling): ending is answered without anyone deciding.
    let ended = tokio::select! {
        r = reins_desktop::work_session::end(&paths, &config) => r.unwrap(),
        () = async {
            loop {
                phone.core.sync(1).await.ok();
            }
        } => unreachable!(),
    };
    assert_eq!(ended, 2);
    assert!(reins_desktop::work_session::current(&paths).is_none());
    assert!(phone.core.pending().await.unwrap().is_empty(), "ending asked nothing");
    let grants = phone.core.grants().await.unwrap();
    assert_eq!(grants.iter().filter(|g| g.origin == "session" && g.active).count(), 0);

    // ---- reins logout / uninstall: the connection ends on the server, not just here ----
    let kept = std::fs::read(paths.session_file()).unwrap();
    let out = reins_desktop::server::oauth::sign_out(&paths).await.unwrap();
    assert_eq!(out, reins_desktop::server::oauth::SignedOut::Revoked);
    assert!(reins_desktop::server::oauth::logged_in_server(&paths).is_none());
    // A copy of the old session is worthless now: its access and refresh tokens died with the connection.
    reins_desktop::config::write_private(&paths.session_file(), &kept).unwrap();
    let revoked = reins_desktop::server::client::DesktopClient::new(&paths).unwrap().phone().await;
    assert!(matches!(revoked, Err(reins_desktop::server::LinkError::LoggedOut(_))), "{revoked:?}");
    assert!(!server.log().contains("panicked"), "server log: {}", server.log());
}
