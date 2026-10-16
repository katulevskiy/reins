//! Phone roles end to end: two phones and a fake server that keeps the roster, the joins and the handovers the way
//! the real one does (it checks every entry with `reins_proto`'s rules), and that can also lie. Covers create, enroll
//! (commit, challenge, reveal, six digits on both phones), signed answers, handover, removal, recovery and rewrap, and
//! the server's lies: a rolled-back or forged roster, a swapped challenge, a replaced recovery wrap.

mod common;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use common::{FakeGoogle, FakeKeys, RecordingNotifier};
use data_encoding::BASE64URL_NOPAD;
use reins_core::crypto::{Kdf, master_key, master_password_hash};
use reins_core::{
    CoreConfig, CoreError, EnrollProgress, GoogleTokenProvider, HandoverProgress, Notifier, PhoneRole, ReinsCore,
};
use reins_proto::answer::{AnswerKind, SignedAnswer, answer_message};
use reins_proto::canon::parse_sig;
use reins_proto::enroll::{EnrollChallenge, EnrollReveal, check_commitment, parse_nonce};
use reins_proto::join::{JoinAnswer, JoinCreated, JoinRequest, JoinState, JoinStatus, NewJoin};
use reins_proto::roster::{
    HandoverCreated, HandoverRequest, HandoverState, HandoverStatus, NewHandover, RecoveryWrap, RecoveryWrapRequest,
    Roster, RosterAppend, RosterAppended, RosterEntry, RosterReason, recovery_wrap_hash, verify_step,
};
use serde_json::{Value, json};
use wiremock::matchers::{method, path, path_regex};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const EMAIL: &str = "me@example.com";
const USER: &str = "user-1";
const PASSWORD: &str = "correct horse battery staple";
const KDF: Kdf = Kdf::Pbkdf2 {
    iterations: 5000,
};

fn verify(key: &[u8; 32], msg: &[u8], sig: &[u8; 64]) -> bool {
    ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, key).verify(msg, sig).is_ok()
}

fn mph(password: &str) -> String {
    master_password_hash(&master_key(password, EMAIL, KDF).unwrap(), password).to_string()
}

struct Join {
    device: String,
    request: JoinRequest,
    status: JoinStatus,
    sealed: Option<String>,
    challenge: Option<EnrollChallenge>,
}

struct Handover {
    device: String,
    request: HandoverRequest,
    status: HandoverStatus,
    epoch: Option<u64>,
}

#[derive(Default)]
#[expect(clippy::struct_excessive_bools, reason = "the fake server's switches, each a lie it can tell")]
struct State {
    roster: Vec<RosterEntry>,
    wrap: Option<String>,
    /// Every wrap the roster brought, by the epoch that brought it (a lying server keeps them).
    wraps: Vec<(u64, String)>,
    bundle: Option<(u64, String)>,
    joins: HashMap<String, Join>,
    handovers: HashMap<String, Handover>,
    requests: Vec<Value>,
    pairings: Vec<Value>,
    /// Answers whose signature verified: (kind, id, bytes).
    signed: Vec<(AnswerKind, String, Vec<u8>)>,
    unsigned: Vec<String>,
    /// Every answer to a request that reached the server (signed or not, taken or refused).
    answered: Vec<String>,
    /// Requests the server hands out by id (`GET /requests/<id>`, what a push names).
    by_id: HashMap<String, Value>,
    password: String,
    next: u32,
    // Lies.
    roll_back: bool,
    extra_entry: Option<RosterEntry>,
    swap_challenge: bool,
    stale_wrap: Option<String>,
    /// Serve the roster as it was at this epoch, with the wrap in force then: a consistent older state.
    serve_at: Option<u64>,
    /// The next roster append answers 503: after storing the entry (`true`) or without storing it (`false`).
    fail_append: Option<bool>,
    /// Answer an empty roster (a server that lost or dropped it).
    empty_roster: bool,
    /// Hold the answer to a reveal this long (the reveal itself is taken at once).
    reveal_delay: Option<std::time::Duration>,
    /// Show the primary this name instead of the one the joining phone gave.
    rename_join: Option<String>,
    /// Recovery-wrap requests that reached the server.
    wrap_requests: u32,
    deleted: bool,
}

#[derive(Clone)]
struct Fake(Arc<Mutex<State>>);

fn err(status: u16, code: &str) -> ResponseTemplate {
    ResponseTemplate::new(status).set_body_json(json!({"error": code, "message": code}))
}

fn ok(body: impl serde::Serialize) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(body)
}

/// The device a bearer token was issued to (the fake's tokens carry it).
fn caller(req: &Request) -> String {
    let auth = req.headers.get("authorization").and_then(|v| v.to_str().ok()).unwrap_or_default();
    let payload = auth.trim_start_matches("Bearer ").split('.').nth(1).unwrap_or_default();
    let claims: Value = serde_json::from_slice(&BASE64URL_NOPAD.decode(payload.as_bytes()).unwrap()).unwrap();
    claims["device"].as_str().unwrap().to_owned()
}

impl State {
    fn primary_at(&self, epoch: u64) -> Option<[u8; 32]> {
        let current = self.roster.last()?;
        let at = self.roster.iter().find(|e| e.epoch == epoch)?;
        at.primary.same_keys(&current.primary).then(|| at.primary.sign_key().unwrap())
    }

    /// A signed answer's bytes when the primary signed it (else `None` and nothing recorded).
    fn open_answer(&mut self, kind: AnswerKind, id: &str, body: &[u8]) -> Option<Vec<u8>> {
        if self.roster.is_empty() {
            self.unsigned.push(id.to_owned());
            return Some(body.to_vec());
        }
        let signed: SignedAnswer = serde_json::from_slice(body).ok()?;
        let bytes = signed.answer_bytes().ok()?;
        let key = self.primary_at(signed.epoch)?;
        verify(&key, &answer_message(USER, kind, id, signed.epoch, &bytes), &parse_sig(&signed.sig)?).then(|| {
            self.signed.push((kind, id.to_owned(), bytes.clone()));
            bytes
        })
    }

