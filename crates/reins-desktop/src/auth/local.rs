//! Without a phone: the local policy decides, asking on the desktop when it says "ask", with each host's token from an
//! environment variable or a file (for GitHub also the GitHub CLI).
//!
//! Limits: another process of the same user can approve through the control API (it can read `control.token`) and
//! can read the token source itself. Local mode guards against an agent's mistakes; against a hostile agent use the
//! phone, and run the daemon as its own OS user.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use reins_proto::desktop::{PushSummary, RefChange};
use zeroize::Zeroizing;

use super::policy::{decide_push, decide_read, is_risky};
use super::prompt::{Asked, Pending, Prompter, spawn_prompt};
use super::{Authorizer, Credential, Refusal, Repo};
use crate::config::{Config, PolicyConfig, Rule};

/// How long a token from `gh auth token` is reused.
const GH_CACHE: Duration = Duration::from_secs(600);
/// How long a local credential is used.
const CREDENTIAL_SECS: i64 = 600;

/// Where a host's token comes from (`token` of the host in the config).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TokenSource {
    /// `gh auth token` for the host.
    Gh {
        host: String,
    },
    Env(String),
    File(std::path::PathBuf),
}

impl TokenSource {
    pub fn parse(spec: &str, host: &str) -> Result<Self, String> {
        if spec == "gh" {
            Ok(Self::Gh {
                host: host.to_owned(),
            })
        } else if let Some(name) = spec.strip_prefix("env:").filter(|n| !n.is_empty()) {
            Ok(Self::Env(name.to_owned()))
        } else if let Some(path) = spec.strip_prefix("file:").filter(|p| !p.is_empty()) {
            let path = match (path.strip_prefix("~/"), crate::config::home_dir()) {
                (Some(rest), Ok(home)) => home.join(rest),
                _ => std::path::PathBuf::from(path),
            };
            Ok(Self::File(path))
        } else {
            Err(format!("the token of {host} must be `gh` (GitHub only), `env:NAME` or `file:PATH`, not `{spec}`"))
        }
    }

