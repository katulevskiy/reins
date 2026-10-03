//! `reins harness add|remove|list` on temporary home directories: what `add` writes in each harness's format, and
//! that `remove` gives back every file that existed byte for byte and deletes the ones `add` created.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use reins_desktop::config::Paths;
use reins_desktop::harness::{self, Harness, Setup};
use serde_json::{Value, json};

const EXE: &str = "/opt/reins/bin/reins";

struct Env {
    home: tempfile::TempDir,
    state: tempfile::TempDir,
}

impl Env {
    fn new() -> Self {
        Self {
            home: tempfile::tempdir().unwrap(),
            state: tempfile::tempdir().unwrap(),
        }
    }

    fn setup(&self) -> Setup {
        Setup {
            home: self.home.path().to_path_buf(),
            exe: PathBuf::from(EXE),
            hook_timeout_secs: 120,
        }
    }

    fn paths(&self) -> Paths {
        Paths::under(self.state.path())
    }

    fn file(&self, rel: &str) -> PathBuf {
        self.home.path().join(rel)
    }

    fn write(&self, rel: &str, text: &str) {
        let p = self.file(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    fn json(&self, rel: &str) -> Value {
        serde_json::from_str(&std::fs::read_to_string(self.file(rel)).unwrap()).unwrap()
    }

    fn add(&self, h: Harness) -> Vec<String> {
        harness::add(&self.paths(), &self.setup(), h).unwrap()
    }

    fn remove(&self, h: Harness) -> Vec<String> {
        harness::remove(&self.paths(), &self.setup(), h).unwrap()
    }

    /// Every file under the home directory with its bytes (directories end with `/`).
    fn snapshot(&self) -> BTreeMap<String, Vec<u8>> {
        fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
            for e in std::fs::read_dir(dir).unwrap().flatten() {
                let p = e.path();
                let rel = p.strip_prefix(root).unwrap().display().to_string();
                if p.is_dir() {
                    out.insert(format!("{rel}/"), Vec::new());
                    walk(root, &p, out);
                } else {
                    out.insert(rel, std::fs::read(&p).unwrap());
                }
            }
        }
        let mut out = BTreeMap::new();
        walk(self.home.path(), self.home.path(), &mut out);
        out
    }
}

fn hook_cmd(h: &str) -> String {
    format!("{EXE} hook {h}")
}

#[test]
fn on_a_fresh_home_add_writes_each_format_and_remove_leaves_nothing() {
    let env = Env::new();
    for h in Harness::ALL {
        let report = env.add(h);
        assert!(report.iter().any(|l| l.contains("MCP server")), "{report:?}");
        assert!(report.iter().any(|l| l.contains("hook")), "{report:?}");
    }
    let args = |label: &str| json!(["mcp", "--via", label]);

    let claude = env.json(".claude.json");
    assert_eq!(
        claude,
        json!({"mcpServers": {"reins": {"type": "stdio", "command": EXE, "args": args("Claude Code"), "env": {}}}})
    );
    let settings = env.json(".claude/settings.json");
    let pre = &settings["hooks"]["PreToolUse"][0];
    assert_eq!(pre["matcher"], "Bash|Edit|Write|MultiEdit|NotebookEdit|Read");
    assert_eq!(pre["hooks"], json!([{"type": "command", "command": hook_cmd("claude-code"), "timeout": 150}]));

    let codex: toml::Table = toml::from_str(&std::fs::read_to_string(env.file(".codex/config.toml")).unwrap()).unwrap();
    assert_eq!(codex["mcp_servers"]["reins"]["command"].as_str(), Some(EXE));
    assert_eq!(
        codex["mcp_servers"]["reins"]["args"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect::<Vec<_>>(),
        ["mcp", "--via", "Codex"]
    );
    let codex_hooks = env.json(".codex/hooks.json");
    assert_eq!(codex_hooks["hooks"]["PreToolUse"][0]["matcher"], "^(Bash|apply_patch)$");
    assert_eq!(codex_hooks["hooks"]["PreToolUse"][0]["hooks"][0]["command"], hook_cmd("codex"));

    let gemini = env.json(".gemini/settings.json");
    assert_eq!(gemini["mcpServers"]["reins"], json!({"command": EXE, "args": args("Gemini CLI")}));
    let before = &gemini["hooks"]["BeforeTool"][0];
    assert_eq!(before["matcher"], "^(run_shell_command|write_file|replace|read_file)$");
    assert_eq!(before["hooks"][0]["timeout"], 150_000, "Gemini counts milliseconds");
    assert_eq!(before["hooks"][0]["command"], hook_cmd("gemini"));

    assert_eq!(
        env.json(".cursor/mcp.json"),
        json!({"mcpServers": {"reins": {"type": "stdio", "command": EXE, "args": args("Cursor")}}})
    );
    let cursor = env.json(".cursor/hooks.json");
    assert_eq!(cursor["version"], 1);
    let item = json!({"command": hook_cmd("cursor"), "timeout": 150});
    assert_eq!(cursor["hooks"]["beforeShellExecution"], json!([item]));
    assert_eq!(cursor["hooks"]["beforeReadFile"], json!([item]));
    assert_eq!(cursor["hooks"]["preToolUse"][0]["matcher"], "Write|Delete");

    // Adding again changes nothing; list sees it.
    let snap = env.snapshot();
    for h in Harness::ALL {
        assert!(env.add(h).iter().all(|l| l.contains("already")));
        let listed = harness::list(&env.paths(), &env.setup(), h).unwrap().join("\n");
        assert!(listed.contains("MCP server: yes") && listed.contains("hook:       yes"), "{listed}");
    }
    assert_eq!(env.snapshot(), snap);

    for h in Harness::ALL {
        assert!(!env.remove(h).is_empty());
        let listed = harness::list(&env.paths(), &env.setup(), h).unwrap().join("\n");
        assert!(listed.contains("MCP server: no") && listed.contains("hook:       no"), "{listed}");
    }
    assert!(env.snapshot().is_empty(), "{:?}", env.snapshot().keys());
    assert!(env.remove(Harness::Cursor).is_empty(), "nothing left to remove");
}

const CLAUDE_JSON: &str = r#"{
  "numStartups": 42,
  "theme": "dark",
  "projects": {
    "/home/me/app": {
      "allowedTools": [],
      "mcpServers": {}
    }
  },
  "mcpServers": {
    "github": {
      "type": "http",
      "url": "https://api.githubcopilot.com/mcp/"
    }
  },
  "userID": "ü-1"
}"#;

const CLAUDE_SETTINGS: &str = r#"{
  "permissions": {
    "allow": ["Bash(npm run test:*)"]
  },
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Write",
        "hooks": [{ "type": "command", "command": "~/bin/lint.sh" }]
      }
    ],
    "Stop": []
  }
}
"#;

