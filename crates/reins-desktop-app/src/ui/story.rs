//! How a request is written and marked (`design/signal/ASK-TEXT.md`): the one formatter every screen uses (the waiting
//! field, Activity's rows and detail, Home's latest, the notification, the tray). From an activity entry, as `reins`
//! writes it, it builds:
//!
//! - the kind's label line ("GitHub · Push");
//! - one sentence in the serif: who, a verb in the request's tense, a count, and the target, the only italic run
//!   ("git pushed 3 commits to *katulevskiy/reins*"); and the same as an order for the waiting field ("Push 3 commits
//!   to *octo/app*");
//! - the machine's details on a mono line of their own (`signal/desktop-release · new branch · github.com`), never in
//!   the serif;
//! - the icon: the integration's mark with the action on a badge in the kind's colour;
//! - the facts as rows, and the detail's other lines for the transcript.
//!
//! Waiting requests are present tense ("wants to push"); decided ones past ("pushed", or "wanted to push" when they
//! did not happen). Nothing the user approves is cut here: long values are the screens' to wrap.

use reins_desktop::guard::GROUPS;
use reins_desktop::journal::{Decider, Entry, Kind, Outcome};

use super::icons::{Brand, Glyph, Mark};
use crate::theme::{Field, Tone};

/// One piece of a sentence; the `bold` one (there is one) is the target, set in italic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Words {
    pub text: String,
    pub bold: bool,
}

pub fn plain(text: impl Into<String>) -> Words {
    Words {
        text: text.into(),
        bold: false,
    }
}

pub fn bold(text: impl Into<String>) -> Words {
    Words {
        text: text.into(),
        bold: true,
    }
}

/// How a fact's value is set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FactStyle {
    /// Words.
    Text,
    /// The machine's: a command, a path, a SHA; wrapped, never cut.
    Mono,
    /// Lines added and removed, in their colours: `+128 −31 · 9 files`.
    Changes,
}

/// A fact about a request, as a row: its key and its value (with a mono tail after it: a rule's pattern).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fact {
    pub key: String,
    pub value: String,
    pub style: FactStyle,
    pub tail: Option<String>,
}

fn fact(key: &str, value: impl Into<String>, style: FactStyle) -> Fact {
    Fact {
        key: key.to_owned(),
        value: value.into(),
        style,
        tail: None,
    }
}

/// A request, written and marked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ask {
    /// The label line: the kind or integration ("GitHub", "Command", "Gmail") and the action ("Push", "Run", "Send").
    pub kind: String,
    pub action: String,
    /// The sentence in the request's tense, with who asked.
    pub sentence: Vec<Words>,
    /// The same as an order, for the waiting field (its top bar says who asks).
    pub order: Vec<Words>,
    /// The machine's details, one mono line.
    pub details: Option<String>,
    /// Who asked ("Claude Code", "git").
    pub who: String,
    /// Who asked, as a mark (the waiting field's top bar).
    pub agent: Mark,
    /// The integration's mark, and the action on its badge.
    pub mark: Mark,
    pub badge: Glyph,
    /// The kind's colour: the badge's, the waiting field's.
    pub field: Field,
    /// The facts, as rows.
    pub facts: Vec<Fact>,
    /// The detail's other lines (what matched, the rule's group, a question's text), for the transcript.
    pub notes: Vec<String>,
}

impl Ask {
    /// The sentence as plain text: a row's title, the tray's line.
    #[must_use]
    pub fn line(&self) -> String {
        self.sentence.iter().map(|w| w.text.as_str()).collect()
    }

    /// The label line's words: "GitHub · Push".
    #[must_use]
    pub fn label(&self) -> String {
        format!("{} · {}", self.kind, self.action)
    }
}

/// When a request stands, for its tense.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tense {
    /// Waiting: "wants to push".
    Waiting,
    /// It happened: "pushed".
    Done,
    /// Decided, and it did not happen: "wanted to push".
    Not,
}

fn tense(e: &Entry) -> Tense {
    match e.outcome {
        Outcome::Waiting => Tense::Waiting,
        Outcome::Approved | Outcome::Allowed => Tense::Done,
        Outcome::Denied | Outcome::TimedOut | Outcome::Failed | Outcome::Blocked => Tense::Not,
    }
}

/// A verb: its base ("push") and past ("pushed").
#[derive(Clone, Copy)]
struct Verb(&'static str, &'static str);

/// The sentence and the order: `who` `verb` `before` *`target`* `after`.
fn say(who: &str, verb: Verb, before: &str, target: &str, after: &str, t: Tense) -> (Vec<Words>, Vec<Words>) {
    let lead = match t {
        Tense::Waiting => format!("{who} wants to {} ", verb.0),
        Tense::Done => format!("{who} {} ", verb.1),
        Tense::Not => format!("{who} wanted to {} ", verb.0),
    };
    let tail = |w: Vec<Words>| -> Vec<Words> {
        let mut w = w;
        if !after.is_empty() {
            w.push(plain(after));
        }
        w
    };
    let sentence = tail(vec![plain(format!("{lead}{before}")), bold(target)]);
    let order = tail(vec![plain(format!("{} {before}", capitalised(verb.0))), bold(target)]);
    (sentence, order)
}

fn capitalised(s: &str) -> String {
    let mut chars = s.chars();
    chars.next().map_or_else(String::new, |c| c.to_uppercase().chain(chars).collect())
}

/// "1 commit", "3 commits".
fn count(n: u64, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

/// "an issue", "a pull request".
fn article(thing: &str) -> String {
    if thing.starts_with(['a', 'e', 'i', 'o', 'u']) {
        format!("an {thing}")
    } else {
        format!("a {thing}")
    }
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
        Mark::Glyph(Glyph::Terminal)
    } else {
        Mark::Glyph(Glyph::Laptop)
    }
}

/// How risky a built-in rule group is, as its field: what can't be undone (rewrites, wipes, payments) is delete,
/// what changes infrastructure or data or publishes is push, what reaches secrets is read.
#[must_use]
pub fn group_tone(id: &str) -> Tone {
    Tone::Field(group_field(id))
}

fn group_field(id: &str) -> Field {
    match id {
        "git-history" | "deletes" | "disks" | "payments" => Field::Delete,
        "infrastructure" | "databases" | "publishing" => Field::Push,
        _ => Field::Read,
    }
}

