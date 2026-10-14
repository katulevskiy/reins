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

    // ---- reins run: the daemon asks once; the user remembers `npm run *` ----
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
                    standing: remember("prefix:1"),
                },
            )
            .await
            .unwrap();
    };
    let (first, ()) = tokio::join!(lease, user);
    let first = first.unwrap();
    assert_eq!(first.values[0], ("API_KEY".to_owned(), "sk-e2e-5ecret".to_owned()));
    assert!(daemon.lease(&req(&["run", "test"]), None).await.unwrap().cached, "the lease covers npm run *");

    // A daemon that starts over asks again, and the phone's permission answers without the user.
    let fresh = SecretsService::new(Arc::new(LeaseCache::default()), Arc::clone(&phone_link));
    let lint = req(&["run", "lint"]);
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
    assert_eq!(phone.core.revoke_secrets_everywhere(Some("acme".to_owned()), None).await.unwrap(), 1);
    let notes = reins_desktop::server::client::DesktopClient::new(&paths).unwrap().revocations().await.unwrap();
    // The store left one too (the environment changed), then the revocation.
    assert_eq!(notes.len(), 2);
    let me = Identity::load_or_create(&paths.identity_file()).unwrap();
    let mut dropped = 0;
    for sealed in &notes {
        let note: reins_proto::desktop::LeaseRevocation = serde_json::from_slice(&me.unseal(sealed).unwrap()).unwrap();
        dropped += cache.apply(&note, reins_desktop::now_unix());
    }
    assert_eq!(dropped, 1);
    assert!(cache.is_empty());
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

/// This computer for an import, without touching the real one: its trash is a folder, the rest says no.
struct TestMachine {
    trash: std::path::PathBuf,
}

