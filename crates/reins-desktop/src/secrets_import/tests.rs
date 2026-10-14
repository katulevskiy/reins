//! Import against the made-up home folder of the scanner's tests, a fake phone and a fake computer (its trash is a
//! folder): grouping, the rewrite of every source, keeping local copies, partial failures, the receipt and undo.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use zeroize::Zeroizing;

use super::*;
use crate::secrets_scan::tests::{Fixture, fake, fixture, leaks};
use crate::secrets_scan::{Caller, ScanReport};

/// Stores everything (or fails the environments named), and remembers what it was given (names only).
#[derive(Default)]
struct FakeWriter {
    fail_envs: Vec<String>,
    seen: Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl VaultWriter for FakeWriter {
    async fn store(&self, batch: &Batch) -> Stored {
        let mut seen = self.seen.lock().unwrap();
        seen.push(format!("{batch:?}"));
        Stored {
            envs: batch
                .envs
                .iter()
                .map(|e| {
                    if self.fail_envs.contains(&e.project) {
                        Err("denied".to_owned())
                    } else {
                        Ok(())
                    }
                })
                .collect(),
            ssh: batch.ssh.iter().map(|_| Ok(())).collect(),
            tokens: batch.tokens.iter().map(|_| Ok(())).collect(),
        }
    }
}

/// A computer whose trash is a folder, whose agent signs (or not), and whose profiles are a map.
struct FakeMachine {
    trash_dir: PathBuf,
    trashed: Mutex<BTreeMap<PathBuf, Vec<PathBuf>>>,
    purged: Mutex<Vec<PathBuf>>,
    signs: bool,
    profiles: Mutex<BTreeMap<String, BTreeMap<String, String>>>,
    log: Mutex<Vec<String>>,
}

impl FakeMachine {
    fn new(dir: &Path, signs: bool) -> Self {
        let trash_dir = dir.join("trash");
        std::fs::create_dir_all(&trash_dir).unwrap();
        Self {
            trash_dir,
            trashed: Mutex::default(),
            purged: Mutex::default(),
            signs,
            profiles: Mutex::default(),
            log: Mutex::default(),
        }
    }

    fn in_trash(&self, path: &Path) -> bool {
        self.trashed.lock().unwrap().get(path).is_some_and(|v| !v.is_empty())
    }
}

#[async_trait::async_trait]
impl Machine for FakeMachine {
    fn trash(&self, path: &Path) -> Result<Option<String>, String> {
        let mut t = self.trashed.lock().unwrap();
        let n: usize = t.values().map(Vec::len).sum::<usize>() + self.purged.lock().unwrap().len();
        let to = self.trash_dir.join(format!("item-{n}"));
        std::fs::rename(path, &to).map_err(|e| format!("{}: {e}", path.display()))?;
        t.entry(path.to_path_buf()).or_default().push(to);
        Ok(Some(format!("item-{n}")))
    }

    fn restore(&self, path: &Path, trash_id: Option<&str>) -> Result<(), String> {
        let mut t = self.trashed.lock().unwrap();
        let items = t.get_mut(path).ok_or("not in the trash")?;
        let at =
            items.iter().position(|i| i.file_name().and_then(|n| n.to_str()) == trash_id).ok_or("not that item")?;
        let from = items.remove(at);
        assert!(std::fs::symlink_metadata(path).is_err(), "never over a file");
        std::fs::rename(from, path).map_err(|e| e.to_string())
    }

    fn purge(&self, path: &Path, trash_id: Option<&str>) -> Result<(), String> {
        let mut t = self.trashed.lock().unwrap();
        let items = t.get_mut(path).ok_or("not in the trash")?;
        let at =
            items.iter().position(|i| i.file_name().and_then(|n| n.to_str()) == trash_id).ok_or("not that item")?;
        let item = items.remove(at);
        std::fs::remove_file(&item).map_err(|e| e.to_string())?;
        self.purged.lock().unwrap().push(item);
        Ok(())
    }

    fn gh_token(&self, _host: &str) -> Option<Zeroizing<String>> {
        Some(Zeroizing::new(fake(&["gh", "o_"].concat(), 36).replace('a', "b")))
    }

