//! A push the reviewer will decide later: real git through the daemon, decided by the phone through a mock Reins
//! server, against a `git http-backend` upstream. git is refused with the ticket (and exits non-zero); once approved,
//! the kept push goes upstream with the phone's credential, unless the remote moved meanwhile (then it is stale, and
//! nothing is pushed).

mod d1_mock;
mod proxy_support;

use std::sync::Arc;

use d1_mock::{Mock, Step, log_in};
use proxy_support::{Home, Proxy, TOKEN, Upstream, config, run_git};
use reins_desktop::config::Mode;
use reins_desktop::daemon::Options;
use reins_desktop::server::client::DesktopClient;
use reins_desktop::tickets::{self, TicketKind, TicketStore};

struct Setup {
    upstream: Upstream,
    mock: Mock,
    proxy: Proxy,
    home: Home,
    /// What main is upstream before the push.
    base: String,
}

/// A private repository upstream, a daemon whose phone is the mock, and a clone with one new commit on main.
async fn setup() -> Setup {
    let upstream = Upstream::start().await;
    upstream.create("me/app", true);
    let mock = Mock::start().await;
    mock.with(|s| TOKEN.clone_into(&mut s.git_token));
    let mut c = config(&upstream);
    c.mode = Mode::Reins;
    let proxy = Proxy::start(
        c,
        Options {
            authorizer: None,
            prompter: Arc::new(reins_desktop::auth::prompt::NoPrompter),
            harden: false,
            process_info: reins_desktop::runner::process_info,
        },
    )
    .await;
    log_in(&mock, &proxy.paths);
    let home = Home::new(proxy.addr());
    home.git_ok("", &["clone", "-q", "https://github.com/me/app.git", "app"]).await;
    let base = upstream.head("me/app", "main").unwrap();
    home.commit("app", "NOTES.md", b"later\n", "Notes").await;
    Setup {
        upstream,
        mock,
        proxy,
        home,
        base,
    }
}

/// Pushes main and checks git was told it is parked; returns the ticket.
async fn push_parked(s: &Setup) -> String {
    s.mock.plan(&[Step::Deferred, Step::Collect]);
    let run = s.home.git("app", &["push", "origin", "main"]).await;
    assert!(!run.ok, "git must not think it landed:\n{}", run.all());
    let store = TicketStore::load(&s.proxy.paths);
    let t = store.tickets().iter().find(|t| t.kind == TicketKind::Push).expect("a parked push");
    assert!(
        run.stderr.contains(&format!(
            "remote: Reins: parked for review (ticket {}). It will be pushed when approved; you can keep working.",
            t.ticket
        )),
        "{}",
        run.all()
    );
    assert_eq!(s.upstream.head("me/app", "main").as_deref(), Some(s.base.as_str()), "nothing pushed yet");
    assert_eq!(t.repo.as_deref(), Some("github.com/me/app"));
    assert_eq!(t.refs.len(), 1);
    assert_eq!((t.refs[0].0.as_str(), t.refs[0].2.as_str()), (s.base.as_str(), "refs/heads/main"));
    assert!(tickets::parked_file(&s.proxy.paths, &t.ticket).is_file());
    t.ticket.clone()
}

fn receive_packs(upstream: &Upstream) -> usize {
    upstream.seen().iter().filter(|r| r.method == "POST" && r.path.ends_with("/git-receive-pack")).count()
}

