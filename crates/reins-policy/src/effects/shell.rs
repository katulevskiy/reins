//! A shell command line's effects: the syntax tree ([`crate::shell::ast`]) walked by a small abstract interpreter
//! that resolves what can be resolved from the line itself and the caller's context (variables and their
//! assignments, `cd`, `for` loops over literal words, functions, `$(mktemp)`, `$(pwd)`, scripts the line writes and
//! then runs), and hands each simple command to its program model ([`super::programs`]).
//!
//! What can't be resolved is named in [`Analysis::unresolved`] and lowers the certainty; the walker never guesses a
//! value that would make a request look milder.

use std::collections::BTreeMap;

use super::level::Analysis;
use super::model::{Area, Certainty, Effect, FileKind, PathRef, SecretKind};
use super::workspace::{Workspace, normalize};
use crate::shell::ast::{self, Command, Heredoc, Part, RedirOp, Script, Simple, Word};

/// What the caller knows about where a command runs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Context {
    /// The working directory (absolute), empty when not known.
    pub cwd: String,
    /// Environment variables the caller knows (`HOME`, `TMPDIR`); any other variable is unknown.
    pub env: BTreeMap<String, String>,
    pub workspace: Workspace,
    /// The repository's default branch (`main`), when known.
    pub default_branch: Option<String>,
    /// The checked-out branch, when known.
    pub branch: Option<String>,
}

impl Context {
    /// A context for `cwd` in `workspace`, with `HOME` set from it.
    #[must_use]
    pub fn new(cwd: &str, workspace: Workspace) -> Self {
        let mut env = BTreeMap::new();
        if !workspace.home.is_empty() {
            env.insert("HOME".to_owned(), workspace.home.clone());
        }
        Self {
            cwd: cwd.to_owned(),
            env,
            workspace,
            default_branch: None,
            branch: None,
        }
    }
}

/// Stands for a part of a word whose value is not known.
pub(super) const UNKNOWN: char = '\u{fffd}';
/// The most effects one analysis keeps; more make the request unreadable.
const MAX_EFFECTS: usize = 512;
/// The most simple commands one analysis walks (loops times functions).
const MAX_STEPS: usize = 4_000;
/// How deep scripts in scripts are read (`sh -c`, `eval`, written scripts, `ssh host '…'`).
const MAX_DEPTH: usize = 6;
/// The most values a `for` loop or a brace expansion is read with.
const MAX_VALUES: usize = 64;

/// A word's value after expansion.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Exp {
    /// The text; [`UNKNOWN`] where a part is not known.
    pub text: String,
    /// Unquoted glob characters are in it.
    pub glob: bool,
    /// Why parts are unknown ("the value of $D").
    pub why: Vec<String>,
}

impl Exp {
    pub fn lit(text: &str) -> Self {
        Self {
            text: text.to_owned(),
            glob: false,
            why: Vec::new(),
        }
    }

    pub fn known(&self) -> bool {
        !self.text.contains(UNKNOWN)
    }

    /// The text when fully known.
    pub fn value(&self) -> Option<&str> {
        self.known().then_some(self.text.as_str())
    }
}

/// What a command's standard input is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Input {
    /// A terminal or nothing that matters.
    None,
    /// Text known exactly (a heredoc, `echo` into a pipe, a file the line wrote).
    Text(String),
    /// Downloaded from a host.
    Downloaded(String),
    /// Decoded or decrypted at run time (`base64 -d`, `xxd -r`, `openssl enc -d`).
    Decoded,
    /// Some other program's output, or a file's content not known here.
    Opaque(String),
    /// Text with values in it that are not known ([`UNKNOWN`] where they are): what it does may be shown, but code in
    /// it is never read with certainty.
    Uncertain(String),
}

/// The walker's state.
pub(super) struct Walk<'a> {
    pub ctx: &'a Context,
    pub cwd: String,
    vars: BTreeMap<String, Option<String>>,
    /// Variables set on some of the ways the line may have gone and not on others: empty on those.
    maybe_unset: std::collections::BTreeSet<String>,
    funcs: BTreeMap<String, Command>,
    /// Files this line wrote, with their content when known.
    pub files: BTreeMap<String, Option<String>>,
    /// Files written on some of the ways the line may have gone and not on others: what was there before, on those.
    pub maybe_unwritten: std::collections::BTreeSet<String>,
    /// A command in this line may have moved HEAD (`checkout`, `switch`, `rebase`…): the request's branch is no longer
    /// known to be the one checked out.
    pub head_moved: bool,
    /// Paths made by `mktemp` in this line: new, so nothing in them is lost.
    pub fresh: Vec<String>,
    /// Commands run on another computer (`ssh host '…'`): their paths are its.
    pub remote: Option<String>,
    pub effects: Vec<Effect>,
    pub unresolved: Vec<String>,
    pub certainty: Certainty,
    depth: usize,
    steps: usize,
    /// Positional parameters inside a function call or a script.
    args: Option<Vec<Exp>>,
    /// The folder before the last `cd` (`cd -`), and `pushd`'s stack.
    pub oldpwd: String,
    pub dirs: Vec<String>,
    /// Whether `dirs` is the whole stack (it starts empty: a fresh shell's).
    pub dirs_known: bool,
}

/// What a branch can change: variables, the folder, the files the line wrote.
#[derive(Clone, PartialEq, Eq)]
struct State {
    vars: BTreeMap<String, Option<String>>,
    maybe_unset: std::collections::BTreeSet<String>,
    cwd: String,
    oldpwd: String,
    dirs: Vec<String>,
    files: BTreeMap<String, Option<String>>,
    maybe_unwritten: std::collections::BTreeSet<String>,
    head_moved: bool,
}

