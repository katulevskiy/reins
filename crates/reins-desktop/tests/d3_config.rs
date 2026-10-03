//! `[[git.hosts]]`: the built-in hosts, overriding them, adding one, and old configs with only `[github]`.

use reins_desktop::config::{Config, GitHost, Paths};

fn load(text: &str) -> Result<Config, String> {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::under(dir.path());
    std::fs::create_dir_all(&paths.config_dir).unwrap();
    std::fs::write(paths.config_file(), text).unwrap();
    Config::load(&paths).map_err(|e| e.to_string())
}

fn host<'a>(hosts: &'a [GitHost], name: &str) -> &'a GitHost {
    hosts.iter().find(|h| h.host == name).unwrap_or_else(|| panic!("no {name} in {hosts:#?}"))
}

#[test]
fn the_built_in_hosts_and_their_defaults() {
    let hosts = Config::default().git_hosts().unwrap();
    let names: Vec<_> = hosts.iter().map(|h| (h.host.as_str(), h.service.as_str(), h.enabled)).collect();
    assert_eq!(
        names,
        [
            ("github.com", "github", true),
            ("gitlab.com", "gitlab", false),
            ("codeberg.org", "codeberg", false),
            ("bitbucket.org", "bitbucket", false),
        ]
    );
    let gh = host(&hosts, "github.com");
    assert_eq!((gh.git_base.as_str(), gh.api_base.as_str()), ("https://github.com", "https://api.github.com"));
    assert_eq!((gh.token.as_deref(), gh.username.as_str()), (Some("gh"), "x-access-token"));
    let gl = host(&hosts, "gitlab.com");
    assert_eq!((gl.git_base.as_str(), gl.api_base.as_str()), ("https://gitlab.com", "https://gitlab.com/api/v4"));
    assert_eq!((gl.token.as_deref(), gl.username.as_str()), (None, "oauth2"));
    let cb = host(&hosts, "codeberg.org");
    assert_eq!(cb.api_base, "https://codeberg.org/api/v1");
    let bb = host(&hosts, "bitbucket.org");
    assert_eq!((bb.api_base.as_str(), bb.username.as_str()), ("https://api.bitbucket.org/2.0", "x-token-auth"));
    assert_eq!(gl.label(), "GitLab");
    assert_eq!(Config::default().proxy_base_for("gitlab.com"), "http://127.0.0.1:7457/gitlab.com/");
}

#[test]
fn an_old_config_with_only_a_github_section_still_loads() {
    let c = load(
        "listen = \"127.0.0.1:9000\"\nmode = \"local\"\n[github]\nhost = \"github.com\"\ngit_base = \"https://github.com\"\napi_base = \"https://api.github.com\"\naccount = \"octo\"\ntoken = \"env:GH_TOKEN\"\n[policy]\npush = \"allow\"\n[[policy.rules]]\nrepo = \"me/*\"\npush = \"ask\"\n",
    )
    .unwrap();
    assert_eq!(c.proxy_base(), "http://127.0.0.1:9000/github.com/");
    let enabled = c.enabled_hosts().unwrap();
    assert_eq!(enabled.len(), 1);
    let gh = &enabled[0];
    assert_eq!((gh.host.as_str(), gh.service.as_str()), ("github.com", "github"));
    assert_eq!((gh.account.as_deref(), gh.token.as_deref()), (Some("octo"), Some("env:GH_TOKEN")));
    assert_eq!(c.policy.rules[0].host, None);
    // A GitHub Enterprise host in `[github]` is the GitHub host.
    let c = load("[github]\nhost = \"ghe.example.com\"\ngit_base = \"https://ghe.example.com\"\n").unwrap();
    let gh = &c.enabled_hosts().unwrap()[0];
    assert_eq!(
        (gh.host.as_str(), gh.service.as_str(), gh.git_base.as_str()),
        ("ghe.example.com", "github", "https://ghe.example.com")
    );
    assert_eq!(c.proxy_base(), "http://127.0.0.1:7457/ghe.example.com/");
}

#[test]
fn hosts_can_be_enabled_overridden_and_added() {
    let c = load(
        r#"
[[git.hosts]]
host = "gitlab.com"
enabled = true
account = "work"
token = "env:GITLAB_TOKEN"

[[git.hosts]]
host = "Codeberg.org"
enabled = true
git_base = "http://127.0.0.1:3000"

[[git.hosts]]
host = "git.example.com"
service = "gitlab"

[[git.hosts]]
host = "github.com"
enabled = false
"#,
    )
    .unwrap();
    let hosts = c.git_hosts().unwrap();
    let gl = host(&hosts, "gitlab.com");
    assert!(gl.enabled);
    assert_eq!((gl.account.as_deref(), gl.token.as_deref()), (Some("work"), Some("env:GITLAB_TOKEN")));
    let cb = host(&hosts, "codeberg.org");
    assert_eq!((cb.git_base.as_str(), cb.api_base.as_str()), ("http://127.0.0.1:3000", "https://codeberg.org/api/v1"));
    let own = host(&hosts, "git.example.com");
    assert_eq!((own.service.as_str(), own.enabled), ("gitlab", true));
    assert_eq!(
        (own.git_base.as_str(), own.api_base.as_str()),
        ("https://git.example.com", "https://git.example.com/api/v4")
    );
    assert_eq!(own.username, "oauth2");
    let enabled: Vec<_> = c.enabled_hosts().unwrap().into_iter().map(|h| h.host).collect();
    assert_eq!(enabled, ["gitlab.com", "codeberg.org", "git.example.com"]);
    // `[[git.hosts]]` for github.com wins over `[github]`.
    let c = load("[github]\naccount = \"a\"\n[[git.hosts]]\nhost = \"github.com\"\naccount = \"b\"\n").unwrap();
    assert_eq!(c.git_hosts().unwrap()[0].account.as_deref(), Some("b"));
}

#[test]
fn bad_host_entries_are_refused() {
    for (text, expected) in [
        ("[[git.hosts]]\nhost = \"git.example.com\"\n", "service"),
        ("[[git.hosts]]\nhost = \"git.example.com\"\nservice = \"sourcehut\"\n", "sourcehut"),
        ("[[git.hosts]]\nhost = \"gitlab.com\"\nservice = \"github\"\n", "gitlab.com"),
        ("[[git.hosts]]\nhost = \"gitlab.com\"\n[[git.hosts]]\nhost = \"GitLab.com\"\n", "more than once"),
        ("[[git.hosts]]\nhost = \"gitlab.com/x\"\n", "host name"),
        ("[[git.hosts]]\nhost = \"\"\n", "host name"),
        ("[[git.hosts]]\nhost = \"gitlab.com\"\ngit_base = \"http://gitlab.com\"\n", "https"),
        ("[[git.hosts]]\nhost = \"gitlab.com\"\napi_base = \"ftp://gitlab.com\"\n", "https"),
        ("[[git.hosts]]\nhost = \"gitlab.com\"\nusername = \"a:b\"\n", "username"),
        ("[[git.hosts]]\nhost = \"gitlab.com\"\nfoo = 1\n", "foo"),
        ("[github]\nhost = \"github.com:443\"\n", "host name"),
    ] {
        let e = load(text).unwrap_err();
        assert!(e.contains(expected), "{text}: {e}");
    }
}
