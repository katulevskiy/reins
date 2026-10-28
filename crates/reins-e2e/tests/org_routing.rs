//! Organizations, O3 (approval routing) end to end: the real server with organizations on, a fake WorkOS, real phone
//! cores, the real org executor, and fakes of PagerDuty, Slack and GitHub (never the real APIs).
//!
//! One pilot org goes through every O3 gate in order:
//! 1. a production write routes to the on-call member and an engineer; the on-call member is told in Slack, opens the
//!    link and approves on the web with a device-bound passkey (confirmed on their phone); the engineer approves in
//!    the app; the executor verifies both and runs exactly the approved digest;
//! 2. an L3 request never offers the web: its Slack message says "Open the Reins app";
//! 3. forged Slack events change nothing;
//! 4. an unanswered obligation escalates to Security admins, then expires: not approved in time.

use std::sync::Arc;
use std::time::Duration;

use reins_e2e::org::{
    Browser, WEBHOOK_SECRET, add_work_account, between, csrf, db, device_sign, dir_user, enrollment_with_key,
    member_state, wait_for, webhook,
};
use reins_e2e::workos::{FakeWorkos, User};
use reins_e2e::{AiClient, Phone, Server};
use reins_executor::connectors::github::GithubApp;
use reins_executor::{Client, Context, run_once};
use ring::rand::SystemRandom;
use ring::signature::{ECDSA_P256_SHA256_ASN1_SIGNING, EcdsaKeyPair, KeyPair};
use serde_json::{Value, json};
use wiremock::matchers::{method, path, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

struct Member {
    phone: Phone,
    key: Vec<u8>,
    org_id: String,
}

impl Member {
    async fn answer(&self, approve: bool, pick: impl Fn(&reins_core::org::OrgReviewView) -> bool) -> usize {
        let reviews = self.phone.core.org_reviews().await.unwrap();
        let mut n = 0;
        for r in reviews.into_iter().filter(|r| pick(r)) {
            let at = reins_executor::now();
            let msg = reins_core::profiles::org_review_message(self.org_id.clone(), r.clone(), approve, at);
            self.phone.core.org_decide(r.id.clone(), approve, at, device_sign(&self.key, &msg)).await.unwrap();
            n += 1;
        }
        n
    }
}

async fn enroll(phone: Phone) -> Member {
    let challenge = phone.core.org_challenge().await.unwrap();
    let (input, key) = enrollment_with_key(&challenge);
    phone.core.org_enroll(challenge, input).await.unwrap();
    let org_id = phone.core.org_membership().await.unwrap().expect("a work account").org_id;
    Member {
        phone,
        key,
        org_id,
    }
}

// ---- A soft WebAuthn authenticator (P-256, device-bound: BE clear).

fn cbor_head(major: u8, len: usize) -> Vec<u8> {
    let m = major * 0x20;
    match len {
        0..=23 => vec![m + u8::try_from(len).unwrap()],
        24..=255 => vec![m + 0x18, u8::try_from(len).unwrap()],
        _ => {
            let mut v = vec![m + 0x19];
            v.extend_from_slice(&u16::try_from(len).unwrap().to_be_bytes());
            v
        }
    }
}

fn cbor_int(n: i64) -> Vec<u8> {
    if n >= 0 {
        cbor_head(0, usize::try_from(n).unwrap())
    } else {
        cbor_head(1, usize::try_from(-1 - n).unwrap())
    }
}

fn cbor_bytes(b: &[u8]) -> Vec<u8> {
    let mut v = cbor_head(2, b.len());
    v.extend_from_slice(b);
    v
}

fn cbor_text(s: &str) -> Vec<u8> {
    let mut v = cbor_head(3, s.len());
    v.extend_from_slice(s.as_bytes());
    v
}

struct Authenticator {
    id: Vec<u8>,
    pkcs8: Vec<u8>,
}

impl Authenticator {
    fn new(id: &[u8]) -> Self {
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &SystemRandom::new()).unwrap();
        Self {
            id: id.to_vec(),
            pkcs8: pkcs8.as_ref().to_vec(),
        }
    }

    fn key(&self) -> EcdsaKeyPair {
        EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &self.pkcs8, &SystemRandom::new()).unwrap()
    }

    fn id_b64(&self) -> String {
        data_encoding::BASE64URL_NOPAD.encode(&self.id)
    }

    /// `create()`: (client data, attestation object), base64url.
    fn register(&self, challenge: &str, origin: &str, rp: &str) -> (String, String) {
        let point = self.key().public_key().as_ref().to_vec();
        let mut cose = cbor_head(5, 5);
        for (k, v) in [
            (1, cbor_int(2)),
            (3, cbor_int(-7)),
            (-1, cbor_int(1)),
            (-2, cbor_bytes(&point[1..33])),
            (-3, cbor_bytes(&point[33..])),
        ] {
            cose.extend(cbor_int(k));
            cose.extend(v);
        }
        let mut auth = reins_proto::canon::sha256(rp.as_bytes()).to_vec();
        auth.push(0x45); // UP | UV | AT, not backup eligible
        auth.extend_from_slice(&[0, 0, 0, 0]);
        auth.extend_from_slice(&[0; 16]);
        auth.extend_from_slice(&u16::try_from(self.id.len()).unwrap().to_be_bytes());
        auth.extend_from_slice(&self.id);
        auth.extend(cose);
        let mut obj = cbor_head(5, 3);
        obj.extend(cbor_text("fmt"));
        obj.extend(cbor_text("none"));
        obj.extend(cbor_text("attStmt"));
        obj.extend(cbor_head(5, 0));
        obj.extend(cbor_text("authData"));
        obj.extend(cbor_bytes(&auth));
        let client = json!({"type": "webauthn.create", "challenge": challenge, "origin": origin}).to_string();
        let b64 = data_encoding::BASE64URL_NOPAD;
        (b64.encode(client.as_bytes()), b64.encode(&obj))
    }

    /// `get()`: (authenticator data, client data, signature), base64url.
    fn assert(&self, challenge: &str, origin: &str, rp: &str) -> (String, String, String) {
        let mut auth = reins_proto::canon::sha256(rp.as_bytes()).to_vec();
        auth.push(0x05);
        auth.extend_from_slice(&[0, 0, 0, 1]);
        let client = json!({"type": "webauthn.get", "challenge": challenge, "origin": origin}).to_string();
        let mut signed = auth.clone();
        signed.extend_from_slice(&reins_proto::canon::sha256(client.as_bytes()));
        let sig = self.key().sign(&SystemRandom::new(), &signed).unwrap();
        let b64 = data_encoding::BASE64URL_NOPAD;
        (b64.encode(&auth), b64.encode(client.as_bytes()), b64.encode(sig.as_ref()))
    }
}

