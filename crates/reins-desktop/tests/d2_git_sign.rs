//! Signing git commits and tags on the phone, driven by real git: `reins` (this crate's binary) is git's
//! `gpg.ssh.program`; it hands the commit to the SSH agent of a daemon in this process, whose mock phone signs it with a
//! real ed25519 key. git then verifies the signature itself (`git verify-commit`, which `reins` hands to the real
//! ssh-keygen). A key that is not the phone's still signs through ssh-keygen. Unix only (the agent's socket); skips
//! without git or ssh-keygen.
#![cfg(unix)]

mod d2_support;

use std::path::{Path, PathBuf};
use std::process::Output;
use std::sync::Arc;

use d2_support::{App, Mock, Step, capture_logs, logged_in};
use data_encoding::BASE64;
use reins_desktop::auth::prompt::NoPrompter;
use reins_desktop::config::{Config, Mode};
use reins_desktop::daemon::{Daemon, Options, Running};

fn have(tool: &str) -> bool {
    std::process::Command::new(tool).arg("--version").output().is_ok()
        || std::process::Command::new(tool).arg("-?").output().is_ok()
}

struct Repo {
    mock: Mock,
    app: App,
    home: PathBuf,
    repo: PathBuf,
    signers: PathBuf,
    _running: Running,
}

impl Repo {
    async fn start() -> Self {
        capture_logs();
        let mock = Mock::start().await;
        let app = logged_in(&mock);
        let socket = app.dir.path().join("agent.sock");
        let mut config = Config {
            listen: "127.0.0.1:0".parse().unwrap(),
            mode: Mode::Reins,
            approval_timeout_secs: 20,
            ..Config::default()
        };
        config.ssh.socket = Some(socket.clone());
        config.ssh.known_hosts = vec![app.dir.path().join("known_hosts")];
        // What `reins -Y sign` reads to find the agent.
        std::fs::create_dir_all(&app.paths.config_dir).unwrap();
        std::fs::write(app.paths.config_file(), format!("[ssh]\nsocket = {:?}\n", socket.to_str().unwrap())).unwrap();
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
        let home = app.dir.path().join("home");
        let repo = app.dir.path().join("app");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&repo).unwrap();
        let signers = app.dir.path().join("allowed_signers");
        std::fs::write(&signers, format!("ada@example.com namespaces=\"git\" {}\n", mock.key.line())).unwrap();
        let r = Self {
            mock,
            app,
            home,
            repo,
            signers,
            _running: running,
        };
        for args in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.name", "Ada"],
            &["config", "user.email", "ada@example.com"],
            &["config", "gpg.format", "ssh"],
            &["config", "gpg.ssh.program", env!("CARGO_BIN_EXE_reins")],
            &["config", "gpg.ssh.allowedSignersFile", r.signers.to_str().unwrap()],
            &["remote", "add", "origin", "https://github.com/me/app.git"],
        ] {
            assert!(r.git(args).await.status.success(), "git {args:?}");
        }
        let key = format!("key::{}", r.mock.key.line());
        assert!(r.git(&["config", "user.signingkey", &key]).await.status.success());
        r
    }

    async fn git(&self, args: &[&str]) -> Output {
        let mut c = tokio::process::Command::new("git");
        c.args(args).current_dir(&self.repo).stdin(std::process::Stdio::null());
        c.envs(reins_desktop::config::home_env(&self.home));
        c.env("REINS_CONFIG_DIR", &self.app.paths.config_dir).env("REINS_STATE_DIR", &self.app.paths.state_dir);
        c.env_remove("SSH_AUTH_SOCK").env_remove("GIT_DIR").env_remove("GIT_WORK_TREE");
        c.output().await.unwrap()
    }
}

fn text(o: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}

fn ready() -> bool {
    if have("git") && have("ssh-keygen") {
        return true;
    }
    eprintln!("skipped: no git or ssh-keygen");
    false
}