#[tokio::test]
async fn parked_push_is_rejected_then_pushed_on_approval() {
    let s = setup().await;
    let ticket = push_parked(&s).await;
    assert!(s.mock.with(|m| m.pushed.is_empty()), "nothing reported before it is pushed");
    let mine = s.home.git_ok("app", &["rev-parse", "HEAD"]).await.stdout.trim().to_owned();
    let client = DesktopClient::new(&s.proxy.paths).unwrap();
    assert_eq!(tickets::refresh(&s.proxy.paths, &client, true).await, Ok(1));
    assert_eq!(s.upstream.head("me/app", "main").as_deref(), Some(mine.as_str()), "pushed once approved");
    // With the phone's credential: the repository is private.
    let pushed = s.upstream.seen().into_iter().rev().find(|r| r.path.ends_with("/git-receive-pack")).unwrap();
    assert_eq!(pushed.status, 200);
    assert_eq!(pushed.authorization.as_deref(), Some(format!("Basic {}", proxy_support::token_basic()).as_str()));
    let mut store = TicketStore::load(&s.proxy.paths);
    assert_eq!(store.get(&ticket).unwrap().state, "done");
    assert!(!tickets::parked_file(&s.proxy.paths, &ticket).exists(), "the kept push is gone");
    assert!(s.mock.with(|m| m.stale.is_empty()));
    assert_eq!(s.mock.with(|m| m.pushed.clone()), std::slice::from_ref(&ticket), "reported pushed once it landed");
    assert_eq!(
        store.news_for("any")[0],
        format!("Reins: {ticket} (push to github.com/me/app: main (1 commit)) was approved and pushed.")
    );
    // Nothing more to do: another look pushes nothing again.
    let before = receive_packs(&s.upstream);
    assert_eq!(tickets::refresh(&s.proxy.paths, &client, true).await, Ok(0));
    assert_eq!(receive_packs(&s.upstream), before);
}

#[tokio::test]
async fn moved_remote_is_stale_not_pushed() {
    let s = setup().await;
    let ticket = push_parked(&s).await;
    // Someone else pushes to main meanwhile, straight to the repository.
    let other = s.upstream.dir.path().join("other");
    let bare = s.upstream.bare("me/app");
    run_git(s.upstream.dir.path(), &[], &["clone", "-q", bare.to_str().unwrap(), other.to_str().unwrap()]);
    std::fs::write(other.join("OTHER.md"), "theirs\n").unwrap();
    run_git(&other, &[], &["add", "."]);
    run_git(&other, &[], &["-c", "user.name=Other", "-c", "user.email=o@example.com", "commit", "-q", "-m", "Theirs"]);
    run_git(&other, &[], &["push", "-q", "origin", "main"]);
    let theirs = s.upstream.head("me/app", "main").unwrap();
    assert_ne!(theirs, s.base);

    let before = receive_packs(&s.upstream);
    let client = DesktopClient::new(&s.proxy.paths).unwrap();
    assert_eq!(tickets::refresh(&s.proxy.paths, &client, true).await, Ok(1));
    assert_eq!(s.upstream.head("me/app", "main").as_deref(), Some(theirs.as_str()), "theirs stays");
    assert_eq!(receive_packs(&s.upstream), before, "nothing was sent");
    let diff = format!("refs/heads/main moved from {} to {}", &s.base[..7], &theirs[..7]);
    assert_eq!(s.mock.with(|m| m.stale.clone()), [(ticket.clone(), diff.clone())]);
    assert!(s.mock.with(|m| m.pushed.is_empty()), "a stale push is never reported pushed");
    let mut store = TicketStore::load(&s.proxy.paths);
    let t = store.get(&ticket).unwrap();
    assert_eq!((t.state.as_str(), t.stale.as_deref()), ("stale", Some(diff.as_str())));
    assert!(!t.waits(), "a stale push is pushed again by git, not from here");
    assert!(!tickets::parked_file(&s.proxy.paths, &ticket).exists());
    assert_eq!(
        store.news_for("any")[0],
        format!("Reins: {ticket} (push to github.com/me/app: main (1 commit)) is stale: {diff}.")
    );
}

#[tokio::test]
async fn pushes_to_one_ref_go_in_ticket_order() {
    let s = setup().await;
    let first = push_parked(&s).await;
    // A second push of main, made after the first was parked: it builds on the first commit.
    let second_commit = s.home.commit("app", "MORE.md", b"more\n", "More").await;
    s.mock.plan(&[Step::Deferred, Step::Collect]);
    assert!(!s.home.git("app", &["push", "origin", "main"]).await.ok);
    let store = TicketStore::load(&s.proxy.paths);
    let second = store.tickets().iter().rfind(|t| t.kind == TicketKind::Push).unwrap().ticket.clone();
    assert_ne!(first, second);
    // Both approved: the first goes first; the second then finds main moved and is stale, though alone it would
    // have pushed both commits.
    let client = DesktopClient::new(&s.proxy.paths).unwrap();
    assert_eq!(tickets::refresh(&s.proxy.paths, &client, true).await, Ok(2));
    let store = TicketStore::load(&s.proxy.paths);
    assert_eq!(store.get(&first).unwrap().state, "done");
    assert_eq!(store.get(&second).unwrap().state, "stale");
    assert_ne!(s.upstream.head("me/app", "main").as_deref(), Some(second_commit.as_str()));
    assert!(!tickets::parked_file(&s.proxy.paths, &second).exists());
}

