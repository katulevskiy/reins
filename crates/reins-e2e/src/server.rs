//! Builds and runs the real server binary against a temporary SQLite database.

use std::fs::File;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use reins_core::crypto::{Kdf, master_key, master_password_hash};
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
        if let Some(path) = std::env::var_os("REINS_TEST_SERVER_BINARY") {
            let binary = PathBuf::from(path).canonicalize().expect("the prebuilt test server must exist");
            assert!(binary.is_file(), "REINS_TEST_SERVER_BINARY must be a file");
            return binary;
        }
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
        // Finish the potentially long build before selecting a port.
        binary();
        for _ in 0..5 {
            let base = format!("http://127.0.0.1:{}", free_port());
            match Self::try_start_at(&base, relay_wait, offline, env).await {
                Ok(server) => return server,
                Err(log) if log.contains("Address already in use") => {}
                Err(log) => panic!("server did not start:\n{log}"),
            }
        }
        panic!("server could not bind a free port after five attempts");
    }

    /// Like [`Server::start_with_env`] at `base` (`http://localhost:8765`: a fixed address an identity provider
    /// knows as a redirect URI).
    pub async fn start_at(base: &str, relay_wait: u64, offline: u64, env: &[(String, String)]) -> Self {
        // An identity provider knows this fixed URL; retrying another port would
        // invalidate its redirect URI, so surface a collision immediately.
        Self::try_start_at(base, relay_wait, offline, env)
            .await
            .unwrap_or_else(|log| panic!("server did not start:\n{log}"))
    }

    async fn try_start_at(base: &str, relay_wait: u64, offline: u64, env: &[(String, String)]) -> Result<Self, String> {
        let bin = binary().to_owned();
        let port = url::Url::parse(base).ok().and_then(|u| u.port()).expect("a base with a port");
        let base = base.to_owned();
        let dir = std::env::temp_dir().join(format!("reins-e2e-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let log = File::create(dir.join("server.log")).expect("log");
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
            .env("REINS_ENABLED", "true")
            .env("REINS_RELAY_WAIT_SECS", relay_wait.to_string())
            .env("REINS_OFFLINE_SECS", offline.to_string())
            // The tests' fake GitHub and MCP servers listen on loopback over http.
            .env("REINS_TEST_ALLOW_LOOPBACK", "1")
            .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .stdout(log.try_clone().expect("clone"))
            .stderr(log);
        let child = command.spawn().expect("spawn vaultwarden");
        let mut server = Self {
            base,
            child,
            dir,
        };
        server.wait_alive().await?;
        Ok(server)
    }

    fn http() -> reqwest::Client {
        crate::init_tls();
        reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().expect("client")
    }

    async fn wait_alive(&mut self) -> Result<(), String> {
        let deadline = Instant::now() + Duration::from_secs(60);
        crate::init_tls();
        let http = reqwest::Client::builder().timeout(Duration::from_secs(1)).build().expect("startup client");
        let launched = format!("Rocket has launched from {}", self.base.replace("localhost", "127.0.0.1"));
        while Instant::now() < deadline {
            if self.child.try_wait().expect("server process status").is_some() {
                return Err(self.log());
            }
            if self.log().contains(&launched)
                && let Ok(r) = http.get(self.url("/alive")).send().await
                && r.status().is_success()
                && self.child.try_wait().expect("server process status").is_none()
            {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Err(format!("startup timed out:\n{}", self.log()))
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

    /// The isolated test database, for fault injection and checking durable lifecycle state.
    pub fn database_path(&self) -> PathBuf {
        self.dir.join("db.sqlite3")
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.child.kill().ok();
        self.child.wait().ok();
        std::fs::remove_dir_all(&self.dir).ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};

    #[tokio::test]
    async fn startup_rejects_a_mock_answering_alive_on_the_reserved_port() {
        binary();
        let mock = MockServer::start().await;
        Mock::given(path("/alive")).respond_with(ResponseTemplate::new(200)).mount(&mock).await;
        let started = Instant::now();
        let Err(error) = Server::try_start_at(&mock.uri(), 4, 2, &[]).await else {
            panic!("accepted a mock as the child server");
        };
        assert!(error.contains("Address already in use"), "{error}");
        assert!(started.elapsed() < Duration::from_secs(20), "did not notice the child exited");
        assert!(mock.received_requests().await.expect("mock requests").is_empty(), "probed an unrelated server");
    }
}
