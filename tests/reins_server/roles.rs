//! Phone roles and live sync: the roster (create, enroll, handover, recovery), the op log and its checks, the sync
//! long-poll, signed answers, and legacy accounts left as they were.

use std::time::{Duration, Instant};

use reins_proto::{
    answer::{AnswerKind, answer_message},
    canon::{encode_payload, encode_sig, hex32},
    desktop::encode_key,
    enroll::{EnrollChallenge, Transcript, commitment, encode_nonce},
    oplog::{Checkpoint, Heartbeat, Op, OpClass},
    roster::{Member, RosterEntry, RosterReason, recovery_wrap_hash},
};
use reqwest::StatusCode;
use ring::signature::{Ed25519KeyPair, KeyPair};
use serde_json::{Value, json};

use crate::harness::{Options, PASSWORD_HASH, Phone, Server, between, client};

/// The recovery wrap the tests' rosters start with (opaque bytes to the server), and the one a recovery brings.
const WRAP: &str = "d3JhcA";
const NEW_WRAP: &str = "bmV3IHdyYXA";
/// The wrap a handover brings ("handover").
const HANDOVER_WRAP: &str = "aGFuZG92ZXI";

fn wrap_hash(wrap: &str) -> String {
    recovery_wrap_hash(wrap).expect("base64url")
}

/// A phone's keys: its device signing key, and a stand-in for its box key (32 bytes the server only relays).
struct Keys {
    sign: Ed25519KeyPair,
    sign_pub: String,
    box_pub: String,
}

fn keys(seed: u8) -> Keys {
    let sign = Ed25519KeyPair::from_seed_unchecked(&[seed; 32]).expect("key");
    let sign_pub = encode_key(&sign.public_key().as_ref().try_into().expect("32 bytes"));
    Keys {
        sign,
        sign_pub,
        box_pub: encode_key(&[seed.wrapping_add(100); 32]),
    }
}

fn owner(seed: u8) -> Ed25519KeyPair {
    Ed25519KeyPair::from_seed_unchecked(&[seed; 32]).expect("owner key")
}

fn public(key: &Ed25519KeyPair) -> String {
    encode_key(&key.public_key().as_ref().try_into().expect("32 bytes"))
}

fn sig(key: &Ed25519KeyPair, message: &[u8]) -> String {
    encode_sig(&key.sign(message).as_ref().try_into().expect("64 bytes"))
}

fn member(phone: &Phone, k: &Keys, name: &str) -> Member {
    Member {
        device_id: phone.device_id.clone(),
        name: name.to_owned(),
        sign_pub: k.sign_pub.clone(),
        box_pub: k.box_pub.clone(),
    }
}

/// `entry` naming `wrap` as its recovery wrap (removals and handovers bring a new one).
fn rewrapped(mut entry: RosterEntry, wrap: &str) -> RosterEntry {
    entry.recovery_wrap_hash = wrap_hash(wrap);
    entry
}

fn signed(mut entry: RosterEntry, by: &Ed25519KeyPair) -> RosterEntry {
    entry.sig = sig(by, &entry.signing_bytes().expect("signing bytes"));
    entry
}

/// The account id the server knows the phone's user by (the access token's `sub`).
fn account_of(phone: &Phone) -> String {
    let payload = phone.token.split('.').nth(1).expect("jwt payload");
    let raw = data_encoding::BASE64URL_NOPAD.decode(payload.trim_end_matches('=').as_bytes()).expect("base64url");
    serde_json::from_slice::<Value>(&raw).expect("json")["sub"].as_str().expect("sub").to_owned()
}

fn error(body: &Value) -> &str {
    body["error"].as_str().unwrap_or_default()
}

/// An account with a primary phone and a roster at epoch 1.
struct Account {
    server: Server,
    user: String,
    primary: Phone,
    pk: Keys,
    owner: Ed25519KeyPair,
    chain: Vec<RosterEntry>,
}

impl Account {
    async fn new(email: &str) -> Self {
        Self::new_with(email, Options::default()).await
    }

    async fn new_with(email: &str, options: Options) -> Self {
        let server = Server::start_with(options).await;
        let primary = server.phone(email).await;
        primary.register_device().await;
        Self::with_roster(server, primary).await
    }

    async fn with_roster(server: Server, primary: Phone) -> Self {
        let user = account_of(&primary);
        let pk = keys(1);
        let owner = owner(50);
        let e1 = signed(
            RosterEntry::genesis(
                &user,
                member(&primary, &pk, "iPhone"),
                public(&owner),
                encode_key(&[60; 32]),
                wrap_hash(WRAP),
                1,
            ),
            &owner,
        );
        let (status, body) = primary
            .post("/roster", &json!({"entry": e1, "recovery_wrap": WRAP, "master_password_hash": PASSWORD_HASH}))
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["epoch"], 1);
        Self {
            server,
            user,
            primary,
            pk,
            owner,
            chain: vec![e1],
        }
    }

    fn current(&self) -> &RosterEntry {
        self.chain.last().expect("an entry")
    }

    /// `viewer` enrolls through a join with the commit-then-reveal exchange; the primary appends the entry.
    async fn enroll(&mut self, viewer: &Phone, vk: &Keys, name: &str) {
        let nj = [7u8; 32];
        let commit = commitment(&self.user, &viewer.device_id, &vk.box_pub, &vk.sign_pub, &nj).expect("commitment");
        let new = json!({"v": 1, "device_name": name, "public_key": vk.box_pub, "sign_pub": vk.sign_pub, "commitment": commit});
        let (status, created) = viewer.post("/joins", &new).await;
        assert_eq!(status, StatusCode::OK, "{created}");
        let id = created["id"].as_str().expect("join id").to_owned();
        let (_, request) = self.primary.get(&format!("/joins/{id}")).await;
        assert_eq!(request["device_id"].as_str(), Some(viewer.device_id.as_str()));
        assert_eq!(request["sign_pub"].as_str(), Some(vk.sign_pub.as_str()));

        let mut viewers = self.current().viewers.clone();
        viewers.push(member(viewer, vk, name));
        let entry = signed(
            self.current().successor(RosterReason::Enroll, self.current().primary.clone(), viewers, 2).expect("next"),
            &self.owner,
        );
        // Not before the exchange is complete: the server binds members only through it.
        let (status, body) = self.primary.post("/roster", &json!({"entry": entry})).await;
        assert_eq!((status, error(&body)), (StatusCode::BAD_REQUEST, "bad_roster"), "{body}");

        let np = [9u8; 32];
        let challenge = EnrollChallenge {
            v: 1,
            nonce: encode_nonce(&np),
            primary_sign_pub: self.pk.sign_pub.clone(),
            owner_pub: public(&self.owner),
            epoch: self.current().epoch,
            expires_at: chrono_now() + 120,
        };
        let (status, body) =
            viewer.post(&format!("/joins/{id}/reveal"), &json!({"v": 1, "nonce": encode_nonce(&nj)})).await;
        assert_eq!(status, StatusCode::CONFLICT, "no reveal before the challenge: {body}");
        let (status, body) = self.primary.post(&format!("/joins/{id}/challenge"), &json!(challenge)).await;
        assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
        assert_eq!(
            self.primary.post(&format!("/joins/{id}/challenge"), &json!(challenge)).await.0,
            StatusCode::CONFLICT,
            "once"
        );
        let (_, state) = viewer.get(&format!("/joins/{id}")).await;
        assert_eq!(state["challenge"]["nonce"], challenge.nonce);
        let (status, _) =
            viewer.post(&format!("/joins/{id}/reveal"), &json!({"v": 1, "nonce": encode_nonce(&[8; 32])})).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "a nonce that does not open the commitment");
        let (status, body) =
            viewer.post(&format!("/joins/{id}/reveal"), &json!({"v": 1, "nonce": encode_nonce(&nj)})).await;
        assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
        let (_, request) = self.primary.get(&format!("/joins/{id}")).await;
        assert_eq!(request["reveal"]["nonce"], encode_nonce(&nj));
        // Both phones compute the same six digits from what each saw.
        let transcript = Transcript {
            account: &self.user,
            join_id: &id,
            device_id: &viewer.device_id,
            box_pub: &vk.box_pub,
            sign_pub: &vk.sign_pub,
            primary_sign_pub: &self.pk.sign_pub,
            owner_pub: &challenge.owner_pub,
            epoch: challenge.epoch,
            expires_at: challenge.expires_at,
            joiner_nonce: nj,
            primary_nonce: np,
        };
        assert_eq!(transcript.code().expect("code").len(), 7);

        let (status, body) = self.primary.post("/roster", &json!({"entry": entry})).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        self.chain.push(entry);
        let answer = json!({"v": 1, "approve": true, "sealed": "U0VBTEVE"});
        // With a roster, a join's answer carries the primary's signature too.
        let (status, body) = self.primary.post(&format!("/joins/{id}/response"), &answer).await;
        assert_eq!((status, error(&body)), (StatusCode::FORBIDDEN, "signature_required"));
        let signed = signed_for(self, AnswerKind::Join, &id, &answer);
        assert_eq!(self.primary.post(&format!("/joins/{id}/response"), &signed).await.0, StatusCode::NO_CONTENT);
    }

    fn op(&self, author: &Phone, k: &Keys, counter: u64, class: OpClass) -> Op {
        let mut op = Op {
            v: 1,
            account: self.user.clone(),
            epoch: self.current().epoch,
            author: author.device_id.clone(),
            counter,
            class,
            notify: false,
            body: encode_payload(format!("sealed op {counter} of {}", author.device_id).as_bytes()),
            sig: String::new(),
        };
        op.sig = sig(&k.sign, &op.signing_bytes().expect("signing bytes"));
        op
    }
}