/// The effects of a shell command line run in `ctx`.
#[must_use]
pub fn analyze_command(command: &str, ctx: &Context) -> Analysis {
    // Too long to read at all: nothing else looks at it (the labels' own reader is not bounded).
    if command.len() > ast::MAX_INPUT {
        return Analysis::new(Vec::new(), Certainty::None, vec!["a command too long to read".to_owned()]);
    }
    let mut w = Walk::new(ctx);
    w.run_text(command);
    let mut a = Analysis::new(std::mem::take(&mut w.effects), w.certainty, std::mem::take(&mut w.unresolved));
    a.labels = crate::guard::labels(&[command]).into_iter().map(|g| g.label).collect();
    a
}

impl<'a> Walk<'a> {
    pub fn new(ctx: &'a Context) -> Self {
        let mut vars = BTreeMap::new();
        for (k, v) in &ctx.env {
            vars.insert(k.clone(), Some(v.clone()));
        }
        Self {
            ctx,
            cwd: ctx.cwd.clone(),
            vars,
            maybe_unset: std::collections::BTreeSet::new(),
            funcs: BTreeMap::new(),
            files: BTreeMap::new(),
            maybe_unwritten: std::collections::BTreeSet::new(),
            head_moved: false,
            fresh: Vec::new(),
            remote: None,
            effects: Vec::new(),
            unresolved: Vec::new(),
            certainty: Certainty::Full,
            depth: 0,
            steps: 0,
            args: None,
            oldpwd: String::new(),
            dirs: Vec::new(),
            dirs_known: true,
        }
    }

    fn state(&self) -> State {
        State {
            vars: self.vars.clone(),
            maybe_unset: self.maybe_unset.clone(),
            cwd: self.cwd.clone(),
            oldpwd: self.oldpwd.clone(),
            dirs: self.dirs.clone(),
            files: self.files.clone(),
            maybe_unwritten: self.maybe_unwritten.clone(),
            head_moved: self.head_moved,
        }
    }

    fn restore(&mut self, s: State) {
        self.vars = s.vars;
        self.maybe_unset = s.maybe_unset;
        self.cwd = s.cwd;
        self.oldpwd = s.oldpwd;
        self.dirs = s.dirs;
        self.files = s.files;
        self.maybe_unwritten = s.maybe_unwritten;
        self.head_moved = s.head_moved;
    }

    /// The state after one of several ways a line may go (branches, `&&`, `||`): what they agree on, unknown
    /// otherwise.
    fn join(&mut self, ends: &[State]) {
        let Some(first) = ends.first().cloned() else {
            return;
        };
        let mut j = first;
        for e in &ends[1..] {
            let names: Vec<String> = j.vars.keys().chain(e.vars.keys()).cloned().collect();
            for n in names {
                match (j.vars.get(&n).cloned(), e.vars.get(&n).cloned()) {
                    (a, b) if a == b => {}
                    // Set one way, never set the other: its value, or nothing.
                    (Some(v), None) | (None, Some(v)) => {
                        j.vars.insert(n.clone(), v);
                        j.maybe_unset.insert(n);
                    }
                    _ => {
                        j.vars.insert(n, None);
                    }
                }
            }
            j.maybe_unset.extend(e.maybe_unset.iter().cloned());
            if j.cwd != e.cwd {
                j.cwd.clear();
            }
            if j.oldpwd != e.oldpwd {
                j.oldpwd.clear();
            }
            if j.dirs != e.dirs {
                j.dirs.clear();
                self.dirs_known = false;
            }
            let paths: Vec<String> = j.files.keys().chain(e.files.keys()).cloned().collect();
            for p in paths {
                match (j.files.get(&p).cloned(), e.files.get(&p).cloned()) {
                    (a, b) if a == b => {}
                    // Written one way, not the other: what it wrote, or what was there.
                    (Some(c), None) | (None, Some(c)) => {
                        j.files.insert(p.clone(), c);
                        j.maybe_unwritten.insert(p);
                    }
                    _ => {
                        j.files.insert(p, None);
                    }
                }
            }
            j.maybe_unwritten.extend(e.maybe_unwritten.iter().cloned());
            j.head_moved |= e.head_moved;
        }
        self.restore(j);
    }

    /// Walks one of several ways from `start`, returning the state it ends in.
    fn branch(&mut self, start: &State, f: impl FnOnce(&mut Self)) -> State {
        self.restore(start.clone());
        f(self);
        self.state()
    }

    /// Lowers the certainty to `c` (never raises it).
    pub fn lower(&mut self, c: Certainty, why: impl Into<String>) {
        self.certainty = self.certainty.max(c);
        let why = why.into();
        if !self.unresolved.contains(&why) {
            self.unresolved.push(why);
        }
    }

    /// Records the reasons an expansion is not known, as partial certainty.
    pub fn note(&mut self, e: &Exp) {
        for w in &e.why {
            self.lower(Certainty::Partial, w.clone());
        }
    }

    pub fn effect(&mut self, e: Effect) {
        // A file the line wrote and then writes or deletes again: its content is no longer what it wrote.
        match &e {
            Effect::FileWrite {
                path,
            } => {
                if let Some(c) = self.files.get_mut(&path.path) {
                    *c = None;
                }
                self.maybe_unwritten.remove(&path.path);
            }
            Effect::FileDelete {
                paths,
                ..
            } => {
                for p in paths {
                    if let Some(c) = self.files.get_mut(&p.path) {
                        *c = None;
                    }
                }
            }
            _ => {}
        }
        if self.effects.contains(&e) {
            return;
        }
        if self.effects.len() >= MAX_EFFECTS {
            self.lower(Certainty::None, "more effects than Reins reads in one request");
            return;
        }
        self.effects.push(e);
    }

    /// Parses and walks a script given as text (a line, `sh -c "…"`, a written script, a remote command).
    pub fn run_text(&mut self, text: &str) {
        if self.depth >= MAX_DEPTH {
            self.lower(Certainty::None, "commands nested too deep to read");
            return;
        }
        let parsed = ast::parse(text);
        if !parsed.complete {
            for p in &parsed.problems {
                self.lower(Certainty::None, format!("the command could not be read: {p}"));
            }
        }
        self.depth += 1;
        self.script(&parsed.script);
        self.depth -= 1;
    }

