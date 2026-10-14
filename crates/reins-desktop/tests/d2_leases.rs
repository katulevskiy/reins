//! The daemon's leases of Reins Secrets against a mock phone: a release is kept for exactly what the phone bound it
//! to and reused only for that; anything else asks again; dropping a lease asks again; the control routes carry it.

mod d2_support;

use std::sync::Arc;
use std::time::Duration;

use d2_support::{Mock, Step, logged_in, quiet};
use reins_desktop::auth::prompt::NoPrompter;
use reins_desktop::config::{Config, Mode};
use reins_desktop::control::Client;
use reins_desktop::daemon::{Daemon, Options};
use reins_desktop::identity::Identity;
use reins_desktop::leases::{LeaseCache, RevokeRequest, SecretsService};
use reins_desktop::phone::PhoneLink;
use reins_desktop::secrets_client::LeaseRequest;

fn req(runner: &str, root: &str, fields: Option<&[&str]>, args: &[&str]) -> LeaseRequest {
    LeaseRequest {
        project: "acme".into(),
        env: "dev".into(),
        fields: fields.map(|f| f.iter().map(|x| (*x).to_owned()).collect()),
        program: "/usr/bin/npm".into(),
        args: args.iter().map(|a| (*a).to_owned()).collect(),
        root: root.into(),
        runner: runner.into(),
        lease_secs: 600,
        purpose: None,
        confirm_always: false,
        print: false,
    }
}

const A: &str = "/home/me/dev/a";

fn service(mock: &Mock) -> (SecretsService, d2_support::App) {
    let app = logged_in(mock);
    let identity = Arc::new(Identity::load_or_create(&app.paths.identity_file()).unwrap());
    let link = Arc::new(PhoneLink::new(Mode::Auto, app.paths.clone(), identity, Duration::from_secs(5)));
    (SecretsService::new(Arc::new(LeaseCache::default()), link), app)
}

#[tokio::test]
async fn a_lease_is_reused_only_for_the_same_runner_folder_variables_and_command() {
    let mock = Mock::start().await;
    let (svc, _app) = service(&mock);
    let asks = || mock.calls_of("vault_env_release").len();
    let first = svc.lease(&req("person", A, Some(&["API_KEY"]), &["run", "dev"]), None).await.unwrap();
    assert_eq!((first.cached, first.values[0].1.as_str()), (false, "sk-env-5ecret-dev"));
    let again = svc.lease(&req("person", A, Some(&["API_KEY"]), &["run", "dev"]), None).await.unwrap();
    assert!(again.cached && asks() == 1, "the same command: no question");
    for (other, why) in [
        (req("agent:claude-code", A, Some(&["API_KEY"]), &["run", "dev"]), "another runner"),
        (req("person", "/home/me/dev/b", Some(&["API_KEY"]), &["run", "dev"]), "another folder"),
        (req("person", A, Some(&["API_KEY", "DB_URL"]), &["run", "dev"]), "more variables"),
        (req("person", A, Some(&["API_KEY"]), &["run", "test"]), "another command"),
    ] {
        let before = asks();
        let a = svc.lease(&other, None).await.unwrap();
        assert!(!a.cached && asks() == before + 1, "{why}: asked again");
    }
    assert_eq!(svc.cache().list(reins_desktop::now_unix()).len(), 5);
    assert_eq!(
        svc.cache().revoke(&RevokeRequest {
            all: true,
            ..RevokeRequest::default()
        }),
        5
    );
    let before = asks();
    svc.lease(&req("person", A, Some(&["API_KEY"]), &["run", "dev"]), None).await.unwrap();
    assert_eq!(asks(), before + 1, "dropped: asked again");
}

#[tokio::test]
async fn a_prefix_the_user_chose_on_the_phone_covers_more_commands_and_nothing_else() {
    let mock = Mock::start().await;
    let (svc, _app) = service(&mock);
    mock.plan(&[Step::ApprovePrefix(1)]);
    svc.lease(&req("person", A, None, &["run", "dev"]), None).await.unwrap();
    let test = svc.lease(&req("person", A, None, &["run", "test", "--watch"]), None).await.unwrap();
    assert!(test.cached, "npm run * covers npm run test --watch");
    assert_eq!(test.values.len(), 2, "every variable");
    let only = svc.lease(&req("person", A, Some(&["DB_URL"]), &["run", "lint"]), None).await.unwrap();
    assert_eq!((only.cached, only.values.len()), (true, 1), "a subset of a whole environment");
    let before = mock.calls_of("vault_env_release").len();
    svc.lease(&req("person", A, None, &["exec", "x"]), None).await.unwrap();
    assert_eq!(mock.calls_of("vault_env_release").len(), before + 1, "npm exec is not npm run");
}

#[tokio::test]
async fn an_answer_bound_to_someone_else_is_refused_and_not_kept() {
    let mock = Mock::start().await;
    let (svc, _app) = service(&mock);
    mock.plan(&[Step::OtherRunner]);
    let e = svc.lease(&req("person", A, None, &["run", "dev"]), None).await.unwrap_err();
    assert!(e.message().contains("someone else"), "{}", e.message());
    assert!(svc.cache().is_empty());
}

#[tokio::test]
async fn the_control_routes_list_and_revoke_but_never_hand_out_a_value() {
    let mock = Mock::start().await;
    let app = logged_in(&mock);
    quiet(&app.paths);
    let mut config = Config {
        listen: "127.0.0.1:0".parse().unwrap(),
        mode: Mode::Auto,
        approval_timeout_secs: 5,
        ..Config::default()
    };
    config.ssh.enabled = false;
    let daemon = Daemon::bind(
        &app.paths,
        config,
        Options {
            authorizer: None,
            prompter: Arc::new(NoPrompter),
            harden: false,
            process_info: reins_desktop::runner::process_info,
        },
    )
    .await
    .unwrap();
    let running = daemon.spawn();
    let client = Client::new(&app.paths, running.addr).unwrap();
    assert!(client.leases().await.unwrap().is_empty());
    assert_eq!(
        client
            .revoke(&RevokeRequest {
                all: true,
                ..RevokeRequest::default()
            })
            .await
            .unwrap(),
        0
    );
    let http = reins_desktop::http::client(None).unwrap();
    let token = std::fs::read_to_string(app.paths.control_token_file()).unwrap();
    let lease = http
        .post(format!("http://{}/_reins/secrets/lease", running.addr))
        .header("x-reins-token", token.trim())
        .body(serde_json::to_vec(&req("person", A, None, &["run", "dev"])).unwrap())
        .send()
        .await
        .unwrap();
    assert_eq!(lease.status().as_u16(), 404, "no route gives values out");
    assert!(mock.calls().is_empty());
    let resp = http.get(format!("http://{}/_reins/secrets/leases", running.addr)).send().await.unwrap();
    assert_eq!(resp.status().as_u16(), 401, "without the token: nothing");
}
