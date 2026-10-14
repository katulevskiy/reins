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
use crate::types::{AskRuleView, AskView, GmailStatus};
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

/// The question as the approval shows it (`None` for anything but `desktop_ask`), with the rule it matches as the
/// phone reads it ([`read_ask`]). A hook's rule lines are not shown as part of the detail.
pub fn ask_view(call: &ConnectorCall) -> Option<AskView> {
    (call.service == DESKTOP && call.op == "ask").then(|| {
        let raw = call.str_arg("detail").unwrap_or_default();
        let question =
            text::truncate_chars(&text::one_line(call.str_arg("question").unwrap_or_default()), MAX_QUESTION);
        let topic = topic(call).ok().flatten().map(|t| text::one_line(&t));
        let reading = read_ask(topic.as_deref(), &question, raw);
        let detail = reported_rule(raw).map_or(raw, |(_, rest)| rest);
        AskView {
            question,
            detail: Some(text::truncate_chars(&text::neutralize(detail), MAX_DETAIL)).filter(|d| !d.trim().is_empty()),
            topic,
            rule: reading.rule,
            uncertain: !reading.certain,
        }
    })
}

/// A built-in guard group's name (`env-files`: ".env files"), for Autopilot's kinds of question.
pub fn guard_group_label(id: &str) -> Option<&'static str> {
    reins_policy::guard::group(id).map(|g| g.label)
}

/// What the phone makes of a desktop question.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AskReading {
    /// The rule it matches, as the phone reads it (else as the computer reports it).
    pub rule: Option<AskRuleView>,
    /// Asked every time: held to approve, never remembered, left to the user by Autopilot in Manual and Auto.
    pub destructive: bool,
    /// Read with certainty. When not (an unbalanced quote, a `Files:` list naming nothing, a detail cut short), the
    /// question is not harmless: Autopilot leaves it to the user in Manual and Auto.
    pub certain: bool,
}

/// Reads a desktop question.
///
/// The phone reads everything the question carries (its detail whatever its layout, its question line, its topic's
/// rule) with its own copy of the guard's built-in rules (`reins_policy::guard`), and the most severe match wins. What
/// the computer reports (a hook's rule lines) comes from where the AI runs, so it can only make a question stricter:
/// reported as something that cannot be undone, it is asked every time; reported as a secret file when the phone reads
/// `rm -rf ~`, the phone's reading wins. A rule only the computer knows (the user's own, a newer group) is shown as
/// reported. Where the phone finds no built-in rule, or cannot read the question with certainty, the question is also
/// read for a destructive command ([`destructive_ask`]).
pub fn read_ask(topic: Option<&str>, question: &str, detail: &str) -> AskReading {
    let (reported, rest) = match reported_rule(detail) {
        Some((rule, rest)) => (Some(rule), rest),
        None => (None, detail),
    };
    // What it carries first (so a match is shown from it), then the rule lines, which can only add.
    let mut texts = vec![rest, question];
    texts.extend(topic.and_then(|t| t.split_once(':')).map(|(_, rule)| rule));
    texts.push(&detail[..detail.len() - rest.len()]);
    let reading = reins_policy::guard::read(&texts);
    // The hook cuts a detail at 8,000 characters, with an ellipsis: what follows is unknown.
    let certain = reading.certain && !detail.trim_end().ends_with('…') && detail.chars().count() < MAX_DETAIL;
    let own = reading.found.map(|c| AskRuleView {
        label: c.group.label.to_owned(),
        pattern: c.pattern.to_owned(),
        matched: text::truncate_chars(&text::one_line(&c.matched), MAX_TOPIC),
        group: Some(c.group.id.to_owned()),
        destructive: c.group.destructive,
        reported: false,
    });
    let said = || destructive_ask(topic, question, detail);
    let (rule, destructive) = match (own, reported) {
        (Some(own), _) if own.destructive => (Some(own), true),
        (_, Some(reported)) if reported.destructive => (Some(reported), true),
        (Some(mut own), _) => {
            own.destructive = !certain && said();
            let destructive = own.destructive;
            (Some(own), destructive)
        }
        (None, Some(mut reported)) => {
            reported.destructive = said();
            let destructive = reported.destructive;
            (Some(reported), destructive)
        }
        (None, None) => (None, said()),
    };
    AskReading {
        rule,
        destructive,
        certain,
    }
}

