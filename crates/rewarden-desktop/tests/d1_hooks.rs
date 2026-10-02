//! `rewarden hook <harness>`: each harness's hook input (fixtures in its documented format), the question the phone
//! gets, and the answer in that harness's exact output format.

mod d1_mock;

use std::io::Write as _;

use d1_mock::{Mock, Step, logged_in};
use rewarden_desktop::auth::prompt::NoPrompter;
use rewarden_desktop::config::Config;
use rewarden_desktop::guard::OnNoAnswer;
use rewarden_desktop::harness::Harness;
use rewarden_desktop::hooks;
use serde_json::{Value, json};

const CLAUDE_BASH: &str = r#"{
  "session_id": "abc123",
  "transcript_path": "/home/me/.claude/projects/app/abc123.jsonl",
  "cwd": "/home/me/app",
  "permission_mode": "default",
  "hook_event_name": "PreToolUse",
  "tool_name": "Bash",
  "tool_input": {"command": "git push --force origin main", "description": "Force push", "timeout": 120000},
  "tool_use_id": "toolu_01"
}"#;

const CLAUDE_READ_ENV: &str = r#"{"session_id": "abc123", "cwd": "/home/me/app", "hook_event_name": "PreToolUse",
  "tool_name": "Read", "tool_input": {"file_path": "/home/me/app/.env"}, "tool_use_id": "toolu_02"}"#;

const CLAUDE_LS: &str = r#"{"session_id": "abc123", "cwd": "/home/me/app", "hook_event_name": "PreToolUse",
  "tool_name": "Bash", "tool_input": {"command": "ls -la"}, "tool_use_id": "toolu_03"}"#;

const CODEX_BASH: &str = r#"{"session_id": "s1", "turn_id": "t1", "cwd": "/home/me/app", "hook_event_name": "PreToolUse",
  "permission_mode": "default", "tool_name": "Bash", "tool_use_id": "call_1",
  "tool_input": {"command": "terraform apply -auto-approve"}}"#;

const CODEX_PATCH: &str = r#"{"session_id": "s1", "turn_id": "t1", "cwd": "/home/me/app", "hook_event_name": "PreToolUse",
  "tool_name": "apply_patch", "tool_use_id": "call_2",
  "tool_input": {"command": "*** Begin Patch\n*** Add File: .env.production\n+SECRET=1\n*** End Patch\n"}}"#;

const CURSOR_SHELL: &str = r#"{"conversation_id": "c1", "generation_id": "g1", "model": "auto",
  "hook_event_name": "beforeShellExecution", "cursor_version": "2.1.0", "workspace_roots": ["/home/me/app"],
  "command": "kubectl delete deployment web", "cwd": "/home/me/app", "sandbox": false}"#;

const CURSOR_READ: &str = r#"{"conversation_id": "c1", "generation_id": "g1", "hook_event_name": "beforeReadFile",
  "workspace_roots": ["/home/me/app"], "file_path": "/home/me/.ssh/id_ed25519", "content": "-----BEGIN", "attachments": []}"#;

const CURSOR_READ_OK: &str = r#"{"conversation_id": "c1", "hook_event_name": "beforeReadFile",
  "file_path": "/home/me/app/src/main.rs", "content": "fn main() {}", "attachments": []}"#;

const CURSOR_WRITE: &str = r#"{"conversation_id": "c1", "hook_event_name": "preToolUse", "tool_name": "Write",
  "tool_input": {"file_path": "/home/me/app/certs/server.key", "contents": "x"}, "tool_use_id": "t", "cwd": "/home/me/app"}"#;

const GEMINI_SHELL: &str = r#"{"session_id": "g1", "transcript_path": "/tmp/t.json", "cwd": "/home/me/app",
  "hook_event_name": "BeforeTool", "timestamp": "2026-09-30T10:00:00Z", "tool_name": "run_shell_command",
  "tool_input": {"command": "rm -rf build dist", "description": "clean"}}"#;

const GEMINI_READ: &str = r#"{"session_id": "g1", "cwd": "/home/me/app", "hook_event_name": "BeforeTool",
  "tool_name": "read_file", "tool_input": {"file_path": "/home/me/app/README.md"}}"#;

struct Run {
    out: Option<Value>,
    code: u8,
}

