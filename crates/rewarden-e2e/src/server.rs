//! Builds and runs the real server binary against a temporary SQLite database.

use std::fs::File;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use rewarden_core::crypto::{Kdf, master_key, master_password_hash};
use serde_json::json;

use crate::PASSWORD;

/// Default PBKDF2 iterations of Vaultwarden accounts.
const KDF_ITERATIONS: u32 = 600_000;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().expect("repo root")
}

/// `target/debug/vaultwarden` (or under `CARGO_TARGET_DIR` when set), built once per test process.
fn binary() -> &'static Path {
    static BIN: OnceLock<PathBuf> = OnceLock::new();
    BIN.get_or_init(|| {
        let root = repo_root();
        let status = Command::new("cargo")
            .args(["build", "--features", "sqlite", "--bin", "vaultwarden"])
            .current_dir(&root)
            .status()
            .expect("cargo build");
        assert!(status.success(), "building the server failed");
        let target = std::env::var_os("CARGO_TARGET_DIR").map_or_else(|| root.join("target"), PathBuf::from);
        target.join("debug/vaultwarden")
    })
}

pub struct Server {
    pub base: String,
    child: Child,
    dir: PathBuf,
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").expect("bind").local_addr().expect("addr").port()
}

impl Server {
    /// Starts a server with short relay timings (`wait` and `offline` in seconds).
    pub async fn start(relay_wait: u64, offline: u64) -> Self {
        Self::start_with_env(relay_wait, offline, &[]).await
    }

    /// Like [`Server::start`], with these environment settings added (SSO, ...).
    pub async fn start_with_env(relay_wait: u64, offline: u64, env: &[(String, String)]) -> Self {
        let bin = binary().to_owned();
        let port = free_port();
        let dir = std::env::temp_dir().join(format!("rewarden-e2e-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let log = File::create(dir.join("server.log")).expect("log");
        let base = format!("http://127.0.0.1:{port}");
        let mut command = Command::new(bin);
        command
            .current_dir(&dir)
            .env_clear()
            .env("DATA_FOLDER", &dir)
            .env("DOMAIN", &base)
            .env("ROCKET_ADDRESS", "127.0.0.1")
            .env("ROCKET_PORT", port.to_string())
            .env("WEB_VAULT_ENABLED", "false")
            .env("LOG_LEVEL", "info")
            .env("REWARDEN_ENABLED", "true")
            .env("REWARDEN_RELAY_WAIT_SECS", relay_wait.to_string())
            .env("REWARDEN_OFFLINE_SECS", offline.to_string())
            // The tests' fake GitHub and MCP servers listen on loopback over http.
            .env("REWARDEN_TEST_ALLOW_LOOPBACK", "1")
            .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .stdout(log.try_clone().expect("clone"))
            .stderr(log);
        let child = command.spawn().expect("spawn vaultwarden");
        let server = Self {
            base,
            child,
            dir,
        };
        server.wait_alive().await;
        server
    }

    fn http() -> reqwest::Client {
        crate::init_tls();
        reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().expect("client")
    }

    async fn wait_alive(&self) {
        let deadline = Instant::now() + Duration::from_secs(60);
        while Instant::now() < deadline {
            if let Ok(r) = Self::http().get(self.url("/alive")).send().await
                && r.status().is_success()
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let log = std::fs::read_to_string(self.dir.join("server.log")).unwrap_or_default();
        panic!("server did not start:\n{log}");
    }

    pub fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    /// Registers `email` with the real master-password hash, exactly as a Bitwarden client would.
    pub async fn register(&self, email: &str) {
        let hash = {
            let email = email.to_owned();
            tokio::task::spawn_blocking(move || {
                let key = master_key(
                    PASSWORD,
                    &email,
                    Kdf::Pbkdf2 {
                        iterations: KDF_ITERATIONS,
                    },
                )
                .expect("kdf");
                master_password_hash(&key, PASSWORD).to_string()
            })
            .await
            .expect("hash task")
        };
        let body = json!({
            "email": email, "name": "E2E", "masterPasswordHash": hash, "masterPasswordHint": null,
            "key": "2.AAAAAAAAAAAAAAAAAAAAAA==|AAAAAAAAAAAAAAAAAAAAAA==|AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
            "kdf": 0, "kdfIterations": KDF_ITERATIONS
        });
        let r = Self::http().post(self.url("/identity/accounts/register")).json(&body).send().await.expect("register");
        let status = r.status();
        assert!(status.is_success(), "register: {status} {}", r.text().await.unwrap_or_default());
    }

    pub fn log(&self) -> String {
        std::fs::read_to_string(self.dir.join("server.log")).unwrap_or_default()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.child.kill().ok();
        self.child.wait().ok();
        std::fs::remove_dir_all(&self.dir).ok();
    }
}
