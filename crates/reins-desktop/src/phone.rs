//! Asking the phone for things other than git credentials (secrets for `reins run` and the API proxy, SSH keys and
//! signatures): one desktop tool call through the Reins server, polled until the phone answers or the approval
//! timeout, with the same rules as git access ([`crate::auth::reins`]): a random nonce per question that the sealed
//! answer must echo, and a retry of an unanswered question polls the earlier request instead of asking again.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use reins_proto::desktop::SEALED_FIELD;
use reins_proto::relay::{RelayOutcome, ToolResult};
use serde::de::DeserializeOwned;
use serde_json::Value;
use zeroize::Zeroizing;

use crate::auth::Refusal;
use crate::config::{Mode, Paths};
use crate::identity::{Identity, IdentityError};
use crate::journal::{Decider, Entry, Journal, Outcome};
use crate::server::LinkError;
use crate::server::client::{CallAnswer, CallStatus, DesktopClient};

/// A retry of the same question within this time polls the earlier request instead of asking again.
const REUSE_WINDOW: Duration = Duration::from_mins(10);
/// Polls are at least this far apart (the server normally holds each poll open itself).
const POLL_SPACING: Duration = Duration::from_secs(1);
/// How long an answer may take before the person waiting is told who is being asked.
pub const NOTICE_AFTER: Duration = Duration::from_millis(800);

/// What the phone answered: the tool's data, and the nonce the question carried.
#[derive(Debug)]
pub struct Answer {
    pub data: Value,
    pub nonce: String,
}

struct InFlight {
    request_id: String,
    nonce: String,
    asked_at: Instant,
}

/// The phone, through the Reins server the app is logged in to.
pub struct Phone {
    server: String,
    identity: Arc<Identity>,
    client: DesktopClient,
    timeout: Duration,
    in_flight: Mutex<HashMap<String, InFlight>>,
}

impl Phone {
    /// Fails when the app is not logged in to a Reins server.
    pub fn new(paths: &Paths, identity: Arc<Identity>, timeout: Duration) -> Result<Self, String> {
        let server = crate::server::oauth::logged_in_server(paths)
            .ok_or_else(|| "not logged in to a Reins server; run `reins login`".to_owned())?;
        Ok(Self {
            server,
            identity,
            client: DesktopClient::new(paths)?,
            timeout,
            in_flight: Mutex::new(HashMap::new()),
        })
    }

    #[must_use]
    pub fn server(&self) -> &str {
        &self.server
    }

    #[must_use]
    pub fn identity(&self) -> &Identity {
        &self.identity
    }

    fn take_in_flight(&self, key: &str) -> Option<InFlight> {
        let mut map = self.in_flight.lock().unwrap_or_else(PoisonError::into_inner);
        map.retain(|_, f| f.asked_at.elapsed() < REUSE_WINDOW);
        map.remove(key)
    }

    fn keep_in_flight(&self, key: Option<&str>, f: InFlight) {
        if let Some(key) = key {
            self.in_flight.lock().unwrap_or_else(PoisonError::into_inner).insert(key.to_owned(), f);
        }
    }

