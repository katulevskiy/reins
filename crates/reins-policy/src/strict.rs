//! A command line read with certainty, or not at all.
//!
//! The phone decides how a desktop question is answered from what it reads in the command. A hand-written reader of
//! all of a shell's language has kept disagreeing with the shell, so this one is closed by construction: it reads a
//! small subset exactly, and anything outside it makes the whole line uncertain (the question then goes to the user
//! in Manual and Auto; Bypass approves it all the same).
//!
//! The subset: plain words; single quotes; double quotes without `$` or a backtick inside; a backslash before a
//! character that is not a letter or a digit (so `C:\Users` is not read the POSIX way by mistake); `;`, `&&`, `||`,
//! `|` and newlines between commands; redirections `>`, `>>`, `<`, `2>`, `2>>`, `2>&1`, `1>&2`, `&>`, `&>>` to a
//! literal target; `NAME=literal` assignments before a command; comments without quotes; a heredoc or a here-string
//! into a program known not to run what it is given (`cat`, `tee`, `grep`, ...), or into a shell, whose text is then
//! read the same way. Wrappers are read with their exact options (`sudo`, `env`, `nice`, `nohup`, `time`, `timeout`,
//! `command`), and `sh -c "…"` is read inside, at most two levels deep. Everything else is uncertain: `$`, backticks,
//! parentheses, braces, `&` alone, globs in a program's name, line continuations, characters that are not ASCII or
//! are control characters outside quotes, a program that runs code it is given or reads it from its input (`eval`,
//! `xargs`, `ssh`, `python3 -c`, `| sh`, ...), more than 64 commands.

use crate::shell::program_name;

/// The most commands a line may hold.
const MAX_SEGMENTS: usize = 64;
/// How deep `sh -c "…"` is read.
const MAX_DEPTH: usize = 2;

/// One simple command, read with certainty: the program's name first, then its arguments (wrappers and assignments
/// left out), and the targets of its redirections.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Seg {
    pub words: Vec<String>,
    pub redirects: Vec<String>,
}

/// A line read with certainty: its commands, in the order they run (a shell's script before the shell). `None`: the
/// line is outside the subset.
#[must_use]
pub fn read(text: &str) -> Option<Vec<Seg>> {
    read_at(text, 0)
}

