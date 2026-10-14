//! Reins Secrets on the phone: an environment released for one command, bound to who runs it, where and what; standing
//! permissions that bind every dimension; production asked every time; changes sent from the computer in a box only
//! the phone opens; names listed. A mock Vaultwarden serves an encrypted vault with a project.

mod common;

use common::desktop::{DESK, Desk, call, choice, desk_with, mount_reins};
use crypto_box::aead::{Aead as _, AeadCore as _, OsRng};
use crypto_box::{PublicKey, SalsaBox};
use data_encoding::BASE64URL_NOPAD;
use reins_core::crypto::{Kdf, VaultKey, master_key};
use reins_core::{ApprovalKind, CoreConfig, GrantScopeChoice, SecretsScopeChoice, StandingGrant};
use reins_proto::desktop::{
    CommandScope, EnvChanges, PhoneKey, SecretGrant, SecretsListing, SecretsStoreAck, SecretsToStore, TO_PHONE,
    decode_key, secrets_check,
};
use serde_json::{Value, json};
use wiremock::matchers::{header_exists, method, path, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

const EMAIL: &str = "me@example.com";
const PASSWORD: &str = "correct horse battery";
const ROOT: &str = "/home/me/dev/acme";
const NPM: &str = "/usr/bin/npm";
const VALUES: &[&str] = &["alpha-secret", "beta-secret", "prod-secret", "store-secret", "a note of my own"];

fn user_key() -> VaultKey {
    VaultKey::from_bytes(&[7u8; 64]).unwrap()
}

fn t(key: &VaultKey, s: &str) -> Value {
    json!(key.encrypt(s.as_bytes()).unwrap())
}

fn note(key: &VaultKey, id: &str, folder: &str, name: &str, fields: &[(&str, &str)]) -> Value {
    let fields: Vec<Value> = fields
        .iter()
        .map(|(n, v)| json!({"name": t(key, n), "value": t(key, v), "type": 1, "linkedId": null}))
        .collect();
    json!({"id": id, "type": 2, "name": t(key, name), "notes": null, "key": null, "folderId": folder,
        "favorite": false, "deletedDate": null, "archivedDate": null, "organizationId": null, "reprompt": 0,
        "fields": fields, "passwordHistory": [], "attachments": null, "revisionDate": format!("rev-{id}"),
        "login": null, "secureNote": {"type": 0}, "card": null, "identity": null, "sshKey": null})
}

fn ciphers(user: &VaultKey) -> Vec<Value> {
    vec![
        note(
            user,
            "dev",
            "f-acme",
            "acme/dev",
            &[
                ("reins.kind", "env/1"),
                ("reins.env", "dev"),
                ("B", "beta-secret"),
                ("A", "alpha-secret"),
                ("not a name", "x"),
            ],
        ),
        note(
            user,
            "prod",
            "f-acme",
            "acme/production",
            &[("reins.kind", "env/1"), ("reins.env", "production"), ("A", "prod-secret")],
        ),
        // Named like an environment, without the marker: something of the user's own.
        note(user, "mine", "f-acme", "acme/mine", &[("A", "a note of my own")]),
        // Marked, but in another project's folder: that project's environment, whatever its name says.
        note(
            user,
            "store-dev",
            "f-store",
            "acme/dev",
            &[("reins.kind", "env/1"), ("reins.env", "dev"), ("A", "store-secret")],
        ),
    ]
}

async fn desk() -> Desk {
    let server = MockServer::start().await;
    mount_reins(&server).await;
    let master = master_key(
        PASSWORD,
        EMAIL,
        Kdf::Pbkdf2 {
            iterations: 5000,
        },
    )
    .unwrap();
    let user = user_key();
    let wrapped = master.stretch().encrypt(&user.to_bytes()).unwrap();
    let folders = json!([
        {"id": "f-acme", "name": t(&user, "Reins/acme")},
        {"id": "f-store", "name": t(&user, "Reins/store")},
        {"id": "f-other", "name": t(&user, "Work")}
    ]);
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
    Mock::given(method("POST"))
        .and(path("/api/folders"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "f-new"})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/ciphers"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "new-env", "revisionDate": "rev-new"})))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path_regex(r"^/api/ciphers/[a-z0-9-]+$"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"revisionDate": "rev-changed"})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/reins/api/desktop/revocations"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/user"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"login": "Octo"})))
        .mount(&server)
        .await;
    let github_base = server.uri();
    let desk = desk_with(
        server,
        EMAIL,
        PASSWORD,
        CoreConfig {
            github_base,
            ..CoreConfig::default()
        },
    )
    .await;
    desk.core.add_token_account("vault".to_owned(), PASSWORD.to_owned()).await.unwrap();
    desk
}

struct Ask<'a> {
    env: &'a str,
    fields: Option<&'a [&'a str]>,
    args: &'a [&'a str],
    root: &'a str,
    runner: &'a str,
}

