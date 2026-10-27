//! Organizations (Decisions v2 · O1) end to end: the real server with organizations on, a fake WorkOS (sign-in,
//! organizations, directories, Directory Sync events) and real phone cores. A pilot org is created in the admin
//! console, verifies its domain, binds its SSO and directory, provisions a member through SCIM, the member signs in
//! with "Add work account" and enrolls the phone, and a SCIM delete removes them within the minute.

use std::time::{Duration, Instant};

use reins_e2e::org::{
    Browser, WEBHOOK_SECRET, add_work_account, between, csrf, db, devices_of, dir_user, enrollment, member_state,
    user_exists, wait_for, webhook,
};
use reins_e2e::workos::{FakeWorkos, User};
use reins_e2e::{Phone, Server};
use serde_json::json;

#[tokio::test(flavor = "multi_thread")]
async fn a_pilot_org_is_provisioned_and_deprovisioned_end_to_end() {
    let workos = FakeWorkos::start().await;
    workos.add_organization("org_ACME", &["acme.com"]);
    workos.add_directory("directory_ACME", "org_ACME");
    let scratch = tempfile::tempdir().unwrap();
    let txt = scratch.path().join("txt");
    std::fs::write(&txt, "").unwrap();
    let mut env = workos.server_env();
    env.extend(
        [
            ("REINS_ORGS_ENABLED", "true".to_owned()),
            // Only a webhook can make the directory sync fast: the timing below is the webhook path's.
            ("REINS_WORKOS_DSYNC_SECS", "3600".to_owned()),
            ("REINS_WORKOS_WEBHOOK_SECRET", WEBHOOK_SECRET.to_owned()),
            ("REINS_TEST_DNS_TXT_FILE", txt.to_string_lossy().into_owned()),
        ]
        .map(|(k, v)| (k.to_owned(), v)),
    );
    let server = Server::start_with_env(5, 3, &env).await;

    // The founder's account (dana@acme.com) exists: she signed in on her phone.
    let dana = User {
        id: "user_dana".to_owned(),
        email: "dana@acme.com".to_owned(),
    };
    workos.sign_in_as(&dana);
    let dana_phone = Phone::signed_out(|_| {}).await;
    dana_phone.sso_sign_in(&server.base).await.unwrap();

    // The console: sign in, create the org.
    let mut browser = Browser::new();
    let home = browser.get(&server.url("/reins/org/login")).await;
    assert_eq!(home.status, 200, "{}", home.body);
    assert!(home.body.contains("New organization"), "{}", home.body);
    let created = browser
        .post(&server.url("/reins/org/new"), &[("csrf", &csrf(&home)), ("name", "Acme"), ("tier", "business")])
        .await;
    let org = created.url.rsplit('/').next().unwrap().to_owned();
    assert!(created.body.contains("_reins-challenge.acme.com"), "{}", created.body);

    // The domain, through DNS.
    let record = between(&created.body, "_reins-challenge.acme.com<br>", "</div>").to_owned();
    std::fs::write(&txt, format!("_reins-challenge.acme.com={record}\n")).unwrap();
    let verified = browser
        .post(
            &server.url(&format!("/reins/org/{org}/domains/verify")),
            &[("csrf", &csrf(&created)), ("domain", "acme.com")],
        )
        .await;
    assert!(verified.body.contains("Verified"), "{}", verified.body);

    // An event of the directory before it is bound is dropped (nothing knows whose it is)…
    workos.emit("dsync.user.created", dir_user("directory_user_kim", "kim@acme.com", "active"));
    webhook(&server, &json!({"id": "wh_0", "event": "dsync.user.created"})).await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(member_state(&server, "kim@acme.com"), None);

    // SSO and the directory, bound only with WorkOS's word.
    let refused = browser
        .post(
            &server.url(&format!("/reins/org/{org}/identity")),
            &[
                ("csrf", &csrf(&verified)),
                ("identity", "sso"),
                ("workos_org_id", "org_OTHER"),
                ("directory_id", ""),
                ("github_org", ""),
            ],
        )
        .await;
    assert!(refused.body.contains("WorkOS didn"), "{}", refused.body);
    let bound = browser
        .post(
            &server.url(&format!("/reins/org/{org}/identity")),
            &[
                ("csrf", &csrf(&refused)),
                ("identity", "sso"),
                ("workos_org_id", "org_ACME"),
                ("directory_id", "directory_ACME"),
                ("github_org", ""),
            ],
        )
        .await;
    assert!(bound.body.contains("Saved"), "{}", bound.body);
    assert!(bound.body.contains("org_ACME") && bound.body.contains("directory_ACME"), "{}", bound.body);

    // …and the binding lists the directory: Kim arrives all the same.
    wait_for("backfill", Duration::from_secs(20), || {
        member_state(&server, "kim@acme.com").as_deref() == Some("active")
    })
    .await;

    // SCIM: the directory adds Lee; the webhook wakes the sync.
    workos.emit("dsync.user.created", dir_user("directory_user_lee", "lee@acme.com", "active"));
    webhook(&server, &json!({"id": "wh_1", "event": "dsync.user.created"})).await;
    wait_for("provisioning", Duration::from_secs(20), || {
        member_state(&server, "lee@acme.com").as_deref() == Some("active")
    })
    .await;

    // Lee adds the work account on a phone: straight to Acme's SSO.
    let lee = User {
        id: "user_lee".to_owned(),
        email: "lee@acme.com".to_owned(),
    };
    workos.sign_in_as(&lee);
    let lee_phone = Phone::signed_out(|_| {}).await;
    add_work_account(&lee_phone, &server, "acme.com").await.unwrap();
    lee_phone.core.register_device(None).await.unwrap();
    let membership = lee_phone.core.org_membership().await.unwrap().expect("a work account");
    assert_eq!((membership.name.as_str(), membership.enrolled), ("Acme", false));
    let challenge = lee_phone.core.org_challenge().await.unwrap();
    lee_phone.core.org_enroll(challenge.clone(), enrollment(&challenge)).await.unwrap();
    assert!(lee_phone.core.org_membership().await.unwrap().unwrap().enrolled);
    // A challenge is good once.
    assert!(lee_phone.core.org_enroll(challenge.clone(), enrollment(&challenge)).await.is_err());

    // A Personal account sees no org (the same answer as a server without orgs).
    workos.sign_in_as(&User {
        id: "user_bob".to_owned(),
        email: "bob@example.com".to_owned(),
    });
    let bob = Phone::signed_out(|_| {}).await;
    bob.sso_sign_in(&server.base).await.unwrap();
    assert_eq!(bob.core.org_membership().await.unwrap(), None);

    // A forged webhook body names Dana: nobody is removed (only events read from WorkOS count).
    webhook(
        &server,
        &json!({"id": "wh_forged", "event": "dsync.user.deleted",
                             "data": dir_user("directory_user_dana", "dana@acme.com", "inactive")}),
    )
    .await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(member_state(&server, "dana@acme.com").as_deref(), Some("active"));

    // The directory deletes Lee: everything of Lee's goes, well within the minute.
    assert!(devices_of(&server, "lee@acme.com") > 0);
    workos.emit("dsync.user.deleted", dir_user("directory_user_lee", "lee@acme.com", "inactive"));
    let started = Instant::now();
    webhook(&server, &json!({"id": "wh_2", "event": "dsync.user.deleted"})).await;
    // Deprovisioned at once; the account the directory deleted is then erased by the same sync pass, so a poll may
    // only ever see it gone.
    let took = wait_for("deprovisioning", Duration::from_secs(60), || {
        !user_exists(&server, "lee@acme.com")
            || devices_of(&server, "lee@acme.com") == 0
                && member_state(&server, "lee@acme.com").as_deref() == Some("deprovisioned_by_directory")
    })
    .await;
    wait_for("erasure", Duration::from_secs(60).saturating_sub(took), || !user_exists(&server, "lee@acme.com")).await;
    assert!(started.elapsed() < Duration::from_secs(60), "deprovisioning took {took:?}");
    assert!(lee_phone.core.org_membership().await.is_err(), "the phone's session is gone");

    // The console's events show it, and the chain is intact.
    let events = browser.get(&server.url(&format!("/reins/org/{org}/events"))).await;
    for kind in [
        "org.created",
        "domain.verified",
        "change.applied",
        "member.provisioned",
        "device.enrolled",
        "member.deprovisioned",
    ] {
        assert!(events.body.contains(kind), "{kind} missing:\n{}", events.body);
    }
    assert!(events.body.contains("Chain intact"), "{}", events.body);
    // Dana and Bob were never touched.
    assert_eq!(member_state(&server, "dana@acme.com").as_deref(), Some("active"));
    assert_eq!(member_state(&server, "bob@example.com"), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_console_needs_its_session_and_csrf() {
    let workos = FakeWorkos::start().await;
    let mut env = workos.server_env();
    env.push(("REINS_ORGS_ENABLED".to_owned(), "true".to_owned()));
    let server = Server::start_with_env(5, 3, &env).await;
    let mut anonymous = Browser::new();
    let page = anonymous.get(&server.url("/reins/org")).await;
    assert!(page.body.contains("Sign in"), "{}", page.body);
    let page = anonymous.get(&server.url("/reins/org/00000000-0000-0000-0000-000000000000")).await;
    assert!(page.body.contains("Not found"));

    let dana = User {
        id: "user_dana2".to_owned(),
        email: "dana@acme.io".to_owned(),
    };
    workos.sign_in_as(&dana);
    Phone::signed_out(|_| {}).await.sso_sign_in(&server.base).await.unwrap();
    let mut browser = Browser::new();
    let home = browser.get(&server.url("/reins/org/login")).await;
    // Without the CSRF token, nothing is created.
    let page =
        browser.post(&server.url("/reins/org/new"), &[("csrf", "forged"), ("name", "Acme"), ("tier", "team")]).await;
    assert!(!page.body.contains("_reins-challenge"), "{}", page.body);
    let count: i64 = db(&server).query_row("SELECT COUNT(*) FROM reins_orgs", [], |r| r.get(0)).unwrap();
    assert_eq!(count, 0);
    // With it, it is.
    browser.post(&server.url("/reins/org/new"), &[("csrf", &csrf(&home)), ("name", "Acme"), ("tier", "team")]).await;
    let count: i64 = db(&server).query_row("SELECT COUNT(*) FROM reins_orgs", [], |r| r.get(0)).unwrap();
    assert_eq!(count, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_work_account_password_is_refused_only_after_a_correct_password() {
    let server = Server::start_with_env(5, 3, &[("REINS_ORGS_ENABLED".to_owned(), "true".to_owned())]).await;
    let phone = Phone::create_account(&server.base, "kim@acme.dev").await;
    phone.core.logout().await.unwrap();
    // Make the account a member of an org, as the console or SCIM would.
    {
        let c = db(&server);
        let user: String =
            c.query_row("SELECT uuid FROM users WHERE email = 'kim@acme.dev'", [], |r| r.get(0)).unwrap();
        c.execute_batch(&format!(
            "INSERT INTO organizations (uuid, name, billing_email) VALUES ('org-1', 'Acme', 'kim@acme.dev');
             INSERT INTO reins_orgs (org_uuid, tier, identity, two_person, created_by, created_at, updated_at)
               VALUES ('org-1', 'business', 'none', 1, '{user}', 1, 1);
             INSERT INTO users_organizations (uuid, user_uuid, org_uuid, access_all, akey, status, atype)
               VALUES ('m-1', '{user}', 'org-1', 0, '', 1, 2);
             INSERT INTO reins_org_members (org_uuid, user_uuid, membership_uuid, state, changed_at)
               VALUES ('org-1', '{user}', 'm-1', 'active', 1);"
        ))
        .unwrap();
    }
    let wrong = phone.core.login(server.base.clone(), "kim@acme.dev".to_owned(), "wrong".to_owned(), None).await;
    let wrong = format!("{:?}", wrong.unwrap_err());
    assert!(!wrong.contains("work account"), "a wrong password must not tell work accounts apart: {wrong}");
    let right =
        phone.core.login(server.base.clone(), "kim@acme.dev".to_owned(), reins_e2e::PASSWORD.to_owned(), None).await;
    let right = format!("{:?}", right.unwrap_err());
    assert!(right.contains("work account"), "{right}");
}
