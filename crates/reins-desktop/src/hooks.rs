//! `reins hook <harness>`: a harness's pre-tool hook. Reads the hook's JSON on stdin, and when the command or file
//! matches the guard rules (`[guard]`) asks the phone (or the person at the computer) and answers in that harness's
//! format; anything else gets no decision, so the harness's own permission settings apply.
//!
//! Formats (each harness's current docs):
//! - Claude Code `PreToolUse` (<https://code.claude.com/docs/en/hooks>): `tool_name` `Bash` (`tool_input.command`),
//!   `Edit`/`Write`/`MultiEdit`/`Read` (`tool_input.file_path`), `NotebookEdit` (`notebook_path`); answer
//!   `{"hookSpecificOutput": {"hookEventName": "PreToolUse", "permissionDecision": "allow"|"deny"|"ask",
//!   "permissionDecisionReason": …}}`, no output for no decision.
//! - Codex `PreToolUse` (<https://learn.chatgpt.com/docs/hooks>): same shape; `Bash` (`tool_input.command`) and
//!   `apply_patch` (the patch in `tool_input.command`); `ask` is not supported, so "leave it to the harness" is no
//!   output.
//! - Cursor (<https://cursor.com/docs/agent/hooks>): `beforeShellExecution` (`command`; answer `permission`
//!   `allow`/`deny`/`ask` with `user_message`, `agent_message`), `beforeReadFile` (`file_path`) and `preToolUse`
//!   (`tool_name` `Write`/`Delete`, `tool_input.file_path`), which know `allow`/`deny` only. `allow` lets the action
//!   run without Cursor's own approval (that is what `ask` is for), so it is only said for an approved request. No
//!   output is no decision: Cursor counts it as a failed hook, which fails open (unless the hook sets `failClosed`,
//!   which `reins harness add` does not) and leaves the action to Cursor's own settings. The exception is
//!   `beforeReadFile`: Cursor never asks before reading, so an explicit `allow` takes nothing from the user there.
//! - Gemini CLI `BeforeTool` (<https://geminicli.com/docs/hooks/reference/>): `run_shell_command`
//!   (`tool_input.command`), `write_file`/`replace`/`read_file` (`tool_input.file_path`); answer `{"decision":
//!   "allow"|"deny", "reason": …}`, no output for no decision.
//!
//! Deferred decisions ([`crate::tickets`]): a question the reviewer will decide later is denied with its ticket and
//! the advice to re-run the exact command once approved; once approved, that command goes through once (a pass).
//! Claude Code also calls the hook on `UserPromptSubmit` and `Stop`: then, and on `PreToolUse`, the session hears what
//! changed for its tickets (`hookSpecificOutput.additionalContext`; on `Stop`, `{"decision": "block", "reason": …}`,
//! only when there is news).

use std::fmt::Write as _;
use std::path::Path;
use std::time::Duration;

use serde_json::{Value, json};

use crate::ask::{Answer, Question};
use crate::auth::prompt::Prompter;
use crate::config::{Config, Paths};
use crate::guard::{GuardConfig, Match, OnNoAnswer};
use crate::harness::Harness;
use crate::journal::{Entry, Kind, Outcome};

/// What the tool is about to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Command(String),
    /// Read (`write: false`) or change files.
    Files {
        write: bool,
        paths: Vec<String>,
    },
    /// Nothing the guard looks at.
    Other,
}

/// Which hook of the harness called.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// Claude Code or Codex `PreToolUse`.
    PreToolUse,
    /// Cursor's `beforeShellExecution`: allow, deny or ask.
    CursorShell,
    /// Cursor's `beforeReadFile`: allow or deny only.
    CursorRead,
    /// Cursor's `preToolUse`: allow or deny only.
    CursorTool,
    GeminiBeforeTool,
    /// Claude Code `UserPromptSubmit`: news of the session's tickets only.
    UserPromptSubmit,
    /// Claude Code `Stop`: news of the session's tickets only, as a reason to go on.
    Stop,
    /// An event this app does not handle (answered with no decision).
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HookCall {
    pub harness: Harness,
    pub event: Event,
    pub action: Action,
    pub cwd: Option<String>,
}

/// The decision, before it is put in the harness's words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Nothing to ask about.
    Unmatched,
    Allow(String),
    Deny(String),
    /// Leave it to the harness's own prompt.
    Ask(String),
}

fn str_at<'a>(v: &'a Value, pointer: &str) -> Option<&'a str> {
    v.pointer(pointer).and_then(Value::as_str)
}

fn file_in(input: &Value) -> Vec<String> {
    ["/file_path", "/absolute_path", "/notebook_path", "/path"]
        .iter()
        .find_map(|p| str_at(input, p))
        .map(|p| vec![p.to_owned()])
        .unwrap_or_default()
}

/// The files an `apply_patch` patch touches (`*** Add File: …`, `*** Update File: …`, `*** Delete File: …`,
/// `*** Move to: …`).
fn patch_files(patch: &str) -> Vec<String> {
    patch
        .lines()
        .filter_map(|l| {
            let l = l.trim_start();
            ["*** Add File: ", "*** Update File: ", "*** Delete File: ", "*** Move to: "]
                .iter()
                .find_map(|p| l.strip_prefix(p))
                .map(|p| p.trim().to_owned())
        })
        .filter(|p| !p.is_empty())
        .collect()
}

/// Reads one hook call of `harness` from its stdin JSON.
pub fn parse(harness: Harness, input: &[u8]) -> Result<HookCall, String> {
    let v: Value = serde_json::from_slice(input).map_err(|e| format!("the hook input is not JSON: {e}"))?;
    if !v.is_object() {
        return Err("the hook input is not a JSON object".to_owned());
    }
    let event_name = str_at(&v, "/hook_event_name").unwrap_or_default();
    let tool = str_at(&v, "/tool_name").unwrap_or_default();
    let tool_input = v.get("tool_input").cloned().unwrap_or(Value::Null);
    let command = || str_at(&tool_input, "/command").map_or(Action::Other, |c| Action::Command(c.to_owned()));
    let files = |write: bool, paths: Vec<String>| {
        if paths.is_empty() {
            Action::Other
        } else {
            Action::Files {
                write,
                paths,
            }
        }
    };
    let (event, action) = match harness {
        Harness::ClaudeCode | Harness::Codex => {
            let event = match event_name {
                "" | "PreToolUse" => Event::PreToolUse,
                "UserPromptSubmit" if harness == Harness::ClaudeCode => Event::UserPromptSubmit,
                "Stop" if harness == Harness::ClaudeCode => Event::Stop,
                _ => Event::Unknown,
            };
            let action = match tool {
                "Bash" | "shell" | "local_shell" | "exec_command" => command(),
                "apply_patch" => files(
                    true,
                    str_at(&tool_input, "/command")
                        .or_else(|| str_at(&tool_input, "/patch"))
                        .or_else(|| str_at(&tool_input, "/input"))
                        .map(patch_files)
                        .unwrap_or_default(),
                ),
                "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => files(true, file_in(&tool_input)),
                "Read" => files(false, file_in(&tool_input)),
                _ => Action::Other,
            };
            (event, action)
        }
        Harness::Cursor => match event_name {
            "beforeShellExecution" => {
                (Event::CursorShell, str_at(&v, "/command").map_or(Action::Other, |c| Action::Command(c.to_owned())))
            }
            "beforeReadFile" => (Event::CursorRead, files(false, file_in(&v))),
            "preToolUse" => {
                // Cursor may send `tool_input` as an object or as JSON text.
                let ti = match &tool_input {
                    Value::String(s) => serde_json::from_str(s).unwrap_or(Value::Null),
                    other => other.clone(),
                };
                let action = match tool {
                    "Write" | "Delete" | "Edit" => files(true, file_in(&ti)),
                    "Read" => files(false, file_in(&ti)),
                    "Shell" => str_at(&ti, "/command").map_or(Action::Other, |c| Action::Command(c.to_owned())),
                    _ => Action::Other,
                };
                (Event::CursorTool, action)
            }
            _ => (Event::Unknown, Action::Other),
        },
        Harness::Gemini => {
            let event = if event_name.is_empty() || event_name == "BeforeTool" {
                Event::GeminiBeforeTool
            } else {
                Event::Unknown
            };
            let action = match tool {
                "run_shell_command" => command(),
                "write_file" | "replace" | "edit" => files(true, file_in(&tool_input)),
                "read_file" => files(false, file_in(&tool_input)),
                "read_many_files" => files(
                    false,
                    tool_input
                        .get("paths")
                        .and_then(Value::as_array)
                        .map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect())
                        .unwrap_or_default(),
                ),
                _ => Action::Other,
            };
            (event, action)
        }
    };
    Ok(HookCall {
        harness,
        event,
        action: if matches!(event, Event::Unknown | Event::UserPromptSubmit | Event::Stop) {
            Action::Other
        } else {
            action
        },
        cwd: str_at(&v, "/cwd").map(str::to_owned),
    })
}

