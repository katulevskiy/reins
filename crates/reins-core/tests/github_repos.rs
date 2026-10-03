//! GitHub `repos` area against a fake server: what each tool asks GitHub for (method, path, query, body), what it
//! makes of the answers, what a write previews before it is approved, and how failures are explained.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::FakeKeys;
use reins_core::CoreError;
use reins_core::connector::github::GitHub;
use reins_core::connector::{Connector, Item, Preview};
use reins_core::store::Store;
use reins_proto::connector::{ConnectorCall, Effect, spec_for_tool};
use serde_json::{Value, json};
use wiremock::matchers::{body_json, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const ME: &str = "octo-cat";

fn call(tool: &str, args: &Value) -> ConnectorCall {
    spec_for_tool(tool).unwrap().parse(args).unwrap().0
}

fn parse_err(tool: &str, args: &Value) -> String {
    spec_for_tool(tool).unwrap().parse(args).unwrap_err()
}

struct Env {
    server: MockServer,
    gh: GitHub,
    _dir: tempfile::TempDir,
}

async fn env() -> Env {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/user"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"login": "Octo-Cat"})))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(dir.path(), &FakeKeys).unwrap());
    let gh = GitHub::new(reins_core::http::client().unwrap(), &server.uri(), store, Duration::from_millis(1));
    assert_eq!(gh.sign_in("ghp_secret").await.unwrap(), ME);
    Env {
        server,
        gh,
        _dir: dir,
    }
}

impl Env {
    /// Answers `verb path` with `status` and `body`, any number of times.
    async fn on(&self, verb: &str, p: &str, status: u16, body: Value) {
        Mock::given(method(verb))
            .and(path(p))
            .respond_with(ResponseTemplate::new(status).set_body_json(body))
            .mount(&self.server)
            .await;
    }

    /// Answers `verb path` with an empty body.
    async fn on_empty(&self, verb: &str, p: &str, status: u16) {
        Mock::given(method(verb)).and(path(p)).respond_with(ResponseTemplate::new(status)).mount(&self.server).await;
    }

    /// Requires exactly one `verb path` with exactly this JSON body.
    async fn expect(&self, verb: &str, p: &str, sent: Value, status: u16, body: Value) {
        Mock::given(method(verb))
            .and(path(p))
            .and(body_json(sent))
            .respond_with(ResponseTemplate::new(status).set_body_json(body))
            .expect(1)
            .mount(&self.server)
            .await;
    }

    /// Requires exactly one `verb path` with no body worth checking.
    async fn expect_any(&self, verb: &str, p: &str, status: u16, body: Value) {
        Mock::given(method(verb))
            .and(path(p))
            .respond_with(ResponseTemplate::new(status).set_body_json(body))
            .expect(1)
            .mount(&self.server)
            .await;
    }

    async fn fetch(&self, tool: &str, args: &Value) -> Vec<Item> {
        self.gh.fetch(ME, &call(tool, args)).await.unwrap()
    }

    async fn fetch_err(&self, tool: &str, args: &Value) -> String {
        self.gh.fetch(ME, &call(tool, args)).await.unwrap_err().to_string()
    }

    async fn preview(&self, tool: &str, args: &Value) -> Preview {
        self.gh.preview(ME, &call(tool, args)).await.unwrap()
    }

    async fn preview_err(&self, tool: &str, args: &Value) -> String {
        self.gh.preview(ME, &call(tool, args)).await.unwrap_err().to_string()
    }

    async fn perform(&self, tool: &str, args: &Value) -> Value {
        self.gh.perform(ME, &call(tool, args)).await.unwrap()
    }

    async fn perform_err(&self, tool: &str, args: &Value) -> String {
        self.gh.perform(ME, &call(tool, args)).await.unwrap_err().to_string()
    }

    /// The requests that reached the fake server, apart from the sign-in.
    async fn requests(&self) -> Vec<(String, String)> {
        self.server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|r| r.url.path() != "/user")
            .map(|r| (r.method.to_string(), r.url.path().to_owned()))
            .collect()
    }
}

fn text_of(p: &Preview) -> String {
    p.lines.join("\n")
}

fn repo_json(name: &str, private: bool) -> Value {
    json!({
        "full_name": name, "private": private, "description": "A repo", "default_branch": "main", "pushed_at": "2026-10-01T00:00:00Z",
        "html_url": format!("https://github.com/{name}"), "stargazers_count": 3, "forks_count": 1, "open_issues_count": 2,
        "subscribers_count": 5, "size": 120, "topics": ["rust", "cli"], "license": {"spdx_id": "MIT"},
        "has_issues": true, "has_wiki": false, "has_projects": true, "delete_branch_on_merge": false,
        "allow_merge_commit": true, "allow_squash_merge": true, "allow_rebase_merge": false, "homepage": "",
        "archived": false, "fork": false,
    })
}

// ---- repositories: reads ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn the_legacy_repository_listing_still_works_and_takes_filters() {
    let e = env().await;
    e.on("GET", "/user/repos", 200, json!([repo_json("octo/cat", true), repo_json("octo/dog", false)])).await;
    let all = e.fetch("github_list_repos", &json!({"query": "CAT"})).await;
    assert_eq!(
        all.iter().map(|r| (r.id.as_str(), r.from.as_str(), r.snippet.as_str())).collect::<Vec<_>>(),
        [("octo/cat", "private", "A repo")]
    );
    assert_eq!(all[0].resource, "octo/cat");
    assert_eq!(all[0].parents, [("octo".to_owned(), "Every repository of octo".to_owned())]);
    assert_eq!(all[0].extra["default_branch"], "main");

    let public = e.fetch("github_list_repos", &json!({"visibility": "public"})).await;
    assert_eq!(public.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["octo/dog"]);
    assert_eq!(e.fetch("github_list_repos", &json!({"limit": 1})).await.len(), 1);
}

