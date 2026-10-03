//! The vault for the paired desktop app: secrets released for one command (sealed, with a lease), the public halves of
//! the vault's SSH keys, and SSH sign-ins signed on the phone. A mock Vaultwarden serves an encrypted vault; signatures
//! are verified here with an independent implementation (ring), from the public keys `ssh-keygen` wrote.

mod common;

use common::desktop::{DESK, Desk, REFUSED, call, choice, desk_with, mount_rewarden, standing};
use data_encoding::BASE64;
use rewarden_core::connector::vault::totp_code;
use rewarden_core::crypto::{Kdf, VaultKey, master_key};
use rewarden_core::store::unix_now;
use rewarden_core::{ApprovalKind, CoreConfig, CoreError};
use rewarden_proto::desktop::{SecretGrant, SshSignature};
use ring::signature::{self, UnparsedPublicKey};
use serde_json::{Value, json};
use wiremock::matchers::{header_exists, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const EMAIL: &str = "me@example.com";
const PASSWORD: &str = "correct horse battery";
const TOTP_SECRET: &str = "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ";
/// Values that must never be shown or kept anywhere but inside the sealed answer.
const SECRETS: &[&str] = &["hunter2!", "s3cret", "sk-live-123", TOTP_SECRET, "OPENSSH PRIVATE KEY"];

struct Key {
    name: &'static str,
    private: &'static str,
    public: &'static str,
    fingerprint: &'static str,
}

const ED: Key = Key {
    name: "Ed key",
    private: include_str!("fixtures/ssh/k_ed"),
    public: include_str!("fixtures/ssh/k_ed.pub"),
    fingerprint: "SHA256:59UqiUdGtW4OqO6NNXeMEo3goCDvszgSgq1R0tab5n0",
};
const RSA: Key = Key {
    name: "RSA key",
    private: include_str!("fixtures/ssh/k_rsa"),
    public: include_str!("fixtures/ssh/k_rsa.pub"),
    fingerprint: "SHA256:JOJ+KLDSJt11ptPY9MDqBEGDntCrME/G3Ff+YnA2Uao",
};
const P256: Key = Key {
    name: "P256 key",
    private: include_str!("fixtures/ssh/k_p256"),
    public: include_str!("fixtures/ssh/k_p256.pub"),
    fingerprint: "SHA256:SzNDDHVCLiGc/bZ2pbeZ3kKARQzCRfo4RRoLs8E0SIs",
};
const P384: Key = Key {
    name: "P384 key",
    private: include_str!("fixtures/ssh/k_p384"),
    public: include_str!("fixtures/ssh/k_p384.pub"),
    fingerprint: "SHA256:PLrMKCsfcC4xatoVOeR4DrQrqHs3X3dwopxSMXvUtTg",
};

impl Key {
    /// `alg base64`, as the phone lists it.
    fn line(&self) -> String {
        self.public.split_whitespace().take(2).collect::<Vec<_>>().join(" ")
    }

    /// The public key blob.
    fn blob(&self) -> Vec<u8> {
        BASE64.decode(self.public.split_whitespace().nth(1).unwrap().as_bytes()).unwrap()
    }
}

fn t(key: &VaultKey, s: &str) -> Value {
    json!(key.encrypt(s.as_bytes()).unwrap())
}

fn cipher(key: &VaultKey, id: &str, kind: i64, name: &str) -> Value {
    json!({"id": id, "type": kind, "name": t(key, name), "notes": null, "key": null, "folderId": null,
        "favorite": false, "deletedDate": null, "archivedDate": null, "organizationId": null,
        "reprompt": 0, "fields": [], "passwordHistory": [], "attachments": null,
        "login": null, "secureNote": null, "card": null, "identity": null, "sshKey": null})
}

fn login(key: &VaultKey, id: &str, name: &str, username: &str, password: &str) -> Value {
    let mut c = cipher(key, id, 1, name);
    c["login"] = json!({"username": t(key, username), "password": t(key, password), "totp": null, "uris": null});
    c
}

fn ssh(key: &VaultKey, id: &str, k: &Key) -> Value {
    let mut c = cipher(key, id, 5, k.name);
    c["sshKey"] = json!({"privateKey": t(key, k.private), "publicKey": t(key, &k.line()), "keyFingerprint": t(key, k.fingerprint)});
    c
}

fn ciphers(user: &VaultKey) -> Vec<Value> {
    let mut git = login(user, "git", "GitHub", "octo", "hunter2!");
    git["folderId"] = json!("f1");
    git["notes"] = t(user, "deploy notes");
    git["login"]["totp"] = t(user, &format!("otpauth://totp/GitHub:octo?secret={TOTP_SECRET}"));
    git["login"]["uris"] = json!([{"uri": t(user, "https://github.com/login"), "match": null}]);
    git["fields"] = json!([{"name": t(user, "API key"), "value": t(user, "sk-live-123"), "type": 1, "linkedId": null}]);
    let bank = login(user, "bank", "Bank", "anna", "s3cret");
    let twin1 = login(user, "twin1", "Twin", "a", "pw-a");
    let twin2 = login(user, "twin2", "Twin", "b", "pw-b");
    let mut old = login(user, "old", "Old", "x", "old-pw");
    old["deletedDate"] = json!("2026-03-01T00:00:00.000000Z");
    let mut broken = cipher(user, "broken", 5, "Broken key");
    broken["sshKey"] = json!({"privateKey": t(user, "not a key"), "publicKey": t(user, "ssh-ed25519 AAAA"),
        "keyFingerprint": t(user, "SHA256:x")});
    vec![
        git,
        bank,
        twin1,
        twin2,
        old,
        ssh(user, "ssh-ed", &ED),
        ssh(user, "ssh-rsa", &RSA),
        ssh(user, "ssh-p256", &P256),
        ssh(user, "ssh-p384", &P384),
        broken,
    ]
}

async fn desk() -> Desk {
    let server = MockServer::start().await;
    mount_rewarden(&server).await;
    let master = master_key(
        PASSWORD,
        EMAIL,
        Kdf::Pbkdf2 {
            iterations: 5000,
        },
    )
    .unwrap();
    let user = VaultKey::from_bytes(&[7u8; 64]).unwrap();
    let wrapped = master.stretch().encrypt(&user.to_bytes()).unwrap();
    let folders = json!([{"id": "f1", "name": t(&user, "Work")}]);
    Mock::given(method("GET"))
        .and(path("/api/sync"))
        .and(header_exists("Bitwarden-Client-Version"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"profile": {"key": wrapped}, "ciphers": ciphers(&user), "folders": folders})),
        )
        .with_priority(3)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/sync"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"profile": {"key": wrapped}, "ciphers": [],
            "folders": folders})))
        .with_priority(4)
        .mount(&server)
        .await;
    let desk = desk_with(server, EMAIL, PASSWORD, CoreConfig::default()).await;
    desk.core.add_token_account("vault".to_owned(), PASSWORD.to_owned()).await.unwrap();
    desk
}

