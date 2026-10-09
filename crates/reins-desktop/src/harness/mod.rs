//! `reins harness add|remove|list`: registers `reins mcp --via <harness>` as an MCP server in an AI harness's
//! user settings, and `reins hook <harness>` as its pre-command hook where it has one.
//!
//! | harness     | MCP server                                   | hook                                                    |
//! |-------------|----------------------------------------------|---------------------------------------------------------|
//! | Claude Code | `~/.claude.json` `mcpServers`                | `~/.claude/settings.json` `hooks.PreToolUse`            |
//! | Codex       | `~/.codex/config.toml` `[mcp_servers.…]`     | `~/.codex/hooks.json` `hooks.PreToolUse`                |
//! | Gemini CLI  | `~/.gemini/settings.json` `mcpServers`       | `~/.gemini/settings.json` `hooks.BeforeTool`            |
//! | Cursor      | `~/.cursor/mcp.json` `mcpServers`            | `~/.cursor/hooks.json` `beforeShellExecution`, `beforeReadFile`, `preToolUse` |
//!
//! Every change is one inserted entry (plus the containers it needed), made as a text edit so the rest of the file is
//! untouched, and recorded in `harnesses.json` in the state directory. `remove` takes out exactly those entries (and
//! the containers and files `add` created): a file that existed before is byte for byte what it was, as long as nobody
//! else edited around the entry meanwhile.

pub mod detect;
pub mod jsonedit;

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::config::Paths;
use jsonedit::{Entry, Node};

/// The MCP server's name in every harness.
pub const SERVER_NAME: &str = "reins";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Harness {
    ClaudeCode,
    Codex,
    Gemini,
    Cursor,
}

impl Harness {
    pub const ALL: [Self; 4] = [Self::ClaudeCode, Self::Codex, Self::Gemini, Self::Cursor];

    /// The name on the command line (`claude-code`).
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude-code",
            Self::Codex => "codex",
            Self::Gemini => "gemini",
            Self::Cursor => "cursor",
        }
    }

    /// The name people know (`Claude Code`), sent as `X-Reins-Via`.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::ClaudeCode => "Claude Code",
            Self::Codex => "Codex",
            Self::Gemini => "Gemini CLI",
            Self::Cursor => "Cursor",
        }
    }
}

impl std::str::FromStr for Harness {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "claude-code" | "claude" | "claudecode" => Ok(Self::ClaudeCode),
            "codex" => Ok(Self::Codex),
            "gemini" | "gemini-cli" => Ok(Self::Gemini),
            "cursor" => Ok(Self::Cursor),
            other => Err(format!("unknown harness `{other}`: claude-code, codex, gemini or cursor")),
        }
    }
}

/// Where the harnesses keep their settings and what to register.
#[derive(Clone, Debug)]
pub struct Setup {
    /// The home directory the harnesses' settings are under.
    pub home: PathBuf,
    /// This program (what the harness runs).
    pub exe: PathBuf,
    /// How long `reins hook` waits for an answer; the harness's own hook timeout is a little longer.
    pub hook_timeout_secs: u64,
}

impl Setup {
    fn hook_command(&self, h: Harness) -> String {
        let exe = self.exe.to_string_lossy();
        let program = if cfg!(windows) {
            let dirs: Vec<PathBuf> =
                std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
            crate::win::hook_program(&exe, crate::win::dir_on_path(&self.exe, &dirs))
        } else {
            shell_quote(&exe)
        };
        format!("{program} hook {}", h.id())
    }

    fn harness_timeout_secs(&self) -> u64 {
        self.hook_timeout_secs + 30
    }
}

/// `s` as one shell word.
fn shell_quote(s: &str) -> String {
    if !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "/._-+=:@%".contains(c)) {
        s.to_owned()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    Mcp,
    Hook,
    /// Needed by the hook file (Cursor's `version`), not shown.
    Support,
}

/// One entry to have in one file.
#[derive(Clone, Debug)]
enum Target {
    /// Member `key` of the object at `path`. `keep_existing`: any value already there is fine.
    JsonMember {
        file: PathBuf,
        path: Vec<String>,
        key: String,
        value: Value,
        keep_existing: bool,
    },
    /// An item of the array at `path`; an item holding a string that contains `marker` counts as this one.
    JsonItem {
        file: PathBuf,
        path: Vec<String>,
        value: Value,
        marker: String,
    },
    /// A TOML table `[header]` with `body` (`key = value` lines).
    TomlTable {
        file: PathBuf,
        header: String,
        body: String,
    },
}

