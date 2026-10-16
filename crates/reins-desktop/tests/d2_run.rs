//! `reins run` (the binary) against a mock Reins server whose phone seals a `SecretGrant`: the variables reach
//! the command, its output and exit code pass through, and answers that do not fit the request are refused before
//! anything runs.

mod common;
mod d2_support;

use std::process::Output;

use d2_support::{App, Mock, Step, logged_in};

fn command(app: &App, config: &str, args: &[&str]) -> tokio::process::Command {
    std::fs::create_dir_all(&app.paths.config_dir).unwrap();
    // No "check your phone" notifications from tests.
    std::fs::write(app.paths.config_file(), format!("{config}\n[notify]\nphone = false\n")).unwrap();
    let home = app.dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let mut c = tokio::process::Command::new(common::reins_exe());
    c.arg("run")
        .args(args)
        .envs(reins_desktop::config::home_env(&home))
        .env("REINS_CONFIG_DIR", &app.paths.config_dir)
        .env("REINS_STATE_DIR", &app.paths.state_dir)
        .env_remove("REINS_LOG")
        .current_dir(app.dir.path());
    // Whatever runs these tests (an AI agent, say) must not make the command an agent's.
    for marker in
        ["REINS_RUNNER", "CLAUDECODE", "CLAUDE_CODE_ENTRYPOINT", "CODEX_SANDBOX", "GEMINI_CLI", "CURSOR_AGENT"]
    {
        c.env_remove(marker);
    }
    c
}

async fn run(app: &App, config: &str, args: &[&str]) -> Output {
    command(app, config, args).output().await.unwrap()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

#[cfg(not(windows))]
const PRINT_AND_EXIT_7: &[&str] = &["--", "sh", "-c", "printf '[%s|%s]' \"$TOKEN\" \"$WHO\"; exit 7"];
#[cfg(windows)]
const PRINT_AND_EXIT_7: &[&str] = &[
    "--",
    "powershell",
    "-NoProfile",
    "-Command",
    "[Console]::Out.Write('[' + $env:TOKEN + '|' + $env:WHO + ']'); exit 7",
];

/// A command that does nothing and succeeds, and how the phone shows it.
const SUCCEED: (&[&str], &str) = if cfg!(windows) {
    (&["cmd", "/c", "exit 0"], "cmd /c 'exit 0'")
} else {
    (&["true"], "true")
};

#[tokio::test]
async fn the_released_secrets_reach_the_command_and_its_exit_code_comes_back() {
    let mock = Mock::start().await;
    let app = logged_in(&mock);
    let mut args = vec!["--env", "TOKEN=vault:Deploy/password", "-e", "WHO=vault:Deploy/username", "--purpose", "ship"];
    args.extend_from_slice(PRINT_AND_EXIT_7);
    let out = run(&app, "", &args).await;
    assert_eq!(out.status.code(), Some(7), "{}", stderr(&out));
    assert_eq!(stdout(&out), "[[vault:Deploy/password]|[vault:Deploy/username]]", "the output shows no secret");
    assert!(!stderr(&out).contains("5ecret"), "{}", stderr(&out));

    let calls = mock.calls();
    assert_eq!(calls.len(), 1);
    let a = &calls[0].arguments;
    assert_eq!(calls[0].tool, "vault_secret_release");
    assert_eq!(a["secrets"], serde_json::json!(["Deploy/password", "Deploy/username"]));
    if cfg!(windows) {
        assert!(
            a["command"].as_str().unwrap().starts_with("powershell -NoProfile -Command '[Console]"),
            "{}",
            a["command"]
        );
    } else {
        assert_eq!(a["command"], "sh -c 'printf '\\''[%s|%s]'\\'' \"$TOKEN\" \"$WHO\"; exit 7'");
    }
    assert_eq!(a["purpose"], "ship");
    assert_eq!(a["lease_secs"], 60);
    assert_eq!(a["client_key"], app.public_key());
    assert_eq!(a["nonce"].as_str().unwrap().len(), 22);
}

#[tokio::test]
async fn profiles_come_from_the_config_and_stdin_passes_through() {
    let mock = Mock::start().await;
    let app = logged_in(&mock);
    // The daemon runs it (on Unix), reading the caller alone, not the tests' own ancestry.
    #[cfg(not(windows))]
    let _daemon = daemon(&app).await;
    let config = "[run.profiles.ai]\npurpose = \"coding\"\nenv = { OPENAI_API_KEY = \"vault:OpenAI/password\" }\n";
    let echo: &[&str] = if cfg!(windows) {
        &[
            "powershell",
            "-NoProfile",
            "-Command",
            "$line = [Console]::In.ReadLine(); [Console]::Out.Write($line + ' ' + $env:OPENAI_API_KEY)",
        ]
    } else {
        &["sh", "-c", "read line; printf '%s %s' \"$line\" \"$OPENAI_API_KEY\""]
    };
    let mut args = vec!["--profile", "ai", "--no-redact", "--"];
    args.extend_from_slice(echo);
    let mut cmd = command(&app, config, &args);
    cmd.stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
    let mut child = cmd.spawn().unwrap();
    {
        use tokio::io::AsyncWriteExt as _;
        let mut stdin = child.stdin.take().unwrap();
        stdin.write_all(b"hello\n").await.unwrap();
    }
    let out = child.wait_with_output().await.unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), "hello sk-live-5ecret-0penai");
    let a = &mock.calls()[0].arguments;
    assert_eq!(a["purpose"], "coding");
    assert_eq!(a["secrets"], serde_json::json!(["OpenAI/password"]));

    let out = run(&app, config, &["--profile", "nope", "--", "true"]).await;
    assert_eq!(out.status.code(), Some(125));
    assert!(stderr(&out).contains("no profile `nope`"), "{}", stderr(&out));
}

