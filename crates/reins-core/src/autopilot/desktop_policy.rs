//! The owner's Auto policy, signed for each paired computer (`docs/design/decisions-v2/08-review-resolutions.md` R1):
//! the computer's daemon decides hook-level requests (shell commands, file tools) with it, without a round trip.
//!
//! For each paired computer with a pinned key the phone builds the body ([`Engine::ap_desktop_policy`]: the matrix,
//! the owner's rules, the connection's own tasks, the facts a computer needs, the mode in force for the connection and
//! when it ends), compares it with what it last sent there, and when it changed (or an hour went by) signs it with the
//! device key, seals it to the computer's key and posts it. When the new body is stricter in a way the computer must
//! hear of at once ([`tighten_of`]) a "tighten now" goes first. Versions only grow. Nothing is sent when nothing
//! changed; a failure is tried again with the next sync, pass or change. Never a notification.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::Ordering;

use reins_policy::desktop_policy::{
    self as signed, Binding, DesktopPolicy, MAX_BODY_BYTES, ModeState, PERSONAL, PolicyMessage, REFRESH_SECS,
    TIGHTEN_SECS, Tighten, Tightening, VALIDITY_SECS, desktop_facts, desktop_rules, desktop_tasks, tighten_of,
};
use reins_policy::matrix::Snapshot;
use reins_proto::desktop::{
    MAX_POLICY_CHARS, MAX_POLICY_ITEMS, MAX_TIGHTEN_CHARS, PolicyKind, PolicyPost, SealedPolicy, decode_key,
};
use serde::{Deserialize, Serialize};

use super::modes::{self, BYPASS_UNTIL_OFF, ModeRow};
use super::types::AutopilotMode;
use crate::CoreError;
use crate::connector::sealed::seal;
use crate::engine::Engine;
use crate::session::{Session, api_call};

/// What was last sent to each computer (sealed with the data key: the bodies name the owner's repositories).
const SENT_KEY: &str = "autopilot.desktop_policy.sent";

/// What the phone last sent, per connection, and the highest version it used.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Sent {
    /// The highest version used for any message (reserved before it is sent).
    #[serde(default)]
    last: u64,
    #[serde(default)]
    to: BTreeMap<String, SentTo>,
}

/// The snapshot a computer was last sent.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct SentTo {
    /// The body with its binding's version and times zeroed (what the next body is compared with).
    body: DesktopPolicy,
    version: u64,
    /// Unix seconds.
    at: i64,
}

/// One computer's messages for a post, and what to keep once the post went through.
struct Outgoing {
    items: Vec<SealedPolicy>,
    connection_id: String,
    body: DesktopPolicy,
    version: u64,
}

/// What a publish sends: each computer's messages, and the connections paired now.
type Outbox = (Vec<Outgoing>, Vec<String>);

/// A computer whose policy is due.
struct Due {
    connection_id: String,
    key: [u8; 32],
    body: DesktopPolicy,
    tightening: Option<Tightening>,
}

fn unix_now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

impl Engine {
    /// The policy body for the computer on `connection_id` (pinned key `client_key`) at `now`, with its binding's
    /// version and times left at 0 (set when it is sent).
    pub(crate) fn ap_desktop_policy(
        &self,
        connection_id: &str,
        client_key: &str,
        now: i64,
    ) -> Result<DesktopPolicy, CoreError> {
        let mut snap = self.ap_snapshot()?;
        self.ap_tasks_into(&mut snap, now);
        let (facts, github_ids) = desktop_facts(&snap.facts);
        // The standard roots, and the folders the owner confirmed for this computer (`desktop_folders`).
        let (roots, scratch) = self.ap_signed_folders(connection_id)?;
        // A computer decides commands and files, never messages: the people a task may reach stay on the phone. Without
        // them a task covers no message, so this only narrows, except where people were its only resource bound (an
        // empty envelope covers any resource): such a task isn't sent at all.
        let tasks = desktop_tasks(&snap.tasks, connection_id)
            .into_iter()
            .filter(|t| t.people.is_empty() || !t.resources.is_empty())
            .map(|mut t| {
                t.people.clear();
                t
            })
            .collect();
        Ok(DesktopPolicy {
            binding: Binding {
                version: 0,
                issued_at: 0,
                not_after: 0,
                client_key: client_key.to_owned(),
                connection_id: connection_id.to_owned(),
                profile: self.ap_desktop_profile(),
            },
            decides: self.ap_matrix_deciding(),
            mode: self.ap_mode_state(connection_id, now)?,
            snapshot: Snapshot {
                version: snap.version,
                matrix: snap.matrix,
                rules: desktop_rules(&snap.rules, connection_id),
                tasks,
                facts,
            },
            github_ids,
            roots,
            scratch,
        })
    }

