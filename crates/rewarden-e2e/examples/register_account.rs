//! Creates a Rewarden login on a server that has `SIGNUPS_ALLOWED=true`.
//!
//! ```text
//! REWARDEN_PASSWORD='...' cargo run -p rewarden-e2e --example register_account -- https://rewarden.example.com you@example.com
//! ```
//!
//! The account carries a placeholder vault key: it signs in to Rewarden (which never touches the vault), but it is not
//! meant to be used as a real Bitwarden vault.

use rewarden_core::crypto::{Kdf, master_key, master_password_hash};
use serde_json::json;

const KDF_ITERATIONS: u32 = 600_000;

#[tokio::main]
async fn main() {
    rewarden_e2e::init_tls();
    let mut args = std::env::args().skip(1);
    let (Some(server), Some(email)) = (args.next(), args.next()) else {
        eprintln!("usage: register_account <server-url> <email>   (password in REWARDEN_PASSWORD)");
        std::process::exit(2);
    };
    let password = std::env::var("REWARDEN_PASSWORD").expect("set REWARDEN_PASSWORD");
    let email = email.trim().to_lowercase();

    let hash = {
        let (email, password) = (email.clone(), password.clone());
        tokio::task::spawn_blocking(move || {
            let key = master_key(
                &password,
                &email,
                Kdf::Pbkdf2 {
                    iterations: KDF_ITERATIONS,
                },
            )
            .expect("key derivation");
            master_password_hash(&key, &password).to_string()
        })
        .await
        .expect("hash task")
    };
    let body = json!({
        "email": email, "name": "Rewarden", "masterPasswordHash": hash, "masterPasswordHint": null,
        "key": "2.AAAAAAAAAAAAAAAAAAAAAA==|AAAAAAAAAAAAAAAAAAAAAA==|AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
        "kdf": 0, "kdfIterations": KDF_ITERATIONS
    });
    let response = reqwest::Client::new()
        .post(format!("{}/identity/accounts/register", server.trim_end_matches('/')))
        .json(&body)
        .send()
        .await
        .expect("request");
    let status = response.status();
    println!("{status}");
    if !status.is_success() {
        eprintln!("{}", response.text().await.unwrap_or_default());
        std::process::exit(1);
    }
}