impl Target {
    fn file(&self) -> &Path {
        match self {
            Self::JsonMember {
                file,
                ..
            }
            | Self::JsonItem {
                file,
                ..
            }
            | Self::TomlTable {
                file,
                ..
            } => file,
        }
    }
}

fn path(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|&p| p.to_owned()).collect()
}

fn targets(h: Harness, s: &Setup) -> Vec<(Role, Target)> {
    let exe = s.exe.to_string_lossy().into_owned();
    let args = json!(["mcp", "--via", h.label()]);
    let hook = s.hook_command(h);
    let secs = s.harness_timeout_secs();
    let home = &s.home;
    match h {
        Harness::ClaudeCode => vec![
            (
                Role::Mcp,
                Target::JsonMember {
                    file: home.join(".claude.json"),
                    path: path(&["mcpServers"]),
                    key: SERVER_NAME.to_owned(),
                    value: json!({"type": "stdio", "command": exe, "args": args, "env": {}}),
                    keep_existing: false,
                },
            ),
            (
                Role::Hook,
                Target::JsonItem {
                    file: home.join(".claude").join("settings.json"),
                    path: path(&["hooks", "PreToolUse"]),
                    value: json!({
                        "matcher": "Bash|Edit|Write|MultiEdit|NotebookEdit|Read",
                        "hooks": [{"type": "command", "command": hook, "timeout": secs}],
                    }),
                    marker: hook,
                },
            ),
        ],
        Harness::Codex => vec![
            (
                Role::Mcp,
                Target::TomlTable {
                    file: home.join(".codex").join("config.toml"),
                    header: format!("mcp_servers.{SERVER_NAME}"),
                    body: format!(
                        "command = {}\nargs = [\"mcp\", \"--via\", {}]\n",
                        toml::Value::String(exe),
                        toml::Value::String(h.label().to_owned())
                    ),
                },
            ),
            (
                Role::Hook,
                Target::JsonItem {
                    file: home.join(".codex").join("hooks.json"),
                    path: path(&["hooks", "PreToolUse"]),
                    value: json!({
                        "matcher": "^(Bash|apply_patch)$",
                        "hooks": [{"type": "command", "command": hook, "timeout": secs, "statusMessage": "Reins: checking the command"}],
                    }),
                    marker: hook,
                },
            ),
        ],
        Harness::Gemini => vec![
            (
                Role::Mcp,
                Target::JsonMember {
                    file: home.join(".gemini").join("settings.json"),
                    path: path(&["mcpServers"]),
                    key: SERVER_NAME.to_owned(),
                    value: json!({"command": exe, "args": args}),
                    keep_existing: false,
                },
            ),
            (
                Role::Hook,
                Target::JsonItem {
                    file: home.join(".gemini").join("settings.json"),
                    path: path(&["hooks", "BeforeTool"]),
                    value: json!({
                        "matcher": "^(run_shell_command|write_file|replace|read_file)$",
                        "hooks": [{"name": "reins", "type": "command", "command": hook, "timeout": secs * 1000}],
                    }),
                    marker: hook,
                },
            ),
        ],
        Harness::Cursor => {
            let hooks = home.join(".cursor").join("hooks.json");
            let item = |matcher: Option<&str>| {
                let mut v = json!({"command": hook, "timeout": secs});
                if let Some(m) = matcher {
                    v["matcher"] = json!(m);
                }
                v
            };
            vec![
                (
                    Role::Mcp,
                    Target::JsonMember {
                        file: home.join(".cursor").join("mcp.json"),
                        path: path(&["mcpServers"]),
                        key: SERVER_NAME.to_owned(),
                        value: json!({"type": "stdio", "command": exe, "args": args}),
                        keep_existing: false,
                    },
                ),
                (
                    Role::Support,
                    Target::JsonMember {
                        file: hooks.clone(),
                        path: Vec::new(),
                        key: "version".to_owned(),
                        value: json!(1),
                        keep_existing: true,
                    },
                ),
                (
                    Role::Hook,
                    Target::JsonItem {
                        file: hooks.clone(),
                        path: path(&["hooks", "beforeShellExecution"]),
                        value: item(None),
                        marker: hook.clone(),
                    },
                ),
                (
                    Role::Hook,
                    Target::JsonItem {
                        file: hooks.clone(),
                        path: path(&["hooks", "beforeReadFile"]),
                        value: item(None),
                        marker: hook.clone(),
                    },
                ),
                (
                    Role::Hook,
                    Target::JsonItem {
                        file: hooks,
                        path: path(&["hooks", "preToolUse"]),
                        value: item(Some("Write|Delete")),
                        marker: hook,
                    },
                ),
            ]
        }
    }
}

