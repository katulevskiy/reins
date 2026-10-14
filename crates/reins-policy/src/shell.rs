//! Shell command lines as the desktop app's guard reads them, shared by the desktop app (its harness hooks) and the
//! phone (which classifies a hook's question itself rather than trusting the computer's label). Command patterns are
//! shell words: the first names the program (`git`, `/usr/bin/git` and `sudo git` all count), the others must appear
//! among its arguments in that order. See `reins_desktop::guard` for the rules.

/// The words of `text`, lowercased (runs of letters, digits and `_`).
pub fn text_words(text: &str) -> Vec<String> {
    text.split(|c: char| !(c.is_alphanumeric() || c == '_')).filter(|w| !w.is_empty()).map(str::to_lowercase).collect()
}

/// `*` matches any run of characters (also none).
#[must_use]
pub fn glob(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (mut pi, mut ti) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while ti < t.len() {
        if pi < p.len() && p[pi] != '*' && p[pi] == t[ti] {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some((pi, ti));
            pi += 1;
        } else if let Some((sp, st)) = star {
            pi = sp + 1;
            ti = st + 1;
            star = Some((sp, st + 1));
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|&c| c == '*')
}

#[must_use]
pub fn word_matches(pattern: &str, word: &str) -> bool {
    let mut chars = pattern.chars();
    if let (Some('-'), Some(letter), None) = (chars.next(), chars.next(), chars.next())
        && letter.is_ascii_alphanumeric()
        && word.len() > 1
        && word.starts_with('-')
        && !word.starts_with("--")
        && word[1..].chars().all(|c| c.is_ascii_alphanumeric())
    {
        return word[1..].contains(letter);
    }
    if let Some(switch) = cmd_switch(pattern) {
        return word.starts_with('/') && word.split('/').any(|s| s.eq_ignore_ascii_case(switch));
    }
    if let Some(name) = powershell_parameter(pattern) {
        let Some(given) = word.strip_prefix('-').filter(|w| !w.starts_with('-')) else {
            return false;
        };
        let given = given.split_once(':').map_or(given, |(p, _)| p);
        return !given.is_empty()
            && given.len() <= name.len()
            && name[..given.len()].eq_ignore_ascii_case(given)
            && given.chars().all(|c| c.is_ascii_alphanumeric());
    }
    glob(pattern, word)
}

/// `s` for the cmd switch `/s` in a pattern.
fn cmd_switch(pattern: &str) -> Option<&str> {
    pattern.strip_prefix('/').filter(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric()))
}

/// `Recurse` for the PowerShell parameter `-Recurse` in a pattern: a dash, a capital letter, then letters.
fn powershell_parameter(pattern: &str) -> Option<&str> {
    let name = pattern.strip_prefix('-')?;
    let mut chars = name.chars();
    (chars.next()?.is_ascii_uppercase() && name.len() > 1 && chars.all(|c| c.is_ascii_alphanumeric())).then_some(name)
}

/// Whether a command pattern (not `text:`) matches one simple command (`seg[0]` the program's name).
#[must_use]
pub fn command_matches(pattern: &str, seg: &[String]) -> bool {
    let p = words(pattern);
    let Some((program, rest)) = p.split_first() else {
        return false;
    };
    let Some((name, args)) = seg.split_first() else {
        return false;
    };
    if !glob(&program.to_lowercase(), &name.to_lowercase()) {
        return false;
    }
    let mut args = args.iter();
    rest.iter().all(|want| args.any(|a| word_matches(want, a)))
}

/// Whether a file pattern matches `path` (see the module docs).
#[must_use]
pub fn file_matches(pattern: &str, path: &str) -> bool {
    let pattern = pattern.trim().trim_start_matches("~/").trim_start_matches("**/");
    let parts: Vec<&str> = path.split(['/', '\\']).filter(|p| !p.is_empty() && *p != ".").collect();
    let want: Vec<&str> = pattern.split('/').filter(|p| !p.is_empty()).collect();
    if want.is_empty() || parts.len() < want.len() {
        return false;
    }
    let tail = &parts[parts.len() - want.len()..];
    want.iter().zip(tail).all(|(w, p)| glob(w, p))
}