fn release(desk: &Desk, id: &str, secrets: &[&str], lease: i64) -> Value {
    call(
        id,
        DESK,
        "vault",
        "secret_release",
        &json!({"secrets": secrets, "command": "npm run deploy", "purpose": "ship the release", "lease_secs": lease,
                "client_key": desk.public(), "nonce": format!("nonce-{id}")}),
    )
}

fn no_secrets(what: &str, shown: &str) {
    for secret in SECRETS {
        assert!(!shown.contains(secret), "{what} leaks {secret}");
    }
}

// ---- secrets --------------------------------------------------------------------------------------------------------

#[tokio::test]
async fn only_the_paired_app_gets_secrets_or_signatures() {
    let desk = desk().await;
    let sign = sign_call(&desk, "r2", &ED, 0, Some("github.com"), &userauth(&ED.blob(), "ssh-ed25519", None));
    desk.send(&[release(&desk, "r1", &["GitHub/password"], 600), sign]).await;
    assert_eq!(desk.error("r1").await, REFUSED);
    assert_eq!(desk.error("r2").await, REFUSED);
    let keys = call("r3", DESK, "vault", "ssh_keys", &json!({"client_key": desk.public(), "nonce": "n"}));
    desk.send(&[keys]).await;
    assert_eq!(desk.error("r3").await, REFUSED);
}