/// The harness session a hook call belongs to (`session_id`; Cursor's `conversation_id`), when it names one that fits
/// on a line.
#[must_use]
pub fn session_of(input: &[u8]) -> Option<String> {
    let v: Value = serde_json::from_slice(input).ok()?;
    let id = str_at(&v, "/session_id").or_else(|| str_at(&v, "/conversation_id"))?;
    (!id.is_empty() && id.len() <= 200 && id.chars().all(|c| c.is_ascii_graphic())).then(|| id.to_owned())
}

/// The rule the call matches and the question to ask about it.
#[must_use]
pub fn question(call: &HookCall, guard: &GuardConfig) -> Option<(Match, Question)> {
    question_in(call, guard, None)
}

/// What a hook's question reports of the repository its command runs in (the phone reads it for a `git push`): the
/// checked-out branch, only when a push of it (`git push`, `git push origin HEAD`) certainly lands on the branch of the
/// same name, and origin's default branch when the repository records it.
///
/// `refs/remotes/origin/HEAD` is a file in the repository, so the agent can write it. The name it holds can only add a
/// protected branch: the phone keeps every built-in protected name whatever the file says. It is origin's default, so
/// a question reports it only for a push to origin. A push to any other remote reports nothing, and the phone asks.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GitHead {
    pub branch: Option<String>,
    pub default: Option<String>,
    /// The remote a push with no repository argument uses, from `branch.<name>.remote`. `None`: git's own default,
    /// which is `origin`.
    pub remote: Option<String>,
    /// Every remote's `url`, by name (one each; the configuration parsed cleanly, with no `pushurl`, `insteadOf` or
    /// include anywhere).
    pub remotes: std::collections::BTreeMap<String, String>,
}

/// Reads a small file of the repository (at most 64 KiB): `None` when it is not there or larger.
fn small_file(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > 64 * 1024 {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

/// A branch name on one line, as git allows it (the phone checks it again).
fn one_branch(name: &str) -> Option<String> {
    let name = name.trim();
    (!name.is_empty() && name.len() <= 200 && name.chars().all(|c| c.is_ascii_graphic())).then(|| name.to_owned())
}

/// The settings of git's configuration that decide where `git push` with no destination sends the current branch:
/// `push.default`, `branch.<name>.merge` and `branch.<name>.remote`.
#[derive(Default)]
struct PushSettings {
    default: Option<String>,
    merge: Option<String>,
    remote: Option<String>,
    remotes: std::collections::BTreeMap<String, String>,
}

/// Reads a git configuration file into `into`; `false` unless every line of it parses cleanly by the plain subset:
/// comments, `[section]`, `[section "subsection"]` or `[section.subsection]` with nothing after the `]`, and `key =
/// value` lines with no continuation, escape or inline comment; no include, no `remote.*.push`, no `push.*` key but
/// `push.default`. Anything else and nothing is reported: the phone then takes the push as one to a protected branch.
fn push_settings(text: &str, branch: &str, into: &mut PushSettings) -> bool {
    let name_ok = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.'));
    let mut section: Option<(String, String)> = None;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with(['#', ';']) {
            continue;
        }
        if line.contains('\\') {
            return false;
        }
        if let Some(inner) = line.strip_prefix('[') {
            let Some(head) = inner.strip_suffix(']') else {
                return false;
            };
            let (name, sub) = match head.split_once(char::is_whitespace) {
                Some((n, s)) => {
                    let s = s.trim();
                    let Some(q) = s.strip_prefix('"').and_then(|q| q.strip_suffix('"')) else {
                        return false;
                    };
                    if q.contains('"') {
                        return false;
                    }
                    (n, q.to_owned())
                }
                // `[section.subsection]`: git reads the subsection without regard to case
                None => head.split_once('.').map_or((head, String::new()), |(n, s)| (n, s.to_ascii_lowercase())),
            };
            if !name_ok(name) || head.contains(']') {
                return false;
            }
            let name = name.to_ascii_lowercase();
            if name == "include" || name == "includeif" {
                return false;
            }
            section = Some((name, sub));
            continue;
        }
        let Some((section, sub)) = &section else {
            return false;
        };
        let (key, value) = line.split_once('=').map_or((line, ""), |(k, v)| (k.trim(), v.trim()));
        if !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') || key.is_empty() {
            return false;
        }
        if value.contains([';', '#']) {
            return false;
        }
        let value = match value.strip_prefix('"') {
            Some(q) => match q.strip_suffix('"') {
                Some(v) if !v.contains('"') => v,
                _ => return false,
            },
            None if value.contains('"') => return false,
            None => value,
        };
        let key = key.to_ascii_lowercase();
        // In the sections that can say where a push goes, only the keys known to be harmless are let through:
        // `push.default`, `remote.<name>.url` and `.fetch`, `branch.<name>.remote` and `.merge`. Any other (a mirror,
        // a push URL, `pushInsteadOf`, a push remote, `remote.pushDefault`…) and nothing is reported.
        match (section.as_str(), key.as_str()) {
            ("push", "default") => into.default = Some(value.to_ascii_lowercase()),
            ("remote", "url") if !sub.is_empty() => {
                // one url per remote: a second (git pushes to every one) and nothing is reported
                if into.remotes.insert(sub.clone(), value.to_owned()).is_some() {
                    return false;
                }
            }
            ("remote", "fetch") if !sub.is_empty() => {}
            ("branch", "remote" | "merge") if !sub.is_empty() => {
                // a section of this branch only by case: which one git reads is not certain
                if sub != branch && sub.eq_ignore_ascii_case(branch) {
                    return false;
                }
                if sub == branch && key == "merge" {
                    into.merge = Some(value.to_owned());
                }
                if sub == branch && key == "remote" {
                    into.remote = Some(value.to_owned());
                }
            }
            ("push" | "remote" | "branch" | "url", _) => return false,
            _ => {}
        }
    }
    true
}

/// A configuration file's text: `Ok(None)` when there is truly none; `Err` when one is there but cannot be read, is no
/// plain file or is larger than 64 KiB (then nothing is reported).
fn config_file(path: &Path) -> Result<Option<String>, ()> {
    match std::fs::metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(()),
        Ok(meta) if !meta.is_file() || meta.len() > 64 * 1024 => Err(()),
        Ok(_) => std::fs::read_to_string(path).map(Some).map_err(|_| ()),
    }
}

/// Reads the repository at or above `cwd` from its files: it never runs git, whose configuration in a repository can
/// make it run programs. The branch is left out unless a push of it certainly goes to the branch of the same name:
/// no configuration included from elsewhere, no `remote.*.push`, `push.default` neither `matching` nor `upstream`
/// to another branch.
#[must_use]
pub fn git_head(cwd: &Path) -> Option<GitHead> {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let xdg = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| home.as_ref().map(|h| h.join(".config")));
    let globals: Vec<std::path::PathBuf> = [
        Some(std::path::PathBuf::from("/etc/gitconfig")),
        home.map(|h| h.join(".gitconfig")),
        xdg.map(|x| x.join("git/config")),
    ]
    .into_iter()
    .flatten()
    .collect();
    git_head_with(cwd, &globals)
}

/// [`git_head`], with the system's and the user's configuration files named (`globals`).
pub(crate) fn git_head_with(cwd: &Path, globals: &[std::path::PathBuf]) -> Option<GitHead> {
    let (top, dot) = cwd.ancestors().find_map(|d| {
        let dot = d.join(".git");
        dot.exists().then(|| (d.to_path_buf(), dot))
    })?;
    let git_dir = if dot.is_dir() {
        dot
    } else {
        let pointer = small_file(&dot)?;
        let to = pointer.lines().next()?.strip_prefix("gitdir:")?.trim();
        top.join(to)
    };
    let common = small_file(&git_dir.join("commondir"))
        .map_or_else(|| git_dir.clone(), |c| git_dir.join(c.lines().next().unwrap_or_default().trim()));
    // one line each, exactly a symbolic ref
    let symref = |text: Option<String>, prefix: &str| {
        text.and_then(|t| {
            let mut lines = t.lines().filter(|l| !l.trim().is_empty());
            let first = lines.next()?;
            if lines.next().is_some() {
                return None;
            }
            first.strip_prefix(prefix).and_then(one_branch)
        })
    };
    let branch = symref(small_file(&git_dir.join("HEAD")), "ref: refs/heads/");
    // Local, and writable by the agent. The phone uses the name only to protect that branch as well; it never
    // withdraws a built-in name, and a question carries it only for a push to this same remote (origin).
    let default = symref(small_file(&common.join("refs/remotes/origin/HEAD")), "ref: refs/remotes/origin/");
    // Every configuration file must parse cleanly, else nothing at all is reported.
    let mut s = PushSettings::default();
    let local = [common.join("config"), git_dir.join("config.worktree")];
    for f in globals.iter().chain(&local) {
        match config_file(f) {
            Ok(None) => {}
            Ok(Some(text)) if push_settings(&text, branch.as_deref().unwrap_or_default(), &mut s) => {}
            _ => return None,
        }
    }
    let branch = branch.filter(|b| {
        let same_name = s.merge.as_deref().is_none_or(|m| m == format!("refs/heads/{b}"));
        match s.default.as_deref() {
            None | Some("simple" | "current" | "nothing") => true,
            Some("upstream" | "tracking") => same_name,
            Some(_) => false,
        }
    });
    Some(GitHead {
        branch,
        default,
        remote: s.remote,
        remotes: s.remotes,
    })
}

