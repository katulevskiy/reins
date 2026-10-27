//! A repository's code owners (05 §6: "the repo's code owners (from CODEOWNERS)"), read from its **default branch**
//! through the org's read-only GitHub App ([`github_reader`]), never from a branch the agent pushed.
//!
//! - The file is `.github/CODEOWNERS`, then `CODEOWNERS`, then `docs/CODEOWNERS`, as GitHub looks for it, cached for
//!   five minutes per repository.
//! - Syntax as GitHub's: `#` comments (`\#` for a literal one); `*`, `**` and `?`; a leading `/` anchors; a trailing
//!   `/` is a directory; the last matching line wins; a line with no owners clears ownership. `!` negation and `[ ]`
//!   ranges aren't supported by GitHub, so those lines are skipped and reported.
//! - Owners are `@user`, `@org/team` (its members, at most [`MAX_TEAM`]) and email addresses. A GitHub login counts
//!   only once it is a member's **verified** link (`reins_org_people.github_login`).

use std::{
    collections::{BTreeSet, HashMap},
    sync::{LazyLock, Mutex},
    time::Duration,
};

use regex::Regex;
use serde_json::Value;

use super::Resolution;
use crate::{
    api::reins::org::{
        OrgError,
        github_reader::{self, GithubReader},
    },
    db::{
        DbConnInner, DbPool,
        models::{reins_org, reins_org_routing},
    },
};

/// Where GitHub looks for the file, in order.
pub const LOCATIONS: [&str; 3] = [".github/CODEOWNERS", "CODEOWNERS", "docs/CODEOWNERS"];
/// The most members one team contributes.
pub const MAX_TEAM: usize = 500;
/// More changed paths than this and the default owners apply.
pub const MAX_PATHS: usize = 3000;
const CACHE_SECS: i64 = 300;
const TIMEOUT: Duration = Duration::from_secs(10);

/// One owner as a CODEOWNERS line names it.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Owner {
    User(String),
    Team {
        org: String,
        slug: String,
    },
    Email(String),
}

/// A CODEOWNERS pattern, compiled.
#[derive(Clone, Debug)]
pub struct Pattern {
    pub text: String,
    re: Regex,
}

impl Pattern {
    /// Compiles a pattern; `None` for one GitHub doesn't support (`!`, `[`).
    #[must_use]
    pub fn new(raw: &str) -> Option<Self> {
        if raw.is_empty() || raw.starts_with('!') || raw.contains('[') || raw.contains(']') {
            return None;
        }
        let dir_only = raw.ends_with('/') && raw.len() > 1;
        let p = raw.trim_end_matches('/');
        let anchored = p.starts_with('/') || p.contains('/');
        let p = p.trim_start_matches('/');
        let mut re = String::from(if anchored {
            "^"
        } else {
            "^(?:.*/)?"
        });
        let chars: Vec<char> = p.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            match chars[i] {
                '*' if chars.get(i + 1) == Some(&'*') => {
                    if chars.get(i + 2) == Some(&'/') {
                        re.push_str("(?:.*/)?");
                        i += 3;
                    } else {
                        re.push_str(".*");
                        i += 2;
                    }
                    continue;
                }
                '*' => re.push_str("[^/]*"),
                '?' => re.push_str("[^/]"),
                c => re.push_str(&regex::escape(&c.to_string())),
            }
            i += 1;
        }
        // A pattern matches the path itself or anything below it; a directory pattern only what is below.
        re.push_str(if dir_only {
            "/.*$"
        } else {
            "(?:/.*)?$"
        });
        Some(Self {
            text: raw.to_owned(),
            re: Regex::new(&re).ok()?,
        })
    }

    #[must_use]
    pub fn matches(&self, path: &str) -> bool {
        self.re.is_match(path.trim_start_matches('/'))
    }
}

/// A parsed CODEOWNERS file: each line's pattern and owners, in file order.
#[derive(Clone, Debug, Default)]
pub struct Rules(pub Vec<(Pattern, Vec<Owner>)>);