fn read_at(text: &str, depth: usize) -> Option<Vec<Seg>> {
    if depth > MAX_DEPTH {
        return None;
    }
    let raws = commands(&tokens(text)?)?;
    let mut out = Vec::new();
    for raw in &raws {
        resolve(raw, &raws, depth, &mut out)?;
        if out.len() > MAX_SEGMENTS {
            return None;
        }
    }
    Some(out)
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Word {
    text: String,
    /// Some of it was quoted or escaped (a glob in it is not a glob).
    quoted: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Tok {
    Word(Word),
    /// `;`, a newline, `&&`, `||`.
    End,
    Pipe,
    /// A redirection whose target is the next word.
    Redirect,
    /// `2>&1`, `1>&2`: no target.
    Dup,
    /// A heredoc's body, read when its line ended.
    Heredoc(String),
    /// A here-string: its text is the next word.
    HereString,
}

/// The tokens of a line, or `None` when anything is outside the subset.
#[allow(clippy::too_many_lines, reason = "one case per character the subset knows")]
fn tokens(text: &str) -> Option<Vec<Tok>> {
    let c: Vec<char> = text.chars().collect();
    let mut toks: Vec<Tok> = Vec::new();
    // Heredocs whose bodies come after this line: their delimiter, `<<-`, and their token.
    let mut pending: Vec<(String, bool, usize)> = Vec::new();
    let mut word = String::new();
    let mut quoted = false;
    let mut in_word = false;
    let mut i = 0;
    macro_rules! end_word {
        () => {
            if in_word {
                toks.push(Tok::Word(Word {
                    text: std::mem::take(&mut word),
                    quoted,
                }));
                in_word = false;
                quoted = false;
            }
        };
    }
    while i < c.len() {
        let ch = c[i];
        let next = c.get(i + 1).copied();
        if !ch.is_ascii() || (ch.is_ascii_control() && !matches!(ch, '\n' | '\t')) {
            return None;
        }
        match ch {
            ' ' | '\t' => end_word!(),
            '\n' | ';' => {
                end_word!();
                toks.push(Tok::End);
                if ch == '\n' && !pending.is_empty() {
                    // The bodies, one after the other; reading goes on at the newline after the last delimiter.
                    let mut start = i + 1;
                    for (delimiter, tabs, at) in std::mem::take(&mut pending) {
                        let (body, after) = heredoc_body(&c, start, &delimiter, tabs)?;
                        toks[at] = Tok::Heredoc(body);
                        start = after + 1;
                    }
                    i = start - 2;
                }
            }
            '&' if next == Some('&') => {
                end_word!();
                toks.push(Tok::End);
                i += 1;
            }
            '&' if next == Some('>') => {
                end_word!();
                i += 1;
                if c.get(i + 1) == Some(&'>') {
                    i += 1;
                }
                toks.push(Tok::Redirect);
            }
            '|' if next == Some('|') => {
                end_word!();
                toks.push(Tok::End);
                i += 1;
            }
            '|' if next != Some('&') => {
                end_word!();
                toks.push(Tok::Pipe);
            }
            '\'' => {
                in_word = true;
                quoted = true;
                let close = c[i + 1..].iter().position(|q| *q == '\'')?;
                word.extend(&c[i + 1..i + 1 + close]);
                i += close + 1;
            }
            '"' => {
                in_word = true;
                quoted = true;
                let mut j = i + 1;
                loop {
                    match c.get(j)? {
                        '"' => break,
                        '$' | '`' => return None,
                        '\\' => {
                            let n = *c.get(j + 1)?;
                            if !matches!(n, '"' | '\\') {
                                return None;
                            }
                            word.push(n);
                            j += 2;
                            continue;
                        }
                        q if q.is_ascii_control() && *q != '\t' && *q != '\n' => return None,
                        q => word.push(*q),
                    }
                    j += 1;
                }
                i = j;
            }
            '\\' => {
                // An escaped character that is not a letter or a digit (`\ `, `\;`, `\*`): what it is. Not a line
                // continuation, and not `\U` (`C:\Users`: a Windows path, read otherwise there).
                let n = next?;
                if n.is_ascii_alphanumeric() || n == '\n' || !n.is_ascii() || n.is_ascii_control() {
                    return None;
                }
                in_word = true;
                quoted = true;
                word.push(n);
                i += 1;
            }
            '#' if !in_word => {
                let end = c[i..].iter().position(|q| *q == '\n').map_or(c.len(), |p| i + p);
                if c[i..end].iter().any(|q| matches!(q, '\'' | '"' | '`')) {
                    return None;
                }
                i = end - 1;
            }
            '<' | '>' => {
                // `2>`, `1>`: the digit is the redirection's, not a word.
                let fd = in_word && !quoted && (word == "1" || word == "2") && ch == '>';
                if fd {
                    word.clear();
                    in_word = false;
                } else {
                    end_word!();
                }
                if ch == '<' && next == Some('<') {
                    if c.get(i + 2) == Some(&'<') {
                        toks.push(Tok::HereString);
                        i += 2;
                    } else {
                        let mut j = i + 2;
                        let tabs = c.get(j) == Some(&'-');
                        if tabs {
                            j += 1;
                        }
                        while c.get(j) == Some(&' ') {
                            j += 1;
                        }
                        let (delimiter, after) = delimiter(&c, j)?;
                        pending.push((delimiter, tabs, toks.len()));
                        toks.push(Tok::Heredoc(String::new()));
                        i = after - 1;
                    }
                } else if ch == '<' {
                    if matches!(next, Some('>' | '&' | '(')) {
                        return None;
                    }
                    toks.push(Tok::Redirect);
                } else {
                    let mut j = i + 1;
                    if c.get(j) == Some(&'>') {
                        j += 1;
                    }
                    match c.get(j) {
                        Some('&') => {
                            // `2>&1`, `1>&2`, `>&2`: a descriptor, then the end of the word.
                            if !matches!(c.get(j + 1), Some('1' | '2'))
                                || c.get(j + 2).is_some_and(|q| !q.is_ascii_whitespace() && !matches!(q, ';' | '|'))
                            {
                                return None;
                            }
                            toks.push(Tok::Dup);
                            i = j + 1;
                        }
                        Some('|' | '(') => return None,
                        _ => {
                            toks.push(Tok::Redirect);
                            i = j - 1;
                        }
                    }
                }
            }
            // `^` is cmd's escape (`r^m` is `rm` there).
            '$' | '`' | '(' | ')' | '{' | '}' | '&' | '|' | '^' => return None,
            ch => {
                in_word = true;
                word.push(ch);
            }
        }
        i += 1;
    }
    if in_word {
        toks.push(Tok::Word(Word {
            text: word,
            quoted,
        }));
    }
    if !pending.is_empty() {
        // A heredoc whose body never came.
        return None;
    }
    Some(toks)
}

/// A heredoc's delimiter: letters, digits and `_`, bare or in quotes. Returns it and where it ends.
fn delimiter(c: &[char], at: usize) -> Option<(String, usize)> {
    let (start, quote) = match c.get(at)? {
        q @ ('\'' | '"') => (at + 1, Some(*q)),
        _ => (at, None),
    };
    let len = c[start..].iter().take_while(|q| q.is_ascii_alphanumeric() || **q == '_').count();
    if len == 0 {
        return None;
    }
    let mut end = start + len;
    if let Some(q) = quote {
        if c.get(end) != Some(&q) {
            return None;
        }
        end += 1;
    }
    Some((c[start..start + len].iter().collect(), end))
}

/// A heredoc's body from `at` (the line after the one it was written on): its lines up to the delimiter line.
/// Returns the body and where the line after the delimiter starts. `None` when the delimiter never comes.
fn heredoc_body(c: &[char], at: usize, delimiter: &str, tabs: bool) -> Option<(String, usize)> {
    let mut i = at;
    let mut body = String::new();
    while i <= c.len() {
        let end = c[i..].iter().position(|q| *q == '\n').map_or(c.len(), |p| i + p);
        let line: String = c[i..end].iter().collect();
        let bare = if tabs {
            line.trim_start_matches('\t')
        } else {
            line.as_str()
        };
        if bare == delimiter {
            return Some((body, end));
        }
        if end >= c.len() {
            return None;
        }
        body.push_str(bare);
        body.push('\n');
        i = end + 1;
    }
    None
}

/// A simple command as written.
#[derive(Clone, Debug, Default)]
struct Raw {
    words: Vec<Word>,
    redirects: Vec<String>,
    heredoc: Option<String>,
    here_string: Option<String>,
    /// Its input comes from the command before it (`… | this`).
    piped_in: bool,
}

/// The commands of a line from its tokens (`None`: an empty command between operators, a redirection without a
/// target, two heredocs on one command).
fn commands(toks: &[Tok]) -> Option<Vec<Raw>> {
    let mut out = Vec::new();
    let mut cur = Raw::default();
    let mut piped = false;
    let mut it = toks.iter().peekable();
    let empty =
        |r: &Raw| r.words.is_empty() && r.redirects.is_empty() && r.heredoc.is_none() && r.here_string.is_none();
    while let Some(t) = it.next() {
        match t {
            Tok::Word(w) => cur.words.push(w.clone()),
            Tok::Redirect => match it.next()? {
                Tok::Word(w) => cur.redirects.push(w.text.clone()),
                _ => return None,
            },
            Tok::Dup => {}
            Tok::Heredoc(body) => {
                if cur.heredoc.replace(body.clone()).is_some() {
                    return None;
                }
            }
            Tok::HereString => match it.next()? {
                Tok::Word(w) if cur.here_string.is_none() => cur.here_string = Some(w.text.clone()),
                _ => return None,
            },
            Tok::Pipe | Tok::End => {
                if empty(&cur) {
                    // Leading, trailing or doubled `;` and blank lines are fine; an empty side of a pipe is not.
                    if matches!(t, Tok::Pipe) || piped {
                        return None;
                    }
                } else {
                    cur.piped_in = piped;
                    out.push(std::mem::take(&mut cur));
                }
                piped = matches!(t, Tok::Pipe);
            }
        }
    }
    if !empty(&cur) {
        cur.piped_in = piped;
        out.push(cur);
    } else if piped {
        return None;
    }
    Some(out)
}

/// A wrapper read with its exact options: those without a value, those with one (separate or attached), long ones
/// (`--name` or `--name=value` for those with a value), and words it takes before the command.
struct Wrapper {
    name: &'static str,
    flags: &'static str,
    values: &'static str,
    long_flags: &'static [&'static str],
    long_values: &'static [&'static str],
    operands: usize,
}