/// Flags of `git push` that do not name the repository and take no following word. Anything else (a value, `--repo`,
/// a short flag run together with another) and the remote is not known.
const PUSH_FLAGS_WITHOUT_A_VALUE: &[&str] = &[
    "-u",
    "--set-upstream",
    "-q",
    "--quiet",
    "-v",
    "--verbose",
    "--no-verify",
    "-f",
    "--force",
    "--force-with-lease",
    "--force-if-includes",
    "-d",
    "--delete",
    "--all",
    "--branches",
    "--tags",
    "--follow-tags",
    "--atomic",
    "--prune",
    "-n",
    "--dry-run",
    "--no-thin",
    "--thin",
    "--progress",
    "--no-progress",
    "-4",
    "--ipv4",
    "-6",
    "--ipv6",
    "--signed",
    "--no-signed",
    "--porcelain",
];

/// The line is one plain `git push` whose repository is origin: the remote the default branch was read from.
/// `configured` is `branch.<name>.remote` (`None`: git uses origin). A push to any other remote, or a line that is
/// not one plain push, is not: the question then reports no branch, and the phone asks.
fn push_targets_origin(command: &str, configured: Option<&str>) -> bool {
    // Quotes, substitutions and a second command can hide the repository. `=` attaches a value to an option
    // (`--repo=upstream`). `@` and `~` are not a remote name this reading knows.
    const HIDDEN: [char; 24] = [
        ';', '&', '|', '(', ')', '`', '$', '<', '>', '\n', '\r', '"', '\'', '\\', '{', '}', '*', '?', '[', '#', '!',
        '=', '~', '@',
    ];
    if command.contains(HIDDEN) {
        return false;
    }
    let words: Vec<&str> = command.split_whitespace().collect();
    if words.first() != Some(&"git") || words.get(1) != Some(&"push") {
        return false;
    }
    let mut operands = Vec::new();
    for w in &words[2..] {
        if w.starts_with('-') {
            if !PUSH_FLAGS_WITHOUT_A_VALUE.contains(w) {
                return false;
            }
        } else {
            operands.push(*w);
        }
    }
    let remote = match operands.first() {
        None => configured.unwrap_or("origin"),
        // The first operand is the repository. A refspec (`HEAD:feature`, `feature/x`) is not one, so the remote is
        // not known here.
        Some(name)
            if !name.is_empty()
                && !name.starts_with(['-', '.'])
                && name.chars().all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-')) =>
        {
            name
        }
        Some(_) => return false,
    };
    remote == "origin"
}

/// [`question`], with what [`git_head`] read of the repository the command runs in: its lines follow the rule's, before
/// the command, so that a command cannot write them.
#[must_use]
pub fn question_in(call: &HookCall, guard: &GuardConfig, head: Option<&GitHead>) -> Option<(Match, Question)> {
    let (m, question, detail) = question_text(call, guard, head)?;
    let q = Question::new(&question, Some(&detail), Some(&m.topic)).ok()?;
    Some((m, q))
}

/// The rule, the question's line and its whole detail as built from the raw call, before anything is cut or cleaned
/// to reach the phone.
fn question_text(call: &HookCall, guard: &GuardConfig, head: Option<&GitHead>) -> Option<(Match, String, String)> {
    let who = call.harness.label();
    let (m, question, mut detail) = match &call.action {
        Action::Command(c) => {
            let m = guard.check_command(c)?;
            // A line separator is written out, never folded away.
            let one_line = crate::ask::one_line(c);
            (m, format!("{who} wants to run: {one_line}"), format!("Command:\n{c}"))
        }
        Action::Files {
            write,
            paths,
        } => {
            let (m, path) = paths.iter().find_map(|p| guard.check_file(p).map(|m| (m, p)))?;
            let verb = if *write {
                "change"
            } else {
                "read"
            };
            let others = paths.len() - 1;
            let more = if others > 0 {
                format!(" and {others} more file(s)")
            } else {
                String::new()
            };
            (m, format!("{who} wants to {verb} {path}{more}"), format!("Files:\n{}", paths.join("\n")))
        }
        Action::Other => return None,
    };
    if let Some(cwd) = &call.cwd {
        let _infallible = write!(detail, "\n\nIn: {cwd}");
    }
    let mut head_lines = String::new();
    if let (Action::Command(command), Some(h)) = (&call.action, head)
        && push_targets_origin(command, h.remote.as_deref())
    {
        if let Some(b) = &h.branch {
            let _infallible = writeln!(head_lines, "Branch: {b}");
        }
        if let Some(d) = &h.default {
            let _infallible = writeln!(head_lines, "Default branch: {d}");
        }
    }
    let detail = format!("{}{head_lines}\n{detail}", rule_lines(&m));
    Some((m, question, detail))
}

/// The first lines of a question's detail: the rule that matched and what matched it, which the phone shows (and
/// reads: the group decides whether the question is asked every time, and is its own kind for Autopilot). First, so a
/// long command never cuts them off, and so a command cannot write them:
///
/// ```text
/// Rule: .env files · .env
/// Matched: app/.env
/// Group: env-files
/// ```
///
/// `Group: git-history (destructive)` for what cannot be undone; no `Group` line for the user's own rules.
#[must_use]
pub fn rule_lines(m: &Match) -> String {
    let one_line = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let group = m.group();
    let mut lines = format!(
        "Rule: {} · {}\nMatched: {}\n",
        group.map_or("Your rule", |g| g.label),
        one_line(&m.pattern),
        one_line(&m.matched)
    );
    if let Some(g) = group {
        let destructive = if g.destructive() {
            " (destructive)"
        } else {
            ""
        };
        let _infallible = writeln!(lines, "Group: {}{destructive}", g.id);
    }
    lines
}

/// The verdict for an answer (`timed_out`: nobody answered within the time), in words that say it was Reins 2FA.
#[must_use]
pub fn verdict(answer: Answer, timed_out: bool, on_no_answer: OnNoAnswer) -> Verdict {
    match answer {
        Answer::Yes => Verdict::Allow("Reins 2FA: approved.".to_owned()),
        Answer::No(why) => Verdict::Deny(format!("Reins 2FA: denied. {why} Do not try another way around this.")),
        Answer::Unanswered(why) => {
            let what = if timed_out {
                format!("timed out, nobody answered ({why})")
            } else {
                format!("could not ask ({why})")
            };
            match on_no_answer {
                OnNoAnswer::Deny => Verdict::Deny(format!(
                    "Reins 2FA: not allowed: {what}. Ask the user to approve it in the Reins app, then try again."
                )),
                OnNoAnswer::Ask => Verdict::Ask(format!("Reins 2FA: {what}.")),
            }
        }
    }
}

/// What to print on stdout (if anything) and the exit code.
#[must_use]
pub fn render(call: &HookCall, verdict: &Verdict) -> (Option<String>, u8) {
    let out = |v: Value| (Some(v.to_string()), 0);
    match (call.event, verdict) {
        // Cursor never asks before a read, so allowing one takes no decision away from the user; an empty answer
        // would only show up as a failed hook in Cursor's log.
        (Event::CursorRead, Verdict::Unmatched) => out(json!({"permission": "allow"})),
        (Event::Unknown | Event::UserPromptSubmit | Event::Stop, _) | (_, Verdict::Unmatched) => (None, 0),
        (Event::PreToolUse, v) => {
            let (decision, reason) = match v {
                Verdict::Allow(r) => ("allow", r),
                Verdict::Deny(r) => ("deny", r),
                Verdict::Ask(_) if call.harness == Harness::Codex => return (None, 0),
                Verdict::Ask(r) => ("ask", r),
                Verdict::Unmatched => return (None, 0),
            };
            let mut answer = json!({"hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": decision,
                "permissionDecisionReason": reason,
            }});
            // Claude Code shows this line to the user, so the outcome of the phone's 2FA is visible there too.
            if call.harness == Harness::ClaudeCode {
                answer["systemMessage"] = json!(reason);
            }
            out(answer)
        }
        (Event::CursorShell | Event::CursorRead | Event::CursorTool, v) => {
            let (permission, message) = match v {
                Verdict::Allow(r) => ("allow", r),
                Verdict::Ask(r) if call.event == Event::CursorShell => ("ask", r),
                Verdict::Deny(r) | Verdict::Ask(r) => ("deny", r),
                Verdict::Unmatched => return (None, 0),
            };
            out(json!({"permission": permission, "user_message": message, "agent_message": message}))
        }
        (Event::GeminiBeforeTool, v) => match v {
            Verdict::Allow(r) => out(json!({"decision": "allow", "reason": r})),
            Verdict::Deny(r) => out(json!({"decision": "deny", "reason": r})),
            Verdict::Ask(_) | Verdict::Unmatched => (None, 0),
        },
    }
}

