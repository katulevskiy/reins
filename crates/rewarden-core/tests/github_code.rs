//! GitHub files, commits, tags and releases against a fake GitHub: the requests each tool sends, what it makes of the
//! answers, what a preview promises and what a write really does.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::FakeKeys;
use data_encoding::BASE64;
use rewarden_core::CoreError;
use rewarden_core::connector::github::GitHub;
use rewarden_core::connector::{Connector, Item, Preview};
use rewarden_proto::connector::{ConnectorCall, spec_for_tool};
use serde_json::{Value, json};
use wiremock::matchers::{body_bytes, body_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const ACCOUNT: &str = "octo-cat";

struct Env {
    server: MockServer,
    uploads: MockServer,
    gh: GitHub,
    _dir: tempfile::TempDir,
}

async fn env() -> Env {
    let server = MockServer::start().await;
    let uploads = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/user"))
        .and(header("authorization", "Bearer ghp_secret"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"login": "Octo-Cat"})))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(rewarden_core::store::Store::open(dir.path(), &FakeKeys).unwrap());
    let client = rewarden_core::http::client().unwrap();
    let gh = GitHub::new(client, &server.uri(), store, Duration::from_millis(1)).with_upload_base(&uploads.uri());
    assert_eq!(gh.sign_in("ghp_secret").await.unwrap(), ACCOUNT);
    Env {
        server,
        uploads,
        gh,
        _dir: dir,
    }
}

fn call(tool: &str, args: &Value) -> ConnectorCall {
    spec_for_tool(tool).unwrap().parse(args).unwrap().0
}

impl Env {
    async fn fetch(&self, tool: &str, args: &Value) -> Result<Vec<Item>, CoreError> {
        self.gh.fetch(ACCOUNT, &call(tool, args)).await
    }

    async fn preview(&self, tool: &str, args: &Value) -> Result<Preview, CoreError> {
        self.gh.preview(ACCOUNT, &call(tool, args)).await
    }

    async fn perform(&self, tool: &str, args: &Value) -> Result<Value, CoreError> {
        self.gh.perform(ACCOUNT, &call(tool, args)).await
    }

    /// Answers `method path` with `status` and a JSON body.
    async fn on(&self, verb: &str, p: &str, status: u16, body: &Value) {
        Mock::given(method(verb))
            .and(path(p))
            .respond_with(ResponseTemplate::new(status).set_body_json(body))
            .mount(&self.server)
            .await;
    }

    /// The requests the fake GitHub saw after sign-in: (method, path, query, body as JSON or null).
    async fn seen(&self) -> Vec<(String, String, String, Value)> {
        self.server
            .received_requests()
            .await
            .unwrap()
            .iter()
            // wiremock probes pooled servers with a HEAD request.
            .filter(|r| r.url.path() != "/user" && r.method.as_str() != "HEAD")
            .map(|r| {
                (
                    r.method.to_string(),
                    r.url.path().to_owned(),
                    r.url.query().unwrap_or_default().to_owned(),
                    serde_json::from_slice(&r.body).unwrap_or(Value::Null),
                )
            })
            .collect()
    }

    async fn seen_paths(&self) -> Vec<String> {
        self.seen().await.into_iter().map(|(m, p, _, _)| format!("{m} {p}")).collect()
    }

    /// The repository exists, `main` is its default branch and the branches given point to the commits given.
    async fn repo(&self, branches: &[(&str, &str)]) {
        self.on("GET", "/repos/octo/cat", 200, &json!({"default_branch": "main"})).await;
        for (name, sha) in branches {
            self.on("GET", &format!("/repos/octo/cat/git/ref/heads/{name}"), 200, &json!({"object": {"sha": sha}}))
                .await;
        }
    }
}

fn sha(c: char) -> String {
    c.to_string().repeat(40)
}

fn b64(s: &str) -> String {
    BASE64.encode(s.as_bytes())
}

fn lines(p: &Preview) -> String {
    p.lines.join("\n")
}

// ---- contents -------------------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_text_file_on_a_branch_is_read_as_text_and_belongs_to_that_branch() {
    let e = env().await;
    e.on("GET", "/repos/octo/cat/git/ref/heads/main", 200, &json!({"object": {"sha": sha('1')}})).await;
    // GitHub wraps the base64 every 60 characters.
    let content = format!("{}\n{}\n", &b64("fn main() {}\n")[..8], &b64("fn main() {}\n")[8..]);
    Mock::given(method("GET"))
        .and(path("/repos/octo/cat/contents/src/main%20x.rs"))
        .and(query_param("ref", "main"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "type": "file", "encoding": "base64", "content": content, "sha": sha('a'), "size": 13,
            "html_url": "https://github.com/octo/cat/blob/main/src/main x.rs"})))
        .expect(1)
        .mount(&e.server)
        .await;
    let items =
        e.fetch("github_file_get", &json!({"repo": "octo/cat", "path": "src/main x.rs", "ref": "main"})).await.unwrap();
    assert_eq!(items.len(), 1);
    let item = &items[0];
    assert_eq!((item.resource.as_str(), item.resource_label.as_str()), ("octo/cat@main", "octo/cat, branch main"));
    assert_eq!(
        item.parents,
        [
            ("octo/cat".to_owned(), "Any branch of octo/cat".to_owned()),
            ("octo".to_owned(), "Every repository of octo".to_owned())
        ]
    );
    assert_eq!(item.body.as_deref(), Some("fn main() {}\n"));
    assert_eq!(item.extra["encoding"], "text");
    assert_eq!(item.extra["sha"], sha('a'));
    assert_eq!(item.extra["path"], "src/main x.rs");
    assert!(!item.sensitive && !item.secret);
}

#[tokio::test]
async fn a_tag_a_commit_or_no_ref_is_read_as_the_whole_repository() {
    let e = env().await;
    // "v1" is not a branch: the branch lookup answers 404.
    e.on("GET", "/repos/octo/cat/git/ref/heads/v1", 404, &json!({"message": "Not Found"})).await;
    let file = json!({"type": "file", "encoding": "base64", "content": b64("hi"), "sha": sha('a'), "size": 2});
    e.on("GET", "/repos/octo/cat/contents/README.md", 200, &file).await;
    for git_ref in [json!("v1"), json!(sha('c')), Value::Null] {
        let mut args = json!({"repo": "octo/cat", "path": "README.md"});
        if !git_ref.is_null() {
            args["ref"] = git_ref;
        }
        let items = e.fetch("github_file_get", &args).await.unwrap();
        assert_eq!(items[0].resource, "octo/cat", "{args}");
        assert_eq!(items[0].parents, [("octo".to_owned(), "Every repository of octo".to_owned())]);
    }
    let seen = e.seen_paths().await;
    assert!(seen.contains(&"GET /repos/octo/cat/git/ref/heads/v1".to_owned()));
    assert!(!seen.iter().any(|p| p.contains("heads/cccc")), "a commit sha is never looked up as a branch: {seen:?}");
    let (_, _, query, _) = e.seen().await.into_iter().find(|(_, p, _, _)| p.ends_with("README.md")).unwrap();
    assert_eq!(query, "ref=v1");
}

#[tokio::test]
async fn binary_files_come_as_base64_and_large_ones_as_metadata() {
    let e = env().await;
    let bytes = [0xffu8, 0x00, 0x01, 0x80];
    e.on(
        "GET",
        "/repos/octo/cat/contents/logo.png",
        200,
        &json!({"type": "file", "encoding": "base64", "content": BASE64.encode(&bytes), "sha": sha('b'), "size": 4}),
    )
    .await;
    let item = &e.fetch("github_file_get", &json!({"repo": "octo/cat", "path": "logo.png"})).await.unwrap()[0];
    assert_eq!(item.body, None);
    assert_eq!(item.extra["encoding"], "base64");
    assert_eq!(item.extra["content_base64"], BASE64.encode(&bytes));
    assert!(item.snippet.contains("binary"));

    // 1.5 MB: the contents API leaves `content` empty, the blob API has it.
    e.on(
        "GET",
        "/repos/octo/cat/contents/big.txt",
        200,
        &json!({"type": "file", "encoding": "none", "content": "", "sha": sha('d'), "size": 1_500_000}),
    )
    .await;
    let big = "x".repeat(1_500_000);
    e.on(
        "GET",
        &format!("/repos/octo/cat/git/blobs/{}", sha('d')),
        200,
        &json!({"encoding": "base64", "content": b64(&big), "size": 1_500_000}),
    )
    .await;
    let item = &e.fetch("github_file_get", &json!({"repo": "octo/cat", "path": "big.txt"})).await.unwrap()[0];
    assert_eq!(item.body.as_ref().unwrap().len(), 100_000, "text is cut at 100000 characters");
    assert_eq!(item.extra["truncated"], true);
    assert_eq!(item.extra["size"], 1_500_000);

    // 3 MB: nothing is downloaded.
    e.on(
        "GET",
        "/repos/octo/cat/contents/huge.bin",
        200,
        &json!({"type": "file", "encoding": "none", "content": "", "sha": sha('e'), "size": 3_000_000}),
    )
    .await;
    let item = &e.fetch("github_file_get", &json!({"repo": "octo/cat", "path": "huge.bin"})).await.unwrap()[0];
    assert_eq!(
        (item.body.as_deref(), item.extra["encoding"].as_str(), item.extra["too_large"].as_bool()),
        (None, Some("none"), Some(true))
    );
    assert!(!e.seen_paths().await.iter().any(|p| p.contains(&sha('e'))), "no blob is fetched for a file over 2 MB");
}

#[tokio::test]
async fn file_get_refuses_bad_paths_directories_and_reports_symlinks() {
    let e = env().await;
    for bad in ["../secret", "/etc/passwd", "a//b", "a\\b", "src/../x", ".git/config", "a/"] {
        let err = e.fetch("github_file_get", &json!({"repo": "octo/cat", "path": bad})).await.unwrap_err();
        assert!(err.to_string().contains("not a valid file path"), "{bad}: {err}");
    }
    assert!(e.seen().await.is_empty(), "a bad path never reaches the network");
    let err = e.fetch("github_file_get", &json!({"repo": "../x", "path": "a"})).await.unwrap_err();
    assert!(err.to_string().contains("owner/name"));
    let err = e.fetch("github_file_get", &json!({"repo": "octo/cat", "path": "a", "ref": "a..b"})).await.unwrap_err();
    assert!(err.to_string().contains("`ref`"));

    e.on("GET", "/repos/octo/cat/contents/src", 200, &json!([{"name": "a.rs", "path": "src/a.rs", "type": "file"}]))
        .await;
    let err = e.fetch("github_file_get", &json!({"repo": "octo/cat", "path": "src"})).await.unwrap_err();
    assert!(err.to_string().contains("github_dir_list"), "{err}");

    e.on(
        "GET",
        "/repos/octo/cat/contents/link",
        200,
        &json!({"type": "symlink", "target": "real.txt", "sha": sha('a'), "size": 8}),
    )
    .await;
    let item = &e.fetch("github_file_get", &json!({"repo": "octo/cat", "path": "link"})).await.unwrap()[0];
    assert_eq!((item.body.as_deref(), item.extra["target"].as_str()), (None, Some("real.txt")));
}