#[tokio::test]
async fn repositories_can_be_listed_for_an_organization_a_user_or_an_affiliation() {
    let e = env().await;
    Mock::given(method("GET"))
        .and(path("/orgs/acme/repos"))
        .and(query_param("type", "private"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([repo_json("acme/secret", true)])))
        .expect(1)
        .mount(&e.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/users/ann/repos"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([repo_json("ann/site", false)])))
        .expect(1)
        .mount(&e.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/user/repos"))
        .and(query_param("affiliation", "organization_member"))
        .and(query_param("visibility", "public"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([repo_json("acme/site", false)])))
        .expect(1)
        .mount(&e.server)
        .await;
    let org = e.fetch("github_list_repos", &json!({"org": "acme", "visibility": "private"})).await;
    assert_eq!(org[0].id, "acme/secret");
    assert_eq!(e.fetch("github_list_repos", &json!({"owner": "ann"})).await[0].id, "ann/site");
    let mine =
        e.fetch("github_list_repos", &json!({"affiliation": "organization_member", "visibility": "public"})).await;
    assert_eq!(mine[0].id, "acme/site");

    assert!(e.fetch_err("github_list_repos", &json!({"org": "a", "owner": "b"})).await.contains("not both"));
    assert!(e.fetch_err("github_list_repos", &json!({"org": "../x"})).await.contains("valid GitHub"));
    assert!(e.fetch_err("github_list_repos", &json!({"owner": "a/b"})).await.contains("valid GitHub"));
}

#[tokio::test]
async fn an_organizations_repositories_are_listed_and_filtered() {
    let e = env().await;
    Mock::given(method("GET"))
        .and(path("/orgs/acme/repos"))
        .and(query_param("type", "sources"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!([repo_json("acme/api", true), repo_json("acme/web", false)])),
        )
        .mount(&e.server)
        .await;
    let repos = e.fetch("github_org_repo_list", &json!({"org": "acme", "type": "sources", "query": "WEB"})).await;
    assert_eq!(repos.len(), 1);
    assert_eq!((repos[0].id.as_str(), repos[0].from.as_str()), ("acme/web", "public"));
    assert!(e.fetch_err("github_org_repo_list", &json!({"org": "a b"})).await.contains("valid GitHub"));
    assert!(parse_err("github_org_repo_list", &json!({})).contains("`org` is required"));
}

#[tokio::test]
async fn a_repository_is_read_with_its_details_and_languages() {
    let e = env().await;
    let mut fork = repo_json("octo/cat", false);
    fork["parent"] = json!({"full_name": "up/cat"});
    fork["source"] = json!({"full_name": "up/cat"});
    fork["fork"] = json!(true);
    e.on("GET", "/repos/octo/cat", 200, fork).await;
    e.on("GET", "/repos/octo/cat/languages", 200, json!({"Rust": 900, "Shell": 100})).await;
    let one = e.fetch("github_repo_get", &json!({"repo": "octo/cat"})).await;
    assert_eq!(one.len(), 1);
    let body = one[0].body.clone().unwrap();
    assert!(body.contains("octo/cat (public)") && body.contains("Default branch: main"), "{body}");
    assert!(
        body.contains("Topics: rust, cli") && body.contains("Fork of up/cat") && body.contains("License: MIT"),
        "{body}"
    );
    assert!(body.contains("Rust: 90.0% (900 bytes)") && body.contains("Shell: 10.0% (100 bytes)"), "{body}");
    assert_eq!(one[0].extra["topics"], json!(["rust", "cli"]));
    assert_eq!(one[0].extra["parent"], "up/cat");
    assert_eq!(one[0].extra["languages"]["Rust"], 900);
    assert_eq!(one[0].extra["features"]["wiki"], false);
    assert_eq!(one[0].resource, "octo/cat");
}

#[tokio::test]
async fn forks_languages_contributors_and_topics_are_read() {
    let e = env().await;
    Mock::given(method("GET"))
        .and(path("/repos/octo/cat/forks"))
        .and(query_param("sort", "stargazers"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([repo_json("ann/cat", false)])))
        .expect(1)
        .mount(&e.server)
        .await;
    assert_eq!(
        e.fetch("github_repo_forks_list", &json!({"repo": "octo/cat", "sort": "stargazers"})).await[0].id,
        "ann/cat"
    );

    e.on("GET", "/repos/octo/cat/languages", 200, json!({"Rust": 3, "C": 1})).await;
    let langs = e.fetch("github_repo_languages", &json!({"repo": "octo/cat"})).await;
    assert_eq!(langs[0].snippet, "2 languages");
    assert_eq!(langs[0].body.as_deref(), Some("Rust: 75.0% (3 bytes)\nC: 25.0% (1 bytes)"));

    e.on(
        "GET",
        "/repos/octo/cat/contributors",
        200,
        json!([
        {"login": "ann", "contributions": 40, "type": "User", "html_url": "https://github.com/ann"},
        {"login": "bot", "contributions": 2, "type": "Bot"}]),
    )
    .await;
    let people = e.fetch("github_repo_contributors", &json!({"repo": "octo/cat", "limit": 5})).await;
    assert_eq!(
        people.iter().map(|p| (p.id.as_str(), p.snippet.as_str(), p.from.as_str())).collect::<Vec<_>>(),
        [("ann", "40 commits", "User"), ("bot", "2 commits", "Bot")]
    );

    e.on("GET", "/repos/octo/cat/topics", 200, json!({"names": ["rust", "cli"]})).await;
    let topics = e.fetch("github_repo_topics_get", &json!({"repo": "octo/cat"})).await;
    assert_eq!(topics[0].extra["topics"], json!(["rust", "cli"]));
    assert_eq!(topics[0].body.as_deref(), Some("rust, cli"));

    e.on("GET", "/repos/octo/empty/contributors", 204, Value::Null).await;
    assert!(e.fetch("github_repo_contributors", &json!({"repo": "octo/empty"})).await.is_empty());
}

#[tokio::test]
async fn a_readme_is_decoded_or_fetched_raw_and_cut_when_huge() {
    let e = env().await;
    // GitHub wraps base64 with newlines.
    e.on(
        "GET",
        "/repos/octo/cat/readme",
        200,
        json!({
        "path": "README.md", "size": 13, "encoding": "base64", "content": "IyBDYXQKSGVs\nbG8gd29ybGQ=\n",
        "html_url": "https://github.com/octo/cat/blob/main/README.md"}),
    )
    .await;
    let readme = e.fetch("github_repo_readme_get", &json!({"repo": "octo/cat"})).await;
    assert_eq!(readme[0].body.as_deref(), Some("# Cat\nHello world"));
    assert_eq!(readme[0].extra["path"], "README.md");
    assert_eq!(readme[0].extra["truncated"], false);
    assert_eq!(readme[0].resource, "octo/cat");

    // A big README comes without content: the raw text is asked for.
    let big = "x".repeat(150_000);
    Mock::given(method("GET"))
        .and(path("/repos/octo/big/readme"))
        .and(wiremock::matchers::header("accept", "application/vnd.github.raw+json"))
        .respond_with(ResponseTemplate::new(200).set_body_string(big))
        .expect(1)
        .mount(&e.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/octo/big/readme"))
        .and(wiremock::matchers::header("accept", "application/vnd.github+json"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"path": "README.md", "size": 150_000, "encoding": "none", "content": ""})),
        )
        .mount(&e.server)
        .await;
    let readme = e.fetch("github_repo_readme_get", &json!({"repo": "octo/big"})).await;
    assert_eq!(readme[0].body.as_ref().unwrap().len(), 100_000);
    assert_eq!(readme[0].extra["truncated"], true);
}

// ---- repositories: writes ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_repository_is_created_private_by_default_for_the_user_or_an_organization() {
    let e = env().await;
    e.on_empty("GET", "/repos/Octo-Cat/new-thing", 404).await;
    e.on_empty("GET", "/repos/acme/tool", 404).await;
    e.expect(
        "POST",
        "/user/repos",
        json!({"name": "new-thing", "private": true, "description": "Fresh", "auto_init": true}),
        201,
        json!({"full_name": "Octo-Cat/new-thing", "html_url": "https://github.com/Octo-Cat/new-thing", "private": true, "default_branch": "main"}),
    )
    .await;
    e.expect(
        "POST",
        "/orgs/acme/repos",
        json!({"name": "tool", "private": false, "license_template": "mit", "gitignore_template": "Rust"}),
        201,
        json!({"full_name": "acme/tool", "html_url": "https://github.com/acme/tool", "private": false, "default_branch": "main"}),
    )
    .await;

    let mine = json!({"name": "new-thing", "description": "Fresh", "auto_init": true});
    let p = e.preview("github_repo_create", &mine).await;
    assert_eq!((p.resource.as_str(), p.once_only), ("Octo-Cat/new-thing", false));
    assert_eq!(p.parents, [("Octo-Cat".to_owned(), "Every repository of Octo-Cat".to_owned())]);
    assert_eq!(p.lines[0], "Create the private repository Octo-Cat/new-thing");
    assert!(text_of(&p).contains("Description: Fresh") && text_of(&p).contains("initial commit"), "{}", text_of(&p));
    let done = e.perform("github_repo_create", &mine).await;
    assert_eq!(done["created"], true);
    assert_eq!(done["full_name"], "Octo-Cat/new-thing");
    assert_eq!(done["url"], "https://github.com/Octo-Cat/new-thing");

    let org = json!({"name": "tool", "org": "acme", "private": false, "license_template": "mit", "gitignore_template": "Rust"});
    let p = e.preview("github_repo_create", &org).await;
    assert_eq!(p.resource, "acme/tool");
    assert!(p.lines[0].contains("PUBLIC repository acme/tool"), "{:?}", p.lines);
    assert_eq!(e.perform("github_repo_create", &org).await["private"], false);
}

#[tokio::test]
async fn creating_a_repository_that_exists_or_with_a_bad_name_is_refused() {
    let e = env().await;
    e.on("GET", "/repos/acme/tool", 200, repo_json("acme/tool", true)).await;
    let err = e.preview_err("github_repo_create", &json!({"name": "tool", "org": "acme"})).await;
    assert!(err.contains("acme/tool already exists"), "{err}");
    for bad in ["a/b", "a b", "..", ".", "x?y", "a%2Fb"] {
        let err = e.preview_err("github_repo_create", &json!({"name": bad})).await;
        assert!(err.contains("not a valid repository name"), "{bad}: {err}");
        assert!(
            e.perform_err("github_repo_create", &json!({"name": bad})).await.contains("not a valid repository name")
        );
    }
    assert!(parse_err("github_repo_create", &json!({"description": "x"})).contains("`name` is required"));
    assert!(e.requests().await.iter().all(|(m, _)| m == "GET"), "nothing was created");
}

#[tokio::test]
async fn settings_are_previewed_as_old_to_new_and_only_the_given_ones_are_sent() {
    let e = env().await;
    let mut now = repo_json("octo/cat", false);
    now["homepage"] = json!("https://old.example");
    e.on("GET", "/repos/octo/cat", 200, now).await;
    e.expect(
        "PATCH",
        "/repos/octo/cat",
        json!({"description": "Better", "homepage": "", "has_wiki": true, "allow_rebase_merge": true, "default_branch": "develop", "archived": true}),
        200,
        json!({"html_url": "https://github.com/octo/cat"}),
    )
    .await;
    let args = json!({
        "repo": "octo/cat", "description": "Better", "homepage": "", "has_wiki": true, "allow_rebase_merge": true,
        "default_branch": "develop", "archived": true
    });
    let p = e.preview("github_repo_update", &args).await;
    let t = text_of(&p);
    assert_eq!((p.resource.as_str(), p.once_only), ("octo/cat", false));
    assert!(t.contains("Description: 'A repo' \u{2192} 'Better'"), "{t}");
    assert!(t.contains("Homepage: 'https://old.example' \u{2192} ''"), "{t}");
    assert!(t.contains("Wiki: off \u{2192} on") && t.contains("Rebase merging: off \u{2192} on"), "{t}");
    assert!(t.contains("Default branch: main \u{2192} develop") && t.contains("becomes read-only"), "{t}");
    assert!(!t.contains("Issues"), "unchanged settings are not listed: {t}");
    let done = e.perform("github_repo_update", &args).await;
    assert_eq!(done["updated"], true);
    assert_eq!(
        done["changed"],
        json!(["allow_rebase_merge", "archived", "default_branch", "description", "has_wiki", "homepage"])
    );
}

#[tokio::test]
async fn a_settings_change_needs_something_to_change_and_refuses_unarchiving_and_bad_branches() {
    let e = env().await;
    e.on("GET", "/repos/octo/cat", 200, repo_json("octo/cat", false)).await;
    assert!(e.preview_err("github_repo_update", &json!({"repo": "octo/cat"})).await.contains("at least one setting"));
    assert!(e.perform_err("github_repo_update", &json!({"repo": "octo/cat"})).await.contains("at least one setting"));
    let err = e.preview_err("github_repo_update", &json!({"repo": "octo/cat", "archived": false})).await;
    assert!(err.contains("cannot unarchive"), "{err}");
    let err = e.preview_err("github_repo_update", &json!({"repo": "octo/cat", "default_branch": "a..b"})).await;
    assert!(err.contains("not a valid branch"), "{err}");
    assert!(e.requests().await.iter().all(|(m, _)| m == "GET"));
}