async fn hook(mock: &Mock, harness: Harness, input: &str, config: &Config) -> (Run, Option<Value>) {
    let app = logged_in(mock, 3600);
    let before = mock.with(|s| s.calls.len());
    let (out, code, err) = hooks::run(harness, input.as_bytes(), &app.paths, config, &NoPrompter).await;
    assert_eq!(err, None);
    let asked = mock.with(|s| (s.calls.len() > before).then(|| s.calls.last().cloned()).flatten());
    (
        Run {
            out: out.map(|o| serde_json::from_str(&o).unwrap()),
            code,
        },
        asked,
    )
}

fn claude(decision: &str, reason_has: &str, run: &Run) {
    assert_eq!(run.code, 0);
    let out = run.out.as_ref().expect("a decision");
    assert_eq!(out["hookSpecificOutput"]["hookEventName"], "PreToolUse");
    assert_eq!(out["hookSpecificOutput"]["permissionDecision"], decision, "{out}");
    let reason = out["hookSpecificOutput"]["permissionDecisionReason"].as_str().unwrap();
    assert!(reason.contains(reason_has), "{reason}");
}

#[tokio::test]
async fn claude_code_pre_tool_use() {
    let mock = Mock::start().await;
    let config = Config::default();
    let (run, asked) = hook(&mock, Harness::ClaudeCode, CLAUDE_BASH, &config).await;
    claude("allow", "Approved", &run);
    let asked = asked.unwrap();
    assert_eq!(asked["question"], "Claude Code wants to run: git push --force origin main");
    assert_eq!(asked["topic"], "command:git push --force*");
    let detail = asked["detail"].as_str().unwrap();
    assert!(detail.contains("git push --force origin main") && detail.contains("In: /home/me/app"), "{detail}");

    mock.plan(&[Step::Denied(Some("Not on main."))]);
    let (run, _) = hook(&mock, Harness::ClaudeCode, CLAUDE_BASH, &config).await;
    claude("deny", "Not on main.", &run);

    mock.plan(&[Step::Approve]);
    let (run, asked) = hook(&mock, Harness::ClaudeCode, CLAUDE_READ_ENV, &config).await;
    claude("allow", "Approved", &run);
    let asked = asked.unwrap();
    assert_eq!(asked["question"], "Claude Code wants to read /home/me/app/.env");
    assert_eq!(asked["topic"], "file:.env");

    // No answer: refused by default, or left to Claude Code's own prompt.
    mock.plan(&[Step::Error("The phone is offline.")]);
    let (run, _) = hook(&mock, Harness::ClaudeCode, CLAUDE_BASH, &config).await;
    claude("deny", "nobody answered (The phone is offline.)", &run);
    let mut ask = Config::default();
    ask.guard.on_no_answer = OnNoAnswer::Ask;
    mock.plan(&[Step::Error("The phone is offline.")]);
    let (run, _) = hook(&mock, Harness::ClaudeCode, CLAUDE_BASH, &ask).await;
    claude("ask", "no answer", &run);

    // Nothing risky: no output, no question.
    let (run, asked) = hook(&mock, Harness::ClaudeCode, CLAUDE_LS, &config).await;
    assert_eq!((run.out, run.code, asked), (None, 0, None));
    // Another event than PreToolUse: no decision.
    let post = CLAUDE_BASH.replace("\"PreToolUse\"", "\"PostToolUse\"");
    let (run, asked) = hook(&mock, Harness::ClaudeCode, &post, &config).await;
    assert_eq!((run.out, run.code, asked), (None, 0, None));
}

#[tokio::test]
async fn codex_pre_tool_use() {
    let mock = Mock::start().await;
    let config = Config::default();
    let (run, asked) = hook(&mock, Harness::Codex, CODEX_BASH, &config).await;
    claude("allow", "Approved", &run);
    assert_eq!(asked.unwrap()["topic"], "command:terraform apply");

    mock.plan(&[Step::Denied(None)]);
    let (run, asked) = hook(&mock, Harness::Codex, CODEX_PATCH, &config).await;
    claude("deny", "Denied on your phone.", &run);
    assert_eq!(asked.unwrap()["question"], "Codex wants to change .env.production");

    // Codex has no "ask": leaving it to Codex is no output.
    let mut ask = Config::default();
    ask.guard.on_no_answer = OnNoAnswer::Ask;
    mock.plan(&[Step::Error("offline")]);
    let (run, _) = hook(&mock, Harness::Codex, CODEX_BASH, &ask).await;
    assert_eq!((run.out, run.code), (None, 0));
}

