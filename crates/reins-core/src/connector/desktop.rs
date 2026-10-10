//! The paired Reins desktop app itself: `desktop_ask`, a yes-or-no question (`reins ask`, a harness hook before a
//! command). The user reads it on the phone; approving answers yes with an [`AskAnswer`] sealed to the app's key and
//! bound to its nonce, denying is the ordinary denied answer. A standing permission can cover a topic
//! (`command:git push --force`), or questions without one (`ask`).
//!
//! There is no account to connect: the paired app is the account, and the flow checks its key.

use reins_proto::connector::{ConnectorCall, DESKTOP};
use reins_proto::desktop::{AskAnswer, SEALED_FIELD};
use serde_json::{Value, json};

use super::sealed::{client_key, nonce_arg, seal};
use super::{Connector, Preview};
use crate::types::{AskView, GmailStatus};
use crate::{CoreError, text};

/// The account every desktop call is made as.
pub const DESKTOP_ACCOUNT: &str = "desktop app";
/// The resource of a question without a topic.
const ASK: &str = "ask";
const MAX_QUESTION: usize = 300;
const MAX_DETAIL: usize = 8_000;
const MAX_TOPIC: usize = 100;

fn bad(message: &str) -> CoreError {
    CoreError::service(message)
}

fn question(call: &ConnectorCall) -> Result<String, CoreError> {
    let q = text::one_line(call.str_arg("question").unwrap_or_default());
    if q.is_empty() {
        return Err(bad("`question` is required."));
    }
    Ok(text::truncate_chars(&q, MAX_QUESTION))
}

/// The topic, when one was given: one line of visible text.
fn topic(call: &ConnectorCall) -> Result<Option<String>, CoreError> {
    match call.str_arg("topic").map(str::trim) {
        None | Some("") => Ok(None),
        Some(t) if t.chars().count() <= MAX_TOPIC && !t.chars().any(char::is_control) && text::one_line(t) == t => {
            Ok(Some(t.to_owned()))
        }
        Some(_) => Err(bad("`topic` must be one line of at most 100 characters.")),
    }
}

/// The question as the approval shows it (`None` for anything but `desktop_ask`).
pub fn ask_view(call: &ConnectorCall) -> Option<AskView> {
    (call.service == DESKTOP && call.op == "ask").then(|| AskView {
        question: text::truncate_chars(&text::one_line(call.str_arg("question").unwrap_or_default()), MAX_QUESTION),
        detail: call
            .str_arg("detail")
            .map(|d| text::truncate_chars(&text::neutralize(d), MAX_DETAIL))
            .filter(|d| !d.trim().is_empty()),
        topic: topic(call).ok().flatten().map(|t| text::one_line(&t)),
    })
}

/// What in a command or a question means it deletes, rewrites history or wipes something: never routine. Matched in
/// lower case anywhere in the topic, the question and the detail (the command), so a harness hook's built-in rules and
/// the same command typed into `reins ask` are treated alike. Publishing is irreversible too, but routine: it stays
/// allowed for a while.
const DESTRUCTIVE: [&str; 46] = [
    "push -f",
    "push --force",
    "push +",
    "push -d",
    "push --delete",
    "push --mirror",
    "push --prune",
    "push :",
    "force-push",
    "force push",
    "reset --hard",
    "clean -f",
    "clean --force",
    "branch -d",
    "branch --delete",
    "checkout -f",
    "filter-branch",
    "filter-repo",
    "rm -r",
    "rm -f",
    "rm --recursive",
    "rm --force",
    "terraform apply",
    "terraform destroy",
    "tofu apply",
    "tofu destroy",
    "kubectl apply",
    "kubectl delete",
    "helm uninstall",
    "helm delete",
    "drop table",
    "drop database",
    "drop schema",
    "truncate table",
    "repo delete",
    "release delete",
    "mkfs",
    "dd of=",
    "-recurse",
    "rd /s",
    "rmdir /s",
    "del /s",
    "erase /s",
    "format-volume",
    "clear-disk",
    "format c:",
];