async fn refused(step: Step, expected: &str) {
    let mock = Mock::start().await;
    let app = logged_in(&mock);
    mock.plan(&[step]);
    let marker = app.dir.path().join("ran");
    let script = if cfg!(windows) {
        format!("New-Item -ItemType File -Path '{}'", marker.display())
    } else {
        format!("touch {}", marker.display())
    };
    let shell: &[&str] = if cfg!(windows) {
        &["powershell", "-NoProfile", "-Command"]
    } else {
        &["sh", "-c"]
    };
    let mut args = vec!["--env", "TOKEN=vault:Deploy/password", "--"];
    args.extend_from_slice(shell);
    args.push(&script);
    let out = run(&app, "", &args).await;
    assert_eq!(out.status.code(), Some(125), "{step:?}: {}", stderr(&out));
    assert!(stderr(&out).contains(expected), "{step:?}: {}", stderr(&out));
    assert!(!marker.exists(), "{step:?}: the command must not run");
    assert!(!stderr(&out).contains("5ecret"));
}

#[tokio::test]
async fn a_forged_nonce_is_refused_and_nothing_runs() {
    refused(Step::ForgedNonce, "nonce differs").await;
}

#[tokio::test]
async fn other_refusals_stop_the_command_too() {
    refused(Step::Denied("Not now"), "Not now").await;
    refused(Step::Expired, "expired").await;
    refused(Step::OtherKey, "not sealed to this app's key").await;
}

#[tokio::test]
async fn local_mode_and_a_missing_login_say_where_secrets_live() {
    let mock = Mock::start().await;
    let app = logged_in(&mock);
    let out = run(&app, "mode = \"local\"\n", &["-e", "A=vault:Deploy/password", "--", "true"]).await;
    assert_eq!(out.status.code(), Some(125));
    assert!(stderr(&out).contains("local mode"), "{}", stderr(&out));
    std::fs::remove_file(app.paths.session_file()).unwrap();
    let out = run(&app, "", &["-e", "A=vault:Deploy/password", "--", "true"]).await;
    assert_eq!(out.status.code(), Some(125));
    assert!(stderr(&out).contains("reins login"), "{}", stderr(&out));
    assert!(mock.calls().is_empty());
}

#[tokio::test]
async fn a_missing_command_is_127() {
    let mock = Mock::start().await;
    let app = logged_in(&mock);
    let out = run(&app, "", &["-e", "A=vault:Deploy/password", "--", "/nonexistent/reins-test-cmd"]).await;
    assert_eq!(out.status.code(), Some(127), "{}", stderr(&out));
}