const WRAPPERS: &[Wrapper] = &[
    Wrapper {
        name: "sudo",
        flags: "EHnkSbP",
        values: "ugChpUD",
        long_flags: &["preserve-env", "set-home", "non-interactive", "reset-timestamp", "stdin", "background"],
        long_values: &["user", "group", "close-from", "host", "prompt", "other-user", "chdir"],
        operands: 0,
    },
    Wrapper {
        name: "env",
        flags: "i",
        values: "uC",
        long_flags: &["ignore-environment"],
        long_values: &["unset", "chdir"],
        operands: 0,
    },
    Wrapper {
        name: "nice",
        flags: "",
        values: "n",
        long_flags: &[],
        long_values: &["adjustment"],
        operands: 0,
    },
    Wrapper {
        name: "nohup",
        flags: "",
        values: "",
        long_flags: &[],
        long_values: &[],
        operands: 0,
    },
    Wrapper {
        name: "time",
        flags: "pvq",
        values: "fo",
        long_flags: &["portability", "verbose", "quiet"],
        long_values: &["format", "output"],
        operands: 0,
    },
    Wrapper {
        name: "timeout",
        flags: "v",
        values: "sk",
        long_flags: &["preserve-status", "foreground", "verbose"],
        long_values: &["signal", "kill-after"],
        operands: 1,
    },
    Wrapper {
        name: "command",
        flags: "p",
        values: "",
        long_flags: &[],
        long_values: &[],
        operands: 0,
    },
];