#[tokio::test]
async fn forking_names_the_target_and_starts_the_fork() {
    let e = env().await;
    e.on("GET", "/repos/up/cat", 200, repo_json("up/cat", false)).await;
    e.expect(
        "POST",
        "/repos/up/cat/forks",
        json!({"organization": "acme", "name": "cat2", "default_branch_only": true}),
        202,
        json!({"full_name": "acme/cat2", "html_url": "https://github.com/acme/cat2"}),
    )
    .await;
    e.expect("POST", "/repos/up/cat/forks", json!({}), 202, json!({"full_name": "octo-cat/cat", "html_url": "u"}))
        .await;
    let args = json!({"repo": "up/cat", "org": "acme", "name": "cat2", "default_branch_only": true});
    let p = e.preview("github_repo_fork", &args).await;
    assert_eq!(p.resource, "up/cat");
    assert!(p.lines[0].contains("Fork up/cat (public) into the organization acme"), "{:?}", p.lines);
    assert!(text_of(&p).contains("Named cat2") && text_of(&p).contains("Only the default branch"));
    assert_eq!(e.perform("github_repo_fork", &args).await["full_name"], "acme/cat2");
    let plain = json!({"repo": "up/cat"});
    assert!(e.preview("github_repo_fork", &plain).await.lines[0].ends_with("into your account"));
    assert_eq!(e.perform("github_repo_fork", &plain).await["forking"], true);
    assert!(
        e.preview_err("github_repo_fork", &json!({"repo": "up/cat", "name": "a/b"}))
            .await
            .contains("valid repository name")
    );
}

#[tokio::test]
async fn deleting_a_repository_is_previewed_prominently_and_asked_every_time() {
    let e = env().await;
    e.on("GET", "/repos/octo/cat", 200, repo_json("octo/cat", true)).await;
    e.expect_any("DELETE", "/repos/octo/cat", 204, Value::Null).await;
    let args = json!({"repo": "octo/cat"});
    let p = e.preview("github_repo_delete", &args).await;
    assert!(p.once_only);
    assert!(p.lines[0].starts_with("PERMANENTLY DELETE the private repository octo/cat"), "{:?}", p.lines);
    assert!(text_of(&p).contains("cannot be undone") && text_of(&p).contains("3 stars, 1 forks, 2 open issues"));
    assert_eq!(e.perform("github_repo_delete", &args).await, json!({"deleted": true, "repo": "octo/cat"}));
}

#[tokio::test]
async fn transferring_and_changing_visibility_are_previewed_once_only() {
    let e = env().await;
    e.on("GET", "/repos/octo/cat", 200, repo_json("octo/cat", true)).await;
    e.expect(
        "POST",
        "/repos/octo/cat/transfer",
        json!({"new_owner": "acme", "new_name": "tiger"}),
        202,
        json!({"html_url": "https://github.com/octo/cat"}),
    )
    .await;
    e.expect("PATCH", "/repos/octo/cat", json!({"visibility": "public"}), 200, json!({})).await;
    let transfer = json!({"repo": "octo/cat", "new_owner": "acme", "new_name": "tiger"});
    let p = e.preview("github_repo_transfer", &transfer).await;
    assert!(p.once_only && p.lines[0] == "Transfer the private repository octo/cat to acme", "{:?}", p.lines);
    assert!(text_of(&p).contains("renamed tiger"));
    assert_eq!(e.perform("github_repo_transfer", &transfer).await["transfer_started"], true);
    for bad in [
        json!({"repo": "octo/cat", "new_owner": "a/b"}),
        json!({"repo": "octo/cat", "new_owner": "acme", "new_name": "x y"}),
    ] {
        assert!(e.preview_err("github_repo_transfer", &bad).await.contains("valid"));
    }

    let visibility = json!({"repo": "octo/cat", "visibility": "public"});
    let p = e.preview("github_repo_visibility_set", &visibility).await;
    assert!(p.once_only && p.lines[0] == "Make octo/cat PUBLIC: it is private now", "{:?}", p.lines);
    assert!(text_of(&p).contains("Everyone on the internet"));
    assert_eq!(e.perform("github_repo_visibility_set", &visibility).await["visibility"], "public");
    let err = e.preview_err("github_repo_visibility_set", &json!({"repo": "octo/cat", "visibility": "private"})).await;
    assert!(err.contains("already private"), "{err}");
    assert!(
        parse_err("github_repo_visibility_set", &json!({"repo": "octo/cat", "visibility": "internal"}))
            .contains("one of")
    );
}

#[tokio::test]
async fn topics_are_replaced_after_showing_old_and_new() {
    let e = env().await;
    e.on("GET", "/repos/octo/cat/topics", 200, json!({"names": ["rust", "cli"]})).await;
    e.expect(
        "PUT",
        "/repos/octo/cat/topics",
        json!({"names": ["rust", "cats-2"]}),
        200,
        json!({"names": ["rust", "cats-2"]}),
    )
    .await;
    e.expect("PUT", "/repos/octo/cat/topics", json!({"names": []}), 200, json!({"names": []})).await;
    let args = json!({"repo": "octo/cat", "topics": ["rust", "cats-2"]});
    let p = e.preview("github_repo_topics_set", &args).await;
    assert!(text_of(&p).contains("Topics: [rust, cli] \u{2192} [rust, cats-2]"), "{}", text_of(&p));
    assert_eq!(e.perform("github_repo_topics_set", &args).await["topics"], json!(["rust", "cats-2"]));
    assert_eq!(
        e.perform("github_repo_topics_set", &json!({"repo": "octo/cat", "topics": []})).await["topics"],
        json!([])
    );
    for bad in ["Rust", "has space", "-lead", "under_score"] {
        let err = e.preview_err("github_repo_topics_set", &json!({"repo": "octo/cat", "topics": [bad]})).await;
        assert!(err.contains("not a valid topic"), "{bad}: {err}");
    }
    assert!(
        parse_err("github_repo_topics_set", &json!({"repo": "octo/cat", "topics": ["x".repeat(51)]})).contains("50")
    );
    assert!(
        e.preview_err("github_repo_topics_set", &json!({"repo": "octo/cat"})).await.contains("`topics` is required")
    );
}

// ---- branches ------------------------------------------------------------------------------------------------------

const SHA: &str = "0123456789abcdef0123456789abcdef01234567";
const SHA2: &str = "fedcba9876543210fedcba9876543210fedcba98";

fn branch_json(name: &str, protected: bool) -> Value {
    json!({
        "name": name, "protected": protected,
        "commit": {"sha": SHA, "commit": {"message": "Fix the thing\n\nLong body", "author": {"name": "Ann", "date": "2026-10-01T10:00:00Z"}}},
        "_links": {"html": format!("https://github.com/octo/cat/tree/{name}")},
    })
}

#[tokio::test]
async fn branches_are_listed_and_read_with_branch_resources() {
    let e = env().await;
    Mock::given(method("GET"))
        .and(path("/repos/octo/cat/branches"))
        .and(query_param("protected", "true"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([branch_json("main", true)])))
        .expect(1)
        .mount(&e.server)
        .await;
    e.on("GET", "/repos/octo/cat/branches", 200, json!([branch_json("main", true), branch_json("feature/x", false)]))
        .await;
    let protected = e.fetch("github_branch_list", &json!({"repo": "octo/cat", "protected": true})).await;
    assert_eq!(protected.len(), 1);
    let all = e.fetch("github_branch_list", &json!({"repo": "octo/cat"})).await;
    assert_eq!(
        all.iter().map(|b| (b.id.as_str(), b.resource.as_str(), b.snippet.as_str())).collect::<Vec<_>>(),
        [("main", "octo/cat@main", "protected"), ("feature/x", "octo/cat@feature/x", "not protected")]
    );
    assert_eq!(all[1].resource_label, "octo/cat, branch feature/x");
    assert_eq!(
        all[1].parents,
        [
            ("octo/cat".to_owned(), "Any branch of octo/cat".to_owned()),
            ("octo".to_owned(), "Every repository of octo".to_owned())
        ]
    );
    assert_eq!(all[1].extra["sha"], SHA);

    e.on("GET", "/repos/octo/cat/branches/feature%2Fx", 200, branch_json("feature/x", false)).await;
    let one = e.fetch("github_branch_get", &json!({"repo": "octo/cat", "branch": "feature/x"})).await;
    assert_eq!(one[0].resource, "octo/cat@feature/x");
    assert_eq!(one[0].snippet, "Fix the thing");
    assert!(one[0].body.as_ref().unwrap().contains("Head 0123456: Fix the thing (Ann)"));
    assert_eq!(one[0].extra["protected"], false);
    assert_eq!(one[0].extra["sha"], SHA);
}

#[tokio::test]
async fn bad_branch_names_are_refused_before_any_request() {
    let e = env().await;
    for bad in ["a..b", "a b", "-x", "x.lock", "a~b", "a\\b", "/x", "a@{b"] {
        let err = e.fetch_err("github_branch_get", &json!({"repo": "octo/cat", "branch": bad})).await;
        assert!(err.contains("not a valid branch"), "{bad}: {err}");
        for tool in ["github_branch_delete", "github_branch_protection_delete"] {
            let err = e.preview_err(tool, &json!({"repo": "octo/cat", "branch": bad})).await;
            assert!(err.contains("not a valid branch"), "{tool} {bad}: {err}");
        }
    }
    let err = e.preview_err("github_branch_create", &json!({"repo": "octo/cat", "branch": "ok", "from": "a..b"})).await;
    assert!(err.contains("`from` is not a valid"), "{err}");
    let err =
        e.preview_err("github_branch_rename", &json!({"repo": "octo/cat", "branch": "ok", "new_name": "a b"})).await;
    assert!(err.contains("`new_name` is not a valid"), "{err}");
    let err = e.preview_err("github_branch_merge", &json!({"repo": "octo/cat", "base": "main", "head": "x:y"})).await;
    assert!(err.contains("`head` is not a valid"), "{err}");
    assert!(e.requests().await.is_empty(), "{:?}", e.requests().await);
    assert!(parse_err("github_branch_get", &json!({"repo": "octo/cat"})).contains("`branch` is required"));
}