    /// The mode in force for `connection_id` at `now`, and when it ends: the end of the running bypass or Overnight
    /// (the connection's own, else the global one it is under); `None` when it runs until changed.
    fn ap_mode_state(&self, connection_id: &str, now: i64) -> Result<ModeState, CoreError> {
        let global = self.store.ap_mode_row("")?;
        let own = self.store.ap_mode_row(connection_id)?;
        let mode = modes::effective(&global, Some(&own), now);
        // `Some(end)` when `row` runs `mode` now (`end` none: until turned off).
        let running = |row: &ModeRow| match mode {
            AutopilotMode::Bypass if row.bypassing(now) => Some(row.bypass_until),
            AutopilotMode::Overnight if row.overnighting(now) => Some(row.overnight_until),
            _ => None,
        };
        let until = running(&own).or_else(|| running(&global)).flatten().filter(|t| *t != BYPASS_UNTIL_OFF);
        Ok(ModeState {
            mode: mode.wire(),
            until,
        })
    }

    /// The profile this core runs as: [`PERSONAL`], or `org:<id>` for a Managed work profile (each runs its own core
    /// in `<base>/profiles/<profile>/core`, `crate::profiles`).
    fn ap_desktop_profile(&self) -> String {
        let dir = self.data_dir.as_path();
        let profile_dir = dir
            .parent()
            .filter(|_| dir.file_name().is_some_and(|n| n == "core"))
            .filter(|p| p.parent().and_then(Path::file_name).is_some_and(|n| n == "profiles"));
        let Some(id) = profile_dir.and_then(Path::file_name).and_then(|n| n.to_str()) else {
            return PERSONAL.to_owned();
        };
        let org = profile_dir
            .and_then(Path::parent)
            .and_then(Path::parent)
            .and_then(|base| {
                crate::profiles::ProfileRegistry::open(base)
                    .list()
                    .into_iter()
                    .find(|p| p.id == id)
                    .and_then(|p| p.org_id)
            })
            .filter(|o| !o.is_empty());
        format!("org:{}", org.as_deref().unwrap_or(id))
    }

    /// Sends each paired computer the owner's policy when it changed since it was last sent there, or an hour went by
    /// ([`REFRESH_SECS`]): a "tighten now" first when the change tightens. Only the phone that decides sends. A failure
    /// (offline, an older server) is logged and tried again on the next call.
    pub(crate) async fn publish_desktop_policy(&self, session: &Session) {
        self.publish_desktop_policy_at(session, unix_now_ms()).await;
    }

    /// [`Engine::publish_desktop_policy`] at `now_ms` (Unix milliseconds).
    pub(crate) async fn publish_desktop_policy_at(&self, session: &Session, now_ms: u64) {
        if self.require_decider().is_err() || self.not_approval.load(Ordering::Acquire) {
            return;
        }
        let _one = self.autopilot.publish.lock().await;
        let now = i64::try_from(now_ms / 1000).unwrap_or(i64::MAX);
        let (out, desktops) = match self.ap_desktop_outgoing(now_ms, now) {
            Ok(Some(o)) => o,
            Ok(None) => return,
            Err(e) => {
                log::warn!("could not prepare the policy for the computers: {e}");
                return;
            }
        };
        let (mut tightens, snapshots): (Vec<SealedPolicy>, Vec<SealedPolicy>) =
            out.iter().flat_map(|o| o.items.iter().cloned()).partition(|i| i.kind == PolicyKind::Tighten);
        tightens.extend(snapshots);
        let post = PolicyPost {
            items: tightens,
        };
        match api_call!(session, |api| api.post_desktop_policy(&post)) {
            Ok(()) => {
                if let Err(e) = self.ap_desktop_sent(out, &desktops, now) {
                    log::warn!("could not keep what was sent to the computers: {e}");
                }
            }
            Err(e) => {
                self.refused_as_approval(&e);
                log::info!("could not send the policy to the computers: {}", e.into_core());
            }
        }
    }

