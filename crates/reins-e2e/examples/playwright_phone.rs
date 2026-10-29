//! The server and the phone for the browser onboarding tests (`playwright/tests/onboarding`).
//!
//! A browser test cannot drive the native phone app, so this plays it: the real phone core (`reins-core`) against the
//! real server, which this also starts and stops. It reads one JSON command per line on stdin and answers each with
//! one JSON line on stdout: `{"ok": ...}` or `{"error": "..."}`.
//!
//! ```text
//! REINS_TEST_SERVER_BINARY=target/debug/vaultwarden cargo run -p reins-e2e --example playwright_phone
//! {"cmd": "start_server"}
//! {"ok": {"base": "http://127.0.0.1:41234"}}
//! ```
//!
//! Commands: `start_server` (`relay_wait`, `offline`), `stop_server`, `create_account` (`email`, optional `server`),
//! `sign_in` (`email`), `sync` (`wait_secs`), `wait_item` (`timeout_secs`), `pairing_view` (`id`), `pairing_by_code`
//! (`code`), `answer_pairing` (`id`, `approve`, `code`, `label`), `approve` (`id`, `messages`), `deny` (`id`),
//! `stock_gmail` (`messages`: `[[id, from], ...]`), `connections`, `revoke` (`id`) and `server_log`.

use std::time::Duration;

use reins_core::{ApprovalChoice, CoreError, PendingItem};
use reins_e2e::{PASSWORD, Phone, Server};
use serde_json::{Value, json};

#[derive(Default)]
struct State {
    server: Option<Server>,
    phone: Option<Phone>,
}

fn text(command: &Value, key: &str) -> Result<String, String> {
    command[key].as_str().map(str::to_owned).ok_or_else(|| format!("`{key}` is missing"))
}

fn item_json(item: &PendingItem) -> Value {
    json!({"id": item.id, "title": item.title, "subtitle": item.subtitle, "action": item.action,
        "connection_label": item.connection_label})
}

fn core_error(error: &CoreError) -> String {
    error.to_string()
}

impl State {
    fn server(&self) -> Result<&Server, String> {
        self.server.as_ref().ok_or_else(|| "no server is running".to_owned())
    }

    fn phone(&self) -> Result<&Phone, String> {
        self.phone.as_ref().ok_or_else(|| "the phone is not signed in".to_owned())
    }

    /// A phone that creates its account on `server` (or the running one), as on first run, and becomes the account's
    /// approval device with Gmail connected. Unlike [`Phone::create_account`], a failure is an answer, not a panic.
    async fn create_account(&mut self, command: &Value) -> Result<Value, String> {
        let email = text(command, "email")?;
        let server = match command["server"].as_str() {
            Some(server) => server.to_owned(),
            None => self.server()?.base.clone(),
        };
        let phone = Phone::signed_out(|_| {}).await;
        phone.core.create_account(server, email, PASSWORD.to_owned()).await.map_err(|e| core_error(&e))?;
        phone.core.register_device(None).await.map_err(|e| core_error(&e))?;
        drop(phone.core.set_send_hold_secs(0).await);
        phone.connect_gmail().await;
        self.phone = Some(phone);
        Ok(json!({}))
    }