    fn append(&mut self, me: &str, append: RosterAppend) -> ResponseTemplate {
        let entry = append.entry;
        let prev = self.roster.last();
        if prev.is_some_and(|p| entry.epoch <= p.epoch) {
            return err(409, "epoch_conflict");
        }
        if let Err(e) = verify_step(prev, &entry, verify) {
            return err(400, &e.to_string());
        }
        let new_wrap = entry.reason.brings_wrap();
        match &append.recovery_wrap {
            Some(w) if new_wrap && recovery_wrap_hash(w).as_deref() == Some(&entry.recovery_wrap_hash) => {}
            None if !new_wrap => {}
            _ => return err(400, "bad_wrap"),
        }
        match entry.reason {
            RosterReason::Create | RosterReason::Recovery if entry.primary.device_id != me => {
                return err(400, "not_the_caller");
            }
            RosterReason::Create | RosterReason::Recovery
                if append.master_password_hash.as_deref() != Some(&self.password) =>
            {
                return err(403, "wrong_proof");
            }
            RosterReason::Handover if append.bundle.is_none() => return err(400, "no_bundle"),
            _ => {}
        }
        if let Some(w) = append.recovery_wrap {
            self.wraps.push((entry.epoch, w.clone()));
            self.wrap = Some(w);
        }
        if let Some(b) = append.bundle {
            self.bundle = Some((entry.epoch, b));
        }
        if entry.reason == RosterReason::Handover {
            for h in self.handovers.values_mut().filter(|h| h.device == entry.primary.device_id) {
                h.status = HandoverStatus::Done;
                h.epoch = Some(entry.epoch);
            }
        }
        let appended = RosterAppended {
            epoch: entry.epoch,
            hash: entry.hash_hex().unwrap(),
        };
        self.roster.push(entry);
        ok(appended)
    }

    fn id(&mut self, prefix: &str) -> String {
        self.next += 1;
        format!("{prefix}{}", self.next)
    }

    fn route(&mut self, req: &Request) -> ResponseTemplate {
        let me = caller(req);
        let p = req.url.path().trim_start_matches("/reins/api").to_owned();
        let parts: Vec<&str> = p.trim_start_matches('/').split('/').collect();
        match (req.method.as_str(), parts.as_slice()) {
            ("GET", ["roster"]) if self.empty_roster => ok(Roster::default()),
            ("GET", ["roster"]) => {
                let mut entries = self.roster.clone();
                if let Some(at) = self.serve_at {
                    entries.retain(|e| e.epoch <= at);
                }
                if self.roll_back {
                    entries.pop();
                }
                entries.extend(self.extra_entry.clone());
                ok(Roster {
                    entries,
                })
            }
            ("POST", ["roster"]) => match self.fail_append.take() {
                Some(true) => {
                    drop(self.append(&me, req.body_json().unwrap()));
                    err(503, "unavailable")
                }
                Some(false) => err(503, "unavailable"),
                None => self.append(&me, req.body_json().unwrap()),
            },
            ("POST", ["account", "delete"]) => {
                self.deleted = true;
                ResponseTemplate::new(204)
            }
            ("POST", ["roster", "recovery-wrap"]) => {
                let r: RecoveryWrapRequest = req.body_json().unwrap();
                self.wrap_requests += 1;
                if r.master_password_hash != self.password {
                    return err(403, "wrong_proof");
                }
                if let Some(at) = self.serve_at {
                    let old = self.wraps.iter().rev().find(|(e, _)| *e <= at).unwrap().1.clone();
                    return ok(RecoveryWrap {
                        epoch: at,
                        recovery_wrap: old,
                    });
                }
                ok(RecoveryWrap {
                    epoch: self.roster.last().unwrap().epoch,
                    recovery_wrap: self.stale_wrap.clone().or_else(|| self.wrap.clone()).unwrap(),
                })
            }
            ("PUT", ["services" | "member"]) => ResponseTemplate::new(204),
            ("PUT", ["device"]) => ok(json!({"replaced_previous": false})),
            ("GET", ["pending"]) => {
                let joins: Vec<&JoinRequest> = self
                    .joins
                    .values()
                    .filter(|j| j.status == JoinStatus::Waiting && j.device != me)
                    .map(|j| &j.request)
                    .collect();
                let handovers: Vec<&HandoverRequest> = self
                    .handovers
                    .values()
                    .filter(|h| h.status == HandoverStatus::Waiting)
                    .map(|h| &h.request)
                    .collect();
                let requests = std::mem::take(&mut self.requests);
                let pairings = std::mem::take(&mut self.pairings);
                ok(json!({"requests": requests, "pairings": pairings, "joins": joins, "handovers": handovers}))
            }
            ("POST", ["joins"]) => {
                let new: NewJoin = req.body_json().unwrap();
                let id = self.id("j");
                let request = JoinRequest {
                    v: 1,
                    id: id.clone(),
                    device_name: self.rename_join.clone().unwrap_or(new.device_name),
                    public_key: new.public_key,
                    created_at: 100,
                    device_id: Some(me.clone()),
                    sign_pub: new.sign_pub,
                    commitment: new.commitment,
                    reveal: None,
                };
                self.joins.insert(
                    id.clone(),
                    Join {
                        device: me,
                        request,
                        status: JoinStatus::Waiting,
                        sealed: None,
                        challenge: None,
                    },
                );
                ok(JoinCreated {
                    id,
                    expires_at: 4_000_000_000,
                })
            }
            ("GET", ["joins", id]) => {
                let swap = self.swap_challenge;
                let Some(j) = self.joins.get(*id) else {
                    return err(404, "not_found");
                };
                if j.device == me {
                    let mut challenge = j.challenge.clone();
                    if swap && let Some(c) = &mut challenge {
                        c.nonce = BASE64URL_NOPAD.encode(&[9; 32]);
                    }
                    ok(JoinState {
                        status: j.status,
                        sealed: j.sealed.clone(),
                        challenge,
                    })
                } else {
                    ok(&j.request)
                }
            }
            ("POST", ["joins", id, "challenge"]) => {
                let Some(j) = self.joins.get_mut(*id) else {
                    return err(404, "not_found");
                };
                if j.challenge.is_some() {
                    return err(409, "already_answered");
                }
                j.challenge = Some(req.body_json().unwrap());
                ResponseTemplate::new(204)
            }
            ("POST", ["joins", id, "reveal"]) => {
                let Some(j) = self.joins.get_mut(*id) else {
                    return err(404, "not_found");
                };
                let reveal: EnrollReveal = req.body_json().unwrap();
                let r = &j.request;
                let opens = check_commitment(
                    r.commitment.as_deref().unwrap(),
                    USER,
                    &me,
                    &r.public_key,
                    r.sign_pub.as_deref().unwrap(),
                    &parse_nonce(&reveal.nonce).unwrap(),
                );
                if j.device != me || j.challenge.is_none() || !opens {
                    return err(400, "bad_reveal");
                }
                j.request.reveal = Some(reveal);
                let answer = ResponseTemplate::new(204);
                match self.reveal_delay {
                    Some(delay) => answer.set_delay(delay),
                    None => answer,
                }
            }
            ("POST", ["joins", id, "response"]) => {
                let id = (*id).to_owned();
                let Some(bytes) = self.open_answer(AnswerKind::Join, &id, &req.body) else {
                    return err(403, "bad_signature");
                };
                let answer: JoinAnswer = serde_json::from_slice(&bytes).unwrap();
                let j = self.joins.get_mut(&id).unwrap();
                j.status = if answer.approve {
                    JoinStatus::Approved
                } else {
                    JoinStatus::Denied
                };
                j.sealed = answer.sealed;
                ResponseTemplate::new(204)
            }
            ("POST", ["handovers"]) => {
                let new: NewHandover = req.body_json().unwrap();
                let id = self.id("h");
                let name = self.roster.last().unwrap().member(&me).unwrap().name.clone();
                self.handovers.insert(
                    id.clone(),
                    Handover {
                        device: me.clone(),
                        request: HandoverRequest {
                            v: 1,
                            id: id.clone(),
                            device_id: me,
                            device_name: name,
                            created_at: 100,
                            signed: new.signed,
                        },
                        status: HandoverStatus::Waiting,
                        epoch: None,
                    },
                );
                ok(HandoverCreated {
                    id,
                    expires_at: 4_000_000_000,
                })
            }
            ("GET", ["handovers", id]) => match self.handovers.get(*id) {
                Some(h) if h.device == me => ok(HandoverState {
                    status: h.status,
                    epoch: h.epoch,
                }),
                Some(h) => ok(&h.request),
                None => err(404, "not_found"),
            },
            ("POST", ["handovers", id, "response"]) => {
                if let Some(h) = self.handovers.get_mut(*id) {
                    h.status = HandoverStatus::Denied;
                }
                ResponseTemplate::new(204)
            }
            ("GET", ["handover-bundle"]) => match &self.bundle {
                Some((epoch, sealed)) if self.roster.last().unwrap().primary.device_id == me => {
                    ok(json!({"epoch": epoch, "sealed": sealed}))
                }
                _ => err(404, "not_found"),
            },
            ("DELETE", ["handover-bundle"]) => {
                self.bundle = None;
                ResponseTemplate::new(204)
            }
            ("GET", ["requests", id]) => match self.by_id.get(*id) {
                Some(request) => ok(request),
                None => err(404, "not_found"),
            },
            ("POST", ["requests", id, "response"]) => {
                let id = (*id).to_owned();
                self.answered.push(id.clone());
                match self.open_answer(AnswerKind::Request, &id, &req.body) {
                    Some(_) => ResponseTemplate::new(204),
                    None => err(403, "bad_signature"),
                }
            }
            _ => err(404, "not_found"),
        }
    }
}

