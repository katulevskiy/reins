//! Organizations (Decisions v2 · O2) end to end: the real server with organizations on, a fake WorkOS, real phone cores
//! for three members, an AI client paired to a member's work account, the real org executor (in this process, talking
//! to the server over HTTP like a separate deployment would) and a fake GitHub.
//!
//! One pilot org goes through every O2 gate in order:
//! 1. an org request decided by policy (Auto, executed; an org deny);
//! 2. an org review routed to Security and device-signed on a reviewer's phone; a rooted phone can't approve;
//! 3. the executor running exactly the approved digest (a request changed after approval is refused);
//! 4. deprovisioning revoking a credential lease upstream within the minute;
//! 5. the audit export verifying, and detecting an edited event and a cut tail.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use reins_e2e::org::{
    Browser, Page, WEBHOOK_SECRET, add_work_account, between, csrf, db, device_sign, dir_user, enrollment_with_key,
    member_state, posture, wait_for, webhook,
};
use reins_e2e::workos::{FakeWorkos, User};
use reins_e2e::{AiClient, Phone, Server};
use reins_executor::connectors::github::GithubApp;
use reins_executor::{Client, Context, Opener, run_once};
use reins_proto::managed::{Checkpoint, ExportLine};
use serde_json::{Value, json};
use wiremock::matchers::{method, path, path_regex};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const ORG_KEY_CONTEXT: &str = "reins/org-kms-key/v1";

/// A member's phone with its enrolled org device key.
struct Member {
    phone: Phone,
    key: Vec<u8>,
    org_id: String,
}

impl Member {
    /// Answers every review waiting on this phone that `pick` selects; returns how many.
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

/// Installation tokens `ghs_1`, `ghs_2`, … in order.
struct Tokens(AtomicUsize);

impl Respond for Tokens {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        let n = self.0.fetch_add(1, Ordering::SeqCst) + 1;
        ResponseTemplate::new(201)
            .set_body_json(json!({"token": format!("ghs_{n}"), "expires_at": "2099-01-01T00:00:00Z"}))
    }
}

async fn fake_github() -> MockServer {
    let gh = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/app/installations/77/access_tokens"))
        .respond_with(Tokens(AtomicUsize::new(0)))
        .mount(&gh)
        .await;
    for (name, id) in [("api", 101), ("prod-db", 102), ("web", 103)] {
        Mock::given(method("GET"))
            .and(path(format!("/repos/acme/{name}")))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"id": id, "full_name": format!("acme/{name}")})),
            )
            .mount(&gh)
            .await;
    }
    Mock::given(method("POST"))
        .and(path_regex(r"^/repos/acme/[a-z-]+/issues$"))
        .respond_with(
            ResponseTemplate::new(201)
                .set_body_json(json!({"number": 42, "html_url": "https://github.com/acme/x/issues/42"})),
        )
        .mount(&gh)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/installation/token"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&gh)
        .await;
    gh
}

/// The requests GitHub received to create issues: their JSON bodies.
async fn issues_created(gh: &MockServer) -> Vec<Value> {
    gh.received_requests()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|r| r.method.as_str() == "POST" && r.url.path().ends_with("/issues"))
        .map(|r| serde_json::from_slice(&r.body).unwrap())
        .collect()
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

fn request_state(server: &Server, id: &str) -> String {
    db(server).query_row("SELECT state FROM reins_org_requests WHERE uuid = ?1", [id], |r| r.get(0)).unwrap()
}

fn audit_kinds(server: &Server, org: &str) -> Vec<String> {
    let c = db(server);
    let mut st = c.prepare("SELECT kind FROM reins_org_events WHERE org_uuid = ?1 ORDER BY seq").unwrap();
    st.query_map([org], |r| r.get::<_, String>(0)).unwrap().map(Result::unwrap).collect()
}