/// A built-in rule group's icon.
#[must_use]
pub fn group_icon(id: &str) -> Glyph {
    match id {
        "git-history" => Glyph::GitBranch,
        "deletes" => Glyph::Trash,
        "infrastructure" => Glyph::CloudCog,
        "databases" => Glyph::Database,
        "publishing" => Glyph::Package,
        "payments" => Glyph::Card,
        "secrets" => Glyph::ScanSearch,
        "disks" => Glyph::HardDrive,
        "env-files" => Glyph::FileLock,
        "keys" => Glyph::Key,
        "cloud" => Glyph::Cloud,
        "reins" => Glyph::Shield,
        _ => Glyph::Token,
    }
}

/// An entry's detail, read: its `Key: value` lines, its blocks (`Command:` and `Files:` followed by lines up to a
/// blank one), and the rest.
#[derive(Default)]
struct Detail {
    keys: Vec<(String, String)>,
    command: Option<String>,
    files: Vec<String>,
    rest: Vec<String>,
}

impl Detail {
    fn of(e: &Entry) -> Self {
        let mut d = Self::default();
        let mut lines = e.detail.as_deref().unwrap_or("").lines().peekable();
        while let Some(line) = lines.next() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if trimmed == "Command:" || trimmed == "Files:" {
                let mut block = Vec::new();
                while let Some(l) = lines.next_if(|l| !l.trim().is_empty()) {
                    block.push(l);
                }
                if trimmed == "Command:" {
                    d.command = Some(block.join("\n"));
                } else {
                    d.files = block.iter().map(|l| l.trim().to_owned()).collect();
                }
                continue;
            }
            match trimmed.split_once(": ") {
                Some((k, v)) if k.len() <= 24 && !k.contains(' ') || matches!(k, "Allowed by your rule" | "Run by") => {
                    d.keys.push((k.to_owned(), v.trim().to_owned()));
                }
                _ => d.rest.push(trimmed.to_owned()),
            }
        }
        d
    }

    fn get(&self, key: &str) -> Option<&str> {
        self.keys.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    fn all(&self, key: &str) -> impl Iterator<Item = &str> {
        self.keys.iter().filter(move |(k, _)| k == key).map(|(_, v)| v.as_str())
    }
}

/// The rule a hook matched: its group (by `Group:`, or by the rule's name), its name and its pattern.
struct Rule {
    group: Option<&'static str>,
    name: String,
    pattern: Option<String>,
}

fn rule_of(d: &Detail) -> Option<Rule> {
    let line = d.get("Rule").or_else(|| d.get("Allowed by your rule"))?;
    // `Rule: Recursive deletes · rm -r`, or (older entries) `Rule: rm -r (Recursive deletes)`.
    let (name, pattern) = match line.split_once(" · ") {
        Some((n, p)) => (n.trim().to_owned(), Some(p.trim().to_owned())),
        None => match line.rsplit_once(" (") {
            Some((p, n)) => (n.trim_end_matches(')').to_owned(), Some(p.trim().to_owned())),
            None if d.get("Rule").is_none() => ("Your rule".to_owned(), Some(line.to_owned())),
            None => (line.to_owned(), None),
        },
    };
    let by_group = d
        .get("Group")
        .map(|g| g.split_whitespace().next().unwrap_or(""))
        .and_then(|id| GROUPS.iter().find(|g| g.id == id));
    let group = by_group.or_else(|| GROUPS.iter().find(|g| g.label == name)).map(|g| g.id);
    Some(Rule {
        group,
        name,
        pattern,
    })
}

/// The commands a command line runs, one by one: split at `&&`, `||`, `;` and line ends outside quotes (a pipeline
/// is one command).
#[must_use]
pub fn commands_in(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let (mut single, mut double, mut escaped) = (false, false, false);
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if escaped {
            cur.push(c);
            escaped = false;
            continue;
        }
        let quoted = single || double;
        match c {
            '\\' if !single => {
                escaped = true;
                cur.push(c);
            }
            '\'' if !double => {
                single = !single;
                cur.push(c);
            }
            '"' if !single => {
                double = !double;
                cur.push(c);
            }
            ';' | '\n' if !quoted => out.push(std::mem::take(&mut cur)),
            '&' | '|' if !quoted && chars.peek() == Some(&c) => {
                chars.next();
                out.push(std::mem::take(&mut cur));
            }
            _ => cur.push(c),
        }
    }
    out.push(cur);
    out.into_iter().map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()).collect()
}

/// The last folder of a path (`~/code/acme-web` → `acme-web`).
fn last_part(path: &str) -> Option<String> {
    let name = path.trim().trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next()?;
    (!name.is_empty() && name != "~").then(|| name.to_owned())
}

/// A git host's brand and the name its label line uses.
fn host_brand(host: &str) -> (Option<Brand>, &'static str) {
    match Brand::of_host(host) {
        Some(Brand::GitHub) => (Some(Brand::GitHub), "GitHub"),
        Some(Brand::GitLab) => (Some(Brand::GitLab), "GitLab"),
        Some(Brand::Codeberg) => (Some(Brand::Codeberg), "Codeberg"),
        Some(Brand::Bitbucket) => (Some(Brand::Bitbucket), "Bitbucket"),
        _ => (None, "git"),
    }
}

/// `github.com/acme/web` → (`github.com`, `acme/web`).
fn split_repo(label: &str) -> (String, String) {
    match label.split_once('/') {
        Some((host, path)) if host.contains('.') => (host.to_owned(), path.to_owned()),
        _ => (String::new(), label.to_owned()),
    }
}

impl Ask {
    #[must_use]
    pub fn of(e: &Entry) -> Self {
        let d = Detail::of(e);
        let who = who_of(e);
        let t = tense(e);
        let mut a = Self {
            kind: e.kind.label().to_owned(),
            action: "Ask".to_owned(),
            sentence: vec![plain(format!("{who}: ")), bold(e.what.clone())],
            order: vec![bold(e.what.clone())],
            details: None,
            who,
            agent: agent_mark(e.source.as_deref()),
            mark: Mark::Glyph(Glyph::Help),
            badge: Glyph::Help,
            field: Field::of(e.kind),
            facts: Vec::new(),
            notes: Vec::new(),
        };
        match e.kind {
            Kind::Command => a.command(e, &d, t),
            Kind::File => a.file(e, &d, t),
            Kind::Git => a.git(e, &d, t),
            Kind::Ssh => a.ssh(e, &d, t),
            Kind::Api => a.api(e, &d, t),
            Kind::Secrets => a.secrets(e, &d, t),
            Kind::Mcp => a.mcp(e, &d, t),
            Kind::Ask if e.source.as_deref() == Some("work session") => a.work_session(e, &d, t),
            Kind::Ask => a.question(e, &d, t),
        }
        a
    }

