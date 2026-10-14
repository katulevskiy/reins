//! Which commands and files a harness hook asks about (`[guard]` in `config.toml`).
//!
//! Command patterns are shell words: the first names the program (`git`, `/usr/bin/git` and `sudo git` all count), the
//! others must appear among its arguments in that order, not necessarily next to each other (`git push --force` matches
//! `git -C app push origin main --force`). In a word `*` matches any run of characters, and `-x` (one dash, one letter)
//! also matches a group of short options containing it (`rm -r` matches `rm -rf`). A pattern starting with `text:`
//! matches those whole words anywhere in the command, ignoring case and punctuation (`text:drop table`).
//!
//! Windows commands too: the program's name matches ignoring case and an `.exe`, `.cmd`, `.bat` or `.com` ending
//! (`C:\Git\cmd\GIT.EXE` is `git`); a cmd switch in a pattern (`/s`) matches ignoring case, also among switches written
//! together (`/S/Q`); a PowerShell parameter in a pattern (a dash and a capital, `-Recurse`) matches ignoring case and
//! any abbreviation PowerShell accepts (`-r`, `-rec`, `-Recurse:$true`). Commands are read as a POSIX shell reads them
//! and also with backslashes as part of the words (cmd, PowerShell: `C:\Users\me\.env`); `cmd /c`, `powershell
//! -Command` and `-EncodedCommand`, `pwsh -c` and `Invoke-Expression` are looked into like `sh -c`.
//!
//! File patterns: without a `/` a pattern matches the file name (`*.pem`, `.env`); with one it matches the end of the
//! path (`.ssh/*`, `.aws/credentials`). The files a command opens are checked against them too: its path arguments
//! and redirections (`cat .env`, `> .env`), not text it is given (`grep .env src`, `git commit -m "fix .env"`, what
//! `echo` prints). A heredoc is read as commands only when a shell runs it; code given to an interpreter (`python3 -c`,
//! `node -e`, a heredoc fed to `python3 -`) is asked about when it opens files and has a string that is exactly a
//! guarded file (`open('.env')`), not for names inside longer strings (code it edits, messages).
//!
//! The `allow_*` lists are exceptions (`*.pub`, `.env.example`), checked first.

use serde::{Deserialize, Serialize};

/// Built-in command patterns (with `defaults = true`).
pub const DEFAULT_COMMANDS: &[&str] = &[
    "git push -f",
    "git push --force*",
    "git push +*",
    "git push -d",
    "git push --delete",
    "git push --mirror",
    "git push --prune",
    "git push :*",
    "git reset --hard",
    "git clean -f",
    "git clean --force",
    "git branch -D",
    "git checkout -f",
    "git filter-branch",
    "git filter-repo",
    "rm -r",
    "rm -R",
    "rm --recursive",
    "terraform apply",
    "terraform destroy",
    "tofu apply",
    "tofu destroy",
    "kubectl apply",
    "kubectl delete",
    "helm uninstall",
    "helm delete",
    "text:drop table",
    "text:drop database",
    "text:drop schema",
    "text:truncate table",
    "npm publish",
    "pnpm publish",
    "yarn publish",
    "yarn npm publish",
    "cargo publish",
    "poetry publish",
    // Trusting a key to open card details is the user's own step (`docs/payments.md`).
    "reins payments-trust",
    "uv publish",
    "twine upload",
    "gem push",
    "docker push",
    "gh repo delete",
    "gh release delete",
    "mkfs*",
    "dd of=*",
    // Windows: PowerShell's Remove-Item and its aliases with -Recurse (`rm -r` above covers `rm -Recurse`), cmd's
    // recursive deletes, and wiping disks.
    "Remove-Item -Recurse",
    "ri -Recurse",
    "del -Recurse",
    "erase -Recurse",
    "rd -Recurse",
    "rmdir -Recurse",
    "rd /s",
    "rmdir /s",
    "del /s",
    "erase /s",
    "format *:",
    "Format-Volume",
    "Clear-Disk",
];

/// Built-in file patterns: secrets and keys.
pub const DEFAULT_FILES: &[&str] = &[
    ".env",
    ".env.*",
    "*.pem",
    "*.key",
    "*.p12",
    "*.pfx",
    "*.jks",
    "*.keystore",
    "*.tfstate",
    "id_rsa",
    "id_dsa",
    "id_ecdsa",
    "id_ed25519",
    ".ssh/*",
    ".aws/credentials",
    ".aws/config",
    ".config/gcloud/*",
    ".azure/*",
    ".kube/config",
    ".docker/config.json",
    ".netrc",
    "_netrc",
    "AppData/Roaming/gcloud/*",
    ".git-credentials",
    ".npmrc",
    ".pypirc",
    ".vault-token",
    "credentials.json",
];

/// Built-in exceptions to the file patterns.
pub const DEFAULT_ALLOW_FILES: &[&str] =
    &["*.pub", "known_hosts", ".env.example", ".env.sample", ".env.template", ".env.dist"];

/// A named part of the built-in rules, which the desktop app (or `disabled_groups`) can turn off on its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Group {
    pub id: &'static str,
    pub label: &'static str,
    pub detail: &'static str,
    pub commands: &'static [&'static str],
    pub files: &'static [&'static str],
}

/// The built-in rules by kind. Together they are exactly [`DEFAULT_COMMANDS`] and [`DEFAULT_FILES`].
pub const GROUPS: &[Group] = &[
    Group {
        id: "git-history",
        label: "Rewriting git history",
        detail: "Force pushes, deleted branches, reset --hard, clean -f, filter-repo",
        commands: &[
            "git push -f",
            "git push --force*",
            "git push +*",
            "git push -d",
            "git push --delete",
            "git push --mirror",
            "git push --prune",
            "git push :*",
            "git reset --hard",
            "git clean -f",
            "git clean --force",
            "git branch -D",
            "git checkout -f",
            "git filter-branch",
            "git filter-repo",
        ],
        files: &[],
    },
    Group {
        id: "deletes",
        label: "Recursive deletes",
        detail: "rm -r, Remove-Item -Recurse, rd /s",
        commands: &[
            "rm -r",
            "rm -R",
            "rm --recursive",
            "Remove-Item -Recurse",
            "ri -Recurse",
            "del -Recurse",
            "erase -Recurse",
            "rd -Recurse",
            "rmdir -Recurse",
            "rd /s",
            "rmdir /s",
            "del /s",
            "erase /s",
        ],
        files: &[],
    },
    Group {
        id: "infrastructure",
        label: "Infrastructure changes",
        detail: "terraform and tofu apply or destroy, kubectl apply or delete, helm uninstall",
        commands: &[
            "terraform apply",
            "terraform destroy",
            "tofu apply",
            "tofu destroy",
            "kubectl apply",
            "kubectl delete",
            "helm uninstall",
            "helm delete",
        ],
        files: &[],
    },
    Group {
        id: "databases",
        label: "Dropping data",
        detail: "DROP TABLE, DROP DATABASE, DROP SCHEMA, TRUNCATE TABLE",
        commands: &["text:drop table", "text:drop database", "text:drop schema", "text:truncate table"],
        files: &[],
    },
    Group {
        id: "publishing",
        label: "Publishing",
        detail: "npm, cargo, PyPI and gem publishes, docker push, deleting GitHub repos and releases",
        commands: &[
            "npm publish",
            "pnpm publish",
            "yarn publish",
            "yarn npm publish",
            "cargo publish",
            "poetry publish",
            "uv publish",
            "twine upload",
            "gem push",
            "docker push",
            "gh repo delete",
            "gh release delete",
        ],
        files: &[],
    },
    Group {
        id: "payments",
        label: "Trusting a payment key",
        detail: "reins payments-trust, which lets this computer open the card details of purchases",
        commands: &["reins payments-trust"],
        files: &[],
    },
    Group {
        id: "disks",
        label: "Wiping disks",
        detail: "mkfs, dd of=, format, Format-Volume, Clear-Disk",
        commands: &["mkfs*", "dd of=*", "format *:", "Format-Volume", "Clear-Disk"],
        files: &[],
    },
    Group {
        id: "env-files",
        label: ".env files",
        detail: "Reading or changing .env and .env.* (not .env.example)",
        commands: &[],
        files: &[".env", ".env.*"],
    },
    Group {
        id: "keys",
        label: "Keys and certificates",
        detail: "*.pem, *.key, *.p12, keystores, SSH private keys, ~/.ssh",
        commands: &[],
        files: &[
            "*.pem",
            "*.key",
            "*.p12",
            "*.pfx",
            "*.jks",
            "*.keystore",
            "id_rsa",
            "id_dsa",
            "id_ecdsa",
            "id_ed25519",
            ".ssh/*",
        ],
    },
    Group {
        id: "cloud",
        label: "Cloud and cluster credentials",
        detail: "AWS, Google Cloud, Azure, kubeconfig, Docker logins, Terraform state",
        commands: &[],
        files: &[
            "*.tfstate",
            ".aws/credentials",
            ".aws/config",
            ".config/gcloud/*",
            ".azure/*",
            ".kube/config",
            ".docker/config.json",
            "AppData/Roaming/gcloud/*",
            "credentials.json",
        ],
    },
    Group {
        id: "tokens",
        label: "Saved tokens",
        detail: ".netrc, .git-credentials, .npmrc, .pypirc, .vault-token",
        commands: &[],
        files: &[".netrc", "_netrc", ".git-credentials", ".npmrc", ".pypirc", ".vault-token"],
    },
];

