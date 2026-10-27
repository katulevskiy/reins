//! A Personal profile and a Managed work profile on one device (Decisions v2 · O1, 06 §2, 08 R6): each runs its own
//! core on its own data directory, signed in to its own account. Nothing of the Personal profile reaches an org, and
//! nothing crosses between the two stores.
mod common;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use data_encoding::BASE64URL_NOPAD;
use reins_core::crypto::{Kdf, master_key};
use reins_core::org::{AttestationInput, AttestationKindInput, EnrollInput, PostureInput};
use reins_core::profiles::{ProfileKind, ProfileRegistry};
use reins_core::{CoreConfig, CoreError, GoogleTokenProvider, Notifier, ReinsCore};
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const PERSONAL: &str = "dana@gmail.com";
const WORK: &str = "dana@acme.com";

fn token(email: &str) -> String {
    format!("h.{}.s", BASE64URL_NOPAD.encode(json!({"sub": email, "email": email}).to_string().as_bytes()))
}

fn owner(request: &Request) -> String {
    let auth = request.headers.get("authorization").map(|h| h.to_str().unwrap_or_default()).unwrap_or_default();
    let Some(payload) = auth.strip_prefix("Bearer ").and_then(|t| t.split('.').nth(1)) else {
        return String::new();
    };
    serde_json::from_slice::<Value>(&BASE64URL_NOPAD.decode(payload.as_bytes()).unwrap()).unwrap()["sub"]
        .as_str()
        .unwrap()
        .to_owned()
}

struct Identity;
impl Respond for Identity {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let form = url::form_urlencoded::parse(&request.body).into_owned().collect::<HashMap<_, _>>();
        ResponseTemplate::new(200)
            .set_body_json(json!({"access_token": token(&form["username"]), "refresh_token": "R", "expires_in": 7200}))
    }
}

struct VaultProfile;
impl Respond for VaultProfile {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let email = owner(request);
        let key = master_key(
            "pw",
            &email,
            Kdf::Pbkdf2 {
                iterations: 5000,
            },
        )
        .unwrap()
        .stretch()
        .encrypt(&[7; 64])
        .unwrap();
        ResponseTemplate::new(200).set_body_json(json!({"profile": {"id": email, "key": key}}))
    }
}

#[derive(Clone, Default)]
struct Cloud(Arc<Mutex<HashMap<String, Value>>>);
impl Respond for Cloud {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let mut states = self.0.lock().unwrap();
        let state = states.entry(owner(request)).or_insert_with(|| json!({"revision": 0, "ciphertext": null}));
        if request.method.as_str() == "PUT" {
            let update: Value = serde_json::from_slice(&request.body).unwrap();
            *state = json!({"revision": state["revision"].as_u64().unwrap() + 1, "ciphertext": update["ciphertext"]});
        }
        ResponseTemplate::new(200).set_body_json(state.clone())
    }
}

/// (account, path, body) of every org request.
type Seen = Vec<(String, String, Vec<u8>)>;

/// The org endpoints as the server answers them: only the work account is a member; everyone else gets 404.
#[derive(Clone, Default)]
struct Org(Arc<Mutex<Seen>>);
impl Respond for Org {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let who = owner(request);
        self.0.lock().unwrap().push((who.clone(), request.url.path().to_owned(), request.body.clone()));
        if who != WORK {
            return ResponseTemplate::new(404).set_body_json(json!({"error": "not_found", "message": "Unknown"}));
        }
        match request.url.path() {
            "/reins/api/org" => ResponseTemplate::new(200).set_body_json(json!({
                "org_id": "o1", "name": "Acme", "tier": "business", "identity": "sso", "domains": ["acme.com"],
                "roles": ["approver"], "enrolled": false, "routes": {"github_orgs": ["acme"], "hosts": []}})),
            "/reins/api/org/enroll/challenge" => ResponseTemplate::new(200)
                .set_body_json(json!({"challenge": "challenge-1", "expires_at": 9_999_999_999i64})),
            _ => ResponseTemplate::new(201),
        }
    }
}

async fn server() -> (MockServer, Org) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/identity/accounts/prelogin"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"kdf": 0, "kdfIterations": 5000})))
        .mount(&server)
        .await;
    Mock::given(method("POST")).and(path("/identity/connect/token")).respond_with(Identity).mount(&server).await;
    Mock::given(method("GET")).and(path("/api/sync")).respond_with(VaultProfile).mount(&server).await;
    Mock::given(path("/reins/api/account-state")).respond_with(Cloud::default()).mount(&server).await;
    let org = Org::default();
    for p in ["/reins/api/org", "/reins/api/org/enroll/challenge", "/reins/api/org/devices", "/reins/api/org/posture"] {
        Mock::given(path(p)).respond_with(org.clone()).mount(&server).await;
    }
    (server, org)
}

fn core(dir: &str) -> Arc<ReinsCore> {
    let google: Arc<dyn GoogleTokenProvider> = Arc::new(common::FakeGoogle::new());
    let notifier: Arc<dyn Notifier> = Arc::new(common::RecordingNotifier::default());
    ReinsCore::with_config(dir, &common::FakeKeys, google, notifier, CoreConfig::default()).unwrap()
}

