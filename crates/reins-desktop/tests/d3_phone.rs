//! The phone's authorizer for the other git hosts: each host's own tools and account, nested GitLab paths, and reads
//! of the same path on two hosts kept apart.

mod link_mock;

use std::sync::Arc;

use link_mock::{Mock, config, logged_in, push_to, tag_push};
use reins_desktop::auth::Repo;
use reins_desktop::auth::reins::ReinsAuthorizer;
use reins_desktop::auth::{Authorizer as _, Refusal};
use reins_desktop::config::HostEntry;

const DIGEST: &str = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08";

fn on(host: &str, service: &str, path: &str) -> Repo {
    Repo::new(host, service, path)
}

#[tokio::test]
async fn each_host_asks_its_own_tools_with_its_own_account() {
    let mock = Mock::start().await;
    let app = logged_in(&mock).await;
    let mut c = config(5);
    c.git.hosts = vec![
        HostEntry {
            host: "gitlab.com".to_owned(),
            enabled: Some(true),
            account: Some("work".to_owned()),
            ..HostEntry::default()
        },
        HostEntry {
            host: "codeberg.org".to_owned(),
            enabled: Some(true),
            ..HostEntry::default()
        },
    ];
    let auth = ReinsAuthorizer::new(&app.paths, Arc::clone(&app.identity), &c).unwrap();

    let gitlab = on("gitlab.com", "gitlab", "group/sub/app");
    let cred = auth.read(&gitlab).await.unwrap();
    assert_eq!(cred.token.as_str(), "ghs-req-1");
    auth.read(&gitlab).await.unwrap();
    auth.push(&gitlab, &push_to("main"), DIGEST).await.unwrap();
    let codeberg = on("codeberg.org", "codeberg", "me/app");
    auth.read(&codeberg).await.unwrap();
    auth.push(&codeberg, &tag_push(), DIGEST).await.unwrap();
    // The same path on GitHub is another question, with GitHub's tool and account.
    auth.read(&on("github.com", "github", "me/app")).await.unwrap();

    let calls = mock.with(|s| {
        s.calls
            .iter()
            .map(|c| (c.tool.clone(), c.arguments["repo"].as_str().unwrap().to_owned(), c.account.clone()))
            .collect::<Vec<_>>()
    });
    let expected: Vec<(String, String, serde_json::Value)> = [
        ("gitlab_git_fetch", "group/sub/app", serde_json::json!("work")),
        ("gitlab_git_push", "group/sub/app", serde_json::json!("work")),
        ("codeberg_git_fetch", "me/app", serde_json::Value::Null),
        ("codeberg_git_tag_push", "me/app", serde_json::Value::Null),
        ("github_git_fetch", "me/app", serde_json::json!("octo")),
    ]
    .into_iter()
    .map(|(t, r, a)| (t.to_owned(), r.to_owned(), a))
    .collect();
    assert_eq!(calls, expected);
}

#[tokio::test]
async fn an_answer_for_another_repository_on_a_nested_path_is_refused() {
    let mock = Mock::start().await;
    let app = logged_in(&mock).await;
    let auth = ReinsAuthorizer::new(&app.paths, Arc::clone(&app.identity), &config(5)).unwrap();
    mock.plan(&[link_mock::Step::Tamper(|g| g.repo = "group/app".to_owned())]);
    let r = auth.read(&on("gitlab.com", "gitlab", "group/sub/app")).await;
    assert!(matches!(&r, Err(Refusal::Unavailable(m)) if m.contains("group/app")), "{r:?}");
    mock.plan(&[link_mock::Step::Tamper(|g| g.access = "write".to_owned())]);
    let r = auth.read(&on("gitlab.com", "gitlab", "group/sub/app")).await;
    assert!(matches!(&r, Err(Refusal::Unavailable(m)) if m.contains("write access")), "{r:?}");
}
