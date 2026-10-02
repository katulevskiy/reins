//! Which commands and files a harness hook asks about (`[guard]` in `config.toml`).
//!
//! Command patterns are shell words: the first names the program (`git`, `/usr/bin/git` and `sudo git` all count), the
//! others must appear among its arguments in that order, not necessarily next to each other (`git push --force` matches
//! `git -C app push origin main --force`). In a word `*` matches any run of characters, and `-x` (one dash, one letter)
//! also matches a group of short options containing it (`rm -r` matches `rm -rf`). A pattern starting with `text:`
//! matches those whole words anywhere in the command, ignoring case and punctuation (`text:drop table`).
//!
//! File patterns: without a `/` a pattern matches the file name (`*.pem`, `.env`); with one it matches the end of the
//! path (`.ssh/*`, `.aws/credentials`). A command's arguments are checked against the file patterns too (`cat .env`).
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
    "uv publish",
    "twine upload",
    "gem push",
    "docker push",
    "gh repo delete",
    "gh release delete",
    "mkfs*",
    "dd of=*",
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
    ".git-credentials",
    ".npmrc",
    ".pypirc",
    ".vault-token",
    "credentials.json",
];

/// Built-in exceptions to the file patterns.
pub const DEFAULT_ALLOW_FILES: &[&str] =
    &["*.pub", "known_hosts", ".env.example", ".env.sample", ".env.template", ".env.dist"];

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
        builtin.iter().copied().chain(extra.iter().map(String::as_str))
    }

    /// The first rule `command` (a shell command line) matches, if any.
    #[must_use]
    pub fn check_command(&self, command: &str) -> Option<Match> {
        let flat = text_words(command);
        let segments = segments(command);
        let allowed = |seg: &[String]| self.allow_commands.iter().any(|p| command_matches(p, seg, None));
        for pattern in self.list(&self.commands, DEFAULT_COMMANDS) {
            let hit = if let Some(text) = pattern.strip_prefix("text:") {
                let want = text_words(text);
                !want.is_empty() && flat.windows(want.len()).any(|w| w == want.as_slice())
            } else {
                segments.iter().any(|seg| !allowed(seg) && command_matches(pattern, seg, None))
            };
            if hit {
                return Some(Match {
                    pattern: pattern.to_owned(),
                    topic: format!("command:{pattern}"),
                });
            }
        }
        // Arguments that name a guarded file (`cat .env`, `cp ~/.ssh/id_ed25519 /tmp`).
        segments.iter().filter(|seg| !allowed(seg)).flat_map(|seg| seg.iter().skip(1)).find_map(|arg| {
            let path = match arg.split_once('=') {
                Some((opt, value)) if opt.starts_with('-') => value,
                _ if arg.starts_with('-') => return None,
                _ => arg.as_str(),
            };
            self.check_file(path)
        })
    }

    /// The first file rule `path` matches, unless an exception covers it.
    #[must_use]
    pub fn check_file(&self, path: &str) -> Option<Match> {
        if path.is_empty() || self.list(&self.allow_files, DEFAULT_ALLOW_FILES).any(|p| file_matches(p, path)) {
            return None;
        }
        self.list(&self.files, DEFAULT_FILES).find(|p| file_matches(p, path)).map(|p| Match {
            pattern: p.to_owned(),
            topic: format!("file:{p}"),
        })
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
    glob(pattern, word)
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
    if !glob(program, name) {
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
/// skipped, and `sh -c "…"` / `bash -c` / `eval` are looked into.
#[must_use]
pub fn segments(command: &str) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    for raw in split_commands(command) {
        resolve(&raw, &mut out, 0);
    }
    out
}

fn resolve(raw: &[String], out: &mut Vec<Vec<String>>, depth: usize) {
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
            _ => None,
        };
        if let Some(inner) = inner {
            for raw in split_commands(&inner) {
                resolve(&raw, out, depth + 1);
            }
        }
    }
    out.push(seg);
}

fn program_name(word: &str) -> String {
    word.rsplit(['/', '\\']).next().unwrap_or(word).to_owned()
}

fn is_assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(name, _)| {
        !name.is_empty()
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            && !name.starts_with(|c: char| c.is_ascii_digit())
    })
}

/// Splits a command line into simple commands (lists of words), resolving quotes and escapes.
fn split_commands(command: &str) -> Vec<Vec<String>> {
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

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(segments("cat < .env > out"), vec![vec!["cat", ".env", "out"]]);
        assert!(segments("   ").is_empty());
    }

    #[test]
    fn the_default_rules_catch_the_dangerous_commands() {
        for (c, rule) in [
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