/// What `add` did to one file, enough to undo it exactly.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Applied {
    file: PathBuf,
    /// The file did not exist.
    #[serde(default)]
    created_file: bool,
    /// The file existed but held only whitespace (this).
    #[serde(default)]
    blank: Option<String>,
    /// Directories made for the file, deepest first.
    #[serde(default)]
    created_dirs: Vec<PathBuf>,
    change: Change,
    /// Not from the record: only entries that are certainly this app's are taken out.
    #[serde(skip)]
    guessed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Change {
    /// The entry (member `key`, or the item matching `value`/`marker`) of the container at `path`; the first `at`
    /// parts of `path` existed, the rest were created. `interior`: the old inside of the container inserted into, when
    /// it was empty.
    Json {
        path: Vec<String>,
        #[serde(default)]
        key: Option<String>,
        #[serde(default)]
        marker: Option<String>,
        value: Value,
        at: usize,
        #[serde(default)]
        interior: Option<String>,
    },
    /// The text appended to the file (a `[header]` table).
    Toml {
        header: String,
        text: String,
    },
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Manifest {
    #[serde(default)]
    harnesses: BTreeMap<String, Vec<Applied>>,
}

fn manifest_file(paths: &Paths) -> PathBuf {
    paths.state_dir.join("harnesses.json")
}

fn load_manifest(paths: &Paths) -> Result<Manifest, String> {
    let file = manifest_file(paths);
    match std::fs::read(&file) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", file.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Manifest::default()),
        Err(e) => Err(format!("{}: {e}", file.display())),
    }
}

fn save_manifest(paths: &Paths, m: &Manifest) -> Result<(), String> {
    paths.ensure().map_err(|e| e.to_string())?;
    let file = manifest_file(paths);
    let bytes = serde_json::to_vec_pretty(m).map_err(|e| e.to_string())?;
    crate::config::write_private(&file, &bytes).map_err(|e| format!("{}: {e}", file.display()))
}

