//! The two files Reins Secrets keeps in a repository, both safe to commit (they hold references, never a value):
//!
//! - `reins.toml` at the project root: the project's name (the vault folder `Reins/<name>`), the default environment
//!   and per-environment settings (how long a release may be held, whether it is always asked).
//! - `.env.reins` next to it (optional): variable names, each a reference (`vault:acme-web/development/KEY`,
//!   `env:development/KEY`) or a literal value (`LOG_LEVEL=debug`), for tools that read dotenv files.
//!
//! Both parsers are strict: anything they do not understand is an error naming the line, never skipped. The writers
//! produce what the parsers read back. [`parse_dotenv`] reads an ordinary (plaintext) `.env` file the same way, for
//! `reins secrets push` and import.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use zeroize::Zeroizing;

use crate::secrets::{EnvRef, Reference, check_name, check_var};

pub const PROJECT_FILE: &str = "reins.toml";
pub const ENV_FILE: &str = ".env.reins";
/// The environment a project uses when `reins.toml` names none.
pub const DEFAULT_ENV: &str = "development";
/// The shortest and longest a release may be held by the daemon (the phone may cap it further).
pub const MIN_LEASE_SECS: u32 = 60;
pub const MAX_LEASE_SECS: u32 = 86_400;
/// Larger files are not read: neither file is ever that big.
const MAX_FILE_BYTES: u64 = 1024 * 1024;
/// How many errors one parse reports at most.
const MAX_ERRORS: usize = 20;
/// How many folders `Project::find` climbs at most.
const MAX_DEPTH: usize = 64;

/// Whether an environment is always asked for on the phone (`confirm = "always"`), whatever grant there is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Confirm {
    /// The phone decides (a grant may cover it; `production` and `prod` are asked every time by default).
    #[default]
    Default,
    Always,
}

/// `[env.<name>]` in `reins.toml`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EnvSettings {
    /// `lease = "8h"`: how long the daemon may hold a release; `None`: the default.
    pub lease_secs: Option<u32>,
    pub confirm: Confirm,
}