/// Where a wrapper's command starts, its options read exactly (`None`: an option it is not known to have).
fn after_wrapper(w: &Wrapper, words: &[Word]) -> Option<usize> {
    let mut i = 1;
    while let Some(word) = words.get(i) {
        let t = word.text.as_str();
        if t == "--" {
            i += 1;
            break;
        }
        if let Some(long) = t.strip_prefix("--") {
            let (name, value) = long.split_once('=').map_or((long, None), |(n, v)| (n, Some(v)));
            if w.long_values.contains(&name) {
                i += 1 + usize::from(value.is_none());
            } else if w.long_flags.contains(&name) && value.is_none() {
                i += 1;
            } else {
                return None;
            }
            continue;
        }
        let Some(short) = t.strip_prefix('-').filter(|s| !s.is_empty()) else {
            break;
        };
        let mut skip = 1;
        for (k, ch) in short.char_indices() {
            if w.values.contains(ch) {
                skip += usize::from(k + 1 == short.len());
                break;
            }
            if !w.flags.contains(ch) {
                return None;
            }
        }
        i += skip;
    }
    if w.name == "timeout" {
        // Its duration: a number, maybe with a unit.
        let d = words.get(i)?.text.as_str();
        let digits = d.trim_end_matches(['s', 'm', 'h', 'd']);
        if digits.is_empty() || !digits.chars().all(|ch| ch.is_ascii_digit() || ch == '.') {
            return None;
        }
    }
    let start = i + w.operands;
    (start <= words.len()).then_some(start)
}

const SHELLS: &[&str] = &["sh", "bash", "zsh", "dash", "ksh", "fish"];

