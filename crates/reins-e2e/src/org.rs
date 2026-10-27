//! Shared helpers for the organization end-to-end tests: a cookie-keeping browser for the admin console, signed
//! WorkOS webhook deliveries, the server's database, and the phone's org enrollment.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use reins_core::org::{AttestationInput, AttestationKindInput, EnrollInput, PostureInput};
use serde_json::{Value, json};

use crate::{Phone, Server};

/// The WorkOS webhook secret the org tests start the server with.
pub const WEBHOOK_SECRET: &str = "whsec_orgs";

/// A browser that keeps cookies and follows redirects by hand (the console signs in through several hosts).
pub struct Browser {
    http: reqwest::Client,
    cookies: HashMap<String, String>,
}

pub struct Page {
    pub url: String,
    pub status: u16,
    pub body: String,
}

impl Browser {
    pub fn new() -> Self {
        crate::init_tls();
        Self {
            http: reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().unwrap(),
            cookies: HashMap::new(),
        }
    }

    fn cookie_header(&self) -> String {
        self.cookies.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("; ")
    }

    fn keep_cookies(&mut self, resp: &reqwest::Response) {
        for value in resp.headers().get_all("set-cookie") {
            let pair = value.to_str().unwrap().split(';').next().unwrap_or_default();
            if let Some((k, v)) = pair.split_once('=') {
                if v.is_empty() {
                    self.cookies.remove(k.trim());
                } else {
                    self.cookies.insert(k.trim().to_owned(), v.trim().to_owned());
                }
            }
        }
    }

    async fn follow(&mut self, mut resp: reqwest::Response, mut url: String) -> Page {
        for _ in 0..10 {
            self.keep_cookies(&resp);
            if !resp.status().is_redirection() {
                let status = resp.status().as_u16();
                return Page {
                    url,
                    status,
                    body: resp.text().await.unwrap(),
                };
            }
            let location = resp.headers()["location"].to_str().unwrap().to_owned();
            url = url::Url::parse(&url).unwrap().join(&location).unwrap().to_string();
            resp = self.http.get(&url).header("Cookie", self.cookie_header()).send().await.unwrap();
        }
        panic!("too many redirects from {url}");
    }

    pub async fn get(&mut self, url: &str) -> Page {
        let resp = self.http.get(url).header("Cookie", self.cookie_header()).send().await.unwrap();
        self.follow(resp, url.to_owned()).await
    }

    pub async fn post(&mut self, url: &str, form: &[(&str, &str)]) -> Page {
        let resp = self.http.post(url).header("Cookie", self.cookie_header()).form(form).send().await.unwrap();
        self.follow(resp, url.to_owned()).await
    }
}

pub fn between<'a>(text: &'a str, before: &str, after: &str) -> &'a str {
    let start = text.find(before).unwrap_or_else(|| panic!("{before:?} not in page:\n{text}")) + before.len();
    let end = text[start..].find(after).unwrap() + start;
    &text[start..end]
}

pub fn csrf(page: &Page) -> String {
    between(&page.body, "name=\"csrf\" value=\"", "\"").to_owned()
}

pub fn dir_user(id: &str, email: &str, state: &str) -> Value {
    json!({"object": "directory_user", "id": id, "directory_id": "directory_ACME", "organization_id": "org_ACME",
           "first_name": "Lee", "last_name": "Park", "state": state,
           "emails": [{"primary": true, "type": "work", "value": email}]})
}

/// A signed WorkOS webhook delivery: it only wakes the syncs; its body is never applied.
pub async fn webhook(server: &Server, body: &Value) {
    let body = body.to_string();
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis();
    let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, WEBHOOK_SECRET.as_bytes());
    let tag = ring::hmac::sign(&key, format!("{t}.{body}").as_bytes());
    let header = format!("t={t}, v1={}", data_encoding::HEXLOWER.encode(tag.as_ref()));
    let r = reqwest::Client::new()
        .post(server.url("/reins/workos/webhook"))
        .header("WorkOS-Signature", header)
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
}

