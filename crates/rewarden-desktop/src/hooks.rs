//! `rewarden hook <harness>`: a harness's pre-tool hook. Reads the hook's JSON on stdin, and when the command or file
//! matches the guard rules (`[guard]`) asks the phone (or the person at the computer) and answers in that harness's
//! format; anything else is allowed silently.
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
//!   (`tool_name` `Write`/`Delete`, `tool_input.file_path`), which know `allow`/`deny` only.
//! - Gemini CLI `BeforeTool` (<https://geminicli.com/docs/hooks/reference/>): `run_shell_command`
//!   (`tool_input.command`), `write_file`/`replace`/`read_file` (`tool_input.file_path`); answer `{"decision":
//!   "allow"|"deny", "reason": …}`, no output for no decision.

use std::fmt::Write as _;
use std::time::Duration;

use serde_json::{Value, json};

use crate::ask::{Answer, Question};
use crate::auth::prompt::Prompter;
use crate::config::{Config, Paths};
use crate::guard::{GuardConfig, Match, OnNoAnswer};
use crate::harness::Harness;

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
    CursorShell,
    /// Cursor's `beforeReadFile` and `preToolUse`: allow or deny only.
    CursorFile,
    GeminiBeforeTool,
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
            let event = if event_name.is_empty() || event_name == "PreToolUse" {
                Event::PreToolUse
            } else {
                Event::Unknown
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
            "beforeReadFile" => (Event::CursorFile, files(false, file_in(&v))),
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
                (Event::CursorFile, action)
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
        action: if event == Event::Unknown {
            Action::Other
        } else {
            action
        },
        cwd: str_at(&v, "/cwd").map(str::to_owned),
    })
}

/// The rule the call matches and the question to ask about it.
#[must_use]
pub fn question(call: &HookCall, guard: &GuardConfig) -> Option<(Match, Question)> {
    let who = call.harness.label();
    let (m, question, mut detail) = match &call.action {
        Action::Command(c) => {
            let m = guard.check_command(c)?;
            let one_line = c.split_whitespace().collect::<Vec<_>>().join(" ");
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
    let _infallible = write!(detail, "\nRule: {}", m.pattern);
    let q = Question::new(&question, Some(&detail), Some(&m.topic)).ok()?;
    Some((m, q))
}

/// The verdict for an answer.
#[must_use]
pub fn verdict(answer: Answer, on_no_answer: OnNoAnswer) -> Verdict {
    match answer {
        Answer::Yes => Verdict::Allow("Approved through Rewarden.".to_owned()),
        Answer::No(why) => Verdict::Deny(format!("Rewarden: {why} Do not try another way around this.")),
        Answer::Unanswered(why) => match on_no_answer {
            OnNoAnswer::Deny => Verdict::Deny(format!(
                "Rewarden: not allowed, nobody answered ({why}). Ask the user to approve it, then try again."
            )),
            OnNoAnswer::Ask => Verdict::Ask(format!("Rewarden got no answer ({why}).")),
        },
    }
}

/// What to print on stdout (if anything) and the exit code.
#[must_use]
pub fn render(call: &HookCall, verdict: &Verdict) -> (Option<String>, u8) {
    let out = |v: Value| (Some(v.to_string()), 0);
    match (call.event, verdict) {
        (Event::CursorShell | Event::CursorFile, Verdict::Unmatched) => out(json!({"permission": "allow"})),
        (Event::Unknown, _) | (_, Verdict::Unmatched) => (None, 0),
        (Event::PreToolUse, v) => {
            let (decision, reason) = match v {
                Verdict::Allow(r) => ("allow", r),
                Verdict::Deny(r) => ("deny", r),
                Verdict::Ask(_) if call.harness == Harness::Codex => return (None, 0),
                Verdict::Ask(r) => ("ask", r),
                Verdict::Unmatched => return (None, 0),
            };
            out(json!({"hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": decision,
                "permissionDecisionReason": reason,
            }}))
        }
        (Event::CursorShell | Event::CursorFile, v) => {
            let (permission, message) = match v {
                Verdict::Allow(r) => ("allow", r),
                Verdict::Ask(r) if call.event == Event::CursorShell => ("ask", r),
                Verdict::Deny(r) | Verdict::Ask(r) => ("deny", r),
                Verdict::Unmatched => ("allow", &String::new()),
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
        Err(e) => return (None, 2, Some(format!("rewarden hook: {e}; not allowed."))),
    };
    let Some((m, q)) = question(&call, &config.guard) else {
        let (out, code) = render(&call, &Verdict::Unmatched);
        return (out, code, None);
    };
    log::info!("hook {}: asking about `{}`", harness.id(), m.pattern);
    let timeout = Duration::from_secs(config.guard.timeout_secs);
    let answer = crate::ask::ask(paths, config, &q, timeout, false, prompter).await;
    let v = verdict(answer, config.guard.on_no_answer);
    let (out, code) = render(&call, &v);
    (out, code, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_patch_names_its_files() {
        let patch = "*** Begin Patch\n*** Update File: src/a.rs\n@@\n-x\n+y\n*** Add File: .env\n+K=V\n*** Delete File: old.txt\n*** End Patch";
        assert_eq!(patch_files(patch), vec!["src/a.rs", ".env", "old.txt"]);
    }

    #[test]
    fn verdicts_follow_the_answer_and_the_no_answer_setting() {
        assert!(matches!(verdict(Answer::Yes, OnNoAnswer::Deny), Verdict::Allow(_)));
        assert!(matches!(verdict(Answer::No("x".into()), OnNoAnswer::Ask), Verdict::Deny(_)));
        assert!(matches!(verdict(Answer::Unanswered("t".into()), OnNoAnswer::Deny), Verdict::Deny(_)));
        assert!(matches!(verdict(Answer::Unanswered("t".into()), OnNoAnswer::Ask), Verdict::Ask(_)));
    }
}