/// The group a built-in pattern belongs to.
#[must_use]
pub fn group_of(pattern: &str) -> Option<&'static Group> {
    GROUPS.iter().find(|g| g.commands.contains(&pattern) || g.files.contains(&pattern))
}

/// The groups of what cannot be undone (history rewritten, files deleted, infrastructure changed, data dropped,
/// something published, a payment key trusted, a disk wiped): the phone asks for those every time, with a hold, and
/// Autopilot leaves them to the user in Manual and Auto. The other groups (secret files and keys) are an ordinary yes
/// or no that Autopilot may learn.
pub const DESTRUCTIVE_GROUPS: &[&str] =
    &["git-history", "deletes", "infrastructure", "databases", "publishing", "payments", "disks"];

impl Group {
    /// Whether what this group covers cannot be undone (see [`DESTRUCTIVE_GROUPS`]).
    #[must_use]
    pub fn destructive(&self) -> bool {
        DESTRUCTIVE_GROUPS.contains(&self.id)
    }
}

/// What a hook does when the question gets no answer (timeout, phone offline, not logged in).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnNoAnswer {
    /// Refuse the command (the agent is told why).
    #[default]
    Deny,
    /// Leave it to the harness's own permission prompt.
    Ask,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GuardConfig {
    /// Use the built-in rules ([`DEFAULT_COMMANDS`], [`DEFAULT_FILES`], [`DEFAULT_ALLOW_FILES`]) besides these lists.
    pub defaults: bool,
    /// Built-in groups ([`GROUPS`]) left out: what they cover goes through without asking.
    pub disabled_groups: Vec<String>,
    pub commands: Vec<String>,
    pub files: Vec<String>,
    pub allow_commands: Vec<String>,
    pub allow_files: Vec<String>,
    /// How long a hook waits for the answer.
    pub timeout_secs: u64,
    pub on_no_answer: OnNoAnswer,
}

impl Default for GuardConfig {
    fn default() -> Self {
        Self {
            defaults: true,
            disabled_groups: Vec::new(),
            commands: Vec::new(),
            files: Vec::new(),
            allow_commands: Vec::new(),
            allow_files: Vec::new(),
            timeout_secs: 120,
            on_no_answer: OnNoAnswer::Deny,
        }
    }
}

/// Why a command or file is asked about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Match {
    /// The rule that matched (`git push --force*`, `.env`).
    pub pattern: String,
    /// What a standing answer on the phone may cover: `command:<pattern>` or `file:<pattern>`.
    pub topic: String,
    /// What in the command matched it, as the phone shows it: the file (`.env`), the command (`git push --force
    /// origin main`) or the words (`drop table`).
    pub matched: String,
}

impl Match {
    fn new(kind: &str, pattern: &str, matched: &str) -> Self {
        Self {
            pattern: pattern.to_owned(),
            topic: format!("{kind}:{pattern}"),
            matched: shown(matched),
        }
    }

    /// The built-in group of the rule that matched (`None` for the user's own rules).
    #[must_use]
    pub fn group(&self) -> Option<&'static Group> {
        group_of(&self.pattern)
    }
}

/// One line of at most 80 characters.
fn shown(text: &str) -> String {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() <= 80 {
        return line;
    }
    let mut cut: String = line.chars().take(79).collect();
    cut.push('…');
    cut
}

impl GuardConfig {
    pub fn validate(&self) -> Result<(), String> {
        if !(5..=3_600).contains(&self.timeout_secs) {
            return Err("`guard.timeout_secs` must be between 5 and 3600".to_owned());
        }
        for p in self.commands.iter().chain(&self.allow_commands) {
            if p.trim().is_empty() || (p.strip_prefix("text:").is_none() && words(p).is_empty()) {
                return Err(format!("`guard`: the command pattern `{p}` is empty"));
            }
        }
        if let Some(g) = self.disabled_groups.iter().find(|g| !GROUPS.iter().any(|known| known.id == g.as_str())) {
            return Err(format!(
                "`guard.disabled_groups`: `{g}` is not one of {}",
                GROUPS.iter().map(|g| g.id).collect::<Vec<_>>().join(", ")
            ));
        }
        if let Some(p) = self.files.iter().chain(&self.allow_files).find(|p| p.trim().trim_matches('/').is_empty()) {
            return Err(format!("`guard`: the file pattern `{p}` is empty"));
        }
        Ok(())
    }

    fn list<'a>(&'a self, extra: &'a [String], builtin: &'static [&'static str]) -> impl Iterator<Item = &'a str> {
        let builtin: &'static [&'static str] = if self.defaults {
            builtin
        } else {
            &[]
        };
        builtin
            .iter()
            .copied()
            .filter(|p| group_of(p).is_none_or(|g| !self.group_off(g.id)))
            .chain(extra.iter().map(String::as_str))
    }

    /// Whether the built-in group `id` is turned off.
    #[must_use]
    pub fn group_off(&self, id: &str) -> bool {
        self.disabled_groups.iter().any(|g| g == id)
    }

    /// The first rule `command` (a shell command line) matches, if any.
    #[must_use]
    pub fn check_command(&self, command: &str) -> Option<Match> {
        let flat = text_words(command);
        let parsed = parse(command);
        let allowed = |seg: &[String]| self.allow_commands.iter().any(|p| command_matches(p, seg, None));
        for pattern in self.list(&self.commands, DEFAULT_COMMANDS) {
            if let Some(text) = pattern.strip_prefix("text:") {
                let want = text_words(text);
                if !want.is_empty() && flat.windows(want.len()).any(|w| w == want.as_slice()) {
                    return Some(Match::new("command", pattern, &want.join(" ")));
                }
            } else if let Some(c) =
                parsed.commands.iter().find(|c| !allowed(&c.words) && command_matches(pattern, &c.words, None))
            {
                return Some(Match::new("command", pattern, &c.words.join(" ")));
            }
        }
        // The files the commands open: their path operands and redirections (`cat .env`, `cp ~/.ssh/id_ed25519
        // /tmp`, `> .env`), not text they are given (`grep .env src`, `git commit -m "fix .env"`).
        let xargs =
            parsed.commands.iter().any(|c| c.words.first().is_some_and(|p| matches!(p.as_str(), "xargs" | "parallel")));
        let found = parsed
            .commands
            .iter()
            .filter(|c| !allowed(&c.words))
            .flat_map(|c| operands(c, xargs))
            .find_map(|path| self.check_file(path));
        // Code an interpreter runs (`python3 -c …`, a heredoc fed to `node`) that opens a guarded file.
        found.or_else(|| parsed.code.iter().find_map(|(lang, code)| self.check_code(*lang, code)))
    }