async fn post_ops(phone: &Phone, ops: &[Op]) -> (StatusCode, Value) {
    phone.post("/ops", &json!({"ops": ops})).await
}

/// A10: the approval device signs `id` out, with the recovery code.
async fn sign_out_device(by: &Phone, id: &str) -> (StatusCode, Value) {
    let response = by
        .raw(reqwest::Method::DELETE, &format!("/devices/{id}"))
        .json(&json!({"master_password_hash": PASSWORD_HASH}))
        .send()
        .await
        .expect("sign out device");
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    (status, serde_json::from_str(&text).unwrap_or(Value::Null))
}

#[tokio::test]
async fn roster_create_enroll_handover_and_recovery() {
    let server = Server::start().await;
    let a = server.phone("rosa@example.com").await;
    a.register_device().await;
    let (status, body) = a.get("/roster").await;
    assert_eq!((status, body), (StatusCode::OK, json!({"entries": []})), "no roster yet");
    // Only the approval device creates the roster.
    let stranger = server.second_device("rosa@example.com").await;
    let sk = keys(9);
    let user = account_of(&a);
    let own = owner(70);
    let e1 = signed(
        RosterEntry::genesis(
            &user,
            member(&stranger, &sk, "Other"),
            public(&own),
            encode_key(&[1; 32]),
            wrap_hash(WRAP),
            1,
        ),
        &own,
    );
    let (status, body) = stranger
        .post("/roster", &json!({"entry": e1, "recovery_wrap": WRAP, "master_password_hash": PASSWORD_HASH}))
        .await;
    assert_eq!((status, error(&body)), (StatusCode::FORBIDDEN, "not_approval_device"));

    let mut acct = Account::with_roster(server, a).await;
    let (_, roster) = acct.primary.get("/roster").await;
    assert_eq!(roster["entries"].as_array().map(Vec::len), Some(1));
    assert!(roster.get("recovery_wrap").is_none(), "the wrap is not handed out with the roster");
    // The legacy state is read-only now.
    let (status, body) = acct.primary.put("/account-state", &json!({"revision": 0, "ciphertext": "c2VhbGVk"})).await;
    assert_eq!((status, error(&body)), (StatusCode::GONE, "legacy_retired"));
    assert_eq!(acct.primary.get("/account-state").await.0, StatusCode::OK);

    // Enroll a viewer.
    let b = acct.server.second_device("rosa@example.com").await;
    let bk = keys(2);
    acct.enroll(&b, &bk, "Pixel").await;
    assert_eq!(b.get("/sync?wait=0").await.1["epoch"], 2);
    assert_eq!(b.put("/member", &json!({"fcm_token": "tok-b"})).await.0, StatusCode::NO_CONTENT);
    // A viewer reads connections but removes none, and does not approve.
    assert_eq!(b.get("/connections").await.0, StatusCode::OK);
    assert_eq!(b.delete("/connections/x").await.0, StatusCode::FORBIDDEN);
    assert_eq!(error(&b.get("/pending?wait=0").await.1), "not_approval_device");
    // The role does not move by a proof any more.
    let (status, body) = b.put("/device", &json!({"master_password_hash": PASSWORD_HASH})).await;
    assert_eq!((status, error(&body)), (StatusCode::CONFLICT, "use_handover"));
    // The same device id without the enrolled device key is not the member.
    let spoof =
        acct.server.device_with_id("rosa@example.com", &b.device_id, Some(crate::harness::new_device_key())).await;
    assert_eq!(error(&spoof.get("/sync?wait=0").await.1), "not_a_member");

    // Handover: b asks, a hands over with the sealed bundle.
    let (status, created) = b.post("/handovers", &json!({"v": 1})).await;
    assert_eq!(status, StatusCode::OK, "{created}");
    let handover_id = created["id"].as_str().expect("id").to_owned();
    let (_, pending) = acct.primary.get("/pending?wait=0").await;
    assert_eq!(pending["handovers"][0]["id"].as_str(), Some(handover_id.as_str()));
    assert_eq!(pending["log_head"], 0, "a roster account's poll carries the log head");
    let e3 = signed(
        rewrapped(
            acct.current()
                .successor(
                    RosterReason::Handover,
                    member(&b, &bk, "Pixel"),
                    vec![member(&acct.primary, &acct.pk, "iPhone")],
                    3,
                )
                .expect("next"),
            HANDOVER_WRAP,
        ),
        &acct.owner,
    );
    let (status, body) = acct.primary.post("/roster", &json!({"entry": e3, "recovery_wrap": HANDOVER_WRAP})).await;
    assert_eq!((status, error(&body)), (StatusCode::BAD_REQUEST, "bad_roster"), "a handover carries the bundle");
    let (status, body) = acct.primary.post("/roster", &json!({"entry": e3, "bundle": "QlVORExF"})).await;
    assert_eq!((status, error(&body)), (StatusCode::BAD_REQUEST, "bad_roster"), "and its new recovery wrap");
    // Naming ops of the outgoing primary the server never stored (a modified primary vouching for ops to come): refused.
    let mut overcounted = e3.clone();
    overcounted.primary_counter = Some(u64::MAX);
    let overcounted = signed(overcounted, &acct.owner);
    let (status, body) = acct
        .primary
        .post("/roster", &json!({"entry": overcounted, "bundle": "QlVORExF", "recovery_wrap": HANDOVER_WRAP}))
        .await;
    assert_eq!((status, error(&body)), (StatusCode::BAD_REQUEST, "bad_roster"), "{body}");
    let with_both = json!({"entry": e3, "bundle": "QlVORExF", "recovery_wrap": HANDOVER_WRAP});
    let (status, body) = acct.primary.post("/roster", &with_both).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    acct.chain.push(e3);
    assert_eq!(b.get("/pending?wait=0").await.0, StatusCode::OK, "b approves now");
    assert_eq!(error(&acct.primary.get("/pending?wait=0").await.1), "not_approval_device");
    assert_eq!(acct.primary.get("/sync?wait=0").await.0, StatusCode::OK, "a views");
    let (_, state) = b.get(&format!("/handovers/{handover_id}")).await;
    assert_eq!(state, json!({"status": "done", "epoch": 3}));
    let (status, bundle) = b.get("/handover-bundle").await;
    assert_eq!((status, bundle), (StatusCode::OK, json!({"epoch": 3, "sealed": "QlVORExF"})));
    assert_eq!(acct.primary.get("/handover-bundle").await.0, StatusCode::FORBIDDEN);
    assert_eq!(b.delete("/handover-bundle").await.0, StatusCode::NO_CONTENT);
    assert_eq!(b.get("/handover-bundle").await.0, StatusCode::NOT_FOUND);
    assert_eq!(b.put("/device", &json!({"fcm_token": "tok-b2"})).await.0, StatusCode::OK, "the primary re-registers");

    // b is lost: a fresh phone recovers with the recovery code, a new owner key, signed by the old one.
    let c = acct.server.second_device("rosa@example.com").await;
    let ck = keys(3);
    let new_owner = owner(51);
    let mut e4 = acct
        .current()
        .successor(
            RosterReason::Recovery,
            member(&c, &ck, "New iPhone"),
            vec![member(&acct.primary, &acct.pk, "iPhone")],
            4,
        )
        .expect("next");
    e4.owner_pub = public(&new_owner);
    e4.recovery_wrap_hash = wrap_hash(NEW_WRAP);
    // b stored no op: the entry may name none past 0.
    e4.primary_counter = Some(0);
    let e4 = signed(e4, &acct.owner);
    let (status, body) = c.post("/roster", &json!({"entry": e4})).await;
    assert_eq!((status, error(&body)), (StatusCode::FORBIDDEN, "proof_required"));
    let (status, body) = c.post("/roster", &json!({"entry": e4, "master_password_hash": "wrong"})).await;
    assert_eq!((status, error(&body)), (StatusCode::FORBIDDEN, "wrong_proof"));
    let forged = signed(e4.clone(), &new_owner);
    let (status, body) = c.post("/roster", &json!({"entry": forged, "master_password_hash": PASSWORD_HASH})).await;
    assert_eq!((status, error(&body)), (StatusCode::FORBIDDEN, "bad_signature"), "the old owner key signs a recovery");
    let (status, body) = c.post("/roster", &json!({"entry": e4, "master_password_hash": PASSWORD_HASH})).await;
    assert_eq!((status, error(&body)), (StatusCode::BAD_REQUEST, "bad_roster"), "a recovery brings its new wrap");
    let with_old = json!({"entry": e4, "master_password_hash": PASSWORD_HASH, "recovery_wrap": WRAP});
    let (status, body) = c.post("/roster", &with_old).await;
    assert_eq!((status, error(&body)), (StatusCode::BAD_REQUEST, "bad_roster"), "not the wrap the entry names");
    let mut overcounted = e4.clone();
    overcounted.primary_counter = Some(1);
    let overcounted = signed(overcounted, &acct.owner);
    let over = json!({"entry": overcounted, "master_password_hash": PASSWORD_HASH, "recovery_wrap": NEW_WRAP});
    let (status, body) = c.post("/roster", &over).await;
    assert_eq!((status, error(&body)), (StatusCode::BAD_REQUEST, "bad_roster"), "b stored no op: {body}");
    let recovery = json!({"entry": e4, "master_password_hash": PASSWORD_HASH, "recovery_wrap": NEW_WRAP});
    let (status, body) = c.post("/roster", &recovery).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (_, wrap) = c.post("/roster/recovery-wrap", &json!({"master_password_hash": PASSWORD_HASH})).await;
    assert_eq!(
        wrap,
        json!({"epoch": 4, "recovery_wrap": NEW_WRAP, "primary_counter": 0}),
        "the recovery replaced the wrap"
    );
    acct.chain.push(e4);
    assert_eq!(c.get("/pending?wait=0").await.0, StatusCode::OK);
    assert_eq!(error(&b.get("/pending?wait=0").await.1), "not_approval_device");
    assert_eq!(error(&b.get("/sync?wait=0").await.1), "not_a_member", "the old primary is removed");
    assert_eq!(acct.primary.get("/sync?wait=0").await.1["epoch"], 4, "the kept viewer follows");
    let (status, body) = b.put("/device", &json!({})).await;
    assert_eq!((status, error(&body)), (StatusCode::CONFLICT, "use_handover"));
    assert_eq!(c.put("/device", &json!({"fcm_token": "tok-c"})).await.0, StatusCode::OK);
    let (_, roster) = c.get("/roster").await;
    assert_eq!(roster["entries"].as_array().map(Vec::len), Some(4));
}

