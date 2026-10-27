//! Policy as code end to end (Decisions v2 · O4): the real server with organizations on, a fake WorkOS, real phone
//! cores for three members, two agents, the real org executor in this process, and one fake GitHub serving both the
//! executor's App and the org's GitHub reader (the policy repo `acme/reins-policy`).
//!
//! - `a_policy_change_goes_from_pr_to_everyone_with_evidence` (07's O4 line): a PR's check run carries the simulation
//!   line; the merge only proposes the activation; the proposer can't approve it and a second Security admin signs;
//!   the change rolls out pilot → 25% → everyone as each stage's dwell passes with the guards green; every decision
//!   records its version and rollout step; the executor runs them; the audit export verifies.
//! - `a_bad_change_rolls_back_by_itself`: a candidate that asks the pilot much more rolls back on the controller's
//!   own tick, and the executor then refuses what was decided under the candidate.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use reins_e2e::org::{
    Browser, WEBHOOK_SECRET, add_work_account, csrf, db, device_sign, dir_user, enrollment_with_key, member_state,
    wait_for, webhook,
};
use reins_e2e::workos::{FakeWorkos, User};
use reins_e2e::{AiClient, Phone, Server};
use reins_executor::connectors::github::GithubApp;
use reins_executor::{Client, Context, run_once};
use serde_json::{Value, json};
use wiremock::matchers::{method, path, path_regex};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

/// The reader App's webhook secret.
const HOOK_SECRET: &str = "policy-repo-webhook-secret";
const READER_INSTALLATION: i64 = 88;

struct Member {
    phone: Phone,
    key: Vec<u8>,
    org_id: String,
}

impl Member {
    /// Answers every review on this phone that `pick` selects; returns how many.
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
    for inst in [77, READER_INSTALLATION] {
        Mock::given(method("POST"))
            .and(path(format!("/app/installations/{inst}/access_tokens")))
            .respond_with(Tokens(AtomicUsize::new(0)))
            .mount(&gh)
            .await;
    }
    for (name, id) in [("api", 101), ("web", 103)] {
        Mock::given(method("GET"))
            .and(path(format!("/repos/acme/{name}")))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"id": id, "full_name": format!("acme/{name}")})),
            )
            .mount(&gh)
            .await;
    }
    Mock::given(method("GET"))
        .and(path("/repos/acme/reins-policy"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"full_name": "acme/reins-policy", "default_branch": "main"})),
        )
        .mount(&gh)
        .await;
    Mock::given(method("POST"))
        .and(path_regex(r"^/repos/acme/[a-z-]+/issues$"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"number": 42})))
        .mount(&gh)
        .await;
    Mock::given(method("POST"))
        .and(path("/repos/acme/reins-policy/check-runs"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"id": 1})))
        .mount(&gh)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/installation/token"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&gh)
        .await;
    gh
}

fn hex_sha256(bytes: &[u8]) -> String {
    reins_proto::canon::hex32(&reins_proto::canon::sha256(bytes))
}