    pub fn script(&mut self, s: &Script) {
        for and_or in &s.0 {
            // Each pipeline after `&&` or `||` runs where the one before left off; the list may end after any of them,
            // so what follows it sees what they agree on.
            let mut ends = Vec::new();
            for (i, p) in and_or.pipelines.iter().enumerate() {
                if i > 0 {
                    ends.push(self.state());
                }
                self.pipeline(p);
            }
            if !ends.is_empty() {
                ends.push(self.state());
                self.join(&ends);
            }
        }
    }

    fn pipeline(&mut self, p: &ast::Pipeline) {
        let mut input = Input::None;
        let n = p.commands.len();
        for (k, c) in p.commands.iter().enumerate() {
            let out = self.command(c, input, k + 1 < n);
            input = out;
        }
    }

    /// Walks one command with `input` on its standard input; returns what it writes to a pipe (when `piped`).
    fn command(&mut self, c: &Command, input: Input, piped: bool) -> Input {
        self.steps += 1;
        if self.steps > MAX_STEPS {
            if self.steps == MAX_STEPS + 1 {
                self.lower(Certainty::None, "a command too long to read in full");
            }
            return Input::None;
        }
        match c {
            Command::Simple(s) => self.simple(s, input, piped),
            Command::Subshell(body, redirects) | Command::Group(body, redirects) => {
                let saved = self.state();
                self.redirects(redirects);
                self.script(body);
                if matches!(c, Command::Subshell(..)) {
                    // A subshell's variables and folder are its own; the files it wrote stay written, and a checkout in it
                    // moved HEAD for everyone.
                    let files = std::mem::take(&mut self.files);
                    let maybe_unwritten = std::mem::take(&mut self.maybe_unwritten);
                    let head_moved = self.head_moved || saved.head_moved;
                    self.restore(State {
                        files,
                        maybe_unwritten,
                        head_moved,
                        ..saved
                    });
                }
                Input::Opaque("a group of commands".to_owned())
            }
            Command::If {
                branches,
                otherwise,
                redirects,
            } => {
                self.redirects(redirects);
                // Each condition runs when the ones before it failed; one body runs, or none.
                let mut ends = Vec::new();
                for (cond, body) in branches {
                    self.script(cond);
                    let after_cond = self.state();
                    ends.push(self.branch(&after_cond, |w| w.script(body)));
                    self.restore(after_cond);
                }
                match otherwise {
                    Some(o) => {
                        let after_conds = self.state();
                        ends.push(self.branch(&after_conds, |w| w.script(o)));
                    }
                    None => ends.push(self.state()),
                }
                self.join(&ends);
                Input::Opaque("a conditional".to_owned())
            }
            Command::While {
                cond,
                body,
                redirects,
            } => {
                self.redirects(redirects);
                self.script(cond);
                let before = self.state();
                self.script(body);
                let after = self.state();
                self.join(&[before, after]);
                Input::Opaque("a loop".to_owned())
            }
            Command::Case {
                word,
                arms,
                redirects,
            } => {
                self.redirects(redirects);
                let _value: Vec<Exp> = self.expand(word);
                // Patterns are expanded too (they may run commands); one arm runs, or none.
                let start = self.state();
                let mut ends = vec![start.clone()];
                for (patterns, body) in arms {
                    self.restore(start.clone());
                    for p in patterns {
                        let _pattern: Vec<Exp> = self.expand(p);
                    }
                    let after_patterns = self.state();
                    ends.push(self.branch(&after_patterns, |w| w.script(body)));
                }
                self.join(&ends);
                Input::Opaque("a case".to_owned())
            }
            Command::For {
                var,
                words,
                body,
                redirects,
            } => {
                self.redirects(redirects);
                let values: Vec<Exp> = match words {
                    Some(ws) => {
                        let mut all = Vec::new();
                        for w in ws {
                            for e in self.expand(w) {
                                // An unquoted variable holding a list is split into words.
                                if w.0.len() == 1 && matches!(w.0[0], Part::Var(_) | Part::Split(_)) && e.known() {
                                    all.extend(e.text.split_whitespace().map(Exp::lit));
                                } else {
                                    all.push(e);
                                }
                            }
                        }
                        all
                    }
                    None => vec![Exp {
                        text: UNKNOWN.to_string(),
                        glob: false,
                        why: vec![format!("the values of ${var}")],
                    }],
                };
                if values.len() > MAX_VALUES {
                    self.lower(Certainty::None, "a loop over more values than Reins reads");
                }
                for v in values.into_iter().take(MAX_VALUES) {
                    if !var.is_empty() {
                        let value = v.value().map(str::to_owned);
                        if value.is_none() {
                            self.note(&v);
                        }
                        self.vars.insert(var.clone(), value);
                    }
                    self.script(body);
                }
                Input::Opaque("a loop".to_owned())
            }
            Command::Function {
                name,
                body,
            } => {
                self.funcs.insert(name.clone(), (**body).clone());
                Input::None
            }
        }
    }

    fn redirects(&mut self, redirects: &[ast::Redirect]) {
        for r in redirects {
            self.redirect(r, None);
        }
    }

    /// A redirection's effect; `content`: what a write puts in the file, when known.
    fn redirect(&mut self, r: &ast::Redirect, content: Option<&str>) {
        if r.op == RedirOp::Dup {
            return;
        }
        for e in self.expand(&r.target) {
            let t = e.text.as_str();
            if t == "-" || device(t) {
                continue;
            }
            match r.op {
                RedirOp::In => self.read_file(&e),
                RedirOp::Out | RedirOp::Append | RedirOp::ReadWrite => {
                    let path = self.path(&e);
                    let key = path.path.clone();
                    let previous = self.files.get(&key).cloned().flatten();
                    // Writing it makes what the line knew of it stale; then its new content, when known.
                    self.write(path);
                    if e.known() {
                        let text = match (r.op, previous, content) {
                            (RedirOp::Append, Some(p), Some(c)) => Some(format!("{p}{c}")),
                            (RedirOp::Append, _, _) => None,
                            _ => content.map(str::to_owned),
                        };
                        self.files.insert(key, text);
                    }
                }
                RedirOp::Dup => {}
            }
        }
    }

