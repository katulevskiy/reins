//! The desktop app guard's built-in groups of rules, and the phone's own reading of a hook's question with them.
//!
//! The desktop app asks the phone before a command or a file a rule matches, and says which rule. That label is the
//! computer's word, and the computer is where the AI runs: the phone does not trust it to decide how a question is
//! asked. It reads everything the question carries with these rules itself ([`read`]); what cannot
//! be undone is asked every time, held to approve and left to the user by Autopilot, whatever the label says.

use crate::shell::{command_matches, file_matches, segments, text_words};

/// A built-in group of rules.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BuiltinGroup {
    pub id: &'static str,
    pub label: &'static str,
    /// What it covers cannot be undone (history rewritten, files deleted, infrastructure changed, data dropped,
    /// something published, a payment key trusted, a disk wiped). The others are secret files and keys.
    pub destructive: bool,
    pub commands: &'static [&'static str],
    pub files: &'static [&'static str],
}

pub const GIT_HISTORY: BuiltinGroup = BuiltinGroup {
    id: "git-history",
    label: "Rewriting git history",
    destructive: true,
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
};

pub const DELETES: BuiltinGroup = BuiltinGroup {
    id: "deletes",
    label: "Recursive deletes",
    destructive: true,
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
};

pub const INFRASTRUCTURE: BuiltinGroup = BuiltinGroup {
    id: "infrastructure",
    label: "Infrastructure changes",
    destructive: true,
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
};

pub const DATABASES: BuiltinGroup = BuiltinGroup {
    id: "databases",
    label: "Dropping data",
    destructive: true,
    commands: &["text:drop table", "text:drop database", "text:drop schema", "text:truncate table"],
    files: &[],
};

pub const PUBLISHING: BuiltinGroup = BuiltinGroup {
    id: "publishing",
    label: "Publishing",
    destructive: true,
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
};

pub const PAYMENTS: BuiltinGroup = BuiltinGroup {
    id: "payments",
    label: "Trusting a payment key",
    destructive: true,
    commands: &["reins payments-trust"],
    files: &[],
};

pub const DISKS: BuiltinGroup = BuiltinGroup {
    id: "disks",
    label: "Wiping disks",
    destructive: true,
    commands: &["mkfs*", "dd of=*", "format *:", "Format-Volume", "Clear-Disk"],
    files: &[],
};

pub const ENV_FILES: BuiltinGroup = BuiltinGroup {
    id: "env-files",
    label: ".env files",
    destructive: false,
    commands: &[],
    files: &[".env", ".env.*"],
};

pub const KEYS: BuiltinGroup = BuiltinGroup {
    id: "keys",
    label: "Keys and certificates",
    destructive: false,
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
};

pub const CLOUD: BuiltinGroup = BuiltinGroup {
    id: "cloud",
    label: "Cloud and cluster credentials",
    destructive: false,
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
};

pub const TOKENS: BuiltinGroup = BuiltinGroup {
    id: "tokens",
    label: "Saved tokens",
    destructive: false,
    commands: &[],
    files: &[".netrc", "_netrc", ".git-credentials", ".npmrc", ".pypirc", ".vault-token"],
};

/// Every built-in group, what cannot be undone first.
pub const BUILTIN: &[BuiltinGroup] =
    &[GIT_HISTORY, DELETES, INFRASTRUCTURE, DATABASES, PUBLISHING, PAYMENTS, DISKS, ENV_FILES, KEYS, CLOUD, TOKENS];

/// Exceptions to the built-in file rules.
pub const ALLOW_FILES: &[&str] = &["*.pub", "known_hosts", ".env.example", ".env.sample", ".env.template", ".env.dist"];

/// The built-in group `id`.
#[must_use]
pub fn group(id: &str) -> Option<&'static BuiltinGroup> {
    BUILTIN.iter().find(|g| g.id == id)
}

/// A built-in rule a question matched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Classified {
    pub group: &'static BuiltinGroup,
    pub pattern: &'static str,
    /// What matched it: the words, the command or the file.
    pub matched: String,
}

/// The first built-in file rule `path` matches, unless an exception covers it.
#[must_use]
pub fn classify_file(path: &str) -> Option<Classified> {
    if path.is_empty() || ALLOW_FILES.iter().any(|p| file_matches(p, path)) {
        return None;
    }
    BUILTIN.iter().find_map(|g| {
        g.files.iter().find(|p| file_matches(p, path)).map(|p| Classified {
            group: g,
            pattern: p,
            matched: path.to_owned(),
        })
    })
}

/// The first built-in rule a command line matches, as the guard reads it: a command rule (what cannot be undone
/// first), else an argument that names a guarded file.
#[must_use]
pub fn classify_command(command: &str) -> Option<Classified> {
    let flat = text_words(command);
    let segs = segments(command);
    for g in BUILTIN {
        for pattern in g.commands {
            let matched = if let Some(text) = pattern.strip_prefix("text:") {
                let want = text_words(text);
                (!want.is_empty() && flat.windows(want.len()).any(|w| w == want.as_slice())).then(|| want.join(" "))
            } else {
                segs.iter().find(|seg| command_matches(pattern, seg)).map(|seg| seg.join(" "))
            };
            if let Some(matched) = matched {
                return Some(Classified {
                    group: g,
                    pattern,
                    matched,
                });
            }
        }
    }
    segs.iter().flat_map(|seg| seg.iter().skip(1)).find_map(|arg| {
        let path = match arg.split_once('=') {
            Some((opt, value)) if opt.starts_with('-') => value,
            _ if arg.starts_with('-') => return None,
            _ => arg.as_str(),
        };
        classify_file(path)
    })
}

