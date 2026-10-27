//! A shell command line as a syntax tree (bash and POSIX sh), for the consequence engine
//! (`crate::effects`). Unlike the guard's tokenizer ([`super::segments`]) it keeps structure: lists, pipelines,
//! redirections with their targets, heredoc bodies, loops, conditionals, functions, and every word as its parts
//! (literals, quoted text, variables, command substitutions, globs, brace expansions), so the engine can resolve
//! what can be resolved and say exactly what can't.
//!
//! It never fails: anything it can't read (an unclosed quote, a `$(` that never closes, a heredoc whose body never
//! comes) is reported in [`Parsed::problems`] and [`Parsed::complete`] is `false`; what it read so far is kept.

/// How deep substitutions and compound commands nest before the rest is left unread.
const MAX_DEPTH: usize = 24;

/// A parsed command line.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Parsed {
    pub script: Script,
    /// Every part was read: no problem below.
    pub complete: bool,
    /// What could not be read, in words.
    pub problems: Vec<String>,
}

/// Commands run one after the other (`;`, `&`, newlines).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Script(pub Vec<AndOr>);

/// Pipelines joined by `&&` and `||` (both may run: the engine reads all of them).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AndOr {
    pub pipelines: Vec<Pipeline>,
    /// For each pipeline after the first: it follows `||` (else `&&`).
    pub ors: Vec<bool>,
    /// It ends with `&`: it runs in the background, in a shell of its own.
    pub background: bool,
}

/// Commands joined by `|`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pipeline {
    pub commands: Vec<Command>,
    /// It starts with `!`: its status is turned around.
    pub negated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Simple(Simple),
    /// `( … )`.
    Subshell(Script, Vec<Redirect>),
    /// `{ …; }`.
    Group(Script, Vec<Redirect>),
    /// `if`/`elif` conditions and bodies, then `else`: every branch may run.
    If {
        branches: Vec<(Script, Script)>,
        otherwise: Option<Script>,
        redirects: Vec<Redirect>,
    },
    /// `for var in words; do body; done` (`words` is `None` for `for var; do`, the positional parameters, and for
    /// the arithmetic `for ((…))`).
    For {
        var: String,
        words: Option<Vec<Word>>,
        body: Script,
        redirects: Vec<Redirect>,
    },
    /// `while`/`until`.
    While {
        cond: Script,
        body: Script,
        redirects: Vec<Redirect>,
    },
    Case {
        word: Word,
        arms: Vec<(Vec<Word>, Script)>,
        redirects: Vec<Redirect>,
    },
    /// `name() { … }`, `function name { … }`.
    Function {
        name: String,
        body: Box<Command>,
    },
}

/// A simple command: assignments, words (the program first) and redirections.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Simple {
    pub assigns: Vec<(String, Word)>,
    pub words: Vec<Word>,
    pub redirects: Vec<Redirect>,
    /// Heredocs and here-strings fed to it, in order.
    pub heredocs: Vec<Heredoc>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RedirOp {
    /// `>`, `>|`, `&>`.
    Out,
    /// `>>`, `&>>`.
    Append,
    /// `<`.
    In,
    /// `<>`.
    ReadWrite,
    /// `>&N`, `<&N`, `>&-`: another descriptor, no file.
    Dup,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Redirect {
    pub fd: Option<u32>,
    pub op: RedirOp,
    pub target: Word,
}

/// A heredoc's body or a here-string's word.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Heredoc {
    /// The text as the shell feeds it (for a here-string: empty, see `word`).
    pub body: String,
    /// Its delimiter was unquoted: `$x`, `$(…)` and backticks in it are expanded.
    pub expand: bool,
    /// A here-string (`<<< word`).
    pub word: Option<Word>,
    slot: usize,
}

/// A word, as its parts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Word(pub Vec<Part>);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Part {
    /// Unquoted text (no glob characters in it).
    Lit(String),
    /// Quoted or escaped text: taken as it is.
    Quoted(String),
    /// `$name`, `${name}`, `$1`, `$@`.
    Var(String),
    /// `${name:-word}`, `${name-word}`, `${name:=word}`: the value, else the word.
    Default(String, Word),
    /// `${…}` with an operation this reader does not model (`${x%/*}`, `${#x}`): its value is unknown.
    Param(String),
    /// `${name:+word}`, `${name%word}`, … whose word runs commands (`${x:+$(…)}`): its value is unknown, its word
    /// is walked for what it runs.
    Op(String, Word),
    /// `$name`, `${name}` outside quotes: split into words when it holds spaces.
    Split(String),
    /// `$(…)` or backticks.
    CmdSub(Script),
    /// `<(…)`, `>(…)`: the name of a pipe.
    ProcSub(Script),
    /// `$((…))`: a number.
    Arith(String),
    /// `*`, `?`, `[…]` unquoted.
    Glob(String),
    /// `~` at the start of a word (`Some(user)` for `~user`).
    Tilde(Option<String>),
    /// `{a,b}`, `{1..3}`: the word once per alternative.
    Brace(Vec<Word>),
}

impl Word {
    /// The word's text when it is made of literal parts only (quoted or not, no glob).
    #[must_use]
    pub fn literal(&self) -> Option<String> {
        let mut s = String::new();
        for p in &self.0 {
            match p {
                Part::Lit(t) | Part::Quoted(t) => s.push_str(t),
                _ => return None,
            }
        }
        Some(s)
    }

    /// The word's text as written, roughly (variables as `$name`, substitutions as `$(…)`): for messages.
    #[must_use]
    pub fn display(&self) -> String {
        let mut s = String::new();
        for p in &self.0 {
            match p {
                Part::Lit(t) | Part::Quoted(t) | Part::Glob(t) => s.push_str(t),
                Part::Var(n) | Part::Split(n) => {
                    s.push('$');
                    s.push_str(n);
                }
                Part::Default(n, _) | Part::Param(n) | Part::Op(n, _) => {
                    s.push_str("${");
                    s.push_str(n);
                    s.push('}');
                }
                Part::CmdSub(_) => s.push_str("$(…)"),
                Part::ProcSub(_) => s.push_str("<(…)"),
                Part::Arith(_) => s.push_str("$((…))"),
                Part::Tilde(u) => {
                    s.push('~');
                    s.push_str(u.as_deref().unwrap_or_default());
                }
                Part::Brace(alts) => {
                    s.push('{');
                    s.push_str(&alts.iter().map(Word::display).collect::<Vec<_>>().join(","));
                    s.push('}');
                }
            }
        }
        s
    }
}

/// The longest text read: anything longer is not read at all (`complete` is false).
pub const MAX_INPUT: usize = 64 * 1024;
/// Brace expansions read per text: nested unclosed braces would otherwise be read again and again.
const MAX_BRACES: usize = 512;

/// Parses a command line.
#[must_use]
pub fn parse(text: &str) -> Parsed {
    parse_at(text, 0)
}

fn new_parser(text: &str, depth: usize) -> Parser {
    Parser {
        s: text.chars().collect(),
        i: 0,
        problems: Vec::new(),
        pending: Vec::new(),
        bodies: Vec::new(),
        depth,
        braces: 0,
    }
}