    /// Records a change to `p` that leaves its content as it was (its mode or owner).
    pub fn write_keeping_content(&mut self, p: PathRef) {
        let key = p.path.clone();
        let kept = self.files.get(&key).cloned();
        self.write(p);
        if let Some(c) = kept {
            self.files.insert(key, c);
        }
    }

    /// Records a write to `p` (nothing for a fresh path's own creation).
    pub fn write(&mut self, p: PathRef) {
        if device(&p.path) {
            return;
        }
        self.effect(Effect::FileWrite {
            path: p,
        });
    }

    /// Records a read of the file `e` names (kind from its name).
    pub fn read_file(&mut self, e: &Exp) {
        self.note(e);
        let path = self.path(e);
        let kind = file_kind(&path.path);
        if device(&path.path) || (kind == FileKind::Other && path.area.in_workspace()) {
            // Reading the project's own files is no effect worth a line.
            return;
        }
        self.effect(Effect::FileRead {
            path,
            kind,
        });
    }

    /// A path whose name, size, times or link is read, never its contents (`ls`, `du`, `stat`): no secret is read.
    pub fn read_meta(&mut self, e: &Exp) {
        self.note(e);
        let path = self.path(e);
        if device(&path.path) || path.area.in_workspace() {
            return;
        }
        self.effect(Effect::FileRead {
            path,
            kind: FileKind::Other,
        });
    }

    /// The path an expanded word names, classified.
    pub fn path(&mut self, e: &Exp) -> PathRef {
        self.note(e);
        if let Some(host) = &self.remote {
            return PathRef {
                path: e.text.replace(UNKNOWN, "…"),
                area: Area::Remote(host.clone()),
                recoverable: false,
            };
        }
        let home = &self.ctx.workspace.home;
        let (text, partial) = match e.text.find(UNKNOWN) {
            None => (e.text.clone(), false),
            // Known up to a folder: the unknown part is assumed inside it (01 §2.4).
            Some(at) if e.text[..at].ends_with('/') && at > 1 => (e.text[..at].trim_end_matches('/').to_owned(), true),
            Some(_) => {
                return PathRef {
                    path: e.text.replace(UNKNOWN, "…"),
                    area: Area::Unknown,
                    recoverable: false,
                };
            }
        };
        if self.cwd.is_empty() && !text.starts_with('/') && !text.starts_with('~') {
            self.lower(Certainty::Partial, format!("which folder `{}` is in", e.text.replace(UNKNOWN, "…")));
        }
        let path = normalize(&text, &self.cwd, home);
        // `/proc/self/root/x` is `/x`, `/proc/self/cwd/x` is `./x`: read through them, never as `/proc`.
        let Some(path) = resolve_proc(&path, &self.cwd) else {
            self.lower(Certainty::Partial, "a path through another process's folders");
            return PathRef {
                path,
                area: Area::Unknown,
                recoverable: false,
            };
        };
        let area = if self.fresh.iter().any(|f| path == *f || path.starts_with(&format!("{f}/"))) {
            Area::Scratch
        } else {
            self.ctx.workspace.area(&path)
        };
        let recoverable = self.ctx.workspace.recoverable(&path);
        PathRef {
            path: if partial {
                format!("{path}/…")
            } else {
                path
            },
            area,
            recoverable,
        }
    }

    /// The value of variable `name` (`None` when unknown).
    fn var(&self, name: &str) -> Option<String> {
        if let Some(args) = &self.args
            && let Ok(n) = name.parse::<usize>()
        {
            return match n {
                0 => None,
                n => args.get(n - 1).and_then(|a| a.value().map(str::to_owned)),
            };
        }
        match name {
            "PWD" => (!self.cwd.is_empty()).then(|| self.cwd.clone()),
            "HOME" if !self.ctx.workspace.home.is_empty() => Some(self.ctx.workspace.home.clone()),
            // Numbers: a process id, a status, a count.
            "$" | "!" | "?" | "#" | "RANDOM" | "SECONDS" | "EPOCHSECONDS" | "LINENO" | "BASHPID" => {
                Some("1234".to_owned())
            }
            _ => self.vars.get(name).cloned().flatten(),
        }
    }