/// [`read_ask`]'s rule and whether the question is asked every time.
pub fn classify(topic: Option<&str>, question: &str, detail: &str) -> (Option<AskRuleView>, bool) {
    let reading = read_ask(topic, question, detail);
    (reading.rule, reading.destructive)
}

/// The next line of `text` when it starts with `prefix`: its value, and `text` moves past it.
fn take_line<'a>(text: &mut &'a str, prefix: &str) -> Option<&'a str> {
    let (line, after) = text.split_once('\n').unwrap_or((text, ""));
    let value = line.strip_prefix(prefix)?;
    *text = after;
    Some(value.trim())
}

/// The rule a harness hook's question says it matched, from the first lines of its detail
/// (`reins_desktop::hooks::rule_lines`: `Rule: <label> · <pattern>`, `Matched: <text>`, `Group: <id>[ (destructive)]`),
/// and the rest of the detail. `None` when the detail does not start with them (`reins ask`, an older desktop app).
/// What the computer reports: see [`classify`] for how far it is trusted.
fn reported_rule(detail: &str) -> Option<(AskRuleView, &str)> {
    let mut rest = detail;
    let (label, pattern) = take_line(&mut rest, "Rule: ")?.split_once(" · ")?;
    if label.trim().is_empty() || pattern.trim().is_empty() {
        return None;
    }
    let matched = take_line(&mut rest, "Matched: ").unwrap_or_default();
    let (group, destructive) = match take_line(&mut rest, "Group: ") {
        Some(g) => {
            let (id, destructive) = g.strip_suffix(" (destructive)").map_or((g, false), |id| (id, true));
            if id.is_empty()
                || id.len() > 40
                || !id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            {
                return None;
            }
            (Some(id.to_owned()), destructive)
        }
        None => (None, false),
    };
    let clean = |s: &str, max: usize| text::truncate_chars(&text::one_line(s), max);
    let rule = AskRuleView {
        label: clean(label, 60),
        pattern: clean(pattern, MAX_TOPIC),
        matched: clean(matched, MAX_TOPIC),
        group,
        destructive,
        reported: true,
    };
    Some((rule, rest.strip_prefix('\n').unwrap_or(rest)))
}

/// What in a command or a question means it deletes, rewrites history or wipes something: never routine. Matched in
/// lower case anywhere in the topic, the question and the detail (the command), for questions that do not name a
/// built-in rule's group (`reins ask`, the user's own rules, older desktop apps), which say it themselves.
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

