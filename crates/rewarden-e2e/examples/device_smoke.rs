//! Host half of the on-device smoke test: runs the real server and plays the AI client, while a real phone
//! (the Android emulator, via `adb reverse`) approves the connection.
//!
//! ```text
//! cargo run -p rewarden-e2e --example device_smoke
//! # prints SERVER <url>, EMAIL, PASSWORD and CODE; then run the instrumented LiveServerTest with those.
//! ```

use std::time::Duration;

use rewarden_e2e::{AiClient, PASSWORD, Server};
use serde_json::json;

const EMAIL: &str = "phone@example.com";

#[tokio::main]
async fn main() {
    rewarden_e2e::init_tls();
    let server = Server::start(45, 10).await;
    server.register(EMAIL).await;
    println!("SERVER {}", server.base);
    println!("EMAIL {EMAIL}");
    println!("PASSWORD {PASSWORD}");

    // The AI connects after the phone registered as the approval device: wait for the go-ahead file.
    let go = std::path::Path::new("/tmp/rewarden-smoke-go");
    while !go.exists() {
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let mut ai = AiClient::new(&server.base);
    ai.register_client().await;
    let wait_url = ai.start_authorization(EMAIL).await;
    let code = ai.browser_code(&wait_url).await;
    println!("CODE {code}");

    ai.finish_when_approved(&wait_url, Duration::from_secs(300)).await;
    println!("PAIRED");

    // The phone has no Gmail access in the emulator, so the request comes back as an error, not a hang.
    let result = ai.tool("gmail_search", &json!({"query": "from:bank"})).await;
    println!("TOOL_RESULT {result}");

    // Keep the server up until the phone side has finished its own assertions.
    let done = std::path::Path::new("/tmp/rewarden-smoke-done");
    while !done.exists() {
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}
