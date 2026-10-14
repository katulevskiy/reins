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
        rule: None,
        uncertain: false,
        // Finding or restoring secrets: asked every time and held, as before.
        destructive: true,
        kind: None,
    }
}

/// The question as the approval shows it (`None` for anything but `desktop_ask`), with what the phone reads in it
/// ([`read_ask`]). A hook's rule lines are not shown as part of the detail. `computer`: the paired computer's name, as
/// the phone knows it (for the questions the phone words itself).
pub fn ask_view(call: &ConnectorCall, computer: &str) -> Option<AskView> {
    if call.service != DESKTOP || call.op != "ask" {
        return None;
    }
    if let Some((o, agent)) = owned(topic(call).ok().flatten().as_deref()) {
        return Some(owned_view(call, computer, o, agent));
    }
    Some({
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
            destructive: reading.destructive,
            kind: reading.kind.map(str::to_owned),
        }
    })
}

/// A built-in guard group's name (`env-files`: ".env files"), for Autopilot's kinds of question.
pub fn guard_group_label(id: &str) -> Option<&'static str> {
    reins_policy::guard::group(id).map(|g| g.label)
}

/// What the phone makes of a desktop question. Every path that decides how a question is answered (asked every
/// time, one tap, Approve all, a standing answer, Autopilot) uses this one reading.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AskReading {
    /// The rule it matches, as the phone reads it (else as the computer reports it).
    pub rule: Option<AskRuleView>,
    /// It cannot be undone: asked every time, held, shown in red.
    pub destructive: bool,
    /// Asked every time in Manual and Auto: never remembered, never one tap, never Autopilot's. What cannot be undone,
    /// private keys, cloud credentials and saved tokens, and whatever the phone cannot read with certainty.
    pub every_time: bool,
    /// Read with certainty (see [`reins_policy::guard::Reading::certain`]); a detail cut short is not.
    pub certain: bool,
    /// The one group everything the command (or the file list) does falls in: Autopilot learns the question in that
    /// group's own kind. `None`: the generic kind.
    pub kind: Option<&'static str>,
    /// What a remembered answer covers: exactly this file, or exactly this command in this folder (`None`: the
    /// question's topic, for a question that is not about a file).
    pub exact: Option<(String, String)>,
}

/// What the question carries, by the layout the hook writes (`Command:` or `Files:`, then `In:`). Used only to
/// narrow (a kind for Autopilot, what a remembered answer covers), never to decide what is asked.
enum Carried<'a> {
    Command(&'a str, &'a str),
    Files(Vec<&'a str>),
    Other,
}

fn carried(rest: &str) -> Carried<'_> {
    let (body, cwd) = rest.rsplit_once("\n\nIn: ").unwrap_or((rest, ""));
    if let Some(command) = body.strip_prefix("Command:\n") {
        Carried::Command(command, cwd)
    } else if let Some(files) = body.strip_prefix("Files:\n") {
        Carried::Files(files.lines().map(str::trim).filter(|l| !l.is_empty()).collect())
    } else {
        Carried::Other
    }
}

/// A hook's question line repeats the command it carries (`… wants to run: <the command on one line>`, `… wants to
/// read <the file>`): then it says nothing more, and is not read again.
fn repeats(question: &str, rest: &str) -> bool {
    let said = question
        .split_once(" wants to run: ")
        .or_else(|| question.split_once(" wants to read "))
        .or_else(|| question.split_once(" wants to change "))
        .map(|(_, said)| said.trim_end_matches('…').split(" and ").next().unwrap_or_default().trim());
    said.is_some_and(|said| !said.is_empty() && text::one_line(rest).contains(said))
}