    fn set(&mut self, (sentence, order): (Vec<Words>, Vec<Words>)) {
        self.sentence = sentence;
        self.order = order;
    }

    /// The rule's row and the transcript's lines about what matched.
    fn rule_facts(&mut self, d: &Detail, rule: Option<&Rule>) {
        if let Some(r) = rule {
            self.facts.push(Fact {
                tail: r.pattern.clone(),
                ..fact("Rule", r.name.clone(), FactStyle::Text)
            });
        }
        if let Some(m) = d.get("Matched") {
            self.notes.push(format!("Matched: {m}"));
        }
        if let Some(g) = d.get("Group") {
            self.notes.push(format!("Group: {g}"));
        }
    }

    fn command(&mut self, e: &Entry, d: &Detail, t: Tense) {
        let full = d.command.clone().unwrap_or_else(|| {
            e.what.split_once("wants to run:").map_or_else(|| e.what.clone(), |(_, c)| c.trim().to_owned())
        });
        let rule = rule_of(d);
        let place = d.get("In").and_then(last_part).unwrap_or_else(|| "this computer".to_owned());
        self.set(say(&self.who.clone(), Verb("run", "ran"), "a command in ", &place, "", t));
        let parts = commands_in(&full);
        let matched = d.get("Matched").map(str::to_owned).or_else(|| parts.first().cloned()).unwrap_or_default();
        let more = parts.len().saturating_sub(1);
        self.details = Some(if more > 0 {
            format!("{matched} · and {}", count(more as u64, "more command", "more commands"))
        } else {
            matched
        });
        self.kind = "Command".into();
        self.action = "Run".into();
        self.mark = Mark::Glyph(Glyph::Terminal);
        let group = rule.as_ref().and_then(|r| r.group);
        self.field = group.map_or(Field::Push, group_field);
        self.badge = match group {
            Some("deletes" | "disks") => Glyph::Trash,
            _ => Glyph::Play,
        };
        self.rule_facts(d, rule.as_ref());
        if let Some(dir) = d.get("In") {
            self.facts.push(fact("In", dir, FactStyle::Mono));
        }
        self.facts.push(fact("Command", full, FactStyle::Mono));
    }

    fn file(&mut self, e: &Entry, d: &Detail, t: Tense) {
        // `Claude Code wants to read /a/.env and 2 more file(s)`.
        let rest = e.what.split_once(" wants to ").map_or(e.what.as_str(), |(_, r)| r);
        let (verb, path) = rest.split_once(' ').unwrap_or(("open", rest));
        let path = path.split(" and ").next().unwrap_or(path).trim();
        let change = verb == "change";
        let name = last_part(path).unwrap_or_else(|| path.to_owned());
        let verb = if change {
            Verb("change", "changed")
        } else {
            Verb("read", "read")
        };
        self.set(say(&self.who.clone(), verb, "", &name, "", t));
        let others = d.files.len().saturating_sub(1);
        self.details = Some(if others > 0 {
            format!("{path} · and {}", count(others as u64, "more file", "more files"))
        } else {
            path.to_owned()
        });
        self.kind = "File".into();
        self.action = capitalised(verb.0);
        self.mark = Mark::Glyph(Glyph::File);
        self.badge = if change {
            Glyph::Pencil
        } else {
            Glyph::Eye
        };
        self.field = if change {
            Field::Push
        } else {
            Field::Read
        };
        let rule = rule_of(d);
        self.rule_facts(d, rule.as_ref());
        let files = if d.files.is_empty() {
            path.to_owned()
        } else {
            d.files.join("\n")
        };
        self.facts.push(fact(
            if d.files.len() > 1 {
                "Files"
            } else {
                "File"
            },
            files,
            FactStyle::Mono,
        ));
        if let Some(dir) = d.get("In") {
            self.facts.push(fact("In", dir, FactStyle::Mono));
        }
    }

    fn git(&mut self, e: &Entry, d: &Detail, t: Tense) {
        let label = e.service.clone().unwrap_or_default();
        let (host, path) = split_repo(&label);
        let (brand, name) = host_brand(&host);
        self.kind = name.into();
        self.who = "git".into();
        self.agent = Mark::Glyph(Glyph::Laptop);
        self.mark = brand.map_or(Mark::Glyph(Glyph::GitBranch), Mark::Brand);
        if let Some(rest) = e.what.strip_prefix("push to ") {
            // `push to github.com/acme/web: feature/x (3 commits), delete old`.
            let refs: Vec<&str> = rest.split_once(": ").map_or(vec![], |(_, r)| r.split(", ").collect());
            let commits: u64 =
                refs.iter().filter_map(|r| r.rsplit_once(" (")?.1.split_whitespace().next()?.parse::<u64>().ok()).sum();
            let deletes_only = !refs.is_empty() && refs.iter().all(|r| r.starts_with("delete "));
            let ref_lines: Vec<&str> = d.all("Ref").collect();
            let forced = ref_lines.iter().any(|r| r.ends_with("force push") || r.ends_with("moved tag"));
            let target = if path.is_empty() {
                label
            } else {
                path
            };
            if deletes_only {
                self.set(say("git", Verb("delete", "deleted"), "a branch of ", &target, "", t));
                self.action = "Delete".into();
                self.badge = Glyph::Trash;
                self.field = Field::Delete;
            } else {
                let what = if commits > 0 {
                    format!("{} to ", count(commits, "commit", "commits"))
                } else {
                    "to ".to_owned()
                };
                self.set(say("git", Verb("push", "pushed"), &what, &target, "", t));
                self.action = if forced {
                    "Force push"
                } else {
                    "Push"
                }
                .into();
                self.badge = Glyph::GitBranch;
                self.field = if forced {
                    Field::Delete
                } else {
                    Field::Push
                };
            }
            // The refs as the proxy wrote them; else their names from the line.
            let shown: Vec<String> = if ref_lines.is_empty() {
                refs.iter()
                    .map(|r| match r.strip_prefix("delete ") {
                        Some(n) => format!("{n} · deleted"),
                        None => r.rsplit_once(" (").map_or(*r, |(n, _)| n).to_owned(),
                    })
                    .collect()
            } else {
                ref_lines.iter().map(|r| (*r).to_owned()).collect()
            };
            let mut details = shown.join(", ");
            if !host.is_empty() {
                details = if details.is_empty() {
                    host.clone()
                } else {
                    format!("{details} · {host}")
                };
            }
            self.details = (!details.is_empty()).then_some(details);
            if let Some(changes) = d.get("Changes") {
                self.facts.push(fact("Changes", changes, FactStyle::Changes));
            }
        } else {
            // `read github.com/acme/web`: a clone or a fetch.
            let target = if path.is_empty() {
                e.what.strip_prefix("read ").unwrap_or(&e.what).to_owned()
            } else {
                path
            };
            self.set(say("git", Verb("fetch", "fetched"), "", &target, "", t));
            self.action = "Fetch".into();
            self.badge = Glyph::Eye;
            self.field = Field::Read;
            self.details = (!host.is_empty()).then(|| host.clone());
        }
        self.facts.push(fact("The token", "stays on your phone", FactStyle::Text));
        self.notes.extend(d.rest.iter().cloned());
    }