/// [`parse`] at a nesting depth (text inside backticks or `${…}` continues its outer text's depth).
fn parse_at(text: &str, depth: usize) -> Parsed {
    if text.len() > MAX_INPUT {
        return Parsed {
            script: Script::default(),
            complete: false,
            problems: vec!["a command too long to read".to_owned()],
        };
    }
    let mut p = new_parser(text, depth);
    let mut script = p.list(&[]);
    while p.i < p.s.len() {
        // A stray `)`, `}`, `;;` or keyword at the top: skip it and read on.
        let c = p.s[p.i];
        p.problem(format!("unexpected `{c}`"));
        p.i += 1;
        script.0.extend(p.list(&[]).0);
    }
    if !p.pending.is_empty() {
        p.problem("a heredoc whose body never comes".to_owned());
        p.pending.clear();
    }
    let bodies = std::mem::take(&mut p.bodies);
    fill_script(&mut script, &bodies);
    Parsed {
        complete: p.problems.is_empty(),
        problems: p.problems,
        script,
    }
}

/// Parses text as the inside of double quotes (a heredoc body that expands): its variables and substitutions.
#[must_use]
pub fn parse_double_quoted(text: &str) -> (Word, bool) {
    parse_double_quoted_at(text, 0)
}

fn parse_double_quoted_at(text: &str, depth: usize) -> (Word, bool) {
    if text.len() > MAX_INPUT || depth > MAX_DEPTH {
        return (Word(vec![Part::Param("a text too long or too deep to read".to_owned())]), false);
    }
    let mut p = new_parser(text, depth);
    let mut parts = Vec::new();
    p.double_quoted_until(&mut parts, None);
    let bodies = std::mem::take(&mut p.bodies);
    let mut word = Word(parts);
    fill_word(&mut word, &bodies);
    (word, p.problems.is_empty())
}

struct Pending {
    delimiter: String,
    tabs: bool,
    slot: usize,
}

struct Parser {
    s: Vec<char>,
    i: usize,
    problems: Vec<String>,
    pending: Vec<Pending>,
    bodies: Vec<Option<String>>,
    depth: usize,
    /// Brace expansions tried so far (see [`MAX_BRACES`]).
    braces: usize,
}

const KEYWORDS: &[&str] = &["then", "else", "elif", "fi", "do", "done", "esac", "in", "}"];

fn name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

fn valid_name(s: &str) -> bool {
    !s.is_empty() && s.chars().all(name_char) && !s.starts_with(|c: char| c.is_ascii_digit())
}

impl Parser {
    fn problem(&mut self, p: String) {
        if !self.problems.contains(&p) {
            self.problems.push(p);
        }
    }

    fn peek(&self) -> Option<char> {
        self.s.get(self.i).copied()
    }

    fn peek_at(&self, k: usize) -> Option<char> {
        self.s.get(self.i + k).copied()
    }

    fn at(&self, text: &str) -> bool {
        text.chars().enumerate().all(|(k, c)| self.peek_at(k) == Some(c))
    }

    /// Skips spaces, tabs, line continuations and a comment (not newlines).
    fn blanks(&mut self) {
        loop {
            match self.peek() {
                Some(' ' | '\t' | '\r') => self.i += 1,
                Some('\\') if self.peek_at(1) == Some('\n') => self.i += 2,
                Some('#') => {
                    while self.peek().is_some_and(|c| c != '\n') {
                        self.i += 1;
                    }
                }
                _ => return,
            }
        }
    }

    /// Consumes a newline (at `self.i`) and reads the bodies of the heredocs started on its line.
    fn newline(&mut self) {
        self.i += 1;
        for d in std::mem::take(&mut self.pending) {
            let mut body = String::new();
            let mut found = false;
            while self.i < self.s.len() {
                let end = self.s[self.i..].iter().position(|c| *c == '\n').map_or(self.s.len(), |p| self.i + p);
                let line: String = self.s[self.i..end].iter().collect();
                self.i = (end + 1).min(self.s.len());
                let bare = if d.tabs {
                    line.trim_start_matches('\t')
                } else {
                    line.as_str()
                };
                if bare.trim_end_matches('\r') == d.delimiter {
                    found = true;
                    break;
                }
                body.push_str(bare);
                body.push('\n');
            }
            if !found {
                self.problem(format!("the heredoc `{}` never ends", d.delimiter));
            }
            if let Some(slot) = self.bodies.get_mut(d.slot) {
                *slot = Some(body);
            }
        }
    }

    /// Skips blanks and newlines (and `;` when `semis`).
    fn separators(&mut self, semis: bool) {
        loop {
            self.blanks();
            match self.peek() {
                Some('\n') => self.newline(),
                Some(';') if semis && self.peek_at(1) != Some(';') => self.i += 1,
                _ => return,
            }
        }
    }

    /// The reserved word at the cursor, if one is there and ends there.
    fn keyword(&self) -> Option<String> {
        let mut k = 0;
        let mut w = String::new();
        while let Some(c) = self.peek_at(k) {
            if c.is_ascii_alphanumeric() || c == '_' || c == '{' || c == '}' || c == '!' || c == '[' || c == ']' {
                w.push(c);
                k += 1;
                if matches!(c, '{' | '}' | '!') && k == 1 {
                    break;
                }
            } else {
                break;
            }
        }
        let next = self.peek_at(k);
        let ends = next.is_none_or(|c| c.is_whitespace() || matches!(c, ';' | '&' | '|' | ')' | '(' | '<' | '>'));
        (ends && !w.is_empty()).then_some(w)
    }

    fn at_terminator(&self, terms: &[&str]) -> bool {
        match self.peek() {
            None | Some(')') => true,
            Some(';') if self.peek_at(1) == Some(';') || self.peek_at(1) == Some('&') => true,
            _ => self.keyword().is_some_and(|k| terms.contains(&k.as_str()) || (k == "}" && terms.contains(&"}"))),
        }
    }

    /// A list up to one of `terms` (keywords), a `)`, a `;;` or the end.
    fn list(&mut self, terms: &[&str]) -> Script {
        let mut out = Vec::new();
        if self.depth > MAX_DEPTH {
            self.problem("nested too deep".to_owned());
            self.i = self.s.len();
            return Script(out);
        }
        self.depth += 1;
        loop {
            self.separators(true);
            if self.at_terminator(terms) {
                break;
            }
            let before = self.i;
            let mut read = self.and_or(terms);
            self.blanks();
            match self.peek() {
                Some(';') if self.peek_at(1) != Some(';') => self.i += 1,
                Some('&') if self.peek_at(1) != Some('&') => {
                    self.i += 1;
                    if let Some(a) = read.as_mut() {
                        a.background = true;
                    }
                }
                Some('\n') => self.newline(),
                _ => {}
            }
            if let Some(a) = read {
                out.push(a);
            }
            if self.i == before {
                // Nothing read: a character no rule takes.
                if self.at_terminator(terms) {
                    break;
                }
                let c = self.s[self.i];
                self.problem(format!("unexpected `{c}`"));
                self.i += 1;
            }
        }
        self.depth -= 1;
        Script(out)
    }