/// A question that is asked every time: about a destructive command, or about reading a secret file (`file:` topics,
/// such as `.env` or a private key, which would show a secret to the AI).
pub fn destructive_ask(topic: Option<&str>, question: &str, detail: &str) -> bool {
    if topic.is_some_and(|t| t.trim_start().to_lowercase().starts_with("file:")) {
        return true;
    }
    let raw = format!("{}\n{question}\n{detail}", topic.unwrap_or_default()).to_lowercase();
    // Spacing as typed does not matter: `rm  -rf` is `rm -rf`.
    let text = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    // Line by line (a command per line) and as one line (a command wrapped onto the next): either may hide nothing.
    DESTRUCTIVE.iter().any(|needle| text.contains(needle)) || dangerous_flags(&raw) || dangerous_flags(&text)
}

/// A command whose flags make it destructive wherever they stand (`git push origin main --force`, `rm build -rf`):
/// each command up to `;`, `&`, `|` or the end of the line, word by word.
fn dangerous_flags(text: &str) -> bool {
    text.split(['\n', ';', '&', '|']).any(|command| {
        // Quotes and escapes do not change what a shell runs: `'+main'` is `+main`.
        let words: Vec<String> = command
            .split_whitespace()
            .map(|w| w.chars().filter(|c| !matches!(c, '\'' | '"' | '\\' | '`')).collect())
            .collect();
        let words: Vec<&str> = words.iter().map(String::as_str).collect();
        words.iter().enumerate().any(|(i, word)| {
            let rest = &words[i + 1..];
            // `-fu`: single-letter flags together.
            let short = |w: &&str, letters: &[char]| {
                w.starts_with('-') && !w.starts_with("--") && w.chars().skip(1).any(|c| letters.contains(&c))
            };
            match word.rsplit('/').next().unwrap_or(word) {
                "push" => rest.iter().any(|w| {
                    w.starts_with('+')
                        || w.starts_with(':')
                        || w.starts_with("--force")
                        || matches!(*w, "--delete" | "--mirror" | "--prune")
                        || short(w, &['f', 'd'])
                }),
                "rm" | "rmdir" => rest.iter().any(|w| matches!(*w, "--recursive" | "--force") || short(w, &['r', 'f'])),
                "reset" => rest.contains(&"--hard"),
                "clean" => rest.iter().any(|w| *w == "--force" || short(w, &['f'])),
                "branch" => rest.iter().any(|w| *w == "--delete" || short(w, &['d'])),
                "checkout" | "switch" => {
                    rest.iter().any(|w| matches!(*w, "--force" | "--discard-changes") || short(w, &['f']))
                }
                _ => false,
            }
        })
    })
}

#[derive(Default)]
pub struct Desktop;