const CODEX_TOML: &str = "model = \"gpt-5-codex\"   # default model\napproval_policy = \"on-request\"\n\n[mcp_servers.docs]\ncommand = \"npx\"\nargs = [\"-y\", \"docs-mcp\"]\n";

const GEMINI_SETTINGS: &str = "{\n    // My Gemini settings\n    \"theme\": \"GitHub\",\n    \"hooks\": {},\n    \"mcpServers\": {\n        \"local\": { \"command\": \"node\", \"args\": [\"srv.js\"] }, /* keep */\n    },\n}\n";

const CURSOR_MCP: &str = "{\"mcpServers\":{}}";

const CURSOR_HOOKS: &str =
    "{\n\t\"version\": 1,\n\t\"hooks\": {\n\t\t\"afterFileEdit\": [{ \"command\": \"./hooks/format.sh\" }]\n\t}\n}\n";

#[test]
fn files_that_existed_are_byte_for_byte_the_same_after_remove() {
    let env = Env::new();
    env.write(".claude.json", CLAUDE_JSON);
    env.write(".claude/settings.json", CLAUDE_SETTINGS);
    env.write(".codex/config.toml", CODEX_TOML);
    env.write(".gemini/settings.json", GEMINI_SETTINGS);
    env.write(".cursor/mcp.json", CURSOR_MCP);
    env.write(".cursor/hooks.json", CURSOR_HOOKS);
    std::fs::create_dir_all(env.file(".codex/sessions")).unwrap();
    let before = env.snapshot();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(env.file(".claude.json"), std::fs::Permissions::from_mode(0o600)).unwrap();
    }

    for h in Harness::ALL {
        env.add(h);
    }
    // The user's settings are all still there, next to the new entries.
    let claude = env.json(".claude.json");
    assert_eq!(claude["mcpServers"]["github"]["type"], "http");
    assert_eq!(claude["mcpServers"]["reins"]["command"], EXE);
    assert_eq!(claude["userID"], "ü-1");
    let settings = env.json(".claude/settings.json");
    assert_eq!(settings["hooks"]["PreToolUse"].as_array().unwrap().len(), 2);
    assert_eq!(settings["hooks"]["PreToolUse"][0]["matcher"], "Write");
    let codex = std::fs::read_to_string(env.file(".codex/config.toml")).unwrap();
    assert!(codex.starts_with(CODEX_TOML) && codex.contains("[mcp_servers.reins]"), "{codex}");
    let gemini = std::fs::read_to_string(env.file(".gemini/settings.json")).unwrap();
    assert!(gemini.contains("// My Gemini settings") && gemini.contains("/* keep */"), "{gemini}");
    assert!(gemini.contains("\n        \"reins\": {\n            \""), "4-space style kept: {gemini}");
    assert!(env.file(".codex/hooks.json").exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(env.file(".claude.json")).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "permissions kept");
    }

    for h in Harness::ALL {
        env.remove(h);
    }
    let after = env.snapshot();
    assert_eq!(after.keys().collect::<Vec<_>>(), before.keys().collect::<Vec<_>>());
    for (name, bytes) in &before {
        assert_eq!(String::from_utf8_lossy(&after[name]), String::from_utf8_lossy(bytes), "{name} is not what it was");
    }
}

