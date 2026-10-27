//! The Family tenant (Decisions v2 · P7, 08 R7) end to end: the real server with organizations on, a fake WorkOS,
//! real phone cores for a guardian and a child, an AI client paired to the child's family account, the real executor
//! (in this process, over HTTP) holding the child's Google token, and a loopback OAuth and Gmail mock.
//!
//! 1. Mom creates a family in the console; its baseline policy activates on her phone alone (she is its only guardian).
//! 2. She invites Sam as a minor; Sam signs in with a new account on a second core and joins.
//! 3. Mom connects a Google account for Sam: the executor makes the PKCE verifier, the provider sends her browser back
//!    with a code the server can't redeem, and only the executor ever holds the token.
//! 4. Sam's agent sends an email to an Outside address: Sam and a guardian must approve; Mom approves on her phone,
//!    and the executor sends it as Sam through the mock.
//! 5. Sam asks to leave: it waits for a guardian; Mom allows it; Sam is deprovisioned within a minute, and the Google
//!    grant is revoked upstream.

use std::sync::Arc;
use std::time::{Duration, Instant};

use reins_e2e::org::{Browser, csrf, db, device_sign, enrollment_with_key, member_state, wait_for};
use reins_e2e::workos::{FakeWorkos, User};
use reins_e2e::{AiClient, Phone, Server};
use reins_executor::connectors::UserOAuth;
use reins_executor::connectors::user_oauth::Client as OAuthClient;
use reins_executor::user_tokens::UserTokens;
use reins_executor::{Client, Context, run_once};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};
use zeroize::Zeroizing;

/// A phone with its enrolled org device key.
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
    let org_id = phone.core.org_membership().await.unwrap().expect("a family account").org_id;
    Member {
        phone,
        key,
        org_id,
    }
}

/// The provider's sign-in page: sends the browser straight back to the `redirect_uri` with a code for its `state`.
struct Consent;

impl Respond for Consent {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let q: std::collections::HashMap<String, String> = req.url.query_pairs().into_owned().collect();
        let back = format!("{}?code=CODE-SAM&state={}", q["redirect_uri"], q["state"]);
        ResponseTemplate::new(302).insert_header("location", back.as_str())
    }
}

/// A loopback Google: sign-in, token (code and refresh), userinfo, Gmail send, revocation.
async fn fake_google() -> MockServer {
    let g = MockServer::start().await;
    Mock::given(method("GET")).and(path("/authorize")).respond_with(Consent).mount(&g).await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(wiremock::matchers::body_string_contains("grant_type=authorization_code"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"access_token": "ya29.first", "refresh_token": "1//sam"})),
        )
        .mount(&g)
        .await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(wiremock::matchers::body_string_contains("grant_type=refresh_token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"access_token": "ya29.sam"})))
        .mount(&g)
        .await;
    Mock::given(method("GET"))
        .and(path("/userinfo"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"email": "sam.kid@gmail.com"})))
        .mount(&g)
        .await;
    Mock::given(method("POST"))
        .and(path("/gmail/v1/users/me/messages/send"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "m1", "threadId": "t1"})))
        .mount(&g)
        .await;
    Mock::given(method("POST")).and(path("/revoke")).respond_with(ResponseTemplate::new(200)).mount(&g).await;
    g
}

async fn hits(g: &MockServer, m: &str, p: &str) -> Vec<Request> {
    g.received_requests()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|r| r.method.as_str() == m && r.url.path() == p)
        .collect()
}

