//! A request as the phone tells it: who (an agent's logo) asks for what (a service's logo, with a small badge for the
//! action), in one sentence with the important words in bold, coloured by what it is and how risky; a short title and
//! "who · kind" for a row. Read from the activity log's entry: its kind, who asked, the service, and its `what` and
//! `detail` lines as `reins` writes them.

use reins_desktop::guard::GROUPS;
use reins_desktop::journal::{Decider, Entry, Kind, Outcome};

use super::icons::{Brand, Icon, Mark};
use crate::theme::{Field, Tone};

/// One piece of a sentence; `bold` ones are the words that matter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Words {
    pub text: String,
    pub bold: bool,
}

fn plain(text: impl Into<String>) -> Words {
    Words {
        text: text.into(),
        bold: false,
    }
}

fn bold(text: impl Into<String>) -> Words {
    Words {
        text: text.into(),
        bold: true,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Story {
    /// "Claude Code wants to **force-push** to **acme-web · main**".
    pub sentence: Vec<Words>,
    /// A row's line: "Force-push to acme-web · main".
    pub title: String,
    /// Who and what kind: "Claude Code · Command".
    pub who: String,
    /// The kind of the request, as its colour: delete for history rewrites and deletes, pair (teal) for git, …
    pub tone: Tone,
    /// The kind's icon, in a square of the tone.
    pub icon: Icon,
    /// Who asks (an agent's logo, or this computer).
    pub agent: Mark,
    /// What it is for (a service's logo).
    pub target: Mark,
    /// The action, on the target's corner.
    pub badge: Icon,
    /// A few words on why it is risky ("Rewrites history"), for the risky ones.
    pub risk: Option<&'static str>,
    /// The value row of the detail: what it is called and the value ("Command", `git push --force`).
    pub fact: Option<(&'static str, String)>,
}

/// The built-in rule group an entry was asked by ("(Rewriting git history)" on its `Rule:` line).
fn group_of(e: &Entry) -> Option<&'static str> {
    let detail = e.detail.as_deref()?;
    let line = detail.lines().find(|l| l.trim_start().starts_with("Rule:"))?;
    let label = line.rsplit_once('(')?.1.trim_end().strip_suffix(')')?;
    GROUPS.iter().find(|g| g.label == label).map(|g| g.id)
}

/// How risky a built-in rule group is, as its field: what can't be undone (rewrites, wipes, payments) is delete,
/// what changes infrastructure or data or publishes is push, what reaches secrets is read.
#[must_use]
pub fn group_tone(id: &str) -> Tone {
    Tone::Field(match id {
        "git-history" | "deletes" | "disks" | "payments" => Field::Delete,
        "infrastructure" | "databases" | "publishing" => Field::Push,
        _ => Field::Read,
    })
}

/// A built-in rule group's icon.
#[must_use]
pub fn group_icon(id: &str) -> Icon {
    match id {
        "git-history" => Icon::GitBranch,
        "deletes" => Icon::Trash,
        "infrastructure" => Icon::CloudCog,
        "databases" => Icon::Database,
        "publishing" => Icon::Package,
        "payments" => Icon::CreditCard,
        "secrets" => Icon::ScanSearch,
        "disks" => Icon::HardDrive,
        "env-files" => Icon::FileLock,
        "keys" => Icon::KeyRound,
        "cloud" => Icon::Cloud,
        "reins" => Icon::ShieldCheck,
        _ => Icon::Ticket,
    }
}

/// A rule group's risk in a few words, for a request it asked about.
fn group_risk(id: &str) -> Option<&'static str> {
    Some(match id {
        "git-history" => "Rewrites history",
        "deletes" => "Deletes files",
        "disks" => "Wipes a disk",
        "infrastructure" => "Changes infrastructure",
        "databases" => "Drops data",
        "publishing" => "Publishes",
        "payments" => "Payment key",
        "secrets" => "Finds or reads secrets",
        _ => return None,
    })
}

/// The command a hook asked about: `Command:` in the detail, else after "wants to run:".
fn command_of(e: &Entry) -> String {
    if let Some(detail) = e.detail.as_deref() {
        let mut lines = detail.lines();
        while let Some(line) = lines.next() {
            if line.trim() == "Command:"
                && let Some(next) = lines.next().filter(|l| !l.trim().is_empty())
            {
                return next.trim().to_owned();
            }
        }
    }
    e.what.split_once("wants to run:").map_or_else(|| e.what.clone(), |(_, c)| c.trim().to_owned())
}

/// The folder a hook ran in (`In: ~/code/acme-web` → `acme-web`).
fn folder_of(e: &Entry) -> Option<String> {
    let line = e.detail.as_deref()?.lines().find_map(|l| l.trim().strip_prefix("In:"))?;
    let name = line.trim().trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next()?;
    (!name.is_empty() && name != "~").then(|| name.to_owned())
}

/// Who asked, by name.
fn who_of(e: &Entry) -> String {
    e.source.clone().filter(|s| !s.trim().is_empty()).unwrap_or_else(|| "A program".to_owned())
}

/// An agent's or program's mark.
#[must_use]
pub fn agent_mark(source: Option<&str>) -> Mark {
    let s = source.unwrap_or("").to_ascii_lowercase();
    if s.contains("claude") {
        Mark::Brand(Brand::Claude)
    } else if s.contains("gemini") {
        Mark::Brand(Brand::Gemini)
    } else if s.contains("cursor") {
        Mark::Brand(Brand::Cursor)
    } else if s.contains("codex") || s.contains("reins run") {
        Mark::Icon(Icon::SquareTerminal)
    } else {
        Mark::Icon(Icon::Laptop)
    }
}

fn capitalised(s: &str) -> String {
    let mut chars = s.chars();
    chars.next().map_or_else(String::new, |c| c.to_uppercase().chain(chars).collect())
}

/// "wants to" while it waits, "asked to" once it ended.
fn wants(e: &Entry) -> &'static str {
    if e.outcome == Outcome::Waiting {
        " wants to "
    } else {
        " asked to "
    }
}