fn name_ok(s: &str) -> bool {
    !s.is_empty() && s.len() <= 100 && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

fn owner(token: &str) -> Option<Owner> {
    if let Some(rest) = token.strip_prefix('@') {
        return match rest.split_once('/') {
            Some((org, slug)) if name_ok(org) && name_ok(slug) => Some(Owner::Team {
                org: org.to_ascii_lowercase(),
                slug: slug.to_ascii_lowercase(),
            }),
            None if name_ok(rest) => Some(Owner::User(rest.to_ascii_lowercase())),
            _ => None,
        };
    }
    (token.contains('@') && !token.contains('/')).then(|| Owner::Email(token.to_ascii_lowercase()))
}

/// Parses a CODEOWNERS file; returns its rules and the (1-based) lines it skipped.
#[must_use]
pub fn parse(text: &str) -> (Rules, Vec<usize>) {
    let mut rules = Vec::new();
    let mut skipped = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix('\\').filter(|l| l.starts_with('#')).unwrap_or(line);
        let mut tokens = line.split_whitespace();
        let Some(raw) = tokens.next() else {
            continue;
        };
        let Some(pattern) = Pattern::new(raw) else {
            skipped.push(n + 1);
            continue;
        };
        let owners: Vec<Owner> = tokens.take_while(|t| !t.starts_with('#')).filter_map(owner).collect();
        rules.push((pattern, owners));
    }
    (Rules(rules), skipped)
}

/// The owners of `path`: the last matching line's (none when it clears ownership or nothing matches).
#[must_use]
pub fn owners_of(rules: &Rules, path: &str) -> Vec<Owner> {
    rules.0.iter().rev().find(|(p, _)| p.matches(path)).map(|(_, o)| o.clone()).unwrap_or_default()
}

/// The default owners: the last `*` line's.
#[must_use]
pub fn default_owners(rules: &Rules) -> Vec<Owner> {
    rules.0.iter().rev().find(|(p, _)| p.text == "*").map(|(_, o)| o.clone()).unwrap_or_default()
}

/// (org, repo) → (fetched at, blob SHA, text).
type FileCache = HashMap<(String, String), (i64, String, String)>;

static FILES: LazyLock<Mutex<FileCache>> = LazyLock::new(Mutex::default);

/// The default branch's CODEOWNERS of `repo`: (blob SHA, text), or `None` when it has none.
pub async fn file(reader: &GithubReader, repo: &str, now: i64) -> Result<Option<(String, String)>, String> {
    let key = (reader.org.clone(), repo.to_owned());
    if let Some((at, sha, text)) = FILES.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(&key)
        && now - at < CACHE_SECS
    {
        return Ok(Some((sha.clone(), text.clone())));
    }
    let info = reader.get_json(&format!("/repos/{repo}")).await?;
    let branch = info["default_branch"].as_str().filter(|b| !b.is_empty()).ok_or("no default branch")?.to_owned();
    for loc in LOCATIONS {
        let branch_q: String = url::form_urlencoded::byte_serialize(branch.as_bytes()).collect();
        let Ok(v) = reader.get_json(&format!("/repos/{repo}/contents/{loc}?ref={branch_q}")).await else {
            continue;
        };
        let (Some(content), Some(sha)) = (v["content"].as_str(), v["sha"].as_str()) else {
            continue;
        };
        let raw: String = content.chars().filter(|c| !c.is_whitespace()).collect();
        let bytes = data_encoding::BASE64.decode(raw.as_bytes()).map_err(|_| "CODEOWNERS isn't base64")?;
        let text = String::from_utf8(bytes).map_err(|_| "CODEOWNERS isn't text")?;
        FILES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(key, (now, sha.to_owned(), text.clone()));
        return Ok(Some((sha.to_owned(), text)));
    }
    Ok(None)
}