#[tokio::test]
async fn a_branch_is_created_from_the_default_branch_a_named_branch_a_tag_or_a_sha() {
    let e = env().await;
    e.on("GET", "/repos/octo/cat", 200, repo_json("octo/cat", false)).await;
    e.on_empty("GET", "/repos/octo/cat/git/ref/heads/new", 404).await;
    e.on("GET", "/repos/octo/cat/git/ref/heads/main", 200, json!({"object": {"sha": SHA, "type": "commit"}})).await;
    e.on_empty("GET", "/repos/octo/cat/git/ref/heads/v1", 404).await;
    e.on("GET", "/repos/octo/cat/git/ref/tags/v1", 200, json!({"object": {"sha": "tagobj", "type": "tag"}})).await;
    e.on("GET", "/repos/octo/cat/git/tags/tagobj", 200, json!({"object": {"sha": SHA2}})).await;
    e.on("GET", &format!("/repos/octo/cat/git/commits/{SHA}"), 200, json!({"message": "Init\n\nmore"})).await;
    e.on("GET", "/repos/octo/cat/git/ref/heads/exists", 200, json!({"object": {"sha": SHA}})).await;
    e.expect(
        "POST",
        "/repos/octo/cat/git/refs",
        json!({"ref": "refs/heads/new", "sha": SHA}),
        201,
        json!({"ref": "refs/heads/new"}),
    )
    .await;
    e.expect(
        "POST",
        "/repos/octo/cat/git/refs",
        json!({"ref": "refs/heads/rel", "sha": SHA2}),
        201,
        json!({"ref": "refs/heads/rel"}),
    )
    .await;
    e.expect(
        "POST",
        "/repos/octo/cat/git/refs",
        json!({"ref": "refs/heads/at", "sha": SHA}),
        201,
        json!({"ref": "refs/heads/at"}),
    )
    .await;
    e.on_empty("GET", "/repos/octo/cat/git/ref/heads/at", 404).await;
    e.on_empty("GET", "/repos/octo/cat/git/ref/heads/rel", 404).await;

    let default = json!({"repo": "octo/cat", "branch": "new"});
    let p = e.preview("github_branch_create", &default).await;
    assert_eq!((p.resource.as_str(), p.resource_label.as_str()), ("octo/cat@new", "octo/cat, branch new"));
    assert_eq!(p.parents[0].0, "octo/cat");
    assert!(
        text_of(&p).contains("Create the branch new in octo/cat") && text_of(&p).contains("branch main (0123456)"),
        "{}",
        text_of(&p)
    );
    let done = e.perform("github_branch_create", &default).await;
    assert_eq!((done["created"].clone(), done["sha"].clone()), (json!(true), json!(SHA)));

    let tag = json!({"repo": "octo/cat", "branch": "rel", "from": "v1"});
    assert!(text_of(&e.preview("github_branch_create", &tag).await).contains("tag v1 (fedcba9)"));
    assert_eq!(e.perform("github_branch_create", &tag).await["sha"], SHA2);

    let sha = json!({"repo": "octo/cat", "branch": "at", "from": SHA});
    assert!(text_of(&e.preview("github_branch_create", &sha).await).contains("commit 0123456: Init"));
    assert_eq!(e.perform("github_branch_create", &sha).await["branch"], "at");

    let err = e.preview_err("github_branch_create", &json!({"repo": "octo/cat", "branch": "exists"})).await;
    assert!(err.contains("already has a branch named exists"), "{err}");
    let err =
        e.preview_err("github_branch_create", &json!({"repo": "octo/cat", "branch": "n2", "from": "nothing"})).await;
    assert!(err.contains("no branch or tag named nothing"), "{err}");
    let err = e.preview_err("github_branch_create", &json!({"repo": "octo/cat", "branch": "n3", "from": SHA2})).await;
    assert!(err.contains("no commit fedcba9"), "{err}");
}

#[tokio::test]
async fn deleting_a_branch_shows_its_head_and_refuses_the_default_branch() {
    let e = env().await;
    e.on("GET", "/repos/octo/cat", 200, repo_json("octo/cat", false)).await;
    e.on("GET", "/repos/octo/cat/branches/old", 200, branch_json("old", false)).await;
    e.on("GET", "/repos/octo/cat/branches/prot", 200, branch_json("prot", true)).await;
    e.on("GET", "/repos/octo/cat/branches/main", 200, branch_json("main", true)).await;
    e.on("GET", "/repos/octo/cat/git/ref/heads/old", 200, json!({"object": {"sha": SHA}})).await;
    e.expect_any("DELETE", "/repos/octo/cat/git/refs/heads/old", 204, Value::Null).await;
    let args = json!({"repo": "octo/cat", "branch": "old"});
    let p = e.preview("github_branch_delete", &args).await;
    assert_eq!(p.resource, "octo/cat@old");
    assert!(!p.once_only);
    assert!(p.lines[1].contains("Its head is 0123456: Fix the thing (Ann)"), "{:?}", p.lines);
    let done = e.perform("github_branch_delete", &args).await;
    assert_eq!((done["deleted"].clone(), done["last_sha"].clone()), (json!(true), json!(SHA)));

    let p = e.preview("github_branch_delete", &json!({"repo": "octo/cat", "branch": "prot"})).await;
    assert!(text_of(&p).contains("protected"));
    let err = e.preview_err("github_branch_delete", &json!({"repo": "octo/cat", "branch": "main"})).await;
    assert!(err.contains("default branch"), "{err}");
}

#[tokio::test]
async fn renaming_a_branch_is_once_only_when_it_is_the_default_branch() {
    let e = env().await;
    e.on("GET", "/repos/octo/cat", 200, repo_json("octo/cat", false)).await;
    e.on("GET", "/repos/octo/cat/branches/main", 200, branch_json("main", true)).await;
    e.on("GET", "/repos/octo/cat/branches/feat", 200, branch_json("feat", false)).await;
    e.on_empty("GET", "/repos/octo/cat/git/ref/heads/trunk", 404).await;
    e.on_empty("GET", "/repos/octo/cat/git/ref/heads/feature2", 404).await;
    e.on("GET", "/repos/octo/cat/git/ref/heads/taken", 200, json!({"object": {"sha": SHA}})).await;
    e.expect(
        "POST",
        "/repos/octo/cat/branches/feat/rename",
        json!({"new_name": "feature2"}),
        201,
        branch_json("feature2", false),
    )
    .await;

    let plain = json!({"repo": "octo/cat", "branch": "feat", "new_name": "feature2"});
    let p = e.preview("github_branch_rename", &plain).await;
    assert!(!p.once_only);
    assert_eq!(p.resource, "octo/cat@feat");
    assert!(p.lines[0].contains("Rename the branch feat of octo/cat to feature2"), "{:?}", p.lines);
    let done = e.perform("github_branch_rename", &plain).await;
    assert_eq!((done["renamed"].clone(), done["new_name"].clone()), (json!(true), json!("feature2")));

    let default =
        e.preview("github_branch_rename", &json!({"repo": "octo/cat", "branch": "main", "new_name": "trunk"})).await;
    assert!(default.once_only, "renaming the default branch is asked every time");
    assert!(text_of(&default).contains("default branch"));

    let err = e
        .preview_err("github_branch_rename", &json!({"repo": "octo/cat", "branch": "feat", "new_name": "taken"}))
        .await;
    assert!(err.contains("already has a branch named taken"), "{err}");
    let err =
        e.preview_err("github_branch_rename", &json!({"repo": "octo/cat", "branch": "feat", "new_name": "feat"})).await;
    assert!(err.contains("same"), "{err}");
}

fn comparison(ahead: i64) -> Value {
    let commits: Vec<Value> = (0..ahead.min(12))
        .map(|i| json!({"sha": format!("{i:07}aaaaaaaa"), "commit": {"message": format!("Commit {i}\n\nbody"), "author": {"name": "Ann"}}}))
        .collect();
    json!({"ahead_by": ahead, "behind_by": 1, "commits": commits, "files": [{"filename": "a"}, {"filename": "b"}]})
}

#[tokio::test]
async fn merging_previews_the_commits_and_targets_the_base_branch() {
    let e = env().await;
    e.on("GET", "/repos/octo/cat", 200, repo_json("octo/cat", false)).await;
    e.on("GET", "/repos/octo/cat/compare/main...feature%2Fx", 200, comparison(3)).await;
    e.on("GET", "/repos/octo/cat/compare/develop...main", 200, comparison(0)).await;
    e.expect(
        "POST",
        "/repos/octo/cat/merges",
        json!({"base": "main", "head": "feature/x", "commit_message": "Merge feature x"}),
        201,
        json!({"sha": SHA, "html_url": "https://github.com/octo/cat/commit/abc"}),
    )
    .await;
    e.expect("POST", "/repos/octo/cat/merges", json!({"base": "develop", "head": "main"}), 204, Value::Null).await;

    let args = json!({"repo": "octo/cat", "base": "main", "head": "feature/x", "commit_message": "Merge feature x"});
    let p = e.preview("github_branch_merge", &args).await;
    let t = text_of(&p);
    assert_eq!((p.resource.as_str(), p.resource_label.as_str()), ("octo/cat@main", "octo/cat, branch main"));
    assert_eq!(p.parents[0].0, "octo/cat");
    assert!(t.contains("Merge feature/x into main of octo/cat (the default branch)"), "{t}");
    assert!(t.contains("3 commit(s) would be merged") && t.contains("- 0000000 Commit 0 (Ann)"), "{t}");
    assert!(t.contains("2 file(s) changed") && t.contains("Merge commit message: Merge feature x"), "{t}");
    let done = e.perform("github_branch_merge", &args).await;
    assert_eq!((done["merged"].clone(), done["sha"].clone()), (json!(true), json!(SHA)));

    let nothing = json!({"repo": "octo/cat", "base": "develop", "head": "main"});
    assert!(text_of(&e.preview("github_branch_merge", &nothing).await).contains("Nothing to merge"));
    let done = e.perform("github_branch_merge", &nothing).await;
    assert_eq!((done["merged"].clone(), done["already_up_to_date"].clone()), (json!(false), json!(true)));
}