#[tokio::test]
async fn a_commit_and_a_tag_are_signed_on_the_phone_and_git_verifies_them() {
    if !ready() {
        return;
    }
    let r = Repo::start().await;
    r.mock.plan(&[Step::Approve]);
    r.mock.plan(&[Step::Pending, Step::Approve]);
    let out = r.git(&["commit", "-q", "--allow-empty", "-S", "-m", "fix: signed on the phone", "-m", "Body."]).await;
    assert!(out.status.success(), "{}", text(&out));
    let verify = r.git(&["verify-commit", "-v", "HEAD"]).await;
    assert!(verify.status.success(), "{}", text(&verify));
    assert!(text(&verify).contains("Good \"git\" signature for ada@example.com"), "{}", text(&verify));
    let shown = r.git(&["log", "-1", "--format=%G?"]).await;
    assert_eq!(String::from_utf8_lossy(&shown.stdout).trim(), "G");

    let calls = r.mock.calls_of("vault_git_sign");
    assert_eq!(calls.len(), 1);
    let args = &calls[0].arguments;
    assert_eq!(args["key"], r.mock.key.fingerprint());
    assert_eq!(
        (args["repo"].as_str(), args["branch"].as_str(), args["hash"].as_str()),
        (Some("me/app"), Some("main"), Some("sha512"))
    );
    let object = BASE64.decode(args["object_base64"].as_str().unwrap().as_bytes()).unwrap();
    let object = String::from_utf8(object).unwrap();
    assert!(object.starts_with("tree ") && object.contains("\n\nfix: signed on the phone\n\nBody.\n"), "{object}");
    assert!(r.mock.calls_of("vault_ssh_sign").is_empty(), "never signed as an SSH login");

    r.mock.plan(&[Step::Approve]);
    let tag = r.git(&["tag", "-s", "v1.0", "-m", "v1.0"]).await;
    assert!(tag.status.success(), "{}", text(&tag));
    let verify = r.git(&["verify-tag", "v1.0"]).await;
    assert!(verify.status.success(), "{}", text(&verify));
    let calls = r.mock.calls_of("vault_git_sign");
    assert_eq!(calls.len(), 2);
    assert!(calls[1].arguments.get("branch").is_none(), "a tag names no branch");
    let tag_object = BASE64.decode(calls[1].arguments["object_base64"].as_str().unwrap().as_bytes()).unwrap();
    assert!(String::from_utf8(tag_object).unwrap().contains("\ntag v1.0\n"));
    // The first commit had no parent; the tag carries the commit it tags, signed, exactly as git stores it.
    assert!(calls[0].arguments.get("ancestors").is_none());
    let head = String::from_utf8_lossy(&r.git(&["rev-parse", "HEAD"]).await.stdout).trim().to_owned();
    let tagged = BASE64.decode(calls[1].arguments["ancestors"][0].as_str().unwrap().as_bytes()).unwrap();
    assert_eq!(reins_proto::sshsig::commit_id(&tagged, false), head);
    assert!(reins_proto::sshsig::strip_signature(&tagged).is_some(), "the phone's signature is in it");

    // A second commit: its parent comes along, and what the phone reaches through it includes what it signed.
    r.mock.plan(&[Step::Approve]);
    let out = r.git(&["commit", "-q", "--allow-empty", "-S", "-m", "second"]).await;
    assert!(out.status.success(), "{}", text(&out));
    let calls = r.mock.calls_of("vault_git_sign");
    let first_object = BASE64.decode(calls[0].arguments["object_base64"].as_str().unwrap().as_bytes()).unwrap();
    let object = BASE64.decode(calls[2].arguments["object_base64"].as_str().unwrap().as_bytes()).unwrap();
    let ancestors: Vec<Vec<u8>> = calls[2].arguments["ancestors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| BASE64.decode(a.as_str().unwrap().as_bytes()).unwrap())
        .collect();
    let parsed = reins_proto::sshsig::parse_object(&object).unwrap();
    let reaches = reins_proto::sshsig::lineage(&parsed, &ancestors);
    assert!(reaches.contains(&format!("id:{head}")), "{reaches:?}");
    assert!(reaches.contains(&format!("sig:{}", reins_proto::sshsig::signed_digest(&first_object))), "{reaches:?}");
}

#[tokio::test]
async fn a_refused_or_forged_signature_fails_the_commit() {
    if !ready() {
        return;
    }
    let r = Repo::start().await;
    r.mock.plan(&[Step::Approve]);
    for (step, said) in [
        (Step::Denied("Not this one"), "Not this one"),
        (Step::ForgedNonce, "nonce differs"),
        (Step::WrongSignature, "does not verify"),
    ] {
        r.mock.plan(&[step]);
        let out = r.git(&["commit", "-q", "--allow-empty", "-S", "-m", "nope"]).await;
        assert!(!out.status.success(), "{step:?}");
        assert!(text(&out).contains(said), "{step:?}: {}", text(&out));
    }
    let log = r.git(&["rev-list", "--all", "--count"]).await;
    assert_eq!(String::from_utf8_lossy(&log.stdout).trim(), "0", "no commit was made");
}

#[tokio::test]
async fn a_key_that_is_not_the_phones_is_signed_by_ssh_keygen_as_before() {
    if !ready() {
        return;
    }
    let r = Repo::start().await;
    r.mock.plan(&[Step::Approve]);
    let key = r.app.dir.path().join("local_key");
    let made = std::process::Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-C", "local", "-f"])
        .arg(&key)
        .output()
        .unwrap();
    assert!(made.status.success());
    assert!(r.git(&["config", "user.signingkey", key.to_str().unwrap()]).await.status.success());
    let public = std::fs::read_to_string(Path::new(&format!("{}.pub", key.display()))).unwrap();
    std::fs::write(&r.signers, format!("ada@example.com namespaces=\"git\" {public}")).unwrap();
    let out = r.git(&["commit", "-q", "--allow-empty", "-S", "-m", "local key"]).await;
    assert!(out.status.success(), "{}", text(&out));
    let verify = r.git(&["verify-commit", "HEAD"]).await;
    assert!(verify.status.success(), "{}", text(&verify));
    assert!(r.mock.calls_of("vault_git_sign").is_empty(), "the phone was not asked");
}