#[tokio::test]
async fn an_epoch_rollback_or_replay_is_refused() {
    let mut acct = Account::new("eve@example.com").await;
    let b = acct.server.second_device("eve@example.com").await;
    acct.enroll(&b, &keys(2), "Pixel").await;
    // The same entry again, an older one, and another entry for an epoch already taken.
    for entry in [acct.chain[1].clone(), acct.chain[0].clone()] {
        let (status, body) = acct.primary.post("/roster", &json!({"entry": entry, "recovery_wrap": "d3JhcA"})).await;
        assert_eq!((status, error(&body)), (StatusCode::CONFLICT, "epoch_conflict"), "{body}");
    }
    let fork = signed(
        acct.chain[0]
            .successor(RosterReason::Enroll, acct.chain[0].primary.clone(), vec![member(&b, &keys(4), "Fork")], 9)
            .expect("next"),
        &acct.owner,
    );
    let (status, body) = acct.primary.post("/roster", &json!({"entry": fork})).await;
    assert_eq!((status, error(&body)), (StatusCode::CONFLICT, "epoch_conflict"));
    // A skipped epoch breaks the chain.
    let mut skip =
        acct.current().successor(RosterReason::Remove, acct.current().primary.clone(), vec![], 9).expect("next");
    skip.epoch += 1;
    let skip = signed(skip, &acct.owner);
    let (status, body) = acct.primary.post("/roster", &json!({"entry": skip})).await;
    assert_eq!((status, error(&body)), (StatusCode::CONFLICT, "epoch_conflict"));
    let (_, roster) = acct.primary.get("/roster").await;
    assert_eq!(roster["entries"].as_array().map(Vec::len), Some(2));
}

#[tokio::test]
async fn ops_are_checked_against_the_roster() {
    let mut acct = Account::new("otto@example.com").await;
    let b = acct.server.second_device("otto@example.com").await;
    let bk = keys(2);
    acct.enroll(&b, &bk, "Pixel").await;

    // A viewer's primary-class op is refused; its brake and its proposal are taken.
    let (status, body) = post_ops(&b, &[acct.op(&b, &bk, 1, OpClass::Primary)]).await;
    assert_eq!((status, error(&body)), (StatusCode::FORBIDDEN, "op_refused"), "{body}");
    let (status, body) = post_ops(&b, &[acct.op(&b, &bk, 1, OpClass::Restrictive)]).await;
    assert_eq!((status, &body["seqs"]), (StatusCode::OK, &json!([1])), "{body}");
    let (status, body) = post_ops(&b, &[acct.op(&b, &bk, 2, OpClass::Proposal)]).await;
    assert_eq!((status, &body["seqs"]), (StatusCode::OK, &json!([2])), "{body}");

    // The primary writes primary ops.
    let first = acct.op(&acct.primary, &acct.pk, 1, OpClass::Primary);
    let (status, body) =
        post_ops(&acct.primary, &[first.clone(), acct.op(&acct.primary, &acct.pk, 2, OpClass::Primary)]).await;
    assert_eq!((status, &body["seqs"], &body["head_seq"]), (StatusCode::OK, &json!([3, 4]), &json!(4)), "{body}");
    // The same op again is a retry (its seq comes back); another op with a used counter is a replay.
    let (status, body) = post_ops(&acct.primary, &[first]).await;
    assert_eq!((status, &body["seqs"]), (StatusCode::OK, &json!([3])), "{body}");
    let mut replay = acct.op(&acct.primary, &acct.pk, 2, OpClass::Primary);
    replay.notify = true;
    replay.sig = sig(&acct.pk.sign, &replay.signing_bytes().unwrap());
    let (status, body) = post_ops(&acct.primary, &[replay]).await;
    assert_eq!((status, error(&body)), (StatusCode::CONFLICT, "counter_replay"), "{body}");

    // Signed by another key, posted for another author, or relabelled after signing: refused.
    let forged = {
        let mut op = acct.op(&b, &bk, 3, OpClass::Restrictive);
        op.sig = sig(&keys(5).sign, &op.signing_bytes().unwrap());
        op
    };
    assert_eq!(error(&post_ops(&b, &[forged]).await.1), "bad_signature");
    let mut relabelled = acct.op(&b, &bk, 3, OpClass::Restrictive);
    relabelled.class = OpClass::Proposal;
    assert_eq!(error(&post_ops(&b, &[relabelled]).await.1), "bad_signature");
    assert_eq!(error(&post_ops(&b, &[acct.op(&acct.primary, &acct.pk, 9, OpClass::Primary)]).await.1), "op_refused");
    // An op for an epoch the roster does not have.
    let mut future = acct.op(&b, &bk, 3, OpClass::Restrictive);
    future.epoch = 9;
    future.sig = sig(&bk.sign, &future.signing_bytes().unwrap());
    assert_eq!(error(&post_ops(&b, &[future]).await.1), "op_refused");
    // Not a member: refused before anything else.
    let stranger = acct.server.second_device("otto@example.com").await;
    assert_eq!(
        error(&post_ops(&stranger, &[acct.op(&stranger, &keys(6), 1, OpClass::Proposal)]).await.1),
        "not_a_member"
    );

    // What viewers read: every op, signed as written.
    let (_, sync) = b.get("/sync?after=0&wait=0").await;
    let seqs: Vec<u64> = sync["ops"].as_array().unwrap().iter().map(|o| o["seq"].as_u64().unwrap()).collect();
    assert_eq!(seqs, [1, 2, 3, 4]);
    assert_eq!(sync["ops"][0]["op"]["class"], "restrictive");
    assert_eq!(sync["head_seq"], 4);
}

