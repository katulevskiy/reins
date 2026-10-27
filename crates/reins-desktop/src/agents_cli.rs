//! The command line of the parts for AI harnesses: `reins ask`, `reins hook`, `reins harness` and `reins
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
    /// Ask a yes-or-no question: on your phone when logged in, else here. Exit code 0 yes, 1 no, 2 no answer, 3
    /// parked for a later review (the ticket on stdout; `reins wait` it, then ask again).
    #[command(display_order = 11)]
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
        /// Do not wait: park the question for a later review on your phone and print its ticket.
        #[arg(long)]
        defer: bool,
    },
    /// Questions and pushes parked for a later review on your phone, and where each stands.
    #[command(display_order = 12)]
    Tickets {
        /// The phone's signing key changed: trust the new one, after comparing its digits with the phone (the
        /// connection's More).
        #[arg(long)]
        trust_key: bool,
    },
    /// Wait for a parked question or push to be decided. Exit code 0 approved (or pushed), 1 denied, 2 expired,
    /// stale, cancelled or out of time.
    #[command(display_order = 13)]
    Wait {
        /// The ticket (`rt_…`).
        ticket: String,
        /// Seconds to wait at most (default 600).
        #[arg(long, value_name = "SECONDS")]
        timeout: Option<u64>,
    },
    /// A harness's pre-command hook (`reins harness add` sets it up): reads the hook's JSON on stdin, asks about
    /// risky commands and secret files (`[guard]` in the config), answers in the harness's format.
    #[command(hide = true)]
    Hook {
        /// claude-code, codex, gemini or cursor.
        harness: Harness,
    },
    /// Add Reins to an AI harness (its MCP server and, where the harness has them, its hook), remove it, or list.
    #[command(display_order = 6)]
    Harness {
        #[command(subcommand)]
        action: HarnessCmd,
    },
    /// A stdio MCP server that passes everything to your Reins server with this app's session.
    #[command(hide = true)]
    Mcp {
        /// Who asks, shown on your phone ("Claude Code").
        #[arg(long, value_name = "NAME")]
        via: Option<String>,
    },
}

#[derive(Clone, Copy, Subcommand)]
pub enum HarnessCmd {
    /// Register `reins mcp` and the hook in the harness's settings.
    Add {
        /// claude-code, codex, gemini or cursor.
        #[arg(required_unless_present = "all", conflicts_with = "all")]
        harness: Option<Harness>,
        /// Every harness found on this computer (settings directory or program installed).
        #[arg(long)]
        all: bool,
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
    eprintln!("reins: {e}");
    ExitCode::FAILURE
}

/// Where the harnesses' settings are and what they run, for this program and `config`.
pub fn setup(config: &Config) -> Result<Setup, String> {
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
                eprintln!("reins: {e}");
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
            defer,
        } => ask(&paths, &config, &question, detail, topic.as_deref(), timeout, defer).await,
        Command::Tickets {
            trust_key,
        } => tickets(&paths, &config, trust_key).await,
        Command::Wait {
            ticket,
            timeout,
        } => wait(&paths, &config, &ticket, timeout).await,
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
    defer: bool,
) -> ExitCode {
    let from_stdin = detail.as_deref() == Some("-");
    let detail = if from_stdin {
        let mut text = String::new();
        if let Err(e) = std::io::stdin().take(64 * 1024).read_to_string(&mut text) {
            eprintln!("reins: reading the detail from stdin: {e}");
            return ExitCode::from(2);
        }
        Some(text)
    } else {
        detail
    };
    let q = match Question::new(question, detail.as_deref(), topic) {
        Ok(q) => q,
        Err(e) => {
            eprintln!("reins: {e}");
            return ExitCode::from(2);
        }
    };
    let timeout = Duration::from_secs(timeout.unwrap_or(config.approval_timeout_secs).clamp(1, 3_600));
    let interactive = !from_stdin && std::io::stdin().is_terminal();
    let secs = timeout.as_secs();
    let phone = matches!(crate::ask::decider(paths, config), Ok(Decider::Phone));
    // The same question asked again where it was parked: an approval since lets it through once.
    let cwd = std::env::current_dir().ok().map(|d| d.display().to_string());
    // Bound to the question as typed; one cut to reach the phone never becomes a ticket.
    let digest = crate::ask::reaches_whole(question, detail.as_deref(), topic)
        .then(|| crate::tickets::ask_digest(cwd.as_deref(), question, detail.as_deref(), topic));
    if phone && let Some(digest) = &digest {
        let now = crate::now_unix();
        if crate::tickets::TicketStore::load(paths).passes().iter().any(|p| p.digest == *digest && p.session.is_none())
            && let Ok(Some(pass)) = crate::tickets::TicketStore::update(paths, |s| s.take_pass(digest, None, now))
        {
            eprintln!("Yes (approved earlier, ticket {}).", pass.ticket);
            return ExitCode::SUCCESS;
        }
    }
    match crate::ask::decider(paths, config) {
        Ok(Decider::Phone) if defer => eprintln!("Parking it for a later review on your phone…"),
        Ok(Decider::Phone) => eprintln!("Asking on your phone; waiting up to {secs} s…"),
        Ok(Decider::Local) if !interactive => eprintln!(
            "Asking on this computer (a desktop notification; not paired with a phone); waiting up to {secs} s…"
        ),
        _ => {}
    }
    let entry = crate::journal::Entry::new(crate::journal::Kind::Ask, &q.question)
        .source(Some("reins ask"))
        .service(q.topic.as_deref())
        .detail(q.detail.as_deref());
    let tell = |line: &str| eprintln!("{line}");
    if defer && digest.is_none() {
        eprintln!("reins: this question is too long to park whole; asking it now instead.");
    }
    let ticketing = digest.as_deref().map(|digest| crate::ask::Ticketing {
        digest,
        defer,
    });
    let (answer, timed_out, _entry) = crate::ask::ask_logged_ticketed(
        paths,
        config,
        &q,
        timeout,
        interactive,
        &DesktopAsk,
        entry,
        Some(&tell),
        ticketing.filter(|_| phone),
    )
    .await;
    let answer = match answer {
        Ok(a) => a,
        Err(parked) => {
            let now = crate::now_unix();
            let ticket = crate::tickets::LocalTicket {
                ticket: parked.ticket.clone(),
                kind: crate::tickets::TicketKind::Ask,
                session: None,
                summary: crate::tickets::summary_of(&q.question),
                digest: digest.unwrap_or_default(),
                nonce: parked.nonce,
                created_at: now,
                expected_by: parked.expected_by,
                state: reins_proto::ticket::TicketState::Deferred.as_str().to_owned(),
                told: None,
                repo: None,
                refs: Vec::new(),
                note: None,
                stale: None,
                push: None,
            };
            if let Err(e) = crate::tickets::TicketStore::update(paths, |s| s.add(ticket)) {
                eprintln!("reins: {e}");
            }
            eprintln!(
                "{} `reins wait {}` waits for it; then ask again.",
                crate::tickets::parked_hook_message(&parked.ticket, parked.expected_by, now)
                    .replace("re-run this exact command", "ask this exact question again"),
                parked.ticket
            );
            say(&parked.ticket);
            return ExitCode::from(3);
        }
    };
    match &answer {
        Answer::Yes => eprintln!("Yes."),
        Answer::No(why) => eprintln!("No: {why}"),
        Answer::Unanswered(why) if timed_out => eprintln!("Timed out: {why}"),
        Answer::Unanswered(why) => eprintln!("No answer: {why}"),
    }
    ExitCode::from(answer.exit_code())
}