/// A compromised server with the database and the KMS master key: replaces an approved request's sealed body.
fn tamper_body(server: &Server, org: &str, request: &str, tool: &str, args: &Value) {
    use ring::aead::{AES_256_GCM, Aad, LessSafeKey, NONCE_LEN, Nonce, UnboundKey};
    let b64 = data_encoding::BASE64URL_NOPAD;
    let master = std::fs::read(server.database_path().parent().unwrap().join("reins_org_kms.key")).unwrap();
    let c = db(server);
    let (key_id, sealed_key): (String, String) = c
        .query_row("SELECT key_id, sealed FROM reins_org_keys WHERE org_uuid = ?1 AND purpose = 'data'", [org], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    let open = |key: &[u8], aad: &[u8], sealed: &str| {
        let raw = b64.decode(sealed.as_bytes()).unwrap();
        let (nonce, ct) = raw.split_at(NONCE_LEN);
        let k = LessSafeKey::new(UnboundKey::new(&AES_256_GCM, key).unwrap());
        let mut buf = ct.to_vec();
        let n =
            k.open_in_place(Nonce::try_assume_unique_for_key(nonce).unwrap(), Aad::from(aad), &mut buf).unwrap().len();
        buf.truncate(n);
        buf
    };
    let mut aad = reins_proto::canon::Canon::new(ORG_KEY_CONTEXT);
    aad.str(org).str("data").str(&key_id);
    let data_key = open(&master, &aad.finish(), &sealed_key);
    let body = reins_proto::managed::canonical_json(&json!({"tool": tool, "args": args}));
    let k = LessSafeKey::new(UnboundKey::new(&AES_256_GCM, &data_key).unwrap());
    let nonce = [7u8; NONCE_LEN];
    let mut buf = body.into_bytes();
    k.seal_in_place_append_tag(
        Nonce::assume_unique_for_key(nonce),
        Aad::from(format!("{request}/request").as_bytes()),
        &mut buf,
    )
    .unwrap();
    let mut out = nonce.to_vec();
    out.extend_from_slice(&buf);
    let n = c
        .execute(
            "UPDATE reins_org_bodies SET sealed = ?1 WHERE request_uuid = ?2 AND part = 'request'",
            [b64.encode(&out), request.to_owned()],
        )
        .unwrap();
    assert_eq!(n, 1);
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

fn verify_export(
    text: &str,
    key: &reins_audit::TrustedKey,
    anchor: Option<&Checkpoint>,
) -> Result<reins_audit::Report, reins_audit::VerifyError> {
    reins_audit::verify(text.as_bytes(), std::slice::from_ref(key), anchor)
}

fn head_of(text: &str) -> Checkpoint {
    let last = text.lines().filter(|l| !l.trim().is_empty()).last().unwrap();
    let ExportLine::Head(env) = serde_json::from_str(last).unwrap() else {
        panic!("the export ends with its head")
    };
    env.parse().unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[allow(clippy::too_many_lines, reason = "one pilot org through every O2 gate, in order")]
async fn an_org_decides_reviews_executes_audits_and_revokes() {
    reins_e2e::init_tls();
    let workos = FakeWorkos::start().await;
    workos.add_organization("org_ACME", &["acme.com"]);
    workos.add_directory("directory_ACME", "org_ACME");
    let gh = fake_github().await;
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
        ]
        .map(|(k, v)| (k.to_owned(), v)),
    );
    let server = Server::start_with_env(5, 3, &env).await;

    // ---- The pilot org (O1): Dana creates Acme, verifies acme.com, binds SSO and the directory.
    let dana = User {
        id: "user_dana".to_owned(),
        email: "dana@acme.com".to_owned(),
    };
    workos.sign_in_as(&dana);
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
    let page = browser
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
    assert!(page.body.contains("Saved"), "{}", page.body);

    // Kim and Lee arrive through SCIM and add their work accounts on their phones.
    for (id, email) in [("directory_user_kim", "kim@acme.com"), ("directory_user_lee", "lee@acme.com")] {
        workos.emit("dsync.user.created", dir_user(id, email, "active"));
    }
    webhook(&server, &json!({"id": "wh_1", "event": "dsync.user.created"})).await;
    wait_for("provisioning", Duration::from_secs(30), || {
        member_state(&server, "kim@acme.com").as_deref() == Some("active")
            && member_state(&server, "lee@acme.com").as_deref() == Some("active")
    })
    .await;
    let mut phones = Vec::new();
    for (id, email) in [("user_kim", "kim@acme.com"), ("user_lee", "lee@acme.com")] {
        workos.sign_in_as(&User {
            id: id.to_owned(),
            email: email.to_owned(),
        });
        let phone = Phone::signed_out(|_| {}).await;
        add_work_account(&phone, &server, "acme.com").await.unwrap();
        phone.core.register_device(None).await.unwrap();
        phones.push(phone);
    }
    let lee_phone = phones.pop().unwrap();
    let kim_phone = phones.pop().unwrap();
    let dana = enroll(dana_phone).await;
    let kim = enroll(kim_phone).await;
    let lee = enroll(lee_phone).await;
    assert_eq!(dana.org_id, org);

    // Dana (the owner) makes Dana and Kim Security admins: she is the only one who can, so it applies at once.
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

    // ---- The org policy: drafted in the console, confirmed on Dana's phone, approved on Kim's, signed by the issuer.
    let mut draft: Value =
        serde_json::from_str(include_str!("../../../src/api/reins/org/policy_baseline.json")).unwrap();
    draft["facts"] = json!({"owners": {"acme": "work"}});
    draft["rules"] = json!([
        {"id": "prod-review", "issuer": "owner", "when": {"resource": "acme/prod-*"}, "then": "review",
         "explain": "Production needs Security", "review": {"reviewers": {"role": "security_admin"}, "class": "deferred"}},
        {"id": "prod-deny", "issuer": "owner", "when": {"resource": "acme/forbidden"}, "then": "deny",
         "explain": "Not through agents"}
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
    let page = browser
        .post(
            &server.url(&format!("/reins/org/{org}/policy/activate")),
            &[("csrf", &csrf(&page)), ("revision", "1"), ("hash", &hash)],
        )
        .await;
    assert!(page.body.contains("Confirm it on your phone"), "{}", page.body);
    // Kim can't approve before Dana confirmed it on her phone.
    assert_eq!(kim.answer(true, |r| r.title.contains("policy")).await, 0);
    assert_eq!(dana.answer(true, |r| r.own).await, 1);
    assert_eq!(kim.answer(true, |r| r.title.contains("policy")).await, 1);
    let active: String = db(&server)
        .query_row("SELECT state FROM reins_org_policies WHERE org_uuid = ?1 AND revision = 1", [&org], |r| r.get(0))
        .unwrap();
    assert_eq!(active, "active");

    // ---- The executor (Reins Cloud mode): its key registered by a device-signed two-person change, its pins compared
    // with the console's.
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
    let console_pins = browser.get(&server.url(&format!("/reins/org/{org}/executors"))).await;
    assert!(console_pins.body.contains(&keys.decision.1) && console_pins.body.contains(&keys.issuer.1));
    let pin = |(id, b64): &(String, String)| reins_audit::TrustedKey::parse(&format!("{id}={b64}")).unwrap();
    let pins = reins_executor::Pins {
        org: org.clone(),
        issuer: pin(&keys.issuer),
        decision: pin(&keys.decision),
    };
    let decision_key = pins.decision.clone();
    let pem = reins_executor::connectors::testing::test_pem();
    let ctx = Arc::new(
        Context::new(client, pins, &exec_dir.join("state"))
            .unwrap()
            .with_github(GithubApp::new(1, 77, "acme", &pem, &gh.uri()).unwrap()),
    );
    // It runs on its own, every 300 ms, unless paused.
    let paused = Arc::new(AtomicBool::new(false));
    let runner = {
        let (ctx, paused) = (ctx.clone(), paused.clone());
        tokio::spawn(async move {
            loop {
                if !paused.load(Ordering::SeqCst) {
                    drop(run_once(&ctx).await);
                }
                tokio::time::sleep(Duration::from_millis(300)).await;
            }
        })
    };

    // Lee's agent pairs with Lee's work account.
    let mut ai = AiClient::new(&server.base);
    ai.register_client().await;
    let wait_url = ai.start_authorization("lee@acme.com").await;
    let code = ai.browser_code(&wait_url).await;
    let item = lee.phone.wait_for_item(Duration::from_secs(20)).await;
    lee.phone.core.answer_pairing(item.id, true, Some(code), Some("Lee's agent".to_owned()), true).await.unwrap();
    ai.finish_authorization(&wait_url).await;

    // ---- 1. Decided by policy: routine work in an org repository is Auto and runs; an org deny says why.
    let r =
        ai.tool("github_create_issue", &json!({"repo": "acme/api", "title": "Flaky test", "body": "seen twice"})).await;
    assert_eq!(r["isError"], json!(false), "{r}");
    assert_eq!(r["structuredContent"]["number"], 42, "{r}");
    assert_eq!(issues_created(&gh).await, [json!({"title": "Flaky test", "body": "seen twice"})]);
    let r = ai.tool("github_create_issue", &json!({"repo": "acme/forbidden", "title": "x"})).await;
    assert_eq!(r["isError"], json!(true));
    assert!(r.to_string().contains("Not through agents"), "{r}");
    let kinds = audit_kinds(&server, &org);
    for k in [
        "policy.activated",
        "executor.registered",
        "request.received",
        "decision.auto",
        "request.executed",
        "decision.denied",
    ] {
        assert!(kinds.iter().any(|x| x == k), "{k} missing: {kinds:?}");
    }
    let snapshot: String = db(&server)
        .query_row(
            "SELECT snapshot FROM reins_org_requests WHERE org_uuid = ?1 AND tool = 'github_create_issue' LIMIT 1",
            [&org],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(snapshot, hash, "every decision names the active snapshot");

    // ---- 2. An org review is routed to Security and signed on a reviewer's phone; the requester can't approve it.
    let r = ai.tool("github_create_issue", &json!({"repo": "acme/prod-db", "title": "Rotate keys"})).await;
    assert_eq!(r["structuredContent"]["status"], "pending_review", "{r}");
    let ticket = r["structuredContent"]["ticket"].as_str().unwrap().to_owned();
    assert!(lee.phone.core.org_reviews().await.unwrap().is_empty(), "no self-approval");
    let mine = kim.phone.core.org_reviews().await.unwrap();
    assert!(mine.iter().any(|v| v.id == ticket && v.lines.iter().any(|l| l.contains("acme/prod-db"))), "{mine:?}");
    assert_eq!(kim.answer(true, |v| v.id == ticket).await, 1);
    let deadline = Instant::now() + Duration::from_secs(20);
    let done = loop {
        let r = ai.tool("reins_get_result", &json!({"request_id": ticket})).await;
        if r["structuredContent"]["number"] == 42 || Instant::now() > deadline {
            break r;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    };
    assert_eq!(done["structuredContent"]["number"], 42, "{done}");
    assert!(audit_kinds(&server, &org).iter().any(|k| k == "review.approved"));
    // A rooted phone can't approve: its review goes to Dana, who can.
    kim.phone.core.org_report_posture(posture(true)).await.unwrap();
    let r = ai.tool("github_create_issue", &json!({"repo": "acme/prod-db", "title": "Second"})).await;
    let second = r["structuredContent"]["ticket"].as_str().unwrap().to_owned();
    let reviews = kim.phone.core.org_reviews().await.unwrap();
    let pending = reviews.into_iter().find(|v| v.id == second).unwrap();
    let at = reins_executor::now();
    let msg = reins_core::profiles::org_review_message(org.clone(), pending.clone(), true, at);
    assert!(kim.phone.core.org_decide(second.clone(), true, at, device_sign(&kim.key, &msg)).await.is_err());
    assert_eq!(request_state(&server, &second), "reviewing");
    kim.phone.core.org_report_posture(posture(false)).await.unwrap();

    // ---- 3. The executor runs exactly the approved digest: a request changed after its approval is refused.
    paused.store(true, Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(400)).await;
    let r = ai.tool("github_create_issue", &json!({"repo": "acme/prod-db", "title": "Approved title"})).await;
    let third = r["structuredContent"]["ticket"].as_str().unwrap().to_owned();
    assert_eq!(dana.answer(true, |v| v.id == third).await, 1);
    assert_eq!(request_state(&server, &third), "approved");
    tamper_body(
        &server,
        &org,
        &third,
        "github_create_issue",
        &json!({"repo": "acme/prod-db", "title": "Drop the database"}),
    );
    let before = issues_created(&gh).await.len();
    let stats = run_once(&ctx).await.unwrap();
    assert!(stats.refused >= 1, "{stats:?}");
    assert_eq!(request_state(&server, &third), "refused");
    assert_eq!(issues_created(&gh).await.len(), before, "nothing reached GitHub");
    assert!(audit_kinds(&server, &org).iter().any(|k| k == "request.refused"));
    // The second request (still waiting) now gets Dana's approval and runs exactly as approved.
    assert_eq!(dana.answer(true, |v| v.id == second).await, 1);
    let stats = run_once(&ctx).await.unwrap();
    assert_eq!(stats.succeeded, 1, "{stats:?}");
    assert_eq!(issues_created(&gh).await.last().unwrap(), &json!({"title": "Second"}));
    paused.store(false, Ordering::SeqCst);

    // ---- 4. Deprovisioning revokes a credential lease upstream, within the minute.
    let (opener, box_key) = Opener::new().unwrap();
    let http = reqwest::Client::new();
    let lease: Value = http
        .post(server.url("/reins/api/org/leases"))
        .bearer_auth(&ai.access)
        .json(&json!({"repo": "acme/api", "access": "push", "box_key": box_key}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let lease_id = lease["id"].as_str().unwrap().to_owned();
    let deadline = Instant::now() + Duration::from_secs(20);
    let issued = loop {
        let v: Value = http
            .get(server.url(&format!("/reins/api/org/leases/{lease_id}")))
            .bearer_auth(&ai.access)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if v["state"] == "issued" || Instant::now() > deadline {
            break v;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    };
    assert_eq!(issued["state"], "issued", "{issued}");
    let token = String::from_utf8(opener.open(issued["sealed"].as_str().unwrap()).unwrap().to_vec()).unwrap();
    assert!(token.starts_with("ghs_"), "{token}");
    workos.emit("dsync.user.deleted", dir_user("directory_user_lee", "lee@acme.com", "inactive"));
    let started = Instant::now();
    webhook(&server, &json!({"id": "wh_2", "event": "dsync.user.deleted"})).await;
    let revoked_upstream = async || {
        gh.received_requests().await.unwrap_or_default().iter().any(|r| {
            r.method.as_str() == "DELETE"
                && r.url.path() == "/installation/token"
                && r.headers.get("authorization").and_then(|h| h.to_str().ok()).is_some_and(|h| h.ends_with(&token))
        })
    };
    while !revoked_upstream().await {
        assert!(started.elapsed() < Duration::from_secs(60), "the lease outlived deprovisioning");
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    let state: String =
        db(&server).query_row("SELECT state FROM reins_org_leases WHERE uuid = ?1", [&lease_id], |r| r.get(0)).unwrap();
    assert_eq!(state, "revoked");
    let again = http
        .post(server.url("/reins/api/org/leases"))
        .bearer_auth(&ai.access)
        .json(&json!({"repo": "acme/api", "access": "push", "box_key": box_key}))
        .send()
        .await
        .unwrap();
    assert_eq!(again.status().as_u16(), 401, "no new lease after removal");
    wait_for("upstream revocation recorded", Duration::from_secs(10), || {
        audit_kinds(&server, &org).iter().any(|k| k == "lease.revoked_upstream")
    })
    .await;
    assert!(audit_kinds(&server, &org).iter().any(|k| k == "lease.revoked"));
    runner.abort();

    // ---- 5. The audit export verifies; an edited event and a cut tail are caught.
    let export_url = server.url(&format!("/reins/org/{org}/audit/export"));
    let first = browser.get(&export_url).await;
    assert_eq!(first.status, 200);
    let report = verify_export(&first.body, &decision_key, None).unwrap();
    assert!(report.events > 20, "{report:?}");
    let anchor = head_of(&first.body);
    let c = db(&server);
    let original: String = c
        .query_row("SELECT detail FROM reins_org_events WHERE org_uuid = ?1 AND seq = 3", [&org], |r| r.get(0))
        .unwrap();
    c.execute("UPDATE reins_org_events SET detail = '{\"edited\":true}' WHERE org_uuid = ?1 AND seq = 3", [&org])
        .unwrap();
    let edited: Page = browser.get(&export_url).await;
    assert_eq!(
        verify_export(&edited.body, &decision_key, None),
        Err(reins_audit::VerifyError::BadHash {
            seq: 3
        })
    );
    c.execute("UPDATE reins_org_events SET detail = ?1 WHERE org_uuid = ?2 AND seq = 3", [original, org.clone()])
        .unwrap();
    // Roll the chain back below what the first export showed: a fresh head re-signs it, the kept head doesn't fit.
    c.execute("DELETE FROM reins_org_events WHERE org_uuid = ?1 AND seq >= ?2", rusqlite::params![org, anchor.seq])
        .unwrap();
    c.execute(
        "DELETE FROM reins_org_checkpoints WHERE org_uuid = ?1 AND seq >= ?2",
        rusqlite::params![org, anchor.seq],
    )
    .unwrap();
    let cut = browser.get(&export_url).await;
    assert!(verify_export(&cut.body, &decision_key, None).is_ok(), "alone, a re-signed shorter chain looks whole");
    assert_eq!(
        verify_export(&cut.body, &decision_key, Some(&anchor)),
        Err(reins_audit::VerifyError::DoesNotExtend {
            seq: anchor.seq
        })
    );
}