/// Reads a desktop question.
///
/// The phone reads everything the question carries with its own copy of the guard's built-in rules
/// (`reins_policy::guard`), and the most severe match wins. What the computer reports (a hook's rule lines, its
/// topic) comes from where the AI runs, so it can only make a question stricter. The words of a destructive command
/// ([`destructive_ask`]) count too, whatever else matched, so nothing reads milder than before.
pub fn read_ask(topic: Option<&str>, question: &str, detail: &str) -> AskReading {
    let (reported, rest) = match reported_rule(detail) {
        Some((rule, rest)) => (Some(rule), rest),
        None => (None, detail),
    };
    let header = &detail[..detail.len() - rest.len()];
    let mut texts = vec![rest];
    texts.extend(topic.and_then(|t| t.split_once(':')).map(|(_, rule)| rule));
    let mut fragments = vec![header];
    if !repeats(question, rest) {
        // A question that says something else: searched (a hook's command line counts toward certainty).
        match question.split_once(" wants to run: ") {
            Some((_, command)) if !command.ends_with('…') => texts.push(command),
            _ => fragments.push(question),
        }
    }
    let reading = reins_policy::guard::read(&texts, &fragments);
    // The hook cuts a detail at 8,000 characters, with an ellipsis: what follows is unknown.
    let certain = reading.certain && !detail.trim_end().ends_with('…') && detail.chars().count() < MAX_DETAIL;
    let reported_group = reported.as_ref().and_then(|r| r.group.as_deref()).and_then(reins_policy::guard::group);
    let destructive = reading.found.as_ref().is_some_and(|c| c.group.destructive)
        || reported.as_ref().is_some_and(|r| r.destructive)
        || destructive_ask(topic, question, detail);
    let every_time = destructive
        || owned(topic).is_some()
        || !certain
        || reading.found.as_ref().is_some_and(|c| c.group.every_time)
        || reported_group.is_some_and(|g| g.every_time);
    let shown = reading.found.as_ref().map(|c| AskRuleView {
        label: c.group.label.to_owned(),
        pattern: c.pattern.to_owned(),
        matched: text::truncate_chars(&text::one_line(&c.matched), MAX_TOPIC),
        group: Some(c.group.id.to_owned()),
        destructive,
        reported: false,
    });
    let rule = match (shown, reported) {
        (Some(own), Some(reported)) if reported.destructive && !own.destructive => Some(AskRuleView {
            destructive,
            ..reported
        }),
        (Some(own), _) => Some(own),
        (None, Some(reported)) => Some(AskRuleView {
            destructive,
            ..reported
        }),
        (None, None) => None,
    };
    let kind = match carried(rest) {
        Carried::Command(command, _) => reins_policy::guard::sole_group(command),
        Carried::Files(files) => reins_policy::guard::sole_file_group(&files),
        Carried::Other => None,
    }
    .filter(|_| certain)
    .map(|g| g.id);
    let about_files = topic.is_some_and(|t| t.trim_start().starts_with("file:"))
        || reading.found.as_ref().is_some_and(|c| !c.group.files.is_empty());
    let exact = about_files.then(|| exact_resource(rest));
    AskReading {
        rule,
        destructive,
        every_time,
        certain,
        kind,
        exact,
    }
}

/// What a remembered answer to a file question covers: exactly that file (`file:/home/me/app/.env`), else exactly
/// that command in that folder (`run:<digest>`), on this computer for this AI (a permission is per connection).
fn exact_resource(rest: &str) -> (String, String) {
    match carried(rest) {
        Carried::Files(files) if files.len() == 1 => {
            let file = text::truncate_chars(&text::one_line(files[0]), MAX_TOPIC);
            (format!("file:{file}"), format!("Reading or changing {file}"))
        }
        carried => {
            let (what, cwd) = match carried {
                Carried::Command(command, cwd) => (command, cwd),
                _ => (rest, ""),
            };
            let digest = ring::digest::digest(&ring::digest::SHA256, format!("{cwd}\n{what}").as_bytes());
            let short = data_encoding::HEXLOWER.encode(&digest.as_ref()[..8]);
            let shown = text::truncate_chars(&text::one_line(what), 60);
            (format!("run:{short}"), format!("Exactly this: {shown}"))
        }
    }
}