#[tokio::test]
async fn a_merge_conflict_or_a_missing_branch_is_reported_not_hidden() {
    let e = env().await;
    e.on("POST", "/repos/octo/cat/merges", 409, json!({"message": "Merge conflict"})).await;
    let err = e.perform_err("github_branch_merge", &json!({"repo": "octo/cat", "base": "main", "head": "x"})).await;
    assert!(err.contains("cannot do that right now: Merge conflict"), "{err}");
    e.on("GET", "/repos/octo/cat/compare/main...gone", 404, json!({"message": "Not Found"})).await;
    let err = e.preview_err("github_branch_merge", &json!({"repo": "octo/cat", "base": "main", "head": "gone"})).await;
    assert!(err.contains("does not exist"), "{err}");
}

// ---- protection and rulesets -----------------------------------------------------------------------------------------

fn protection_json() -> Value {
    json!({
        "required_status_checks": {"strict": true, "contexts": ["ci", "lint"]},
        "required_pull_request_reviews": {"required_approving_review_count": 2, "dismiss_stale_reviews": true, "require_code_owner_reviews": false},
        "enforce_admins": {"enabled": true}, "required_linear_history": {"enabled": true},
        "allow_force_pushes": {"enabled": false}, "allow_deletions": {"enabled": false},
        "restrictions": {"users": [{"login": "ann"}], "teams": [{"slug": "core"}]},
    })
}

#[tokio::test]
async fn branch_protection_is_read_including_the_unprotected_case() {
    let e = env().await;
    e.on("GET", "/repos/octo/cat/branches/main/protection", 200, protection_json()).await;
    e.on("GET", "/repos/octo/cat/branches/open/protection", 404, json!({"message": "Branch not protected"})).await;
    e.on("GET", "/repos/octo/cat/branches/gone/protection", 404, json!({"message": "Branch not found"})).await;
    let p = e.fetch("github_branch_protection_get", &json!({"repo": "octo/cat", "branch": "main"})).await;
    let body = p[0].body.clone().unwrap();
    assert_eq!(p[0].resource, "octo/cat@main");
    assert!(body.contains("Required checks: ci, lint, branch must be up to date"), "{body}");
    assert!(
        body.contains("Pull request reviews: 2 approving, stale approvals dismissed on, code owners off"),
        "{body}"
    );
    assert!(body.contains("Applies to administrators") && body.contains("Linear history required"), "{body}");
    assert!(!body.contains("Force pushes allowed"), "{body}");
    assert!(body.contains("Only these may push: users [ann], teams [core]"), "{body}");
    assert_eq!(p[0].extra["protected"], true);
    assert_eq!(p[0].extra["required_status_checks"]["contexts"], json!(["ci", "lint"]));

    let open = e.fetch("github_branch_protection_get", &json!({"repo": "octo/cat", "branch": "open"})).await;
    assert_eq!((open[0].snippet.as_str(), open[0].extra["protected"].clone()), ("not protected", json!(false)));
    let err = e.fetch_err("github_branch_protection_get", &json!({"repo": "octo/cat", "branch": "gone"})).await;
    assert!(err.contains("does not exist") && err.contains("Branch not found"), "{err}");
}

#[tokio::test]
async fn setting_protection_replaces_everything_and_shows_before_and_after() {
    let e = env().await;
    e.on("GET", "/repos/octo/cat/branches/main/protection", 200, protection_json()).await;
    e.on("GET", "/repos/octo/cat/branches/fresh/protection", 404, json!({"message": "Branch not protected"})).await;
    e.expect(
        "PUT",
        "/repos/octo/cat/branches/main/protection",
        json!({
            "required_status_checks": {"strict": true, "contexts": ["ci"]},
            "enforce_admins": true,
            "required_pull_request_reviews": {"required_approving_review_count": 1, "dismiss_stale_reviews": false, "require_code_owner_reviews": true},
            "restrictions": {"users": ["ann"], "teams": [], "apps": []},
            "required_linear_history": false, "allow_force_pushes": false, "allow_deletions": false,
            "required_conversation_resolution": true, "lock_branch": false,
        }),
        200,
        json!({}),
    )
    .await;
    e.expect(
        "PUT",
        "/repos/octo/cat/branches/fresh/protection",
        json!({
            "required_status_checks": null, "enforce_admins": false, "required_pull_request_reviews": null,
            "restrictions": null, "required_linear_history": false, "allow_force_pushes": false,
            "allow_deletions": false, "required_conversation_resolution": false, "lock_branch": false,
        }),
        200,
        json!({}),
    )
    .await;
    let args = json!({
        "repo": "octo/cat", "branch": "main", "status_check_contexts": ["ci"], "strict_status_checks": true,
        "required_approvals": 1, "require_code_owner_reviews": true, "enforce_admins": true,
        "required_conversation_resolution": true, "restrict_push_users": ["ann"]
    });
    let p = e.preview("github_branch_protection_set", &args).await;
    let t = text_of(&p);
    assert!(p.once_only);
    assert_eq!((p.resource.as_str(), p.parents[0].0.as_str()), ("octo/cat@main", "octo/cat"));
    let (before, after) = t.split_once("After:").unwrap();
    assert!(before.contains("Now:") && before.contains("ci, lint") && before.contains("2 approving"), "{t}");
    assert!(
        after.contains("Required checks: ci, branch must be up to date")
            && after.contains("1 approving, stale approvals dismissed off, code owners on"),
        "{t}"
    );
    assert!(after.contains("Conversations must be resolved") && after.contains("users [ann], teams []"), "{t}");
    assert!(!after.contains("Linear history"), "rules not asked for are turned off: {t}");
    let done = e.perform("github_branch_protection_set", &args).await;
    assert_eq!(done["updated"], true);

    let bare = json!({"repo": "octo/cat", "branch": "fresh"});
    let t = text_of(&e.preview("github_branch_protection_set", &bare).await);
    assert!(t.contains("Now:\n  not protected"), "{t}");
    assert_eq!(e.perform("github_branch_protection_set", &bare).await["updated"], true);
}

#[tokio::test]
async fn protection_arguments_are_checked() {
    let e = env().await;
    let err = e
        .preview_err(
            "github_branch_protection_set",
            &json!({"repo": "octo/cat", "branch": "main", "dismiss_stale_reviews": true}),
        )
        .await;
    assert!(err.contains("need `required_approvals`"), "{err}");
    let err = e
        .preview_err(
            "github_branch_protection_set",
            &json!({"repo": "octo/cat", "branch": "main", "restrict_push_teams": ["a/b"]}),
        )
        .await;
    assert!(err.contains("invalid name"), "{err}");
    assert!(
        parse_err(
            "github_branch_protection_set",
            &json!({"repo": "octo/cat", "branch": "main", "required_approvals": 7})
        )
        .contains("0..=6")
    );
    assert!(e.requests().await.is_empty());
}

#[tokio::test]
async fn removing_protection_lists_the_rules_that_go_and_refuses_when_there_are_none() {
    let e = env().await;
    e.on("GET", "/repos/octo/cat/branches/main/protection", 200, protection_json()).await;
    e.on("GET", "/repos/octo/cat/branches/open/protection", 404, json!({"message": "Branch not protected"})).await;
    e.expect_any("DELETE", "/repos/octo/cat/branches/main/protection", 204, Value::Null).await;
    let args = json!({"repo": "octo/cat", "branch": "main"});
    let p = e.preview("github_branch_protection_delete", &args).await;
    assert!(p.once_only && p.resource == "octo/cat@main");
    assert!(
        p.lines[0].contains("Remove ALL protection from the branch main of octo/cat")
            && text_of(&p).contains("ci, lint")
    );
    assert_eq!(e.perform("github_branch_protection_delete", &args).await, json!({"removed": true, "branch": "main"}));
    let err = e.preview_err("github_branch_protection_delete", &json!({"repo": "octo/cat", "branch": "open"})).await;
    assert!(err.contains("no protection to remove"), "{err}");
}

#[tokio::test]
async fn rulesets_are_listed_and_read() {
    let e = env().await;
    e.on("GET", "/repos/octo/cat/rulesets", 200, json!([
        {"id": 5, "name": "Main rules", "target": "branch", "enforcement": "active", "source_type": "Repository", "source": "octo/cat", "updated_at": "2026-10-01T00:00:00Z"}])).await;
    e.on(
        "GET",
        "/repos/octo/cat/rulesets/5",
        200,
        json!({
        "id": 5, "name": "Main rules", "target": "branch", "enforcement": "active",
        "conditions": {"ref_name": {"include": ["~DEFAULT_BRANCH"], "exclude": ["refs/heads/wip"]}},
        "rules": [{"type": "deletion"}, {"type": "pull_request"}], "bypass_actors": [{"actor_id": 1}]}),
    )
    .await;
    let list = e.fetch("github_ruleset_list", &json!({"repo": "octo/cat"})).await;
    assert_eq!(
        (list[0].id.as_str(), list[0].title.as_str(), list[0].snippet.as_str()),
        ("5", "Main rules", "branch · active")
    );
    let one = e.fetch("github_ruleset_get", &json!({"repo": "octo/cat", "ruleset_id": 5})).await;
    let body = one[0].body.clone().unwrap();
    assert!(
        body.contains("Applies to: ~DEFAULT_BRANCH (except refs/heads/wip)")
            && body.contains("Rules: deletion, pull_request"),
        "{body}"
    );
    assert!(body.contains("Bypass actors: 1"));
    assert_eq!(one[0].extra["rules"][1]["type"], "pull_request");
    assert!(parse_err("github_ruleset_get", &json!({"repo": "octo/cat"})).contains("`ruleset_id` is required"));
}

// ---- access -------------------------------------------------------------------------------------------------------