/// While the phone's signing key changed and is not trusted: says so (both fingerprints) and the exit code 2.
fn refused_for_key(paths: &Paths) -> Option<ExitCode> {
    let (was, now) = crate::tickets::TicketStore::load(paths).key_change()?;
    eprintln!("reins: {}", crate::tickets::key_changed_message(&was, &now));
    Some(ExitCode::from(2))
}

/// `reins tickets`: this computer's tickets, after a look at where they stand now; `trust_key`: pin the phone's
/// changed signing key first.
async fn tickets(paths: &Paths, config: &Config, trust_key: bool) -> ExitCode {
    use crate::tickets::TicketStore;
    if trust_key {
        match TicketStore::update(paths, TicketStore::trust_key) {
            Ok(Some(_)) => {}
            Ok(None) => say("The phone's signing key has not changed; nothing to trust."),
            Err(e) => return fail(&e),
        }
    }
    if crate::server::oauth::logged_in_server(paths).is_some() {
        let pushes = !crate::tickets::daemon_running(paths, config).await;
        match crate::server::client::DesktopClient::new(paths) {
            Ok(client) => {
                let look = async {
                    if TicketStore::load(paths).needs_look() {
                        crate::tickets::refresh(paths, &client, pushes).await.map(|_| ())
                    } else {
                        crate::tickets::see_signing_key(paths, &client).await
                    }
                };
                match tokio::time::timeout(Duration::from_secs(20), look).await {
                    Ok(Ok(())) => {}
                    Ok(Err(e)) => eprintln!("reins: cannot look at the tickets now: {e}"),
                    Err(_) => eprintln!("reins: the Reins server did not answer in time; showing what is known."),
                }
            }
            Err(e) => eprintln!("reins: {e}"),
        }
    }
    if let Some(code) = refused_for_key(paths) {
        return code;
    }
    let store = TicketStore::load(paths);
    if let Some(fp) = store.signing_key_fingerprint() {
        say(&format!("Phone key {fp}"));
    }
    let mut list: Vec<_> = store.tickets().iter().collect();
    if list.is_empty() {
        say("No tickets: nothing waits for a later review.");
        return ExitCode::SUCCESS;
    }
    list.sort_by_key(|t| std::cmp::Reverse(t.created_at));
    let now = crate::now_unix();
    say(&format!("{:<24} {:<5} {:<12} {:<10} WHAT", "TICKET", "KIND", "STATE", "DUE"));
    for t in list {
        let due = if t.waits() && t.expected_by > now {
            format!("in {}", crate::tickets::within(t.expected_by - now))
        } else {
            "-".to_owned()
        };
        let mut what = t.summary.clone();
        if let Some(n) = t.note.as_deref().or(t.stale.as_deref()) {
            what = format!("{what} ({n})");
        }
        say(&format!("{:<24} {:<5} {:<12} {:<10} {what}", t.ticket, t.kind.label(), t.state.replace('_', " "), due));
    }
    ExitCode::SUCCESS
}