#[tokio::test]
async fn directories_are_listed_from_the_top_or_from_a_path() {
    let e = env().await;
    let listing = json!([
        {"name": "src", "path": "src", "type": "dir", "size": 0, "sha": sha('1')},
        {"name": "Cargo.toml", "path": "Cargo.toml", "type": "file", "size": 120, "sha": sha('2')},
        {"name": "x", "path": "x", "type": "file", "size": 1, "sha": sha('3')}]);
    e.on("GET", "/repos/octo/cat/contents", 200, &listing).await;
    e.on("GET", "/repos/octo/cat/contents/docs/api", 200, &listing).await;
    let items = e.fetch("github_dir_list", &json!({"repo": "octo/cat", "limit": 2})).await.unwrap();
    assert_eq!(
        items.iter().map(|i| (i.id.as_str(), i.from.as_str(), i.snippet.as_str())).collect::<Vec<_>>(),
        [("src", "dir", "dir"), ("Cargo.toml", "file", "file · 120 bytes")]
    );
    assert_eq!(items[0].resource, "octo/cat");
    let items =
        e.fetch("github_dir_list", &json!({"repo": "octo/cat", "path": "docs/api", "ref": "v2"})).await.unwrap();
    assert_eq!(items.len(), 3);
    let seen = e.seen().await;
    assert!(seen.iter().any(|(_, p, q, _)| p == "/repos/octo/cat/contents/docs/api" && q == "ref=v2"));
    e.on("GET", "/repos/octo/cat/contents/README.md", 200, &json!({"type": "file"})).await;
    let err = e.fetch("github_dir_list", &json!({"repo": "octo/cat", "path": "README.md"})).await.unwrap_err();
    assert!(err.to_string().contains("github_file_get"));
    assert!(e.fetch("github_dir_list", &json!({"repo": "octo/cat", "path": "../.."})).await.is_err());
}

#[tokio::test]
async fn the_tree_is_listed_recursively_cut_and_filtered() {
    let e = env().await;
    e.on("GET", "/repos/octo/cat", 200, &json!({"default_branch": "trunk"})).await;
    let tree = json!({"sha": sha('9'), "truncated": false, "tree": [
        {"path": "README.md", "type": "blob", "size": 10},
        {"path": "src", "type": "tree"},
        {"path": "src/lib.rs", "type": "blob", "size": 20},
        {"path": "src/util", "type": "tree"},
        {"path": "src/util/mod.rs", "type": "blob", "size": 30}]});
    e.on("GET", "/repos/octo/cat/git/trees/trunk", 200, &tree).await;
    let item = &e.fetch("github_tree_get", &json!({"repo": "octo/cat"})).await.unwrap()[0];
    assert_eq!(
        item.body.as_deref().unwrap(),
        "blob 10 README.md\ntree - src\nblob 20 src/lib.rs\ntree - src/util\nblob 30 src/util/mod.rs\n"
    );
    assert_eq!((item.extra["entries"].as_i64(), item.extra["truncated"].as_bool()), (Some(5), Some(false)));
    assert_eq!(item.resource, "octo/cat", "no ref: the whole repository");
    let seen = e.seen().await;
    assert!(seen.iter().any(|(_, p, q, _)| p == "/repos/octo/cat/git/trees/trunk" && q == "recursive=1"));

    let item = &e.fetch("github_tree_get", &json!({"repo": "octo/cat", "limit": 2})).await.unwrap()[0];
    assert_eq!((item.extra["entries"].as_i64(), item.extra["truncated"].as_bool()), (Some(2), Some(true)));

    // A directory, one level.
    let item =
        &e.fetch("github_tree_get", &json!({"repo": "octo/cat", "path": "src", "recursive": false})).await.unwrap()[0];
    assert_eq!(item.body.as_deref().unwrap(), "blob 20 src/lib.rs\ntree - src/util\n");

    // GitHub says the listing itself was cut.
    e.on(
        "GET",
        "/repos/octo/cat/git/trees/big",
        200,
        &json!({"truncated": true, "tree": [{"path": "a", "type": "blob", "size": 1}]}),
    )
    .await;
    e.on("GET", "/repos/octo/cat/git/ref/heads/big", 404, &json!({})).await;
    let item = &e.fetch("github_tree_get", &json!({"repo": "octo/cat", "ref": "big"})).await.unwrap()[0];
    assert_eq!(item.extra["truncated"], true);
}

#[tokio::test]
async fn blobs_are_read_by_sha() {
    let e = env().await;
    e.on(
        "GET",
        &format!("/repos/octo/cat/git/blobs/{}", sha('a')),
        200,
        &json!({"encoding": "base64", "content": b64("plain"), "size": 5}),
    )
    .await;
    let item =
        &e.fetch("github_blob_get", &json!({"repo": "octo/cat", "sha": sha('a').to_uppercase()})).await.unwrap()[0];
    assert_eq!((item.body.as_deref(), item.resource.as_str()), (Some("plain"), "octo/cat"));
    e.on(
        "GET",
        &format!("/repos/octo/cat/git/blobs/{}", sha('b')),
        200,
        &json!({"encoding": "base64", "content": BASE64.encode(&[0u8, 1, 2]), "size": 3}),
    )
    .await;
    let item = &e.fetch("github_blob_get", &json!({"repo": "octo/cat", "sha": sha('b')})).await.unwrap()[0];
    assert_eq!(item.extra["content_base64"], BASE64.encode(&[0u8, 1, 2]));
    e.on(
        "GET",
        &format!("/repos/octo/cat/git/blobs/{}", sha('c')),
        200,
        &json!({"encoding": "base64", "content": "", "size": 5_000_000}),
    )
    .await;
    let item = &e.fetch("github_blob_get", &json!({"repo": "octo/cat", "sha": sha('c')})).await.unwrap()[0];
    assert_eq!(item.extra["too_large"], true);
    for bad in ["main", "abc", "../../x", &"g".repeat(40)] {
        let err = e.fetch("github_blob_get", &json!({"repo": "octo/cat", "sha": bad})).await.unwrap_err();
        assert!(err.to_string().contains("git sha"), "{bad}: {err}");
    }
}