    async fn fetch(&self) -> Result<Zeroizing<String>, String> {
        let token = match self {
            Self::Gh {
                host,
            } => {
                let mut gh = tokio::process::Command::new("gh");
                // The background daemon has no console; without this every token fetch would flash a window.
                #[cfg(windows)]
                crate::win::hidden_async(&mut gh);
                let run = gh
                    .args(["auth", "token", "--hostname", host])
                    .stdin(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .kill_on_drop(true)
                    .output();
                let out = tokio::time::timeout(Duration::from_secs(15), run)
                    .await
                    .map_err(|_| "`gh auth token` did not answer".to_owned())?
                    .map_err(|e| format!("cannot run the GitHub CLI (`gh`): {e}"))?;
                if !out.status.success() {
                    return Err("the GitHub CLI is not logged in (`gh auth login`)".to_owned());
                }
                first_line(&Zeroizing::new(out.stdout))
            }
            Self::Env(name) => {
                let value = std::env::var(name).map_err(|_| format!("the environment variable {name} is not set"))?;
                first_line(&Zeroizing::new(value.into_bytes()))
            }
            Self::File(path) => {
                let bytes = tokio::fs::read(path).await.map_err(|e| format!("{}: {e}", path.display()))?;
                first_line(&Zeroizing::new(bytes))
            }
        };
        token.filter(|t| !t.is_empty()).ok_or_else(|| "the token is empty".to_owned())
    }
}

fn short(oid: &str) -> &str {
    oid.get(..7).unwrap_or(oid)
}

fn first_line(bytes: &[u8]) -> Option<Zeroizing<String>> {
    let text = std::str::from_utf8(bytes).ok()?;
    Some(Zeroizing::new(text.lines().next().unwrap_or("").trim().to_owned()))
}

/// The lines a person reads before allowing a push.
#[must_use]
pub fn describe_push(summary: &PushSummary) -> Vec<String> {
    let mut lines = Vec::new();
    for u in &summary.updates {
        let label = u.branch().or_else(|| u.tag()).unwrap_or(&u.name);
        let commits = match u.commit_count {
            1 => "1 commit".to_owned(),
            n => format!("{n} commits"),
        };
        lines.push(match (u.change, u.tag().is_some()) {
            (RefChange::Delete, true) => format!("Delete tag {label}"),
            (RefChange::Delete, false) => format!("Delete branch {label}"),
            (RefChange::Create, true) => format!("Tag {label} -> {}", short(&u.new)),
            (RefChange::Create, false) => format!("Create branch {label} with {commits}"),
            (RefChange::Update, true) => format!("Move tag {label} -> {}", short(&u.new)),
            (RefChange::Update, false) => match u.fast_forward {
                Some(true) => format!("Push {commits} to {label}"),
                Some(false) => format!("Force push to {label}: rewrites history"),
                None => format!("Push to {label} (could not check history; treated like a force push)"),
            },
        });
        for c in u.commits.iter().take(5) {
            lines.push(format!("  {} {}", short(&c.sha), c.subject));
        }
        if let (Some(a), Some(d)) = (u.additions, u.deletions) {
            lines.push(format!("  +{a} -{d} in {} files", u.files_changed));
        }
    }
    lines.extend(summary.notes.iter().cloned());
    lines
}

/// A host's token and the user name sent with it.
struct HostToken {
    label: &'static str,
    /// `None`: no token configured for the host.
    source: Option<TokenSource>,
    username: String,
}

/// The local authorizer.
pub struct LocalAuthorizer {
    policy: PolicyConfig,
    /// By host.
    tokens: HashMap<String, HostToken>,
    timeout: Duration,
    pending: Arc<Pending>,
    prompter: Arc<dyn Prompter>,
    /// `gh auth token` answers by host.
    cached: Mutex<HashMap<String, (Zeroizing<String>, Instant)>>,
}

impl LocalAuthorizer {
    pub fn new(config: &Config, pending: Arc<Pending>, prompter: Arc<dyn Prompter>) -> Result<Self, String> {
        let mut tokens = HashMap::new();
        for h in config.git_hosts()? {
            let source = match h.token.as_deref() {
                Some("gh") if h.service != "github" => {
                    return Err(format!("the token of {}: `gh` gives GitHub tokens only", h.host));
                }
                Some(spec) => Some(TokenSource::parse(spec, &h.host)?),
                None => None,
            };
            tokens.insert(
                h.host.clone(),
                HostToken {
                    label: h.label(),
                    source,
                    username: h.username.clone(),
                },
            );
        }
        Ok(Self {
            policy: config.policy.clone(),
            tokens,
            timeout: Duration::from_secs(config.approval_timeout_secs),
            pending,
            prompter,
            cached: Mutex::new(HashMap::new()),
        })
    }

    async fn credential(&self, repo: &Repo) -> Result<Credential, Refusal> {
        let host = self
            .tokens
            .get(&repo.host)
            .ok_or_else(|| Refusal::Unavailable(format!("{} is not a git host in the config.", repo.host)))?;
        let Some(source) = &host.source else {
            return Err(Refusal::Unavailable(format!(
                "No {} token: set `token` for {} under [[git.hosts]] in config.toml.",
                host.label, repo.host
            )));
        };
        let cacheable = matches!(source, TokenSource::Gh { .. });
        if cacheable {
            let cached = self.cached.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some((token, _)) = cached.get(&repo.host).filter(|(_, at)| at.elapsed() < GH_CACHE) {
                return Ok(Self::wrap(&host.username, token.clone()));
            }
        }
        let token = source.fetch().await.map_err(|e| Refusal::Unavailable(format!("No {} token: {e}.", host.label)))?;
        if cacheable {
            self.cached
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(repo.host.clone(), (token.clone(), Instant::now()));
        }
        Ok(Self::wrap(&host.username, token))
    }