/// `reins.toml`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectFile {
    /// The vault folder `Reins/<name>`.
    pub name: String,
    pub default_env: String,
    pub envs: BTreeMap<String, EnvSettings>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFile {
    project: RawProject,
    #[serde(default)]
    env: BTreeMap<String, RawEnv>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawProject {
    name: String,
    default_env: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEnv {
    lease: Option<String>,
    confirm: Option<String>,
}

/// `"8h"`, `"15m"`, `"90s"`, `"1h30m"`, `"1d"` in seconds.
pub fn parse_duration(text: &str) -> Result<u32, String> {
    let bad = || format!("`{text}` is not a duration: write 15m, 8h, 1h30m or 1d");
    let mut total: u64 = 0;
    let mut digits = String::new();
    let mut any = false;
    for c in text.trim().chars() {
        if c.is_ascii_digit() {
            digits.push(c);
            continue;
        }
        let unit = match c {
            's' => 1,
            'm' => 60,
            'h' => 3_600,
            'd' => 86_400,
            _ => return Err(bad()),
        };
        let n: u64 = digits.parse().map_err(|_| bad())?;
        digits.clear();
        total = total.saturating_add(n.saturating_mul(unit));
        any = true;
    }
    if !digits.is_empty() || !any {
        return Err(bad());
    }
    u32::try_from(total).map_err(|_| bad())
}

/// `8h`, `15m`, `1h30m`: the shortest way to write `secs` that [`parse_duration`] reads back.
#[must_use]
pub fn format_duration(secs: u32) -> String {
    if secs == 0 {
        return "0s".to_owned();
    }
    let mut out = String::new();
    let mut rest = secs;
    for (unit, size) in [('d', 86_400), ('h', 3_600), ('m', 60), ('s', 1)] {
        if rest >= size {
            let _ = write!(out, "{}{unit}", rest / size);
            rest %= size;
        }
    }
    out
}

impl ProjectFile {
    pub fn new(name: &str, default_env: &str) -> Result<Self, String> {
        check_name("project", name)?;
        check_name("environment", default_env)?;
        Ok(Self {
            name: name.to_owned(),
            default_env: default_env.to_owned(),
            envs: BTreeMap::new(),
        })
    }

    /// Parses `reins.toml`. Unknown keys, bad names and bad durations are errors.
    pub fn parse(text: &str) -> Result<Self, String> {
        let raw: RawFile = toml::from_str(text).map_err(|e| e.to_string().trim().to_owned())?;
        let default_env = raw.project.default_env.unwrap_or_else(|| DEFAULT_ENV.to_owned());
        let mut file = Self::new(raw.project.name.trim(), default_env.trim()).map_err(|e| format!("[project]: {e}"))?;
        for (name, env) in raw.env {
            check_name("environment", &name).map_err(|e| format!("[env.{name}]: {e}"))?;
            let lease_secs = env
                .lease
                .map(|l| {
                    let secs = parse_duration(&l)?;
                    if (MIN_LEASE_SECS..=MAX_LEASE_SECS).contains(&secs) {
                        Ok(secs)
                    } else {
                        Err(format!("`lease = \"{l}\"` must be between 1m and 24h"))
                    }
                })
                .transpose()
                .map_err(|e| format!("[env.{name}]: {e}"))?;
            let confirm = match env.confirm.as_deref() {
                None => Confirm::Default,
                Some("always") => Confirm::Always,
                Some(other) => {
                    return Err(format!("[env.{name}]: `confirm = \"{other}\"`: the only value is \"always\""));
                }
            };
            file.envs.insert(
                name,
                EnvSettings {
                    lease_secs,
                    confirm,
                },
            );
        }
        Ok(file)
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        Self::parse(&read_small(path)?).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// The settings of `env` (the defaults when it has no `[env.*]`).
    #[must_use]
    pub fn env(&self, env: &str) -> EnvSettings {
        self.envs.get(env).cloned().unwrap_or_default()
    }

    /// The environment `env` of this project (the default one when `None`).
    pub fn env_ref(&self, env: Option<&str>) -> Result<EnvRef, String> {
        EnvRef::new(&self.name, env.unwrap_or(&self.default_env))
    }

    /// The file's text: what [`ProjectFile::parse`] reads back.
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str("# Reins Secrets: values live in the vault on your phone (folder Reins/");
        out.push_str(&self.name);
        out.push_str("). Safe to commit.\n");
        out.push_str("[project]\n");
        let _ = writeln!(out, "name = {}", toml_string(&self.name));
        let _ = writeln!(out, "default_env = {}", toml_string(&self.default_env));
        for (name, env) in &self.envs {
            if env == &EnvSettings::default() {
                continue;
            }
            let _ = write!(out, "\n[env.{}]\n", toml_key(name));
            if let Some(secs) = env.lease_secs {
                let _ = writeln!(out, "lease = \"{}\"", format_duration(secs));
            }
            if env.confirm == Confirm::Always {
                out.push_str("confirm = \"always\"\n");
            }
        }
        out
    }

    pub fn write(&self, path: &Path) -> Result<(), String> {
        write_file(path, &self.render())
    }
}

/// A TOML basic string.
fn toml_string(s: &str) -> String {
    toml::Value::String(s.to_owned()).to_string()
}

/// A TOML key: bare when it can be (names allow `.` only quoted).
fn toml_key(s: &str) -> String {
    if s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-')) {
        s.to_owned()
    } else {
        toml_string(s)
    }
}

fn read_small(path: &Path) -> Result<String, String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if meta.len() > MAX_FILE_BYTES {
        return Err(format!("{}: larger than 1 MB", path.display()));
    }
    std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))
}

/// Writes `text` to `path` through a temporary file in the same folder, so a reader never sees half of it.
fn write_file(path: &Path, text: &str) -> Result<(), String> {
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
    let mut tmp = tempfile::NamedTempFile::new_in(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    std::io::Write::write_all(&mut tmp, text.as_bytes()).map_err(|e| format!("{}: {e}", path.display()))?;
    tmp.persist(path).map_err(|e| format!("{}: {}", path.display(), e.error))?;
    Ok(())
}

/// A project: the folder its `reins.toml` is in, and the file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Project {
    pub root: PathBuf,
    pub file: ProjectFile,
}