    fn ssh(&mut self, e: &Entry, d: &Detail, t: Tense) {
        self.who = "ssh".into();
        self.kind = "SSH".into();
        self.agent = Mark::Glyph(Glyph::Laptop);
        self.badge = Glyph::Key;
        self.field = Field::Read;
        let host = e.service.clone().unwrap_or_default();
        self.mark = Brand::of_host(&host).map_or(Mark::Glyph(Glyph::Key), Mark::Brand);
        let asked = e.what.as_str();
        if asked == "the list of SSH keys" {
            self.set(say("ssh", Verb("list", "listed"), "", "your SSH keys", "", t));
            "List".clone_into(&mut self.action);
        } else if let Some(rest) = asked.strip_prefix("sign in ") {
            // `to h as u with k`, `to the server fp as u with k`, `as u with k`.
            let (to, rest) = match rest.strip_prefix("to ") {
                Some(to) => {
                    let (server, after) = to.split_once(" as ").unwrap_or((to, ""));
                    (server.strip_prefix("the server ").unwrap_or(server).to_owned(), after.to_owned())
                }
                None => ("a server".to_owned(), rest.strip_prefix("as ").unwrap_or(rest).to_owned()),
            };
            let (user, key) = rest.split_once(" with ").unwrap_or((rest.as_str(), ""));
            self.set(say("ssh", Verb("sign in", "signed in"), "to ", &to, "", t));
            "Sign in".clone_into(&mut self.action);
            let details: Vec<&str> = [user, key].into_iter().filter(|s| !s.is_empty()).collect();
            self.details = (!details.is_empty()).then(|| format!("as {}", details.join(" · ")));
        } else {
            let key = asked.strip_prefix("sign with ").unwrap_or(asked);
            self.set(say("ssh", Verb("sign with", "signed with"), "", key, "", t));
            "Sign".clone_into(&mut self.action);
        }
        if let Some(k) = d.get("Key") {
            self.facts.push(fact("Key", k, FactStyle::Mono));
        }
        self.facts.push(fact("The key", "stays on your phone", FactStyle::Text));
    }

    fn api(&mut self, e: &Entry, d: &Detail, t: Tense) {
        let name = e.service.clone().or_else(|| e.what.rsplit(' ').next().map(str::to_owned)).unwrap_or_default();
        self.set(say(&self.who.clone(), Verb("use", "used"), "the key for ", &name, "", t));
        self.kind = "API key".into();
        "Use".clone_into(&mut self.action);
        self.mark = if name.contains("anthropic") {
            Mark::Brand(Brand::Anthropic)
        } else if name.contains("openai") {
            Mark::Brand(Brand::OpenAI)
        } else {
            Mark::Glyph(Glyph::Key)
        };
        self.badge = Glyph::Key;
        self.field = Field::Read;
        // `Requests to api.openai.com through the Reins proxy`.
        let host = d
            .rest
            .iter()
            .find_map(|l| l.strip_prefix("Requests to ").and_then(|r| r.split_whitespace().next()))
            .map(str::to_owned);
        let held = d.get("Lease").and_then(|l| l.trim_end_matches(" s").parse::<i64>().ok());
        let parts: Vec<String> =
            host.iter().cloned().chain(held.map(|s| format!("held {}", crate::format::duration(s)))).collect();
        self.details = (!parts.is_empty()).then(|| parts.join(" · "));
        if let Some(s) = d.get("Secret") {
            self.facts.push(fact("Secret", s, FactStyle::Mono));
        }
        self.facts.push(fact("The key", "stays on your phone", FactStyle::Text));
    }

    fn secrets(&mut self, e: &Entry, d: &Detail, t: Tense) {
        let asked = e.what.as_str();
        let quoted = |s: &str| s.split('`').nth(1).map(str::to_owned);
        let program = |cmd: &str| {
            let first = cmd.split_whitespace().find(|p| !p.contains('=')).unwrap_or(cmd);
            last_part(first).unwrap_or_else(|| first.to_owned())
        };
        let who = self.who.clone();
        let mut details = None;
        let (verb, before, target, action) = if let Some(rest) = asked.strip_prefix("the secret names of ") {
            (Verb("list", "listed"), "the secret names of ", rest.to_owned(), "List")
        } else if let Some(rest) = asked.strip_prefix("add a ") {
            (Verb("add", "added"), "", format!("a {rest}"), "Add")
        } else if let Some(rest) = asked.strip_prefix("save ") {
            let what = rest.strip_suffix(" in the vault").unwrap_or(rest);
            (Verb("save", "saved"), "", what.to_owned(), "Save")
        } else if let Some(item) = asked.strip_suffix(", to print") {
            (Verb("print", "printed"), "", item.to_owned(), "Print")
        } else if let Some(cmd) = quoted(asked) {
            details = Some(cmd.clone());
            let item = asked.split_once(" for `").map(|(i, _)| i).filter(|i| *i != "secrets");
            match (asked.starts_with("run `"), item) {
                (true, _) => (Verb("get secrets to run", "got secrets to run"), "", program(&cmd), "Get"),
                (false, Some(name)) => (Verb("get", "got"), "", name.to_owned(), "Get"),
                (false, None) => (Verb("get secrets for", "got secrets for"), "", program(&cmd), "Get"),
            }
        } else {
            (Verb("get", "got"), "", asked.to_owned(), "Get")
        };
        self.set(say(&who, verb, before, &target, "", t));
        "Secrets".clone_into(&mut self.kind);
        action.clone_into(&mut self.action);
        self.details = details.or_else(|| d.get("Command").map(str::to_owned));
        self.mark = Mark::Glyph(Glyph::Lock);
        self.badge = Glyph::Key;
        self.field = Field::Grant;
        for key in ["Command", "Environment", "Variables", "Secrets", "In", "Run by"] {
            if let Some(v) = d.get(key) {
                self.facts.push(fact(key, v, FactStyle::Mono));
            }
        }
    }