/// A team's members' logins (at most [`MAX_TEAM`]).
async fn team_logins(reader: &GithubReader, org: &str, slug: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for page in 1..=5 {
        let v = reader.get_json(&format!("/orgs/{org}/teams/{slug}/members?per_page=100&page={page}")).await?;
        let list = v.as_array().cloned().unwrap_or_default();
        out.extend(list.iter().filter_map(|m| m["login"].as_str()).map(str::to_ascii_lowercase));
        if list.len() < 100 || out.len() >= MAX_TEAM {
            break;
        }
    }
    out.truncate(MAX_TEAM);
    Ok(out)
}

/// What GitHub said: the owners' logins (teams expanded) and emails, before they're matched to members.
#[derive(Clone, Debug, Default)]
pub struct Fetched {
    pub logins: BTreeSet<String>,
    pub emails: BTreeSet<String>,
    pub unresolved: Vec<String>,
    pub evidence: String,
}

/// Reads the owners of `paths` in `repo` (the default `*` owners when the paths aren't known or are too many).
pub async fn fetch(reader: &GithubReader, repo: &str, paths: Option<&[String]>, now: i64) -> Result<Fetched, String> {
    let file = match tokio::time::timeout(TIMEOUT, file(reader, repo, now)).await {
        Ok(Ok(Some(f))) => f,
        Ok(Ok(None)) => {
            return Ok(Fetched {
                unresolved: vec!["no CODEOWNERS".to_owned()],
                ..Fetched::default()
            });
        }
        Ok(Err(e)) => return Err(e),
        Err(_) => return Err("timed out".to_owned()),
    };
    let (rules, _) = parse(&file.1);
    let mut out = Fetched {
        evidence: reins_proto::canon::hex32(&reins_proto::canon::sha256(format!("{}\n{}", file.0, file.1).as_bytes())),
        ..Fetched::default()
    };
    let owners: BTreeSet<Owner> = if let Some(paths) = paths.filter(|p| p.len() <= MAX_PATHS) {
        paths.iter().flat_map(|p| owners_of(&rules, p)).collect()
    } else {
        out.unresolved.push("paths unknown".to_owned());
        default_owners(&rules).into_iter().collect()
    };
    for o in owners {
        match o {
            Owner::User(l) => {
                out.logins.insert(l);
            }
            Owner::Email(e) => {
                out.emails.insert(e);
            }
            Owner::Team {
                org,
                slug,
            } => match team_logins(reader, &org, &slug).await {
                Ok(l) => out.logins.extend(l),
                Err(_) => out.unresolved.push(format!("team @{org}/{slug} unavailable")),
            },
        }
    }
    Ok(out)
}

/// The active members of `org` among `fetched`'s verified logins and emails.
pub fn map_members(c: &mut DbConnInner, org: &str, fetched: &Fetched) -> Result<Resolution, OrgError> {
    use crate::db::schema::users;
    use diesel::prelude::*;
    let mut members = BTreeSet::new();
    for p in reins_org_routing::q_people(c, org)? {
        let verified = p.github_verified_via.is_some();
        if verified && p.github_login.as_deref().is_some_and(|l| fetched.logins.contains(&l.to_ascii_lowercase())) {
            members.insert(p.user_uuid);
        }
    }
    for e in &fetched.emails {
        if let Some(u) = users::table.filter(users::email.eq(e)).select(users::uuid).first::<String>(c).optional()? {
            members.insert(u);
        }
    }
    let mut active = BTreeSet::new();
    for u in members {
        if reins_org::q_member_state(c, org, &u)?.is_some_and(|m| m.is_active()) {
            active.insert(u);
        }
    }
    let mut unresolved = fetched.unresolved.clone();
    if active.is_empty() {
        unresolved.push("no linked code owners".to_owned());
    }
    Ok(Resolution {
        members: active,
        unresolved,
        source: "codeowners".to_owned(),
        evidence: fetched.evidence.clone(),
    })
}

async fn reader_of(pool: &DbPool, org: &str) -> Option<GithubReader> {
    let conn = pool.get().await.ok()?;
    let o = org.to_owned();
    conn.run(move |c| github_reader::for_org(c, &o)).await.ok().flatten()
}

