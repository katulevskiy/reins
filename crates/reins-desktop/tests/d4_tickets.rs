//! Deferred decisions through the hooks and `reins wait`: a question the reviewer will decide later is parked and the
//! agent told so; once approved the session hears it once, and the exact command goes through once (a pass bound to
//! its nonce, its digest and an hour).

mod d1_mock;

use d1_mock::{App, DEFERRED_SECS, Mock, Step, logged_in};
use reins_desktop::auth::prompt::NoPrompter;
use reins_desktop::config::Config;
use reins_desktop::harness::Harness;
use reins_desktop::hooks;
use reins_desktop::server::client::DesktopClient;
use reins_desktop::tickets::{self, LocalTicket, TicketKind, TicketStore};
use reins_proto::desktop::AskAnswer;
use serde_json::{Value, json};

const FORCE_PUSH: &str = "git push --force origin main";

fn input(session: &str, event: &str, command: Option<&str>) -> String {
    let mut v = json!({"session_id": session, "cwd": "/home/me/app", "hook_event_name": event});
    if let Some(c) = command {
        v["tool_name"] = json!("Bash");
        v["tool_input"] = json!({"command": c});
    }
    v.to_string()
}

async fn hook(app: &App, input: &str) -> Option<Value> {
    let (out, code, err) =
        hooks::run(Harness::ClaudeCode, input.as_bytes(), &app.paths, &Config::default(), &NoPrompter).await;
    assert_eq!((code, err), (0, None));
    out.map(|o| serde_json::from_str(&o).unwrap())
}

fn decision(out: &Value) -> (&str, &str) {
    let h = &out["hookSpecificOutput"];
    (h["permissionDecision"].as_str().unwrap(), h["permissionDecisionReason"].as_str().unwrap())
}

fn store(app: &App) -> TicketStore {
    TicketStore::load(&app.paths)
}

/// Parks `command` for session `s-one`; returns its ticket.
async fn parked(mock: &Mock, app: &App, command: &str, then: &[Step]) -> String {
    let mut steps = vec![Step::Deferred];
    steps.extend_from_slice(then);
    mock.plan(&steps);
    let out = hook(app, &input("s-one", "PreToolUse", Some(command))).await.unwrap();
    assert_eq!(decision(&out).0, "deny", "{out}");
    let s = store(app);
    s.tickets().last().unwrap().ticket.clone()
}

#[tokio::test]
async fn a_deferred_hook_answer_parks_and_tells_the_agent() {
    let mock = Mock::start().await;
    let app = logged_in(&mock, 3600);
    mock.plan(&[Step::Deferred]);
    let out = hook(&app, &input("s-one", "PreToolUse", Some(FORCE_PUSH))).await.unwrap();
    let asked = mock.with(|s| s.calls.last().cloned()).unwrap();
    let (decision, reason) = decision(&out);
    assert_eq!(decision, "deny");
    let s = store(&app);
    let t = s.get(&s.tickets()[0].ticket).unwrap();
    assert!(t.ticket.starts_with("rt_req-"), "{t:?}");
    assert_eq!(
        reason,
        format!(
            "Parked for review (ticket {}, expected within 4 h). Continue with other work; when it's approved, re-run \
             this exact command.",
            t.ticket
        )
    );
    assert_eq!(out["systemMessage"], reason, "the user sees it too");
    // The question carried the command's digest, which the ticket keeps with the question's nonce.
    assert_eq!(asked["digest"].as_str(), Some(t.digest.as_str()));
    assert_eq!(t.digest.len(), 64);
    assert_eq!(asked["nonce"].as_str(), Some(t.nonce.as_str()));
    assert_eq!(t.kind, TicketKind::Hook);
    assert_eq!(t.session.as_deref(), Some("s-one"));
    assert_eq!(t.summary, FORCE_PUSH);
    assert_eq!(t.state, "deferred");
    let now = reins_desktop::now_unix();
    assert!((t.expected_by - now - DEFERRED_SECS).abs() < 10, "{}", t.expected_by - now);
    // An Instant question does not ask to be deferred: the server (or the reviewer) parks it.
    assert_eq!(mock.with(|s| s.defers.clone()), [false]);
}

