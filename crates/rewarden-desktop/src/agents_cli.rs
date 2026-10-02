//! The command line of the parts for AI harnesses: `rewarden ask`, `rewarden hook`, `rewarden harness` and `rewarden
//! mcp` (flattened into the main command line).

use std::io::{IsTerminal as _, Read as _, Write as _};
use std::process::ExitCode;
use std::time::Duration;

use clap::Subcommand;

use crate::ask::{Answer, Decider, DesktopAsk, Question};
use crate::config::{Config, Paths};
use crate::harness::{self, Harness, Setup};

/// Largest hook input read from stdin.
const MAX_HOOK_INPUT: u64 = 4 * 1024 * 1024;

#[derive(Subcommand)]
pub enum Command {
    /// Ask a yes-or-no question: on your phone when logged in, else here. Exit code 0 yes, 1 no, 2 no answer.
    Ask {
        question: String,
        /// What exactly would happen (`-` reads it from stdin).
        #[arg(long, value_name = "TEXT")]
        detail: Option<String>,
        /// What a standing answer on the phone may cover (`command:make deploy`).
        #[arg(long, value_name = "TOPIC")]
        topic: Option<String>,
        /// Seconds to wait for the answer (default: `approval_timeout_secs`).
        #[arg(long, value_name = "SECONDS")]
        timeout: Option<u64>,
    },
    /// A harness's pre-command hook (`rewarden harness add` sets it up): reads the hook's JSON on stdin, asks about
    /// risky commands and secret files (`[guard]` in the config), answers in the harness's format.
    Hook {
        /// claude-code, codex, gemini or cursor.
        harness: Harness,
    },
    /// Add Rewarden to an AI harness (its MCP server and, where the harness has them, its hook), remove it, or list.
    Harness {
        #[command(subcommand)]
        action: HarnessCmd,
    },
    /// A stdio MCP server that passes everything to your Rewarden server with this app's session.
    Mcp {
        /// Who asks, shown on your phone ("Claude Code").
        #[arg(long, value_name = "NAME")]
        via: Option<String>,
    },
}

#[derive(Clone, Copy, Subcommand)]
pub enum HarnessCmd {
    /// Register `rewarden mcp` and the hook in the harness's settings.
    Add {
        /// claude-code, codex, gemini or cursor.
        harness: Harness,
    },
    /// Take out exactly what `add` put in.
    Remove {
        harness: Harness,
    },
    /// What is set up (for one harness, or all).
    List {
        harness: Option<Harness>,
    },
}

/// `println!` that stops quietly when stdout is gone.
fn say(line: &str) {
    if writeln!(std::io::stdout(), "{line}").is_err() {
        std::process::exit(0);
    }
}

fn fail(e: &str) -> ExitCode {
    eprintln!("rewarden: {e}");
    ExitCode::FAILURE
}

fn setup(config: &Config) -> Result<Setup, String> {
    let home = crate::config::home_dir()?;
    Ok(Setup {
        home,
        exe: crate::update::current_executable()?,
        hook_timeout_secs: config.guard.timeout_secs,
    })
}

pub async fn run(cmd: Command) -> ExitCode {
    let paths = match Paths::from_env() {
        Ok(p) => p,
        Err(e) => return fail(&e.to_string()),
    };
    let config = match Config::load(&paths) {
        Ok(c) => c,
        Err(e) => {
            // A hook must still answer in its harness's terms: refuse, with the reason.
            if matches!(cmd, Command::Ask { .. } | Command::Hook { .. }) {
                eprintln!("rewarden: {e}");
                return ExitCode::from(2);
            }
            return fail(&e.to_string());
        }
    };
    match cmd {
        Command::Ask {
            question,
            detail,
            topic,
            timeout,
        } => ask(&paths, &config, &question, detail, topic.as_deref(), timeout).await,
        Command::Hook {
            harness,
        } => hook(&paths, &config, harness).await,
        Command::Harness {
            action,
        } => match harness_cmd(&paths, &config, action) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => fail(&e),
        },
        Command::Mcp {
            via,
        } => {
            crate::daemon::init_logging(None);
            let stdin = tokio::io::BufReader::new(tokio::io::stdin());
            match crate::mcp_bridge::run(&paths, via.as_deref(), stdin, tokio::io::stdout()).await {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => fail(&e),
            }
        }
    }
}

async fn ask(
    paths: &Paths,
    config: &Config,
    question: &str,
    detail: Option<String>,
    topic: Option<&str>,
    timeout: Option<u64>,
) -> ExitCode {
    let from_stdin = detail.as_deref() == Some("-");
    let detail = if from_stdin {
        let mut text = String::new();
        if let Err(e) = std::io::stdin().take(64 * 1024).read_to_string(&mut text) {
            eprintln!("rewarden: reading the detail from stdin: {e}");
            return ExitCode::from(2);
        }
        Some(text)
    } else {
        detail
    };
    let q = match Question::new(question, detail.as_deref(), topic) {
        Ok(q) => q,
        Err(e) => {
            eprintln!("rewarden: {e}");
            return ExitCode::from(2);
        }
    };
    let timeout = Duration::from_secs(timeout.unwrap_or(config.approval_timeout_secs).clamp(1, 3_600));
    let interactive = !from_stdin && std::io::stdin().is_terminal();
    match crate::ask::decider(paths, config) {
        Ok(Decider::Phone) => eprintln!("Asking on your phone…"),
        Ok(Decider::Local) if !interactive => eprintln!("Asking on the desktop…"),
        _ => {}
    }
    let answer = crate::ask::ask(paths, config, &q, timeout, interactive, &DesktopAsk).await;
    match &answer {
        Answer::Yes => eprintln!("Yes."),
        Answer::No(why) => eprintln!("No: {why}"),
        Answer::Unanswered(why) => eprintln!("No answer: {why}"),
    }
    ExitCode::from(answer.exit_code())
}

async fn hook(paths: &Paths, config: &Config, harness: Harness) -> ExitCode {
    let mut input = Vec::new();
    if let Err(e) = std::io::stdin().take(MAX_HOOK_INPUT).read_to_end(&mut input) {
        eprintln!("rewarden hook: reading stdin: {e}; not allowed.");
        return ExitCode::from(2);
    }
    let (out, code, err) = crate::hooks::run(harness, &input, paths, config, &DesktopAsk).await;
    if let Some(e) = err {
        eprintln!("{e}");
    }
    if let Some(out) = out {
        say(&out);
    }
    ExitCode::from(code)
}

fn harness_cmd(paths: &Paths, config: &Config, action: HarnessCmd) -> Result<(), String> {
    let s = setup(config)?;
    match action {
        HarnessCmd::Add {
            harness,
        } => {
            for line in harness::add(paths, &s, harness)? {
                say(&line);
            }
            say(harness::after_add_note(harness));
            if crate::server::oauth::logged_in_server(paths).is_none() {
                say("Not logged in yet: `rewarden login <server>` so the MCP server and the hook reach your phone.");
            }
        }
        HarnessCmd::Remove {
            harness,
        } => {
            let lines = harness::remove(paths, &s, harness)?;
            if lines.is_empty() {
                say(&format!("Nothing of Rewarden's was in {}'s settings.", harness.label()));
            }
            for line in lines {
                say(&line);
            }
        }
        HarnessCmd::List {
            harness,
        } => {
            let all = harness.map_or_else(|| Harness::ALL.to_vec(), |h| vec![h]);
            for h in all {
                for line in harness::list(paths, &s, h)? {
                    say(&line);
                }
            }
        }
    }
    Ok(())
}
