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

/// How one hook request ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ended {
    /// The computer allowed it itself, with the policy it holds.
    Here,
    /// The phone answered yes on its own.
    Phone,
    /// The owner was asked (and said no).
    Owner,
}

/// Runs the hook for `command` with the phone app open: the phone decides what it may decide on its own, and the
/// owner (this test) denies whatever reaches them.
async fn decide(paths: &Paths, config: &Config, phone: &Phone, command: &str) -> Ended {
    let decided_before = phone.notes.decided.lock().unwrap().len();
    let mut asked_owner = false;
    let said = {
        let mut run = std::pin::pin!(hook(paths, config, command));
        loop {
            tokio::select! {
                said = &mut run => break said,
                items = phone.core.sync(1) => {
                    for item in items.expect("sync") {
                        asked_owner = true;
                        phone.core.deny(item.id).await.unwrap();
                    }
                }
            }
        }
    };
    // The computer hears the answer before the phone's pass has told its app who decided: give it a moment.
    let decided = || phone.notes.decided.lock().unwrap().len() > decided_before;
    let waited = Instant::now();
    while said.contains("Reins 2FA: approved") && !decided() && waited.elapsed() < Duration::from_secs(5) {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let by_phone = decided();
    if said.contains("allowed by Auto") {
        assert!(!asked_owner && !by_phone, "{command}: {said}");
        Ended::Here
    } else if said.contains("Reins 2FA: approved") {
        assert!(by_phone && !asked_owner, "{command}: approved, but not by the phone's own rules: {said}");
        Ended::Phone
    } else {
        assert!(asked_owner && said.contains("denied"), "{command}: {said}");
        Ended::Owner
    }
}

/// The copy of the policy a computer holds no longer decides (here: it still says Manual, the phone moved on to
/// Auto; the same path as a copy older than its ten minutes). The computer asks the phone, and sends what it read of
/// the command. The phone's own matrix decides the question exactly as the computer does with a fresh copy, and the
/// owner is asked only where the matrix asks. A computer whose settings an agent rewrote gets nothing more.
#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn the_phone_decides_a_hook_question_as_the_computer_would_with_a_fresh_copy() {
    reins_e2e::init_tls();
    let server = Server::start(20, 10).await;
    server.register(EMAIL).await;
    // (A computer paired less than ten minutes ago waits for its owner in every automatic mode: not this test's.)
    let phone = Phone::sign_in_with_config(&server.base, EMAIL, |cfg| cfg.new_connection_secs = 0).await;
    let (_state, paths) = logged_in_app(&server, &phone).await;
    let mut config = Config {
        mode: Mode::Reins,
        listen: SocketAddr::from(([127, 0, 0, 1], 0)),
        ..Config::default()
    };
    config.guard.timeout_secs = 30;
    phone.core.finish_auto_setup(Arc::new(Owner)).await.unwrap();
    // The computer says where its home folder is (the daemon's proposal); the phone reads it when it looks.
    reins_desktop::folders::propose(&paths, &reins_desktop::folders::proposal(&config.workspace)).await.unwrap();
    phone.core.folder_questions().await.unwrap();

    // What a hook asks about, in a project: build output, a folder of the home, a `.env`, an SSH configuration and a
    // private key, git history, the project's own sources.
    let commands = [
        "rm -rf build dist",
        "rm -rf ~/reins-e2e-elsewhere/photos",
        "cat .env",
        "cat ~/.ssh/config",
        "cat ~/.ssh/id_ed25519",
        "git push --force origin main",
        "rm -rf src",
    ];

    // ---- a fresh copy: the computer decides, and asks only where the matrix asks ----
    phone.core.sync(0).await.unwrap();
    pull(&paths).await;
    assert!(matches!(gate(&paths, &config), Gate::Matrix { .. }));
    let mut fresh = Vec::new();
    for command in commands {
        fresh.push(decide(&paths, &config, &phone, command).await);
    }
    assert_eq!(fresh[0], Ended::Here, "build output is the owner's own, and undoable");
    assert_eq!(fresh[1], Ended::Owner, "a folder of the home can't be brought back");
    assert!(fresh.iter().all(|e| *e != Ended::Phone), "with a fresh copy the phone answers nothing itself: {fresh:?}");

    // ---- the copy the computer holds says Manual; the phone is back on Auto, and the computer hasn't heard ----
    phone.core.set_autopilot_mode(None, Some(AutopilotMode::Manual), None).await.unwrap();
    pull(&paths).await;
    assert_eq!(gate(&paths, &config), Gate::Fallback(reins_policy::desktop_policy::Fallback::Manual));
    phone.core.set_autopilot_mode(None, Some(AutopilotMode::Auto), None).await.unwrap();
    assert!(matches!(gate(&paths, &config), Gate::Fallback(_)), "it decides nothing itself");
    let mut asked = Vec::new();
    for command in commands {
        asked.push(decide(&paths, &config, &phone, command).await);
    }
    // The same decisions: what the computer allowed itself, the phone now answers without the owner.
    let expected: Vec<Ended> = fresh
        .iter()
        .map(|e| match e {
            Ended::Here => Ended::Phone,
            other => *other,
        })
        .collect();
    assert_eq!(asked, expected, "{commands:?}");
    for ((command, here), there) in commands.iter().zip(&fresh).zip(&asked) {
        eprintln!("{command}: a fresh copy {here:?}, a copy that no longer decides {there:?}");
    }
    let by: Vec<String> = phone.notes.decided.lock().unwrap().iter().map(|d| d.decided_by.clone()).collect();
    assert!(!by.is_empty() && by.iter().all(|b| b == "autopilot"), "{by:?}");
    // The computer's own log: the phone answered, and what the request does is kept as before.
    let entry = journal::read(&paths, 50, 3_600)
        .into_iter()
        .find(|e| e.what.ends_with("rm -rf build dist") && e.outcome == journal::Outcome::Approved)
        .expect("the phone's yes is logged");
    assert_eq!(entry.effects.as_ref().map(|e| e.level), Some(Level::L0), "{entry:?}");
    assert!(entry.matrix.is_some(), "{entry:?}");

    // ---- an agent rewrites this computer's settings: everything a root, the home folder scratch, Auto everywhere ----
    let mut edited = config.clone();
    for z in Zone::ALL {
        for l in [Level::L0, Level::L1, Level::L2, Level::L3] {
            edited.auto.matrix.set(z, l, Cell::Auto);
        }
    }
    edited.auto.facts.set(FactKind::Owner, "evil", Zone::Mine);
    edited.auto.matrix_decides = true;
    edited.auto.setup_done = true;
    edited.workspace.roots.push("/".to_owned());
    edited.workspace.scratch = vec!["~".to_owned()];
    for command in ["rm -rf ~/reins-e2e-elsewhere/photos", "cat ~/.ssh/id_ed25519", "git push --force origin main"] {
        assert_eq!(decide(&paths, &edited, &phone, command).await, Ended::Owner, "{command}");
    }
    // What the owner's own rules allow is still answered.
    assert_eq!(decide(&paths, &edited, &phone, "rm -rf build dist").await, Ended::Phone);

    // ---- the phone sent a fresh copy meanwhile: once the computer has it, it decides itself again ----
    pull(&paths).await;
    assert!(matches!(gate(&paths, &config), Gate::Matrix { .. }));
    assert_eq!(decide(&paths, &config, &phone, "rm -rf build dist").await, Ended::Here);
}