#[tokio::test]
async fn the_archive_link_is_the_redirect_target_and_nothing_is_downloaded() {
    let e = env().await;
    let target = format!("{}/codeload/octo/cat/zip/main", e.server.uri());
    Mock::given(method("GET"))
        .and(path("/repos/octo/cat/zipball/main"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", target.as_str()))
        .expect(1)
        .mount(&e.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/octo/cat/tarball"))
        .respond_with(
            ResponseTemplate::new(302)
                .insert_header("location", "https://codeload.github.com/octo/cat/legacy.tar.gz/refs/heads/main"),
        )
        .mount(&e.server)
        .await;
    e.on("GET", "/repos/octo/cat/git/ref/heads/main", 200, &json!({"object": {"sha": sha('1')}})).await;
    let item = &e.fetch("github_archive_link", &json!({"repo": "octo/cat", "ref": "main"})).await.unwrap()[0];
    assert_eq!(item.extra["url"], target);
    assert_eq!(item.resource, "octo/cat@main");
    assert!(!e.seen_paths().await.iter().any(|p| p.contains("codeload")), "the archive is not fetched");
    let item = &e.fetch("github_archive_link", &json!({"repo": "octo/cat", "format": "tar"})).await.unwrap()[0];
    assert_eq!(item.extra["url"], "https://codeload.github.com/octo/cat/legacy.tar.gz/refs/heads/main");
    assert_eq!(item.extra["format"], "tar.gz");

    // A redirect to a plain http address elsewhere is not passed on.
    Mock::given(method("GET"))
        .and(path("/repos/octo/cat/zipball/evil"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", "http://evil.example/x"))
        .mount(&e.server)
        .await;
    let err = e.fetch("github_archive_link", &json!({"repo": "octo/cat", "ref": "evil"})).await.unwrap_err();
    assert!(err.to_string().contains("not https"), "{err}");
    // No redirect at all, or an error.
    e.on("GET", "/repos/octo/cat/zipball/none", 404, &json!({"message": "Not Found"})).await;
    let err = e.fetch("github_archive_link", &json!({"repo": "octo/cat", "ref": "none"})).await.unwrap_err();
    assert!(err.to_string().contains("does not exist"), "{err}");
}

// ---- commits --------------------------------------------------------------------------------------------------------

#[tokio::test]
async fn commits_are_listed_with_their_filters() {
    let e = env().await;
    e.on("GET", "/repos/octo/cat/git/ref/heads/dev", 200, &json!({"object": {"sha": sha('1')}})).await;
    Mock::given(method("GET"))
        .and(path("/repos/octo/cat/commits"))
        .and(query_param("sha", "dev"))
        .and(query_param("path", "src/lib.rs"))
        .and(query_param("author", "ann"))
        .and(query_param("since", "2026-10-05T00:00:00Z"))
        .and(query_param("until", "2026-10-06T14:00:00Z"))
        .and(query_param("per_page", "5"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"sha": sha('a'), "html_url": "https://gh/a", "author": {"login": "ann"},
             "commit": {"message": "Fix the bug\n\nlong text", "author": {"name": "Ann L", "date": "2026-10-05T14:30:00Z"}}},
            {"sha": sha('b'), "html_url": "https://gh/b", "author": null,
             "commit": {"message": "Second", "author": {"name": "Bob", "date": "2026-10-04T10:00:00Z"}}}])))
        .expect(1)
        .mount(&e.server)
        .await;
    let items = e
        .fetch(
            "github_commit_list",
            &json!({"repo": "octo/cat", "ref": "dev", "path": "src/lib.rs", "author": "ann",
            "since": "2026-10-05", "until": "2026-10-06T14:00:00Z", "limit": 5}),
        )
        .await
        .unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!((items[0].title.as_str(), items[0].from.as_str(), items[0].date), ("Fix the bug", "ann", 1_791_210_600));
    assert_eq!(items[0].snippet, "aaaaaaa · ann");
    assert_eq!(items[0].resource, "octo/cat@dev");
    assert_eq!(items[1].from, "Bob", "falls back to the commit's author name");
    for (arg, value) in [("since", "yesterday"), ("until", "2026-13-40")] {
        let mut args = json!({"repo": "octo/cat"});
        args[arg] = json!(value);
        assert!(e.fetch("github_commit_list", &args).await.unwrap_err().to_string().contains(arg));
    }
    assert!(e.fetch("github_commit_list", &json!({"repo": "octo/cat", "path": "../x"})).await.is_err());
}

#[tokio::test]
async fn a_commit_is_read_with_its_message_stats_and_patches() {
    let e = env().await;
    e.on("GET", "/repos/octo/cat/git/ref/heads/feature/x", 404, &json!({})).await;
    e.on(
        "GET",
        &format!("/repos/octo/cat/commits/{}", sha('a')),
        200,
        &json!({"sha": sha('a'), "html_url": "https://gh/a", "stats": {"additions": 3, "deletions": 1, "total": 4},
            "parents": [{"sha": sha('p')}],
            "commit": {"message": "Rename things\n\nDetails", "author": {"name": "Ann", "email": "ann@x.io", "date": "2026-10-05T14:30:00Z"}},
            "files": [
                {"filename": "b.rs", "previous_filename": "a.rs", "status": "renamed", "additions": 0, "deletions": 0},
                {"filename": "c.rs", "status": "modified", "additions": 3, "deletions": 1, "patch": "@@ -1 +1,3 @@\n-old\n+new"},
                {"filename": "d.rs", "status": "removed", "additions": 0, "deletions": 9}]}),
    )
    .await;
    let item = &e.fetch("github_commit_get", &json!({"repo": "octo/cat", "ref": sha('a')})).await.unwrap()[0];
    let body = item.body.as_deref().unwrap();
    assert!(
        body.starts_with("Rename things\n\nDetails\n\nAuthor: Ann <ann@x.io>\nStats: +3 -1 in 3 file(s)\n"),
        "{body}"
    );
    assert!(body.contains("R a.rs -> b.rs (+0 -0)\nM c.rs (+3 -1)\nD d.rs (+0 -9)\n"), "{body}");
    assert!(body.contains("--- c.rs\n@@ -1 +1,3 @@\n-old\n+new"), "{body}");
    assert_eq!((item.title.as_str(), item.extra["truncated"].as_bool()), ("Rename things", Some(false)));
    assert_eq!(item.extra["parents"], json!([sha('p')]));
    assert_eq!(item.resource, "octo/cat");

    // A huge patch is cut at 60000 characters.
    e.on(
        "GET",
        "/repos/octo/cat/commits/feature/x",
        200,
        &json!({"sha": sha('b'), "commit": {"message": "Big", "author": {}}, "files": [
            {"filename": "big.txt", "status": "added", "additions": 1, "deletions": 0, "patch": "+".repeat(200_000)}]}),
    )
    .await;
    let item = &e.fetch("github_commit_get", &json!({"repo": "octo/cat", "ref": "feature/x"})).await.unwrap()[0];
    assert_eq!(item.body.as_ref().unwrap().chars().count(), 60_000);
    assert_eq!(item.extra["truncated"], true);
}

#[tokio::test]
async fn two_refs_are_compared() {
    let e = env().await;
    e.on(
        "GET",
        "/repos/octo/cat/compare/main...feature/x",
        200,
        &json!({"status": "ahead", "ahead_by": 2, "behind_by": 0, "total_commits": 2, "html_url": "https://gh/cmp",
            "merge_base_commit": {"sha": sha('m')},
            "commits": [{"sha": sha('1'), "commit": {"message": "One\nmore"}}, {"sha": sha('2'), "commit": {"message": "Two"}}],
            "files": [{"filename": "a.rs", "status": "added", "additions": 5, "deletions": 0, "patch": "+fn a() {}"}]}),
    )
    .await;
    let item = &e
        .fetch("github_commit_compare", &json!({"repo": "octo/cat", "base": "main", "head": "feature/x"}))
        .await
        .unwrap()[0];
    let body = item.body.as_deref().unwrap();
    assert!(body.starts_with("feature/x is 2 ahead of and 0 behind main (status: ahead)."), "{body}");
    assert!(
        body.contains("1111111 One\n2222222 Two\n")
            && body.contains("A a.rs (+5 -0)")
            && body.contains("--- a.rs\n+fn a() {}")
    );
    assert_eq!(
        (item.extra["ahead_by"].as_i64(), item.extra["merge_base"].as_str()),
        (Some(2), Some(sha('m').as_str()))
    );
    assert_eq!(item.resource, "octo/cat", "two refs: the whole repository");
    let seen = e.seen().await;
    assert_eq!(seen[0].2, "per_page=100");
    assert!(e.fetch("github_commit_compare", &json!({"repo": "octo/cat", "base": "a b", "head": "x"})).await.is_err());
}

#[tokio::test]
async fn statuses_and_check_runs_are_summarised() {
    let e = env().await;
    e.on("GET", "/repos/octo/cat/git/ref/heads/main", 200, &json!({"object": {"sha": sha('1')}})).await;
    e.on(
        "GET",
        "/repos/octo/cat/commits/main/status",
        200,
        &json!({"state": "failure", "sha": sha('1'), "statuses": [
            {"context": "ci/legacy", "state": "success", "description": "ok", "target_url": "https://ci/1"},
            {"context": "ci/lint", "state": "failure", "description": "3 errors", "target_url": "https://ci/2"}]}),
    )
    .await;
    e.on(
        "GET",
        "/repos/octo/cat/commits/main/check-runs",
        200,
        &json!({"total_count": 3, "check_runs": [
            {"name": "build", "status": "completed", "conclusion": "success", "html_url": "https://gh/r/1"},
            {"name": "test", "status": "in_progress", "conclusion": null, "html_url": "https://gh/r/2"},
            {"name": "deploy", "status": "completed", "conclusion": "cancelled", "html_url": "https://gh/r/3"}]}),
    )
    .await;
    let item = &e.fetch("github_checks_get", &json!({"repo": "octo/cat", "ref": "main"})).await.unwrap()[0];
    assert_eq!(item.snippet, "2 passed, 2 failed, 1 pending");
    assert_eq!(item.resource, "octo/cat@main");
    let body = item.body.as_deref().unwrap();
    assert!(
        body.contains("status ci/lint: failure 3 errors")
            && body.contains("check test: in_progress")
            && body.contains("check deploy: cancelled"),
        "{body}"
    );
    assert_eq!(item.extra["state"], "failure");
    assert_eq!(
        item.extra["check_runs"][1],
        json!({"name": "test", "status": "in_progress", "conclusion": null, "url": "https://gh/r/2"})
    );
}

// ---- tags -----------------------------------------------------------------------------------------------------------

#[tokio::test]
async fn tags_are_listed_and_read_lightweight_or_annotated() {
    let e = env().await;
    e.on(
        "GET",
        "/repos/octo/cat/tags",
        200,
        &json!([{"name": "v2", "commit": {"sha": sha('2')}}, {"name": "v1", "commit": {"sha": sha('1')}}]),
    )
    .await;
    let items = e.fetch("github_tag_list", &json!({"repo": "octo/cat", "limit": 7})).await.unwrap();
    assert_eq!(
        items.iter().map(|i| (i.id.as_str(), i.snippet.as_str())).collect::<Vec<_>>(),
        [("v2", "commit 2222222"), ("v1", "commit 1111111")]
    );
    assert_eq!(e.seen().await[0].2, "per_page=7");

    e.on("GET", "/repos/octo/cat/git/ref/tags/v1", 200, &json!({"object": {"type": "commit", "sha": sha('1')}})).await;
    let item = &e.fetch("github_tag_get", &json!({"repo": "octo/cat", "tag": "v1"})).await.unwrap()[0];
    assert_eq!(
        (item.extra["annotated"].as_bool(), item.extra["commit_sha"].as_str()),
        (Some(false), Some(sha('1').as_str()))
    );

    e.on("GET", "/repos/octo/cat/git/ref/tags/v2", 200, &json!({"object": {"type": "tag", "sha": sha('t')}})).await;
    e.on(
        "GET",
        &format!("/repos/octo/cat/git/tags/{}", sha('t')),
        200,
        &json!({"message": "Release two", "tagger": {"name": "Ann", "email": "a@x.io", "date": "2026-10-05T14:30:00Z"}, "object": {"type": "commit", "sha": sha('2')}}),
    )
    .await;
    let item = &e.fetch("github_tag_get", &json!({"repo": "octo/cat", "tag": "v2"})).await.unwrap()[0];
    assert_eq!(item.body.as_deref(), Some("Release two"));
    assert_eq!(
        (item.extra["annotated"].as_bool(), item.extra["commit_sha"].as_str(), item.extra["tag_sha"].as_str()),
        (Some(true), Some(sha('2').as_str()), Some(sha('t').as_str()))
    );
    assert_eq!(item.extra["tagger"]["name"], "Ann");
    assert!(e.fetch("github_tag_get", &json!({"repo": "octo/cat", "tag": "v1..2"})).await.is_err());
}

#[tokio::test]
async fn a_lightweight_tag_is_created_at_the_default_branch_head_by_default() {
    let e = env().await;
    e.repo(&[]).await;
    Mock::given(method("GET"))
        .and(path("/repos/octo/cat/commits/main"))
        .and(header("accept", "application/vnd.github.sha"))
        .respond_with(ResponseTemplate::new(200).set_body_string(sha('c')))
        .mount(&e.server)
        .await;
    e.on("GET", "/repos/octo/cat/git/ref/tags/v3", 404, &json!({"message": "Not Found"})).await;
    e.on("GET", &format!("/repos/octo/cat/git/commits/{}", sha('c')), 200, &json!({"message": "Ship it\n\nbody"}))
        .await;
    let args = json!({"repo": "octo/cat", "tag": "v3"});
    let p = e.preview("github_tag_create", &args).await.unwrap();
    assert_eq!((p.resource.as_str(), p.resource_label.as_str()), ("octo/cat", "octo/cat"));
    assert_eq!(p.parents, [("octo".to_owned(), "Every repository of octo".to_owned())]);
    assert!(!p.once_only);
    assert_eq!(p.lines[0], "Create the lightweight tag `v3` in octo/cat at commit ccccccc (Ship it)");
    Mock::given(method("POST"))
        .and(path("/repos/octo/cat/git/refs"))
        .and(body_json(json!({"ref": "refs/tags/v3", "sha": sha('c')})))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"ref": "refs/tags/v3"})))
        .expect(1)
        .mount(&e.server)
        .await;
    let done = e.perform("github_tag_create", &args).await.unwrap();
    assert_eq!(
        done,
        json!({"created": true, "tag": "v3", "ref": "refs/tags/v3", "commit_sha": sha('c'), "annotated": false})
    );
}