    fn wrap(username: &str, token: Zeroizing<String>) -> Credential {
        Credential {
            username: username.to_owned(),
            token,
            expires_at: crate::now_unix() + CREDENTIAL_SECS,
        }
    }

    /// Asks the person (once per `key` while open; an answer is remembered for git run again).
    async fn ask(&self, key: &str, what: &str, lines: Vec<String>) -> Result<(), Refusal> {
        let asked = self.pending.ask(key, what, lines).map_err(Refusal::Unavailable)?;
        let (id, mut answer) = match asked {
            Asked::Answered(true) => return Ok(()),
            Asked::Answered(false) => return Err(Refusal::Denied(format!("{what}: denied on this computer."))),
            Asked::Open {
                id,
                answer,
                fresh,
            } => {
                if fresh && let Some(item) = self.pending.get(&id) {
                    spawn_prompt(Arc::clone(&self.pending), Arc::clone(&self.prompter), item, answer.clone());
                }
                (id, answer)
            }
        };
        log::info!("asking on this computer: {what} (reins approve {id})");
        let wait = async {
            loop {
                if let Some(approved) = *answer.borrow_and_update() {
                    return Some(approved);
                }
                if answer.changed().await.is_err() {
                    return None;
                }
            }
        };
        match tokio::time::timeout(self.timeout, wait).await {
            Ok(Some(true)) => Ok(()),
            Ok(Some(false)) => Err(Refusal::Denied(format!("{what}: denied on this computer."))),
            Ok(None) | Err(_) => Err(Refusal::Waiting(format!(
                "Waiting for approval on this computer: {what}. Approve it (the desktop prompt, or `reins approve {id}`), then run git again."
            ))),
        }
    }
}

#[async_trait::async_trait]
impl Authorizer for LocalAuthorizer {
    async fn read(&self, repo: &Repo) -> Result<Credential, Refusal> {
        let name = repo.short();
        match decide_read(&self.policy, &repo.host, &repo.path) {
            Rule::Allow => {}
            Rule::Deny => return Err(Refusal::Denied(format!("The local policy does not allow reading {name}."))),
            Rule::Ask => {
                let key = format!("read {}", repo.label().to_ascii_lowercase());
                let lines = vec![format!("Git on this computer wants to clone or fetch {}.", repo.label())];
                self.ask(&key, &format!("Read {name}"), lines).await?;
            }
        }
        self.credential(repo).await
    }

    async fn push(&self, repo: &Repo, summary: &PushSummary, digest: &str) -> Result<Credential, Refusal> {
        let name = repo.short();
        match decide_push(&self.policy, &repo.host, &repo.path, summary) {
            Rule::Allow => {}
            Rule::Deny => {
                let what = if is_risky(summary) {
                    "this kind of push"
                } else {
                    "pushing"
                };
                return Err(Refusal::Denied(format!("The local policy does not allow {what} to {name}.")));
            }
            Rule::Ask => {
                self.ask(&format!("push {digest}"), &format!("Push to {name}"), describe_push(summary)).await?;
            }
        }
        self.credential(repo).await
    }

    fn waiting_hint(&self) -> String {
        "waiting for approval on this computer (the notification, or `reins pending` and `reins approve <id>`)"
            .to_owned()
    }

    fn describe(&self) -> String {
        "the local policy on this computer".to_owned()
    }

    fn decider(&self) -> crate::journal::Decider {
        crate::journal::Decider::Local
    }
}

#[cfg(test)]
mod tests {
    use reins_proto::desktop::{CommitInfo, RefUpdate, ZERO_OID};

    use super::*;
    use crate::auth::prompt::{NoPrompter, PendingItem};

    const A: &str = "1111111111111111111111111111111111111111";
    const B: &str = "2222222222222222222222222222222222222222";