impl Project {
    /// The nearest `reins.toml` at `start` or above it; `Ok(None)` when there is none. A `reins.toml` that does not
    /// parse is an error (a typo must not silently fall back to another project).
    pub fn find(start: &Path) -> Result<Option<Self>, String> {
        for dir in start.ancestors().take(MAX_DEPTH) {
            let path = dir.join(PROJECT_FILE);
            if path.is_file() {
                return Ok(Some(Self {
                    root: dir.to_path_buf(),
                    file: ProjectFile::load(&path)?,
                }));
            }
        }
        Ok(None)
    }

    #[must_use]
    pub fn env_file_path(&self) -> PathBuf {
        self.root.join(ENV_FILE)
    }

    /// `.env.reins` next to `reins.toml`, when there is one.
    pub fn env_file(&self) -> Result<Option<EnvFile>, String> {
        let path = self.env_file_path();
        if !path.is_file() {
            return Ok(None);
        }
        EnvFile::load(&path).map(Some)
    }
}

// ---- dotenv -------------------------------------------------------------------------------------------------------

/// What a dotenv parse found wrong, line by line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    /// `(line, message)`, lines counted from 1.
    pub errors: Vec<(usize, String)>,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let lines: Vec<String> = self.errors.iter().map(|(n, m)| format!("line {n}: {m}")).collect();
        f.write_str(&lines.join("\n"))
    }
}

impl std::error::Error for ParseError {}

/// One `NAME=value` as written, before its value is interpreted.
struct RawVar {
    name: String,
    value: Zeroizing<String>,
    /// The line it starts on.
    line: usize,
}

enum RawLine {
    Blank,
    Comment(String),
    Var(RawVar),
}

/// The value after `=`: quoted (`"…"` with escapes, `'…'` as is) or bare (up to ` #`). Double quotes may span lines
/// only when `multiline` (a plaintext `.env`; `.env.reins` holds one line per variable).
fn lex(text: &str, multiline: bool) -> (Vec<RawLine>, Vec<(usize, String)>) {
    let mut out = Vec::new();
    let mut errors = Vec::new();
    let mut lines: Vec<&str> = text.split('\n').collect();
    // The newline that ends the last line starts no line of its own.
    if text.ends_with('\n') {
        lines.pop();
    }
    let mut i = 0;
    while i < lines.len() {
        let number = i + 1;
        let line = lines[i].strip_suffix('\r').unwrap_or(lines[i]);
        i += 1;
        let trimmed = line.trim_start();
        if trimmed.trim().is_empty() {
            out.push(RawLine::Blank);
            continue;
        }
        if trimmed.starts_with('#') {
            out.push(RawLine::Comment(line.to_owned()));
            continue;
        }
        let body = trimmed.strip_prefix("export ").map_or(trimmed, str::trim_start);
        let Some((name, rest)) = body.split_once('=') else {
            errors.push((number, "expected NAME=value".to_owned()));
            continue;
        };
        let name = name.trim_end();
        if let Err(e) = check_var(name) {
            errors.push((number, e));
            continue;
        }
        let rest = rest.trim_start();
        let mut value = Zeroizing::new(String::new());
        let after: String;
        if let Some(quoted) = rest.strip_prefix('"') {
            // Up to the closing quote, across lines when allowed.
            let mut chunk = quoted.to_owned();
            let mut closed = None;
            loop {
                if let Some(end) = closing_quote(&chunk) {
                    closed = Some(end);
                    break;
                }
                if !multiline || i >= lines.len() {
                    break;
                }
                chunk.push('\n');
                chunk.push_str(lines[i].strip_suffix('\r').unwrap_or(lines[i]));
                i += 1;
            }
            let Some(end) = closed else {
                errors.push((
                    number,
                    if multiline {
                        "a double quote is never closed".to_owned()
                    } else {
                        "a double quote is not closed on this line (values are one line)".to_owned()
                    },
                ));
                continue;
            };
            match unescape(&chunk[..end]) {
                Ok(v) => value.push_str(&v),
                Err(e) => {
                    errors.push((number, e));
                    continue;
                }
            }
            after = chunk[end + 1..].to_owned();
        } else if let Some(quoted) = rest.strip_prefix('\'') {
            let Some(end) = quoted.find('\'') else {
                errors.push((number, "a single quote is not closed on this line".to_owned()));
                continue;
            };
            value.push_str(&quoted[..end]);
            after = quoted[end + 1..].to_owned();
        } else {
            // A bare value ends at a `#` that follows whitespace (a comment).
            let mut end = rest.len();
            let bytes = rest.as_bytes();
            for (j, b) in bytes.iter().enumerate() {
                if *b == b'#' && (j == 0 || bytes[j - 1].is_ascii_whitespace()) {
                    end = j;
                    break;
                }
            }
            value.push_str(rest[..end].trim_end());
            after = String::new();
        }
        let tail = after.trim();
        if !tail.is_empty() && !tail.starts_with('#') {
            errors.push((number, "unexpected text after the closing quote".to_owned()));
            continue;
        }
        out.push(RawLine::Var(RawVar {
            name: name.to_owned(),
            value,
            line: number,
        }));
    }
    (out, errors)
}