/// [`render`], with `news` of the session's tickets for Claude Code: added to a `PreToolUse` answer and said on
/// `UserPromptSubmit` as `additionalContext`, and on `Stop` as the reason to go on (`decision: block`). No news: as
/// [`render`].
#[must_use]
pub fn render_with(call: &HookCall, verdict: &Verdict, news: &[String]) -> (Option<String>, u8) {
    let out = |v: Value| (Some(v.to_string()), 0);
    let news = (call.harness == Harness::ClaudeCode && !news.is_empty()).then(|| news.join("\n"));
    match (call.event, news) {
        (Event::Stop, Some(n)) => out(json!({"decision": "block", "reason": n})),
        (Event::UserPromptSubmit, Some(n)) => {
            out(json!({"hookSpecificOutput": {"hookEventName": "UserPromptSubmit", "additionalContext": n}}))
        }
        (Event::PreToolUse, Some(n)) => {
            let mut answer = render(call, verdict)
                .0
                .and_then(|o| serde_json::from_str::<Value>(&o).ok())
                .unwrap_or_else(|| json!({"hookSpecificOutput": {"hookEventName": "PreToolUse"}}));
            answer["hookSpecificOutput"]["additionalContext"] = json!(n);
            out(answer)
        }
        _ => render(call, verdict),
    }
}

/// The digest a pass for this call is bound to, over the call as the harness sent it, never as shown: the harness, the
/// hook event and tool, the folder, the whole command (or each path, and whether it is read or changed) and what was
/// read of the repository. Two calls that look alike on the phone but differ anywhere get different digests.
#[must_use]
pub fn pass_digest(call: &HookCall, tool: &str, head: Option<&GitHead>) -> String {
    use crate::tickets::optional;
    let event = format!("{:?}", call.event);
    let mut parts: Vec<Vec<u8>> =
        vec![call.harness.id().into(), event.into_bytes(), tool.as_bytes().to_vec(), optional(call.cwd.as_deref())];
    match &call.action {
        Action::Command(c) => parts.extend([b"command".to_vec(), c.as_bytes().to_vec()]),
        Action::Files {
            write,
            paths,
        } => {
            let how: &[u8] = if *write {
                b"write"
            } else {
                b"read"
            };
            parts.extend([b"files".to_vec(), how.to_vec(), paths.len().to_string().into_bytes()]);
            parts.extend(paths.iter().map(|p| p.as_bytes().to_vec()));
        }
        Action::Other => parts.push(b"other".to_vec()),
    }
    match head {
        Some(h) => parts.extend([
            b"head".to_vec(),
            optional(h.branch.as_deref()),
            optional(h.default.as_deref()),
            optional(h.remote.as_deref()),
        ]),
        None => parts.push(b"no head".to_vec()),
    }
    let parts: Vec<&[u8]> = parts.iter().map(Vec::as_slice).collect();
    crate::tickets::pass_digest("hook", &parts)
}

/// Whether a pass may be bound to this call, which is asked about with `q`: the command, every path and the folder are
/// printable text and spaces only (no [`crate::ask::invisible`] character: no control character, so no newline that
/// could forge a line of the detail, no bidi override, zero width or separator), and `q`'s detail exactly the detail
/// built from this raw call, nothing cut, cleaned or written out. Anything else
/// is asked as an ordinary Instant question: no digest is sent, no ticket kept, no pass honoured.
#[must_use]
pub fn passable(call: &HookCall, guard: &GuardConfig, head: Option<&GitHead>, q: &Question) -> bool {
    let plain = |s: &str| !s.is_empty() && !s.chars().any(crate::ask::invisible);
    let raw = match &call.action {
        Action::Command(c) => plain(c),
        Action::Files {
            paths,
            ..
        } => !paths.is_empty() && paths.iter().all(|p| plain(p)),
        Action::Other => false,
    };
    raw && call.cwd.as_deref().is_none_or(plain)
        && question_text(call, guard, head).is_some_and(|(_, _, detail)| q.detail.as_deref() == Some(detail.as_str()))
}

/// One line saying what a guarded action is, for a ticket (`git push --force origin fix/ci`, `read .env`).
fn summary(action: &Action) -> String {
    match action {
        Action::Command(c) => crate::tickets::summary_of(c),
        Action::Files {
            write,
            paths,
        } => {
            let verb = if *write {
                "change"
            } else {
                "read"
            };
            crate::tickets::summary_of(&format!("{verb} {}", paths.join(", ")))
        }
        Action::Other => String::new(),
    }
}

/// Before a hook decides: when tickets wait and the daemon (which looks every 30 s) does not run, one quick look
/// ([`crate::tickets::HOOK_REFRESH`]).
async fn look_at_tickets(paths: &Paths, config: &Config) {
    use crate::tickets::{HOOK_REFRESH, TicketStore};
    if !TicketStore::load(paths).any_waiting() || crate::tickets::daemon_running(paths, config).await {
        return;
    }
    let Ok(client) = crate::server::client::DesktopClient::new(paths) else {
        return;
    };
    match tokio::time::timeout(HOOK_REFRESH, crate::tickets::refresh(paths, &client, false)).await {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => log::info!("tickets: {e}"),
        Err(_) => log::info!("tickets: no answer within {} s", HOOK_REFRESH.as_secs()),
    }
}

/// What `session` has not been told yet (marked told).
fn news(paths: &Paths, session: &str) -> Vec<String> {
    use crate::tickets::TicketStore;
    // Most calls have none: only then is the file written.
    if TicketStore::load(paths).news_for(session).is_empty() {
        return Vec::new();
    }
    TicketStore::update(paths, |s| s.news_for(session)).unwrap_or_else(|e| {
        log::warn!("tickets: {e}");
        Vec::new()
    })
}

/// Uses the pass for `digest`, if there is one.
/// Uses the pass for `digest` in `session`, if there is one.
fn take_pass(paths: &Paths, digest: &str, session: &str) -> Option<crate::tickets::Pass> {
    use crate::tickets::TicketStore;
    let now = crate::now_unix();
    let mine =
        |p: &crate::tickets::Pass| p.digest == digest && p.session.as_deref() == Some(session) && p.expires_at > now;
    if !TicketStore::load(paths).passes().iter().any(mine) {
        return None;
    }
    TicketStore::update(paths, |s| s.take_pass(digest, Some(session), now)).ok().flatten()
}

