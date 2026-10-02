//! Git for GitLab and Codeberg through the proxy: real git, rewritten by `git setup` for every enabled host, against a
//! `git http-backend` upstream, decided by an authorizer that records which phone tool each request would ask for.

mod proxy_support;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use proxy_support::{Home, Proxy, TOKEN, Upstream, config, raw_http, token_basic};
use rewarden_desktop::auth::prompt::NoPrompter;
use rewarden_desktop::auth::{Authorizer, Credential, Refusal, Repo};
use rewarden_desktop::config::{Config, HostEntry};
use rewarden_desktop::daemon::Options;
use rewarden_desktop::setup::{Git, Scope};
use rewarden_proto::desktop::{PushSummary, fetch_tool_for};

/// One question as the phone would get it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Asked {
    tool: String,
    service: String,
    host: String,
    path: String,
}

#[derive(Default)]
struct Recorder {
    asked: Mutex<Vec<Asked>>,
    summaries: Mutex<Vec<PushSummary>>,
    delay: Mutex<Duration>,
}

impl Recorder {
    fn record(&self, tool: String, repo: &Repo) {
        self.asked.lock().unwrap().push(Asked {
            tool,
            service: repo.service.clone(),
            host: repo.host.clone(),
            path: repo.path.clone(),
        });
    }

    fn tools(&self) -> Vec<String> {
        let mut tools: Vec<String> = self.asked.lock().unwrap().iter().map(|a| a.tool.clone()).collect();
        tools.dedup();
        tools
    }

    fn take(&self) -> Vec<Asked> {
        std::mem::take(&mut *self.asked.lock().unwrap())
    }
}

fn credential() -> Credential {
    Credential {
        username: "x-access-token".to_owned(),
        token: zeroize::Zeroizing::new(TOKEN.to_owned()),
        expires_at: rewarden_desktop::now_unix() + 600,
    }
}

#[async_trait::async_trait]
impl Authorizer for Recorder {
    async fn read(&self, repo: &Repo) -> Result<Credential, Refusal> {
        self.record(fetch_tool_for(&repo.service), repo);
        let delay = *self.delay.lock().unwrap();
        tokio::time::sleep(delay).await;
        Ok(credential())
    }

    async fn push(&self, repo: &Repo, summary: &PushSummary, _digest: &str) -> Result<Credential, Refusal> {
        self.record(summary.tool_for(&repo.service), repo);
        self.summaries.lock().unwrap().push(summary.clone());
        Ok(credential())
    }

    fn describe(&self) -> String {
        "a recorder".to_owned()
    }
}

fn entry(host: &str, up: &Upstream) -> HostEntry {
    HostEntry {
        host: host.to_owned(),
        enabled: Some(true),
        git_base: Some(up.url()),
        api_base: Some(format!("{}/api", up.url())),
        ..HostEntry::default()
    }
}

/// The proxy with GitLab and Codeberg enabled (both served by `up`), and a `HOME` whose git config was written by
/// `git setup` for every enabled host.
async fn start(up: &Upstream, auth: &Arc<Recorder>) -> (Proxy, Home) {
    let mut c = config(up);
    c.git.hosts = vec![entry("gitlab.com", up), entry("codeberg.org", up)];
    let proxy = Proxy::start(
        c.clone(),
        Options {
            authorizer: Some(Arc::<Recorder>::clone(auth)),
            prompter: Arc::new(NoPrompter),
            harden: false,
        },
    )
    .await;
    let home = Home::new(proxy.addr());
    let listening = Config {
        listen: proxy.addr(),
        ..c
    };
    let h = home.dir.path();
    let git = Git::default()
        .env("HOME", h)
        .env("XDG_CONFIG_HOME", h.join(".config"))
        .env("GIT_CONFIG_GLOBAL", h.join(".gitconfig"))
        .env("GIT_CONFIG_NOSYSTEM", "1");
    let (added, _) = git.setup_hosts(&Scope::Global, &listening).unwrap();
    // Home already sends https://github.com/ to the proxy: the two other GitHub forms, three each for the others.
    assert_eq!(added.len(), 8, "{added:?}");
    (proxy, home)
}