    fn and_or(&mut self, terms: &[&str]) -> Option<AndOr> {
        let mut pipelines = vec![self.pipeline(terms)?];
        let mut ors = Vec::new();
        loop {
            self.blanks();
            if self.at("&&") || self.at("||") {
                let or = self.at("||");
                self.i += 2;
                self.separators(false);
                if let Some(p) = self.pipeline(terms) {
                    pipelines.push(p);
                    ors.push(or);
                } else {
                    self.problem("a `&&` or `||` with nothing after it".to_owned());
                    break;
                }
            } else {
                break;
            }
        }
        Some(AndOr {
            pipelines,
            ors,
            background: false,
        })
    }

    fn pipeline(&mut self, terms: &[&str]) -> Option<Pipeline> {
        self.blanks();
        let mut negated = false;
        if self.keyword().as_deref() == Some("!") {
            self.i += 1;
            negated = true;
            self.blanks();
        }
        if self.keyword().as_deref() == Some("time") {
            self.i += 4;
            self.blanks();
        }
        // `time ! cmd`, and `! ! cmd`.
        while self.keyword().as_deref() == Some("!") {
            self.i += 1;
            negated = !negated;
            self.blanks();
        }
        let mut commands = vec![self.command(terms)?];
        loop {
            self.blanks();
            if self.peek() == Some('|') && self.peek_at(1) != Some('|') {
                self.i += 1;
                if self.peek() == Some('&') {
                    self.i += 1;
                }
                self.separators(false);
                if let Some(c) = self.command(terms) {
                    commands.push(c);
                } else {
                    self.problem("a `|` with nothing after it".to_owned());
                    break;
                }
            } else {
                break;
            }
        }
        Some(Pipeline {
            commands,
            negated,
        })
    }

    fn redirects_after(&mut self) -> Vec<Redirect> {
        let mut out = Vec::new();
        loop {
            self.blanks();
            match self.redirect() {
                Some(Ok(r)) => out.push(r),
                Some(Err(_)) => {}
                None => return out,
            }
        }
    }

    fn expect_keyword(&mut self, kw: &str) -> bool {
        self.separators(true);
        if self.keyword().as_deref() == Some(kw) {
            self.i += kw.chars().count();
            true
        } else {
            self.problem(format!("expected `{kw}`"));
            false
        }
    }

    #[allow(clippy::too_many_lines, reason = "one case per compound command")]
    /// A command, nested no deeper than [`MAX_DEPTH`] (functions' bodies and compound commands count).
    fn command(&mut self, terms: &[&str]) -> Option<Command> {
        if self.depth > MAX_DEPTH {
            self.problem("nested too deep".to_owned());
            self.i = self.s.len();
            return None;
        }
        self.depth += 1;
        let c = self.command_at(terms);
        self.depth -= 1;
        c
    }

    fn command_at(&mut self, terms: &[&str]) -> Option<Command> {
        self.blanks();
        if self.at_terminator(terms) {
            return None;
        }
        if self.at("((") {
            // An arithmetic command: no effect of its own.
            let text = self.until_double_paren();
            return Some(Command::Simple(Simple {
                words: vec![Word(vec![Part::Lit("((".to_owned())]), Word(vec![Part::Arith(text)])],
                ..Simple::default()
            }));
        }
        if self.peek() == Some('(') {
            self.i += 1;
            let body = self.list(&[]);
            if self.peek() == Some(')') {
                self.i += 1;
            } else {
                self.problem("a `(` that never closes".to_owned());
            }
            let redirects = self.redirects_after();
            return Some(Command::Subshell(body, redirects));
        }
        let kw = self.keyword();
        match kw.as_deref() {
            Some("{") => {
                self.i += 1;
                let body = self.list(&["}"]);
                self.expect_keyword("}");
                let redirects = self.redirects_after();
                return Some(Command::Group(body, redirects));
            }
            Some("if") => {
                self.i += 2;
                let mut branches = Vec::new();
                let mut otherwise = None;
                loop {
                    let cond = self.list(&["then"]);
                    self.expect_keyword("then");
                    let body = self.list(&["elif", "else", "fi"]);
                    branches.push((cond, body));
                    match self.keyword().as_deref() {
                        Some("elif") => self.i += 4,
                        Some("else") => {
                            self.i += 4;
                            otherwise = Some(self.list(&["fi"]));
                            self.expect_keyword("fi");
                            break;
                        }
                        _ => {
                            self.expect_keyword("fi");
                            break;
                        }
                    }
                }
                let redirects = self.redirects_after();
                return Some(Command::If {
                    branches,
                    otherwise,
                    redirects,
                });
            }
            Some("while" | "until") => {
                self.i += kw.as_deref().map_or(0, str::len);
                let cond = self.list(&["do"]);
                self.expect_keyword("do");
                let body = self.list(&["done"]);
                self.expect_keyword("done");
                let redirects = self.redirects_after();
                return Some(Command::While {
                    cond,
                    body,
                    redirects,
                });
            }
            Some("for" | "select") => {
                self.i += kw.as_deref().map_or(0, str::len);
                self.blanks();
                if self.at("((") {
                    let header: String = self.until_double_paren();
                    if header.contains("$(") || header.contains('`') {
                        self.problem("a for loop whose header runs commands".to_owned());
                    }
                    self.separators(true);
                    let body = self.loop_body();
                    let redirects = self.redirects_after();
                    return Some(Command::For {
                        var: String::new(),
                        words: None,
                        body,
                        redirects,
                    });
                }
                let var = self.word().and_then(|w| w.literal()).unwrap_or_default();
                if !valid_name(&var) {
                    self.problem("a `for` without a variable name".to_owned());
                }
                self.separators(false);
                let words = if self.keyword().as_deref() == Some("in") {
                    self.i += 2;
                    let mut list = Vec::new();
                    loop {
                        self.blanks();
                        match self.peek() {
                            Some(';' | '\n') | None => break,
                            _ => match self.word() {
                                Some(w) => list.push(w),
                                None => break,
                            },
                        }
                    }
                    Some(list)
                } else {
                    None
                };
                self.separators(true);
                let body = self.loop_body();
                let redirects = self.redirects_after();
                return Some(Command::For {
                    var,
                    words,
                    body,
                    redirects,
                });
            }
            Some("case") => {
                self.i += 4;
                self.blanks();
                let word = self.word().unwrap_or_default();
                self.separators(false);
                if !self.expect_keyword("in") {
                    return Some(Command::Case {
                        word,
                        arms: Vec::new(),
                        redirects: Vec::new(),
                    });
                }
                let mut arms = Vec::new();
                loop {
                    self.separators(false);
                    if self.keyword().as_deref() == Some("esac") || self.peek().is_none() {
                        break;
                    }
                    if self.peek() == Some('(') {
                        self.i += 1;
                    }
                    let mut patterns = Vec::new();
                    loop {
                        self.blanks();
                        if let Some(w) = self.word() {
                            patterns.push(w);
                        }
                        self.blanks();
                        match self.peek() {
                            Some('|') => self.i += 1,
                            Some(')') => {
                                self.i += 1;
                                break;
                            }
                            _ => {
                                self.problem("a `case` pattern without `)`".to_owned());
                                break;
                            }
                        }
                    }
                    let body = self.list(&["esac"]);
                    arms.push((patterns, body));
                    self.blanks();
                    if self.at(";;&") {
                        self.i += 3;
                    } else if self.at(";;") || self.at(";&") {
                        self.i += 2;
                    } else {
                        break;
                    }
                }
                self.expect_keyword("esac");
                let redirects = self.redirects_after();
                return Some(Command::Case {
                    word,
                    arms,
                    redirects,
                });
            }
            Some("function") => {
                self.i += 8;
                self.blanks();
                let name = self.word().and_then(|w| w.literal()).unwrap_or_default();
                self.blanks();
                if self.at("()") {
                    self.i += 2;
                }
                self.separators(false);
                let body = self.command(&[])?;
                return Some(Command::Function {
                    name,
                    body: Box::new(body),
                });
            }
            Some("[[") => {
                // A test: its words up to `]]`, read as literal text (comparisons, not redirections).
                let start = self.i;
                let end = (self.i..self.s.len().saturating_sub(1))
                    .find(|&k| self.s[k] == ']' && self.s[k + 1] == ']')
                    .unwrap_or(self.s.len());
                let text: String = self.s[start..end].iter().collect();
                self.i = (end + 2).min(self.s.len());
                if end == self.s.len() {
                    self.problem("a `[[` that never closes".to_owned());
                }
                let mut simple = Simple::default();
                for w in text.split_whitespace() {
                    simple.words.push(Word(vec![Part::Quoted(w.to_owned())]));
                }
                // Substitutions inside are commands too.
                if text.contains("$(") || text.contains('`') {
                    let (w, _) = parse_double_quoted_at(&text, self.depth + 1);
                    simple.words.push(w);
                }
                return Some(Command::Simple(simple));
            }
            _ => {}
        }
        let simple = self.simple()?;
        // `name() { … }`
        if simple.words.len() == 1 && simple.assigns.is_empty() && simple.redirects.is_empty() {
            self.blanks();
            if self.at("()") {
                let name = simple.words[0].literal().unwrap_or_default();
                self.i += 2;
                self.separators(false);
                let body = self.command(&[])?;
                return Some(Command::Function {
                    name,
                    body: Box::new(body),
                });
            }
        }
        Some(Command::Simple(simple))
    }