#[tokio::test]
async fn secrets_are_shown_by_name_and_released_sealed_with_their_lease() {
    let desk = desk().await;
    desk.pair().await;
    let wanted = ["GitHub/password", "GitHub/totp", "Bank/password", "git/API key"];
    desk.send(&[release(&desk, "r1", &wanted, 1_800)]).await;
    assert_eq!(desk.waiting().await, ["r1"]);
    let view = desk.core.approval_view("r1".to_owned()).await.unwrap();
    assert_eq!((view.kind, view.class.as_str()), (ApprovalKind::Write, "secrets"));
    let shown = view.secrets.clone().expect("the release");
    assert_eq!((shown.command.as_str(), shown.purpose.as_deref()), ("npm run deploy", Some("ship the release")));
    assert_eq!(
        shown.items,
        ["GitHub \u{b7} password", "GitHub \u{b7} one-time code", "Bank \u{b7} password", "GitHub \u{b7} API key"]
    );
    assert_eq!(shown.lease_secs, 1_800);
    assert_eq!(view.preview[0], "Use 4 secrets for npm run deploy");
    assert_eq!(view.preview.last().unwrap(), "The computer keeps them for 30 minutes");
    assert_eq!((view.resources[0].id.as_str(), view.resources[0].label.as_str()), ("secrets", "2 vault items"));
    no_secrets("the approval", &format!("{view:?}"));

    let before = unix_now();
    desk.core.approve("r1".to_owned(), choice(&[], None)).await.unwrap();
    let after = unix_now();
    let data = desk.data("r1").await;
    let grant: SecretGrant = desk.open(&data["sealed"]);
    assert_eq!((grant.v, grant.nonce.as_str()), (1, "nonce-r1"));
    let names: Vec<&str> = grant.secrets.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, wanted, "in the order asked for");
    assert_eq!(grant.secrets[0].1, "hunter2!");
    assert_eq!(grant.secrets[2].1, "s3cret");
    assert_eq!(grant.secrets[3].1, "sk-live-123");
    let codes = [totp_code(TOTP_SECRET, before.try_into().unwrap()), totp_code(TOTP_SECRET, after.try_into().unwrap())]
        .map(|c| c.unwrap().0);
    assert!(codes.contains(&grant.secrets[1].1), "the current one-time code");
    assert!((before + 1_800..=after + 1_800).contains(&grant.expires_at), "the lease ends then");
    assert_eq!(data["expires_at"], grant.expires_at);
    assert!(!format!("{grant:?}").contains("hunter2!"), "a grant's Debug shows names only");
    no_secrets("everything shown and sent", &desk.everything_shown(&["r1"]).await);
}

#[tokio::test]
async fn one_item_is_its_own_resource_and_a_permission_for_it_covers_the_next_release() {
    let desk = desk().await;
    desk.pair().await;
    desk.send(&[release(&desk, "r1", &["GitHub/username"], 600)]).await;
    let view = desk.core.approval_view("r1".to_owned()).await.unwrap();
    let resources: Vec<_> = view.resources.iter().map(|r| (r.id.as_str(), r.wider)).collect();
    assert_eq!(resources, [("f1/login/git", false), ("f1/login", true), ("f1", true)]);
    desk.core.approve("r1".to_owned(), choice(&[], standing(&["f1/login/git"], &["secrets"]))).await.unwrap();

    desk.send(&[
        release(&desk, "r2", &["GitHub/password", "git/notes"], 120),
        release(&desk, "r3", &["Bank/password"], 600),
    ])
    .await;
    assert_eq!(desk.waiting().await, ["r3"], "the same item went through, another is asked for");
    let grant: SecretGrant = desk.open(&desk.data("r2").await["sealed"]);
    assert_eq!(
        grant.secrets,
        [("GitHub/password".to_owned(), "hunter2!".to_owned()), ("git/notes".to_owned(), "deploy notes".to_owned())]
    );
    let now = unix_now();
    assert!((now + 110..=now + 120).contains(&grant.expires_at), "a short lease");
}

