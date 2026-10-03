//! The SSH agent in a daemon in this process, with a mock phone that holds a real ed25519 key, driven by the real
//! OpenSSH tools: `ssh-add -l` lists the phone's key, `ssh-keygen -Y sign` gets a signature that verifies, a real
//! `ssh` login to a throwaway `sshd` is signed on the phone with the server named (session-bind + known_hosts), and
//! refusals fail the client. Also `reins ssh setup|unsetup` on a temporary HOME. Skips what needs a missing tool.

mod d2_support;

use std::path::{Path, PathBuf};
use std::process::Output;
use std::sync::Arc;
use std::time::Duration;

use d2_support::{App, Mock, Step, capture_logs, logged_in};
use reins_desktop::auth::prompt::NoPrompter;
use reins_desktop::config::{Config, Mode};
use reins_desktop::daemon::{Daemon, Options, Running};

/// Whether the agent can write to the client's stderr: it finds the client through /proc, so only on Linux (elsewhere
/// it says nothing; see `tell_pid` in ssh_agent/mod.rs).
const TELLS_THE_CLIENT: bool = cfg!(target_os = "linux");

/// An OpenSSH tool: on Windows the one that comes with Windows (Git's MSYS OpenSSH, maybe first on `PATH`, cannot talk
/// to a named pipe).
fn openssh(tool: &str) -> PathBuf {
    if cfg!(windows) {
        reins_desktop::win::system32(&format!("OpenSSH\\{tool}.exe"))
    } else {
        PathBuf::from(tool)
    }
}

fn have(tool: &str) -> bool {
    std::process::Command::new(openssh(tool)).arg("-?").output().is_ok()
}

struct Agent {
    mock: Mock,
    app: App,
    socket: PathBuf,
    known_hosts: PathBuf,
    running: Option<Running>,
}

impl Agent {
    async fn start() -> Self {
        capture_logs();
        let mock = Mock::start().await;
        let app = logged_in(&mock);
        let socket = if cfg!(windows) {
            reins_desktop::win::ssh_pipe(&app.paths.state_dir)
        } else {
            app.dir.path().join("agent.sock")
        };
        let known_hosts = app.dir.path().join("known_hosts");
        let mut config = Config {
            listen: "127.0.0.1:0".parse().unwrap(),
            mode: Mode::Reins,
            approval_timeout_secs: 20,
            ..Config::default()
        };
        config.ssh.socket = Some(socket.clone());
        config.ssh.known_hosts = vec![known_hosts.clone()];
        let daemon = Daemon::bind(
            &app.paths,
            config,
            Options {
                authorizer: None,
                prompter: Arc::new(NoPrompter),
                harden: false,
            },
        )
        .await
        .unwrap();
        assert_eq!(daemon.ssh_socket(), Some(socket.as_path()));
        Self {
            mock,
            app,
            socket,
            known_hosts,
            running: Some(daemon.spawn()),
        }
    }

    fn file(&self, name: &str, contents: &str) -> PathBuf {
        let p = self.app.dir.path().join(name);
        std::fs::write(&p, contents).unwrap();
        p
    }

    async fn tool(&self, program: &str, args: &[&str], stdin: Option<&Path>) -> Output {
        let mut c = tokio::process::Command::new(openssh(program));
        c.args(args).env("SSH_AUTH_SOCK", &self.socket).current_dir(self.app.dir.path());
        if let Some(f) = stdin {
            c.stdin(std::fs::File::open(f).unwrap());
        }
        c.output().await.unwrap()
    }
}

fn text(o: &Output) -> (String, String) {
    (String::from_utf8_lossy(&o.stdout).into_owned(), String::from_utf8_lossy(&o.stderr).into_owned())
}