    async fn agent_signs_with(&self, _public_key: &[u8]) -> Result<(), String> {
        if self.signs {
            Ok(())
        } else {
            Err("your phone does not list the key yet".to_owned())
        }
    }

    fn ssh_setup(&self) -> Result<bool, String> {
        self.log.lock().unwrap().push("ssh setup".to_owned());
        Ok(true)
    }

    fn ssh_unsetup(&self) -> Result<(), String> {
        self.log.lock().unwrap().push("ssh unsetup".to_owned());
        Ok(())
    }

    fn git_through_reins(&self, host: &str) -> Result<(), String> {
        self.log.lock().unwrap().push(format!("git {host}"));
        Ok(())
    }

    fn gh_logout(&self, host: &str) -> Result<(), String> {
        self.log.lock().unwrap().push(format!("gh logout {host}"));
        Ok(())
    }

    fn add_profile(&self, name: &str, env: &BTreeMap<String, String>, _purpose: &str) -> Result<(), String> {
        self.profiles.lock().unwrap().insert(name.to_owned(), env.clone());
        Ok(())
    }

    fn remove_profile(&self, name: &str) -> Result<(), String> {
        self.profiles.lock().unwrap().remove(name);
        Ok(())
    }
}

fn report(fx: &Fixture) -> ScanReport {
    secrets_scan::scan(&fx.options(), &mut |_| {}).unwrap()
}

fn ctx(fx: &Fixture) -> Context {
    Context {
        home: fx.home(),
        state_dir: fx.dir.path().join("state"),
        computer: "Dana's MacBook".to_owned(),
        caller: Caller::Person,
    }
}

/// Every file under the home folder and its text.
fn snapshot(home: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![home.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(&dir).unwrap().filter_map(Result::ok) {
            let p = e.path();
            if e.file_type().unwrap().is_dir() {
                stack.push(p);
            } else if e.file_type().unwrap().is_file() {
                out.insert(p.clone(), std::fs::read(&p).unwrap());
            }
        }
    }
    out
}

async fn run(
    fx: &Fixture,
    report: &ScanReport,
    selected: &BTreeSet<String>,
    keep_local: bool,
    writer: &FakeWriter,
    machine: &FakeMachine,
) -> Result<(Receipt, Option<PathBuf>), String> {
    let mut shown = None;
    let r = import(&ctx(fx), report, selected, keep_local, writer, machine, &mut |plan, check| {
        shown = Some((plan.count(), check.to_owned()));
        Ok(())
    })
    .await;
    if r.is_ok() {
        let (count, check) = shown.expect("the plan and check digits are shown first");
        assert!(count > 0 && check.len() == 4 && check.chars().all(|c| c.is_ascii_digit()));
    }
    r
}

#[test]
fn files_name_their_environments_and_credentials_their_variables() {
    assert_eq!(env_for(".env"), "development");
    assert_eq!(env_for(".env.local"), "development");
    assert_eq!(env_for(".env.development.local"), "development");
    assert_eq!(env_for(".env.production"), "production");
    assert_eq!(env_for(".env.production.local"), "production");
    assert_eq!(env_for(".env.staging"), "staging");
    assert_eq!(env_for(".env.Preview_EU"), "preview-eu");
    assert_eq!(
        machine_var(Source::Aws, Some("prod"), "aws_secret_access_key"),
        ("aws-prod".to_owned(), "AWS_SECRET_ACCESS_KEY".to_owned())
    );
    assert_eq!(
        machine_var(Source::Npm, Some("registry.npmjs.org"), "_authToken"),
        ("npm".to_owned(), "NPM_TOKEN".to_owned())
    );
    assert_eq!(machine_var(Source::Npm, Some("npm.pkg.github.com"), "_authToken").0, "npm-npm-pkg-github-com");
    assert_eq!(machine_var(Source::PyPi, Some("pypi"), "password"), ("pypi".to_owned(), "TWINE_PASSWORD".to_owned()));
    assert_eq!(machine_var(Source::Cargo, Some("crates.io"), "token").1, "CARGO_REGISTRY_TOKEN");
    assert_eq!(machine_var(Source::Cargo, Some("acme"), "token").1, "CARGO_REGISTRIES_ACME_TOKEN");
    assert_eq!(var_name("apiKey.v2"), "APIKEY_V2");
    assert_eq!(var_name("1x"), "V_1X");
}

#[test]
fn the_selection_is_grouped_by_where_it_goes() {
    let fx = fixture();
    let report = report(&fx);
    let selected = select(&report, &[], true).unwrap();
    let plan = plan(&report, &selected, "Dana's MacBook");
    let envs: Vec<(String, usize)> = plan.envs.iter().map(|e| (e.item(), e.vars.len())).collect();
    for want in [
        ("acme-web/development", 4),
        ("api/development", 1),
        ("shop/production", 1),
        ("machine/Dana's MacBook/aws-default", 2),
        ("machine/Dana's MacBook/aws-prod", 2),
        ("machine/Dana's MacBook/npm", 1),
        ("machine/Dana's MacBook/pypi", 1),
        ("machine/Dana's MacBook/cargo", 1),
        ("machine/Dana's MacBook/shell", 1),
    ] {
        assert!(envs.contains(&(want.0.to_owned(), want.1)), "{want:?} not in {envs:?}");
    }
    let acme = plan.envs.iter().find(|e| e.project == "acme-web").unwrap();
    assert_eq!(acme.folder(), "Reins/acme-web");
    assert_eq!(acme.reference("OPENAI_API_KEY"), "vault:acme-web/development/OPENAI_API_KEY");
    let names: Vec<&str> = acme.vars.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(names, ["OPENAI_API_KEY", "STRIPE_SECRET_KEY", "DATABASE_URL", "SUPABASE_SERVICE_KEY"]);
    let ssh: Vec<&str> = plan.ssh.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(ssh, ["id_ed25519 (Dana's MacBook)", "work_key (Dana's MacBook)"]);
    let hosts: Vec<&str> = plan.tokens.iter().map(|t| t.host.as_str()).collect();
    assert_eq!(hosts, ["github.com", "ghe.acme.dev", "gitlab.com"]);
    assert!(!plan.tokens[0].covers.is_empty(), "the same host's token in another file goes with it");
    assert!(plan.lines().contains(&"acme-web/development: 4 variables".to_owned()));
    // Selecting a finding takes its pre-ticked items; an item, just it.
    let acme_id = report.findings.iter().find(|f| f.title == "acme-web/.env").unwrap().id.clone();
    assert_eq!(select(&report, std::slice::from_ref(&acme_id), false).unwrap().len(), 4);
    assert_eq!(select(&report, &[format!("{acme_id}/LOG_LEVEL")], false).unwrap().len(), 1);
    assert!(select(&report, &["nope".to_owned()], false).is_err());
}

#[test]
fn projects_with_the_same_name_stay_apart() {
    let fx = fixture();
    let home = fx.home();
    std::fs::create_dir_all(home.join("work/api/.git")).unwrap();
    std::fs::write(home.join("work/api/.env"), format!("API_SECRET={}\n", fake("", 33))).unwrap();
    let report = report(&fx);
    let selected = select(&report, &[], true).unwrap();
    let projects: BTreeSet<String> = plan(&report, &selected, "x").envs.iter().map(|e| e.project.clone()).collect();
    assert!(projects.contains("api") && projects.contains("work-api"), "{projects:?}");
}

#[tokio::test]
async fn an_import_moves_everything_and_rewrites_each_source() {
    let fx = fixture();
    let home = fx.home();
    // A suspected secret left unticked stays in its file.
    std::fs::write(
        home.join("dev/api/.env.local"),
        format!("API_SECRET={}\nPORT=3000\nOTHER_TOKEN={}\n", fake("", 32), fake("", 31)),
    )
    .unwrap();
    let report = report(&fx);
    let mut selected = select(&report, &[], true).unwrap();
    let api = report.findings.iter().find(|f| f.title == "api/.env.local").unwrap();
    selected.remove(&format!("{}/OTHER_TOKEN", api.id));
    let writer = FakeWriter::default();
    let machine = FakeMachine::new(fx.dir.path(), true);
    let (receipt, path) = run(&fx, &report, &selected, false, &writer, &machine).await.unwrap();
    let path = path.unwrap();
    assert!(path.starts_with(fx.dir.path().join("state/imports")));

    // .env → reins.toml + .env.reins, the original in the trash.
    let acme = home.join("dev/acme-web");
    assert!(!acme.join(".env").exists() && machine.in_trash(&acme.join(".env")));
    assert_eq!(
        std::fs::read_to_string(acme.join("reins.toml")).unwrap(),
        "[project]\nname = \"acme-web\"\ndefault_env = \"development\"\n\n[env.development]\nlease = \"8h\"\n"
    );
    assert_eq!(
        std::fs::read_to_string(acme.join(".env.reins")).unwrap(),
        "OPENAI_API_KEY=vault:acme-web/development/OPENAI_API_KEY\n\
         STRIPE_SECRET_KEY=vault:acme-web/development/STRIPE_SECRET_KEY\n\
         DATABASE_URL=vault:acme-web/development/DATABASE_URL\n\
         SUPABASE_SERVICE_KEY=vault:acme-web/development/SUPABASE_SERVICE_KEY\n\
         NEXT_PUBLIC_SITE_URL=https://acme.dev\nLOG_LEVEL=debug\n"
    );
    // The unticked suspected secret stays in its file; the setting goes to .env.reins.
    let api_dir = home.join("dev/api");
    let rest = std::fs::read_to_string(api_dir.join(".env.local")).unwrap();
    assert!(rest.starts_with("OTHER_TOKEN=") && !rest.contains("API_SECRET"), "{rest}");
    assert!(std::fs::read_to_string(api_dir.join(".env.reins")).unwrap().contains("PORT=3000"));
    // Production asks every time.
    assert!(std::fs::read_to_string(home.join("clients/shop/reins.toml")).unwrap().contains("confirm = \"always\""));
    assert!(!home.join("clients/shop/.env.reins").exists(), "only development has an .env.reins");

    // SSH: in the trash once the agent signed; the .pub stays; ssh points at the agent.
    assert!(machine.in_trash(&home.join(".ssh/id_ed25519")) && home.join(".ssh/id_ed25519.pub").exists());
    assert!(receipt.changes.contains(&Change::SshSetup));
    // gh and git: through Reins, tokens out of the files, the keychain's signed out.
    let log = machine.log.lock().unwrap().clone();
    assert!(
        log.contains(&"git github.com".to_owned()) && log.contains(&"gh logout ghe.acme.dev".to_owned()),
        "{log:?}"
    );
    assert!(!std::fs::read_to_string(home.join(".config/gh/hosts.yml")).unwrap().contains("oauth_token"));
    assert_eq!(std::fs::read_to_string(home.join(".git-credentials")).unwrap(), "");
    // Tools that read variables: profiles, and the tokens out of their files.
    let profiles = machine.profiles.lock().unwrap().clone();
    assert_eq!(
        profiles["aws-prod"]["AWS_SECRET_ACCESS_KEY"],
        "vault:machine/Dana's MacBook/aws-prod/AWS_SECRET_ACCESS_KEY"
    );
    for p in ["aws-default", "npm", "pypi", "cargo", "shell"] {
        assert!(profiles.contains_key(p), "{p} in {profiles:?}");
    }
    let aws = std::fs::read_to_string(home.join(".aws/credentials")).unwrap();
    assert_eq!(aws, "[default]\n[prod]\n");
    assert_eq!(
        std::fs::read_to_string(home.join(".npmrc")).unwrap(),
        "//registry.npmjs.org/:_authToken=${NPM_TOKEN}\n"
    );
    assert!(!std::fs::read_to_string(home.join(".pypirc")).unwrap().contains("password"));
    let zshrc = std::fs::read_to_string(home.join(".zshrc")).unwrap();
    assert!(zshrc.contains("# OPENAI_API_KEY is in your vault now (Reins): reins run --profile shell"), "{zshrc}");
    assert!(zshrc.contains("export EDITOR=vim"));
    // Tools that read only their own file keep it, and say so.
    assert!(home.join(".docker/config.json").exists() && home.join(".kube/config").exists());
    assert!(receipt.done.iter().any(|d| d.source == Source::Docker && !d.ok));
    assert!(receipt.done.iter().any(|d| d.what == "3 .env files → .env.reins" && d.ok), "{:?}", receipt.done);
    assert!(receipt.notes.iter().any(|n| n.contains("1 variable not moved")), "{:?}", receipt.notes);

    // No value anywhere: the receipt, what the phone side was shown in Debug, the notes.
    let text = std::fs::read_to_string(&path).unwrap() + &writer.seen.lock().unwrap().join("\n");
    for secret in &fx.secrets {
        assert!(!leaks(&text, secret), "a value leaked");
    }
}

#[tokio::test]
async fn undo_puts_every_file_back() {
    let fx = fixture();
    let before = snapshot(&fx.home());
    let report = report(&fx);
    let selected = select(&report, &[], true).unwrap();
    let machine = FakeMachine::new(fx.dir.path(), true);
    let (_, path) = run(&fx, &report, &selected, false, &FakeWriter::default(), &machine).await.unwrap();
    let path = path.unwrap();
    assert!(snapshot(&fx.home()).len() != before.len() || snapshot(&fx.home()).iter().ne(before.iter()));
    let c = ctx(&fx);
    let (loaded_path, mut receipt) = receipt::load(&c.state_dir, "last").unwrap();
    assert_eq!(loaded_path, path);
    let lines = receipt::undo(&c.home, &c.state_dir, &path, &mut receipt, &machine).unwrap();
    assert!(lines.iter().all(|l| !l.starts_with("not undone")), "{lines:?}");
    let after = snapshot(&fx.home());
    let differ: Vec<&PathBuf> = before.keys().chain(after.keys()).filter(|p| before.get(*p) != after.get(*p)).collect();
    assert!(differ.is_empty(), "not as it was: {differ:?}");
    assert!(machine.profiles.lock().unwrap().is_empty());
    assert!(machine.log.lock().unwrap().contains(&"ssh unsetup".to_owned()));
    // Once only.
    let (_, mut again) = receipt::load(&c.state_dir, path.to_str().unwrap()).unwrap();
    assert!(again.undone_at.is_some());
    assert!(receipt::undo(&c.home, &c.state_dir, &path, &mut again, &machine).is_err());
}

#[tokio::test]
async fn keeping_local_copies_changes_nothing() {
    let fx = fixture();
    let before = snapshot(&fx.home());
    let report = report(&fx);
    let selected = select(&report, &[], true).unwrap();
    let machine = FakeMachine::new(fx.dir.path(), true);
    let (receipt, path) = run(&fx, &report, &selected, true, &FakeWriter::default(), &machine).await.unwrap();
    assert!(snapshot(&fx.home()) == before, "nothing changed");
    assert!(receipt.keep_local && receipt.changes.is_empty() && !receipt.moved.is_empty());
    assert!(path.is_some(), "what moved is still recorded");
}

#[tokio::test]
async fn what_did_not_arrive_stays_and_an_unsigned_key_waits() {
    let fx = fixture();
    let report = report(&fx);
    let selected = select(&report, &[], true).unwrap();
    let writer = FakeWriter {
        fail_envs: vec!["acme-web".to_owned()],
        ..FakeWriter::default()
    };
    let machine = FakeMachine::new(fx.dir.path(), false);
    let (receipt, path) = run(&fx, &report, &selected, false, &writer, &machine).await.unwrap();
    let home = fx.home();
    assert!(home.join("dev/acme-web/.env").exists() && !home.join("dev/acme-web/reins.toml").exists());
    assert!(receipt.notes.iter().any(|n| n.contains("acme-web/development was not stored: denied")));
    // The key stays until the agent signs with it (and says it is still on disk); `finish` tries again.
    assert!(home.join(".ssh/id_ed25519").exists());
    assert!(receipt.moved.iter().filter(|m| m.to.starts_with("Reins/ssh/")).all(|m| m.on_disk));
    assert_eq!(receipt.pending_keys.len(), 2);
    let path = path.unwrap();
    let mut receipt = receipt;
    let signing = FakeMachine::new(&fx.dir.path().join("later"), true);
    let c = ctx(&fx);
    let lines = receipt::finish(&c.home, &c.state_dir, &path, &mut receipt, &signing).await.unwrap();
    assert!(lines[0].contains("in the trash"), "{lines:?}");
    assert!(!home.join(".ssh/id_ed25519").exists() && receipt.pending_keys.is_empty());
    assert!(receipt.moved.iter().filter(|m| m.to.starts_with("Reins/ssh/")).all(|m| !m.on_disk));
}

#[tokio::test]
async fn a_file_changed_since_the_scan_stops_the_import() {
    let fx = fixture();
    let report = report(&fx);
    let selected = select(&report, &[], true).unwrap();
    std::fs::write(fx.home().join("dev/acme-web/.env"), "LOG_LEVEL=debug\n").unwrap();
    let machine = FakeMachine::new(fx.dir.path(), true);
    let writer = FakeWriter::default();
    let err = run(&fx, &report, &selected, false, &writer, &machine).await.unwrap_err();
    assert!(err.contains("changed since the scan"), "{err}");
    assert!(writer.seen.lock().unwrap().is_empty(), "nothing was sent");
}

#[tokio::test]
async fn agents_cannot_import_and_a_no_stops_before_sending() {
    let fx = fixture();
    let report = report(&fx);
    let selected = select(&report, &[], true).unwrap();
    let machine = FakeMachine::new(fx.dir.path(), true);
    let writer = FakeWriter::default();
    let mut c = ctx(&fx);
    c.caller = Caller::Agent("CLAUDECODE".to_owned());
    let err = import(&c, &report, &selected, false, &writer, &machine, &mut |_, _| Ok(())).await.unwrap_err();
    assert!(err.contains("AI agent"), "{err}");
    let err =
        import(&ctx(&fx), &report, &selected, false, &writer, &machine, &mut |_, _| Err("Nothing done.".to_owned()))
            .await
            .unwrap_err();
    assert_eq!(err, "Nothing done.");
    assert!(writer.seen.lock().unwrap().is_empty());
}

#[test]
fn values_never_print() {
    let secret = fake("sk-proj-", 60);
    let batch = Batch {
        computer: "x".to_owned(),
        envs: vec![EnvWrite {
            project: "p".to_owned(),
            env: "development".to_owned(),
            vars: vec![("OPENAI_API_KEY".to_owned(), Zeroizing::new(secret.clone()))],
        }],
        ssh: vec![SshWrite {
            name: "k".to_owned(),
            folder: "Reins/ssh".to_owned(),
            private_key: Zeroizing::new(secret.clone()),
        }],
        tokens: vec![TokenWrite {
            host: "github.com".to_owned(),
            user: None,
            token: Zeroizing::new(secret.clone()),
        }],
    };
    let line = formats::EnvLine::Literal {
        name: "X".to_owned(),
        value: Zeroizing::new(secret.clone()),
    };
    let shown = format!("{batch:?} {:?} {:?} {:?} {line:?}", batch.envs[0], batch.ssh[0], batch.tokens[0]);
    assert!(!leaks(&shown, &secret), "{shown}");
    assert!(shown.contains("OPENAI_API_KEY"));
    // The check digits change with any value.
    let check = batch.check();
    let mut other = batch;
    other.envs[0].vars[0].1 = Zeroizing::new(fake("sk-proj-", 61));
    assert_ne!(check, other.check());
}

#[test]
fn the_receipt_format_is_stable() {
    let receipt = Receipt {
        version: RECEIPT_VERSION,
        at: 1_700_000_000,
        computer: "Dana's MacBook".to_owned(),
        keep_local: false,
        moved: vec![Moved {
            item: "dotenv-0123/OPENAI_API_KEY".to_owned(),
            finding: "dotenv-0123".to_owned(),
            to: "vault:acme-web/development/OPENAI_API_KEY".to_owned(),
            on_disk: false,
        }],
        changes: vec![
            Change::Created {
                path: PathBuf::from("/h/dev/acme-web/reins.toml"),
            },
            Change::Replaced {
                path: PathBuf::from("/h/.npmrc"),
                trash_id: Some("item-1".to_owned()),
            },
            Change::Trashed {
                path: PathBuf::from("/h/dev/acme-web/.env"),
                trash_id: None,
            },
            Change::Profile {
                name: "npm".to_owned(),
            },
            Change::SshSetup,
            Change::GitHost {
                host: "github.com".to_owned(),
            },
            Change::GhLogout {
                host: "github.com".to_owned(),
            },
        ],
        done: vec![Done {
            source: Source::Dotenv,
            what: "1 .env file → .env.reins".to_owned(),
            how: "reins run -- <your command>".to_owned(),
            ok: true,
        }],
        notes: vec![],
        pending_keys: vec![],
        undone_at: None,
        purged_at: None,
    };
    let expected = serde_json::json!({
        "version": 1, "at": 1_700_000_000, "computer": "Dana's MacBook", "keep_local": false,
        "moved": [{"item": "dotenv-0123/OPENAI_API_KEY", "finding": "dotenv-0123", "to": "vault:acme-web/development/OPENAI_API_KEY", "on_disk": false}],
        "changes": [
            {"kind": "created", "path": "/h/dev/acme-web/reins.toml"},
            {"kind": "replaced", "path": "/h/.npmrc", "trash_id": "item-1"},
            {"kind": "trashed", "path": "/h/dev/acme-web/.env"},
            {"kind": "profile", "name": "npm"},
            {"kind": "ssh_setup"},
            {"kind": "git_host", "host": "github.com"},
            {"kind": "gh_logout", "host": "github.com"}
        ],
        "done": [{"source": "dotenv", "what": "1 .env file → .env.reins", "how": "reins run -- <your command>", "ok": true}],
        "notes": [], "pending_keys": []
    });
    assert_eq!(serde_json::to_value(&receipt).unwrap(), expected);
    assert_eq!(serde_json::from_value::<Receipt>(expected).unwrap(), receipt);
}

#[tokio::test]
async fn a_receipt_is_authenticated_and_only_its_own_folder_counts() {
    let fx = fixture();
    let report = report(&fx);
    let selected = select(&report, &[], true).unwrap();
    let machine = FakeMachine::new(fx.dir.path(), true);
    let (_, path) = run(&fx, &report, &selected, false, &FakeWriter::default(), &machine).await.unwrap();
    let path = path.unwrap();
    let c = ctx(&fx);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(std::fs::metadata(path.parent().unwrap()).unwrap().permissions().mode() & 0o777, 0o700);
    }
    // Changed by anything else: refused, and nothing is undone.
    let text = std::fs::read_to_string(&path).unwrap();
    let tampered = text.replacen("/dev/acme-web/.env\"", "/.ssh/authorized_keys\"", 1);
    assert_ne!(tampered, text);
    std::fs::write(&path, &tampered).unwrap();
    let err = receipt::load(&c.state_dir, "last").unwrap_err();
    assert!(err.contains("refused"), "{err}");
    std::fs::write(&path, &text).unwrap();
    assert!(receipt::load(&c.state_dir, "last").is_ok());
    // A copy elsewhere, even a valid one, is not a receipt.
    let elsewhere = fx.dir.path().join("receipt.json");
    std::fs::write(&elsewhere, &text).unwrap();
    assert!(receipt::load(&c.state_dir, elsewhere.to_str().unwrap()).is_err());
    // Another install's key does not verify it.
    std::fs::write(c.state_dir.join("import-receipts.key"), [9u8; 32]).unwrap();
    assert!(receipt::load(&c.state_dir, "last").is_err());
}