    /// Expands a word: its values (more than one for a brace expansion or `$@`).
    pub fn expand(&mut self, w: &Word) -> Vec<Exp> {
        // `"$@"` alone is one word per argument.
        if let [Part::Var(name)] = w.0.as_slice()
            && name == "@"
            && let Some(args) = &self.args
        {
            return args.clone();
        }
        // `$X` alone outside quotes: its value split into words, each a glob when it has glob characters.
        if let [Part::Split(name)] = w.0.as_slice()
            && let Some(v) = self.var(name)
        {
            let words: Vec<Exp> = v
                .split_whitespace()
                .map(|t| Exp {
                    text: t.to_owned(),
                    glob: t.contains(['*', '?', '[']),
                    why: Vec::new(),
                })
                .collect();
            // Nothing at all when empty (bash drops the word).
            return words;
        }
        let mut outs = vec![Exp::lit("")];
        for part in &w.0 {
            let alts: Vec<Exp> = match part {
                Part::Lit(t) | Part::Quoted(t) => vec![Exp::lit(t)],
                Part::Glob(g) => vec![Exp {
                    text: g.clone(),
                    glob: true,
                    why: Vec::new(),
                }],
                Part::Tilde(user) => match user {
                    None if !self.ctx.workspace.home.is_empty() => vec![Exp::lit(&self.ctx.workspace.home)],
                    None => vec![Exp::lit("~")],
                    Some(u) => vec![Exp {
                        text: UNKNOWN.to_string(),
                        glob: false,
                        why: vec![format!("the home folder of {u}")],
                    }],
                },
                Part::Var(name) if name == "@" || name == "*" => match self.args.clone() {
                    Some(args) => {
                        if args.is_empty() {
                            vec![Exp::lit("")]
                        } else {
                            vec![Exp {
                                text: args.iter().map(|a| a.text.clone()).collect::<Vec<_>>().join(" "),
                                glob: false,
                                why: args.iter().flat_map(|a| a.why.clone()).collect(),
                            }]
                        }
                    }
                    None => vec![unknown(format!("the value of ${name}"))],
                },
                Part::Var(name) | Part::Split(name) => match self.var(name) {
                    // Maybe never set: in a word with more in it, nothing is one of its values (`"$D"/x` is `/x`).
                    Some(v) if self.maybe_unset.contains(name) && w.0.len() > 1 => vec![
                        Exp {
                            glob: matches!(part, Part::Split(_)) && v.contains(['*', '?', '[']),
                            ..Exp::lit(&v)
                        },
                        Exp::lit(""),
                    ],
                    Some(v) => vec![Exp {
                        glob: matches!(part, Part::Split(_)) && v.contains(['*', '?', '[']),
                        ..Exp::lit(&v)
                    }],
                    None => vec![unknown(format!("the value of ${name}"))],
                },
                Part::Default(name, word) => match self.var(name) {
                    Some(v) if !v.is_empty() => vec![Exp::lit(&v)],
                    // Set on this line to a value not known: not the default.
                    None if self.vars.get(name).is_some_and(Option::is_none) => {
                        let _default: Vec<Exp> = self.expand(word);
                        vec![unknown(format!("the value of ${name}"))]
                    }
                    _ => {
                        let alts = self.expand(word);
                        // The variable may be set where the command runs: the default is one possibility.
                        if self.vars.contains_key(name) || self.args.is_some() {
                            alts
                        } else {
                            alts.into_iter()
                                .map(|mut e| {
                                    if !self.ctx.env.contains_key(name) {
                                        e.why.push(format!("whether ${name} is set"));
                                    }
                                    e
                                })
                                .collect()
                        }
                    }
                },
                Part::Param(inner) => vec![self.param(inner)],
                Part::Op(inner, word) => {
                    // What its word runs counts; its value is not known.
                    let _word: Vec<Exp> = self.expand(word);
                    vec![unknown(format!("the value of ${{{inner}}}"))]
                }
                Part::Arith(text) => {
                    // A number; commands in it run.
                    if text.contains("$(") || text.contains('`') {
                        let (word, complete) = ast::parse_double_quoted(text);
                        if !complete {
                            self.lower(Certainty::None, "arithmetic that could not be read");
                        }
                        let _value: Vec<Exp> = self.expand(&word);
                    }
                    vec![Exp::lit("0")]
                }
                Part::CmdSub(script) => vec![self.substitution(script)],
                Part::ProcSub(script) => {
                    self.script(script);
                    // A pipe, named `/dev/fd/N`: its commands were read above.
                    vec![Exp::lit("/dev/stdin")]
                }
                Part::Brace(words) => {
                    let mut all = Vec::new();
                    for w in words {
                        all.extend(self.expand(w));
                    }
                    all
                }
            };
            let mut next = Vec::new();
            if outs.len().saturating_mul(alts.len()) > MAX_VALUES {
                self.lower(Certainty::None, "a word with more values than Reins reads");
            }
            for o in &outs {
                for a in &alts {
                    if next.len() >= MAX_VALUES {
                        break;
                    }
                    let mut why = o.why.clone();
                    why.extend(a.why.iter().cloned());
                    next.push(Exp {
                        text: format!("{}{}", o.text, a.text),
                        glob: o.glob || a.glob,
                        why,
                    });
                }
            }
            outs = next;
        }
        outs
    }

