//! `rewarden ask`: one yes-or-no question to the user. With the phone (logged in) it goes through the server's desktop
//! API as `desktop_ask`; the phone seals its decision ([`AskAnswer`]) to this app's key, echoing this question's random
//! nonce, so neither the server nor an old answer can say yes in the user's place. Without the phone the person at the
//! computer answers: in the terminal when there is one, else in a desktop notification or dialog.

use std::fmt::Write as _;
use std::io::{BufRead, Write};
use std::time::{Duration, Instant};

use rewarden_proto::desktop::{AskAnswer, SEALED_FIELD};
use rewarden_proto::relay::{RelayOutcome, ToolResult};
use serde_json::{Value, json};

use crate::auth::prompt::{PendingItem, Prompter};
use crate::config::{Config, Mode, Paths};
use crate::identity::{Identity, IdentityError};
use crate::server::LinkError;
use crate::server::client::{CallAnswer, CallStatus, DesktopClient};

pub const ASK_TOOL: &str = "desktop_ask";
const MAX_QUESTION: usize = 300;
const MAX_DETAIL: usize = 8_000;
const MAX_TOPIC: usize = 100;
const POLL_SPACING: Duration = Duration::from_secs(1);

/// A question, cut to what the phone shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Question {
    pub question: String,
    pub detail: Option<String>,
    pub topic: Option<String>,
}

/// At most `max` characters; a cut text ends with `…`.
fn cut(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut s: String = text.chars().take(max - 1).collect();
    s.push('…');
    s
}

fn one_line(text: &str) -> String {
    text.split(char::is_control).flat_map(str::split_whitespace).collect::<Vec<_>>().join(" ")
}

impl Question {
    /// The question on one line (≤ 300 characters), the detail without control characters but newlines and tabs (≤
    /// 8,000), the topic on one line (≤ 100). An empty question is refused.
    pub fn new(question: &str, detail: Option<&str>, topic: Option<&str>) -> Result<Self, String> {
        let question = cut(&one_line(question), MAX_QUESTION);
        if question.is_empty() {
            return Err("the question is empty".to_owned());
        }
        let detail = detail
            .map(|d| d.chars().filter(|c| !c.is_control() || matches!(c, '\n' | '\t')).collect::<String>())
            .map(|d| cut(d.trim_end(), MAX_DETAIL))
            .filter(|d| !d.trim().is_empty());
        let topic = topic.map(|t| cut(&one_line(t), MAX_TOPIC)).filter(|t| !t.is_empty());
        Ok(Self {
            question,
            detail,
            topic,
        })
    }

    fn arguments(&self, client_key: &str, nonce: &str) -> Value {
        let mut args = json!({"question": self.question, "client_key": client_key, "nonce": nonce});
        if let Some(d) = &self.detail {
            args["detail"] = json!(d);
        }
        if let Some(t) = &self.topic {
            args["topic"] = json!(t);
        }
        args
    }
}

/// The outcome of a question.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Answer {
    Yes,
    /// Said no (the reason, when one was given, else a plain sentence).
    No(String),
    /// Nobody answered, or the answer could not be trusted (the reason).
    Unanswered(String),
}

impl Answer {
    /// `rewarden ask`'s exit code: 0 yes, 1 no, 2 no answer.
    #[must_use]
    pub fn exit_code(&self) -> u8 {
        match self {
            Self::Yes => 0,
            Self::No(_) => 1,
            Self::Unanswered(_) => 2,
        }
    }
}

/// Who answers, from the config's mode and whether the app is logged in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decider {
    Phone,
    Local,
}

pub fn decider(paths: &Paths, config: &Config) -> Result<Decider, String> {
    let logged_in = crate::server::oauth::logged_in_server(paths).is_some();
    match (config.mode, logged_in) {
        (Mode::Local, _) | (Mode::Auto, false) => Ok(Decider::Local),
        (_, true) => Ok(Decider::Phone),
        (Mode::Rewarden, false) => {
            Err("not logged in to a Rewarden server (mode = \"rewarden\"); run `rewarden login <server>`".to_owned())
        }
    }
}