#[async_trait::async_trait]
impl reins_desktop::secrets_import::Machine for TestMachine {
    fn trash(&self, path: &std::path::Path) -> Result<Option<String>, String> {
        let to = self.trash.join(uuid::Uuid::new_v4().to_string());
        std::fs::rename(path, &to).map_err(|e| e.to_string())?;
        Ok(to.file_name().map(|n| n.to_string_lossy().into_owned()))
    }
    fn restore(&self, _: &std::path::Path, _: Option<&str>) -> Result<(), String> {
        Err("not in this test".to_owned())
    }
    fn purge(&self, _: &std::path::Path, _: Option<&str>) -> Result<(), String> {
        Err("not in this test".to_owned())
    }
    fn gh_token(&self, _: &str) -> Option<zeroize::Zeroizing<String>> {
        None
    }
    async fn agent_signs_with(&self, _: &[u8]) -> Result<(), String> {
        Err("no agent in this test".to_owned())
    }
    fn ssh_setup(&self) -> Result<bool, String> {
        Ok(false)
    }
    fn ssh_unsetup(&self) -> Result<(), String> {
        Ok(())
    }
    fn git_through_reins(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn gh_logout(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn add_profile(&self, _: &str, _: &std::collections::BTreeMap<String, String>, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn remove_profile(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
}

/// Approves the requests the phone gets until `done` has `n` of them; checks the store's view on the way (the import's
/// check digits, no value) and returns it.
async fn approve_all(phone: &Phone, n: usize, check: &std::sync::Mutex<Option<String>>, secret: &str) -> usize {
    let mut stores = 0;
    for _ in 0..n {
        let item = phone.wait_for_item(Duration::from_secs(60)).await;
        let view = phone.core.approval_view(item.id.clone()).await.unwrap();
        assert!(!format!("{view:?}").contains(secret), "the phone's view never shows a value");
        if let Some(s) = &view.secrets_store {
            stores += 1;
            assert_eq!(Some(s.check.clone()), check.lock().unwrap().clone(), "the same check digits on both sides");
        }
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
    }
    stores
}

#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn an_import_moves_a_projects_secrets_into_the_vault_with_one_approval() {
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

    // ---- a project with an .env, as a scan finds it ----
    let home = tempfile::tempdir().unwrap();
    let repo = home.path().join("dev/web");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    std::fs::write(repo.join(".git/config"), "[remote \"origin\"]\n\turl = git@github.com:acme/web.git\n").unwrap();
    let secret = format!("sk-proj-{}", "Ab3dE5fG7hJ9kLmN2pQ4rS6tU8vWxYz0cR1aB3dE5fG7hJ9k");
    std::fs::write(repo.join(".env"), format!("OPENAI_API_KEY={secret}\nLOG_LEVEL=debug\n")).unwrap();
    let mut opts = reins_desktop::secrets_scan::ScanOptions::new(home.path().to_path_buf());
    opts.caller = reins_desktop::secrets_scan::Caller::Person;
    opts.gh_cli = false;
    opts.git_history = false;
    let report = reins_desktop::secrets_scan::scan(&opts, &mut |_| {}).unwrap();
    let selected = reins_desktop::secrets_import::select(&report, &[], true).unwrap();
    assert_eq!(selected.len(), 1);

    // ---- the import: the vault's projects (to never merge unasked), then one store, the same digits on both sides ----
    let digits = phone.core.phone_key_fingerprint().await.unwrap();
    let writer = reins_desktop::secrets_import::real::PhoneWriter {
        paths: paths.clone(),
        config: config.clone(),
        phone_key_digits: Some(digits),
    };
    let trash = tempfile::tempdir().unwrap();
    let machine = TestMachine {
        trash: trash.path().to_path_buf(),
    };
    let ctx = reins_desktop::secrets_import::Context {
        home: home.path().to_path_buf(),
        state_dir: paths.state_dir.clone(),
        computer: "Laptop".to_owned(),
        machine: reins_desktop::secrets_import::machine_project("Laptop", "e2e"),
        caller: reins_desktop::secrets_scan::Caller::Person,
        merge: std::collections::BTreeSet::new(),
    };
    let check = std::sync::Mutex::new(None::<String>);
    let mut shown = |_: &reins_desktop::secrets_import::Plan, c: &str| {
        *check.lock().unwrap() = Some(c.to_owned());
        Ok(())
    };
    let import = reins_desktop::secrets_import::import(&ctx, &report, &selected, false, &writer, &machine, &mut shown);
    // The phone's key, the list of projects, the store.
    let (imported, stores) = tokio::join!(import, approve_all(&phone, 3, &check, &secret));
    let (receipt, _) = imported.unwrap();
    assert_eq!(stores, 1, "one approval for the whole batch");
    assert_eq!(receipt.moved.len(), 1);
    assert_eq!(receipt.moved[0].to, "vault:acme-web/development/OPENAI_API_KEY");
    // Out of the repository; the original is in the trash, plain text there until shredded.
    assert!(receipt.moved[0].in_trash && receipt.moved[0].on_disk);
    assert!(!server.log().contains(&secret), "the server never sees a value");

    // The repository uses it through Reins; the original is in the trash.
    assert!(!repo.join(".env").exists());
    let project = reins_desktop::project::ProjectFile::load(&repo.join("reins.toml")).unwrap();
    assert_eq!((project.name.as_str(), project.origin.as_deref()), ("acme-web", Some("github.com/acme/web")));
    let env = std::fs::read_to_string(repo.join(".env.reins")).unwrap();
    assert!(
        env.contains("OPENAI_API_KEY=vault:acme-web/development/OPENAI_API_KEY") && env.contains("LOG_LEVEL=debug")
    );

    // The vault knows where the project came from: a later import of another `web` is not merged into it.
    let listed = reins_desktop::secrets_import::VaultWriter::projects(&writer);
    let (projects, _) = tokio::join!(listed, approve_all(&phone, 1, &check, &secret));
    let projects = projects.unwrap();
    let web = projects.iter().find(|p| p.name == "acme-web").expect("the project is in the vault");
    assert_eq!(web.origin.as_deref(), Some("github.com/acme/web"));
}
