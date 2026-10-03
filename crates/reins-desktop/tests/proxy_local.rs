//! The daemon in local mode: the policy from the config decides, the token comes from a file, and "ask" goes to a
//! scripted desktop prompt or to the control API.

mod proxy_support;

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use proxy_support::{Home, Proxy, TOKEN, Upstream, config, logs, token_basic};
use reins_desktop::auth::prompt::{PendingItem, Prompter};
use reins_desktop::config::{Mode, PolicyConfig};
use reins_desktop::control::Client;
use reins_desktop::daemon::Options;

/// Answers prompts from a script (`None`: the person does not answer) and records them.
#[derive(Default)]
struct Prompt {
    answers: Mutex<VecDeque<Option<bool>>>,
    asked: Mutex<Vec<PendingItem>>,
}

#[async_trait::async_trait]
impl Prompter for Prompt {
    async fn ask(&self, item: &PendingItem) -> Option<bool> {
        self.asked.lock().unwrap().push(item.clone());
        self.answers.lock().unwrap().pop_front().flatten()
    }
}

impl Prompt {
    fn script(&self, answers: &[Option<bool>]) {
        self.answers.lock().unwrap().extend(answers.iter().copied());
    }

    fn whats(&self) -> Vec<String> {
        self.asked.lock().unwrap().iter().map(|i| i.what.clone()).collect()
    }
}

const POLICY: &str = r#"
read = "allow"
push = "ask"
risky = "deny"

[[rules]]
repo = "me/priv"
branch = "feature/**"
push = "allow"

[[rules]]
repo = "me/secret"
read = "deny"

[[rules]]
repo = "me/asked"
read = "ask"
"#;

async fn local(up: &Upstream, prompt: Arc<Prompt>) -> (Proxy, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("token"), format!("{TOKEN}\n")).unwrap();
    let mut c = config(up);
    c.mode = Mode::Local;
    c.approval_timeout_secs = 5;
    c.github.token = format!("file:{}", dir.path().join("token").display());
    c.policy = toml::from_str::<PolicyConfig>(POLICY).unwrap();
    let proxy = Proxy::start(
        c,
        Options {
            authorizer: None,
            prompter: prompt,
            harden: false,
        },
    )
    .await;
    (proxy, dir)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reads_follow_the_policy() {
    let up = Upstream::start().await;
    for repo in ["me/priv", "me/secret", "me/asked"] {
        up.create(repo, true);
    }
    let prompt = Arc::new(Prompt::default());
    let (proxy, _token) = local(&up, Arc::clone(&prompt)).await;
    let home = Home::new(proxy.addr());
    let run = home.git_ok("", &["clone", "https://github.com/me/priv", "priv"]).await;
    assert!(!run.all().contains(TOKEN));
    let run = home.git("", &["clone", "https://github.com/me/secret", "secret"]).await;
    assert!(!run.ok);
    assert!(run.stderr.contains("remote: The local policy does not allow reading me/secret."), "{}", run.stderr);
    assert!(prompt.whats().is_empty());
    prompt.script(&[Some(true)]);
    home.git_ok("", &["clone", "https://github.com/me/asked", "asked"]).await;
    assert_eq!(prompt.whats(), vec!["Read me/asked"]);
    assert!(!logs().contains(TOKEN) && !logs().contains(&token_basic()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pushes_follow_the_policy_and_the_prompt() {
    let up = Upstream::start().await;
    up.create("me/priv", true);
    let prompt = Arc::new(Prompt::default());
    let (proxy, _token) = local(&up, Arc::clone(&prompt)).await;
    let home = Home::new(proxy.addr());
    home.git_ok("", &["clone", "-q", "https://github.com/me/priv", "priv"]).await;

    // A rule allows feature branches: no prompt.
    let feature = home.commit("priv", "f.txt", b"f\n", "Feature").await;
    home.git_ok("priv", &["push", "origin", "HEAD:refs/heads/feature/x"]).await;
    assert_eq!(up.head("me/priv", "feature/x").as_deref(), Some(feature.as_str()));
    assert!(prompt.whats().is_empty());

    // Other branches are asked: approved, then denied.
    prompt.script(&[Some(true), Some(false)]);
    home.git_ok("priv", &["push", "origin", "HEAD:refs/heads/topic"]).await;
    assert_eq!(up.head("me/priv", "topic").as_deref(), Some(feature.as_str()));
    let run = home.git("priv", &["push", "origin", "HEAD:refs/heads/nope"]).await;
    assert!(
        run.stderr.contains("! [remote rejected] HEAD -> nope (Push to me/priv: denied on this computer.)"),
        "{}",
        run.stderr
    );
    assert_eq!(up.head("me/priv", "nope"), None);
    let asked = prompt.asked.lock().unwrap().clone();
    assert_eq!(asked.len(), 2);
    assert_eq!(asked[0].what, "Push to me/priv");
    assert!(asked[0].lines[0].starts_with("Create branch topic"), "{:?}", asked[0].lines);

    // Rewriting history is risky, and risky pushes are denied here even on an allowed branch.
    home.git_ok("priv", &["commit", "-q", "--amend", "-m", "Rewritten"]).await;
    let run = home.git("priv", &["push", "-f", "origin", "HEAD:refs/heads/feature/x"]).await;
    assert!(
        run.stderr.contains(
            "! [remote rejected] HEAD -> feature/x (The local policy does not allow this kind of push to me/priv.)"
        ),
        "{}",
        run.stderr
    );
    assert_eq!(up.head("me/priv", "feature/x").as_deref(), Some(feature.as_str()));
    assert_eq!(prompt.asked.lock().unwrap().len(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_unanswered_push_waits_and_a_later_approval_counts_for_the_retry() {
    let up = Upstream::start().await;
    up.create("me/priv", true);
    let prompt = Arc::new(Prompt::default());
    let (proxy, _token) = local(&up, Arc::clone(&prompt)).await;
    let home = Home::new(proxy.addr());
    home.git_ok("", &["clone", "-q", "https://github.com/me/priv", "priv"]).await;
    let new = home.commit("priv", "w.txt", b"w\n", "Wait for it").await;
    let run = home.git("priv", &["push", "origin", "HEAD:refs/heads/later"]).await;
    assert!(!run.ok);
    assert!(
        run.stderr.contains("! [remote rejected] HEAD -> later (Waiting for approval on this computer"),
        "{}",
        run.stderr
    );
    assert!(run.stderr.contains("remote: Waiting for approval on this computer: Push to me/priv."), "{}", run.stderr);
    assert_eq!(up.head("me/priv", "later"), None);

    let client = Client::new(&proxy.paths, proxy.addr()).unwrap();
    let pending = client.pending().await.unwrap();
    assert_eq!(pending.len(), 1);
    assert!(run.stderr.contains(&format!("reins approve {}", pending[0].id)));
    assert_eq!(client.status().await.unwrap().pending, 1);
    client.answer(&pending[0].id, true).await.unwrap();
    assert!(client.pending().await.unwrap().is_empty());

    home.git_ok("priv", &["push", "origin", "HEAD:refs/heads/later"]).await;
    assert_eq!(up.head("me/priv", "later").as_deref(), Some(new.as_str()));
    assert_eq!(prompt.asked.lock().unwrap().len(), 1, "the retry is not asked again");
}