/// Every text the server keeps about family connections and events.
fn server_texts(server: &Server) -> String {
    let c = db(server);
    let mut out = String::new();
    for sql in [
        "SELECT COALESCE(oauth_state,'') || COALESCE(sealed_code,'') || COALESCE(auth_url,'') FROM reins_family_connect_jobs",
        "SELECT COALESCE(account,'') || state FROM reins_family_connections",
        "SELECT detail FROM reins_org_events",
    ] {
        let mut st = c.prepare(sql).unwrap();
        for row in st.query_map([], |r| r.get::<_, String>(0)).unwrap() {
            out.push_str(&row.unwrap());
        }
    }
    out
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[allow(clippy::too_many_lines, reason = "one family through every P7 step, in order")]
async fn family_end_to_end() {
    reins_e2e::init_tls();
    let workos = FakeWorkos::start().await;
    let google = fake_google().await;
    let scratch = tempfile::tempdir().unwrap();
    let mut env = workos.server_env();
    env.push(("REINS_ORGS_ENABLED".to_owned(), "true".to_owned()));
    let server = Server::start_with_env(5, 3, &env).await;

    // ---- 1. Mom creates the family; its baseline activates on her phone alone.
    let mom_user = User {
        id: "user_mom".to_owned(),
        email: "mom@lees.example".to_owned(),
    };
    workos.sign_in_as(&mom_user);
    let mom_phone = Phone::signed_out(|_| {}).await;
    mom_phone.sso_sign_in(&server.base).await.unwrap();
    mom_phone.core.register_device(None).await.unwrap();
    let mut browser = Browser::new();
    let home = browser.get(&server.url("/reins/org/login")).await;
    assert!(home.body.contains("New family"), "{}", home.body);
    let created = browser
        .post(&server.url("/reins/org/new"), &[("csrf", &csrf(&home)), ("name", "The Lees"), ("tier", "family")])
        .await;
    let org = created.url.rsplit('/').next().unwrap().to_owned();
    assert!(created.body.contains("Family"), "{}", created.body);
    let mom = enroll(mom_phone).await;
    assert_eq!(mom.org_id, org);
    assert_eq!(mom.answer(true, |r| r.own).await, 1, "the baseline's activation, confirmed on her phone");
    let state: String = db(&server)
        .query_row("SELECT state FROM reins_org_policies WHERE org_uuid = ?1 AND revision = 1", [&org], |r| r.get(0))
        .unwrap();
    assert_eq!(state, "active", "a single guardian activates alone");

    // The family's executor: registered by Mom's device-signed change, holding its own OAuth client for Google.
    let exec_dir = scratch.path().join("executor");
    std::fs::create_dir_all(&exec_dir).unwrap();
    let key_file = exec_dir.join("executor.key");
    let public = reins_executor::client::generate_key(&key_file).unwrap();
    let page = browser.get(&server.url(&format!("/reins/org/{org}/executors"))).await;
    browser
        .post(
            &server.url(&format!("/reins/org/{org}/executors")),
            &[("csrf", &csrf(&page)), ("executor_id", "home"), ("mode", "cloud"), ("public_key", &public)],
        )
        .await;
    assert_eq!(mom.answer(true, |r| r.own).await, 1);
    let client =
        Client::new(&server.base, &org, "home", Arc::new(reins_executor::client::load_key(&key_file).unwrap()))
            .unwrap();
    let keys = client.keys().await.unwrap();
    let pin = |(id, b64): &(String, String)| reins_audit::TrustedKey::parse(&format!("{id}={b64}")).unwrap();
    let pins = reins_executor::Pins {
        org: org.clone(),
        issuer: pin(&keys.issuer),
        decision: pin(&keys.decision),
    };
    let state_dir = exec_dir.join("state");
    std::fs::create_dir_all(&state_dir).unwrap();
    let tokens = Arc::new(UserTokens::open(&state_dir, &std::fs::read(&key_file).unwrap()).unwrap());
    let oauth = UserOAuth::new(
        vec![OAuthClient::google("family-client", Zeroizing::new("family-secret".to_owned())).at(&google.uri())],
        Arc::clone(&tokens),
        &state_dir,
    )
    .unwrap();
    let ctx = Arc::new(Context::new(client, pins, &state_dir).unwrap().with_user_oauth(Arc::new(oauth)));
    let runner = {
        let ctx = Arc::clone(&ctx);
        tokio::spawn(async move {
            loop {
                drop(run_once(&ctx).await);
                tokio::time::sleep(Duration::from_millis(300)).await;
            }
        })
    };

    // ---- 2. Mom invites Sam as a minor; Sam joins with a new account on his own phone.
    let page = browser.get(&server.url(&format!("/reins/org/{org}"))).await;
    let invited = browser
        .post(
            &server.url(&format!("/reins/org/{org}/family/invites")),
            &[("csrf", &csrf(&page)), ("email", "sam@lees.example"), ("minor", "true")],
        )
        .await;
    let at = invited.body.find("reins-family://").unwrap_or_else(|| panic!("no link:\n{}", invited.body));
    let link: String = invited.body[at..].chars().take_while(|c| !c.is_whitespace() && *c != '<').collect();
    workos.sign_in_as(&User {
        id: "user_sam".to_owned(),
        email: "sam@lees.example".to_owned(),
    });
    let sam_phone = Phone::signed_out(|_| {}).await;
    sam_phone.sso_sign_in(&server.base).await.unwrap();
    let joined = sam_phone.core.family_join(link.clone()).await.unwrap();
    // Invited as a minor: it counts only once Sam accepts on his phone.
    assert!(!joined.minor && joined.minor_proposed && !joined.guardian);
    sam_phone.core.family_answer_minor(true).await.unwrap();
    assert!(sam_phone.core.family_view().await.unwrap().unwrap().minor);
    assert_eq!(joined.guardians, ["mom@lees.example"]);
    assert!(sam_phone.core.family_join(link).await.is_err(), "an invitation works once");
    sam_phone.core.register_device(None).await.unwrap();
    let sam = enroll(sam_phone).await;
    assert_eq!(member_state(&server, "sam@lees.example").as_deref(), Some("active"));

    // ---- 3. Mom connects Sam's Google account: the browser goes to the provider's sign-in the executor prepared.
    let page = browser.get(&server.url(&format!("/reins/org/{org}"))).await;
    let sam_id: String =
        db(&server).query_row("SELECT uuid FROM users WHERE email = 'sam@lees.example'", [], |r| r.get(0)).unwrap();
    let waiting = browser
        .post(
            &server.url(&format!("/reins/org/{org}/family/connect")),
            &[("csrf", &csrf(&page)), ("member", &sam_id), ("provider", "google")],
        )
        .await;
    let job_url = waiting.url.clone();
    assert!(job_url.contains("/family/connect/"), "{job_url}");
    let deadline = Instant::now() + Duration::from_secs(20);
    let back = loop {
        let p = browser.get(&job_url).await;
        if !p.url.contains("/family/connect/") || Instant::now() > deadline {
            break p;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    };
    assert!(back.body.contains("Connecting"), "{}", back.body);
    let consent = hits(&google, "GET", "/authorize").await;
    let q: std::collections::HashMap<String, String> = consent[0].url.query_pairs().into_owned().collect();
    assert_eq!(q["code_challenge_method"], "S256");
    assert_eq!(q["client_id"], "family-client");
    wait_for("the connection", Duration::from_secs(20), || server_texts(&server).contains("sam.kid@gmail.com")).await;
    // The server never held a token, and the code is gone once the executor took it.
    let texts = server_texts(&server);
    for secret in ["1//sam", "ya29.", "CODE-SAM", "family-secret"] {
        assert!(!texts.contains(secret), "{secret} reached the server");
    }
    let exchange = hits(&google, "POST", "/token").await;
    let form = String::from_utf8_lossy(&exchange[0].body).into_owned();
    assert!(form.contains("code=CODE-SAM") && form.contains("code_verifier="), "{form}");
    assert_eq!(tokens.get(reins_proto::managed::OAuthProvider::Google, "sam@lees.example").unwrap().token, "1//sam");

    // ---- 4. Sam's agent emails an Outside address: Sam and a guardian approve; the executor sends it as Sam.
    let mut ai = AiClient::new(&server.base);
    ai.register_client().await;
    let wait_url = ai.start_authorization("sam@lees.example").await;
    let code = ai.browser_code(&wait_url).await;
    let item = sam.phone.wait_for_item(Duration::from_secs(20)).await;
    sam.phone.core.answer_pairing(item.id, true, Some(code), Some("Sam's agent".to_owned()), true).await.unwrap();
    ai.finish_authorization(&wait_url).await;
    let r =
        ai.tool("gmail_send", &json!({"to": ["stranger@elsewhere.example"], "subject": "Hi", "body": "Hello"})).await;
    let ticket = r["structuredContent"]["ticket"]
        .as_str()
        .or_else(|| r["structuredContent"]["request_id"].as_str())
        .unwrap_or_else(|| panic!("no ticket: {r}"))
        .to_owned();
    assert!(hits(&google, "POST", "/gmail/v1/users/me/messages/send").await.is_empty(), "nothing before approval");
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        sam.answer(true, |v| v.id == ticket).await;
        mom.answer(true, |v| v.id == ticket).await;
        let r = ai.tool("reins_get_result", &json!({"request_id": ticket})).await;
        if r["structuredContent"]["id"] == "m1" {
            break;
        }
        assert!(Instant::now() < deadline, "not sent: {r}");
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    let sent = hits(&google, "POST", "/gmail/v1/users/me/messages/send").await;
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].headers.get("authorization").unwrap().to_str().unwrap(), "Bearer ya29.sam");

    // ---- 5. Sam asks to leave: it waits for a guardian; Mom allows it; Sam goes within a minute.
    let leave = sam.phone.core.family_request_leave().await.unwrap();
    assert!(leave.needs_consent && leave.effective_at.is_none());
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(member_state(&server, "sam@lees.example").as_deref(), Some("active"), "a minor waits for consent");
    let view = mom.phone.core.family_view().await.unwrap().unwrap();
    let ask = view.leaves.iter().find(|l| l.email == "sam@lees.example").expect("Mom sees the request");
    let started = Instant::now();
    mom.phone.core.family_consent_leave(ask.member.clone()).await.unwrap();
    wait_for("Sam leaves", Duration::from_secs(60), || {
        member_state(&server, "sam@lees.example").as_deref() == Some("left_family")
    })
    .await;
    assert!(started.elapsed() < Duration::from_secs(60));
    let deadline = Instant::now() + Duration::from_secs(20);
    while hits(&google, "POST", "/revoke").await.is_empty() {
        assert!(Instant::now() < deadline, "the Google grant was not revoked upstream");
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    assert!(tokens.get(reins_proto::managed::OAuthProvider::Google, "sam@lees.example").is_none());
    let kinds: Vec<String> = {
        let c = db(&server);
        let mut st = c.prepare("SELECT kind FROM reins_org_events WHERE org_uuid = ?1 ORDER BY seq").unwrap();
        st.query_map([&org], |r| r.get::<_, String>(0)).unwrap().map(Result::unwrap).collect()
    };
    for k in ["family.invited", "family.leave_requested", "family.leave_consented", "member.deprovisioned"] {
        assert!(kinds.iter().any(|x| x == k), "{k} missing: {kinds:?}");
    }
    runner.abort();
}