    fn mcp(&mut self, e: &Entry, d: &Detail, t: Tense) {
        let tool = e.service.clone().unwrap_or_else(|| e.what.rsplit(": ").next().unwrap_or(&e.what).to_owned());
        let (service, action) = tool.split_once('_').unwrap_or((tool.as_str(), ""));
        let (mark, name): (Mark, Option<&str>) = match service {
            "gmail" => (Mark::Brand(Brand::Gmail), Some("Gmail")),
            "github" => (Mark::Brand(Brand::GitHub), Some("GitHub")),
            "calendar" | "gcalendar" => (Mark::Glyph(Glyph::Calendar), Some("Calendar")),
            "contacts" => (Mark::Glyph(Glyph::Person), Some("Contacts")),
            "telegram" => (Mark::Glyph(Glyph::Send), Some("Telegram")),
            "sms" => (Mark::Glyph(Glyph::Phone), Some("Messages")),
            "vault" => (Mark::Glyph(Glyph::Vault), Some("Vault")),
            "payments" | "pay" => (Mark::Glyph(Glyph::Card), Some("Payments")),
            _ => (Mark::Glyph(Glyph::Chip), None),
        };
        let first = action.split('_').next().unwrap_or("");
        let thing = match service {
            "gmail" => "an email",
            "telegram" | "sms" => "a message",
            _ => "",
        };
        let (verb, object, badge, field): (Verb, String, Glyph, Field) = match first {
            "send" | "reply" => (Verb("send", "sent"), thing.to_owned(), Glyph::Send, Field::Push),
            "read" | "get" | "list" | "view" => (Verb("read", "read"), "from".to_owned(), Glyph::Eye, Field::Read),
            "search" | "find" => (Verb("search", "searched"), String::new(), Glyph::Search, Field::Read),
            "create" | "add" | "new" => {
                let what = action.split_once('_').map_or("something", |(_, w)| w).replace('_', " ");
                (Verb("create", "created"), article(&what), Glyph::Plus, Field::Push)
            }
            "update" | "edit" | "move" | "organize" | "rename" => {
                (Verb("change", "changed"), String::new(), Glyph::Pencil, Field::Push)
            }
            "delete" | "trash" | "remove" => (Verb("delete", "deleted"), String::new(), Glyph::Trash, Field::Delete),
            _ => (Verb("use", "used"), String::new(), Glyph::Play, Field::Push),
        };
        let target = name.map_or_else(|| service.to_owned(), str::to_owned);
        let before = match (object.as_str(), first) {
            ("", _) => String::new(),
            ("from", _) => "from ".to_owned(),
            (o, _) => format!("{o} with "),
        };
        self.set(say(&self.who.clone(), verb, &before, &target, "", t));
        self.kind = name.unwrap_or("MCP tool").into();
        self.action = capitalised(verb.0);
        self.mark = mark;
        self.badge = badge;
        self.field = field;
        let args = d.get("Arguments");
        self.details = Some(match args {
            Some(a) => format!("{tool} · {a}"),
            None => tool.clone(),
        });
        self.facts.push(fact("Tool", tool.clone(), FactStyle::Mono));
        if let Some(a) = args {
            self.facts.push(fact("Arguments", a, FactStyle::Mono));
        }
    }

    fn question(&mut self, e: &Entry, d: &Detail, t: Tense) {
        let q = e.what.trim();
        let lead = match t {
            Tense::Waiting => format!("{} asks ", self.who),
            Tense::Done | Tense::Not => format!("{} asked ", self.who),
        };
        self.sentence = vec![plain(lead), bold(q)];
        self.order = vec![plain("Answer "), bold(q)];
        "Question".clone_into(&mut self.kind);
        "Ask".clone_into(&mut self.action);
        self.mark = Mark::Glyph(Glyph::Help);
        self.badge = Glyph::Phone;
        self.field = Field::Grant;
        self.details = e.service.clone().filter(|s| !s.is_empty() && s != q);
        self.notes.extend(d.rest.iter().cloned());
        self.notes.extend(d.keys.iter().map(|(k, v)| format!("{k}: {v}")));
    }

    fn work_session(&mut self, e: &Entry, d: &Detail, t: Tense) {
        "Work session".clone_into(&mut self.kind);
        "You".clone_into(&mut self.who);
        self.agent = Mark::Glyph(Glyph::Person);
        self.mark = Mark::Glyph(Glyph::Timer);
        self.field = Field::Grant;
        if let Some(reason) = e.what.strip_prefix("ended the work session: ") {
            self.set(say("You", Verb("end", "ended"), "the work session ", reason, "", t));
            "End".clone_into(&mut self.action);
            self.badge = Glyph::Close;
        } else {
            // `work session for 2 hours: Fix the login bug`.
            let rest = e.what.strip_prefix("work session for ").unwrap_or(&e.what);
            let (span, reason) = rest.split_once(": ").unwrap_or(("", rest));
            self.set(say("You", Verb("start", "started"), "a work session for ", reason, "", t));
            "Start".clone_into(&mut self.action);
            self.badge = Glyph::Play;
            let mut parts: Vec<String> = Vec::new();
            if !span.is_empty() {
                parts.push(span.to_owned());
            }
            parts.extend(d.all("Read").map(|r| format!("read {r}")));
            parts.extend(d.all("Push").map(|p| format!("push {p}")));
            self.details = (!parts.is_empty()).then(|| parts.join(" · "));
            for key in ["Read", "Push"] {
                for v in d.all(key) {
                    self.facts.push(fact(key, v, FactStyle::Mono));
                }
            }
        }
    }
}