fn read_text(file: &Path) -> Result<Option<String>, String> {
    match std::fs::read(file) {
        Ok(b) => String::from_utf8(b).map(Some).map_err(|_| format!("{} is not UTF-8 text", file.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: {e}", file.display())),
    }
}

/// Replaces `file`'s contents atomically, keeping its permissions, writing through a symbolic link (dotfile managers).
fn write_text(file: &Path, text: &str) -> Result<(), String> {
    let real = std::fs::canonicalize(file).unwrap_or_else(|_| file.to_path_buf());
    let shown = |e: std::io::Error| format!("{}: {e}", file.display());
    let dir = real.parent().unwrap_or_else(|| Path::new("."));
    let mut tmp = tempfile::NamedTempFile::new_in(dir).map_err(shown)?;
    let perms = std::fs::metadata(&real).map(|m| m.permissions());
    #[cfg(unix)]
    let perms = perms.or_else(|_| {
        use std::os::unix::fs::PermissionsExt as _;
        Ok::<_, std::io::Error>(std::fs::Permissions::from_mode(0o600))
    });
    if let Ok(p) = perms {
        tmp.as_file().set_permissions(p).map_err(shown)?;
    }
    std::io::Write::write_all(&mut tmp, text.as_bytes()).map_err(shown)?;
    tmp.as_file().sync_all().map_err(shown)?;
    tmp.persist(&real).map_err(|e| shown(e.error))?;
    Ok(())
}

/// Makes `dir` and its missing parents; returns the ones it made, deepest first.
fn make_dirs(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut missing = Vec::new();
    let mut d = Some(dir);
    while let Some(p) = d {
        if p.as_os_str().is_empty() || p.exists() {
            break;
        }
        missing.push(p.to_path_buf());
        d = p.parent();
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    Ok(missing)
}

/// Whether any string in `v` contains `marker`.
fn holds(v: &Value, marker: &str) -> bool {
    match v {
        Value::String(s) => s.contains(marker),
        Value::Array(a) => a.iter().any(|v| holds(v, marker)),
        Value::Object(m) => m.values().any(|v| holds(v, marker)),
        _ => false,
    }
}

fn kind(n: &Node) -> &'static str {
    match n {
        Node::Object {
            ..
        } => "an object",
        Node::Array {
            ..
        } => "a list",
        Node::Scalar {
            ..
        } => "a single value",
    }
}

/// The index of our entry in `container`, if it is there.
fn find_entry(text: &str, container: &Node, key: Option<&str>, value: &Value, marker: Option<&str>) -> Option<usize> {
    match (container, key) {
        (
            Node::Object {
                members,
                ..
            },
            Some(k),
        ) => members.iter().position(|(name, _, v)| name == k && marker.is_none_or(|m| holds(&v.value(text), m))),
        (
            Node::Array {
                items,
                ..
            },
            None,
        ) => items.iter().position(|n| {
            let v = n.value(text);
            v == *value || marker.is_some_and(|m| holds(&v, m))
        }),
        _ => None,
    }
}

/// Where a JSON entry goes: member `key` (or an item, matched by `value` or `marker`) of the container at `path`.
#[derive(Clone, Copy)]
struct Spot<'a> {
    path: &'a [String],
    key: Option<&'a str>,
    value: &'a Value,
    marker: Option<&'a str>,
}

/// Adds the JSON entry to `text`; `None` when it is there already.
fn add_json(text: &str, shown: &str, spot: &Spot<'_>, keep_existing: bool) -> Result<Option<(String, Change)>, String> {
    let Spot {
        path,
        key,
        value,
        marker,
    } = *spot;
    let root = jsonedit::parse(text).map_err(|e| format!("{shown}: {e}"))?;
    if !matches!(root, Node::Object { .. }) {
        return Err(format!("{shown}: expected a JSON object"));
    }
    let mut at = 0;
    let mut node = &root;
    while at < path.len() {
        match node.member(&path[at]) {
            Some(n) => {
                node = n;
                at += 1;
            }
            None => break,
        }
    }
    let dotted = || {
        if path.is_empty() {
            "the top level".to_owned()
        } else {
            format!("`{}`", path.join("."))
        }
    };
    let (new_text, interior) = if at == path.len() {
        match (node, key) {
            (
                Node::Object {
                    ..
                },
                Some(k),
            ) => {
                if let Some(existing) = node.member(k) {
                    if keep_existing || existing.value(text) == *value {
                        return Ok(None);
                    }
                    return Err(format!(
                        "{shown} already has a different `{k}` in {}; remove it or `reins harness remove` first",
                        dotted()
                    ));
                }
                jsonedit::insert(
                    text,
                    &root,
                    node,
                    &Entry {
                        key: Some(k),
                        value,
                    },
                )
            }
            (
                Node::Array {
                    ..
                },
                None,
            ) => {
                if find_entry(text, node, None, value, marker).is_some() {
                    return Ok(None);
                }
                jsonedit::insert(
                    text,
                    &root,
                    node,
                    &Entry {
                        key: None,
                        value,
                    },
                )
            }
            (n, _) => return Err(format!("{shown}: {} is {}, not what this harness expects", dotted(), kind(n))),
        }
    } else {
        if !matches!(node, Node::Object { .. }) {
            return Err(format!("{shown}: `{}` is {}, not an object", path[..at].join("."), kind(node)));
        }
        let mut wrapped = match key {
            Some(k) => json!({ k: value }),
            None => json!([value]),
        };
        for part in path[at + 1..].iter().rev() {
            wrapped = json!({ part.as_str(): wrapped });
        }
        jsonedit::insert(
            text,
            &root,
            node,
            &Entry {
                key: Some(&path[at]),
                value: &wrapped,
            },
        )
    };
    jsonedit::parse(&new_text).map_err(|e| format!("{shown}: the edit would break the file ({e}); nothing changed"))?;
    Ok(Some((
        new_text,
        Change::Json {
            path: path.to_vec(),
            key: key.map(str::to_owned),
            marker: marker.map(str::to_owned),
            value: value.clone(),
            at,
            interior,
        },
    )))
}

/// Takes the JSON entry out of `text`, then the containers `add` created for it once they are empty.
fn undo_json(text: &str, shown: &str, spot: &Spot<'_>, at: usize, interior: Option<&str>) -> Result<String, String> {
    let Spot {
        path,
        key,
        value,
        marker,
    } = *spot;
    let parse = |t: &str| jsonedit::parse(t).map_err(|e| format!("{shown}: {e}"));
    let mut text = text.to_owned();
    let root = parse(&text)?;
    let Some(container) = root.at(path) else {
        return Ok(text);
    };
    if let Some(i) = find_entry(&text, container, key, value, marker) {
        let restore = if path.len() == at {
            interior
        } else {
            None
        };
        text = jsonedit::remove(&text, container, i, restore);
    }
    for depth in (at + 1..=path.len()).rev() {
        let root = parse(&text)?;
        if root.at(&path[..depth]).is_some_and(|c| !c.is_empty()) {
            break;
        }
        let Some(parent) = root.at(&path[..depth - 1]) else {
            break;
        };
        let Some(i) = find_entry(&text, parent, Some(&path[depth - 1]), &Value::Null, None) else {
            break;
        };
        let restore = if depth - 1 == at {
            interior
        } else {
            None
        };
        text = jsonedit::remove(&text, parent, i, restore);
    }
    Ok(text)
}

fn toml_has_table(text: &str, header: &str) -> bool {
    let want = format!("[{header}]");
    text.lines().any(|l| l.trim() == want)
}

fn add_toml(text: &str, shown: &str, header: &str, body: &str) -> Result<Option<(String, Change)>, String> {
    if toml_has_table(text, header) {
        if text.contains(&format!("[{header}]\n{body}")) {
            return Ok(None);
        }
        return Err(format!("{shown} already has a different [{header}]; remove it or `reins harness remove` first"));
    }
    let mut appended = String::new();
    if !text.is_empty() {
        if !text.ends_with('\n') {
            appended.push('\n');
        }
        appended.push('\n');
    }
    let _infallible = write!(appended, "[{header}]\n{body}");
    let new_text = format!("{text}{appended}");
    toml::from_str::<toml::Table>(&new_text)
        .map_err(|e| format!("{shown}: cannot add [{header}] here ({}); nothing changed", e.message()))?;
    Ok(Some((
        new_text,
        Change::Toml {
            header: header.to_owned(),
            text: appended,
        },
    )))
}

fn undo_toml(text: &str, header: &str, appended: &str) -> String {
    if let Some(before) = text.strip_suffix(appended) {
        return before.to_owned();
    }
    // Something was added after it: take out the table's lines only.
    let want = format!("[{header}]");
    let sub = format!("[{header}.");
    let mut out = String::with_capacity(text.len());
    let mut inside = false;
    for line in text.split_inclusive('\n') {
        let t = line.trim();
        if t.starts_with('[') {
            inside = t == want || t.starts_with(&sub);
        }
        if !inside {
            out.push_str(line);
        }
    }
    out
}

fn blank_root(text: &str) -> bool {
    text.trim().is_empty() || jsonedit::parse(text).is_ok_and(|n| matches!(n, Node::Object { .. }) && n.is_empty())
}

/// One line of `add`'s or `remove`'s report.
fn tilde(s: &Setup, p: &Path) -> String {
    match p.strip_prefix(&s.home) {
        Ok(rest) if cfg!(windows) => format!("~/{}", rest.display().to_string().replace('\\', "/")),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => p.display().to_string(),
    }
}

/// Registers the MCP server and the hook in `h`'s settings; returns what it did, one line each.
pub fn add(paths: &Paths, s: &Setup, h: Harness) -> Result<Vec<String>, String> {
    let mut manifest = load_manifest(paths)?;
    let mut report: Vec<(Role, bool, String)> = Vec::new();
    for (role, target) in targets(h, s) {
        let file = target.file().to_path_buf();
        let shown = tilde(s, &file);
        let existing = read_text(&file)?;
        let blank = existing.as_ref().filter(|t| t.trim().is_empty()).cloned();
        let change = match &target {
            Target::JsonMember {
                path,
                key,
                value,
                keep_existing,
                ..
            } => {
                let start = existing.clone().filter(|t| !t.trim().is_empty()).unwrap_or_else(|| "{}\n".to_owned());
                add_json(
                    &start,
                    &shown,
                    &Spot {
                        path,
                        key: Some(key),
                        value,
                        marker: None,
                    },
                    *keep_existing,
                )?
            }
            Target::JsonItem {
                path,
                value,
                marker,
                ..
            } => {
                let start = existing.clone().filter(|t| !t.trim().is_empty()).unwrap_or_else(|| "{}\n".to_owned());
                add_json(
                    &start,
                    &shown,
                    &Spot {
                        path,
                        key: None,
                        value,
                        marker: Some(marker),
                    },
                    false,
                )?
            }
            Target::TomlTable {
                header,
                body,
                ..
            } => add_toml(existing.as_deref().unwrap_or_default(), &shown, header, body)?,
        };
        let Some((text, change)) = change else {
            report.push((role, false, shown));
            continue;
        };
        let created_dirs = match file.parent() {
            Some(dir) if existing.is_none() => make_dirs(dir)?,
            _ => Vec::new(),
        };
        write_text(&file, &text)?;
        manifest.harnesses.entry(h.id().to_owned()).or_default().push(Applied {
            file: file.clone(),
            created_file: existing.is_none(),
            blank,
            created_dirs,
            change,
            guessed: false,
        });
        // Saved after every file, so a failure half way can still be undone.
        save_manifest(paths, &manifest)?;
        report.push((role, true, shown));
    }
    Ok(add_report(&report))
}

/// What `add` did, one line per file: a harness with several hook entries in one file (Cursor) reads "added 3 hooks
/// to ~/.cursor/hooks.json", not the same line three times.
fn add_report(parts: &[(Role, bool, String)]) -> Vec<String> {
    let mut lines: Vec<((Role, bool, &str), usize)> = Vec::new();
    for (role, added, shown) in parts {
        if *role == Role::Support {
            continue;
        }
        let key = (*role, *added, shown.as_str());
        match lines.iter_mut().find(|(k, _)| *k == key) {
            Some((_, n)) => *n += 1,
            None => lines.push((key, 1)),
        }
    }
    lines
        .into_iter()
        .map(|((role, added, shown), n)| match (role, added, n) {
            (Role::Mcp, true, _) => format!("added the MCP server `{SERVER_NAME}` to {shown}"),
            (Role::Mcp, false, _) => format!("MCP server already in {shown}"),
            (_, true, 1) => format!("added the hook to {shown}"),
            (_, true, n) => format!("added {n} hooks to {shown}"),
            (_, false, 1) => format!("hook already in {shown}"),
            (_, false, n) => format!("{n} hooks already in {shown}"),
        })
        .collect()
}

/// Undoes what `add` did for `h` (or, with no record of it, takes out this app's entries by name).
pub fn remove(paths: &Paths, s: &Setup, h: Harness) -> Result<Vec<String>, String> {
    let mut manifest = load_manifest(paths)?;
    let recorded = manifest.harnesses.remove(h.id());
    let applied = recorded.unwrap_or_else(|| unrecorded(h, s));
    let mut report = Vec::new();
    let mut failed = Vec::new();
    for a in applied.iter().rev() {
        match undo(a) {
            Ok(true) => {
                let line = format!("removed this app's entry from {}", tilde(s, &a.file));
                if !report.contains(&line) {
                    report.push(line);
                }
            }
            Ok(false) => {}
            Err(e) => failed.push((a.clone(), e)),
        }
    }
    if !failed.is_empty() {
        // Keep what could not be undone, to try again.
        let lines: Vec<String> = failed.iter().map(|(_, e)| e.clone()).collect();
        manifest.harnesses.insert(h.id().to_owned(), failed.into_iter().rev().map(|(a, _)| a).collect());
        save_manifest(paths, &manifest)?;
        return Err(lines.join("; "));
    }
    if manifest_file(paths).exists() {
        save_manifest(paths, &manifest)?;
    }
    Ok(report)
}

/// This app's entries as if `add` had put them into existing containers (no record of the real `add`).
fn unrecorded(h: Harness, s: &Setup) -> Vec<Applied> {
    targets(h, s)
        .into_iter()
        .filter(|(role, _)| *role != Role::Support)
        .map(|(_, t)| {
            let file = t.file().to_path_buf();
            let change = match t {
                Target::JsonMember {
                    path,
                    key,
                    value,
                    ..
                } => Change::Json {
                    at: path.len(),
                    path,
                    key: Some(key),
                    marker: Some(s.exe.to_string_lossy().into_owned()),
                    value,
                    interior: None,
                },
                Target::JsonItem {
                    path,
                    value,
                    marker,
                    ..
                } => Change::Json {
                    at: path.len(),
                    path,
                    key: None,
                    marker: Some(marker),
                    value,
                    interior: None,
                },
                Target::TomlTable {
                    header,
                    body,
                    ..
                } => Change::Toml {
                    text: format!("[{header}]\n{body}"),
                    header,
                },
            };
            Applied {
                file,
                created_file: false,
                blank: None,
                created_dirs: Vec::new(),
                change,
                guessed: true,
            }
        })
        .collect()
}

/// Undoes one change; `Ok(false)` when there was nothing to undo.
fn undo(a: &Applied) -> Result<bool, String> {
    let shown = a.file.display().to_string();
    let Some(text) = read_text(&a.file)? else {
        remove_dirs(&a.created_dirs);
        return Ok(false);
    };
    let new_text = match &a.change {
        Change::Json {
            path,
            key,
            marker,
            value,
            at,
            interior,
        } => undo_json(
            &text,
            &shown,
            &Spot {
                path,
                key: key.as_deref(),
                value,
                marker: marker.as_deref(),
            },
            *at,
            interior.as_deref(),
        )?,
        Change::Toml {
            header,
            text: appended,
        } => {
            if let Some(at) = text.rfind(appended.as_str()) {
                format!("{}{}", &text[..at], &text[at + appended.len()..])
            } else if toml_has_table(&text, header) && !a.guessed {
                undo_toml(&text, header, appended)
            } else {
                text.clone()
            }
        }
    };
    let changed = new_text != text;
    let json = matches!(a.change, Change::Json { .. });
    let empty = if json {
        blank_root(&new_text)
    } else {
        new_text.trim().is_empty()
    };
    if a.created_file && empty {
        std::fs::remove_file(&a.file).map_err(|e| format!("{shown}: {e}"))?;
        remove_dirs(&a.created_dirs);
        return Ok(true);
    }
    if let Some(blank) = a.blank.as_ref().filter(|_| empty) {
        write_text(&a.file, blank)?;
        return Ok(true);
    }
    if changed {
        write_text(&a.file, &new_text)?;
    }
    Ok(changed)
}

fn remove_dirs(dirs: &[PathBuf]) {
    for d in dirs {
        // Only empty ones: the harness may have put its own files there since.
        let _kept = std::fs::remove_dir(d);
    }
}

/// What is in `h`'s settings of this app's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Registered {
    /// The MCP server entry.
    pub mcp: bool,
    /// How many of the hook entries are there, of how many.
    pub hooks: (usize, usize),
    /// `add` recorded what it did (so `remove` undoes exactly that).
    pub recorded: bool,
}

