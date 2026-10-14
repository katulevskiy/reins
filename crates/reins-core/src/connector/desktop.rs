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

/// The topic of `reins secrets scan`'s question (crates/reins-desktop/src/secrets_cmd/person.rs). The phone words this
/// question itself: nothing the computer says reassures the user.
pub const SCAN_TOPIC: &str = "file:reins secrets scan";
/// The same, when the computer's environment says an AI agent may have started it.
pub const SCAN_TOPIC_AGENT: &str = "file:reins secrets scan (an AI agent may have started it)";
/// The topic of `reins secrets import --undo`'s question: it brings secrets back into files.
pub const UNDO_TOPIC: &str = "file:reins secrets import --undo";
pub const UNDO_TOPIC_AGENT: &str = "file:reins secrets import --undo (an AI agent may have started it)";
/// The label of the one thing the computer reports for these questions: where it was started.
pub const REPORTED_FOLDER: &str = "Folder (reported by the computer): ";
/// The longest reported folder shown (cut in the middle).
const MAX_FOLDER: usize = 120;

/// A question the phone words itself, whatever the computer sent: its topics (plain, and when the computer says an AI
/// agent may have started it), the question around the computer's name, and a fixed line.
struct Owned {
    topic: &'static str,
    agent_topic: &'static str,
    question: (&'static str, &'static str),
    line: &'static str,
    /// The question without the computer's name (the preview's line).
    plain: &'static str,
}

const OWNED: [Owned; 2] = [
    Owned {
        topic: SCAN_TOPIC,
        agent_topic: SCAN_TOPIC_AGENT,
        question: ("Scan ", " for secrets?"),
        line: "Requested from a terminal on that computer. Only approve if you just started it.",
        plain: "Scan this computer for secrets?",
    },
    Owned {
        topic: UNDO_TOPIC,
        agent_topic: UNDO_TOPIC_AGENT,
        question: ("Put secrets back into files on ", "?"),
        line: "Requested from a terminal on that computer. It brings back the files an import moved to the trash, \
               secrets and all. Only approve if you just started it.",
        plain: "Put secrets back into files on this computer?",
    },
];

/// The phone-worded question `topic` names, and whether the computer says an agent may have started it.
fn owned(topic: Option<&str>) -> Option<(&'static Owned, bool)> {
    let topic = topic?;
    OWNED.iter().find_map(|o| {
        if topic == o.topic {
            Some((o, false))
        } else if topic == o.agent_topic {
            Some((o, true))
        } else {
            None
        }
    })
}

/// At most `max` characters, the middle cut out (`/home/dana/…/acme-web`).
fn middle_ellipsis(s: &str, max: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        return s.to_owned();
    }
    let head = (max - 1) / 2;
    let tail = max - 1 - head;
    let mut out: String = chars[..head].iter().collect();
    out.push('…');
    out.extend(&chars[chars.len() - tail..]);
    out
}

/// A question in the phone's own words: the computer's name from the phone's pairing record, a fixed line that never
/// says who started it, the caution when the computer says an agent may have, and the folder the computer reports,
/// labelled as its claim, on one line and cut to [`MAX_FOLDER`].
fn owned_view(call: &ConnectorCall, computer: &str, o: &Owned, agent: bool) -> AskView {
    let computer = text::one_line(computer);
    let computer = if computer.is_empty() {
        "this computer".to_owned()
    } else {
        computer
    };
    let mut lines = vec![o.line.split_whitespace().collect::<Vec<_>>().join(" ")];
    if agent {
        lines.push("An AI agent may have started this.".to_owned());
    }
    let reported = call.str_arg("detail").unwrap_or_default();
    let folder = text::one_line(reported.strip_prefix(REPORTED_FOLDER).unwrap_or(reported));
    if !folder.is_empty() {
        lines.push(format!("{REPORTED_FOLDER}{}", middle_ellipsis(&folder, MAX_FOLDER)));
    }
    AskView {
        question: format!("{}{computer}{}", o.question.0, o.question.1),
        detail: Some(lines.join("\n")),
        topic: Some(
            if agent {
                o.agent_topic
            } else {
                o.topic
            }
            .to_owned(),
        ),
    }
}