/// Asks whoever decides (see [`decider`]); `interactive`: the terminal may be used (stdin is a terminal).
pub async fn ask(
    paths: &Paths,
    config: &Config,
    q: &Question,
    timeout: Duration,
    interactive: bool,
    prompter: &dyn Prompter,
) -> Answer {
    match decider(paths, config) {
        Err(e) => Answer::Unanswered(e),
        Ok(Decider::Phone) => {
            let identity = match Identity::load_or_create(&paths.identity_file()) {
                Ok(id) => id,
                Err(e) => return Answer::Unanswered(e.to_string()),
            };
            ask_phone(paths, &identity, q, timeout).await
        }
        Ok(Decider::Local) if interactive => ask_terminal_stdin(q, timeout).await,
        Ok(Decider::Local) => ask_desktop(prompter, q, timeout).await,
    }
}

/// Asks the phone through the Rewarden server and waits up to `timeout` for its sealed answer.
pub async fn ask_phone(paths: &Paths, identity: &Identity, q: &Question, timeout: Duration) -> Answer {
    let client = match DesktopClient::new(paths) {
        Ok(c) => c,
        Err(e) => return Answer::Unanswered(e),
    };
    let deadline = Instant::now() + timeout;
    let nonce = crate::server::random_token(16);
    let args = q.arguments(&identity.public_key(), &nonce);
    let late = || Answer::Unanswered(format!("No answer on your phone within {} s.", timeout.as_secs()));
    let mut answer = match tokio::time::timeout(timeout, client.call(ASK_TOOL, &args, None)).await {
        Ok(Ok(a)) => a,
        Ok(Err(e)) => return link_failure(e),
        Err(_) => return late(),
    };
    loop {
        let CallAnswer {
            request_id,
            status,
        } = answer;
        match status {
            CallStatus::Answered(outcome) => {
                log::info!("{ASK_TOOL}: the phone answered request {request_id}");
                return open(identity, &nonce, outcome);
            }
            CallStatus::Pending | CallStatus::Offline => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return late();
                }
                tokio::time::sleep(POLL_SPACING.min(remaining)).await;
                let remaining = deadline.saturating_duration_since(Instant::now());
                answer = match tokio::time::timeout(remaining, client.poll(&request_id)).await {
                    Ok(Ok(a)) => a,
                    Ok(Err(e)) => return link_failure(e),
                    Err(_) => return late(),
                };
            }
        }
    }
}

fn link_failure(e: LinkError) -> Answer {
    match e {
        LinkError::LoggedOut(m) => Answer::Unanswered(m),
        LinkError::NotFound => Answer::Unanswered("The Rewarden server no longer has this question.".to_owned()),
        LinkError::Failed(m) => Answer::Unanswered(format!("Cannot ask your phone: {m}")),
    }
}

/// The phone's outcome: a sealed decision for exactly this question, or a denial.
fn open(identity: &Identity, nonce: &str, outcome: RelayOutcome) -> Answer {
    let refused = |m: &str| Answer::Unanswered(m.to_owned());
    let data = match outcome {
        RelayOutcome::Result {
            result: ToolResult::Connector {
                data,
            },
        } => data,
        RelayOutcome::Result {
            ..
        } => return refused("The phone's answer is not a decision; refused."),
        RelayOutcome::Denied {
            reason,
        } => {
            return Answer::No(
                reason.filter(|r| !r.trim().is_empty()).unwrap_or_else(|| "Denied on your phone.".to_owned()),
            );
        }
        RelayOutcome::Error {
            message,
        } => return Answer::Unanswered(message),
    };
    let Some(sealed) =
        data.get(SEALED_FIELD).or_else(|| data.pointer(&format!("/items/0/{SEALED_FIELD}"))).and_then(Value::as_str)
    else {
        return refused("The phone's answer has no sealed decision; refused.");
    };
    let plain = match identity.unseal(sealed) {
        Ok(p) => p,
        Err(IdentityError::Unseal) => {
            return refused(
                "The phone's answer is not sealed to this app's key; refused. Pair again with `rewarden login`.",
            );
        }
        Err(_) => return refused("The phone's answer is malformed; refused."),
    };
    let Ok(answer) = serde_json::from_slice::<AskAnswer>(&plain) else {
        return refused("The phone's answer is malformed; refused.");
    };
    if answer.nonce != nonce {
        return refused("The phone's answer is for another question (the nonce differs); refused.");
    }
    if answer.approved {
        Answer::Yes
    } else {
        Answer::No("Denied on your phone.".to_owned())
    }
}