/// The code owners of `paths` in `repo` through the org's reader.
pub async fn owners(pool: &DbPool, org: &str, repo: &str, paths: Option<&[String]>) -> Resolution {
    let Some(reader) = reader_of(pool, org).await else {
        return Resolution::nobody("codeowners", "no GitHub reader");
    };
    let fetched = match fetch(&reader, repo, paths, crate::api::reins::now_unix()).await {
        Ok(f) => f,
        Err(e) => {
            info!("Org {org}: code owners of {repo}: {e}");
            return Resolution::nobody("codeowners", "code owners unavailable");
        }
    };
    let Ok(conn) = pool.get().await else {
        return Resolution::nobody("codeowners", "code owners unavailable");
    };
    let o = org.to_owned();
    conn.run(move |c| map_members(c, &o, &fetched))
        .await
        .unwrap_or_else(|_| Resolution::nobody("codeowners", "code owners unavailable"))
}

/// The files a pull request changes (at most [`MAX_PATHS`] + 1, so "too many" shows), through the org's reader.
pub async fn pr_files(pool: &DbPool, org: &str, repo: &str, pr: u64) -> Option<Vec<String>> {
    let reader = reader_of(pool, org).await?;
    pr_files_with(&reader, repo, pr).await
}

pub async fn pr_files_with(reader: &GithubReader, repo: &str, pr: u64) -> Option<Vec<String>> {
    let mut out = Vec::new();
    for page in 1..=31 {
        let v: Value =
            reader.get_json(&format!("/repos/{repo}/pulls/{pr}/files?per_page=100&page={page}")).await.ok()?;
        let list = v.as_array()?.clone();
        out.extend(list.iter().filter_map(|f| f["filename"].as_str()).map(str::to_owned));
        if list.len() < 100 || out.len() > MAX_PATHS {
            break;
        }
    }
    Some(out)
}

/// The GitHub organization's SAML identities: (`nameId` lowercased, login).
pub async fn saml_identities(reader: &GithubReader, github_org: &str) -> Result<Vec<(String, String)>, String> {
    let query = "query($org: String!, $after: String) { organization(login: $org) { samlIdentityProvider { \
                 externalIdentities(first: 100, after: $after) { pageInfo { hasNextPage endCursor } \
                 nodes { samlIdentity { nameId } user { login } } } } } }";
    let mut pairs = Vec::new();
    let mut after = Value::Null;
    for _ in 0..50 {
        let v = reader.graphql(query, serde_json::json!({"org": github_org, "after": after})).await?;
        let ids = &v["data"]["organization"]["samlIdentityProvider"]["externalIdentities"];
        for n in ids["nodes"].as_array().into_iter().flatten() {
            if let (Some(email), Some(login)) = (n["samlIdentity"]["nameId"].as_str(), n["user"]["login"].as_str()) {
                pairs.push((email.to_ascii_lowercase(), login.to_owned()));
            }
        }
        if ids["pageInfo"]["hasNextPage"] != Value::Bool(true) {
            break;
        }
        after = ids["pageInfo"]["endCursor"].clone();
    }
    Ok(pairs)
}

/// The daily re-check: for each org with a reader and a GitHub organization, links members by SAML identity.
pub async fn relink_all(pool: &DbPool, now: i64) {
    let Ok(conn) = pool.get().await else {
        return;
    };
    let orgs = conn
        .run(|c| -> Result<Vec<(String, String)>, OrgError> {
            use crate::db::schema::reins_org_github_reader as readers;
            use diesel::prelude::*;
            let orgs: Vec<String> = readers::table.select(readers::org_uuid).load(c)?;
            let mut out = Vec::new();
            for org in orgs {
                if let Some(gh) = reins_org::q_org(c, &org)?.and_then(|t| t.github_org) {
                    out.push((org, gh));
                }
            }
            Ok(out)
        })
        .await
        .unwrap_or_default();
    drop(conn);
    for (org, gh) in orgs {
        let Some(reader) = reader_of(pool, &org).await else {
            continue;
        };
        match saml_identities(&reader, &gh).await {
            Ok(pairs) => {
                if let Ok(conn) = pool.get().await {
                    let o = org.clone();
                    if let Err(e) = conn.run(move |c| apply_saml_links(c, &o, &pairs, now)).await {
                        warn!("Org {org}: GitHub links: {e:?}");
                    }
                }
            }
            // Unknown isn't "gone": links stay until GitHub answers.
            Err(e) => info!("Org {org}: GitHub SAML identities: {e}"),
        }
    }
}

