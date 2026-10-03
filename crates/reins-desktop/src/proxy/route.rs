//! Which proxy request a path is: `/<host>/<repo path>[.git]/<smart-HTTP or LFS path>`, the repository path being
//! `owner/name`, on GitLab `group/subgroup/.../name`. Everything else is refused, and names are checked strictly because
//! they end up in the upstream URL.

use crate::auth::Repo;
use crate::config::GitHost;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Service {
    UploadPack,
    ReceivePack,
}

impl Service {
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::UploadPack => "git-upload-pack",
            Self::ReceivePack => "git-receive-pack",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `GET info/refs?service=…`
    InfoRefs(Service),
    /// `POST git-upload-pack` / `POST git-receive-pack`
    Rpc(Service),
    /// `info/lfs/<rest>` with the query string, if any.
    Lfs {
        rest: String,
        query: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Route {
    pub repo: Repo,
    pub kind: Kind,
}

impl Route {
    /// The path below the repository (`/info/refs?service=git-upload-pack`, `/git-receive-pack`, `/info/lfs/...`).
    #[must_use]
    pub fn suffix(&self) -> String {
        match &self.kind {
            Kind::InfoRefs(s) => format!("/info/refs?service={}", s.name()),
            Kind::Rpc(s) => format!("/{}", s.name()),
            Kind::Lfs {
                rest,
                query,
            } => match query {
                Some(q) => format!("/info/lfs/{rest}?{q}"),
                None => format!("/info/lfs/{rest}"),
            },
        }
    }
}

fn valid_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 100
        && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        && !s.contains("..")
        && s != "."
        // GitLab's separator between a project and its pages (`group/project/-/tree`).
        && s != "-"
        // Only the last part may name the repository's `.git`, and only once.
        && !s.to_ascii_lowercase().ends_with(".git")
}

/// How many parts a repository path has on `service`: GitLab nests groups (up to 20 parts), the others are
/// `owner/name`.
fn depth(service: &str) -> Option<std::ops::RangeInclusive<usize>> {
    match service {
        "gitlab" => Some(2..=20),
        "github" | "codeberg" | "bitbucket" => Some(2..=2),
        _ => None,
    }
}

/// The longest repository path the phone accepts.
const MAX_REPO_PATH: usize = 250;

/// Checks a repository path on `service` (`group/sub/name`, the last part optionally with `.git`) and returns it
/// without `.git`; `None` when it is not one.
#[must_use]
pub fn repo_path(service: &str, raw: &str) -> Option<String> {
    let parts: Vec<&str> = raw.split('/').collect();
    repo_parts(service, &parts)
}

fn repo_parts(service: &str, parts: &[&str]) -> Option<String> {
    if !depth(service)?.contains(&parts.len()) {
        return None;
    }
    let (last, groups) = parts.split_last()?;
    let last = last.strip_suffix(".git").unwrap_or(last);
    if !groups.iter().all(|p| valid_name(p)) || !valid_name(last) {
        return None;
    }
    let mut path = groups.join("/");
    if !path.is_empty() {
        path.push('/');
    }
    path.push_str(last);
    (path.len() <= MAX_REPO_PATH).then_some(path)
}

fn valid_lfs(rest: &str) -> bool {
    !rest.is_empty()
        && rest.len() <= 512
        && rest.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b'/'))
        && rest.split('/').all(|part| !part.is_empty() && part != "." && !part.contains(".."))
}

fn valid_query(q: &str) -> bool {
    q.len() <= 1_024 && q.bytes().all(|b| b.is_ascii_graphic() && b != b'#')
}