#[tokio::test]
async fn collaborators_invitations_and_teams_are_listed() {
    let e = env().await;
    Mock::given(method("GET"))
        .and(path("/repos/octo/cat/collaborators"))
        .and(query_param("affiliation", "outside"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"login": "ann", "role_name": "write", "permissions": {"push": true}, "site_admin": false}])))
        .expect(1)
        .mount(&e.server)
        .await;
    let people = e.fetch("github_collaborator_list", &json!({"repo": "octo/cat", "affiliation": "outside"})).await;
    assert_eq!(
        (people[0].id.as_str(), people[0].from.as_str(), people[0].snippet.as_str()),
        ("ann", "write", "write access")
    );
    assert_eq!(people[0].extra["permissions"]["push"], true);

    e.on("GET", "/repos/octo/cat/invitations", 200, json!([
        {"id": 11, "invitee": {"login": "bob"}, "inviter": {"login": "octo"}, "permissions": "read", "created_at": "2026-10-01T00:00:00Z", "expired": false}])).await;
    let inv = e.fetch("github_invitation_list", &json!({"repo": "octo/cat"})).await;
    assert_eq!(
        (inv[0].id.as_str(), inv[0].title.as_str(), inv[0].snippet.as_str()),
        ("11", "bob", "read · invited by octo")
    );

    e.on(
        "GET",
        "/repos/octo/cat/teams",
        200,
        json!([{"slug": "core", "name": "Core team", "permission": "push", "privacy": "closed"}]),
    )
    .await;
    let teams = e.fetch("github_repo_team_list", &json!({"repo": "octo/cat"})).await;
    assert_eq!(
        (teams[0].id.as_str(), teams[0].title.as_str(), teams[0].snippet.as_str()),
        ("core", "Core team", "push access · closed")
    );
}

#[tokio::test]
async fn adding_a_collaborator_shows_who_and_what_permission_and_is_once_only() {
    let e = env().await;
    e.on("GET", "/users/ann", 200, json!({"login": "ann", "name": "Ann Lee"})).await;
    e.on("GET", "/users/bob", 200, json!({"login": "bob", "name": null})).await;
    e.on_empty("GET", "/repos/octo/cat/collaborators/ann", 404).await;
    e.on_empty("GET", "/repos/octo/cat/collaborators/bob", 204).await;
    e.expect("PUT", "/repos/octo/cat/collaborators/ann", json!({"permission": "admin"}), 201, json!({"id": 77})).await;
    e.expect("PUT", "/repos/octo/cat/collaborators/bob", json!({"permission": "pull"}), 204, Value::Null).await;

    let ann = json!({"repo": "octo/cat", "username": "ann", "permission": "admin"});
    let p = e.preview("github_collaborator_add", &ann).await;
    assert!(p.once_only);
    assert_eq!(p.lines[0], "Give ann (Ann Lee) admin access to octo/cat");
    assert!(
        text_of(&p).contains("Admin access includes") && text_of(&p).contains("invitation by email"),
        "{}",
        text_of(&p)
    );
    let done = e.perform("github_collaborator_add", &ann).await;
    assert_eq!((done["invited"].clone(), done["invitation_id"].clone()), (json!(true), json!(77)));

    let bob = json!({"repo": "octo/cat", "username": "bob", "permission": "pull"});
    assert!(text_of(&e.preview("github_collaborator_add", &bob).await).contains("already have access"));
    assert_eq!(e.perform("github_collaborator_add", &bob).await["added"], true);

    for bad in ["a/b", "../x", "a b"] {
        let err = e
            .preview_err("github_collaborator_add", &json!({"repo": "octo/cat", "username": bad, "permission": "pull"}))
            .await;
        assert!(err.contains("valid GitHub"), "{bad}: {err}");
    }
    assert!(
        parse_err("github_collaborator_add", &json!({"repo": "octo/cat", "username": "ann"}))
            .contains("`permission` is required")
    );
    assert!(
        parse_err("github_collaborator_add", &json!({"repo": "octo/cat", "username": "ann", "permission": "owner"}))
            .contains("one of")
    );
}

#[tokio::test]
async fn removing_a_collaborator_and_cancelling_an_invitation_show_what_goes() {
    let e = env().await;
    e.on(
        "GET",
        "/repos/octo/cat/collaborators/ann/permission",
        200,
        json!({"permission": "write", "role_name": "write"}),
    )
    .await;
    e.on("GET", "/repos/octo/cat/collaborators/zed/permission", 404, json!({"message": "Not Found"})).await;
    e.expect_any("DELETE", "/repos/octo/cat/collaborators/ann", 204, Value::Null).await;
    let ann = json!({"repo": "octo/cat", "username": "ann"});
    let p = e.preview("github_collaborator_remove", &ann).await;
    assert!(
        p.once_only
            && text_of(&p).contains("Remove ann from octo/cat")
            && text_of(&p).contains("current permission: write"),
        "{}",
        text_of(&p)
    );
    assert_eq!(e.perform("github_collaborator_remove", &ann).await, json!({"removed": true, "username": "ann"}));
    let err = e.preview_err("github_collaborator_remove", &json!({"repo": "octo/cat", "username": "zed"})).await;
    assert!(err.contains("zed is not a collaborator"), "{err}");

    e.on(
        "GET",
        "/repos/octo/cat/invitations",
        200,
        json!([{"id": 11, "invitee": {"login": "bob"}, "permissions": "read"}]),
    )
    .await;
    e.expect_any("DELETE", "/repos/octo/cat/invitations/11", 204, Value::Null).await;
    let p = e.preview("github_invitation_cancel", &json!({"repo": "octo/cat", "invitation_id": 11})).await;
    assert!(p.once_only && p.lines[0] == "Cancel the invitation of bob to octo/cat (read access)", "{:?}", p.lines);
    assert_eq!(
        e.perform("github_invitation_cancel", &json!({"repo": "octo/cat", "invitation_id": 11})).await["cancelled"],
        true
    );
    let err = e.preview_err("github_invitation_cancel", &json!({"repo": "octo/cat", "invitation_id": 12})).await;
    assert!(err.contains("no pending invitation 12"), "{err}");
}

#[tokio::test]
async fn teams_are_added_and_removed_through_the_repository_owner_organization() {
    let e = env().await;
    e.on("GET", "/orgs/acme/teams/core", 200, json!({"slug": "core", "name": "Core team", "members_count": 4})).await;
    e.expect("PUT", "/orgs/acme/teams/core/repos/acme/tool", json!({"permission": "maintain"}), 204, Value::Null).await;
    e.expect_any("DELETE", "/orgs/acme/teams/core/repos/acme/tool", 204, Value::Null).await;
    let add = json!({"repo": "acme/tool", "team": "core", "permission": "maintain"});
    let p = e.preview("github_repo_team_add", &add).await;
    assert!(p.once_only);
    assert_eq!(p.lines[0], "Give the team acme/core (Core team) maintain access to acme/tool");
    assert!(p.lines[1].contains("4 member(s)"));
    assert_eq!(
        e.perform("github_repo_team_add", &add).await,
        json!({"added": true, "team": "core", "permission": "maintain"})
    );
    let remove = json!({"repo": "acme/tool", "team": "core"});
    assert_eq!(
        e.preview("github_repo_team_remove", &remove).await.lines[0],
        "Remove the team acme/core (Core team) from acme/tool"
    );
    assert_eq!(e.perform("github_repo_team_remove", &remove).await, json!({"removed": true, "team": "core"}));
    let err =
        e.preview_err("github_repo_team_add", &json!({"repo": "acme/tool", "team": "a/b", "permission": "pull"})).await;
    assert!(err.contains("valid GitHub"), "{err}");
}

// ---- integrations ----------------------------------------------------------------------------------------------------

const SLACK: &str = "https://hooks.slack.com/services/T000/B000/SECRETPART";

fn hook_json() -> Value {
    json!({
        "id": 9, "active": true, "events": ["push", "pull_request"], "updated_at": "2026-10-01T00:00:00Z",
        "config": {"url": SLACK, "content_type": "json", "secret": "********", "insecure_ssl": "0"},
        "last_response": {"code": 200, "status": "active", "message": "OK"},
    })
}

#[tokio::test]
async fn webhooks_are_listed_and_read_without_their_secret_address() {
    let e = env().await;
    e.on("GET", "/repos/octo/cat/hooks", 200, json!([hook_json()])).await;
    e.on("GET", "/repos/octo/cat/hooks/9", 200, hook_json()).await;
    let list = e.fetch("github_webhook_list", &json!({"repo": "octo/cat"})).await;
    assert_eq!(list[0].id, "9");
    assert_eq!(list[0].snippet, "https://hooks.slack.com/\u{2026} \u{b7} push, pull_request \u{b7} active");
    assert_eq!(list[0].extra["events"], json!(["push", "pull_request"]));
    assert_eq!(list[0].extra["last_response"]["code"], 200);
    let one = e.fetch("github_webhook_get", &json!({"repo": "octo/cat", "hook_id": 9})).await;
    let body = one[0].body.clone().unwrap();
    assert!(
        body.contains("Delivers to https://hooks.slack.com/\u{2026}") && body.contains("Last delivery: 200 OK"),
        "{body}"
    );
    for item in list.iter().chain(&one) {
        let all = serde_json::to_string(item).unwrap();
        assert!(!all.contains("SECRETPART") && !all.contains("********"), "{all}");
    }
    assert!(parse_err("github_webhook_get", &json!({"repo": "octo/cat"})).contains("`hook_id` is required"));
}