#[tokio::test]
async fn a_failed_parked_push_is_not_reported_pushed() {
    let s = setup().await;
    let ticket = push_parked(&s).await;
    // The phone's credential does not open the repository: the host refuses the push.
    s.mock.with(|m| "ghs-wrong".clone_into(&mut m.git_token));
    let client = DesktopClient::new(&s.proxy.paths).unwrap();
    assert_eq!(tickets::refresh(&s.proxy.paths, &client, true).await, Ok(1));
    assert_eq!(TicketStore::load(&s.proxy.paths).get(&ticket).unwrap().state, "failed");
    assert_eq!(s.upstream.head("me/app", "main").as_deref(), Some(s.base.as_str()));
    assert!(s.mock.with(|m| m.pushed.is_empty()), "a failed push is never reported pushed");
}

#[tokio::test]
async fn a_lost_pushed_report_is_sent_again() {
    let s = setup().await;
    let ticket = push_parked(&s).await;
    s.mock.with(|m| m.pushed_fail = 1);
    let client = DesktopClient::new(&s.proxy.paths).unwrap();
    tickets::refresh(&s.proxy.paths, &client, true).await.unwrap();
    assert_eq!(TicketStore::load(&s.proxy.paths).get(&ticket).unwrap().state, "done");
    assert!(s.mock.with(|m| m.pushed.is_empty()));
    assert!(TicketStore::load(&s.proxy.paths).needs_look(), "the report is still owed");
    // The next look reports it, once; then nothing is owed.
    let before = receive_packs(&s.upstream);
    tickets::refresh(&s.proxy.paths, &client, true).await.unwrap();
    assert_eq!(s.mock.with(|m| m.pushed.clone()), std::slice::from_ref(&ticket));
    assert_eq!(receive_packs(&s.upstream), before, "not pushed again");
    assert!(!TicketStore::load(&s.proxy.paths).needs_look());
    tickets::refresh(&s.proxy.paths, &client, true).await.unwrap();
    assert_eq!(s.mock.with(|m| m.pushed.len()), 1);
}

#[tokio::test]
async fn a_changed_signing_key_stops_parked_pushes_until_trusted() {
    let s = setup().await;
    let client = DesktopClient::new(&s.proxy.paths).unwrap();
    let key = |b: u8| data_encoding::BASE64URL_NOPAD.encode(&[b; 32]);
    s.mock.with(|m| m.signing_key = Some(key(1)));
    tickets::see_signing_key(&s.proxy.paths, &client).await.unwrap();
    let ticket = push_parked(&s).await;
    s.mock.with(|m| m.signing_key = Some(key(2)));
    tickets::refresh(&s.proxy.paths, &client, true).await.unwrap();
    assert_eq!(s.upstream.head("me/app", "main").as_deref(), Some(s.base.as_str()), "not pushed");
    let store = TicketStore::load(&s.proxy.paths);
    assert!(store.key_change().is_some());
    assert_eq!(store.get(&ticket).unwrap().state, "deferred");
    assert!(tickets::parked_file(&s.proxy.paths, &ticket).is_file(), "kept for when it is trusted");
    // Trusted: pushed on the next look.
    TicketStore::update(&s.proxy.paths, TicketStore::trust_key).unwrap();
    tickets::refresh(&s.proxy.paths, &client, true).await.unwrap();
    assert_eq!(TicketStore::load(&s.proxy.paths).get(&ticket).unwrap().state, "done");
    assert_ne!(s.upstream.head("me/app", "main").as_deref(), Some(s.base.as_str()));
}