#[test]
fn remove_keeps_what_the_harness_or_the_user_added_since() {
    let env = Env::new();
    env.write(".claude.json", CLAUDE_JSON);
    env.add(Harness::ClaudeCode);
    // Claude Code rewrites its file: another server after ours, a new top-level key.
    let mut v = env.json(".claude.json");
    v["mcpServers"]["later"] = json!({"type": "stdio", "command": "x"});
    v["numStartups"] = json!(43);
    env.write(".claude.json", &serde_json::to_string_pretty(&v).unwrap());
    // The user adds another hook after ours.
    let mut s = env.json(".claude/settings.json");
    s["hooks"]["PreToolUse"].as_array_mut().unwrap().push(json!({"matcher": "Read", "hooks": []}));
    s["hooks"]["PostToolUse"] = json!([]);
    env.write(".claude/settings.json", &serde_json::to_string_pretty(&s).unwrap());

    env.remove(Harness::ClaudeCode);
    let v = env.json(".claude.json");
    assert!(v["mcpServers"].get("reins").is_none());
    assert_eq!(v["mcpServers"]["later"]["command"], "x");
    assert_eq!(v["numStartups"], 43);
    let s = env.json(".claude/settings.json");
    assert_eq!(s["hooks"], json!({"PreToolUse": [{"matcher": "Read", "hooks": []}], "PostToolUse": []}));
}