#[tokio::test]
async fn creating_a_webhook_shows_the_address_and_never_the_secret() {
    let e = env().await;
    e.expect(
        "POST",
        "/repos/octo/cat/hooks",
        json!({
            "name": "web", "active": false, "events": ["push", "issues"],
            "config": {"url": "https://ci.example.com/hook", "content_type": "form", "insecure_ssl": "1", "secret": "s3cr3t-value"},
        }),
        201,
        json!({"id": 21, "events": ["push", "issues"], "active": false, "config": {"secret": "********"}}),
    )
    .await;
    e.expect(
        "POST",
        "/repos/octo/cat/hooks",
        json!({"name": "web", "active": true, "events": ["push"], "config": {"url": "https://ci.example.com/x", "content_type": "json", "insecure_ssl": "0"}}),
        201,
        json!({"id": 22, "events": ["push"], "active": true}),
    )
    .await;
    let args = json!({
        "repo": "octo/cat", "url": "https://ci.example.com/hook", "events": ["push", "issues"], "content_type": "form",
        "secret": "s3cr3t-value", "insecure_ssl": true, "active": false
    });
    let p = e.preview("github_webhook_create", &args).await;
    let t = text_of(&p);
    assert!(p.once_only);
    assert!(
        t.contains("Sends push, issues to https://ci.example.com/hook")
            && t.contains("signed with a secret (not shown"),
        "{t}"
    );
    assert!(t.contains("certificate will NOT be checked") && t.contains("inactive"), "{t}");
    assert!(!t.contains("s3cr3t"), "{t}");
    let done = e.perform("github_webhook_create", &args).await;
    assert!(!done.to_string().contains("s3cr3t"), "{done}");
    assert_eq!((done["created"].clone(), done["hook_id"].clone()), (json!(true), json!(21)));

    let plain = json!({"repo": "octo/cat", "url": "https://ci.example.com/x"});
    assert!(text_of(&e.preview("github_webhook_create", &plain).await).contains("No secret"));
    assert_eq!(e.perform("github_webhook_create", &plain).await["hook_id"], 22);
}

#[tokio::test]
async fn webhook_addresses_and_events_are_validated() {
    let e = env().await;
    for bad in
        ["http://plain.example.com/x", "ftp://x.example.com", "not a url", "https://user:pw@x.example.com/", "https://"]
    {
        let err = e.preview_err("github_webhook_create", &json!({"repo": "octo/cat", "url": bad})).await;
        assert!(err.contains("https address"), "{bad}: {err}");
    }
    for bad in ["Push", "a b", "push;drop", "../x"] {
        let err = e
            .preview_err(
                "github_webhook_create",
                &json!({"repo": "octo/cat", "url": "https://x.example.com/", "events": [bad]}),
            )
            .await;
        assert!(err.contains("is not an event name"), "{bad}: {err}");
    }
    assert!(
        e.preview_err(
            "github_webhook_create",
            &json!({"repo": "octo/cat", "url": "https://x.example.com/", "events": []})
        )
        .await
        .contains("at least one")
    );
    assert!(
        e.preview_err("github_webhook_update", &json!({"repo": "octo/cat", "hook_id": 9}))
            .await
            .contains("at least one thing")
    );
    assert!(e.requests().await.is_empty());
}

#[tokio::test]
async fn updating_deleting_and_pinging_a_webhook_keep_the_rest_of_its_configuration() {
    let e = env().await;
    e.on("GET", "/repos/octo/cat/hooks/9", 200, hook_json()).await;
    e.expect(
        "PATCH",
        "/repos/octo/cat/hooks/9",
        json!({
            "events": ["release"], "active": false,
            "config": {"url": "https://new.example.com/h", "content_type": "json", "secret": "********", "insecure_ssl": "0"},
        }),
        200,
        json!({"events": ["release"], "active": false}),
    )
    .await;
    e.expect(
        "PATCH",
        "/repos/octo/cat/hooks/9",
        json!({"active": true}),
        200,
        json!({"events": ["push"], "active": true}),
    )
    .await;
    e.expect_any("DELETE", "/repos/octo/cat/hooks/9", 204, Value::Null).await;
    e.expect_any("POST", "/repos/octo/cat/hooks/9/pings", 204, Value::Null).await;

    let args = json!({"repo": "octo/cat", "hook_id": 9, "url": "https://new.example.com/h", "events": ["release"], "active": false});
    let p = e.preview("github_webhook_update", &args).await;
    let t = text_of(&p);
    assert!(p.once_only);
    assert!(t.contains("Address: https://hooks.slack.com/\u{2026} \u{2192} https://new.example.com/h"), "{t}");
    assert!(
        t.contains("Events: push, pull_request \u{2192} release") && t.contains("Active: true \u{2192} false"),
        "{t}"
    );
    assert!(!t.contains("SECRETPART"), "{t}");
    assert_eq!(e.perform("github_webhook_update", &args).await["updated"], true);
    assert_eq!(
        e.perform("github_webhook_update", &json!({"repo": "octo/cat", "hook_id": 9, "active": true})).await["active"],
        true
    );

    let touch = json!({"repo": "octo/cat", "hook_id": 9});
    let p = e.preview("github_webhook_delete", &touch).await;
    assert!(p.once_only && p.lines[0] == "Delete the webhook 9 of octo/cat", "{:?}", p.lines);
    assert!(
        p.lines[1].contains("push, pull_request to https://hooks.slack.com/\u{2026}")
            && !text_of(&p).contains("SECRETPART")
    );
    assert_eq!(e.perform("github_webhook_delete", &touch).await, json!({"deleted": true, "hook_id": 9}));
    assert!(
        e.preview("github_webhook_ping", &touch).await.lines[0].starts_with("Send a test delivery to the webhook 9")
    );
    assert_eq!(e.perform("github_webhook_ping", &touch).await["pinged"], true);
}

#[tokio::test]
async fn deploy_keys_are_listed_added_and_deleted() {
    let e = env().await;
    let key = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIExampleExampleExampleExampleExample me@laptop";
    e.on("GET", "/repos/octo/cat/keys", 200, json!([{"id": 4, "title": "CI", "key": key, "read_only": true, "verified": true, "created_at": "2026-10-01T00:00:00Z"}])).await;
    e.on("GET", "/repos/octo/cat/keys/4", 200, json!({"id": 4, "title": "CI", "read_only": false})).await;
    e.expect(
        "POST",
        "/repos/octo/cat/keys",
        json!({"title": "Deploy", "key": key, "read_only": false}),
        201,
        json!({"id": 5, "read_only": false}),
    )
    .await;
    e.expect_any("DELETE", "/repos/octo/cat/keys/4", 204, Value::Null).await;
    let list = e.fetch("github_deploy_key_list", &json!({"repo": "octo/cat"})).await;
    assert_eq!((list[0].id.as_str(), list[0].title.as_str(), list[0].snippet.as_str()), ("4", "CI", "read-only"));
    assert_eq!(list[0].extra["key"], key);

    let add = json!({"repo": "octo/cat", "title": "Deploy", "key": key, "read_only": false});
    let p = e.preview("github_deploy_key_add", &add).await;
    assert!(p.once_only);
    assert!(p.lines[0].contains("'Deploy'") && p.lines[0].contains("can also PUSH"), "{:?}", p.lines);
    assert!(p.lines[1].starts_with("Key: ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAA"), "{:?}", p.lines);
    assert!(!p.lines[1].contains("ExampleExampleExample"), "only the start of the key is shown");
    assert_eq!(e.perform("github_deploy_key_add", &add).await, json!({"added": true, "key_id": 5, "read_only": false}));
    for bad in ["hunter2", "-----BEGIN OPENSSH PRIVATE KEY-----", "ssh-rsa", "ssh-rsa AAAA\nssh-rsa BBBB"] {
        let err = e.preview_err("github_deploy_key_add", &json!({"repo": "octo/cat", "title": "T", "key": bad})).await;
        assert!(err.contains("public key line"), "{bad}: {err}");
    }
    let del = json!({"repo": "octo/cat", "key_id": 4});
    let p = e.preview("github_deploy_key_delete", &del).await;
    assert!(
        p.once_only && p.lines[0] == "Delete the deploy key 'CI' of octo/cat" && p.lines[1].contains("also push"),
        "{:?}",
        p.lines
    );
    assert_eq!(e.perform("github_deploy_key_delete", &del).await["deleted"], true);
}

// ---- traffic ---------------------------------------------------------------------------------------------------------

#[tokio::test]
async fn traffic_and_commit_activity_are_read() {
    let e = env().await;
    Mock::given(method("GET"))
        .and(path("/repos/octo/cat/traffic/clones"))
        .and(query_param("per", "week"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"count": 12, "uniques": 5, "clones": [{"timestamp": "2026-09-28T00:00:00Z", "count": 12, "uniques": 5}]})))
        .expect(1)
        .mount(&e.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/octo/cat/traffic/views"))
        .and(query_param("per", "day"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"count": 30, "uniques": 9, "views": [{"timestamp": "2026-10-01T00:00:00Z", "count": 30, "uniques": 9}]})))
        .expect(1)
        .mount(&e.server)
        .await;
    let clones = e.fetch("github_traffic_clones", &json!({"repo": "octo/cat", "per": "week"})).await;
    assert_eq!(clones[0].snippet, "12 total, 5 unique");
    assert!(clones[0].body.as_ref().unwrap().contains("2026-09-28T00:00:00Z week: 12 (5 unique)"));
    assert_eq!(clones[0].extra["days"][0]["count"], 12);
    let views = e.fetch("github_traffic_views", &json!({"repo": "octo/cat"})).await;
    assert_eq!((views[0].snippet.as_str(), views[0].extra["uniques"].clone()), ("30 total, 9 unique", json!(9)));

    e.on(
        "GET",
        "/repos/octo/cat/stats/commit_activity",
        200,
        json!([
        {"week": 1_790_000_000, "total": 4, "days": [0, 1, 1, 2, 0, 0, 0]},
        {"week": 1_790_604_800, "total": 6, "days": [1, 1, 1, 1, 2, 0, 0]}]),
    )
    .await;
    let activity = e.fetch("github_commit_activity", &json!({"repo": "octo/cat"})).await;
    assert_eq!(activity[0].snippet, "10 commits in 2 weeks");
    assert_eq!(activity[0].extra["total"], 10);
    assert_eq!(activity[0].extra["weeks"][1]["total"], 6);
    assert_eq!(activity[0].extra["weeks"][0]["week"], "2026-09-21T14:13:20Z");

    e.on_empty("GET", "/repos/octo/new/stats/commit_activity", 202).await;
    let computing = e.fetch("github_commit_activity", &json!({"repo": "octo/new"})).await;
    assert_eq!((computing[0].snippet.as_str(), computing[0].extra["ready"].clone()), ("being computed", json!(false)));
}