#[test]
fn undo_touches_only_plain_places_in_the_home_folder() {
    let fx = fixture();
    let home = fx.home();
    assert!(receipt::check_place(&home, &home.join("dev/acme-web/.env")).is_ok());
    for bad in
        [home.join("dev/../../outside"), PathBuf::from("/etc/passwd"), PathBuf::from("relative/.env"), home.clone()]
    {
        assert!(receipt::check_place(&home, &bad).is_err(), "{}", bad.display());
    }
    #[cfg(unix)]
    {
        let outside = fx.dir.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, home.join("dev/linked")).unwrap();
        std::os::unix::fs::symlink(home.join("dev/acme-web"), home.join("dev/inner-link")).unwrap();
        let err = receipt::check_place(&home, &home.join("dev/linked/.env")).unwrap_err();
        assert!(err.contains("symbolic link") || err.contains("outside"), "{err}");
        assert!(receipt::check_place(&home, &home.join("dev/inner-link/.env")).unwrap_err().contains("symbolic link"));
    }
}

#[tokio::test]
async fn undo_never_overwrites_a_file_that_is_back() {
    let fx = fixture();
    let report = report(&fx);
    let selected = select(&report, &[], true).unwrap();
    let machine = FakeMachine::new(fx.dir.path(), true);
    let (_, path) = run(&fx, &report, &selected, false, &FakeWriter::default(), &machine).await.unwrap();
    let path = path.unwrap();
    let env = fx.home().join("dev/acme-web/.env");
    std::fs::write(&env, "NEW=1\n").unwrap();
    let c = ctx(&fx);
    let (_, mut r) = receipt::load(&c.state_dir, "last").unwrap();
    let lines = receipt::undo(&c.home, &c.state_dir, &path, &mut r, &machine).unwrap();
    assert!(lines.iter().any(|l| l.starts_with("not undone") && l.contains("not overwritten")), "{lines:?}");
    assert_eq!(std::fs::read_to_string(&env).unwrap(), "NEW=1\n");
    assert!(r.undone_at.is_none(), "not all of it was undone");
}