    /// The first file rule `path` matches, unless an exception covers it.
    #[must_use]
    pub fn check_file(&self, path: &str) -> Option<Match> {
        if path.is_empty() || self.list(&self.allow_files, DEFAULT_ALLOW_FILES).any(|p| file_matches(p, path)) {
            return None;
        }
        self.list(&self.files, DEFAULT_FILES).find(|p| file_matches(p, path)).map(|p| Match::new("file", p, path))
    }

    /// Code that clearly opens a guarded file: it opens files (`open(`, `readFile`, `File.read`, a subprocess) and
    /// has a string that is exactly such a file (`'.env'`, `"~/.ssh/id_ed25519"`), or a command in a string that
    /// reads one (`os.system("cat .env")`). Names inside longer strings (code being edited, messages) are text.
    fn check_code(&self, lang: Lang, code: &str) -> Option<Match> {
        if !opens_files(lang, code) {
            return None;
        }
        literals(code, lang).iter().map(|l| l.trim()).filter(|l| !l.is_empty() && !l.contains('\n')).find_map(|l| {
            if !l.contains(char::is_whitespace) {
                return self.check_file(l);
            }
            // A command in a string: the files it reads (`cat .env`), or redirects to (`> .env`).
            parse(l)
                .commands
                .iter()
                .flat_map(|c| {
                    if c.words.first().is_some_and(|p| FILE_COMMANDS.contains(&p.as_str())) {
                        operands(c, false)
                    } else {
                        c.redirects.iter().map(String::as_str).collect()
                    }
                })
                .find_map(|path| self.check_file(path))
        })
    }
}

/// Programs whose arguments are files they read or write, for commands found inside code's strings.
const FILE_COMMANDS: &[&str] = &[
    "cat",
    "tac",
    "head",
    "tail",
    "less",
    "more",
    "bat",
    "nl",
    "cut",
    "sort",
    "uniq",
    "wc",
    "diff",
    "cmp",
    "cp",
    "mv",
    "ln",
    "scp",
    "rsync",
    "tar",
    "zip",
    "gzip",
    "base64",
    "xxd",
    "od",
    "hexdump",
    "strings",
    "source",
    ".",
    "sed",
    "awk",
    "grep",
    "egrep",
    "rg",
    "tee",
    "type",
    "copy",
    "Get-Content",
    "gc",
];

/// What in code opens files or runs commands.
const OPENERS: &[&str] = &[
    "open(",
    "open (",
    "fopen",
    "read_text",
    "read_bytes",
    "write_text",
    "write_bytes",
    "readFile",
    "writeFile",
    "appendFile",
    "createReadStream",
    "createWriteStream",
    "readlines",
    "File.",
    "IO.",
    "Path(",
    "pathlib",
    "file_get_contents",
    "file_put_contents",
    "copyfile",
    "shutil.",
    "subprocess",
    "system(",
    "popen",
    "exec(",
    "execSync",
    "execFile",
    "spawn",
    "Deno.read",
    "Deno.open",
    "Bun.file",
    "dotenv",
];

fn opens_files(lang: Lang, code: &str) -> bool {
    OPENERS.iter().any(|o| code.contains(o))
        // Ruby's and Perl's backticks run a command.
        || (lang == Lang::Hash && code.contains('`'))
        || text_words(code).iter().any(|w| matches!(w.as_str(), "cat" | "cp"))
}

/// The language family of an interpreter, for its comments.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Lang {
    /// `#` comments: Python, Ruby, Perl.
    Hash,
    /// `//` and `/* */` comments: JavaScript.
    Slash,
    /// Both: PHP.
    Both,
}

/// The interpreter a program is, if it is one.
fn interpreter(program: &str) -> Option<Lang> {
    let p = program.to_ascii_lowercase();
    if p.starts_with("python") || p.starts_with("pypy") || p.starts_with("perl") || p == "ruby" {
        Some(Lang::Hash)
    } else if matches!(p.as_str(), "node" | "nodejs" | "deno" | "bun") {
        Some(Lang::Slash)
    } else if p == "php" {
        Some(Lang::Both)
    } else {
        None
    }
}

/// The code an interpreter is given on its command line (`python3 -c CODE`, `node -e CODE`, `ruby -e`, `perl -ne`,
/// `php -r`, `deno eval`): the argument's index and the code.
fn inline_code(seg: &[String]) -> Option<(Lang, usize, String)> {
    let (program, args) = seg.split_first()?;
    let lang = interpreter(program)?;
    let p = program.to_ascii_lowercase();
    let short = |w: &str, last: &[char]| {
        w.len() > 1
            && w.starts_with('-')
            && !w.starts_with("--")
            && w[1..].chars().all(|c| c.is_ascii_alphabetic())
            && w.ends_with(last)
    };
    let mut i = 0;
    while i < args.len() {
        let w = args[i].as_str();
        let flag = if p.starts_with("python") || p.starts_with("pypy") {
            short(w, &['c'])
        } else if p.starts_with("perl") {
            short(w, &['e', 'E'])
        } else if p == "ruby" {
            short(w, &['e'])
        } else if p == "php" {
            w == "-r"
        } else if p == "deno" {
            w == "eval"
        } else {
            if let Some(code) = w.strip_prefix("--eval=").or_else(|| w.strip_prefix("--print=")) {
                return Some((lang, i + 1, code.to_owned()));
            }
            matches!(w, "-e" | "--eval" | "-p" | "--print")
        };
        if flag {
            return args.get(i + 1).map(|code| (lang, i + 2, code.clone()));
        }
        // Options with a value; the first other word is the script, and what follows is its own.
        if matches!(w, "-W" | "-X" | "-I" | "-r" | "-M" | "--require" | "--import" | "--loader" | "-C") {
            i += 2;
            continue;
        }
        if !w.starts_with('-') || w == "-m" {
            return None;
        }
        i += 1;
    }
    None
}