pub fn db(server: &Server) -> rusqlite::Connection {
    let c = rusqlite::Connection::open(server.database_path()).unwrap();
    c.busy_timeout(Duration::from_secs(5)).unwrap();
    c
}

pub fn member_state(server: &Server, email: &str) -> Option<String> {
    db(server)
        .query_row(
            "SELECT m.state FROM reins_org_members m JOIN users u ON u.uuid = m.user_uuid WHERE u.email = ?1",
            [email],
            |r| r.get(0),
        )
        .ok()
}

pub fn user_exists(server: &Server, email: &str) -> bool {
    db(server).query_row("SELECT COUNT(*) FROM users WHERE email = ?1", [email], |r| r.get::<_, i64>(0)).unwrap() > 0
}

pub fn devices_of(server: &Server, email: &str) -> i64 {
    db(server)
        .query_row(
            "SELECT COUNT(*) FROM devices d JOIN users u ON u.uuid = d.user_uuid WHERE u.email = ?1",
            [email],
            |r| r.get(0),
        )
        .unwrap()
}

pub async fn wait_for(what: &str, timeout: Duration, mut done: impl FnMut() -> bool) -> Duration {
    let start = Instant::now();
    while !done() {
        assert!(start.elapsed() < timeout, "{what} did not happen within {timeout:?}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    start.elapsed()
}

/// "Add work account" on a phone: the sign-in goes to the org that verified `domain`.
pub async fn add_work_account(phone: &Phone, server: &Server, domain: &str) -> Result<(), reins_core::CoreError> {
    let start = phone.core.sso_begin_work(server.base.clone(), domain.to_owned()).await?;
    let callback = crate::workos::browse_to_callback(&start.url, &start.callback_scheme).await;
    phone.core.sso_finish(server.base.clone(), callback, start.state, start.verifier).await.map(drop)
}

/// The phone's org device key (P-256) and its enrollment for `challenge`.
pub fn enrollment(challenge: &str) -> EnrollInput {
    enrollment_with_key(challenge).0
}

/// An enrollment for `challenge` and the device key's PKCS#8, so a test can sign as that phone.
pub fn enrollment_with_key(challenge: &str) -> (EnrollInput, Vec<u8>) {
    use ring::signature::{ECDSA_P256_SHA256_ASN1_SIGNING, EcdsaKeyPair, KeyPair};
    let rng = ring::rand::SystemRandom::new();
    let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng).unwrap();
    let key = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, pkcs8.as_ref(), &rng).unwrap();
    let mut spki = vec![
        0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08, 0x2a, 0x86, 0x48,
        0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00,
    ];
    spki.extend_from_slice(key.public_key().as_ref());
    let message = reins_core::profiles::org_enrollment_message(challenge.to_owned());
    let input = EnrollInput {
        device_key: data_encoding::BASE64.encode(&spki),
        signature: data_encoding::BASE64.encode(key.sign(&rng, &message).unwrap().as_ref()),
        hardware: true,
        attestations: vec![AttestationInput {
            kind: AttestationKindInput::None,
            data: String::new(),
        }],
        posture: posture(false),
    };
    (input, pkcs8.as_ref().to_vec())
}

/// A phone's posture: a screen lock, a current OS, `rooted` as given.
pub fn posture(rooted: bool) -> PostureInput {
    PostureInput {
        screen_lock: true,
        os_name: "Android".to_owned(),
        os_version: "16".to_owned(),
        rooted: Some(rooted),
        app_version: "1.0.0".to_owned(),
    }
}

/// Signs `message` with an enrolled device key (DER ECDSA P-256, base64), as the phone's secure hardware does.
pub fn device_sign(pkcs8: &[u8], message: &[u8]) -> String {
    use ring::signature::{ECDSA_P256_SHA256_ASN1_SIGNING, EcdsaKeyPair};
    let rng = ring::rand::SystemRandom::new();
    let key = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, pkcs8, &rng).unwrap();
    data_encoding::BASE64.encode(key.sign(&rng, message).unwrap().as_ref())
}