    /// [`Engine::publish_desktop_policy`] when the phone is signed in (right after the owner's policy changed).
    pub(crate) async fn publish_desktop_policy_now(&self) {
        if let Ok(session) = self.session() {
            self.publish_desktop_policy(&session).await;
        }
    }

    /// `result`, once the computers heard of the change it made (when it made one).
    pub(crate) async fn and_publish<T>(&self, result: Result<T, CoreError>) -> Result<T, CoreError> {
        if result.is_ok() {
            self.publish_desktop_policy_now().await;
        }
        result
    }

    /// What is due to be sent, signed and sealed, with the paired connections; `None` when nothing is due.
    fn ap_desktop_outgoing(&self, now_ms: u64, now: i64) -> Result<Option<Outbox>, CoreError> {
        let mut desktops = self.store.desktop_connections()?;
        if desktops.is_empty() {
            return Ok(None);
        }
        // Two messages at most per computer in one post.
        desktops.truncate(MAX_POLICY_ITEMS / 2);
        let sent = self.ap_desktop_sent_state();
        let mut due = Vec::new();
        for connection_id in &desktops {
            let Some(client_key) = self.store.desktop_key(connection_id)? else {
                continue;
            };
            let Some(key) = decode_key(&client_key) else {
                continue;
            };
            let body = self.ap_desktop_policy(connection_id, &client_key, now)?;
            let last = sent.to.get(connection_id);
            if last.is_some_and(|l| l.body == body && now - l.at < REFRESH_SECS) {
                continue;
            }
            due.push(Due {
                tightening: last.and_then(|l| tighten_of(&l.body, &body)),
                connection_id: connection_id.clone(),
                key,
                body,
            });
        }
        if due.is_empty() {
            return Ok(None);
        }
        let needed = due
            .iter()
            .map(|d| {
                if d.tightening.is_some() {
                    2
                } else {
                    1
                }
            })
            .sum();
        let mut version = self.ap_reserve_versions(needed, now_ms)?;
        let signer = self.sign_key()?;
        let public = signer.public_b64();
        let mut out = Vec::new();
        for d in due {
            let mut messages = Vec::new();
            if let Some(tightening) = d.tightening {
                messages.push(PolicyMessage::Tighten(Tighten {
                    binding: Binding {
                        version,
                        issued_at: now,
                        not_after: now.saturating_add(TIGHTEN_SECS),
                        ..d.body.binding.clone()
                    },
                    tightening,
                }));
                version += 1;
            }
            let mut body = d.body.clone();
            body.binding.version = version;
            body.binding.issued_at = now;
            body.binding.not_after = now.saturating_add(VALIDITY_SECS);
            messages.push(PolicyMessage::Snapshot(Box::new(body)));
            let snapshot_version = version;
            version += 1;
            let seal_one = |m: &PolicyMessage| -> Option<SealedPolicy> {
                let s = signed::sign(m, &public, &|b| signer.sign_b64(b));
                if s.body.len() > MAX_BODY_BYTES {
                    return None;
                }
                let limit = match m {
                    PolicyMessage::Snapshot(_) => MAX_POLICY_CHARS,
                    PolicyMessage::Tighten(_) => MAX_TIGHTEN_CHARS,
                };
                let sealed = seal(d.key, &s).ok().filter(|x| x.len() <= limit)?;
                Some(SealedPolicy {
                    connection_id: d.connection_id.clone(),
                    version: m.version(),
                    kind: match m {
                        PolicyMessage::Snapshot(_) => PolicyKind::Snapshot,
                        PolicyMessage::Tighten(_) => PolicyKind::Tighten,
                    },
                    sealed,
                })
            };
            let items: Option<Vec<SealedPolicy>> = messages
                .iter()
                .map(|m| {
                    seal_one(m).or_else(|| match m {
                        // A tightening too large to relay (many new rules): the computer stops deciding (and keeps a
                        // Lockdown) until the snapshot, which carries the rules, arrives.
                        PolicyMessage::Tighten(t) => seal_one(&PolicyMessage::Tighten(Tighten {
                            binding: t.binding.clone(),
                            tightening: Tightening {
                                lockdown: t.tightening.lockdown,
                                stop_auto: true,
                                ..Tightening::default()
                            },
                        })),
                        PolicyMessage::Snapshot(_) => None,
                    })
                })
                .collect();
            let Some(items) = items else {
                log::warn!("the policy could not be sealed for a computer");
                continue;
            };
            out.push(Outgoing {
                items,
                connection_id: d.connection_id,
                body: d.body,
                version: snapshot_version,
            });
        }
        Ok((!out.is_empty()).then_some((out, desktops)))
    }