/// The string literals of code: quoted with `'`, `"` or a backtick, Python's triple quotes too; comments skipped.
fn literals(code: &str, lang: Lang) -> Vec<String> {
    let chars: Vec<char> = code.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    let skip_to = |from: usize, end: &str| -> usize {
        let end: Vec<char> = end.chars().collect();
        (from..chars.len()).find(|&j| chars[j..].starts_with(&end)).map_or(chars.len(), |j| j + end.len())
    };
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        if (c == '#' && lang != Lang::Slash) || (c == '/' && lang != Lang::Hash && next == Some('/')) {
            i = skip_to(i, "\n");
        } else if c == '/' && lang != Lang::Hash && next == Some('*') {
            i = skip_to(i + 2, "*/");
        } else if matches!(c, '\'' | '"' | '`') {
            if c != '`' && next == Some(c) && chars.get(i + 2) == Some(&c) {
                let quote: String = [c, c, c].iter().collect();
                let end = skip_to(i + 3, &quote);
                out.push(chars[i + 3..end.saturating_sub(3).max(i + 3)].iter().collect());
                i = end;
                continue;
            }
            let mut text = String::new();
            let mut j = i + 1;
            let mut closed = false;
            while j < chars.len() {
                match chars[j] {
                    '\\' if j + 1 < chars.len() => {
                        text.push(chars[j + 1]);
                        j += 2;
                        continue;
                    }
                    q if q == c => {
                        closed = true;
                        break;
                    }
                    q => text.push(q),
                }
                j += 1;
            }
            if closed {
                out.push(text);
                i = j + 1;
            } else {
                // A stray quote (an apostrophe in a comment of another language): not a string.
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    out
}

/// The words of a command that are files it opens: the targets of its redirections, and its arguments but the
/// options and what a few well-known programs take as text (a pattern to search for, a script, a message).
fn operands(c: &Cmd, xargs: bool) -> Vec<&str> {
    let mut out: Vec<&str> = c.redirects.iter().map(String::as_str).collect();
    let Some((program, args)) = c.words.split_first() else {
        return out;
    };
    let text = text_args(program, args, xargs);
    for (arg, is_text) in args.iter().zip(text) {
        if is_text {
            continue;
        }
        match arg.split_once('=') {
            Some((opt, value)) if opt.starts_with('-') => out.push(value),
            _ if arg.starts_with('-') => {}
            _ => out.push(arg),
        }
    }
    out
}

/// Which arguments of a command are text rather than files: `echo`'s, the pattern of `grep` and `rg`, the script of
/// `sed` and `awk`, a commit's message. `xargs` in the line: what is echoed may become a file name.
fn text_args(program: &str, args: &[String], xargs: bool) -> Vec<bool> {
    let mut text = vec![false; args.len()];
    let seg: Vec<String> = std::iter::once(program.to_owned()).chain(args.iter().cloned()).collect();
    if let Some((_, at, _)) = inline_code(&seg) {
        // The code: read for what it opens instead.
        text[at - 1] = true;
    }
    match program.to_ascii_lowercase().as_str() {
        "echo" | "printf" if !xargs => text.fill(true),
        name @ ("grep" | "egrep" | "fgrep" | "zgrep" | "rg" | "ag" | "ack") => {
            first_is_text(args, &mut text, &search_options(name));
        }
        "sed" | "gsed" => first_is_text(
            args,
            &mut text,
            &Options {
                text: &["-e", "--expression"],
                given: &["-f", "--file"],
                value: &["-l", "--line-length"],
            },
        ),
        "awk" | "gawk" | "mawk" | "nawk" => first_is_text(
            args,
            &mut text,
            &Options {
                text: &[],
                given: &["-f", "--file"],
                value: &["-v", "-F", "--assign", "--field-separator"],
            },
        ),
        "git" => git_text(args, &mut text),
        "gh" | "glab" => values_are_text(
            args,
            &mut text,
            &["-m", "--message", "-t", "--title", "-b", "--body", "-n", "--notes", "--description", "--search"],
        ),
        "hg" | "jj" | "svn" => values_are_text(args, &mut text, &["-m", "--message"]),
        "select-string" | "sls" => values_are_text(args, &mut text, &["-pattern"]),
        _ => {}
    }
    text
}

/// How a program that takes its text first (a pattern, a script) reads its options.
struct Options {
    /// The text comes with this option instead (`grep -e PATTERN`).
    text: &'static [&'static str],
    /// The text comes from a file (`grep -f FILE`): no text among the arguments.
    given: &'static [&'static str],
    /// Other options with a value (`grep -m 3`).
    value: &'static [&'static str],
}

fn search_options(name: &str) -> Options {
    Options {
        text: &["-e", "--regexp"],
        given: if name == "rg" {
            &["-f", "--file", "--files", "--type-list"]
        } else {
            &["-f", "--file"]
        },
        value: match name {
            "rg" => &[
                "-A",
                "-B",
                "-C",
                "-g",
                "--glob",
                "--iglob",
                "-t",
                "--type",
                "-T",
                "--type-not",
                "-m",
                "--max-count",
                "-M",
                "--max-columns",
                "-j",
                "--threads",
                "-E",
                "--encoding",
                "-r",
                "--replace",
                "--sort",
                "--sortr",
                "--color",
                "--colors",
                "-d",
                "--max-depth",
                "--max-filesize",
                "--pre",
                "--pre-glob",
                "--type-add",
                "--ignore-file",
                "--path-separator",
                "--context-separator",
                "--engine",
            ],
            "ag" | "ack" => &["-A", "-B", "-C", "-G", "-m", "--ignore", "--ignore-dir", "--max-count", "--context"],
            _ => &["-m", "-A", "-B", "-C", "-d", "-D", "--max-count", "--context", "--label", "--color", "--colour"],
        },
    }
}

/// Marks the program's text: the value of a `text` option, else its first positional argument.
fn first_is_text(args: &[String], text: &mut [bool], o: &Options) {
    let mut given = false;
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if a == "--" {
            if !given && i + 1 < args.len() {
                text[i + 1] = true;
            }
            return;
        }
        let short_group = a.len() > 2 && a.starts_with('-') && !a.starts_with("--");
        let short_text = |t: &&str| t.len() == 2 && short_group;
        if o.text.iter().any(|t| a.starts_with(&format!("{t}=")) || (short_text(t) && a.starts_with(t))) {
            // `--regexp=PATTERN`, `-ePATTERN`.
            text[i] = true;
            given = true;
        } else if o.text.contains(&a) || o.text.iter().any(|t| short_text(t) && a.ends_with(&t[1..])) {
            // `-e PATTERN`, `-ie PATTERN`.
            if let Some(t) = text.get_mut(i + 1) {
                *t = true;
            }
            given = true;
            i += 2;
            continue;
        } else if o.given.contains(&a) {
            // `-f FILE` (the file is read, so checked like any other); `rg --files` searches nothing.
            given = true;
            i += if matches!(a, "-f" | "--file") {
                2
            } else {
                1
            };
            continue;
        } else if o.value.contains(&a) {
            i += 2;
            continue;
        } else if !a.starts_with('-') && !given {
            text[i] = true;
            given = true;
        }
        i += 1;
    }
}

/// Marks the values of these options as text (`-m MESSAGE`, `--title=TITLE`).
fn values_are_text(args: &[String], text: &mut [bool], options: &[&str]) {
    for (i, a) in args.iter().enumerate() {
        let lower = a.to_ascii_lowercase();
        if options.contains(&lower.as_str()) {
            if let Some(t) = text.get_mut(i + 1) {
                *t = true;
            }
        } else if options.iter().any(|o| o.starts_with("--") && lower.starts_with(&format!("{o}="))) {
            text[i] = true;
        }
    }
}

/// git: a commit's, tag's or merge's message, `--grep`, and `git grep`'s pattern.
fn git_text(args: &[String], text: &mut [bool]) {
    let mut i = 0;
    while i < args.len() && args[i].starts_with('-') {
        i += if matches!(args[i].as_str(), "-C" | "-c") {
            2
        } else {
            1
        };
    }
    let Some(sub) = args.get(i) else {
        return;
    };
    let rest = i + 1;
    if sub == "grep" {
        first_is_text(&args[rest..], &mut text[rest..], &search_options("grep"));
        return;
    }
    values_are_text(&args[rest..], &mut text[rest..], &["--grep"]);
    if !matches!(sub.as_str(), "commit" | "tag" | "stash" | "notes" | "merge") {
        return;
    }
    for j in rest..args.len() {
        let a = args[j].as_str();
        let short_group = a.len() > 1 && a.starts_with('-') && !a.starts_with("--");
        if a == "--message" || (short_group && a[1..].chars().all(|c| c.is_ascii_alphabetic()) && a.ends_with('m')) {
            if let Some(t) = text.get_mut(j + 1) {
                *t = true;
            }
        } else if a.starts_with("--message=") || (a.starts_with("-m") && a.len() > 2) {
            text[j] = true;
        }
    }
}

