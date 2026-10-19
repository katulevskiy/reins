//! Phone roles end to end: two phones and a fake server that keeps the roster, the joins and the handovers the way
//! the real one does (it checks every entry with `reins_proto`'s rules), and that can also lie. Covers create, enroll
//! (commit, challenge, reveal, six digits on both phones), signed answers, handover, removal, recovery and rewrap, and
//! the server's lies: a rolled-back or forged roster, a swapped challenge, a replaced recovery wrap.

mod common;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use common::{FakeGoogle, FakeKeys, RecordingNotifier};
use data_encoding::BASE64URL_NOPAD;
use reins_core::autopilot::AutopilotMode;
use reins_core::crypto::{Kdf, master_key, master_password_hash};
use reins_core::permission_types::{
    AccountPermissionPreset, CapabilityPermissionInput, DelegationChange, PermissionCapability, PermissionMode,
    PermissionScopeKind, PermissionScopeSelection,
};
use reins_core::{
    CoreConfig, CoreError, EnrollProgress, GoogleTokenProvider, HandoverProgress, Notifier, PhoneRole, ReinsCore,
};
use reins_proto::answer::{AnswerKind, SignedAnswer, answer_message};
use reins_proto::canon::parse_sig;
use reins_proto::enroll::{EnrollChallenge, EnrollReveal, check_commitment, parse_nonce};
use reins_proto::join::{JoinAnswer, JoinCreated, JoinRequest, JoinState, JoinStatus, NewJoin};
use reins_proto::oplog::{Checkpoint, Heartbeat, OpBatch, OpsAppended, SyncResponse, SyncedOp, Waiting, authorize_op};
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
    /// The next roster append is refused (a definite 409) and not stored.
    refuse_append: bool,
    /// Answer an empty roster (a server that lost or dropped it).
    empty_roster: bool,
    /// Hold the answer to a reveal this long (the reveal itself is taken at once).
    reveal_delay: Option<std::time::Duration>,
    /// Show the primary this name instead of the one the joining phone gave.
    rename_join: Option<String>,
    /// Recovery-wrap requests that reached the server.
    wrap_requests: u32,
    deleted: bool,
    // The op log, kept as the real server does: each op checked against the roster and its author's counter.
    ops: Vec<SyncedOp>,
    counters: HashMap<(String, String), u64>,
    checkpoint: Option<Checkpoint>,
    heartbeat: Option<Heartbeat>,
    /// Ops up to here were dropped (behind the checkpoint).
    floor: u64,
    /// `POST /ops` answers 503 (the phone is offline for its uploads).
    ops_down: bool,
    /// Make sync fail only after the receiving phone fetched its handover bundle (keys may already be installed).
    stall_after_bundle: bool,
    pulls_down: bool,
    /// The next `POST /ops` is refused with this status and message (and nothing of it stored).
    refuse_ops_once: Option<(u16, String)>,
    /// `PUT /checkpoint` is refused, and how many came.
    refuse_checkpoint: bool,
    checkpoint_puts: u32,
    /// Serve this device ops only up to this seq (the heartbeat still signs the real head).
    serve_up_to: Option<(String, u64)>,
    /// Serve the heartbeat with its applied hash changed (its signature no longer verifies).
    forge_heartbeat: bool,
    /// Claim, with the recovery wrap, that the primary wrote this many ops (more than the log holds).
    inflate_counter: Option<u64>,
    /// Serve a recovering phone ops only up to this seq.
    recovery_up_to: Option<u64>,
    /// Serve the checkpoint with a signature that does not verify.
    corrupt_checkpoint: bool,
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
        // The heartbeat served is the primary member's: a new primary (or new keys) has none yet.
        if self.roster.last().is_some_and(|p| !p.primary.same_keys(&entry.primary)) {
            self.heartbeat = None;
        }
        let appended = RosterAppended {
            epoch: entry.epoch,
            hash: entry.hash_hex().unwrap(),
        };
        self.roster.push(entry);
        ok(appended)
    }

    fn post_ops(&mut self, me: &str, batch: OpBatch) -> ResponseTemplate {
        if self.ops_down {
            return err(503, "unavailable");
        }
        if let Some((status, message)) = self.refuse_ops_once.take() {
            return ResponseTemplate::new(status).set_body_json(json!({"error": "op_refused", "message": message}));
        }
        let mut seqs = Vec::new();
        for op in batch.ops {
            if op.author != me {
                return err(403, "op_refused");
            }
            let Ok(signer) = authorize_op(&op, &self.roster) else {
                return err(403, "op_refused");
            };
            let ok_sig = parse_sig(&op.sig)
                .zip(op.signing_bytes().ok())
                .is_some_and(|(sig, msg)| verify(&signer.sign_key().unwrap(), &msg, &sig));
            if !ok_sig {
                return err(403, "bad_signature");
            }
            let last = self.counters.entry((op.author.clone(), signer.sign_pub.clone())).or_insert(0);
            if op.counter <= *last {
                match self.ops.iter().find(|o| o.op == op) {
                    Some(stored) => {
                        seqs.push(stored.seq);
                        continue;
                    }
                    None => return err(409, "counter_replay"),
                }
            }
            *last = op.counter;
            let seq = self.ops.last().map_or(self.floor, |o| o.seq) + 1;
            self.ops.push(SyncedOp {
                seq,
                op,
            });
            seqs.push(seq);
        }
        ok(OpsAppended {
            seqs,
            head_seq: self.head(),
        })
    }

    fn head(&self) -> u64 {
        self.ops.last().map_or(self.floor, |o| o.seq)
    }

    fn sync(&self, req: &Request) -> ResponseTemplate {
        if self.pulls_down {
            return err(503, "offline");
        }
        let after: u64 =
            req.url.query_pairs().find(|(k, _)| k == "after").and_then(|(_, v)| v.parse().ok()).unwrap_or(0);
        let from = after.max(self.floor);
        let me = caller(req);
        let up_to = self.serve_up_to.as_ref().filter(|(device, _)| *device == me).map_or(u64::MAX, |(_, seq)| *seq);
        let ops: Vec<SyncedOp> =
            self.ops.iter().filter(|o| o.seq > from && o.seq <= up_to).take(500).cloned().collect();
        let mut heartbeat = self.heartbeat.clone();
        if self.forge_heartbeat
            && let Some(hb) = &mut heartbeat
        {
            hb.applied_hash = "7".repeat(64);
        }
        ok(SyncResponse {
            epoch: self.roster.last().map_or(0, |e| e.epoch),
            head_seq: self.head(),
            floor_seq: self.floor,
            more: self.ops.iter().filter(|o| o.seq > from && o.seq <= up_to).count() > ops.len(),
            ops,
            checkpoint_seq: self.checkpoint.as_ref().map(|c| c.seq),
            heartbeat,
            min_acked_seq: 0,
            waiting: Waiting::default(),
            answered: Vec::new(),
            cursor: format!("{}-{}", self.roster.len(), self.head()),
        })
    }

    /// Drops the ops the latest checkpoint covers (what the real server does with the older of its two).
    fn drop_behind_checkpoint(&mut self) {
        if let Some(cp) = &self.checkpoint {
            self.floor = cp.seq;
            self.ops.retain(|o| o.seq > cp.seq);
        }
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
            ("POST", ["roster"]) if std::mem::take(&mut self.refuse_append) => err(409, "conflict"),
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
                        primary_counter: None,
                    });
                }
                let primary = &self.roster.last().unwrap().primary;
                let stored = self.counters.get(&(primary.device_id.clone(), primary.sign_pub.clone())).copied();
                ok(RecoveryWrap {
                    epoch: self.roster.last().unwrap().epoch,
                    recovery_wrap: self.stale_wrap.clone().or_else(|| self.wrap.clone()).unwrap(),
                    primary_counter: self.inflate_counter.or(stored),
                })
            }
            ("POST", ["roster", "recovery-log"]) => {
                let r: reins_proto::oplog::RecoveryLogRequest = req.body_json().unwrap();
                if r.master_password_hash != self.password {
                    return err(403, "wrong_proof");
                }
                let from = r.after.max(self.floor);
                let up_to = self.recovery_up_to.unwrap_or(u64::MAX);
                let ops: Vec<SyncedOp> =
                    self.ops.iter().filter(|o| o.seq > from && o.seq <= up_to).take(500).cloned().collect();
                let mut checkpoint = if r.with_checkpoint {
                    self.checkpoint.clone()
                } else {
                    None
                };
                if self.corrupt_checkpoint
                    && let Some(cp) = &mut checkpoint
                {
                    cp.sig = BASE64URL_NOPAD.encode(&[7; 64]);
                }
                ok(reins_proto::oplog::RecoveryLog {
                    checkpoint,
                    heartbeat: self.heartbeat.clone(),
                    floor_seq: self.floor,
                    head_seq: self.head(),
                    more: self.ops.iter().filter(|o| o.seq > from && o.seq <= up_to).count() > ops.len(),
                    ops,
                })
            }
            ("PUT", ["services" | "member"]) | ("POST", ["ack"]) => ResponseTemplate::new(204),
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
            ("GET", ["handover-bundle"]) => {
                if self.stall_after_bundle {
                    self.pulls_down = true;
                }
                match &self.bundle {
                    Some((epoch, sealed)) if self.roster.last().unwrap().primary.device_id == me => {
                        ok(json!({"epoch": epoch, "sealed": sealed}))
                    }
                    _ => err(404, "not_found"),
                }
            }
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
            ("POST", ["ops"]) => self.post_ops(&me, req.body_json().unwrap()),
            ("GET", ["sync"]) => self.sync(req),
            ("GET", ["checkpoint"]) => match &self.checkpoint {
                Some(cp) => ok(cp),
                None => err(404, "not_found"),
            },
            ("PUT", ["checkpoint"]) => {
                self.checkpoint_puts += 1;
                if self.refuse_checkpoint {
                    return err(400, "bad_request");
                }
                let cp: Checkpoint = req.body_json().unwrap();
                let primary = self.roster.last().map(|e| e.primary.device_id.clone());
                if primary.as_deref() != Some(me.as_str()) || cp.seq > self.head() {
                    return err(403, "not_approval_device");
                }
                if self.checkpoint.as_ref().is_some_and(|c| c.seq >= cp.seq) {
                    return err(409, "epoch_conflict");
                }
                self.checkpoint = Some(cp);
                ResponseTemplate::new(204)
            }
            ("POST", ["heartbeat"]) => {
                // Only the current primary's (as the real server stores it on the primary's member row).
                if self.roster.last().is_none_or(|e| e.primary.device_id != me) {
                    return err(403, "not_primary");
                }
                self.heartbeat = Some(req.body_json().unwrap());
                ResponseTemplate::new(204)
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
        self.phone_config(CoreConfig::default()).await
    }

    async fn phone_config(&mut self, config: CoreConfig) -> Arc<ReinsCore> {
        let dir = tempfile::tempdir().unwrap();
        let google: Arc<dyn GoogleTokenProvider> = Arc::new(FakeGoogle::new());
        let notifier: Arc<dyn Notifier> = Arc::new(RecordingNotifier::default());
        let core = ReinsCore::with_config(dir.path().to_str().unwrap(), &FakeKeys, google, notifier, config).unwrap();
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
    core.roster_recover(typed.to_owned(), name.to_owned(), preview.tip, None).await
}

