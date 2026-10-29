//! The coding-agent hook end to end, the way a user meets it: `scripts/install.sh` puts `reins` in a stand-in home,
//! `reins harness add claude-code` registers its PreToolUse hook, `reins login` pairs it with the phone core through
//! the real server, and the registered hook command is run with the JSON a coding agent in auto mode sends for a Bash
//! call running `git push --force`. The call is held until the phone answers: an approval lets it run, a denial refuses
//! it with the reason, no answer refuses it (or hands it back to the agent's own prompt), and a harmless command is
//! never asked about.
//!
//! The agent itself is not run: the test runs exactly what it would (`sh -c <command>`, the payload on stdin).
#![cfg(unix)]

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::OnceLock;
use std::time::Duration;

use reins_core::ApprovalChoice;
use reins_desktop::config::{Paths, home_env};
use reins_desktop::identity::Identity;
use reins_e2e::{Phone, Server};
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;

const EMAIL: &str = "dev@example.com";
const FORCE_PUSH: &str = "git push --force origin main";
/// The guard's default wait for an answer (`[guard] timeout_secs`).
const GUARD_SECS: u64 = 120;
/// The shortest wait the guard accepts (`timeout_secs` is 5 to 3600), for the questions nobody answers.
const NO_ANSWER_SECS: u64 = 5;

/// The installer under test, named from the repository root so CI's change planner (scripts/ci/reuse.py) follows it.
const INSTALL_SH: &str = "scripts/install.sh";

fn install_script() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(INSTALL_SH).canonicalize().expect("scripts/install.sh")
}

/// The repository root, two levels above the installer.
fn repo_root() -> PathBuf {
    install_script().parent().and_then(Path::parent).expect("repo root").to_path_buf()
}

/// `target/debug/reins` (or `REINS_TEST_DESKTOP_BINARY`), built once per test process.
fn reins_binary() -> &'static Path {
    static BIN: OnceLock<PathBuf> = OnceLock::new();
    BIN.get_or_init(|| {
        // CI's Nextest run has already built it (reins-desktop's own integration tests need it).
        if let Some(binary) = std::env::var_os("REINS_TEST_DESKTOP_BINARY").map(PathBuf::from).filter(|b| b.is_file()) {
            return binary;
        }
        let root = repo_root();
        let status = Command::new("cargo")
            .args(["build", "-p", "reins-desktop", "--bin", "reins"])
            .current_dir(&root)
            .status()
            .expect("cargo build");
        assert!(status.success(), "building reins failed");
        let target = std::env::var_os("CARGO_TARGET_DIR").map_or_else(|| root.join("target"), PathBuf::from);
        target.join("debug/reins")
    })
}

fn sha256_hex(file: &Path) -> String {
    let mut f = std::fs::File::open(file).unwrap();
    let mut ctx = ring::digest::Context::new(&ring::digest::SHA256);
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf).unwrap();
        if n == 0 {
            break;
        }
        ctx.update(&buf[..n]);
    }
    data_encoding::HEXLOWER.encode(ctx.finish().as_ref())
}

fn in_home(command: &mut Command, home: &Path) {
    command.envs(home_env(home)).env_remove("REINS_CONFIG_DIR").env_remove("REINS_STATE_DIR");
}

/// `curl -fsSL <site>/install.sh | sh` against a local release list holding the program under test.
fn install(home: &Path, releases: &Path) -> PathBuf {
    let os = if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    };
    let platform = format!("{os}-{}", std::env::consts::ARCH);
    std::fs::create_dir_all(releases.join("files")).unwrap();
    std::fs::copy(reins_binary(), releases.join("files/reins")).unwrap();
    let sha = sha256_hex(&releases.join("files/reins"));
    std::fs::write(releases.join(format!("latest-{platform}.txt")), format!("reins {sha} 0.0.0-test e2e\n")).unwrap();
    let mut sh = Command::new("sh");
    sh.arg(install_script());
    in_home(&mut sh, home);
    let out = sh
        .env("REINS_RELEASES", format!("file://{}", releases.display()))
        .env("REINS_APP", "no")
        .env("REINS_NO_SETUP", "1")
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let said = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "install.sh: {said}{}", String::from_utf8_lossy(&out.stderr));
    let installed = home.join(".local/bin/reins");
    assert!(said.contains(&format!("at {}", installed.display())), "{said}");
    assert_eq!(sha256_hex(&installed), sha, "the installed program is the published one");
    installed
}