#[tokio::test]
async fn an_annotated_tag_makes_the_tag_object_first_and_an_existing_tag_is_never_moved() {
    let e = env().await;
    Mock::given(method("GET"))
        .and(path("/repos/octo/cat/commits/release"))
        .respond_with(ResponseTemplate::new(200).set_body_string(format!("{}\n", sha('d'))))
        .mount(&e.server)
        .await;
    e.on("GET", "/repos/octo/cat/git/ref/tags/v4", 404, &json!({})).await;
    e.on("GET", &format!("/repos/octo/cat/git/commits/{}", sha('d')), 200, &json!({"message": "Release commit"})).await;
    let args = json!({"repo": "octo/cat", "tag": "v4", "target": "release", "message": "Version 4", "tagger_name": "Ann", "tagger_email": "ann@x.io"});
    let p = e.preview("github_tag_create", &args).await.unwrap();
    let text = lines(&p);
    assert!(
        text.contains("annotated tag `v4`")
            && text.contains("Tag message:\nVersion 4")
            && text.contains("Tagger: Ann <ann@x.io>"),
        "{text}"
    );
    Mock::given(method("POST"))
        .and(path("/repos/octo/cat/git/tags"))
        .and(body_json(json!({"tag": "v4", "message": "Version 4", "object": sha('d'), "type": "commit",
            "tagger": {"name": "Ann", "email": "ann@x.io"}})))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"sha": sha('t')})))
        .expect(1)
        .mount(&e.server)
        .await;
    Mock::given(method("POST"))
        .and(path("/repos/octo/cat/git/refs"))
        .and(body_json(json!({"ref": "refs/tags/v4", "sha": sha('t')})))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"ref": "refs/tags/v4"})))
        .expect(1)
        .mount(&e.server)
        .await;
    let done = e.perform("github_tag_create", &args).await.unwrap();
    assert_eq!((done["annotated"].as_bool(), done["commit_sha"].as_str()), (Some(true), Some(sha('d').as_str())));

    e.on("GET", "/repos/octo/cat/git/ref/tags/old", 200, &json!({"object": {"sha": sha('1')}})).await;
    let err = e
        .preview("github_tag_create", &json!({"repo": "octo/cat", "tag": "old", "target": "release"}))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("already exists") && err.to_string().contains("never moved"), "{err}");
    let err = e
        .preview(
            "github_tag_create",
            &json!({"repo": "octo/cat", "tag": "v5", "tagger_name": "A", "tagger_email": "a@b.c"}),
        )
        .await
        .unwrap_err();
    assert!(err.to_string().contains("annotated"), "{err}");
    let err = e
        .preview("github_tag_create", &json!({"repo": "octo/cat", "tag": "v5", "message": "m", "tagger_name": "A"}))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("together"), "{err}");
    let err = e.preview("github_tag_create", &json!({"repo": "octo/cat", "tag": "-bad"})).await.unwrap_err();
    assert!(err.to_string().contains("`tag`"), "{err}");
    // A target that is not a commit.
    Mock::given(method("GET"))
        .and(path("/repos/octo/cat/commits/nothing"))
        .respond_with(ResponseTemplate::new(200).set_body_string("{}"))
        .mount(&e.server)
        .await;
    e.on("GET", "/repos/octo/cat/git/ref/tags/v6", 404, &json!({})).await;
    let err = e
        .preview("github_tag_create", &json!({"repo": "octo/cat", "tag": "v6", "target": "nothing"}))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("does not name a commit"), "{err}");
}

#[tokio::test]
async fn a_tag_is_deleted_after_saying_what_stays() {
    let e = env().await;
    e.on("GET", "/repos/octo/cat/git/ref/tags/v1", 200, &json!({"object": {"sha": sha('1')}})).await;
    e.on("GET", "/repos/octo/cat/releases/tags/v1", 200, &json!({"name": "First", "tag_name": "v1"})).await;
    let p = e.preview("github_tag_delete", &json!({"repo": "octo/cat", "tag": "v1"})).await.unwrap();
    assert_eq!(p.resource, "octo/cat");
    let text = lines(&p);
    assert!(
        text.contains("Delete the tag `v1` in octo/cat (it points to 1111111)")
            && text.contains("The release \"First\" was made from this tag; it is not deleted"),
        "{text}"
    );
    Mock::given(method("DELETE"))
        .and(path("/repos/octo/cat/git/refs/tags/v1"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&e.server)
        .await;
    assert_eq!(
        e.perform("github_tag_delete", &json!({"repo": "octo/cat", "tag": "v1"})).await.unwrap(),
        json!({"deleted": true, "tag": "v1"})
    );
    // Unknown tag: the preview fails, nothing is deleted.
    e.on("GET", "/repos/octo/cat/git/ref/tags/nope", 404, &json!({"message": "Not Found"})).await;
    assert!(
        e.preview("github_tag_delete", &json!({"repo": "octo/cat", "tag": "nope"}))
            .await
            .unwrap_err()
            .to_string()
            .contains("does not exist")
    );
}

// ---- releases -------------------------------------------------------------------------------------------------------

fn release(id: i64, tag: &str) -> Value {
    json!({"id": id, "tag_name": tag, "name": format!("Release {tag}"), "draft": false, "prerelease": false, "body": format!("Notes for {tag}"),
        "target_commitish": "main", "html_url": format!("https://github.com/octo/cat/releases/tag/{tag}"), "author": {"login": "ann"},
        "published_at": "2026-10-05T14:30:00Z",
        "assets": [{"id": 77, "name": "app.apk", "label": null, "size": 1234, "content_type": "application/zip", "download_count": 3,
            "state": "uploaded", "browser_download_url": "https://github.com/octo/cat/releases/download/v1/app.apk"}]})
}

#[tokio::test]
async fn releases_are_listed_and_read_by_id_latest_and_tag() {
    let e = env().await;
    e.on("GET", "/repos/octo/cat/releases", 200, &json!([release(5, "v2"), release(4, "v1")])).await;
    let items = e.fetch("github_release_list", &json!({"repo": "octo/cat"})).await.unwrap();
    assert_eq!(items[0].id, "5");
    assert_eq!(items[0].snippet, "v2 · published · 1 asset(s)");
    assert_eq!(items[0].body, None, "a list shows no notes");
    assert_eq!(items[0].resource, "octo/cat");
    e.on("GET", "/repos/octo/cat/releases/5", 200, &release(5, "v2")).await;
    e.on("GET", "/repos/octo/cat/releases/latest", 200, &release(5, "v2")).await;
    e.on("GET", "/repos/octo/cat/releases/tags/v2", 200, &release(5, "v2")).await;
    for (tool, args) in [
        ("github_release_get", json!({"repo": "octo/cat", "release_id": 5})),
        ("github_release_latest", json!({"repo": "octo/cat"})),
        ("github_release_by_tag", json!({"repo": "octo/cat", "tag": "v2"})),
    ] {
        let item = &e.fetch(tool, &args).await.unwrap()[0];
        assert_eq!(
            (item.title.as_str(), item.body.as_deref(), item.from.as_str()),
            ("Release v2", Some("Notes for v2"), "ann"),
            "{tool}"
        );
        assert_eq!(item.extra["tag_name"], "v2");
        assert_eq!(item.extra["assets"][0]["name"], "app.apk");
        assert_eq!(item.extra["assets"][0]["url"], "https://github.com/octo/cat/releases/download/v1/app.apk");
        assert_eq!(item.date, 1_791_210_600);
    }
    e.on("GET", "/repos/octo/cat/releases/9", 404, &json!({"message": "Not Found"})).await;
    assert!(
        e.fetch("github_release_get", &json!({"repo": "octo/cat", "release_id": 9}))
            .await
            .unwrap_err()
            .to_string()
            .contains("does not exist")
    );
}

#[tokio::test]
async fn release_notes_are_generated_without_creating_anything() {
    let e = env().await;
    Mock::given(method("POST"))
        .and(path("/repos/octo/cat/releases/generate-notes"))
        .and(body_json(json!({"tag_name": "v3", "target_commitish": "main", "previous_tag_name": "v2"})))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"name": "v3", "body": "## What's Changed\n* Fix by @ann"})),
        )
        .expect(1)
        .mount(&e.server)
        .await;
    let item = &e
        .fetch(
            "github_release_notes_generate",
            &json!({"repo": "octo/cat", "tag_name": "v3", "target_commitish": "main", "previous_tag_name": "v2"}),
        )
        .await
        .unwrap()[0];
    assert_eq!(item.body.as_deref(), Some("## What's Changed\n* Fix by @ann"));
    assert_eq!(item.title, "v3");
}

#[tokio::test]
async fn a_release_is_previewed_truthfully_then_created() {
    let e = env().await;
    e.repo(&[]).await;
    e.on("GET", "/repos/octo/cat/releases/tags/v3", 404, &json!({})).await;
    e.on("GET", "/repos/octo/cat/git/ref/tags/v3", 404, &json!({})).await;
    let args = json!({"repo": "octo/cat", "tag_name": "v3", "name": "Three", "body": "Notes here", "draft": true, "prerelease": true,
        "generate_release_notes": true, "make_latest": "false"});
    let p = e.preview("github_release_create", &args).await.unwrap();
    assert_eq!(p.resource, "octo/cat");
    let text = lines(&p);
    for needle in [
        "Create a draft release \"Three\" for tag `v3` in octo/cat (marked as a prerelease)",
        "The tag `v3` does not exist: GitHub creates it at main.",
        "It stays unpublished",
        "make_latest: false",
        "GitHub generates the release notes.",
        "Notes:\nNotes here",
    ] {
        assert!(text.contains(needle), "{needle} not in {text}");
    }
    Mock::given(method("POST"))
        .and(path("/repos/octo/cat/releases"))
        .and(body_json(
            json!({"tag_name": "v3", "name": "Three", "body": "Notes here", "draft": true, "prerelease": true,
            "generate_release_notes": true, "make_latest": "false"}),
        ))
        .respond_with(ResponseTemplate::new(201).set_body_json(release(12, "v3")))
        .expect(1)
        .mount(&e.server)
        .await;
    let done = e.perform("github_release_create", &args).await.unwrap();
    assert_eq!(
        (done["created"].as_bool(), done["id"].as_i64(), done["tag_name"].as_str()),
        (Some(true), Some(12), Some("v3"))
    );
    assert!(done["url"].as_str().unwrap().ends_with("/v3"));

    // An existing tag is used; an existing release for it is refused.
    e.on("GET", "/repos/octo/cat/git/ref/tags/v2", 200, &json!({"object": {"sha": sha('2')}})).await;
    e.on("GET", "/repos/octo/cat/releases/tags/v2", 404, &json!({})).await;
    let p = e.preview("github_release_create", &json!({"repo": "octo/cat", "tag_name": "v2"})).await.unwrap();
    assert!(
        lines(&p).contains("Publish release \"v2\" for tag `v2`")
            && lines(&p).contains("Uses the existing tag `v2` (commit 2222222)."),
        "{}",
        lines(&p)
    );
    e.on("GET", "/repos/octo/cat/releases/tags/v1", 200, &release(4, "v1")).await;
    let err = e.preview("github_release_create", &json!({"repo": "octo/cat", "tag_name": "v1"})).await.unwrap_err();
    assert!(err.to_string().contains("already exists") && err.to_string().contains("release_update"), "{err}");
}

