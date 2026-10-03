//! Host half of the iOS onboarding test (`ios/scripts/live-onboarding.sh`): runs the real server with no account, and
//! plays the desktop app, while the real iOS app on a simulator creates the account and pairs the "computer" by its
//! code.
//!
//! ```text
//! cargo run -p reins-e2e --example ios_onboarding -- /tmp/reins-onboarding
//! # prints SERVER, EMAIL (no account yet) and PASSWORD; once <dir>/created exists, USER_CODE and CONFIRM (what the
//! # computer shows); once the phone approved, PAIRED, then an MCP call with the new session and ALL_DONE.
//! ```

use std::path::{Path, PathBuf};
use std::time::Duration;

use reins_desktop::config::Paths;
use reins_desktop::identity::Identity;
use reins_desktop::server::device::DevicePairing;
use reins_e2e::{PASSWORD, Server};
use serde_json::{Value, json};

const EMAIL: &str = "onboarding@example.com";

async fn wait_file(path: &Path) {
    while !path.exists() {
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

fn fail(why: &str) -> ! {
    println!("FAILED {why}");
    std::process::exit(1);
}

#[tokio::main]
async fn main() {
    reins_e2e::init_tls();
    let dir = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| "/tmp/reins-onboarding".to_owned()));
    std::fs::create_dir_all(&dir).expect("coordination dir");
    let server = Server::start(45, 10).await;
    println!("SERVER {}", server.base);
    println!("EMAIL {EMAIL}");
    println!("PASSWORD {PASSWORD}");

    // The phone created the account and registered as the approval device; the app shows "connect your computer".
    wait_file(&dir.join("created")).await;
    if !server.log().contains("PUT /reins/api/device") {
        fail("the phone never registered as the approval device");
    }
    println!("REGISTERED");

    // `reins login`, as a library: the QR code's link, its code and the number to tap.
    let state = tempfile::tempdir().expect("desktop state");
    let paths = Paths::under(state.path());
    paths.ensure().expect("desktop dirs");
    let identity = Identity::load_or_create(&paths.identity_file()).expect("desktop key");
    let mut pairing = DevicePairing::start(&identity, &server.base).await.unwrap_or_else(|e| fail(&e.to_string()));
    println!("QR {}", pairing.qr_url);
    println!("FINGERPRINT {}", identity.fingerprint());
    println!("USER_CODE {}", pairing.user_code);
    println!("CONFIRM {:02}", pairing.confirm_code.unwrap_or_else(|| fail("no number to tap")));
    let server_base = pairing.wait(&paths).await.unwrap_or_else(|e| fail(&e));
    println!("PAIRED {server_base}");

    // The session works: the MCP endpoint answers this app.
    let session: Value =
        serde_json::from_slice(&std::fs::read(paths.session_file()).expect("session.json")).expect("session JSON");
    let token = session["access_token"].as_str().unwrap_or_else(|| fail("no access token"));
    let listed: Value = reqwest::Client::new()
        .post(server.url("/mcp"))
        .bearer_auth(token)
        .json(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {}}))
        .send()
        .await
        .unwrap_or_else(|e| fail(&e.to_string()))
        .json()
        .await
        .unwrap_or_else(|e| fail(&e.to_string()));
    let tools = listed["result"]["tools"].as_array().map_or(0, Vec::len);
    if tools == 0 {
        fail(&format!("tools/list with the new session: {listed}"));
    }
    println!("TOOLS {tools}");
    if server.log().contains("panicked") {
        fail("the server panicked");
    }
    println!("ALL_DONE");

    // Keep the server up while the app is looked at.
    wait_file(&dir.join("done")).await;
}