#[tokio::test]
async fn mistakes_are_told_before_the_user_is_asked() {
    let desk = desk().await;
    desk.pair().await;
    let cases = [
        ("r1", "Nobody/password", "No vault item"),
        ("r2", "GitHub/colour", "no field named colour"),
        ("r3", "Twin/password", "several vault items"),
        ("r4", "Bank/totp", "Bank has no one-time code"),
        ("r5", "Old/password", "No vault item"),
        ("r6", "GitHub", "must be item/field"),
        ("r7", "ssh-ed/password", "not a login"),
    ];
    let requests: Vec<Value> = cases.iter().map(|(id, r, _)| release(&desk, id, &[r], 600)).collect();
    desk.send(&requests).await;
    for (id, _, says) in cases {
        let message = desk.error(id).await;
        assert!(message.contains(says), "{id}: {message}");
    }
    let short_lease = release(&desk, "r8", &["GitHub/password"], 30);
    desk.send(&[short_lease]).await;
    desk.error("r8").await;
    assert!(desk.waiting().await.is_empty());
}

// ---- SSH ------------------------------------------------------------------------------------------------------------

fn ssh_string(out: &mut Vec<u8>, s: &[u8]) {
    out.extend_from_slice(&u32::try_from(s.len()).unwrap().to_be_bytes());
    out.extend_from_slice(s);
}

/// An SSH user authentication request for `blob`, as an SSH client asks its agent to sign it.
fn userauth(blob: &[u8], alg: &str, host_key: Option<&[u8]>) -> Vec<u8> {
    let mut out = Vec::new();
    ssh_string(&mut out, &[42; 32]);
    out.push(50);
    ssh_string(&mut out, b"git");
    ssh_string(&mut out, b"ssh-connection");
    ssh_string(
        &mut out,
        if host_key.is_some() {
            b"publickey-hostbound-v00@openssh.com"
        } else {
            b"publickey"
        },
    );
    out.push(1);
    ssh_string(&mut out, alg.as_bytes());
    ssh_string(&mut out, blob);
    if let Some(k) = host_key {
        ssh_string(&mut out, k);
    }
    out
}

fn sign_call(desk: &Desk, id: &str, key: &Key, flags: i64, host: Option<&str>, data: &[u8]) -> Value {
    let mut args = json!({"key": key.fingerprint, "data_base64": BASE64.encode(data), "flags": flags,
        "client_key": desk.public(), "nonce": format!("nonce-{id}")});
    if let Some(h) = host {
        args["host"] = json!(h);
    }
    call(id, DESK, "vault", "ssh_sign", &args)
}