#[tokio::test]
async fn a_release_update_shows_old_and_new_and_sends_only_what_changed() {
    let e = env().await;
    let mut draft = release(5, "v2");
    draft["draft"] = json!(true);
    e.on("GET", "/repos/octo/cat/releases/5", 200, &draft).await;
    let args = json!({"repo": "octo/cat", "release_id": 5, "name": "Two", "draft": false, "body": "New notes"});
    let p = e.preview("github_release_update", &args).await.unwrap();
    let text = lines(&p);
    for needle in [
        "Change the release \"Release v2\" (tag `v2`)",
        "name: 'Release v2' -> 'Two'",
        "draft: true -> false",
        "This publishes the release.",
        "Notes are replaced (12 -> 9 characters)",
    ] {
        assert!(text.contains(needle), "{needle} not in {text}");
    }
    Mock::given(method("PATCH"))
        .and(path("/repos/octo/cat/releases/5"))
        .and(body_json(json!({"name": "Two", "draft": false, "body": "New notes"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(release(5, "v2")))
        .expect(1)
        .mount(&e.server)
        .await;
    assert_eq!(e.perform("github_release_update", &args).await.unwrap()["updated"], true);
    let none = json!({"repo": "octo/cat", "release_id": 5});
    assert!(e.preview("github_release_update", &none).await.unwrap_err().to_string().contains("at least one"));
    assert!(e.perform("github_release_update", &none).await.unwrap_err().to_string().contains("at least one"));
}

#[tokio::test]
async fn a_release_is_deleted_but_not_its_tag() {
    let e = env().await;
    e.on("GET", "/repos/octo/cat/releases/5", 200, &release(5, "v2")).await;
    let p = e.preview("github_release_delete", &json!({"repo": "octo/cat", "release_id": 5})).await.unwrap();
    assert!(
        lines(&p).contains("Delete the published release \"Release v2\" (tag `v2`) of octo/cat with its 1 asset(s)")
            && lines(&p).contains("The tag itself is not deleted."),
        "{}",
        lines(&p)
    );
    Mock::given(method("DELETE"))
        .and(path("/repos/octo/cat/releases/5"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&e.server)
        .await;
    assert_eq!(
        e.perform("github_release_delete", &json!({"repo": "octo/cat", "release_id": 5})).await.unwrap(),
        json!({"deleted": true, "id": 5})
    );
    assert!(!e.seen_paths().await.iter().any(|p| p.contains("/git/refs")), "the tag is left alone");
}

// ---- assets ---------------------------------------------------------------------------------------------------------

#[tokio::test]
async fn assets_are_listed() {
    let e = env().await;
    e.on("GET", "/repos/octo/cat/releases/5/assets", 200, &json!([
        {"id": 77, "name": "app.apk", "label": "Android", "size": 1234, "content_type": "application/vnd.android.package-archive", "download_count": 3, "state": "uploaded",
         "browser_download_url": "https://github.com/x/app.apk", "updated_at": "2026-10-05T14:30:00Z"}])).await;
    let items = e.fetch("github_release_asset_list", &json!({"repo": "octo/cat", "release_id": 5})).await.unwrap();
    assert_eq!(items[0].id, "77");
    assert_eq!(items[0].title, "app.apk");
    assert_eq!(items[0].snippet, "1234 bytes · application/vnd.android.package-archive · 3 download(s)");
    assert_eq!(items[0].extra["url"], "https://github.com/x/app.apk");
    assert_eq!(items[0].extra["label"], "Android");
}

async fn asset_download_env(e: &Env, id: i64, size: usize, content: &[u8]) {
    let target = format!("{}/storage/asset{id}", e.server.uri());
    // The more specific mock is registered first: the octet-stream request is redirected.
    Mock::given(method("GET"))
        .and(path(format!("/repos/octo/cat/releases/assets/{id}")))
        .and(header("accept", "application/octet-stream"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", target.as_str()))
        .mount(&e.server)
        .await;
    e.on(
        "GET",
        &format!("/repos/octo/cat/releases/assets/{id}"),
        200,
        &json!({"id": id, "name": format!("file{id}"), "size": size}),
    )
    .await;
    Mock::given(method("GET"))
        .and(path(format!("/storage/asset{id}")))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(content.to_vec()))
        .mount(&e.server)
        .await;
}

#[tokio::test]
async fn an_asset_downloads_as_text_or_base64_and_a_big_one_only_as_metadata() {
    let e = env().await;
    let text_target = format!("{}/storage/asset1", e.server.uri());
    Mock::given(method("GET"))
        .and(path("/repos/octo/cat/releases/assets/1"))
        .and(header("accept", "application/octet-stream"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", text_target.as_str()))
        .mount(&e.server)
        .await;
    e.on(
        "GET",
        "/repos/octo/cat/releases/assets/1",
        200,
        &json!({"id": 1, "name": "notes.txt", "size": 11, "content_type": "text/plain"}),
    )
    .await;
    Mock::given(method("GET"))
        .and(path("/storage/asset1"))
        .and(|req: &wiremock::Request| !req.headers.contains_key("authorization"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"hello world".to_vec()))
        .expect(1)
        .mount(&e.server)
        .await;
    let item = &e.fetch("github_release_asset_download", &json!({"repo": "octo/cat", "asset_id": 1})).await.unwrap()[0];
    assert_eq!(
        (item.title.as_str(), item.body.as_deref(), item.extra["encoding"].as_str()),
        ("notes.txt", Some("hello world"), Some("text"))
    );
    assert_eq!(item.extra["size"], 11);

    let bin = [0u8, 159, 146, 150];
    let target = format!("{}/storage/asset2", e.server.uri());
    Mock::given(method("GET"))
        .and(path("/repos/octo/cat/releases/assets/2"))
        .and(header("accept", "application/octet-stream"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", target.as_str()))
        .mount(&e.server)
        .await;
    e.on("GET", "/repos/octo/cat/releases/assets/2", 200, &json!({"id": 2, "name": "a.bin", "size": 4})).await;
    Mock::given(method("GET"))
        .and(path("/storage/asset2"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(bin.to_vec()))
        .mount(&e.server)
        .await;
    let item = &e.fetch("github_release_asset_download", &json!({"repo": "octo/cat", "asset_id": 2})).await.unwrap()[0];
    assert_eq!(
        (item.body.as_deref(), item.extra["content_base64"].as_str()),
        (None, Some(BASE64.encode(&bin).as_str()))
    );

    // Over 2 MB by the listing: no download is even started.
    e.on("GET", "/repos/octo/cat/releases/assets/3", 200, &json!({"id": 3, "name": "big.iso", "size": 5_000_000}))
        .await;
    let item = &e.fetch("github_release_asset_download", &json!({"repo": "octo/cat", "asset_id": 3})).await.unwrap()[0];
    assert_eq!(
        (item.extra["too_large"].as_bool(), item.extra["encoding"].as_str(), item.body.as_deref()),
        (Some(true), Some("none"), None)
    );
    assert_eq!(item.extra["name"], "big.iso");

    // An answer that turns out bigger than announced is refused.
    asset_download_env(&e, 4, 10, &vec![b'x'; 2_500_000]).await;
    let err = e.fetch("github_release_asset_download", &json!({"repo": "octo/cat", "asset_id": 4})).await.unwrap_err();
    assert!(err.to_string().contains("larger than 2 MB"), "{err}");
}

fn upload_args(content: &[u8]) -> Value {
    json!({"repo": "octo/cat", "release_id": 5, "name": "app-1.0.zip", "content_base64": BASE64.encode(content),
        "label": "The app", "content_type": "application/zip"})
}

#[tokio::test]
async fn an_asset_is_uploaded_to_the_upload_host_as_raw_bytes() {
    let e = env().await;
    let mut rel = release(5, "v2");
    rel["draft"] = json!(false);
    e.on("GET", "/repos/octo/cat/releases/5", 200, &rel).await;
    let content = [1u8, 2, 3, 0, 255, 254];
    let args = upload_args(&content);
    let p = e.preview("github_release_asset_upload", &args).await.unwrap();
    let text = lines(&p);
    for needle in [
        "Attach `app-1.0.zip` to the release \"Release v2\" (tag `v2`) of octo/cat",
        "binary, 6 bytes, type application/zip",
        "Label: The app",
        "downloadable at once",
    ] {
        assert!(text.contains(needle), "{needle} not in {text}");
    }
    assert_eq!(p.resource, "octo/cat");
    Mock::given(method("POST"))
        .and(path("/repos/octo/cat/releases/5/assets"))
        .and(query_param("name", "app-1.0.zip"))
        .and(query_param("label", "The app"))
        .and(header("content-type", "application/zip"))
        .and(header("authorization", "Bearer ghp_secret"))
        .and(body_bytes(content.to_vec()))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"id": 88, "name": "app-1.0.zip", "size": 6, "state": "uploaded", "browser_download_url": "https://github.com/x/app-1.0.zip"})))
        .expect(1)
        .mount(&e.uploads)
        .await;
    let done = e.perform("github_release_asset_upload", &args).await.unwrap();
    assert_eq!(
        done,
        json!({"uploaded": true, "id": 88, "name": "app-1.0.zip", "size": 6, "state": "uploaded", "url": "https://github.com/x/app-1.0.zip"})
    );
    assert!(
        e.seen().await.iter().all(|(m, p, _, _)| !(m == "POST" && p.contains("assets"))),
        "nothing is uploaded to the API host"
    );
}

#[tokio::test]
async fn asset_uploads_are_validated_before_anything_is_sent() {
    let e = env().await;
    e.on("GET", "/repos/octo/cat/releases/5", 200, &release(5, "v2")).await;
    // The existing asset is called app.apk.
    let mut args = upload_args(b"x");
    args["name"] = json!("app.apk");
    let err = e.preview("github_release_asset_upload", &args).await.unwrap_err();
    assert!(err.to_string().contains("already has an asset named `app.apk`"), "{err}");
    let cases = [
        ("name", json!("dir/file.zip"), "`name`"),
        ("name", json!(".."), "`name`"),
        ("name", json!("a\\b"), "`name`"),
        ("content_base64", json!("%%%not base64"), "not valid base64"),
        ("content_base64", json!("\n"), "empty"),
        ("content_base64", json!(BASE64.encode(&vec![0u8; 2_000_001])), "larger than 2 MB"),
        ("content_type", json!("application/zip; x=1"), "`content_type`"),
        ("content_type", json!("zip"), "`content_type`"),
    ];
    for (key, value, expected) in cases {
        let mut args = upload_args(b"x");
        args[key] = value;
        for err in [
            e.preview("github_release_asset_upload", &args).await.unwrap_err(),
            e.perform("github_release_asset_upload", &args).await.unwrap_err(),
        ] {
            assert!(err.to_string().contains(expected), "{key}: {err}");
        }
    }
    let up: Vec<_> =
        e.uploads.received_requests().await.unwrap().into_iter().filter(|r| r.method.as_str() != "HEAD").collect();
    assert!(up.is_empty(), "{up:?}");
    // 2 MB exactly is accepted by validation (base64 of 2000000 bytes).
    let mut ok = upload_args(&vec![7u8; 2_000_000]);
    ok["name"] = json!("exactly-2mb.bin");
    assert!(e.preview("github_release_asset_upload", &ok).await.is_ok());
}

#[tokio::test]
async fn an_asset_is_renamed_and_deleted() {
    let e = env().await;
    e.on(
        "GET",
        "/repos/octo/cat/releases/assets/77",
        200,
        &json!({"id": 77, "name": "old.zip", "label": "Old", "size": 10, "download_count": 4}),
    )
    .await;
    let args = json!({"repo": "octo/cat", "asset_id": 77, "name": "new.zip", "label": ""});
    let p = e.preview("github_release_asset_update", &args).await.unwrap();
    assert!(
        lines(&p).contains("name: 'old.zip' -> 'new.zip'") && lines(&p).contains("label: 'Old' -> ''"),
        "{}",
        lines(&p)
    );
    Mock::given(method("PATCH"))
        .and(path("/repos/octo/cat/releases/assets/77"))
        .and(body_json(json!({"name": "new.zip", "label": ""})))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"id": 77, "name": "new.zip", "label": "", "browser_download_url": "https://gh/new.zip"}),
        ))
        .expect(1)
        .mount(&e.server)
        .await;
    assert_eq!(e.perform("github_release_asset_update", &args).await.unwrap()["name"], "new.zip");
    assert!(
        e.preview("github_release_asset_update", &json!({"repo": "octo/cat", "asset_id": 77}))
            .await
            .unwrap_err()
            .to_string()
            .contains("`name` and/or `label`")
    );
    assert!(
        e.preview("github_release_asset_update", &json!({"repo": "octo/cat", "asset_id": 77, "name": "a/b"}))
            .await
            .is_err()
    );

    let p = e.preview("github_release_asset_delete", &json!({"repo": "octo/cat", "asset_id": 77})).await.unwrap();
    assert_eq!(p.lines[0], "Delete the release asset `old.zip` (10 bytes, 4 download(s)) of octo/cat");
    Mock::given(method("DELETE"))
        .and(path("/repos/octo/cat/releases/assets/77"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&e.server)
        .await;
    assert_eq!(
        e.perform("github_release_asset_delete", &json!({"repo": "octo/cat", "asset_id": 77})).await.unwrap(),
        json!({"deleted": true, "id": 77})
    );
}

// ---- writing code: one file -------------------------------------------------------------------------------------

#[tokio::test]
async fn creating_a_file_on_a_feature_branch_is_previewed_and_committed() {
    let e = env().await;
    e.repo(&[("feature/login", &sha('1'))]).await;
    e.on("GET", "/repos/octo/cat/contents/docs/new.md", 404, &json!({"message": "Not Found"})).await;
    let args = json!({"repo": "octo/cat", "path": "docs/new.md", "content": "# Title\nBody\n", "message": "Add docs", "branch": "feature/login",
        "author_name": "Ann", "author_email": "ann@x.io"});
    let p = e.preview("github_file_put", &args).await.unwrap();
    assert_eq!(
        (p.resource.as_str(), p.resource_label.as_str()),
        ("octo/cat@feature/login", "octo/cat, branch feature/login")
    );
    assert_eq!(
        p.parents,
        [
            ("octo/cat".to_owned(), "Any branch of octo/cat".to_owned()),
            ("octo".to_owned(), "Every repository of octo".to_owned())
        ]
    );
    assert_eq!(p.lines[0], "Create the file `docs/new.md` on branch feature/login of octo/cat");
    let text = lines(&p);
    for needle in [
        "Advances feature/login from commit 1111111.",
        "Commit message:\nAdd docs",
        "New content: text, 13 bytes, 2 lines",
        "Starts with:\n# Title\nBody",
        "Author: Ann <ann@x.io>",
    ] {
        assert!(text.contains(needle), "{needle} not in {text}");
    }
    assert!(!text.contains("default branch"), "{text}");
    assert!(!p.once_only);
    Mock::given(method("PUT"))
        .and(path("/repos/octo/cat/contents/docs/new.md"))
        .and(body_json(json!({"message": "Add docs", "content": b64("# Title\nBody\n"), "branch": "feature/login", "author": {"name": "Ann", "email": "ann@x.io"}})))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"content": {"sha": sha('b')}, "commit": {"sha": sha('c'), "html_url": "https://gh/c"}})))
        .expect(1)
        .mount(&e.server)
        .await;
    let done = e.perform("github_file_put", &args).await.unwrap();
    assert_eq!(
        done,
        json!({"committed": true, "created": true, "path": "docs/new.md", "branch": "feature/login", "commit_sha": sha('c'), "content_sha": sha('b'), "url": "https://gh/c"})
    );
}