/// The byte index of the first `"` not escaped by a backslash.
fn closing_quote(s: &str) -> Option<usize> {
    let mut escaped = false;
    for (i, c) in s.char_indices() {
        match c {
            _ if escaped => escaped = false,
            '\\' => escaped = true,
            '"' => return Some(i),
            _ => {}
        }
    }
    None
}

fn unescape(s: &str) -> Result<String, String> {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some(c @ ('"' | '\\' | '$' | '\'')) => out.push(c),
            Some(other) => return Err(format!("unknown escape `\\{other}` in a double-quoted value")),
            None => return Err("a double-quoted value ends with a lone backslash".to_owned()),
        }
    }
    Ok(out)
}

fn duplicates(vars: &[&RawVar], errors: &mut Vec<(usize, String)>) {
    let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
    for v in vars {
        if let Some(first) = seen.insert(&v.name, v.line) {
            errors.push((v.line, format!("{} is set twice (first on line {first})", v.name)));
        }
    }
}

fn finish<T>(value: T, mut errors: Vec<(usize, String)>) -> Result<T, ParseError> {
    if errors.is_empty() {
        return Ok(value);
    }
    errors.sort_by_key(|(n, _)| *n);
    errors.truncate(MAX_ERRORS);
    Err(ParseError {
        errors,
    })
}

/// The variables of a plaintext dotenv file (`.env`, `.env.production`), in order. `export NAME=`, quotes, escapes
/// in double quotes, multi-line double-quoted values and `#` comments are understood; `${VAR}` is kept as written.
pub fn parse_dotenv(text: &str) -> Result<Vec<(String, Zeroizing<String>)>, ParseError> {
    let (lines, mut errors) = lex(text, true);
    let vars: Vec<&RawVar> = lines
        .iter()
        .filter_map(|l| match l {
            RawLine::Var(v) => Some(v),
            _ => None,
        })
        .collect();
    duplicates(&vars, &mut errors);
    let out = vars.iter().map(|v| (v.name.clone(), v.value.clone())).collect();
    finish(out, errors)
}

/// The value of a variable in `.env.reins`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EnvValue {
    /// Filled in from the vault by `reins run`.
    Reference(Reference),
    /// Passed through as it is (`LOG_LEVEL=debug`): not a secret.
    Literal(String),
}

/// One variable of `.env.reins`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnvVar {
    pub name: String,
    pub value: EnvValue,
    /// Where it is (from 1), for messages.
    pub line: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Line {
    Blank,
    Comment(String),
    Var(EnvVar),
}

/// `.env.reins`: its lines in order, comments and blank lines kept so that a rewrite changes only what it must.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EnvFile {
    lines: Vec<Line>,
}