/// The detail's chip: how it ended and where ("Approved on Dana's iPhone").
#[must_use]
pub fn answer(e: &Entry, phone: &str) -> (Glyph, Tone, String) {
    let on = |word: &str| match e.decider {
        Decider::Phone => format!("{word} on {phone}"),
        Decider::Local => format!("{word} here"),
        Decider::Settings => format!("{word} by your rules"),
    };
    match e.outcome {
        Outcome::Waiting => (Glyph::Phone, Tone::Field(Field::Grant), "Waiting on your phone".to_owned()),
        Outcome::Approved => (Glyph::Phone, Tone::Success, on("Approved")),
        Outcome::Denied => (Glyph::Phone, Tone::Field(Field::Delete), on("Denied")),
        Outcome::TimedOut => (Glyph::TimerOff, Tone::Field(Field::Warn), "No answer".to_owned()),
        Outcome::Failed => (Glyph::CircleAlert, Tone::Field(Field::Delete), "Couldn't ask".to_owned()),
        Outcome::Allowed => (Glyph::Check, Tone::Neutral, "Allowed by your rules".to_owned()),
        Outcome::Blocked => (Glyph::CircleClose, Tone::Field(Field::Delete), "Blocked by your rules".to_owned()),
    }
}

/// The request in one plain line, for a notification and the tray: the sentence, then the details in brackets
/// ("Claude Code wants to run a command in reins (rm -rf jarx · and 7 more commands)").
#[must_use]
pub fn spoken(e: &Entry) -> String {
    let a = Ask::of(e);
    match &a.details {
        Some(d) => format!("{} ({d})", a.line()),
        None => a.line(),
    }
}

/// The field a request waits on.
#[must_use]
pub fn field_of(e: &Entry) -> Field {
    Ask::of(e).field
}

/// Who asks, and where from: "Claude Code · ~/code/acme-web".
#[must_use]
pub fn who_from(e: &Entry) -> String {
    let who = Ask::of(e).who;
    match e.detail.as_deref().and_then(|d| d.lines().find_map(|l| l.trim().strip_prefix("In:"))) {
        Some(dir) if !dir.trim().is_empty() => format!("{who} · {}", dir.trim()),
        _ => who,
    }
}