#[tokio::test]
async fn editing_a_workflow_on_the_default_branch_says_it_pushes_to_the_default_branch() {
    let e = env().await;
    e.repo(&[("main", &sha('1'))]).await;
    e.on(
        "GET",
        "/repos/octo/cat/contents/.github/workflows/ci.yml",
        200,
        &json!({"type": "file", "sha": sha('a'), "size": 40}),
    )
    .await;
    let args = json!({"repo": "octo/cat", "path": ".github/workflows/ci.yml", "content": "name: ci\non: push\n", "message": "Update CI"});
    let p = e.preview("github_file_put", &args).await.unwrap();
    assert_eq!(p.resource, "octo/cat@main");
    assert_eq!(p.lines[0], "Replace the file `.github/workflows/ci.yml` on main, the default branch of octo/cat");
    let text = lines(&p);
    assert!(
        text.contains("This pushes directly to the default branch main.") && text.contains("(was 40 bytes)"),
        "{text}"
    );
    // Without a sha the current one is fetched and sent.
    Mock::given(method("PUT"))
        .and(path("/repos/octo/cat/contents/.github/workflows/ci.yml"))
        .and(body_json(
            json!({"message": "Update CI", "content": b64("name: ci\non: push\n"), "branch": "main", "sha": sha('a')}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"content": {"sha": sha('b')}, "commit": {"sha": sha('c'), "html_url": "https://gh/c"}}),
        ))
        .expect(1)
        .mount(&e.server)
        .await;
    let done = e.perform("github_file_put", &args).await.unwrap();
    assert_eq!((done["created"].as_bool(), done["branch"].as_str()), (Some(false), Some("main")));
    // The content lookup asked for the right branch.
    let seen = e.seen().await;
    assert!(seen.iter().any(|(m, p, q, _)| m == "GET" && p.ends_with("ci.yml") && q == "ref=main"));
}

#[tokio::test]
async fn file_put_refuses_what_would_do_nothing_or_clobber_a_newer_version() {
    let e = env().await;
    e.repo(&[("main", &sha('1'))]).await;
    // git blob sha of "hello\n"
    let same = "ce013625030ba8dba906f756967f9e9ca394464a";
    e.on("GET", "/repos/octo/cat/contents/a.txt", 200, &json!({"type": "file", "sha": same, "size": 6})).await;
    let err = e
        .preview("github_file_put", &json!({"repo": "octo/cat", "path": "a.txt", "content": "hello\n", "message": "m"}))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("nothing to commit"), "{err}");
    let err = e
        .preview(
            "github_file_put",
            &json!({"repo": "octo/cat", "path": "a.txt", "content": "changed", "message": "m", "sha": sha('f')}),
        )
        .await
        .unwrap_err();
    assert!(err.to_string().contains("has changed") && err.to_string().contains(same), "{err}");
    let err = e
        .preview(
            "github_file_put",
            &json!({"repo": "octo/cat", "path": "gone.txt", "content": "x", "message": "m", "sha": sha('f')}),
        )
        .await
        .unwrap_err();
    assert!(err.to_string().contains("does not exist"), "{err}");
    e.on("GET", "/repos/octo/cat/contents/dir", 200, &json!([])).await;
    let err = e
        .preview("github_file_put", &json!({"repo": "octo/cat", "path": "dir", "content": "x", "message": "m"}))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("directory"), "{err}");
    // A branch that does not exist is not created by this tool.
    e.on("GET", "/repos/octo/cat/git/ref/heads/ghost", 404, &json!({})).await;
    let err = e
        .preview(
            "github_file_put",
            &json!({"repo": "octo/cat", "path": "a.txt", "content": "x", "message": "m", "branch": "ghost"}),
        )
        .await
        .unwrap_err();
    assert!(err.to_string().contains("never creates branches") && err.to_string().contains("base_branch"), "{err}");
}

#[tokio::test]
async fn file_put_validates_content_paths_and_authors() {
    let e = env().await;
    let base = json!({"repo": "octo/cat", "path": "a.txt", "message": "m"});
    let with = |pairs: &[(&str, Value)]| {
        let mut a = base.clone();
        for (k, v) in pairs {
            a[*k] = v.clone();
        }
        a
    };
    let cases = [
        (base.clone(), "content"),
        (with(&[("content", json!("x")), ("content_base64", json!("eA=="))]), "not both"),
        (with(&[("content_base64", json!("!!!"))]), "not valid base64"),
        (with(&[("content_base64", json!(BASE64.encode(&vec![1u8; 2_000_001])))]), "larger than 2 MB"),
        (with(&[("content", json!("x".repeat(2_000_001)))]), "at most 2000000"),
        (with(&[("content", json!("x")), ("path", json!("../etc/passwd"))]), "not a valid file path"),
        (with(&[("content", json!("x")), ("author_name", json!("Ann"))]), "together"),
        (
            with(&[("content", json!("x")), ("author_name", json!("Ann")), ("author_email", json!("not-an-email"))]),
            "plain name",
        ),
        (with(&[("content", json!("x")), ("branch", json!("bad..branch"))]), "`branch`"),
        (with(&[("content", json!("x")), ("sha", json!("abc123"))]), "full git sha"),
        (with(&[("content", json!("x")), ("message", json!("   "))]), "message"),
    ];
    for (args, expected) in cases {
        let msg = match spec_for_tool("github_file_put").unwrap().parse(&args) {
            Ok((c, _)) => e.gh.preview(ACCOUNT, &c).await.map(|_| ()).unwrap_err().to_string(),
            Err(msg) => msg,
        };
        assert!(msg.contains(expected), "{args}: {msg}");
    }
    let seen = e.seen().await;
    assert!(seen.is_empty(), "every case failed before the network: {seen:?}");
}

#[tokio::test]
async fn a_binary_file_is_committed_from_base64_and_previewed_by_size() {
    let e = env().await;
    e.repo(&[("main", &sha('1'))]).await;
    e.on("GET", "/repos/octo/cat/contents/img/logo.png", 404, &json!({})).await;
    let bytes = [0x89u8, b'P', b'N', b'G', 0, 1, 2];
    let args =
        json!({"repo": "octo/cat", "path": "img/logo.png", "content_base64": BASE64.encode(&bytes), "message": "logo"});
    let p = e.preview("github_file_put", &args).await.unwrap();
    assert!(lines(&p).contains("New content: binary, 7 bytes") && !lines(&p).contains("Starts with"), "{}", lines(&p));
    Mock::given(method("PUT"))
        .and(path("/repos/octo/cat/contents/img/logo.png"))
        .and(body_json(json!({"message": "logo", "content": BASE64.encode(&bytes), "branch": "main"})))
        .respond_with(
            ResponseTemplate::new(201)
                .set_body_json(json!({"content": {"sha": sha('b')}, "commit": {"sha": sha('c')}})),
        )
        .expect(1)
        .mount(&e.server)
        .await;
    assert_eq!(e.perform("github_file_put", &args).await.unwrap()["created"], true);
}