/// Shell words of a pattern or a simple command.
#[must_use]
pub fn words(text: &str) -> Vec<String> {
    segments(text).into_iter().next().unwrap_or_default()
}

/// A command line as the guard reads it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Parsed {
    /// The simple commands, each as its words with the program's name first (see [`segments`]).
    pub segments: Vec<Vec<String>>,
    /// Read with certainty: every quote, backtick and `$(` closes (read the POSIX way and the Windows way), and no
    /// command runs code the guard cannot see (a shell or an interpreter reading its standard input from a pipe,
    /// an interpreter fed a heredoc, `git -c core.sshCommand=…`, an option of a wrapper it does not know).
    pub certain: bool,
}

/// Reads a command line: its simple commands and whether it was read with certainty. The commands are those of the
/// POSIX reading, then those the Windows reading adds; both readings must be certain.
#[must_use]
pub fn parse(command: &str) -> Parsed {
    let mut out = Vec::new();
    let (raws, mut certain) = split_commands(command, false);
    for raw in &raws {
        certain &= resolve(raw, &mut out, 0, false);
    }
    let mut windows = Vec::new();
    let (raws, windows_certain) = split_commands(command, true);
    certain &= windows_certain;
    for raw in &raws {
        certain &= resolve(raw, &mut windows, 0, true);
    }
    for seg in windows {
        if !out.contains(&seg) {
            out.push(seg);
        }
    }
    Parsed {
        segments: out,
        certain,
    }
}

/// The simple commands in a shell command line, each as its words with the program's name first: quotes and escapes
/// resolved, `;`, `&`, `|`, `&&`, `||`, newlines, parentheses, `$(…)` and backticks separate commands (a heredoc's
/// body is read as commands too); variable assignments and wrappers (`sudo`, `env`, `nohup`, `time`, `timeout`,
/// `xargs`, ...) before the program are skipped, and what `sh -c`, `eval`, `env -S`, `su -c`, `script -c`, `flock
/// -c`, `watch`, `find -exec`, `ssh host …` and a here-string fed to a shell run is looked into. Then the commands the
/// line holds when read the Windows way (backslashes are part of words, a backtick escapes; `cmd /c`, `powershell
/// -Command` looked into), as far as they differ.
#[must_use]
pub fn segments(command: &str) -> Vec<Vec<String>> {
    parse(command).segments
}

/// The command PowerShell runs for `-EncodedCommand`: base64 of UTF-16LE text.
#[must_use]
pub fn decode_powershell(encoded: &str) -> Option<String> {
    let bytes = data_encoding::BASE64.decode(encoded.trim().as_bytes()).ok()?;
    let (pairs, rest) = bytes.as_chunks::<2>();
    if !rest.is_empty() {
        return None;
    }
    let units: Vec<u16> = pairs.iter().map(|b| u16::from_le_bytes(*b)).collect();
    String::from_utf16(&units).ok()
}

/// What `cmd /c …`, `powershell -Command …`, `pwsh -EncodedCommand …` or `Invoke-Expression …` runs.
fn windows_inner(seg: &[String]) -> Option<String> {
    let program = seg.first()?.to_ascii_lowercase();
    let rest = |i: usize| (i + 1 < seg.len()).then(|| seg[i + 1..].join(" "));
    match program.as_str() {
        "cmd" => {
            let i = seg.iter().position(|w| w.eq_ignore_ascii_case("/c") || w.eq_ignore_ascii_case("/k"))?;
            rest(i)
        }
        "powershell" | "pwsh" => seg.iter().enumerate().skip(1).find_map(|(i, w)| {
            let opt = w.strip_prefix('-')?.to_ascii_lowercase();
            if opt.is_empty() {
                None
            } else if "command".starts_with(&opt) {
                rest(i)
            } else if opt == "ec" || "encodedcommand".starts_with(&opt) {
                decode_powershell(seg.get(i + 1)?)
            } else {
                None
            }
        }),
        "iex" | "invoke-expression" => rest(0),
        _ => None,
    }
}