/// Stores SAML-proven links (`nameId` equals a member's email) for active members, and drops SAML links that no
/// longer hold (the daily re-check).
pub fn apply_saml_links(
    c: &mut DbConnInner,
    org: &str,
    pairs: &[(String, String)],
    now: i64,
) -> Result<usize, OrgError> {
    use crate::db::schema::users;
    use diesel::prelude::*;
    let mut linked = BTreeSet::new();
    for (email, login) in pairs {
        let user: Option<String> =
            users::table.filter(users::email.eq(email)).select(users::uuid).first(c).optional()?;
        let Some(user) = user else {
            continue;
        };
        if !reins_org::q_member_state(c, org, &user)?.is_some_and(|m| m.is_active()) || !name_ok(login) {
            continue;
        }
        reins_org_routing::q_edit_person(c, org, &user, now, |p| {
            p.github_login = Some(login.clone());
            p.github_verified_via = Some(reins_org_routing::github_via::SAML.to_owned());
        })?;
        linked.insert(user);
    }
    for p in reins_org_routing::q_people(c, org)? {
        if p.github_verified_via.as_deref() == Some(reins_org_routing::github_via::SAML)
            && !linked.contains(&p.user_uuid)
        {
            reins_org_routing::q_edit_person(c, org, &p.user_uuid, now, |row| {
                row.github_login = None;
                row.github_verified_via = None;
            })?;
        }
    }
    Ok(linked.len())
}

#[cfg(all(test, sqlite))]
mod tests {
    use serde_json::json;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::api::reins::org::testing::{add_org, add_user, db, member};

    fn pem() -> String {
        String::from_utf8(openssl::rsa::Rsa::generate(2048).unwrap().private_key_to_pem().unwrap()).unwrap()
    }

    fn files(text: &str) -> Vec<String> {
        text.split_whitespace().map(str::to_owned).collect()
    }

    fn team(org: &str, slug: &str) -> Owner {
        Owner::Team {
            org: org.into(),
            slug: slug.into(),
        }
    }

    #[test]
    fn codeowners_last_match_wins() {
        let (rules, skipped) = parse(
            "# Owners\n*       @acme/eng\n*.rs    @rustacean\n/docs/  docs@acme.com\nsrc/gen/\napps/**/ui @ui-lead  # UI\n",
        );
        assert!(skipped.is_empty());
        assert_eq!(owners_of(&rules, "README.md"), [team("acme", "eng")]);
        assert_eq!(owners_of(&rules, "src/main.rs"), [Owner::User("rustacean".into())]);
        assert_eq!(owners_of(&rules, "docs/guide.md"), [Owner::Email("docs@acme.com".into())]);
        // A later line with no owners clears ownership.
        assert!(owners_of(&rules, "src/gen/x.rs").is_empty());
        assert_eq!(owners_of(&rules, "apps/web/ui/button.ts"), [Owner::User("ui-lead".into())]);
        assert_eq!(default_owners(&rules), [team("acme", "eng")]);
    }