#[async_trait::async_trait]
impl Connector for Desktop {
    fn service(&self) -> &'static str {
        DESKTOP
    }

    async fn preview(&self, _account: &str, call: &ConnectorCall) -> Result<Preview, CoreError> {
        if call.op == reins_proto::connector::SESSION_OP {
            client_key(call)?;
            nonce_arg(call)?;
            return Ok(crate::work_session::preview(&crate::work_session::plan(call)?));
        }
        if call.op != "ask" {
            return Err(CoreError::service(format!("the desktop app cannot {}", call.op)));
        }
        client_key(call)?;
        nonce_arg(call)?;
        let question = question(call)?;
        let topic = topic(call)?;
        // A question about something that cannot be undone, or about a secret file, is asked every time: never
        // answered from a notification, with "Approve all" or by a standing answer, and never by Autopilot.
        let once_only = destructive_ask(topic.as_deref(), &question, call.str_arg("detail").unwrap_or_default());
        let mut lines = vec![question];
        if let Some(t) = &topic {
            lines.push(format!("About: {t}"));
        }
        let (resource, resource_label) = match topic {
            Some(t) => (t.clone(), format!("Questions about {t}")),
            None => (ASK.to_owned(), "Questions without a topic".to_owned()),
        };
        Ok(Preview {
            resource,
            resource_label,
            lines,
            parents: Vec::new(),
            once_only,
            ..Preview::default()
        })
    }

    async fn perform(&self, _account: &str, call: &ConnectorCall) -> Result<Value, CoreError> {
        let answer = AskAnswer {
            v: 1,
            nonce: nonce_arg(call)?,
            approved: true,
        };
        Ok(json!({ SEALED_FIELD: seal(client_key(call)?, &answer)? }))
    }

    async fn status(&self, _account: &str) -> GmailStatus {
        GmailStatus::Ready
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(args: &Value) -> ConnectorCall {
        ConnectorCall {
            service: DESKTOP.to_owned(),
            op: "ask".to_owned(),
            args: args.as_object().unwrap().clone(),
        }
    }

    #[test]
    fn the_view_shows_the_question_detail_and_topic_cleaned() {
        let v = ask_view(&call(&json!({"question": "Push\u{202e} to main?", "detail": "git push\n--force",
            "topic": "command:git push --force"})))
        .unwrap();
        assert_eq!(v.question, "Push to main?");
        assert_eq!(v.detail.as_deref(), Some("git push\n--force"));
        assert_eq!(v.topic.as_deref(), Some("command:git push --force"));
        let bare = ask_view(&call(&json!({"question": "Ok?", "detail": "  "}))).unwrap();
        assert_eq!((bare.detail, bare.topic), (None, None));
        let mut other = call(&json!({}));
        other.op = "git_fetch".to_owned();
        assert!(ask_view(&other).is_none());
    }

    #[test]
    fn topics_are_one_line() {
        assert!(topic(&call(&json!({"topic": "a\nb"}))).is_err());
        assert!(topic(&call(&json!({"topic": "x".repeat(101)}))).is_err());
        assert_eq!(topic(&call(&json!({"topic": " "}))).unwrap(), None);
    }

    #[test]
    fn destructive_commands_and_secret_files_are_never_routine() {
        let ask = |topic: Option<&str>, question: &str, detail: &str| destructive_ask(topic, question, detail);
        for topic in [
            "command:git push --force*",
            "command:git push -f",
            "command:git reset --hard",
            "command:git branch -D",
            "command:rm -r",
            "command:terraform destroy",
            "command:kubectl delete",
            "command:text:drop table",
            "command:gh repo delete",
            "command:Remove-Item -Recurse",
            "file:.env",
            "file:**/id_ed25519",
        ] {
            assert!(ask(Some(topic), "Run it?", ""), "{topic}");
        }
        assert!(ask(None, "Force-push main?", ""), "said in the question");
        assert!(ask(None, "Run this?", "rm  -rf build"), "said in the command, spacing as typed");
        assert!(ask(None, "Clean up?", "git push origin :old-branch"));
        assert!(ask(None, "Push?", "git push origin +main"));
        assert!(!ask(None, "Push?", "git push origin main && echo +ok"), "another command's words");
        for command in [
            "git push origin main --force",
            "git push origin main -f",
            "git push -uf origin main",
            "git push origin --force-with-lease main",
            "git push origin --delete old",
            "rm build -rf",
            "/bin/rm -R dist",
            "git reset HEAD~3 --hard",
            "git clean -xdf",
            "git branch old -D",
            "git checkout . --force",
            "git push origin '+main'",
            "git push origin \"--force\"",
            "git push origin\n+main",
            "git push origin main --force=true",
        ] {
            assert!(ask(None, "Run this?", command), "{command}");
        }
        for command in
            ["git push origin main", "git push -u origin feature", "rm notes.txt", "git branch -v", "git reset HEAD~1"]
        {
            assert!(!ask(None, "Run this?", command), "{command}");
        }
        for topic in ["command:cargo test", "command:npm publish", "command:make deploy", "ask"] {
            assert!(!ask(Some(topic), "Run the tests?", "cargo test --all"), "{topic}");
        }
        assert!(!ask(None, "Format the code?", "cargo fmt"), "formatting code is not formatting a disk");
    }
}
