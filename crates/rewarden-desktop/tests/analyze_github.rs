//! `GitHubRemote` against a mock of the GitHub REST API answering from a bare repository.

mod analyze_util;

use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::{Arc, Mutex};

use analyze_util::{Fixture, cmd, git_bytes, git_run, lines, oid};
use bytes::Bytes;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::{Request, Response, StatusCode};
use rewarden_desktop::auth::Credential;
use rewarden_desktop::git::object::{ObjectKind, hash_object};
use rewarden_desktop::git::remote::{CommitMeta, RemoteError};
use rewarden_desktop::git::{GitHubRemote, Remote, analyze_push};
use serde_json::json;

#[derive(Default)]
struct Mock {
    dir: std::path::PathBuf,
    /// Changes one entry of every tree it serves.
    tamper_trees: bool,
    /// Compare answers by `a...b`: a status, or None for 404.
    compare: HashMap<String, Option<String>>,
    seen: Mutex<Vec<Seen>>,
}

#[derive(Clone, Debug)]
struct Seen {
    path: String,
    accept: String,
    version: String,
    agent: String,
    authorization: String,
}

fn answer(status: StatusCode, body: impl Into<Bytes>) -> Response<Full<Bytes>> {
    Response::builder().status(status).body(Full::new(body.into())).unwrap()
}

impl Mock {
    fn kind(&self, sha: &str) -> Option<String> {
        let out = git_run(&self.dir, &["cat-file", "-t", sha], None);
        out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
    }

    fn compare_status(&self, range: &str) -> Option<String> {
        if let Some(scripted) = self.compare.get(range) {
            return scripted.clone();
        }
        let (a, b) = range.split_once("...").unwrap();
        self.kind(a)?;
        self.kind(b)?;
        let ancestor =
            |x: &str, y: &str| git_run(&self.dir, &["merge-base", "--is-ancestor", x, y], None).status.success();
        let status = match (ancestor(a, b), ancestor(b, a)) {
            (true, true) => "identical",
            (true, false) => "ahead",
            (false, true) => "behind",
            (false, false) => "diverged",
        };
        Some(status.to_owned())
    }

    fn handle(&self, req: &Request<Incoming>) -> Response<Full<Bytes>> {
        let header = |name: &str| req.headers().get(name).map(|v| v.to_str().unwrap().to_owned()).unwrap_or_default();
        let path = req.uri().path().to_owned();
        self.seen.lock().unwrap().push(Seen {
            path: req.uri().to_string(),
            accept: header("accept"),
            version: header("x-github-api-version"),
            agent: header("user-agent"),
            authorization: header("authorization"),
        });
        let Some(rest) = path.strip_prefix("/repos/o/r/") else {
            return answer(StatusCode::NOT_FOUND, "");
        };
        let (what, sha) = rest.rsplit_once('/').unwrap();
        match what {
            "git/blobs" => match self.kind(sha).as_deref() {
                Some("blob") => answer(StatusCode::OK, git_bytes(&self.dir, &["cat-file", "blob", sha], None)),
                Some(_) => answer(StatusCode::UNPROCESSABLE_ENTITY, "{}"),
                None => answer(StatusCode::NOT_FOUND, "{}"),
            },
            "git/trees" => {
                if self.kind(sha).as_deref() != Some("tree") {
                    return answer(StatusCode::NOT_FOUND, "{}");
                }
                let raw = git_bytes(&self.dir, &["ls-tree", "-z", sha], None);
                let mut tree: Vec<_> = raw
                    .split(|&b| b == 0)
                    .filter(|l| !l.is_empty())
                    .map(|l| {
                        let l = String::from_utf8(l.to_vec()).unwrap();
                        let (meta, path) = l.split_once('\t').unwrap();
                        let parts: Vec<_> = meta.split(' ').collect();
                        json!({"path": path, "mode": parts[0], "type": parts[1], "sha": parts[2], "url": "x"})
                    })
                    .collect();
                if self.tamper_trees && !tree.is_empty() {
                    tree[0]["sha"] = json!("1111111111111111111111111111111111111111");
                }
                answer(StatusCode::OK, json!({"sha": sha, "tree": tree, "truncated": false}).to_string())
            }
            "git/commits" => {
                if self.kind(sha).as_deref() != Some("commit") {
                    return answer(StatusCode::UNPROCESSABLE_ENTITY, "{}");
                }
                let raw = String::from_utf8(git_bytes(&self.dir, &["cat-file", "commit", sha], None)).unwrap();
                let field = |key: &str| -> Vec<String> {
                    raw.lines()
                        .take_while(|l| !l.is_empty())
                        .filter_map(|l| l.strip_prefix(key).map(str::to_owned))
                        .collect()
                };
                let parents: Vec<_> = field("parent ").into_iter().map(|p| json!({"sha": p})).collect();
                let body = json!({"sha": sha, "tree": {"sha": field("tree ")[0]}, "parents": parents, "message": "m"});
                answer(StatusCode::OK, body.to_string())
            }
            "compare" => {
                let status = self.compare_status(sha);
                match status {
                    Some(s) => answer(StatusCode::OK, json!({"status": s, "files": [], "commits": []}).to_string()),
                    None => answer(StatusCode::NOT_FOUND, "{}"),
                }
            }
            _ => answer(StatusCode::NOT_FOUND, "{}"),
        }
    }
}