    /// `${…}` operations: the common suffix and prefix removals on known values.
    fn param(&mut self, inner: &str) -> Exp {
        let name_end = inner.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).unwrap_or(inner.len());
        let (name, op) = inner.split_at(name_end);
        let Some(value) = (!name.is_empty()).then(|| self.var(name)).flatten() else {
            return unknown(format!("the value of ${{{inner}}}"));
        };
        let strip = |pattern: &str, suffix: bool, longest: bool| -> String {
            let n = value.len();
            let cuts: Vec<usize> = (0..=n).filter(|&k| value.is_char_boundary(k)).collect();
            let matches = |piece: &str| crate::shell::glob(pattern, piece);
            if suffix {
                let order: Box<dyn Iterator<Item = &usize>> = if longest {
                    Box::new(cuts.iter())
                } else {
                    Box::new(cuts.iter().rev())
                };
                for &k in order {
                    if matches(&value[k..]) {
                        return value[..k].to_owned();
                    }
                }
            } else {
                let order: Box<dyn Iterator<Item = &usize>> = if longest {
                    Box::new(cuts.iter().rev())
                } else {
                    Box::new(cuts.iter())
                };
                for &k in order {
                    if matches(&value[..k]) {
                        return value[k..].to_owned();
                    }
                }
            }
            value.clone()
        };
        if let Some(p) = op.strip_prefix("%%") {
            Exp::lit(&strip(p, true, true))
        } else if let Some(p) = op.strip_prefix('%') {
            Exp::lit(&strip(p, true, false))
        } else if let Some(p) = op.strip_prefix("##") {
            Exp::lit(&strip(p, false, true))
        } else if let Some(p) = op.strip_prefix('#') {
            Exp::lit(&strip(p, false, false))
        } else {
            unknown(format!("the value of ${{{inner}}}"))
        }
    }

    /// `$(…)`: its commands are walked (their effects count), and its output is known for a few programs.
    fn substitution(&mut self, script: &Script) -> Exp {
        let simple = match script.0.as_slice() {
            [one] if one.pipelines.len() == 1 && one.pipelines[0].commands.len() == 1 => {
                match &one.pipelines[0].commands[0] {
                    Command::Simple(s) => Some(s.clone()),
                    _ => None,
                }
            }
            _ => None,
        };
        let value = simple.as_ref().and_then(|s| self.known_output(s));
        let saved_cwd = self.cwd.clone();
        self.script(script);
        self.cwd = saved_cwd;
        if let Some(v) = value {
            Exp::lit(&v)
        } else {
            let what = simple
                .and_then(|s| s.words.first().map(Word::display))
                .map_or_else(|| "commands".to_owned(), |p| format!("`{p}`"));
            unknown(format!("the output of {what}"))
        }
    }

    /// The output of a command that only computes a value from its words.
    fn known_output(&mut self, s: &Simple) -> Option<String> {
        let mut words: Vec<String> = Vec::new();
        for w in &s.words {
            for e in self.expand(w) {
                words.push(e.value()?.to_owned());
            }
        }
        let (program, args) = words.split_first()?;
        let program = crate::shell::program_name(program);
        match program.as_str() {
            "pwd" => (!self.cwd.is_empty()).then(|| self.cwd.clone()),
            "mktemp" => Some(self.mktemp(args)),
            "echo" => Some(
                args.iter()
                    .filter(|a| !matches!(a.as_str(), "-n" | "-e" | "-E"))
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(" "),
            ),
            "dirname" => args.first().map(|a| match a.trim_end_matches('/').rsplit_once('/') {
                Some(("", _)) => "/".to_owned(),
                Some((d, _)) => d.to_owned(),
                None => ".".to_owned(),
            }),
            "basename" => {
                args.first().map(|a| a.trim_end_matches('/').rsplit('/').next().unwrap_or_default().to_owned())
            }
            "realpath" | "readlink" => {
                let p = args.iter().find(|a| !a.starts_with('-'))?;
                Some(normalize(p, &self.cwd, &self.ctx.workspace.home))
            }
            "git" => {
                let top = args.iter().any(|a| a == "--show-toplevel");
                if top {
                    self.repo_root()
                } else {
                    None
                }
            }
            // Values that are never a path anywhere a request would care about.
            // (`date +FORMAT`, `expr`, `seq -f` and `wc` print what their words say: not here.)
            "whoami" | "id" | "hostname" | "uname" | "nproc" => Some("x".to_owned()),
            // Numbers, unless a format or a separator says otherwise.
            "seq" if args.iter().all(|a| !a.starts_with('-') || a == "-w" || a.parse::<f64>().is_ok()) => {
                Some("1".to_owned())
            }
            _ => None,
        }
    }

    /// The branch checked out where git runs now: the request's own, while the line stays in the worktree the request
    /// came from (after `cd` into another repository it is not known) and nothing in it may have moved HEAD.
    pub fn current_branch(&self) -> Option<String> {
        // Any move, even to a branch the line names (`checkout -b x` may fail, `-B` may reuse one): not known.
        if self.head_moved {
            return None;
        }
        let here = self.repo_root()?;
        let start = Walk::new(self.ctx).repo_root()?;
        (here == start).then(|| self.ctx.branch.clone()).flatten()
    }

    /// The worktree `cwd` is in, when the workspace knows it.
    pub fn repo_root(&self) -> Option<String> {
        let home = &self.ctx.workspace.home;
        let mut best: Option<String> = None;
        for w in &self.ctx.workspace.worktrees {
            let root = normalize(w, "/", home);
            if root.contains('*') {
                // A pattern of worktrees: the one cwd is in, by as many parts.
                let n = root.split('/').count();
                let parts: Vec<&str> = self.cwd.split('/').collect();
                if parts.len() >= n {
                    let candidate = parts[..n].join("/");
                    if root.split('/').zip(candidate.split('/')).all(|(r, c)| crate::shell::glob(r, c)) {
                        best = Some(candidate);
                    }
                }
            } else if (self.cwd == root || self.cwd.starts_with(&format!("{root}/")))
                && best.as_ref().is_none_or(|b| root.len() > b.len())
            {
                best = Some(root);
            }
        }
        best
    }

    /// `mktemp [-d] [-p DIR | --tmpdir=DIR] [TEMPLATE]`: a new path, recorded as fresh.
    pub fn mktemp(&mut self, args: &[String]) -> String {
        let mut dir = self.vars.get("TMPDIR").cloned().flatten().unwrap_or_else(|| "/tmp".to_owned());
        let mut template = None;
        let mut k = 0;
        while k < args.len() {
            let a = &args[k];
            if a == "-p" || a == "--tmpdir" {
                if let Some(d) = args.get(k + 1) {
                    dir.clone_from(d);
                }
                k += 1;
            } else if let Some(d) = a.strip_prefix("--tmpdir=") {
                d.clone_into(&mut dir);
            } else if !a.starts_with('-') {
                template = Some(a.clone());
            }
            k += 1;
        }
        let name = template.map_or_else(|| "tmp.reins".to_owned(), |t| t.replace('X', "r"));
        let path = if name.contains('/') {
            normalize(&name, &self.cwd, &self.ctx.workspace.home)
        } else {
            normalize(&format!("{dir}/{name}"), &self.cwd, &self.ctx.workspace.home)
        };
        self.fresh.push(path.clone());
        path
    }

    /// Binds an assignment.
    pub fn assign(&mut self, name: &str, value: &Word) {
        let exps = self.expand(value);
        let v = match exps.as_slice() {
            [one] => one.value().map(str::to_owned),
            _ => None,
        };
        self.maybe_unset.remove(name.trim_end_matches('+'));
        // `X+=v` appends to what X held (nothing, when never set).
        if let Some(base) = name.strip_suffix('+') {
            let joined = match self.vars.get(base) {
                Some(None) => None,
                Some(Some(old)) => v.map(|v| format!("{old}{v}")),
                None => self.ctx.env.get(base).map_or(v.clone(), |old| v.map(|v| format!("{old}{v}"))),
            };
            self.vars.insert(base.to_owned(), joined);
            return;
        }
        self.vars.insert(name.to_owned(), v);
    }

    /// `unset X`: X is no longer set.
    pub fn unset(&mut self, name: &str) {
        // Expands to nothing from here on.
        self.vars.insert(name.to_owned(), Some(String::new()));
    }

    /// Marks a variable as set to an unknown value (`read x`).
    pub fn forget(&mut self, name: &str) {
        self.maybe_unset.remove(name);
        self.vars.insert(name.to_owned(), None);
    }

    /// `shift [n]`: drops the first `n` positional parameters.
    pub fn shift(&mut self, n: usize) {
        if let Some(args) = &mut self.args {
            let n = n.min(args.len());
            args.drain(..n);
        }
    }

    /// A variable this line set: `Some(Some(value))`, `Some(None)` when set to a value not known, `None` when the line
    /// never set it (nor the caller's environment).
    /// A variable whose name starts with `prefix` is set on this line (or in the caller's environment).
    pub fn has_var_prefix(&self, prefix: &str) -> bool {
        self.vars.keys().any(|k| k.starts_with(prefix))
    }

    #[allow(clippy::option_option, reason = "never set, set to a value not known, or set to a value")]
    pub fn var_state(&self, name: &str) -> Option<Option<String>> {
        self.vars.get(name).cloned()
    }

    pub fn set_var(&mut self, name: &str, value: Option<String>) {
        self.maybe_unset.remove(name);
        self.vars.insert(name.to_owned(), value);
    }

    fn simple(&mut self, s: &Simple, input: Input, piped: bool) -> Input {
        // The program and its words.
        let mut words: Vec<Exp> = Vec::new();
        for w in &s.words {
            words.extend(self.expand(w));
        }
        // Heredocs and here-strings: the input the command reads.
        let mut input = input;
        for d in &s.heredocs {
            input = self.heredoc_input(d);
        }
        if words.is_empty() {
            // Only assignments (and redirections): they bind for the rest of the line.
            for (name, value) in &s.assigns {
                self.assign(name, value);
            }
            let content = match &input {
                Input::Text(t) => Some(t.clone()),
                _ => None,
            };
            for r in &s.redirects {
                self.redirect(r, content.as_deref());
            }
            return Input::None;
        }
        // Assignments before a command apply to it only.
        let saved: Vec<(String, Option<Option<String>>)> = s
            .assigns
            .iter()
            .map(|(name, _)| {
                let base = name.trim_end_matches('+').to_owned();
                let old = self.vars.get(&base).cloned();
                (base, old)
            })
            .collect();
        for (name, value) in &s.assigns {
            self.assign(name, value);
        }
        let out = self.run_simple(s, &words, input, piped);
        for (name, old) in saved {
            match old {
                Some(v) => {
                    self.vars.insert(name, v);
                }
                None => {
                    self.vars.remove(&name);
                }
            }
        }
        out
    }

    fn run_simple(&mut self, s: &Simple, words: &[Exp], input: Input, piped: bool) -> Input {
        let written = &s.words[0];
        if !words[0].known() || words[0].glob {
            self.lower(Certainty::None, format!("the program `{}` is named at run time", written.display()));
            self.note(&words[0]);
            return Input::Opaque("an unknown program".to_owned());
        }
        if words[0].text.chars().any(|c| !c.is_ascii() || c.is_ascii_control()) {
            self.lower(Certainty::None, "a program name with characters that only look like others");
            return Input::Opaque("an unknown program".to_owned());
        }
        // A function defined in this line.
        if let Some(body) = self.funcs.get(&words[0].text).cloned() {
            let saved = self.args.replace(words[1..].to_vec());
            self.depth += 1;
            if self.depth < MAX_DEPTH {
                let _out: Input = self.command(&body, input, piped);
            } else {
                self.lower(Certainty::None, "a function calling itself too deep");
            }
            self.depth -= 1;
            self.args = saved;
            self.redirects(&s.redirects);
            return Input::Opaque("a function".to_owned());
        }
        let content_for_redirect = super::programs::writes_text(self, words, &input);
        let out = super::programs::run(self, words, input);
        for r in &s.redirects {
            let content = if matches!(r.op, RedirOp::Out | RedirOp::Append) {
                content_for_redirect.as_deref()
            } else {
                None
            };
            self.redirect(r, content);
        }
        out
    }

    fn heredoc_input(&mut self, d: &Heredoc) -> Input {
        if let Some(w) = &d.word {
            let exps = self.expand(w);
            return match exps.as_slice() {
                [one] if one.known() => Input::Text(format!("{}\n", one.text)),
                // What it feeds matters only to a shell or an interpreter, which then can't read it.
                _ => Input::Opaque("a here-string with values not known".to_owned()),
            };
        }
        if !d.expand {
            return Input::Text(d.body.clone());
        }
        let (word, complete) = ast::parse_double_quoted(&d.body);
        if !complete {
            self.lower(Certainty::None, "a heredoc that could not be read");
        }
        let exps = self.expand(&word);
        match exps.as_slice() {
            [one] if one.known() => Input::Text(one.text.clone()),
            // Unknown values in a heredoc's text: known but for them; never read as code with certainty.
            [one] => Input::Uncertain(one.text.clone()),
            _ => Input::Opaque("a heredoc".to_owned()),
        }
    }

    /// Runs `text` as a script with positional `args` (a written script run, `sh -c`), in a nested scope.
    pub fn run_script(&mut self, text: &str, args: Vec<Exp>) {
        let saved = self.args.replace(args);
        let saved_cwd = self.cwd.clone();
        self.run_text(text);
        self.cwd = saved_cwd;
        self.args = saved;
    }

    /// Runs `text` on `host` (its paths are the host's).
    pub fn run_remote(&mut self, host: &str, text: &str) {
        let saved = self.remote.replace(host.to_owned());
        let saved_cwd = std::mem::take(&mut self.cwd);
        self.run_text(text);
        self.cwd = saved_cwd;
        self.remote = saved;
    }
}

