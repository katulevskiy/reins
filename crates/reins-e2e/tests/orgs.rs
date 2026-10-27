//! Organizations (Decisions v2 · O1) end to end: the real server with organizations on, a fake WorkOS (sign-in,
//! organizations, directories, Directory Sync events) and real phone cores. A pilot org is created in the admin
//! console, verifies its domain, binds its SSO and directory, provisions a member through SCIM, the member signs in
//! with "Add work account" and enrolls the phone, and a SCIM delete removes them within the minute.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use reins_core::org::{AttestationInput, AttestationKindInput, EnrollInput, PostureInput};
use reins_e2e::workos::{FakeWorkos, User};
use reins_e2e::{Phone, Server};
use serde_json::{Value, json};

const WEBHOOK_SECRET: &str = "whsec_orgs";

/// A browser that keeps cookies and follows redirects by hand (the console signs in through several hosts).
struct Browser {
    http: reqwest::Client,
    cookies: HashMap<String, String>,
}

struct Page {
    url: String,
    status: u16,
    body: String,
}

impl Browser {
    fn new() -> Self {
        reins_e2e::init_tls();
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

    async fn get(&mut self, url: &str) -> Page {
        let resp = self.http.get(url).header("Cookie", self.cookie_header()).send().await.unwrap();
        self.follow(resp, url.to_owned()).await
    }

    async fn post(&mut self, url: &str, form: &[(&str, &str)]) -> Page {
        let resp = self.http.post(url).header("Cookie", self.cookie_header()).form(form).send().await.unwrap();
        self.follow(resp, url.to_owned()).await
    }
}

fn between<'a>(text: &'a str, before: &str, after: &str) -> &'a str {
    let start = text.find(before).unwrap_or_else(|| panic!("{before:?} not in page:\n{text}")) + before.len();
    let end = text[start..].find(after).unwrap() + start;
    &text[start..end]
}

fn csrf(page: &Page) -> String {
    between(&page.body, "name=\"csrf\" value=\"", "\"").to_owned()
}

fn dir_user(id: &str, email: &str, state: &str) -> Value {
    json!({"object": "directory_user", "id": id, "directory_id": "directory_ACME", "organization_id": "org_ACME",
           "first_name": "Lee", "last_name": "Park", "state": state,
           "emails": [{"primary": true, "type": "work", "value": email}]})
}

