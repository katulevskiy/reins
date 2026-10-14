//! Who runs a command: a person, or an AI agent (`agent:<harness>`). The daemon decides it from facts it can check,
//! never from what the caller says:
//!
//! - the caller is the process on the other end of the daemon's run socket (its peer credentials: the same user, and
//!   its process id);
//! - that process and every ancestor of it are looked at: one that is a known agent harness (Claude Code, Codex,
//!   Gemini CLI, Cursor's agent, as their program or as the script node runs) makes the caller that agent;
//! - what the caller says (`REINS_RUNNER`, the harnesses' own markers in its environment) can only make it an agent,
//!   never a person.
//!
//! The limit: a process of the same user that detaches from its ancestry completely (re-parented to init through a
//! double fork, or started by a service of the user) and does not say it is an agent passes for a person. Running the
//! daemon as its own user is what closes that (security-model.md, same-user limits); the phone's approval, which names
//! the exact command, is the backstop.

use std::collections::BTreeMap;

use reins_proto::desktop::PERSON;

/// The most ancestors looked at.
const MAX_DEPTH: usize = 64;

/// One process as the walker sees it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProcessInfo {
    pub parent: Option<u32>,
    /// The program's file name (`/proc/<pid>/exe`'s, else the first argument's).
    pub program: String,
    /// The arguments, the program first.
    pub args: Vec<String>,
}

/// The harness a process is, if it is one: by its program, or by the script an interpreter runs.
#[must_use]
pub fn harness_of(p: &ProcessInfo) -> Option<&'static str> {
    let name = |s: &str| {
        let file = s.rsplit(['/', '\\']).next().unwrap_or(s).to_ascii_lowercase();
        file.strip_suffix(".exe").map(str::to_owned).unwrap_or(file)
    };
    let by_name = |n: &str| match n {
        "claude" | "claude-code" => Some("claude-code"),
        "codex" | "codex-exec" => Some("codex"),
        "gemini" => Some("gemini"),
        "cursor-agent" => Some("cursor"),
        _ => None,
    };
    if let Some(h) = by_name(&name(&p.program)) {
        return Some(h);
    }
    let first = p.args.first().map(|a| name(a)).unwrap_or_default();
    if let Some(h) = by_name(&first) {
        return Some(h);
    }
    // node, bun, deno, python running a harness's entry point.
    let joined = p.args.iter().skip(1).take(4).map(|a| a.to_ascii_lowercase()).collect::<Vec<_>>().join(" ");
    for (needle, harness) in [
        ("@anthropic-ai/claude-code", "claude-code"),
        ("@openai/codex", "codex"),
        ("@google/gemini-cli", "gemini"),
        ("cursor-agent", "cursor"),
    ] {
        if joined.contains(needle) {
            return Some(harness);
        }
    }
    p.args.iter().skip(1).take(4).find_map(|a| by_name(&name(a)))
}

/// Environment variables a harness sets for what it runs, and the harness each names.
const MARKERS: &[(&str, &str)] = &[
    ("CLAUDECODE", "claude-code"),
    ("CLAUDE_CODE_ENTRYPOINT", "claude-code"),
    ("CODEX_SANDBOX", "codex"),
    ("GEMINI_CLI", "gemini"),
    ("CURSOR_AGENT", "cursor"),
];

/// What the caller's environment says, which can only make it an agent: `REINS_RUNNER=agent:<h>` (any other value
/// but `person`, or a malformed one, counts as an unnamed agent), or a harness's marker.
#[must_use]
pub fn claimed(env: &BTreeMap<String, String>) -> Option<String> {
    if let Some(r) = env.get("REINS_RUNNER").map(|r| r.trim()).filter(|r| !r.is_empty() && *r != PERSON) {
        return Some(if reins_proto::desktop::runner_ok(r) {
            r.to_owned()
        } else {
            "agent:unknown".to_owned()
        });
    }
    MARKERS.iter().find(|(k, _)| env.get(*k).is_some_and(|v| !v.is_empty())).map(|(_, h)| format!("agent:{h}"))
}