impl Respond for Fake {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        self.0.lock().unwrap().route(request)
    }
}

/// Tokens name the device they were issued to, so the fake knows who calls.
struct Tokens;

impl Respond for Tokens {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let form: HashMap<String, String> = url::form_urlencoded::parse(&request.body).into_owned().collect();
        let device = form
            .get("deviceIdentifier")
            .cloned()
            .or_else(|| form.get("refresh_token").and_then(|t| t.strip_prefix("REFRESH:")).map(str::to_owned))
            .unwrap();
        let claims = json!({"sub": USER, "email": EMAIL, "exp": 4_000_000_000_i64, "device": device}).to_string();
        ok(json!({
            "access_token": format!("h.{}.s", BASE64URL_NOPAD.encode(claims.as_bytes())),
            "refresh_token": format!("REFRESH:{device}"),
            "expires_in": 7200,
        }))
    }
}

struct World {
    server: MockServer,
    fake: Fake,
    dirs: Vec<tempfile::TempDir>,
}

async fn world() -> World {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/identity/accounts/prelogin"))
        .respond_with(ok(json!({"kdf": 0, "kdfIterations": 5000})))
        .mount(&server)
        .await;
    Mock::given(method("POST")).and(path("/identity/connect/token")).respond_with(Tokens).mount(&server).await;
    common::mount_account_vault(&server, EMAIL, PASSWORD).await;
    let fake = Fake(Arc::new(Mutex::new(State {
        password: mph(PASSWORD),
        ..State::default()
    })));
    Mock::given(path_regex("^/reins/api/")).respond_with(fake.clone()).mount(&server).await;
    World {
        server,
        fake,
        dirs: Vec::new(),
    }
}

impl World {
    async fn phone(&mut self) -> Arc<ReinsCore> {
        let dir = tempfile::tempdir().unwrap();
        let google: Arc<dyn GoogleTokenProvider> = Arc::new(FakeGoogle::new());
        let notifier: Arc<dyn Notifier> = Arc::new(RecordingNotifier::default());
        let core =
            ReinsCore::with_config(dir.path().to_str().unwrap(), &FakeKeys, google, notifier, CoreConfig::default())
                .unwrap();
        core.login(self.server.uri(), EMAIL.to_owned(), PASSWORD.to_owned(), None).await.unwrap();
        self.dirs.push(dir);
        core
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.fake.0.lock().unwrap()
    }
}

/// Recovers `core` the way the app does: the preview, which the user confirms, then the recovery from its tip.
async fn recover(core: &ReinsCore, typed: &str, name: &str) -> Result<reins_core::RosterSummary, CoreError> {
    let preview = core.roster_recover_preview(typed.to_owned()).await?;
    core.roster_recover(typed.to_owned(), name.to_owned(), preview.tip).await
}

