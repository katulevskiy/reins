//! Presentation only. Never used by policy, command identity, risk, grants, or execution.
//!
//! The strict reader is a prerequisite, then a deliberately smaller reader preserves source spelling and operators.
//! Unsupported input produces no shortened command. No shell, interpolation, or filesystem lookup is performed.
use crate::types::CommandPresentation;

const MAX_INPUT: usize = 32 * 1024;
const MAX_DEPTH: usize = 2;

#[derive(Debug)]
struct Token<'a> {
    raw: &'a str,
    value: String,
    start: usize,
    end: usize,
}

/// Tokenize only the literal presentation subset. Strict reading has already checked the shell semantics.
fn tokens(text: &str) -> Option<Vec<Token<'_>>> {
    if !text.is_ascii() {
        return None;
    }
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if matches!(bytes[i], b' ' | b'\t') {
            i += 1;
            continue;
        }
        let start = i;
        if matches!(bytes[i], b';' | b'\n' | b'&' | b'|') {
            i += 1;
            if bytes.get(i) == Some(&bytes[start]) && matches!(bytes[start], b'&' | b'|') {
                i += 1;
            }
            out.push(Token {
                raw: &text[start..i],
                value: text[start..i].to_owned(),
                start,
                end: i,
            });
            continue;
        }
        let mut value = String::new();
        while i < bytes.len() && !matches!(bytes[i], b' ' | b'\t' | b'\n' | b';' | b'&' | b'|') {
            match bytes[i] {
                // Redirections, comments, heredocs, and shell expansion retain the full invocation on fallback.
                b'<' | b'>' | b'#' | b'$' | b'`' | b'(' | b')' | b'{' | b'}' => return None,
                quote @ (b'\'' | b'"') => {
                    i += 1;
                    while i < bytes.len() && bytes[i] != quote {
                        if quote == b'"' && matches!(bytes[i], b'$' | b'`' | b'\\') {
                            return None;
                        }
                        value.push(char::from(bytes[i]));
                        i += 1;
                    }
                    if i == bytes.len() {
                        return None;
                    }
                    i += 1;
                }
                b'\\' => {
                    i += 1;
                    value.push(char::from(*bytes.get(i)?));
                    i += 1;
                }
                byte if byte.is_ascii() => {
                    value.push(char::from(byte));
                    i += 1;
                }
                _ => return None,
            }
        }
        out.push(Token {
            raw: &text[start..i],
            value,
            start,
            end: i,
        });
        if out.len() > 512 {
            return None;
        }
    }
    Some(out)
}

