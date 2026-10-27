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
];

/// Names of the owner's own notes chat in the messaging connectors.
const SELF_CHATS: &[&str] = &["me", "self", "saved messages"];

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
    pub chats: BTreeMap<String, Zone>,
}

/// The lookup form of a key: trimmed and lowercased, `None` when empty or not ASCII (a lookalike can't match).
#[must_use]
pub fn key(raw: &str) -> Option<String> {
    let k = raw.trim().to_ascii_lowercase();
    (!k.is_empty() && k.is_ascii() && !k.chars().any(char::is_control)).then_some(k)
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

    /// Forgets a fact.
    pub fn remove(&mut self, kind: FactKind, raw: &str) {
        if let Some(k) = key(raw) {
            self.map_mut(kind).remove(&k);
        }
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
    /// The single repository (`owner/name`), else the recipients (sorted, joined by `,`), else the host.
    pub resource: Option<String>,
}

/// A GitHub-style login or repository name part.
fn login(s: &str) -> bool {
    !s.is_empty() && s.len() <= 100 && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// `(owner, name)` of `https://host/o/n(.git)`, `git@host:o/n`, `github.com/o/n` or `o/n`.
#[must_use]
pub fn split_repo(repo: &str) -> Option<(String, String)> {
    let r = repo.trim();
    let path = match r.split_once("://") {
        Some((_, rest)) => rest.split_once('/').map_or("", |(_, path)| path),
        None => match r.split_once(':') {
            Some((host, path)) if host.contains('@') => path,
            Some(_) => return None,
            None => {
                let mut parts = r.splitn(3, '/');
                match (parts.next(), parts.next(), parts.next()) {
                    (Some(h), Some(_), Some(_)) if h.contains('.') => r.split_once('/').map_or("", |(_, p)| p),
                    _ => r,
                }
            }
        },
    };
    let path = path.trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let (o, n) = path.split_once('/')?;
    (login(o) && login(n) && !n.contains('/')).then(|| (o.to_ascii_lowercase(), n.to_ascii_lowercase()))
}

/// The zone of a repository: the repository's own fact, else its owner's; unknown owners are Outside.
#[must_use]
pub fn repo_zone(facts: &Facts, repo: &str) -> (Zone, Option<Unknown>) {
    let Some((o, n)) = split_repo(repo) else {
        return (Zone::Outside, None);
    };
    if let Some(z) = facts.repos.get(&format!("{o}/{n}")) {
        return (*z, None);
    }
    match facts.owners.get(&o) {
        Some(z) => (*z, None),
        None => (
            Zone::Outside,
            Some(Unknown {
                kind: FactKind::Owner,
                key: o,
            }),
        ),
    }
}

/// An email address's domain, when it looks like one address.
fn address_domain(addr: &str) -> Option<&str> {
    let (local, domain) = addr.rsplit_once('@')?;
    (!local.is_empty() && !local.contains(char::is_whitespace) && domain.contains('.') && login(domain))
        .then_some(domain)
}

/// `Name <a@b.c>` → `a@b.c`.
fn bare_address(party: &str) -> &str {
    match (party.rfind('<'), party.rfind('>')) {
        (Some(a), Some(b)) if a < b => &party[a + 1..b],
        _ => party,
    }
}

/// Whether a chat party looks like one handle, number or id (so it can be asked about).
fn handle(s: &str) -> bool {
    !s.is_empty() && s.len() <= 100 && !s.contains(char::is_whitespace)
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
        Channel::Email | Channel::Calendar => {
            let Some(addr) = key(bare_address(party)) else {
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
        Channel::Telegram | Channel::Sms => {
            let Some(k) = key(party) else {
                return (Zone::Outside, None);
            };
            if SELF_CHATS.contains(&k.as_str()) {
                return (Zone::Mine, None);
            }
            if let Some(z) = facts.chats.get(&k).or_else(|| facts.people.get(&k)) {
                return (*z, None);
            }
            unknown(FactKind::Person, handle(&k).then_some(k))
        }
        Channel::GitHub => repo_zone(facts, party),
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
            login(&k).then(|| Unknown {
                kind: FactKind::Host,
                key: k,
            }),
        ),
    }
}

/// A request's zone: the least trusted zone of everything it changes or reaches (01 §3). Reads add no zone (01 §3 would
/// make every read outside the workspace Outside, so `grep Host ~/.ssh/config` would ask at L0; a read's risk is its
/// level and the always-ask floor). With nothing that sets a zone, it is `base_repo`'s (the repository the request acts
/// on), else Mine.
#[must_use]
pub fn request_zone(a: &Analysis, facts: &Facts, base_repo: Option<&str>) -> Resolved {
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
    let base = || base_repo.map_or((Zone::Outside, None), |b| repo_zone(facts, b));
    let area = |area: &Area| match area {
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
                let (z, u) = if split_repo(repo).is_some() {
                    repos.insert(repo.clone());
                    repo_zone(facts, repo)
                } else {
                    if let Some(b) = base_repo {
                        repos.insert(b.to_owned());
                    }
                    base()
                };
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
                    let (z, u) = party_zone(facts, channel, p);
                    if *channel == Channel::GitHub {
                        repos.insert(p.clone());
                    } else {
                        parties.insert(p.trim().to_ascii_lowercase());
                    }
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
                let (z, u) = repo_zone(facts, resource);
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
    let zone = zone.unwrap_or_else(|| match base_repo {
        Some(_) => {
            let (z, u) = base();
            if let Some(u) = u
                && !unknown.contains(&u)
            {
                unknown.push(u);
            }
            z
        }
        None => Zone::Mine,
    });
    let resource = if repos.len() == 1 {
        repos.into_iter().next().and_then(|r| split_repo(&r)).map(|(o, n)| format!("{o}/{n}"))
    } else if !parties.is_empty() && repos.is_empty() {
        Some(parties.into_iter().collect::<Vec<_>>().join(","))
    } else if hosts.len() == 1 && repos.is_empty() {
        hosts.into_iter().next()
    } else {
        None
    };
    Resolved {
        zone,
        unknown,
        resource,
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
        assert_eq!(request_zone(&push("github.com/dkat/reins"), &f, None).zone, Zone::Mine);
        assert_eq!(request_zone(&push("https://github.com/acme/api.git"), &f, None).zone, Zone::Work);
        assert_eq!(request_zone(&push("git@github.com:octo/x.git"), &f, None).zone, Zone::Friends);
        assert_eq!(request_zone(&analyze_command("rm -rf target", &ctx()), &f, None).zone, Zone::Mine);
        assert_eq!(request_zone(&analyze_command("rm -rf ~/x", &ctx()), &f, None).zone, Zone::Outside);
        let r = request_zone(&push("github.com/dkat/reins"), &f, None);
        assert_eq!(r.resource.as_deref(), Some("dkat/reins"));
    }

    #[test]
    fn unknown_is_outside_and_named() {
        let r = request_zone(&push("octo/y"), &facts(), None);
        assert_eq!(r.zone, Zone::Outside);
        assert_eq!(
            r.unknown,
            vec![Unknown {
                kind: FactKind::Owner,
                key: "octo".to_owned()
            }]
        );
        // a placeholder, not a party, is Outside and not asked about
        let r = request_zone(&analyze_tool("gmail_set_vacation", &json!({})), &facts(), None);
        assert_eq!(r.zone, Zone::Outside);
        assert!(r.unknown.is_empty(), "{:?}", r.unknown);
    }

    #[test]
    fn lookalikes_are_outside() {
        let f = facts();
        assert_eq!(request_zone(&push("dkat-evil/reins"), &f, None).zone, Zone::Outside);
        assert_eq!(request_zone(&push("DKAT/reins"), &f, None).zone, Zone::Mine, "case only");
        assert_eq!(request_zone(&push("dk\u{0430}t/reins"), &f, None).zone, Zone::Outside, "Cyrillic a");
        assert_eq!(request_zone(&mail(&["me@gmail.com.evil.io"]), &f, None).zone, Zone::Outside);
        assert_eq!(request_zone(&mail(&["me@gmail.com"]), &f, None).zone, Zone::Mine);
        assert_eq!(request_zone(&mail(&["x@sub.acme.io"]), &f, None).zone, Zone::Outside, "not a subdomain match");
        assert_eq!(host_zone(&f, "localhost.evil.io").0, Zone::Outside);
        assert_eq!(host_zone(&f, "localhost").0, Zone::Mine);
    }

    #[test]
    fn one_outside_party_makes_it_outside() {
        let f = facts();
        let r = request_zone(&mail(&["me@gmail.com", "x@stranger.io"]), &f, None);
        assert_eq!(r.zone, Zone::Outside);
        assert_eq!(r.unknown.len(), 1);
        assert_eq!(request_zone(&mail(&["me@gmail.com", "ann@example.com"]), &f, None).zone, Zone::Friends);
        let two = Analysis::new(
            [push("dkat/a").effects, push("octo/z").effects].concat(),
            effects::Certainty::Full,
            Vec::new(),
        );
        assert_eq!(request_zone(&two, &f, None).zone, Zone::Outside);
        assert_eq!(request_zone(&mail(&[]), &f, None).zone, Zone::Outside, "nobody named is not nobody");
    }

    #[test]
    fn public_mail_domains_are_never_zoned() {
        let mut f = Facts::default();
        assert!(!f.set(FactKind::Domain, "gmail.com", Zone::Work));
        assert!(!f.set(FactKind::Domain, "", Zone::Work));
        assert!(!f.set(FactKind::Owner, "dk\u{0430}t", Zone::Mine));
        assert!(f.domains.is_empty() && f.owners.is_empty());
    }

    #[test]
    fn reads_add_no_zone() {
        assert_eq!(request_zone(&analyze_command("cat ~/.bashrc", &ctx()), &facts(), None).zone, Zone::Mine);
        assert_eq!(request_zone(&analyze_tool("gmail_search", &json!({"q": "x"})), &facts(), None).zone, Zone::Mine);
    }

    #[test]
    fn work_domain_recipients_are_work() {
        assert_eq!(request_zone(&mail(&["Bob <bob@acme.io>"]), &facts(), None).zone, Zone::Work);
    }

    #[test]
    fn origin_is_the_base_repository() {
        let a = analyze_command("git push origin feat/x", &ctx());
        let f = facts();
        assert_eq!(request_zone(&a, &f, Some("github.com/dkat/app")).zone, Zone::Mine);
        assert_eq!(request_zone(&a, &f, None).zone, Zone::Outside, "no repository known: Outside");
    }
}
