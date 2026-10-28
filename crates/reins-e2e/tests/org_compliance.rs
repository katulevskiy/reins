//! Organizations (Decisions v2 · O5) end to end, the program's acceptance: a pilot org is provisioned and deprovisioned
//! end to end, and its audit export is verified by `reins-audit verify` (the program's own code path,
//! `reins_audit::cli::run`), which also catches tampering.
//!
//! On one real server with organizations on, a fake WorkOS, real phone cores, a mock Splunk collector and a mock RFC 3161
//! time-stamping authority:
//! 1. **Provision:** Acme is created, verifies its domain, binds SSO and its directory; Kim and Lee arrive by SCIM, add
//!    their work accounts and enroll their phones; Dana makes herself and Kim Security admins.
//! 2. **Configure:** an executor is registered and Splunk streaming turned on, each by a two-person device-signed
//!    change; the authority timestamps a checkpoint.
//! 3. **Deprovision:** the executor is removed and streaming turned off (two people); the directory deletes Lee and
//!    Kim, each gone within the minute; the sign-in binding is removed.
//! 4. **Verify:** the console's export and key file verify with `reins-audit verify`; `reins-audit report` and the
//!    console's evidence report show the provisioning and deprovisioning; Splunk got the chain in order, unchanged.
//! 5. **Tamper:** an edited event fails verification; a chain cut back and re-signed fails against an earlier head.

use std::path::Path;
use std::time::{Duration, Instant};

use reins_e2e::org::{
    Browser, Page, WEBHOOK_SECRET, add_work_account, between, csrf, db, device_sign, dir_user, enrollment_with_key,
    member_state, user_exists, wait_for, webhook,
};
use reins_e2e::workos::{FakeWorkos, User};
use reins_e2e::{Phone, Server};
use reins_proto::managed::{AuditEvent, ExportLine};
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

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

fn membership(server: &Server, email: &str) -> String {
    db(server)
        .query_row(
            "SELECT m.uuid FROM users_organizations m JOIN users u ON u.uuid = m.user_uuid WHERE u.email = ?1",
            [email],
            |r| r.get(0),
        )
        .unwrap()
}

fn count(server: &Server, sql: &str, org: &str) -> i64 {
    db(server).query_row(sql, [org], |r| r.get(0)).unwrap()
}

/// DER: one TLV.
fn tlv(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    if content.len() < 0x80 {
        out.push(u8::try_from(content.len()).unwrap());
    } else {
        let len = u16::try_from(content.len()).unwrap().to_be_bytes();
        out.extend([0x82, len[0], len[1]]);
    }
    out.extend_from_slice(content);
    out
}

/// A mock RFC 3161 authority: answers "granted" with a token echoing the request's message imprint and nonce.
struct Tsa;

impl Respond for Tsa {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        // TimeStampReq: SEQUENCE { version, messageImprint SEQUENCE {...}, nonce INTEGER, certReq }.
        let b = &req.body;
        let imprint_at = 2 + 3; // the outer header (short form), then version 02 01 01
        let imprint_len = 2 + usize::from(b[imprint_at + 1]);
        let imprint = &b[imprint_at..imprint_at + imprint_len];
        let nonce_at = imprint_at + imprint_len;
        let nonce = &b[nonce_at..nonce_at + 2 + usize::from(b[nonce_at + 1])];
        let mut tst = tlv(0x02, &[1]);
        tst.extend(tlv(0x06, &[0x2a, 0x03, 0x04]));
        tst.extend_from_slice(imprint);
        tst.extend(tlv(0x02, &[0x10, 0x92]));
        tst.extend(tlv(0x18, b"20261027120000Z"));
        tst.extend_from_slice(nonce);
        let token = tlv(0x30, &[tlv(0x06, &[0x2a, 0x86, 0x48]), tlv(0xa0, &tlv(0x04, &tlv(0x30, &tst)))].concat());
        let body = tlv(0x30, &[tlv(0x30, &tlv(0x02, &[0])), token].concat());
        ResponseTemplate::new(200).insert_header("content-type", "application/timestamp-reply").set_body_bytes(body)
    }
}

/// `reins-audit <args>`, as the program runs it.
fn reins_audit(args: &[&str]) -> reins_audit::cli::Outcome {
    let args: Vec<String> = args.iter().map(|a| (*a).to_owned()).collect();
    reins_audit::cli::run(&args, &mut std::io::empty())
}

