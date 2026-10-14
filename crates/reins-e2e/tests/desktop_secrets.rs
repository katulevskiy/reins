//! Reins Secrets through the whole chain: the real server binary, the real phone core with its vault, and this app
//! logged in with its key pinned. The computer saves an environment (one approval, the same check digits on both
//! sides, the values boxed to the phone's key), the daemon gets it released for one command and keeps it for exactly
//! what the phone bound it to, a remembered prefix covers the next command, and Revoke on all computers reaches the
//! daemon through the server.

use std::sync::Arc;
use std::time::Duration;

use reins_core::{ApprovalChoice, GrantScopeChoice, SecretsScopeChoice, StandingGrant};
use reins_desktop::config::{Config, Mode, Paths};
use reins_desktop::identity::Identity;
use reins_desktop::leases::{LeaseCache, SecretsService};
use reins_desktop::phone::PhoneLink;
use reins_desktop::secrets_client::{LeaseRequest, PhoneVaultWriter};
use reins_desktop::vault_writer::{EnvChanges, VaultWriter, check_digits};
use reins_e2e::{PASSWORD, Phone, Server};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const EMAIL: &str = "dev@example.com";
const ROOT: &str = "/home/dev/acme";

fn between<'a>(haystack: &'a str, before: &str, after: &str) -> Option<&'a str> {
    let start = haystack.find(before)? + before.len();
    let end = haystack[start..].find(after)?;
    Some(&haystack[start..start + end])
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

fn remember(command: &str) -> Option<StandingGrant> {
    Some(StandingGrant {
        duration_secs: Some(3_600),
        max_uses: None,
        scope: GrantScopeChoice {
            all_mail: false,
            selected_messages_only: false,
            sender_addresses: vec![],
            sender_domains: vec![],
            subject_pattern: None,
            recipient_addresses: vec![],
            recipient_domains: vec![],
            resources: vec![],
            classes: vec![],
            secrets: Some(SecretsScopeChoice {
                command: command.to_owned(),
                all_variables: false,
            }),
        },
    })
}

