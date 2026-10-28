//! Decisions v2, 08 R1, end to end: the real server, the real phone core and this computer's app. The phone signs the
//! owner's Auto rules for the paired computer; the computer decides hook requests with them, falls back to asking when
//! what it holds is stale or tampered with, never loosens from its own settings, and hears a Lockdown within seconds.
//!
//! It lives here rather than in `reins-desktop` so the desktop crate (Apache-2.0) has no dependency on the AGPL-3.0
//! crates (`reins-core`, `reins-e2e`). See LICENSING.md.

#[path = "../../reins-desktop/tests/proxy_support/mod.rs"]
mod proxy_support;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use reins_core::autopilot::{CellChoice, ZoneChoice};
use reins_core::{AutopilotMode, PendingItem};
use reins_desktop::auth::prompt::NoPrompter;
use reins_desktop::config::{Config, Mode, Paths};
use reins_desktop::daemon::{Daemon, Options};
use reins_desktop::harness::Harness;
use reins_desktop::identity::Identity;
use reins_desktop::server::client::DesktopClient;
use reins_desktop::{hooks, journal, policy};
use reins_e2e::{Phone, Server};
use reins_policy::context::{FactKind, Zone};
use reins_policy::desktop_policy::Gate;
use reins_policy::effects::Level;
use reins_policy::matrix::{Cell, Outcome};

const EMAIL: &str = "dev@example.com";

struct Owner;
#[async_trait::async_trait]
impl reins_core::OwnerCheck for Owner {
    async fn confirm(&self, _reason: String) -> Result<bool, reins_core::ForeignError> {
        Ok(true)
    }
}

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

async fn next_item(phone: &Phone) -> PendingItem {
    phone.wait_for_item(Duration::from_secs(60)).await
}

/// `reins login`, the phone pinning the app's key.
async fn logged_in_app(server: &Server, phone: &Phone) -> (tempfile::TempDir, Paths) {
    let state = tempfile::tempdir().unwrap();
    let paths = Paths::under(state.path());
    paths.ensure().unwrap();
    let identity = Identity::load_or_create(&paths.identity_file()).unwrap();
    let (url_tx, url_rx) = tokio::sync::oneshot::channel::<String>();
    let (code_tx, code_rx) = tokio::sync::oneshot::channel::<u8>();
    let login = reins_desktop::server::oauth::login_with_browser(&paths, &identity, &server.base, move |url| {
        url_tx.send(url.to_owned()).unwrap();
    });
    let browser = async { browse(url_rx.await.unwrap(), code_tx).await };
    let user = async {
        let item = next_item(phone).await;
        let code = code_rx.await.unwrap();
        phone.core.answer_pairing(item.id, true, Some(code), Some("Laptop".to_owned()), true).await.unwrap();
    };
    let (logged_in, (), ()) = tokio::join!(login, browser, user);
    assert_eq!(logged_in.unwrap(), server.base);
    (state, paths)
}

/// What the server holds for this computer, applied.
async fn pull(paths: &Paths) -> policy::Applied {
    let identity = Identity::load_or_create(&paths.identity_file()).unwrap();
    let client = DesktopClient::new(paths).unwrap();
    let after = policy::PolicyStore::load(paths).newest();
    policy::pull(paths, &identity, &client, after, Duration::ZERO, 0).await.expect("the server answers")
}

/// One hook call of Claude Code running `command` in a project; the reason it was given.
async fn hook(paths: &Paths, config: &Config, command: &str) -> String {
    let home = reins_desktop::config::home_dir().unwrap();
    let cwd = home.join("Documents/dev/app");
    let input = serde_json::json!({"session_id": "s1", "cwd": cwd, "hook_event_name": "PreToolUse",
        "tool_name": "Bash", "tool_input": {"command": command}, "tool_use_id": "t"})
    .to_string();
    let (out, code, _) = hooks::run(Harness::ClaudeCode, input.as_bytes(), paths, config, &NoPrompter).await;
    assert_eq!(code, 0);
    out.map(|o| {
        let v: serde_json::Value = serde_json::from_str(&o).unwrap();
        v["hookSpecificOutput"]["permissionDecisionReason"].as_str().unwrap_or_default().to_owned()
    })
    .unwrap_or_default()
}

fn gate(paths: &Paths, config: &Config) -> Gate {
    policy::effective(paths, config, reins_desktop::now_unix()).0.gate
}