/// The device's two profiles, each signed in on its own core at the registry's paths.
async fn two_profiles(
    server: &MockServer,
) -> (tempfile::TempDir, Arc<ProfileRegistry>, Arc<ReinsCore>, Arc<ReinsCore>) {
    let base = tempfile::tempdir().unwrap();
    let registry = ProfileRegistry::new(base.path().to_string_lossy().into_owned());
    registry.note_personal(Some(PERSONAL.to_owned())).unwrap();
    let personal = core(&registry.active().data_dir);
    personal.login(server.uri(), PERSONAL.to_owned(), "pw".to_owned(), None).await.unwrap();
    // "Add work account": a reserved profile, the work account signed in on its own core, then confirmed.
    let work_profile = registry.begin_managed(server.uri()).unwrap();
    std::fs::create_dir_all(&work_profile.data_dir).unwrap();
    let work = core(&work_profile.data_dir);
    work.login(server.uri(), WORK.to_owned(), "pw".to_owned(), None).await.unwrap();
    let org = work.org_membership().await.unwrap().expect("a work account");
    registry.confirm_managed(work_profile.id, org.org_id, org.name, WORK.to_owned()).unwrap();
    (base, registry, personal, work)
}

fn enroll_input() -> EnrollInput {
    EnrollInput {
        device_key: "MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAE".to_owned(),
        signature: "MEUCIQ".to_owned(),
        hardware: true,
        attestations: vec![AttestationInput {
            kind: AttestationKindInput::AppAttest,
            data: "o2NmbXQ".to_owned(),
        }],
        posture: PostureInput {
            screen_lock: true,
            os_name: "iOS".to_owned(),
            os_version: "26.1".to_owned(),
            rooted: Some(false),
            app_version: "1.4.0".to_owned(),
        },
    }
}

#[tokio::test]
async fn a_personal_core_never_reaches_an_org() {
    let (server, org) = server().await;
    let (_base, _registry, personal, work) = two_profiles(&server).await;
    assert_eq!(personal.org_membership().await.unwrap(), None);
    assert_eq!(personal.org_challenge().await.unwrap_err(), CoreError::NotFound);
    let view = work.org_membership().await.unwrap().expect("the work account's org");
    assert_eq!((view.name.as_str(), view.tier.as_str()), ("Acme", "business"));
    assert_eq!(view.roles, ["Approver"]);
    // The Personal core asked as the Personal account and nothing else; the org only ever heard the work account.
    let seen = org.0.lock().unwrap().clone();
    assert!(seen.iter().filter(|(who, _, _)| who == PERSONAL).all(|(_, p, _)| p.starts_with("/reins/api/org")));
    assert!(seen.iter().filter(|(who, _, _)| who != PERSONAL).all(|(who, _, _)| who == WORK));
}

#[tokio::test]
async fn the_enrollment_carries_only_the_work_account() {
    let (server, org) = server().await;
    let (base, _registry, _personal, work) = two_profiles(&server).await;
    let challenge = work.org_challenge().await.unwrap();
    assert_eq!(challenge, "challenge-1");
    work.org_enroll(challenge, enroll_input()).await.unwrap();
    let seen = org.0.lock().unwrap().clone();
    let (who, _, body) = seen.iter().find(|(_, p, _)| p == "/reins/api/org/devices").expect("an enrollment").clone();
    assert_eq!(who, WORK);
    let text = String::from_utf8(body).unwrap();
    for personal in [PERSONAL, "gmail", &base.path().to_string_lossy()] {
        assert!(!text.contains(personal), "the enrollment mentions {personal}: {text}");
    }
    let json: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(json["challenge"], "challenge-1");
    assert_eq!(json["posture"]["os_name"], "iOS");
}

#[tokio::test]
async fn activity_and_accounts_never_cross_profiles() {
    let (server, _org) = server().await;
    let (_base, _registry, personal, work) = two_profiles(&server).await;
    personal.engine().register_account("github", "dana-personal").unwrap();
    work.engine().register_account("github", "dana-acme").unwrap();
    let personal_accounts = personal.accounts().await.unwrap();
    let work_accounts = work.accounts().await.unwrap();
    assert!(personal_accounts.iter().any(|a| a.account == "dana-personal"));
    assert!(!personal_accounts.iter().any(|a| a.account == "dana-acme"));
    assert!(work_accounts.iter().any(|a| a.account == "dana-acme"));
    assert!(!work_accounts.iter().any(|a| a.account == "dana-personal"));
    assert_eq!(personal.session().await.unwrap().unwrap().email, PERSONAL);
    assert_eq!(work.session().await.unwrap().unwrap().email, WORK);
}

#[tokio::test]
async fn removing_the_work_profile_leaves_the_personal_one_whole() {
    let (server, _org) = server().await;
    let (_base, registry, personal, work) = two_profiles(&server).await;
    personal.engine().register_account("github", "dana-personal").unwrap();
    let profiles = registry.list();
    assert_eq!(profiles.iter().map(|p| p.kind).collect::<Vec<_>>(), [ProfileKind::Personal, ProfileKind::Managed]);
    let work_dir = profiles[1].data_dir.clone();
    drop(work);
    registry.remove(profiles[1].id.clone()).unwrap();
    assert!(!std::path::Path::new(&work_dir).exists());
    assert!(personal.accounts().await.unwrap().iter().any(|a| a.account == "dana-personal"));
    assert_eq!(registry.list().len(), 1);
}