const DEV: Ask<'static> = Ask {
    env: "dev",
    fields: None,
    args: &["run", "dev"],
    root: ROOT,
    runner: "person",
};

fn release(desk: &Desk, id: &str, ask: &Ask<'_>) -> Value {
    let mut args = json!({"project": "acme", "env": ask.env, "program": NPM, "args": ask.args, "root": ask.root,
        "runner": ask.runner, "lease_secs": 3_600, "client_key": desk.public(), "nonce": format!("nonce-{id}")});
    if let Some(f) = ask.fields {
        args["fields"] = json!(f);
    }
    call(id, DESK, "vault", "env_release", &args)
}

fn remember(command: &str, all_variables: bool) -> Option<StandingGrant> {
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
            resources: vec![],
            classes: vec![],
            secrets: Some(SecretsScopeChoice {
                command: command.to_owned(),
                all_variables,
            }),
        },
    })
}

fn no_values(what: &str, shown: &str) {
    for v in VALUES {
        assert!(!shown.contains(v), "{what} shows {v}");
    }
}

fn words(w: &[&str]) -> Vec<String> {
    w.iter().map(|x| (*x).to_owned()).collect()
}

#[tokio::test]
async fn an_environment_is_shown_by_names_and_released_bound_to_who_where_and_what() {
    let desk = desk().await;
    desk.pair().await;
    desk.send(&[release(&desk, "r1", &DEV)]).await;
    assert_eq!(desk.waiting().await, ["r1"]);
    let view = desk.core.approval_view("r1".to_owned()).await.unwrap();
    assert_eq!(view.kind, ApprovalKind::Write);
    let r = view.env_release.clone().expect("the release");
    assert_eq!((r.project.as_str(), r.env.as_str(), r.command.as_str()), ("acme", "dev", "npm run dev"));
    assert_eq!(r.variables, ["A", "B"], "sorted; reins.kind and names that are no variables left out");
    assert!(r.all_variables && !r.agent && !r.always_ask && !r.print);
    assert_eq!((r.runner_label.as_str(), r.root.as_str(), r.program.as_str()), ("You", ROOT, NPM));
    let offered: Vec<(&str, &str)> = r.commands.iter().map(|c| (c.id.as_str(), c.label.as_str())).collect();
    assert_eq!(offered, [("exact", "npm run dev"), ("prefix:1", "npm run *")]);
    assert!(view.quick.is_none(), "never from a notification");
    assert!(view.headline.contains("all of acme"), "{}", view.headline);
    no_values("the approval", &format!("{view:?}"));

    desk.core.approve("r1".to_owned(), choice(&[], None)).await.unwrap();
    let grant: SecretGrant = desk.open(&desk.data("r1").await["sealed"]);
    assert_eq!(
        grant.secrets,
        [("A".to_owned(), "alpha-secret".to_owned()), ("B".to_owned(), "beta-secret".to_owned())]
    );
    assert_eq!((grant.project.as_deref(), grant.env.as_deref()), (Some("acme"), Some("dev")));
    assert_eq!(grant.revision.as_deref(), Some("rev-dev"));
    assert_eq!((grant.runner.as_deref(), grant.root.as_deref()), (Some("person"), Some(ROOT)));
    assert_eq!(grant.command, Some(CommandScope::exact(NPM, &words(&["run", "dev"]))), "exactly what was approved");
    no_values("everything else", &desk.everything_shown(&["r1"]).await.replace(&desk.data("r1").await.to_string(), ""));
}