    /// What was last sent; when that can't be read, it starts over (each computer is sent its policy again, with
    /// versions from the clock, so still growing).
    fn ap_desktop_sent_state(&self) -> Sent {
        let read = self.store.meta_get_sealed(SENT_KEY).and_then(|b| {
            b.map(|b| serde_json::from_slice(&b).map_err(|e| CoreError::storage(e.to_string()))).transpose()
        });
        match read {
            Ok(sent) => sent.unwrap_or_default(),
            Err(e) => {
                log::warn!("what was sent to the computers could not be read, so it is sent again: {e}");
                if let Err(e) = self.store.meta_set_sealed(SENT_KEY, b"{}") {
                    log::warn!("could not start over: {e}");
                }
                Sent::default()
            }
        }
    }

    /// Reserves `n` versions, the first above every one used and at least `now_ms`: the first is returned.
    fn ap_reserve_versions(&self, n: u64, now_ms: u64) -> Result<u64, CoreError> {
        self.store.meta_update_sealed(SENT_KEY, |current| {
            let mut sent: Sent = current.and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
            let first = sent.last.saturating_add(1).max(now_ms);
            sent.last = first.saturating_add(n.saturating_sub(1));
            let json = serde_json::to_vec(&sent).map_err(|e| CoreError::storage(e.to_string()))?;
            Ok((Some(json), first))
        })
    }