    fn summary(name: &str, old: &str, ff: Option<bool>) -> PushSummary {
        PushSummary {
            updates: vec![RefUpdate {
                name: name.to_owned(),
                change: if old == ZERO_OID {
                    RefChange::Create
                } else {
                    RefChange::Update
                },
                old: old.to_owned(),
                new: B.to_owned(),
                fast_forward: ff,
                commit_count: 2,
                commits: vec![CommitInfo {
                    sha: B.to_owned(),
                    subject: "Fix it".to_owned(),
                    author: "A <a@b>".to_owned(),
                    date: 0,
                }],
                files_changed: 1,
                files: vec![],
                additions: Some(3),
                deletions: Some(1),
            }],
            ..PushSummary::default()
        }
    }

    fn repo() -> Repo {
        Repo::new("github.com", "github", "me/app")
    }

    fn config(dir: &std::path::Path, policy: &str) -> Config {
        std::fs::write(dir.join("token"), "tok-123\nignored\n").unwrap();
        let mut c: Config = toml::from_str(policy).unwrap();
        c.github.token = format!("file:{}", dir.join("token").display());
        c.approval_timeout_secs = 5;
        c
    }

    struct Scripted(Option<bool>);

    #[async_trait::async_trait]
    impl Prompter for Scripted {
        async fn ask(&self, _item: &PendingItem) -> Option<bool> {
            self.0
        }
    }

    #[test]
    fn token_sources_parse() {
        assert_eq!(
            TokenSource::parse("gh", "github.com").unwrap(),
            TokenSource::Gh {
                host: "github.com".into()
            }
        );
        assert_eq!(TokenSource::parse("env:GH_T", "h").unwrap(), TokenSource::Env("GH_T".into()));
        assert_eq!(TokenSource::parse("file:/x/y", "h").unwrap(), TokenSource::File("/x/y".into()));
        assert!(TokenSource::parse("env:", "h").is_err());
        assert!(TokenSource::parse("ghp_secret", "h").unwrap_err().contains("ghp_secret"));
    }

    #[test]
    fn push_lines_say_what_happens() {
        let lines = describe_push(&summary("refs/heads/main", A, Some(true)));
        assert_eq!(lines, vec!["Push 2 commits to main", "  2222222 Fix it", "  +3 -1 in 1 files"]);
        assert_eq!(
            describe_push(&summary("refs/heads/main", A, Some(false)))[0],
            "Force push to main: rewrites history"
        );
        assert_eq!(describe_push(&summary("refs/heads/x", ZERO_OID, None))[0], "Create branch x with 2 commits");
        assert_eq!(describe_push(&summary("refs/tags/v1", ZERO_OID, None))[0], "Tag v1 -> 2222222");
    }

    #[tokio::test]
    async fn allowed_reads_get_the_token_from_the_file_and_denied_ones_do_not() {
        let dir = tempfile::tempdir().unwrap();
        let c = config(dir.path(), "[[policy.rules]]\nrepo = \"me/secret\"\nread = \"deny\"\n");
        let auth = LocalAuthorizer::new(&c, Arc::default(), Arc::new(NoPrompter)).unwrap();
        let cred = auth.read(&repo()).await.unwrap();
        assert_eq!((cred.username.as_str(), cred.token.as_str()), ("x-access-token", "tok-123"));
        assert!(cred.is_live(crate::now_unix()));
        let secret = Repo {
            path: "me/secret".into(),
            ..repo()
        };
        assert!(matches!(auth.read(&secret).await, Err(Refusal::Denied(_))));
    }