/// A question about a destructive command, said in its topic, the question or the detail. Reading a secret file
/// (`file:` topics) is not: it is an ordinary yes or no, which Autopilot may learn.
pub fn destructive_ask(topic: Option<&str>, question: &str, detail: &str) -> bool {
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
        // A question about something that cannot be undone is asked every time: never answered from a notification,
        // with "Approve all" or by a standing answer, and never by Autopilot in Manual or Auto.
        let (_, once_only) = classify(topic.as_deref(), &question, call.str_arg("detail").unwrap_or_default());
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
        // No rule lines: the phone reads the rule from the question (here its topic).
        assert_eq!(v.rule.map(|r| (r.group, r.destructive)), Some((Some("git-history".to_owned()), true)));
        let bare = ask_view(&call(&json!({"question": "Ok?", "detail": "  "}))).unwrap();
        assert_eq!((bare.detail, bare.topic), (None, None));
        let mut other = call(&json!({}));
        other.op = "git_fetch".to_owned();
        assert!(ask_view(&other).is_none());
    }

    /// The lines `reins_desktop::hooks::rule_lines` writes.
    const ENV_READ: &str =
        "Rule: .env files · .env\nMatched: app/.env\nGroup: env-files\n\nFiles:\napp/.env\n\nIn: /home/me/app";
    const FORCE_PUSH: &str = "Rule: Rewriting git history · git push --force*\nMatched: git push --force origin \
                              main\nGroup: git-history (destructive)\n\nCommand:\ngit push --force origin main";

    #[test]
    fn the_phone_reads_the_rule_from_the_question_itself() {
        let v = ask_view(&call(&json!({"question": "Claude Code wants to read app/.env", "detail": ENV_READ,
            "topic": "file:.env"})))
        .unwrap();
        let rule = v.rule.unwrap();
        assert_eq!(
            (rule.label.as_str(), rule.pattern.as_str(), rule.matched.as_str()),
            (".env files", ".env", "app/.env")
        );
        assert_eq!((rule.group.as_deref(), rule.destructive, rule.reported), (Some("env-files"), false, false));
        assert_eq!(v.detail.as_deref(), Some("Files:\napp/.env\n\nIn: /home/me/app"), "the rule lines are not shown");
        let (force, destructive) = classify(Some("command:git push --force*"), "Run it?", FORCE_PUSH);
        assert!(destructive);
        assert_eq!((force.unwrap().group.as_deref(), destructive), (Some("git-history"), true));
        // Without rule lines (`reins ask`, an older desktop app) the phone finds the rule all the same.
        let (old, destructive) = classify(Some("file:.env"), "Read .env?", "Files:\n.env");
        assert_eq!((old.unwrap().group.as_deref(), destructive), (Some("env-files"), false));
        let (plain, destructive) = classify(None, "Run this?", "rm -rf build");
        assert_eq!((plain.unwrap().group.as_deref(), destructive), (Some("deletes"), true));
        // A rule only the computer knows is shown as reported, and read for a destructive command.
        let own = "Rule: Your rule · make deploy\nMatched: make deploy\n\nCommand:\nmake deploy";
        let (rule, destructive) = classify(Some("command:make deploy"), "Run it?", own);
        let rule = rule.unwrap();
        assert_eq!((rule.label.as_str(), rule.group, rule.reported, destructive), ("Your rule", None, true, false));
        // Anything else is plain detail.
        for detail in ["Command:\nRule: .env files · .env", "Rule: no separator", "Rule: x · y\nGroup: Not An Id"] {
            assert!(reported_rule(detail).is_none(), "{detail}");
        }
        assert_eq!(guard_group_label("env-files"), Some(".env files"));
        assert_eq!(guard_group_label("nope"), None);
    }

    /// What the computer reports can make a question stricter, never looser.
    #[test]
    fn the_computers_label_can_only_make_a_question_stricter() {
        // Understated: `rm -rf ~` labelled as a secret file. The phone reads it as a recursive delete.
        let understated =
            "Rule: .env files · .env\nMatched: .env\nGroup: env-files\n\nCommand:\nrm -rf ~\n\nIn: /home/me";
        let (rule, destructive) = classify(Some("file:.env"), "Claude Code wants to run: rm -rf ~", understated);
        let rule = rule.unwrap();
        assert!(destructive, "asked every time, held, never Autopilot's");
        assert_eq!(
            (rule.label.as_str(), rule.group.as_deref(), rule.reported),
            ("Recursive deletes", Some("deletes"), false)
        );
        // Overstated: a file read labelled as something that cannot be undone. It stays that strict.
        let overstated = "Rule: Recursive deletes · rm -r\nMatched: cat\nGroup: deletes (destructive)\n\nCommand:\ncat \
                          README.md";
        let (rule, destructive) = classify(Some("command:rm -r"), "Run it?", overstated);
        assert!(destructive);
        assert!(rule.unwrap().destructive);
        // A plain file read: the ordinary yes or no.
        let (_, destructive) = classify(Some("file:.env"), "Read app/.env?", ENV_READ);
        assert!(!destructive);
        // A file rule's question whose command also deletes: the phone's reading of the command decides.
        let read_then_rm = ENV_READ.replace("Files:\napp/.env", "Command:\ncat .env; rm -rf old");
        let (rule, destructive) = classify(Some("file:.env"), "Run it?", &read_then_rm);
        assert_eq!((rule.unwrap().group.as_deref(), destructive), (Some("deletes"), true));
        // The class Autopilot learns comes from the phone's reading too.
        let view = AskView {
            question: "Run it?".to_owned(),
            detail: None,
            topic: Some("file:.env".to_owned()),
            rule: classify(Some("file:.env"), "Run it?", understated).0,
            uncertain: false,
        };
        assert_eq!(view.rule.and_then(|r| r.group).as_deref(), Some("deletes"));
    }

    /// Questions crafted on the computer to look milder than what they carry.
    #[test]
    fn a_crafted_layout_or_an_unreadable_question_is_not_milder() {
        let env = "Rule: .env files · .env\nMatched: .env\nGroup: env-files\n\n";
        let read =
            |detail: &str| read_ask(Some("file:.env"), "Claude Code wants to read .env", &format!("{env}{detail}"));
        // A crafted `Files:` prefix, and a blank line splitting the files from the command.
        for detail in [
            "Files:\n\nCommand: rm -rf ~",
            "Files:\n.env\n\nrm -rf ~\n\nIn: /home/me",
            "Files:\n.env\n\nCommand:\nrm -rf ~",
        ] {
            let r = read(detail);
            assert!(r.destructive, "{detail}");
            assert_eq!(r.rule.unwrap().group.as_deref(), Some("deletes"), "{detail}");
        }
        // A mild match listed before a destructive one.
        let r = read("Command:\ncat .env && git push --force origin main");
        assert_eq!((r.rule.unwrap().group.as_deref(), r.destructive), (Some("git-history"), true));
        // The command in the question line counts as much as the detail.
        let r =
            read_ask(Some("file:.env"), "Claude Code wants to run: terraform destroy", &format!("{env}Files:\n.env"));
        assert!(r.destructive);
        // A destructive group relabelled as a file read.
        let relabelled =
            "Rule: .env files · .env\nMatched: .env\nGroup: env-files\n\nCommand:\ngit reset --hard HEAD~3";
        assert!(read_ask(Some("file:.env"), "Run it?", relabelled).destructive);
        // An empty `Files:` list, an unbalanced quote, a detail cut short: not read with certainty.
        for detail in
            ["Files:\n\nIn: /home/me", "Command:\ncat .env 'x", &format!("Command:\ncat .env {}…", "x".repeat(50))]
        {
            let r = read(detail);
            assert!(!r.certain, "{detail}");
            let shown = ask_view(&call(
                &json!({"question": "Read it?", "detail": format!("{env}{detail}"), "topic": "file:.env",
                "client_key": "k", "nonce": "n"}),
            ))
            .unwrap();
            assert!(shown.uncertain, "Autopilot leaves it to the user: {detail}");
        }
        // Unreadable, and saying something destructive: asked every time.
        assert!(read("Command:\ncat .env; rm -rf ~ 'x").destructive);
        // A plain file read stays the ordinary yes or no.
        let r = read("Files:\napp/.env\n\nIn: /home/me/app");
        assert!(r.certain && !r.destructive);
    }

    #[test]
    fn topics_are_one_line() {
        assert!(topic(&call(&json!({"topic": "a\nb"}))).is_err());
        assert!(topic(&call(&json!({"topic": "x".repeat(101)}))).is_err());
        assert_eq!(topic(&call(&json!({"topic": " "}))).unwrap(), None);
    }

    #[test]
    fn destructive_commands_are_never_routine() {
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
        ] {
            assert!(ask(Some(topic), "Run it?", ""), "{topic}");
        }
        for topic in ["file:.env", "file:**/id_ed25519"] {
            assert!(!ask(Some(topic), "Read it?", ""), "{topic}: a secret file is an ordinary question");
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
