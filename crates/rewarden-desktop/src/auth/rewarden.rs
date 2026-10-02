//! The phone decides, through the Rewarden server: git access requests go to the server's desktop API, and the
//! phone's answer is a credential sealed to this app's key.
//!
//! The server only relays: it cannot open the sealed credential, and an answer is accepted only when it echoes this
//! request's random nonce, the repository, the access asked for and (for a push) the digest of exactly what git sends,
//! and has not expired. So the server can neither read the token nor replay an older answer for another request.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use rewarden_proto::desktop::{CredentialGrant, PushSummary, SEALED_FIELD, fetch_tool_for};
use rewarden_proto::relay::{RelayOutcome, ToolResult};
use serde_json::{Value, json};
use zeroize::Zeroizing;

use super::{Authorizer, Credential, Refusal, Repo};
use crate::config::{Config, Paths};
use crate::identity::{Identity, IdentityError};
use crate::server::LinkError;
use crate::server::client::{CallAnswer, CallStatus, DesktopClient};

/// A retry of the same fetch or push within this time polls the earlier request instead of asking again.
const REUSE_WINDOW: Duration = Duration::from_mins(10);
/// Polls are at least this far apart (the server normally holds each poll open itself).
const POLL_SPACING: Duration = Duration::from_secs(1);

/// One question to the phone: the same tool, host, repository and push digest is the same question.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Question {
    /// `github_git_fetch`, `gitlab_git_push`, ...
    tool: String,
    /// Read questions are fetches; the others pushes.
    read: bool,
    host: String,
    repo: String,
    digest: String,
}

/// A question the phone has not answered yet; its nonce is kept because the eventual answer echoes it.
struct InFlight {
    request_id: String,
    nonce: String,
    asked_at: Instant,
}

pub struct RewardenAuthorizer {
    server: String,
    identity: Arc<Identity>,
    client: DesktopClient,
    /// Which of the phone's accounts each host uses (by host; none: the phone's only one).
    accounts: HashMap<String, String>,
    timeout: Duration,
    reads: Mutex<HashMap<String, Credential>>,
    in_flight: Mutex<HashMap<Question, InFlight>>,
    /// One ask per question at a time, so concurrent git requests do not ask the phone twice.
    asking: Mutex<HashMap<Question, Arc<tokio::sync::Mutex<()>>>>,
}

impl RewardenAuthorizer {
    /// Fails when the app is not logged in to a Rewarden server.
    pub fn new(paths: &Paths, identity: Arc<Identity>, config: &Config) -> Result<Self, String> {
        let server = crate::server::oauth::logged_in_server(paths)
            .ok_or_else(|| "not logged in to a Rewarden server; run `rewarden login <server>`".to_owned())?;
        Ok(Self {
            server,
            identity,
            client: DesktopClient::new(paths)?,
            accounts: config.git_hosts()?.into_iter().filter_map(|h| Some((h.host, h.account?))).collect(),
            timeout: Duration::from_secs(config.approval_timeout_secs),
            reads: Mutex::new(HashMap::new()),
            in_flight: Mutex::new(HashMap::new()),
            asking: Mutex::new(HashMap::new()),
        })
    }

    fn cached_read(&self, repo: &str) -> Option<Credential> {
        let mut reads = self.reads.lock().unwrap_or_else(PoisonError::into_inner);
        let now = crate::now_unix();
        reads.retain(|_, c| c.is_live(now));
        reads.get(repo).cloned()
    }

    fn turn(&self, q: &Question) -> Arc<tokio::sync::Mutex<()>> {
        let mut asking = self.asking.lock().unwrap_or_else(PoisonError::into_inner);
        // Only the map holds an idle question's lock: forget those, or every push digest would stay forever.
        asking.retain(|_, turn| Arc::strong_count(turn) > 1);
        Arc::clone(asking.entry(q.clone()).or_default())
    }

    fn take_in_flight(&self, q: &Question) -> Option<InFlight> {
        let mut map = self.in_flight.lock().unwrap_or_else(PoisonError::into_inner);
        map.retain(|_, f| f.asked_at.elapsed() < REUSE_WINDOW);
        map.remove(q)
    }