/// The computer proposes its `[workspace]` folders; the owner confirms one on the phone (the owner check); the
/// phone's signed snapshot then carries it, and the computer decides with it here. Removing it applies at once.
#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn the_computer_proposes_its_folders_and_uses_the_ones_confirmed() {
    use reins_core::autopilot::FolderKindChoice;

    reins_e2e::init_tls();
    let server = Server::start(20, 10).await;
    server.register(EMAIL).await;
    let phone = Phone::sign_in(&server.base, EMAIL).await;
    let (_state, paths) = logged_in_app(&server, &phone).await;
    phone.core.finish_auto_setup(Arc::new(Owner)).await.unwrap();
    // A scratch folder outside the standard roots (a real one: the computer resolves it before proposing).
    let home = reins_desktop::config::home_dir().unwrap();
    let scratch_dir = tempfile::Builder::new().prefix(".reins-e2e-scratch-").tempdir_in(&home).unwrap();
    let mut config = Config {
        mode: Mode::Reins,
        listen: SocketAddr::from(([127, 0, 0, 1], 0)),
        ..Config::default()
    };
    config.guard.timeout_secs = 5;
    config.workspace.scratch = vec![scratch_dir.path().to_string_lossy().into_owned(), "/".to_owned()];
    let resolved = reins_desktop::folders::canonical(&config.workspace.scratch[0], "").unwrap();

    // ---- before: the computer holds the signed policy, and its scratch folder isn't in it ----
    phone.core.sync(0).await.unwrap();
    pull(&paths).await;
    let (e, _) = policy::effective(&paths, &config, reins_desktop::now_unix());
    assert!(matches!(e.gate, Gate::Matrix { .. }), "{:?}", e.gate);
    assert!(!e.scratch.contains(&resolved), "{:?}", e.scratch);

    // ---- the computer proposes; the phone shows a card (only for what isn't signed, never `/`) ----
    let proposal = reins_desktop::folders::proposal(&config.workspace);
    assert!(proposal.folders.iter().all(|f| f.path != "/"), "{proposal:?}");
    reins_desktop::folders::propose(&paths, &proposal).await.unwrap();
    let cards = phone.core.folder_questions().await.unwrap();
    let card = cards.iter().find(|q| q.path == resolved).unwrap_or_else(|| panic!("no card: {cards:?}"));
    assert_eq!((card.kind, card.computer.as_str()), (FolderKindChoice::Scratch, "Laptop"));

    // ---- the owner confirms it: the snapshot carries it, the computer decides with it ----
    phone
        .core
        .confirm_computer_folder(card.connection_id.clone(), resolved.clone(), card.kind, Arc::new(Owner))
        .await
        .unwrap();
    assert!(phone.core.folder_questions().await.unwrap().iter().all(|q| q.path != resolved));
    let applied = pull(&paths).await;
    assert!(applied.snapshots >= 1, "{applied:?}");
    let (e, h) = policy::effective(&paths, &config, reins_desktop::now_unix());
    assert!(h.policy.as_ref().is_some_and(|p| p.scratch.contains(&resolved)), "signed for this computer");
    assert!(e.scratch.contains(&resolved), "{:?}", e.scratch);
    let started = Instant::now();
    let said = hook(&paths, &config, &format!("rm -rf {resolved}/old-build")).await;
    assert!(said.contains("allowed by Auto"), "{said}");
    assert!(started.elapsed() < Duration::from_secs(3), "decided here, not asked");

    // ---- removing it on the phone: the computer stops counting it at once ----
    let folders = phone.core.computer_folders(card.connection_id.clone()).await.unwrap();
    assert_eq!(folders.confirmed.iter().map(|f| f.path.clone()).collect::<Vec<_>>(), std::slice::from_ref(&resolved));
    phone.core.remove_computer_folder(card.connection_id.clone(), resolved.clone(), card.kind).await.unwrap();
    pull(&paths).await;
    let (e, _) = policy::effective(&paths, &config, reins_desktop::now_unix());
    assert!(!e.scratch.contains(&resolved), "{:?}", e.scratch);
}