async fn fake_github() -> MockServer {
    let gh = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/app/installations/77/access_tokens"))
        .respond_with(
            ResponseTemplate::new(201).set_body_json(json!({"token": "ghs_1", "expires_at": "2099-01-01T00:00:00Z"})),
        )
        .mount(&gh)
        .await;
    Mock::given(method("GET"))
        .and(path_regex(r"^/repos/acme/[a-z-]+$"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": 1, "full_name": "acme/prod-db"})))
        .mount(&gh)
        .await;
    Mock::given(method("POST"))
        .and(path("/repos/acme/prod-db/issues"))
        .respond_with(
            ResponseTemplate::new(201)
                .set_body_json(json!({"number": 7, "html_url": "https://github.com/acme/prod-db/issues/7"})),
        )
        .mount(&gh)
        .await;
    gh
}

async fn fake_slack() -> MockServer {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/users.lookupByEmail"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true, "user": {"id": "U1"}})))
        .mount(&s)
        .await;
    Mock::given(method("POST"))
        .and(path("/conversations.open"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true, "channel": {"id": "D1"}})))
        .mount(&s)
        .await;
    Mock::given(method("POST"))
        .and(path_regex(r"^/chat\.(postMessage|update)$"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true, "ts": "1.1"})))
        .mount(&s)
        .await;
    s
}

async fn slack_posts(s: &MockServer) -> Vec<Value> {
    s.received_requests()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|r| r.url.path() == "/chat.postMessage")
        .map(|r| serde_json::from_slice(&r.body).unwrap())
        .collect()
}

