//! Proxy paths for every git host: `/<host>/<repo path>(.git)/<smart-HTTP or LFS path>`, nested repository paths on
//! GitLab only, and strict refusals.

use hyper::Method;
use rewarden_desktop::config::{Config, GitHost};
use rewarden_desktop::proxy::route::{Kind, Route, Service, parse, repo_path};

fn hosts() -> Vec<GitHost> {
    let c: Config = toml::from_str(
        "[[git.hosts]]\nhost = \"gitlab.com\"\nenabled = true\n[[git.hosts]]\nhost = \"codeberg.org\"\nenabled = true\n[[git.hosts]]\nhost = \"bitbucket.org\"\nenabled = true\n",
    )
    .unwrap();
    c.enabled_hosts().unwrap()
}

fn p(method: &Method, path: &str, query: Option<&str>) -> Option<Route> {
    parse(&hosts(), method, path, query)
}

const UP: Option<&str> = Some("service=git-upload-pack");

#[test]
fn each_host_routes_with_its_service() {
    for (host, service) in [
        ("github.com", "github"),
        ("gitlab.com", "gitlab"),
        ("codeberg.org", "codeberg"),
        ("bitbucket.org", "bitbucket"),
    ] {
        let r = p(&Method::GET, &format!("/{host}/me/app.git/info/refs"), UP).unwrap();
        assert_eq!((r.repo.host.as_str(), r.repo.service.as_str(), r.repo.path.as_str()), (host, service, "me/app"));
        assert_eq!(r.repo.full_name(), "me/app");
        assert_eq!(r.repo.label(), format!("{host}/me/app"));
        assert_eq!(r.kind, Kind::InfoRefs(Service::UploadPack));
    }
    // The host is matched ignoring case, and the configured spelling is kept.
    let r = p(&Method::POST, "/GitLab.COM/me/app/git-receive-pack", None).unwrap();
    assert_eq!((r.repo.host.as_str(), r.kind), ("gitlab.com", Kind::Rpc(Service::ReceivePack)));
}

#[test]
fn gitlab_paths_nest_groups() {
    let r =
        p(&Method::GET, "/gitlab.com/group/sub/deeper/app.git/info/refs", Some("service=git-receive-pack")).unwrap();
    assert_eq!(r.repo.path, "group/sub/deeper/app");
    assert_eq!(r.kind, Kind::InfoRefs(Service::ReceivePack));
    assert_eq!(r.suffix(), "/info/refs?service=git-receive-pack");
    let r = p(&Method::POST, "/gitlab.com/group/sub/app/git-upload-pack", None).unwrap();
    assert_eq!((r.repo.path.as_str(), r.kind), ("group/sub/app", Kind::Rpc(Service::UploadPack)));
    let lfs = p(&Method::POST, "/gitlab.com/group/sub/app.git/info/lfs/objects/batch", None).unwrap();
    assert_eq!((lfs.repo.path.as_str(), lfs.suffix().as_str()), ("group/sub/app", "/info/lfs/objects/batch"));
    let locks = p(&Method::GET, "/gitlab.com/g/s/app.git/info/lfs/locks", Some("path=a.bin")).unwrap();
    assert_eq!(locks.suffix(), "/info/lfs/locks?path=a.bin");
    // Twenty parts are fine, twenty-one are not.
    let deep = vec!["g"; 20].join("/");
    assert!(p(&Method::POST, &format!("/gitlab.com/{deep}/git-upload-pack"), None).is_some());
    let deeper = vec!["g"; 21].join("/");
    assert!(p(&Method::POST, &format!("/gitlab.com/{deeper}/git-upload-pack"), None).is_none());
}

#[test]
fn repo_paths_are_checked_per_service() {
    assert_eq!(repo_path("gitlab", "a/b/c.git").as_deref(), Some("a/b/c"));
    assert_eq!(repo_path("github", "a/b").as_deref(), Some("a/b"));
    assert_eq!(repo_path("github", "a/b/c"), None);
    assert_eq!(repo_path("codeberg", "a/b/c.git"), None);
    assert_eq!(repo_path("gitlab", "a"), None);
    assert_eq!(repo_path("gitlab", "a.git/b"), None);
    assert_eq!(repo_path("gitlab", "a/-/b"), None);
    assert_eq!(repo_path("gitlab", "a/b.git.git"), None);
    assert_eq!(repo_path("nope", "a/b"), None);
    let long = format!("{}/{}/{}", "a".repeat(100), "b".repeat(100), "c".repeat(60));
    assert_eq!(repo_path("gitlab", &long), None, "longer than the phone accepts");
}

#[test]
fn anything_else_is_refused() {
    for (m, path, q) in [
        // Not a configured (or enabled) host.
        (Method::GET, "/gitea.com/me/app/info/refs", UP),
        (Method::GET, "/example.com/me/app/info/refs", UP),
        // Nesting only on GitLab.
        (Method::GET, "/github.com/a/b/c/info/refs", UP),
        (Method::GET, "/codeberg.org/a/b/c.git/info/refs", UP),
        (Method::POST, "/bitbucket.org/a/b/c/git-upload-pack", None),
        (Method::GET, "/codeberg.org/a/b/c/info/lfs/objects/batch", None),
        // Too short.
        (Method::GET, "/gitlab.com/app/info/refs", UP),
        (Method::GET, "/gitlab.com/info/refs", UP),
        // Bad parts anywhere in a nested path.
        (Method::GET, "/gitlab.com/g/../app/info/refs", UP),
        (Method::GET, "/gitlab.com/g/./app/info/refs", UP),
        (Method::GET, "/gitlab.com/g//app/info/refs", UP),
        (Method::GET, "/gitlab.com/g/%2e%2e/app/info/refs", UP),
        (Method::GET, "/gitlab.com/g/a..b/app/info/refs", UP),
        (Method::GET, "/gitlab.com/g/sub.git/app/info/refs", UP),
        (Method::GET, "/gitlab.com/g/-/app/info/refs", UP),
        (Method::GET, "/gitlab.com/g/s p/app/info/refs", UP),
        (Method::GET, "/gitlab.com/g/.git/info/refs", UP),
        // Wrong services, methods and tails.
        (Method::GET, "/gitlab.com/g/s/app/info/refs", Some("service=git-other")),
        (Method::GET, "/gitlab.com/g/s/app/info/refs", None),
        (Method::POST, "/gitlab.com/g/s/app/info/refs", UP),
        (Method::GET, "/gitlab.com/g/s/app/git-upload-pack", None),
        (Method::GET, "/gitlab.com/g/s/app/HEAD", None),
        (Method::GET, "/gitlab.com/g/s/app/info/lfs/../x", None),
        (Method::GET, "/gitlab.com/g/s/app/info/lfs/", None),
        (Method::DELETE, "/gitlab.com/g/s/app/info/lfs/locks", None),
        (Method::GET, "/gitlab.com/g/s/app/info/lfs/locks", Some("a=b#c")),
        (Method::GET, "/gitlab.com", None),
        (Method::GET, "/gitlab.com/", None),
    ] {
        assert!(p(&m, path, q).is_none(), "{m} {path} {q:?}");
    }
}

#[test]
fn disabled_hosts_are_not_served() {
    let defaults = Config::default().enabled_hosts().unwrap();
    assert_eq!(defaults.iter().map(|h| h.host.as_str()).collect::<Vec<_>>(), ["github.com"]);
    assert!(parse(&defaults, &Method::GET, "/gitlab.com/g/app/info/refs", UP).is_none());
    assert!(parse(&defaults, &Method::GET, "/github.com/me/app/info/refs", UP).is_some());
}