/// Device files that are no file: reading or writing them touches nothing. Only these exact paths: `/dev/sda`,
/// `/dev/mem` or `/proc/self/environ` are not.
pub(super) const DEVICES: &[&str] =
    &["/dev/null", "/dev/stdin", "/dev/stdout", "/dev/stderr", "/dev/zero", "/dev/urandom", "/dev/random", "/dev/tty"];

pub(super) fn device(path: &str) -> bool {
    DEVICES.contains(&path)
}

/// A path through `/proc/<pid>/root` or `/proc/<pid>/cwd` as the path it reaches: `/proc/self/root/home/u/x` is
/// `/home/u/x`, `/proc/self/cwd/x` is `x` in `cwd`. `None` for another process's working directory (not known here).
#[must_use]
pub fn resolve_proc(path: &str, cwd: &str) -> Option<String> {
    let mut p = path.to_owned();
    for _ in 0..8 {
        let parts: Vec<&str> = p.trim_start_matches('/').split('/').collect();
        if parts.len() < 3 || parts[0] != "proc" {
            return Some(p);
        }
        let own = matches!(parts[1], "self" | "thread-self");
        let pid = own || parts[1].chars().all(|c| c.is_ascii_digit());
        if !pid {
            return Some(p);
        }
        let rest = parts[3..].join("/");
        p = match parts[2] {
            "root" => format!("/{rest}"),
            "cwd" if own && cwd.starts_with('/') => normalize(&rest, cwd, ""),
            "cwd" => return None,
            _ => return Some(p),
        };
    }
    Some(p)
}