#[tokio::test]
async fn news_is_injected_once_per_session() {
    let mock = Mock::start().await;
    let app = logged_in(&mock, 3600);
    let ticket = parked(&mock, &app, FORCE_PUSH, &[Step::Collect]).await;
    // Another session: the hook collects the approval, but the news is not for it.
    assert_eq!(hook(&app, &input("s-two", "UserPromptSubmit", None)).await, None);
    assert_eq!(store(&app).get(&ticket).unwrap().state, "approved");
    let out = hook(&app, &input("s-one", "UserPromptSubmit", None)).await.unwrap();
    assert_eq!(
        out,
        json!({"hookSpecificOutput": {"hookEventName": "UserPromptSubmit", "additionalContext":
            format!("Reins: {ticket} ({FORCE_PUSH}) was approved — re-run it now.")}})
    );
    // Told once.
    assert_eq!(hook(&app, &input("s-one", "UserPromptSubmit", None)).await, None);
    assert_eq!(hook(&app, &input("s-one", "PreToolUse", Some("ls -la"))).await, None);

    // News also rides on a PreToolUse answer, beside its decision.
    let other = parked(&mock, &app, "git push --force origin next", &[Step::Denied(Some("Not on main."))]).await;
    mock.plan(&[Step::Approve]);
    let out = hook(&app, &input("s-one", "PreToolUse", Some("rm -rf build"))).await.unwrap();
    assert_eq!(decision(&out).0, "allow");
    assert_eq!(
        out["hookSpecificOutput"]["additionalContext"],
        "Reins: ".to_owned() + &other + " (git push --force origin next) was denied: Not on main."
    );
}

#[tokio::test]
async fn a_pass_works_once_for_the_same_command_only() {
    let mock = Mock::start().await;
    let app = logged_in(&mock, 3600);
    let ticket = parked(&mock, &app, FORCE_PUSH, &[Step::Collect]).await;
    let client = DesktopClient::new(&app.paths).unwrap();
    assert_eq!(tickets::refresh(&app.paths, &client, false).await, Ok(1));
    assert_eq!(store(&app).passes().len(), 1);
    let asked = || mock.with(|s| s.calls.len());

    // Another command is asked about as usual.
    let before = asked();
    mock.plan(&[Step::Denied(None)]);
    let out = hook(&app, &input("s-one", "PreToolUse", Some("git push --force origin other"))).await.unwrap();
    assert_eq!(decision(&out).0, "deny");
    assert_eq!(asked(), before + 1);
    // The same command elsewhere is another command.
    let elsewhere = input("s-one", "PreToolUse", Some(FORCE_PUSH)).replace("/home/me/app", "/home/me/other");
    mock.plan(&[Step::Denied(None)]);
    assert_eq!(decision(&hook(&app, &elsewhere).await.unwrap()).0, "deny");
    assert_eq!(asked(), before + 2);

    // The exact command goes through without asking, once, in the session that asked.
    let out = hook(&app, &input("s-one", "PreToolUse", Some(FORCE_PUSH))).await.unwrap();
    let (d, r) = decision(&out);
    // With P3's nets the passed push also gets its backup refs: "… Reins keeps a copy of what it deletes."
    assert!(d == "allow" && r.starts_with(&format!("Approved earlier (ticket {ticket})")), "{out}");
    assert_eq!(asked(), before + 2);
    mock.plan(&[Step::Denied(Some("Once was enough."))]);
    let out = hook(&app, &input("s-one", "PreToolUse", Some(FORCE_PUSH))).await.unwrap();
    assert_eq!(decision(&out).0, "deny");
    assert!(decision(&out).1.contains("Once was enough."));
    assert_eq!(asked(), before + 3);
    assert!(store(&app).passes().is_empty());
    // The pass was logged as an approval.
    let log = reins_desktop::journal::read(&app.paths, 20, 3_600);
    assert!(
        log.iter().any(|e| e.reason.as_deref() == Some(format!("Approved earlier (ticket {ticket})").as_str())),
        "{log:?}"
    );
}

/// Changes a good answer into one that must be refused.
type Tamper = fn(&mut AskAnswer);