impl EnvFile {
    /// Parses `.env.reins`: a value starting with `vault:` or `env:` is a reference ([`Reference::parse_value`]),
    /// anything else a literal.
    pub fn parse(text: &str) -> Result<Self, ParseError> {
        let (raw, mut errors) = lex(text, false);
        let vars: Vec<&RawVar> = raw
            .iter()
            .filter_map(|l| match l {
                RawLine::Var(v) => Some(v),
                _ => None,
            })
            .collect();
        duplicates(&vars, &mut errors);
        let mut lines = Vec::with_capacity(raw.len());
        for l in raw {
            lines.push(match l {
                RawLine::Blank => Line::Blank,
                RawLine::Comment(c) => Line::Comment(c),
                RawLine::Var(v) => {
                    let value = if Reference::looks_like(&v.value) {
                        match Reference::parse_value(&v.value) {
                            Ok(r) => EnvValue::Reference(r),
                            Err(e) => {
                                errors.push((v.line, format!("{}: {e}", v.name)));
                                continue;
                            }
                        }
                    } else {
                        EnvValue::Literal(v.value.to_string())
                    };
                    Line::Var(EnvVar {
                        name: v.name,
                        value,
                        line: v.line,
                    })
                }
            });
        }
        finish(
            Self {
                lines,
            },
            errors,
        )
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        Self::parse(&read_small(path)?).map_err(|e| {
            let shown: Vec<String> = e.errors.iter().map(|(n, m)| format!("{}:{n}: {m}", path.display())).collect();
            shown.join("\n")
        })
    }

    /// A new file with the header comment and no variables.
    #[must_use]
    pub fn new() -> Self {
        Self {
            lines: vec![
                Line::Comment(
                    "# Reins Secrets: references only, safe to commit. `reins run -- <command>` fills them in."
                        .to_owned(),
                ),
                Line::Blank,
            ],
        }
    }

    pub fn vars(&self) -> impl Iterator<Item = &EnvVar> {
        self.lines.iter().filter_map(|l| match l {
            Line::Var(v) => Some(v),
            _ => None,
        })
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&EnvVar> {
        self.vars().find(|v| v.name == name)
    }

    /// Sets `name` where it is, or adds it at the end. A literal that would read back as a reference is refused.
    pub fn set(&mut self, name: &str, value: EnvValue) -> Result<(), String> {
        check_var(name)?;
        if let EnvValue::Literal(s) = &value
            && Reference::looks_like(s)
        {
            return Err(format!("{name}: a literal value cannot start with `vault:` or `env:`"));
        }
        for l in &mut self.lines {
            if let Line::Var(v) = l
                && v.name == name
            {
                v.value = value;
                return Ok(());
            }
        }
        let line = self.lines.len() + 1;
        self.lines.push(Line::Var(EnvVar {
            name: name.to_owned(),
            value,
            line,
        }));
        Ok(())
    }

    /// Removes `name`; whether it was there.
    pub fn remove(&mut self, name: &str) -> bool {
        let before = self.lines.len();
        self.lines.retain(|l| !matches!(l, Line::Var(v) if v.name == name));
        self.lines.len() != before
    }

    /// The file's text: what [`EnvFile::parse`] reads back.
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::new();
        for l in &self.lines {
            match l {
                Line::Blank => {}
                Line::Comment(c) => out.push_str(c),
                Line::Var(v) => {
                    out.push_str(&v.name);
                    out.push('=');
                    out.push_str(&match &v.value {
                        EnvValue::Reference(r) => quote_if_needed(&r.to_string()),
                        EnvValue::Literal(s) => quote_if_needed(s),
                    });
                }
            }
            out.push('\n');
        }
        // One trailing newline, however the lines ended.
        while out.ends_with("\n\n") {
            out.pop();
        }
        out
    }

    pub fn write(&self, path: &Path) -> Result<(), String> {
        write_file(path, &self.render())
    }
}