/// `push to acme/web (main)` → (`push`, `acme/web · main`).
fn git_parts(what: &str) -> (String, String) {
    let (verb, rest) = what.split_once(" to ").or_else(|| what.split_once(' ')).unwrap_or((what, ""));
    let rest = match rest.rsplit_once(" (") {
        Some((repo, branch)) => format!("{repo} · {}", branch.trim_end_matches(')')),
        None => rest.to_owned(),
    };
    (verb.trim().to_owned(), rest.trim().to_owned())
}

/// An MCP tool's action and service: `gmail_send` → ("send an email", "Gmail").
fn mcp_parts(tool: &str) -> (String, Option<String>, Mark, Icon) {
    let (service, action) = tool.split_once('_').unwrap_or((tool, ""));
    let (mark, name) = match service {
        "gmail" => (Mark::Brand(Brand::Gmail), Some("Gmail")),
        "github" => (Mark::Brand(Brand::GitHub), Some("GitHub")),
        "calendar" => (Mark::Icon(Icon::Calendar), Some("Calendar")),
        _ => (Mark::Icon(Icon::Wrench), None),
    };
    let (verb, badge) = match (service, action) {
        ("gmail", "send") => ("send an email".to_owned(), Icon::Send),
        ("gmail", "read" | "search") => ("read your email".to_owned(), Icon::Mail),
        ("calendar", "create_event") => ("create an event".to_owned(), Icon::Plus),
        ("github", "create_issue") => ("open an issue".to_owned(), Icon::Plus),
        _ => (format!("use {tool}"), Icon::Wrench),
    };
    (verb, name.map(str::to_owned), mark, badge)
}