/// Recovers `core`, confirming what the recovery asks to be confirmed (as the user would after reading it): with the
/// token of what it was shown.
async fn recover_confirmed(core: &ReinsCore, typed: &str, name: &str) -> Result<reins_core::RosterSummary, CoreError> {
    let preview = core.roster_recover_preview(typed.to_owned()).await?;
    match core.roster_recover(typed.to_owned(), name.to_owned(), preview.tip.clone(), None).await {
        Err(CoreError::RecoveryNeedsConfirmation {
            token,
            ..
        }) => core.roster_recover(typed.to_owned(), name.to_owned(), preview.tip, Some(token)).await,
        other => other,
    }
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
    // A viewer cannot decline a handover request (its own, or another's): only the primary answers them.
    assert_eq!(b.handover_deny(id.clone()).await, Err(CoreError::ViewerOnly));
    assert_eq!(b.handover_poll().await.unwrap(), HandoverProgress::Waiting);
    a.sync(0).await.unwrap();
    let asks = a.handover_requests().await.unwrap();
    assert_eq!((asks.len(), asks[0].device_name.as_str()), (1, "Pixel"));
    let b_id = device_of(&b).await;
    assert_eq!(asks[0].device_id, b_id);
    a.handover_approve(id, b_id.clone()).await.unwrap();
    // The entry names the outgoing primary's last op: its later ones are a replaced primary's.
    let a_id = device_of(&a).await;
    let last = stored_ops(&w, &a_id).iter().map(|(_, counter)| *counter).max().unwrap_or(0);
    assert_eq!(w.state().roster.last().unwrap().primary_counter, Some(last));
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
    let recovered = recover_confirmed(&a, PASSWORD, "iPhone").await.unwrap();
    assert_eq!((recovered.epoch, recovered.role), (5, PhoneRole::Primary));
    assert_ne!(recovered.owner_fingerprint, b.roster_summary(false).await.unwrap().unwrap().owner_fingerprint);
    b.roster_summary(true).await.unwrap();
    assert_eq!(b.phone_role().await.unwrap(), PhoneRole::Removed);
    // B's late answer is refused: it is not the primary now.
    let entries = w.state().roster.clone();
    assert_eq!(entries.last().unwrap().reason, RosterReason::Recovery);
    let last = stored_ops(&w, &b_id).iter().map(|(_, counter)| *counter).max().unwrap_or(0);
    assert_eq!(entries.last().unwrap().primary_counter, Some(last), "recovery includes the new primary's sync ops");

    // A rewraps for the (same) password; a mistyped one is refused before anything is sealed.
    assert!(a.roster_rewrap("not it".to_owned()).await.is_err());
    a.roster_rewrap(PASSWORD.to_owned()).await.unwrap();
    assert_eq!(w.state().roster.last().unwrap().reason, RosterReason::Rewrap);
    // And recovering again from the new wrap still works (B, with the password), with nothing to confirm once the
    // lost phone's status covers what it wrote.
    a.live_sync(0).await.unwrap();
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
async fn managed_handover_requires_the_viewers_signed_policy_support_before_keys_move() {
    let mut w = world().await;
    let gmail = MockServer::start().await;
    let a = permission_phone(&mut w, &gmail).await;
    a.roster_create("iPhone".into(), Some(PASSWORD.into())).await.unwrap();
    let b = w.phone().await;
    enroll(&a, &b, "Pixel").await;
    let device = device_of(&b).await;
    let id = b.handover_request().await.unwrap();
    // An old viewer or old server cannot supply/retain this optional signed proof.
    let proof = w.state().handovers.get_mut(&id).unwrap().request.signed.as_mut().unwrap().policy_enforcement.take();
    assert!(proof.is_some());
    a.sync(0).await.unwrap();
    let error = a.handover_approve(id.clone(), device.clone()).await.unwrap_err();
    assert!(error.to_string().contains("policy support"), "{error}");
    assert_eq!(a.phone_role().await.unwrap(), PhoneRole::Primary);
    assert_eq!(b.phone_role().await.unwrap(), PhoneRole::Viewer);
    assert!(w.state().bundle.is_none(), "no owner keys or provider secrets were transferred");
    // A capable viewer's exact authenticated proof permits the ordinary transfer.
    w.state().handovers.get_mut(&id).unwrap().request.signed.as_mut().unwrap().policy_enforcement = proof;
    a.handover_approve(id, device).await.unwrap();
    assert_eq!(b.handover_poll().await.unwrap(), HandoverProgress::Done);
    assert_eq!(b.phone_role().await.unwrap(), PhoneRole::Primary);
}

#[tokio::test]
async fn failed_remote_revocation_still_requires_policy_support_at_handover() {
    let mut w = world().await;
    let a = w.phone().await;
    let b = w.phone().await;
    a.roster_create("iPhone".into(), Some(PASSWORD.into())).await.unwrap();
    enroll(&a, &b, "Pixel").await;
    Mock::given(method("DELETE"))
        .and(path("/reins/api/connections/c1"))
        .respond_with(ResponseTemplate::new(500))
        .with_priority(1)
        .mount(&w.server)
        .await;
    let error = a.revoke_connection("c1".into()).await.unwrap_err();
    assert!(error.to_string().contains("blocked on this phone"), "{error}");
    // No account permission was installed. The retained local revocation is the only new brake.
    let id = b.handover_request().await.unwrap();
    let proof = w.state().handovers.get_mut(&id).unwrap().request.signed.as_mut().unwrap().policy_enforcement.take();
    let device = device_of(&b).await;
    let error = a.handover_approve(id.clone(), device.clone()).await.unwrap_err();
    assert!(error.to_string().contains("policy support"), "{error}");
    assert_eq!(a.phone_role().await.unwrap(), PhoneRole::Primary);
    assert!(w.state().bundle.is_none(), "a failed remote revoke cannot reopen access through an old phone");
    w.state().handovers.get_mut(&id).unwrap().request.signed.as_mut().unwrap().policy_enforcement = proof;
    a.handover_approve(id, device).await.unwrap();
    assert_eq!(b.handover_poll().await.unwrap(), HandoverProgress::Done);
}

#[tokio::test]
async fn unmanaged_handover_keeps_the_legacy_signature_compatible() {
    let mut w = world().await;
    let a = w.phone().await;
    let b = w.phone().await;
    a.roster_create("iPhone".into(), Some(PASSWORD.into())).await.unwrap();
    enroll(&a, &b, "Pixel").await;
    let id = b.handover_request().await.unwrap();
    w.state().handovers.get_mut(&id).unwrap().request.signed.as_mut().unwrap().policy_enforcement = None;
    a.handover_approve(id, device_of(&b).await).await.unwrap();
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
    let refused = b.roster_recover(PASSWORD.to_owned(), "Pixel".to_owned(), preview.tip, None).await.unwrap_err();
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
    // A recovery the server refused for good (a 4xx): nothing of it stays, not the old owner's view keys nor the
    // wrap key.
    let held = a.engine().held_roles_keys().await.unwrap();
    w.state().refuse_append = true;
    assert!(recover_confirmed(&a, PASSWORD, "iPhone").await.is_err());
    assert_eq!(a.engine().held_roles_keys().await.unwrap(), held);
    assert_eq!(a.roster_summary(true).await.unwrap().unwrap().role, PhoneRole::Viewer);
    // A recovery the server never got: refused, A's standing unchanged; tried again, it goes through.
    w.state().fail_append = Some(false);
    assert!(recover_confirmed(&a, PASSWORD, "iPhone").await.is_err());
    assert_eq!(a.roster_summary(true).await.unwrap().unwrap().role, PhoneRole::Viewer);
    let recovered = recover_confirmed(&a, PASSWORD, "iPhone").await.unwrap();
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
    assert!(a.answer_pairing("p1".to_owned(), true, Some(47), None, false).await.is_err());
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

/// Naming a connection is the primary's, like any other change to it: a viewer is refused before anything is kept.
#[tokio::test]
async fn only_the_primary_renames_a_connection() {
    let mut w = world().await;
    let a = w.phone().await;
    let b = w.phone().await;
    a.roster_create("iPhone".to_owned(), Some(PASSWORD.to_owned())).await.unwrap();
    enroll(&a, &b, "Pixel").await;
    assert_eq!(b.rename_connection("c1".to_owned(), Some("Work".to_owned())).await, Err(CoreError::ViewerOnly));
    assert_eq!(b.rename_connection("c1".to_owned(), None).await, Err(CoreError::ViewerOnly), "nor take a name away");
    assert_eq!(a.rename_connection("c1".to_owned(), Some("Work".to_owned())).await, Ok(Some("Work".to_owned())));
}

// ---- the live sync (stage 5) ---------------------------------------------------------------------------------------

/// The ops the server stored, by author: (class, counter).
fn stored_ops(w: &World, author: &str) -> Vec<(reins_proto::oplog::OpClass, u64)> {
    w.state().ops.iter().filter(|o| o.op.author == author).map(|o| (o.op.class, o.op.counter)).collect()
}

#[tokio::test]
async fn the_primarys_changes_reach_the_viewer_and_the_viewers_brake_reaches_the_primary() {
    let mut w = world().await;
    let a = w.phone().await;
    let b = w.phone().await;
    a.roster_create("iPhone".to_owned(), Some(PASSWORD.to_owned())).await.unwrap();
    enroll(&a, &b, "Pixel").await;
    // The primary runs a bypass: it goes out with the next cycle, and the viewer sees it.
    a.start_bypass(None, Some(60)).await.unwrap();
    a.live_sync(0).await.unwrap();
    let state = b.live_sync(0).await.unwrap();
    assert_eq!(state.role, PhoneRole::Viewer);
    assert!(state.forked.is_none() && !state.waiting_for_key && !state.disagrees, "{state:?}");
    assert!(b.autopilot_settings().await.unwrap().bypass_until.is_some(), "the viewer sees the bypass");
    assert!(state.live, "the primary's heartbeat is fresh and matches: {state:?}");
    assert_eq!(state.applied_seq, state.head_seq);
    // The viewer pulls the brake: Lockdown at once on it, and on the primary with its next cycle.
    b.set_autopilot_mode(None, Some(AutopilotMode::Lockdown), None).await.unwrap();
    let b_id = device_of(&b).await;
    assert!(
        stored_ops(&w, &b_id).iter().all(|(class, _)| *class == reins_proto::oplog::OpClass::Restrictive),
        "a viewer sends only the brake"
    );
    a.live_sync(0).await.unwrap();
    let settings = a.autopilot_settings().await.unwrap();
    assert_eq!((settings.mode, settings.bypass_until), (AutopilotMode::Lockdown, None));
    // A viewer changes nothing else, and a change it makes locally never goes out.
    assert_eq!(b.start_bypass(None, Some(30)).await, Err(CoreError::ViewerOnly));
    let before = stored_ops(&w, &b_id).len();
    b.live_sync(0).await.unwrap();
    assert_eq!(stored_ops(&w, &b_id).len(), before, "nothing new from the viewer");
}

#[tokio::test]
async fn after_a_removal_the_viewers_left_get_the_new_key_and_the_removed_one_nothing() {
    let mut w = world().await;
    let a = w.phone().await;
    let b = w.phone().await;
    let c = w.phone().await;
    a.roster_create("iPhone".to_owned(), Some(PASSWORD.to_owned())).await.unwrap();
    enroll(&a, &b, "Pixel").await;
    enroll(&a, &c, "Tablet").await;
    a.start_bypass(None, Some(60)).await.unwrap();
    a.live_sync(0).await.unwrap();
    for viewer in [&b, &c] {
        viewer.live_sync(0).await.unwrap();
        assert!(viewer.autopilot_settings().await.unwrap().bypass_until.is_some());
    }
    // C is removed: the view key rotates, and what the primary writes next is sealed with the new one.
    let c_id = device_of(&c).await;
    a.roster_remove_member(c_id).await.unwrap();
    a.set_autopilot_mode(None, Some(AutopilotMode::Lockdown), None).await.unwrap();
    a.live_sync(0).await.unwrap();
    // B opens the grant the log brought it (sealed under the key it had) and reads on.
    let state = b.live_sync(0).await.unwrap();
    assert!(!state.waiting_for_key && state.forked.is_none(), "{state:?}");
    assert_eq!(b.autopilot_settings().await.unwrap().mode, AutopilotMode::Lockdown);
    // C was given no key: it reads nothing written since.
    let state = c.live_sync(0).await.unwrap();
    assert!(state.waiting_for_key, "{state:?}");
    assert_ne!(c.autopilot_settings().await.unwrap().mode, AutopilotMode::Lockdown);
}

#[tokio::test]
async fn a_phone_enrolled_later_starts_from_the_checkpoint_at_its_epoch() {
    let mut w = world().await;
    let a = w.phone().await;
    let b = w.phone().await;
    let c = w.phone().await;
    a.roster_create("iPhone".to_owned(), Some(PASSWORD.to_owned())).await.unwrap();
    // Written before any viewer: ops at an epoch a phone enrolled later cannot verify.
    a.start_bypass(None, Some(60)).await.unwrap();
    a.live_sync(0).await.unwrap();
    enroll(&a, &b, "Pixel").await;
    // The primary marks the new epoch and writes a checkpoint at it.
    a.live_sync(0).await.unwrap();
    let cp_epoch = w.state().checkpoint.as_ref().map(|cp| cp.epoch);
    assert_eq!(cp_epoch, Some(2));
    let state = b.live_sync(0).await.unwrap();
    assert!(state.forked.is_none() && state.notice.is_none(), "{state:?}");
    assert!(b.autopilot_settings().await.unwrap().bypass_until.is_some(), "from the checkpoint");
    // Once the server dropped the ops behind it, a phone enrolled after that starts from it too.
    enroll(&a, &c, "Tablet").await;
    a.live_sync(0).await.unwrap();
    w.state().drop_behind_checkpoint();
    let state = c.live_sync(0).await.unwrap();
    assert!(state.forked.is_none() && state.notice.is_none(), "{state:?}");
    assert!(c.autopilot_settings().await.unwrap().bypass_until.is_some());
    assert_eq!(state.applied_seq, state.head_seq);
}

#[tokio::test]
async fn a_handover_waits_for_the_primarys_queued_changes_and_names_them() {
    let mut w = world().await;
    let a = w.phone().await;
    let b = w.phone().await;
    a.roster_create("iPhone".to_owned(), Some(PASSWORD.to_owned())).await.unwrap();
    enroll(&a, &b, "Pixel").await;
    a.live_sync(0).await.unwrap();
    // A change the server never got: queued, numbered, signed.
    w.state().ops_down = true;
    a.start_bypass(None, Some(60)).await.unwrap();
    let state = a.live_sync(0).await.unwrap();
    assert!(state.queued > 0 && state.notice.is_some(), "{state:?}");
    let id = b.handover_request().await.unwrap();
    a.sync(0).await.unwrap();
    let b_id = device_of(&b).await;
    assert!(a.handover_approve(id.clone(), b_id.clone()).await.is_err(), "not before its changes are in the log");
    assert_eq!(a.phone_role().await.unwrap(), PhoneRole::Primary);
    // Online again: the changes go first, and the handover names the last of them.
    w.state().ops_down = false;
    a.handover_approve(id, b_id).await.unwrap();
    let a_id = device_of(&a).await;
    let last = stored_ops(&w, &a_id).iter().map(|(_, counter)| *counter).max().unwrap();
    assert_eq!(w.state().roster.last().unwrap().primary_counter, Some(last));
    assert_eq!(b.handover_poll().await.unwrap(), HandoverProgress::Done);
    b.live_sync(0).await.unwrap();
    assert!(
        b.autopilot_settings().await.unwrap().bypass_until.is_some(),
        "the new primary has the old one's last change"
    );
    // The old primary sends nothing as primary any more.
    let state = a.live_sync(0).await.unwrap();
    assert_eq!((state.role, state.queued), (PhoneRole::Viewer, 0));
}

#[tokio::test]
async fn a_viewer_proposes_the_primary_keeps_and_tokens_never_reach_a_viewer() {
    let mut w = world().await;
    let a = w.phone().await;
    let b = w.phone().await;
    // B held a token before phone roles; as a viewer it keeps none.
    b.engine().keep_token("github", "old", b"ghp_old").unwrap();
    a.roster_create("iPhone".to_owned(), Some(PASSWORD.to_owned())).await.unwrap();
    enroll(&a, &b, "Pixel").await;
    b.live_sync(0).await.unwrap();
    assert!(!b.engine().holds_token("github", "old").unwrap(), "a viewer forgets its tokens");
    // B proposes a GitHub account with its token; never a grant or a setting.
    let rows = json!([
        {"table": "accounts", "value": {"service": "github", "account": "octo"}},
        {"table": "secrets", "value": {"service": "github", "account": "octo",
            "value": {"$b64": BASE64URL_NOPAD.encode(b"ghp_new")}}},
    ]);
    let forbidden = json!([{"table": "grants", "value": {"id": "g1"}}]);
    assert!(b.proposal_create("github".into(), "x".into(), forbidden.to_string()).await.is_err());
    assert!(
        a.proposal_create("github".into(), "x".into(), rows.to_string()).await.is_err(),
        "the primary adds it itself"
    );
    let id = b.proposal_create("github".into(), "GitHub · octo".into(), rows.to_string()).await.unwrap();
    a.live_sync(0).await.unwrap();
    let waiting = a.proposals().await.unwrap();
    assert_eq!(waiting.len(), 1);
    assert_eq!(
        (waiting[0].id.as_str(), waiting[0].author_name.as_str(), waiting[0].mine),
        (id.as_str(), "Pixel", false)
    );
    assert!(b.proposal_keep(id.clone(), String::new()).await.is_err(), "a viewer keeps nothing");
    assert!(b.proposal_preview(id.clone()).await.is_err(), "nor opens one");
    // The primary keeps it, as previewed: the account and its token are its own now, and the token never reaches the
    // viewer.
    let preview = a.proposal_preview(id.clone()).await.unwrap();
    assert_eq!(preview.refused, None);
    assert_eq!(preview.rows.len(), 2);
    assert!(preview.rows.iter().all(|r| !r.replaces && r.summary == "github · octo"), "{:?}", preview.rows);
    assert!(a.proposal_keep(id.clone(), "0".repeat(64)).await.is_err(), "not what was shown");
    a.proposal_keep(id, preview.digest).await.unwrap();
    assert!(a.engine().holds_token("github", "octo").unwrap());
    b.live_sync(0).await.unwrap();
    assert!(b.proposals().await.unwrap().is_empty(), "resolved");
    assert!(!b.engine().holds_token("github", "octo").unwrap(), "a viewer never holds a token");
    // Another, discarded: nothing of it is added.
    let mcp = json!([{"table": "mcp_servers", "value": {"id": "m1", "name": "Notes", "url": "https://notes.example",
        "auth": "none"}}]);
    let other = b.proposal_create("mcp".into(), "Notes".into(), mcp.to_string()).await.unwrap();
    a.live_sync(0).await.unwrap();
    a.proposal_discard(other).await.unwrap();
    b.live_sync(0).await.unwrap();
    assert!(b.proposals().await.unwrap().is_empty());
    // The primary sees its own resolution once the log brings it back (not before: no background sync is waited for).
    a.live_sync(0).await.unwrap();
    assert!(a.proposals().await.unwrap().is_empty());
}

// ---- the live sync, security review --------------------------------------------------------------------------------

#[tokio::test]
async fn a_fresh_phone_that_recovers_is_not_stuck_when_the_log_before_it_was_dropped() {
    let mut w = world().await;
    let a = w.phone().await;
    let b = w.phone().await;
    a.roster_create("iPhone".to_owned(), Some(PASSWORD.to_owned())).await.unwrap();
    enroll(&a, &b, "Pixel").await;
    a.start_bypass(None, Some(60)).await.unwrap();
    a.live_sync(0).await.unwrap();
    a.live_sync(0).await.unwrap();
    w.state().drop_behind_checkpoint();
    // The primary is lost; a fresh phone recovers with the code.
    let c = w.phone().await;
    recover(&c, PASSWORD, "New iPhone").await.unwrap();
    let state = c.live_sync(0).await.unwrap();
    assert_eq!(state.role, PhoneRole::Primary);
    assert!(state.notice.is_none() && state.forked.is_none(), "{state:?}");
    assert_eq!(state.applied_seq, state.head_seq);
    // It started from a checkpoint it cannot vouch for (a thief could have put it just before): nothing it brought
    // loosens anything. The bypass is off and the mode Manual, until the user turns them on again.
    let settings = c.autopilot_settings().await.unwrap();
    assert_eq!((settings.bypass_until, settings.base_mode), (None, AutopilotMode::Manual));
    // And the viewer goes on with it, tightened too.
    let state = b.live_sync(0).await.unwrap();
    assert!(state.forked.is_none() && !state.waiting_for_key, "{state:?}");
    assert_eq!(b.autopilot_settings().await.unwrap().bypass_until, None);
}

#[tokio::test]
async fn a_viewers_lockdown_lands_on_a_fresh_manual_account_and_after_a_bypass_ended() {
    let mut w = world().await;
    let a = w.phone().await;
    let b = w.phone().await;
    a.roster_create("iPhone".to_owned(), Some(PASSWORD.to_owned())).await.unwrap();
    enroll(&a, &b, "Pixel").await;
    a.live_sync(0).await.unwrap();
    b.live_sync(0).await.unwrap();
    // No global setting anywhere yet (Manual): the viewer's Lockdown makes one.
    b.set_autopilot_mode(None, Some(AutopilotMode::Lockdown), None).await.unwrap();
    let state = a.live_sync(0).await.unwrap();
    assert!(state.notice.is_none(), "{state:?}");
    assert_eq!(a.autopilot_settings().await.unwrap().mode, AutopilotMode::Lockdown);
    // Lifted by the primary, a bypass run and stopped; Lockdown again from the viewer lands too.
    a.set_autopilot_mode(None, Some(AutopilotMode::Manual), None).await.unwrap();
    a.start_bypass(None, Some(60)).await.unwrap();
    a.set_autopilot_mode(None, Some(AutopilotMode::Manual), None).await.unwrap();
    a.live_sync(0).await.unwrap();
    b.live_sync(0).await.unwrap();
    assert_ne!(b.autopilot_settings().await.unwrap().mode, AutopilotMode::Lockdown);
    b.set_autopilot_mode(None, Some(AutopilotMode::Lockdown), None).await.unwrap();
    let state = a.live_sync(0).await.unwrap();
    assert!(state.notice.is_none(), "{state:?}");
    assert_eq!(a.autopilot_settings().await.unwrap().mode, AutopilotMode::Lockdown);
}

#[tokio::test]
async fn the_primary_keeps_only_what_it_previewed_and_only_what_a_proposal_may_add() {
    let mut w = world().await;
    let a = w.phone().await;
    let b = w.phone().await;
    a.roster_create("iPhone".to_owned(), Some(PASSWORD.to_owned())).await.unwrap();
    enroll(&a, &b, "Pixel").await;
    a.live_sync(0).await.unwrap();
    let token = |service: &str, account: &str| {
        json!({"table": "secrets", "value": {"service": service, "account": account,
            "value": {"$b64": BASE64URL_NOPAD.encode(b"t")}}})
    };
    let account =
        |service: &str, account: &str| json!({"table": "accounts", "value": {"service": service, "account": account}});
    // Refused when proposed: a key of payments or of Reins, a token without its account, rows the kind does not
    // name, columns a proposal may not set, an MCP server over plain http.
    for (kind, rows) in [
        ("github", json!([account("github", "octo"), token("payments.mandate-key", "")])),
        ("github", json!([account("github", "octo"), token("reins.sign-key", "")])),
        ("github", json!([token("github", "octo")])),
        ("gitlab", json!([account("github", "octo"), token("github", "octo")])),
        ("github", json!([{"table": "accounts", "value": {"service": "github", "account": "octo", "added_at": 1}}])),
        (
            "mcp",
            json!([{"table": "mcp_servers", "value": {"id": "m1", "name": "N", "url": "https://n.example", "auth": "none",
            "tools": "[{\"name\":\"delete_all\",\"read_only\":true}]"}}]),
        ),
        (
            "mcp",
            json!([{"table": "mcp_servers", "value": {"id": "m1", "name": "N", "url": "http://n.example", "auth": "none"}}]),
        ),
    ] {
        assert!(b.proposal_create(kind.into(), "x".into(), rows.to_string()).await.is_err(), "{kind} {rows}");
    }
    // An MCP server with its token: Keep writes it, its tools listed afresh by the primary.
    let mcp = json!([
        {"table": "mcp_servers", "value": {"id": "m1", "name": "Notes", "url": "https://notes.example", "auth": "token"}},
        token("mcp", "m1"),
    ]);
    let id = b.proposal_create("mcp".into(), "Totally harmless".into(), mcp.to_string()).await.unwrap();
    a.live_sync(0).await.unwrap();
    let preview = a.proposal_preview(id.clone()).await.unwrap();
    assert_eq!(preview.refused, None);
    assert!(preview.rows.iter().any(|r| r.table == "mcp_servers" && r.summary == "Notes · https://notes.example"));
    // The same again: it would replace what the primary has now, so Keep refuses it.
    a.proposal_keep(id, preview.digest).await.unwrap();
    let again = b.proposal_create("mcp".into(), "Notes".into(), mcp.to_string()).await.unwrap();
    a.live_sync(0).await.unwrap();
    let preview = a.proposal_preview(again.clone()).await.unwrap();
    assert!(preview.rows.iter().any(|r| r.replaces) && preview.refused.is_some(), "{preview:?}");
    assert!(a.proposal_keep(again, preview.digest).await.is_err());
}

#[tokio::test]
async fn live_comes_only_from_a_verified_heartbeat_that_matches_what_this_phone_applied() {
    let mut w = world().await;
    let a = w.phone().await;
    let b = w.phone().await;
    a.roster_create("iPhone".to_owned(), Some(PASSWORD.to_owned())).await.unwrap();
    enroll(&a, &b, "Pixel").await;
    a.start_bypass(None, Some(60)).await.unwrap();
    a.live_sync(0).await.unwrap();
    // The primary signed a head this phone (which has applied nothing yet) is not given: not live, and the signed
    // head is shown.
    let signed = w.state().heartbeat.as_ref().unwrap().head_seq;
    assert!(signed > 0);
    let b_id = device_of(&b).await;
    w.state().serve_up_to = Some((b_id, signed - 1));
    let state = b.live_sync(0).await.unwrap();
    assert!(signed > state.applied_seq, "{signed} {state:?}");
    assert!(!state.live, "{state:?}");
    assert_eq!(state.primary_head_seq, Some(signed));
    assert_eq!(state.counter_gaps, 0);
    // Given everything: live.
    w.state().serve_up_to = None;
    assert!(b.live_sync(0).await.unwrap().live);
    // A heartbeat that does not verify: not live, and no warning made of it.
    w.state().forge_heartbeat = true;
    let state = b.live_sync(0).await.unwrap();
    assert!(!state.live && !state.disagrees && state.primary_head_seq.is_none(), "{state:?}");
    w.state().forge_heartbeat = false;
    assert!(b.live_sync(0).await.unwrap().live);
}

#[tokio::test]
async fn a_refused_checkpoint_is_not_rebuilt_every_cycle() {
    let mut w = world().await;
    let a = w.phone().await;
    a.roster_create("iPhone".to_owned(), Some(PASSWORD.to_owned())).await.unwrap();
    w.state().refuse_checkpoint = true;
    a.start_bypass(None, Some(60)).await.unwrap();
    a.live_sync(0).await.unwrap();
    let tries = w.state().checkpoint_puts;
    assert!(tries >= 1);
    a.set_autopilot_mode(None, Some(AutopilotMode::Manual), None).await.unwrap();
    a.live_sync(0).await.unwrap();
    a.live_sync(0).await.unwrap();
    assert_eq!(w.state().checkpoint_puts, tries, "backed off after the refusal");
}

#[tokio::test]
async fn a_refused_brake_is_told_and_the_ones_after_it_still_arrive_without_a_gap() {
    let mut w = world().await;
    let a = w.phone().await;
    let b = w.phone().await;
    a.roster_create("iPhone".to_owned(), Some(PASSWORD.to_owned())).await.unwrap();
    enroll(&a, &b, "Pixel").await;
    a.start_bypass(None, Some(60)).await.unwrap();
    a.live_sync(0).await.unwrap();
    b.live_sync(0).await.unwrap();
    let b_id = device_of(&b).await;
    // A brake held back with a 409 is kept, and goes with a later sync.
    w.state().ops_down = true;
    b.set_autopilot_mode(None, Some(AutopilotMode::Lockdown), None).await.unwrap();
    {
        let mut st = w.state();
        st.refuse_ops_once = Some((409, "counter_replay".to_owned()));
        st.ops_down = false;
    }
    for _ in 0..3 {
        if b.live_sync(0).await.unwrap().queued == 0 {
            break;
        }
    }
    assert_eq!(stored_ops(&w, &b_id).len(), 1, "the held-back brake arrived");
    // Two brakes queued while the server takes none; the first is then refused for good: told, never silent, and
    // the other renumbered, so it lands with no gap.
    w.state().ops_down = true;
    b.set_autopilot_mode(None, Some(AutopilotMode::Lockdown), None).await.unwrap();
    b.set_autopilot_mode(None, Some(AutopilotMode::Lockdown), None).await.unwrap();
    {
        let mut st = w.state();
        st.refuse_ops_once = Some((403, "op 0: not allowed".to_owned()));
        st.ops_down = false;
    }
    let state = b.live_sync(0).await.unwrap();
    assert!(state.notice.as_deref().is_some_and(|n| n.contains("refused")), "{state:?}");
    for _ in 0..3 {
        if b.live_sync(0).await.unwrap().queued == 0 {
            break;
        }
    }
    assert_eq!(stored_ops(&w, &b_id).len(), 2, "the brake after the refused one arrived");
    let state = a.live_sync(0).await.unwrap();
    assert_eq!(state.counter_gaps, 0, "{state:?}");
    assert_eq!(a.autopilot_settings().await.unwrap().mode, AutopilotMode::Lockdown);
}

#[tokio::test]
async fn a_viewers_leftover_rows_go_once_it_takes_the_log() {
    let mut w = world().await;
    let a = w.phone().await;
    let b = w.phone().await;
    // B connected an account before phone roles; the primary never had it.
    b.add_service_account("payments".into(), String::new()).await.unwrap();
    assert!(b.accounts().await.unwrap().iter().any(|v| v.service == "payments"));
    a.roster_create("iPhone".to_owned(), Some(PASSWORD.to_owned())).await.unwrap();
    enroll(&a, &b, "Pixel").await;
    a.start_bypass(None, Some(60)).await.unwrap();
    a.live_sync(0).await.unwrap();
    let state = b.live_sync(0).await.unwrap();
    assert!(state.notice.is_none(), "{state:?}");
    assert!(!b.accounts().await.unwrap().iter().any(|v| v.service == "payments"), "the leftover is gone");
    // Handed over, B publishes nothing of it.
    let id = b.handover_request().await.unwrap();
    a.sync(0).await.unwrap();
    a.handover_approve(id, device_of(&b).await).await.unwrap();
    assert_eq!(b.handover_poll().await.unwrap(), HandoverProgress::Done);
    b.live_sync(0).await.unwrap();
    a.live_sync(0).await.unwrap();
    assert!(!a.accounts().await.unwrap().iter().any(|v| v.service == "payments"));
}

// ---- the live sync, security review round 2 ------------------------------------------------------------------------

#[tokio::test]
async fn a_recovery_names_only_what_it_verified_and_asks_when_a_server_claims_more() {
    let mut w = world().await;
    let a = w.phone().await;
    let b = w.phone().await;
    a.roster_create("iPhone".to_owned(), Some(PASSWORD.to_owned())).await.unwrap();
    enroll(&a, &b, "Pixel").await;
    a.start_bypass(None, Some(60)).await.unwrap();
    a.live_sync(0).await.unwrap();
    let a_id = device_of(&a).await;
    let stored = stored_ops(&w, &a_id).iter().map(|(_, counter)| *counter).max().unwrap();
    // The server claims the lost primary wrote more than it serves: the user is asked, nothing signed before.
    w.state().inflate_counter = Some(stored + 50);
    let c = w.phone().await;
    let epoch = w.state().roster.last().unwrap().epoch;
    let refused = recover(&c, PASSWORD, "New iPhone").await.unwrap_err();
    assert!(matches!(refused, CoreError::RecoveryNeedsConfirmation { .. }), "{refused:?}");
    assert!(refused.to_string().contains("shows"), "{refused}");
    assert_eq!(w.state().roster.last().unwrap().epoch, epoch, "nothing signed");
    // Honest: the fresh phone reads the log with its proof, verifies it, and names exactly what it verified.
    w.state().inflate_counter = None;
    recover(&c, PASSWORD, "New iPhone").await.unwrap();
    let entry = w.state().roster.last().unwrap().clone();
    assert_eq!((entry.reason, entry.primary_counter), (RosterReason::Recovery, Some(stored)));
    // The viewer takes the recovery (nothing it has not seen is vouched for).
    let state = b.live_sync(0).await.unwrap();
    assert_eq!((state.epoch, state.forked.is_none()), (entry.epoch, true), "{state:?}");
}

#[tokio::test]
async fn renumbered_activity_and_view_key_grants_still_reach_the_viewer() {
    let mut w = world().await;
    let a = w.phone().await;
    let b = w.phone().await;
    let c = w.phone().await;
    a.roster_create("iPhone".to_owned(), Some(PASSWORD.to_owned())).await.unwrap();
    enroll(&a, &b, "Pixel").await;
    enroll(&a, &c, "Tablet").await;
    a.live_sync(0).await.unwrap();
    b.live_sync(0).await.unwrap();
    // Offline: a change, then C removed (the view key rotates: a grant for B is queued), then a decision (an
    // Activity entry).
    w.state().ops_down = true;
    a.start_bypass(None, Some(60)).await.unwrap();
    a.live_sync(0).await.unwrap();
    a.roster_remove_member(device_of(&c).await).await.unwrap();
    let request = json!({"v": 1, "id": "r1", "connection_id": "c1", "connection_label": "Claude", "created_at": 100,
        "call": {"tool": "gmail_search", "query": "from:bank", "max_results": 1}});
    w.state().requests.push(request);
    // The running bypass decides it: an Activity entry.
    a.sync(0).await.unwrap();
    let decided = a.activity(50).await.unwrap();
    assert!(!decided.is_empty());
    a.live_sync(0).await.unwrap();
    // Back online, the first queued op is refused: everything after it is renumbered.
    {
        let mut st = w.state();
        st.refuse_ops_once = Some((403, "op 0: not allowed".to_owned()));
        st.ops_down = false;
    }
    for _ in 0..4 {
        if a.live_sync(0).await.unwrap().queued == 0 {
            break;
        }
    }
    // B opens its grant (it went under the key B holds) and gets the Activity entry under its new key.
    let state = b.live_sync(0).await.unwrap();
    assert!(!state.waiting_for_key && state.forked.is_none(), "{state:?}");
    let seen = b.activity(50).await.unwrap();
    for entry in &decided {
        assert!(
            seen.iter().any(|e| e.action == entry.action && e.outcome == entry.outcome && e.at == entry.at),
            "{entry:?} not in {seen:?}"
        );
    }
    assert!(b.autopilot_settings().await.unwrap().bypass_until.is_some());
}

#[tokio::test]
async fn a_handover_waits_for_rows_too_large_to_send_and_a_demoted_phone_keeps_them() {
    let mut w = world().await;
    let a = w.phone().await;
    let b = w.phone().await;
    a.roster_create("iPhone".to_owned(), Some(PASSWORD.to_owned())).await.unwrap();
    enroll(&a, &b, "Pixel").await;
    // An MCP server whose tool list is too large for any op: it stays on the primary only.
    let tools = json!([{"name": "x".repeat(80 * 1024)}]).to_string();
    let server = json!({"id": "m-big", "name": "Big", "url": "https://big.example", "auth": "none", "tools": tools,
        "heavy": "[]", "status": "ok", "added_at": 1});
    a.engine().keep_sync_row("mcp_servers", &server.to_string()).unwrap();
    a.live_sync(0).await.unwrap();
    // The handover would leave it behind: refused, and said why.
    let id = b.handover_request().await.unwrap();
    a.sync(0).await.unwrap();
    let refused = a.handover_approve(id.clone(), device_of(&b).await).await.unwrap_err();
    assert!(refused.to_string().contains("too large"), "{refused}");
    // Once the row is gone, the handover is no longer held back by it.
    a.engine()
        .keep_sync_row(
            "mcp_servers",
            &json!({"id": "m-small", "name": "S", "url": "https://s.example",
        "auth": "none", "tools": "[]", "heavy": "[]", "status": "ok", "added_at": 1})
            .to_string(),
        )
        .unwrap();
    a.mcp_remove("m-big".to_owned()).await.unwrap();
    a.live_sync(0).await.unwrap();
    assert!(a.engine().has_sync_row("mcp_servers", "m-small").unwrap());
    a.engine().keep_sync_row("mcp_servers", &server.to_string()).unwrap();
    a.live_sync(0).await.unwrap();
    // B recovers instead (A is demoted without a say): A keeps the row, never deleted as a leftover.
    recover(&b, PASSWORD, "Pixel").await.unwrap();
    let state = a.live_sync(0).await.unwrap();
    assert_eq!(state.role, PhoneRole::Removed);
    assert!(a.engine().has_sync_row("mcp_servers", "m-big").unwrap());
}

// ---- the live sync, security review round 3 ------------------------------------------------------------------------

/// Takes the op at `index` of the log out and closes the seq gap (what a server that drops an op shows).
fn drop_op(w: &World, index: usize) {
    let mut st = w.state();
    st.ops.remove(index);
    for o in st.ops.iter_mut().skip(index) {
        o.seq -= 1;
    }
}

#[tokio::test]
async fn a_recovery_starts_from_the_primarys_earlier_checkpoint_when_no_newer_one_exists() {
    let mut w = world().await;
    let a = w.phone().await;
    let b = w.phone().await;
    let c = w.phone().await;
    a.roster_create("iPhone".to_owned(), Some(PASSWORD.to_owned())).await.unwrap();
    enroll(&a, &b, "Pixel").await;
    a.start_bypass(None, Some(60)).await.unwrap();
    a.live_sync(0).await.unwrap();
    a.live_sync(0).await.unwrap();
    // The roster moves on and the server keeps no checkpoint after that; the old ops are gone.
    w.state().refuse_checkpoint = true;
    enroll(&a, &c, "Tablet").await;
    a.live_sync(0).await.unwrap();
    w.state().drop_behind_checkpoint();
    let cp_epoch = w.state().checkpoint.as_ref().unwrap().epoch;
    assert!(cp_epoch < w.state().roster.last().unwrap().epoch);
    // A fresh phone still recovers: the primary's own checkpoint from its tenure, as untrusted.
    let fresh = w.phone().await;
    recover(&fresh, PASSWORD, "New iPhone").await.unwrap();
    let state = fresh.live_sync(0).await.unwrap();
    assert_eq!(state.role, PhoneRole::Primary);
    assert_eq!(fresh.autopilot_settings().await.unwrap().bypass_until, None, "tightened");
}

#[tokio::test]
async fn a_short_log_or_a_gap_never_blocks_a_recovery_the_user_confirms() {
    let mut w = world().await;
    let a = w.phone().await;
    let b = w.phone().await;
    a.roster_create("iPhone".to_owned(), Some(PASSWORD.to_owned())).await.unwrap();
    enroll(&a, &b, "Pixel").await;
    a.start_bypass(None, Some(60)).await.unwrap();
    a.set_autopilot_mode(None, Some(AutopilotMode::Manual), None).await.unwrap();
    a.live_sync(0).await.unwrap();
    a.live_sync(0).await.unwrap();
    let epoch = w.state().roster.last().unwrap().epoch;
    // The server serves less than the head the lost phone's last status signs: the user is asked.
    let head = w.state().heartbeat.as_ref().unwrap().head_seq;
    assert!(head > 1);
    w.state().recovery_up_to = Some(head - 1);
    let phone = w.phone().await;
    let asked = recover(&phone, PASSWORD, "New iPhone").await.unwrap_err();
    assert!(matches!(asked, CoreError::RecoveryNeedsConfirmation { .. }), "{asked:?}");
    assert!(asked.to_string().contains("less"), "{asked}");
    // The op held back is the lost phone's own: its status signed more of its changes than this phone verified.
    assert!(asked.to_string().contains("own changes"), "{asked}");
    assert_eq!(w.state().roster.last().unwrap().epoch, epoch, "nothing signed");
    w.state().recovery_up_to = None;
    // An op of the lost phone after a gap: dropped (a warning), never a veto; with its status gone too, the user
    // confirms and the recovery goes through, naming only what was verified.
    let a_id = device_of(&a).await;
    let first_of_a = w.state().ops.iter().position(|o| o.op.author == a_id).unwrap();
    drop_op(&w, first_of_a);
    w.state().heartbeat = None;
    let other = w.phone().await;
    let asked = recover(&other, PASSWORD, "Other").await.unwrap_err();
    assert!(asked.to_string().contains("missing"), "{asked}");
    let done = recover_confirmed(&other, PASSWORD, "Other").await.unwrap();
    assert!(done.warnings.iter().any(|w| w.contains("dropped")), "{:?}", done.warnings);
    let entry = w.state().roster.last().unwrap().clone();
    assert_eq!((entry.reason, entry.primary_counter), (RosterReason::Recovery, Some(0)));
}

// ---- the live sync, security review round 4 ------------------------------------------------------------------------

#[tokio::test]
async fn an_omitted_status_is_confirmed_and_a_new_account_needs_none() {
    // A new account: the lost phone wrote nothing to the log, so its status is not needed.
    let mut w = world().await;
    let a = w.phone().await;
    w.state().ops_down = true;
    a.roster_create("iPhone".to_owned(), Some(PASSWORD.to_owned())).await.unwrap();
    w.state().heartbeat = None;
    assert!(w.state().ops.is_empty());
    let fresh = w.phone().await;
    let done = recover(&fresh, PASSWORD, "New iPhone").await.unwrap();
    assert!(done.warnings.is_empty(), "{:?}", done.warnings);

    // An account with history: the status omitted asks the user, and the recovery goes on once confirmed.
    let mut w = world().await;
    let a = w.phone().await;
    a.roster_create("iPhone".to_owned(), Some(PASSWORD.to_owned())).await.unwrap();
    a.start_bypass(None, Some(60)).await.unwrap();
    a.live_sync(0).await.unwrap();
    w.state().heartbeat = None;
    let fresh = w.phone().await;
    let asked = recover(&fresh, PASSWORD, "New iPhone").await.unwrap_err();
    assert!(matches!(asked, CoreError::RecoveryNeedsConfirmation { .. }) && asked.to_string().contains("missing"));
    let done = recover_confirmed(&fresh, PASSWORD, "New iPhone").await.unwrap();
    assert_eq!(done.role, PhoneRole::Primary);
}

#[tokio::test]
async fn a_stale_status_from_an_earlier_primary_is_confirmed_never_a_block() {
    let mut w = world().await;
    let a = w.phone().await;
    let b = w.phone().await;
    a.roster_create("iPhone".to_owned(), Some(PASSWORD.to_owned())).await.unwrap();
    enroll(&a, &b, "Pixel").await;
    a.live_sync(0).await.unwrap();
    let from_a = w.state().heartbeat.clone().unwrap();
    // B takes over and writes; then B is lost, and the server shows A's old status instead of B's.
    let id = b.handover_request().await.unwrap();
    a.sync(0).await.unwrap();
    a.handover_approve(id, device_of(&b).await).await.unwrap();
    assert_eq!(b.handover_poll().await.unwrap(), HandoverProgress::Done);
    b.start_bypass(None, Some(60)).await.unwrap();
    b.live_sync(0).await.unwrap();
    w.state().heartbeat = Some(from_a);
    let fresh = w.phone().await;
    let asked = recover(&fresh, PASSWORD, "New iPhone").await.unwrap_err();
    assert!(matches!(asked, CoreError::RecoveryNeedsConfirmation { .. }), "{asked:?}");
    assert!(asked.to_string().contains("does not check out"), "{asked}");
    let done = recover_confirmed(&fresh, PASSWORD, "New iPhone").await.unwrap();
    assert_eq!(done.role, PhoneRole::Primary);
}

#[tokio::test]
async fn another_history_than_the_lost_phone_signed_is_a_warning_the_user_can_override() {
    let mut w = world().await;
    let a = w.phone().await;
    let b = w.phone().await;
    a.roster_create("iPhone".to_owned(), Some(PASSWORD.to_owned())).await.unwrap();
    enroll(&a, &b, "Pixel").await;
    a.live_sync(0).await.unwrap();
    b.live_sync(0).await.unwrap();
    // The viewer pulls the brake, then the primary writes: two ops of two phones, side by side.
    b.set_autopilot_mode(None, Some(AutopilotMode::Lockdown), None).await.unwrap();
    a.live_sync(0).await.unwrap();
    a.set_autopilot_mode(None, Some(AutopilotMode::Manual), None).await.unwrap();
    a.live_sync(0).await.unwrap();
    let b_id = device_of(&b).await;
    // A fresh status signed over that history (a new epoch makes the primary post at once).
    let c = w.phone().await;
    enroll(&a, &c, "Tablet").await;
    a.live_sync(0).await.unwrap();
    // The server swaps the brake with the op after it: the same ops, another history at the signed head.
    {
        let mut st = w.state();
        let brake = st.ops.iter().position(|o| o.op.author == b_id).unwrap();
        assert!(brake + 1 < st.ops.len());
        let (first, second) = (st.ops[brake].seq, st.ops[brake + 1].seq);
        st.ops.swap(brake, brake + 1);
        st.ops[brake].seq = first;
        st.ops[brake + 1].seq = second;
    }
    let fresh = w.phone().await;
    let asked = recover(&fresh, PASSWORD, "New iPhone").await.unwrap_err();
    assert!(matches!(asked, CoreError::RecoveryNeedsConfirmation { .. }), "{asked:?}");
    assert!(asked.to_string().contains("another history"), "{asked}");
    let done = recover_confirmed(&fresh, PASSWORD, "New iPhone").await.unwrap();
    assert!(done.warnings.iter().any(|w| w.contains("another history")), "{:?}", done.warnings);
}

#[tokio::test]
async fn an_unverifiable_checkpoint_asks_for_a_fresh_start_instead_of_failing() {
    let mut w = world().await;
    let a = w.phone().await;
    a.roster_create("iPhone".to_owned(), Some(PASSWORD.to_owned())).await.unwrap();
    a.start_bypass(None, Some(60)).await.unwrap();
    a.live_sync(0).await.unwrap();
    a.live_sync(0).await.unwrap();
    w.state().drop_behind_checkpoint();
    w.state().corrupt_checkpoint = true;
    let fresh = w.phone().await;
    let asked = recover(&fresh, PASSWORD, "New iPhone").await.unwrap_err();
    assert!(asked.to_string().contains("afresh"), "{asked}");
    let done = recover_confirmed(&fresh, PASSWORD, "New iPhone").await.unwrap();
    assert_eq!(done.role, PhoneRole::Primary);
}

#[tokio::test]
async fn a_recovered_primarys_own_changes_are_never_tightened_again() {
    let mut w = world().await;
    let a = w.phone().await;
    let b = w.phone().await;
    a.roster_create("iPhone".to_owned(), Some(PASSWORD.to_owned())).await.unwrap();
    enroll(&a, &b, "Pixel").await;
    a.start_bypass(None, Some(60)).await.unwrap();
    a.live_sync(0).await.unwrap();
    a.live_sync(0).await.unwrap();
    b.live_sync(0).await.unwrap();
    w.state().drop_behind_checkpoint();
    let c = w.phone().await;
    recover_confirmed(&c, PASSWORD, "New iPhone").await.unwrap();
    c.live_sync(0).await.unwrap();
    assert_eq!(c.autopilot_settings().await.unwrap().bypass_until, None, "the untrusted start is tightened");
    // What the new primary sets itself stays set, cycle after cycle.
    c.start_bypass(None, Some(60)).await.unwrap();
    for _ in 0..3 {
        c.live_sync(0).await.unwrap();
    }
    assert!(c.autopilot_settings().await.unwrap().bypass_until.is_some());
    b.live_sync(0).await.unwrap();
    assert!(b.autopilot_settings().await.unwrap().bypass_until.is_some(), "and reaches the viewer");
}

#[tokio::test]
async fn a_viewer_behind_the_floor_that_recovers_takes_the_checkpoint_as_untrusted() {
    let mut w = world().await;
    let a = w.phone().await;
    let b = w.phone().await;
    a.roster_create("iPhone".to_owned(), Some(PASSWORD.to_owned())).await.unwrap();
    enroll(&a, &b, "Pixel").await;
    a.live_sync(0).await.unwrap();
    b.live_sync(0).await.unwrap();
    // The viewer is away while the primary runs a bypass, a phone joins (a checkpoint at the new epoch), and the
    // server drops what the viewer had not taken.
    let before = w.state().ops.last().map_or(0, |o| o.seq);
    a.start_bypass(None, Some(60)).await.unwrap();
    a.live_sync(0).await.unwrap();
    let c = w.phone().await;
    enroll(&a, &c, "Tablet").await;
    a.live_sync(0).await.unwrap();
    w.state().drop_behind_checkpoint();
    assert!(w.state().floor > before, "the viewer is behind the floor");
    recover_confirmed(&b, PASSWORD, "Pixel").await.unwrap();
    b.live_sync(0).await.unwrap();
    assert_eq!(b.autopilot_settings().await.unwrap().bypass_until, None, "not taken as trusted");
}

#[tokio::test]
async fn a_confirmation_stands_only_for_what_the_user_was_shown() {
    let mut w = world().await;
    let a = w.phone().await;
    a.roster_create("iPhone".to_owned(), Some(PASSWORD.to_owned())).await.unwrap();
    a.start_bypass(None, Some(60)).await.unwrap();
    a.live_sync(0).await.unwrap();
    w.state().heartbeat = None;
    let fresh = w.phone().await;
    let preview = fresh.roster_recover_preview(PASSWORD.to_owned()).await.unwrap();
    let again = || fresh.roster_recover(PASSWORD.to_owned(), "New iPhone".to_owned(), preview.tip.clone(), None);
    let Err(CoreError::RecoveryNeedsConfirmation {
        reason,
        token,
    }) = again().await
    else {
        panic!("asks first");
    };
    assert!(reason.contains("missing"), "{reason}");
    let epoch = w.state().roster.last().unwrap().epoch;
    // A token for anything else (made up, or for what the server showed before it changed) signs nothing.
    let recover_with = |token: String| {
        fresh.roster_recover(PASSWORD.to_owned(), "New iPhone".to_owned(), preview.tip.clone(), Some(token))
    };
    assert!(matches!(recover_with("0".repeat(64)).await, Err(CoreError::RecoveryNeedsConfirmation { .. })));
    w.state().inflate_counter = Some(500);
    let Err(CoreError::RecoveryNeedsConfirmation {
        reason: changed,
        token: other,
    }) = recover_with(token.clone()).await
    else {
        panic!("asks again");
    };
    assert_ne!(other, token);
    assert!(changed.contains("500"), "{changed}");
    assert_eq!(w.state().roster.last().unwrap().epoch, epoch, "nothing signed");
    // What the user was shown, confirmed: it goes through.
    w.state().inflate_counter = None;
    let done = recover_with(token).await.unwrap();
    assert_eq!(done.role, PhoneRole::Primary);
}

struct PermissionOwner;
#[async_trait::async_trait]
impl reins_core::OwnerCheck for PermissionOwner {
    async fn confirm(&self, _reason: String) -> Result<bool, reins_core::ForeignError> {
        Ok(true)
    }
}

struct SendStarted(Arc<tokio::sync::Notify>);
impl Respond for SendStarted {
    fn respond(&self, _request: &Request) -> ResponseTemplate {
        self.0.notify_one();
        ok(json!({"id":"secret-provider-result","threadId":"t"})).set_delay(std::time::Duration::from_millis(500))
    }
}

async fn permission_phone(w: &mut World, gmail: &MockServer) -> Arc<ReinsCore> {
    Mock::given(method("GET")).and(path("/reins/api/connections")).respond_with(ok(json!({"connections":[{"id":"c1","label":"Agent","client_name":"Claude","client_host":"claude.ai","created_at":1,"last_used_at":null}]}))).with_priority(1).mount(&w.server).await;
    Mock::given(method("GET"))
        .and(path("/users/me/profile"))
        .respond_with(ok(json!({"emailAddress":EMAIL})))
        .mount(gmail)
        .await;
    Mock::given(method("POST"))
        .and(path("/users/me/messages/send"))
        .respond_with(ok(json!({"id":"sent","threadId":"t"})))
        .mount(gmail)
        .await;
    let core = w
        .phone_config(CoreConfig {
            gmail_base: gmail.uri(),
            ..CoreConfig::default()
        })
        .await;
    core.add_account(EMAIL.into()).await.unwrap();
    core
}

fn permission_send(id: &str, recipients: &[&str]) -> reins_proto::relay::RelayRequest {
    serde_json::from_value(json!({"v":1,"id":id,"connection_id":"c1","connection_label":"Agent","created_at":reins_core::store::unix_now(),"account":EMAIL,"call":{"tool":"gmail_send","email":{"to":recipients,"subject":"Hi","body":"A note","cc":[]}}})).unwrap()
}

async fn sent_count(gmail: &MockServer) -> usize {
    gmail
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.method == "POST" && r.url.path() == "/users/me/messages/send")
        .count()
}

#[tokio::test]
async fn typed_permissions_enforce_selected_recipients_limits_all_modes_and_truthful_native_defaults() {
    let mut w = world().await;
    let gmail = MockServer::start().await;
    let core = permission_phone(&mut w, &gmail).await;
    let legacy = core.account_permissions("gmail".into(), EMAIL.into()).await.unwrap();
    assert!(legacy.managed, "verified connection installs Review baseline before UI setup");
    assert!(!legacy.available_presets.contains(&AccountPermissionPreset::ReadAssist));
    assert!(legacy.capabilities.iter().all(|c| !c.supported_modes.contains(&PermissionMode::Auto)));
    core.roster_create("Phone".into(), Some(PASSWORD.into())).await.unwrap();
    let mut draft = core
        .account_permission_draft("gmail".into(), EMAIL.into(), AccountPermissionPreset::ReviewEveryRequest)
        .await
        .unwrap();
    draft.agent_ids = vec!["c1".into()];
    draft.grant_account_access = true;
    draft.max_uses = Some(2);
    let send = draft.rules.iter_mut().find(|r| r.capability == PermissionCapability::MailSend).unwrap();
    send.mode = PermissionMode::Auto;
    send.scope.ids = vec!["friend@example.com".into()];
    let options = core
        .permission_scope_options(
            "gmail".into(),
            EMAIL.into(),
            PermissionCapability::MailSend,
            "Friend@EXAMPLE.com".into(),
            10,
        )
        .await
        .unwrap();
    assert!(options.options.iter().any(|o| o.id == "friend@example.com"));
    let expires = draft.expires_at;
    let saved = core.save_account_permissions(draft.clone(), Arc::new(PermissionOwner)).await.unwrap();
    assert_eq!(saved.remaining_uses, Some(2));
    assert!(core.save_account_permissions(draft, Arc::new(PermissionOwner)).await.is_err(), "stale CAS");
    core.engine().process_relayed(permission_send("allowed-manual", &["friend@example.com"])).await.unwrap();
    assert_eq!(sent_count(&gmail).await, 1);
    let custom =
        core.account_permission_draft("gmail".into(), EMAIL.into(), AccountPermissionPreset::Custom).await.unwrap();
    assert_eq!(custom.expires_at, expires);
    assert_eq!(custom.max_uses, Some(1), "opening editor cannot replenish quota");
    core.set_autopilot_mode(None, Some(AutopilotMode::Bypass), Some(30)).await.unwrap();
    core.engine()
        .process_relayed(permission_send("outside-scope", &["friend@example.com", "corporate@example.com"]))
        .await
        .unwrap();
    core.engine().process_relayed(permission_send("allowed-bypass", &["friend@example.com"])).await.unwrap();
    core.engine().process_relayed(permission_send("exhausted", &["friend@example.com"])).await.unwrap();
    assert_eq!(sent_count(&gmail).await, 2, "Bypass cannot approve outside policy or replenish its use limit");
    assert!(core.pending().await.unwrap().iter().any(|p| p.id == "outside-scope"));
    core.set_autopilot_mode(None, Some(AutopilotMode::Lockdown), None).await.unwrap();
    core.engine().process_relayed(permission_send("locked", &["friend@example.com"])).await.unwrap();
    assert_eq!(sent_count(&gmail).await, 2);
    core.set_autopilot_mode(None, Some(AutopilotMode::Manual), None).await.unwrap();
    core.remove_account(EMAIL.into()).await.unwrap();
    core.add_account(EMAIL.into()).await.unwrap();
    let reconnected = core.account_permissions("gmail".into(), EMAIL.into()).await.unwrap();
    assert!(reconnected.managed);
    assert!(
        reconnected.capabilities.iter().all(|capability| capability.configured_mode != Some(PermissionMode::Auto)),
        "disconnect permanently revokes automatic authority"
    );
    core.engine().process_relayed(permission_send("after-reconnect", &["friend@example.com"])).await.unwrap();
    assert_eq!(sent_count(&gmail).await, 2);
    let mut fresh = core
        .account_permission_draft("gmail".into(), EMAIL.into(), AccountPermissionPreset::ReviewEveryRequest)
        .await
        .unwrap();
    fresh.agent_ids = vec!["c1".into()];
    fresh.grant_account_access = true;
    fresh.max_uses = Some(1);
    let send = fresh.rules.iter_mut().find(|rule| rule.capability == PermissionCapability::MailSend).unwrap();
    send.mode = PermissionMode::Auto;
    send.scope.ids = vec!["friend@example.com".into()];
    core.save_account_permissions(fresh, Arc::new(PermissionOwner)).await.unwrap();
    core.engine().process_relayed(permission_send("fresh-after-reconnect", &["friend@example.com"])).await.unwrap();
    assert_eq!(sent_count(&gmail).await, 3, "a new explicit owner authorization can create fresh bounded authority");
}

#[tokio::test]
async fn delegation_consumption_survives_pause_and_primary_handover_without_legacy_fallback() {
    let mut w = world().await;
    let gmail = MockServer::start().await;
    let a = permission_phone(&mut w, &gmail).await;
    a.roster_create("Primary".into(), Some(PASSWORD.into())).await.unwrap();
    let mut draft = a.delegation_draft("gmail".into(), EMAIL.into()).await.unwrap();
    draft.agent_ids = vec!["c1".into()];
    draft.grant_account_access = true;
    draft.max_uses = 1;
    draft.rules = vec![CapabilityPermissionInput {
        capability: PermissionCapability::MailSend,
        mode: PermissionMode::Auto,
        scope: PermissionScopeSelection {
            kind: PermissionScopeKind::SelectedRecipients,
            ids: vec!["friend@example.com".into()],
        },
    }];
    let issued = a.issue_delegation(draft, Arc::new(PermissionOwner)).await.unwrap();
    a.engine().process_relayed(permission_send("delegated-first", &["friend@example.com"])).await.unwrap();
    assert_eq!(sent_count(&gmail).await, 1);
    let view = a.delegations().await.unwrap().remove(0);
    assert_eq!(view.used, 1);
    let paused = a
        .change_delegation(issued.id.clone(), view.revision, DelegationChange::Pause, Arc::new(PermissionOwner))
        .await
        .unwrap();
    let resumed = a
        .change_delegation(issued.id, paused.revision, DelegationChange::Resume, Arc::new(PermissionOwner))
        .await
        .unwrap();
    assert_eq!(resumed.used, 1);
    let b = w
        .phone_config(CoreConfig {
            gmail_base: gmail.uri(),
            ..CoreConfig::default()
        })
        .await;
    enroll(&a, &b, "Viewer").await;
    let request = b.handover_request().await.unwrap();
    a.handover_approve(request, device_of(&b).await).await.unwrap();
    assert!(matches!(b.handover_poll().await.unwrap(), HandoverProgress::Done));
    b.engine().process_relayed(permission_send("after-handover", &["friend@example.com"])).await.unwrap();
    assert_eq!(sent_count(&gmail).await, 1, "new primary cannot reuse the previous primary's spent delegation");
    assert_eq!(b.delegations().await.unwrap()[0].used, 1);
}

#[tokio::test]
async fn interrupted_handover_holds_effects_until_policy_history_is_materialized_and_can_retry() {
    let mut w = world().await;
    let gmail = MockServer::start().await;
    let a = permission_phone(&mut w, &gmail).await;
    a.roster_create("Primary".into(), Some(PASSWORD.into())).await.unwrap();
    let b = w
        .phone_config(CoreConfig {
            gmail_base: gmail.uri(),
            ..CoreConfig::default()
        })
        .await;
    enroll(&a, &b, "Viewer").await;
    let mut draft = a.delegation_draft("gmail".into(), EMAIL.into()).await.unwrap();
    draft.agent_ids = vec!["c1".into()];
    draft.grant_account_access = true;
    draft.max_uses = 1;
    draft.rules = vec![CapabilityPermissionInput {
        capability: PermissionCapability::MailSend,
        mode: PermissionMode::Auto,
        scope: PermissionScopeSelection {
            kind: PermissionScopeKind::SelectedRecipients,
            ids: vec!["friend@example.com".into()],
        },
    }];
    a.issue_delegation(draft, Arc::new(PermissionOwner)).await.unwrap();
    let request = b.handover_request().await.unwrap();
    a.handover_approve(request, device_of(&b).await).await.unwrap();
    w.state().stall_after_bundle = true;
    assert!(b.handover_poll().await.is_err(), "key installation alone must not report Done");
    assert!(w.state().bundle.is_some(), "the bundle remains available for retry");
    let error = b.engine().authorization_policies().unwrap_err();
    assert!(error.to_string().contains("authorization history"), "{error}");
    if let Err(error) = b.engine().process_relayed(permission_send("before-ready", &["friend@example.com"])).await {
        assert!(error.to_string().contains("authorization history"), "{error}");
    }
    assert_eq!(sent_count(&gmail).await, 0, "missing history never reopens legacy authorization");
    {
        let mut state = w.state();
        state.stall_after_bundle = false;
        state.pulls_down = false;
    }
    assert_eq!(b.handover_poll().await.unwrap(), HandoverProgress::Done);
    assert!(w.state().bundle.is_none());
    assert_eq!(b.delegations().await.unwrap().len(), 1, "Done includes the actual policy state");
    b.engine().process_relayed(permission_send("after-ready", &["friend@example.com"])).await.unwrap();
    assert_eq!(sent_count(&gmail).await, 1, "verified scoped authority becomes usable only after readiness");
}

#[tokio::test]
async fn managed_reads_require_exact_resources_and_never_release_suspected_codes() {
    let mut w = world().await;
    let gmail = MockServer::start().await;
    let core = permission_phone(&mut w, &gmail).await;
    core.roster_create("Primary".into(), Some(PASSWORD.into())).await.unwrap();
    for id in ["messagea", "messageb"] {
        Mock::given(method("GET"))
            .and(path(format!("/users/me/messages/{id}")))
            .respond_with(ok(common::gmail_message(id, "friend@example.com", "Lunch", Some("At noon"))))
            .mount(&gmail)
            .await;
    }
    let mut draft = core
        .account_permission_draft("gmail".into(), EMAIL.into(), AccountPermissionPreset::ReviewEveryRequest)
        .await
        .unwrap();
    draft.agent_ids = vec!["c1".into()];
    draft.grant_account_access = true;
    draft.max_uses = Some(3);
    let read = draft.rules.iter_mut().find(|r| r.capability == PermissionCapability::MailRead).unwrap();
    read.mode = PermissionMode::Auto;
    read.scope = PermissionScopeSelection {
        kind: PermissionScopeKind::SelectedResources,
        ids: vec!["messagea".into()],
    };
    core.save_account_permissions(draft, Arc::new(PermissionOwner)).await.unwrap();
    let read_request = |id: &str, message: &str| {
        let mut request = permission_send(id, &["friend@example.com"]);
        request.call = reins_proto::gmail::ToolCall::GmailRead {
            message_ids: vec![message.into()],
        };
        request
    };
    core.engine().process_relayed(read_request("read-allowed", "messagea")).await.unwrap();
    assert!(!core.pending().await.unwrap().iter().any(|p| p.id == "read-allowed"));
    core.engine().process_relayed(read_request("read-outside", "messageb")).await.unwrap();
    assert!(core.pending().await.unwrap().iter().any(|p| p.id == "read-outside"));
    Mock::given(method("GET"))
        .and(path("/users/me/messages/messagea"))
        .respond_with(ok(common::gmail_message(
            "messagea",
            "security@example.com",
            "Your verification code 123456",
            Some("Do not share this sign-in code"),
        )))
        .with_priority(1)
        .mount(&gmail)
        .await;
    core.engine().process_relayed(read_request("read-code", "messagea")).await.unwrap();
    assert!(core.pending().await.unwrap().iter().any(|p| p.id == "read-code"));
    let custom =
        core.account_permission_draft("gmail".into(), EMAIL.into(), AccountPermissionPreset::Custom).await.unwrap();
    assert_eq!(custom.max_uses, Some(2), "held secret and out-of-scope requests must not spend the permission");
    let preview = core.preview_account_permissions(custom).await.unwrap();
    assert!(preview.eligible_history_count.is_some());
    assert!(preview.review_history_count.is_some());
}

#[tokio::test]
async fn ambiguous_gmail_send_retains_consumption_and_prevents_duplicate_retry() {
    let mut w = world().await;
    let gmail = MockServer::start().await;
    let core = permission_phone(&mut w, &gmail).await;
    core.roster_create("Primary".into(), Some(PASSWORD.into())).await.unwrap();
    let mut draft = core
        .account_permission_draft("gmail".into(), EMAIL.into(), AccountPermissionPreset::ReviewEveryRequest)
        .await
        .unwrap();
    draft.agent_ids = vec!["c1".into()];
    draft.grant_account_access = true;
    draft.max_uses = Some(1);
    let send = draft.rules.iter_mut().find(|r| r.capability == PermissionCapability::MailSend).unwrap();
    send.mode = PermissionMode::Auto;
    send.scope.ids = vec!["friend@example.com".into()];
    core.save_account_permissions(draft, Arc::new(PermissionOwner)).await.unwrap();
    Mock::given(method("POST"))
        .and(path("/users/me/messages/send"))
        .respond_with(ResponseTemplate::new(500))
        .with_priority(1)
        .mount(&gmail)
        .await;
    core.engine().process_relayed(permission_send("uncertain", &["friend@example.com"])).await.unwrap();
    assert_eq!(sent_count(&gmail).await, 1);
    let activity = core.activity(20).await.unwrap();
    assert!(activity.iter().any(|entry| entry.outcome == "unknown" && entry.detail.contains("may have sent")));
    assert_eq!(core.account_permissions("gmail".into(), EMAIL.into()).await.unwrap().remaining_uses, Some(0));
    // Same request id cannot acquire another Started receipt, even when no provider result was returned.
    let _retry = core.engine().process_relayed(permission_send("uncertain", &["friend@example.com"])).await;
    core.engine().process_relayed(permission_send("new-after-uncertain", &["friend@example.com"])).await.unwrap();
    assert_eq!(sent_count(&gmail).await, 1);
}

#[tokio::test]
async fn gmail_success_is_withheld_when_policy_changes_during_positive_or_owner_send() {
    use reins_proto::authorization::{
        OperationId, PolicyEffect, PolicyIssuer, PolicyKind, PolicyLifecycle, PolicyRecord, PolicyScope, UsageLimits,
    };
    for automatic in [true, false] {
        let mut world = world().await;
        let gmail = MockServer::start().await;
        let core = permission_phone(&mut world, &gmail).await;
        core.roster_create("Primary".into(), Some(PASSWORD.into())).await.unwrap();
        let mut draft = core
            .account_permission_draft("gmail".into(), EMAIL.into(), AccountPermissionPreset::ReviewEveryRequest)
            .await
            .unwrap();
        draft.agent_ids = vec!["c1".into()];
        draft.grant_account_access = true;
        draft.max_uses = Some(1);
        let send = draft.rules.iter_mut().find(|rule| rule.capability == PermissionCapability::MailSend).unwrap();
        if automatic {
            send.mode = PermissionMode::Auto;
            send.scope.ids = vec!["friend@example.com".into()];
        }
        core.save_account_permissions(draft, Arc::new(PermissionOwner)).await.unwrap();
        let entered = Arc::new(tokio::sync::Notify::new());
        Mock::given(method("POST"))
            .and(path("/users/me/messages/send"))
            .respond_with(SendStarted(Arc::clone(&entered)))
            .with_priority(1)
            .mount(&gmail)
            .await;
        let request = permission_send("change-during-send", &["friend@example.com"]);
        if !automatic {
            core.engine().process_relayed(request.clone()).await.unwrap();
        }
        let task_core = Arc::clone(&core);
        let mut sending = tokio::spawn(async move {
            if automatic {
                task_core.engine().process_relayed(request).await
            } else {
                task_core
                    .approve(
                        "change-during-send".into(),
                        reins_core::ApprovalChoice {
                            selected_message_ids: vec![],
                            standing: None,
                        },
                    )
                    .await
            }
        });
        tokio::select! {
            () = entered.notified() => {},
            result = &mut sending => panic!("send ended before provider invocation: {result:?}"),
        }
        let engine = core.engine();
        let state = engine.authorization_policies().unwrap();
        let deny = PolicyRecord {
            v: 1,
            id: "deny-during-send".into(),
            revision: 1,
            issuer: PolicyIssuer {
                principal_id: USER.into(),
                tenant_id: None,
            },
            kind: PolicyKind::Mandatory,
            lifecycle: PolicyLifecycle::Active,
            valid_from: reins_core::store::unix_now(),
            expires_at: None,
            scope: PolicyScope {
                account_ids: vec![serde_json::json!(["gmail", EMAIL]).to_string()],
                operations: vec![OperationId {
                    service: "gmail".into(),
                    capability: "write".into(),
                    action: "send".into(),
                }],
                ..PolicyScope::default()
            },
            conditions: vec![],
            effect: PolicyEffect::Deny,
            parent_policy_ids: vec![],
            limits: UsageLimits::default(),
        };
        engine.save_authorization_policy(state.revision, deny, &PermissionOwner).await.unwrap();
        sending.await.unwrap().unwrap();
        assert_eq!(sent_count(&gmail).await, 1);
        let answer = world
            .state()
            .signed
            .iter()
            .filter(|(_, id, _)| id == "change-during-send")
            .map(|(_, _, bytes)| String::from_utf8(bytes.clone()).unwrap())
            .next_back()
            .unwrap();
        assert!(answer.contains("Do not repeat"), "{answer}");
        assert!(!answer.contains("secret-provider-result"), "provider result must remain withheld");
        assert!(core.activity(20).await.unwrap().iter().any(|entry| entry.outcome == "succeeded_withheld"));
        if automatic {
            assert_eq!(core.account_permissions("gmail".into(), EMAIL.into()).await.unwrap().remaining_uses, Some(0));
        }
    }
}

struct PausedPermissionOwner {
    started: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
}
#[async_trait::async_trait]
impl reins_core::OwnerCheck for PausedPermissionOwner {
    async fn confirm(&self, _reason: String) -> Result<bool, reins_core::ForeignError> {
        self.started.notify_one();
        self.release.notified().await;
        Ok(true)
    }
}

#[tokio::test]
async fn revoking_an_agent_during_owner_confirmation_cannot_recreate_account_or_delegation_authority() {
    for (delegation, remote_confirmed) in [(false, true), (true, true), (false, false)] {
        let mut world = world().await;
        let gmail = MockServer::start().await;
        let core = permission_phone(&mut world, &gmail).await;
        core.roster_create("Phone".into(), Some(PASSWORD.into())).await.unwrap();
        Mock::given(method("DELETE"))
            .and(path("/reins/api/connections/c1"))
            .respond_with(ResponseTemplate::new(if remote_confirmed {
                200
            } else {
                500
            }))
            .with_priority(1)
            .expect(1)
            .mount(&world.server)
            .await;
        let started = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let owner = Arc::new(PausedPermissionOwner {
            started: Arc::clone(&started),
            release: Arc::clone(&release),
        });
        let pending = if delegation {
            let mut draft = core.delegation_draft("gmail".into(), EMAIL.into()).await.unwrap();
            draft.agent_ids = vec!["c1".into()];
            draft.grant_account_access = true;
            let saving = Arc::clone(&core);
            tokio::spawn(async move { saving.issue_delegation(draft, owner).await.map(|_| ()) })
        } else {
            let mut draft = core
                .account_permission_draft("gmail".into(), EMAIL.into(), AccountPermissionPreset::ReadAssist)
                .await
                .unwrap();
            draft.agent_ids = vec!["c1".into()];
            draft.grant_account_access = true;
            let saving = Arc::clone(&core);
            tokio::spawn(async move { saving.save_account_permissions(draft, owner).await.map(|_| ()) })
        };
        started.notified().await;
        let revoke = core.revoke_connection("c1".into()).await;
        assert_eq!(revoke.is_ok(), remote_confirmed);
        if let Err(error) = revoke {
            assert!(error.to_string().contains("blocked on this phone"));
        }
        assert!(
            core.account_permissions("gmail".into(), EMAIL.into()).await.unwrap().agents.is_empty(),
            "stale server connection listings cannot offer revoked agents for new authority"
        );
        release.notify_one();
        assert!(pending.await.unwrap().is_err(), "late owner confirmation must fail after connection revocation");
        assert!(core.grants().await.unwrap().iter().all(|grant| grant.connection_id != "c1"));
        assert!(core.delegations().await.unwrap().is_empty());
        core.engine().process_relayed(permission_send("revoked", &["friend@example.com"])).await.unwrap();
        assert!(!core.pending().await.unwrap().iter().any(|pending| pending.id == "revoked"));
        assert_eq!(sent_count(&gmail).await, 0);
    }
}

#[tokio::test]
async fn non_gmail_account_defaults_are_readonly_until_reversible_settings_are_supported() {
    let mut world = world().await;
    let gmail = MockServer::start().await;
    let core = permission_phone(&mut world, &gmail).await;
    core.engine().register_account("github", "person").unwrap();
    let view = core.account_permissions("github".into(), "person".into()).await.unwrap();
    assert!(!view.managed, "informational views cannot migrate legacy authorization");
    assert!(view.available_presets.is_empty());
    assert!(view.capabilities.iter().all(|capability| capability.supported_modes.is_empty()));
    assert!(
        core.account_permission_draft("github".into(), "person".into(), AccountPermissionPreset::Custom).await.is_err()
    );
    assert!(!core.account_permissions("github".into(), "person".into()).await.unwrap().managed);
}
