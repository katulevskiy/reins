//! Circles over HTTP: invitations bind one helper, packets and answers reach only their parties, viewers never answer
//! for others, a paused or ended arrangement takes nothing, and the server keeps no link secret in clear.

use reins_proto::{
    canon::{encode_payload, encode_sig},
    circle::{self, LinkChange, LinkState, Side, SignedChange},
    desktop::encode_key,
    enroll::encode_nonce,
    roster::{Member, RosterEntry, recovery_wrap_hash},
};
use reqwest::StatusCode;
use ring::signature::{Ed25519KeyPair, KeyPair};
use serde_json::{Value, json};

use crate::harness::{PASSWORD_HASH, Phone, Server};

const WRAP: &str = "d3JhcA";

fn key_pair(seed: u8) -> Ed25519KeyPair {
    Ed25519KeyPair::from_seed_unchecked(&[seed; 32]).expect("key")
}

fn public(key: &Ed25519KeyPair) -> String {
    encode_key(&key.public_key().as_ref().try_into().expect("32 bytes"))
}

fn sig(key: &Ed25519KeyPair, message: &[u8]) -> String {
    encode_sig(&key.sign(message).as_ref().try_into().expect("64 bytes"))
}

fn account_of(phone: &Phone) -> String {
    let payload = phone.token.split('.').nth(1).expect("jwt payload");
    let raw = data_encoding::BASE64URL_NOPAD.decode(payload.trim_end_matches('=').as_bytes()).expect("base64url");
    serde_json::from_slice::<Value>(&raw).expect("json")["sub"].as_str().expect("sub").to_owned()
}

/// An account with a primary phone and a roster at epoch 1.
struct Person {
    phone: Phone,
    user: String,
    owner: Ed25519KeyPair,
}

impl Person {
    async fn new(server: &Server, email: &str, seed: u8) -> Self {
        let phone = server.phone(email).await;
        phone.register_device().await;
        let user = account_of(&phone);
        let owner = key_pair(seed);
        let device = key_pair(seed.wrapping_add(1));
        let mut entry = RosterEntry::genesis(
            &user,
            Member {
                device_id: phone.device_id.clone(),
                name: "Phone".to_owned(),
                sign_pub: public(&device),
                box_pub: encode_key(&[seed.wrapping_add(2); 32]),
            },
            public(&owner),
            encode_key(&[seed.wrapping_add(3); 32]),
            recovery_wrap_hash(WRAP).expect("wrap"),
            1,
        );
        entry.sig = sig(&owner, &entry.signing_bytes().expect("bytes"));
        let (status, body) = phone
            .post("/roster", &json!({"entry": entry, "recovery_wrap": WRAP, "master_password_hash": PASSWORD_HASH}))
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        Self {
            phone,
            user,
            owner,
        }
    }

    fn owner_pub(&self) -> String {
        public(&self.owner)
    }

    fn change(&self, link: &str, seq: u64, state: LinkState, by: Side) -> Value {
        let c = LinkChange {
            v: 1,
            link: link.to_owned(),
            arrangement: circle::arrangement_hash("{}"),
            seq,
            state,
            by,
            at: 1,
        };
        let body = serde_json::to_string(&c).expect("json");
        json!(SignedChange {
            sig: sig(&self.owner, &circle::change_message(&body)),
            body,
        })
    }
}

/// Dana invites Alex; both steps through; returns the link id.
async fn link(dana: &Person, alex: &Person) -> String {
    link_as(dana, alex, "cosign").await
}

