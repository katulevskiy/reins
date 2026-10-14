//! Only a person runs `reins secrets scan` and `import`, at their own terminal. Three locks, each one an agent cannot
//! take off by itself:
//!
//! 1. The guard's built-in rules (`guard::GROUPS`, `secrets`) ask the phone before any harness that has Reins' hooks
//!    runs `reins secrets scan`, `import` or `get`, whatever the command's environment says.
//! 2. These commands need a terminal: stdin a terminal (and stdout too, where the output is for reading), which an
//!    agent's tool call does not have.
//! 3. They ask the person to type a confirmation on the terminal itself (`/dev/tty`, the Windows console), not on
//!    stdin.
//!
//! The environment's markers ([`Caller`]) are checked first, as a courtesy: they say early and clearly why an agent
//! cannot do this, but an agent can unset them, so they are never the only lock.

use std::io::{IsTerminal as _, Write as _};

use crate::secrets_scan::Caller;

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
    confirm: &mut dyn FnMut(&str) -> Option<String>,
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

#[cfg(test)]
mod tests {
    use super::*;

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