#[tokio::test]
async fn ssh_add_lists_the_phones_key_and_the_socket_is_private() {
    if !have("ssh-add") {
        eprintln!("skipped: no ssh-add");
        return;
    }
    let mut a = Agent::start().await;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(std::fs::metadata(&a.socket).unwrap().permissions().mode() & 0o777, 0o600);
    }
    let out = a.tool("ssh-add", &["-l"], None).await;
    let (stdout, stderr) = text(&out);
    assert!(out.status.success(), "{stdout}{stderr}");
    assert!(stdout.contains(&a.mock.key.fingerprint()), "{stdout}");
    assert!(stdout.contains("Deploy key (ED25519)"), "{stdout}");
    let pubfile = a.file("key.pub", &format!("{}\n", a.mock.key.line()));
    let keygen = text(&a.tool("ssh-keygen", &["-l", "-f", pubfile.to_str().unwrap()], None).await).0;
    assert!(keygen.contains(&a.mock.key.fingerprint()), "the same fingerprint as OpenSSH computes: {keygen}");
    // `ssh-add -L` prints the key line itself.
    let listed = text(&a.tool("ssh-add", &["-L"], None).await).0;
    assert!(listed.starts_with(&a.mock.key.line()[..80]), "{listed}");
    // Kept a few minutes: one question for all of these.
    assert_eq!(a.mock.calls_of("vault_ssh_keys").len(), 1);
    let args = &a.mock.calls_of("vault_ssh_keys")[0].arguments;
    assert_eq!(args["client_key"], a.app.public_key());

    // Adding or removing keys is not this agent's business.
    let out = a.tool("ssh-add", &["-D"], None).await;
    assert!(!out.status.success());

    // The socket goes away with the daemon (a named pipe is no file to look for).
    drop(a.running.take());
    if cfg!(windows) {
        return;
    }
    for _ in 0..100 {
        if !a.socket.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(!a.socket.exists());
}

async fn sign(a: &Agent) -> (Output, PathBuf) {
    let pubfile = a.file("key.pub", &format!("{}\n", a.mock.key.line()));
    let data = a.file("message.txt", "release v1.2.3\n");
    std::fs::remove_file(a.app.dir.path().join("message.txt.sig")).ok();
    let out = a
        .tool(
            "ssh-keygen",
            &["-Y", "sign", "-f", pubfile.to_str().unwrap(), "-n", "file", data.to_str().unwrap()],
            None,
        )
        .await;
    (out, data)
}

#[tokio::test]
async fn a_signature_made_on_the_phone_verifies_with_ssh_keygen() {
    if !have("ssh-keygen") {
        eprintln!("skipped: no ssh-keygen");
        return;
    }
    let a = Agent::start().await;
    a.mock.plan(&[Step::Approve]);
    a.mock.plan(&[Step::Pending, Step::Approve]);
    let (out, data) = sign(&a).await;
    let (_, stderr) = text(&out);
    assert!(out.status.success(), "{stderr}");
    if TELLS_THE_CLIENT {
        assert!(stderr.contains("reins: waiting for approval in your Reins app: sign with Deploy key"), "{stderr}");
        assert!(stderr.contains("reins: approved."), "{stderr}");
    }
    let signers = a.file("allowed_signers", &format!("me@example.com {}\n", a.mock.key.line()));
    let sig = format!("{}.sig", data.display());
    let verify = a
        .tool(
            "ssh-keygen",
            &["-Y", "verify", "-f", signers.to_str().unwrap(), "-I", "me@example.com", "-n", "file", "-s", &sig],
            Some(&data),
        )
        .await;
    let (stdout, stderr) = text(&verify);
    assert!(verify.status.success(), "{stdout}{stderr}");
    assert!(stdout.contains("Good \"file\" signature for me@example.com"), "{stdout}");

    let calls = a.mock.calls_of("vault_ssh_sign");
    assert_eq!(calls.len(), 1);
    let args = &calls[0].arguments;
    assert_eq!(args["key"], a.mock.key.fingerprint());
    assert_eq!(args["flags"], 0);
    assert!(args.get("host").is_none(), "not a login: no host");
    assert!(args["data_base64"].as_str().unwrap().len() > 20);
}