/// A signed WorkOS webhook delivery: it only wakes the syncs; its body is never applied.
async fn webhook(server: &Server, body: &Value) {
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

fn db(server: &Server) -> rusqlite::Connection {
    let c = rusqlite::Connection::open(server.database_path()).unwrap();
    c.busy_timeout(Duration::from_secs(5)).unwrap();
    c
}

fn member_state(server: &Server, email: &str) -> Option<String> {
    db(server)
        .query_row(
            "SELECT m.state FROM reins_org_members m JOIN users u ON u.uuid = m.user_uuid WHERE u.email = ?1",
            [email],
            |r| r.get(0),
        )
        .ok()
}

fn devices_of(server: &Server, email: &str) -> i64 {
    db(server)
        .query_row(
            "SELECT COUNT(*) FROM devices d JOIN users u ON u.uuid = d.user_uuid WHERE u.email = ?1",
            [email],
            |r| r.get(0),
        )
        .unwrap()
}

async fn wait_for(what: &str, timeout: Duration, mut done: impl FnMut() -> bool) -> Duration {
    let start = Instant::now();
    while !done() {
        assert!(start.elapsed() < timeout, "{what} did not happen within {timeout:?}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    start.elapsed()
}

/// "Add work account" on a phone: the sign-in goes to the org that verified `domain`.
async fn add_work_account(phone: &Phone, server: &Server, domain: &str) -> Result<(), reins_core::CoreError> {
    let start = phone.core.sso_begin_work(server.base.clone(), domain.to_owned()).await?;
    let callback = reins_e2e::workos::browse_to_callback(&start.url, &start.callback_scheme).await;
    phone.core.sso_finish(server.base.clone(), callback, start.state, start.verifier).await.map(drop)
}

/// The phone's org device key (P-256) and its enrollment for `challenge`.
fn enrollment(challenge: &str) -> EnrollInput {
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
    EnrollInput {
        device_key: data_encoding::BASE64.encode(&spki),
        signature: data_encoding::BASE64.encode(key.sign(&rng, &message).unwrap().as_ref()),
        hardware: true,
        attestations: vec![AttestationInput {
            kind: AttestationKindInput::None,
            data: String::new(),
        }],
        posture: PostureInput {
            screen_lock: true,
            os_name: "Android".to_owned(),
            os_version: "16".to_owned(),
            rooted: Some(false),
            app_version: "1.0.0".to_owned(),
        },
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pilot_org_is_provisioned_and_deprovisioned_end_to_end() {
    let workos = FakeWorkos::start().await;
    workos.add_organization("org_ACME", &["acme.com"]);
    workos.add_directory("directory_ACME", "org_ACME");
    let scratch = tempfile::tempdir().unwrap();
    let txt = scratch.path().join("txt");
    std::fs::write(&txt, "").unwrap();
    let mut env = workos.server_env();
    env.extend(
        [
            ("REINS_ORGS_ENABLED", "true".to_owned()),
            // Only a webhook can make the directory sync fast: the timing below is the webhook path's.
            ("REINS_WORKOS_DSYNC_SECS", "3600".to_owned()),
            ("REINS_WORKOS_WEBHOOK_SECRET", WEBHOOK_SECRET.to_owned()),
            ("REINS_TEST_DNS_TXT_FILE", txt.to_string_lossy().into_owned()),
        ]
        .map(|(k, v)| (k.to_owned(), v)),
    );
    let server = Server::start_with_env(5, 3, &env).await;

    // The founder's account (dana@acme.com) exists: she signed in on her phone.
    let dana = User {
        id: "user_dana".to_owned(),
        email: "dana@acme.com".to_owned(),
    };
    workos.sign_in_as(&dana);
    let dana_phone = Phone::signed_out(|_| {}).await;
    dana_phone.sso_sign_in(&server.base).await.unwrap();

    // The console: sign in, create the org.
    let mut browser = Browser::new();
    let home = browser.get(&server.url("/reins/org/login")).await;
    assert_eq!(home.status, 200, "{}", home.body);
    assert!(home.body.contains("New organization"), "{}", home.body);
    let created = browser
        .post(&server.url("/reins/org/new"), &[("csrf", &csrf(&home)), ("name", "Acme"), ("tier", "business")])
        .await;
    let org = created.url.rsplit('/').next().unwrap().to_owned();
    assert!(created.body.contains("_reins-challenge.acme.com"), "{}", created.body);

    // The domain, through DNS.
    let record = between(&created.body, "_reins-challenge.acme.com<br>", "</div>").to_owned();
    std::fs::write(&txt, format!("_reins-challenge.acme.com={record}\n")).unwrap();
    let verified = browser
        .post(
            &server.url(&format!("/reins/org/{org}/domains/verify")),
            &[("csrf", &csrf(&created)), ("domain", "acme.com")],
        )
        .await;
    assert!(verified.body.contains("Verified"), "{}", verified.body);

    // SSO and the directory, bound only with WorkOS's word.
    let refused = browser
        .post(
            &server.url(&format!("/reins/org/{org}/identity")),
            &[
                ("csrf", &csrf(&verified)),
                ("identity", "sso"),
                ("workos_org_id", "org_OTHER"),
                ("directory_id", ""),
                ("github_org", ""),
            ],
        )
        .await;
    assert!(refused.body.contains("WorkOS didn"), "{}", refused.body);
    let bound = browser
        .post(
            &server.url(&format!("/reins/org/{org}/identity")),
            &[
                ("csrf", &csrf(&refused)),
                ("identity", "sso"),
                ("workos_org_id", "org_ACME"),
                ("directory_id", "directory_ACME"),
                ("github_org", ""),
            ],
        )
        .await;
    assert!(bound.body.contains("Saved"), "{}", bound.body);
    assert!(bound.body.contains("org_ACME") && bound.body.contains("directory_ACME"), "{}", bound.body);

    // SCIM: the directory adds Lee; the webhook wakes the sync.
    workos.emit("dsync.user.created", dir_user("directory_user_lee", "lee@acme.com", "active"));
    webhook(&server, &json!({"id": "wh_1", "event": "dsync.user.created"})).await;
    wait_for("provisioning", Duration::from_secs(20), || {
        member_state(&server, "lee@acme.com").as_deref() == Some("active")
    })
    .await;

    // Lee adds the work account on a phone: straight to Acme's SSO.
    let lee = User {
        id: "user_lee".to_owned(),
        email: "lee@acme.com".to_owned(),
    };
    workos.sign_in_as(&lee);
    let lee_phone = Phone::signed_out(|_| {}).await;
    add_work_account(&lee_phone, &server, "acme.com").await.unwrap();
    lee_phone.core.register_device(None).await.unwrap();
    let membership = lee_phone.core.org_membership().await.unwrap().expect("a work account");
    assert_eq!((membership.name.as_str(), membership.enrolled), ("Acme", false));
    let challenge = lee_phone.core.org_challenge().await.unwrap();
    lee_phone.core.org_enroll(challenge.clone(), enrollment(&challenge)).await.unwrap();
    assert!(lee_phone.core.org_membership().await.unwrap().unwrap().enrolled);
    // A challenge is good once.
    assert!(lee_phone.core.org_enroll(challenge.clone(), enrollment(&challenge)).await.is_err());

    // A Personal account sees no org (the same answer as a server without orgs).
    workos.sign_in_as(&User {
        id: "user_bob".to_owned(),
        email: "bob@example.com".to_owned(),
    });
    let bob = Phone::signed_out(|_| {}).await;
    bob.sso_sign_in(&server.base).await.unwrap();
    assert_eq!(bob.core.org_membership().await.unwrap(), None);

    // A forged webhook body names Dana: nobody is removed (only events read from WorkOS count).
    webhook(
        &server,
        &json!({"id": "wh_forged", "event": "dsync.user.deleted",
                             "data": dir_user("directory_user_dana", "dana@acme.com", "inactive")}),
    )
    .await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(member_state(&server, "dana@acme.com").as_deref(), Some("active"));

    // The directory deletes Lee: everything of Lee's goes, well within the minute.
    assert!(devices_of(&server, "lee@acme.com") > 0);
    workos.emit("dsync.user.deleted", dir_user("directory_user_lee", "lee@acme.com", "inactive"));
    let started = Instant::now();
    webhook(&server, &json!({"id": "wh_2", "event": "dsync.user.deleted"})).await;
    let took = wait_for("deprovisioning", Duration::from_secs(60), || {
        devices_of(&server, "lee@acme.com") == 0
            && member_state(&server, "lee@acme.com").as_deref() == Some("deprovisioned_by_directory")
    })
    .await;
    assert!(started.elapsed() < Duration::from_secs(60), "deprovisioning took {took:?}");
    assert!(lee_phone.core.org_membership().await.is_err(), "the phone's session is gone");

    // The console's events show it, and the chain is intact.
    let events = browser.get(&server.url(&format!("/reins/org/{org}/events"))).await;
    for kind in [
        "org.created",
        "domain.verified",
        "change.applied",
        "member.provisioned",
        "device.enrolled",
        "member.deprovisioned",
    ] {
        assert!(events.body.contains(kind), "{kind} missing:\n{}", events.body);
    }
    assert!(events.body.contains("Chain intact"), "{}", events.body);
    // Dana and Bob were never touched.
    assert_eq!(member_state(&server, "dana@acme.com").as_deref(), Some("active"));
    assert_eq!(member_state(&server, "bob@example.com"), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_console_needs_its_session_and_csrf() {
    let workos = FakeWorkos::start().await;
    let mut env = workos.server_env();
    env.push(("REINS_ORGS_ENABLED".to_owned(), "true".to_owned()));
    let server = Server::start_with_env(5, 3, &env).await;
    let mut anonymous = Browser::new();
    let page = anonymous.get(&server.url("/reins/org")).await;
    assert!(page.body.contains("Sign in"), "{}", page.body);
    let page = anonymous.get(&server.url("/reins/org/00000000-0000-0000-0000-000000000000")).await;
    assert!(page.body.contains("Not found"));

    let dana = User {
        id: "user_dana2".to_owned(),
        email: "dana@acme.io".to_owned(),
    };
    workos.sign_in_as(&dana);
    Phone::signed_out(|_| {}).await.sso_sign_in(&server.base).await.unwrap();
    let mut browser = Browser::new();
    let home = browser.get(&server.url("/reins/org/login")).await;
    // Without the CSRF token, nothing is created.
    let page =
        browser.post(&server.url("/reins/org/new"), &[("csrf", "forged"), ("name", "Acme"), ("tier", "team")]).await;
    assert!(!page.body.contains("_reins-challenge"), "{}", page.body);
    let count: i64 = db(&server).query_row("SELECT COUNT(*) FROM reins_orgs", [], |r| r.get(0)).unwrap();
    assert_eq!(count, 0);
    // With it, it is.
    browser.post(&server.url("/reins/org/new"), &[("csrf", &csrf(&home)), ("name", "Acme"), ("tier", "team")]).await;
    let count: i64 = db(&server).query_row("SELECT COUNT(*) FROM reins_orgs", [], |r| r.get(0)).unwrap();
    assert_eq!(count, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_work_account_password_is_refused_only_after_a_correct_password() {
    let server = Server::start_with_env(5, 3, &[("REINS_ORGS_ENABLED".to_owned(), "true".to_owned())]).await;
    let phone = Phone::create_account(&server.base, "kim@acme.dev").await;
    phone.core.logout().await.unwrap();
    // Make the account a member of an org, as the console or SCIM would.
    {
        let c = db(&server);
        let user: String =
            c.query_row("SELECT uuid FROM users WHERE email = 'kim@acme.dev'", [], |r| r.get(0)).unwrap();
        c.execute_batch(&format!(
            "INSERT INTO organizations (uuid, name, billing_email) VALUES ('org-1', 'Acme', 'kim@acme.dev');
             INSERT INTO reins_orgs (org_uuid, tier, identity, two_person, created_by, created_at, updated_at)
               VALUES ('org-1', 'business', 'none', 1, '{user}', 1, 1);
             INSERT INTO users_organizations (uuid, user_uuid, org_uuid, access_all, akey, status, atype)
               VALUES ('m-1', '{user}', 'org-1', 0, '', 1, 2);
             INSERT INTO reins_org_members (org_uuid, user_uuid, membership_uuid, state, changed_at)
               VALUES ('org-1', '{user}', 'm-1', 'active', 1);"
        ))
        .unwrap();
    }
    let wrong = phone.core.login(server.base.clone(), "kim@acme.dev".to_owned(), "wrong".to_owned(), None).await;
    let wrong = format!("{:?}", wrong.unwrap_err());
    assert!(!wrong.contains("work account"), "a wrong password must not tell work accounts apart: {wrong}");
    let right =
        phone.core.login(server.base.clone(), "kim@acme.dev".to_owned(), reins_e2e::PASSWORD.to_owned(), None).await;
    let right = format!("{:?}", right.unwrap_err());
    assert!(right.contains("work account"), "{right}");
}