/// This phone's device id, as the roster lists it.
async fn device_of(core: &ReinsCore) -> String {
    let summary = core.roster_summary(false).await.unwrap().unwrap();
    summary.members.into_iter().find(|m| m.this_phone).unwrap().device_id
}

/// Enrolls `viewer` through `primary`: both phones show the same code, and the viewer ends up enrolled.
async fn enroll(primary: &ReinsCore, viewer: &ReinsCore, name: &str) {
    let start = viewer.enroll_begin(name.to_owned()).await.unwrap();
    assert_eq!(viewer.enroll_poll().await.unwrap(), EnrollProgress::Waiting, "no challenge yet");
    primary.sync(0).await.unwrap();
    let view = primary.enroll_view(start.id.clone()).await.unwrap();
    assert_eq!((view.code.as_deref(), view.device_name.as_str()), (None, name), "no code before the reveal");
    let EnrollProgress::Compare {
        code,
    } = viewer.enroll_poll().await.unwrap()
    else {
        panic!("the viewer shows a code once challenged")
    };
    let view = primary.enroll_view(start.id.clone()).await.unwrap();
    assert_eq!(view.code.as_deref(), Some(code.as_str()), "both phones show the same six digits");
    assert!(!view.proven, "a phone without the account secret proves nothing");
    primary.enroll_confirm(start.id.clone(), true).await.unwrap();
    assert_eq!(viewer.enroll_poll().await.unwrap(), EnrollProgress::Enrolled);
}

#[tokio::test]
async fn phones_create_enroll_hand_over_remove_and_recover() {
    let mut w = world().await;
    let a = w.phone().await;
    let b = w.phone().await;
    assert_eq!(a.phone_role().await.unwrap(), PhoneRole::Legacy);

    // A legacy account answers unsigned, as before.
    w.state().requests.push(json!({"v": 1, "id": "r0", "connection_id": "c1", "connection_label": "Claude",
        "created_at": 100, "call": {"tool": "gmail_search", "query": "x", "max_results": 1}}));
    a.sync(0).await.unwrap();
    for item in a.pending().await.unwrap() {
        a.deny(item.id).await.unwrap();
    }
    assert_eq!(w.state().unsigned, ["r0"]);

    // Create: A is primary at epoch 1; the wrap's hash is the entry's.
    let summary = a.roster_create("iPhone".to_owned(), Some(PASSWORD.to_owned())).await.unwrap();
    assert_eq!((summary.epoch, summary.role), (1, PhoneRole::Primary));
    assert!(a.roster_create("iPhone".to_owned(), Some(PASSWORD.to_owned())).await.is_err(), "once");

    // Enroll B.
    enroll(&a, &b, "Pixel").await;
    assert_eq!(b.phone_role().await.unwrap(), PhoneRole::Viewer);
    let seen = b.roster_summary(true).await.unwrap().unwrap();
    assert_eq!(seen.epoch, 2);
    assert_eq!(
        seen.members.iter().map(|m| (m.name.as_str(), m.primary, m.this_phone)).collect::<Vec<_>>(),
        [("iPhone", true, false), ("Pixel", false, true)]
    );
    assert_eq!(seen.owner_fingerprint, a.roster_summary(false).await.unwrap().unwrap().owner_fingerprint);
    assert!(w.state().signed.iter().any(|(k, _, _)| *k == AnswerKind::Join), "the join answer was signed");

    // A4 on a roster account: signed by the primary.
    w.state().requests.push(json!({"v": 1, "id": "r1", "connection_id": "c1", "connection_label": "Claude",
        "created_at": 100, "call": {"tool": "gmail_search", "query": "x", "max_results": 1}}));
    a.sync(0).await.unwrap();
    for item in a.pending().await.unwrap() {
        a.deny(item.id).await.unwrap();
    }
    assert!(w.state().signed.iter().any(|(k, id, _)| *k == AnswerKind::Request && id == "r1"));
    // Only the primary changes the roster.
    assert!(b.roster_remove_member(device_of(&a).await).await.is_err());

    // Handover to B.
    let id = b.handover_request().await.unwrap();
    assert_eq!(b.handover_poll().await.unwrap(), HandoverProgress::Waiting);
    a.sync(0).await.unwrap();
    let asks = a.handover_requests().await.unwrap();
    assert_eq!((asks.len(), asks[0].device_name.as_str()), (1, "Pixel"));
    let b_id = device_of(&b).await;
    assert_eq!(asks[0].device_id, b_id);
    a.handover_approve(id, b_id).await.unwrap();
    assert_eq!(b.handover_poll().await.unwrap(), HandoverProgress::Done);
    assert_eq!(b.phone_role().await.unwrap(), PhoneRole::Primary);
    assert_eq!(a.phone_role().await.unwrap(), PhoneRole::Viewer);
    assert!(w.state().bundle.is_none(), "the new primary dropped the bundle");
    assert!(!a.engine().holds_owner_seed().await.unwrap(), "the old primary dropped the owner seed");
    assert!(b.engine().holds_owner_seed().await.unwrap());

    // B removes A; A sees it.
    let a_id = device_of(&a).await;
    b.roster_remove_member(a_id).await.unwrap();
    a.roster_summary(true).await.unwrap();
    assert_eq!(a.phone_role().await.unwrap(), PhoneRole::Removed);
    assert_eq!(b.roster_summary(false).await.unwrap().unwrap().epoch, 4);

    // A recovers with the master password: a new owner key, B removed.
    assert!(recover(&a, "wrong password", "iPhone").await.is_err());
    let preview = a.roster_recover_preview(PASSWORD.to_owned()).await.unwrap();
    assert_eq!((preview.epoch, preview.removed_primary.as_str()), (5, "Pixel"));
    assert!(preview.viewers.is_empty(), "the user sees who stays: nobody but this phone");
    let recovered = a.roster_recover(PASSWORD.to_owned(), "iPhone".to_owned(), preview.tip).await.unwrap();
    assert_eq!((recovered.epoch, recovered.role), (5, PhoneRole::Primary));
    assert_ne!(recovered.owner_fingerprint, b.roster_summary(false).await.unwrap().unwrap().owner_fingerprint);
    b.roster_summary(true).await.unwrap();
    assert_eq!(b.phone_role().await.unwrap(), PhoneRole::Removed);
    // B's late answer is refused: it is not the primary now.
    let entries = w.state().roster.clone();
    assert_eq!(entries.last().unwrap().reason, RosterReason::Recovery);

    // A rewraps for the (same) password; a mistyped one is refused before anything is sealed.
    assert!(a.roster_rewrap("not it".to_owned()).await.is_err());
    a.roster_rewrap(PASSWORD.to_owned()).await.unwrap();
    assert_eq!(w.state().roster.last().unwrap().reason, RosterReason::Rewrap);
    // And recovering again from the new wrap still works (B, with the password).
    let again = recover(&b, PASSWORD, "Pixel").await.unwrap();
    assert_eq!(again.role, PhoneRole::Primary);
}