#[tokio::test]
async fn cursor_before_shell_execution_and_file_hooks() {
    let mock = Mock::start().await;
    let config = Config::default();
    let (run, asked) = hook(&mock, Harness::Cursor, CURSOR_SHELL, &config).await;
    assert_eq!(run.code, 0);
    let out = run.out.unwrap();
    assert_eq!(out["permission"], "allow");
    assert_eq!(asked.unwrap()["question"], "Cursor wants to run: kubectl delete deployment web");

    mock.plan(&[Step::Refuse]);
    let (run, _) = hook(&mock, Harness::Cursor, CURSOR_SHELL, &config).await;
    let out = run.out.unwrap();
    assert_eq!(out["permission"], "deny");
    assert!(out["agent_message"].as_str().unwrap().contains("Denied on your phone."));
    assert_eq!(out["user_message"], out["agent_message"]);

    let mut ask = Config::default();
    ask.guard.on_no_answer = OnNoAnswer::Ask;
    mock.plan(&[Step::Error("offline")]);
    let (run, _) = hook(&mock, Harness::Cursor, CURSOR_SHELL, &ask).await;
    assert_eq!(run.out.unwrap()["permission"], "ask");

    mock.plan(&[Step::Denied(None)]);
    let (run, asked) = hook(&mock, Harness::Cursor, CURSOR_READ, &config).await;
    assert_eq!(run.out.unwrap()["permission"], "deny");
    assert_eq!(asked.unwrap()["topic"], "file:id_ed25519");

    // `beforeReadFile` and `preToolUse` know no "ask": without an answer they refuse.
    mock.plan(&[Step::Error("offline")]);
    let (run, asked) = hook(&mock, Harness::Cursor, CURSOR_WRITE, &ask).await;
    assert_eq!(run.out.unwrap()["permission"], "deny");
    assert_eq!(asked.unwrap()["question"], "Cursor wants to change /home/me/app/certs/server.key");

    // Unmatched: Cursor gets an explicit allow (its file hooks read a missing answer as a refusal).
    let (run, asked) = hook(&mock, Harness::Cursor, CURSOR_READ_OK, &config).await;
    assert_eq!((run.out, run.code, asked), (Some(json!({"permission": "allow"})), 0, None));
}

#[tokio::test]
async fn gemini_before_tool() {
    let mock = Mock::start().await;
    let config = Config::default();
    let (run, asked) = hook(&mock, Harness::Gemini, GEMINI_SHELL, &config).await;
    assert_eq!(run.code, 0);
    assert_eq!(run.out.unwrap()["decision"], "allow");
    assert_eq!(asked.unwrap()["question"], "Gemini CLI wants to run: rm -rf build dist");

    mock.plan(&[Step::Denied(Some("Keep dist."))]);
    let (run, _) = hook(&mock, Harness::Gemini, GEMINI_SHELL, &config).await;
    let out = run.out.unwrap();
    assert_eq!(out["decision"], "deny");
    assert!(out["reason"].as_str().unwrap().contains("Keep dist."));

    let mut ask = Config::default();
    ask.guard.on_no_answer = OnNoAnswer::Ask;
    mock.plan(&[Step::Error("offline")]);
    let (run, _) = hook(&mock, Harness::Gemini, GEMINI_SHELL, &ask).await;
    assert_eq!((run.out, run.code), (None, 0));

    let (run, asked) = hook(&mock, Harness::Gemini, GEMINI_READ, &config).await;
    assert_eq!((run.out, run.code, asked), (None, 0, None));
}

/// The binary: unreadable input is refused with exit 2 (every harness blocks on it), plain commands pass silently.
#[test]
fn the_command_line_reads_stdin_and_answers() {
    let dir = tempfile::tempdir().unwrap();
    let run = |harness: &str, input: &str| {
        let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_rewarden"))
            .args(["hook", harness])
            .env("REWARDEN_CONFIG_DIR", dir.path().join("config"))
            .env("REWARDEN_STATE_DIR", dir.path().join("state"))
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
        let out = child.wait_with_output().unwrap();
        (out.status.code().unwrap(), String::from_utf8(out.stdout).unwrap(), String::from_utf8(out.stderr).unwrap())
    };
    assert_eq!(run("claude-code", CLAUDE_LS), (0, String::new(), String::new()));
    assert_eq!(run("cursor", CURSOR_READ_OK).1.trim(), r#"{"permission":"allow"}"#);
    let (code, out, err) = run("gemini", "not json");
    assert_eq!((code, out.as_str()), (2, ""));
    assert!(err.contains("not JSON") && err.contains("not allowed"), "{err}");
    let (code, _, err) = run("vim", "{}");
    assert_ne!(code, 0);
    assert!(err.contains("unknown harness"), "{err}");
}