fn local(ticket: &str, kind: TicketKind) -> LocalTicket {
    LocalTicket {
        ticket: ticket.to_owned(),
        kind,
        session: Some("s".to_owned()),
        summary: "make deploy".to_owned(),
        digest: "d".repeat(64),
        nonce: "nonce-1".to_owned(),
        created_at: reins_desktop::now_unix(),
        expected_by: 0,
        state: "deferred".to_owned(),
        told: None,
        repo: None,
        refs: Vec::new(),
        note: None,
        stale: None,
        push: None,
    }
}

#[test]
fn a_pass_needs_the_right_nonce_and_digest_and_expires_after_an_hour() {
    let dir = tempfile::tempdir().unwrap();
    let paths = reins_desktop::config::Paths::under(dir.path());
    let now = 1_800_000_000;
    let good = AskAnswer {
        v: 1,
        nonce: "nonce-1".to_owned(),
        approved: true,
        digest: Some("d".repeat(64)),
        expires_at: Some(now + 7_200),
    };
    TicketStore::update(&paths, |s| {
        s.add(local("rt_a", TicketKind::Hook));
        s.add(local("rt_push", TicketKind::Push));
    })
    .unwrap();
    let refused: [(&str, Tamper); 6] = [
        ("another nonce", |a| a.nonce = "replayed".to_owned()),
        ("another digest", |a| a.digest = Some("e".repeat(64))),
        ("no digest", |a| a.digest = None),
        ("not approved", |a| a.approved = false),
        ("no expiry", |a| a.expires_at = None),
        ("expired", |a| a.expires_at = Some(1_800_000_000)),
    ];
    for (why, change) in refused {
        let mut a = good.clone();
        change(&mut a);
        assert!(!TicketStore::update(&paths, |s| s.grant_pass("rt_a", &a, now)).unwrap(), "{why}");
    }
    assert!(!TicketStore::update(&paths, |s| s.grant_pass("rt_unknown", &good, now)).unwrap());
    assert!(!TicketStore::update(&paths, |s| s.grant_pass("rt_push", &good, now)).unwrap(), "a push gets no pass");
    let mut s = TicketStore::load(&paths);
    assert!(s.passes().is_empty());

    assert!(s.grant_pass("rt_a", &good, now));
    assert_eq!(s.passes()[0].expires_at, now + 3_600, "at most an hour, whatever the answer says");
    assert_eq!(s.get("rt_a").unwrap().state, "approved");
    assert!(!s.grant_pass("rt_a", &good, now), "one pass per ticket");
    assert_eq!(s.take_pass(&"d".repeat(64), Some("other"), now + 10), None, "another session");
    assert_eq!(s.take_pass(&"d".repeat(64), None, now + 10), None, "reins ask cannot use a hook's pass");
    assert_eq!(s.take_pass(&"d".repeat(64), Some("s"), now + 3_600), None, "expired after the hour");

    let mut s = TicketStore::default();
    s.add(local("rt_b", TicketKind::Ask));
    assert!(s.grant_pass("rt_b", &good, now));
    assert_eq!(s.take_pass(&"e".repeat(64), None, now + 10), None, "another command");
    // A `reins ask` pass carries no session: no hook can use it, only `reins ask`.
    assert_eq!(s.take_pass(&"d".repeat(64), Some("s"), now + 10), None, "a hook cannot use an ask's pass");
    assert_eq!(s.take_pass(&"d".repeat(64), None, now + 10).unwrap().ticket, "rt_b");
    assert_eq!(s.take_pass(&"d".repeat(64), None, now + 11), None, "used once");
    // A hook's ticket with no session gets no pass at all.
    let mut nameless = local("rt_c", TicketKind::Hook);
    nameless.session = None;
    s.add(nameless);
    assert!(!s.grant_pass("rt_c", &good, now));
}