/// Asks in the terminal: the question on `out`, the answer a line from `input` (`y`/`yes` yes; `n`, `no` or an empty
/// line no; end of input no answer).
pub fn ask_terminal(input: &mut impl BufRead, out: &mut impl Write, q: &Question) -> Answer {
    let mut text = format!("{}\n", q.question);
    if let Some(d) = &q.detail {
        for line in d.lines() {
            let _infallible = writeln!(text, "  {line}");
        }
    }
    text.push_str("Allow? [y/N] ");
    if out.write_all(text.as_bytes()).and_then(|()| out.flush()).is_err() {
        return Answer::Unanswered("Cannot write the question to the terminal.".to_owned());
    }
    let mut line = String::new();
    match input.read_line(&mut line) {
        Ok(0) | Err(_) => Answer::Unanswered("No answer in the terminal.".to_owned()),
        Ok(_) => match line.trim().to_ascii_lowercase().as_str() {
            "y" | "yes" => Answer::Yes,
            _ => Answer::No("Denied in the terminal.".to_owned()),
        },
    }
}

/// [`ask_terminal`] on stdin and stderr, giving up after `timeout`.
pub async fn ask_terminal_stdin(q: &Question, timeout: Duration) -> Answer {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let q2 = q.clone();
    // A plain thread, not the runtime's blocking pool: a read that never returns must not keep the program alive.
    std::thread::spawn(move || {
        let answer = ask_terminal(&mut std::io::stdin().lock(), &mut std::io::stderr(), &q2);
        let _sent = tx.send(answer);
    });
    match tokio::time::timeout(timeout, rx).await {
        Ok(Ok(a)) => a,
        Ok(Err(_)) => Answer::Unanswered("No answer in the terminal.".to_owned()),
        Err(_) => {
            eprintln!();
            Answer::Unanswered(format!("No answer within {} s.", timeout.as_secs()))
        }
    }
}

/// Asks with a desktop notification or dialog.
pub async fn ask_desktop(prompter: &dyn Prompter, q: &Question, timeout: Duration) -> Answer {
    let item = PendingItem {
        id: String::new(),
        what: q.question.clone(),
        lines: q.detail.as_deref().map(|d| d.lines().map(str::to_owned).collect()).unwrap_or_default(),
        created_at: crate::now_unix(),
    };
    match tokio::time::timeout(timeout, prompter.ask(&item)).await {
        Ok(Some(true)) => Answer::Yes,
        Ok(Some(false)) => Answer::No("Denied on the desktop.".to_owned()),
        Ok(None) => Answer::Unanswered(
            "No answer: the desktop prompt was dismissed or is not available (notify-send on Linux, osascript on \
             macOS). Run `rewarden ask` in a terminal, or log in to decide on your phone."
                .to_owned(),
        ),
        Err(_) => Answer::Unanswered(format!("No answer within {} s.", timeout.as_secs())),
    }
}

/// A desktop notification with Allow/Deny on Linux (`notify-send`), a dialog on macOS (`osascript`).
pub struct DesktopAsk;