/// Shells: they run what they are given as commands.
const SHELLS: &[&str] = &["sh", "bash", "zsh", "dash", "ksh", "fish", "ash", "mksh", "tcsh", "csh"];

/// Interpreters: what they run is code the guard does not read.
#[must_use]
pub fn interpreter(program: &str) -> bool {
    let p = program.to_ascii_lowercase();
    ["python", "pypy", "perl", "ruby", "php", "lua", "node", "deno", "bun", "osascript", "rscript", "julia", "tclsh"]
        .iter()
        .any(|i| p.starts_with(i))
        || matches!(p.as_str(), "nodejs" | "irb" | "jshell" | "groovy" | "pwsh" | "powershell")
}

/// A wrapper: a program that runs another given after its own options (`watch` runs its words as a command line,
/// which reads the same). An option it is not known to have makes the line uncertain: its value might be the
/// command.
struct Wrapper {
    name: &'static str,
    /// Short options without a value.
    flags: &'static str,
    /// Short options with a value (separate or attached: `-u root`, `-uroot`).
    values: &'static str,
    long_flags: &'static [&'static str],
    /// Long options with a value (`--user root`, `--user=root`).
    long_values: &'static [&'static str],
    /// Words it takes before the command (`timeout 10 cmd`, `chroot /new cmd`).
    operands: usize,
}

const fn wrapper(
    name: &'static str,
    flags: &'static str,
    values: &'static str,
    long_flags: &'static [&'static str],
    long_values: &'static [&'static str],
    operands: usize,
) -> Wrapper {
    Wrapper {
        name,
        flags,
        values,
        long_flags,
        long_values,
        operands,
    }
}

const WRAPPERS: &[Wrapper] = &[
    wrapper(
        "sudo",
        "AbBEHiKklLnPSsVv",
        "ugChpUrtDRT",
        &["preserve-env", "login", "shell", "non-interactive", "stdin", "reset-timestamp", "background", "set-home"],
        &["user", "group", "host", "prompt", "other-user", "role", "type", "chdir", "chroot", "close-from"],
        0,
    ),
    wrapper("doas", "Lns", "uC", &[], &[], 0),
    wrapper("env", "i0v", "uCP", &["ignore-environment", "null", "debug"], &["unset", "chdir"], 0),
    wrapper("nice", "", "n", &[], &["adjustment"], 0),
    wrapper("nohup", "", "", &[], &[], 0),
    wrapper("time", "pvaq", "fo", &["portability", "verbose", "append", "quiet"], &["format", "output"], 0),
    wrapper("command", "pvV", "", &[], &[], 0),
    wrapper("exec", "cl", "a", &[], &[], 0),
    wrapper("builtin", "", "", &[], &[], 0),
    wrapper("setsid", "cfw", "", &["ctty", "fork", "wait"], &[], 0),
    wrapper("timeout", "v", "sk", &["preserve-status", "foreground", "verbose"], &["signal", "kill-after"], 1),
    wrapper("stdbuf", "", "ioe", &[], &["input", "output", "error"], 0),
    wrapper("ionice", "t", "cnp", &["ignore"], &["class", "classdata", "pid"], 0),
    wrapper("chroot", "", "", &["skip-chdir"], &["userspec", "groups"], 1),
    wrapper(
        "xargs",
        "0prtx",
        "IinPLdEase",
        &["null", "no-run-if-empty", "verbose", "interactive", "exit", "open-tty"],
        &["replace", "max-args", "max-procs", "max-lines", "delimiter", "eof", "arg-file", "max-chars"],
        0,
    ),
    wrapper(
        "watch",
        "dtbegcxpwqC",
        "n",
        &["differences", "no-title", "beep", "errexit", "chgexit", "color", "exec", "precise"],
        &["interval"],
        0,
    ),
    wrapper(
        "parallel",
        "kvq0",
        "jSaN",
        &["keep-order", "verbose", "quote", "null", "will-cite"],
        &["jobs", "sshlogin", "arg-file"],
        0,
    ),
];