#[tokio::test]
async fn sync_long_polls_wake_on_ops_heartbeats_and_waiting_items() {
    let server = Server::start().await;
    let phone = server.phone("sam@example.com").await;
    phone.register_device().await;
    let tokens = server.connect_ai(&phone, "sam@example.com", Some("Claude")).await;
    let mut acct = Account::with_roster(server, phone).await;
    let b = acct.server.second_device("sam@example.com").await;
    let bk = keys(2);
    acct.enroll(&b, &bk, "Pixel").await;
    let (_, first) = b.get("/sync?wait=0").await;
    let cursor = first["cursor"].as_str().unwrap().to_owned();

    // Nothing new: the poll waits for its time.
    let start = Instant::now();
    let (_, idle) = b.get(&format!("/sync?after=0&seen={cursor}&wait=1")).await;
    assert!(start.elapsed() >= Duration::from_millis(900), "waited");
    assert_eq!(idle["ops"], json!([]));

    // An op wakes it.
    let start = Instant::now();
    let path = format!("/sync?after=0&seen={cursor}&wait=20");
    let poll = b.get(&path);
    let write = async {
        tokio::time::sleep(Duration::from_millis(300)).await;
        post_ops(&acct.primary, &[acct.op(&acct.primary, &acct.pk, 1, OpClass::Primary)]).await
    };
    let ((status, woke), (written, _)) = tokio::join!(poll, write);
    assert_eq!((status, written), (StatusCode::OK, StatusCode::OK));
    assert_eq!(woke["ops"][0]["seq"], 1);
    assert!(start.elapsed() < Duration::from_secs(10), "woken by the op, not the timeout");
    let cursor = woke["cursor"].as_str().unwrap().to_owned();

    // A heartbeat wakes it and is served as signed.
    let mut hb = Heartbeat {
        v: 1,
        account: acct.user.clone(),
        epoch: acct.current().epoch,
        author: acct.primary.device_id.clone(),
        head_seq: 1,
        applied_hash: hex32(&[3; 32]),
        t: chrono_now(),
        last_counter: Some(1),
        sig: String::new(),
    };
    hb.sig = sig(&acct.pk.sign, &hb.signing_bytes().unwrap());
    let path = format!("/sync?after=1&seen={cursor}&wait=20");
    let poll = b.get(&path);
    let beat = async {
        tokio::time::sleep(Duration::from_millis(300)).await;
        acct.primary.post("/heartbeat", &json!(hb)).await
    };
    let ((_, woke), (status, body)) = tokio::join!(poll, beat);
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    assert_eq!(woke["heartbeat"], json!(hb));
    let cursor = woke["cursor"].as_str().unwrap().to_owned();
    let (status, body) = acct.primary.post("/heartbeat", &json!(hb)).await;
    assert_eq!((status, error(&body)), (StatusCode::TOO_MANY_REQUESTS, "rate_limited"), "one per 10 s");
    let mut viewer_beat = hb.clone();
    viewer_beat.author = b.device_id.clone();
    viewer_beat.sig = sig(&bk.sign, &viewer_beat.signing_bytes().unwrap());
    assert_eq!(b.post("/heartbeat", &json!(viewer_beat)).await.0, StatusCode::FORBIDDEN, "only the primary");

    // A request from an AI wakes it, and looking does not take it from the primary.
    let ai = tokio::spawn(mcp_tool(acct.server.url("/mcp"), tokens.access.clone()));
    let start = Instant::now();
    let (_, woke) = b.get(&format!("/sync?after=1&seen={cursor}&wait=20")).await;
    assert!(start.elapsed() < Duration::from_secs(10), "woken by the request");
    let request_id = woke["waiting"]["requests"][0]["id"].as_str().expect("the viewer sees the request").to_owned();
    assert_eq!(woke["waiting"]["requests"][0]["call"]["tool"], "gmail_read");
    let cursor = woke["cursor"].as_str().unwrap().to_owned();
    let (_, pending) = acct.primary.get("/pending?wait=10").await;
    assert_eq!(pending["requests"][0]["id"].as_str(), Some(request_id.as_str()), "the primary still gets it");
    // Its answer wakes the viewer again: the request moves from waiting to answered.
    let path = format!("/sync?after=1&seen={cursor}&wait=20");
    let poll = b.get(&path);
    let answer = async {
        tokio::time::sleep(Duration::from_millis(300)).await;
        let answer = signed_answer(&acct, &request_id, &json!({"v": 1, "outcome": "denied", "reason": null}));
        acct.primary.post(&format!("/requests/{request_id}/response"), &answer).await.0
    };
    let ((_, after), answered) = tokio::join!(poll, answer);
    assert_eq!(answered, StatusCode::NO_CONTENT);
    assert!(after["answered"].as_array().unwrap().iter().any(|a| a == &json!(request_id)), "{after}");
    assert_eq!(after["waiting"]["requests"], json!([]));
    assert_eq!(ai.await.expect("ai task")["isError"], true);

    // Acknowledgements: the lowest of the members' is what tombstones may be dropped up to.
    assert_eq!(b.post("/ack", &json!({"seq": 1})).await.0, StatusCode::NO_CONTENT);
    assert_eq!(b.get("/sync?wait=0").await.1["min_acked_seq"], 0, "the primary has not acknowledged");
    assert_eq!(acct.primary.post("/ack", &json!({"seq": 99})).await.0, StatusCode::NO_CONTENT);
    assert_eq!(b.get("/sync?wait=0").await.1["min_acked_seq"], 1, "capped at the head");

    // Checkpoints: the primary writes, members read.
    let mut cp = Checkpoint {
        v: 1,
        account: acct.user.clone(),
        epoch: acct.current().epoch,
        author: acct.primary.device_id.clone(),
        seq: 1,
        applied_hash: hex32(&[3; 32]),
        at: 1,
        body: encode_payload(b"sealed snapshot"),
        sig: String::new(),
    };
    cp.sig = sig(&acct.pk.sign, &cp.signing_bytes().unwrap());
    assert_eq!(b.put("/checkpoint", &json!(cp)).await.0, StatusCode::FORBIDDEN);
    assert_eq!(acct.primary.put("/checkpoint", &json!(cp)).await.0, StatusCode::NO_CONTENT);
    assert_eq!(acct.primary.put("/checkpoint", &json!(cp)).await.0, StatusCode::CONFLICT, "not above the latest");
    assert_eq!(b.get("/checkpoint").await.1, json!(cp));
    assert_eq!(b.get("/sync?wait=0").await.1["checkpoint_seq"], 1);
}

#[tokio::test]
async fn answers_need_the_primarys_signature_once_there_is_a_roster() {
    let server = Server::start().await;
    let phone = server.phone("ana@example.com").await;
    phone.register_device().await;
    let tokens = server.connect_ai(&phone, "ana@example.com", Some("Claude")).await;
    let mut acct = Account::with_roster(server, phone).await;
    let viewer = acct.server.second_device("ana@example.com").await;
    let vk = keys(2);
    acct.enroll(&viewer, &vk, "Pixel").await;

    let ai = mcp_tool(acct.server.url("/mcp"), tokens.access.clone());
    let phone_side = async {
        let (_, pending) = acct.primary.get("/pending?wait=10").await;
        let id = pending["requests"][0]["id"].as_str().expect("request").to_owned();
        let denied = json!({"v": 1, "outcome": "denied", "reason": null});
        // From the viewer's device: refused, whether it relays the primary's signed answer or signs its own.
        let bytes = serde_json::to_vec(&denied).unwrap();
        let mut by_viewer = signed_answer(&acct, &id, &denied);
        by_viewer["sig"] =
            json!(sig(&vk.sign, &answer_message(&acct.user, AnswerKind::Request, &id, acct.current().epoch, &bytes)));
        for body in [signed_answer(&acct, &id, &denied), by_viewer] {
            let (status, refused) = viewer.post(&format!("/requests/{id}/response"), &body).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{refused}");
        }
        // Unsigned, as before phone roles: refused.
        let (status, body) = acct.primary.post(&format!("/requests/{id}/response"), &denied).await;
        assert_eq!((status, error(&body)), (StatusCode::FORBIDDEN, "signature_required"));
        // Signed by another key, or for another request: refused.
        let mut wrong_key = signed_answer(&acct, &id, &denied);
        let bytes = serde_json::to_vec(&denied).unwrap();
        wrong_key["sig"] = json!(sig(
            &keys(8).sign,
            &answer_message(&acct.user, AnswerKind::Request, &id, acct.current().epoch, &bytes)
        ));
        let (status, body) = acct.primary.post(&format!("/requests/{id}/response"), &wrong_key).await;
        assert_eq!((status, error(&body)), (StatusCode::FORBIDDEN, "bad_signature"));
        let other = signed_answer(&acct, "another-request", &denied);
        let (status, body) = acct.primary.post(&format!("/requests/{id}/response"), &other).await;
        assert_eq!((status, error(&body)), (StatusCode::FORBIDDEN, "bad_signature"));
        // Signed by the primary: taken.
        let (status, body) =
            acct.primary.post(&format!("/requests/{id}/response"), &signed_answer(&acct, &id, &denied)).await;
        assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    };
    let (result, ()) = tokio::join!(ai, phone_side);
    assert_eq!(result["isError"], true, "{result}");
    assert_eq!(result["content"][0]["text"], "Denied by the user on their Reins device.");
}