#[tokio::test]
async fn a_remembered_release_binds_every_dimension() {
    let desk = desk().await;
    desk.pair().await;
    let asked: &[&str] = &["A"];
    let first = Ask {
        fields: Some(asked),
        ..DEV
    };
    desk.send(&[release(&desk, "r1", &first)]).await;
    desk.core.approve("r1".to_owned(), choice(&[], remember("prefix:1", false))).await.unwrap();
    let grant: SecretGrant = desk.open(&desk.data("r1").await["sealed"]);
    assert_eq!(
        grant.command,
        Some(CommandScope {
            program: NPM.to_owned(),
            args: words(&["run"]),
            prefix: true
        }),
        "the prefix the user picked"
    );

    // The same runner, folder and variables, another `npm run …`: covered.
    let test = Ask {
        args: &["run", "test", "--watch"],
        ..first
    };
    desk.send(&[release(&desk, "r2", &test)]).await;
    assert!(desk.waiting().await.is_empty(), "covered by the permission");
    let covered: SecretGrant = desk.open(&desk.data("r2").await["sealed"]);
    assert_eq!(covered.secrets, [("A".to_owned(), "alpha-secret".to_owned())]);
    assert_eq!(covered.command.unwrap().shown(), "npm run *");

    // Anything else is asked again.
    let both: &[&str] = &["A", "B"];
    let others = [
        (
            "r3",
            Ask {
                runner: "agent:claude-code",
                ..first
            },
        ),
        (
            "r4",
            Ask {
                root: "/home/me/dev/other",
                ..first
            },
        ),
        (
            "r5",
            Ask {
                fields: Some(both),
                ..first
            },
        ),
        (
            "r6",
            Ask {
                fields: None,
                ..first
            },
        ),
        (
            "r7",
            Ask {
                args: &["runx"],
                ..first
            },
        ),
        (
            "r8",
            Ask {
                args: &["exec", "run"],
                ..first
            },
        ),
    ];
    let requests: Vec<Value> = others.iter().map(|(id, a)| release(&desk, id, a)).collect();
    desk.send(&requests).await;
    assert_eq!(desk.waiting().await, ["r3", "r4", "r5", "r6", "r7", "r8"]);
}

#[tokio::test]
async fn without_a_choice_on_the_phone_a_permission_covers_only_the_exact_command() {
    let desk = desk().await;
    desk.pair().await;
    desk.send(&[release(&desk, "r1", &DEV)]).await;
    let mut plain = remember("exact", false);
    plain.as_mut().unwrap().scope.secrets = None;
    desk.core.approve("r1".to_owned(), choice(&[], plain)).await.unwrap();
    desk.send(&[release(&desk, "r2", &DEV)]).await;
    assert!(desk.waiting().await.is_empty(), "the same command is covered");
    let test = Ask {
        args: &["run", "test"],
        ..DEV
    };
    desk.send(&[release(&desk, "r3", &test)]).await;
    assert_eq!(desk.waiting().await, ["r3"], "another command is asked again");
    // A command the phone did not offer cannot be picked.
    let err = desk.core.approve("r3".to_owned(), choice(&[], remember("prefix:0", true))).await;
    assert!(err.is_err());
}

#[tokio::test]
async fn production_is_asked_every_time_until_the_user_says_otherwise_on_the_phone() {
    let desk = desk().await;
    desk.pair().await;
    let prod = Ask {
        env: "production",
        ..DEV
    };
    desk.send(&[release(&desk, "r1", &prod)]).await;
    let view = desk.core.approval_view("r1".to_owned()).await.unwrap();
    assert!(view.no_standing && view.env_release.as_ref().unwrap().always_ask);
    assert!(desk.core.approve("r1".to_owned(), choice(&[], remember("exact", false))).await.is_err());
    desk.core.approve("r1".to_owned(), choice(&[], None)).await.unwrap();

    desk.core.set_secrets_always_ask("acme".to_owned(), "production".to_owned(), Some(false)).await.unwrap();
    desk.send(&[release(&desk, "r2", &prod)]).await;
    let view = desk.core.approval_view("r2".to_owned()).await.unwrap();
    let r = view.env_release.clone().unwrap();
    assert!(!view.no_standing && !r.always_ask && r.always_ask_set);

    // The repository can make a development environment stricter.
    let mut strict = release(&desk, "r3", &DEV);
    strict["call"]["args"]["confirm"] = json!("always");
    desk.send(&[strict]).await;
    assert!(desk.core.approval_view("r3".to_owned()).await.unwrap().no_standing);
}

/// `release` with some arguments changed.
fn release_with(desk: &Desk, id: &str, ask: &Ask<'_>, changed: &Value) -> Value {
    let mut r = release(desk, id, ask);
    for (k, v) in changed.as_object().unwrap() {
        r["call"]["args"][k] = v.clone();
    }
    r
}