/// Where the wrapper's command starts (`None`: an option it is not known to have).
fn after_wrapper(w: &Wrapper, seg: &[String]) -> Option<usize> {
    let mut i = 1;
    while let Some(word) = seg.get(i) {
        if word == "--" {
            i += 1;
            break;
        }
        if let Some(long) = word.strip_prefix("--") {
            let (name, value) = long.split_once('=').map_or((long, None), |(n, v)| (n, Some(v)));
            if w.long_values.contains(&name) {
                i += 1 + usize::from(value.is_none());
            } else if w.long_flags.contains(&name) {
                i += 1;
            } else {
                return None;
            }
            continue;
        }
        let Some(short) = word.strip_prefix('-').filter(|s| !s.is_empty()) else {
            break;
        };
        // A group of flags (`-iE`) whose last may take a value; a value may be attached (`-n5`).
        let mut skip = 1;
        for (k, c) in short.char_indices() {
            if w.values.contains(c) {
                skip += usize::from(k + c.len_utf8() == short.len());
                break;
            }
            if !w.flags.contains(c) {
                return None;
            }
        }
        i += skip;
    }
    Some((i + w.operands).min(seg.len()))
}

/// A shell's script given on its command line (`sh -c "…"`, `bash -lc "…"`), if any.
fn shell_inner(seg: &[String]) -> Option<String> {
    seg.iter()
        .position(|w| w == "-c" || (w.starts_with('-') && !w.starts_with("--") && w.ends_with('c')))
        .and_then(|p| seg.get(p + 1).cloned())
}

/// Whether a shell or an interpreter reads the code it runs from its standard input: no script or command given
/// (`bash`, `sh -s`, `python3 -`, `node`).
fn reads_stdin(seg: &[String]) -> bool {
    let positional = seg.iter().skip(1).find(|w| !w.starts_with('-') || *w == "-");
    let inline = seg.iter().skip(1).any(|w| {
        matches!(w.as_str(), "-c" | "-e" | "-E" | "-r" | "-p" | "--eval" | "--print" | "-Command" | "-EncodedCommand")
            || (w.starts_with('-') && !w.starts_with("--") && (w.ends_with('c') || w.ends_with('e')))
    });
    !inline && positional.is_none_or(|p| p == "-") || seg.iter().any(|w| w == "-s")
}