fn escape_markup(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

#[async_trait::async_trait]
impl Prompter for DesktopAsk {
    async fn ask(&self, item: &PendingItem) -> Option<bool> {
        let body = if item.lines.is_empty() {
            item.what.clone()
        } else {
            item.lines.join("\n")
        };
        let mut cmd = if cfg!(target_os = "macos") {
            let mut c = tokio::process::Command::new("osascript");
            // The text goes in as arguments, never into the script.
            c.args([
                "-e",
                "on run argv",
                "-e",
                "display dialog (item 2 of argv) with title (item 1 of argv) buttons {\"Deny\", \"Allow\"} default button \"Deny\"",
                "-e",
                "end run",
                &format!("Rewarden: {}", item.what),
                &body,
            ]);
            c
        } else if cfg!(unix) {
            let mut c = tokio::process::Command::new("notify-send");
            c.args([
                "--app-name=Rewarden",
                "--urgency=critical",
                "--action=allow=Allow",
                "--action=deny=Deny",
                "--wait",
                &format!("Rewarden: {}", escape_markup(&item.what)),
                &escape_markup(&body),
            ]);
            c
        } else {
            return None;
        };
        cmd.stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null()).kill_on_drop(true);
        let out = cmd.output().await.ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        match text.trim() {
            t if t == "allow" || t.ends_with("button returned:Allow") => Some(true),
            t if t == "deny" || t.ends_with("button returned:Deny") => Some(false),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn questions_are_cut_to_what_the_phone_shows() {
        let q =
            Question::new("  Push\nto main?\t ", Some("line 1\nline 2\u{7}\n\n"), Some("command:git\npush")).unwrap();
        assert_eq!(q.question, "Push to main?");
        assert_eq!(q.detail.as_deref(), Some("line 1\nline 2"));
        assert_eq!(q.topic.as_deref(), Some("command:git push"));
        let long = Question::new(&"x".repeat(400), Some(&"d".repeat(9_000)), Some(&"t".repeat(200))).unwrap();
        assert_eq!(long.question.chars().count(), 300);
        assert!(long.question.ends_with('…'));
        assert_eq!(long.detail.unwrap().chars().count(), 8_000);
        assert_eq!(long.topic.unwrap().chars().count(), 100);
        assert!(Question::new(" \n ", None, None).is_err());
        let q = Question::new("q", Some("  "), Some("")).unwrap();
        assert_eq!((q.detail, q.topic), (None, None));
        let args = Question::new("q", None, None).unwrap().arguments("k", "n");
        assert_eq!(args, json!({"question": "q", "client_key": "k", "nonce": "n"}));
    }

    #[test]
    fn the_terminal_answer_is_yes_only_when_typed() {
        let q = Question::new("Deploy?", Some("make deploy"), None).unwrap();
        for (typed, want) in [("y\n", 0), ("YES\n", 0), ("n\n", 1), ("\n", 1), ("maybe\n", 1), ("", 2)] {
            let mut out = Vec::new();
            let a = ask_terminal(&mut typed.as_bytes(), &mut out, &q);
            assert_eq!(a.exit_code(), want, "{typed:?}");
            let shown = String::from_utf8(out).unwrap();
            assert!(shown.starts_with("Deploy?\n  make deploy\nAllow? [y/N]"), "{shown}");
        }
    }

    #[tokio::test]
    async fn the_desktop_prompt_answers_or_leaves_it_unanswered() {
        struct Fixed(Option<bool>);
        #[async_trait::async_trait]
        impl Prompter for Fixed {
            async fn ask(&self, item: &PendingItem) -> Option<bool> {
                assert_eq!(item.what, "Deploy?");
                assert_eq!(item.lines, vec!["make deploy"]);
                self.0
            }
        }
        struct Never;
        #[async_trait::async_trait]
        impl Prompter for Never {
            async fn ask(&self, _: &PendingItem) -> Option<bool> {
                std::future::pending().await
            }
        }
        let q = Question::new("Deploy?", Some("make deploy"), None).unwrap();
        let t = Duration::from_secs(5);
        assert_eq!(ask_desktop(&Fixed(Some(true)), &q, t).await, Answer::Yes);
        assert_eq!(ask_desktop(&Fixed(Some(false)), &q, t).await.exit_code(), 1);
        assert_eq!(ask_desktop(&Fixed(None), &q, t).await.exit_code(), 2);
        assert_eq!(ask_desktop(&Never, &q, Duration::from_millis(20)).await.exit_code(), 2);
    }

    #[test]
    fn who_decides_follows_the_mode_and_the_login() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        let mut c = Config::default();
        assert_eq!(decider(&paths, &c), Ok(Decider::Local));
        c.mode = Mode::Rewarden;
        assert!(decider(&paths, &c).unwrap_err().contains("rewarden login"));
        paths.ensure().unwrap();
        std::fs::write(
            paths.session_file(),
            r#"{"server":"https://rw","client_id":"c","token_endpoint":"https://rw/t","access_token":"a","access_expires_at":0}"#,
        )
        .unwrap();
        assert_eq!(decider(&paths, &c), Ok(Decider::Phone));
        c.mode = Mode::Auto;
        assert_eq!(decider(&paths, &c), Ok(Decider::Phone));
        c.mode = Mode::Local;
        assert_eq!(decider(&paths, &c), Ok(Decider::Local));
    }
}