#[tokio::test]
async fn a_program_out_of_place_is_shown_by_its_path_with_a_warning() {
    let desk = desk().await;
    desk.pair().await;
    desk.send(&[release_with(&desk, "t1", &DEV, &json!({"program": "/tmp/x/npm", "typed": "npm"}))]).await;
    let view = desk.core.approval_view("t1".to_owned()).await.unwrap();
    let r = view.env_release.clone().unwrap();
    assert_eq!(r.command, "/tmp/x/npm run dev");
    assert_eq!(r.program_warning.as_deref(), Some("In a temporary folder"));
    assert!(view.headline.contains("/tmp/x/npm"), "{}", view.headline);
    let inside = format!("{ROOT}/node_modules/.bin/npm");
    desk.send(&[release_with(&desk, "t2", &DEV, &json!({"program": inside}))]).await;
    let r = desk.core.approval_view("t2".to_owned()).await.unwrap().env_release.unwrap();
    assert_eq!(r.program_warning.as_deref(), Some("Inside the project folder"));
    desk.send(&[release_with(&desk, "t3", &DEV, &json!({"program": "/home/me/bin/npm", "path_check": "other"}))]).await;
    let r = desk.core.approval_view("t3".to_owned()).await.unwrap().env_release.unwrap();
    assert_eq!(
        (r.command.as_str(), r.program_warning.as_deref()),
        ("/home/me/bin/npm run dev", Some("Not the one on your PATH"))
    );
    desk.send(&[release(&desk, "t4", &DEV)]).await;
    let r = desk.core.approval_view("t4".to_owned()).await.unwrap().env_release.unwrap();
    assert_eq!((r.command.as_str(), r.program_warning), ("npm run dev", None));
}

#[tokio::test]
async fn launchers_by_either_name_and_long_code_are_never_remembered_wider() {
    let desk = desk().await;
    desk.pair().await;
    let cli = "/usr/lib/node_modules/npm/bin/npm-cli.js";
    let exec = Ask {
        args: &["exec", "vite"],
        ..DEV
    };
    desk.send(&[release_with(&desk, "l1", &exec, &json!({"program": cli, "typed": "npm"}))]).await;
    let r = desk.core.approval_view("l1".to_owned()).await.unwrap().env_release.unwrap();
    let offered: Vec<&str> = r.commands.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(offered, ["exact"], "npm exec * is never offered");
    // Typed as a launcher, resolved to something with another name: still a launcher.
    desk.send(&[release_with(&desk, "l2", &exec, &json!({"program": "/opt/tools/run", "typed": "npx"}))]).await;
    let r = desk.core.approval_view("l2".to_owned()).await.unwrap().env_release.unwrap();
    assert_eq!(r.commands.len(), 1);
    // Code longer than the phone shows: approved once at a time.
    let code = "print(1); ".repeat(40);
    let long = Ask {
        args: &["-c", &code],
        ..DEV
    };
    desk.send(&[release_with(&desk, "l3", &long, &json!({"program": "/usr/bin/python3"}))]).await;
    let view = desk.core.approval_view("l3".to_owned()).await.unwrap();
    let r = view.env_release.clone().unwrap();
    assert!(r.commands.is_empty() && view.no_standing && r.always_ask);
    assert!(r.command.contains(code.trim_end()), "all of the code is shown");
    assert!(desk.core.approve("l3".to_owned(), choice(&[], remember("exact", false))).await.is_err());
}

#[tokio::test]
async fn mistakes_are_told_before_the_user_is_asked() {
    let desk = desk().await;
    desk.pair().await;
    let missing: &[&str] = &["A", "NOPE"];
    let cases = [
        (
            "r1",
            Ask {
                env: "staging",
                ..DEV
            },
            "No environment acme/staging",
        ),
        (
            "r2",
            Ask {
                env: "mine",
                ..DEV
            },
            "an item of your own",
        ),
        (
            "r3",
            Ask {
                fields: Some(missing),
                ..DEV
            },
            "has no NOPE",
        ),
    ];
    let requests: Vec<Value> = cases.iter().map(|(id, a, _)| release(&desk, id, a)).collect();
    desk.send(&requests).await;
    for (id, _, says) in cases {
        let message = desk.error(id).await;
        assert!(message.contains(says), "{id}: {message}");
    }
    assert!(desk.waiting().await.is_empty());
}

// ---- store and list -------------------------------------------------------------------------------------------------

async fn phone_key(desk: &Desk) -> String {
    desk.send(&[call("k1", DESK, "vault", "phone_key", &json!({"client_key": desk.public(), "nonce": "nk"}))]).await;
    desk.core.approve("k1".to_owned(), choice(&[], None)).await.unwrap();
    let answer: PhoneKey = desk.open(&desk.data("k1").await["sealed"]);
    answer.public_key
}

fn box_to(desk: &Desk, phone: &str, value: &impl serde::Serialize) -> String {
    let cipher = SalsaBox::new(&PublicKey::from(decode_key(phone).unwrap()), &desk.key);
    let nonce = SalsaBox::generate_nonce(&mut OsRng);
    let mut out = nonce.to_vec();
    out.extend(cipher.encrypt(&nonce, serde_json::to_vec(value).unwrap().as_slice()).unwrap());
    BASE64URL_NOPAD.encode(&out)
}