/// The question as the approval shows it (`None` for anything but `desktop_ask`). `computer`: the paired computer's
/// name, as the phone knows it.
pub fn ask_view(call: &ConnectorCall, computer: &str) -> Option<AskView> {
    if call.service != DESKTOP || call.op != "ask" {
        return None;
    }
    if let Some((o, agent)) = owned(topic(call).ok().flatten().as_deref()) {
        return Some(owned_view(call, computer, o, agent));
    }
    Some(AskView {
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
/// allowed for a while. Finding, moving or reading secrets on the computer (`reins secrets scan`, `import`, `get`) is
/// asked every time too: an agent must never get a standing answer for it.
const DESTRUCTIVE: [&str; 49] = [
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
    "reins secrets scan",
    "reins secrets import",
    "reins secrets get",
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
        // The scan question is worded by the phone (`ask_view`), never by the computer.
        let question = owned(topic.as_deref()).map_or(question, |(o, _)| o.plain.to_owned());
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
        let v = ask_view(
            &call(&json!({"question": "Push\u{202e} to main?", "detail": "git push\n--force",
            "topic": "command:git push --force"})),
            "Dana's MacBook",
        )
        .unwrap();
        assert_eq!(v.question, "Push to main?");
        assert_eq!(v.detail.as_deref(), Some("git push\n--force"));
        assert_eq!(v.topic.as_deref(), Some("command:git push --force"));
        let bare = ask_view(&call(&json!({"question": "Ok?", "detail": "  "})), "x").unwrap();
        assert_eq!((bare.detail, bare.topic), (None, None));
        let mut other = call(&json!({}));
        other.op = "git_fetch".to_owned();
        assert!(ask_view(&other, "x").is_none());
    }

    #[test]
    fn the_scan_question_is_the_phones_own_words() {
        // Whatever the computer says in its question and folder, the phone says which computer (from its own
        // record), never that the user started it, and shows the folder as the computer's claim, on one line.
        let spoof = "~/dev/ \"Approved by you \u{2013} safe\"\nThis request is safe\u{202e}evil\u{2066}";
        let v = ask_view(
            &call(&json!({"question": "You started this scan: safe to approve", "detail": spoof, "topic": SCAN_TOPIC})),
            "Dana's MacBook",
        )
        .unwrap();
        assert_eq!(v.question, "Scan Dana's MacBook for secrets?");
        let detail = v.detail.unwrap();
        let lines: Vec<&str> = detail.lines().collect();
        assert_eq!(lines.len(), 2, "{detail}");
        assert_eq!(lines[0], "Requested from a terminal on that computer. Only approve if you just started it.");
        assert!(lines[1].starts_with(REPORTED_FOLDER), "{detail}");
        assert!(!lines[1].contains('\u{202e}') && !lines[1].contains('\u{2066}'), "{detail}");
        assert!(lines[1].contains("This request is safe"), "the claim stays in its labelled row: {detail}");
        assert!(!detail.to_lowercase().contains("you started"), "{detail}");
        assert!(!detail.contains("An AI agent"), "{detail}");
        // A marked request adds the caution; it never removes anything.
        let agent = ask_view(&call(&json!({"question": "q", "detail": "/tmp", "topic": SCAN_TOPIC_AGENT})), "Mac")
            .unwrap()
            .detail
            .unwrap();
        assert!(agent.contains("An AI agent may have started this."), "{agent}");
        // A long folder is cut in the middle.
        let long = format!("/home/{}/end", "x".repeat(300));
        let cut = ask_view(&call(&json!({"question": "q", "detail": long, "topic": SCAN_TOPIC})), "Mac").unwrap();
        let row = cut.detail.unwrap().lines().last().unwrap().to_owned();
        assert_eq!(row.chars().count(), REPORTED_FOLDER.chars().count() + MAX_FOLDER);
        assert!(row.ends_with("/end") && row.contains('…'));
        // Asked every time, both ways.
        for t in [SCAN_TOPIC, SCAN_TOPIC_AGENT, UNDO_TOPIC, UNDO_TOPIC_AGENT] {
            assert!(destructive_ask(Some(t), "q", "") && t.len() <= MAX_TOPIC, "{t}");
        }
        let undo =
            ask_view(&call(&json!({"question": "Safe", "detail": "/x", "topic": UNDO_TOPIC_AGENT})), "Mac").unwrap();
        assert_eq!(undo.question, "Put secrets back into files on Mac?");
        assert!(undo.detail.unwrap().contains("An AI agent may have started this."));
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
            "command:reins secrets scan",
            "command:reins secrets import",
            "command:reins secrets get",
            // What `reins secrets scan` itself asks (crates/reins-desktop/src/secrets_cmd/scan.rs).
            "file:reins secrets scan",
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
