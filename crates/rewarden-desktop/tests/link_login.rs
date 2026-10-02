//! `rewarden login` / `logout` and the session: a full OAuth login against a mock Rewarden server, driven by a scripted
//! browser, then token refresh with rotation and logging out.

mod link_mock;

use std::sync::Arc;

use link_mock::{Mock, browse, config, logged_in, repo};
use rewarden_desktop::auth::rewarden::RewardenAuthorizer;
use rewarden_desktop::auth::{Authorizer as _, Refusal};
use rewarden_desktop::config::Paths;
use rewarden_desktop::identity::Identity;
use rewarden_desktop::server::oauth::{logged_in_server, login, login_with_browser, logout};

fn session(paths: &Paths) -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(paths.session_file()).unwrap()).unwrap()
}

#[tokio::test]
async fn login_pairs_the_key_and_saves_a_private_session() {
    let mock = Mock::start().await;
    let app = logged_in(&mock).await;

    assert_eq!(logged_in_server(&app.paths), Some(mock.base.clone()));
    let s = session(&app.paths);
    assert_eq!(s["server"], mock.base);
    assert_eq!(s["client_id"], "client-1");
    assert_eq!(s["token_endpoint"], format!("{}/rewarden/oauth/token", mock.base));
    assert!(s["access_token"].as_str().unwrap().starts_with("at-"));
    assert!(s["refresh_token"].as_str().unwrap().starts_with("rt-"));
    assert!(s["access_expires_at"].as_i64().unwrap() > rewarden_desktop::now_unix() + 3000);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(app.paths.session_file()).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    let params = mock.with(|s| s.authorize_params.clone()).unwrap();
    assert_eq!(params["rewarden_client_key"], app.identity.public_key(), "the phone pins this app's key");
    assert_eq!(mock.with(|s| s.pinned_key.clone()), Some(app.identity.public_key()));
    assert_eq!(params["resource"], format!("{}/mcp", mock.base));
    assert!(params["redirect_uri"].starts_with("http://127.0.0.1:"));
    assert!(params["redirect_uri"].ends_with("/callback"));
    assert_ne!(params["redirect_uri"], "http://127.0.0.1/callback", "a real port, not the registered one");
    let names = mock.with(|s| s.client_names.clone());
    assert_eq!(names.len(), 1);
    assert!(names[0].starts_with("Rewarden desktop app on "));

    assert_eq!(logout(&app.paths), Ok(true));
    assert_eq!(logged_in_server(&app.paths), None);
    assert_eq!(logout(&app.paths), Ok(false));
    assert!(RewardenAuthorizer::new(&app.paths, Arc::clone(&app.identity), &config(5)).is_err());
}

#[tokio::test]
async fn the_callback_page_says_the_tab_can_be_closed() {
    let mock = Mock::start().await;
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::under(dir.path());
    let identity = Identity::generate();
    let (tx, rx) = tokio::sync::oneshot::channel();
    login_with_browser(&paths, &identity, &format!("{}/", mock.base), |link| {
        let link = link.to_owned();
        tokio::spawn(async move {
            let _sent = tx.send(browse(link).await);
        });
    })
    .await
    .unwrap();
    let (status, page) = rx.await.unwrap();
    assert_eq!(status, 200);
    assert!(page.contains("You can close this tab"), "{page}");
}

#[tokio::test]
async fn a_callback_with_another_state_is_ignored_and_a_refusal_ends_the_login() {
    let mock = Mock::start().await;
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::under(dir.path());
    let identity = Identity::generate();
    let err = login_with_browser(&paths, &identity, &mock.base, |link| {
        let link = url::Url::parse(link).unwrap();
        let q: std::collections::HashMap<String, String> = link.query_pairs().into_owned().collect();
        let callback = q["redirect_uri"].clone();
        let state = q["state"].clone();
        tokio::spawn(async move {
            let http = rewarden_desktop::http::client(None).unwrap();
            let forged = http.get(format!("{callback}?code=evil&state=forged")).send().await.unwrap();
            assert_eq!(forged.status(), 400);
            let denied = http
                .get(format!("{callback}?error=access_denied&error_description=Denied+on+the+phone&state={state}"))
                .send()
                .await
                .unwrap();
            assert_eq!(denied.status(), 200);
        });
    })
    .await
    .unwrap_err();
    assert!(err.contains("Denied on the phone"), "{err}");
    assert_eq!(logged_in_server(&paths), None);
}

#[tokio::test]
async fn only_https_or_local_servers_are_accepted() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::under(dir.path());
    let identity = Identity::generate();
    for bad in ["http://rewarden.example.com", "ftp://rewarden.example.com", "https://u:p@rewarden.example.com"] {
        let err = login(&paths, &identity, bad, false).await.unwrap_err();
        assert!(err.contains("https") || err.contains("user name"), "{bad}: {err}");
    }
    assert_eq!(logged_in_server(&paths), None);
}

#[tokio::test]
async fn an_expired_access_token_is_refreshed_and_the_refresh_token_rotates() {
    let mock = Mock::start().await;
    mock.with(|s| s.expires_in = 1);
    let app = logged_in(&mock).await;
    let before = session(&app.paths);

    let auth = RewardenAuthorizer::new(&app.paths, Arc::clone(&app.identity), &config(5)).unwrap();
    auth.read(&repo("octo/hello")).await.unwrap();
    assert_eq!(mock.with(|s| s.refreshes), 1);
    let after = session(&app.paths);
    assert_ne!(after["access_token"], before["access_token"]);
    assert_ne!(after["refresh_token"], before["refresh_token"], "the new refresh token is kept");

    // The new one is short-lived too, so the next call refreshes again with the rotated token.
    auth.read(&repo("octo/other")).await.unwrap();
    assert_eq!(mock.with(|s| s.refreshes), 2);
}

#[tokio::test]
async fn a_refused_refresh_logs_the_app_out() {
    let mock = Mock::start().await;
    mock.with(|s| s.expires_in = 1);
    let app = logged_in(&mock).await;
    mock.with(|s| s.refuse_refresh = true);

    let auth = RewardenAuthorizer::new(&app.paths, Arc::clone(&app.identity), &config(5)).unwrap();
    let Err(Refusal::Unavailable(m)) = auth.read(&repo("octo/hello")).await else {
        panic!("expected a refusal")
    };
    assert!(m.contains("rewarden login"), "{m}");
    assert_eq!(logged_in_server(&app.paths), None);
    assert!(mock.with(|s| s.calls.is_empty()), "nothing reached the phone");
}

#[tokio::test]
async fn a_rejected_token_is_refreshed_and_the_call_retried_once() {
    let mock = Mock::start().await;
    let app = logged_in(&mock).await;
    let before = session(&app.paths);
    mock.with(|s| s.reject_bearer_once = true);

    let auth = RewardenAuthorizer::new(&app.paths, Arc::clone(&app.identity), &config(5)).unwrap();
    let c = auth.read(&repo("octo/hello")).await.unwrap();
    assert!(c.token.starts_with("ghs-"));
    assert_eq!(mock.with(|s| s.refreshes), 1);
    assert_eq!(mock.with(|s| s.calls.len()), 1, "the rejected request never reached the phone");
    assert_ne!(session(&app.paths)["refresh_token"], before["refresh_token"]);
}