/// Resolves one simple command: skips assignments and wrappers, looks into what it runs, and adds its commands to
/// `out`. Returns whether it was read with certainty.
#[allow(clippy::too_many_lines, reason = "one case per program that runs another")]
fn resolve(raw: &Raw, out: &mut Vec<Vec<String>>, depth: usize, windows: bool) -> bool {
    let inner = |text: &str, out: &mut Vec<Vec<String>>| -> bool {
        if depth >= 4 {
            return false;
        }
        let (raws, mut certain) = split_commands(text, windows);
        for raw in &raws {
            certain &= resolve(raw, out, depth + 1, windows);
        }
        certain
    };
    let words = &raw.words;
    let mut i = 0;
    loop {
        while words.get(i).is_some_and(|w| is_assignment(w)) {
            i += 1;
        }
        let Some(first) = words.get(i) else {
            return true;
        };
        let name = program_name(first);
        let Some(w) = WRAPPERS.iter().find(|w| w.name == name) else {
            break;
        };
        let seg = &words[i..];
        // `env -S "cmd args"`: a command line of its own, with what follows it.
        if name == "env"
            && let Some(at) =
                seg.iter().position(|w| w == "-S" || w.starts_with("-S") || w.starts_with("--split-string"))
        {
            let given = seg[at].trim_start_matches("--split-string").trim_start_matches("-S").trim_start_matches('=');
            let (line, rest) = if given.is_empty() {
                (seg.get(at + 1).cloned().unwrap_or_default(), at + 2)
            } else {
                (given.to_owned(), at + 1)
            };
            let all = std::iter::once(line).chain(seg.get(rest..).unwrap_or_default().iter().cloned());
            return inner(&all.collect::<Vec<_>>().join(" "), out);
        }
        let Some(start) = after_wrapper(w, seg) else {
            out.push(seg.to_vec());
            return false;
        };
        if start >= seg.len() {
            // A wrapper with nothing to run (or `xargs` running `echo`): itself.
            out.push(seg.iter().map(|w| program_name(w)).take(1).chain(seg[1..].iter().cloned()).collect());
            return true;
        }
        i += start;
    }
    let mut seg: Vec<String> = words[i..].to_vec();
    // A program named at run time (`$cmd`, `r*`, `/bin/r?`) is not known.
    let named = !seg[0].contains(['$', '*', '?', '[']);
    seg[0] = program_name(&seg[0]);
    let program = seg[0].to_ascii_lowercase();
    let mut certain = true;
    let given = match program.as_str() {
        p if SHELLS.contains(&p) => match shell_inner(&seg) {
            Some(script) => Some(script),
            // A here-string is a word of its own: what the shell runs.
            None if raw.here_string => seg.last().cloned().filter(|_| seg.len() > 1),
            // A heredoc's body is read as commands already; a pipe or a terminal is not seen.
            None if reads_stdin(&seg) && !raw.heredoc => {
                certain = false;
                None
            }
            None => None,
        },
        "eval" => Some(seg[1..].join(" ")),
        "su" | "script" | "flock" => {
            let at = seg.iter().position(|w| w == "-c" || w == "--command");
            match at.and_then(|a| seg.get(a + 1)) {
                Some(cmd) => Some(cmd.clone()),
                None if program == "flock" => {
                    // `flock [-opts] lockfile command…`
                    let rest: Vec<&String> = seg.iter().skip(1).skip_while(|w| w.starts_with('-')).skip(1).collect();
                    (!rest.is_empty()).then(|| rest.iter().map(|w| w.as_str()).collect::<Vec<_>>().join(" "))
                }
                None => {
                    certain = program == "script" && seg.len() <= 2;
                    None
                }
            }
        }
        "ssh" => {
            const VALUES: &str = "bcDEeFIiJLlmOopQRSWw";
            let mut k = 1;
            while let Some(w) = seg.get(k) {
                let Some(opts) = w.strip_prefix('-') else {
                    break;
                };
                let takes = opts.chars().last().is_some_and(|c| VALUES.contains(c)) && opts.len() == 1;
                k += 1 + usize::from(takes);
            }
            let command: Vec<&str> = seg.iter().skip(k + 1).map(String::as_str).collect();
            if command.is_empty() {
                certain = raw.heredoc;
                None
            } else {
                Some(command.join(" "))
            }
        }
        "find" => {
            let mut k = 1;
            while k < seg.len() {
                if matches!(seg[k].as_str(), "-exec" | "-execdir" | "-ok" | "-okdir") {
                    let end = seg[k + 1..].iter().position(|w| w == ";" || w == "+").map_or(seg.len(), |p| k + 1 + p);
                    let cmd = seg[k + 1..end].join(" ");
                    certain &= inner(&cmd, out);
                    k = end;
                }
                k += 1;
            }
            None
        }
        "git" => {
            // Configuration that runs a program: `-c core.sshCommand=…`, an alias that runs one (`alias.x=!…`).
            const RUNS: &[&str] = &[
                "core.sshcommand",
                "core.pager",
                "core.editor",
                "core.fsmonitor",
                "core.hookspath",
                "core.gitproxy",
                "core.askpass",
                "sequence.editor",
                "diff.external",
                "credential.helper",
                "uploadpack.packobjectshook",
                "gpg.program",
                "ssh.variant",
                "protocol.ext.allow",
            ];
            let mut k = 1;
            while let Some(w) = seg.get(k) {
                if w == "-c" {
                    let setting = seg.get(k + 1).map(|s| s.to_ascii_lowercase()).unwrap_or_default();
                    let key = setting.split_once('=').map_or(setting.as_str(), |(k, _)| k);
                    if RUNS.iter().any(|r| key == *r || key.ends_with(".command") || key.starts_with(&format!("{r}.")))
                        || (key.starts_with("alias.") && setting.contains('!'))
                    {
                        certain = false;
                    }
                    k += 2;
                } else if w.starts_with('-') {
                    k += 1;
                } else {
                    break;
                }
            }
            None
        }
        "iex" | "invoke-expression" if seg.len() == 1 => {
            certain = false;
            None
        }
        _ if interpreter(&program) && (reads_stdin(&seg) || raw.heredoc || raw.here_string) => {
            // Code fed on its standard input (a pipe, a heredoc, a here-string): not read.
            certain = false;
            if windows {
                windows_inner(&seg)
            } else {
                None
            }
        }
        _ if windows => windows_inner(&seg),
        _ => None,
    };
    if let Some(script) = given {
        certain &= inner(&script, out);
    }
    out.push(seg);
    certain && named
}