/// The owner approved deleting in one folder three times. The computer suggests that folder as scratch, though no
/// configuration names it; the phone shows a card; nothing counts until the owner confirms it there (the owner
/// check). Then the snapshot signs it and the computer decides deletes in it here, still with no local entry.
#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn the_computer_suggests_a_scratch_folder_and_uses_it_once_confirmed() {
    use reins_core::autopilot::FolderKindChoice;
    use reins_desktop::{folders, suggest};

    reins_e2e::init_tls();
    let server = Server::start(20, 10).await;
    server.register(EMAIL).await;
    let phone = Phone::sign_in(&server.base, EMAIL).await;
    let (_state, paths) = logged_in_app(&server, &phone).await;
    phone.core.finish_auto_setup(Arc::new(Owner)).await.unwrap();
    // A real folder in the home folder, outside the standard roots (the computer looks at it before suggesting it).
    let home = reins_desktop::config::home_dir().unwrap();
    let dir = tempfile::Builder::new().prefix(".reins-e2e-suggested-").tempdir_in(&home).unwrap();
    let folder = folders::canonical(&dir.path().to_string_lossy(), "").unwrap();
    let mut config = Config {
        mode: Mode::Reins,
        listen: SocketAddr::from(([127, 0, 0, 1], 0)),
        ..Config::default()
    };
    config.guard.timeout_secs = 5;
    assert!(config.workspace.scratch.is_empty(), "the folder is in no configuration");

    // ---- nothing approved yet: nothing suggested ----
    let suggested = |paths: &Paths, config: &Config| {
        let all = suggest::suggestions(paths, config, reins_desktop::now_unix());
        all.into_iter().filter(|s| s.path == folder).collect::<Vec<_>>()
    };
    assert!(suggested(&paths, &config).is_empty());

    // ---- the owner approved three deletes in it on the phone (as the hooks log them) ----
    let log = journal::Journal::new(&paths);
    for name in ["unpacked", "old-build", "iso"] {
        let command = format!("rm -rf {folder}/{name}");
        let mut entry = journal::Entry::new(journal::Kind::Command, &format!("Claude Code wants to run: {command}"))
            .detail(Some(&format!("Command:\n{command}\n\nIn: {folder}")));
        entry.end(journal::Outcome::Approved, None);
        log.record(&entry);
    }
    let found = suggested(&paths, &config);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].proposable, "{found:?}");

    // ---- a suggestion makes nothing count: not here, not in what the computer proposes as its own ----
    phone.core.sync(0).await.unwrap();
    pull(&paths).await;
    let (e, _) = policy::effective(&paths, &config, reins_desktop::now_unix());
    assert!(matches!(e.gate, Gate::Matrix { .. }), "{:?}", e.gate);
    assert!(!e.scratch.contains(&folder), "{:?}", e.scratch);
    let proposal = folders::with_suggestions(folders::proposal(&config.workspace), &found);
    assert!(proposal.folders.iter().all(|f| f.path != folder), "{proposal:?}");
    assert_eq!(proposal.suggested_scratch, std::slice::from_ref(&folder));

    // ---- the phone shows it as a card; until the owner confirms it, the computer still asks ----
    folders::propose(&paths, &proposal).await.unwrap();
    let cards = phone.core.folder_questions().await.unwrap();
    let card = cards.iter().find(|q| q.path == folder).unwrap_or_else(|| panic!("no card: {cards:?}"));
    assert_eq!((card.kind, card.computer.as_str()), (FolderKindChoice::Scratch, "Laptop"));
    pull(&paths).await;
    let (e, h) = policy::effective(&paths, &config, reins_desktop::now_unix());
    assert!(h.policy.as_ref().is_some_and(|p| !p.scratch.contains(&folder)), "a card signs nothing");
    assert!(!e.scratch.contains(&folder), "{:?}", e.scratch);

    // ---- the owner confirms it: signed as scratch, and decided here without a local entry ----
    phone
        .core
        .confirm_computer_folder(card.connection_id.clone(), folder.clone(), card.kind, Arc::new(Owner))
        .await
        .unwrap();
    assert!(phone.core.folder_questions().await.unwrap().iter().all(|q| q.path != folder));
    let applied = pull(&paths).await;
    assert!(applied.snapshots >= 1, "{applied:?}");
    let (e, h) = policy::effective(&paths, &config, reins_desktop::now_unix());
    assert!(h.policy.as_ref().is_some_and(|p| p.scratch.contains(&folder)), "signed for this computer");
    assert!(e.scratch.contains(&folder), "{:?}", e.scratch);
    assert!(config.workspace.scratch.is_empty());
    let started = Instant::now();
    let said = hook(&paths, &config, &format!("rm -rf {folder}/next-build")).await;
    assert!(said.contains("allowed by Auto"), "{said}");
    assert!(started.elapsed() < Duration::from_secs(3), "decided here, not asked");
    // It is scratch now, so it is no longer suggested; the computer proposing it again makes no second card.
    assert!(suggested(&paths, &config).is_empty());
    folders::propose(&paths, &proposal).await.unwrap();
    assert!(phone.core.folder_questions().await.unwrap().iter().all(|q| q.path != folder));

    // ---- removing it on the phone: the computer stops counting it at once ----
    phone.core.remove_computer_folder(card.connection_id.clone(), folder.clone(), card.kind).await.unwrap();
    pull(&paths).await;
    let (e, _) = policy::effective(&paths, &config, reins_desktop::now_unix());
    assert!(!e.scratch.contains(&folder), "{:?}", e.scratch);
}