#[tokio::test]
async fn stop_hook_blocks_only_with_news() {
    let mock = Mock::start().await;
    let app = logged_in(&mock, 3600);
    assert_eq!(hook(&app, &input("s-one", "Stop", None)).await, None);
    let ticket = parked(&mock, &app, FORCE_PUSH, &[]).await;
    assert_eq!(hook(&app, &input("s-one", "Stop", None)).await, None, "still waiting: nothing to say");
    mock.decide(&ticket, &[Step::Closed("expired")]);
    let out = hook(&app, &input("s-one", "Stop", None)).await.unwrap();
    assert_eq!(
        out,
        json!({"decision": "block", "reason": format!("Reins: {ticket} ({FORCE_PUSH}) expired, not approved.")})
    );
    assert_eq!(hook(&app, &input("s-one", "Stop", None)).await, None);
    // Other harnesses hear nothing on events they do not have.
    let (out, code, _) = hooks::run(
        Harness::Codex,
        input("s-one", "Stop", None).as_bytes(),
        &app.paths,
        &Config::default(),
        &NoPrompter,
    )
    .await;
    assert_eq!((out, code), (None, 0));
}

async fn reins(app: &App, args: &[&str]) -> (u8, String, String) {
    let home = app.dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let out = tokio::process::Command::new(env!("CARGO_BIN_EXE_reins"))
        .args(args)
        .envs(reins_desktop::config::home_env(&home))
        .env("REINS_CONFIG_DIR", &app.paths.config_dir)
        .env("REINS_STATE_DIR", &app.paths.state_dir)
        .output()
        .await
        .unwrap();
    (
        u8::try_from(out.status.code().unwrap()).unwrap(),
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

#[tokio::test]
async fn reins_wait_exit_codes() {
    let mock = Mock::start().await;
    let app = logged_in(&mock, 3600);
    let approved = parked(&mock, &app, "git push --force origin a", &[Step::Collect]).await;
    let denied = parked(&mock, &app, "git push --force origin b", &[Step::Denied(Some("No."))]).await;
    let expired = parked(&mock, &app, "git push --force origin c", &[Step::Closed("expired")]).await;
    let cancelled = parked(&mock, &app, "git push --force origin d", &[Step::Closed("cancelled")]).await;
    let waiting = parked(&mock, &app, "git push --force origin e", &[]).await;

    let (code, _, err) = reins(&app, &["wait", &approved, "--timeout", "10"]).await;
    assert_eq!(code, 0, "{err}");
    assert_eq!(store(&app).passes().len(), 1, "waiting collected the approval");
    let (code, _, err) = reins(&app, &["wait", &denied, "--timeout", "10"]).await;
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("denied: No."), "{err}");
    for t in [&expired, &cancelled] {
        let (code, _, err) = reins(&app, &["wait", t, "--timeout", "10"]).await;
        assert_eq!(code, 2, "{t}: {err}");
    }
    let started = std::time::Instant::now();
    let (code, _, err) = reins(&app, &["wait", &waiting, "--timeout", "1"]).await;
    assert_eq!(code, 2, "{err}");
    assert!(err.contains("not decided within 1 s"), "{err}");
    assert!(started.elapsed() < std::time::Duration::from_secs(10));
    // A ticket the server does not know, and something that is no ticket.
    assert_eq!(reins(&app, &["wait", "rt_nope", "--timeout", "5"]).await.0, 2);
    assert_eq!(reins(&app, &["wait", "a/b"]).await.0, 2);

    // `reins tickets` lists them with where they stand.
    let (code, out, err) = reins(&app, &["tickets"]).await;
    assert_eq!(code, 0, "{err}");
    for (t, state, branch) in [
        (&approved, "approved", "a"),
        (&denied, "denied", "b"),
        (&expired, "expired", "c"),
        (&waiting, "deferred", "e"),
    ] {
        let line = out.lines().find(|l| l.starts_with(t.as_str())).unwrap_or_else(|| panic!("{t}: {out}"));
        assert!(line.contains(state) && line.contains(&format!("git push --force origin {branch}")), "{line}");
    }
    assert!(out.lines().any(|l| l.starts_with(waiting.as_str()) && l.contains("in 4 h")), "{out}");

    // `reins ask --defer` parks at once: exit 3, the ticket on stdout; once approved, the same question is yes once.
    mock.plan(&[Step::Deferred, Step::Collect]);
    let (code, out, err) = reins(&app, &["ask", "Deploy?", "--detail", "make deploy", "--defer"]).await;
    assert_eq!(code, 3, "{err}");
    let ask = out.trim().to_owned();
    assert!(ask.starts_with("rt_req-"), "{out}");
    assert!(mock.with(|s| *s.defers.last().unwrap()), "asked to be deferred");
    assert_eq!(store(&app).get(&ask).unwrap().kind, TicketKind::Ask);
    assert_eq!(reins(&app, &["wait", &ask, "--timeout", "10"]).await.0, 0);
    let calls = mock.with(|s| s.calls.len());
    let (code, _, err) = reins(&app, &["ask", "Deploy?", "--detail", "make deploy"]).await;
    assert_eq!(code, 0, "{err}");
    assert!(err.contains(&format!("approved earlier, ticket {ask}")), "{err}");
    assert_eq!(mock.with(|s| s.calls.len()), calls, "not asked again");
}

#[tokio::test]
async fn genuine_sealed_approval_exits_0() {
    let mock = Mock::start().await;
    let app = logged_in(&mock, 3600);
    let ticket = parked(&mock, &app, FORCE_PUSH, &[Step::Collect]).await;
    let (code, _, err) = reins(&app, &["wait", &ticket, "--timeout", "10"]).await;
    assert_eq!(code, 0, "{err}");
    let s = store(&app);
    assert_eq!(s.get(&ticket).unwrap().state, "approved");
    assert_eq!(s.passes().len(), 1);
}

#[tokio::test]
async fn unknown_ticket_with_a_server_result_exits_2() {
    let mock = Mock::start().await;
    let first = logged_in(&mock, 3600);
    let ticket = parked(&mock, &first, FORCE_PUSH, &[Step::Collect]).await;
    // Another app, same server: the server would answer with an approval, but not one this app asked for.
    let other = logged_in(&mock, 3600);
    mock.with(|s| s.pinned_key = Some(first.identity.public_key()));
    let (code, _, err) = reins(&other, &["wait", &ticket, "--timeout", "5"]).await;
    assert_eq!(code, 2, "{err}");
    assert!(err.contains("not a ticket of this computer"), "{err}");
    assert!(store(&other).passes().is_empty());
}

#[tokio::test]
async fn forged_answer_with_wrong_digest_is_not_approved() {
    let mock = Mock::start().await;
    let app = logged_in(&mock, 3600);
    let forged = parked(&mock, &app, FORCE_PUSH, &[Step::CollectOtherDigest]).await;
    let (code, _, err) = reins(&app, &["wait", &forged, "--timeout", "5"]).await;
    assert_eq!(code, 2, "{err}");
    let mut s = store(&app);
    assert!(s.passes().is_empty(), "no pass from an answer for another command");
    assert_eq!(s.get(&forged).unwrap().state, "failed");
    assert!(s.news_for("s-one").iter().all(|n| !n.contains("approved")), "never told it was approved");
    // The command is asked about again, not let through.
    let before = mock.with(|m| m.calls.len());
    mock.plan(&[Step::Denied(None)]);
    let out = hook(&app, &input("s-one", "PreToolUse", Some(FORCE_PUSH))).await.unwrap();
    assert_eq!(decision(&out).0, "deny");
    assert_eq!(mock.with(|m| m.calls.len()), before + 1);

    // A server that says the ticket is done, approved in all but proof: not approved either.
    let done = parked(&mock, &app, "git push --force origin x", &[Step::Closed("done")]).await;
    let (code, _, err) = reins(&app, &["wait", &done, "--timeout", "5"]).await;
    assert_eq!(code, 2, "{err}");
    let mut s = store(&app);
    assert_eq!(s.get(&done).unwrap().state, "unknown");
    assert!(s.passes().is_empty());
    assert!(s.news_for("s-one").iter().all(|n| !n.contains("approved")));
}

#[tokio::test]
async fn commands_equal_up_to_the_display_limit_get_different_digests() {
    let mock = Mock::start().await;
    let app = logged_in(&mock, 3600);
    let shared = format!("rm -rf ./build {}", "x".repeat(400));
    let (safe, other) = (format!("{shared} && echo done"), format!("{shared} && rm -rf ~"));
    let ticket = parked(&mock, &app, &safe, &[Step::Collect]).await;
    let client = DesktopClient::new(&app.paths).unwrap();
    assert_eq!(tickets::refresh(&app.paths, &client, false).await, Ok(1));
    let digest = store(&app).get(&ticket).unwrap().digest.clone();
    // The look-alike is asked about: the pass is not for it.
    let before = mock.with(|m| m.calls.len());
    mock.plan(&[Step::Denied(None)]);
    let out = hook(&app, &input("s-one", "PreToolUse", Some(&other))).await.unwrap();
    assert_eq!(decision(&out).0, "deny");
    assert_eq!(mock.with(|m| m.calls.len()), before + 1);
    assert_ne!(mock.with(|m| m.calls.last().unwrap()["digest"].clone()), json!(digest));
    assert_eq!(store(&app).passes().len(), 1, "the pass is still there for its own command");
    let out = hook(&app, &input("s-one", "PreToolUse", Some(&safe))).await.unwrap();
    let (d, r) = decision(&out);
    // With P3's nets the passed push also gets its backup refs: "… Reins keeps a copy of what it deletes."
    assert!(d == "allow" && r.starts_with(&format!("Approved earlier (ticket {ticket})")), "{out}");
}

#[tokio::test]
async fn a_command_cut_to_reach_the_phone_is_never_parked() {
    let mock = Mock::start().await;
    let app = logged_in(&mock, 3600);
    let long = format!("rm -rf ./build {} && rm -rf ~", "x".repeat(9_000));
    mock.plan(&[Step::Deferred]);
    let out = hook(&app, &input("s-one", "PreToolUse", Some(&long))).await.unwrap();
    let (verdict, reason) = decision(&out);
    assert_eq!(verdict, "deny");
    assert!(!reason.starts_with("Parked for review"), "{reason}");
    assert_eq!(mock.with(|m| m.calls.last().unwrap()["digest"].clone()), Value::Null, "no digest to bind a pass to");
    assert!(store(&app).tickets().is_empty());
}

#[tokio::test]
async fn the_exact_same_command_rerun_passes_once() {
    let mock = Mock::start().await;
    let app = logged_in(&mock, 3600);
    let ticket = parked(&mock, &app, FORCE_PUSH, &[Step::Collect]).await;
    let client = DesktopClient::new(&app.paths).unwrap();
    assert_eq!(tickets::refresh(&app.paths, &client, false).await, Ok(1));
    let before = mock.with(|m| m.calls.len());
    let out = hook(&app, &input("s-one", "PreToolUse", Some(FORCE_PUSH))).await.unwrap();
    let (d, r) = decision(&out);
    // With P3's nets the passed push also gets its backup refs: "… Reins keeps a copy of what it deletes."
    assert!(d == "allow" && r.starts_with(&format!("Approved earlier (ticket {ticket})")), "{out}");
    assert_eq!(mock.with(|m| m.calls.len()), before, "not asked");
    let netted = out["hookSpecificOutput"]["updatedInput"]["command"].as_str().unwrap_or_default();
    assert!(netted.contains("net run"), "the pass runs the push with its net: {out}");
    mock.plan(&[Step::Denied(None)]);
    let out = hook(&app, &input("s-one", "PreToolUse", Some(FORCE_PUSH))).await.unwrap();
    assert_eq!(decision(&out).0, "deny");
    assert_eq!(mock.with(|m| m.calls.len()), before + 1, "asked again: the pass was used");
}

#[tokio::test]
async fn a_pass_from_another_session_is_not_honoured() {
    let mock = Mock::start().await;
    let app = logged_in(&mock, 3600);
    let ticket = parked(&mock, &app, FORCE_PUSH, &[Step::Collect]).await;
    let client = DesktopClient::new(&app.paths).unwrap();
    assert_eq!(tickets::refresh(&app.paths, &client, false).await, Ok(1));
    assert_eq!(store(&app).passes()[0].session.as_deref(), Some("s-one"));
    // The same command in another session is asked about, and the pass stays for its own session.
    let before = mock.with(|m| m.calls.len());
    mock.plan(&[Step::Denied(None)]);
    let out = hook(&app, &input("s-two", "PreToolUse", Some(FORCE_PUSH))).await.unwrap();
    assert_eq!(decision(&out).0, "deny");
    assert_eq!(mock.with(|m| m.calls.len()), before + 1);
    assert_eq!(store(&app).passes().len(), 1);
    let out = hook(&app, &input("s-one", "PreToolUse", Some(FORCE_PUSH))).await.unwrap();
    let (d, r) = decision(&out);
    // With P3's nets the passed push also gets its backup refs: "… Reins keeps a copy of what it deletes."
    assert!(d == "allow" && r.starts_with(&format!("Approved earlier (ticket {ticket})")), "{out}");

    // A hook call that names no session is never parked: there would be nothing to bind its pass to.
    let nameless = json!({"cwd": "/home/me/app", "hook_event_name": "PreToolUse", "tool_name": "Bash",
        "tool_input": {"command": "git push --force origin nameless"}})
    .to_string();
    mock.plan(&[Step::Deferred]);
    let out = hook(&app, &nameless).await.unwrap();
    assert!(!decision(&out).1.starts_with("Parked for review"), "{out}");
    assert_eq!(mock.with(|m| m.calls.last().unwrap()["digest"].clone()), Value::Null);
    assert!(store(&app).tickets().iter().all(|t| t.summary != "git push --force origin nameless"));
}

fn key(byte: u8) -> String {
    data_encoding::BASE64URL_NOPAD.encode(&[byte; 32])
}

fn fingerprint(key: &str) -> String {
    reins_proto::ticket::signing_key_fingerprint(key).unwrap()
}

#[tokio::test]
async fn the_phones_signing_key_is_pinned_and_a_change_refused_until_trusted() {
    let mock = Mock::start().await;
    let app = logged_in(&mock, 3600);
    let (first, second) = (key(7), key(9));
    mock.with(|s| s.signing_key = Some(first.clone()));
    let ticket = parked(&mock, &app, FORCE_PUSH, &[]).await;

    // The first key seen is pinned, and shown.
    let (code, out, err) = reins(&app, &["tickets"]).await;
    assert_eq!(code, 0, "{err}");
    assert_eq!(store(&app).signing_key(), Some(first.as_str()));
    assert!(out.lines().any(|l| l == format!("Phone key {}", fingerprint(&first))), "{out}");

    // The server names another key, and the phone's approval is ready to collect: refused, nothing collected.
    mock.with(|s| s.signing_key = Some(second.clone()));
    mock.decide(&ticket, &[Step::Collect]);
    let polled = || mock.with(|s| s.polls.iter().filter(|p| **p == ticket).count());
    let before = polled();
    let client = DesktopClient::new(&app.paths).unwrap();
    tickets::refresh(&app.paths, &client, true).await.unwrap();
    let changed = format!(
        "The phone's signing key changed (was {}, now {}). Compare with the phone (the connection's More), then \
         run: reins tickets --trust-key",
        fingerprint(&first),
        fingerprint(&second)
    );
    for args in [&["tickets"][..], &["wait", &ticket, "--timeout", "3"]] {
        let (code, _, err) = reins(&app, args).await;
        assert_eq!(code, 2, "{args:?}: {err}");
        assert!(err.contains(&changed), "{args:?}: {err}");
    }
    assert_eq!(polled(), before, "no collect while the key is not trusted");
    let s = store(&app);
    assert!(s.passes().is_empty());
    assert_eq!(s.get(&ticket).unwrap().state, "deferred");
    assert_eq!(s.signing_key(), Some(first.as_str()), "still the first until trusted");

    // Trusted: the new key is pinned and the approval collected.
    let (code, out, err) = reins(&app, &["tickets", "--trust-key"]).await;
    assert_eq!(code, 0, "{err}");
    assert!(out.contains(&format!("Phone key {}", fingerprint(&second))), "{out}");
    assert_eq!(store(&app).signing_key(), Some(second.as_str()));
    let (code, _, err) = reins(&app, &["wait", &ticket, "--timeout", "10"]).await;
    assert_eq!(code, 0, "{err}");
    assert_eq!(store(&app).passes().len(), 1);
    // Nothing more to trust.
    let (code, out, _) = reins(&app, &["tickets", "--trust-key"]).await;
    assert_eq!(code, 0);
    assert!(out.contains("has not changed"), "{out}");
}