    fn keep_in_flight(&self, q: &Question, f: InFlight) {
        self.in_flight.lock().unwrap_or_else(PoisonError::into_inner).insert(q.clone(), f);
    }

    /// Asks the phone (or polls the earlier identical question) until it answers or the approval timeout; returns the
    /// opened, checked grant.
    async fn ask(
        &self,
        q: &Question,
        arguments: impl Fn(&str) -> Value,
        waiting: &str,
    ) -> Result<CredentialGrant, Refusal> {
        let deadline = Instant::now() + self.timeout;
        let mut reused = None;
        if let Some(earlier) = self.take_in_flight(q) {
            log::info!("{} {}: polling the earlier request {}", q.tool, q.repo, earlier.request_id);
            match self.client.poll(&earlier.request_id).await {
                Ok(a) => reused = Some((a, earlier)),
                Err(LinkError::NotFound) => log::info!("the server no longer has request {}", earlier.request_id),
                Err(e) => {
                    self.keep_in_flight(q, earlier);
                    return Err(link_refusal(e));
                }
            }
        }
        let (mut answer, asked) = if let Some(pair) = reused {
            pair
        } else {
            let nonce = crate::server::random_token(16);
            let account = self.accounts.get(&q.host).map(String::as_str);
            let a = self.client.call(&q.tool, &arguments(&nonce), account).await.map_err(link_refusal)?;
            log::info!("{} {}: asked the phone, request {}", q.tool, q.repo, a.request_id);
            let f = InFlight {
                request_id: a.request_id.clone(),
                nonce,
                asked_at: Instant::now(),
            };
            (a, f)
        };
        loop {
            let CallAnswer {
                request_id,
                status,
            } = answer;
            match status {
                CallStatus::Answered(outcome) => {
                    log::info!("{} {}: the phone answered request {request_id}", q.tool, q.repo);
                    return self.open(q, &asked.nonce, outcome);
                }
                CallStatus::Pending | CallStatus::Offline => {
                    let polled_at = Instant::now();
                    let remaining = deadline.saturating_duration_since(polled_at);
                    if remaining.is_zero() {
                        self.keep_in_flight(q, asked);
                        return Err(Refusal::Waiting(waiting.to_owned()));
                    }
                    tokio::time::sleep(POLL_SPACING.min(remaining)).await;
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    match tokio::time::timeout(remaining, self.client.poll(&asked.request_id)).await {
                        Ok(Ok(a)) => answer = a,
                        Ok(Err(LinkError::NotFound)) => {
                            return Err(Refusal::Unavailable(
                                "The Rewarden server no longer has this request. Run git again.".to_owned(),
                            ));
                        }
                        Ok(Err(e)) => {
                            self.keep_in_flight(q, asked);
                            return Err(link_refusal(e));
                        }
                        Err(_) => {
                            self.keep_in_flight(q, asked);
                            return Err(Refusal::Waiting(waiting.to_owned()));
                        }
                    }
                }
            }
        }
    }