impl Story {
    #[must_use]
    pub fn of(e: &Entry) -> Self {
        let who = who_of(e);
        let kind = e.kind.label();
        let who_line = if e.source.as_deref().is_some_and(|s| !s.eq_ignore_ascii_case(kind)) {
            format!("{who} · {kind}")
        } else {
            kind.to_owned()
        };
        let agent = agent_mark(e.source.as_deref());
        let mut s = Self {
            sentence: vec![plain(e.what.clone())],
            title: e.what.clone(),
            who: who_line,
            tone: Tone::Field(Field::Push),
            icon: Icon::SquareTerminal,
            agent,
            target: Mark::Icon(Icon::SquareTerminal),
            badge: Icon::Terminal,
            risk: None,
            fact: None,
        };
        match e.kind {
            Kind::Command => {
                let cmd = command_of(e);
                let group = group_of(e);
                let first = cmd.split_whitespace().collect::<Vec<_>>();
                let force_push = first.starts_with(&["git", "push"])
                    && first.iter().any(|a| matches!(*a, "--force" | "-f" | "--force-with-lease"));
                let delete = first.first().is_some_and(|c| matches!(*c, "rm" | "rd" | "rmdir" | "Remove-Item"));
                s.tone = group.map_or(Tone::Field(Field::Push), group_tone);
                s.risk = group.and_then(group_risk);
                s.icon = group.map_or(Icon::SquareTerminal, group_icon);
                s.badge = s.icon;
                s.fact = Some(("Command", cmd.clone()));
                if force_push {
                    let branch = first.iter().rev().find(|a| !a.starts_with('-')).copied().unwrap_or("");
                    let to = match folder_of(e) {
                        Some(folder) => format!("{folder} · {branch}"),
                        None => branch.to_owned(),
                    };
                    s.sentence =
                        vec![plain(format!("{who}{}", wants(e))), bold("force-push"), plain(" to "), bold(&to)];
                    s.title = format!("Force-push to {to}");
                    s.icon = Icon::GitBranch;
                    s.badge = Icon::GitBranch;
                    s.target = Mark::Icon(Icon::GitBranch);
                } else if delete {
                    let what = first.iter().skip(1).filter(|a| !a.starts_with('-')).copied().collect::<Vec<_>>();
                    s.sentence =
                        vec![plain(format!("{who}{}", wants(e))), bold("delete"), plain(" "), bold(what.join(" "))];
                    s.title = cmd;
                    s.icon = Icon::Trash;
                    s.badge = Icon::Trash;
                    s.target = Mark::Icon(Icon::HardDrive);
                } else {
                    s.sentence = vec![plain(format!("{who}{}", wants(e))), plain("run "), bold(&cmd)];
                    s.title = cmd;
                    s.target = Mark::Icon(Icon::SquareTerminal);
                }
            }
            Kind::File => {
                let (verb, path) = e
                    .what
                    .split_once("wants to ")
                    .and_then(|(_, rest)| rest.split_once(' '))
                    .map_or(("open", e.what.as_str()), |(v, p)| (v, p));
                let group = group_of(e);
                s.tone = Tone::Field(Field::Read);
                s.icon = Icon::FileLock;
                s.badge = Icon::FileLock;
                s.target = Mark::Icon(group.map_or(Icon::FileLock, group_icon));
                s.sentence = vec![plain(format!("{who}{}", wants(e))), bold(verb), plain(" "), bold(path)];
                s.title = format!("{} {path}", capitalised(verb));
                s.fact = Some(("File", path.to_owned()));
            }
            Kind::Mcp => {
                let tool = e.service.clone().unwrap_or_else(|| e.what.clone());
                let (verb, service, target, badge) = mcp_parts(&tool);
                // Reads and searches are read; everything else a tool does acts (push).
                s.tone = Tone::Field(if badge == Icon::Mail {
                    Field::Read
                } else {
                    Field::Push
                });
                s.icon = if target == Mark::Brand(Brand::Gmail) {
                    Icon::Mail
                } else {
                    Icon::Wrench
                };
                s.target = target;
                s.badge = badge;
                s.sentence = vec![plain(format!("{who}{}", wants(e))), bold(&verb)];
                if let Some(service) = &service {
                    s.sentence.push(plain(" with "));
                    s.sentence.push(bold(service));
                }
                s.title = match &service {
                    Some(service) => format!("{} with {service}", capitalised(&verb)),
                    None => capitalised(&verb),
                };
                s.fact = Some(("Tool", tool));
            }
            Kind::Git => {
                let (verb, to) = git_parts(&e.what);
                let host = e.service.as_deref().unwrap_or("");
                s.tone = Tone::Field(Field::Pair);
                s.icon = Icon::GitBranch;
                s.badge = Icon::GitBranch;
                s.agent = Mark::Icon(Icon::Laptop);
                s.target = Brand::of_host(host).map_or(Mark::Icon(Icon::GitBranch), Mark::Brand);
                s.sentence = vec![plain(format!("git{}", wants(e))), bold(&verb), plain(" to "), bold(&to)];
                s.title = format!("{} to {to}", capitalised(&verb));
                let host_name = host.split('/').next().unwrap_or(host);
                s.who = format!("git · {host_name}");
                s.fact = Some(("Repository", host.to_owned()));
            }
            Kind::Api => {
                let name =
                    e.service.clone().or_else(|| e.what.rsplit(' ').next().map(str::to_owned)).unwrap_or_default();
                s.tone = Tone::Field(Field::Read);
                s.icon = Icon::KeyRound;
                s.badge = Icon::KeyRound;
                s.target = if name.contains("anthropic") {
                    Mark::Brand(Brand::Anthropic)
                } else {
                    Mark::Icon(Icon::Globe)
                };
                // Who calls it is on the detail's first line ("Codex calls https://…").
                let caller = e
                    .detail
                    .as_deref()
                    .and_then(|d| d.lines().next())
                    .and_then(|l| l.split_once(" calls "))
                    .map(|(c, _)| c.trim().to_owned());
                if let Some(c) = &caller {
                    s.agent = agent_mark(Some(c));
                }
                let who = caller.unwrap_or(who);
                s.sentence = vec![plain(format!("{who}{}", wants(e))), plain("use "), bold(format!("the {name} key"))];
                s.title = format!("The key for {name}");
                "API key".clone_into(&mut s.who);
                s.fact = Some(("API", name));
            }
            Kind::Ssh => {
                let host = e.service.clone().unwrap_or_default();
                let key = e.what.split_once(" with ").map(|(_, k)| k.to_owned());
                s.tone = Tone::Neutral;
                s.icon = Icon::Terminal;
                s.badge = Icon::KeyRound;
                s.agent = Mark::Icon(Icon::Laptop);
                s.target = Brand::of_host(&host).map_or(Mark::Icon(Icon::Server), Mark::Brand);
                s.sentence = vec![plain(format!("ssh{}", wants(e))), bold("sign in"), plain(" to "), bold(&host)];
                s.title = match &key {
                    Some(k) => format!("ssh {host} with {k}"),
                    None => format!("ssh {host}"),
                };
                "SSH".clone_into(&mut s.who);
                s.fact = Some(("Host", host));
            }
            Kind::Secrets => {
                let what = e.what.strip_prefix("secrets for ").unwrap_or(&e.what).trim_matches('`').to_owned();
                s.tone = Tone::Field(Field::Grant);
                s.icon = Icon::Lock;
                s.badge = Icon::KeyRound;
                s.target = Mark::Icon(Icon::Lock);
                s.sentence = vec![
                    plain(format!("{who}{}", wants(e))),
                    plain("get "),
                    bold("secrets"),
                    plain(" for "),
                    bold(&what),
                ];
                s.title = format!("Secrets for {what}");
                s.fact = Some(("Command", what));
            }
            Kind::Ask => {
                s.tone = Tone::Field(Field::Grant);
                s.icon = Icon::Question;
                s.badge = Icon::Question;
                s.target = Mark::Icon(Icon::Smartphone);
                s.sentence = vec![plain(format!("{who} asks: ")), bold(&e.what)];
            }
        }
        s
    }
}

