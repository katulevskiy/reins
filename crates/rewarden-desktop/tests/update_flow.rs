//! `rewarden update` against a release server: a newer signed release is downloaded and checked; an older release
//! (a replayed manifest), another key's signature, or a binary that differs from the signed hash are refused.

use std::collections::BTreeMap;

use data_encoding::{BASE64URL_NOPAD, HEXLOWER};
use rewarden_desktop::update::{Asset, Check, Manifest, SIGNING_CONTEXT, Signed, Updater};
use ring::signature::{Ed25519KeyPair, KeyPair as _};
use sha2::{Digest, Sha256};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const PLATFORM: &str = "linux-x86_64";
const BINARY: &[u8] = b"\x7fELF pretend binary";

fn keypair() -> Ed25519KeyPair {
    let pkcs8 = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
    Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap()
}

fn signed(key: &Ed25519KeyPair, build_time: i64, sha256: &str) -> Vec<u8> {
    let manifest = Manifest {
        version: "0.2.0".into(),
        build: format!("0.2.0-{build_time}-abcdef12"),
        build_time,
        assets: BTreeMap::from([(
            PLATFORM.to_owned(),
            Asset {
                file: "rewarden-0.2.0-linux-x86_64".into(),
                sha256: sha256.to_owned(),
                size: BINARY.len() as u64,
            },
        )]),
    };
    let text = serde_json::to_string(&manifest).unwrap();
    let mut message = SIGNING_CONTEXT.to_vec();
    message.extend_from_slice(text.as_bytes());
    serde_json::to_vec(&Signed {
        signature: BASE64URL_NOPAD.encode(key.sign(&message).as_ref()),
        manifest: text,
    })
    .unwrap()
}

async fn server(latest: Vec<u8>, binary: &[u8]) -> MockServer {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/releases/latest.json"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(latest))
        .mount(&s)
        .await;
    Mock::given(method("GET"))
        .and(path("/releases/files/rewarden-0.2.0-linux-x86_64"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(binary.to_vec()))
        .mount(&s)
        .await;
    s
}

fn updater(s: &MockServer, key: &Ed25519KeyPair, current_time: i64) -> Updater {
    Updater::new(&format!("{}/releases", s.uri()), key.public_key().as_ref().to_vec(), PLATFORM, current_time).unwrap()
}

fn sha(bytes: &[u8]) -> String {
    HEXLOWER.encode(&Sha256::digest(bytes))
}

#[tokio::test]
async fn a_newer_signed_release_is_downloaded_and_checked() {
    let key = keypair();
    let s = server(signed(&key, 2_000, &sha(BINARY)), BINARY).await;
    let up = updater(&s, &key, 1_000);
    let Check::Available(latest, asset) = up.check().await.unwrap() else {
        panic!("a newer release must be offered")
    };
    assert_eq!(latest.build_time, 2_000);
    assert_eq!(up.download(&asset).await.unwrap(), BINARY);
}

#[tokio::test]
async fn the_same_or_an_older_release_is_not_an_update() {
    let key = keypair();
    let s = server(signed(&key, 2_000, &sha(BINARY)), BINARY).await;
    assert!(matches!(updater(&s, &key, 2_000).check().await.unwrap(), Check::UpToDate(_)));
    assert!(matches!(updater(&s, &key, 3_000).check().await.unwrap(), Check::UpToDate(_)), "no downgrade");
}

#[tokio::test]
async fn a_release_signed_with_another_key_is_refused() {
    let key = keypair();
    let s = server(signed(&keypair(), 2_000, &sha(BINARY)), BINARY).await;
    let err = updater(&s, &key, 1_000).check().await.unwrap_err();
    assert!(err.contains("not signed"), "{err}");
}

#[tokio::test]
async fn a_binary_that_differs_from_the_signed_hash_is_refused() {
    let key = keypair();
    let s = server(signed(&key, 2_000, &sha(b"something else entirely")), BINARY).await;
    let up = updater(&s, &key, 1_000);
    let Check::Available(_, asset) = up.check().await.unwrap() else {
        panic!("offered")
    };
    let err = up.download(&asset).await.unwrap_err();
    assert!(err.contains("does not match"), "{err}");
}

#[tokio::test]
async fn a_release_without_a_build_for_this_platform_says_so() {
    let key = keypair();
    let s = server(signed(&key, 2_000, &sha(BINARY)), BINARY).await;
    let up = Updater::new(&format!("{}/releases", s.uri()), key.public_key().as_ref().to_vec(), "linux-riscv64", 1_000)
        .unwrap();
    assert!(matches!(up.check().await.unwrap(), Check::NoBuild(_)));
}