fn req(args: &[&str]) -> LeaseRequest {
    LeaseRequest {
        project: "acme".into(),
        env: "dev".into(),
        fields: None,
        program: "/usr/bin/npm".into(),
        args: args.iter().map(|a| (*a).to_owned()).collect(),
        root: ROOT.into(),
        runner: "person".into(),
        lease_secs: 3_600,
        purpose: None,
        confirm_always: false,
        print: false,
        typed: None,
        aliases: Vec::new(),
        path_check: None,
        exec_env: String::new(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn an_environment_saved_from_the_computer_is_released_bound_to_one_command_and_revoked_everywhere() {
    reins_e2e::init_tls();
    let server = Server::start(20, 10).await;
    server.register(EMAIL).await;
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/user"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"login": "me", "id": 1})))
        .mount(&api)
        .await;
    let phone = Phone::sign_in_with_github(&server.base, EMAIL, &api.uri()).await;
    phone.core.add_token_account("vault".to_owned(), PASSWORD.to_owned()).await.expect("the vault unlocks");

    // ---- reins login ----
    let state = tempfile::tempdir().unwrap();
    let paths = Paths::under(state.path());
    paths.ensure().unwrap();
    std::fs::write(paths.config_file(), "[notify]\nphone = false\n").unwrap();
    let identity = Identity::load_or_create(&paths.identity_file()).unwrap();
    let (url_tx, url_rx) = tokio::sync::oneshot::channel::<String>();
    let (code_tx, code_rx) = tokio::sync::oneshot::channel::<u8>();
    let login = reins_desktop::server::oauth::login_with_browser(&paths, &identity, &server.base, move |url| {
        url_tx.send(url.to_owned()).unwrap();
    });
    let browser = async { browse(url_rx.await.unwrap(), code_tx).await };
    let user = async {
        let item = phone.wait_for_item(Duration::from_secs(60)).await;
        let code = code_rx.await.unwrap();
        phone.core.answer_pairing(item.id, true, Some(code), Some("Laptop".to_owned())).await.unwrap();
    };
    let (logged_in, (), ()) = tokio::join!(login, browser, user);
    logged_in.unwrap();
    let config = Config {
        mode: Mode::Reins,
        approval_timeout_secs: 60,
        ..Config::default()
    };

    // ---- reins secrets set: the phone's key first (typed digits), then one approval with the same check digits ----
    let digits = phone.core.phone_key_fingerprint().await.unwrap();
    let writer = PhoneVaultWriter::new(&paths, &config, Some(digits));
    let mut dev = EnvChanges::new("acme", "dev");
    dev.set = vec![("API_KEY".into(), "sk-e2e-5ecret".into()), ("DB_URL".into(), "postgres://u:e2e-5ecret@db".into())];
    let check = check_digits(std::slice::from_ref(&dev));
    let store = writer.store_envs(std::slice::from_ref(&dev));
    let user = async {
        let key = phone.wait_for_item(Duration::from_secs(60)).await;
        phone
            .core
            .approve(
                key.id,
                ApprovalChoice {
                    selected_message_ids: vec![],
                    standing: None,
                },
            )
            .await
            .unwrap();
        let item = phone.wait_for_item(Duration::from_secs(60)).await;
        let view = phone.core.approval_view(item.id.clone()).await.unwrap();
        let s = view.secrets_store.clone().expect("a store");
        assert_eq!((s.check.as_str(), s.total, s.envs[0].new_env), (check.as_str(), 2, true));
        assert!(!format!("{view:?}").contains("5ecret"));
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
    let (stored, ()) = tokio::join!(store, user);
    let receipt = stored.unwrap();
    assert_eq!((receipt.envs[0].created, receipt.envs[0].added), (true, 2));
    assert!(!server.log().contains("5ecret"), "the server never sees a value");

    // ---- reins run: the daemon asks once; the user remembers `npm run dev *` ----
    let phone_link = Arc::new(PhoneLink::new(
        Mode::Reins,
        paths.clone(),
        Arc::new(Identity::load_or_create(&paths.identity_file()).unwrap()),
        Duration::from_secs(60),
    ));
    let cache = Arc::new(LeaseCache::default());
    let daemon = SecretsService::new(Arc::clone(&cache), Arc::clone(&phone_link));
    let dev_run = req(&["run", "dev"]);
    let lease = daemon.lease(&dev_run, None);
    let user = async {
        let item = phone.wait_for_item(Duration::from_secs(60)).await;
        let view = phone.core.approval_view(item.id.clone()).await.unwrap();
        let r = view.env_release.clone().expect("a release");
        assert_eq!(
            (r.command.as_str(), r.variables.clone()),
            ("npm run dev", vec!["API_KEY".to_owned(), "DB_URL".to_owned()])
        );
        phone
            .core
            .approve(
                item.id,
                ApprovalChoice {
                    selected_message_ids: vec![],
                    standing: remember("prefix:2"),
                },
            )
            .await
            .unwrap();
    };
    let (first, ()) = tokio::join!(lease, user);
    let first = first.unwrap();
    assert_eq!(first.values[0], ("API_KEY".to_owned(), "sk-e2e-5ecret".to_owned()));
    let more = req(&["run", "dev", "--silent"]);
    assert!(daemon.lease(&more, None).await.unwrap().cached, "the lease covers npm run dev *");

    // A daemon that starts over asks again, and the phone's permission answers without the user.
    let fresh = SecretsService::new(Arc::new(LeaseCache::default()), Arc::clone(&phone_link));
    let lint = req(&["run", "dev", "app"]);
    let lease = fresh.lease(&lint, None);
    let mut lease = std::pin::pin!(lease);
    // The phone app is open: it syncs, and its permission answers.
    let again = loop {
        tokio::select! {
            done = &mut lease => break done.unwrap(),
            synced = phone.core.sync(2) => assert!(synced.unwrap().is_empty(), "nothing for the user to decide"),
        }
    };
    assert!(!again.cached && again.values.len() == 2);

    // ---- Revoke on all computers: the permission ends and the daemon's lease goes ----
    let revoked = phone.core.revoke_secrets_everywhere(Some("acme".to_owned()), None).await.unwrap();
    assert_eq!((revoked.told, revoked.not_delivered.len()), (1, 0));
    let client = reins_desktop::server::client::DesktopClient::new(&paths).unwrap();
    let mail = client.revocations("daemon-1").await.unwrap();
    // The store left one too (the environment changed), then the revocation.
    assert_eq!(mail.items.len(), 2);
    let me = Identity::load_or_create(&paths.identity_file()).unwrap();
    let ack = reins_desktop::leases::apply_mail(
        &cache,
        &|sealed| me.unseal(sealed).ok(),
        &mail,
        "daemon-1",
        reins_desktop::now_unix(),
    );
    assert!(cache.is_empty());
    // Acknowledged: this reader is not handed them again; another reader still is.
    client.ack_revocations(&ack).await.unwrap();
    let again = client.revocations("daemon-1").await.unwrap();
    assert!(again.items.is_empty() && again.lost.is_none());
    assert_eq!(client.revocations("daemon-2").await.unwrap().items.len(), 2);
    let asked = fresh.cache();
    asked.revoke(&reins_desktop::leases::RevokeRequest {
        all: true,
        ..Default::default()
    });
    let lease = fresh.lease(&lint, None);
    let user = async {
        let item = phone.wait_for_item(Duration::from_secs(60)).await;
        assert!(phone.core.approval_view(item.id.clone()).await.unwrap().env_release.is_some(), "asked again");
        phone.core.deny(item.id).await.unwrap();
    };
    let (denied, ()) = tokio::join!(lease, user);
    assert!(denied.is_err());
}
