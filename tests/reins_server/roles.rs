//! Phone roles and live sync: the roster (create, enroll, handover, recovery), the op log and its checks, the sync
//! long-poll, signed answers, and legacy accounts left as they were.

use std::time::{Duration, Instant};

use reins_proto::{
    answer::{AnswerKind, answer_message},
    canon::{encode_payload, encode_sig, hex32},
    desktop::encode_key,
    enroll::{EnrollChallenge, Transcript, commitment, encode_nonce},
    oplog::{Checkpoint, Heartbeat, Op, OpClass},
    roster::{Member, RosterEntry, RosterReason},
};
use reqwest::StatusCode;
use ring::signature::{Ed25519KeyPair, KeyPair};
use serde_json::{Value, json};

use crate::harness::{PASSWORD_HASH, Phone, Server, client};

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
        let server = Server::start().await;
        let primary = server.phone(email).await;
        primary.register_device().await;
        Self::with_roster(server, primary).await
    }

    async fn with_roster(server: Server, primary: Phone) -> Self {
        let user = account_of(&primary);
        let pk = keys(1);
        let owner = owner(50);
        let e1 = signed(
            RosterEntry::genesis(&user, member(&primary, &pk, "iPhone"), public(&owner), encode_key(&[60; 32]), 1),
            &owner,
        );
        let (status, body) = primary.post("/roster", &json!({"entry": e1, "recovery_wrap": "d3JhcA"})).await;
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
            joiner_nonce: nj,
            primary_nonce: np,
        };
        assert_eq!(transcript.code().expect("code").len(), 7);

        let (status, body) = self.primary.post("/roster", &json!({"entry": entry})).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        self.chain.push(entry);
        let answer = json!({"v": 1, "approve": true, "sealed": "U0VBTEVE"});
        assert_eq!(self.primary.post(&format!("/joins/{id}/response"), &answer).await.0, StatusCode::NO_CONTENT);
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

#[tokio::test]
async fn roster_create_enroll_handover_and_recovery() {
    let server = Server::start().await;
    let a = server.phone("rosa@example.com").await;
    a.register_device().await;
    let (status, body) = a.get("/roster").await;
    assert_eq!((status, body), (StatusCode::OK, json!({"entries": [], "recovery_wrap": null})), "no roster yet");
    // Only the approval device creates the roster.
    let stranger = server.second_device("rosa@example.com").await;
    let sk = keys(9);
    let user = account_of(&a);
    let own = owner(70);
    let e1 = signed(
        RosterEntry::genesis(&user, member(&stranger, &sk, "Other"), public(&own), encode_key(&[1; 32]), 1),
        &own,
    );
    let (status, body) = stranger.post("/roster", &json!({"entry": e1, "recovery_wrap": "d3JhcA"})).await;
    assert_eq!((status, error(&body)), (StatusCode::FORBIDDEN, "not_approval_device"));

    let mut acct = Account::with_roster(server, a).await;
    let (_, roster) = acct.primary.get("/roster").await;
    assert_eq!(
        (roster["entries"].as_array().map(Vec::len), roster["recovery_wrap"].as_str()),
        (Some(1), Some("d3JhcA"))
    );
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
        acct.current()
            .successor(
                RosterReason::Handover,
                member(&b, &bk, "Pixel"),
                vec![member(&acct.primary, &acct.pk, "iPhone")],
                3,
            )
            .expect("next"),
        &acct.owner,
    );
    let (status, body) = acct.primary.post("/roster", &json!({"entry": e3})).await;
    assert_eq!((status, error(&body)), (StatusCode::BAD_REQUEST, "bad_roster"), "a handover carries the bundle");
    let (status, body) = acct.primary.post("/roster", &json!({"entry": e3, "bundle": "QlVORExF"})).await;
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
    let e4 = signed(e4, &acct.owner);
    let (status, body) = c.post("/roster", &json!({"entry": e4})).await;
    assert_eq!((status, error(&body)), (StatusCode::FORBIDDEN, "proof_required"));
    let (status, body) = c.post("/roster", &json!({"entry": e4, "master_password_hash": "wrong"})).await;
    assert_eq!((status, error(&body)), (StatusCode::FORBIDDEN, "wrong_proof"));
    let forged = signed(e4.clone(), &new_owner);
    let (status, body) = c.post("/roster", &json!({"entry": forged, "master_password_hash": PASSWORD_HASH})).await;
    assert_eq!((status, error(&body)), (StatusCode::FORBIDDEN, "bad_signature"), "the old owner key signs a recovery");
    let (status, body) = c.post("/roster", &json!({"entry": e4, "master_password_hash": PASSWORD_HASH})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
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
    let mut acct = Account::new("sam@example.com").await;
    let tokens = acct.server.connect_ai(&acct.primary, "sam@example.com", Some("Claude")).await;
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
    let acct = Account::with_roster(server, phone).await;

    let ai = mcp_tool(acct.server.url("/mcp"), tokens.access.clone());
    let phone_side = async {
        let (_, pending) = acct.primary.get("/pending?wait=10").await;
        let id = pending["requests"][0]["id"].as_str().expect("request").to_owned();
        let denied = json!({"v": 1, "outcome": "denied", "reason": null});
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
    assert_eq!(phone.get("/roster").await.1, json!({"entries": [], "recovery_wrap": null}));
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
    let bytes = serde_json::to_vec(answer).unwrap();
    let epoch = acct.current().epoch;
    let message = answer_message(&acct.user, AnswerKind::Request, id, epoch, &bytes);
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