fn reins(home: &Path, args: &[&str]) -> Output {
    let mut command = Command::new(home.join(".local/bin/reins"));
    in_home(&mut command, home);
    let out = command.args(args).stdin(Stdio::null()).output().unwrap();
    assert!(out.status.success(), "reins {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    out
}

/// Plays the browser of `reins login`: the email form, the code the phone must pick, the redirect to the app.
async fn browse(authorize: String, code_out: tokio::sync::oneshot::Sender<u8>) {
    let between = |haystack: &str, before: &str, after: &str| -> Option<String> {
        let start = haystack.find(before)? + before.len();
        let end = haystack[start..].find(after)?;
        Some(haystack[start..start + end].to_owned())
    };
    let http = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().unwrap();
    let page = http.get(&authorize).send().await.unwrap().text().await.unwrap();
    let session = between(&page, "name=\"session\" value=\"", "\"").expect("session field");
    let u = url::Url::parse(&authorize).unwrap();
    let origin = format!("{}://{}:{}", u.scheme(), u.host_str().unwrap(), u.port_or_known_default().unwrap());
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

/// What the agent sends on stdin for a Bash tool call (its documented PreToolUse input), in auto mode.
fn bash_call(home: &Path, command: &str) -> Value {
    json!({
        "session_id": "e2e-session",
        "transcript_path": home.join(".claude/projects/app/e2e-session.jsonl"),
        "cwd": home.join("project"),
        "permission_mode": "auto",
        "hook_event_name": "PreToolUse",
        "tool_name": "Bash",
        "tool_input": {
            "command": command,
            "description": "Push the branch",
            "timeout": 120_000,
            "run_in_background": false,
        },
        "tool_use_id": "toolu_01e2e",
    })
}

/// The hook command as the agent runs it: `sh -c <command>`, the call's JSON on stdin.
async fn fire(home: &Path, hook: &str, call: &Value) -> tokio::process::Child {
    let mut command = tokio::process::Command::new("sh");
    command.envs(home_env(home)).env_remove("REINS_CONFIG_DIR").env_remove("REINS_STATE_DIR");
    let mut child = command
        .args(["-c", hook])
        .current_dir(home.join("project"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(call.to_string().as_bytes()).await.unwrap();
    drop(stdin);
    child
}

/// The decision the agent reads: exit code, `permissionDecision`, its reason and the message shown to the user.
struct Decision {
    code: i32,
    decision: Option<String>,
    reason: String,
    message: String,
    stdout: String,
}

async fn decision(child: tokio::process::Child) -> Decision {
    let out = tokio::time::timeout(Duration::from_secs(GUARD_SECS + 30), child.wait_with_output())
        .await
        .expect("the hook answers within the agent's own limit")
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    let v: Value = if stdout.is_empty() {
        Value::Null
    } else {
        serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("{e}: {stdout}"))
    };
    let at = |p: &str| v.pointer(p).and_then(Value::as_str).map(str::to_owned);
    Decision {
        code: out.status.code().expect("an exit code"),
        decision: at("/hookSpecificOutput/permissionDecision"),
        reason: at("/hookSpecificOutput/permissionDecisionReason").unwrap_or_default(),
        message: at("/systemMessage").unwrap_or_default(),
        stdout,
    }
}

fn set_guard(home: &Path, timeout_secs: u64, on_no_answer: &str) {
    let dir = home.join(".config/reins");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("config.toml"),
        format!("[guard]\ntimeout_secs = {timeout_secs}\non_no_answer = \"{on_no_answer}\"\n"),
    )
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[allow(clippy::too_many_lines)]
async fn a_force_push_from_the_coding_agent_waits_for_the_phone() {
    reins_e2e::init_tls();
    let home_dir = tempfile::tempdir().unwrap();
    let home = home_dir.path();
    let releases = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(home.join("project")).unwrap();

    // ---- install: the published installer, then the agent's settings ----
    let installed = tokio::task::block_in_place(|| install(home, releases.path()));
    set_guard(home, GUARD_SECS, "deny");
    std::fs::create_dir_all(home.join(".claude")).unwrap();
    tokio::task::block_in_place(|| reins(home, &["harness", "add", "claude-code"]));
    let settings: Value =
        serde_json::from_str(&std::fs::read_to_string(home.join(".claude/settings.json")).unwrap()).unwrap();
    let entries = settings["hooks"]["PreToolUse"].as_array().expect("PreToolUse hooks");
    let entry = entries
        .iter()
        .find(|e| e["matcher"].as_str().is_some_and(|m| m.split('|').any(|t| t == "Bash")))
        .unwrap_or_else(|| panic!("no PreToolUse hook for Bash: {settings}"));
    let hook = &entry["hooks"][0];
    assert_eq!(hook["type"], "command");
    let hook_command = hook["command"].as_str().unwrap().to_owned();
    assert_eq!(hook_command, format!("{} hook claude-code", installed.display()));
    assert_eq!(hook["timeout"], GUARD_SECS + 30, "the agent waits longer than Reins does");
    let mcp: Value = serde_json::from_str(&std::fs::read_to_string(home.join(".claude.json")).unwrap()).unwrap();
    assert_eq!(mcp["mcpServers"]["reins"]["command"].as_str(), installed.to_str());
    let listed = tokio::task::block_in_place(|| reins(home, &["harness", "list", "claude-code"]));
    assert!(String::from_utf8_lossy(&listed.stdout).contains("set up"), "{listed:?}");

    // ---- pairing: `reins login`, the phone checks the computer's key fingerprint and pins the key ----
    let server = Server::start(20, 10).await;
    server.register(EMAIL).await;
    let phone = Phone::sign_in(&server.base, EMAIL).await;
    let paths = Paths {
        config_dir: home.join(".config/reins"),
        state_dir: home.join(".local/state/reins"),
    };
    paths.ensure().unwrap();
    let identity = Identity::load_or_create(&paths.identity_file()).unwrap();
    let fingerprint = identity.fingerprint();
    let (url_tx, url_rx) = tokio::sync::oneshot::channel::<String>();
    let (code_tx, code_rx) = tokio::sync::oneshot::channel::<u8>();
    let login = reins_desktop::server::oauth::login_with_browser(&paths, &identity, &server.base, move |url| {
        url_tx.send(url.to_owned()).unwrap();
    });
    let browser = async { browse(url_rx.await.unwrap(), code_tx).await };
    let user = async {
        let item = phone.wait_for_item(Duration::from_secs(60)).await;
        let view = phone.core.pairing_view(item.id.clone()).await.unwrap();
        assert_eq!(view.key_fingerprint.as_deref(), Some(fingerprint.as_str()), "the phone shows the computer's key");
        let code = code_rx.await.unwrap();
        phone.core.answer_pairing(item.id, true, Some(code), Some("Laptop".to_owned()), true).await.unwrap();
    };
    let (logged_in, (), ()) = tokio::join!(login, browser, user);
    assert_eq!(logged_in.unwrap(), server.base);
    let status = tokio::task::block_in_place(|| reins(home, &["status"]));
    assert!(String::from_utf8_lossy(&status.stdout).contains(&server.base), "{status:?}");

    // ---- a harmless command: no question, no output, the agent's own flow goes on ----
    let quiet = decision(fire(home, &hook_command, &bash_call(home, "git status")).await).await;
    assert_eq!((quiet.code, quiet.stdout.as_str()), (0, ""), "git status is not asked about");
    assert!(phone.core.sync(1).await.unwrap().is_empty(), "nothing reached the phone");

    // ---- the force push, approved and then denied on the phone ----
    for approve in [true, false] {
        let mut child = fire(home, &hook_command, &bash_call(home, FORCE_PUSH)).await;
        let item = phone.wait_for_item(Duration::from_secs(60)).await;
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert!(child.try_wait().unwrap().is_none(), "the agent is held while the phone decides");
        let view = phone.core.approval_view(item.id.clone()).await.unwrap();
        assert_eq!(view.connection_label, "Laptop", "from the paired computer");
        assert!(view.no_standing, "a force push is approved once, never for a while");
        let ask = view.ask.clone().expect("a question from the hook");
        let shown = format!("{} {}", ask.question, ask.detail.clone().unwrap_or_default());
        assert!(shown.contains(FORCE_PUSH), "the phone shows the command: {ask:?}");
        assert!(ask.destructive, "a force push is shown as destructive: {ask:?}");
        let rule = ask.rule.clone().expect("the rule that asked");
        assert!(rule.destructive && rule.matched.contains("push --force"), "{rule:?}");
        if approve {
            let choice = ApprovalChoice {
                selected_message_ids: vec![],
                standing: None,
            };
            phone.core.approve(item.id, choice).await.unwrap();
        } else {
            phone.core.deny(item.id).await.unwrap();
        }
        let d = decision(child).await;
        assert_eq!(d.code, 0, "{}", d.stdout);
        if approve {
            assert_eq!(d.decision.as_deref(), Some("allow"), "{}", d.stdout);
            assert!(d.reason.contains("approved"), "{}", d.stdout);
        } else {
            assert_eq!(d.decision.as_deref(), Some("deny"), "{}", d.stdout);
            assert!(d.reason.contains("denied") && d.reason.contains("Do not try another way"), "{}", d.stdout);
        }
        assert_eq!(d.message, d.reason, "the user sees the reason too");
    }

    // ---- nobody answers: refused after the guard's wait, or handed back to the agent's prompt ----
    for (on_no_answer, expected) in [("deny", "deny"), ("ask", "ask")] {
        set_guard(home, NO_ANSWER_SECS, on_no_answer);
        let started = std::time::Instant::now();
        let child = fire(home, &hook_command, &bash_call(home, FORCE_PUSH)).await;
        let d = decision(child).await;
        let waited = started.elapsed();
        assert!(
            waited >= Duration::from_secs(NO_ANSWER_SECS) && waited < Duration::from_secs(NO_ANSWER_SECS + 25),
            "waited {waited:?}"
        );
        assert_eq!((d.code, d.decision.as_deref()), (0, Some(expected)), "{}", d.stdout);
        if expected == "deny" {
            assert!(d.reason.contains("timed out"), "{}", d.stdout);
        }
    }
    let unanswered = phone.core.pending().await.unwrap();
    assert!(!unanswered.is_empty(), "the unanswered questions reached the phone");
}
