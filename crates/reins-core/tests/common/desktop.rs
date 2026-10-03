//! A phone paired with a fake Reins desktop app: a fake Reins server that relays calls, the app's key pair, and
//! helpers to pair, send calls, read the phone's answers and open what it sealed.

use std::fmt::Write as _;
use std::sync::Arc;
use std::time::Duration;

use crypto_box::SecretKey;
use crypto_box::aead::OsRng;
use data_encoding::BASE64URL_NOPAD;
use reins_core::{ApprovalChoice, CoreConfig, GrantScopeChoice, ReinsCore, StandingGrant};
use reins_proto::desktop::encode_key;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use wiremock::matchers::{method, path, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::{FakeGoogle, FakeKeys, RecordingNotifier};

/// The connection of the paired desktop app.
pub const DESK: &str = "desk1";
/// What the phone answers a desktop-only call from anything but the paired app.
pub const REFUSED: &str = "This must come from the Reins desktop app paired with this phone.";

pub struct Desk {
    pub server: MockServer,
    pub core: Arc<ReinsCore>,
    pub key: SecretKey,
    pub dir: tempfile::TempDir,
}

/// The fake Reins server (sign-in, answers), mounted on `server`.
pub async fn mount_reins(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/identity/accounts/prelogin"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"kdf": 0, "kdfIterations": 5000})))
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path("/identity/connect/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "ACCESS", "refresh_token": "REFRESH", "expires_in": 7200})))
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path_regex(r"^/reins/api/(requests|pairings)/[^/]+/response$"))
        .respond_with(ResponseTemplate::new(204))
        .mount(server)
        .await;
}

/// A signed-in phone (as `email` with `password`) whose server is `server`, configured by `cfg`.
pub async fn desk_with(server: MockServer, email: &str, password: &str, cfg: CoreConfig) -> Desk {
    let dir = tempfile::tempdir().unwrap();
    let core = ReinsCore::with_connectors(
        dir.path().to_str().unwrap(),
        &FakeKeys,
        Arc::new(FakeGoogle::new()),
        Arc::new(RecordingNotifier::default()),
        CoreConfig {
            backoff_base: Duration::from_millis(1),
            ..cfg
        },
        vec![],
    )
    .unwrap();
    core.login(server.uri(), email.to_owned(), password.to_owned(), None).await.unwrap();
    Desk {
        server,
        core,
        key: SecretKey::generate(&mut OsRng),
        dir,
    }
}

impl Desk {
    /// The app's public key as it sends it.
    pub fn public(&self) -> String {
        encode_key(self.key.public_key().as_bytes())
    }

    /// The server hands the phone these requests and pairings once.
    pub async fn serve(&self, requests: &[Value], pairings: &[Value]) {
        Mock::given(method("GET"))
            .and(path("/reins/api/pending"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"requests": requests, "pairings": pairings})))
            .up_to_n_times(1)
            .with_priority(1)
            .mount(&self.server)
            .await;
        Mock::given(method("GET"))
            .and(path("/reins/api/pending"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"requests": [], "pairings": []})))
            .mount(&self.server)
            .await;
    }

    /// Sends requests to the phone and lets it handle them.
    pub async fn send(&self, requests: &[Value]) {
        self.serve(requests, &[]).await;
        self.core.sync(0).await.unwrap();
    }

    /// The desktop app is paired with its key on connection [`DESK`].
    pub async fn pair(&self) {
        let pairing = json!({"v": 1, "id": "p1", "client_name": "Reins desktop app", "client_host": "127.0.0.1",
            "choices": [12, 47, 83], "created_at": 50, "client_key": self.public()});
        self.serve(&[], &[pairing]).await;
        self.core.sync(0).await.unwrap();
        Mock::given(method("POST"))
            .and(path("/reins/api/pairings/p1/response"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"connection_id": DESK})))
            .with_priority(1)
            .mount(&self.server)
            .await;
        self.core.answer_pairing("p1".to_owned(), true, Some(47), None).await.unwrap();
    }

    /// What the phone answered, by request id.
    pub async fn answer(&self, id: &str) -> Option<Value> {
        self.server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .rev()
            .find(|r| r.method.as_str() == "POST" && r.url.path() == format!("/reins/api/requests/{id}/response"))
            .map(|r| serde_json::from_slice(&r.body).unwrap())
    }

    /// The answer's result data (it must be a result).
    pub async fn data(&self, id: &str) -> Value {
        let answer = self.answer(id).await.unwrap_or_else(|| panic!("{id} was not answered"));
        assert_eq!(answer["outcome"], "result", "{id}: {answer}");
        answer["result"]["data"].clone()
    }

    /// The error the phone answered (it must be an error).
    pub async fn error(&self, id: &str) -> String {
        let answer = self.answer(id).await.unwrap_or_else(|| panic!("{id} was not answered"));
        assert_eq!(answer["outcome"], "error", "{id}: {answer}");
        answer["message"].as_str().unwrap().to_owned()
    }

    /// The ids of the requests waiting for the user.
    pub async fn waiting(&self) -> Vec<String> {
        self.core.pending().await.unwrap().into_iter().map(|p| p.id).collect()
    }

    /// Opens something sealed to the app's key.
    pub fn open<T: DeserializeOwned>(&self, sealed: &Value) -> T {
        let bytes = BASE64URL_NOPAD.decode(sealed.as_str().expect("sealed is a string").as_bytes()).unwrap();
        serde_json::from_slice(&self.key.unseal(&bytes).expect("sealed to the app's key")).unwrap()
    }

    /// Everything the phone sent to the server and everything the app can show, as one text.
    pub async fn everything_shown(&self, ids: &[&str]) -> String {
        let mut shown = format!(
            "{:?} {:?} {:?}",
            self.core.pending().await.unwrap(),
            self.core.activity(200).await.unwrap(),
            self.core.grants().await.unwrap()
        );
        for id in ids {
            if let Ok(view) = self.core.approval_view((*id).to_owned()).await {
                write!(shown, " {view:?}").unwrap();
            }
        }
        for r in self.server.received_requests().await.unwrap() {
            shown.push_str(&String::from_utf8_lossy(&r.body));
        }
        shown
    }
}

/// A desktop-only call from connection `connection`.
pub fn call(id: &str, connection: &str, service: &str, op: &str, args: &Value) -> Value {
    json!({"v": 1, "id": id, "connection_id": connection, "connection_label": "Reins desktop app",
           "created_at": 100, "call": {"tool": "connector", "service": service, "op": op, "args": args}})
}

pub fn standing(resources: &[&str], classes: &[&str]) -> Option<StandingGrant> {
    Some(StandingGrant {
        duration_secs: Some(3_600),
        max_uses: None,
        scope: GrantScopeChoice {
            all_mail: false,
            selected_messages_only: false,
            sender_addresses: vec![],
            sender_domains: vec![],
            subject_pattern: None,
            recipient_addresses: vec![],
            recipient_domains: vec![],
            resources: resources.iter().map(|r| (*r).to_owned()).collect(),
            classes: classes.iter().map(|c| (*c).to_owned()).collect(),
        },
    })
}

pub fn choice(selected: &[&str], standing: Option<StandingGrant>) -> ApprovalChoice {
    ApprovalChoice {
        selected_message_ids: selected.iter().map(|s| (*s).to_owned()).collect(),
        standing,
    }
}