#[tokio::test]
async fn a_legacy_account_is_unaffected() {
    let server = Server::start().await;
    let phone = server.phone("leo@example.com").await;
    phone.register_device().await;
    let tokens = server.connect_ai(&phone, "leo@example.com", Some("Claude")).await;
    assert_eq!(phone.get("/roster").await.1, json!({"entries": []}));
    // The poll looks as it always did.
    assert_eq!(phone.get("/pending?wait=0").await.1, json!({"requests": [], "pairings": []}));
    // The account state is written as before.
    let (status, body) = phone.put("/account-state", &json!({"revision": 0, "ciphertext": "c2VhbGVk"})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // Unsigned answers count.
    let ai = mcp_tool(server.url("/mcp"), tokens.access.clone());
    let phone_side = async {
        let (_, pending) = phone.get("/pending?wait=10").await;
        let id = pending["requests"][0]["id"].as_str().expect("request").to_owned();
        phone.post(&format!("/requests/{id}/response"), &json!({"v": 1, "outcome": "denied", "reason": null})).await.0
    };
    let (result, status) = tokio::join!(ai, phone_side);
    assert_eq!((status, &result["isError"]), (StatusCode::NO_CONTENT, &json!(true)));
    // The new endpoints say there is no roster; nothing else changes.
    assert_eq!(error(&phone.get("/sync?wait=0").await.1), "no_roster");
    assert_eq!(error(&phone.post("/ops", &json!({"ops": []})).await.1), "no_roster");
    // The approval role still moves with a proof.
    let other = server.second_device("leo@example.com").await;
    assert_eq!(other.put("/device", &json!({"master_password_hash": PASSWORD_HASH})).await.0, StatusCode::OK);
}

/// The primary's signed answer to request `id`.
fn signed_answer(acct: &Account, id: &str, answer: &Value) -> Value {
    signed_for(acct, AnswerKind::Request, id, answer)
}

/// The primary's signed answer to item `id` of `kind`.
fn signed_for(acct: &Account, kind: AnswerKind, id: &str, answer: &Value) -> Value {
    let bytes = serde_json::to_vec(answer).unwrap();
    let epoch = acct.current().epoch;
    let message = answer_message(&acct.user, kind, id, epoch, &bytes);
    json!({"v": 1, "epoch": epoch, "answer": encode_payload(&bytes), "sig": sig(&acct.pk.sign, &message)})
}

/// An AI calls a Gmail tool (it waits for the phone's answer); returns the tool result.
async fn mcp_tool(url: String, token: String) -> Value {
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": {"name": "gmail_read", "arguments": {"message_ids": ["m1"]}}});
    let response = client().post(url).bearer_auth(token).json(&body).send().await.expect("mcp");
    let value: Value = response.json().await.expect("json");
    value["result"].clone()
}

fn chrono_now() -> i64 {
    i64::try_from(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()).unwrap()
}

#[tokio::test]
async fn pairing_answers_need_the_primarys_signature_once_there_is_a_roster() {
    let mut acct = Account::new("pia@example.com").await;
    let viewer = acct.server.second_device("pia@example.com").await;
    acct.enroll(&viewer, &keys(2), "Pixel").await;
    let client_id = acct.server.register_client().await;
    let wait_url = acct.server.start_authorization(&client_id, "pia@example.com").await;
    let html = client().get(&wait_url).send().await.unwrap().text().await.unwrap();
    let code: u8 = between(&html, "aria-label=\"code\">", "<").parse().unwrap();
    let (_, pending) = acct.primary.get("/pending?wait=5").await;
    let id = pending["pairings"][0]["id"].as_str().expect("pairing").to_owned();
    let approve = json!({"v": 1, "approved": true, "chosen_code": code, "label": "Claude"});
    let (status, body) = acct.primary.post(&format!("/pairings/{id}/response"), &approve).await;
    assert_eq!((status, error(&body)), (StatusCode::FORBIDDEN, "signature_required"));
    let wrong_kind = signed_for(&acct, AnswerKind::Request, &id, &approve);
    let (status, body) = acct.primary.post(&format!("/pairings/{id}/response"), &wrong_kind).await;
    assert_eq!((status, error(&body)), (StatusCode::FORBIDDEN, "bad_signature"), "signed for a request, not a pairing");
    // The viewer's device relaying the primary's signed answer: refused (only the primary's device answers).
    let signed = signed_for(&acct, AnswerKind::Pairing, &id, &approve);
    let (status, body) = viewer.post(&format!("/pairings/{id}/response"), &signed).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    // So is a join answer from it.
    let joiner = acct.server.second_device("pia@example.com").await;
    let (_, created) = joiner
        .post(
            "/joins",
            &json!({"v": 1, "device_name": "Tablet", "public_key": encode_key(&[4; 32]),
                    "sign_pub": keys(4).sign_pub, "commitment": hex32(&[4; 32])}),
        )
        .await;
    let join = created["id"].as_str().expect("join").to_owned();
    let deny = json!({"v": 1, "approve": false});
    let signed_join = signed_for(&acct, AnswerKind::Join, &join, &deny);
    let (status, body) = viewer.post(&format!("/joins/{join}/response"), &signed_join).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    let (status, body) = acct
        .primary
        .post(&format!("/pairings/{id}/response"), &signed_for(&acct, AnswerKind::Pairing, &id, &approve))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["connection_id"].is_string(), "{body}");
}