impl Registered {
    /// The MCP server and every hook entry are there.
    #[must_use]
    pub fn complete(&self) -> bool {
        self.mcp && self.hooks.0 == self.hooks.1
    }

    /// Anything of this app's is there.
    #[must_use]
    pub fn any(&self) -> bool {
        self.mcp || self.hooks.0 > 0
    }
}

/// What is registered for `h`.
pub fn registered(paths: &Paths, s: &Setup, h: Harness) -> Result<Registered, String> {
    let (mcp, hooks, recorded) = parts(paths, s, h)?;
    Ok(Registered {
        mcp: mcp.is_some_and(|(present, _)| present),
        hooks: (hooks.iter().filter(|(present, _)| *present).count(), hooks.len()),
        recorded,
    })
}

type Parts = (Option<(bool, String)>, Vec<(bool, String)>, bool);

/// The MCP server part and the hook parts of `h` (present, file shown), and whether `add` recorded them.
fn parts(paths: &Paths, s: &Setup, h: Harness) -> Result<Parts, String> {
    let recorded = load_manifest(paths)?.harnesses.contains_key(h.id());
    let mut mcp = None;
    let mut hooks: Vec<(bool, String)> = Vec::new();
    for (role, t) in targets(h, s) {
        let shown = tilde(s, t.file());
        let text = read_text(t.file())?;
        let present = text.as_deref().is_some_and(|text| match &t {
            Target::JsonMember {
                path,
                key,
                ..
            } => jsonedit::parse(text).is_ok_and(|root| root.at(path).and_then(|c| c.member(key)).is_some()),
            Target::JsonItem {
                path,
                value,
                marker,
                ..
            } => jsonedit::parse(text).is_ok_and(|root| {
                root.at(path).is_some_and(|c| find_entry(text, c, None, value, Some(marker)).is_some())
            }),
            Target::TomlTable {
                header,
                ..
            } => toml_has_table(text, header),
        });
        match role {
            Role::Mcp => mcp = Some((present, shown)),
            Role::Hook => hooks.push((present, shown)),
            Role::Support => {}
        }
    }
    Ok((mcp, hooks, recorded))
}