/// The runner of the process `pid`: the nearest harness among it and its ancestors, else what its environment says,
/// else a person. `info` reads one process (`None`: gone, or not readable).
#[must_use]
pub fn runner_of(pid: u32, env: &BTreeMap<String, String>, info: &dyn Fn(u32) -> Option<ProcessInfo>) -> String {
    let mut at = Some(pid);
    for _ in 0..MAX_DEPTH {
        let Some(current) = at.filter(|p| *p > 1) else {
            break;
        };
        let Some(p) = info(current) else {
            break;
        };
        if let Some(h) = harness_of(&p) {
            return format!("agent:{h}");
        }
        at = p.parent;
    }
    claimed(env).unwrap_or_else(|| PERSON.to_owned())
}

/// How the daemon reads a process (tests give their own).
pub type ProcessReader = fn(u32) -> Option<ProcessInfo>;

/// One process of this computer, as the operating system tells it.
#[must_use]
pub fn process_info(pid: u32) -> Option<ProcessInfo> {
    os::process_info(pid)
}

/// The environment of process `pid`, when this user may read it (Linux).
#[must_use]
pub fn process_env(pid: u32) -> BTreeMap<String, String> {
    os::process_env(pid)
}

#[cfg(target_os = "linux")]
mod os {
    use std::collections::BTreeMap;

    use super::ProcessInfo;

    pub(super) fn process_info(pid: u32) -> Option<ProcessInfo> {
        let dir = std::path::PathBuf::from(format!("/proc/{pid}"));
        let stat = std::fs::read_to_string(dir.join("stat")).ok()?;
        // `pid (comm) state ppid …`: the name may hold spaces and parentheses, so read after the last `)`.
        let after = stat.rsplit_once(')')?.1;
        let parent = after.split_whitespace().nth(1)?.parse().ok();
        let args: Vec<String> = std::fs::read(dir.join("cmdline"))
            .ok()?
            .split(|b| *b == 0)
            .filter(|a| !a.is_empty())
            .map(|a| String::from_utf8_lossy(a).into_owned())
            .collect();
        let program = std::fs::read_link(dir.join("exe"))
            .ok()
            .map(|p| p.to_string_lossy().into_owned())
            .or_else(|| args.first().cloned())
            .unwrap_or_default();
        Some(ProcessInfo {
            parent,
            program,
            args,
        })
    }

    pub(super) fn process_env(pid: u32) -> BTreeMap<String, String> {
        std::fs::read(format!("/proc/{pid}/environ"))
            .unwrap_or_default()
            .split(|b| *b == 0)
            .filter_map(|kv| {
                let kv = String::from_utf8_lossy(kv);
                let (k, v) = kv.split_once('=')?;
                Some((k.to_owned(), v.to_owned()))
            })
            .collect()
    }
}

#[cfg(target_os = "macos")]
mod os {
    use std::collections::BTreeMap;

    use super::ProcessInfo;

    /// `ps` says the parent and the arguments (macOS has no `/proc`).
    pub(super) fn process_info(pid: u32) -> Option<ProcessInfo> {
        let out = std::process::Command::new("/bin/ps")
            .args(["-o", "ppid=", "-o", "comm=", "-p", &pid.to_string()])
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        let line = text.lines().next()?.trim();
        let (ppid, comm) = line.split_once(char::is_whitespace)?;
        let args_out =
            std::process::Command::new("/bin/ps").args(["-o", "args=", "-p", &pid.to_string()]).output().ok()?;
        let args: Vec<String> =
            String::from_utf8_lossy(&args_out.stdout).split_whitespace().map(str::to_owned).collect();
        Some(ProcessInfo {
            parent: ppid.trim().parse().ok(),
            program: comm.trim().to_owned(),
            args,
        })
    }