/// What the phone reads in a question: the most severe built-in rule anything in it matches, and whether it could
/// read it with certainty.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reading {
    pub found: Option<Classified>,
    /// Every part read as a shell reads it (quotes, `$(` and heredocs close) and a `Files:` list names files. When
    /// not, the question must be treated as guarded, not harmless.
    pub certain: bool,
}

/// How severe a match is: what cannot be undone first, then the groups in [`BUILTIN`] order.
fn severity(c: &Classified) -> (bool, std::cmp::Reverse<usize>) {
    let at = BUILTIN.iter().position(|g| g.id == c.group.id).unwrap_or(usize::MAX);
    (c.group.destructive, std::cmp::Reverse(at))
}

/// Reads every part of a question, whatever its layout: each text as a command line, and each of its lines as a
/// file; the most severe match wins. The layout the computer wrote (`Files:`, `Command:`, blank lines) decides
/// nothing: the computer is where the AI runs. The first text is what would run (the detail), so it must also read
/// as a shell reads it; the others (a one-line rendering of it, a rule) are only searched.
#[must_use]
pub fn read(texts: &[&str]) -> Reading {
    let mut found: Vec<Classified> = Vec::new();
    let mut certain = true;
    for (i, text) in texts.iter().enumerate() {
        found.extend(classify_command(text));
        for line in text.lines() {
            found.extend(classify_file(line.trim()));
            // A labelled line (`Command: rm -rf ~`, `… wants to run: rm -rf ~`): what follows the label too.
            if let Some((_, rest)) = line.split_once(": ") {
                found.extend(classify_command(rest));
            }
        }
        if i > 0 {
            continue;
        }
        certain &= crate::shell::balanced(text);
        // A `Files:` list that names nothing.
        let mut lines = text.lines().map(str::trim).skip_while(|l| *l != "Files:");
        if lines.next().is_some() && lines.next().is_none_or(str::is_empty) {
            certain = false;
        }
    }
    Reading {
        // The most severe; of equal ones, the first found.
        found: found.into_iter().rev().max_by_key(severity),
        certain,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_and_files_are_read_as_the_guard_reads_them() {
        let c = classify_command("cd app && sudo rm -rf ~").unwrap();
        assert_eq!((c.group.id, c.pattern, c.matched.as_str()), ("deletes", "rm -r", "rm -rf ~"));
        assert_eq!(classify_command("psql -c 'DROP TABLE x'").unwrap().group.id, "databases");
        assert_eq!(classify_command("cat app/.env").unwrap().group.id, "env-files");
        // A command rule wins over a file in the same line.
        assert_eq!(classify_command("cat .env; git push --force").unwrap().group.id, "git-history");
        assert_eq!(classify_command("cat .env.example"), None);
        assert_eq!(classify_command("cargo test"), None);
        let id = |texts: &[&str]| read(texts).found.map(|c| c.group.id);
        assert_eq!(id(&["Files:\n/a/README.md\n/a/.env\n\nIn: /a"]), Some("env-files"));
        assert_eq!(id(&["Command:\ngit push --force origin main\n\nIn: /a"]), Some("git-history"));
        assert!(read(&["Command:\ncargo test\n\nIn: /a"]).certain);
        assert!(BUILTIN.iter().all(|g| g.destructive == g.files.is_empty()));
        assert_eq!(group("keys").map(|g| g.label), Some("Keys and certificates"));
    }

    /// The question's layout is the computer's, and the computer is where the AI runs.
    #[test]
    fn every_part_is_read_and_the_most_severe_match_wins() {
        let id = |texts: &[&str]| read(texts).found.map(|c| c.group.id);
        // A crafted `Files:` prefix, and a blank line before the command.
        assert_eq!(id(&["Files:\n\nCommand: rm -rf ~"]), Some("deletes"));
        assert_eq!(id(&["Files:\nREADME.md\n\nrm -rf ~"]), Some("deletes"));
        // A mild match listed before a destructive one.
        assert_eq!(id(&["Files:\n.env\n\nCommand:\ncat .env && git reset --hard"]), Some("git-history"));
        assert_eq!(id(&["cat ~/.ssh/id_rsa; cat .env"]), Some("keys"), "of equal severity: the first group");
        // Across the texts of a question (its detail, its question line).
        assert_eq!(
            id(&["Command:\ncat .env", "Claude Code wants to run: x; terraform destroy"]),
            Some("infrastructure")
        );
        // What cannot be read with certainty.
        assert!(!read(&["Files:\n\nIn: /a"]).certain, "an empty Files list");
        assert!(!read(&["Files:"]).certain);
        assert!(!read(&["echo 'unclosed; rm -rf ~"]).certain, "an unbalanced quote");
        assert!(!read(&["echo \"$(rm -rf ~\""]).certain);
        assert!(!read(&["cat <<EOF\nnever ends"]).certain);
        // A heredoc's body is not shell text: its apostrophes are not quotes.
        assert!(read(&["python3 - <<'EOF'\nprint(\"don't\")\nEOF\n\nIn: /a"]).certain);
        assert!(read(&["ls # don't"]).certain, "a comment");
    }
}