fn open_from_phone<T: serde::de::DeserializeOwned>(desk: &Desk, phone: &str, boxed: &Value) -> T {
    let bytes = BASE64URL_NOPAD.decode(boxed.as_str().unwrap().as_bytes()).unwrap();
    let (nonce, ciphertext) = bytes.split_at(24);
    let nonce = crypto_box::Nonce::from(<[u8; 24]>::try_from(nonce).unwrap());
    let plain =
        SalsaBox::new(&PublicKey::from(decode_key(phone).unwrap()), &desk.key).decrypt(&nonce, ciphertext).unwrap();
    serde_json::from_slice(&plain).unwrap()
}

fn store(desk: &Desk, id: &str, phone: &str, envs: Vec<EnvChanges>) -> Value {
    store_all(desk, id, phone, envs, vec![], vec![])
}

fn store_all(
    desk: &Desk,
    id: &str,
    phone: &str,
    envs: Vec<EnvChanges>,
    ssh: Vec<reins_proto::desktop::SshKeyToStore>,
    tokens: Vec<reins_proto::desktop::HostToken>,
) -> Value {
    let nonce = format!("nonce-{id}");
    let names: Vec<String> = envs.iter().map(EnvChanges::item).collect();
    let keys: Vec<String> = ssh.iter().map(|k| k.name.clone()).collect();
    let hosts: Vec<String> = tokens.iter().map(|t| t.host.clone()).collect();
    let boxed = box_to(
        desk,
        phone,
        &SecretsToStore {
            v: 1,
            dir: TO_PHONE.to_owned(),
            nonce: nonce.clone(),
            created_at: reins_core::store::unix_now(),
            envs,
            ssh,
            tokens,
        },
    );
    call(
        id,
        DESK,
        "vault",
        "secrets_store",
        &json!({"envs": names, "ssh": keys, "hosts": hosts, "sealed": boxed, "client_key": desk.public(),
            "nonce": nonce}),
    )
}

fn changes(project: &str, env: &str, set: &[(&str, &str)]) -> EnvChanges {
    let mut c = EnvChanges::new(project, env);
    c.set = set.iter().map(|(n, v)| ((*n).to_owned(), (*v).to_owned())).collect();
    c
}

async fn sent(desk: &Desk, verb: &str, on: &str) -> Vec<Value> {
    desk.server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.method.as_str() == verb && r.url.path() == on)
        .map(|r| serde_json::from_slice(&r.body).unwrap_or(Value::Null))
        .collect()
}

fn dec(v: &Value) -> String {
    user_key().decrypt_text(v.as_str().unwrap()).unwrap().to_string()
}

fn fields_of(body: &Value) -> Vec<(String, String)> {
    body["fields"].as_array().unwrap().iter().map(|f| (dec(&f["name"]), dec(&f["value"]))).collect()
}

#[tokio::test]
async fn a_new_project_is_made_with_one_approval_and_the_server_never_sees_a_value() {
    let desk = desk().await;
    desk.pair().await;
    let phone = phone_key(&desk).await;
    let batch = vec![changes("shop", "dev", &[("KEY", "new-secret-1"), ("TOKEN", "new-secret-2")])];
    let check = secrets_check(&batch);
    desk.send(&[store(&desk, "s1", &phone, batch)]).await;
    let view = desk.core.approval_view("s1".to_owned()).await.unwrap();
    let s = view.secrets_store.clone().expect("the store");
    assert_eq!((s.check.as_str(), s.total), (check.as_str(), 2));
    assert!(s.envs[0].new_env && s.envs[0].added == ["KEY", "TOKEN"]);
    assert!(view.no_standing && view.quick.is_none(), "asked every time");
    desk.core.approve("s1".to_owned(), choice(&[], None)).await.unwrap();

    let folders = sent(&desk, "POST", "/api/folders").await;
    assert_eq!(dec(&folders[0]["name"]), "Reins/shop");
    let made = sent(&desk, "POST", "/api/ciphers").await;
    assert_eq!((made[0]["type"].as_i64(), made[0]["folderId"].as_str()), (Some(2), Some("f-new")));
    assert_eq!(dec(&made[0]["name"]), "shop/dev");
    assert_eq!(
        fields_of(&made[0]),
        [
            ("reins.kind".to_owned(), "env/1".to_owned()),
            ("reins.env".to_owned(), "dev".to_owned()),
            ("KEY".to_owned(), "new-secret-1".to_owned()),
            ("TOKEN".to_owned(), "new-secret-2".to_owned())
        ]
    );
    assert!(made[0]["fields"].as_array().unwrap().iter().all(|f| f["type"] == 1), "hidden fields");
    let ack: SecretsStoreAck = open_from_phone(&desk, &phone, &desk.data("s1").await["sealed"]);
    assert_eq!((ack.nonce.as_str(), ack.envs[0].created, ack.envs[0].added), ("nonce-s1", true, 2));
    let shown = desk.everything_shown(&["s1"]).await;
    assert!(!shown.contains("new-secret-1") && !shown.contains("new-secret-2"));
}