async fn serve(mock: Arc<Mock>) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let mock = Arc::clone(&mock);
            tokio::spawn(async move {
                let service = hyper::service::service_fn(move |req: Request<Incoming>| {
                    let mock = Arc::clone(&mock);
                    async move { Ok::<_, Infallible>(mock.handle(&req)) }
                });
                hyper::server::conn::http1::Builder::new()
                    .serve_connection(hyper_util::rt::TokioIo::new(stream), service)
                    .await
                    .ok();
            });
        }
    });
    format!("http://{addr}/")
}

fn credential() -> Credential {
    Credential {
        username: "x-access-token".to_owned(),
        token: zeroize::Zeroizing::new("ghs_secret".to_owned()),
        expires_at: i64::MAX,
    }
}

fn github(base: &str) -> GitHubRemote {
    GitHubRemote::new(rewarden_desktop::http::client(None).unwrap(), base, "o/r", Some(&credential()))
}

/// Server at `base` with a big file; the work tree one commit ahead with that file edited (a thin delta).
fn pushed() -> (Fixture, String, String) {
    let f = Fixture::new();
    let base = f.commit(&[("big.txt", Some(&lines(300, "v1"))), ("dir/a.txt", Some(b"a\n"))], "Start");
    f.publish("main");
    let mut big = lines(300, "v1");
    big.extend_from_slice(b"appended\n");
    let new = f.commit(&[("big.txt", Some(&big)), ("dir/b.txt", Some(b"b\n"))], "Edit");
    (f, base, new)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_thin_push_is_described_from_the_github_api() {
    let (f, base, new) = pushed();
    let mock = Arc::new(Mock {
        dir: f.server.clone(),
        ..Mock::default()
    });
    let url = serve(Arc::clone(&mock)).await;
    let pack = f.push_pack(&new, &[&base]);
    let s = analyze_push("o/r", &[cmd(&base, &new, "refs/heads/main")], &[], Some(pack.path()), &github(&url)).await;
    s.validate().unwrap();
    assert!(s.notes.is_empty(), "{:?}", s.notes);
    let u = &s.updates[0];
    assert_eq!((u.fast_forward, u.commit_count, u.files_changed), (Some(true), 1, 2));
    assert_eq!((u.additions, u.deletions), (Some(2), Some(0)));

    let seen = mock.seen.lock().unwrap().clone();
    assert!(!seen.is_empty());
    for r in &seen {
        assert_eq!(r.version, "2022-11-28");
        assert!(r.agent.starts_with("rewarden-desktop/"), "{r:?}");
        assert!(r.authorization.starts_with("Basic "), "{r:?}");
        let expected = if r.path.contains("/git/blobs/") {
            "application/vnd.github.raw"
        } else {
            "application/vnd.github+json"
        };
        assert_eq!(r.accept, expected, "{r:?}");
    }
    // The old tree came from the API (the pack only has the new one), the thin base as a raw blob.
    assert!(seen.iter().any(|r| r.path.contains("/git/trees/")));
    assert!(seen.iter().any(|r| r.path.contains("/git/blobs/")));
    assert!(seen.iter().any(|r| r.path.contains("/git/commits/")));
    // The walk reached `old`: no compare needed.
    assert!(!seen.iter().any(|r| r.path.contains("/compare/")));
}

#[tokio::test(flavor = "multi_thread")]
async fn objects_trees_and_commits_are_checked() {
    let (f, base, _) = pushed();
    let url = serve(Arc::new(Mock {
        dir: f.server.clone(),
        ..Mock::default()
    }))
    .await;
    let gh = github(&url);
    let tree = oid(&f.rev(&format!("{base}^{{tree}}")));
    let blob = oid(&f.rev(&format!("{base}:dir/a.txt")));

    let (kind, raw) = gh.object(&tree, None, 1 << 20).await.unwrap().unwrap();
    assert_eq!((kind, hash_object(kind, &raw)), (ObjectKind::Tree, tree));
    assert_eq!(
        gh.object(&blob, Some(ObjectKind::Blob), 1 << 20).await.unwrap(),
        Some((ObjectKind::Blob, b"a\n".to_vec()))
    );
    assert_eq!(gh.object(&blob, Some(ObjectKind::Blob), 1).await, Err(RemoteError::TooLarge));
    assert_eq!(gh.object(&blob, Some(ObjectKind::Tree), 1 << 20).await.unwrap(), None);
    // Commits are never had raw.
    assert_eq!(gh.object(&oid(&base), None, 1 << 20).await.unwrap(), None);
    assert_eq!(gh.object(&oid(&base), Some(ObjectKind::Commit), 1 << 20).await.unwrap(), None);
    assert_eq!(
        gh.commit(&oid(&base)).await.unwrap(),
        Some(CommitMeta {
            tree,
            parents: vec![]
        })
    );
    assert_eq!(gh.commit(&blob).await.unwrap(), None);

    let tampered = serve(Arc::new(Mock {
        dir: f.server.clone(),
        tamper_trees: true,
        ..Mock::default()
    }))
    .await;
    let err = github(&tampered).object(&tree, Some(ObjectKind::Tree), 1 << 20).await.unwrap_err();
    assert!(err.to_string().contains("does not match"), "{err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_tampered_tree_is_a_note() {
    let (f, base, new) = pushed();
    let url = serve(Arc::new(Mock {
        dir: f.server.clone(),
        tamper_trees: true,
        ..Mock::default()
    }))
    .await;
    // Not thin: the pack has every new object, only the old tree comes from the API.
    let pack = f.pack(&new, &[&base], &[]);
    let s = analyze_push("o/r", &[cmd(&base, &new, "refs/heads/main")], &[], Some(pack.path()), &github(&url)).await;
    s.validate().unwrap();
    assert!(s.notes.iter().any(|n| n.contains("does not match")), "{:?}", s.notes);
    assert_eq!(s.updates[0].additions, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn compare_statuses_give_ancestry() {
    let (a, b) = ("1".repeat(40), "2".repeat(40));
    let mut compare = HashMap::new();
    for (i, status) in ["ahead", "identical", "behind", "diverged", "odd"].iter().enumerate() {
        compare.insert(format!("{a}...{}", (i + 3).to_string().repeat(40)), Some((*status).to_owned()));
    }
    compare.insert(format!("{a}...{b}"), None);
    let url = serve(Arc::new(Mock {
        dir: std::env::temp_dir(),
        compare,
        ..Mock::default()
    }))
    .await;
    let gh = github(&url);
    for (i, want) in [Some(true), Some(true), Some(false), Some(false), None].into_iter().enumerate() {
        assert_eq!(gh.is_ancestor(&oid(&a), &oid(&(i + 3).to_string().repeat(40))).await.unwrap(), want, "{i}");
    }
    assert_eq!(gh.is_ancestor(&oid(&a), &oid(&b)).await.unwrap(), None);
    assert_eq!(gh.is_ancestor(&oid(&a), &oid(&a)).await.unwrap(), Some(true));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_request_budget_is_kept() {
    let (f, base, new) = pushed();
    let mock = Arc::new(Mock {
        dir: f.server.clone(),
        ..Mock::default()
    });
    let url = serve(Arc::clone(&mock)).await;
    let gh = github(&url).with_budget(1);
    assert_eq!(gh.is_ancestor(&oid(&base), &oid(&new)).await.unwrap(), None);
    assert_eq!(gh.is_ancestor(&oid(&base), &oid(&new)).await, Err(RemoteError::Budget));
    assert_eq!(mock.seen.lock().unwrap().len(), 1);

    let pack = f.push_pack(&new, &[&base]);
    let s = analyze_push(
        "o/r",
        &[cmd(&base, &new, "refs/heads/main")],
        &[],
        Some(pack.path()),
        &github(&url).with_budget(0),
    )
    .await;
    s.validate().unwrap();
    assert!(s.notes.iter().any(|n| n.contains("budget")), "{:?}", s.notes);
    // The commit is in the pack (not a delta), so the walk still reaches `old`.
    assert_eq!(s.updates[0].fast_forward, Some(true));
    assert_eq!(s.updates[0].additions, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn errors_from_github_are_notes() {
    let (_fixture, base, new) = pushed();
    // Nothing listens there any more.
    let url = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        format!("http://{}", l.local_addr().unwrap())
    };
    let s = analyze_push("o/r", &[cmd(&base, &new, "refs/heads/main")], &[], None, &github(&url)).await;
    s.validate().unwrap();
    assert_eq!(s.updates[0].fast_forward, None);
    assert!(s.notes.iter().any(|n| n.contains("GitHub request failed")), "{:?}", s.notes);
}