/// [`read_ask`]'s rule and whether the question cannot be undone (for tests).
#[cfg(test)]
fn classify(topic: Option<&str>, question: &str, detail: &str) -> (Option<AskRuleView>, bool) {
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
pub(crate) fn reported_rule(detail: &str) -> Option<(AskRuleView, &str)> {
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
/// lower case anywhere in the topic, the question and the detail (the command), whatever else the phone reads in the
/// question (`read_ask`). Finding, moving or reading secrets on the computer (`reins secrets scan`, `import`, `get`) is
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
        // The scan question is worded by the phone (`ask_view`), never by the computer.
        let question = owned(topic.as_deref()).map_or(question, |(o, _)| o.plain.to_owned());
        // The one reading every path uses: what is asked every time is never answered from a notification, with
        // "Approve all" or by a standing answer, and never by Autopilot in Manual or Auto; a remembered answer to a
        // file question covers exactly that file or that command.
        let reading = read_ask(topic.as_deref(), &question, call.str_arg("detail").unwrap_or_default());
        let once_only = reading.every_time;
        let mut lines = vec![question];
        if let Some(t) = &topic {
            lines.push(format!("About: {t}"));
        }
        let (resource, resource_label) = match (reading.exact, topic) {
            (Some(exact), _) => exact,
            (None, Some(t)) => (t.clone(), format!("Questions about {t}")),
            (None, None) => (ASK.to_owned(), "Questions without a topic".to_owned()),
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
            "topic": "command:git push --force"})), "x")
        .unwrap();
        assert_eq!(v.question, "Push to main?");
        assert_eq!(v.detail.as_deref(), Some("git push\n--force"));
        assert_eq!(v.topic.as_deref(), Some("command:git push --force"));
        // No rule lines: the phone reads the rule from the question (here its topic).
        assert_eq!(v.rule.map(|r| (r.group, r.destructive)), Some((Some("git-history".to_owned()), true)));
        let bare = ask_view(&call(&json!({"question": "Ok?", "detail": "  "})), "x").unwrap();
        assert_eq!((bare.detail, bare.topic), (None, None));
        let mut other = call(&json!({}));
        other.op = "git_fetch".to_owned();
        assert!(ask_view(&other, "x").is_none());
    }

    /// The lines `reins_desktop::hooks::rule_lines` writes.
    const ENV_READ: &str =
        "Rule: .env files · .env\nMatched: app/.env\nGroup: env-files\n\nFiles:\napp/.env\n\nIn: /home/me/app";
    const FORCE_PUSH: &str = "Rule: Rewriting git history · git push --force*\nMatched: git push --force origin \
                              main\nGroup: git-history (destructive)\n\nCommand:\ngit push --force origin main";

    #[test]
    fn the_phone_reads_the_rule_from_the_question_itself() {
        let v = ask_view(&call(&json!({"question": "Claude Code wants to read app/.env", "detail": ENV_READ,
            "topic": "file:.env"})), "x")
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
        // The kind Autopilot learns comes from the phone's reading of the command too.
        assert_eq!(read_ask(Some("file:.env"), "Run it?", understated).kind, Some("deletes"));
    }

    /// One reading decides everything, and it never reads milder than the words of a destructive command did.
    #[test]
    fn one_reading_that_is_never_milder_than_before() {
        let env = "Rule: .env files · .env\nMatched: .env\nGroup: env-files\n\nCommand:\n";
        // A mild match does not hide what the destructive words say (`rm -f` is no built-in rule).
        let r = read_ask(Some("file:.env"), "Run it?", &format!("{env}cat .env; rm -f prod.db"));
        assert!(r.destructive && r.every_time);
        // A question line that says something else than the command is read as a command line too, certainty
        // included; one that repeats the command adds nothing.
        let r = read_ask(Some("file:.env"), "Claude Code wants to run: echo 'x", &format!("{env}cat .env"));
        assert!(!r.certain, "the question line's unbalanced quote counts");
        let r = read_ask(Some("file:.env"), "Claude Code wants to run: cat .env", &format!("{env}cat .env"));
        assert!(r.certain && !r.every_time && r.kind == Some("env-files"));
        // Keys, cloud credentials and tokens: asked every time, not held.
        let keys = "Rule: Keys and certificates · id_rsa\nMatched: id_rsa\nGroup: keys\n\nCommand:\ncat ~/.ssh/id_rsa";
        let r = read_ask(Some("file:id_rsa"), "Run it?", keys);
        assert!(r.every_time && !r.destructive);
        // A remembered answer covers exactly that file or that command in that folder.
        let one = read_ask(Some("file:.env"), "Read it?", "Files:\n/home/me/app/.env\n\nIn: /home/me/app").exact;
        assert_eq!(one.map(|(r, _)| r).as_deref(), Some("file:/home/me/app/.env"));
        let here = read_ask(Some("file:.env"), "Run it?", &format!("{env}cat .env\n\nIn: /home/me/a")).exact.unwrap();
        let there = read_ask(Some("file:.env"), "Run it?", &format!("{env}cat .env\n\nIn: /home/me/b")).exact.unwrap();
        assert!(here.0.starts_with("run:") && here.0 != there.0, "another folder, another answer");
        assert_eq!(here.1, "Exactly this: cat .env");
        // A question about no file keeps its topic.
        assert_eq!(read_ask(Some("command:make deploy"), "Run it?", "Command:\nmake deploy").exact, None);
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
            let shown = ask_view(&call(&json!({"question": "Read it?", "detail": format!("{env}{detail}"), "topic": "file:.env",
                "client_key": "k", "nonce": "n"})), "x")
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
            assert!(read_ask(Some(t), "q", "").every_time && t.len() <= MAX_TOPIC, "{t}");
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
            "command:reins secrets scan",
            "command:reins secrets import",
            "command:reins secrets get",
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