/// Serves the policy repo's files at commit `sha` (its tree and blobs).
async fn mount_commit(gh: &MockServer, sha: &str, files: &[(String, String)]) {
    let mut tree = Vec::new();
    for (p, text) in files {
        let blob = hex_sha256(text.as_bytes());
        tree.push(json!({"path": p, "type": "blob", "sha": blob, "size": text.len()}));
        Mock::given(method("GET"))
            .and(path(format!("/repos/acme/reins-policy/git/blobs/{blob}")))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"content": data_encoding::BASE64.encode(text.as_bytes())})),
            )
            .mount(gh)
            .await;
    }
    tree.push(json!({"path": "README.md", "type": "blob", "sha": hex_sha256(b"readme"), "size": 6}));
    Mock::given(method("GET"))
        .and(path(format!("/repos/acme/reins-policy/git/trees/{sha}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"tree": tree, "truncated": false})))
        .mount(gh)
        .await;
}

/// The policy repo: the org's current rules plus `extra`, rolling out pilot → 25% → everyone with a second's dwell and
/// guards judged at `min` decisions per cohort.
fn repo(extra: &str, min: u32) -> Vec<(String, String)> {
    vec![
        (
            "reins-policy.toml".into(),
            format!(
                "version = 1\norg = \"acme\"\nexplain = \"Developer baseline\"\n\n[rollout]\npilot_group = \"Pilot\"\n\
stages = [\"pilot\", \"25%\", \"100%\"]\ndwell = \"1s\"\n\
guards = {{ max_ask_rate_delta_pp = 2.0, max_median_ttd = \"30m\", min_decisions = {min} }}\n"
            ),
        ),
        (
            "matrix.toml".into(),
            "mine = [\"auto\", \"auto\", \"auto\", \"ask\"]\nfriends = [\"auto\", \"ask_first\", \"auto\", \"ask\"]\n\
work = [\"auto\", \"auto\", \"auto\", \"ask\"]\noutside = [\"ask\", \"ask\", \"ask\", \"ask\"]\n"
                .into(),
        ),
        ("facts.toml".into(), "[owners]\nacme = \"work\"\n".into()),
        (
            "rules/prod.toml".into(),
            format!(
                "[[rule]]\nid = \"prod-review\"\nexplain = \"Production needs Security\"\n\
when = {{ resource = \"acme/prod-*\" }}\nreview = {{ reviewers = \"role: security_admin\", class = \"deferred\" }}\n\n\
[[rule]]\nid = \"prod-deny\"\nexplain = \"Not through agents\"\nwhen = {{ resource = \"acme/forbidden\" }}\ndeny = true\n{extra}"
            ),
        ),
    ]
}

fn sign(body: &[u8]) -> String {
    let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, HOOK_SECRET.as_bytes());
    format!("sha256={}", data_encoding::HEXLOWER.encode(ring::hmac::sign(&key, body).as_ref()))
}

/// Posts a GitHub webhook from the reader App: its status.
async fn github_hook(server: &Server, event: &str, body: &Value, signature: Option<&str>) -> u16 {
    let text = body.to_string();
    let sig = signature.map_or_else(|| sign(text.as_bytes()), str::to_owned);
    reqwest::Client::new()
        .post(server.url("/reins/org/github/policy-webhook"))
        .header("X-GitHub-Event", event)
        .header("X-GitHub-Delivery", uuid::Uuid::new_v4().to_string())
        .header("X-Hub-Signature-256", sig)
        .header("Content-Type", "application/json")
        .body(text)
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}

fn pr_event(number: u64, sha: &str) -> Value {
    json!({"action": "opened", "number": number, "installation": {"id": READER_INSTALLATION},
           "repository": {"full_name": "acme/reins-policy"},
           "pull_request": {"number": number, "base": {"ref": "main"}, "head": {"sha": sha}}})
}

fn push_event(sha: &str) -> Value {
    json!({"ref": "refs/heads/main", "after": sha, "installation": {"id": READER_INSTALLATION},
           "repository": {"full_name": "acme/reins-policy"}})
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

fn one<T: rusqlite::types::FromSql>(server: &Server, sql: &str, org: &str) -> T {
    db(server).query_row(sql, [org], |r| r.get(0)).unwrap()
}

/// The org's newest rollout: (id, state, published seq, candidate).
fn rollout(server: &Server, org: &str) -> Option<(String, String, i64, String)> {
    db(server)
        .query_row(
            "SELECT rollout_id, state, published_seq, candidate FROM reins_org_rollouts WHERE org_uuid = ?1 \
             ORDER BY epoch DESC LIMIT 1",
            [org],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .ok()
}

/// A request as stored: (id, state, snapshot, rollout id, rollout seq).
type Row = (String, String, String, Option<String>, Option<i64>);

/// Requests of `email` since `since`.
fn requests_of(server: &Server, email: &str, since: i64) -> Vec<Row> {
    let c = db(server);
    let mut st = c
        .prepare(
            "SELECT r.uuid, r.state, r.snapshot, r.rollout_id, r.rollout_seq FROM reins_org_requests r \
             JOIN users u ON u.uuid = r.user_uuid WHERE u.email = ?1 AND r.created_at >= ?2 ORDER BY r.created_at",
        )
        .unwrap();
    st.query_map(rusqlite::params![email, since], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
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

async fn pair_agent(server: &Server, member: &Member, email: &str) -> AiClient {
    let mut ai = AiClient::new(&server.base);
    ai.register_client().await;
    let wait_url = ai.start_authorization(email).await;
    let code = ai.browser_code(&wait_url).await;
    let item = member.phone.wait_for_item(Duration::from_secs(20)).await;
    member.phone.core.answer_pairing(item.id, true, Some(code), Some("Agent".to_owned()), true).await.unwrap();
    ai.finish_authorization(&wait_url).await;
    ai
}

/// The pilot org, ready for policy as code.
struct World {
    _workos: FakeWorkos,
    gh: MockServer,
    server: Server,
    _scratch: tempfile::TempDir,
    browser: Browser,
    org: String,
    dana: Member,
    kim: Member,
    /// Lee's agent and Kim's agent.
    lee_ai: AiClient,
    kim_ai: AiClient,
    ctx: Arc<Context>,
    paused: Arc<AtomicBool>,
    decision_key: reins_audit::TrustedKey,
}

#[allow(clippy::too_many_lines, reason = "the org, its phones, policy, executor, reader and repo, in order")]
async fn world() -> World {
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
            ("REINS_TEST_GITHUB_API", gh.uri()),
        ]
        .map(|(k, v)| (k.to_owned(), v)),
    );
    let server = Server::start_with_env(5, 3, &env).await;

    // Dana creates Acme, verifies acme.com and binds SSO and the directory; Kim and Lee arrive through SCIM.
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
    let record = reins_e2e::org::between(&created.body, "_reins-challenge.acme.com<br>", "</div>").to_owned();
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
    let lee_member = enroll(lee_phone).await;

    // Dana and Kim are Security admins.
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
    // The pilot group: Lee.
    db(&server)
        .execute(
            "INSERT INTO groups (uuid, organizations_uuid, name, access_all, creation_date, revision_date) \
             VALUES ('g-pilot', ?1, 'Pilot', 0, '2026-10-27 00:00:00', '2026-10-27 00:00:00')",
            [&org],
        )
        .unwrap();
    db(&server)
        .execute(
            "INSERT INTO groups_users (groups_uuid, users_organizations_uuid) VALUES ('g-pilot', ?1)",
            [membership(&server, "lee@acme.com")],
        )
        .unwrap();

    // The first policy (no history to simulate yet): drafted in the console, confirmed by Dana, approved by Kim.
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
    let hash: String = one(&server, "SELECT hash FROM reins_org_policies WHERE org_uuid = ?1 AND revision = 1", &org);
    browser
        .post(
            &server.url(&format!("/reins/org/{org}/policy/activate")),
            &[("csrf", &csrf(&page)), ("revision", "1"), ("hash", &hash)],
        )
        .await;
    assert_eq!(dana.answer(true, |r| r.own).await, 1);
    assert_eq!(kim.answer(true, |r| r.title.contains("policy")).await, 1);

    // The executor.
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
    };
    let decision_key = pins.decision.clone();
    let pem = reins_executor::connectors::testing::test_pem();
    let ctx = Arc::new(
        Context::new(client, pins, &exec_dir.join("state"))
            .unwrap()
            .with_github(GithubApp::new(1, 77, "acme", &pem, &gh.uri()).unwrap()),
    );
    let paused = Arc::new(AtomicBool::new(false));
    {
        let (ctx, paused) = (Arc::clone(&ctx), Arc::clone(&paused));
        tokio::spawn(async move {
            loop {
                if !paused.load(Ordering::SeqCst) {
                    drop(run_once(&ctx).await);
                }
                tokio::time::sleep(Duration::from_millis(300)).await;
            }
        });
    }

    // The GitHub reader (its ids by two admins; its key and webhook secret posted apart) and the policy repo.
    let page = browser.get(&server.url(&format!("/reins/org/{org}/policy/repo"))).await;
    let page = browser
        .post(
            &server.url(&format!("/reins/org/{org}/policy/reader")),
            &[("csrf", &csrf(&page)), ("app_id", "2"), ("installation_id", &READER_INSTALLATION.to_string())],
        )
        .await;
    assert_eq!(kim.answer(true, |r| r.title.contains("GitHub reader")).await, 1);
    let page = browser
        .post(
            &server.url(&format!("/reins/org/{org}/policy/reader/secrets")),
            &[("csrf", &csrf(&page)), ("pem", &pem), ("webhook_secret", HOOK_SECRET)],
        )
        .await;
    let page = browser
        .post(
            &server.url(&format!("/reins/org/{org}/policy/repo")),
            &[("csrf", &csrf(&page)), ("repo", "acme/reins-policy")],
        )
        .await;
    assert!(page.body.contains("acme/reins-policy"), "{}", page.body);

    let lee_ai = pair_agent(&server, &lee_member, "lee@acme.com").await;
    let kim_ai = pair_agent(&server, &kim, "kim@acme.com").await;
    World {
        _workos: workos,
        gh,
        server,
        _scratch: scratch,
        browser,
        org,
        dana,
        kim,
        lee_ai,
        kim_ai,
        ctx,
        paused,
        decision_key,
    }
}

/// `n` issues on `repo` by `ai`.
async fn issues(ai: &mut AiClient, repo: &str, n: usize) {
    for i in 0..n {
        ai.tool("github_create_issue", &json!({"repo": repo, "title": format!("t{i}")})).await;
    }
}

fn now() -> i64 {
    reins_executor::now()
}

/// Waits (the controller ticks with the sweeper, every 15 s) for the rollout to reach `seq` or `state`.
async fn wait_rollout(w: &World, what: &str, done: impl Fn(&(String, String, i64, String)) -> bool) {
    wait_for(what, Duration::from_secs(60), || rollout(&w.server, &w.org).as_ref().is_some_and(&done)).await;
}

/// The merge proposes the activation; Dana confirms it (and can't approve her own), Kim approves: the rollout starts.
async fn merge_and_activate(w: &World, sha: &str) {
    assert_eq!(github_hook(&w.server, "push", &push_event(sha), None).await, 202);
    assert!(rollout(&w.server, &w.org).is_none(), "a merge applies nothing");
    let pending = w.dana.phone.core.org_reviews().await.unwrap();
    let change = pending.iter().find(|r| r.requester == "Policy repo").expect("the merge's activation").clone();
    assert!(change.own, "every Security admin may confirm it");
    // A first admin confirms it on her phone...
    assert_eq!(w.dana.answer(true, |r| r.id == change.id).await, 1);
    assert!(rollout(&w.server, &w.org).is_none(), "one person never activates");
    // ...and can't approve it too.
    let mut as_approval = change.clone();
    as_approval.own = false;
    let at = now();
    let msg = reins_core::profiles::org_review_message(w.dana.org_id.clone(), as_approval, true, at);
    assert!(w.dana.phone.core.org_decide(change.id.clone(), true, at, device_sign(&w.dana.key, &msg)).await.is_err());
    // A second one approves on hers.
    assert_eq!(w.kim.answer(true, |r| r.id == change.id).await, 1);
    let r = rollout(&w.server, &w.org).expect("the rollout started");
    assert_eq!((r.1.as_str(), r.2), ("running", 1));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[allow(clippy::too_many_lines, reason = "one change from PR to everyone, in order")]
async fn a_policy_change_goes_from_pr_to_everyone_with_evidence() {
    let mut w = world().await;
    let stable: String =
        one(&w.server, "SELECT hash FROM reins_org_policies WHERE org_uuid = ?1 AND state = 'active'", &w.org);
    // Some history to simulate on.
    issues(&mut w.lee_ai, "acme/api", 2).await;
    issues(&mut w.kim_ai, "acme/web", 2).await;

    // ---- 1. A PR adds a review rule: its check run carries the simulation line. A forged webhook does nothing.
    let rule = "\n[[rule]]\nid = \"vault-review\"\nexplain = \"Vault repos need Security\"\n\
when = { resource = \"acme/vault-*\" }\nreview = { reviewers = \"role: security_admin\", class = \"deferred\" }\n";
    let files = repo(rule, 3);
    let pr_sha = "1111111111111111111111111111111111111111";
    mount_commit(&w.gh, pr_sha, &files).await;
    assert_eq!(github_hook(&w.server, "pull_request", &pr_event(5, pr_sha), Some("sha256=00")).await, 401);
    assert_eq!(github_hook(&w.server, "pull_request", &pr_event(5, pr_sha), None).await, 202);
    let runs: Vec<Value> =
        w.gh.received_requests()
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|r| r.method.as_str() == "POST" && r.url.path().ends_with("/check-runs"))
            .map(|r| serde_json::from_slice(&r.body).unwrap())
            .collect();
    assert_eq!(runs.len(), 1, "one check run");
    assert_eq!((runs[0]["name"].as_str(), runs[0]["conclusion"].as_str()), (Some("reins/policy"), Some("neutral")));
    let title = runs[0]["output"]["title"].as_str().unwrap();
    assert!(title.starts_with("Ask rate +0.0%, 0 → Auto, 0 → Ask"), "{title}");
    assert_eq!(runs[0]["head_sha"], pr_sha);

    // ---- 2-3. The merge only proposes the activation; two Security admins activate it.
    let merge_sha = "2222222222222222222222222222222222222222";
    mount_commit(&w.gh, merge_sha, &files).await;
    merge_and_activate(&w, merge_sha).await;
    let (rid, _, _, candidate) = rollout(&w.server, &w.org).unwrap();
    assert_ne!(candidate, stable);

    // ---- 4-6. Pilot (Lee) → 25% → everyone, as each stage's dwell passes with enough decisions and green guards.
    let stage_start = now();
    issues(&mut w.lee_ai, "acme/api", 3).await;
    issues(&mut w.kim_ai, "acme/api", 3).await;
    for (who, want) in [("lee@acme.com", &candidate), ("kim@acme.com", &stable)] {
        let rows = requests_of(&w.server, who, stage_start);
        assert_eq!(rows.len(), 3);
        for (_, _, snapshot, rollout_id, seq) in &rows {
            assert_eq!((snapshot, rollout_id.as_deref(), *seq), (want, Some(rid.as_str()), Some(1)), "{who}");
        }
    }
    wait_rollout(&w, "stage 25%", |r| r.2 == 2).await;
    let stage_start = now();
    issues(&mut w.lee_ai, "acme/api", 3).await;
    issues(&mut w.kim_ai, "acme/api", 3).await;
    for (_, _, snapshot, _, seq) in requests_of(&w.server, "lee@acme.com", stage_start) {
        assert_eq!((snapshot.as_str(), seq), (candidate.as_str(), Some(2)), "the pilot stays on the candidate");
    }
    wait_rollout(&w, "everyone", |r| r.2 == 3).await;
    let stage_start = now();
    issues(&mut w.lee_ai, "acme/api", 3).await;
    issues(&mut w.kim_ai, "acme/api", 3).await;
    for (_, _, snapshot, _, seq) in requests_of(&w.server, "kim@acme.com", stage_start) {
        assert_eq!((snapshot.as_str(), seq), (candidate.as_str(), Some(3)), "everyone");
    }
    wait_rollout(&w, "complete", |r| r.1 == "complete").await;
    let active: String =
        one(&w.server, "SELECT hash FROM reins_org_policies WHERE org_uuid = ?1 AND state = 'active'", &w.org);
    assert_eq!(active, candidate);

    // ---- 7. The executor ran every decision (verified against the signed rollout), and none was refused.
    wait_for("executions", Duration::from_secs(30), || {
        let n: i64 = one(
            &w.server,
            "SELECT count(*) FROM reins_org_requests WHERE org_uuid = ?1 AND state IN ('approved', 'executing')",
            &w.org,
        );
        n == 0
    })
    .await;
    let refused: i64 = one(
        &w.server,
        "SELECT count(*) FROM reins_org_requests WHERE org_uuid = ?1 AND state IN ('refused', 'stale')",
        &w.org,
    );
    assert_eq!(refused, 0);
    let done: i64 =
        one(&w.server, "SELECT count(*) FROM reins_org_requests WHERE org_uuid = ?1 AND state = 'done'", &w.org);
    assert_eq!(done, 22);

    // ---- 8. The audit export verifies, with the whole change in it.
    let text = w.browser.get(&w.server.url(&format!("/reins/org/{}/audit/export", w.org))).await.body;
    reins_audit::verify(text.as_bytes(), std::slice::from_ref(&w.decision_key), None).unwrap();
    for kind in [
        "policy_repo.checked",
        "policy.simulated",
        "change.adopted",
        "policy.activated",
        "policy.stage_advanced",
        "policy.rollout_completed",
    ] {
        assert!(text.contains(&format!("\"kind\":\"{kind}\"")), "{kind} missing");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_bad_change_rolls_back_by_itself() {
    let mut w = world().await;
    let stable: String =
        one(&w.server, "SELECT hash FROM reins_org_policies WHERE org_uuid = ?1 AND state = 'active'", &w.org);
    // The change reviews every issue on acme/web.
    let rule = "\n[[rule]]\nid = \"web-review\"\nwhen = { resource = \"acme/web\" }\n\
review = { reviewers = \"role: security_admin\", class = \"deferred\" }\n";
    let sha = "3333333333333333333333333333333333333333";
    mount_commit(&w.gh, sha, &repo(rule, 10)).await;
    merge_and_activate(&w, sha).await;
    let (rid, _, _, candidate) = rollout(&w.server, &w.org).unwrap();

    // The executor is held, so some of Lee's candidate decisions are still waiting to run when the rollback lands.
    w.paused.store(true, Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(400)).await;
    // Lee (the pilot, on the candidate) asks 2 of 10; Kim (stable) 0 of 10: 20 points more, over the 2-point guard.
    issues(&mut w.lee_ai, "acme/web", 2).await;
    issues(&mut w.lee_ai, "acme/api", 8).await;
    issues(&mut w.kim_ai, "acme/web", 10).await;
    let lee_rows = requests_of(&w.server, "lee@acme.com", 0);
    assert!(lee_rows.iter().all(|r| r.2 == candidate && r.3.as_deref() == Some(rid.as_str())));
    wait_rollout(&w, "rollback", |r| r.1 == "rolled_back").await;
    let reason: String = one(&w.server, "SELECT reason FROM reins_org_rollouts WHERE org_uuid = ?1", &w.org);
    assert!(reason.contains("ask rate"), "{reason}");
    let state: String = one(
        &w.server,
        "SELECT state FROM reins_org_policies WHERE org_uuid = ?1 AND hash = (SELECT candidate FROM reins_org_rollouts WHERE org_uuid = ?1)",
        &w.org,
    );
    assert_eq!(state, "draft");
    let active: String =
        one(&w.server, "SELECT hash FROM reins_org_policies WHERE org_uuid = ?1 AND state = 'active'", &w.org);
    assert_eq!(active, stable);

    // The executor now holds the signed rollback: the candidate's decisions are refused, the stable ones run.
    w.paused.store(false, Ordering::SeqCst);
    run_once(&w.ctx).await.unwrap();
    let busy = |r: &Row| r.1 == "approved" || r.1 == "executing";
    wait_for("receipts", Duration::from_secs(30), || {
        !requests_of(&w.server, "lee@acme.com", 0).iter().any(busy)
            && !requests_of(&w.server, "kim@acme.com", 0).iter().any(busy)
    })
    .await;
    let lee_states: Vec<String> =
        requests_of(&w.server, "lee@acme.com", 0).into_iter().filter(|r| r.1 != "reviewing").map(|r| r.1).collect();
    assert_eq!(lee_states.len(), 8);
    assert!(lee_states.iter().all(|s| s == "refused"), "{lee_states:?}");
    let kim_rows = requests_of(&w.server, "kim@acme.com", 0);
    assert!(kim_rows.iter().all(|r| r.1 == "done"), "{kim_rows:?}");
    // And a new decision for Lee is on the stable policy again.
    issues(&mut w.lee_ai, "acme/api", 1).await;
    let last = requests_of(&w.server, "lee@acme.com", 0).pop().unwrap();
    assert_eq!(last.2, stable);
}

/// A Family (P7's tier, by its stored name): its only guardian activates alone, with her phone's signature; an
/// ordinary org with one Security admin can't (covered in the server's tests).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_family_guardian_activates_alone() {
    reins_e2e::init_tls();
    let workos = FakeWorkos::start().await;
    workos.add_organization("org_FAM", &["fam.example"]);
    let server = Server::start_with_env(5, 3, &{
        let mut env = workos.server_env();
        env.push(("REINS_ORGS_ENABLED".to_owned(), "true".to_owned()));
        env
    })
    .await;
    workos.sign_in_as(&User {
        id: "user_mom".to_owned(),
        email: "mom@fam.example".to_owned(),
    });
    let phone = Phone::signed_out(|_| {}).await;
    phone.sso_sign_in(&server.base).await.unwrap();
    phone.core.register_device(None).await.unwrap();
    let mut browser = Browser::new();
    let home = browser.get(&server.url("/reins/org/login")).await;
    let created =
        browser.post(&server.url("/reins/org/new"), &[("csrf", &csrf(&home)), ("name", "Fam"), ("tier", "team")]).await;
    let org = created.url.rsplit('/').next().unwrap().to_owned();
    db(&server).execute("UPDATE reins_orgs SET tier = 'family' WHERE org_uuid = ?1", [&org]).unwrap();
    let mom = enroll(phone).await;
    let mut page = browser.get(&server.url(&format!("/reins/org/{org}/members"))).await;
    let subject = format!("member:{}", membership(&server, "mom@fam.example"));
    page = browser
        .post(
            &server.url(&format!("/reins/org/{org}/roles")),
            &[("csrf", &csrf(&page)), ("subject", &subject), ("role", "security_admin"), ("op", "grant")],
        )
        .await;
    let page = browser
        .post(
            &server.url(&format!("/reins/org/{org}/policy/draft")),
            &[("csrf", &csrf(&page)), ("body", include_str!("../../../src/api/reins/org/policy_baseline.json"))],
        )
        .await;
    let hash: String = one(&server, "SELECT hash FROM reins_org_policies WHERE org_uuid = ?1 AND revision = 1", &org);
    let page = browser
        .post(
            &server.url(&format!("/reins/org/{org}/policy/activate")),
            &[("csrf", &csrf(&page)), ("revision", "1"), ("hash", &hash)],
        )
        .await;
    assert!(page.body.contains("Confirm it on your phone"), "{}", page.body);
    let state: String = one(&server, "SELECT state FROM reins_org_policies WHERE org_uuid = ?1 AND revision = 1", &org);
    assert_eq!(state, "draft", "not before her phone signs");
    assert_eq!(mom.answer(true, |r| r.own).await, 1);
    let state: String = one(&server, "SELECT state FROM reins_org_policies WHERE org_uuid = ?1 AND revision = 1", &org);
    assert_eq!(state, "active");
}