    async fn wait_item(&self, command: &Value) -> Result<Value, String> {
        let phone = self.phone()?;
        let timeout = Duration::from_secs(command["timeout_secs"].as_u64().unwrap_or(20));
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let items = phone.core.sync(2).await.map_err(|e| core_error(&e))?;
            if let Some(item) = items.first() {
                return Ok(item_json(item));
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(format!("no item arrived within {timeout:?}"));
            }
        }
    }

    async fn run(&mut self, command: &Value) -> Result<Value, String> {
        let id = || text(command, "id");
        match command["cmd"].as_str().unwrap_or_default() {
            "start_server" => {
                let relay_wait = command["relay_wait"].as_u64().unwrap_or(30);
                let offline = command["offline"].as_u64().unwrap_or(10);
                let server = Server::start(relay_wait, offline).await;
                let base = server.base.clone();
                self.server = Some(server);
                Ok(json!({"base": base}))
            }
            "stop_server" => {
                drop(self.server.take());
                Ok(json!({}))
            }
            "server_log" => Ok(json!({"log": self.server()?.log()})),
            "create_account" => self.create_account(command).await,
            "sign_in" => {
                let email = text(command, "email")?;
                let server = self.server()?;
                server.register(&email).await;
                self.phone = Some(Phone::sign_in(&server.base, &email).await);
                Ok(json!({}))
            }
            "sync" => {
                let wait = u32::try_from(command["wait_secs"].as_u64().unwrap_or(0)).unwrap_or(0);
                let items = self.phone()?.core.sync(wait).await.map_err(|e| core_error(&e))?;
                Ok(Value::Array(items.iter().map(item_json).collect()))
            }
            "wait_item" => self.wait_item(command).await,
            "pairing_view" => {
                let view = self.phone()?.core.pairing_view(id()?).await.map_err(|e| core_error(&e))?;
                Ok(json!({"id": view.id, "client_name": view.client_name, "client_host": view.client_host,
                    "choices": view.choices}))
            }
            "pairing_by_code" => {
                let view = self.phone()?.core.pairing_by_code(text(command, "code")?).await.map_err(|e| core_error(&e))?;
                Ok(json!({"id": view.id, "client_name": view.client_name, "client_host": view.client_host,
                    "choices": view.choices}))
            }
            "answer_pairing" => {
                let approve = command["approve"].as_bool().unwrap_or(false);
                let code = command["code"].as_u64().and_then(|c| u8::try_from(c).ok());
                let label = command["label"].as_str().map(str::to_owned);
                let named = label.is_some();
                self.phone()?.core.answer_pairing(id()?, approve, code, label, named).await.map_err(|e| core_error(&e))?;
                Ok(json!({}))
            }
            "approve" => {
                let messages = command["messages"]
                    .as_array()
                    .map(|ids| ids.iter().filter_map(Value::as_str).map(str::to_owned).collect())
                    .unwrap_or_default();
                let choice = ApprovalChoice {
                    selected_message_ids: messages,
                    standing: None,
                };
                self.phone()?.core.approve(id()?, choice).await.map_err(|e| core_error(&e))?;
                Ok(json!({}))
            }
            "deny" => {
                self.phone()?.core.deny(id()?).await.map_err(|e| core_error(&e))?;
                Ok(json!({}))
            }
            "stock_gmail" => {
                let pairs: Vec<(String, String)> = command["messages"]
                    .as_array()
                    .map(|all| {
                        all.iter()
                            .filter_map(|m| Some((m[0].as_str()?.to_owned(), m[1].as_str()?.to_owned())))
                            .collect()
                    })
                    .unwrap_or_default();
                let borrowed: Vec<(&str, &str)> = pairs.iter().map(|(i, f)| (i.as_str(), f.as_str())).collect();
                self.phone()?.stock_gmail(&borrowed).await;
                Ok(json!({}))
            }
            "connections" => {
                let all = self.phone()?.core.connections().await.map_err(|e| core_error(&e))?;
                Ok(Value::Array(all.iter().map(|c| json!({"id": c.id, "label": c.label})).collect()))
            }
            "revoke" => {
                self.phone()?.core.revoke_connection(id()?).await.map_err(|e| core_error(&e))?;
                Ok(json!({}))
            }
            other => Err(format!("unknown command `{other}`")),
        }
    }
}

#[tokio::main]
async fn main() {
    reins_e2e::init_tls();
    let mut state = State::default();
    loop {
        // Blocking stdin on a blocking thread: one command at a time, in order.
        let read = tokio::task::spawn_blocking(|| {
            let mut line = String::new();
            std::io::stdin().read_line(&mut line).map(|n| (n, line))
        });
        let Ok(Ok((read, line))) = read.await else { break };
        if read == 0 {
            break;
        }
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(command) => match state.run(&command).await {
                Ok(value) => json!({"ok": value}),
                Err(error) => json!({"error": error}),
            },
            Err(error) => json!({"error": format!("not JSON: {error}")}),
        };
        println!("{reply}");
    }
}