#[tokio::test]
async fn new_files_hold_no_secret_and_keep_their_permissions() {
    let fx = fixture();
    let home = fx.home();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(home.join(".aws/credentials"), std::fs::Permissions::from_mode(0o600)).unwrap();
        std::fs::set_permissions(home.join(".npmrc"), std::fs::Permissions::from_mode(0o640)).unwrap();
    }
    let report = report(&fx);
    let selected = select(&report, &[], true).unwrap();
    let machine = FakeMachine::new(fx.dir.path(), true);
    let before = snapshot(&home);
    let (receipt, _) = run(&fx, &report, &selected, false, &FakeWriter::default(), &machine).await.unwrap();
    // Every file the import wrote or changed is free of values; what it left alone stays as it was (the tools that
    // read only their own file are reported as still on disk below).
    for (path, text) in snapshot(&home) {
        if before.get(&path) == Some(&text) {
            continue;
        }
        let text = String::from_utf8_lossy(&text).into_owned();
        for secret in &fx.secrets {
            assert!(!leaks(&text, secret), "a value in {}", path.display());
        }
    }
    let on_disk: BTreeSet<&str> = receipt.moved.iter().filter(|m| m.on_disk).map(|m| m.finding.as_str()).collect();
    let kept_ids: BTreeSet<&str> = report
        .findings
        .iter()
        .filter(|f| matches!(f.source, Source::Docker | Source::Kube | Source::Gcp | Source::Netrc))
        .map(|f| f.id.as_str())
        .collect();
    assert_eq!(on_disk, kept_ids);
    assert!(receipt.done.iter().filter(|d| !d.ok).all(|d| d.how.starts_with("still on disk")));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = |p: &str| std::fs::metadata(home.join(p)).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(".aws/credentials"), 0o600);
        assert_eq!(mode(".npmrc"), 0o640);
        assert_eq!(mode("dev/acme-web/.env.reins"), 0o644);
        assert_eq!(mode("dev/acme-web/reins.toml"), 0o644);
    }
    // No temporary file is left behind.
    assert!(!snapshot(&home).keys().any(|p| p.to_string_lossy().contains(".reins-")));
}

#[tokio::test]
async fn the_originals_can_be_deleted_from_the_trash_for_good() {
    let fx = fixture();
    let report = report(&fx);
    let selected = select(&report, &[], true).unwrap();
    let machine = FakeMachine::new(fx.dir.path(), true);
    let (receipt, path) = run(&fx, &report, &selected, false, &FakeWriter::default(), &machine).await.unwrap();
    let path = path.unwrap();
    let n = receipt.trashed().len();
    assert!(n > 5);
    let c = ctx(&fx);
    let (_, mut r) = receipt::load(&c.state_dir, "last").unwrap();
    let lines = receipt::purge(&c.state_dir, &path, &mut r, &machine).unwrap();
    assert_eq!(lines.len(), n);
    assert_eq!(machine.purged.lock().unwrap().len(), n);
    assert!(std::fs::read_dir(&machine.trash_dir).unwrap().next().is_none(), "the trash holds none of them");
    let (_, mut r) = receipt::load(&c.state_dir, "last").unwrap();
    assert!(r.purged_at.is_some() && receipt::purge(&c.state_dir, &path, &mut r, &machine).is_err());
}