/// How a row ends: an icon in a colour, or a chip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ending {
    Icon(Icon, Tone),
    /// "By your rules" (let through without asking).
    Rules,
    /// "Waiting", violet.
    Waiting,
}

#[must_use]
pub fn ending(e: &Entry) -> Ending {
    match e.outcome {
        Outcome::Waiting => Ending::Waiting,
        Outcome::Approved => Ending::Icon(Icon::CircleCheck, Tone::Success),
        Outcome::Denied | Outcome::Blocked => Ending::Icon(Icon::CircleX, Tone::Field(Field::Delete)),
        Outcome::TimedOut => Ending::Icon(Icon::TimerOff, Tone::Field(Field::Warn)),
        Outcome::Failed => Ending::Icon(Icon::CircleAlert, Tone::Field(Field::Delete)),
        Outcome::Allowed => Ending::Rules,
    }
}

/// The detail's chip: how it ended and where ("Approved on Dana's iPhone").
#[must_use]
pub fn answer(e: &Entry, phone: &str) -> (Icon, Tone, String) {
    let on = |word: &str| match e.decider {
        Decider::Phone => format!("{word} on {phone}"),
        Decider::Local => format!("{word} here"),
        Decider::Settings => format!("{word} by your rules"),
    };
    match e.outcome {
        Outcome::Waiting => (Icon::Smartphone, Tone::Field(Field::Grant), "Waiting on your phone".to_owned()),
        Outcome::Approved => (Icon::Smartphone, Tone::Success, on("Approved")),
        Outcome::Denied => (Icon::Smartphone, Tone::Field(Field::Delete), on("Denied")),
        Outcome::TimedOut => (Icon::TimerOff, Tone::Field(Field::Warn), "No answer".to_owned()),
        Outcome::Failed => (Icon::CircleAlert, Tone::Field(Field::Delete), "Couldn't ask".to_owned()),
        Outcome::Allowed => (Icon::Check, Tone::Neutral, "Allowed by your rules".to_owned()),
        Outcome::Blocked => (Icon::CircleX, Tone::Field(Field::Delete), "Blocked by your rules".to_owned()),
    }
}