/// Reads SSH `string`s.
fn strings(mut buf: &[u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    while !buf.is_empty() {
        let len = usize::try_from(u32::from_be_bytes(buf[..4].try_into().unwrap())).unwrap();
        out.push(buf[4..4 + len].to_vec());
        buf = &buf[4 + len..];
    }
    out
}

/// An mpint as a fixed-size big-endian number.
fn fixed(mpint: &[u8], size: usize) -> Vec<u8> {
    let trimmed: Vec<u8> = mpint.iter().copied().skip_while(|b| *b == 0).collect();
    let mut out = vec![0; size - trimmed.len()];
    out.extend(trimmed);
    out
}

/// Checks an SSH signature blob over `data` against the public key `key` with ring.
fn verify(key: &Key, data: &[u8], blob: &[u8], alg: &str) {
    let parts = strings(blob);
    assert_eq!(parts.len(), 2, "string alg, string signature");
    assert_eq!(String::from_utf8_lossy(&parts[0]), alg);
    let sig = &parts[1];
    let public = strings(&key.blob());
    match alg {
        "ssh-ed25519" => UnparsedPublicKey::new(&signature::ED25519, &public[1]).verify(data, sig).unwrap(),
        "rsa-sha2-256" | "rsa-sha2-512" => {
            let components = signature::RsaPublicKeyComponents {
                n: fixed(&public[2], public[2].iter().skip_while(|b| **b == 0).count()),
                e: fixed(&public[1], public[1].iter().skip_while(|b| **b == 0).count()),
            };
            let params = if alg == "rsa-sha2-256" {
                &signature::RSA_PKCS1_2048_8192_SHA256
            } else {
                &signature::RSA_PKCS1_2048_8192_SHA512
            };
            components.verify(params, data, sig).unwrap();
        }
        _ => {
            let (algorithm, size) = if alg.ends_with("nistp256") {
                (&signature::ECDSA_P256_SHA256_FIXED, 32)
            } else {
                (&signature::ECDSA_P384_SHA384_FIXED, 48)
            };
            let rs = strings(sig);
            let mut fixed_sig = fixed(&rs[0], size);
            fixed_sig.extend(fixed(&rs[1], size));
            UnparsedPublicKey::new(algorithm, &public[2]).verify(data, &fixed_sig).unwrap();
        }
    }
}

#[tokio::test]
async fn ssh_keys_are_listed_with_their_public_line_fingerprint_and_name() {
    let desk = desk().await;
    desk.pair().await;
    let keys = call("r1", DESK, "vault", "ssh_keys", &json!({"client_key": desk.public(), "nonce": "n"}));
    desk.send(&[keys]).await;
    let view = desk.core.approval_view("r1".to_owned()).await.unwrap();
    assert_eq!(view.kind, ApprovalKind::Fetch);
    assert_eq!(view.messages.len(), 4, "the broken key is left out");
    let ids: Vec<String> = view.messages.iter().map(|m| m.id.clone()).collect();
    desk.core
        .approve(
            "r1".to_owned(),
            choice(&ids.iter().map(String::as_str).collect::<Vec<_>>(), standing(&["ssh_keys"], &[])),
        )
        .await
        .unwrap();
    let items = desk.data("r1").await["items"].as_array().unwrap().clone();
    for key in [&ED, &P256, &P384, &RSA] {
        let item = items.iter().find(|i| i["name"] == key.name).unwrap_or_else(|| panic!("{} listed", key.name));
        assert_eq!(item["fingerprint"], key.fingerprint);
        assert_eq!(item["public_key"], key.line());
    }
    no_secrets("the keys", &format!("{items:?}"));
    // The permission answers the next listing at once.
    let again = call("r2", DESK, "vault", "ssh_keys", &json!({"client_key": desk.public(), "nonce": "n"}));
    desk.send(&[again]).await;
    assert_eq!(desk.data("r2").await["items"].as_array().unwrap().len(), 4);
}

#[tokio::test]
async fn ssh_signatures_verify_for_every_kind_of_key() {
    let desk = desk().await;
    desk.pair().await;
    let cases: [(&str, &Key, i64, &str, &str); 5] = [
        ("r1", &ED, 0, "ssh-ed25519", "ssh-ed25519"),
        ("r2", &RSA, 2, "rsa-sha2-256", "rsa-sha2-256"),
        ("r3", &RSA, 4, "rsa-sha2-512", "rsa-sha2-512"),
        ("r4", &P256, 0, "ecdsa-sha2-nistp256", "ecdsa-sha2-nistp256"),
        ("r5", &P384, 0, "ecdsa-sha2-nistp384", "ecdsa-sha2-nistp384"),
    ];
    for (id, key, flags, request_alg, alg) in cases {
        let data = userauth(&key.blob(), request_alg, None);
        desk.send(&[sign_call(&desk, id, key, flags, Some("GitHub.com"), &data)]).await;
        let view = desk.core.approval_view(id.to_owned()).await.unwrap();
        let ssh = view.ssh.clone().expect("the sign-in");
        assert_eq!(
            (ssh.key_name.as_str(), ssh.key_fingerprint.as_str(), ssh.host.as_deref(), ssh.host_key),
            (key.name, key.fingerprint, Some("github.com"), None)
        );
        assert_eq!(view.preview, [format!("Sign in to github.com with {}", key.name), "As user git".to_owned()]);
        assert_eq!(view.class, "ssh");
        desk.core.approve(id.to_owned(), choice(&[], None)).await.unwrap();
        let data_out = desk.data(id).await;
        assert_eq!(data_out.as_object().unwrap().len(), 1, "only the sealed signature");
        let signed: SshSignature = desk.open(&data_out["sealed"]);
        assert_eq!(signed.nonce, format!("nonce-{id}"));
        verify(key, &data, &BASE64.decode(signed.signature_base64.as_bytes()).unwrap(), alg);
    }
    no_secrets("signing", &desk.everything_shown(&[]).await);
}

#[tokio::test]
async fn an_ssh_permission_covers_one_key_on_one_server() {
    let desk = desk().await;
    desk.pair().await;
    let data = userauth(&ED.blob(), "ssh-ed25519", None);
    desk.send(&[sign_call(&desk, "r1", &ED, 0, Some("github.com"), &data)]).await;
    let view = desk.core.approval_view("r1".to_owned()).await.unwrap();
    let resources: Vec<_> = view.resources.iter().map(|r| (r.id.clone(), r.label.as_str(), r.wider)).collect();
    assert_eq!(
        resources,
        [
            (format!("{}@github.com", ED.fingerprint), "Ed key on github.com", false),
            (ED.fingerprint.to_owned(), "Ed key", true)
        ]
    );
    let on_github = format!("{}@github.com", ED.fingerprint);
    desk.core.approve("r1".to_owned(), choice(&[], standing(&[on_github.as_str()], &["ssh"]))).await.unwrap();

    let host_key = b"server host key blob";
    desk.send(&[
        sign_call(&desk, "r2", &ED, 0, Some("github.com"), &data),
        sign_call(&desk, "r3", &ED, 0, Some("gitlab.com"), &data),
        sign_call(&desk, "r4", &ED, 0, None, &data),
        sign_call(&desk, "r5", &ED, 0, None, &userauth(&ED.blob(), "ssh-ed25519", Some(host_key))),
    ])
    .await;
    assert_eq!(desk.waiting().await, ["r3", "r4", "r5"]);
    let signed: SshSignature = desk.open(&desk.data("r2").await["sealed"]);
    verify(&ED, &data, &BASE64.decode(signed.signature_base64.as_bytes()).unwrap(), "ssh-ed25519");

    let unknown = desk.core.approval_view("r4".to_owned()).await.unwrap();
    assert!(unknown.no_standing, "a server nobody named is asked for every time");
    assert_eq!(unknown.preview[0], "Sign in to an unknown server with Ed key");
    assert!(matches!(
        desk.core.approve("r4".to_owned(), choice(&[], standing(&[ED.fingerprint], &["ssh"]))).await,
        Err(CoreError::Invalid { .. })
    ));
    let bound = desk.core.approval_view("r5".to_owned()).await.unwrap();
    let host_fp = bound.ssh.unwrap().host_key.expect("the host key the sign-in is bound to");
    assert!(host_fp.starts_with("SHA256:"));
    assert!(!bound.no_standing);
    assert_eq!(bound.preview[0], format!("Sign in to {host_fp} with Ed key"));
}

#[tokio::test]
async fn only_sign_ins_with_the_named_key_are_signed() {
    let desk = desk().await;
    desk.pair().await;
    let mut unknown_key = sign_call(&desk, "r4", &ED, 0, Some("h"), &userauth(&ED.blob(), "ssh-ed25519", None));
    unknown_key["call"]["args"]["key"] = json!("SHA256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA");
    let mut mismatch = sign_call(&desk, "r5", &ED, 0, None, &userauth(&ED.blob(), "ssh-ed25519", Some(b"hk")));
    mismatch["call"]["args"]["host_key"] = json!("SHA256:somethingelse");
    desk.send(&[
        sign_call(&desk, "r1", &RSA, 0, Some("h"), &userauth(&RSA.blob(), "ssh-rsa", None)),
        sign_call(&desk, "r2", &ED, 0, Some("h"), b"SSHSIG\0\0\0\x01anything"),
        sign_call(&desk, "r3", &ED, 0, Some("h"), &userauth(&RSA.blob(), "rsa-sha2-256", None)),
        unknown_key,
        mismatch,
        sign_call(&desk, "r6", &ED, 0, Some("bad host!"), &userauth(&ED.blob(), "ssh-ed25519", None)),
    ])
    .await;
    for (id, says) in [
        ("r1", "SHA-1"),
        ("r2", "Only an SSH sign-in"),
        ("r3", "another key"),
        ("r4", "No SSH key"),
        ("r5", "host key"),
        ("r6", "host"),
    ] {
        let message = desk.error(id).await;
        assert!(message.contains(says), "{id}: {message}");
    }
    assert!(desk.waiting().await.is_empty());
}