    pub(super) fn process_env(_pid: u32) -> BTreeMap<String, String> {
        BTreeMap::new()
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
mod os {
    use std::collections::BTreeMap;

    use super::ProcessInfo;

    pub(super) fn process_info(_pid: u32) -> Option<ProcessInfo> {
        None
    }

    pub(super) fn process_env(_pid: u32) -> BTreeMap<String, String> {
        BTreeMap::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(parent: u32, program: &str, args: &[&str]) -> ProcessInfo {
        ProcessInfo {
            parent: Some(parent),
            program: program.to_owned(),
            args: args.iter().map(|a| (*a).to_owned()).collect(),
        }
    }

    fn tree(procs: Vec<(u32, ProcessInfo)>) -> impl Fn(u32) -> Option<ProcessInfo> {
        let map: BTreeMap<u32, ProcessInfo> = procs.into_iter().collect();
        move |pid| map.get(&pid).cloned()
    }

    #[test]
    fn any_harness_among_the_ancestors_makes_the_caller_that_agent() {
        let none = BTreeMap::new();
        let claude = tree(vec![
            (40, p(30, "/usr/bin/reins", &["reins", "run", "--", "npm", "test"])),
            (30, p(20, "/bin/bash", &["bash", "-c", "reins run -- npm test"])),
            (20, p(10, "/usr/bin/node", &["node", "/usr/lib/node_modules/@anthropic-ai/claude-code/cli.js"])),
            (10, p(1, "/usr/bin/zsh", &["-zsh"])),
        ]);
        assert_eq!(runner_of(40, &none, &claude), "agent:claude-code");
        let codex = tree(vec![(5, p(4, "/bin/sh", &["sh"])), (4, p(1, "/home/me/.local/bin/codex", &["codex"]))]);
        assert_eq!(runner_of(5, &none, &codex), "agent:codex");
        let cursor = tree(vec![(5, p(1, "/x/cursor-agent", &["cursor-agent"]))]);
        assert_eq!(runner_of(5, &none, &cursor), "agent:cursor");
        let gemini = tree(vec![(5, p(1, "/usr/bin/node", &["node", "/opt/@google/gemini-cli/dist/index.js"]))]);
        assert_eq!(runner_of(5, &none, &gemini), "agent:gemini");
    }

    #[test]
    fn a_terminal_without_a_harness_is_a_person_and_saying_so_changes_nothing() {
        let terminal =
            tree(vec![(40, p(30, "/usr/bin/reins", &["reins", "run"])), (30, p(1, "/usr/bin/zsh", &["-zsh"]))]);
        assert_eq!(runner_of(40, &BTreeMap::new(), &terminal), PERSON);
        // An agent's process that unset its markers still has the harness above it.
        let agent = tree(vec![(40, p(30, "/usr/bin/reins", &["reins"])), (30, p(1, "/usr/bin/claude", &["claude"]))]);
        let says_person = BTreeMap::from([("REINS_RUNNER".to_owned(), "person".to_owned())]);
        assert_eq!(runner_of(40, &says_person, &agent), "agent:claude-code", "never upgraded to a person");
    }

    #[test]
    fn what_the_caller_says_can_only_make_it_an_agent() {
        let terminal = tree(vec![(40, p(1, "/usr/bin/reins", &["reins"]))]);
        let env = |k: &str, v: &str| BTreeMap::from([(k.to_owned(), v.to_owned())]);
        assert_eq!(runner_of(40, &env("REINS_RUNNER", "agent:codex"), &terminal), "agent:codex");
        assert_eq!(runner_of(40, &env("REINS_RUNNER", "robot"), &terminal), "agent:unknown");
        assert_eq!(runner_of(40, &env("CLAUDECODE", "1"), &terminal), "agent:claude-code");
        assert_eq!(runner_of(40, &env("CLAUDECODE", ""), &terminal), PERSON);
        // A process that cannot be read is judged by what it says.
        assert_eq!(runner_of(99, &BTreeMap::new(), &terminal), PERSON);
    }

    #[test]
    fn programs_that_merely_resemble_a_harness_are_not_one() {
        for (program, args) in [
            ("/usr/bin/node", vec!["node", "server.js"]),
            ("/Applications/Cursor.app/Contents/MacOS/Cursor", vec!["Cursor"]),
            ("/usr/bin/vim", vec!["vim", "codex-notes.md"]),
        ] {
            let args: Vec<&str> = args;
            assert_eq!(harness_of(&p(1, program, &args)), None, "{program}");
        }
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn this_process_can_be_read() {
        let me = process_info(std::process::id()).unwrap();
        assert!(me.parent.is_some() && !me.program.is_empty());
    }
}