    /// Asks the phone `tool` with the arguments `arguments(nonce)` (plus this app's `client_key` and the `nonce`), and
    /// waits until it answers or the approval timeout. With `reuse`, a question left unanswered under the same key is
    /// polled again instead of asked again. `waiting` is the message when the timeout passes first.
    pub async fn ask(
        &self,
        tool: &str,
        reuse: Option<&str>,
        arguments: impl Fn(&str) -> Value,
        waiting: &str,
    ) -> Result<Answer, Refusal> {
        let deadline = Instant::now() + self.timeout;
        let mut reused = None;
        if let Some(earlier) = reuse.and_then(|key| self.take_in_flight(key)) {
            log::info!("{tool}: polling the earlier request {}", earlier.request_id);
            match self.client.poll(&earlier.request_id).await {
                Ok(a) => reused = Some((a, earlier)),
                Err(LinkError::NotFound) => log::info!("the server no longer has request {}", earlier.request_id),
                Err(e) => {
                    self.keep_in_flight(reuse, earlier);
                    return Err(link_refusal(e));
                }
            }
        }
        let (mut answer, asked) = if let Some(pair) = reused {
            pair
        } else {
            let nonce = crate::server::random_token(16);
            let mut args = arguments(&nonce);
            if let Some(o) = args.as_object_mut() {
                o.insert("client_key".to_owned(), Value::String(self.identity.public_key()));
                o.insert("nonce".to_owned(), Value::String(nonce.clone()));
            }
            let waits = deadline.saturating_duration_since(Instant::now());
            let a = self.client.call_waiting(tool, &args, None, Some(waits)).await.map_err(link_refusal)?;
            log::info!("{tool}: asked the phone, request {}", a.request_id);
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
                    log::info!("{tool}: the phone answered request {request_id}");
                    return outcome_data(outcome).map(|data| Answer {
                        data,
                        nonce: asked.nonce,
                    });
                }
                CallStatus::Pending | CallStatus::Offline => {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        self.keep_in_flight(reuse, asked);
                        return Err(Refusal::Waiting(waiting.to_owned()));
                    }
                    tokio::time::sleep(POLL_SPACING.min(remaining)).await;
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    match tokio::time::timeout(remaining, self.client.poll(&asked.request_id)).await {
                        Ok(Ok(a)) => answer = a,
                        Ok(Err(LinkError::NotFound)) => {
                            return Err(unavailable("The Reins server no longer has this request. Try again."));
                        }
                        Ok(Err(e)) => {
                            self.keep_in_flight(reuse, asked);
                            return Err(link_refusal(e));
                        }
                        Err(_) => {
                            self.keep_in_flight(reuse, asked);
                            return Err(Refusal::Waiting(waiting.to_owned()));
                        }
                    }
                }
            }
        }
    }

    /// Opens the box the phone sealed to this app under `sealed` in `data`.
    pub fn open<T: DeserializeOwned>(&self, data: &Value) -> Result<T, Refusal> {
        let sealed = data
            .get(SEALED_FIELD)
            .and_then(Value::as_str)
            .ok_or_else(|| unavailable("The phone's answer has nothing sealed for this app."))?;
        let plain: Zeroizing<Vec<u8>> = self.identity.unseal(sealed).map_err(|e| match e {
            IdentityError::Unseal => unavailable(
                "The phone's answer is not sealed to this app's key; refused. Pair again with `reins login`.",
            ),
            _ => unavailable("The phone's answer is malformed; refused."),
        })?;
        serde_json::from_slice(&plain).map_err(|_| unavailable("The phone's answer is malformed; refused."))
    }
}

fn outcome_data(outcome: RelayOutcome) -> Result<Value, Refusal> {
    match outcome {
        RelayOutcome::Result {
            result: ToolResult::Connector {
                data,
            },
        } => Ok(data),
        RelayOutcome::Result {
            ..
        } => Err(unavailable("The phone's answer has an unexpected kind.")),
        // Text the server relays (and could have written): never printed with its control characters.
        RelayOutcome::Denied {
            reason,
        } => Err(Refusal::Denied(
            reason
                .map(|r| reins_proto::desktop::printable(&r))
                .filter(|r| !r.is_empty())
                .unwrap_or_else(|| "Denied on your phone.".to_owned()),
        )),
        RelayOutcome::Error {
            message,
        } => Err(Refusal::Unavailable(reins_proto::desktop::printable(&message))),
    }
}

pub(crate) fn unavailable(message: &str) -> Refusal {
    Refusal::Unavailable(message.to_owned())
}

fn link_refusal(e: LinkError) -> Refusal {
    match e {
        LinkError::LoggedOut(m) => Refusal::Unavailable(m),
        LinkError::NotFound => unavailable("The Reins server no longer has this request. Try again."),
        LinkError::Failed(m) => Refusal::Unavailable(format!("Cannot ask your phone: {m}")),
    }
}

/// The phone as the daemon sees it: checked on every use, so logging in or out takes effect without a restart. In local
/// mode there is no phone, and secrets and SSH keys, which live only on the phone, are unavailable.
pub struct PhoneLink {
    mode: Mode,
    paths: Paths,
    identity: Arc<Identity>,
    timeout: Duration,
    journal: Journal,
    current: Mutex<Option<(String, Arc<Phone>)>>,
}

impl PhoneLink {
    #[must_use]
    pub fn new(mode: Mode, paths: Paths, identity: Arc<Identity>, timeout: Duration) -> Self {
        Self {
            mode,
            journal: Journal::new(&paths),
            paths,
            identity,
            timeout,
            current: Mutex::new(None),
        }
    }

    /// Where the requests asked through this link are logged.
    #[must_use]
    pub fn journal(&self) -> &Journal {
        &self.journal
    }

    /// How long the phone is given to answer.
    #[must_use]
    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// The phone, or why there is none.
    pub fn phone(&self) -> Result<Arc<Phone>, Refusal> {
        if self.mode == Mode::Local {
            return Err(unavailable(
                "Secrets and SSH keys live on your phone, and this app is in local mode (`mode = \"local\"`). Set \
                 `mode = \"auto\"` and run `reins login`.",
            ));
        }
        let Some(server) = crate::server::oauth::logged_in_server(&self.paths) else {
            return Err(unavailable(
                "Secrets and SSH keys live on your phone: run `reins login` to pair this app with it.",
            ));
        };
        let mut current = self.current.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((s, p)) = current.as_ref()
            && *s == server
        {
            return Ok(Arc::clone(p));
        }
        let p =
            Arc::new(Phone::new(&self.paths, Arc::clone(&self.identity), self.timeout).map_err(Refusal::Unavailable)?);
        *current = Some((server, Arc::clone(&p)));
        Ok(p)
    }
}

