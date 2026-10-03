//! `rewarden run` (the binary) against a mock Rewarden server whose phone seals a `SecretGrant`: the variables reach
//! the command, its output and exit code pass through, and answers that do not fit the request are refused before
//! anything runs.

mod d2_support;

use std::process::Output;

use d2_support::{App, Mock, Step, logged_in};

fn command(app: &App, config: &str, args: &[&str]) -> tokio::process::Command {
    std::fs::create_dir_all(&app.paths.config_dir).unwrap();
    std::fs::write(app.paths.config_file(), config).unwrap();
    let home = app.dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let mut c = tokio::process::Command::new(env!("CARGO_BIN_EXE_rewarden"));
    c.arg("run")
        .args(args)
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env("REWARDEN_CONFIG_DIR", &app.paths.config_dir)
        .env("REWARDEN_STATE_DIR", &app.paths.state_dir)
        .env_remove("REWARDEN_LOG")
        .current_dir(app.dir.path());
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
    assert_eq!(stdout(&out), "[deploy-5ecret value|deployer]");
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
    let mut args = vec!["--profile", "ai", "--"];
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
    assert!(stderr(&out).contains("rewarden login"), "{}", stderr(&out));
    assert!(mock.calls().is_empty());
}

#[tokio::test]
async fn a_missing_command_is_127() {
    let mock = Mock::start().await;
    let app = logged_in(&mock);
    let out = run(&app, "", &["-e", "A=vault:Deploy/password", "--", "/nonexistent/rewarden-test-cmd"]).await;
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
    assert!(err.contains(&format!("waiting for approval in your Rewarden app: secrets for `{shown}`")), "{err}");
    assert!(err.contains("rewarden: approved."), "{err}");
}