/// `reins wait <ticket>`.
async fn wait(paths: &Paths, config: &Config, ticket: &str, timeout: Option<u64>) -> ExitCode {
    let Some(id) = reins_proto::ticket::request_of(ticket.trim()) else {
        eprintln!("reins: `{ticket}` is not a ticket (rt_…)");
        return ExitCode::from(2);
    };
    let ticket = reins_proto::ticket::ticket_of(&id);
    if crate::server::oauth::logged_in_server(paths).is_none() {
        eprintln!("reins: not logged in to a Reins server; run `reins login`");
        return ExitCode::from(2);
    }
    let client = match crate::server::client::DesktopClient::new(paths) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("reins: {e}");
            return ExitCode::from(2);
        }
    };
    let timeout = Duration::from_secs(timeout.unwrap_or(reins_proto::ticket::MAX_WAIT_SECS).clamp(1, 86_400));
    // The daemon pushes parked pushes; without it, waiting for one pushes it here.
    let pushes = !crate::tickets::daemon_running(paths, config).await;
    let code = crate::tickets::wait(paths, &client, &ticket, timeout, pushes).await;
    if let Some(code) = refused_for_key(paths) {
        return code;
    }
    let store = crate::tickets::TicketStore::load(paths);
    let known = store.get(&ticket);
    if known.is_none() {
        eprintln!("{ticket}: not a ticket of this computer; `reins tickets` lists them.");
        return ExitCode::from(code);
    }
    match (code, known.and_then(crate::tickets::LocalTicket::state)) {
        (0, _) => eprintln!("{ticket}: approved."),
        (1, _) => eprintln!(
            "{ticket}: denied{}",
            known.and_then(|t| t.note.as_deref()).map_or_else(|| ".".to_owned(), |n| format!(": {n}"))
        ),
        (_, Some(s)) if !s.is_open() => eprintln!("{ticket}: {}.", s.as_str().replace('_', " ")),
        _ => eprintln!("{ticket}: not decided within {} s.", timeout.as_secs()),
    }
    ExitCode::from(code)
}

async fn hook(paths: &Paths, config: &Config, harness: Harness) -> ExitCode {
    let mut input = Vec::new();
    if let Err(e) = std::io::stdin().take(MAX_HOOK_INPUT).read_to_end(&mut input) {
        eprintln!("reins hook: reading stdin: {e}; not allowed.");
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

/// `set up`, `partly set up`, `not set up` or `not installed`, for the one-line list.
fn state_word(paths: &Paths, s: &Setup, h: Harness) -> &'static str {
    match harness::registered(paths, s, h) {
        Ok(r) if r.complete() => "set up",
        Ok(r) if r.any() => "partly set up (`reins harness add` repairs it)",
        _ if harness::detect::found(h, &s.home) => "not set up",
        _ => "not installed",
    }
}

fn harness_cmd(paths: &Paths, config: &Config, action: HarnessCmd) -> Result<(), String> {
    let s = setup(config)?;
    match action {
        HarnessCmd::Add {
            harness,
            all,
        } => {
            let chosen = match harness {
                Some(h) if !all => vec![h],
                _ => harness::detect::all_found(&s.home),
            };
            if chosen.is_empty() {
                say("No AI harness found on this computer (Claude Code, Codex, Gemini CLI, Cursor).");
            }
            for h in chosen {
                if all {
                    say(&format!("{}:", h.label()));
                }
                for line in harness::add(paths, &s, h)? {
                    say(&line);
                }
                say(harness::after_add_note(h));
            }
            if crate::server::oauth::logged_in_server(paths).is_none() {
                say("Not logged in yet: `reins login` so the MCP server and the hook reach your phone.");
            }
        }
        HarnessCmd::Remove {
            harness,
        } => {
            let lines = harness::remove(paths, &s, harness)?;
            if lines.is_empty() {
                say(&format!("Nothing of Reins's was in {}'s settings.", harness.label()));
            }
            for line in lines {
                say(&line);
            }
        }
        HarnessCmd::List {
            harness,
        } => {
            if let Some(h) = harness {
                // One harness: every part, with its file.
                for line in harness::list(paths, &s, h)? {
                    say(&line);
                }
                if !harness::detect::found(h, &s.home) {
                    say("  (not found on this computer)");
                }
            } else {
                for h in Harness::ALL {
                    say(&format!("{:<12} {:<12} {}", h.id(), h.label(), state_word(paths, &s, h)));
                }
                say("`reins harness list <name>` shows each part and its file.");
            }
        }
    }
    Ok(())
}