fn write(dir: &Path, name: &str, body: &str) -> String {
    let p = dir.join(name);
    std::fs::write(&p, body).unwrap();
    p.to_string_lossy().into_owned()
}

/// Every audit event Splunk received, by sequence number.
async fn splunk_events(splunk: &MockServer) -> std::collections::BTreeMap<i64, AuditEvent> {
    let mut out = std::collections::BTreeMap::new();
    for r in splunk.received_requests().await.unwrap_or_default() {
        for line in String::from_utf8_lossy(&r.body).lines().filter(|l| !l.trim().is_empty()) {
            let v: Value = serde_json::from_str(line).unwrap();
            assert_eq!(v["sourcetype"], "reins:audit");
            let e: AuditEvent = serde_json::from_value(v["event"].clone()).unwrap();
            out.insert(e.seq, e);
        }
    }
    out
}

/// Two-person changes: Dana confirms her own on her phone, Kim approves it on hers.
async fn dana_then_kim(dana: &Member, kim: &Member, what: &str) {
    assert_eq!(dana.answer(true, |r| r.own).await, 1, "{what}: Dana's confirmation");
    assert_eq!(kim.answer(true, |r| r.title.contains(what)).await, 1, "{what}: Kim's approval");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_pilot_org_is_provisioned_deprovisioned_and_its_audit_verifies() {
    reins_e2e::init_tls();
    let workos = FakeWorkos::start().await;
    workos.add_organization("org_ACME", &["acme.com"]);
    workos.add_directory("directory_ACME", "org_ACME");
    let splunk = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/services/collector/event"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"text": "Success", "code": 0})))
        .mount(&splunk)
        .await;
    let tsa = MockServer::start().await;
    Mock::given(method("POST")).respond_with(Tsa).mount(&tsa).await;
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
            ("REINS_ORG_AUDIT_TSA_URL", tsa.uri()),
        ]
        .map(|(k, v)| (k.to_owned(), v)),
    );
    let server = Server::start_with_env(5, 3, &env).await;

    // ---- 1. Provision.
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

    // ---- 2. Configure: an executor, Splunk streaming, a timestamped checkpoint.
    let exec_dir = scratch.path().join("executor");
    std::fs::create_dir_all(&exec_dir).unwrap();
    let public = reins_executor::client::generate_key(&exec_dir.join("executor.key")).unwrap();
    let page = browser.get(&server.url(&format!("/reins/org/{org}/executors"))).await;
    browser
        .post(
            &server.url(&format!("/reins/org/{org}/executors")),
            &[("csrf", &csrf(&page)), ("executor_id", "e1"), ("mode", "self_hosted"), ("public_key", &public)],
        )
        .await;
    dana_then_kim(&dana, &kim, "executor").await;
    let streaming = server.url(&format!("/reins/org/{org}/audit/streaming"));
    let page = browser.get(&streaming).await;
    assert!(page.body.contains("Splunk") && page.body.contains("Not set up"), "{}", page.body);
    let page = browser
        .post(
            &server.url(&format!("/reins/org/{org}/audit/streaming/splunk")),
            &[("csrf", &csrf(&page)), ("url", &splunk.uri()), ("token", "hec-secret-token"), ("index", "")],
        )
        .await;
    assert!(page.body.contains("Saved") && !page.body.contains("hec-secret-token"), "{}", page.body);
    let page = browser
        .post(&server.url(&format!("/reins/org/{org}/audit/streaming/splunk/on")), &[("csrf", &csrf(&page))])
        .await;
    assert!(page.body.contains("Confirm it on your phone"), "{}", page.body);
    dana_then_kim(&dana, &kim, "Splunk").await;
    wait_for("streaming", Duration::from_secs(30), || {
        count(&server, "SELECT sent_seq FROM reins_org_audit_sinks WHERE org_uuid = ?1 AND kind = 'splunk'", &org) > 20
    })
    .await;
    wait_for("a timestamped checkpoint", Duration::from_secs(30), || {
        count(&server, "SELECT COUNT(*) FROM reins_org_witness WHERE org_uuid = ?1", &org) > 0
    })
    .await;
    let stamped: i64 = count(&server, "SELECT MIN(seq) FROM reins_org_witness WHERE org_uuid = ?1", &org);
    let tsr = browser.get(&server.url(&format!("/reins/org/{org}/audit/checkpoints/{stamped}.tsr"))).await;
    assert_eq!(tsr.status, 200);
    let cp = browser.get(&server.url(&format!("/reins/org/{org}/audit/checkpoints/{stamped}.json"))).await;
    assert!(cp.body.contains("\"kind\":\"checkpoint\""), "{}", cp.body);

    // ---- 3. Deprovision: the executor and streaming go (two people), then Lee and Kim (the directory), then the
    // sign-in binding.
    let page = browser.get(&server.url(&format!("/reins/org/{org}/executors"))).await;
    browser
        .post(
            &server.url(&format!("/reins/org/{org}/executors/remove")),
            &[("csrf", &csrf(&page)), ("executor_id", "e1")],
        )
        .await;
    dana_then_kim(&dana, &kim, "executor").await;
    let page = browser.get(&streaming).await;
    browser.post(&server.url(&format!("/reins/org/{org}/audit/streaming/splunk/off")), &[("csrf", &csrf(&page))]).await;
    dana_then_kim(&dana, &kim, "Splunk").await;
    let streamed_to: i64 =
        count(&server, "SELECT sent_seq FROM reins_org_audit_sinks WHERE org_uuid = ?1 AND kind = 'splunk'", &org);
    assert_eq!(
        count(&server, "SELECT enabled FROM reins_org_audit_sinks WHERE org_uuid = ?1 AND kind = 'splunk'", &org),
        0
    );
    for (n, (id, email)) in
        [("directory_user_lee", "lee@acme.com"), ("directory_user_kim", "kim@acme.com")].into_iter().enumerate()
    {
        workos.emit("dsync.user.deleted", dir_user(id, email, "inactive"));
        let started = Instant::now();
        webhook(&server, &json!({"id": format!("wh_del_{n}"), "event": "dsync.user.deleted"})).await;
        let gone = || {
            !user_exists(&server, email)
                || member_state(&server, email).as_deref() == Some("deprovisioned_by_directory")
        };
        let deadline = Instant::now() + Duration::from_secs(60);
        let mut retry = 0;
        while !gone() && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(200)).await;
            // WorkOS retries a delivery; a sync that lost a race for SQLite's write lock runs again on the next one.
            retry += 1;
            if retry % 15 == 0 {
                webhook(&server, &json!({"id": format!("wh_del_{n}_{retry}"), "event": "dsync.user.deleted"})).await;
            }
        }
        assert!(gone(), "{email}: {:?}\n{}", member_state(&server, email), server.log());
        assert!(started.elapsed() < Duration::from_secs(60), "{email} took {:?}", started.elapsed());
    }
    assert!(lee.phone.core.org_membership().await.is_err() && kim.phone.core.org_membership().await.is_err());
    let page = browser.get(&server.url(&format!("/reins/org/{org}"))).await;
    let page = browser
        .post(
            &server.url(&format!("/reins/org/{org}/identity")),
            &[
                ("csrf", &csrf(&page)),
                ("identity", "none"),
                ("workos_org_id", ""),
                ("directory_id", ""),
                ("github_org", ""),
            ],
        )
        .await;
    assert!(page.body.contains("Saved"), "{}", page.body);
    assert_eq!(member_state(&server, "dana@acme.com").as_deref(), Some("active"), "the owner stays to export");

    // ---- 4. Verify with reins-audit.
    let export_url = server.url(&format!("/reins/org/{org}/audit/export"));
    let first: Page = browser.get(&export_url).await;
    assert_eq!(first.status, 200);
    let keys: Page = browser.get(&server.url(&format!("/reins/org/{org}/audit/keys.json"))).await;
    let dir = scratch.path();
    let export_file = write(dir, "export.jsonl", &first.body);
    let keys_file = write(dir, "keys.json", &keys.body);
    let ok = reins_audit(&["verify", &export_file, "--keys", &keys_file]);
    assert_eq!(ok.code, 0, "{}", ok.stderr);
    assert!(ok.stdout.starts_with("OK: ") && ok.stdout.contains(&org), "{}", ok.stdout);
    let kinds: Vec<String> = first
        .body
        .lines()
        .filter_map(|l| match serde_json::from_str::<ExportLine>(l).unwrap() {
            ExportLine::Event(e) => Some(e.kind),
            _ => None,
        })
        .collect();
    for k in [
        "org.created",
        "domain.verified",
        "member.provisioned",
        "device.enrolled",
        "executor.registered",
        "audit.sink_installed",
        "audit.sink_changed",
        "executor.removed",
        "member.deprovisioned",
        "directory.unlinked",
    ] {
        assert!(kinds.iter().any(|x| x == k), "{k} missing: {kinds:?}");
    }
    assert!(!first.body.contains("hec-secret-token"), "no sink secret in the audit log");
    let soc2 = reins_audit(&["report", "--framework", "soc2", &export_file, "--keys", &keys_file]);
    assert_eq!(soc2.code, 0, "{}", soc2.stderr);
    let cc62 = between(&soc2.stdout, "CC6.2", "CC6.3");
    assert!(cc62.contains("member.provisioned"), "{}", soc2.stdout);
    let cc63 = between(&soc2.stdout, "CC6.3", "CC6.6");
    assert!(cc63.contains("member.deprovisioned"), "{}", soc2.stdout);
    let iso = browser.get(&server.url(&format!("/reins/org/{org}/audit/report?framework=iso27001&format=json"))).await;
    assert_eq!(iso.status, 200, "{}", iso.body);
    let iso: Value = serde_json::from_str(&iso.body).unwrap();
    assert_eq!(iso["org"], json!(org));
    assert!(iso["controls"].as_array().unwrap().iter().any(|c| c["id"] == "A.5.18" && c["total"].as_u64() > Some(0)));
    // Splunk got the chain up to where it stopped: in order, from the first event, every hash right.
    let received = splunk_events(&splunk).await;
    assert!(received.len() >= usize::try_from(streamed_to).unwrap(), "{} < {streamed_to}", received.len());
    let mut prev = reins_proto::org::GENESIS_HASH.to_owned();
    for (seq, e) in received.range(1..=streamed_to) {
        assert_eq!(e.org, org);
        assert_eq!(e.prev_hash, prev, "event {seq}");
        assert_eq!(e.hash, e.expected_hash(), "event {seq}");
        prev.clone_from(&e.hash);
    }
    assert_eq!(received.keys().next(), Some(&1));

    // ---- 5. Tamper.
    let anchor = {
        let last = first.body.lines().rev().find(|l| !l.trim().is_empty()).unwrap();
        let ExportLine::Head(env) = serde_json::from_str(last).unwrap() else {
            panic!("the export ends with its head")
        };
        env
    };
    let anchor_file = write(dir, "anchor.json", &serde_json::to_string(&anchor).unwrap());
    let anchor_seq = anchor.parse::<reins_proto::managed::Checkpoint>().unwrap().seq;
    let c = db(&server);
    let original: String = c
        .query_row("SELECT detail FROM reins_org_events WHERE org_uuid = ?1 AND seq = 4", [&org], |r| r.get(0))
        .unwrap();
    c.execute("UPDATE reins_org_events SET detail = '{\"edited\":true}' WHERE org_uuid = ?1 AND seq = 4", [&org])
        .unwrap();
    let edited = write(dir, "edited.jsonl", &browser.get(&export_url).await.body);
    let bad = reins_audit(&["verify", &edited, "--keys", &keys_file]);
    assert_eq!(bad.code, 1);
    assert!(bad.stdout.is_empty() && bad.stderr.contains("event 4 was changed"), "{}", bad.stderr);
    let bad = reins_audit(&["report", "--framework", "soc2", &edited, "--keys", &keys_file]);
    assert_eq!((bad.code, bad.stdout.as_str()), (1, ""), "no report from a tampered export");
    c.execute("UPDATE reins_org_events SET detail = ?1 WHERE org_uuid = ?2 AND seq = 4", [original, org.clone()])
        .unwrap();
    // A server rolling the chain back and re-signing it: whole alone, caught against the head kept from before.
    c.execute("DELETE FROM reins_org_events WHERE org_uuid = ?1 AND seq >= ?2", rusqlite::params![org, anchor_seq])
        .unwrap();
    c.execute(
        "DELETE FROM reins_org_checkpoints WHERE org_uuid = ?1 AND seq >= ?2",
        rusqlite::params![org, anchor_seq],
    )
    .unwrap();
    let cut = write(dir, "cut.jsonl", &browser.get(&export_url).await.body);
    assert_eq!(reins_audit(&["verify", &cut, "--keys", &keys_file]).code, 0, "alone, a re-signed chain looks whole");
    let caught = reins_audit(&["verify", &cut, "--keys", &keys_file, "--anchor", &anchor_file]);
    assert_eq!(caught.code, 1);
    assert!(caught.stderr.contains("does not extend"), "{}", caught.stderr);
}
