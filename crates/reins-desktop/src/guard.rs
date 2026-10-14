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
//! path (`.ssh/*`, `.aws/credentials`). A command's arguments are checked against the file patterns too (`cat .env`).
//!
//! The `allow_*` lists are exceptions (`*.pub`, `.env.example`), checked first.

use reins_policy::guard as builtin;
use reins_policy::shell::{command_matches, text_words, words};
pub use reins_policy::shell::{file_matches, glob, segments};
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
        commands: builtin::GIT_HISTORY.commands,
        files: builtin::GIT_HISTORY.files,
    },
    Group {
        id: "deletes",
        label: "Recursive deletes",
        detail: "rm -r, Remove-Item -Recurse, rd /s",
        commands: builtin::DELETES.commands,
        files: builtin::DELETES.files,
    },
    Group {
        id: "infrastructure",
        label: "Infrastructure changes",
        detail: "terraform and tofu apply or destroy, kubectl apply or delete, helm uninstall",
        commands: builtin::INFRASTRUCTURE.commands,
        files: builtin::INFRASTRUCTURE.files,
    },
    Group {
        id: "databases",
        label: "Dropping data",
        detail: "DROP TABLE, DROP DATABASE, DROP SCHEMA, TRUNCATE TABLE",
        commands: builtin::DATABASES.commands,
        files: builtin::DATABASES.files,
    },
    Group {
        id: "publishing",
        label: "Publishing",
        detail: "npm, cargo, PyPI and gem publishes, docker push, deleting GitHub repos and releases",
        commands: builtin::PUBLISHING.commands,
        files: builtin::PUBLISHING.files,
    },
    Group {
        id: "payments",
        label: "Trusting a payment key",
        detail: "reins payments-trust, which lets this computer open the card details of purchases",
        commands: builtin::PAYMENTS.commands,
        files: builtin::PAYMENTS.files,
    },
    Group {
        id: "disks",
        label: "Wiping disks",
        detail: "mkfs, dd of=, format, Format-Volume, Clear-Disk",
        commands: builtin::DISKS.commands,
        files: builtin::DISKS.files,
    },
    Group {
        id: "env-files",
        label: ".env files",
        detail: "Reading or changing .env and .env.* (not .env.example)",
        commands: builtin::ENV_FILES.commands,
        files: builtin::ENV_FILES.files,
    },
    Group {
        id: "keys",
        label: "Keys and certificates",
        detail: "*.pem, *.key, *.p12, keystores, SSH private keys, ~/.ssh",
        commands: builtin::KEYS.commands,
        files: builtin::KEYS.files,
    },
    Group {
        id: "cloud",
        label: "Cloud and cluster credentials",
        detail: "AWS, Google Cloud, Azure, kubeconfig, Docker logins, Terraform state",
        commands: builtin::CLOUD.commands,
        files: builtin::CLOUD.files,
    },
    Group {
        id: "tokens",
        label: "Saved tokens",
        detail: ".netrc, .git-credentials, .npmrc, .pypirc, .vault-token",
        commands: builtin::TOKENS.commands,
        files: builtin::TOKENS.files,
    },
];

/// The group a built-in pattern belongs to.
#[must_use]
pub fn group_of(pattern: &str) -> Option<&'static Group> {
    GROUPS.iter().find(|g| g.commands.contains(&pattern) || g.files.contains(&pattern))
}

impl Group {
    /// Whether what this group covers cannot be undone (history rewritten, files deleted, infrastructure changed,
    /// data dropped, something published, a payment key trusted, a disk wiped): the phone asks for those every time,
    /// with a hold, and Autopilot leaves them to the user in Manual and Auto. The phone reads that from its own copy
    /// of these rules (`reins_policy::guard`), not from what this app says.
    #[must_use]
    pub fn destructive(&self) -> bool {
        builtin::group(self.id).is_some_and(|g| g.destructive)
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
        let segments = segments(command);
        let allowed = |seg: &[String]| self.allow_commands.iter().any(|p| command_matches(p, seg));
        for pattern in self.list(&self.commands, DEFAULT_COMMANDS) {
            if let Some(text) = pattern.strip_prefix("text:") {
                let want = text_words(text);
                if !want.is_empty() && flat.windows(want.len()).any(|w| w == want.as_slice()) {
                    return Some(Match::new("command", pattern, &want.join(" ")));
                }
            } else if let Some(seg) = segments.iter().find(|seg| !allowed(seg) && command_matches(pattern, seg)) {
                return Some(Match::new("command", pattern, &seg.join(" ")));
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
        self.list(&self.files, DEFAULT_FILES).find(|p| file_matches(p, path)).map(|p| Match::new("file", p, path))
    }
}

#[cfg(test)]
mod tests {
    use reins_policy::shell::{decode_powershell, program_name, word_matches};

    use super::*;

    /// The phone reads a hook's question with its own copy of the built-in rules: the same groups, the same labels,
    /// the same allow list.
    #[test]
    fn the_phone_knows_the_built_in_groups() {
        for g in GROUPS {
            let phone = builtin::group(g.id).unwrap_or_else(|| panic!("{} is unknown to the phone", g.id));
            assert_eq!((phone.label, phone.commands, phone.files), (g.label, g.commands, g.files), "{}", g.id);
        }
        assert_eq!(builtin::BUILTIN.len(), GROUPS.len());
        assert_eq!(builtin::ALLOW_FILES, DEFAULT_ALLOW_FILES);
    }

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
        assert_eq!(segments("cat < .env > out"), vec![vec!["cat", ".env", "out"]]);
        assert!(segments("   ").is_empty());
    }

    /// The command from the phone's screenshot: a Python script, fed as a heredoc, that edits a Rust test file. The
    /// guard reads every word of it, so a test's string naming `.config/gcloud/…` asks: a heuristic first line, whose
    /// needless questions Autopilot learns and Bypass answers.
    #[test]
    fn every_word_is_read_heredocs_and_code_too() {
        let screenshot = include_str!("../tests/fixtures/heredoc-python-edit.sh");
        assert_eq!(cmd(screenshot).as_deref(), Some(".config/gcloud/*"));
    }

    #[test]
    fn a_match_says_what_matched() {
        let m = |c: &str| guard().check_command(c).unwrap();
        let force = m("cd app && git push --force origin main");
        assert_eq!(force.matched, "git push --force origin main");
        assert_eq!(force.group().map(|g| g.id), Some("git-history"));
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