/// Runs one hook: parse, match, ask (up to `guard.timeout_secs`), answer. Unreadable input is refused (exit 2, the
/// reason on stderr, which every harness takes as "blocked").
pub async fn run(
    harness: Harness,
    input: &[u8],
    paths: &Paths,
    config: &Config,
    prompter: &dyn Prompter,
) -> (Option<String>, u8, Option<String>) {
    let call = match parse(harness, input) {
        Ok(c) => c,
        Err(e) => return (None, 2, Some(format!("reins hook: {e}; not allowed."))),
    };
    let session = session_of(input);
    let phone = matches!(crate::ask::decider(paths, config), Ok(crate::ask::Decider::Phone));
    if phone && call.event != Event::Unknown {
        look_at_tickets(paths, config).await;
    }
    let news = match (&session, harness, call.event) {
        (Some(s), Harness::ClaudeCode, Event::PreToolUse | Event::UserPromptSubmit | Event::Stop) => news(paths, s),
        _ => Vec::new(),
    };
    // For a push, what the repository says of its branch (read from its files, never by running git).
    let head = match (&call.action, &call.cwd) {
        (Action::Command(c), Some(cwd)) if c.contains("push") => git_head(Path::new(cwd)),
        _ => None,
    };
    let kind = match call.action {
        Action::Files {
            ..
        } => Kind::File,
        _ => Kind::Command,
    };
    // Decisions v2: with `auto.matrix_decides` (and this computer's setup done), the matrix only adds asks: what the
    // guard asks about is still asked, and what the matrix asks about is asked even with no guard rule. Without a
    // reading in time, or with it off, the guard decides as before.
    let decided = if config.auto.matrix_decides && config.auto.setup_done {
        crate::consequences::evaluate_action(config, &call.action, call.cwd.as_deref(), head.as_ref(), harness.label())
    } else {
        None
    };
    let guarded = question_in(&call, &config.guard, head.as_ref());
    let (m, q, reading) = if let Some((summary, shadow)) = decided {
        if shadow.outcome == reins_policy::matrix::Outcome::Deny {
            let reason = format!("Reins: not allowed: {}", shadow.line);
            let mut entry = Entry::new(kind, &action_line(&call))
                .source(Some(harness.label()))
                .decider(crate::journal::Decider::Settings)
                .effects(Some(summary))
                .matrix(Some(shadow));
            entry.end(Outcome::Blocked, Some(&reason));
            crate::journal::Journal::new(paths).record(&entry);
            let (out, code) = render(&call, &Verdict::Deny(reason));
            return (out, code, None);
        }
        let asks = matches!(shadow.outcome, reins_policy::matrix::Outcome::Ask { .. });
        let question = guarded.or_else(|| asks.then(|| matrix_question(&call, &shadow.line)).flatten());
        let Some((m, q)) = question else {
            let (out, code) = render_with(&call, &Verdict::Unmatched, &news);
            return (out, code, None);
        };
        (m, q, Some((summary, shadow)))
    } else {
        let Some((m, q)) = guarded else {
            let (out, code) = render_with(&call, &Verdict::Unmatched, &news);
            return (out, code, None);
        };
        (m, q, None)
    };
    let entry =
        Entry::new(kind, &q.question).source(Some(harness.label())).service(Some(&m.topic)).detail(q.detail.as_deref());
    let tool = serde_json::from_slice::<Value>(input)
        .ok()
        .and_then(|v| str_at(&v, "/tool_name").map(str::to_owned))
        .unwrap_or_default();
    // A pass is bound to the session as well: a call that names none is never parked, and never passed.
    let digest = (session.is_some() && passable(&call, &config.guard, head.as_ref(), &q))
        .then(|| pass_digest(&call, &tool, head.as_ref()));
    let pass = match (&digest, &session) {
        (Some(d), Some(s)) => take_pass(paths, d, s),
        _ => None,
    };
    if let Some(pass) = pass {
        log::info!("hook {}: `{}` approved earlier ({})", harness.id(), m.pattern, pass.ticket);
        let reason = format!("Approved earlier (ticket {})", pass.ticket);
        let mut entry = entry.decider(crate::journal::Decider::Phone);
        entry.end(crate::journal::Outcome::Approved, Some(&reason));
        crate::journal::Journal::new(paths).record(&entry);
        let (out, code) = render_with(&call, &Verdict::Allow(reason), &news);
        return (out, code, None);
    }
    log::info!("hook {}: asking about `{}`", harness.id(), m.pattern);
    let timeout = Duration::from_secs(config.guard.timeout_secs);
    // stderr: the harness's log, and its transcript where it shows hook output.
    let tell = |line: &str| eprintln!("{line}");
    let ticketing = digest.as_deref().map(|digest| crate::ask::Ticketing {
        digest,
        defer: false,
    });
    let (answer, timed_out, entry) =
        crate::ask::ask_logged_ticketed(paths, config, &q, timeout, false, prompter, entry, Some(&tell), ticketing)
            .await;
    let v = match answer {
        Ok(answer) => verdict(answer, timed_out, config.guard.on_no_answer),
        Err(parked) => {
            let now = crate::now_unix();
            let ticket = crate::tickets::LocalTicket {
                ticket: parked.ticket.clone(),
                kind: crate::tickets::TicketKind::Hook,
                session,
                summary: summary(&call.action),
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
                log::warn!("tickets: {e}");
            }
            Verdict::Deny(crate::tickets::parked_hook_message(&parked.ticket, parked.expected_by, now))
        }
    };
    let (out, code) = render_with(&call, &v, &news);
    // What it does, for Activity (Decisions v2 P1: shown, never deciding): read only once the verdict is fixed, on
    // its own thread with a deadline; whatever happens there, the answer above is what the harness gets.
    let reading = reading.or_else(|| {
        crate::consequences::evaluate_action(config, &call.action, call.cwd.as_deref(), head.as_ref(), harness.label())
    });
    if let Some((effects, shadow)) = reading {
        crate::journal::Journal::new(paths).record(&entry.effects(Some(effects)).matrix(Some(shadow)));
    }
    (out, code, None)
}

/// What an action is, on one line, for an Activity entry the phone never saw.
fn action_line(call: &HookCall) -> String {
    let who = call.harness.label();
    match &call.action {
        Action::Command(c) => format!("{who} ran: {}", c.split_whitespace().collect::<Vec<_>>().join(" ")),
        Action::Files {
            write,
            paths,
        } => format!(
            "{who} {} {}",
            if *write {
                "changed"
            } else {
                "read"
            },
            paths.join(", ")
        ),
        Action::Other => format!("{who} used a tool"),
    }
}