    fn loop_body(&mut self) -> Script {
        if self.keyword().as_deref() == Some("{") {
            // `for x in a; { …; }`
            self.i += 1;
            let body = self.list(&["}"]);
            self.expect_keyword("}");
            return body;
        }
        if !self.expect_keyword("do") {
            return Script::default();
        }
        let body = self.list(&["done"]);
        self.expect_keyword("done");
        body
    }

    /// From `((` to the matching `))`: the text between.
    fn until_double_paren(&mut self) -> String {
        self.i += 2;
        let start = self.i;
        let mut depth = 0_usize;
        while let Some(c) = self.peek() {
            if c == '(' {
                depth += 1;
            } else if c == ')' {
                if depth == 0 && self.peek_at(1) == Some(')') {
                    let text: String = self.s[start..self.i].iter().collect();
                    self.i += 2;
                    return text;
                }
                depth = depth.saturating_sub(1);
            }
            self.i += 1;
        }
        self.problem("a `((` that never closes".to_owned());
        self.s[start..].iter().collect()
    }

    fn simple(&mut self) -> Option<Simple> {
        let mut s = Simple::default();
        loop {
            self.blanks();
            match self.peek() {
                Some('&') if self.peek_at(1) == Some('>') => {}
                None | Some('\n' | ';' | '&' | '|' | ')') => break,
                Some('(') => {
                    if (s.words.is_empty() && s.assigns.is_empty()) || (s.words.len() == 1 && self.at("()")) {
                        break;
                    }
                    // `cmd (x)` is a syntax error in sh; read the rest as a subshell's words.
                    self.problem("unexpected `(`".to_owned());
                    self.i += 1;
                    continue;
                }
                _ => {}
            }
            match self.redirect() {
                Some(Ok(r)) => {
                    s.redirects.push(r);
                    continue;
                }
                Some(Err(doc)) => {
                    s.heredocs.push(doc);
                    continue;
                }
                None => {}
            }
            if s.words.is_empty() && s.assigns.is_empty() && self.at_terminator(KEYWORDS) {
                break;
            }
            let Some(w) = self.word() else {
                break;
            };
            if s.words.is_empty()
                && let Some((name, value)) = assignment(&w)
            {
                s.assigns.push((name, value));
                continue;
            }
            s.words.push(w);
        }
        (!s.words.is_empty() || !s.assigns.is_empty() || !s.redirects.is_empty() || !s.heredocs.is_empty()).then_some(s)
    }

    /// A redirection at the cursor: `Ok` a redirection, `Err` a heredoc or here-string, `None` when there is none.
    fn redirect(&mut self) -> Option<Result<Redirect, Heredoc>> {
        let start = self.i;
        let mut fd = None;
        let mut k = 0;
        while self.peek_at(k).is_some_and(|c| c.is_ascii_digit()) {
            k += 1;
        }
        if k > 0 && matches!(self.peek_at(k), Some('<' | '>')) {
            let digits: String = self.s[self.i..self.i + k].iter().collect();
            fd = digits.parse().ok();
            self.i += k;
        } else if self.peek() == Some('{') {
            // `{fd}>file`
            let close = self.s[self.i..].iter().position(|c| *c == '}');
            if let Some(c) = close
                && matches!(self.peek_at(c + 1), Some('<' | '>'))
                && self.s[self.i + 1..self.i + c].iter().all(|ch| name_char(*ch))
            {
                self.i += c + 1;
            }
        }
        let op = if self.at("&>>") {
            self.i += 3;
            RedirOp::Append
        } else if self.at("&>") {
            self.i += 2;
            RedirOp::Out
        } else if self.at("<<<") {
            self.i += 3;
            self.blanks();
            let word = self.word().unwrap_or_default();
            return Some(Err(Heredoc {
                body: String::new(),
                expand: true,
                word: Some(word),
                slot: usize::MAX,
            }));
        } else if self.at("<<") {
            self.i += 2;
            let tabs = self.peek() == Some('-');
            if tabs {
                self.i += 1;
            }
            self.blanks();
            let (delimiter, quoted) = self.delimiter();
            if delimiter.is_empty() {
                self.problem("a heredoc without a delimiter".to_owned());
            }
            let slot = self.bodies.len();
            self.bodies.push(None);
            self.pending.push(Pending {
                delimiter,
                tabs,
                slot,
            });
            return Some(Err(Heredoc {
                body: String::new(),
                expand: !quoted,
                word: None,
                slot,
            }));
        } else if self.at(">>") {
            self.i += 2;
            RedirOp::Append
        } else if self.at(">&") || self.at("<&") {
            self.i += 2;
            // `>&2`, `>&-`; `>&file` (bash) writes a file.
            if self.peek().is_some_and(|c| c.is_ascii_digit() || c == '-') {
                let mut t = String::new();
                while let Some(c) = self.peek().filter(|c| c.is_ascii_digit() || *c == '-') {
                    t.push(c);
                    self.i += 1;
                }
                return Some(Ok(Redirect {
                    fd,
                    op: RedirOp::Dup,
                    target: Word(vec![Part::Lit(t)]),
                }));
            }
            RedirOp::Out
        } else if self.at("<>") {
            self.i += 2;
            RedirOp::ReadWrite
        } else if self.at(">|") {
            self.i += 2;
            RedirOp::Out
        } else if self.peek() == Some('>') && self.peek_at(1) != Some('(') {
            self.i += 1;
            RedirOp::Out
        } else if self.peek() == Some('<') && self.peek_at(1) != Some('(') {
            self.i += 1;
            RedirOp::In
        } else {
            self.i = start;
            return None;
        };
        self.blanks();
        let target = self.word().unwrap_or_else(|| {
            self.problem("a redirection without a target".to_owned());
            Word::default()
        });
        Some(Ok(Redirect {
            fd,
            op,
            target,
        }))
    }

