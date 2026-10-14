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

/// Whether a command line can be read with certainty, as a POSIX shell reads it: every quote, backtick and `$(`
/// closes. A heredoc's body is not shell text, so it is skipped (up to its delimiter line), as the shell does.
#[must_use]
pub fn balanced(text: &str) -> bool {
    let mut chars = text.chars().peekable();
    let mut depth = 0_usize;
    let mut heredocs: Vec<String> = Vec::new();
    let mut word_start = true;
    while let Some(c) = chars.next() {
        let at_start = std::mem::replace(&mut word_start, c.is_whitespace() || matches!(c, ';' | '|' | '&' | '('));
        match c {
            '#' if at_start => {
                // A comment, up to the line's end.
                while chars.next_if(|n| *n != '\n').is_some() {}
            }
            '\\' => {
                chars.next();
            }
            '\'' => {
                if !chars.by_ref().any(|q| q == '\'') {
                    return false;
                }
            }
            '"' | '`' => {
                let mut closed = false;
                // `$(` inside double quotes must close inside them.
                let mut inner = 0_usize;
                while let Some(q) = chars.next() {
                    match q {
                        '\\' => {
                            chars.next();
                        }
                        '$' if c == '"' && chars.peek() == Some(&'(') => {
                            chars.next();
                            inner += 1;
                        }
                        ')' if inner > 0 => inner -= 1,
                        q if q == c => {
                            closed = true;
                            break;
                        }
                        _ => {}
                    }
                }
                if !closed || inner > 0 {
                    return false;
                }
            }
            '$' if chars.peek() == Some(&'(') => {
                chars.next();
                depth += 1;
            }
            ')' if depth > 0 => depth -= 1,
            '<' if chars.peek() == Some(&'<') => {
                chars.next();
                if chars.next_if_eq(&'<').is_some() {
                    continue;
                }
                chars.next_if_eq(&'-');
                while chars.next_if(|n| *n == ' ' || *n == '\t').is_some() {}
                let mut delimiter = String::new();
                while let Some(&n) = chars.peek() {
                    if n.is_whitespace() || matches!(n, ';' | '|' | '&' | '<' | '>' | '(' | ')') {
                        break;
                    }
                    chars.next();
                    if !matches!(n, '\'' | '"' | '\\') {
                        delimiter.push(n);
                    }
                }
                if delimiter.is_empty() {
                    return false;
                }
                heredocs.push(delimiter);
            }
            '\n' => {
                for delimiter in std::mem::take(&mut heredocs) {
                    loop {
                        let line: String = chars.by_ref().take_while(|n| *n != '\n').collect();
                        if line.trim_start_matches('\t').trim_end_matches('\r') == delimiter {
                            break;
                        }
                        if chars.peek().is_none() {
                            // A heredoc that never ends.
                            return false;
                        }
                    }
                }
            }
            _ => {}
        }
    }
    depth == 0 && heredocs.is_empty()
}

/// The simple commands in a shell command line, each as its words with the program's name first: quotes and escapes
/// resolved, `;`, `&`, `|`, `&&`, `||`, newlines, parentheses, `$(…)` and backticks separate commands; variable
/// assignments and wrappers (`sudo`, `env`, `nohup`, `time`, `command`, `exec`, `nice`, `doas`) before the program are
/// skipped, and `sh -c "…"` / `bash -c` / `eval` are looked into. Then the commands the line holds when read the
/// Windows way (backslashes are part of words, a backtick escapes; `cmd /c`, `powershell -Command` looked into), as far
/// as they differ.
#[must_use]
pub fn segments(command: &str) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    for raw in split_commands(command, false) {
        resolve(&raw, &mut out, 0, false);
    }
    let mut windows = Vec::new();
    for raw in split_commands(command, true) {
        resolve(&raw, &mut windows, 0, true);
    }
    for seg in windows {
        if !out.contains(&seg) {
            out.push(seg);
        }
    }
    out
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

fn resolve(raw: &[String], out: &mut Vec<Vec<String>>, depth: usize, windows: bool) {
    const WRAPPERS: &[&str] = &["sudo", "doas", "env", "nohup", "time", "command", "exec", "nice", "builtin"];
    let mut i = 0;
    loop {
        while raw.get(i).is_some_and(|w| is_assignment(w)) {
            i += 1;
        }
        let Some(first) = raw.get(i) else {
            return;
        };
        if WRAPPERS.contains(&program_name(first).as_str()) {
            i += 1;
            while raw.get(i).is_some_and(|w| w.starts_with('-')) {
                // `sudo -u root`, `nice -n 5`: an option's value may follow.
                let takes_value = matches!(raw[i].as_str(), "-u" | "-g" | "-n" | "-C" | "-h" | "-p");
                i += if takes_value {
                    2
                } else {
                    1
                };
            }
            continue;
        }
        break;
    }
    let mut seg: Vec<String> = raw[i..].to_vec();
    seg[0] = program_name(&seg[0]);
    if depth < 3 {
        let inner = match seg[0].as_str() {
            "sh" | "bash" | "zsh" | "dash" | "ksh" | "fish" => seg
                .iter()
                .position(|w| w == "-c" || (w.starts_with('-') && !w.starts_with("--") && w.ends_with('c')))
                .and_then(|p| seg.get(p + 1).cloned()),
            "eval" => Some(seg[1..].join(" ")),
            _ if windows => windows_inner(&seg),
            _ => None,
        };
        if let Some(inner) = inner {
            for raw in split_commands(&inner, windows) {
                resolve(&raw, out, depth + 1, windows);
            }
        }
    }
    out.push(seg);
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

/// Splits a command line into simple commands (lists of words), resolving quotes and escapes. `windows`: as cmd and
/// PowerShell read it, where a backslash is part of the word (`C:\Users`) and a backtick escapes the next character.
fn split_commands(command: &str, windows: bool) -> Vec<Vec<String>> {
    let mut commands: Vec<Vec<String>> = Vec::new();
    let mut current: Vec<String> = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut chars = command.chars().peekable();
    let end_word = |word: &mut String, in_word: &mut bool, current: &mut Vec<String>| {
        if *in_word {
            current.push(std::mem::take(word));
            *in_word = false;
        }
    };
    let end_command = |current: &mut Vec<String>, commands: &mut Vec<Vec<String>>| {
        if !current.is_empty() {
            commands.push(std::mem::take(current));
        }
    };
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                in_word = true;
                for q in chars.by_ref() {
                    if q == '\'' {
                        break;
                    }
                    word.push(q);
                }
            }
            '"' if windows => {
                in_word = true;
                while let Some(q) = chars.next() {
                    match q {
                        '"' => break,
                        '`' => {
                            if let Some(n) = chars.next() {
                                word.push(n);
                            }
                        }
                        _ => word.push(q),
                    }
                }
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
                while let Some(q) = chars.next() {
                    match q {
                        '"' => break,
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
                        _ => word.push(q),
                    }
                }
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
            ';' | '&' | '|' | '\n' | '(' | ')' | '`' | '{' | '}' if !(matches!(c, '{' | '}') && in_word) => {
                end_word(&mut word, &mut in_word, &mut current);
                end_command(&mut current, &mut commands);
            }
            '$' if chars.peek() == Some(&'(') => {
                chars.next();
                end_word(&mut word, &mut in_word, &mut current);
                end_command(&mut current, &mut commands);
            }
            '<' | '>' => {
                // A redirection: its target becomes a word of its own (`> .env`).
                end_word(&mut word, &mut in_word, &mut current);
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
    commands
}