/// Parses a proxy request for one of `hosts` (the enabled ones); `None` for anything that is not one (the caller
/// answers 404).
///
/// The repository path ends where the git path starts: `info/refs`, `git-upload-pack` or `git-receive-pack` at the
/// end, else the first `info/lfs/` after it. The upstream URL is built from the parsed path, so however a path splits,
/// the repository asked about is the one reached.
#[must_use]
pub fn parse(hosts: &[GitHost], method: &hyper::Method, path: &str, query: Option<&str>) -> Option<Route> {
    let rest = path.strip_prefix('/')?;
    let (req_host, rest) = rest.split_once('/')?;
    let host = hosts.iter().find(|h| h.host.eq_ignore_ascii_case(req_host))?;
    let parts: Vec<&str> = rest.split('/').collect();
    let get = method == hyper::Method::GET;
    let post = method == hyper::Method::POST;
    let (repo_len, kind) = match parts.as_slice() {
        [.., "info", "refs"] => {
            if !get {
                return None;
            }
            let service = query?.split('&').find_map(|p| p.strip_prefix("service="))?;
            let service = match service {
                "git-upload-pack" => Service::UploadPack,
                "git-receive-pack" => Service::ReceivePack,
                _ => return None,
            };
            (parts.len() - 2, Kind::InfoRefs(service))
        }
        [.., "git-upload-pack"] if post => (parts.len() - 1, Kind::Rpc(Service::UploadPack)),
        [.., "git-receive-pack"] if post => (parts.len() - 1, Kind::Rpc(Service::ReceivePack)),
        _ => {
            let at = (2..parts.len().saturating_sub(1)).find(|&i| parts[i] == "info" && parts[i + 1] == "lfs")?;
            let lfs = parts[at + 2..].join("/");
            if !valid_lfs(&lfs) || !(get || post) || query.is_some_and(|q| !valid_query(q)) {
                return None;
            }
            (
                at,
                Kind::Lfs {
                    rest: lfs,
                    query: query.map(str::to_owned),
                },
            )
        }
    };
    let repo = repo_parts(&host.service, &parts[..repo_len])?;
    Some(Route {
        repo: Repo::new(&host.host, &host.service, &repo),
        kind,
    })
}

#[cfg(test)]
mod tests {
    use hyper::Method;

    use super::*;

    fn p(method: &Method, path: &str, query: Option<&str>) -> Option<Route> {
        parse(&crate::config::Config::default().enabled_hosts().unwrap(), method, path, query)
    }

    #[test]
    fn git_paths_are_routed() {
        let r = p(&Method::GET, "/github.com/me/app.git/info/refs", Some("service=git-upload-pack")).unwrap();
        assert_eq!(r.repo.full_name(), "me/app");
        assert_eq!(r.kind, Kind::InfoRefs(Service::UploadPack));
        assert_eq!(r.suffix(), "/info/refs?service=git-upload-pack");
        let r = p(&Method::GET, "/GitHub.com/me/app/info/refs", Some("service=git-receive-pack")).unwrap();
        assert_eq!(r.kind, Kind::InfoRefs(Service::ReceivePack));
        assert_eq!(
            p(&Method::POST, "/github.com/me/a.b_c-d.git/git-receive-pack", None).unwrap().kind,
            Kind::Rpc(Service::ReceivePack)
        );
        assert_eq!(
            p(&Method::POST, "/github.com/me/app/git-upload-pack", None).unwrap().kind,
            Kind::Rpc(Service::UploadPack)
        );
        let lfs = p(&Method::POST, "/github.com/me/app.git/info/lfs/objects/batch", None).unwrap();
        assert_eq!(lfs.suffix(), "/info/lfs/objects/batch");
        let locks = p(&Method::GET, "/github.com/me/app.git/info/lfs/locks", Some("path=a.bin&limit=10")).unwrap();
        assert_eq!(locks.suffix(), "/info/lfs/locks?path=a.bin&limit=10");
    }

    #[test]
    fn anything_else_is_refused() {
        for (m, path, q) in [
            (Method::GET, "/gitlab.com/me/app/info/refs", Some("service=git-upload-pack")),
            (Method::GET, "/github.com/me/app/info/refs", Some("service=git-other")),
            (Method::GET, "/github.com/me/app/info/refs", None),
            (Method::POST, "/github.com/me/app/info/refs", Some("service=git-upload-pack")),
            (Method::GET, "/github.com/me/app/git-upload-pack", None),
            (Method::GET, "/github.com/../app/info/refs", Some("service=git-upload-pack")),
            (Method::GET, "/github.com/me/a..b/info/refs", Some("service=git-upload-pack")),
            (Method::GET, "/github.com/me/%2e%2e/info/refs", Some("service=git-upload-pack")),
            (Method::GET, "/github.com/me/.git/info/refs", Some("service=git-upload-pack")),
            (Method::GET, "/github.com/me//info/refs", Some("service=git-upload-pack")),
            (Method::GET, "/github.com/me/app/HEAD", None),
            (Method::GET, "/github.com/me/app/info/lfs/../x", None),
            (Method::GET, "/github.com/me/app/info/lfs/objects//x", None),
            (Method::DELETE, "/github.com/me/app/info/lfs/locks", None),
            (Method::GET, "/github.com/me/app/info/lfs/locks", Some("a=b#c")),
            (Method::GET, "/github.com/me", None),
        ] {
            assert!(p(&m, path, q).is_none(), "{m} {path} {q:?}");
        }
    }
}