#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn the_computer_decides_with_the_phones_signed_policy() {
    reins_e2e::init_tls();
    let server = Server::start(20, 10).await;
    server.register(EMAIL).await;
    let phone = Phone::sign_in(&server.base, EMAIL).await;
    let (_state, paths) = logged_in_app(&server, &phone).await;
    let mut config = Config {
        mode: Mode::Reins,
        listen: SocketAddr::from(([127, 0, 0, 1], 0)),
        ..Config::default()
    };
    // A question the phone isn't open to answer fails fast.
    config.guard.timeout_secs = 5;

    // ---- nothing signed yet: today's flow ----
    assert_eq!(gate(&paths, &config), Gate::Fallback(reins_policy::desktop_policy::Fallback::NoSnapshot));

    // ---- the owner finishes Auto's setup on the phone: the computer gets the signed policy ----
    phone.core.finish_auto_setup(Arc::new(Owner)).await.unwrap();
    phone.core.sync(0).await.unwrap();
    let applied = pull(&paths).await;
    assert!(applied.snapshots >= 1, "{applied:?}");
    assert!(applied.refused.is_empty(), "{applied:?}");
    assert_eq!(
        gate(&paths, &config),
        Gate::Matrix {
            overnight: false
        }
    );
    let tickets = reins_desktop::tickets::TicketStore::load(&paths);
    assert_eq!(
        tickets.signing_key_fingerprint(),
        Some(phone.core.signing_key_fingerprint().await.unwrap()),
        "pinned the phone's own key"
    );

    // ---- the phone's matrix on the computer: build output deleted here, without the phone ----
    let started = Instant::now();
    let said = hook(&paths, &config, "rm -rf build dist").await;
    assert!(said.contains("allowed by Auto"), "{said}");
    assert!(started.elapsed() < Duration::from_secs(3), "decided here, not asked");
    let entry = journal::read(&paths, 1, 3_600).remove(0);
    let m = entry.matrix.expect("the matrix's line");
    assert!(m.decided && m.outcome == Outcome::Auto && m.zone == Zone::Mine, "{m:?}");

    // ---- the phone makes Mine · Undoable an Ask: the computer decides the same way ----
    phone.core.set_matrix_cell(ZoneChoice::Mine, 0, CellChoice::Ask, Arc::new(Owner)).await.unwrap();
    let applied = pull(&paths).await;
    assert!(applied.snapshots + applied.tightens >= 1, "{applied:?}");
    let (e, _) = policy::effective(&paths, &config, reins_desktop::now_unix());
    assert_eq!(e.snapshot.matrix.cell(Zone::Mine, Level::L0), Cell::Ask);
    let said = hook(&paths, &config, "rm -rf build dist").await;
    assert!(!said.contains("allowed by Auto"), "asked the phone now: {said}");
    phone.core.set_matrix_cell(ZoneChoice::Mine, 0, CellChoice::Auto, Arc::new(Owner)).await.unwrap();
    pull(&paths).await;
    assert!(hook(&paths, &config, "rm -rf build dist").await.contains("allowed by Auto"));

    // ---- an agent edits this computer's config to loosen it: nothing loosens ----
    let mut edited = config.clone();
    for z in Zone::ALL {
        for l in [Level::L0, Level::L1, Level::L2, Level::L3] {
            edited.auto.matrix.set(z, l, Cell::Auto);
        }
    }
    edited.auto.facts.set(FactKind::Owner, "evil", Zone::Mine);
    edited.auto.matrix_decides = true;
    edited.auto.setup_done = true;
    edited.workspace.roots = vec!["/".to_owned()];
    edited.guard.disabled_groups = reins_desktop::guard::GROUPS.iter().map(|g| g.id.to_owned()).collect();
    for command in ["cp notes.txt /etc/notes.txt", "cat ~/.local/state/reins/tickets.json"] {
        let said = hook(&paths, &edited, command).await;
        assert!(!said.contains("allowed by Auto"), "{command}: {said}");
    }

    // ---- what the computer holds, tampered with: asked as before ----
    let file = paths.state_dir.join("policy.json");
    let kept = std::fs::read_to_string(&file).unwrap();
    std::fs::write(&file, kept.replace("\\\"decides\\\":true", "\\\"decides\\\":false")).unwrap();
    assert!(matches!(gate(&paths, &config), Gate::Fallback(_)));
    assert!(!hook(&paths, &config, "rm -rf build dist").await.contains("allowed by Auto"));
    std::fs::write(&file, &kept).unwrap();
    assert_eq!(
        gate(&paths, &config),
        Gate::Matrix {
            overnight: false
        }
    );

    // ---- the daemon long-polls: a Lockdown on the phone reaches the computer within seconds ----
    let daemon = Daemon::bind(
        &paths,
        config.clone(),
        Options {
            authorizer: None,
            prompter: Arc::new(NoPrompter),
            harden: false,
            process_info: reins_desktop::runner::process_info,
        },
    )
    .await
    .unwrap();
    let _running = daemon.spawn();
    // The daemon's first pick-up, then its long poll.
    tokio::time::sleep(Duration::from_secs(2)).await;
    let started = Instant::now();
    phone.core.set_autopilot_mode(None, Some(AutopilotMode::Lockdown), None).await.unwrap();
    while gate(&paths, &config) != Gate::Lockdown {
        assert!(started.elapsed() < Duration::from_secs(5), "no Lockdown after {:?}", started.elapsed());
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let took = started.elapsed();
    eprintln!("Lockdown reached the computer in {} ms", took.as_millis());
    let said = hook(&paths, &config, "ls -la").await;
    assert!(said.contains("Lockdown"), "{said}");

    // ---- back to Auto: the computer decides again ----
    phone.core.set_autopilot_mode(None, Some(AutopilotMode::Auto), None).await.unwrap();
    let started = Instant::now();
    while gate(&paths, &config) == Gate::Lockdown {
        assert!(started.elapsed() < Duration::from_secs(10), "still locked down");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
