//! Inline code (`python3 - <<EOF`, `python3 -c`, `node -e`, `ruby -e`, `perl -e`, a script the line writes): what it
//! does to files, which programs it runs and which hosts it talks to (01 §2.2).
//!
//! The code is read as tokens, never run. String literals are values, not effects: `s.replace("~/.ssh/id_rsa", x)`
//! mentions a path, it doesn't read it. A literal becomes an effect only as the argument of a call that touches a file,
//! runs a program or opens a connection. Simple constants are followed (`p = 'x.rs'`, `Path('a') / 'b'`,
//! `os.path.join(…)`, `for p in ['a', 'b']:`, a function's parameter from its calls); anything else is an unknown value
//! and lowers the certainty.

use std::collections::BTreeMap;

use super::model::{Certainty, Channel, CodeSource, Effect, NetMethod};
use super::shell::{Exp, UNKNOWN, Walk, unknown};
use super::workspace::normalize;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    Python,
    Node,
    Ruby,
    Perl,
    Shell,
}

/// The language an interpreter runs.
#[must_use]
pub fn interpreter(program: &str) -> Option<Lang> {
    let p = program.to_ascii_lowercase();
    if p.starts_with("python") || p.starts_with("pypy") || p == "py" {
        Some(Lang::Python)
    } else if matches!(p.as_str(), "node" | "nodejs" | "deno" | "ts-node" | "tsx") {
        Some(Lang::Node)
    } else if p == "ruby" || p == "irb" {
        Some(Lang::Ruby)
    } else if p.starts_with("perl") {
        Some(Lang::Perl)
    } else {
        None
    }
}