#[tokio::test]
async fn an_environment_is_changed_only_as_asked_and_keeps_its_earlier_values() {
    let desk = desk().await;
    desk.pair().await;
    let phone = phone_key(&desk).await;
    // Without replace, a different value is refused before the user is asked.
    desk.send(&[store(&desk, "s1", &phone, vec![changes("acme", "dev", &[("A", "changed")])])]).await;
    assert!(desk.error("s1").await.contains("has a A already"));

    let mut edit = changes("acme", "dev", &[("A", "changed"), ("C", "added")]);
    edit.replace = true;
    edit.unset = vec!["B".to_owned()];
    desk.send(&[store(&desk, "s2", &phone, vec![edit])]).await;
    let view = desk.core.approval_view("s2".to_owned()).await.unwrap();
    let e = &view.secrets_store.as_ref().unwrap().envs[0];
    assert_eq!(
        (e.added.clone(), e.replaced.clone(), e.removed.clone()),
        (vec!["C".to_owned()], vec!["A".to_owned()], vec!["B".to_owned()])
    );
    desk.core.approve("s2".to_owned(), choice(&[], None)).await.unwrap();
    let put = sent(&desk, "PUT", "/api/ciphers/dev").await;
    let fields = fields_of(&put[0]);
    assert!(fields.contains(&("A".to_owned(), "changed".to_owned())));
    assert!(fields.contains(&("C".to_owned(), "added".to_owned())));
    assert!(!fields.iter().any(|(n, _)| n == "B"));
    assert!(fields.contains(&("not a name".to_owned(), "x".to_owned())), "what it does not touch stays");
    let mut history: Vec<String> =
        put[0]["passwordHistory"].as_array().unwrap().iter().map(|h| dec(&h["password"])).collect();
    history.sort();
    assert_eq!(history, ["A: alpha-secret", "B: beta-secret"]);
    assert_eq!(put[0]["lastKnownRevisionDate"], "rev-dev");
    // Computers holding the earlier values drop them.
    let told = sent(&desk, "POST", "/reins/api/desktop/revocations").await;
    let note: reins_proto::desktop::LeaseRevocation = desk.open(&told[0]["items"][0]["sealed"]);
    assert_eq!((note.project.as_deref(), note.env.as_deref()), (Some("acme"), Some("dev")));

    // Copying an environment happens on the phone: the values never travel.
    let mut copy = EnvChanges::new("acme", "staging");
    copy.copy_from = Some("dev".to_owned());
    desk.send(&[store(&desk, "s3", &phone, vec![copy])]).await;
    let view = desk.core.approval_view("s3".to_owned()).await.unwrap();
    assert_eq!(view.secrets_store.as_ref().unwrap().envs[0].copied, 2);
    // A box sent again (the same nonce, another request) is never shown twice.
    let mut again = store(&desk, "s2", &phone, vec![changes("acme", "dev", &[("Z", "z")])]);
    again["id"] = json!("s4");
    desk.send(&[again]).await;
    assert!(desk.error("s4").await.contains("seen already"));
}

#[tokio::test]
async fn names_are_listed_never_values() {
    let desk = desk().await;
    desk.pair().await;
    let phone = phone_key(&desk).await;
    desk.send(&[call("l1", DESK, "vault", "secrets_list", &json!({"client_key": desk.public(), "nonce": "nl"}))]).await;
    desk.core.approve("l1".to_owned(), choice(&[], None)).await.unwrap();
    let listing: SecretsListing = open_from_phone(&desk, &phone, &desk.data("l1").await["sealed"]);
    let names: Vec<&str> = listing.projects.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["acme", "store"], "by folder and marker, whatever the items are called");
    let envs: Vec<(&str, Vec<String>)> =
        listing.projects[0].envs.iter().map(|e| (e.name.as_str(), e.variables.clone())).collect();
    assert_eq!(envs, [("dev", words(&["A", "B"])), ("production", words(&["A"]))]);
    assert!(!format!("{listing:?}").contains("secret"));
}