/// What is registered for `h`: one line per part.
pub fn list(paths: &Paths, s: &Setup, h: Harness) -> Result<Vec<String>, String> {
    let (mcp, hooks, recorded) = parts(paths, s, h)?;
    let yes_no = |(present, shown): &(bool, String)| {
        if *present {
            format!("yes ({shown})")
        } else {
            "no".to_owned()
        }
    };
    let mut lines = vec![format!("{:<12} {}", h.id(), h.label())];
    if let Some(m) = &mcp {
        lines.push(format!("  MCP server: {}", yes_no(m)));
    }
    let hook = if hooks.iter().all(|(p, _)| *p) {
        hooks.first().map_or_else(|| "no".to_owned(), yes_no)
    } else if hooks.iter().any(|(p, _)| *p) {
        format!("partly ({})", hooks.first().map(|h| h.1.clone()).unwrap_or_default())
    } else {
        "no".to_owned()
    };
    lines.push(format!("  hook:       {hook}"));
    if recorded {
        lines.push("  (added by `reins harness add`)".to_owned());
    }
    Ok(lines)
}

/// What to tell the user after `add`.
#[must_use]
pub fn after_add_note(h: Harness) -> &'static str {
    match h {
        Harness::ClaudeCode => "Restart Claude Code to pick it up (`/mcp` and `/hooks` show it).",
        Harness::Codex => "Restart Codex, then trust the new hook once with `/hooks` (Codex runs only reviewed hooks).",
        Harness::Gemini => "Restart Gemini CLI to pick it up (`/mcp` and `/hooks` show it).",
        Harness::Cursor => "Restart Cursor to pick it up (Settings → MCP and Hooks show it).",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn several_hooks_in_one_file_are_one_line() {
        let f = |role, added, shown: &str| (role, added, shown.to_owned());
        let parts = [
            f(Role::Mcp, true, "~/.cursor/mcp.json"),
            f(Role::Support, true, "~/.cursor/hooks.json"),
            f(Role::Hook, true, "~/.cursor/hooks.json"),
            f(Role::Hook, true, "~/.cursor/hooks.json"),
            f(Role::Hook, true, "~/.cursor/hooks.json"),
        ];
        assert_eq!(
            add_report(&parts),
            ["added the MCP server `reins` to ~/.cursor/mcp.json", "added 3 hooks to ~/.cursor/hooks.json"]
        );
        let again = [f(Role::Hook, false, "~/.claude/settings.json")];
        assert_eq!(add_report(&again), ["hook already in ~/.claude/settings.json"]);
    }

    #[test]
    fn names_and_quoting() {
        for h in Harness::ALL {
            assert_eq!(h.id().parse::<Harness>().unwrap(), h);
        }
        assert_eq!("Claude".parse::<Harness>().unwrap(), Harness::ClaudeCode);
        assert!("vim".parse::<Harness>().is_err());
        assert_eq!(shell_quote("/home/me/.local/bin/reins"), "/home/me/.local/bin/reins");
        assert_eq!(shell_quote("/Users/me/My Apps/reins"), "'/Users/me/My Apps/reins'");
        assert_eq!(shell_quote("/x/it's"), "'/x/it'\\''s'");
    }

    #[test]
    fn toml_tables_are_appended_and_taken_out_again() {
        let body = "command = \"/bin/reins\"\nargs = [\"mcp\"]\n";
        for before in ["", "model = \"o3\"\n", "model = \"o3\"", "[mcp_servers.other]\ncommand = \"x\"\n"] {
            let (after, change) = add_toml(before, "f", "mcp_servers.reins", body).unwrap().unwrap();
            let Change::Toml {
                text,
                ..
            } = change
            else {
                panic!()
            };
            assert!(toml_has_table(&after, "mcp_servers.reins"));
            assert!(add_toml(&after, "f", "mcp_servers.reins", body).unwrap().is_none());
            assert_eq!(undo_toml(&after, "mcp_servers.reins", &text), before);
            // Something added after it by the user stays.
            let edited = format!("{after}[profiles.x]\nmodel = \"y\"\n");
            let back = undo_toml(&edited, "mcp_servers.reins", &text);
            assert!(!back.contains("reins") && back.contains("[profiles.x]"), "{back}");
        }
        assert!(add_toml("[mcp_servers.reins]\ncommand = \"other\"\n", "f", "mcp_servers.reins", body).is_err());
        assert!(add_toml("mcp_servers = { a = 1 }\n", "f", "mcp_servers.reins", body).is_err());
    }
}