fn request_state(server: &Server, id: &str) -> String {
    db(server).query_row("SELECT state FROM reins_org_requests WHERE uuid = ?1", [id], |r| r.get(0)).unwrap()
}

fn group_id(server: &Server, name: &str) -> String {
    db(server).query_row("SELECT uuid FROM groups WHERE name = ?1", [name], |r| r.get(0)).unwrap()
}

fn audit_kinds(server: &Server, org: &str) -> Vec<String> {
    let c = db(server);
    let mut st = c.prepare("SELECT kind FROM reins_org_events WHERE org_uuid = ?1 ORDER BY seq").unwrap();
    st.query_map([org], |r| r.get::<_, String>(0)).unwrap().map(Result::unwrap).collect()
}

fn membership(server: &Server, email: &str) -> String {
    db(server)
        .query_row(
            "SELECT m.uuid FROM users_organizations m JOIN users u ON u.uuid = m.user_uuid WHERE u.email = ?1",
            [email],
            |r| r.get(0),
        )
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[allow(clippy::too_many_lines, reason = "one pilot org through every O3 gate, in order")]
async fn org_routing_end_to_end() {
    reins_e2e::init_tls();
    let workos = FakeWorkos::start().await;
    workos.add_organization("org_ACME", &["acme.com"]);
    workos.add_directory("directory_ACME", "org_ACME");
    let (gh, slack, pd) = (fake_github().await, fake_slack().await, MockServer::start().await);
    Mock::given(method("GET"))
        .and(path("/oncalls"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"oncalls": [{"user": {"email": "ola@acme.com"}}]})),
        )
        .mount(&pd)
        .await;
    let scratch = tempfile::tempdir().unwrap();
    let txt = scratch.path().join("txt");
    std::fs::write(&txt, "").unwrap();
    let mut env = workos.server_env();
    env.extend(
        [
            ("REINS_ORGS_ENABLED", "true".to_owned()),
            ("REINS_WORKOS_DSYNC_SECS", "3600".to_owned()),
            ("REINS_WORKOS_WEBHOOK_SECRET", WEBHOOK_SECRET.to_owned()),
            ("REINS_TEST_DNS_TXT_FILE", txt.to_string_lossy().into_owned()),
            ("REINS_TEST_ORG_SWEEP_SECS", "1".to_owned()),
            ("REINS_TEST_SLACK_API", slack.uri()),
            ("REINS_TEST_PAGERDUTY_API", pd.uri()),
        ]
        .map(|(k, v)| (k.to_owned(), v)),
    );
    let server = Server::start_with_env(5, 3, &env).await;
    let origin = reins_executor::config::web_origin(&server.base).unwrap();
    let rp = url::Url::parse(&server.base).unwrap().host_str().unwrap().to_owned();

    // ---- The pilot org: Dana creates Acme with SSO and the directory; Kim, Lee and Ola arrive by SCIM.
    let dana_user = User {
        id: "user_dana".to_owned(),
        email: "dana@acme.com".to_owned(),
    };
    workos.sign_in_as(&dana_user);
    let dana_phone = Phone::signed_out(|_| {}).await;
    dana_phone.sso_sign_in(&server.base).await.unwrap();
    dana_phone.core.register_device(None).await.unwrap();
    let mut browser = Browser::new();
    let home = browser.get(&server.url("/reins/org/login")).await;
    let created = browser
        .post(&server.url("/reins/org/new"), &[("csrf", &csrf(&home)), ("name", "Acme"), ("tier", "business")])
        .await;
    let org = created.url.rsplit('/').next().unwrap().to_owned();
    let record = between(&created.body, "_reins-challenge.acme.com<br>", "</div>").to_owned();
    std::fs::write(&txt, format!("_reins-challenge.acme.com={record}\n")).unwrap();
    let page = browser
        .post(
            &server.url(&format!("/reins/org/{org}/domains/verify")),
            &[("csrf", &csrf(&created)), ("domain", "acme.com")],
        )
        .await;
    browser
        .post(
            &server.url(&format!("/reins/org/{org}/identity")),
            &[
                ("csrf", &csrf(&page)),
                ("identity", "sso"),
                ("workos_org_id", "org_ACME"),
                ("directory_id", "directory_ACME"),
                ("github_org", ""),
            ],
        )
        .await;
    let people = [("kim", "kim@acme.com"), ("lee", "lee@acme.com"), ("ola", "ola@acme.com")];
    for (id, email) in people {
        workos.emit("dsync.user.created", dir_user(&format!("directory_user_{id}"), email, "active"));
    }
    let group = json!({"object": "directory_group", "id": "directory_group_eng", "name": "Engineering",
                       "directory_id": "directory_ACME", "organization_id": "org_ACME"});
    workos.emit("dsync.group.created", group.clone());
    workos.emit(
        "dsync.group.user_added",
        json!({"directory_id": "directory_ACME", "user": dir_user("directory_user_kim", "kim@acme.com", "active"), "group": group}),
    );
    webhook(&server, &json!({"id": "wh_1", "event": "dsync.user.created"})).await;
    wait_for("provisioning", Duration::from_secs(30), || {
        people.iter().all(|(_, e)| member_state(&server, e).as_deref() == Some("active"))
    })
    .await;
    let mut members = Vec::new();
    for (id, email) in people {
        workos.sign_in_as(&User {
            id: format!("user_{id}"),
            email: email.to_owned(),
        });
        let phone = Phone::signed_out(|_| {}).await;
        add_work_account(&phone, &server, "acme.com").await.unwrap();
        phone.core.register_device(None).await.unwrap();
        members.push(enroll(phone).await);
    }
    let ola = members.pop().unwrap();
    let lee = members.pop().unwrap();
    let kim = members.pop().unwrap();
    let dana = enroll(dana_phone).await;
    let mut page = browser.get(&server.url(&format!("/reins/org/{org}/members"))).await;
    for email in ["dana@acme.com", "kim@acme.com"] {
        let subject = format!("member:{}", membership(&server, email));
        page = browser
            .post(
                &server.url(&format!("/reins/org/{org}/roles")),
                &[("csrf", &csrf(&page)), ("subject", &subject), ("role", "security_admin"), ("op", "grant")],
            )
            .await;
    }

    // ---- The policy: Production change control (on-call and an engineer, Deferred 4 h), and a slow review that
    // escalates (Engineering for 3 s, then Security admins for 3 s).
    let eng = group_id(&server, "Engineering");
    let mut draft: Value =
        serde_json::from_str(include_str!("../../../src/api/reins/org/policy_baseline.json")).unwrap();
    draft["facts"] = json!({"owners": {"acme": "work"}});
    let review = |reviewers: Value, extra: Value| {
        let mut r = json!({"reviewers": reviewers, "class": "deferred", "window_secs": 14400, "urgent": true});
        for (k, v) in extra.as_object().unwrap() {
            r[k] = v.clone();
        }
        r
    };
    draft["rules"] = json!([
        {"id": "tpl.production_change_control.1", "issuer": "owner", "then": "review",
         "when": {"resource": "acme/prod-*"}, "explain": "Production: on-call approves.",
         "review": review(json!({"on_call": {"alias": "payments"}}), json!({}))},
        {"id": "tpl.production_change_control.2", "issuer": "owner", "then": "review",
         "when": {"resource": "acme/prod-*"}, "explain": "Production: a second engineer approves.",
         "review": review(json!({"group": eng}), json!({}))},
        {"id": "slow", "issuer": "owner", "then": "review", "when": {"resource": "acme/slow-*"}, "explain": "Slow",
         "review": {"reviewers": {"group": eng}, "class": "deferred", "window_secs": 3,
                    "escalate": [{"role": "security_admin"}]}}
    ]);
    let page = browser.get(&server.url(&format!("/reins/org/{org}/policy"))).await;
    let page = browser
        .post(
            &server.url(&format!("/reins/org/{org}/policy/draft")),
            &[("csrf", &csrf(&page)), ("body", &draft.to_string())],
        )
        .await;
    assert!(page.body.contains("Saved as revision 1"), "{}", page.body);
    let hash: String = db(&server)
        .query_row("SELECT hash FROM reins_org_policies WHERE org_uuid = ?1 AND revision = 1", [&org], |r| r.get(0))
        .unwrap();
    browser
        .post(
            &server.url(&format!("/reins/org/{org}/policy/activate")),
            &[("csrf", &csrf(&page)), ("revision", "1"), ("hash", &hash)],
        )
        .await;
    assert_eq!(dana.answer(true, |r| r.own).await, 1);
    assert_eq!(kim.answer(true, |r| r.title.contains("policy")).await, 1);

    // ---- Routing: on-call `payments` from PagerDuty, and Slack turned on: each a two-person change.
    let page = browser.get(&server.url(&format!("/reins/org/{org}/routing"))).await;
    let page = browser
        .post(
            &server.url(&format!("/reins/org/{org}/routing/oncall")),
            &[("csrf", &csrf(&page)), ("alias", "payments"), ("provider", "pagerduty"), ("schedule_id", "PSCHED1")],
        )
        .await;
    assert_eq!(dana.answer(true, |r| r.own).await, 1);
    assert_eq!(kim.answer(true, |r| r.title.contains("On-call")).await, 1);
    let page = browser
        .post(
            &server.url(&format!("/reins/org/{org}/routing/oncall/key")),
            &[("csrf", &csrf(&page)), ("alias", "payments"), ("key", "pd-key")],
        )
        .await;
    assert!(page.body.contains("Key saved"), "{}", page.body);
    assert!(!page.body.contains("pd-key"), "a key is never shown again");
    let page = browser.get(&server.url(&format!("/reins/org/{org}/channels"))).await;
    let page = browser
        .post(
            &server.url(&format!("/reins/org/{org}/channels/install")),
            &[
                ("csrf", &csrf(&page)),
                ("kind", "slack"),
                ("bot_token", "xoxb-test"),
                ("signing_secret", "slack-secret"),
            ],
        )
        .await;
    browser
        .post(
            &server.url(&format!("/reins/org/{org}/channels/toggle")),
            &[("csrf", &csrf(&page)), ("kind", "slack"), ("enabled", "true")],
        )
        .await;
    assert_eq!(dana.answer(true, |r| r.own).await, 1);
    assert_eq!(kim.answer(true, |r| r.title.contains("Slack")).await, 1);

    // ---- Ola adds a device-bound passkey in the console and confirms it on her phone.
    workos.sign_in_as(&User {
        id: "user_ola".to_owned(),
        email: "ola@acme.com".to_owned(),
    });
    workos.sign_in_via_sso("org_ACME");
    let mut ola_browser = Browser::new();
    let keys_page = ola_browser.get(&server.url("/reins/org/login?next=/reins/org/passkeys/new")).await;
    assert!(keys_page.body.contains("Passkeys"), "{}", keys_page.body);
    let challenge = between(&keys_page.body, "data-ch=\"", "\"").to_owned();
    let key = Authenticator::new(b"ola-security-key-1");
    let (client, att) = key.register(&challenge, &origin, &rp);
    let added = ola_browser
        .post(
            &server.url("/reins/org/passkeys/new"),
            &[
                ("csrf", &csrf(&keys_page)),
                ("credential_id", &key.id_b64()),
                ("client_data_json", &client),
                ("attestation_object", &att),
            ],
        )
        .await;
    assert!(added.body.contains("Confirm it on your phone"), "{}", added.body);
    let pending = ola.phone.core.org_passkeys().await.unwrap();
    let pk = pending.iter().find(|p| p.credential_id == key.id_b64()).unwrap().clone();
    assert!(!pk.confirmed);
    let at = reins_executor::now();
    let msg = reins_core::profiles::org_passkey_confirm_message(
        ola.org_id.clone(),
        pk.credential_id.clone(),
        pk.spki_hash.clone(),
        pk.created_at,
        at,
    );
    ola.phone.core.org_confirm_passkey(pk.credential_id.clone(), true, at, device_sign(&ola.key, &msg)).await.unwrap();

    // ---- The executor, pinned with the server's web origin.
    let exec_dir = scratch.path().join("executor");
    std::fs::create_dir_all(&exec_dir).unwrap();
    let key_file = exec_dir.join("executor.key");
    let public = reins_executor::client::generate_key(&key_file).unwrap();
    let page = browser.get(&server.url(&format!("/reins/org/{org}/executors"))).await;
    browser
        .post(
            &server.url(&format!("/reins/org/{org}/executors")),
            &[("csrf", &csrf(&page)), ("executor_id", "e1"), ("mode", "cloud"), ("public_key", &public)],
        )
        .await;
    assert_eq!(dana.answer(true, |r| r.own).await, 1);
    assert_eq!(kim.answer(true, |r| r.title.contains("executor")).await, 1);
    let client =
        Client::new(&server.base, &org, "e1", Arc::new(reins_executor::client::load_key(&key_file).unwrap())).unwrap();
    let keys = client.keys().await.unwrap();
    let pin = |(id, b64): &(String, String)| reins_audit::TrustedKey::parse(&format!("{id}={b64}")).unwrap();
    let pins = reins_executor::Pins {
        org: org.clone(),
        issuer: pin(&keys.issuer),
        decision: pin(&keys.decision),
        web_origin: Some(origin.clone()),
    };
    let pem = reins_executor::connectors::testing::test_pem();
    let ctx = Arc::new(
        Context::new(client, pins, &exec_dir.join("state"))
            .unwrap()
            .with_github(GithubApp::new(1, 77, "acme", &pem, &gh.uri()).unwrap()),
    );

    // Lee's agent.
    let mut ai = AiClient::new(&server.base);
    ai.register_client().await;
    let wait_url = ai.start_authorization("lee@acme.com").await;
    let code = ai.browser_code(&wait_url).await;
    let item = lee.phone.wait_for_item(Duration::from_secs(20)).await;
    lee.phone.core.answer_pairing(item.id, true, Some(code), Some("Lee's agent".to_owned()), true).await.unwrap();
    ai.finish_authorization(&wait_url).await;

    // ---- 1. A production write: on-call (Ola, told in Slack) and an engineer (Kim, in the app).
    let write = json!({"repo": "acme/prod-db", "title": "Roll out", "body": "eA=="});
    let r = ai.tool("github_create_issue", &write).await;
    assert_eq!(r["structuredContent"]["status"], "pending_review", "{r}");
    let ticket = r["structuredContent"]["ticket"].as_str().unwrap().to_owned();
    assert!(lee.phone.core.org_reviews().await.unwrap().iter().all(|v| v.id != ticket), "no self-approval");
    // Ola (on call) and Kim (Engineering) are each told in Slack, with their own link.
    wait_for("the Slack messages", Duration::from_secs(20), || futures_lite_block(slack_posts(&slack)).len() >= 2)
        .await;
    // The posts go out one reviewer after another, in no set order: read them until Ola's own link is there.
    let mut mine = None;
    let mut seen = Vec::new();
    let mut opened = std::collections::BTreeSet::new();
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    while mine.is_none() && std::time::Instant::now() < deadline {
        let posted = slack_posts(&slack).await;
        let urls: Vec<String> = posted
            .iter()
            .map(|p| {
                let line = p.to_string();
                assert!(!line.contains("Roll out") && !line.contains("eA=="), "no body in Slack: {line}");
                between(&line, "\"url\":\"", "\"").to_owned()
            })
            .collect();
        assert!(urls.iter().all(|u| u.contains("/reins/org/approve/")), "{urls:?}");
        for u in urls.into_iter().filter(|u| opened.insert(u.clone())) {
            let page = ola_browser.get(&u).await;
            seen.push((page.status, between(&page.body, "<title>", "</title>").to_owned()));
            if page.status == 200 {
                mine = Some((u, page));
                break;
            }
            // A link that is someone else's: refused, whoever forwarded it.
            assert_eq!(page.status, 403, "{seen:?}");
        }
        if mine.is_none() {
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }
    let Some((url, card)) = mine else {
        let pd_calls = pd.received_requests().await.unwrap_or_default().len();
        let log: Vec<String> =
            server.log().lines().filter(|l| l.contains("on-call") || l.contains("WARN")).map(str::to_owned).collect();
        panic!("Ola's own link: {seen:?}; PagerDuty calls {pd_calls}; {log:?}");
    };

    let ch = between(&card.body, "data-approve=\"", "\"").to_owned();
    let at = between(&card.body, "name=\"at\" value=\"", "\"").to_owned();
    let (auth, client_data, sig) = key.assert(&ch, &origin, &rp);
    let approve_url = url.replace(&server.base, "");
    let done = ola_browser
        .post(
            &server.url(&approve_url),
            &[
                ("csrf", &csrf(&card)),
                ("decision", "approve"),
                ("at", &at),
                ("credential_id", &key.id_b64()),
                ("authenticator_data", &auth),
                ("client_data_json", &client_data),
                ("signature", &sig),
            ],
        )
        .await;
    assert!(done.body.contains("Approved"), "{}", done.body);
    // The link worked once.
    let again = ola_browser.get(&url).await;
    assert_ne!(again.status, 200);
    assert_eq!(request_state(&server, &ticket), "reviewing", "the engineer still has to approve");
    assert_eq!(kim.answer(true, |v| v.id == ticket).await, 1);
    assert_eq!(request_state(&server, &ticket), "approved");
    let stats = run_once(&ctx).await.unwrap();
    assert_eq!(stats.succeeded, 1, "{stats:?}");
    let created: Vec<Value> = gh
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.method.as_str() == "POST" && r.url.path().ends_with("/issues"))
        .map(|r| serde_json::from_slice(&r.body).unwrap())
        .collect();
    assert_eq!(created, [json!({"title": "Roll out", "body": "eA=="})], "exactly the approved digest");
    let kinds = audit_kinds(&server, &org);
    for k in ["review.resolved", "review.web", "passkey.confirmed", "routing.changed"] {
        assert!(kinds.iter().any(|x| x == k), "{k} missing: {kinds:?}");
    }

    // ---- 2. L3 never offers the web: a push whose branch can't be read is L3; its Slack messages have no link.
    let before = slack_posts(&slack).await.len();
    let push = json!({"method": "PUT", "path": "/repos/acme/prod-db/contents/deploy.yaml", "body": {"message": "x"}});
    let r = ai.tool("github_api_write", &push).await;
    let l3 = r["structuredContent"]["ticket"].as_str().unwrap_or_default().to_owned();
    assert!(
        !l3.is_empty(),
        "{r}\n{}",
        server.log().lines().filter(|l| l.contains("WARN") || l.contains("ERROR")).collect::<Vec<_>>().join("\n")
    );
    let l3_posts = || -> Vec<String> {
        futures_lite_block(slack_posts(&slack))[before..]
            .iter()
            .map(ToString::to_string)
            .filter(|t| t.contains("· L3"))
            .collect()
    };
    wait_for("the L3 messages", Duration::from_secs(20), || !l3_posts().is_empty()).await;
    for line in l3_posts() {
        assert!(line.contains("Open the Reins app") && !line.contains("/reins/org/approve/"), "{line}");
    }

    // ---- 3. Forged Slack events change nothing.
    let http = reqwest::Client::new();
    let now = reins_executor::now().to_string();
    for (ts, sig) in [("1", "v0=00"), (now.as_str(), "v0=deadbeef")] {
        let resp = http
            .post(server.url("/reins/org/slack/events"))
            .header("X-Slack-Request-Timestamp", ts)
            .header("X-Slack-Signature", sig)
            .body(r#"{"type":"block_actions","actions":[{"action_id":"approve"}]}"#)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status().as_u16(), 401);
    }
    assert_eq!(request_state(&server, &l3), "reviewing");

    // ---- 4. Unanswered: Engineering for 3 s, then Security admins, then expired.
    let r = ai.tool("github_create_issue", &json!({"repo": "acme/slow-svc", "title": "Slow"})).await;
    let slow = r["structuredContent"]["ticket"].as_str().unwrap().to_owned();
    wait_for("the escalation", Duration::from_secs(30), || {
        audit_kinds(&server, &org).iter().any(|k| k == "review.escalated")
    })
    .await;
    wait_for("the expiry", Duration::from_secs(30), || request_state(&server, &slow) == "expired").await;
    let r = ai.tool("reins_get_result", &json!({"request_id": slow})).await;
    assert!(r.to_string().contains("Expired: not approved in time."), "{r}");
}

/// Runs a future to completion from a synchronous `wait_for` closure.
fn futures_lite_block<F: Future>(f: F) -> F::Output {
    tokio::task::block_in_place(|| tokio::runtime::Handle::current().block_on(f))
}