    #[test]
    fn globs_follow_github_rules() {
        let m = |p: &str, path: &str| Pattern::new(p).unwrap().matches(path);
        // Anchored vs anywhere.
        assert!(m("/docs/", "docs/a.md") && !m("/docs/", "src/docs/a.md"));
        assert!(m("docs/", "docs/a.md") && m("docs/", "src/docs/a.md"));
        assert!(!m("docs/", "docs"), "a directory pattern needs something below it");
        // `*` stays in a component; `**` crosses.
        assert!(m("*.rs", "a.rs") && m("*.rs", "deep/x/a.rs") && !m("*.rs", "a.rsx"));
        assert!(m("apps/**/ui", "apps/ui") && m("apps/**/ui", "apps/a/b/ui/x.ts") && !m("apps/**/ui", "apps/uix"));
        assert!(m("src/*", "src/a") && m("src/*", "src/a/b"), "a matched directory covers what's under it");
        assert!(!m("/src/*.md", "src/a/b.md"));
        // `?` is one character, never a slash.
        assert!(m("v?.txt", "v1.txt") && !m("v?.txt", "v10.txt") && !m("v?.txt", "v/.txt"));
    }

    #[test]
    fn unsupported_lines_are_skipped_and_reported() {
        let (rules, skipped) = parse("*.js @a\n!vendor/ @b\n[Mm]akefile @c\n\\#hash @d\n");
        assert_eq!(skipped, [2, 3]);
        assert_eq!(rules.0.len(), 2);
        assert_eq!(owners_of(&rules, "#hash"), [Owner::User("d".into())]);
    }