    /// A heredoc's delimiter, its quotes removed, and whether any part was quoted.
    fn delimiter(&mut self) -> (String, bool) {
        let mut d = String::new();
        let mut quoted = false;
        while let Some(c) = self.peek() {
            match c {
                '\'' | '"' => {
                    quoted = true;
                    self.i += 1;
                    while let Some(q) = self.peek() {
                        self.i += 1;
                        if q == c {
                            break;
                        }
                        d.push(q);
                    }
                }
                '\\' => {
                    quoted = true;
                    self.i += 1;
                    if let Some(n) = self.peek() {
                        d.push(n);
                        self.i += 1;
                    }
                }
                c if c.is_whitespace() || matches!(c, ';' | '|' | '&' | '<' | '>' | '(' | ')') => break,
                c => {
                    d.push(c);
                    self.i += 1;
                }
            }
        }
        (d, quoted)
    }

    /// A word at the cursor, or `None` when a metacharacter (or the end) is there.
    fn word(&mut self) -> Option<Word> {
        let mut parts: Vec<Part> = Vec::new();
        let start = self.i;
        if self.peek() == Some('~') {
            self.i += 1;
            let mut user = String::new();
            while let Some(c) = self.peek().filter(|c| name_char(*c) || *c == '-' || *c == '.') {
                user.push(c);
                self.i += 1;
            }
            if self.peek().is_none_or(|c| c == '/' || c == ':' || is_meta(c)) {
                parts.push(Part::Tilde((!user.is_empty()).then_some(user)));
            } else {
                self.i = start;
            }
        }
        self.word_parts(&mut parts, false);
        (self.i > start).then(|| Word(merge(parts)))
    }