#[tokio::test]
async fn deleting_a_file_needs_it_to_exist_and_sends_its_sha() {
    let e = env().await;
    e.repo(&[("dev", &sha('1'))]).await;
    e.on("GET", "/repos/octo/cat/contents/old.txt", 200, &json!({"type": "file", "sha": sha('a'), "size": 99})).await;
    e.on("GET", "/repos/octo/cat/contents/nope.txt", 404, &json!({})).await;
    let args = json!({"repo": "octo/cat", "path": "old.txt", "message": "Remove old", "branch": "dev"});
    let p = e.preview("github_file_delete", &args).await.unwrap();
    assert_eq!(p.resource, "octo/cat@dev");
    assert_eq!(p.lines[0], "Delete the file `old.txt` (99 bytes) on branch dev of octo/cat");
    assert!(
        lines(&p).contains("Commit message:\nRemove old") && lines(&p).contains("Advances dev from commit 1111111.")
    );
    Mock::given(method("DELETE"))
        .and(path("/repos/octo/cat/contents/old.txt"))
        .and(body_json(json!({"message": "Remove old", "sha": sha('a'), "branch": "dev"})))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"commit": {"sha": sha('c'), "html_url": "https://gh/c"}})),
        )
        .expect(1)
        .mount(&e.server)
        .await;
    let done = e.perform("github_file_delete", &args).await.unwrap();
    assert_eq!((done["deleted"].as_bool(), done["commit_sha"].as_str()), (Some(true), Some(sha('c').as_str())));
    let missing = json!({"repo": "octo/cat", "path": "nope.txt", "message": "x", "branch": "dev"});
    assert!(e.preview("github_file_delete", &missing).await.unwrap_err().to_string().contains("nothing to delete"));
    assert!(e.perform("github_file_delete", &missing).await.unwrap_err().to_string().contains("nothing to delete"));
    let stale = json!({"repo": "octo/cat", "path": "old.txt", "message": "x", "branch": "dev", "sha": sha('f')});
    assert!(e.preview("github_file_delete", &stale).await.unwrap_err().to_string().contains("has changed"));
    // On the default branch, and with the file given by sha, perform needs no lookup.
    e.on("GET", "/repos/octo/cat/git/ref/heads/main", 200, &json!({"object": {"sha": sha('2')}})).await;
    let p =
        e.preview("github_file_delete", &json!({"repo": "octo/cat", "path": "old.txt", "message": "x"})).await.unwrap();
    assert!(lines(&p).contains("This pushes directly to the default branch main."));
}

// ---- writing code: several files --------------------------------------------------------------------------------

async fn commit_flow_mocks(e: &Env, tree_entries: Value) {
    e.on("GET", &format!("/repos/octo/cat/git/commits/{}", sha('1')), 200, &json!({"tree": {"sha": sha('7')}})).await;
    e.on(
        "GET",
        &format!("/repos/octo/cat/git/trees/{}", sha('1')),
        200,
        &json!({"truncated": false, "tree": tree_entries}),
    )
    .await;
}

fn files_args(branch: Option<&str>) -> Value {
    let mut args = json!({"repo": "octo/cat", "message": "Rework\n\nMany files", "files": [
        {"path": "src/new.rs", "content": "fn new() {}\n"},
        {"path": "src/lib.rs", "content_base64": BASE64.encode(b"pub mod new;\n"), "mode": "100755"},
        {"path": "old/gone.txt", "delete": true},
        {"path": "same.txt", "content": "hello\n"}]});
    if let Some(b) = branch {
        args["branch"] = json!(b);
    }
    args
}

fn tree_entries() -> Value {
    json!([
        {"path": "src", "type": "tree"},
        {"path": "src/lib.rs", "type": "blob", "sha": sha('a'), "size": 50},
        {"path": "old/gone.txt", "type": "blob", "sha": sha('b'), "size": 8},
        {"path": "same.txt", "type": "blob", "sha": "ce013625030ba8dba906f756967f9e9ca394464a", "size": 6}])
}

#[tokio::test]
async fn a_multi_file_commit_lists_every_path_with_its_action() {
    let e = env().await;
    e.repo(&[("dev", &sha('1'))]).await;
    commit_flow_mocks(&e, tree_entries()).await;
    let p = e.preview("github_commit_files", &files_args(Some("dev"))).await.unwrap();
    assert_eq!(p.resource, "octo/cat@dev");
    assert_eq!(p.lines[0], "Commit 4 files on branch dev of octo/cat: 1 new, 1 changed, 1 deleted, 1 unchanged");
    let text = lines(&p);
    for needle in [
        "Advances dev from commit 1111111.",
        "Commit message:\nRework\n\nMany files",
        "Files:",
        "  create src/new.rs (text, 12 bytes, 1 line)\nfn new() {}",
        "  update src/lib.rs (text, 13 bytes, 1 line, executable)\npub mod new;",
        "  delete old/gone.txt (8 bytes)",
        "  unchanged same.txt (text, 6 bytes, 1 line)",
    ] {
        assert!(text.contains(needle), "{needle} not in\n{text}");
    }
    assert!(!text.contains("default branch"), "{text}");
    let seen = e.seen().await;
    assert!(seen.iter().any(|(_, p, q, _)| p.ends_with(&format!("/git/trees/{}", sha('1'))) && q == "recursive=1"));
    assert!(seen.iter().all(|(m, _, _, _)| m == "GET"), "a preview changes nothing");
}

#[tokio::test]
async fn a_multi_file_commit_goes_through_blobs_tree_commit_and_a_plain_ref_update() {
    let e = env().await;
    commit_flow_mocks(&e, tree_entries()).await;
    e.on("GET", "/repos/octo/cat/git/ref/heads/dev", 200, &json!({"object": {"sha": sha('1')}})).await;
    for (content, blob) in [(b64("fn new() {}\n"), 'n'), (b64("pub mod new;\n"), 'l'), (b64("hello\n"), 's')] {
        Mock::given(method("POST"))
            .and(path("/repos/octo/cat/git/blobs"))
            .and(body_json(json!({"content": content, "encoding": "base64"})))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({"sha": sha(blob)})))
            .expect(1)
            .mount(&e.server)
            .await;
    }
    Mock::given(method("POST"))
        .and(path("/repos/octo/cat/git/trees"))
        .and(body_json(json!({"base_tree": sha('7'), "tree": [
            {"path": "src/new.rs", "mode": "100644", "type": "blob", "sha": sha('n')},
            {"path": "src/lib.rs", "mode": "100755", "type": "blob", "sha": sha('l')},
            {"path": "old/gone.txt", "mode": "100644", "type": "blob", "sha": null},
            {"path": "same.txt", "mode": "100644", "type": "blob", "sha": sha('s')}]})))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"sha": sha('8')})))
        .expect(1)
        .mount(&e.server)
        .await;
    Mock::given(method("POST"))
        .and(path("/repos/octo/cat/git/commits"))
        .and(body_json(json!({"message": "Rework\n\nMany files", "tree": sha('8'), "parents": [sha('1')], "author": {"name": "Ann", "email": "ann@x.io"}})))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"sha": sha('c'), "html_url": "https://gh/commit"})))
        .expect(1)
        .mount(&e.server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/repos/octo/cat/git/refs/heads/dev"))
        .and(body_json(json!({"sha": sha('c'), "force": false})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(1)
        .mount(&e.server)
        .await;
    let mut args = files_args(Some("dev"));
    args["author_name"] = json!("Ann");
    args["author_email"] = json!("ann@x.io");
    let done = e.perform("github_commit_files", &args).await.unwrap();
    assert_eq!(
        done,
        json!({"committed": true, "branch": "dev", "created_branch": false, "commit_sha": sha('c'), "files": 4, "url": "https://gh/commit"})
    );
    let order: Vec<String> = e.seen_paths().await;
    let pos = |needle: &str| {
        order.iter().position(|p| p.contains(needle)).unwrap_or_else(|| panic!("{needle} missing in {order:?}"))
    };
    assert!(
        pos("GET /repos/octo/cat/git/ref/heads/dev") < pos("git/commits/")
            && pos("git/blobs") < pos("POST /repos/octo/cat/git/trees")
    );
    assert!(
        pos("POST /repos/octo/cat/git/trees") < pos("POST /repos/octo/cat/git/commits")
            && pos("POST /repos/octo/cat/git/commits") < pos("PATCH /repos/octo/cat/git/refs/heads/dev")
    );
    assert_eq!(order.last().unwrap(), "PATCH /repos/octo/cat/git/refs/heads/dev", "the branch moves last");
    assert!(
        order.iter().all(|p| !p.starts_with("DELETE")),
        "nothing is deleted through the API: the tree drops the file"
    );
    let force_seen = e.seen().await.iter().any(|(_, _, _, b)| b.get("force") == Some(&json!(true)));
    assert!(!force_seen, "never forced");
}

#[tokio::test]
async fn a_missing_branch_is_created_from_the_base_branch_with_the_commit_on_top() {
    let e = env().await;
    e.repo(&[("main", &sha('1'))]).await;
    e.on("GET", "/repos/octo/cat/git/ref/heads/topic/new", 404, &json!({"message": "Not Found"})).await;
    commit_flow_mocks(&e, tree_entries()).await;
    let mut args = files_args(Some("topic/new"));
    args["base_branch"] = json!("main");
    let p = e.preview("github_commit_files", &args).await.unwrap();
    assert_eq!(p.resource, "octo/cat@topic/new");
    let text = lines(&p);
    assert!(
        text.contains("Creates the new branch topic/new from main (commit 1111111) with this commit on top."),
        "{text}"
    );
    assert!(!text.contains("default branch"), "{text}");

    Mock::given(method("POST"))
        .and(path("/repos/octo/cat/git/blobs"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"sha": sha('b')})))
        .mount(&e.server)
        .await;
    e.on("POST", "/repos/octo/cat/git/trees", 201, &json!({"sha": sha('8')})).await;
    Mock::given(method("POST"))
        .and(path("/repos/octo/cat/git/commits"))
        .and(body_json(json!({"message": "Rework\n\nMany files", "tree": sha('8'), "parents": [sha('1')]})))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"sha": sha('c')})))
        .expect(1)
        .mount(&e.server)
        .await;
    Mock::given(method("POST"))
        .and(path("/repos/octo/cat/git/refs"))
        .and(body_json(json!({"ref": "refs/heads/topic/new", "sha": sha('c')})))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({})))
        .expect(1)
        .mount(&e.server)
        .await;
    let done = e.perform("github_commit_files", &args).await.unwrap();
    assert_eq!((done["created_branch"].as_bool(), done["branch"].as_str()), (Some(true), Some("topic/new")));
    assert!(e.seen_paths().await.iter().all(|p| !p.starts_with("PATCH")), "a new branch is created, not updated");

    // Without base_branch a missing branch is an error, and so is a missing base.
    let err = e.preview("github_commit_files", &files_args(Some("topic/new"))).await.unwrap_err();
    assert!(err.to_string().contains("base_branch"), "{err}");
    e.on("GET", "/repos/octo/cat/git/ref/heads/nobase", 404, &json!({})).await;
    let mut args = files_args(Some("topic/new"));
    args["base_branch"] = json!("nobase");
    assert!(
        e.preview("github_commit_files", &args)
            .await
            .unwrap_err()
            .to_string()
            .contains("`base_branch` `nobase` does not exist")
    );
}

