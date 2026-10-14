//! Only a person runs `reins secrets scan` (and `import`, which scans or moves what a scan found).
//!
//! **The lock is the phone** ([`authorize`]): before it scans, the command asks the phone (`desktop_ask`, as the
//! harness hooks do) "Scan <computer> for secrets?", with where it was started. Only an approved answer lets it run;
//! a denial, no answer, or no paired phone refuses. The question's topic is a `file:` one, so the phone asks it every
//! time: never covered by a standing answer, "Approve all" or Autopilot. Bypass on the phone approves it like
//! everything else, by the user's own decision. `import` then stores through the phone's batch approval.
//!
//! Before that, courtesy checks that stop the casual case early and say why, but that a determined agent can get
//! around (so they are never the lock): the environment's markers ([`Caller`]: `REINS_RUNNER=agent:*`, `CLAUDECODE`,
//! …, which can be unset); a terminal on stdin and the controlling tty (a pseudo-terminal from `script` or `expect`
//! gives one); and a `y` typed on that terminal (which such a program can type). The guard's built-in `secrets` rules
//! also ask the phone, every time, when a harness with Reins' hooks runs these commands.
//!
//! The desktop app's Scan button is a person by construction (an agent does not click the window): its scan runs in
//! the app's process and asks nothing.

use std::io::{IsTerminal as _, Write as _};
use std::path::Path;
use std::time::Duration;

use crate::ask::{self, Answer, Question};
use crate::config::{Config, Paths};
use crate::journal::{Entry, Kind};
use crate::secrets_scan::Caller;

/// The phone question's topic: a `file:` topic is asked every time on the phone (and the phone's core lists
/// `reins secrets scan` as never routine).
pub const SCAN_TOPIC: &str = "file:reins secrets scan";

/// What this process's terminal is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Terminal {
    pub stdin: bool,
    pub stdout: bool,
    /// The controlling terminal opens (`/dev/tty`, `CONIN$`).
    pub tty: bool,
}

impl Terminal {
    #[must_use]
    pub fn now() -> Self {
        Self {
            stdin: std::io::stdin().is_terminal(),
            stdout: std::io::stdout().is_terminal(),
            tty: open_tty().is_some(),
        }
    }
}

#[cfg(unix)]
fn open_tty() -> Option<std::fs::File> {
    std::fs::OpenOptions::new().read(true).write(true).open("/dev/tty").ok()
}

#[cfg(windows)]
fn open_tty() -> Option<std::fs::File> {
    std::fs::OpenOptions::new().read(true).write(true).open("CONIN$").ok()
}

#[cfg(not(any(unix, windows)))]
fn open_tty() -> Option<std::fs::File> {
    None
}

/// Asks on the terminal itself (never stdin): the line typed, `None` without a terminal.
#[must_use]
pub fn ask_tty(prompt: &str) -> Option<String> {
    let tty = open_tty()?;
    #[cfg(windows)]
    let mut out = std::fs::OpenOptions::new().write(true).open("CONOUT$").ok()?;
    #[cfg(not(windows))]
    let mut out = tty.try_clone().ok()?;
    write!(out, "{prompt}").ok()?;
    out.flush().ok()?;
    let mut line = String::new();
    std::io::BufRead::read_line(&mut std::io::BufReader::new(tty), &mut line).ok()?;
    Some(line.trim().to_owned())
}

/// What the command needs from where it runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Needs {
    /// Its output is for a person to read (not `--json` to a file).
    pub stdout: bool,
}

/// Lets a person through: the caller's markers, a terminal, then `confirm` (the question asked on the terminal) must
/// be answered `y`. `what` names the command in the messages.
pub fn check(
    what: &str,
    question: &str,
    caller: &Caller,
    terminal: Terminal,
    needs: Needs,
    confirm: &mut (dyn FnMut(&str) -> Option<String> + Send),
) -> Result<(), String> {
    caller.check(what)?;
    if !terminal.stdin || !terminal.tty || (needs.stdout && !terminal.stdout) {
        return Err(format!(
            "{what} runs only in your own terminal, where you confirm it (or from the Reins app: Keys & secrets, Scan). \
             It cannot be run by a program or an AI agent."
        ));
    }
    match confirm(&format!("{question} [y/N] ")).as_deref().map(str::trim) {
        Some("y" | "Y" | "yes" | "Yes") => Ok(()),
        _ => Err("Nothing done.".to_owned()),
    }
}