async fn link_as(dana: &Person, alex: &Person, kind: &str) -> String {
    let commitment = circle::invite_commitment(&dana.user, &dana.owner_pub(), 1, &[1; 32]).expect("commitment");
    let (status, created) = dana
        .phone
        .post(
            "/circles/invites",
            &json!({"v": 1, "kind": kind, "commitment": commitment, "owner_pub": dana.owner_pub(), "owner_epoch": 1}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{created}");
    let id = created["id"].as_str().expect("id").to_owned();
    let token = created["token"].as_str().expect("token").to_owned();
    // The owner can't answer its own link.
    let (status, _) = dana.phone.post("/circles/join", &json!({"token": token})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, opened) = alex.phone.post("/circles/join", &json!({"token": token})).await;
    assert_eq!(status, StatusCode::OK, "{opened}");
    assert_eq!(opened["id"], id.as_str());
    assert_eq!(opened["commitment"], commitment.as_str());
    let challenge = json!({"v": 1, "token": token, "helper_pub": alex.owner_pub(), "helper_epoch": 1, "nonce": encode_nonce(&[2; 32])});
    let (status, body) = alex.phone.post(&format!("/circles/invites/{id}/challenge"), &challenge).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    // The same challenge again (its answer was lost): taken; another one from the same helper: refused.
    assert_eq!(
        alex.phone.post(&format!("/circles/invites/{id}/challenge"), &challenge).await.0,
        StatusCode::NO_CONTENT
    );
    let mut other = challenge.clone();
    other["nonce"] = json!(encode_nonce(&[3; 32]));
    assert_eq!(alex.phone.post(&format!("/circles/invites/{id}/challenge"), &other).await.0, StatusCode::CONFLICT);
    let (_, owner_view) = dana.phone.get(&format!("/circles/invites/{id}")).await;
    assert_eq!(owner_view["state"], "challenged");
    assert_eq!(owner_view["challenge"]["token"], "", "the link secret is never shown back");
    assert_eq!(owner_view["helper_account"], alex.user.as_str());
    let reveal = json!({"v": 1, "nonce": encode_nonce(&[1; 32]), "sealed_terms": encode_payload(b"sealed terms")});
    // Only the owner reveals.
    assert_eq!(alex.phone.post(&format!("/circles/invites/{id}/reveal"), &reveal).await.0, StatusCode::NOT_FOUND);
    assert_eq!(dana.phone.post(&format!("/circles/invites/{id}/reveal"), &reveal).await.0, StatusCode::NO_CONTENT);
    let (_, helper_view) = alex.phone.get(&format!("/circles/invites/{id}/helper")).await;
    assert_eq!(helper_view["reveal"]["nonce"], encode_nonce(&[1; 32]));
    let accept = json!({"v": 1, "sealed": encode_payload(b"sealed accept")});
    assert_eq!(alex.phone.post(&format!("/circles/invites/{id}/accept"), &accept).await.0, StatusCode::NO_CONTENT);
    assert_eq!(alex.phone.post(&format!("/circles/invites/{id}/accept"), &accept).await.0, StatusCode::CONFLICT);
    let link = "0123456789abcdef0123456789abcdef".to_owned();
    let activate = json!({"v": 1, "link": link, "owner_copy": encode_payload(b"owner copy"), "helper_copy": encode_payload(b"helper copy")});
    assert_eq!(dana.phone.post(&format!("/circles/invites/{id}/activate"), &activate).await.0, StatusCode::NO_CONTENT);
    assert_eq!(dana.phone.post(&format!("/circles/invites/{id}/activate"), &activate).await.0, StatusCode::CONFLICT);
    link
}

fn packet(id: &str, link: &str, deadline_in: i64) -> Value {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).expect("time").as_secs();
    let deadline = i64::try_from(now).expect("i64") + deadline_in;
    json!({"v": 1, "id": id, "link": link, "kind": "review", "deadline": deadline, "sealed": encode_payload(b"sealed packet")})
}

#[tokio::test]
async fn invite_flow_binds_one_helper() {
    let server = Server::start().await;
    let dana = Person::new(&server, "dana@example.com", 10).await;
    let alex = Person::new(&server, "alex@example.com", 20).await;
    let eve = Person::new(&server, "eve@example.com", 30).await;
    // A second challenge (another account holding the same link) finds the invitation taken.
    let commitment = circle::invite_commitment(&dana.user, &dana.owner_pub(), 1, &[1; 32]).expect("commitment");
    let (_, created) = dana
        .phone
        .post(
            "/circles/invites",
            &json!({"v": 1, "kind": "delegate", "commitment": commitment, "owner_pub": dana.owner_pub(), "owner_epoch": 1}),
        )
        .await;
    let (id, token) = (created["id"].as_str().expect("id"), created["token"].as_str().expect("token"));
    let challenge = |p: &Person| json!({"v": 1, "token": token, "helper_pub": p.owner_pub(), "helper_epoch": 1, "nonce": encode_nonce(&[2; 32])});
    assert_eq!(
        alex.phone.post(&format!("/circles/invites/{id}/challenge"), &challenge(&alex)).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        eve.phone.post(&format!("/circles/invites/{id}/challenge"), &challenge(&eve)).await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(eve.phone.get(&format!("/circles/invites/{id}/helper")).await.0, StatusCode::NOT_FOUND);
    // A wrong secret is not found.
    let mut wrong = challenge(&eve);
    wrong["token"] = json!("AAAA");
    assert_eq!(eve.phone.post(&format!("/circles/invites/{id}/challenge"), &wrong).await.0, StatusCode::NOT_FOUND);
    // An owner key that isn't the account's own is refused.
    let mut lying = challenge(&eve);
    lying["helper_pub"] = json!(alex.owner_pub());
    assert_eq!(eve.phone.post(&format!("/circles/invites/{id}/challenge"), &lying).await.0, StatusCode::CONFLICT);

    let l = link(&dana, &alex).await;
    let (_, dana_links) = dana.phone.get("/circles/links").await;
    let (_, alex_links) = alex.phone.get("/circles/links").await;
    let (_, eve_links) = eve.phone.get("/circles/links").await;
    assert_eq!(dana_links[0]["id"], l.as_str());
    assert_eq!(dana_links[0]["side"], "owner");
    assert_eq!(dana_links[0]["copy"], encode_payload(b"owner copy"));
    assert_eq!(alex_links[0]["side"], "helper");
    assert_eq!(alex_links[0]["copy"], encode_payload(b"helper copy"));
    assert_eq!(alex_links[0]["peer"], "dana@example.com");
    assert_eq!(eve_links, json!([]));
    // The server keeps the link secret's hash only.
    let mut db = std::fs::read(server.data_dir().join("db.sqlite3")).expect("the database file");
    db.extend(std::fs::read(server.data_dir().join("db.sqlite3-wal")).unwrap_or_default());
    assert!(db.windows(10).any(|w| w == b"challenged"), "the check reads the stored rows");
    assert!(!db.windows(token.len()).any(|w| w == token.as_bytes()), "the link secret is stored in clear");
}

#[tokio::test]
async fn packets_are_only_for_their_parties() {
    let server = Server::start().await;
    let dana = Person::new(&server, "dana@example.com", 10).await;
    let alex = Person::new(&server, "alex@example.com", 20).await;
    let eve = Person::new(&server, "eve@example.com", 30).await;
    let l = link(&dana, &alex).await;
    let p = "fedcba9876543210fedcba9876543210";
    // Only the link's owner sends packets.
    assert_eq!(alex.phone.post("/circles/packets", &packet(p, &l, 600)).await.0, StatusCode::NOT_FOUND);
    assert_eq!(dana.phone.post("/circles/packets", &packet(p, &l, 600)).await.0, StatusCode::NO_CONTENT);
    let (_, inbox) = alex.phone.get("/circles/inbox").await;
    assert_eq!(inbox[0]["id"], p);
    assert_eq!(inbox[0]["sealed"], encode_payload(b"sealed packet"));
    assert_eq!(eve.phone.get("/circles/inbox").await.1, json!([]));
    assert_eq!(dana.phone.get("/circles/inbox").await.1, json!([]));
    // The others' rosters: each party sees the other's, a stranger nothing.
    let (status, roster) = alex.phone.get(&format!("/circles/peers/{l}/roster")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(roster["entries"][0]["owner_pub"], dana.owner_pub().as_str());
    let (_, roster) = dana.phone.get(&format!("/circles/peers/{l}/roster")).await;
    assert_eq!(roster["entries"][0]["owner_pub"], alex.owner_pub().as_str());
    assert_eq!(eve.phone.get(&format!("/circles/peers/{l}/roster")).await.0, StatusCode::NOT_FOUND);
    // Answers: the helper only, once.
    let answer = json!({"v": 1, "sealed": encode_payload(b"sealed answer")});
    assert_eq!(eve.phone.post(&format!("/circles/packets/{p}/answer"), &answer).await.0, StatusCode::NOT_FOUND);
    assert_eq!(dana.phone.post(&format!("/circles/packets/{p}/answer"), &answer).await.0, StatusCode::NOT_FOUND);
    assert_eq!(alex.phone.post(&format!("/circles/packets/{p}/answer"), &answer).await.0, StatusCode::NO_CONTENT);
    assert_eq!(alex.phone.post(&format!("/circles/packets/{p}/answer"), &answer).await.0, StatusCode::CONFLICT);
    let (_, answers) = dana.phone.get("/circles/answers").await;
    assert_eq!(answers[0]["packet"], p);
    assert_eq!(answers[0]["state"], "answered");
    assert_eq!(answers[0]["sealed"], encode_payload(b"sealed answer"));
    assert_eq!(eve.phone.get("/circles/answers").await.1, json!([]));
    assert_eq!(alex.phone.get("/circles/answers").await.1, json!([]));
    // Seen by someone else changes nothing; by the owner, it's gone.
    assert_eq!(eve.phone.post("/circles/answers/seen", &json!({"ids": [p]})).await.0, StatusCode::NO_CONTENT);
    assert_eq!(dana.phone.get("/circles/answers").await.1.as_array().map(Vec::len), Some(1));
    assert_eq!(dana.phone.post("/circles/answers/seen", &json!({"ids": [p]})).await.0, StatusCode::NO_CONTENT);
    assert_eq!(dana.phone.get("/circles/answers").await.1, json!([]));
}

#[tokio::test]
async fn viewers_cannot_answer_for_others() {
    let server = Server::start().await;
    let dana = Person::new(&server, "dana@example.com", 10).await;
    let alex = Person::new(&server, "alex@example.com", 20).await;
    let l = link(&dana, &alex).await;
    let p = "fedcba9876543210fedcba9876543210";
    assert_eq!(dana.phone.post("/circles/packets", &packet(p, &l, 600)).await.0, StatusCode::NO_CONTENT);
    // Another signed-in device of Alex's (not the roster's primary) reads and answers nothing.
    let other = server.second_device("alex@example.com").await;
    assert_eq!(other.get("/circles/inbox").await.0, StatusCode::FORBIDDEN);
    let answer = json!({"v": 1, "sealed": encode_payload(b"sealed answer")});
    assert_eq!(other.post(&format!("/circles/packets/{p}/answer"), &answer).await.0, StatusCode::FORBIDDEN);
    assert_eq!(other.get("/circles/links").await.0, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn a_paused_link_takes_no_packets_and_an_ended_one_nothing() {
    let server = Server::start().await;
    let dana = Person::new(&server, "dana@example.com", 10).await;
    let alex = Person::new(&server, "alex@example.com", 20).await;
    let l = link(&dana, &alex).await;
    let p1 = "fedcba9876543210fedcba9876543211";
    assert_eq!(dana.phone.post("/circles/packets", &packet(p1, &l, 600)).await.0, StatusCode::NO_CONTENT);
    // The helper can't pause; a change signed by another key is refused.
    let path = format!("/circles/links/{l}/change");
    assert_eq!(
        alex.phone.post(&path, &alex.change(&l, 1, LinkState::Paused, Side::Helper)).await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        dana.phone.post(&path, &alex.change(&l, 1, LinkState::Paused, Side::Owner)).await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        dana.phone.post(&path, &dana.change(&l, 1, LinkState::Paused, Side::Owner)).await.0,
        StatusCode::NO_CONTENT
    );
    // A replayed (not newer) change is refused.
    assert_eq!(
        dana.phone.post(&path, &dana.change(&l, 1, LinkState::Paused, Side::Owner)).await.0,
        StatusCode::CONFLICT
    );
    let (_, links) = alex.phone.get("/circles/links").await;
    assert_eq!(links[0]["state"], "paused");
    assert!(links[0]["change"]["sig"].is_string());
    // Paused: the waiting packet was withdrawn, a new one and an answer are refused.
    let (_, inbox) = alex.phone.get("/circles/inbox").await;
    assert_eq!(inbox[0]["state"], "withdrawn");
    let p2 = "fedcba9876543210fedcba9876543212";
    assert_eq!(dana.phone.post("/circles/packets", &packet(p2, &l, 600)).await.0, StatusCode::CONFLICT);
    let answer = json!({"v": 1, "sealed": encode_payload(b"x")});
    assert_eq!(alex.phone.post(&format!("/circles/packets/{p1}/answer"), &answer).await.0, StatusCode::CONFLICT);
    // Resumed, then the helper leaves: ended for good.
    assert_eq!(
        dana.phone.post(&path, &dana.change(&l, 2, LinkState::Active, Side::Owner)).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        alex.phone.post(&path, &alex.change(&l, 3, LinkState::Ended, Side::Helper)).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        dana.phone.post(&path, &dana.change(&l, 4, LinkState::Active, Side::Owner)).await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(dana.phone.post("/circles/packets", &packet(p2, &l, 600)).await.0, StatusCode::CONFLICT);
}

#[tokio::test]
async fn a_guardianship_is_ended_never_paused() {
    let server = Server::start().await;
    let ward = Person::new(&server, "sam@example.com", 10).await;
    let guardian = Person::new(&server, "mom@example.com", 20).await;
    let l = link_as(&ward, &guardian, "guardian").await;
    let path = format!("/circles/links/{l}/change");
    assert_eq!(
        ward.phone.post(&path, &ward.change(&l, 1, LinkState::Paused, Side::Owner)).await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        ward.phone.post(&path, &ward.change(&l, 1, LinkState::Ended, Side::Owner)).await.0,
        StatusCode::NO_CONTENT
    );
}