    /// The phone's outcome: a sealed grant that must answer exactly this question, or a refusal.
    fn open(&self, q: &Question, nonce: &str, outcome: RelayOutcome) -> Result<CredentialGrant, Refusal> {
        let data = match outcome {
            RelayOutcome::Result {
                result: ToolResult::Connector {
                    data,
                },
            } => data,
            RelayOutcome::Result {
                ..
            } => return Err(unavailable("The phone's answer is not a git credential.")),
            RelayOutcome::Denied {
                reason,
            } => {
                return Err(Refusal::Denied(
                    reason.filter(|r| !r.trim().is_empty()).unwrap_or_else(|| "Denied on your phone.".to_owned()),
                ));
            }
            RelayOutcome::Error {
                message,
            } => return Err(Refusal::Unavailable(message)),
        };
        let sealed = if q.read {
            data.pointer(&format!("/items/0/{SEALED_FIELD}"))
        } else {
            data.get(SEALED_FIELD)
        };
        let sealed =
            sealed.and_then(Value::as_str).ok_or_else(|| unavailable("The phone's answer has no credential."))?;
        let grant = self.identity.open_grant(sealed).map_err(|e| match e {
            IdentityError::Unseal => unavailable(
                "The phone's answer is not sealed to this app's key; refused. Pair again with `rewarden login`.",
            ),
            _ => unavailable("The phone's answer is malformed; refused."),
        })?;
        let access = if q.read {
            "read"
        } else {
            "write"
        };
        if grant.nonce != nonce {
            return Err(unavailable("The phone's answer is for another request (the nonce differs); refused."));
        }
        if grant.repo != q.repo {
            return Err(Refusal::Unavailable(format!(
                "The phone's answer is for {}, not {}; refused.",
                grant.repo, q.repo
            )));
        }
        if grant.access != access {
            return Err(Refusal::Unavailable(format!(
                "The phone's answer gives {} access, not {access}; refused.",
                grant.access
            )));
        }
        if access == "write" && grant.digest.as_deref() != Some(q.digest.as_str()) {
            return Err(unavailable(
                "The phone approved a different push (the digest differs); refused. Run git push again.",
            ));
        }
        if grant.expires_at <= crate::now_unix() {
            return Err(unavailable("The phone's answer has expired; refused. Run git again."));
        }
        if grant.token.is_empty() {
            return Err(unavailable("The phone's answer has no token; refused."));
        }
        Ok(grant)
    }
}

fn unavailable(message: &str) -> Refusal {
    Refusal::Unavailable(message.to_owned())
}

fn link_refusal(e: LinkError) -> Refusal {
    match e {
        LinkError::LoggedOut(m) => Refusal::Unavailable(m),
        LinkError::NotFound => unavailable("The Rewarden server no longer has this request. Run git again."),
        LinkError::Failed(m) => Refusal::Unavailable(format!("Cannot ask your phone: {m}")),
    }
}

fn credential(grant: CredentialGrant) -> Credential {
    Credential {
        username: grant.username,
        token: Zeroizing::new(grant.token),
        expires_at: grant.expires_at,
    }
}

#[async_trait::async_trait]
impl Authorizer for RewardenAuthorizer {
    async fn read(&self, repo: &Repo) -> Result<Credential, Refusal> {
        let name = repo.full_name();
        let cache_key = repo.label();
        if let Some(c) = self.cached_read(&cache_key) {
            return Ok(c);
        }
        let q = Question {
            tool: fetch_tool_for(&repo.service),
            read: true,
            host: repo.host.clone(),
            repo: name.clone(),
            digest: String::new(),
        };
        let turn = self.turn(&q);
        let _mine = turn.lock().await;
        // Another request for the same repository may have been answered while this one waited its turn.
        if let Some(c) = self.cached_read(&cache_key) {
            return Ok(c);
        }
        let key = self.identity.public_key();
        let grant = self
            .ask(
                &q,
                |nonce| json!({"repo": name, "client_key": key, "nonce": nonce}),
                "Waiting for approval on your phone. Approve it, then run git again.",
            )
            .await?;
        let c = credential(grant);
        self.reads.lock().unwrap_or_else(PoisonError::into_inner).insert(cache_key, c.clone());
        Ok(c)
    }

    async fn push(&self, repo: &Repo, summary: &PushSummary, digest: &str) -> Result<Credential, Refusal> {
        let name = repo.full_name();
        let q = Question {
            tool: summary.tool_for(&repo.service),
            read: false,
            host: repo.host.clone(),
            repo: name.clone(),
            digest: digest.to_owned(),
        };
        let turn = self.turn(&q);
        let _mine = turn.lock().await;
        let key = self.identity.public_key();
        let summary = serde_json::to_value(summary).map_err(|e| Refusal::Unavailable(format!("push summary: {e}")))?;
        let grant = self
            .ask(
                &q,
                |nonce| json!({"repo": name, "client_key": key, "nonce": nonce, "digest": digest, "summary": summary}),
                "Waiting for approval on your phone. Approve it, then run git push again.",
            )
            .await?;
        Ok(credential(grant))
    }

    fn waiting_hint(&self) -> String {
        "waiting for approval in your Rewarden app".to_owned()
    }

    fn describe(&self) -> String {
        format!("your phone, through {}", self.server)
    }
}