    #[tokio::test]
    async fn each_host_has_its_own_token_user_name_and_rules() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("gl"), "glpat-1\n").unwrap();
        let text = format!(
            "[[git.hosts]]\nhost = \"gitlab.com\"\nenabled = true\ntoken = 'file:{}'\n[[git.hosts]]\nhost = \"codeberg.org\"\nenabled = true\n[[policy.rules]]\nhost = \"gitlab.com\"\nrepo = \"group/secret/**\"\nread = \"deny\"\n",
            dir.path().join("gl").display()
        );
        let c = config(dir.path(), &text);
        let auth = LocalAuthorizer::new(&c, Arc::default(), Arc::new(NoPrompter)).unwrap();
        let gl = auth.read(&Repo::new("gitlab.com", "gitlab", "group/sub/app")).await.unwrap();
        assert_eq!((gl.username.as_str(), gl.token.as_str()), ("oauth2", "glpat-1"));
        let gh = auth.read(&repo()).await.unwrap();
        assert_eq!((gh.username.as_str(), gh.token.as_str()), ("x-access-token", "tok-123"));
        let denied = auth.read(&Repo::new("gitlab.com", "gitlab", "group/secret/deep/app")).await;
        assert!(
            matches!(&denied, Err(Refusal::Denied(m)) if m.contains("gitlab.com/group/secret/deep/app")),
            "{denied:?}"
        );
        let none = auth.read(&Repo::new("codeberg.org", "codeberg", "me/app")).await;
        assert!(
            matches!(&none, Err(Refusal::Unavailable(m)) if m.starts_with("No Codeberg token") && m.contains("[[git.hosts]]")),
            "{none:?}"
        );
        let mut gh_for_gitlab = c.clone();
        gh_for_gitlab.git.hosts[0].token = Some("gh".to_owned());
        assert!(LocalAuthorizer::new(&gh_for_gitlab, Arc::default(), Arc::new(NoPrompter)).is_err());
    }

    #[tokio::test]
    async fn a_missing_token_is_unavailable() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = config(dir.path(), "");
        c.github.token = format!("file:{}", dir.path().join("none").display());
        let auth = LocalAuthorizer::new(&c, Arc::default(), Arc::new(NoPrompter)).unwrap();
        assert!(matches!(auth.read(&repo()).await, Err(Refusal::Unavailable(m)) if m.starts_with("No GitHub token")));
    }

    #[tokio::test]
    async fn the_prompt_answers_pushes_and_risky_pushes_use_their_own_rule() {
        let dir = tempfile::tempdir().unwrap();
        let c = config(dir.path(), "[policy]\nrisky = \"deny\"\n");
        let yes = LocalAuthorizer::new(&c, Arc::default(), Arc::new(Scripted(Some(true)))).unwrap();
        yes.push(&repo(), &summary("refs/heads/main", A, Some(true)), "d1").await.unwrap();
        let force = yes.push(&repo(), &summary("refs/heads/main", A, Some(false)), "d2").await;
        assert!(matches!(force, Err(Refusal::Denied(m)) if m.contains("this kind of push")));
        let no = LocalAuthorizer::new(&c, Arc::default(), Arc::new(Scripted(Some(false)))).unwrap();
        assert!(matches!(
            no.push(&repo(), &summary("refs/heads/main", A, Some(true)), "d1").await,
            Err(Refusal::Denied(_))
        ));
    }

    #[tokio::test(start_paused = true)]
    async fn no_answer_is_waiting_and_a_later_answer_counts_for_the_same_push() {
        let dir = tempfile::tempdir().unwrap();
        let c = config(dir.path(), "");
        let pending = Arc::new(Pending::default());
        let auth = LocalAuthorizer::new(&c, Arc::clone(&pending), Arc::new(NoPrompter)).unwrap();
        let s = summary("refs/heads/main", A, Some(true));
        let first = auth.push(&repo(), &s, "d1").await;
        let Err(Refusal::Waiting(message)) = first else {
            panic!("{first:?}")
        };
        let items = pending.list();
        assert_eq!(items.len(), 1);
        assert!(message.contains(&format!("reins approve {}", items[0].id)));
        assert_eq!(items[0].lines[0], "Push 2 commits to main");
        assert!(pending.answer(&items[0].id, true));
        auth.push(&repo(), &s, "d1").await.unwrap();
        // Another push (another digest) is asked again.
        assert!(matches!(auth.push(&repo(), &s, "d2").await, Err(Refusal::Waiting(_))));
    }
}