#[tokio::test]
async fn the_recovery_wrap_is_bound_to_the_roster_and_given_only_with_a_proof() {
    let mut acct = Account::new("wren@example.com").await;
    let other = acct.server.second_device("wren@example.com").await;
    let (status, body) = other.post("/roster/recovery-wrap", &json!({"master_password_hash": "wrong"})).await;
    assert_eq!((status, error(&body)), (StatusCode::FORBIDDEN, "wrong_proof"));
    let (status, body) = other.post("/roster/recovery-wrap", &json!({"master_password_hash": PASSWORD_HASH})).await;
    assert_eq!((status, body), (StatusCode::OK, json!({"epoch": 1, "recovery_wrap": WRAP, "primary_counter": 0})));
    // Only a rewrap (or a recovery) replaces it, and with the wrap the entry names.
    let b = acct.server.second_device("wren@example.com").await;
    acct.enroll(&b, &keys(2), "Pixel").await;
    let mut rewrap = acct
        .current()
        .successor(RosterReason::Rewrap, acct.current().primary.clone(), acct.current().viewers.clone(), 5)
        .expect("next");
    rewrap.recovery_wrap_hash = wrap_hash(NEW_WRAP);
    let rewrap = signed(rewrap, &acct.owner);
    let (status, body) = acct.primary.post("/roster", &json!({"entry": rewrap})).await;
    assert_eq!((status, error(&body)), (StatusCode::BAD_REQUEST, "bad_roster"), "a rewrap carries the wrap");
    let (status, body) = b.post("/roster", &json!({"entry": rewrap, "recovery_wrap": NEW_WRAP})).await;
    assert_eq!((status, error(&body)), (StatusCode::FORBIDDEN, "not_approval_device"), "the primary rewraps");
    let (status, body) = acct.primary.post("/roster", &json!({"entry": rewrap, "recovery_wrap": NEW_WRAP})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    acct.chain.push(rewrap);
    let (_, wrap) = other.post("/roster/recovery-wrap", &json!({"master_password_hash": PASSWORD_HASH})).await;
    assert_eq!(wrap, json!({"epoch": 3, "recovery_wrap": NEW_WRAP, "primary_counter": 0}));
    // A removal that keeps the wrap in force is refused (an older wrap would still fit the roster after it), and so
    // is one whose wrap is not the one it names.
    let removal = signed(
        acct.current().successor(RosterReason::Remove, acct.current().primary.clone(), vec![], 6).expect("next"),
        &acct.owner,
    );
    let (status, body) = acct.primary.post("/roster", &json!({"entry": removal})).await;
    assert_eq!((status, error(&body)), (StatusCode::BAD_REQUEST, "bad_roster"));
    let removal = signed(
        rewrapped(
            acct.current().successor(RosterReason::Remove, acct.current().primary.clone(), vec![], 6).expect("next"),
            HANDOVER_WRAP,
        ),
        &acct.owner,
    );
    let (status, body) = acct.primary.post("/roster", &json!({"entry": removal, "recovery_wrap": WRAP})).await;
    assert_eq!((status, error(&body)), (StatusCode::BAD_REQUEST, "bad_roster"));
}

#[tokio::test]
async fn viewers_cannot_fill_the_log_and_the_primary_keeps_writing() {
    let options = Options {
        env: vec![("REINS_MEMBER_LOG_MAX_OPS", "2".to_owned())],
        ..Options::default()
    };
    let mut acct = Account::new_with("finn@example.com", options).await;
    let b = acct.server.second_device("finn@example.com").await;
    let bk = keys(2);
    acct.enroll(&b, &bk, "Pixel").await;
    // A restriction is small; a proposal is at most 16 KiB.
    let mut big = acct.op(&b, &bk, 1, OpClass::Restrictive);
    big.body = encode_payload(&[7u8; 2048]);
    big.sig = sig(&bk.sign, &big.signing_bytes().unwrap());
    assert_eq!(error(&post_ops(&b, &[big]).await.1), "bad_request");
    let mut proposal = acct.op(&b, &bk, 1, OpClass::Proposal);
    proposal.body = encode_payload(&vec![7u8; 17 * 1024]);
    proposal.sig = sig(&bk.sign, &proposal.signing_bytes().unwrap());
    assert_eq!(error(&post_ops(&b, &[proposal]).await.1), "bad_request");
    // The viewer's share is two ops.
    let (status, body) =
        post_ops(&b, &[acct.op(&b, &bk, 1, OpClass::Restrictive), acct.op(&b, &bk, 2, OpClass::Restrictive)]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = post_ops(&b, &[acct.op(&b, &bk, 3, OpClass::Restrictive)]).await;
    assert_eq!((status, error(&body)), (StatusCode::CONFLICT, "log_full"));
    // The primary is not held back by it.
    let (status, body) = post_ops(&acct.primary, &[acct.op(&acct.primary, &acct.pk, 1, OpClass::Primary)]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn other_devices_cannot_hide_requests_from_the_primary() {
    let mut acct = Account::new("hana@example.com").await;
    let b = acct.server.second_device("hana@example.com").await;
    acct.enroll(&b, &keys(2), "Pixel").await;
    let c = acct.server.second_device("hana@example.com").await;
    let (_, created) = c
        .post(
            "/joins",
            &json!({"v": 1, "device_name": "Tablet", "public_key": encode_key(&[4; 32]),
                    "sign_pub": keys(4).sign_pub, "commitment": hex32(&[4; 32])}),
        )
        .await;
    let join = created["id"].as_str().unwrap().to_owned();
    let (status, body) = b.get(&format!("/joins/{join}")).await;
    assert_eq!((status, error(&body)), (StatusCode::FORBIDDEN, "not_approval_device"));
    let (_, handover) = b.post("/handovers", &json!({"v": 1})).await;
    let handover = handover["id"].as_str().unwrap().to_owned();
    assert_eq!(error(&c.get(&format!("/handovers/{handover}")).await.1), "not_approval_device");
    // The primary still gets both, as new.
    let (_, pending) = acct.primary.get("/pending?wait=0").await;
    assert_eq!(pending["joins"][0]["id"].as_str(), Some(join.as_str()), "{pending}");
    assert_eq!(pending["handovers"][0]["id"].as_str(), Some(handover.as_str()), "{pending}");
}

#[tokio::test]
async fn a_signed_handover_request_is_kept_and_a_badly_signed_one_refused() {
    let mut acct = Account::new("jon@example.com").await;
    let b = acct.server.second_device("jon@example.com").await;
    let bk = keys(2);
    acct.enroll(&b, &bk, "Pixel").await;
    let epoch = acct.current().epoch;
    let nonce = encode_payload(&[5; 32]);
    let now = chrono_now();
    let signed_at = |by: &Ed25519KeyPair, device: &str, epoch: u64, at: i64| {
        let message =
            reins_proto::roster::handover_request_message(&acct.user, epoch, device, &nonce, at).expect("message");
        json!({"v": 1, "signed": {"epoch": epoch, "nonce": nonce, "at": at, "sig": sig(by, &message)}})
    };
    let signed = |by: &Ed25519KeyPair, device: &str, epoch: u64| signed_at(by, device, epoch, now);
    // Signed by another key, for another phone, at another epoch, or long ago: refused.
    for body in [
        signed(&keys(9).sign, &b.device_id, epoch),
        signed(&bk.sign, &acct.primary.device_id, epoch),
        signed(&bk.sign, &b.device_id, epoch + 1),
        signed_at(&bk.sign, &b.device_id, epoch, now - 3600),
    ] {
        let (status, refused) = b.post("/handovers", &body).await;
        assert_eq!((status, error(&refused)), (StatusCode::FORBIDDEN, "bad_signature"), "{body}");
    }
    // A join over one key (no enrollment) is refused once the account has a roster.
    let c = acct.server.second_device("jon@example.com").await;
    let (status, body) =
        c.post("/joins", &json!({"v": 1, "device_name": "Old", "public_key": encode_key(&[4; 32])})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    // Signed by the viewer's roster key: the primary gets the signature with the request.
    let (status, created) = b.post("/handovers", &signed(&bk.sign, &b.device_id, epoch)).await;
    assert_eq!(status, StatusCode::OK, "{created}");
    let (_, pending) = acct.primary.get("/pending?wait=0").await;
    assert_eq!(pending["handovers"][0]["signed"]["epoch"], epoch, "{pending}");
    // The same signed request again is a replay.
    let (status, body) = b.post("/handovers", &signed(&bk.sign, &b.device_id, epoch)).await;
    assert_eq!((status, error(&body)), (StatusCode::CONFLICT, "already_answered"));
}

#[tokio::test]
async fn signed_extreme_handover_times_are_refused_without_overflow_or_consuming_the_nonce() {
    let mut acct = Account::new("jon@example.com").await;
    let viewer = acct.server.second_device("jon@example.com").await;
    let key = keys(2);
    acct.enroll(&viewer, &key, "Pixel").await;
    let epoch = acct.current().epoch;
    let nonce = encode_payload(&[8; 32]);
    for at in [i64::MIN, i64::MAX, chrono_now()] {
        let message =
            reins_proto::roster::handover_request_message(&acct.user, epoch, &viewer.device_id, &nonce, at).unwrap();
        let body = json!({"v":1,"signed":{"epoch":epoch,"nonce":nonce,"at":at,"sig":sig(&key.sign,&message)}});
        let (status, response) = viewer.post("/handovers", &body).await;
        if matches!(at, i64::MIN | i64::MAX) {
            assert_eq!((status, error(&response)), (StatusCode::FORBIDDEN, "bad_signature"));
        } else {
            assert_eq!(status, StatusCode::OK, "{response}");
        }
    }
}

#[tokio::test]
async fn handover_policy_support_is_verified_and_forwarded_without_changing_legacy_signatures() {
    let mut acct = Account::new("jon@example.com").await;
    let viewer = acct.server.second_device("jon@example.com").await;
    let key = keys(2);
    acct.enroll(&viewer, &key, "Pixel").await;
    let epoch = acct.current().epoch;
    let nonce = encode_payload(&[6; 32]);
    let now = chrono_now();
    let legacy =
        reins_proto::roster::handover_request_message(&acct.user, epoch, &viewer.device_id, &nonce, now).unwrap();
    let proof =
        reins_proto::roster::handover_policy_support_message(&acct.user, epoch, &viewer.device_id, &nonce, now, &[1])
            .unwrap();
    let body = json!({"v":1,"signed":{"epoch":epoch,"nonce":nonce,"at":now,"sig":sig(&key.sign,&legacy),"policy_enforcement":{"authorization_versions":[1],"sig":sig(&key.sign,&proof)}}});
    let mut bad = body.clone();
    bad["signed"]["policy_enforcement"]["authorization_versions"] = json!([2]);
    let (status, refused) = viewer.post("/handovers", &bad).await;
    assert_eq!((status, error(&refused)), (StatusCode::FORBIDDEN, "bad_signature"));
    let mut bad = body.clone();
    bad["signed"]["policy_enforcement"]["sig"] = body["signed"]["sig"].clone();
    let (status, refused) = viewer.post("/handovers", &bad).await;
    assert_eq!((status, error(&refused)), (StatusCode::FORBIDDEN, "bad_signature"));
    let (status, created) = viewer.post("/handovers", &body).await;
    assert_eq!(status, StatusCode::OK, "{created}");
    let (_, pending) = acct.primary.get("/pending?wait=0").await;
    assert_eq!(pending["handovers"][0]["signed"]["policy_enforcement"], body["signed"]["policy_enforcement"]);
}

#[tokio::test]
async fn a_roster_is_created_only_with_the_password_and_proofs_are_counted_per_device() {
    let options = Options {
        env: vec![("REINS_DEVICE_PROOF_MAX_FAILURES", "3".to_owned())],
        ..Options::default()
    };
    let server = Server::start_with(options).await;
    let a = server.phone("una@example.com").await;
    a.register_device().await;
    let user = account_of(&a);
    let own = owner(70);
    let e1 = signed(
        RosterEntry::genesis(
            &user,
            member(&a, &keys(1), "iPhone"),
            public(&own),
            encode_key(&[1; 32]),
            wrap_hash(WRAP),
            1,
        ),
        &own,
    );
    // Without the master password hash, or with a wrong one: no roster (the wrap would open for nobody).
    let (status, body) = a.post("/roster", &json!({"entry": e1, "recovery_wrap": WRAP})).await;
    assert_eq!((status, error(&body)), (StatusCode::FORBIDDEN, "proof_required"));
    let wrong = json!({"entry": e1, "recovery_wrap": WRAP, "master_password_hash": "not-it"});
    let (status, body) = a.post("/roster", &wrong).await;
    assert_eq!((status, error(&body)), (StatusCode::FORBIDDEN, "wrong_proof"));
    let right = json!({"entry": e1, "recovery_wrap": WRAP, "master_password_hash": PASSWORD_HASH});
    assert_eq!(a.post("/roster", &right).await.0, StatusCode::OK);
    // Guesses from ever new device ids (with the wrong one above, three for the account): each device has tries left,
    // but the account's limit, shared with every other proof, stops them all.
    let guess = json!({"master_password_hash": "guess"});
    for _ in 0..2 {
        let device = server.second_device("una@example.com").await;
        assert_eq!(device.post("/roster/recovery-wrap", &guess).await.0, StatusCode::FORBIDDEN);
    }
    let fresh = server.second_device("una@example.com").await;
    let (status, body) = fresh.post("/roster/recovery-wrap", &guess).await;
    assert_eq!((status, error(&body)), (StatusCode::TOO_MANY_REQUESTS, "rate_limited"), "the account's limit");
    // The phone that proved itself before keeps its own tries: the flood does not lock the owner out.
    let (status, body) = a.post("/roster/recovery-wrap", &json!({"master_password_hash": PASSWORD_HASH})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn one_devices_wrong_proofs_leave_another_devices_tries_alone() {
    let options = Options {
        env: vec![
            ("REINS_DEVICE_PROOF_MAX_FAILURES", "10".to_owned()),
            ("REINS_DEVICE_PROOF_MAX_FAILURES_PER_DEVICE", "2".to_owned()),
        ],
        ..Options::default()
    };
    let acct = Account::new_with("vic@example.com", options).await;
    let thief = acct.server.second_device("vic@example.com").await;
    let other = acct.server.second_device("vic@example.com").await;
    let guess = json!({"master_password_hash": "guess"});
    for _ in 0..2 {
        assert_eq!(thief.post("/roster/recovery-wrap", &guess).await.0, StatusCode::FORBIDDEN);
    }
    let (status, body) = thief.post("/roster/recovery-wrap", &guess).await;
    assert_eq!((status, error(&body)), (StatusCode::TOO_MANY_REQUESTS, "rate_limited"), "its own tries are used up");
    // Another device still has its own: a wrong guess is a wrong guess, a right one gets the wrap.
    let (status, body) = other.post("/roster/recovery-wrap", &guess).await;
    assert_eq!((status, error(&body)), (StatusCode::FORBIDDEN, "wrong_proof"));
    let (status, body) = other.post("/roster/recovery-wrap", &json!({"master_password_hash": PASSWORD_HASH})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn proven_exemption_follows_the_device_key_and_ends_when_the_device_is_gone() {
    let options = Options {
        env: vec![("REINS_DEVICE_PROOF_MAX_FAILURES", "2".to_owned())],
        ..Options::default()
    };
    let mut acct = Account::new_with("willa@example.com", options).await;
    let email = "willa@example.com";
    let right = json!({"master_password_hash": PASSWORD_HASH});
    let viewer = acct.server.second_device(email).await;
    acct.enroll(&viewer, &keys(2), "Pixel").await;
    assert_eq!(viewer.post("/roster/recovery-wrap", &right).await.0, StatusCode::OK, "the viewer proves its key");

    // A right proof with no device key is accepted and grants no exemption.
    let nokey = acct.server.device_with_id(email, &uuid::Uuid::new_v4().to_string(), None).await;
    assert_eq!(nokey.post("/roster/recovery-wrap", &right).await.0, StatusCode::OK);

    // Deleted, or signed out, before the account's tries are used up: the key they proved with is not exempt after.
    let gone = acct.server.second_device(email).await;
    let gone_key = gone.key.clone().expect("device key");
    assert_eq!(gone.post("/roster/recovery-wrap", &right).await.0, StatusCode::OK);
    let (status, body) = sign_out_device(&acct.primary, &gone.device_id).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let signed_out = acct.server.second_device(email).await;
    let signed_out_id = signed_out.device_id.clone();
    let signed_out_key = signed_out.key.clone().expect("device key");
    assert_eq!(signed_out.post("/roster/recovery-wrap", &right).await.0, StatusCode::OK);
    // Logout deletes the device row, so this token dies. A later sign-in with the same key must not still be exempt.
    assert_eq!(signed_out.post("/logout", &json!({})).await.0, StatusCode::OK);

    let guess = json!({"master_password_hash": "guess"});
    for _ in 0..2 {
        let device = acct.server.second_device(email).await;
        assert_eq!(device.post("/roster/recovery-wrap", &guess).await.0, StatusCode::FORBIDDEN);
    }

    let (status, body) = viewer.post("/roster/recovery-wrap", &right).await;
    assert_eq!(status, StatusCode::OK, "the device key that proved keeps its tries: {body}");
    let other_key = acct.server.device_with_id(email, &viewer.device_id, Some(crate::harness::new_device_key())).await;
    let (status, body) = other_key.post("/roster/recovery-wrap", &right).await;
    assert_eq!(
        (status, error(&body)),
        (StatusCode::TOO_MANY_REQUESTS, "rate_limited"),
        "the same device id with another key is not exempt: {body}"
    );
    let (status, body) = nokey.post("/roster/recovery-wrap", &right).await;
    assert_eq!(
        (status, error(&body)),
        (StatusCode::TOO_MANY_REQUESTS, "rate_limited"),
        "no device key is never exempt: {body}"
    );
    let revived = acct.server.device_with_id(email, &uuid::Uuid::new_v4().to_string(), Some(gone_key)).await;
    let (status, body) = revived.post("/roster/recovery-wrap", &right).await;
    assert_eq!(
        (status, error(&body)),
        (StatusCode::TOO_MANY_REQUESTS, "rate_limited"),
        "deleting the device clears the key it proved with: {body}"
    );
    let signed_in_again = acct.server.device_with_id(email, &signed_out_id, Some(signed_out_key)).await;
    let (status, body) = signed_in_again.post("/roster/recovery-wrap", &right).await;
    assert_eq!(
        (status, error(&body)),
        (StatusCode::TOO_MANY_REQUESTS, "rate_limited"),
        "signing out clears the exemption: {body}"
    );

    let removal = signed(
        rewrapped(
            acct.current().successor(RosterReason::Remove, acct.current().primary.clone(), vec![], 5).expect("next"),
            NEW_WRAP,
        ),
        &acct.owner,
    );
    let (status, body) = acct.primary.post("/roster", &json!({"entry": removal, "recovery_wrap": NEW_WRAP})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = viewer.post("/roster/recovery-wrap", &right).await;
    assert_eq!(
        (status, error(&body)),
        (StatusCode::TOO_MANY_REQUESTS, "rate_limited"),
        "leaving the roster clears the exemption: {body}"
    );
    let (status, body) = acct.primary.post("/roster/recovery-wrap", &right).await;
    assert_eq!(status, StatusCode::OK, "the primary, still on the roster, keeps its own: {body}");
}

#[tokio::test]
async fn handover_requests_are_limited_per_phone() {
    let options = Options {
        env: vec![("REINS_HANDOVER_REQUESTS_PER_HOUR", "2".to_owned())],
        ..Options::default()
    };
    let mut acct = Account::new_with("ida@example.com", options).await;
    let b = acct.server.second_device("ida@example.com").await;
    acct.enroll(&b, &keys(2), "Pixel").await;
    for _ in 0..2 {
        assert_eq!(b.post("/handovers", &json!({"v": 1})).await.0, StatusCode::OK);
    }
    let (status, body) = b.post("/handovers", &json!({"v": 1})).await;
    assert_eq!((status, error(&body)), (StatusCode::TOO_MANY_REQUESTS, "rate_limited"));
    // The replacement left one request, which the primary sees once.
    let (_, pending) = acct.primary.get("/pending?wait=0").await;
    assert_eq!(pending["handovers"].as_array().map(Vec::len), Some(1));
}

#[tokio::test]
async fn small_order_keys_and_spent_enrollments_are_refused() {
    let server = Server::start().await;
    let a = server.phone("ugo@example.com").await;
    a.register_device().await;
    let user = account_of(&a);
    let own = owner(70);
    let mut weak = member(&a, &keys(1), "iPhone");
    weak.sign_pub = encode_key(&[0; 32]);
    let e1 = signed(RosterEntry::genesis(&user, weak, public(&own), encode_key(&[1; 32]), wrap_hash(WRAP), 1), &own);
    let (status, body) =
        a.post("/roster", &json!({"entry": e1, "recovery_wrap": WRAP, "master_password_hash": PASSWORD_HASH})).await;
    assert_eq!((status, error(&body)), (StatusCode::BAD_REQUEST, "bad_roster"), "a small-order signing key");

    let mut acct = Account::with_roster(server, a).await;
    let b = acct.server.second_device("ugo@example.com").await;
    let bk = keys(2);
    acct.enroll(&b, &bk, "Pixel").await;
    let removal = signed(
        rewrapped(
            acct.current().successor(RosterReason::Remove, acct.current().primary.clone(), vec![], 5).expect("next"),
            NEW_WRAP,
        ),
        &acct.owner,
    );
    let (status, body) = acct.primary.post("/roster", &json!({"entry": removal, "recovery_wrap": NEW_WRAP})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    acct.chain.push(removal);
    // Listing b again needs a new enrollment: the one it went through bound the earlier entry.
    let again = signed(
        acct.current()
            .successor(RosterReason::Enroll, acct.current().primary.clone(), vec![member(&b, &bk, "Pixel")], 6)
            .expect("next"),
        &acct.owner,
    );
    let (status, body) = acct.primary.post("/roster", &json!({"entry": again})).await;
    assert_eq!((status, error(&body)), (StatusCode::BAD_REQUEST, "bad_roster"), "{body}");
}

/// The connection ids a `GET /connections` listed.
fn listed(body: &Value) -> Option<Vec<String>> {
    body["connections"]
        .as_array()
        .map(|list| list.iter().map(|c| c["id"].as_str().unwrap_or_default().to_owned()).collect())
}

/// `GET /connections` (read-only, the design's "opens read-only to members"): the approval device, and with a roster
/// the primary and the viewers it enrolled, each with its own device key. A device of the account that signed in but
/// was never enrolled, the viewer's id with another key, a removed viewer and another account read nothing of it.
#[tokio::test]
async fn connections_are_read_by_the_approval_device_and_enrolled_viewers_only() {
    let server = Server::start().await;
    let email = "cora@example.com";
    let primary = server.phone(email).await;
    primary.register_device().await;
    let claude = server.connect_ai(&primary, email, Some("Claude")).await.connection_id;
    let mine = Some(vec![claude.clone()]);

    // Without a roster: the approval device only.
    let second = server.second_device(email).await;
    assert_eq!(listed(&primary.get("/connections").await.1), mine);
    let (status, body) = second.get("/connections").await;
    assert_eq!((status, error(&body)), (StatusCode::FORBIDDEN, "not_approval_device"));

    // With a roster: the primary reads; a signed-in phone of the account that never enrolled does not.
    let mut acct = Account::with_roster(server, primary).await;
    assert_eq!(listed(&acct.primary.get("/connections").await.1), mine);
    let (status, body) = second.get("/connections").await;
    assert_eq!((status, error(&body)), (StatusCode::FORBIDDEN, "not_a_member"), "{body}");

    // An enrolled viewer reads the same list (and changes none of it).
    let viewer = acct.server.second_device(email).await;
    acct.enroll(&viewer, &keys(2), "iPhone").await;
    assert_eq!(listed(&viewer.get("/connections").await.1), mine);
    assert_eq!(viewer.delete(&format!("/connections/{claude}")).await.0, StatusCode::FORBIDDEN);
    // The viewer's device id with another device key is not the viewer.
    let spoof = acct.server.device_with_id(email, &viewer.device_id, Some(crate::harness::new_device_key())).await;
    let (status, body) = spoof.get("/connections").await;
    assert_eq!((status, error(&body)), (StatusCode::FORBIDDEN, "not_a_member"));
    // Nor is the never-enrolled phone, still.
    assert_eq!(error(&second.get("/connections").await.1), "not_a_member");

    // Another account's approval device sees its own (none), never this account's.
    let other = acct.server.phone("otto@example.com").await;
    other.register_device().await;
    assert_eq!(listed(&other.get("/connections").await.1), Some(vec![]));

    // A removed viewer reads nothing more.
    let removal = signed(
        rewrapped(
            acct.current().successor(RosterReason::Remove, acct.current().primary.clone(), vec![], 5).expect("next"),
            NEW_WRAP,
        ),
        &acct.owner,
    );
    let (status, body) = acct.primary.post("/roster", &json!({"entry": removal, "recovery_wrap": NEW_WRAP})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = viewer.get("/connections").await;
    assert_eq!((status, error(&body)), (StatusCode::FORBIDDEN, "not_a_member"));
    assert_eq!(listed(&acct.primary.get("/connections").await.1), mine);
}

#[tokio::test]
async fn a_removed_viewer_stops_syncing_at_once() {
    let mut acct = Account::new("max@example.com").await;
    let b = acct.server.second_device("max@example.com").await;
    acct.enroll(&b, &keys(2), "Pixel").await;
    let (_, first) = b.get("/sync?wait=0").await;
    let path = format!("/sync?after=0&seen={}&wait=20", first["cursor"].as_str().unwrap());
    let removal = signed(
        rewrapped(
            acct.current().successor(RosterReason::Remove, acct.current().primary.clone(), vec![], 5).expect("next"),
            NEW_WRAP,
        ),
        &acct.owner,
    );
    let start = Instant::now();
    let remove = async {
        tokio::time::sleep(Duration::from_millis(300)).await;
        acct.primary.post("/roster", &json!({"entry": removal, "recovery_wrap": NEW_WRAP})).await.0
    };
    let ((status, body), removed) = tokio::join!(b.get(&path), remove);
    assert_eq!(removed, StatusCode::OK);
    assert_eq!((status, error(&body)), (StatusCode::FORBIDDEN, "not_a_member"));
    assert!(start.elapsed() < Duration::from_secs(10), "at once, not at the end of the wait");
}

#[tokio::test]
async fn the_log_is_read_before_a_recovery_only_with_the_proof() {
    let server = Server::start().await;
    let a = server.phone("rosa@example.com").await;
    a.register_device().await;
    let acct = Account::with_roster(server, a).await;
    let ops =
        [acct.op(&acct.primary, &acct.pk, 1, OpClass::Primary), acct.op(&acct.primary, &acct.pk, 2, OpClass::Primary)];
    assert_eq!(post_ops(&acct.primary, &ops).await.0, StatusCode::OK);
    // A phone of the account that is no member reads it only with the master password hash.
    let fresh = acct.server.second_device("rosa@example.com").await;
    let (status, body) =
        fresh.post("/roster/recovery-log", &json!({"master_password_hash": "wrong", "after": 0})).await;
    assert_eq!((status, error(&body)), (StatusCode::FORBIDDEN, "wrong_proof"));
    let (status, _) = fresh.post("/roster/recovery-log", &json!({"after": 0})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "no proof, no log");
    let (status, log) =
        fresh.post("/roster/recovery-log", &json!({"master_password_hash": PASSWORD_HASH, "after": 0})).await;
    assert_eq!(status, StatusCode::OK, "{log}");
    assert_eq!(log["ops"].as_array().map(Vec::len), Some(2));
    assert_eq!(log["head_seq"], 2);
    assert!(log.get("checkpoint").is_none_or(Value::is_null), "the checkpoint only when asked");
    let (_, after) =
        fresh.post("/roster/recovery-log", &json!({"master_password_hash": PASSWORD_HASH, "after": 1})).await;
    assert_eq!(after["ops"].as_array().map(Vec::len), Some(1));
    // The wrap's count of the primary's ops is what the server stored (a cross-check for the phone, never named).
    let (_, wrap) = fresh.post("/roster/recovery-wrap", &json!({"master_password_hash": PASSWORD_HASH})).await;
    assert_eq!(wrap["primary_counter"], 2);
}