fn assignment(word: &str) -> Option<&str> {
    let (name, _) = word.split_once('=')?;
    (!name.is_empty()
        && !name.starts_with(|c: char| c.is_ascii_digit())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
    .then_some(name)
}

fn program(word: &str) -> &str {
    word.rsplit('/').next().unwrap_or(word)
}

fn shell_spelling(word: &str) -> bool {
    let name = word.strip_prefix("/bin/").or_else(|| word.strip_prefix("/usr/bin/")).unwrap_or(word);
    ["sh", "bash", "zsh", "dash", "ksh"].contains(&name)
}

fn fallback(command: &str) -> CommandPresentation {
    CommandPresentation {
        heading: "Review a shell command".into(),
        notices: vec!["Reins cannot safely shorten this invocation. Inspect the full invocation, including its setup and shell syntax.".into()],
        original: command.to_owned(),
        ..CommandPresentation::default()
    }
}

/// The smaller display reader only peels literal leading assignments, plain `env`, an exact shell wrapper, and
/// leading `cd path &&`. Everything else stays actionable, including privilege-changing wrappers.
fn read(text: &str, depth: usize, out: &mut CommandPresentation) -> Option<()> {
    if depth > MAX_DEPTH {
        return None;
    }
    let toks = tokens(text)?;
    let mut start = 0;
    while let Some(t) = toks.get(start) {
        if let Some(name) = assignment(&t.value) {
            // Quoting/escaping the name can turn a shell assignment into a program name. Never peel that spelling.
            if !t.raw.starts_with(&format!("{name}=")) {
                return None;
            }
            out.setup.push(format!("{name}=•••• (value hidden)"));
            if matches!(
                name,
                "PATH"
                    | "JAVA_TOOL_OPTIONS"
                    | "JDK_JAVA_OPTIONS"
                    | "GRADLE_OPTS"
                    | "JAVA_OPTS"
                    | "LD_PRELOAD"
                    | "LD_LIBRARY_PATH"
                    | "DYLD_INSERT_LIBRARIES"
                    | "BASH_ENV"
                    | "ENV"
                    | "SHELLOPTS"
                    | "NODE_OPTIONS"
                    | "PYTHONPATH"
            ) {
                out.notices.push(format!(
                    "{name} changes executable resolution or runtime behavior. Inspect its value before approving."
                ));
            }
            start += 1;
        } else if t.value == "env" || t.value == "/usr/bin/env" {
            // Options such as --chdir, -i and -u remain in the actionable command, with their effects intact.
            if toks.get(start + 1).is_some_and(|next| assignment(&next.value).is_some()) {
                out.setup.push(format!("Environment wrapper: {}", t.raw));
                start += 1;
            } else {
                break;
            }
        } else {
            break;
        }
    }
    let first = toks.get(start)?;
    if shell_spelling(&first.value)
        && toks.get(start + 1).is_some_and(|t| matches!(t.value.as_str(), "-c" | "-lc"))
        && toks.len() == start + 3
    {
        let flag = &toks[start + 1].value;
        out.setup.push(format!("Shell: {} {flag}", first.raw));
        out.notices.push("The shell wrapper can load shell startup configuration.".into());
        return read(&toks[start + 2].value, depth + 1, out);
    }
    if ["sh", "bash", "zsh", "dash", "ksh", "fish"].contains(&program(&first.value)) {
        return None;
    }
    if first.value == "cd" && toks.get(start + 2).is_some_and(|t| t.raw == "&&") {
        let directory = toks.get(start + 1)?;
        out.setup.push(format!("Directory change: cd {}", directory.raw));
        return read(text[toks[start + 2].end..].trim_start(), depth + 1, out);
    }
    // Unpeeled assignment-shaped operands can be wrapper environment arguments, including names a shell would
    // reject (`env FOO.bar=value`). Never expose those values or silently change their command-specific scope.
    if toks.iter().skip(start).any(|t| t.value.contains('=') && !t.value.starts_with('-')) {
        return None;
    }
    text[first.start..].trim().clone_into(&mut out.actionable);
    let parsed = reins_policy::strict::read(text)?;
    if parsed.len() > 1 {
        out.heading = "Run multiple commands".into();
        out.notices.push(
            "This invocation contains multiple commands. Review every command and the operators between them.".into(),
        );
    } else if matches!(program(&first.value), "gradlew" | "gradle") {
        out.heading = "Run Gradle".into();
        out.notices.push(
            "Gradle can execute project build scripts and plugins with this process’s permissions and credentials."
                .into(),
        );
    } else {
        out.heading = "Run a command".into();
    }
    if parsed.iter().any(|s| s.words.first().is_some_and(|w| matches!(program(w), "gradlew" | "gradle")))
        && !out.notices.iter().any(|n| n.starts_with("Gradle"))
    {
        out.notices.push(
            "Gradle can execute project build scripts and plugins with this process’s permissions and credentials."
                .into(),
        );
    }
    if toks.iter().enumerate().skip(start).any(|(i, t)| {
        (i == start || matches!(toks[i - 1].raw, "&&" | "||" | "|" | ";" | "\n"))
            && matches!(program(&t.value), "sudo" | "doas" | "su")
    }) {
        out.notices.push("This invocation requests a different user or elevated privileges.".into());
    }
    Some(())
}

/// Build display facts without changing, normalizing, or evaluating the original authorization payload.
pub fn of(command: &str) -> CommandPresentation {
    if command.len() > MAX_INPUT || reins_policy::strict::read(command).is_none() {
        return fallback(command);
    }
    let mut out = CommandPresentation::default();
    if read(command.trim(), 0, &mut out).is_none() {
        return fallback(command);
    }
    command.clone_into(&mut out.original);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn separates_literal_environment_from_exact_gradle_arguments() {
        let p =
            of("JAVA_HOME=/opt/jdk ANDROID_HOME=/sdk TOKEN='private secret' ./gradlew :app:assembleDebug --no-daemon");
        assert_eq!(p.actionable, "./gradlew :app:assembleDebug --no-daemon");
        assert_eq!(p.heading, "Run Gradle");
        assert_eq!(p.setup.len(), 3);
        assert!(!format!("{p:?}").contains("private secret"));
        assert!(p.notices.iter().any(|n| n.starts_with("Gradle")));
    }

    #[test]
    fn unwraps_exact_shell_and_leading_directory_but_preserves_other_actions() {
        let p = of("/bin/sh -lc 'cd /repo && ./gradlew test && rm -rf /tmp/other'");
        assert_eq!(p.actionable, "./gradlew test && rm -rf /tmp/other");
        assert_eq!(p.heading, "Run multiple commands");
        assert!(p.setup.contains(&"Directory change: cd /repo".into()));
        assert!(p.setup.contains(&"Shell: /bin/sh -lc".into()));
    }

    #[test]
    fn privilege_and_runtime_context_remain_visible() {
        let p = of("PATH=/other JAVA_TOOL_OPTIONS=-javaagent:agent.jar sudo ./gradlew test");
        assert_eq!(p.actionable, "sudo ./gradlew test");
        assert!(p.notices.iter().any(|n| n.starts_with("PATH")));
        assert!(p.notices.iter().any(|n| n.starts_with("JAVA_TOOL_OPTIONS")));
        assert!(p.notices.iter().any(|n| n.contains("privileges")));
        assert!(!format!("{p:?}").contains("agent.jar"));
        assert!(!of("echo sudo").notices.iter().any(|n| n.contains("privileges")));
    }

    #[test]
    fn unsupported_or_effectful_shell_never_gets_a_reassuring_shortcut() {
        for command in [
            "./gradlew test > /etc/config",
            "./gradlew test $(curl evil)",
            "sh -lc './gradlew test' extra",
            "./gradlew test && TOKEN=secret curl host",
            "cat <<EOF\ntext\nEOF",
            "cd /repo; ./gradlew test",
            "echo '💥'",
            "'TOKEN=not-an-assignment' ./gradlew test",
            "TO\"K\"EN=not-an-assignment ./gradlew test",
            "/repo/sh -lc './gradlew test'",
        ] {
            let p = of(command);
            // Some literal extra arguments remain fully visible; the rest use the explicit fallback.
            assert!(p.actionable.is_empty() || p.actionable == command, "{command}: {p:?}");
        }
        assert!(of(&"x".repeat(MAX_INPUT + 1)).actionable.is_empty());
    }

    #[test]
    fn presentation_does_not_change_strict_certainty() {
        for command in [
            "JAVA_HOME=/jdk ./gradlew test",
            "PATH=/other sudo ./gradlew test",
            "./gradlew test > /etc/config",
            "./gradlew test $(curl evil)",
        ] {
            let before = reins_policy::strict::read(command);
            let original = command.to_owned();
            drop(of(command));
            assert_eq!(reins_policy::strict::read(command), before);
            assert_eq!(command, original);
        }
    }

    #[test]
    fn readable_equivalence_does_not_broaden_exact_approval_identity() {
        let a = "JAVA_HOME=/jdk17 ./gradlew test";
        let b = "JAVA_HOME=/jdk21 ./gradlew test";
        assert_eq!(of(a).actionable, of(b).actionable);
        let resource = |command: &str, cwd: &str| {
            crate::connector::desktop::exact_resource(
                "computer-a",
                "Run it?",
                &format!("Command:\n{command}\n\nIn: {cwd}"),
            )
            .0
        };
        assert_ne!(resource(a, "/project"), resource(b, "/project"));
        assert_ne!(resource(a, "/project"), resource(a, "/another-project"));
    }

    #[test]
    fn old_stored_labels_deserialize_without_presentation() {
        let label: crate::types::RequestLabel = serde_json::from_str(
            r#"{"provider":"terminal","action":"run","lead":"Run","subject":"./gradlew test","code":true}"#,
        )
        .unwrap();
        assert!(label.command.is_none());
    }

    #[test]
    fn explicit_inspection_preserves_original_beyond_legacy_detail_limit() {
        let command = format!("TOKEN='{}' ./gradlew test --info", "private".repeat(1400));
        let p = of(&command);
        assert_eq!(p.original, command);
        assert_eq!(p.actionable, "./gradlew test --info");
        assert!(!format!("{p:?}").contains("privateprivate"));
        for source in ["env FOO.bar=private ./gradlew test", "sudo FOO.bar=private ./gradlew test"] {
            let p = of(source);
            assert!(p.actionable.is_empty());
            assert!(!format!("{p:?}").contains("private"));
            assert_eq!(p.original, source);
        }
    }
}