/// The program's name: no directory, and without a Windows program ending (`git.exe`, `npm.cmd`).
#[must_use]
pub fn program_name(word: &str) -> String {
    let name = word.rsplit(['/', '\\']).next().unwrap_or(word);
    let lower = name.to_ascii_lowercase();
    match [".exe", ".cmd", ".bat", ".com"].iter().find(|ext| lower.ends_with(*ext) && name.len() > ext.len()) {
        Some(ext) => name[..name.len() - ext.len()].to_owned(),
        None => name.to_owned(),
    }
}

fn is_assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(name, _)| {
        !name.is_empty()
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            && !name.starts_with(|c: char| c.is_ascii_digit())
    })
}

/// A simple command as written: its words, and whether a heredoc (`<<EOF`) or a here-string (`<<< word`) feeds it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Raw {
    words: Vec<String>,
    heredoc: bool,
    here_string: bool,
}

/// Whether a `{` starts a brace expansion (`{a,b}`, `{1..3}`): a `}` before any space, with a comma or `..` between.
fn brace_expansion(after: &std::iter::Peekable<std::str::Chars<'_>>) -> bool {
    let ahead: String = after.clone().take_while(|c| !c.is_whitespace()).collect();
    ahead.find('}').is_some_and(|end| ahead[..end].contains(',') || ahead[..end].contains(".."))
}