/// How long the phone has to answer `e`, in seconds: the hooks' wait for commands and files, the approvals' for the
/// rest.
#[must_use]
pub fn wait_secs(kind: Kind, hook_wait: u64, approval_wait: u64) -> u64 {
    match kind {
        Kind::Command | Kind::File => hook_wait,
        _ => approval_wait,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(kind: Kind, source: Option<&str>, service: Option<&str>, what: &str, detail: Option<&str>) -> Entry {
        let mut e = Entry::new(kind, what).source(source).service(service);
        e.detail = detail.map(str::to_owned);
        e.outcome = Outcome::Waiting;
        e
    }

    fn said(s: &Story) -> String {
        s.sentence
            .iter()
            .map(|w| {
                if w.bold {
                    format!("*{}*", w.text)
                } else {
                    w.text.clone()
                }
            })
            .collect()
    }

    #[test]
    fn a_force_push_is_red_and_says_where() {
        let e = entry(
            Kind::Command,
            Some("Claude Code"),
            Some("command:git push --force*"),
            "Claude Code wants to run: git push --force origin main",
            Some(
                "Command:\ngit push --force origin main\n\nIn: ~/code/acme-web\nRule: git push --force* (Rewriting git history)",
            ),
        );
        let s = Story::of(&e);
        assert_eq!(said(&s), "Claude Code wants to *force-push* to *acme-web · main*");
        assert_eq!((s.tone, s.risk), (Tone::Field(Field::Delete), Some("Rewrites history")));
        assert_eq!(s.agent, Mark::Brand(Brand::Claude));
        assert_eq!(s.title, "Force-push to acme-web · main");
        assert_eq!(s.who, "Claude Code · Command");
        assert_eq!(s.fact, Some(("Command", "git push --force origin main".to_owned())));
    }

    #[test]
    fn deletes_commands_and_files() {
        let mut rm = entry(
            Kind::Command,
            Some("Claude Code"),
            None,
            "Claude Code wants to run: rm -rf /",
            Some("Command:\nrm -rf /\n\nIn: ~/code/acme-web\nRule: rm -r (Recursive deletes)"),
        );
        rm.outcome = Outcome::Denied;
        let s = Story::of(&rm);
        assert_eq!(said(&s), "Claude Code asked to *delete* */*");
        assert_eq!((s.tone, s.icon, s.title.as_str()), (Tone::Field(Field::Delete), Icon::Trash, "rm -rf /"));

        let ls = Story::of(&entry(Kind::Command, Some("Codex"), None, "Codex wants to run: make deploy", None));
        assert_eq!(said(&ls), "Codex wants to run *make deploy*");
        assert_eq!(ls.tone, Tone::Field(Field::Push));

        let env = Story::of(&entry(Kind::File, Some("Claude Code"), None, "Claude Code wants to read .env", None));
        assert_eq!(said(&env), "Claude Code wants to *read* *.env*");
        assert_eq!(env.title, "Read .env");
    }

    #[test]
    fn tools_git_keys_ssh_and_questions() {
        let mail = Story::of(&entry(Kind::Mcp, Some("Codex"), Some("gmail_send"), "Codex: gmail_send", None));
        assert_eq!(said(&mail), "Codex wants to *send an email* with *Gmail*");
        assert_eq!((mail.target, mail.badge), (Mark::Brand(Brand::Gmail), Icon::Send));
        assert_eq!(mail.title, "Send an email with Gmail");

        let other = Story::of(&entry(Kind::Mcp, Some("Codex"), Some("notion_search"), "Codex: notion_search", None));
        assert_eq!(said(&other), "Codex wants to *use notion_search*");

        let push =
            Story::of(&entry(Kind::Git, Some("git"), Some("github.com/acme/web"), "push to acme/web (main)", None));
        assert_eq!(said(&push), "git wants to *push* to *acme/web · main*");
        assert_eq!((push.target, push.who.as_str()), (Mark::Brand(Brand::GitHub), "git · github.com"));
        assert_eq!(push.title, "Push to acme/web · main");

        let key = Story::of(&entry(
            Kind::Api,
            None,
            Some("anthropic"),
            "the key for API anthropic",
            Some("Codex calls https://api.anthropic.com through Reins"),
        ));
        assert_eq!(said(&key), "Codex wants to use *the anthropic key*");
        assert_eq!((key.target, key.agent), (Mark::Brand(Brand::Anthropic), Mark::Icon(Icon::SquareTerminal)));

        let ssh = Story::of(&entry(
            Kind::Ssh,
            Some("ssh"),
            Some("github.com"),
            "sign in to github.com as git with MacBook key",
            None,
        ));
        assert_eq!(ssh.title, "ssh github.com with MacBook key");
        assert_eq!(ssh.who, "SSH");

        let ask = Story::of(&entry(Kind::Ask, Some("Claude Code"), None, "Deploy now?", None));
        assert_eq!(said(&ask), "Claude Code asks: *Deploy now?*");
    }

    #[test]
    fn rule_groups_have_a_colour_and_an_icon_each() {
        for g in GROUPS {
            let _ = (group_tone(g.id), group_icon(g.id));
        }
        assert_eq!(group_tone("git-history"), Tone::Field(Field::Delete));
        assert_eq!(group_tone("databases"), Tone::Field(Field::Push));
        assert_eq!(group_tone("payments"), Tone::Field(Field::Delete));
        assert_eq!(group_tone("env-files"), Tone::Field(Field::Read));
        let icons: std::collections::HashSet<_> = GROUPS.iter().map(|g| group_icon(g.id)).collect();
        assert_eq!(icons.len(), GROUPS.len());
    }

    #[test]
    fn endings_and_answers() {
        let mut e = entry(Kind::Git, None, None, "push", None);
        assert_eq!(ending(&e), Ending::Waiting);
        e.outcome = Outcome::Approved;
        assert_eq!(answer(&e, "Dana's iPhone").2, "Approved on Dana's iPhone");
        e.outcome = Outcome::Allowed;
        e.decider = Decider::Settings;
        assert_eq!(ending(&e), Ending::Rules);
        assert_eq!(wait_secs(Kind::Command, 120, 300), 120);
        assert_eq!(wait_secs(Kind::Git, 120, 300), 300);
    }
}