/// Programs that run something they are given (a command, code, a remote or another context): their line is not
/// read with certainty.
const RUNNERS: &[&str] = &[
    "eval",
    "source",
    ".",
    "exec",
    "alias",
    "function",
    "trap",
    "xargs",
    "parallel",
    "ssh",
    "su",
    "sudoedit",
    "script",
    "chroot",
    "stdbuf",
    "ionice",
    "setsid",
    "watch",
    "flock",
    "at",
    "batch",
    "crontab",
    "nsenter",
    "unshare",
    "runuser",
    "doas",
    "systemd-run",
    "busybox",
    "strace",
    "ltrace",
    "gdb",
    "cmd",
    "powershell",
    "pwsh",
    "iex",
    "invoke-expression",
    "start-process",
    "osascript",
    "expect",
];

/// Interpreters: they run code that is not shell.
fn interpreter(program: &str) -> bool {
    ["python", "pypy", "perl", "ruby", "php", "lua", "node", "deno", "bun", "rscript", "julia", "tclsh", "irb"]
        .iter()
        .any(|p| program.starts_with(p))
}

/// Programs known not to run what they are fed on their input (a heredoc's or a here-string's text).
fn text_consumer(program: &str, args: &[Word]) -> bool {
    let has = |w: &str| args.iter().any(|a| a.text == w);
    match program {
        "cat" | "tee" | "grep" | "egrep" | "fgrep" | "rg" | "wc" | "sort" | "uniq" | "head" | "tail" | "diff"
        | "cmp" | "base64" | "md5sum" | "sha1sum" | "sha256sum" | "shasum" | "jq" => true,
        "git" => {
            args.first().is_some_and(|s| matches!(s.text.as_str(), "commit" | "tag"))
                && (has("-F") || has("--file=-") || has("--file"))
        }
        "gh" => has("--body-file") || has("-F"),
        _ => false,
    }
}

/// Resolves one command into `out`: assignments and wrappers left out, what it runs read; `None` when anything about
/// it is outside the subset.
fn resolve(raw: &Raw, line: &[Raw], depth: usize, out: &mut Vec<Seg>) -> Option<()> {
    let mut words: &[Word] = &raw.words;
    loop {
        // assignments, and the shell's reserved words before a command (`then git push …`, `! git push …`)
        while words.first().is_some_and(|w| {
            assignment(&w.text) || (!w.quoted && crate::shell::RESERVED_PREFIXES.contains(&w.text.as_str()))
        }) {
            words = &words[1..];
        }
        let Some(first) = words.first() else {
            break;
        };
        let Some(w) = WRAPPERS.iter().find(|w| w.name == program_name(&first.text)) else {
            break;
        };
        words = &words[after_wrapper(w, words)?..];
    }
    let Some((first, args)) = words.split_first() else {
        // Only redirections (`> out.txt`).
        if raw.heredoc.is_some() || raw.here_string.is_some() {
            return None;
        }
        if raw.redirects.is_empty() {
            // Only assignments (`FOO=1`).
            return Some(());
        }
        out.push(Seg {
            words: Vec::new(),
            redirects: raw.redirects.clone(),
        });
        return Some(());
    };
    if !first.quoted && first.text.contains(['*', '?', '[', '%']) {
        return None;
    }
    let name = program_name(&first.text);
    let program = name.to_ascii_lowercase();
    if RUNNERS.contains(&program.as_str()) || interpreter(&program) {
        return None;
    }
    if program == "find" && args.iter().any(|a| matches!(a.text.as_str(), "-exec" | "-execdir" | "-ok" | "-okdir")) {
        return None;
    }
    if program == "git" && git_runs(args) {
        return None;
    }
    if program == "tee" {
        // A file `tee` writes must not be run later in the line.
        let files: Vec<&str> = args.iter().map(|a| a.text.as_str()).filter(|a| !a.starts_with('-')).collect();
        let others = line.iter().filter(|r| !std::ptr::eq(*r, raw)).flat_map(|r| &r.words);
        if others.clone().any(|w| files.iter().any(|f| w.text == *f || w.text.ends_with(&format!("/{f}")))) {
            return None;
        }
    }
    if SHELLS.contains(&program.as_str()) {
        let at = args.iter().position(|a| {
            a.text == "-c"
                || (a.text.starts_with('-')
                    && !a.text.starts_with("--")
                    && a.text.ends_with('c')
                    && a.text[1..].chars().all(|ch| ch.is_ascii_lowercase()))
        });
        let script = match (at, &raw.heredoc, &raw.here_string) {
            (Some(at), None, None) => Some(args.get(at + 1)?.text.clone()),
            (None, Some(body), None) | (None, None, Some(body)) => Some(body.clone()),
            (None, None, None) if !raw.piped_in && args.iter().any(|a| !a.text.starts_with('-')) => None,
            _ => return None,
        };
        if let Some(script) = script {
            out.extend(read_at(&script, depth + 1)?);
        }
    } else if (raw.heredoc.is_some() || raw.here_string.is_some()) && !text_consumer(&program, args) {
        return None;
    }
    // The program as written (`/usr/bin/git`, `/home/me/.ssh/config`): a file it names counts too.
    let mut seg_words = vec![first.text.clone()];
    seg_words.extend(args.iter().map(|a| a.text.clone()));
    out.push(Seg {
        words: seg_words,
        redirects: raw.redirects.clone(),
    });
    Some(())
}