/// Splits a command line into simple commands (lists of words), resolving quotes and escapes, and says whether every
/// quote, backtick and `$(` closed. A heredoc's body is read like any other lines (as commands). `windows`: as cmd and
/// PowerShell read it, where a backslash is part of the word (`C:\Users`) and a backtick escapes the next character.
#[allow(clippy::too_many_lines, reason = "one case per character that means something")]
fn split_commands(command: &str, windows: bool) -> (Vec<Raw>, bool) {
    let mut commands: Vec<Raw> = Vec::new();
    let mut current = Raw::default();
    let mut word = String::new();
    let mut in_word = false;
    let mut certain = true;
    let mut substitutions = 0_usize;
    let mut backticks = 0_usize;
    let mut chars = command.chars().peekable();
    let end_word = |word: &mut String, in_word: &mut bool, current: &mut Raw| {
        if *in_word {
            current.words.push(std::mem::take(word));
            *in_word = false;
        }
    };
    let end_command = |current: &mut Raw, commands: &mut Vec<Raw>| {
        if current.words.is_empty() {
            *current = Raw::default();
        } else {
            commands.push(std::mem::take(current));
        }
    };
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                in_word = true;
                let mut closed = false;
                for q in chars.by_ref() {
                    if q == '\'' {
                        closed = true;
                        break;
                    }
                    word.push(q);
                }
                certain &= closed;
            }
            '"' if windows => {
                in_word = true;
                let mut closed = false;
                while let Some(q) = chars.next() {
                    match q {
                        '"' => {
                            closed = true;
                            break;
                        }
                        '`' => {
                            if let Some(n) = chars.next() {
                                word.push(n);
                            }
                        }
                        _ => word.push(q),
                    }
                }
                certain &= closed;
            }
            '`' if windows => {
                in_word = true;
                match chars.next() {
                    Some('\n') | None => {}
                    Some(n) => word.push(n),
                }
            }
            '\\' if windows => {
                in_word = true;
                word.push('\\');
            }
            '"' => {
                in_word = true;
                let mut closed = false;
                let mut inner = 0_usize;
                while let Some(q) = chars.next() {
                    match q {
                        '"' => {
                            closed = true;
                            break;
                        }
                        '\\' => {
                            if let Some(&n) = chars.peek()
                                && matches!(n, '"' | '\\' | '$' | '`' | '\n')
                            {
                                chars.next();
                                if n != '\n' {
                                    word.push(n);
                                }
                                continue;
                            }
                            word.push('\\');
                        }
                        '$' if chars.peek() == Some(&'(') => {
                            // A command run inside the quotes: its output is a word nobody reads here.
                            certain = false;
                            inner += 1;
                            word.push(q);
                        }
                        '`' => {
                            certain = false;
                            word.push(q);
                        }
                        ')' if inner > 0 => {
                            inner -= 1;
                            word.push(q);
                        }
                        _ => word.push(q),
                    }
                }
                certain &= closed && inner == 0;
            }
            '\\' => {
                in_word = true;
                match chars.next() {
                    Some('\n') | None => {}
                    Some(n) => word.push(n),
                }
            }
            '#' if !in_word => {
                for n in chars.by_ref() {
                    if n == '\n' {
                        break;
                    }
                }
                end_command(&mut current, &mut commands);
            }
            '{' if !windows && brace_expansion(&chars) => {
                // `{r,}m`, `{a..z}`: words the shell makes up.
                certain = false;
                in_word = true;
                word.push(c);
            }
            '$' if !windows && chars.peek() == Some(&'\'') => {
                // `$'\x72m'`: a word the shell decodes.
                certain = false;
                in_word = true;
                word.push(c);
            }
            ';' | '&' | '|' | '\n' | '(' | ')' | '`' | '{' | '}' if !(matches!(c, '{' | '}') && in_word) => {
                if c == '`' {
                    // A command substitution: what it prints becomes words nobody reads here.
                    backticks += 1;
                    certain &= windows;
                }
                if c == ')' && substitutions > 0 {
                    substitutions -= 1;
                }
                end_word(&mut word, &mut in_word, &mut current);
                end_command(&mut current, &mut commands);
            }
            '$' if chars.peek() == Some(&'(') => {
                chars.next();
                substitutions += 1;
                // Its commands are read; what they print becomes words nobody reads here.
                certain = false;
                end_word(&mut word, &mut in_word, &mut current);
                end_command(&mut current, &mut commands);
            }
            '<' | '>' => {
                // A redirection: its target becomes a word of its own (`> .env`). `<<` feeds a heredoc, `<<<` a
                // here-string.
                end_word(&mut word, &mut in_word, &mut current);
                if c == '<' && chars.next_if_eq(&'<').is_some() {
                    if chars.next_if_eq(&'<').is_some() {
                        current.here_string = true;
                    } else {
                        current.heredoc = true;
                        chars.next_if_eq(&'-');
                    }
                }
                while chars.peek().is_some_and(|n| matches!(n, '>' | '&' | '|')) {
                    chars.next();
                }
            }
            c if c.is_whitespace() => end_word(&mut word, &mut in_word, &mut current),
            c => {
                in_word = true;
                word.push(c);
            }
        }
    }
    end_word(&mut word, &mut in_word, &mut current);
    end_command(&mut current, &mut commands);
    let closed = substitutions == 0 && (windows || backticks.is_multiple_of(2));
    (commands, certain && closed)
}