/// What the phone is asked before a terminal scans: which computer, where it was started, who the environment says
/// runs it.
#[must_use]
pub fn scan_question(computer: &str, cwd: &Path, caller: &Caller) -> Question {
    let who = match caller {
        Caller::Person => {
            "Its environment names no AI agent (an agent could hide that: approve only if you started it).".to_owned()
        }
        Caller::Agent(marker) => format!("Its environment says an AI agent runs it ({marker})."),
    };
    let detail = format!(
        "Started in a terminal in {}.\nIt lists where secrets are on this computer (names, places, masked previews); \
         nothing leaves it.\n{who}",
        cwd.display()
    );
    Question::new(&format!("Scan {computer} for secrets?"), Some(&detail), Some(SCAN_TOPIC)).unwrap_or_else(|_| {
        Question {
            question: "Scan this computer for secrets?".to_owned(),
            detail: None,
            topic: Some(SCAN_TOPIC.to_owned()),
        }
    })
}

/// Who answers the phone question.
#[async_trait::async_trait]
pub trait Approver: Send + Sync {
    async fn approve(&self, q: &Question) -> Answer;
}

/// The paired phone, through the Reins server (logged in this computer's activity like a hook's question). Without
/// a paired phone nothing can approve: the answer is no.
pub struct Phone<'a> {
    pub paths: &'a Paths,
    pub config: &'a Config,
}

#[async_trait::async_trait]
impl Approver for Phone<'_> {
    async fn approve(&self, q: &Question) -> Answer {
        if !matches!(ask::decider(self.paths, self.config), Ok(ask::Decider::Phone)) {
            return Answer::Unanswered(
                "this computer is not paired with a phone, which has to approve a scan: pair it (reins setup), or \
                 scan from the Reins app (Keys & secrets)"
                    .to_owned(),
            );
        }
        let entry = Entry::new(Kind::Ask, &q.question).source(Some("reins secrets scan")).detail(q.detail.as_deref());
        let tell = |line: &str| eprintln!("{line}");
        let timeout = Duration::from_secs(self.config.approval_timeout_secs);
        ask::ask_logged(self.paths, self.config, q, timeout, false, &ask::DesktopAsk, entry, Some(&tell)).await.0
    }
}