/// git settings and subcommands that run a program (`-c core.sshCommand=…`, an alias with `!`, `rebase --exec`,
/// `bisect run`, `submodule foreach`).
fn git_runs(args: &[Word]) -> bool {
    let text: Vec<&str> = args.iter().map(|a| a.text.as_str()).collect();
    text.windows(2).any(|w| {
        w[0] == "-c"
            && (w[1].contains('!')
                || w[1].to_ascii_lowercase().contains("command")
                || [
                    "core.pager",
                    "core.editor",
                    "core.fsmonitor",
                    "core.hookspath",
                    "sequence.editor",
                    "diff.external",
                    "credential.helper",
                    "gpg.program",
                    "core.askpass",
                ]
                .iter()
                .any(|k| w[1].to_ascii_lowercase().starts_with(k)))
    }) || text.iter().any(|a| matches!(*a, "--exec" | "-x" | "foreach" | "--extcmd" | "--tool"))
        || text.windows(2).any(|w| w == ["bisect", "run"])
}

fn assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(name, _)| {
        !name.is_empty()
            && name.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
            && !name.starts_with(|ch: char| ch.is_ascii_digit())
    })
}

/// A word quoted so that [`read`] reads it back as itself.
#[must_use]
pub fn quote(word: &str) -> String {
    format!("'{}'", word.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(text: &str) -> Option<Vec<Vec<String>>> {
        read(text).map(|segs| segs.into_iter().map(|s| s.words).collect())
    }

    /// The shell's reserved words before a command are not its program: what follows them is read, in both readings.
    #[test]
    fn reserved_words_never_hide_the_command_after_them() {
        let push: Vec<String> = ["git", "push", "origin", "main"].map(str::to_owned).to_vec();
        for line in [
            "if true; then git push origin main; fi",
            "while true; do git push origin main; done",
            "until false; do git push origin main; done",
            "! git push origin main",
            "{ git push origin main; }",
            "if git push origin main; then echo ok; else echo no; fi",
        ] {
            // the strict reading reads it with the push, or not at all (outside its subset: uncertain)
            let strict = words(line).unwrap_or_default();
            assert!(strict.is_empty() || strict.contains(&push), "strict: {line}: {strict:?}");
            let guard = crate::shell::parse(line).segments;
            assert!(guard.contains(&push), "guard: {line}: {guard:?}");
            assert!(
                !strict
                    .iter()
                    .chain(&guard)
                    .any(|s| s.first().is_some_and(|p| matches!(p.as_str(), "then" | "do" | "!" | "{"))),
                "{line}"
            );
        }
        // the `if` forms are inside the subset: read, with the push
        assert!(words("if true; then git push origin main; fi").unwrap().contains(&push));
        assert!(words("! git push origin main").unwrap().contains(&push));
        // quoted, a reserved word is a word like another
        assert_eq!(words("'then' x"), Some(vec![vec!["then".to_owned(), "x".to_owned()]]));
        assert_eq!(words("time -p git status").unwrap()[0], ["git", "status"], "time stays a wrapper");
    }

    #[test]
    fn the_subset_is_read_exactly() {
        assert_eq!(
            words("cd app && FOO=1 sudo -u root /usr/bin/git push 'origin' \"ma in\"; echo done | tee x"),
            Some(vec![
                vec!["cd".into(), "app".into()],
                vec!["/usr/bin/git".into(), "push".into(), "origin".into(), "ma in".into()],
                vec!["echo".into(), "done".into()],
                vec!["tee".into(), "x".into()],
            ])
        );
        let segs = read("cat < in.txt > out.txt 2>&1").unwrap();
        assert_eq!(
            (segs[0].words.clone(), segs[0].redirects.clone()),
            (vec!["cat".into()], vec!["in.txt".into(), "out.txt".into()])
        );
        assert_eq!(words("bash -c 'rm -rf build'").unwrap()[0], ["rm", "-rf", "build"]);
        assert_eq!(words("cat > notes.md <<'EOF'\nrm -rf ~ and $(x)\nEOF\nls").unwrap().len(), 2, "text into cat");
        assert_eq!(words("bash <<EOF\nrm -rf build\nEOF").unwrap()[0], ["rm", "-rf", "build"]);
        assert_eq!(words("ls # a comment"), Some(vec![vec!["ls".to_owned()]]));
        assert_eq!(words("timeout 10 rm -rf x").unwrap()[0], ["rm", "-rf", "x"]);
        assert_eq!(words("command -p rm -rf x").unwrap()[0], ["rm", "-rf", "x"]);
        assert_eq!(words("time -p rm -rf x").unwrap()[0], ["rm", "-rf", "x"]);
        assert_eq!(words("env -i FOO=1 rm -rf x").unwrap()[0], ["rm", "-rf", "x"]);
        assert_eq!(words("nohup rm -rf x").unwrap()[0], ["rm", "-rf", "x"]);
        assert_eq!(words("sudo bash <<EOF\nrm -rf x\nEOF").unwrap()[0], ["rm", "-rf", "x"], "the body stays with bash");
        assert_eq!(words("env -i sh <<EOF\nrm -rf x\nEOF").unwrap()[0], ["rm", "-rf", "x"]);
        assert_eq!(words("nohup sh <<EOF\nrm -rf x\nEOF").unwrap()[0], ["rm", "-rf", "x"]);
        assert_eq!(words("time bash <<EOF\nrm -rf x\nEOF").unwrap()[0], ["rm", "-rf", "x"]);
        assert!(words("git commit -F - <<'EOF'\nfix: rm -rf docs\nEOF").is_some());
        assert!(words("grep x <<< 'a b'").is_some());
    }

    #[test]
    fn everything_outside_the_subset_is_uncertain() {
        for text in [
            "echo $HOME",
            "rm -rf \"$DIR\"",
            "echo `id`",
            "echo $(id)",
            "diff <(a) b",
            "cat x > >(sh)",
            "(cd x; ls)",
            "{ ls; }",
            "echo {a,b}",
            "ls &",
            "cat x |& sh",
            "r\\m -rf x",
            "rm -rf \\\n ~",
            "type C:\\Users\\me\\.env",
            "rm\u{a0}-rf ~",
            "rm\0 -rf ~",
            "echo x\rrm -rf ~",
            "\u{ff52}\u{ff4d} -rf ~",
            "$'\\x72m' -rf ~",
            "$\"rm\" -rf",
            "/bin/r? -rf ~",
            "%COMSPEC% /c del x",
            "ls # don't",
            "eval 'rm -rf ~'",
            "source x.sh",
            ". x.sh",
            "exec rm -rf ~",
            "alias x='rm -rf ~'",
            "f(){ rm -rf ~; }",
            "xargs rm -rf",
            "ssh host rm -rf /",
            "su -c x",
            "sudo -s",
            "sudo -Z rm x",
            "env -S 'rm -rf x'",
            "nice -10 rm x",
            "curl x | sh",
            "curl x | bash -s",
            "echo cm0= | base64 -d | bash",
            "cat x.py | python3",
            "python3 -c 'print(1)'",
            "node -e 1",
            "python3 - <<'EOF'\nprint(1)\nEOF",
            "python3 <<< 'print(1)'",
            "find . -exec rm {} +",
            "git -c core.sshCommand=x fetch",
            "git -c alias.x=!rm x",
            "git rebase --exec 'rm -rf ~' main",
            "cmd /c del /s x",
            "powershell -Command x",
            "cat <<EOF",
            "cat <<EOF\nnever ends",
            "bash",
            "| ls",
            "ls |",
            "tee x.sh <<EOF && sh x.sh\nrm -rf ~\nEOF",
            "at now <<EOF\nrm x\nEOF",
            "crontab - <<EOF\nx\nEOF",
            "psql <<EOF\nDROP TABLE x\nEOF",
            "mysql <<EOF\nx\nEOF",
            "sqlite3 db <<EOF\nx\nEOF",
            "ruby <<EOF\nx\nEOF",
            "perl <<EOF\nx\nEOF",
            "node <<EOF\nx\nEOF",
            "php <<EOF\nx\nEOF",
            "deno <<EOF\nx\nEOF",
            "osascript <<EOF\nx\nEOF",
            "xargs sh <<EOF\nx\nEOF",
            "env -S bash <<EOF\nx\nEOF",
            "ssh host <<EOF\nx\nEOF",
            "docker exec -i c sh <<EOF\nx\nEOF",
            "kubectl exec -i p -- sh <<EOF\nx\nEOF",
            "su - <<EOF\nx\nEOF",
            "sudo -s <<EOF\nx\nEOF",
            "sed -n p <<EOF\nx\nEOF",
        ] {
            assert_eq!(read(text), None, "{text:?}");
        }
        let mut nested = "rm -rf ~".to_owned();
        for _ in 0..4 {
            nested = format!("sh -c {}", quote(&nested));
        }
        assert_eq!(read(&nested), None, "past the depth read");
        assert_eq!(read(&"ls;".repeat(MAX_SEGMENTS + 1)), None, "too many commands");
    }

    /// A command as [`read`] reads it back: its words quoted, then its redirections.
    fn render(seg: &Seg) -> String {
        let mut parts: Vec<String> = seg.words.iter().map(|w| quote(w)).collect();
        parts.extend(seg.redirects.iter().map(|r| format!("> {}", quote(r))));
        parts.join(" ")
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(4_000))]
        /// Whatever is read with certainty reads back as itself: no line comes out certain with commands that are
        /// not what it says.
        #[test]
        fn what_is_read_with_certainty_reads_back_as_itself(
            text in "[a-z0-9.~/=*?'\"\\\\$`(){};&|> \t\n#-]{0,40}"
        ) {
            if let Some(segs) = read(&text) {
                for seg in &segs {
                    let again = read(&render(seg));
                    proptest::prop_assert!(
                        again.as_ref().is_some_and(|a| a.contains(seg)),
                        "{text:?} gave {seg:?}, which reads back as {again:?}"
                    );
                }
            }
        }

        /// Random bytes: certain only within the subset, and never a panic.
        #[test]
        fn random_bytes_never_panic_and_are_mostly_uncertain(bytes in proptest::collection::vec(0u8..=255, 0..64)) {
            let text = String::from_utf8_lossy(&bytes);
            if let Some(segs) = read(&text) {
                for seg in &segs {
                    let again = read(&render(seg));
                    proptest::prop_assert!(again.as_ref().is_some_and(|a| a.contains(seg)), "{text:?}");
                }
            }
        }
    }
}