#[test]
fn a_different_entry_of_the_same_name_is_not_overwritten() {
    let env = Env::new();
    let mine = "{\n  \"mcpServers\": {\n    \"reins\": {\"type\": \"http\", \"url\": \"https://rw.example.com/mcp\"}\n  }\n}\n";
    env.write(".cursor/mcp.json", mine);
    let e = harness::add(&env.paths(), &env.setup(), Harness::Cursor).unwrap_err();
    assert!(e.contains("already has a different `reins`"), "{e}");
    assert_eq!(std::fs::read_to_string(env.file(".cursor/mcp.json")).unwrap(), mine);
    assert!(!env.file(".cursor/hooks.json").exists());
    assert!(env.remove(Harness::Cursor).is_empty());
    assert_eq!(std::fs::read_to_string(env.file(".cursor/mcp.json")).unwrap(), mine, "not ours: kept");
    // A file that is not JSON is refused, untouched.
    env.write(".gemini/settings.json", "{ nope");
    assert!(harness::add(&env.paths(), &env.setup(), Harness::Gemini).unwrap_err().contains("settings.json"));
    assert_eq!(std::fs::read_to_string(env.file(".gemini/settings.json")).unwrap(), "{ nope");
}

#[test]
fn without_the_record_remove_still_takes_out_this_apps_entries() {
    let env = Env::new();
    env.write(".gemini/settings.json", "{\n  \"theme\": \"x\"\n}\n");
    env.add(Harness::Gemini);
    std::fs::remove_file(env.paths().state_dir.join("harnesses.json")).unwrap();
    assert!(!env.remove(Harness::Gemini).is_empty());
    let v = env.json(".gemini/settings.json");
    assert_eq!(v["theme"], "x");
    assert!(v["mcpServers"].get("reins").is_none());
    assert_eq!(v["hooks"]["BeforeTool"], json!([]));
}

#[cfg(unix)]
#[test]
fn a_symlinked_settings_file_is_edited_where_it_lives() {
    let env = Env::new();
    let dotfiles = env.home.path().join("dotfiles");
    std::fs::create_dir_all(&dotfiles).unwrap();
    std::fs::write(dotfiles.join("claude.json"), "{}\n").unwrap();
    std::os::unix::fs::symlink(dotfiles.join("claude.json"), env.file(".claude.json")).unwrap();
    env.add(Harness::ClaudeCode);
    assert!(std::fs::symlink_metadata(env.file(".claude.json")).unwrap().file_type().is_symlink());
    assert!(std::fs::read_to_string(dotfiles.join("claude.json")).unwrap().contains("reins"));
    env.remove(Harness::ClaudeCode);
    assert_eq!(std::fs::read_to_string(dotfiles.join("claude.json")).unwrap(), "{}\n");
}

#[test]
fn the_command_line_uses_home_and_this_program() {
    let env = Env::new();
    let run = |args: &[&str]| {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_reins"))
            .args(args)
            .env("HOME", env.home.path())
            .env("USERPROFILE", env.home.path())
            .env("REINS_CONFIG_DIR", env.state.path().join("config"))
            .env("REINS_STATE_DIR", env.state.path().join("state"))
            .output()
            .unwrap();
        assert!(out.status.success(), "{args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8(out.stdout).unwrap()
    };
    let said = run(&["harness", "add", "cursor"]);
    assert!(said.contains("~/.cursor/mcp.json") && said.contains("Restart Cursor"), "{said}");
    let exe = reins_desktop::win::strip_verbatim(&std::fs::canonicalize(env!("CARGO_BIN_EXE_reins")).unwrap());
    assert_eq!(env.json(".cursor/mcp.json")["mcpServers"]["reins"]["command"], exe.display().to_string());
    let listed = run(&["harness", "list"]);
    assert!(listed.contains("cursor") && listed.contains("MCP server: yes (~/.cursor/mcp.json)"), "{listed}");
    assert!(listed.contains("claude-code"), "{listed}");
    run(&["harness", "remove", "cursor"]);
    assert!(env.snapshot().is_empty());
}