#[tokio::test]
async fn refused_or_forged_signatures_fail_the_client() {
    if !have("ssh-keygen") {
        eprintln!("skipped: no ssh-keygen");
        return;
    }
    let a = Agent::start().await;
    a.mock.plan(&[Step::Approve]);
    for (step, said) in [
        (Step::Denied("Not this one"), Some("reins: Not this one")),
        (Step::ForgedNonce, Some("nonce differs")),
        (Step::WrongSignature, Some("does not verify")),
        (Step::OtherKey, Some("not sealed to this app's key")),
    ] {
        a.mock.plan(&[step]);
        let (out, data) = sign(&a).await;
        let (_, stderr) = text(&out);
        assert!(!out.status.success(), "{step:?}");
        if let Some(said) = said.filter(|_| TELLS_THE_CLIENT) {
            assert!(stderr.contains(said), "{step:?}: {stderr}");
        }
        assert!(!Path::new(&format!("{}.sig", data.display())).exists(), "{step:?}");
    }
}

fn sshd() -> Option<PathBuf> {
    ["/usr/sbin/sshd", "/usr/bin/sshd", "/usr/local/sbin/sshd"].iter().map(PathBuf::from).find(|p| p.exists())
}

fn user() -> String {
    let out = std::process::Command::new("id").arg("-un").output().unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

struct Sshd(std::process::Child);

impl Drop for Sshd {
    fn drop(&mut self) {
        self.0.kill().ok();
        self.0.wait().ok();
    }
}

#[tokio::test]
async fn a_real_ssh_login_is_signed_on_the_phone_and_names_the_server() {
    let Some(sshd) = sshd().filter(|_| have("ssh") && have("ssh-keygen")) else {
        eprintln!("skipped: no sshd/ssh");
        return;
    };
    // The server's session-bind signature is checked for each kind of host key.
    for (kind, algorithms) in [("ed25519", "ssh-ed25519"), ("rsa", "rsa-sha2-512"), ("ecdsa", "ecdsa-sha2-nistp256")] {
        login(&sshd, kind, algorithms).await;
    }
}

async fn login(sshd: &Path, kind: &str, algorithms: &str) {
    let a = Agent::start().await;
    let dir = a.app.dir.path();
    let host_key = dir.join("host_key");
    let made = std::process::Command::new("ssh-keygen")
        .args(["-q", "-t", kind, "-N", "", "-f"])
        .arg(&host_key)
        .status()
        .unwrap();
    assert!(made.success());
    let authorized = a.file("authorized_keys", &format!("{}\n", a.mock.key.line()));
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let config = a.file(
        "sshd_config",
        &format!(
            "Port {port}\nListenAddress 127.0.0.1\nHostKey {}\nAuthorizedKeysFile {}\nPidFile none\nUsePAM no\nStrictModes no\nPasswordAuthentication no\nKbdInteractiveAuthentication no\n",
            host_key.display(),
            authorized.display()
        ),
    );
    let server = Sshd(
        std::process::Command::new(sshd)
            .args(["-D", "-e", "-f"])
            .arg(&config)
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap(),
    );
    for _ in 0..100 {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let host_pub = std::fs::read_to_string(dir.join("host_key.pub")).unwrap();
    std::fs::write(&a.known_hosts, format!("[127.0.0.1]:{port} {host_pub}")).unwrap();
    let pubfile = a.file("key.pub", &format!("{}\n", a.mock.key.line()));
    a.mock.plan(&[Step::Approve]);
    a.mock.plan(&[Step::Pending, Step::Approve]);
    let user = user();
    let out = a
        .tool(
            "ssh",
            &[
                "-F",
                "none",
                "-p",
                &port.to_string(),
                "-o",
                &format!("IdentityAgent={}", a.socket.display()),
                "-o",
                &format!("IdentityFile={}", pubfile.display()),
                "-o",
                "IdentitiesOnly=yes",
                "-o",
                &format!("UserKnownHostsFile={}", a.known_hosts.display()),
                "-o",
                "StrictHostKeyChecking=yes",
                "-o",
                &format!("HostKeyAlgorithms={algorithms}"),
                "-o",
                "BatchMode=yes",
                &format!("{user}@127.0.0.1"),
                "echo",
                "hello-through-the-phone",
            ],
            None,
        )
        .await;
    drop(server);
    let (stdout, stderr) = text(&out);
    assert!(out.status.success(), "{kind}: {stdout}{stderr}");
    assert_eq!(stdout.trim(), "hello-through-the-phone");
    let server_name = format!("127.0.0.1:{port}");
    assert!(
        !TELLS_THE_CLIENT
            || stderr.contains(&format!(
                "reins: waiting for approval in your Reins app: sign in to {server_name} as {user} with Deploy key"
            )),
        "{stderr}"
    );
    let calls = a.mock.calls_of("vault_ssh_sign");
    assert_eq!(calls.len(), 1);
    let args = &calls[0].arguments;
    assert_eq!(args["host"], server_name);
    let host_blob = reins_desktop::ssh_agent::keys::parse_line(&host_pub).unwrap().0;
    assert_eq!(args["host_key"], reins_desktop::ssh_agent::keys::fingerprint(&host_blob));
    assert_eq!(args["key"], a.mock.key.fingerprint());
    // ssh announced the server (session-bind), and the agent checked the server's signature.
    let logged = d2_support::logs();
    assert!(
        logged.contains(&format!("session bound to the server key {}", args["host_key"].as_str().unwrap())),
        "{kind}: {logged}"
    );
}

#[test]
fn ssh_setup_and_unsetup_edit_only_the_given_home() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir_all(home.join(".ssh")).unwrap();
    let user_config = "Host work\n    User me\n";
    std::fs::write(home.join(".ssh/config"), user_config).unwrap();
    let state = dir.path().join("state");
    let run = |args: &[&str]| {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_reins"))
            .args(args)
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("REINS_CONFIG_DIR", dir.path().join("config"))
            .env("REINS_STATE_DIR", &state)
            .env_remove("XDG_RUNTIME_DIR")
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    // With its own state directory, the socket is there (on Windows: that directory's named pipe).
    let socket = if cfg!(windows) {
        reins_desktop::win::ssh_pipe(&state)
    } else {
        state.join("ssh-agent.sock")
    };
    let written = reins_desktop::ssh_agent::setup::config_form(&socket.display().to_string());
    let said = run(&["ssh", "setup"]);
    assert!(said.contains(&socket.display().to_string()), "{said}");
    let text = std::fs::read_to_string(home.join(".ssh/config")).unwrap();
    assert!(text.starts_with(user_config), "{text}");
    assert!(text.contains(&format!("Host *\n    IdentityAgent \"{written}\"")), "{text}");
    assert!(run(&["ssh", "setup"]).contains("already"));
    assert_eq!(std::fs::read_to_string(home.join(".ssh/config")).unwrap(), text);
    let status = run(&["ssh", "status"]);
    assert!(status.contains("not running"), "{status}");
    assert!(status.contains(&format!("uses {written}")), "{status}");
    if have("ssh") {
        // ssh itself reads the block.
        let effective = reins_desktop::ssh_agent::setup::effective_agent(&home.join(".ssh/config")).unwrap_or_default();
        let effective = reins_desktop::ssh_agent::setup::config_form(&effective);
        assert!(effective.eq_ignore_ascii_case(&written), "{effective} is not {written}");
    }
    assert!(run(&["ssh", "unsetup"]).contains("removed"));
    assert_eq!(std::fs::read_to_string(home.join(".ssh/config")).unwrap(), user_config);
    assert!(run(&["ssh", "unsetup"]).contains("no Reins block"));
}