/// The words of `text`, lowercased (runs of letters, digits and `_`).
fn text_words(text: &str) -> Vec<String> {
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

fn word_matches(pattern: &str, word: &str) -> bool {
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
fn command_matches(pattern: &str, seg: &[String], _: Option<()>) -> bool {
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
fn words(text: &str) -> Vec<String> {
    segments(text).into_iter().next().unwrap_or_default()
}

/// The simple commands in a shell command line, each as its words with the program's name first: quotes and escapes
/// resolved, `;`, `&`, `|`, `&&`, `||`, newlines, parentheses, `$(…)` and backticks separate commands; variable
/// assignments and wrappers (`sudo`, `env`, `nohup`, `time`, `command`, `exec`, `nice`, `doas`) before the program are
/// skipped, redirections' targets are not words, and `sh -c "…"` / `bash -c` / `eval` are looked into, as is a heredoc
/// or here-string fed to a shell (`bash <<EOF`, `ssh host <<EOF`, `cat <<EOF | sh`); one fed to anything else is text
/// (an interpreter's code is read for the files it opens, see [`GuardConfig::check_command`]). Then the commands the
/// line holds when read the Windows way (backslashes are part of words, a backtick escapes; `cmd /c`, `powershell
/// -Command` looked into), as far as they differ.
#[must_use]
pub fn segments(command: &str) -> Vec<Vec<String>> {
    parse(command).commands.into_iter().map(|c| c.words).filter(|w| !w.is_empty()).collect()
}

/// A simple command, resolved: the program's name first, then its arguments; and the targets of its redirections.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Cmd {
    words: Vec<String>,
    redirects: Vec<String>,
}

/// A command line, read.
#[derive(Debug, Default)]
struct Parsed {
    commands: Vec<Cmd>,
    /// Code an interpreter runs: given on its command line (`python3 -c …`) or fed to it (`python3 - <<EOF`).
    code: Vec<(Lang, String)>,
}

/// The POSIX reading of `command`, and the Windows one as far as it differs.
fn parse(command: &str) -> Parsed {
    let mut parsed = Parsed::default();
    resolve(split_commands(command, false), &mut parsed, 0, false);
    let mut windows = Parsed::default();
    resolve(split_commands(command, true), &mut windows, 0, true);
    for c in windows.commands {
        if !parsed.commands.contains(&c) {
            parsed.commands.push(c);
        }
    }
    for c in windows.code {
        if !parsed.code.contains(&c) {
            parsed.code.push(c);
        }
    }
    parsed
}

/// The command PowerShell runs for `-EncodedCommand`: base64 of UTF-16LE text.
fn decode_powershell(encoded: &str) -> Option<String> {
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

/// Resolves the commands of a line, then what heredocs and here-strings feed them: commands for a shell (or `ssh`),
/// code for an interpreter, else text.
fn resolve(lexed: Lexed, out: &mut Parsed, depth: usize, windows: bool) {
    let resolved: Vec<Option<Cmd>> =
        lexed.commands.into_iter().map(|raw| resolve_one(raw, out, depth, windows)).collect();
    for fed in lexed.fed {
        let line: Vec<&Cmd> = resolved.get(fed.line.clone()).unwrap_or_default().iter().flatten().collect();
        if line.iter().any(|c| runs_shell(&c.words)) {
            if depth < 3 {
                resolve(split_commands(&fed.text, windows), out, depth + 1, windows);
            }
        } else if let Some(lang) = line.iter().find_map(|c| c.words.first().and_then(|p| interpreter(p))) {
            out.code.push((lang, fed.text));
        }
    }
}

const SHELLS: &[&str] = &["sh", "bash", "zsh", "dash", "ksh", "fish"];

/// A command that runs what it is fed as shell commands: a shell, `ssh`, or one that starts a shell (`docker exec -i c
/// sh`, `kubectl exec -i pod -- bash`).
fn runs_shell(words: &[String]) -> bool {
    words.first().is_some_and(|p| p == "ssh") || words.iter().any(|w| SHELLS.contains(&program_name(w).as_str()))
}

fn resolve_one(raw: Raw, out: &mut Parsed, depth: usize, windows: bool) -> Option<Cmd> {
    const WRAPPERS: &[&str] = &["sudo", "doas", "env", "nohup", "time", "command", "exec", "nice", "builtin"];
    let (mut words, mut redirects) = (Vec::new(), Vec::new());
    for (word, redirect) in raw.words.into_iter().zip(raw.redirect) {
        if redirect {
            redirects.push(word);
        } else {
            words.push(word);
        }
    }
    let mut i = 0;
    loop {
        while words.get(i).is_some_and(|w: &String| is_assignment(w)) {
            i += 1;
        }
        let Some(first) = words.get(i) else {
            break;
        };
        if WRAPPERS.contains(&program_name(first).as_str()) {
            i += 1;
            while words.get(i).is_some_and(|w| w.starts_with('-')) {
                // `sudo -u root`, `nice -n 5`: an option's value may follow.
                let takes_value = matches!(words[i].as_str(), "-u" | "-g" | "-n" | "-C" | "-h" | "-p");
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
    let mut seg: Vec<String> = words.get(i..).unwrap_or_default().to_vec();
    if let Some(first) = seg.first_mut() {
        *first = program_name(first);
    }
    if depth < 3 && !seg.is_empty() {
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
            resolve(split_commands(&inner, windows), out, depth + 1, windows);
        }
        if let Some((lang, _, code)) = inline_code(&seg) {
            out.code.push((lang, code));
        }
    }
    if seg.is_empty() && redirects.is_empty() {
        return None;
    }
    let cmd = Cmd {
        words: seg,
        redirects,
    };
    out.commands.push(cmd.clone());
    Some(cmd)
}

/// The program's name: no directory, and without a Windows program ending (`git.exe`, `npm.cmd`).
fn program_name(word: &str) -> String {
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

/// A simple command as written: its words, each marked when it is the target of a redirection (`> .env`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Raw {
    words: Vec<String>,
    redirect: Vec<bool>,
}

/// What a heredoc (its body) or a here-string feeds the commands of its line on their standard input.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Fed {
    text: String,
    /// The commands of the line it was written on.
    line: std::ops::Range<usize>,
}

/// A command line split into simple commands, and what is fed to them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Lexed {
    commands: Vec<Raw>,
    fed: Vec<Fed>,
}

/// Splits a command line into simple commands (lists of words), resolving quotes and escapes; heredoc bodies and
/// here-strings are kept apart, not read as commands. `windows`: as cmd and PowerShell read it, where a backslash is
/// part of the word (`C:\Users`) and a backtick escapes the next character.
fn split_commands(command: &str, windows: bool) -> Lexed {
    let mut l = Lexer {
        chars: command.chars().peekable(),
        out: Lexed::default(),
        current: Raw::default(),
        word: String::new(),
        in_word: false,
        redirect_next: false,
        here_string_next: false,
        heredocs: Vec::new(),
        open_fed: Vec::new(),
        line_start: 0,
    };
    while let Some(c) = l.chars.next() {
        match c {
            '\'' => {
                l.in_word = true;
                for q in l.chars.by_ref() {
                    if q == '\'' {
                        break;
                    }
                    l.word.push(q);
                }
            }
            '"' if windows => {
                l.in_word = true;
                while let Some(q) = l.chars.next() {
                    match q {
                        '"' => break,
                        '`' => {
                            if let Some(n) = l.chars.next() {
                                l.word.push(n);
                            }
                        }
                        _ => l.word.push(q),
                    }
                }
            }
            '`' if windows => {
                l.in_word = true;
                match l.chars.next() {
                    Some('\n') | None => {}
                    Some(n) => l.word.push(n),
                }
            }
            '\\' if windows => {
                l.in_word = true;
                l.word.push('\\');
            }
            '"' => {
                l.in_word = true;
                while let Some(q) = l.chars.next() {
                    match q {
                        '"' => break,
                        '\\' => {
                            if let Some(&n) = l.chars.peek()
                                && matches!(n, '"' | '\\' | '$' | '`' | '\n')
                            {
                                l.chars.next();
                                if n != '\n' {
                                    l.word.push(n);
                                }
                                continue;
                            }
                            l.word.push('\\');
                        }
                        _ => l.word.push(q),
                    }
                }
            }
            '\\' => {
                l.in_word = true;
                match l.chars.next() {
                    Some('\n') | None => {}
                    Some(n) => l.word.push(n),
                }
            }
            '#' if !l.in_word => {
                // A comment, up to the line's end (which still ends the line: heredoc bodies start after it).
                while l.chars.next_if(|n| *n != '\n').is_some() {}
            }
            '\n' => l.end_line(),
            '&' if l.chars.peek() == Some(&'>') => {
                // `&> file`: both outputs to the file.
                l.end_word();
                l.chars.next();
                while l.chars.next_if(|n| *n == '>').is_some() {}
                l.redirect_next = true;
            }
            ';' | '&' | '|' | '(' | ')' | '`' | '{' | '}' if !(matches!(c, '{' | '}') && l.in_word) => {
                l.end_word();
                l.end_command();
            }
            '$' if l.chars.peek() == Some(&'(') => {
                l.chars.next();
                l.end_word();
                l.end_command();
            }
            '<' if l.chars.peek() == Some(&'<') => {
                l.end_word();
                l.chars.next();
                if l.chars.next_if_eq(&'<').is_some() {
                    l.here_string_next = true;
                } else {
                    let tabs = l.chars.next_if_eq(&'-').is_some();
                    while l.chars.next_if(|n| *n == ' ' || *n == '\t').is_some() {}
                    let delimiter = l.delimiter();
                    if !delimiter.is_empty() {
                        l.heredocs.push((delimiter, tabs));
                    }
                }
            }
            '<' | '>' => {
                // A redirection: its target is the next word (`> .env`, `2>&1`, `< key.pem`).
                l.end_word();
                while l.chars.next_if(|n| matches!(n, '>' | '&' | '|')).is_some() {}
                l.redirect_next = true;
            }
            c if c.is_whitespace() => l.end_word(),
            c => {
                l.in_word = true;
                l.word.push(c);
            }
        }
    }
    l.end_line();
    l.out
}

struct Lexer<'a> {
    chars: std::iter::Peekable<std::str::Chars<'a>>,
    out: Lexed,
    current: Raw,
    word: String,
    in_word: bool,
    /// The next word is a redirection's target.
    redirect_next: bool,
    /// The next word is a here-string (`<<< word`).
    here_string_next: bool,
    /// Heredocs whose bodies start on the next line: the delimiter, and whether leading tabs are stripped (`<<-`).
    heredocs: Vec<(String, bool)>,
    /// Here-strings of the current line (indices into `out.fed`).
    open_fed: Vec<usize>,
    /// The first command of the current line.
    line_start: usize,
}

impl Lexer<'_> {
    fn end_word(&mut self) {
        if !self.in_word {
            return;
        }
        self.in_word = false;
        let word = std::mem::take(&mut self.word);
        if std::mem::take(&mut self.here_string_next) {
            self.open_fed.push(self.out.fed.len());
            self.out.fed.push(Fed {
                text: word,
                line: self.line_start..self.line_start,
            });
        } else {
            self.current.words.push(word);
            self.current.redirect.push(std::mem::take(&mut self.redirect_next));
        }
    }

    fn end_command(&mut self) {
        self.redirect_next = false;
        if !self.current.words.is_empty() {
            self.out.commands.push(std::mem::take(&mut self.current));
        }
    }

    /// The line's commands are complete: what its here-strings feed is known, and its heredocs' bodies follow.
    fn end_line(&mut self) {
        self.end_word();
        self.end_command();
        self.here_string_next = false;
        let end = self.out.commands.len();
        let line = self.line_start..end;
        for i in std::mem::take(&mut self.open_fed) {
            self.out.fed[i].line = line.clone();
        }
        for (delimiter, tabs) in std::mem::take(&mut self.heredocs) {
            let text = self.body(&delimiter, tabs);
            self.out.fed.push(Fed {
                text,
                line: line.clone(),
            });
        }
        self.line_start = end;
    }

    /// A heredoc's body: the lines up to the one that is its delimiter (or the end).
    fn body(&mut self, delimiter: &str, tabs: bool) -> String {
        let mut body = String::new();
        loop {
            let mut line = String::new();
            let mut ended = false;
            for c in self.chars.by_ref() {
                if c == '\n' {
                    ended = true;
                    break;
                }
                line.push(c);
            }
            let bare = if tabs {
                line.trim_start_matches('\t')
            } else {
                line.as_str()
            };
            if bare.trim_end_matches('\r') == delimiter || (!ended && line.is_empty()) {
                return body;
            }
            body.push_str(bare);
            body.push('\n');
            if !ended {
                return body;
            }
        }
    }

    /// A heredoc's delimiter word, its quotes removed (`'EOF'`, `"EOF"`, `\EOF`).
    fn delimiter(&mut self) -> String {
        let mut d = String::new();
        while let Some(&c) = self.chars.peek() {
            match c {
                '\'' | '"' => {
                    self.chars.next();
                    for q in self.chars.by_ref() {
                        if q == c {
                            break;
                        }
                        d.push(q);
                    }
                }
                '\\' => {
                    self.chars.next();
                    if let Some(n) = self.chars.next() {
                        d.push(n);
                    }
                }
                c if c.is_whitespace() || matches!(c, ';' | '|' | '&' | '<' | '>' | '(' | ')') => break,
                c => {
                    self.chars.next();
                    d.push(c);
                }
            }
        }
        d
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_groups_are_exactly_the_built_in_rules() {
        let mut commands: Vec<&str> = GROUPS.iter().flat_map(|g| g.commands.iter().copied()).collect();
        let mut files: Vec<&str> = GROUPS.iter().flat_map(|g| g.files.iter().copied()).collect();
        let (mut want_c, mut want_f) = (DEFAULT_COMMANDS.to_vec(), DEFAULT_FILES.to_vec());
        for v in [&mut commands, &mut files, &mut want_c, &mut want_f] {
            v.sort_unstable();
        }
        assert_eq!(commands, want_c);
        assert_eq!(files, want_f);
        let mut ids: Vec<&str> = GROUPS.iter().map(|g| g.id).collect();
        ids.dedup();
        assert_eq!(ids.len(), GROUPS.len());
    }

    #[test]
    fn a_disabled_group_lets_its_commands_and_files_through() {
        let mut g = GuardConfig::default();
        assert!(g.check_command("npm publish").is_some());
        assert!(g.check_file("/app/.env").is_some());
        g.disabled_groups = vec!["publishing".to_owned(), "env-files".to_owned()];
        g.validate().unwrap();
        assert!(g.check_command("npm publish").is_none());
        assert!(g.check_file("/app/.env").is_none());
        // The other groups and the user's own rules still ask.
        assert!(g.check_command("git push --force").is_some());
        g.commands.push("npm publish".to_owned());
        assert!(g.check_command("npm publish").is_some());
        g.disabled_groups.push("nope".to_owned());
        assert!(g.validate().unwrap_err().contains("nope"));
    }

    fn guard() -> GuardConfig {
        GuardConfig::default()
    }

    fn cmd(c: &str) -> Option<String> {
        guard().check_command(c).map(|m| m.pattern)
    }

    #[test]
    fn shell_lines_split_into_simple_commands() {
        assert_eq!(
            segments("cd app && FOO=1 sudo -u root /usr/bin/git push 'origin' \"ma in\"; echo done | tee x"),
            vec![vec!["cd", "app"], vec!["git", "push", "origin", "ma in"], vec!["echo", "done"], vec!["tee", "x"],]
        );
        assert_eq!(
            segments("bash -lc 'rm -rf build'"),
            vec![vec!["rm", "-rf", "build"], vec!["bash", "-lc", "rm -rf build"]]
        );
        assert_eq!(
            segments("echo $(git reset --hard) # git push -f"),
            vec![vec!["echo"], vec!["git", "reset", "--hard"]]
        );
        // Redirections' targets are files, not arguments.
        assert_eq!(segments("cat < .env > out"), vec![vec!["cat"]]);
        assert!(segments("   ").is_empty());
        // A heredoc's body is what it feeds: text for `cat`, commands for a shell.
        assert_eq!(
            segments("cat <<'EOF' > notes.md # notes\nrm -rf /\nEOF\necho done"),
            vec![vec!["cat"], vec!["echo", "done"]]
        );
        assert_eq!(segments("bash <<EOF\nrm -rf build\nEOF"), vec![vec!["bash"], vec!["rm", "-rf", "build"]]);
        assert_eq!(
            segments("cat <<-EOF | sh\n\trm -rf x\n\tEOF\nls"),
            vec![vec!["cat"], vec!["sh"], vec!["ls"], vec!["rm", "-rf", "x"]]
        );
        assert_eq!(segments("bash <<< 'rm -rf x'"), vec![vec!["bash"], vec!["rm", "-rf", "x"]]);
        // An unterminated heredoc runs to the end.
        assert_eq!(segments("ssh host <<EOF\nrm -rf /srv"), vec![vec!["ssh", "host"], vec!["rm", "-rf", "/srv"]]);
    }

    /// The command from the phone's screenshot: a Python script, fed as a heredoc, that edits a Rust test file whose
    /// code names `.config/gcloud/…`, `.env` and keys. Nothing in it opens those files.
    #[test]
    fn code_edited_through_a_heredoc_is_not_a_file_access() {
        let screenshot = include_str!("../tests/fixtures/heredoc-python-edit.sh");
        assert!(screenshot.contains(".config/gcloud/application_default_credentials.json"));
        assert_eq!(guard().check_command(screenshot), None);
        // As it was read before: the Rust code in the Python strings, word by word, as shell commands.
        assert!(screenshot.lines().any(|line| guard().check_command(line).is_some()));
    }

    #[test]
    fn interpreters_that_open_a_guarded_file_are_asked() {
        for (c, rule, matched) in [
            ("python3 -c \"print(open('.env').read())\"", ".env", ".env"),
            (
                "python3 - <<'EOF'\nimport os\np = os.path.expanduser('~/.aws/credentials')\nprint(open(p).read())\nEOF",
                ".aws/credentials",
                "~/.aws/credentials",
            ),
            ("python3 <<< \"print(open('.env').read())\"", ".env", ".env"),
            ("cat <<EOF | python3\nwith open('id_ed25519') as f: print(f.read())\nEOF", "id_ed25519", "id_ed25519"),
            ("python3 -c \"import os; os.system('cat .env.production')\"", ".env.*", ".env.production"),
            ("node -e \"console.log(require('fs').readFileSync('.env', 'utf8'))\"", ".env", ".env"),
            ("node --eval=\"require('fs').readFileSync(process.env.HOME + '/.ssh/id_rsa')\"", "id_rsa", "/.ssh/id_rsa"),
            ("ruby -e 'puts File.read(\"~/.ssh/id_ed25519\")'", "id_ed25519", "~/.ssh/id_ed25519"),
            ("perl -e 'print `cat ~/.netrc`'", ".netrc", "~/.netrc"),
            ("perl -ne 'print' .env", ".env", ".env"),
        ] {
            let m = guard().check_command(c).unwrap_or_else(|| panic!("not asked: {c}"));
            assert_eq!((m.pattern.as_str(), m.matched.as_str()), (rule, matched), "{c}");
        }
        for c in [
            // Named, not opened.
            "python3 -c \"print('.env')\"",
            "python3 -c \"import sys; print(sys.argv)\" -- docs",
            // Inside longer text.
            "python3 -c \"open('notes.md', 'w').write('Never commit .env or id_rsa')\"",
            "node -e \"fs.writeFileSync('README.md', 'Copy .env.example to .env')\"",
            // In a comment.
            "python3 - <<'EOF'\n# don't read '.env' here\nprint(open('notes.md').read())\nEOF",
        ] {
            assert_eq!(cmd(c), None, "{c}");
        }
    }

    #[test]
    fn text_given_to_a_command_is_not_a_file() {
        for c in [
            "git commit -m \"mention .env\"",
            "git commit -am .env",
            "git commit --message=.env",
            "git log --grep=.env",
            "git grep -n id_rsa -- src",
            "grep -rn '.env' src",
            "grep -e .env -e id_rsa -r src",
            "rg 'id_rsa'",
            "rg -n --hidden '\\.env' crates",
            "sed -n '/.env/p' notes.md",
            "awk '/.env/' notes.md",
            "echo .env >> .gitignore",
            "printf '%s\\n' .env.local >> .gitignore",
            "gh pr create --title 'Load .env' --body 'Reads ~/.ssh/id_rsa no more'",
            "cat > notes.md <<'EOF'\nRun cat .env and rm -rf build\nEOF",
        ] {
            assert_eq!(cmd(c), None, "{c}");
        }
        for (c, rule) in [
            ("cat .env", ".env"),
            ("cat \".env\"", ".env"),
            ("cp ~/.ssh/id_rsa /tmp/x", "id_rsa"),
            ("sed -i 's/a/b/' .env", ".env"),
            ("sed -i -e 's/a/b/' .env.local", ".env.*"),
            ("awk '{print}' .env", ".env"),
            ("grep KEY .env", ".env"),
            ("grep -f .env notes.md", ".env"),
            ("rg -g '.env' KEY", ".env"),
            ("git add .env", ".env"),
            ("git commit -F .env", ".env"),
            ("echo .env | xargs cat", ".env"),
            ("echo SECRET=1 &> .env", ".env"),
            ("diff <(cat .env) .env.example", ".env"),
            ("bash <<'EOF'\ncat ~/.ssh/id_ed25519\nEOF", "id_ed25519"),
            ("bash <<< 'cat .env'", ".env"),
        ] {
            assert_eq!(cmd(c).as_deref(), Some(rule), "{c}");
        }
    }

    #[test]
    fn a_match_says_what_matched() {
        let m = |c: &str| guard().check_command(c).unwrap();
        let force = m("cd app && git push --force origin main");
        assert_eq!(
            (force.matched.as_str(), force.group().map(|g| g.id)),
            ("git push --force origin main", Some("git-history"))
        );
        assert!(force.group().unwrap().destructive());
        let env = m("cat ~/app/.env");
        assert_eq!((env.matched.as_str(), env.group().map(|g| g.id)), ("~/app/.env", Some("env-files")));
        assert!(!env.group().unwrap().destructive());
        assert_eq!(m("psql -c 'DROP   TABLE users'").matched, "drop table");
        let long = m(&format!("rm -rf {}", "build/".repeat(30)));
        assert!(long.matched.ends_with('…') && long.matched.chars().count() == 80, "{}", long.matched);
        let own = GuardConfig {
            commands: vec!["make deploy".to_owned()],
            ..GuardConfig::default()
        };
        assert_eq!(own.check_command("make -j4 deploy").unwrap().group(), None, "the user's own rule");
        for g in GROUPS {
            assert_eq!(g.destructive(), g.files.is_empty(), "{}: commands cannot be undone, files are secrets", g.id);
        }
    }

    #[test]
    fn the_payment_key_this_computer_trusts_is_a_guarded_file() {
        let g = GuardConfig::default();
        for f in ["/home/me/.local/state/reins/phone-payments.key", "/home/me/.local/state/reins/identity.key"] {
            assert!(g.check_file(f).is_some(), "{f}");
        }
        assert!(g.check_command("echo abc > ~/.local/state/reins/phone-payments.key").is_some());
    }

    #[test]
    fn the_default_rules_catch_the_dangerous_commands() {
        for (c, rule) in [
            ("reins payments-trust", "reins payments-trust"),
            ("/usr/local/bin/reins payments-trust", "reins payments-trust"),
            ("sh -c 'reins payments-trust'", "reins payments-trust"),
            ("git push --force origin main", "git push --force*"),
            ("git -C repo push -fu origin x", "git push -f"),
            ("git push --force-with-lease", "git push --force*"),
            ("git push origin +main", "git push +*"),
            ("git push origin :old", "git push :*"),
            ("git push origin --delete old", "git push --delete"),
            ("git reset --hard HEAD~3", "git reset --hard"),
            ("rm -rf /tmp/x", "rm -r"),
            ("sudo rm -fR /", "rm -R"),
            ("terraform -chdir=infra apply -auto-approve", "terraform apply"),
            ("kubectl delete pod x", "kubectl delete"),
            ("psql -c 'DROP   TABLE users'", "text:drop table"),
            ("cargo publish -p x", "cargo publish"),
            ("npm publish", "npm publish"),
            ("sh -c \"git push -f\"", "git push -f"),
            ("cat .env", ".env"),
            ("cp ~/.ssh/id_ed25519 /tmp/k", "id_ed25519"),
            ("cat ~/.ssh/config", ".ssh/*"),
            ("docker run --env-file=.env.local img", ".env.*"),
            ("echo x > secrets/server.pem", "*.pem"),
        ] {
            assert_eq!(cmd(c).as_deref(), Some(rule), "{c}");
        }
    }

    #[test]
    fn ordinary_commands_are_not_asked_about() {
        for c in [
            "git push origin main",
            "git commit -m 'push --force later'",
            "git log --grep=reset",
            "rm build/out.o",
            "rm --force x",
            "ls -la",
            "cargo build --release",
            "terraform plan",
            "kubectl get pods",
            "cat .env.example",
            "cat ~/.ssh/id_ed25519.pub",
            "echo 'drop tables are furniture'",
            "npm install",
        ] {
            assert_eq!(cmd(c), None, "{c}");
        }
    }

    #[test]
    fn windows_commands_are_caught_too() {
        let encoded = |s: &str| {
            let bytes: Vec<u8> = s.encode_utf16().flat_map(u16::to_le_bytes).collect();
            data_encoding::BASE64.encode(&bytes)
        };
        let hidden = format!("powershell.exe -NoProfile -EncodedCommand {}", encoded("Remove-Item -Recurse C:\\src"));
        for (c, rule) in [
            ("Remove-Item -Recurse -Force C:\\Users\\me\\src", "Remove-Item -Recurse"),
            ("remove-item .\\build -rec", "Remove-Item -Recurse"),
            ("Remove-Item -Path build -Recurse:$true", "Remove-Item -Recurse"),
            ("ri build -r", "ri -Recurse"),
            ("rm -Recurse -Force build", "rm -r"),
            ("rd /s /q C:\\work", "rd /s"),
            ("RMDIR /S/Q build", "rmdir /s"),
            ("del /f /s /q *.log", "del /s"),
            ("cmd /c \"rd /s /q build\"", "rd /s"),
            ("cmd.exe /C del /S x", "del /s"),
            ("pwsh -c \"Remove-Item -Recurse x\"", "Remove-Item -Recurse"),
            (hidden.as_str(), "Remove-Item -Recurse"),
            ("iex 'Remove-Item x -Recurse'", "Remove-Item -Recurse"),
            ("format D: /q", "format *:"),
            ("Format-Volume -DriveLetter D", "Format-Volume"),
            ("& \"C:\\Program Files\\Git\\cmd\\git.exe\" push --force", "git push --force*"),
            ("GIT.EXE push -f origin main", "git push -f"),
            ("npm.cmd publish", "npm publish"),
            ("type C:\\Users\\me\\project\\.env", ".env"),
            ("Get-Content $env:USERPROFILE\\.ssh\\id_ed25519", "id_ed25519"),
            ("copy C:\\Users\\me\\.aws\\credentials C:\\tmp", ".aws/credentials"),
            ("type %USERPROFILE%\\_netrc", "_netrc"),
            ("Remove-Item `\n  -Recurse build", "Remove-Item -Recurse"),
        ] {
            assert_eq!(cmd(c).as_deref(), Some(rule), "{c}");
        }
        for c in [
            "Remove-Item build.log",
            "Remove-Item -Force x",
            "rd build",
            "del /q x.tmp",
            "dir /s",
            "Get-ChildItem -Recurse",
            "git push origin main",
            "type C:\\Users\\me\\.env.example",
            "Get-Content C:\\Users\\me\\.ssh\\id_ed25519.pub",
            "powershell -Command \"Get-ChildItem\"",
            "cmd /c dir",
        ] {
            assert_eq!(cmd(c), None, "{c}");
        }
        let g = guard();
        assert_eq!(
            g.check_file("C:\\Users\\me\\AppData\\Roaming\\gcloud\\credentials.db").map(|m| m.pattern),
            Some("AppData/Roaming/gcloud/*".to_owned())
        );
    }

    #[test]
    fn windows_switches_and_parameters_match_the_windows_way() {
        assert!(word_matches("/s", "/S"));
        assert!(word_matches("/s", "/q/s"));
        assert!(!word_matches("/s", "/q"));
        assert!(!word_matches("/s", "s"));
        assert!(word_matches("-Recurse", "-r"));
        assert!(word_matches("-Recurse", "-RECURSE"));
        assert!(word_matches("-Recurse", "-recurse:$false"));
        assert!(!word_matches("-Recurse", "-Recursive"));
        assert!(!word_matches("-Recurse", "--recurse"));
        assert!(!word_matches("-Recurse", "-"));
        // Unix patterns keep their case.
        assert!(!word_matches("-D", "-d"));
        assert_eq!(program_name("C:\\Git\\cmd\\git.exe"), "git");
        assert_eq!(program_name("/usr/bin/git"), "git");
        assert_eq!(program_name(".exe"), ".exe");
        assert_eq!(decode_powershell("not base64!"), None);
    }

    #[test]
    fn files_match_by_name_or_path_end() {
        let g = guard();
        for (path, rule) in [
            (".env", ".env"),
            ("/home/me/app/.env", ".env"),
            ("app/.env.production", ".env.*"),
            ("/home/me/.ssh/config", ".ssh/*"),
            ("C:\\Users\\me\\.aws\\credentials", ".aws/credentials"),
            ("certs/tls.key", "*.key"),
            ("/home/me/.ssh/id_rsa", "id_rsa"),
            ("./id_ed25519", "id_ed25519"),
        ] {
            assert_eq!(g.check_file(path).map(|m| m.topic), Some(format!("file:{rule}")), "{path}");
        }
        for path in ["src/main.rs", ".env.example", "/home/me/.ssh/known_hosts", "/home/me/.ssh/id_rsa.pub", "", "env"]
        {
            assert_eq!(g.check_file(path), None, "{path}");
        }
    }

    #[test]
    fn the_config_adds_rules_and_exceptions_or_drops_the_defaults() {
        let g: GuardConfig = toml::from_str(
            "commands = [\"make deploy\", \"text:shutdown\"]\nfiles = [\"*.sqlite\"]\nallow_commands = [\"git push --force* origin feature/*\"]\nallow_files = [\"test.key\"]\n",
        )
        .unwrap();
        g.validate().unwrap();
        assert_eq!(g.check_command("make -j4 deploy").unwrap().topic, "command:make deploy");
        assert_eq!(g.check_command("sudo SHUTDOWN now").unwrap().pattern, "text:shutdown");
        assert_eq!(g.check_file("db/app.sqlite").unwrap().pattern, "*.sqlite");
        assert_eq!(g.check_file("fixtures/test.key"), None);
        assert_eq!(g.check_command("git push --force-with-lease origin feature/x"), None);
        assert!(g.check_command("git push --force origin main").is_some());
        let none = GuardConfig {
            defaults: false,
            ..GuardConfig::default()
        };
        assert_eq!(none.check_command("rm -rf / && cat .env"), None);
        assert_eq!(none.check_file(".env"), None);
        assert!(
            GuardConfig {
                commands: vec!["  ".into()],
                ..GuardConfig::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            GuardConfig {
                timeout_secs: 1,
                ..GuardConfig::default()
            }
            .validate()
            .is_err()
        );
        assert!(toml::from_str::<GuardConfig>("nope = 1").is_err());
    }

    #[test]
    fn globs_and_option_groups() {
        assert!(glob("*.pem", "a.pem"));
        assert!(glob("a*b*c", "aXbYc"));
        assert!(!glob("a*b", "ac"));
        assert!(glob("*", ""));
        assert!(word_matches("-r", "-rf"));
        assert!(word_matches("-r", "-r"));
        assert!(!word_matches("-r", "--recursive"));
        assert!(!word_matches("-r", "-x=r"));
    }
}