    /// Keeps what the server took, and forgets computers no longer paired.
    fn ap_desktop_sent(&self, out: Vec<Outgoing>, desktops: &[String], now: i64) -> Result<(), CoreError> {
        self.store.meta_update_sealed(SENT_KEY, |current| {
            let mut sent: Sent = current.and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
            for o in out {
                sent.last = sent.last.max(o.version);
                if sent.to.get(&o.connection_id).is_none_or(|l| l.version < o.version) {
                    sent.to.insert(
                        o.connection_id,
                        SentTo {
                            body: o.body,
                            version: o.version,
                            at: now,
                        },
                    );
                }
            }
            sent.to.retain(|c, _| desktops.contains(c));
            let json = serde_json::to_vec(&sent).map_err(|e| CoreError::storage(e.to_string()))?;
            Ok((Some(json), ()))
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use reins_policy::context::{FactKind, Zone};
    use reins_policy::desktop_policy::{Signed, open};
    use reins_policy::effects::Level;
    use reins_policy::matrix::Cell;
    use reins_proto::autopilot::Mode;
    use serde_json::{Value, json};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::api::ReinsCore;
    use crate::engine::CoreConfig;
    use crate::store::tests::FakeKeys;
    use crate::store::unix_now;
    use crate::{ForeignError, GoogleTokenProvider, Notifier, PendingItem};

    const DESK: &str = "desk1";
    const OTHER: &str = "desk2";
    const POLICY: &str = "/reins/api/desktop/policy";

    struct Quiet;

    impl Notifier for Quiet {
        fn item_pending(&self, _: PendingItem) {}
        fn item_resolved(&self, _: String) {}
        fn auto_decided(&self, _: crate::autopilot::AutoDecisionView) {}
        fn autopilot_changed(&self, _: crate::autopilot::AutopilotEvent) {}
        fn circle_changed(&self, _: crate::circles::CircleEvent) {}
    }

    struct NoGoogle;

    #[async_trait::async_trait]
    impl GoogleTokenProvider for NoGoogle {
        async fn access_token(&self, _: String, _: String) -> Result<String, ForeignError> {
            Err(ForeignError::Failed {
                reason: "no Google here".to_owned(),
            })
        }
    }

    struct Env {
        server: MockServer,
        core: Arc<ReinsCore>,
        /// The paired computer's secret key.
        computer: crypto_box::SecretKey,
        _dir: tempfile::TempDir,
    }

    async fn mount(server: &MockServer, verb: &str, at: &str, status: u16, body: Value) {
        Mock::given(method(verb))
            .and(path(at))
            .respond_with(ResponseTemplate::new(status).set_body_json(body))
            .mount(server)
            .await;
    }

    /// A signed-in phone with a computer paired on [`DESK`].
    async fn env() -> Env {
        let server = MockServer::builder().start().await;
        mount(&server, "POST", "/identity/accounts/prelogin", 200, json!({"kdf": 0, "kdfIterations": 5000})).await;
        let token = format!(
            "h.{}.s",
            data_encoding::BASE64URL_NOPAD.encode(br#"{"sub":"account-1","email":"me@example.com"}"#)
        );
        mount(
            &server,
            "POST",
            "/identity/connect/token",
            200,
            json!({"access_token": token, "refresh_token": "REFRESH", "expires_in": 7200}),
        )
        .await;
        let master = crate::crypto::master_key(
            "pw",
            "me@example.com",
            crate::crypto::Kdf::Pbkdf2 {
                iterations: 5000,
            },
        )
        .unwrap();
        let key = master.stretch().encrypt(&[7; 64]).unwrap();
        mount(&server, "GET", "/api/sync", 200, json!({"profile": {"key": key}})).await;
        Mock::given(method("PUT"))
            .and(path("/reins/api/modes"))
            .respond_with(ResponseTemplate::new(204))
            .mount(&server)
            .await;
        Mock::given(method("POST")).and(path(POLICY)).respond_with(ResponseTemplate::new(204)).mount(&server).await;
        let dir = tempfile::tempdir().unwrap();
        let cfg = CoreConfig {
            backoff_base: Duration::from_millis(1),
            ..CoreConfig::default()
        };
        let core = ReinsCore::with_config(
            dir.path().to_str().unwrap(),
            &FakeKeys::default(),
            Arc::new(NoGoogle),
            Arc::new(Quiet),
            cfg,
        )
        .unwrap();
        core.login(server.uri(), "me@example.com".to_owned(), "pw".to_owned(), None).await.unwrap();
        let computer = crypto_box::SecretKey::generate(&mut crypto_box::aead::OsRng);
        let env = Env {
            server,
            core,
            computer,
            _dir: dir,
        };
        env.engine().store.pin_desktop_key(DESK, &env.client_key(), "Laptop", unix_now() - 86_400).unwrap();
        env
    }

    impl Env {
        fn engine(&self) -> Arc<Engine> {
            self.core.engine()
        }

        fn client_key(&self) -> String {
            reins_proto::desktop::encode_key(self.computer.public_key().as_bytes())
        }

        async fn publish_at(&self, now: i64) {
            let engine = self.engine();
            let session = engine.session().unwrap();
            engine.publish_desktop_policy_at(&session, u64::try_from(now).unwrap() * 1000).await;
        }

        /// Every policy post so far.
        async fn posts(&self) -> Vec<PolicyPost> {
            self.server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .filter(|r| r.method.as_str() == "POST" && r.url.path() == POLICY)
                .map(|r| serde_json::from_slice(&r.body).unwrap())
                .collect()
        }

        /// The message in `item`, opened with the computer's key and checked against the device's key.
        fn open(&self, item: &SealedPolicy) -> PolicyMessage {
            let bytes = data_encoding::BASE64URL_NOPAD.decode(item.sealed.as_bytes()).unwrap();
            let signed: Signed = serde_json::from_slice(&self.computer.unseal(&bytes).unwrap()).unwrap();
            let device = self.engine().sign_key().unwrap().public_b64();
            open(&signed, Some(&device), &crate::roles::keys::ed25519_verify).unwrap()
        }

        fn change_snapshot(&self, f: impl FnOnce(&mut Snapshot)) {
            let engine = self.engine();
            let mut snap = engine.ap_snapshot().unwrap();
            f(&mut snap);
            engine.ap_put_snapshot(&snap).unwrap();
        }
    }

    fn snapshot_of(m: PolicyMessage) -> DesktopPolicy {
        match m {
            PolicyMessage::Snapshot(p) => *p,
            PolicyMessage::Tighten(_) => panic!("a snapshot expected"),
        }
    }

    /// Starts a task for `connection` as the owner would; its id.
    fn start_task(env: &Env, connection: &str, now: i64) -> String {
        start_task_on(env, connection, now, &["acme/app"])
    }

    fn start_task_on(env: &Env, connection: &str, now: i64, repos: &[&str]) -> String {
        let draft = crate::autopilot::TaskView {
            id: String::new(),
            connection_id: connection.to_owned(),
            connection_label: "Claude Code".to_owned(),
            title: "Fix the CI".to_owned(),
            reason: None,
            repos: repos.iter().map(|r| (*r).to_owned()).collect(),
            branches: vec!["fix/*".to_owned()],
            paths: Vec::new(),
            people: vec!["ann@acme.io".to_owned()],
            operations: vec!["git_push".to_owned()],
            ceiling: 1,
            budget: None,
            uses_left: None,
            started_at: now,
            ends_at: now + 3_600,
            ended_at: None,
            started_by: "you".to_owned(),
            auto: 0,
            asked: 0,
            parked: 0,
            any: false,
            requested_ceiling: 1,
        };
        env.engine().ap_begin_task(&draft, "you", now).unwrap().envelope.id
    }

    #[tokio::test]
    async fn body_has_no_private_facts() {
        let env = env().await;
        env.change_snapshot(|s| {
            s.facts.set(FactKind::Owner, "acme", Zone::Work);
            s.facts.set(FactKind::Repo, "pal/app", Zone::Friends);
            s.facts.set(FactKind::Host, "build.local", Zone::Mine);
            s.facts.set(FactKind::Person, "bob@example.com", Zone::Friends);
            s.facts.set_label(FactKind::Person, "bob@example.com", "Bob Builder");
            s.facts.set(FactKind::Domain, "secret-client.io", Zone::Work);
            s.facts.set(FactKind::Chat, "987654321", Zone::Friends);
            s.facts.own_addresses.insert("me@example.com".to_owned());
        });
        let now = unix_now();
        let body = env.engine().ap_desktop_policy(DESK, &env.client_key(), now).unwrap();
        let json = serde_json::to_string(&body).unwrap();
        for private in ["bob@example.com", "Bob Builder", "secret-client.io", "987654321", "me@example.com"] {
            assert!(!json.contains(private), "{private} left the phone: {json}");
        }
        for kept in ["acme", "pal/app", "build.local"] {
            assert!(json.contains(kept), "{kept} is needed on the computer");
        }
        // What is posted is the same body.
        env.publish_at(now).await;
        let posted = snapshot_of(env.open(&env.posts().await[0].items[0]));
        let posted = serde_json::to_string(&posted).unwrap();
        assert!(!posted.contains("bob@example.com") && posted.contains("build.local"));
    }

    #[tokio::test]
    async fn published_on_change_and_hourly() {
        let env = env().await;
        let now = unix_now();
        env.publish_at(now).await;
        assert_eq!(env.posts().await.len(), 1, "sent once paired");
        env.publish_at(now + 10).await;
        assert_eq!(env.posts().await.len(), 1, "nothing changed: no post");
        env.change_snapshot(|s| s.matrix.set(Zone::Mine, Level::L0, Cell::Ask));
        env.publish_at(now + 20).await;
        assert_eq!(env.posts().await.len(), 2, "a cell changed");
        env.publish_at(now + 30).await;
        assert_eq!(env.posts().await.len(), 2);
        env.publish_at(now + 20 + REFRESH_SECS).await;
        assert_eq!(env.posts().await.len(), 3, "signed again after an hour");
        // A change made through the app goes at once.
        let owner = crate::autopilot::testing::FakeOwner::new(Ok(true));
        env.core
            .set_matrix_cell(crate::autopilot::ZoneChoice::Work, 0, crate::autopilot::CellChoice::Ask, owner)
            .await
            .unwrap();
        assert_eq!(env.posts().await.len(), 4, "the owner's change was sent");
        // An older server: nothing kept as sent, so tried again.
        env.server.reset().await;
        Mock::given(method("POST")).and(path(POLICY)).respond_with(ResponseTemplate::new(404)).mount(&env.server).await;
        env.change_snapshot(|s| s.matrix.set(Zone::Mine, Level::L1, Cell::Ask));
        env.publish_at(now + 40 + REFRESH_SECS).await;
        env.publish_at(now + 50 + REFRESH_SECS).await;
        assert_eq!(env.posts().await.len(), 2, "tried each time while not delivered");
    }

    #[tokio::test]
    async fn lockdown_sends_a_tighten_first() {
        let env = env().await;
        env.publish_at(unix_now()).await;
        env.core.set_autopilot_mode(None, Some(AutopilotMode::Lockdown), None).await.unwrap();
        let posts = env.posts().await;
        assert_eq!(posts.len(), 2, "the mode change was sent at once");
        let items = &posts[1].items;
        assert_eq!(items.iter().map(|i| i.kind).collect::<Vec<_>>(), [PolicyKind::Tighten, PolicyKind::Snapshot]);
        let PolicyMessage::Tighten(t) = env.open(&items[0]) else {
            panic!("a tighten first")
        };
        assert!(t.tightening.lockdown);
        assert_eq!(t.binding.not_after - t.binding.issued_at, TIGHTEN_SECS);
        let p = snapshot_of(env.open(&items[1]));
        assert_eq!(p.mode.mode, Mode::Lockdown);
        assert_eq!(p.binding.version, t.binding.version + 1);
        assert_eq!((items[0].version, items[1].version), (t.binding.version, p.binding.version));
    }

    #[tokio::test]
    async fn sealed_to_the_pinned_key_and_signed_by_the_device() {
        let env = env().await;
        let now = unix_now();
        // A global bypass: the computer is under it, and is told when it ends.
        env.core.start_bypass(None, Some(30)).await.unwrap();
        env.publish_at(now).await;
        let posts = env.posts().await;
        let item = &posts.last().unwrap().items[0];
        assert_eq!((item.connection_id.as_str(), item.kind), (DESK, PolicyKind::Snapshot));
        let p = snapshot_of(env.open(item));
        assert_eq!(p.binding.client_key, env.client_key());
        assert_eq!(p.binding.connection_id, DESK);
        assert_eq!(p.binding.profile, PERSONAL);
        assert_eq!(p.binding.not_after - p.binding.issued_at, VALIDITY_SECS);
        assert_eq!(p.roots, ["~/Documents/dev", "~/src", "~/code", "~/projects"]);
        assert_eq!(p.mode.mode, Mode::Bypass);
        let until = p.mode.until.expect("a bypass for 30 minutes ends");
        assert!((until - now - 30 * 60).abs() <= 5, "{until}");
        // Another key can't open it, and another signer isn't the device.
        let other = crypto_box::SecretKey::generate(&mut crypto_box::aead::OsRng);
        let bytes = data_encoding::BASE64URL_NOPAD.decode(item.sealed.as_bytes()).unwrap();
        assert!(other.unseal(&bytes).is_err());
        let signed: Signed = serde_json::from_slice(&env.computer.unseal(&bytes).unwrap()).unwrap();
        let mut not_ours = [9u8; 32];
        not_ours[0] = 1;
        let not_ours = reins_proto::desktop::encode_key(&not_ours);
        assert!(open(&signed, Some(&not_ours), &crate::roles::keys::ed25519_verify).is_err());
    }

    #[tokio::test]
    async fn versions_only_grow() {
        let env = env().await;
        let now = unix_now();
        let mut seen = Vec::new();
        // The clock going back does not take the versions with it.
        for (i, at) in [now, now - 7_200, now + 5, now - 86_400].into_iter().enumerate() {
            let cell = if i % 2 == 0 {
                Cell::Auto
            } else {
                Cell::Ask
            };
            env.change_snapshot(|s| s.matrix.set(Zone::Work, Level::L3, cell));
            env.publish_at(at).await;
            let posts = env.posts().await;
            seen.extend(posts.last().unwrap().items.iter().map(|i| (i.version, env.open(i).version())));
        }
        assert!(seen.len() >= 4, "{seen:?}");
        assert!(seen.iter().all(|(posted, signed)| posted == signed), "{seen:?}");
        assert!(seen.windows(2).all(|w| w[0].0 < w[1].0), "{seen:?}");
        assert!(seen[0].0 >= u64::try_from(now).unwrap() * 1000, "from the clock");
    }

    #[tokio::test]
    async fn tasks_are_the_connections_own() {
        let env = env().await;
        let now = unix_now();
        env.engine().store.pin_desktop_key(OTHER, &env.client_key(), "Desktop", now).unwrap();
        let mine_id = start_task(&env, DESK, now);
        let theirs_id = start_task(&env, OTHER, now);
        let ids = |p: &DesktopPolicy| p.snapshot.tasks.iter().map(|t| t.id.clone()).collect::<Vec<_>>();
        let mine = env.engine().ap_desktop_policy(DESK, &env.client_key(), now).unwrap();
        assert_eq!(ids(&mine), std::slice::from_ref(&mine_id));
        assert_eq!(mine.snapshot.tasks[0].resources, ["acme/app"]);
        assert!(mine.snapshot.tasks[0].people.is_empty(), "the people a task reaches stay on the phone");
        let theirs = env.engine().ap_desktop_policy(OTHER, &env.client_key(), now).unwrap();
        assert_eq!(ids(&theirs), [theirs_id]);
        // A task bounded only by the people it may reach isn't sent: without them its envelope would bound nothing.
        let people_only = start_task_on(&env, OTHER, now, &[]);
        let theirs = env.engine().ap_desktop_policy(OTHER, &env.client_key(), now).unwrap();
        assert!(!ids(&theirs).contains(&people_only), "{:?}", ids(&theirs));
        // Each computer gets its own; a task stopped is named in a tighten first.
        env.publish_at(now).await;
        assert_eq!(env.posts().await[0].items.len(), 2, "one snapshot per computer");
        env.core.end_task(mine_id.clone()).await.unwrap();
        let posts = env.posts().await;
        assert_eq!(posts.len(), 2, "Stop is sent at once");
        let items = &posts[1].items;
        assert_eq!(
            items.iter().map(|i| (i.connection_id.as_str(), i.kind)).collect::<Vec<_>>(),
            [(DESK, PolicyKind::Tighten), (DESK, PolicyKind::Snapshot)]
        );
        let PolicyMessage::Tighten(t) = env.open(&items[0]) else {
            panic!("a tighten first")
        };
        assert_eq!(t.tightening.ended_tasks, [mine_id]);
        assert!(snapshot_of(env.open(&items[1])).snapshot.tasks.is_empty());
    }
    /// A folder the computer proposed, confirmed with the owner check, goes into its snapshot at once; removing it
    /// tells the computer to stop deciding first (a tighten), then sends the snapshot without it. The proposals come
    /// with the syncs, never as a notification.
    #[tokio::test]
    async fn confirmed_folders_are_signed_and_removing_tightens() {
        let env = env().await;
        let memory = "/home/me/.claude/projects/*/memory";
        mount(
            &env.server,
            "GET",
            "/reins/api/desktop/folders",
            200,
            json!({"items": [{"connection_id": DESK, "at": 100, "proposal": {"home": "/home/me", "folders": [
                {"path": memory, "kind": "scratch"}, {"path": "/home/me", "kind": "root"}]}}]}),
        )
        .await;
        env.publish_at(unix_now()).await;
        let engine = env.engine();
        engine.fetch_desktop_folders(&engine.session().unwrap()).await;
        let qs = env.core.folder_questions().await.unwrap();
        assert_eq!(qs.iter().map(|q| q.path.as_str()).collect::<Vec<_>>(), [memory], "the home folder is refused");
        let owner = crate::autopilot::testing::FakeOwner::new(Ok(true));
        env.core
            .confirm_computer_folder(
                DESK.to_owned(),
                memory.to_owned(),
                crate::autopilot::FolderKindChoice::Scratch,
                owner,
            )
            .await
            .unwrap();
        let posts = env.posts().await;
        let last = posts.last().unwrap();
        assert_eq!(
            last.items.iter().map(|i| i.kind).collect::<Vec<_>>(),
            [PolicyKind::Snapshot],
            "loosening: no tighten"
        );
        assert_eq!(snapshot_of(env.open(&last.items[0])).scratch, [memory]);
        env.core
            .remove_computer_folder(DESK.to_owned(), memory.to_owned(), crate::autopilot::FolderKindChoice::Scratch)
            .await
            .unwrap();
        let posts = env.posts().await;
        let items = &posts.last().unwrap().items;
        assert_eq!(items.iter().map(|i| i.kind).collect::<Vec<_>>(), [PolicyKind::Tighten, PolicyKind::Snapshot]);
        let PolicyMessage::Tighten(t) = env.open(&items[0]) else {
            panic!("a tighten first")
        };
        assert!(t.tightening.stop_auto);
        assert!(snapshot_of(env.open(&items[1])).scratch.is_empty());
    }
}