/// The request as a terminal would have shown it: what matched and the rule's group, what Reins did, and what the
/// agent was told. The command itself is a fact row, not repeated here.
#[must_use]
pub fn transcript(e: &Entry, phone: &str) -> Vec<String> {
    let a = Ask::of(e);
    let mut lines = a.notes.clone();
    let took = e.ended_at.map(|end| crate::format::duration(end - e.at));
    let after = |words: String| match &took {
        Some(t) => format!("reins: {words} ({t})"),
        None => format!("reins: {words}"),
    };
    if e.decider == Decider::Phone {
        lines.push(format!("reins: asking {phone}…"));
    }
    match e.outcome {
        Outcome::Waiting => {}
        Outcome::Approved => lines.push(after(format!("approved on {phone}"))),
        Outcome::Denied => lines.push(after(format!("denied on {phone}"))),
        Outcome::TimedOut => lines.push(after(format!("no answer from {phone}"))),
        Outcome::Failed => lines.push(format!("reins: couldn't ask {phone}")),
        Outcome::Allowed => lines.push("reins: allowed by your rules".to_owned()),
        Outcome::Blocked => lines.push("reins: blocked by your rules".to_owned()),
    }
    if let Some(reason) = e.reason.as_deref().filter(|_| e.outcome != Outcome::Approved) {
        lines.push(format!("{} was told: {reason}", a.who));
    }
    lines
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
        e.outcome = Outcome::Approved;
        e
    }

    fn waiting(mut e: Entry) -> Entry {
        e.outcome = Outcome::Waiting;
        e
    }

    fn denied(mut e: Entry) -> Entry {
        e.outcome = Outcome::Denied;
        e
    }

    /// The sentence with its target in *stars*.
    fn said(w: &[Words]) -> String {
        w.iter()
            .map(|w| {
                if w.bold {
                    format!("*{}*", w.text)
                } else {
                    w.text.clone()
                }
            })
            .collect()
    }

    /// Every sentence has exactly one italic run: the target.
    fn one_target(a: &Ask) {
        assert_eq!(a.sentence.iter().filter(|w| w.bold).count(), 1, "{a:?}");
        assert_eq!(a.order.iter().filter(|w| w.bold).count(), 1, "{a:?}");
    }

    fn push() -> Entry {
        entry(
            Kind::Git,
            Some("git"),
            Some("github.com/katulevskiy/reins"),
            "push to github.com/katulevskiy/reins: signal/desktop-release (3 commits)",
            Some("Ref: signal/desktop-release · new branch\nChanges: +4629 −3747 · 64 files"),
        )
    }

    const COMMAND: &str = "cd /tmp/sigwork && rm -rf jarx && mkdir jarx && cd jarx && unzip -q -o a.jar 'x/*' && javap \
                           -cp . a.B | grep -i \"x\"; javap -cp . c.D | grep -i \"y\\|z\"; javap -cp . e.F | grep -i z";

    fn command() -> Entry {
        let detail = format!(
            "Rule: Recursive deletes · rm -r\nMatched: rm -rf jarx\nGroup: deletes (destructive)\n\nCommand:\n{COMMAND}\n\n\
             In: ~/Documents/dev/reins"
        );
        entry(
            Kind::Command,
            Some("Claude Code"),
            Some("command:rm -r"),
            &format!("Claude Code wants to run: {COMMAND}"),
            Some(&detail),
        )
    }

    #[test]
    fn a_push_counts_its_commits_and_keeps_branches_in_mono() {
        let a = Ask::of(&push());
        assert_eq!(said(&a.sentence), "git pushed 3 commits to *katulevskiy/reins*");
        assert_eq!(a.details.as_deref(), Some("signal/desktop-release · new branch · github.com"));
        assert_eq!(a.label(), "GitHub · Push");
        assert_eq!((a.mark, a.badge, a.field), (Mark::Brand(Brand::GitHub), Glyph::GitBranch, Field::Push));
        assert_eq!(a.facts[0], fact("Changes", "+4629 −3747 · 64 files", FactStyle::Changes));
        let w = Ask::of(&waiting(push()));
        assert_eq!(said(&w.sentence), "git wants to push 3 commits to *katulevskiy/reins*");
        assert_eq!(said(&w.order), "Push 3 commits to *katulevskiy/reins*");
        assert_eq!(said(&Ask::of(&denied(push())).sentence), "git wanted to push 3 commits to *katulevskiy/reins*");
        one_target(&a);
        // Force pushes are red; a push without the proxy's detail names its refs from the line.
        let mut force = push();
        force.detail = Some("Ref: main · force push".to_owned());
        assert_eq!((Ask::of(&force).field, Ask::of(&force).action.as_str()), (Field::Delete, "Force push"));
        let plain_line = entry(
            Kind::Git,
            Some("git"),
            Some("gitlab.com/g/s/p"),
            "push to gitlab.com/g/s/p: main (1 commit), delete old",
            None,
        );
        let a = Ask::of(&plain_line);
        assert_eq!(said(&a.sentence), "git pushed 1 commit to *g/s/p*");
        assert_eq!(a.details.as_deref(), Some("main, old · deleted · gitlab.com"));
        assert_eq!(a.kind, "GitLab");
        let delete = entry(Kind::Git, Some("git"), Some("github.com/o/r"), "push to github.com/o/r: delete old", None);
        assert_eq!(said(&Ask::of(&delete).sentence), "git deleted a branch of *o/r*");
    }

    #[test]
    fn fetches_and_clones_read_the_repository() {
        // git asks the same for a fetch and a clone.
        let fetch = entry(Kind::Git, Some("git"), Some("github.com/acme/web"), "read github.com/acme/web", None);
        let a = Ask::of(&fetch);
        assert_eq!(said(&a.sentence), "git fetched *acme/web*");
        assert_eq!(said(&Ask::of(&waiting(fetch)).order), "Fetch *acme/web*");
        assert_eq!((a.details.as_deref(), a.badge, a.field), (Some("github.com"), Glyph::Eye, Field::Read));
        one_target(&a);
    }

    #[test]
    fn a_command_names_its_folder_and_what_matched() {
        let a = Ask::of(&command());
        assert_eq!(said(&a.sentence), "Claude Code ran a command in *reins*");
        assert_eq!(a.details.as_deref(), Some("rm -rf jarx · and 7 more commands"));
        assert_eq!(said(&Ask::of(&waiting(command())).order), "Run a command in *reins*");
        assert_eq!((a.label().as_str(), a.field, a.badge), ("Command · Run", Field::Delete, Glyph::Trash));
        let keys: Vec<&str> = a.facts.iter().map(|f| f.key.as_str()).collect();
        assert_eq!(keys, ["Rule", "In", "Command"]);
        assert_eq!((a.facts[0].value.as_str(), a.facts[0].tail.as_deref()), ("Recursive deletes", Some("rm -r")));
        assert_eq!(a.facts[2].value, COMMAND, "the command whole, never cut");
        // The transcript says what matched, not the command again.
        let lines = transcript(&command(), "your phone");
        assert_eq!(lines[..2], ["Matched: rm -rf jarx", "Group: deletes (destructive)"]);
        assert!(lines.iter().all(|l| !l.contains("unzip")), "{lines:?}");
        one_target(&a);
    }

    #[test]
    fn commands_split_outside_quotes() {
        assert_eq!(commands_in("a && b || c; d\ne | f"), ["a", "b", "c", "d", "e | f"]);
        assert_eq!(commands_in("echo 'a && b'; echo \"c; d\" && x\\;y"), ["echo 'a && b'", "echo \"c; d\"", "x\\;y"]);
        assert_eq!(commands_in(COMMAND).len(), 8);
        let one = entry(Kind::Command, Some("Codex"), None, "Codex wants to run: make deploy", None);
        let a = Ask::of(&one);
        assert_eq!(said(&a.sentence), "Codex ran a command in *this computer*");
        assert_eq!(a.details.as_deref(), Some("make deploy"));
    }

    #[test]
    fn files_name_the_file_and_keep_the_path_in_mono() {
        let read = entry(
            Kind::File,
            Some("Claude Code"),
            None,
            "Claude Code wants to read /home/me/app/.env and 1 more file(s)",
            Some(
                "Rule: .env files · .env\nMatched: /home/me/app/.env\nGroup: env-files\n\nFiles:\n/home/me/app/.env\n/home/me/app/.env.local\n\nIn: /home/me/app",
            ),
        );
        let a = Ask::of(&read);
        assert_eq!(said(&a.sentence), "Claude Code read *.env*");
        assert_eq!(a.details.as_deref(), Some("/home/me/app/.env · and 1 more file"));
        assert_eq!((a.label().as_str(), a.badge, a.field), ("File · Read", Glyph::Eye, Field::Read));
        assert_eq!(
            a.facts.iter().find(|f| f.key == "Files").unwrap().value,
            "/home/me/app/.env\n/home/me/app/.env.local"
        );
        let change = entry(Kind::File, Some("Codex"), None, "Codex wants to change src/main.rs", None);
        assert_eq!(said(&Ask::of(&waiting(change.clone())).sentence), "Codex wants to change *main.rs*");
        assert_eq!(Ask::of(&change).field, Field::Push);
        one_target(&a);
    }

    #[test]
    fn ssh_signs_in_to_a_host() {
        let e = entry(
            Kind::Ssh,
            Some("ssh"),
            Some("github.com"),
            "sign in to github.com as git with MacBook key",
            Some("Key: MacBook key (SHA256:abc)"),
        );
        let a = Ask::of(&e);
        assert_eq!(said(&a.sentence), "ssh signed in to *github.com*");
        assert_eq!(a.details.as_deref(), Some("as git · MacBook key"));
        assert_eq!((a.mark, a.badge), (Mark::Brand(Brand::GitHub), Glyph::Key));
        let list = entry(Kind::Ssh, Some("ssh"), None, "the list of SSH keys", None);
        assert_eq!(said(&Ask::of(&list).sentence), "ssh listed *your SSH keys*");
        one_target(&a);
    }

    #[test]
    fn api_keys_name_the_api_and_its_host() {
        let e = entry(
            Kind::Api,
            None,
            Some("openai"),
            "the key for API openai",
            Some("Requests to api.openai.com through the Reins proxy\nSecret: vault:OpenAI/api key\nLease: 900 s"),
        );
        let a = Ask::of(&e);
        assert_eq!(said(&a.sentence), "A program used the key for *openai*");
        assert_eq!(
            a.details.as_deref(),
            Some(format!("api.openai.com · held {}", crate::format::duration(900)).as_str())
        );
        assert_eq!((a.mark, a.badge, a.kind.as_str()), (Mark::Brand(Brand::OpenAI), Glyph::Key, "API key"));
        assert_eq!(said(&Ask::of(&waiting(e)).order), "Use the key for *openai*");
        one_target(&a);
    }

    #[test]
    fn secrets_name_what_they_are_for() {
        let run = entry(
            Kind::Secrets,
            Some("reins run"),
            None,
            "secrets for `./deploy.sh --prod`",
            Some("Command: ./deploy.sh --prod\nSecrets: vault:AWS deploy/password"),
        );
        let a = Ask::of(&run);
        assert_eq!(said(&a.sentence), "reins run got secrets for *deploy.sh*");
        assert_eq!(a.details.as_deref(), Some("./deploy.sh --prod"));
        assert_eq!((a.mark, a.badge, a.field), (Mark::Glyph(Glyph::Lock), Glyph::Key, Field::Grant));
        let env = entry(Kind::Secrets, Some("reins run"), None, "acme/production for `npm start`", None);
        assert_eq!(said(&Ask::of(&env).sentence), "reins run got *acme/production*");
        let save = entry(Kind::Secrets, Some("reins secrets"), None, "save acme/dev, MacBook key in the vault", None);
        assert_eq!(said(&Ask::of(&waiting(save)).order), "Save *acme/dev, MacBook key*");
        one_target(&a);
    }

    #[test]
    fn mcp_tools_say_the_action_and_the_service() {
        let send = entry(
            Kind::Mcp,
            Some("Codex"),
            Some("gmail_send"),
            "Codex: gmail_send",
            Some("Arguments: to, subject, body"),
        );
        let a = Ask::of(&send);
        assert_eq!(said(&a.sentence), "Codex sent an email with *Gmail*");
        assert_eq!(a.details.as_deref(), Some("gmail_send · to, subject, body"));
        assert_eq!(
            (a.label().as_str(), a.mark, a.badge, a.field),
            ("Gmail · Send", Mark::Brand(Brand::Gmail), Glyph::Send, Field::Push)
        );
        let read = entry(Kind::Mcp, Some("Codex"), Some("gmail_read"), "Codex: gmail_read", None);
        assert_eq!(said(&Ask::of(&read).sentence), "Codex read from *Gmail*");
        let issue = entry(Kind::Mcp, Some("Codex"), Some("github_create_issue"), "Codex: github_create_issue", None);
        assert_eq!(said(&Ask::of(&waiting(issue)).order), "Create an issue with *GitHub*");
        let other = entry(Kind::Mcp, Some("Codex"), Some("notion_search"), "Codex: notion_search", None);
        assert_eq!(
            (said(&Ask::of(&other).sentence), Ask::of(&other).badge),
            ("Codex searched *notion*".to_owned(), Glyph::Search)
        );
        one_target(&a);
    }

    #[test]
    fn questions_and_work_sessions() {
        let q =
            entry(Kind::Ask, Some("Claude Code"), None, "Deploy the staging branch now?", Some("The tests passed."));
        let a = Ask::of(&q);
        assert_eq!(said(&a.sentence), "Claude Code asked *Deploy the staging branch now?*");
        assert_eq!(said(&Ask::of(&waiting(q)).sentence), "Claude Code asks *Deploy the staging branch now?*");
        assert_eq!((a.mark, a.field), (Mark::Glyph(Glyph::Help), Field::Grant));
        one_target(&a);

        let start = entry(
            Kind::Ask,
            Some("work session"),
            None,
            "work session for 2 hours: Fix the login bug",
            Some("Read: gmail\nPush: github.com acme/web, branch feature/login"),
        );
        let s = Ask::of(&start);
        assert_eq!(said(&s.sentence), "You started a work session for *Fix the login bug*");
        assert_eq!(said(&Ask::of(&waiting(start)).order), "Start a work session for *Fix the login bug*");
        assert_eq!(s.details.as_deref(), Some("2 hours · read gmail · push github.com acme/web, branch feature/login"));
        assert_eq!((s.label().as_str(), s.mark), ("Work session · Start", Mark::Glyph(Glyph::Timer)));
        let end = entry(Kind::Ask, Some("work session"), None, "ended the work session: Fix the login bug", None);
        assert_eq!(said(&Ask::of(&end).sentence), "You ended the work session *Fix the login bug*");
        one_target(&s);
    }

    #[test]
    fn notifications_say_it_in_one_line() {
        assert_eq!(
            spoken(&waiting(command())),
            "Claude Code wants to run a command in reins (rm -rf jarx · and 7 more commands)"
        );
    }

    #[test]
    fn rule_groups_have_a_colour_and_an_icon_each() {
        for g in GROUPS {
            let _ = (group_tone(g.id), group_icon(g.id));
        }
        assert_eq!(group_tone("git-history"), Tone::Field(Field::Delete));
        assert_eq!(group_tone("databases"), Tone::Field(Field::Push));
        assert_eq!(group_tone("env-files"), Tone::Field(Field::Read));
        let icons: std::collections::HashSet<_> = GROUPS.iter().map(|g| group_icon(g.id)).collect();
        assert_eq!(icons.len(), GROUPS.len());
    }

    #[test]
    fn endings_and_answers() {
        let mut e = entry(Kind::Git, None, None, "push", None);
        e.outcome = Outcome::Waiting;
        assert_eq!(answer(&e, "Dana's iPhone").2, "Waiting on your phone");
        e.outcome = Outcome::Approved;
        assert_eq!(answer(&e, "Dana's iPhone").2, "Approved on Dana's iPhone");
        assert_eq!(wait_secs(Kind::Command, 120, 300), 120);
        assert_eq!(wait_secs(Kind::Git, 120, 300), 300);
    }
}