#[tokio::test]
async fn a_commit_to_the_default_branch_is_flagged_and_a_failed_ref_update_is_an_error() {
    let e = env().await;
    e.repo(&[("main", &sha('1'))]).await;
    commit_flow_mocks(&e, tree_entries()).await;
    let p = e.preview("github_commit_files", &files_args(None)).await.unwrap();
    assert_eq!(p.resource, "octo/cat@main");
    assert!(p.lines[0].contains("on main, the default branch of octo/cat"), "{}", p.lines[0]);
    assert!(lines(&p).contains("This pushes directly to the default branch main."));

    Mock::given(method("POST"))
        .and(path("/repos/octo/cat/git/blobs"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"sha": sha('b')})))
        .mount(&e.server)
        .await;
    e.on("POST", "/repos/octo/cat/git/trees", 201, &json!({"sha": sha('8')})).await;
    e.on("POST", "/repos/octo/cat/git/commits", 201, &json!({"sha": sha('c')})).await;
    e.on("PATCH", "/repos/octo/cat/git/refs/heads/main", 422, &json!({"message": "Update is not a fast forward"}))
        .await;
    let err = e.perform("github_commit_files", &files_args(None)).await.unwrap_err();
    assert!(err.to_string().contains("not a fast forward"), "{err}");
}

#[tokio::test]
async fn commit_files_rejects_bad_file_lists_before_touching_the_network() {
    let e = env().await;
    let ok = json!({"path": "a.txt", "content": "x"});
    let cases: Vec<(Value, &str)> = vec![
        (json!([]), "between 1 and 100"),
        (json!("a.txt"), "`files` must be a list"),
        (json!([1]), "must be an object"),
        (json!([{"path": "../a", "content": "x"}]), "not a valid file path"),
        (json!([{"content": "x"}]), "not a valid file path"),
        (json!([{"path": "a.txt"}]), "exactly one of"),
        (json!([{"path": "a.txt", "content": "x", "content_base64": "eA=="}]), "exactly one of"),
        (json!([{"path": "a.txt", "content": "x", "delete": true}]), "exactly one of"),
        (json!([{"path": "a.txt", "delete": false}]), "exactly one of"),
        (json!([{"path": "a.txt", "delete": "yes"}]), "true or false"),
        (json!([{"path": "a.txt", "content": "x", "mode": "120000"}]), "`mode`"),
        (json!([{"path": "a.txt", "delete": true, "mode": "100644"}]), "`mode`"),
        (json!([{"path": "a.txt", "contents": "x"}]), "unknown key `contents`"),
        (json!([{"path": "a.txt", "content_base64": "@@@"}]), "not valid base64"),
        (json!([ok.clone(), ok.clone()]), "clashes"),
        (json!([{"path": "a", "content": "x"}, {"path": "a/b", "content": "y"}]), "clashes"),
        (json!([{"path": "a/b", "content": "x"}, {"path": "a", "content": "y"}]), "clashes"),
        (json!([{"path": "big", "content": "x".repeat(2_000_001)}]), "larger than 2 MB"),
        (
            json!([{"path": "a", "content": "x".repeat(1_900_000)}, {"path": "b", "content": "x".repeat(1_900_000)}, {"path": "c", "content": "x".repeat(1_900_000)}]),
            "at most 4 MB",
        ),
        (
            Value::Array((0..101).map(|i| json!({"path": format!("f{i}"), "content": "x"})).collect()),
            "between 1 and 100",
        ),
    ];
    for (files, expected) in cases {
        let args = json!({"repo": "octo/cat", "message": "m", "files": files, "branch": "dev"});
        for err in [
            e.preview("github_commit_files", &args).await.unwrap_err(),
            e.perform("github_commit_files", &args).await.unwrap_err(),
        ] {
            assert!(err.to_string().contains(expected), "{files}: {err}");
        }
    }
    let seen = e.seen().await;
    assert!(seen.is_empty(), "{seen:?}");
}

#[tokio::test]
async fn commit_files_notices_deletions_of_missing_files_and_commits_that_change_nothing() {
    let e = env().await;
    e.repo(&[("dev", &sha('1'))]).await;
    commit_flow_mocks(&e, tree_entries()).await;
    let missing = json!({"repo": "octo/cat", "message": "m", "branch": "dev", "files": [{"path": "not/there.txt", "delete": true}]});
    let err = e.preview("github_commit_files", &missing).await.unwrap_err();
    assert!(err.to_string().contains("does not exist on dev"), "{err}");
    let nothing = json!({"repo": "octo/cat", "message": "m", "branch": "dev", "files": [{"path": "same.txt", "content": "hello\n"}]});
    let err = e.preview("github_commit_files", &nothing).await.unwrap_err();
    assert!(err.to_string().contains("nothing to commit"), "{err}");

    // When GitHub cuts the listing of a huge repository, "create" cannot be told from "update".
    e.on("GET", "/repos/octo/cat/git/ref/heads/huge", 200, &json!({"object": {"sha": sha('2')}})).await;
    e.on(
        "GET",
        "/repos/octo/cat/git/trees/2222222222222222222222222222222222222222",
        200,
        &json!({"truncated": true, "tree": []}),
    )
    .await;
    let args = json!({"repo": "octo/cat", "message": "m", "branch": "huge", "files": [{"path": "x.txt", "content": "y"}, {"path": "z.txt", "delete": true}]});
    let p = e.preview("github_commit_files", &args).await.unwrap();
    let text = lines(&p);
    assert!(
        text.contains("  write x.txt") && text.contains("  delete z.txt") && text.contains("too large to check"),
        "{text}"
    );
}

// ---- errors ---------------------------------------------------------------------------------------------------------

#[tokio::test]
async fn failures_are_explained_and_never_leak_the_token() {
    let e = env().await;
    e.on("GET", "/repos/octo/cat/releases/1", 404, &json!({"message": "Not Found"})).await;
    e.on(
        "GET",
        "/repos/octo/cat/releases/2",
        403,
        &json!({"message": "Resource not accessible by personal access token"}),
    )
    .await;
    e.on("GET", "/repos/octo/cat/releases/3", 401, &json!({"message": "Bad credentials"})).await;
    e.on("POST", "/repos/octo/cat/releases", 422, &json!({"message": "Validation Failed"})).await;
    let get = |id: i64| json!({"repo": "octo/cat", "release_id": id});
    let err = e.fetch("github_release_get", &get(1)).await.unwrap_err();
    assert!(err.to_string().contains("does not exist"), "{err}");
    let err = e.fetch("github_release_get", &get(2)).await.unwrap_err();
    assert!(err.to_string().contains("Resource not accessible"), "{err}");
    let err = e.fetch("github_release_get", &get(3)).await.unwrap_err();
    assert!(matches!(err, CoreError::ServiceNeedsAttention { .. }), "{err:?}");
    let err = e.perform("github_release_create", &json!({"repo": "octo/cat", "tag_name": "v9"})).await.unwrap_err();
    assert!(err.to_string().contains("did not accept") && err.to_string().contains("Validation Failed"), "{err}");
    for err in [
        e.fetch("github_release_get", &get(1)).await.unwrap_err(),
        e.fetch("github_release_get", &get(2)).await.unwrap_err(),
    ] {
        assert!(!err.to_string().contains("ghp_"), "{err}");
    }
    // A server error is retried, then reported.
    Mock::given(method("GET"))
        .and(path("/repos/octo/cat/releases/latest"))
        .respond_with(ResponseTemplate::new(503))
        .expect(3)
        .mount(&e.server)
        .await;
    assert!(e.fetch("github_release_latest", &json!({"repo": "octo/cat"})).await.is_err());
    // A failed write is an error, never a success.
    e.on("PUT", "/repos/octo/cat/contents/a.txt", 409, &json!({"message": "a.txt does not match abc"})).await;
    e.on("GET", "/repos/octo/cat/contents/a.txt", 404, &json!({})).await;
    let err = e
        .perform(
            "github_file_put",
            &json!({"repo": "octo/cat", "path": "a.txt", "content": "x", "message": "m", "branch": "main"}),
        )
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("cannot do that right now") && err.to_string().contains("does not match"),
        "{err}"
    );
    // Bad repos never reach the network.
    let before = e.seen().await.len();
    for repo in ["../../user", "a/b/c", "a b/c"] {
        assert!(e.fetch("github_release_latest", &json!({"repo": repo})).await.is_err());
    }
    assert_eq!(e.seen().await.len(), before);
}

#[tokio::test]
async fn every_operation_of_the_area_is_dispatched_and_writes_carry_their_class() {
    let e = env().await;
    let mine = [
        "file_get",
        "dir_list",
        "tree_get",
        "blob_get",
        "archive_link",
        "commit_list",
        "commit_get",
        "commit_compare",
        "checks_get",
        "tag_list",
        "tag_get",
        "release_list",
        "release_get",
        "release_latest",
        "release_by_tag",
        "release_notes_generate",
        "release_asset_list",
        "release_asset_download",
        "file_put",
        "file_delete",
        "commit_files",
        "tag_create",
        "tag_delete",
        "release_create",
        "release_update",
        "release_delete",
        "release_asset_upload",
        "release_asset_update",
        "release_asset_delete",
    ];
    let specs: Vec<_> =
        rewarden_proto::connector::specs().iter().filter(|s| s.service == "github" && mine.contains(&s.op)).collect();
    assert_eq!(specs.len(), mine.len(), "every tool is registered");
    for s in specs {
        let empty = ConnectorCall {
            service: "github".to_owned(),
            op: s.op.to_owned(),
            args: serde_json::Map::new(),
        };
        let err = match s.effect {
            rewarden_proto::connector::Effect::Write => {
                assert!(matches!(s.class, "code" | "releases"), "{}", s.tool);
                assert_eq!(
                    s.class == "code",
                    matches!(s.op, "file_put" | "file_delete" | "commit_files"),
                    "{}",
                    s.tool
                );
                assert!(!s.once_only);
                e.gh.preview(ACCOUNT, &empty).await.unwrap_err()
            }
            _ => e.gh.fetch(ACCOUNT, &empty).await.unwrap_err(),
        };
        assert!(!err.to_string().contains("GitHub cannot"), "{} is not handled: {err}", s.tool);
    }
}