pub(super) fn unknown(why: String) -> Exp {
    Exp {
        text: UNKNOWN.to_string(),
        glob: false,
        why: vec![why],
    }
}

/// `name` ends with `.ext`, in any case.
fn has_ext(name: &str, ext: &str) -> bool {
    name.rsplit_once('.').is_some_and(|(_, e)| e.eq_ignore_ascii_case(ext))
}

/// Names of secret files a pattern or an unknown name in their folder may be.
const SECRET_NAMES: &[&str] = &[
    "credentials",
    "config",
    "config.json",
    "credentials.json",
    "terraform.tfstate",
    ".netrc",
    ".git-credentials",
    ".npmrc",
    ".pypirc",
    ".vault-token",
    "id_rsa",
    "key.pem",
    "session.json",
    "control.token",
    "phone-key.json",
];

/// What kind of file `path` is, by its name (08 R4): private keys, keystores, cloud credentials and token files are
/// secrets; `.ssh/config`, `known_hosts`, `authorized_keys` and public keys are not.
#[must_use]
pub fn file_kind(path: &str) -> FileKind {
    let lower = path.to_lowercase();
    let name = lower.rsplit('/').next().unwrap_or(&lower);
    // A process's environment, memory and arguments hold its tokens; raw memory and disks hold everything.
    // An unknown name under /proc is any of these: the worst plausible.
    if lower.starts_with("/proc/")
        && (matches!(name, "environ" | "mem" | "cmdline" | "maps" | "smaps" | "auxv" | "pagemap")
            || lower.contains('\u{2026}'))
    {
        return FileKind::Secret(SecretKind::Token);
    }
    if lower.starts_with("/dev/")
        && (matches!(name, "mem" | "kmem" | "port")
            || ["sd", "nvme", "hd", "vd", "xvd", "mmcblk", "dm-", "md", "disk", "loop"]
                .iter()
                .any(|d| name.starts_with(d))
            || lower.starts_with("/dev/mapper/")
            || lower.starts_with("/dev/disk/"))
    {
        return FileKind::Secret(SecretKind::Token);
    }
    let in_ssh = lower.contains("/.ssh/") || lower.starts_with(".ssh/");
    if in_ssh {
        let public = has_ext(name, "pub")
            || matches!(name, "config" | "known_hosts" | "known_hosts.old" | "authorized_keys" | "environment" | "rc")
            || lower.contains("/.ssh/config.d/");
        return if public || name.is_empty() {
            FileKind::Other
        } else {
            FileKind::Secret(SecretKind::PrivateKey)
        };
    }
    if has_ext(name, "pub") {
        return FileKind::Other;
    }
    // A pattern or an unknown name: a secret when it may name one in its folder.
    if name == "\u{2026}" || name.contains(['*', '?', '[']) {
        let dir = &lower[..lower.len() - name.len()];
        for candidate in SECRET_NAMES {
            if name == "\u{2026}" || crate::shell::glob(name, candidate) {
                let k = file_kind(&format!("{dir}{candidate}"));
                if matches!(k, FileKind::Secret(_)) {
                    return k;
                }
            }
        }
    }
    let Some(c) = crate::guard::classify_file(path) else {
        return FileKind::Other;
    };
    match c.group.id {
        "env-files" => FileKind::Env,
        "keys" => {
            if [".keystore", ".jks", ".p12", ".pfx"].iter().any(|e| name.ends_with(e)) {
                FileKind::Secret(SecretKind::Keystore)
            } else {
                FileKind::Secret(SecretKind::PrivateKey)
            }
        }
        "cloud" => FileKind::Secret(SecretKind::CloudCredential),
        "tokens" => FileKind::Secret(SecretKind::Token),
        "reins" => FileKind::Secret(SecretKind::Reins),
        _ => FileKind::Other,
    }
}