#[tokio::test]
async fn a_server_that_rolls_back_or_forges_the_roster_is_refused() {
    let mut w = world().await;
    let a = w.phone().await;
    let b = w.phone().await;
    a.roster_create("iPhone".to_owned(), Some(PASSWORD.to_owned())).await.unwrap();
    enroll(&a, &b, "Pixel").await;
    // Rollback: the server serves the chain without its newest entry.
    w.state().roll_back = true;
    assert!(a.roster_summary(true).await.is_err());
    assert!(b.roster_summary(true).await.is_err());
    w.state().roll_back = false;
    // Forgery: an entry the owner key did not sign.
    let last = w.state().roster.last().unwrap().clone();
    let mut forged = last.successor(RosterReason::Remove, last.primary.clone(), vec![], 99).unwrap();
    forged.sig = reins_proto::canon::encode_sig(&[1; 64]);
    w.state().extra_entry = Some(forged);
    assert!(b.roster_summary(true).await.is_err());
    // The role is what this phone verified, whatever the server says.
    assert_eq!(b.phone_role().await.unwrap(), PhoneRole::Viewer);
    w.state().extra_entry = None;
    assert_eq!(b.roster_summary(true).await.unwrap().unwrap().epoch, 2);
}

#[tokio::test]
async fn a_swapped_challenge_ends_the_enrollment() {
    let mut w = world().await;
    let a = w.phone().await;
    let b = w.phone().await;
    a.roster_create("iPhone".to_owned(), Some(PASSWORD.to_owned())).await.unwrap();
    let start = b.enroll_begin("Pixel".to_owned()).await.unwrap();
    a.sync(0).await.unwrap();
    a.enroll_view(start.id.clone()).await.unwrap();
    assert!(matches!(b.enroll_poll().await.unwrap(), EnrollProgress::Compare { .. }));
    w.state().swap_challenge = true;
    assert!(b.enroll_poll().await.is_err(), "another challenge after the reveal");
    assert_eq!(b.enroll_poll().await, Err(CoreError::NotFound), "and the enrollment is over");
}

#[tokio::test]
async fn an_unopenable_or_swapped_recovery_wrap_is_refused() {
    let mut w = world().await;
    let a = w.phone().await;
    let b = w.phone().await;
    a.roster_create("iPhone".to_owned(), Some(PASSWORD.to_owned())).await.unwrap();
    enroll(&a, &b, "Pixel").await;
    // A wrap the roster does not name (the server keeps an older one, or makes one up).
    w.state().stale_wrap = Some(BASE64URL_NOPAD.encode(b"some other wrap bytes, sealed by nobody"));
    let refused = b.roster_recover_preview(PASSWORD.to_owned()).await.unwrap_err();
    assert!(refused.to_string().contains("recovery wrap"), "{refused}");
    w.state().stale_wrap = None;
    assert_eq!(b.phone_role().await.unwrap(), PhoneRole::Viewer, "nothing changed");
}

#[tokio::test]
async fn a_handover_request_the_server_made_up_or_pointed_elsewhere_is_refused() {
    let mut w = world().await;
    let a = w.phone().await;
    let b = w.phone().await;
    let c = w.phone().await;
    a.roster_create("iPhone".to_owned(), Some(PASSWORD.to_owned())).await.unwrap();
    enroll(&a, &b, "Pixel").await;
    enroll(&a, &c, "Tablet").await;
    let (b_id, c_id) = (device_of(&b).await, device_of(&c).await);

    // B asks; the server points its signed request at C.
    let id = b.handover_request().await.unwrap();
    w.state().handovers.get_mut(&id).unwrap().request.device_id = c_id.clone();
    a.sync(0).await.unwrap();
    assert!(a.handover_requests().await.unwrap().is_empty(), "C never signed it");
    assert!(a.handover_approve(id.clone(), c_id.clone()).await.is_err());
    assert!(a.handover_approve(id.clone(), b_id.clone()).await.is_err(), "the request names C, not B");

    // A request the server made up for C (unsigned), and B's renamed by the server.
    w.state().handovers.get_mut(&id).unwrap().request.device_id = b_id.clone();
    let mut forged = w.state().handovers[&id].request.clone();
    forged.id = "h-forged".to_owned();
    forged.device_id = c_id.clone();
    forged.signed = None;
    w.state().handovers.insert(
        "h-forged".to_owned(),
        Handover {
            device: c_id.clone(),
            request: forged,
            status: HandoverStatus::Waiting,
            epoch: None,
        },
    );
    w.state().handovers.get_mut(&id).unwrap().request.device_name = "Your bank".to_owned();
    a.sync(0).await.unwrap();
    let asks = a.handover_requests().await.unwrap();
    assert_eq!(asks.len(), 1, "only B's own, signed request");
    assert_eq!(
        (asks[0].device_id.as_str(), asks[0].device_name.as_str()),
        (b_id.as_str(), "Pixel"),
        "named by the verified roster"
    );
    assert!(a.handover_approve("h-forged".to_owned(), c_id.clone()).await.is_err());
    // The phone the user approved must be the one the request is from.
    assert!(a.handover_approve(id.clone(), c_id.clone()).await.is_err());
    assert_eq!(a.phone_role().await.unwrap(), PhoneRole::Primary, "nothing changed");

    // A request signed at an older epoch (the phones changed since) is refused too.
    a.roster_remove_member(c_id).await.unwrap();
    a.sync(0).await.unwrap();
    assert!(a.handover_requests().await.unwrap().is_empty());
    assert!(a.handover_approve(id, b_id.clone()).await.is_err());
    // Asked again at the current epoch, it goes through.
    let again = b.handover_request().await.unwrap();
    a.handover_approve(again, b_id).await.unwrap();
    assert_eq!(b.handover_poll().await.unwrap(), HandoverProgress::Done);
}