/// Where waiting lines go (a terminal, a git or ssh process's stderr).
pub type Tell = Arc<dyn Fn(&str) + Send + Sync>;

#[must_use]
pub fn teller(f: impl Fn(&str) + Send + Sync + 'static) -> Tell {
    Arc::new(f)
}

/// The activity log's outcome for a decision.
#[must_use]
pub fn outcome_of<T>(r: &Result<T, Refusal>) -> (Outcome, Option<String>) {
    match r {
        Ok(_) => (Outcome::Approved, None),
        Err(Refusal::Denied(m)) => (Outcome::Denied, Some(m.clone())),
        Err(Refusal::Waiting(m)) => (Outcome::TimedOut, Some(m.clone())),
        Err(Refusal::Unavailable(m)) => (Outcome::Failed, Some(m.clone())),
    }
}

/// The line that says a request waits: who is asked, for how long at most, and what.
#[must_use]
pub fn waiting_line(decider: Decider, timeout: Duration, what: &str) -> String {
    match decider {
        Decider::Local => format!(
            "reins: waiting for approval on this computer (the notification, or `reins pending`), up to {} s: {what}…",
            timeout.as_secs()
        ),
        Decider::Phone | Decider::Settings => {
            format!("reins: waiting for Reins 2FA on your phone, up to {} s: {what}…", timeout.as_secs())
        }
    }
}

/// The line that says how a request that waited ended.
#[must_use]
pub fn ended_line(decider: Decider, outcome: Outcome, reason: Option<&str>, timeout: Duration) -> String {
    let where_ = match decider {
        Decider::Local => "on this computer",
        Decider::Phone | Decider::Settings => "on your phone",
    };
    let why = reason.map(str::trim).filter(|r| !r.is_empty());
    match outcome {
        Outcome::Approved | Outcome::Allowed => format!("reins: approved {where_}."),
        Outcome::Denied | Outcome::Blocked => match why {
            Some(r) if !r.starts_with("Denied") => format!("reins: denied {where_}: {r}"),
            _ => format!("reins: denied {where_}."),
        },
        Outcome::TimedOut => format!(
            "reins: timed out: no answer {where_} within {} s. Approve it there, then try again.",
            timeout.as_secs()
        ),
        Outcome::Failed | Outcome::Waiting => format!("reins: not approved: {}", why.unwrap_or("no answer")),
    }
}

/// Waits for `decision` and writes it to the activity log as `entry`. When it takes longer than [`NOTICE_AFTER`]
/// (somebody has to look), the entry is logged as waiting, the desktop says "check your phone" (phone decisions only),
/// and `tell` gets a waiting line; afterwards `tell` hears how it ended: approved, denied or timed out. A quick answer
/// (a grant, Autopilot) is only logged. `tell` may block: it runs on a blocking thread.
pub async fn awaiting<T>(
    journal: &Journal,
    entry: Entry,
    tell: Option<Tell>,
    timeout: Duration,
    decision: impl Future<Output = Result<T, Refusal>>,
) -> Result<T, Refusal> {
    awaiting_logged(journal, entry, tell, timeout, true, decision).await
}

/// [`awaiting`]; with `log_quick_yes` false, a quick yes is not logged (a git read with a credential still valid).
pub async fn awaiting_logged<T>(
    journal: &Journal,
    mut entry: Entry,
    tell: Option<Tell>,
    timeout: Duration,
    log_quick_yes: bool,
    decision: impl Future<Output = Result<T, Refusal>>,
) -> Result<T, Refusal> {
    let mut decision = std::pin::pin!(decision);
    tokio::select! {
        r = &mut decision => {
            if log_quick_yes || r.is_err() {
                let (outcome, reason) = outcome_of(&r);
                entry.end(outcome, reason.as_deref());
                journal.record(&entry);
            }
            return r;
        }
        () = tokio::time::sleep(NOTICE_AFTER) => {}
    }
    journal.record(&entry);
    if entry.decider == Decider::Phone {
        journal.notify_waiting(&entry.what);
    }
    let told = tell.clone().map(|t| {
        let line = waiting_line(entry.decider, timeout, &entry.what);
        tokio::task::spawn_blocking(move || t(&line))
    });
    let r = decision.await;
    if let Some(told) = told {
        told.await.ok();
    }
    let (outcome, reason) = outcome_of(&r);
    entry.end(outcome, reason.as_deref());
    journal.record(&entry);
    if let Some(tell) = tell {
        let line = ended_line(entry.decider, outcome, reason.as_deref(), timeout);
        tokio::task::spawn_blocking(move || tell(&line)).await.ok();
    }
    r
}