fn asked(tool: &str, service: &str, host: &str, path: &str) -> Asked {
    Asked {
        tool: tool.to_owned(),
        service: service.to_owned(),
        host: host.to_owned(),
        path: path.to_owned(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_gitlab_repository_in_nested_groups_is_cloned_and_pushed() {
    let up = Upstream::start().await;
    up.create("group/sub/app", true);
    let auth = Arc::new(Recorder::default());
    let (_proxy, home) = start(&up, &auth).await;

    let clone = home.git_ok("", &["clone", "git@gitlab.com:group/sub/app.git", "app"]).await;
    assert!(home.path("app/README.md").exists());
    assert!(!clone.all().contains(TOKEN));
    let reads = auth.take();
    assert!(!reads.is_empty());
    assert!(
        reads.iter().all(|a| *a == asked("gitlab_git_fetch", "gitlab", "gitlab.com", "group/sub/app")),
        "{reads:#?}"
    );
    let authed = format!("Basic {}", token_basic());
    let seen = up.seen();
    assert!(seen.iter().all(|s| s.path.starts_with("/group/sub/app.git/")), "{seen:#?}");
    assert!(seen.iter().filter(|s| s.status == 200).all(|s| s.authorization.as_deref() == Some(&authed)));

    // A push is asked as a GitLab push, described from the pack alone (no GitHub API), and lands upstream.
    let new = home.commit("app", "a.txt", b"hello\n", "Add a").await;
    let push = home.git_ok("app", &["push", "origin", "main"]).await;
    assert!(!push.all().contains(TOKEN));
    assert_eq!(up.head("group/sub/app", "main").as_deref(), Some(new.as_str()));
    assert_eq!(auth.tools(), ["gitlab_git_fetch", "gitlab_git_push"]);
    let pushed = auth.take().pop().unwrap();
    assert_eq!(pushed, asked("gitlab_git_push", "gitlab", "gitlab.com", "group/sub/app"));
    let summary = auth.summaries.lock().unwrap().pop().unwrap();
    assert_eq!(summary.updates[0].name, "refs/heads/main");
    assert_eq!(summary.updates[0].fast_forward, Some(true), "the pack itself shows the old tip as the parent");
    assert!(summary.updates[0].commits.iter().any(|c| c.subject == "Add a"), "{summary:#?}");
    assert!(up.seen().iter().all(|s| !s.path.starts_with("/api")), "only GitHub's API is used");

    // Tags alone are a tag push.
    home.git_ok("app", &["tag", "v1"]).await;
    home.git_ok("app", &["push", "origin", "v1"]).await;
    assert_eq!(auth.tools().last().map(String::as_str), Some("gitlab_git_tag_push"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_codeberg_repository_is_cloned_and_pushed_apart_from_the_same_path_on_github() {
    let up = Upstream::start().await;
    up.create("me/app", true);
    let auth = Arc::new(Recorder::default());
    let (_proxy, home) = start(&up, &auth).await;

    home.git_ok("", &["clone", "-q", "https://codeberg.org/me/app", "cb"]).await;
    let reads = auth.take();
    assert!(!reads.is_empty());
    assert!(
        reads.iter().all(|a| *a == asked("codeberg_git_fetch", "codeberg", "codeberg.org", "me/app")),
        "{reads:#?}"
    );
    let new = home.commit("cb", "b.txt", b"b\n", "Add b").await;
    home.git_ok("cb", &["push", "-q", "origin", "main"]).await;
    assert_eq!(up.head("me/app", "main").as_deref(), Some(new.as_str()));
    assert_eq!(auth.take().pop().unwrap(), asked("codeberg_git_push", "codeberg", "codeberg.org", "me/app"));

    // The same owner/name on GitHub is another repository: asked again, as GitHub.
    home.git_ok("", &["clone", "-q", "https://github.com/me/app", "gh"]).await;
    let reads = auth.take();
    assert!(!reads.is_empty());
    assert!(reads.iter().all(|a| *a == asked("github_git_fetch", "github", "github.com", "me/app")), "{reads:#?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn hosts_that_are_not_enabled_are_not_served() {
    let up = Upstream::start().await;
    up.create("me/app", false);
    let auth = Arc::new(Recorder::default());
    let (proxy, _home) = start(&up, &auth).await;
    let addr = proxy.addr();
    for (path, status) in [
        ("/codeberg.org/me/app.git/info/refs?service=git-upload-pack", "200"),
        ("/bitbucket.org/me/app.git/info/refs?service=git-upload-pack", "404"),
        ("/codeberg.org/g/me/app.git/info/refs?service=git-upload-pack", "404"),
    ] {
        let answer = raw_http(
            addr,
            &format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n", addr.port()),
        )
        .await;
        assert!(answer.starts_with(&format!("HTTP/1.1 {status}")), "{path}\n{answer}");
    }
}

#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_waiting_notice_names_the_host() {
    let up = Upstream::start().await;
    up.create("group/sub/app", true);
    let auth = Arc::new(Recorder::default());
    *auth.delay.lock().unwrap() = Duration::from_secs(2);
    let (_proxy, home) = start(&up, &auth).await;
    let clone = home.git_ok("", &["clone", "-q", "https://gitlab.com/group/sub/app", "app"]).await;
    assert!(clone.stderr.contains("rewarden: waiting for approval: read gitlab.com/group/sub/app…"), "{}", clone.all());
}