#[tokio::test]
async fn revoking_on_all_computers_ends_the_permissions_and_tells_every_computer() {
    let desk = desk().await;
    desk.pair().await;
    desk.send(&[release(&desk, "r1", &DEV)]).await;
    desk.core.approve("r1".to_owned(), choice(&[], remember("exact", false))).await.unwrap();
    let told = desk.core.revoke_secrets_everywhere(Some("acme".to_owned()), None).await.unwrap();
    assert_eq!(told, 1);
    let posted = sent(&desk, "POST", "/reins/api/desktop/revocations").await;
    assert_eq!(posted[0]["items"][0]["connection_id"], DESK);
    let revocation: reins_proto::desktop::LeaseRevocation = desk.open(&posted[0]["items"][0]["sealed"]);
    assert_eq!(revocation.project.as_deref(), Some("acme"));
    desk.send(&[release(&desk, "r2", &DEV)]).await;
    assert_eq!(desk.waiting().await, ["r2"], "the permission ended");
}

// ---- a git host's token ---------------------------------------------------------------------------------------------

fn token_call(desk: &Desk, id: &str, phone: &str, service: &str, host: &str, boxed_host: &str) -> Value {
    let nonce = format!("nonce-{id}");
    let boxed = box_to(
        desk,
        phone,
        &reins_proto::desktop::TokenToStore {
            v: 1,
            dir: TO_PHONE.to_owned(),
            nonce: nonce.clone(),
            created_at: reins_core::store::unix_now(),
            service: service.to_owned(),
            host: boxed_host.to_owned(),
            scopes: vec!["repo".to_owned(), "read:org".to_owned()],
            token: "ghp_token-secret".to_owned(),
        },
    );
    call(
        id,
        DESK,
        "desktop",
        "token_store",
        &json!({"service": service, "host": host, "scopes": ["repo", "read:org"], "sealed": boxed,
            "client_key": desk.public(), "nonce": nonce}),
    )
}

#[tokio::test]
async fn a_git_hosts_token_goes_to_the_phones_integration_with_one_approval() {
    let desk = desk().await;
    desk.pair().await;
    let phone = phone_key(&desk).await;
    desk.send(&[token_call(&desk, "t1", &phone, "github", "github.com", "github.com")]).await;
    let view = desk.core.approval_view("t1".to_owned()).await.unwrap();
    let t = view.token_store.clone().expect("the token");
    assert_eq!(
        (t.service.as_str(), t.host.as_str(), t.scopes.clone()),
        ("github", "github.com", words(&["repo", "read:org"]))
    );
    assert_eq!(t.check, reins_proto::desktop::value_check("ghp_token-secret"));
    assert!(view.no_standing && view.quick.is_none());
    desk.core.approve("t1".to_owned(), choice(&[], None)).await.unwrap();
    let ack: reins_proto::desktop::TokenStoreAck = open_from_phone(&desk, &phone, &desk.data("t1").await["sealed"]);
    assert_eq!((ack.service.as_str(), ack.account.as_str()), ("github", "octo"));
    assert!(!desk.everything_shown(&["t1"]).await.contains("ghp_token-secret"));

    // A box for another host, or a host the integration does not serve, is refused.
    desk.send(&[
        token_call(&desk, "t2", &phone, "github", "github.com", "gitlab.com"),
        token_call(&desk, "t3", &phone, "github", "github.example.com", "github.example.com"),
    ])
    .await;
    assert!(desk.error("t2").await.contains("another host"));
    assert!(desk.error("t3").await.contains("Only a token for"));
}