/// A value as written after `=`: bare when it reads back the same, else double-quoted with escapes.
fn quote_if_needed(s: &str) -> String {
    let bare = !s.is_empty()
        && s.trim() == s
        && !s.starts_with(['"', '\''])
        && !s.contains(char::is_whitespace)
        && !s.chars().any(char::is_control);
    if bare {
        return s.to_owned();
    }
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::SecretRef;

    #[test]
    fn reins_toml_reads_the_documented_format() {
        let f = ProjectFile::parse(
            "[project]\nname = \"acme-web\"   # the vault folder\ndefault_env = \"development\"\n\n\
             [env.development]\nlease = \"8h\"\n\n[env.production]\nlease = \"15m\"\nconfirm = \"always\"\n",
        )
        .unwrap();
        assert_eq!((f.name.as_str(), f.default_env.as_str()), ("acme-web", "development"));
        assert_eq!(f.env("development").lease_secs, Some(8 * 3_600));
        assert_eq!(
            f.env("production"),
            EnvSettings {
                lease_secs: Some(900),
                confirm: Confirm::Always
            }
        );
        assert_eq!(f.env("staging"), EnvSettings::default());
        assert_eq!(f.env_ref(None).unwrap().item(), "acme-web/development");
        assert_eq!(f.env_ref(Some("production")).unwrap().folder(), "Reins/acme-web");
        assert_eq!(ProjectFile::parse(&f.render()).unwrap(), f, "the writer's output reads back");
        let minimal = ProjectFile::parse("[project]\nname = \"x\"\n").unwrap();
        assert_eq!(minimal.default_env, DEFAULT_ENV);
    }

    #[test]
    fn reins_toml_mistakes_are_errors() {
        for (bad, says) in [
            ("[project]\n", "name"),
            ("[project]\nname = \"a/b\"\n", "project name"),
            ("[project]\nname = \"x\"\nnmae = 1\n", "nmae"),
            ("[project]\nname = \"x\"\n[env.dev]\nlease = \"8 hours\"\n", "duration"),
            ("[project]\nname = \"x\"\n[env.dev]\nlease = \"30s\"\n", "between"),
            ("[project]\nname = \"x\"\n[env.dev]\nlease = \"2d\"\n", "between"),
            ("[project]\nname = \"x\"\n[env.dev]\nconfirm = \"never\"\n", "always"),
            ("[project]\nname = \"x\"\n[env.\"a b\"]\n", "environment name"),
            ("[project]\nname = \"x\"\n[run]\n", "run"),
        ] {
            let e = ProjectFile::parse(bad).unwrap_err();
            assert!(e.contains(says), "{bad:?}: {e}");
        }
    }

    #[test]
    fn durations_read_and_write_back() {
        for (text, secs) in [("15m", 900), ("8h", 28_800), ("1h30m", 5_400), ("90s", 90), ("1d", 86_400)] {
            assert_eq!(parse_duration(text).unwrap(), secs);
        }
        assert_eq!(format_duration(5_400), "1h30m");
        assert_eq!(format_duration(86_400), "1d");
        for bad in ["", "h", "8", "8x", "-1h"] {
            assert!(parse_duration(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_project_is_found_from_any_folder_below_it() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("repo");
        let deep = root.join("src").join("app");
        std::fs::create_dir_all(&deep).unwrap();
        assert_eq!(Project::find(&deep).unwrap().map(|p| p.root), None);
        ProjectFile::new("acme-web", "development").unwrap().write(&root.join(PROJECT_FILE)).unwrap();
        let p = Project::find(&deep).unwrap().unwrap();
        assert_eq!((p.root.as_path(), p.file.name.as_str()), (root.as_path(), "acme-web"));
        assert_eq!(p.env_file().unwrap(), None);
        std::fs::write(root.join(PROJECT_FILE), "[project]\nname = 1\n").unwrap();
        assert!(Project::find(&deep).is_err(), "a broken reins.toml is said, not skipped");
    }

    #[test]
    fn env_reins_holds_references_and_literals() {
        let text = "# refs\nOPENAI_API_KEY=vault:acme-web/development/OPENAI_API_KEY\n\
                    export STRIPE_SECRET_KEY = env:production/STRIPE_SECRET_KEY\n\
                    LOG_LEVEL=debug                   # non-secret values may stay literal\n\
                    LEGACY=\"vault:Work/AWS prod/Access key\"\nEMPTY=\nGREETING='hello # world'\n";
        let f = EnvFile::parse(text).unwrap();
        let vars: Vec<(&str, &EnvValue)> = f.vars().map(|v| (v.name.as_str(), &v.value)).collect();
        assert_eq!(vars.len(), 6);
        assert_eq!(
            vars[0].1,
            &EnvValue::Reference(Reference::Field(
                SecretRef::parse("vault:acme-web/development/OPENAI_API_KEY").unwrap()
            ))
        );
        assert_eq!(
            vars[1].1,
            &EnvValue::Reference(Reference::ProjectVar {
                env: "production".to_owned(),
                name: "STRIPE_SECRET_KEY".to_owned()
            })
        );
        assert_eq!(vars[2].1, &EnvValue::Literal("debug".to_owned()));
        assert!(matches!(vars[3].1, EnvValue::Reference(Reference::Field(r)) if r.item == "Work/AWS prod"));
        assert_eq!(vars[4].1, &EnvValue::Literal(String::new()));
        assert_eq!(vars[5].1, &EnvValue::Literal("hello # world".to_owned()));
        assert_eq!(f.get("LOG_LEVEL").unwrap().line, 4);
        assert_eq!(EnvFile::parse(&f.render()).unwrap().vars().count(), 6, "the writer's output reads back");
    }

    #[test]
    fn env_reins_mistakes_name_their_lines() {
        let e = EnvFile::parse("A=1\nnot a line\n1B=x\nC=vault:nofield\nA=2\nD=\"open\nE='x' y\n").unwrap_err();
        let lines: Vec<usize> = e.errors.iter().map(|(n, _)| *n).collect();
        assert_eq!(lines, [2, 3, 4, 5, 6, 7], "{e}");
        assert!(e.to_string().contains("line 5: A is set twice (first on line 1)"), "{e}");
        assert!(e.to_string().contains("one line"), "{e}");
    }

    #[test]
    fn rewriting_keeps_comments_and_changes_only_what_it_must() {
        let mut f = EnvFile::parse("# top\nA=vault:x/y/A\n\n# keep\nB=1\n").unwrap();
        f.set("A", EnvValue::Reference(Reference::parse_value("env:dev/A").unwrap())).unwrap();
        f.set("C", EnvValue::Literal("two words".to_owned())).unwrap();
        assert!(f.remove("B") && !f.remove("B"));
        assert_eq!(f.render(), "# top\nA=env:dev/A\n\n# keep\nC=\"two words\"\n");
        assert!(f.set("1X", EnvValue::Literal(String::new())).is_err());
        let fresh = EnvFile::new();
        assert!(fresh.render().starts_with("# Reins Secrets"));
        assert_eq!(EnvFile::parse(&fresh.render()).unwrap().vars().count(), 0);
    }

    #[test]
    fn plaintext_env_files_are_read_with_quotes_exports_and_multiline_keys() {
        let text = "export A=1\nB=\"line\\nbreak\"\nKEY=\"-----BEGIN KEY-----\nabc\n-----END KEY-----\"\nC='$NOT_EXPANDED'\n\
                    D=x # comment\nE=a#b\r\n";
        let vars = parse_dotenv(text).unwrap();
        let got: Vec<(&str, &str)> = vars.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        assert_eq!(
            got,
            [
                ("A", "1"),
                ("B", "line\nbreak"),
                ("KEY", "-----BEGIN KEY-----\nabc\n-----END KEY-----"),
                ("C", "$NOT_EXPANDED"),
                ("D", "x"),
                ("E", "a#b"),
            ]
        );
        assert!(parse_dotenv("A=\"never closed\nB=1\n").is_err());
        assert!(parse_dotenv("A=1\nA=2\n").is_err());
    }

    #[test]
    fn values_are_quoted_only_when_needed() {
        assert_eq!(quote_if_needed("debug"), "debug");
        assert_eq!(quote_if_needed(""), "\"\"");
        assert_eq!(quote_if_needed("a #b"), "\"a #b\"");
        assert_eq!(quote_if_needed(" x"), "\" x\"");
        assert_eq!(quote_if_needed("say \"hi\"\n"), "\"say \\\"hi\\\"\\n\"");
    }
}