    async fn github(text: &str, installation: i64) -> MockServer {
        rustls::crypto::ring::default_provider().install_default().ok();
        let gh = MockServer::start().await;
        let token = json!({"token": "ghs_r", "expires_at": "2099-01-01T00:00:00Z"});
        Mock::given(method("POST"))
            .and(path(format!("/app/installations/{installation}/access_tokens")))
            .respond_with(ResponseTemplate::new(201).set_body_json(token))
            .mount(&gh)
            .await;
        Mock::given(method("GET"))
            .and(path("/repos/acme/api"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"default_branch": "main"})))
            .mount(&gh)
            .await;
        let content = json!({"content": data_encoding::BASE64.encode(text.as_bytes()), "sha": "abc"});
        Mock::given(method("GET"))
            .and(path("/repos/acme/api/contents/.github/CODEOWNERS"))
            .and(query_param("ref", "main"))
            .respond_with(ResponseTemplate::new(200).set_body_json(content))
            .mount(&gh)
            .await;
        // Any other ref (a branch the agent pushed) is refused.
        Mock::given(method("GET"))
            .and(path("/repos/acme/api/contents/.github/CODEOWNERS"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&gh)
            .await;
        Mock::given(method("GET"))
            .and(path("/orgs/acme/teams/eng/members"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([{"login": "Kim-GH"}, {"login": "stranger"}])))
            .mount(&gh)
            .await;
        gh
    }

    fn link(c: &mut DbConnInner, user: &str, login: &str, verified: bool) {
        reins_org_routing::q_edit_person(c, "o1", user, 1, |p| {
            p.github_login = Some(login.into());
            p.github_verified_via = verified.then(|| reins_org_routing::github_via::SAML.to_owned());
        })
        .unwrap();
    }

    fn setup() -> DbConnInner {
        let mut c = db();
        add_org(&mut c, "o1");
        for u in ["kim", "lee", "ann"] {
            member(&mut c, "o1", u, &[]);
        }
        link(&mut c, "kim", "kim-gh", true);
        link(&mut c, "ann", "ann-gh", false);
        c
    }

    #[tokio::test]
    async fn teams_expand_to_linked_members_only() {
        let mut c = setup();
        let gh = github("* @acme/eng @ann-gh\n", 9001).await;
        let reader = GithubReader::for_tests("o9001", 1, 9001, &pem(), &gh.uri());
        let fetched = fetch(&reader, "acme/api", Some(&files("a.txt")), 1000).await.unwrap();
        assert!(fetched.logins.contains("stranger") && fetched.logins.contains("ann-gh"));
        let r = map_members(&mut c, "o1", &fetched).unwrap();
        // Kim's verified login is on the team; the stranger isn't a member; Ann's link isn't verified.
        assert_eq!(r.members, ["kim".to_owned()].into(), "{r:?}");
        assert_eq!((r.source.as_str(), r.evidence.len()), ("codeowners", 64));
    }

    #[tokio::test]
    async fn owners_come_from_the_default_branch_only() {
        let mut c = setup();
        let gh = github("* @kim-gh\n", 9002).await;
        let reader = GithubReader::for_tests("o9002", 1, 9002, &pem(), &gh.uri());
        let fetched = fetch(&reader, "acme/api", None, 2000).await.unwrap();
        assert_eq!(map_members(&mut c, "o1", &fetched).unwrap().members, ["kim".to_owned()].into());
        let refs: Vec<String> = gh
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|q| q.url.path().ends_with("CODEOWNERS"))
            .map(|q| q.url.query().unwrap_or_default().to_owned())
            .collect();
        assert_eq!(refs, ["ref=main"]);
    }

    #[tokio::test]
    async fn unknown_paths_use_the_default_owners_and_say_so() {
        let mut c = setup();
        let gh = github("* @kim-gh\n/secret/ @ann-gh\n", 9003).await;
        let reader = GithubReader::for_tests("o9003", 1, 9003, &pem(), &gh.uri());
        let fetched = fetch(&reader, "acme/api", None, 3000).await.unwrap();
        let r = map_members(&mut c, "o1", &fetched).unwrap();
        assert_eq!(r.members, ["kim".to_owned()].into());
        assert!(r.unresolved.contains(&"paths unknown".to_owned()), "{r:?}");
        let too_many: Vec<String> = (0..=MAX_PATHS).map(|i| format!("f{i}")).collect();
        let fetched = fetch(&reader, "acme/api", Some(&too_many), 3001).await.unwrap();
        assert!(fetched.unresolved.contains(&"paths unknown".to_owned()));
        // Known paths take their own owners.
        let fetched = fetch(&reader, "acme/api", Some(&files("secret/k.pem")), 3002).await.unwrap();
        assert_eq!(fetched.logins, ["ann-gh".to_owned()].into());
    }

    #[tokio::test]
    async fn the_reader_token_is_cached_and_refreshed() {
        let gh = github("* @kim-gh\n", 9004).await;
        let reader = GithubReader::for_tests("o9004", 1, 9004, &pem(), &gh.uri());
        fetch(&reader, "acme/api", None, 4000).await.unwrap();
        reader.get_json("/repos/acme/api").await.unwrap();
        let requests = gh.received_requests().await.unwrap();
        assert_eq!(requests.iter().filter(|q| q.url.path().ends_with("/access_tokens")).count(), 1, "one token");
        // Every API call carries the installation token, never the App's key; the token request carries a JWT.
        let auth =
            |q: &wiremock::Request| q.headers.get("authorization").and_then(|h| h.to_str().ok()).map(str::to_owned);
        assert!(
            requests
                .iter()
                .filter(|q| q.url.path().starts_with("/repos"))
                .all(|q| auth(q).as_deref() == Some("Bearer ghs_r"))
        );
        let jwt = requests.iter().find(|q| q.url.path().ends_with("/access_tokens")).and_then(auth).unwrap();
        assert!(jwt.starts_with("Bearer ey") && jwt.matches('.').count() == 2, "{jwt}");
    }

    #[test]
    fn saml_links_bind_logins_to_members_by_email() {
        let mut c = setup();
        add_user(&mut c, "kim", "kim@member.example");
        apply_saml_links(&mut c, "o1", &[("kim@member.example".into(), "Kim-GH".into())], 5).unwrap();
        let kim = reins_org_routing::q_person(&mut c, "o1", "kim").unwrap().unwrap();
        assert_eq!(kim.github_login.as_deref(), Some("Kim-GH"));
        // The daily re-check drops a SAML link GitHub no longer lists.
        apply_saml_links(&mut c, "o1", &[], 6).unwrap();
        let kim = reins_org_routing::q_person(&mut c, "o1", "kim").unwrap().unwrap();
        assert_eq!((kim.github_login, kim.github_verified_via), (None, None));
    }
}