    /// Reads word parts up to a metacharacter (or, `in_brace`, up to a `,` or `}` at this level).
    #[allow(clippy::too_many_lines, reason = "one case per character that means something")]
    fn word_parts(&mut self, parts: &mut Vec<Part>, in_brace: bool) {
        while let Some(c) = self.peek() {
            if is_meta(c) {
                // `<(…)`/`>(…)`: process substitution, part of a word.
                if matches!(c, '<' | '>') && self.peek_at(1) == Some('(') && !parts.is_empty() {
                    break;
                }
                if matches!(c, '<' | '>') && self.peek_at(1) == Some('(') {
                    self.i += 2;
                    let script = self.sub_until_paren();
                    parts.push(Part::ProcSub(script));
                    continue;
                }
                // `a=(x y)`: an array assignment, its words kept as text.
                if c == '(' && matches!(parts.last(), Some(Part::Lit(t)) if t.ends_with('=')) {
                    let mut depth = 0_usize;
                    let start = self.i;
                    while let Some(ch) = self.peek() {
                        self.i += 1;
                        if ch == '(' {
                            depth += 1;
                        } else if ch == ')' {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                    }
                    let text: String = self.s[start..self.i].iter().collect();
                    if text.contains("$(") || text.contains('`') || text.contains("<(") || text.contains(">(") {
                        self.problem("an array whose words run commands".to_owned());
                    }
                    parts.push(Part::Quoted(text));
                    continue;
                }
                break;
            }
            if in_brace && matches!(c, ',' | '}') {
                break;
            }
            match c {
                '\'' => {
                    self.i += 1;
                    let mut t = String::new();
                    let mut closed = false;
                    while let Some(q) = self.peek() {
                        self.i += 1;
                        if q == '\'' {
                            closed = true;
                            break;
                        }
                        t.push(q);
                    }
                    if !closed {
                        self.problem("a `'` that never closes".to_owned());
                    }
                    parts.push(Part::Quoted(t));
                }
                '"' => {
                    self.i += 1;
                    self.double_quoted_until(parts, Some('"'));
                }
                '\\' => {
                    self.i += 1;
                    match self.peek() {
                        Some('\n') => self.i += 1,
                        Some(n) => {
                            self.i += 1;
                            parts.push(Part::Quoted(n.to_string()));
                        }
                        None => {}
                    }
                }
                '$' => {
                    self.dollar(parts, false);
                    // Outside quotes a variable's value is split into words.
                    if let Some(Part::Var(name)) = parts.last()
                        && name != "@"
                        && name != "*"
                    {
                        let name = name.clone();
                        parts.pop();
                        parts.push(Part::Split(name));
                    }
                }
                '`' => {
                    self.i += 1;
                    let script = self.backticks();
                    parts.push(Part::CmdSub(script));
                }
                '*' | '?' => {
                    self.i += 1;
                    parts.push(Part::Glob(c.to_string()));
                }
                '[' => {
                    // A bracket expression when a `]` closes it in the same word.
                    let close = (self.i + 1..self.s.len())
                        .take_while(|&k| !is_meta(self.s[k]))
                        .find(|&k| self.s[k] == ']' && k > self.i + 1);
                    if let Some(end) = close {
                        let t: String = self.s[self.i..=end].iter().collect();
                        self.i = end + 1;
                        parts.push(Part::Glob(t));
                    } else {
                        self.i += 1;
                        parts.push(Part::Lit("[".to_owned()));
                    }
                }
                '{' => {
                    if let Some(alts) = self.brace() {
                        parts.push(Part::Brace(alts));
                    } else {
                        self.i += 1;
                        parts.push(Part::Lit("{".to_owned()));
                    }
                }
                c => {
                    self.i += 1;
                    match parts.last_mut() {
                        Some(Part::Lit(t)) => t.push(c),
                        _ => parts.push(Part::Lit(c.to_string())),
                    }
                }
            }
        }
    }

    /// A brace expansion at `{`: its alternatives (`{a,b}`, `{1..3}`), or `None` (the cursor unmoved) when it is
    /// not one (`{}`, `{x}`, `${`).
    fn brace(&mut self) -> Option<Vec<Word>> {
        self.braces += 1;
        if self.braces > MAX_BRACES || self.depth > MAX_DEPTH {
            self.problem("brace expansions too many or too deep to read".to_owned());
            return None;
        }
        let start = self.i;
        // A range: `{1..5}`, `{a..e}`.
        let close = (self.i + 1..self.s.len()).take_while(|&k| !is_meta(self.s[k])).find(|&k| self.s[k] == '}');
        if let Some(end) = close {
            let inner: String = self.s[self.i + 1..end].iter().collect();
            // `{1..10}`, `{1..10..2}`: `range` reads the step.
            if let Some((a, b)) = inner.split_once("..") {
                let alts = range(a, b)?;
                self.i = end + 1;
                return Some(alts.into_iter().map(|s| Word(vec![Part::Lit(s)])).collect());
            }
        }
        self.i += 1;
        let mut alts = Vec::new();
        loop {
            let mut parts = Vec::new();
            self.word_parts(&mut parts, true);
            alts.push(Word(merge(parts)));
            match self.peek() {
                Some(',') => self.i += 1,
                Some('}') if alts.len() > 1 => {
                    self.i += 1;
                    return Some(alts);
                }
                _ => {
                    self.i = start;
                    return None;
                }
            }
        }
    }

    /// Double-quoted text up to `end` (`None`: to the end of the input, as a heredoc body).
    fn double_quoted_until(&mut self, parts: &mut Vec<Part>, end: Option<char>) {
        let mut text = String::new();
        let flush = |text: &mut String, parts: &mut Vec<Part>| {
            if !text.is_empty() {
                parts.push(Part::Quoted(std::mem::take(text)));
            }
        };
        loop {
            let Some(c) = self.peek() else {
                if end.is_some() {
                    self.problem("a `\"` that never closes".to_owned());
                }
                break;
            };
            if Some(c) == end {
                self.i += 1;
                break;
            }
            match c {
                '\\' => {
                    self.i += 1;
                    match self.peek() {
                        Some(n @ ('"' | '\\' | '$' | '`')) => {
                            text.push(n);
                            self.i += 1;
                        }
                        Some('\n') => self.i += 1,
                        _ => text.push('\\'),
                    }
                }
                '$' => {
                    flush(&mut text, parts);
                    let before = parts.len();
                    self.dollar(parts, true);
                    if parts.len() == before {
                        text.push('$');
                    }
                }
                '`' => {
                    flush(&mut text, parts);
                    self.i += 1;
                    let script = self.backticks();
                    parts.push(Part::CmdSub(script));
                }
                c => {
                    text.push(c);
                    self.i += 1;
                }
            }
        }
        flush(&mut text, parts);
        if parts.is_empty() {
            // `""`: an empty word, still a word.
            parts.push(Part::Quoted(String::new()));
        }
    }

    /// At `$`: a variable, a parameter expansion, a substitution, or a literal `$`.
    fn dollar(&mut self, parts: &mut Vec<Part>, quoted: bool) {
        let next = self.peek_at(1);
        match next {
            Some('(') if self.peek_at(2) == Some('(') => {
                self.i += 1;
                let text = self.until_double_paren();
                parts.push(Part::Arith(text));
            }
            Some('(') => {
                self.i += 2;
                let script = self.sub_until_paren();
                parts.push(Part::CmdSub(script));
            }
            Some('{') => {
                self.i += 2;
                let start = self.i;
                let mut depth = 0_usize;
                let mut closed = false;
                while let Some(c) = self.peek() {
                    if c == '{' {
                        depth += 1;
                    } else if c == '}' {
                        if depth == 0 {
                            closed = true;
                            break;
                        }
                        depth -= 1;
                    }
                    self.i += 1;
                }
                let inner: String = self.s[start..self.i].iter().collect();
                if closed {
                    self.i += 1;
                } else {
                    self.problem("a `${` that never closes".to_owned());
                }
                parts.push(param(&inner, self.depth + 1));
            }
            Some('\'') if !quoted => {
                // `$'…'`: ANSI-C quoting, decoded.
                self.i += 2;
                let mut out = String::new();
                let mut closed = false;
                while let Some(c) = self.peek() {
                    self.i += 1;
                    match c {
                        '\'' => {
                            closed = true;
                            break;
                        }
                        '\\' => out.push_str(&self.ansi_escape()),
                        c => out.push(c),
                    }
                }
                if !closed {
                    self.problem("a `$'` that never closes".to_owned());
                }
                parts.push(Part::Quoted(out));
            }
            Some('"') if !quoted => {
                // `$"…"`: a translated string, read as double quotes.
                self.i += 2;
                self.double_quoted_until(parts, Some('"'));
            }
            Some(c) if c.is_ascii_alphabetic() || c == '_' => {
                self.i += 1;
                let mut name = String::new();
                while let Some(c) = self.peek().filter(|c| name_char(*c)) {
                    name.push(c);
                    self.i += 1;
                }
                parts.push(Part::Var(name));
            }
            Some(c) if c.is_ascii_digit() || matches!(c, '@' | '*' | '#' | '?' | '$' | '!' | '-') => {
                self.i += 2;
                parts.push(Part::Var(c.to_string()));
            }
            _ => {
                self.i += 1;
                if quoted {
                    return;
                }
                match parts.last_mut() {
                    Some(Part::Lit(t)) => t.push('$'),
                    _ => parts.push(Part::Lit("$".to_owned())),
                }
            }
        }
    }

    /// After `\` in `$'…'`.
    fn ansi_escape(&mut self) -> String {
        let Some(c) = self.peek() else {
            return String::new();
        };
        self.i += 1;
        let hex = |p: &mut Self, max: usize| {
            let mut v = 0_u32;
            let mut n = 0;
            while n < max {
                match p.peek().and_then(|c| c.to_digit(16)) {
                    Some(d) => {
                        v = v * 16 + d;
                        p.i += 1;
                        n += 1;
                    }
                    None => break,
                }
            }
            char::from_u32(v).map(String::from).unwrap_or_default()
        };
        match c {
            'n' => "\n".to_owned(),
            't' => "\t".to_owned(),
            'r' => "\r".to_owned(),
            'a' => "\x07".to_owned(),
            'b' => "\x08".to_owned(),
            'e' | 'E' => "\x1b".to_owned(),
            'f' => "\x0c".to_owned(),
            'v' => "\x0b".to_owned(),
            'x' => hex(self, 2),
            'u' => hex(self, 4),
            'U' => hex(self, 8),
            '0'..='7' => {
                let mut v = c.to_digit(8).unwrap_or(0);
                let mut n = 1;
                while n < 3 {
                    match self.peek().and_then(|c| c.to_digit(8)) {
                        Some(d) => {
                            v = v * 8 + d;
                            self.i += 1;
                            n += 1;
                        }
                        None => break,
                    }
                }
                char::from_u32(v).map(String::from).unwrap_or_default()
            }
            c => c.to_string(),
        }
    }

    /// The commands of a `$(…)` or `<(…)` whose `(` was just read, up to its `)`.
    fn sub_until_paren(&mut self) -> Script {
        let saved = std::mem::take(&mut self.pending);
        let script = self.list(&[]);
        if self.peek() == Some(')') {
            self.i += 1;
        } else {
            self.problem("a `$(` that never closes".to_owned());
        }
        self.pending.extend(saved);
        script
    }

    /// The commands between backticks whose opening one was just read.
    fn backticks(&mut self) -> Script {
        let mut text = String::new();
        let mut closed = false;
        while let Some(c) = self.peek() {
            self.i += 1;
            match c {
                '`' => {
                    closed = true;
                    break;
                }
                '\\' if matches!(self.peek(), Some('`' | '\\' | '$')) => {
                    text.push(self.s[self.i]);
                    self.i += 1;
                }
                c => text.push(c),
            }
        }
        if !closed {
            self.problem("a backtick that never closes".to_owned());
        }
        let inner = parse_at(&text, self.depth + 1);
        for p in inner.problems {
            self.problem(p);
        }
        inner.script
    }
}

fn is_meta(c: char) -> bool {
    c.is_whitespace() || matches!(c, ';' | '&' | '|' | '<' | '>' | '(' | ')')
}

/// Adjacent literal parts merged.
fn merge(parts: Vec<Part>) -> Vec<Part> {
    let mut out: Vec<Part> = Vec::new();
    for p in parts {
        match (out.last_mut(), p) {
            (Some(Part::Lit(a)), Part::Lit(b)) => a.push_str(&b),
            (_, p) => out.push(p),
        }
    }
    out
}

/// `{a..e}`, `{1..10}`, `{1..10..2}` (at most 256 values).
#[allow(clippy::many_single_char_names, reason = "the two ends and their characters, as the shell names them")]
fn range(a: &str, b: &str) -> Option<Vec<String>> {
    let (b, step) = match b.split_once("..") {
        Some((b, s)) => (b, s.parse::<i64>().ok()?.unsigned_abs().max(1)),
        None => (b, 1),
    };
    if let (Ok(x), Ok(y)) = (a.parse::<i64>(), b.parse::<i64>()) {
        let n = x.abs_diff(y) / step + 1;
        if n > 256 {
            return None;
        }
        let step = i64::try_from(step).ok()?;
        let dir = if y >= x {
            step
        } else {
            -step
        };
        return (0..n)
            .map(|k| {
                i64::try_from(k)
                    .ok()
                    .and_then(|k| dir.checked_mul(k))
                    .and_then(|d| x.checked_add(d))
                    .map(|v| v.to_string())
            })
            .collect();
    }
    let (mut x, mut y) = (a.chars(), b.chars());
    let (Some(x), None, Some(y), None) = (x.next(), x.next(), y.next(), y.next()) else {
        return None;
    };
    if !x.is_ascii_alphabetic() || !y.is_ascii_alphabetic() {
        return None;
    }
    let (lo, hi) = (x.min(y) as u8, x.max(y) as u8);
    let mut v: Vec<String> = (lo..=hi).map(|c| (c as char).to_string()).collect();
    if x > y {
        v.reverse();
    }
    Some(v)
}

/// The inside of `${…}`.
fn param(inner: &str, depth: usize) -> Part {
    let name_end = inner.find(|c: char| !name_char(c)).unwrap_or(inner.len());
    let (name, rest) = inner.split_at(name_end);
    if name.is_empty() {
        return Part::Param(inner.to_owned());
    }
    if rest.is_empty() {
        return Part::Var(name.to_owned());
    }
    for op in [":-", ":=", "-", "="] {
        if let Some(word) = rest.strip_prefix(op) {
            let (w, _) = parse_double_quoted_at(word, depth);
            return Part::Default(name.to_owned(), w);
        }
    }
    // Any other operation whose word runs commands: walked, its value unknown.
    if rest.contains("$(") || rest.contains('`') || rest.contains("<(") {
        let word = rest.trim_start_matches([':', '+', '?', '%', '#', '/', '^', ',']);
        let (w, _) = parse_double_quoted_at(word, depth);
        return Part::Op(inner.to_owned(), w);
    }
    Part::Param(inner.to_owned())
}

/// `NAME=value`, `NAME+=value` at the start of a word.
fn assignment(w: &Word) -> Option<(String, Word)> {
    let Some(Part::Lit(first)) = w.0.first() else {
        return None;
    };
    let (name, value) = first.split_once('=')?;
    // `NAME+=value` keeps its `+`: the walker appends.
    if !valid_name(name.strip_suffix('+').unwrap_or(name)) {
        return None;
    }
    let mut parts = Vec::new();
    if !value.is_empty() {
        parts.push(Part::Lit(value.to_owned()));
    }
    parts.extend(w.0[1..].iter().cloned());
    Some((name.to_owned(), Word(parts)))
}

fn fill_script(s: &mut Script, bodies: &[Option<String>]) {
    for a in &mut s.0 {
        for p in &mut a.pipelines {
            for c in &mut p.commands {
                fill_command(c, bodies);
            }
        }
    }
}

fn fill_redirects(rs: &mut [Redirect], bodies: &[Option<String>]) {
    for r in rs {
        fill_word(&mut r.target, bodies);
    }
}

fn fill_command(c: &mut Command, bodies: &[Option<String>]) {
    match c {
        Command::Simple(s) => {
            for (_, w) in &mut s.assigns {
                fill_word(w, bodies);
            }
            for w in &mut s.words {
                fill_word(w, bodies);
            }
            fill_redirects(&mut s.redirects, bodies);
            for d in &mut s.heredocs {
                if let Some(Some(body)) = bodies.get(d.slot) {
                    d.body.clone_from(body);
                }
                if let Some(w) = &mut d.word {
                    fill_word(w, bodies);
                }
            }
        }
        Command::Subshell(s, r) | Command::Group(s, r) => {
            fill_script(s, bodies);
            fill_redirects(r, bodies);
        }
        Command::If {
            branches,
            otherwise,
            redirects,
        } => {
            for (a, b) in branches {
                fill_script(a, bodies);
                fill_script(b, bodies);
            }
            if let Some(o) = otherwise {
                fill_script(o, bodies);
            }
            fill_redirects(redirects, bodies);
        }
        Command::For {
            words,
            body,
            redirects,
            ..
        } => {
            for w in words.iter_mut().flatten() {
                fill_word(w, bodies);
            }
            fill_script(body, bodies);
            fill_redirects(redirects, bodies);
        }
        Command::While {
            cond,
            body,
            redirects,
        } => {
            fill_script(cond, bodies);
            fill_script(body, bodies);
            fill_redirects(redirects, bodies);
        }
        Command::Case {
            word,
            arms,
            redirects,
        } => {
            fill_word(word, bodies);
            for (ps, b) in arms {
                for p in ps {
                    fill_word(p, bodies);
                }
                fill_script(b, bodies);
            }
            fill_redirects(redirects, bodies);
        }
        Command::Function {
            body,
            ..
        } => fill_command(body, bodies),
    }
}

fn fill_word(w: &mut Word, bodies: &[Option<String>]) {
    for p in &mut w.0 {
        match p {
            Part::CmdSub(s) | Part::ProcSub(s) => fill_script(s, bodies),
            Part::Default(_, w) | Part::Op(_, w) => fill_word(w, bodies),
            Part::Brace(alts) => {
                for a in alts {
                    fill_word(a, bodies);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn simples(script: &Script) -> Vec<Simple> {
        fn walk(s: &Script, out: &mut Vec<Simple>) {
            for a in &s.0 {
                for p in &a.pipelines {
                    for c in &p.commands {
                        cmd(c, out);
                    }
                }
            }
        }
        fn cmd(c: &Command, out: &mut Vec<Simple>) {
            match c {
                Command::Simple(s) => out.push(s.clone()),
                Command::Subshell(s, _) | Command::Group(s, _) => walk(s, out),
                Command::If {
                    branches,
                    otherwise,
                    ..
                } => {
                    for (a, b) in branches {
                        walk(a, out);
                        walk(b, out);
                    }
                    if let Some(o) = otherwise {
                        walk(o, out);
                    }
                }
                Command::For {
                    body,
                    ..
                } => walk(body, out),
                Command::While {
                    cond,
                    body,
                    ..
                } => {
                    walk(cond, out);
                    walk(body, out);
                }
                Command::Case {
                    arms,
                    ..
                } => {
                    for (_, b) in arms {
                        walk(b, out);
                    }
                }
                Command::Function {
                    body,
                    ..
                } => cmd(body, out),
            }
        }
        let mut out = Vec::new();
        walk(script, &mut out);
        out
    }

    fn words(text: &str) -> Vec<Vec<String>> {
        let p = parse(text);
        simples(&p.script).iter().map(|s| s.words.iter().map(Word::display).collect()).collect()
    }

    #[test]
    fn parses_lists_pipes_and_redirections() {
        assert_eq!(
            words("cd app && cargo test -q 2>&1 | tail -3; ls"),
            vec![vec!["cd", "app"], vec!["cargo", "test", "-q"], vec!["tail", "-3"], vec!["ls"]]
        );
        let p = parse("echo hi > out.txt 2>>err.log < in &> all");
        assert!(p.complete);
        let s = &simples(&p.script)[0];
        let ops: Vec<(Option<u32>, RedirOp, String)> =
            s.redirects.iter().map(|r| (r.fd, r.op, r.target.display())).collect();
        assert_eq!(
            ops,
            vec![
                (None, RedirOp::Out, "out.txt".to_owned()),
                (Some(2), RedirOp::Append, "err.log".to_owned()),
                (None, RedirOp::In, "in".to_owned()),
                (None, RedirOp::Out, "all".to_owned()),
            ]
        );
        let s = &simples(&parse("A=1 B='x y' make").script)[0];
        assert_eq!(s.assigns.len(), 2);
        assert_eq!(s.assigns[1].1.literal().as_deref(), Some("x y"));
    }

    #[test]
    fn heredocs_and_here_strings_keep_their_bodies() {
        let p = parse("cd /a && python3 - <<'EOF'\np='x.rs'\nopen(p,'w').write(\"$HOME\")\nEOF\necho done");
        assert!(p.complete, "{:?}", p.problems);
        let all = simples(&p.script);
        assert_eq!(all.len(), 3);
        assert_eq!(all[1].heredocs[0].body, "p='x.rs'\nopen(p,'w').write(\"$HOME\")\n");
        assert!(!all[1].heredocs[0].expand);
        assert_eq!(all[2].words[0].literal().as_deref(), Some("echo"));
        let p = parse("cat > f <<EOF\n$(rm -rf ~)\nEOF");
        assert!(simples(&p.script)[0].heredocs[0].expand);
        let p = parse("bash <<< 'rm -rf x'");
        assert_eq!(
            simples(&p.script)[0].heredocs[0].word.as_ref().and_then(Word::literal).as_deref(),
            Some("rm -rf x")
        );
        assert!(!parse("cat <<EOF\nnever ends").complete);
        let p = parse("a <<A; b <<-B\none\nA\n\ttwo\n\tB\nc");
        let all = simples(&p.script);
        assert_eq!((all[0].heredocs[0].body.as_str(), all[1].heredocs[0].body.as_str()), ("one\n", "two\n"));
        assert_eq!(all[2].words[0].literal().as_deref(), Some("c"));
    }

    #[test]
    fn loops_ifs_functions() {
        let p = parse("for c in a b; do d=/tmp/x-$c; rm -rf $d; done");
        assert!(p.complete);
        let Command::For {
            var,
            words,
            ..
        } = &p.script.0[0].pipelines[0].commands[0]
        else {
            panic!("{p:?}")
        };
        assert_eq!(var, "c");
        assert_eq!(words.as_ref().map(Vec::len), Some(2));
        assert_eq!(words_of(&p), vec![vec!["rm", "-rf", "$d"]]);
        let p = parse("if [ -d x ]; then rm -r x; elif true; then :; else echo no; fi");
        assert!(p.complete, "{:?}", p.problems);
        assert_eq!(simples(&p.script).len(), 5);
        let p = parse("f() { local a=$1; cp $2 \"$S/$3\"; }; f 1 2 3");
        assert!(p.complete, "{:?}", p.problems);
        assert!(matches!(p.script.0[0].pipelines[0].commands[0], Command::Function { .. }));
        let p = parse("while read -r l; do echo $l; done < list.txt");
        assert!(p.complete);
        let p = parse("case $x in a|b) rm -rf a ;; *) echo ;; esac");
        assert!(p.complete, "{:?}", p.problems);
        assert_eq!(simples(&p.script).len(), 2);
        assert!(parse("[[ -n $x && $y > 2 ]] && echo ok").complete);
        assert!(parse("(( i++ )); for ((i=0;i<3;i++)); do echo $i; done").complete);
    }

    fn words_of(p: &Parsed) -> Vec<Vec<String>> {
        simples(&p.script)
            .iter()
            .filter(|s| !s.words.is_empty())
            .map(|s| s.words.iter().map(Word::display).collect())
            .collect()
    }

    #[test]
    fn substitutions_nest() {
        let p = parse("T=$(mktemp -d -p ~/x) && echo \"$(date) `whoami`\" ${HOME:-/h} $((1+2)) <(ls)");
        assert!(p.complete, "{:?}", p.problems);
        let s = simples(&p.script);
        let Part::CmdSub(inner) = &s[0].assigns[0].1.0[0] else {
            panic!("{:?}", s[0])
        };
        assert_eq!(
            words_of(&Parsed {
                script: inner.clone(),
                complete: true,
                problems: vec![]
            }),
            vec![vec!["mktemp", "-d", "-p", "~/x"]]
        );
        assert!(matches!(s[1].words[2].0[0], Part::Default(..)));
        assert!(matches!(s[1].words[3].0[0], Part::Arith(_)));
        assert!(matches!(s[1].words[4].0[0], Part::ProcSub(_)));
        let p = parse("mkdir -p $T/{home,config} x{1..3}");
        let s = simples(&p.script);
        assert!(s[0].words[2].0.iter().any(|p| matches!(p, Part::Brace(a) if a.len() == 2)));
        assert!(matches!(&s[0].words[3].0[1], Part::Brace(a) if a.len() == 3));
        assert_eq!(words("$'\\x72m' -rf ~")[0][0], "rm", "ANSI-C quoting decoded");
        assert_eq!(words("find . -exec rm {} +")[0], vec!["find", ".", "-exec", "rm", "{}", "+"]);
    }

    #[test]
    fn incomplete_input_is_flagged() {
        for text in ["echo 'x", "echo \"x", "echo $(ls", "echo `ls", "if true; then ls", "for x in a; do ls", "(ls"] {
            assert!(!parse(text).complete, "{text}");
        }
        assert!(parse("ls # don't").complete);
        assert!(parse("echo a\\\nb").complete);
    }

    proptest::proptest! {
        /// Any text: no panic, and what is read is read in time.
        #[test]
        fn never_panics(text in r"[ -~\n\t]{0,200}") {
            let _parsed: Parsed = parse(&text);
        }

        #[test]
        fn never_panics_on_shell_shapes(text in r"((\$\(|\(|\)|`|'|\x22|<<-?|<<<|EOF|\$\{|\}|\{|;;|\||&|\n|for|do|done|if|fi|case|esac|in| |a|=|\*|\[|\]|,|\.\.|\\)){0,60}") {
            let _parsed: Parsed = parse(&text);
        }
    }
}