#[tokio::test]
async fn a_slow_phone_is_announced_on_stderr() {
    let mock = Mock::start().await;
    let app = logged_in(&mock);
    mock.plan(&[Step::Pending, Step::Approve]);
    let (succeed, shown) = SUCCEED;
    let mut args = vec!["-e", "A=vault:Deploy/password", "--"];
    args.extend_from_slice(succeed);
    let out = run(&app, "", &args).await;
    assert!(out.status.success(), "{}", stderr(&out));
    let err = stderr(&out);
    assert!(err.contains(&format!("waiting for Reins 2FA on your phone, up to 120 s: secrets for `{shown}`")), "{err}");
    assert!(err.contains("reins: approved on your phone."), "{err}");
}

// ---- Reins Secrets: projects ------------------------------------------------------------------------------------------

/// The caller alone, without its ancestors: the tests' own ancestry (an AI agent running them, say) must not decide.
#[cfg(not(windows))]
fn alone(pid: u32) -> Option<reins_desktop::runner::ProcessInfo> {
    reins_desktop::runner::process_info(pid).map(|mut p| {
        p.parent = None;
        p
    })
}

/// The daemon `reins run` hands the command to.
#[cfg(not(windows))]
async fn daemon(app: &App) -> reins_desktop::daemon::Running {
    let mut config = reins_desktop::config::Config {
        listen: "127.0.0.1:0".parse().unwrap(),
        approval_timeout_secs: 20,
        ..reins_desktop::config::Config::default()
    };
    config.ssh.enabled = false;
    reins_desktop::daemon::Daemon::bind(
        &app.paths,
        config,
        reins_desktop::daemon::Options {
            authorizer: None,
            prompter: std::sync::Arc::new(reins_desktop::auth::prompt::NoPrompter),
            harden: false,
            process_info: alone,
        },
    )
    .await
    .unwrap()
    .spawn()
}

#[cfg(not(windows))]
fn in_project(app: &App) -> std::path::PathBuf {
    let root = app.dir.path().join("acme");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("reins.toml"), "[project]\nname = \"acme\"\ndefault_env = \"dev\"\n").unwrap();
    root
}

#[cfg(not(windows))]
const PRINT_ENV: &[&str] =
    &["--", "sh", "-c", "printf '%s|%s' \"$API_KEY\" \"$DB_URL\"; printf 'err %s' \"$API_KEY\" >&2"];

#[cfg(not(windows))]
#[tokio::test]
async fn in_a_project_the_environment_is_released_bound_to_the_command_and_redacted() {
    let mock = Mock::start().await;
    let app = logged_in(&mock);
    let _daemon = daemon(&app).await;
    let root = in_project(&app);
    let out = command(&app, "", PRINT_ENV).current_dir(root.join("src")).output().await.unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), "[vault:acme/dev/API_KEY]|[vault:acme/dev/DB_URL]");
    assert_eq!(stderr(&out), "err [vault:acme/dev/API_KEY]");
    let calls = mock.calls_of("vault_env_release");
    assert_eq!(calls.len(), 1);
    let a = &calls[0].arguments;
    assert_eq!(
        (a["project"].as_str(), a["env"].as_str(), a["runner"].as_str()),
        (Some("acme"), Some("dev"), Some("person"))
    );
    assert!(a.get("fields").is_none(), "the whole environment");
    assert!(a["program"].as_str().unwrap().starts_with('/'), "the program by its path: {}", a["program"]);
    assert_eq!(a["args"][0], "-c");
    assert_eq!(a["root"].as_str().unwrap(), std::fs::canonicalize(&root).unwrap().to_string_lossy());

    // A person may see the values; an agent may not ask to.
    let out = command(&app, "", &[&["--no-redact"][..], PRINT_ENV].concat()).current_dir(&root).output().await.unwrap();
    assert_eq!(stdout(&out), "sk-env-5ecret-dev|postgres://u:pw-5ecret@db");
    let out = command(&app, "", &[&["--no-redact"][..], PRINT_ENV].concat())
        .current_dir(&root)
        .env("REINS_RUNNER", "agent:claude-code")
        .output()
        .await
        .unwrap();
    assert_eq!(out.status.code(), Some(125));
    assert!(stderr(&out).contains("--no-redact is for people"), "{}", stderr(&out));
    let out = command(&app, "", PRINT_ENV)
        .current_dir(&root)
        .env("REINS_RUNNER", "agent:claude-code")
        .output()
        .await
        .unwrap();
    assert!(!stdout(&out).contains("5ecret"));
    assert_eq!(mock.calls_of("vault_env_release").last().unwrap().arguments["runner"], "agent:claude-code");
}