// ---- the registry and failures ------------------------------------------------------------------------------------------

/// Every tool of this area: name, effect, class, once-only.
const TOOLS: &[(&str, Effect, &str, bool)] = &[
    ("github_list_repos", Effect::List, "", false),
    ("github_org_repo_list", Effect::List, "", false),
    ("github_repo_get", Effect::Read, "", false),
    ("github_repo_create", Effect::Write, "settings", false),
    ("github_repo_update", Effect::Write, "settings", false),
    ("github_repo_fork", Effect::Write, "settings", false),
    ("github_repo_forks_list", Effect::List, "", false),
    ("github_repo_delete", Effect::Write, "settings", true),
    ("github_repo_transfer", Effect::Write, "settings", true),
    ("github_repo_visibility_set", Effect::Write, "settings", true),
    ("github_repo_languages", Effect::Read, "", false),
    ("github_repo_contributors", Effect::Read, "", false),
    ("github_repo_topics_get", Effect::Read, "", false),
    ("github_repo_topics_set", Effect::Write, "settings", false),
    ("github_repo_readme_get", Effect::Read, "", false),
    ("github_branch_list", Effect::List, "", false),
    ("github_branch_get", Effect::Read, "", false),
    ("github_branch_create", Effect::Write, "code", false),
    ("github_branch_delete", Effect::Write, "code", false),
    ("github_branch_rename", Effect::Write, "code", false),
    ("github_branch_merge", Effect::Write, "code", false),
    ("github_branch_protection_get", Effect::Read, "", false),
    ("github_branch_protection_set", Effect::Write, "settings", true),
    ("github_branch_protection_delete", Effect::Write, "settings", true),
    ("github_ruleset_list", Effect::Read, "", false),
    ("github_ruleset_get", Effect::Read, "", false),
    ("github_collaborator_list", Effect::Read, "", false),
    ("github_collaborator_add", Effect::Write, "settings", true),
    ("github_collaborator_remove", Effect::Write, "settings", true),
    ("github_invitation_list", Effect::Read, "", false),
    ("github_invitation_cancel", Effect::Write, "settings", true),
    ("github_repo_team_list", Effect::Read, "", false),
    ("github_repo_team_add", Effect::Write, "settings", true),
    ("github_repo_team_remove", Effect::Write, "settings", true),
    ("github_webhook_list", Effect::Read, "", false),
    ("github_webhook_get", Effect::Read, "", false),
    ("github_webhook_create", Effect::Write, "settings", true),
    ("github_webhook_update", Effect::Write, "settings", true),
    ("github_webhook_delete", Effect::Write, "settings", true),
    ("github_webhook_ping", Effect::Write, "settings", true),
    ("github_deploy_key_list", Effect::Read, "", false),
    ("github_deploy_key_add", Effect::Write, "settings", true),
    ("github_deploy_key_delete", Effect::Write, "settings", true),
    ("github_traffic_clones", Effect::Read, "", false),
    ("github_traffic_views", Effect::Read, "", false),
    ("github_commit_activity", Effect::Read, "", false),
];

#[test]
fn every_tool_is_registered_with_its_effect_class_and_once_only_flag() {
    for (name, effect, class, once) in TOOLS {
        let spec = spec_for_tool(name).unwrap_or_else(|| panic!("{name} is not registered"));
        assert_eq!(spec.effect, *effect, "{name}");
        assert_eq!(spec.class, *class, "{name}");
        assert_eq!(spec.once_only, *once, "{name}");
        assert_eq!(
            spec.op,
            name.strip_prefix("github_").unwrap(),
            "{name}: the op is the tool name without the prefix"
        );
        assert_eq!(spec.service, "github");
        assert!(spec.params.iter().all(|p| !p.description.is_empty()), "{name}");
        if *effect == Effect::Write {
            assert!(!class.is_empty(), "{name}: every write has a class");
        }
    }
    let legacy = spec_for_tool("github_list_repos").unwrap();
    assert_eq!(legacy.op, "list_repos");
}

/// A value for a required parameter, so that a call parses.
fn filler(p: &reins_proto::connector::Param) -> Value {
    use reins_proto::connector::Kind;
    match p.kind {
        Kind::Choice(options) => json!(options[0]),
        Kind::Int {
            ..
        } => json!(1),
        Kind::Bool => json!(true),
        _ => json!("x"),
    }
}

#[tokio::test]
async fn a_malformed_repository_never_reaches_the_network_for_any_tool() {
    let e = env().await;
    let mut tried = 0;
    for (name, effect, _, _) in TOOLS {
        let spec = spec_for_tool(name).unwrap();
        if !spec.params.iter().any(|p| p.name == "repo") {
            continue;
        }
        for repo in ["../../user", "a/b/c", "a b/c", "octo", "octo/cat?x=1", "a/b#1"] {
            let mut args = serde_json::Map::new();
            for p in spec.params.iter().filter(|p| p.required) {
                args.insert(p.name.to_owned(), filler(p));
            }
            args.insert("repo".to_owned(), json!(repo));
            let c = call(name, &Value::Object(args));
            let err = if *effect == Effect::Write {
                e.gh.preview(ME, &c).await.unwrap_err()
            } else {
                e.gh.fetch(ME, &c).await.unwrap_err()
            };
            assert!(err.to_string().contains("owner/name"), "{name} {repo}: {err}");
            if *effect == Effect::Write {
                assert!(matches!(e.gh.perform(ME, &c).await, Err(CoreError::Service { .. })), "{name} {repo}");
            }
            tried += 1;
        }
    }
    assert!(tried > 200, "{tried}");
    assert!(e.requests().await.is_empty(), "{:?}", e.requests().await);
}

#[tokio::test]
async fn github_failures_are_explained_with_githubs_own_message() {
    let e = env().await;
    e.on("GET", "/repos/octo/cat", 404, json!({"message": "Not Found"})).await;
    let err = e.fetch_err("github_repo_get", &json!({"repo": "octo/cat"})).await;
    assert!(err.contains("does not exist, or the token cannot see it: Not Found"), "{err}");

    e.on("PATCH", "/repos/octo/cat", 403, json!({"message": "Resource not accessible by personal access token"})).await;
    let err = e.perform_err("github_repo_update", &json!({"repo": "octo/cat", "has_wiki": true})).await;
    assert!(err.contains("refused") && err.contains("Resource not accessible by personal access token"), "{err}");

    e.on("POST", "/repos/octo/cat/git/refs", 422, json!({"message": "Reference already exists"})).await;
    e.on("GET", "/repos/octo/cat/git/ref/heads/main", 200, json!({"object": {"sha": SHA, "type": "commit"}})).await;
    let err =
        e.perform_err("github_branch_create", &json!({"repo": "octo/cat", "branch": "main", "from": "main"})).await;
    assert!(err.contains("did not accept that: Reference already exists"), "{err}");

    e.on(
        "PUT",
        "/repos/octo/cat/branches/main/protection",
        403,
        json!({"message": "Upgrade to GitHub Pro or make this repository public to enable this feature."}),
    )
    .await;
    let err = e.perform_err("github_branch_protection_set", &json!({"repo": "octo/cat", "branch": "main"})).await;
    assert!(err.contains("Upgrade to GitHub Pro"), "{err}");

    e.on("GET", "/repos/octo/cat/hooks", 401, json!({"message": "Bad credentials"})).await;
    let err = e.gh.fetch(ME, &call("github_webhook_list", &json!({"repo": "octo/cat"}))).await.unwrap_err();
    assert!(matches!(err, CoreError::ServiceNeedsAttention { .. }), "{err}");
    assert!(!err.to_string().contains("ghp_"));

    e.on("DELETE", "/repos/octo/cat", 403, json!({"message": "Must have admin rights to Repository."})).await;
    let err = e.perform_err("github_repo_delete", &json!({"repo": "octo/cat"})).await;
    assert!(err.contains("Must have admin rights"), "{err}");
    assert!(!err.contains("ghp_secret"));
}

#[tokio::test]
async fn required_arguments_are_enforced_by_the_registry() {
    for (tool, args, needle) in [
        ("github_repo_get", json!({}), "`repo` is required"),
        ("github_branch_create", json!({"repo": "a/b"}), "`branch` is required"),
        ("github_branch_merge", json!({"repo": "a/b", "base": "main"}), "`head` is required"),
        ("github_branch_rename", json!({"repo": "a/b", "branch": "x"}), "`new_name` is required"),
        ("github_repo_transfer", json!({"repo": "a/b"}), "`new_owner` is required"),
        ("github_deploy_key_add", json!({"repo": "a/b", "title": "t"}), "`key` is required"),
        ("github_webhook_create", json!({"repo": "a/b"}), "`url` is required"),
        ("github_repo_team_add", json!({"repo": "a/b", "team": "t"}), "`permission` is required"),
        ("github_invitation_cancel", json!({"repo": "a/b"}), "`invitation_id` is required"),
        ("github_deploy_key_delete", json!({"repo": "a/b"}), "`key_id` is required"),
        ("github_repo_get", json!({"repo": "a/b", "extra": 1}), "Unknown property `extra`"),
        ("github_list_repos", json!({"visibility": "secret"}), "one of"),
    ] {
        assert!(parse_err(tool, &args).contains(needle), "{tool}: {}", parse_err(tool, &args));
    }
}