/// The language of a script a line wrote, from its name or its `#!` line.
#[must_use]
pub fn lang_of_script(path: &str, text: &str) -> Lang {
    let first = text.lines().next().unwrap_or_default();
    if let Some(shebang) = first.strip_prefix("#!") {
        let words: Vec<&str> = shebang.split_whitespace().collect();
        let program = match words.as_slice() {
            [env, p, ..] if env.ends_with("/env") => (*p).to_owned(),
            [p, ..] => crate::shell::program_name(p),
            [] => String::new(),
        };
        if let Some(l) = interpreter(&program) {
            return l;
        }
    }
    let ext = path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "py" => Lang::Python,
        "js" | "mjs" | "cjs" | "ts" => Lang::Node,
        "rb" => Lang::Ruby,
        "pl" => Lang::Perl,
        _ => Lang::Shell,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Tok {
    Ident(String),
    /// A string literal: its value, with [`UNKNOWN`] where an interpolation is.
    Str(String),
    /// Shell text run by the language (Ruby and Perl backticks, `%x(…)`, `qx(…)`).
    Cmd(String),
    Punct(char),
    Num(String),
}

/// The tokens of `code`.
fn lex(lang: Lang, code: &str) -> Vec<Tok> {
    let c: Vec<char> = code.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    let mut depth = 0_usize;
    while i < c.len() {
        let ch = c[i];
        // A newline outside brackets ends a statement.
        if ch == '\n' && depth == 0 {
            if out.last() != Some(&Tok::Punct('\n')) {
                out.push(Tok::Punct('\n'));
            }
            i += 1;
            continue;
        }
        // Regular expression literals (`/…/flags` where a value starts; Perl's `m/…/`, `qr/…/`, `s/…/…/`, `tr/…/…/`):
        // a value, never code, and their `/`, `#` and quotes are not comments or strings.
        if let Some(end) = regex_literal(lang, &c, i, out.last()) {
            out.push(Tok::Str(UNKNOWN.to_string()));
            i = end;
            continue;
        }
        // Perl's `$#array` (the last index) is no comment.
        if ch == '$' && lang == Lang::Perl && c.get(i + 1) == Some(&'#') {
            let mut k = i + 2;
            if c.get(k) == Some(&'{') || c.get(k) == Some(&'$') {
                k += 1;
            }
            while k < c.len() && (c[k].is_ascii_alphanumeric() || c[k] == '_' || c[k] == '}') {
                k += 1;
            }
            out.push(Tok::Num("0".to_owned()));
            i = k;
            continue;
        }
        // Comments.
        if (ch == '#' && matches!(lang, Lang::Python | Lang::Ruby | Lang::Perl))
            || (lang == Lang::Node && ch == '/' && c.get(i + 1) == Some(&'/'))
        {
            while i < c.len() && c[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if lang == Lang::Node && ch == '/' && c.get(i + 1) == Some(&'*') {
            i += 2;
            while i + 1 < c.len() && !(c[i] == '*' && c[i + 1] == '/') {
                i += 1;
            }
            i += 2;
            continue;
        }
        if ch.is_whitespace() {
            i += 1;
            continue;
        }
        // String prefixes (Python r'', f'', b'', rb'').
        let mut prefix = String::new();
        if lang == Lang::Python && ch.is_ascii_alphabetic() {
            let mut k = i;
            while k < c.len() && k - i < 2 && matches!(c[k].to_ascii_lowercase(), 'r' | 'f' | 'b' | 'u') {
                k += 1;
            }
            if k > i && matches!(c.get(k), Some('\'' | '"')) {
                prefix = c[i..k].iter().collect::<String>().to_ascii_lowercase();
                i = k;
            }
        }
        let ch = c[i];
        if matches!(ch, '\'' | '"') || (ch == '`' && lang == Lang::Node) {
            let triple = lang == Lang::Python && c.get(i + 1) == Some(&ch) && c.get(i + 2) == Some(&ch);
            let quote_len = if triple {
                3
            } else {
                1
            };
            let mut k = i + quote_len;
            let mut value = String::new();
            let raw = prefix.contains('r');
            let interpolates =
                prefix.contains('f') || ch == '`' || (ch == '"' && matches!(lang, Lang::Ruby | Lang::Perl));
            loop {
                if k >= c.len() {
                    break;
                }
                if c[k] == ch && (!triple || (c.get(k + 1) == Some(&ch) && c.get(k + 2) == Some(&ch))) {
                    k += quote_len;
                    break;
                }
                if c[k] == '\\' && k + 1 < c.len() {
                    if raw {
                        value.push('\\');
                        value.push(c[k + 1]);
                    } else {
                        value.push(match c[k + 1] {
                            'n' => '\n',
                            't' => '\t',
                            other => other,
                        });
                    }
                    k += 2;
                    continue;
                }
                if interpolates {
                    let opens = match lang {
                        Lang::Python => c[k] == '{' && c.get(k + 1) != Some(&'{'),
                        Lang::Node => c[k] == '$' && c.get(k + 1) == Some(&'{'),
                        Lang::Ruby => c[k] == '#' && c.get(k + 1) == Some(&'{'),
                        Lang::Perl => {
                            (c[k] == '$' || c[k] == '@')
                                && c.get(k + 1).is_some_and(|n| n.is_ascii_alphabetic() || *n == '{')
                        }
                        Lang::Shell => false,
                    };
                    if opens {
                        // Skip the interpolation.
                        if lang == Lang::Perl {
                            k += 1;
                            while k < c.len()
                                && (c[k].is_ascii_alphanumeric() || c[k] == '_' || c[k] == '{' || c[k] == '}')
                            {
                                k += 1;
                            }
                        } else {
                            let mut depth = 0;
                            while k < c.len() {
                                if c[k] == '{' {
                                    depth += 1;
                                } else if c[k] == '}' {
                                    depth -= 1;
                                    if depth == 0 {
                                        k += 1;
                                        break;
                                    }
                                }
                                k += 1;
                            }
                        }
                        value.push(UNKNOWN);
                        continue;
                    }
                    if lang == Lang::Python && c[k] == '{' && c.get(k + 1) == Some(&'{') {
                        value.push('{');
                        k += 2;
                        continue;
                    }
                }
                value.push(c[k]);
                k += 1;
            }
            i = k;
            out.push(Tok::Str(value));
            continue;
        }
        if ch == '`' && matches!(lang, Lang::Ruby | Lang::Perl) {
            let mut k = i + 1;
            let mut value = String::new();
            while k < c.len() && c[k] != '`' {
                value.push(c[k]);
                k += 1;
            }
            out.push(Tok::Cmd(value));
            i = k + 1;
            continue;
        }
        // Perl and Ruby: `qx(…)`, `%x(…)`.
        if matches!(lang, Lang::Ruby | Lang::Perl)
            && (code_at(&c, i, "qx(") || code_at(&c, i, "%x(") || code_at(&c, i, "qx{") || code_at(&c, i, "%x{"))
        {
            let close = if c[i + 2] == '(' {
                ')'
            } else {
                '}'
            };
            let mut k = i + 3;
            let mut value = String::new();
            while k < c.len() && c[k] != close {
                value.push(c[k]);
                k += 1;
            }
            out.push(Tok::Cmd(value));
            i = k + 1;
            continue;
        }
        if ch.is_ascii_alphabetic()
            || ch == '_'
            || (ch == '$' && matches!(lang, Lang::Perl | Lang::Node))
            || (ch == '@' && lang == Lang::Ruby)
        {
            let mut k = i + 1;
            while k < c.len()
                && (c[k].is_ascii_alphanumeric()
                    || c[k] == '_'
                    || (lang == Lang::Ruby && matches!(c[k], '?' | '!') && k == k.max(i + 1)))
            {
                k += 1;
            }
            // Ruby/Perl `::`
            let mut name: String = c[i..k].iter().collect();
            while matches!(lang, Lang::Ruby | Lang::Perl)
                && k + 2 < c.len()
                && c[k] == ':'
                && c[k + 1] == ':'
                && (c[k + 2].is_ascii_alphabetic())
            {
                let mut j = k + 2;
                while j < c.len() && (c[j].is_ascii_alphanumeric() || c[j] == '_') {
                    j += 1;
                }
                name.push('.');
                name.extend(&c[k + 2..j]);
                k = j;
            }
            out.push(Tok::Ident(name));
            i = k;
            continue;
        }
        if ch.is_ascii_digit() {
            let start = i;
            while i < c.len() && (c[i].is_ascii_alphanumeric() || c[i] == '.' || c[i] == '_') {
                i += 1;
            }
            out.push(Tok::Num(c[start..i].iter().collect()));
            continue;
        }
        match ch {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            _ => {}
        }
        out.push(Tok::Punct(ch));
        i += 1;
    }
    out
}

/// A regular expression literal starting at `i`: where it ends.
fn regex_literal(lang: Lang, c: &[char], i: usize, last: Option<&Tok>) -> Option<usize> {
    if !matches!(lang, Lang::Node | Lang::Ruby | Lang::Perl) {
        return None;
    }
    // Perl's quote-like operators with a slash: `m/`, `qr/`, `s/` and `tr/`, `y/` (two parts).
    let mut start = i;
    let mut parts = 1;
    if lang == Lang::Perl
        && c[i].is_ascii_alphabetic()
        && (i == 0 || !(c[i - 1].is_ascii_alphanumeric() || c[i - 1] == '_' || c[i - 1] == '$'))
    {
        let word: String = c[i..].iter().take_while(|ch| ch.is_ascii_alphabetic()).collect();
        if !matches!(word.as_str(), "m" | "qr" | "s" | "tr" | "y") || c.get(i + word.len()) != Some(&'/') {
            return None;
        }
        parts = if matches!(word.as_str(), "s" | "tr" | "y") {
            2
        } else {
            1
        };
        start = i + word.len();
    } else {
        if c[i] != '/' || matches!(c.get(i + 1), Some('/' | '*')) && lang == Lang::Node {
            return None;
        }
        // A slash starts a regex only where a value is expected.
        let value_expected = match last {
            None => true,
            Some(Tok::Punct(p)) => !matches!(p, ')' | ']' | '}'),
            Some(Tok::Ident(w)) => matches!(
                w.as_str(),
                "return"
                    | "typeof"
                    | "case"
                    | "in"
                    | "of"
                    | "when"
                    | "if"
                    | "unless"
                    | "and"
                    | "or"
                    | "not"
                    | "split"
                    | "grep"
                    | "map"
            ),
            _ => false,
        };
        if !value_expected {
            return None;
        }
    }
    let mut k = start + 1;
    let mut class = false;
    while k < c.len() {
        match c[k] {
            '\\' => k += 1,
            '[' => class = true,
            ']' => class = false,
            '/' if !class => {
                parts -= 1;
                if parts == 0 {
                    break;
                }
            }
            '\n' => return None,
            _ => {}
        }
        k += 1;
    }
    if k >= c.len() {
        return None;
    }
    k += 1;
    while k < c.len() && c[k].is_ascii_alphabetic() {
        k += 1;
    }
    Some(k)
}

fn code_at(c: &[char], i: usize, s: &str) -> bool {
    s.chars().enumerate().all(|(k, ch)| c.get(i + k) == Some(&ch))
}

/// A value the code computes, as far as it is known.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Val {
    /// One or more possible strings ([`UNKNOWN`] marks unknown pieces).
    Strs(Vec<String>),
    /// A list of strings (an argv).
    List(Vec<String>),
    Unknown,
}

struct Reader<'w, 'a> {
    w: &'w mut Walk<'a>,
    lang: Lang,
    toks: Vec<Tok>,
    vars: BTreeMap<String, Val>,
    /// Parameters of functions defined in the code, with the values their calls pass (by name).
    params: BTreeMap<String, Val>,
    cwd: String,
    /// The script's arguments (`sys.argv[1:]`, `process.argv[2:]`), as the shell gave them.
    argv: Vec<Val>,
    /// How deep [`Reader::eval`] is nested (deeper expressions are unknown).
    depth: std::cell::Cell<usize>,
}

/// The deepest expression [`Reader::eval`] reads.
const MAX_EVAL_DEPTH: usize = 48;

/// Records what `code` in `lang` does.
#[allow(clippy::similar_names, reason = "the shell's `args` and the script's own `argv`")]
pub(super) fn analyze_code(w: &mut Walk<'_>, lang: Lang, code: &str, args: &[Exp]) {
    if lang == Lang::Shell {
        w.run_script(code, args.to_vec());
        return;
    }
    if code.len() > crate::shell::ast::MAX_INPUT {
        w.lower(Certainty::None, "code too long to read");
        return;
    }
    let toks = lex(lang, code);
    let cwd = w.cwd.clone();
    let argv = args
        .iter()
        .map(|a| {
            if a.known() {
                Val::Strs(vec![a.text.clone()])
            } else {
                Val::Unknown
            }
        })
        .collect();
    let mut r = Reader {
        w,
        lang,
        toks,
        vars: BTreeMap::new(),
        params: BTreeMap::new(),
        cwd,
        argv,
        depth: std::cell::Cell::new(0),
    };
    r.function_params();
    r.read();
}

fn ident(t: Option<&Tok>) -> Option<&str> {
    match t {
        Some(Tok::Ident(s)) => Some(s.as_str()),
        _ => None,
    }
}

fn punct(t: Option<&Tok>, c: char) -> bool {
    t == Some(&Tok::Punct(c))
}

impl Reader<'_, '_> {
    /// The index of the `)` (or `]`, `}`) closing the bracket at `open`.
    fn close(&self, open: usize) -> usize {
        let (o, c) = match self.toks.get(open) {
            Some(Tok::Punct('(')) => ('(', ')'),
            Some(Tok::Punct('[')) => ('[', ']'),
            Some(Tok::Punct('{')) => ('{', '}'),
            _ => return open,
        };
        let mut depth = 0_usize;
        for k in open..self.toks.len() {
            if self.toks[k] == Tok::Punct(o) {
                depth += 1;
            } else if self.toks[k] == Tok::Punct(c) {
                depth -= 1;
                if depth == 0 {
                    return k;
                }
            }
        }
        self.toks.len()
    }

    /// The arguments of the call whose `(` is at `open`: token ranges, keyword name if any.
    fn args(&self, open: usize) -> Vec<(Option<String>, usize, usize)> {
        let end = self.close(open);
        let mut out = Vec::new();
        let mut start = open + 1;
        let mut depth = 0_usize;
        for k in open + 1..end.min(self.toks.len()) {
            match &self.toks[k] {
                Tok::Punct('(' | '[' | '{') => depth += 1,
                Tok::Punct(')' | ']' | '}') => depth = depth.saturating_sub(1),
                Tok::Punct(',') if depth == 0 => {
                    out.push((start, k));
                    start = k + 1;
                }
                _ => {}
            }
        }
        if start < end {
            out.push((start, end));
        }
        out.into_iter()
            .map(|(s, e)| {
                // `name=value` (Python), `name: value` (Ruby).
                if let (Some(name), true) =
                    (ident(self.toks.get(s)), punct(self.toks.get(s + 1), '=') && !punct(self.toks.get(s + 2), '='))
                {
                    (Some(name.to_owned()), s + 2, e)
                } else {
                    (None, s, e)
                }
            })
            .collect()
    }

    /// The value of the expression in tokens `s..e`.
    #[allow(clippy::too_many_lines, reason = "one case per expression shape")]
    #[allow(clippy::many_single_char_names, reason = "token indexes: s(tart), e(nd), k")]
    fn eval(&self, s: usize, e: usize) -> Val {
        if self.depth.get() >= MAX_EVAL_DEPTH {
            return Val::Unknown;
        }
        self.depth.set(self.depth.get() + 1);
        let v = self.eval_at(s, e);
        self.depth.set(self.depth.get() - 1);
        v
    }

    #[allow(clippy::too_many_lines, reason = "one case per expression shape")]
    #[allow(clippy::many_single_char_names, reason = "token indexes: s(tart), e(nd), k")]
    fn eval_at(&self, s: usize, e: usize) -> Val {
        if s >= e {
            return Val::Unknown;
        }
        let toks = &self.toks[s..e];
        // `a / b / c` (Python paths) and `a + b` (strings).
        let mut parts: Vec<(usize, usize)> = Vec::new();
        let mut depth = 0_usize;
        let mut start = s;
        let mut joiner = None;
        for (k, t) in toks.iter().enumerate() {
            match t {
                Tok::Punct('(' | '[' | '{') => depth += 1,
                Tok::Punct(')' | ']' | '}') => depth = depth.saturating_sub(1),
                Tok::Punct(op @ ('/' | '+')) if depth == 0 => {
                    joiner = Some(*op);
                    parts.push((start, s + k));
                    start = s + k + 1;
                }
                _ => {}
            }
        }
        if let Some(op) = joiner {
            parts.push((start, e));
            let mut acc: Vec<String> = vec![String::new()];
            for (k, (ps, pe)) in parts.iter().enumerate() {
                let v = match self.eval(*ps, *pe) {
                    Val::Strs(v) => v,
                    _ => vec![UNKNOWN.to_string()],
                };
                let mut next = Vec::new();
                for a in &acc {
                    for b in &v {
                        let sep = if op == '/' && k > 0 && !a.ends_with('/') {
                            "/"
                        } else {
                            ""
                        };
                        next.push(format!("{a}{sep}{b}"));
                    }
                }
                acc = next.into_iter().take(16).collect();
            }
            return Val::Strs(acc);
        }
        match toks {
            [Tok::Str(v)] => Val::Strs(vec![v.clone()]),
            // `sys.argv[1]` (Python), `process.argv[2]` (Node), `ARGV[0]` (Ruby), `$ARGV[0]` (Perl).
            [Tok::Ident(m), Tok::Punct('.'), Tok::Ident(a), Tok::Punct('['), Tok::Num(n), Tok::Punct(']')]
                if (m == "sys" || m == "process") && a == "argv" =>
            {
                let skip = if m == "process" {
                    2
                } else {
                    1
                };
                n.parse::<usize>()
                    .ok()
                    .and_then(|n| n.checked_sub(skip))
                    .and_then(|n| self.argv.get(n))
                    .cloned()
                    .unwrap_or(Val::Unknown)
            }
            [Tok::Ident(a), Tok::Punct('['), Tok::Num(n), Tok::Punct(']')] if a == "ARGV" || a == "$ARGV" => {
                n.parse::<usize>().ok().and_then(|n| self.argv.get(n)).cloned().unwrap_or(Val::Unknown)
            }
            [Tok::Ident(name)] => {
                self.vars.get(name).or_else(|| self.params.get(name)).cloned().unwrap_or(Val::Unknown)
            }
            // `(…)`
            [Tok::Punct('('), .., Tok::Punct(')')] if self.close(s) == e - 1 => self.eval(s + 1, e - 1),
            // `[a, b]`, `%w[…]` not handled
            [Tok::Punct('['), .., Tok::Punct(']')] if self.close(s) == e - 1 => {
                let mut list = Vec::new();
                let mut start = s + 1;
                let mut depth = 0_usize;
                for k in s + 1..e {
                    match &self.toks[k] {
                        Tok::Punct('(' | '[' | '{') => depth += 1,
                        Tok::Punct(')' | ']' | '}') if depth > 0 => depth -= 1,
                        Tok::Punct(',' | ']') if depth == 0 => {
                            if start < k {
                                match self.eval(start, k) {
                                    Val::Strs(v) if v.len() == 1 => list.push(v[0].clone()),
                                    _ => list.push(UNKNOWN.to_string()),
                                }
                            }
                            start = k + 1;
                        }
                        _ => {}
                    }
                }
                Val::List(list)
            }
            _ => {
                // A call: `Path(x)`, `os.path.join(a, b)`, `x.expanduser()`, `Path.home()`.
                let (callee, open) = self.callee_ending(s, e);
                let Some(open) = open else {
                    return Val::Unknown;
                };
                let args = self.args(open);
                let arg = |k: usize| args.get(k).map_or(Val::Unknown, |(_, a, b)| self.eval(*a, *b));
                let last = callee.rsplit('.').next().unwrap_or(&callee);
                match last {
                    "Path" | "PurePath" | "PosixPath" | "str" | "abspath" | "realpath" | "normpath" | "resolve"
                    | "String" | "fspath" | "absolute" => {
                        if args.is_empty() {
                            // `p.resolve()`: the receiver's value.
                            self.receiver(open)
                        } else {
                            arg(0)
                        }
                    }
                    "expanduser" | "expandvars" => {
                        let v = if args.is_empty() {
                            self.receiver(open)
                        } else {
                            arg(0)
                        };
                        match v {
                            Val::Strs(v) => Val::Strs(
                                v.into_iter()
                                    .map(|p| {
                                        if p == "~" || p.starts_with("~/") {
                                            format!("{}{}", self.w.ctx.workspace.home, &p[1..])
                                        } else {
                                            p
                                        }
                                    })
                                    .collect(),
                            ),
                            other => other,
                        }
                    }
                    "home" if callee.ends_with("Path.home") => Val::Strs(vec![self.w.ctx.workspace.home.clone()]),
                    "homedir" | "gethome" => Val::Strs(vec![self.w.ctx.workspace.home.clone()]),
                    "cwd" | "getcwd" => Val::Strs(vec![self.cwd.clone()]),
                    "join" | "joinpath" | "resolve_path" if callee != "join" => {
                        let mut acc = if last == "joinpath" {
                            vec![self.receiver(open)]
                        } else {
                            Vec::new()
                        };
                        acc.extend((0..args.len()).map(arg));
                        let mut out = String::new();
                        for v in acc {
                            match v {
                                Val::Strs(v) if v.len() == 1 => {
                                    if v[0].starts_with('/') {
                                        out.clone_from(&v[0]);
                                    } else {
                                        if !out.is_empty() && !out.ends_with('/') {
                                            out.push('/');
                                        }
                                        out.push_str(&v[0]);
                                    }
                                }
                                _ => {
                                    if !out.is_empty() && !out.ends_with('/') {
                                        out.push('/');
                                    }
                                    out.push(UNKNOWN);
                                }
                            }
                        }
                        Val::Strs(vec![out])
                    }
                    "dirname" | "parent" => match if args.is_empty() {
                        self.receiver(open)
                    } else {
                        arg(0)
                    } {
                        Val::Strs(v) => Val::Strs(
                            v.into_iter()
                                .map(|p| p.rsplit_once('/').map_or_else(|| ".".to_owned(), |(d, _)| d.to_owned()))
                                .collect(),
                        ),
                        other => other,
                    },
                    _ => Val::Unknown,
                }
            }
        }
    }

    /// For tokens `s..e` ending in a call `…name(…)`: the dotted callee and the index of its `(`.
    fn callee_ending(&self, s: usize, e: usize) -> (String, Option<usize>) {
        if !punct(self.toks.get(e - 1), ')') {
            // `x.parent` (a property).
            if let Some(name) = ident(self.toks.get(e - 1))
                && name == "parent"
                && e >= 2
                && punct(self.toks.get(e - 2), '.')
            {
                let v = self.eval(s, e - 2);
                return (format!("{v:?}"), None);
            }
            return (String::new(), None);
        }
        // Find the `(` matching the last `)`.
        let mut depth = 0_usize;
        let mut open = None;
        for k in (s..e).rev() {
            match self.toks[k] {
                Tok::Punct(')') => depth += 1,
                Tok::Punct('(') => {
                    depth -= 1;
                    if depth == 0 {
                        open = Some(k);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(open) = open else {
            return (String::new(), None);
        };
        (self.dotted_before(open), Some(open))
    }

    /// The dotted name just before `open` (`os.path.join`, `shutil.rmtree`).
    fn dotted_before(&self, open: usize) -> String {
        let mut names = Vec::new();
        let mut k = open;
        while k > 0 {
            let Some(name) = ident(self.toks.get(k - 1)) else {
                break;
            };
            names.push(name.to_owned());
            if k >= 2 && punct(self.toks.get(k - 2), '.') {
                k -= 2;
            } else {
                break;
            }
        }
        names.reverse();
        names.join(".")
    }

    /// The value of the receiver of the method call whose `(` is at `open` (`Path('x').write_text(…)`, `p.unlink()`).
    fn receiver(&self, open: usize) -> Val {
        // tokens: <receiver> . method (
        let method_at = open.checked_sub(1);
        let Some(dot) = method_at.and_then(|m| m.checked_sub(1)) else {
            return Val::Unknown;
        };
        if !punct(self.toks.get(dot), '.') {
            return Val::Unknown;
        }
        // The receiver: an identifier, or a call `X(...)` ending at `dot`.
        let end = dot;
        let Some(last) = end.checked_sub(1) else {
            return Val::Unknown;
        };
        match self.toks.get(last) {
            Some(Tok::Ident(_)) => {
                // `a.b.c`: the whole dotted name before; evaluate the last identifier as a variable.
                let mut start = last;
                while start >= 2 && punct(self.toks.get(start - 1), '.') && ident(self.toks.get(start - 2)).is_some() {
                    start -= 2;
                }
                if start == last {
                    self.eval(start, end)
                } else {
                    Val::Unknown
                }
            }
            Some(Tok::Punct(')')) => {
                let mut depth = 0_usize;
                for k in (0..end).rev() {
                    match self.toks[k] {
                        Tok::Punct(')') => depth += 1,
                        Tok::Punct('(') => {
                            depth -= 1;
                            if depth == 0 {
                                let mut start = k;
                                while start >= 1 && ident(self.toks.get(start - 1)).is_some() {
                                    start -= 1;
                                    if start >= 1 && punct(self.toks.get(start - 1), '.') {
                                        start -= 1;
                                    } else {
                                        break;
                                    }
                                }
                                return self.eval(start, end);
                            }
                        }
                        _ => {}
                    }
                }
                Val::Unknown
            }
            Some(Tok::Str(v)) => Val::Strs(vec![v.clone()]),
            _ => Val::Unknown,
        }
    }

    /// Collects functions' parameters and the values their calls pass, so `def edit(p): open(p, 'w')` followed by
    /// `edit('a.rs')` writes `a.rs`.
    fn function_params(&mut self) {
        let mut defs: Vec<(String, Vec<String>)> = Vec::new();
        for k in 0..self.toks.len() {
            let is_def = matches!(ident(self.toks.get(k)), Some("def" | "function" | "sub"));
            if is_def
                && let Some(name) = ident(self.toks.get(k + 1))
                && punct(self.toks.get(k + 2), '(')
            {
                let params: Vec<String> = self
                    .args(k + 2)
                    .into_iter()
                    .filter_map(|(kw, a, _)| kw.or_else(|| ident(self.toks.get(a)).map(str::to_owned)))
                    .collect();
                defs.push((name.to_owned(), params));
            }
        }
        for (name, params) in defs {
            let mut values: Vec<Vec<String>> = vec![Vec::new(); params.len()];
            let mut known = vec![true; params.len()];
            let mut calls = 0;
            for k in 0..self.toks.len() {
                if ident(self.toks.get(k)) == Some(name.as_str())
                    && punct(self.toks.get(k + 1), '(')
                    && !matches!(ident(self.toks.get(k.wrapping_sub(1))), Some("def" | "function" | "sub"))
                {
                    calls += 1;
                    for (i, (_, a, b)) in self.args(k + 1).into_iter().enumerate().take(params.len()) {
                        match self.eval(a, b) {
                            Val::Strs(v) => values[i].extend(v),
                            _ => known[i] = false,
                        }
                    }
                }
            }
            for (i, p) in params.iter().enumerate() {
                if calls > 0 && known[i] && !values[i].is_empty() {
                    self.params.insert(p.clone(), Val::Strs(values[i].clone()));
                }
            }
        }
    }

    /// Walks the tokens in order: assignments update variables, calls make effects.
    #[allow(clippy::too_many_lines, reason = "one case per kind of call")]
    #[allow(clippy::many_single_char_names, reason = "token indexes: s(tart), e(nd), k, n")]
    fn read(&mut self) {
        let n = self.toks.len();
        let mut k = 0;
        while k < n {
            // Commands in backticks.
            if let Tok::Cmd(text) = &self.toks[k] {
                let text = text.clone();
                self.shell(&Val::Strs(vec![text]));
                k += 1;
                continue;
            }
            // `for x in [a, b]:` / `for x in (a, b)`
            if ident(self.toks.get(k)) == Some("for")
                && let Some(var) = ident(self.toks.get(k + 1)).map(str::to_owned)
                && ident(self.toks.get(k + 2)) == Some("in")
            {
                let s = k + 3;
                let e = match self.toks.get(s) {
                    Some(Tok::Punct('[' | '(')) => self.close(s) + 1,
                    _ => s + 1,
                };
                let v = match self.eval(s, e) {
                    Val::List(l) => Val::Strs(l),
                    Val::Strs(v) => Val::Strs(v),
                    Val::Unknown => {
                        // `for (a, b) in pairs`: a tuple of names, unknown.
                        Val::Unknown
                    }
                };
                self.vars.insert(var, v);
                k = e;
                continue;
            }
            // `name = value` (not `==`, not a keyword argument inside a call).
            if let Some(name) = ident(self.toks.get(k)).map(str::to_owned)
                && punct(self.toks.get(k + 1), '=')
                && !punct(self.toks.get(k + 2), '=')
                && !punct(self.toks.get(k.wrapping_sub(1)), '(')
                && !punct(self.toks.get(k.wrapping_sub(1)), ',')
            {
                // The value runs to the end of the statement: a newline is not a token, so up to a `;` or the next
                // `name =` at depth 0, at most 64 tokens.
                let s = k + 2;
                let mut e = s;
                let mut depth = 0_usize;
                while e < n && e - s < 256 {
                    match &self.toks[e] {
                        Tok::Punct('(' | '[' | '{') => depth += 1,
                        Tok::Punct(')' | ']' | '}') => {
                            if depth == 0 {
                                break;
                            }
                            depth -= 1;
                        }
                        Tok::Punct(';' | '\n') if depth == 0 => break,
                        Tok::Ident(_)
                            if depth == 0
                                && e > s
                                && punct(self.toks.get(e + 1), '=')
                                && !punct(self.toks.get(e + 2), '=') =>
                        {
                            break;
                        }
                        Tok::Ident(kw)
                            if depth == 0
                                && e > s
                                && matches!(
                                    kw.as_str(),
                                    "if" | "for"
                                        | "while"
                                        | "def"
                                        | "import"
                                        | "from"
                                        | "return"
                                        | "print"
                                        | "with"
                                        | "try"
                                        | "class"
                                        | "assert"
                                        | "open"
                                        | "const"
                                        | "let"
                                        | "var"
                                        | "my"
                                )
                            // A new statement starts (the newline before it is not a token).
                            && !punct(self.toks.get(e - 1), '.')
                                && !punct(self.toks.get(e - 1), '(')
                                && !punct(self.toks.get(e - 1), ',')
                                && !punct(self.toks.get(e - 1), '=') =>
                        {
                            break;
                        }
                        _ => {}
                    }
                    e += 1;
                }
                // Only a value the whole statement is: a literal, a name, a path expression, a call that computes
                // a path. `s = open(p).read()` is not a path, and is unknown.
                let v = self.eval(s, e);
                let name = name.trim_start_matches('$').to_owned();
                self.vars.insert(name, v);
                k += 2;
                continue;
            }
            if punct(self.toks.get(k + 1), '(')
                && let Some(_) = ident(self.toks.get(k))
            {
                let open = k + 1;
                let callee = self.dotted_before(open);
                self.call(&callee, open);
            }
            k += 1;
        }
    }

    fn arg_val(&self, args: &[(Option<String>, usize, usize)], pos: usize, kw: &[&str]) -> Option<Val> {
        if let Some((_, a, b)) = args.iter().find(|(name, _, _)| name.as_deref().is_some_and(|n| kw.contains(&n))) {
            return Some(self.eval(*a, *b));
        }
        args.iter().filter(|(name, _, _)| name.is_none()).nth(pos).map(|(_, a, b)| self.eval(*a, *b))
    }

    fn paths(&self, v: &Val) -> Vec<Exp> {
        match v {
            Val::Strs(list) | Val::List(list) => list
                .iter()
                .map(|p| {
                    let p = if p == "~" || p.starts_with("~/") {
                        format!("{}{}", self.w.ctx.workspace.home, &p[1..])
                    } else {
                        p.clone()
                    };
                    let text = if p.starts_with('/') || p.starts_with(UNKNOWN) || self.cwd.is_empty() {
                        p
                    } else {
                        normalize(&p, &self.cwd, &self.w.ctx.workspace.home)
                    };
                    Exp {
                        why: if text.contains(UNKNOWN) {
                            vec!["a path the code computes".to_owned()]
                        } else {
                            Vec::new()
                        },
                        text,
                        glob: false,
                    }
                })
                .collect(),
            Val::Unknown => vec![unknown("a path the code computes".to_owned())],
        }
    }

    fn write(&mut self, v: &Val) {
        for e in self.paths(v) {
            let p = self.w.path(&e);
            self.w.write(p);
        }
    }

    fn read_path(&mut self, v: &Val) {
        for e in self.paths(v) {
            if e.known() {
                self.w.read_file(&e);
            }
        }
    }

    fn delete(&mut self, v: &Val, recursive: bool) {
        let paths: Vec<_> = self.paths(v).iter().map(|e| self.w.path(e)).collect();
        self.w.effect(Effect::FileDelete {
            paths,
            recursive,
        });
    }

    /// A program the code runs: a command line (`shell`), or an argv.
    fn shell(&mut self, v: &Val) {
        match v {
            Val::Strs(list) if list.iter().all(|s| !s.contains(UNKNOWN)) => {
                for s in list.clone() {
                    self.w.run_script(&s, Vec::new());
                }
            }
            Val::Strs(list) => {
                // A command line with values the code computes: shown as far as it can be read, never certain.
                for s in list.clone() {
                    self.w.run_script(&s.replace(UNKNOWN, "\"$__reins_unknown__\""), Vec::new());
                }
                self.w.lower(Certainty::None, "a command line the code builds at run time");
            }
            Val::List(argv) if !argv.is_empty() && !argv[0].contains(UNKNOWN) => {
                let line = argv
                    .iter()
                    .map(|a| {
                        if a.contains(UNKNOWN) {
                            "\"$an_argument_the_code_computes\"".to_owned()
                        } else {
                            quote(a)
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" ");
                self.w.run_script(&line, Vec::new());
            }
            _ => {
                self.w.effect(Effect::Exec {
                    program: "a program the code chooses".to_owned(),
                    source: CodeSource::Unknown,
                });
                self.w.lower(Certainty::None, "a program the code runs, built at run time");
            }
        }
    }

    fn network(&mut self, v: Option<&Val>, write: bool) {
        let host = match v {
            Some(Val::Strs(list)) => {
                list.iter().find_map(|u| super::programs::url_host(u)).filter(|h| !h.contains(UNKNOWN))
            }
            _ => None,
        };
        let host = host.unwrap_or_else(|| {
            self.w.lower(Certainty::Partial, "the host the code connects to");
            "a host the code chooses".to_owned()
        });
        self.w.effect(Effect::Network {
            host,
            method: if write {
                NetMethod::Write
            } else {
                NetMethod::Read
            },
        });
    }

    #[allow(clippy::too_many_lines, reason = "one arm per library call")]
    #[allow(clippy::similar_names, reason = "a call's `args` and the code's `argv`")]
    fn call(&mut self, callee: &str, open: usize) {
        let args = self.args(open);
        let last = callee.rsplit('.').next().unwrap_or(callee);
        let has_kw = |k: &str| args.iter().any(|(n, _, _)| n.as_deref() == Some(k));
        let has_data = has_kw("data") || has_kw("json") || has_kw("files") || has_kw("content") || has_kw("body");
        let first = self.arg_val(&args, 0, &["file", "path", "filename", "name", "src", "args", "cmd"]);
        let second = self.arg_val(&args, 1, &["mode", "dst", "dest", "flags"]);
        match self.lang {
            Lang::Python => match callee {
                "open" | "io.open" | "codecs.open" | "builtins.open" | "os.open" => {
                    let mode = second;
                    let writes =
                        matches!(&mode, Some(Val::Strs(m)) if m.iter().any(|m| m.contains(['w', 'a', 'x', '+'])));
                    let path = first.unwrap_or(Val::Unknown);
                    if writes {
                        self.write(&path);
                    } else {
                        self.read_path(&path);
                    }
                }
                "os.remove" | "os.unlink" | "os.rmdir" | "os.removedirs" | "send2trash" | "send2trash.send2trash" => {
                    self.delete(&first.unwrap_or(Val::Unknown), false);
                }
                "shutil.rmtree" => self.delete(&first.unwrap_or(Val::Unknown), true),
                "os.rename" | "os.replace" | "shutil.move" | "os.renames" => {
                    self.write(&first.unwrap_or(Val::Unknown));
                    self.write(&second.unwrap_or(Val::Unknown));
                }
                "shutil.copy" | "shutil.copy2" | "shutil.copyfile" | "shutil.copytree" | "os.symlink" | "os.link" => {
                    if let Some(f) = &first {
                        self.read_path(f);
                    }
                    self.write(&second.unwrap_or(Val::Unknown));
                }
                "os.chmod" | "os.chown" | "os.truncate" | "os.utime" => self.write(&first.unwrap_or(Val::Unknown)),
                "os.chdir" => {
                    if let Some(Val::Strs(v)) = &first
                        && let [one] = v.as_slice()
                        && !one.contains(UNKNOWN)
                    {
                        self.cwd = normalize(one, &self.cwd.clone(), &self.w.ctx.workspace.home);
                    }
                }
                "os.system"
                | "os.popen"
                | "subprocess.getoutput"
                | "subprocess.getstatusoutput"
                | "commands.getoutput"
                | "pty.spawn"
                | "asyncio.create_subprocess_shell" => self.shell(&first.unwrap_or(Val::Unknown)),
                "subprocess.run"
                | "subprocess.call"
                | "subprocess.check_call"
                | "subprocess.check_output"
                | "subprocess.Popen"
                | "asyncio.create_subprocess_exec"
                | "os.execv"
                | "os.execvp"
                | "os.execl"
                | "os.execlp"
                | "os.spawnl"
                | "os.spawnv"
                | "os.spawnvp"
                | "os.posix_spawn" => {
                    let v = first.unwrap_or(Val::Unknown);
                    // `subprocess.run("…", shell=True)` and `run(["a", "b"])` both read as a command line.
                    self.shell(&v);
                }
                "urllib.request.urlopen"
                | "urlopen"
                | "requests.get"
                | "requests.head"
                | "httpx.get"
                | "httpx.head"
                | "urllib.request.urlretrieve"
                | "urlretrieve"
                | "aiohttp.request" => {
                    let write =
                        has_data || matches!(&first, Some(Val::Unknown) | None) && self.recent_request_data(open);
                    self.network(first.as_ref(), write);
                    if matches!(callee, "urllib.request.urlretrieve" | "urlretrieve")
                        && let Some(dst) = second
                    {
                        self.write(&dst);
                    }
                }
                "urllib.request.Request" | "Request" => {
                    let write = has_data || has_kw("method") || args.len() >= 2;
                    self.network(first.as_ref(), write);
                }
                "requests.post"
                | "requests.put"
                | "requests.patch"
                | "requests.delete"
                | "requests.request"
                | "httpx.post"
                | "httpx.put"
                | "httpx.patch"
                | "httpx.delete"
                | "httpx.request"
                | "http.client.HTTPSConnection"
                | "http.client.HTTPConnection"
                | "socket.create_connection"
                | "ftplib.FTP"
                | "paramiko.SSHClient" => {
                    self.network(first.as_ref(), true);
                }
                "smtplib.SMTP" | "smtplib.SMTP_SSL" => self.w.effect(Effect::Message {
                    channel: Channel::Email,
                    to: vec!["recipients the code chooses".to_owned()],
                    attachments: 0,
                }),
                "socket.socket" => self.network(None, true),
                "eval"
                | "exec"
                | "compile"
                | "__import__"
                | "importlib.import_module"
                | "runpy.run_path"
                | "runpy.run_module" => {
                    if !matches!(first, Some(Val::Strs(_))) {
                        if matches!(last, "eval" | "exec" | "compile") {
                            // Code made while it runs (decoded, downloaded, assembled): nothing to read.
                            self.w.lower(Certainty::None, format!("code the Python code builds and runs ({last})"));
                        } else {
                            self.w.lower(Certainty::Partial, format!("a module the Python code chooses ({last})"));
                        }
                    }
                }
                _ => match last {
                    "write_text" | "write_bytes" | "touch" | "symlink_to" | "hardlink_to" | "chmod" | "mkdir" => {
                        let v = self.receiver(open);
                        if last == "mkdir" {
                            if let Val::Strs(_) = v {
                                // Making a folder loses nothing.
                            }
                        } else {
                            self.write(&v);
                        }
                    }
                    "unlink" | "rmdir" => {
                        let v = self.receiver(open);
                        if callee.split('.').count() > 1 {
                            self.delete(&v, false);
                        }
                    }
                    "rename" | "replace" if callee.contains('.') && !callee.starts_with("str") => {
                        let v = self.receiver(open);
                        // `s.replace(a, b)` on a string is text: only a receiver that is a path counts.
                        if matches!(&v, Val::Strs(list) if list.iter().all(|p| p.contains('/') || p.contains('.')))
                            && args.len() == 1
                        {
                            self.write(&v);
                            self.write(&first.clone().unwrap_or(Val::Unknown));
                        }
                    }
                    "read_text" | "read_bytes" => {
                        let v = self.receiver(open);
                        self.read_path(&v);
                    }
                    "open" if callee.contains('.') => {
                        let v = self.receiver(open);
                        if !matches!(v, Val::Unknown) {
                            let writes = matches!(&first, Some(Val::Strs(m)) if m.iter().any(|m| m.contains(['w', 'a', 'x', '+'])));
                            if writes {
                                self.write(&v);
                            } else {
                                self.read_path(&v);
                            }
                        }
                    }
                    _ => {}
                },
            },
            Lang::Node => match last {
                "writeFileSync" | "writeFile" | "appendFileSync" | "appendFile" | "createWriteStream"
                | "truncateSync" | "truncate" | "chmodSync" | "chmod" | "symlinkSync" | "symlink" | "utimesSync" => {
                    self.write(&first.unwrap_or(Val::Unknown));
                }
                "rmSync" | "rm" | "rmdirSync" | "rmdir" | "unlinkSync" | "unlink"
                    if callee.contains('.') || last.ends_with("Sync") =>
                {
                    let recursive = self.has_true_option(open, "recursive");
                    self.delete(&first.unwrap_or(Val::Unknown), recursive || last.starts_with("rmdir"));
                }
                "renameSync" | "rename" | "copyFileSync" | "copyFile" | "cpSync" | "cp"
                    if callee.contains('.') || last.ends_with("Sync") =>
                {
                    if last.starts_with("rename") {
                        self.write(&first.clone().unwrap_or(Val::Unknown));
                    } else if let Some(f) = &first {
                        self.read_path(f);
                    }
                    self.write(&second.unwrap_or(Val::Unknown));
                }
                "readFileSync" | "readFile" | "createReadStream" => self.read_path(&first.unwrap_or(Val::Unknown)),
                "exec" | "execSync" | "spawn" | "spawnSync" | "execFile" | "execFileSync" | "fork"
                    if callee.contains("child_process") || callee == last || callee.starts_with("cp.") =>
                {
                    let mut v = first.unwrap_or(Val::Unknown);
                    if matches!(last, "spawn" | "spawnSync" | "execFile" | "execFileSync")
                        && let (Val::Strs(p), Some(Val::List(rest))) = (&v, &second)
                        && p.len() == 1
                    {
                        let mut argv = vec![p[0].clone()];
                        argv.extend(rest.clone());
                        v = Val::List(argv);
                    }
                    self.shell(&v);
                }
                "fetch" | "get" | "request" | "post" | "put" | "delete" | "patch"
                    if matches!(
                        callee,
                        "fetch"
                            | "http.get"
                            | "https.get"
                            | "http.request"
                            | "https.request"
                            | "axios.get"
                            | "axios.post"
                            | "axios.put"
                            | "axios.delete"
                            | "axios.patch"
                            | "axios.request"
                            | "got"
                            | "got.post"
                    ) =>
                {
                    let write = matches!(last, "post" | "put" | "delete" | "patch") || self.has_method_option(open);
                    self.network(first.as_ref(), write);
                }
                "connect" | "createConnection" if callee.starts_with("net.") || callee.starts_with("tls.") => {
                    self.network(None, true);
                }
                "eval" | "Function" | "runInThisContext" | "runInNewContext" | "runInContext"
                    if !matches!(first, Some(Val::Strs(_))) =>
                {
                    self.w.lower(Certainty::None, "code the JavaScript builds and runs");
                }
                _ => {}
            },
            Lang::Ruby => match callee {
                "File.write" | "File.binwrite" | "IO.write" | "FileUtils.touch" | "FileUtils.chmod" | "File.chmod" => {
                    self.write(&first.unwrap_or(Val::Unknown));
                }
                "File.open" | "open" => {
                    let writes = matches!(&second, Some(Val::Strs(m)) if m.iter().any(|m| m.contains(['w', 'a', '+'])));
                    let path = first.unwrap_or(Val::Unknown);
                    if writes {
                        self.write(&path);
                    } else {
                        self.read_path(&path);
                    }
                }
                "File.delete" | "File.unlink" | "FileUtils.rm" | "FileUtils.rm_f" | "Dir.rmdir" | "Dir.delete" => {
                    self.delete(&first.unwrap_or(Val::Unknown), false);
                }
                "FileUtils.rm_rf" | "FileUtils.rm_r" | "FileUtils.remove_dir" | "FileUtils.remove_entry" => {
                    self.delete(&first.unwrap_or(Val::Unknown), true);
                }
                "FileUtils.mv" | "File.rename" | "FileUtils.cp" | "FileUtils.cp_r" => {
                    self.write(&second.unwrap_or(Val::Unknown));
                }
                "File.read" | "IO.read" | "File.readlines" => self.read_path(&first.unwrap_or(Val::Unknown)),
                "system" | "exec" | "spawn" | "Open3.capture2" | "Open3.capture3" | "Open3.popen3" | "IO.popen"
                | "Kernel.system" => {
                    let v = match (&first, args.len()) {
                        (Some(Val::Strs(p)), n) if n > 1 => {
                            let mut argv = vec![p.first().cloned().unwrap_or_default()];
                            for (_, a, b) in args.iter().skip(1) {
                                match self.eval(*a, *b) {
                                    Val::Strs(v) => argv.push(v.first().cloned().unwrap_or_default()),
                                    _ => argv.push(UNKNOWN.to_string()),
                                }
                            }
                            Val::List(argv)
                        }
                        (Some(v), _) => v.clone(),
                        (None, _) => Val::Unknown,
                    };
                    self.shell(&v);
                }
                "Net.HTTP.get" | "Net.HTTP.get_response" | "URI.open" => self.network(first.as_ref(), false),
                "Net.HTTP.post" | "Net.HTTP.post_form" | "Net.HTTP.start" | "Net.HTTP.new" | "TCPSocket.new" => {
                    self.network(first.as_ref(), true);
                }
                "eval" | "instance_eval" | "class_eval" => {
                    self.w.lower(Certainty::Partial, "code the Ruby code builds and runs");
                }
                _ => {}
            },
            Lang::Perl => match callee {
                "open" => {
                    // `open(FH, '>', $p)`, `open(FH, ">$p")`
                    let mode = self.arg_val(&args, 1, &[]);
                    let path = self.arg_val(&args, 2, &[]);
                    match (mode, path) {
                        (Some(Val::Strs(m)), Some(p)) if m.iter().any(|m| m.contains('>') || m.contains('+')) => {
                            self.write(&p);
                        }
                        (Some(Val::Strs(m)), None) if m.iter().any(|m| m.starts_with('>')) => {
                            let p: Vec<String> =
                                m.iter().map(|m| m.trim_start_matches(['>', '+', ' ']).to_owned()).collect();
                            self.write(&Val::Strs(p));
                        }
                        (Some(Val::Strs(m)), None) => {
                            let p: Vec<String> =
                                m.iter().map(|m| m.trim_start_matches(['<', ' ']).to_owned()).collect();
                            self.read_path(&Val::Strs(p));
                        }
                        (_, Some(p)) => self.read_path(&p),
                        _ => {}
                    }
                }
                "unlink" | "rmdir" => self.delete(&first.unwrap_or(Val::Unknown), false),
                "rmtree" | "remove_tree" | "File.Path.rmtree" | "File.Path.remove_tree" => {
                    self.delete(&first.unwrap_or(Val::Unknown), true);
                }
                "system" | "exec" => self.shell(&first.unwrap_or(Val::Unknown)),
                "eval" => self.w.lower(Certainty::Partial, "code the Perl code builds and runs"),
                _ => {
                    if callee.contains("LWP") || callee.contains("HTTP.Tiny") || callee.contains("IO.Socket") {
                        self.network(first.as_ref(), true);
                    }
                }
            },
            Lang::Shell => {}
        }
    }

    /// `urlopen(req)` where `req` was made with data earlier: approximated by any `data=` in the code before.
    fn recent_request_data(&self, open: usize) -> bool {
        self.toks[..open].windows(2).any(|w| matches!(&w[0], Tok::Ident(n) if n == "data") && w[1] == Tok::Punct('='))
    }

    fn has_true_option(&self, open: usize, name: &str) -> bool {
        let end = self.close(open);
        self.toks[open..end.min(self.toks.len())].windows(3).any(|w| {
            matches!(&w[0], Tok::Ident(n) if n == name)
                && w[1] == Tok::Punct(':')
                && matches!(&w[2], Tok::Ident(v) if v == "true")
        })
    }

    fn has_method_option(&self, open: usize) -> bool {
        let end = self.close(open);
        self.toks[open..end.min(self.toks.len())].windows(3).any(|w| {
            matches!(&w[0], Tok::Ident(n) if n == "method")
                && w[1] == Tok::Punct(':')
                && matches!(&w[2], Tok::Str(v) if !v.eq_ignore_ascii_case("GET"))
                || matches!(&w[0], Tok::Ident(n) if n == "body") && w[1] == Tok::Punct(':')
        })
    }
}

/// A word quoted for a POSIX shell.
fn quote(s: &str) -> String {
    if !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "-_./=:@%+,".contains(c)) {
        s.to_owned()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}