#[cfg(not(windows))]
#[tokio::test]
async fn env_reins_names_the_variables_and_literals_pass_through() {
    let mock = Mock::start().await;
    let app = logged_in(&mock);
    let _daemon = daemon(&app).await;
    let root = in_project(&app);
    std::fs::write(root.join(".env.reins"), "KEY=env:dev/API_KEY\nLOG_LEVEL=debug\n").unwrap();
    let out = command(
        &app,
        "",
        &["--no-redact", "--", "sh", "-c", "printf '%s %s %s' \"$KEY\" \"$LOG_LEVEL\" \"${DB_URL:-unset}\""],
    )
    .current_dir(&root)
    .output()
    .await
    .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), "sk-env-5ecret-dev debug unset");
    assert_eq!(mock.calls_of("vault_env_release")[0].arguments["fields"], serde_json::json!(["API_KEY"]));
}

#[cfg(not(windows))]
#[tokio::test]
async fn a_release_bound_to_someone_else_is_refused_and_nothing_runs() {
    let mock = Mock::start().await;
    let app = logged_in(&mock);
    let _daemon = daemon(&app).await;
    let root = in_project(&app);
    mock.plan(&[Step::OtherRunner]);
    let marker = root.join("ran");
    let out = command(&app, "", &["--", "touch", marker.to_str().unwrap()]).current_dir(&root).output().await.unwrap();
    assert_eq!(out.status.code(), Some(125), "{}", stderr(&out));
    assert!(stderr(&out).contains("someone else"), "{}", stderr(&out));
    assert!(!marker.exists());
}

#[cfg(not(windows))]
#[tokio::test]
async fn without_the_daemon_a_projects_secrets_are_not_released() {
    let mock = Mock::start().await;
    let app = logged_in(&mock);
    let root = in_project(&app);
    let out = command(&app, "", &["--", "true"]).current_dir(&root).output().await.unwrap();
    assert_eq!(out.status.code(), Some(125));
    assert!(stderr(&out).contains("reins resume"), "{}", stderr(&out));
    assert!(mock.calls().is_empty());
}

#[cfg(not(windows))]
#[tokio::test]
async fn the_daemon_runs_the_exact_command_and_keeps_the_lease_for_it() {
    let mock = Mock::start().await;
    let app = logged_in(&mock);
    let _daemon = daemon(&app).await;
    let root = in_project(&app);
    let script = ["--", "sh", "-c", "printf '%s' \"$API_KEY\"; exit 3"];
    let first = command(&app, "", &script).current_dir(&root).output().await.unwrap();
    assert_eq!(first.status.code(), Some(3), "{}", stderr(&first));
    assert_eq!(stdout(&first), "[vault:acme/dev/API_KEY]");
    let again = command(&app, "", &script).current_dir(&root).output().await.unwrap();
    assert_eq!(stdout(&again), "[vault:acme/dev/API_KEY]");
    assert_eq!(mock.calls_of("vault_env_release").len(), 1, "the same command: the lease");
    // Another command, or the same one for an agent, asks again.
    command(&app, "", &["--", "sh", "-c", "true"]).current_dir(&root).output().await.unwrap();
    command(&app, "", &script).current_dir(&root).env("REINS_RUNNER", "agent:codex").output().await.unwrap();
    let calls = mock.calls_of("vault_env_release");
    assert_eq!(calls.len(), 3);
    assert_eq!(calls[2].arguments["runner"], "agent:codex");
}