#[tokio::test]
async fn a_roster_account_takes_no_join_over_one_key() {
    let mut w = world().await;
    let a = w.phone().await;
    let old = w.phone().await;
    // Before phone roles: an older phone's join is parked as it always was.
    old.join_begin("Old phone".to_owned()).await.unwrap();
    a.sync(0).await.unwrap();
    let parked = a.pending().await.unwrap();
    assert_eq!(parked.len(), 1);
    let parked_id = parked[0].id.clone();
    a.roster_create("iPhone".to_owned(), Some(PASSWORD.to_owned())).await.unwrap();
    // With a roster, the parked one is neither shown nor answered (signed or not), and nothing is sealed to it.
    assert!(a.join_view(parked_id.clone()).await.is_err());
    assert!(a.answer_join(parked_id.clone(), true).await.is_err());
    {
        let state = w.state();
        let join = &state.joins[&parked_id];
        assert_eq!((join.status, join.sealed.is_none()), (JoinStatus::Waiting, true));
        assert!(state.signed.iter().all(|(k, _, _)| *k != AnswerKind::Join), "no join answer was signed");
    }
    assert!(a.pending().await.unwrap().is_empty(), "and the item is gone");
    // A new one is not even parked.
    let other = w.phone().await;
    other.join_begin("Another old phone".to_owned()).await.unwrap();
    a.sync(0).await.unwrap();
    assert!(a.pending().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_declined_handover_request_replayed_is_not_shown_again() {
    let mut w = world().await;
    let a = w.phone().await;
    let b = w.phone().await;
    a.roster_create("iPhone".to_owned(), Some(PASSWORD.to_owned())).await.unwrap();
    enroll(&a, &b, "Pixel").await;
    let b_id = device_of(&b).await;
    let id = b.handover_request().await.unwrap();
    a.sync(0).await.unwrap();
    assert_eq!(a.handover_requests().await.unwrap().len(), 1);
    a.handover_deny(id.clone()).await.unwrap();
    // The server replays B's signed request under a new id, and twice at once.
    for copy in ["h-replay-1", "h-replay-2"] {
        let mut replayed = w.state().handovers[&id].request.clone();
        replayed.id = copy.to_owned();
        w.state().handovers.insert(
            copy.to_owned(),
            Handover {
                device: b_id.clone(),
                request: replayed,
                status: HandoverStatus::Waiting,
                epoch: None,
            },
        );
    }
    a.sync(0).await.unwrap();
    assert!(a.handover_requests().await.unwrap().is_empty(), "declined once, never shown again");
    assert!(a.handover_approve("h-replay-1".to_owned(), b_id.clone()).await.is_err());
    assert_eq!(a.phone_role().await.unwrap(), PhoneRole::Primary);
    // A fresh request from B is shown.
    let fresh = b.handover_request().await.unwrap();
    a.sync(0).await.unwrap();
    let asks = a.handover_requests().await.unwrap();
    assert_eq!(asks.iter().map(|h| h.id.as_str()).collect::<Vec<_>>(), [fresh.as_str()]);
}

#[tokio::test]
async fn a_recovery_never_starts_from_an_older_roster_than_the_phone_saw() {
    let mut w = world().await;
    let a = w.phone().await;
    let b = w.phone().await;
    let c = w.phone().await;
    a.roster_create("iPhone".to_owned(), Some(PASSWORD.to_owned())).await.unwrap();
    enroll(&a, &b, "Pixel").await;
    enroll(&a, &c, "Tablet").await;
    let c_id = device_of(&c).await;
    a.roster_remove_member(c_id).await.unwrap();
    b.roster_summary(true).await.unwrap();
    assert_eq!(w.state().roster.last().unwrap().epoch, 4);
    // Every removal brought a new wrap.
    assert_eq!(w.state().wraps.iter().map(|(e, _)| *e).collect::<Vec<_>>(), [1, 4]);

    // The server serves the roster as it was before the removal (epoch 3, Tablet still in it) with that time's own
    // wrap. B saw epoch 4: refused.
    w.state().serve_at = Some(3);
    assert!(b.roster_recover_preview(PASSWORD.to_owned()).await.is_err(), "older than this phone saw");
    // A phone with no roster of its own cannot know: it opens the old wrap, but the preview shows what it would
    // recover into (Tablet, removed since, is in it), so the user can refuse it.
    let fresh = w.phone().await;
    let preview = fresh.roster_recover_preview(PASSWORD.to_owned()).await.unwrap();
    assert_eq!(preview.epoch, 4);
    let names: Vec<&str> = preview.viewers.iter().map(|v| v.name.as_str()).collect();
    assert!(names.contains(&"Tablet"), "{names:?}");
    // The newer wrap does not open for the older roster (it names another wrap), and vice versa.
    let newest = w.state().wraps.last().unwrap().1.clone();
    w.state().stale_wrap = Some(newest);
    w.state().serve_at = None;
    let old = w.state().wraps[0].1.clone();
    w.state().stale_wrap = Some(old);
    assert!(fresh.roster_recover_preview(PASSWORD.to_owned()).await.is_err(), "an old wrap for the newer roster");
    w.state().stale_wrap = None;

    // The tip the user confirmed must still be the roster's when the recovery is signed.
    let preview = b.roster_recover_preview(PASSWORD.to_owned()).await.unwrap();
    assert_eq!(preview.removed_primary, "iPhone");
    enroll(&a, &c, "Tablet again").await;
    let refused = b.roster_recover(PASSWORD.to_owned(), "Pixel".to_owned(), preview.tip).await.unwrap_err();
    assert!(refused.to_string().contains("changed"), "{refused}");
    assert_eq!(b.phone_role().await.unwrap(), PhoneRole::Viewer);
    assert_eq!(a.phone_role().await.unwrap(), PhoneRole::Primary);
}

#[tokio::test]
async fn a_viewer_with_bypass_on_answers_nothing_pushed_to_it() {
    let mut w = world().await;
    let a = w.phone().await;
    let b = w.phone().await;
    // B ran a bypass before phone roles (any phone could): it would approve whatever arrives.
    b.start_bypass(None, Some(60)).await.unwrap();
    a.roster_create("iPhone".to_owned(), Some(PASSWORD.to_owned())).await.unwrap();
    enroll(&a, &b, "Pixel").await;
    let request = |id: &str| {
        json!({"v": 1, "id": id, "connection_id": "c1", "connection_label": "Claude", "created_at": 100,
               "call": {"tool": "gmail_search", "query": "from:bank", "max_results": 1}})
    };
    // A server (or a stale token) pushes the request to the viewer, and its long-poll carries another.
    w.state().by_id.insert("r9".to_owned(), request("r9"));
    b.handle_push("req".to_owned(), "r9".to_owned()).await.unwrap();
    b.handle_push_deferring_autopilot("req".to_owned(), "r9".to_owned()).await.unwrap();
    w.state().requests.push(request("r10"));
    b.sync(0).await.unwrap();
    // Nothing was answered, approved or parked: the requests wait for the primary.
    assert!(w.state().answered.is_empty(), "{:?}", w.state().answered);
    assert!(b.pending().await.unwrap().is_empty());
    // And a tap on the viewer cannot answer either.
    assert_eq!(b.deny("r9".to_owned()).await, Err(CoreError::ViewerOnly));
    // The primary answers, signed.
    w.state().requests.push(request("r11"));
    a.sync(0).await.unwrap();
    for item in a.pending().await.unwrap() {
        a.deny(item.id).await.unwrap();
    }
    assert_eq!(w.state().answered, ["r11"]);
    // Removed, B decides nothing either.
    let b_id = device_of(&b).await;
    a.roster_remove_member(b_id).await.unwrap();
    b.roster_summary(true).await.unwrap();
    w.state().by_id.insert("r12".to_owned(), request("r12"));
    b.handle_push("req".to_owned(), "r12".to_owned()).await.unwrap();
    assert_eq!(w.state().answered, ["r11"]);
}

#[tokio::test]
async fn a_second_challenge_is_refused_even_while_the_first_reveal_is_in_flight() {
    let mut w = world().await;
    let a = w.phone().await;
    let b = Arc::new(w.phone().await);
    a.roster_create("iPhone".to_owned(), Some(PASSWORD.to_owned())).await.unwrap();
    let start = b.enroll_begin("Pixel".to_owned()).await.unwrap();
    a.sync(0).await.unwrap();
    a.enroll_view(start.id).await.unwrap();
    // The server holds the answer to B's reveal; meanwhile a second poll runs and the server offers another
    // challenge. The first one was kept before the nonce went out, so the second is refused.
    w.state().reveal_delay = Some(std::time::Duration::from_millis(400));
    let first = {
        let b = Arc::clone(&b);
        tokio::spawn(async move { b.enroll_poll().await })
    };
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    w.state().swap_challenge = true;
    let second = b.enroll_poll().await;
    let first = first.await.unwrap();
    assert!(
        matches!((&first, &second), (Ok(EnrollProgress::Compare { .. }), Err(_))),
        "the first poll shows the code, the second refuses the swapped challenge: {first:?} {second:?}"
    );
}

#[tokio::test]
async fn the_joining_phone_refuses_to_be_listed_under_a_name_it_did_not_give() {
    let mut w = world().await;
    let a = w.phone().await;
    let b = w.phone().await;
    a.roster_create("iPhone".to_owned(), Some(PASSWORD.to_owned())).await.unwrap();
    w.state().rename_join = Some("Your bank".to_owned());
    let start = b.enroll_begin("Pixel".to_owned()).await.unwrap();
    a.sync(0).await.unwrap();
    a.enroll_view(start.id.clone()).await.unwrap();
    assert!(matches!(b.enroll_poll().await.unwrap(), EnrollProgress::Compare { .. }));
    let view = a.enroll_view(start.id.clone()).await.unwrap();
    assert_eq!(view.device_name, "Your bank", "the primary sees what the server relayed");
    a.enroll_confirm(start.id, true).await.unwrap();
    let refused = b.enroll_poll().await.unwrap_err();
    assert!(refused.to_string().contains("names this phone differently"), "{refused}");
}

#[tokio::test]
async fn a_change_the_server_took_without_answering_completes_and_one_it_refused_changes_nothing() {
    let mut w = world().await;
    let a = w.phone().await;
    let b = w.phone().await;
    let c = w.phone().await;
    // Create: the server stored it, then the answer was lost. The retry finds it and keeps the same keys.
    w.state().fail_append = Some(true);
    assert!(a.roster_create("iPhone".to_owned(), Some(PASSWORD.to_owned())).await.is_ok());
    assert_eq!(a.phone_role().await.unwrap(), PhoneRole::Primary);
    assert!(a.engine().holds_owner_seed().await.unwrap());
    enroll(&a, &b, "Pixel").await;
    enroll(&a, &c, "Tablet").await;
    // A removal that never reached the server: refused, nothing changed, the keys kept.
    w.state().fail_append = Some(false);
    let c_id = device_of(&c).await;
    assert!(a.roster_remove_member(c_id.clone()).await.is_err());
    assert_eq!(a.roster_summary(true).await.unwrap().unwrap().members.len(), 3);
    assert!(a.engine().holds_owner_seed().await.unwrap());
    // A removal stored but unanswered: confirmed by the fetch, done.
    w.state().fail_append = Some(true);
    a.roster_remove_member(c_id).await.unwrap();
    assert_eq!(a.roster_summary(false).await.unwrap().unwrap().members.len(), 2);
    // A handover stored but unanswered: the old primary is a viewer, and drops the owner seed, once it sees it.
    let b_id = device_of(&b).await;
    let id = b.handover_request().await.unwrap();
    a.sync(0).await.unwrap();
    w.state().fail_append = Some(true);
    a.handover_approve(id, b_id).await.unwrap();
    assert_eq!(a.phone_role().await.unwrap(), PhoneRole::Viewer);
    assert!(!a.engine().holds_owner_seed().await.unwrap());
    assert_eq!(b.handover_poll().await.unwrap(), HandoverProgress::Done);
    // A recovery the server never got: refused, A's standing unchanged; tried again, it goes through.
    w.state().fail_append = Some(false);
    assert!(recover(&a, PASSWORD, "iPhone").await.is_err());
    assert_eq!(a.roster_summary(true).await.unwrap().unwrap().role, PhoneRole::Viewer);
    let recovered = recover(&a, PASSWORD, "iPhone").await.unwrap();
    assert_eq!(recovered.role, PhoneRole::Primary);
    // A refusal (4xx) is final: a create with a wrong master password keeps nothing.
    let mut w2 = world().await;
    let lone = w2.phone().await;
    assert!(lone.roster_create("iPhone".to_owned(), Some("not the password".to_owned())).await.is_err());
    assert!(!lone.engine().holds_owner_seed().await.unwrap());
    assert_eq!(lone.phone_role().await.unwrap(), PhoneRole::Legacy);
}

#[tokio::test]
async fn an_empty_roster_from_the_server_is_never_the_end_of_phone_roles_but_a_deletion_here_is() {
    let mut w = world().await;
    let a = w.phone().await;
    a.roster_create("iPhone".to_owned(), Some(PASSWORD.to_owned())).await.unwrap();
    // The server answers an empty roster: refused (a rollback), and nothing is dropped.
    w.state().empty_roster = true;
    assert!(a.roster_summary(true).await.is_err());
    assert_eq!(a.phone_role().await.unwrap(), PhoneRole::Primary);
    assert!(a.engine().holds_owner_seed().await.unwrap());
    // Nothing irreversible (a new AI connected) runs while the server does not show this phone as primary.
    w.state().pairings.push(json!({"v": 1, "id": "p1", "client_name": "Claude", "client_host": "claude.ai",
        "choices": [12, 47, 83], "created_at": 7}));
    a.sync(0).await.unwrap();
    assert!(a.pending().await.unwrap().iter().any(|p| p.id == "p1"));
    assert!(a.answer_pairing("p1".to_owned(), true, Some(47), None).await.is_err());
    assert!(a.pending().await.unwrap().iter().any(|p| p.id == "p1"), "it still waits");
    // The user deletes the account here: everything phone roles kept for it goes with it.
    w.state().empty_roster = false;
    a.delete_account(EMAIL.to_owned()).await.unwrap();
    assert!(w.state().deleted);
    a.login(w.server.uri(), EMAIL.to_owned(), PASSWORD.to_owned(), None).await.unwrap();
    assert_eq!(a.phone_role().await.unwrap(), PhoneRole::Legacy);
    assert!(!a.engine().holds_owner_seed().await.unwrap());
}

#[tokio::test]
async fn a_mistyped_recovery_code_never_reaches_the_server_and_the_preview_shows_what_to_compare() {
    let mut w = world().await;
    let a = w.phone().await;
    let b = w.phone().await;
    let created = a.roster_create("iPhone".to_owned(), Some(PASSWORD.to_owned())).await.unwrap();
    enroll(&a, &b, "Pixel").await;
    let typo = "ABCD-EFGH-IJKL-MNOP-QRST-UVWX-YZ23-4567-ABCD-EFGH-IJKL-MNOP-QRS";
    assert!(b.roster_recover_preview(typo.to_owned()).await.is_err());
    assert_eq!(w.state().wrap_requests, 0, "no proof was spent on it");
    let preview = b.roster_recover_preview(PASSWORD.to_owned()).await.unwrap();
    assert_eq!(w.state().wrap_requests, 1);
    assert_eq!((preview.roster_epoch, preview.epoch), (2, 3));
    assert_eq!(preview.owner_fingerprint, created.owner_fingerprint, "the same as the primary shows");
}

#[tokio::test]
async fn a_viewer_changes_nothing_but_may_pull_the_brake() {
    use reins_core::AutopilotMode;
    let mut w = world().await;
    let a = w.phone().await;
    let b = w.phone().await;
    // Before phone roles, B starts a bypass of its own (as any phone could).
    b.start_bypass(None, Some(30)).await.unwrap();
    a.roster_create("iPhone".to_owned(), Some(PASSWORD.to_owned())).await.unwrap();
    enroll(&a, &b, "Pixel").await;
    // Every change is the primary's: refused on the viewer before it touches anything.
    assert_eq!(b.start_bypass(None, Some(30)).await, Err(CoreError::ViewerOnly));
    assert_eq!(b.set_autopilot_mode(None, Some(AutopilotMode::Auto), None).await, Err(CoreError::ViewerOnly));
    assert_eq!(b.deny("r1".to_owned()).await, Err(CoreError::ViewerOnly));
    assert_eq!(b.revoke_connection("c1".to_owned()).await, Err(CoreError::ViewerOnly));
    assert_eq!(b.mcp_remove("m1".to_owned()).await, Err(CoreError::ViewerOnly));
    assert_eq!(b.payments_remove_limit("l1".to_owned()).await, Err(CoreError::ViewerOnly));
    assert_eq!(b.delete_account(EMAIL.to_owned()).await, Err(CoreError::ViewerOnly));
    // The brake stays open (Q2): stopping the running bypass, Lockdown, revoking a grant.
    b.set_autopilot_mode(None, None, None).await.unwrap();
    assert_eq!(b.autopilot_settings().await.unwrap().bypass_until, None, "the bypass stopped");
    b.set_autopilot_mode(None, Some(AutopilotMode::Lockdown), None).await.unwrap();
    assert_ne!(b.revoke_grant("g1".to_owned()).await, Err(CoreError::ViewerOnly));
    assert_ne!(b.delete_grant("g1".to_owned()).await, Err(CoreError::ViewerOnly), "a removal only tightens");
    // The primary goes on as before; a removed phone is refused like a viewer.
    a.start_bypass(None, Some(30)).await.unwrap();
    let b_id = device_of(&b).await;
    a.roster_remove_member(b_id).await.unwrap();
    b.roster_summary(true).await.unwrap();
    assert_eq!(b.start_bypass(None, Some(30)).await, Err(CoreError::ViewerOnly));
}
