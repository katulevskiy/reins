//! `git setup` / `unsetup` (and so `resume` / `pause` / `status`) for every enabled git host, on a temporary `HOME`.

use std::path::Path;
use std::process::Command;

use reins_desktop::config::{Config, HostEntry};
use reins_desktop::setup::{Git, Scope};

fn git(home: &Path) -> Git {
    Git::default()
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("GIT_CONFIG_GLOBAL", home.join(".gitconfig"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
}

fn config(enabled: &[&str]) -> Config {
    let mut c = Config::default();
    c.git.hosts = ["gitlab.com", "codeberg.org", "bitbucket.org"]
        .iter()
        .map(|host| HostEntry {
            host: (*host).to_owned(),
            enabled: Some(enabled.contains(host)),
            ..HostEntry::default()
        })
        .collect();
    c
}

/// What git turns `url` into with the config in `home`.
fn rewritten(home: &Path, url: &str) -> String {
    let out = Command::new("git")
        .args(["ls-remote", "--get-url", url])
        .env("HOME", home)
        .env("GIT_CONFIG_GLOBAL", home.join(".gitconfig"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .current_dir(home)
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

#[test]
fn setup_covers_every_enabled_host_and_unsetup_removes_them_all() {
    let home = tempfile::tempdir().unwrap();
    let h = home.path();
    let g = git(h);
    std::fs::write(h.join(".gitconfig"), "[user]\n\tname = Me\n[url \"https://example.com/\"]\n\tinsteadOf = ex:\n")
        .unwrap();
    let c = config(&["gitlab.com", "codeberg.org"]);
    assert!(g.hosts_set_up(&Scope::Global, &c).unwrap().is_empty());
    let (added, removed) = g.setup_hosts(&Scope::Global, &c).unwrap();
    assert_eq!((added.len(), removed), (9, 0), "{added:?}");
    assert_eq!(g.hosts_set_up(&Scope::Global, &c).unwrap(), ["github.com", "gitlab.com", "codeberg.org"]);
    assert_eq!(g.setup_hosts(&Scope::Global, &c).unwrap(), (vec![], 0), "setting up twice changes nothing");

    assert_eq!(rewritten(h, "git@gitlab.com:group/sub/app.git"), "http://127.0.0.1:7457/gitlab.com/group/sub/app.git");
    assert_eq!(rewritten(h, "ssh://git@codeberg.org/me/app.git"), "http://127.0.0.1:7457/codeberg.org/me/app.git");
    assert_eq!(rewritten(h, "https://github.com/me/app"), "http://127.0.0.1:7457/github.com/me/app");
    assert_eq!(rewritten(h, "https://bitbucket.org/me/app.git"), "https://bitbucket.org/me/app.git", "not enabled");

    // Disabling a host and setting up again takes its rules away; enabling another adds its own.
    let c = config(&["codeberg.org", "bitbucket.org"]);
    let (added, removed) = g.setup_hosts(&Scope::Global, &c).unwrap();
    assert_eq!((added.len(), removed), (3, 3));
    assert_eq!(g.hosts_set_up(&Scope::Global, &c).unwrap(), ["github.com", "codeberg.org", "bitbucket.org"]);
    assert_eq!(rewritten(h, "git@gitlab.com:group/app.git"), "git@gitlab.com:group/app.git");

    // Unsetup removes the rules of every known host, enabled or not.
    let all_off = config(&[]);
    assert_eq!(g.unsetup_hosts(&Scope::Global, &all_off).unwrap(), 9);
    assert_eq!(g.unsetup_hosts(&Scope::Global, &all_off).unwrap(), 0);
    let text = std::fs::read_to_string(h.join(".gitconfig")).unwrap();
    assert!(!text.contains("7457"), "{text}");
    assert!(text.contains("insteadOf = ex:") && text.contains("name = Me"), "{text}");
}

#[test]
fn a_repository_scope_and_a_custom_host() {
    let home = tempfile::tempdir().unwrap();
    let g = git(home.path());
    let repo = home.path().join("r");
    assert!(Command::new("git").args(["init", "-q"]).arg(&repo).env("HOME", home.path()).status().unwrap().success());
    let c: Config = toml::from_str(
        "listen = \"127.0.0.1:9001\"\n[[git.hosts]]\nhost = \"git.example.com\"\nservice = \"gitlab\"\n",
    )
    .unwrap();
    let scope = Scope::Repo(repo.clone());
    let (added, _) = g.setup_hosts(&scope, &c).unwrap();
    assert!(added.contains(&"git@git.example.com:".to_owned()), "{added:?}");
    assert!(!home.path().join(".gitconfig").exists());
    let local = std::fs::read_to_string(repo.join(".git/config")).unwrap();
    assert!(local.contains("[url \"http://127.0.0.1:9001/git.example.com/\"]"), "{local}");
    assert_eq!(g.unsetup_hosts(&scope, &c).unwrap(), 6);
    assert!(!std::fs::read_to_string(repo.join(".git/config")).unwrap().contains("url"));
}