/// The question for something the matrix asks about that no guard rule matches.
fn matrix_question(call: &HookCall, line: &str) -> Option<(Match, Question)> {
    let who = call.harness.label();
    let (question, mut detail) = match &call.action {
        Action::Command(c) => {
            let one_line = c.split_whitespace().collect::<Vec<_>>().join(" ");
            (format!("{who} wants to run: {one_line}"), format!("Command:\n{c}"))
        }
        Action::Files {
            write,
            paths,
        } => {
            let verb = if *write {
                "change"
            } else {
                "read"
            };
            (format!("{who} wants to {verb} {}", paths.join(", ")), format!("Files:\n{}", paths.join("\n")))
        }
        Action::Other => return None,
    };
    if let Some(cwd) = &call.cwd {
        let _infallible = write!(detail, "\n\nIn: {cwd}");
    }
    let detail = format!("Auto: {line}\n\n{detail}");
    let m = Match {
        pattern: "auto".to_owned(),
        topic: "auto".to_owned(),
        matched: line.to_owned(),
    };
    let q = Question::new(&question, Some(&detail), None).ok()?;
    Some((m, q))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bash(command: &str) -> HookCall {
        HookCall {
            harness: Harness::ClaudeCode,
            event: Event::PreToolUse,
            action: Action::Command(command.to_owned()),
            cwd: Some("/home/me/app".to_owned()),
        }
    }

    #[test]
    fn commands_equal_up_to_the_display_limit_get_different_digests() {
        let guard = GuardConfig::default();
        let shared = format!("rm -rf ./build {}", "x".repeat(400));
        let (a, b) = (bash(&format!("{shared} && echo done")), bash(&format!("{shared} && rm -rf ~")));
        let (qa, qb) = (question(&a, &guard).unwrap().1, question(&b, &guard).unwrap().1);
        assert_eq!(qa.question, qb.question, "the phone's one line is the same");
        assert!(passable(&a, &guard, None, &qa) && passable(&b, &guard, None, &qb), "the detail shows each whole");
        assert_ne!(pass_digest(&a, "Bash", None), pass_digest(&b, "Bash", None));
        // Past what the detail carries, a command is not shown whole: never a ticket or a pass.
        let long = bash(&format!("rm -rf ./build {} && rm -rf ~", "x".repeat(9_000)));
        let ql = question(&long, &guard).unwrap().1;
        assert!(!passable(&long, &guard, None, &ql));
        // Everything else that is hashed counts too.
        let elsewhere = HookCall {
            cwd: Some("/home/me/other".to_owned()),
            ..a.clone()
        };
        assert_ne!(pass_digest(&a, "Bash", None), pass_digest(&elsewhere, "Bash", None));
        assert_ne!(pass_digest(&a, "Bash", None), pass_digest(&a, "shell", None));
        let head = GitHead {
            branch: Some("main".into()),
            ..GitHead::default()
        };
        assert_ne!(pass_digest(&a, "Bash", None), pass_digest(&a, "Bash", Some(&head)));
        assert_eq!(pass_digest(&a, "Bash", Some(&head)), pass_digest(&a.clone(), "Bash", Some(&head.clone())));
    }

    #[test]
    fn control_characters_are_hashed_raw() {
        let guard = GuardConfig::default();
        let (plain, bell) = (bash("rm -rf build"), bash("rm -rf build\u{7}"));
        let (qp, qb) = (question(&plain, &guard).unwrap().1, question(&bell, &guard).unwrap().1);
        assert_eq!(qp, qb, "the phone shows the two alike");
        assert_ne!(pass_digest(&plain, "Bash", None), pass_digest(&bell, "Bash", None));
        // So the one the phone cannot show is never parked or passed.
        assert!(passable(&plain, &guard, None, &qp));
        assert!(!passable(&bell, &guard, None, &qb));
    }

    fn files(paths: &[&str]) -> HookCall {
        HookCall {
            harness: Harness::ClaudeCode,
            event: Event::PreToolUse,
            action: Action::Files {
                write: false,
                paths: paths.iter().map(|p| (*p).to_owned()).collect(),
            },
            cwd: Some("/home/me/app".to_owned()),
        }
    }

    /// `call` asked about with the question of `shown`: never a pass unless they are the same call.
    fn passes_as(call: &HookCall, shown: &HookCall) -> bool {
        let guard = GuardConfig::default();
        let (_, q) = question(shown, &guard).expect("asked about");
        passable(call, &guard, None, &q)
    }

    #[test]
    fn a_shown_path_that_only_contains_the_real_one_gets_no_pass() {
        let real = files(&["/home/me/.env"]);
        assert!(passes_as(&real, &real));
        assert!(!passes_as(&real, &files(&["/home/me/.env.local"])));
        assert!(!passes_as(&real, &files(&["/home/me/.env/x/.env"])));
        // One path of several shown: the list must be the same, element for element and in order.
        assert!(!passes_as(&real, &files(&["/home/me/.env", "/home/me/.ssh/id_ed25519"])));
        let two = files(&["/home/me/.env", "/home/me/.ssh/id_ed25519"]);
        assert!(!passes_as(&two, &files(&["/home/me/.ssh/id_ed25519", "/home/me/.env"])));
    }

    #[test]
    fn a_path_suffix_trick_gets_no_pass() {
        assert!(!passes_as(&files(&[".env"]), &files(&["/home/me/app/.env"])));
        assert!(!passes_as(&files(&["app/.env"]), &files(&["/home/me/app/.env"])));
    }

    #[test]
    fn a_newline_in_a_path_gets_no_pass() {
        // Each still ends as a secret file, so it is asked about; none may be passed.
        for path in ["/home/me/safe\nIn: /home/me/app/.env", "/home/me/x\r/home/me/app/.env", "/home/me/x\t/app/.env"] {
            let call = files(&[path]);
            assert!(!passes_as(&call, &call), "{path:?}");
        }
        let mut cwd = files(&["/home/me/app/.env"]);
        cwd.cwd = Some("/home/me/app\nRule: none".to_owned());
        assert!(!passes_as(&cwd, &cwd));
        // A command with a newline (a heredoc too) is asked about every time.
        let heredoc = bash("rm -rf build\nrm -rf dist");
        assert!(!passes_as(&heredoc, &heredoc));
    }

    /// `call` is asked about, never passable, and the phone is shown `code` written out in its question's detail.
    fn shown_never_passed(call: &HookCall, code: &str) {
        let guard = GuardConfig::default();
        let (_, q) = question(call, &guard).expect("asked about");
        assert!(!passable(call, &guard, None, &q), "{call:?}");
        let sent = q.sent();
        assert!(sent.detail.as_deref().unwrap().contains(code), "the phone is shown it: {:?}", sent.detail);
    }

    #[test]
    fn an_rtl_override_command_is_not_passable() {
        let call = bash("rm -rf build \u{202e}txt.exe");
        shown_never_passed(&call, "⟨U+202E⟩");
        let (_, q) = question(&call, &GuardConfig::default()).unwrap();
        assert!(q.sent().question.contains("⟨U+202E⟩"), "{}", q.sent().question);
    }

    #[test]
    fn a_zero_width_space_in_a_path_is_not_passable() {
        shown_never_passed(&files(&["/home/me/a\u{200b}pp/.env"]), "⟨U+200B⟩");
        let mut cwd = files(&["/home/me/app/.env"]);
        cwd.cwd = Some("/home/me/app\u{feff}".to_owned());
        shown_never_passed(&cwd, "⟨U+FEFF⟩");
    }

    #[test]
    fn a_line_separator_in_a_command_is_not_passable() {
        let call = bash("rm -rf build\u{2028}rm -rf ~");
        shown_never_passed(&call, "⟨U+2028⟩");
        let (_, q) = question(&call, &GuardConfig::default()).unwrap();
        assert!(q.question.contains("⟨U+2028⟩"), "the one line keeps it too: {}", q.question);
        shown_never_passed(&bash("rm -rf build\u{2029}"), "⟨U+2029⟩");
    }

    #[test]
    fn a_command_shown_as_a_substring_of_the_real_one_gets_no_pass() {
        let (short, long) = (bash("rm -rf build"), bash("rm -rf build && rm -rf ~"));
        assert!(passes_as(&short, &short) && passes_as(&long, &long));
        assert!(!passes_as(&long, &short), "the real one is longer than what was shown");
        assert!(!passes_as(&short, &long), "what was shown is longer than the real one");
        let elsewhere = HookCall {
            cwd: Some("/home/me".to_owned()),
            ..short.clone()
        };
        assert!(!passes_as(&elsewhere, &short));
    }

    #[test]
    fn apply_patch_names_its_files() {
        let patch = "*** Begin Patch\n*** Update File: src/a.rs\n@@\n-x\n+y\n*** Add File: .env\n+K=V\n*** Delete File: old.txt\n*** End Patch";
        assert_eq!(patch_files(patch), vec!["src/a.rs", ".env", "old.txt"]);
    }

    #[test]
    fn verdicts_follow_the_answer_and_the_no_answer_setting() {
        assert!(matches!(verdict(Answer::Yes, false, OnNoAnswer::Deny), Verdict::Allow(_)));
        assert!(matches!(verdict(Answer::No("x".into()), false, OnNoAnswer::Ask), Verdict::Deny(_)));
        assert!(matches!(verdict(Answer::Unanswered("t".into()), true, OnNoAnswer::Deny), Verdict::Deny(_)));
        assert!(matches!(verdict(Answer::Unanswered("t".into()), true, OnNoAnswer::Ask), Verdict::Ask(_)));
        // The agent is told which it was: approved, denied, timed out.
        let Verdict::Deny(late) = verdict(Answer::Unanswered("120 s".into()), true, OnNoAnswer::Deny) else {
            unreachable!()
        };
        assert!(late.starts_with("Reins 2FA: not allowed: timed out"), "{late}");
        let Verdict::Deny(no) = verdict(Answer::No("Denied on your phone.".into()), false, OnNoAnswer::Deny) else {
            unreachable!()
        };
        assert!(no.starts_with("Reins 2FA: denied."), "{no}");
        let Verdict::Deny(off) = verdict(Answer::Unanswered("not logged in".into()), false, OnNoAnswer::Deny) else {
            unreachable!()
        };
        assert!(off.contains("could not ask (not logged in)"), "{off}");
    }

    fn cursor(input: &str) -> HookCall {
        parse(Harness::Cursor, input.as_bytes()).unwrap()
    }

    #[test]
    fn cursor_gets_no_decision_for_unmatched_commands_and_tools() {
        // An explicit `allow` would run the command without Cursor's own approval; no output leaves it to Cursor.
        let shell = cursor(r#"{"hook_event_name": "beforeShellExecution", "command": "rm -rf build"}"#);
        assert_eq!(shell.event, Event::CursorShell);
        assert_eq!(render(&shell, &Verdict::Unmatched), (None, 0));
        let write =
            cursor(r#"{"hook_event_name": "preToolUse", "tool_name": "Delete", "tool_input": {"file_path": "/a"}}"#);
        assert_eq!(write.event, Event::CursorTool);
        assert_eq!(render(&write, &Verdict::Unmatched), (None, 0));
        let shell_tool =
            cursor(r#"{"hook_event_name": "preToolUse", "tool_name": "Shell", "tool_input": "{\"command\": \"ls\"}"}"#);
        assert_eq!(shell_tool.action, Action::Command("ls".to_owned()));
        assert_eq!(render(&shell_tool, &Verdict::Unmatched), (None, 0));
    }

    #[test]
    fn cursor_reads_are_allowed_explicitly_and_decisions_keep_their_words() {
        let read = cursor(r#"{"hook_event_name": "beforeReadFile", "file_path": "/a/main.rs"}"#);
        assert_eq!(read.event, Event::CursorRead);
        assert_eq!(render(&read, &Verdict::Unmatched), (Some(r#"{"permission":"allow"}"#.to_owned()), 0));

        let shell = cursor(r#"{"hook_event_name": "beforeShellExecution", "command": "kubectl delete ns x"}"#);
        let permission = |call: &HookCall, v: Verdict| {
            let (out, code) = render(call, &v);
            assert_eq!(code, 0);
            serde_json::from_str::<Value>(&out.unwrap()).unwrap()["permission"].as_str().unwrap().to_owned()
        };
        assert_eq!(permission(&shell, Verdict::Allow("ok".into())), "allow");
        assert_eq!(permission(&shell, Verdict::Deny("no".into())), "deny");
        assert_eq!(permission(&shell, Verdict::Ask("?".into())), "ask");
        // Read and tool hooks know no "ask".
        assert_eq!(permission(&read, Verdict::Ask("?".into())), "deny");
    }

    #[test]
    fn claude_code_shows_the_outcome_to_the_user_too() {
        let input = r#"{"hook_event_name": "PreToolUse", "tool_name": "Bash", "tool_input": {"command": "rm -rf x"}}"#;
        let claude = parse(Harness::ClaudeCode, input.as_bytes()).unwrap();
        let (out, _) = render(&claude, &Verdict::Allow("Reins 2FA: approved.".into()));
        let v: Value = serde_json::from_str(&out.unwrap()).unwrap();
        assert_eq!(v["systemMessage"], "Reins 2FA: approved.");
        assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "allow");
        let codex = parse(Harness::Codex, input.as_bytes()).unwrap();
        let (out, _) = render(&codex, &Verdict::Deny("no".into()));
        assert!(serde_json::from_str::<Value>(&out.unwrap()).unwrap().get("systemMessage").is_none());
    }

    fn claude_call(action: Action) -> HookCall {
        HookCall {
            harness: Harness::ClaudeCode,
            event: Event::PreToolUse,
            action,
            cwd: Some("/home/me/app".to_owned()),
        }
    }

    #[test]
    fn the_question_starts_with_the_rule_and_what_matched_it() {
        let detail =
            |action: Action, guard: &GuardConfig| question(&claude_call(action), guard).unwrap().1.detail.unwrap();
        let guard = GuardConfig::default();
        assert_eq!(
            detail(Action::Command("git push --force origin main".to_owned()), &guard),
            "Rule: Rewriting git history · git push --force*\nMatched: git push --force origin main\nGroup: \
             git-history (destructive)\n\nCommand:\ngit push --force origin main\n\nIn: /home/me/app"
        );
        let read = Action::Files {
            write: false,
            paths: vec!["/home/me/app/.env".to_owned()],
        };
        assert_eq!(
            detail(read, &guard),
            "Rule: .env files · .env\nMatched: /home/me/app/.env\nGroup: env-files\n\nFiles:\n/home/me/app/.env\n\nIn: \
             /home/me/app"
        );
        let own = GuardConfig {
            commands: vec!["make deploy".to_owned()],
            ..GuardConfig::default()
        };
        assert!(
            detail(Action::Command("make deploy".to_owned()), &own)
                .starts_with("Rule: Your rule · make deploy\nMatched: make deploy\n\nCommand:\n")
        );
        // What the command says comes after the rule.
        let forged = "rm -rf build\nRule: .env files · .env\nGroup: env-files";
        assert!(detail(Action::Command(forged.to_owned()), &guard).starts_with("Rule: Recursive deletes · rm -r\n"));
    }

    #[test]
    fn a_heredoc_that_edits_code_says_which_word_matched() {
        let screenshot = include_str!("../tests/fixtures/heredoc-python-edit.sh");
        let (_, q) = question(&claude_call(Action::Command(screenshot.to_owned())), &GuardConfig::default()).unwrap();
        let detail = q.detail.unwrap();
        let head: Vec<&str> = detail.lines().take(3).collect();
        assert_eq!(head[0], "Rule: Cloud and cluster credentials · .config/gcloud/*", "{head:?}");
        assert!(head[1].starts_with("Matched: ") && head[1].contains(".config/gcloud/"), "{head:?}");
        assert_eq!(head[2], "Group: cloud", "an ordinary question, not one that cannot be undone");
    }

    /// A repository on disk: `.git/HEAD`, the remote's default, and a configuration.
    fn repo(head: &str, origin_head: Option<&str>, config: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let git = dir.path().join(".git");
        std::fs::create_dir_all(git.join("refs/remotes/origin")).unwrap();
        std::fs::write(git.join("HEAD"), format!("{head}\n")).unwrap();
        if let Some(o) = origin_head {
            std::fs::write(git.join("refs/remotes/origin/HEAD"), format!("ref: refs/remotes/origin/{o}\n")).unwrap();
        }
        std::fs::write(git.join("config"), config).unwrap();
        dir
    }

    #[test]
    fn the_branch_a_push_lands_on_is_read_from_the_repositorys_files() {
        let plain = "[core]\n\tbare = false\n[remote \"origin\"]\n\turl = git@github.com:me/app.git\n";
        let r = repo("ref: refs/heads/feature/x", Some("live"), plain);
        let sub = r.path().join("src/deep");
        std::fs::create_dir_all(&sub).unwrap();
        let head = git_head_with(&sub, &[]).unwrap();
        assert_eq!(
            head,
            GitHead {
                branch: Some("feature/x".into()),
                default: Some("live".into()),
                remote: None,
                remotes: [("origin".to_owned(), "git@github.com:me/app.git".to_owned())].into(),
            }
        );
        // detached: no branch
        let r = repo("0123456789abcdef0123456789abcdef01234567", Some("main"), plain);
        assert_eq!(git_head_with(r.path(), &[]).unwrap().branch, None);
        // where a clean configuration sends a push of the branch elsewhere, the branch is not reported
        for config in [
            "[push]\n\tdefault = matching\n",
            "[push]\n\tdefault = upstream\n[branch \"feature/x\"]\n\tmerge = refs/heads/main\n",
        ] {
            let r = repo("ref: refs/heads/feature/x", Some("main"), config);
            let head = git_head_with(r.path(), &[]).unwrap();
            assert_eq!((head.branch, head.default.as_deref()), (None, Some("main")), "{config}");
        }
        // a configuration that does not parse cleanly, includes others or says where pushes go: nothing reported at
        // all (each a way found around the first reading)
        for config in [
            "[remote \"origin\"]\n\tpush = refs/heads/*:refs/heads/main\n",
            "[include]\n\tpath = ~/other.gitconfig\n",
            "[includeIf \"gitdir:~/work/\"]\n\tpath = ~/work.gitconfig\n",
            "[push] default = matching\n",
            "[push]\n\tdefault = \\\nmatching\n",
            "[push]\n\tdefault = sim\\ple\n",
            "[push]\n\tdefault = simple ; matching\n",
            "[push]\n\tdefault = \"simple\" \"matching\"\n",
            "[push]\n\tautoSetupRemote = true\n",
            "\tdefault = matching\n[core]\n",
            "[remote \"or\\\"igin\"]\n\turl = x\n",
            "[push \"x\"] # comment\n",
        ] {
            let r = repo("ref: refs/heads/feature/x", Some("main"), config);
            assert_eq!(git_head_with(r.path(), &[]), None, "{config}");
        }
        // in the sections that can say where a push goes, only the known keys: anything else, nothing reported
        for config in [
            "[remote \"origin\"]\n\turl = x\n\tmirror = true\n",
            "[remote \"origin\"]\n\turl = x\n\tpushurl = y\n",
            "[url \"git@evil:\"]\n\tpushInsteadOf = git@github.com:\n",
            "[branch \"feature/x\"]\n\tpushRemote = other\n",
            "[remote]\n\tpushDefault = other\n",
            "[branch \"feature/x\"]\n\trebase = true\n",
        ] {
            let r = repo("ref: refs/heads/feature/x", Some("main"), config);
            assert_eq!(git_head_with(r.path(), &[]), None, "{config}");
        }
        let known = "[remote \"origin\"]\n\turl = x\n\tfetch = +refs/heads/*:refs/remotes/origin/*\n\
                     [branch \"feature/x\"]\n\tremote = origin\n\tmerge = refs/heads/feature/x\n";
        let r = repo("ref: refs/heads/feature/x", Some("main"), known);
        assert_eq!(git_head_with(r.path(), &[]).unwrap().branch.as_deref(), Some("feature/x"));
        // a configuration file that is there but cannot be read as one (a folder, too large): nothing reported
        let r = repo("ref: refs/heads/feature/x", Some("main"), plain);
        let folder = r.path().join("folder.gitconfig");
        std::fs::create_dir_all(&folder).unwrap();
        assert_eq!(git_head_with(r.path(), &[folder]), None, "a folder where a file is expected");
        let large = r.path().join("large.gitconfig");
        std::fs::write(&large, format!("[core]\n{}", "\t# padding\n".repeat(8_000))).unwrap();
        assert_eq!(git_head_with(r.path(), &[large]), None, "over the size cap");
        assert!(git_head_with(r.path(), &[r.path().join("missing.gitconfig")]).is_some(), "a missing file is skipped");
        // a branch section that matches only by case (git reads `[branch.Name]` without regard to it): nothing reported
        for (head_ref, config) in [
            ("ref: refs/heads/Feature/X", "[branch \"feature/x\"]\n\tmerge = refs/heads/main\n"),
            ("ref: refs/heads/Feature/X", "[branch.Feature/X]\n\tmerge = refs/heads/main\n"),
        ] {
            let r = repo(head_ref, Some("main"), config);
            assert_eq!(git_head_with(r.path(), &[]), None, "{config}");
        }
        // two lines in HEAD, or a HEAD that is no symbolic ref: no branch
        let r = repo("ref: refs/heads/feature/x\nref: refs/heads/main", Some("main"), plain);
        assert_eq!(git_head_with(r.path(), &[]).unwrap().branch, None);
        // upstream to the branch of the same name, simple, current: reported
        for config in [
            "[push]\n\tdefault = upstream\n[branch \"feature/x\"]\n\tmerge = refs/heads/feature/x\n",
            "[push]\n\tdefault = simple\n",
            "[push]\n\tdefault = current\n",
        ] {
            let r = repo("ref: refs/heads/feature/x", None, config);
            assert_eq!(git_head_with(r.path(), &[]).unwrap().branch.as_deref(), Some("feature/x"), "{config}");
        }
        // the user's own configuration counts as much as the repository's
        let r = repo("ref: refs/heads/feature/x", None, plain);
        let global = r.path().join("global.gitconfig");
        std::fs::write(&global, "[push]\n\tdefault = matching\n").unwrap();
        assert_eq!(git_head_with(r.path(), &[global]).unwrap().branch, None);
        // a worktree: `.git` is a file naming its git directory
        let main = repo("ref: refs/heads/main", Some("main"), plain);
        let wt_git = main.path().join(".git/worktrees/wt");
        std::fs::create_dir_all(&wt_git).unwrap();
        std::fs::write(wt_git.join("HEAD"), "ref: refs/heads/feature/y\n").unwrap();
        std::fs::write(wt_git.join("commondir"), "../..\n").unwrap();
        let wt = tempfile::tempdir().unwrap();
        std::fs::write(wt.path().join(".git"), format!("gitdir: {}\n", wt_git.display())).unwrap();
        assert_eq!(
            git_head_with(wt.path(), &[]).unwrap(),
            GitHead {
                branch: Some("feature/y".into()),
                default: Some("main".into()),
                remote: None,
                remotes: [("origin".to_owned(), "git@github.com:me/app.git".to_owned())].into(),
            }
        );
        // `branch.<name>.remote` is where a push with no repository argument goes. Only origin matches the default
        // read above; another remote is remembered so the question can stay silent.
        let upstream = "[branch \"feature/x\"]\n\tremote = upstream\n\tmerge = refs/heads/feature/x\n";
        let r = repo("ref: refs/heads/feature/x", Some("live"), upstream);
        assert_eq!(git_head_with(r.path(), &[]).unwrap().remote.as_deref(), Some("upstream"));
        // not a repository
        let none = tempfile::tempdir().unwrap();
        assert!(git_head_with(none.path(), &[]).is_none_or(|h| h.branch.is_none()));
    }

    fn push_call(command: &str) -> HookCall {
        HookCall {
            harness: Harness::ClaudeCode,
            event: Event::PreToolUse,
            action: Action::Command(command.to_owned()),
            cwd: Some("/home/me/app".to_owned()),
        }
    }

    fn reported_lines(command: &str, head: &GitHead) -> Vec<String> {
        let (_, q) = question_in(&push_call(command), &GuardConfig::default(), Some(head)).expect("asked");
        q.detail.unwrap().lines().map(str::to_owned).collect()
    }

    #[test]
    fn a_push_question_reports_the_branch_in_lines_a_command_cannot_write() {
        // a force push is always asked about; its command tries to write a branch line of its own
        let head = GitHead {
            branch: Some("feature/x".into()),
            default: Some("live".into()),
            remote: None,
            ..GitHead::default()
        };
        let lines = reported_lines("git push --force origin HEAD", &head);
        let command = lines.iter().position(|l| l == "Command:").unwrap();
        let branch = lines.iter().position(|l| l == "Branch: feature/x").unwrap();
        assert!(branch < command, "{lines:?}");
        assert!(lines.iter().any(|l| l == "Default branch: live"));
        // a second line cannot become the header, and it hides where the push goes, so nothing is reported
        let forged = reported_lines("git push --force origin HEAD\nBranch: main", &head);
        let command = forged.iter().position(|l| l == "Command:").unwrap();
        assert!(forged.iter().take(command).all(|l| !l.starts_with("Branch:")), "{forged:?}");
        assert!(forged.iter().take(command).all(|l| !l.starts_with("Default branch:")), "{forged:?}");
        assert!(forged.iter().skip(command).any(|l| l.contains("Branch: main")));
    }

    /// Lines of the question that come before `Command:`. The default guard asks about a force push, and `--force`
    /// does not name a remote, so the same decision as the plain line is visible in the question.
    fn header_lines(command: &str, head: &GitHead) -> Vec<String> {
        let lines = reported_lines(command, head);
        let at = lines.iter().position(|l| l == "Command:").expect("command line");
        lines.into_iter().take(at).collect()
    }

    /// The default in `refs/remotes/origin/HEAD` is origin's. A push to origin reports the checked-out branch and that
    /// name (the phone adds it to the built-in protected names). A push to any other remote reports neither, so the
    /// phone asks. Whatever the file says, the checked-out branch is still reported for origin: the file cannot hide it.
    #[test]
    fn only_a_push_to_origin_reports_the_branch_the_default_was_read_for() {
        let on_main = GitHead {
            branch: Some("main".into()),
            default: Some("decoy".into()),
            remote: None,
            ..GitHead::default()
        };
        let feature = GitHead {
            branch: Some("feature/x".into()),
            default: Some("live".into()),
            remote: None,
            ..GitHead::default()
        };
        let upstream = GitHead {
            remote: Some("upstream".into()),
            ..feature.clone()
        };
        // (plain line, the force-push the guard asks about, head, whether the branch is reported)
        let cases: &[(&str, &str, &GitHead, bool)] = &[
            ("git push origin HEAD", "git push --force origin HEAD", &on_main, true),
            ("git push", "git push --force", &feature, true),
            ("git push origin", "git push --force origin", &feature, true),
            ("git push -u origin feature/x", "git push -u --force origin feature/x", &feature, true),
            ("git push origin HEAD:feature/y", "git push --force origin HEAD:feature/y", &feature, true),
            ("git push origin feature/x", "git push --force origin feature/x", &upstream, true),
            ("git push upstream feature/x", "git push --force upstream feature/x", &feature, false),
            ("git push other", "git push --force other", &feature, false),
            (
                "git push github.com:me/app.git feature/x",
                "git push --force github.com:me/app.git feature/x",
                &feature,
                false,
            ),
            ("git push HEAD:feature/x", "git push --force HEAD:feature/x", &feature, false),
            ("git push", "git push --force", &upstream, false),
            ("git push -u HEAD", "git push -u --force HEAD", &upstream, false),
            ("git push --repo upstream feature/x", "git push --force --repo upstream feature/x", &feature, false),
            ("git -C ../other push origin feature/x", "git -C ../other push --force origin feature/x", &feature, false),
        ];
        for &(plain, asked, head, reports) in cases {
            assert_eq!(push_targets_origin(plain, head.remote.as_deref()), reports, "{plain}");
            let header = header_lines(asked, head);
            let saw = header.iter().any(|l| l.starts_with("Branch:") || l.starts_with("Default branch:"));
            assert_eq!(saw, reports, "{asked}: {header:?}");
            if reports {
                let branch = head.branch.as_deref().unwrap();
                let default = head.default.as_deref().unwrap();
                assert!(header.iter().any(|l| l == &format!("Branch: {branch}")), "{asked}: {header:?}");
                assert!(header.iter().any(|l| l == &format!("Default branch: {default}")), "{asked}: {header:?}");
            }
        }
    }

    #[test]
    fn a_command_of_any_length_makes_a_question_the_server_takes() {
        let cmd = format!("git push --force origin {}", "feature/a-very-long-branch-name-".repeat(40));
        let call = HookCall {
            harness: Harness::ClaudeCode,
            event: Event::PreToolUse,
            action: Action::Command(cmd.clone()),
            cwd: Some("/home/me/app".to_owned()),
        };
        let (_, q) = question(&call, &GuardConfig::default()).unwrap();
        assert!(q.question.ends_with('…'));
        assert!(q.detail.as_deref().unwrap().contains(&cmd), "the whole command is in the detail");
        let spec = reins_proto::connector::spec_for_tool(crate::ask::ASK_TOOL).unwrap();
        let mut args = json!({"question": q.question, "client_key": "k".repeat(43), "nonce": "n"});
        args["detail"] = json!(q.detail);
        args["topic"] = json!(q.topic);
        spec.parse(&args).unwrap();
    }
}