/// The courtesy checks ([`check`]), then the lock: the phone must approve `q`.
#[allow(clippy::too_many_arguments, reason = "each check's input, injectable for tests")]
pub async fn authorize(
    what: &str,
    question: &str,
    caller: &Caller,
    terminal: Terminal,
    needs: Needs,
    confirm: &mut (dyn FnMut(&str) -> Option<String> + Send),
    approver: &dyn Approver,
    q: &Question,
) -> Result<(), String> {
    check(what, question, caller, terminal, needs, confirm)?;
    eprintln!("Approve it on your phone…");
    match approver.approve(q).await {
        Answer::Yes => Ok(()),
        Answer::No(why) => Err(format!("Not scanned: your phone said no ({why}).")),
        Answer::Unanswered(why) => Err(format!("Not scanned: {why}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Says(Answer, std::sync::Mutex<u32>);

    #[async_trait::async_trait]
    impl Approver for Says {
        async fn approve(&self, _q: &Question) -> Answer {
            *self.1.lock().unwrap() += 1;
            self.0.clone()
        }
    }

    fn says(a: Answer) -> Says {
        Says(a, std::sync::Mutex::new(0))
    }

    async fn run(caller: &Caller, terminal: Terminal, approver: &Says) -> Result<(), String> {
        let q = scan_question("Dana's MacBook", Path::new("/home/dana/dev"), caller);
        authorize("reins secrets scan", "Scan?", caller, terminal, HUMAN, &mut |_| Some("y".to_owned()), approver, &q)
            .await
    }

    #[tokio::test]
    async fn the_phone_is_the_lock() {
        // A pseudo-terminal (`script -qc`, `expect`) passes the terminal checks and types `y` itself: without the
        // phone's yes, nothing runs.
        let unanswered = says(Answer::Unanswered("No answer on your phone within 120 s.".to_owned()));
        let err = run(&Caller::Person, TTY, &unanswered).await.unwrap_err();
        assert!(err.contains("No answer"), "{err}");
        let denied = says(Answer::No("not now".to_owned()));
        assert!(run(&Caller::Person, TTY, &denied).await.unwrap_err().contains("said no"));
        let approved = says(Answer::Yes);
        assert!(run(&Caller::Person, TTY, &approved).await.is_ok());
        assert_eq!(*approved.1.lock().unwrap(), 1);
        // The courtesy checks still come first: no phone question without a terminal, or for a marked agent.
        let asked = says(Answer::Yes);
        let no_tty = Terminal {
            tty: false,
            ..TTY
        };
        assert!(run(&Caller::Person, no_tty, &asked).await.is_err());
        assert!(run(&Caller::Agent("CLAUDECODE".to_owned()), TTY, &asked).await.is_err());
        assert_eq!(*asked.1.lock().unwrap(), 0);
    }

    #[test]
    fn the_question_is_asked_every_time_and_says_where_and_who() {
        let q = scan_question("Dana's MacBook", Path::new("/home/dana/dev"), &Caller::Person);
        assert_eq!(q.question, "Scan Dana's MacBook for secrets?");
        // A `file:` topic: the phone never covers it with a standing answer or Autopilot.
        assert_eq!(q.topic.as_deref(), Some(SCAN_TOPIC));
        assert!(SCAN_TOPIC.starts_with("file:") && SCAN_TOPIC.contains("reins secrets scan"));
        let detail = q.detail.unwrap();
        assert!(detail.contains("/home/dana/dev") && detail.contains("names no AI agent"), "{detail}");
        let agent = scan_question("x", Path::new("/"), &Caller::Agent("REINS_RUNNER=agent:codex".to_owned()));
        assert!(agent.detail.unwrap().contains("AI agent runs it (REINS_RUNNER=agent:codex)"));
    }

    #[test]
    fn without_a_paired_phone_nothing_approves() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        let config = Config::default();
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let q = scan_question("x", Path::new("/"), &Caller::Person);
        let answer = rt.block_on(
            Phone {
                paths: &paths,
                config: &config,
            }
            .approve(&q),
        );
        assert!(matches!(answer, Answer::Unanswered(ref m) if m.contains("not paired")), "{answer:?}");
    }

    const TTY: Terminal = Terminal {
        stdin: true,
        stdout: true,
        tty: true,
    };
    const HUMAN: Needs = Needs {
        stdout: true,
    };

    fn yes(_: &str) -> Option<String> {
        Some("y".to_owned())
    }

    #[test]
    fn a_person_at_a_terminal_who_confirms_gets_through() {
        assert!(check("A scan", "Scan?", &Caller::Person, TTY, HUMAN, &mut yes).is_ok());
        let mut asked = String::new();
        let r = check("A scan", "Scan?", &Caller::Person, TTY, HUMAN, &mut |q| {
            asked = q.to_owned();
            Some("n".to_owned())
        });
        assert_eq!(r.unwrap_err(), "Nothing done.");
        assert_eq!(asked, "Scan? [y/N] ");
        assert!(check("A scan", "Scan?", &Caller::Person, TTY, HUMAN, &mut |_| None).is_err());
    }

    #[test]
    fn without_a_terminal_it_is_refused_even_with_the_markers_stripped() {
        // `env -u CLAUDECODE -u REINS_RUNNER reins secrets scan`: the caller looks like a person, but a tool call
        // has no terminal.
        for t in [
            Terminal {
                stdin: false,
                ..TTY
            },
            Terminal {
                tty: false,
                ..TTY
            },
            Terminal {
                stdout: false,
                ..TTY
            },
            Terminal {
                stdin: false,
                stdout: false,
                tty: false,
            },
        ] {
            let mut asked = false;
            let err = check("A secrets scan", "Scan?", &Caller::Person, t, HUMAN, &mut |_| {
                asked = true;
                Some("y".to_owned())
            })
            .unwrap_err();
            assert!(err.contains("your own terminal"), "{err}");
            assert!(!asked, "nothing is asked without a terminal");
        }
        // `--json` to a file: stdout may be a file, the rest still holds.
        let json = Needs {
            stdout: false,
        };
        let file = Terminal {
            stdout: false,
            ..TTY
        };
        assert!(check("A scan", "Scan?", &Caller::Person, file, json, &mut yes).is_ok());
        let no_tty = Terminal {
            tty: false,
            ..file
        };
        assert!(check("A scan", "Scan?", &Caller::Person, no_tty, json, &mut yes).is_err());
    }

    #[test]
    fn an_agent_is_refused_first() {
        let err = check("A scan", "Scan?", &Caller::Agent("CLAUDECODE".to_owned()), TTY, HUMAN, &mut yes).unwrap_err();
        assert!(err.contains("AI agent"), "{err}");
    }

    #[test]
    fn the_guard_asks_before_these_commands_run() {
        let g = crate::guard::GuardConfig::default();
        for c in [
            "reins secrets scan",
            "reins secrets scan --json ~/dev",
            "env -u CLAUDECODE -u REINS_RUNNER reins secrets scan",
            "reins secrets import --all-suggested",
            "reins secrets get vault:acme-web/development/OPENAI_API_KEY",
        ] {
            assert!(g.check_command(c).is_some(), "{c}");
        }
        assert!(g.check_command("reins status").is_none());
    }
}