#[tokio::test]
async fn one_approval_saves_environments_ssh_keys_and_tokens_together() {
    let desk = desk().await;
    desk.pair().await;
    let phone = phone_key(&desk).await;
    let envs = vec![changes("shop", "dev", &[("KEY", "batch-secret-1")])];
    let ssh = vec![reins_proto::desktop::SshKeyToStore {
        name: "laptop key".to_owned(),
        private_key: include_str!("fixtures/ssh/k_ed").to_owned(),
    }];
    let tokens = vec![reins_proto::desktop::HostToken {
        host: "github.com".to_owned(),
        user: None,
        scopes: vec!["repo".to_owned()],
        token: "ghp_batch-secret-2".to_owned(),
    }];
    let check = reins_proto::desktop::batch_check(&envs, &ssh, &tokens);
    desk.send(&[store_all(&desk, "b1", &phone, envs, ssh, tokens)]).await;
    let view = desk.core.approval_view("b1".to_owned()).await.unwrap();
    let s = view.secrets_store.clone().expect("the batch");
    assert_eq!((s.check.as_str(), s.total), (check.as_str(), 3));
    assert_eq!((s.ssh[0].name.as_str(), s.ssh[0].fingerprint.starts_with("SHA256:")), ("laptop key", true));
    assert_eq!((s.tokens[0].service.as_str(), s.tokens[0].host.as_str()), ("github", "github.com"));
    desk.core.approve("b1".to_owned(), choice(&[], None)).await.unwrap();
    let ack: SecretsStoreAck = open_from_phone(&desk, &phone, &desk.data("b1").await["sealed"]);
    assert_eq!((ack.envs.len(), ack.envs[0].error.clone()), (1, None));
    assert_eq!(ack.ssh[0].error, None, "{:?}", ack.ssh);
    assert_eq!((ack.tokens[0].account.as_str(), ack.tokens[0].error.clone()), ("octo", None));
    let folders: Vec<String> = sent(&desk, "POST", "/api/folders").await.iter().map(|f| dec(&f["name"])).collect();
    assert_eq!(folders, ["Reins/shop", "Reins/ssh"]);
    let shown = desk.everything_shown(&["b1"]).await;
    assert!(!shown.contains("batch-secret") && !shown.contains("OPENSSH PRIVATE"), "no value in the clear");

    // The box must name the same keys and hosts as the call.
    let mut lying = store_all(
        &desk,
        "b2",
        &phone,
        vec![],
        vec![],
        vec![reins_proto::desktop::HostToken {
            host: "gitlab.com".to_owned(),
            user: None,
            scopes: vec![],
            token: "glpat".to_owned(),
        }],
    );
    lying["call"]["args"]["hosts"] = json!(["github.com"]);
    desk.send(&[lying]).await;
    assert!(desk.error("b2").await.contains("other environments, keys or hosts"));
}

#[tokio::test]
async fn only_marked_items_in_their_project_folder_are_environments() {
    let desk = desk().await;
    desk.pair().await;
    let phone = phone_key(&desk).await;
    // The item named acme/dev in Reins/store is store's dev; acme's dev is the one in Reins/acme.
    desk.send(&[release(&desk, "r1", &DEV)]).await;
    desk.core.approve("r1".to_owned(), choice(&[], None)).await.unwrap();
    let grant: SecretGrant = desk.open(&desk.data("r1").await["sealed"]);
    assert_eq!(grant.secrets[0], ("A".to_owned(), "alpha-secret".to_owned()));
    // An unmarked item named like an environment is neither released nor overwritten, nor adopted.
    let mine = Ask {
        env: "mine",
        ..DEV
    };
    desk.send(&[release(&desk, "r2", &mine), store(&desk, "s1", &phone, vec![changes("acme", "mine", &[("A", "x")])])])
        .await;
    assert!(desk.error("r2").await.contains("an item of your own"));
    assert!(desk.error("s1").await.contains("an item of your own"));
    assert!(sent(&desk, "PUT", "/api/ciphers/mine").await.is_empty());
    // Names with a slash are refused: they would split two ways.
    let slashed = Ask {
        env: "laptop/aws",
        ..DEV
    };
    desk.send(&[release(&desk, "r3", &slashed)]).await;
    assert!(!desk.error("r3").await.is_empty());
    // A new environment carries its name in a hidden field, not only in the item's name.
    desk.send(&[store(&desk, "s2", &phone, vec![changes("acme", "staging", &[("K", "v-staging")])])]).await;
    desk.core.approve("s2".to_owned(), choice(&[], None)).await.unwrap();
    let made = sent(&desk, "POST", "/api/ciphers").await;
    assert!(fields_of(&made[0]).contains(&("reins.env".to_owned(), "staging".to_owned())));
    // Legacy item/field references keep working, and reach an environment only by its own split.
    let legacy = |id: &str, secret: &str| {
        call(
            id,
            DESK,
            "vault",
            "secret_release",
            &json!({"secrets": [secret], "command": "x", "lease_secs": 600,
            "client_key": desk.public(), "nonce": format!("n-{id}")}),
        )
    };
    desk.send(&[legacy("l1", "acme/dev/A"), legacy("l2", "dev/reins.kind")]).await;
    desk.core.approve("l1".to_owned(), choice(&[], None)).await.unwrap();
    let old: SecretGrant = desk.open(&desk.data("l1").await["sealed"]);
    assert_eq!(old.secrets[0].1, "alpha-secret");
    assert!(!desk.error("l2").await.is_empty(), "never by another split or Reins's own field");
}
